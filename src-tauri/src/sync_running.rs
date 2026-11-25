//! The global `sync_running` flag (spec §5.4 SYS-004/MAIN-030, §9.4
//! phase 0 step 1 and phase 4 step 5, §10's `E_SYNC_RUNNING` row;
//! T-105): the one atomic "a sync is running" fact the whole app
//! shares, taken by the sync pipeline at its start and released at
//! its finish.
//!
//! **The acquire seam (§9.4 phase 0).** A sync run's first act is
//! [`SyncRunning::try_acquire`] — an atomic compare-and-swap, so two
//! starts racing for the flag are decided by the hardware, never by
//! ordering luck: exactly one gets [`SyncGuard`], the other gets the
//! typed [`AlreadyRunning`] refusal (SYS-004: "the global
//! `sync_running` flag is taken atomically; a second start returns
//! the `E_SYNC_RUNNING` error"). The refusal is this module's own
//! vocabulary — a marker, not a wire value: converting it to the §7.10
//! `AppError` envelope happens at the command boundary
//! (`commands::sync_running_refusal`), the same
//! typed-outcome-then-envelope split `project_add_manual` uses for
//! its non-add answers (D-103).
//!
//! **The release is RAII (§9.4 phase 4 step 5).** The guard owns an
//! `Arc` to the flag and frees it in `Drop` — a panic, an early
//! `?`-return or a cancelled task unwinds through the same drop, so
//! the flag can never stay "running" behind a dead pipeline
//! (principle: make operations idempotent — a crashed run leaves
//! behind nothing the next run must recognize). This is the
//! `availability.rs` in-flight-slot shape (D-112) pointed at the
//! opposite problem: that slot coalesces redundant passes, this flag
//! excludes a second run.
//!
//! **Who acquires, who only asks.** Only the sync pipeline (T-112+,
//! the `sync_run` command) ever holds the guard — the flag means "a
//! sync is running", so holding it is the sync's alone. The
//! project-*changing* commands (SYS-005: today `project_add_manual`,
//! `exclusion_remove`, `projects_reorder`, `scan_commit` (it creates
//! project rows), `project_archive` (T-108), `project_restore`
//! (T-109), `project_update` (T-106) and `project_relink` (T-110) —
//! the mutation commands that exist; the spec's `project_remove` and
//! P1 `backup_restore` join
//! the same gate as they land) never take the flag: they **check** it through
//! [`SyncRunning::is_running`] via the `commands::reject_during_sync`
//! gate and answer the typed `E_SYNC_RUNNING` refusal while it is up.
//! Check, not hold, is deliberate: a mutation that held the guard
//! would make `is_running` lie ("a sync is running" while a reorder
//! commits) and would block a sync *start* behind an edit — SYS-005's
//! rule is one-directional ("commands … return `E_SYNC_RUNNING`
//! during a sync"), and the mutation's own atomicity is already the
//! D-037 one-transaction executor, which serializes it against the
//! sync's snapshot read on the one connection. A command that begins
//! just before a sync starts therefore commits cleanly on either side
//! of the snapshot — SYS-005's "edits … do not go into the current
//! run and stay dirty" is the dirty-recalculation service's half
//! (D-121), not this flag's. **T-112 requirement this argument
//! rests on** (recorded in D-123): the pipeline's §9.4 phase-0
//! snapshot is ONE `Db::run` read transaction on the shared
//! connection — a snapshot taken any other way would void the
//! serialization claim above.
//!
//! **Reads are never gated** (`projects_list`, `exclusions_list`, the
//! settings and history commands): SYS-005 names the changing
//! commands only, and a UI frozen mid-sync with no data to show would
//! serve nothing — the spec's screen keeps rendering from the
//! snapshot-era database.
//!
//! **Recovery mode** (no database, the T-024 state) is honest the
//! house way: the state is managed unconditionally by the `setup`
//! hook before anything that could fail (the `SessionPaths` shape),
//! so the gate finds it in every mode; with no database no sync can
//! run, the flag stays down, and the mutation commands keep their
//! honest `NoDatabase` answers — the gate does not reinterpret
//! recovery mode.
//!
//! `begin_sync` (commands.rs) consumes [`SyncRunning::try_acquire`]:
//! it is what the pipeline's phase 0 step 1 calls (T-118/D-138).

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// The global sync-running flag as managed state — one per app,
/// managed by the `setup` hook before anything that could fail.
/// Deliberately not `Default`: the constructor names the fresh state
/// ("no sync is running") in the caller's setup code.
pub(crate) struct SyncRunning {
    /// The shared flag; `Arc` so the guard owns a share for its
    /// lifetime — across `.await`s and task boundaries included
    /// (`Send + Sync` by construction, the `RecheckSlotGuard`
    /// reasoning).
    flag: Arc<SyncFlag>,
}

/// The atomic behind [`SyncRunning`] — `false`: no sync is running;
/// `true`: one holds the flag from its `try_acquire` to its guard's
/// `Drop`.
#[derive(Debug)]
struct SyncFlag {
    running: AtomicBool,
}

impl SyncRunning {
    /// The free flag a fresh launch starts from (§9.4: "recovery
    /// after a crash" never sees a stale `true` — the flag lives in
    /// process memory, and a dead process left no flag behind).
    pub(crate) fn new() -> Self {
        Self {
            flag: Arc::new(SyncFlag {
                running: AtomicBool::new(false),
            }),
        }
    }

    /// Atomically takes the flag for a sync run (§9.4 phase 0 step
    /// 1): `Ok` carries the [`SyncGuard`] that holds the flag until
    /// dropped — the pipeline's RAII lease on "a sync is running" —
    /// and `Err` is the typed SYS-004 refusal for a second start
    /// while one runs. The atomicity of the `compare_exchange` IS the
    /// exclusion (the conservative Acquire/Release pair around the
    /// flag, the `availability.rs` slot orderings: no other memory is
    /// published through it).
    pub(crate) fn try_acquire(&self) -> Result<SyncGuard, AlreadyRunning> {
        self.flag
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| SyncGuard {
                flag: Arc::clone(&self.flag),
            })
            .map_err(|_| AlreadyRunning)
    }

    /// Whether a sync currently holds the flag — the read-side status
    /// the project-changing commands' gate (SYS-005) and the tests'
    /// observability share. A plain load: asking never takes.
    pub(crate) fn is_running(&self) -> bool {
        self.flag.running.load(Ordering::Acquire)
    }
}

/// The sync run's lease on the global flag: dropped at §9.4 phase 4
/// step 5 ("Reset `sync_running`") — or earlier by an error path, a
/// panic (the unwind runs `Drop`) or a cancelled task — and the flag
/// is free the moment it goes. `Send + Sync` by construction (it owns
/// an `Arc`), so the pipeline may hold it across `.await`s for the
/// whole run, snapshot to summary.
#[must_use = "dropping the guard releases the sync flag immediately — a sync must keep it"]
#[derive(Debug)]
pub(crate) struct SyncGuard {
    flag: Arc<SyncFlag>,
}

impl Drop for SyncGuard {
    fn drop(&mut self) {
        // Release pairs with the acquire-side loads and the CAS: the
        // pipeline's own writes around the flag land before the flag
        // reads free.
        self.flag.running.store(false, Ordering::Release);
    }
}

/// The typed refusal of a second start (SYS-004): a marker, not a
/// wire value — the command boundary renders the §7.10
/// `E_SYNC_RUNNING` envelope from it (`commands::
/// sync_running_refusal`), so the service stays free of the IPC
/// vocabulary the way the store's typed outcomes are.
#[derive(Debug)]
pub(crate) struct AlreadyRunning;

#[cfg(test)]
mod tests {
    //! Behavior tests of the flag seam itself, through the public
    //! surface the sync pipeline (T-112+) and the command gate use:
    //! take-once atomicity, RAII release (drop and panic), the
    //! parallel race, and the status observable. The mock-runtime
    //! halves (commands refusing under the flag) live in
    //! `ipc_tests.rs`, which owns the mock app harness.

    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;

    #[test]
    fn the_flag_is_taken_once_and_freed_by_the_guard() {
        // The card's criterion at the seam: the first start takes the
        // flag, a second start while it is held is refused, and the
        // refusal clears the moment the guard drops — the same
        // take/hold/release triple the availability slot pins.
        let state = SyncRunning::new();
        assert!(!state.is_running(), "a fresh app is not running a sync");

        let guard = state
            .try_acquire()
            .expect("the first start must take the flag");
        assert!(state.is_running(), "the flag is up while the guard lives");
        assert!(
            state.try_acquire().is_err(),
            "a second start is refused while the first run holds the flag"
        );

        drop(guard);
        assert!(
            !state.is_running(),
            "dropping the guard resets the flag (§9.4 phase 4 step 5)"
        );
        assert!(
            state.try_acquire().is_ok(),
            "the freed flag is takeable by the next run"
        );
    }

    #[test]
    fn a_panicking_run_releases_the_flag() {
        // The RAII half of "a panic does not leave the flag hanging":
        // the unwind drops the guard, so the next start is not locked
        // out behind a dead run — the recovery guarantee that makes
        // the crash-after-phase-0 window harmless.
        let state = SyncRunning::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = state.try_acquire().expect("the flag is free");
            panic!("the pipeline died mid-run");
        }));
        assert!(result.is_err(), "the scenario itself must have panicked");
        assert!(!state.is_running(), "the unwinding run released the flag");
        assert!(
            state.try_acquire().is_ok(),
            "the next start takes the flag the dead run left behind"
        );
    }

    #[test]
    fn parallel_starts_elect_exactly_one_winner() {
        // SYS-004's "taken atomically" under real parallelism: several
        // starts racing for the flag — exactly one guard comes out,
        // the rest refusals, whichever thread wins. Which start wins
        // is scheduler luck; the count is not. Two barriers make the
        // overlap deterministic instead of sleep-based (a fixed pause
        // would let a late thread acquire AFTER the winner dropped
        // and flake the count on a loaded runner): every thread
        // attempts only between the two barriers, and the winner
        // cannot release the flag before all of them have attempted.
        const STARTS: usize = 8;
        let state = Arc::new(SyncRunning::new());
        let start_line = std::sync::Barrier::new(STARTS);
        let attempted_line = std::sync::Barrier::new(STARTS);
        let winners = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..STARTS {
                let state = Arc::clone(&state);
                let start_line = &start_line;
                let attempted_line = &attempted_line;
                let winners = Arc::clone(&winners);
                scope.spawn(move || {
                    start_line.wait();
                    // The guard binding (NOT an `if let` — its body
                    // scope would drop the winner's guard before the
                    // barrier and let a laggard take the freed flag,
                    // the exact temporary-drop trap the availability
                    // slot's round-1 review recorded) lives to the
                    // explicit drop below.
                    let guard = state.try_acquire();
                    if guard.is_ok() {
                        winners.fetch_add(1, Ordering::AcqRel);
                    }
                    // Everyone — the winner included — meets here
                    // before any guard drops, so every attempt above
                    // raced a held flag.
                    attempted_line.wait();
                    drop(guard);
                });
            }
        });
        assert_eq!(
            winners.load(Ordering::Acquire),
            1,
            "exactly one of the {STARTS} parallel starts may hold the flag"
        );
        assert!(
            !state.is_running(),
            "the last guard's drop left the flag free"
        );
    }
}
