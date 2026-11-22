//! Integration tests for the spec 15.1 fault-injection points
//! (task T-069): each point is armed through the public
//! `fault_injection` API from OUTSIDE the crate — the way the
//! pipeline-level tests of later tasks and the NFR-024 kill tests
//! will drive them — and the observable behavior is asserted through
//! the production entry points:
//!
//! * the N-th backup copy failure (T-F1's fs-level half): only the
//!   armed copy refuses, the partial bytes are removed by the real
//!   cleanup, the copies before and after land;
//! * the replace failure (15.1's "`rename` failure"): the original
//!   stays whole, the staging file is cleaned, and the fault dies
//!   with its guard;
//! * the pre-write disk-full verdict (15.1's "out of space") is
//!   injected OVER the caller's own plentiful probe — the shape a
//!   higher-layer test has, not owning the closure — while a FAILED
//!   probe still passes through (the write stays the authority);
//!   the mid-write `ENOSPC` shape rides the step fault;
//! * the read-only row: the write-rights gate is driven by a REAL
//!   read-only mark (it needs no injection — the mark is cheaply
//!   arrangeable and the probe is the production one), and the
//!   write path's read-only-shaped refusal (`EROFS` at the staging
//!   create) is injected;
//! * the locked replace (15.1's "locked file (Windows)"): the
//!   simulated lock drives the REAL §9.5 schedule on this Linux leg,
//!   ending typed `E_FILE_LOCKED` with the pauses recorded;
//! * the pause points (15.1's "force-killing the process between
//!   phases and inside a write", the T-F2/T-F3 windows): a parked
//!   worker thread is inspected from outside — the disk state in
//!   the window is exactly what a kill would leave — then released;
//!   the skip counter selects a later arrival, and the pause is
//!   one-shot.
//!
//! The real kill scenarios (SIGKILL against a child parked at these
//! points, NFR-024) are separate tasks: these tests pin the POINTS —
//! that they park exactly where the card says and that the disk
//! around them matches the windows' contracts.

// Not `#![cfg(feature = "fault-injection")]`: a crate-level cfg would
// silently compile this whole suite away (0 tests, exit 0) if the
// self dev-dependency that enables the feature were ever dropped or
// broken — the classic silent-skip shape. Instead the suite demands
// the feature loudly: in a healthy tree the dev-dependency edge
// always provides it, and without it the test target fails to build
// with a named error. (Brace form of the macro for the same SEC-008
// reason as in src/fault_injection.rs — see D-101's addendum.)
#[cfg(not(feature = "fault-injection"))]
compile_error! {
    "the eunomia-fs fault-injection tests require the `fault-injection` \
     feature: it arrives through the crate's self dev-dependency — if this \
     error fires, that edge was dropped or broken (T-069, DECISIONS.md D-101)"
}

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use eunomia_core::backup_name::BackupTime;
use eunomia_core::project::{BaseName, ProjectId, Scheme};
use eunomia_fs::atomic::{AtomicWriteError, WriteStep, write_target_file};
use eunomia_fs::backup::{BackupError, BackupOutcome, BackupTarget, backup_target_file};
use eunomia_fs::fault_injection::{
    self, AtomicFault, AtomicStep, LockedReplacePlan, PausePlan, PausePoint,
};
use eunomia_fs::free_space::FreeSpaceError;
use eunomia_fs::temp_files::is_app_tmp_file;
use eunomia_fs::write_access::{WriteAccessError, check_write_access};

use tempfile::TempDir;

/// How long a test waits for a worker thread's cooperation: far more
/// than a healthy run needs, so a miss means a bug, not a flake.
const WAIT: Duration = Duration::from_secs(30);

/// The FIPS 180-2 digest of "one" and friends are computed by the
/// assertions through the file contents themselves, not pinned here:
/// what the tests pin is WHICH copies exist, not their spellings.
fn plenty_free() -> impl FnOnce() -> Result<u64, FreeSpaceError> {
    || Ok(1 << 40)
}

/// A target file with `contents`, for the overwrite fixtures.
fn existing(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("writing the fixture file must work");
    path
}

/// The sorted entry names of `folder` — the "what is on disk" oracle.
fn entry_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("reading the fixture folder must work")
        .map(|entry| {
            entry
                .expect("iterating the fixture folder must work")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// A canonical project id for the backup fixtures.
fn fixture_id() -> ProjectId {
    ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned())
}

/// The injected run time (D-061: time is a parameter).
fn fixture_time() -> BackupTime {
    BackupTime {
        year: 2026,
        month: 11,
        day: 20,
        hour: 11,
        minute: 0,
        second: 0,
        millisecond: 0,
    }
}

/// The one-call backup shape every fixture uses: scheme A,
/// AGENTS.md, the fixture id/name/time, no dedup input.
fn backup_call(source: &Path, data_dir: &Path) -> Result<BackupOutcome, BackupError> {
    backup_target_file(&BackupTarget {
        source,
        data_dir,
        project_id: &fixture_id(),
        project_name: "My Project",
        scheme: Scheme::A,
        base_name: BaseName::AgentsMd,
        time: fixture_time(),
        last_backup_sha256_hex: None,
    })
}

/// Every `.bak` file in the fixture project's backup folder, as
/// contents (what a reader observes), sorted.
fn backup_contents(data_dir: &Path) -> Vec<Vec<u8>> {
    let folder = data_dir
        .join("backups")
        .join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project");
    let mut contents: Vec<Vec<u8>> = fs::read_dir(&folder)
        .expect("the backup folder must exist")
        .map(|entry| {
            let path = entry.expect("reading a backup entry must work").path();
            fs::read(&path).expect("reading the copy must work")
        })
        .collect();
    contents.sort();
    contents
}

// ---- the N-th backup copy failure (spec 15.1, T-F1's fs half) ----

#[test]
fn the_nth_backup_copy_failure_hits_only_that_copy() {
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let data_dir = dir.path().join("data");
    fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
    let source = existing(&data_dir, "AGENTS.md", b"one");

    // EIO: an environmental refusal no fixture can arrange portably.
    let guard = fault_injection::arm_backup_copy_failure(2, io::Error::from_raw_os_error(5));

    backup_call(&source, &data_dir).expect("the first copy must be created");
    fs::write(&source, b"two").expect("rewriting the source must work");
    let err = backup_call(&source, &data_dir).expect_err("the second copy must refuse to write");

    let BackupError::CopyFailed {
        path,
        source: refusal,
        cleanup,
    } = &err
    else {
        panic!("the injected refusal must be CopyFailed, got: {err:?}");
    };
    assert_eq!(
        refusal.raw_os_error(),
        Some(5),
        "the injected OS code rides"
    );
    assert!(
        cleanup.is_none(),
        "the partial copy was removed by the production cleanup"
    );
    assert!(!path.exists(), "no partial bytes linger under the name");

    fs::write(&source, b"three").expect("rewriting the source must work");
    backup_call(&source, &data_dir)
        .expect("the third copy must land — the fault was consumed by the second");

    let mut contents = backup_contents(&data_dir);
    assert_eq!(
        contents,
        [b"one".to_vec(), b"three".to_vec()],
        "exactly the copies before and after the armed one exist"
    );

    // The guard's drop resets the seam: a later copy lands normally.
    drop(guard);
    fs::write(&source, b"four").expect("rewriting the source must work");
    backup_call(&source, &data_dir).expect("a copy after the guard dropped must land");
    contents = backup_contents(&data_dir);
    assert_eq!(contents.len(), 3, "no fault leaked past the guard");
}

// ---- the replace failure (spec 15.1 "rename failure") ---------------

#[test]
fn an_injected_replace_refusal_keeps_the_original_and_cleans_up() {
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = existing(dir.path(), "AGENTS.md", b"untouched");
    let guard = fault_injection::arm_atomic_fault(AtomicFault::Fail(
        AtomicStep::Replace,
        io::Error::from_raw_os_error(13),
    ));

    let err = write_target_file(&target, b"abc", plenty_free())
        .expect_err("the injected refusal must fail the write");

    let AtomicWriteError::WriteFailed(failed) = &err else {
        panic!("the refusal must be WriteFailed, got: {err:?}");
    };
    assert_eq!(failed.step, WriteStep::Replace);
    assert_eq!(err.os_code(), Some(13));
    assert!(failed.cleanup.is_none(), "the staging file was removed");
    assert_eq!(
        fs::read(&target).expect("the target must survive"),
        b"untouched"
    );
    assert_eq!(entry_names(dir.path()), ["AGENTS.md"], "no staging debris");

    drop(guard);
    write_target_file(&target, b"abc", plenty_free())
        .expect("a write after the guard dropped must land — no fault leaked");
    assert_eq!(fs::read(&target).expect("the write must land"), b"abc");
}

// ---- out of space (spec 15.1): the pre-write verdict, the mid-write --

#[test]
fn the_pre_write_disk_full_verdict_is_injectable_over_the_callers_own_probe() {
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = existing(dir.path(), "AGENTS.md", b"untouched");
    // The closure reports a plentiful volume — an integration test of
    // a HIGHER layer cannot reach into the closure the production
    // caller passes; the override stands in for the full volume.
    let _guard = fault_injection::arm_free_space_override(2);

    let err = write_target_file(&target, b"abc", plenty_free())
        .expect_err("two free bytes against three demanded must refuse");

    let AtomicWriteError::DiskFull(failed) = &err else {
        panic!("the shortage verdict must be DiskFull, got: {err:?}");
    };
    assert_eq!(failed.free, Some(2), "the injected observation is the fact");
    assert_eq!(failed.required, 3);
    assert_eq!(failed.step, None, "no step had run yet");
    assert_eq!(err.os_code(), None, "a decision, not an OS failure");
    assert_eq!(
        fs::read(&target).expect("the target must survive"),
        b"untouched"
    );
    assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);

    // The override is a STATE, not a one-shot event: under the same
    // guard a second write's probe reports the same number (a
    // one-shot consumption would let this write through).
    let err = write_target_file(&target, b"abc", plenty_free())
        .expect_err("the second probe under the same guard must see the same shortage");
    assert!(
        matches!(err, AtomicWriteError::DiskFull(_) if err.os_code().is_none()),
        "the second verdict is the same pre-write shortage: {err:?}"
    );
}

#[test]
fn the_override_never_fabricates_a_shortage_from_a_failed_probe() {
    // ERR-003's posture under injection: the override replaces what a
    // SUCCESSFUL probe observed; a failed probe still passes through
    // and must not block the write — the write stays the authority.
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = dir.path().join("AGENTS.md");
    let _guard = fault_injection::arm_free_space_override(0);
    let probe = || {
        Err(FreeSpaceError::ProbeFailed {
            source: io::Error::from_raw_os_error(2),
        })
    };

    write_target_file(&target, b"abc", probe)
        .expect("a failed probe must not be overridden into a shortage");
    assert_eq!(fs::read(&target).expect("the file must exist"), b"abc");
}

#[test]
fn mid_write_enospc_is_injectable_as_disk_full() {
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = existing(dir.path(), "AGENTS.md", b"untouched");
    // The OS no-space refusal std maps to ErrorKind::StorageFull — the
    // mid-write verdict the module classifies by: ENOSPC (28) on Unix,
    // ERROR_DISK_FULL (112) on Windows (CI 44864444674: a raw 28 on
    // Windows is ERROR_OUT_OF_PAPER, which is correctly NOT DiskFull).
    #[cfg(unix)]
    let no_space = io::Error::from_raw_os_error(28);
    #[cfg(windows)]
    let no_space = io::Error::from_raw_os_error(112);
    #[cfg(unix)]
    let no_space_code = no_space
        .raw_os_error()
        .expect("a raw no-space code was just set");
    #[cfg(windows)]
    let no_space_code = 112;
    let _guard =
        fault_injection::arm_atomic_fault(AtomicFault::Fail(AtomicStep::WriteSync, no_space));

    let err = write_target_file(&target, b"abc", plenty_free())
        .expect_err("a no-space refusal must fail the write");

    let AtomicWriteError::DiskFull(failed) = &err else {
        panic!("the no-space refusal must be classified DiskFull, got: {err:?}");
    };
    assert_eq!(failed.step, Some(WriteStep::WriteSync));
    assert_eq!(failed.free, None, "no probe number exists mid-write");
    assert_eq!(err.os_code(), Some(no_space_code));
    assert!(failed.cleanup.is_none(), "the staging file was removed");
    assert_eq!(
        fs::read(&target).expect("the target must survive"),
        b"untouched"
    );
    assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
}

// ---- read-only (spec 15.1): the gate by fixture, the write path ------

#[test]
fn the_read_only_gate_is_driven_by_a_real_read_only_mark() {
    // T-F5's shape. The write-rights gate answers a real mark — the
    // probe IS the production check, so the point is fixture-driven
    // (no injection: the mark is cheaply arrangeable, and injecting
    // here would only test the injection).
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = existing(dir.path(), "AGENTS.md", b"untouched");
    // Keep the original rights for the restore: `set_readonly(false)`
    // would add write bits the file may never have had; putting the
    // original back is exact.
    let original = fs::metadata(&target)
        .expect("stating the fixture must work")
        .permissions();
    let mut marked = original.clone();
    marked.set_readonly(true);
    fs::set_permissions(&target, marked).expect("marking read-only must work");

    let err = check_write_access(&target).expect_err("a read-only target must be refused");

    assert!(
        matches!(err, WriteAccessError::FileReadOnly { .. }),
        "the verdict is the §10 E_NO_WRITE_ACCESS fact: {err:?}"
    );

    // Restore so the TempDir teardown can remove the file.
    fs::set_permissions(&target, original).expect("restoring must work");
}

#[test]
fn a_read_only_volume_refusal_on_the_write_path_is_injectable() {
    // The write path's own read-only shape (EROFS = 30 on Linux,
    // a read-only filesystem refusing the staging create): the same
    // one-shot step fault injects it, and the classification is the
    // plain E_WRITE_FAILED step fact.
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = existing(dir.path(), "AGENTS.md", b"untouched");
    let _guard = fault_injection::arm_atomic_fault(AtomicFault::Fail(
        AtomicStep::TempCreate,
        io::Error::from_raw_os_error(30),
    ));

    let err = write_target_file(&target, b"abc", plenty_free())
        .expect_err("an injected EROFS must fail the write");

    let AtomicWriteError::WriteFailed(failed) = &err else {
        panic!("a read-only-shaped refusal must be WriteFailed, got: {err:?}");
    };
    assert_eq!(failed.step, WriteStep::TempCreate);
    assert_eq!(err.os_code(), Some(30));
    assert_eq!(
        fs::read(&target).expect("the target must survive"),
        b"untouched"
    );
}

// ---- locked file (spec 15.1, T-F8's schedule) -------------------------

#[test]
fn a_locked_replace_exhausts_into_file_locked() {
    // ERROR_SHARING_VIOLATION = 32: the simulated lock drives the
    // REAL §9.5 retry schedule with the recording clock, so the
    // typed endgame is drivable from outside on every platform.
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = existing(dir.path(), "AGENTS.md", b"untouched");
    let _guard = fault_injection::arm_locked_replace(LockedReplacePlan::Exhaust { code: 32 });

    let err = write_target_file(&target, b"abc", plenty_free())
        .expect_err("a permanently locked target must fail the write");

    let AtomicWriteError::FileLocked(failed) = &err else {
        panic!("exhausted lock retries are E_FILE_LOCKED, got: {err:?}");
    };
    assert_eq!(failed.step, WriteStep::Replace);
    assert_eq!(failed.path, target);
    assert_eq!(failed.retries, 3, "the whole §9.5 schedule ran");
    assert_eq!(err.os_code(), Some(32));
    assert!(failed.cleanup.is_none(), "the staging file was removed");
    assert_eq!(
        fault_injection::recorded_lock_pauses(),
        [
            Duration::from_millis(100),
            Duration::from_millis(300),
            Duration::from_millis(900),
        ],
        "the real schedule ran, in order, without real sleeping"
    );
    assert_eq!(
        fs::read(&target).expect("the target must survive"),
        b"untouched"
    );
    assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
}

// ---- the pause points (spec 15.1 "force-killing the process") --------

#[test]
fn the_pause_inside_a_write_parks_between_staging_and_rename() {
    // T-F2's window: while the write is parked, the disk holds
    // exactly what a kill there would leave — the untouched original
    // plus one sweepable staging file. The pause state is
    // thread-local (the module's documented discipline), so the
    // worker thread arms it and hands the control end over.
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = existing(dir.path(), "AGENTS.md", b"untouched");
    let (control_tx, control_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = {
        let target = target.clone();
        let control_tx = control_tx;
        thread::spawn(move || {
            let (_guard, control) = fault_injection::arm_pause(PausePlan {
                point: PausePoint::StagingSettled,
                skip: 0,
            });
            control_tx.send(control).expect("handing the control over");
            let outcome = write_target_file(&target, b"new", plenty_free());
            done_tx
                .send(outcome.expect("the released write must complete"))
                .expect("reporting the outcome");
        })
    };

    let control = control_rx
        .recv_timeout(WAIT)
        .expect("the worker must arm the pause");
    assert_eq!(
        control.wait_for_arrival(WAIT),
        Some(PausePoint::StagingSettled),
        "the write must park inside the window"
    );

    // The disk state in the window: the original is whole, and
    // exactly one grammar-conforming staging file exists (the debris
    // a kill would leave, sweepable by temp_files' hour rule).
    assert_eq!(
        fs::read(&target).expect("the target must survive"),
        b"untouched"
    );
    let staging: Vec<String> = entry_names(dir.path())
        .into_iter()
        .filter(|name| name != "AGENTS.md")
        .collect();
    assert_eq!(staging.len(), 1, "exactly one staging file: {staging:?}");
    assert!(
        is_app_tmp_file(&staging[0], "AGENTS.md"),
        "the parked staging file is sweepable debris: {}",
        staging[0]
    );
    // The "written and settled" half of the window: the staging file
    // holds the COMPLETE new content — a pause parked one step
    // earlier (after the create, before the write) would leave an
    // empty file and fail this assert, so the point's position
    // within the write is pinned, not just its existence.
    assert_eq!(
        fs::read(dir.path().join(&staging[0])).expect("the parked staging file must be readable"),
        b"new",
        "the staging file is settled with the full new content"
    );
    // The worker is parked, not merely past the signal: no outcome
    // can exist before the release.
    assert!(
        done_rx.try_recv().is_err(),
        "the worker must be parked inside the window"
    );

    control.release();
    worker.join().expect("the worker must not panic");
    let written = done_rx
        .recv_timeout(WAIT)
        .expect("the released write must report");
    assert_eq!(fs::read(&target).expect("the write must land"), b"new");
    assert_eq!(written.size, 3);
    assert_eq!(entry_names(dir.path()), ["AGENTS.md"], "no debris left");

    // The guard died with the worker: no pause leaked into this
    // thread's writes.
    write_target_file(&target, b"again", plenty_free())
        .expect("a write on the test thread must not park");
    assert_eq!(fs::read(&target).expect("the rewrite must land"), b"again");
}

#[test]
fn the_pause_between_phases_parks_before_anything_the_write_touches() {
    // The backup phase (§9.4 phase 2) has run; the write (§9.5) is
    // parked at its entry — the between-phases window. The disk
    // holds the completed backup and the untouched target.
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).expect("creating the fixture repo must work");
    let target = existing(&repo, "AGENTS.md", b"old");
    let data_dir = dir.path().join("data");
    fs::create_dir(&data_dir).expect("creating the fixture data dir must work");

    let (control_tx, control_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = {
        let target = target.clone();
        let data_dir = data_dir.clone();
        let control_tx = control_tx;
        thread::spawn(move || {
            let (_guard, control) = fault_injection::arm_pause(PausePlan {
                point: PausePoint::WriteEntry,
                skip: 0,
            });
            control_tx.send(control).expect("handing the control over");
            backup_call(&target, &data_dir)
                .expect("the backup phase must complete before the pause");
            write_target_file(&target, b"new", plenty_free())
                .expect("the released write must complete");
            done_tx.send(()).expect("reporting the outcome");
        })
    };

    let control = control_rx
        .recv_timeout(WAIT)
        .expect("the worker must arm the pause");
    assert_eq!(
        control.wait_for_arrival(WAIT),
        Some(PausePoint::WriteEntry),
        "the write must park at its entry"
    );

    // The between-phases disk state: the backup copy exists, the
    // target is untouched, nothing is staged.
    assert_eq!(
        backup_contents(&data_dir),
        [b"old".to_vec()],
        "phase 2's copy is complete and verified"
    );
    assert_eq!(
        fs::read(&target).expect("the target must be untouched"),
        b"old"
    );
    assert_eq!(entry_names(&repo), ["AGENTS.md"], "nothing is staged yet");
    // Parked, not merely past the signal (an implementation that
    // signalled without blocking would have the write done).
    assert!(
        done_rx.try_recv().is_err(),
        "the worker must be parked at the write's entry"
    );

    control.release();
    worker.join().expect("the worker must not panic");
    done_rx.recv_timeout(WAIT).expect("the write must finish");
    assert_eq!(fs::read(&target).expect("the write must land"), b"new");
    assert_eq!(
        backup_contents(&data_dir),
        [b"old".to_vec()],
        "the write phase does not add copies"
    );
    assert_eq!(entry_names(&repo), ["AGENTS.md"]);
}

#[test]
fn the_pause_skip_counter_selects_the_later_arrival() {
    // skip = 1: the FIRST write's entry passes unparked (target A is
    // already written when the pause fires — the discriminator: a
    // broken skip counter would park at A's entry, while A is still
    // old), the second write parks. One shot: a third write after
    // the release completes without parking (the done signal could
    // not arrive otherwise).
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let repo = dir.path().join("repo");
    fs::create_dir(&repo).expect("creating the fixture repo must work");
    let target_a = existing(&repo, "A.md", b"old-a");
    let target_b = existing(&repo, "B.md", b"old-b");

    let (control_tx, control_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = {
        let target_a = target_a.clone();
        let target_b = target_b.clone();
        let control_tx = control_tx;
        thread::spawn(move || {
            let (_guard, control) = fault_injection::arm_pause(PausePlan {
                point: PausePoint::WriteEntry,
                skip: 1,
            });
            control_tx.send(control).expect("handing the control over");
            write_target_file(&target_a, b"new-a", plenty_free())
                .expect("the skipped write must complete unparked");
            write_target_file(&target_b, b"new-b", plenty_free())
                .expect("the released write must complete");
            write_target_file(&target_a, b"new-a-2", plenty_free())
                .expect("a one-shot pause must not park the third write");
            done_tx.send(()).expect("reporting the outcome");
        })
    };

    let control = control_rx
        .recv_timeout(WAIT)
        .expect("the worker must arm the pause");
    assert_eq!(
        control.wait_for_arrival(WAIT),
        Some(PausePoint::WriteEntry),
        "the second arrival must be the parked one"
    );
    assert_eq!(
        fs::read(&target_a).expect("A must already be written"),
        b"new-a",
        "the first arrival was skipped — a pause there would leave A old"
    );
    assert_eq!(fs::read(&target_b).expect("B must still be old"), b"old-b");
    // Parked on the SECOND arrival: B's write cannot be done before
    // the release.
    assert!(
        done_rx.try_recv().is_err(),
        "the worker must be parked on the second arrival"
    );

    control.release();
    worker.join().expect("the worker must not panic");
    done_rx
        .recv_timeout(WAIT)
        .expect("the one-shot pause let the third write through");
    assert_eq!(fs::read(&target_b).expect("B must land"), b"new-b");
    assert_eq!(
        fs::read(&target_a).expect("A must be rewritten"),
        b"new-a-2"
    );
    assert_eq!(entry_names(&repo), ["A.md", "B.md"]);
}
