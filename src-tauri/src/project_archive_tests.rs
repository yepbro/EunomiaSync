//! Mock-IPC behavior tests for the `project_archive` command (T-108,
//! PRJ-013…018, §6.4, DB-014, D-10/D-194, SYS-005).
//!
//! The command runs over a real migrated `Db` through the mock-IPC
//! invoke path; assertions read the store (row, assignments,
//! exclusions, journal), the build snapshot (the sync set) and the
//! real project folder (byte-for-byte) back. The card's DoDs lead:
//! files on disk untouched, the project out of the sync set and not
//! dirty, assignments kept (DB-014), the `project_exclusions` row
//! (PRJ-017), the `E_SYNC_RUNNING` refusal (SYS-005), and the honest
//! no-database answer.

use eunomia_store::action_log::{
    Action, EntityType, Level, LogRecord, Outcome, Visibility, recent,
};
use eunomia_store::build_snapshot;
use eunomia_store::library::{
    NewAssignment, NewLibraryItem, NewVariant, assign, create_item, create_variant,
    list_assignments_for_project,
};
use eunomia_store::projects::{
    AddedVia, ExclusionReason, NewProject, ProjectState, create_exclusion,
    find_exclusion_by_path_key, list_exclusions,
};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use tauri::Manager;

/// The healthy archive world: a fresh database, the SYS-005 gate's
/// flag state managed (recovery mode manages it too — the no-database
/// test below does the same), and the command registered.
fn project_archive_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_archive::project_archive
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    (temp, app)
}

fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("mock main webview builds")
}

fn invoke_request(cmd: &str, body: serde_json::Value) -> tauri::webview::InvokeRequest {
    tauri::webview::InvokeRequest {
        cmd: cmd.into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost".parse().unwrap()
        } else {
            "tauri://localhost".parse().unwrap()
        },
        body: tauri::ipc::InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    }
}

fn invoke_project_archive(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
) -> Result<(), serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "project_archive",
            serde_json::json!({ "projectId": project_id }),
        ),
    );
    match response {
        Ok(payload) => {
            // The success payload is the unit value; deserializing it
            // pins the wire shape (a `null` body, not something else).
            payload
                .deserialize::<()>()
                .expect("the success payload must deserialize into the unit");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

fn seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::db::sqlite_error_to_app_error(match error {
        eunomia_store::library::LibraryError::Sql(source) => source,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })
}

/// One project row with a caller-chosen origin and path — the archive
/// flow branches on `added_via` (PRJ-017), so it is the parameter. The
/// text lands owned in the executor's `'static` closure (the
/// assignments suite's seeding shape).
fn seed_project(
    database: &crate::db::Db,
    added_via: AddedVia,
    path: &str,
    path_key: &str,
) -> eunomia_store::projects::Project {
    let path = path.to_owned();
    let path_key = path_key.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = NewProject {
            path: &path,
            path_key: &path_key,
            name: "Archived One",
            label: "A1",
            color: "red",
            description: "",
            added_via,
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
        };
        eunomia_store::projects::create(tx, &new)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work")
}

/// One instruction with a `Main` variant, for assignment seeding.
fn seed_instruction(
    database: &crate::db::Db,
) -> (
    eunomia_store::library::LibraryItem,
    eunomia_store::library::Variant,
) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item =
            create_item(tx, &NewLibraryItem::Instruction { title: "I1" }).map_err(seed_error)?;
        let main = create_variant(
            tx,
            &NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: "shared text",
                is_main: true,
            },
        )
        .map_err(seed_error)?;
        Ok((item, main))
    }))
    .expect("instruction seed creates")
}

fn seed_assignment(
    database: &crate::db::Db,
    project_id: &str,
    instruction_id: &str,
    variant_id: &str,
) {
    let project_id = project_id.to_owned();
    let instruction_id = instruction_id.to_owned();
    let variant_id = variant_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        assign(
            tx,
            &NewAssignment {
                project_id: &project_id,
                instruction_id: &instruction_id,
                variant_id: &variant_id,
            },
        )
        .map(|_| ())
        .map_err(seed_error)
    }))
    .expect("assignment seed creates")
}

fn stored_project(database: &crate::db::Db, id: &str) -> eunomia_store::projects::Project {
    let id = id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::get(tx, &id).map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("project read succeeds")
    .expect("the seeded row exists")
}

fn assignments_of(
    database: &crate::db::Db,
    project_id: &str,
) -> Vec<eunomia_store::library::Assignment> {
    let project_id = project_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(move |tx| list_assignments_for_project(tx, &project_id).map_err(seed_error)),
    )
    .expect("assignment list reads")
}

fn exclusions_of(database: &crate::db::Db) -> Vec<eunomia_store::projects::Exclusion> {
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(move |tx| {
            list_exclusions(tx).map_err(crate::commands::projects_store_to_app_error)
        }),
    )
    .expect("exclusion list reads")
}

fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(|tx| recent(tx, 1000).map_err(crate::commands::activity_log_to_app_error)),
    )
    .expect("journal reads")
}

fn archive_journal(database: &crate::db::Db) -> Vec<LogRecord> {
    journal_of(database)
        .into_iter()
        .filter(|entry| entry.action == Action::ProjectArchived)
        .collect()
}

#[test]
fn project_archive_archives_the_row_keeps_assignments_and_files_and_excludes_the_path() {
    // The §6.4 happy path in one world: a scan-born project with a real
    // folder, a target file with known bytes, and one assignment. The
    // archive must move the row, keep the assignment (DB-014), leave
    // the folder byte-for-byte (PRJ-015), write the PRJ-017 exclusion,
    // and journal one attributable entry (LOG-001…004/D-194).
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("repository folder creates");
    let target = repo.join("AGENTS.md");
    let original = b"managed by hand, not by this command\n";
    std::fs::write(&target, original).expect("target file writes");
    let repo_path = repo.display().to_string();
    // The identity oracle (round 2's P3-1): the key is seeded
    // DELIBERATELY different from the path — a regression that keys
    // the exclusion by `archived.path` instead of `archived.path_key`
    // (DB-030: the key is the row's own identity, not its display
    // path) must turn the exclusion assertions below red, and the
    // verbatim-path assertion must catch the mirror mistake.
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &repo_path,
        "/a-deliberately-different-key",
    );
    // D-194 point 3's kept-fact oracle needs a REAL stored sync hash:
    // a never-synced row would compare `None` to `None` — vacuous.
    // The store's own OUT-020 writer stamps the fact; the re-read
    // then carries it into the assertions below.
    {
        let id = project.id.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::projects::record_last_synced(
                tx,
                &id,
                "2026-11-24T00:00:00.000Z",
                "a-stored-build-hash",
            )
            .map_err(crate::commands::projects_store_to_app_error)
            .map(|_| ())
        }))
        .expect("seeding the sync fact must work");
    }
    let project = stored_project(&database, &project.id);
    let (instruction, main) = seed_instruction(&database);
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let assignments_before = assignments_of(&database, &project.id);
    assert_eq!(assignments_before.len(), 1, "the world starts assigned");
    let webview = main_webview(&app);

    invoke_project_archive(&webview, &project.id).expect("archiving an active project succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.state, ProjectState::Archived);
    assert!(stored.archived_at.is_some(), "archived_at is stamped");
    assert_eq!(
        stored.sort_order, project.sort_order,
        "the position is kept"
    );
    assert_eq!(stored.path_key, project.path_key, "DB-015: the key is kept");
    // DB-014 + D-194: the assignments and the sync facts stay for the
    // restore (T-109).
    assert_eq!(
        assignments_of(&database, &project.id),
        assignments_before,
        "DB-014: the archived project keeps its assignments"
    );
    assert_eq!(
        stored.last_synced_hash, project.last_synced_hash,
        "the stored hash is not churned by the archive"
    );
    // PRJ-015: the folder is untouched — byte-for-byte on the file, the
    // same entry set on the folder.
    assert_eq!(
        std::fs::read(&target).expect("target reads"),
        original,
        "PRJ-015: no file write"
    );
    let entries = std::fs::read_dir(&repo)
        .expect("repository folder reads")
        .map(|entry| {
            entry
                .expect("entry reads")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    let mut sorted_entries = entries;
    sorted_entries.sort();
    assert_eq!(
        sorted_entries,
        vec![".git".to_owned(), "AGENTS.md".to_owned()]
    );
    // PRJ-017: the scan-born path is excluded, keyed by the row's own
    // identity key, reason `removed_scanned`.
    let exclusions = exclusions_of(&database);
    assert_eq!(exclusions.len(), 1, "exactly one exclusion row is written");
    assert_eq!(exclusions[0].path_key, project.path_key);
    assert_eq!(exclusions[0].path, project.path);
    assert_eq!(exclusions[0].reason, ExclusionReason::RemovedScanned);
    // LOG-001…004/D-194: ONE attributable entry, no details (the row
    // itself holds the path).
    let journal = archive_journal(&database);
    assert_eq!(journal.len(), 1, "exactly one project.archived entry");
    assert_eq!(journal[0].level, Level::Info);
    assert_eq!(journal[0].visibility, Visibility::User);
    assert_eq!(journal[0].result, Outcome::Ok);
    assert_eq!(
        journal[0].entity,
        Some((EntityType::Project, project.id.clone()))
    );
    assert_eq!(journal[0].project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(journal[0].details, None);
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_failed_journal_insert_rolls_back_the_row_and_the_exclusion_together() {
    // D-051's in-transaction semantics, proven through the real
    // command (the `a_failed_exclusion_remove_rolls_back…` shape):
    // the row transition, the PRJ-017 exclusion and the journal entry
    // share ONE `Db::run` transaction, so a failure AFTER both writes
    // succeeded — the journal insert, staged to abort by a trigger —
    // rolls all of it back: the row stays active with no stamp, no
    // exclusion exists, and the journal claims nothing. The failure is
    // staged deterministically inside the database (RAISE(ABORT) →
    // SQLITE_CONSTRAINT → the shared base classifier's `E_VALIDATION`
    // with the SQLite facts in details).
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, &repo_path);
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_every_journal_entry BEFORE INSERT ON action_log
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");
    let webview = main_webview(&app);

    let error = invoke_project_archive(&webview, &project.id)
        .expect_err("the staged abort must fail the archive");
    assert_eq!(
        error["code"],
        serde_json::json!("E_VALIDATION"),
        "the constraint abort maps through the base classifier: {error}"
    );
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation"),
        "the SQLite facts travel in details: {error}"
    );

    // The rollback: no partial state anywhere — the row, the stamp,
    // the exclusion and the journal entry are all gone.
    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.state, ProjectState::Active, "the row rolled back");
    assert_eq!(stored.archived_at, None, "no stamp survived the abort");
    assert_eq!(
        exclusions_of(&database),
        Vec::new(),
        "no exclusion survived the abort"
    );
    assert_eq!(
        archive_journal(&database),
        Vec::new(),
        "no entry claimed an archive"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_manual_born_project_archives_without_an_exclusion_row() {
    // §6.4's table: "If the project was found by a scan" — the manual
    // arm writes no exclusion (PRJ-001's picker is its way back).
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("manual-repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/manual-repo");
    let webview = main_webview(&app);

    invoke_project_archive(&webview, &project.id).expect("archiving a manual project succeeds");

    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Archived
    );
    assert_eq!(
        exclusions_of(&database),
        Vec::new(),
        "no exclusion row for a manual-born project"
    );
    assert_eq!(archive_journal(&database).len(), 1);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_second_archive_is_a_converging_noop() {
    // D-194: the re-send converges — no second journal entry, no
    // `archived_at` restamp, and no exclusion churn even after the
    // exclusion row was removed by hand (the removal is the user's
    // own §6.4 act; a re-send does not override it).
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, &repo_path);
    let webview = main_webview(&app);
    invoke_project_archive(&webview, &project.id).expect("the first archive succeeds");
    let after_first = stored_project(&database, &project.id);
    let journal_after_first = archive_journal(&database).len();

    invoke_project_archive(&webview, &project.id).expect("the re-send converges with Ok");

    let after_second = stored_project(&database, &project.id);
    assert_eq!(
        after_second.archived_at, after_first.archived_at,
        "archived_at is not restamped"
    );
    assert_eq!(
        archive_journal(&database).len(),
        journal_after_first,
        "the re-send journals nothing"
    );

    // The hand-removal the re-send must not override: delete the
    // exclusion row through the store (the exclusions screen's own
    // path), re-send, and the row stays gone.
    let path_key = project.path_key.clone();
    let database_for_delete = database.clone();
    tauri::async_runtime::block_on(database_for_delete.run(move |tx| {
        let exclusion = find_exclusion_by_path_key(tx, &path_key)
            .map_err(crate::commands::projects_store_to_app_error)?
            .expect("the archive wrote the exclusion row");
        eunomia_store::projects::delete_exclusion(tx, &exclusion.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("the hand-removal succeeds");
    invoke_project_archive(&webview, &project.id).expect("the third send still converges");
    assert_eq!(
        exclusions_of(&database),
        Vec::new(),
        "the hand-removed exclusion stays removed — no churn"
    );
    assert_eq!(
        archive_journal(&database).len(),
        journal_after_first,
        "still one journal entry"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_existing_exclusion_row_converges_with_its_own_reason_intact() {
    // D-194's convergence arm: the store lets a project and an
    // exclusion share one `path_key`, so an exclusion may already
    // exist (here: a manual one, the pre-archive shape). The archive
    // leaves it untouched — its own reason and date are facts of
    // their own, and PRJ-017's end state is already true.
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, &repo_path);
    let seeded = {
        let path = project.path.clone();
        let path_key = project.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::Manual,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("the manual exclusion seed creates")
    };
    let webview = main_webview(&app);

    invoke_project_archive(&webview, &project.id)
        .expect("the archive converges over the existing row");

    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Archived
    );
    let exclusions = exclusions_of(&database);
    assert_eq!(exclusions.len(), 1, "no duplicate exclusion row");
    assert_eq!(exclusions[0], seeded, "the existing row is untouched");
    assert_eq!(archive_journal(&database).len(), 1);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_archive_archives_a_project_whose_folder_is_gone() {
    // "The project folder is not touched" is total (D-194): no
    // availability probe, no folder check — `projects_list` keeps
    // "Archive" an available action on an unavailable column, so a
    // dead folder must not refuse the archive.
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let never_created = temp.path().join("never-created").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &never_created, &never_created);
    let webview = main_webview(&app);

    invoke_project_archive(&webview, &project.id).expect("a dead folder never blocks the archive");

    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Archived
    );
    assert_eq!(
        exclusions_of(&database).len(),
        1,
        "the exclusion is still written"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_archive_is_refused_during_a_sync_before_anything_runs() {
    // SYS-005 (the card's own line): while a sync holds the global
    // flag, the archive answers the typed `E_SYNC_RUNNING` envelope —
    // asserted against a spec-literal JSON, not the constructor's own
    // output, so the mapping cannot drift from the wire form — and
    // nothing is written. After the run ends, the same archive
    // converges normally.
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, &repo_path);
    let webview = main_webview(&app);

    let guard = crate::commands::begin_sync(app.app_handle())
        .expect("the test's flag must be free to take");
    let error = invoke_project_archive(&webview, &project.id)
        .expect_err("the archive must be refused during a sync");
    assert_eq!(
        error,
        serde_json::json!({
            "code": "E_SYNC_RUNNING",
            "message_key": "errors.sync_running",
            "params": {},
            "details": null,
        }),
        "the §10 refusal envelope"
    );
    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Active,
        "no row was written while the flag was up"
    );
    assert_eq!(
        exclusions_of(&database),
        Vec::new(),
        "no exclusion was written"
    );
    assert_eq!(
        archive_journal(&database),
        Vec::new(),
        "no journal entry was written"
    );

    drop(guard);
    invoke_project_archive(&webview, &project.id)
        .expect("the retried archive after the run ended succeeds");
    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Archived
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_archive_answers_honestly_without_a_database() {
    // Recovery mode (the `activity_list` pin): no `Db` managed — the
    // recorded D-037 fallback with the `NoDatabase` diagnostic, never
    // a pretend success that would lie about the projects table. The
    // SYS-005 gate state recovery mode also manages, so the answer
    // comes from `managed_db`, not the gate's missing-state fallback.
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_archive::project_archive
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::sync_running::SyncRunning::new());
    let webview = main_webview(&app);

    let error =
        invoke_project_archive(&webview, "any-id").expect_err("the archive needs the database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest state answer"
    );
    drop(app);
}

#[test]
fn project_archive_refuses_an_unknown_id_with_the_typed_envelope() {
    // A stale screen's re-send: the id matches no row — the typed
    // `E_VALIDATION` refusal naming `project_id`, with nothing
    // written.
    let (temp, app) = project_archive_app();
    let database = database_of(&app);
    let webview = main_webview(&app);

    let error =
        invoke_project_archive(&webview, "no-such-row").expect_err("an unknown id must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("project_id"),
        "{error}"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("no project has this id"),
        "{error}"
    );
    assert_eq!(exclusions_of(&database), Vec::new(), "nothing was written");
    assert_eq!(
        archive_journal(&database),
        Vec::new(),
        "nothing was journaled"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_archived_project_leaves_the_sync_set_and_the_dirty_count() {
    // The card's "не участвует в синхронизации и не dirty" DoD. The
    // project starts DIRTY through the real service (an assignment,
    // an empty `last_synced_hash`, a non-empty build text —
    // `recalc_and_emit` records the verdict), then the archive runs
    // and the two halves decide: the build snapshot (ALG-001 — what
    // the sync consumes) no longer lists the project, and
    // `projects_list`'s row-decidable DB-014 arm serves `dirty:
    // false` even though the cache still holds the stale `true`
    // verdict (no service task runs here, so nothing prunes it — the
    // read is what settles it, the module docs' own contract).
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_archive::project_archive,
            crate::commands::projects_list
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    let (dirty_state, _triggers) = crate::dirty_recalc::DirtyState::new();
    app.manage(dirty_state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::dirty_recalc::DirtyEvent
        ])
        .mount_events(&app);
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, &repo_path);
    let (instruction, main) = seed_instruction(&database);
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let webview = main_webview(&app);

    let changes =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the pre-archive recalculation succeeds");
    assert_eq!(
        changes,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: project.id.clone(),
            dirty: true,
        }],
        "the world starts dirty — the verdict the cache recorded"
    );

    invoke_project_archive(&webview, &project.id)
        .expect("the archive succeeds over a dirty project");

    // The sync set: ALG-001's snapshot — what §9.4 phase 0 builds.
    let project_id = project.id.clone();
    let snapshot_ids = {
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            build_snapshot::read(tx).map_err(crate::dirty_recalc::build_snapshot_to_app_error)
        }))
        .expect("the build snapshot reads")
        .projects()
        .iter()
        .map(|snapshot_project| snapshot_project.id.as_str().to_owned())
        .collect::<Vec<_>>()
    };
    assert!(
        !snapshot_ids.contains(&project_id),
        "the archived project is out of the sync set (ALG-001 reads the active subset)"
    );
    // The dirty count: the read serves `false` for the archived row
    // while the cache still holds the stale `true` (DB-014's
    // row-decidable arm — D-128).
    let columns = match tauri::test::get_ipc_response(
        &webview,
        invoke_request(
            "projects_list",
            serde_json::json!({ "includeArchived": true }),
        ),
    ) {
        Ok(payload) => payload
            .deserialize::<Vec<crate::commands::ProjectColumnEntry>>()
            .expect("the success payload must deserialize into the DTO"),
        Err(error) => panic!("the widened read must succeed: {error}"),
    };
    let entry = columns
        .iter()
        .find(|column| column.id == project.id)
        .expect("the archived row is served by the widened read");
    assert_eq!(entry.state, "archived");
    assert!(
        !entry.dirty,
        "DB-014: an archived project is never counted as dirty"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_project_archive_command_is_registered_once_with_specta() {
    // The §7.10 Projects-module name (D-009's fixed registry),
    // registered exactly once (the D-034 wiring-pin rationale) — the
    // bindings carry `projectArchive` to the frontend through it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("project_archive::project_archive::<").count(),
        1,
        "lib.rs registers `project_archive` once"
    );
}
