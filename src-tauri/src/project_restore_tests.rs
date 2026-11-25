//! Mock-IPC behavior tests for the `project_restore` command (T-109,
//! PRJ-013…018, §6.4, D-10/DB-015, SYS-005, SYS-007, D-205).
//!
//! The command runs over a real migrated `Db` through the mock-IPC
//! invoke path; assertions read the store (row, assignments,
//! exclusions, journal), the real recalculation service's diff and
//! `projects_list`'s column, and the real project folder
//! (byte-for-byte) back. The card's DoD leads: the assignments on
//! place verbatim (with a REAL seeded sync hash), the path not in
//! the exclusions, the row active, the journal entry; the pinned
//! owner ruling (a pre-existing manual exclusion is deleted too),
//! the converging arms, the `E_SYNC_RUNNING` refusal (SYS-005), the
//! honest no-database answer, the D-051 rollback proof, the
//! row-decidable dirty verdict through the real service, and the
//! mutation-seam watch pass (SYS-007's reverse membership arm).

use eunomia_core::hash::build_hash;
use eunomia_store::action_log::{
    Action, Entity, EntityType, Level, LogEntry, LogRecord, Outcome, Visibility, insert, recent,
};
use eunomia_store::build_snapshot;
use eunomia_store::library::{
    NewAssignment, NewLibraryItem, NewVariant, assign, create_item, create_variant,
    list_assignments_for_project,
};
use eunomia_store::projects::{
    AddedVia, ExclusionReason, NewProject, ProjectState, create_exclusion,
    find_exclusion_by_path_key,
};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use tauri::Manager;

/// The mutation-refresh budget of the non-wiring tests: tiny, so the
/// degraded pass (no service task is spawned there — the tests assert
/// the RESTORE, not the pass) costs nothing at all: with the command
/// bus's receiving half dropped, the send fails fast before any wait
/// (D-156's `FileWatchGone` shape). The wiring test below uses the
/// production budget with a real service.
const DEGRADED_REFRESH_BUDGET: std::time::Duration = std::time::Duration::from_millis(20);

/// The healthy restore world: a fresh database, the SYS-005 gate's
/// flag state and the file-watch state managed (the committed restore
/// always reaches the refresh seam — production manages the state in
/// `setup`), and the command registered. No service task is spawned
/// here: the refresh degrades at the tiny budget and the restore
/// stands (D-156's never-fatal contract).
fn project_restore_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_restore::project_restore
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    let (state, _commands) =
        crate::file_watch::FileWatchState::with_budget(DEGRADED_REFRESH_BUDGET);
    app.manage(state);
    (temp, app)
}

/// The DB-021/DB-023 DoD world: the dirty service's cache and event
/// mounted (the T-107 surface — `recalc_and_emit` and
/// `projects_list`'s column) beside the restore command.
fn project_restore_dirty_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_restore::project_restore,
            crate::commands::projects_list
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    let (dirty_state, _triggers) = crate::dirty_recalc::DirtyState::new();
    app.manage(dirty_state);
    let (watch_state, _commands) =
        crate::file_watch::FileWatchState::with_budget(DEGRADED_REFRESH_BUDGET);
    app.manage(watch_state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::dirty_recalc::DirtyEvent
        ])
        .mount_events(&app);
    (temp, app)
}

/// The wiring world (SYS-007's reverse membership arm): the watch
/// state at the PRODUCTION budget, the command channel's receiving
/// half handed back so a real service task can own it, and the
/// file-changed event mounted.
fn project_restore_watch_service_app() -> (
    tempfile::TempDir,
    tauri::App<tauri::test::MockRuntime>,
    tauri::async_runtime::Receiver<crate::file_watch::Command>,
) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_restore::project_restore
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    let (state, commands) =
        crate::file_watch::FileWatchState::with_budget(crate::file_watch::MUTATION_REFRESH_BUDGET);
    app.manage(state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::file_watch::FileChangedEvent
        ])
        .mount_events(&app);
    (temp, app, commands)
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

fn invoke_project_restore(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
) -> Result<(), serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "project_restore",
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

/// One project row with a caller-chosen origin and path — the restore
/// flow's exclusion half keys by `path_key` (DB-030), so the key is
/// the parameter alongside the path. The text lands owned in the
/// executor's `'static` closure (the assignments suite's seeding
/// shape).
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
            name: "Restored One",
            label: "R1",
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
    title: &str,
    body: &str,
) -> (
    eunomia_store::library::LibraryItem,
    eunomia_store::library::Variant,
) {
    let title = title.to_owned();
    let body = body.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item =
            create_item(tx, &NewLibraryItem::Instruction { title: &title }).map_err(seed_error)?;
        let main = create_variant(
            tx,
            &NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: &body,
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

/// Stamps the row's `last_synced_hash` (the DB-024 cell a sync writes
/// — the OUT-020 fact the archive keeps for the restore, D-194
/// point 3): the store's own writer, never the command under test.
fn seed_last_synced_hash(database: &crate::db::Db, project_id: &str, hash: &str) {
    let project_id = project_id.to_owned();
    let hash = hash.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::record_last_synced(
            tx,
            &project_id,
            "2026-11-24T00:00:00.000Z",
            &hash,
        )
        .map_err(crate::commands::projects_store_to_app_error)
        .map(|_| ())
    }))
    .expect("seeding the sync fact must work");
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
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::list_exclusions(tx)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("exclusion list reads")
}

fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(|tx| recent(tx, 1000).map_err(crate::commands::activity_log_to_app_error)),
    )
    .expect("journal reads")
}

fn restore_journal(database: &crate::db::Db) -> Vec<LogRecord> {
    journal_of(database)
        .into_iter()
        .filter(|entry| entry.action == Action::ProjectRestored)
        .collect()
}

/// The world's PRE-state (§6.4): the row archived through the STORE —
/// `projects::archive` writes the row only, so the exclusion row (the
/// archive command's own composition) is seeded by the caller when a
/// test's world needs one. The store call is exactly what the archive
/// command runs over the row.
fn archive_through_the_store(database: &crate::db::Db, project_id: &str) {
    let project_id = project_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::archive(tx, &project_id)
            .map_err(crate::commands::projects_store_to_app_error)
            .map(|_| ())
    }))
    .expect("the store-level archive (the world's PRE-state) must work");
}

#[test]
fn project_restore_restores_the_row_keeps_assignments_and_hash_and_removes_the_exclusion() {
    // The §6.4/PRJ-013…018 happy path in one world: a scan-born
    // project with a real folder, a target file with known bytes, REAL
    // assignments and a REAL stored sync hash — archived (with the
    // archive's exclusion row seeded beside the row, the archive
    // command's own composition) — then restored. The restore must
    // return the row with the assignments and the hash EXACTLY where
    // the archive kept them (D-194 point 3's own contract), remove the
    // exclusion row, leave the folder byte-for-byte (no file writes),
    // and journal one attributable entry (LOG-001…004/D-205).
    let (temp, app) = project_restore_app();
    let database = database_of(&app);
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("repository folder creates");
    let target = repo.join("AGENTS.md");
    let original = b"managed by hand, not by this command\n";
    std::fs::write(&target, original).expect("target file writes");
    let repo_path = repo.display().to_string();
    // The identity oracle (the T-108 round-2 P3-1 shape): the key is
    // seeded DELIBERATELY different from the path — a regression that
    // keys the exclusion removal by the stored path instead of the
    // identity key (DB-030) must leave the exclusion row behind and
    // turn the assertions below red.
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
    seed_last_synced_hash(&database, &project.id, "a-stored-build-hash");
    let project = stored_project(&database, &project.id);
    let (instruction, main) = seed_instruction(&database, "I1", "shared text");
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let assignments_before = assignments_of(&database, &project.id);
    assert_eq!(assignments_before.len(), 1, "the world starts assigned");
    // The world's PRE-state: the archived row plus the archive
    // command's exclusion row (§6.4's pair), seeded through the store
    // — the archive's own writes, one layer down. A NEIGHBOUR
    // exclusion for an unrelated path key rides along: the deletion's
    // scoping oracle (a regression that wiped the table, or deleted
    // by a broader predicate, would take this row too).
    archive_through_the_store(&database, &project.id);
    let neighbour = {
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: "/an-unrelated-path",
                path_key: "/an-unrelated-key",
                reason: ExclusionReason::Manual,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("seeding the neighbour exclusion must work")
    };
    {
        let path = project.path.clone();
        let path_key = project.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::RemovedScanned,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("seeding the archive's exclusion row must work");
    }
    let webview = main_webview(&app);

    invoke_project_restore(&webview, &project.id).expect("restoring an archived project succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.state, ProjectState::Active, "the row is back");
    assert_eq!(
        stored.archived_at, None,
        "the stamp is cleared (the invariant)"
    );
    assert_eq!(
        stored.sort_order, project.sort_order,
        "the position is kept (the store's §6.4 reading)"
    );
    assert_eq!(stored.path_key, project.path_key, "DB-015: the key is kept");
    assert_eq!(stored.path, project.path, "the path is immutable");
    // DB-014 + D-194 point 3 + D-205 point 6: the assignments and the
    // stored hash are where the archive left them — the restore
    // writes no hash of its own.
    assert_eq!(
        assignments_of(&database, &project.id),
        assignments_before,
        "PRJ-013…018: the restored project keeps its assignments"
    );
    assert_eq!(
        stored.last_synced_hash, project.last_synced_hash,
        "the stored hash is not churned by the restore"
    );
    // PRJ-013…018's second half: the path is not in the exclusions —
    // the target row alone is gone, the neighbour survives (the
    // deletion's scoping).
    assert_eq!(
        exclusions_of(&database),
        vec![neighbour],
        "the archive-born exclusion row is removed; the neighbour stays"
    );
    // §6.4's folder guarantee: byte-for-byte on the file — the watch
    // refresh the committed restore awaits only READS target files,
    // and here (no service task) no pass runs at all.
    assert_eq!(
        std::fs::read(&target).expect("target reads"),
        original,
        "no file write"
    );
    // LOG-001…004/D-205 point 7: ONE attributable entry, no details
    // (the row itself holds the path). The TOTAL count (not just the
    // `ProjectRestored` filter) pins point 7's other half: no
    // separate `exclusion.removed` member rides along.
    let journal = restore_journal(&database);
    assert_eq!(journal.len(), 1, "exactly one project.restored entry");
    assert_eq!(
        journal_of(&database).len(),
        1,
        "the whole journal holds one entry — no exclusion.removed member"
    );
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
fn a_preexisting_manual_exclusion_is_deleted_too() {
    // The pinned owner ruling (T-108's card comment, D-194 round-2
    // P3-6, D-205 point 1): the archive deliberately leaves a
    // pre-existing MANUAL exclusion with the row's `path_key` in
    // place — the restore deletes it. PRJ-013…018's end state names
    // the path, not a row class, so the deletion is reason-agnostic.
    let (temp, app) = project_restore_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, "/repo-key");
    // The manual exclusion PREDATES the archive (the archive's own
    // convergence arm — D-194 point 1 — leaves it untouched).
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
    assert_eq!(
        seeded.reason,
        ExclusionReason::Manual,
        "the fixture is the manual arm"
    );
    archive_through_the_store(&database, &project.id);
    assert_eq!(
        exclusions_of(&database),
        vec![seeded],
        "the archive left the manual row in place (D-194 point 1)"
    );
    let webview = main_webview(&app);

    invoke_project_restore(&webview, &project.id)
        .expect("the restore over the manual exclusion succeeds");

    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Active
    );
    assert_eq!(
        exclusions_of(&database),
        Vec::new(),
        "the pinned ruling: the manual exclusion is deleted too"
    );
    assert_eq!(
        restore_journal(&database).len(),
        1,
        "one project.restored entry"
    );
    assert_eq!(
        journal_of(&database).len(),
        1,
        "the whole journal holds one entry — no exclusion.removed member"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_restore_without_any_exclusion_row_converges() {
    // D-205 point 2, the no-existence arm: a row whose exclusion was
    // hand-removed while it was archived (the §6.4 physical-delete
    // act — the T-108 round-1 "hand-removed exclusion stays removed"
    // mirror, read from the restore side) restores fine — the removal
    // end state is already true, the transition and the journal entry
    // are the act. The manual-born arm (the archive wrote no row at
    // all) is the same find-answers-`None` branch, pinned beside it.
    // Both worlds live on never-created folders: no availability
    // probe exists (D-205 point 8 — a dead folder restores fine).
    let (temp, app) = project_restore_app();
    let database = database_of(&app);
    let scanned_path = temp.path().join("scanned").display().to_string();
    let scanned = seed_project(&database, AddedVia::Scan, &scanned_path, "/scanned-key");
    let manual_path = temp.path().join("manual").display().to_string();
    let manual = seed_project(&database, AddedVia::Manual, &manual_path, "/manual-key");
    // The scan-born world: archived with the exclusion row (the
    // archive's composition), then the hand-removal through the store
    // — the exclusions screen's own path.
    archive_through_the_store(&database, &scanned.id);
    {
        let path = scanned.path.clone();
        let path_key = scanned.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::RemovedScanned,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("seeding the archive's exclusion row must work");
    }
    {
        let path_key = scanned.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let exclusion = find_exclusion_by_path_key(tx, &path_key)
                .map_err(crate::commands::projects_store_to_app_error)?
                .expect("the archive wrote the exclusion row");
            eunomia_store::projects::delete_exclusion(tx, &exclusion.id)
                .map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("the hand-removal succeeds");
    }
    // The manual-born world: the archive wrote no exclusion (PRJ-001's
    // picker is its way back).
    archive_through_the_store(&database, &manual.id);
    assert_eq!(
        exclusions_of(&database),
        Vec::new(),
        "both worlds carry no exclusion row"
    );
    let webview = main_webview(&app);

    invoke_project_restore(&webview, &scanned.id).expect("the hand-removed world restores fine");
    invoke_project_restore(&webview, &manual.id).expect("the manual-born world restores fine");

    assert_eq!(
        stored_project(&database, &scanned.id).state,
        ProjectState::Active
    );
    assert_eq!(
        stored_project(&database, &manual.id).state,
        ProjectState::Active
    );
    assert_eq!(
        exclusions_of(&database),
        Vec::new(),
        "no exclusion churn — nothing to remove, nothing re-added"
    );
    assert_eq!(
        restore_journal(&database).len(),
        2,
        "each restore journals its own entry (the transition is the act)"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_second_restore_is_a_converging_noop() {
    // D-205 point 3, the D-194 point-2 mirror: a re-send on an
    // already-active row answers Ok with NOTHING written — no second
    // journal entry, no `updated_at` restamp, no exclusion churn even
    // when an exclusion row with the same `path_key` has since
    // appeared (a fact the re-send did not own; D-194's own
    // no-override posture read from the restore side).
    let (temp, app) = project_restore_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, "/repo-key");
    archive_through_the_store(&database, &project.id);
    {
        let path = project.path.clone();
        let path_key = project.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::RemovedScanned,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("seeding the archive's exclusion row must work");
    }
    let webview = main_webview(&app);
    invoke_project_restore(&webview, &project.id).expect("the first restore succeeds");
    let after_first = stored_project(&database, &project.id);
    assert_eq!(after_first.state, ProjectState::Active);
    let journal_after_first = restore_journal(&database).len();
    // A manual exclusion appears while the row is active (a seeded
    // world — the store allows the pair, D-194 point 1).
    {
        let path = project.path.clone();
        let path_key = project.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::Manual,
            };
            create_exclusion(tx, &new)
                .map_err(crate::commands::projects_store_to_app_error)
                .map(|_| ())
        }))
        .expect("the post-restore exclusion seed creates")
    }
    let exclusions_before = exclusions_of(&database);

    invoke_project_restore(&webview, &project.id).expect("the re-send converges with Ok");

    let after_second = stored_project(&database, &project.id);
    assert_eq!(after_second.state, ProjectState::Active);
    assert_eq!(
        after_second.updated_at, after_first.updated_at,
        "the row is not restamped by the re-send"
    );
    assert_eq!(
        restore_journal(&database).len(),
        journal_after_first,
        "the re-send journals nothing"
    );
    assert_eq!(
        exclusions_of(&database),
        exclusions_before,
        "the re-send does not override a fact it did not own"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_restore_is_refused_during_a_sync_before_anything_runs() {
    // SYS-005 (the family's own line): while a sync holds the global
    // flag, the restore answers the typed `E_SYNC_RUNNING` envelope —
    // asserted against a spec-literal JSON, not the constructor's own
    // output, so the mapping cannot drift from the wire form — and
    // nothing is written (the row stays archived, the exclusion
    // stays, the journal claims nothing). After the run ends, the
    // same restore converges normally.
    let (temp, app) = project_restore_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, "/repo-key");
    archive_through_the_store(&database, &project.id);
    {
        let path = project.path.clone();
        let path_key = project.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::RemovedScanned,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("seeding the archive's exclusion row must work");
    }
    let webview = main_webview(&app);

    let guard = crate::commands::begin_sync(app.app_handle())
        .expect("the test's flag must be free to take");
    let error = invoke_project_restore(&webview, &project.id)
        .expect_err("the restore must be refused during a sync");
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
        ProjectState::Archived,
        "no row was written while the flag was up"
    );
    assert_eq!(
        exclusions_of(&database).len(),
        1,
        "the exclusion was not removed"
    );
    assert_eq!(
        restore_journal(&database),
        Vec::new(),
        "no journal entry was written"
    );

    drop(guard);
    invoke_project_restore(&webview, &project.id)
        .expect("the retried restore after the run ended succeeds");
    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Active
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_restore_answers_honestly_without_a_database() {
    // Recovery mode (the `activity_list` pin): no `Db` managed — the
    // recorded D-037 fallback with the `NoDatabase` diagnostic, never
    // a pretend success that would lie about the projects table. The
    // SYS-005 gate state recovery mode also manages, so the answer
    // comes from `managed_db`, not the gate's missing-state fallback.
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_restore::project_restore
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::sync_running::SyncRunning::new());
    let (watch_state, _commands) =
        crate::file_watch::FileWatchState::with_budget(DEGRADED_REFRESH_BUDGET);
    app.manage(watch_state);
    let webview = main_webview(&app);

    let error =
        invoke_project_restore(&webview, "any-id").expect_err("the restore needs the database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest state answer"
    );
    drop(app);
}

#[test]
fn project_restore_refuses_an_unknown_id_with_the_typed_envelope() {
    // A stale screen's re-send: the id matches no row — the typed
    // `E_VALIDATION` refusal naming `project_id`, with the bounded
    // echo (D-115's cap: a webview string is unbounded by
    // definition), and nothing written. The world starts LOADED (an
    // archived neighbour with its exclusion row and its journal
    // entry) precisely so the "nothing written" half can fail: an
    // implementation that touched anything would move one of them
    // (round 1's P3-2).
    let (temp, app) = project_restore_app();
    let database = database_of(&app);
    let neighbour_path = temp.path().join("neighbour").display().to_string();
    let neighbour = seed_project(&database, AddedVia::Scan, &neighbour_path, "/neighbour-key");
    archive_through_the_store(&database, &neighbour.id);
    let neighbour_exclusion = {
        let path = neighbour.path.clone();
        let path_key = neighbour.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::RemovedScanned,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("seeding the neighbour exclusion must work")
    };
    {
        let neighbour_id = neighbour.id.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let mut entry = LogEntry::new(
                Action::ProjectArchived,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: neighbour_id.as_str(),
            });
            entry.project_id = Some(neighbour_id.as_str());
            insert(tx, &entry)
                .map_err(crate::commands::activity_log_to_app_error)
                .map(|_| ())
        }))
        .expect("seeding the neighbour journal entry must work")
    }
    let webview = main_webview(&app);

    let error =
        invoke_project_restore(&webview, "no-such-row").expect_err("an unknown id must be refused");
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
    // D-115's cap, pinned at the boundary: an over-long id echoes
    // truncated with the suffix, never whole.
    let long_id = "x".repeat(70);
    let error = invoke_project_restore(&webview, &long_id)
        .expect_err("an over-long unknown id must be refused the same way");
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(format!("{} (+6 more code points)", "x".repeat(64))),
        "{error}"
    );
    // Nothing was written: the loaded world stands byte-for-byte —
    // the neighbour's row, exclusion and journal entry all untouched.
    assert_eq!(
        stored_project(&database, &neighbour.id).state,
        ProjectState::Archived,
        "the neighbour's row was not touched"
    );
    assert_eq!(
        exclusions_of(&database),
        vec![neighbour_exclusion],
        "the neighbour's exclusion was not touched"
    );
    assert_eq!(
        journal_of(&database).len(),
        1,
        "the neighbour's journal entry was not touched"
    );
    assert_eq!(
        restore_journal(&database),
        Vec::new(),
        "nothing was journaled"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_failed_journal_insert_rolls_back_the_row_and_the_exclusion_removal_together() {
    // D-051's in-transaction semantics, proven through the real
    // command (the T-108 trigger shape): the row transition, the
    // exclusion deletion and the journal entry share ONE `Db::run`
    // transaction, so a failure AFTER both writes succeeded — the
    // journal insert, staged to abort by a trigger — rolls all of it
    // back: the row stays archived with its stamp, the exclusion row
    // survives, and the journal claims nothing. The failure is staged
    // deterministically inside the database (RAISE(ABORT) →
    // SQLITE_CONSTRAINT → the shared base classifier's `E_VALIDATION`
    // with the SQLite facts in details).
    let (temp, app) = project_restore_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &repo_path, "/repo-key");
    archive_through_the_store(&database, &project.id);
    {
        let path = project.path.clone();
        let path_key = project.path_key.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let new = eunomia_store::projects::NewExclusion {
                path: &path,
                path_key: &path_key,
                reason: ExclusionReason::RemovedScanned,
            };
            create_exclusion(tx, &new).map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("seeding the archive's exclusion row must work");
    }
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_every_journal_entry BEFORE INSERT ON action_log
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");
    let webview = main_webview(&app);

    let error = invoke_project_restore(&webview, &project.id)
        .expect_err("the staged abort must fail the restore");
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
    // the exclusion and the journal entry all stand as they were.
    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.state, ProjectState::Archived, "the row rolled back");
    assert!(stored.archived_at.is_some(), "the stamp survived the abort");
    assert_eq!(
        exclusions_of(&database).len(),
        1,
        "the exclusion deletion rolled back"
    );
    assert_eq!(
        restore_journal(&database),
        Vec::new(),
        "no entry claimed a restore"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_restored_project_is_dirty_exactly_as_the_service_decides() {
    // D-205 point 6, the card's dirty half pinned the T-107 way: the
    // command writes no hash (the stored cell is the pre-archive
    // fact), and the verdict is the DB-022 service's own over the
    // now-active row (DB-021 over DB-020). Two worlds in one database:
    // CLEAN — the world unchanged since the last sync answers nothing;
    // DIRTY — an assignment landed while the row was archived (the
    // store's `assign` is state-blind), so the recomputed build
    // differs from the stored pre-archive hash. Both verdicts are
    // read back through `projects_list`'s column, the row-decidable
    // read the UI itself sees.
    let (temp, app) = project_restore_dirty_app();
    let database = database_of(&app);
    // The CLEAN world: a manual-born project (no exclusion half), one
    // assignment, the synced hash of exactly that build.
    let clean_repo = temp.path().join("clean-repo");
    std::fs::create_dir_all(clean_repo.join(".git")).expect("repository folder creates");
    let clean_target = clean_repo.join("AGENTS.md");
    let clean = seed_project(
        &database,
        AddedVia::Manual,
        &clean_repo.display().to_string(),
        "/clean-repo-key",
    );
    let (clean_instruction, clean_main) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(&database, &clean.id, &clean_instruction.id, &clean_main.id);
    seed_last_synced_hash(
        &database,
        &clean.id,
        &synced_hash_of(&clean_target, "Be brief."),
    );
    // The DIRTY world: a scan-born project with the same one-block
    // build and hash, then a SECOND assignment seeded while the row
    // is archived.
    let edited_repo = temp.path().join("edited-repo");
    std::fs::create_dir_all(edited_repo.join(".git")).expect("repository folder creates");
    let edited_target = edited_repo.join("AGENTS.md");
    let edited = seed_project(
        &database,
        AddedVia::Scan,
        &edited_repo.display().to_string(),
        "/edited-repo-key",
    );
    seed_assignment(&database, &edited.id, &clean_instruction.id, &clean_main.id);
    seed_last_synced_hash(
        &database,
        &edited.id,
        &synced_hash_of(&edited_target, "Be brief."),
    );
    let (extra_instruction, extra_main) = seed_instruction(&database, "Tone", "Stay kind.");
    // The independent baseline: the pass over the unmodified ACTIVE
    // rows diffs nothing, or every verdict below would prove nothing.
    let baseline =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the baseline pass must succeed");
    assert_eq!(
        baseline,
        Vec::new(),
        "both worlds start clean — the control is the oracle's own"
    );
    let webview = main_webview(&app);
    // Archive both worlds (the PRE-state), then edit-while-archived:
    // the second assignment lands on the archived row through the
    // store (the store's `assign` is state-blind — the user's library
    // edits during an archived life are exactly DB-014's kept data).
    archive_through_the_store(&database, &clean.id);
    archive_through_the_store(&database, &edited.id);
    seed_assignment(&database, &edited.id, &extra_instruction.id, &extra_main.id);
    // The archived rows are outside the service's active set: the
    // pass between the archive and the restore answers nothing.
    let while_archived =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the archived pass must succeed");
    assert_eq!(
        while_archived,
        Vec::new(),
        "an archived row contributes no verdict (DB-014's own rule)"
    );

    invoke_project_restore(&webview, &clean.id).expect("the clean world restores");
    invoke_project_restore(&webview, &edited.id).expect("the edited world restores");

    // The verdicts through the real service: clean stays clean (the
    // stored hash still covers the unchanged build), edited answers
    // dirty (the assignment that landed while archived is in the
    // recomputed build now).
    let verdicts =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the post-restore pass must succeed");
    assert_eq!(
        verdicts,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: edited.id.clone(),
            dirty: true,
        }],
        "the service — not the command — decides: unchanged answers clean, \
         edited-while-archived answers dirty"
    );
    assert!(
        !dirty_column_of(&webview, &clean.id),
        "the unchanged world's column reads clean"
    );
    assert!(
        dirty_column_of(&webview, &edited.id),
        "the edited world's column carries the unsynced dot"
    );
    // The command wrote no hash: both cells still hold the seeded
    // pre-archive values.
    assert_eq!(
        stored_project(&database, &clean.id)
            .last_synced_hash
            .as_deref(),
        Some(synced_hash_of(&clean_target, "Be brief.").as_str()),
        "OUT-020: the hash cell keeps the pre-archive value"
    );
    assert_eq!(
        stored_project(&database, &edited.id)
            .last_synced_hash
            .as_deref(),
        Some(synced_hash_of(&edited_target, "Be brief.").as_str()),
        "OUT-020: the hash cell keeps the pre-archive value (the dirty \
         verdict is the service's, never a written cell)"
    );
    // The restored project is back in the sync set (ALG-001's active
    // read — D-194 point 3's construction reversed).
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
        snapshot_ids.contains(&clean.id) && snapshot_ids.contains(&edited.id),
        "the restored projects are back in the sync set"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_restore_runs_one_awaited_watch_pass_through_the_mutation_seam() {
    // D-205 point 5, the wiring's proof (SYS-007/D-144, T-279's arm):
    // the committed restore — the row actually returned to the active
    // set — awaits ONE full watch pass through the bounded mutation
    // seam, the same refresh path (command, ack, service pass)
    // `project_add_manual`'s committed arm and T-107's arm run. The
    // deterministic observable is the pass counter, and the awaited
    // seam means the count is settled when the invoke returns. The
    // pass reads target files and writes nothing (OUT-011).
    let (temp, app, commands) = project_restore_watch_service_app();
    let repo = repo_with_agents_md(temp.path());
    let database = database_of(&app);
    // The project is seeded ACTIVE (the startup baseline needs a
    // watched file), then archived through the STORE — the world's
    // PRE-state, and the store-level archive fires no pass of its own
    // (D-194: the archive runs no refresh; the leaver exits at the
    // next anchor's pass, which nothing here triggers).
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &repo.display().to_string(),
        "/repo-key",
    );
    crate::file_watch::spawn_service(
        app.handle().clone(),
        commands,
        crate::ipc_tests::WATCH_DEBOUNCE,
    )
    .expect("the watcher must start");
    // The settled startup baseline (the repo's own macOS FSEvents
    // discipline: a pre-spawn fixture write can still deliver a late
    // debounced pass after the stream starts, so absolute pass
    // indexing is a CI flake — the delta over a QUIESCENT count is
    // the deterministic shape). The helper rewrites the target with
    // its own just-read bytes, so any disk oracle below still sees
    // the fixture's bytes.
    let settled = crate::ipc_tests::converge_startup_watch(&app, &repo.join("AGENTS.md"));
    archive_through_the_store(&database, &project.id);
    let webview = main_webview(&app);

    invoke_project_restore(&webview, &project.id)
        .expect("the restore succeeds — the refresh is never-fatal aftermath");

    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled + 1,
        "the awaited seam ran exactly one full pass after the settled startup baseline"
    );
    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Active,
        "the row the pass covered is back"
    );
    // Display-only even through the mutation's refresh: the pass
    // reads target files, it never writes them (OUT-011).
    assert_eq!(
        std::fs::read(repo.join("AGENTS.md")).expect("the file must be readable"),
        b"hand-written content\n".to_vec(),
        "the watch pass writes no target file"
    );
    // Windows teardown rule.
    crate::ipc_tests::stop_watch_service(app.handle());
    drop(app);
    drop(temp);
}

#[test]
fn a_converging_restore_resend_runs_no_watch_pass() {
    // The wiring's negative half (D-205 point 5): a re-send that
    // converges (the row is already active) runs NO pass — a pass
    // over a row that did not move is file reads with no observable
    // change (D-201 point 7's own boundary). Deterministic without
    // any sleep: the seam is AWAITED inside the command, so nothing
    // is left in flight when the invoke returns — the count cannot
    // move after the assert (no service trigger runs, and no file
    // event fires over an untouched repo).
    let (temp, app, commands) = project_restore_watch_service_app();
    let repo = repo_with_agents_md(temp.path());
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &repo.display().to_string(),
        "/repo-key",
    );
    crate::file_watch::spawn_service(
        app.handle().clone(),
        commands,
        crate::ipc_tests::WATCH_DEBOUNCE,
    )
    .expect("the watcher must start");
    let settled = crate::ipc_tests::converge_startup_watch(&app, &repo.join("AGENTS.md"));
    archive_through_the_store(&database, &project.id);
    let webview = main_webview(&app);
    invoke_project_restore(&webview, &project.id).expect("the first restore runs the pass");
    let after_first = app
        .state::<crate::file_watch::FileWatchState>()
        .pass_count();
    assert_eq!(after_first, settled + 1, "the committed arm ran one pass");

    invoke_project_restore(&webview, &project.id).expect("the re-send converges with Ok");

    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        after_first,
        "the converging re-send runs no watch pass"
    );
    // Windows teardown rule.
    crate::ipc_tests::stop_watch_service(app.handle());
    drop(app);
    drop(temp);
}

#[test]
fn the_project_restore_command_is_registered_once_with_specta() {
    // The §7.10 Projects-module name (D-009's fixed registry),
    // registered exactly once (the D-034 wiring-pin rationale) — the
    // bindings carry `projectRestore` to the frontend through it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("project_restore::project_restore::<").count(),
        1,
        "lib.rs registers `project_restore` once"
    );
}

/// The DB-024 hash of the plain one-block body over the fixture
/// project's write configuration — the value a sync of that build
/// stores, spelled directly through the public core seam (never
/// through the code under test; the T-107 fixture's own shape). With
/// both extra files off the managed block IS the body (ALG-030 joins
/// no reading lines around it).
fn synced_hash_of(target: &std::path::Path, body: &str) -> String {
    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    build_hash(eunomia_core::project::Scheme::A, target, off, off, body)
}

/// One repository directory with a `.git` entry (the watch pass's
/// world) and a hand-written target file with KNOWN bytes (the disk
/// oracle: whatever the command does, these bytes must survive it).
fn repo_with_agents_md(root: &std::path::Path) -> std::path::PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".git"))
        .expect("creating the repository folder and its .git entry must work");
    std::fs::write(repo.join("AGENTS.md"), b"hand-written content\n")
        .expect("writing the old target file must work");
    repo
}

/// Reads the project's `dirty` flag through `projects_list`'s column —
/// the row-decidable read the UI itself sees (D-128; T-108's surface).
fn dirty_column_of(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
) -> bool {
    let columns = match tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "projects_list",
            serde_json::json!({ "includeArchived": false }),
        ),
    ) {
        Ok(payload) => payload
            .deserialize::<Vec<crate::commands::ProjectColumnEntry>>()
            .expect("the success payload must deserialize into the DTO"),
        Err(error) => panic!("the column read must succeed: {error}"),
    };
    columns
        .iter()
        .find(|column| column.id == project_id)
        .unwrap_or_else(|| panic!("the restored row must be served: {project_id}"))
        .dirty
}
