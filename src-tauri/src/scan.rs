//! The scan commands and the `scan://progress` event (spec §7.10
//! Projects module `scan_start`/`scan_cancel`/`scan_commit`, §9.6,
//! PRJ-002, SYS-003/NFR-015, ALG-053, ERR-002; T-076, D-153; T-077,
//! D-179).
//!
//! This module is the COMMAND LAYER the walker's docs assigned to
//! "T-076's plumbing": it drives
//! [`scan_repositories`](eunomia_fs::scan::scan_repositories) over a
//! SEC-003-validated root, classifies every event through
//! [`classify_entry`](eunomia_fs::scan_classify::classify_entry)
//! against a per-scan registry snapshot, serializes the live facts
//! into the throttled `scan://progress` event, and answers with the
//! found/candidate set — the input PRJ-002's result view selects from
//! and [`scan_commit`] consumes. The SCAN half persists nothing
//! anywhere (ERR-002): `scan_start` is read-only over the filesystem
//! and the database, and a cancelled or interrupted scan answers
//! [`ScanRunOutcome::Cancelled`] with no payload at all. The module's
//! ONE writer is `scan_commit` (T-077, D-179): the checked
//! repositories of a completed scan's answer become project rows in
//! one transaction, through the new-project rules `commands.rs` owns
//! (`ProjectFields`, `WriteDefaults`, `ColorOccupancy`,
//! `register_project`) and `project_add_manual` calls too.
//!
//! # The discovered-set rule (SEC-003, D-100's reserved rule)
//!
//! A path the UI sends back from a scan answer is neither a registered
//! project nor a folder picked in the dialog — SEC-003's two halves do
//! not cover it as worded, and D-100 reserved the rule for this
//! command. The rule: a COMPLETED scan publishes the `path_key` of
//! every repository it reported into the session-scoped
//! [`ScanFindings`] (replacing the previous completed scan's set; a
//! cancelled or failed scan publishes and clears nothing), and
//! `scan_commit` admits a path iff the key of its own text AND of its
//! canonical form are both in that set. The webview can therefore
//! commit exactly what Rust itself found under a root the gate
//! admitted — nothing under a picked tree that the walk skipped, and
//! nothing swapped in under a reported name. The set is in-memory
//! session bookkeeping (the `session_paths::SessionPaths` class), never
//! persisted; see [`scan_commit`] for the commit's own gate order.
//!
//! # The composition, in order
//!
//! 1. **The token publication — the single-step guard.** The fresh
//!    [`CancelToken`] is placed in the managed [`ScanCancelSlot`]
//!    BEFORE every other step (set-if-empty): a `scan_cancel`
//!    latched while the SEC-003 walk and the settings read are still
//!    running is honored on the walk's FIRST entry check — the same
//!    missed-cancel window D-147's pre-take publish closes for sync.
//!    A start that finds the slot occupied answers the
//!    [`E_SYNC_RUNNING` envelope](crate::commands::sync_running_refusal)
//!    (§10's "a second run — command rejected — wait until it
//!    finishes" row; the registry is closed at twenty codes, so no
//!    `E_SCAN_RUNNING` literal exists to invent) and publishes
//!    nothing. The slot IS the guard — there is no separate
//!    scan-running flag whose race a second publication step would
//!    reconcile (the D-147 two-step collapses to one because the walk
//!    follows the publication immediately); that is also why the
//!    refusal is honest even for a start whose path would fail
//!    validation anyway: with a scan live, "wait" is the answer
//!    whatever the input is (the [`reject_during_sync` ordering]
//!    reasoning, mirrored here at the slot).
//! 2. **SEC-003** — [`validate_ui_path`](crate::session_paths::
//!    validate_ui_path): the UI-named root must be a folder picked in
//!    the system dialog this session or a registered project; the
//!    check returns the CANONICAL form, which becomes the walk root —
//!    the [`scan_classify`](eunomia_fs::scan_classify) caller
//!    obligation (walked paths and registry keys must spell the
//!    identity the same way).
//! 3. **The per-scan snapshot** — ONE `Db::run` read transaction
//!    loads `scan.max_depth` (D-04, 1–20 → [`NonZeroUsize`]),
//!    `scan.exclude_names` (D-05) and the three registry key sets
//!    (active projects, archived projects, `project_exclusions`) —
//!    the deterministic-over-the-scan's-lifetime rule of
//!    [`ScanRegistry`](eunomia_fs::scan_classify::ScanRegistry):
//!    adds and archives committed mid-walk cannot flip a class.
//! 4. **The walk on the blocking pool** (SYS-001, the `sync_run`
//!    discipline): every walker event is classified outside any
//!    lock, then updates the shared [`WalkState`] behind one `std`
//!    mutex (the guard never crosses an await — D-037's reasoning;
//!    the critical section is a counter bump and, at most ten
//!    times a second, one synchronous event emit), and the throttled
//!    [`ScanProgressEvent`] fires from there.
//! 5. **The answer** — [`ScanOutcome::Completed`] maps to
//!    [`ScanRunOutcome::Completed`] with the accumulated sets and the
//!    terminal counters, and publishes the reported repositories'
//!    keys into [`ScanFindings`] — the discovered set `scan_commit`
//!    admits against (the rule below); [`ScanOutcome::Cancelled`]
//!    drops everything accumulated and answers the payload-less
//!    [`ScanRunOutcome::Cancelled`] (ERR-002 — an interrupted scan
//!    saves nothing, publishes nothing, and the wire shape cannot lie
//!    about it).
//!
//! # The throttle (SYS-003, ALG-053, NFR-015)
//!
//! At most ten emissions per second is ≥ 100 ms of spacing between
//! consecutive emissions ([`PROGRESS_MIN_INTERVAL`]), enforced by
//! [`ProgressTracker`]: every walk event updates the coalesced state
//! (the newest counts and path), and an emission fires only when no
//! emission happened in the previous 100 ms. The first event emits
//! immediately (the window opens at the walk's start), a burst of
//! thousands of events coalesces into the newest state, and while
//! events flow the UI sees a fresh state at least every ~100 ms.
//! There is deliberately NO timer-based and NO end-of-walk trailing
//! flush: a forced final emission can land < 100 ms after the
//! previous one — breaking the very bound this module exists to hold
//! — and the terminal state lands in the command's ANSWER, which
//! supersedes progress the moment the walk ends. The bound is stated
//! as the spec's own words and tested over a simulated clock (the
//! tracker takes the timestamp as a parameter, so no test sleeps on
//! real seconds; mutation — dropping the gate — turns the rate test
//! red).
//!
//! # The token lifecycle (the D-147 slot shape, one step)
//!
//! | moment | the slot holds |
//! | ------ | -------------- |
//! | no scan live (fresh app, between scans, recovery mode) | `None` |
//! | `scan_start_with` entered, before SEC-003 | the start's token (`try_publish`: set-if-empty) |
//! | a second start while one is live | refused `E_SYNC_RUNNING`; publishes nothing, clears nothing |
//! | the walk runs | the live scan's token; `scan_cancel` reads a clone and latches the shared flag |
//! | the scan answered — every exit: completed, cancelled, any `Err`, a panic's unwind | `None` again (the [`ScanLease`] RAII drop's `clear_if_current`) |
//! | the process dies mid-scan | the slot dies with it (in-memory state); no row exists to recover — the scan wrote nothing |
//!
//! The lease is the clearing half: held across the whole call,
//! dropped at the answer, on every `Err`'s early return and on a
//! panic's unwind; `clear_if_current` compares the `Arc` identity, so
//! a refused start that never published (and a lease whose scan lost
//! the slot race) clears nothing of the live scan's. The token is
//! [`eunomia_fs::scan::CancelToken`] — the walker's own primitive,
//! checked on every entry; the sync pipeline's token
//! (`commands::CancelToken`) stays a separate latch, uncoupled by
//! design.
//!
//! # What the counts mean (PRJ-002's window facts)
//!
//! `checked` counts every walk EVENT — a found repository, a plain
//! folder and a skipped folder are each one folder the walk stopped
//! at and made a decision about, and the window's counter climbs on
//! all of them; `found` counts repository events (all four classes —
//! the class is the registry's verdict on a FOUND path, and the
//! progress event reports the walk, not the classification);
//! `current_path` is the newest event's path. A non-UTF-8 skip's
//! path reaches the payload lossily (`to_string_lossy`) — the
//! consumer's rendering of a path that can never key, exactly the
//! split the walker's docs draw (the skip is data; the lossy spelling
//! is display, never identity).
//!
//! # Events and capabilities (the T-246 truth)
//!
//! `scan://progress` is a manual [`tauri_specta::Event`]
//! implementation (the derive macro cannot produce the `://` wire
//! name — the D-112 ruling for `state://dirty`), EMITTED FROM RUST
//! ONLY: the webview has exactly `core:event:allow-listen`/
//! `-unlisten` (T-246/D-116) and subscribes; emitting stays denied.
//! A refused emission is display-only degradation — logged through
//! the facade, never fatal to the walk (the answer carries the truth
//! the dropped event would have shown).
//!
//! [`reject_during_sync` ordering]: crate::commands::sync_run_with
//!
//! # Testing plan
//!
//! The pure halves are pinned here against a simulated clock: the
//! interval literal, the immediate first emission, the coalescing of
//! a burst into the newest state, and the rate bound — every sliding
//! second of a simulated 10-second walk (one event per simulated
//! millisecond) holds at most ten emissions, with the total pinned
//! to the arithmetic the interval dictates. The command surface is
//! driven over the mock runtime in `scan_tests.rs`: the classified
//! completed scan (nothing written anywhere — the ERR-002 twin),
//! the deterministic mid-walk cancel (the progress listener latches
//! the token from inside the walk, the D-147 "three deterministic
//! tests instead of one racy one" composition), the slot lifecycle,
//! the second-start refusal, SEC-003, the settings shaping the walk,
//! the honest no-database answer, and the registration pins. The
//! commit's suite (same file) pins the card's acceptance pair — a
//! repeated scan and commit create no duplicates; an error on one
//! project rolls back the whole batch — plus the discovered-set rule,
//! the manual-add rules over a batch, the typed outcomes, the
//! write-access warnings, the SYS-005 gate, and the missing-state
//! answers.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eunomia_fs::paths::NotARepoError;
use eunomia_fs::scan::CancelToken;
use eunomia_fs::scan_classify::{self, ClassifiedScanEvent, ScanClass, ScanRegistry};
use eunomia_store::projects::{AddedVia, Project, ProjectState};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use eunomia_store::action_log::{
    Action, Details, Entity, EntityType, Level, LogEntry, Outcome, Visibility, insert,
};

use crate::commands::{
    AddedProject, ColorOccupancy, ExistingProject, ProjectFields, Registered, WriteDefaults,
    activity_log_to_app_error, managed_db, probe_target_name, projects_store_to_app_error,
    register_project, reject_during_sync, rejected_field, repo_lookup_failed,
    settings_cell_to_app_error, sync_running_refusal, write_access_warning,
};
use crate::db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::file_watch;
use crate::logging;
use crate::session_paths;

/// The minimum spacing between two `scan://progress` emissions
/// (SYS-003, ALG-053: "no more than 10 times per second" — one tenth
/// of a second between consecutive emissions). Pinned by a unit test
/// against the literal, the `dirty_recalc::DEBOUNCE` pin shape.
pub(crate) const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------
// The wire DTOs (§7.10, PRJ-002's result groups)
// ---------------------------------------------------------------------------

/// One found repository with its ALG-053 facts, on the
/// `scan_start` wire: the path exactly as the walk reported it, its
/// `path_key`, and the registry's class. PRJ-002's three result
/// groups are the UI's mapping over this list — `new` feeds the
/// "New" checkboxes, `known` the "Already added" group, and
/// `archived`/`excluded` the "Skipped" group with the reason
/// "archived or in exclusions" (the class IS the reason for these
/// two); [`scan_commit`] consumes the user's selection from the same
/// list — the `path` strings, sent back and checked against
/// [`ScanFindings`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct ScannedRepository {
    /// The repository root exactly as the walk reported it (the
    /// canonical root's descendant spelling; a verbatim fact, never
    /// re-keyed here).
    pub(crate) path: String,
    /// The repository's registered identity
    /// ([`eunomia_fs::paths::path_key`]) — the string the store
    /// holds when the class is not `new`.
    pub(crate) path_key: String,
    /// The registry's verdict (ALG-053's four classes, serialized
    /// as the spec's own lower-case words).
    pub(crate) class: ScanClassWire,
}

/// ALG-053's class vocabulary on the wire, serialized as the spec's
/// own words (`new`, `known`, `archived`, `excluded`) — the log and
/// dialog spelling the classifier's `Display` already pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) enum ScanClassWire {
    /// Never added, never excluded: PRJ-002's "New" group.
    #[serde(rename = "new")]
    New,
    /// An active project: PRJ-002's "Already added" group.
    #[serde(rename = "known")]
    Known,
    /// An archived project: "Skipped" with the restore offer.
    #[serde(rename = "archived")]
    Archived,
    /// In `project_exclusions` with no project row: "Skipped".
    #[serde(rename = "excluded")]
    Excluded,
}

impl From<ScanClass> for ScanClassWire {
    fn from(class: ScanClass) -> Self {
        match class {
            ScanClass::New => ScanClassWire::New,
            ScanClass::Known => ScanClassWire::Known,
            ScanClass::Archived => ScanClassWire::Archived,
            ScanClass::Excluded => ScanClassWire::Excluded,
        }
    }
}

/// One entry of the scan answer's "Skipped" group (PRJ-002): the
/// path, the reason vocabulary, and — for the access refusals — the
/// OS code the UI's fix-it hint quotes ("os error 13 — permission
/// denied; fix the rights and rescan").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct ScanSkip {
    /// The skipped entry's path (lossily rendered only for the
    /// non-UTF-8 skips, whose bytes no wire string can hold).
    pub(crate) path: String,
    /// Why the scan skipped it.
    pub(crate) reason: ScanSkipReason,
    /// The raw OS error code behind an access skip; `None` for the
    /// reasons that are facts about a name or the text (§10's
    /// first-class log fact, riding the wire for the hint).
    pub(crate) os_code: Option<i32>,
}

/// The skip-reason vocabulary on the wire (PRJ-002's list): the
/// walker's four reasons plus nothing else — the "archived or in
/// exclusions" reason is a repository's [`ScanClassWire`], carried
/// on the repository rows, not a skip row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) enum ScanSkipReason {
    /// The folder's name is excluded (the settings list or the
    /// ALG-051 system folders); not descended into.
    #[serde(rename = "excluded")]
    Excluded,
    /// `.git` is a file or link — the submodule/worktree form; the
    /// user can add it by hand (D-06).
    #[serde(rename = "submodule_or_worktree")]
    SubmoduleOrWorktree,
    /// The path's text is not valid UTF-8 (the D-086 addendum); the
    /// subtree under it is unregistrable.
    #[serde(rename = "not_utf8")]
    NotUtf8,
    /// The OS refused to inspect the folder (PRJ-007); the walk
    /// continued with the siblings. The OS code rides
    /// [`ScanSkip::os_code`].
    #[serde(rename = "access")]
    Access,
}

impl ScanSkip {
    /// Builds the wire row from the walker's skip facts.
    fn from_walk(path: &Path, reason: &eunomia_fs::scan::SkipReason) -> Self {
        use eunomia_fs::scan::SkipReason;
        let (reason, os_code) = match reason {
            SkipReason::Excluded => (ScanSkipReason::Excluded, None),
            SkipReason::SubmoduleOrWorktree => (ScanSkipReason::SubmoduleOrWorktree, None),
            SkipReason::NotUtf8 => (ScanSkipReason::NotUtf8, None),
            SkipReason::Access { source } => (ScanSkipReason::Access, source.raw_os_error()),
        };
        Self {
            path: path.to_string_lossy().into_owned(),
            reason,
            os_code,
        }
    }
}

/// The `scan_start` command's wire answer (§7.10, ERR-002): either
/// the completed scan's candidate set, or — payload-less, the
/// structural pin of "an interrupted scan saves nothing" — the
/// cancelled verdict. Cancellation is NOT an `Err`: the user asked
/// for it (the `sync_run_aborted` posture), and an error envelope
/// would imply a failure where a decision happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub(crate) enum ScanRunOutcome {
    /// The walk ran to completion: the terminal counters (the same
    /// facts the progress event carried, at their final values) and
    /// the classified result the UI groups into New / Already added
    /// / Skipped. Nothing is persisted with them — [`scan_commit`] is
    /// the only writer.
    Completed {
        /// Folders the walk made a decision about (found, plain and
        /// skipped alike — the window's "folders checked" counter's
        /// final value).
        checked: u32,
        /// Repository events delivered, all classes combined.
        found: u32,
        /// Every found repository with its key and class, in walk
        /// delivery order (unspecified beyond the parent-before-
        /// child guarantee the walker pins; the UI sorts for
        /// display).
        repositories: Vec<ScannedRepository>,
        /// The walker's skips with their reasons.
        skipped: Vec<ScanSkip>,
    },
    /// The cancel token latched and a walker thread observed it:
    /// everything the walk accumulated is dropped, nothing was
    /// written anywhere, and the answer deliberately carries no
    /// partial anything (ERR-002).
    Cancelled,
}

/// The `scan_commit` answer (§7.10, PRJ-002's "Add selected"; T-077,
/// D-179, D-196): one entry per DISTINCT repository of the selection,
/// in the order the UI sent — which is also the order the rows
/// appended in. Only failures are `Err`: the entries are the
/// per-repository facts of a committed transaction (or of a selection
/// with nothing to add).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ScanCommitResult {
    /// The selection's verdicts, sent order, duplicates collapsed.
    pub(crate) entries: Vec<ScanCommitEntry>,
    /// The typed partial success (D-196): `true` when the rows ARE
    /// committed but the post-commit §6.9 write-access probe did not
    /// run — its blocking task failed to join. The `"added"` entries'
    /// `write_access_warning` is then `None` because UNCHECKED, not
    /// because writable; the failure is logged once, class-only.
    /// How the UI presents the two live kinds of answers is the
    /// deferred UI-002 design.
    pub(crate) probe_failed: bool,
}

/// One selected repository's verdict — the `ProjectAddResult`
/// discriminant shape: a closed outcome string plus the one payload
/// arm it carries (the house form for "one response, several
/// meanings").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ScanCommitEntry {
    /// The repository's canonical path (DB-030) — for `"added"`, the
    /// stored `projects.path`.
    pub(crate) path: String,
    /// What became of the repository, one of `"added"`,
    /// `"already_added"`, `"archived"` — a commit over an excluded
    /// path lifts the exclusion and answers `"added"` (D-196), so
    /// `"excluded"` can no longer occur.
    pub(crate) outcome: String,
    /// The created row with its §6.9 write-access warning — the
    /// `"added"` outcome only.
    pub(crate) project: Option<AddedProject>,
    /// The project row that already holds the path (DB-015) — the
    /// `"already_added"` and `"archived"` outcomes; for the latter the
    /// id is what the restore offer calls back with (PRJ-013…018).
    pub(crate) existing: Option<ExistingProject>,
}

/// The `scan://progress` event payload (§7.10, SYS-003, PRJ-002's
/// scan-window facts): the three live numbers the window renders
/// while the walk runs.
///
/// The event name is the spec's literal wire name, pinned by the
/// manual [`Event`] implementation (the D-112 ruling: the derive
/// macro cannot produce the `://` spelling). Emitted from Rust only;
/// the webview subscribes through the T-246 listen/unlisten grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct ScanProgressEvent {
    /// Folders the walk has made a decision about so far.
    pub(crate) checked: u32,
    /// Repositories found so far (all classes — the class is the
    /// registry's verdict, reported in the answer, not the walk).
    pub(crate) found: u32,
    /// The newest visited path (the walk's current position;
    /// lossily rendered only for non-UTF-8 skips).
    pub(crate) current_path: String,
}

impl Event for ScanProgressEvent {
    /// The §7.10 wire name, verbatim. Tauri permits exactly the
    /// characters it contains (alphanumeric, `-`, `/`, `:`, `_`).
    const NAME: &'static str = "scan://progress";
}

// ---------------------------------------------------------------------------
// The progress pipeline: counters + the throttle (SYS-003, ALG-053)
// ---------------------------------------------------------------------------

/// The per-event progress pipeline: the counters the window renders
/// and the coalescing throttle that bounds their emissions.
///
/// Pure over an injected timestamp — [`ProgressTracker::on_event`]
/// takes `now` as a parameter, so the rate bound is testable over a
/// simulated clock (no test sleeps on real seconds), the
/// `dirty_recalc::DEBOUNCE` testing posture. The caller (the walk
/// callback) supplies `Instant::now` in production.
///
/// The coalescing rule, stated once: every event updates
/// [`Self::checked`]/[`Self::found`]/[`Self::current_path`] — the
/// newest state is ALWAYS what the next emission carries — and an
/// emission fires iff no emission happened in the previous
/// `min_interval` (the first event emits immediately). See the
/// module docs for why there is no end-of-walk flush: the terminal
/// state lands in the command's answer.
#[derive(Debug)]
struct ProgressTracker {
    /// The minimum spacing between emissions ([`PROGRESS_MIN_INTERVAL`]).
    min_interval: Duration,
    /// Folders decided so far (every walk event).
    checked: u32,
    /// Repository events so far.
    found: u32,
    /// The newest visited path (the coalesced display state).
    current_path: String,
    /// When the last emission fired; `None` until the first.
    last_emit: Option<Instant>,
}

impl ProgressTracker {
    /// The tracker over `min_interval` ([`PROGRESS_MIN_INTERVAL`] in
    /// production; any window the tests want).
    fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            checked: 0,
            found: 0,
            current_path: String::new(),
            last_emit: None,
        }
    }

    /// Records one walk event at `path` (`repository` telling the
    /// two counters apart) and decides the emission: `Some(payload)`
    /// iff this event's state may reach the wire now — the first
    /// event, or the first after a full `min_interval` since the
    /// last emission. The payload is built from the state AFTER this
    /// event was recorded (coalescing: a burst's newest state is
    /// what lands, never a stale intermediate).
    fn on_event(
        &mut self,
        path: &Path,
        repository: bool,
        now: Instant,
    ) -> Option<ScanProgressEvent> {
        self.checked = self.checked.saturating_add(1);
        if repository {
            self.found = self.found.saturating_add(1);
        }
        self.current_path = path.to_string_lossy().into_owned();
        let may_emit = match self.last_emit {
            None => true,
            Some(last) => now.duration_since(last) >= self.min_interval,
        };
        if !may_emit {
            return None;
        }
        self.last_emit = Some(now);
        Some(ScanProgressEvent {
            checked: self.checked,
            found: self.found,
            current_path: self.current_path.clone(),
        })
    }
}

/// Everything the walk callback accumulates behind one lock: the
/// progress pipeline (counters + throttle) and the answer's two
/// lists. The mutex guard lives for one event's classification and
/// its (at most ten-per-second) emit — never across an await
/// (D-037's std-mutex reasoning: there is no await inside a walk
/// callback at all).
struct WalkState {
    /// The counters and the throttle.
    progress: ProgressTracker,
    /// The classified repositories (the answer's payload half).
    repositories: Vec<ScannedRepository>,
    /// The walker's skips (the answer's other half).
    skipped: Vec<ScanSkip>,
}

// ---------------------------------------------------------------------------
// The cancel slot and its lease (the D-147 shape, one step)
// ---------------------------------------------------------------------------

/// The managed slot publishing the LIVE scan's cancel token (the
/// [`crate::commands::SyncCancelSlot`] shape, deliberately a
/// separate type and latch — sync and scan cancel through their own
/// flags and never block each other): `scan_start_with` publishes
/// the token it drives the walk with before anything else, and
/// [`scan_cancel`](self::scan_cancel) reads it back. Managed by
/// `setup` in every mode (recovery included) for the same reason the
/// sync slot is: the cancel command must FIND the state, and in
/// recovery mode it simply stays empty (a scan refuses at the
/// settings read — no database).
///
/// The slot holds the token behind an [`Arc`] so identity — not
/// value equality — decides the clearing: [`ScanCancelSlot::
/// clear_if_current`] compares the `Arc` pointer, and no scan's exit
/// can unpublish another's token.
#[derive(Debug, Default)]
pub(crate) struct ScanCancelSlot {
    /// `std::sync::Mutex` — the guard never crosses an await point
    /// (each method body is one lock); `None` means no scan is live.
    token: Mutex<Option<Arc<CancelToken>>>,
}

impl ScanCancelSlot {
    /// An empty slot — the state `setup` manages before anything
    /// that could fail.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Publishes `token` only when the slot is EMPTY, answering
    /// whether THIS caller won it: the single-step guard — the live
    /// scan's publisher wins, a concurrent second start is refused
    /// by the caller, and nothing here can evict a live scan's
    /// token (D-147's set-if-empty law, the sync slot's `publish`).
    pub(crate) fn try_publish(&self, token: &Arc<CancelToken>) -> bool {
        let mut slot = self
            .token
            .lock()
            .expect("the scan cancel slot must not be poisoned");
        if slot.is_none() {
            *slot = Some(Arc::clone(token));
            true
        } else {
            false
        }
    }

    /// The live scan's token, if one is published — the
    /// `scan_cancel` command's read. Cancelling the clone IS
    /// cancelling the walk's flag (one `Arc<AtomicBool>` latches
    /// both spellings); `None` answers the command's idempotent
    /// no-op.
    pub(crate) fn token(&self) -> Option<Arc<CancelToken>> {
        self.token
            .lock()
            .expect("the scan cancel slot must not be poisoned")
            .clone()
    }

    /// Clears the slot only while it still holds exactly `token` —
    /// a lease whose scan lost the slot race clears nothing, so no
    /// scan's exit unpublishes another's (the `Arc::ptr_eq` identity
    /// rule of D-147's `clear_if_current`).
    fn clear_if_current(&self, token: &Arc<CancelToken>) {
        let mut slot = self
            .token
            .lock()
            .expect("the scan cancel slot must not be poisoned");
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, token))
        {
            *slot = None;
        }
    }
}

/// The publication's RAII other half (the [`CancelLease`]/`SyncGuard`
/// shape, D-123/D-147): clearing the slot on EVERY exit of
/// `scan_start_with` — held across the call, dropped at the answer;
/// an `Err`'s early return and a panic's unwind drop it the same
/// way. The drop's state read is the non-panicking `try_state` on
/// purpose: publication proved the state exists, and a `Drop` must
/// not turn a wiring bug into an abort mid-unwind.
struct ScanLease<R: Runtime> {
    app: AppHandle<R>,
    token: Arc<CancelToken>,
}

impl<R: Runtime> Drop for ScanLease<R> {
    fn drop(&mut self) {
        if let Some(slot) = self.app.try_state::<ScanCancelSlot>() {
            slot.clear_if_current(&self.token);
        }
    }
}

// ---------------------------------------------------------------------------
// The discovered set (SEC-003's rule for `scan_commit`, D-100/D-179)
// ---------------------------------------------------------------------------

/// The repositories the session's last COMPLETED scan reported, by
/// their DB-030 `path_key` — the only paths [`scan_commit`] admits.
///
/// A scan ANSWER's paths are descendants of the picked root, so
/// SEC-003's two halves ("a registered project or a folder picked in
/// the dialog this session") do not cover them as worded; D-100
/// reserved the rule for the commit and D-179 fixed it: what Rust
/// itself found under a gate-admitted root is what the commit may
/// register. [`scan_start_with`] publishes a completed walk's
/// repositories here — all four ALG-053 classes, because what the
/// commit does with an already-registered or excluded one is the
/// transaction's typed answer, not a "never scanned" rejection — and
/// each publication REPLACES the previous set: the set is the last
/// result view's universe, bounded by what that walk found, never a
/// session-long union. A cancelled or failed scan publishes nothing
/// and clears nothing (ERR-002).
///
/// Managed by `setup` in every mode beside the cancel slot. In-memory
/// session bookkeeping of the [`session_paths::SessionPaths`] class —
/// never persisted, and nothing here is a project: the commit's
/// transaction is the only writer. A poisoned lock is recovered
/// (`into_inner`): the set is plain data that survives a panicking
/// caller, exactly the allowlist's posture.
#[derive(Debug, Default)]
pub(crate) struct ScanFindings {
    /// `std::sync::Mutex` — each method holds the guard for one set
    /// swap or lookup, never across an await.
    keys: Mutex<HashSet<String>>,
}

impl ScanFindings {
    /// The empty set — the state `setup` manages before anything that
    /// could fail (nothing was scanned yet: the commit admits nothing).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Makes `keys` the discovered set — the path keys of one
    /// completed scan's reported repositories, replacing whatever the
    /// previous completed scan left.
    pub(crate) fn publish(&self, keys: impl IntoIterator<Item = String>) {
        *self.locked() = keys.into_iter().collect();
    }

    /// Whether `key` is the DB-030 key of a repository the last
    /// completed scan reported.
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.locked().contains(key)
    }

    /// Whether `path` is a repository the last completed scan
    /// reported: its DB-030 key — the one identity rule the session
    /// gate and the classifier key by — is in the set. A path whose
    /// text cannot key simply matches nothing.
    pub(crate) fn admits(&self, path: &Path) -> bool {
        eunomia_fs::paths::path_key(path).is_ok_and(|key| self.contains(&key))
    }

    /// How many repositories the last completed scan reported — the
    /// size the tests assert; no production code reads it (the
    /// `SessionPaths::len` shape).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.locked().len()
    }

    /// The poison-recovering lock of the methods above.
    fn locked(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ---------------------------------------------------------------------------
// The commands (§7.10 Projects module)
// ---------------------------------------------------------------------------

/// Serves PRJ-002's "Scan folder" (§7.10 `scan_start`): validates
/// the root through SEC-003, snapshots the settings and the
/// classification registry, walks the tree on the blocking pool with
/// the throttled `scan://progress` events, and answers the found
/// candidate set — or the payload-less cancelled verdict. Every
/// failure path flows through the ONE logging edge
/// ([`logging::scan_start_failed`], the D-096 law).
#[tauri::command]
#[specta::specta]
pub(crate) async fn scan_start<R: Runtime>(
    app: AppHandle<R>,
    path: String,
) -> Result<ScanRunOutcome, AppError> {
    scan_start_with(app, path, Arc::new(CancelToken::new()))
        .await
        .inspect_err(logging::scan_start_failed)
}

/// The body of [`scan_start`], split the `dialog_pick_folder_with`
/// way so the one logging edge covers every failure path and the
/// tests drive the walk with their own cancel token (the injected
/// token is the cancel seam — a pre-latched token answers
/// `Cancelled` deterministically, the walker's own pinned behavior).
///
/// The order of the steps is the module docs' composition: publish
/// first (the missed-cancel window), SEC-003 second, the snapshot
/// third, the walk fourth. See [`ScanRunOutcome`] for the answer and
/// [`ProgressTracker`] for the throttle.
pub(crate) async fn scan_start_with<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    cancel: Arc<CancelToken>,
) -> Result<ScanRunOutcome, AppError> {
    // Step 1 — the token publication, the single-step guard. A
    // refused start publishes nothing and clears nothing; the lease
    // below holds the clearing duty for every exit that follows.
    if !app.state::<ScanCancelSlot>().try_publish(&cancel) {
        return Err(sync_running_refusal());
    }
    let _lease = ScanLease {
        app: app.clone(),
        token: Arc::clone(&cancel),
    };

    // Step 2 — SEC-003: the UI-named root must be allowlisted, and
    // the canonical form it answers is the walk root (the
    // classification caller obligation).
    let root = session_paths::validate_ui_path(&app, &path).await?;

    // Step 3 — the per-scan snapshot: settings + registry in ONE
    // read transaction (deterministic classification over the
    // scan's lifetime).
    let (options, registry) = load_scan_inputs(&app).await?;

    // Step 4 — the walk on the blocking pool (SYS-001): the
    // callback runs on the walker's threads, so everything it
    // touches lives behind the one lock and the `AppHandle` clone.
    let walk_state = Arc::new(Mutex::new(WalkState {
        progress: ProgressTracker::new(PROGRESS_MIN_INTERVAL),
        repositories: Vec::new(),
        skipped: Vec::new(),
    }));
    let callback_state = Arc::clone(&walk_state);
    let callback_app = app.clone();
    let registry = Arc::new(registry);
    let callback_registry = Arc::clone(&registry);
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        eunomia_fs::scan::scan_repositories(&root, &options, &cancel, |event| {
            let classified = scan_classify::classify_entry(event, &callback_registry);
            let mut state = callback_state
                .lock()
                .expect("the scan walk state must not be poisoned");
            let payload = match &classified {
                Ok(ClassifiedScanEvent::Repository(found)) => {
                    state.repositories.push(ScannedRepository {
                        path: found.path().to_string_lossy().into_owned(),
                        path_key: found.path_key().to_owned(),
                        class: found.class().into(),
                    });
                    state.progress.on_event(event.path(), true, Instant::now())
                }
                Ok(ClassifiedScanEvent::Directory { .. }) => {
                    state.progress.on_event(event.path(), false, Instant::now())
                }
                Ok(ClassifiedScanEvent::Skipped { path, reason }) => {
                    state.skipped.push(ScanSkip::from_walk(path, reason));
                    state.progress.on_event(event.path(), false, Instant::now())
                }
                // A repository event whose path text cannot key —
                // unreachable for WALK-driven events (the walker's
                // NotUtf8 gate fires one step earlier, pinned by the
                // classifier's own test), honest for hand-built
                // inputs: reported as the nearest skip fact instead
                // of an unwrap (the signature's `Result` exists for
                // exactly this, the `classify_entry` docs' rule).
                Err(source) => {
                    state.skipped.push(ScanSkip::from_walk(
                        Path::new(source.path()),
                        &eunomia_fs::scan::SkipReason::NotUtf8,
                    ));
                    state.progress.on_event(event.path(), false, Instant::now())
                }
            };
            if let Some(payload) = payload {
                // Display-only: a refused emission never stops the
                // walk — the answer carries the truth this event
                // would have previewed (the availability posture).
                if let Err(error) = payload.emit(&callback_app) {
                    logging::scan_progress_emit_failed(&error);
                }
            }
        })
    })
    .await
    .map_err(db::join_failure_to_app_error)?;

    // Step 5 — the answer. `Cancelled` drops everything the walk
    // accumulated (ERR-002): the arm carries no payload to show and
    // publishes nothing. `Completed` answers the accumulated sets
    // and publishes their keys as the discovered set `scan_commit`
    // admits against (the module docs' rule) — after the walk lock
    // is released, so the state lookup never runs under it.
    let mut walk = walk_state
        .lock()
        .expect("the scan walk state must not be poisoned");
    let answer = match outcome {
        eunomia_fs::scan::ScanOutcome::Completed => ScanRunOutcome::Completed {
            checked: walk.progress.checked,
            found: walk.progress.found,
            repositories: std::mem::take(&mut walk.repositories),
            skipped: std::mem::take(&mut walk.skipped),
        },
        eunomia_fs::scan::ScanOutcome::Cancelled => {
            logging::scan_cancelled(walk.progress.checked);
            drop(walk);
            return Ok(ScanRunOutcome::Cancelled);
        }
    };
    drop(walk);
    publish_findings(&app, &answer)?;
    Ok(answer)
}

/// The discovered-set publication of a completed scan (D-179): the
/// reported repositories' keys replace [`ScanFindings`]. The missing
/// state is the honest fallback (the `scan_slot_missing` shape): the
/// state is managed by `setup` before anything that could fail, so its
/// absence is an uninitialized-app bug — a refusal naming it, not a
/// completed scan whose commit could only ever answer "not reported".
fn publish_findings<R: Runtime>(
    app: &AppHandle<R>,
    answer: &ScanRunOutcome,
) -> Result<(), AppError> {
    if let ScanRunOutcome::Completed { repositories, .. } = answer {
        managed_findings(app)?.publish(repositories.iter().map(|repo| repo.path_key.clone()));
    }
    Ok(())
}

/// Serves PRJ-002's "Cancel" and NFR-015's "closing the scan window
/// means cancel" (§7.10 `scan_cancel`): latches the live scan's
/// token, which the walk observes on its next entry check and
/// answers with [`ScanRunOutcome::Cancelled`] — nothing persisted.
///
/// Idempotent by design: with no scan live the command is the
/// documented no-op `Ok(())` — the window-close race (the scan
/// finishing between the user's click and this call's read) must not
/// error in the loser branch, and the postcondition "no scan keeps
/// running" holds either way.
#[tauri::command]
#[specta::specta]
pub(crate) async fn scan_cancel<R: Runtime>(app: AppHandle<R>) -> Result<(), AppError> {
    // The missing-state fallback (the `managed_session` shape): the
    // state is managed by `setup` before anything that could fail,
    // so its absence is an uninitialized-app bug — the recorded
    // D-037 fallback with a diagnostic naming it, not a panic, and
    // not a silent Ok that would hide the wiring gap.
    let Some(slot) = app.try_state::<ScanCancelSlot>() else {
        return Err(scan_slot_missing());
    };
    if let Some(token) = slot.token() {
        token.cancel();
    }
    Ok(())
}

/// The honest missing-state answer of [`scan_cancel`] (the
/// `managed_session` shape): the D-037 fallback with the class-only
/// diagnostic naming the state.
fn scan_slot_missing() -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("ScanCancelSlotMissing".to_owned()),
        os_message: Some("the scan cancel slot was not managed at startup".to_owned()),
    });
    error
}

/// Serves PRJ-002's "Add selected" (§7.10 `scan_commit`, T-077, D-179):
/// the checked "New" repositories of a scan answer become projects in
/// ONE transaction, with the rules of
/// [`project_add_manual`](crate::commands::project_add_manual) and
/// without duplicates.
///
/// `paths` are the repository paths the `scan_start` answer carried,
/// sent back by the result view. The command's first act is the SYS-005
/// sync gate — while a sync holds `sync_running`, the answer is the
/// typed `E_SYNC_RUNNING` refusal before anything else runs. With the
/// flag down, the database is resolved (recovery mode answers its
/// honest fallback before any probe), and then the gates run in this
/// order; the FIRST failure in the order sent is the whole command's
/// answer, and nothing is written:
///
/// 1. **SEC-003, the discovered-set rule.** A path is admitted iff the
///    DB-030 key of its OWN text is in the session's
///    [`ScanFindings`] — the keys of every repository the last
///    COMPLETED `scan_start` reported. Anything else, and an empty
///    entry, is the typed `E_VALIDATION` rejection of the `path`
///    field, raised BEFORE any disk probe: the webview can only commit
///    what Rust itself found under a root the SEC-003 gate admitted.
/// 2. **Canonicalization** (blocking pool): a folder that vanished
///    since the scan is §10's `E_PROJECT_UNAVAILABLE` with the path
///    and the OS facts ("could not check" is not "not a repo", D-090);
///    a non-UTF-8 canonical form is the D-086 encoding rejection. The
///    CANONICAL form's key must be in the findings too — a folder
///    swapped in under a reported name resolves elsewhere and meets
///    the same SEC-003 rejection. The selection is collapsed twice —
///    first by the sent text's key (step 1: duplicates cost nothing
///    downstream), then by the resolved identity (two spellings can
///    lead to one folder) — the first occurrence's position kept both
///    times, which is the append order.
/// 3. **The repository root** (blocking pool):
///    [`find_git_root`](eunomia_fs::paths::find_git_root) must answer
///    the folder ITSELF (by key) — a folder whose own `.git` is gone
///    is `E_NOT_A_REPO`, §10's row and the manual add's envelope; a
///    refused probe is `E_PROJECT_UNAVAILABLE`.
/// 4. **The §6.9 fields** of every candidate, through the shared
///    [`ProjectFields::derive`] (the manual add's "Fields" step). A
///    rejection names the field AND carries the repository's `path`
///    param, so the result view can mark the row.
/// 5. **One transaction** (D-037, §6.9 "the whole operation runs in
///    one transaction"), each candidate in sent order: the DB-015
///    duplicate check (`find_by_path_key` — an active row answers the
///    typed `"already_added"` entry, an archived row the `"archived"`
///    one carrying the id of the DB-015 restore offer); the exclusion
///    check (a `project_exclusions` row with no project has its
///    exclusion LIFTED — `delete_exclusion` plus ONE `exclusion.removed`
///    journal entry, the D-119 shape attributed to the created row —
///    and the candidate registers like any other, answering `"added"`
///    (D-196): the checked "Skipped" row's way back into the registry,
///    which the manual re-add never offers); otherwise
///    [`register_project`] with `added_via = scan` — the PRJ-012
///    copy, D-29's color over the active set as it stands INSIDE the
///    transaction (the batch's earlier rows count, so twelve rows
///    take twelve colors), `create`, and one `project.added` journal
///    entry (LOG-001…004, info/user/ok). The typed entries are
///    answers about the selection, not errors: the other
///    repositories are still added, and a re-sent selection
///    converges. Any `Err` — a store or journal failure on ANY row —
///    rolls back every row of the batch with its entries: an error on
///    one project refuses the whole commit.
/// 6. **Write-access warnings** (§6.9, D-094): after the commit, one
///    [`check_write_access`](eunomia_fs::write_access::
///    check_write_access) per added row on the blocking pool — never
///    inside the transaction (the D-096 probe-order rule) — riding
///    the entry's `project` exactly as in the manual add. A join
///    failure of that task is NOT an error (D-196): the rows are
///    committed, so the command answers the typed partial success
///    (`probe_failed`, the verdicts absent) and logs the failure once
///    through [`logging::scan_commit_probe_degraded`].
/// 7. **The watch refresh** (§5.7 SYS-007, D-144): when at least one
///    row was created — whatever the probe half answered — ONE pass
///    through
///    [`refresh_after_mutation`](crate::file_watch::
///    refresh_after_mutation) — never fatal to the answer; the DB-022
///    recalculation rides the commit hook.
///
/// An empty selection is the typed no-op `entries: []`, `probe_failed`
/// false — nothing is written and nothing journaled. A live scan does
/// NOT refuse the commit: the walk's registry snapshot predates it by
/// design, so its answer may show a just-added repository as `new`,
/// and the duplicate check inside the transaction keeps that harmless.
///
/// Failures reach the log with their §10 code through the ONE edge
/// [`logging::scan_commit_failed`]; the typed outcomes and the empty
/// selection log nothing (the probe's degraded half has its own edge,
/// D-196), and a successful commit is its own journal entries (the
/// D-051 mirror policy).
#[tauri::command]
#[specta::specta]
pub(crate) async fn scan_commit<R: Runtime>(
    app: AppHandle<R>,
    paths: Vec<String>,
) -> Result<ScanCommitResult, AppError> {
    scan_commit_impl(app, paths)
        .await
        .inspect_err(logging::scan_commit_failed)
}

/// The body of [`scan_commit`], split the `scan_start_with` way so
/// the one logging edge covers every failure path and the tests can
/// drive the body directly. The gates follow the command docs' order.
///
/// Complexity over untrusted input (N sent paths of at most L bytes):
/// the text gate and its dedupe are O(N·L) and touch no disk; disk
/// work — canonicalization, the `.git` probes, the transaction, the
/// write-access probes — runs only over the paths that survive it,
/// at most one per key the last scan reported (R), never one per
/// sent path: O(R) probes and statements, the color rule linear in R
/// (D-29 over relative counts).
pub(crate) async fn scan_commit_impl<R: Runtime>(
    app: AppHandle<R>,
    paths: Vec<String>,
) -> Result<ScanCommitResult, AppError> {
    // Gate 1 — SYS-005 (§5.4): the mode gate runs before everything —
    // while a sync holds `sync_running`, the commit is refused with
    // `E_SYNC_RUNNING` whatever the input is (it creates project
    // rows, the rows the sync's snapshot was built from).
    reject_during_sync(&app)?;
    // The database every write needs, resolved BEFORE any probe, so
    // recovery mode answers its honest fallback consistently.
    let database = managed_db(&app)?;
    // Gate 2 — SEC-003, the text half of the discovered-set rule: no
    // disk probe for a path the last scan did not report. The same
    // pass collapses the in-request duplicates by the sent text's key,
    // so everything downstream — the probes, the transaction — is
    // bounded by the distinct REPORTED paths (R), never by what the
    // webview sent (N): every admitted key is in the findings, so at
    // most |findings| paths survive. A text that cannot key matches
    // nothing (the D-086 addendum's rule at this seam).
    let findings = managed_findings(&app)?;
    let mut admitted: Vec<String> = Vec::with_capacity(paths.len());
    let mut seen_text: HashSet<String> = HashSet::with_capacity(paths.len());
    for path in &paths {
        if path.trim().is_empty() {
            return Err(rejected_field(
                "path",
                path,
                "the repository path must not be empty",
            ));
        }
        match eunomia_fs::paths::path_key(Path::new(path)) {
            Ok(key) if findings.contains(&key) => {
                if seen_text.insert(key) {
                    admitted.push(path.clone());
                }
            }
            _ => return Err(rejected_unreported_path(path)),
        }
    }
    // Gate 3 — canonicalization (SYS-001: a dead mount must not stall
    // an async worker). The D-086 addendum seam is `canonicalize`'s
    // own gate: a non-UTF-8 form answers the encoding rejection, any
    // other refusal the availability error — the manual add's two
    // mappings over the same typed failure.
    let canonical_forms = tauri::async_runtime::spawn_blocking(move || {
        admitted
            .iter()
            .map(|text| eunomia_fs::paths::canonicalize(Path::new(text)))
            .collect::<Result<Vec<PathBuf>, _>>()
    })
    .await
    .map_err(db::join_failure_to_app_error)?
    .map_err(|source| repo_lookup_failed(&NotARepoError::Canonicalize { source }))?;
    // The canonical-form half of the rule, then the second collapse —
    // by the RESOLVED identity: two admitted spellings (a symlinked
    // name inside the root, a case-folded sibling) can lead to one
    // folder. Sent order throughout, which is the append order.
    let mut seen: HashSet<String> = HashSet::with_capacity(canonical_forms.len());
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(canonical_forms.len());
    for canonical in canonical_forms {
        if !findings.admits(&canonical) {
            return Err(rejected_unreported_path(&canonical.to_string_lossy()));
        }
        let key = eunomia_fs::paths::path_key(&canonical)
            .map_err(|source| crate::commands::rejected_non_utf8_path(source.path()))?;
        if seen.insert(key) {
            candidates.push(canonical);
        }
    }
    // Gate 4 — the repository root: the folder itself, by the one
    // identity rule (a folder whose own `.git` is gone resolves to an
    // ancestor's root, or to none — `E_NOT_A_REPO` either way).
    let walked = candidates.clone();
    let root_verdicts = tauri::async_runtime::spawn_blocking(move || {
        walked
            .iter()
            .map(
                |canonical| match eunomia_fs::paths::find_git_root(canonical) {
                    Ok(root) if is_own_root(&root, canonical) => Ok(()),
                    Ok(_) => Err(NotARepoError::NoGitEntry {
                        path: canonical.to_string_lossy().into_owned(),
                    }),
                    Err(source) => Err(source),
                },
            )
            .collect::<Vec<Result<(), NotARepoError>>>()
    })
    .await
    .map_err(db::join_failure_to_app_error)?;
    for verdict in root_verdicts {
        verdict.map_err(|source| repo_lookup_failed(&source))?;
    }
    // Gate 5 — the §6.9 fields of every candidate, before anything is
    // written: a rejection names the field and the repository.
    let fields: Vec<ProjectFields> = candidates
        .iter()
        .map(|root| ProjectFields::derive(root).map_err(|error| field_rejection_of(root, error)))
        .collect::<Result<_, _>>()?;

    // The transaction: every row of the batch, its journal entries and
    // the typed outcomes in one `Db::run` (D-037). Everything the
    // closure reads it reads inside the transaction, so a busy retry
    // replays it over the committed state.
    let slots = database
        .run(move |transaction| {
            let defaults = WriteDefaults::load(transaction)?;
            let mut occupancy = ColorOccupancy::load(transaction)?;
            let mut slots: Vec<CommitSlot> = Vec::with_capacity(fields.len());
            for candidate in &fields {
                // DB-015: the key is unique across active AND archived
                // rows — the same single-connection serialization the
                // manual add relies on (D-037).
                if let Some(existing) =
                    eunomia_store::projects::find_by_path_key(transaction, &candidate.path_key)
                        .map_err(projects_store_to_app_error)?
                {
                    slots.push(CommitSlot::Entry(duplicate_entry(
                        &candidate.path,
                        &existing,
                    )));
                    continue;
                }
                if let Some(exclusion) = eunomia_store::projects::find_exclusion_by_path_key(
                    transaction,
                    &candidate.path_key,
                )
                .map_err(projects_store_to_app_error)?
                {
                    // D-196: the commit over an excluded path LIFTS the
                    // exclusion and registers the project — the checked
                    // "Skipped" row's way back into the registry, which
                    // the manual re-add never offers. The registered
                    // project carries this branch: the row the scan
                    // showed under "Skipped" answers `"added"` like any
                    // other created row.
                    eunomia_store::projects::delete_exclusion(transaction, &exclusion.id)
                        .map_err(projects_store_to_app_error)?;
                    // The lift's journal entry, inside the same
                    // transaction (D-051): the D-119 shape (entity =
                    // the removed row, `File` detail = its stored path)
                    // with one recorded deviation — `project_id` names
                    // the row created below, because here a project row
                    // IS involved and the Activity window's project
                    // filter attributes the lift to it.
                    let registered = register_project(
                        transaction,
                        candidate,
                        AddedVia::Scan,
                        &defaults,
                        &mut occupancy,
                    )?;
                    let mut lifted = LogEntry::new(
                        Action::ExclusionRemoved,
                        Level::Info,
                        Visibility::User,
                        Outcome::Ok,
                    );
                    lifted.entity = Some(Entity {
                        kind: EntityType::Exclusion,
                        id: exclusion.id.as_str(),
                    });
                    lifted.details = Some(Details::File {
                        path: PathBuf::from(&exclusion.path),
                    });
                    let mut created_id: Option<String> = None;
                    let slot = match registered {
                        Registered::Created(created) => {
                            created_id = Some(created.id.clone());
                            CommitSlot::Created(created)
                        }
                        // Unreachable after the check above inside one
                        // transaction; honest anyway — the same typed
                        // duplicate answer the check would have built,
                        // with the lift (which DID happen) still
                        // journaled.
                        Registered::Existing(existing) => {
                            CommitSlot::Entry(duplicate_entry(&candidate.path, &existing))
                        }
                    };
                    // The attribution: the row created in the same
                    // transaction; the unreachable duplicate arm has no
                    // row to name.
                    lifted.project_id = created_id.as_deref();
                    insert(transaction, &lifted).map_err(activity_log_to_app_error)?;
                    // The D-051 mirror contract: one call per written
                    // entry (info+user+ok is the unmirrored shape).
                    logging::action_logged(&lifted);
                    slots.push(slot);
                    continue;
                }
                match register_project(
                    transaction,
                    candidate,
                    AddedVia::Scan,
                    &defaults,
                    &mut occupancy,
                )? {
                    Registered::Created(created) => slots.push(CommitSlot::Created(created)),
                    // Unreachable after the check above inside one
                    // transaction; honest anyway — the same typed
                    // duplicate answer the check would have built.
                    Registered::Existing(existing) => slots.push(CommitSlot::Entry(
                        duplicate_entry(&candidate.path, &existing),
                    )),
                }
            }
            Ok(slots)
        })
        .await?;

    // Step 6 — the §6.9 write-access probe per created row, after the
    // commit: the project IS added whatever the verdict (the manual
    // add's step 5). The probe covers the whole batch in ONE blocking
    // task, so its join failure kills every verdict at once — the
    // typed partial success (D-196): the rows stay committed, the
    // answer carries `probe_failed`, the verdicts are simply absent.
    let created_count = slots
        .iter()
        .filter(|slot| matches!(slot, CommitSlot::Created(_)))
        .count();
    let created: Vec<&Project> = slots
        .iter()
        .filter_map(|slot| match slot {
            CommitSlot::Created(project) => Some(project),
            CommitSlot::Entry(_) => None,
        })
        .collect();
    let (warnings, probe_failed) = if created.is_empty() {
        (Vec::new(), false)
    } else {
        let targets: Vec<PathBuf> = created
            .iter()
            .map(|project| {
                Path::new(&project.path).join(probe_target_name(project.scheme, project.base_name))
            })
            .collect();
        let joined = tauri::async_runtime::spawn_blocking(move || {
            targets
                .iter()
                .map(|target| {
                    eunomia_fs::write_access::check_write_access(target)
                        .err()
                        .as_ref()
                        .and_then(write_access_warning)
                })
                .collect()
        })
        .await;
        probe_verdicts(joined)
    };
    let result = commit_result(slots, warnings, probe_failed);

    // Step 7 — one watch refresh for the whole batch, never fatal
    // (the `project_add_manual` step 6 posture: the rows ARE added
    // whatever the pass answers, and the next anchor re-converges).
    // The gate counts CREATED rows, not delivered warnings: a probe
    // that failed to join still leaves rows to arm the watch over.
    if created_count > 0
        && let Err(refused) = file_watch::refresh_after_mutation(&app).await
    {
        logging::file_watch_mutation_refresh_degraded(&refused);
    }
    Ok(result)
}

/// The post-commit probe's verdicts from its join result (D-196): the
/// positional warnings — one per created row — and whether the probe
/// half failed. A failed join keeps the COMMITTED answer honest: the
/// rows are merged and the command answers `Ok` with `probe_failed`
/// and empty verdicts (the entries' warnings are `None` because
/// unchecked), never the plain `Err` that would read as "the commit
/// failed" while every row is merged. The failure logs once, through
/// the class-only edge, exactly the fields the failure edge carries.
fn probe_verdicts(join: Result<Vec<Option<String>>, tauri::Error>) -> (Vec<Option<String>>, bool) {
    match join {
        Ok(warnings) => (warnings, false),
        Err(join_error) => {
            let degraded = db::join_failure_to_app_error(join_error);
            logging::scan_commit_probe_degraded(&degraded);
            (Vec::new(), true)
        }
    }
}

/// The committed answer (D-196): one entry per slot — a created row
/// rides its own warning positionally, a writable row's `None`
/// included — plus the probe marker. The warnings arrive in the same
/// order the `CommitSlot::Created` slots have in `slots`; a skipped
/// `None` would land the next row's warning on this row (the round-1
/// review's P1 of the original assembly).
fn commit_result(
    slots: Vec<CommitSlot>,
    warnings: Vec<Option<String>>,
    probe_failed: bool,
) -> ScanCommitResult {
    let mut warnings = warnings.into_iter();
    let entries = slots
        .into_iter()
        .map(|slot| match slot {
            CommitSlot::Created(project) => added_entry(&project, warnings.next().flatten()),
            CommitSlot::Entry(entry) => entry,
        })
        .collect();
    ScanCommitResult {
        entries,
        probe_failed,
    }
}

/// One candidate's result inside the commit's transaction: a typed
/// non-add entry, or the created row awaiting its post-commit probe.
enum CommitSlot {
    /// The repository was already registered or is excluded — the
    /// answer is a fact about the selection, nothing was written.
    Entry(ScanCommitEntry),
    /// The row was created and journaled; the commit is the
    /// transaction's.
    Created(Project),
}

/// The `"added"` entry around a created row and its §6.9 warning.
fn added_entry(project: &Project, warning: Option<String>) -> ScanCommitEntry {
    ScanCommitEntry {
        path: project.path.clone(),
        outcome: "added".to_owned(),
        project: Some(AddedProject::from_store(project, warning)),
        existing: None,
    }
}

/// The duplicate entry: the outcome is the existing row's own state —
/// `"already_added"` for an active row, `"archived"` for the DB-015
/// restore offer.
fn duplicate_entry(path: &str, existing: &Project) -> ScanCommitEntry {
    ScanCommitEntry {
        path: path.to_owned(),
        outcome: match existing.state {
            ProjectState::Active => "already_added",
            ProjectState::Archived => "archived",
        }
        .to_owned(),
        project: None,
        existing: Some(ExistingProject::of(existing)),
    }
}

/// Whether `root` is `folder` itself, by DB-030's identity — the ONE
/// rule the session gate and the classifier key by — not by bytes:
/// both forms came out of `canonicalize`, so their keys exist, and a
/// key that cannot be built cannot be equal.
fn is_own_root(root: &Path, folder: &Path) -> bool {
    match (
        eunomia_fs::paths::path_key(root),
        eunomia_fs::paths::path_key(folder),
    ) {
        (Ok(root_key), Ok(folder_key)) => root_key == folder_key,
        _ => false,
    }
}

/// The SEC-003 discovered-set rejection of a path no completed scan
/// reported: the settings shape through the shared builder (the
/// `session_paths::rejected_ui_path` twin for this command's rule).
/// `value` is the caller's own input or the lossy spelling of the
/// folder it resolved to — a diagnostic, never stored (SEC-008).
fn rejected_unreported_path(path: &str) -> AppError {
    rejected_field(
        "path",
        path,
        "must be a repository the last completed scan of this session reported (SEC-003)",
    )
}

/// A §6.9 field rejection of one candidate, carrying the repository's
/// path beside the field — the batch's answer to "which row": the
/// manual add names the field alone because its path is the one pick.
fn field_rejection_of(root: &Path, mut error: AppError) -> AppError {
    if !error
        .params
        .get("field")
        .is_some_and(|field| field == "path")
    {
        error
            .params
            .insert("path".to_owned(), root.to_string_lossy().into_owned());
    }
    error
}

/// The discovered-set state, with the honest missing-state answer (the
/// `scan_slot_missing` shape): managed by `setup` before anything that
/// could fail, so its absence is an uninitialized-app bug — a refusal
/// naming it, never a commit that admits everything or nothing
/// silently.
fn managed_findings<'a, R: Runtime>(
    app: &'a AppHandle<R>,
) -> Result<tauri::State<'a, ScanFindings>, AppError> {
    app.try_state::<ScanFindings>().ok_or_else(|| {
        let mut error = AppError::new(ErrorCode::DbLocked);
        error.details = Some(ErrorDetails {
            os_code: None,
            os_kind: Some("ScanFindingsMissing".to_owned()),
            os_message: Some("the scan findings set was not managed at startup".to_owned()),
        });
        error
    })
}

/// The per-scan snapshot of step 3: the §6.5 scan settings and the
/// ALG-053 registry, in ONE `Db::run` read transaction — the walk's
/// `ScanOptions` (built here, so the walker stays settings-free) and
/// the classifier's `ScanRegistry` (the three key sets exactly as
/// stored, the identity the add flow computed).
async fn load_scan_inputs<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<
    (
        eunomia_fs::scan::ScanOptions,
        eunomia_fs::scan_classify::ScanRegistry,
    ),
    AppError,
> {
    let database = managed_db(app)?;
    database
        .run(|transaction| {
            let depth = eunomia_store::settings::get(
                transaction,
                eunomia_store::settings::SettingKey::ScanMaxDepth,
            )
            .map_err(settings_cell_to_app_error)?;
            let names = eunomia_store::settings::get(
                transaction,
                eunomia_store::settings::SettingKey::ScanExcludeNames,
            )
            .map_err(settings_cell_to_app_error)?;
            let options = scan_options(depth, names)?;
            let active = eunomia_store::projects::list(
                transaction,
                eunomia_store::projects::ListScope::Active,
            )
            .map_err(projects_store_to_app_error)?;
            let archived = eunomia_store::projects::list(
                transaction,
                eunomia_store::projects::ListScope::Archived,
            )
            .map_err(projects_store_to_app_error)?;
            let excluded = eunomia_store::projects::list_exclusions(transaction)
                .map_err(projects_store_to_app_error)?;
            let registry = ScanRegistry::new(
                &active
                    .iter()
                    .map(|p| p.path_key.clone())
                    .collect::<Vec<_>>(),
                &archived
                    .iter()
                    .map(|p| p.path_key.clone())
                    .collect::<Vec<_>>(),
                &excluded
                    .iter()
                    .map(|e| e.path_key.clone())
                    .collect::<Vec<_>>(),
            );
            Ok((options, registry))
        })
        .await
}

/// Builds the walker's [`eunomia_fs::scan::ScanOptions`] from the
/// stored settings values — the boundary where the §6.5 shapes
/// become the walker's parameters. The store's own validation pins
/// `scan.max_depth` to 1–20 and `scan.exclude_names` to a string
/// list, and the guarantees are ENFORCED here, not assumed (the
/// `session_paths::contains` rule): a value that somehow defies its
/// key's contract answers the recorded fallback rather than a panic.
fn scan_options(
    depth: eunomia_store::settings::SettingValue,
    names: eunomia_store::settings::SettingValue,
) -> Result<eunomia_fs::scan::ScanOptions, AppError> {
    use eunomia_store::settings::SettingValue;
    let max_depth = match depth {
        SettingValue::Int(value) => {
            let as_usize = usize::try_from(value).ok().and_then(NonZeroUsize::new);
            match as_usize {
                Some(depth) if (1..=20).contains(&depth.get()) => depth,
                other => {
                    return Err(scan_settings_shape(format!(
                        "scan.max_depth answered {} (outside 1..=20)",
                        other.map_or_else(|| "non-positive".to_owned(), |d| d.get().to_string())
                    )));
                }
            }
        }
        other => {
            return Err(scan_settings_shape(format!(
                "scan.max_depth answered the wrong value kind: {}",
                kind_name(&other)
            )));
        }
    };
    let exclude_names = match names {
        SettingValue::StrList(names) => names,
        other => {
            return Err(scan_settings_shape(format!(
                "scan.exclude_names answered the wrong value kind: {}",
                kind_name(&other)
            )));
        }
    };
    Ok(eunomia_fs::scan::ScanOptions {
        max_depth,
        exclude_names,
    })
}

/// The [`scan_options`] refusal: the recorded D-037 fallback with a
/// class-only diagnostic (a value that defies its key's contract is
/// a stored-data anomaly, not caller input — the
/// `settings_cell_to_app_error` corruption posture; the rendered
/// message carries the kind and range, never a stored path or name).
fn scan_settings_shape(message: String) -> AppError {
    let mut error = AppError::new(ErrorCode::DbLocked);
    error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some("ScanSettingsShape".to_owned()),
        os_message: Some(message),
    });
    error
}

/// The value-kind name for the [`scan_options`] diagnostic — the
/// §6.5 "Type" column's own words, class-only.
fn kind_name(value: &eunomia_store::settings::SettingValue) -> &'static str {
    use eunomia_store::settings::SettingValue;
    match value {
        SettingValue::Scheme(_) => "scheme",
        SettingValue::BaseName(_) => "base name",
        SettingValue::ExtraFile(_) => "extra-file hint",
        SettingValue::Bool(_) => "boolean",
        SettingValue::Int(_) => "integer",
        SettingValue::Str(_) => "string",
        SettingValue::StrList(_) => "string list",
    }
}

#[cfg(test)]
mod tests {
    //! The pure halves: the throttle's literal, its immediate first
    //! emission, the coalescing of a burst into the newest state,
    //! and the rate bound over a simulated clock (the command
    //! surface lives in `scan_tests.rs`). Oracles are the spec's own
    //! words — "no more than 10 times per second" — computed over
    //! windows of simulated time, never wall seconds.

    use super::*;

    /// A path of the simulated walk (distinct strings so the
    /// coalescing assertions can tell events apart).
    fn step_path(n: u64) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("/scan/step-{n}"))
    }

    /// The simulated clock's epoch.
    fn t0() -> Instant {
        // `Instant::now()` is only the anchor the synthetic offsets
        // are added to; every assertion compares offsets, so the
        // anchor's real value never matters.
        Instant::now()
    }

    #[test]
    fn the_min_interval_is_the_spec_derived_literal() {
        // "No more than 10 times per second" (SYS-003, ALG-053) is
        // one tenth of a second of spacing — pinned against the
        // literal the way `dirty_recalc::DEBOUNCE` is.
        assert_eq!(PROGRESS_MIN_INTERVAL, Duration::from_millis(100));
    }

    #[test]
    fn the_event_name_is_the_spec_literal() {
        // The wire name is the frontend's whole contract; pinned
        // against the §7.10 literal exactly like `state://dirty`'s.
        assert_eq!(ScanProgressEvent::NAME, "scan://progress");
    }

    #[test]
    fn the_findings_admit_exactly_the_published_keys_and_a_new_scan_replaces_them() {
        // The discovered-set rule at its seam (D-179): the set admits
        // a path iff its DB-030 key — the one identity rule — is one
        // of the published keys. The exact-folder boundary of D-100
        // holds (a folder UNDER a reported repository is not the
        // repository, and the picked root is not a repository unless
        // the walk reported it), each publication REPLACES the set
        // (the last completed scan is the commit's universe, never a
        // session-long union), and an empty publication is the honest
        // empty set.
        let findings = ScanFindings::new();
        let key =
            |path: &str| eunomia_fs::paths::path_key(Path::new(path)).expect("UTF-8 text keys");
        assert!(
            !findings.admits(Path::new("/scan/a")),
            "a fresh session reports nothing"
        );

        findings.publish([key("/scan/a"), key("/scan/b")]);
        assert!(findings.admits(Path::new("/scan/a")));
        assert!(findings.admits(Path::new("/scan/b")));
        assert!(
            !findings.admits(Path::new("/scan/a/sub")),
            "a folder under a reported repository is not the repository"
        );
        assert!(
            !findings.admits(Path::new("/scan")),
            "the scan root is not a repository unless the walk reported it"
        );
        assert_eq!(findings.len(), 2, "two reported keys");

        findings.publish([key("/scan/c")]);
        assert!(
            !findings.admits(Path::new("/scan/a")),
            "the next completed scan replaces the set"
        );
        assert!(findings.admits(Path::new("/scan/c")));
        assert_eq!(findings.len(), 1, "only the last scan's keys remain");

        findings.publish(std::iter::empty());
        assert_eq!(
            findings.len(),
            0,
            "a scan that found nothing reports nothing"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_cannot_key_is_admitted_by_no_findings() {
        // The D-086 addendum at the set's seam: a path whose text is
        // not valid UTF-8 cannot equal any reported key (the walk
        // refuses such paths before reporting a repository), so
        // membership honestly answers false instead of failing.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let findings = ScanFindings::new();
        findings.publish([
            eunomia_fs::paths::path_key(Path::new("/scan/a")).expect("UTF-8 text keys"),
            "/scan/repo\u{fffd}".to_owned(),
        ]);
        assert!(
            !findings.admits(Path::new(OsStr::from_bytes(b"/scan/repo\xff"))),
            "an unkeyable probe is admitted by nothing"
        );
    }

    #[test]
    fn a_probe_join_failure_answers_the_partial_marker_not_an_error() {
        // D-196's mapping, fed a REAL join error — a panicking
        // blocking task, the probe's one realistic failure (release
        // builds abort before this; in tests the panic is the
        // deterministic vector). The verdicts are absent and the
        // marker is on: the committed answer must not become the
        // plain `Err` that would read as "the commit failed" while
        // every row is merged. The `Ok` arm is the pass-through: the
        // delivered verdicts and no marker.
        let join_error = tauri::async_runtime::block_on(async {
            tauri::async_runtime::spawn_blocking(|| -> Vec<Option<String>> {
                panic!("the probe task panicked");
            })
            .await
            .expect_err("a panicked task's join fails")
        });
        let (warnings, probe_failed) = probe_verdicts(Err(join_error));
        assert!(warnings.is_empty(), "a failed join delivers no verdicts");
        assert!(probe_failed, "the marker rides the degraded answer");

        let delivered = vec![None, Some("folder_not_writable".to_owned())];
        let (passed, marker) = probe_verdicts(Ok(delivered.clone()));
        assert_eq!(
            passed, delivered,
            "a joined probe passes its verdicts through"
        );
        assert!(!marker, "a joined probe sets no marker");
    }

    #[test]
    fn the_commit_result_carries_the_probe_marker_with_warnings_aligned() {
        // The assembly (D-196): each created row rides its OWN
        // positional warning — the `None` of a writable row must not
        // be skipped over (the round-1 review's P1) — and
        // `probe_failed` rides the result, not any entry. With the
        // marker on, the verdicts are absent and every added row's
        // warning is `None` because unchecked.
        let project = |id: &str| eunomia_store::projects::Project {
            id: id.to_owned(),
            path: format!("/repos/{id}"),
            path_key: format!("/repos/{id}"),
            name: id.to_owned(),
            label: "PROJE".to_owned(),
            color: "blue".to_owned(),
            description: String::new(),
            state: eunomia_store::projects::ProjectState::Active,
            added_via: eunomia_store::projects::AddedVia::Scan,
            sort_order: 1,
            scheme: eunomia_store::settings::Scheme::A,
            base_name: eunomia_store::settings::BaseName::AgentsMd,
            extra_local: eunomia_store::settings::ExtraFile {
                enabled: false,
                position: eunomia_store::settings::Position::End,
            },
            extra_project: eunomia_store::settings::ExtraFile {
                enabled: false,
                position: eunomia_store::settings::Position::End,
            },
            last_synced_at: None,
            last_synced_hash: None,
            created_at: "2026-11-24T00:00:00.000Z".to_owned(),
            updated_at: "2026-11-24T00:00:00.000Z".to_owned(),
            archived_at: None,
        };
        let slots = vec![
            CommitSlot::Created(project("first")),
            CommitSlot::Entry(ScanCommitEntry {
                path: "/repos/known".to_owned(),
                outcome: "already_added".to_owned(),
                project: None,
                existing: None,
            }),
            CommitSlot::Created(project("second")),
        ];
        let warnings = vec![None, Some("folder_not_writable".to_owned())];
        let result = commit_result(slots, warnings, false);
        assert!(!result.probe_failed);
        assert_eq!(result.entries.len(), 3);
        let first = result.entries[0]
            .project
            .as_ref()
            .expect("the first created row answers added");
        assert_eq!(
            first.write_access_warning, None,
            "the writable row's own verdict"
        );
        let second = result.entries[2]
            .project
            .as_ref()
            .expect("the second created row answers added");
        assert_eq!(
            second.write_access_warning.as_deref(),
            Some("folder_not_writable"),
            "the warning lands on ITS row, not on the earlier one"
        );

        // The degraded assembly: no verdicts arrived, the marker is
        // on, every created row's warning is None because unchecked.
        let slots = vec![CommitSlot::Created(project("first"))];
        let result = commit_result(slots, Vec::new(), true);
        assert!(result.probe_failed);
        assert_eq!(
            result.entries[0]
                .project
                .as_ref()
                .expect("the committed row still answers added")
                .write_access_warning,
            None,
            "unchecked is not writable — the marker is the distinguishing fact"
        );
    }

    #[test]
    fn the_first_event_emits_immediately_with_its_own_facts() {
        // The window opens at the walk's start: the very first event
        // reaches the wire at once, carrying that event's counts and
        // path (a scan of one folder must not show a silent window
        // until a second passes).
        let mut tracker = ProgressTracker::new(PROGRESS_MIN_INTERVAL);
        let now = t0();

        let payload = tracker
            .on_event(&step_path(0), false, now)
            .expect("the first event emits");

        assert_eq!(payload.checked, 1);
        assert_eq!(payload.found, 0);
        assert_eq!(payload.current_path, step_path(0).to_string_lossy());
    }

    #[test]
    fn a_repository_event_counts_into_found() {
        // The payload's two counters are distinct facts: a
        // repository event bumps both, a plain folder only the
        // checked one (PRJ-002's "folders checked" and
        // "repositories found").
        let mut tracker = ProgressTracker::new(PROGRESS_MIN_INTERVAL);
        let now = t0();

        let first = tracker.on_event(&step_path(0), false, now).expect("emits");
        assert_eq!((first.checked, first.found), (1, 0));

        let second = tracker
            .on_event(&step_path(1), true, now + PROGRESS_MIN_INTERVAL)
            .expect("a full interval later emits");
        assert_eq!((second.checked, second.found), (2, 1));
    }

    #[test]
    fn events_inside_the_interval_coalesce_and_the_newest_state_lands() {
        // The coalescing rule: events inside the quiet window update
        // the state silently, and the first event after the window
        // carries the NEWEST facts — never the stale intermediate
        // state of the event that opened the window.
        let mut tracker = ProgressTracker::new(PROGRESS_MIN_INTERVAL);
        let now = t0();

        tracker
            .on_event(&step_path(0), false, now)
            .expect("the first event emits");
        // Three quiet events (a plain folder, a repository, a skip
        // shaped as a plain one for the counter's sake).
        assert!(
            tracker
                .on_event(&step_path(1), false, now + Duration::from_millis(10))
                .is_none(),
            "an event inside the interval does not emit"
        );
        assert!(
            tracker
                .on_event(&step_path(2), true, now + Duration::from_millis(50))
                .is_none(),
            "a repository event inside the interval does not emit either"
        );
        assert!(
            tracker
                .on_event(&step_path(3), true, now + Duration::from_millis(99))
                .is_none(),
            "99 ms after the last emission is still inside the 100 ms window"
        );
        // The next event opens the next window with everything it
        // coalesced: four prior events plus itself, two of them
        // repositories, and ITS path — not step-0's or step-1's.
        let payload = tracker
            .on_event(&step_path(4), false, now + PROGRESS_MIN_INTERVAL)
            .expect("a full interval after the last emission emits");
        assert_eq!(payload.checked, 5);
        assert_eq!(payload.found, 2);
        assert_eq!(payload.current_path, step_path(4).to_string_lossy());
    }

    #[test]
    fn a_simulated_walk_never_exceeds_ten_emissions_per_second() {
        // The card's own bound, as the spec words it: one event per
        // simulated millisecond over a ten-second walk (10_001
        // events — real walks burst far faster, the bound must hold
        // for any rate), and EVERY sliding one-second window of the
        // emission timeline holds at most ten. The total is pinned
        // to the arithmetic the interval dictates (emissions at 0,
        // 100, …, 10_000 ms — 101), so a throttle that emitted one
        // extra time anywhere fails the count, and one that dropped
        // the coalescing (mutating the gate away) fails the window
        // sweep by orders of magnitude.
        let interval = PROGRESS_MIN_INTERVAL;
        let mut tracker = ProgressTracker::new(interval);
        let start = t0();
        let mut emission_times: Vec<Instant> = Vec::new();

        for step in 0..=10_000u64 {
            let now = start + Duration::from_millis(step);
            if tracker
                .on_event(&step_path(step), step % 7 == 0, now)
                .is_some()
            {
                emission_times.push(now);
            }
        }

        assert_eq!(
            emission_times.len(),
            101,
            "a 10-second walk at one event per millisecond emits exactly at 0,100,…,10_000 ms"
        );
        for window in emission_times.windows(2) {
            let spacing = window[1].duration_since(window[0]);
            assert!(
                spacing >= interval,
                "consecutive emissions are at least {interval:?} apart; saw {spacing:?}"
            );
        }
        for from in &emission_times {
            let in_window = emission_times
                .iter()
                .filter(|time| {
                    let time = **time;
                    time >= *from && time.duration_since(*from) < Duration::from_secs(1)
                })
                .count();
            assert!(
                in_window <= 10,
                "a sliding one-second window holds at most 10 emissions; one starting at \
                 {:?} holds {in_window}",
                from.duration_since(start)
            );
        }
    }
}
