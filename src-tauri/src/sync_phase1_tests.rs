//! Behavior tests of the sync pipeline's phase 1 (T-113, §9.4 phase
//! 1, §10's skip rows, D-09/D-24/D-25, OUT-008).
//!
//! The suite drives the phase the way production reaches it —
//! [`sync_phase1::run`] over a real phase-0 outcome recorded by
//! [`sync_phase0::run`] — against REAL project folders in a temp
//! root (`.git` directory, target files on disk), so every step's
//! verdict is the dependency's own probe answering over a real
//! fixture, never a mock:
//!
//! * the card's criterion 1 twice: an unavailable project and a
//!   rights-refused project are `skipped` with their §10 codes, and
//!   the run continues to the healthy projects — not `aborted`;
//! * the card's criterion 2: the injected free-space seam (the
//!   atomic.rs/D-089 pattern — a plain number instead of a volume
//!   probe) below the sum-plus-margin line aborts the whole run with
//!   `E_DISK_FULL`, the exact-equality boundary passes, and an empty
//!   copy list never probes at all;
//! * every step 1–7 by name, each with the §10 code or product the
//!   spec pins (steps 2 and 4 carry several verdicts each);
//! * the ALG-033 `unchanged` verdict (A and B forms, the setting's
//!   gate) and the D-25 confirmation list with its idempotence edge;
//! * the order (the projects' `sort_order`), the run-row states the
//!   phase leaves behind, and the "changes nothing" rule itself:
//!   every test that reaches a healthy target file asserts its bytes
//!   verbatim.
//!
//! The mock app is the `phase0_app` shape (mock runtime, real
//! database in a temp root, `SyncRunning` managed): phase 1 needs
//! the database and phase 0's outcome, and the flag the outcome
//! holds is the run's own lease. The harness owns its copy because
//! the sibling suites' helpers are private to them.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_fs::free_space::FreeSpaceError;
use eunomia_store::projects::NewProject;
use eunomia_store::settings::{BaseName, SettingKey, SettingValue};
use eunomia_store::sync_runs::SyncRun;

use crate::commands::begin_sync;
use crate::data_dirs;
use crate::db::Db;
use crate::errors::{AppError, ErrorCode, ErrorDetails};
use crate::sync_phase0::{self, Phase0Outcome};
use crate::sync_phase1::{self, Phase1Outcome, PreflightVerdict};
use crate::sync_running::SyncRunning;

/// The ALG-032 example marker, verbatim — a marker line of the exact
/// shape the comparison and the fixtures recognize, with a timestamp
/// deliberately older than "now" (the timestamp never decides).
const MARKER: &str = "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

// ---- the harness ------------------------------------------------------

/// A mock app in the healthy state the phase serves (the
/// `sync_phase0_tests.rs` harness shape: mock runtime, a fresh
/// database in a temp root, the `SyncRunning` flag managed), plus the
/// `repos/` root the fixtures live under.
fn phase1_app() -> (
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
/// `Sql` is reachable on a database this suite just wrote). The
/// non-`Sql` fallback deliberately embeds the error's `Display` —
/// the richest panic message when an assertion-side read fails — and
/// never reaches the wire; the wire's class-only rule lives in
/// `db::sync_runs_to_app_error` (SEC-008).
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

/// Creates a real repository folder (`repos/repo-<n>` with a `.git`
/// directory) and seeds its project row with the canonical path —
/// the DB-030 posture the target-path resolution contracts on.
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
    // The canonical form (macOS temp fixtures live behind /var): what
    // the availability probe and the target resolution contract on.
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
/// needs (the D-093 fallback family, the `sync_phase0_tests.rs`
/// shape).
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
/// verbatim (extras off, the `sync_phase0_tests.rs` shape). The title
/// carries a process-wide counter — visible instruction titles are
/// unique in the library, and one test seeds several of these.
fn seed_assigned_instruction(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
    content: &'static str,
) {
    static SEEDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seeded = SEEDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

/// Flips `sync.skip_identical` through the settings repository — the
/// production write path of §6.5.
fn set_skip_identical(app: &tauri::App<tauri::test::MockRuntime>, enabled: bool) {
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::settings::set(
            tx,
            SettingKey::SyncSkipIdentical,
            SettingValue::Bool(enabled),
        )
        .map_err(crate::commands::settings_write_to_app_error)
    }))
    .expect("setting sync.skip_identical must work");
}

/// Runs phase 0 and phase 1 back to back over the app's database,
/// with the given free-space probe and the real clock — the
/// composition every passing test starts from. The phase-0 outcome
/// (and its flag lease) is dropped inside: tests that run the
/// pipeline again must not trip the `E_SYNC_RUNNING` gate on their
/// own leftover guard, and the run's id travels in the phase-1
/// outcome.
fn preflight_ok(
    app: &tauri::App<tauri::test::MockRuntime>,
    free_space: impl FnOnce() -> Result<u64, FreeSpaceError> + Send + 'static,
) -> Phase1Outcome {
    let phase0 = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over the seeded database");
    let outcome = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        free_space,
        SystemTime::now(),
    ))
    .expect("phase 1 must pass");
    drop(phase0);
    outcome
}

/// The plenty-everywhere free-space probe.
fn plenty() -> Result<u64, FreeSpaceError> {
    Ok(u64::MAX)
}

/// The verdict of one project, by id, from the outcome.
fn verdict_of<'a>(outcome: &'a Phase1Outcome, project_id: &str) -> &'a PreflightVerdict {
    &outcome
        .projects
        .iter()
        .find(|(id, _)| id == project_id)
        .expect("the project must be in the outcome")
        .1
}

/// The §10 literal a skipped verdict carries.
fn skip_code_of(verdict: &PreflightVerdict) -> &'static str {
    let PreflightVerdict::Skipped { code, .. } = verdict else {
        panic!("the verdict must be skipped, got {verdict:?}");
    };
    code.as_str()
}

/// Marks a file read-only the portable way (std's `set_readonly`,
/// honored by the write-access probe on every platform, root
/// included).
fn make_read_only(path: &Path) {
    let mut perms = fs::metadata(path)
        .expect("stating the fixture file must work")
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms).expect("marking the fixture read-only must work");
}

/// A staging-grammar tmp file name for a target (D-088's grammar, the
/// exact spelling the sweep recognizes).
fn tmp_name(target_name: &str, token: &str) -> String {
    format!(".{target_name}.eunomia-{token}.tmp")
}

/// The old-sync A/C file form of `managed`: the managed text, a blank
/// line, the marker, one break (ALG-030) — what a previous run's
/// write phase left on disk.
fn old_sync_file(managed: &str) -> String {
    if managed.is_empty() {
        format!("{MARKER}\n")
    } else {
        format!("{managed}\n\n{MARKER}\n")
    }
}

/// The old-sync scheme-B file form: the marker-fenced managed block
/// between BEGIN and END (ALG-031), wrapped in the given outside
/// text.
fn old_sync_b_file(managed: &str, head: &str, tail: &str) -> String {
    let block = if managed.is_empty() {
        format!("{MARKER}\n")
    } else {
        format!("{managed}\n\n{MARKER}\n")
    };
    format!("{head}<!-- EUNOMIASYNC:BEGIN -->\n{block}<!-- EUNOMIASYNC:END -->\n{tail}")
}

/// The sorted entry names of a folder — the sweep assertions' view.
fn entries_of(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("listing the folder must work")
        .map(|entry| {
            entry
                .expect("reading an entry must work")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

// ---- criterion 1: a skipped project does not stop the run ----------

#[test]
fn an_unavailable_project_is_skipped_and_the_run_continues() {
    // §9.4 phase 1 step 1 / D-09: a project whose folder is gone is
    // `skipped` with E_PROJECT_UNAVAILABLE, and the run continues —
    // the healthy project still gets its verdict, the run row stays
    // `running` (not aborted), and not one byte of the healthy
    // target moves.
    let (_temp, app, repos) = phase1_app();
    let (gone, _gone_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    fs::remove_dir_all(repos.join("repo-1")).expect("removing the folder must work");
    let (healthy, healthy_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &gone, "Gone body.");
    seed_assigned_instruction(&app, &healthy, "Be brief.");
    let healthy_target = healthy_repo.join("AGENTS.md");
    fs::write(&healthy_target, b"old bytes").expect("writing the fixture must work");

    let outcome = preflight_ok(&app, plenty);

    assert_eq!(
        skip_code_of(verdict_of(&outcome, &gone)),
        "E_PROJECT_UNAVAILABLE"
    );
    // The healthy project is ready (the fixture's bytes differ from
    // the build) — the skip did not stop the loop.
    assert!(matches!(
        verdict_of(&outcome, &healthy),
        PreflightVerdict::Ready(_)
    ));
    // The database: the skipped row with its code, the ready row
    // still `not_started` (phase 3's), the run row still `running`.
    let rows = run_rows_of(&app, &outcome.run_id);
    assert_eq!(
        rows[0].status,
        eunomia_store::sync_runs::SyncRunProjectStatus::Skipped
    );
    assert_eq!(rows[0].error_code.as_deref(), Some("E_PROJECT_UNAVAILABLE"));
    assert!(
        rows[0]
            .error_message
            .as_deref()
            .is_some_and(|m| !m.is_empty()),
        "the skip row carries a message: {:?}",
        rows[0].error_message
    );
    assert_eq!(
        rows[1].status,
        eunomia_store::sync_runs::SyncRunProjectStatus::NotStarted
    );
    assert_eq!(
        run_row_of(&app, &outcome.run_id).status,
        eunomia_store::sync_runs::SyncRunStatus::Running
    );
    // "Changes nothing": the healthy target's bytes are verbatim.
    assert_eq!(
        fs::read(&healthy_target).expect("reading the target back must work"),
        b"old bytes"
    );
}

#[test]
fn a_read_only_target_is_skipped_and_the_run_continues() {
    // §9.4 phase 1 step 3 / T-F5 / §10's E_NO_WRITE_ACCESS row: a
    // read-only target file skips the project — the file-flag half of
    // the rights check, portable (std's flag is honored on every
    // platform, root included) — and the run continues.
    let (_temp, app, repos) = phase1_app();
    let (locked, locked_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (healthy, _healthy_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &locked, "Be brief.");
    seed_assigned_instruction(&app, &healthy, "Be brief.");
    let locked_target = locked_repo.join("AGENTS.md");
    fs::write(&locked_target, b"locked bytes").expect("writing the fixture must work");
    make_read_only(&locked_target);

    let outcome = preflight_ok(&app, plenty);

    assert_eq!(
        skip_code_of(verdict_of(&outcome, &locked)),
        "E_NO_WRITE_ACCESS"
    );
    assert!(matches!(
        verdict_of(&outcome, &healthy),
        PreflightVerdict::Ready(_)
    ));
}

// ---- criterion 2: the free-space tail --------------------------------

#[test]
fn a_free_space_shortage_aborts_the_run_with_e_disk_full() {
    // §9.4's closing rule / §10's E_DISK_FULL row: one ready project
    // whose existing file is 100 bytes needs 100 + 20% = 120 bytes of
    // free space (the margin spelled independently here, from the
    // spec's "the total size of the copies plus a 20% margin"); a
    // probe answering 119 aborts the whole run — the run row
    // `aborted` with the code, the ready row still `not_started`, the
    // target's bytes verbatim, and the caller hears the typed
    // envelope with the numbers.
    let (_temp, app, repos) = phase1_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    let target = repo.join("AGENTS.md");
    let fixture = b"x".repeat(100);
    fs::write(&target, &fixture).expect("writing the fixture must work");

    let phase0 =
        tauri::async_runtime::block_on(sync_phase0::run(app.handle())).expect("phase 0 must run");
    let refusal = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        || Ok(119u64),
        SystemTime::now(),
    ))
    .expect_err("the shortage must abort the run");

    assert_eq!(refusal.code, ErrorCode::DiskFull);
    let details = refusal.details.expect("the envelope carries the numbers");
    assert_eq!(details.os_kind.as_deref(), Some("NotEnoughSpace"));
    assert_eq!(
        details.os_message.as_deref(),
        Some("free 119 bytes, required 120 bytes")
    );

    // The run row: aborted with the §10 literal and a finish stamp;
    // the project row keeps its `not_started` (nothing claims a
    // write that did not happen); the target is untouched.
    let run = run_row_of(&app, &phase0.run.id);
    assert_eq!(run.status, eunomia_store::sync_runs::SyncRunStatus::Aborted);
    assert_eq!(run.error_code.as_deref(), Some("E_DISK_FULL"));
    assert!(run.finished_at.is_some(), "the aborted run is finished");
    assert!(run.details.as_deref().is_some_and(|d| d.contains("119")));
    let rows = run_rows_of(&app, &phase0.run.id);
    assert_eq!(
        rows[0].status,
        eunomia_store::sync_runs::SyncRunProjectStatus::NotStarted
    );
    assert_eq!(
        fs::read(&target).expect("reading the target back must work"),
        fixture
    );
}

#[test]
fn the_exact_free_space_boundary_passes_and_a_backupless_run_never_probes() {
    // The strict-less-than ruling (D-089): free == required passes —
    // exact equality is not a shortage.
    let (_temp, app, repos) = phase1_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    fs::write(repo.join("AGENTS.md"), b"x".repeat(100)).expect("writing the fixture must work");

    let outcome = preflight_ok(&app, || Ok(120u64));
    assert!(matches!(
        verdict_of(&outcome, &project),
        PreflightVerdict::Ready(_)
    ));

    // An empty copy list (no project has a file to back up) skips
    // the probe entirely: the closure refuses to be called.
    let (_temp2, app2, repos2) = phase1_app();
    let (fresh, fresh_repo) = seed_repo_project(
        &app2,
        &repos2,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app2, &fresh, "Be brief.");
    let outcome2 = preflight_ok(&app2, || {
        panic!("a backupless run must not consult the free-space probe")
    });
    let PreflightVerdict::Ready(ready) = verdict_of(&outcome2, &fresh) else {
        panic!("the creation-case project must be ready");
    };
    assert_eq!(ready.existing_size, None, "no existing file, no copy");
    assert!(
        !fresh_repo.join("AGENTS.md").exists(),
        "preflight creates nothing"
    );
}

#[test]
fn a_failed_free_space_probe_proceeds_without_aborting() {
    // The module docs' proceed ruling (the atomic.rs write-time
    // posture): only a DEFINITIVE shortage aborts — a probe that
    // could not answer (a dropped volume, a refused statvfs) is not
    // a shortage, the run continues, and phase 2's own backup
    // failure (`E_BACKUP_FAILED`) is the honest reporter of a volume
    // that cannot take the copies.
    let (_temp, app, repos) = phase1_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    fs::write(repo.join("AGENTS.md"), b"x".repeat(100)).expect("writing the fixture must work");

    let outcome = preflight_ok(&app, || {
        Err(FreeSpaceError::ProbeFailed {
            source: std::io::Error::from_raw_os_error(5),
        })
    });
    assert!(matches!(
        verdict_of(&outcome, &project),
        PreflightVerdict::Ready(_)
    ));
    assert_eq!(
        run_row_of(&app, &outcome.run_id).status,
        eunomia_store::sync_runs::SyncRunStatus::Running,
        "a failed probe must not abort the run"
    );
}

// ---- step 2: the resolved target path -------------------------------

#[cfg(unix)]
#[test]
fn target_path_refusals_skip_with_their_own_codes() {
    // §9.4 phase 1 step 2's three refusal verdicts, each against its
    // own fixture: a dangling link (E_TARGET_BROKEN_LINK), a link
    // resolving outside the project root (E_TARGET_OUTSIDE_ROOT), and
    // a folder sitting on the target name (E_TARGET_IS_DIR).
    let (temp, app, repos) = phase1_app();
    let (broken, broken_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::ClaudeMd,
    );
    let (outside, outside_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (is_dir, is_dir_repo) = seed_repo_project(
        &app,
        &repos,
        3,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    for (project, content) in [(&broken, "A."), (&outside, "B."), (&is_dir, "C.")] {
        seed_assigned_instruction(&app, project, content);
    }

    // A dangling link: CLAUDE.md points at a name that does not exist.
    std::os::unix::fs::symlink("nothing-here.md", broken_repo.join("CLAUDE.md"))
        .expect("creating the dangling link must work");
    // A link outside the root: AGENTS.md points at a real file in the
    // temp root — outside the registered root.
    let outside_file = temp.path().join("elsewhere.md");
    fs::write(&outside_file, b"outside").expect("writing the fixture must work");
    std::os::unix::fs::symlink(&outside_file, outside_repo.join("AGENTS.md"))
        .expect("creating the outside link must work");
    // A folder on the target name.
    fs::create_dir(is_dir_repo.join("AGENTS.md")).expect("creating the folder must work");

    let outcome = preflight_ok(&app, plenty);
    assert_eq!(
        skip_code_of(verdict_of(&outcome, &broken)),
        "E_TARGET_BROKEN_LINK"
    );
    assert_eq!(
        skip_code_of(verdict_of(&outcome, &outside)),
        "E_TARGET_OUTSIDE_ROOT"
    );
    assert_eq!(
        skip_code_of(verdict_of(&outcome, &is_dir)),
        "E_TARGET_IS_DIR"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_target_resolves_to_the_real_file() {
    // §9.4 phase 1 step 2's acceptance case (T-F6): `CLAUDE.md`
    // points at `AGENTS.md`; the write target is the real file — and
    // the ALG-033 comparison reads the REAL file through the link
    // (an old-sync AGENTS.md behind the link is `unchanged`).
    let (_temp, app, repos) = phase1_app();
    let (project, repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::ClaudeMd,
    );
    seed_assigned_instruction(&app, &project, "Be brief.");
    let real = repo.join("AGENTS.md");
    fs::write(&real, old_sync_file("Be brief.")).expect("writing the fixture must work");
    std::os::unix::fs::symlink("AGENTS.md", repo.join("CLAUDE.md"))
        .expect("creating the link must work");

    let outcome = preflight_ok(&app, plenty);
    // The comparison read the real file through the link.
    assert_eq!(verdict_of(&outcome, &project), &PreflightVerdict::Unchanged);

    // And with a differing real file, the ready product names the
    // real file — the link is NOT the write target, and the resolved
    // name keys staging and sweeping (D-088/D-092).
    fs::write(&real, old_sync_file("Different.")).expect("writing the fixture must work");
    let outcome_b = preflight_ok(&app, plenty);
    let PreflightVerdict::Ready(ready) = verdict_of(&outcome_b, &project) else {
        panic!("the changed real file must be ready");
    };
    assert_eq!(ready.resolved_path, real);
    assert_eq!(ready.resolved_name, "AGENTS.md");
}

// ---- step 4: the existing file, scheme B's gates ---------------------

#[test]
fn scheme_b_refuses_non_utf8_and_broken_markers() {
    // §9.4 phase 1 step 4 / §10's two scheme-B rows: a non-UTF-8 file
    // (E_NOT_UTF8) and the §7.5 marker failures (E_MARKERS_INVALID:
    // no markers, two pairs, reversed order), plus SYS-009's deleted
    // target — a skip for scheme B, not a creation.
    let (_temp, app, repos) = phase1_app();
    let ids: Vec<String> = (1..=4)
        .map(|n| {
            seed_repo_project(
                &app,
                &repos,
                n,
                eunomia_store::settings::Scheme::B,
                BaseName::AgentsMd,
            )
            .0
        })
        .collect();
    for (index, id) in ids.iter().enumerate() {
        seed_assigned_instruction(&app, id, ["A.", "B.", "C.", "D."][index]);
    }
    let repo = |n: usize| repos.join(format!("repo-{n}"));

    // Non-UTF-8 bytes between a valid marker pair.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"<!-- EUNOMIASYNC:BEGIN -->\n");
    bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
    bytes.extend_from_slice(b"<!-- EUNOMIASYNC:END -->\n");
    fs::write(repo(1).join("AGENTS.md"), bytes).expect("writing the fixture must work");
    // No markers at all.
    fs::write(repo(2).join("AGENTS.md"), "Just prose.\n").expect("writing the fixture must work");
    // Two pairs.
    fs::write(
        repo(3).join("AGENTS.md"),
        format!(
            "{}{}",
            old_sync_b_file("A.", "", ""),
            "<!-- EUNOMIASYNC:BEGIN -->\nB.\n<!-- EUNOMIASYNC:END -->\n"
        ),
    )
    .expect("writing the fixture must work");
    // Reversed order.
    fs::write(
        repo(4).join("AGENTS.md"),
        "<!-- EUNOMIASYNC:END -->\n<!-- EUNOMIASYNC:BEGIN -->\n",
    )
    .expect("writing the fixture must work");

    let outcome = preflight_ok(&app, plenty);
    assert_eq!(skip_code_of(verdict_of(&outcome, &ids[0])), "E_NOT_UTF8");
    assert_eq!(
        skip_code_of(verdict_of(&outcome, &ids[1])),
        "E_MARKERS_INVALID"
    );
    assert_eq!(
        skip_code_of(verdict_of(&outcome, &ids[2])),
        "E_MARKERS_INVALID"
    );
    assert_eq!(
        skip_code_of(verdict_of(&outcome, &ids[3])),
        "E_MARKERS_INVALID"
    );

    // SYS-009: the deleted scheme-B target is the same skip — a
    // first sync of a fresh scheme-B project without markers on disk
    // refuses too.
    let (deleted, _repo) = seed_repo_project(
        &app,
        &repos,
        5,
        eunomia_store::settings::Scheme::B,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &deleted, "E.");
    let outcome_b = preflight_ok(&app, plenty);
    assert_eq!(
        skip_code_of(verdict_of(&outcome_b, &deleted)),
        "E_MARKERS_INVALID"
    );
}

#[test]
fn scheme_b_builds_the_splice_form_and_compares_by_alg033() {
    // The scheme-B happy path: one marker pair in order passes the
    // gates, the markerless form is the ALG-031 splice minus the
    // marker line (the outside preserved byte for byte, the managed
    // converted to the file's style), and a file that already holds
    // the build is `unchanged` while a different one stays `ready`.
    let (_temp, app, repos) = phase1_app();
    let (same, same_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::B,
        BaseName::AgentsMd,
    );
    let (moved, moved_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::B,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &same, "Be brief.");
    seed_assigned_instruction(&app, &moved, "Be brief.");

    // Same build: unchanged (whatever the old marker said).
    fs::write(
        same_repo.join("AGENTS.md"),
        old_sync_b_file("Be brief.", "Intro.\n", "Tail.\n"),
    )
    .expect("writing the fixture must work");
    // Different managed block inside a CRLF-dominant file: ready.
    let crlf_file = "Intro.\r\n<!-- EUNOMIASYNC:BEGIN -->\r\nOld body.\r\n<!-- EUNOMIASYNC:END -->\r\nTail.\r\n";
    fs::write(moved_repo.join("AGENTS.md"), crlf_file).expect("writing the fixture must work");

    let outcome = preflight_ok(&app, plenty);
    assert_eq!(
        verdict_of(&outcome, &same),
        &PreflightVerdict::Unchanged,
        "the file holding the build is unchanged"
    );
    let PreflightVerdict::Ready(ready) = verdict_of(&outcome, &moved) else {
        panic!("the changed block must stay ready");
    };
    // The expected markerless form spelled from the spec's rules, not
    // from the code: the outside preserved byte for byte, the managed
    // text in the file's dominant style (CRLF — strictly more
    // frequent), ALG-030's blank line before the absent marker.
    assert_eq!(
        ready.markerless,
        "Intro.\r\n<!-- EUNOMIASYNC:BEGIN -->\r\nBe brief.\r\n\r\n<!-- EUNOMIASYNC:END -->\r\nTail.\r\n"
    );
    // The file survives byte for byte (preflight changes nothing).
    assert_eq!(
        fs::read(moved_repo.join("AGENTS.md")).expect("reading the target back must work"),
        crlf_file.as_bytes()
    );
}

// ---- step 5: ALG-033 and the setting's gate --------------------------

#[test]
fn identical_content_is_unchanged_by_alg033_and_the_setting_gates_it() {
    // D-24: the comparison drops the marker line from the existing
    // file and matches the build's markerless form — the timestamp
    // never decides. `sync.skip_identical` off turns the same file
    // into a write (ready), and the row records `unchanged` when on.
    let (_temp, app, repos) = phase1_app();
    let (same, same_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (different, different_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &same, "Be brief.");
    seed_assigned_instruction(&app, &different, "Be brief.");
    fs::write(same_repo.join("AGENTS.md"), old_sync_file("Be brief."))
        .expect("writing the fixture must work");
    // One word of difference in the body: ready.
    fs::write(
        different_repo.join("AGENTS.md"),
        old_sync_file("Be very brief."),
    )
    .expect("writing the fixture must work");

    let outcome = preflight_ok(&app, plenty);
    assert_eq!(verdict_of(&outcome, &same), &PreflightVerdict::Unchanged);
    assert!(matches!(
        verdict_of(&outcome, &different),
        PreflightVerdict::Ready(_)
    ));
    // The row transition the verdict caused.
    let rows = run_rows_of(&app, &outcome.run_id);
    assert_eq!(
        rows[0].status,
        eunomia_store::sync_runs::SyncRunProjectStatus::Unchanged
    );
    assert_eq!(rows[0].error_code, None);
    assert_eq!(
        rows[1].status,
        eunomia_store::sync_runs::SyncRunProjectStatus::NotStarted
    );

    // The setting's gate: with skip_identical off, the identical file
    // is a write, not a verdict.
    set_skip_identical(&app, false);
    let outcome_b = preflight_ok(&app, plenty);
    assert!(matches!(
        verdict_of(&outcome_b, &same),
        PreflightVerdict::Ready(_)
    ));
}

// ---- step 6: the D-25 confirmation list ------------------------------

#[test]
fn an_empty_build_over_a_non_empty_file_needs_confirmation() {
    // D-25: an empty build (the project has no assignments) over a
    // non-empty file joins the confirmation list; a missing file and
    // a non-empty build never do. The idempotence edge: with
    // skip_identical on, a file holding ONLY the old marker is
    // `unchanged` (the empty build's own output), never a
    // confirmation — and turning the setting off moves exactly that
    // file onto the list (the raw size reading, recorded in the
    // task's D-entry).
    let (_temp, app, repos) = phase1_app();
    let (wipe, wipe_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (fresh, _fresh_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (marker_only, marker_repo) = seed_repo_project(
        &app,
        &repos,
        3,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (body, body_repo) = seed_repo_project(
        &app,
        &repos,
        4,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &body, "Be brief.");
    // No assignments for wipe/fresh/marker_only: their builds are
    // empty. The `body` project pins that a NON-empty build never
    // joins the list.
    fs::write(wipe_repo.join("AGENTS.md"), "precious hand notes")
        .expect("writing the fixture must work");
    fs::write(marker_repo.join("AGENTS.md"), old_sync_file(""))
        .expect("writing the fixture must work");
    fs::write(body_repo.join("AGENTS.md"), "old").expect("writing the fixture must work");
    // fresh_repo's target deliberately absent: the creation case.

    let outcome = preflight_ok(&app, plenty);
    // Only the genuinely wiped file joins the list while the setting
    // is on (the marker-only file is `unchanged`).
    assert_eq!(outcome.needs_confirmation(), vec![wipe.as_str()]);

    let PreflightVerdict::Ready(fresh_ready) = verdict_of(&outcome, &fresh) else {
        panic!("the missing target is the creation case");
    };
    assert_eq!(fresh_ready.existing_size, None);
    assert!(!fresh_ready.needs_confirmation);

    assert_eq!(
        verdict_of(&outcome, &marker_only),
        &PreflightVerdict::Unchanged
    );

    let PreflightVerdict::Ready(body_ready) = verdict_of(&outcome, &body) else {
        panic!("the non-empty build must be ready");
    };
    assert!(!body_ready.needs_confirmation);

    // skip_identical off: the marker-only file becomes a write over a
    // non-empty (raw size) file — exactly the recorded edge, and the
    // list is in sort_order.
    set_skip_identical(&app, false);
    let outcome_b = preflight_ok(&app, plenty);
    assert_eq!(
        outcome_b.needs_confirmation(),
        vec![wipe.as_str(), marker_only.as_str()]
    );
}

// ---- step 7: the stale-tmp sweep --------------------------------------

#[test]
fn the_sweep_deletes_only_the_stale_files_of_reached_targets() {
    // §9.4 phase 1 step 7: stale (older than an hour) app tmp files
    // of the RESOLVED name are deleted; fresh ones and foreign names
    // stay; a skipped project is never swept (its steps stop at the
    // refusal). Staleness is created the honest way — the fixture's
    // own mtime is backdated two hours through std's portable
    // `File::set_modified` — so the phase runs on the real clock.
    let (_temp, app, repos) = phase1_app();
    let (healthy, healthy_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (locked, locked_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &healthy, "Be brief.");
    seed_assigned_instruction(&app, &locked, "Be brief.");
    // The healthy project's file holds the current build: its verdict
    // is `unchanged`, so the sweep assertion below pins the step-7
    // call on the UNCHANGED branch too (the ready branch shares the
    // same call site's product; a dropped unchanged-sweep would leave
    // the stale file behind and fail the entries assertion).
    fs::write(healthy_repo.join("AGENTS.md"), old_sync_file("Be brief."))
        .expect("writing the fixture must work");
    let locked_target = locked_repo.join("AGENTS.md");
    fs::write(&locked_target, b"bytes").expect("writing the fixture must work");
    make_read_only(&locked_target);

    // Backdates a file's modification time by `age` — the std
    // portable spelling (write handle, `set_modified`), the same fact
    // the sweep reads back.
    fn backdate(path: &Path, age: std::time::Duration) {
        let file = fs::File::options()
            .write(true)
            .open(path)
            .expect("opening the fixture for the time write must work");
        file.set_modified(
            SystemTime::now()
                .checked_sub(age)
                .expect("the clock must be past the fixture's age"),
        )
        .expect("backdating the fixture's mtime must work");
    }

    for folder in [&healthy_repo, &locked_repo] {
        let stale = folder.join(tmp_name("AGENTS.md", "00000000"));
        fs::write(&stale, b"old debris").expect("writing the fixture must work");
        backdate(&stale, std::time::Duration::from_secs(2 * 60 * 60));
        fs::write(
            folder.join(tmp_name("AGENTS.md", "11111111")),
            b"fresh debris",
        )
        .expect("writing the fixture must work");
        fs::write(
            folder.join(tmp_name("CLAUDE.md", "00000000")),
            b"foreign key",
        )
        .expect("writing the fixture must work");
        fs::write(folder.join("notes.txt"), b"foreign name")
            .expect("writing the fixture must work");
    }

    let phase0 =
        tauri::async_runtime::block_on(sync_phase0::run(app.handle())).expect("phase 0 must run");
    let outcome = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        plenty,
        SystemTime::now(),
    ))
    .expect("the sweep never fails the run");
    assert_eq!(
        verdict_of(&outcome, &healthy),
        &PreflightVerdict::Unchanged,
        "the healthy project's file holds the build — the sweep ran on the unchanged branch"
    );

    // The healthy (reached) folder: only the stale file of the right
    // key is gone; the fresh one, the foreign key, the foreign name
    // and the repository's own .git stay.
    assert_eq!(
        entries_of(&healthy_repo),
        vec![
            ".AGENTS.md.eunomia-11111111.tmp".to_owned(),
            ".CLAUDE.md.eunomia-00000000.tmp".to_owned(),
            ".git".to_owned(),
            "AGENTS.md".to_owned(),
            "notes.txt".to_owned(),
        ],
        "the sweep deletes exactly the stale file of the resolved name"
    );
    // The skipped (rights-refused) folder: untouched — steps after a
    // skip do not run for the skipped project.
    assert_eq!(
        entries_of(&locked_repo),
        vec![
            ".AGENTS.md.eunomia-00000000.tmp".to_owned(),
            ".AGENTS.md.eunomia-11111111.tmp".to_owned(),
            ".CLAUDE.md.eunomia-00000000.tmp".to_owned(),
            ".git".to_owned(),
            "AGENTS.md".to_owned(),
            "notes.txt".to_owned(),
        ],
        "a skipped project's folder is not swept"
    );
}

// ---- the creation case and scheme C -----------------------------------

#[test]
fn a_missing_ac_target_is_the_creation_case_and_scheme_c_gets_its_name() {
    // OUT-005 / SYS-009: a missing A/C target is written, not
    // skipped; scheme C's target is `<STEM>.project.md` (D-20).
    let (_temp, app, repos) = phase1_app();
    let (creation, creation_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (scheme_c, scheme_c_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::C,
        BaseName::ClaudeMd,
    );
    seed_assigned_instruction(&app, &creation, "Be brief.");
    seed_assigned_instruction(&app, &scheme_c, "Be brief.");

    let outcome = preflight_ok(&app, plenty);
    let PreflightVerdict::Ready(a) = verdict_of(&outcome, &creation) else {
        panic!("the missing A target must be ready");
    };
    assert_eq!(
        a.resolved_path,
        eunomia_fs::paths::canonicalize(&creation_repo)
            .expect("the repo must canonicalize")
            .join("AGENTS.md")
    );
    assert_eq!(a.existing_size, None);
    let PreflightVerdict::Ready(c) = verdict_of(&outcome, &scheme_c) else {
        panic!("the missing C target must be ready");
    };
    assert_eq!(
        c.resolved_path,
        eunomia_fs::paths::canonicalize(&scheme_c_repo)
            .expect("the repo must canonicalize")
            .join("CLAUDE.project.md")
    );
    assert_eq!(c.resolved_name, "CLAUDE.project.md");
    // Neither file came into existence (preflight changes nothing).
    assert!(!creation_repo.join("AGENTS.md").exists());
    assert!(!scheme_c_repo.join("CLAUDE.project.md").exists());
}

// ---- the order and the honest refusals --------------------------------

#[test]
fn projects_are_preflighted_in_sort_order() {
    // §9.4 phase 1: "for each active project, in order" — the
    // outcome's order is the projects' `sort_order`, here reversed by
    // the reorder repository after seeding.
    let (_temp, app, repos) = phase1_app();
    let first = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    )
    .0;
    let second = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    )
    .0;
    let third = seed_repo_project(
        &app,
        &repos,
        3,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    )
    .0;
    // The expected order kept aside: the reorder's closure owns its
    // inputs (the 'static executor closure).
    let expected_order = [third.clone(), first.clone(), second.clone()];
    tauri::async_runtime::block_on(database_of(&app).run(move |tx| {
        eunomia_store::projects::reorder(tx, &[&third, &first, &second])
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("reordering the projects must work");

    let outcome = preflight_ok(&app, plenty);
    assert_eq!(
        outcome
            .projects
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        expected_order
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        "the verdicts come back in sort_order"
    );
}

#[test]
fn phase_1_without_a_database_refuses_honestly() {
    // Recovery mode: with no database managed, the phase answers the
    // recorded NoDatabase fallback (the managed_db shape) at its own
    // first step — before any probe, any verdict, any write.
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(SyncRunning::new());

    // A minimal phase-0 outcome (the flag lease and a run row) — the
    // database step must refuse before the projects are ever touched.
    let guard = begin_sync(app.handle()).expect("the flag is free");
    let phase0 = Phase0Outcome {
        guard,
        run: SyncRun {
            id: "0193f1a0-0000-7000-8000-000000000000".to_owned(),
            started_at: eunomia_store::values::now_utc(),
            finished_at: None,
            status: eunomia_store::sync_runs::SyncRunStatus::Running,
            projects_total: 0,
            projects_written: 0,
            projects_unchanged: 0,
            projects_skipped: 0,
            projects_failed: 0,
            error_code: None,
            details: None,
        },
        projects: Vec::new(),
    };

    let error = tauri::async_runtime::block_on(sync_phase1::run(
        app.handle(),
        &phase0,
        plenty,
        SystemTime::now(),
    ))
    .expect_err("phase 1 without a database must refuse");
    assert_eq!(error.code, ErrorCode::DbLocked);
    let details = error.details.expect("the refusal carries its reason");
    assert_eq!(details.os_kind.as_deref(), Some("NoDatabase"));
}

// ---- the non-regular entry gate ---------------------------------------

#[cfg(unix)]
#[test]
fn a_non_regular_target_entry_skips_instead_of_blocking_the_phase() {
    // The regular-file gate: `resolve_target_path` accepts any
    // non-directory, non-link entry by design, but a FIFO named like
    // a target would block the sequential preflight's read until a
    // writer appears (verified: an open-for-read on an empty FIFO
    // hangs) — hanging the whole run and its flag for the process's
    // life. The gate refuses the entry kind up front with the
    // unavailability row's class-only message, and the run continues.
    let (_temp, app, repos) = phase1_app();
    let (fifo, fifo_repo) = seed_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    let (healthy, _healthy_repo) = seed_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        BaseName::AgentsMd,
    );
    seed_assigned_instruction(&app, &fifo, "Be brief.");
    seed_assigned_instruction(&app, &healthy, "Be brief.");
    // mkfifo(1) builds the fixture std cannot create — the fixture is
    // the environment's business, not the production code's.
    let status = std::process::Command::new("mkfifo")
        .arg(fifo_repo.join("AGENTS.md"))
        .status()
        .expect("running mkfifo must work");
    assert!(status.success(), "mkfifo must create the fixture");

    // The watchdog (the round-2 review's P3-3): the whole premise is
    // that an ungated read on the FIFO BLOCKS, so a gate regression
    // would hang this test to the harness timeout. The preflight runs
    // on a worker thread and the assertions wait behind a bounded
    // receive — a regression fails THIS test, loudly, instead.
    let worker_handle = app.handle().clone();
    let (settled, watchdog) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let outcome = {
            let phase0 = tauri::async_runtime::block_on(sync_phase0::run(&worker_handle))
                .expect("phase 0 must run");
            let outcome = tauri::async_runtime::block_on(sync_phase1::run(
                &worker_handle,
                &phase0,
                plenty,
                SystemTime::now(),
            ))
            .expect("the gated preflight must settle");
            drop(phase0);
            outcome
        };
        settled.send(outcome).expect("the channel must be alive");
    });
    let outcome = match watchdog.recv_timeout(std::time::Duration::from_secs(60)) {
        Ok(outcome) => outcome,
        Err(_) => panic!(
            "the regular-file gate regressed: the preflight did not settle \
             within 60 s — the FIFO read is blocking the sequential pass"
        ),
    };

    let PreflightVerdict::Skipped { code, message, .. } = verdict_of(&outcome, &fifo) else {
        panic!(
            "the FIFO target must skip, got {:?}",
            verdict_of(&outcome, &fifo)
        );
    };
    assert_eq!(*code, ErrorCode::ProjectUnavailable);
    assert_eq!(message, "the target entry is not a regular file");
    assert!(matches!(
        verdict_of(&outcome, &healthy),
        PreflightVerdict::Ready(_)
    ));
    // The worker finished its send; joining only reaps it.
    worker.join().expect("the preflight worker must not panic");
}
