//! Mock-runtime tests of the five variant commands (T-329, §7.10
//! Library module; the batch absorbing T-087…T-091) — the
//! `scan_tests.rs` harness shape: a real `Db` over a temp root, the
//! commands registered over the mock runtime, driven through the real
//! IPC envelope, with every "nothing was written" claim read back
//! through the store or direct SQL.
//!
//! The absorbed cards' definitions of done lead, one battery each:
//!
//! - **T-087 `variant_create`** — both creation modes (a copy of the
//!   main one, and empty) plus the duplicate-name refusal.
//! - **T-088 `variant_update`** — every save rule (the CRLF→LF fold,
//!   NUL, the reserved marker line, the 262,144-character limit) and
//!   the no-version-without-a-real-change gate.
//! - **T-089 `variant_set_main`** — after the change the instruction
//!   has exactly one main variant.
//! - **T-090 `variant_delete`** — no assignment disappears (they move
//!   to the replacement, the main by default), the last active
//!   variant is refused even when a replacement is named, the main
//!   needs a named successor, and one transaction rolls everything
//!   back when a later write fails.
//! - **T-091 `variant_restore`** — the soft-deleted variant is active
//!   again, and a name reused by an active twin is `E_VALIDATION`.
//!
//! The seeds mirror the world the future `instruction_create` task
//! will build: every instruction's `Main` variant carries its `create`
//! version from the start (§9.8), so the version assertions measure
//! the COMMAND's own writes, not the seed's.
//!
//! Cross-cutting pins: the versions' op spellings and rev_no growth
//! (ALG-071), the D-16 no-version rule for the main flag, the
//! journal's `variant.*` actions with their entity columns (LOG-001
//! …004), §5.4's "Library edits during a sync are allowed" (SYS-005
//! does not gate these commands), the honest no-database answer, and
//! the registration pins.

use eunomia_store::action_log::LogRecord;
use eunomia_store::library::{LibraryItem, NewLibraryItem, NewVariant, Variant};
use eunomia_store::projects::{AddedVia, NewProject};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use tauri::Manager;

use crate::variants::VariantRecord;

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the variant commands serve: the
/// five commands registered over the mock runtime and a fresh
/// database in a temp root. No `SyncRunning` state is managed — these
/// are library commands and §5.4's SYS-005 scope never gates them
/// (pinned by `library_edits_are_allowed_while_a_sync_runs`).
fn variants_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::variants::variant_create,
            crate::variants::variant_delete,
            crate::variants::variant_restore,
            crate::variants::variant_set_main,
            crate::variants::variant_update
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    (temp, app)
}

/// The database behind the app (the `scan_tests.rs` shape).
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

/// The mock `main` webview window the IPC requests go through (the
/// `scan_tests.rs` shape).
fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("failed to build the mock `main` webview window")
}

/// One invoke request for `cmd` with a JSON body (the `ipc_tests.rs`
/// shape).
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

/// Runs one IPC request and splits the answer into its typed halves.
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

/// Invokes `variant_create` with the given source spelling.
fn invoke_variant_create(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    instruction_id: &str,
    name: &str,
    source: &str,
) -> Result<VariantRecord, serde_json::Value> {
    invoke(
        webview,
        "variant_create",
        serde_json::json!({
            "instructionId": instruction_id,
            "name": name,
            "source": source,
        }),
    )
}

/// Invokes `variant_update`; `None` leaves the field unchanged on the
/// wire too (an explicit JSON `null`).
fn invoke_variant_update(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
    name: Option<&str>,
    content: Option<&str>,
) -> Result<VariantRecord, serde_json::Value> {
    invoke(
        webview,
        "variant_update",
        serde_json::json!({
            "id": id,
            "name": name,
            "content": content,
        }),
    )
}

/// Invokes `variant_set_main`.
fn invoke_variant_set_main(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
) -> Result<VariantRecord, serde_json::Value> {
    invoke(webview, "variant_set_main", serde_json::json!({ "id": id }))
}

/// Invokes `variant_delete` with the given replacement (`None` is
/// LIB-023's default-to-main).
fn invoke_variant_delete(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
    replacement: Option<&str>,
) -> Result<VariantRecord, serde_json::Value> {
    invoke(
        webview,
        "variant_delete",
        serde_json::json!({ "id": id, "replacement": replacement }),
    )
}

/// Invokes `variant_restore`.
fn invoke_variant_restore(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
) -> Result<VariantRecord, serde_json::Value> {
    invoke(webview, "variant_restore", serde_json::json!({ "id": id }))
}

// ---------------------------------------------------------------------------
// The seed helpers (every write through the `Db::run` executor, the
// path a producing command takes)
// ---------------------------------------------------------------------------

/// One [`NewProject`] with a distinct path and the default write
/// configuration (the `ipc_tests.rs` `column_project` shape).
fn project_sample<'a>(n: usize) -> NewProject<'a> {
    NewProject {
        path: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
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

/// Maps a seed-time store failure onto the wire shape the executor
/// speaks: the seeds use legal input, so any failure here is a test
/// bug — the fallback carries the store's own error class, never used
/// as an assertion target.
fn seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    crate::db::sqlite_error_to_app_error(match error {
        eunomia_store::library::LibraryError::Sql(source) => source,
        other => rusqlite::Error::InvalidParameterName(other.to_string()),
    })
}

/// Records the variant's `create` version — the half of the world the
/// future `instruction_create`/`variant_create` commands own, so the
/// command under test finds a versioned entity (§9.8) exactly as in
/// production.
fn record_create_version(tx: &rusqlite::Transaction<'_>, variant: &Variant) -> Result<(), ()> {
    let max_revisions = match eunomia_store::settings::get(
        tx,
        eunomia_store::settings::SettingKey::HistoryMaxRevisions,
    ) {
        Ok(eunomia_store::settings::SettingValue::Int(value)) => value,
        _ => 200,
    };
    eunomia_store::library_versions::record_variant(
        tx,
        &variant.id,
        eunomia_store::library_versions::VariantOp::Create,
        &eunomia_core::version_snapshot::variant_snapshot(&variant.name, &variant.content),
        &eunomia_core::version_snapshot::variant_snapshot_hash(&variant.name, &variant.content),
        max_revisions,
    )
    .map(|_| ())
    .map_err(|_| ())
}

/// Seeds one project and answers its row id.
fn seed_project(database: &crate::db::Db, n: usize) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create(tx, &project_sample(n))
            .map(|project| project.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project must work")
}

/// Creates instruction `n` with its main variant named `Main` (the
/// LIB-002 create-dialog shape, the create version included) and
/// returns both rows — the world every variant command starts from.
fn seed_instruction(database: &crate::db::Db, n: usize) -> (LibraryItem, Variant) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &NewLibraryItem::Instruction {
                title: &format!("Instruction {n}"),
            },
        )
        .map_err(seed_error)?;
        let main = eunomia_store::library::create_variant(
            tx,
            &NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content: &format!("main content {n}"),
                is_main: true,
            },
        )
        .map_err(seed_error)?;
        record_create_version(tx, &main).map_err(|()| seed_no_version(&main.id))?;
        Ok((item, main))
    }))
    .expect("seeding an instruction with its main variant must work")
}

/// The seed's version-recording failure as an `AppError` (a test bug:
/// the recording is over rows the seed just wrote).
fn seed_no_version(id: &str) -> crate::errors::AppError {
    crate::commands::rejected_field("id", id, "the seed could not record the create version")
}

/// Creates one more (non-main) variant of the instruction — the
/// editor's "Add variant" outcome, seeded straight through the store
/// with its create version.
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
        let variant = eunomia_store::library::create_variant(
            tx,
            &NewVariant {
                instruction_id: &instruction_id,
                name: &name,
                content: &content,
                is_main: false,
            },
        )
        .map_err(seed_error)?;
        record_create_version(tx, &variant).map_err(|()| seed_no_version(&variant.id))?;
        Ok(variant)
    }))
    .expect("seeding a variant must work")
}

/// Seeds one heading (the MAIN-001 counter-example: headings have no
/// variants).
fn seed_heading(database: &crate::db::Db) -> LibraryItem {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::create_item(
            tx,
            &NewLibraryItem::Heading {
                title: "Section",
                level: 2,
            },
        )
        .map_err(seed_error)
    }))
    .expect("seeding a heading must work")
}

/// Assigns `variant_id` of `instruction_id` to `project_id` (D-02:
/// one variant of one instruction per project).
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
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &instruction_id,
                variant_id: &variant_id,
            },
        )
        .map(|_| ())
        .map_err(seed_error)
    }))
    .expect("seeding an assignment must work")
}

/// Reads one stored variant row by id.
fn stored_variant(database: &crate::db::Db, id: &str) -> Option<Variant> {
    let id = id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(
        database.run(move |tx| eunomia_store::library::get_variant(tx, &id).map_err(seed_error)),
    )
    .expect("reading a variant must work")
}

/// How many variant rows the instruction carries.
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
    .expect("counting the variants must work")
}

/// The instruction's assignment rows: `(project_id, variant_id)` in
/// table order — the census behind "no assignment disappeared".
fn assignments_of(database: &crate::db::Db, instruction_id: &str) -> Vec<(String, String)> {
    let instruction_id = instruction_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let mut statement = tx
            .prepare(
                "SELECT project_id, variant_id FROM project_instructions
                   WHERE instruction_id = ?1 ORDER BY project_id",
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let rows = statement
            .query_map([&instruction_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(crate::db::sqlite_error_to_app_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(rows)
    }))
    .expect("reading the assignments must work")
}

/// The variant's recorded versions: `(rev_no, op)` in rev_no order —
/// the ALG-070/071 census.
fn revisions_of(database: &crate::db::Db, variant_id: &str) -> Vec<(i64, String)> {
    let variant_id = variant_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let mut statement = tx
            .prepare(
                "SELECT rev_no, op FROM revisions
                   WHERE entity_type = 'variant' AND entity_id = ?1
                   ORDER BY rev_no",
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let rows = statement
            .query_map([&variant_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(crate::db::sqlite_error_to_app_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(rows)
    }))
    .expect("reading the versions must work")
}

/// The whole `revisions` table's row count — the rollback census for
/// entities whose row never existed (a failed create has no id to
/// look up; its would-be version row would be an orphan).
fn revisions_total(database: &crate::db::Db) -> i64 {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row("SELECT COUNT(*) FROM revisions", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("counting the versions must work")
}

/// The variant snapshot JSON of the entity's latest version — the
/// ALG-070 fact the hash gates on.
fn last_snapshot_of(database: &crate::db::Db, variant_id: &str) -> String {
    let variant_id = variant_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row(
            "SELECT snapshot FROM revisions
               WHERE entity_type = 'variant' AND entity_id = ?1
               ORDER BY rev_no DESC LIMIT 1",
            [&variant_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("reading the last snapshot must work")
}

/// The whole journal, newest last (insertion order).
fn journal_of(database: &crate::db::Db) -> Vec<LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 1000)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("reading the journal must work")
}

/// The journal's `variant.*` actions, in order — the LOG-001…004
/// census for the library battery.
fn variant_actions_of(database: &crate::db::Db) -> Vec<&'static str> {
    journal_of(database)
        .into_iter()
        .filter(|entry| entry.action.as_str().starts_with("variant."))
        .map(|entry| entry.action.as_str())
        .collect()
}

/// How many of the instruction's ACTIVE variants carry the main flag
/// — the DB-010 census behind T-089's definition of done.
fn active_main_count(database: &crate::db::Db, instruction_id: &str) -> i64 {
    let instruction_id = instruction_id.to_owned();
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row(
            "SELECT COUNT(*) FROM instruction_variants
               WHERE instruction_id = ?1 AND is_main = 1 AND state = 'active'",
            [&instruction_id],
            |row| row.get(0),
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("counting the active mains must work")
}

// ---------------------------------------------------------------------------
// T-087 `variant_create`: both modes, the duplicate-name refusal
// ---------------------------------------------------------------------------

#[test]
fn variant_create_copies_the_main_variants_text_in_copy_main_mode() {
    // THE card criterion, mode one: "New variant as a copy of the
    // main". The copy is the main's CURRENT stored text read inside
    // the same transaction (D-189 reading 2) — pinned against BOTH
    // ways the wrong variant could be sourced: the instruction holds
    // a SECOND variant (so "any variant" is ambiguous), and the main
    // flag has MOVED to it with new text (so "the first variant" and
    // "the original main" are both wrong answers). The created row is
    // never main (DB-010's constructive half), the create is the
    // entity's first version (ALG-071), and the journal carries
    // `variant.created` with the variant's entity columns (LOG-001
    // …004).
    let (_temp, app) = variants_app();
    let (instruction, original_main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "stale text");
    let webview = main_webview(&app);
    invoke_variant_set_main(&webview, &second.id)
        .expect("the main flag moves to the second variant");
    invoke_variant_update(&webview, &second.id, None, Some("the new main text"))
        .expect("the new main's text changes");

    let created = invoke_variant_create(&webview, &instruction.id, "Copy", "copy_main")
        .expect("the copy-mode create must succeed");
    assert_eq!(created.instruction_id, instruction.id);
    assert_eq!(created.name, "Copy");
    assert_eq!(
        created.content, "the new main text",
        "the copy carries the CURRENT main's text — not the original main's \
         ({}) and not the stale first read ({})",
        original_main.content, "stale text"
    );
    assert_eq!(
        stored_variant(&database_of(&app), &second.id)
            .expect("the current main must still exist")
            .content,
        "the new main text",
        "the source is read, not moved"
    );
    assert!(!created.is_main, "a created variant is never the main one");
    assert_eq!(created.state, "active");
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "the instruction still has exactly one active main: the old one"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &created.id),
        vec![(1, "create".to_owned())],
        "the create is the entity's first version"
    );
    assert_eq!(
        last_snapshot_of(&database_of(&app), &created.id),
        "{\"name\":\"Copy\",\"text\":\"the new main text\"}",
        "the version snapshot stores the copied name and text"
    );
    let journal = journal_of(&database_of(&app));
    let created_entry = journal
        .iter()
        .find(|entry| entry.action.as_str() == "variant.created")
        .expect("the journal carries the create");
    assert_eq!(
        created_entry.entity,
        Some((
            eunomia_store::action_log::EntityType::Variant,
            created.id.clone()
        )),
        "the journal entry names the created variant"
    );
    assert_eq!(
        journal
            .iter()
            .filter(|entry| entry.action.as_str() == "variant.created")
            .count(),
        1,
        "exactly one create entry — no double journal"
    );
}

#[test]
fn variant_create_answers_an_empty_text_in_empty_mode() {
    // THE card criterion, mode two: an empty variant is legal and
    // assignable (LIB-014/015, D-13) — the empty snapshot is a
    // legitimate version (§9.8's own example).
    let (_temp, app) = variants_app();
    let (instruction, _) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let created = invoke_variant_create(&webview, &instruction.id, "Empty", "empty")
        .expect("the empty-mode create must succeed");
    assert_eq!(created.content, "", "the empty mode stores the empty text");
    assert_eq!(
        last_snapshot_of(&database_of(&app), &created.id),
        "{\"name\":\"Empty\",\"text\":\"\"}",
        "the empty snapshot is the version's own first record"
    );
}

#[test]
fn variant_create_refuses_a_duplicate_name_case_insensitively() {
    // THE card criterion: "name is unique within the instruction
    // case-insensitively" (§6.9, the `ux_variant_name` scope). The
    // refusal is the field-shaped `E_VALIDATION` envelope, and the
    // census proves nothing was written: no row, no version, no
    // journal entry.
    let (_temp, app) = variants_app();
    let (instruction, _) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let error = invoke_variant_create(&webview, &instruction.id, "MAIN", "empty")
        .expect_err("the NOCASE twin of `Main` must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("variant_name"));
    assert_eq!(
        variant_count(&database_of(&app), &instruction.id),
        1,
        "no second variant row was written"
    );
    assert_eq!(
        variant_actions_of(&database_of(&app)),
        Vec::<&str>::new(),
        "the refused create journaled nothing"
    );
}

#[test]
fn variant_create_validates_the_name_against_the_6_9_rules() {
    // §6.9's variant-name row through the command boundary: empty
    // after trimming, 101 characters, an inner line break, and NUL
    // (D-126) — each the `E_VALIDATION` field refusal, with the
    // measured facts riding `params` (the length, the index), never
    // the name itself (D-189 reading 7).
    let (_temp, app) = variants_app();
    let (instruction, _) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let empty = invoke_variant_create(&webview, &instruction.id, "   ", "empty")
        .expect_err("a name that trims to nothing must be refused");
    assert_eq!(empty["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(empty["params"]["field"], serde_json::json!("variant_name"));

    let too_long = invoke_variant_create(&webview, &instruction.id, &"a".repeat(101), "empty")
        .expect_err("a 101-character name must be refused");
    assert_eq!(too_long["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        too_long["params"]["field"],
        serde_json::json!("variant_name")
    );
    assert_eq!(too_long["params"]["length"], serde_json::json!("101"));

    let line_break = invoke_variant_create(&webview, &instruction.id, "a\nb", "empty")
        .expect_err("a two-line name must be refused");
    assert_eq!(line_break["params"]["index"], serde_json::json!("1"));

    let nul = invoke_variant_create(&webview, &instruction.id, "a\0b", "empty")
        .expect_err("a NUL inside the name must be refused");
    assert_eq!(nul["params"]["index"], serde_json::json!("1"));

    // The legal edge: 100 characters pass.
    let legal = invoke_variant_create(&webview, &instruction.id, &"a".repeat(100), "empty")
        .expect("a 100-character name is the legal edge");
    assert_eq!(legal.name, "a".repeat(100));

    // The trim is persisted (§6.9 "1–100 characters after trimming"):
    // a padded-but-legal name is stored trimmed.
    let padded = invoke_variant_create(&webview, &instruction.id, "  Padded  ", "empty")
        .expect("a padded legal name saves");
    assert_eq!(padded.name, "Padded", "the stored name is the trimmed form");
}

#[test]
fn variant_create_refuses_an_unknown_instruction_and_a_heading() {
    // The instruction-scoped refusals: an id that names no row, and a
    // heading's id (headings have no variants, MAIN-001) — both the
    // `instruction_id` field refusal, nothing written.
    let (_temp, app) = variants_app();
    let (_, _) = seed_instruction(&database_of(&app), 1);
    let heading = seed_heading(&database_of(&app));
    let webview = main_webview(&app);

    let unknown = invoke_variant_create(&webview, "no-such-instruction", "Copy", "empty")
        .expect_err("an unknown instruction id must be refused");
    assert_eq!(unknown["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        unknown["params"]["field"],
        serde_json::json!("instruction_id")
    );

    let heading_refusal = invoke_variant_create(&webview, &heading.id, "Copy", "empty")
        .expect_err("a heading has no variants to add to");
    assert_eq!(heading_refusal["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        heading_refusal["params"]["field"],
        serde_json::json!("instruction_id")
    );
    assert_eq!(
        variant_actions_of(&database_of(&app)),
        Vec::<&str>::new(),
        "neither refusal wrote anything"
    );
}

#[test]
fn variant_create_refuses_copy_main_when_no_active_main_exists() {
    // DB-010's constructive half, defensively: an instruction whose
    // main flag was cleared out-of-band has nothing to copy — the
    // refusal surfaces the broken invariant instead of guessing
    // (DB-017's posture, the store's `MainRequired` twin).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let main_id = main.id.clone();
    let database = database_of(&app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE instruction_variants SET is_main = 0 WHERE id = ?1",
            [&main.id],
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("tampering the main flag must work");
    let webview = main_webview(&app);

    let error = invoke_variant_create(&webview, &instruction.id, "Copy", "copy_main")
        .expect_err("the copy of a main that does not exist must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("instruction_id")
    );
    assert!(
        !stored_variant(&database_of(&app), &main_id)
            .expect("the tampered row must still exist")
            .is_main,
        "the tampered row is untouched — the refusal repaired nothing (DB-017)"
    );
}

// ---------------------------------------------------------------------------
// T-088 `variant_update`: every save rule, the version gate
// ---------------------------------------------------------------------------

#[test]
fn variant_update_folds_crlf_to_lf_on_save() {
    // §6.9's save-time fold (D-076): the stored form is the folded
    // form, and the answer's `content` is the stored form.
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let updated = invoke_variant_update(
        &webview,
        &main.id,
        None,
        Some("Be brief.\r\nSecond line\r\n"),
    )
    .expect("the CRLF save must succeed");
    assert_eq!(
        updated.content, "Be brief.\nSecond line\n",
        "the stored text is the folded form"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "a text save never touches the main flag"
    );
}

#[test]
fn variant_update_refuses_nul_with_the_field_validation_envelope() {
    // §10's split (D-189 reading 3): NUL is field validation —
    // `E_VALIDATION`, the index riding `params`.
    let (_temp, app) = variants_app();
    let (_, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let error = invoke_variant_update(&webview, &main.id, None, Some("a\0b"))
        .expect_err("a NUL inside the text must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("variant_text"));
    assert_eq!(error["params"]["index"], serde_json::json!("1"));
    assert_eq!(
        stored_variant(&database_of(&app), &main.id)
            .expect("the row must still exist")
            .content,
        "main content 1",
        "the refused save changed nothing"
    );
}

#[test]
fn variant_update_refuses_a_reserved_marker_line_with_e_content_reserved() {
    // §10's own row: "The text contains a scheme B marker line" —
    // `E_CONTENT_RESERVED`, with the built form's 1-based line and
    // the D-19 literal riding `params` (DB-016, D-15's registry).
    let (_temp, app) = variants_app();
    let (_, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let error = invoke_variant_update(
        &webview,
        &main.id,
        None,
        Some("x\n<!-- EUNOMIASYNC:END -->"),
    )
    .expect_err("the reserved marker line must be refused");
    assert_eq!(error["code"], serde_json::json!("E_CONTENT_RESERVED"));
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("variant_text"),
        "the refusal names the field (§7.3's aria-invalid rule)"
    );
    assert_eq!(error["params"]["line"], serde_json::json!("2"));
    assert_eq!(
        error["params"]["marker"],
        serde_json::json!("<!-- EUNOMIASYNC:END -->")
    );
    assert_eq!(
        stored_variant(&database_of(&app), &main.id)
            .expect("the row must still exist")
            .content,
        "main content 1",
        "the refused save changed nothing"
    );

    // A marker hidden behind a leading BOM is caught at the BUILT
    // form's own line number (the validator scans
    // `normalize::variant_text`'s output — D-076's reserved-line
    // ruling), and the refusal still names the field.
    let hidden = invoke_variant_update(
        &webview,
        &main.id,
        None,
        Some("\u{feff}<!-- EUNOMIASYNC:BEGIN -->"),
    )
    .expect_err("the marker behind the leading BOM must be refused");
    assert_eq!(hidden["code"], serde_json::json!("E_CONTENT_RESERVED"));
    assert_eq!(hidden["params"]["field"], serde_json::json!("variant_text"));
    assert_eq!(
        hidden["params"]["line"],
        serde_json::json!("1"),
        "the built form dropped the BOM, so the marker is its first line"
    );
    assert_eq!(
        hidden["params"]["marker"],
        serde_json::json!("<!-- EUNOMIASYNC:BEGIN -->")
    );
}

#[test]
fn variant_update_enforces_the_262144_character_limit_at_both_edges() {
    // DB-006 at both edges — the large-input test of project rule 6:
    // exactly 262,144 characters save (and version), one more is the
    // field refusal naming the measured length.
    let (_temp, app) = variants_app();
    let (_, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let at_limit = invoke_variant_update(&webview, &main.id, None, Some(&"a".repeat(262_144)))
        .expect("the text at the exact limit saves");
    assert_eq!(at_limit.content.len(), 262_144);
    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        vec![(1, "create".to_owned()), (2, "update".to_owned())],
        "the limit-length save is a real change: one new version"
    );

    let over_limit = invoke_variant_update(&webview, &main.id, None, Some(&"a".repeat(262_145)))
        .expect_err("one character over the limit is refused");
    assert_eq!(over_limit["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        over_limit["params"]["field"],
        serde_json::json!("variant_text")
    );
    assert_eq!(over_limit["params"]["length"], serde_json::json!("262145"));
    assert_eq!(
        revisions_of(&database_of(&app), &main.id).len(),
        2,
        "the refused save wrote no version"
    );
}

#[test]
fn variant_update_without_real_changes_writes_no_version_and_no_journal() {
    // THE card criterion: "a save without changes does not create a
    // version" (ALG-070's hash gate, D-16) — and the journal stays
    // silent too (the D-051 no-false-claim rule, D-189 reading 5).
    // Both no-op shapes: the fieldless call, and the same values
    // re-sent.
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let fieldless = invoke_variant_update(&webview, &main.id, None, None)
        .expect("the fieldless save is the verified no-op");
    assert_eq!(fieldless.name, "Main");
    assert_eq!(fieldless.content, main.content);

    let same_values = invoke_variant_update(
        &webview,
        &main.id,
        Some("Main"),
        Some(main.content.as_str()),
    )
    .expect("re-sending the stored values is the same no-op");
    assert_eq!(same_values.content, main.content);

    // The SEMANTIC no-op (ALG-070's hash gate over the STORED form):
    // a text whose folded form equals the stored one — its CRLF twin —
    // is not a real change either.
    let stored = invoke_variant_update(&webview, &main.id, None, Some("folded\ntext\n"))
        .expect("the LF-form save lands");
    assert_eq!(stored.content, "folded\ntext\n");
    let crlf_twin = invoke_variant_update(&webview, &main.id, None, Some("folded\r\ntext\r\n"))
        .expect("the CRLF twin of the stored text is the same no-op");
    assert_eq!(
        crlf_twin.content, "folded\ntext\n",
        "the answer is the stored (folded) form"
    );

    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        // The seed's own create version plus the ONE real change (the
        // LF-form save), nothing more.
        vec![(1, "create".to_owned()), (2, "update".to_owned())],
        "neither no-op save wrote a version"
    );
    assert_eq!(
        variant_actions_of(&database_of(&app)),
        vec!["variant.changed"],
        "only the one real change journaled a variant action"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "the world is unchanged"
    );
}

#[test]
fn variant_update_records_one_version_and_one_journal_entry_per_real_change() {
    // ALG-070/071 through the command: a save that changes both
    // fields is ONE version (rev 2, op `update`) whose snapshot
    // carries the new name and text, and ONE `variant.changed` entry
    // naming the variant.
    let (_temp, app) = variants_app();
    let (_, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let updated = invoke_variant_update(&webview, &main.id, Some("Renamed"), Some("new text"))
        .expect("the real change must save");
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.content, "new text");
    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        vec![(1, "create".to_owned()), (2, "update".to_owned())],
        "one new version for the one real change"
    );
    assert_eq!(
        last_snapshot_of(&database_of(&app), &main.id),
        "{\"name\":\"Renamed\",\"text\":\"new text\"}",
        "the version's snapshot is the saved pair"
    );
    let journal = journal_of(&database_of(&app));
    let entry = journal
        .iter()
        .find(|entry| entry.action.as_str() == "variant.changed")
        .expect("the real change is journaled");
    assert_eq!(
        entry.entity,
        Some((
            eunomia_store::action_log::EntityType::Variant,
            main.id.clone()
        )),
        "the journal entry names the edited variant"
    );
    assert_eq!(
        journal
            .iter()
            .filter(|entry| entry.action.as_str() == "variant.changed")
            .count(),
        1,
        "exactly one changed entry for the one real change — no double journal"
    );

    // The trim is persisted on the rename path too: a padded legal
    // name is stored trimmed (§6.9).
    let padded_rename = invoke_variant_update(&webview, &main.id, Some("  Padded  "), None)
        .expect("a padded legal rename saves");
    assert_eq!(
        padded_rename.name, "Padded",
        "the stored name is the trimmed form"
    );
}

#[test]
fn variant_update_refuses_a_duplicate_name_and_a_soft_deleted_variant() {
    // The rename rules: the NOCASE twin of an ACTIVE sibling is
    // refused (the store's `DuplicateVariantName`), keeping the
    // variant's own name is legal (the self-exemption); a
    // soft-deleted variant cannot be edited (restore it first).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let webview = main_webview(&app);

    let error = invoke_variant_update(&webview, &main.id, Some("SECOND"), None)
        .expect_err("the NOCASE twin of the sibling must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("variant_name"));
    assert_eq!(
        stored_variant(&database_of(&app), &main.id)
            .expect("the row must still exist")
            .name,
        "Main",
        "the refused rename changed nothing"
    );

    let own_name = invoke_variant_update(&webview, &main.id, Some("Main"), None)
        .expect("keeping the own name is the no-op, not a refusal");
    assert_eq!(own_name.name, "Main");

    // The soft-deleted arm: delete the sibling, then try to edit it.
    invoke_variant_delete(&webview, &second.id, None)
        .expect("the delete of a non-main variant defaults to the main");
    let deleted = invoke_variant_update(&webview, &second.id, Some("Late"), None)
        .expect_err("a soft-deleted variant cannot be edited");
    assert_eq!(deleted["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(deleted["params"]["field"], serde_json::json!("id"));
}

#[test]
fn variant_update_refuses_an_unknown_id() {
    let (_temp, app) = variants_app();
    let (_, _) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let error = invoke_variant_update(&webview, "no-such-variant", Some("Late"), None)
        .expect_err("an unknown variant id must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("id"));
}

// ---------------------------------------------------------------------------
// T-089 `variant_set_main`: exactly one main among the actives
// ---------------------------------------------------------------------------

#[test]
fn variant_set_main_leaves_exactly_one_active_main() {
    // THE card criterion: "after the change the instruction has
    // exactly one main variant" (DB-010). The flag change is
    // metadata: NO version (D-16 — the main flag is not
    // snapshot-visible), but the journal's `variant.changed` entry
    // (the action's own "main choice" member).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let third = seed_variant(&database_of(&app), &instruction.id, "Third", "third text");
    let webview = main_webview(&app);

    let promoted =
        invoke_variant_set_main(&webview, &second.id).expect("the promotion must succeed");
    assert!(promoted.is_main);
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "the instruction has exactly one active main after the change"
    );
    assert!(
        !stored_variant(&database_of(&app), &main.id)
            .expect("the old main must still exist")
            .is_main,
        "the previous main was demoted"
    );
    assert!(
        !stored_variant(&database_of(&app), &third.id)
            .expect("the untouched sibling must still exist")
            .is_main,
        "an untouched sibling never gains the flag"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &second.id),
        vec![(1, "create".to_owned())],
        "the promotion wrote no version (D-16: the flag is not snapshot-visible)"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        vec![(1, "create".to_owned())],
        "the demotion wrote no version either"
    );
    let journal = journal_of(&database_of(&app));
    let entry = journal
        .iter()
        .find(|entry| entry.action.as_str() == "variant.changed")
        .expect("the promotion is journaled");
    assert_eq!(
        entry.entity,
        Some((
            eunomia_store::action_log::EntityType::Variant,
            second.id.clone()
        )),
        "the journal entry names the promoted variant"
    );
    assert_eq!(
        journal
            .iter()
            .filter(|entry| entry.action.as_str() == "variant.changed")
            .count(),
        1,
        "exactly one changed entry for the one promotion — no double journal"
    );
}

#[test]
fn variant_set_main_on_the_current_main_is_a_verified_no_op() {
    // The replayed "Make main" (the UI's disabled button's defensive
    // twin): nothing is written, journaled, or versioned.
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let answer = invoke_variant_set_main(&webview, &main.id)
        .expect("re-promoting the current main is the no-op");
    assert!(answer.is_main);
    assert_eq!(active_main_count(&database_of(&app), &instruction.id), 1);
    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        vec![(1, "create".to_owned())],
        "the no-op wrote no version"
    );
    assert_eq!(
        variant_actions_of(&database_of(&app)),
        Vec::<&str>::new(),
        "the no-op journaled nothing"
    );
}

#[test]
fn variant_set_main_refuses_a_soft_deleted_variant_and_an_unknown_id() {
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let webview = main_webview(&app);
    invoke_variant_delete(&webview, &second.id, None)
        .expect("the delete of a non-main variant defaults to the main");

    let deleted = invoke_variant_set_main(&webview, &second.id)
        .expect_err("a soft-deleted variant cannot become main");
    assert_eq!(deleted["code"], serde_json::json!("E_VALIDATION"));
    assert!(
        stored_variant(&database_of(&app), &main.id)
            .expect("the main must still exist")
            .is_main,
        "the main flag never moved"
    );

    let unknown = invoke_variant_set_main(&webview, "no-such-variant")
        .expect_err("an unknown variant id must be refused");
    assert_eq!(unknown["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(unknown["params"]["field"], serde_json::json!("id"));
}

// ---------------------------------------------------------------------------
// T-090 `variant_delete`: assignments preserved, the last-variant
// refusal, the named successor, one transaction
// ---------------------------------------------------------------------------

#[test]
fn variant_delete_moves_every_assignment_to_the_default_main() {
    // THE card criterion: "after the deletion not a single assignment
    // disappeared" (LIB-023, DB-012, D-125's C3) — the default
    // replacement is the instruction's main variant, the moved count
    // rides the journal's `Details::Bulk`, and the delete is the
    // `delete` version (ALG-071).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let project_one = seed_project(&database_of(&app), 1);
    let project_two = seed_project(&database_of(&app), 2);
    seed_assignment(
        &database_of(&app),
        &project_one,
        &instruction.id,
        &second.id,
    );
    seed_assignment(
        &database_of(&app),
        &project_two,
        &instruction.id,
        &second.id,
    );
    let before = assignments_of(&database_of(&app), &instruction.id);
    assert_eq!(before.len(), 2);
    let webview = main_webview(&app);

    let deleted = invoke_variant_delete(&webview, &second.id, None)
        .expect("the default-replacement delete must succeed");
    assert_eq!(deleted.state, "deleted");
    assert!(
        deleted.deleted_at.is_some(),
        "the soft delete stamps the time"
    );
    assert!(
        !deleted.is_main,
        "a deleted row never carries the main flag"
    );
    let after = assignments_of(&database_of(&app), &instruction.id);
    assert_eq!(
        after.len(),
        before.len(),
        "not a single assignment disappeared"
    );
    assert!(
        after.iter().all(|(_, variant_id)| variant_id == &main.id),
        "every assignment now points at the main variant: {after:?}"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "DB-010 holds after the delete"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &second.id),
        vec![(1, "create".to_owned()), (2, "delete".to_owned())],
        "the delete is a version (ALG-071's op vocabulary)"
    );
    let journal = journal_of(&database_of(&app));
    let entry = journal
        .iter()
        .find(|entry| entry.action.as_str() == "variant.deleted")
        .expect("the delete is journaled");
    assert_eq!(
        entry.entity,
        Some((
            eunomia_store::action_log::EntityType::Variant,
            second.id.clone()
        )),
        "the journal entry names the deleted variant"
    );
    assert_eq!(
        entry.details,
        Some(serde_json::json!({ "changed": 2 })),
        "the moved-assignment count rides Details::Bulk (LIB-023's number)"
    );
}

#[test]
fn variant_delete_refuses_the_last_active_variant_even_with_a_replacement() {
    // THE card criterion: "deleting the last variant is rejected"
    // (LIB-025) — the absolute refusal, whatever the replacement
    // names (the store runs the check first; the spec keeps it in
    // force behind the UI's disabled button).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let unnamed = invoke_variant_delete(&webview, &main.id, None)
        .expect_err("the last active variant cannot be deleted");
    assert_eq!(unnamed["code"], serde_json::json!("E_VALIDATION"));

    let named = invoke_variant_delete(&webview, &main.id, Some(&main.id))
        .expect_err("even a named replacement cannot delete the last variant");
    assert_eq!(named["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        named["params"]["field"],
        serde_json::json!("id"),
        "the LIB-025 check runs FIRST: the refusal is the last-variant one \
         (field `id`), not the replacement-shape one (field `replacement`)"
    );

    let variant = stored_variant(&database_of(&app), &main.id).expect("the row must still exist");
    assert_eq!(variant.state.as_str(), "active");
    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        vec![(1, "create".to_owned())],
        "the refused delete wrote no version"
    );
    assert_eq!(
        variant_actions_of(&database_of(&app)),
        Vec::<&str>::new(),
        "the refused delete journaled nothing"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "the world is unchanged"
    );
}

#[test]
fn variant_delete_of_the_main_requires_a_named_successor() {
    // The card's "when deleting the main a new main is mandatory":
    // without a successor the refusal names the replacement field;
    // with one, the successor takes over the assignments AND the main
    // flag (the store's promotion), keeping DB-010 at exactly one.
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let project = seed_project(&database_of(&app), 1);
    seed_assignment(&database_of(&app), &project, &instruction.id, &main.id);
    let webview = main_webview(&app);

    let refusal = invoke_variant_delete(&webview, &main.id, None)
        .expect_err("deleting the main without a successor must be refused");
    assert_eq!(refusal["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(refusal["params"]["field"], serde_json::json!("replacement"));

    let deleted = invoke_variant_delete(&webview, &main.id, Some(&second.id))
        .expect("the named successor deletes the main");
    assert_eq!(deleted.state, "deleted");
    assert!(
        stored_variant(&database_of(&app), &second.id)
            .expect("the successor must still exist")
            .is_main,
        "the successor was promoted to main"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "exactly one active main remains"
    );
    let assignments = assignments_of(&database_of(&app), &instruction.id);
    assert_eq!(assignments.len(), 1, "the assignment survived the delete");
    assert_eq!(
        assignments[0].1, second.id,
        "the assignment moved to the successor"
    );
}

#[test]
fn variant_delete_refuses_a_replacement_from_another_instruction() {
    // The replacement must be an ACTIVE variant of the SAME
    // instruction (the store's `ReplacementInvalid`).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let (other_instruction, _) = seed_instruction(&database_of(&app), 2);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let foreign = seed_variant(&database_of(&app), &other_instruction.id, "Foreign", "text");
    let webview = main_webview(&app);

    let error = invoke_variant_delete(&webview, &main.id, Some(&foreign.id))
        .expect_err("a foreign instruction's variant cannot be the replacement");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("replacement"));
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(foreign.id),
        "the id the caller sent rides the refusal (an id, not prose)"
    );
    assert_eq!(
        stored_variant(&database_of(&app), &main.id)
            .expect("the row must still exist")
            .state
            .as_str(),
        "active",
        "nothing was deleted"
    );
    assert_eq!(
        stored_variant(&database_of(&app), &second.id)
            .expect("the sibling must still exist")
            .state
            .as_str(),
        "active"
    );
}

#[test]
fn variant_delete_rolls_back_everything_when_a_later_write_fails() {
    // The one-transaction DoD (§6.9's tail "an entity is never saved
    // partly"): a trigger refuses the JOURNAL insert — after the
    // assignments moved and the variant row was soft-deleted inside
    // the transaction — and the census proves everything rolled back
    // with it (the `scan_tests.rs` staged-trigger shape).
    let (_temp, app) = variants_app();
    let (instruction, _main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let project = seed_project(&database_of(&app), 1);
    seed_assignment(&database_of(&app), &project, &instruction.id, &second.id);
    let database = database_of(&app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_variant_deleted_journal BEFORE INSERT ON action_log
             WHEN NEW.action = 'variant.deleted'
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");
    let webview = main_webview(&app);

    let error = invoke_variant_delete(&webview, &second.id, None)
        .expect_err("the staged abort must fail the whole delete");
    assert_eq!(
        error["code"],
        serde_json::json!("E_VALIDATION"),
        "the constraint abort maps through the base classifier: {error}"
    );
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation"),
        "the error is the staged trigger's constraint abort, not an unrelated failure"
    );
    assert_eq!(
        stored_variant(&database_of(&app), &second.id)
            .expect("the row must still exist")
            .state
            .as_str(),
        "active",
        "the soft delete rolled back"
    );
    let assignments = assignments_of(&database_of(&app), &instruction.id);
    assert_eq!(
        assignments.len(),
        1,
        "the assignment row survived the aborted transaction"
    );
    assert_eq!(
        assignments[0].1, second.id,
        "the assignment still points at the variant the delete would have removed"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &second.id),
        vec![(1, "create".to_owned())],
        "the delete version rolled back with the rest"
    );
}

#[test]
fn every_variant_command_rolls_back_when_its_journal_write_fails() {
    // The one-transaction claim for the REST of the group (the module
    // docs' "the row write, the version row and the journal entry
    // commit or roll back together"): a trigger refuses each
    // command's journal insert, and the census proves nothing of its
    // own write survived — the store row, the version row, the main
    // flag, the state. (`variant_delete` has its own dedicated
    // staged-trigger test above.)
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let third = seed_variant(&database_of(&app), &instruction.id, "Third", "third text");
    let webview = main_webview(&app);
    // The restore arm needs a soft-deleted variant BEFORE the trigger
    // goes up (the delete's own journal entry must land first).
    invoke_variant_delete(&webview, &third.id, None).expect("the pre-trigger delete must succeed");
    let versions_before = revisions_of(&database_of(&app), &second.id);
    let variants_before = variant_count(&database_of(&app), &instruction.id);
    let revisions_total_before = revisions_total(&database_of(&app));
    let database = database_of(&app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_variant_journal BEFORE INSERT ON action_log
             WHEN NEW.action IN ('variant.created', 'variant.changed', 'variant.restored')
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");

    // create: the insert rolls back with its journal entry AND its
    // version row.
    let create_error = invoke_variant_create(&webview, &instruction.id, "Late", "empty")
        .expect_err("the create must fail when its journal write fails");
    assert_eq!(create_error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        create_error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation"),
        "the error is the staged trigger's constraint abort, not an unrelated failure"
    );
    assert_eq!(
        variant_count(&database_of(&app), &instruction.id),
        variants_before,
        "the created row rolled back"
    );
    assert_eq!(
        revisions_total(&database_of(&app)),
        revisions_total_before,
        "the create's version row rolled back with the rest"
    );

    // update: the row write (and the version) rolls back.
    let update_error = invoke_variant_update(&webview, &main.id, Some("Renamed"), Some("new"))
        .expect_err("the update must fail when its journal write fails");
    assert_eq!(update_error["code"], serde_json::json!("E_VALIDATION"));
    let stored_main =
        stored_variant(&database_of(&app), &main.id).expect("the row must still exist");
    assert_eq!(stored_main.name, "Main", "the rename rolled back");
    assert_eq!(
        stored_main.content, "main content 1",
        "the text rolled back"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        vec![(1, "create".to_owned())],
        "the update version rolled back"
    );

    // set_main: the demote/promote pair rolls back.
    let promote_error = invoke_variant_set_main(&webview, &second.id)
        .expect_err("the promotion must fail when its journal write fails");
    assert_eq!(promote_error["code"], serde_json::json!("E_VALIDATION"));
    assert!(
        stored_variant(&database_of(&app), &main.id)
            .expect("the main must still exist")
            .is_main,
        "the main flag never moved"
    );
    assert!(
        !stored_variant(&database_of(&app), &second.id)
            .expect("the sibling must still exist")
            .is_main,
        "the sibling never gained the flag"
    );

    // restore: the state change (and the version) rolls back.
    let restore_error = invoke_variant_restore(&webview, &third.id)
        .expect_err("the restore must fail when its journal write fails");
    assert_eq!(restore_error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        stored_variant(&database_of(&app), &third.id)
            .expect("the row must still exist")
            .state
            .as_str(),
        "deleted",
        "the restore rolled back"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &third.id),
        vec![(1, "create".to_owned()), (2, "delete".to_owned())],
        "the restore's version row rolled back — the ladder stops at the delete"
    );
    assert_eq!(
        revisions_total(&database_of(&app)),
        revisions_total_before,
        "no arm of the table left any version row behind"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &second.id),
        versions_before,
        "an unrelated entity's versions are untouched by every arm"
    );
}

#[test]
fn variant_delete_on_a_deleted_variant_is_a_verified_no_op() {
    // The idempotent re-send (a stale screen's double click): the row
    // comes back unchanged, no second journal entry, no second
    // version.
    let (_temp, app) = variants_app();
    let (instruction, _main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let webview = main_webview(&app);
    invoke_variant_delete(&webview, &second.id, None).expect("the first delete must succeed");
    let versions_after_first = revisions_of(&database_of(&app), &second.id);

    let again = invoke_variant_delete(&webview, &second.id, None)
        .expect("the re-send converges, never errs");
    assert_eq!(again.state, "deleted");
    assert_eq!(
        revisions_of(&database_of(&app), &second.id),
        versions_after_first,
        "the re-send wrote no second version"
    );
    assert_eq!(
        variant_actions_of(&database_of(&app)),
        vec!["variant.deleted"],
        "the re-send journaled no second entry"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "the main never moved"
    );
}

#[test]
fn variant_delete_refuses_an_unknown_id() {
    let (_temp, app) = variants_app();
    let (_, _) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let error = invoke_variant_delete(&webview, "no-such-variant", None)
        .expect_err("an unknown variant id must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("id"));
}

// ---------------------------------------------------------------------------
// T-091 `variant_restore`: active again, the name-conflict refusal
// ---------------------------------------------------------------------------

#[test]
fn variant_restore_returns_a_soft_deleted_variant_to_active() {
    // THE card criterion: "the variant is active again" — state
    // active, `deleted_at` cleared, a `restore` version, a
    // `variant.restored` journal entry — and the restored variant
    // comes back WITHOUT its old main flag (DB-010 keeps exactly one
    // active main; the delete cleared it).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let webview = main_webview(&app);
    invoke_variant_delete(&webview, &second.id, None).expect("the delete must succeed");

    let restored = invoke_variant_restore(&webview, &second.id).expect("the restore must succeed");
    assert_eq!(restored.state, "active");
    assert_eq!(restored.deleted_at, None, "the delete stamp is cleared");
    assert_eq!(restored.name, "Second");
    assert!(!restored.is_main, "the restored variant is not main again");
    assert_eq!(
        revisions_of(&database_of(&app), &second.id),
        vec![
            (1, "create".to_owned()),
            (2, "delete".to_owned()),
            (3, "restore".to_owned())
        ],
        "the restore is the third version (ALG-071's op vocabulary)"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "the main variant kept its flag through the sibling's round trip"
    );
    assert!(
        stored_variant(&database_of(&app), &main.id)
            .expect("the main must still exist")
            .is_main,
        "the untouched main variant is still the main"
    );
    let journal = journal_of(&database_of(&app));
    let entry = journal
        .iter()
        .find(|entry| entry.action.as_str() == "variant.restored")
        .expect("the restore is journaled");
    assert_eq!(
        entry.entity,
        Some((
            eunomia_store::action_log::EntityType::Variant,
            second.id.clone()
        )),
        "the journal entry names the restored variant"
    );
    assert_eq!(
        journal
            .iter()
            .filter(|entry| entry.action.as_str() == "variant.restored")
            .count(),
        1,
        "exactly one restored entry for the one restore — no double journal"
    );
}

#[test]
fn variant_restore_refuses_a_name_reused_by_an_active_twin() {
    // THE card criterion: "the name does not conflict with active
    // ones (otherwise E_VALIDATION)" — the NOCASE index covers active
    // rows only, so the freed name can be taken while the variant is
    // deleted; the restore then collides and nothing changes.
    let (_temp, app) = variants_app();
    let (instruction, _) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let webview = main_webview(&app);
    invoke_variant_delete(&webview, &second.id, None).expect("the delete must succeed");
    let twin = invoke_variant_create(&webview, &instruction.id, "SECOND", "empty")
        .expect("the freed name is legal for a NEW variant while the owner is deleted");

    let error = invoke_variant_restore(&webview, &second.id)
        .expect_err("the restore onto the reused name must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("variant_name"));
    assert_eq!(
        stored_variant(&database_of(&app), &second.id)
            .expect("the row must still exist")
            .state
            .as_str(),
        "deleted",
        "the refused restore changed nothing"
    );
    assert_eq!(
        stored_variant(&database_of(&app), &twin.id)
            .expect("the twin must still exist")
            .state
            .as_str(),
        "active",
        "the active twin is untouched too"
    );
    assert_eq!(
        revisions_of(&database_of(&app), &second.id),
        vec![(1, "create".to_owned()), (2, "delete".to_owned())],
        "the refused restore wrote no version"
    );
}

#[test]
fn variant_restore_on_an_active_variant_is_a_verified_no_op() {
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let answer = invoke_variant_restore(&webview, &main.id)
        .expect("restoring an active variant is the no-op");
    assert_eq!(answer.state, "active");
    assert_eq!(
        revisions_of(&database_of(&app), &main.id),
        vec![(1, "create".to_owned())],
        "the no-op wrote no version"
    );
    assert_eq!(
        variant_actions_of(&database_of(&app)),
        Vec::<&str>::new(),
        "the no-op journaled nothing"
    );
    assert_eq!(
        active_main_count(&database_of(&app), &instruction.id),
        1,
        "the main never moved"
    );
}

#[test]
fn variant_restore_refuses_an_unknown_id() {
    let (_temp, app) = variants_app();
    let (_, _) = seed_instruction(&database_of(&app), 1);
    let webview = main_webview(&app);

    let error = invoke_variant_restore(&webview, "no-such-variant")
        .expect_err("an unknown variant id must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["params"]["field"], serde_json::json!("id"));
}

// ---------------------------------------------------------------------------
// Cross-cutting pins
// ---------------------------------------------------------------------------

#[test]
fn library_edits_are_allowed_while_a_sync_runs() {
    // §5.4's SYS-005 sentence, pinned at the command boundary for the
    // WHOLE group: "Library edits during a sync are allowed; they do
    // not go into the current run and stay dirty" — so while the flag
    // is up, every variant command SUCCEEDS (no `E_SYNC_RUNNING` gate
    // here, unlike the project-changing commands).
    let (_temp, app) = variants_app();
    let (instruction, main) = seed_instruction(&database_of(&app), 1);
    let second = seed_variant(&database_of(&app), &instruction.id, "Second", "second text");
    let webview = main_webview(&app);
    app.manage(crate::sync_running::SyncRunning::new());
    let guard =
        crate::commands::begin_sync(app.handle()).expect("the test's flag must be free to take");

    let created = invoke_variant_create(&webview, &instruction.id, "During sync", "empty")
        .expect("the create is allowed while the sync runs");
    assert_eq!(created.name, "During sync");
    let updated = invoke_variant_update(&webview, &main.id, None, Some("sync-time text"))
        .expect("the save is allowed while the sync runs");
    assert_eq!(updated.content, "sync-time text");
    let promoted = invoke_variant_set_main(&webview, &second.id)
        .expect("the promotion is allowed while the sync runs");
    assert!(promoted.is_main);
    // `second` is main now (promoted above), so its delete names the
    // successor — the card's "deleting the main requires a new main".
    let deleted = invoke_variant_delete(&webview, &second.id, Some(&main.id))
        .expect("the delete is allowed while the sync runs");
    assert_eq!(deleted.state, "deleted");
    let restored = invoke_variant_restore(&webview, &second.id)
        .expect("the restore is allowed while the sync runs");
    assert_eq!(restored.state, "active");

    drop(guard);
}

#[test]
fn the_variant_commands_answer_honestly_without_a_database() {
    // Recovery mode (no `Db` managed): the honest `managed_db`
    // fallback for every command of the group — never a pretend
    // answer about a database that is not running. The invalid-name
    // arm also pins the gate order (the module docs' "§6.9 field
    // validation runs before the database is even resolved"): the
    // field refusal answers whatever the database state is.
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::variants::variant_create,
            crate::variants::variant_delete,
            crate::variants::variant_restore,
            crate::variants::variant_set_main,
            crate::variants::variant_update
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let webview = main_webview(&app);

    let bodies = [
        serde_json::json!({
            "instructionId": "any",
            "name": "Any",
            "source": "empty",
        }),
        serde_json::json!({ "id": "any", "name": null, "content": null }),
        serde_json::json!({ "id": "any" }),
        serde_json::json!({ "id": "any", "replacement": null }),
        serde_json::json!({ "id": "any" }),
    ];
    let commands = [
        "variant_create",
        "variant_update",
        "variant_set_main",
        "variant_delete",
        "variant_restore",
    ];
    for (command, body) in commands.iter().zip(bodies) {
        let error: Result<VariantRecord, serde_json::Value> = invoke(&webview, command, body);
        let error = error.expect_err("without a database every command refuses");
        assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{command}");
        assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
    }

    // The gate order: an INVALID name refuses with the field envelope
    // even with no database behind the command.
    let invalid = invoke_variant_create(&webview, "any", "   ", "empty")
        .expect_err("the §6.9 refusal precedes the database resolution");
    assert_eq!(invalid["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        invalid["params"]["field"],
        serde_json::json!("variant_name")
    );
}

#[test]
fn the_five_variant_commands_are_registered_with_the_specta_builder() {
    // The §7.10 Library-module names, each registered exactly once —
    // the D-034 wiring-pin rationale: without this, a dropped
    // registration would silently rewrite the committed bindings
    // without the command.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for name in [
        "variants::variant_create",
        "variants::variant_update",
        "variants::variant_set_main",
        "variants::variant_delete",
        "variants::variant_restore",
    ] {
        assert_eq!(
            lib.matches(name).count(),
            1,
            "lib.rs must register `{name}` exactly once (§7.10, D-009)"
        );
    }
}
