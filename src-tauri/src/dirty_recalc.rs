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
//! the projects' `last_synced_hash` cells together, the blocking pool
//! then builds every project's markerless managed text (the D-064
//! hash seam: the §9.2 body, scheme A's reading lines spliced around
//! it — the byte-exact mirror of `target_file`'s private
//! `managed_block`, pinned against it by a test), hashes it through
//! [`eunomia_core::hash::build_hash`] (DB-020), and hands the facts to
//! [`eunomia_core::dirty::is_dirty`]. The heavy half is CPU and read
//! I/O — `spawn_blocking` per SYS-001, after the transaction (the
//! D-096 ordering).
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
//! transaction safe.
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
//! entry a listener that sets state keyed by project id survives, and
//! the next pass corrects (the round-1 review finding that sharpened
//! the earlier "duplicates are benign" claim into this precise one).
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

use eunomia_core::backup_name::target_file_name;
use eunomia_core::body::build_body;
use eunomia_core::dirty::{ProjectState, is_dirty};
use eunomia_core::extra_files::{ExtraFileLines, extra_file_lines};
use eunomia_core::hash::build_hash;
use eunomia_core::project::Scheme;
use eunomia_core::snapshot::BuildSnapshot;
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
    /// still emitted — at-least-once — and the next pass corrects any
    /// spurious change it carried). Pruning emits nothing: an
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

/// How many projects share one scoped mini-snapshot in
/// [`compute_dirty_verdicts`] — the batching knob of the §12.2 set-L
/// budget, and the parallel unit of the pass (one `spawn_blocking`
/// per chunk in [`recalc_and_emit`]). See that function's docs for
/// the arithmetic.
///
/// 25 keeps the per-`build_body` assignment rescan at
/// `25 × (assignments / projects)` (set L: 5,000 rows — 2.5M compares
/// over the whole pass) while paying the item-list clone and the
/// constructor's validation only `500 / 25 = 20` times instead of
/// once per project. Smaller chunks trade clones for rescans and
/// larger ones the reverse; 25 sits in the flat middle for both
/// bounds (anything from ~10 to ~50 measures within noise of the
/// same total on set L).
const MINI_SNAPSHOT_PROJECTS: usize = 25;

/// The assignment half of the batching: row indices into the
/// snapshot's assignment list, grouped by project id — one pass over
/// the assignments instead of a rescan per chunk, and the grouping is
/// built once per pass (owned keys, so an `Arc` of it shares across
/// the parallel chunk tasks). The same map answers DB-021's exception
/// sentence (`has_assignments`), to a hidden instruction's assignment
/// alike (DB-013).
type AssignmentsByProject = HashMap<String, Vec<usize>>;

/// Groups the snapshot's assignment row indices by project id.
fn group_assignments(snapshot: &BuildSnapshot) -> AssignmentsByProject {
    let mut grouped: AssignmentsByProject = HashMap::with_capacity(snapshot.projects().len());
    for (index, assignment) in snapshot.assignments().iter().enumerate() {
        grouped
            .entry(assignment.project_id.as_str().to_owned())
            .or_default()
            .push(index);
    }
    grouped
}

/// Computes every active project's dirty verdict (DB-021 over the
/// T-100 snapshot): the markerless managed text per D-064's seam, its
/// DB-020 hash, and the comparison with the stored
/// `last_synced_hash`.
///
/// Pure — no clock, no I/O, no runtime — so the rule is testable
/// without a database (the integration half in
/// `dirty_recalc_tests.rs` drives it through [`recalc_and_emit`]).
/// `stored_hashes` is keyed by project row id; a missing key reads as
/// *never synced* (the column is NULL until DB-024's first writer —
/// `None` and an empty string mean the same thing, D-068).
///
/// # The batching the §12.2 budget needs
///
/// `build_body` is per-project and rescans the snapshot's whole
/// assignment list every call — its own docs assign the multi-project
/// reuse to the caller ("a full sync building many projects calls it
/// per project, and batching that reuse belongs to the sync
/// pipeline"). This is that batching, on set L's numbers: calling it
/// naively per project over one snapshot means 500 × 100,000
/// assignment comparisons (50M — the quadratic the 500 ms budget
/// cannot hold), while one singleton snapshot per project means
/// cloning and re-validating the 1,100-item library 500 times (four
/// million short-lived allocations — measured at twice the budget in
/// release). The middle: **scoped mini-snapshots of
/// [`MINI_SNAPSHOT_PROJECTS`] projects** — the same items, only the
/// chunk's assignments — so `selected_variants`' internal project
/// filter meets a mostly-scoped list and both costs land in the flat
/// middle. The scoping is body-equivalent by construction: filtering
/// the assignment list by the project id is exactly what
/// `selected_variants` does inside, so every project's built bytes
/// are the full snapshot's bytes (pinned by the crossing-wires unit
/// test and the integration hash pins).
///
/// [`recalc_and_emit`] runs the same composition with the chunks on
/// parallel blocking-pool tasks ([`chunk_verdicts`] is the per-chunk
/// half this sequential spelling loops over); small snapshots have
/// one chunk and both spellings coincide. The sequential spelling is
/// the unit tests' entry and the L probe's cross-check against the
/// parallel pass — production awaits [`recalc_and_emit`] — hence the
/// test-gated `allow`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn compute_dirty_verdicts(
    snapshot: &BuildSnapshot,
    stored_hashes: &HashMap<String, Option<String>>,
) -> Vec<(String, bool)> {
    let grouped = group_assignments(snapshot);
    let mut verdicts: Vec<(String, bool)> = Vec::with_capacity(snapshot.projects().len());
    for start in (0..snapshot.projects().len()).step_by(MINI_SNAPSHOT_PROJECTS) {
        verdicts.extend(chunk_verdicts(snapshot, &grouped, start, stored_hashes));
    }
    verdicts
}

/// Computes the verdicts of the projects in chunk `start..` (up to
/// [`MINI_SNAPSHOT_PROJECTS`] of them, `start` a multiple of it): the
/// shared sequential/parallel per-chunk half of
/// [`compute_dirty_verdicts`] — a fresh scoped mini-snapshot of the
/// chunk, then the §9.2 body and the DB-020 hash per project, in
/// chunk (=`sort_order`) order.
fn chunk_verdicts(
    snapshot: &BuildSnapshot,
    grouped: &AssignmentsByProject,
    start: usize,
    stored_hashes: &HashMap<String, Option<String>>,
) -> Vec<(String, bool)> {
    // The last chunk may be shorter than MINI_SNAPSHOT_PROJECTS —
    // clamp the range end, or `get` would answer None and silently
    // drop the chunk's projects (the release suite caught exactly
    // that: every small fixture is one partial chunk).
    let end = (start + MINI_SNAPSHOT_PROJECTS).min(snapshot.projects().len());
    let Some(projects) = snapshot.projects().get(start..end) else {
        // `start` is always a chunk boundary of the sequential loop;
        // the parallel path hands over the same boundaries.
        return Vec::new();
    };
    // This chunk's assignments, scoped once from the grouped indices.
    let mut scoped: Vec<eunomia_core::assignment::Assignment> = Vec::new();
    for project in projects {
        if let Some(indices) = grouped.get(project.id.as_str()) {
            scoped.extend(
                indices
                    .iter()
                    .map(|&index| snapshot.assignments()[index].clone()),
            );
        }
    }
    let mini = BuildSnapshot::new(projects.to_vec(), snapshot.items().to_vec(), scoped).expect(
        "the full snapshot passed the same validation: a subset of its \
         assignments and projects over the same items cannot fail it",
    );
    projects
        .iter()
        .map(|project| {
            // `build_body` answers None only for a project the
            // snapshot does not hold; the mini holds this chunk's, so
            // None is a broken invariant of this composition — the
            // reasoned-panic posture (D-061), never a silent default
            // (a `unwrap_or_default` here would mask a batching bug:
            // the release suite caught exactly such a drop).
            let body = build_body(&mini, &project.id)
                .expect("the mini-snapshot holds every project of its chunk");
            let settings = &project.settings;
            // D-064's seam, mirrored (see `managed_text`'s docs): the
            // assembled managed block, marker excluded, scheme A's
            // reading lines included. The argument order is
            // project-then-local — §7.5's tie order, the reverse of
            // the settings struct's field order.
            let lines = extra_file_lines(
                settings.base_name,
                settings.extra_project,
                settings.extra_local,
            );
            let managed = managed_text(settings.scheme, &body, &lines);
            // The DB-020 target path: `<root>/<base_name>` (A, B) or
            // `<root>/<STEM>.project.md` (C) — §9.4 phase 1 step 2's
            // spelling, through the one public file-name owner.
            let target_path = project
                .root
                .join(target_file_name(settings.scheme, settings.base_name));
            let current_hash = build_hash(
                settings.scheme,
                &target_path,
                settings.extra_local,
                settings.extra_project,
                &managed,
            );
            let stored = stored_hashes
                .get(project.id.as_str())
                .and_then(|hash| hash.as_deref());
            // The snapshot holds active projects only (ALG-001), so
            // the state fact is `Active` by construction — DB-014's
            // archived half is the store scope's work, already done.
            let dirty = is_dirty(
                ProjectState::Active,
                stored,
                &current_hash,
                &managed,
                grouped.contains_key(project.id.as_str()),
            );
            (project.id.as_str().to_owned(), dirty)
        })
        .collect()
}

/// The markerless managed text of one project — D-064's seam ("the
/// assembled managed, marker excluded"): ALG-030's parts joined with
/// one empty line between neighbours, scheme A splicing the reading
/// lines around the body, schemes B and C taking the body alone
/// (ALG-030's "(A only)" gate; B's managed block "is the body alone",
/// `assembly_b`'s docs).
///
/// A deliberate byte-exact mirror of `target_file`'s private
/// `managed_block` (D-065): that helper is private, and its public
/// wrapper [`eunomia_core::target_file::build_content`] panics on
/// scheme B by design (ALG-031 is a different builder), so the hash
/// seam needs its own spelling of the join. The mirror is pinned by a
/// test against `build_content`'s real output (the file minus its
/// marker tail) for schemes A and C, and against the body for B —
/// drift fails the suite.
fn managed_text(scheme: Scheme, body: &str, lines: &ExtraFileLines) -> String {
    /// One reading-lines position as a single owned block: a
    /// position's lines are ONE part (adjacent lines joined with
    /// single breaks), and only neighbouring parts get the empty line
    /// — the same two-level split `managed_block` performs.
    fn lines_part(block: &[String]) -> Option<String> {
        (!block.is_empty()).then(|| block.join("\n"))
    }

    let mut parts: Vec<String> = Vec::with_capacity(3);
    match scheme {
        Scheme::A => {
            if let Some(start) = lines_part(&lines.start) {
                parts.push(start);
            }
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
            if let Some(end) = lines_part(&lines.end) {
                parts.push(end);
            }
        }
        Scheme::B | Scheme::C => {
            if !body.is_empty() {
                parts.push(body.to_owned());
            }
        }
    }
    parts.join("\n\n")
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
    // [`compute_dirty_verdicts`] as parallel units (the sequential
    // spelling is that function; this is the same composition with
    // the chunk loop spread over the pool). Awaiting the handles in
    // chunk order keeps the verdicts in `sort_order` however the
    // tasks interleave, and a panicked chunk surfaces as the honest
    // blocking-join failure.
    let snapshot = std::sync::Arc::new(snapshot);
    let grouped = std::sync::Arc::new(group_assignments(&snapshot));
    let chunk_tasks: Vec<_> = (0..snapshot.projects().len())
        .step_by(MINI_SNAPSHOT_PROJECTS)
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
    mut triggers: tauri::async_runtime::Receiver<Trigger>,
) {
    app.state::<DirtyState>().trigger(Trigger::Immediate);
    tauri::async_runtime::spawn(async move {
        while let Some(trigger) = triggers.recv().await {
            if matches!(trigger, Trigger::Commit) {
                // DB-022's window. Tokio's sleep is the house-approved
                // timer (the retention task's D-054 precedent — the
                // `time` feature src-tauri already declares).
                tokio::time::sleep(DEBOUNCE).await;
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

#[cfg(test)]
mod tests {
    //! The pure halves of the service — the diff semantics, the
    //! verdict computation, the managed-text mirror, the wire name.
    //! The mock-runtime halves (the triggers, the debounce, the
    //! card's criteria, the L budget) live in `dirty_recalc_tests.rs`,
    //! which owns its own mock-app harness.

    use std::collections::HashMap;
    use std::path::Path;

    use super::*;
    use eunomia_core::assignment::Assignment;
    use eunomia_core::extra_files::extra_file_lines;
    use eunomia_core::library::{
        Instruction, InstructionState, ItemId, LibraryItem, Variant, VariantId,
    };
    use eunomia_core::project::{
        BaseName, ExtraFile, ExtraFilePosition, ProjectId, ProjectWriteSettings,
    };
    use eunomia_core::snapshot::SnapshotProject;
    use eunomia_core::target_file::build_content;

    // ---- fixtures ------------------------------------------------------

    /// The extras-off option both fields default to in these fixtures.
    fn off() -> ExtraFile {
        ExtraFile {
            enabled: false,
            position: ExtraFilePosition::End,
        }
    }

    /// A canonical scheme-A project (the `body` module's fixture
    /// shape); tests vary the settings through mutation.
    fn project(id: &str) -> SnapshotProject {
        SnapshotProject {
            id: ProjectId::new(id.to_owned()),
            name: format!("project {id}"),
            root: Path::new("/repos").join(id),
            settings: ProjectWriteSettings {
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: off(),
                extra_project: off(),
            },
        }
    }

    /// An active instruction with one variant `v-<id>` carrying
    /// `content`.
    fn instruction(id: &str, content: &str) -> LibraryItem {
        LibraryItem::Instruction(Instruction {
            id: ItemId::new(id.to_owned()),
            title: format!("Instruction {id}"),
            state: InstructionState::Active,
            variants: vec![Variant {
                id: VariantId::new(format!("v-{id}")),
                name: "Main".to_owned(),
                content: content.to_owned(),
            }],
        })
    }

    fn assigned(project_id: &str, instruction_id: &str) -> Assignment {
        Assignment {
            project_id: ProjectId::new(project_id.to_owned()),
            instruction_id: ItemId::new(instruction_id.to_owned()),
            variant_id: VariantId::new(format!("v-{instruction_id}")),
        }
    }

    fn snapshot(
        projects: Vec<SnapshotProject>,
        items: Vec<LibraryItem>,
        assignments: Vec<Assignment>,
    ) -> BuildSnapshot {
        BuildSnapshot::new(projects, items, assignments)
            .expect("the test fixture is a valid snapshot")
    }

    fn verdict(id: &str, dirty: bool) -> (String, bool) {
        (id.to_owned(), dirty)
    }

    fn change(id: &str, dirty: bool) -> ProjectDirtyChange {
        ProjectDirtyChange {
            project_id: id.to_owned(),
            dirty,
        }
    }

    /// The hash of the plain `Be brief.` body over the fixture
    /// project's target path — the value a sync would store for that
    /// build (DB-024), spelled directly through the public core seam,
    /// never through the code under test.
    fn synced_hash_of_plain_body() -> String {
        build_hash(
            Scheme::A,
            Path::new("/repos/p1/AGENTS.md"),
            off(),
            off(),
            "Be brief.",
        )
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

    // ---- the verdict computation (DB-021 through the real build) --------

    /// The stored-hash side of [`compute_dirty_verdicts`]'s input.
    fn stored(entries: &[(&str, Option<&str>)]) -> HashMap<String, Option<String>> {
        entries
            .iter()
            .map(|(id, hash)| ((*id).to_owned(), hash.map(str::to_owned)))
            .collect()
    }

    /// Computes the verdicts over a fixture snapshot — the tests
    /// speak in (id, verdict) pairs.
    fn verdicts_of(
        projects: Vec<SnapshotProject>,
        items: Vec<LibraryItem>,
        assignments: Vec<Assignment>,
        hashes: &[(&str, Option<&str>)],
    ) -> Vec<(String, bool)> {
        compute_dirty_verdicts(&snapshot(projects, items, assignments), &stored(hashes))
    }

    #[test]
    fn a_never_synced_project_with_assignments_and_a_build_is_dirty() {
        // DB-021's first clause: active, never synced (`NULL`), a
        // non-empty build — dirty. The empty-string spelling of
        // "never synced" agrees (D-068).
        for never_synced in [None, Some("")] {
            let verdicts = verdicts_of(
                vec![project("p1")],
                vec![instruction("i1", "Be brief.")],
                vec![assigned("p1", "i1")],
                &[("p1", never_synced)],
            );
            assert_eq!(
                verdicts,
                vec![verdict("p1", true)],
                "stored={never_synced:?}"
            );
        }
    }

    #[test]
    fn a_never_synced_project_without_assignments_is_not_dirty_even_with_reading_lines() {
        // DB-021's exception sentence is absolute: no assignments,
        // never synced — not counted as dirty, even though scheme A
        // with enabled reading lines builds a non-empty managed text
        // (ALG-030 joins the lines without a single assignment).
        let mut unsynced = project("p1");
        unsynced.settings.extra_local = ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        };
        unsynced.settings.extra_project = ExtraFile {
            enabled: true,
            position: ExtraFilePosition::End,
        };
        let verdicts = verdicts_of(
            vec![unsynced],
            vec![instruction("i1", "Never assigned.")],
            Vec::new(),
            &[("p1", None)],
        );
        assert_eq!(verdicts, vec![verdict("p1", false)]);
    }

    #[test]
    fn a_stored_hash_matching_the_current_build_keeps_the_project_clean() {
        // The after-sync state (DB-024 stored the hash of the build
        // the run wrote): clean.
        let verdicts = verdicts_of(
            vec![project("p1")],
            vec![instruction("i1", "Be brief.")],
            vec![assigned("p1", "i1")],
            &[("p1", Some(&synced_hash_of_plain_body()))],
        );
        assert_eq!(verdicts, vec![verdict("p1", false)]);
    }

    #[test]
    fn a_stored_hash_of_a_different_build_is_dirty() {
        // The synced-clean project, then the edit: the stored hash no
        // longer fingerprints the build — dirty (the difference
        // clause; OUT-013's warning case).
        let verdicts = verdicts_of(
            vec![project("p1")],
            vec![instruction("i1", "Be very brief.")],
            vec![assigned("p1", "i1")],
            &[("p1", Some(&synced_hash_of_plain_body()))],
        );
        assert_eq!(verdicts, vec![verdict("p1", true)]);
    }

    #[test]
    fn enabling_an_extra_file_line_makes_a_synced_project_dirty() {
        // DB-023's family through the real inputs: a settings change
        // alone (no library edit) moves the build — the reading line
        // enters the managed text and the hash follows. The stored
        // value is the hash of the same project with both extras off.
        let mut synced = project("p1");
        synced.settings.extra_local = ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        };
        let verdicts = verdicts_of(
            vec![synced],
            vec![instruction("i1", "Be brief.")],
            vec![assigned("p1", "i1")],
            &[("p1", Some(&synced_hash_of_plain_body()))],
        );
        assert_eq!(verdicts, vec![verdict("p1", true)]);
    }

    #[test]
    fn per_project_verdicts_follow_the_snapshot_order() {
        // `sort_order` in, `sort_order` out: the event's diff keeps
        // the column order.
        let verdicts = verdicts_of(
            vec![project("p1"), project("p2"), project("p3")],
            vec![instruction("i1", "Shared text.")],
            vec![assigned("p1", "i1"), assigned("p2", "i1")],
            &[("p1", None), ("p2", None), ("p3", None)],
        );
        assert_eq!(
            verdicts,
            vec![
                verdict("p1", true),
                verdict("p2", true),
                verdict("p3", false)
            ]
        );
    }

    #[test]
    fn scoped_builds_never_cross_projects_wires() {
        // The scoping's own proof: two projects pick different
        // variants of one shared instruction, and each verdict
        // follows its OWN build — p1's stored hash (spelled over the
        // `Main` body) keeps it clean exactly because its scoped
        // snapshot built that body, while p2, whose scoped build is
        // the `Compact` body the same instruction also carries, does
        // not match p1's stored value and is dirty. A scoping bug
        // that leaked p2's assignment into p1's mini-snapshot (or the
        // reverse) flips both cells.
        let shared = LibraryItem::Instruction(Instruction {
            id: ItemId::new("shared".to_owned()),
            title: "Shared".to_owned(),
            state: InstructionState::Active,
            variants: vec![
                Variant {
                    id: VariantId::new("v-main".to_owned()),
                    name: "Main".to_owned(),
                    content: "Main body.".to_owned(),
                },
                Variant {
                    id: VariantId::new("v-compact".to_owned()),
                    name: "Compact".to_owned(),
                    content: "Compact body.".to_owned(),
                },
            ],
        });
        let p1_stored = {
            // Spelled directly through the public core seam: the
            // DB-024 value a sync of p1's Main-variant build stores.
            let target = Path::new("/repos/p1/AGENTS.md");
            build_hash(Scheme::A, target, off(), off(), "Main body.")
        };
        let verdicts = verdicts_of(
            vec![project("p1"), project("p2")],
            vec![shared],
            vec![
                Assignment {
                    project_id: ProjectId::new("p1".to_owned()),
                    instruction_id: ItemId::new("shared".to_owned()),
                    variant_id: VariantId::new("v-main".to_owned()),
                },
                Assignment {
                    project_id: ProjectId::new("p2".to_owned()),
                    instruction_id: ItemId::new("shared".to_owned()),
                    variant_id: VariantId::new("v-compact".to_owned()),
                },
            ],
            &[
                ("p1", Some(p1_stored.as_str())),
                ("p2", Some(p1_stored.as_str())),
            ],
        );
        assert_eq!(
            verdicts,
            vec![verdict("p1", false), verdict("p2", true)],
            "p1 clean over its own Main build; p2 dirty over the \
             Compact build p1's stored hash does not fingerprint"
        );
    }

    // ---- the managed-text mirror ---------------------------------------

    /// The D-21 reading lines for the AGENTS stem, both positions on.
    fn both_lines() -> ExtraFileLines {
        extra_file_lines(
            BaseName::AgentsMd,
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::Start,
            },
            ExtraFile {
                enabled: true,
                position: ExtraFilePosition::End,
            },
        )
    }

    /// The no-lines value of a project with both options disabled —
    /// `ExtraFileLines` derives no `Default`, and this is the exact
    /// value the disabled options produce.
    fn no_lines() -> ExtraFileLines {
        extra_file_lines(BaseName::AgentsMd, off(), off())
    }

    const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

    /// The mirror's proof shape for the whole-file schemes:
    /// `build_content`'s real output must be exactly the mirror's
    /// string with the marker tail attached (`<managed>\n\n<marker>\n`
    /// for a non-empty managed, the marker-only file for an empty
    /// one) — drift in the join fails against the real builder.
    #[test]
    fn the_managed_mirror_is_byte_exact_for_schemes_a_and_c() {
        for (scheme, lines, body) in [
            (Scheme::A, both_lines(), "Be brief."),
            (Scheme::A, both_lines(), ""),
            (Scheme::A, no_lines(), "Be brief."),
            (Scheme::C, both_lines(), "Be brief."),
            (Scheme::C, both_lines(), ""),
        ] {
            let content = build_content(scheme, body, &lines, MARKER);
            let managed = managed_text(scheme, body, &lines);
            let expected = if managed.is_empty() {
                format!("{MARKER}\n")
            } else {
                format!("{managed}\n\n{MARKER}\n")
            };
            assert_eq!(content, expected, "scheme={scheme:?} body={body:?}");
        }
    }

    #[test]
    fn the_managed_mirror_takes_the_body_alone_for_scheme_b() {
        // B has no whole-file builder to pin against
        // (`build_content` panics on B by design, D-065); the managed
        // block is the body alone (ALG-030's "(A only)" gate,
        // `assembly_b`'s own docs), spelled here as the oracle.
        let lines = both_lines();
        assert_eq!(managed_text(Scheme::B, "Body.", &lines), "Body.");
        assert_eq!(managed_text(Scheme::B, "", &lines), "");
    }

    // ---- clean pins for every scheme and settings shape ---------------

    /// A synced-clean verdict through an independently spelled stored
    /// hash: `settings` for the hash inputs, `path` for the target,
    /// `body` for the managed text (extras off → the body IS the
    /// managed; extras on → the reading lines join it, spelled here
    /// through the real `extra_file_lines` + the §7.5 join, not
    /// through `managed_text`).
    fn clean_pin(settings: ProjectWriteSettings, path: &str, body: &str) -> Vec<(String, bool)> {
        let managed = if settings.extra_local.enabled || settings.extra_project.enabled {
            let lines = extra_file_lines(
                settings.base_name,
                settings.extra_project,
                settings.extra_local,
            );
            let mut parts: Vec<String> = Vec::new();
            if !lines.start.is_empty() {
                parts.push(lines.start.join("\n"));
            }
            parts.push(body.to_owned());
            if !lines.end.is_empty() {
                parts.push(lines.end.join("\n"));
            }
            parts.join("\n\n")
        } else {
            body.to_owned()
        };
        let stored = build_hash(
            settings.scheme,
            Path::new(path),
            settings.extra_local,
            settings.extra_project,
            &managed,
        );
        let mut synced = project("p1");
        synced.settings = settings;
        verdicts_of(
            vec![synced],
            vec![instruction("i1", body)],
            vec![assigned("p1", "i1")],
            &[("p1", Some(stored.as_str()))],
        )
    }

    #[test]
    fn scheme_b_c_and_enabled_extras_each_stay_clean_over_their_own_hash() {
        // The round-1 review's coverage hole: the clean verdict was
        // pinned for scheme A with both extras off only, leaving the
        // `extra_file_lines` argument order and the scheme-C target
        // spelling unpinned by any passing case. Each shape below
        // spells its stored hash independently and must stay clean —
        // a swapped argument order or a wrong target path moves the
        // hash and the verdict turns dirty.
        let extras =
            |local_enabled: bool,
             local_position: ExtraFilePosition,
             project_enabled: bool,
             project_position: ExtraFilePosition| ProjectWriteSettings {
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: ExtraFile {
                    enabled: local_enabled,
                    position: local_position,
                },
                extra_project: ExtraFile {
                    enabled: project_enabled,
                    position: project_position,
                },
            };
        // Scheme B: `<root>/<base_name>`, no reading lines.
        let mut b = extras(false, ExtraFilePosition::End, false, ExtraFilePosition::End);
        b.scheme = Scheme::B;
        b.base_name = BaseName::ClaudeMd;
        assert_eq!(
            clean_pin(b, "/repos/p1/CLAUDE.md", "Body."),
            vec![verdict("p1", false)]
        );
        // Scheme C: `<root>/<STEM>.project.md`, no reading lines.
        let mut c = extras(false, ExtraFilePosition::End, false, ExtraFilePosition::End);
        c.scheme = Scheme::C;
        assert_eq!(
            clean_pin(c, "/repos/p1/AGENTS.project.md", "Body."),
            vec![verdict("p1", false)]
        );
        // Scheme A with both reading lines on, `.project.md` at start
        // and `.local.md` at end: the extras' positions and the
        // project-then-local argument order are fingerprinted by the
        // stored value.
        let a_full = extras(
            false,
            ExtraFilePosition::End,
            true,
            ExtraFilePosition::Start,
        );
        assert_eq!(
            clean_pin(a_full, "/repos/p1/AGENTS.md", "Body."),
            vec![verdict("p1", false)]
        );
    }

    #[test]
    fn a_partial_last_chunk_keeps_every_project_in_order() {
        // The release-suite catch (D-121): the first chunked version
        // dropped the last partial chunk; 26 projects = one full
        // chunk of 25 + one project in the second. Every id must come
        // back, in `sort_order`, with the per-project verdicts the
        // fixture decides (even-indexed projects assigned, odd ones
        // not).
        let ids: Vec<String> = (0..26).map(|index| format!("p{index:02}")).collect();
        let projects: Vec<SnapshotProject> = (0..26).map(|index| project(&ids[index])).collect();
        let assignments: Vec<Assignment> = (0..26)
            .filter(|index| index % 2 == 0)
            .map(|index| assigned(&ids[index], "i1"))
            .collect();
        let hashes: Vec<(&str, Option<&str>)> = ids.iter().map(|id| (id.as_str(), None)).collect();
        let verdicts = verdicts_of(
            projects,
            vec![instruction("i1", "Shared.")],
            assignments,
            &hashes,
        );
        let expected: Vec<(String, bool)> = (0..26)
            .map(|index| verdict(&ids[index], index % 2 == 0))
            .collect();
        assert_eq!(verdicts, expected);
    }
}
