//! EunomiaSync Tauri application entry point.
//!
//! Commands are registered through `tauri-specta`, which generates the
//! typed TypeScript bindings in `src/bindings.ts` on debug builds. The
//! generated file is committed so the frontend can be type-checked
//! without a Rust toolchain.

use specta_typescript::Typescript;
use tauri_specta::{Builder, collect_commands};

pub mod commands;

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
    // process) — the plugin REQUIRES being registered before every other
    // plugin, so it is wired first. Desktop-only: the crate is compiled
    // out on mobile, hence the cfg. The callback is a T-007 placeholder
    // that only logs; focusing the first instance's main window is T-008.
    // `eprintln!` stands in until the tracing setup task lands (spec §5.3
    // file logging); argv/cwd are deliberately not logged — they can
    // carry user paths and the event needs no content until T-008.
    #[cfg(desktop)]
    let tauri_builder =
        tauri_builder.plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {
            eprintln!("single-instance: second launch handed off and ignored");
        }));

    // `dialog` (SEC-001, SEC-003): the native folder picker runs in Rust
    // behind the `dialog_pick_folder` app command (upcoming task), so the
    // picked path can be tracked per session and every other path stays
    // registered-project-only. The webview has no `dialog:` permission.
    let tauri_builder = tauri_builder.plugin(tauri_plugin_dialog::init());

    // `updater` (D-005/D-027, SEC-001: network belongs to Rust): update
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
