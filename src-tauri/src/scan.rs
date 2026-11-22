//! The scan commands and the `scan://progress` event (spec §7.10
//! Projects module `scan_start`/`scan_cancel`, §9.6, PRJ-002,
//! SYS-003/NFR-015, ALG-053, ERR-002; T-076, D-153).
//!
//! This module is the COMMAND LAYER the walker's docs assigned to
//! "T-076's plumbing": it drives
//! [`scan_repositories`](eunomia_fs::scan::scan_repositories) over a
//! SEC-003-validated root, classifies every event through
//! [`classify_entry`](eunomia_fs::scan_classify::classify_entry)
//! against a per-scan registry snapshot, serializes the live facts
//! into the throttled `scan://progress` event, and answers with the
//! found/candidate set — the input PRJ-002's result view selects from
//! and T-077's `scan_commit` will consume. Nothing is persisted
//! anywhere in this module (ERR-002): the scan is read-only over the
//! filesystem and the database, and a cancelled or interrupted scan
//! answers [`ScanRunOutcome::Cancelled`] with no payload at all.
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
//!    terminal counters; [`ScanOutcome::Cancelled`] drops everything
//!    accumulated and answers the payload-less
//!    [`ScanRunOutcome::Cancelled`] (ERR-002 — an interrupted scan
//!    saves nothing, and the wire shape cannot lie about it).
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
//! the honest no-database answer, and the registration pins.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eunomia_fs::scan::CancelToken;
use eunomia_fs::scan_classify::{self, ClassifiedScanEvent, ScanClass, ScanRegistry};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use crate::commands::{
    managed_db, projects_store_to_app_error, settings_cell_to_app_error, sync_running_refusal,
};
use crate::db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
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
/// two); T-077's `scan_commit` consumes the user's selection from
/// the same list.
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
    /// / Skipped. Nothing is persisted with them — `scan_commit`
    /// (T-077) is the only writer.
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
    // accumulated (ERR-002): the arm carries no payload to show.
    let mut walk = walk_state
        .lock()
        .expect("the scan walk state must not be poisoned");
    match outcome {
        eunomia_fs::scan::ScanOutcome::Completed => Ok(ScanRunOutcome::Completed {
            checked: walk.progress.checked,
            found: walk.progress.found,
            repositories: std::mem::take(&mut walk.repositories),
            skipped: std::mem::take(&mut walk.skipped),
        }),
        eunomia_fs::scan::ScanOutcome::Cancelled => {
            logging::scan_cancelled(walk.progress.checked);
            Ok(ScanRunOutcome::Cancelled)
        }
    }
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
