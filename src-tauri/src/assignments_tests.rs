//! Mock-IPC behavior tests for the three assignment commands (T-331).
//!
//! The commands run over a real migrated `Db`; assertions read the store,
//! journal, revision count, build snapshot and (for OUT-010) a real project
//! file back. The absorbed DoDs lead: MAIN-005 isolation/no file writes,
//! one-transaction 100-pair bulk assignment with rollback, and MAIN-007's
//! shared-variant-safe idempotent per-project custom edit — plus MAIN-034
//! (T-356): the extend mode's base resolution, the ALG-023 build
//! composition and the custom-cell carry-over.

use eunomia_store::action_log::{Action, LogRecord};
use eunomia_store::library::{
    AssignmentTarget, LibraryItem, NewAssignment, NewLibraryItem, NewVariant, Variant, assign,
    create_item, create_variant, delete_variant, get_assignment, get_variant,
    list_assignments_for_project, set_custom,
};
use eunomia_store::projects::{AddedVia, NewProject};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use tauri::Manager;

use crate::assignments::{
    AssignmentPair, AssignmentRecord, AssignmentTargetRecord, BulkAssignmentResult,
};

fn assignments_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path()).expect("data layout creates");
    let database = crate::db::Db::open(temp.path()).expect("fresh database opens");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::assignments::assignment_set,
            crate::assignments::assignment_set_custom,
            crate::assignments::assignments_bulk,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds");
    app.manage(database);
    (temp, app)
}

fn assignments_app_without_db() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::assignments::assignment_set,
            crate::assignments::assignment_set_custom,
            crate::assignments::assignments_bulk,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds")
}

fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
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

fn invoke<T: serde::de::DeserializeOwned>(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    body: serde_json::Value,
) -> Result<T, serde_json::Value> {
    match tauri::test::get_ipc_response(webview, invoke_request(cmd, body)) {
        Ok(payload) => Ok(payload
            .deserialize::<T>()
            .expect("success payload deserializes into the command DTO")),
        Err(error) => Err(error),
    }
}

fn invoke_assignment_set(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
    instruction_id: &str,
    enabled: bool,
    variant_id: Option<&str>,
) -> Result<Option<AssignmentRecord>, serde_json::Value> {
    invoke(
        webview,
        "assignment_set",
        serde_json::json!({
            "projectId": project_id,
            "instructionId": instruction_id,
            "enabled": enabled,
            "variantId": variant_id,
        }),
    )
}

fn invoke_assignment_set_custom(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
    instruction_id: &str,
    content: Option<&str>,
    mode: Option<&str>,
) -> Result<Option<AssignmentRecord>, serde_json::Value> {
    invoke(
        webview,
        "assignment_set_custom",
        serde_json::json!({
            "projectId": project_id,
            "instructionId": instruction_id,
            "content": content,
            "mode": mode,
        }),
    )
}

fn invoke_assignments_bulk(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    instruction_ids: &[&str],
    project_ids: &[&str],
    enabled: bool,
    variant_id: Option<&str>,
) -> Result<BulkAssignmentResult, serde_json::Value> {
    invoke(
        webview,
        "assignments_bulk",
        serde_json::json!({
            "instructionIds": instruction_ids,
            "projectIds": project_ids,
            "enabled": enabled,
            "variantId": variant_id,
        }),
    )
}

fn project_sample<'a>(n: usize, path: &'a str) -> NewProject<'a> {
    NewProject {
        path: Box::leak(path.to_owned().into_boxed_str()),
        path_key: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
        name: Box::leak(format!("Project {n}").into_boxed_str()),
        label: Box::leak(format!("P{n}").into_boxed_str()),
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
    }
}

fn seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::db::sqlite_error_to_app_error(match error {
        eunomia_store::library::LibraryError::Sql(source) => source,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })
}

fn seed_project(database: &crate::db::Db, n: usize) -> String {
    seed_project_at(database, n, &format!("/repos/repo-{n}"))
}

fn seed_project_at(database: &crate::db::Db, n: usize, path: &str) -> String {
    let path = path.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create(tx, &project_sample(n, &path))
            .map(|project| project.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("project seed creates")
}

fn seed_instruction(database: &crate::db::Db, n: usize) -> (LibraryItem, Variant) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item = create_item(
            tx,
            &NewLibraryItem::Instruction {
                title: &format!("I{n}"),
            },
        )
        .map_err(seed_error)?;
        let main = create_variant(
            tx,
            &NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: &format!("shared {n}"),
                is_main: true,
            },
        )
        .map_err(seed_error)?;
        Ok((item, main))
    }))
    .expect("instruction seed creates")
}

fn seed_variant(
    database: &crate::db::Db,
    instruction_id: &str,
    name: &str,
    content: &str,
) -> Variant {
    let instruction_id = instruction_id.to_owned();
    let name = name.to_owned();
    let content = content.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        create_variant(
            tx,
            &NewVariant {
                instruction_id: &instruction_id,
                name: &name,
                content: &content,
                is_main: false,
            },
        )
        .map_err(seed_error)
    }))
    .expect("variant seed creates")
}

fn seed_heading(database: &crate::db::Db) -> LibraryItem {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        create_item(
            tx,
            &NewLibraryItem::Heading {
                title: "Section",
                level: 2,
            },
        )
        .map_err(seed_error)
    }))
    .expect("heading seed creates")
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

fn seed_custom(database: &crate::db::Db, project_id: &str, instruction_id: &str, content: &str) {
    let project_id = project_id.to_owned();
    let instruction_id = instruction_id.to_owned();
    let content = content.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        set_custom(
            tx,
            &project_id,
            &instruction_id,
            Some(&content),
            eunomia_store::library::CustomMode::Replace,
        )
        .map(|_| ())
        .map_err(seed_error)
    }))
    .expect("custom assignment seed creates")
}

fn stored_variant(database: &crate::db::Db, id: &str) -> Option<Variant> {
    let id = id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| get_variant(tx, &id).map_err(seed_error)))
        .expect("variant read succeeds")
}

/// Builds one project's body from the live snapshot and asserts it —
/// the extend tests' build oracle (ALG-023 over the real DB state).
fn assert_body_of(database: &crate::db::Db, project_id: &str, expected: &str) {
    let database = database.clone();
    let project_id = project_id.to_owned();
    let expected = expected.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let snapshot = eunomia_store::build_snapshot::read(tx)
            .map_err(|error| crate::commands::rejected_field("snapshot", "", &error.to_string()))?;
        let id = eunomia_core::project::ProjectId::new(project_id.clone());
        assert_eq!(
            eunomia_core::body::build_body(&snapshot, &id).as_deref(),
            Some(expected.as_str())
        );
        Ok::<_, crate::errors::AppError>(())
    }))
    .expect("snapshot and body read");
}

fn stored_assignment(
    database: &crate::db::Db,
    project_id: &str,
    instruction_id: &str,
) -> Option<eunomia_store::library::Assignment> {
    let project_id = project_id.to_owned();
    let instruction_id = instruction_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(
        database
            .run(move |tx| get_assignment(tx, &project_id, &instruction_id).map_err(seed_error)),
    )
    .expect("assignment read succeeds")
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

fn variant_count(database: &crate::db::Db, instruction_id: &str) -> i64 {
    let instruction_id = instruction_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row(
            "SELECT COUNT(*) FROM instruction_variants WHERE instruction_id = ?1",
            [&instruction_id],
            |row| row.get(0),
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("variant count reads")
}

fn revisions_total(database: &crate::db::Db) -> i64 {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.query_row("SELECT COUNT(*) FROM revisions", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("revision count reads")
}

fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 1000)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("journal reads")
}

fn assignment_journal(database: &crate::db::Db) -> Vec<LogRecord> {
    journal_of(database)
        .into_iter()
        .filter(|entry| entry.action == Action::AssignmentChanged)
        .collect()
}

fn assert_validation(error: serde_json::Value, field: &str) {
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!(field),
        "{error}"
    );
}

#[test]
fn assignment_set_switches_only_one_project_and_never_touches_project_files() {
    let (temp, app) = assignments_app();
    let database = database_of(&app);
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).expect("repo directory creates");
    let target = repo.join("AGENTS.md");
    let original = b"do not rewrite this file\n";
    std::fs::write(&target, original).expect("target file writes");
    let p1 = seed_project_at(&database, 1, repo.to_str().expect("UTF-8 temp path"));
    let p2 = seed_project(&database, 2);
    let (instruction, main) = seed_instruction(&database, 1);
    let alt = seed_variant(&database, &instruction.id, "Alt", "alternative");
    seed_assignment(&database, &p1, &instruction.id, &main.id);
    seed_assignment(&database, &p2, &instruction.id, &main.id);
    let p2_before = stored_assignment(&database, &p2, &instruction.id).expect("p2 row exists");
    let webview = main_webview(&app);

    let changed = invoke_assignment_set(&webview, &p1, &instruction.id, true, Some(&alt.id))
        .expect("variant switch succeeds")
        .expect("enabled cell returns its row");
    assert_eq!(
        changed.target,
        AssignmentTargetRecord::Variant(alt.id.clone())
    );
    assert_eq!(
        stored_assignment(&database, &p2, &instruction.id).as_ref(),
        Some(&p2_before),
        "MAIN-005: the other project keeps its exact row"
    );
    assert_eq!(
        std::fs::read(&target).expect("target reads"),
        original,
        "OUT-010: no file write"
    );
    assert_eq!(
        revisions_total(&database),
        0,
        "assignments are not versioned entities"
    );
    let journal = assignment_journal(&database);
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].project_id.as_deref(), Some(p1.as_str()));
    assert_eq!(
        journal[0].details,
        Some(serde_json::json!({ "changed": 1 }))
    );
}

#[test]
fn assignment_set_defaults_to_main_toggles_off_and_refuses_invalid_variants() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, main) = seed_instruction(&database, 1);
    let (other, other_main) = seed_instruction(&database, 2);
    let dead = seed_variant(&database, &instruction.id, "Dead", "dead");
    let heading = seed_heading(&database);
    let database_for_delete = database.clone();
    let dead_id = dead.id.clone();
    tauri::async_runtime::block_on(
        database_for_delete.run(move |tx| delete_variant(tx, &dead_id, None).map_err(seed_error)),
    )
    .expect("soft delete succeeds");
    let webview = main_webview(&app);

    let enabled = invoke_assignment_set(&webview, &project, &instruction.id, true, None)
        .expect("main default succeeds")
        .expect("row exists");
    assert_eq!(
        enabled.target,
        AssignmentTargetRecord::Variant(main.id.clone())
    );
    let before = stored_assignment(&database, &project, &instruction.id).expect("row exists");
    let same = invoke_assignment_set(&webview, &project, &instruction.id, true, None)
        .expect("same target succeeds")
        .expect("row exists");
    assert_eq!(
        stored_assignment(&database, &project, &instruction.id).as_ref(),
        Some(&before)
    );
    assert_eq!(same, enabled);
    assert_eq!(
        assignment_journal(&database).len(),
        1,
        "the first enable journals once"
    );
    let journal_after_same = assignment_journal(&database);
    assert_eq!(
        journal_after_same.len(),
        1,
        "same-target re-send writes no second journal"
    );

    for variant in ["missing", other_main.id.as_str(), dead.id.as_str()] {
        let error = invoke_assignment_set(&webview, &project, &instruction.id, true, Some(variant))
            .expect_err("unknown/cross-instruction/deleted variant is refused");
        assert_validation(error, "variant_id");
    }
    let unknown_project = invoke_assignment_set(&webview, "missing", &instruction.id, true, None)
        .expect_err("unknown project is refused");
    assert_validation(unknown_project, "project_id");
    let unknown_instruction = invoke_assignment_set(&webview, &project, "missing", true, None)
        .expect_err("unknown instruction is refused");
    assert_validation(unknown_instruction, "instruction_id");
    let heading_error = invoke_assignment_set(&webview, &project, &heading.id, true, None)
        .expect_err("heading has no cell");
    assert_validation(heading_error, "instruction_id");
    let other_error = invoke_assignment_set(&webview, &project, &other.id, true, Some(&main.id))
        .expect_err("another instruction's main cannot be assigned");
    assert_validation(other_error, "variant_id");
    let off_with_variant =
        invoke_assignment_set(&webview, &project, &instruction.id, false, Some(&main.id))
            .expect_err("disable does not accept a variant");
    assert_validation(off_with_variant, "variant_id");
    assert_eq!(
        stored_assignment(&database, &project, &instruction.id).as_ref(),
        Some(&before)
    );

    assert_eq!(
        invoke_assignment_set(&webview, &project, &instruction.id, false, None)
            .expect("toggle off succeeds"),
        None
    );
    assert!(stored_assignment(&database, &project, &instruction.id).is_none());
    assert_eq!(
        assignment_journal(&database).len(),
        2,
        "only the real disable adds a journal"
    );
    assert_eq!(
        invoke_assignment_set(&webview, &project, &instruction.id, false, None)
            .expect("repeat off succeeds"),
        None
    );
    assert_eq!(
        assignment_journal(&database).len(),
        2,
        "repeat off is a no-op"
    );
}

#[test]
fn assignment_set_enable_without_a_variant_preserves_an_existing_custom_cell() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, main) = seed_instruction(&database, 1);
    seed_custom(&database, &project, &instruction.id, "keep custom");
    let before =
        stored_assignment(&database, &project, &instruction.id).expect("custom row exists");
    let webview = main_webview(&app);

    let answer = invoke_assignment_set(&webview, &project, &instruction.id, true, None)
        .expect("enable re-send succeeds")
        .expect("custom row remains");
    assert_eq!(
        answer.target,
        AssignmentTargetRecord::Custom("keep custom".to_owned())
    );
    assert_eq!(
        stored_assignment(&database, &project, &instruction.id).as_ref(),
        Some(&before)
    );
    assert!(
        assignment_journal(&database).is_empty(),
        "an already-enabled custom cell is a no-op"
    );

    let explicit = invoke_assignment_set(&webview, &project, &instruction.id, true, Some(&main.id))
        .expect("explicit variant selection succeeds")
        .expect("row remains");
    assert_eq!(explicit.target, AssignmentTargetRecord::Variant(main.id));
    assert_eq!(
        assignment_journal(&database).len(),
        1,
        "only the explicit conversion journals"
    );
}

#[test]
fn assignments_bulk_deduplicates_input_ids_before_forming_the_product() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let first = seed_instruction(&database, 1);
    let second = seed_instruction(&database, 2);
    let p1 = seed_project(&database, 1);
    let p2 = seed_project(&database, 2);
    let webview = main_webview(&app);

    let result = invoke_assignments_bulk(
        &webview,
        &[&first.0.id, &first.0.id, &second.0.id],
        &[&p1, &p1, &p2],
        true,
        None,
    )
    .expect("small dedupe batch succeeds");
    assert_eq!(result.changed, 4);
    assert!(result.already_assigned.is_empty());
    assert_eq!(assignments_of(&database, &p1).len(), 2);
    assert_eq!(assignments_of(&database, &p2).len(), 2);
    let journal = assignment_journal(&database);
    assert_eq!(journal.len(), 1, "one entry for the whole product");
    assert_eq!(
        journal[0].details,
        Some(serde_json::json!({ "changed": 4 }))
    );
}

#[test]
fn assignments_bulk_applies_100_pairs_in_one_operation() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let instructions: Vec<(LibraryItem, Variant)> =
        (1..=10).map(|n| seed_instruction(&database, n)).collect();
    let projects: Vec<String> = (1..=10).map(|n| seed_project(&database, n)).collect();
    let instruction_ids: Vec<&str> = instructions
        .iter()
        .map(|(item, _)| item.id.as_str())
        .collect();
    let project_ids: Vec<&str> = projects.iter().map(String::as_str).collect();
    let webview = main_webview(&app);

    let result = invoke_assignments_bulk(&webview, &instruction_ids, &project_ids, true, None)
        .expect("one 100-pair batch succeeds");
    assert_eq!(result.changed, 100);
    assert!(result.already_assigned.is_empty());
    let total: usize = projects
        .iter()
        .map(|project| assignments_of(&database, project).len())
        .sum();
    assert_eq!(
        total, 100,
        "the full row/column product exists exactly once"
    );
    let journal = assignment_journal(&database);
    assert_eq!(journal.len(), 1, "one operation writes one journal entry");
    assert_eq!(
        journal[0].entity, None,
        "a bulk entry identifies no single synthetic entity"
    );
    assert_eq!(journal[0].project_id, None);
    assert_eq!(
        journal[0].details,
        Some(serde_json::json!({ "changed": 100 }))
    );
    assert_eq!(revisions_total(&database), 0);
}

#[test]
fn assignments_bulk_returns_existing_variants_and_custom_cells_without_replacing_them() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let alt = seed_variant(&database, &instruction.id, "Alt", "alternative");
    let p1 = seed_project(&database, 1);
    let p2 = seed_project(&database, 2);
    let p3 = seed_project(&database, 3);
    seed_assignment(&database, &p1, &instruction.id, &alt.id);
    seed_custom(&database, &p2, &instruction.id, "custom project two");
    let p1_before = stored_assignment(&database, &p1, &instruction.id).expect("p1 exists");
    let p2_before = stored_assignment(&database, &p2, &instruction.id).expect("p2 exists");
    let webview = main_webview(&app);

    let result = invoke_assignments_bulk(
        &webview,
        &[&instruction.id],
        &[&p1, &p2, &p3],
        true,
        Some(&main.id),
    )
    .expect("bulk assign succeeds");
    assert_eq!(result.changed, 1);
    assert_eq!(result.already_assigned.len(), 2);
    assert_eq!(
        stored_assignment(&database, &p1, &instruction.id).as_ref(),
        Some(&p1_before)
    );
    assert_eq!(
        stored_assignment(&database, &p2, &instruction.id).as_ref(),
        Some(&p2_before)
    );
    assert_eq!(
        stored_assignment(&database, &p3, &instruction.id)
            .expect("p3 exists")
            .target,
        AssignmentTarget::Variant(main.id)
    );
    assert_eq!(assignment_journal(&database).len(), 1);
    assert_eq!(
        assignment_journal(&database)[0].details,
        Some(serde_json::json!({ "changed": 1 }))
    );
}

#[test]
fn assignments_bulk_clear_column_skips_custom_cells_and_removes_variant_cells() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let p1 = seed_project(&database, 1);
    let p2 = seed_project(&database, 2);
    let p3 = seed_project(&database, 3);
    seed_assignment(&database, &p1, &instruction.id, &main.id);
    seed_custom(&database, &p2, &instruction.id, "keep me");
    let custom_before = stored_assignment(&database, &p2, &instruction.id).expect("custom exists");
    let webview = main_webview(&app);

    let result =
        invoke_assignments_bulk(&webview, &[&instruction.id], &[&p1, &p2, &p3], false, None)
            .expect("clear column succeeds");
    assert_eq!(result.changed, 1);
    assert_eq!(result.already_assigned.len(), 1);
    assert_eq!(result.already_assigned[0].project_id, p2);
    assert!(stored_assignment(&database, &p1, &instruction.id).is_none());
    assert!(stored_assignment(&database, &p3, &instruction.id).is_none());
    assert_eq!(
        stored_assignment(&database, &p2, &instruction.id).as_ref(),
        Some(&custom_before)
    );
    assert_eq!(assignment_journal(&database).len(), 1);
}

#[test]
fn assignments_bulk_rolls_back_every_pair_when_a_later_insert_fails() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let instructions: Vec<(LibraryItem, Variant)> =
        (1..=10).map(|n| seed_instruction(&database, n)).collect();
    let projects: Vec<String> = (1..=10).map(|n| seed_project(&database, n)).collect();
    let doomed_project = projects[4].clone();
    let doomed_instruction = instructions[4].0.id.clone();
    let database_for_trigger = database.clone();
    tauri::async_runtime::block_on(database_for_trigger.run(move |tx| {
        tx.execute_batch(&format!(
            "CREATE TRIGGER reject_one_bulk_pair BEFORE INSERT ON project_instructions
             WHEN NEW.project_id = '{doomed_project}' AND NEW.instruction_id = '{doomed_instruction}'
             BEGIN SELECT RAISE(ABORT, 'test: reject one pair'); END;"
        ))
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("trigger stages");
    let instruction_ids: Vec<&str> = instructions
        .iter()
        .map(|(item, _)| item.id.as_str())
        .collect();
    let project_ids: Vec<&str> = projects.iter().map(String::as_str).collect();
    let webview = main_webview(&app);

    let error = invoke_assignments_bulk(&webview, &instruction_ids, &project_ids, true, None)
        .expect_err("the staged 45th-pair refusal fails the whole batch");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    for project in &projects {
        assert!(
            assignments_of(&database, project).is_empty(),
            "no earlier pair survives"
        );
    }
    assert!(
        assignment_journal(&database).is_empty(),
        "no batch journal survives"
    );
}

#[test]
fn assignment_set_custom_is_idempotent_never_forks_and_builds_only_that_projects_text() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let p1 = seed_project(&database, 1);
    let p2 = seed_project(&database, 2);
    seed_assignment(&database, &p2, &instruction.id, &main.id);
    let shared_before = stored_variant(&database, &main.id).expect("main exists");
    let webview = main_webview(&app);

    let first = invoke_assignment_set_custom(
        &webview,
        &p1,
        &instruction.id,
        Some("project one\r\n"),
        None,
    )
    .expect("custom edit succeeds")
    .expect("custom row exists");
    assert_eq!(
        first.target,
        AssignmentTargetRecord::Custom("project one\n".to_owned())
    );
    let first_row = stored_assignment(&database, &p1, &instruction.id).expect("custom row exists");
    let same =
        invoke_assignment_set_custom(&webview, &p1, &instruction.id, Some("project one\n"), None)
            .expect("same custom edit succeeds")
            .expect("custom row exists");
    assert_eq!(
        stored_assignment(&database, &p1, &instruction.id).as_ref(),
        Some(&first_row)
    );
    assert_eq!(same, first);
    assert_eq!(
        assignment_journal(&database).len(),
        1,
        "same text does not journal twice"
    );

    let second = invoke_assignment_set_custom(
        &webview,
        &p1,
        &instruction.id,
        Some("project one updated"),
        None,
    )
    .expect("second edit succeeds")
    .expect("custom row exists");
    assert_eq!(
        second.created_at, first.created_at,
        "the second call updates the same assignment"
    );
    assert_eq!(
        variant_count(&database, &instruction.id),
        1,
        "no fork variant is ever created"
    );
    assert_eq!(
        revisions_total(&database),
        0,
        "custom assignment edits create no versions"
    );
    assert_eq!(
        stored_variant(&database, &main.id).expect("main exists"),
        shared_before,
        "the shared variant is untouched (S5)"
    );
    assert_eq!(
        stored_assignment(&database, &p2, &instruction.id)
            .expect("p2 exists")
            .target,
        AssignmentTarget::Variant(main.id)
    );

    let database_for_build = database.clone();
    let p1_for_build = p1.clone();
    let p2_for_build = p2.clone();
    tauri::async_runtime::block_on(database_for_build.run(move |tx| {
        let snapshot = eunomia_store::build_snapshot::read(tx)
            .map_err(|error| crate::commands::rejected_field("snapshot", "", &error.to_string()))?;
        let p1_id = eunomia_core::project::ProjectId::new(p1_for_build.clone());
        let p2_id = eunomia_core::project::ProjectId::new(p2_for_build.clone());
        assert_eq!(
            eunomia_core::body::build_body(&snapshot, &p1_id).as_deref(),
            Some("project one updated")
        );
        assert_eq!(
            eunomia_core::body::build_body(&snapshot, &p2_id).as_deref(),
            Some("shared 1")
        );
        Ok::<_, crate::errors::AppError>(())
    }))
    .expect("snapshot and bodies read");

    let cleared = invoke_assignment_set_custom(&webview, &p1, &instruction.id, None, None)
        .expect("clear succeeds")
        .expect("row remains assigned");
    assert_eq!(
        cleared.target,
        AssignmentTargetRecord::Variant(shared_before.id)
    );
    assert_eq!(
        assignment_journal(&database).len(),
        3,
        "create, update and clear are three real changes"
    );
}

#[test]
fn assignment_set_custom_validates_text_before_the_database_and_never_echoes_it() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    let webview = main_webview(&app);

    for content in ["a\0b", &"x".repeat(262_145)] {
        let error =
            invoke_assignment_set_custom(&webview, &project, &instruction.id, Some(content), None)
                .expect_err("invalid custom text is refused");
        assert_validation(error.clone(), "custom_text");
        assert!(!error.to_string().contains(&"x".repeat(10)));
    }
    let marker = invoke_assignment_set_custom(
        &webview,
        &project,
        &instruction.id,
        Some("text\n<!-- EUNOMIASYNC:BEGIN -->"),
        None,
    )
    .expect_err("reserved marker is refused");
    assert_eq!(marker["code"], serde_json::json!("E_CONTENT_RESERVED"));
    assert_eq!(marker["params"]["field"], serde_json::json!("custom_text"));
    assert!(stored_assignment(&database, &project, &instruction.id).is_none());
    assert!(assignment_journal(&database).is_empty());

    let empty = invoke_assignment_set_custom(&webview, &project, &instruction.id, Some(""), None)
        .expect("empty custom text is legal")
        .expect("row exists");
    assert_eq!(empty.target, AssignmentTargetRecord::Custom(String::new()));

    let app_without_db = assignments_app_without_db();
    let webview_without_db = main_webview(&app_without_db);
    let error =
        invoke_assignment_set_custom(&webview_without_db, "p", "i", Some("bad\0text"), None)
            .expect_err("field validation precedes database resolution");
    assert_validation(error, "custom_text");
}

#[test]
fn assignment_set_custom_extend_keeps_the_base_composes_at_build_time_and_clears_to_main() {
    // MAIN-034/ALG-023: the extend write keeps the cell's variant as the
    // base beside the folded supplement; the build composes the base's
    // CURRENT text with the supplement (a library edit flows into the
    // next build); clearing drops the mode and restores the MAIN.
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let alt = seed_variant(&database, &instruction.id, "Alt", "alt shared text");
    let p1 = seed_project(&database, 1);
    seed_assignment(&database, &p1, &instruction.id, &alt.id);
    let webview = main_webview(&app);

    let extend = invoke_assignment_set_custom(
        &webview,
        &p1,
        &instruction.id,
        Some("supplement\r\n"),
        Some("extend"),
    )
    .expect("extend write succeeds")
    .expect("row exists");
    assert_eq!(
        extend.target,
        AssignmentTargetRecord::Extend {
            base: alt.id.clone(),
            supplement: "supplement\n".to_owned()
        },
        "the assigned variant becomes the base, the supplement is folded"
    );
    assert_eq!(
        stored_assignment(&database, &p1, &instruction.id)
            .expect("row exists")
            .target,
        AssignmentTarget::Extend {
            base: alt.id.clone(),
            supplement: "supplement\n".to_owned()
        }
    );
    assert_eq!(
        variant_count(&database, &instruction.id),
        2,
        "no fork variant is ever created"
    );
    assert_eq!(assignment_journal(&database).len(), 1);

    // The same-content re-send is a no-op: nothing written, nothing
    // journaled twice.
    let re_sent = invoke_assignment_set_custom(
        &webview,
        &p1,
        &instruction.id,
        Some("supplement\n"),
        Some("extend"),
    )
    .expect("same extend write succeeds")
    .expect("row exists");
    assert_eq!(re_sent, extend);
    assert_eq!(assignment_journal(&database).len(), 1);

    // The build composes the base's current text with the supplement.
    assert_body_of(&database, &p1, "alt shared text\n\nsupplement");

    // ALG-023's build-time read: a library edit to the base variant
    // flows into the next build without touching the cell.
    let alt_id = alt.id.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE instruction_variants SET content = ?1 WHERE id = ?2",
            rusqlite::params!["edited alt text", alt_id],
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("library edit applies");
    assert_body_of(&database, &p1, "edited alt text\n\nsupplement");
    assert_eq!(
        stored_assignment(&database, &p1, &instruction.id)
            .expect("row exists")
            .target,
        AssignmentTarget::Extend {
            base: alt.id.clone(),
            supplement: "supplement\n".to_owned()
        },
        "the library edit never rewrites the cell"
    );

    // Clearing (whatever the mode) restores the MAIN, not the base.
    let cleared =
        invoke_assignment_set_custom(&webview, &p1, &instruction.id, None, Some("extend"))
            .expect("clear succeeds")
            .expect("row remains assigned");
    assert_eq!(cleared.target, AssignmentTargetRecord::Variant(main.id));
    // Two real changes journaled: the extend write and the clear (the
    // raw library edit is a test fixture, not a command).
    assert_eq!(assignment_journal(&database).len(), 2);
}

#[test]
fn assignment_set_custom_extend_validates_the_supplement_and_adopts_the_main_for_a_variantless_cell()
 {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let p1 = seed_project(&database, 1);
    let p2 = seed_project(&database, 2);
    let webview = main_webview(&app);

    // DB-016's reserved-marker rule covers the supplement (MAIN-034):
    // refused before anything runs, no row, no journal.
    let marker = invoke_assignment_set_custom(
        &webview,
        &p1,
        &instruction.id,
        Some("text\n<!-- EUNOMIASYNC:BEGIN -->"),
        Some("extend"),
    )
    .expect_err("reserved marker is refused");
    assert_eq!(marker["code"], serde_json::json!("E_CONTENT_RESERVED"));
    assert_eq!(marker["params"]["field"], serde_json::json!("custom_text"));
    assert!(stored_assignment(&database, &p1, &instruction.id).is_none());
    assert!(assignment_journal(&database).is_empty());

    // An unassigned cell adopts the active main as its base (MAIN-034:
    // "the main variant for an unassigned cell").
    let unassigned = invoke_assignment_set_custom(
        &webview,
        &p1,
        &instruction.id,
        Some("supplement"),
        Some("extend"),
    )
    .expect("extend on an unassigned cell succeeds")
    .expect("row exists");
    assert_eq!(
        unassigned.target,
        AssignmentTargetRecord::Extend {
            base: main.id.clone(),
            supplement: "supplement".to_owned()
        }
    );

    // A replace-custom cell has no variant to keep, so it takes the same
    // main (switching replace → extend keeps nothing of the old shape).
    let replaced =
        invoke_assignment_set_custom(&webview, &p2, &instruction.id, Some("whole cell"), None)
            .expect("replace write succeeds")
            .expect("row exists");
    assert_eq!(
        replaced.target,
        AssignmentTargetRecord::Custom("whole cell".to_owned())
    );
    let extended = invoke_assignment_set_custom(
        &webview,
        &p2,
        &instruction.id,
        Some("supplement"),
        Some("extend"),
    )
    .expect("extend over a replace cell succeeds")
    .expect("row exists");
    assert_eq!(
        extended.target,
        AssignmentTargetRecord::Extend {
            base: main.id.clone(),
            supplement: "supplement".to_owned()
        }
    );

    // DB-010 broken out-of-band (no active main to adopt): the honest
    // refusal, never a guess.
    let main_id = main.id.clone();
    let p3 = seed_project(&database, 3);
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE instruction_variants SET is_main = 0 WHERE id = ?1",
            rusqlite::params![main_id],
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("out-of-band main flag clear applies");
    let error =
        invoke_assignment_set_custom(&webview, &p3, &instruction.id, Some("x"), Some("extend"))
            .expect_err("no active main to adopt refuses");
    assert_validation(error, "instruction_id");
    assert!(stored_assignment(&database, &p3, &instruction.id).is_none());
}

#[test]
fn assignment_set_and_bulk_treat_an_extend_cell_as_a_custom_cell() {
    // D-203 reading 4e: everything already ruled for custom cells carries
    // over — a null-variant assignment_set is a no-op, an explicit
    // variant converts the cell (the supplement is dropped), and bulk
    // work never touches the cell.
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let alt = seed_variant(&database, &instruction.id, "Alt", "alt shared text");
    let p1 = seed_project(&database, 1);
    let p2 = seed_project(&database, 2);
    seed_assignment(&database, &p1, &instruction.id, &alt.id);
    seed_assignment(&database, &p2, &instruction.id, &alt.id);
    let webview = main_webview(&app);
    for project in [&p1, &p2] {
        invoke_assignment_set_custom(
            &webview,
            project,
            &instruction.id,
            Some("supplement"),
            Some("extend"),
        )
        .expect("extend write succeeds")
        .expect("row exists");
    }
    let before = stored_assignment(&database, &p1, &instruction.id).expect("row exists");
    let journal_before = assignment_journal(&database).len();

    // Null variant: a verified no-op — the extend cell is a custom cell.
    let no_op = invoke_assignment_set(&webview, &p1, &instruction.id, true, None)
        .expect("null variant is a no-op")
        .expect("row exists");
    assert_eq!(
        no_op.target,
        AssignmentTargetRecord::Extend {
            base: alt.id.clone(),
            supplement: "supplement".to_owned()
        }
    );
    assert_eq!(
        stored_assignment(&database, &p1, &instruction.id).as_ref(),
        Some(&before)
    );
    assert_eq!(assignment_journal(&database).len(), journal_before);

    // An explicit variant converts the cell: the supplement and the mode
    // are dropped with the custom shape.
    let converted = invoke_assignment_set(&webview, &p1, &instruction.id, true, Some(&main.id))
        .expect("explicit variant converts the extend cell")
        .expect("row exists");
    assert_eq!(
        converted.target,
        AssignmentTargetRecord::Variant(main.id.clone())
    );
    assert_eq!(
        stored_assignment(&database, &p1, &instruction.id)
            .expect("row exists")
            .target,
        AssignmentTarget::Variant(main.id)
    );
    assert_eq!(assignment_journal(&database).len(), journal_before + 1);

    // Bulk clear skips the extend cell.
    let bulk = invoke_assignments_bulk(&webview, &[&instruction.id], &[&p2], false, None)
        .expect("bulk clear succeeds");
    assert_eq!(bulk.changed, 0);
    assert_eq!(
        bulk.already_assigned,
        vec![AssignmentPair {
            project_id: p2.clone(),
            instruction_id: instruction.id.clone(),
        }]
    );
    assert_eq!(
        stored_assignment(&database, &p2, &instruction.id)
            .expect("row exists")
            .target,
        AssignmentTarget::Extend {
            base: alt.id,
            supplement: "supplement".to_owned()
        },
        "bulk work never touches the extend cell"
    );
}

#[test]
fn every_assignment_write_rolls_back_when_its_journal_insert_fails() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let alt = seed_variant(&database, &instruction.id, "Alt", "alt");
    let p1 = seed_project(&database, 1);
    let p2 = seed_project(&database, 2);
    seed_assignment(&database, &p1, &instruction.id, &main.id);
    let before = stored_assignment(&database, &p1, &instruction.id).expect("row exists");
    let database_for_trigger = database.clone();
    tauri::async_runtime::block_on(database_for_trigger.run(move |tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_assignment_journal BEFORE INSERT ON action_log
             WHEN NEW.action = 'assignment.changed'
             BEGIN SELECT RAISE(ABORT, 'test: reject journal'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("trigger stages");
    let webview = main_webview(&app);

    let single = invoke_assignment_set(&webview, &p1, &instruction.id, true, Some(&alt.id))
        .expect_err("single write fails with its journal");
    assert_eq!(
        single["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert_eq!(
        stored_assignment(&database, &p1, &instruction.id).as_ref(),
        Some(&before)
    );

    let custom = invoke_assignment_set_custom(&webview, &p2, &instruction.id, Some("custom"), None)
        .expect_err("custom upsert fails with its journal");
    assert_eq!(
        custom["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert!(stored_assignment(&database, &p2, &instruction.id).is_none());

    let bulk = invoke_assignments_bulk(&webview, &[&instruction.id], &[&p2], true, None)
        .expect_err("bulk write fails with its one journal");
    assert_eq!(
        bulk["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert!(stored_assignment(&database, &p2, &instruction.id).is_none());
    assert!(assignment_journal(&database).is_empty());
}

#[test]
fn assignment_commands_are_allowed_during_sync_and_answer_without_a_database() {
    let (_temp, app) = assignments_app();
    let database = database_of(&app);
    let project = seed_project(&database, 1);
    let (instruction, _) = seed_instruction(&database, 1);
    let webview = main_webview(&app);
    app.manage(crate::sync_running::SyncRunning::new());
    let guard = crate::commands::begin_sync(app.handle()).expect("sync flag starts free");

    invoke_assignment_set(&webview, &project, &instruction.id, true, None)
        .expect("library edit allowed");
    invoke_assignment_set_custom(
        &webview,
        &project,
        &instruction.id,
        Some("during sync"),
        None,
    )
    .expect("custom edit allowed");
    invoke_assignments_bulk(&webview, &[&instruction.id], &[&project], true, None)
        .expect("bulk library edit allowed");
    drop(guard);

    let app_without_db = assignments_app_without_db();
    let webview_without_db = main_webview(&app_without_db);
    let requests = [
        (
            "assignment_set",
            serde_json::json!({"projectId":"p","instructionId":"i","enabled":true,"variantId":null}),
        ),
        (
            "assignment_set_custom",
            serde_json::json!({"projectId":"p","instructionId":"i","content":"text"}),
        ),
        (
            "assignments_bulk",
            serde_json::json!({"instructionIds":["i"],"projectIds":["p"],"enabled":true,"variantId":null}),
        ),
    ];
    for (command, body) in requests {
        let error: Result<Option<AssignmentRecord>, serde_json::Value> =
            invoke(&webview_without_db, command, body);
        let error = error.expect_err("every assignment command refuses without a database");
        assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{command}");
        assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
    }
}

#[test]
fn the_three_assignment_commands_are_registered_once_with_specta() {
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for name in [
        "assignments::assignment_set::<",
        "assignments::assignment_set_custom::<",
        "assignments::assignments_bulk::<",
    ] {
        assert_eq!(
            lib.matches(name).count(),
            1,
            "lib.rs registers `{name}` once"
        );
    }
    assert!(
        !lib.contains("variant_fork_for_project"),
        "D-125 removed the fork command"
    );
}
