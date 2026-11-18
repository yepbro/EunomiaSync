//! Configuration acceptance tests for the bundle identifier, the crate
//! authors (spec 5.5, REL-004), and the main window geometry (UI-001).
//!
//! The data directory (`app_local_data_dir()`, spec 5.5) is derived by
//! Tauri as `<local_data_dir>/<bundle identifier>` on every OS, so
//! pinning the identifier in `tauri.conf.json` pins the whole 5.5 path
//! table: Windows `%LOCALAPPDATA%/pro.yepbro.eunomiasync/`, macOS
//! `~/Library/Application Support/pro.yepbro.eunomiasync/`, Linux
//! `$XDG_DATA_HOME/pro.yepbro.eunomiasync/`
//! (default `~/.local/share/pro.yepbro.eunomiasync/`).
//!
//! One Tauri key could redirect the directory without touching the
//! identifier: `app > appDirectoriesOverride`. This repo does not set it,
//! and a companion test asserts it stays unset, so the identifier remains
//! the only source of the data-directory path in practice.
//!
//! The main window geometry is pinned the same way: UI-001 (§11.1) asks
//! for one main window that is resizable but never below 960×600 on any
//! OS. Tauri forwards the config's `minWidth`/`minHeight` to the OS
//! window manager as hard minimum-size constraints at window creation
//! (tao `set_min_inner_size`: `WM_GETMINMAXINFO` on Windows, geometry
//! hints on GTK, `NSWindow::setMinSize` on macOS), which clamp interactive and
//! programmatic resizes alike. The two `tauri.conf.json` keys alone
//! therefore decide the acceptance criterion, and the tests below pin
//! them — including that the startup size does not undercut the minimum
//! and that the overflow clamp protecting it (D-021) stays on.

use serde_json::Value;

const EXPECTED_IDENTIFIER: &str = "pro.yepbro.eunomiasync";

/// Expected main window minimum width, from UI-001 (spec §11.1).
const MIN_WINDOW_WIDTH: u64 = 960;
/// Expected main window minimum height, from UI-001 (spec §11.1).
const MIN_WINDOW_HEIGHT: u64 = 600;

/// Reads the real `tauri.conf.json` shipped with the app.
fn tauri_conf() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let raw =
        std::fs::read_to_string(path).expect("tauri.conf.json should exist next to Cargo.toml");
    serde_json::from_str(&raw).expect("tauri.conf.json should be valid JSON")
}

/// Returns the `main` window declaration from the parsed configuration.
///
/// The window is located by its label rather than by its position in
/// `app > windows`: the array may grow (the UI-002 modal windows are
/// planned), and these tests must stay pinned to the main window. An
/// omitted explicit label is still the main window: tauri's serde
/// default for `WindowConfig::label` is `"main"`.
fn main_window(config: &Value) -> &Value {
    let windows = config["app"]["windows"]
        .as_array()
        .expect("tauri.conf.json should declare `app > windows` as an array");
    windows
        .iter()
        .find(|window| window["label"].as_str().unwrap_or("main") == "main")
        .expect("tauri.conf.json should declare a window labelled `main`")
}

/// Reads a numeric window field from a window declaration, trying each
/// JSON spelling in order. Tauri accepts camelCase and kebab-case
/// spellings (serde aliases in `tauri_utils::config::WindowConfig`) and
/// stores the values as `f64`, so `960` and `960.0` are the same value.
/// The effective value is the behavior, not the spelling used to write
/// it down.
fn window_field(window: &Value, spellings: &[&str]) -> u64 {
    let value = spellings
        .iter()
        .find_map(|spelling| window[*spelling].as_f64())
        .unwrap_or_else(|| {
            panic!(
                "the `main` window must set `{}` to a number",
                spellings.join("` or `")
            )
        });
    assert!(
        value.fract() == 0.0 && value.is_sign_positive(),
        "the `main` window `{}` must be a whole non-negative number, got {value}",
        spellings[0]
    );
    value as u64
}

#[test]
fn bundle_identifier_is_pro_yepbro_eunomiasync() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let identifier = config["identifier"]
        .as_str()
        .expect("tauri.conf.json should carry a string `identifier` field");

    // Assert
    assert_eq!(
        identifier, EXPECTED_IDENTIFIER,
        "the bundle identifier — and with it app_local_data_dir() on every OS — \
         must match spec 5.5 / REL-004"
    );
}

#[test]
fn data_directory_override_is_not_set() {
    // Arrange & Act
    let config = tauri_conf();

    // Assert: the key must be absent (`Null`) in both spellings serde
    // accepts (camelCase plus the kebab-case alias); setting either would
    // relocate `app_local_data_dir()` without changing the identifier.
    for key in ["appDirectoriesOverride", "app-directories-override"] {
        assert!(
            config["app"][key].is_null(),
            "app > {key} must stay unset: it would silently move the data \
             directory away from the spec 5.5 paths"
        );
    }
}

#[test]
fn main_window_minimum_size_is_960_by_600() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let window = main_window(&config);

    // Assert: UI-001 (§11.1) caps the smallest allowed window at
    // 960×600. Tauri passes these keys to the OS window manager as hard
    // minimum-size constraints (see the module docs), so pinning the
    // keys pins the "cannot be resized below it on any OS" criterion.
    assert_eq!(
        window_field(window, &["minWidth", "min-width"]),
        MIN_WINDOW_WIDTH,
        "the `main` window minimum width must be {MIN_WINDOW_WIDTH} (UI-001)"
    );
    assert_eq!(
        window_field(window, &["minHeight", "min-height"]),
        MIN_WINDOW_HEIGHT,
        "the `main` window minimum height must be {MIN_WINDOW_HEIGHT} (UI-001)"
    );
}

#[test]
fn main_window_start_size_is_not_below_the_minimum() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let window = main_window(&config);
    let width = window_field(window, &["width"]);
    let height = window_field(window, &["height"]);

    // Assert: the window opens at this size before any user resize, so
    // the startup size must honor the UI-001 minimum too — a smaller
    // start would show an undersized window on first launch. The exact
    // value is a free choice; only the floor is the requirement.
    assert!(
        width >= MIN_WINDOW_WIDTH,
        "the `main` window start width {width} must be at least \
         {MIN_WINDOW_WIDTH} (UI-001)"
    );
    assert!(
        height >= MIN_WINDOW_HEIGHT,
        "the `main` window start height {height} must be at least \
         {MIN_WINDOW_HEIGHT} (UI-001)"
    );
}

#[test]
fn main_window_startup_overflow_is_prevented() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let window = main_window(&config);

    // Assert: the startup size (1280×800) exceeds some common work areas
    // (1920×1080 at 150% scaling is ~1280×688), and tauri clamps the
    // startup frame to the work area only when `preventOverflow` is set —
    // never letting the clamp undercut the 960×600 minimum. This is the
    // D-021 decision, so the key must stay on.
    let prevent_overflow = window["preventOverflow"]
        .as_bool()
        .or_else(|| window["prevent-overflow"].as_bool())
        .expect(
            "the `main` window must set `preventOverflow` (or \
             `prevent-overflow`) to a boolean",
        );
    assert!(
        prevent_overflow,
        "the `main` window must keep `preventOverflow` enabled: without \
         it a 1280×800 startup frame can push the sync panel (MAIN-025) \
         off-screen on small or scaled work areas (D-021)"
    );
}

#[test]
fn cargo_package_authors_are_den_brooks() {
    // Arrange & Act: `CARGO_PKG_AUTHORS` is the compiled form of the
    // `[package] authors` array (multiple entries joined with `:`).
    let authors = env!("CARGO_PKG_AUTHORS");

    // Assert
    assert_eq!(
        authors, "Den Brooks <denis@yepbro.pro>",
        "the crate authors must match REL-004"
    );
}
