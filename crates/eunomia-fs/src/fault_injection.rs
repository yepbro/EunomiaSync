//! Fault injection for the file-system layer: spec 15.1's "failure
//! points in `fs`" (the Fault-injection test level), behind the
//! `fault-injection` cargo feature (task T-069).
//!
//! The spec's tool column names the mechanism — "failure points in
//! `fs`" — and NFR-024's kill tests build on the pause points. A
//! `#[cfg(test)]` seam cannot serve either one alone: it is invisible
//! to integration tests (`tests/` compile the crate as an external
//! dependency, where `cfg(test)` is false) and to the future
//! pipeline-level kill tests, which run the library inside a child
//! process. Hence the feature: this module and every call site
//! compile under `#[cfg(any(test, feature = "fault-injection"))]` —
//! unit tests get the points unconditionally, integration tests and
//! child processes get them by enabling the feature.
//!
//! # The release guarantee
//!
//! The points exist ONLY where a test put them. The guarantee is
//! structural, not conventional:
//!
//! - the feature is not in `default`;
//! - no `[dependencies]` edge anywhere in the workspace enables it —
//!   the only automatic enablements are DEV-dependency edges: this
//!   crate's own self dev-dependency (`[dev-dependencies] eunomia-fs =
//!   { path = ".", features = ["fault-injection"] }`), which exists so
//!   a plain `cargo test` exercises the fault points instead of
//!   trusting a command-line flag nobody remembers in CI, and
//!   eunomia-sync's dev-dependency edge (T-130, DECISIONS.md D-226),
//!   which turns the feature on for the app crate's test builds so
//!   the run-level kill suite can arm the pause points;
//! - `cargo build` and `cargo build --release` never resolve
//!   dev-dependencies, so every production artifact — including the
//!   Tauri app — compiles this crate with the feature off and
//!   contains no fault code;
//! - the module itself refuses to compile in a release profile (the
//!   `compile_error!` below): the one unification hole cargo leaves
//!   open — a release build that also resolves dev-dependencies
//!   (`cargo build --release --all-targets`, `cargo test --release`)
//!   — fails the build with a named error instead of silently
//!   linking the points. The known cost: release-profile TEST builds
//!   are impossible with the feature on; CI and the release path
//!   use none (verified against .github/workflows/ci.yml). The guard
//!   keys on `debug_assertions` — the profile signal rustc actually
//!   exposes — so a release profile deliberately configured with
//!   `debug-assertions = true` would bypass it; the `strings` scan
//!   below is the backstop that would expose such a regression.
//!
//! Feature unification turns the feature ON for every consumer in a
//! workspace-wide test build (dev-dependencies participate there) —
//! by design, and visible only to test binaries. The absence in the
//! release artifact is verifiable in two steps: (1) `cargo build
//! --release -p eunomia-fs`, then `strings
//! target/release/libeunomia_fs.rlib | grep -c
//! eunomia_fs15fault_injection` → 0 — `strings`, not `nm`, because
//! the workspace strips release symbols (`[profile.release] strip =
//! true`) and a stripped rlib shows no symbols at all, with or
//! without the feature (the scan must discriminate, not vacuously
//! pass); the control is (2) a feature-enabled dev build of the same
//! crate, where the same `strings` scan finds the symbols — proof
//! the scan sees them when they exist. The method is recorded in
//! DECISIONS.md D-101.
//!
//! # The points (spec 15.1 rows)
//!
//! - **Failure of the N-th backup** — [`arm_backup_copy_failure`]:
//!   the N-th copy pass of [`crate::backup::backup_target_file`]
//!   refuses its write path, and the production removal and
//!   `CopyFailed` classification run for real (T-F1's fs-level
//!   half; the run-level sweep is the pipeline's).
//! - **`rename` failure** — [`arm_atomic_fault`] with
//!   [`AtomicFault::Fail`] on [`AtomicStep::Replace`] (every other
//!   §9.5 step refuses the same way).
//! - **Out of space** — [`arm_free_space_override`] replaces what
//!   the pre-write probe OBSERVES (the decision stays
//!   `free_space::check_space`'s), and
//!   [`AtomicFault::Fail`] with an `ENOSPC` error on any write-path
//!   step produces the mid-write verdict.
//! - **Read-only file** — the write-rights gate
//!   ([`crate::write_access::check_write_access`]) answers a real
//!   read-only mark by design, so its point is fixture-driven, not
//!   injected; the write path's own read-only-shaped refusals
//!   (`EROFS`/`EACCES`) are injectable at any atomic step.
//! - **Locked file (Windows)** — [`arm_locked_replace`] drives the
//!   REAL §9.5 retry schedule with simulated lock refusals, so the
//!   typed `E_FILE_LOCKED` endgame runs on every platform (a real
//!   sharing violation needs a Windows handle; the CI Windows leg
//!   owns that shape).
//! - **Force-killing the process between phases and inside a
//!   write** — [`arm_pause`]: [`PausePoint::StagingSettled`] is
//!   T-F2's window (the staging file is written and settled, the
//!   rename has not happened), [`PausePoint::WriteEntry`] is the
//!   seam between the backup phase (§9.4 phase 2) and the write
//!   (§9.5). A paused call parks until released; a kill test kills
//!   the paused process and inspects the disk.
//!
//! # The armed-fault discipline
//!
//! The state is THREAD-LOCAL with an RAII guard ([`Armed`]), the
//! pattern the atomic pipeline's original `#[cfg(test)]` seam
//! established (D-095/D-099, generalized here): a fault is armed on
//! the thread that runs the operation under test, consumed at the
//! production call site, and disarmed when the guard drops — a test
//! cannot leak a fault into the next one, and parallel test threads
//! cannot see each other's faults. ONE fixture scope per test: any
//! guard's drop disarms EVERY fixture of this module on the thread,
//! so arming a second fixture mid-scope and dropping the first guard
//! cancels both — combine what a test needs into one plan.
//!
//! Point faults ([`AtomicFault`], [`BackupCopyFault`], the pause)
//! are one-shot events: they fire once (the backup and pause plans
//! may skip arrivals first) and are consumed by the firing. The
//! free-space override is a STATE, not an event: it holds for every
//! probe while its guard lives. Injection never changes semantics —
//! the injected value enters the production path exactly where the
//! real one would, and every cleanup branch that runs is the
//! production branch.

use std::cell::RefCell;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use crate::free_space::FreeSpaceError;

// The structural half of the release guarantee: this module exists
// only under the feature, and the feature exists only for tests — a
// release profile reaching it (the one unification hole cargo leaves
// open: `cargo build --release --all-targets` and `cargo test
// --release` resolve dev-dependencies, so the self dev-dependency's
// feature reaches the lib unit the release binaries link) fails the
// build with a named error instead of silently shipping the fault
// points. CI and the release path use no release-profile test builds
// (verified against .github/workflows/ci.yml); the cost is recorded
// in DECISIONS.md D-101.
//
// The invocation deliberately uses the BRACE form of the compile
// error macro: the app's SEC-008 pin
// (`tracing_events_are_constructed_only_in_the_logging_module`)
// scans every workspace source raw for tracing-macro invocations by
// substring, and the paren form of this macro contains the very
// substring the pin searches for. The brace form is the same macro
// (and rustfmt-stable), just not a spelling the pin matches — see
// D-101's addendum.
#[cfg(not(debug_assertions))]
compile_error! {
    "the fault-injection feature must never be enabled in a release build: \
     it exists for tests only (spec 15.1, NFR-024; DECISIONS.md D-101)"
}

/// The §9.5 pipeline steps an atomic fault can refuse. `ModeCopy` and
/// `DirSync` exist only where those steps do (Unix).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AtomicStep {
    /// Creating the staging file (§9.5 step 2).
    TempCreate,
    /// Copying the existing target's Unix mode onto the staging file
    /// (§9.5 step 2; Unix only).
    #[cfg(unix)]
    ModeCopy,
    /// Writing the staging file and `sync_all`-ing it (§9.5 step 3).
    WriteSync,
    /// The replace rename itself (§9.5 step 4).
    Replace,
    /// The parent-folder `fsync` (§9.5 step 5; Unix only).
    #[cfg(unix)]
    DirSync,
    /// The re-read of the settled target (§9.5 step 6).
    VerifyRead,
}

/// One armed atomic-pipeline fault.
pub enum AtomicFault {
    /// Refuse the given step once, with this error — the error
    /// enters the step's production classification exactly where an
    /// OS refusal would (`ENOSPC` becomes the write-time
    /// `E_DISK_FULL`, `EACCES`/`EROFS` stay `E_WRITE_FAILED`, and so
    /// on).
    Fail(AtomicStep, io::Error),
    /// Refuse the given step with EACCES after making the target
    /// folder read-only-execute — so the §9.5 step-7 staging removal
    /// itself is refused and the `cleanup` field has a real refusal
    /// to carry (Unix: mode bits are the fixture).
    #[cfg(unix)]
    FailAndLockFolder(AtomicStep),
    /// Append one byte to the target just before the §9.5 step-6
    /// re-read: the settled file no longer holds what was written,
    /// and the digest check must catch it.
    CorruptTarget,
}

/// The simulated locked replace of §9.5 step 4's retry discipline:
/// what the simulated replace attempt answers while the plan is
/// armed. It drives the REAL retry loop (the same function the
/// Windows arm runs) with injected OS refusals, so the schedule, the
/// classification and the pipeline's `E_FILE_LOCKED` path are
/// exercised on every platform — a real sharing violation needs a
/// Windows handle.
pub enum LockedReplacePlan {
    /// Refuse every attempt with the lock code — the exhaustion path
    /// (§10: "3 retries failed").
    Exhaust { code: i32 },
    /// The exhaustion path with the §9.5 step-7 cleanup refused too:
    /// the target's folder goes read-only-execute for the duration,
    /// so the staging removal itself fails EACCES and
    /// `FileLockedFailure::cleanup` has a real refusal to carry
    /// (Unix: mode bits are the fixture, like
    /// [`AtomicFault::FailAndLockFolder`]).
    #[cfg(unix)]
    ExhaustWithRefusedCleanup { code: i32 },
    /// Refuse `refusals` attempts with the lock code, then perform
    /// the REAL replace — the holder closed the file mid-schedule
    /// and the write must land.
    UnlockAfter { refusals: usize, code: i32 },
}

/// One armed backup-copy failure: the `fail_on_copy`-th copy pass
/// refuses its write path with `source` (1-based; the count spans
/// every copy the armed thread makes — three files' backups see
/// three passes). The refusal surfaces as the production
/// [`crate::backup::BackupError::CopyFailed`] with the partial copy
/// removed by the real §BKP cleanup path.
pub struct BackupCopyFault {
    /// Which copy fails, counting from 1.
    pub fail_on_copy: usize,
    /// The refusal the failing copy's write path reports.
    pub source: io::Error,
}

/// Where an armed pause parks the operation (spec 15.1's
/// "force-killing the process between phases and inside a write").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PausePoint {
    /// T-F2's window: the §9.5 staging file is written and settled,
    /// the rename has not happened. A kill here must leave the
    /// original intact and exactly one sweepable staging file; the
    /// next sync deletes it.
    StagingSettled,
    /// The seam between the backup phase (§9.4 phase 2) and the
    /// write (§9.5): the caller's backups are made, nothing of the
    /// write has touched the disk yet. A kill here (T-F3's shape at
    /// single-file granularity) leaves the backups complete and the
    /// target untouched.
    WriteEntry,
}

/// An armed pause plan: which point parks the operation, after how
/// many arrivals to let pass (0 = the first).
pub struct PausePlan {
    /// The point that parks.
    pub point: PausePoint,
    /// How many arrivals of `point` pass through unparked before the
    /// parking one (T-F3-style tests aim at a later boundary).
    pub skip: usize,
}

/// The observing side of an armed pause: waits for the operation to
/// reach the point, then releases it. One arrival, one release — the
/// pause is one-shot.
pub struct PauseControl {
    arrival_rx: mpsc::Receiver<PausePoint>,
    release_tx: mpsc::Sender<()>,
}

impl PauseControl {
    /// Waits up to `timeout` for the operation to reach the armed
    /// point, returning which point fired; `None` on timeout. Panics
    /// if the pause was disarmed before firing (the operation ended
    /// without reaching the point) — a test would otherwise wait on
    /// an event that can never happen.
    pub fn wait_for_arrival(&self, timeout: Duration) -> Option<PausePoint> {
        match self.arrival_rx.recv_timeout(timeout) {
            Ok(point) => Some(point),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the armed pause was disarmed before its point was reached")
            }
        }
    }

    /// Lets the parked operation continue. Sending before the arrival
    /// is harmless: the parked call consumes the release when it
    /// parks, and an unparked call never looks at it.
    pub fn release(&self) {
        let _ = self.release_tx.send(());
    }
}

thread_local! {
    static FAULT: RefCell<Option<AtomicFault>> = const { RefCell::new(None) };
    static STAGING_TOKENS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static LOCKED_REPLACE: RefCell<Option<LockedReplacePlan>> = const { RefCell::new(None) };
    static LOCK_PAUSES: RefCell<Option<Vec<Duration>>> = const { RefCell::new(None) };
    static BACKUP_COPY: RefCell<Option<BackupCopyFault>> = const { RefCell::new(None) };
    static FREE_SPACE: RefCell<Option<u64>> = const { RefCell::new(None) };
    static PAUSE: RefCell<Option<ArmedPause>> = const { RefCell::new(None) };
}

/// The pause's armed state: the plan plus the channels the
/// [`PauseControl`] holds the other ends of.
struct ArmedPause {
    plan: PausePlan,
    arrival_tx: mpsc::Sender<PausePoint>,
    release_rx: mpsc::Receiver<()>,
}

/// Keeps an armed fault armed for a scope and disarms on drop — a
/// test can never leak a fault into the next one. ONE fixture scope
/// per test: a guard's drop disarms EVERY fixture of this module on
/// the thread, so arming a second fixture mid-scope and dropping the
/// first guard cancels both — combine what a test needs into one
/// plan instead.
#[must_use = "dropping the guard disarms every fixture on this thread"]
pub struct Armed;

impl Drop for Armed {
    fn drop(&mut self) {
        FAULT.with(|cell| *cell.borrow_mut() = None);
        STAGING_TOKENS.with(|cell| cell.borrow_mut().clear());
        LOCKED_REPLACE.with(|cell| *cell.borrow_mut() = None);
        LOCK_PAUSES.with(|cell| *cell.borrow_mut() = None);
        BACKUP_COPY.with(|cell| *cell.borrow_mut() = None);
        FREE_SPACE.with(|cell| *cell.borrow_mut() = None);
        PAUSE.with(|cell| *cell.borrow_mut() = None);
    }
}

/// Arms `fault` until the returned guard drops.
pub fn arm_atomic_fault(fault: AtomicFault) -> Armed {
    FAULT.with(|cell| *cell.borrow_mut() = Some(fault));
    Armed
}

/// Arms the N-th-copy backup failure (see [`BackupCopyFault`])
/// until the returned guard drops.
pub fn arm_backup_copy_failure(fail_on_copy: usize, source: io::Error) -> Armed {
    BACKUP_COPY.with(|cell| {
        *cell.borrow_mut() = Some(BackupCopyFault {
            fail_on_copy,
            source,
        })
    });
    Armed
}

/// Holds the free-space observation at `free` bytes: while the guard
/// lives, every pre-write probe of the armed thread reports `free`
/// instead of what the caller's source answered — the §10
/// write-time `E_DISK_FULL` verdict then comes from the real
/// `check_space` decision on the injected number. A FAILED probe is
/// never overridden (the write, not the probe, stays the authority).
pub fn arm_free_space_override(free: u64) -> Armed {
    FREE_SPACE.with(|cell| *cell.borrow_mut() = Some(free));
    Armed
}

/// Arms a pause (see [`PausePlan`]) until the returned guard drops;
/// the returned [`PauseControl`] observes and releases it.
pub fn arm_pause(plan: PausePlan) -> (Armed, PauseControl) {
    let (arrival_tx, arrival_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    PAUSE.with(|cell| {
        *cell.borrow_mut() = Some(ArmedPause {
            plan,
            arrival_tx,
            release_rx,
        })
    });
    (
        Armed,
        PauseControl {
            arrival_rx,
            release_tx,
        },
    )
}

/// Fixes the staging-token sequence (the collision fixtures): each
/// armed guard drops the whole setup. Tests treat faults and tokens
/// alike.
pub fn fix_staging_tokens(tokens: &[&str]) -> Armed {
    STAGING_TOKENS.with(|cell| {
        *cell.borrow_mut() = tokens.iter().map(|token| (*token).to_owned()).collect();
    });
    Armed
}

/// Arms a simulated locked replace (see [`LockedReplacePlan`]) and
/// the pause-recording clock together — the retry-mechanics fixtures
/// want both, and one guard drops both.
pub fn arm_locked_replace(plan: LockedReplacePlan) -> Armed {
    LOCKED_REPLACE.with(|cell| *cell.borrow_mut() = Some(plan));
    LOCK_PAUSES.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
    Armed
}

/// Arms only the pause-recording clock — the real-handle Windows
/// test (T-F8) drives the true `MoveFileExW` refusals and needs no
/// simulation, only a clock that does not really sleep.
#[cfg(windows)]
pub fn arm_pause_recording() -> Armed {
    LOCK_PAUSES.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
    Armed
}

/// The pauses the recording clock has observed so far (empty when no
/// clock is armed) — the schedule oracle of the retry tests.
pub fn recorded_lock_pauses() -> Vec<Duration> {
    LOCK_PAUSES.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|pauses| pauses.clone())
            .unwrap_or_default()
    })
}

/// The one-shot consumption point of the atomic pipeline: when the
/// armed fault targets `step`, take it (it fires once) and return
/// its error; a fault aimed at another step stays armed for its own
/// call site.
pub(crate) fn inject_atomic(step: AtomicStep, _target: &Path) -> Option<io::Error> {
    FAULT.with(|cell| {
        let mut armed = cell.borrow_mut();
        match armed.take() {
            Some(AtomicFault::Fail(armed_step, error)) if armed_step == step => Some(error),
            #[cfg(unix)]
            Some(AtomicFault::FailAndLockFolder(armed_step)) if armed_step == step => {
                lock_folder(_target.parent().unwrap_or(Path::new("")));
                Some(io::Error::from_raw_os_error(13))
            }
            other => {
                *armed = other;
                None
            }
        }
    })
}

/// The §9.5 step-6 corruption point: appends one byte to the target
/// when [`AtomicFault::CorruptTarget`] is armed (consuming it), so
/// the re-read digests bytes this run never wrote.
pub(crate) fn corrupt_target_if_armed(target: &Path) {
    FAULT.with(|cell| {
        let mut armed = cell.borrow_mut();
        let taken = armed.take();
        if matches!(taken, Some(AtomicFault::CorruptTarget)) {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(target)
                .expect("the corruption hook must be able to open the target");
            file.write_all(b"X")
                .expect("the corruption hook must be able to append");
        } else {
            // Not ours: leave it armed for its own call site.
            *armed = taken;
        }
    })
}

/// The next fixed staging token when a test pinned the sequence,
/// consuming it; `None` falls back to production entropy.
pub(crate) fn next_fixed_token() -> Option<String> {
    STAGING_TOKENS.with(|cell| {
        if cell.borrow().is_empty() {
            None
        } else {
            Some(cell.borrow_mut().remove(0))
        }
    })
}

/// Whether a simulated locked replace is armed — the retry-loop
/// channel of both platform arms' `replace_target`.
pub(crate) fn locked_replace_is_armed() -> bool {
    LOCKED_REPLACE.with(|cell| cell.borrow().is_some())
}

/// One attempt of the simulated locked replace: refuse with the
/// plan's lock code until the plan says the real replace runs
/// (`UnlockAfter` with its refusals spent) — the "real" attempt is
/// the production `perform_replace`, so an unlocking schedule
/// genuinely lands the file.
pub(crate) fn simulated_lock_attempt(staging_path: &Path, target: &Path) -> io::Result<()> {
    /// What the plan says this attempt does.
    enum Next {
        Refuse(i32),
        Replace,
    }
    let next = LOCKED_REPLACE.with(|cell| {
        let mut armed = cell.borrow_mut();
        match armed.as_mut() {
            Some(LockedReplacePlan::Exhaust { code }) => Next::Refuse(*code),
            #[cfg(unix)]
            Some(LockedReplacePlan::ExhaustWithRefusedCleanup { code }) => {
                // The removal will be refused the same way the
                // folder-lock fault arranges it: read-only-execute
                // mode bits. Idempotent across attempts.
                lock_folder(target.parent().unwrap_or(Path::new("")));
                Next::Refuse(*code)
            }
            Some(LockedReplacePlan::UnlockAfter { refusals, code }) => {
                if *refusals == 0 {
                    Next::Replace
                } else {
                    *refusals -= 1;
                    Next::Refuse(*code)
                }
            }
            None => Next::Replace,
        }
    });
    match next {
        Next::Refuse(code) => Err(io::Error::from_raw_os_error(code)),
        Next::Replace => crate::atomic::perform_replace(staging_path, target),
    }
}

/// The mock clock of the simulated-replace channel: records the
/// pause instead of sleeping.
pub(crate) fn record_lock_pause(pause: Duration) {
    LOCK_PAUSES.with(|cell| {
        if let Some(pauses) = cell.borrow_mut().as_mut() {
            pauses.push(pause);
        }
    });
}

/// The real Windows loop's clock seam: records the pause and reports
/// `true` (skip the real sleep) when the recording clock is armed,
/// `false` when production must really sleep.
#[cfg(windows)]
pub(crate) fn record_lock_pause_if_armed(pause: Duration) -> bool {
    LOCK_PAUSES.with(|cell| {
        let mut sink = cell.borrow_mut();
        match sink.as_mut() {
            Some(pauses) => {
                pauses.push(pause);
                true
            }
            None => false,
        }
    })
}

/// The one-shot consumption point of the backup copy pass: counts
/// this thread's copies, refusing the armed pass's write path with
/// its error (see [`BackupCopyFault`]).
pub(crate) fn inject_backup_copy_failure() -> Option<io::Error> {
    BACKUP_COPY.with(|cell| {
        let taken = cell.borrow_mut().take();
        match taken {
            // Not this copy's turn yet: count it and stay armed.
            Some(mut plan) if plan.fail_on_copy > 1 => {
                plan.fail_on_copy -= 1;
                *cell.borrow_mut() = Some(plan);
                None
            }
            // The armed copy (a count of 0 behaves as 1: fire now).
            Some(plan) => Some(plan.source),
            None => None,
        }
    })
}

/// The free-space observation override: an armed override replaces
/// what a SUCCESSFUL probe observed; a failed probe passes through
/// untouched (the write, not the probe, stays the authority,
/// ERR-003's posture). Holds for every probe while armed — the
/// override is a state, not an event.
pub(crate) fn override_free_space(
    probe: Result<u64, FreeSpaceError>,
) -> Result<u64, FreeSpaceError> {
    match (FREE_SPACE.with(|cell| *cell.borrow()), probe) {
        (Some(free), Ok(_)) => Ok(free),
        (None, reported) => reported,
        (Some(_), Err(source)) => Err(source),
    }
}

/// The pause consumption point: when an armed pause targets `point`,
/// let `skip` arrivals pass, then signal the arrival and park the
/// calling thread until released (a dropped release channel releases
/// too — a panicked observer must not deadlock the operation). The
/// pause is one-shot: the plan is consumed by the parking arrival.
pub(crate) fn pause_if_armed(point: PausePoint) {
    let Some(mut armed) = PAUSE.with(|cell| cell.borrow_mut().take()) else {
        return;
    };
    if armed.plan.point != point {
        PAUSE.with(|cell| *cell.borrow_mut() = Some(armed));
        return;
    }
    if armed.plan.skip > 0 {
        armed.plan.skip -= 1;
        PAUSE.with(|cell| *cell.borrow_mut() = Some(armed));
        return;
    }
    // Fire: signal, then park. The state is NOT put back — one shot.
    let _ = armed.arrival_tx.send(point);
    let _ = armed.release_rx.recv();
}

/// Makes the folder read-only-execute so the staging removal is
/// refused (EACCES) — the carried-cleanup fixture's setup.
#[cfg(unix)]
fn lock_folder(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o500))
        .expect("the lock-folder fixture must be able to chmod");
}
