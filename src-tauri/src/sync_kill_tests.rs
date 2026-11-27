//! Forced process termination at the run level (task T-130; spec
//! §15.2's kill rows T-F2 and T-F3, §15.1's fault-injection row
//! "force-killing the process between phases and inside a write",
//! NFR-024; DECISIONS.md D-226).
//!
//! The sibling suites pin the pipeline's refusals in-process; a kill
//! test cannot: the process must DIE for real — `SIGKILL` on Unix,
//! `TerminateProcess` on Windows, one portable primitive
//! ([`std::process::Child::kill`]) — with no unwinding, no §9.5
//! step-7 cleanup, no RAII teardown, exactly the crash §9.4's
//! "recovery after a crash" paragraph and ERR-002's "an interrupted
//! file write leaves the old file (9.5)" are about. So each test here
//! runs the REAL `sync_run` command in a CHILD PROCESS — this same
//! test binary re-executed under its own `--exact` filter — and the
//! parent kills it inside the window, then inspects the disk and the
//! database the child left behind.
//!
//! # The child-process design
//!
//! - **Re-exec.** The parent spawns `std::env::current_exe()` with
//!   `--exact <this test's name>` and a specification carried as env
//!   vars on the [`Command`] (`Command::env` — the process-local
//!   API; the parent's own environment is never touched, so parallel
//!   tests in one process cannot see each other's children). The
//!   child's first act inside the test fn is the role switch: the
//!   spec env var present → the child half runs and never returns;
//!   absent → the normal parent body. Under nextest (one test per
//!   process) and plain `cargo test` (tests in threads) the shape is
//!   identical: the `--exact` filter guarantees the child process
//!   runs ONLY the test that spawned it.
//! - **The world.** The child does not build its own world: it opens
//!   the PARENT's data root (the same SQLite database, the same
//!   watched repositories) through the same `Db::open` a restarted
//!   app would run, manages the same mock-app shell, and goes live
//!   the same way — the next process over a crashed one, which is
//!   exactly the position §9.4's recovery story puts the app in.
//! - **The kill window.** Two park-site kinds, one per §15.1
//!   injection surface. The fs pause points (the `fault-injection`
//!   cargo feature, T-069/D-101 — enabled for this crate's test
//!   builds by the dev-dependency edge recorded in D-226) park the
//!   run INSIDE an `fs` operation: [`PausePoint::StagingSettled`]
//!   after the staging file is written and settled, before the
//!   `rename` (T-F2's window); [`PausePoint::WriteEntry`] at the seam
//!   between the backup phase and the write (§15.1's "between
//!   phases") and — gated on a later project's target — between two
//!   projects' writes (T-F3's window). The pause state is
//!   THREAD-LOCAL, and phase 3 runs every write on the blocking
//!   pool, so the arm happens inside the `write` seam's closure — on
//!   the very thread that runs the production §9.5 operation —
//!   gated on the resolved target path (one arm per matching write;
//!   the closure's thread is whichever the pool chose, so
//!   target-gating replaces arrival counting). The closure hands the
//!   [`PauseControl`] to a supervisor thread, which waits for the
//!   arrival, writes a marker file, and HOLDS the control (a dropped
//!   control releases the park) until the parent's kill or a bounded
//!   ceiling — an orphaned child whose parent died self-releases and
//!   exits; a healthy CI runner keeps no parked process. The
//!   seam-side parks (T-127/T-128's app-layer injector pattern, the
//!   `SyncRunSeams` closures' own code) cover the step boundaries
//!   BETWEEN a seam's production call and the phase's next
//!   transaction: the backup closure parks after the copy returned
//!   Ok, before phase 2's ONE record transaction (inside phase 2
//!   step 1); the write closure parks after the §9.5 call returned
//!   Ok, before phase 3's verdict/stamp transaction (the step 1→2
//!   boundary) — each holding its blocking-pool thread with
//!   `thread::park_timeout` under the same ceiling.
//! - **The handshake.** The parent polls the marker's existence with
//!   a 30 s failure ceiling and simultaneously watches for the
//!   child's early exit (a run that completed without parking — a
//!   broken fixture — fails fast with the child's exit status, not a
//!   timeout). The marker is written only AFTER the arrival, and the
//!   run provably parks to write it, so when the parent sees the
//!   marker the child is INSIDE the window: the kill lands there by
//!   construction, not by timing. A [`ReapOnDrop`] guard kills and
//!   reaps the child on every parent exit path — a parked child is
//!   never leaked.
//!
//! # What pins what
//!
//! | test | spec clauses |
//! | ---- | ------------- |
//! | `a_kill_after_the_temp_file_is_written_before_the_rename_leaves_the_original_intact_and_the_next_sync_deletes_the_temp_file` | T-F2, NFR-024, OUT-007/008 ("Killing the process during a write does not leave a partly written file"), §9.5 steps 2–4/7, §9.4 phase 1 step 7 (D-088), §9.4 "recovery after a crash", ERR-002 |
//! | `a_kill_between_writing_project_1_and_project_2_keeps_project_1s_current_stamp_and_marks_the_run_interrupted` | T-F3, NFR-024, §9.4 phase 3 steps 1–2 (OUT-020's right-away stamp), §9.4 "recovery after a crash" (the `interrupted` marking, no project row touched), D-147's NULL `error_code` ruling, MAIN-027 (§9.4 phase 4 step 4) |
//! | `a_kill_between_the_backup_phase_and_the_write_touches_no_target_file` | NFR-024 ("for each step of pipeline phases 2–3"), §15.1's "between phases", §9.4 phase 2 step 1 (the backups are committed work), OUT-007 (the §9.5 write — phase 3 step 1 — is the first thing that touches a target; phase 2 only copies), §9.4 "recovery after a crash", ERR-002 |
//! | `a_kill_between_the_backup_copy_and_its_record_touches_no_target_file` | NFR-024, §9.4 phase 2 step 1 (the copy, the check, the record — one step, killed between its copy and record halves), OUT-007 (phase 2 only copies; the write is phase 3's), §9.4 "recovery after a crash" |
//! | `a_kill_between_the_write_and_its_stamp_leaves_the_file_whole_the_row_unwritten_and_marks_the_run_interrupted` | NFR-024, OUT-007/008, §9.4 phase 3 steps 1–2 (the step boundary), §9.4 "recovery after a crash" (the contrast case: the row does NOT yet reflect the write), §10's idempotence + ALG-033 (the convergence epilogue), OUT-020, MAIN-027 |
//!
//! # The scoping of NFR-024's "each step"
//!
//! NFR-024 asks for a kill test "for each step of pipeline phases
//! 2–3", and §9.4 enumerates the steps: phase 2 step 1 (the copy, its
//! check, its record), phase 2's tail (step 3's point of no return —
//! the phase 2→3 boundary), phase 3 step 1 (the §9.5 write, per
//! project), and phase 3 step 2 (the right-away verdict/stamp
//! transaction). The error-handling steps (phase 2 step 2, phase 3
//! step 3) are failure paths, not kill targets. All four kill-relevant
//! step windows are covered, each by its own park site: INSIDE phase
//! 2 step 1 ([`ParkSite::AfterBackup`] — after the copy landed,
//! before phase 2's ONE record transaction), the phase 2→3 boundary
//! and between projects ([`ParkSite::PausePoint`] with
//! `WriteEntry`), inside the §9.5 write ([`PausePoint`] with
//! `StagingSettled`, T-F2), and phase 3's step 1→2 boundary
//! ([`ParkSite::AfterWrite`] — the file wholly written, before the
//! verdict/stamp transaction). The two park-site kinds are the two
//! §15.1 injection surfaces: the fs pause points (D-101's catalogue)
//! for the windows INSIDE an `fs` operation, and the seam closures'
//! own parks (T-127/T-128's app-layer injector pattern) for the
//! boundaries BETWEEN a seam's production call and the phase's next
//! transaction — the round-1 review's P0, resolved by covering the
//! step windows the pause catalogue alone could not reach.
//!
//! The one honest boundary left is positional, not step-level: a kill
//! in the MIDDLE of the copy's byte stream (rather than after it
//! landed) leaves a shorter uncommitted file in the app's own backups
//! tree — the same crash class as the landed unrecorded copy (the
//! targets are untouched either way, and the uncommitted copy is
//! debris the spec assigns no sweep to: D-088's sweep is for target
//! folders, retention prunes records, not files). Recorded in D-226
//! with the follow-up for the board.
//!
//! # Oracles and boundaries
//!
//! The expected values are the spec's literals, spelled here (the
//! sibling suite's posture): the intact file is the hand-written
//! fixture bytes, the built bodies are `BODY`/`BODY_REVISED` from
//! ALG-020…022, the statuses are the §6.2 CHECK literals, and T-F3's
//! "old" stamp is the run-1 captured pair — the ORACLE is the
//! "unchanged" relation, not a predicted stamp, and the stamp's
//! DB-002 shape is checked independently by `assert_stamped`.
//!
//! The `interrupted` marking is driven the way production drives it:
//! the parent calls `Db::mark_interrupted_runs` with
//! `eunomia_store::values::now_utc()` — the exact call
//! `start_database_session` (lib.rs's session tail) makes with the
//! same input. The startup WIRING (that the tail calls it) is
//! D-147's marker pin and stays there: the real `setup` hook cannot
//! launch under a unit harness (the house posture), and this suite's
//! subject is the crash state, not the launch.
//!
//! T-F2's "on the next sync, the temp file is deleted" is read with
//! §9.4 phase 1 step 7's own rule in view: the sweep deletes debris
//! "older than one hour" (D-088's window — it exists so a live
//! write's staging file is never a deletion candidate). The next
//! sync that can delete a killed run's debris is the one after the
//! debris crosses the hour; the test simulates the hour by backdating
//! the debris's mtime (the fs suite's own technique) and then runs
//! the REAL next sync, which sweeps and writes through the command.
//! The one-hour window itself — a fresh staging file staying — is
//! `temp_files`' own pin, not re-pinned here.
//!
//! The kill is the portable primitive on every platform
//! (`Child::kill` is `SIGKILL`/`TerminateProcess`), so the fixture
//! carries no platform gate; the Unix leg additionally asserts the
//! signal number (the §15.1 literal) behind `#[cfg(unix)]`. The CI
//! matrix runs the suite on all three OSes; nothing here is
//! manual-smoke. The watch service's verdicts, the backup records'
//! lifecycle, the sweep's near-miss grammar and the pause points'
//! positions within the §9.5 step sequence are the owning suites'
//! pins, not re-pinned here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use jiff::Timestamp;
use tauri::Manager;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};

use eunomia_fs::atomic::{AtomicWriteError, WrittenFile};
use eunomia_fs::backup::{BackupError, BackupOutcome};
use eunomia_fs::fault_injection::{PauseControl, PausePlan, PausePoint, arm_pause};
use eunomia_fs::temp_files::is_app_tmp_file;
use eunomia_fs::temp_repos::{RepoRoot, TempRepos};
use eunomia_store::library::{
    NewAssignment, NewLibraryItem, NewVariant, VariantUpdate, assign, create_item, create_variant,
    update_variant,
};
use eunomia_store::projects::{AddedVia, NewProject, Project};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::sync_runs::list_projects;

use crate::commands::{
    CancelToken, SyncRunOutcome, SyncRunSeams, SyncRunSummary, projects_store_to_app_error,
};
use crate::data_dirs;
use crate::db::{Db, sqlite_error_to_app_error, sync_runs_app_error_for_tests};
use crate::dirty_recalc::library_seed_error_for_tests;
use crate::errors::AppError;
use crate::file_watch::{self, FileChangedEvent, FileWatchState};

// ---- the spec's literals ---------------------------------------------------

/// The §9.2 body the standard library builds — ALG-020…022 spelled by
/// hand (the sibling suite's own literal, kept identical so the two
/// files cannot drift apart):
///
/// ```text
/// Preamble text.
///
/// ## Wild
///
/// Zebra text.
///
/// ### Tame
///
/// Yak text.
/// ```
const BODY: &str = "Preamble text.\n\n## Wild\n\nZebra text.\n\n### Tame\n\nYak text.";

/// The body after T-F3's library edit: Xray's main variant text is
/// revised, everything else stays — the only change the second run
/// carries.
const BODY_REVISED: &str =
    "Preamble text, revised.\n\n## Wild\n\nZebra text.\n\n### Tame\n\nYak text.";

/// ALG-032's fixed frame, for the marker-shape checks only (the full
/// field-by-field validation is the sibling suite's pin).
const MARKER_PREFIX: &str = "<!-- Created by EunomiaSync ";
const MARKER_SUFFIX: &str = " -->";

/// The label of this suite's repository group inside `TempRepos`'s
/// base (the group is unique per `TempRepos::create` — its own
/// `TempDir` — so the label is a name, never a collision risk).
const REPOS_LABEL: &str = "eunomia-t130";

/// The test debounce: the production 300 ms of SYS-007 is the watch
/// service's own suite's pin; a short window keeps this suite quick.
const DEBOUNCE: Duration = Duration::from_millis(40);

/// The child-handshake ceiling: arming the pause, parking and writing
/// the marker answer in well under a second; 30 s is the failure
/// ceiling (the D-140 posture).
const HANDSHAKE: Duration = Duration::from_secs(30);

/// How long a parked child holds the kill window open when no kill
/// comes: the parent kills it milliseconds after the marker, so this
/// ceiling exists only for an ORPHANED child (its parent died before
/// the kill) — the deadline drops the [`PauseControl`], the park
/// releases, the run completes, and the child exits instead of
/// outliving its test on the CI runner.
const PARK_CEILING: Duration = Duration::from_secs(120);

/// The age the T-F2 debris is backdated to: comfortably past §9.4
/// phase 1 step 7's "older than one hour" (D-088's strict window), so
/// the next sync's sweep sees debris, not a live write's staging file.
const DEBRIS_AGE: Duration = Duration::from_secs(2 * 60 * 60);

// ---- the child specification ------------------------------------------------

/// The role switch: present in the spawned child's environment only.
const CHILD_SWITCH: &str = "EUNOMIA_SYNC_KILL_CHILD";
/// The parent's data root — the database and backups the child opens.
const CHILD_DATA_ROOT: &str = "EUNOMIA_SYNC_KILL_CHILD_DATA_ROOT";
/// Where the child parks the run, encoded as one site literal
/// (`pause:staging-settled`, `pause:write-entry`, `after-write`,
/// `after-backup`) — see [`ParkSite`].
const CHILD_SITE: &str = "EUNOMIA_SYNC_KILL_CHILD_SITE";
/// The resolved path the park is gated on: the write's target for the
/// write-side sites, the copy's source for the backup-side site.
const CHILD_TARGET: &str = "EUNOMIA_SYNC_KILL_CHILD_TARGET";
/// Where the child announces it is parked inside the window.
const CHILD_MARKER: &str = "EUNOMIA_SYNC_KILL_CHILD_MARKER";

/// Where the child parks the run for the kill — one per NFR-024 step
/// window, in the pipeline's own order:
///
/// - [`ParkSite::PausePoint`] parks through eunomia-fs's
///   fault-injection pause (the §15.1 "failure points in `fs`"
///   mechanism, D-101), armed inside the write seam on the resolved
///   target: `StagingSettled` is T-F2's window (inside §9.5's write,
///   after the staging file is settled, before the rename);
///   `WriteEntry` is the backup-phase→write seam (§15.1's "between
///   phases") and — gated on a later project — T-F3's
///   between-projects window.
/// - [`ParkSite::AfterWrite`] parks the write seam's closure AFTER
///   the production §9.5 call returned Ok (the file wholly written
///   and renamed) and BEFORE phase 3's verdict/stamp transaction
///   runs — phase 3's step 1→2 boundary, the "file whole, row
///   `not_started`" crash shape §9.4's recovery clause contrasts
///   with T-F3's.
/// - [`ParkSite::AfterBackup`] parks the backup seam's closure AFTER
///   the production copy returned Ok (the copy landed and was
///   checked) and BEFORE phase 2's ONE record transaction — inside
///   phase 2 step 1, the "copy on disk, no `backup_records` row"
///   crash shape.
///
/// The seam-side parks are the app layer's own injector pattern
/// (T-127/T-128's `SyncRunSeams` precedent — the §15.1 "failure
/// points" surface at the run level): the closure blocks on its own
/// channel before returning, freezing the pipeline at a real step
/// boundary; the kill that follows is the same real `Child::kill`.
enum ParkSite {
    /// The fs pause point, armed on the write seam for this target.
    PausePoint {
        /// The pause point that parks the run.
        point: PausePoint,
        /// The write's resolved target that arms the pause.
        target: PathBuf,
    },
    /// After the write, before phase 3's verdict/stamp transaction.
    AfterWrite {
        /// The write's resolved target that gates the park.
        target: PathBuf,
    },
    /// After the copy, before phase 2's record transaction.
    AfterBackup {
        /// The copy's resolved source path that gates the park.
        target: PathBuf,
    },
}

impl ParkSite {
    /// The env encoding: one literal per site.
    fn encode(&self) -> String {
        match self {
            ParkSite::PausePoint { point, .. } => {
                format!("pause:{}", encode_point(*point))
            }
            ParkSite::AfterWrite { .. } => "after-write".to_owned(),
            ParkSite::AfterBackup { .. } => "after-backup".to_owned(),
        }
    }

    /// The gate the park is keyed on (the write target or the copy
    /// source).
    fn target(&self) -> &Path {
        match self {
            ParkSite::PausePoint { target, .. }
            | ParkSite::AfterWrite { target }
            | ParkSite::AfterBackup { target } => target,
        }
    }
}

/// Whether this process was spawned as a kill child: the role switch
/// requires the VALUE, not just the variable's presence — a
/// developer environment that happens to carry a stray
/// `EUNOMIA_SYNC_KILL_CHILD` must not flip a parent run into the
/// child role (the child spec's other vars would then fail loudly,
/// but the fragility itself is the mistake; the value is the
/// witness).
fn running_as_kill_child() -> bool {
    std::env::var(CHILD_SWITCH).as_deref() == Ok("1")
}

fn decode_site(site: &str, target: PathBuf) -> ParkSite {
    match site {
        "pause:staging-settled" => ParkSite::PausePoint {
            point: PausePoint::StagingSettled,
            target,
        },
        "pause:write-entry" => ParkSite::PausePoint {
            point: PausePoint::WriteEntry,
            target,
        },
        "after-write" => ParkSite::AfterWrite { target },
        "after-backup" => ParkSite::AfterBackup { target },
        other => panic!("an unknown park site reached the child: {other:?}"),
    }
}

fn encode_point(point: PausePoint) -> &'static str {
    match point {
        PausePoint::StagingSettled => "staging-settled",
        PausePoint::WriteEntry => "write-entry",
    }
}

/// One kill test's child specification: which test to re-enter, which
/// world to open, where the run parks, and where the park is
/// announced.
struct ChildSpec {
    /// The `--exact` filter — this test's own libtest name.
    test: String,
    /// The parent's data root: the child opens the same database.
    data_root: PathBuf,
    /// Where the child parks the run inside the kill window.
    site: ParkSite,
    /// The marker file the child writes once parked.
    marker: PathBuf,
}

/// This test's own libtest name, derived from `module_path!` (the
/// crate segment is not part of a test's name): the child re-executes
/// the binary under `--exact <name>`, so a renamed module cannot
/// silently break the spawn — the child would run zero tests, exit,
/// and the parent's handshake would fail fast naming the exit.
fn self_test_path(fn_name: &str) -> String {
    let module = module_path!();
    let rest = module
        .split_once("::")
        .expect("the module path has a crate segment")
        .1;
    format!("{rest}::{fn_name}")
}

/// Spawns the child: this test binary, `--exact`-filtered to the one
/// test (`--quiet`: the child harness's own "running 1 test" lines
/// would otherwise pollute the parent's captured output — panics
/// still print, which is the useful half), carrying the
/// specification as process-local env vars.
fn spawn_kill_child(spec: &ChildSpec) -> Child {
    let exe = std::env::current_exe().expect("the test binary's own path must resolve");
    Command::new(exe)
        .arg("--exact")
        .arg(&spec.test)
        .arg("--quiet")
        // The harness's own summary lines would otherwise pollute the
        // parent's captured output (`--quiet` halves them already);
        // stderr stays inherited — the child's panic text is the
        // useful failure diagnostic, and it rides stderr.
        .stdout(Stdio::null())
        .env(CHILD_SWITCH, "1")
        .env(CHILD_DATA_ROOT, &spec.data_root)
        .env(CHILD_SITE, spec.site.encode())
        .env(CHILD_TARGET, spec.site.target())
        .env(CHILD_MARKER, &spec.marker)
        .spawn()
        .expect("the kill-child test process must spawn")
}

// ---- the child half ---------------------------------------------------------

/// Holds the calling blocking-pool thread parked until
/// [`PARK_CEILING`] passes: the loop absorbs `thread::park_timeout`'s
/// spurious wakeups, so only the deadline (or the parent's kill, which
/// takes the whole process) ends the park — the deadline being the
/// orphaned-child self-release.
fn park_until_ceiling() {
    let deadline = Instant::now() + PARK_CEILING;
    while Instant::now() < deadline {
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
    }
}

/// The child half of a kill test, entered from the top of the test
/// whose name the `--exact` filter selected: opens the parent's world
/// through the production app shell, arms the pause inside the write
/// seam, runs the REAL `sync_run` command, and parks — dying by the
/// parent's kill. Never returns: reaching the end means the park
/// never engaged (or the ceiling released it — an orphaned child),
/// and the panic names what the run answered.
fn run_child_role() -> ! {
    let data_root = PathBuf::from(
        std::env::var(CHILD_DATA_ROOT).expect("the child spec carries the data root"),
    );
    let site = decode_site(
        &std::env::var(CHILD_SITE).expect("the child spec carries the park site"),
        PathBuf::from(std::env::var(CHILD_TARGET).expect("the child spec carries the gate")),
    );
    let marker =
        PathBuf::from(std::env::var(CHILD_MARKER).expect("the child spec carries the marker path"));

    // The app shell over the PARENT's world: the same database file
    // and the same watched repositories, a fresh process — the exact
    // position of a restarted app over a crashed run's data root
    // (multi-process SQLite over WAL is NFR-007's own posture). The
    // layout exists already; the child creates nothing.
    let (app, bus) = app_shell(&data_root);
    go_live(&app, bus);

    // The park plumbing: `parked_tx` is what a seam closure signals on
    // the moment it parks; the supervisor answers by writing the
    // marker. The signal sender rides a `Mutex`: `mpsc::Sender` is
    // not `Sync`, and the seam closures must be. A SEAM-side park
    // then holds its own blocking-pool thread with
    // `thread::park_timeout` until the deadline (the loop absorbs
    // spurious wakeups) — the kill takes the process down with the
    // thread still parked, and the deadline itself is the
    // orphaned-child self-release (the pause-point sites release
    // through the control's drop instead).
    enum Parked {
        /// The fs pause fired; the control holds the window open.
        Pause(PauseControl),
        /// A seam closure parked after its production call returned.
        Seam,
    }
    let (parked_tx, parked_rx) = mpsc::channel::<Parked>();
    let parked_sender = Mutex::new(parked_tx);

    // The seams, per site: production everywhere except the parked
    // seam. The pause state is THREAD-LOCAL and phase 3 runs each
    // write on the blocking pool, so a `PausePoint` site arms INSIDE
    // the write closure — on the very thread the §9.5 operation runs
    // on — gated on the resolved target (the gate replaces arrival
    // counting: the closure's thread is whichever the pool chose). An
    // `AfterWrite` site parks the write closure AFTER the production
    // call returned Ok, freezing the pipeline between phase 3's write
    // and its verdict/stamp transaction; an `AfterBackup` site parks
    // the backup closure after the copy returned Ok, freezing phase 2
    // between the copy and its ONE record transaction. Each arm is a
    // struct-literal coercion site, so the closures need no spelled
    // `dyn` types (the fields carry them).
    let seams = match &site {
        ParkSite::PausePoint { point, target } => {
            let point = *point;
            let gate = target.clone();
            SyncRunSeams {
                backup: Arc::new(eunomia_fs::backup::backup_target_file),
                write: Arc::new(
                    move |target: &Path,
                          contents: &[u8]|
                          -> Result<WrittenFile, AtomicWriteError> {
                        // Arm only for the gated target, and keep the
                        // guard ALIVE ACROSS the production call — the
                        // pause fires inside it, and a guard scoped to
                        // this arm's block would disarm the pause
                        // before its point (fault_injection's own
                        // Disconnected panic is what names that
                        // mistake: the underscore-prefixed binding
                        // still owns the guard, so it lives until the
                        // closure's tail returns — a bare `let _`
                        // would drop it immediately).
                        let _guard = (target == gate).then(|| {
                            let (guard, control) = arm_pause(PausePlan { point, skip: 0 });
                            parked_sender
                                .lock()
                                .expect("the park sender is never poisoned")
                                .send(Parked::Pause(control))
                                .expect("the supervisor is waiting for the park");
                            guard
                        });
                        crate::sync_phase3::production_write(target, contents)
                    },
                ),
                abort_clock: SyncRunSeams::production().abort_clock,
                progress_clock: SyncRunSeams::production().progress_clock,
            }
        }
        ParkSite::AfterWrite { target } => {
            let gate = target.clone();
            SyncRunSeams {
                backup: Arc::new(eunomia_fs::backup::backup_target_file),
                write: Arc::new(
                    move |target: &Path,
                          contents: &[u8]|
                          -> Result<WrittenFile, AtomicWriteError> {
                        let outcome = crate::sync_phase3::production_write(target, contents);
                        if outcome.is_ok() && target == gate {
                            parked_sender
                                .lock()
                                .expect("the park sender is never poisoned")
                                .send(Parked::Seam)
                                .expect("the supervisor is waiting for the park");
                            park_until_ceiling();
                        }
                        outcome
                    },
                ),
                abort_clock: SyncRunSeams::production().abort_clock,
                progress_clock: SyncRunSeams::production().progress_clock,
            }
        }
        ParkSite::AfterBackup { target } => {
            let gate = target.clone();
            SyncRunSeams {
                backup: Arc::new(
                    move |target: &eunomia_fs::backup::BackupTarget<'_>|
                          -> Result<BackupOutcome, BackupError> {
                        let outcome = eunomia_fs::backup::backup_target_file(target);
                        if outcome.is_ok() && target.source == gate {
                            parked_sender
                                .lock()
                                .expect("the park sender is never poisoned")
                                .send(Parked::Seam)
                                .expect("the supervisor is waiting for the park");
                            park_until_ceiling();
                        }
                        outcome
                    },
                ),
                write: Arc::new(crate::sync_phase3::production_write),
                abort_clock: SyncRunSeams::production().abort_clock,
                progress_clock: SyncRunSeams::production().progress_clock,
            }
        }
    };

    // The supervisor: waits for the park, announces it, and — for the
    // pause-point sites — HOLDS the control (a dropped control
    // releases the park; the window would close before the kill). A
    // handshake failure is a panic in this thread: loud on the
    // child's stderr, and the run still ends (the write completes or
    // fails, `block_on` returns, the child panics below) — no path
    // hangs.
    let expected_point = match site {
        ParkSite::PausePoint { point, .. } => Some(point),
        _ => None,
    };
    thread::spawn(move || {
        match parked_rx
            .recv_timeout(HANDSHAKE)
            .expect("a seam must park the run")
        {
            Parked::Pause(control) => {
                assert_eq!(
                    control.wait_for_arrival(HANDSHAKE),
                    expected_point,
                    "the run must park at the armed point"
                );
                fs::write(&marker, b"parked").expect("the park marker must be writable");
                let deadline = Instant::now() + PARK_CEILING;
                while Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(50));
                }
                // The ceiling passed: drop the control (the park
                // releases) and end — an orphaned child must not
                // outlive its test.
            }
            Parked::Seam => {
                fs::write(&marker, b"parked").expect("the park marker must be writable");
                // The closure's own recv timeout releases the park at
                // the ceiling; nothing to hold here.
            }
        }
    });

    // The real command over the production seams (the parked seam
    // excepted): the run parks inside the kill window and the process
    // dies there. Returning is the failure path — the park never
    // engaged, or the ceiling released it — and the panic carries
    // WHAT the run answered, so the parent's failure output names the
    // broken half instead of a bare exit code.
    let outcome = tauri::async_runtime::block_on(crate::commands::sync_run_with(
        app.handle().clone(),
        None,
        CancelToken::new(),
        seams,
    ));
    let description = match &outcome {
        Ok(SyncRunOutcome::Completed { summary }) => format!(
            "completed, verdict {}, statuses {:?}",
            summary.verdict,
            summary
                .projects
                .iter()
                .map(|entry| entry.status.as_str())
                .collect::<Vec<_>>()
        ),
        Ok(SyncRunOutcome::NeedsConfirmation { projects }) => format!(
            "the empty-build confirmation gate opened for {} project(s)",
            projects.len()
        ),
        Err(error) => format!("refused: {}", error.code.as_str()),
    };
    panic!("the child's run ended without parking at the kill window: {description}");
}

// ---- the parent half --------------------------------------------------------

/// A parent world: a mock app, a temp data root, real repositories,
/// and the projects seeded over them in `sort_order` — the sibling
/// suite's shape, owned here per the harness posture (each suite
/// carries its own copies; the kill suite's is leaner: scheme-A
/// projects only, no scheme-B fencing, no watch-verdict reads).
struct World {
    /// The mock app, declared first so it drops before the temp
    /// roots (the live watch service task owns an `AppHandle` and the
    /// managed `Db` — T-342's teardown seam, the sibling suite's
    /// `Drop`).
    app: tauri::App<MockRuntime>,
    /// The watch command bus, held until [`go_live`] hands it to the
    /// service (the D-137 hang shape).
    bus: Option<tauri::async_runtime::Receiver<crate::file_watch::Command>>,
    data_root: tempfile::TempDir,
    /// Underscore-named: holding the fixture IS the effect — the
    /// repositories go with the world.
    _repos: TempRepos,
    projects: Vec<SeededProject>,
}

impl Drop for World {
    fn drop(&mut self) {
        if self.bus.is_none() {
            tauri::async_runtime::block_on(self.app.state::<FileWatchState>().shutdown());
        }
    }
}

impl World {
    fn project(&self, index: usize) -> &SeededProject {
        &self.projects[index]
    }

    fn data_root(&self) -> &Path {
        self.data_root.path()
    }
}

/// A project row seeded over a real repository.
struct SeededProject {
    id: String,
    root: PathBuf,
    target: PathBuf,
}

/// The mock-app shell over a data root: the database, the managed
/// states the command path reads, and the specta events — the sibling
/// suite's `build` head, shared by the parent (its own fresh root)
/// and the child (the parent's root — the only difference is who
/// created the directory).
fn app_shell(
    data_root: &Path,
) -> (
    tauri::App<MockRuntime>,
    tauri::async_runtime::Receiver<crate::file_watch::Command>,
) {
    let database = Db::open(data_root).expect("opening the world's database must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(data_dirs::DataRoot(data_root.to_path_buf()));
    app.manage(crate::sync_running::SyncRunning::new());
    app.manage(crate::commands::SyncCancelSlot::new());
    // The write-in-flight gate phase 3's per-project guard reads
    // (T-123/D-219).
    app.manage(crate::commands::SyncWriteGate::new());
    app.manage(crate::availability::AvailabilityState::new());
    app.manage(crate::dirty_recalc::DirtyState::new().0);
    let (watch_state, bus) = FileWatchState::new();
    app.manage(watch_state);
    tauri_specta::Builder::<MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::availability::ProjectAvailabilityEvent,
            crate::commands::SyncProgressEvent,
            crate::dirty_recalc::DirtyEvent,
            FileChangedEvent,
        ])
        .mount_events(&app);
    (app, bus)
}

/// Builds the parent world and seeds one scheme-A project per entry,
/// in `sort_order`. Not live yet: the caller seeds its fixture files
/// and the library, then calls [`go_live`].
fn build(count: usize) -> World {
    let data_root = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(data_root.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let (app, bus) = app_shell(data_root.path());
    let repos = TempRepos::create(count, REPOS_LABEL).expect("the repository fixture must create");
    let projects = repos
        .roots()
        .iter()
        .enumerate()
        .map(|(index, root)| seed_project(&app, index, root))
        .collect();
    World {
        app,
        bus: Some(bus),
        data_root,
        _repos: repos,
        projects,
    }
}

/// Registers the project over its repository with the canonical path
/// (the DB-030 spelling the target resolution contracts on), as a
/// plain scheme-A `AGENTS.md` project.
fn seed_project(app: &tauri::App<MockRuntime>, index: usize, root: &RepoRoot) -> SeededProject {
    let database = app.state::<Db>().inner().clone();
    let path = root.path.to_string_lossy().into_owned();
    let path_key = root.path_key.clone();
    let name = format!("Project {}", index + 1);
    let label = format!("P{}", index + 1);
    let id = tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path: &path,
                path_key: &path_key,
                name: &name,
                label: &label,
                color: "blue",
                description: "",
                added_via: AddedVia::Manual,
                scheme: Scheme::A,
                base_name: BaseName::AgentsMd,
                extra_local: ExtraFile {
                    enabled: false,
                    position: Position::End,
                },
                extra_project: ExtraFile {
                    enabled: false,
                    position: Position::End,
                },
            },
        )
        .map(|project| project.id)
        .map_err(projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work");
    SeededProject {
        id,
        root: root.path.clone(),
        target: root.path.join("AGENTS.md"),
    }
}

/// The standard library of the sibling suite, verbatim: Xray, `##
/// Wild`, Zebra, `### Tame`, Yak, `## Silent`, Mute (creation order
/// is the library order), Xray/Zebra/Yak assigned to every project —
/// each project's built body is [`BODY`]. Answers Xray's variant id
/// for the library edit.
fn seed_standard_library(world: &World) -> String {
    let database = database_of(world);
    let project_ids: Vec<String> = world
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let (xray, xray_variant) = instruction(tx, "Xray preamble", "Preamble text.")?;
        heading(tx, "Wild", 2)?;
        let (zebra, zebra_variant) = instruction(tx, "Zebra", "Zebra text.")?;
        heading(tx, "Tame", 3)?;
        let (yak, yak_variant) = instruction(tx, "Yak", "Yak text.")?;
        heading(tx, "Silent", 2)?;
        let (_mute, _mute_variant) = instruction(tx, "Mute", "Mute text.")?;
        for project_id in &project_ids {
            for (instruction_id, variant_id) in [
                (&xray, &xray_variant),
                (&zebra, &zebra_variant),
                (&yak, &yak_variant),
            ] {
                assign(
                    tx,
                    &NewAssignment {
                        project_id,
                        instruction_id,
                        variant_id,
                    },
                )
                .map_err(library_seed_error_for_tests)?;
            }
        }
        Ok(xray_variant)
    }))
    .expect("seeding the standard library must work")
}

/// One active instruction with its main variant: `(item id, variant id)`.
fn instruction(
    tx: &rusqlite::Transaction<'_>,
    title: &str,
    content: &str,
) -> Result<(String, String), AppError> {
    let item = create_item(tx, &NewLibraryItem::Instruction { title })
        .map_err(library_seed_error_for_tests)?;
    let variant = create_variant(
        tx,
        &NewVariant {
            instruction_id: &item.id,
            name: "Main",
            content,
            is_main: true,
        },
    )
    .map_err(library_seed_error_for_tests)?;
    Ok((item.id, variant.id))
}

/// One section heading created at the end of the library.
fn heading(tx: &rusqlite::Transaction<'_>, title: &str, level: u8) -> Result<String, AppError> {
    create_item(tx, &NewLibraryItem::Heading { title, level })
        .map(|item| item.id)
        .map_err(library_seed_error_for_tests)
}

/// Starts the real watch service behind the app and waits for its
/// startup pass — the service must be live before any sync (a run
/// awaits the after-sync refresh's ack, and that await carries no
/// budget — the D-137 hang note; a child whose run completes without
/// parking must finish normally, not hang at that await).
fn go_live(
    app: &tauri::App<MockRuntime>,
    bus: tauri::async_runtime::Receiver<crate::file_watch::Command>,
) {
    file_watch::spawn_service(app.handle().clone(), bus, DEBOUNCE).expect("the watcher must start");
    let passes = app.state::<FileWatchState>();
    wait_until("the watch service's startup pass", || {
        passes.pass_count() >= 1
    });
}

/// Runs the real `sync_run` command — the production seams, the whole
/// §9.4 pipeline — and answers the finished run's summary.
fn run_sync(world: &World) -> SyncRunSummary {
    let SyncRunOutcome::Completed { summary } =
        tauri::async_runtime::block_on(crate::commands::sync_run(world.app.handle().clone(), None))
            .expect("the run must answer Ok")
    else {
        panic!("the run must complete, not ask for the empty-build confirmation");
    };
    summary
}

/// The per-project statuses of a summary, in `sort_order`.
fn statuses(summary: &SyncRunSummary) -> Vec<&str> {
    summary
        .projects
        .iter()
        .map(|entry| entry.status.as_str())
        .collect()
}

/// One library operation, committed on its own through the `Db::run`
/// executor — the way the library commands write.
fn library_write<T: Send + 'static>(
    world: &World,
    operation: impl FnMut(&rusqlite::Transaction<'_>) -> Result<T, AppError> + Send + 'static,
) -> T {
    let database = database_of(world);
    tauri::async_runtime::block_on(database.run(operation))
        .expect("the library operation must commit")
}

fn database_of(world: &World) -> Db {
    world.app.state::<Db>().inner().clone()
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).expect("the file must be readable")
}

fn text_of(bytes: &[u8], what: &str) -> String {
    String::from_utf8(bytes.to_owned())
        .unwrap_or_else(|_| panic!("{what}: the app writes UTF-8 (D-22)"))
}

fn write(path: &Path, contents: impl AsRef<[u8]>) {
    fs::write(path, contents).expect("writing the file must work")
}

/// The sorted entry names of a folder.
fn entries_of(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("the folder must be listable")
        .map(|entry| {
            entry
                .expect("every entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// Backdates a file's modification time (the fs suite's own
/// technique): opened for write, because Windows' `SetFileTime` needs
/// `FILE_WRITE_ATTRIBUTES` — the reason `temp_files`' own tests open
/// their fixtures the same way.
fn backdate(path: &Path, age: Duration) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("opening the debris for backdating must work");
    file.set_times(fs::FileTimes::new().set_modified(SystemTime::now() - age))
        .expect("setting the debris's modification time must work");
}

/// Kills and reaps the child on drop — a parent that panics between
/// the spawn and the kill must not leak a parked process.
struct ReapOnDrop(Child);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Waits for the child to park inside the kill window: the marker
/// appears after the run has provably parked, so its existence IS the
/// window being open. An early child exit fails fast with the exit
/// status (a run that completed without parking — the fixture is
/// broken, and the failure names it instead of timing out).
fn wait_for_park(child: &mut ReapOnDrop, marker: &Path) {
    let deadline = Instant::now() + HANDSHAKE;
    loop {
        if marker.exists() {
            return;
        }
        match child.0.try_wait() {
            Ok(Some(status)) => {
                panic!("the child exited before parking at the kill window: {status}")
            }
            Ok(None) => {}
            Err(error) => panic!("reaping the child failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the child to park at the kill window"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// The force kill itself — §15.1's row: `SIGKILL` on Unix,
/// `TerminateProcess` on Windows, the one portable primitive — and
/// the reap. The Unix leg asserts the signal number (the row's own
/// literal); every leg asserts the child did not exit cleanly (a
/// force-terminated process never does).
fn kill_and_reap(child: &mut ReapOnDrop) {
    child.0.kill().expect("the force kill must be deliverable");
    let status = child.0.wait().expect("the killed child must be reaped");
    assert!(
        !status.success(),
        "a force-killed process cannot report a clean exit"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        assert_eq!(
            status.signal(),
            Some(9),
            "§15.1: the Unix kill is SIGKILL, not a graceful stop"
        );
    }
}

/// Polls `condition` until it holds; panics after 30 s (the sibling
/// suite's bound — a failure ceiling, not a duration).
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// One project row, read straight back.
fn project_row(world: &World, index: usize) -> Project {
    let database = database_of(world);
    let project_id = world.project(index).id.clone();
    let row = tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::get(tx, &project_id).map_err(projects_store_to_app_error)
    }))
    .expect("reading the project must work");
    row.expect("the seeded project must persist")
}

/// A project's `last_synced_*` cells — OUT-020's observable.
fn stamps(world: &World, index: usize) -> (Option<String>, Option<String>) {
    let project = project_row(world, index);
    (project.last_synced_at, project.last_synced_hash)
}

/// The newest `sync_runs` row in insertion order (`rowid` — a plain
/// table): the run under test, read raw so the status literals are
/// the oracle, not decoded DTOs — `(id, status, started_at,
/// finished_at, error_code)`. The `error_code` cell is D-147's
/// `interrupted`-row ruling: a process death is nobody's recorded
/// decision, so the corpse and the marked row both carry NULL ("" is
/// exclusive to the two deliberate user-shaped stops, and the
/// census over those spellings is the ipc suite's pin).
fn newest_run_row(world: &World) -> (String, String, String, Option<String>, Option<String>) {
    let database = database_of(world);
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.query_row(
            "SELECT id, status, started_at, finished_at, error_code FROM sync_runs
             ORDER BY rowid DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .map_err(sqlite_error_to_app_error)
    }))
    .expect("reading the newest run row must work")
}

/// One run's per-project status literals, in `sort_order` — the rows
/// the panel reads back for a run whose command never answered.
fn run_project_statuses(world: &World, run_id: &str) -> Vec<&'static str> {
    let database = database_of(world);
    let run_id = run_id.to_owned();
    let rows = tauri::async_runtime::block_on(
        database.run(move |tx| list_projects(tx, &run_id).map_err(sync_runs_app_error_for_tests)),
    )
    .expect("reading the run's project rows must work");
    rows.iter().map(|row| row.status.as_str()).collect()
}

/// MAIN-027's overall time: the `finished_at` of the last `success`
/// run — the read the panel serves between runs.
fn last_success(world: &World) -> Option<String> {
    let database = database_of(world);
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::sync_runs::last_success_finished_at(tx)
            .map_err(sync_runs_app_error_for_tests)
    }))
    .expect("reading the last successful sync must work")
}

/// The startup `interrupted` recovery, driven the way production
/// drives it: `Db::mark_interrupted_runs` with `now_utc` — the exact
/// call `start_database_session` (lib.rs's session tail) makes. The
/// parent process IS the next process over the crashed one; the
/// startup wiring itself is D-147's marker pin. Answers the recovery
/// stamp it passed, and asserts exactly the one corpse row was taken.
fn mark_interrupted_runs_the_way_startup_does(world: &World) -> String {
    let database = database_of(world);
    let stamp = eunomia_store::values::now_utc();
    let taken = database
        .mark_interrupted_runs(&stamp)
        .expect("the startup sweep must work");
    assert_eq!(
        taken, 1,
        "the sweep took exactly the killed run's corpse row (§9.4 recovery)"
    );
    stamp
}

/// Splits a whole scheme-A target (or staged whole file) into
/// `(managed, marker)` — the sibling suite's helper, verbatim.
fn split_whole_file(bytes: &[u8], what: &str) -> (String, String) {
    split_closing_marker(&text_of(bytes, what), "\n", what)
}

fn split_closing_marker(text: &str, line_break: &str, what: &str) -> (String, String) {
    let body = text
        .strip_suffix(line_break)
        .unwrap_or_else(|| panic!("{what}: must end with one line break, got {text:?}"));
    assert!(
        !body.ends_with(line_break),
        "{what}: exactly one line break at the end, got {text:?}"
    );
    let (before, marker) = body.rsplit_once(line_break).unwrap_or_else(|| {
        panic!("{what}: the marker line must follow the managed block, got {text:?}")
    });
    let managed = before.strip_suffix(line_break).unwrap_or_else(|| {
        panic!(
            "{what}: exactly one empty line between the managed block and the marker, got {text:?}"
        )
    });
    (managed.to_owned(), marker.to_owned())
}

/// The marker line's shape: ALG-032's fixed frame. The field-by-field
/// validation (date/time/zone) is the sibling suite's pin; the kill
/// tests only need "this is the app's marker line".
fn assert_marker_shape(marker: &str, what: &str) {
    assert!(
        marker.starts_with(MARKER_PREFIX) && marker.ends_with(MARKER_SUFFIX),
        "{what}: ALG-032's frame, got {marker:?}"
    );
}

/// Every backup copy kept for one project — the files under
/// `backups/<project_id>__<slug>/` (§5.5), oldest first.
fn backup_bytes(world: &World, index: usize) -> Vec<Vec<u8>> {
    fn collect(folder: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(folder).expect("the backups tree must be listable") {
            let path = entry.expect("every entry must be readable").path();
            if path.is_dir() {
                collect(&path, found);
            } else {
                found.push(path);
            }
        }
    }
    let root = world.data_root().join("backups");
    let prefix = format!("{}__", world.project(index).id);
    let mut paths = Vec::new();
    if root.is_dir() {
        for entry in fs::read_dir(&root).expect("the backups tree must be listable") {
            let entry = entry.expect("every entry must be readable");
            if entry.file_name().to_string_lossy().starts_with(&prefix) && entry.path().is_dir() {
                collect(&entry.path(), &mut paths);
            }
        }
    }
    paths.sort();
    paths.iter().map(|path| read(path)).collect()
}

// ---- the kill rows ----------------------------------------------------------

#[test]
fn a_kill_after_the_temp_file_is_written_before_the_rename_leaves_the_original_intact_and_the_next_sync_deletes_the_temp_file()
 {
    // T-F2, verbatim: "The process is killed after the temp file is
    // written, before `rename`" → "The original file is intact; on the
    // next sync, the temp file is deleted". The kill is real
    // (NFR-024: the process dies with `SIGKILL`/`TerminateProcess`
    // inside [`PausePoint::StagingSettled`] — the window eunomia-fs's
    // own comment names as T-F2's), and the assertions are the two
    // halves of the Expected cell plus the crash-state rows every kill
    // shares: the corpse `running` row, the startup marking's
    // `interrupted`.
    if running_as_kill_child() {
        run_child_role();
    }
    const SELF: &str = "a_kill_after_the_temp_file_is_written_before_the_rename_leaves_the_original_intact_and_the_next_sync_deletes_the_temp_file";

    // The world: one scheme-A project whose target exists with
    // hand-written bytes — the "original" of the Expected cell, an
    // oracle spelled here, not derived from the build.
    let mut world = build(1);
    const OLD_BYTES: &[u8] = b"Hand-written notes.\nKept by the crash.\n";
    write(&world.project(0).target, OLD_BYTES);
    seed_standard_library(&world);
    go_live(
        &world.app,
        world.bus.take().expect("a world goes live exactly once"),
    );

    // The child: the real command, parked after the staging file is
    // written and settled, before the rename.
    let marker = world.data_root().join("kill-parked.marker");
    let mut child = ReapOnDrop(spawn_kill_child(&ChildSpec {
        test: self_test_path(SELF),
        data_root: world.data_root().to_path_buf(),
        site: ParkSite::PausePoint {
            point: PausePoint::StagingSettled,
            target: world.project(0).target.clone(),
        },
        marker: marker.clone(),
    }));
    wait_for_park(&mut child, &marker);
    kill_and_reap(&mut child);

    // The Expected cell's first half: the original is intact — §9.5's
    // atomicity under a real process death (OUT-007/008, ERR-002):
    // the rename never happened, so the old bytes are whole.
    assert_eq!(
        read(&world.project(0).target),
        OLD_BYTES,
        "T-F2: the original file is intact"
    );
    // And exactly one sweepable staging file is left — the debris the
    // next sync must delete (D-088's grammar, matched as the sweep
    // matches it).
    let debris: Vec<String> = entries_of(&world.project(0).root)
        .into_iter()
        .filter(|name| is_app_tmp_file(name, "AGENTS.md"))
        .collect();
    assert_eq!(
        debris.len(),
        1,
        "exactly one staging debris file, matched by the sweep's own grammar"
    );
    let debris_path = world.project(0).root.join(&debris[0]);
    // The window's own position: the parked staging file is settled
    // with the FULL new content (the write completed, the rename did
    // not) — the fs suite's position pin, read here at run level over
    // the composed seam.
    let (staged_body, staged_marker) = split_whole_file(&read(&debris_path), "the staging debris");
    assert_eq!(
        staged_body, BODY,
        "the staging file is settled with the whole build"
    );
    assert_marker_shape(&staged_marker, "the staging file's marker");

    // The crash state: phase 0's `running` row is the corpse the
    // startup sweep owns (§9.4 "recovery after a crash").
    let (_, status, started_at, finished_at, error_code) = newest_run_row(&world);
    assert_eq!(status, "running", "the killed run's row is still `running`");
    assert!(finished_at.is_none(), "a corpse row has no finish stamp");
    assert_eq!(
        error_code, None,
        "D-147: a process death is nobody's recorded decision"
    );

    // The startup marking — the next process's recovery, driven the
    // way production drives it: exactly this row, `interrupted`, with
    // the recovery moment D-147 stamps.
    let recovery_stamp = mark_interrupted_runs_the_way_startup_does(&world);
    let (_, status, run_started, finished_at, error_code) = newest_run_row(&world);
    assert_eq!(
        status, "interrupted",
        "§9.4: the corpse is marked `interrupted`"
    );
    assert_eq!(
        run_started, started_at,
        "the marking touches no row but the status and the finish stamp"
    );
    assert_eq!(
        error_code, None,
        "D-147: the interrupted row names no §10 code"
    );
    assert_eq!(
        finished_at.as_deref(),
        Some(recovery_stamp.as_str()),
        "D-147: the recovery moment is the row's `finished_at`"
    );

    // The Expected cell's second half: "on the next sync, the temp
    // file is deleted". The next sync that can delete debris is the
    // one after it crosses §9.4 phase 1 step 7's one-hour window
    // (D-088 — the window exists so a live write's staging file is
    // never a deletion candidate); the hour is simulated by
    // backdating the debris's mtime, and the next sync is the REAL
    // command in this, the next process.
    backdate(&debris_path, DEBRIS_AGE);
    let second = run_sync(&world);
    assert_eq!(
        second.verdict, "success",
        "the run after the crash succeeds"
    );
    assert_eq!(
        entries_of(&world.project(0).root),
        [".git", "AGENTS.md"],
        "the next sync deleted the debris and left nothing else behind"
    );
    assert!(
        !debris_path.exists(),
        "T-F2: the temp file is deleted by the next sync"
    );
    // And the write the crash interrupted is landed: the target now
    // holds the build (the run is idempotent — §10's general rule).
    let (managed, marker_line) =
        split_whole_file(&read(&world.project(0).target), "the written target");
    assert_eq!(
        managed, BODY,
        "the next sync writes what the crash interrupted"
    );
    assert_marker_shape(&marker_line, "the written target's marker");
}

#[test]
fn a_kill_between_writing_project_1_and_project_2_keeps_project_1s_current_stamp_and_marks_the_run_interrupted()
 {
    // T-F3, verbatim: "The process is killed between writing project 1
    // and project 2" → "Project 1 has the current sync timestamp,
    // project 2 has the old one, the run is marked `interrupted`". The
    // window is [`PausePoint::WriteEntry`] armed on project 2's write:
    // project 1 is fully written AND stamped (§9.4 phase 3 step 2's
    // right-away transaction — committed before the kill), project 2's
    // write has not started. §9.4's recovery clause ("the project
    // status already reflects what was written at the time of the
    // crash") is the run rows' half of the same crash.
    if running_as_kill_child() {
        run_child_role();
    }
    const SELF: &str = "a_kill_between_writing_project_1_and_project_2_keeps_project_1s_current_stamp_and_marks_the_run_interrupted";

    // The world: two scheme-A projects, the seeded order is the sync
    // order (§9.4 phase 3 processes `sort_order`).
    let mut world = build(2);
    let xray_variant = seed_standard_library(&world);
    go_live(
        &world.app,
        world.bus.take().expect("a world goes live exactly once"),
    );

    // Run 1 — success: both projects written and stamped. The "old"
    // stamp of the Expected cell is this run's own, captured verbatim;
    // so is project 2's exact file (the next run's comparison bytes).
    let first = run_sync(&world);
    assert_eq!(first.verdict, "success");
    let old_stamps = [stamps(&world, 0), stamps(&world, 1)];
    let old_file_2 = read(&world.project(1).target);
    assert_stamped(
        first.started_at.as_str(),
        old_stamps[1].0.as_deref(),
        "run 1, project 2",
    );

    // The library edit that makes both projects dirty again — the
    // second run carries a different build.
    library_write(&world, move |tx| {
        update_variant(
            tx,
            &xray_variant,
            &VariantUpdate {
                name: None,
                content: Some("Preamble text, revised."),
            },
        )
        .map(|_| ())
        .map_err(library_seed_error_for_tests)
    });

    // The child: the real command, parked at project 2's write entry —
    // between writing project 1 and project 2.
    let marker = world.data_root().join("kill-parked.marker");
    let mut child = ReapOnDrop(spawn_kill_child(&ChildSpec {
        test: self_test_path(SELF),
        data_root: world.data_root().to_path_buf(),
        site: ParkSite::PausePoint {
            point: PausePoint::WriteEntry,
            target: world.project(1).target.clone(),
        },
        marker: marker.clone(),
    }));
    wait_for_park(&mut child, &marker);
    kill_and_reap(&mut child);

    // Project 1: written by the killed run — the CURRENT sync
    // timestamp (phase 3 step 2 stamps right away, and the stamp's
    // transaction committed before the kill) and the revised build.
    let (run_id, status, started_at, _, error_code) = newest_run_row(&world);
    assert_eq!(status, "running", "the killed run's row is the corpse");
    assert_eq!(
        error_code, None,
        "D-147: a process death is nobody's recorded decision"
    );
    let stamp_1 = stamps(&world, 0);
    assert_stamped(
        started_at.as_str(),
        stamp_1.0.as_deref(),
        "the killed run's stamp on project 1",
    );
    assert_ne!(
        stamp_1.0, old_stamps[0].0,
        "project 1 carries the CURRENT run's timestamp, not the old one"
    );
    let (managed_1, marker_1) =
        split_whole_file(&read(&world.project(0).target), "project 1's file");
    assert_eq!(managed_1, BODY_REVISED, "project 1 holds the new build");
    assert_marker_shape(&marker_1, "project 1's marker");

    // Project 2: untouched — the OLD stamp (verbatim equality with
    // run 1's captured pair) and run 1's exact bytes.
    assert_eq!(
        stamps(&world, 1),
        old_stamps[1],
        "project 2 keeps the old timestamp (OUT-020: nothing was written)"
    );
    assert_eq!(
        read(&world.project(1).target),
        old_file_2,
        "project 2's file is exactly what run 1 wrote"
    );

    // §9.4's recovery clause, the run rows' half: the project status
    // already reflects what was written at the time of the crash —
    // project 1 `written` (phase 3's own committed row), project 2
    // still `not_started` (phase 0's own value; the marking owns no
    // project row, D-147).
    assert_eq!(
        run_project_statuses(&world, &run_id),
        ["written", "not_started"],
        "the crash froze the per-project rows where phase 3 left them"
    );

    // The Expected cell's third clause: the run is marked
    // `interrupted` — by the NEXT process's startup, driven the way
    // production drives it.
    let recovery_stamp = mark_interrupted_runs_the_way_startup_does(&world);
    let (_, status, _, finished_at, error_code) = newest_run_row(&world);
    assert_eq!(
        status, "interrupted",
        "T-F3: the run is marked `interrupted`"
    );
    assert_eq!(
        finished_at.as_deref(),
        Some(recovery_stamp.as_str()),
        "D-147: the recovery moment is the row's `finished_at`"
    );
    assert_eq!(
        error_code, None,
        "D-147: the interrupted row names no §10 code"
    );
    // MAIN-027: an interrupted run is no successful sync — the
    // overall time still names run 1.
    assert_eq!(
        last_success(&world),
        first.finished_at,
        "the interrupted run does not move the overall time (§9.4 phase 4 step 4)"
    );
}

#[test]
fn a_kill_between_the_backup_phase_and_the_write_touches_no_target_file() {
    // NFR-024's own sentence — "for each step of pipeline phases 2–3,
    // there is a test that force-kills the process" — for the step
    // boundary no §15.2 row names: between phases (§15.1's "between
    // phases" phrase), phase 2's backups committed, phase 3's first
    // write not started. The window is [`PausePoint::WriteEntry`]
    // armed on project 1's write — nothing of the write has touched
    // the disk (eunomia-fs's own comment for the point). File
    // integrity is the NFR's own check: every target byte-identical,
    // no staging debris anywhere, the phase-2 copies present, the run
    // `interrupted` after the startup marking.
    if running_as_kill_child() {
        run_child_role();
    }
    const SELF: &str = "a_kill_between_the_backup_phase_and_the_write_touches_no_target_file";

    // The world: two scheme-A projects with hand-written targets —
    // the phase-2 backups have bytes to copy, and the "not touched"
    // oracle is spelled here.
    let mut world = build(2);
    const OLD_1: &[u8] = b"Project one's own notes.\n";
    const OLD_2: &[u8] = b"Project two's own notes.\n";
    write(&world.project(0).target, OLD_1);
    write(&world.project(1).target, OLD_2);
    seed_standard_library(&world);
    go_live(
        &world.app,
        world.bus.take().expect("a world goes live exactly once"),
    );

    // The child: the real command, parked at project 1's write entry —
    // the between-phases boundary.
    let marker = world.data_root().join("kill-parked.marker");
    let mut child = ReapOnDrop(spawn_kill_child(&ChildSpec {
        test: self_test_path(SELF),
        data_root: world.data_root().to_path_buf(),
        site: ParkSite::PausePoint {
            point: PausePoint::WriteEntry,
            target: world.project(0).target.clone(),
        },
        marker: marker.clone(),
    }));
    wait_for_park(&mut child, &marker);
    kill_and_reap(&mut child);

    // File integrity: both targets byte-identical, no debris in
    // either repository (nothing of the write started), and phase 2's
    // committed work is on disk — one copy of each original.
    assert_eq!(
        read(&world.project(0).target),
        OLD_1,
        "phase 2 touches no target file"
    );
    assert_eq!(
        read(&world.project(1).target),
        OLD_2,
        "phase 2 touches no target file"
    );
    for index in 0..2 {
        let debris: Vec<String> = entries_of(&world.project(index).root)
            .into_iter()
            .filter(|name| is_app_tmp_file(name, "AGENTS.md"))
            .collect();
        assert!(
            debris.is_empty(),
            "project {index}: the write never started, so no staging debris exists"
        );
    }
    assert_eq!(
        backup_bytes(&world, 0),
        [OLD_1],
        "phase 2 step 1: the backup of project 1 is committed work"
    );
    assert_eq!(
        backup_bytes(&world, 1),
        [OLD_2],
        "phase 2 step 1: the backup of project 2 is committed work"
    );

    // The crash state and the startup marking, the shared shape: the
    // corpse row goes `interrupted`, the projects keep their unstamped
    // OUT-020 state, and the run's rows sit where phase 0 left them.
    let (run_id, status, _, _, error_code) = newest_run_row(&world);
    assert_eq!(status, "running", "the killed run's row is the corpse");
    assert_eq!(
        error_code, None,
        "D-147: a process death is nobody's recorded decision"
    );
    assert_eq!(
        run_project_statuses(&world, &run_id),
        ["not_started", "not_started"],
        "phase 0's own rows: nothing was written"
    );
    assert_eq!(
        stamps(&world, 0),
        (None, None),
        "OUT-020: a project nothing was written to keeps its sync state"
    );
    assert_eq!(
        stamps(&world, 1),
        (None, None),
        "OUT-020: a project nothing was written to keeps its sync state"
    );
    let recovery_stamp = mark_interrupted_runs_the_way_startup_does(&world);
    let (_, status, _, finished_at, error_code) = newest_run_row(&world);
    assert_eq!(
        status, "interrupted",
        "the between-phases kill is marked `interrupted`"
    );
    assert_eq!(
        finished_at.as_deref(),
        Some(recovery_stamp.as_str()),
        "D-147: the recovery moment is the row's `finished_at`"
    );
    assert_eq!(
        error_code, None,
        "D-147: the interrupted row names no §10 code"
    );
}

#[test]
fn a_kill_between_the_backup_copy_and_its_record_touches_no_target_file() {
    // NFR-024's own sentence — "for each step of pipeline phases
    // 2–3, there is a test that force-kills the process" — for the
    // step the three windows above bracket but never enter: phase 2
    // step 1 itself (§9.4: "copy the file to the backups folder
    // (7.6), check the size and SHA-256, and add a record to
    // `backup_records`" — one step, three sub-steps). The window is
    // [`ParkSite::AfterBackup`]: the backup seam's closure parks
    // AFTER the production copy returned Ok (the copy landed on disk
    // and was checked) and BEFORE phase 2's ONE record transaction —
    // phase 2's own module docs name the crash shape ("the copies
    // such a failure orphaned", "no records and a `running` row").
    // File integrity is NFR-024's check: every target byte-identical;
    // the crash state is the copy-without-its-record and the corpse
    // row the startup marking owns.
    if running_as_kill_child() {
        run_child_role();
    }
    const SELF: &str = "a_kill_between_the_backup_copy_and_its_record_touches_no_target_file";

    // The world: two scheme-A projects with hand-written targets —
    // phase 2 has bytes to copy, and the "not touched" oracle is
    // spelled here.
    let mut world = build(2);
    const OLD_1: &[u8] = b"Project one's own notes.\n";
    const OLD_2: &[u8] = b"Project two's own notes.\n";
    write(&world.project(0).target, OLD_1);
    write(&world.project(1).target, OLD_2);
    seed_standard_library(&world);
    go_live(
        &world.app,
        world.bus.take().expect("a world goes live exactly once"),
    );

    // The child: the real command, parked inside phase 2 — after
    // project 1's copy landed, before any record was written.
    let marker = world.data_root().join("kill-parked.marker");
    let mut child = ReapOnDrop(spawn_kill_child(&ChildSpec {
        test: self_test_path(SELF),
        data_root: world.data_root().to_path_buf(),
        site: ParkSite::AfterBackup {
            target: world.project(0).target.clone(),
        },
        marker: marker.clone(),
    }));
    wait_for_park(&mut child, &marker);
    kill_and_reap(&mut child);

    // File integrity: phase 2 only copies — the §9.5 write (phase 3
    // step 1, OUT-007) is the first thing that touches a target, and
    // the crash never reached it.
    assert_eq!(
        read(&world.project(0).target),
        OLD_1,
        "the backup phase touches no target file"
    );
    assert_eq!(
        read(&world.project(1).target),
        OLD_2,
        "the backup phase touches no target file"
    );
    // The crash state, the copy's half: project 1's copy is ON DISK
    // (step 1's committed-to-disk work), project 2's was never
    // reached (the copy loop froze at project 1's iteration).
    assert_eq!(
        backup_bytes(&world, 0),
        [OLD_1],
        "phase 2 step 1: the copy of project 1 landed before the kill"
    );
    assert!(
        backup_bytes(&world, 1).is_empty(),
        "the copy loop never reached project 2"
    );
    // And the record's half: NO `backup_records` row exists — phase
    // 2's ONE record transaction never ran (a record without its copy
    // cannot exist; the copy without its record is exactly this crash
    // shape).
    assert!(
        backup_records_of(&world, 0).is_empty(),
        "the record transaction never ran: no row for the landed copy"
    );
    assert!(
        backup_records_of(&world, 1).is_empty(),
        "the record transaction never ran"
    );
    // No staging debris anywhere (phase 3 never started).
    for index in 0..2 {
        let debris: Vec<String> = entries_of(&world.project(index).root)
            .into_iter()
            .filter(|name| is_app_tmp_file(name, "AGENTS.md"))
            .collect();
        assert!(
            debris.is_empty(),
            "project {index}: the write phase never started, so no staging debris exists"
        );
    }

    // The corpse row and the startup marking, the shared shape —
    // with D-147's NULL `error_code` on both reads.
    let (run_id, status, _, _, error_code) = newest_run_row(&world);
    assert_eq!(status, "running", "the killed run's row is the corpse");
    assert_eq!(
        error_code, None,
        "D-147: a process death is nobody's recorded decision"
    );
    assert_eq!(
        run_project_statuses(&world, &run_id),
        ["not_started", "not_started"],
        "phase 0's own rows: nothing was written"
    );
    assert_eq!(
        stamps(&world, 0),
        (None, None),
        "OUT-020: a project nothing was written to keeps its sync state"
    );
    assert_eq!(
        stamps(&world, 1),
        (None, None),
        "OUT-020: a project nothing was written to keeps its sync state"
    );
    let recovery_stamp = mark_interrupted_runs_the_way_startup_does(&world);
    let (_, status, _, finished_at, error_code) = newest_run_row(&world);
    assert_eq!(
        status, "interrupted",
        "the mid-phase-2 kill is marked `interrupted`"
    );
    assert_eq!(
        finished_at.as_deref(),
        Some(recovery_stamp.as_str()),
        "D-147: the recovery moment is the row's `finished_at`"
    );
    assert_eq!(
        error_code, None,
        "D-147: the interrupted row names no §10 code"
    );
}

#[test]
fn a_kill_between_the_write_and_its_stamp_leaves_the_file_whole_the_row_unwritten_and_marks_the_run_interrupted()
 {
    // NFR-024's remaining step boundary: phase 3's step 1 → step 2
    // (§9.4: the atomic write, then "status `written`; the project's
    // `last_synced_at` and `last_synced_hash` are updated right away
    // in a separate short transaction"). The window is
    // [`ParkSite::AfterWrite`]: the write seam's closure parks AFTER
    // the production §9.5 call returned Ok — the file wholly written
    // AND renamed, NFR-024's own integrity claim ("does not leave a
    // partly written file") at its most exposed moment — and BEFORE
    // the verdict/stamp transaction. The crash shape is the contrast
    // case of §9.4's recovery clause: the file on disk IS the new
    // build while the run's row still says `not_started` and the
    // project keeps its old stamp — nothing lies, because the file is
    // whole and the next sync converges (§10's idempotence: ALG-033
    // answers `unchanged` for the file the crash already wrote).
    if running_as_kill_child() {
        run_child_role();
    }
    const SELF: &str = "a_kill_between_the_write_and_its_stamp_leaves_the_file_whole_the_row_unwritten_and_marks_the_run_interrupted";

    // The world: two scheme-A projects, a first successful run (the
    // "old" stamps and files), then the library edit the killed run
    // carries — the T-F3 fixture shape, parked one step boundary
    // earlier.
    let mut world = build(2);
    let xray_variant = seed_standard_library(&world);
    go_live(
        &world.app,
        world.bus.take().expect("a world goes live exactly once"),
    );

    let first = run_sync(&world);
    assert_eq!(first.verdict, "success");
    let old_stamps = [stamps(&world, 0), stamps(&world, 1)];
    let old_file_2 = read(&world.project(1).target);
    assert_stamped(
        first.started_at.as_str(),
        old_stamps[1].0.as_deref(),
        "run 1, project 2",
    );

    library_write(&world, move |tx| {
        update_variant(
            tx,
            &xray_variant,
            &VariantUpdate {
                name: None,
                content: Some("Preamble text, revised."),
            },
        )
        .map(|_| ())
        .map_err(library_seed_error_for_tests)
    });

    // The child: the real command, parked after project 1's write
    // returned Ok, before phase 3's verdict/stamp transaction.
    let marker = world.data_root().join("kill-parked.marker");
    let mut child = ReapOnDrop(spawn_kill_child(&ChildSpec {
        test: self_test_path(SELF),
        data_root: world.data_root().to_path_buf(),
        site: ParkSite::AfterWrite {
            target: world.project(0).target.clone(),
        },
        marker: marker.clone(),
    }));
    wait_for_park(&mut child, &marker);
    kill_and_reap(&mut child);

    // File integrity at the step boundary: project 1's file is the
    // NEW build, whole — the §9.5 write completed (the rename
    // happened; a partly written file cannot be this shape) — while
    // project 2 is exactly run 1's bytes.
    let (managed_1, marker_1) =
        split_whole_file(&read(&world.project(0).target), "project 1's file");
    assert_eq!(
        managed_1, BODY_REVISED,
        "the write landed wholly before the kill (NFR-024: no partly written file)"
    );
    assert_marker_shape(&marker_1, "project 1's marker");
    assert_eq!(
        read(&world.project(1).target),
        old_file_2,
        "project 2's file is exactly what run 1 wrote"
    );
    // The crash state: the run's row still says `not_started` (the
    // verdict transaction never ran) and BOTH projects keep their old
    // stamps (phase 3's right-away stamping is step 2, which the kill
    // precedes) — the honest contrast to T-F3's `written` row.
    let (run_id, status, _, _, error_code) = newest_run_row(&world);
    assert_eq!(status, "running", "the killed run's row is the corpse");
    assert_eq!(
        error_code, None,
        "D-147: a process death is nobody's recorded decision"
    );
    assert_eq!(
        run_project_statuses(&world, &run_id),
        ["not_started", "not_started"],
        "the verdict transaction never ran: the rows still say nothing was written"
    );
    assert_eq!(
        stamps(&world, 0),
        old_stamps[0],
        "the stamp transaction never ran: project 1 keeps its old stamp"
    );
    assert_eq!(
        stamps(&world, 1),
        old_stamps[1],
        "project 2 keeps its old stamp"
    );

    // The startup marking, the shared shape.
    let recovery_stamp = mark_interrupted_runs_the_way_startup_does(&world);
    let (_, status, _, finished_at, error_code) = newest_run_row(&world);
    assert_eq!(
        status, "interrupted",
        "the between-write-and-stamp kill is marked `interrupted`"
    );
    assert_eq!(
        finished_at.as_deref(),
        Some(recovery_stamp.as_str()),
        "D-147: the recovery moment is the row's `finished_at`"
    );
    assert_eq!(
        error_code, None,
        "D-147: the interrupted row names no §10 code"
    );
    // MAIN-027: the interrupted run is no successful sync.
    assert_eq!(
        last_success(&world),
        first.finished_at,
        "the interrupted run does not move the overall time (§9.4 phase 4 step 4)"
    );

    // The convergence epilogue: the run after the crash is a normal,
    // successful one — project 1 answers `unchanged` (ALG-033: the
    // crash already wrote this build; D-24's markerless comparison),
    // project 2 is written, and both stamps land (§9.4 phase 4 step 1
    // for the unchanged, phase 3 step 2 for the written). The crash
    // state was safe to re-run against — §10's idempotence rule.
    let second = run_sync(&world);
    assert_eq!(
        second.verdict, "success",
        "§10: a sync after a crash is safe to run again"
    );
    assert_eq!(
        statuses(&second),
        ["unchanged", "written"],
        "the file the crash wrote is `unchanged`; the untouched one is written"
    );
    assert_stamped(
        second.started_at.as_str(),
        stamps(&world, 0).0.as_deref(),
        "the unchanged project 1, stamped by the run after the crash",
    );
    assert_stamped(
        second.started_at.as_str(),
        stamps(&world, 1).0.as_deref(),
        "the written project 2, stamped by the run after the crash",
    );
}

/// One project's whole `backup_records` history — the §6.4 rows phase
/// 2's ONE record transaction writes after every copy lands (the
/// read the Backups tab serves).
fn backup_records_of(world: &World, index: usize) -> Vec<String> {
    let database = database_of(world);
    let project_id = world.project(index).id.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::backup_records::list_for_project(tx, &project_id)
            .map_err(crate::db::backup_records_to_app_error)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| row.status.as_str().to_owned())
                    .collect()
            })
    }))
    .expect("reading the backup records must work")
}

/// OUT-020's stamp window, the crash shape: a `written` project's
/// stamp is DB-002's UTC instant at or after the run's own start —
/// phase 3 captures it when the write phase opens, phase 0 recorded
/// `started_at` before that, so the killed run's stamp lies between
/// the two clocks, and no later clock (the recovery stamp, the next
/// run) can fake it.
fn assert_stamped(run_started: &str, stamp: Option<&str>, what: &str) {
    let Some(stamp) = stamp else {
        panic!("{what}: the project was never stamped");
    };
    assert_eq!(
        stamp.len(),
        24,
        "{what}: DB-002's `YYYY-MM-DDTHH:MM:SS.sssZ`, got {stamp:?}"
    );
    Timestamp::from_str(stamp)
        .unwrap_or_else(|error| panic!("{what}: DB-002's shape, got {stamp:?}: {error}"));
    assert!(
        run_started <= stamp,
        "{what}: the stamp names the run's own time, got {stamp:?} for a run started {run_started:?}"
    );
}
