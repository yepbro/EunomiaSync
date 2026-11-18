//! Configuration acceptance tests for the bundle identifier and the crate
//! authors (spec 5.5, REL-004).
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

use serde_json::Value;

const EXPECTED_IDENTIFIER: &str = "pro.yepbro.eunomiasync";

/// Reads the real `tauri.conf.json` shipped with the app.
fn tauri_conf() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let raw =
        std::fs::read_to_string(path).expect("tauri.conf.json should exist next to Cargo.toml");
    serde_json::from_str(&raw).expect("tauri.conf.json should be valid JSON")
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
