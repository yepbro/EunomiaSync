//! Mock-IPC behavior tests for the four Library structure commands
//! (T-330, §7.10; the batch absorbing T-092…T-095).
//!
//! Every command is driven through the real Tauri mock IPC envelope over
//! a real migrated `Db`. Assertions read the repository back: row order,
//! levels, timestamps, journal rows and the absence of heading revisions.
//! The four absorbed definitions of done lead the suite:
//!
//! - T-092: heading name/level validation, journal presence, no versions.
//! - T-093: physical delete keeps the section's instructions in order;
//!   LIB-037 refuses a non-empty root section (instructions, nested
//!   headings, hidden rows included) before any write, while an empty
//!   root still deletes.
//! - T-094: point-wise level change and whole-branch ±1 shift; an
//!   out-of-range shift is `E_VALIDATION` and changes nothing.
//! - T-095: one transaction with renumbering, restart persistence, and
//!   rejection of a branch moved inside itself.

use eunomia_store::action_log::{Action, LogRecord};
use eunomia_store::library::{
    ItemScope, LibraryItem, NewLibraryItem, NewVariant, create_item, create_variant, get_item,
    hide_item, list_items,
};
use tauri::Manager;

use crate::headings::HeadingRecord;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn headings_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::headings::heading_create,
            crate::headings::heading_update,
            crate::headings::heading_delete,
            crate::headings::items_move
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    (temp, app)
}

fn headings_app_without_db() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::headings::heading_create,
            crate::headings::heading_update,
            crate::headings::heading_delete,
            crate::headings::items_move
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app")
}

fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("failed to build the mock `main` webview window")
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
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

fn invoke_heading_create(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    name: &str,
    level: i32,
    parent: Option<&str>,
) -> Result<HeadingRecord, serde_json::Value> {
    invoke(
        webview,
        "heading_create",
        serde_json::json!({ "name": name, "level": level, "parent": parent }),
    )
}

fn invoke_heading_update(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
    update: serde_json::Value,
) -> Result<HeadingRecord, serde_json::Value> {
    invoke(
        webview,
        "heading_update",
        serde_json::json!({ "id": id, "update": update }),
    )
}

fn invoke_heading_delete(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
) -> Result<(), serde_json::Value> {
    invoke(webview, "heading_delete", serde_json::json!({ "id": id }))
}

fn invoke_items_move(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
    target: serde_json::Value,
) -> Result<(), serde_json::Value> {
    invoke(
        webview,
        "items_move",
        serde_json::json!({ "id": id, "target": target }),
    )
}

// ---------------------------------------------------------------------------
// Seeds and read-back
// ---------------------------------------------------------------------------

fn seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::db::sqlite_error_to_app_error(match error {
        eunomia_store::library::LibraryError::Sql(source) => source,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })
}

fn seed_heading(database: &crate::db::Db, title: &str, level: u8) -> LibraryItem {
    let title = title.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        create_item(
            tx,
            &NewLibraryItem::Heading {
                title: &title,
                level,
            },
        )
        .map_err(seed_error)
    }))
    .expect("seeding a heading must work")
}

fn seed_headings(database: &crate::db::Db, count: usize) -> Vec<LibraryItem> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        (0..count)
            .map(|index| {
                create_item(
                    tx,
                    &NewLibraryItem::Heading {
                        title: &format!("Heading {index}"),
                        level: 2,
                    },
                )
                .map_err(seed_error)
            })
            .collect()
    }))
    .expect("seeding a large heading list must work")
}

fn seed_instruction(database: &crate::db::Db, title: &str) -> LibraryItem {
    let title = title.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item =
            create_item(tx, &NewLibraryItem::Instruction { title: &title }).map_err(seed_error)?;
        create_variant(
            tx,
            &NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: "content",
                is_main: true,
            },
        )
        .map_err(seed_error)?;
        Ok(item)
    }))
    .expect("seeding an instruction with its main variant must work")
}

fn hide_instruction(database: &crate::db::Db, id: &str) {
    let id = id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| hide_item(tx, &id).map_err(seed_error)))
        .expect("hiding a seeded instruction must work");
}

fn rows_of(database: &crate::db::Db) -> Vec<LibraryItem> {
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(move |tx| list_items(tx, ItemScope::All).map_err(seed_error)),
    )
    .expect("reading the library must work")
}

fn row_of(database: &crate::db::Db, id: &str) -> Option<LibraryItem> {
    let id = id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| get_item(tx, &id).map_err(seed_error)))
        .expect("reading one library row must work")
}

fn ids_of(rows: &[LibraryItem]) -> Vec<&str> {
    rows.iter().map(|row| row.id.as_str()).collect()
}

fn levels_of(rows: &[LibraryItem]) -> Vec<Option<u8>> {
    rows.iter().map(|row| row.heading_level).collect()
}

fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 1000)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("reading the journal must work")
}

fn journal_count(database: &crate::db::Db, action: Action, entity: &str) -> usize {
    journal_of(database)
        .into_iter()
        .filter(|entry| {
            entry.action == action && entry.entity.as_ref().is_some_and(|(_, id)| id == entity)
        })
        .count()
}

fn revisions_total(database: &crate::db::Db) -> i64 {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.query_row("SELECT COUNT(*) FROM revisions", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("counting revisions must work")
}

fn assert_validation(error: serde_json::Value, field: &str) {
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!(field),
        "{error}"
    );
}

fn stage_journal_refusal(database: &crate::db::Db, actions: &[&str]) {
    let actions = actions.join("', '");
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute_batch(&format!(
            "CREATE TRIGGER reject_structure_journal BEFORE INSERT ON action_log
             WHEN NEW.action IN ('{actions}')
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;"
        ))
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the journal trigger must work");
}

// ---------------------------------------------------------------------------
// T-092 — create and rename/update
// ---------------------------------------------------------------------------

#[test]
fn heading_create_validates_the_name_and_level_before_writing() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let webview = main_webview(&app);

    for name in ["", "   ", "line\nbreak", "nul\0name", &"x".repeat(201)] {
        let error = invoke_heading_create(&webview, name, 2, None)
            .expect_err("an invalid heading name must be refused");
        assert_validation(error, "name");
    }
    for level in [1, 7, -1, 256, 300] {
        let error = invoke_heading_create(&webview, "Valid", level, None)
            .expect_err("a level outside 2 to 6 must be refused");
        assert_validation(error, "level");
    }
    assert!(
        rows_of(&database).is_empty(),
        "no refused create wrote a row"
    );
    assert!(
        journal_of(&database).is_empty(),
        "no refused create journaled"
    );
    assert_eq!(revisions_total(&database), 0);
}

#[test]
fn heading_create_rejects_a_large_name_without_echoing_user_content() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let webview = main_webview(&app);
    let secret = "PRIVATE-HEADING-".repeat(10_000);

    let error = invoke_heading_create(&webview, &secret, 2, None)
        .expect_err("a 160000-character name must be refused in one linear validation pass");
    assert!(
        !error.to_string().contains("PRIVATE-HEADING-"),
        "rule 8: the rejected title never enters the error envelope"
    );
    assert_validation(error, "name");
    assert!(rows_of(&database).is_empty());
}

#[test]
fn heading_create_places_root_and_child_rows_and_journals_without_versions() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let first = seed_instruction(&database, "First");
    let beta = seed_heading(&database, "Beta", 2);
    let second = seed_instruction(&database, "Second");
    let webview = main_webview(&app);

    let child = invoke_heading_create(&webview, "  Child  ", 3, Some(&alpha.id))
        .expect("a child heading must be created");
    assert_eq!(child.title, "Child");
    assert_eq!(child.level, 3);
    assert_eq!(
        child.position, 3,
        "the child lands at its parent's section end"
    );
    let root = invoke_heading_create(&webview, "Root", 2, None)
        .expect("a root heading must append at the table end");
    assert_eq!(root.position, 6);

    assert_eq!(
        ids_of(&rows_of(&database)),
        vec![
            alpha.id.as_str(),
            first.id.as_str(),
            child.id.as_str(),
            beta.id.as_str(),
            second.id.as_str(),
            root.id.as_str()
        ]
    );
    assert_eq!(
        journal_count(&database, Action::HeadingCreated, &child.id),
        1
    );
    assert_eq!(
        journal_count(&database, Action::HeadingCreated, &root.id),
        1
    );
    assert_eq!(
        revisions_total(&database),
        0,
        "headings never have revisions"
    );
}

#[test]
fn heading_create_accepts_the_200_character_boundary() {
    let (_temp, app) = headings_app();
    let webview = main_webview(&app);
    let name = "é".repeat(200);
    let created = invoke_heading_create(&webview, &name, 6, None)
        .expect("200 Unicode scalar values are a legal heading name");
    assert_eq!(created.title, name);
    assert_eq!(created.level, 6);
}

#[test]
fn heading_update_renames_and_journals_once_without_versions() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let heading = seed_heading(&database, "Old", 2);
    let webview = main_webview(&app);

    let renamed = invoke_heading_update(
        &webview,
        &heading.id,
        serde_json::json!({ "mode": "rename", "value": "  New title  " }),
    )
    .expect("a valid rename must succeed");
    assert_eq!(renamed.title, "New title");
    assert_eq!(renamed.id, heading.id);
    assert_eq!(renamed.position, i32::try_from(heading.position).unwrap());
    assert_eq!(
        journal_count(&database, Action::HeadingChanged, &heading.id),
        1
    );
    assert_eq!(revisions_total(&database), 0);

    let before = row_of(&database, &heading.id).expect("the heading must exist");
    let again = invoke_heading_update(
        &webview,
        &heading.id,
        serde_json::json!({ "mode": "rename", "value": "New title" }),
    )
    .expect("a same-name rename is a verified no-op");
    assert_eq!(again.title, "New title");
    assert_eq!(
        row_of(&database, &heading.id).as_ref(),
        Some(&before),
        "a no-op does not restamp the row"
    );
    assert_eq!(
        journal_count(&database, Action::HeadingChanged, &heading.id),
        1
    );
}

#[test]
fn heading_update_validates_every_rename_before_writing() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let heading = seed_heading(&database, "Original", 2);
    let webview = main_webview(&app);
    let before = rows_of(&database);

    for name in ["", "   ", "line\nbreak", "nul\0name", &"x".repeat(201)] {
        let error = invoke_heading_update(
            &webview,
            &heading.id,
            serde_json::json!({ "mode": "rename", "value": name }),
        )
        .expect_err("an invalid rename must be refused");
        assert_validation(error, "name");
        assert_eq!(
            rows_of(&database),
            before,
            "the refused rename changes nothing"
        );
    }
    assert!(journal_of(&database).is_empty());
}

#[test]
fn heading_update_changes_one_level_or_shifts_the_whole_branch() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let item = seed_instruction(&database, "Item");
    let child = seed_heading(&database, "Child", 3);
    let beta = seed_heading(&database, "Beta", 2);
    let webview = main_webview(&app);

    let changed = invoke_heading_update(
        &webview,
        &alpha.id,
        serde_json::json!({ "mode": "change_level", "value": 4 }),
    )
    .expect("a point-wise level change must succeed");
    assert_eq!(changed.level, 4);
    assert_eq!(
        levels_of(&rows_of(&database)),
        vec![Some(4), None, Some(3), Some(2)],
        "change_level touches only the named heading"
    );

    let shifted = invoke_heading_update(
        &webview,
        &child.id,
        serde_json::json!({ "mode": "shift_branch", "value": 1 }),
    )
    .expect("a branch that fits must shift");
    assert_eq!(shifted.level, 4);
    assert_eq!(
        ids_of(&rows_of(&database)),
        vec![
            alpha.id.as_str(),
            item.id.as_str(),
            child.id.as_str(),
            beta.id.as_str()
        ]
    );
    assert_eq!(
        journal_count(&database, Action::HeadingChanged, &alpha.id),
        1
    );
    assert_eq!(
        journal_count(&database, Action::HeadingChanged, &child.id),
        1
    );
    assert_eq!(revisions_total(&database), 0);
}

#[test]
fn heading_update_rejects_invalid_levels_and_shifts_without_changing_anything() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let _item = seed_instruction(&database, "Item");
    let _child = seed_heading(&database, "Child", 3);
    let webview = main_webview(&app);
    let before = rows_of(&database);

    for level in [1, 7, -1, 256, 300] {
        let error = invoke_heading_update(
            &webview,
            &alpha.id,
            serde_json::json!({ "mode": "change_level", "value": level }),
        )
        .expect_err("a level outside 2 to 6 must be refused");
        assert_validation(error, "level");
    }
    for update in [
        serde_json::json!({ "mode": "shift_branch", "value": -1 }),
        serde_json::json!({ "mode": "shift_branch", "value": 2 }),
        serde_json::json!({ "mode": "shift_branch", "value": 128 }),
        serde_json::json!({ "mode": "shift_branch", "value": -129 }),
        serde_json::json!({ "mode": "shift_branch", "value": 300 }),
    ] {
        let error = invoke_heading_update(&webview, &alpha.id, update)
            .expect_err("an invalid branch shift must be refused");
        assert_validation(error, "branch_shift");
    }
    assert_eq!(
        rows_of(&database),
        before,
        "every refusal is all-or-nothing"
    );
    assert!(journal_of(&database).is_empty());
}

#[test]
fn heading_update_rejects_unknown_and_instruction_ids() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let instruction = seed_instruction(&database, "Instruction");
    let webview = main_webview(&app);
    for id in ["missing", &instruction.id] {
        let error = invoke_heading_update(
            &webview,
            id,
            serde_json::json!({ "mode": "change_level", "value": 3 }),
        )
        .expect_err("only a heading id can be updated");
        assert_validation(error, "id");
    }
    assert!(journal_of(&database).is_empty());
}

// ---------------------------------------------------------------------------
// T-093 — physical heading delete
// ---------------------------------------------------------------------------

#[test]
fn heading_delete_keeps_instructions_in_their_previous_order_and_renumbers() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let first = seed_instruction(&database, "First");
    let second = seed_instruction(&database, "Second");
    let child = seed_heading(&database, "Child", 3);
    let third = seed_instruction(&database, "Third");
    let beta = seed_heading(&database, "Beta", 2);
    let fourth = seed_instruction(&database, "Fourth");
    let webview = main_webview(&app);

    invoke_heading_delete(&webview, &child.id).expect("a heading must be physically deleted");

    assert!(row_of(&database, &child.id).is_none());
    let rows = rows_of(&database);
    assert_eq!(
        ids_of(&rows),
        vec![
            alpha.id.as_str(),
            first.id.as_str(),
            second.id.as_str(),
            third.id.as_str(),
            beta.id.as_str(),
            fourth.id.as_str()
        ],
        "the section's instructions stay in their previous order"
    );
    assert_eq!(
        rows.iter().map(|row| row.position).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        journal_count(&database, Action::HeadingDeleted, &child.id),
        1
    );
    assert_eq!(revisions_total(&database), 0);
}

#[test]
fn heading_delete_refuses_an_instruction_or_unknown_id_without_writing() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let instruction = seed_instruction(&database, "Instruction");
    let webview = main_webview(&app);
    let before = rows_of(&database);
    for id in ["missing", &instruction.id] {
        let error = invoke_heading_delete(&webview, id)
            .expect_err("only a heading can be physically deleted");
        assert_validation(error, "id");
    }
    assert_eq!(rows_of(&database), before);
    assert!(journal_of(&database).is_empty());
}

#[test]
fn heading_delete_refuses_a_root_section_that_still_holds_an_instruction() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let _item = seed_instruction(&database, "Item");
    let _beta = seed_heading(&database, "Beta", 2);
    let webview = main_webview(&app);
    let before = rows_of(&database);

    let error = invoke_heading_delete(&webview, &alpha.id)
        .expect_err("a root section holding an instruction is refused (LIB-037)");
    assert_eq!(
        error["params"]["items"],
        serde_json::json!("1"),
        "the refusal carries the blocking item count"
    );
    assert_validation(error, "id");

    assert_eq!(
        rows_of(&database),
        before,
        "the refused delete touches no row, position or order"
    );
    assert!(journal_of(&database).is_empty());
    assert_eq!(revisions_total(&database), 0);
}

#[test]
fn heading_delete_refuses_a_root_section_that_holds_a_nested_heading() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let _child = seed_heading(&database, "Child", 3);
    let _grandchild = seed_instruction(&database, "Grandchild");
    let bare_root = seed_heading(&database, "Bare", 2);
    let _empty_nested = seed_heading(&database, "Nested", 3);
    let webview = main_webview(&app);
    let before = rows_of(&database);

    // A nested heading with content under it: the H3 and its
    // instruction are both items of the root's section.
    let error = invoke_heading_delete(&webview, &alpha.id)
        .expect_err("a root section holding a nested heading is refused (LIB-037)");
    assert_eq!(error["params"]["items"], serde_json::json!("2"));
    assert_validation(error, "id");

    // A nested heading with nothing under it is still an item of the
    // root's section and blocks the delete.
    let error = invoke_heading_delete(&webview, &bare_root.id)
        .expect_err("a nested heading blocks the root's delete even when itself empty");
    assert_eq!(error["params"]["items"], serde_json::json!("1"));
    assert_validation(error, "id");

    assert_eq!(rows_of(&database), before);
    assert!(journal_of(&database).is_empty());
}

#[test]
fn heading_delete_refuses_a_root_section_whose_only_item_is_hidden() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let hidden = seed_instruction(&database, "Hidden");
    hide_instruction(&database, &hidden.id);
    let webview = main_webview(&app);
    let before = rows_of(&database);

    // No closing H2 follows: the section runs to the end of the list,
    // and the invisible row still blocks the delete (LIB-037).
    let error = invoke_heading_delete(&webview, &alpha.id)
        .expect_err("a hidden instruction still counts as the section's item (LIB-037)");
    assert_eq!(error["params"]["items"], serde_json::json!("1"));
    assert_validation(error, "id");

    assert_eq!(rows_of(&database), before);
    assert!(journal_of(&database).is_empty());
}

#[test]
fn heading_delete_still_deletes_an_empty_root_section() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let first = seed_instruction(&database, "First");
    let empty = seed_heading(&database, "Empty", 2);
    let beta = seed_heading(&database, "Beta", 2);
    let last = seed_instruction(&database, "Last");
    let webview = main_webview(&app);

    invoke_heading_delete(&webview, &empty.id)
        .expect("an empty root section still deletes (LIB-037)");

    assert!(row_of(&database, &empty.id).is_none());
    let rows = rows_of(&database);
    assert_eq!(
        ids_of(&rows),
        vec![
            alpha.id.as_str(),
            first.id.as_str(),
            beta.id.as_str(),
            last.id.as_str()
        ],
        "the rows around the deleted empty root keep their order"
    );
    assert_eq!(
        rows.iter().map(|row| row.position).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(
        journal_count(&database, Action::HeadingDeleted, &empty.id),
        1,
        "the successful delete journals exactly as before"
    );
    assert_eq!(revisions_total(&database), 0);
}

// ---------------------------------------------------------------------------
// T-094/T-095 — move semantics, levels, persistence and rollback
// ---------------------------------------------------------------------------

#[test]
fn items_move_keeps_a_branch_whole_and_survives_a_restart() {
    let (temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let first = seed_instruction(&database, "First");
    let child = seed_heading(&database, "Child", 3);
    let hidden = seed_instruction(&database, "Hidden");
    hide_instruction(&database, &hidden.id);
    let beta = seed_heading(&database, "Beta", 2);
    let last = seed_instruction(&database, "Last");
    let webview = main_webview(&app);

    invoke_items_move(
        &webview,
        &alpha.id,
        serde_json::json!({ "mode": "after_item", "id": last.id }),
    )
    .expect("moving a branch after another branch must succeed");
    let expected = vec![
        beta.id.clone(),
        last.id.clone(),
        alpha.id.clone(),
        first.id.clone(),
        child.id.clone(),
        hidden.id.clone(),
    ];
    assert_eq!(ids_of(&rows_of(&database)), expected);
    assert_eq!(
        journal_count(&database, Action::HeadingChanged, &alpha.id),
        1
    );
    assert_eq!(revisions_total(&database), 0);
    drop(app);

    let reopened = crate::db::Db::open(temp.path()).expect("reopening the database must work");
    assert_eq!(
        ids_of(&rows_of(&reopened)),
        expected,
        "the moved order is persisted, not only held in memory"
    );
    assert_eq!(
        rows_of(&reopened)
            .iter()
            .map(|row| row.position)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
}

#[test]
fn items_move_places_an_instruction_after_a_row_or_at_a_heading_section_end() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let first = seed_instruction(&database, "First");
    let child = seed_heading(&database, "Child", 3);
    let beta = seed_heading(&database, "Beta", 2);
    let last = seed_instruction(&database, "Last");
    let webview = main_webview(&app);

    invoke_items_move(
        &webview,
        &first.id,
        serde_json::json!({ "mode": "after_item", "id": last.id }),
    )
    .expect("an instruction can move after another row");
    assert_eq!(
        ids_of(&rows_of(&database)),
        vec![
            alpha.id.as_str(),
            child.id.as_str(),
            beta.id.as_str(),
            last.id.as_str(),
            first.id.as_str()
        ]
    );

    invoke_items_move(
        &webview,
        &first.id,
        serde_json::json!({ "mode": "into_heading", "id": alpha.id }),
    )
    .expect("an instruction dropped on a heading lands at the section end");
    assert_eq!(
        ids_of(&rows_of(&database)),
        vec![
            alpha.id.as_str(),
            child.id.as_str(),
            first.id.as_str(),
            beta.id.as_str(),
            last.id.as_str()
        ]
    );
    assert_eq!(
        journal_count(&database, Action::InstructionChanged, &first.id),
        2
    );
}

#[test]
fn items_move_drops_a_branch_as_the_targets_last_child_and_aligns_its_levels() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let target = seed_heading(&database, "Target", 3);
    let target_item = seed_instruction(&database, "Target item");
    let source = seed_heading(&database, "Source", 2);
    let source_item = seed_instruction(&database, "Source item");
    let nested = seed_heading(&database, "Nested", 3);
    let neighbor = seed_heading(&database, "Neighbor", 2);
    let webview = main_webview(&app);

    invoke_items_move(
        &webview,
        &source.id,
        serde_json::json!({ "mode": "into_heading", "id": target.id }),
    )
    .expect("a branch dropped on a heading becomes its last child");
    let rows = rows_of(&database);
    assert_eq!(
        ids_of(&rows),
        vec![
            target.id.as_str(),
            target_item.id.as_str(),
            source.id.as_str(),
            source_item.id.as_str(),
            nested.id.as_str(),
            neighbor.id.as_str()
        ]
    );
    assert_eq!(
        levels_of(&rows),
        vec![Some(3), None, Some(4), None, Some(5), Some(2)],
        "the moved root and nested heading align to target level + 1"
    );
    assert_eq!(
        journal_count(&database, Action::HeadingChanged, &source.id),
        1
    );
}

#[test]
fn items_move_drops_a_branch_at_the_root_and_aligns_it_to_level_two() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let first = seed_heading(&database, "First", 2);
    let source = seed_heading(&database, "Source", 4);
    let nested = seed_heading(&database, "Nested", 5);
    let source_item = seed_instruction(&database, "Source item");
    let webview = main_webview(&app);

    invoke_items_move(&webview, &source.id, serde_json::json!({ "mode": "root" }))
        .expect("a branch dropped on the library header becomes the first root");
    let rows = rows_of(&database);
    assert_eq!(
        ids_of(&rows),
        vec![
            source.id.as_str(),
            nested.id.as_str(),
            source_item.id.as_str(),
            first.id.as_str()
        ]
    );
    assert_eq!(levels_of(&rows), vec![Some(2), Some(3), None, Some(2)]);
}

#[test]
fn items_move_alignment_touches_only_the_dragged_pre_move_range() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let target = seed_heading(&database, "Target", 4);
    let target_item = seed_instruction(&database, "Target item");
    let closer = seed_heading(&database, "Closer", 3);
    let closer_item = seed_instruction(&database, "Closer item");
    let final_root = seed_heading(&database, "Final root", 2);
    let final_item = seed_instruction(&database, "Final item");
    let source = seed_heading(&database, "Source", 2);
    let source_item = seed_instruction(&database, "Source item");
    let nested = seed_heading(&database, "Nested", 3);
    let webview = main_webview(&app);

    invoke_items_move(
        &webview,
        &source.id,
        serde_json::json!({ "mode": "into_heading", "id": target.id }),
    )
    .expect("the dragged H2/H3 branch can legally align to H5/H6 under the H4 target");

    let rows = rows_of(&database);
    assert_eq!(
        ids_of(&rows),
        vec![
            target.id.as_str(),
            target_item.id.as_str(),
            source.id.as_str(),
            source_item.id.as_str(),
            nested.id.as_str(),
            closer.id.as_str(),
            closer_item.id.as_str(),
            final_root.id.as_str(),
            final_item.id.as_str()
        ]
    );
    assert_eq!(
        levels_of(&rows),
        vec![
            Some(4),
            None,
            Some(5),
            None,
            Some(6),
            Some(3),
            None,
            Some(2),
            None
        ],
        "the H3 that closes the target section is adjacent, not part of the dragged branch"
    );
}

#[test]
fn items_move_rejects_a_branch_dropped_inside_itself() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let alpha = seed_heading(&database, "Alpha", 2);
    let item = seed_instruction(&database, "Item");
    let child = seed_heading(&database, "Child", 3);
    let _child_item = seed_instruction(&database, "Child item");
    let beta = seed_heading(&database, "Beta", 2);
    let webview = main_webview(&app);
    let before = rows_of(&database);

    for target in [
        serde_json::json!({ "mode": "into_heading", "id": alpha.id }),
        serde_json::json!({ "mode": "into_heading", "id": child.id }),
        serde_json::json!({ "mode": "after_item", "id": item.id }),
    ] {
        let error = invoke_items_move(&webview, &alpha.id, target)
            .expect_err("a branch cannot move into its own range");
        assert_validation(error, "target");
        assert_eq!(
            rows_of(&database),
            before,
            "the rejected move changes nothing"
        );
    }
    assert!(journal_of(&database).is_empty());
    assert_eq!(
        row_of(&database, &beta.id)
            .expect("the neighbor survives")
            .position,
        5
    );
}

#[test]
fn items_move_rejects_automatic_alignment_beyond_level_six_without_changes() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let target = seed_heading(&database, "Target", 5);
    let target_item = seed_instruction(&database, "Target item");
    let source = seed_heading(&database, "Source", 5);
    let _nested = seed_heading(&database, "Nested", 6);
    let source_item = seed_instruction(&database, "Source item");
    let webview = main_webview(&app);
    let before = rows_of(&database);

    let error = invoke_items_move(
        &webview,
        &source.id,
        serde_json::json!({ "mode": "into_heading", "id": target.id }),
    )
    .expect_err("aligning the nested H6 to H7 must be refused");
    assert_validation(error, "heading_level");
    assert_eq!(
        rows_of(&database),
        before,
        "neither order nor levels changed"
    );
    assert!(journal_of(&database).is_empty());
    assert_eq!(
        row_of(&database, &target_item.id)
            .expect("target item survives")
            .position,
        2
    );
    assert_eq!(
        row_of(&database, &source_item.id)
            .expect("source item survives")
            .position,
        5
    );
}

#[test]
fn items_move_refuses_unknown_ids_and_a_noop_journals_nothing() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let heading = seed_heading(&database, "Heading", 2);
    let webview = main_webview(&app);
    let before = rows_of(&database);

    let missing_source =
        invoke_items_move(&webview, "missing", serde_json::json!({ "mode": "root" }))
            .expect_err("an unknown source must be refused");
    assert_validation(missing_source, "id");
    let missing_target = invoke_items_move(
        &webview,
        &heading.id,
        serde_json::json!({ "mode": "after_item", "id": "missing" }),
    )
    .expect_err("an unknown target must be refused");
    assert_validation(missing_target, "target");
    assert_eq!(rows_of(&database), before);

    invoke_items_move(&webview, &heading.id, serde_json::json!({ "mode": "root" }))
        .expect("moving the first root to the root is a no-op");
    assert_eq!(rows_of(&database), before);
    assert!(
        journal_of(&database).is_empty(),
        "a no-op writes no journal entry"
    );
}

#[test]
fn items_move_rolls_back_both_order_and_levels_when_its_journal_fails() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let target = seed_heading(&database, "Target", 3);
    let target_item = seed_instruction(&database, "Target item");
    let source = seed_heading(&database, "Source", 2);
    let _nested = seed_heading(&database, "Nested", 3);
    let source_item = seed_instruction(&database, "Source item");
    let before = rows_of(&database);
    stage_journal_refusal(&database, &["heading.changed"]);
    let webview = main_webview(&app);

    let error = invoke_items_move(
        &webview,
        &source.id,
        serde_json::json!({ "mode": "into_heading", "id": target.id }),
    )
    .expect_err("the staged journal refusal must fail the whole move");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation"),
        "the refusal is the staged trigger's constraint abort"
    );
    assert_eq!(
        rows_of(&database),
        before,
        "order and levels rolled back together"
    );
    assert!(journal_of(&database).is_empty());
    assert_eq!(
        row_of(&database, &target_item.id)
            .expect("target item survives")
            .position,
        2
    );
    assert_eq!(
        row_of(&database, &source_item.id)
            .expect("source item survives")
            .position,
        5
    );
}

#[test]
fn heading_create_update_and_delete_each_roll_back_when_their_journal_fails() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let heading = seed_heading(&database, "Heading", 2);
    let doomed = seed_heading(&database, "Doomed", 2);
    let before = rows_of(&database);
    stage_journal_refusal(
        &database,
        &["heading.created", "heading.changed", "heading.deleted"],
    );
    let webview = main_webview(&app);

    let create_error = invoke_heading_create(&webview, "Late", 2, None)
        .expect_err("the create must fail with its journal insert");
    assert_eq!(
        create_error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert_eq!(
        rows_of(&database),
        before,
        "the failed create inserted no row"
    );

    let delete_error = invoke_heading_delete(&webview, &doomed.id)
        .expect_err("the delete must fail with its journal insert");
    assert_eq!(
        delete_error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert_eq!(
        rows_of(&database),
        before,
        "the failed delete restored the row and positions"
    );

    let update_error = invoke_heading_update(
        &webview,
        &heading.id,
        serde_json::json!({ "mode": "rename", "value": "Changed" }),
    )
    .expect_err("the rename must fail with its journal insert");
    assert_eq!(
        update_error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert_eq!(
        rows_of(&database),
        before,
        "the failed rename restored the original title"
    );
    assert!(journal_of(&database).is_empty());
    assert_eq!(revisions_total(&database), 0);
    assert_eq!(
        row_of(&database, &heading.id)
            .expect("the original heading survives")
            .position,
        1
    );
}

#[test]
fn a_large_library_move_writes_the_complete_dense_order() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let headings = seed_headings(&database, 512);
    let webview = main_webview(&app);

    invoke_items_move(
        &webview,
        &headings[511].id,
        serde_json::json!({ "mode": "after_item", "id": headings[0].id }),
    )
    .expect("moving one heading in a large library must succeed");
    let rows = rows_of(&database);
    assert_eq!(rows.len(), 512);
    assert_eq!(rows[0].id, headings[0].id);
    assert_eq!(rows[1].id, headings[511].id);
    assert_eq!(
        rows.iter().map(|row| row.position).collect::<Vec<_>>(),
        (1..=512).collect::<Vec<i64>>(),
        "the large move leaves no gaps or duplicates"
    );
}

// ---------------------------------------------------------------------------
// Cross-cutting command-boundary pins
// ---------------------------------------------------------------------------

#[test]
fn library_structure_edits_are_allowed_while_a_sync_runs() {
    let (_temp, app) = headings_app();
    let database = database_of(&app);
    let heading = seed_heading(&database, "Heading", 2);
    let item = seed_instruction(&database, "Item");
    let doomed = seed_heading(&database, "Doomed", 2);
    let webview = main_webview(&app);
    app.manage(crate::sync_running::SyncRunning::new());
    let guard = crate::commands::begin_sync(app.handle()).expect("the test flag must be free");

    let created = invoke_heading_create(&webview, "During sync", 3, Some(&heading.id))
        .expect("library edits are allowed during a sync");
    invoke_heading_update(
        &webview,
        &heading.id,
        serde_json::json!({ "mode": "rename", "value": "During sync" }),
    )
    .expect("a heading rename is allowed during a sync");
    invoke_items_move(
        &webview,
        &item.id,
        serde_json::json!({ "mode": "into_heading", "id": created.id }),
    )
    .expect("a library move is allowed during a sync");
    invoke_heading_delete(&webview, &doomed.id).expect("a heading delete is allowed during a sync");
    drop(guard);
}

#[test]
fn the_structure_commands_answer_honestly_without_a_database() {
    let app = headings_app_without_db();
    let webview = main_webview(&app);
    let requests = [
        (
            "heading_create",
            serde_json::json!({ "name": "Any", "level": 2, "parent": null }),
        ),
        (
            "heading_update",
            serde_json::json!({ "id": "any", "update": { "mode": "rename", "value": "Any" } }),
        ),
        ("heading_delete", serde_json::json!({ "id": "any" })),
        (
            "items_move",
            serde_json::json!({ "id": "any", "target": { "mode": "root" } }),
        ),
    ];
    for (command, body) in requests {
        let error: Result<HeadingRecord, serde_json::Value> = invoke(&webview, command, body);
        let error = error.expect_err("without a database every structure command refuses");
        assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{command}");
        assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
    }

    let invalid: Result<HeadingRecord, serde_json::Value> = invoke(
        &webview,
        "heading_create",
        serde_json::json!({ "name": "   ", "level": 2, "parent": null }),
    );
    assert_validation(
        invalid.expect_err("field validation precedes database resolution"),
        "name",
    );
}

#[test]
fn the_four_structure_commands_are_registered_once_with_specta() {
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for name in [
        "headings::heading_create",
        "headings::heading_update",
        "headings::heading_delete",
        "headings::items_move",
    ] {
        assert_eq!(
            lib.matches(name).count(),
            1,
            "lib.rs must register `{name}` exactly once (§7.10)"
        );
    }
}
