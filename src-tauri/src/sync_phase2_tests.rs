//! Behavior tests of the sync pipeline's phase 2 (T-115, §9.4 phase
//! 2, §7.6, §10's `E_BACKUP_FAILED` row, OUT-006/OUT-019).
//!
//! The suite drives the phase the way production reaches it —
//! [`sync_phase2::run`] over a real phase-1 outcome produced by
//! [`sync_phase0::run`] + [`sync_phase1::run`] — against REAL project
//! folders and REAL §5.5 backup trees in a temp root, so every copy
//! on the success path is `eunomia-fs`'s own verified operation
//! answering over a real fixture. The failure points of spec 15.1
//! (T-F1's run-level half) are injected through the phase's `backup`
//! seam (the D-089 pattern): a stateful closure delegates to the REAL
//! `backup_target_file` until the failing call, then answers with the
//! typed error — so the copies that must be swept away on the abort
//! path are real files on disk, exactly the debris production would
//! face.
//!
//! Pinned behavior:
//!
//! * the success path: one verified copy per eligible project under
//!   the builder-spelled §5.5 name (the fixed instant's stamp), one
//!   `ok` record per copy (size, SHA-256 — pinned against the FIPS
//!   180-2 "abc" oracle — resolved `source_path`, the run's one
//!   `created_at`), the run still `running`, the project rows still
//!   `not_started`, no target byte moved, no `last_synced` touch;
//! * BKP-010's dedup: a source digest equal to the pair's last
//!   checked copy makes no file and a record pointing at the existing
//!   copy's path;
//! * every injected failure (a copy error, a size mismatch, a digest
//!   mismatch) aborts the run the §9.4 phase 2 step 2 way: the run
//!   row `aborted` with `E_BACKUP_FAILED`, this run's copies
//!   physically deleted with `discarded` history rows, the
//!   non-`skipped` verdicts reset to `not_started`, `last_synced`
//!   untouched, every target byte verbatim — and the caller hears
//!   the typed envelope carrying ALL of BKP-005's fields (project
//!   name, source path, backups folder, OS code and error type,
//!   cause, recommendation);
//! * a deletion the filesystem refuses on the abort path keeps that
//!   copy's record `ok` over the live file — the never-lying-mark
//!   rule (D-122's prune posture, applied to `discarded`);
//! * eligibility: a `ready` project without a target file (OUT-005's
//!   creation case), an `unchanged` project and a `skipped` project
//!   get no copy, no record and no folder — and a run with NOTHING to
//!   back up performs no copy at all and continues.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_core::backup_name::{BackupTime, backup_dir_name, backup_relative_path};
use eunomia_core::project::{BaseName as CoreBaseName, ProjectId, Scheme as CoreScheme};
use eunomia_fs::backup::{
    BackupError, BackupOutcome, BackupTarget, VerifyFailed, VerifyMismatch, backup_target_file,
};
use eunomia_store::backup_records::{BackupRecordStatus, NewBackupRecord, insert as insert_record};
use eunomia_store::projects::NewProject;
use eunomia_store::settings::BaseName;
use eunomia_store::sync_runs::{SyncRunProjectStatus, SyncRunStatus};

use crate::data_dirs;
use crate::db::Db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_phase0;
use crate::sync_phase1::{self, Phase1Outcome};
use crate::sync_phase2::{self, BackupFact, Phase2Outcome};
use crate::sync_running::SyncRunning;

/// The ALG-032 example marker, verbatim (the phase-1 suite's fixture
/// shape — a marker line of the exact form ALG-033 strips).
const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

/// The FIPS 180-2 vector for "abc" — the independent SHA-256 oracle
/// for the fixture content (re-checked with Python's hashlib).
const ABC_DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// The run's fixed backup instant — 2026-11-21T10:30:00.123Z. One
/// value pins three spellings at once: the record `created_at`
/// (DB-002), the copy name's stamp (BKP-001), and the aborted run's
/// `finished_at`.
fn fixed_now() -> jiff::Timestamp {
    "2026-11-21T10:30:00.123Z"
        .parse()
        .expect("the fixed instant must parse")
}

/// The `created_at` every record of a run shares (the fixed instant's
/// DB-002 spelling).
const CREATED_AT: &str = "2026-11-21T10:30:00.123Z";

// ---- the harness ------------------------------------------------------

/// A mock app in the healthy state the phase serves (the
/// `sync_phase1_tests.rs` harness shape: mock runtime, a fresh
/// database in a temp data root, the `SyncRunning` flag managed),
/// plus the `repos/` root the fixtures live under. The data root IS
/// the backups `data_dir` the tests pass to the phase.
fn phase2_app() -> (
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

/// One `backup_records` row, read straight back.
struct RecordRow {
    project_id: String,
    source_path: String,
    backup_path: String,
    size_bytes: i64,
    sha256: String,
    status: String,
    created_at: String,
}

/// Every `backup_records` row of one run — the phase's records, read
/// through the app's own database.
fn records_of(app: &tauri::App<tauri::test::MockRuntime>, run_id: &str) -> Vec<RecordRow> {
    let database = database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let mut statement = tx
            .prepare(
                "SELECT project_id, source_path, backup_path, size_bytes, sha256, \
                        status, created_at \
                   FROM backup_records WHERE sync_run_id = ?1 ORDER BY created_at, id",
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let rows = statement
            .query_map([&run_id], |row| {
                Ok(RecordRow {
                    project_id: row.get(0)?,
                    source_path: row.get(1)?,
                    backup_path: row.get(2)?,
                    size_bytes: row.get(3)?,
                    sha256: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(crate::db::sqlite_error_to_app_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(rows)
    }))
    .expect("reading the run's backup records must work")
}

/// Creates a real repository folder (`repos/repo-<n>` with a `.git`
/// directory) and seeds its project row — the phase-1 harness shape.
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
    let canonical = fs::canonicalize(&repo).expect("canonicalizing the fresh repo must work");
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
/// needs (the phase-1 harness shape).
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
/// verbatim (the phase-1 harness shape, process-unique titles).
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
/// phase-2 drive starts from.
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

/// Drives phase 2 with the REAL copy operation and the fixed instant.
fn run_phase2(
    app: &tauri::App<tauri::test::MockRuntime>,
    phase1: &Phase1Outcome,
    data_dir: &Path,
) -> Result<Phase2Outcome, AppError> {
    tauri::async_runtime::block_on(sync_phase2::run(
        app.handle(),
        phase1,
        data_dir,
        backup_target_file,
        fixed_now(),
    ))
}

/// A stateful injected copy operation: delegates to the REAL
/// `backup_target_file` until the `fail_at`-th call (1-based), then
/// answers with `error` — real copies exist on disk before the
/// failure, exactly the debris the abort sweep must remove. The
/// `before_fail` hook runs right before the failing answer (the
/// undeletable-copy fixture's chmod). The one-shot payload rides a
/// `Mutex<Option<_>>` because the phase calls through `Fn` (the
/// injector runs once and then only delegates).
fn injected_backup(
    fail_at: usize,
    error: BackupError,
    before_fail: Option<Box<dyn FnOnce() + Send>>,
) -> impl Fn(&BackupTarget<'_>) -> Result<BackupOutcome, BackupError> + Send {
    let calls = AtomicUsize::new(0);
    let error = std::sync::Mutex::new(Some(error));
    let hook = std::sync::Mutex::new(before_fail);
    move |target| {
        if calls.fetch_add(1, Ordering::Relaxed) + 1 == fail_at {
            if let Some(hook) = hook
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                hook();
            }
            return Err(error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .expect("the injector fails at most once"));
        }
        backup_target_file(target)
    }
}

/// The old-sync A/C file form of `managed` (ALG-030) — the file a
/// previous run's write phase left, and the content an `unchanged`
/// fixture needs (the phase-1 suite's helper).
fn old_sync_file(managed: &str) -> String {
    if managed.is_empty() {
        format!("{MARKER}\n")
    } else {
        format!("{managed}\n\n{MARKER}\n")
    }
}

/// The project's §5.5 backup folder under the data root.
fn project_backup_folder(data_dir: &Path, project_id: &str, n: usize) -> PathBuf {
    data_dir.join("backups").join(
        backup_dir_name(
            &ProjectId::new(project_id.to_owned()),
            &format!("Project {n}"),
        )
        .expect("the DB-seeded id and name always pass the builders"),
    )
}

/// Every `.bak` file anywhere under the data root's `backups/` tree.
fn every_copy(data_dir: &Path) -> Vec<PathBuf> {
    fn walk(folder: &Path, out: &mut Vec<PathBuf>) {
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
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

// ---- the success path -------------------------------------------------

#[test]
fn ready_projects_with_files_get_verified_copies_and_ok_records() {
    let (temp, app, repos) = phase2_app();
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
    fs::write(&first_target, b"abc").expect("writing the fixture must work");
    fs::write(&second_target, b"abc").expect("writing the fixture must work");

    let phase1 = preflight_ok(&app);
    let outcome = run_phase2(&app, &phase1, temp.path()).expect("phase 2 must succeed");

    // The outcome: one Created fact per eligible project, in order.
    assert_eq!(outcome.backups.len(), 2, "both files get a copy");
    assert_eq!(outcome.backups[0].0, first);
    assert_eq!(outcome.backups[1].0, second);
    for (_, fact) in &outcome.backups {
        let BackupFact::Created(copy) = fact else {
            panic!("a fresh copy must be Created, got {fact:?}");
        };
        assert_eq!(copy.size, 3);
        assert_eq!(copy.sha256_hex, ABC_DIGEST);
    }

    // The files: builder-spelled §5.5 names under the project
    // folders — the fixed instant's stamp and the digest's hash8,
    // pinned as literals (never derived from the module's helpers).
    let first_name = "AGENTS.md.20261121T103000123Z.ba7816bf.bak";
    let second_name = "CLAUDE.project.md.20261121T103000123Z.ba7816bf.bak";
    let first_copy = project_backup_folder(temp.path(), &first, 1).join(first_name);
    let second_copy = project_backup_folder(temp.path(), &second, 2).join(second_name);
    assert_eq!(
        every_copy(temp.path()),
        vec![first_copy.clone(), second_copy.clone()],
        "exactly the two copies exist, under their builder-spelled names"
    );
    assert_eq!(
        fs::read(&first_copy).expect("reading the copy must work"),
        b"abc",
        "the copy holds the source's bytes"
    );

    // The records: one `ok` row per copy — resolved source path, the
    // FIPS-oracle digest and size, the run's one created_at stamp.
    let records = records_of(&app, &phase1.run_id);
    assert_eq!(records.len(), 2, "one record per copy");
    let first_source =
        fs::canonicalize(&first_target).expect("the fixture target must canonicalize");
    assert_eq!(records[0].project_id, first);
    assert_eq!(
        records[0].source_path,
        first_source.to_str().expect("the canonical path is UTF-8"),
        "source_path is the resolved spelling (D-122's contract)"
    );
    assert_eq!(records[0].sha256, ABC_DIGEST);
    assert_eq!(records[0].size_bytes, 3);
    assert_eq!(records[0].status, "ok");
    assert_eq!(records[0].created_at, CREATED_AT);
    assert!(records[0].backup_path.ends_with(first_name));
    assert_eq!(records[1].status, "ok");

    // The run and its rows: still phase 1's shape — phase 2 writes
    // no verdict of its own on success.
    assert_eq!(
        run_row_of(&app, &phase1.run_id).status,
        SyncRunStatus::Running
    );
    for row in run_rows_of(&app, &phase1.run_id) {
        assert_eq!(row.status, SyncRunProjectStatus::NotStarted);
    }

    // "No target file is touched" — the sources are verbatim.
    assert_eq!(
        fs::read(&first_target).expect("reading the target back must work"),
        b"abc"
    );
    assert_eq!(
        fs::read(&second_target).expect("reading the target back must work"),
        b"abc"
    );
}

#[test]
fn a_source_matching_the_last_copy_dedups_into_the_existing_record() {
    // BKP-010's dedup, end to end: the store holds the pair's checked
    // copy with the same digest the fs operation will compute, so no
    // second file appears and this run's record POINTS AT the
    // existing copy's path ("the record points to the existing one").
    let (temp, app, repos) = phase2_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    let target = repo.join("AGENTS.md");
    fs::write(&target, b"abc").expect("writing the fixture must work");
    let resolved = fs::canonicalize(&target).expect("the fixture target must canonicalize");
    let source_path = resolved
        .to_str()
        .expect("the canonical path is UTF-8")
        .to_owned();

    // The pair's existing checked copy: a real file under a
    // builder-spelled name and its `ok` record, seeded through the
    // store's own writer (the go-live history shape). The record
    // references an earlier run row, created through the repository
    // the way phase 0 would have.
    let run_before = {
        let database = database_of(&app);
        let seed_source = source_path.clone();
        let seed_project = project.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::sync_runs::create_running(
                tx,
                "2026-11-20T09:00:00.000Z",
                &[eunomia_store::sync_runs::NewSyncRunProject {
                    project_id: &seed_project,
                    target_path: &seed_source,
                    build_hash: &"b".repeat(64),
                }],
            )
            .map_err(runs_app_error)
        }))
        .expect("seeding the earlier run must work")
        .id
    };
    let earlier_time = BackupTime {
        year: 2026,
        month: 11,
        day: 20,
        hour: 9,
        minute: 0,
        second: 0,
        millisecond: 0,
    };
    let earlier_path = backup_relative_path(
        &ProjectId::new(project.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        earlier_time,
        ABC_DIGEST,
    )
    .expect("the builder accepts the fixture inputs");
    let earlier_copy = temp.path().join(&earlier_path);
    fs::create_dir_all(
        earlier_copy
            .parent()
            .expect("the builder's layout always has a parent"),
    )
    .expect("the earlier copy's folder must be creatable");
    fs::write(&earlier_copy, b"abc").expect("writing the earlier copy must work");
    let database = database_of(&app);
    let seed_input = (
        run_before.clone(),
        project.clone(),
        source_path.clone(),
        earlier_path.clone(),
    );
    tauri::async_runtime::block_on(database.run(move |tx| {
        insert_record(
            tx,
            &NewBackupRecord {
                project_id: &seed_input.1,
                sync_run_id: Some(&seed_input.0),
                source_path: &seed_input.2,
                backup_path: &seed_input.3,
                size_bytes: 3,
                sha256_hex: ABC_DIGEST,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-20T09:00:00.000Z",
            },
        )
        .map_err(|error| {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("SeedRecord".to_owned()),
                os_message: Some(error.to_string()),
            });
            app_error
        })
    }))
    .expect("seeding the earlier record must work");

    let phase1 = preflight_ok(&app);
    let outcome = run_phase2(&app, &phase1, temp.path()).expect("phase 2 must succeed");

    // No second file: the tree holds exactly the earlier copy.
    assert_eq!(
        every_copy(temp.path()),
        vec![earlier_copy.clone()],
        "an identical file does not multiply copies"
    );
    // The outcome and the record both point at the existing copy.
    let BackupFact::Deduplicated {
        size,
        sha256_hex,
        backup_path,
    } = &outcome.backups[0].1
    else {
        panic!(
            "the matching digest must dedup, got {:?}",
            outcome.backups[0].1
        );
    };
    assert_eq!((*size, sha256_hex.as_str()), (3, ABC_DIGEST));
    assert_eq!(backup_path, &earlier_path);
    // No new record: the UNIQUE `backup_path` column keeps one row
    // per copy, and the existing `ok` row already documents it — the
    // phase's run writes no row of its own for a dedup decision.
    assert!(
        records_of(&app, &phase1.run_id).is_empty(),
        "a dedup decision writes no record"
    );
    let still_checked = tauri::async_runtime::block_on(database_of(&app).run({
        let project = project.clone();
        let source_path = source_path.clone();
        move |tx| {
            eunomia_store::backup_records::last_checked(tx, &project, &source_path).map_err(
                |error| {
                    let mut app_error = AppError::new(ErrorCode::DbLocked);
                    app_error.details = Some(ErrorDetails {
                        os_code: None,
                        os_kind: Some("LookupRecord".to_owned()),
                        os_message: Some(error.to_string()),
                    });
                    app_error
                },
            )
        }
    }))
    .expect("the dedup lookup must work");
    assert_eq!(
        still_checked.map(|checked| (checked.sha256_hex, checked.backup_path)),
        Some((ABC_DIGEST.to_owned(), earlier_path.clone())),
        "the existing record remains the pair's one row"
    );
    // The target is untouched.
    assert_eq!(
        fs::read(&target).expect("reading the target back must work"),
        b"abc"
    );
}

// ---- the abort path (§9.4 phase 2 step 2) -----------------------------

/// Seeds the pair's earlier checked-copy RECORD through the store's
/// own writer (the go-live history shape), referencing an earlier
/// run row. Returns the earlier run's id.
fn seed_earlier_record(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
    source_path: &str,
    earlier_path: &str,
) -> String {
    let database = database_of(app);
    let seed_input = (
        project_id.to_owned(),
        source_path.to_owned(),
        earlier_path.to_owned(),
    );
    tauri::async_runtime::block_on(database.run(move |tx| {
        let run = eunomia_store::sync_runs::create_running(
            tx,
            "2026-11-20T09:00:00.000Z",
            &[eunomia_store::sync_runs::NewSyncRunProject {
                project_id: &seed_input.0,
                target_path: &seed_input.1,
                build_hash: &"b".repeat(64),
            }],
        )
        .map_err(runs_app_error)?;
        insert_record(
            tx,
            &NewBackupRecord {
                project_id: &seed_input.0,
                sync_run_id: Some(&run.id),
                source_path: &seed_input.1,
                backup_path: &seed_input.2,
                size_bytes: 3,
                sha256_hex: ABC_DIGEST,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-20T09:00:00.000Z",
            },
        )
        .map_err(|error| {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(ErrorDetails {
                os_code: None,
                os_kind: Some("SeedRecord".to_owned()),
                os_message: Some(error.to_string()),
            });
            app_error
        })?;
        Ok(run.id)
    }))
    .expect("seeding the earlier record must work")
}

#[test]
fn a_copy_error_aborts_the_run_sweeps_the_copies_and_resets_the_rows() {
    // The card's criterion: a simulated copy error changes no file,
    // aborts the run with E_BACKUP_FAILED, and the envelope carries
    // every BKP-009/005 field. The fixture is the full verdict mix —
    // a healthy copy that must be swept, the failing project, a
    // `skipped` one (unavailable) and an `unchanged` one — so the
    // reset rule ("not_started except skipped") is pinned by the rows
    // themselves.
    let (temp, app, repos) = phase2_app();
    let (healthy, healthy_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (failing, failing_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (gone, _gone_repo) = seed_repo_project(
        &app,
        &repos,
        3,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (same, same_repo) = seed_repo_project(
        &app,
        &repos,
        4,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &healthy, "Be brief.");
    seed_assigned_instruction(&app, &failing, "Be terse.");
    seed_assigned_instruction(&app, &gone, "Be gone.");
    seed_assigned_instruction(&app, &same, "Be same.");
    let healthy_target = healthy_repo.join("AGENTS.md");
    let failing_target = failing_repo.join("AGENTS.md");
    let same_target = same_repo.join("AGENTS.md");
    fs::write(&healthy_target, b"old healthy bytes").expect("writing the fixture must work");
    fs::write(&failing_target, b"old failing bytes").expect("writing the fixture must work");
    fs::write(&same_target, old_sync_file("Be same.")).expect("writing the fixture must work");
    fs::remove_dir_all(repos.join("repo-3")).expect("removing the folder must work");

    let phase1 = preflight_ok(&app);
    // The verdict mix, asserted before the abort rewrites the rows.
    let rows_before = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows_before[0].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(rows_before[1].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(rows_before[2].status, SyncRunProjectStatus::Skipped);
    assert_eq!(rows_before[3].status, SyncRunProjectStatus::Unchanged);

    // The injection: the SECOND eligible copy (the failing project's)
    // answers with the typed write-path error (T-F1's shape).
    let failing_source =
        fs::canonicalize(&failing_target).expect("the fixture target must canonicalize");
    let injection = injected_backup(
        2,
        BackupError::CopyFailed {
            path: PathBuf::from("/data/backups/failing/x.bak"),
            source: std::io::Error::from_raw_os_error(28),
            cleanup: None,
        },
        None,
    );
    let refusal = tauri::async_runtime::block_on(sync_phase2::run(
        app.handle(),
        &phase1,
        temp.path(),
        injection,
        fixed_now(),
    ))
    .expect_err("the copy error must abort the run");

    // The envelope: E_BACKUP_FAILED with EVERY BKP-005 field. The
    // user-visible half is `params` (UI-011's "Where" row), the
    // diagnostic half is `details`' composed line.
    assert_eq!(refusal.code, ErrorCode::BackupFailed);
    assert_eq!(
        refusal.params.get("project").map(String::as_str),
        Some("Project 2"),
        "the project param feeds the UI's Where row"
    );
    assert_eq!(
        refusal.params.get("path").map(String::as_str),
        Some(
            failing_source
                .to_str()
                .expect("the canonical path is UTF-8")
        ),
        "the path param feeds the UI's Where row"
    );
    let details = refusal.details.expect("the envelope carries details");
    assert_eq!(details.os_code, Some(28), "the OS code (errno 28)");
    assert_eq!(
        details.os_kind.as_deref(),
        Some("CopyFailed"),
        "the error type"
    );
    let message = details.os_message.expect("the BKP-005 message");
    let failing_folder = project_backup_folder(temp.path(), &failing, 2);
    for field in [
        ("project name", "Project 2".to_owned()),
        ("source path", failing_source.display().to_string()),
        ("backups folder", failing_folder.display().to_string()),
        ("cause", "cannot write the backup copy".to_owned()),
        ("recommendation", "run the sync again".to_owned()),
    ] {
        assert!(
            message.contains(&field.1),
            "the message must carry the {} ({:?}); message: {message}",
            field.0,
            field.1
        );
    }

    // This run's copies are deleted — the healthy project's real copy
    // is gone from disk, and its record is the `discarded` history.
    assert!(
        every_copy(temp.path()).is_empty(),
        "the copies of this run are deleted"
    );
    let records = records_of(&app, &phase1.run_id);
    assert_eq!(records.len(), 1, "only the made copy gets a record");
    assert_eq!(records[0].project_id, healthy);
    assert_eq!(records[0].status, "discarded");

    // The run row: aborted with the §10 literal and the fixed stamp.
    let run = run_row_of(&app, &phase1.run_id);
    assert_eq!(run.status, SyncRunStatus::Aborted);
    assert_eq!(run.error_code.as_deref(), Some("E_BACKUP_FAILED"));
    assert_eq!(run.finished_at.as_deref(), Some(CREATED_AT));
    assert!(
        run.details
            .as_deref()
            .is_some_and(|details| details.contains(&failing)),
        "the row's class-only details carry the failing project's id: {:?}",
        run.details
    );
    // The SEC-008 split: the stored row carries the id and the OS
    // facts, never the project NAME (that rides the envelope).
    assert!(
        !run.details
            .as_deref()
            .is_some_and(|details| details.contains("Project 2")),
        "the run row's details must not carry the project name: {:?}",
        run.details
    );
    // Every deletion succeeded on this path — the refused-deletion
    // count appears only when a copy survived the sweep.
    assert!(
        !run.details
            .as_deref()
            .is_some_and(|details| details.contains("could not be deleted")),
        "a fully swept run carries no refusal note: {:?}",
        run.details
    );

    // The rows: every verdict except `skipped` is back to
    // `not_started` — the `unchanged` verdict included.
    let rows = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows[0].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(rows[1].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(
        rows[2].status,
        SyncRunProjectStatus::Skipped,
        "the skipped verdict survives (§9.4's except-skipped)"
    );
    assert_eq!(rows[3].status, SyncRunProjectStatus::NotStarted);

    // OUT-020: the last-sync facts of every project stay untouched,
    // and no target byte moved.
    for (project, target, bytes) in [
        (&healthy, &healthy_target, b"old healthy bytes".as_slice()),
        (&failing, &failing_target, b"old failing bytes".as_slice()),
        (&same, &same_target, old_sync_file("Be same.").as_bytes()),
    ] {
        assert_eq!(
            fs::read(target).expect("reading the target back must work"),
            bytes,
            "no target file is touched"
        );
        let stored = tauri::async_runtime::block_on(database_of(&app).run({
            let project = project.clone();
            move |tx| {
                let row = tx
                    .query_row(
                        "SELECT last_synced_at, last_synced_hash FROM projects WHERE id = ?1",
                        [&project],
                        |row| {
                            Ok((
                                row.get::<_, Option<String>>(0)?,
                                row.get::<_, Option<String>>(1)?,
                            ))
                        },
                    )
                    .map_err(crate::db::sqlite_error_to_app_error)?;
                Ok(row)
            }
        }))
        .expect("reading the project's sync facts must work");
        assert_eq!(
            stored,
            (None, None),
            "the last sync of every project stays unchanged"
        );
    }
}

#[test]
fn a_size_or_digest_mismatch_aborts_the_run_the_same_way() {
    // BKP-010's check failures are §10's E_BACKUP_FAILED row too: a
    // size mismatch and a digest mismatch (the substituted damaged
    // copy's shape) each abort the run. The physical removal of the
    // corrupt copy is the fs operation's own contract (its suite
    // pins it); the phase's half is the verdict — run aborted, rows
    // reset, no records for a copy that never counted as created,
    // target verbatim.
    for (label, mismatch) in [
        ("size", VerifyMismatch::Size { actual_size: 999 }),
        (
            "digest",
            VerifyMismatch::Digest {
                actual_size: 3,
                actual_sha256_hex: "ff".repeat(64),
            },
        ),
    ] {
        let (temp, app, repos) = phase2_app();
        let (project, repo) = seed_repo_project(
            &app,
            &repos,
            1,
            eunomia_store::settings::Scheme::A,
            BaseName::AgentsMd,
        );
        seed_assigned_instruction(&app, &project, "Be brief.");
        let target = repo.join("AGENTS.md");
        fs::write(&target, b"abc").expect("writing the fixture must work");

        let phase1 = preflight_ok(&app);
        let injection = injected_backup(
            1,
            BackupError::VerifyFailed(VerifyFailed {
                path: PathBuf::from("/data/backups/p/x.bak"),
                expected_size: 3,
                expected_sha256_hex: ABC_DIGEST.to_owned(),
                mismatch,
                cleanup: None,
            }),
            None,
        );
        let refusal = tauri::async_runtime::block_on(sync_phase2::run(
            app.handle(),
            &phase1,
            temp.path(),
            injection,
            fixed_now(),
        ))
        .expect_err(&format!("the {label} mismatch must abort the run"));

        assert_eq!(refusal.code, ErrorCode::BackupFailed, "the {label} case");
        let details = refusal.details.expect("the envelope carries details");
        assert_eq!(details.os_kind.as_deref(), Some("VerifyFailed"));
        assert!(
            details.os_message.as_deref().is_some_and(
                |message| message.contains("Project 1") && message.contains("failed the check")
            ),
            "the {label} message names the project and the cause"
        );
        let run = run_row_of(&app, &phase1.run_id);
        assert_eq!(run.status, SyncRunStatus::Aborted, "the {label} case");
        assert_eq!(run.error_code.as_deref(), Some("E_BACKUP_FAILED"));
        assert!(
            records_of(&app, &phase1.run_id).is_empty(),
            "a copy that failed the check never gets a record ({label})"
        );
        assert_eq!(
            run_rows_of(&app, &phase1.run_id)[0].status,
            SyncRunProjectStatus::NotStarted
        );
        assert_eq!(
            fs::read(&target).expect("reading the target back must work"),
            b"abc",
            "no target file is touched ({label})"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_deletion_the_filesystem_refuses_keeps_the_records_honest() {
    // The never-lying-mark rule: a copy whose file cannot be deleted
    // (the fixture turns the project's backup folder read-only before
    // the failing answer) keeps its `ok` record over the LIVE file —
    // `discarded` asserts the file is gone, and D-122's prune ruling
    // is the precedent. Root cannot stage this (mode bits are not
    // enforced), so the test skips loudly there (the house pattern).
    let (temp, app, repos) = phase2_app();
    let (healthy, healthy_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (failing, failing_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &healthy, "Be brief.");
    seed_assigned_instruction(&app, &failing, "Be terse.");
    fs::write(healthy_repo.join("AGENTS.md"), b"old healthy bytes")
        .expect("writing the fixture must work");
    fs::write(failing_repo.join("AGENTS.md"), b"old failing bytes")
        .expect("writing the fixture must work");
    let healthy_folder = project_backup_folder(temp.path(), &healthy, 1);
    // The fixture's owner uid is this process's (it created the temp
    // root): uid 0 means the tests run as root, where mode bits are
    // not enforced — the precondition cannot be staged, and the test
    // skips loudly instead of passing vacuously (the house pattern).
    let running_as_root = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(temp.path())
            .expect("the temp root must be statable")
            .uid()
            == 0
    };
    if running_as_root {
        eprintln!("skipping: running as root, mode bits are not enforced");
        return;
    }

    let phase1 = preflight_ok(&app);
    use std::os::unix::fs::PermissionsExt;
    let hook_folder = healthy_folder.clone();
    let injection = injected_backup(
        2,
        BackupError::SourceFailed {
            path: PathBuf::from("/repos/repo-2/AGENTS.md"),
            source: std::io::Error::from_raw_os_error(5),
        },
        Some(Box::new(move || {
            // The healthy copy exists by now; a read-only folder
            // refuses its removal (EACCES for this process).
            fs::set_permissions(&hook_folder, fs::Permissions::from_mode(0o500))
                .expect("loosening the folder must work");
        })),
    );
    let refusal = tauri::async_runtime::block_on(sync_phase2::run(
        app.handle(),
        &phase1,
        temp.path(),
        injection,
        fixed_now(),
    ))
    .expect_err("the failure must abort the run");
    assert_eq!(refusal.code, ErrorCode::BackupFailed);

    // The undeleted copy is still on disk, and its record is `ok` —
    // a verified live copy, not a lying `discarded` row.
    let copies = every_copy(temp.path());
    assert_eq!(copies.len(), 1, "the refused copy stays");
    let records = records_of(&app, &phase1.run_id);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "ok");
    assert_eq!(
        records[0].backup_path,
        copies[0]
            .strip_prefix(temp.path())
            .ok()
            .and_then(|relative| relative.to_str())
            .expect("the copy lives under the data root"),
        "the ok record names the live copy"
    );
    // The run still aborted: one refused deletion does not un-fail
    // the backup phase — and the refusal rides the run row's details
    // as the class-only count (the round-1 P3-3 fix, pinned): the
    // operator reading the aborted row understands the `ok` record
    // inside it.
    let run = run_row_of(&app, &phase1.run_id);
    assert_eq!(run.status, SyncRunStatus::Aborted);
    assert!(
        run.details
            .as_deref()
            .is_some_and(|details| details.contains("could not be deleted")),
        "the refused-deletion count reaches the row's details: {:?}",
        run.details
    );
    // Restore the folder's rights so the temp teardown can clean it.
    fs::set_permissions(&healthy_folder, fs::Permissions::from_mode(0o700))
        .expect("restoring the folder's rights must work");
}

// ---- eligibility ------------------------------------------------------

#[test]
fn fileless_unchanged_and_skipped_projects_get_no_copy_and_the_run_continues() {
    // §9.4 phase 2 step 1's eligibility: only `ready` projects WITH
    // an existing file are backed up — the creation case (OUT-005),
    // `unchanged` and `skipped` get nothing. A run with NOTHING to
    // back up makes no copy at all, writes no record, and the run
    // continues (`running`): zero copies cannot fail.
    let (temp, app, repos) = phase2_app();
    let (fresh, fresh_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (same, same_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (gone, _gone_repo) = seed_repo_project(
        &app,
        &repos,
        3,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &fresh, "Be brief.");
    seed_assigned_instruction(&app, &same, "Be same.");
    seed_assigned_instruction(&app, &gone, "Be gone.");
    // `fresh` has NO target file (the creation case); `same` matches
    // its build; `gone`'s folder is removed.
    fs::write(same_repo.join("AGENTS.md"), old_sync_file("Be same."))
        .expect("writing the fixture must work");
    fs::remove_dir_all(repos.join("repo-3")).expect("removing the folder must work");

    let phase1 = preflight_ok(&app);
    let outcome =
        run_phase2(&app, &phase1, temp.path()).expect("a backupless run must succeed, not abort");

    assert!(
        outcome.backups.is_empty(),
        "no eligible project, no backup fact"
    );
    assert!(
        every_copy(temp.path()).is_empty(),
        "no copy exists anywhere in the tree"
    );
    assert!(
        records_of(&app, &phase1.run_id).is_empty(),
        "no record was written"
    );
    assert_eq!(
        run_row_of(&app, &phase1.run_id).status,
        SyncRunStatus::Running,
        "the run continues past an empty backup list"
    );
    // The rows keep phase 1's verdicts; the creation-case target is
    // still absent (the write phase owns creating it).
    let rows = run_rows_of(&app, &phase1.run_id);
    assert_eq!(rows[0].status, SyncRunProjectStatus::NotStarted);
    assert_eq!(rows[1].status, SyncRunProjectStatus::Unchanged);
    assert_eq!(rows[2].status, SyncRunProjectStatus::Skipped);
    assert!(
        !fresh_repo.join("AGENTS.md").exists(),
        "phase 2 creates nothing"
    );
}

#[test]
fn a_vanished_recorded_copy_does_not_dedup() {
    // The round-1 review's P1-2, pinned: an `ok` record whose FILE is
    // gone (the user deleted the backups tree — D-122's recorded
    // window) must not send the run into phase 3 with no restore
    // point. The record's digest matches the source exactly (the
    // dedup shape), but the copy does not exist — so the phase makes
    // a FRESH copy under the run's own instant instead.
    let (temp, app, repos) = phase2_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    let target = repo.join("AGENTS.md");
    fs::write(&target, b"abc").expect("writing the fixture must work");
    let resolved = fs::canonicalize(&target).expect("the fixture target must canonicalize");
    let source_path = resolved.to_str().expect("the canonical path is UTF-8");
    let earlier_path = backup_relative_path(
        &ProjectId::new(project.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        BackupTime {
            year: 2026,
            month: 11,
            day: 20,
            hour: 9,
            minute: 0,
            second: 0,
            millisecond: 0,
        },
        ABC_DIGEST,
    )
    .expect("the builder accepts the fixture inputs");
    // The record exists, the copy does NOT.
    seed_earlier_record(&app, &project, source_path, &earlier_path);

    let phase1 = preflight_ok(&app);
    let outcome = run_phase2(&app, &phase1, temp.path()).expect("phase 2 must succeed");

    let BackupFact::Created(copy) = &outcome.backups[0].1 else {
        panic!(
            "a vanished copy must force a fresh copy, got {:?}",
            outcome.backups[0].1
        );
    };
    // The fresh copy is real, on disk, under the RUN's instant — a
    // different name than the vanished one (whose folder was never
    // recreated).
    assert_eq!(copy.sha256_hex, ABC_DIGEST);
    assert_ne!(copy.relative_path, earlier_path);
    assert_eq!(
        every_copy(temp.path()),
        vec![temp.path().join(&copy.relative_path)],
        "exactly the fresh copy exists"
    );
    // And its record is this run's `ok` row.
    let records = records_of(&app, &phase1.run_id);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "ok");
    assert_eq!(records[0].backup_path, copy.relative_path);
}

#[test]
fn the_failed_copies_leftover_bytes_are_swept_with_the_run() {
    // The fs operation's `cleanup: Some` handshake (D-091): a failed
    // copy whose partial bytes it could not remove itself names the
    // file, and THIS sweep is the designated retrier — the staged
    // partial must not survive the aborted run as unreferenced
    // debris under a final backup name.
    let (temp, app, repos) = phase2_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    fs::write(repo.join("AGENTS.md"), b"abc").expect("writing the fixture must work");
    // The leftover partial: a real file under a final-name-shaped
    // path inside the project's backup folder.
    let partial = project_backup_folder(temp.path(), &project, 1)
        .join("AGENTS.md.20261121T103000123Z.deadbeef.bak");
    fs::create_dir_all(
        partial
            .parent()
            .expect("the fixture folder always has a parent"),
    )
    .expect("the partial's folder must be creatable");
    fs::write(&partial, b"partial bytes").expect("writing the partial must work");

    let phase1 = preflight_ok(&app);
    let injection = injected_backup(
        1,
        BackupError::CopyFailed {
            path: partial.clone(),
            source: std::io::Error::from_raw_os_error(28),
            cleanup: Some(std::io::Error::from_raw_os_error(13)),
        },
        None,
    );
    let refusal = tauri::async_runtime::block_on(sync_phase2::run(
        app.handle(),
        &phase1,
        temp.path(),
        injection,
        fixed_now(),
    ))
    .expect_err("the copy error must abort the run");
    assert_eq!(refusal.code, ErrorCode::BackupFailed);

    assert!(
        !partial.exists(),
        "the failed copy's leftover bytes are swept with the run"
    );
    assert!(
        every_copy(temp.path()).is_empty(),
        "no copy of this run survives the abort"
    );
    assert!(
        records_of(&app, &phase1.run_id).is_empty(),
        "a copy that never counted as created gets no record"
    );
    assert_eq!(
        run_row_of(&app, &phase1.run_id).status,
        SyncRunStatus::Aborted
    );
}

#[test]
fn a_record_transaction_failure_cleans_the_copies_and_leaves_the_run_running() {
    // The success path's store-failure posture (D-132): the record
    // transaction fails (a stale record already claims the §5.5 name
    // the builders spell for this run — the split-window shape D-122
    // warns about, seeded under a historic source-path spelling so
    // the dedup lookup does not shortcut it), the transaction rolls
    // back, the copies are swept best-effort, and the store error is
    // the honest answer with the run row left `running` (the
    // crash-recovery rule's case).
    let (temp, app, repos) = phase2_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    fs::write(repo.join("AGENTS.md"), b"abc").expect("writing the fixture must work");
    // The exact §5.5 name this run's copy will receive (the fixed
    // instant and the FIPS digest spell it), claimed by a stale
    // record under a historic spelling of the source path.
    let claimed_path = backup_relative_path(
        &ProjectId::new(project.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        BackupTime {
            year: 2026,
            month: 11,
            day: 21,
            hour: 10,
            minute: 30,
            second: 0,
            millisecond: 123,
        },
        ABC_DIGEST,
    )
    .expect("the builder accepts the fixture inputs");
    seed_earlier_record(
        &app,
        &project,
        "/historic/spelling/AGENTS.md",
        &claimed_path,
    );

    let phase1 = preflight_ok(&app);
    let error = run_phase2(&app, &phase1, temp.path())
        .expect_err("the UNIQUE claim must fail the record transaction");

    // The store error through the shared SQLite classifier, with the
    // constraint named — not a silent success, not a backup abort.
    assert_eq!(error.code, ErrorCode::Validation);
    assert!(
        error
            .details
            .as_ref()
            .and_then(|details| details.os_kind.as_deref())
            .is_some_and(|kind| kind == "ConstraintViolation"),
        "the UNIQUE refusal is the typed constraint failure: {error:?}"
    );
    // The copies were swept: nothing survives under the backups tree
    // (the copy the fs operation made and the transaction orphaned).
    assert!(
        every_copy(temp.path()).is_empty(),
        "the orphaned copies are deleted best-effort"
    );
    // The run row stays `running` — the crash-recovery rule's case,
    // never a half-abort.
    assert_eq!(
        run_row_of(&app, &phase1.run_id).status,
        SyncRunStatus::Running
    );
    assert!(
        records_of(&app, &phase1.run_id).is_empty(),
        "the rolled-back transaction wrote no records"
    );
    assert_eq!(
        run_rows_of(&app, &phase1.run_id)[0].status,
        SyncRunProjectStatus::NotStarted
    );
}

#[test]
fn a_copy_vanishing_inside_the_hash_window_is_made_fresh() {
    // The round-2 residual TOCTOU, pinned: the recorded copy exists
    // when the loop hands its digest over, and is GONE by the time
    // the operation answers `Deduplicated` (the injector deletes it
    // between the two moments — the quarantine-inside-the-hash
    // window). The phase must not accept the dedup: it re-copies
    // with no digest, so the write keeps a restore point.
    let (temp, app, repos) = phase2_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    let target = repo.join("AGENTS.md");
    fs::write(&target, b"abc").expect("writing the fixture must work");
    let resolved = fs::canonicalize(&target).expect("the fixture target must canonicalize");
    let source_path = resolved.to_str().expect("the canonical path is UTF-8");
    let earlier_path = backup_relative_path(
        &ProjectId::new(project.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        BackupTime {
            year: 2026,
            month: 11,
            day: 20,
            hour: 9,
            minute: 0,
            second: 0,
            millisecond: 0,
        },
        ABC_DIGEST,
    )
    .expect("the builder accepts the fixture inputs");
    seed_earlier_record(&app, &project, source_path, &earlier_path);
    let earlier_copy = temp.path().join(&earlier_path);
    fs::create_dir_all(
        earlier_copy
            .parent()
            .expect("the builder's layout always has a parent"),
    )
    .expect("the earlier copy's folder must be creatable");
    fs::write(&earlier_copy, b"abc").expect("writing the earlier copy must work");

    // The injector: call 1 "answers" the digest comparison honestly
    // (the source matches the recorded digest) but deletes the
    // recorded copy first — the vanishing inside the hash window.
    // Call 2 delegates to the real operation with whatever digest
    // the phase now supplies (None — the re-copy).
    let vanished = earlier_copy.clone();
    let calls = AtomicUsize::new(0);
    let injection = move |target: &BackupTarget<'_>| -> Result<BackupOutcome, BackupError> {
        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
            fs::remove_file(&vanished).expect("the fixture vanishing must work");
            return Ok(BackupOutcome::Deduplicated {
                size: 3,
                sha256_hex: ABC_DIGEST.to_owned(),
            });
        }
        backup_target_file(target)
    };

    let phase1 = preflight_ok(&app);
    let outcome = tauri::async_runtime::block_on(sync_phase2::run(
        app.handle(),
        &phase1,
        temp.path(),
        injection,
        fixed_now(),
    ))
    .expect("the vanished copy must not fail the run");

    // The re-copy is real and recorded — never a dedup over a
    // missing file.
    let BackupFact::Created(copy) = &outcome.backups[0].1 else {
        panic!(
            "a vanished copy must force a fresh copy, got {:?}",
            outcome.backups[0].1
        );
    };
    assert_ne!(copy.relative_path, earlier_path);
    assert_eq!(
        every_copy(temp.path()),
        vec![temp.path().join(&copy.relative_path)],
        "exactly the fresh copy exists (the vanished one is gone)"
    );
    let records = records_of(&app, &phase1.run_id);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "ok");
    assert_eq!(records[0].backup_path, copy.relative_path);
}
