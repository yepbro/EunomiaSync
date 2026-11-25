//! Mock-IPC behavior tests for the `project_relink` command (T-110,
//! D-09, DB-015, SYS-005, SYS-007, D-208).
//!
//! The command runs over a real migrated `Db` through the mock-IPC
//! invoke path, and every picked folder goes through the real dialog
//! seam (`dialog_pick_folder_with`) so the SEC-003 session allowlist
//! holds exactly what a production pick would leave. Assertions read
//! the store (row, assignments, journal), the real availability
//! service's diff and `projects_list`'s columns, the real dirty
//! service's diff, the real project folders (byte-for-byte), and the
//! watch service's pass counter. The card's DoD leads: the project
//! available again after the path change (the real probe surfaces),
//! the assignments preserved verbatim; the root resolution, the
//! converging arms, the two DB-015 collision refusals (the archived
//! arm carrying the restore offer's id), the non-repo and
//! foreign-path refusals, the `E_SYNC_RUNNING` gate (SYS-005), the
//! honest no-database answer, the D-051 rollback proof, and the
//! mutation-seam watch pass.

use eunomia_core::hash::build_hash;
use eunomia_store::action_log::{
    Action, Entity, EntityType, Level, LogEntry, LogRecord, Outcome, Visibility, insert, recent,
};
use eunomia_store::library::{
    NewAssignment, NewLibraryItem, NewVariant, assign, create_item, create_variant,
    list_assignments_for_project,
};
use eunomia_store::projects::{AddedVia, NewProject, ProjectState};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use tauri::Manager;

/// The mutation-refresh budget of the non-wiring tests: tiny, so the
/// degraded pass (no service task is spawned there — the tests assert
/// the RELINK, not the pass) costs nothing at all: with the command
/// bus's receiving half dropped, the send fails fast before any wait
/// (D-156's `FileWatchGone` shape). The wiring tests below use the
/// production budget with a real service.
const DEGRADED_REFRESH_BUDGET: std::time::Duration = std::time::Duration::from_millis(20);

/// The healthy relink world: a fresh database, the SYS-005 gate's
/// flag state, the SEC-003 session allowlist and the file-watch state
/// managed (the committed relink always reaches the refresh seam —
/// production manages the state in `setup`), and the command
/// registered. No service task is spawned here: the refresh degrades
/// at the tiny budget and the relink stands (D-156's never-fatal
/// contract).
fn project_relink_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_relink::project_relink
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    app.manage(crate::session_paths::SessionPaths::new());
    let (state, _commands) =
        crate::file_watch::FileWatchState::with_budget(DEGRADED_REFRESH_BUDGET);
    app.manage(state);
    (temp, app)
}

/// The verdicts world (the DoD's two service halves): the
/// availability service's cache and event, the dirty service's cache
/// and event, and `projects_list` (the column read the UI itself
/// sees) beside the relink command — the production `setup` shape.
fn project_relink_verdicts_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_relink::project_relink,
            crate::commands::projects_list
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    app.manage(crate::session_paths::SessionPaths::new());
    let (dirty_state, _triggers) = crate::dirty_recalc::DirtyState::new();
    app.manage(dirty_state);
    app.manage(crate::availability::AvailabilityState::new());
    let (watch_state, _commands) =
        crate::file_watch::FileWatchState::with_budget(DEGRADED_REFRESH_BUDGET);
    app.manage(watch_state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::dirty_recalc::DirtyEvent,
            crate::availability::ProjectAvailabilityEvent
        ])
        .mount_events(&app);
    (temp, app)
}

/// The wiring world (SYS-007's path-input arm): the watch state at
/// the PRODUCTION budget, the command channel's receiving half handed
/// back so a real service task can own it, and the file-changed event
/// mounted.
fn project_relink_watch_service_app() -> (
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
            crate::project_relink::project_relink
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::sync_running::SyncRunning::new());
    app.manage(crate::session_paths::SessionPaths::new());
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

fn invoke_project_relink(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
    path: &str,
) -> Result<(), serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "project_relink",
            serde_json::json!({ "projectId": project_id, "path": path }),
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

/// The pick half of every relink test (the add suite's shape): the
/// folder goes through the real dialog seam
/// (`dialog_pick_folder_with`), so the session allowlist holds
/// exactly what a production pick would leave — the CANONICAL form
/// the SEC-003 gate then accepts, and the string the Locate flow
/// would send back.
fn pick_folder(app: &tauri::App<tauri::test::MockRuntime>, folder: &std::path::Path) -> String {
    let folder = folder.to_path_buf();
    tauri::async_runtime::block_on(crate::commands::dialog_pick_folder_with(
        app.handle().clone(),
        move |_: &tauri::AppHandle<tauri::test::MockRuntime>| Ok(Some(folder.clone())),
    ))
    .expect("the pick must succeed")
    .expect("a completed pick answers a path")
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

/// One project row with a caller-chosen origin, path and key — the
/// key is the parameter beside the path (DB-030's identity is its own
/// column; the test worlds seed deliberate, non-derived keys so a
/// regression that derives the new key wrongly reddens the
/// assertions). The text lands owned in the executor's `'static`
/// closure (the assignments suite's seeding shape).
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
            name: "Relinked One",
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
/// — the OUT-020 fact the relink keeps, D-208 point 4): the store's
/// own writer, never the command under test.
fn seed_last_synced_hash(database: &crate::db::Db, project_id: &str, hash: &str) {
    let project_id = project_id.to_owned();
    let hash = hash.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::record_last_synced(
            tx,
            &project_id,
            "2026-11-25T00:00:00.000Z",
            &hash,
        )
        .map_err(crate::commands::projects_store_to_app_error)
        .map(|_| ())
    }))
    .expect("seeding the sync fact must work")
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

fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(|tx| recent(tx, 1000).map_err(crate::commands::activity_log_to_app_error)),
    )
    .expect("journal reads")
}

fn relink_journal(database: &crate::db::Db) -> Vec<LogRecord> {
    journal_of(database)
        .into_iter()
        .filter(|entry| entry.action == Action::ProjectChanged)
        .collect()
}

/// The DB-024 hash of the plain one-block body over the given
/// fixture target — the value a sync of that build stores, spelled
/// directly through the public core seam (never through the code
/// under test; the restore suite's own shape). With both extra files
/// off the managed block IS the body (ALG-030 joins no reading lines
/// around it).
fn synced_hash_of(target: &std::path::Path, body: &str) -> String {
    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    build_hash(eunomia_core::project::Scheme::A, target, off, off, body)
}

/// One repository directory with a `.git` entry (the probe's and the
/// watch pass's world) and a hand-written target file with KNOWN
/// bytes (the disk oracle: whatever the command does, these bytes
/// must survive it).
fn repo_with_agents_md(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let repo = root.join(name);
    std::fs::create_dir_all(repo.join(".git"))
        .expect("creating the repository folder and its .git entry must work");
    std::fs::write(repo.join("AGENTS.md"), b"hand-written content\n")
        .expect("writing the old target file must work");
    repo
}

/// The project's column entry through `projects_list` — the read the
/// UI itself sees (the `available`/`dirty` halves of the DoD).
fn column_entry_of(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
) -> crate::commands::ProjectColumnEntry {
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
        .into_iter()
        .find(|column| column.id == project_id)
        .unwrap_or_else(|| panic!("the row must be served: {project_id}"))
}

#[test]
fn project_relink_moves_the_row_keeps_assignments_and_hash_and_journals_once() {
    // The D-09/D-208 happy path in one world: a scan-born project
    // over a real old repository with a hand-written target file,
    // REAL assignments and a REAL stored sync hash — the folder
    // "moved" (the user picked its new home). The relink must move
    // the row's `path` and `path_key` to the new root's canonical
    // identity (the derivation the add flow uses), keep the
    // assignments and the stored hash EXACTLY where they were (the
    // family posture, D-208 point 4), leave both target files
    // byte-for-byte (no file writes), restamp only `updated_at`, and
    // journal one attributable entry (LOG-001…004/D-208 point 3).
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let old_target = old_repo.join("AGENTS.md");
    // The seeded identity is deliberately NOT the derivation of the
    // path — a regression that copies the stored path into the new
    // key (or vice versa) cannot stay green over this world.
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &old_repo.display().to_string(),
        "/an-old-identity-key",
    );
    let (instruction, main) = seed_instruction(&database, "I1", "shared text");
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let assignments_before = assignments_of(&database, &project.id);
    assert_eq!(assignments_before.len(), 1, "the world starts assigned");
    // The kept-fact oracle needs a REAL stored sync hash: a
    // never-synced row would compare `None` to `None` — vacuous (the
    // D-194 round-1 lesson).
    seed_last_synced_hash(
        &database,
        &project.id,
        &synced_hash_of(&old_target, "shared text"),
    );
    let before = stored_project(&database, &project.id);
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    // Sleeping past a millisecond boundary before the relink (the
    // D-031 store-suite precedent) makes the stamp assertion below
    // discriminating: seeding and the relink cannot land on the same
    // millisecond, so a relink that stopped restamping `updated_at`
    // would keep it equal and fail it.
    std::thread::sleep(std::time::Duration::from_millis(2));
    let webview = main_webview(&app);

    invoke_project_relink(&webview, &project.id, &picked)
        .expect("relinking to a live repository succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(
        stored.path, picked,
        "the row points at the new root's canonical form"
    );
    assert_eq!(
        stored.path_key,
        eunomia_fs::paths::canonical_path_key(&new_repo)
            .expect("the freshly created repository folder must canonicalize"),
        "the key is the canonical derivation (DB-030, the add flow's identity half)"
    );
    assert_ne!(
        stored.path_key, project.path_key,
        "the identity moved with the path"
    );
    // Everything the move does not own is verbatim.
    assert_eq!(stored.state, ProjectState::Active);
    assert_eq!(stored.sort_order, before.sort_order);
    assert_eq!(stored.name, before.name);
    assert_eq!(stored.added_via, before.added_via);
    assert_eq!(stored.created_at, before.created_at);
    assert!(
        stored.updated_at.as_str() > before.updated_at.as_str(),
        "the row's edit stamp moved: {} vs {}",
        stored.updated_at,
        before.updated_at
    );
    // The DoD's second half: the assignments verbatim.
    assert_eq!(
        assignments_of(&database, &project.id),
        assignments_before,
        "the DoD: the assignments survive the path change"
    );
    // OUT-020: the sync facts are the sync's own — the command writes
    // no hash of its own.
    assert_eq!(
        stored.last_synced_hash, before.last_synced_hash,
        "the stored hash is not churned by the relink"
    );
    assert_eq!(stored.last_synced_at, before.last_synced_at);
    // LOG-001…004/D-208 point 3: ONE attributable entry, no details
    // (the `project_update` shape: the row itself holds the facts).
    // The TOTAL count (not just the filtered one) pins the other
    // half: no second member rides along.
    let journal = relink_journal(&database);
    assert_eq!(journal.len(), 1, "exactly one project.changed entry");
    assert_eq!(
        journal_of(&database).len(),
        1,
        "the whole journal holds one entry — no other member"
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
    // No file writes (D-208 point 8): the old target survives
    // byte-for-byte (the copied-repository world — the old folder may
    // well still exist), and so does the new one.
    assert_eq!(
        std::fs::read(&old_target).expect("the old target reads"),
        b"hand-written content\n".to_vec(),
        "no file write at the old location"
    );
    assert_eq!(
        std::fs::read(new_repo.join("AGENTS.md")).expect("the new target reads"),
        b"hand-written content\n".to_vec(),
        "no file write at the new location"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_subfolder_pick_relinks_to_the_canonical_repository_root() {
    // D-208 point 2: the card's dependency sentence — find the git
    // repository ROOT for the selected folder. A pick two levels
    // inside the moved repository resolves to the repository's
    // canonical root (nearest root wins, D-06): the row's path and
    // key are the root's, never the subfolder's.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let gone_path = temp.path().join("gone-repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &gone_path, "/gone-key");
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let deep = new_repo.join("src").join("fs");
    std::fs::create_dir_all(&deep).expect("the subfolder creates");
    let picked = pick_folder(&app, &deep);
    let webview = main_webview(&app);

    invoke_project_relink(&webview, &project.id, &picked)
        .expect("relinking from a subfolder pick succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(
        stored.path,
        eunomia_fs::paths::canonicalize(&new_repo)
            .expect("the repository folder must canonicalize")
            .display()
            .to_string(),
        "the row holds the repository ROOT, not the picked subfolder"
    );
    assert_eq!(
        stored.path_key,
        eunomia_fs::paths::canonical_path_key(&new_repo)
            .expect("the repository folder must canonicalize"),
        "the key is the root's derivation"
    );
    assert_eq!(relink_journal(&database).len(), 1, "the act journaled once");
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_relink_to_the_rows_own_location_converges() {
    // D-208 point 3's idempotence rule: a re-send whose resolved root
    // equals the row's stored `path` AND `path_key` converges —
    // `Ok(())` with NOTHING written (no `updated_at` restamp, no
    // journal entry), the D-201 point-5/D-204 point-9 verified no-op.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let gone_path = temp.path().join("gone-repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &gone_path, "/gone-key");
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    let webview = main_webview(&app);
    invoke_project_relink(&webview, &project.id, &picked).expect("the first relink succeeds");
    let after_first = stored_project(&database, &project.id);
    assert_eq!(after_first.path, picked);
    let journal_after_first = journal_of(&database).len();

    invoke_project_relink(&webview, &project.id, &picked).expect("the re-send converges with Ok");

    let after_second = stored_project(&database, &project.id);
    assert_eq!(
        after_second, after_first,
        "the re-send restamps and writes nothing"
    );
    assert_eq!(
        journal_of(&database).len(),
        journal_after_first,
        "the re-send journals nothing"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_path_key_collision_with_an_active_project_is_refused() {
    // DB-015's active half (D-208 point 5): the resolved key belongs
    // to another ACTIVE project — the typed `E_VALIDATION` refusal
    // naming `path`, the caller's path echoed bounded, a class-only
    // reason, and nothing written (the mover's and the occupier's
    // rows both verbatim, the journal empty).
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let mover = seed_project(
        &database,
        AddedVia::Scan,
        &old_repo.display().to_string(),
        "/mover-key",
    );
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let new_key = eunomia_fs::paths::canonical_path_key(&new_repo)
        .expect("the freshly created repository folder must canonicalize");
    let occupier = seed_project(&database, AddedVia::Manual, "/elsewhere/occupier", &new_key);
    let picked = pick_folder(&app, &new_repo);
    let before = stored_project(&database, &mover.id);
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, &mover.id, &picked)
        .expect_err("a key an active project holds must be refused");

    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("path"),
        "{error}"
    );
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(picked),
        "the caller's path echoes bounded (short here, verbatim)"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("an active project already has this path (DB-015)"),
        "{error}"
    );
    assert!(
        error["params"].get("existing_project_id").is_none(),
        "the active arm carries no offer id: {error}"
    );
    // Nothing was written: both rows verbatim, no journal entry.
    assert_eq!(
        stored_project(&database, &mover.id),
        before,
        "the refused relink left the mover verbatim"
    );
    assert_eq!(
        stored_project(&database, &occupier.id).path_key,
        new_key,
        "the refused relink left the occupier verbatim"
    );
    assert_eq!(
        journal_of(&database),
        Vec::new(),
        "the whole journal claims nothing"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_path_key_collision_with_an_archived_project_carries_the_restore_offer() {
    // DB-015's archived half (D-208 point 5 — the pinned cross-link's
    // territory, consumed): the resolved key belongs to an ARCHIVED
    // project. The refusal is the same typed envelope naming `path`,
    // but it carries `existing_project_id` — the archived row's id,
    // exactly what `project_restore` (T-109/D-205, landed) consumes;
    // the UI cannot find the row itself (`projects_list` serves no
    // path column). Nothing is written.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let mover = seed_project(
        &database,
        AddedVia::Scan,
        &old_repo.display().to_string(),
        "/mover-key",
    );
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let new_key = eunomia_fs::paths::canonical_path_key(&new_repo)
        .expect("the freshly created repository folder must canonicalize");
    let occupier = seed_project(
        &database,
        AddedVia::Manual,
        "/elsewhere/archived-occupier",
        &new_key,
    );
    {
        let occupier_id = occupier.id.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::projects::archive(tx, &occupier_id)
                .map_err(crate::commands::projects_store_to_app_error)
                .map(|_| ())
        }))
        .expect("archiving the occupier through the store must work");
    }
    let picked = pick_folder(&app, &new_repo);
    let before = stored_project(&database, &mover.id);
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, &mover.id, &picked)
        .expect_err("a key an archived project holds must be refused");

    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("an archived project already has this path (DB-015)"),
        "{error}"
    );
    assert_eq!(
        error["params"]["existing_project_id"],
        serde_json::json!(occupier.id),
        "the DB-015 offer shape: the id `project_restore` consumes: {error}"
    );
    // Nothing was written: the mover verbatim, the occupier still
    // archived, no journal entry.
    assert_eq!(
        stored_project(&database, &mover.id),
        before,
        "the refused relink left the mover verbatim"
    );
    assert_eq!(
        stored_project(&database, &occupier.id).state,
        ProjectState::Archived,
        "the occupier stays archived (the offer stands)"
    );
    assert_eq!(journal_of(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_folder_outside_any_repository_is_refused_with_not_a_repo() {
    // D-09's `.git` check: a picked folder that is not inside any
    // repository answers §10's `E_NOT_A_REPO` ("pick the repository
    // root") — the walk probed every ancestor and none has `.git`.
    // Nothing is written.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &old_repo.display().to_string(),
        "/old-key",
    );
    let plain = temp.path().join("plain-folder");
    std::fs::create_dir_all(&plain).expect("the plain folder creates");
    let picked = pick_folder(&app, &plain);
    let before = stored_project(&database, &project.id);
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, &project.id, &picked)
        .expect_err("a folder without .git must be refused");

    assert_eq!(error["code"], serde_json::json!("E_NOT_A_REPO"), "{error}");
    assert_eq!(
        error["params"]["path"],
        serde_json::json!(picked),
        "the probed path travels as the param: {error}"
    );
    assert_eq!(
        stored_project(&database, &project.id),
        before,
        "the refused relink left the row verbatim"
    );
    assert_eq!(journal_of(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_foreign_path_is_refused_by_the_session_gate() {
    // SEC-003 (D-208 point 1's second gate): a path that neither
    // canonicalizes onto a registered project nor onto a folder
    // picked in this session is rejected before the `.git` check, the
    // database, or the disk probes — the add flow's own input gate.
    // A dead new path honestly meets the same arm (the sent path
    // cannot canonicalize, D-100's recorded cost), so this one pin
    // covers both spellings.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &old_repo.display().to_string(),
        "/old-key",
    );
    let before = stored_project(&database, &project.id);
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, &project.id, "/definitely/never/picked")
        .expect_err("a foreign path must be refused");

    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("path"),
        "{error}"
    );
    assert!(
        error["params"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("SEC-003")),
        "the session gate's own sentence: {error}"
    );
    assert_eq!(
        stored_project(&database, &project.id),
        before,
        "the refused relink left the row verbatim"
    );
    assert_eq!(journal_of(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_relink_is_refused_during_a_sync_before_anything_runs() {
    // SYS-005 (the family's own line): while a sync holds the global
    // flag, the relink answers the typed `E_SYNC_RUNNING` envelope —
    // asserted against a spec-literal JSON, not the constructor's own
    // output, so the mapping cannot drift from the wire form — and
    // nothing is written. The refusal is sent with a FOREIGN path
    // precisely so the gate's ORDER is pinned: the mode gate runs
    // before the SEC-003 input gate, so during the sync even a path
    // the session never picked meets the mode answer first (the add
    // flow's own posture). After the run ends, the same command over
    // the picked path converges normally.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let gone_path = temp.path().join("gone-repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &gone_path, "/gone-key");
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    let webview = main_webview(&app);

    let guard = crate::commands::begin_sync(app.app_handle())
        .expect("the test's flag must be free to take");
    let error = invoke_project_relink(&webview, &project.id, "/definitely/never/picked")
        .expect_err("the relink must be refused during a sync");
    assert_eq!(
        error,
        serde_json::json!({
            "code": "E_SYNC_RUNNING",
            "message_key": "errors.sync_running",
            "params": {},
            "details": null,
        }),
        "the §10 refusal envelope — the mode answer precedes the input gate"
    );
    assert_eq!(
        stored_project(&database, &project.id).path,
        gone_path,
        "no row was written while the flag was up"
    );
    assert_eq!(
        journal_of(&database),
        Vec::new(),
        "no journal entry was written"
    );

    drop(guard);
    invoke_project_relink(&webview, &project.id, &picked)
        .expect("the retried relink after the run ended succeeds");
    assert_eq!(stored_project(&database, &project.id).path, picked);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_relink_answers_honestly_without_a_database() {
    // Recovery mode (the `activity_list` pin): no `Db` managed — the
    // recorded D-037 fallback with the `NoDatabase` diagnostic, never
    // a pretend success that would lie about the projects table. The
    // gate states recovery mode also manages, so the answer comes
    // from `managed_db`, not a missing-state fallback; the picked
    // path passes the SEC-003 session half first (it needs no
    // database), so the answer is the database's own.
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_relink::project_relink
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::sync_running::SyncRunning::new());
    app.manage(crate::session_paths::SessionPaths::new());
    let (watch_state, _commands) =
        crate::file_watch::FileWatchState::with_budget(DEGRADED_REFRESH_BUDGET);
    app.manage(watch_state);
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, "any-id", &picked)
        .expect_err("the relink needs the database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest state answer"
    );
    drop(app);
    drop(temp);
}

#[test]
fn project_relink_refuses_an_unknown_id_with_the_typed_envelope() {
    // A stale screen's re-send: the id matches no row — the typed
    // `E_VALIDATION` refusal naming `project_id`, with the bounded
    // echo (D-115's cap: a webview string is unbounded by
    // definition), and nothing written. The world starts LOADED (an
    // active neighbour with its journal entry) precisely so the
    // "nothing written" half can fail: an implementation that touched
    // anything would move one of them.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let neighbour_repo = repo_with_agents_md(temp.path(), "neighbour-repo");
    let neighbour = seed_project(
        &database,
        AddedVia::Manual,
        &neighbour_repo.display().to_string(),
        "/neighbour-key",
    );
    {
        let neighbour_id = neighbour.id.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let mut entry = LogEntry::new(
                Action::ProjectChanged,
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
        .expect("seeding the neighbour journal entry must work");
    }
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    let neighbour_before = stored_project(&database, &neighbour.id);
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, "no-such-row", &picked)
        .expect_err("an unknown id must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("project_id"),
        "{error}"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("no active project has this id"),
        "{error}"
    );
    // D-115's cap, pinned at the boundary: an over-long id echoes
    // truncated with the suffix, never whole.
    let long_id = "x".repeat(70);
    let error = invoke_project_relink(&webview, &long_id, &picked)
        .expect_err("an over-long unknown id must be refused the same way");
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(format!("{} (+6 more code points)", "x".repeat(64))),
        "{error}"
    );
    // Nothing was written: the loaded world stands byte-for-byte.
    assert_eq!(
        stored_project(&database, &neighbour.id),
        neighbour_before,
        "the neighbour's row was not touched"
    );
    assert_eq!(
        journal_of(&database).len(),
        1,
        "the neighbour's journal entry was not touched"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_relink_refuses_an_archived_id_as_the_stale_screen_shape() {
    // D-208 point 3's id filter: the Locate action rides an ACTIVE
    // matrix column (D-09/MAIN-028; the archived screen's own action
    // is Restore), so an id whose row archived since the column
    // rendered is the `project_update` stale-screen refusal — "no
    // active project has this id" — and the row stays archived.
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &old_repo.display().to_string(),
        "/old-key",
    );
    {
        let project_id = project.id.clone();
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            eunomia_store::projects::archive(tx, &project_id)
                .map_err(crate::commands::projects_store_to_app_error)
                .map(|_| ())
        }))
        .expect("archiving through the store must work");
    }
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, &project.id, &picked)
        .expect_err("an archived id must be refused");

    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("project_id"),
        "{error}"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("no active project has this id"),
        "{error}"
    );
    assert_eq!(
        stored_project(&database, &project.id).state,
        ProjectState::Archived,
        "the row stays archived"
    );
    assert_eq!(
        stored_project(&database, &project.id).path,
        old_repo.display().to_string(),
        "the path did not move"
    );
    assert_eq!(journal_of(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_failed_journal_insert_rolls_back_the_path_transition() {
    // D-051's in-transaction semantics, proven through the real
    // command (the family's trigger shape): the path transition and
    // the journal entry share ONE `Db::run` transaction, so a failure
    // AFTER the transition succeeded — the journal insert, staged to
    // abort by a trigger — rolls all of it back: the row stays at its
    // old path and key, and the journal claims nothing. The failure
    // is staged deterministically inside the database (RAISE(ABORT)
    // → SQLITE_CONSTRAINT → the shared base classifier's
    // `E_VALIDATION` with the SQLite facts in details).
    let (temp, app) = project_relink_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &old_repo.display().to_string(),
        "/old-key",
    );
    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_every_journal_entry BEFORE INSERT ON action_log
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");
    let webview = main_webview(&app);

    let error = invoke_project_relink(&webview, &project.id, &picked)
        .expect_err("the staged abort must fail the relink");
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

    // The rollback: no partial state anywhere — the row's identity
    // and the journal all stand as they were.
    let stored = stored_project(&database, &project.id);
    assert_eq!(
        stored.path,
        old_repo.display().to_string(),
        "the path transition rolled back"
    );
    assert_eq!(stored.path_key, "/old-key", "the key rolled back");
    assert_eq!(
        relink_journal(&database),
        Vec::new(),
        "no entry claimed a relink"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_relinked_project_is_available_again_through_the_real_probe() {
    // The DoD's first half ("the project is available again after the
    // path change"), pinned the family way — through the REAL probe
    // surfaces, never a written verdict (D-208 point 7): the world
    // starts in D-09's situation (the row's folder gone), the first
    // `recheck_and_emit` pass records the unavailable verdict and
    // emits the D-09 diff, then the Locate act lands, and the next
    // pass answers the RECOVERY change (available, the reason gone)
    // while `projects_list`'s column reads available over the new
    // path (its fresh per-read D-084 probe).
    let (temp, app) = project_relink_verdicts_app();
    let database = database_of(&app);
    let gone_path = temp.path().join("gone-repo").display().to_string();
    let project = seed_project(&database, AddedVia::Scan, &gone_path, "/gone-key");
    let webview = main_webview(&app);
    // BEFORE: the column carries D-09's marking over the dead path.
    let before = column_entry_of(&webview, &project.id);
    assert!(!before.available, "the moved-folder world starts marked");
    assert_eq!(
        before.unavailable_reason.as_deref(),
        Some("missing"),
        "D-09's reason text half"
    );
    let first =
        tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.app_handle()))
            .expect("the first availability pass must succeed");
    assert_eq!(
        first,
        vec![crate::availability::ProjectAvailabilityChange {
            project_id: project.id.clone(),
            available: false,
            unavailable_reason: Some("missing".to_owned()),
        }],
        "the session's first probe reports the unavailable project"
    );
    // The Locate act.
    let new_repo = repo_with_agents_md(temp.path(), "located-repo");
    let picked = pick_folder(&app, &new_repo);
    invoke_project_relink(&webview, &project.id, &picked).expect("the locate act succeeds");

    // AFTER: the recovery change through the real recheck, and the
    // fresh column over the new path.
    let second =
        tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.app_handle()))
            .expect("the post-relink availability pass must succeed");
    assert_eq!(
        second,
        vec![crate::availability::ProjectAvailabilityChange {
            project_id: project.id.clone(),
            available: true,
            unavailable_reason: None,
        }],
        "the project is available again — the recovery direction is a change"
    );
    let after = column_entry_of(&webview, &project.id);
    assert!(after.available, "the DoD: the column reads available");
    assert_eq!(
        after.unavailable_reason, None,
        "the marking's text half is gone"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_relinked_project_is_dirty_exactly_as_the_service_decides() {
    // D-208 point 4's row-decidable dirty posture, pinned the T-107
    // way: the command writes no hash (the stored cell is the
    // pre-relink fact), and the verdict is the DB-022 service's own —
    // DB-020's formula covers the TARGET PATH, so the recomputation
    // over the new root's target differs from the stored hash and
    // answers dirty. The control: the same world answers clean before
    // the relink (the stored hash still covers the old target's
    // build). Both verdicts are read back through `projects_list`'s
    // column, the read the UI itself sees.
    let (temp, app) = project_relink_verdicts_app();
    let database = database_of(&app);
    let old_repo = repo_with_agents_md(temp.path(), "old-repo");
    let old_target = old_repo.join("AGENTS.md");
    let project = seed_project(
        &database,
        AddedVia::Manual,
        &old_repo.display().to_string(),
        "/old-key",
    );
    let (instruction, main) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    seed_last_synced_hash(
        &database,
        &project.id,
        &synced_hash_of(&old_target, "Be brief."),
    );
    let webview = main_webview(&app);
    // The independent baseline: the pass over the unmodified world
    // diffs nothing, or every verdict below would prove nothing.
    let baseline =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the baseline pass must succeed");
    assert_eq!(
        baseline,
        Vec::new(),
        "the world starts clean — the control is the oracle's own"
    );
    assert!(
        !column_entry_of(&webview, &project.id).dirty,
        "the column reads clean before the relink"
    );

    let new_repo = repo_with_agents_md(temp.path(), "new-repo");
    let picked = pick_folder(&app, &new_repo);
    invoke_project_relink(&webview, &project.id, &picked).expect("the relink succeeds");

    // The verdict through the real service: the target path moved,
    // so the recomputed hash differs from the stored one.
    let verdicts =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the post-relink pass must succeed");
    assert_eq!(
        verdicts,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: project.id.clone(),
            dirty: true,
        }],
        "the service — not the command — decides: the moved target path answers dirty"
    );
    assert!(
        column_entry_of(&webview, &project.id).dirty,
        "the column carries the unsynced dot"
    );
    // The command wrote no hash: the cell still holds the seeded
    // pre-relink value.
    assert_eq!(
        stored_project(&database, &project.id)
            .last_synced_hash
            .as_deref(),
        Some(synced_hash_of(&old_target, "Be brief.").as_str()),
        "OUT-020: the hash cell keeps the pre-relink value (the dirty \
         verdict is the service's, never a written cell)"
    );
    // The DoD's second half rides the same world: the assignments
    // survived the path change verbatim.
    assert_eq!(
        assignments_of(&database, &project.id).len(),
        1,
        "the assignments survive the path change"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_relink_runs_one_awaited_watch_pass_through_the_mutation_seam() {
    // D-208 point 6, the wiring's proof (SYS-007/D-144, T-279's LAST
    // arm): the committed relink — the row's root actually moved, and
    // the root is a watch input (`ProjectFact` derives the watched
    // file from it) — awaits ONE full watch pass through the bounded
    // mutation seam, the same refresh path (command, ack, service
    // pass) `project_add_manual`'s committed arm, T-107's arm and
    // T-109's arm run. The deterministic observable is the pass
    // counter, and the awaited seam means the count is settled when
    // the invoke returns. The pass reads target files and writes
    // nothing (OUT-011).
    let (temp, app, commands) = project_relink_watch_service_app();
    let repo_a = repo_with_agents_md(temp.path(), "repo-a");
    let repo_b = repo_with_agents_md(temp.path(), "repo-b");
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &repo_a.display().to_string(),
        "/repo-a-key",
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
    let settled = crate::ipc_tests::converge_startup_watch(&app, &repo_a.join("AGENTS.md"));
    let webview = main_webview(&app);
    let picked = pick_folder(&app, &repo_b);

    invoke_project_relink(&webview, &project.id, &picked)
        .expect("the relink succeeds — the refresh is never-fatal aftermath");

    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled + 1,
        "the awaited seam ran exactly one full pass after the settled startup baseline"
    );
    assert_eq!(
        stored_project(&database, &project.id).path,
        picked,
        "the row the pass covered points at the new root"
    );
    // Display-only even through the mutation's refresh: the pass
    // reads target files, it never writes them (OUT-011).
    assert_eq!(
        std::fs::read(repo_b.join("AGENTS.md")).expect("the file must be readable"),
        b"hand-written content\n".to_vec(),
        "the watch pass writes no target file"
    );
    // Windows teardown rule.
    crate::ipc_tests::stop_watch_service(app.handle());
    drop(app);
    drop(temp);
}

#[test]
fn a_converging_relink_resend_runs_no_watch_pass() {
    // The wiring's negative half (D-208 point 6): a re-send that
    // converges (the row already holds the resolved location) runs NO
    // pass — a pass over a row that did not move is file reads with
    // no observable change (D-201 point 7's own boundary).
    // Deterministic without any sleep: the seam is AWAITED inside
    // the command, so nothing is left in flight when the invoke
    // returns — the count cannot move after the assert (no service
    // trigger runs, and no file event fires over untouched repos).
    let (temp, app, commands) = project_relink_watch_service_app();
    let repo_a = repo_with_agents_md(temp.path(), "repo-a");
    let repo_b = repo_with_agents_md(temp.path(), "repo-b");
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &repo_a.display().to_string(),
        "/repo-a-key",
    );
    crate::file_watch::spawn_service(
        app.handle().clone(),
        commands,
        crate::ipc_tests::WATCH_DEBOUNCE,
    )
    .expect("the watcher must start");
    let settled = crate::ipc_tests::converge_startup_watch(&app, &repo_a.join("AGENTS.md"));
    let webview = main_webview(&app);
    let picked = pick_folder(&app, &repo_b);
    invoke_project_relink(&webview, &project.id, &picked).expect("the first relink runs the pass");
    let after_first = app
        .state::<crate::file_watch::FileWatchState>()
        .pass_count();
    assert_eq!(after_first, settled + 1, "the committed arm ran one pass");

    invoke_project_relink(&webview, &project.id, &picked).expect("the re-send converges with Ok");

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
fn a_refused_relink_runs_no_watch_pass() {
    // The wiring's third arm (round 1's P3-4 — the acceptance
    // criterion names committed/converging/refused, and only the
    // first two were pinned): a REFUSED relink runs NO pass. The
    // DB-015 collision is the arm that reaches deepest — the probe
    // passed, the transaction opened, the row read — so a refactor
    // that moved the refresh ahead of the transaction's `?` would
    // redden exactly here (the structural early-return alone cannot
    // be the guarantee). Deterministic without any sleep: the seam is
    // AWAITED inside the command, so nothing is left in flight when
    // the invoke returns.
    let (temp, app, commands) = project_relink_watch_service_app();
    let repo_a = repo_with_agents_md(temp.path(), "repo-a");
    let repo_b = repo_with_agents_md(temp.path(), "repo-b");
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Scan,
        &repo_a.display().to_string(),
        "/repo-a-key",
    );
    let repo_b_key = eunomia_fs::paths::canonical_path_key(&repo_b)
        .expect("the freshly created repository folder must canonicalize");
    let occupier = seed_project(
        &database,
        AddedVia::Manual,
        "/elsewhere/occupier",
        &repo_b_key,
    );
    crate::file_watch::spawn_service(
        app.handle().clone(),
        commands,
        crate::ipc_tests::WATCH_DEBOUNCE,
    )
    .expect("the watcher must start");
    let settled = crate::ipc_tests::converge_startup_watch(&app, &repo_a.join("AGENTS.md"));
    let webview = main_webview(&app);
    let picked = pick_folder(&app, &repo_b);

    let error = invoke_project_relink(&webview, &project.id, &picked)
        .expect_err("the occupied key must be refused");
    assert_eq!(
        error["code"],
        serde_json::json!("E_VALIDATION"),
        "the DB-015 collision refusal: {error}"
    );

    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled,
        "the refused arm runs no watch pass"
    );
    assert_eq!(
        stored_project(&database, &project.id).path,
        repo_a.display().to_string(),
        "the refused relink moved nothing"
    );
    assert_eq!(
        stored_project(&database, &occupier.id).path_key,
        repo_b_key,
        "the occupier stands"
    );
    // Windows teardown rule.
    crate::ipc_tests::stop_watch_service(app.handle());
    drop(app);
    drop(temp);
}

#[test]
fn the_project_relink_command_is_registered_once_with_specta() {
    // The §7.10 Projects-module name (D-009's fixed registry),
    // registered exactly once (the D-034 wiring-pin rationale) — the
    // bindings carry `projectRelink` to the frontend through it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("project_relink::project_relink::<").count(),
        1,
        "lib.rs registers `project_relink` once"
    );
}
