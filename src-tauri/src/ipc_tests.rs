//! IPC round-trip tests for the recovery commands (T-024) against the
//! Tauri mock runtime — the harness shape the removed `greet` tests
//! used (D-037): a real invoke through `tauri::test::get_ipc_response`,
//! no windowing system involved. Lives inside the lib crate because
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
