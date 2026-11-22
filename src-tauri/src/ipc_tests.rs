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
//!
//! The `project_add_manual` tests (T-074, §7.10 Projects module, §7.1
//! PRJ-001/003/004/010/012, scenario S1) drive the add flow end to
//! end over the mock runtime: the picks go through the T-073 dialog
//! seam (so SEC-003's session half is the real production path), the
//! repositories are real `.git` fixtures, and the seeding helpers of
//! the sections above write through the repositories the command
//! itself uses. The card's four acceptance cases are pinned by name —
//! repository root, subfolder (the PRJ-003 offer plus its
//! `accept_repo_root` confirm, and the nearest-root rule), adding
//! again (the duplicate message and the archive-restore offer, with
//! the database proven unchanged), and the non-Git folder
//! (`E_NOT_A_REPO`) — plus the SEC-003 gate ordering (a foreign
//! folder never reaches the `.git` check), the PRJ-012 copy from
//! changed `default.*` settings, the §6.9 write-access warning over
//! read-only fixtures, the honest no-database answer, and the
//! registration pin.
//!
//! The `exclusions_list`/`exclusion_remove` tests (T-078, §7.10
//! Projects module, §6.4 "Path exclusion", D-10/PRJ-013…018) pin the
//! D-10 "Excluded paths" screen's backend over the mock runtime: the
//! card's acceptance criterion composed with T-075's classifier end
//! to end (a real `.git` fixture whose path is excluded in the store
//! classifies `excluded`; after the command, the SAME scan +
//! classification classifies it `new`, §6.4's "the path is found by a
//! scan again" — the walker and classifier are driven directly, the
//! scan command layer T-076 does not exist yet), the list's every
//! field in the stable order, the empty-database and honest
//! no-database answers, the corrupt-cell fallback envelope, the
//! removal's one-transaction delete + journal (with the rollback
//! proof in the settings test's trigger shape), the typed
//! unknown-id refusal, and the registration pins.
//!
//! The `state://project-availability` tests (T-079, §7.10 events,
//! D-09/DB-032, D-084) drive the availability service over the mock
//! runtime with the event mounted the way `lib.rs`'s setup mounts it:
//! the card's acceptance criterion end to end (a real `.git` fixture
//! whose folder is moved away with `fs::rename` — the recheck marks
//! the project unavailable and the typed event carries exactly that
//! change; moving it back marks it available again), the diff-only
//! contract (a first healthy probe and an unchanged recheck emit
//! nothing — the baseline rule), DB-032's startup and window-focus
//! triggers through their seams (`spawn_recheck` awaited by handle,
//! `on_window_event` ruling on constructed window events), the
//! future sync pipeline's direct entry, the in-flight coalescing
//! slot (both a real pass parked mid-run on the gated database and
//! the simpler externally-held slot), the empty active list, the
//! honest no-database answer, and the registration pins.
//!
//! The `projects_reorder` tests (T-080, §7.10 Projects module, §7.1
//! PRJ-019/020, DB-005, LOG-001…004) use the `projects_app` shape with
//! the projects seeded through the T-071 repository: the card's
//! acceptance criterion — the new order survives a real database
//! REOPEN (the managed `Db` is dropped, the same data root is opened
//! again through `Db::open`, and the stored `sort_order` still answers
//! the new column order) — plus the archived-rows composition rule
//! (their slots are untouched by a column drag), the one-transaction
//! journal with its rollback proof, the verified no-op of a re-sent
//! unchanged order, every typed refusal form with the table proven
//! untouched, the corrupt-cell fallback, the empty and single-column
//! boundaries, the honest no-database answer, and the registration
//! pin.
//!
//! The `sync_running` gate tests (T-105, §5.4 SYS-004/SYS-005, §7.4
//! MAIN-030, §9.4 phase 0/phase 4, §10 `E_SYNC_RUNNING`) drive the
//! global flag through the acquire seam the future sync pipeline
//! (T-112+) will call: the card's criterion (a second start while
//! one runs is refused, and the refusal renders as the §7.10
//! `E_SYNC_RUNNING` envelope the commands answer with), each
//! project-changing command refused while an externally-held guard
//! keeps the flag up — with the read side proven alive, the tables
//! proven untouched, and the same call converging once the guard
//! drops — and the add flow's gate ordering (the mode gate precedes
//! SEC-003: during a sync even a validly-picked path meets
//! `E_SYNC_RUNNING`). The take-once/RAII/panic/parallel-race halves
//! of the seam itself live in `sync_running.rs`'s own suite.

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

// ---------------------------------------------------------------------------
// `project_add_manual` (T-074, §7.10 Projects module, §7.1
// PRJ-001/003/004/010/012, §6.9, §8 S1, §5.6 SEC-003, DB-015/DB-030/
// DB-042, D-07/D-08/D-29)
// ---------------------------------------------------------------------------

/// A mock app with `project_add_manual` registered over a fresh
/// database and the SEC-003 session allowlist managed — the
/// healthy-app state the add flow serves (the `sec003_app` shape plus
/// the command).
fn add_manual_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::project_add_manual
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::session_paths::SessionPaths::new());
    // The SYS-005 gate state the healthy app manages at startup
    // (T-105): without it the add flow would meet the gate's honest
    // missing-state fallback instead of the real path under test.
    app.manage(crate::sync_running::SyncRunning::new());
    (temp, app)
}

/// Invokes `project_add_manual` with the given picked `path` and the
/// `acceptRepoRoot` wire value (`null` omitted is the plain call),
/// deserializing the success payload into the typed DTO.
fn invoke_project_add_manual(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    path: &str,
    accept_repo_root: serde_json::Value,
) -> Result<crate::commands::ProjectAddResult, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "project_add_manual",
            serde_json::json!({ "path": path, "acceptRepoRoot": accept_repo_root }),
        ),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<crate::commands::ProjectAddResult>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

/// The pick half of every add test: the folder goes through the real
/// dialog seam (`dialog_pick_folder_with`), so the session allowlist
/// holds exactly what a production pick would leave — the canonical
/// form the SEC-003 gate then accepts.
fn pick_folder(app: &tauri::App<tauri::test::MockRuntime>, folder: &std::path::Path) -> String {
    pick_folder_via_dialog(app, Ok(Some(folder.to_path_buf())))
        .expect("the pick must succeed")
        .expect("a completed pick answers a path")
}

/// Every project row, read through the repository the command writes
/// (the seeding path's store twin) — what the "database unchanged"
/// assertions count and inspect.
fn stored_projects(database: &crate::db::Db) -> Vec<eunomia_store::projects::Project> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::list(tx, eunomia_store::projects::ListScope::All)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("reading the projects through the repository must work")
}

/// Archives a project through the repository (§6.4) — the state the
/// DB-015 restore offer answers from.
fn archive_project(database: &crate::db::Db, id: String) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::archive(tx, &id)
            .map(|_| ())
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("archiving through the repository must work")
}

/// Seeds one §6.5 setting through the store — the PRJ-012 test
/// changes the `default.*` keys the new project copies.
fn seed_setting(
    database: &crate::db::Db,
    key: eunomia_store::settings::SettingKey,
    value: eunomia_store::settings::SettingValue,
) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        // The closure is `FnMut` (the executor may replay it on a busy
        // retry, D-050), so the value is cloned per attempt rather
        // than consumed.
        eunomia_store::settings::set(tx, key, value.clone())
            .map_err(crate::commands::settings_write_to_app_error)
    }))
    .expect("seeding a setting through the store must work");
}

#[test]
fn project_add_manual_registers_a_repository_root_with_all_fields_and_the_journal() {
    // AC case 1 of the card ("repository root"): a picked repository
    // root is registered with every field the entry carries — the
    // canonical path (DB-030), the folder-name display name (S1), the
    // DB-042 label, a palette color by D-29, `added_via = manual`, the
    // appended sort_order, the §6.5 default write settings (PRJ-012)
    // — and exactly one `project.added` journal entry (LOG-001…004,
    // D-051). A second add appends (sort_order 2) and, by D-29's
    // count-0 rule, lands on a different color.
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let repo = repo_dir(temp.path(), "eunomia-repo");
    let picked = pick_folder(&app, &repo);

    let result = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("adding a repository root must succeed");
    assert_eq!(result.outcome, "added");
    assert!(result.offered_root.is_none() && result.existing.is_none());
    let first = result.project.expect("the added arm carries the project");
    assert_eq!(
        first.path,
        eunomia_fs::paths::canonicalize(&repo)
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(first.name, "eunomia-repo");
    assert_eq!(first.label, "EUNOMI", "DB-042: the first six upper-cased");
    let palette: Vec<&str> = eunomia_core::project::Color::ALL
        .iter()
        .map(|color| color.as_str())
        .collect();
    assert!(palette.contains(&first.color.as_str()), "{}", first.color);
    assert_eq!(first.added_via, "manual");
    assert_eq!(first.sort_order, 1, "the first row appends at 1");
    // PRJ-012/§6.5: the four write settings, straight from the
    // untouched defaults.
    assert_eq!(first.scheme, "A");
    assert_eq!(first.base_name, "AGENTS.md");
    assert!(!first.extra_local_enabled);
    assert_eq!(first.extra_local_position, "end");
    assert!(!first.extra_project_enabled);
    assert_eq!(first.extra_project_position, "end");
    // The writable fixture carries no §6.9 warning.
    assert_eq!(first.write_access_warning, None);

    // The journal: exactly one entry, the flow's own event.
    let entries = journal_entries(&database);
    assert_eq!(entries.len(), 1, "one add, one entry");
    assert_eq!(entries[0].action.as_str(), "project.added");
    assert_eq!(entries[0].level.as_str(), "info");
    assert_eq!(entries[0].visibility.as_str(), "user");
    assert_eq!(entries[0].result.as_str(), "ok");
    assert_eq!(
        entries[0]
            .entity
            .as_ref()
            .map(|(kind, id)| (kind.as_str(), id.as_str())),
        Some(("project", first.id.as_str()))
    );
    assert_eq!(entries[0].project_id.as_deref(), Some(first.id.as_str()));

    // A second repository appends and takes a different color (the
    // count-0 candidates are every color but the first one's).
    let second_repo = repo_dir(temp.path(), "beta");
    let second_picked = pick_folder(&app, &second_repo);
    let second = invoke_project_add_manual(&webview, &second_picked, serde_json::json!(null))
        .expect("the second add must succeed")
        .project
        .expect("the added arm carries the project");
    assert_eq!(second.sort_order, 2);
    assert_ne!(second.color, first.color, "D-29: a still-unused color wins");
    assert_eq!(journal_entries(&database).len(), 2);

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_new_project_copies_its_write_settings_from_the_changed_defaults() {
    // PRJ-012/DEF-005: "For a new project, values are copied from the
    // default settings" — the copy reads whatever `default.*` holds
    // at the add, not the §6.5 table constants (which AC case 1
    // already pinned).
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    use eunomia_store::settings::{
        BaseName, ExtraFile, Position, Scheme, SettingKey, SettingValue,
    };
    seed_setting(
        &database,
        SettingKey::DefaultScheme,
        SettingValue::Scheme(Scheme::B),
    );
    seed_setting(
        &database,
        SettingKey::DefaultBaseName,
        SettingValue::BaseName(BaseName::ClaudeMd),
    );
    seed_setting(
        &database,
        SettingKey::DefaultExtraLocal,
        SettingValue::ExtraFile(ExtraFile {
            enabled: true,
            position: Position::Start,
        }),
    );
    seed_setting(
        &database,
        SettingKey::DefaultExtraProject,
        SettingValue::ExtraFile(ExtraFile {
            enabled: true,
            position: Position::End,
        }),
    );

    let repo = repo_dir(temp.path(), "custom-defaults");
    let picked = pick_folder(&app, &repo);
    let project = invoke_project_add_manual(&main_webview(&app), &picked, serde_json::json!(null))
        .expect("the add must succeed")
        .project
        .expect("the added arm carries the project");

    assert_eq!(project.scheme, "B");
    assert_eq!(project.base_name, "CLAUDE.md");
    assert!(project.extra_local_enabled);
    assert_eq!(project.extra_local_position, "start");
    assert!(project.extra_project_enabled);
    assert_eq!(project.extra_project_position, "end");

    // PRJ-012's second clause (AC = DEF-005's): changing the defaults
    // does NOT change existing projects — the row keeps its copied
    // values after the same keys move again.
    seed_setting(
        &database,
        SettingKey::DefaultScheme,
        SettingValue::Scheme(Scheme::A),
    );
    seed_setting(
        &database,
        SettingKey::DefaultBaseName,
        SettingValue::BaseName(BaseName::AgentsMd),
    );
    let stored = stored_projects(&database);
    assert_eq!(stored.len(), 1, "the add created exactly one row");
    assert_eq!(
        stored[0].scheme.as_str(),
        "B",
        "the copy is a copy, not a link"
    );
    assert_eq!(stored[0].base_name.as_str(), "CLAUDE.md");
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn project_add_manual_offers_the_repository_root_for_a_picked_subfolder() {
    // AC case 2 of the card ("subfolder"): a subfolder pick answers
    // the PRJ-003 offer — the repository root, nothing written — and
    // the confirm call re-sends the SAME picked path with
    // `acceptRepoRoot: true`, which registers the root (the offer is
    // a question, not a silent re-registration of another folder).
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let repo = repo_dir(temp.path(), "monorepo");
    let subfolder = repo.join("packages").join("inner");
    std::fs::create_dir_all(&subfolder).expect("the subfolder fixture must be creatable");
    let picked = pick_folder(&app, &subfolder);

    let offered = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("the subfolder call is a typed answer, not an error");
    assert_eq!(offered.outcome, "repo_root_offered");
    assert_eq!(
        offered.offered_root.as_deref(),
        Some(
            eunomia_fs::paths::canonicalize(&repo)
                .unwrap()
                .display()
                .to_string()
                .as_str()
        )
    );
    assert!(offered.project.is_none() && offered.existing.is_none());
    // The offer wrote nothing: no row, no journal entry.
    assert_eq!(stored_projects(&database).len(), 0);
    assert_eq!(journal_entries(&database).len(), 0);

    // The confirm: same picked path, the flag set — the root is
    // re-derived and registered.
    let confirmed = invoke_project_add_manual(&webview, &picked, serde_json::json!(true))
        .expect("the confirmed add must succeed");
    assert_eq!(confirmed.outcome, "added");
    assert_eq!(
        confirmed.project.as_ref().expect("the added arm").path,
        eunomia_fs::paths::canonicalize(&repo)
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(confirmed.project.as_ref().unwrap().name, "monorepo");

    // The confirm is idempotent (D-103's claim, pinned): a replayed
    // confirm converges on the typed duplicate answer, not a second
    // row — the root the replay re-derives is already registered.
    let replayed = invoke_project_add_manual(&webview, &picked, serde_json::json!(true))
        .expect("a replayed confirm is a typed answer");
    assert_eq!(replayed.outcome, "duplicate_active");
    assert_eq!(
        replayed.existing.expect("the duplicate names the row").id,
        confirmed.project.as_ref().unwrap().id
    );
    assert_eq!(stored_projects(&database).len(), 1);

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_root_offer_targets_the_nearest_repository() {
    // D-06 via PRJ-003: nested regular repositories are separate
    // projects, and `find_git_root` answers the NEAREST root — a
    // folder inside an inner repository is offered the inner root,
    // never the outer one; and the inner repository picked AT its own
    // root is added directly (no offer of the outer repository) —
    // D-06's "registered as separate projects" half.
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let outer = repo_dir(temp.path(), "outer");
    let inner = repo_dir(&outer, "inner");
    let deep = inner.join("src").join("deep");
    std::fs::create_dir_all(&deep).expect("the deep fixture must be creatable");
    let picked = pick_folder(&app, &deep);

    let offered = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("the deep subfolder is a typed offer");
    assert_eq!(
        offered.offered_root.as_deref(),
        Some(
            eunomia_fs::paths::canonicalize(&inner)
                .unwrap()
                .display()
                .to_string()
                .as_str()
        ),
        "the nearest .git wins, not the outer repository's"
    );

    // The inner repository picked at its own root: `root == picked`,
    // so no offer — the row lands directly (its own project, separate
    // from the outer repository's).
    let inner_picked = pick_folder(&app, &inner);
    let added = invoke_project_add_manual(&webview, &inner_picked, serde_json::json!(null))
        .expect("a repository root picked at itself is added directly");
    assert_eq!(added.outcome, "added");
    assert_eq!(
        added.project.as_ref().expect("the added arm").path,
        eunomia_fs::paths::canonicalize(&inner)
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(stored_projects(&database).len(), 1);

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn project_add_manual_answers_the_duplicate_message_and_the_archive_offer() {
    // AC case 3 of the card ("adding again"): an active project with
    // the same path_key answers the typed duplicate message, an
    // archived one answers the restore offer (DB-015/PRJ-001) — both
    // without an error envelope, and with the database provably
    // unchanged (no new row, no new journal entry).
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let repo = repo_dir(temp.path(), "twice");
    let picked = pick_folder(&app, &repo);
    let first = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("the first add must succeed")
        .project
        .expect("the added arm carries the project")
        .id;
    let rows_after_add = stored_projects(&database).len();
    let journal_after_add = journal_entries(&database).len();

    // The active duplicate: the message, the existing row's identity,
    // nothing changed.
    let duplicate = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("a duplicate is a typed answer, not an error");
    assert_eq!(duplicate.outcome, "duplicate_active");
    let existing = duplicate.existing.expect("the duplicate arm names the row");
    assert_eq!(existing.id, first);
    assert_eq!(existing.state, "active");
    assert_eq!(
        existing.path,
        eunomia_fs::paths::canonicalize(&repo)
            .unwrap()
            .display()
            .to_string()
    );
    assert!(duplicate.project.is_none() && duplicate.offered_root.is_none());
    assert_eq!(stored_projects(&database).len(), rows_after_add);
    assert_eq!(journal_entries(&database).len(), journal_after_add);

    // The archived duplicate: the restore offer (the future
    // `project_restore`, T-109, takes the carried id).
    archive_project(&database, first.clone());
    let archived = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("an archived duplicate is a typed offer, not an error");
    assert_eq!(archived.outcome, "archived_duplicate");
    let existing = archived.existing.expect("the offer names the row");
    assert_eq!(existing.id, first);
    assert_eq!(existing.state, "archived");
    assert_eq!(stored_projects(&database).len(), rows_after_add);
    assert_eq!(journal_entries(&database).len(), journal_after_add);

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_folder_whose_derived_default_label_is_unstorable_is_rejected_naming_the_label() {
    // D-079's recorded counterexample, at the command boundary: names
    // reserve no control-character rule, so a repository folder named
    // with a leading control character is a LEGAL name whose DB-042
    // derivation ("\u{7}REPO") is a label §6.9 rejects — the command's
    // honest answer is the E_VALIDATION envelope naming the LABEL
    // field, not a panic and not a stored row (the round-2 review
    // finding: the branch was implemented but unpinned). Unix-gated:
    // the fixture needs a control character in a directory name.
    #[cfg(unix)]
    {
        let (temp, app) = add_manual_app();
        let database = app.state::<crate::db::Db>().inner().clone();
        let webview = main_webview(&app);
        let repo = temp.path().join("\u{7}repo");
        std::fs::create_dir_all(repo.join(".git"))
            .expect("the control-named repository fixture must be creatable");
        let picked = pick_folder(&app, &repo);

        let error = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
            .expect_err("an unstorable derived label must be rejected");
        assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
        assert_eq!(
            error["params"]["field"],
            serde_json::json!("label"),
            "the failed field is the derived label, not the name"
        );
        assert_eq!(error["params"]["value"], serde_json::json!("\u{7}REPO"));
        assert_eq!(stored_projects(&database).len(), 0);
        assert_eq!(journal_entries(&database).len(), 0);
        // Windows teardown rule: the app's managed `Db` closes first.
        drop(app);
        drop(temp);
    }
}

#[test]
fn project_add_manual_rejects_a_folder_outside_any_repository_with_e_not_a_repo() {
    // AC case 4 of the card ("non-Git folder"): a picked folder with
    // no `.git` in itself or any ancestor is §10's `E_NOT_A_REPO` —
    // the full envelope (code, default key, the picked path in
    // params, no details: a fact about the tree, not an OS failure) —
    // and nothing reaches the database.
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let plain = temp.path().join("plain");
    std::fs::create_dir_all(&plain).expect("the plain fixture must be creatable");
    let picked = pick_folder(&app, &plain);

    let error = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect_err("a folder outside any repository must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_NOT_A_REPO"), "{error}");
    assert_eq!(error["message_key"], serde_json::json!("errors.not_a_repo"));
    assert_eq!(
        error["params"],
        serde_json::json!({ "path": picked }),
        "the picked path is the §10 log line's subject"
    );
    assert_eq!(error["details"], serde_json::json!(null));
    assert_eq!(stored_projects(&database).len(), 0);
    assert_eq!(journal_entries(&database).len(), 0);

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_foreign_path_is_rejected_before_the_repository_checks_run() {
    // SEC-003 as the input gate (§5.6): a path that was neither picked
    // this session nor is a registered project is rejected with
    // `E_VALIDATION` BEFORE the flow runs — proven here by sending a
    // REAL repository folder unpicked: the answer is the gate's
    // rejection, not `E_NOT_A_REPO`'s and not an add.
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let never_picked_repo = repo_dir(temp.path(), "unpicked-repo");

    // A real repository folder this session never picked: the answer
    // is the gate's rejection, not `E_NOT_A_REPO`'s and not an add —
    // which also proves the ordering (the gate runs before the `.git`
    // walk).
    let error = invoke_project_add_manual(
        &webview,
        &never_picked_repo.display().to_string(),
        serde_json::json!(null),
    )
    .expect_err("a path from nowhere must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("path"),
        "the SEC-003 rejection names the field"
    );
    assert!(
        error["params"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("SEC-003")),
        "the rejection names the policy, got: {error}"
    );
    assert_eq!(stored_projects(&database).len(), 0);
    assert_eq!(journal_entries(&database).len(), 0);

    // The D-086-addendum non-UTF-8 seam moved UP a boundary: the pick
    // itself is now refused (the dialog canonicalizes before
    // remembering, and a non-UTF-8 canonical form is the typed
    // encoding rejection), so the old flow — pick remembered with the
    // real bytes, add rejected over the lossy answer — is gone. The
    // whole seam, pick and add halves, is pinned by
    // `a_non_utf8_folder_is_refused_at_the_dialog_and_add_boundaries`
    // (Linux-only, same fixture family).

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

/// The D-086 addendum (owner ruling 2026-11-20, variant 2) at the
/// dialog and add boundaries, over a REAL Linux non-UTF-8 fixture:
/// the folder is picked through the real dialog seam and refused
/// typed — the encoding envelope, not an availability error — nothing
/// is remembered for it, the hand-sent lossy spelling gets the same
/// honest answer at the add gate, the database is untouched, and a
/// valid sibling still picks and adds. Linux-only: only Linux
/// filesystems accept the invalid-byte name (the `temp_files`
/// platform posture).
#[cfg(target_os = "linux")]
#[test]
fn a_non_utf8_folder_is_refused_at_the_dialog_and_add_boundaries() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let raw = OsStr::from_bytes(b"repo\xff");
    let non_utf8 = temp.path().join(raw);
    std::fs::create_dir_all(non_utf8.join(".git"))
        .expect("the non-UTF-8 repository fixture must be creatable");
    let sibling = repo_dir(temp.path(), "sibling-repo");

    // The dialog boundary: the pick canonicalizes to a form whose
    // text is not valid UTF-8, and the bookkeeping refuses it with
    // the typed encoding envelope — E_VALIDATION naming the path
    // field and the honest reason. Nothing is remembered (the
    // allowlist does not grow), so no later command can name the
    // folder through SEC-003 either.
    let error = pick_folder_via_dialog(&app, Ok(Some(non_utf8.clone())))
        .expect_err("a non-UTF-8 pick is refused at the dialog boundary");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("path"),
        "the refusal names the path field"
    );
    assert!(
        error["params"]["value"]
            .as_str()
            .is_some_and(|value| value.contains('\u{FFFD}')),
        "the echoed value is the lossy spelling — a diagnostic, never stored: {error}"
    );
    assert!(
        error["params"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("UTF-8")),
        "the refusal names the encoding, got: {error}"
    );
    assert_eq!(
        session_len(&app),
        0,
        "nothing is remembered for a refused pick"
    );

    // The add-gate twin: the UI could only ever hold the LOSSY
    // spelling, and sending it to `project_add_manual` answers the
    // same honest encoding rejection (the D-100 answer, unchanged in
    // shape) — never an add, never an availability error.
    let lossy = non_utf8.to_string_lossy().into_owned();
    assert!(
        lossy.contains('\u{FFFD}'),
        "the fixture is lossy on the wire"
    );
    let error = invoke_project_add_manual(&webview, &lossy, serde_json::json!(null))
        .expect_err("a lossy spelling names no real folder");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert!(
        error["params"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("UTF-8")),
        "the rejection names the encoding, not the allowlist, got: {error}"
    );

    // The TYPED arm of the same gate — valid UTF-8 on the wire, real
    // bytes underneath: a UTF-8-named alias pointing at the
    // non-UTF-8 folder RESOLVES to an unrepresentable canonical
    // form, and the SEC-003 check answers the encoding rejection for
    // it too (the input string contains no U+FFFD, so this is the
    // typed branch, not the lossy-answer heuristic).
    let alias = temp.path().join("utf8-alias");
    std::os::unix::fs::symlink(&non_utf8, &alias)
        .expect("creating the UTF-8-named alias must work");
    let error = validate_ui_path_over(&app, &alias.display().to_string())
        .expect_err("an alias onto an unrepresentable folder must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert!(
        error["params"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("UTF-8")),
        "the typed refusal names the encoding, got: {error}"
    );
    assert_eq!(stored_projects(&database).len(), 0);
    assert_eq!(journal_entries(&database).len(), 0);

    // Valid paths are untouched: the sibling picks and adds exactly
    // as before the addendum.
    let picked = pick_folder(&app, &sibling);
    let added = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("a valid repository still adds")
        .project
        .expect("the added answer carries the project");
    let canonical =
        eunomia_fs::paths::canonicalize(&sibling).expect("the valid fixture must canonicalize");
    assert_eq!(
        added.path,
        canonical.display().to_string(),
        "the stored path is the lossless canonical form"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_read_only_repository_is_added_with_the_write_access_warning() {
    // §6.9's project-path row: "Write access is checked when adding
    // (warning)" — the D-094 verdicts travel on the added answer and
    // the project STAYS registered. Both warning classes: an existing
    // read-only target file (T-F5's fact, portable), and (Unix, not
    // under root) a folder that refuses new files.
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);

    // The read-only target file: the probe answers FileReadOnly.
    let file_repo = repo_dir(temp.path(), "ro-file");
    std::fs::write(file_repo.join("AGENTS.md"), b"hand-written")
        .expect("the fixture file must be creatable");
    let mut perms = std::fs::metadata(file_repo.join("AGENTS.md"))
        .expect("the fixture file must be statable")
        .permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(file_repo.join("AGENTS.md"), perms)
        .expect("marking the fixture read-only must work");
    let picked = pick_folder(&app, &file_repo);
    let project = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("a read-only target file is a warning, not a rejection")
        .project
        .expect("the added arm carries the project");
    assert_eq!(
        project.write_access_warning.as_deref(),
        Some("file_read_only")
    );
    assert_eq!(stored_projects(&database).len(), 1, "the project is added");

    // The unwritable folder: the probe answers FolderNotWritable.
    // Mode bits are not enforced for root, so the precondition is
    // checked first (the D-088/D-094 test posture).
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let folder_repo = repo_dir(temp.path(), "ro-folder");
        let untestable = std::fs::metadata(&folder_repo).is_ok_and(|metadata| metadata.uid() == 0);
        if untestable {
            eprintln!(
                "skipping the folder_not_writable arm: running as root — \
                 mode bits are not enforced, the precondition cannot be \
                 set up"
            );
        } else {
            std::fs::set_permissions(&folder_repo, std::fs::Permissions::from_mode(0o555))
                .expect("removing the write bits must work");
            let picked = pick_folder(&app, &folder_repo);
            let result = invoke_project_add_manual(&webview, &picked, serde_json::json!(null));
            // Restore even when the assertions below panic, so
            // TempDir's teardown can remove the directory.
            std::fs::set_permissions(&folder_repo, std::fs::Permissions::from_mode(0o755))
                .expect("restoring mode bits must work");
            let project = result
                .expect("an unwritable folder is a warning, not a rejection")
                .project
                .expect("the added arm carries the project");
            assert_eq!(
                project.write_access_warning.as_deref(),
                Some("folder_not_writable")
            );
        }
    }

    // Scheme C targets `<STEM>.project.md`, never the main file
    // (D-20): the probe must warn about a read-only
    // `AGENTS.project.md` and stay SILENT about a read-only
    // `AGENTS.md` the app will never write (the round-1 review
    // finding — probing the row's `base_name` mis-targets every
    // scheme-C project).
    {
        use eunomia_store::settings::{Scheme, SettingKey, SettingValue};
        seed_setting(
            &database,
            SettingKey::DefaultScheme,
            SettingValue::Scheme(Scheme::C),
        );
        let scheme_c_repo = repo_dir(temp.path(), "scheme-c");
        std::fs::write(
            scheme_c_repo.join("AGENTS.md"),
            b"hand-written, never touched",
        )
        .expect("the main-file fixture must be creatable");
        let mut perms = std::fs::metadata(scheme_c_repo.join("AGENTS.md"))
            .expect("the main-file fixture must be statable")
            .permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(scheme_c_repo.join("AGENTS.md"), perms)
            .expect("marking the main file read-only must work");
        std::fs::write(scheme_c_repo.join("AGENTS.project.md"), b"the real target")
            .expect("the target fixture must be creatable");
        let mut perms = std::fs::metadata(scheme_c_repo.join("AGENTS.project.md"))
            .expect("the target fixture must be statable")
            .permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(scheme_c_repo.join("AGENTS.project.md"), perms)
            .expect("marking the target read-only must work");
        let picked = pick_folder(&app, &scheme_c_repo);
        let project = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
            .expect("the scheme-C add must succeed")
            .project
            .expect("the added arm carries the project");
        assert_eq!(
            project.write_access_warning.as_deref(),
            Some("file_read_only"),
            "the warning targets AGENTS.project.md, the file scheme C writes"
        );

        // The complement: a read-only MAIN file under scheme C warns
        // about nothing — the app never writes it.
        let quiet_repo = repo_dir(temp.path(), "scheme-c-quiet");
        std::fs::write(quiet_repo.join("AGENTS.md"), b"hand-written, never touched")
            .expect("the main-file fixture must be creatable");
        let mut perms = std::fs::metadata(quiet_repo.join("AGENTS.md"))
            .expect("the main-file fixture must be statable")
            .permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(quiet_repo.join("AGENTS.md"), perms)
            .expect("marking the main file read-only must work");
        let picked = pick_folder(&app, &quiet_repo);
        let project = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
            .expect("the scheme-C add must succeed")
            .project
            .expect("the added arm carries the project");
        assert_eq!(
            project.write_access_warning, None,
            "a read-only main file is not scheme C's target"
        );
    }

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn project_add_manual_answers_honestly_without_a_database() {
    // Recovery mode: the picker still works and the session half still
    // validates a picked folder (D-100's ordering), but every real
    // consumer needs the database — the honest answer is the recorded
    // `NoDatabase` fallback, not a pretend empty result.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::project_add_manual
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::session_paths::SessionPaths::new());
    // The SYS-005 gate state recovery mode also manages (T-105) —
    // the honest no-database answer below must come from
    // `managed_db`, not from the gate's missing-state fallback.
    app.manage(crate::sync_running::SyncRunning::new());
    let repo = repo_dir(temp.path(), "recovery-repo");
    let picked = pick_folder(&app, &repo);
    let webview = main_webview(&app);

    let error = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect_err("the add flow needs the database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest state answer"
    );
    // Windows teardown rule: nothing managed needs closing, but the
    // shape stays uniform.
    drop(app);
    drop(temp);
}

#[test]
fn the_project_add_manual_command_is_registered_with_the_specta_builder() {
    // The §7.10 Projects-module name (D-009's fixed registry),
    // registered exactly once (the D-034 wiring-pin rationale) — the
    // bindings carry `projectAddManual` to the frontend through it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::project_add_manual").count(),
        1,
        "lib.rs must register the `project_add_manual` command exactly once (§7.10, D-009)"
    );
}

// ---------------------------------------------------------------------------
// `exclusions_list` and `exclusion_remove` (T-078, §7.10 Projects
// module, §6.4 "Path exclusion", D-10/PRJ-013…018, D-093, D-098)
// ---------------------------------------------------------------------------

/// A mock app with both exclusion commands registered over a fresh
/// database in a temp root — the healthy-app state (the
/// `projects_app` shape).
fn exclusions_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::exclusions_list,
            crate::commands::exclusion_remove
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    // The SYS-005 gate state the healthy app manages at startup
    // (T-105) — the removal under test must meet the real gate.
    app.manage(crate::sync_running::SyncRunning::new());
    (temp, app)
}

/// Seeds one exclusion row through the repository (the `Db::run` path
/// a producing command takes — the archive flow's composition) and
/// returns its row id. The path and key arrive owned: the executor's
/// closure is `'static`, and the `FnMut` replay only borrows them.
fn seed_exclusion(
    database: &crate::db::Db,
    path: String,
    path_key: String,
    reason: eunomia_store::projects::ExclusionReason,
) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = eunomia_store::projects::NewExclusion {
            path: &path,
            path_key: &path_key,
            reason,
        };
        eunomia_store::projects::create_exclusion(tx, &new)
            .map(|row| row.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding an exclusion through the repository must work")
}

/// Reads the stored exclusions through the repository — the
/// "database unchanged" assertions' window into the table.
fn stored_exclusions(database: &crate::db::Db) -> Vec<eunomia_store::projects::Exclusion> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::list_exclusions(tx)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("reading the exclusions through the repository must work")
}

/// Invokes `exclusions_list` over the app's `main` webview,
/// deserializing the success payload into the typed DTO.
fn invoke_exclusions_list(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> Result<Vec<crate::commands::ExclusionEntry>, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request("exclusions_list", serde_json::json!({})),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<Vec<crate::commands::ExclusionEntry>>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

/// Invokes `exclusion_remove` with the `id` wire value over the app's
/// `main` webview.
fn invoke_exclusion_remove(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
) -> Result<(), serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request("exclusion_remove", serde_json::json!({ "id": id })),
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

/// Builds the ALG-053 [`ScanRegistry`] exactly the way the scan
/// command task (T-076) will build it from the store: the three
/// membership sets read through the repositories (D-093), keys as
/// stored (D-098's snapshot contract).
fn scan_registry_from(database: &crate::db::Db) -> eunomia_fs::scan_classify::ScanRegistry {
    let database = database.clone();
    let (active, archived, excluded) =
        tauri::async_runtime::block_on(database.run(move |tx| {
            let keys = |scope: eunomia_store::projects::ListScope| -> Result<Vec<String>, crate::errors::AppError> {
                eunomia_store::projects::list(tx, scope)
                    .map_err(crate::commands::projects_store_to_app_error)
                    .map(|rows| rows.into_iter().map(|project| project.path_key).collect())
            };
            let active = keys(eunomia_store::projects::ListScope::Active)?;
            let archived = keys(eunomia_store::projects::ListScope::Archived)?;
            let excluded: Vec<String> = eunomia_store::projects::list_exclusions(tx)
                .map_err(crate::commands::projects_store_to_app_error)?
                .into_iter()
                .map(|row| row.path_key)
                .collect();
            Ok((active, archived, excluded))
        }))
        .expect("building the registry from the store must work");
    eunomia_fs::scan_classify::ScanRegistry::new(&active, &archived, &excluded)
}

/// Runs the walker + classifier directly over `root` — the T-076
/// composition in miniature (the scan command layer does not exist
/// yet): the root is canonicalized first (the caller obligation
/// D-098 records), the walk uses §6.5's default depth and D-05's
/// default exclusion names exactly as T-076 will load them from the
/// settings, and every found repository is classified against
/// `registry`.
fn scan_classified_from(
    root: &std::path::Path,
    registry: &eunomia_fs::scan_classify::ScanRegistry,
) -> Vec<eunomia_fs::scan_classify::ClassifiedRepository> {
    let canonical =
        eunomia_fs::paths::canonicalize(root).expect("the fixture root must canonicalize");
    let options = eunomia_fs::scan::ScanOptions {
        max_depth: std::num::NonZeroUsize::new(6).expect("6 is non-zero"),
        exclude_names: [
            "node_modules",
            "vendor",
            ".git",
            "dist",
            "build",
            "target",
            ".venv",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    };
    let cancel = eunomia_fs::scan::CancelToken::new();
    let found = std::sync::Mutex::new(Vec::new());
    let outcome = eunomia_fs::scan::scan_repositories(&canonical, &options, &cancel, |event| {
        if let eunomia_fs::scan::ScanEvent::Repository { path } = event {
            found
                .lock()
                .expect("the test collector cannot be poisoned")
                .push(
                    eunomia_fs::scan_classify::classify_repository(path, registry)
                        .expect("the walker's UTF-8 gate keeps every repository keyable"),
                );
        }
    });
    assert_eq!(outcome, eunomia_fs::scan::ScanOutcome::Completed);
    found
        .into_inner()
        .expect("the test collector cannot be poisoned")
}

/// The classified verdict of the one repository whose path ends with
/// `name` — the per-name accessor the acceptance test asserts
/// through, so walker order never enters the assertion.
fn class_of(
    found: &[eunomia_fs::scan_classify::ClassifiedRepository],
    name: &str,
) -> eunomia_fs::scan_classify::ScanClass {
    found
        .iter()
        .find(|repo| repo.path().ends_with(name))
        .unwrap_or_else(|| panic!("the {name} repository must be found, got: {found:?}"))
        .class()
}

#[test]
fn removing_an_exclusion_makes_the_next_scan_classify_the_path_new() {
    // T-078's acceptance criterion, composed with T-075/D-098 end to
    // end: a real `.git` fixture on disk, a real exclusion row in the
    // store, the real command over IPC — the scan + classification
    // (driven directly: the scan command layer, T-076, does not exist
    // yet) answers `excluded` before the removal and `new` after it,
    // while a never-excluded sibling stays `new` throughout (§6.4:
    // "the path is found by a scan again").
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let excluded_repo = repo_dir(temp.path(), "excluded-repo");
    repo_dir(temp.path(), "free-repo");
    let canonical = eunomia_fs::paths::canonicalize(&excluded_repo)
        .expect("the fixture repository must canonicalize");
    let key = eunomia_fs::paths::path_key(&canonical).expect("the canonical text is valid UTF-8");
    let id = seed_exclusion(
        &database,
        canonical.display().to_string(),
        key,
        eunomia_store::projects::ExclusionReason::Manual,
    );

    // Before: the excluded repository classifies `excluded`, the
    // never-excluded sibling `new` (ALG-053 over the store-built
    // registry — both groups of PRJ-002's scan window).
    let before = scan_classified_from(temp.path(), &scan_registry_from(&database));
    assert_eq!(
        class_of(&before, "excluded-repo"),
        eunomia_fs::scan_classify::ScanClass::Excluded,
        "the store's exclusion row keeps the path out of the New group"
    );
    assert_eq!(
        class_of(&before, "free-repo"),
        eunomia_fs::scan_classify::ScanClass::New
    );

    // Act: the command under test, over the real IPC envelope.
    invoke_exclusion_remove(&webview, &id).expect("removing the exclusion must succeed");

    // After: the SAME scan + registry construction answers `new` for
    // the removed path — the physical delete is what changed the
    // verdict — and the sibling is untouched.
    let after = scan_classified_from(temp.path(), &scan_registry_from(&database));
    assert_eq!(
        class_of(&after, "excluded-repo"),
        eunomia_fs::scan_classify::ScanClass::New,
        "§6.4: after the removal the path is found by a scan again"
    );
    assert_eq!(
        class_of(&after, "free-repo"),
        eunomia_fs::scan_classify::ScanClass::New
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn removing_an_exclusion_of_an_archived_project_keeps_the_scan_verdict_archived() {
    // The PRJ-017 production shape (the round-1 review's ask):
    // archiving a scan-found project leaves an ARCHIVED row AND a
    // `removed_scanned` exclusion on the same `path_key` (§6.4, the
    // D-093 coexistence). D-098's fixed priority answers `archived`
    // — the project row is the truer fact — so removing the
    // exclusion alone must NOT flip the verdict to `new`: the
    // archived row still holds the key (DB-015), and only
    // `project_restore` (T-109) clears the last trace. The removal
    // itself is still §6.4's physical delete: the exclusion row is
    // gone, and the projects row is untouched.
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let repo = repo_dir(temp.path(), "archived-repo");
    let canonical =
        eunomia_fs::paths::canonicalize(&repo).expect("the fixture repository must canonicalize");
    let key = eunomia_fs::paths::path_key(&canonical).expect("the canonical text is valid UTF-8");
    // The archived project row (§6.4): created and archived through
    // the repository — the archive flow's own composition, the seam
    // T-109's `project_archive` will own. `seed_project` takes a
    // `'static` key, so the row is seeded inline with the runtime
    // key (the `column_project` shape, owned values into the `'static`
    // closure).
    let (seed_path, seed_key) = (canonical.display().to_string(), key.clone());
    let project_id = tauri::async_runtime::block_on(database.run(move |tx| {
        let new = column_project(&seed_path, &seed_key, "Archived", "ARCH", "slate");
        eunomia_store::projects::create(tx, &new)
            .map(|project| project.id)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding the project through the repository must work");
    archive_project(&database, project_id.clone());
    let id = seed_exclusion(
        &database,
        canonical.display().to_string(),
        key,
        eunomia_store::projects::ExclusionReason::RemovedScanned,
    );

    // Before: the coexistence classifies `archived` (D-098's
    // priority — the row over its exclusion side effect).
    let before = scan_classified_from(temp.path(), &scan_registry_from(&database));
    assert_eq!(
        class_of(&before, "archived-repo"),
        eunomia_fs::scan_classify::ScanClass::Archived,
        "the archived row wins over its exclusion side effect (PRJ-017)"
    );

    // Act: the command under test removes the exclusion half.
    invoke_exclusion_remove(&webview, &id).expect("removing the exclusion must succeed");

    // After: the verdict stays `archived` — the archived row still
    // holds the key — and the removal did its own work: no exclusion
    // row left, the projects row untouched in its archived state.
    let after = scan_classified_from(temp.path(), &scan_registry_from(&database));
    assert_eq!(
        class_of(&after, "archived-repo"),
        eunomia_fs::scan_classify::ScanClass::Archived,
        "the exclusion removal alone does not resurrect an archived project's key"
    );
    assert_eq!(
        stored_exclusions(&database).len(),
        0,
        "the exclusion row is gone"
    );
    let projects = stored_projects(&database);
    assert_eq!(projects.len(), 1, "the projects row is untouched");
    assert_eq!(
        projects[0].state,
        eunomia_store::projects::ProjectState::Archived
    );
    assert_eq!(projects[0].id, project_id);

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn exclusions_list_serves_every_field_in_the_stable_order_over_ipc() {
    // §6.2's every column on the wire — id, path, path_key, reason
    // (both vocabulary members, stored spellings), created_at — in
    // the repository's stable order: `created_at` first, the UUIDv7
    // id's insertion order inside one millisecond. The second half of
    // the test pins that tiebreak for real: both rows' `created_at`
    // are forced to ONE timestamp through the executor, so the order
    // the re-read returns can only be the id's.
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let first = seed_exclusion(
        &database,
        "/repos/first".to_owned(),
        "/repos/first".to_owned(),
        eunomia_store::projects::ExclusionReason::RemovedScanned,
    );
    let second = seed_exclusion(
        &database,
        "/repos/second".to_owned(),
        "/repos/second".to_owned(),
        eunomia_store::projects::ExclusionReason::Manual,
    );

    let webview = main_webview(&app);
    let entries = invoke_exclusions_list(&webview).expect("the listing must succeed");

    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [first.as_str(), second.as_str()],
        "the order is created-at/id — the insertion order here"
    );
    let entry = &entries[0];
    assert_eq!(entry.path, "/repos/first", "the canonical path (DB-030)");
    assert_eq!(entry.path_key, "/repos/first", "the identity key (DB-030)");
    assert_eq!(entry.reason, "removed_scanned", "the stored CHECK spelling");
    assert_eq!(entries[1].reason, "manual", "the second vocabulary member");
    assert!(
        entry.created_at.ends_with('Z')
            && entry.created_at.len() == "2026-11-20T13:03:22.123Z".len(),
        "the DB-002 UTC ISO-8601 shape, got: {}",
        entry.created_at
    );

    // The tiebreak pin: one shared timestamp for both rows (written
    // through the executor the command uses — `created_at` has no
    // CHECK, a plain UPDATE is the seam), and the order must still be
    // the id's — a created-at-only sort could answer either way.
    let (tie_first, tie_second) = (first.clone(), second.clone());
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE project_exclusions SET created_at = '2026-11-20T10:00:00.000Z'
             WHERE id = ?1 OR id = ?2",
            rusqlite::params![tie_first, tie_second],
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("tying the timestamps through the executor must work");
    let tied = invoke_exclusions_list(&webview).expect("the re-read must succeed");
    assert_eq!(
        tied.iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [first.as_str(), second.as_str()],
        "a same-millisecond tie resolves by the UUIDv7 id, in insertion order"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn exclusions_list_answers_an_empty_list_for_a_fresh_database() {
    // The empty app's honest answer: an empty list, not an error and
    // not a null payload the frontend has to special-case (the
    // `projects_list` pin).
    let (temp, app) = exclusions_app();
    let entries = invoke_exclusions_list(&main_webview(&app))
        .expect("a fresh database must answer successfully");
    assert_eq!(entries, Vec::new());
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_exclusion_commands_answer_honestly_without_a_database() {
    // Recovery mode (the `projects_list` pin): no `Db` managed — the
    // recorded D-037 fallback with the `NoDatabase` diagnostic, never
    // a pretend-empty list (or a successful no-op removal) that would
    // lie about the exclusions table being empty.
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::exclusions_list,
            crate::commands::exclusion_remove
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    // The SYS-005 gate state recovery mode also manages (T-105): the
    // honest `NoDatabase` answers below must come from `managed_db`,
    // not from the gate's missing-state fallback.
    app.manage(crate::sync_running::SyncRunning::new());
    let webview = main_webview(&app);
    let error =
        invoke_exclusions_list(&webview).expect_err("the no-database state must fail honestly");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
    let error = invoke_exclusion_remove(&webview, "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f")
        .expect_err("the removal needs the database too");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn a_corrupt_exclusion_reason_cell_answers_the_fallback_envelope_not_validation() {
    // The corrupt-cells wire contract (the T-027 round-2 rule, applied
    // to the exclusions table): a stored vocabulary cell outside its
    // CHECK vocabulary is corruption, NOT caller input — the read
    // fails with the recorded D-037 fallback, a `ProjectsCorrupt`
    // diagnostic, empty params and a class-only `os_message` (a
    // tampered cell is arbitrary database text; `params` and
    // `os_message` feed the log line and the localized message).
    // Written through the `ignore_check_constraints` test seam — the
    // schema CHECKs make such a cell unreachable through the API.
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    seed_exclusion(
        &database,
        "/repos/tampered".to_owned(),
        "/repos/tampered".to_owned(),
        eunomia_store::projects::ExclusionReason::Manual,
    );
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "PRAGMA ignore_check_constraints = 1;\n\
             UPDATE project_exclusions SET reason = 'bogus';\n\
             PRAGMA ignore_check_constraints = 0;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("tampering the reason cell through the executor must work");

    let error =
        invoke_exclusions_list(&main_webview(&app)).expect_err("a corrupt cell must fail the read");

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
        serde_json::json!("a stored reason cell is outside its vocabulary"),
        "the diagnostic names the column and class only, never the stored value"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn exclusion_remove_deletes_the_row_and_journals_the_path_in_one_transaction() {
    // §6.4's physical delete through the real command: the row is
    // gone, the list reflects it, and exactly one journal entry
    // records the removal inside the same transaction —
    // `exclusion.removed` at info/user/ok (the D-106 follow-up
    // member, D-119) whose entity names the removed exclusion row
    // and whose File detail carries its path, with no project id (no
    // project row is involved). Rows the T-078 era wrote as
    // `project.changed` keep reading — the store's compatibility
    // test pins that half.
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let removed = seed_exclusion(
        &database,
        "/repos/removed".to_owned(),
        "/repos/removed".to_owned(),
        eunomia_store::projects::ExclusionReason::RemovedScanned,
    );
    let kept = seed_exclusion(
        &database,
        "/repos/kept".to_owned(),
        "/repos/kept".to_owned(),
        eunomia_store::projects::ExclusionReason::Manual,
    );

    invoke_exclusion_remove(&webview, &removed).expect("the removal must succeed");

    // The row is physically gone; the sibling stays.
    let stored = stored_exclusions(&database);
    assert_eq!(
        stored.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        [kept.as_str()],
        "the physical delete removed exactly the addressed row"
    );
    let entries = invoke_exclusions_list(&webview).expect("the list must reflect the removal");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        [kept.as_str()]
    );

    // The journal: exactly one entry, the removal's own event.
    let journal = journal_entries(&database);
    assert_eq!(journal.len(), 1, "one removal, one entry");
    assert_eq!(journal[0].action.as_str(), "exclusion.removed");
    assert_eq!(journal[0].level.as_str(), "info");
    assert_eq!(journal[0].visibility.as_str(), "user");
    assert_eq!(journal[0].result.as_str(), "ok");
    assert_eq!(
        journal[0].entity,
        Some((
            eunomia_store::action_log::EntityType::Exclusion,
            removed.clone()
        )),
        "the entity columns name the removed exclusion row"
    );
    assert_eq!(journal[0].project_id, None, "no project row is involved");
    assert_eq!(
        journal[0].details,
        Some(serde_json::json!({ "path": "/repos/removed" })),
        "the File detail carries the removed path — the fact the screen shows"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn exclusion_remove_rejects_an_unknown_id_with_the_typed_envelope_and_changes_nothing() {
    // The typed NotFound refusal: an id no row matches (a stale
    // screen or a replayed click) is `E_VALIDATION` naming the field
    // — the honest §10 fit of the frozen registry — with the whole
    // operation rejected: the rows and the journal are provably
    // unchanged.
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    seed_exclusion(
        &database,
        "/repos/only".to_owned(),
        "/repos/only".to_owned(),
        eunomia_store::projects::ExclusionReason::Manual,
    );

    let error = invoke_exclusion_remove(&webview, "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f")
        .expect_err("an unknown id must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["message_key"], serde_json::json!("errors.validation"));
    assert_eq!(
        error["params"],
        serde_json::json!({
            "field": "id",
            "value": "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f",
            "reason": "no exclusion with this id",
        })
    );
    assert_eq!(stored_exclusions(&database).len(), 1, "no row was touched");
    assert_eq!(journal_entries(&database).len(), 0, "nothing was journaled");

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_failed_exclusion_remove_rolls_back_the_delete_with_its_journal_entry() {
    // D-051's in-transaction semantics, proven through the real
    // command (the settings rollback test's shape): the journal entry
    // shares the operation's transaction, so a failure AFTER the
    // DELETE succeeded rolls both back — the row survives and the
    // journal claims nothing. The failure is staged deterministically
    // inside the database: a trigger that rejects every journal
    // insert (RAISE(ABORT) → SQLITE_CONSTRAINT → the shared base
    // classifier's E_VALIDATION with the SQLite facts in details).
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let id = seed_exclusion(
        &database,
        "/repos/rollback".to_owned(),
        "/repos/rollback".to_owned(),
        eunomia_store::projects::ExclusionReason::Manual,
    );
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_every_journal_entry BEFORE INSERT ON action_log
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");

    let error =
        invoke_exclusion_remove(&webview, &id).expect_err("the staged abort must fail the removal");
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

    // Assert: the DELETE rolled back with the journal insert — the
    // row still stands, the journal still empty. No tails.
    assert_eq!(stored_exclusions(&database).len(), 1, "the row survived");
    assert_eq!(
        journal_entries(&database).len(),
        0,
        "no entry claimed a removal"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_exclusion_commands_are_registered_with_the_specta_builder() {
    // The §7.10 Projects-module names (D-009's fixed registry),
    // registered exactly once each (the D-034 wiring-pin rationale) —
    // the bindings carry `exclusionsList`/`exclusionRemove` to the
    // frontend through them.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for name in ["exclusions_list", "exclusion_remove"] {
        assert_eq!(
            lib.matches(&format!("commands::{name}")).count(),
            1,
            "lib.rs must register the `{name}` command exactly once (§7.10, D-009)"
        );
    }
}

// ---------------------------------------------------------------------------
// `state://project-availability` (T-079, §7.10 events, D-09/DB-032, D-084)
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the availability service serves
/// (the `projects_app` shape) with the availability state managed and
/// the event mounted — the registration `lib.rs`'s setup performs for
/// `tauri::Wry`, pinned here for the mock runtime the emit/listen
/// round trip needs (the specta builder is runtime-generic; the
/// `EventRegistry` mount is what makes `emit` resolve the spec's wire
/// name instead of panicking on a lookup).
fn availability_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::availability::AvailabilityState::new());
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::availability::ProjectAvailabilityEvent
        ])
        .mount_events(&app);
    (temp, app)
}

/// Subscribes the typed listener the frontend's generated
/// `events.stateProjectAvailability` mirrors, returning the receiver
/// every emitted payload lands on. Delivery to Rust listeners is
/// synchronous inside `emit`, so once a recheck call has returned, the
/// receiver already holds everything that pass emitted — the
/// `try_recv`/timeout-free assertions below are deterministic.
fn listen_availability(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> std::sync::mpsc::Receiver<crate::availability::ProjectAvailabilityEvent> {
    let (sender, receiver) = std::sync::mpsc::channel();
    tauri_specta::Event::listen(app, move |event| {
        let _ = sender.send(event.payload);
    });
    receiver
}

/// One expected diff entry, the typed payload's own shape — spelled
/// out per test, never computed from the code under test.
fn expected_change(
    project_id: &str,
    available: bool,
    reason: Option<&str>,
) -> crate::availability::ProjectAvailabilityChange {
    crate::availability::ProjectAvailabilityChange {
        project_id: project_id.to_owned(),
        available,
        unavailable_reason: reason.map(str::to_owned),
    }
}

#[test]
fn moving_a_project_folder_marks_it_unavailable_and_emits_the_change() {
    // The card's acceptance criterion (D-09): a registered project
    // with a real folder stays in the list when the folder moves —
    // the recheck marks it unavailable, the event carries exactly
    // that change, and moving the folder back marks it available
    // again. Also pins the diff-only contract around it: the first
    // healthy probe emits NOTHING (the baseline rule — a healthy
    // startup has nothing to tell the UI) and an unchanged recheck is
    // silent. This direct `recheck_and_emit` await is also the future
    // sync pipeline's entry (§9.4/T-118 will call it as DB-032's
    // before-sync checkpoint).
    let (temp, app) = availability_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let folder = repo_dir(temp.path(), "alpha");
    let project_id = seed_project(
        &database,
        folder.display().to_string(),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let events = listen_availability(&app);

    // First probe: healthy — no change against the implicit baseline,
    // nothing on the wire.
    let first = tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
        .expect("the first recheck must succeed");
    assert_eq!(first, Vec::new(), "a healthy first probe is no change");
    assert!(
        events.try_recv().is_err(),
        "no event may be emitted for an empty diff"
    );

    // The folder moves away — the D-09 case.
    let moved = temp.path().join("alpha-moved");
    std::fs::rename(&folder, &moved).expect("moving the fixture folder must work");
    let second =
        tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
            .expect("the recheck after the move must succeed");
    assert_eq!(
        second,
        vec![expected_change(&project_id, false, Some("missing"))],
        "the moved folder marks the project unavailable"
    );
    assert_eq!(
        events.try_recv().expect("the change must be emitted"),
        crate::availability::ProjectAvailabilityEvent {
            changes: vec![expected_change(&project_id, false, Some("missing"))],
        },
        "the event carries exactly the diff"
    );

    // Unchanged world: the repeat recheck is silent (idempotence —
    // what makes rechecking on every focus safe).
    let third = tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
        .expect("the unchanged recheck must succeed");
    assert_eq!(third, Vec::new());
    assert!(
        events.try_recv().is_err(),
        "an unchanged diff emits nothing"
    );

    // The folder returns: available again — the marking must clear,
    // so the recovery direction is a change too.
    std::fs::rename(&moved, &folder).expect("moving the fixture folder back must work");
    let fourth =
        tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
            .expect("the recheck after the restore must succeed");
    assert_eq!(
        fourth,
        vec![expected_change(&project_id, true, None)],
        "the returned folder marks the project available again"
    );
    assert_eq!(
        events.try_recv().expect("the recovery must be emitted"),
        crate::availability::ProjectAvailabilityEvent {
            changes: vec![expected_change(&project_id, true, None)],
        },
        "the event carries the recovery"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_missing_folder_on_the_first_probe_is_emitted_immediately() {
    // The other half of the baseline rule: a project FIRST probed
    // while its folder is already gone (a drive that was unplugged
    // before the launch) is a change at startup — D-09's marking must
    // not wait for a second pass.
    let (temp, app) = availability_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    // Seeded without ever creating the folder — a `.git` fixture that
    // does not exist.
    let project_id = seed_project(
        &database,
        temp.path().join("gone").display().to_string(),
        "/gone",
        "Gone",
        "GONE",
        "blue",
    );
    let events = listen_availability(&app);

    // The startup pass, awaited by its join handle for determinism
    // (production drops the handle — the task runs on).
    tauri::async_runtime::block_on(crate::availability::spawn_recheck(app.handle().clone()))
        .expect("the startup pass must not fail");
    assert_eq!(
        events
            .try_recv()
            .expect("the first missing verdict is news"),
        crate::availability::ProjectAvailabilityEvent {
            changes: vec![expected_change(&project_id, false, Some("missing"))],
        },
        "a first probe finding the folder gone marks the project at startup"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_focus_gain_triggers_the_recheck_and_other_window_events_do_not() {
    // DB-032's window check point through the `on_window_event` seam:
    // `Focused(true)` (which is also how a window's opening is
    // delivered) runs the recheck; losing focus and unrelated events
    // do not. The reaction is asserted on the seam's own outcome, and
    // the spawned pass is awaited through the handle the `Recheck`
    // arm carries — deterministic, no sleeps.
    let (temp, app) = availability_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let folder = repo_dir(temp.path(), "beta");
    let project_id = seed_project(
        &database,
        folder.display().to_string(),
        "/beta",
        "Beta",
        "BET",
        "green",
    );
    let events = listen_availability(&app);
    // Baseline the cache, so the later move is the one observable
    // change (the startup pass's job in production — done through the
    // service entry here because this test pins the window seam, not
    // the startup spawn).
    tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
        .expect("the baseline recheck must succeed");
    assert!(events.try_recv().is_err());

    // The folder moves; a focus gain must pick it up.
    std::fs::rename(&folder, temp.path().join("beta-moved"))
        .expect("moving the fixture folder must work");
    let reaction =
        crate::availability::on_window_event(app.handle(), &tauri::WindowEvent::Focused(true));
    match reaction {
        crate::availability::WindowEventReaction::Recheck(pass) => {
            tauri::async_runtime::block_on(pass).expect("the focus-triggered pass must not fail");
        }
        other => panic!("a focus gain must recheck, got {other:?}"),
    }
    assert_eq!(
        events
            .try_recv()
            .expect("the focus recheck must emit the move"),
        crate::availability::ProjectAvailabilityEvent {
            changes: vec![expected_change(&project_id, false, Some("missing"))],
        }
    );

    // The folder returns BEFORE the ignored events are driven: the
    // cache still says `missing` while the world is available again,
    // so any spurious recheck WOULD have a diff to emit — which is
    // what makes the silence below meaningful (round-2 review P3-1:
    // with the folder still moved, a recheck of an already-recorded
    // `missing` verdict is silent by construction and proves nothing).
    std::fs::rename(temp.path().join("beta-moved"), &folder)
        .expect("moving the fixture folder back must work");
    // Losing focus and an unrelated event are ignored — and emit
    // nothing.
    assert!(matches!(
        crate::availability::on_window_event(app.handle(), &tauri::WindowEvent::Focused(false)),
        crate::availability::WindowEventReaction::Ignored
    ));
    assert!(matches!(
        crate::availability::on_window_event(
            app.handle(),
            &tauri::WindowEvent::Resized(Default::default())
        ),
        crate::availability::WindowEventReaction::Ignored
    ));
    assert!(
        events.try_recv().is_err(),
        "an ignored window event must not recheck — a diff is pending, so a spurious pass would emit"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn an_in_flight_recheck_coalesces_a_second_spawn() {
    // The focus-storm guard, end to end over the real service: a pass
    // is held mid-run (parked on the one database connection, which
    // `Db::run` serializes — the gate below holds it), and while that
    // pass is genuinely in flight a second spawn must coalesce — no
    // probes, no event. Releasing the gate lets the parked pass finish
    // and emit. This is the round-1 review P1-1 regression test: the
    // first cut released the slot before the pass started, which the
    // in-flight observation below would have caught (`is_taken`
    // staying false while the pass is parked).
    let (temp, app) = availability_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    // The project's folder was never created: any real recheck has a
    // diff to emit.
    let project_id = seed_project(
        &database,
        temp.path().join("ghost").display().to_string(),
        "/ghost",
        "Ghost",
        "GHO",
        "amber",
    );
    let events = listen_availability(&app);
    let slot = app
        .state::<crate::availability::AvailabilityState>()
        .inner()
        .running
        .clone();

    // The gate: a `Db::run` operation that holds the connection and
    // parks until the test releases it. It signals once it is inside
    // the transaction, so "the gate is held" is deterministic.
    let (release_gate_tx, release_gate_rx) = std::sync::mpsc::channel::<()>();
    let (gate_held_tx, gate_held_rx) = std::sync::mpsc::channel::<()>();
    let gate_database = database.clone();
    let gate = tauri::async_runtime::spawn(async move {
        gate_database
            .run(move |_transaction| {
                gate_held_tx.send(()).expect("the test waits for the gate");
                release_gate_rx
                    .recv()
                    .expect("the test releases the gate before teardown");
                Ok(())
            })
            .await
    });
    gate_held_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the gate must reach its transaction");

    // The first pass: parks inside its own `Db::run` (the connection
    // is taken), holding the in-flight slot across that await. The
    // bounded wait observes the pass mid-run — the observable the
    // broken slot could not provide.
    let pass = crate::availability::spawn_recheck(app.handle().clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !slot.is_taken() {
        assert!(
            std::time::Instant::now() < deadline,
            "the spawned pass must take the in-flight slot (it is parked on the gated database)"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    // A second spawn while the first is in flight: coalesced — it
    // returns without probing, and nothing is emitted yet (the parked
    // pass has not even read the project list).
    tauri::async_runtime::block_on(crate::availability::spawn_recheck(app.handle().clone()))
        .expect("a coalesced spawn must not fail");
    assert!(
        events.try_recv().is_err(),
        "a coalesced spawn must not probe or emit"
    );

    // Releasing the gate lets the parked pass run to completion: the
    // diff is emitted, and the slot frees with the pass. The sender is
    // kept alive until after the pass: a dropped sender would fail the
    // gate's `recv` instead of releasing it.
    release_gate_tx
        .send(())
        .expect("the gate receiver waits for this release");
    // Two layers to unwrap: the join (the task ran) and the gated
    // operation's own `Db::run` result (the transaction committed).
    tauri::async_runtime::block_on(gate)
        .expect("the gate task must join")
        .expect("the gated operation must commit");
    tauri::async_runtime::block_on(pass).expect("the parked pass must finish cleanly");
    assert_eq!(
        events
            .try_recv()
            .expect("the in-flight pass emits its diff"),
        crate::availability::ProjectAvailabilityEvent {
            changes: vec![expected_change(&project_id, false, Some("missing"))],
        }
    );
    assert!(!slot.is_taken(), "the slot frees when the pass finishes");

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn an_externally_held_slot_coalesces_a_spawn_and_the_next_one_runs() {
    // The slot held from outside (the documented `pub(crate)` seam) —
    // the same skip behavior for a caller that cannot drive the
    // database gate, plus the self-healing half: the moment the slot
    // frees, the next spawn runs.
    let (temp, app) = availability_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    // The project's folder was never created: any real recheck has a
    // diff to emit.
    let project_id = seed_project(
        &database,
        temp.path().join("ghost").display().to_string(),
        "/ghost",
        "Ghost",
        "GHO",
        "amber",
    );
    let events = listen_availability(&app);
    let held = app
        .state::<crate::availability::AvailabilityState>()
        .inner()
        .running
        .clone()
        .try_take()
        .expect("the slot must be free before any recheck");

    // The spawned pass finds the slot taken and returns silently.
    tauri::async_runtime::block_on(crate::availability::spawn_recheck(app.handle().clone()))
        .expect("a coalesced spawn must not fail");
    assert!(
        events.try_recv().is_err(),
        "a coalesced spawn must not probe or emit"
    );

    // Freed, the next spawn does the work — the FULL payload, not just
    // "some event" (round-2 review P3-4: the pending diff is exactly
    // the one first-probe change).
    drop(held);
    tauri::async_runtime::block_on(crate::availability::spawn_recheck(app.handle().clone()))
        .expect("the pass after the slot freed must not fail");
    assert_eq!(
        events
            .try_recv()
            .expect("the pass after the slot freed emits the pending diff"),
        crate::availability::ProjectAvailabilityEvent {
            changes: vec![expected_change(&project_id, false, Some("missing"))],
        }
    );
    assert!(
        events.try_recv().is_err(),
        "exactly one pass ran, so exactly one event"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn an_empty_active_project_list_emits_nothing() {
    // The empty-world answer: no active projects means no probes, no
    // diff, no event — the recheck succeeds (an empty database is a
    // healthy state, not an error).
    let (temp, app) = availability_app();
    let events = listen_availability(&app);

    let changes =
        tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
            .expect("the empty-list recheck must succeed");
    assert_eq!(changes, Vec::new());
    assert!(events.try_recv().is_err());

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_recheck_without_a_database_answers_the_honest_no_database_error() {
    // Recovery mode (the T-024 state): no `Db` managed. The recheck
    // answers the same honest `NoDatabase` fallback every database
    // command does — not a panic, not a pretend-empty diff — and
    // emits nothing; the fire-and-forget spawn takes the same error
    // through its logging edge without failing its own task.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::availability::AvailabilityState::new());
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::availability::ProjectAvailabilityEvent
        ])
        .mount_events(&app);
    let events = listen_availability(&app);

    let error = tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
        .expect_err("the no-database state must fail honestly");
    assert_eq!(error.code.as_str(), "E_DB_LOCKED");
    assert_eq!(error.message_key, "errors.db_locked");
    assert_eq!(
        error.details.map(|details| details.os_kind),
        Some(Some("NoDatabase".to_owned())),
        "the diagnostic names the missing state"
    );
    assert!(events.try_recv().is_err());

    // The spawned trigger survives the same state (its failure goes
    // to the log facade, not the task's own result).
    tauri::async_runtime::block_on(crate::availability::spawn_recheck(app.handle().clone()))
        .expect("the spawned pass must complete even without a database");
    assert!(events.try_recv().is_err());

    // Windows teardown rule: no `Db` is managed here; the temp root
    // still closes last.
    drop(app);
    drop(temp);
}

#[test]
fn the_project_availability_event_and_its_triggers_are_registered_with_the_specta_builder() {
    // The §7.10 event plus DB-032's two wired check points, each
    // registered exactly once (the D-034 wiring-pin rationale — the
    // round-1 review P2-1 gap): without these pins, dropping the
    // event registration or either trigger call site would leave
    // `export_bindings_are_up_to_date` green (it silently rewrites the
    // committed bindings without the event) and every seam test green
    // (they drive the seams, not `lib.rs`'s call sites), while the
    // startup and window check points silently disappear.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("availability::ProjectAvailabilityEvent")
            .count(),
        1,
        "lib.rs must register the `state://project-availability` event exactly once (§7.10)"
    );
    assert_eq!(
        lib.matches("availability::spawn_recheck").count(),
        1,
        "lib.rs must wire DB-032's startup check point exactly once"
    );
    assert_eq!(
        lib.matches("availability::on_window_event").count(),
        1,
        "lib.rs must wire DB-032's window check point exactly once"
    );
}

// ---------------------------------------------------------------------------
// `projects_reorder` (T-080, §7.10 Projects module, §7.1 PRJ-019/020,
// DB-005, LOG-001…004, D-051, D-093)
// ---------------------------------------------------------------------------

/// A mock app with `projects_reorder` registered over a fresh database
/// — the `projects_app` shape plus the command under test.
fn reorder_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::projects_reorder])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    // The SYS-005 gate state the healthy app manages at startup
    // (T-105) — the reorder under test must meet the real gate.
    app.manage(crate::sync_running::SyncRunning::new());
    (temp, app)
}

/// Invokes `projects_reorder` with the `orderedIds` wire value over the
/// app's `main` webview.
fn invoke_projects_reorder(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    ordered_ids: &[&str],
) -> Result<(), serde_json::Value> {
    let ids: Vec<String> = ordered_ids.iter().map(|id| (*id).to_owned()).collect();
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request("projects_reorder", serde_json::json!({ "orderedIds": ids })),
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

/// Every project row in stored `sort_order`, as `(id, sort_order,
/// state)` triples — the assertions' window into the table (the
/// `stored_projects` shape, keeping the position and state the
/// reorder's semantics are about).
fn stored_order(database: &crate::db::Db) -> Vec<(String, i64, String)> {
    stored_projects(database)
        .into_iter()
        .map(|project| {
            (
                project.id,
                project.sort_order,
                project.state.as_str().to_owned(),
            )
        })
        .collect()
}

#[test]
fn projects_reorder_persists_the_new_column_order_across_a_database_reopen() {
    // T-080's acceptance criterion: "the order is saved right away"
    // (PRJ-019/020) and survives a restart — proven here at the layer
    // the restart acts on: the managed `Db` is DROPPED (its connection
    // closed), the same data root is opened again through `Db::open`,
    // and the stored `sort_order` still answers the new column order,
    // gapless (DB-005, D-093). The reorder's own journal entry is
    // asserted in the transaction test below.
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
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

    // Act: a full reversal — the drag's result the UI would send.
    invoke_projects_reorder(&webview, &[&beta, &gamma, &alpha])
        .expect("saving the new order must succeed");

    // The reopen: EVERY handle to the first connection closes first —
    // the webview and the test's own `Db` clone both hold the app (and
    // the clone holds the connection, D-037's `Arc<Mutex<…>>`), so
    // `drop(webview)`, `drop(database)` and `drop(app)` must all run
    // before the second open; then the same data root opens again and
    // the fresh connection reads what the command committed (the
    // `app_info` second-open shape).
    drop(webview);
    drop(database);
    drop(app);
    let reopened = crate::db::Db::open(temp.path())
        .expect("reopening the database after the reorder must work");
    assert_eq!(
        stored_order(&reopened)
            .iter()
            .map(|(id, _, state)| (id.as_str(), state.as_str()))
            .collect::<Vec<_>>(),
        [
            (beta.as_str(), "active"),
            (gamma.as_str(), "active"),
            (alpha.as_str(), "active"),
        ],
        "PRJ-019/020: the dragged order is the stored order after a reopen"
    );
    assert_eq!(
        stored_order(&reopened)
            .iter()
            .map(|(_, position, _)| *position)
            .collect::<Vec<_>>(),
        [1, 2, 3],
        "the renumbering stays gapless table-wide (DB-005)"
    );

    drop(reopened);
    drop(temp);
}

#[test]
fn projects_reorder_keeps_the_archived_rows_at_their_own_slots() {
    // The composition rule over the table-wide positions (D-093, §6.4):
    // the stored order [Alpha, Ar1, Beta, Ar2] — an archived row among
    // the actives and one at the tail — reorders its ACTIVE slots to
    // the UI's order while every archived row keeps exactly its own
    // `sort_order` (a column drag must not move the rows the matrix
    // does not render; archive/restore keep positions, and so does a
    // reorder of the columns).
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let alpha = seed_project(
        &database,
        repo_dir(temp.path(), "alpha").display().to_string(),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let archived_middle = seed_project(
        &database,
        repo_dir(temp.path(), "archived-middle")
            .display()
            .to_string(),
        "/archived-middle",
        "Archived Middle",
        "ARM",
        "blue",
    );
    let beta = seed_project(
        &database,
        repo_dir(temp.path(), "beta").display().to_string(),
        "/beta",
        "Beta",
        "BET",
        "green",
    );
    let archived_tail = seed_project(
        &database,
        repo_dir(temp.path(), "archived-tail").display().to_string(),
        "/archived-tail",
        "Archived Tail",
        "ART",
        "purple",
    );
    archive_project(&database, archived_middle.clone());
    archive_project(&database, archived_tail.clone());

    // Act: the UI drags Beta's column before Alpha's.
    invoke_projects_reorder(&webview, &[&beta, &alpha])
        .expect("reordering around the archived rows must succeed");

    // Assert: the actives swapped into the active slots 1 and 3; the
    // archived rows still own the slots they had (2 and 4) — the
    // full-table renumbering wrote around them.
    assert_eq!(
        stored_order(&database)
            .iter()
            .map(|(id, position, state)| (id.as_str(), *position, state.as_str()))
            .collect::<Vec<_>>(),
        [
            (beta.as_str(), 1, "active"),
            (archived_middle.as_str(), 2, "archived"),
            (alpha.as_str(), 3, "active"),
            (archived_tail.as_str(), 4, "archived"),
        ]
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn projects_reorder_renumbers_and_journals_in_one_transaction() {
    // LOG-001…004 ("changing projects", D-051): exactly one
    // `project.changed` entry inside the operation's own transaction —
    // info/user/ok, the count of moved positions in the Bulk detail,
    // and no entity or project id (the reorder is one fact about many
    // projects at once; the Activity window's project filter must not
    // mis-attribute it).
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let a = seed_project(
        &database,
        repo_dir(temp.path(), "a").display().to_string(),
        "/a",
        "A",
        "A",
        "red",
    );
    let b = seed_project(
        &database,
        repo_dir(temp.path(), "b").display().to_string(),
        "/b",
        "B",
        "B",
        "blue",
    );

    invoke_projects_reorder(&webview, &[&b, &a]).expect("the reorder must succeed");

    let journal = journal_entries(&database);
    assert_eq!(journal.len(), 1, "one reorder, one entry");
    assert_eq!(journal[0].action.as_str(), "project.changed");
    assert_eq!(journal[0].level.as_str(), "info");
    assert_eq!(journal[0].visibility.as_str(), "user");
    assert_eq!(journal[0].result.as_str(), "ok");
    assert_eq!(journal[0].entity, None, "no single project owns the change");
    assert_eq!(journal[0].project_id, None);
    assert_eq!(
        journal[0].details,
        Some(serde_json::json!({ "changed": 2 })),
        "the Bulk detail carries how many positions moved"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_failed_projects_reorder_rolls_back_the_renumbering_with_its_journal_entry() {
    // D-051's in-transaction semantics, proven through the real
    // command (the settings/exclusion rollback shape): the journal
    // entry shares the operation's transaction, so a failure AFTER the
    // renumbering succeeded rolls both back — the stored order stands
    // and the journal claims nothing. The failure is staged
    // deterministically inside the database: a trigger that rejects
    // every journal insert (RAISE(ABORT) → SQLITE_CONSTRAINT → the
    // shared base classifier's E_VALIDATION with the SQLite facts in
    // details).
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let a = seed_project(
        &database,
        repo_dir(temp.path(), "a").display().to_string(),
        "/a",
        "A",
        "A",
        "red",
    );
    let b = seed_project(
        &database,
        repo_dir(temp.path(), "b").display().to_string(),
        "/b",
        "B",
        "B",
        "blue",
    );
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER reject_every_journal_entry BEFORE INSERT ON action_log
             BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the trigger must work");

    let error = invoke_projects_reorder(&webview, &[&b, &a])
        .expect_err("the staged abort must fail the reorder");
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

    // Assert: the renumbering rolled back with the journal insert —
    // the stored order and the journal are exactly what they were.
    assert_eq!(
        stored_order(&database)
            .iter()
            .map(|(id, _, _)| id.as_str())
            .collect::<Vec<_>>(),
        [a.as_str(), b.as_str()],
        "the table kept its order"
    );
    assert_eq!(
        journal_entries(&database).len(),
        0,
        "no entry claimed a reorder"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn resending_the_current_order_is_a_verified_noop_that_writes_nothing() {
    // The replayed-drag rule (make-operations-idempotent): a list that
    // composes to the stored order is a no-op — nothing renumbered,
    // nothing journaled. Re-sending the same NEW order after a real
    // reorder is the same rule one step later: the second call changes
    // zero positions and adds no journal entry.
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let a = seed_project(
        &database,
        repo_dir(temp.path(), "a").display().to_string(),
        "/a",
        "A",
        "A",
        "red",
    );
    let b = seed_project(
        &database,
        repo_dir(temp.path(), "b").display().to_string(),
        "/b",
        "B",
        "B",
        "blue",
    );

    // The current order, re-sent: a no-op with zero journal entries.
    invoke_projects_reorder(&webview, &[&a, &b])
        .expect("re-sending the current order must succeed");
    assert_eq!(journal_entries(&database).len(), 0, "nothing changed");

    // A real reorder, then its replay: one journal entry in total.
    invoke_projects_reorder(&webview, &[&b, &a]).expect("the reorder must succeed");
    invoke_projects_reorder(&webview, &[&b, &a]).expect("replaying the same order must succeed");
    assert_eq!(
        journal_entries(&database).len(),
        1,
        "the replay wrote nothing new"
    );
    assert_eq!(
        stored_order(&database)
            .iter()
            .map(|(id, _, _)| id.as_str())
            .collect::<Vec<_>>(),
        [b.as_str(), a.as_str()]
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn every_reorder_refusal_form_leaves_the_table_and_the_journal_untouched() {
    // The three D-093 mismatch forms at the command's active-set
    // boundary — an unknown id, an archived id (the stale-screen
    // shape), a missing active (the empty list against a non-empty
    // matrix), a repeated id — each answered with the typed
    // `E_VALIDATION` envelope naming all three counts, with the whole
    // operation rejected before the first write: the stored order and
    // the journal are provably unchanged.
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
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
    let archived = seed_project(
        &database,
        repo_dir(temp.path(), "archived").display().to_string(),
        "/archived",
        "Archived",
        "ARC",
        "blue",
    );
    archive_project(&database, archived.clone());
    let ghost = "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f";

    // An unknown id: reported as not-an-active-project, with the
    // actives it displaced counted as missing.
    let error = invoke_projects_reorder(&webview, &[&alpha, ghost])
        .expect_err("an unknown id must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["message_key"], serde_json::json!("errors.validation"));
    assert_eq!(
        error["params"],
        serde_json::json!({
            "field": "ordered_ids",
            "value": ghost,
            "reason": "the list must name every active project exactly once: \
                       1 not an active project, 1 missing, 0 repeated",
        })
    );

    // An archived id: the same not-an-active-project bucket (a stale
    // screen's column list).
    let error = invoke_projects_reorder(&webview, &[&alpha, &archived])
        .expect_err("an archived id must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(archived),
        "the offending caller id is echoed"
    );
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!(
            "the list must name every active project exactly once: \
             1 not an active project, 1 missing, 0 repeated"
        )
    );

    // The pure unknown form (the round-1 review gap): every active
    // named exactly once PLUS one archived id — the exact stale-screen
    // shape, with nothing missing. The composition must not run.
    let error = invoke_projects_reorder(&webview, &[&alpha, &beta, &archived])
        .expect_err("an archived id alongside a complete active set must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!(
            "the list must name every active project exactly once: \
             1 not an active project, 0 missing, 0 repeated"
        ),
        "the complete active set makes the refusal pure-unknown: {error}"
    );

    // The empty list against the non-empty active matrix: the pure
    // missing form.
    let error = invoke_projects_reorder(&webview, &[]).expect_err("an empty list must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!(
            "the list must name every active project exactly once: \
             0 not an active project, 2 missing, 0 repeated"
        )
    );

    // A repeated id: counted on its own.
    let error = invoke_projects_reorder(&webview, &[&alpha, &beta, &beta])
        .expect_err("a repeated id must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!(
            "the list must name every active project exactly once: \
             0 not an active project, 0 missing, 1 repeated"
        )
    );

    // Every refusal left the table and the journal exactly as they
    // were: the creation order Alpha(1), Beta(2), Archived(3), with
    // the archived row's position untouched (§6.4).
    assert_eq!(
        stored_order(&database)
            .iter()
            .map(|(id, position, state)| (id.as_str(), *position, state.as_str()))
            .collect::<Vec<_>>(),
        [
            (alpha.as_str(), 1, "active"),
            (beta.as_str(), 2, "active"),
            (archived.as_str(), 3, "archived"),
        ]
    );
    assert_eq!(journal_entries(&database).len(), 0, "nothing was journaled");

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn an_empty_matrix_and_a_single_column_are_noops() {
    // The boundaries: a fresh database's empty active set accepts its
    // empty order as a no-op (the drag surface has no columns), and a
    // single active project has no other position to take — both
    // succeed with nothing written and nothing journaled.
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);

    invoke_projects_reorder(&webview, &[]).expect("the empty matrix accepts its empty order");
    assert_eq!(journal_entries(&database).len(), 0);

    let only = seed_project(
        &database,
        repo_dir(temp.path(), "only").display().to_string(),
        "/only",
        "Only",
        "ONLY",
        "red",
    );
    invoke_projects_reorder(&webview, &[&only]).expect("the single column is its own order");
    assert_eq!(
        stored_order(&database)
            .iter()
            .map(|(id, position, _)| (id.as_str(), *position))
            .collect::<Vec<_>>(),
        [(only.as_str(), 1)]
    );
    assert_eq!(journal_entries(&database).len(), 0);

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn a_corrupt_projects_cell_fails_the_reorder_with_the_fallback_envelope() {
    // The corrupt-cells wire contract on the reorder's own read: the
    // composition reads the whole table (`list(All)` — D-093), so a
    // stored vocabulary cell outside its CHECK vocabulary fails the
    // command with the recorded D-037 fallback and the
    // `ProjectsCorrupt` diagnostic — never `E_VALIDATION` (corruption
    // is not the caller's input) and never a raw cell on any SEC-008
    // surface. Written through the `ignore_check_constraints` seam.
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let webview = main_webview(&app);
    let a = seed_project(
        &database,
        repo_dir(temp.path(), "a").display().to_string(),
        "/a",
        "A",
        "A",
        "red",
    );
    let b = seed_project(
        &database,
        repo_dir(temp.path(), "b").display().to_string(),
        "/b",
        "B",
        "B",
        "blue",
    );
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "PRAGMA ignore_check_constraints = 1;\n\
             UPDATE projects SET state = 'bogus';\n\
             PRAGMA ignore_check_constraints = 0;",
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("tampering the state cell through the executor must work");

    let error = invoke_projects_reorder(&webview, &[&b, &a])
        .expect_err("a corrupt cell must fail the reorder");

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
        "the diagnostic names the column and class only"
    );
    // The renumbering never ran: the raw table keeps its creation
    // order (read as raw cells — the repository read now fails on the
    // tampered state).
    let positions: Vec<(String, i64)> = tauri::async_runtime::block_on(database.run(|tx| {
        let mut statement = tx
            .prepare("SELECT id, sort_order FROM projects ORDER BY sort_order")
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(crate::db::sqlite_error_to_app_error)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("reading the raw positions must work");
    assert_eq!(
        positions
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        [a.as_str(), b.as_str()],
        "the table was not renumbered"
    );

    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn projects_reorder_answers_honestly_without_a_database() {
    // The `projects_list` rule: recovery mode never pretends a write
    // succeeded — the honest `NoDatabase` fallback, nothing written
    // anywhere (there is no database to write to).
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::projects_reorder])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    // The SYS-005 gate state recovery mode also manages (T-105): the
    // honest `NoDatabase` answer below must come from `managed_db`,
    // not from the gate's missing-state fallback.
    app.manage(crate::sync_running::SyncRunning::new());
    let error = invoke_projects_reorder(
        &main_webview(&app),
        &["0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f"],
    )
    .expect_err("the no-database state must fail honestly");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["message_key"], serde_json::json!("errors.db_locked"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn the_projects_reorder_command_is_registered_with_the_specta_builder() {
    // The §7.10 Projects-module name (D-009's fixed registry),
    // registered exactly once (the D-034 wiring-pin rationale) — the
    // bindings carry `projectsReorder` to the frontend through it.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::projects_reorder").count(),
        1,
        "lib.rs must register the `projects_reorder` command exactly once (§7.10, D-009)"
    );
}

// ---------------------------------------------------------------------------
// The `sync_running` gate (T-105, §5.4 SYS-004/SYS-005, §7.4 MAIN-030,
// §9.4 phase 0/phase 4, §10 `E_SYNC_RUNNING`)
// ---------------------------------------------------------------------------

/// The wire form of the `E_SYNC_RUNNING` refusal the three
/// per-command tests compare their errors against — the §7.10
/// envelope `sync_running_refusal` builds. The chain is anchored, not
/// circular: the criterion test above pins this envelope to a
/// spec-literal JSON, so a command answering anything else (or the
/// constructor drifting) turns one of the two red.
fn sync_running_envelope() -> serde_json::Value {
    serde_json::to_value(crate::commands::sync_running_refusal())
        .expect("serializing the refusal envelope must work")
}

/// Holds "a sync is running" for a test's duration: the guard taken
/// through `begin_sync` — the same start seam the sync pipeline
/// (T-112+) calls — dropped at the end of the scope, §9.4 phase 4
/// step 5's release (the availability suite's externally-held-slot
/// shape: two real syncs racing for the flag cannot be made
/// deterministic).
fn held_sync_flag(app: &tauri::App<tauri::test::MockRuntime>) -> crate::sync_running::SyncGuard {
    crate::commands::begin_sync(app.app_handle()).expect("the test's flag must be free to take")
}

#[test]
fn the_second_sync_start_is_refused_with_e_sync_running() {
    // The card's criterion through the production start seam: the
    // first `begin_sync` takes the flag (a guard comes back), and
    // the SECOND — the parallel start SYS-004 forbids — is refused
    // with exactly the §7.10 `E_SYNC_RUNNING` envelope the gated
    // commands answer with (asserted against a spec-literal JSON,
    // not the constructor's own output, so the mapping cannot drift
    // from the wire form). No command is invoked here (the
    // `availability_app` shape — the seam test needs only the
    // managed state).
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::sync_running::SyncRunning::new());

    let first =
        crate::commands::begin_sync(app.app_handle()).expect("the first start must take the flag");
    let error = crate::commands::begin_sync(app.app_handle())
        .expect_err("a second start while one runs must be refused");
    assert_eq!(
        serde_json::to_value(&error).expect("serializing the refusal must work"),
        serde_json::json!({
            "code": "E_SYNC_RUNNING",
            "message_key": "errors.sync_running",
            "params": {},
            "details": null,
        }),
        "the refusal's envelope is the §10/§7.10 wire form"
    );
    drop(first);
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "the guard's drop resets the flag"
    );
}

#[test]
fn the_gate_answers_the_honest_fallback_without_the_flag_state() {
    // The uninitialized-app bug the gate answers honestly (the
    // `the_missing_session_state_answers_the_honest_fallback`
    // shape): a healthy database but no `SyncRunning` state —
    // `setup` manages the flag before anything that could fail, so
    // this state cannot occur in a real launch, and the mutation
    // commands answer the recorded fallback with a diagnostic naming
    // it (never a panic, never a silently-permissive gate). The
    // database being present pins the ordering too: the `E_DB_LOCKED`
    // here can only come from the gate's missing state, since
    // `managed_db` would have answered `Ok`.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::commands::exclusions_list,
            crate::commands::exclusion_remove
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    // Deliberately NOT managing `SyncRunning` — the state under test.
    let webview = main_webview(&app);

    let error = invoke_exclusion_remove(&webview, "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f")
        .expect_err("the gate must fail honestly without its state");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("SyncRunningMissing"),
        "the diagnostic names the state, got {error}"
    );
    // The read side never touches the gate: `exclusions_list` answers
    // the healthy empty list over the healthy database.
    assert_eq!(
        invoke_exclusions_list(&webview)
            .expect("the read needs no gate state")
            .len(),
        0
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn the_sync_running_flag_is_managed_at_startup() {
    // The setup wiring pin (the `config_tests` marker shape, the
    // registration pins' rationale): `lib.rs` must manage the flag
    // exactly once — without this, dropping the `manage` line would
    // leave every suite green while production turns all three
    // mutation commands into the missing-state fallback.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("sync_running::SyncRunning::new").count(),
        1,
        "lib.rs must manage the sync-running flag exactly once (SYS-004, T-105)"
    );
}

#[test]
fn projects_list_keeps_working_during_a_sync() {
    // SYS-005's read half (the `exclusions_list` pin's twin): the
    // columns the matrix renders stay readable while the flag is up
    // — a UI frozen mid-sync with no data to show would serve
    // nothing, and no future gate may appear on this read path
    // without this test going red.
    let (temp, app) = projects_app();
    app.manage(crate::sync_running::SyncRunning::new());
    let database = app.state::<crate::db::Db>().inner().clone();
    seed_project(
        &database,
        repo_dir(temp.path(), "alpha").display().to_string(),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let webview = main_webview(&app);

    let _guard = held_sync_flag(&app);
    let columns = invoke_projects_list(&webview, serde_json::json!(null))
        .expect("the read must keep working during a sync");
    assert_eq!(columns.len(), 1, "the column is served from the live read");
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn exclusion_remove_is_refused_during_a_sync_and_recovers_after_it() {
    // SYS-005 end to end over the real IPC path: while the flag is
    // up, the removal answers `E_SYNC_RUNNING` and writes nothing
    // (the row survives), the READ on the same screen keeps working
    // (reads are never gated), and the moment the run ends the same
    // call converges — the retry the refusal's "wait until it
    // finishes" action promises.
    let (temp, app) = exclusions_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let id = seed_exclusion(
        &database,
        temp.path().join("excluded-repo").display().to_string(),
        "/excluded-repo".to_owned(),
        eunomia_store::projects::ExclusionReason::Manual,
    );
    let webview = main_webview(&app);

    let guard = held_sync_flag(&app);
    let error = invoke_exclusion_remove(&webview, &id)
        .expect_err("the removal must be refused during a sync");
    assert_eq!(error, sync_running_envelope(), "the §10 refusal envelope");
    assert_eq!(
        invoke_exclusions_list(&webview)
            .expect("the read must keep working during a sync")
            .len(),
        1,
        "reads are never gated (SYS-005) and the row was not deleted"
    );

    drop(guard);
    invoke_exclusion_remove(&webview, &id).expect("the removal must work once the sync has ended");
    assert_eq!(
        invoke_exclusions_list(&webview)
            .expect("the read must keep working after the sync")
            .len(),
        0,
        "the retried removal converged on the empty table"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn projects_reorder_is_refused_during_a_sync_and_recovers_after_it() {
    // SYS-005 for the column drag: the refusal carries the envelope,
    // the table is provably untouched (D-093's window), and the
    // re-sent order after the run ends still lands.
    let (temp, app) = reorder_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let first = seed_project(
        &database,
        repo_dir(temp.path(), "alpha").display().to_string(),
        "/alpha",
        "Alpha",
        "ALP",
        "red",
    );
    let second = seed_project(
        &database,
        repo_dir(temp.path(), "beta").display().to_string(),
        "/beta",
        "Beta",
        "BET",
        "green",
    );
    let webview = main_webview(&app);

    let guard = held_sync_flag(&app);
    let error = invoke_projects_reorder(&webview, &[second.as_str(), first.as_str()])
        .expect_err("the reorder must be refused during a sync");
    assert_eq!(error, sync_running_envelope(), "the §10 refusal envelope");
    assert_eq!(
        stored_order(&database),
        [
            (first.clone(), 1, "active".to_owned()),
            (second.clone(), 2, "active".to_owned())
        ],
        "the table was not renumbered while the flag was up"
    );

    drop(guard);
    invoke_projects_reorder(&webview, &[second.as_str(), first.as_str()])
        .expect("the reorder must work once the sync has ended");
    assert_eq!(
        stored_order(&database),
        [
            (second.clone(), 1, "active".to_owned()),
            (first.clone(), 2, "active".to_owned())
        ],
        "the re-sent drag landed after the run ended"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn project_add_manual_is_refused_during_a_sync_before_the_sec003_gate() {
    // The add flow's gate ordering: the mode gate runs FIRST, so
    // during a sync even a validly-picked repository path is refused
    // with `E_SYNC_RUNNING` (it is the flag, not the input, that
    // answers), and a path from nowhere meets the same mode answer —
    // SEC-003's invariant holds either way (a foreign folder still
    // never reaches the `.git` check, the database, or the probes).
    // After the run ends, the same picked path adds normally.
    let (temp, app) = add_manual_app();
    let database = app.state::<crate::db::Db>().inner().clone();
    let repo = repo_dir(temp.path(), "during-sync-repo");
    let picked = pick_folder(&app, &repo);
    let webview = main_webview(&app);

    let guard = held_sync_flag(&app);
    let error = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect_err("the add must be refused during a sync");
    assert_eq!(error, sync_running_envelope(), "the §10 refusal envelope");
    let error =
        invoke_project_add_manual(&webview, "/nowhere/never-picked", serde_json::json!(null))
            .expect_err("a foreign path must also meet the mode answer first");
    assert_eq!(
        error,
        sync_running_envelope(),
        "the mode gate precedes SEC-003 — the refusal is input-independent"
    );
    assert!(
        stored_projects(&database).is_empty(),
        "no project row was written while the flag was up"
    );

    drop(guard);
    let added = invoke_project_add_manual(&webview, &picked, serde_json::json!(null))
        .expect("the add must work once the sync has ended");
    assert_eq!(
        added.outcome, "added",
        "the retried add after the run ended succeeded"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// `state://file-changed` (T-102, spec 5.7, SYS-007…009, OUT-014, T-F13)
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the watch service serves (the
/// `dirty_app` shape): a fresh database in a temp root, the
/// [`FileWatchState`] managed, and BOTH events mounted —
/// `state://file-changed` for the emit/listen round trip and
/// `state://project-availability` because the service's listener
/// subscribes to it (the registration `lib.rs`'s setup performs is
/// pinned by the source scan at the bottom of this section).
fn file_watch_app() -> (
    tempfile::TempDir,
    tauri::App<tauri::test::MockRuntime>,
    tauri::async_runtime::Receiver<crate::file_watch::Command>,
) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    let (state, commands) = crate::file_watch::FileWatchState::new();
    app.manage(state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::file_watch::FileChangedEvent,
            crate::availability::ProjectAvailabilityEvent,
        ])
        .mount_events(&app);
    (temp, app, commands)
}

/// Subscribes the typed listener the frontend's generated
/// `events.stateFileChanged` mirrors, returning the receiver every
/// emitted payload lands on. Delivery to Rust listeners is
/// synchronous inside `emit` (the availability suite's rule).
fn listen_file_changed(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> std::sync::mpsc::Receiver<crate::file_watch::FileChangedEvent> {
    let (sender, receiver) = std::sync::mpsc::channel();
    tauri_specta::Event::listen(app, move |event| {
        let _ = sender.send(event.payload);
    });
    receiver
}

/// The test debounce: a real window the backend debounces inside,
/// short enough to keep the suite quick (SYS-007's production 300 ms
/// is pinned by the unit test).
const WATCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(40);

/// Waits until the service ran `at_least` passes, or fails after a
/// generous deadline — the pass counter is the deterministic
/// observable (an empty diff is silent on the wire by contract).
fn wait_for_watch_passes(app: &tauri::App<tauri::test::MockRuntime>, at_least: u64) {
    let state = app.state::<crate::file_watch::FileWatchState>();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let passes = state.pass_count();
        if passes >= at_least {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the watch service ran only {passes} passes, waiting for {at_least}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Seeds one project with the given scheme/base pair through the
/// repository, returning its row id. Owned inputs — the executor's
/// closure is `'static`.
fn seed_watched_project(
    database: &crate::db::Db,
    path: String,
    path_key: &'static str,
    scheme: eunomia_store::settings::Scheme,
) -> String {
    seed_watched_named_project(
        database,
        path,
        path_key,
        scheme,
        eunomia_store::settings::BaseName::AgentsMd,
    )
}

/// The full-control seeder [`seed_watched_project`] delegates to: the
/// base name is a parameter for the tests that need `CLAUDE.md`.
fn seed_watched_named_project(
    database: &crate::db::Db,
    path: String,
    path_key: &'static str,
    scheme: eunomia_store::settings::Scheme,
    base_name: eunomia_store::settings::BaseName,
) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &eunomia_store::projects::NewProject {
                path: &path,
                path_key,
                name: "Demo",
                label: "DEM",
                color: "red",
                description: "",
                added_via: eunomia_store::projects::AddedVia::Manual,
                scheme,
                base_name,
                extra_local: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
                extra_project: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
            },
        )
        .map(|project| project.id)
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work")
}

/// Seeds one active instruction with the given title/content and
/// assigns it to the project — the title is a parameter because the
/// library's titles are unique (DB-010) and a test may seed more
/// than one instruction.
fn seed_assigned_instruction(
    database: &crate::db::Db,
    project_id: String,
    title: &'static str,
    content: &'static str,
) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Instruction { title },
        )
        .map_err(library_watch_seed_error)?;
        let variant = eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content,
                is_main: true,
            },
        )
        .map_err(library_watch_seed_error)?;
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &item.id,
                variant_id: &variant.id,
            },
        )
        .map_err(library_watch_seed_error)
    }))
    .expect("seeding the assignment through the repository must work");
}

/// Maps a library seeding failure onto the wire shape — test-local,
/// the `library_seed_error` shape of the dirty suite.
fn library_watch_seed_error(
    error: eunomia_store::library::LibraryError,
) -> crate::errors::AppError {
    match error {
        eunomia_store::library::LibraryError::Sql(sql) => crate::db::sqlite_error_to_app_error(sql),
        other => {
            let mut app_error = crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
            app_error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("LibrarySeed".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// The DB-024 simulation of the sync pipeline (the `simulate_sync`
/// shape): store the build hash a `written`/`unchanged` status
/// stores. Owned inputs — the executor's closure is `'static`.
fn simulate_sync_hash(database: &crate::db::Db, project_id: String, hash: String) {
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

/// The managed database handle.
fn watch_database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

#[test]
fn an_external_edit_marks_the_project_externally_modified_without_writing_anything() {
    // T-F13's criterion, end to end (OUT-014: "An external edit flips
    // the state without any dialog"): a synced scheme-A project whose
    // target file is edited outside the app — a plain std write, what
    // an editor does — is reported as externally modified through the
    // debounced event, and NOTHING is written back: the file still
    // holds the external bytes after the pass (the display-only pin),
    // and no other surface appears. The story then continues into the
    // honest half of "the next sync overwrites": the simulated sync
    // write (the bytes a real sync would leave) plus the
    // `refresh_after_sync` seam clear the state — T-F13's readiness
    // for T-118, whose pipeline will do exactly this write.
    let (temp, app, commands) = file_watch_app();
    let database = watch_database_of(&app);
    let root = repo_dir(temp.path(), "alpha");
    let target = root.join("AGENTS.md");
    let project_id = seed_watched_project(
        &database,
        root.display().to_string(),
        "/alpha",
        eunomia_store::settings::Scheme::A,
    );
    seed_assigned_instruction(&database, project_id.clone(), "Style", "Be brief.");

    // The sync simulation: the file a sync would leave (the real
    // builder, the real marker) and the hash it would store.
    let marker = eunomia_core::marker::creation_marker_line(
        eunomia_core::marker::MarkerTime {
            year: 2026,
            month: 11,
            day: 21,
            hour: 9,
            minute: 30,
            second: 5,
        },
        None,
        0,
    );
    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    let synced_file = eunomia_core::target_file::build_content(
        eunomia_core::project::Scheme::A,
        "Be brief.",
        &eunomia_core::extra_files::extra_file_lines(
            eunomia_core::project::BaseName::AgentsMd,
            off,
            off,
        ),
        &marker,
    );
    std::fs::write(&target, &synced_file).expect("writing the synced fixture file must work");
    let hash = eunomia_core::hash::build_hash(
        eunomia_core::project::Scheme::A,
        &target,
        off,
        off,
        "Be brief.",
    );
    simulate_sync_hash(&database, project_id.clone(), hash.clone());

    crate::file_watch::spawn_service(app.handle().clone(), commands, WATCH_DEBOUNCE)
        .expect("the watcher must start");
    let events = listen_file_changed(&app);
    wait_for_watch_passes(&app, 1);
    // A clean startup: the file matches the build — no event.
    assert!(events.try_recv().is_err(), "a matching file emits nothing");

    // The external edit, made completely outside the app.
    let external = "My own edit, made in an editor.\n";
    std::fs::write(&target, external).expect("the external edit must work");
    wait_for_watch_passes(&app, 2);
    assert_eq!(
        events
            .try_recv()
            .expect("the external edit must be reported"),
        crate::file_watch::FileChangedEvent {
            changes: vec![crate::file_watch::ProjectFileChange {
                project_id: project_id.clone(),
                external_changed: true,
                file_deleted: false,
            }],
        },
        "the debounce re-read flipped the verdict — no dialog involved"
    );
    // The display-only pin: the service wrote NOTHING back; the file
    // still holds the external bytes.
    assert_eq!(
        std::fs::read_to_string(&target).expect("the file must still be readable"),
        external,
        "a watch event must never write a file (OUT-014)"
    );
    // Idempotence: the debounce settles, no further events.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(events.try_recv().is_err(), "an unchanged verdict is silent");

    // The simulated next sync: the pipeline's own write (fresh build
    // bytes) plus the hash store, then the after-sync seam — the
    // state clears through the same wire. Which pass emits the
    // clearing — the write's own debounced partial re-check or the
    // seam's full one — is a scheduling race (the DB commit between
    // the write and the seam can outlast the 40 ms window on a
    // loaded runner), so this asserts the race-free facts instead
    // (round-1 review's P2-2): the seam never returns a stale
    // verdict, and the clearing lands on the wire exactly once.
    std::fs::write(&target, &synced_file).expect("the sync's rewrite must work");
    simulate_sync_hash(&database, project_id.clone(), hash);
    let cleared =
        tauri::async_runtime::block_on(crate::file_watch::refresh_after_sync(app.handle()))
            .expect("the after-sync pass must succeed");
    let expected_clearing = crate::file_watch::ProjectFileChange {
        project_id: project_id.clone(),
        external_changed: false,
        file_deleted: false,
    };
    assert!(
        cleared
            .iter()
            .all(|change| !change.external_changed && !change.file_deleted),
        "the seam must not return a stale verdict: {cleared:?}"
    );
    assert!(
        cleared.iter().all(|change| *change == expected_clearing),
        "any diff the seam does return is the clearing itself: {cleared:?}"
    );
    // Drain the wire for the clearing (whoever produced it) …
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut clearings: Vec<crate::file_watch::ProjectFileChange> = Vec::new();
    while clearings.is_empty() {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match events.recv_timeout(remaining) {
            Ok(event) => clearings.extend(
                event
                    .changes
                    .into_iter()
                    .filter(|change| !change.external_changed && !change.file_deleted),
            ),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the event listener died mid-test")
            }
        }
    }
    assert_eq!(
        clearings,
        vec![expected_clearing],
        "the clearing arrives exactly once — T-118's half, proven on \
         the seam it will call"
    );
    // … and once the debounce settles, no duplicate clearing follows.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut duplicates = Vec::new();
    while let Ok(event) = events.try_recv() {
        duplicates.extend(
            event
                .changes
                .into_iter()
                .filter(|change| !change.external_changed && !change.file_deleted),
        );
    }
    assert!(
        duplicates.is_empty(),
        "the clearing is emitted exactly once: {duplicates:?}"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}

#[test]
fn deleting_a_synced_target_file_reports_target_deleted_a_never_synced_one_reports_nothing() {
    // SYS-009's признак: the deletion of a SYNCED project's target
    // file is its own flag; a never-synced project without a file is
    // not deleted (OUT-005 creates it — no flag would be honest).
    let (temp, app, commands) = file_watch_app();
    let database = watch_database_of(&app);
    let synced_root = repo_dir(temp.path(), "synced");
    let synced_target = synced_root.join("AGENTS.md");
    let synced_id = seed_watched_project(
        &database,
        synced_root.display().to_string(),
        "/synced",
        eunomia_store::settings::Scheme::A,
    );
    seed_assigned_instruction(&database, synced_id.clone(), "Synced Style", "Be brief.");
    let fresh_root = repo_dir(temp.path(), "fresh");
    let fresh_id = seed_watched_project(
        &database,
        fresh_root.display().to_string(),
        "/fresh",
        eunomia_store::settings::Scheme::A,
    );
    seed_assigned_instruction(&database, fresh_id, "Fresh Style", "Be brief.");

    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    let synced_file = eunomia_core::target_file::build_content(
        eunomia_core::project::Scheme::A,
        "Be brief.",
        &eunomia_core::extra_files::extra_file_lines(
            eunomia_core::project::BaseName::AgentsMd,
            off,
            off,
        ),
        &eunomia_core::marker::creation_marker_line(
            eunomia_core::marker::MarkerTime {
                year: 2026,
                month: 11,
                day: 21,
                hour: 9,
                minute: 30,
                second: 5,
            },
            None,
            0,
        ),
    );
    std::fs::write(&synced_target, &synced_file).expect("the synced fixture must work");
    simulate_sync_hash(
        &database,
        synced_id.clone(),
        eunomia_core::hash::build_hash(
            eunomia_core::project::Scheme::A,
            &synced_target,
            off,
            off,
            "Be brief.",
        ),
    );

    crate::file_watch::spawn_service(app.handle().clone(), commands, WATCH_DEBOUNCE)
        .expect("the watcher must start");
    let events = listen_file_changed(&app);
    wait_for_watch_passes(&app, 1);
    // Startup over a missing-but-never-synced file: silent.
    assert!(
        events.try_recv().is_err(),
        "a never-synced project without a file is not deleted"
    );

    std::fs::remove_file(&synced_target).expect("the deletion must work");
    wait_for_watch_passes(&app, 2);
    assert_eq!(
        events.try_recv().expect("the deletion must be reported"),
        crate::file_watch::FileChangedEvent {
            changes: vec![crate::file_watch::ProjectFileChange {
                project_id: synced_id,
                external_changed: false,
                file_deleted: true,
            }],
        },
        "SYS-009's own flag — A/C recreate the file at sync, B would \
         skip with E_MARKERS_INVALID (the sync pipeline's half)"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_scheme_b_project_reports_edits_and_deletion_through_its_own_semantics() {
    // The B side of the card: a marked file that matches the build is
    // clean; the block's markers going missing (the file a sync will
    // skip with E_MARKERS_INVALID) reports externally modified; the
    // deleted B file reports target deleted — the same event flags,
    // the B-semantics themselves belong to the sync pipeline.
    let (temp, app, commands) = file_watch_app();
    let database = watch_database_of(&app);
    let root = repo_dir(temp.path(), "bravo");
    let target = root.join("AGENTS.md");
    let project_id = seed_watched_project(
        &database,
        root.display().to_string(),
        "/bravo",
        eunomia_store::settings::Scheme::B,
    );
    seed_assigned_instruction(&database, project_id.clone(), "Style", "Be brief.");

    let marker = eunomia_core::marker::creation_marker_line(
        eunomia_core::marker::MarkerTime {
            year: 2026,
            month: 11,
            day: 21,
            hour: 9,
            minute: 30,
            second: 5,
        },
        None,
        0,
    );
    let outside = "My notes.\n";
    let synced_file = eunomia_core::assembly_b::assemble(
        &format!("{outside}<!-- EUNOMIASYNC:BEGIN -->\n<!-- EUNOMIASYNC:END -->\n"),
        "Be brief.",
        &marker,
    )
    .expect("the fixture assembly must work");
    std::fs::write(&target, &synced_file).expect("the synced fixture must work");
    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    simulate_sync_hash(
        &database,
        project_id.clone(),
        eunomia_core::hash::build_hash(
            eunomia_core::project::Scheme::B,
            &target,
            off,
            off,
            "Be brief.",
        ),
    );

    crate::file_watch::spawn_service(app.handle().clone(), commands, WATCH_DEBOUNCE)
        .expect("the watcher must start");
    let events = listen_file_changed(&app);
    wait_for_watch_passes(&app, 1);
    assert!(events.try_recv().is_err(), "the assembled file matches");

    // The markers are removed by hand: never a match again.
    std::fs::write(&target, format!("{outside}Be brief.\n"))
        .expect("the markerless rewrite must work");
    wait_for_watch_passes(&app, 2);
    assert_eq!(
        events
            .try_recv()
            .expect("the markerless rewrite must be reported"),
        crate::file_watch::FileChangedEvent {
            changes: vec![crate::file_watch::ProjectFileChange {
                project_id: project_id.clone(),
                external_changed: true,
                file_deleted: false,
            }],
        }
    );

    // The deletion: the same target-deleted flag as A/C.
    std::fs::remove_file(&target).expect("the deletion must work");
    wait_for_watch_passes(&app, 3);
    assert_eq!(
        events.try_recv().expect("the deletion must be reported"),
        crate::file_watch::FileChangedEvent {
            changes: vec![crate::file_watch::ProjectFileChange {
                project_id,
                external_changed: false,
                file_deleted: true,
            }],
        }
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn an_unavailable_project_is_unwatched_until_its_folder_returns() {
    // SYS-007's removal rule through the availability event: the
    // folder dies → the T-079 recheck emits → the service's listener
    // refreshes → the watch is gone (edits to a recreated file are
    // invisible); the folder returns → the recheck emits again → the
    // watch is back and the (junk) file reports externally modified.
    // The availability subscription is the service's own
    // `listen_availability` — registered exactly as `lib.rs` does.
    let (temp, app, commands) = file_watch_app();
    let database = watch_database_of(&app);
    let root = repo_dir(temp.path(), "delta");
    let target = root.join("AGENTS.md");
    let project_id = seed_watched_project(
        &database,
        root.display().to_string(),
        "/delta",
        eunomia_store::settings::Scheme::A,
    );
    seed_assigned_instruction(&database, project_id.clone(), "Style", "Be brief.");
    let marker = eunomia_core::marker::creation_marker_line(
        eunomia_core::marker::MarkerTime {
            year: 2026,
            month: 11,
            day: 21,
            hour: 9,
            minute: 30,
            second: 5,
        },
        None,
        0,
    );
    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    let synced_file = eunomia_core::target_file::build_content(
        eunomia_core::project::Scheme::A,
        "Be brief.",
        &eunomia_core::extra_files::extra_file_lines(
            eunomia_core::project::BaseName::AgentsMd,
            off,
            off,
        ),
        &marker,
    );
    std::fs::write(&target, &synced_file).expect("the synced fixture must work");
    simulate_sync_hash(
        &database,
        project_id.clone(),
        eunomia_core::hash::build_hash(
            eunomia_core::project::Scheme::A,
            &target,
            off,
            off,
            "Be brief.",
        ),
    );

    app.manage(crate::availability::AvailabilityState::new());
    crate::file_watch::listen_availability(app.handle());
    crate::file_watch::spawn_service(app.handle().clone(), commands, WATCH_DEBOUNCE)
        .expect("the watcher must start");
    let events = listen_file_changed(&app);
    wait_for_watch_passes(&app, 1);
    assert!(events.try_recv().is_err(), "the healthy startup is silent");

    // The folder dies (the .git entry goes — the D-084 probe fails)
    // and the availability recheck runs, as the focus trigger would.
    std::fs::remove_dir_all(root.join(".git")).expect("removing the .git entry must work");
    tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
        .expect("the availability recheck must succeed");
    wait_for_watch_passes(&app, 2); // the listener's refresh ran
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        events.try_recv().is_err(),
        "leaving the watch set emits nothing (the prune is silent)"
    );

    // The watch is gone: a recreated junk file is invisible.
    std::fs::write(&target, "junk while unwatched\n").expect("the junk write must work");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        events.try_recv().is_err(),
        "an unavailable project has no watch — SYS-007's removal rule"
    );

    // The folder returns: the recheck emits, the refresh re-watches,
    // and the junk file now reports externally modified.
    std::fs::create_dir_all(root.join(".git")).expect("restoring the .git entry must work");
    tauri::async_runtime::block_on(crate::availability::recheck_and_emit(app.handle()))
        .expect("the second availability recheck must succeed");
    wait_for_watch_passes(&app, 3);
    assert_eq!(
        events
            .try_recv()
            .expect("the returned folder's junk file must be reported"),
        crate::file_watch::FileChangedEvent {
            changes: vec![crate::file_watch::ProjectFileChange {
                project_id,
                external_changed: true,
                file_deleted: false,
            }],
        },
        "the full pass re-reads every available project — the junk \
         file is caught even though no watch event fired for it"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn a_queued_fire_and_forget_refresh_behind_file_events_still_runs_a_full_pass() {
    // The round-1 review's P1-2 pin: a `Refresh { done: None }` (the
    // availability listener's, the SYS-010 backend error's) queued
    // behind debounced file events must stay a FULL pass — the naive
    // coalescing stored only the ack sender, so a fire-and-forget
    // Refresh collapsed into the partial re-read and the watch set
    // never re-converged. Deterministic shape: a LONG debounce (the
    // fixtures' writes schedule no event inside the test), a burst of
    // filler file events for project alpha, then the fire-and-forget
    // Refresh — the full pass re-reads bravo too, whose edited file
    // no event named, so bravo's marking is the proof the pass was
    // full and not alpha-only.
    let long_debounce = std::time::Duration::from_secs(30);
    let (temp, app, commands) = file_watch_app();
    let database = watch_database_of(&app);
    let alpha_root = repo_dir(temp.path(), "alpha");
    let alpha_target = alpha_root.join("AGENTS.md");
    let alpha_id = seed_watched_project(
        &database,
        alpha_root.display().to_string(),
        "/alpha",
        eunomia_store::settings::Scheme::A,
    );
    let bravo_root = repo_dir(temp.path(), "bravo");
    let bravo_target = bravo_root.join("AGENTS.md");
    let bravo_id = seed_watched_project(
        &database,
        bravo_root.display().to_string(),
        "/bravo",
        eunomia_store::settings::Scheme::A,
    );
    seed_assigned_instruction(&database, alpha_id.clone(), "Alpha Style", "Be brief.");
    seed_assigned_instruction(&database, bravo_id.clone(), "Bravo Style", "Be brief.");

    let marker = eunomia_core::marker::creation_marker_line(
        eunomia_core::marker::MarkerTime {
            year: 2026,
            month: 11,
            day: 21,
            hour: 9,
            minute: 30,
            second: 5,
        },
        None,
        0,
    );
    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    let lines = eunomia_core::extra_files::extra_file_lines(
        eunomia_core::project::BaseName::AgentsMd,
        off,
        off,
    );
    let synced = eunomia_core::target_file::build_content(
        eunomia_core::project::Scheme::A,
        "Be brief.",
        &lines,
        &marker,
    );
    for target in [&alpha_target, &bravo_target] {
        std::fs::write(target, &synced).expect("the synced fixture must work");
    }

    crate::file_watch::spawn_service(app.handle().clone(), commands, long_debounce)
        .expect("the watcher must start");
    let events = listen_file_changed(&app);
    wait_for_watch_passes(&app, 1);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        events.try_recv().is_err(),
        "both projects match their builds: a clean startup is silent"
    );

    // bravo's file changes OUTSIDE any event (the long debounce holds
    // the write's own event back past this test's lifetime).
    std::fs::write(&bravo_target, "edited without an event\n").expect("the bravo edit must work");
    // The burst: filler file events for alpha, then the fire-and-
    // forget Refresh — all queued while the service idles, so the
    // coalescing loop drains them together.
    for _ in 0..8 {
        app.state::<crate::file_watch::FileWatchState>().trigger(
            crate::file_watch::Command::FilesChanged(vec![alpha_target.clone()]),
        );
    }
    app.state::<crate::file_watch::FileWatchState>()
        .trigger(crate::file_watch::Command::Refresh { done: None });
    wait_for_watch_passes(&app, 2);
    // The clearing wire: bravo's marking must arrive (a full pass
    // re-read it); alpha stays clean (nothing named it but fillers).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut markings = Vec::new();
    while markings.is_empty() {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match events.recv_timeout(remaining) {
            Ok(event) => markings.extend(
                event
                    .changes
                    .into_iter()
                    .filter(|change| change.external_changed || change.file_deleted),
            ),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the event listener died mid-test")
            }
        }
    }
    assert_eq!(
        markings,
        vec![crate::file_watch::ProjectFileChange {
            project_id: bravo_id,
            external_changed: true,
            file_deleted: false,
        }],
        "the coalesced Refresh ran a FULL pass — bravo's unannounced \
         edit is re-read, which no alpha-only partial pass would do"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
#[cfg(unix)]
fn a_symlinked_target_is_watched_by_its_real_file() {
    // §9.4 preflight step 2's common case, watched: `CLAUDE.md` points
    // at `AGENTS.md` — the sync writes the REAL file, and the watch
    // must sit there too (the round-1 review's P2-1: a watch pinned
    // to the link spelling would never fire, because editors and
    // §9.5 atomic saves rename over the real name). An external edit
    // of AGENTS.md is what the event carries; the verdict reads
    // through the resolution either way.
    let (temp, app, commands) = file_watch_app();
    let database = watch_database_of(&app);
    let root = repo_dir(temp.path(), "claude");
    let real = root.join("AGENTS.md");
    let link = root.join("CLAUDE.md");
    let project_id = seed_watched_named_project(
        &database,
        root.display().to_string(),
        "/claude",
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::ClaudeMd,
    );
    seed_assigned_instruction(&database, project_id.clone(), "Style", "Be brief.");

    let off = eunomia_core::project::ExtraFile {
        enabled: false,
        position: eunomia_core::project::ExtraFilePosition::End,
    };
    let lines = eunomia_core::extra_files::extra_file_lines(
        eunomia_core::project::BaseName::ClaudeMd,
        off,
        off,
    );
    let synced = eunomia_core::target_file::build_content(
        eunomia_core::project::Scheme::A,
        "Be brief.",
        &lines,
        &eunomia_core::marker::creation_marker_line(
            eunomia_core::marker::MarkerTime {
                year: 2026,
                month: 11,
                day: 21,
                hour: 9,
                minute: 30,
                second: 5,
            },
            None,
            0,
        ),
    );
    std::fs::write(&real, &synced).expect("the real file fixture must work");
    std::os::unix::fs::symlink("AGENTS.md", &link).expect("the CLAUDE.md link must work");
    simulate_sync_hash(
        &database,
        project_id.clone(),
        eunomia_core::hash::build_hash(
            eunomia_core::project::Scheme::A,
            &link,
            off,
            off,
            "Be brief.",
        ),
    );

    crate::file_watch::spawn_service(app.handle().clone(), commands, WATCH_DEBOUNCE)
        .expect("the watcher must start");
    let events = listen_file_changed(&app);
    wait_for_watch_passes(&app, 1);
    assert!(
        events.try_recv().is_err(),
        "the resolved file matches the build through the link"
    );

    // The external edit lands on the REAL file — the name an editor
    // actually saves; the watch resolved there, so it fires.
    let edited = synced.replace("Be brief.", "Be very brief.");
    std::fs::write(&real, &edited).expect("the real-file edit must work");
    wait_for_watch_passes(&app, 2);
    assert_eq!(
        events
            .try_recv()
            .expect("the real-file edit must be reported"),
        crate::file_watch::FileChangedEvent {
            changes: vec![crate::file_watch::ProjectFileChange {
                project_id,
                external_changed: true,
                file_deleted: false,
            }],
        },
        "the watch sits on the resolved name, not the link spelling"
    );
    // Windows teardown rule.
    drop(app);
    drop(temp);
}

#[test]
fn the_after_sync_seam_answers_honestly_without_a_database() {
    // The recovery-mode contract (the availability/dirty precedent):
    // no `Db` managed — the honest `NoDatabase` answer, no event, no
    // panic. The service itself is never spawned in that mode (the
    // `lib.rs` setup returns before the spawn), pinned by the source
    // scan below.
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    let (state, _commands) = crate::file_watch::FileWatchState::new();
    app.manage(state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::file_watch::FileChangedEvent
        ])
        .mount_events(&app);
    let events = listen_file_changed(&app);

    let error = tauri::async_runtime::block_on(crate::file_watch::refresh_after_sync(app.handle()))
        .expect_err("a database-less pass must fail honestly");
    assert_eq!(
        error
            .details
            .as_ref()
            .and_then(|details| details.os_kind.as_deref()),
        Some("NoDatabase"),
        "the honest no-database answer, the managed_db edge every \
         database command shares"
    );
    assert!(events.try_recv().is_err(), "no event without a database");
}

#[test]
fn the_file_changed_event_and_its_wiring_are_registered_with_the_specta_builder() {
    // The D-034/D-112 registration-pin rationale: the registration
    // cannot be exercised over the mock runtime (the builder serves
    // `tauri::Wry`), so the source is scanned — the event in
    // `collect_events!`, and every production wiring call site exactly
    // once, in the order the module docs promise (listener before the
    // startup availability recheck).
    let lib = include_str!("lib.rs");
    assert_eq!(
        lib.matches("file_watch::FileChangedEvent").count(),
        1,
        "lib.rs must register the `state://file-changed` event exactly once (§7.10)"
    );
    assert_eq!(
        lib.matches("file_watch::FileWatchState::new").count(),
        1,
        "lib.rs must manage the session state exactly once"
    );
    assert_eq!(
        lib.matches("file_watch::spawn_service").count(),
        1,
        "lib.rs must spawn the service (§5.7's startup point) exactly once"
    );
    assert_eq!(
        lib.matches("file_watch::listen_availability").count(),
        1,
        "lib.rs must subscribe the availability listener exactly once"
    );
    let listen_at = lib
        .find("file_watch::listen_availability")
        .expect("the listener call site must exist");
    let recheck_at = lib
        .find("availability::spawn_recheck")
        .expect("the startup recheck call site must exist");
    assert!(
        listen_at < recheck_at,
        "the listener is registered before the startup availability \
         recheck is spawned — the session's first verdicts cannot miss it"
    );
}

// ---------------------------------------------------------------------------
// `sync_run` (T-118, §7.10 Sync module, §9.4 the pipeline, D-25,
// SYS-004/SYS-005, MAIN-024/030)
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the whole pipeline serves: the
/// command registered over the mock runtime, a fresh database in a
/// temp root, and every managed state the run's seams read — the flag
/// (SYS-004), `DataRoot` (the §5.5 tree phases 1/2/4 write into), the
/// availability cache (the DB-032 checkpoint inside the run), the
/// dirty cache (the after-commit pass), and the watch state (the
/// after-sync refresh). The watch bus's RECEIVING half is dropped on
/// purpose: with no service spawned the refresh answers its honest
/// `FileWatchGone` refusal immediately (never a hang), which every
/// test here reads as the never-fatal aftermath it is — the one test
/// that wants a converging refresh spawns the real service itself.
fn sync_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::sync_run])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(data_dirs::DataRoot(temp.path().to_path_buf()));
    app.manage(crate::sync_running::SyncRunning::new());
    app.manage(crate::availability::AvailabilityState::new());
    app.manage(crate::dirty_recalc::DirtyState::new().0);
    let (watch_state, _no_service) = crate::file_watch::FileWatchState::new();
    app.manage(watch_state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::availability::ProjectAvailabilityEvent,
            crate::dirty_recalc::DirtyEvent,
            crate::file_watch::FileChangedEvent,
        ])
        .mount_events(&app);
    (temp, app)
}

/// The ALG-032 example marker, verbatim — the old-run fixture shape the
/// `unchanged` comparisons recognize (the timestamp never decides).
const SYNC_MARKER: &str =
    "<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->";

/// The managed database handle.
fn sync_database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    use tauri::Manager;
    app.state::<crate::db::Db>().inner().clone()
}

/// Creates a real repository folder (`repos/repo-<n>` with a `.git`
/// directory) and seeds its project row with the canonical path (the
/// `sync_phase1_tests.rs` shape: the DB-030 posture the target-path
/// resolution contracts on). Returns `(project_id, repo_path)`.
fn sync_repo_project(
    app: &tauri::App<tauri::test::MockRuntime>,
    repos: &std::path::Path,
    n: usize,
    scheme: eunomia_store::settings::Scheme,
    base_name: eunomia_store::settings::BaseName,
) -> (String, std::path::PathBuf) {
    use eunomia_store::projects::{AddedVia, NewProject};
    let repo = repos.join(format!("repo-{n}"));
    std::fs::create_dir(&repo).expect("the repository folder must be creatable");
    std::fs::create_dir(repo.join(".git")).expect("the .git directory must be creatable");
    let canonical =
        eunomia_fs::paths::canonicalize(&repo).expect("canonicalizing the fresh repo must work");
    let path = Box::leak(canonical.to_string_lossy().into_owned().into_boxed_str());
    let id = tauri::async_runtime::block_on(sync_database_of(app).run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path,
                path_key: path,
                name: Box::leak(format!("Project {n}").into_boxed_str()),
                label: Box::leak(format!("P{n}").into_boxed_str()),
                color: "blue",
                description: "",
                added_via: AddedVia::Manual,
                scheme,
                base_name,
                extra_local: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
                extra_project: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
            },
        )
        .map(|project| project.id)
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work");
    (id, canonical)
}

/// Seeds one active instruction with its main variant and assigns it
/// to the project (the `sync_phase1_tests.rs` shape): the project's
/// built managed text is then `content` verbatim. The title carries a
/// process-wide counter — visible titles are unique in the library.
fn sync_seed_instruction(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
    content: &'static str,
) {
    static SEEDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seeded = SEEDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let title: &'static str = Box::leak(format!("Be brief {seeded}").into_boxed_str());
    let project_id = project_id.to_owned();
    tauri::async_runtime::block_on(sync_database_of(app).run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Instruction { title },
        )
        .map_err(sync_library_seed_error)?;
        let variant = eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content,
                is_main: true,
            },
        )
        .map_err(sync_library_seed_error)?;
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &item.id,
                variant_id: &variant.id,
            },
        )
        .map_err(sync_library_seed_error)?;
        Ok(())
    }))
    .expect("seeding the assigned instruction must work");
}

/// Maps a library seeding failure onto the wire shape the executor
/// needs (the D-093 fallback family).
fn sync_library_seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    match error {
        eunomia_store::library::LibraryError::Sql(sql) => crate::db::sqlite_error_to_app_error(sql),
        other => {
            let mut app_error = crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
            app_error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("LibrarySeed".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// Maps a run-repository read failure onto the wire shape the executor
/// needs (the `sync_phase1_tests.rs` assertion-side shape).
fn sync_runs_read_error(error: eunomia_store::sync_runs::SyncRunsError) -> crate::errors::AppError {
    match error {
        eunomia_store::sync_runs::SyncRunsError::Sql(sql) => {
            crate::db::sqlite_error_to_app_error(sql)
        }
        other => {
            let mut app_error = crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
            app_error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("SyncRunsRead".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// The run row through the app's database (the assertion-side read).
fn sync_run_row(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> eunomia_store::sync_runs::SyncRun {
    let database = sync_database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::sync_runs::get(tx, &run_id)
            .map_err(sync_runs_read_error)
            .map(|row| row.expect("the run row must persist"))
    }))
    .expect("reading the run row must work")
}

/// The run's per-project rows, in `sort_order`.
fn sync_run_rows(
    app: &tauri::App<tauri::test::MockRuntime>,
    run_id: &str,
) -> Vec<eunomia_store::sync_runs::SyncRunProject> {
    let database = sync_database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::sync_runs::list_projects(tx, &run_id).map_err(sync_runs_read_error)
    }))
    .expect("reading the run's rows must work")
}

/// Every run row's `(id, status)` pair, seeded-date order.
fn sync_all_runs(app: &tauri::App<tauri::test::MockRuntime>) -> Vec<(String, String)> {
    let database = sync_database_of(app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        let mut statement = tx
            .prepare("SELECT id, status FROM sync_runs ORDER BY started_at, id")
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(crate::db::sqlite_error_to_app_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(rows)
    }))
    .expect("reading the runs must work")
}

/// One project's `last_synced_*` cells — OUT-020's observable.
fn sync_project_stamps(
    app: &tauri::App<tauri::test::MockRuntime>,
    project_id: &str,
) -> (Option<String>, Option<String>) {
    let database = sync_database_of(app);
    let project_id = project_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row(
            "SELECT last_synced_at, last_synced_hash FROM projects WHERE id = ?1",
            rusqlite::params![project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("reading the project's stamps must work")
}

/// The number of journal entries a run wrote (§7.7's per-project plus
/// summary shapes, D-051).
fn sync_journal_count(app: &tauri::App<tauri::test::MockRuntime>, run_id: &str) -> i64 {
    let database = sync_database_of(app);
    let run_id = run_id.to_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row(
            "SELECT COUNT(*) FROM action_log WHERE sync_run_id = ?1",
            rusqlite::params![run_id],
            |row| row.get(0),
        )
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("counting the run's journal entries must work")
}

/// Every file under the data root's `backups/` tree, recursively — the
/// OUT-019 observable (empty when no copy survived).
fn sync_backup_files(temp: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn walk(folder: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(folder).expect("the backups tree must be listable") {
            let path = entry.expect("every entry must be readable").path();
            if path.is_dir() {
                walk(&path, found);
            } else {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&temp.join("backups"), &mut found);
    found.sort();
    found
}

/// Invokes `sync_run` with the given `confirmEmpty` wire value
/// (`null`/`false` omitted the flag, `true` confirmed), deserializing
/// the success payload into the typed DTO.
fn invoke_sync_run(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    confirm_empty: serde_json::Value,
) -> Result<crate::commands::SyncRunOutcome, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        invoke_request(
            "sync_run",
            serde_json::json!({ "confirmEmpty": confirm_empty }),
        ),
    );
    match response {
        Ok(payload) => Ok(payload
            .deserialize::<crate::commands::SyncRunOutcome>()
            .expect("the success payload must deserialize into the DTO")),
        Err(error) => Err(error),
    }
}

/// Drives the pipeline through the body seam with the caller's cancel
/// token and seams (the `dialog_pick_folder_with` split's test half).
fn run_sync_with(
    app: &tauri::App<tauri::test::MockRuntime>,
    confirm_empty: Option<bool>,
    cancel: crate::commands::CancelToken,
    seams: crate::commands::SyncRunSeams,
) -> Result<crate::commands::SyncRunOutcome, crate::errors::AppError> {
    let handle = app.app_handle().clone();
    tauri::async_runtime::block_on(crate::commands::sync_run_with(
        handle,
        confirm_empty,
        cancel,
        seams,
    ))
}

#[test]
fn sync_run_without_confirm_returns_needs_confirmation_and_changes_nothing() {
    // The card's criterion (§9.4 phase 1 step 6, D-25): an empty build
    // over a non-empty file, called without the flag, answers the
    // confirmation LIST and changes nothing — the target files byte
    // for byte, no backup, no stamp, the flag free — and the run row
    // is terminal (D-138's reading of "changes nothing": §9.4's phrase
    // is about the files; a `running` row behind a released flag
    // would lie until the next restart). Two such projects pin the
    // list's `sort_order` and the stop row's no-code literal.
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (first, first_repo) = sync_repo_project(
        &app,
        &repos,
        1,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    let (second, second_repo) = sync_repo_project(
        &app,
        &repos,
        13,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    // NO assignments — the builds are empty — over non-empty files.
    let first_target = first_repo.join("AGENTS.md");
    let second_target = second_repo.join("AGENTS.md");
    std::fs::write(&first_target, b"precious old bytes").expect("the fixture must be writable");
    std::fs::write(&second_target, b"other old bytes").expect("the fixture must be writable");
    let webview = main_webview(&app);

    let outcome = invoke_sync_run(&webview, serde_json::json!(null))
        .expect("the unconfirmed call must answer, not fail");

    // The answer IS the list, in `sort_order`.
    let crate::commands::SyncRunOutcome::NeedsConfirmation { projects } = &outcome else {
        panic!("the unconfirmed call must ask for confirmation, got {outcome:?}");
    };
    assert_eq!(
        projects,
        &[first.clone(), second.clone()],
        "the D-25 list, by id, in sort_order"
    );
    // …and nothing changed: the files, the backups tree, the stamps.
    assert_eq!(
        std::fs::read(&first_target).expect("the target must be readable"),
        b"precious old bytes",
        "not one target byte moved"
    );
    assert_eq!(
        std::fs::read(&second_target).expect("the target must be readable"),
        b"other old bytes"
    );
    assert!(
        sync_backup_files(temp.path()).is_empty(),
        "no backup was made"
    );
    assert_eq!(
        sync_project_stamps(&app, &first),
        (None, None),
        "OUT-020: an unconfirmed stop stamps nothing"
    );
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "the flag is free the moment the command answered (RAII)"
    );
    // The run row is terminal: `aborted`, the D-131 tail shape — its
    // rows keep phase 1's verdicts, here the untouched `not_started`,
    // and the no-code literal sits in `error_code` (the store's
    // writer takes `&str`; "" is the no-code spelling, D-138).
    let runs = sync_all_runs(&app);
    assert_eq!(runs.len(), 1, "exactly one run was recorded");
    assert_eq!(runs[0].1, "aborted", "the asking run is terminal");
    let row = sync_run_row(&app, &runs[0].0);
    assert_eq!(row.error_code.as_deref(), Some(""));
    assert!(
        row.details
            .as_deref()
            .is_some_and(|details| details.contains("awaiting the empty-build confirmation")),
        "the class-only stop line, got: {:?}",
        row.details
    );
    assert!(
        sync_run_rows(&app, &runs[0].0)
            .iter()
            .all(|project| project.status.as_str() == "not_started"),
        "both asking rows are untouched"
    );
    // Windows teardown rule: the app's managed `Db` closes first.
    drop(app);
    drop(temp);
}
#[test]
fn sync_run_with_confirm_empty_empties_the_file_through_the_full_pipeline() {
    // The confirmed re-call (§9.4: "the UI calls the command again
    // with confirm_empty=true") is a FULL NEW RUN: the asking run's
    // row stays aborted in history, the confirmed run records its own,
    // and the destructive case D-25 guards — an emptied file — lands
    // with a backup of the overwritten bytes behind it.
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (project, repo) = sync_repo_project(
        &app,
        &repos,
        2,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    let target = repo.join("AGENTS.md");
    std::fs::write(&target, b"precious old bytes").expect("writing the fixture must work");
    let webview = main_webview(&app);
    let asking = invoke_sync_run(&webview, serde_json::json!(null))
        .expect("the unconfirmed call must answer");
    assert!(matches!(
        asking,
        crate::commands::SyncRunOutcome::NeedsConfirmation { .. }
    ));

    let outcome = invoke_sync_run(&webview, serde_json::json!(true))
        .expect("the confirmed call must run the pipeline");

    let crate::commands::SyncRunOutcome::Completed { summary } = outcome else {
        panic!("the confirmed call must complete, not ask again");
    };
    assert_eq!(summary.verdict, "success");
    assert_eq!(summary.projects_total, 1);
    assert_eq!(summary.projects_written, 1);
    // The emptied file: the marker and its break alone (ALG-030's
    // empty-build form), around the run's real clock.
    let written = std::fs::read_to_string(&target).expect("the emptied file must be readable");
    let marker = written.trim_end_matches('\n');
    assert!(
        marker.starts_with("<!-- Created by EunomiaSync ") && marker.ends_with(" -->"),
        "the file is the marker line alone, got: {written:?}"
    );
    assert_eq!(written, format!("{marker}\n"));
    // The overwritten bytes are restorable: one verified copy (BKP-…
    // through phase 2), and the project is stamped (OUT-020).
    let backups = sync_backup_files(temp.path());
    assert_eq!(backups.len(), 1, "the non-empty target was backed up");
    assert_eq!(
        std::fs::read(&backups[0]).expect("the copy must be readable"),
        b"precious old bytes",
        "the backup holds the overwritten bytes"
    );
    let (stamped_at, stamped_hash) = sync_project_stamps(&app, &project);
    assert!(stamped_at.is_some(), "the emptied project is synced");
    assert!(stamped_hash.is_some());
    // History keeps BOTH runs: the aborted ask and the confirmed run.
    let runs = sync_all_runs(&app);
    assert_eq!(
        runs.iter()
            .map(|(_, status)| status.as_str())
            .collect::<Vec<_>>(),
        ["aborted", "success"],
        "the confirmed re-call is a full new run"
    );
    drop(app);
    drop(temp);
}

#[test]
fn a_second_sync_run_while_one_runs_is_refused_with_e_sync_running() {
    // SYS-004 through the real pipeline: while one run is parked
    // inside its write phase (the injected seam holds it between the
    // atomic write's steps — the flag is up, §9.4 phase 3), a second
    // `sync_run` over the real IPC path meets the §7.10
    // `E_SYNC_RUNNING` envelope and writes nothing; the moment the
    // first run finishes, the flag is free.
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (project, _repo) = sync_repo_project(
        &app,
        &repos,
        4,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    sync_seed_instruction(&app, &project, "Be brief.");
    let webview = main_webview(&app);
    // The parked write seam: signal entry, hold until released, then
    // perform the real §9.5 write. The release receiver rides a Mutex
    // because the seam must be `Sync` (phase 3 shares it over an Arc).
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<std::path::PathBuf>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let seams = crate::commands::SyncRunSeams {
        backup: crate::commands::SyncRunSeams::production().backup,
        write: std::sync::Arc::new(move |target: &std::path::Path, contents: &[u8]| {
            entered_tx
                .send(target.to_path_buf())
                .expect("the test must live to receive the entry signal");
            release_rx
                .lock()
                .expect("the release lock must not be poisoned")
                .recv()
                .expect("the test must live to release the parked write");
            crate::sync_phase3::production_write(target, contents)
        }),
    };
    let handle = app.app_handle().clone();
    let running = tauri::async_runtime::spawn(async move {
        crate::commands::sync_run_with(handle, None, crate::commands::CancelToken::new(), seams)
            .await
    });
    let _parked = entered_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the pipeline must reach its write phase");

    let error = invoke_sync_run(&webview, serde_json::json!(null))
        .expect_err("a second start while one runs must be refused");
    assert_eq!(error, sync_running_envelope(), "the §10 refusal envelope");
    assert_eq!(
        sync_all_runs(&app).len(),
        1,
        "the refused start recorded no run of its own"
    );

    release_tx
        .send(())
        .expect("the parked write must be released");
    let outcome = tauri::async_runtime::block_on(running)
        .expect("the parked run must join")
        .expect("the parked run must finish");
    assert!(
        matches!(
            outcome,
            crate::commands::SyncRunOutcome::Completed { ref summary } if summary.verdict == "success"
        ),
        "the first run finished cleanly, got {outcome:?}"
    );
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "the flag freed with the run's end"
    );
    drop(app);
    drop(temp);
}

#[test]
fn a_cancelled_token_stops_the_run_before_the_backups() {
    // The cancel seam's one check point (§9.4: cancel is possible
    // before the backups, impossible after phase 2): a latched token
    // stops the run after preflight — the D-131 tail shape (the row
    // `aborted`, phase 1's verdicts stay: the `unchanged` project
    // keeps its honest row), no file moves, no backup, no stamp, and
    // the answer is the aborted summary, not an error (the user asked
    // for this).
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (written, written_repo) = sync_repo_project(
        &app,
        &repos,
        5,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    sync_seed_instruction(&app, &written, "Be brief.");
    let target = written_repo.join("AGENTS.md");
    std::fs::write(&target, format!("Be brief.\n\n{SYNC_MARKER}\n"))
        .expect("writing the matching fixture must work");
    let cancel = crate::commands::CancelToken::new();
    cancel.cancel();

    let outcome = run_sync_with(
        &app,
        None,
        cancel,
        crate::commands::SyncRunSeams::production(),
    )
    .expect("a cancelled run is an outcome, not an error");

    let crate::commands::SyncRunOutcome::Completed { summary } = outcome else {
        panic!("the cancelled run answers its summary");
    };
    assert_eq!(summary.verdict, "aborted");
    assert_eq!(
        std::fs::read(&target).expect("the target must be readable"),
        format!("Be brief.\n\n{SYNC_MARKER}\n").into_bytes(),
        "not one target byte moved"
    );
    assert!(sync_backup_files(temp.path()).is_empty(), "no backup");
    assert_eq!(
        sync_project_stamps(&app, &written),
        (None, None),
        "OUT-020: a cancelled run stamps nothing"
    );
    // The D-131 tail shape: phase 1's `unchanged` verdict survives the
    // stop (it claims no write), the ready row stays `not_started`.
    let rows = sync_run_rows(&app, &summary.run_id);
    assert_eq!(
        rows.iter()
            .map(|row| row.status.as_str())
            .collect::<Vec<_>>(),
        ["unchanged"],
        "the verdicts phase 1 wrote stay"
    );
    let row = sync_run_row(&app, &summary.run_id);
    assert_eq!(row.error_code.as_deref(), Some(""));
    assert_eq!(
        row.details.as_deref(),
        Some("cancelled by the user before the backups"),
        "the stop row's no-code literal and class-only line"
    );
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "the flag freed with the stop"
    );
    drop(app);
    drop(temp);
}

#[test]
fn sync_run_completes_over_the_real_phases_and_serves_the_summary() {
    // The full happy path through the REAL phases (MAIN-024's summary
    // chips): a scheme-A project whose differing file is backed up and
    // rewritten around the run's one marker, and a scheme-C creation
    // case (OUT-005: no target yet, nothing to back up) — the verdict
    // `success`, the counters, the per-project rows in `sort_order`,
    // the §7.7 journal, the stamps — and the flag free afterwards.
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (alpha, alpha_repo) = sync_repo_project(
        &app,
        &repos,
        6,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    sync_seed_instruction(&app, &alpha, "Be brief.");
    let alpha_target = alpha_repo.join("AGENTS.md");
    std::fs::write(&alpha_target, b"stale bytes").expect("the fixture must be writable");
    let (gamma, gamma_repo) = sync_repo_project(
        &app,
        &repos,
        7,
        eunomia_store::settings::Scheme::C,
        eunomia_store::settings::BaseName::ClaudeMd,
    );
    sync_seed_instruction(&app, &gamma, "Run the tests.");
    let gamma_target = gamma_repo.join("CLAUDE.project.md");
    let webview = main_webview(&app);

    let outcome =
        invoke_sync_run(&webview, serde_json::json!(null)).expect("the run must complete");

    let crate::commands::SyncRunOutcome::Completed { summary } = outcome else {
        panic!("a healthy run completes");
    };
    assert_eq!(summary.verdict, "success");
    assert_eq!(summary.projects_total, 2);
    assert_eq!(summary.projects_written, 2);
    assert_eq!(summary.projects_skipped, 0);
    assert_eq!(summary.projects_failed, 0);
    assert!(summary.finished_at.is_some(), "phase 4 stamped the end");
    // The per-project rows, in `sort_order`, with their paths.
    assert_eq!(
        summary
            .projects
            .iter()
            .map(|entry| (entry.project_id.as_str(), entry.status.as_str()))
            .collect::<Vec<_>>(),
        [(alpha.as_str(), "written"), (gamma.as_str(), "written")],
    );
    assert_eq!(
        summary.projects[0].target_path.as_deref(),
        Some(
            eunomia_fs::paths::canonicalize(&alpha_target)
                .expect("the path is UTF-8 canonical")
                .to_str()
                .expect("the canonical path is UTF-8")
        ),
    );
    // The byte-exact files: ALG-030 around the run's one marker.
    let written = std::fs::read_to_string(&alpha_target).expect("the file must be readable");
    let marker = written
        .trim_end_matches('\n')
        .rsplit_once("\n\n")
        .unwrap()
        .1;
    assert!(
        marker.starts_with("<!-- Created by EunomiaSync ") && marker.ends_with(" -->"),
        "the marker's ALG-032 shape, got: {marker:?}"
    );
    assert_eq!(written, format!("Be brief.\n\n{marker}\n"));
    let created = std::fs::read_to_string(&gamma_target)
        .expect("the scheme-C creation case writes the target");
    assert_eq!(created, format!("Run the tests.\n\n{marker}\n"));
    // One backup (the differing file), none for the creation case.
    let backups = sync_backup_files(temp.path());
    assert_eq!(backups.len(), 1, "only the overwritten file was backed up");
    assert_eq!(
        std::fs::read(&backups[0]).expect("the copy must be readable"),
        b"stale bytes"
    );
    // OUT-020 stamps, the §7.7 journal (two project results + the
    // summary), and the free flag.
    assert!(sync_project_stamps(&app, &alpha).0.is_some());
    assert!(sync_project_stamps(&app, &gamma).0.is_some());
    assert_eq!(
        sync_journal_count(&app, &summary.run_id),
        3,
        "D-051's per-project entries plus the run summary"
    );
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "§9.4 phase 4 step 5 freed the flag"
    );
    drop(app);
    drop(temp);
}

#[test]
fn a_failing_backup_aborts_the_run_with_the_typed_envelope() {
    // §9.4 phase 2 step 2 / S10 through the command: a refusing copy
    // aborts the whole run — the typed `E_BACKUP_FAILED` envelope with
    // BKP-005's fields, the run row `aborted` (written by the phase
    // itself; the command's wrapper recognizes the code and does not
    // abort twice), and not one target byte moved.
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (project, repo) = sync_repo_project(
        &app,
        &repos,
        8,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    sync_seed_instruction(&app, &project, "Be brief.");
    let target = repo.join("AGENTS.md");
    std::fs::write(&target, b"old bytes").expect("the fixture must be writable");
    let seams = crate::commands::SyncRunSeams {
        backup: std::sync::Arc::new(|_target: &eunomia_fs::backup::BackupTarget<'_>| {
            Err(eunomia_fs::backup::BackupError::FolderCreateFailed {
                path: std::path::PathBuf::from("/data/backups"),
                source: std::io::Error::from_raw_os_error(13),
            })
        }),
        write: crate::commands::SyncRunSeams::production().write,
    };

    let error = run_sync_with(&app, None, crate::commands::CancelToken::new(), seams)
        .expect_err("the failing copy must abort the run");

    assert_eq!(error.code.as_str(), "E_BACKUP_FAILED");
    assert_eq!(
        error.params.get("project").map(String::as_str),
        Some("Project 8"),
        "BKP-005's project field"
    );
    assert_eq!(
        error.params.get("path").map(String::as_str),
        Some(
            eunomia_fs::paths::canonicalize(&target)
                .expect("the path is UTF-8 canonical")
                .to_str()
                .expect("the canonical path is UTF-8")
        ),
        "BKP-005's path field (the resolved source)"
    );
    let details = error.details.expect("the envelope carries the details");
    assert_eq!(details.os_kind.as_deref(), Some("FolderCreateFailed"));
    assert_eq!(details.os_code, Some(13), "§10's first-class OS code");
    // The verdict: aborted by the phase, untouched by the wrapper.
    let runs = sync_all_runs(&app);
    assert_eq!(runs[0].1, "aborted");
    assert_eq!(
        sync_run_row(&app, &runs[0].0).error_code.as_deref(),
        Some("E_BACKUP_FAILED"),
        "the phase's own abort wrote the row"
    );
    assert_eq!(
        std::fs::read(&target).expect("the target must be readable"),
        b"old bytes",
        "no target file was touched"
    );
    assert!(
        sync_backup_files(temp.path()).is_empty(),
        "the run's copies were discarded"
    );
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "the flag freed with the abort"
    );
    drop(app);
    drop(temp);
}

#[test]
fn a_phase_three_store_failure_aborts_the_run_and_a_retry_converges() {
    // The P3-4 ruling through the command (D-133's recorded follow-up,
    // resolved in D-138): a phase-level `Err` after the run row exists
    // leaves NO `running` corpse — the wrapper writes the terminal
    // `aborted` row with the error's own code — and §10's idempotence
    // promise holds: the retried run converges over what the failed
    // run already wrote. The store failure is forced the honest way:
    // while the write seam parks mid-phase, the ready row is planted
    // into `failed`, so the success verdict's `not_started`-scoped
    // UPDATE matches no row (the store's wrong-state refusal).
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (project, repo) = sync_repo_project(
        &app,
        &repos,
        9,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    sync_seed_instruction(&app, &project, "Be brief.");
    let target = repo.join("AGENTS.md");
    std::fs::write(&target, b"old bytes").expect("the fixture must be writable");
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<std::path::PathBuf>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    // The seam must be `Sync` (phase 3 shares it over an Arc), so the
    // release receiver rides a Mutex.
    let release_rx = std::sync::Mutex::new(release_rx);
    let seams = crate::commands::SyncRunSeams {
        backup: crate::commands::SyncRunSeams::production().backup,
        write: std::sync::Arc::new(move |target: &std::path::Path, contents: &[u8]| {
            entered_tx
                .send(target.to_path_buf())
                .expect("the test must live to receive the entry signal");
            release_rx
                .lock()
                .expect("the release lock must not be poisoned")
                .recv()
                .expect("the test must live to release the parked write");
            crate::sync_phase3::production_write(target, contents)
        }),
    };
    let handle = app.app_handle().clone();
    let running = tauri::async_runtime::spawn(async move {
        crate::commands::sync_run_with(handle, None, crate::commands::CancelToken::new(), seams)
            .await
    });
    let parked = entered_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the pipeline must reach its write phase");
    let database = sync_database_of(&app);
    let parked_text = parked.to_string_lossy().into_owned();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE sync_run_projects SET status = 'failed', \
             error_code = 'E_FILE_LOCKED', error_message = 'planted' \
             WHERE sync_run_id = (SELECT id FROM sync_runs WHERE status = 'running') \
             AND target_path = ?1",
            rusqlite::params![parked_text],
        )
        .map(|_| ())
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("planting the stale row state must work");
    release_tx
        .send(())
        .expect("the parked write must be released");

    let error = tauri::async_runtime::block_on(running)
        .expect("the parked run must join")
        .expect_err("the planted row must refuse the verdict transaction");
    assert_eq!(
        error.code.as_str(),
        "E_DB_LOCKED",
        "the store's wrong-state refusal through the base classifier"
    );
    // The terminal row: aborted with the error's own literal — the
    // P3-4 answer. The FILE the failed run already wrote stays
    // (§9.5's old-or-new; D-133's recorded residual window).
    let runs = sync_all_runs(&app);
    assert_eq!(runs[0].1, "aborted", "no running corpse stays behind");
    assert_eq!(
        sync_run_row(&app, &runs[0].0).error_code.as_deref(),
        Some("E_DB_LOCKED"),
        "the wrapper aborts with the error's own §10 literal"
    );
    let written = std::fs::read_to_string(&target).expect("the written file stays");
    assert!(
        written.starts_with("Be brief.\n\n<!-- Created by EunomiaSync "),
        "the file is wholly the new content, got: {written:?}"
    );
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "the flag freed with the refusal"
    );

    // §10's idempotence: a retried sync over what the failed run
    // wrote converges — the file matches the build, so `unchanged`.
    let webview = main_webview(&app);
    let outcome = invoke_sync_run(&webview, serde_json::json!(null)).expect("the retry must run");
    let crate::commands::SyncRunOutcome::Completed { summary } = outcome else {
        panic!("the retried run completes");
    };
    assert_eq!(summary.verdict, "success", "the retry converges");
    assert_eq!(summary.projects_unchanged, 1);
    assert!(
        sync_project_stamps(&app, &project).0.is_some(),
        "the converged run stamps the project"
    );
    drop(app);
    drop(temp);
}

#[test]
fn a_classifier_shaped_disk_full_error_still_aborts_the_running_row() {
    // The round-1 P1 fix's pin: `pipeline_refusal` decides by the
    // ROW's state, never by the error's code. The shared SQL
    // classifier maps `SQLITE_FULL` to `E_DISK_FULL` from ANY phase's
    // transaction (db.rs `sqlite_error_to_app_error`) — an error shape
    // that looks exactly like phase 1's in-phase free-space tail but
    // leaves the row `running` — so the code-skip the first cut used
    // would leave a corpse. Here the classifier-shaped error meets a
    // `running` row: the wrapper aborts it with the error's own
    // literal; the same error over the terminal row changes nothing
    // (the in-phase aborts arrive exactly that way).
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (_project, _repo) = sync_repo_project(
        &app,
        &repos,
        12,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    let phase0 = tauri::async_runtime::block_on(crate::sync_phase0::run(app.handle()))
        .expect("phase 0 must record the running row");
    let run_id = phase0.run.id.clone();
    // The guard goes here: the wrapper owns no flag, and the flag must
    // not outlive the test's run.
    drop(phase0);
    assert_eq!(sync_run_row(&app, &run_id).status.as_str(), "running");

    // The classifier-shaped tail look-alike over a running row: the
    // abort MUST land (the first cut returned it untouched).
    let answered = tauri::async_runtime::block_on(crate::commands::pipeline_refusal(
        app.handle(),
        &run_id,
        "the write phase",
        crate::errors::AppError::new(crate::errors::ErrorCode::DiskFull),
    ));
    assert_eq!(
        answered.code.as_str(),
        "E_DISK_FULL",
        "the original error returns"
    );
    let row = sync_run_row(&app, &run_id);
    assert_eq!(row.status.as_str(), "aborted", "no running corpse");
    assert_eq!(row.error_code.as_deref(), Some("E_DISK_FULL"));
    assert!(
        row.details
            .as_deref()
            .is_some_and(|details| details.contains("the run stopped at the write phase")),
        "the class-only stage line, got: {:?}",
        row.details
    );

    // The same wrapper over the now-terminal row: a no-op — the row
    // keeps the FIRST verdict (this is how phase 1's in-phase tail and
    // phase 2's backup abort arrive at the wrapper).
    let first = sync_run_row(&app, &run_id);
    let answered = tauri::async_runtime::block_on(crate::commands::pipeline_refusal(
        app.handle(),
        &run_id,
        "the backups",
        crate::errors::AppError::new(crate::errors::ErrorCode::BackupFailed),
    ));
    assert_eq!(answered.code.as_str(), "E_BACKUP_FAILED");
    let after = sync_run_row(&app, &run_id);
    assert_eq!(
        (
            after.status.as_str(),
            after.error_code.as_deref(),
            after.finished_at
        ),
        (
            first.status.as_str(),
            first.error_code.as_deref(),
            first.finished_at
        ),
        "the terminal row is never rewritten"
    );
    drop(app);
    drop(temp);
}

#[test]
fn the_summary_carries_the_skip_and_failure_codes_by_name() {
    // §10's codes reach the summary by name (MAIN-024: "skip and error
    // rows carry their `E_*` code"): a scheme-B target without markers
    // skips as `E_MARKERS_INVALID` (§9.4 phase 1 step 4), and a
    // refused write lands `E_FILE_LOCKED` (the §10 table's locked
    // file row) while the run itself stays `failed` (nothing was
    // written or left unchanged).
    let (temp, app) = sync_app();
    let repos = temp.path().join("repos");
    std::fs::create_dir(&repos).expect("the repos root must be creatable");
    let (beta, beta_repo) = sync_repo_project(
        &app,
        &repos,
        10,
        eunomia_store::settings::Scheme::B,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    sync_seed_instruction(&app, &beta, "Be brief.");
    std::fs::write(beta_repo.join("AGENTS.md"), b"no markers at all\n")
        .expect("the markerless fixture must be writable");
    let (alpha, alpha_repo) = sync_repo_project(
        &app,
        &repos,
        11,
        eunomia_store::settings::Scheme::A,
        eunomia_store::settings::BaseName::AgentsMd,
    );
    sync_seed_instruction(&app, &alpha, "Be brief.");
    let alpha_target = alpha_repo.join("AGENTS.md");
    std::fs::write(&alpha_target, b"old bytes").expect("the fixture must be writable");
    let seams = crate::commands::SyncRunSeams {
        backup: crate::commands::SyncRunSeams::production().backup,
        write: std::sync::Arc::new(|target: &std::path::Path, _contents: &[u8]| {
            Err::<eunomia_fs::atomic::WrittenFile, _>(
                eunomia_fs::atomic::AtomicWriteError::FileLocked(
                    eunomia_fs::atomic::FileLockedFailure {
                        step: eunomia_fs::atomic::WriteStep::Replace,
                        path: target.to_path_buf(),
                        retries: 3,
                        source: std::io::Error::from_raw_os_error(32),
                        cleanup: None,
                    },
                ),
            )
        }),
    };

    let outcome = run_sync_with(&app, None, crate::commands::CancelToken::new(), seams)
        .expect("the run itself completes over its rows");

    let crate::commands::SyncRunOutcome::Completed { summary } = outcome else {
        panic!("a run with per-project refusals still completes");
    };
    assert_eq!(summary.verdict, "failed");
    assert_eq!(summary.projects_skipped, 1);
    assert_eq!(summary.projects_failed, 1);
    assert_eq!(
        summary
            .projects
            .iter()
            .map(|entry| (
                entry.status.as_str(),
                entry.error_code.as_deref(),
                entry.error_message.is_some()
            ))
            .collect::<Vec<_>>(),
        [
            ("skipped", Some("E_MARKERS_INVALID"), true),
            ("failed", Some("E_FILE_LOCKED"), true),
        ],
        "the §10 literals, by name, with their class-only messages"
    );
    assert_eq!(
        std::fs::read(&alpha_target).expect("the target must be readable"),
        b"old bytes",
        "the refused write moved nothing (§9.5's old-or-new)"
    );
    assert_eq!(
        sync_project_stamps(&app, &alpha),
        (None, None),
        "OUT-020: the failed project keeps its previous sync state"
    );
    drop(app);
    drop(temp);
}

#[test]
fn sync_run_answers_honestly_without_a_database() {
    // Recovery mode (the `managed_db` honesty every database command
    // shares): no `Db` managed — the honest `NoDatabase` refusal, and
    // the flag the start took is RAII-released on the way out, so the
    // refusal cannot wedge SYS-004's state.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::commands::sync_run])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(data_dirs::DataRoot(temp.path().to_path_buf()));
    app.manage(crate::sync_running::SyncRunning::new());
    let webview = main_webview(&app);

    let error = invoke_sync_run(&webview, serde_json::json!(null))
        .expect_err("a database-less sync must fail honestly");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the honest recovery-mode answer"
    );
    assert!(
        !app.state::<crate::sync_running::SyncRunning>().is_running(),
        "the refused start left the flag down"
    );
    drop(app);
    drop(temp);
}

#[test]
fn the_sync_run_command_is_registered_with_the_specta_builder() {
    // The §7.10 Sync-module name, registered exactly once — the D-034
    // wiring-pin rationale: without this pin, dropping the
    // registration would leave `export_bindings_are_up_to_date` green
    // while it silently rewrote the committed bindings without the
    // command.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("commands::sync_run").count(),
        1,
        "lib.rs must register the `sync_run` command exactly once (§7.10, T-118)"
    );
}
