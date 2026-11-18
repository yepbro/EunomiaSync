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
//!
//! The webview security posture (SEC-002, §5.6; SEC-004, §13) is pinned
//! the same way. `app > security > csp` is a static policy closed over
//! the app bundle: `'self'`/`'none'` sources everywhere, with the single
//! deliberate exception of the tauri-internal IPC transports on
//! `connect-src` (`ipc:` and `http://ipc.localhost`, D-022). tauri
//! appends the nonces and hashes of its own injected scripts and styles
//! to `script-src`/`style-src` at serve time, so the checked-in policy
//! never needs `'unsafe-inline'` or a remote source. DevTools
//! stay release-off by tauri's defaults (the `devtools` cargo feature is
//! disabled and no window declaration forces `devtools: true`), and no
//! deep-link plugin is wired up anywhere (SEC-004). The tests below keep
//! each of those facts from silently regressing.

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

/// Returns this crate's `Cargo.toml`, embedded at compile time with
/// `include_str!`.
///
/// The whole manifest is scanned, not a single section: a `devtools`
/// feature could also hide in a `[target.'cfg(...)'.dependencies]` table
/// or in `[features]`, and a deep-link crate could be added under a
/// target-specific dependency table — the section-scoped scan those
/// tests used first would have missed both. Accepted residual gaps of an
/// absence pin over file text: the workspace-root manifest and
/// `--features` flags on the command line or in CI live outside this
/// file. If the manifest is ever renamed or moved, this panics with a
/// clear compile/test error, so the pins are consciously updated instead
/// of silently checking nothing.
fn crate_manifest() -> &'static str {
    include_str!("../Cargo.toml")
}

/// The only source expressions the static CSP may use, outside the
/// per-directive exceptions below: everything is either the app bundle
/// (`'self'`) or switched off (`'none'`).
///
/// Nonces and hashes for tauri's own injected code are appended at serve
/// time by tauri itself (asset serving rewrites `script-src`/`style-src`,
/// adding `'self'`, the request nonces and the build-time inline-script
/// hashes) and never appear in the checked-in file. Any other token in
/// the config — a remote host, a wildcard, `data:`, `blob:`, an
/// `unsafe-*` keyword — is a policy change and must fail here until it is
/// a consciously reviewed decision.
const ALLOWED_CSP_SOURCES: &[&str] = &["'self'", "'none'"];

/// Sources allowed in one specific directive only: the tauri IPC
/// transports on `connect-src`. `ipc:` is tauri's own scheme and
/// `http://ipc.localhost` is the loopback host the WebView2 IPC fetch
/// targets (`core.js` `convertFileSrc`) — both are tauri-internal, not
/// internet hosts. Tauri's IPC script (`scripts/ipc-protocol.js`) tries
/// this fetch transport first and only falls back to `postMessage` after
/// a failure, so without these sources every `invoke` would take a
/// doomed fetch round-trip per page load and then latch the fallback,
/// which loses the response channel for the rest of the session.
const ALLOWED_CSP_SOURCES_BY_DIRECTIVE: &[(&str, &[&str])] =
    &[("connect-src", &["ipc:", "http://ipc.localhost"])];

/// Parses a CSP string into its `(directive, sources)` pairs.
fn csp_directives(csp: &str) -> Vec<(&str, Vec<&str>)> {
    csp.split(';')
        .filter_map(|directive| {
            // No `.trim()` here: `split_whitespace` already ignores
            // leading and trailing whitespace around the directive.
            let mut tokens = directive.split_whitespace();
            let name = tokens.next()?;
            Some((name, tokens.collect()))
        })
        .collect()
}

/// Asserts that a CSP directive lists no source outside
/// [`ALLOWED_CSP_SOURCES`] plus its per-directive exceptions
/// ([`ALLOWED_CSP_SOURCES_BY_DIRECTIVE`]).
fn assert_csp_sources_are_local(directive: &str, sources: &[&str]) {
    let directive_sources = ALLOWED_CSP_SOURCES_BY_DIRECTIVE
        .iter()
        .find(|(name, _)| *name == directive)
        .map(|(_, allowed)| *allowed)
        .unwrap_or(&[]);
    for source in sources {
        assert!(
            ALLOWED_CSP_SOURCES.contains(source) || directive_sources.contains(source),
            "the CSP `{directive}` directive lists the source `{source}`: \
             SEC-002 forbids remote sources (http(s):// hosts, bare \
             hostnames, wildcards), data:/blob: URIs and `unsafe-*` \
             keywords in the static policy"
        );
    }
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
fn no_window_forces_devtools() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let windows = config["app"]["windows"]
        .as_array()
        .expect("tauri.conf.json should declare `app > windows` as an array");

    // Assert: `devtools: true` on a window declaration enables the web
    // inspector for that window (tauri forwards the key into the
    // `devtools` webview attribute), which matters the moment the
    // release `devtools` feature is ever turned on — in debug builds the
    // inspector is attached anyway unless the key says otherwise. The
    // key must stay unset or false on every window, so the inspector
    // remains debug-only (SEC-002).
    for window in windows {
        let label = window["label"].as_str().unwrap_or("main");
        let forced = window
            .get("devtools")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        assert!(
            !forced,
            "the `{label}` window must not set `devtools: true`: the web \
             inspector is debug-only (SEC-002)"
        );
    }
}

#[test]
fn csp_is_set_and_defaults_to_self() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .map(str::trim)
        .filter(|csp| !csp.is_empty())
        .expect(
            "app > security > csp must be a non-empty policy string: with \
             `null` (or an empty string) the webview ships without a CSP \
             and every origin is allowed (SEC-002)",
        );

    // Assert: the SEC-002 baseline — the fallback directive locks the
    // webview to the app bundle unless a directive says otherwise.
    assert!(
        csp.contains("default-src 'self'"),
        "the CSP must contain `default-src 'self'` (SEC-002), got: {csp}"
    );
}

#[test]
fn csp_blocks_remote_scripts_styles_and_images() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("app > security > csp must be a policy string (SEC-002)");
    let directives = csp_directives(csp);

    // Assert: SEC-002 names scripts, styles, and images explicitly, so
    // the three directives must exist on their own — not merely via the
    // `default-src` fallback — and must be closed over `'self'`. The
    // explicit directives also keep those resource types locked if
    // `default-src` is ever relaxed for another feature.
    for directive in ["script-src", "style-src", "img-src"] {
        let sources = directives
            .iter()
            .find(|(name, _)| *name == directive)
            .map(|(_, sources)| sources)
            .unwrap_or_else(|| {
                panic!(
                    "the CSP must declare `{directive}` explicitly (SEC-002), \
                     got: {csp}"
                )
            });
        assert_csp_sources_are_local(directive, sources);
    }
}

#[test]
fn csp_has_no_remote_sources_in_any_directive() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("app > security > csp must be a policy string (SEC-002)");
    let directives = csp_directives(csp);

    // Assert: sweep every directive — including ones added later — so a
    // remote source cannot sneak in under a directive this suite does not
    // name (SEC-004: network use belongs to the Rust updater, not the
    // webview).
    for (name, sources) in &directives {
        assert_csp_sources_are_local(name, sources);
    }
}

#[test]
fn csp_declares_the_hardening_directives() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("app > security > csp must be a policy string (SEC-002)");
    let directives = csp_directives(csp);

    // Assert: D-022 adds four directives beyond the resource types
    // because they either do not fall back to `default-src`
    // (`base-uri`, `form-action`) or must be off outright (`object-src`,
    // `frame-src`). Each is pinned to its exact single source so that
    // removing — not just weakening — the directive turns this red.
    let expected = [
        ("object-src", &["'none'"][..]),
        ("base-uri", &["'self'"]),
        ("form-action", &["'self'"]),
        ("frame-src", &["'none'"]),
    ];
    for (name, expected_sources) in expected {
        let sources = directives
            .iter()
            .find(|(directive, _)| *directive == name)
            .map(|(_, sources)| sources.as_slice())
            .unwrap_or_else(|| {
                panic!(
                    "the CSP must declare `{name}` explicitly (D-022), \
                     got: {csp}"
                )
            });
        assert!(
            sources == expected_sources,
            "the CSP `{name}` directive must be exactly \
             `{}` (D-022), got: {csp}",
            expected_sources.join(" ")
        );
    }
}

#[test]
fn dangerous_csp_modification_stays_unset() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();

    // Assert: `dangerousDisableAssetCspModification` would stop tauri
    // from appending its nonces/hashes to `script-src`/`style-src`,
    // breaking tauri's own injected bootstrap code under the strict
    // policy. The key must be absent (`Null`) in both spellings serde
    // accepts (camelCase plus the kebab-case alias), like the
    // neighboring data-directory-override pin. Tauri v2 has no
    // `dangerousRemoteDomainIpcAccess` anymore (a v1 key; the v2 config
    // is `deny_unknown_fields` and rejects it at build time), so there
    // is nothing to pin on that side.
    for key in [
        "dangerousDisableAssetCspModification",
        "dangerous-disable-asset-csp-modification",
    ] {
        assert!(
            config["app"]["security"][key].is_null(),
            "app > security > {key} must stay unset: it would strip the \
             nonce/hash injection tauri applies to this CSP (SEC-002)"
        );
    }
}

#[test]
fn dev_csp_stays_unset() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();

    // Assert: D-022 pins exactly one policy for the app. Desktop dev
    // never reaches the config CSP anyway (the webview loads the Vite
    // server URL directly), so a `devCsp` could only diverge the policy
    // — it must stay unset in both spellings serde accepts (`devCsp`,
    // alias `dev-csp`).
    for key in ["devCsp", "dev-csp"] {
        assert!(
            config["app"]["security"][key].is_null(),
            "app > security > {key} must stay unset: D-022 keeps a single \
             static CSP (SEC-002)"
        );
    }
}

#[test]
fn csp_keeps_the_ipc_fetch_transport_allowed() {
    // Arrange & Act: the real configuration file the app is built from.
    let config = tauri_conf();
    let csp = config["app"]["security"]["csp"]
        .as_str()
        .expect("app > security > csp must be a policy string (SEC-002)");
    let directives = csp_directives(csp);

    // Assert: tauri's IPC script fetches `ipc://localhost/<command>`
    // (`http://ipc.localhost/<command>` on Windows) as its primary
    // transport, and `fetch` is governed by `connect-src`. The directive
    // must exist and list both IPC endpoints: without them every invoke
    // first fails a CSP-blocked fetch and then latches the postMessage
    // fallback, which silently drops the response channel — a slower
    // path for exactly the large payloads (matrix, preview HTML) this
    // app plans.
    let sources = directives
        .iter()
        .find(|(name, _)| *name == "connect-src")
        .map(|(_, sources)| sources)
        .unwrap_or_else(|| {
            panic!(
                "the CSP must declare `connect-src` with the tauri IPC \
                 transports (D-022), got: {csp}"
            )
        });
    for required in ["ipc:", "http://ipc.localhost"] {
        assert!(
            sources.contains(&required),
            "`connect-src` must allow `{required}` — the tauri IPC fetch \
             transport (D-022), got: {csp}"
        );
    }
    assert_csp_sources_are_local("connect-src", sources);
}

#[test]
fn manifest_enables_no_devtools_feature() {
    // Arrange & Act: the whole `src-tauri/Cargo.toml`, compile-time
    // constant.
    let manifest = crate_manifest();

    // Assert: tauri gates DevTools behind its `devtools` cargo feature —
    // without it the web inspector only exists in debug builds
    // (`WindowConfig::devtools` docs: "works in debug builds, but
    // requires `devtools` feature flag in release builds"). The word
    // must not appear in the manifest at all: a `features =
    // ["devtools"]` under any section — `[dependencies]`, a
    // target-specific table, or `[features]` — would re-enable the
    // inspector and violate SEC-002. The bare-word match is deliberately
    // conservative: even a comment mentioning `devtools` turns this red
    // and forces a conscious edit.
    assert!(
        !manifest.contains("devtools"),
        "src-tauri/Cargo.toml must not enable the tauri `devtools` \
         feature anywhere: release builds ship without the web \
         inspector (SEC-002)"
    );
}

#[test]
fn no_deep_link_plugin_is_registered() {
    // Arrange & Act: both registration paths a deep link could take —
    // the plugin crate anywhere in src-tauri/Cargo.toml and a `plugins`
    // entry in tauri.conf.json.
    let manifest = crate_manifest();
    let config = tauri_conf();

    // Assert: SEC-004 — there are no deep links. Wiring one up needs the
    // `tauri-plugin-deep-link` crate in either spelling, in any section.
    assert!(
        !manifest.contains("deep-link")
            && !manifest.contains("deep_link")
            && !manifest.contains("deeplink"),
        "src-tauri/Cargo.toml must not depend on a deep-link plugin \
         crate (SEC-004: no deep links)"
    );

    // ... or a plugin configuration entry. Matching on the key substring
    // keeps the check open to tauri's future plugins (e.g. the planned
    // updater) while still catching a deep-link config block.
    if let Some(plugins) = config["plugins"].as_object() {
        for key in plugins.keys() {
            assert!(
                !key.to_lowercase().contains("deep"),
                "tauri.conf.json `plugins` must not configure a deep-link \
                 plugin (SEC-004: no deep links), found: {key}"
            );
        }
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
