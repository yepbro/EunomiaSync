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
//!
//! The `settings_get`/`settings_set` tests use the same healthy-app
//! shape and pin the Settings contract (§7.10, §6.5, DEF-001…005,
//! LOG-001…004): the §6.5 defaults in table order, the
//! applies-without-a-restart round trip, the E_VALIDATION envelope of
//! every rejected change, the in-transaction journal (D-051) with its
//! rollback proof, the DEF-005 pin over a seeded project row, and the
//! corrupt-cell and no-database answers.
//!
//! The `app_info` tests (T-031, UPD-005/REL-004, §5.5) pin the About
//! window's fact sheet: every field against its spec oracle, every
//! state of `db_created_fresh` through the real `Db::open` seam
//! (missing file, 0-byte file, existing database), the no-database
//! answer (the command is non-DB by design and creates no database
//! file), and the missing-`DataRoot` fallback envelope; the recovery
//! round-trip tests above pin the flag's recreate/restore
//! consequences. The `running_as_root` tests (T-032, NFR-023) pin the
//! root-launch flag on the wire (with the environment's half asserted
//! conditionally) and both halves of the uid predicate through the
//! injected-uid sheet.
//!
//! The `markdown_preview` tests (T-056 the pipeline, T-057 the
//! warnings; §7.10 Library module, SEC-005, D-15) pin the acceptance
//! pair on the wire: a `<script>` block and an
//! external image (both spellings — Markdown `![alt](https://…)` and
//! raw `<img src="https://…">`) reach the command's result only as
//! escaped visible text, never as executable or referenceable markup;
//! plus the empty-input answer and the specta registration pin. The
//! renderer's own policy has its unit suite in `markdown.rs`; these
//! tests prove the command carries it over the real IPC envelope.
//!
//! The `projects_list` tests (T-072, §7.10 Projects module, §7.1,
//! §7.4 MAIN-028) use the healthy-app shape with the projects seeded
//! through the T-071 repository (the `Db::run` path a producing
//! command takes) and real `.git` folders for the availability probe:
//! the sort_order answer after a repository reorder, the
//! archived-filter widening with DB-014's never-dirty pin, the full
//! column-field set including the D-09 unavailable flag + reason, the
//! empty-database answer, the no-database honesty pin, the
//! corrupt-cell fallback envelope, and the registration pin.
//!
//! The `dialog_pick_folder` tests (T-073, §7.10 System module, §5.6
//! SEC-001/SEC-003) drive the command body through its picker seam
//! (`dialog_pick_folder_with`) — the plugin call itself opens a real
//! desktop dialog and is the recorded untestable seam (the
//! `app_open_data_folder` treatment), pinned as wiring by the
//! registration test. The SEC-003 check (`validate_ui_path`) is
//! driven directly: it is the reusable boundary the upcoming
//! path-consuming commands call, not a command of its own. Together
//! they pin the card's criterion — a path from nowhere is rejected —
//! plus both allowlist halves (session pick, registered project in
//! either state), the canonical-form matching (an aliased spelling of
//! a pick passes), the exact-folder boundary (a project's subfolder
//! is NOT the project), cancel (null, nothing remembered), the
//! vanish race, and the honest no-database/missing-state answers.

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
    let database = app
        .try_state::<crate::db::Db>()
        .expect("the recreated database is managed — the app continues without a restart");
    assert!(
        database.created_fresh(),
        "the recreate is a fresh bootstrap: the flag answers true (D-057)"
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
    assert!(
        !database.created_fresh(),
        "the restored copy held a database: the flag answers false (D-057)"
    );
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

// ---------------------------------------------------------------------------
// `settings_get` / `settings_set` (T-030, §7.10 Settings module,
// §6.5/§7.2 DEF-001…005, LOG-001…004)
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the settings commands serve: both
/// commands registered and a real `Db` managed over a fresh temp root
/// (the `activity_app` shape).
fn settings_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::settings_get,
            crate::commands::settings_set
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    (temp, app)
}

/// Invokes `settings_get` over the app's `main` webview.
fn invoke_settings_get(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> Result<Vec<crate::commands::SettingItem>, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request("settings_get", serde_json::json!({})),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<Vec<crate::commands::SettingItem>>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

/// Invokes `settings_set` with `changes` over the app's `main` webview
/// (the wire body's `changes` argument).
fn invoke_settings_set(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    changes: serde_json::Value,
) -> Result<Vec<crate::commands::SettingItem>, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request("settings_set", serde_json::json!({ "changes": changes })),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<Vec<crate::commands::SettingItem>>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

/// Reads the journal's newest entries (up to 100) through the
/// `Db::run` executor — the assertions' window into what a command
/// journaled.
fn journal_entries(database: &crate::db::Db) -> Vec<eunomia_store::action_log::LogRecord> {
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 100)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("reading the journal through the executor must work")
}

/// One setting's value from a settings map, panicking with the map on
/// a missing key (the map always holds all thirteen).
fn value_of<'a>(items: &'a [crate::commands::SettingItem], key: &str) -> &'a str {
    items
        .iter()
        .find(|item| item.key == key)
        .unwrap_or_else(|| panic!("the map must contain {key}: {items:?}"))
        .value
        .as_str()
}

#[test]
fn settings_get_serves_the_spec_defaults_in_table_order_over_ipc() {
    // Arrange: a fresh database — the settings table has zero rows, so
    // every answer is a §6.5 default applied by the read.
    let (temp, app) = settings_app();
    let webview = main_webview(&app);

    // Act
    let items = invoke_settings_get(&webview).expect("the read must succeed");

    // Assert: all thirteen §6.5 keys, in table order, with their
    // defaults as canonical JSON text — hardcoded spec literals, an
    // oracle independent of the production constants (the D-039 test
    // split, applied to the wire).
    let expected: [(&str, &str); 13] = [
        ("default.scheme", "\"A\""),
        ("default.base_name", "\"AGENTS.md\""),
        (
            "default.extra_local",
            "{\"enabled\":false,\"position\":\"end\"}",
        ),
        (
            "default.extra_project",
            "{\"enabled\":false,\"position\":\"end\"}",
        ),
        ("backup.retention", "10"),
        ("sync.skip_identical", "true"),
        ("scan.max_depth", "6"),
        (
            "scan.exclude_names",
            "[\"node_modules\",\"vendor\",\".git\",\"dist\",\"build\",\"target\",\".venv\"]",
        ),
        ("history.max_revisions", "200"),
        ("log.max_entries", "10000"),
        ("log.max_days", "180"),
        ("ui.language", "\"en\""),
        ("update.check_on_start", "true"),
    ];
    assert_eq!(items.len(), expected.len());
    for (item, (key, value)) in items.iter().zip(expected) {
        assert_eq!(item.key, key, "§6.5 table order");
        assert_eq!(item.value, value, "the canonical JSON default of {key}");
    }
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn settings_set_applies_without_a_restart_and_journals_each_changed_key() {
    // Arrange
    let (temp, app) = settings_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);

    // Act: a partial update — two of the thirteen keys.
    let applied = invoke_settings_set(
        &webview,
        serde_json::json!([
            { "key": "scan.max_depth", "value": "9" },
            { "key": "ui.language", "value": "\"cs\"" },
        ]),
    )
    .expect("the save must succeed");

    // Assert (1): the response is the fresh full map — the changed
    // keys applied, everything else at its default, §6.5 table order.
    assert_eq!(applied.len(), 13);
    assert_eq!(value_of(&applied, "scan.max_depth"), "9");
    assert_eq!(value_of(&applied, "ui.language"), "\"cs\"");
    assert_eq!(value_of(&applied, "backup.retention"), "10");

    // Assert (2): DEF-001…004's "each setting applies without a
    // restart", honestly adapted (no scanner exists yet — the
    // T-012/D-032 precedent): the NEXT read — a new command call over
    // the same session's `Db` — returns the new values. The mechanism
    // the criterion stands on is exactly this: every reader goes
    // through the repository to the table, nothing caches in between,
    // so the future scanner's "next scan takes the new depth" follows
    // from the same read path this pins.
    let items = invoke_settings_get(&webview).expect("the read must succeed");
    assert_eq!(value_of(&items, "scan.max_depth"), "9");
    assert_eq!(value_of(&items, "ui.language"), "\"cs\"");

    // Assert (3): LOG-001…004 — one `settings.changed` entry per
    // changed key, Info/User/Ok, the dotted key as the setting entity
    // id, no details (no value travels; SEC-008). `recent` is newest
    // first, so the last-written entry (ui.language) comes first.
    let entries = journal_entries(&database);
    assert_eq!(entries.len(), 2, "one entry per changed key");
    for (entry, key) in entries.iter().zip(["ui.language", "scan.max_depth"]) {
        assert_eq!(entry.action.as_str(), "settings.changed");
        assert_eq!(entry.level.as_str(), "info");
        assert_eq!(entry.visibility.as_str(), "user");
        assert_eq!(entry.result.as_str(), "ok");
        assert_eq!(
            entry
                .entity
                .as_ref()
                .map(|(kind, id)| (kind.as_str(), id.as_str())),
            Some(("setting", key)),
            "the entry names the changed key through its entity"
        );
        assert!(
            entry.details.is_none(),
            "no setting value travels into the journal"
        );
    }
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn settings_set_accepts_the_inclusive_range_bounds_over_ipc() {
    // The §6.5 ranges are inclusive — both bounds of every ranged key
    // apply over the real command (the D-039 store suite pins the same
    // fact at the repository seam with one-past rejections; this pins
    // the wire half).
    let (temp, app) = settings_app();
    let webview = main_webview(&app);
    for (key, low, high) in [
        ("backup.retention", "1", "200"),
        ("scan.max_depth", "1", "20"),
        ("history.max_revisions", "10", "1000"),
        ("log.max_entries", "1000", "100000"),
        ("log.max_days", "7", "3650"),
    ] {
        for bound in [low, high] {
            invoke_settings_set(
                &webview,
                serde_json::json!([{ "key": key, "value": bound }]),
            )
            .unwrap_or_else(|error| panic!("{key}={bound} is inside the range: {error}"));
        }
    }
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn settings_set_rejects_invalid_changes_with_the_spec_ten_error_envelope() {
    // Every rejection is the §7.10 AppError shape (code, message_key,
    // params naming the key/value/reason, null details) — the D-034
    // pin shape, applied to the settings contract: the §6.5 ranges
    // (one past each side of scan.max_depth), the types, the closed
    // vocabularies (case-sensitive), the extra-file shape, unknown
    // keys, duplicates, non-JSON value text, and the empty list.
    let (temp, app) = settings_app();
    let webview = main_webview(&app);
    let cases: [(&str, serde_json::Value); 9] = [
        (
            "an unknown key",
            serde_json::json!([{ "key": "scan.depth", "value": "3" }]),
        ),
        (
            "a duplicate key",
            serde_json::json!([
                { "key": "scan.max_depth", "value": "3" },
                { "key": "scan.max_depth", "value": "4" },
            ]),
        ),
        (
            "value text that is not JSON",
            serde_json::json!([{ "key": "scan.max_depth", "value": "six" }]),
        ),
        (
            "a string where an integer belongs",
            serde_json::json!([{ "key": "scan.max_depth", "value": "\"6\"" }]),
        ),
        (
            "one below the range",
            serde_json::json!([{ "key": "scan.max_depth", "value": "0" }]),
        ),
        (
            "one above the range",
            serde_json::json!([{ "key": "scan.max_depth", "value": "21" }]),
        ),
        (
            "a case-wrong enum member",
            serde_json::json!([{ "key": "default.scheme", "value": "\"a\"" }]),
        ),
        (
            "an extra-file object missing a field",
            serde_json::json!([{ "key": "default.extra_local", "value": "{\"enabled\":true}" }]),
        ),
        ("an empty changes list", serde_json::json!([])),
    ];
    for (label, changes) in cases {
        let error = invoke_settings_set(&webview, changes)
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
        for key in ["key", "value", "reason"] {
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
fn a_rejected_settings_set_writes_nothing() {
    // The all-or-nothing half of §6.9 at the IPC boundary: a list with
    // any invalid entry writes nothing at all — not the valid entries'
    // settings rows, and no journal entry claiming a change.
    let (temp, app) = settings_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);

    // Act: scan.max_depth=15 is valid, backup.retention=0 is not.
    let error = invoke_settings_set(
        &webview,
        serde_json::json!([
            { "key": "scan.max_depth", "value": "15" },
            { "key": "backup.retention", "value": "0" },
        ]),
    )
    .expect_err("the list with an invalid entry must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        error["params"]["key"],
        serde_json::json!("backup.retention"),
        "the rejection names the offending key, not the valid one"
    );

    // Assert: the valid entry's key stayed at its default, and the
    // journal is empty.
    let items = invoke_settings_get(&webview).expect("the read must succeed");
    assert_eq!(value_of(&items, "scan.max_depth"), "6");
    assert_eq!(value_of(&items, "backup.retention"), "10");
    assert!(
        journal_entries(&database).is_empty(),
        "a rejected save journals nothing"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_failed_settings_set_rolls_back_its_journal_entries() {
    // D-051's in-transaction semantics, proven through the real
    // command: the journal entry shares the operation's transaction,
    // so a failure AFTER a set and its entry both succeeded rolls both
    // back — the journal must not claim a change that never committed.
    // The failure is staged deterministically inside the database: a
    // trigger that rejects exactly the second key's journal insert
    // (RAISE(ABORT) → SQLITE_CONSTRAINT → the shared base classifier's
    // E_VALIDATION with the SQLite facts in details).
    let (temp, app) = settings_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    // One committed change first — the baseline the rollback must
    // restore to (and its journal entry must stay).
    invoke_settings_set(
        &webview,
        serde_json::json!([{ "key": "scan.max_depth", "value": "9" }]),
    )
    .expect("the baseline save must succeed");
    assert_eq!(journal_entries(&database).len(), 1);
    // Stage the trigger through the executor (the same connection the
    // command uses).
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_language_journal BEFORE INSERT ON action_log
             WHEN NEW.entity_id = 'ui.language'
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");

    // Act: scan.max_depth=12 applies and journals; then ui.language
    // applies, and its journal insert aborts the whole transaction.
    let error = invoke_settings_set(
        &webview,
        serde_json::json!([
            { "key": "scan.max_depth", "value": "12" },
            { "key": "ui.language", "value": "\"cs\"" },
        ]),
    )
    .expect_err("the staged abort must fail the save");
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

    // Assert: BOTH keys are back — the set that had already succeeded
    // (scan.max_depth) and the one whose journal insert aborted
    // (ui.language) — and the journal still holds exactly the
    // baseline's one entry: the rolled-back entry vanished with the
    // rolled-back set.
    let items = invoke_settings_get(&webview).expect("the read must succeed");
    assert_eq!(value_of(&items, "scan.max_depth"), "9");
    assert_eq!(value_of(&items, "ui.language"), "\"en\"");
    let entries = journal_entries(&database);
    assert_eq!(entries.len(), 1, "only the baseline entry survived");
    assert_eq!(
        entries[0].entity.as_ref().map(|(_, id)| id.as_str()),
        Some("scan.max_depth")
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn changing_default_settings_never_touches_existing_projects() {
    // DEF-005/PRJ-012 at the IPC boundary, in the D-039 store-suite
    // shape: a real project row (seeded directly — no project command
    // exists yet, the boundary this test records) survives a change of
    // all four `default.*` settings untouched. The second half of the
    // boundary is structural: `settings_set` only ever addresses the
    // thirteen §6.5 keys (the unknown-key rejection), and the settings
    // path writes only `settings` and `action_log` rows.
    let (temp, app) = settings_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let project_id = eunomia_store::values::new_id();
    let seeded_id = project_id.clone();
    let seeded_at = eunomia_store::values::now_utc();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "INSERT INTO projects (id, path, path_key, name, label, color, added_via,
                                   sort_order, scheme, base_name, created_at, updated_at)
             VALUES (?1, ?2, ?2, 'Kept project', 'KEPT', 'teal', 'manual',
                     1, 'C', 'CLAUDE.md', ?3, ?3)",
            rusqlite::params![seeded_id, "/tmp/eunomia-test-project", seeded_at],
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("seeding the project row must work");
    let webview = main_webview(&app);

    // Act: change all four `default.*` settings away from the
    // project's own values.
    invoke_settings_set(
        &webview,
        serde_json::json!([
            { "key": "default.scheme", "value": "\"B\"" },
            { "key": "default.base_name", "value": "\"AGENTS.md\"" },
            { "key": "default.extra_local", "value": "{\"enabled\":true,\"position\":\"start\"}" },
            { "key": "default.extra_project", "value": "{\"enabled\":true,\"position\":\"start\"}" },
        ]),
    )
    .expect("changing the defaults must succeed");

    // Assert: the project row is unchanged — its own scheme, base name
    // and all four extra-file columns, and the same single row.
    let expected_id = project_id;
    let (
        scheme,
        base_name,
        local_enabled,
        local_pos,
        project_enabled,
        project_pos,
        project_count,
    ): (String, String, i64, String, i64, String, i64) =
        tauri::async_runtime::block_on(database.run(move |tx| {
            tx.query_row(
                "SELECT scheme, base_name, extra_local_enabled,
                        extra_local_position, extra_project_enabled,
                        extra_project_position,
                        (SELECT COUNT(*) FROM projects)
                 FROM projects WHERE id = ?1",
                rusqlite::params![expected_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .map_err(crate::db::sqlite_error_to_app_error)
        }))
        .expect("re-reading the project row must work");
    assert_eq!(scheme, "C");
    assert_eq!(base_name, "CLAUDE.md");
    assert_eq!(local_enabled, 0);
    assert_eq!(local_pos, "end");
    assert_eq!(project_enabled, 0);
    assert_eq!(project_pos, "end");
    assert_eq!(project_count, 1, "no project rows were created or changed");
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_corrupt_settings_cell_answers_the_fallback_envelope_not_validation() {
    // The corrupt-cells wire contract (the D-052 T-027 round-2 rule,
    // applied to the settings table): a stored value cell outside its
    // contract is database corruption, NOT caller input — both the
    // read command and the save command's final map fail with the
    // recorded D-037 fallback, a `SettingsCorrupt` diagnostic, empty
    // params, and class-only `os_message` (a tampered cell is
    // arbitrary database text; params and os_message feed the log line
    // and the localized message).
    let (temp, app) = settings_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    invoke_settings_set(
        &webview,
        serde_json::json!([{ "key": "scan.max_depth", "value": "9" }]),
    )
    .expect("the baseline save must succeed");
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute(
            "UPDATE settings SET value = 'not json' WHERE key = 'scan.max_depth'",
            [],
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("tampering the value cell through the executor must work");

    // The read command.
    let error = invoke_settings_get(&webview).expect_err("a corrupt cell must fail the read");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(
        error["params"],
        serde_json::json!({}),
        "no raw cell travels"
    );
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("SettingsCorrupt")
    );
    assert_eq!(
        error["details"]["os_message"],
        serde_json::json!("a stored settings value cell is not valid JSON"),
        "the diagnostic names the class only, never the stored value"
    );

    // The save command's final map fails the same way — its read half
    // uses the read mapping (a healthy key cannot be saved past a
    // corrupt sibling).
    let error = invoke_settings_set(
        &webview,
        serde_json::json!([{ "key": "ui.language", "value": "\"cs\"" }]),
    )
    .expect_err("the save's final map must fail on the corrupt cell");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("SettingsCorrupt")
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn settings_commands_answer_honestly_without_a_database() {
    // Recovery mode (no `Db` managed): the Settings window cannot be
    // served, and the honest answer for both commands is the recorded
    // fallback with a diagnostic naming the state — not a panic, and
    // not the §6.5 defaults pretending to be the user's settings.
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::settings_get,
            crate::commands::settings_set
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    let webview = main_webview(&app);
    let error = invoke_settings_get(&webview).expect_err("the read without a database must fail");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert!(
        error["details"]["os_kind"] == serde_json::json!("NoDatabase"),
        "the diagnostic names the state, got {error}"
    );
    let error = invoke_settings_set(
        &webview,
        serde_json::json!([{ "key": "ui.language", "value": "\"cs\"" }]),
    )
    .expect_err("the save without a database must fail");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert!(
        error["details"]["os_kind"] == serde_json::json!("NoDatabase"),
        "the diagnostic names the state, got {error}"
    );
}

#[test]
fn the_settings_commands_are_registered_with_the_specta_builder() {
    // The §7.10 Settings-module names, each registered exactly once —
    // without this pin, dropping a registration would leave
    // `export_bindings_are_up_to_date` green while it silently rewrote
    // the committed bindings without the command (the D-034
    // wiring-pin rationale).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for name in ["settings_get", "settings_set"] {
        assert_eq!(
            lib.matches(&format!("commands::{name}")).count(),
            1,
            "lib.rs must register the `{name}` command exactly once (§7.10, D-009)"
        );
    }
}

// ---------------------------------------------------------------------------
// `projects_list` (T-072, §7.10 Projects module, §7.1, §7.4 MAIN-028,
// DB-014/DB-021, D-09/DB-032, D-084)
// ---------------------------------------------------------------------------

/// A mock app with `projects_list` registered over a fresh database in
/// a temp root — the healthy-app state (the `activity_app` shape).
fn projects_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::projects_list])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    (temp, app)
}

/// Creates a real available repository folder under `root`: the D-084
/// probe needs the directory, read access and a `.git` entry — a bare
/// `.git` directory is enough, no git plumbing is involved.
fn repo_dir(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let repo = root.join(name);
    std::fs::create_dir_all(repo.join(".git"))
        .expect("creating the repository folder and its .git entry must work");
    repo
}

/// One [`NewProject`] with the given display fields and a fixed
/// default write configuration — the column fields are what the tests
/// vary; the §6.5 settings the column never reads stay at defaults.
fn column_project<'a>(
    path: &'a str,
    path_key: &'a str,
    name: &'a str,
    label: &'a str,
    color: &'a str,
) -> eunomia_store::projects::NewProject<'a> {
    use eunomia_store::projects::AddedVia;
    use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
    eunomia_store::projects::NewProject {
        path,
        path_key,
        name,
        label,
        color,
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

/// Seeds one project through the repository (the `Db::run` path a
/// producing command takes) and returns its row id. The path arrives
/// owned and the display fields as `'static` spellings — the
/// executor's closure is `'static`, so nothing may borrow the
/// test's stack.
fn seed_project(
    database: &crate::db::Db,
    path: String,
    path_key: &'static str,
    name: &'static str,
    label: &'static str,
    color: &'static str,
) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = column_project(&path, path_key, name, label, color);
        eunomia_store::projects::create(tx, &new)
            .map(|project| project.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work")
}

/// Invokes `projects_list` with the given `includeArchived` wire value
/// (`null` omitted is the default-active read), deserializing the
/// success payload into the typed DTO.
fn invoke_projects_list(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    include_archived: serde_json::Value,
) -> Result<Vec<crate::commands::ProjectColumnEntry>, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "projects_list",
            serde_json::json!({ "includeArchived": include_archived }),
        ),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<Vec<crate::commands::ProjectColumnEntry>>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

#[test]
fn projects_list_serves_active_columns_in_sort_order_over_ipc() {
    // Arrange: three real repositories created in Gamma-Alpha-Beta
    // order (creation appends sort_order 1,2,3), then reordered
    // through the repository to Alpha-Beta-Gamma — the list must
    // follow `sort_order`, not insertion or id order; plus a fourth
    // project whose folder was never created (the D-09 unavailable
    // case) and a seeded `last_synced_at` on one row (no sync writer
    // exists yet — the seeding path the tests record).
    let (temp, app) = projects_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let gamma = seed_project(
        &database,
        repo_dir(temp.path(), "gamma").display().to_string(),
        "/gamma",
        "Gamma",
        "GAM",
        "blue",
    );
    let alpha = seed_project(
        &database,
        repo_dir(temp.path(), "alpha").display().to_string(),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let beta = seed_project(
        &database,
        repo_dir(temp.path(), "beta").display().to_string(),
        "/beta",
        "Beta",
        "BET",
        "green",
    );
    let vanished_path = temp.path().join("never-created").display().to_string();
    let vanished = seed_project(
        &database,
        vanished_path,
        "/never-created",
        "Vanished",
        "VAN",
        "amber",
    );
    let (alpha_id, beta_id, gamma_id, vanished_id) = (alpha, beta, gamma, vanished);
    // D-093: `reorder` renumbers the COMPLETE id set — all four rows,
    // moved to Alpha-Beta-Gamma-Vanished (the vanished project rides
    // along at the tail; its folder state is irrelevant to order).
    let ids = [
        alpha_id.clone(),
        beta_id.clone(),
        gamma_id.clone(),
        vanished_id.clone(),
    ];
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::reorder(tx, &ids.iter().map(String::as_str).collect::<Vec<_>>())
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("reordering through the repository must work");
    // `last_synced_at` has no writer yet (OUT-020: the sync tasks own
    // it); seed the wire fact directly, the way the corrupt-cell
    // tests seed their cells.
    let vanished_for_stamp = vanished_id.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE projects SET last_synced_at = ?1 WHERE id = ?2",
            rusqlite::params!["2026-11-19T10:30:00.000Z", vanished_for_stamp],
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("stamping last_synced_at through the executor must work");

    // Act
    let webview = main_webview(&app);
    let columns = invoke_projects_list(&webview, serde_json::Value::Null)
        .expect("the active-columns read must succeed");

    // Assert: sort_order order — the reorder moved Alpha to the front
    // — with every column field on the wire. The vanished folder is a
    // flag + reason, not an error (D-09/MAIN-028); its dirty flag is
    // false under the boundary `dirty_verdict` documents; its stamped
    // time round-trips and the never-synced rows answer `None`.
    assert_eq!(
        columns
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [
            alpha_id.as_str(),
            beta_id.as_str(),
            gamma_id.as_str(),
            vanished_id.as_str()
        ],
        "the columns follow sort_order (ALG-001's read order), not insertion order"
    );
    let first = &columns[0];
    assert_eq!(first.id, alpha_id);
    assert_eq!(first.name, "Alpha");
    assert_eq!(first.label, "ALP");
    assert_eq!(first.color, "red");
    assert_eq!(first.state, "active");
    assert!(first.available, "a real repository folder is available");
    assert_eq!(first.unavailable_reason, None);
    assert!(!first.dirty);
    assert_eq!(first.last_synced_at, None);
    let vanished_column = &columns[3];
    assert!(!vanished_column.available, "the missing folder is a flag");
    assert_eq!(
        vanished_column.unavailable_reason.as_deref(),
        Some("missing"),
        "the D-084 verdict names the D-09 text"
    );
    assert!(!vanished_column.dirty);
    assert_eq!(
        vanished_column.last_synced_at.as_deref(),
        Some("2026-11-19T10:30:00.000Z")
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn projects_list_widens_to_archived_rows_by_the_wire_flag() {
    // Arrange: Alpha, Beta, Gamma created in order, then Beta archived
    // — §6.4 keeps its position, so the table order stays
    // Alpha, Beta, Gamma with Beta carrying `archived`.
    let (temp, app) = projects_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let alpha = seed_project(
        &database,
        repo_dir(temp.path(), "alpha").display().to_string(),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let beta = seed_project(
        &database,
        repo_dir(temp.path(), "beta").display().to_string(),
        "/beta",
        "Beta",
        "BET",
        "green",
    );
    let gamma = seed_project(
        &database,
        repo_dir(temp.path(), "gamma").display().to_string(),
        "/gamma",
        "Gamma",
        "GAM",
        "blue",
    );
    let beta_id = beta.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::archive(tx, &beta_id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("archiving through the repository must work");
    let webview = main_webview(&app);

    // Act + assert, first half: the default read carries the active
    // columns only — the archived row is filtered out, and the
    // survivors keep their relative sort_order.
    let active = invoke_projects_list(&webview, serde_json::Value::Null)
        .expect("the default read must succeed");
    assert_eq!(
        active
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [alpha.as_str(), gamma.as_str()],
        "the default read is the active subset in sort_order"
    );

    // Second half: the explicit flag widens the same ordered read to
    // the whole table — Beta returns at its kept position, marked
    // archived, and DB-014's never-dirty rule rides on the wire.
    let all = invoke_projects_list(&webview, serde_json::json!(true))
        .expect("the widened read must succeed");
    assert_eq!(
        all.iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [alpha.as_str(), beta.as_str(), gamma.as_str()],
        "include_archived keeps the table-wide sort_order (archive keeps the position)"
    );
    let archived_entry = &all[1];
    assert_eq!(archived_entry.state, "archived");
    assert!(
        !archived_entry.dirty,
        "DB-014: an archived project is never dirty"
    );

    // Third half: an explicit false is the default, not a third mode.
    let explicit = invoke_projects_list(&webview, serde_json::json!(false))
        .expect("the explicit-false read must succeed");
    assert_eq!(
        explicit
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [alpha.as_str(), gamma.as_str()],
        "an explicit false is the default active read"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn projects_list_answers_an_empty_list_for_a_fresh_database() {
    // The empty app's honest answer: an empty list, not an error and
    // not a null payload the frontend has to special-case.
    let (temp, app) = projects_app();
    let webview = main_webview(&app);
    let columns = invoke_projects_list(&webview, serde_json::Value::Null)
        .expect("a fresh database must answer successfully");
    assert_eq!(columns, Vec::new());
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn projects_list_answers_honestly_without_a_database() {
    // Recovery mode (the `activity_list` pin): no `Db` managed — the
    // recorded D-037 fallback with the `NoDatabase` diagnostic, never
    // a pretend-empty list that would lie about the projects table
    // being empty.
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::projects_list])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    let webview = main_webview(&app);
    let error = invoke_projects_list(&webview, serde_json::Value::Null)
        .expect_err("the no-database state must fail honestly");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn a_corrupt_projects_cell_answers_the_fallback_envelope_not_validation() {
    // The corrupt-cells wire contract (the T-027 round-2 rule, applied
    // to the projects table): a stored vocabulary cell outside its
    // CHECK vocabulary is corruption, NOT caller input — the read
    // fails with the recorded D-037 fallback, a `ProjectsCorrupt`
    // diagnostic, empty params, and a class-only `os_message` (a
    // tampered cell is arbitrary database text; `params` and
    // `os_message` feed the log line and the localized message).
    // Written through the `ignore_check_constraints` test seam — the
    // schema CHECKs make such a cell unreachable through the API.
    let (temp, app) = projects_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    seed_project(
        &database,
        repo_dir(temp.path(), "alpha").display().to_string(),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "PRAGMA ignore_check_constraints = 1;\n\
             UPDATE projects SET state = 'vanished';\n\
             PRAGMA ignore_check_constraints = 0;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("tampering the state cell through the executor must work");

    // Act: the widened read — the default `state = 'active'` filter
    // would shield the tampered row from ever being decoded, so the
    // whole-table read is what surfaces the corrupt cell.
    let webview = main_webview(&app);
    let error = invoke_projects_list(&webview, serde_json::json!(true))
        .expect_err("a corrupt cell must fail the read");

    // Assert
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(
        error["params"],
        serde_json::json!({}),
        "no raw cell travels"
    );
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ProjectsCorrupt")
    );
    assert_eq!(
        error["details"]["os_message"],
        serde_json::json!("a stored state cell is outside its vocabulary"),
        "the diagnostic names the column and class only, never the stored value"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_projects_list_command_is_registered_with_the_specta_builder() {
    // The §7.10 Projects-module name, registered exactly once — the
    // D-034 wiring-pin rationale: without this pin, dropping the
    // registration would leave `export_bindings_are_up_to_date` green
    // while it silently rewrote the committed bindings without the
    // command.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::projects_list").count(),
        1,
        "lib.rs must register the `projects_list` command exactly once (§7.10, D-009)"
    );
}

// ---------------------------------------------------------------------------
// `app_info` (T-031, §7.10 System module, §7.9 UPD-005/REL-004, §5.5)
// ---------------------------------------------------------------------------

/// Invokes `app_info` over the app's `main` webview, deserializing the
/// success payload into the typed DTO.
fn invoke_app_info(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> Result<crate::commands::AppInfo, serde_json::Value> {
    let response =
        tauri::test::get_ipc_response(webview, invoke_request("app_info", serde_json::json!({})));
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<crate::commands::AppInfo>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

/// A mock app in the healthy state `app_info` serves: the command
/// registered, `DataRoot` managed at `root` and the given real `Db`
/// managed. `database` decides the `db_created_fresh` half under
/// test — the caller opens it over `root` either as the first open of
/// a clean root (this open creates the database: the §5.5 "empty DB"
/// state) or as a second open over a root whose database already
/// exists (every launch after the first).
fn app_info_app(
    database: crate::db::Db,
    root: std::path::PathBuf,
) -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::app_info])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(data_dirs::DataRoot(root));
    app.manage(database);
    app
}

#[test]
fn app_info_serves_the_spec_facts_and_the_fresh_database_flag() {
    // Arrange: the first launch over a clean root — no database file
    // exists, so the open this harness performs creates one from
    // scratch (the §5.5 "empty DB" state the flag exists to name).
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let root = temp.path().to_path_buf();
    let app = app_info_app(database, root);

    // Act
    let info = invoke_app_info(&main_webview(&app)).expect("the command must succeed");

    // Assert: every field equals its spec oracle. Identifier, author,
    // license and repository are UPD-005/REL-004 literals (never
    // derived from the production constants — a mutated source fails
    // here); `data_dir` is the harness's temp root (the D-049
    // managed-`DataRoot` seam — the real `~/.local/share/...` never
    // enters a test); `db_created_fresh` is true — the state this
    // arrangement produces. The version is compared against
    // `env!("CARGO_PKG_VERSION")`, NOT a literal — deliberately: the
    // value is defined by the build and changes with every release,
    // so a literal oracle would turn each release into a red test;
    // the macro instead pins the plumbing (the DTO carries the crate
    // version macro the command reads), and the crate↔tauri.conf.json
    // drift risk is pinned by the unit test beside the DTO.
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.identifier, "pro.yepbro.eunomiasync");
    assert_eq!(info.author, "Den Brooks <denis@yepbro.pro>");
    assert_eq!(info.license, "MIT OR Apache-2.0");
    assert_eq!(info.repository, "https://github.com/yepbro/EunomiaSync");
    assert_eq!(
        info.data_dir,
        temp.path().display().to_string(),
        "the data dir is the managed DataRoot (the test temp root)"
    );
    assert!(info.db_created_fresh);
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn app_info_reports_an_existing_database_as_not_created_fresh() {
    // The other state of the flag, driven through the same real seam
    // (`Db::open`): a root whose database already exists — created by
    // a first open, closed, then opened again, exactly the shape of
    // every launch after the first. The open that produces the
    // managed handle must observe an existing file and answer false.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the layout in a temp directory must work");
    let first = crate::db::Db::open(temp.path()).expect("the bootstrap open must succeed");
    assert!(
        first.created_fresh(),
        "the bootstrap open itself must observe the fresh state"
    );
    drop(first); // closes the connection — the database file remains
    let second = crate::db::Db::open(temp.path()).expect("the second open must succeed");
    let root = temp.path().to_path_buf();
    let app = app_info_app(second, root);

    // Act
    let info = invoke_app_info(&main_webview(&app)).expect("the command must succeed");

    // Assert: only the flag differs — the facts sheet stays whole.
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.identifier, "pro.yepbro.eunomiasync");
    assert!(
        !info.db_created_fresh,
        "an existing database is not 'created from scratch at this launch'"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn app_info_answers_without_a_database() {
    // The non-DB half of the contract (the D-049 pattern): recovery
    // mode manages `DataRoot` but no `Db`, and the About window must
    // still get its facts — the flag degrades to false (no fresh
    // database was created; the recovery screen is the state display),
    // it does not fail the whole sheet.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the layout in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::app_info])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(data_dirs::DataRoot(temp.path().to_path_buf()));

    let info = invoke_app_info(&main_webview(&app)).expect("the command must succeed");

    assert_eq!(info.identifier, "pro.yepbro.eunomiasync");
    assert_eq!(
        info.data_dir,
        temp.path().display().to_string(),
        "the data dir is served from DataRoot, not from a database"
    );
    assert!(
        !info.db_created_fresh,
        "no database running means no fresh database was created"
    );
    assert!(
        !data_dirs::db_file_path(temp.path()).exists(),
        "the non-Db path must not create the database file — answering \
         the sheet is a read of constants and managed state only"
    );
    drop(app);
    drop(temp);
}

#[test]
fn a_preexisting_empty_database_file_is_created_fresh() {
    // The 0-byte half of the observation predicate: an empty file at
    // the §5.5 path is "no previous database" exactly like a missing
    // one (the shape a crashed first launch or a gate's bare
    // connection leaves behind, D-040) — the open bootstraps it, and
    // the flag answers true. The counter-case (a non-empty file
    // answering false) is the neighboring test's second open.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the layout in a temp directory must work");
    std::fs::write(data_dirs::db_file_path(temp.path()), b"")
        .expect("creating the empty fixture file must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening over an empty file must bootstrap, not fail");
    let root = temp.path().to_path_buf();
    let app = app_info_app(database, root);

    let info = invoke_app_info(&main_webview(&app)).expect("the command must succeed");

    assert!(
        info.db_created_fresh,
        "a 0-byte file is an empty database: bootstrapping it is a \
         fresh creation"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn app_info_carries_the_root_launch_flag() {
    // NFR-023's flag on the About sheet's wire: present and bool-typed
    // (the typed DTO deserialization enforces both — a missing field or
    // a non-bool value fails it) and equal to what the platform policy
    // answers for this process's uid. That equality is a plumbing pin
    // in the D-057 `version`-macro shape — honest about its reach: it
    // bites under a root run (a hardcoded-false flag fails it there);
    // in an unprivileged run both sides read false, so the falsifiable
    // both-halves coverage lives in the driven test below. The
    // conditional if/else asserts whichever half of the card's
    // acceptance pair this environment produces — including the
    // under-root half when the suite runs privileged.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let root = temp.path().to_path_buf();
    let app = app_info_app(database, root);

    let info = invoke_app_info(&main_webview(&app)).expect("the command must succeed");

    assert_eq!(
        info.running_as_root,
        crate::commands::running_as_root(crate::commands::current_uid()),
        "the wire flag is what the platform policy answers for this \
         process's uid (NFR-023)"
    );
    #[cfg(target_os = "linux")]
    if crate::commands::current_uid() == 0 {
        assert!(
            info.running_as_root,
            "under root the flag is true (NFR-023)"
        );
    } else {
        assert!(
            !info.running_as_root,
            "a regular-user launch must not raise the root flag (NFR-023)"
        );
    }
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_sheets_root_flag_answers_the_uid() {
    // Both halves of the card's acceptance pair ("under root true,
    // under a regular user false"), falsifiable without privilege:
    // `app_info_impl` takes the uid the command read as a parameter,
    // so the sheet is driven with chosen uids — a hardcoded flag fails
    // one half in either direction. Off Linux the same driven uids pin
    // the platform twin (false regardless of uid, NFR-023's
    // Linux-only scope); only `DataRoot` is needed — the sheet is a
    // read of constants and managed state.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the layout in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::app_info])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(data_dirs::DataRoot(temp.path().to_path_buf()));

    let as_root = crate::commands::app_info_impl(app.handle(), 0).expect("uid 0 must succeed");
    let as_user =
        crate::commands::app_info_impl(app.handle(), 1000).expect("uid 1000 must succeed");

    #[cfg(target_os = "linux")]
    {
        assert!(
            as_root.running_as_root,
            "uid 0 is root: the sheet must carry the warning fact (NFR-023)"
        );
        assert!(
            !as_user.running_as_root,
            "a regular user's uid must not raise the root flag (NFR-023)"
        );
    }
    #[cfg(not(target_os = "linux"))]
    {
        assert!(
            !as_root.running_as_root,
            "off Linux even uid 0 does not raise the flag (NFR-023's \
             Linux-only scope)"
        );
        assert!(
            !as_user.running_as_root,
            "off Linux a regular uid does not raise the flag (NFR-023's \
             Linux-only scope)"
        );
    }
    drop(app);
    drop(temp);
}

#[test]
fn app_info_names_the_missing_data_root_state() {
    // The command's only reachable failure (an uninitialized-app bug):
    // the same fallback envelope the recovery commands answer for the
    // same missing state — code, default key, the `DataRootMissing`
    // diagnostic in details.
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::app_info])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    let error =
        invoke_app_info(&main_webview(&app)).expect_err("the command without DataRoot must fail");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("DataRootMissing"),
        "the diagnostic names the state, got {error}"
    );
}

#[test]
fn the_app_info_command_is_registered_with_the_specta_builder() {
    // The §7.10 System-module name, registered exactly once (the
    // D-034 wiring-pin rationale: without it, dropping the
    // registration would leave `export_bindings_are_up_to_date` green
    // while silently rewriting the committed bindings).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::app_info").count(),
        1,
        "lib.rs must register the `app_info` command exactly once (§7.10)"
    );
}

// ---------------------------------------------------------------------------
// `markdown_preview` (T-056 the pipeline, T-057 the warnings; §7.10
// Library module, SEC-005, D-15)
// ---------------------------------------------------------------------------

/// Invokes `markdown_preview` over the app's `main` webview,
/// deserializing the success payload into the typed DTO.
fn invoke_markdown_preview(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    text: &str,
) -> Result<crate::commands::MarkdownPreview, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request("markdown_preview", serde_json::json!({ "text": text })),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<crate::commands::MarkdownPreview>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

/// The mock app `markdown_preview` serves: the command is stateless
/// (no managed state, no database — the reason it answers in any mode,
/// recovery included).
fn markdown_preview_app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::markdown_preview])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app")
}

#[test]
fn markdown_preview_never_returns_script_or_external_images_as_markup() {
    // Arrange: the T-056 acceptance pair in one text — a `<script>`
    // block and an external image in BOTH spellings (Markdown syntax
    // and raw HTML, the latter in both cases), around a heading that
    // proves ordinary Markdown still renders.
    let app = markdown_preview_app();
    let text = "# Guide\n\n<script>alert('xss')</script>\n\n\
                ![logo](https://tracker.example/pixel.png)\n\n\
                <img src=\"https://ads.example/banner.png\">\n\n\
                <IMG SRC=\"https://ads.example/tall.png\">\n";

    // Act
    let preview =
        invoke_markdown_preview(&main_webview(&app), text).expect("the command must succeed");

    // Assert: no script or img element reaches the result as markup —
    // executable code (script) and resource references (img, either
    // spelling) are unrepresentable in the answer; the Markdown
    // spelling's remote URL must not survive anywhere, because nothing
    // in the output would be allowed to load it.
    assert!(
        !preview.html.contains("<script"),
        "script markup must not reach the result: {}",
        preview.html
    );
    assert!(
        !preview.html.contains("<img") && !preview.html.contains("<IMG"),
        "img markup must not reach the result: {}",
        preview.html
    );
    assert!(
        !preview.html.contains("tracker.example"),
        "the Markdown-spelling image URL must be gone entirely: {}",
        preview.html
    );
    // The raw-HTML spellings survive only as escaped visible text —
    // the SEC-005 escape requirement, observed on the wire.
    assert!(
        preview.html.contains("&lt;img src=") && preview.html.contains("&lt;IMG SRC="),
        "raw img tags must survive as escaped visible text: {}",
        preview.html
    );
    assert!(
        preview.html.contains("&lt;script&gt;"),
        "the script block must survive as escaped visible text: {}",
        preview.html
    );
    assert!(
        preview.html.contains("<h1>Guide</h1>"),
        "ordinary Markdown around the hostile fragments still renders: {}",
        preview.html
    );
    assert!(
        preview.html.contains("logo"),
        "the image alt text survives: {}",
        preview.html
    );
    drop(app);
}

#[test]
fn markdown_preview_escapes_raw_html_and_renders_markdown_over_ipc() {
    // Arrange: benign raw HTML (escaped, not kept as markup) next to a
    // fenced code block — the wire answer for the editor's mixed
    // drafts.
    let app = markdown_preview_app();
    let text = "see <b>bold</b>\n\n```\ncode <here>\n```";

    // Act
    let preview =
        invoke_markdown_preview(&main_webview(&app), text).expect("the command must succeed");

    // Assert: the raw tag is a visible literal, the fence renders with
    // its content escaped — both observable through the same result.
    assert!(
        preview.html.contains("&lt;b&gt;bold&lt;/b&gt;"),
        "raw HTML arrives as escaped text: {}",
        preview.html
    );
    assert!(
        !preview.html.contains("<b>"),
        "no raw tag passes as markup: {}",
        preview.html
    );
    assert!(
        preview.html.contains("<pre><code>") && preview.html.contains("&lt;here&gt;"),
        "the fenced block renders with escaped content: {}",
        preview.html
    );
    drop(app);
}

#[test]
fn markdown_preview_renders_empty_text_to_an_empty_result() {
    // Arrange: the §6.9-empty variant text (LIB-002's empty variant)
    // is a real editor state — and, trivially, clean text.
    let app = markdown_preview_app();

    // Act
    let preview =
        invoke_markdown_preview(&main_webview(&app), "").expect("the command must succeed");

    // Assert: the honest preview of nothing is nothing — no HTML, no
    // warnings (the T-057 empty half of the acceptance pair).
    assert_eq!(preview.html, "");
    assert!(
        preview.warnings.is_empty(),
        "empty text must carry no warnings: {:?}",
        preview.warnings
    );
    drop(app);
}

#[test]
fn markdown_preview_reports_an_unclosed_code_fence_over_ipc() {
    // Arrange: the T-057 acceptance test — an unclosed ``` fence in
    // the text the editor would send. The command must still SUCCEED
    // (warnings do not block anything) and carry the warning as data.
    let app = markdown_preview_app();
    let text = "para\n\n```\ncode that never closes\n";

    // Act
    let preview =
        invoke_markdown_preview(&main_webview(&app), text).expect("the command must succeed");

    // Assert: one warning of the spec's named kind, pointing at the
    // opening fence's line (3, one-based) — the piece of data the
    // editor UI renders; the swallowed text still renders as a code
    // block, proving the HTML half is unaffected by the warning.
    assert_eq!(
        preview.warnings,
        vec![crate::markdown::MarkdownWarning {
            kind: crate::markdown::MarkdownWarningKind::UnclosedCodeFence,
            line: 3,
        }],
        "the unclosed fence must arrive as one warning at line 3: {:?}",
        preview.warnings
    );
    assert!(
        preview.html.contains("<pre><code>") && preview.html.contains("code that never closes"),
        "the code block still renders: {}",
        preview.html
    );
    drop(app);
}

#[test]
fn markdown_preview_returns_no_warnings_for_clean_text_over_ipc() {
    // Arrange: the T-057 acceptance test's clean half — ordinary
    // Markdown with a closed fence, the shape the editor shows
    // without a warning.
    let app = markdown_preview_app();
    let text = "# Guide\n\n```rust\nlet x = 1;\n```\n\nDone.\n";

    // Act
    let preview =
        invoke_markdown_preview(&main_webview(&app), text).expect("the command must succeed");

    // Assert: no warnings, the render intact.
    assert!(
        preview.warnings.is_empty(),
        "clean text must carry no warnings: {:?}",
        preview.warnings
    );
    assert!(
        preview.html.contains("<h1>Guide</h1>") && preview.html.contains("<pre><code>"),
        "the ordinary render is intact: {}",
        preview.html
    );
    drop(app);
}

#[test]
fn the_markdown_preview_command_is_registered_with_the_specta_builder() {
    // The §7.10 Library-module name (D-009's registry), registered
    // exactly once (the D-034 wiring-pin rationale).
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::markdown_preview").count(),
        1,
        "lib.rs must register the `markdown_preview` command exactly once (§7.10)"
    );
}

// ---------------------------------------------------------------------------
// `dialog_pick_folder` + the SEC-003 check (T-073, §7.10 System
// module, §5.6 SEC-001/SEC-003, D-009)
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the picker and the SEC-003 check
/// serve: a real `Db` over a fresh temp root and the session allowlist
/// managed — the state the `setup` hook produces on a normal launch.
/// No command needs registration: the tests drive the command body
/// (`dialog_pick_folder_with`) and the validator directly, because the
/// production picker seam opens a real desktop dialog (module docs).
fn sec003_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::session_paths::SessionPaths::new());
    (temp, app)
}

/// Drives `dialog_pick_folder_with` over `app` with a test picker
/// answer — the seam the plugin half cannot be (module docs). Errors
/// are serialized into the same JSON value shape the IPC error
/// assertions read.
fn pick_folder_via_dialog(
    app: &tauri::App<tauri::test::MockRuntime>,
    picker_answer: Result<Option<std::path::PathBuf>, crate::errors::AppError>,
) -> Result<Option<String>, serde_json::Value> {
    match tauri::async_runtime::block_on(crate::commands::dialog_pick_folder_with(
        app.handle().clone(),
        move |_app| picker_answer,
    )) {
        Ok(picked) => Ok(picked),
        Err(error) => Err(serde_json::to_value(&error).expect("AppError must serialize")),
    }
}

/// Drives the SEC-003 check (`validate_ui_path`) over `app`, in the
/// same JSON-error shape.
fn validate_ui_path_over(
    app: &tauri::App<tauri::test::MockRuntime>,
    path: &str,
) -> Result<std::path::PathBuf, serde_json::Value> {
    match tauri::async_runtime::block_on(crate::session_paths::validate_ui_path(app.handle(), path))
    {
        Ok(canonical) => Ok(canonical),
        Err(error) => Err(serde_json::to_value(&error).expect("AppError must serialize")),
    }
}

/// How many folders this app's session allowlist holds.
fn session_len(app: &tauri::App<tauri::test::MockRuntime>) -> usize {
    app.state::<crate::session_paths::SessionPaths>()
        .inner()
        .len()
}

/// Seeds one project whose stored identity is the REAL canonical
/// identity of `repo` — the SEC-003 project half looks the folder up
/// by the key computed from the UI path, so unlike the T-072 column
/// fixtures (display-only keys) this seeding must register the key the
/// validator computes. Returns the row id.
fn seed_registered_project(
    database: &crate::db::Db,
    repo: &std::path::Path,
    name: &'static str,
    label: &'static str,
    color: &'static str,
) -> String {
    let path = repo.display().to_string();
    let path_key = eunomia_fs::paths::canonical_path_key(repo)
        .expect("the freshly created repository folder must canonicalize");
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = column_project(&path, &path_key, name, label, color);
        eunomia_store::projects::create(tx, &new)
            .map(|project| project.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work")
}

#[test]
fn a_foreign_path_is_rejected_by_the_sec003_check() {
    // The card's acceptance criterion, driven at the boundary the
    // future path-consuming commands will call: a path that was NOT
    // picked in the dialog this session and is NOT a registered
    // project is rejected — whether the folder exists on disk (a
    // neighbor anyone could name) or not at all (a path that cannot
    // canonicalize cannot be on either allowlist: the dialog only
    // returns existing folders, the repository only stores canonical
    // paths). The rejection is the §7.10 E_VALIDATION envelope naming
    // the field, the sent value and the reason.
    let (temp, app) = sec003_app();
    let foreign = temp.path().join("foreign");
    std::fs::create_dir_all(&foreign).expect("the foreign fixture folder must be creatable");
    assert_eq!(session_len(&app), 0, "nothing was picked in this session");

    for (label, sent) in [
        ("an existing unpicked folder", foreign.display().to_string()),
        (
            "a path that cannot canonicalize",
            "/definitely/not/here/eunomia-t073".to_owned(),
        ),
    ] {
        let error = validate_ui_path_over(&app, &sent)
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
        assert_eq!(
            error["params"],
            serde_json::json!({
                "field": "path",
                "value": sent,
                "reason": "must be a registered project or a folder picked in the \
                           dialog this session (SEC-003)",
            }),
            "{label}: the params name the field, the sent value and the reason"
        );
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
fn a_picked_folder_is_remembered_and_passes_the_check() {
    // The session half of SEC-003 end to end: the dialog answer is
    // canonicalized, remembered (duplicates collapse), returned to the
    // UI as the canonical path, and validates — including through an
    // aliased spelling, because the check compares canonical forms.
    // The alias has two grades: a dot-segment spelling (`…/sub/../
    // picked`, resolved by the canonicalization itself) and, where
    // the platform can create one, a real symlink — DB-031's rule
    // (a link and its target are one folder) exercised by the
    // filesystem feature it names.
    let (temp, app) = sec003_app();
    let picked = temp.path().join("picked");
    std::fs::create_dir_all(&picked).expect("the picked fixture folder must be creatable");
    std::fs::create_dir_all(temp.path().join("sub"))
        .expect("the alias intermediate folder must be creatable");
    let canonical = eunomia_fs::paths::canonicalize(&picked)
        .expect("the fresh fixture folder must canonicalize");

    // Act: the user picks the folder — twice, to pin the dedupe.
    let first = pick_folder_via_dialog(&app, Ok(Some(picked.clone())))
        .expect("the pick must succeed")
        .expect("a completed pick answers a path");
    pick_folder_via_dialog(&app, Ok(Some(picked.clone()))).expect("the second pick must succeed");

    // Assert: the answer is the canonical form, the list holds ONE
    // entry, and the exact folder validates.
    assert_eq!(first, canonical.display().to_string());
    assert_eq!(session_len(&app), 1, "a repeated pick is one entry");
    let validated = validate_ui_path_over(&app, &picked.display().to_string())
        .expect("a picked folder passes the SEC-003 check");
    assert_eq!(validated, canonical);

    // A dot-segment spelling canonicalizes onto the same folder and
    // passes too.
    let dot_alias = temp.path().join("sub").join("..").join("picked");
    let via_dots = validate_ui_path_over(&app, &dot_alias.display().to_string())
        .expect("a dot-segment alias of a pick is the picked folder");
    assert_eq!(via_dots, canonical);

    // A real symlink is the same folder by DB-031 — tested where the
    // platform can create one unprivileged (the `not_readable`
    // fixture's cfg twin).
    #[cfg(unix)]
    {
        let link = temp.path().join("link-to-picked");
        std::os::unix::fs::symlink(&picked, &link).expect("the symlink fixture must be creatable");
        let via_link = validate_ui_path_over(&app, &link.display().to_string())
            .expect("a symlink to a picked folder is the picked folder (DB-031)");
        assert_eq!(via_link, canonical);
    }

    // The boundary the project half pins for its rows holds for the
    // session half too: a pick allowlists the exact folder, never the
    // tree below it — a subfolder of the pick is rejected, exactly
    // like a stranger.
    let inner = picked.join("inner");
    std::fs::create_dir_all(&inner).expect("the subfolder fixture must be creatable");
    let error = validate_ui_path_over(&app, &inner.display().to_string())
        .expect_err("a subfolder of a pick is not the pick");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn an_empty_or_blank_path_is_rejected_not_matched_loudly() {
    // The degenerate inputs of a path field: an empty string and a
    // blank one. Neither can canonicalize onto an allowlisted folder
    // (canonicalize refuses them), so both are the same E_VALIDATION
    // rejection — never a panic, and never a silent wildcard match an
    // empty-key comparison might produce.
    let (temp, app) = sec003_app();
    for sent in ["", " "] {
        let error = validate_ui_path_over(&app, sent)
            .err()
            .unwrap_or_else(|| panic!("{sent:?} must be rejected"));
        assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
        assert_eq!(
            error["params"]["value"],
            serde_json::json!(sent),
            "the sent value travels back: {error}"
        );
    }
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_registered_projects_folder_passes_the_check() {
    // The registered-project half of SEC-003: the repository's own
    // identity lookup (DB-015, active AND archived — an archived
    // project is still registered) answers for the exact folder, and
    // only for the exact folder: a subfolder of a registered project
    // is NOT the project (SEC-003 allowlists folders, not trees).
    let (temp, app) = sec003_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let alpha = repo_dir(temp.path(), "alpha");
    seed_registered_project(&database, &alpha, "Alpha", "ALP", "red");
    let beta = repo_dir(temp.path(), "beta");
    let beta_id = seed_registered_project(&database, &beta, "Beta", "BET", "green");
    // Beta is archived — §6.4 keeps the row, and DB-015 keeps the key
    // unique across both states.
    let beta_for_archive = beta_id.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::archive(tx, &beta_for_archive)
            .map(|_| ())
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("archiving through the repository must work");

    // The active project's folder validates.
    let validated = validate_ui_path_over(&app, &alpha.display().to_string())
        .expect("a registered project's folder passes the SEC-003 check");
    assert_eq!(
        validated,
        eunomia_fs::paths::canonicalize(&alpha).expect("the fixture must canonicalize")
    );
    // The archived project's folder validates the same way.
    validate_ui_path_over(&app, &beta.display().to_string())
        .expect("an archived project is still a registered project");
    // A subfolder of the registered project does not.
    let inner = alpha.join("inner");
    std::fs::create_dir_all(&inner).expect("the subfolder fixture must be creatable");
    let error = validate_ui_path_over(&app, &inner.display().to_string())
        .expect_err("a subfolder of a project is not the project");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    // And the session list stayed empty — registering is not picking.
    assert_eq!(session_len(&app), 0);
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_cancelled_pick_answers_null_and_remembers_nothing() {
    // §7.10: "returns the picked path (nothing on cancel)". The
    // nothing is observable twice — the UI answer is null, and the
    // session allowlist did not grow.
    let (temp, app) = sec003_app();
    let answer = pick_folder_via_dialog(&app, Ok(None)).expect("a cancelled pick is not an error");
    assert_eq!(answer, None, "cancel answers null, not an empty string");
    assert_eq!(
        session_len(&app),
        0,
        "a cancelled pick is remembered for nobody"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_vanished_pick_answers_project_unavailable() {
    // The vanish race the command documents: the dialog answered a
    // folder that cannot be canonicalized afterwards — the typed
    // E_PROJECT_UNAVAILABLE envelope with the picked path in params
    // and the OS refusal in details, and nothing remembered.
    let (temp, app) = sec003_app();
    let vanished = temp.path().join("never-created");
    let error = pick_folder_via_dialog(&app, Ok(Some(vanished.clone())))
        .expect_err("a vanished pick must fail honestly");
    assert_eq!(error["code"], serde_json::json!("E_PROJECT_UNAVAILABLE"));
    assert_eq!(
        error["message_key"],
        serde_json::json!("errors.project_unavailable")
    );
    assert_eq!(
        error["params"]["path"],
        serde_json::json!(vanished.display().to_string())
    );
    assert!(
        error["details"]["os_code"].is_number(),
        "the OS refusal travels in details: {error}"
    );
    assert_eq!(
        session_len(&app),
        0,
        "nothing is remembered for a vanished pick"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_session_half_answers_without_a_database() {
    // Recovery mode (no `Db` managed): the picker still works — it
    // needs no database — and the session half of the check still
    // answers for what was picked. A path on NEITHER half gets the
    // honest no-database fallback (the documented ordering), not a
    // validation rejection: every real consumer of the check needs
    // the database for its own operation.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::session_paths::SessionPaths::new());
    let picked = temp.path().join("picked");
    std::fs::create_dir_all(&picked).expect("the picked fixture folder must be creatable");
    let foreign = temp.path().join("foreign");
    std::fs::create_dir_all(&foreign).expect("the foreign fixture folder must be creatable");

    let answer = pick_folder_via_dialog(&app, Ok(Some(picked.clone())))
        .expect("the picker works without a database")
        .expect("the pick answered a path");
    assert_eq!(
        answer,
        eunomia_fs::paths::canonicalize(&picked)
            .expect("the fixture must canonicalize")
            .display()
            .to_string()
    );
    validate_ui_path_over(&app, &picked.display().to_string())
        .expect("the session half needs no database");
    let error = validate_ui_path_over(&app, &foreign.display().to_string())
        .expect_err("a path on neither half must fail");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest state answer, got {error}"
    );
}

#[test]
fn the_missing_session_state_answers_the_honest_fallback() {
    // The uninitialized-app bug the state helpers answer honestly
    // (the `data_root` pattern): a database without the session
    // allowlist — `setup` manages the allowlist before anything that
    // could fail, so this state cannot occur in a real launch. Both
    // the pick bookkeeping and the check answer the recorded fallback
    // with a diagnostic naming the state, never a panic.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    let picked = temp.path().join("picked");
    std::fs::create_dir_all(&picked).expect("the picked fixture folder must be creatable");

    let error = pick_folder_via_dialog(&app, Ok(Some(picked.clone())))
        .expect_err("the pick bookkeeping must fail without the allowlist state");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("SessionPathsMissing"),
        "the diagnostic names the state, got {error}"
    );
    let error = validate_ui_path_over(&app, &picked.display().to_string())
        .expect_err("the check must fail without the allowlist state");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("SessionPathsMissing")
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_dialog_pick_folder_command_is_registered_with_the_specta_builder() {
    // The §7.10 System-module name (D-009's fixed registry), registered
    // exactly once (the D-034 wiring-pin rationale) — the bindings
    // carry `dialogPickFolder` to the frontend through it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::dialog_pick_folder").count(),
        1,
        "lib.rs must register the `dialog_pick_folder` command exactly once (§7.10, D-009)"
    );
}
