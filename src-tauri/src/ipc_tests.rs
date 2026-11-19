//! IPC round-trip tests for the recovery commands (T-024) and the
//! `activity_list` command (T-027) against the Tauri mock runtime —
//! the harness shape the removed `greet` tests used (D-037): a real
//! invoke through `tauri::test::get_ipc_response`, no windowing
//! system involved. Lives inside the lib crate because
//! `tauri::generate_handler!` requires the command's generated helper
//! macros, which are crate-private.
//!
//! The commands are runtime-generic, so the mock registration pins
//! `tauri::test::MockRuntime` — the same commands the app registers
//! for `tauri::Wry` in `specta_builder`.
//!
//! `app_open_data_folder` is deliberately NOT driven through the mock
//! IPC: its opener call spawns the real OS file manager (there is no
//! mock seam inside the plugin), which a test must not do. The
//! command's pure halves (path resolution, error mapping) are covered
//! by `commands.rs`'s own suite and the wiring pin in
//! `recovery_tests.rs`; the recorded untested seam is the opener call
//! itself (manual smoke runs).
//!
//! The three tests that invoke the destructive commands share the
//! process-wide `RECOVERY_OPERATION` gate with each other (tests run
//! in parallel inside one binary), so they serialize on their own
//! lock: a locked-out call would answer "recovery already in
//! progress" — the gate working as designed, but not what these tests
//! exist to observe.
//!
//! The `activity_list` tests build the healthy-app state instead (a
//! real `Db` managed over a temp root, seeded through the `Db::run`
//! executor the command itself uses) and pin the command's IPC
//! contract: the wire DTO, LOG-005's user-by-default visibility (the
//! card's acceptance criterion), the §7.10 error envelope of a
//! rejected filter, and the honest answer when no database is
//! running.

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use crate::data_dirs;

/// Serializes this file's destructive-command tests against each other
/// (see the module docs): the production gate is `commands::
/// RECOVERY_OPERATION`, which parallel test bodies would contend on.
static IPC_RECOVERY_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A mock app with the recovery commands registered and the
/// `DataRoot` state managed at a fresh temp root — the recovery-mode
/// state the `setup` hook produces (layout up, `DataRoot` published,
/// no `Db`).
fn recovery_mode_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::db_backups_list,
            crate::commands::db_restore_backup,
            crate::commands::db_recreate,
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(data_dirs::DataRoot(temp.path().to_path_buf()));
    (temp, app)
}

/// One invoke request for `cmd` with a JSON body (the same shape the
/// removed greet tests used; mirrors tauri's own `tauri::test`
/// doctest).
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

/// The mock `main` webview window the requests are delivered through
/// (the same construction the removed greet tests used).
fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("failed to create the mock `main` webview window")
}

#[test]
fn db_backups_lists_the_copies_over_ipc() {
    // Arrange: two copies with distinct mtimes in the recovery-mode
    // data root, plus staging debris the listing must skip.
    let (temp, app) = recovery_mode_app();
    let backups = temp.path().join("db-backups");
    std::fs::write(backups.join("daily-20261117.sqlite3"), b"a copy")
        .expect("writing a copy must work");
    std::fs::write(backups.join("daily-20261118.sqlite3.tmp"), b"debris")
        .expect("writing debris must work");

    // Act
    let response = tauri::test::get_ipc_response(
        &main_webview(&app),
        invoke_request("db_backups_list", serde_json::json!({})),
    )
    .expect("the listing must respond successfully");
    let listed: Vec<crate::recovery::DbBackupInfo> = response
        .deserialize()
        .expect("the listing must deserialize into the DTO");

    // Assert: the DTO over the wire — exactly the real copies.
    let names: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["daily-20261117.sqlite3"]);
    assert_eq!(listed[0].size_bytes, 6);
    assert!(
        listed[0].modified_utc.ends_with('Z'),
        "the mtime is UTC ISO-8601, got: {}",
        listed[0].modified_utc
    );
    drop(temp);
}

#[test]
fn db_restore_rejects_an_escape_attempt_with_the_spec_ten_error_envelope() {
    // Arrange: a healthy copy an escape would reach, so the rejection
    // demonstrably comes from the validation, not a missing file. The
    // argument travels under its wire name `backupName` (the tauri
    // command macro's casing; the generated bindings do the same on
    // the frontend side).
    let _serial = IPC_RECOVERY_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (temp, app) = recovery_mode_app();
    std::fs::write(
        temp.path()
            .join("db-backups")
            .join("daily-20261117.sqlite3"),
        b"a copy",
    )
    .expect("writing a copy must work");

    // Act
    let response = tauri::test::get_ipc_response(
        &main_webview(&app),
        invoke_request(
            "db_restore_backup",
            serde_json::json!({ "backupName": "../daily-20261117.sqlite3" }),
        ),
    );

    // Assert: the rejected invoke's payload IS the §7.10 AppError
    // shape — code, message key, params — the pin D-034 recorded for
    // the first fallible command task.
    let error = response.expect_err("an escape attempt must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["message_key"], serde_json::json!("errors.validation"));
    assert_eq!(
        error["params"],
        serde_json::json!({
            "name": "../daily-20261117.sqlite3",
            "reason": "not a plain file name",
        })
    );
    assert_eq!(error["details"], serde_json::json!(null));
    drop(temp);
}

#[test]
fn db_recreate_recovers_the_database_over_ipc() {
    // Arrange: the recovery-mode state — a damaged database at the
    // §5.5 path, rejected by the startup seam, no `Db` managed.
    let _serial = IPC_RECOVERY_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (temp, app) = recovery_mode_app();
    let db_path = temp.path().join("db").join("eunomiasync.sqlite3");
    std::fs::write(&db_path, b"this is not a database at all")
        .expect("writing the damaged fixture must work");
    assert!(
        app.try_state::<crate::db::Db>().is_none(),
        "recovery mode starts without a managed database"
    );

    // Act
    tauri::test::get_ipc_response(
        &main_webview(&app),
        invoke_request("db_recreate", serde_json::json!({})),
    )
    .expect("recreate must succeed over IPC")
    .deserialize::<()>()
    .expect("the success payload is the unit value");

    // Assert — the card's criterion through the real IPC path: the
    // damaged original is preserved under its pre-recovery name, the
    // database path holds a fresh working database, and the command
    // published it as managed state (§7.10: "…and reopens it") — the
    // same state a healthy startup would have managed.
    let backups = temp.path().join("db-backups");
    let mut preserved: Vec<String> = std::fs::read_dir(&backups)
        .expect("db-backups/ must be readable")
        .map(|entry| {
            entry
                .expect("every entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("pre-recovery-"))
        .collect();
    preserved.sort();
    assert_eq!(
        preserved.len(),
        1,
        "exactly one preserved original, found: {preserved:?}"
    );
    assert_eq!(
        std::fs::read(backups.join(&preserved[0])).expect("the preserved file must be readable"),
        b"this is not a database at all",
        "the damaged original is preserved byte-identically"
    );
    assert!(
        db_path.is_file(),
        "a fresh database file exists at the §5.5 path"
    );
    assert!(
        app.try_state::<crate::db::Db>().is_some(),
        "the recreated database is managed — the app continues without a restart"
    );
    drop(temp);
}

#[test]
fn db_recreate_is_rejected_once_the_database_is_running() {
    // The recovery-mode guard over IPC: after a successful recovery
    // the same command must be rejected (E_VALIDATION) — a stale
    // screen retrying the button must not preserve and replace a
    // *working* database.
    let _serial = IPC_RECOVERY_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (temp, app) = recovery_mode_app();
    std::fs::write(
        temp.path().join("db").join("eunomiasync.sqlite3"),
        b"this is not a database at all",
    )
    .expect("writing the damaged fixture must work");
    let webview = main_webview(&app);
    tauri::test::get_ipc_response(
        &webview,
        invoke_request("db_recreate", serde_json::json!({})),
    )
    .expect("the first recreate must succeed")
    .deserialize::<()>()
    .expect("the success payload is the unit value");

    // Act
    let response = tauri::test::get_ipc_response(
        &webview,
        invoke_request("db_recreate", serde_json::json!({})),
    );

    // Assert: the rejection, with its reason param.
    let error = response.expect_err("a second recreate must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("a database is already running")
    );
    drop(temp);
}

#[test]
fn db_restore_applies_the_chosen_copy_over_ipc() {
    // The restore half of the card's criterion at the IPC level: from
    // recovery mode, the chosen copy becomes the database (the marker
    // row reads through the managed `Db`), the damaged original is
    // preserved, and the copy itself survives.
    let _serial = IPC_RECOVERY_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (temp, app) = recovery_mode_app();
    let backups = temp.path().join("db-backups");
    let db_path = temp.path().join("db").join("eunomiasync.sqlite3");
    // A valid copy holding a marker row (a real database file, the way
    // every copy scheme writes it), then the damaged live file.
    let scratch = temp.path().join("scratch.sqlite3");
    let connection =
        rusqlite::Connection::open(&scratch).expect("creating the copy fixture must work");
    connection
        .execute_batch("CREATE TABLE probe (name TEXT); INSERT INTO probe VALUES ('ipc-restore');")
        .expect("seeding the copy fixture must work");
    drop(connection);
    std::fs::copy(&scratch, backups.join("daily-20261117.sqlite3"))
        .expect("placing the copy must work");
    std::fs::remove_file(&scratch).expect("removing the scratch file must work");
    std::fs::write(&db_path, b"this is not a database at all")
        .expect("writing the damaged fixture must work");

    // Act
    tauri::test::get_ipc_response(
        &main_webview(&app),
        invoke_request(
            "db_restore_backup",
            serde_json::json!({ "backupName": "daily-20261117.sqlite3" }),
        ),
    )
    .expect("restore must succeed over IPC")
    .deserialize::<()>()
    .expect("the success payload is the unit value");

    // Assert: the chosen copy's data is the database now, the damaged
    // original is preserved byte-identically, the copy survives, and
    // the reopened database is managed.
    let database = app
        .try_state::<crate::db::Db>()
        .expect("the restored database must be managed");
    let marker: String = tauri::async_runtime::block_on(database.run(|tx| {
        tx.query_row("SELECT name FROM probe", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("the restored database must answer queries");
    assert_eq!(marker, "ipc-restore");
    let preserved: Vec<_> = std::fs::read_dir(&backups)
        .expect("db-backups/ must be readable")
        .map(|entry| {
            entry
                .expect("every entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("pre-recovery-") && name.ends_with(".sqlite3"))
        .collect();
    assert_eq!(preserved.len(), 1, "one preserved original: {preserved:?}");
    assert_eq!(
        std::fs::read(backups.join(&preserved[0])).expect("the preserved file must be readable"),
        b"this is not a database at all"
    );
    assert!(
        backups.join("daily-20261117.sqlite3").exists(),
        "the chosen copy survives the restore"
    );
    drop(temp);
}

#[test]
fn a_payload_without_the_backup_name_is_rejected() {
    // The command argument's wire name is `backupName`; anything else
    // must be rejected by the invoke layer itself (the same shape the
    // greet tests pinned for the argument contract).
    let (temp, app) = recovery_mode_app();
    let outcome = tauri::test::get_ipc_response(
        &main_webview(&app),
        invoke_request(
            "db_restore_backup",
            serde_json::json!({ "wrong": "payload" }),
        ),
    );
    assert!(
        outcome.is_err(),
        "a payload without `backupName` must be rejected"
    );
    drop(temp);
}

// ---------------------------------------------------------------------------
// `activity_list` (T-027, §7.10 History module, §7.7 UI row)
// ---------------------------------------------------------------------------

/// A mock app in the healthy state `activity_list` serves: the
/// `activity_list` command registered and a real `Db` managed over a
/// fresh temp root — the state the `setup` hook produces on a normal
/// launch (layout up, database open and managed).
fn activity_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::activity_list])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    (temp, app)
}

/// A mock app with `activity_list` registered but **no** `Db` managed —
/// the recovery-mode state from the Activity window's point of view:
/// the layout exists, the command answers, but there is no journal to
/// serve from.
fn database_less_activity_app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::activity_list])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app")
}

/// Seeds one journal entry through the `Db::run` executor (the exact
/// path a producing command takes) with its `ts` cell pinned to a
/// chosen DB-002 value — the read filters must be assertable against
/// chosen times, and `ts` is `insert`'s own stamp (D-051).
fn seed_activity_entry(
    database: &crate::db::Db,
    ts: &str,
    action: eunomia_store::action_log::Action,
    visibility: eunomia_store::action_log::Visibility,
    result: eunomia_store::action_log::Outcome,
    project_id: Option<&str>,
) {
    use eunomia_store::action_log::{LogEntry, insert};
    let ts = ts.to_owned();
    let project_id = project_id.map(str::to_owned);
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let mut entry = LogEntry::new(
            action,
            eunomia_store::action_log::Level::Info,
            visibility,
            result,
        );
        entry.project_id = project_id.as_deref();
        let id = insert(tx, &entry).map_err(crate::commands::activity_log_to_app_error)?;
        tx.execute(
            "UPDATE action_log SET ts = ?1 WHERE id = ?2",
            rusqlite::params![ts, id],
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("seeding a journal entry through the executor must work");
}

/// Invokes `activity_list` with `body` as the `query` argument, over
/// the app's existing `main` webview (the mock window is created once
/// per test — a second `WebviewWindowBuilder` call on the same app
/// fails with `WebviewLabelAlreadyExists`).
fn invoke_activity_list(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    body: serde_json::Value,
) -> Result<crate::commands::ActivityPage, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request("activity_list", serde_json::json!({ "query": body })),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<crate::commands::ActivityPage>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

#[test]
fn activity_list_serves_a_filtered_page_over_ipc() {
    // Arrange: three entries — an error inside the day's range on the
    // alpha project, a same-day ok outside the result filter, and a
    // next-day entry the date filter excludes. The oldest (alpha,
    // error) entry carries details, entity and run ids so the wire
    // DTO round-trips every column.
    let (temp, app) = activity_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    use eunomia_store::action_log::{
        Action, Details, Entity, EntityType, FailureCode, Outcome, Visibility,
    };
    {
        use eunomia_store::action_log::{Level, LogEntry, insert};
        let database = database.clone();
        tauri::async_runtime::block_on(database.run(move |tx| {
            let mut entry = LogEntry::new(
                Action::SyncProjectResult,
                Level::Error,
                Visibility::User,
                Outcome::Error,
            );
            entry.entity = Some(Entity {
                kind: EntityType::Project,
                id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f",
            });
            entry.project_id = Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f");
            entry.sync_run_id = Some("0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f");
            entry.details = Some(Details::Failure {
                code: FailureCode::new("E_SYNC_RUNNING").expect("the registry shape holds"),
                path: None,
            });
            let id = insert(tx, &entry).map_err(crate::commands::activity_log_to_app_error)?;
            tx.execute(
                "UPDATE action_log SET ts = ?1 WHERE id = ?2",
                rusqlite::params!["2026-11-10T08:00:00.000Z", id],
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
            Ok(())
        }))
        .expect("seeding the detailed entry must work");
    }
    seed_activity_entry(
        &database,
        "2026-11-10T09:00:00.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f"),
    );
    seed_activity_entry(
        &database,
        "2026-11-11T09:00:00.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Error,
        Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f"),
    );

    // Act: the §7.7 UI row's filters — date range, project, type,
    // result — with an explicit page size.
    let webview = main_webview(&app);
    let page = invoke_activity_list(
        &webview,
        serde_json::json!({
            "from_ts": "2026-11-10T00:00:00.000Z",
            "to_ts": "2026-11-10T23:59:59.999Z",
            "project_id": "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f",
            "actions": ["sync.project_result", "project.added"],
            "result": "error",
            "limit": 10,
        }),
    )
    .expect("the filtered query must succeed");

    // Assert: exactly the one entry every filter agrees on, in the
    // wire DTO's shape — snake_case fields, closed-vocabulary strings
    // in their stored spellings, details as canonical JSON text.
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.total, 1, "total counts the filters' matches");
    let entry = &page.entries[0];
    assert_eq!(entry.ts, "2026-11-10T08:00:00.000Z");
    assert_eq!(entry.level, "error");
    assert_eq!(entry.visibility, "user");
    assert_eq!(entry.action, "sync.project_result");
    assert_eq!(entry.entity_type, Some("project".to_owned()));
    assert_eq!(
        entry.entity_id,
        Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned())
    );
    assert_eq!(
        entry.project_id,
        Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned())
    );
    assert_eq!(
        entry.sync_run_id,
        Some("0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f".to_owned())
    );
    assert_eq!(entry.result, "error");
    assert_eq!(
        entry.details,
        Some(r#"{"code":"E_SYNC_RUNNING"}"#.to_owned())
    );
    // The app holds the managed `Db` (an open connection): drop it
    // before the directory — the Windows teardown rule the store suite
    // pins (SQLite files are not share-deletable there).
    drop(app);
    drop(temp);
}

#[test]
fn activity_list_returns_user_entries_only_by_default() {
    // The card's acceptance criterion, driven at the IPC boundary:
    // without an explicit diagnostic request, diagnostic entries are
    // never returned — not by the default call, not under filters
    // that also match them; with the explicit request, they are.
    let (temp, app) = activity_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    use eunomia_store::action_log::{Action, Outcome, Visibility};
    // Twin pairs: identical apart from visibility, so any filter that
    // matches one twin matches the other too.
    for index in 0..4 {
        let visibility = if index % 2 == 0 {
            Visibility::User
        } else {
            Visibility::Diagnostic
        };
        seed_activity_entry(
            &database,
            &format!("2026-11-10T12:00:{index:02}.000Z"),
            Action::ProjectAdded,
            visibility,
            Outcome::Ok,
            Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f"),
        );
    }

    // The default call: user entries only.
    let webview = main_webview(&app);
    let default_page = invoke_activity_list(&webview, serde_json::json!({}))
        .expect("the default query must succeed");
    assert_eq!(default_page.total, 2, "only the user entries");
    assert!(
        default_page
            .entries
            .iter()
            .all(|entry| entry.visibility == "user"),
        "a diagnostic entry is never returned without the explicit request"
    );

    // An explicit false is the same answer, not an opt-in.
    let explicit_false =
        invoke_activity_list(&webview, serde_json::json!({ "include_diagnostic": false }))
            .expect("the query must succeed");
    assert_eq!(explicit_false.total, 2);

    // Filters that match diagnostic twins do not leak them either.
    let filtered = invoke_activity_list(
        &webview,
        serde_json::json!({
            "from_ts": "2026-11-10T00:00:00.000Z",
            "project_id": "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f",
            "actions": ["project.added"],
        }),
    )
    .expect("the filtered query must succeed");
    assert_eq!(
        filtered.total, 2,
        "the diagnostic twins stay hidden under filters"
    );

    // The explicit request is the only way in (the diagnostics
    // section's query).
    let including =
        invoke_activity_list(&webview, serde_json::json!({ "include_diagnostic": true }))
            .expect("the including query must succeed");
    assert_eq!(including.total, 4);
    assert_eq!(
        including
            .entries
            .iter()
            .filter(|entry| entry.visibility == "diagnostic")
            .count(),
        2,
        "the diagnostic entries arrive only now"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn activity_list_rejects_invalid_filters_with_the_spec_ten_error_envelope() {
    // Every rejection is the §7.10 AppError shape (code, message_key,
    // params naming the field/value/reason, null details) — the D-034
    // pin shape, applied to the filter contract.
    let (temp, app) = activity_app();
    let webview = main_webview(&app);
    for (label, body) in [
        (
            "a timestamp without milliseconds",
            serde_json::json!({ "from_ts": "2026-11-10T00:00:00Z" }),
        ),
        (
            "a date-only bound",
            serde_json::json!({ "to_ts": "2026-11-10" }),
        ),
        (
            "an inverted range",
            serde_json::json!({
                "from_ts": "2026-11-11T00:00:00.000Z",
                "to_ts": "2026-11-10T00:00:00.000Z",
            }),
        ),
        (
            "an unknown action",
            serde_json::json!({ "actions": ["project.added", "project.exploded"] }),
        ),
        (
            "an unknown result",
            serde_json::json!({ "result": "maybe" }),
        ),
        ("a zero limit", serde_json::json!({ "limit": 0 })),
        ("an over-max limit", serde_json::json!({ "limit": 501 })),
        ("a negative offset", serde_json::json!({ "offset": -1 })),
    ] {
        let error = invoke_activity_list(&webview, body)
            .err()
            .unwrap_or_else(|| panic!("{label} must be rejected"));
        assert_eq!(
            error["code"],
            serde_json::json!("E_VALIDATION"),
            "{label}: {error}"
        );
        assert_eq!(
            error["message_key"],
            serde_json::json!("errors.validation"),
            "{label}: {error}"
        );
        for key in ["field", "value", "reason"] {
            assert!(
                error["params"]
                    .get(key)
                    .is_some_and(|value| !value.is_null()),
                "{label}: the params must name {key}, got {error}"
            );
        }
        assert_eq!(
            error["details"],
            serde_json::json!(null),
            "{label}: {error}"
        );
    }
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn activity_list_answers_honestly_without_a_database() {
    // Recovery mode (no `Db` managed): the Activity window cannot be
    // served, and the honest answer is the recorded fallback code with
    // a diagnostic naming the state — not a panic, not an empty page
    // pretending the journal is empty.
    let app = database_less_activity_app();
    let webview = main_webview(&app);
    let error = invoke_activity_list(&webview, serde_json::json!({}))
        .expect_err("the query without a database must fail");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert!(
        error["details"]["os_kind"] == serde_json::json!("NoDatabase"),
        "the diagnostic names the state, got {error}"
    );
}

#[test]
fn the_activity_list_command_is_registered_with_the_specta_builder() {
    // The §7.10 History-module name, registered exactly once — without
    // this pin, dropping the registration would leave
    // `export_bindings_are_up_to_date` green while it silently
    // rewrote the committed bindings without the command (the D-034
    // wiring-pin rationale).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::activity_list").count(),
        1,
        "lib.rs must register the `activity_list` command exactly once (§7.10, D-009)"
    );
}

#[test]
fn an_empty_actions_array_is_no_action_filter_and_duplicates_are_bounded() {
    // Two pinned edges of the actions contract. (1) An empty array
    // means NO filter (every action matches) — the documented DTO
    // semantics ("a filter the caller did not select"), the same
    // reading an unset dropdown has; the opposite choice (match
    // nothing) would make an accidental `[]` blank the window. (2) A
    // long array of duplicates parses and de-duplicates into the
    // vocabulary's own bound — the generated `IN (…)` placeholder list
    // can never reach SQLite's variable limit through a frontend bug.
    let (temp, app) = activity_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    use eunomia_store::action_log::{Action, Outcome, Visibility};
    seed_activity_entry(
        &database,
        "2026-11-10T12:00:00.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    seed_activity_entry(
        &database,
        "2026-11-10T12:00:01.000Z",
        Action::BackupCreated,
        Visibility::User,
        Outcome::Ok,
        None,
    );

    let webview = main_webview(&app);
    // (1) The empty array is the absent filter.
    let empty = invoke_activity_list(&webview, serde_json::json!({ "actions": [] }))
        .expect("an empty actions array must succeed");
    assert_eq!(empty.total, 2, "an empty actions array is no action filter");

    // (2) A hostile-length array of one repeated valid member: the
    // de-duplicated parse bounds the placeholder list, the query
    // succeeds and filters exactly.
    let repeated = vec!["project.added"; 5000];
    let deduped = invoke_activity_list(&webview, serde_json::json!({ "actions": repeated }))
        .expect("a long duplicate array must succeed (the parse de-duplicates)");
    assert_eq!(deduped.total, 1, "the de-duplicated filter still selects");

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_typoed_filter_key_is_rejected_loudly() {
    // `deny_unknown_fields` on the DTO: a misspelled filter name must
    // fail the deserialization loudly (the invoke layer's rejection),
    // not silently widen the query to everything — the "typo returns
    // the whole journal" surprise the round-1 review flagged.
    let (temp, app) = activity_app();
    let webview = main_webview(&app);
    let outcome = tauri::test::get_ipc_response(
        &webview,
        invoke_request(
            "activity_list",
            serde_json::json!({ "query": { "fromTs": "2026-11-10T00:00:00.000Z" } }),
        ),
    );
    assert!(
        outcome.is_err(),
        "a typo'd filter key must be rejected, not ignored"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_tampered_ts_cell_answers_the_fallback_envelope_not_validation() {
    // The corrupt-cells wire contract end to end (the round-2 review
    // criterion): a stored `ts` cell outside the DB-002 shape is
    // database corruption, NOT caller input — the answer is the
    // D-037 fallback with the `ActionLogCorrupt` diagnostic, empty
    // params, and an `os_message` that names the failure class
    // without the raw cell (a tampered cell is arbitrary database
    // text; params and os_message feed the log line and the localized
    // message — SEC-008's surfaces).
    let (temp, app) = activity_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    use eunomia_store::action_log::{Action, Outcome, Visibility};
    seed_activity_entry(
        &database,
        "2026-11-10T12:00:00.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute("UPDATE action_log SET ts = 'not-a-timestamp'", [])
            .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("tampering the ts cell through the executor must work");

    let webview = main_webview(&app);
    let error = invoke_activity_list(&webview, serde_json::json!({}))
        .expect_err("a corrupt journal must fail the read");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(
        error["params"],
        serde_json::json!({}),
        "no raw cell travels in params"
    );
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ActionLogCorrupt")
    );
    assert_eq!(
        error["details"]["os_message"],
        serde_json::json!("a stored ts cell is not the DB-002 timestamp shape"),
        "the diagnostic names the class only, never the stored value"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}
