//! Mock-IPC behavior tests for the `project_update` command (T-106,
//! PRJ-008/009/011/021, §6.9, SYS-005, LOG-001…004, D-201).
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
//! D-051 one-transaction rollback.

use eunomia_store::action_log::{
    Action, EntityType, Level, LogRecord, Outcome, Visibility, recent,
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

/// Invokes `project_update` over the mock-IPC path. Each optional
/// field crosses as its wire value — `null` (absent, the patch
/// semantics) or the sent string — so the tests exercise exactly the
/// payloads the webview would send.
#[allow(clippy::too_many_arguments)]
fn invoke_project_update(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    project_id: &str,
    name: serde_json::Value,
    label: serde_json::Value,
    color: serde_json::Value,
    description: serde_json::Value,
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
