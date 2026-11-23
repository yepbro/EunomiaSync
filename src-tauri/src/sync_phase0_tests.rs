//! Behavior tests of the sync pipeline's phase 0 (T-112, §9.4 phase
//! 0, SYS-004/SYS-005, ALG-001).
//!
//! The suite drives the phase the ways production reaches it:
//!
//! * the **whole entry** `sync_phase0::run` — flag taken, run
//!   recorded, content built and returned, the flag held until the
//!   outcome drops and the run row left `running` (phase 0 writes no
//!   final status — phases 1–4 own the verdicts);
//! * the **steps separately** (`read_snapshot` → `build_projects` →
//!   `record_run`) — the deterministic interleaving the card's
//!   criterion needs: a library edit committed **between the
//!   snapshot read and the record write** (by the app's own `Db`
//!   handle, the production shape of "an edit during a sync", and by
//!   a second `open_db` connection, the out-of-band writer) must not
//!   enter the run's built content, and the project must read dirty
//!   afterwards — the DB-021 recalculation over the post-edit
//!   database is exactly the "stays dirty" half of SYS-005 (D-121's
//!   service, awaited directly);
//! * the **flag seam**: a second start while one holds the flag is
//!   the typed `E_SYNC_RUNNING` envelope (the T-105 seam consumed
//!   here), and every error path — no database among them — releases
//!   the flag (RAII, D-123).
//!
//! The mock app is the `dirty_app` shape (mock runtime, real database
//! in a temp root, `SyncRunning`/`DirtyState` managed, the
//! `state://dirty` event mounted); the harness owns its copy because
//! the ipc suite's helpers are private to it.

use std::path::{Path, PathBuf};

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_core::hash::build_hash;
use eunomia_core::project::{ExtraFile, ExtraFilePosition, Scheme};
use eunomia_store::projects::NewProject;
use eunomia_store::storage::open_db;

use crate::commands::begin_sync;
use crate::data_dirs;
use crate::db::Db;
use crate::db::sync_runs_app_error_for_tests;
use crate::dirty_recalc::{DirtyEvent, DirtyState, recalc_and_emit};
use crate::errors::{AppError, ErrorCode};
use crate::sync_phase0::{self, PreparedProject};
use crate::sync_running::SyncRunning;

// ---- the harness ------------------------------------------------------

/// A mock app in the healthy state the phase serves: a fresh database
/// in a temp root, the [`SyncRunning`] flag and the [`DirtyState`]
/// cache managed, and the `state://dirty` event mounted — the
/// registration `lib.rs`'s setup performs for `tauri::Wry`, pinned
/// here for the mock runtime the emit/listen round trip needs (the
/// `dirty_recalc_tests.rs` harness, plus the flag the phase takes).
/// The database file path rides along for the second-connection
/// writer of the isolation criterion.
fn phase0_app() -> (
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
    let (state, _triggers) = DirtyState::new();
    app.manage(state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![DirtyEvent])
        .mount_events(&app);
    let db_path = data_dirs::db_file_path(temp.path());
    (temp, app, db_path)
}

/// The managed database handle.
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> Db {
    app.state::<Db>().inner().clone()
}

/// The run repository's per-project rows through the app's database —
/// the assertion-side read (the store suite covers its behavior; the
/// phase's tests read claims, not plumbing).
fn run_rows_of(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> Vec<eunomia_store::sync_runs::SyncRunProject> {
    let database = database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::sync_runs::list_projects(tx, &run_id).map_err(sync_runs_app_error_for_tests)
    }))
    .expect("reading the run's rows must work")
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

// ---- seeding ----------------------------------------------------------

/// Maps a library seeding failure onto the wire shape the executor
/// needs — test-local, the D-093 fallback family (the
/// `dirty_recalc_tests.rs` shape).
fn library_seed_error(error: eunomia_store::library::LibraryError) -> AppError {
    match error {
        eunomia_store::library::LibraryError::Sql(sql) => crate::db::sqlite_error_to_app_error(sql),
        other => {
            let mut app_error = AppError::new(ErrorCode::DbLocked);
            app_error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("LibrarySeed".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// Seeds one project with the given scheme (both extra files off),
/// returning its row id. Owned inputs — the executor's closure is
/// `'static`.
fn seed_project(
    app: &tauri::App<tauri::test::MockRuntime>,
    n: usize,
    scheme: eunomia_store::settings::Scheme,
) -> String {
    seed_project_with(
        app,
        n,
        scheme,
        eunomia_store::settings::BaseName::AgentsMd,
        eunomia_store::settings::ExtraFile {
            enabled: false,
            position: eunomia_store::settings::Position::End,
        },
        eunomia_store::settings::ExtraFile {
            enabled: false,
            position: eunomia_store::settings::Position::End,
        },
    )
}

/// Seeds one project with the full write-settings surface — the
/// settings-coverage fixture (scheme C, `CLAUDE.md`, enabled extras
/// at different positions).
fn seed_project_with(
    app: &tauri::App<tauri::test::MockRuntime>,
    n: usize,
    scheme: eunomia_store::settings::Scheme,
    base_name: eunomia_store::settings::BaseName,
    extra_local: eunomia_store::settings::ExtraFile,
    extra_project: eunomia_store::settings::ExtraFile,
) -> String {
    let path = format!("/repos/repo-{n}");
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path: &path,
                path_key: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
                name: Box::leak(format!("Project {n}").into_boxed_str()),
                label: Box::leak(format!("P{n}").into_boxed_str()),
                color: "blue",
                description: "",
                added_via: eunomia_store::projects::AddedVia::Manual,
                scheme,
                base_name,
                extra_local,
                extra_project,
            },
        )
        .map(|project| project.id)
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work")
}

/// Seeds one active instruction with its main variant, returning
/// `(instruction_id, variant_id)` (the `dirty_recalc_tests.rs` shape).
fn seed_instruction(
    app: &tauri::App<tauri::test::MockRuntime>,
    title: &'static str,
    content: &'static str,
) -> (String, String) {
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
        Ok((item.id, variant.id))
    }))
    .expect("seeding an instruction with its main variant must work")
}

/// Seeds a second (non-main) variant on an existing instruction —
/// the crossing-wires fixture's second rail.
fn seed_extra_variant(
    app: &tauri::App<tauri::test::MockRuntime>,
    instruction_id: &str,
    content: &'static str,
) -> String {
    let instruction_id = instruction_id.to_owned();
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &instruction_id,
                name: "Compact",
                content,
                is_main: false,
            },
        )
        .map(|variant| variant.id)
        .map_err(library_seed_error)
    }))
    .expect("seeding the extra variant must work")
}

/// Assigns `variant_id` of `instruction_id` to the project.
fn seed_assignment(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: String,
    instruction_id: String,
    variant_id: String,
) {
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &instruction_id,
                variant_id: &variant_id,
            },
        )
        .map(|_| ())
        .map_err(library_seed_error)
    }))
    .expect("seeding an assignment through the repository must work")
}

/// Rewrites a variant's text through the app's own `Db` handle — the
/// production shape of "an edit during a sync" (the D-037 executor
/// serializes it between the run's transactions).
fn edit_variant(app: &tauri::App<tauri::test::MockRuntime>, variant_id: String, content: String) {
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        eunomia_store::library::update_variant(
            tx,
            &variant_id,
            &eunomia_store::library::VariantUpdate {
                name: None,
                content: Some(&content),
            },
        )
        .map(|_| ())
        .map_err(library_seed_error)
    }))
    .expect("editing the variant must work")
}

/// Rewrites a variant's text through a **second connection** opened
/// directly on the database file — the out-of-band writer of the
/// T-100 isolation pattern: phase 0's guarantee must hold against any
/// connection, not just the app's own.
fn edit_variant_second_connection(db_path: &Path, variant_id: &str, content: &str) {
    let conn = open_db(db_path).expect("the second connection must open");
    conn.execute(
        "UPDATE instruction_variants SET content = ?1 WHERE id = ?2",
        rusqlite::params![content, variant_id],
    )
    .expect("the out-of-band edit must commit");
}

/// The DB-024 simulation (the `dirty_recalc_tests.rs` shape): store a
/// project's synced hash, as the pipeline's phase 3/4 writer will.
fn simulate_sync(app: &tauri::App<tauri::test::MockRuntime>, project_id: String, hash: String) {
    tauri::async_runtime::block_on(database_of(app).run(move |tx| {
        tx.execute(
            "UPDATE projects SET last_synced_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, project_id],
        )
        .map(|_| ())
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("storing the synced hash must work");
}

/// The no-extras settings of the seeded projects, for independently
/// spelling the DB-020 hash oracle.
fn off() -> ExtraFile {
    ExtraFile {
        enabled: false,
        position: ExtraFilePosition::End,
    }
}

// ---- the card's criterion: the mid-run edit --------------------------

/// The criterion scenario, parameterized by the writer: a synced-clean
/// project (its stored hash fingerprints the `Original.` build of its
/// ASSIGNED instruction), a phase-0 start whose snapshot is read
/// **before** the edit, the edit of that assigned variant landing
/// between the read and the record, and then both halves of SYS-005's
/// promise:
///
/// * the run's built content is still `Original.` — the edit did not
///   enter the current run (the snapshot is one read transaction, and
///   the build consumes the snapshot value alone);
/// * the project reads **dirty** afterwards — the DB-021 recalc
///   over the post-edit database (the live service logic, D-121)
///   sees the build move under the stored hash.
///
/// `out_of_band` picks the writer: `false` — the app's own `Db`
/// handle (the production shape of "an edit during a sync": the
/// D-037 executor serializes it between the run's transactions);
/// `true` — a second `open_db` connection on the database file (the
/// T-100 isolation pattern: the guarantee must hold against any
/// writer).
fn mid_run_edit_stays_out_of_the_run(
    app: &tauri::App<tauri::test::MockRuntime>,
    db_path: &Path,
    out_of_band: bool,
) {
    let project = seed_project(app, 1, eunomia_store::settings::Scheme::A);
    let (instruction, variant) = seed_instruction(app, "Be brief", "Original.");
    seed_assignment(app, project.clone(), instruction, variant.clone());

    // The project is synced-clean: its stored hash fingerprints the
    // build as it stands NOW (spelled through the public core seam,
    // never through the code under test).
    let target = Path::new("/repos/repo-1").join("AGENTS.md");
    let synced = build_hash(Scheme::A, &target, off(), off(), "Original.");
    simulate_sync(app, project.clone(), synced.clone());
    // The pre-run world is clean — the service says so.
    let before = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the pre-run recalculation must work");
    assert!(
        !before
            .iter()
            .any(|change| change.project_id == project && change.dirty),
        "the project starts synced-clean, got {before:?}"
    );

    // The run's steps, interleaved with the edit: the snapshot is
    // read first, the edit commits while the run is in flight, the
    // content is built and the run recorded afterwards. The start
    // time is captured at the flag take — `run`'s own order, spelled
    // here for the manual drive.
    let guard = begin_sync(app.handle()).expect("the flag is free");
    let started_at = eunomia_store::values::now_utc();
    let snapshot = tauri::async_runtime::block_on(sync_phase0::read_snapshot(app.handle()))
        .expect("the snapshot read must work");
    if out_of_band {
        edit_variant_second_connection(db_path, &variant, "Changed.");
    } else {
        edit_variant(app, variant, "Changed.".to_owned());
    }
    let prepared = sync_phase0::build_projects(&snapshot);
    let inputs = prepared
        .iter()
        .map(|built| {
            (
                built.project_id.clone(),
                built
                    .target_path
                    .to_str()
                    .expect("a UTF-8 target path")
                    .to_owned(),
                built.build_hash.clone(),
            )
        })
        .collect();
    let run =
        tauri::async_runtime::block_on(sync_phase0::record_run(app.handle(), started_at, inputs))
            .expect("recording the run must work");
    drop(guard);

    // Half one: the edit did not enter the run. The built content,
    // the hash, and the recorded row all fingerprint `Original.`.
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].managed, "Original.");
    assert_eq!(prepared[0].build_hash, synced);
    let rows = run_rows_of(app, &run.id);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].build_hash.as_deref(), Some(synced.as_str()));
    assert_eq!(
        rows[0].target_path.as_deref(),
        Some(target.to_str().expect("a UTF-8 target"))
    );

    // Half two: the project is dirty after the run — the recalc
    // (the live DB-021 service logic) sees the post-edit build move
    // under the stored hash.
    let changes = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the post-run recalculation must work");
    assert!(
        changes
            .iter()
            .any(|change| change.project_id == project && change.dirty),
        "the mid-run edit must leave the project dirty, got {changes:?}"
    );
}

#[test]
fn a_library_edit_during_the_run_stays_out_and_leaves_the_project_dirty() {
    // The writer is the app's own `Db` handle — the production shape
    // of an edit that slips in mid-sync (the D-037 executor
    // serializes it between the run's transactions).
    let (_temp, app, db_path) = phase0_app();
    mid_run_edit_stays_out_of_the_run(&app, &db_path, false);
}

#[test]
fn an_out_of_band_edit_during_the_run_stays_out_too() {
    // The writer is a second connection on the database file — the
    // T-100 isolation pattern: the guarantee must hold against ANY
    // writer, not just the app's serialized one.
    let (_temp, app, db_path) = phase0_app();
    mid_run_edit_stays_out_of_the_run(&app, &db_path, true);
}

// ---- the whole entry ---------------------------------------------------

#[test]
fn run_takes_the_flag_records_the_run_and_builds_every_project() {
    let (_temp, app, _db_path) = phase0_app();
    let first = seed_project(&app, 1, eunomia_store::settings::Scheme::A);
    let second = seed_project(&app, 2, eunomia_store::settings::Scheme::B);
    // One shared instruction, two variants, one per project — the
    // crossing-wires pin: a scoping bug that leaks one project's
    // assignment into the other's mini-snapshot flips both managed
    // texts (the D-121 suite pins the verdict side; this pins the
    // content side).
    let (instruction, main_variant) = seed_instruction(&app, "Shared", "Main body.");
    let compact_variant = seed_extra_variant(&app, &instruction, "Compact body.");
    seed_assignment(&app, first.clone(), instruction.clone(), main_variant);
    seed_assignment(&app, second.clone(), instruction, compact_variant);

    let outcome = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over the seeded database");

    // The flag is held by the outcome, and freed by its drop.
    assert!(
        app.state::<SyncRunning>().is_running(),
        "the run holds the flag while the outcome lives"
    );

    // The content: one entry per active project, in sort_order, the
    // managed text and hash spelled independently through the public
    // core seams (extras off → the body IS the managed, D-065) — each
    // project its OWN variant's body.
    assert_eq!(
        outcome
            .projects
            .iter()
            .map(|built| built.project_id.as_str())
            .collect::<Vec<_>>(),
        vec![first.as_str(), second.as_str()]
    );
    let first_target = Path::new("/repos/repo-1").join("AGENTS.md");
    let second_target = Path::new("/repos/repo-2").join("AGENTS.md");
    let first_hash = build_hash(Scheme::A, &first_target, off(), off(), "Main body.");
    let second_hash = build_hash(Scheme::B, &second_target, off(), off(), "Compact body.");
    assert_eq!(
        outcome.projects,
        vec![
            PreparedProject {
                project_id: first.clone(),
                managed: "Main body.".to_owned(),
                target_path: first_target.clone(),
                build_hash: first_hash.clone(),
            },
            PreparedProject {
                project_id: second.clone(),
                managed: "Compact body.".to_owned(),
                target_path: second_target.clone(),
                build_hash: second_hash.clone(),
            },
        ]
    );

    // The database: the `running` run and its `not_started` rows,
    // read back through the store's own reads.
    let run_id = outcome.run.id.clone();
    let run = run_row_of(&app, &run_id);
    assert_eq!(run.status, eunomia_store::sync_runs::SyncRunStatus::Running);
    assert_eq!(run.projects_total, 2);
    let rows = run_rows_of(&app, &run_id);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].project_id, first);
    assert_eq!(
        rows[0].status,
        eunomia_store::sync_runs::SyncRunProjectStatus::NotStarted
    );
    assert_eq!(
        rows[0].target_path.as_deref(),
        Some(first_target.to_str().expect("a UTF-8 target"))
    );
    assert_eq!(rows[0].build_hash.as_deref(), Some(first_hash.as_str()));
    assert_eq!(rows[1].project_id, second);
    assert_eq!(rows[1].build_hash.as_deref(), Some(second_hash.as_str()));

    // The exit: dropping the outcome frees the flag, and the run row
    // STAYS `running` — phase 0 writes no final status (§9.4's
    // verdicts are phase 4's; a dead pipeline's row is the
    // crash-recovery rule's `interrupted`, not phase 0's business).
    drop(outcome);
    assert!(
        !app.state::<SyncRunning>().is_running(),
        "dropping the outcome resets the flag (§9.4 phase 4 step 5)"
    );
    let status = run_row_of(&app, &run_id).status;
    assert_eq!(status, eunomia_store::sync_runs::SyncRunStatus::Running);
}

#[test]
fn scheme_c_claude_md_and_enabled_extras_build_over_their_own_shape() {
    // The settings-coverage pin (the round-1 review's P2-2): every
    // other fixture runs A or B with `AGENTS.md` and both extras off,
    // so a swapped `extra_local`/`extra_project` argument, a broken
    // scheme-C name, or a dropped extras-in-hash rule would pass the
    // whole suite. Here: p1 is scheme A with BOTH extras on at Start
    // (the §7.5 tie — `.project.md` before `.local.md` — and the
    // ALG-030 splice, expected managed spelled from the D-21 sentence
    // literals), p2 is scheme C with `CLAUDE.md` and the extras at
    // DIFFERENT positions (managed ignores them — "(A only)" — but
    // the DB-020 hash includes them under every scheme, so the
    // positions must reach the hash in the right argument slots).
    let (_temp, app, _db_path) = phase0_app();
    let start = |enabled: bool| eunomia_store::settings::ExtraFile {
        enabled,
        position: eunomia_store::settings::Position::Start,
    };
    let end = |enabled: bool| eunomia_store::settings::ExtraFile {
        enabled,
        position: eunomia_store::settings::Position::End,
    };
    let first = seed_project_with(
        &app,
        1,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
        start(true), // local at Start
        start(true), // project at Start — §7.5: project line first
    );
    let second = seed_project_with(
        &app,
        2,
        eunomia_store::settings::Scheme::C,
        eunomia_store::settings::BaseName::ClaudeMd,
        start(true), // local at Start
        end(true),   // project at End — the two positions differ
    );
    let (instruction, variant) = seed_instruction(&app, "Be brief", "Be brief.");
    seed_assignment(&app, first.clone(), instruction.clone(), variant.clone());
    seed_assignment(&app, second.clone(), instruction, variant);

    let outcome = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over the settings matrix");

    const AGENTS_PROJECT_LINE: &str = "If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.";
    const AGENTS_LOCAL_LINE: &str = "If AGENTS.local.md exists in the repository root, read it before starting work and follow its instructions.";
    // Scheme A: the two Start lines are ONE part (joined by a single
    // break), the body a second, one empty line between.
    let first_managed = format!("{AGENTS_PROJECT_LINE}\n{AGENTS_LOCAL_LINE}\n\nBe brief.");
    let first_target = Path::new("/repos/repo-1").join("AGENTS.md");
    let first_hash = build_hash(
        Scheme::A,
        &first_target,
        ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        },
        ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        },
        &first_managed,
    );
    // Scheme C: the body alone, but the extras ride the hash.
    let second_target = Path::new("/repos/repo-2").join("CLAUDE.project.md");
    let second_hash = build_hash(
        Scheme::C,
        &second_target,
        ExtraFile {
            enabled: true,
            position: ExtraFilePosition::Start,
        },
        ExtraFile {
            enabled: true,
            position: ExtraFilePosition::End,
        },
        "Be brief.",
    );
    assert_eq!(
        outcome.projects,
        vec![
            PreparedProject {
                project_id: first,
                managed: first_managed,
                target_path: first_target,
                build_hash: first_hash,
            },
            PreparedProject {
                project_id: second,
                managed: "Be brief.".to_owned(),
                target_path: second_target,
                build_hash: second_hash,
            },
        ]
    );
}

#[test]
fn the_chunk_boundary_never_crosses_projects_wires() {
    // The batching's own proof, at the content layer: 26 projects —
    // one past the shared pass's chunk size
    // (`eunomia_core::content_build::MINI_SNAPSHOT_PROJECTS`) —
    // straddling the chunk boundary, odd projects on the `Main`
    // variant, even on `Compact`. Every project's built text must
    // follow ITS OWN assignment across the boundary; a scoping bug
    // that leaks another chunk's assignment into a mini-snapshot
    // either builds the wrong text or fails the mini's validation
    // outright.
    let (_temp, app, _db_path) = phase0_app();
    let (instruction, main_variant) = seed_instruction(&app, "Shared", "Main body.");
    let compact_variant = seed_extra_variant(&app, &instruction, "Compact body.");
    for n in 1..=26 {
        let project = seed_project(&app, n, eunomia_store::settings::Scheme::A);
        let variant = if n % 2 == 1 {
            main_variant.clone()
        } else {
            compact_variant.clone()
        };
        seed_assignment(&app, project.clone(), instruction.clone(), variant);
    }

    let outcome = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over the straddling database");
    assert_eq!(outcome.projects.len(), 26);
    for (index, built) in outcome.projects.iter().enumerate() {
        let n = index + 1;
        let expected = if n % 2 == 1 {
            "Main body."
        } else {
            "Compact body."
        };
        assert_eq!(
            built.managed, expected,
            "project {n} must build its own variant's body"
        );
    }
}

#[test]
fn run_over_an_empty_project_list_records_the_empty_run() {
    // §9.4 phase 4 step 3's empty case starts here: a run over zero
    // active projects is a recorded run with no rows — the
    // "success with a message" verdict is phase 4's, not phase 0's.
    let (_temp, app, _db_path) = phase0_app();
    let outcome = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("phase 0 must run over an empty database");
    assert!(outcome.projects.is_empty());
    assert_eq!(outcome.run.projects_total, 0);
    assert_eq!(run_rows_of(&app, &outcome.run.id), Vec::new());
    drop(outcome);
    assert!(!app.state::<SyncRunning>().is_running());
}

// ---- the flag seam -----------------------------------------------------

#[test]
fn a_second_start_while_one_runs_is_the_typed_refusal() {
    let (_temp, app, _db_path) = phase0_app();
    let outcome = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("the first start must take the flag");

    let refusal = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect_err("the second start must be refused while one runs");
    assert_eq!(refusal.code, ErrorCode::SyncRunning);
    assert!(
        refusal.details.is_none(),
        "the refusal envelope carries no details (D-123)"
    );
    // The refused start left nothing behind: exactly one run row
    // exists (the first run's — a duplicate-record mutation is
    // invisible without the count), and the first run's flag is
    // still up.
    let database = database_of(&app);
    let runs: i64 = tauri::async_runtime::block_on(database.run(|tx| {
        tx.query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("counting runs must work");
    assert_eq!(runs, 1, "the refused start must record no run of its own");
    assert!(app.state::<SyncRunning>().is_running());

    drop(outcome);
    let after = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect("the freed flag is takeable by the next run");
    drop(after);
    assert!(!app.state::<SyncRunning>().is_running());
}

#[test]
fn a_failed_start_releases_the_flag() {
    // Recovery mode's honest answer, through the flag first: with no
    // database managed, the read step refuses `NoDatabase` — and the
    // guard's drop releases the flag on the way out (RAII, D-123),
    // so a syncless app cannot wedge the flag behind a refusal.
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(SyncRunning::new());

    let error = tauri::async_runtime::block_on(sync_phase0::run(app.handle()))
        .expect_err("phase 0 without a database must refuse");
    assert_eq!(error.code, ErrorCode::DbLocked);
    let details = error.details.expect("the refusal carries its reason");
    assert_eq!(details.os_kind.as_deref(), Some("NoDatabase"));
    assert!(
        !app.state::<SyncRunning>().is_running(),
        "the failed start must release the flag"
    );
}
