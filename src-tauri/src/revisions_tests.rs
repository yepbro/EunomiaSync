//! Mock-IPC behavior tests for `revisions_list` and `revision_get` (T-331).

use eunomia_store::action_log::LogRecord;
use eunomia_store::library::{
    LibraryItem, NewLibraryItem, NewVariant, Variant, create_item, create_variant, delete_variant,
};
use eunomia_store::library_versions::{
    InstructionOp, VariantOp, record_instruction, record_variant,
};
use tauri::Manager;

use crate::revisions::{RevisionRecord, RevisionSummaryRecord};

fn revisions_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("temp directory creates");
    crate::data_dirs::ensure_data_layout(temp.path()).expect("data layout creates");
    let database = crate::db::Db::open(temp.path()).expect("database opens");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::revisions::revisions_list,
            crate::revisions::revision_get,
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds");
    app.manage(database);
    (temp, app)
}

fn revisions_app_without_db() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::revisions::revisions_list,
            crate::revisions::revision_get,
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
            .expect("success payload deserializes into the revision DTO")),
        Err(error) => Err(error),
    }
}

fn seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::db::sqlite_error_to_app_error(match error {
        eunomia_store::library::LibraryError::Sql(source) => source,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })
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
        record_instruction(
            tx,
            &item.id,
            InstructionOp::Create,
            &format!("{{\"name\":\"I{n}\",\"state\":\"active\"}}"),
            &format!("instruction-{n}"),
            200,
        )
        .map_err(|error| crate::commands::rejected_field("revision", "", &error.to_string()))?;
        record_variant(
            tx,
            &main.id,
            VariantOp::Create,
            &format!("{{\"name\":\"Main\",\"text\":\"shared {n}\"}}"),
            &format!("main-{n}"),
            200,
        )
        .map_err(|error| crate::commands::rejected_field("revision", "", &error.to_string()))?;
        Ok((item, main))
    }))
    .expect("instruction and versions seed")
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
    .expect("heading seeds")
}

fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 1000)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("journal reads")
}

#[test]
fn revisions_list_returns_instruction_and_variant_versions_newest_first() {
    let (_temp, app) = revisions_app();
    let database = database_of(&app);
    let (instruction, main) = seed_instruction(&database, 1);
    let (unrelated, _) = seed_instruction(&database, 2);
    let database_for_versions = database.clone();
    let instruction_id = instruction.id.clone();
    let main_id = main.id.clone();
    let alt_id = tauri::async_runtime::block_on(database_for_versions.run(move |tx| {
        let alt = create_variant(
            tx,
            &NewVariant {
                instruction_id: &instruction_id,
                name: "Alt",
                content: "alternative",
                is_main: false,
            },
        )
        .map_err(seed_error)?;
        record_variant(
            tx,
            &alt.id,
            VariantOp::Create,
            "{\"name\":\"Alt\",\"text\":\"alternative\"}",
            "alt-create",
            200,
        )
        .map_err(|error| crate::commands::rejected_field("revision", "", &error.to_string()))?;
        delete_variant(tx, &alt.id, None).map_err(seed_error)?;
        record_variant(
            tx,
            &alt.id,
            VariantOp::Delete,
            "{\"name\":\"Alt\",\"text\":\"alternative\"}",
            "alt-delete",
            200,
        )
        .map_err(|error| crate::commands::rejected_field("revision", "", &error.to_string()))?;
        record_variant(
            tx,
            &main_id,
            VariantOp::Update,
            "{\"name\":\"Main\",\"text\":\"updated\"}",
            "main-update",
            200,
        )
        .map_err(|error| crate::commands::rejected_field("revision", "", &error.to_string()))?;
        for (entity_type, entity_id, rev_no, stamp) in [
            (
                "instruction",
                instruction_id.as_str(),
                1,
                "2026-11-24T12:00:01.000Z",
            ),
            ("variant", main_id.as_str(), 1, "2026-11-24T12:00:02.000Z"),
            ("variant", main_id.as_str(), 2, "2026-11-24T12:00:03.000Z"),
            ("variant", alt.id.as_str(), 1, "2026-11-24T12:00:04.000Z"),
            ("variant", alt.id.as_str(), 2, "2026-11-24T12:00:05.000Z"),
        ] {
            tx.execute(
                "UPDATE revisions SET created_at = ?1
                  WHERE entity_type = ?2 AND entity_id = ?3 AND rev_no = ?4",
                rusqlite::params![stamp, entity_type, entity_id, rev_no],
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
        }
        Ok::<_, crate::errors::AppError>(alt.id)
    }))
    .expect("versions seed");
    let webview = main_webview(&app);

    let history: Vec<RevisionSummaryRecord> = invoke(
        &webview,
        "revisions_list",
        serde_json::json!({ "instructionId": instruction.id }),
    )
    .expect("history list succeeds");
    assert_eq!(
        history
            .iter()
            .map(|row| (
                row.entity_type.as_str(),
                row.entity_id.as_str(),
                row.rev_no.as_str(),
                row.op.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("variant", alt_id.as_str(), "2", "delete"),
            ("variant", alt_id.as_str(), "1", "create"),
            ("variant", main.id.as_str(), "2", "update"),
            ("variant", main.id.as_str(), "1", "create"),
            ("instruction", instruction.id.as_str(), "1", "create"),
        ],
        "the instruction and all variants (including deleted) are newest-first; unrelated history is absent"
    );
    assert_eq!(history[0].created_at, "2026-11-24T12:00:05.000Z");
    let raw: serde_json::Value = invoke(
        &webview,
        "revisions_list",
        serde_json::json!({ "instructionId": instruction.id }),
    )
    .expect("raw history list succeeds");
    assert!(
        raw.as_array()
            .and_then(|rows| rows.first())
            .and_then(|row| row.get("snapshot"))
            .is_none(),
        "the list wire carries summaries only; full snapshots require revision_get"
    );
    assert_eq!(history[0].snapshot_hash, "alt-delete");
    assert_eq!(history[0].reverted_from, None);
    assert_ne!(history[0].entity_id, unrelated.id);

    for row in &history {
        let fetched: RevisionRecord = invoke(
            &webview,
            "revision_get",
            serde_json::json!({ "id": row.id }),
        )
        .expect("one revision reads");
        assert_eq!(fetched.id, row.id);
        assert_eq!(fetched.entity_type, row.entity_type);
        assert_eq!(fetched.entity_id, row.entity_id);
        assert_eq!(fetched.rev_no, row.rev_no);
        assert_eq!(fetched.op, row.op);
        assert_eq!(fetched.snapshot_hash, row.snapshot_hash);
        if row.op == "delete" {
            assert_eq!(
                fetched.snapshot,
                "{\"name\":\"Alt\",\"text\":\"alternative\"}"
            );
        }
    }
    assert!(
        journal_of(&database).is_empty(),
        "history reads journal nothing"
    );
}

#[test]
fn revision_get_refuses_unknown_ids_and_history_refuses_unknown_or_heading_instructions() {
    let (_temp, app) = revisions_app();
    let database = database_of(&app);
    let (instruction, _) = seed_instruction(&database, 1);
    let heading = seed_heading(&database);
    let webview = main_webview(&app);

    let unknown: Result<RevisionRecord, serde_json::Value> = invoke(
        &webview,
        "revision_get",
        serde_json::json!({ "id": "missing" }),
    );
    assert_eq!(
        unknown.expect_err("unknown revision refuses")["params"]["field"],
        serde_json::json!("id")
    );
    let unknown: Result<Vec<RevisionSummaryRecord>, serde_json::Value> = invoke(
        &webview,
        "revisions_list",
        serde_json::json!({ "instructionId": "missing" }),
    );
    let unknown = unknown.expect_err("unknown instruction refuses");
    assert_eq!(
        unknown["params"]["reason"],
        serde_json::json!("no instruction has this id")
    );
    let heading_error: Result<Vec<RevisionSummaryRecord>, serde_json::Value> = invoke(
        &webview,
        "revisions_list",
        serde_json::json!({ "instructionId": heading.id }),
    );
    let heading_error = heading_error.expect_err("heading refuses");
    assert_eq!(heading_error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        heading_error["params"]["field"],
        serde_json::json!("instruction_id")
    );
    assert_eq!(
        heading_error["params"]["reason"],
        serde_json::json!("a heading has no version history (LIB-026/6.2)")
    );
    let empty: Vec<RevisionSummaryRecord> = invoke(
        &webview,
        "revisions_list",
        serde_json::json!({ "instructionId": instruction.id }),
    )
    .expect("valid instruction history reads");
    assert_eq!(empty.len(), 2);
    assert!(journal_of(&database).is_empty());
}

#[test]
fn history_commands_answer_honestly_without_a_database() {
    let app = revisions_app_without_db();
    let webview = main_webview(&app);
    let list: Result<Vec<RevisionSummaryRecord>, serde_json::Value> = invoke(
        &webview,
        "revisions_list",
        serde_json::json!({ "instructionId": "i" }),
    );
    let error = list.expect_err("list refuses without DB");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
    let get: Result<RevisionRecord, serde_json::Value> =
        invoke(&webview, "revision_get", serde_json::json!({ "id": "r" }));
    let error = get.expect_err("get refuses without DB");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn the_two_history_commands_are_registered_once_with_specta() {
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for name in ["revisions::revisions_list", "revisions::revision_get"] {
        assert_eq!(
            lib.matches(name).count(),
            1,
            "lib.rs registers `{name}` once"
        );
    }
}
