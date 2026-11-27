//! Behavior tests of the sync pipeline's phase 4 (T-117, §9.4 phase 4,
//! BKP-003, MAIN-027, §7.7's run journal, DB-024/OUT-020).
//!
//! The suite drives the phase the way production reaches it —
//! [`sync_phase4::run`] over a real phase-3 outcome produced by the
//! full [`sync_phase0::run`] + [`sync_phase1::run`] +
//! [`sync_phase2::run_with_abort_clock`] + [`sync_phase3::run`] chain
//! (§9.4's own order) — against REAL project folders in a temp data
//! root, so the
//! retention sweep answers over real `.bak` files phase 2 itself
//! copied. The `state://dirty` seam is injected as a counting
//! closure (the phase-2/3 injector pattern over an async seam); the
//! write-failure paths of the `failed` verdict are injected through
//! phase 3's `write` seam, exactly as its own suite does.
//!
//! Pinned behavior:
//!
//! * the success path: the `unchanged` project is stamped in the
//!   written project's own form (the run's one DB-002 instant, the
//!   run's frozen DB-020 fingerprint), the run row lands `success`
//!   with the counted counters and `finished_at`, MAIN-027's derived
//!   answer is that instant, the journal holds the D-051 dictionary
//!   (one `sync.project_result` per project plus the
//!   `sync.run_finished` summary), and the dirty seam ran exactly
//!   once — after the commit;
//! * the retention sweep applies `backup.retention` inside the
//!   finish (copies beyond the limit leave the disk and their
//!   records go `pruned`), and its refusals are REPORTED, never the
//!   verdict's;
//! * a partial run (a skipped project beside a good one) keeps
//!   MAIN-027's answer at the last success and re-stamps its
//!   unchanged project in the written form;
//! * a run where nobody was written or left unchanged is `failed`;
//! * the empty run is `success` with the §9.4 message;
//! * a run with an unsettled row refuses — the row stays `running`,
//!   nothing is journaled.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_core::backup_name::{BackupTime, backup_relative_path};
use eunomia_core::project::{BaseName as CoreBaseName, ProjectId, Scheme as CoreScheme};
use eunomia_store::action_log::{Action, Level, LogRecord, Outcome};
use eunomia_store::backup_records::{BackupRecordStatus, NewBackupRecord};
use eunomia_store::projects::NewProject;
use eunomia_store::settings::{BaseName, SettingKey, SettingValue};
use eunomia_store::sync_runs::SyncRunStatus;

use crate::data_dirs;
use crate::db::Db;
use crate::db::sync_runs_app_error_for_tests;
use crate::dirty_recalc::library_seed_error_for_tests;
use crate::errors::AppError;
use crate::sync_phase0;
use crate::sync_phase1::{self, Phase1Outcome};
use crate::sync_phase2;
use crate::sync_phase3::{self, Phase3Outcome};
use crate::sync_phase4;

/// The run's fixed write instant (the phase-2/3 suite's value): the
/// marker line's Prague civil time and the stamps of the written
/// projects.
fn fixed_now() -> jiff::Timestamp {
    "2026-11-21T10:30:00.123Z"
        .parse()
        .expect("the fixed instant must parse")
}

/// The suite-wide marker of the previous run's files (the phase-3
/// suite's literal).
const MARKER_LINE: &str =
    "<!-- Created by EunomiaSync 2026-11-21 11:30:00 Europe/Prague +01:00 -->";

/// The FIRST run's finish instant, as a DB-002 literal oracle: the
/// value `finished_at` and the unchanged stamps must carry verbatim.
const FIRST_FINISH_AT: &str = "2026-11-21T11:45:00.456Z";

// ---- the harness ------------------------------------------------------

/// A mock app in the healthy state the phase serves (the
/// `sync_phase3_tests.rs` harness shape: mock runtime, a fresh
/// database in a temp data root, the `SyncRunning` flag managed),
/// plus the `repos/` root the fixtures live under.
fn phase4_app() -> (
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
    app.manage(crate::sync_running::SyncRunning::new());
    // The write-in-flight gate phase 3's per-project guard reads
    // (T-123/D-219 — the state `setup` manages beside the flag).
    app.manage(crate::commands::SyncWriteGate::new());
    let repos = temp.path().join("repos");
    fs::create_dir(&repos).expect("the repos root must be creatable");
    (temp, app, repos)
}

/// The managed database handle.
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> Db {
    app.state::<Db>().inner().clone()
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
            .map_err(sync_runs_app_error_for_tests)
            .map(|row| row.expect("the run row this suite recorded must persist"))
    }))
    .expect("reading the run row must work")
}

/// MAIN-027's derived answer through the app's database.
fn last_success_of(app: &tauri::App<tauri::test::MockRuntime>) -> Option<String> {
    let database = database_of(app);
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::sync_runs::last_success_finished_at(tx)
            .map_err(sync_runs_app_error_for_tests)
    }))
    .expect("the MAIN-027 read must work")
}

/// The run's journal entries (newest first), filtered to the run —
/// the seeded settings writes journal too, and the filter is the
/// entries' own `sync_run_id` fact.
fn journal_of(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> Vec<(Action, Level, Outcome)> {
    let database = database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::action_log::recent(tx, 100).map_err(|error| {
            crate::db::sqlite_error_to_app_error(match error {
                eunomia_store::storage::StorageError::Sql(sql) => sql,
                other => rusqlite::Error::InvalidParameterName(other.to_string()),
            })
        })
    }))
    .expect("reading the journal must work")
    .into_iter()
    .filter(|record: &LogRecord| record.sync_run_id.as_deref() == Some(run_id.as_str()))
    .map(|record| (record.action, record.level, record.result))
    .collect()
}

/// The run's journal entries as full records (newest first, filtered
/// to the run) — the details-level oracle (`journal_of`'s shape twin
/// for the assertions that read payloads).
fn journal_records_of(app: &tauri::App<tauri::test::MockRuntime>, run_id: &str) -> Vec<LogRecord> {
    let database = database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::action_log::recent(tx, 100).map_err(|error| {
            crate::db::sqlite_error_to_app_error(match error {
                eunomia_store::storage::StorageError::Sql(sql) => sql,
                other => rusqlite::Error::InvalidParameterName(other.to_string()),
            })
        })
    }))
    .expect("reading the journal must work")
    .into_iter()
    .filter(|record: &LogRecord| record.sync_run_id.as_deref() == Some(run_id.as_str()))
    .collect()
}

/// Creates a real repository folder (`repos/repo-<n>` with a `.git`
/// directory) and seeds its project row — the phase-3 harness shape.
/// Returns `(project_id, repo_path)`.
fn seed_repo_project(
    app: &tauri::App<tauri::test::MockRuntime>,
    repos: &Path,
    n: usize,
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
                scheme: eunomia_store::settings::Scheme::A,
                base_name: BaseName::AgentsMd,
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

/// Seeds one active instruction with its main variant and assigns it
/// to the project: the project's built managed text is then `content`
/// verbatim (the phase-2/3 harness shape, process-unique titles).
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
        .map_err(library_seed_error_for_tests)?;
        let variant = eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content,
                is_main: true,
            },
        )
        .map_err(library_seed_error_for_tests)?;
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &item.id,
                variant_id: &variant.id,
            },
        )
        .map_err(library_seed_error_for_tests)?;
        Ok(())
    }))
    .expect("seeding the assigned instruction must work");
}

/// The old-sync A/C file form of `managed` (ALG-030) — the file a
/// previous run's write phase left (the phase-2/3 suite's helper):
/// pre-written into a repo, phase 1's step-5 comparison (ALG-033
/// drops the marker line) reads the build as `unchanged`.
fn old_sync_file(managed: &str) -> String {
    if managed.is_empty() {
        format!("{MARKER_LINE}\n")
    } else {
        format!("{managed}\n\n{MARKER_LINE}\n")
    }
}

/// Runs phases 0–3 back to back over the healthy fixtures (the
/// plenty-everywhere probe, the real clock, the real §9.5 write) and
/// drops phase 0's flag lease — the composition every phase-4 drive
/// starts from. Returns `(run_id, phase1)`.
fn chain_through_phase3(
    app: &tauri::App<tauri::test::MockRuntime>,
    data_dir: &Path,
) -> (String, Phase1Outcome) {
    let phase0 = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over the seeded database");
    let phase1 = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        || Ok(u64::MAX),
        SystemTime::now(),
    ))
    .expect("phase 1 must pass");
    tauri::async_runtime::block_on(sync_phase2::run_with_abort_clock(
        app.handle(),
        &phase1,
        data_dir,
        eunomia_fs::backup::backup_target_file,
        fixed_now(),
        jiff::Timestamp::now,
    ))
    .expect("phase 2 must succeed over the healthy fixtures");
    let phase3 = tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        sync_phase3::production_write,
        || {
            eunomia_core::marker_clock::MarkerClock::from_instant(
                fixed_now(),
                jiff::tz::TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
            )
        },
        fixed_now(),
    ))
    .expect("phase 3 must succeed over the healthy fixtures");
    let run_id = phase3.run_id.clone();
    drop(phase0);
    (run_id, phase1)
}

/// The run's finish instant of the first drive, as a [`jiff`]
/// timestamp.
fn first_finish_instant() -> jiff::Timestamp {
    FIRST_FINISH_AT
        .parse()
        .expect("the fixed finish instant must parse")
}

/// Drives phase 4 with a COUNTING, OBSERVING dirty seam and returns
/// the outcome, the call count, and what the seam saw of the run row
/// — the §9.4 phase 4 step 5 ordering pin: the seam runs only after
/// the transaction commits, so the row it reads must already carry
/// the verdict and the finish stamp; a seam moved ahead of the
/// commit reads `running`/`None` and fails the per-test assert on
/// the observed pair.
#[allow(clippy::type_complexity)]
fn run_phase4_counting(
    app: &tauri::App<tauri::test::MockRuntime>,
    phase1: &Phase1Outcome,
    phase3_run_id: &str,
    data_dir: &Path,
    now: jiff::Timestamp,
) -> (
    sync_phase4::Phase4Outcome,
    usize,
    Option<(String, Option<String>)>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let observed: Arc<std::sync::Mutex<Option<(String, Option<String>)>>> =
        Arc::new(std::sync::Mutex::new(None));
    let observed_seam = Arc::clone(&observed);
    let phase3 = Phase3Outcome {
        run_id: phase3_run_id.to_owned(),
        results: Vec::new(),
    };
    let seam_run_id = phase3_run_id.to_owned();
    let outcome = tauri::async_runtime::block_on(sync_phase4::run(
        app.handle(),
        phase1,
        &phase3,
        data_dir,
        now,
        move |seam_app| {
            // The handle's use stays OUTSIDE the async block, so the
            // future owns its facts and borrows nothing from the
            // seam's argument.
            let database = seam_app.state::<Db>().inner().clone();
            let counted = Arc::clone(&counted);
            let observed_seam = Arc::clone(&observed_seam);
            let seam_run_id = seam_run_id.clone();
            async move {
                counted.fetch_add(1, Ordering::Relaxed);
                // The seam observes the COMMITTED world through its
                // own read — the ordering fact the counter alone
                // cannot prove.
                let row = database
                    .run(move |tx| {
                        eunomia_store::sync_runs::get(tx, &seam_run_id)
                            .map_err(sync_runs_app_error_for_tests)
                            .map(|row| row.expect("the run row must persist"))
                    })
                    .await?;
                *observed_seam
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some((row.status.as_str().to_owned(), row.finished_at));
                Ok(Vec::new())
            }
        },
    ))
    .expect("phase 4 must finish the healthy run");
    let count = calls.load(Ordering::Relaxed);
    let seen = observed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    (outcome, count, seen)
}

/// Every `.bak` FILE anywhere under the data root's `backups/` tree
/// (the phase-2/3 suite's view; a directory squatting on a `.bak`
/// name is not a copy).
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

/// Sets `backup.retention` through the settings repository — the
/// production write path, journal entry and all.
fn set_retention(app: &tauri::App<tauri::test::MockRuntime>, keep: i64) {
    let database = database_of(app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::settings::set(tx, SettingKey::BackupRetention, SettingValue::Int(keep))
            .map_err(|error| {
                crate::db::sqlite_error_to_app_error(match error {
                    eunomia_store::storage::StorageError::Sql(sql) => sql,
                    other => rusqlite::Error::InvalidParameterName(other.to_string()),
                })
            })
    }))
    .expect("setting backup.retention must work");
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

/// The run's build-hash fingerprint per project, off the run's own
/// rows — the oracle the stamps must equal (D-130: the frozen value,
/// never a recomputation).
fn fingerprints_of(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> Vec<(String, Option<String>)> {
    let database = database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::sync_runs::list_projects(tx, &run_id).map_err(sync_runs_app_error_for_tests)
    }))
    .expect("reading the run's rows must work")
    .into_iter()
    .map(|row| (row.project_id, row.build_hash))
    .collect()
}

// ---- the success path ---------------------------------------------------

#[test]
fn the_success_run_stamps_finishes_journals_and_recalculates() {
    // §9.4 phase 4, end to end over the real chain: one project
    // written (the creation case) and one unchanged (its file already
    // holds the build by ALG-033) — the unchanged project is stamped
    // in the written project's own form (the run's one DB-002
    // instant, the run's frozen fingerprint), the run row lands
    // `success` with the counted counters, MAIN-027's derived answer
    // is the finish instant, the journal holds the D-051 dictionary,
    // and the dirty seam ran exactly once.
    let (temp, app, repos) = phase4_app();
    let (written_id, _written_repo) = seed_repo_project(&app, &repos, 1);
    let (unchanged_id, unchanged_repo) = seed_repo_project(&app, &repos, 2);
    seed_assigned_instruction(&app, &written_id, "Be written.");
    seed_assigned_instruction(&app, &unchanged_id, "Be matched.");
    // The unchanged project's target already holds the build (the
    // old-sync form of its managed text) — phase 1 step 5's verdict.
    fs::write(
        unchanged_repo.join("AGENTS.md"),
        old_sync_file("Be matched."),
    )
    .expect("pre-writing the matched target must work");

    let (run_id, phase1) = chain_through_phase3(&app, temp.path());
    assert_eq!(
        phase1
            .projects
            .iter()
            .filter(|(_, verdict)| {
                matches!(verdict, crate::sync_phase1::PreflightVerdict::Unchanged)
            })
            .count(),
        1,
        "the fixture's premise: exactly one unchanged project"
    );
    let (outcome, dirty_calls, seam_saw) =
        run_phase4_counting(&app, &phase1, &run_id, temp.path(), first_finish_instant());

    // §9.4 phase 4 step 3: the verdict, the counters, the finish.
    let row = run_row_of(&app, &run_id);
    assert_eq!(outcome.run.status, SyncRunStatus::Success);
    assert_eq!(row.status, SyncRunStatus::Success);
    assert_eq!(row.finished_at.as_deref(), Some(FIRST_FINISH_AT));
    assert_eq!(row.projects_total, 2);
    assert_eq!(row.projects_written, 1);
    assert_eq!(row.projects_unchanged, 1);
    assert_eq!(row.projects_skipped, 0);
    assert_eq!(row.projects_failed, 0);
    assert_eq!(row.error_code, None);
    assert_eq!(row.details, None);

    // Step 1 + the DB-024 form ruling: BOTH stamps follow the same
    // rule — the run's one DB-002 instant and the run's own frozen
    // fingerprint per project (the written project's from phase 3's
    // short transaction, the unchanged project's from this phase's).
    let fingerprints = fingerprints_of(&app, &run_id);
    let fingerprint_of = |id: &str| -> String {
        fingerprints
            .iter()
            .find(|(row_id, _)| row_id == id)
            .unwrap_or_else(|| panic!("the row of {id} must exist"))
            .1
            .clone()
            .expect("phase 0 froze the fingerprint")
    };
    let unchanged = project_of(&app, &unchanged_id);
    let written = project_of(&app, &written_id);
    assert_eq!(unchanged.last_synced_at.as_deref(), Some(FIRST_FINISH_AT));
    assert_eq!(
        unchanged.last_synced_hash.as_deref(),
        Some(fingerprint_of(&unchanged_id).as_str()),
        "the unchanged stamp is the run's frozen fingerprint"
    );
    assert_eq!(
        written.last_synced_at.as_deref(),
        Some("2026-11-21T10:30:00.123Z")
    );
    assert_eq!(
        written.last_synced_hash.as_deref(),
        Some(fingerprint_of(&written_id).as_str()),
        "the written stamp is the same rule, one transaction family earlier"
    );
    assert_ne!(
        unchanged.last_synced_hash, written.last_synced_hash,
        "the builds differ, so the fingerprints differ — each row's own value"
    );

    // Step 4 (MAIN-027): the derived answer is this run's finish.
    assert_eq!(last_success_of(&app).as_deref(), Some(FIRST_FINISH_AT));

    // §7.7/D-051: the dictionary — the two project results (both ok)
    // and the run summary (info/ok), and nothing else for this run.
    // `recent` answers newest first: the summary lands last, so it
    // reads first.
    assert_eq!(
        journal_of(&app, &run_id),
        vec![
            (Action::SyncRunFinished, Level::Info, Outcome::Ok),
            (Action::SyncProjectResult, Level::Info, Outcome::Ok),
            (Action::SyncProjectResult, Level::Info, Outcome::Ok),
            // T-124/D-223: phase 0 opened the run's bracket in its
            // own record transaction; newest-first, it reads last.
            (Action::SyncRunStarted, Level::Info, Outcome::Ok),
        ]
    );

    // Step 5's send half: exactly one after-commit pass that read
    // the COMMITTED verdict and finish stamp (a seam moved ahead of
    // the commit reads running/None and fails here), the default
    // retention found nothing to prune, and nothing was swallowed.
    assert_eq!(dirty_calls, 1, "the dirty seam runs exactly once");
    assert_eq!(
        seam_saw,
        Some(("success".to_owned(), Some(FIRST_FINISH_AT.to_owned()))),
        "the seam observes the committed run, not the in-flight one"
    );
    assert_eq!(outcome.retention.pruned, 0);
    assert!(outcome.retention.failures.is_empty());
    assert!(outcome.journal.failures.is_empty());
    assert!(outcome.dirty.failure.is_none());
}

#[test]
fn retention_applies_the_settings_limit_inside_the_finish() {
    // §9.4 phase 4 step 2 (BKP-003): with the limit at 2 and three
    // checked copies of one "project and file" pair (one real, from
    // phase 2 of the run; two older, planted through the record
    // writer over real files), the oldest leaves the disk and its
    // record goes `pruned` — inside the SAME transaction as the
    // verdict, which stays `success`.
    let (temp, app, repos) = phase4_app();
    let (project_id, repo) = seed_repo_project(&app, &repos, 1);
    seed_assigned_instruction(&app, &project_id, "Be backed up.");
    // The target exists, so phase 2 of the run copies it (the real,
    // checked copy — the newest of the pair).
    fs::write(repo.join("AGENTS.md"), old_sync_file("Old text."))
        .expect("pre-writing the target must work");

    let (run_id, phase1) = chain_through_phase3(&app, temp.path());
    // The two older copies: real files in the §5.5 tree, records
    // through the store writer, `created_at` older than the run's
    // (the real copy's) instant — distinct stamps and digests so the
    // names cannot collide.
    let (source_path, real_created_at) = {
        let database = database_of(&app);
        let run_id = run_id.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let row = tx
                .query_row(
                    "SELECT source_path, created_at FROM backup_records \
                     WHERE sync_run_id = ?1 LIMIT 1",
                    [&run_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<_, AppError>(row)
        }))
        .expect("reading the real copy's record must work")
    };
    assert_eq!(
        real_created_at, "2026-11-21T10:30:00.123Z",
        "the premise: the run's copy is the newest"
    );
    // The NAME's stamp hour matches the `created_at` hour, so the
    // on-disk name identifies its record and the oldest one is the
    // name the assertion expects to vanish.
    let older = [
        (8, "2026-11-20T08:00:00.000Z"),
        (9, "2026-11-20T09:00:00.000Z"),
    ];
    for (digit, created_at) in older {
        let digest = digit.to_string().repeat(64);
        let backup_path = backup_relative_path(
            &ProjectId::new(project_id.clone()),
            "Project 1",
            CoreScheme::A,
            CoreBaseName::AgentsMd,
            BackupTime {
                year: 2026,
                month: 11,
                day: 20,
                hour: u8::try_from(digit).expect("a small hour"),
                minute: 0,
                second: 0,
                millisecond: 0,
            },
            &digest,
        )
        .expect("building the layout path must work");
        let target = temp.path().join(&backup_path);
        fs::create_dir_all(target.parent().expect("the layout has a parent"))
            .expect("the backups folder must be creatable");
        fs::write(&target, b"an older copy").expect("the older copy must be creatable");
        let database = database_of(&app);
        let project_id = project_id.clone();
        let backup_path = backup_path.clone();
        let digest = digest.clone();
        let created_at = created_at.to_owned();
        let source_path = source_path.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::backup_records::insert(
                tx,
                &NewBackupRecord {
                    project_id: &project_id,
                    sync_run_id: None,
                    source_path: &source_path,
                    backup_path: &backup_path,
                    size_bytes: 13,
                    sha256_hex: &digest,
                    status: BackupRecordStatus::Ok,
                    created_at: &created_at,
                },
            )
            .map_err(|error| {
                crate::db::sqlite_error_to_app_error(match error {
                    eunomia_store::backup_records::BackupRecordsError::Sql(sql) => sql,
                    other => rusqlite::Error::InvalidParameterName(other.to_string()),
                })
            })
        }))
        .expect("planting the older record must work");
    }
    set_retention(&app, 2);
    assert_eq!(every_bak(temp.path()).len(), 3);

    let (outcome, _dirty_calls, _seam_saw) =
        run_phase4_counting(&app, &phase1, &run_id, temp.path(), first_finish_instant());

    // Exactly the newest two copies survive; the oldest is gone from
    // the disk and `pruned` in the records — and the verdict carries
    // the sweep's count, not an error.
    assert_eq!(outcome.retention.pruned, 1);
    assert!(outcome.retention.failures.is_empty());
    let surviving = every_bak(temp.path());
    assert_eq!(surviving.len(), 2, "keep=2 leaves exactly two copies");
    assert!(
        !surviving
            .iter()
            .any(|path| path.to_string_lossy().contains("20261120T08"),),
        "the oldest copy (08:00) is the one beyond the limit"
    );
    let (pruned, ok) = {
        let database = database_of(&app);
        tauri::async_runtime::block_on(database.run(|tx| {
            let pruned = tx
                .query_row(
                    "SELECT COUNT(*) FROM backup_records WHERE status = 'pruned'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(crate::db::sqlite_error_to_app_error)?;
            let ok = tx
                .query_row(
                    "SELECT COUNT(*) FROM backup_records WHERE status = 'ok'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<_, AppError>((pruned, ok))
        }))
        .expect("counting the record statuses must work")
    };
    assert_eq!((pruned, ok), (1, 2));
    assert_eq!(outcome.run.status, SyncRunStatus::Success);
    assert_eq!(run_row_of(&app, &run_id).status, SyncRunStatus::Success);
}

#[test]
fn a_retention_refusal_is_reported_and_never_changes_the_verdict() {
    // The module docs' ruling, pinned: a copy the sweep cannot remove
    // (a directory squatting on the `.bak` name — `remove_file`'
    // honest refusal) lands in the report, the record keeps its `ok`
    // status for the next run, and the run's verdict is untouched —
    // BKP-003's retention is housekeeping after the finish, not a
    // step of the verdict.
    let (temp, app, repos) = phase4_app();
    let (project_id, repo) = seed_repo_project(&app, &repos, 1);
    seed_assigned_instruction(&app, &project_id, "Be backed up.");
    fs::write(repo.join("AGENTS.md"), old_sync_file("Old text."))
        .expect("pre-writing the target must work");

    let (run_id, phase1) = chain_through_phase3(&app, temp.path());
    let source_path = {
        let database = database_of(&app);
        let run_id = run_id.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let source = tx
                .query_row(
                    "SELECT source_path FROM backup_records WHERE sync_run_id = ?1 LIMIT 1",
                    [&run_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<_, AppError>(source)
        }))
        .expect("reading the real copy's source path must work")
    };
    // The refused copy: a DIRECTORY at the layout name, older than
    // the run's real copy, with the limit at 1 — so it is the one
    // record the sweep must try to prune.
    let digest = "7".repeat(64);
    let backup_path = backup_relative_path(
        &ProjectId::new(project_id.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        BackupTime {
            year: 2026,
            month: 11,
            day: 20,
            hour: 7,
            minute: 0,
            second: 0,
            millisecond: 0,
        },
        &digest,
    )
    .expect("building the layout path must work");
    let squatting = temp.path().join(&backup_path);
    fs::create_dir_all(&squatting).expect("the squatting directory must be creatable");
    {
        let database = database_of(&app);
        let project_id = project_id.clone();
        let backup_path = backup_path.clone();
        let digest = digest.clone();
        let source_path = source_path.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::backup_records::insert(
                tx,
                &NewBackupRecord {
                    project_id: &project_id,
                    sync_run_id: None,
                    source_path: &source_path,
                    backup_path: &backup_path,
                    size_bytes: 0,
                    sha256_hex: &digest,
                    status: BackupRecordStatus::Ok,
                    created_at: "2026-11-20T07:00:00.000Z",
                },
            )
            .map_err(|error| {
                crate::db::sqlite_error_to_app_error(match error {
                    eunomia_store::backup_records::BackupRecordsError::Sql(sql) => sql,
                    other => rusqlite::Error::InvalidParameterName(other.to_string()),
                })
            })
        }))
        .expect("planting the refused record must work");
    }
    set_retention(&app, 1);

    let (outcome, _dirty_calls, _seam_saw) =
        run_phase4_counting(&app, &phase1, &run_id, temp.path(), first_finish_instant());

    // The refusal is a report line, the record keeps `ok`, the
    // directory is still there — and the verdict is `success`.
    assert_eq!(outcome.retention.pruned, 0, "nothing left the disk");
    assert_eq!(
        outcome.retention.failures.len(),
        1,
        "the refused removal is reported, exactly once"
    );
    assert!(squatting.is_dir(), "the obstacle survives the sweep");
    let ok_records = {
        let database = database_of(&app);
        tauri::async_runtime::block_on(database.run(|tx| {
            tx.query_row(
                "SELECT COUNT(*) FROM backup_records WHERE status = 'ok'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(crate::db::sqlite_error_to_app_error)
        }))
        .expect("counting the ok records must work")
    };
    assert_eq!(ok_records, 2, "both records keep their ok status");
    assert_eq!(outcome.run.status, SyncRunStatus::Success);
    assert_eq!(run_row_of(&app, &run_id).status, SyncRunStatus::Success);
}

// ---- the partial and failed verdicts ------------------------------------

#[test]
fn a_partial_run_keeps_the_overall_success_time_and_restamps_the_unchanged() {
    // §9.4 phase 4 step 3's partial arm over two REAL runs: the first
    // succeeds (both projects written) and fixes MAIN-027's answer;
    // the second loses one project to preflight (its folder is gone —
    // `skipped`) while the other's file already matches its build
    // (`unchanged`) — `partial`, the unchanged project re-stamped in
    // the written form at the second run's instant, and step 4's
    // pin: MAIN-027 stays the FIRST success's finish.
    let (temp, app, repos) = phase4_app();
    let (first_id, _first_repo) = seed_repo_project(&app, &repos, 1);
    let (second_id, second_repo) = seed_repo_project(&app, &repos, 2);
    seed_assigned_instruction(&app, &first_id, "First body.");
    seed_assigned_instruction(&app, &second_id, "Second body.");

    // Run 1: both written, finished at the first instant.
    let (first_run, first_phase1) = chain_through_phase3(&app, temp.path());
    let (first_outcome, first_dirty, _first_seam_saw) = run_phase4_counting(
        &app,
        &first_phase1,
        &first_run,
        temp.path(),
        first_finish_instant(),
    );
    assert_eq!(first_outcome.run.status, SyncRunStatus::Success);
    assert_eq!(first_dirty, 1);
    let first_fingerprints = fingerprints_of(&app, &first_run);

    // Run 2: the second project's folder is gone (preflight skips
    // it), the first project's file already holds the same build
    // (phase 1 says `unchanged`).
    fs::remove_dir_all(&second_repo).expect("removing the repo folder must work");
    let (second_run, second_phase1) = chain_through_phase3(&app, temp.path());
    assert_eq!(
        second_phase1
            .projects
            .iter()
            .filter(|(_, verdict)| {
                matches!(
                    verdict,
                    crate::sync_phase1::PreflightVerdict::Skipped { .. }
                )
            })
            .count(),
        1,
        "the fixture's premise: exactly one skipped project"
    );
    const SECOND_FINISH_AT: &str = "2026-11-21T12:15:00.789Z";
    let (second_outcome, second_dirty, second_seam_saw) = run_phase4_counting(
        &app,
        &second_phase1,
        &second_run,
        temp.path(),
        SECOND_FINISH_AT
            .parse()
            .expect("the second finish instant must parse"),
    );

    let row = run_row_of(&app, &second_run);
    assert_eq!(second_outcome.run.status, SyncRunStatus::Partial);
    assert_eq!(row.status, SyncRunStatus::Partial);
    assert_eq!(row.finished_at.as_deref(), Some(SECOND_FINISH_AT));
    assert_eq!(row.projects_written, 0);
    assert_eq!(row.projects_unchanged, 1);
    assert_eq!(row.projects_skipped, 1);
    assert_eq!(row.projects_failed, 0);
    assert_eq!(second_dirty, 1);
    assert_eq!(
        second_seam_saw,
        Some(("partial".to_owned(), Some(SECOND_FINISH_AT.to_owned()))),
        "the seam observes the committed partial verdict"
    );

    // MAIN-027's pin: the partial run did not move the overall time.
    assert_eq!(
        last_success_of(&app).as_deref(),
        Some(FIRST_FINISH_AT),
        "the overall time stays the first success's finish"
    );

    // The unchanged re-stamp: the second run's instant, the second
    // run's frozen fingerprint — which is the FIRST run's value (the
    // same build), the written form's ruling across runs.
    let first_project = project_of(&app, &first_id);
    assert_eq!(
        first_project.last_synced_at.as_deref(),
        Some(SECOND_FINISH_AT),
        "the unchanged project is re-stamped at the run's instant"
    );
    let second_fingerprints = fingerprints_of(&app, &second_run);
    let second_hash = second_fingerprints
        .iter()
        .find(|(id, _)| id == &first_id)
        .expect("the unchanged project's row must exist")
        .1
        .as_deref()
        .expect("phase 0 froze the fingerprint");
    let first_hash = first_fingerprints
        .iter()
        .find(|(id, _)| id == &first_id)
        .expect("the first run's row must exist")
        .1
        .as_deref()
        .expect("phase 0 froze the fingerprint");
    assert_eq!(second_hash, first_hash, "the same build, one fingerprint");
    assert_eq!(first_project.last_synced_hash.as_deref(), Some(second_hash));

    // The journal of the partial run (newest first): the summary's
    // warn/skipped chip (the module docs' ruling — the closed result
    // vocabulary's nearest member for a run that completed short of
    // full success), the skipped project's warn/skipped chip, and the
    // unchanged project's ok.
    assert_eq!(
        journal_of(&app, &second_run),
        vec![
            (Action::SyncRunFinished, Level::Warn, Outcome::Skipped),
            (Action::SyncProjectResult, Level::Warn, Outcome::Skipped),
            (Action::SyncProjectResult, Level::Info, Outcome::Ok),
            (Action::SyncRunStarted, Level::Info, Outcome::Ok),
        ]
    );
    // §10's field list on the error row (T-124/D-223): the SKIPPED
    // project's entry carries its §10 code, the project (its own
    // column) and — this task's reading — the row's own target path
    // (phase 0 froze it for every project; it is the honest "path"
    // of an error about that project's target). The OS code RIDES:
    // an unavailable folder IS an OS-call failure (the availability
    // probe's stat over the deleted folder), so reading 6's "sourced
    // from the outcomes" hands the entry the captured errno — the
    // expected fact is read off the phase-1 outcome (the parametric
    // oracle for "the journal copies the outcome's fact", platform-
    // independent by construction the T-110 way; the ipc battery
    // pins an injected errno 32 as the independent literal). The OK
    // entry needs no payload (an unchanged verdict claims no file).
    let records = journal_records_of(&app, &second_run);
    let skipped_entry = records
        .iter()
        .find(|record| {
            record.action == Action::SyncProjectResult
                && record.project_id.as_deref() == Some(second_id.as_str())
        })
        .expect("the skipped project's entry exists");
    let skipped_target = {
        let database = database_of(&app);
        let run_id = second_run.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::sync_runs::list_projects(tx, &run_id)
                .map_err(sync_runs_app_error_for_tests)
        }))
        .expect("reading the run's rows must work")
        .into_iter()
        .find(|row| row.project_id == second_id)
        .expect("the skipped project's row exists")
        .target_path
        .expect("phase 0 froze the target path")
    };
    let skipped_os_code = second_phase1
        .projects
        .iter()
        .find(|(project_id, _)| project_id == &second_id)
        .and_then(|(_, verdict)| match verdict {
            crate::sync_phase1::PreflightVerdict::Skipped { details, .. } => {
                details.as_ref().and_then(|facts| facts.os_code)
            }
            _ => None,
        })
        .expect("the unavailable-folder skip carries its OS fact");
    assert_eq!(
        skipped_entry.details,
        Some(serde_json::json!({
            "code": "E_PROJECT_UNAVAILABLE",
            "path": skipped_target,
            "os_code": skipped_os_code,
        })),
        "§10's fields on a skip: the code, the row's own target path, the OS code"
    );
    let ok_entry = records
        .iter()
        .find(|record| {
            record.action == Action::SyncProjectResult
                && record.project_id.as_deref() == Some(first_id.as_str())
        })
        .expect("the unchanged project's entry exists");
    assert_eq!(ok_entry.details, None);
}

#[test]
fn a_run_where_nobody_was_written_or_unchanged_fails() {
    // §9.4 phase 4 step 3's failed arm: both writes refuse (injected
    // through phase 3's `write` seam) — nobody was written or left
    // unchanged, the verdict is `failed` with both counters, the
    // journal carries the two error chips and the error summary, and
    // MAIN-027 has nothing to say (no success ever landed).
    let (temp, app, repos) = phase4_app();
    let (first_id, _first_repo) = seed_repo_project(&app, &repos, 1);
    let (second_id, _second_repo) = seed_repo_project(&app, &repos, 2);
    seed_assigned_instruction(&app, &first_id, "First body.");
    seed_assigned_instruction(&app, &second_id, "Second body.");

    // The chain with an all-refusing write: phases 0–2 for real, the
    // phase-3 seam answering the typed refusal every call.
    let phase0 =
        tauri::async_runtime::block_on(sync_phase0::run(app.handle())).expect("phase 0 must run");
    let phase1 = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        || Ok(u64::MAX),
        SystemTime::now(),
    ))
    .expect("phase 1 must pass");
    tauri::async_runtime::block_on(sync_phase2::run_with_abort_clock(
        app.handle(),
        &phase1,
        temp.path(),
        eunomia_fs::backup::backup_target_file,
        fixed_now(),
        jiff::Timestamp::now,
    ))
    .expect("phase 2 must succeed");
    let refusing_write = |_target: &Path, _contents: &[u8]| {
        Err(eunomia_fs::atomic::AtomicWriteError::WriteFailed(
            eunomia_fs::atomic::WriteFailed {
                step: eunomia_fs::atomic::WriteStep::TempCreate,
                path: std::path::PathBuf::from("/refused/AGENTS.md"),
                source: std::io::Error::other("injected refusal"),
                cleanup: None,
            },
        ))
    };
    let phase3 = tauri::async_runtime::block_on(sync_phase3::run(
        app.handle(),
        &phase1,
        refusing_write,
        || {
            eunomia_core::marker_clock::MarkerClock::from_instant(
                fixed_now(),
                jiff::tz::TimeZone::get("Europe/Prague").expect("Europe/Prague is in the tzdb"),
            )
        },
        fixed_now(),
    ))
    .expect("phase 3 must walk past the refusals");
    drop(phase0);

    let (outcome, _dirty_calls, seam_saw) = run_phase4_counting(
        &app,
        &phase1,
        &phase3.run_id,
        temp.path(),
        first_finish_instant(),
    );

    let row = run_row_of(&app, &phase3.run_id);
    assert_eq!(outcome.run.status, SyncRunStatus::Failed);
    assert_eq!(row.status, SyncRunStatus::Failed);
    assert_eq!(row.finished_at.as_deref(), Some(FIRST_FINISH_AT));
    assert_eq!(row.projects_written, 0);
    assert_eq!(row.projects_unchanged, 0);
    assert_eq!(row.projects_failed, 2);
    assert_eq!(row.details, None);
    // Neither project was stamped (OUT-020: a failed project keeps
    // its previous sync state — here: none).
    assert_eq!(project_of(&app, &first_id).last_synced_at, None);
    assert_eq!(project_of(&app, &second_id).last_synced_at, None);
    assert_eq!(
        last_success_of(&app),
        None,
        "no success ever landed — the overall time stays unknown"
    );
    assert_eq!(
        seam_saw,
        Some(("failed".to_owned(), Some(FIRST_FINISH_AT.to_owned()))),
        "the seam observes the committed failed verdict"
    );
    assert_eq!(
        journal_of(&app, &phase3.run_id),
        vec![
            (Action::SyncRunFinished, Level::Error, Outcome::Error),
            (Action::SyncProjectResult, Level::Error, Outcome::Error),
            (Action::SyncProjectResult, Level::Error, Outcome::Error),
            (Action::SyncRunStarted, Level::Info, Outcome::Ok),
        ]
    );
}

// ---- the empty run and the broken chain ---------------------------------

#[test]
fn the_empty_run_finishes_success_with_the_message() {
    // §9.4 phase 4 step 3's empty case over the real chain: a run
    // with no active projects is `success` — the §9.4 message in the
    // row's `details`, MAIN-027 takes its finish, the journal holds
    // exactly the run summary (no projects to report), and the dirty
    // seam still runs (§9.4 phase 4 step 5 has no "unless empty").
    let (temp, app, _repos) = phase4_app();
    let (run_id, phase1) = chain_through_phase3(&app, temp.path());
    let (outcome, dirty_calls, seam_saw) =
        run_phase4_counting(&app, &phase1, &run_id, temp.path(), first_finish_instant());

    let row = run_row_of(&app, &run_id);
    assert_eq!(outcome.run.status, SyncRunStatus::Success);
    assert_eq!(row.status, SyncRunStatus::Success);
    assert_eq!(row.finished_at.as_deref(), Some(FIRST_FINISH_AT));
    assert_eq!(row.projects_total, 0);
    assert_eq!(
        row.details.as_deref(),
        Some("no active projects to synchronize")
    );
    assert_eq!(last_success_of(&app).as_deref(), Some(FIRST_FINISH_AT));
    assert_eq!(
        journal_of(&app, &run_id),
        vec![
            (Action::SyncRunFinished, Level::Info, Outcome::Ok),
            (Action::SyncRunStarted, Level::Info, Outcome::Ok),
        ]
    );
    assert_eq!(outcome.journal.written, 1);
    assert_eq!(dirty_calls, 1);
    assert_eq!(
        seam_saw,
        Some(("success".to_owned(), Some(FIRST_FINISH_AT.to_owned())))
    );
    assert_eq!(outcome.retention.pruned, 0);
}

#[test]
fn a_run_with_an_unsettled_row_refuses_and_stays_running() {
    // The broken-chain refusal: phase 4 with a `not_started` row (a
    // phase of the chain did not settle its verdict — planted by
    // direct SQL, the state-machine scoping's making the natural
    // sequence unreachable) refuses with the class-only diagnostic,
    // the run row stays `running` (the crash-recovery rule's case),
    // nothing is journaled, and MAIN-027 has nothing to say.
    let (temp, app, repos) = phase4_app();
    let (first_id, _first_repo) = seed_repo_project(&app, &repos, 1);
    let (second_id, _second_repo) = seed_repo_project(&app, &repos, 2);
    seed_assigned_instruction(&app, &first_id, "First body.");
    seed_assigned_instruction(&app, &second_id, "Second body.");

    let (run_id, _phase1) = chain_through_phase3(&app, temp.path());
    // Both rows settled (`written`); break one back to `not_started`.
    {
        let database = database_of(&app);
        let run_id = run_id.clone();
        let second_id = second_id.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            tx.execute(
                "UPDATE sync_run_projects SET status = 'not_started', \
                 error_code = NULL, error_message = NULL \
                 WHERE sync_run_id = ?1 AND project_id = ?2",
                rusqlite::params![run_id, second_id],
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<(), AppError>(())
        }))
        .expect("planting the unsettled row must work");
    }

    let phase3 = Phase3Outcome {
        run_id: run_id.clone(),
        results: Vec::new(),
    };
    let phase1 = Phase1Outcome {
        run_id: run_id.clone(),
        projects: Vec::new(),
    };
    let error = tauri::async_runtime::block_on(sync_phase4::run(
        app.handle(),
        &phase1,
        &phase3,
        temp.path(),
        first_finish_instant(),
        |_| async { Ok(Vec::new()) },
    ))
    .expect_err("the unsettled run must refuse");

    let details = error.details.expect("the refusal carries details");
    assert_eq!(details.os_kind.as_deref(), Some("Phase4RowUnsettled"));
    let row = run_row_of(&app, &run_id);
    assert_eq!(row.status, SyncRunStatus::Running, "the run stays running");
    assert_eq!(row.finished_at, None);
    // Phase 4 journaled nothing OF ITS OWN: the run's journal holds
    // exactly phase 0's start bracket (T-124/D-223) — the refused
    // finish added no per-project entries and no summary.
    assert_eq!(
        journal_of(&app, &run_id),
        vec![(Action::SyncRunStarted, Level::Info, Outcome::Ok)],
        "a refused finish journals nothing of its own"
    );
    assert_eq!(last_success_of(&app), None);
}

#[test]
fn an_unchanged_row_without_a_fingerprint_refuses() {
    // The phase-3 family's other member, pinned the same way: an
    // `unchanged` row whose `build_hash` cell is NULL (only tampering
    // or a broken phase-0 chain produces one — the column is nullable
    // in the DDL, planted here by direct SQL) refuses with the
    // class-only `Phase4FingerprintMissing`, the run stays
    // `running`, and nothing is stamped or journaled.
    let (temp, app, repos) = phase4_app();
    let (first_id, _first_repo) = seed_repo_project(&app, &repos, 1);
    let (second_id, second_repo) = seed_repo_project(&app, &repos, 2);
    seed_assigned_instruction(&app, &first_id, "First body.");
    seed_assigned_instruction(&app, &second_id, "Second body.");
    fs::write(second_repo.join("AGENTS.md"), old_sync_file("Second body."))
        .expect("pre-writing the matched target must work");

    let (run_id, _phase1) = chain_through_phase3(&app, temp.path());
    {
        let database = database_of(&app);
        let run_id = run_id.clone();
        let second_id = second_id.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            tx.execute(
                "UPDATE sync_run_projects SET build_hash = NULL \
                 WHERE sync_run_id = ?1 AND project_id = ?2",
                rusqlite::params![run_id, second_id],
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<(), AppError>(())
        }))
        .expect("planting the missing fingerprint must work");
    }

    let phase3 = Phase3Outcome {
        run_id: run_id.clone(),
        results: Vec::new(),
    };
    let phase1 = Phase1Outcome {
        run_id: run_id.clone(),
        projects: Vec::new(),
    };
    let error = tauri::async_runtime::block_on(sync_phase4::run(
        app.handle(),
        &phase1,
        &phase3,
        temp.path(),
        first_finish_instant(),
        |_| async { Ok(Vec::new()) },
    ))
    .expect_err("the fingerprintless run must refuse");

    let details = error.details.expect("the refusal carries details");
    assert_eq!(details.os_kind.as_deref(), Some("Phase4FingerprintMissing"));
    let row = run_row_of(&app, &run_id);
    assert_eq!(row.status, SyncRunStatus::Running, "the run stays running");
    // Neither project was stamped BY PHASE 4: the unchanged one's
    // stamp never ran, and the written one's phase-3 stamp was
    // committed in its own short transaction before this phase —
    // a phase-4 refusal does not touch it (§9.4 phase 3 step 2).
    assert_eq!(project_of(&app, &second_id).last_synced_at, None);
    // Phase 4 journaled nothing of its own — the start bracket alone.
    assert_eq!(
        journal_of(&app, &run_id),
        vec![(Action::SyncRunStarted, Level::Info, Outcome::Ok)],
        "a refused finish journals nothing of its own"
    );
}

#[test]
fn a_retention_service_refusal_is_swallowed_into_the_report() {
    // The task card's own sentence — "retention errors do NOT fail
    // the run" — over the SERVICE's refusal, not the filesystem's: a
    // trigger that ABORTs the pruned-record UPDATE makes `apply`
    // answer `Err(StorageError::Sql)` (the statement-class failure
    // that leaves the transaction alive), and the phase must swallow
    // it into the report and finish the run `success`. The delete-
    // first residue is the D-122 posture spelled: the prunable copy's
    // FILE left the disk before the mark refused, its record keeps
    // `ok`, and the next run converges.
    let (temp, app, repos) = phase4_app();
    let (project_id, repo) = seed_repo_project(&app, &repos, 1);
    seed_assigned_instruction(&app, &project_id, "Be backed up.");
    fs::write(repo.join("AGENTS.md"), old_sync_file("Old text."))
        .expect("pre-writing the target must work");

    let (run_id, phase1) = chain_through_phase3(&app, temp.path());
    let source_path = {
        let database = database_of(&app);
        let run_id = run_id.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let source = tx
                .query_row(
                    "SELECT source_path FROM backup_records WHERE sync_run_id = ?1 LIMIT 1",
                    [&run_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<_, AppError>(source)
        }))
        .expect("reading the real copy's source path must work")
    };
    // The older, prunable copy — a real FILE this time, so the sweep
    // reaches the mark and the trigger answers there.
    let digest = "6".repeat(64);
    let backup_path = backup_relative_path(
        &ProjectId::new(project_id.clone()),
        "Project 1",
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        BackupTime {
            year: 2026,
            month: 11,
            day: 20,
            hour: 6,
            minute: 0,
            second: 0,
            millisecond: 0,
        },
        &digest,
    )
    .expect("building the layout path must work");
    let older_copy = temp.path().join(&backup_path);
    fs::create_dir_all(older_copy.parent().expect("the layout has a parent"))
        .expect("the backups folder must be creatable");
    fs::write(&older_copy, b"the prunable copy").expect("the older copy must be creatable");
    {
        let database = database_of(&app);
        let project_id = project_id.clone();
        let backup_path = backup_path.clone();
        let digest = digest.clone();
        let source_path = source_path.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::backup_records::insert(
                tx,
                &NewBackupRecord {
                    project_id: &project_id,
                    sync_run_id: None,
                    source_path: &source_path,
                    backup_path: &backup_path,
                    size_bytes: 19,
                    sha256_hex: &digest,
                    status: BackupRecordStatus::Ok,
                    created_at: "2026-11-20T06:00:00.000Z",
                },
            )
            .map_err(|error| {
                crate::db::sqlite_error_to_app_error(match error {
                    eunomia_store::backup_records::BackupRecordsError::Sql(sql) => sql,
                    other => rusqlite::Error::InvalidParameterName(other.to_string()),
                })
            })
        }))
        .expect("planting the prunable record must work");
    }
    set_retention(&app, 1);
    // The injected service refusal: every pruned-record UPDATE ABORTs.
    {
        let database = database_of(&app);
        tauri::async_runtime::block_on(database.run(|tx| {
            tx.execute_batch(
                "CREATE TRIGGER refuse_pruned_mark BEFORE UPDATE ON backup_records \
                 WHEN NEW.status = 'pruned' \
                 BEGIN SELECT RAISE(ABORT, 'injected retention refusal'); END",
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<(), AppError>(())
        }))
        .expect("planting the refusal trigger must work");
    }

    let (outcome, _dirty_calls, seam_saw) =
        run_phase4_counting(&app, &phase1, &run_id, temp.path(), first_finish_instant());

    assert_eq!(
        outcome.run.status,
        SyncRunStatus::Success,
        "the service refusal never fails the run"
    );
    assert_eq!(
        seam_saw,
        Some(("success".to_owned(), Some(FIRST_FINISH_AT.to_owned())))
    );
    assert_eq!(outcome.retention.pruned, 0, "the mark never happened");
    assert_eq!(
        outcome.retention.failures.len(),
        1,
        "the service refusal is reported, exactly once"
    );
    // The D-122 delete-first residue, pinned: the FILE left the disk
    // before the mark refused, and the record keeps its `ok` status
    // for the next run to converge.
    assert!(
        !older_copy.exists(),
        "the prunable copy's file was removed first"
    );
    let ok_records = {
        let database = database_of(&app);
        tauri::async_runtime::block_on(database.run(|tx| {
            tx.query_row(
                "SELECT COUNT(*) FROM backup_records WHERE status = 'ok'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(crate::db::sqlite_error_to_app_error)
        }))
        .expect("counting the ok records must work")
    };
    assert_eq!(ok_records, 2, "no record was marked pruned");
    assert_eq!(run_row_of(&app, &run_id).status, SyncRunStatus::Success);
}

#[test]
fn a_second_finish_of_one_run_refuses_before_any_stamp_or_prune() {
    // The step-A containment, behaviorally (the round-2 P3-1 fix):
    // phase 4 owns only the `running → finished` edge, so a second
    // finish of an already-finished run — and a finish over an
    // aborted run — refuses with the class-only
    // `Phase4RunNotRunning` BEFORE any stamp or prune, the run row
    // keeps its first verdict verbatim, and no journal entry is
    // added. Without the check, the second call would run its
    // stamps and prunes first and only then hit the store's zero-row
    // refusal, stranding the pruned files (the round-1 P3-4 finding).
    let (temp, app, repos) = phase4_app();
    let (project_id, _repo) = seed_repo_project(&app, &repos, 1);
    seed_assigned_instruction(&app, &project_id, "Be finished once.");

    // The finished case: one full chain, finished once.
    let (run_id, phase1) = chain_through_phase3(&app, temp.path());
    let (_outcome, _dirty, _seam_saw) =
        run_phase4_counting(&app, &phase1, &run_id, temp.path(), first_finish_instant());
    let first_row = run_row_of(&app, &run_id);
    let first_journal = journal_of(&app, &run_id);
    assert_eq!(first_row.status, SyncRunStatus::Success);
    assert_eq!(
        first_journal.len(),
        3,
        "the start bracket, one project entry and the summary"
    );

    // The second finish of the same run.
    let phase3 = Phase3Outcome {
        run_id: run_id.clone(),
        results: Vec::new(),
    };
    let no_facts = Phase1Outcome {
        run_id: run_id.clone(),
        projects: Vec::new(),
    };
    let error = tauri::async_runtime::block_on(sync_phase4::run(
        app.handle(),
        &no_facts,
        &phase3,
        temp.path(),
        first_finish_instant(),
        |_| async { Ok(Vec::new()) },
    ))
    .expect_err("a second finish of one run must refuse");
    let details = error.details.expect("the refusal carries details");
    assert_eq!(details.os_kind.as_deref(), Some("Phase4RunNotRunning"));
    assert_eq!(
        run_row_of(&app, &run_id),
        first_row,
        "the first verdict stays verbatim"
    );
    assert_eq!(
        journal_of(&app, &run_id),
        first_journal,
        "no journal entry is added by the refused finish"
    );

    // The aborted case: a fresh run, aborted by the phase-1 tail rule
    // through the store's own writer, then handed to phase 4.
    let (aborted_id, aborted_phase1) = chain_through_phase3(&app, temp.path());
    {
        let database = database_of(&app);
        let aborted_id = aborted_id.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::sync_runs::abort_run(
                tx,
                &aborted_id,
                "2026-11-21T12:00:00.000Z",
                "E_DISK_FULL",
                "free 0 bytes, required 1 byte",
            )
            .map_err(sync_runs_app_error_for_tests)
        }))
        .expect("aborting the run must work");
    }
    let phase3 = Phase3Outcome {
        run_id: aborted_id.clone(),
        results: Vec::new(),
    };
    let error = tauri::async_runtime::block_on(sync_phase4::run(
        app.handle(),
        &aborted_phase1,
        &phase3,
        temp.path(),
        first_finish_instant(),
        |_| async { Ok(Vec::new()) },
    ))
    .expect_err("a finish over an aborted run must refuse");
    let details = error.details.expect("the refusal carries details");
    assert_eq!(details.os_kind.as_deref(), Some("Phase4RunNotRunning"));
    assert_eq!(
        run_row_of(&app, &aborted_id).status,
        SyncRunStatus::Aborted,
        "the abort's verdict stays"
    );
    // The aborted run keeps phase 0's start bracket alone: phase 4
    // added nothing (a store-writer abort — this test's shortcut —
    // journals no cancel entry of its own; the app-layer abort paths
    // do, in their own transactions).
    assert_eq!(
        journal_of(&app, &aborted_id),
        vec![(Action::SyncRunStarted, Level::Info, Outcome::Ok)],
        "the aborted run keeps phase 0's bracket and nothing more"
    );
}

#[test]
fn an_unreadable_retention_setting_is_swallowed_into_the_report() {
    // The other swallowed service branch: a `backup.retention` cell
    // the settings repository refuses (out of the 1–200 range,
    // planted by direct SQL — D-039's error-instead-of-default rule)
    // keeps the limit from being applied at all, is reported in the
    // class-only sentence, and — the point — never fails the run.
    let (temp, app, repos) = phase4_app();
    let (project_id, _repo) = seed_repo_project(&app, &repos, 1);
    seed_assigned_instruction(&app, &project_id, "Be written.");

    let (run_id, phase1) = chain_through_phase3(&app, temp.path());
    {
        let database = database_of(&app);
        tauri::async_runtime::block_on(database.run(|tx| {
            tx.execute(
                "INSERT INTO settings (key, value, updated_at) VALUES \
                 ('backup.retention', '0', '2026-11-20T00:00:00.000Z')",
                [],
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok::<(), AppError>(())
        }))
        .expect("planting the out-of-range setting must work");
    }

    let (outcome, _dirty_calls, _seam_saw) =
        run_phase4_counting(&app, &phase1, &run_id, temp.path(), first_finish_instant());

    assert_eq!(
        outcome.run.status,
        SyncRunStatus::Success,
        "an unreadable limit never fails the run"
    );
    assert_eq!(outcome.retention.pruned, 0);
    assert_eq!(outcome.retention.failures.len(), 1);
    assert!(
        outcome.retention.failures[0].contains("backup.retention"),
        "the report names the setting, class-only: {:?}",
        outcome.retention.failures[0]
    );
    assert_eq!(run_row_of(&app, &run_id).status, SyncRunStatus::Success);
}
// ---- T-324: the partial run's mirror through the real phase -----

/// T-324/D-228, end to end through the real phase: a `partial` run
/// journals its skipped project at `warn` and its `sync.run_finished`
/// summary at `warn`, and those — and ONLY those — reach the file
/// log, exactly once each: the flush happens after the verdict
/// transaction commits, on the executor's blocking thread, so the
/// observation goes through the `cfg(test)` emission recorder in
/// `logging::test_support` (a process-global subscriber cannot serve
/// two tests in a same-process `cargo test` run, the CI gate's
/// shape), with every assert scoped by the runs' own unique ids —
/// the fixture's real UUIDv7 run ids, immune to any parallel test's
/// emissions. The unmirrored half is pinned too: the info/user/ok
/// shapes of the same run (`sync.run_started`, the written/unchanged
/// project entries) produce no emission, and the first (fully
/// successful) run's phase-4 entries are absent for the same reason.
#[test]
fn a_partial_run_mirrors_its_warn_entries_exactly_once() {
    // The partial-run fixture of
    // `a_partial_run_keeps_the_overall_success_time_and_restamps_the_
    // unchanged`: two projects, run 1 writes both, run 2 skips one
    // (its folder is gone) and finds the other unchanged.
    let (temp, app, repos) = phase4_app();
    const SECOND_FINISH_AT: &str = "2026-11-21T12:15:00.789Z";
    let (first_id, _first_repo) = seed_repo_project(&app, &repos, 1);
    let (second_id, second_repo) = seed_repo_project(&app, &repos, 2);
    seed_assigned_instruction(&app, &first_id, "First body.");
    seed_assigned_instruction(&app, &second_id, "Second body.");

    // Run 1: both written, finished — all info/user/ok shapes, no
    // mirror emission of theirs may exist.
    let (first_run, first_phase1) = chain_through_phase3(&app, temp.path());
    let (_first_outcome, _first_dirty, _first_seam) = run_phase4_counting(
        &app,
        &first_phase1,
        &first_run,
        temp.path(),
        first_finish_instant(),
    );

    // Run 2: the second project's folder is gone — `skipped` at warn,
    // the run's verdict `partial`.
    fs::remove_dir_all(&second_repo).expect("removing the repo folder must work");
    let (second_run, second_phase1) = chain_through_phase3(&app, temp.path());
    let (second_outcome, _second_dirty, _second_seam) = run_phase4_counting(
        &app,
        &second_phase1,
        &second_run,
        temp.path(),
        SECOND_FINISH_AT
            .parse()
            .expect("the second finish instant must parse"),
    );
    assert_eq!(second_outcome.run.status, SyncRunStatus::Partial);

    // The journal side: the run's entries exist (the mirror's
    // referents) — the warn summary, the skipped project's warn row,
    // the unchanged project's info row, and the run's bracket opener
    // (info/user/ok).
    let journal = journal_of(&app, &second_run);
    assert_eq!(
        journal,
        vec![
            (Action::SyncRunFinished, Level::Warn, Outcome::Skipped),
            (Action::SyncProjectResult, Level::Warn, Outcome::Skipped),
            (Action::SyncProjectResult, Level::Info, Outcome::Ok),
            (Action::SyncRunStarted, Level::Info, Outcome::Ok),
        ],
        "the partial run's journal: the warn summary, the skipped \
         project's warn row, the unchanged project's info row, the \
         run's opener"
    );

    // The emission side: exactly one line per warn entry, with the
    // run's and project's own ids — and nothing from the unmirrored
    // shapes.
    let emitted = crate::logging::test_support::emitted_mirror_lines();
    let first_run_lines: Vec<_> = emitted
        .iter()
        .filter(|line| line.sync_run_id == first_run)
        .collect();
    assert!(
        first_run_lines.is_empty(),
        "the fully successful run's info/user/ok entries mirror nothing \
         (the policy half, end to end), got: {first_run_lines:?}"
    );
    let second_run_lines: Vec<_> = emitted
        .iter()
        .filter(|line| line.sync_run_id == second_run)
        .collect();
    assert_eq!(
        second_run_lines.len(),
        2,
        "exactly two mirror lines for the partial run — the skipped \
         project's warn row and the warn summary (T-324/D-228), got: \
         {second_run_lines:?}"
    );
    let project_lines: Vec<_> = second_run_lines
        .iter()
        .filter(|line| line.action == "sync.project_result")
        .collect();
    assert_eq!(project_lines.len(), 1, "one project-result line");
    assert_eq!(project_lines[0].level, tracing::Level::WARN);
    assert_eq!(project_lines[0].result, "skipped");
    assert_eq!(project_lines[0].entity_type, "project");
    assert_eq!(
        project_lines[0].entity_id, second_id,
        "the skipped project's line names its project"
    );
    assert_eq!(project_lines[0].project_id, second_id);
    let summary_lines: Vec<_> = second_run_lines
        .iter()
        .filter(|line| line.action == "sync.run_finished")
        .collect();
    assert_eq!(summary_lines.len(), 1, "one run-finished line");
    assert_eq!(summary_lines[0].level, tracing::Level::WARN);
    assert_eq!(summary_lines[0].result, "skipped");
    assert!(
        second_run_lines
            .iter()
            .all(|line| line.action != "sync.run_started"),
        "the run's info/user/ok opener stays unmirrored, got: \
         {second_run_lines:?}"
    );
    drop(app);
    drop(temp);
}
