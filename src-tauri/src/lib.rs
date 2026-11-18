//! EunomiaSync Tauri application entry point.
//!
//! Commands are registered through `tauri-specta`, which generates the
//! typed TypeScript bindings in `src/bindings.ts` on debug builds. The
//! generated file is committed so the frontend can be type-checked
//! without a Rust toolchain.

use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands};

pub mod commands;

// The machine-level `config.toml` (spec §5.8, SYS-011…013) is needed on
// every platform, exactly like the app itself.
pub(crate) mod config;

// The data directory layout (spec §5.5) is needed on every platform,
// exactly like the app itself.
pub(crate) mod data_dirs;

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
    Builder::<tauri::Wry>::new().commands(collect_commands![commands::greet,])
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
    // plugin-free and only application commands (`greet`, later
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod config_tests;

#[cfg(test)]
mod ipc_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerates `src/bindings.ts` so `cargo test` keeps the committed
    /// bindings in sync with the Rust command definitions. Run
    /// `cargo test -p eunomia-sync` after changing any command, then commit
    /// the updated file.
    #[test]
    fn export_bindings_are_up_to_date() {
        specta_builder()
            .export(Typescript::default(), "../src/bindings.ts")
            .expect("Failed to export typescript bindings");
    }
}
