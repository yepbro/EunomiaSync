//! Behavior tests of the sync pipeline's phase 3 (T-116, §9.4 phase
//! 3, §9.3's ALG-030/031/032 assembly, §9.5 through the real fs
//! operation, §10's four write rows, DB-024/OUT-020).
//!
//! The suite drives the phase the way production reaches it —
//! [`sync_phase3::run`] over a real phase-1 outcome produced by
//! [`sync_phase0::run`] + [`sync_phase1::run`], with phase 2 run for
//! real first (§9.4's own order) — against REAL project folders in a
//! temp root, so every successful write is `eunomia-fs`'s own
//! seven-step §9.5 pipeline answering over a real fixture. The
//! failure points are injected through the phase's `write` seam (the
//! D-089/phase-2 pattern): a stateful closure delegates to the REAL
//! [`sync_phase3::production_write`] until the failing call, then
//! answers with the typed error.
//!
//! Pinned behavior:
//!
//! * the success path: every `ready` project's file holds the final
//!   bytes byte for byte — the managed text, one empty line, the
//!   run's ONE marker, one break (ALG-030), or the ALG-031 splice for
//!   scheme B with the outside preserved — the row is `written`, the
//!   project's `last_synced_at` is the run's one DB-002 instant and
//!   `last_synced_hash` is the run's own DB-020 fingerprint (NOT the
//!   digest of the written bytes), and the run row stays `running`
//!   (phase 4 owns the verdict);
//! * the T-111 criterion: every file of the run carries the SAME
//!   marker line, byte for byte;
//! * a refused write (an injected pipeline error, an injected
//!   E_VERIFY_FAILED hash mismatch) lands `failed` with its §10 code
//!   and class-only message while the OTHER projects continue, the
//!   failed project's file and `last_synced_*` keep their previous
//!   values, and the backup copy stays;
//! * an empty `ready` list writes nothing at all (no write call, no
//!   verdict, the phase-1 rows untouched).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_core::hash::Sha256Hex;
use eunomia_core::marker_clock::MarkerClock;
use eunomia_store::projects::NewProject;
use eunomia_store::settings::BaseName;
use eunomia_store::sync_runs::{SyncRunProjectStatus, SyncRunStatus};

use crate::data_dirs;
use crate::db::Db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_phase0;
use crate::sync_phase1::{self, Phase1Outcome};
use crate::sync_phase2;
use crate::sync_phase3::{self, Phase3Outcome, WriteOutcome};
use crate::sync_running::SyncRunning;

/// The run's fixed write instant — 2026-11-21T10:30:00.123Z. One
/// value pins two spellings: the marker line (Prague civil time,
/// +01:00, second-granular) and the `last_synced_at` DB-002 stamp.
fn fixed_now() -> jiff::Timestamp {
    "2026-11-21T10:30:00.123Z"
        .parse()
        .expect("the fixed instant must parse")
}

/// The run's ONE marker line, pinned as a literal: 10:30:00.123Z is
/// 11:30:00 in Prague (winter, +01:00), sub-second precision drops
/// (MarkerTime is second-granular) — the ALG-032 example's shape at
/// this suite's instant.
const MARKER_LINE: &str =
    "<!-- Created by EunomiaSync 2026-11-21 11:30:00 Europe/Prague +01:00 -->";

/// The `last_synced_at` every written project of the run shares (the
/// fixed instant's DB-002 spelling).
const LAST_SYNCED_AT: &str = "2026-11-21T10:30:00.123Z";

// ---- the harness ------------------------------------------------------

/// A mock app in the healthy state the phase serves (the
/// `sync_phase2_tests.rs` harness shape: mock runtime, a fresh
/// database in a temp data root, the `SyncRunning` flag managed),
/// plus the `repos/` root the fixtures live under.
fn phase3_app() -> (
    tempfile::TempDir,
    tauri::App<tauri::test::MockRuntime>,
    PathBuf,
) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database =
        Db::open(temp.path()).expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(SyncRunning::new());
    let repos = temp.path().join("repos");
    fs::create_dir(&repos).expect("the repos root must be creatable");
    (temp, app, repos)
}

/// The managed database handle.
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> Db {
    app.state::<Db>().inner().clone()
}

/// The assertion-side error mapping for run-repository reads (only
/// `Sql` is reachable on a database this suite just wrote).
fn runs_app_error(error: eunomia_store::sync_runs::SyncRunsError) -> AppError {
    match error {
        eunomia_store::sync_runs::SyncRunsError::Sql(sql) => {
            crate::db::sqlite_error_to_app_error(sql)
        }
        other => {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("SyncRunsRead".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// The run repository's row through the app's database.
fn run_row_of(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> eunomia_store::sync_runs::SyncRun {
    let database = database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::sync_runs::get(tx, &run_id)
            .map_err(runs_app_error)
            .map(|row| row.expect("the run row this suite recorded must persist"))
    }))
    .expect("reading the run row must work")
}

/// The run repository's per-project rows, in `sort_order`.
fn run_rows_of(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> Vec<eunomia_store::sync_runs::SyncRunProject> {
    let database = database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::sync_runs::list_projects(tx, &run_id).map_err(runs_app_error)
    }))
    .expect("reading the run's rows must work")
}

/// One stored project row, read straight back.
fn project_of(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
) -> eunomia_store::projects::Project {
    let database = database_of(app);
    let project_id = project_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::get(tx, &project_id)
            .map_err(crate::commands::projects_store_to_app_error)
            .map(|row| row.expect("the seeded project must persist"))
    }))
    .expect("reading the project must work")
}

/// Creates a real repository folder (`repos/repo-<n>` with a `.git`
/// directory) and seeds its project row — the phase-2 harness shape.
/// Returns `(project_id, repo_path)`.
fn seed_repo_project(
    app: &tauri::App<tauri::test::MockRuntime>,
    repos: &Path,
    n: usize,
    scheme: eunomia_store::settings::Scheme,
    base_name: BaseName,
) -> (String, PathBuf) {
    let repo = repos.join(format!("repo-{n}"));
    fs::create_dir(&repo).expect("the repository folder must be creatable");
    fs::create_dir(repo.join(".git")).expect("the .git directory must be creatable");
    let canonical =
        eunomia_fs::paths::canonicalize(&repo).expect("canonicalizing the fresh repo must work");
    let path = Box::leak(canonical.to_string_lossy().into_owned().into_boxed_str());
    let id = tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path,
                path_key: path,
                name: Box::leak(format!("Project {n}").into_boxed_str()),
                label: Box::leak(format!("P{n}").into_boxed_str()),
                color: "blue",
                description: "",
                added_via: eunomia_store::projects::AddedVia::Manual,
                scheme,
                base_name,
                extra_local: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
                extra_project: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
            },
        )
        .map(|project| project.id)
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work");
    (id, canonical)
}

/// Maps a library seeding failure onto the wire shape the executor
/// needs (the phase-2 harness shape).
fn library_seed_error(error: eunomia_store::library::LibraryError) -> AppError {
    match error {
        eunomia_store::library::LibraryError::Sql(sql) => crate::db::sqlite_error_to_app_error(sql),
        other => {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("LibrarySeed".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// Seeds one active instruction with its main variant and assigns it
/// to the project: the project's built managed text is then `content`
/// verbatim (the phase-2 harness shape, process-unique titles).
fn seed_assigned_instruction(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
    content: &'static str,
) {
    static SEEDED: AtomicUsize = AtomicUsize::new(0);
    let seeded = SEEDED.fetch_add(1, Ordering::Relaxed);
    let title: &'static str = Box::leak(format!("Be brief {seeded}").into_boxed_str());
    let project_id = project_id.to_owned();
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Instruction { title },
        )
        .map_err(library_seed_error)?;
        let variant = eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content,
                is_main: true,
            },
        )
        .map_err(library_seed_error)?;
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &item.id,
                variant_id: &variant.id,
            },
        )
        .map_err(library_seed_error)?;
        Ok(())
    }))
    .expect("seeding the assigned instruction must work");
}

/// Runs phases 0 and 1 back to back (the plenty-everywhere probe, the
/// real clock) and drops phase 0's flag lease — the composition every
/// phase-3 drive starts from.
fn preflight_ok(app: &tauri::App<tauri::test::MockRuntime>) -> Phase1Outcome {
    let phase0 = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over the seeded database");
    let outcome = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        || Ok(u64::MAX),
        SystemTime::now(),
    ))
    .expect("phase 1 must pass");
    drop(phase0);
    outcome
}

/// Runs phase 2 for real (§9.4's own order before the write) with the
/// suite's fixed instant.
fn run_phase2(app: &tauri::App<tauri::test::MockRuntime>, phase1: &Phase1Outcome, data_dir: &Path) {
    tauri::async_runtime::block_on(sync_phase2::run(
        app.handle(),
        phase1,
        data_dir,
        eunomia_fs::backup::backup_target_file,
        fixed_now(),
    ))
    .expect("phase 2 must succeed over the healthy fixtures");
}

/// The run's fixed marker clock: the fixed instant in Prague.
fn fixed_marker_clock() -> MarkerClock {
    MarkerClock::from_instant(
        fixed_now(),
        jiff::tz::TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
    )
}

/// The run's marker clock seam for the success-path drive: a
/// SEQUENCED capture (the round-1 P3-1 fix). The first call returns
/// the fixed Prague instant — the one [`MARKER_LINE`] spells — and
/// every later call would return a LATER minute, so a phase that
/// re-captured the clock per project (instead of the once-per-run
/// ALG-032/T-111 rule) writes files whose marker lines disagree and
/// fails the byte-exact assertions; the counter's 1 pins that the
/// capture happened exactly once. Returns the clock closure and the
/// shared capture counter. The closure's type is spelled `Fn` (it
/// mutates only through the shared `Arc`, and an `Fn` is a valid
/// `FnOnce` at the phase's seam) so the exact refactor this pin
/// guards against — widening the seam and re-capturing per project —
/// compiles and dies on the assertions instead of on the bound.
fn sequenced_marker_clock() -> (impl Fn() -> MarkerClock + Send, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let clock = move || {
        let capture = calls.fetch_add(1, Ordering::Relaxed);
        let instant = fixed_now()
            + jiff::Span::new().minutes(i64::try_from(capture).expect("the counter is tiny"));
        MarkerClock::from_instant(
            instant,
            jiff::tz::TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
        )
    };
    (clock, counted)
}

/// Drives phase 3 with the REAL §9.5 operation and the fixed clock.
fn run_phase3(
    app: &tauri::App<tauri::test::MockRuntime>,
    phase1: &Phase1Outcome,
) -> Result<Phase3Outcome, AppError> {
    tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        phase1,
        sync_phase3::production_write,
        fixed_marker_clock,
        fixed_now(),
    ))
}

/// A stateful injected write operation: delegates to the REAL
/// [`sync_phase3::production_write`] until the `fail_at`-th call
/// (1-based), then answers with `error` (the phase-2 injector shape).
fn injected_write(fail_at: usize, error: eunomia_fs::atomic::AtomicWriteError) -> WriteOp {
    let calls = AtomicUsize::new(0);
    // The one-shot payload rides a `Mutex<Option<_>>` because the
    // phase calls through `Fn` (the injector fails once and then
    // only delegates — the phase-2 shape).
    let error = std::sync::Mutex::new(Some(error));
    Box::new(move |target: &Path, contents: &[u8]| {
        if calls.fetch_add(1, Ordering::Relaxed) + 1 == fail_at {
            return Err(error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .expect("the injector fails at most once"));
        }
        sync_phase3::production_write(target, contents)
    })
}

/// The write operation's type — the phase's `write` seam, as the
/// injectors hand it over (a boxed closure satisfies the phase's
/// `Fn + Send + Sync + 'static` bound through `Box`'s own `Fn` impl).
type WriteOp = Box<
    dyn Fn(
            &Path,
            &[u8],
        ) -> Result<eunomia_fs::atomic::WrittenFile, eunomia_fs::atomic::AtomicWriteError>
        + Send
        + Sync,
>;

/// A counting-only write operation: delegates to the real one and
/// counts every call — the empty-ready-list sentinel (the counter
/// staying at zero proves the phase never asked for a write).
fn counting_write() -> (WriteOp, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let operation = move |target: &Path, contents: &[u8]| {
        calls.fetch_add(1, Ordering::Relaxed);
        sync_phase3::production_write(target, contents)
    };
    (Box::new(operation), counted)
}

/// Every `.bak` file anywhere under the data root's `backups/` tree
/// (the phase-2 suite's `every_copy` view).
fn every_bak(data_dir: &Path) -> Vec<PathBuf> {
    fn walk(folder: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "bak") {
                out.push(path);
            }
        }
    }
    let mut copies = Vec::new();
    walk(&data_dir.join("backups"), &mut copies);
    copies.sort();
    copies
}

/// The old-sync A/C file form of `managed` (ALG-030) — the file a
/// previous run's write phase left (the phase-2 suite's helper).
fn old_sync_file(managed: &str) -> String {
    if managed.is_empty() {
        format!("{MARKER_LINE}\n")
    } else {
        format!("{managed}\n\n{MARKER_LINE}\n")
    }
}

/// The run's build-hash fingerprint per project, read off the run's
/// own rows BEFORE the write phase — the oracle the stamps must equal
/// (D-130: the fingerprint phase 0 froze, not a recomputation).
fn fingerprints_of(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> Vec<(String, Option<String>)> {
    run_rows_of(app, run_id)
        .into_iter()
        .map(|row| (row.project_id, row.build_hash))
        .collect()
}

// ---- the success path -------------------------------------------------

#[test]
fn ready_projects_are_written_with_the_run_s_one_marker_and_stamped() {
    // §9.4 phase 3 steps 1–2, end to end: both ready projects' files
    // hold the final bytes byte for byte — ALG-030's exact shape
    // around the run's ONE marker — the rows land `written`, and the
    // stamps land right away: `last_synced_at` is the run's one
    // DB-002 instant, `last_synced_hash` is the run's own DB-020
    // fingerprint (NOT the digest of the written bytes — those carry
    // the marker; the ruling this task's D-entry fixes).
    let (temp, app, repos) = phase3_app();
    let (first, first_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (second, second_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::C,
        BaseName::ClaudeMd,
    );
    seed_assigned_instruction(&app, &first, "Be brief.");
    seed_assigned_instruction(&app, &second, "Also be kind.");
    let first_target = first_repo.join("AGENTS.md");
    let second_target = second_repo.join("CLAUDE.project.md");
    // Project 1 is an overwrite; project 2 is the creation case
    // (OUT-005 — a missing C target is written, not skipped).
    fs::write(&first_target, b"the old content").expect("writing the fixture must work");

    let phase1 = preflight_ok(&app);
    run_phase2(&app, &phase1, temp.path());
    let fingerprints = fingerprints_of(&app, &phase1.run_id);
    // The sequenced clock (the round-1 P3-1 fix): a second capture
    // would stamp a later minute, so the byte-exact assertions below
    // hold only if the phase captured the marker ONCE.
    let (clock, captures) = sequenced_marker_clock();
    let outcome = tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        sync_phase3::production_write,
        clock,
        fixed_now(),
    ))
    .expect("phase 3 must succeed");
    assert_eq!(
        captures.load(Ordering::Relaxed),
        1,
        "the marker clock is captured exactly once per run (T-111)"
    );

    // The files: ALG-030's exact shape around the ONE marker line,
    // pinned as literals (never derived from the module's helpers).
    let first_expected = format!("Be brief.\n\n{MARKER_LINE}\n");
    let second_expected = format!("Also be kind.\n\n{MARKER_LINE}\n");
    assert_eq!(
        fs::read(&first_target).expect("reading the target back must work"),
        first_expected.as_bytes(),
        "the overwrite holds exactly the final bytes"
    );
    assert_eq!(
        fs::read(&second_target).expect("the created file must be readable"),
        second_expected.as_bytes(),
        "the creation case writes the file"
    );
    // The T-111 criterion: every file of the run carries the SAME
    // marker line, byte for byte — exactly one occurrence per file,
    // and the identical line in both (both files equal their
    // literals, and the literals share MARKER_LINE, so the marker
    // cannot differ between projects).
    for target in [&first_target, &second_target] {
        let written = fs::read_to_string(target).expect("reading the target back must work");
        assert_eq!(
            written.matches(MARKER_LINE).count(),
            1,
            "exactly one marker, and only the run's one: {written:?}"
        );
    }

    // The outcome: one Written fact per project, in ready order, with
    // the §9.5-verified size and digest of the WRITTEN bytes (an
    // independent hasher over the expected literal is the oracle).
    assert_eq!(outcome.results.len(), 2);
    assert_eq!(outcome.results[0].0, first);
    assert_eq!(outcome.results[1].0, second);
    for (result, expected) in [
        (&outcome.results[0], &first_expected),
        (&outcome.results[1], &second_expected),
    ] {
        let WriteOutcome::Written { size, sha256_hex } = &result.1 else {
            panic!("a successful write must be Written, got {:?}", result.1);
        };
        let mut hasher = Sha256Hex::new();
        hasher.update(expected.as_bytes());
        assert_eq!(*size, expected.len() as u64);
        assert_eq!(*sha256_hex, hasher.finalize_hex());
    }

    // The rows: `written` for both, in sort_order.
    let rows = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows[0].status, SyncRunProjectStatus::Written);
    assert_eq!(rows[1].status, SyncRunProjectStatus::Written);

    // The stamps: the run's one DB-002 instant and the run's own
    // DB-020 fingerprint — and the fingerprint is NOT the written
    // bytes' digest (the concepts must not be substituted).
    let first_hash = fingerprints[0].1.as_deref().expect("phase 0 froze a hash");
    let second_hash = fingerprints[1].1.as_deref().expect("phase 0 froze a hash");
    let stored_first = project_of(&app, &first);
    assert_eq!(stored_first.last_synced_at.as_deref(), Some(LAST_SYNCED_AT));
    assert_eq!(stored_first.last_synced_hash.as_deref(), Some(first_hash));
    let stored_second = project_of(&app, &second);
    assert_eq!(
        stored_second.last_synced_at.as_deref(),
        Some(LAST_SYNCED_AT)
    );
    assert_eq!(stored_second.last_synced_hash.as_deref(), Some(second_hash));
    let WriteOutcome::Written { sha256_hex, .. } = &outcome.results[0].1 else {
        unreachable!("asserted above");
    };
    assert_ne!(
        stored_first.last_synced_hash.as_deref(),
        Some(sha256_hex.as_str()),
        "last_synced_hash is the DB-020 markerless fingerprint, never the \
         digest of the written (marker-carrying) bytes"
    );
    // The sync stamp is not a project edit: updated_at keeps phase 0's
    // world (the row's own creation stamp here).
    assert_eq!(
        stored_first.updated_at, stored_first.created_at,
        "a sync stamp does not restamp updated_at"
    );

    // The run row: still `running` — §9.4's final verdict is phase
    // 4's, and the counters are untouched.
    let run = run_row_of(&app, &phase1.run_id);
    assert_eq!(run.status, SyncRunStatus::Running);
    assert_eq!(run.projects_written, 0, "the counters are phase 4's");
}

#[test]
fn a_scheme_b_splice_preserves_the_outside_and_carries_the_marker() {
    // ALG-031/OUT-016: scheme B's write replaces exactly the managed
    // block between the markers — the outside text (head and tail)
    // survives byte for byte, the block joins in the file's dominant
    // style (LF here), and the run's marker rides inside the fence
    // before the END line. The expected bytes are spelled as a
    // literal (the assembly_b doctest's shape at this suite's
    // managed text), never derived from the splice under test.
    let (temp, app, repos) = phase3_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::B,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "New body.");
    let target = repo.join("AGENTS.md");
    let existing =
        "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nOld body.\n<!-- EUNOMIASYNC:END -->\nTail.\n";
    fs::write(&target, existing).expect("writing the fixture must work");

    let phase1 = preflight_ok(&app);
    run_phase2(&app, &phase1, temp.path());
    run_phase3(&app, &phase1).expect("phase 3 must succeed");

    let expected = format!(
        "Intro.\n<!-- EUNOMIASYNC:BEGIN -->\nNew body.\n\n{MARKER_LINE}\n\
         <!-- EUNOMIASYNC:END -->\nTail.\n"
    );
    assert_eq!(
        fs::read_to_string(&target).expect("reading the target back must work"),
        expected,
        "the splice keeps the outside byte for byte and carries the marker"
    );
    let row = &run_rows_of(&app, &phase1.run_id)[0];
    assert_eq!(row.status, SyncRunProjectStatus::Written);
    assert_eq!(
        project_of(&app, &project).last_synced_at.as_deref(),
        Some(LAST_SYNCED_AT)
    );
}

// ---- the failure path (§9.4 phase 3 step 3) ---------------------------

#[test]
fn a_failed_write_marks_the_row_and_the_run_continues() {
    // The card's criterion: the FIRST project's write refuses; its
    // row lands `failed` with the §10 code and a class-only message,
    // the SECOND project is still written, the failed project's file
    // and last-sync facts keep their previous values, and the backup
    // copy stays on disk (§9.4's own "the backup stays").
    let (temp, app, repos) = phase3_app();
    let (first, first_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (second, second_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &first, "Be brief.");
    seed_assigned_instruction(&app, &second, "Also be kind.");
    let first_target = first_repo.join("AGENTS.md");
    let second_target = second_repo.join("AGENTS.md");
    fs::write(&first_target, b"old first bytes").expect("writing the fixture must work");
    fs::write(&second_target, b"old second bytes").expect("writing the fixture must work");

    let phase1 = preflight_ok(&app);
    run_phase2(&app, &phase1, temp.path());
    // The run's backups: one real verified copy per existing target.
    let backups_before = every_bak(temp.path());
    assert_eq!(backups_before.len(), 2, "phase 2 made both copies");

    // The injection: the FIRST write call answers with the typed
    // replace-refusal (os 13, EACCES) — §10's E_WRITE_FAILED row.
    let first_resolved = eunomia_fs::paths::canonicalize(&first_target)
        .expect("the fixture target must canonicalize");
    let injection = injected_write(
        1,
        eunomia_fs::atomic::AtomicWriteError::WriteFailed(eunomia_fs::atomic::WriteFailed {
            step: eunomia_fs::atomic::WriteStep::Replace,
            path: first_resolved.clone(),
            source: std::io::Error::from_raw_os_error(13),
            cleanup: None,
        }),
    );
    let outcome = tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        injection,
        fixed_marker_clock,
        fixed_now(),
    ))
    .expect("a project's write refusal is a row, never a run error");

    // The outcome: the first project failed, the second written — in
    // ready order (the order phase 1 established).
    assert_eq!(outcome.results.len(), 2);
    assert_eq!(outcome.results[0].0, first);
    assert_eq!(outcome.results[1].0, second);
    let WriteOutcome::Failed {
        code,
        message,
        details,
    } = &outcome.results[0].1
    else {
        panic!(
            "the refused write must be Failed, got {:?}",
            outcome.results[0].1
        );
    };
    assert_eq!(*code, ErrorCode::WriteFailed);
    assert!(
        message.contains(
            first_resolved
                .to_str()
                .expect("the canonical path is UTF-8")
        ),
        "the class-only message carries the path and OS facts: {message}"
    );
    let details = details.as_ref().expect("an OS refusal carries details");
    assert_eq!(
        details.os_code,
        Some(13),
        "the §10 log fact rides the outcome"
    );
    assert!(matches!(
        &outcome.results[1].1,
        WriteOutcome::Written { .. }
    ));

    // The rows: `failed` with the §10 literal and the message, then
    // `written` — the other project continued.
    let rows = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows[0].status, SyncRunProjectStatus::Failed);
    assert_eq!(rows[0].error_code.as_deref(), Some("E_WRITE_FAILED"));
    assert!(
        rows[0]
            .error_message
            .as_deref()
            .is_some_and(|text| text.contains("replacing the target")),
        "the row's message is the typed Display: {:?}",
        rows[0].error_message
    );
    assert_eq!(rows[1].status, SyncRunProjectStatus::Written);

    // The failed project's world is unchanged: the file keeps its old
    // bytes, the last-sync facts stay unset, the fingerprint row
    // stays, and the backup copy stays.
    assert_eq!(
        fs::read(&first_target).expect("reading the target back must work"),
        b"old first bytes",
        "the refused write moved no byte of the target"
    );
    assert_eq!(
        project_of(&app, &first).last_synced_at,
        None,
        "OUT-020: a failed project keeps its previous sync state"
    );
    assert_eq!(
        fs::read(&second_target).expect("reading the target back must work"),
        format!("Also be kind.\n\n{MARKER_LINE}\n").as_bytes(),
        "the second project was written whole"
    );
    assert_eq!(
        project_of(&app, &second).last_synced_at.as_deref(),
        Some(LAST_SYNCED_AT)
    );

    // §9.4's "the backup stays": both copies are still on disk,
    // byte for byte.
    assert_eq!(every_bak(temp.path()), backups_before, "the backup stays");
}

#[test]
fn a_verify_mismatch_fails_the_project_with_e_verify_failed() {
    // §9.5 step 6 / §10's E_VERIFY_FAILED row: an injected digest
    // mismatch (the settled file does not hold what this run wrote)
    // lands `failed` with the code and both digests in the message —
    // and the run still completes around it.
    let (temp, app, repos) = phase3_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    let target = repo.join("AGENTS.md");
    fs::write(&target, b"old bytes").expect("writing the fixture must work");

    let phase1 = preflight_ok(&app);
    run_phase2(&app, &phase1, temp.path());
    let injection = injected_write(
        1,
        eunomia_fs::atomic::AtomicWriteError::VerifyFailed(eunomia_fs::atomic::VerifyFailed {
            path: eunomia_fs::paths::canonicalize(&target)
                .expect("the fixture target must canonicalize"),
            expected_sha256_hex: "0".repeat(64),
            mismatch: eunomia_fs::atomic::VerifyMismatch::Digest {
                actual_sha256_hex: "f".repeat(64),
            },
        }),
    );
    let outcome = tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        injection,
        fixed_marker_clock,
        fixed_now(),
    ))
    .expect("the verify refusal is a row, never a run error");

    let WriteOutcome::Failed { code, details, .. } = &outcome.results[0].1 else {
        panic!(
            "the mismatch must be Failed, got {:?}",
            outcome.results[0].1
        );
    };
    assert_eq!(*code, ErrorCode::VerifyFailed);
    // A digest mismatch is a decision, not an OS failure — no io
    // error exists to carry (§10's os_code column reads absent).
    assert!(details.is_none(), "a digest mismatch has no OS fact");

    let rows = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows[0].status, SyncRunProjectStatus::Failed);
    assert_eq!(rows[0].error_code.as_deref(), Some("E_VERIFY_FAILED"));
    assert!(
        rows[0]
            .error_message
            .as_deref()
            .is_some_and(|text| text.contains("failed the check")),
        "the message names the check: {:?}",
        rows[0].error_message
    );
    assert_eq!(
        project_of(&app, &project).last_synced_at,
        None,
        "an unverified write stamps nothing"
    );
}

// ---- eligibility ------------------------------------------------------

#[test]
fn a_missing_fingerprint_refuses_the_phase_before_any_write() {
    // The round-1 P3-2 fix's own pin: a broken phase-0 chain (a
    // `ready` project whose run row holds no build hash) refuses the
    // WHOLE phase before the first byte moves — zero write calls
    // (the counting sentinel), the class-only refusal, the rows and
    // the target files untouched — instead of a half-processed run.
    let (temp, app, repos) = phase3_app();
    let (first, first_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (second, second_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &first, "Be brief.");
    seed_assigned_instruction(&app, &second, "Also be kind.");
    fs::write(first_repo.join("AGENTS.md"), b"old first bytes")
        .expect("writing the fixture must work");
    fs::write(second_repo.join("AGENTS.md"), b"old second bytes")
        .expect("writing the fixture must work");

    let phase1 = preflight_ok(&app);
    // The broken chain: the SECOND ready project's fingerprint is
    // NULL — planted by direct SQL (the nullable column the phase-0
    // writer never leaves NULL; the store suite's planting shape).
    let run_id = phase1.run_id.clone();
    let planted_project = second.clone();
    tauri::async_runtime::block_on(database_of(&app).run(move |tx| {
        tx.execute(
            "UPDATE sync_run_projects SET build_hash = NULL \
             WHERE sync_run_id = ?1 AND project_id = ?2",
            rusqlite::params![run_id, planted_project],
        )
        .map(|_| ())
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("planting the NULL fingerprint must work");

    let (sentinel, calls) = counting_write();
    let refusal = tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        sentinel,
        fixed_marker_clock,
        fixed_now(),
    ))
    .expect_err("the broken chain must refuse the phase");

    // The class-only refusal (SEC-008): the label, no path bytes.
    assert_eq!(refusal.code, ErrorCode::DbLocked);
    let details = refusal.details.expect("the refusal carries details");
    assert_eq!(details.os_code, None);
    assert_eq!(details.os_kind.as_deref(), Some("Phase3FingerprintMissing"));
    // Zero writes: the precondition fired before the FIRST project,
    // even though the first project's own fingerprint is intact.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "no write happens when the phase cannot proceed"
    );
    // The world is untouched: rows keep phase 1's shape, the files
    // keep their bytes, no stamps.
    let rows = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows[0].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(rows[1].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(rows[1].build_hash, None, "the planted NULL stays");
    for (repo, bytes) in [
        (&first_repo, b"old first bytes".as_slice()),
        (&second_repo, b"old second bytes".as_slice()),
    ] {
        assert_eq!(
            fs::read(repo.join("AGENTS.md")).expect("reading the target back must work"),
            bytes,
            "no target file is touched"
        );
    }
    assert_eq!(project_of(&app, &first).last_synced_at, None);
    assert_eq!(project_of(&app, &second).last_synced_at, None);
    drop(temp);
}

#[test]
fn an_empty_ready_list_writes_nothing() {
    // A run whose every project is `unchanged` or `skipped` reaches
    // phase 3 with nothing to write: no write call happens (the
    // counting sentinel stays at zero), no marker is captured for any
    // file, no verdict changes, and the run continues (`running`) —
    // zero writes cannot fail (the phase-2 ruling).
    let (temp, app, repos) = phase3_app();
    let (same, same_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (gone, _gone_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &same, "Be same.");
    seed_assigned_instruction(&app, &gone, "Be gone.");
    // `same` matches its build by ALG-033 (the marker never decides);
    // `gone`'s folder is removed.
    fs::write(same_repo.join("AGENTS.md"), old_sync_file("Be same."))
        .expect("writing the fixture must work");
    fs::remove_dir_all(repos.join("repo-2")).expect("removing the folder must work");

    let phase1 = preflight_ok(&app);
    run_phase2(&app, &phase1, temp.path());
    let (sentinel, calls) = counting_write();
    let outcome = tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        sentinel,
        fixed_marker_clock,
        fixed_now(),
    ))
    .expect("an empty write list must succeed, not fail");

    assert!(
        outcome.results.is_empty(),
        "no ready project, no outcome entry"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "the phase never asked for a write"
    );
    assert_eq!(
        run_row_of(&app, &phase1.run_id).status,
        SyncRunStatus::Running,
        "the run continues to phase 4"
    );
    let rows = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows[0].status, SyncRunProjectStatus::Unchanged);
    assert_eq!(rows[1].status, SyncRunProjectStatus::Skipped);
    assert_eq!(
        fs::read_to_string(same_repo.join("AGENTS.md")).expect("reading the target back"),
        old_sync_file("Be same."),
        "no target file is touched"
    );
    assert_eq!(project_of(&app, &same).last_synced_at, None);
    assert_eq!(project_of(&app, &gone).last_synced_at, None);
}
