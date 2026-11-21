//! Project-availability tracking (D-09, DB-032, §7.10's
//! `state://project-availability` event; T-079): the service that
//! re-probes every active project's folder and tells the UI what
//! changed since the last probe.
//!
//! D-09/DB-032 fix three check points — startup, window open/focus,
//! and before a sync — and §9.4 phase 1 step 1 is the sync pipeline's
//! own per-project gate. This module owns the first three through one
//! recheck, [`recheck_and_emit`]:
//!
//! - **Startup** — the `setup` hook in `lib.rs` spawns it right after
//!   the database is managed (DB-032's "at startup"; a background
//!   task, because four metadata probes per project over possibly
//!   dead drives must not hold the launch — §5.4 SYS-001, NFR-002's
//!   startup budget).
//! - **Window open/focus** — `lib.rs` routes every window event
//!   through [`on_window_event`]; `Focused(true)` is the recheck
//!   trigger, `Focused(false)` and every other event are ignored.
//!   The real "window opened" moment is platform-delivered as a
//!   focus gain (the window cannot be visible without focus), so one
//!   arm covers both DB-032 wordings.
//! - **Before a sync** — the reusable entry is the function itself:
//!   the future sync pipeline (§9.4, T-118) awaits
//!   [`recheck_and_emit`] as its DB-032 checkpoint and then runs its
//!   own phase-1 probes (the pipeline must never trust this cache —
//!   a verdict can go stale between the checkpoint and the write).
//!
//! The probe is `eunomia_fs::paths::check_project_available` (D-084,
//! read-only reuse): four metadata probes — exists, is a directory,
//! listable, has a `.git` entry — with no drive heuristics, so
//! network and removable drives pass exactly like local ones
//! (DB-032's allowance). The probes are blocking file I/O and run on
//! the blocking pool **after** the `Db::run` read (SYS-001, the
//! D-096 ordering: a dead drive delays only this pass, never the one
//! database lock).
//!
//! **The diff contract (D-09's "event with the list of changes").**
//! The last known verdicts live in the [`AvailabilityState`] managed
//! cache — session memory, deliberately not the database: availability
//! is a fact about the *machine's current filesystem*, meaningless
//! across restarts, so there is nothing honest to persist. A project
//! that has not been probed yet carries the implicit baseline
//! "available": the first check therefore reports only projects found
//! unavailable (T-072's `projects_list` already probes fresh on every
//! read, so an all-healthy startup has nothing to tell the UI), while
//! a later probe that *contradicts* a known verdict — in either
//! direction, unavailable → available included — is a change. The
//! event carries only the changed projects, in `sort_order` (the
//! probe order), and is not emitted at all when the diff is empty:
//! repeated checks with unchanged folders are silent, which is what
//! makes rechecking on every focus cheap and safe
//! (principle: make operations idempotent).
//!
//! **Delivery is at-least-once, not at-most-once.** The new verdicts
//! are recorded only *after* the event was emitted successfully: a
//! failed emit leaves the cache stale on purpose, so the next check
//! point re-detects the same change and re-emits it (a lost UI update
//! is permanent until the folder changes again; a duplicate is a
//! benign no-op for the listener). The price — two overlapping checks
//! can emit the same change twice, since the guarded spawn of the
//! startup/focus path can overlap the sync pipeline's direct await —
//! is accepted for exactly that reason.
//!
//! **The in-flight slot.** [`spawn_recheck`] coalesces focus storms:
//! while one recheck runs, a spawned one returns immediately instead
//! of queueing more probes behind it. The slot is an atomic flag
//! taken by `compare_exchange` and released by the RAII guard's
//! `Drop` — a guard that is `Send` (it owns an `Arc`), so it is held
//! across the task's `.await`s, which is the point: the whole pass,
//! DB read to emit, counts as "in flight" (round-1 review P1-1: the
//! first cut dropped a `std::sync::MutexGuard` at the end of the
//! `if` condition that took it — a `!Send` temporary that cannot
//! cross an `.await` at all, so the slot guarded nothing; the atomic
//! guard makes the lifetime compile-time visible). Nothing ever
//! waits on the slot, and a panic frees it with the unwind (the
//! guard drops) — a crashed recheck cannot silence every later one.
//!
//! **Recovery mode** (no database, the T-024 state) is honest the
//! house way: the recheck takes the [`crate::commands::managed_db`]
//! error — the same `NoDatabase` diagnostic every database command
//! answers with — logs it through the facade, and emits nothing.
//! There are no project columns to mark on the recovery screen, so
//! there is nothing to pretend about.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime, WindowEvent};
use tauri_specta::Event;

use eunomia_fs::paths::{ProjectUnavailableError, check_project_available};
use eunomia_store::projects::ListScope;

use crate::commands::{managed_db, projects_store_to_app_error};
use crate::db::join_failure_to_app_error;
use crate::errors::AppError;
use crate::logging;

/// One project's availability as this service speaks about it: the
/// D-084 verdict pair behind the `available`/`unavailable_reason`
/// fields `projects_list` already serves (D-096), single-sourced here
/// so the command layer and this event can never drift apart. The
/// reason spellings are the wire's own snake_case names of the
/// D-084 verdict classes, not stored database cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verdict {
    /// Whether all four D-084 probes passed.
    pub(crate) available: bool,
    /// Why [`Verdict::available`] is `false` (`missing`,
    /// `not_a_directory`, `not_readable`, `no_git_entry`);
    /// `None` when the folder is available.
    pub(crate) unavailable_reason: Option<String>,
}

impl Verdict {
    /// The implicit baseline for a project this session has not probed
    /// yet: available. See the module docs for why the first check
    /// reports only unavailable projects.
    pub(crate) const AVAILABLE: Verdict = Verdict {
        available: true,
        unavailable_reason: None,
    };

    /// Runs the D-084 probe and shapes its verdict. Read-only; the
    /// blocking-pool placement is the caller's (this service probes on
    /// the pool after its transaction, the D-096 ordering).
    pub(crate) fn probe(root: &Path) -> Verdict {
        match check_project_available(root) {
            Ok(()) => Verdict::AVAILABLE,
            Err(error) => Verdict {
                available: false,
                unavailable_reason: Some(
                    match error {
                        ProjectUnavailableError::Missing { .. } => "missing",
                        ProjectUnavailableError::NotADirectory => "not_a_directory",
                        ProjectUnavailableError::NotReadable { .. } => "not_readable",
                        ProjectUnavailableError::NoGitEntry => "no_git_entry",
                    }
                    .to_owned(),
                ),
            },
        }
    }
}

/// One entry of the `state://project-availability` diff (§7.10): the
/// project a change is about, addressed the way every project action
/// addresses it (its row id), plus the verdict that now holds. The
/// display name is deliberately absent — the UI already holds it
/// keyed by id from `projects_list`, and a copy here could only
/// drift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ProjectAvailabilityChange {
    /// The project row id (UUIDv7 TEXT, DB-002) whose availability
    /// changed since the previous check.
    pub(crate) project_id: String,
    /// The verdict that now holds for the folder.
    pub(crate) available: bool,
    /// Why [`ProjectAvailabilityChange::available`] is `false`, in the
    /// D-084 reason spellings; `None` when the folder is (again)
    /// available.
    pub(crate) unavailable_reason: Option<String>,
}

/// The `state://project-availability` event payload (§7.10): only the
/// projects whose verdict changed since the previous check, in probe
/// order (`sort_order`). Never emitted with an empty `changes`.
///
/// The event name is the spec's literal wire name (it is what the
/// frontend listens for), pinned by the manual
/// [`tauri_specta::Event`] implementation below — the crate's derive
/// macro would build the name from the struct's identifier and cannot
/// produce the `state://` prefix, and the `derive` cargo feature that
/// ships it is not enabled (the workspace pins
/// `tauri-specta = "=2.0.0-rc.29"` with `typescript` only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct ProjectAvailabilityEvent {
    /// The changed projects; empty diffs are not emitted (see the
    /// module docs' diff contract).
    pub(crate) changes: Vec<ProjectAvailabilityChange>,
}

impl Event for ProjectAvailabilityEvent {
    /// The §7.10 wire name, verbatim. Tauri permits exactly the
    /// characters it contains (alphanumeric, `-`, `/`, `:`, `_`).
    const NAME: &'static str = "state://project-availability";
}

/// The in-flight coalescing flag behind [`spawn_recheck`]: `false` —
/// no spawned recheck is running, `true` — one is (from its slot-take
/// to its finish, DB read to emit). Taken by
/// [`RecheckSlot::try_take`]'s `compare_exchange`, released by the
/// returned guard's `Drop`.
///
/// `pub(crate)` in the `AvailabilityState` (the `running` field): the
/// IPC tests hold a pass "in flight" from outside through it (the
/// `projects_store_to_app_error` seam pattern — two real spawns
/// racing for the slot cannot be made deterministic).
pub(crate) struct RecheckSlot {
    running: AtomicBool,
}

impl RecheckSlot {
    /// The free slot a fresh launch starts from.
    pub(crate) fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
        }
    }

    /// Takes the slot if no spawned recheck is running, returning the
    /// guard that holds it until dropped; `None` — a recheck is in
    /// flight, and the caller coalesces into it. The atomicity of the
    /// `compare_exchange` IS the mutual exclusion (the orderings are
    /// the conservative Acquire/Release pair around the flag; the
    /// pass's own data — the cache, the wire — is guarded by its own
    /// synchronization).
    pub(crate) fn try_take(self: &Arc<Self>) -> Option<RecheckSlotGuard> {
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RecheckSlotGuard {
                slot: Arc::clone(self),
            })
    }

    /// Whether a spawned recheck currently holds the slot — the
    /// observable the tests poll for a pass that has started but not
    /// finished (a pass mid-`Db::run` cannot be awaited without also
    /// driving it to completion). Test-only by design: production
    /// never asks, it only takes.
    #[cfg(test)]
    pub(crate) fn is_taken(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
}

/// The in-flight guard: holds [`RecheckSlot`] for its owner's
/// lifetime — across `.await`s included (`Send` by construction: it
/// owns an `Arc`), and freed by `Drop` even on a panic (the unwind
/// drops it), so a crashed recheck cannot leave the slot stuck taken.
pub(crate) struct RecheckSlotGuard {
    slot: Arc<RecheckSlot>,
}

impl Drop for RecheckSlotGuard {
    fn drop(&mut self) {
        self.slot.running.store(false, Ordering::Release);
    }
}

/// The session cache behind the diff (module docs): the last verdict
/// this session recorded per active project id, plus the in-flight
/// slot that coalesces overlapping spawned rechecks.
///
/// Managed once in the `setup` hook before anything that could fail
/// (the [`crate::session_paths::SessionPaths`] shape): window events
/// cannot arrive before the event loop starts, and the startup
/// recheck is spawned after the database, so the state exists in
/// every mode — recovery included, where the recheck answers the
/// honest `NoDatabase` error instead of panicking on a lookup.
pub(crate) struct AvailabilityState {
    /// Last known verdict per active project id. Absent ≡ the
    /// [`Verdict::AVAILABLE`] baseline; entries for ids that left the
    /// active list (archived, removed) are pruned on every check.
    last: Mutex<HashMap<String, Verdict>>,
    /// The recheck coalescing slot ([`spawn_recheck`]); `Arc` so the
    /// spawned task's guard owns a share for the task's lifetime.
    pub(crate) running: Arc<RecheckSlot>,
}

impl AvailabilityState {
    /// The empty session state a fresh launch starts from.
    pub(crate) fn new() -> Self {
        Self {
            last: Mutex::new(HashMap::new()),
            running: Arc::new(RecheckSlot::new()),
        }
    }

    /// Computes the changes `probes` carry against the cache, without
    /// recording them: a project is changed when its new verdict
    /// differs from the recorded one — or from the implicit
    /// [`Verdict::AVAILABLE`] baseline when it was never probed this
    /// session. Read-only against the cache (the write half is
    /// [`Self::record`], called after a successful emit; the module
    /// docs explain the at-least-once ordering).
    fn diff_against_last(&self, probes: &[(String, Verdict)]) -> Vec<ProjectAvailabilityChange> {
        let last = self
            .last
            .lock()
            .expect("the availability cache is locked only inside this short synchronous section");
        probes
            .iter()
            .filter_map(|(id, verdict)| {
                let prior = last.get(id).unwrap_or(&Verdict::AVAILABLE);
                if prior == verdict {
                    None
                } else {
                    Some(ProjectAvailabilityChange {
                        project_id: id.clone(),
                        available: verdict.available,
                        unavailable_reason: verdict.unavailable_reason.clone(),
                    })
                }
            })
            .collect()
    }

    /// Records the probe verdicts and prunes the ids that are no
    /// longer active — the cache ends up exactly the probed world,
    /// whatever it held before. Pruning emits nothing: a project that
    /// left the active list has no column to mark anymore (archiving
    /// already re-renders the matrix), and a stale cache entry for it
    /// would only resurface as a phantom change if the project is
    /// restored later.
    fn record(&self, probes: &[(String, Verdict)]) {
        let mut last = self
            .last
            .lock()
            .expect("the availability cache is locked only inside this short synchronous section");
        let live: HashSet<&str> = probes.iter().map(|(id, _)| id.as_str()).collect();
        last.retain(|id, _| live.contains(id.as_str()));
        for (id, verdict) in probes {
            last.insert(id.clone(), verdict.clone());
        }
    }
}

/// The DB-032 recheck every check point shares: read the active
/// projects (ALG-001's order — the probe order the event's diff
/// keeps), probe each folder on the blocking pool (D-084, SYS-001,
/// after the transaction — the D-096 ordering), diff against the
/// session cache, and emit `state://project-availability` when the
/// diff is non-empty.
///
/// The returned changes are exactly what the event carried (empty —
/// nothing was emitted): the future sync pipeline (§9.4, T-118)
/// awaits this as its DB-032 "before a sync" checkpoint and may log
/// or branch on the diff; its phase-1 per-project probes stay its own
/// gate. Failures propagate in the §7.10 `AppError` shape — the
/// honest `NoDatabase` answer in recovery mode, the classified store
/// errors otherwise — and emit nothing: a read that could not see the
/// project list has no diff worth claiming.
pub(crate) async fn recheck_and_emit<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ProjectAvailabilityChange>, AppError> {
    let database = managed_db(app)?;
    let rows = database
        .run(move |transaction| {
            eunomia_store::projects::list(transaction, ListScope::Active)
                .map_err(projects_store_to_app_error)
        })
        .await?;
    // The probes (D-084) are blocking file I/O over live folders: they
    // run on the blocking pool, after the transaction — a dead drive
    // delays only this pass, never the one database lock.
    let probes = tauri::async_runtime::spawn_blocking(move || {
        rows.into_iter()
            .map(|project| {
                let verdict = Verdict::probe(Path::new(&project.path));
                (project.id, verdict)
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(join_failure_to_app_error)?;

    let state = app.state::<AvailabilityState>();
    let changes = state.diff_against_last(&probes);
    if !changes.is_empty() {
        let event = ProjectAvailabilityEvent {
            changes: changes.clone(),
        };
        if let Err(error) = event.emit(app) {
            // Deliberately NOT recording the probes: the cache stays
            // stale so the next check point re-detects and re-emits
            // this diff (the module docs' at-least-once contract). The
            // recheck itself succeeded — the failure is the UI
            // plumbing's, so the changes still go back to the caller.
            logging::availability_emit_failed(&error);
            return Ok(changes);
        }
    }
    state.record(&probes);
    Ok(changes)
}

/// Spawns [`recheck_and_emit`] as a fire-and-forget task behind the
/// in-flight slot — the startup and window-focus shape. The slot is
/// taken inside the task and held by its RAII guard for the whole
/// pass, DB read to emit: a recheck already running makes the new
/// task return immediately (focus storms coalesce into the running
/// probe pass; the next focus rechecks again — a skipped run never
/// loses a verdict change for longer than one check point). The
/// guard frees the slot when the task ends or panics.
///
/// Failures are logged through the facade, never propagated: the
/// callers (the `setup` hook, the window-event route) have no one to
/// return them to, and a failed availability pass must not disturb
/// the launch or the window it was triggered by.
///
/// The returned join handle lets the IPC tests await the pass
/// deterministically; production callers drop it (the task runs on —
/// tauri's handle detaches on drop, like tokio's).
pub(crate) fn spawn_recheck<R: Runtime>(app: AppHandle<R>) -> tauri::async_runtime::JoinHandle<()> {
    let slot = app.state::<AvailabilityState>().running.clone();
    tauri::async_runtime::spawn(async move {
        // The guard is held for the task's whole lifetime — across
        // every `.await` below (a `Send` RAII guard over the atomic,
        // NOT a `std::sync::MutexGuard`: that is `!Send`, cannot cross
        // an `.await`, and one taken in an `if` condition is dropped
        // at the condition's end — the round-1 review P1-1 regression
        // this shape exists to make impossible). `None` IS the
        // coalescing: a recheck is in flight, and this task returns.
        let Some(_recheck_slot) = slot.try_take() else {
            return;
        };
        if let Err(error) = recheck_and_emit(&app).await {
            logging::availability_check_failed(&error);
        }
    })
}

/// What [`on_window_event`] did with a window event — the observable
/// seam for the trigger rule (the mock runtime delivers no real focus
/// events, exactly like the single-instance handler's outcome: the
/// reaction value plus the emitted event are what the tests pin).
#[derive(Debug)]
pub(crate) enum WindowEventReaction {
    /// The event was the recheck trigger (`Focused(true)`); carries
    /// the spawned pass's join handle (see [`spawn_recheck`]).
    Recheck(tauri::async_runtime::JoinHandle<()>),
    /// The event does not recheck (anything but a focus gain — losing
    /// focus, resizes, theme changes, close requests, …).
    Ignored,
}

/// The DB-032 window trigger: route one window event to the recheck
/// decision. Only `Focused(true)` rechecks — the window being visible
/// to the user is the moment a stale marking misleads, and "the
/// window opens" arrives as its first focus gain. Any window of this
/// app qualifies, not just `main`: every window renders project
/// state (future UI-002 windows included), and the in-flight slot
/// already makes the extra passes harmless. The spawning is
/// fire-and-forget — the event loop's thread must not wait on probes
/// (SYS-001).
pub(crate) fn on_window_event<R: Runtime>(
    app: &AppHandle<R>,
    event: &WindowEvent,
) -> WindowEventReaction {
    match event {
        WindowEvent::Focused(true) => WindowEventReaction::Recheck(spawn_recheck(app.clone())),
        _ => WindowEventReaction::Ignored,
    }
}

#[cfg(test)]
mod tests {
    //! The pure halves of the service (the diff semantics and the
    //! cache's lifecycle rules) — behavior tests over the state's own
    //! interface. The mock-runtime halves (the event round trip, the
    //! triggers, the criterion move-folder scenario) live in
    //! `ipc_tests.rs`, which already owns the mock app harness.

    use super::*;

    /// A probe pair with an owned id, the shape `recheck_and_emit`
    /// hands the cache.
    fn probe(id: &str, verdict: Verdict) -> (String, Verdict) {
        (id.to_owned(), verdict)
    }

    #[test]
    fn the_event_name_is_the_spec_literal() {
        // The wire name is the frontend's whole contract: every
        // round-trip test reads the same constant on both sides, so a
        // typo in it would stay green — pinned here against the §7.10
        // literal (tauri permits exactly its characters).
        assert_eq!(
            ProjectAvailabilityEvent::NAME,
            "state://project-availability"
        );
    }

    #[test]
    fn the_in_flight_slot_is_taken_once_and_freed_by_the_guard() {
        // The slot mechanism the round-1 review P1-1 made load-bearing:
        // exactly one taker while the guard lives, and the flag back to
        // free the moment it drops — including the panic path (the
        // unwind runs `Drop`, asserted here through a scoped drop
        // rather than an actual panic).
        let slot = Arc::new(RecheckSlot::new());
        assert!(!slot.is_taken(), "a fresh slot is free");

        let guard = slot.try_take().expect("a free slot must be takeable");
        assert!(slot.is_taken(), "the slot is taken while the guard lives");
        assert!(
            slot.try_take().is_none(),
            "a second taker is coalesced while the guard lives"
        );

        drop(guard);
        assert!(
            !slot.is_taken(),
            "dropping the guard frees the slot (the panic path drops it the same way)"
        );
        assert!(
            slot.try_take().is_some(),
            "the freed slot is takeable again"
        );
    }

    /// The verdict of a folder that failed the D-084 probe, in one of
    /// the wire spellings.
    fn unavailable(reason: &str) -> Verdict {
        Verdict {
            available: false,
            unavailable_reason: Some(reason.to_owned()),
        }
    }

    #[test]
    fn a_first_probe_of_an_available_project_is_no_change() {
        // The baseline rule: an unprobed project counts as available,
        // so a healthy first check has nothing to report — an
        // all-healthy startup emits nothing.
        let state = AvailabilityState::new();
        let changes = state.diff_against_last(&[probe("p1", Verdict::AVAILABLE)]);
        assert_eq!(changes, Vec::new());
    }

    #[test]
    fn a_first_probe_of_an_unavailable_project_is_a_change() {
        // The other half of the baseline: D-09's marking exists for
        // unavailable projects, so the very first check must report
        // one found missing (id + the verdict, nothing else).
        let state = AvailabilityState::new();
        let changes = state.diff_against_last(&[probe("p1", unavailable("missing"))]);
        assert_eq!(
            changes,
            vec![ProjectAvailabilityChange {
                project_id: "p1".to_owned(),
                available: false,
                unavailable_reason: Some("missing".to_owned()),
            }]
        );
    }

    #[test]
    fn a_changed_verdict_is_reported_in_both_directions() {
        // available → unavailable (the moved-folder case) and back
        // (the folder returned) are both changes: the marking has to
        // be set AND cleared.
        let state = AvailabilityState::new();
        state.record(&[probe("p1", Verdict::AVAILABLE)]);
        assert_eq!(
            state.diff_against_last(&[probe("p1", unavailable("missing"))]),
            vec![ProjectAvailabilityChange {
                project_id: "p1".to_owned(),
                available: false,
                unavailable_reason: Some("missing".to_owned()),
            }]
        );
        // The recovery direction, against a RECORDED unavailable
        // verdict: available again, with the reason gone.
        state.record(&[probe("p1", unavailable("missing"))]);
        assert_eq!(
            state.diff_against_last(&[probe("p1", Verdict::AVAILABLE)]),
            vec![ProjectAvailabilityChange {
                project_id: "p1".to_owned(),
                available: true,
                unavailable_reason: None,
            }]
        );
    }

    #[test]
    fn an_unchanged_verdict_is_not_a_change_even_unavailable() {
        // Idempotence, both halves: a still-available project is no
        // news, and a project that STAYS unavailable is no news
        // either — repeated focus checks emit nothing.
        let state = AvailabilityState::new();
        state.record(&[probe("p1", unavailable("missing"))]);
        assert_eq!(
            state.diff_against_last(&[probe("p1", unavailable("missing"))]),
            Vec::new()
        );
        assert_eq!(
            state.diff_against_last(&[probe("p2", Verdict::AVAILABLE)]),
            Vec::new()
        );
    }

    #[test]
    fn a_reason_flip_between_two_unavailable_verdicts_is_a_change() {
        // `missing` → `not_readable` is a different verdict pair: the
        // UI's text half (D-09's reason) must follow the new class.
        let state = AvailabilityState::new();
        state.record(&[probe("p1", unavailable("missing"))]);
        assert_eq!(
            state.diff_against_last(&[probe("p1", unavailable("not_readable"))]),
            vec![ProjectAvailabilityChange {
                project_id: "p1".to_owned(),
                available: false,
                unavailable_reason: Some("not_readable".to_owned()),
            }]
        );
    }

    #[test]
    fn recording_prunes_ids_that_left_the_active_list() {
        // The prune rule: an archived/removed project's cache entry
        // disappears with the next check. Observable through the
        // diff: restoring the id later must look like a FIRST probe
        // (baseline available), not like a recovery from a verdict
        // the active list no longer holds.
        let state = AvailabilityState::new();
        state.record(&[
            probe("p1", unavailable("missing")),
            probe("p2", Verdict::AVAILABLE),
        ]);
        state.record(&[probe("p2", Verdict::AVAILABLE)]); // p1 gone
        // p1 returns, still missing: first-probe semantics — one
        // change, exactly as at startup.
        assert_eq!(
            state.diff_against_last(&[
                probe("p1", unavailable("missing")),
                probe("p2", Verdict::AVAILABLE)
            ]),
            vec![ProjectAvailabilityChange {
                project_id: "p1".to_owned(),
                available: false,
                unavailable_reason: Some("missing".to_owned()),
            }]
        );
    }

    #[test]
    fn the_changes_keep_the_probe_order() {
        // The event's diff is in `sort_order` (the probe order), so
        // the UI applies changes in column order — pinned against a
        // HashMap cache deliberately (iteration order of the cache
        // must never leak into the wire).
        let state = AvailabilityState::new();
        state.record(&[
            probe("p1", Verdict::AVAILABLE),
            probe("p2", Verdict::AVAILABLE),
            probe("p3", Verdict::AVAILABLE),
        ]);
        let changes = state.diff_against_last(&[
            probe("p1", Verdict::AVAILABLE),
            probe("p2", unavailable("missing")),
            probe("p3", unavailable("no_git_entry")),
        ]);
        let ids: Vec<&str> = changes
            .iter()
            .map(|change| change.project_id.as_str())
            .collect();
        assert_eq!(ids, ["p2", "p3"]);
    }
}
