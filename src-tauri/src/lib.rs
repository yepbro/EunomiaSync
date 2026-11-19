//! EunomiaSync Tauri application entry point.
//!
//! Commands are registered through `tauri-specta`, which generates the
//! typed TypeScript bindings in `src/bindings.ts` on debug builds. The
//! generated file is committed so the frontend can be type-checked
//! without a Rust toolchain.

use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands};

// The IPC error contract (spec §7.10, §10): AppError and the MVP
// error-code registry, exported to the frontend through the specta
// builder below.
pub(crate) mod errors;

// The machine-level `config.toml` (spec §5.8, SYS-011…013) is needed on
// every platform, exactly like the app itself.
pub(crate) mod config;

// The data directory layout (spec §5.5) is needed on every platform,
// exactly like the app itself.
pub(crate) mod data_dirs;

// The application database state and the one-transaction command
// executor (SYS-001/SYS-002, NFR-008; DECISIONS.md D-037).
pub(crate) mod db;

// The recovery-screen backend (spec §7.10 command notes, §10
// E_DB_CORRUPT row; T-024): the file mechanics behind
// `db_recreate`/`db_restore_backup` and the `db-backups/` listing.
pub(crate) mod recovery;

// The IPC commands (spec §7.10): the recovery-screen group of the
// System module (D-009) and the History module's `activity_list`
// (T-027) — the first registered commands since `greet` was removed
// (D-037); every later command task extends this module.
pub(crate) mod commands;

// File logging with daily rotation (spec §5.3, §7.7; SEC-008) — needed on
// every platform, exactly like the app itself.
pub(crate) mod logging;

// The single-instance handoff handler (SYS-006) is desktop-only, exactly
// like the plugin whose callback calls it below.
#[cfg(desktop)]
pub(crate) mod single_instance;

/// Constructs the tauri-specta builder with all commands and events
/// registered. Shared by the app entry point and the binding export test.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        // The registered §7.10 commands: the recovery-screen group of
        // the System module (D-009), the History module's
        // `activity_list` (T-027) and the Settings module's
        // `settings_get`/`settings_set` (T-030). Every later command
        // task (dialog, library, …) lists its own §7.10 names here.
        // The commands are runtime-generic (the tauri-specta shape for
        // commands that take an `AppHandle`), so the registration pins
        // the concrete runtime of the shipped app.
        .commands(collect_commands![
            commands::activity_list::<tauri::Wry>,
            commands::app_open_data_folder::<tauri::Wry>,
            commands::db_backups_list::<tauri::Wry>,
            commands::db_restore_backup::<tauri::Wry>,
            commands::db_recreate::<tauri::Wry>,
            commands::settings_get::<tauri::Wry>,
            commands::settings_set::<tauri::Wry>,
        ])
        // `AppError` (§7.10: every command returns `Result<T, AppError>`)
        // is registered directly instead of through a command
        // signature: the type must stay reachable even when a command
        // set change would otherwise drop it from the export, and
        // `typ` is tauri-specta's documented mechanism for types not
        // reachable from commands or events; the type (and its
        // `ErrorCode`/`ErrorDetails` dependencies) reaches
        // `src/bindings.ts` through it, pinned by `errors_tests.rs`.
        .typ::<errors::AppError>()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // Export the typed bindings only on debug builds; the generated
    // file is committed and linted as part of the frontend toolchain.
    #[cfg(debug_assertions)]
    builder
        .export(Typescript::default(), "../src/bindings.ts")
        .expect("Failed to export typescript bindings");

    // Tauri plugins (spec §5.3 table, SEC-001): all four are invoked from
    // Rust only — the webview gets neither their JavaScript packages nor
    // any `opener:`/`dialog:`/`updater:` permission, so their commands are
    // unreachable from the UI. `capabilities/default.json` stays
    // plugin-free and only application commands (later
    // `dialog_pick_folder`, `app_open_data_folder`, `update_*`) cross the
    // IPC boundary. `config_tests.rs` pins the exact plugin set and the
    // registration order below.
    let tauri_builder = tauri::Builder::default();

    // `single-instance` (SYS-006: a second launch must not start a second
    // process — the window of the first one is brought to the front) — the
    // plugin REQUIRES being registered before every other plugin, so it is
    // wired first. Desktop-only: the crate is compiled out on mobile, hence
    // the cfg. The callback hands off to the `single_instance::focus_main_window`
    // handler (that module documents the show/unminimize/set_focus order
    // and the missing-window case); the returned outcome is a test seam —
    // there is nothing to branch on here. `_argv`/`_cwd` are deliberately
    // unused: deep links are out of v1 (SEC-004) and no CLI behavior
    // consumes them yet. Diagnostics go through the logging facade
    // (`logging.rs`) into the tracing file log; argv/cwd are never
    // logged — they can carry user paths and the handoff needs none of
    // them. (The plugin registers before the `setup` hook below runs,
    // so a handoff arriving in that startup window is emitted before
    // the subscriber exists and is dropped — no crash, one lost
    // diagnostic line, accepted.)
    #[cfg(desktop)]
    let tauri_builder =
        tauri_builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            single_instance::focus_main_window(app);
        }));

    // `dialog` (SEC-001, SEC-003): the native folder picker runs in Rust
    // behind the `dialog_pick_folder` app command (upcoming task), so the
    // picked path can be tracked per session and every other path stays
    // registered-project-only. The webview has no `dialog:` permission.
    let tauri_builder = tauri_builder.plugin(tauri_plugin_dialog::init());

    // `updater` (D-005/D-023, SEC-001: network belongs to Rust): update
    // check and install run in Rust behind the `update_check` /
    // `update_install` app commands (upcoming tasks); the webview never
    // contacts the release feed. The plugin refuses to initialize without
    // a `plugins.updater` config section (2.14.1 `Config` has a required
    // `pubkey` field), so `tauri.conf.json` carries a disabled stub
    // (`pubkey: ""`, no endpoints) until the release task sets the real
    // key and endpoints; with it, `updater()` fails fast with
    // `EmptyEndpoints` instead of the app failing to start.
    let tauri_builder = tauri_builder.plugin(tauri_plugin_updater::Builder::new().build());

    // `opener` (SEC-001): "open the data folder" runs in Rust behind the
    // `app_open_data_folder` app command (upcoming task); the webview has
    // no `opener:` permission and cannot open arbitrary URLs/paths.
    let tauri_builder = tauri_builder.plugin(tauri_plugin_opener::init());

    tauri_builder
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            // Data directory (spec §5.5): resolve `app_local_data_dir()`
            // and create the layout (`db/`, `db-backups/`, `backups/`,
            // `logs/`) before any of the app's own code reads or writes
            // the directory — `config.toml` (SYS-011, the config task),
            // the database and the logs all live inside. (Tauri builds
            // the config-declared windows before this hook runs and
            // their WebView data sits under this root on Windows and
            // Linux, so the webview layer may create the root first —
            // irrelevant here: creation is idempotent.) Creation
            // converges to the full layout even after a partially
            // deleted directory. A failure aborts the launch: without
            // the layout there is no database, no logs and no config to
            // run with — and SYS-011 orders the layout *before* the
            // config and the config *before* logging, so no diagnostics
            // channel exists at this point; the error propagates out of
            // `setup`, and tauri then panics in the event-loop `Ready`
            // handler with `Failed to setup app: …`, whose message
            // carries the offending path and cause (DECISIONS.md
            // D-027).
            let data_dir = app.path().app_local_data_dir()?;
            data_dirs::ensure_data_layout(&data_dir)?;
            // The resolved root, published for the recovery commands
            // (T-024): they run exactly when the database step below
            // fails, so the one path fact they need must be managed
            // before it can fail — and being a plain value (not a
            // resolver call) is what keeps the IPC tests on temp
            // roots instead of the real user data directory.
            app.manage(data_dirs::DataRoot(data_dir.clone()));

            // Machine-level config (spec §5.8, SYS-011…013): read or
            // create `config.toml` from the data directory — before
            // logging (SYS-011: the log level comes from here) and
            // before the database (both spec §5.8; the failure policy is
            // DECISIONS.md D-028, the seam shape D-029). A missing file
            // is created with defaults; unknown keys are ignored with a
            // warning; unparseable content — invalid UTF-8, broken TOML
            // or an invalid `log_level` — falls back to in-memory
            // defaults with a warning and the file is never rewritten
            // (SYS-012). Only an unreadable/unwritable file aborts the
            // launch, like the layout above (still pre-logging, so
            // still no diagnostics channel to degrade into). The config
            // is published as managed state first; the logging step
            // below reads it back through `app.state::<AppConfig>()`.
            let (config, warnings) = config::load_or_create(&data_dir)?;
            app.manage(config);
            let level = app.state::<config::AppConfig>().log_level;

            // File logging (spec §5.3, §5.5, §7.7): initialized after
            // the config (SYS-011) with the `log_level` it holds, at
            // `<data dir>/logs/` explicitly — never Tauri's
            // `app_log_dir()` (macOS would move the logs out of the
            // data directory; pinned in `config_tests.rs`). `init`
            // never aborts the launch: logging is diagnostics, so a
            // broken log file degrades to a stderr subscriber with one
            // `error` event (module docs in `logging.rs`). The returned
            // `Logger` owns the non-blocking writer's guard; keeping it
            // as managed state holds the writer thread for the whole
            // run (dropping it early would silently stop the file log —
            // tauri exits the process without dropping managed state,
            // so the flush-at-exit path belongs to the worker's
            // per-batch flushing, not to this value's destructor). The
            // config warnings and the startup line are emitted after
            // `init`, so they land in the log file.
            let logs_dir = data_dir.join(data_dirs::LOGS_SUBDIRECTORY);
            let logger = logging::init(&logs_dir, level);
            app.manage(logger);
            config::emit_warnings(&warnings);
            logging::startup(&data_dir, level);

            // The database (SYS-002): one SQLite connection for the
            // whole app, opened from the data directory (the §5.5
            // paths — `db/eunomiasync.sqlite3` and the `db-backups/`
            // the migration run's safety copies go to — are derived
            // inside `db::Db::open`) with the DB-001 pragma set and
            // migrated to the latest schema, then published as managed
            // state. Every database command goes through its
            // `run` executor — one operation, one transaction, off
            // the async runtime (SYS-001, NFR-008; DECISIONS.md
            // D-037). A database newer than this app (DB-004,
            // NFR-010) is rejected before anything touches the file
            // and hands back the typed `E_DB_TOO_NEW` `AppError`
            // (`DbInitError::app_error`), logged here through the
            // facade with its §10 code. A migration step that failed
            // and was rolled back (UPD-004) is another: the error
            // names the pre-migration copy, and the typed
            // `E_UPDATE_FAILED` form is logged through its own facade
            // event. A migration failure *without* a copy (the run
            // created the database) deliberately gets no facade line:
            // not the UPD-004 situation, it keeps the D-027
            // anonymous-abort treatment — the panic `Display` is its
            // report (D-043). These failures still ABORT the launch
            // (D-027/D-028): their §10 effects ("Update the app",
            // "install the previous version") end the session, and
            // their screens are the future update task's.
            //
            // A database damaged enough to fail `PRAGMA quick_check`
            // (spec §6.3, §10 `E_DB_CORRUPT`, T-F11's damaged-DB
            // half) is the deliberate exception since T-024: the
            // verdict is logged through the facade and the hook
            // returns `Ok` with **no** `Db` managed — recovery mode.
            // §10's effect for this code is a screen *instead of* the
            // app ("data directory path, the list of `db-backups/`
            // copies, 'Open data folder' and 'Start with empty
            // database' buttons"), which requires a live process: the
            // recovery commands (`db_backups_list`,
            // `db_restore_backup`, `db_recreate`,
            // `app_open_data_folder`) work in exactly this state over
            // the managed `DataRoot`, and the recreate/restore ones
            // reopen and manage the database themselves (§7.10: "…
            // and reopens it"). The frontend screen itself is the
            // recovery-UI task's; this hook's contract is: corrupt →
            // logged, alive, database-less.
            let database = match db::Db::open(&data_dir) {
                Ok(database) => database,
                Err(error @ db::DbInitError::Corrupt { .. }) => {
                    // The typed verdict exists (the match arm owns
                    // it); log it through the facade and continue
                    // without the database. `app_error()` is `Some`
                    // for this shape by construction (D-045).
                    if let Some(app_error) = error.app_error() {
                        logging::db_corrupt(&app_error);
                    }
                    // Recovery mode: no `Db` is managed, the daily
                    // step below is skipped with it (there is no
                    // database to copy), and the window opens into
                    // the recovery UI. The command that recovers
                    // performs this step's daily+manage tail itself.
                    return Ok(());
                }
                Err(error) => {
                    match (&error, error.app_error()) {
                        (db::DbInitError::TooNew { .. }, Some(app_error)) => {
                            logging::db_too_new(&app_error);
                        }
                        (db::DbInitError::Migration { .. }, Some(app_error)) => {
                            logging::db_migration_failed(&app_error);
                        }
                        _ => {}
                    }
                    return Err(error.into());
                }
            };
            // The ERR-001 daily backup (spec §10): after the database
            // is open and migrated, one `VACUUM INTO` copy per UTC day
            // (`db-backups/daily-YYYYMMDD.sqlite3`), the newest 7 kept
            // — idempotent within a day, decided by the file name. A
            // fresh first launch gets its copy too ("at every
            // startup"): a cheap copy of the empty database is an
            // honest baseline for the day. Degrade, never abort
            // (DECISIONS.md D-046): the daily copy is extra
            // reliability, not a precondition — a failure is logged
            // through the facade and the launch continues (unlike a
            // pre-migration copy failure, D-038). An existing same-day
            // copy gets no line: it is the idempotent no-op, and a
            // line per restart would only repeat itself.
            match database.ensure_daily_backup(jiff::Timestamp::now()) {
                Ok(eunomia_store::storage::DailyBackupOutcome::Written(path)) => {
                    logging::db_backup_daily(&path);
                }
                Ok(eunomia_store::storage::DailyBackupOutcome::AlreadyPresent(_)) => {}
                Err(error) => logging::db_backup_daily_failed(&error),
            }
            app.manage(database);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod config_tests;

#[cfg(test)]
mod db_tests;

#[cfg(test)]
mod errors_tests;

#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod ipc_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerates `src/bindings.ts` so `cargo test` keeps the committed
    /// bindings in sync with the Rust command definitions. Run
    /// `cargo test -p eunomia-sync` after changing any command, then commit
    /// the updated file.
    ///
    /// The export is followed by an equality check: the file is read back
    /// after the export and compared with the content `include_str!`
    /// embedded at compile time (before this test rewrote the working-tree
    /// copy). A developer who changed the Rust contract but has not
    /// regenerated `src/bindings.ts` sees the test red on that first run —
    /// the same run performs the regeneration, so repeating it is green;
    /// CI, which compiles the committed file before any export, stays red
    /// until the regenerated file is committed. Without the check the test
    /// would silently rewrite the file and stay green (D-034; tightened in
    /// the T-013 review).
    #[test]
    fn export_bindings_are_up_to_date() {
        specta_builder()
            .export(Typescript::default(), "../src/bindings.ts")
            .expect("Failed to export typescript bindings");
        let on_disk = std::fs::read_to_string("../src/bindings.ts")
            .expect("the exported bindings file must be readable");
        assert_eq!(
            on_disk,
            include_str!("../../src/bindings.ts"),
            "src/bindings.ts is stale: the committed bindings differ from \
             what the registered commands and types generate — commit the \
             regenerated file"
        );
    }
}
