//! The background dirty-flag recalculation (spec §6.6 DB-022, OUT-013,
//! MAIN-028, §7.10's `state://dirty` event; task T-101): the service
//! that recomputes every active project's build hash and tells the UI
//! which projects' dirty verdicts changed since the last pass.
//!
//! DB-022 fixes three recalculation points — 150 ms after each
//! committed transaction, at startup, and after a sync — and this
//! module owns all three through one pass, [`recalc_and_emit`]:
//!
//! - **Startup** — the `setup` hook in `lib.rs` calls
//!   [`spawn_service`] right after the database is managed; the spawn
//!   puts the startup trigger on the bus itself, so "spawned" and
//!   "startup pass scheduled" cannot drift apart.
//! - **After each committed transaction** — the executor's point seam:
//!   `Db::run` fires the hook this module's [`commit_hook`] builds
//!   after a transaction **committed with at least one row change**
//!   (a read-only transaction, or a write that changed nothing, fires
//!   nothing), the closure drops a [`Trigger::Commit`] permit on the
//!   bus, and the service task debounces it 150 ms before
//!   recalculating.
//! - **After a sync** — the reusable entry is the function itself: the
//!   future sync pipeline (§9.4 phase 4 step 5, T-118) awaits
//!   [`recalc_and_emit`] once its DB-024 hash updates committed; the
//!   pass it runs is the same one the bus schedules, so §9.4's "send
//!   `state://dirty`" and this service share every line of the
//!   computation.
//!
//! The pass is the DB-021 rule over the T-100 snapshot: one read
//! transaction reads the ALG-001 build snapshot (SYS-005/D-105) and
//! the projects' `last_synced_hash` cells together, then the blocking
//! pool runs the shared content-build pass
//! ([`eunomia_core::content_build`] — the chunked mini-snapshots of
//! 25 and the per-chunk build/hash/verdict slice this module once
//! mirrored; consolidated there by T-265/D-145, its verdict
//! semantics pinned by that module's own suite), which builds every
//! project's markerless managed text, hashes it (DB-020) and compares
//! it against the stored cells (DB-021). The heavy half is CPU and
//! read I/O — `spawn_blocking` per SYS-001, after the transaction
//! (the D-096 ordering).
//!
//! **The diff contract (D-112's shape, one judgment different).** The
//! last known verdicts live in the [`DirtyState`] managed cache —
//! session memory, like availability's: dirty-ness is recomputed from
//! the live database on every pass, so there is nothing honest to
//! persist across restarts. A project this session has not yet
//! observed carries the implicit baseline **clean** — the opposite of
//! availability's "available" baseline but the same reasoning: the
//! first pass therefore reports exactly the projects found dirty
//! (OUT-013's startup warning — every dirty project IS a change
//! against the clean baseline, and a fully-clean startup has nothing
//! to tell the UI), while a later pass that *contradicts* a recorded
//! verdict — in either direction, dirty → clean after a sync included
//! — is a change the UI must apply. The event carries only the
//! changed projects, in `sort_order`, and is not emitted at all when
//! the diff is empty: repeated passes over an unchanged database are
//! silent, which is what makes firing one after every committed
//! transaction safe. `projects_list` reads the same cache
//! ([`DirtyState::last_verdict`], T-254 closing D-121's follow-up):
//! every verdict the wire serves is one the service truthfully
//! recorded, the command inherits the clean baseline (before the
//! first pass every verdict reads clean), and it is the reconciling
//! read for the at-least-once event above — the event may carry a
//! superseded change the cache no longer holds, and the next pass
//! re-emits one only if the world moved again, so the read, not the
//! silence, settles a stale dot.
//!
//! **Delivery is at-least-once, with a stale-record guard.** The new
//! verdicts are recorded only *after* the event was emitted
//! successfully; a failed emit leaves the cache stale so the next
//! trigger re-detects and re-emits the same diff. Overlapping passes
//! (the service task's and a future sync pipeline's direct await) can
//! finish out of order; the cache's ticket guard refuses an older
//! pass's recording after a newer one's, so the cache only moves
//! forward in pass order — but an older pass's event may still carry
//! a change a newer pass already superseded: a duplicate or spurious
//! entry a listener that sets state keyed by project id survives,
//! and the next pass corrects it only if the world moved again (an
//! unchanged world diffs empty against the newer recording and the
//! pass stays silent — `projects_list`'s read, above, is what
//! settles a stale dot; the round-1 review finding that sharpened
//! the earlier "duplicates are benign" claim into this precise one,
//! with the correction's condition made explicit by T-254's
//! round-1).
//!
//! **A panic in the service kills the consumer task** (the sole
//! channel receiver), and every later trigger permit silently fills a
//! channel nobody drains — the dirty state then only refreshes at the
//! next app start. The panics reachable inside a pass are the two
//! reasoned broken-invariant `expect`s below (both mark this module's
//! own composition bugs, not data conditions); a panic elsewhere in
//! the pass surfaces as the blocking-join error the service
//! swallows. The facade lines that would make both failures visible
//! in the log are the recorded follow-up (below).
//!
//! **The bus is the coalescing debounce.** [`DirtyState::new`] hands
//! out one `tauri::async_runtime` mpsc channel of capacity **1**:
//! [`Trigger`] permits are the currency, and a full channel swallows
//! the next `try_send` — which is exactly DB-022's "150 ms after each
//! committed transaction" read as a coalescing window. A burst of
//! commits becomes one permit; the service task sleeps 150 ms
//! ([`DEBOUNCE`]), drains whatever permits piled up (all collapsed —
//! one recalculation covers every commit behind the window), runs the
//! pass, and returns to `recv`. A commit that lands *during* a pass
//! leaves a fresh permit in the channel, so the next pass follows one
//! debounce later — no trigger is ever lost, and none is ever
//! queue-jumped. The single consumer task serializes every pass; no
//! in-flight slot is needed (the channel is the slot).
//!
//! **Logging gap, recorded honestly:** this window's parallel task
//! owns `logging.rs`, so the facade lines this service deserves (a
//! `dirty_check_failed`/`dirty_emit_failed` pair in the
//! `availability_*` shape) cannot be added here; the failure paths
//! degrade safely without them — a failed pass records nothing, and
//! the next trigger re-runs it (the at-least-once contract). The
//! follow-up belongs to the first task allowed to edit `logging.rs`
//! and is recorded in DECISIONS.md.
//!
//! **Recovery mode** (no database, the T-024 state) answers the
//! honest `NoDatabase` error through the same
//! [`crate::commands::managed_db`] edge every database command uses;
//! the service is never spawned there (the `setup` hook returns
//! before it), and the recovery screen has no matrix columns to mark.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use eunomia_core::content_build::{chunk_verdicts, group_assignments};
use eunomia_store::build_snapshot::{BuildSnapshotError, read as read_build_snapshot};
use eunomia_store::projects::ListScope;

use crate::commands::{managed_db, projects_store_to_app_error};
use crate::db::join_failure_to_app_error;
use crate::errors::{AppError, ErrorCode, ErrorDetails};

/// DB-022's window: a committed transaction's recalculation runs no
/// sooner than this after the commit that triggered it, and triggers
/// arriving inside the window collapse into that one pass (the bus
/// docs above).
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(150);

/// One entry of the `state://dirty` diff (§7.10): the project a
/// change is about, addressed the way every project action addresses
/// it (its row id), plus the verdict that now holds. The display name
/// is deliberately absent — the UI already holds it keyed by id from
/// `projects_list`, and a copy here could only drift (D-112).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct ProjectDirtyChange {
    /// The project row id (UUIDv7 TEXT, DB-002) whose dirty verdict
    /// changed since the previous pass.
    pub(crate) project_id: String,
    /// The verdict that now holds: unsynced build changes exist
    /// (DB-021) — the dot icon MAIN-028 marks the column with.
    pub(crate) dirty: bool,
}

/// The `state://dirty` event payload (§7.10): only the projects whose
/// dirty verdict changed since the previous pass, in `sort_order`
/// (the snapshot order). Never emitted with an empty `changes`.
///
/// The event name is the spec's literal wire name, pinned by the
/// manual [`tauri_specta::Event`] implementation below for the same
/// reason as `availability`'s (D-112): the derive macro would build
/// the name from the struct's identifier and cannot produce the
/// `state://` prefix, and the `derive` cargo feature that ships it is
/// not enabled (the workspace pins
/// `tauri-specta = "=2.0.0-rc.29"` with `typescript` only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub(crate) struct DirtyEvent {
    /// The changed projects; empty diffs are not emitted (see the
    /// module docs' diff contract).
    pub(crate) changes: Vec<ProjectDirtyChange>,
}

impl Event for DirtyEvent {
    /// The §7.10 wire name, verbatim. Tauri permits exactly the
    /// characters it contains (alphanumeric, `-`, `/`, `:`, `_`).
    const NAME: &'static str = "state://dirty";
}

/// What wakes the service task: a committed write (DB-022's debounced
/// trigger) or a pass wanted now (the startup spawn).
///
/// The distinction is only the debounce: [`Trigger::Commit`] waits
/// [`DEBOUNCE`] first so a burst of transactions collapses into one
/// pass, [`Trigger::Immediate`] runs the pass as soon as the task
/// picks the permit up. Both coalesce on the capacity-1 bus — an
/// `Immediate` dropped behind a pending `Commit` (or the reverse)
/// still runs exactly one pass, which is the guarantee that matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// A transaction committed with row changes: recalculate no
    /// sooner than [`DEBOUNCE`] after the commit.
    Commit,
    /// Recalculate now: the DB-022 startup pass (sent by
    /// [`spawn_service`] itself).
    Immediate,
}

/// The session state behind the service: the diff cache, the bus's
/// sending half, and the pass counter.
///
/// Managed in the `setup` hook before anything that could fail (the
/// [`crate::session_paths::SessionPaths`] shape, like
/// `AvailabilityState`): the commit hook reads it through the managed
/// state at fire time, and in recovery mode it exists unused — no
/// service is spawned, no hook is ever set.
pub(crate) struct DirtyState {
    /// The diff cache with its monotonic "applied" ticket: the last
    /// known verdicts per active project id (absent ≡ the clean
    /// baseline, module docs), plus the highest ticket that ever
    /// recorded into it. The ticket is the round-1 review's
    /// stale-record guard: two overlapping passes (the service task's
    /// and the sync pipeline's direct await) can finish out of order,
    /// and an OLDER pass recording after a newer one would leave the
    /// cache describing a past moment of the database — `record`
    /// refuses any ticket below `applied`, so the cache only ever
    /// moves forward in pass order. Entries for ids that left the
    /// active list (archived) are pruned on every recording pass.
    last: Mutex<DiffCache>,
    /// The bus's sending half (capacity 1 — the coalescing). Cloned
    /// into the commit hook's closure; a `try_send` onto a full bus is
    /// the intended drop.
    trigger: tauri::async_runtime::Sender<Trigger>,
    /// Monotonic pass tickets — the initiation order
    /// [`recalc_and_emit`] captures per pass for the guard above.
    tickets: AtomicU64,
    /// How many passes ran (attempted — success or failure). The
    /// tests' observable for "a pass happened" — the event alone
    /// cannot prove it, because an empty diff is silent by contract.
    /// The counter is written by the service loop in production too
    /// (one atomic increment per pass) but read by nothing outside
    /// the tests.
    passes: AtomicU64,
}

/// The guarded cache behind [`DirtyState::last`].
struct DiffCache {
    /// Last recorded verdict per active project id.
    verdicts: HashMap<String, bool>,
    /// The highest ticket that recorded; `0` — nothing recorded yet.
    applied: u64,
}

impl DirtyState {
    /// The empty session state and the bus's receiving half, which
    /// [`spawn_service`] consumes — the pair is created together
    /// because the channel is the state's own coalescing seam (a
    /// receiver kept elsewhere would race the service for permits).
    pub(crate) fn new() -> (Self, tauri::async_runtime::Receiver<Trigger>) {
        let (trigger, triggers) = tauri::async_runtime::channel(1);
        (
            Self {
                last: Mutex::new(DiffCache {
                    verdicts: HashMap::new(),
                    applied: 0,
                }),
                trigger,
                tickets: AtomicU64::new(0),
                passes: AtomicU64::new(0),
            },
            triggers,
        )
    }

    /// Captures the next pass ticket — the initiation order the
    /// record guard reads. Taken at pass start, before the read, so
    /// a pass that observes older data than a concurrent one carries
    /// the older ticket whatever finish order the scheduler picks.
    fn next_ticket(&self) -> u64 {
        self.tickets.fetch_add(1, Ordering::AcqRel)
    }

    /// Computes the changes `verdicts` carry against the cache,
    /// without recording them: a project is changed when its new
    /// verdict differs from the recorded one — or from the implicit
    /// clean baseline when it was never observed this session. The
    /// verdicts keep the snapshot's `sort_order`, so the diff does
    /// too. Read-only against the cache (the write half is
    /// [`Self::record`], called after a successful emit; the module
    /// docs explain the at-least-once ordering).
    fn diff_against_last(&self, verdicts: &[(String, bool)]) -> Vec<ProjectDirtyChange> {
        let last = self
            .last
            .lock()
            .expect("the dirty cache is locked only inside this short synchronous section");
        verdicts
            .iter()
            .filter_map(|(id, dirty)| {
                if last.verdicts.get(id).copied().unwrap_or(false) == *dirty {
                    None
                } else {
                    Some(ProjectDirtyChange {
                        project_id: id.clone(),
                        dirty: *dirty,
                    })
                }
            })
            .collect()
    }

    /// Records the verdicts under `ticket` and prunes the ids that
    /// are no longer active — the cache ends up exactly the observed
    /// world, whatever it held before. The stale-record guard: a
    /// ticket older than [`DiffCache::applied`] records nothing (a
    /// newer pass already moved the cache; the older pass's event was
    /// still emitted — at-least-once — and a later pass corrects any
    /// spurious change it carried only if the world moved again; an
    /// unchanged world diffs empty against the newer recording and
    /// stays silent, so `projects_list`'s read is what settles it —
    /// the module docs' overlap contract, sharpened by T-254's
    /// reviews). Pruning emits nothing: an
    /// archived project has no column to mark anymore (archiving
    /// re-renders the matrix), and a stale entry would resurface as a
    /// phantom change if the project is restored later.
    fn record(&self, verdicts: &[(String, bool)], ticket: u64) {
        let mut last = self
            .last
            .lock()
            .expect("the dirty cache is locked only inside this short synchronous section");
        if ticket < last.applied {
            return;
        }
        last.applied = ticket;
        let live: HashSet<&str> = verdicts.iter().map(|(id, _)| id.as_str()).collect();
        last.verdicts.retain(|id, _| live.contains(id.as_str()));
        for (id, dirty) in verdicts {
            last.verdicts.insert(id.clone(), *dirty);
        }
    }

    /// The last recorded verdict of one project (DB-021) — the read
    /// side of the cache `projects_list` serves (T-254, D-121's
    /// follow-up): the same map the service diffs its next
    /// `state://dirty` event against, so every verdict the wire
    /// serves is one the service truthfully recorded. The event
    /// itself is at-least-once and may carry a superseded change
    /// this cache no longer holds (the module docs' overlap
    /// contract), so a consumer reconciling the two treats the read
    /// as the truth. An id this session has not
    /// observed — before the service's first pass — reads as the
    /// baseline **clean**: the diff rule's own absent ≡ clean
    /// reading ([`Self::diff_against_last`]), not a separate default
    /// the two halves could disagree on.
    pub(crate) fn last_verdict(&self, project_id: &str) -> bool {
        let last = self
            .last
            .lock()
            .expect("the dirty cache is locked only inside this short synchronous section");
        last.verdicts.get(project_id).copied().unwrap_or(false)
    }

    /// Drops one permit onto the bus; a full bus swallows it (the
    /// pending permit already covers this trigger — the coalescing).
    /// `pub(crate)`: the commit-hook closure and the tests both fire
    /// triggers without naming the channel type.
    pub(crate) fn trigger(&self, trigger: Trigger) {
        let _ = self.trigger.try_send(trigger);
    }

    /// Notes that one pass ran. Private: only the service task counts.
    fn note_pass(&self) {
        self.passes.fetch_add(1, Ordering::Release);
    }

    /// The number of passes that ran — the tests' deterministic
    /// observable for the debounce and coalescing assertions (they
    /// poll it, then read the event receiver).
    #[cfg(test)]
    pub(crate) fn pass_count(&self) -> u64 {
        self.passes.load(Ordering::Acquire)
    }
}

/// The DB-022 pass every trigger shares: read the snapshot and the
/// stored hashes in one transaction, compute the verdicts on the
/// blocking pool, diff against the session cache, and emit
/// `state://dirty` when the diff is non-empty.
///
/// The returned changes are exactly what the event carried (empty —
/// nothing was emitted): the future sync pipeline (§9.4 phase 4, its
/// step 5 "send `state://dirty`") awaits this as its after-sync
/// recalculation and may log or branch on the diff. Failures
/// propagate in the §7.10 `AppError` shape — the honest
/// `NoDatabase` answer in recovery mode, the classified store errors
/// otherwise — and emit nothing: a read that could not see the data
/// has no diff worth claiming.
pub(crate) async fn recalc_and_emit<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<ProjectDirtyChange>, AppError> {
    let database = managed_db(app)?;
    // The pass's ticket — its initiation order in the stale-record
    // guard — captured before the read, so a pass observing older
    // data than a concurrent one carries the older ticket however the
    // scheduler interleaves their tails.
    let ticket = app.state::<DirtyState>().next_ticket();
    // One read transaction for both halves (SYS-005/D-105): the
    // snapshot and the stored hashes must describe the same committed
    // state, or a diff between them would be a lie about a single
    // moment of the database.
    let (snapshot, rows) = database
        .run(move |transaction| {
            let snapshot = read_build_snapshot(transaction).map_err(build_snapshot_to_app_error)?;
            let rows = eunomia_store::projects::list(transaction, ListScope::Active)
                .map_err(projects_store_to_app_error)?;
            Ok((snapshot, rows))
        })
        .await?;
    let stored_hashes: HashMap<String, Option<String>> = rows
        .into_iter()
        .map(|project| (project.id, project.last_synced_hash))
        .collect();

    // The build+hash half is CPU over the whole library (§12.2's
    // budget): the blocking pool, after the transaction — SYS-001,
    // the D-096 ordering — with the chunks of
    // [`eunomia_core::content_build`] (the shared pass, T-265/D-145)
    // as parallel units; the sequential spelling of the same pass is
    // `content_build::compute_dirty_verdicts`. Awaiting the handles
    // in chunk order keeps the verdicts in `sort_order` however the
    // tasks interleave, and a panicked chunk surfaces as the honest
    // blocking-join failure.
    let snapshot = std::sync::Arc::new(snapshot);
    let grouped = std::sync::Arc::new(group_assignments(&snapshot));
    let chunk_tasks: Vec<_> = eunomia_core::content_build::chunk_starts(snapshot.projects().len())
        .map(|start| {
            let snapshot = std::sync::Arc::clone(&snapshot);
            let grouped = std::sync::Arc::clone(&grouped);
            let stored_hashes = stored_hashes.clone();
            tauri::async_runtime::spawn_blocking(move || {
                chunk_verdicts(&snapshot, &grouped, start, &stored_hashes)
            })
        })
        .collect();
    let mut verdicts: Vec<(String, bool)> = Vec::with_capacity(snapshot.projects().len());
    for task in chunk_tasks {
        let chunk = task.await.map_err(join_failure_to_app_error)?;
        verdicts.extend(chunk);
    }

    let state = app.state::<DirtyState>();
    let changes = state.diff_against_last(&verdicts);
    if !changes.is_empty() {
        let event = DirtyEvent {
            changes: changes.clone(),
        };
        if let Err(_error) = event.emit(app) {
            // Deliberately NOT recording the verdicts: the cache stays
            // stale so the next trigger re-detects and re-emits this
            // diff (the module docs' at-least-once contract). The
            // recalculation itself succeeded — the failure is the UI
            // plumbing's, so the changes still go back to the caller.
            // The `warn` facade line belongs to the logging follow-up
            // recorded in the module docs (the parallel window owns
            // `logging.rs`).
            return Ok(changes);
        }
    }
    state.record(&verdicts, ticket);
    Ok(changes)
}

/// Spawns the service task over the bus's receiving half and sends
/// the DB-022 startup trigger — the `setup` hook's one call. The task
/// is the bus's only consumer, so passes serialize on it: `recv` a
/// trigger, debounce it when it is a [`Trigger::Commit`], drain
/// whatever permits collapsed behind it, run one
/// [`recalc_and_emit`], repeat. A failed pass is counted and
/// swallowed — there is no caller to return to, the cache recorded
/// nothing, and the next trigger re-runs it (at-least-once); the
/// `error` facade line is the same recorded follow-up.
///
/// The startup trigger is sent **before** the spawn, so it can never
/// race the task's first `recv`: the capacity-1 bus holds it. A
/// commit permit that somehow preceded it (the retention task's
/// startup cleanup writes) already covers the pass — one pass runs
/// either way, which is the coalescing guarantee.
pub(crate) fn spawn_service<R: Runtime>(
    app: AppHandle<R>,
    triggers: tauri::async_runtime::Receiver<Trigger>,
) {
    spawn_service_with_window(app, triggers, DEBOUNCE);
}

/// The service loop with an explicit debounce window: production
/// takes [`DEBOUNCE`] above; tests that count passes across a burst
/// of real commits widen it (CI 44947235666: on a loaded runner three
/// `block_on` writes can straddle a 150 ms window, splitting the
/// burst — the coalescing proof needs the window to dominate the
/// commit cadence, not the wall clock to be kind).
pub(crate) fn spawn_service_with_window<R: Runtime>(
    app: AppHandle<R>,
    mut triggers: tauri::async_runtime::Receiver<Trigger>,
    window: Duration,
) {
    app.state::<DirtyState>().trigger(Trigger::Immediate);
    tauri::async_runtime::spawn(async move {
        while let Some(trigger) = triggers.recv().await {
            if matches!(trigger, Trigger::Commit) {
                // DB-022's window. Tokio's sleep is the house-approved
                // timer (the retention task's D-054 precedent — the
                // `time` feature src-tauri already declares).
                tokio::time::sleep(window).await;
            }
            // Triggers that arrived during the sleep (or during an
            // earlier pass) collapse here: their recalculation IS this
            // pass.
            while triggers.try_recv().is_ok() {}
            let _ = recalc_and_emit(&app).await;
            app.state::<DirtyState>().note_pass();
        }
    });
}

/// Builds the executor's commit seam closure (the `setup` hook hands
/// it to `Db::set_commit_hook`): one [`Trigger::Commit`] permit onto
/// the bus per committed write transaction. The closure runs on the
/// blocking pool while the connection's mutex is still held, so it
/// must never block — `try_send` onto the capacity-1 bus is exactly
/// that, and a full bus is the coalescing, not a loss (the pending
/// permit already schedules the pass that will read whatever this
/// transaction wrote).
pub(crate) fn commit_hook<R: Runtime>(app: AppHandle<R>) -> std::sync::Arc<dyn Fn() + Send + Sync> {
    std::sync::Arc::new(move || {
        // `try_state`, never `state`: a background hook has no caller
        // to panic to. The state is managed before the hook can be
        // set (the `setup` order), so this is belt-and-suspenders.
        if let Some(state) = app.try_state::<DirtyState>() {
            state.trigger(Trigger::Commit);
        }
    })
}

/// Maps the snapshot read's failures onto the §7.10 wire — the
/// D-093/D-096 boundary in the `projects_store_to_app_error` shape:
/// SQLite failures through the shared base classifier, corrupt
/// vocabulary cells as class-only fallbacks (never the stored value —
/// a tampered cell is arbitrary database text on the SEC-008
/// surfaces), and the graph-validation arm as its own honest
/// fallback. The library read has no command-layer mapper yet (the
/// library commands are another task's), so its arms are mapped here
/// in the same shape; of `LibraryError`'s many variants only `Sql`
/// and `Member` are reachable from the three reads the snapshot
/// performs, so the rest share one family fallback rather than a
/// variant-by-variant litany a future library variant would break for
/// no behavioral reason (a write-path outcome arriving here is a
/// broken-invariant event either way). `pub(crate)`: the set-L budget
/// test's read probe maps through the production seam instead of
/// duplicating the fallback spellings.
pub(crate) fn build_snapshot_to_app_error(error: BuildSnapshotError) -> AppError {
    match error {
        BuildSnapshotError::Projects(source) => projects_store_to_app_error(source),
        BuildSnapshotError::Library(source) => match source {
            eunomia_store::library::LibraryError::Sql(sql) => {
                crate::db::sqlite_error_to_app_error(sql)
            }
            eunomia_store::library::LibraryError::Member { column, .. } => dirty_fallback(
                "LibraryCorrupt",
                &format!("a stored {column} cell is outside its vocabulary"),
            ),
            write_outcome => dirty_fallback(
                "LibraryError",
                match write_outcome {
                    eunomia_store::library::LibraryError::NotFound { .. }
                    | eunomia_store::library::LibraryError::NotAnInstruction { .. }
                    | eunomia_store::library::LibraryError::NotAHeading { .. }
                    | eunomia_store::library::LibraryError::DuplicateInstructionTitle { .. }
                    | eunomia_store::library::LibraryError::DuplicateVariantName { .. }
                    | eunomia_store::library::LibraryError::MainVariantExists { .. }
                    | eunomia_store::library::LibraryError::MainRequired { .. }
                    | eunomia_store::library::LibraryError::LastVariant { .. }
                    | eunomia_store::library::LibraryError::ReplacementRequired { .. }
                    | eunomia_store::library::LibraryError::ReplacementInvalid { .. }
                    | eunomia_store::library::LibraryError::NoActiveMain { .. }
                    | eunomia_store::library::LibraryError::PositionOutOfRange { .. }
                    | eunomia_store::library::LibraryError::InvalidItemOrder { .. }
                    | eunomia_store::library::LibraryError::DuplicateOrderId { .. }
                    | eunomia_store::library::LibraryError::UnknownOrderId { .. }
                    | eunomia_store::library::LibraryError::AlreadyAssigned { .. }
                    | eunomia_store::library::LibraryError::VariantNotAssignable { .. }
                    | eunomia_store::library::LibraryError::VariantNotActive { .. } => {
                        "a write-path library outcome reached a read"
                    }
                    eunomia_store::library::LibraryError::Sql(_)
                    | eunomia_store::library::LibraryError::Member { .. } => {
                        unreachable!("the Sql and Member arms are matched above")
                    }
                },
            ),
        },
        BuildSnapshotError::HeadingLevel(source) => dirty_fallback(
            "LibraryCorrupt",
            &format!("a stored heading level is outside 2..=6: {source}"),
        ),
        BuildSnapshotError::Invalid(source) => dirty_fallback(
            "SnapshotInvalid",
            &format!("the assembled snapshot is inconsistent: {source}"),
        ),
    }
}

/// Builds the D-037 fallback with a family label and a fixed,
/// class-only diagnostic — the `projects_fallback`/`storage_fallback`
/// shape (those builders are private to `commands.rs`; this is the
/// same spelling for this module's own arms).
fn dirty_fallback(kind: &str, message: &str) -> AppError {
    let mut app_error = AppError::new(ErrorCode::DbLocked);
    app_error.details = Some(ErrorDetails {
        os_code: None,
        os_kind: Some(kind.to_owned()),
        os_message: Some(message.to_owned()),
    });
    app_error
}

/// The assertion-side seam of the library seeding failures for the
/// test suites (T-304, D-162's play over the sibling family): the
/// eight per-suite `LibraryError` → `AppError` seed helpers (the
/// `library_seed_error` of the five `sync_phase*_tests.rs` suites and
/// of `dirty_recalc_tests.rs`, `sync_library_seed_error` and
/// `library_watch_seed_error` in `ipc_tests.rs` — the census D-161
/// already inventoried) each hand-rolled their own copy of the
/// mapping; they all route through here now. The `Sql` arm routes
/// through the production mapping — [`build_snapshot_to_app_error`]'s
/// `Library` arm, wrapped as `BuildSnapshotError::Library` — so a
/// suite's mapped `Sql` envelope is exactly the envelope production
/// puts on the wire (the D-149/D-162 one-spelling law, now on the
/// library test side too; the arm's treatment there today is the
/// shared base classifier). The other arms cannot be produced by the
/// suites and panic instead of inventing an envelope (the D-162
/// documented-panic shape): the old helpers' silent
/// `DbLocked`+`LibrarySeed`+`Display` fallback was never observed by
/// any test.
///
/// # Panics
///
/// Panics if `error` carries the `Member` envelope — the seeds write
/// only through the sanctioned library writers, so no suite can hand
/// the helper a corrupt vocabulary cell — or a write-path outcome
/// (`NotFound`, `DuplicateInstructionTitle`, `AlreadyAssigned`, and
/// the rest): the seeds construct valid-by-construction inputs (ids
/// read back from the rows they just wrote; titles unique within the
/// test's database — the process-unique counter titles of the phase
/// 1–4/ipc fixtures, distinct fixture titles per call site
/// elsewhere). The panic names the arm and embeds the error's
/// `Display` (deliberately value-embedding — assertion-side wording
/// that never reaches the wire; the wire's class-only rule stays in
/// the production mapping, SEC-008). The guards are pinned by
/// `dirty_recalc_tests`'s
/// `the_library_seed_helper_panics_on_the_unreachable_member_envelope`
/// and
/// `the_library_seed_helper_panics_on_an_unreachable_write_path_outcome`;
/// the `Sql` delegation by
/// `the_library_seed_helper_routes_sql_through_the_classifier`.
#[cfg(test)]
pub(crate) fn library_seed_error_for_tests(
    error: eunomia_store::library::LibraryError,
) -> AppError {
    match error {
        error @ eunomia_store::library::LibraryError::Sql(_) => {
            build_snapshot_to_app_error(BuildSnapshotError::Library(error))
        }
        error @ eunomia_store::library::LibraryError::Member { .. } => panic!(
            "the assertion-side library seed hit the `Member` envelope, which the suites cannot produce (the seeds write only through the sanctioned library writers): {error}"
        ),
        error @ (eunomia_store::library::LibraryError::NotFound { .. }
        | eunomia_store::library::LibraryError::NotAnInstruction { .. }
        | eunomia_store::library::LibraryError::NotAHeading { .. }
        | eunomia_store::library::LibraryError::DuplicateInstructionTitle { .. }
        | eunomia_store::library::LibraryError::DuplicateVariantName { .. }
        | eunomia_store::library::LibraryError::MainVariantExists { .. }
        | eunomia_store::library::LibraryError::MainRequired { .. }
        | eunomia_store::library::LibraryError::LastVariant { .. }
        | eunomia_store::library::LibraryError::ReplacementRequired { .. }
        | eunomia_store::library::LibraryError::ReplacementInvalid { .. }
        | eunomia_store::library::LibraryError::NoActiveMain { .. }
        | eunomia_store::library::LibraryError::PositionOutOfRange { .. }
        | eunomia_store::library::LibraryError::InvalidItemOrder { .. }
        | eunomia_store::library::LibraryError::DuplicateOrderId { .. }
        | eunomia_store::library::LibraryError::UnknownOrderId { .. }
        | eunomia_store::library::LibraryError::AlreadyAssigned { .. }
        | eunomia_store::library::LibraryError::VariantNotAssignable { .. }
        | eunomia_store::library::LibraryError::VariantNotActive { .. }) => panic!(
            "the assertion-side library seed hit a write-path outcome, which the suites cannot produce (the seeds construct valid-by-construction inputs — ids read back from the rows they just wrote, titles unique within the test's database): {error}"
        ),
    }
}

#[cfg(test)]
mod tests {
    //! The service's own pure halves — the diff semantics over the
    //! clean baseline and the wire name. The verdict computation
    //! (DB-021 through the real build) and the managed-text seam
    //! moved to the shared mechanics' suite in
    //! `eunomia_core::content_build` when this module's private
    //! spelling dissolved (T-265/D-145); the mock-runtime halves (the
    //! triggers, the debounce, the card's criteria, the L budget)
    //! live in `dirty_recalc_tests.rs`, which owns its own mock-app
    //! harness.

    use super::*;

    fn verdict(id: &str, dirty: bool) -> (String, bool) {
        (id.to_owned(), dirty)
    }

    #[test]
    fn the_debounce_window_is_the_spec_literal() {
        // DB-022 pins 150 ms; the burst test runs with a widened
        // window (CI 44999287190), so the production value needs its
        // own pin (review finding P2-2).
        assert_eq!(super::DEBOUNCE, Duration::from_millis(150));
    }

    fn change(id: &str, dirty: bool) -> ProjectDirtyChange {
        ProjectDirtyChange {
            project_id: id.to_owned(),
            dirty,
        }
    }

    // ---- the wire name --------------------------------------------------

    #[test]
    fn the_event_name_is_the_spec_literal() {
        // The wire name is the frontend's whole contract; pinned
        // against the §7.10 literal exactly like availability's.
        assert_eq!(DirtyEvent::NAME, "state://dirty");
    }

    // ---- the diff semantics (the D-112 shape over the clean baseline) ---

    /// The state without its receiver — the pure half's fixture.
    fn state_only() -> DirtyState {
        DirtyState::new().0
    }

    #[test]
    fn a_first_observation_of_a_clean_project_is_no_change() {
        // The baseline rule: an unobserved project counts as clean, so
        // a fully-clean first pass has nothing to report.
        let state = state_only();
        assert_eq!(state.diff_against_last(&[verdict("p1", false)]), Vec::new());
    }

    #[test]
    fn a_first_observation_of_a_dirty_project_is_a_change() {
        // OUT-013's startup warning: every project found dirty IS a
        // change against the clean baseline — the marking must not
        // wait for a second pass.
        let state = state_only();
        assert_eq!(
            state.diff_against_last(&[verdict("p1", true)]),
            vec![change("p1", true)]
        );
    }

    #[test]
    fn a_changed_verdict_is_reported_in_both_directions() {
        // dirty → clean (a sync stored the matching hash) and clean →
        // dirty (an edit moved the build) are both changes: the
        // MAIN-028 dot has to be set AND cleared.
        let state = state_only();
        state.record(&[verdict("p1", true)], 1);
        assert_eq!(
            state.diff_against_last(&[verdict("p1", false)]),
            vec![change("p1", false)]
        );
        state.record(&[verdict("p1", false)], 2);
        assert_eq!(
            state.diff_against_last(&[verdict("p1", true)]),
            vec![change("p1", true)]
        );
    }

    #[test]
    fn an_unchanged_verdict_is_not_a_change_even_dirty() {
        // Idempotence, both halves: repeated passes over an unchanged
        // database are silent — what makes firing after every commit
        // safe.
        let state = state_only();
        state.record(&[verdict("p1", true), verdict("p2", false)], 1);
        assert_eq!(
            state.diff_against_last(&[verdict("p1", true), verdict("p2", false)]),
            Vec::new()
        );
    }

    #[test]
    fn recording_prunes_ids_that_left_the_active_list() {
        // The prune rule: an archived project's cache entry disappears
        // with the next pass; restoring the id later looks like a
        // FIRST observation (clean baseline), not a recovery from a
        // verdict the active list no longer holds.
        let state = state_only();
        state.record(&[verdict("p1", true)], 1);
        state.record(&[], 2); // p1 gone from the observed world
        assert_eq!(
            state.diff_against_last(&[verdict("p1", true)]),
            vec![change("p1", true)],
            "the restored project is a first observation again, not an \
             unchanged verdict"
        );
    }

    #[test]
    fn an_older_pass_finishing_last_records_nothing() {
        // The round-1 review's stale-record guard: two overlapping
        // passes can finish out of order, and the older one recording
        // after the newer would rewind the cache to a past moment of
        // the database. The guard refuses tickets below `applied` —
        // observable through the diff: the older verdict must not
        // resurface as a change against the newer recording.
        let state = state_only();
        // The newer pass (ticket 2) records p1 clean…
        state.record(&[verdict("p1", false)], 2);
        // …then the older pass (ticket 1, started earlier, observing
        // p1 still dirty) finishes late: its record is refused.
        state.record(&[verdict("p1", true)], 1);
        assert_eq!(
            state.diff_against_last(&[verdict("p1", true)]),
            vec![change("p1", true)],
            "the cache still holds the newer pass's clean verdict — \
             the older one is a change against it, not the truth"
        );
        assert_eq!(
            state.diff_against_last(&[verdict("p1", false)]),
            Vec::new(),
            "the newer recording is the cache's content"
        );
    }

    #[test]
    fn the_changes_keep_the_snapshot_order() {
        // The diff is in `sort_order` (the verdicts' order), so the UI
        // applies changes in column order — pinned against the HashMap
        // cache deliberately.
        let state = state_only();
        state.record(
            &[
                verdict("p1", false),
                verdict("p2", false),
                verdict("p3", false),
            ],
            1,
        );
        let changes = state.diff_against_last(&[
            verdict("p1", false),
            verdict("p2", true),
            verdict("p3", true),
        ]);
        assert_eq!(changes, vec![change("p2", true), change("p3", true)]);
    }
}
