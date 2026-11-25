//! Mock-IPC behavior tests for the `project_update` command (T-106,
//! PRJ-008/009/011/021, §6.9, SYS-005, LOG-001…004, D-201; extended
//! by T-107, PRJ-012, DB-023, §7.5, D-01, D-03, SYS-007, D-204).
//!
//! The command runs over a real migrated `Db` through the mock-IPC
//! invoke path; assertions read the store row and the journal back.
//! The card's DoDs lead: an invalid field saves NOTHING (row verbatim,
//! journal absent), and the path/origin columns cannot change (the
//! contract itself — asserted as the row staying verbatim in every
//! column the command does not own). The D-201 readings follow: patch
//! semantics, the saved-not-refused duplicate name, the verified
//! no-op, the stale-screen refusal for an unknown or archived id, the
//! `E_SYNC_RUNNING` gate, the honest no-database answer, and the
//! D-051 one-transaction rollback. The T-107 battery extends every
//! one of those arms to the write-configuration half (scheme, file
//! name, extra files) and adds DB-023's own two: the scheme or
//! file-name change dirties the project THROUGH THE REAL SERVICE
//! (`dirty_recalc::recalc_and_emit` — the row-decidable arm, D-204
//! point 5) while the old file on disk stays byte-untouched, and the
//! committed watch-input change awaits ONE full watch pass through
//! the bounded mutation seam (D-204 point 7) while a name-only or
//! no-op send runs none.

use eunomia_core::hash::build_hash;
use eunomia_store::action_log::{
    Action, EntityType, Level, LogRecord, Outcome, Visibility, recent,
};
use eunomia_store::library::{
    NewAssignment, NewLibraryItem, NewVariant, assign, create_item, create_variant,
};
use eunomia_store::projects::{AddedVia, NewProject, ProjectState};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use tauri::Manager;

/// The healthy update world: a fresh database, the SYS-005 gate's
/// flag state managed (recovery mode manages it too — the no-database
/// test below does the same), and the command registered.
fn project_update_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_update::project_update
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

/// Invokes `project_update` over the mock-IPC path with the T-106
/// field set — the write-configuration half sent as absent (`null`).
/// Kept so the T-106 battery's bodies stay verbatim; the full patch
/// helper below carries the T-107 half.
#[allow(clippy::too_many_arguments)]
fn invoke_project_update(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
    name: serde_json::Value,
    label: serde_json::Value,
    color: serde_json::Value,
    description: serde_json::Value,
) -> Result<(), serde_json::Value> {
    invoke_project_update_patch(
        webview,
        project_id,
        name,
        label,
        color,
        description,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
}

/// Invokes `project_update` over the mock-IPC path with the WHOLE
/// patch surface (D-204 point 1). Each optional field crosses as its
/// wire value — `null` (absent, the patch semantics), the sent
/// string, or the §6.5 extra-file object — so the tests exercise
/// exactly the payloads the webview would send.
#[allow(clippy::too_many_arguments)]
fn invoke_project_update_patch(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
    name: serde_json::Value,
    label: serde_json::Value,
    color: serde_json::Value,
    description: serde_json::Value,
    scheme: serde_json::Value,
    base_name: serde_json::Value,
    extra_local: serde_json::Value,
    extra_project: serde_json::Value,
) -> Result<(), serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "project_update",
            serde_json::json!({
                "projectId": project_id,
                "name": name,
                "label": label,
                "color": color,
                "description": description,
                "scheme": scheme,
                "baseName": base_name,
                "extraLocal": extra_local,
                "extraProject": extra_project,
            }),
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

/// One project row, the update flow's world. `Some("Update Me")` in the
/// seed keeps the name/label/color/description assertions below from
/// passing vacuously (the sent values differ from the stored ones).
fn seed_project(
    database: &crate::db::Db,
    added_via: AddedVia,
    path: &str,
    path_key: &str,
) -> eunomia_store::projects::Project {
    seed_project_with_scheme(database, added_via, path, path_key, Scheme::A)
}

/// One project row with a caller-chosen scheme — the T-107 battery
/// varies the write configuration (D-204 point 3's scheme-B seed) the
/// way the T-106 battery varied the origin. Everything else stays at
/// the §6.5 defaults the fixture world uses.
fn seed_project_with_scheme(
    database: &crate::db::Db,
    added_via: AddedVia,
    path: &str,
    path_key: &str,
    scheme: Scheme,
) -> eunomia_store::projects::Project {
    let path = path.to_owned();
    let path_key = path_key.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = NewProject {
            path: &path,
            path_key: &path_key,
            name: "Update Me",
            label: "UPD",
            color: "red",
            description: "the seeded description",
            added_via,
            scheme,
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

fn stored_project(database: &crate::db::Db, id: &str) -> eunomia_store::projects::Project {
    let id = id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::get(tx, &id).map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("project read succeeds")
    .expect("the seeded row exists")
}

fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(|tx| recent(tx, 1000).map_err(crate::commands::activity_log_to_app_error)),
    )
    .expect("journal reads")
}

fn changed_journal(database: &crate::db::Db) -> Vec<LogRecord> {
    journal_of(database)
        .into_iter()
        .filter(|entry| entry.action == Action::ProjectChanged)
        .collect()
}

/// The journal entry's attributable shape — one reusable assert so
/// every writing test pins the same LOG-001…004/D-201 contract.
fn assert_changed_entry(entry: &LogRecord, project_id: &str) {
    assert_eq!(entry.level, Level::Info);
    assert_eq!(entry.visibility, Visibility::User);
    assert_eq!(entry.result, Outcome::Ok);
    assert_eq!(
        entry.entity,
        Some((EntityType::Project, project_id.to_owned()))
    );
    assert_eq!(entry.project_id.as_deref(), Some(project_id));
    assert_eq!(entry.details, None, "the row itself holds the facts");
}

// ---- the T-107 harnesses (D-204) ---------------------------------------

/// The mutation-refresh budget of the non-wiring tests: tiny, so the
/// degraded pass (no service task is spawned there — the tests assert
/// the SAVE, not the pass) costs milliseconds. The wiring tests below
/// use the production budget with a real service.
const DEGRADED_REFRESH_BUDGET: std::time::Duration = std::time::Duration::from_millis(20);

/// The healthy update world WITH the file-watch state managed — every
/// test whose send commits a watch-input change (scheme, file name,
/// extras) reaches the refresh seam, and `app.state::<FileWatchState>`
/// panics when the state is missing (production manages it always, the
/// setup hook; D-144's own harness posture). No service task is
/// spawned here: the refresh degrades at the tiny budget and the save
/// stands (D-156's never-fatal contract — the very behavior the
/// non-wiring battery runs over).
fn project_update_watch_state_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_update::project_update
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

/// The DB-023 DoD world: the dirty service's cache and event mounted
/// (the T-108 surface — `recalc_and_emit` and `projects_list`'s row),
/// plus the watch state the committed scheme change reaches.
fn project_update_dirty_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_update::project_update,
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

/// One instruction with a `Main` variant (the dirty world needs an
/// assignment, or DB-021's never-synced-empty-build rule reads the
/// project clean whatever the configuration — a vacuous oracle).
fn seed_instruction(
    database: &crate::db::Db,
) -> (
    eunomia_store::library::LibraryItem,
    eunomia_store::library::Variant,
) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item = create_item(tx, &NewLibraryItem::Instruction { title: "I1" })
            .map_err(seed_library_error)?;
        let main = create_variant(
            tx,
            &NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: "Be brief.",
                is_main: true,
            },
        )
        .map_err(seed_library_error)?;
        Ok((item, main))
    }))
    .expect("instruction seed creates")
}

/// The shared library-seed error route (the `library_seed_error_for_tests`
/// shape: SQL through the production classifier, the rest a loud
/// unreachable-parameter stand-in).
fn seed_library_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::db::sqlite_error_to_app_error(match error {
        eunomia_store::library::LibraryError::Sql(source) => source,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })
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
        .map_err(seed_library_error)
    }))
    .expect("assignment seed creates")
}

/// Stamps the row's `last_synced_hash` (the DB-024 cell a sync writes)
/// — the clean-baseline fixture.
fn simulate_sync(database: &crate::db::Db, project_id: &str, hash: &str) {
    let project_id = project_id.to_owned();
    let hash = hash.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE projects SET last_synced_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, project_id],
        )
        .map(|_| ())
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("storing the synced hash must work");
}

/// The DB-024 hash of the plain one-block body over the fixture
/// project's write configuration — the value a sync of that build
/// stores, spelled directly through the public core seam (never
/// through the code under test; the `dirty_recalc_tests` fixture's
/// own shape). With both extra files off the managed block IS the
/// body (ALG-030 joins no reading lines around it).
fn synced_hash_of(scheme: Scheme, path: &std::path::Path, body: &str) -> String {
    build_hash(
        core_scheme(scheme),
        path,
        core_extra_off(),
        core_extra_off(),
        body,
    )
}

/// The core twin of the fixture scheme (the hash's own vocabulary).
fn core_scheme(scheme: Scheme) -> eunomia_core::project::Scheme {
    match scheme {
        Scheme::A => eunomia_core::project::Scheme::A,
        Scheme::B => eunomia_core::project::Scheme::B,
        Scheme::C => eunomia_core::project::Scheme::C,
    }
}

/// The extras-off option of the fixture settings, in core's shape.
fn core_extra_off() -> eunomia_core::project::ExtraFile {
    eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    }
}

/// One repository directory with a `.git` entry (the pass's world) and
/// a hand-written target file with KNOWN bytes (DB-023's disk oracle:
/// whatever the command does, these bytes must survive it).
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
    let entry = columns
        .iter()
        .find(|column| column.id == project_id)
        .expect("the project's column is served");
    entry.dirty
}

#[test]
fn project_update_saves_all_four_fields_in_one_transaction_and_journals_one_entry() {
    // PRJ-008's happy path (D-201 point 1): every editable field at its
    // §6.9 INCLUSIVE boundary — a 100-code-point name, a 6-character
    // label, a 1000-character description, a palette color — saved by
    // one call, with the columns the command does not own verbatim
    // (PRJ-008's "the path cannot be changed" AC, D-201 point 3) and
    // exactly one attributable `project.changed` entry (LOG-001).
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("x".repeat(100)),
        serde_json::json!("L".repeat(6)),
        serde_json::json!("teal"),
        serde_json::json!("d".repeat(1000)),
    )
    .expect("saving four legal fields succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.name, "x".repeat(100), "§6.9's inclusive name edge");
    assert_eq!(stored.label, "L".repeat(6), "§6.9's inclusive label edge");
    assert_eq!(stored.color, "teal", "a palette member");
    assert_eq!(
        stored.description,
        "d".repeat(1000),
        "§6.9's inclusive description edge"
    );
    // The immutable half (PRJ-008): the identity, origin, state,
    // position and sync facts are the columns this command never
    // touches — verbatim after the save.
    assert_eq!(stored.path, project.path, "the path cannot be changed");
    assert_eq!(
        stored.path_key, project.path_key,
        "the identity key is fixed"
    );
    assert_eq!(stored.added_via, project.added_via, "the origin is fixed");
    assert_eq!(stored.state, ProjectState::Active);
    assert_eq!(stored.sort_order, project.sort_order);
    assert_eq!(stored.created_at, project.created_at);
    assert_eq!(
        stored.last_synced_hash, project.last_synced_hash,
        "OUT-020's fact stays the sync's own"
    );
    // LOG-001…004/D-201 point 5: ONE attributable entry.
    let journal = changed_journal(&database);
    assert_eq!(journal.len(), 1, "exactly one project.changed entry");
    assert_changed_entry(&journal[0], &project.id);
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn an_invalid_name_saves_nothing() {
    // The card's DoD: an invalid field refuses the whole operation —
    // §6.9's "an entity is never saved partly" — so the row stays
    // verbatim and the journal claims nothing. 101 code points is the
    // name limit's far edge.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("x".repeat(101)),
        serde_json::json!("L".repeat(6)),
        serde_json::json!("teal"),
        serde_json::json!("new description"),
    )
    .expect_err("a 101-code-point name must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("name"),
        "{error}"
    );
    // The name echoes BOUNDED (D-115's cap): 101 code points come back
    // as 64 plus the count (round 1's P3-6 — the echo rule itself is
    // pinned, not just the field name).
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(format!("{} (+37 more code points)", "x".repeat(64))),
        "{error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "nothing was journaled"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_invalid_label_saves_nothing() {
    // §6.9's label edge: 7 characters is one past the limit.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("New Name"),
        serde_json::json!("ABCDEFG"),
        serde_json::json!("teal"),
        serde_json::json!("new description"),
    )
    .expect_err("a 7-character label must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("label"),
        "{error}"
    );
    // The short label echoes verbatim (D-115's cap never reaches it).
    assert_eq!(
        error["params"]["value"],
        serde_json::json!("ABCDEFG"),
        "{error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_invalid_description_saves_nothing() {
    // §6.9's description edge: 1001 characters is one past the limit.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("New Name"),
        serde_json::json!("NEWL"),
        serde_json::json!("teal"),
        serde_json::json!("d".repeat(1001)),
    )
    .expect_err("a 1001-character description must be refused");
    assert_eq!(
        error["code"],
        serde_json::json!("E_VALIDATION"),
        "the whole operation is refused: {error}"
    );
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("description"),
        "{error}"
    );
    // SEC-008's no-echo rule for stored prose (round 1's P3-6): the
    // description is the one field whose refusal carries NO value —
    // a refactor that "helpfully" echoes it would leak the caller's
    // text onto the envelope and the §10 log line.
    assert!(
        error["params"].get("value").is_none(),
        "the description refusal echoes no value: {error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_off_palette_color_saves_nothing() {
    // §6.9's color rule: only the DB-040 palette members pass.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("New Name"),
        serde_json::json!("NEWL"),
        serde_json::json!("purple"),
        serde_json::json!("new description"),
    )
    .expect_err("an off-palette color must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("color"),
        "{error}"
    );
    // A short off-palette color echoes verbatim (a closed-vocabulary
    // word, like the activity_list action filter's echo).
    assert_eq!(
        error["params"]["value"],
        serde_json::json!("purple"),
        "{error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_duplicate_name_is_saved_without_refusal() {
    // D-201 point 2: §6.9's name row — "Duplicates are allowed with a
    // warning" — the warning is the modal's live field validation
    // (T-160), never a store refusal, so a name another row already
    // holds saves exactly like a unique one.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let first = seed_project(
        &database,
        AddedVia::Manual,
        &temp.path().join("first").display().to_string(),
        "/first",
    );
    let second = seed_project(
        &database,
        AddedVia::Manual,
        &temp.path().join("second").display().to_string(),
        "/second",
    );
    let webview = main_webview(&app);

    // The second row first takes a name of its own (a real change), so
    // the duplicate send below is a CHANGE for its row — not the
    // verified no-op a same-value re-send would be.
    invoke_project_update(
        &webview,
        &second.id,
        serde_json::json!("Unique Two"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the first rename succeeds");

    invoke_project_update(
        &webview,
        &second.id,
        serde_json::json!(first.name.clone()),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("a duplicate name is allowed (6.9)");

    let stored = stored_project(&database, &second.id);
    assert_eq!(stored.name, first.name, "the duplicate name is stored");
    // The other rows are untouched by the save.
    assert_eq!(stored_project(&database, &first.id), first);
    let journal = changed_journal(&database);
    assert_eq!(journal.len(), 2, "both real changes journaled");
    assert_changed_entry(&journal[1], &second.id);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn absent_fields_keep_their_stored_values() {
    // D-201 point 1's patch half: `null` fields are unchanged — the
    // modal's "Save" sends every field, but a stale screen that
    // re-sends one field cannot wipe the other three.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    invoke_project_update(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::json!("ONLY"),
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("a one-field patch succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.label, "ONLY", "the present field is written");
    assert_eq!(stored.name, project.name, "the absent name is unchanged");
    assert_eq!(stored.color, project.color, "the absent color is unchanged");
    assert_eq!(
        stored.description, project.description,
        "the absent description is unchanged"
    );
    let journal = changed_journal(&database);
    assert_eq!(journal.len(), 1);
    assert_changed_entry(&journal[0], &project.id);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_unchanged_resend_is_a_verified_noop() {
    // D-201 point 5's no-op half (the D-188 precedent): a re-send of
    // the stored values writes and journals nothing — the journal is
    // the primary oracle (a regression that passes the fields through
    // to the store journals a change), `updated_at` the secondary one.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!(project.name.clone()),
        serde_json::json!(project.label.clone()),
        serde_json::json!(project.color.clone()),
        serde_json::json!(project.description.clone()),
    )
    .expect("the unchanged re-send converges with Ok");

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "the verified no-op: not even updated_at churned"
    );
    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "the re-send journals nothing"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_description_with_a_nul_is_saved_stripped() {
    // §6.9's description exception, its own literal: "a `NUL` in it is
    // silently stripped before the length is checked — the user's
    // visible text is kept, that invisible byte is not". The store's
    // column therefore never holds one.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    invoke_project_update(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("kept text\0stripped byte"),
    )
    .expect("a description with a NUL is legal (6.9)");

    assert_eq!(
        stored_project(&database, &project.id).description,
        "kept textstripped byte",
        "the NUL is silently stripped"
    );
    let journal = changed_journal(&database);
    assert_eq!(journal.len(), 1);
    assert_changed_entry(&journal[0], &project.id);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_update_is_refused_during_a_sync_before_anything_runs() {
    // SYS-005 (the card's own line): while a sync holds the global
    // flag, the update answers the typed `E_SYNC_RUNNING` envelope —
    // asserted against a spec-literal JSON, not the constructor's own
    // output, so the mapping cannot drift from the wire form — and
    // nothing is written. After the run ends, the same save converges
    // normally.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let guard = crate::commands::begin_sync(app.app_handle())
        .expect("the test's flag must be free to take");
    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("During Sync"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the update must be refused during a sync");
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
        stored_project(&database, &project.id),
        project,
        "no row was written"
    );
    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "no journal entry was written"
    );

    // Round 1's P2-4: the gate precedes per-input validation, so the
    // refusal is the answer WHATEVER the input is — an INVALID name
    // during the run must meet the same envelope, not the field
    // refusal a validation-first ordering would answer.
    let gate_error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("x".repeat(101)),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the gate answers before the field validation runs");
    assert_eq!(
        gate_error,
        serde_json::json!({
            "code": "E_SYNC_RUNNING",
            "message_key": "errors.sync_running",
            "params": {},
            "details": null,
        }),
        "the §10 refusal envelope, invalid input included"
    );

    // T-107's arm of the same pin (D-204 point 7's ordering): the
    // gate also precedes the watch-input half — a scheme change is
    // a SYS-005-gated project mutation like every other, refused
    // before its refresh could run.
    let scheme_gate_error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("B"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the gate answers before the scheme arm runs");
    assert_eq!(
        scheme_gate_error,
        serde_json::json!({
            "code": "E_SYNC_RUNNING",
            "message_key": "errors.sync_running",
            "params": {},
            "details": null,
        }),
        "the §10 refusal envelope, the scheme change included"
    );

    drop(guard);
    invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("During Sync"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the retried save after the run ended succeeds");
    assert_eq!(stored_project(&database, &project.id).name, "During Sync");
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_invalid_field_is_answered_before_the_id_and_in_table_order() {
    // Round 1's P2-4: D-201 point 6's ordering claims, pinned — the
    // present fields validate BEFORE the row is read (a stale id plus
    // an invalid field answers about the FIELD, not the id), and the
    // §6.9 table order decides which invalid field answers (name is
    // first, color last). Both would drift silently otherwise.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        "no-such-row",
        serde_json::json!("x".repeat(101)),
        serde_json::Value::Null,
        serde_json::json!("purple"),
        serde_json::Value::Null,
    )
    .expect_err("the field validation precedes the row read");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("name"),
        "the name (table order's first field) answers, not the id or the color"
    );

    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "nothing was journaled"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_middle_fields_answer_in_table_order_too() {
    // Round 2's P3-3: the first two-fault pin covers the table's ENDS
    // (name first, color last); this one covers the MIDDLE pair — an
    // invalid label and an over-long description together answer
    // about the LABEL, so a label/description validator swap cannot
    // survive the battery either.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("New Name"),
        serde_json::json!("ABCDEFG"),
        serde_json::Value::Null,
        serde_json::json!("d".repeat(1001)),
    )
    .expect_err("the earlier field in table order answers");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("label"),
        "the label answers, not the description"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_empty_name_saves_nothing() {
    // §6.9's name lower bound: 1–100 characters AFTER trimming, so a
    // whitespace-only name is the empty refusal (the happy path pins
    // the 100 edge; this pins the 1 edge).
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("   "),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("a whitespace-only name must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("name"),
        "{error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_empty_description_is_a_legal_change() {
    // §6.9's description lower bound: "empty is legal" — sending
    // `Some("")` over a seeded non-empty description is a REAL change
    // (the modal's cleared text field), saved and journaled like any
    // other (not the no-op a null send would be).
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    invoke_project_update(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!(""),
    )
    .expect("clearing the description is a legal save");

    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.description, "", "the cleared description is stored");
    assert_eq!(stored.name, project.name, "the other fields are untouched");
    let journal = changed_journal(&database);
    assert_eq!(journal.len(), 1, "the real change is journaled");
    assert_changed_entry(&journal[0], &project.id);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_all_null_send_is_a_verified_noop() {
    // D-201 point 5's other no-op arm: every field `null` — an empty
    // patch. Nothing is written, nothing journaled (the store's own
    // empty-update rule, reached without a single field comparison).
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    invoke_project_update(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the all-null send converges with Ok");

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "the empty patch writes nothing"
    );
    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "the empty patch journals nothing"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_update_refuses_an_unknown_id_with_the_typed_envelope() {
    // A stale screen's re-send: the id matches no row — the typed
    // `E_VALIDATION` refusal naming `project_id`, with nothing
    // written. D-115's cap rides the echo: a webview string is
    // unbounded by definition.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        "no-such-row",
        serde_json::json!("New Name"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
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
    assert_eq!(
        error["params"]["value"],
        serde_json::json!("no-such-row"),
        "{error}"
    );

    // D-115's bounded echo, pinned at the boundary: an over-long id
    // echoes truncated with the suffix, never whole.
    let long_id = "x".repeat(70);
    let error = invoke_project_update(
        &webview,
        &long_id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("an over-long unknown id is refused the same way");
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(format!("{} (+6 more code points)", "x".repeat(64))),
        "{error}"
    );
    // T-107's arm: a VALID scheme change over an unknown id is the
    // same stale screen — the fields validate first (D-201 point 6's
    // ordering), then the row read refuses.
    let scheme_error = invoke_project_update_patch(
        &webview,
        "no-such-row",
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("B"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("a valid scheme over an unknown id must be refused");
    assert_eq!(
        scheme_error["params"]["field"],
        serde_json::json!("project_id"),
        "{scheme_error}"
    );
    assert_eq!(
        scheme_error["params"]["reason"],
        serde_json::json!("no active project has this id"),
        "{scheme_error}"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_archived_row_is_refused_as_a_stale_screen() {
    // D-201 point 4 (D-115's reorder precedent): the modal opens from
    // an ACTIVE column's pencil (PRJ-008), so an id naming an archived
    // row is the same stale-screen re-send as an unknown id — the
    // typed refusal naming `project_id`, nothing written even though
    // the store itself could update an archived row.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let id = project.id.clone();
    let database_for_archive = database.clone();
    tauri::async_runtime::block_on(database_for_archive.run(move |tx| {
        eunomia_store::projects::archive(tx, &id)
            .map_err(crate::commands::projects_store_to_app_error)
            .map(|_| ())
    }))
    .expect("seeding the archived row must work");
    let archived = stored_project(&database, &project.id);
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("New Name"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("an archived row must be refused as a stale screen");
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
        stored_project(&database, &project.id),
        archived,
        "the archived row is untouched"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn project_update_answers_honestly_without_a_database() {
    // Recovery mode (the `project_archive` pin): no `Db` managed — the
    // recorded D-037 fallback with the `NoDatabase` diagnostic, never a
    // pretend success that would lie about the projects table. The
    // SYS-005 gate state recovery mode also manages, so the answer
    // comes from `managed_db`; the fields validate first, so the test
    // sends legal ones.
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::project_update::project_update
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::sync_running::SyncRunning::new());
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        "any-id",
        serde_json::json!("New Name"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the update needs the database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest state answer"
    );
    // T-107's arm (D-204 point 7): the watch refresh is unreachable
    // without a database — `managed_db` answers before the commit
    // could ever reach the seam, so a scheme-carrying send meets the
    // same honest refusal, never a hang and never a pretend pass.
    let scheme_error = invoke_project_update_patch(
        &webview,
        "any-id",
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("B"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the scheme arm needs the database too");
    assert_eq!(
        scheme_error["code"],
        serde_json::json!("E_DB_LOCKED"),
        "{scheme_error}"
    );
    assert_eq!(
        scheme_error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest state answer, the watch-input arm included"
    );
    drop(app);
}

#[test]
fn a_failed_journal_insert_rolls_back_the_field_writes() {
    // D-051's in-transaction semantics, proven through the real
    // command (the `a_failed_journal_insert_rolls_back…` shape): the
    // field writes and the journal entry share ONE `Db::run`
    // transaction, so a failure AFTER the fields were written — the
    // journal insert, staged to abort by a trigger — rolls all of it
    // back: the row stays verbatim and the journal claims nothing.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_every_journal_entry BEFORE INSERT ON action_log
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");
    let webview = main_webview(&app);

    let error = invoke_project_update(
        &webview,
        &project.id,
        serde_json::json!("Rolled Back"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the staged abort must fail the update");
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

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "the field writes rolled back — not even updated_at churned"
    );
    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "no entry claimed a change"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_project_update_command_is_registered_once_with_specta() {
    // The §7.10 Projects-module name (D-009's fixed registry),
    // registered exactly once (the D-034 wiring-pin rationale) — the
    // bindings carry `projectUpdate` to the frontend through it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("project_update::project_update::<").count(),
        1,
        "lib.rs registers `project_update` once"
    );
}

// ---- the T-107 battery: the write-configuration half (D-204) -------

#[test]
fn the_write_configuration_saves_and_journals_one_entry() {
    // D-204's happy path: the whole PRJ-012 half saved by one call —
    // the scheme, the file name, and both extra-file options (one
    // enabled at start, one re-sent at its stored disabled value: the
    // patch's equal-strip on the new half, D-204 point 9) — with the
    // columns the command does not own verbatim (PRJ-008) and exactly
    // one attributable `project.changed` entry (LOG-001, D-204
    // point 8: the entry carries NO field set — the row holds the
    // facts).
    let (temp, app) = project_update_watch_state_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("B"),
        serde_json::json!("CLAUDE.md"),
        serde_json::json!({ "enabled": true, "position": "start" }),
        serde_json::json!({ "enabled": false, "position": "end" }),
    )
    .expect("saving the whole write configuration succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(
        stored.scheme,
        Scheme::B,
        "PRJ-012: the project's own scheme"
    );
    assert_eq!(stored.base_name, BaseName::ClaudeMd, "D-03's list member");
    assert_eq!(
        (stored.extra_local.enabled, stored.extra_local.position),
        (true, Position::Start),
        "the enabled extra file carries its position (§7.5 scheme A)"
    );
    assert_eq!(
        (stored.extra_project.enabled, stored.extra_project.position),
        (false, Position::End),
        "the re-sent stored value is stripped, not churned"
    );
    // The T-106 fields are absent — unchanged.
    assert_eq!(stored.name, project.name);
    assert_eq!(stored.label, project.label);
    assert_eq!(stored.color, project.color);
    assert_eq!(stored.description, project.description);
    // The immutable half (PRJ-008): identity, origin, state, position
    // and sync facts are the columns this command never touches.
    assert_eq!(stored.path, project.path, "the path cannot be changed");
    assert_eq!(
        stored.path_key, project.path_key,
        "the identity key is fixed"
    );
    assert_eq!(stored.added_via, project.added_via, "the origin is fixed");
    assert_eq!(stored.state, ProjectState::Active);
    assert_eq!(stored.sort_order, project.sort_order);
    assert_eq!(stored.created_at, project.created_at);
    assert_eq!(
        stored.last_synced_hash, project.last_synced_hash,
        "OUT-020's fact stays the sync's own"
    );
    // LOG-001…004/D-204 point 8: ONE attributable entry, no details.
    let journal = changed_journal(&database);
    assert_eq!(journal.len(), 1, "exactly one project.changed entry");
    assert_changed_entry(&journal[0], &project.id);
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_scheme_change_dirties_the_project_and_leaves_the_file_untouched() {
    // THE card DoD, scheme arm (DB-023): after a scheme change the
    // project is dirty AND the old file on disk is not touched. The
    // world starts CLEAN — the synced hash a real sync of that build
    // would store, spelled through the public core seam, never through
    // the code under test — so the dirty verdict after the act can
    // only come from the configuration change: DB-020's formula hashes
    // the scheme, and the real service's own pass (not the command)
    // decides (D-204 point 5 — the row-decidable arm, nothing written).
    let (temp, app) = project_update_dirty_app();
    let database = database_of(&app);
    let repo = repo_with_agents_md(temp.path());
    let project = seed_project(
        &database,
        AddedVia::Manual,
        &repo.display().to_string(),
        "/repo-key",
    );
    let (instruction, main) = seed_instruction(&database);
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let target = repo.join("AGENTS.md");
    let synced_hash = synced_hash_of(Scheme::A, &target, "Be brief.");
    simulate_sync(&database, &project.id, &synced_hash);
    let webview = main_webview(&app);

    // The independent baseline: the pass over the unmodified row
    // diffs nothing, or the dirty assert below would prove nothing.
    let clean =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the clean pass must succeed");
    assert_eq!(
        clean,
        Vec::new(),
        "the world starts clean — the baseline is the oracle's own control"
    );
    assert!(
        !dirty_column_of(&webview, &project.id),
        "the column reads clean before the act"
    );

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("B"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the scheme change succeeds");

    // DB-023's disk half: the old file is not touched, byte for byte.
    assert_eq!(
        std::fs::read(&target).expect("the old file must be readable"),
        b"hand-written content\n".to_vec(),
        "DB-023: the old file is not deleted, edited or recreated"
    );
    // DB-023's dirty half, through the real service (DB-021 over
    // DB-020): the hash now covers scheme B, the stored cell does not.
    let dirty =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the dirty pass must succeed");
    assert_eq!(
        dirty,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: project.id.clone(),
            dirty: true,
        }],
        "the scheme change makes the project dirty (DB-023 through DB-020/DB-021)"
    );
    assert!(
        dirty_column_of(&webview, &project.id),
        "the column the UI reads carries the unsynced dot"
    );
    // The row's own facts: the scheme changed, the sync facts did not
    // (OUT-020 — the next sync owns the hash cell).
    let stored = stored_project(&database, &project.id);
    assert_eq!(stored.scheme, Scheme::B, "the scheme is stored");
    assert_eq!(
        stored.last_synced_hash.as_deref(),
        Some(synced_hash.as_str()),
        "OUT-020: the hash cell keeps the pre-change value"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_base_name_change_dirties_the_project_and_leaves_the_old_file_untouched() {
    // THE card DoD, file-name arm (DB-023): a base-name change makes
    // the project dirty (DB-020 hashes the target path), the old file
    // under the previous name is not deleted, and the NEW name is not
    // created either — no file I/O happens in the command at all (the
    // next sync writes the new file, D-204 point 6).
    let (temp, app) = project_update_dirty_app();
    let database = database_of(&app);
    let repo = repo_with_agents_md(temp.path());
    let project = seed_project(
        &database,
        AddedVia::Manual,
        &repo.display().to_string(),
        "/repo-key",
    );
    let (instruction, main) = seed_instruction(&database);
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let old_target = repo.join("AGENTS.md");
    let synced_hash = synced_hash_of(Scheme::A, &old_target, "Be brief.");
    simulate_sync(&database, &project.id, &synced_hash);
    let webview = main_webview(&app);

    // The independent baseline (the scheme test's own control): the
    // pass over the unmodified row diffs nothing, or the dirty
    // assert below would prove nothing.
    let clean =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the clean pass must succeed");
    assert_eq!(
        clean,
        Vec::new(),
        "the world starts clean — the baseline is the oracle's own control"
    );

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("CLAUDE.md"),
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the file-name change succeeds");

    assert_eq!(
        std::fs::read(&old_target).expect("the old file must be readable"),
        b"hand-written content\n".to_vec(),
        "DB-023: the old file with the previous name is not deleted"
    );
    assert!(
        !repo.join("CLAUDE.md").exists(),
        "no file I/O: the new name is the next sync's to write"
    );
    let dirty =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the dirty pass must succeed");
    assert_eq!(
        dirty,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: project.id.clone(),
            dirty: true,
        }],
        "the file-name change makes the project dirty (the target path is a DB-020 input)"
    );
    assert_eq!(
        stored_project(&database, &project.id).base_name,
        BaseName::ClaudeMd,
        "the new file name is stored"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_extra_file_change_dirties_the_project_and_leaves_the_file_untouched() {
    // DB-023's hash arm through the extra files (DB-020 hashes the
    // extra-file flags and positions under every scheme — no scheme
    // qualifier): enabling `.local.md` at start changes the build
    // configuration AND the managed text (ALG-030's reading line),
    // so the project goes dirty; the disk is not touched.
    let (temp, app) = project_update_dirty_app();
    let database = database_of(&app);
    let repo = repo_with_agents_md(temp.path());
    let project = seed_project(
        &database,
        AddedVia::Manual,
        &repo.display().to_string(),
        "/repo-key",
    );
    let (instruction, main) = seed_instruction(&database);
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let target = repo.join("AGENTS.md");
    simulate_sync(
        &database,
        &project.id,
        &synced_hash_of(Scheme::A, &target, "Be brief."),
    );
    let webview = main_webview(&app);

    // The independent baseline (the scheme test's own control): the
    // pass over the unmodified row diffs nothing, or the dirty
    // assert below would prove nothing.
    let clean =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the clean pass must succeed");
    assert_eq!(
        clean,
        Vec::new(),
        "the world starts clean — the baseline is the oracle's own control"
    );

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": true, "position": "start" }),
        serde_json::Value::Null,
    )
    .expect("the extra-file change succeeds");

    assert_eq!(
        std::fs::read(&target).expect("the old file must be readable"),
        b"hand-written content\n".to_vec(),
        "no file I/O — the reading line is the next sync's to write"
    );
    let dirty =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the dirty pass must succeed");
    assert_eq!(
        dirty,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: project.id.clone(),
            dirty: true,
        }],
        "an extra-file change makes the project dirty (DB-020's no-qualifier formula)"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_extra_file_change_dirties_a_scheme_b_row_too() {
    // D-204 point 3's DIRTY half, the load-bearing arm of
    // store-anyway (round 1's P2-2): under scheme B the managed text
    // gains NO reading line (ALG-030's "(A only)"), so the dirty
    // verdict can come only through DB-020's own inputs — the
    // extra-file flags and positions, hashed with no scheme
    // qualifier. A row already dirty for another reason would make
    // this vacuous, so the world starts clean over the scheme-B
    // baseline and the control asserts it.
    let (temp, app) = project_update_dirty_app();
    let database = database_of(&app);
    let repo = repo_with_agents_md(temp.path());
    let project = seed_project_with_scheme(
        &database,
        AddedVia::Manual,
        &repo.display().to_string(),
        "/repo-key",
        Scheme::B,
    );
    let (instruction, main) = seed_instruction(&database);
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let target = repo.join("AGENTS.md");
    simulate_sync(
        &database,
        &project.id,
        &synced_hash_of(Scheme::B, &target, "Be brief."),
    );
    let webview = main_webview(&app);

    let clean =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the clean pass must succeed");
    assert_eq!(
        clean,
        Vec::new(),
        "the scheme-B world starts clean — the baseline is the oracle's own control"
    );

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": true, "position": "start" }),
        serde_json::Value::Null,
    )
    .expect("the extra-file change over the scheme-B row succeeds");

    let dirty =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the dirty pass must succeed");
    assert_eq!(
        dirty,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: project.id.clone(),
            dirty: true,
        }],
        "DB-020's formula hashes the extra options with no scheme \
         qualifier: the flags alone moved the hash"
    );
    assert!(
        stored_project(&database, &project.id).extra_local.enabled,
        "the dormant value is stored (the row, not the scheme window)"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_disabled_extra_position_change_is_a_real_row_change() {
    // D-204 point 9's whole-pair comparison, the other load-bearing
    // half (round 1's P2-2): a position-only change with the flag
    // STILL OFF is a real row change — the column moves, DB-020
    // hashes the position whatever the flag says, and the journal
    // records it. A no-op strip that compared `enabled` alone (the
    // alternative the reading rejected) would leave this test red
    // in exactly one of its three asserts.
    let (temp, app) = project_update_dirty_app();
    let database = database_of(&app);
    let repo = repo_with_agents_md(temp.path());
    let project = seed_project(
        &database,
        AddedVia::Manual,
        &repo.display().to_string(),
        "/repo-key",
    );
    let (instruction, main) = seed_instruction(&database);
    seed_assignment(&database, &project.id, &instruction.id, &main.id);
    let target = repo.join("AGENTS.md");
    simulate_sync(
        &database,
        &project.id,
        &synced_hash_of(Scheme::A, &target, "Be brief."),
    );
    let webview = main_webview(&app);

    let clean =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the clean pass must succeed");
    assert_eq!(
        clean,
        Vec::new(),
        "the world starts clean — the baseline is the oracle's own control"
    );

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": false, "position": "start" }),
        serde_json::Value::Null,
    )
    .expect("the disabled position change succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(
        (stored.extra_local.enabled, stored.extra_local.position),
        (false, Position::Start),
        "the dormant position column moved while the flag stayed off"
    );
    assert_eq!(
        changed_journal(&database).len(),
        1,
        "the real row change is journaled"
    );
    let dirty =
        tauri::async_runtime::block_on(crate::dirty_recalc::recalc_and_emit(app.app_handle()))
            .expect("the dirty pass must succeed");
    assert_eq!(
        dirty,
        vec![crate::dirty_recalc::ProjectDirtyChange {
            project_id: project.id.clone(),
            dirty: true,
        }],
        "DB-020 hashes the position input even while the flag is off"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_invalid_scheme_saves_nothing() {
    // D-204 point 2's vocabulary refusal: §7.5's closed A/B/C (the
    // schema CHECK's own case-sensitive posture) — an off-vocabulary
    // literal refuses the whole save (§6.9's all-or-nothing), row
    // verbatim, journal absent, and the refusal echoes the caller's
    // literal bounded (D-115's cap, the color's precedent).
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("D"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("an off-vocabulary scheme must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("scheme"),
        "{error}"
    );
    assert_eq!(
        error["params"]["value"],
        serde_json::json!("D"),
        "the short literal echoes verbatim (D-115's cap never reaches it)"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("the scheme must be one of A, B, C (spec 7.5)"),
        "{error}"
    );

    // Case-sensitive, like the schema CHECK: a lower-case member is
    // not a member.
    let case_error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("a"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("a lower-case scheme literal must be refused");
    assert_eq!(
        case_error["params"]["field"],
        serde_json::json!("scheme"),
        "{case_error}"
    );

    // D-115's bounded echo, pinned at the boundary (the id arm's own
    // rule): an over-long scheme literal echoes truncated with the
    // suffix, never whole — a webview string is unbounded by
    // definition, the vocabulary's shortness notwithstanding.
    let long_error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("A".repeat(70)),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("an over-long scheme literal must be refused the same way");
    assert_eq!(
        long_error["params"]["value"],
        serde_json::json!(format!("{} (+6 more code points)", "A".repeat(64))),
        "{long_error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "nothing was journaled"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_off_list_base_name_saves_nothing() {
    // §6.9's File-name row ("a value from the list") over D-03's list
    // — `AGENTS.md`/`CLAUDE.md`, nothing else ("path and extension
    // are not typed in" is why the vocabulary is closed at all):
    // an off-list literal refuses the whole save.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("GEMINI.md"),
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("an off-list file name must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("base_name"),
        "{error}"
    );
    assert_eq!(
        error["params"]["value"],
        serde_json::json!("GEMINI.md"),
        "{error}"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("the file name must be AGENTS.md or CLAUDE.md (spec D-03)"),
        "{error}"
    );

    // Case-sensitive here too (the schema CHECK's posture): the
    // lower-case spelling of a member is not a member.
    let case_error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("agents.md"),
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("a lower-case file name must be refused");
    assert_eq!(
        case_error["params"]["field"],
        serde_json::json!("base_name"),
        "{case_error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_invalid_extra_position_saves_nothing() {
    // The extra-file object's position member is a closed vocabulary
    // (§6.5 `start`/`end`): an off-vocabulary literal refuses the
    // whole save naming the WIRE field (`extra_local` — the object,
    // the one the modal's control tree names), the offending position
    // echoed bounded.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project(&database, AddedVia::Manual, &repo_path, "/repo-key");
    let webview = main_webview(&app);

    let error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": true, "position": "middle" }),
        serde_json::Value::Null,
    )
    .expect_err("an off-vocabulary position must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("extra_local"),
        "{error}"
    );
    assert_eq!(
        error["params"]["value"],
        serde_json::json!("middle"),
        "{error}"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("the position must be start or end (spec 6.5)"),
        "{error}"
    );

    // The OTHER extra's own token (round 1's P3-1): the envelope
    // names the WIRE field the modal's control tree names, so a
    // swapped literal in either arm cannot survive.
    let project_error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": true, "position": "middle" }),
    )
    .expect_err("an invalid extra_project position must be refused");
    assert_eq!(
        project_error["params"]["field"],
        serde_json::json!("extra_project"),
        "{project_error}"
    );
    assert_eq!(
        project_error["params"]["value"],
        serde_json::json!("middle"),
        "{project_error}"
    );

    // The pair's own order (PRJ-012's sentence: extra_local before
    // extra_project): a two-fault send with BOTH positions invalid
    // answers about `extra_local`.
    let both_error = invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": true, "position": "middle" }),
        serde_json::json!({ "enabled": true, "position": "middle" }),
    )
    .expect_err("the earlier extra in the pair's order answers");
    assert_eq!(
        both_error["params"]["field"],
        serde_json::json!("extra_local"),
        "{both_error}"
    );

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "nothing was saved"
    );
    assert_eq!(changed_journal(&database), Vec::new());
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_new_fields_validate_after_the_6_9_fields_and_in_their_own_order() {
    // D-204 point 2's ordering claims, pinned (D-201's two-fault
    // precedent extended): the §6.9 fields answer before the new half
    // whatever the row, and the new half answers in PRJ-012's own
    // sentence order — scheme before file name before the extras. A
    // validator reorder cannot survive the battery.
    let (temp, app) = project_update_app();
    let database = database_of(&app);
    let webview = main_webview(&app);

    let name_first = invoke_project_update_patch(
        &webview,
        "no-such-row",
        serde_json::json!("x".repeat(101)),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("D"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the §6.9 fields validate before the new half");
    assert_eq!(
        name_first["params"]["field"],
        serde_json::json!("name"),
        "the name answers, not the scheme"
    );

    let scheme_before_base_name = invoke_project_update_patch(
        &webview,
        "no-such-row",
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("D"),
        serde_json::json!("GEMINI.md"),
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect_err("the scheme answers before the file name");
    assert_eq!(
        scheme_before_base_name["params"]["field"],
        serde_json::json!("scheme"),
        "the scheme answers, not the file name"
    );

    let base_name_before_extras = invoke_project_update_patch(
        &webview,
        "no-such-row",
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("GEMINI.md"),
        serde_json::json!({ "enabled": true, "position": "middle" }),
        serde_json::Value::Null,
    )
    .expect_err("the file name answers before the extras");
    assert_eq!(
        base_name_before_extras["params"]["field"],
        serde_json::json!("base_name"),
        "the file name answers, not the extra position"
    );

    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "nothing was journaled"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn extra_file_fields_are_stored_under_scheme_b() {
    // D-204 point 3, the store-anyway reading the spec's own words
    // decide: the §6.2 columns are NOT NULL on every row, §6.5 seeds
    // `default.extra_*` for every new project whatever the scheme,
    // and the builder — not the store — is what ignores them under
    // B/C (ALG-030's "(A only)"). A scheme-B project therefore SAVES
    // an extra-file patch like any other: stored and journaled here
    // (the dirty half of the same reading is pinned by its own test
    // below — `an_extra_file_change_dirties_a_scheme_b_row_too`).
    // The modal hides the controls under B/C (PRJ-008's
    // scheme-conditional block) — the wire never refuses them.
    let (temp, app) = project_update_watch_state_app();
    let database = database_of(&app);
    let repo_path = temp.path().join("repo").display().to_string();
    let project = seed_project_with_scheme(
        &database,
        AddedVia::Manual,
        &repo_path,
        "/repo-key",
        Scheme::B,
    );
    let webview = main_webview(&app);

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": true, "position": "start" }),
        serde_json::Value::Null,
    )
    .expect("an extra-file patch over a scheme-B row succeeds");

    let stored = stored_project(&database, &project.id);
    assert_eq!(
        stored.scheme,
        Scheme::B,
        "the scheme is untouched by the patch"
    );
    assert_eq!(
        (stored.extra_local.enabled, stored.extra_local.position),
        (true, Position::Start),
        "the dormant value is stored (the builder ignores it under B, the row keeps it)"
    );
    let journal = changed_journal(&database);
    assert_eq!(journal.len(), 1, "the real row change is journaled");
    assert_changed_entry(&journal[0], &project.id);
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

/// The wiring world (D-204 point 7): the watch service task spawned
/// over the production budget, so the committed arm's awaited pass is
/// the real one — the `add_manual_watch_app` shape with this
/// command's registration (no SessionPaths: this command never joins
/// the SEC-003 allowlist).
fn project_update_watch_service_app() -> (
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
            crate::project_update::project_update
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

#[test]
fn a_scheme_change_runs_one_awaited_watch_pass_through_the_mutation_seam() {
    // D-204 point 7, the wiring's proof (SYS-007/D-144, T-279's arm):
    // the committed watch-input change awaits ONE full watch pass
    // through the bounded mutation seam — the same refresh path
    // (command, ack, service pass) `project_add_manual`'s committed
    // arm runs. The deterministic observable is the pass counter
    // (the add-flow test's own rule), and the awaited seam means the
    // count is settled when the invoke returns. The pass covers the
    // row — and writes no file (OUT-011: only a sync writes).
    let (temp, app, commands) = project_update_watch_service_app();
    let repo = repo_with_agents_md(temp.path());
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Manual,
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
    let webview = main_webview(&app);

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("B"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the scheme change succeeds — the refresh is never-fatal aftermath");

    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled + 1,
        "the awaited seam ran exactly one full pass after the settled startup baseline"
    );
    assert_eq!(
        stored_project(&database, &project.id).scheme,
        Scheme::B,
        "the row the pass covered carries the new scheme"
    );
    assert_eq!(
        changed_journal(&database).len(),
        1,
        "one journal entry for the committed change"
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
fn a_base_name_or_extra_change_runs_one_awaited_watch_pass_each() {
    // The wiring condition's other arms, pinned the pass-counter way:
    // the file name and the extra-file options are watch-verdict
    // inputs like the scheme (the pass's `ProjectFact` consumes all
    // four), so each committed change of theirs awaits its own full
    // pass. A refresh condition narrowed to the scheme alone (the
    // SYS-007 sentence's most literal reading) would leave both acts
    // below silent and this test red.
    let (temp, app, commands) = project_update_watch_service_app();
    let repo = repo_with_agents_md(temp.path());
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Manual,
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
    let webview = main_webview(&app);

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("CLAUDE.md"),
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the file-name change succeeds");
    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled + 1,
        "the file-name change awaited one full pass over the settled baseline"
    );

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!({ "enabled": true, "position": "start" }),
        serde_json::Value::Null,
    )
    .expect("the extra-file change succeeds");
    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled + 2,
        "the extra-file change awaited its own full pass"
    );
    assert_eq!(
        changed_journal(&database).len(),
        2,
        "both real changes journaled"
    );
    // Windows teardown rule.
    crate::ipc_tests::stop_watch_service(app.handle());
    drop(app);
    drop(temp);
}

#[test]
fn a_name_only_change_runs_no_watch_pass() {
    // The wiring's negative half, D-201 point 7's boundary extended:
    // a patch that changed NO watch-verdict input (the name is not
    // one — SYS-007's own list) runs no pass; the save itself stands.
    // Deterministic without any sleep: the seam is AWAITED inside the
    // command, so nothing is left in flight when the invoke returns —
    // the count cannot move after the assert (no dirty service runs
    // here, and no file event fires over an untouched repo).
    let (temp, app, commands) = project_update_watch_service_app();
    let repo = repo_with_agents_md(temp.path());
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Manual,
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
    let webview = main_webview(&app);

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::json!("Only The Name"),
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
    )
    .expect("the name-only save succeeds");

    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled,
        "a name-only change runs no watch pass — the refresh is the \
         watch-input arm's alone (D-201 point 7, extended)"
    );
    assert_eq!(
        stored_project(&database, &project.id).name,
        "Only The Name",
        "the save itself stands"
    );
    assert_eq!(
        changed_journal(&database).len(),
        1,
        "the real change is journaled"
    );
    // Windows teardown rule.
    crate::ipc_tests::stop_watch_service(app.handle());
    drop(app);
    drop(temp);
}

#[test]
fn the_unchanged_write_configuration_resend_runs_no_pass_and_journals_nothing() {
    // D-204 point 9, the no-op extension with the wiring's own
    // observable: a re-send of the stored write configuration (the
    // scheme, the file name and both extra pairs, compared as whole
    // values) writes and journals nothing — and runs no watch pass
    // either (a pass over unchanged inputs would be file I/O with no
    // observable change, D-201 point 7's own reading).
    let (temp, app, commands) = project_update_watch_service_app();
    let repo = repo_with_agents_md(temp.path());
    let database = database_of(&app);
    let project = seed_project(
        &database,
        AddedVia::Manual,
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
    let webview = main_webview(&app);

    invoke_project_update_patch(
        &webview,
        &project.id,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::Value::Null,
        serde_json::json!("A"),
        serde_json::json!("AGENTS.md"),
        serde_json::json!({ "enabled": false, "position": "end" }),
        serde_json::json!({ "enabled": false, "position": "end" }),
    )
    .expect("the unchanged re-send converges with Ok");

    assert_eq!(
        stored_project(&database, &project.id),
        project,
        "the verified no-op: not even updated_at churned"
    );
    assert_eq!(
        changed_journal(&database),
        Vec::new(),
        "the re-send journals nothing"
    );
    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        settled,
        "the re-send runs no watch pass over the settled baseline"
    );
    // Windows teardown rule.
    crate::ipc_tests::stop_watch_service(app.handle());
    drop(app);
    drop(temp);
}
