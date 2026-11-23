//! The target-file watch service (spec §5.7, SYS-007…009, OUT-014,
//! MAIN-028, §7.10's `state://file-changed` event; T-102, T-F13):
//! watches the target files of active, available projects, re-reads
//! them on debounced events, and tells the UI which projects'
//! externally-changed / target-deleted verdicts changed.
//!
//! Watching is **display-only** (OUT-014): a watch event never writes
//! a file, never merges, never opens a dialog — the next sync
//! overwrites as it would anyway. The service's only effects are the
//! [`state://file-changed`][`FileChangedEvent`] event and its own
//! session caches; the write-back half of T-F13 belongs to the sync
//! pipeline (T-118), which this service prepares and proves readiness
//! for (the clearing pass after a simulated sync write, below).
//!
//! # The two layers (the crate split of D-010, applied to §5.7)
//!
//! - [`eunomia_fs::watcher`] owns the mechanics: one debounced
//!   `notify` watch per parent directory of the wanted files,
//!   name-filtered so each target file is observed individually
//!   (SYS-007), tolerant of unwatchable roots.
//! - This module owns the semantics: WHICH files (active + available
//!   projects, one file per project through
//!   [`eunomia_core::backup_name::target_file_name`] — §5.7's closed
//!   list, D-080's reading: A|B → `AGENTS.md`/`CLAUDE.md`, C →
//!   `<STEM>.project.md`), watched and read at their RESOLVED path
//!   (§9.4 preflight step 2: a symlinked `CLAUDE.md` resolving to
//!   `AGENTS.md` is watched as the real file — the name external
//!   editors change), and WHAT an event means (SYS-008's re-read
//!   and comparison).
//!
//! # When the set and the state are computed
//!
//! SYS-007 fixes the watch-set points, SYS-008 the state points; both
//! run through one pass, [`refresh_pass`]:
//!
//! - **Startup** — the `setup` hook spawns [`spawn_service`], which
//!   drops the first `Refresh` permit before its spawn (the dirty
//!   service's precedent: the permit cannot race the task's first
//!   `recv`).
//! - **Availability changes** — [`listen_availability`] subscribes to
//!   T-079's `state://project-availability` event; every diff (either
//!   direction) drops a `Refresh` permit, so watches follow folders
//!   that die and return. Registered in `lib.rs` BEFORE the startup
//!   availability recheck is spawned, so the session's first verdict
//!   cannot slip past the listener.
//! - **After a sync** — [`refresh_after_sync`], the awaited seam the
//!   sync pipeline (T-118/D-138) calls once its writes and DB-024
//!   hash updates committed: files may have appeared, and every
//!   written/unchanged project's external state must be re-computed
//!   (SYS-008's "right after a sync").
//! - **Debounced watch events** — the pump thread turns the watcher's
//!   file deliveries into `FilesChanged` batches (the backend's
//!   lost-events signal becomes a full `Refresh`, below); the pass
//!   re-reads only the affected projects (the noise filter SYS-008
//!   names).
//! - **Project mutations** — SYS-007's own set points: a project
//!   ADDED joins the watched set (`project_add_manual`'s committed
//!   arm awaits the bounded mutation seam
//!   [`refresh_after_mutation`] since T-280 — the after-sync seam's
//!   twin with a wait budget, so the watch pass can no longer hang
//!   the interactive answer; the add flow's own older probe
//!   exposure stays D-144's recorded boundary), and the
//!   scheme/file-name/archive/restore/
//!   relink changes call the same seam as their §7.10 commands land
//!   (`project_update`, `project_archive`, `project_restore`,
//!   `project_relink` — the SYS-005 gate's "join as they land"
//!   precedent). No commit-hook piggyback exists here by design: the
//!   DB-022 hook is the dirty service's, and this service's pass
//!   probes folders — a hook that fires inside the connection's
//!   mutex must never block (D-121).
//!
//! The backend's lost-events signal ([`WatchEvent::Backend`], SYS-010)
//! also lands as a `Refresh`: the documented "re-check all target
//! files once" degrade, no crash. Both of the signal's shapes ask for
//! that one full re-check — an event-queue overflow (which inotify
//! delivers as a rescan flag riding an ORDINARY batch, not a stream
//! error: corrected by D-139) and a stream-error batch alike; the
//! pump's mapping ([`pump_event`]) is pinned per variant since T-259.
//! A REFUSED watch (an unwatchable root, the inotify limit) is the
//! other SYS-010 half: not an event but a persistent fact, recorded
//! per full pass as the wanted-but-unwatched set with the refusal
//! class that explains each entry
//! ([`FileWatchState::unwatched_projects`]) — the seam the
//! refused-watch warn line ([`logging::file_watch_refused`], once
//! per degradation episode) and the non-blocking notice (the
//! `projects_list` carrier, D-155) read. A refused RE-ARM counts the
//! same way (since the watcher's liveness fix, D-139's phantom-entry
//! follow-up): a watch the backend proves dead — the directory gone
//! for good, the watch lost with the limit blocking the re-add — is
//! reported as unwatched, never kept as a phantom entry that
//! `watched_files` would count.
//!
//! # The verdict (SYS-008, SYS-009)
//!
//! [`classify_target_file`] is the pure rule, per project:
//!
//! - The **build** is the project's §9.2 body over the T-100 snapshot
//!   (the shared chunked pass, `eunomia_core::content_build` over a
//!   snapshot scoped to the wanted subset — D-145's consolidation,
//!   joined by this service in T-267);
//!   the comparison is ALG-033 (`compare::contents_match`) against
//!   the build's markerless new side — spelled through the
//!   production builders (`target_file::build_content` minus its
//!   marker tail for A/C; `assembly_b::assemble` of the current file
//!   minus the inserted marker for B), so no hand-rolled mirror can
//!   drift. A scheme-B file whose markers are gone or invalid, or
//!   whose bytes are not UTF-8, can never match the build (the sync
//!   will answer `E_MARKERS_INVALID` / `E_NOT_UTF8`) — externally
//!   modified is the honest verdict.
//! - A **missing file** after a recorded sync (`last_synced_hash`
//!   non-empty, DB-024/D-068) is *target deleted* (SYS-009). A
//!   never-synced project with no file is not: OUT-005's sync creates
//!   the file, there is nothing "deleted" yet — the flag would mark
//!   every fresh project's column. A file that exists but differs is
//!   externally modified regardless of sync history (a pre-existing
//!   `AGENTS.md` a sync would overwrite IS worth showing).
//! - The two flags are mutually exclusive in the verdict; a project
//!   leaving the watch set (archived, unavailable) is pruned from the
//!   cache silently — its column shows unavailable/archived, and a
//!   later return is a first observation against the clean baseline.
//!
//! # The diff contract (D-112's shape, third instance)
//!
//! The [`FileWatchState`] cache holds the last verdicts per project
//! — session memory, like availability's and dirty's. The baseline is
//! **clean** (`external_changed`/`file_deleted` both false): the
//! startup pass reports exactly the projects found modified or
//! deleted (an all-healthy startup is silent), later passes report
//! contradictions in both directions (a sync's write clears the
//! marking — the MAIN-028 icon must clear, and it does through this
//! event), empty diffs emit nothing, and emit-then-record keeps
//! delivery at-least-once. Unlike the dirty service there is **no
//! ticket guard**: every pass — startup, availability-triggered,
//! after-sync, debounced — runs on the single service task, so passes
//! serialize on the bus and cannot finish out of order;
//! [`refresh_after_sync`] does not run a pass of its own but awaits
//! one on that task through its ack channel.
//!
//! # Recovery mode
//!
//! No database (the T-024 state): the honest `NoDatabase` answer
//! through the [`crate::commands::managed_db`] edge, no event, no
//! panic; the service is never spawned there (the `setup` hook
//! returns before the spawn).
//!
//! # The diagnostic edges (D-137's follow-up 1, landed D-155)
//!
//! The facade lines this service's failure paths flow through, all
//! in `logging.rs` (SEC-008's structural pin): the watcher-init warn
//! (`file_watch_init_failed`, `lib.rs`'s degrade arm — no watching
//! this session, the launch unaffected, SYS-010's no-crash posture),
//! the pass-failure error (`file_watch_pass_failed`, §10's
//! every-error rule — logged by the service loop for the
//! FIRE-AND-FORGET triggers; an awaited seam failure's durable half
//! is the caller's own line, so every failure logs exactly once —
//! the add flow's awaited seam stays T-278's line and logs nowhere
//! until that card lands; the mutation twin's budget expiry is a
//! third class beside the refused seam and the pass failure, its
//! boundary recorded in D-156),
//! the emit warn (`file_watch_emit_failed`, a missed UI update —
//! the at-least-once retry contract) and the refused-watch warn
//! (`file_watch_refused`, once per degradation episode over the
//! recorded refusal classes — a converged pass logs nothing, the
//! D-046/D-054 no-line policy). The lines fire from the service
//! task, where the tests' thread-local subscriber does not exist
//! (the D-032/D-143 worker-thread boundary): the lines are tested
//! in `logging.rs`, the gate ([`refused_warn_counts`]) and the
//! recording are unit-tested here, and the one-line call-site
//! forwards are the honest unpinned remainder (the D-146 shape).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use eunomia_core::assembly_b::assemble;
use eunomia_core::assignment::Assignment;
use eunomia_core::backup_name::target_file_name;
use eunomia_core::compare::contents_match;
use eunomia_core::extra_files::{ExtraFileLines, extra_file_lines};
use eunomia_core::marker::{MarkerTime, creation_marker_line};
use eunomia_core::project::Scheme;
use eunomia_core::snapshot::{BuildSnapshot, SnapshotProject};
use eunomia_core::target_file::build_content;
use eunomia_fs::paths::resolve_target_path;
use eunomia_fs::watcher::{
    TargetWatcher, WatchBackend, WatchEvent, WatchRefusalKind, WatcherInitError,
};
use eunomia_store::build_snapshot::read as read_build_snapshot;
use eunomia_store::projects::ListScope;
use eunomia_store::projects::Project;

use crate::availability::{ProjectAvailabilityEvent, Verdict};
use crate::commands::{managed_db, projects_store_to_app_error};
use crate::db::join_failure_to_app_error;
use crate::dirty_recalc::build_snapshot_to_app_error;
use crate::errors::AppError;
use crate::logging;

/// SYS-007's debounce: every watch delivery waits this window before
/// the service sees it. A parameter of [`spawn_service`] only so the
/// tests can run a short window; production passes this constant.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(300);

/// How long an INTERACTIVE caller waits for its mutation-initiated
/// refresh pass ([`refresh_after_mutation`], T-280/D-156) before
/// degrading and answering without the pass — the bounded twin of the
/// after-sync seam's unbounded wait. Ten seconds sits an order of
/// magnitude above every healthy full pass's worst case (local-fs
/// work: one DB transaction, per project a stat, a symlink
/// resolution, a read and an in-memory build — milliseconds per
/// project, sub-second for hundreds), so a loaded machine cannot
/// trip a false degrade, while staying inside the band where the
/// user still reads the answer instead of retrying. A constant, not
/// a config key (§5.8/D-129's ruling for this class); a field of
/// [`FileWatchState`] only so the tests can run a short window (the
/// [`DEBOUNCE`] parameter precedent).
pub(crate) const MUTATION_REFRESH_BUDGET: Duration = Duration::from_secs(10);

/// The capacity of the command bus: room for a burst of debounced
/// batches behind a running pass. The pump uses `try_send` — a drop
/// is covered by the next refresh point (the reconciliation anchor;
/// watch events are best-effort triggers, refresh points are the
/// correctness anchors), never by a lost state: the verdict is
/// recomputed from the file on the next pass whatever this batch saw.
/// `pub(crate)` for the full-bus test's fill and post-expiry length
/// check (the mutation seam's send-half pin, D-156).
pub(crate) const BUS_CAPACITY: usize = 64;

/// One entry of the `state://file-changed` diff (§7.10): the project
/// a change is about, addressed by its row id, plus the two §5.7
/// flags that now hold. The display name is deliberately absent — the
/// UI already holds it keyed by id from `projects_list` (D-112).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct ProjectFileChange {
    /// The project row id (UUIDv7 TEXT, DB-002) whose target-file
    /// verdict changed since the previous pass.
    pub(crate) project_id: String,
    /// SYS-008: the target file on disk does not match the project's
    /// current build by the ALG-033 comparison.
    pub(crate) external_changed: bool,
    /// SYS-009: the target file is gone after a recorded sync.
    pub(crate) file_deleted: bool,
}

/// The `state://file-changed` event payload (§7.10): only the
/// projects whose verdict changed since the previous pass, in
/// `sort_order` (the snapshot order). Never emitted with an empty
/// `changes`.
///
/// The event name is the spec's literal wire name, pinned by the
/// manual [`tauri_specta::Event`] implementation below for the same
/// reason as `availability`'s and `dirty`'s (D-112): the derive macro
/// cannot produce the `state://` prefix, and the `derive` cargo
/// feature that ships it is not enabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct FileChangedEvent {
    /// The changed projects; empty diffs are not emitted (see the
    /// module docs' diff contract).
    pub(crate) changes: Vec<ProjectFileChange>,
}

impl Event for FileChangedEvent {
    /// The §7.10 wire name, verbatim. Tauri permits exactly the
    /// characters it contains (alphanumeric, `-`, `/`, `:`, `_`).
    const NAME: &'static str = "state://file-changed";
}

/// What wakes the service task: a full pass (subscriptions + every
/// verdict) or a debounced batch touching one file set. A full pass
/// may carry an ack — the completion channel of both refresh seams
/// ([`refresh_after_sync`]'s unbounded await and
/// [`refresh_after_mutation`]'s budgeted one; the service cannot
/// tell them apart — one command, two wait policies).
/// `pub(crate)` because it rides the pub(crate) signatures of
/// [`FileWatchState::new`], [`FileWatchState::trigger`] and
/// [`spawn_service`].
pub(crate) enum Command {
    /// Recompute the watch set and every available project's verdict.
    /// `done` receives the pass's result (both awaited seams — the
    /// after-sync one and its bounded mutation twin); the
    /// fire-and-forget triggers (startup, availability, the backend's
    /// lost-events signal) pass `None`.
    Refresh {
        done: Option<tauri::async_runtime::Sender<Result<Vec<ProjectFileChange>, AppError>>>,
    },
    /// Debounced watcher deliveries: re-check only the projects whose
    /// target files are named here (SYS-008's noise filter).
    FilesChanged(Vec<PathBuf>),
}

/// The two §5.7 flags as one comparable value — the diff cache's
/// currency and the event's payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileFlags {
    external_changed: bool,
    file_deleted: bool,
}

/// The implicit baseline of the diff contract: an unobserved project
/// counts as clean (module docs).
const CLEAN: FileFlags = FileFlags {
    external_changed: false,
    file_deleted: false,
};

/// The session state behind the service: the diff cache, the bus's
/// sending half, and the pass counter.
///
/// Managed in the `setup` hook before anything that could fail (the
/// [`crate::session_paths::SessionPaths`] shape, like the
/// availability and dirty states); in recovery mode it exists unused
/// — no service is spawned, no listener registered.
pub(crate) struct FileWatchState {
    /// Last recorded verdicts per project id (absent ≡ [`CLEAN`]).
    /// Guarded by a short synchronous critical section, never held
    /// across an `.await` or a watch syscall.
    last: Mutex<HashMap<String, FileFlags>>,
    /// The wanted-but-unwatched target files of the last full pass,
    /// keyed by their PROJECT id with the refusal class that explains
    /// each — SYS-010's observable, the seam the refused-watch warn
    /// line and the non-blocking notice read (a refused watch is
    /// persistent, not an event; the watcher's docs explain why it
    /// must not loop). Recorded by the service task after each full
    /// pass; the id key is composed at the recording seam itself
    /// ([`blocking_half`] owns both the watcher's path-keyed answer
    /// and the path→id routing table), so every consumer reads a
    /// project-addressed fact.
    unwatched: Mutex<BTreeMap<String, WatchRefusalKind>>,
    /// The command bus's sending half (capacity
    /// [`BUS_CAPACITY`]). Cloned into the availability listener and
    /// the pump thread; a `try_send` onto a full bus is the intended
    /// drop (see [`BUS_CAPACITY`]).
    trigger: tauri::async_runtime::Sender<Command>,
    /// How many passes ran (attempted — success or failure). The
    /// tests' deterministic observable for "a pass happened" (an
    /// empty diff is silent by contract). Written by the service loop
    /// in production too, read by nothing outside the tests.
    passes: AtomicU64,
    /// The bounded wait's budget for mutation-initiated refreshes
    /// ([`refresh_after_mutation`]): the production default is
    /// [`MUTATION_REFRESH_BUDGET`]; [`with_budget`] exists only so
    /// the tests can run a short window (the [`DEBOUNCE`] parameter
    /// precedent, T-280/D-156).
    mutation_budget: Duration,
}

impl FileWatchState {
    /// The empty session state and the bus's receiving half, which
    /// [`spawn_service`] consumes — created together because the
    /// channel is the state's own seam.
    pub(crate) fn new() -> (Self, tauri::async_runtime::Receiver<Command>) {
        Self::with_budget(MUTATION_REFRESH_BUDGET)
    }

    /// [`new`] with an explicit mutation-refresh budget — the test
    /// seam for the bounded wait ([`refresh_after_mutation`]):
    /// production constructs through [`new`] with the recorded
    /// constant, while a harness parks a service that never acks and
    /// wants the expiry in milliseconds, not seconds.
    pub(crate) fn with_budget(
        mutation_budget: Duration,
    ) -> (Self, tauri::async_runtime::Receiver<Command>) {
        let (trigger, commands) = tauri::async_runtime::channel(BUS_CAPACITY);
        (
            Self {
                last: Mutex::new(HashMap::new()),
                unwatched: Mutex::new(BTreeMap::new()),
                trigger,
                passes: AtomicU64::new(0),
                mutation_budget,
            },
            commands,
        )
    }

    /// The session's current verdicts, one entry per observed project
    /// in id order — the read side the `projects_list` column carrier
    /// reads (T-257, D-137's round-1 P1-1): the startup diff reaches
    /// the webview's listener only through a replayed read like this,
    /// because events are not replayed to late listeners. The shape
    /// is the event's own [`ProjectFileChange`], so the carrier is a
    /// read, not a redesign.
    pub(crate) fn snapshot(&self) -> Vec<ProjectFileChange> {
        let last = self
            .last
            .lock()
            .expect("the file-watch cache is locked only inside short synchronous sections");
        let mut entries: Vec<ProjectFileChange> = last
            .iter()
            .map(|(id, flags)| ProjectFileChange {
                project_id: id.clone(),
                external_changed: flags.external_changed,
                file_deleted: flags.file_deleted,
            })
            .collect();
        entries.sort_by(|a, b| a.project_id.cmp(&b.project_id));
        entries
    }

    /// The wanted-but-unwatched target files of the last full pass,
    /// keyed by project id with the refusal class that explains each
    /// — SYS-010's observable, the data the refused-watch warn line
    /// counts and the non-blocking notice serves per row (empty
    /// until the first full pass records).
    pub(crate) fn unwatched_projects(&self) -> BTreeMap<String, WatchRefusalKind> {
        self.unwatched
            .lock()
            .expect("the file-watch cache is locked only inside short synchronous sections")
            .clone()
    }

    /// Records the unwatched set (full passes only; a partial pass
    /// runs no `set_files`, so its answer would be stale) and returns
    /// the PREVIOUS recording — the episode gate's input: the caller
    /// warns through the facade only when a non-empty answer differs
    /// from it ([`refused_warn_counts`]). `pub(crate)` because the
    /// ipc carrier tests seed the recording through this same seam —
    /// a real refusal is unreachable through the real backend at the
    /// app layer (D-148's boundary: the availability probe gates the
    /// watch set), while the composite unit proof below drives the
    /// real recording path over the fake backend.
    pub(crate) fn note_unwatched(
        &self,
        unwatched: BTreeMap<String, WatchRefusalKind>,
    ) -> BTreeMap<String, WatchRefusalKind> {
        std::mem::replace(
            &mut *self
                .unwatched
                .lock()
                .expect("the file-watch cache is locked only inside short synchronous sections"),
            unwatched,
        )
    }

    /// Computes the changes `verdicts` carry against the cache,
    /// without recording them: a project is changed when its new
    /// flags differ from the recorded ones — or from [`CLEAN`] when
    /// it was never observed this session. The verdicts keep the
    /// snapshot's `sort_order`, so the diff does too.
    fn diff_against_last(&self, verdicts: &[(String, FileFlags)]) -> Vec<ProjectFileChange> {
        let last = self
            .last
            .lock()
            .expect("the file-watch cache is locked only inside short synchronous sections");
        verdicts
            .iter()
            .filter_map(|(id, flags)| {
                if last.get(id).copied().unwrap_or(CLEAN) == *flags {
                    None
                } else {
                    Some(ProjectFileChange {
                        project_id: id.clone(),
                        external_changed: flags.external_changed,
                        file_deleted: flags.file_deleted,
                    })
                }
            })
            .collect()
    }

    /// Records the verdicts; a full pass also prunes the ids that
    /// left the watch set (archived, unavailable) — the cache ends up
    /// exactly the intended world. The prune set is the ids the pass
    /// COVERED (the projects it set out to classify), not the ids it
    /// managed to classify: a project whose file could not be read
    /// contributed no verdict, and dropping it here would reset its
    /// last known flags to the clean baseline — the opposite of the
    /// unreadable branch's "keep the last known state" promise. A
    /// partial pass (debounced re-checks, `prune_to: None`) merges
    /// only its projects: the rest of the world was not re-read, and
    /// dropping it would reset live verdicts to the baseline.
    fn record(&self, verdicts: &[(String, FileFlags)], prune_to: Option<&[String]>) {
        let mut last = self
            .last
            .lock()
            .expect("the file-watch cache is locked only inside short synchronous sections");
        if let Some(covered) = prune_to {
            let live: HashSet<&str> = covered.iter().map(String::as_str).collect();
            last.retain(|id, _| live.contains(id.as_str()));
        }
        for (id, flags) in verdicts {
            last.insert(id.clone(), *flags);
        }
    }

    /// Drops one permit onto the bus; a full bus swallows it (see
    /// [`BUS_CAPACITY`]). `pub(crate)`: the tests fire triggers
    /// without naming the channel type.
    pub(crate) fn trigger(&self, command: Command) {
        let _ = self.trigger.try_send(command);
    }

    /// Notes that one pass ran (the tests' observable).
    fn note_pass(&self) {
        self.passes.fetch_add(1, Ordering::Release);
    }

    /// The number of passes that ran — the tests' deterministic
    /// observable for the debounce and trigger assertions.
    #[cfg(test)]
    pub(crate) fn pass_count(&self) -> u64 {
        self.passes.load(Ordering::Acquire)
    }
}

// ---- the verdict (SYS-008, SYS-009) ------------------------------------

/// SYS-008's answer for one project's target file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetFileVerdict {
    /// The file on disk is (still) the project's current build by the
    /// ALG-033 comparison — nothing to report.
    Matches,
    /// The file exists but differs from the build (ALG-033): SYS-008's
    /// externally modified.
    ExternallyModified,
    /// The file is gone after a recorded sync: SYS-009's target
    /// deleted.
    Deleted,
}

impl TargetFileVerdict {
    /// The event's flags for this verdict (mutually exclusive by
    /// construction: a missing file reports deleted, not modified).
    fn flags(self) -> FileFlags {
        match self {
            Self::Matches => CLEAN,
            Self::ExternallyModified => FileFlags {
                external_changed: true,
                file_deleted: false,
            },
            Self::Deleted => FileFlags {
                external_changed: false,
                file_deleted: true,
            },
        }
    }
}

/// The probe marker of the comparison sides: any single-line creation
/// marker works — ALG-033 drops the last marker line from the
/// existing file and the new side is the build minus its own marker,
/// so the timestamp between the phrases never reaches the comparison.
/// A fixed, real [`creation_marker_line`] output keeps the sides
/// byte-realistic (what a sync would actually write).
fn probe_marker_line() -> String {
    creation_marker_line(
        MarkerTime {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        },
        None,
        0,
    )
}

/// Computes SYS-008's verdict for one project's target file. Pure: no
/// I/O, no clock — the caller read the bytes.
///
/// `body` is the project's §9.2 body; `lines` its scheme-A reading
/// lines; `disk` the target file's current bytes, [`None`] when the
/// file does not exist; `ever_synced` whether a past sync recorded a
/// build hash for the project (DB-024; `NULL`/empty ≡ never, D-068).
pub(crate) fn classify_target_file(
    scheme: Scheme,
    body: &str,
    lines: &ExtraFileLines,
    disk: Option<&[u8]>,
    ever_synced: bool,
) -> TargetFileVerdict {
    let Some(bytes) = disk else {
        // SYS-009's flag exists only for a file that a sync once left
        // behind; a never-synced project simply has no file yet
        // (OUT-005 creates it) and must not light up every fresh
        // column.
        return if ever_synced {
            TargetFileVerdict::Deleted
        } else {
            TargetFileVerdict::Matches
        };
    };
    let side = match scheme {
        Scheme::A | Scheme::C => markerless_overwrite_side(scheme, body, lines),
        Scheme::B => match scheme_b_side(bytes, body) {
            Some(side) => side,
            // Not UTF-8, or the marker pair is gone/invalid: the file
            // can never equal the build (the sync will answer
            // E_NOT_UTF8 / E_MARKERS_INVALID — SYS-009's B sentence
            // for the deleted-file case is the same skip).
            None => return TargetFileVerdict::ExternallyModified,
        },
    };
    if contents_match(bytes, &side) {
        TargetFileVerdict::Matches
    } else {
        TargetFileVerdict::ExternallyModified
    }
}

/// The ALG-033 new side for the full-overwrite schemes (A, C): the
/// production builder's output minus the marker line it was handed —
/// derived through [`build_content`] so no hand-rolled join can drift
/// from ALG-030 (compare.rs pins the markerless form as `managed +
/// "\n\n"`, or the empty string for an empty managed block, which is
/// exactly this tail strip).
fn markerless_overwrite_side(scheme: Scheme, body: &str, lines: &ExtraFileLines) -> String {
    let probe = probe_marker_line();
    let full = build_content(scheme, body, lines, &probe);
    // build_content's contract: the file is the managed block, an
    // empty line, the marker line, one line break — so stripping that
    // known tail is the markerless new side, whatever the managed
    // block was (the empty-managed file is the marker line alone).
    let without_break = full
        .strip_suffix('\n')
        .expect("build_content ends with exactly one line break");
    without_break
        .strip_suffix(probe.as_str())
        .expect("build_content ends with the marker line it was given")
        .to_owned()
}

/// The ALG-033 new side for scheme B: what the next sync would leave
/// on disk, minus its marker line — assembled from the CURRENT file
/// text, because ALG-031 preserves everything outside the markers, so
/// the outside is part of both sides (an edit outside the markers
/// does NOT make a B project externally modified; the build follows
/// the file). [`None`] when the side cannot be built: the bytes are
/// not UTF-8, or the marker pair is missing/invalid.
fn scheme_b_side(bytes: &[u8], body: &str) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let probe = probe_marker_line();
    let assembled = assemble(text, body, &probe).ok()?;
    // The assembled string re-parses (assembly_b's own pinned
    // property); its managed span brackets exactly the block the
    // splice wrote: managed', the empty line, the probe marker, one
    // break in the file's dominant style.
    let span = eunomia_core::markers::parse(&assembled).ok()?;
    let block = assembled.get(span.managed_start..span.managed_end)?;
    let without_break = block
        .strip_suffix("\r\n")
        .or_else(|| block.strip_suffix('\n'))
        .expect("the assembled block ends with one break after the marker");
    let markerless_block = without_break
        .strip_suffix(probe.as_str())
        .expect("the break precedes the marker line the splice just inserted");
    Some(format!(
        "{}{}{}",
        &assembled[..span.managed_start],
        markerless_block,
        &assembled[span.managed_end..]
    ))
}

// ---- the build half ------------------------------------------------------

/// The per-project facts a pass needs from the database side.
struct ProjectFact {
    id: String,
    /// The project's repository root (DB-030's canonical stored
    /// path); the target path is `root` + [`target_file_name`].
    root: PathBuf,
    scheme: Scheme,
    base_name: eunomia_core::project::BaseName,
    extra_local: eunomia_core::project::ExtraFile,
    extra_project: eunomia_core::project::ExtraFile,
    /// A sync recorded a build hash (DB-024; `NULL`/empty ≡ never).
    ever_synced: bool,
}

/// Joins the snapshot's projects (settings, root — ALG-001 order)
/// with the stored hashes (`rows`, keyed by id). Pure.
fn facts_of(snapshot: &BuildSnapshot, rows: &[Project]) -> Vec<ProjectFact> {
    let stored: HashMap<&str, Option<&str>> = rows
        .iter()
        .map(|row| (row.id.as_str(), row.last_synced_hash.as_deref()))
        .collect();
    snapshot
        .projects()
        .iter()
        .map(|project| {
            let settings = &project.settings;
            ProjectFact {
                id: project.id.as_str().to_owned(),
                root: project.root.clone(),
                scheme: settings.scheme,
                base_name: settings.base_name,
                extra_local: settings.extra_local,
                extra_project: settings.extra_project,
                // D-068: an empty hash cell means the same as NULL.
                ever_synced: stored
                    .get(project.id.as_str())
                    .copied()
                    .flatten()
                    .is_some_and(|hash| !hash.is_empty()),
            }
        })
        .collect()
}

/// Builds the §9.2 bodies of the wanted projects over the snapshot,
/// in snapshot order — the shared chunked pass
/// (`eunomia_core::content_build::build_bodies`) over a snapshot
/// scoped to the wanted subset: the same items, only those projects
/// and their assignments (D-121's §12.2 set-L reasoning, owned by
/// core since T-265/D-145). The scoping is body-equivalent to the
/// full snapshot: filtering projects and assignments by project id
/// preserves each project's assignment sequence, which is all
/// `build_body` reads. Pure; CPU-heavy, so the pass runs it on the
/// blocking pool.
fn bodies_of(snapshot: &BuildSnapshot, wanted: &HashSet<&str>) -> Vec<(String, String)> {
    let scoped_projects: Vec<SnapshotProject> = snapshot
        .projects()
        .iter()
        .filter(|project| wanted.contains(project.id.as_str()))
        .cloned()
        .collect();
    if scoped_projects.is_empty() {
        return Vec::new();
    }
    let scoped_assignments: Vec<Assignment> = snapshot
        .assignments()
        .iter()
        .filter(|assignment| wanted.contains(assignment.project_id.as_str()))
        .cloned()
        .collect();
    let scoped = BuildSnapshot::new(
        scoped_projects,
        snapshot.items().to_vec(),
        scoped_assignments,
    )
    .expect("the full snapshot passed the same validation: a subset cannot fail it");
    eunomia_core::content_build::build_bodies(&scoped)
}

// ---- the pass -------------------------------------------------------------

/// The outcome of the blocking half of a pass.
struct PassOutcome {
    /// The new path → project id routing table (full passes only;
    /// partial passes keep the caller's table untouched).
    by_path: Option<HashMap<PathBuf, String>>,
    /// Every project the pass covered — the intended classification
    /// set, verdict or not (an unreadable file contributed no verdict
    /// but IS covered; its last known flags must survive the prune).
    covered: Vec<String>,
    /// The verdicts computed, in snapshot order.
    verdicts: Vec<(String, FileFlags)>,
    /// The wanted-but-unwatched files of this full pass's
    /// `set_files`, keyed by their PROJECT id with the refusal class
    /// that explains each (SYS-010's observable, composed here where
    /// both the watcher's path-keyed answer and the routing table
    /// live; partial passes run no `set_files` and change nothing).
    unwatched: Option<BTreeMap<String, WatchRefusalKind>>,
}

/// The blocking half of every pass: probe availability (the same
/// D-084 probes the availability service runs — the pass's own truth,
/// never a stale cache), converge the watch set (full passes), read
/// the target files, and classify. Runs on the blocking pool
/// (SYS-001); the watcher mutex is taken only by this half and only
/// here — never across an `.await` — and the watcher's own watch map
/// follows its inner discipline (short critical sections, syscalls
/// outside them). Generic over [`WatchBackend`] (the `TargetWatcher`
/// default-parameter shape) so the composite unit proof drives this
/// same half over the fake backend — the limit seam no kernel
/// fixture can produce (D-139's premise) at the layer that consumes
/// it; production instantiates the real `NotifyBackend` through
/// [`spawn_service`].
fn blocking_half<B: WatchBackend>(
    watcher: &Arc<Mutex<TargetWatcher<B>>>,
    snapshot: &BuildSnapshot,
    facts: &[ProjectFact],
    affected: Option<&HashSet<String>>,
) -> PassOutcome {
    let full = affected.is_none();
    // The relevant projects: all of them on a full pass, exactly the
    // affected ids on a debounced partial (whose siblings are not
    // classified there anyway — probing them would spend four
    // metadata probes per project per event for nothing).
    let relevant: Vec<&ProjectFact> = facts
        .iter()
        .filter(|fact| {
            affected
                .map(|ids| ids.contains(fact.id.as_str()))
                .unwrap_or(true)
        })
        .collect();
    // The routing table (and, on full passes, the watch set) is built
    // from the AVAILABLE projects only — SYS-007's removal rule; the
    // probe is this pass's own, so a folder that died a moment ago
    // cannot leave a watch or a verdict behind.
    let available: Vec<&ProjectFact> = relevant
        .iter()
        .copied()
        .filter(|fact| Verdict::probe(&fact.root).available)
        .collect();
    // §9.4 preflight step 2's resolution, once per project: the
    // watched and read path is the REAL file behind a symlinked
    // target (`CLAUDE.md` → `AGENTS.md`, the spec's "common case") —
    // the name an external editor actually changes. A resolution
    // refusal falls back to the plain joined path (target_of's docs).
    let targets: HashMap<String, PathBuf> = available
        .iter()
        .map(|fact| (fact.id.clone(), target_of(fact)))
        .collect();
    let by_path: Option<HashMap<PathBuf, String>> = full.then(|| {
        available
            .iter()
            .map(|fact| {
                (
                    targets
                        .get(fact.id.as_str())
                        .expect("every available fact has a resolved target")
                        .clone(),
                    fact.id.clone(),
                )
            })
            .collect::<HashMap<PathBuf, String>>()
    });
    let mut unwatched: Option<BTreeMap<String, WatchRefusalKind>> = None;
    if let Some(table) = &by_path {
        let files: Vec<PathBuf> = table.keys().cloned().collect();
        let mut watcher = watcher
            .lock()
            .expect("the watcher is locked only inside short synchronous sections");
        watcher.set_files(&files);
        // SYS-010's observable: the wanted files the backend refused
        // (an unwatchable root, the inotify limit), WITH the refusal
        // class that explains each — re-keyed by project id here, the
        // one seam where both the watcher's path-keyed answer and the
        // path→id routing table live (a total mapping: every
        // unwatched file came from the wanted table `set_files` was
        // handed).
        unwatched = Some(
            watcher
                .unwatched_kinds()
                .into_iter()
                .map(|(path, kind)| {
                    (
                        table
                            .get(&path)
                            .expect("every unwatched file came from the wanted table")
                            .clone(),
                        kind,
                    )
                })
                .collect(),
        );
    }
    let classified = available;
    let wanted: HashSet<&str> = classified.iter().map(|fact| fact.id.as_str()).collect();
    let bodies: HashMap<String, String> = bodies_of(snapshot, &wanted).into_iter().collect();
    let mut verdicts = Vec::with_capacity(classified.len());
    for fact in &classified {
        let Some(body) = bodies.get(fact.id.as_str()) else {
            continue;
        };
        // Project-then-local — §7.5's tie order and the signature's
        // argument order, the reverse of `ProjectWriteSettings`'s
        // field order (`extra_local` is declared first; the two
        // arguments share a type, so the order lives only here).
        let lines = extra_file_lines(fact.base_name, fact.extra_project, fact.extra_local);
        let target = targets
            .get(fact.id.as_str())
            .expect("every classified fact has a resolved target");
        let disk = match std::fs::read(target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_unseen) => {
                // An unreadable file (permissions, a dying drive) is
                // not a verdict: keep the last known state and let
                // the next pass try again — claiming "deleted" or
                // "matches" here would be a guess. Staying OUT of the
                // verdicts is what keeps the promise: the prune set
                // is the covered ids, not the classified-successful
                // ones (record's docs).
                continue;
            }
        };
        let verdict =
            classify_target_file(fact.scheme, body, &lines, disk.as_deref(), fact.ever_synced);
        verdicts.push((fact.id.clone(), verdict.flags()));
    }
    PassOutcome {
        by_path,
        // The covered ids: every project the pass intended to
        // classify, verdict or not — the prune set of a full record.
        covered: classified
            .iter()
            .map(|fact| fact.id.clone())
            .collect::<Vec<String>>(),
        verdicts,
        unwatched,
    }
}

/// The watched and read path of one project's target: §9.4 preflight
/// step 2's resolution — a symlinked target (`CLAUDE.md` points at
/// `AGENTS.md`, the spec's "common case") resolves to the REAL file,
/// so the watch sits on the name external editors actually change and
/// the read sees the bytes a sync would compare (an event delivered
/// for the link spelling would never fire: §9.5 writes and atomic
/// saves rename over the real file). A resolution refusal (a broken
/// link, a target escaping the root, a failed probe) falls back to
/// the plain joined path — for regular and absent files the resolved
/// path IS the joined path, so nothing regresses, and a genuinely
/// refused target reports its state honestly on the next pass.
fn target_of(fact: &ProjectFact) -> PathBuf {
    let name = target_file_name(fact.scheme, fact.base_name);
    match resolve_target_path(&fact.root, name) {
        Ok(resolved) => resolved.path().to_owned(),
        Err(_) => fact.root.join(name),
    }
}

/// The pass every trigger shares (the service task's own body):
/// read the snapshot and the stored hashes in one transaction
/// (SYS-005/D-105 — both halves must describe one committed
/// moment), run the blocking half on the pool, then diff, emit and
/// record. `affected: None` is the full pass (subscriptions + every
/// available project); `Some(ids)` re-checks only those projects and
/// leaves the routing table and the pruning alone. Generic over
/// [`WatchBackend`] only so the composite unit proof drives this
/// half's pieces over the fake backend; production runs the real
/// backend.
///
/// Failures propagate in the §7.10 `AppError` shape and emit
/// nothing; nothing is recorded either, so the next trigger re-runs
/// the pass (at-least-once). A failed emit is a missed update, not
/// lost state: the cache deliberately stays unrecorded and the
/// [`logging::file_watch_emit_failed`] warn marks it. A full pass's
/// refused-watch set is recorded here and, when a non-empty answer
/// differs from the previously recorded one, the one-line
/// [`logging::file_watch_refused`] warn opens the degradation
/// episode ([`refused_warn_counts`] — the gate).
async fn refresh_pass<B: WatchBackend + Send + 'static, R: Runtime>(
    app: &AppHandle<R>,
    watcher: &Arc<Mutex<TargetWatcher<B>>>,
    by_path: &mut HashMap<PathBuf, String>,
    affected: Option<&HashSet<String>>,
) -> Result<Vec<ProjectFileChange>, AppError> {
    let database = managed_db(app)?;
    let (snapshot, rows) = database
        .run(move |transaction| {
            let snapshot = read_build_snapshot(transaction).map_err(build_snapshot_to_app_error)?;
            let rows = eunomia_store::projects::list(transaction, ListScope::Active)
                .map_err(projects_store_to_app_error)?;
            Ok((snapshot, rows))
        })
        .await?;
    let facts = facts_of(&snapshot, &rows);
    let snapshot = Arc::new(snapshot);
    let pool_watcher = Arc::clone(watcher);
    let affected = affected.cloned();
    // Captured before the move: the recording pass below needs to
    // know whether this was a full pass (pruning) after the closure
    // consumed the set.
    let full_pass = affected.is_none();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        blocking_half(&pool_watcher, &snapshot, &facts, affected.as_ref())
    })
    .await
    .map_err(join_failure_to_app_error)?;

    if let Some(ref new_table) = outcome.by_path {
        *by_path = new_table.clone();
    }
    let state = app.state::<FileWatchState>();
    let changes = finish_pass(&state, outcome, full_pass, |event| event.emit(app));
    Ok(changes)
}

/// The post-pass bookkeeping every pass shares — diff, deliver,
/// record — factored around an injectable emitter (the round-2 F-1
/// fix) so the unit suite can drive the production sequence,
/// failing emitter included:
///
/// - The refused-watch fact is recorded and its episode gated BEFORE
///   the delivery, deliberately outside the at-least-once verdict
///   discipline: a refused watch is a fact about the WATCH, not about
///   any file's content, so a failed diff emit must neither drop the
///   episode's warn nor leave the notice one pass stale (the round-1
///   P2-1 ordering — pinned by the failing-emitter test, which is the
///   reason this function exists).
/// - The diff is delivered through `emit`; a failure logs the
///   missed-update warn and deliberately records NOTHING — the cache
///   stays stale so the next trigger re-detects and re-emits
///   (at-least-once).
/// - A delivered diff (or an empty one) records the verdicts, a full
///   pass pruning to its covered set.
fn finish_pass<F>(
    state: &FileWatchState,
    outcome: PassOutcome,
    full_pass: bool,
    emit: F,
) -> Vec<ProjectFileChange>
where
    F: FnOnce(&FileChangedEvent) -> Result<(), tauri::Error>,
{
    if let Some(unwatched) = outcome.unwatched
        && let Some((gone, limit, other)) = record_refusals(state, unwatched)
    {
        logging::file_watch_refused(gone, limit, other);
    }
    let changes = state.diff_against_last(&outcome.verdicts);
    if !changes.is_empty() {
        let event = FileChangedEvent {
            changes: changes.clone(),
        };
        if let Err(error) = emit(&event) {
            logging::file_watch_emit_failed(&error);
            return changes;
        }
    }
    state.record(
        &outcome.verdicts,
        if full_pass {
            Some(&outcome.covered)
        } else {
            None
        },
    );
    changes
}

/// Records a full pass's refused-watch set into the session state and
/// answers whether the pass opened (or reshaped) a degradation
/// episode — the transition [`logging::file_watch_refused`] warns on
/// ([`refused_warn_counts`] is the pure decision). Extracted as one
/// step so the composite proof drives the production sequence
/// (record, then gate on the swap's previous) instead of mirroring it
/// (the round-1 P3-3 fix): a dropped or reordered half fails the
/// unit suite, not just the code reader.
fn record_refusals(
    state: &FileWatchState,
    unwatched: BTreeMap<String, WatchRefusalKind>,
) -> Option<(usize, usize, usize)> {
    let previous = state.note_unwatched(unwatched.clone());
    refused_warn_counts(&unwatched, &previous)
}

/// The refused-watch warn's decision and payload (D-155): a full pass
/// warns through [`logging::file_watch_refused`] only when its
/// refused map is NON-EMPTY and differs from the previously recorded
/// one — the transition opens (or reshapes) a degradation episode,
/// the `log_rotation_failed` gate — and the payload is the per-class
/// counts. A standing refusal (`new == previous`, both non-empty)
/// repeats nothing; a recovered pass (`new` empty) logs nothing at
/// all: the no-line policy for converged passes, the healed
/// convergence speaks for itself. Pure over the two maps so the gate
/// is unit-testable apart from the service task the warn fires from
/// (the thread-local subscriber cannot capture the runtime's
/// threads — the D-032/D-143 boundary; the line itself is tested in
/// `logging.rs`, the call site is a one-line forward).
fn refused_warn_counts(
    new: &BTreeMap<String, WatchRefusalKind>,
    previous: &BTreeMap<String, WatchRefusalKind>,
) -> Option<(usize, usize, usize)> {
    if new.is_empty() || new == previous {
        return None;
    }
    let count = |kind: WatchRefusalKind| new.values().filter(|k| **k == kind).count();
    Some((
        count(WatchRefusalKind::Gone),
        count(WatchRefusalKind::Limit),
        count(WatchRefusalKind::Other),
    ))
}

/// The wire spelling of a refusal class — the closed vocabulary the
/// SYS-010 non-blocking notice serves per row (`"gone" | "limit" |
/// "other"`, the `unavailable_reason` precedent for a closed
/// vocabulary on the `projects_list` wire). Single-sourced here so
/// the notice carrier and any later consumer cannot drift.
pub(crate) fn refusal_wire_class(kind: WatchRefusalKind) -> &'static str {
    match kind {
        WatchRefusalKind::Gone => "gone",
        WatchRefusalKind::Limit => "limit",
        WatchRefusalKind::Other => "other",
    }
}

// ---- the service ----------------------------------------------------------

/// Subscribes to T-079's `state://project-availability` event: every
/// diff drops a `Refresh` permit, so the watch set follows folders
/// that die and return (SYS-007's add/remove rule). Called in the
/// `setup` hook BEFORE the startup availability recheck is spawned —
/// Rust listeners deliver synchronously inside `emit`, so a listener
/// registered earlier cannot miss the session's first verdicts.
pub(crate) fn listen_availability<R: Runtime>(app: &AppHandle<R>) {
    let trigger = app.state::<FileWatchState>().trigger.clone();
    <ProjectAvailabilityEvent as Event>::listen(app, move |_diff| {
        let _ = trigger.try_send(Command::Refresh { done: None });
    });
}

/// The pump's one step, extracted after the watcher's own `dispatch`
/// precedent (D-139, its round-1 P2-2 follow-up) so the mapping is
/// drivable without notify's event thread: one debounced
/// [`WatchEvent`] becomes the command the service task consumes on
/// the bus. A `Changed`/`Removed` delivery names exactly its own
/// file — the partial re-read's input, SYS-008's noise filter. The
/// backend's LOST-EVENTS signal ([`WatchEvent::Backend`], SYS-010: an
/// event-queue overflow — delivered by inotify as a rescan flag
/// riding an ordinary batch — or a stream-error batch alike, D-139)
/// is the one full re-check of everything, FIRE-AND-FORGET:
/// `done: None`, because no caller waits on this trigger's outcome
/// (both awaited seams build their own ack pairs).
///
/// The send is a `try_send` onto the bounded bus: a FULL bus drops
/// the delivery instead of blocking the pump behind a slow pass (the
/// drop is covered by the next refresh point, the reconciliation
/// anchor — see [`BUS_CAPACITY`]), and a CLOSED bus (the service task
/// gone at app teardown) drops it too — never a panic, the same
/// delivery-while-the-consumer-lives contract as the watcher's sink.
fn pump_event(event: WatchEvent, trigger: &tauri::async_runtime::Sender<Command>) {
    let command = match event {
        WatchEvent::Changed(path) | WatchEvent::Removed(path) => Command::FilesChanged(vec![path]),
        // SYS-010's degrade: one full re-check of everything.
        WatchEvent::Backend => Command::Refresh { done: None },
    };
    let _ = trigger.try_send(command);
}

/// Spawns the service: the fs watcher (failure to start it degrades —
/// no watching this session, the launch unaffected, SYS-010's
/// no-crash posture; the diagnostic line is
/// [`logging::file_watch_init_failed`], logged by the `lib.rs`
/// degrade arm), the pump thread that turns debounced
/// [`WatchEvent`]s into commands, and the single consumer task that
/// serializes every pass (the no-ticket guarantee of the diff
/// contract). The startup `Refresh` permit is dropped before the
/// spawn (the dirty service's precedent).
///
/// # Errors
///
/// [`WatcherInitError`] when the watcher backend cannot be
/// constructed: nothing is spawned, the bus's receiving half is
/// dropped (later `try_send`s fail silently — the intended degrade).
pub(crate) fn spawn_service<R: Runtime>(
    app: AppHandle<R>,
    commands: tauri::async_runtime::Receiver<Command>,
    debounce: Duration,
) -> Result<(), WatcherInitError> {
    let (event_sender, event_receiver) = std::sync::mpsc::channel::<WatchEvent>();
    let watcher = Arc::new(Mutex::new(TargetWatcher::new(debounce, event_sender)?));
    app.state::<FileWatchState>()
        .trigger(Command::Refresh { done: None });

    // The pump: the debouncer's closure delivers onto a std channel
    // from its own thread; this thread is the async side's bridge.
    // It exits when the watcher drops (the channel closes) — with the
    // service task at app teardown, or never in between. The
    // event→command step itself lives in [`pump_event`], extracted so
    // the mapping is testable without notify's event thread (the
    // `dispatch` precedent, D-139).
    let pump_trigger = app.state::<FileWatchState>().trigger.clone();
    std::thread::spawn(move || {
        while let Ok(event) = event_receiver.recv() {
            pump_event(event, &pump_trigger);
        }
    });

    let service_watcher = Arc::clone(&watcher);
    tauri::async_runtime::spawn(async move {
        let mut commands = commands;
        let mut by_path: HashMap<PathBuf, String> = HashMap::new();
        while let Some(command) = commands.recv().await {
            match command {
                Command::Refresh { done } => {
                    let result = refresh_pass(&app, &service_watcher, &mut by_path, None).await;
                    // The §10 every-error line, only for the
                    // fire-and-forget trigger: an awaited failure's
                    // durable half is the CALLER's (the sync
                    // pipeline's `sync_run_degraded` watch arm; the
                    // add flow's line is T-278's card), so every
                    // failure is logged exactly once (D-155).
                    if let (Err(error), None) = (&result, done.as_ref()) {
                        logging::file_watch_pass_failed(error);
                    }
                    app.state::<FileWatchState>().note_pass();
                    if let Some(done) = done {
                        let _ = done.try_send(result);
                    }
                }
                Command::FilesChanged(paths) => {
                    // Coalesce: batches queued behind this one collapse
                    // into a single re-read; a Refresh queued behind
                    // file events SUBSUMES them (it re-checks
                    // everything) and runs instead — whether or not it
                    // wants an ack. `refresh_seen` is tracked apart
                    // from the ack sender precisely because a
                    // fire-and-forget Refresh carries `done: None`:
                    // storing the sender alone would collapse "a
                    // Refresh happened" into "an ack is owed" and
                    // silently downgrade the availability listener's
                    // and the SYS-010 lost-events re-checks into a
                    // partial pass (the round-1 review's P1-2).
                    let mut all_paths = paths;
                    let mut refresh_seen = false;
                    let mut refresh_done = None;
                    while let Ok(next) = commands.try_recv() {
                        match next {
                            Command::FilesChanged(mut more) => all_paths.append(&mut more),
                            Command::Refresh { done } => {
                                refresh_seen = true;
                                refresh_done = done;
                                break;
                            }
                        }
                    }
                    if refresh_seen {
                        let result = refresh_pass(&app, &service_watcher, &mut by_path, None).await;
                        // Same every-error-once rule as the Refresh
                        // arm: warn here only when no caller awaits
                        // the failure (D-155).
                        if let (Err(error), None) = (&result, refresh_done.as_ref()) {
                            logging::file_watch_pass_failed(error);
                        }
                        app.state::<FileWatchState>().note_pass();
                        if let Some(done) = refresh_done {
                            let _ = done.try_send(result);
                        }
                        continue;
                    }
                    let affected: HashSet<String> = all_paths
                        .iter()
                        .filter_map(|path| by_path.get(path).cloned())
                        .collect();
                    // Empty after routing = noise (events for files
                    // no longer in the set): no pass, no read.
                    if !affected.is_empty() {
                        // A partial pass carries no ack by construction
                        // — fire-and-forget like its trigger, so the
                        // durable half of a failure is here (D-155).
                        if let Err(error) =
                            refresh_pass(&app, &service_watcher, &mut by_path, Some(&affected))
                                .await
                        {
                            logging::file_watch_pass_failed(&error);
                        }
                        app.state::<FileWatchState>().note_pass();
                    }
                }
            }
        }
    });
    Ok(())
}

/// The after-sync seam (SYS-008's "right after a sync"; the §9.4
/// pipeline's T-118 entry): one FULL pass — subscriptions re-converged
/// (a sync may have created files), every available project's verdict
/// recomputed, `written`/`unchanged` projects therefore clean again —
/// awaited through the service task's own ack channel, so the caller
/// sees the computed diff when the pass completes. A failed emit is
/// NOT the caller's problem to detect: the diff is returned anyway,
/// the cache deliberately stays unrecorded, and the next trigger
/// re-detects and re-emits it (the at-least-once contract — the same
/// posture as `dirty_recalc`'s seam). The project-mutation commands
/// refresh through the BOUNDED twin of this seam
/// ([`refresh_after_mutation`], T-280/D-156): the same `Refresh`
/// command and ack, but an interactive answer that waits at most a
/// budget instead of as long as a stuck mount — one refresh path,
/// two wait policies (`project_add_manual`'s committed arm since
/// T-280; the scheme/name/archive/restore commands join as they
/// land).
///
/// # Errors
///
/// The pass's [`AppError`]. Recovery mode (no `Db` managed) is
/// answered HERE, before anything is sent: the service is never
/// spawned there, so waiting for its ack would be waiting for a
/// courier that does not exist — the honest `NoDatabase` answer
/// through the [`managed_db`] edge instead, like every database
/// command. If the service task itself is gone while its channel
/// still accepts the send (a reasoned-impossible panic;
/// blocking-half panics surface as join errors instead), the ack
/// never arrives and the await never returns — the reasoned-away
/// branch, stated here so it is not discovered by a hang. The
/// callers treat the refusal as never-fatal aftermath (the sync
/// pipeline logs its class-only §10 code; the add flow degrades
/// silently — its `logging.rs` line is the recorded follow-up).
pub(crate) async fn refresh_after_sync<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ProjectFileChange>, AppError> {
    // The recovery-mode fast answer (module docs): the check is the
    // command edge's own, so the seam is honest with or without the
    // service task behind it.
    managed_db(app)?;
    let (done_sender, mut done) =
        tauri::async_runtime::channel::<Result<Vec<ProjectFileChange>, AppError>>(1);
    send_refresh_command(app, done_sender).await?;
    await_ack(&mut done).await
}

/// The bounded twin of [`refresh_after_sync`] for INTERACTIVE
/// mutation callers (T-280/D-156): the same `Refresh` command, the
/// same ack channel, the same single service pass — but the caller
/// waits at most [`FileWatchState`]'s budget
/// ([`MUTATION_REFRESH_BUDGET`] in production) for the answer,
/// because an interactive command's response must not hang on a
/// stuck mount the pass probes. The budget bounds the WHOLE
/// send-and-await region: a full bus backpressures the send behind a
/// hung pass just as it would delay the ack, and bounding only half
/// the region would leave the unbounded path (D-156).
///
/// # Degrade (the honest reading, D-156)
///
/// On expiry the caller answers [`mutation_refresh_timeout`] — the
/// class-only "the pass did not answer in time" error — and the
/// refresh CONTINUES IN THE BACKGROUND, never cancelled: an
/// enqueued command stays queued (the service runs the pass whenever
/// it becomes free; its ack then fires into this caller's dropped
/// receiver and is ignored by the service's own `let _`), and if the
/// bus was full the send was dropped and never enqueued — that
/// refresh is honestly abandoned. Either way convergence is the
/// anchor set's, not the wait's: every full pass re-derives the
/// watch set from the project table (D-144's idempotent
/// convergence), so the next anchor re-derives it without loss.
/// Cancelling the walk is deliberately NOT offered — for an ADD the
/// verdict inputs the user just created matter, and a walk the
/// service can still finish is worth more than a walk nobody ran.
///
/// # Errors
///
/// The pass's [`AppError`] when the pass answered within the budget;
/// [`send_error()`]'s `FileWatchGone` when the bus itself is gone
/// (no service — the fast refusal, unchanged by the budget);
/// [`mutation_refresh_timeout`] when the budget expires. Recovery
/// mode is answered HERE exactly like [`refresh_after_sync`] — the
/// honest `NoDatabase` through [`managed_db`], never a wait.
pub(crate) async fn refresh_after_mutation<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ProjectFileChange>, AppError> {
    managed_db(app)?;
    let budget = app.state::<FileWatchState>().mutation_budget;
    let (done_sender, mut done) =
        tauri::async_runtime::channel::<Result<Vec<ProjectFileChange>, AppError>>(1);
    // The region bounded is send + ack BOTH (the doc above): one
    // timeout wraps them together, so a backpressured send degrades
    // at the same budget the ack would. `tokio::time` is already a
    // dependency of this crate (D-054's retention tick; no new
    // feature crosses the rule 9 line — D-156).
    let bounded = async {
        send_refresh_command(app, done_sender).await?;
        await_ack(&mut done).await
    };
    match tokio::time::timeout(budget, bounded).await {
        Ok(result) => result,
        Err(_expired) => Err(mutation_refresh_timeout()),
    }
}

/// The command send both seams share: one `Refresh` with an ack, onto
/// the single service task's bus — there is no second refresh path
/// (D-144), only a second WAIT POLICY ([`refresh_after_mutation`]).
async fn send_refresh_command<R: Runtime>(
    app: &AppHandle<R>,
    done: tauri::async_runtime::Sender<Result<Vec<ProjectFileChange>, AppError>>,
) -> Result<(), AppError> {
    app.state::<FileWatchState>()
        .trigger
        .clone()
        .send(Command::Refresh { done: Some(done) })
        .await
        .map_err(|_send| send_error())
}

/// The ack await both seams share: the pass's own result, or the
/// honest "the service is not answering" fallback when the channel
/// closed first.
async fn await_ack(
    done: &mut tauri::async_runtime::Receiver<Result<Vec<ProjectFileChange>, AppError>>,
) -> Result<Vec<ProjectFileChange>, AppError> {
    match done.recv().await {
        Some(result) => result,
        None => Err(send_error()),
    }
}

/// The honest "the watch service is not answering" fallback — the
/// D-037 class-only diagnostic shape.
fn send_error() -> AppError {
    let mut error = AppError::new(crate::errors::ErrorCode::DbLocked);
    error.details = Some(crate::errors::ErrorDetails {
        os_code: None,
        os_kind: Some("FileWatchGone".to_owned()),
        os_message: Some("the file-watch service is not running".to_owned()),
    });
    error
}

/// The bounded mutation seam's expiry answer — [`send_error`]'s
/// D-037 class-only shape with its own discriminable kind: the
/// service exists, the pass simply did not answer within the
/// budget (the stuck-mount world; T-280/D-156). Never logged by
/// this seam: the add flow degrades silently and the refresh-refused
/// warn line is T-278's card — whether that line also covers this
/// timeout class is that card's decision (D-156's boundary).
fn mutation_refresh_timeout() -> AppError {
    let mut error = AppError::new(crate::errors::ErrorCode::DbLocked);
    error.details = Some(crate::errors::ErrorDetails {
        os_code: None,
        os_kind: Some("FileWatchTimeout".to_owned()),
        os_message: Some("the watch pass did not answer within the mutation budget".to_owned()),
    });
    error
}

#[cfg(test)]
mod tests {
    //! The pure halves of the service — the SYS-008 verdict over
    //! literal file bytes, the diff semantics over the clean
    //! baseline, the wire name, the debounce literal, the pump's
    //! event→command mapping over a bounded channel, and the SYS-010
    //! refused-watch halves (the recording/swap, the warn gate, the
    //! wire vocabulary, and the composite proof over the fake
    //! backend). The mock-runtime halves (the card's criteria, the
    //! triggers, the subscription changes, the notice carrier) live
    //! in `ipc_tests.rs`, which owns the mock-app harness.

    use std::path::Path;

    use super::*;
    use eunomia_core::extra_files::ExtraFileLines;
    use eunomia_core::library::{
        Instruction, InstructionState, ItemId, LibraryItem, Variant, VariantId,
    };
    use eunomia_core::marker::MarkerTime;
    use eunomia_core::project::{
        BaseName, ExtraFile, ExtraFilePosition, ProjectId, ProjectWriteSettings,
    };
    use eunomia_core::snapshot::SnapshotProject;

    // ---- the wire name and the debounce literal ------------------------

    #[test]
    fn the_event_name_is_the_spec_literal() {
        // The wire name is the frontend's whole contract; pinned
        // against the §7.10/§5.7 literal exactly like the other two
        // state events.
        assert_eq!(FileChangedEvent::NAME, "state://file-changed");
    }

    #[test]
    fn the_debounce_is_the_sys_007_literal() {
        // SYS-007 fixes 300 ms; the constant is production's only
        // spelling of it (the service's parameter exists for the
        // tests' short windows).
        assert_eq!(DEBOUNCE, Duration::from_millis(300));
    }

    #[test]
    fn the_mutation_budget_is_the_recorded_ten_seconds() {
        // D-156's recorded value: an order of magnitude above any
        // healthy full pass's worst case (no false degrade under
        // load), inside the interactive-patience band. Pinned so the
        // value cannot drift silently — changing it is a decision,
        // and the decision trail lives in DECISIONS.md.
        assert_eq!(MUTATION_REFRESH_BUDGET, Duration::from_secs(10));
    }

    #[test]
    fn the_state_defaults_to_the_recorded_mutation_budget() {
        // Production constructs through `new()`; the constructor's
        // default IS the constant (the knob changes only what a test
        // explicitly overrides).
        assert_eq!(state_only().mutation_budget, MUTATION_REFRESH_BUDGET);
        assert_eq!(
            FileWatchState::with_budget(Duration::from_millis(1))
                .0
                .mutation_budget,
            Duration::from_millis(1)
        );
    }

    // ---- the diff semantics (D-112's shape over the clean baseline) ---

    /// The state without its receiver — the pure half's fixture.
    fn state_only() -> FileWatchState {
        FileWatchState::new().0
    }

    fn verdict(id: &str, flags: FileFlags) -> (String, FileFlags) {
        (id.to_owned(), flags)
    }

    fn modified() -> FileFlags {
        FileFlags {
            external_changed: true,
            file_deleted: false,
        }
    }

    fn deleted() -> FileFlags {
        FileFlags {
            external_changed: false,
            file_deleted: true,
        }
    }

    fn change(id: &str, flags: FileFlags) -> ProjectFileChange {
        ProjectFileChange {
            project_id: id.to_owned(),
            external_changed: flags.external_changed,
            file_deleted: flags.file_deleted,
        }
    }

    #[test]
    fn a_first_observation_of_a_clean_project_is_no_change() {
        // The baseline rule: an unobserved project counts as clean, so
        // an all-healthy startup has nothing to report.
        let state = state_only();
        assert_eq!(state.diff_against_last(&[verdict("p1", CLEAN)]), Vec::new());
    }

    #[test]
    fn first_observations_of_modified_and_deleted_projects_are_changes() {
        // SYS-008's startup warning and SYS-009's hint both surface on
        // the very first pass — no second pass may be needed.
        let state = state_only();
        assert_eq!(
            state.diff_against_last(&[
                verdict("p1", modified()),
                verdict("p2", deleted()),
                verdict("p3", CLEAN),
            ]),
            vec![change("p1", modified()), change("p2", deleted())]
        );
    }

    #[test]
    fn a_changed_verdict_is_reported_in_both_directions() {
        // modified → clean (a sync rewrote the file: MAIN-028's icon
        // must clear) and clean → modified (the external edit) are
        // both changes the event carries.
        let state = state_only();
        state.record(&[verdict("p1", modified())], None);
        assert_eq!(
            state.diff_against_last(&[verdict("p1", CLEAN)]),
            vec![change("p1", CLEAN)]
        );
        state.record(&[verdict("p1", CLEAN)], None);
        assert_eq!(
            state.diff_against_last(&[verdict("p1", modified())]),
            vec![change("p1", modified())]
        );
    }

    #[test]
    fn an_unchanged_verdict_is_not_a_change_even_flagged() {
        // Idempotence: repeated passes over an unchanged world are
        // silent — what makes the debounced re-checks cheap and safe.
        let state = state_only();
        state.record(&[verdict("p1", modified()), verdict("p2", deleted())], None);
        assert_eq!(
            state.diff_against_last(&[verdict("p1", modified()), verdict("p2", deleted())]),
            Vec::new()
        );
    }

    #[test]
    fn a_full_record_prunes_but_a_partial_record_merges() {
        // The two recording rules: a full pass ends with exactly the
        // covered world (an archived/unavailable project's entry
        // goes, and its return is a FIRST observation), while a
        // debounced partial pass touches only its projects — dropping
        // the rest would reset live verdicts to the baseline.
        let state = state_only();
        let covered =
            |ids: &[&str]| -> Vec<String> { ids.iter().map(|id| (*id).to_owned()).collect() };
        state.record(
            &[verdict("p1", modified()), verdict("p2", deleted())],
            Some(&covered(&["p1", "p2"])),
        );
        // Partial: p1 re-checked clean; p2 was not re-read. The proof
        // the merge kept p2: re-reporting p2's recorded verdict is NO
        // change — it would be a change (against the clean baseline)
        // had the partial pass dropped it.
        state.record(&[verdict("p1", CLEAN)], None);
        assert_eq!(
            state.diff_against_last(&[verdict("p2", deleted())]),
            Vec::new(),
            "p2 was not re-observed; its recorded verdict stands"
        );
        assert_eq!(
            state.diff_against_last(&[verdict("p1", CLEAN)]),
            Vec::new(),
            "the partial pass recorded p1's fresh verdict"
        );
        // Full over a smaller world: p2 left the watch set (archived,
        // unavailable) — not covered, so pruned; its return looks
        // first-observed.
        state.record(&[verdict("p1", CLEAN)], Some(&covered(&["p1"])));
        assert_eq!(
            state.diff_against_last(&[verdict("p1", CLEAN), verdict("p2", deleted())]),
            vec![change("p2", deleted())],
            "the pruned project is a first observation again, not an \
             unchanged verdict"
        );
    }

    #[test]
    fn a_full_record_prunes_to_the_covered_not_the_successful() {
        // The round-1 review's P2-3 pin: a project whose file could
        // not be read contributed NO verdict but WAS covered — its
        // last known flags must survive the full pass's prune. Were
        // the prune set the verdict ids, the entry would reset to the
        // clean baseline and the later recovery (the file readable
        // again and matching the build) would emit NO clearing event
        // — a column stuck on "externally modified" for the session.
        let state = state_only();
        state.record(
            &[verdict("p1", modified()), verdict("p2", modified())],
            Some(&["p1".to_owned(), "p2".to_owned()]),
        );
        // A full pass covering both, but p2's read failed: only p1
        // reports (clean — the sync rewrote it).
        state.record(
            &[verdict("p1", CLEAN)],
            Some(&["p1".to_owned(), "p2".to_owned()]),
        );
        // p2's stuck verdict survived: when its file becomes readable
        // again and matches, the clearing IS a change against the
        // recorded (not baseline-reset) state.
        assert_eq!(
            state.diff_against_last(&[verdict("p1", CLEAN), verdict("p2", CLEAN)]),
            vec![change("p2", CLEAN)],
            "p2's recorded modified verdict survived the prune; its \
             recovery emits the clearing event"
        );
        // And the snapshot carrier (P1-1's read side) serves the
        // same survival: p2 still shows its last known flags.
        assert_eq!(
            state.snapshot(),
            vec![change("p1", CLEAN), change("p2", modified())]
        );
    }

    #[test]
    fn the_snapshot_serves_the_recorded_world_in_id_order() {
        // The P1-1 enabler: the future column carrier reads the
        // session's verdicts (events are not replayed to late
        // listeners); the shape is the event's own, the order is
        // stable (id-sorted) so the consumer can merge by id.
        let state = state_only();
        assert!(
            state.snapshot().is_empty(),
            "an unobserved session has no verdicts to serve"
        );
        state.record(
            &[
                verdict("p2", deleted()),
                verdict("p1", modified()),
                verdict("p3", CLEAN),
            ],
            None,
        );
        assert_eq!(
            state.snapshot(),
            vec![
                change("p1", modified()),
                change("p2", deleted()),
                change("p3", CLEAN)
            ]
        );
    }

    #[test]
    fn the_changes_keep_the_snapshot_order() {
        // The diff is in `sort_order`, so the UI applies changes in
        // column order — pinned against the HashMap cache
        // deliberately.
        let state = state_only();
        state.record(
            &[
                verdict("p1", CLEAN),
                verdict("p2", CLEAN),
                verdict("p3", CLEAN),
            ],
            None,
        );
        assert_eq!(
            state.diff_against_last(&[
                verdict("p1", CLEAN),
                verdict("p2", modified()),
                verdict("p3", deleted()),
            ]),
            vec![change("p2", modified()), change("p3", deleted())]
        );
    }

    #[test]
    fn the_unwatched_recording_is_served_and_swapped_across_full_passes() {
        // SYS-010's observable seam (D-155's id-keyed form): the
        // service records the refused-watch gap after each full pass
        // (`note_unwatched`), keyed by project id WITH the refusal
        // class — the shape both consumers read (the warn gate counts
        // the classes, the notice serves them per row). The swap
        // returns the PREVIOUS recording (the episode gate's input);
        // empty before the first pass, the last full pass's answer
        // afterwards, cleared by a later healthy convergence.
        let state = state_only();
        assert!(
            state.unwatched_projects().is_empty(),
            "no full pass yet — no gap to report"
        );
        let refused = BTreeMap::from([("p1".to_owned(), WatchRefusalKind::Gone)]);
        let previous = state.note_unwatched(refused.clone());
        assert!(
            previous.is_empty(),
            "the first recording swaps out the empty baseline — the gate \
             sees a new episode"
        );
        assert_eq!(
            state.unwatched_projects(),
            refused,
            "the refused watch is the served observable, class included"
        );
        let previous = state.note_unwatched(BTreeMap::new());
        assert_eq!(
            previous, refused,
            "the swap hands back the recording it replaced"
        );
        assert!(
            state.unwatched_projects().is_empty(),
            "a later healthy full pass clears the gap"
        );
    }

    #[test]
    fn the_warn_gate_opens_only_on_a_changed_non_empty_refused_set() {
        // The refused-watch warn's decision, pure over the two maps
        // (D-155, the `log_rotation_failed` episode precedent): a
        // NON-EMPTY answer that DIFFERS from the previous recording
        // warns with the per-class counts; a standing refusal repeats
        // nothing; a converged or recovered pass (empty) logs nothing
        // at all — the no-line policy for converged passes.
        let empty = BTreeMap::new();
        let one_limit = BTreeMap::from([("p1".to_owned(), WatchRefusalKind::Limit)]);
        let one_gone = BTreeMap::from([("p1".to_owned(), WatchRefusalKind::Gone)]);
        let two = BTreeMap::from([
            ("p1".to_owned(), WatchRefusalKind::Limit),
            ("p2".to_owned(), WatchRefusalKind::Gone),
            ("p3".to_owned(), WatchRefusalKind::Other),
        ]);
        // The episode transitions that DO warn, with their counts.
        assert_eq!(
            refused_warn_counts(&one_limit, &empty),
            Some((0, 1, 0)),
            "the first refusal opens the episode with its class count"
        );
        assert_eq!(
            refused_warn_counts(&two, &one_limit),
            Some((1, 1, 1)),
            "a reshaped refusal is a new episode — gone/limit/other counted"
        );
        assert_eq!(
            refused_warn_counts(&one_gone, &one_limit),
            Some((1, 0, 0)),
            "a changed class on the same project is a new episode too"
        );
        // The transitions that stay silent.
        assert_eq!(
            refused_warn_counts(&empty, &empty),
            None,
            "a converged world logs nothing"
        );
        assert_eq!(
            refused_warn_counts(&one_limit, &one_limit),
            None,
            "a standing refusal repeats nothing — the episode is already open"
        );
        assert_eq!(
            refused_warn_counts(&two, &two),
            None,
            "an unchanged multi-file refusal repeats nothing"
        );
        assert_eq!(
            refused_warn_counts(&empty, &one_limit),
            None,
            "a recovered pass logs nothing — the healed convergence speaks \
             for itself"
        );
    }

    #[test]
    fn the_wire_refusal_vocabulary_is_the_closed_snake_case_set() {
        // The notice's closed vocabulary (the `unavailable_reason`
        // precedent): exactly the three snake_case spellings of the
        // refusal classes, single-sourced in `refusal_wire_class` so
        // the column and any later consumer cannot drift.
        assert_eq!(refusal_wire_class(WatchRefusalKind::Gone), "gone");
        assert_eq!(refusal_wire_class(WatchRefusalKind::Limit), "limit");
        assert_eq!(refusal_wire_class(WatchRefusalKind::Other), "other");
    }

    // ---- the composite SYS-010 proof over the fake backend ---------------

    /// The composite proof's backend (the D-139 `WatchBackend` seam):
    /// refuses exactly the directories the shared plan names, with
    /// the chosen class — everything else arms. The plan is shared
    /// with the test so the refusal can be lifted with the watcher
    /// already built, exactly the LimitBackend battery's shape.
    struct RefusingBackend {
        plan: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<PathBuf>>>,
        kind: WatchRefusalKind,
    }

    impl WatchBackend for RefusingBackend {
        fn watch(
            &mut self,
            dir: &std::path::Path,
        ) -> Result<(), eunomia_fs::watcher::WatchRefused> {
            if self
                .plan
                .lock()
                .expect("only the watcher calls this backend")
                .contains(dir)
            {
                Err(eunomia_fs::watcher::WatchRefused::new(
                    self.kind,
                    "the synthetic refusal (SYS-010 composite proof)".to_owned(),
                ))
            } else {
                Ok(())
            }
        }

        fn unwatch(&mut self, _dir: &std::path::Path) {}
    }

    /// A repository root that passes the D-084 availability probe —
    /// the pass probes before it watches, so the fixture must be a
    /// real available root (the ipc suite's `repo_dir` shape).
    fn repo_root(temp: &tempfile::TempDir, name: &str) -> PathBuf {
        let root = temp.path().join(name);
        std::fs::create_dir_all(root.join(".git"))
            .expect("creating the repository root and its .git entry must work");
        root
    }

    #[test]
    fn a_limit_refused_target_flows_through_the_real_pass_to_the_gate_and_notice() {
        // The card's composite proof: the REAL `blocking_half` probes,
        // converges and composes over the fake backend's limit seam —
        // the refusal shape no kernel fixture can produce on demand
        // (D-139's premise) and, at the app layer, no real backend
        // can produce at all (D-148's boundary: the availability
        // probe gates the watch set first). A limit-refused target
        // flows to the recording WITH its class, opens exactly one
        // warn episode, is served by the notice seam, and the healed
        // pass clears everything with no second warn.
        let temp = tempfile::tempdir().expect("creating a temp directory must work");
        let healthy_root = repo_root(&temp, "healthy");
        let limited_root = repo_root(&temp, "limited");
        let project = |id: &str, root: &std::path::Path| SnapshotProject {
            id: ProjectId::new(id.to_owned()),
            name: format!("project {id}"),
            root: root.to_owned(),
            settings: ProjectWriteSettings {
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: off(),
                extra_project: off(),
            },
        };
        let snapshot = BuildSnapshot::new(
            vec![
                project("healthy", &healthy_root),
                project("limited", &limited_root),
            ],
            Vec::new(),
            Vec::new(),
        )
        .expect("the fixture is a valid snapshot");
        let facts = facts_of(&snapshot, &[]);
        let plan = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::from([
            limited_root.clone(),
        ])));
        let watcher = Arc::new(Mutex::new(TargetWatcher::with_backend(RefusingBackend {
            plan: std::sync::Arc::clone(&plan),
            kind: WatchRefusalKind::Limit,
        })));

        // The refused pass: the healthy sibling stays clean, the
        // refused target lands in the seam id-keyed with its class.
        let outcome = blocking_half(&watcher, &snapshot, &facts, None);
        let refused = outcome
            .unwatched
            .expect("a full pass composes the SYS-010 seam");
        assert_eq!(
            refused,
            BTreeMap::from([("limited".to_owned(), WatchRefusalKind::Limit)]),
            "exactly the limit-refused target, keyed by its project, class \
             included — the healthy sibling is watched"
        );

        // The recording and the gate, through the production step
        // (record_refusals — the same call refresh_pass makes, the
        // round-1 P3-3 fix): one episode opens, a repeated pass under
        // the standing refusal adds no second warn.
        let state = state_only();
        assert_eq!(
            record_refusals(&state, refused.clone()),
            Some((0, 1, 0)),
            "the first refused pass opens the warn episode with the class \
             count — the facade line the service task fires"
        );
        let outcome = blocking_half(&watcher, &snapshot, &facts, None);
        let refused_again = outcome
            .unwatched
            .expect("a full pass composes the SYS-010 seam");
        assert_eq!(
            record_refusals(&state, refused_again),
            None,
            "the standing refusal repeats nothing"
        );

        // The notice seam: the poll-able read serves the loss with
        // its class — the data the `projects_list` carrier maps onto
        // the row.
        assert_eq!(
            state.unwatched_projects(),
            refused,
            "the notice reads exactly what the pass recorded"
        );

        // The healed pass: the plan lifts, everything arms, the
        // recording clears and the gate stays silent — the converged
        // no-line policy.
        plan.lock()
            .expect("the plan handle is the test's own")
            .clear();
        let outcome = blocking_half(&watcher, &snapshot, &facts, None);
        let healed = outcome
            .unwatched
            .expect("a full pass composes the SYS-010 seam");
        assert!(healed.is_empty(), "the lifted refusal converges everything");
        assert_eq!(
            record_refusals(&state, healed),
            None,
            "the healed pass logs nothing"
        );
        assert!(
            state.unwatched_projects().is_empty(),
            "the notice reports no lost files after the heal"
        );
    }

    #[test]
    fn a_failed_emit_still_records_the_refused_watch_and_opens_the_episode() {
        // The round-2 F-1 pin, the reason `finish_pass` exists: the
        // round-1 P2-1 property — the refused-watch fact is recorded
        // and its episode opened even when the diff emit FAILS — was
        // correct in code but nothing drove its call site (the
        // round's isolated-copy mutations proved the whole suite
        // green with the recording moved after the emit branch, or
        // deleted). Driving the production bookkeeping with a failing
        // emitter makes both regressions red: the recording is the
        // watch's own fact, owed by the pass that observed it,
        // while the VERDICT cache deliberately stays stale
        // (at-least-once).
        let state = state_only();
        let refusal = BTreeMap::from([("p2".to_owned(), WatchRefusalKind::Limit)]);
        let outcome = PassOutcome {
            by_path: None,
            covered: vec!["p1".to_owned()],
            verdicts: vec![verdict("p1", modified())],
            unwatched: Some(refusal.clone()),
        };
        let changes = finish_pass(&state, outcome, true, |_event| {
            Err(tauri::Error::WindowLabelAlreadyExists("main".to_owned()))
        });
        assert_eq!(
            changes,
            vec![change("p1", modified())],
            "the undelivered diff is still the pass's answer"
        );
        assert_eq!(
            state.unwatched_projects(),
            refusal,
            "the refused-watch fact is recorded DESPITE the failed emit — \
             the notice is not left a pass stale"
        );
        assert_eq!(
            record_refusals(&state, refusal),
            None,
            "the failed-emit pass opened the episode: an identical repeated \
             recording would not re-warn"
        );
        assert!(
            state.snapshot().is_empty(),
            "the verdict cache stays stale on the failed emit (at-least-once)"
        );

        // The delivered control: the same world through a healthy
        // emitter records the verdicts and prunes to the covered set.
        let state = state_only();
        let outcome = PassOutcome {
            by_path: None,
            covered: vec!["p1".to_owned(), "p2".to_owned()],
            verdicts: vec![verdict("p1", modified())],
            unwatched: Some(BTreeMap::from([("p2".to_owned(), WatchRefusalKind::Limit)])),
        };
        let _ = finish_pass(&state, outcome, true, |_event| Ok(()));
        assert_eq!(
            state.snapshot(),
            vec![change("p1", modified())],
            "a delivered diff records its verdicts"
        );
    }

    // ---- the pump mapping (D-139's round-1 P2-2, the dispatch
    // precedent) ---------------------------------------------------

    /// Drives one [`WatchEvent`] through the extracted pump step and
    /// hands back the command it landed on, plus the drained bus: the
    /// same observable the service task consumes, read without
    /// notify's event thread — the reason the step is extracted (the
    /// watcher's `dispatch` shape, D-139).
    fn pumped(event: WatchEvent) -> (Command, tauri::async_runtime::Receiver<Command>) {
        let (trigger, mut commands) = tauri::async_runtime::channel(BUS_CAPACITY);
        pump_event(event, &trigger);
        let command = commands
            .try_recv()
            .expect("try_send lands the command on the bus before the step returns");
        (command, commands)
    }

    /// The FilesChanged shape asserted without `Command: Debug` (the
    /// enum deliberately carries no derives): exactly one path, the
    /// event's own.
    fn assert_files_changed(command: Command, path: &str) {
        match command {
            Command::FilesChanged(paths) => {
                assert_eq!(
                    paths,
                    vec![PathBuf::from(path)],
                    "the batch names its own file"
                );
            }
            Command::Refresh { .. } => {
                panic!("a file delivery must become a FilesChanged batch, not a Refresh");
            }
        }
    }

    #[test]
    fn changed_and_removed_events_become_files_changed_batches_of_their_own_path() {
        // SYS-008's noise filter, input side: one delivery names
        // EXACTLY its own file. A dropped path would re-check nothing
        // (the empty batch is noise to the consumer), a swapped or
        // widened batch would re-check the wrong projects — both
        // variants are pinned separately because the shared or-pattern
        // could hide a variant swap behind one combined assertion.
        let (command, mut commands) = pumped(WatchEvent::Changed(PathBuf::from("/repo/AGENTS.md")));
        assert_files_changed(command, "/repo/AGENTS.md");
        assert!(
            commands.try_recv().is_err(),
            "one event forwards exactly one command"
        );
        let (command, mut commands) = pumped(WatchEvent::Removed(PathBuf::from("/repo/CLAUDE.md")));
        assert_files_changed(command, "/repo/CLAUDE.md");
        assert!(
            commands.try_recv().is_err(),
            "one event forwards exactly one command"
        );
    }

    #[test]
    fn a_backend_event_becomes_a_fire_and_forget_full_refresh() {
        // SYS-010's lost-events signal: since D-139 the `Backend`
        // event carries BOTH of its shapes (the overflow rescan batch
        // and the stream-error batch), and the pump's whole answer is
        // the one full re-check the spec sanctions — a `Refresh` with
        // no ack. `done` must be `None`: no caller waits on this
        // trigger (both awaited seams build their own ack pairs), so a
        // sender here would answer a channel nobody receives.
        let (command, mut commands) = pumped(WatchEvent::Backend);
        match command {
            Command::Refresh { done } => assert!(
                done.is_none(),
                "the pump's Refresh is fire-and-forget: done must be None"
            ),
            Command::FilesChanged(_) => {
                panic!("the lost-events signal must become a full Refresh, not a batch");
            }
        }
        assert!(
            commands.try_recv().is_err(),
            "one event forwards exactly one command"
        );
    }

    #[test]
    fn a_full_bus_drops_the_delivery_without_panicking_or_displacing_queued_commands() {
        // The bounded-bus contract ([`BUS_CAPACITY`]'s docs): a full
        // bus is an intended drop — the pump must neither block behind
        // a slow pass nor displace the commands already queued. The
        // two fillers still lead the bus and nothing follows them; the
        // falsifiable form is an `expect` on the try_send, which
        // panics here.
        let (trigger, mut commands) = tauri::async_runtime::channel(2);
        let _ = trigger.try_send(Command::FilesChanged(vec![PathBuf::from("/f0")]));
        let _ = trigger.try_send(Command::FilesChanged(vec![PathBuf::from("/f1")]));
        pump_event(WatchEvent::Backend, &trigger);
        pump_event(
            WatchEvent::Changed(PathBuf::from("/repo/AGENTS.md")),
            &trigger,
        );
        let first = commands.try_recv().expect("filler 0 survived the overflow");
        assert_files_changed(first, "/f0");
        let second = commands.try_recv().expect("filler 1 survived the overflow");
        assert_files_changed(second, "/f1");
        assert!(
            commands.try_recv().is_err(),
            "the deliveries past the capacity were dropped, not queued"
        );
    }

    #[test]
    fn a_dead_bus_drops_every_variant_delivery_instead_of_panicking() {
        // The teardown shape: the service task is gone while the pump
        // thread still lives, so the bus's receiving half is dropped.
        // A panicking pump thread dies silently and every later watch
        // event is lost — exactly the crash SYS-010 forbids in
        // miniature. All three variants go through the one send site
        // (the watcher's own dead-consumer pin, D-139's round-2 P3-2);
        // reaching the end is the surviving behavior, falsifiable by
        // an `expect` on the try_send.
        let (trigger, commands) = tauri::async_runtime::channel(BUS_CAPACITY);
        drop(commands);
        pump_event(
            WatchEvent::Changed(PathBuf::from("/repo/AGENTS.md")),
            &trigger,
        );
        pump_event(
            WatchEvent::Removed(PathBuf::from("/repo/AGENTS.md")),
            &trigger,
        );
        pump_event(WatchEvent::Backend, &trigger);
    }

    // ---- the verdict (SYS-008/SYS-009 through the real builders) ------

    /// The extras-off option both extra fields default to.
    fn off() -> ExtraFile {
        ExtraFile {
            enabled: false,
            position: ExtraFilePosition::End,
        }
    }

    /// The empty reading-lines pair of the off() fixtures.
    fn no_lines() -> ExtraFileLines {
        ExtraFileLines {
            start: Vec::new(),
            end: Vec::new(),
        }
    }

    /// A real marker line from the real formatter — the file fixtures
    /// are what a sync would actually leave.
    fn real_marker() -> String {
        creation_marker_line(
            MarkerTime {
                year: 2026,
                month: 11,
                day: 21,
                hour: 9,
                minute: 30,
                second: 5,
            },
            Some("Europe/Prague"),
            60,
        )
    }

    #[test]
    fn a_freshly_written_scheme_a_file_matches_its_build() {
        // The anchor: the file the sync's own builder writes (any
        // marker time — ALG-033 drops it) is NOT externally modified.
        let file = build_content(Scheme::A, "Be brief.", &no_lines(), &real_marker());
        assert_eq!(
            classify_target_file(
                Scheme::A,
                "Be brief.",
                &no_lines(),
                Some(file.as_bytes()),
                true
            ),
            TargetFileVerdict::Matches
        );
    }

    #[test]
    fn a_body_edit_of_a_scheme_a_file_is_externally_modified() {
        let file = build_content(Scheme::A, "Be brief.", &no_lines(), &real_marker());
        let edited = file.replace("Be brief.", "Be very brief.");
        assert_eq!(
            classify_target_file(
                Scheme::A,
                "Be brief.",
                &no_lines(),
                Some(edited.as_bytes()),
                true
            ),
            TargetFileVerdict::ExternallyModified
        );
    }

    #[test]
    fn a_missing_file_is_deleted_only_after_a_recorded_sync() {
        // SYS-009 vs OUT-005: the deletion flag needs a past sync; a
        // never-synced project without a file is simply not written
        // yet — no flag, whatever the scheme.
        for scheme in [Scheme::A, Scheme::B, Scheme::C] {
            assert_eq!(
                classify_target_file(scheme, "Be brief.", &no_lines(), None, true),
                TargetFileVerdict::Deleted,
                "scheme {scheme:?}: a synced project's vanished file is deleted"
            );
            assert_eq!(
                classify_target_file(scheme, "Be brief.", &no_lines(), None, false),
                TargetFileVerdict::Matches,
                "scheme {scheme:?}: a never-synced project's absent file is clean"
            );
        }
    }

    #[test]
    fn a_pre_existing_file_a_sync_would_overwrite_is_externally_modified() {
        // SYS-008's literal reading is not conditioned on a sync
        // history: a file that exists and differs from the build is
        // externally modified even before the first sync (the typical
        // case: the user's own AGENTS.md, present before the project
        // was added).
        let foreign = b"# My own house rules\n\nBe verbose.\n".to_vec();
        assert_eq!(
            classify_target_file(Scheme::A, "Be brief.", &no_lines(), Some(&foreign), false),
            TargetFileVerdict::ExternallyModified
        );
    }

    #[test]
    fn a_scheme_a_file_with_reading_lines_matches_its_real_build() {
        // The A side goes through the production builder with the
        // reading lines spliced in — no hand-rolled join is involved.
        let lines = eunomia_core::extra_files::extra_file_lines(
            BaseName::AgentsMd,
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start,
            },
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::End,
            },
        );
        let file = build_content(Scheme::A, "Be brief.", &lines, &real_marker());
        assert_eq!(
            classify_target_file(Scheme::A, "Be brief.", &lines, Some(file.as_bytes()), true),
            TargetFileVerdict::Matches
        );
        // One dropped reading line differs:
        let edited = file.replacen(
            "If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.\n",
            "",
            1,
        );
        assert_eq!(
            classify_target_file(
                Scheme::A,
                "Be brief.",
                &lines,
                Some(edited.as_bytes()),
                true
            ),
            TargetFileVerdict::ExternallyModified
        );
    }

    #[test]
    fn a_scheme_c_project_compares_its_stem_file() {
        // D-080's closed list: C's target is <STEM>.project.md; the
        // CONTENT is the body without reading lines (ALG-030's "A
        // only"), which is exactly what the C call through
        // build_content produces.
        let file = build_content(Scheme::C, "Be brief.", &no_lines(), &real_marker());
        assert_eq!(
            classify_target_file(
                Scheme::C,
                "Be brief.",
                &no_lines(),
                Some(file.as_bytes()),
                true
            ),
            TargetFileVerdict::Matches
        );
    }

    #[test]
    fn a_markerless_but_exact_scheme_a_file_still_matches() {
        // ALG-033 with no marker line compares the file whole; a file
        // that IS the markerless build matches (compare.rs pins the
        // same rule) — the user deleting only the marker line must
        // not light the column. The fixture is the LITERAL ALG-030
        // form (the managed block plus one empty line), not the
        // helper under test, so the oracle stands on its own.
        let side = "Be brief.\n\n";
        assert_eq!(
            classify_target_file(
                Scheme::A,
                "Be brief.",
                &no_lines(),
                Some(side.as_bytes()),
                true
            ),
            TargetFileVerdict::Matches
        );
    }

    #[test]
    fn an_empty_build_matches_the_marker_only_file() {
        // ALG-030's empty-managed shape: the file is the marker line
        // alone, and it matches the empty build.
        let file = build_content(Scheme::C, "", &no_lines(), &real_marker());
        assert_eq!(
            classify_target_file(Scheme::C, "", &no_lines(), Some(file.as_bytes()), true),
            TargetFileVerdict::Matches
        );
    }

    // ---- scheme B: the assembled side -----------------------------------

    /// A scheme-B file exactly as a sync leaves it: outside text,
    /// markers, the managed block with the marker at its end.
    fn b_file(outside: &str, body: &str, marker: &str) -> String {
        use eunomia_core::markers::{BEGIN_MARKER, END_MARKER};
        let inside = if body.is_empty() {
            format!("{marker}\n")
        } else {
            format!("{body}\n\n{marker}\n")
        };
        format!("{outside}{BEGIN_MARKER}\n{inside}{END_MARKER}\n")
    }

    #[test]
    fn a_freshly_written_scheme_b_file_matches_its_build() {
        // The B anchor, through the real splice: what a sync leaves
        // between the markers matches the build, marker aside.
        let file = b_file("My notes.\n", "Be brief.", &real_marker());
        assert_eq!(
            classify_target_file(
                Scheme::B,
                "Be brief.",
                &no_lines(),
                Some(file.as_bytes()),
                true
            ),
            TargetFileVerdict::Matches
        );
    }

    #[test]
    fn an_edit_inside_the_managed_block_is_externally_modified() {
        let file = b_file("My notes.\n", "Be brief.", &real_marker());
        let edited = file.replace("Be brief.", "Be very brief.");
        assert_eq!(
            classify_target_file(
                Scheme::B,
                "Be brief.",
                &no_lines(),
                Some(edited.as_bytes()),
                true
            ),
            TargetFileVerdict::ExternallyModified
        );
    }

    #[test]
    fn an_edit_outside_the_markers_is_not_externally_modified() {
        // The B-specific honesty: the build preserves everything
        // outside the markers byte for byte, so the "current build"
        // follows the file — an outside edit is not a difference from
        // the build (the sync will not even touch it).
        let file = b_file("My notes.\n", "Be brief.", &real_marker());
        let edited = file.replace("My notes.", "My edited notes.");
        assert_eq!(
            classify_target_file(
                Scheme::B,
                "Be brief.",
                &no_lines(),
                Some(edited.as_bytes()),
                true
            ),
            TargetFileVerdict::Matches
        );
    }

    #[test]
    fn a_scheme_b_file_with_broken_markers_is_externally_modified() {
        // SYS-009's B sentence and §7.5: no valid marker pair, no
        // possible match — the sync will skip with
        // E_MARKERS_INVALID, and the column must show the state.
        let markerless = "My notes.\nBe brief.\n".to_owned();
        assert_eq!(
            classify_target_file(
                Scheme::B,
                "Be brief.",
                &no_lines(),
                Some(markerless.as_bytes()),
                true
            ),
            TargetFileVerdict::ExternallyModified
        );
    }

    #[test]
    fn a_non_utf8_scheme_b_file_is_externally_modified() {
        // §9.4 phase 1: only scheme B promises UTF-8 upstream of the
        // assembly; bytes that are not text can never equal the
        // build.
        let mut raw = b"\xFF\xFE".to_vec();
        raw.extend_from_slice(b_file("x\n", "Be brief.", &real_marker()).as_bytes());
        assert_eq!(
            classify_target_file(Scheme::B, "Be brief.", &no_lines(), Some(&raw), true),
            TargetFileVerdict::ExternallyModified
        );
    }

    #[test]
    fn a_crlf_scheme_b_file_matches_its_crlf_build() {
        // T-F12's shape: the outside is CRLF, so the splice's block
        // is CRLF too (the dominant-style rule), and the file the
        // sync leaves matches the next build of the same body.
        let crlf_outside = "Notes.\r\n";
        let file = b_file(crlf_outside, "Be brief.", &real_marker())
            .replace('\n', "\r\n")
            .replace("\r\r\n", "\r\n");
        // ^ every "\n" becomes "\r\n" (the ones already after "\r"
        // collapse back) — the honest way to spell the fixture.
        assert_eq!(
            classify_target_file(
                Scheme::B,
                "Be brief.",
                &no_lines(),
                Some(file.as_bytes()),
                true
            ),
            TargetFileVerdict::Matches,
            "the dominant-CRLF file the sync writes must match its own build"
        );
    }

    // ---- facts_of: the ever_synced join ---------------------------------

    /// The store row's minimal shape for the join test — spelled as
    /// the repository returns it, only the fields `facts_of` reads.
    fn store_project(id: &str, hash: Option<&str>) -> Project {
        Project {
            id: id.to_owned(),
            path: format!("/repos/{id}"),
            path_key: format!("/{id}"),
            name: format!("project {id}"),
            label: "PRJ".to_owned(),
            color: "red".to_owned(),
            description: String::new(),
            state: eunomia_store::projects::ProjectState::Active,
            added_via: eunomia_store::projects::AddedVia::Manual,
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
            last_synced_hash: hash.map(str::to_owned),
            created_at: String::new(),
            updated_at: String::new(),
            archived_at: None,
        }
    }

    /// A one-project snapshot fixture (the dirty suite's shape).
    fn one_project_snapshot() -> BuildSnapshot {
        let project = SnapshotProject {
            id: ProjectId::new("p1".to_owned()),
            name: "project p1".to_owned(),
            root: Path::new("/repos").join("p1"),
            settings: ProjectWriteSettings {
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: off(),
                extra_project: off(),
            },
        };
        BuildSnapshot::new(vec![project], Vec::new(), Vec::new())
            .expect("the test fixture is a valid snapshot")
    }

    #[test]
    fn ever_synced_reads_null_and_empty_as_never() {
        // D-068: the empty hash cell means the same as NULL — no
        // recorded sync, so no deletion flag for a missing file.
        let snapshot = one_project_snapshot();
        let facts = facts_of(&snapshot, &[store_project("p1", None)]);
        assert!(!facts[0].ever_synced, "NULL is never-synced");
        let facts = facts_of(&snapshot, &[store_project("p1", Some(""))]);
        assert!(!facts[0].ever_synced, "the empty cell is never-synced");
        let facts = facts_of(
            &snapshot,
            &[store_project(
                "p1",
                Some("d2a2446b2437df4c64a4db2b3a49a45e519fddf98e8718ac0fb9f682f9b83d27"),
            )],
        );
        assert!(facts[0].ever_synced, "a recorded hash is a past sync");
    }

    // ---- bodies_of: the chunked build over a real snapshot --------------

    #[test]
    fn bodies_of_builds_each_wanted_project_once() {
        // Two projects, one assigned instruction each: the bodies come
        // out in snapshot order, only for the wanted ids, with the
        // assignment's variant text (§9.2 through the real builder).
        let instruction = LibraryItem::Instruction(Instruction {
            id: ItemId::new("i1".to_owned()),
            title: "Style".to_owned(),
            state: InstructionState::Active,
            variants: vec![Variant {
                id: VariantId::new("v1".to_owned()),
                name: "Main".to_owned(),
                content: "Be brief.".to_owned(),
            }],
        });
        let mut p1 = one_project_snapshot().projects()[0].clone();
        p1.id = ProjectId::new("p1".to_owned());
        let mut p2 = p1.clone();
        p2.id = ProjectId::new("p2".to_owned());
        let assigned = |project: &str| Assignment {
            project_id: ProjectId::new(project.to_owned()),
            instruction_id: ItemId::new("i1".to_owned()),
            variant_id: VariantId::new("v1".to_owned()),
        };
        let snapshot = BuildSnapshot::new(
            vec![p1, p2],
            vec![instruction],
            vec![assigned("p1"), assigned("p2")],
        )
        .expect("the fixture is a valid snapshot");
        let wanted: HashSet<&str> = ["p2"].into_iter().collect();
        let bodies = bodies_of(&snapshot, &wanted);
        assert_eq!(
            bodies,
            vec![("p2".to_owned(), "Be brief.".to_owned())],
            "only the wanted project is built, in snapshot order — the \
             bare instruction's body is its variant text (§9.2: the \
             heading line belongs to assigned headings, and this \
             fixture has none)"
        );
        let empty: HashSet<&str> = HashSet::new();
        assert!(bodies_of(&snapshot, &empty).is_empty());
    }
}
