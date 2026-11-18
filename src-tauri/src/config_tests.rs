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
//!
//! The plugin surface (SEC-001, §5.6) is pinned the same way: exactly the
//! four spec §5.3 plugins are wired (`dialog`, `updater`,
//! `single-instance`, `opener`), every one of them is invoked from Rust
//! only — the frontend depends on no plugin JavaScript package and the
//! capability grants the webview no permissions at all — and
//! `single-instance` stays the first registered plugin (plugin
//! requirement), while the app keeps defining no ACL manifest of its own
//! (the precondition that leaves application commands ungated). The
//! updater registers from a deliberately disabled
//! `plugins.updater` stub in `tauri.conf.json`: the plugin refuses to
//! initialize without the section (its config has a required `pubkey`
//! field), and the tests pin both halves of that fact — the stub must
//! exist and stay empty, and a section-less app must fail to build.

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
/// absence pin over file text: `--features` flags on the command line or
/// in CI live outside this file (the workspace-root manifest — the other
/// gap the plugin-set and tray pins once had — is scanned alongside this
/// manifest by [`manifest_texts`]). If the manifest is ever renamed or
/// moved, this panics with a clear compile/test error, so the pins are
/// consciously updated instead of silently checking nothing.
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
    // keeps the check open to plugins added later (the updater is the
    // only `plugins` entry today and passes), while still catching a
    // deep-link config block.
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

/// The exact set of Tauri plugins this app may wire (spec §5.3
/// technologies table): folder picking, updates, second-launch detection,
/// and "open the data folder" — every one invoked from Rust only
/// (SEC-001). `fs` and `shell` are banned outright by SEC-001.
const ALLOWED_PLUGIN_CRATES: &[&str] = &["dialog", "opener", "single-instance", "updater"];

/// Extracts every `tauri-plugin-<name>` crate name mentioned in the
/// manifest texts — the same conservative whole-file scan the devtools
/// and deep-link pins use, so a plugin crate hiding in a target-specific
/// dependency table (or a comment) is caught too. Both the `src-tauri`
/// and the workspace-root manifest are scanned: a plugin could otherwise
/// hide behind a `[workspace.dependencies]` alias
/// (`dialog = { workspace = true }`) that never spells the crate name in
/// `src-tauri/Cargo.toml`.
fn plugin_crate_names(manifest: &str) -> Vec<String> {
    let mut rest = manifest;
    let mut names = Vec::new();
    while let Some(start) = rest.find("tauri-plugin-") {
        let after = &rest[start + "tauri-plugin-".len()..];
        let token: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let name = token.trim_end_matches('-');
        if !name.is_empty() {
            names.push(name.to_string());
        }
        rest = after;
    }
    names.sort();
    names.dedup();
    names
}

/// Concatenates the `src-tauri` and workspace-root manifests for the
/// pins that must see a plugin (or an enabling feature) whichever of the
/// two files declares it.
fn manifest_texts() -> String {
    format!(
        "{}\n{}",
        include_str!("../Cargo.toml"),
        include_str!("../../Cargo.toml")
    )
}

#[test]
fn manifest_pins_the_exact_tauri_plugin_set() {
    // Arrange & Act: the `src-tauri` and workspace-root manifests,
    // compile-time constants.
    let manifest = manifest_texts();
    let names = plugin_crate_names(&manifest);
    let allowed: Vec<String> = ALLOWED_PLUGIN_CRATES
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Assert: SEC-001 / §5.3 — exactly these four plugins and nothing
    // else, in any section of either manifest.
    assert_eq!(
        names, allowed,
        "the manifests must wire exactly {allowed:?} (spec §5.3): the \
         `fs` and `shell` plugins are banned outright (SEC-001) and no \
         other plugin crate is allowed"
    );
}

#[test]
fn manifest_enables_no_tray_support() {
    // Arrange & Act: the `src-tauri` and workspace-root manifests,
    // compile-time constants.
    let manifest = manifest_texts();

    // Assert: D-28 keeps the system tray out of v1. Tray support is not a
    // plugin crate — it is the `tray-icon` feature of the tauri crate
    // itself — so the exact-plugin-set pin cannot catch it; the bare-word
    // scan (deliberately matching comments too, like the devtools pin)
    // does.
    assert!(
        !manifest.contains("tray"),
        "the manifests must not enable tray support in any spelling \
         (tauri crate feature or plugin crate): the tray is out of v1 \
         (D-28)"
    );
}

/// Parses the workspace-root `package.json`, embedded at compile time.
fn frontend_manifest() -> Value {
    serde_json::from_str(include_str!("../../package.json"))
        .expect("package.json should be valid JSON")
}

/// Collects the `@tauri-apps/*` package names from one manifest section.
fn tauri_npm_packages(section: &Value) -> Vec<String> {
    section
        .as_object()
        .expect("the package.json section should be an object")
        .keys()
        .filter(|name| name.starts_with("@tauri-apps/"))
        .cloned()
        .collect()
}

#[test]
fn frontend_manifest_has_no_tauri_plugin_js_packages() {
    // Arrange & Act: the real `package.json` the frontend installs from.
    let manifest = frontend_manifest();
    let dependencies = tauri_npm_packages(&manifest["dependencies"]);
    let dev_dependencies = tauri_npm_packages(&manifest["devDependencies"]);

    // Assert: SEC-001 keeps every plugin on the Rust side, so the
    // frontend depends on the typed IPC core alone (`src/bindings.ts`
    // imports only `@tauri-apps/api/core`); the CLI stays a dev tool. An
    // `@tauri-apps/plugin-*` package here would put a plugin's JS API
    // back within the webview's reach — under any dependency section,
    // so the optional/peer sections must not exist at all.
    assert!(
        manifest["optionalDependencies"].is_null() && manifest["peerDependencies"].is_null(),
        "package.json must not carry optionalDependencies or \
         peerDependencies sections: they would escape the exact \
         dependencies/devDependencies pins below"
    );
    assert_eq!(
        dependencies,
        ["@tauri-apps/api"],
        "dependencies must carry exactly @tauri-apps/api among the \
         @tauri-apps/* packages: plugin JS packages are banned (SEC-001 — \
         plugins are invoked from Rust)"
    );
    assert_eq!(
        dev_dependencies,
        ["@tauri-apps/cli"],
        "devDependencies must carry exactly @tauri-apps/cli among the \
         @tauri-apps/* packages"
    );
}

/// Recursively lists every capability file tauri would load from
/// `src-tauri/capabilities/`, as paths relative to that directory.
/// `tauri-build` discovers `capabilities/**/*` — a recursive glob — and
/// loads every `json`/`toml`/`json5` file it matches, referenced from
/// `tauri.conf.json` or not, so the pin below must see exactly that set
/// (a second file, in any accepted format at any depth, would otherwise
/// grant permissions outside every pin). Schema files under a
/// `schemas/` subfolder are not special-cased: tauri ignores them, but
/// this repo has none, and adding one is a conscious edit.
fn capability_files() -> Vec<String> {
    const CAPABILITY_FILE_EXTENSIONS: &[&str] = &["json", "toml", "json5"];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut files = Vec::new();
    fn walk(dir: &std::path::Path, root: &std::path::Path, files: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("the capabilities directory should exist") {
            let path = entry
                .expect("every capabilities entry should be readable")
                .path();
            if path.is_dir() {
                walk(&path, root, files);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| CAPABILITY_FILE_EXTENSIONS.contains(&ext))
            {
                let relative = path
                    .strip_prefix(root)
                    .expect("walked paths are inside the capabilities root")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push(relative);
            }
        }
    }
    walk(&root, &root, &mut files);
    files.sort();
    files
}

#[test]
fn capability_grants_the_webview_no_permissions_at_all() {
    // Arrange & Act: the real capability set tauri would load. The file
    // set itself is pinned to exactly one file — a second capability
    // file (any accepted extension, at any depth) cannot appear
    // unnoticed — and inline capabilities, which `tauri.conf.json` can
    // also carry under `app > security > capabilities` (tauri-utils
    // `CapabilityEntry`), are checked separately.
    let files = capability_files();
    let config = tauri_conf();
    assert_eq!(
        files,
        ["default.json"],
        "src-tauri/capabilities/ must contain exactly the pinned \
         `default.json` (any other json/toml/json5 file, at any depth, \
         would be loaded into the ACL and grant permissions outside \
         D-024)"
    );

    // Assert: SEC-001 / D-024 — the webview talks only to application
    // commands, and application commands are not capability-gated, so the
    // honest grant is the empty set. Anything listed in the capability
    // (`opener:`, `dialog:`, `fs:`, `shell:`, or even a `core:*` default
    // set) hands a native API back to the webview; extend consciously —
    // together with this pin and D-024 — only when the frontend actually
    // needs one.
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json"),
    )
    .expect("capabilities/default.json should be readable");
    let capability: Value =
        serde_json::from_str(&raw).expect("capabilities/default.json should be valid JSON");
    let permissions = capability["permissions"]
        .as_array()
        .expect("capabilities/default.json should list `permissions` as an array");
    assert!(
        permissions.is_empty(),
        "capabilities/default.json must grant nothing to the webview \
         (SEC-001: plugins are invoked from Rust; application commands \
         are not gated), found: {permissions:?}"
    );

    // Assert: no inline capability may slip past the file pins. A
    // non-empty inline array is a grant made outside D-024; a string
    // entry would merely reference a capability file, which the exact
    // file-set pin above already covers.
    let inline = config["app"]["security"]["capabilities"].as_array();
    assert!(
        inline.map(|entries| entries.is_empty()).unwrap_or(true),
        "app > security > capabilities must stay unset or empty: inline \
         capabilities would grant permissions outside the pinned \
         capability files (SEC-001 / D-024)"
    );
}

#[test]
fn app_defines_no_acl_manifest() {
    // Arrange & Act: the two places an application ACL manifest could
    // come from — a custom `permissions/` directory picked up by
    // tauri-build, and an explicit `app_manifest` in `build.rs`.
    let permissions_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("permissions");
    let build_script = include_str!("../build.rs");

    // Assert: the empty capability (D-024) only leaves application
    // commands callable while the app defines no ACL manifest of its own.
    // With one, tauri-build inserts `__app-acl__`, `has_app_acl` flips
    // true, and the invoke path starts gating every application command
    // the empty capability does not grant — the whole IPC surface would
    // die at runtime while every other test stays green. Granting app
    // commands consciously (permissions directory plus capability
    // entries) must update this test and D-024 together.
    assert!(
        !permissions_dir.exists(),
        "src-tauri/permissions/ must not exist: defining an app ACL \
         manifest would capability-gate every application command while \
         the capability grants nothing (D-024)"
    );
    assert!(
        !build_script.contains("app_manifest"),
        "build.rs must not declare an app ACL manifest \
         (tauri_build::Attributes::app_manifest): it would \
         capability-gate every application command the empty capability \
         does not grant (D-024)"
    );
}

/// Strips `//` line comments and `/* ... */` block comments, cutting
/// each line at the first `//` and removing everything between `/*` and
/// the next `*/`, ignoring string literals. Deliberately conservative
/// for the marker scan below: stripping can only remove text, so it can
/// never over-detect a marker — the worst case is a string-embedded
/// comment delimiter hiding a live marker, which fails the pin with its
/// exactly-once message instead of passing silently.
fn strip_comments(source: &str) -> String {
    let mut stripped = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(block_start) = rest.find("/*") {
        // Line comments inside the text before the block comment, but
        // not a `//` line comment containing the `/*` itself — that only
        // over-strips, which the conservativeness note above covers.
        stripped.push_str(&strip_line_comments(&rest[..block_start]));
        match rest[block_start + 2..].find("*/") {
            Some(block_len) => rest = &rest[block_start + 2 + block_len + 2..],
            // Unterminated block comment: strip everything that follows.
            None => return stripped,
        }
    }
    stripped.push_str(&strip_line_comments(rest));
    stripped
}

/// Cuts every line at the first `//` (string literals are ignored, so a
/// `//` inside one truncates its line — see [`strip_comments`]).
fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| &line[..line.find("//").unwrap_or(line.len())])
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn single_instance_plugin_registers_before_the_others() {
    // Arrange & Act: `lib.rs`, compile-time constant, with line and
    // block comments stripped so prose mentioning a registration cannot
    // satisfy the pin. The markers are the registration expressions, and
    // the required registration order is single-instance first, then the
    // remaining three.
    let lib = strip_comments(include_str!("lib.rs"));
    let markers = [
        "tauri_plugin_single_instance::init(",
        "tauri_plugin_dialog::init()",
        "tauri_plugin_updater::Builder::new()",
        "tauri_plugin_opener::init()",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must register `{marker}` exactly once, live (a \
                 commented-out or duplicated registration must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    // Assert: the single-instance plugin documents that it must be
    // registered before every other plugin; the remaining three are
    // pinned ascending as well, so removing or reordering any
    // registration turns this red.
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must register `single-instance` first and every later \
             plugin after its predecessor (single-instance plugin \
             requirement); marker positions: {positions:?}"
        );
    }
}

#[test]
fn updater_config_section_is_a_disabled_stub_that_deserializes() {
    // Arrange & Act: the real `plugins.updater` value from
    // `tauri.conf.json`, deserialized with the actual config type the
    // plugin initializes from.
    let config = tauri_conf();
    let stub = config["plugins"]["updater"].clone();

    // Assert (negative control): tauri feeds a plugin `Value::Null` when
    // its config section is absent, and the updater refuses that — the
    // companion `updater_plugin_refuses_to_initialize_without_a_config_
    // section` test proves it end-to-end. If a future plugin version
    // starts accepting an absent section, this assertion fails and the
    // stub becomes removable: decide consciously, then.
    assert!(
        serde_json::from_value::<tauri_plugin_updater::Config>(serde_json::Value::Null).is_err(),
        "the updater config must keep rejecting an absent `plugins.updater` \
         section; once it accepts one, the tauri.conf.json stub is dead \
         weight and should be removed"
    );

    // Assert: the stub deserializes into the real plugin config with the
    // updater effectively disabled — no endpoints, no public key, no
    // dangerous transport relaxation. The release task fills the key and
    // endpoints; the escapes must stay off.
    let parsed: tauri_plugin_updater::Config = serde_json::from_value(stub).expect(
        "`plugins.updater` must deserialize into the plugin config: \
         without a valid section the app fails to start (the plugin \
         config has a required `pubkey` field)",
    );
    assert!(
        parsed.endpoints.is_empty(),
        "`plugins.updater` must carry no endpoints until the release task \
         sets the real update feed"
    );
    assert!(
        parsed.pubkey.is_empty(),
        "`plugins.updater` must carry no public key until the release \
         task generates one"
    );
    assert!(
        !parsed.dangerous_insecure_transport_protocol
            && !parsed.dangerous_accept_invalid_certs
            && !parsed.dangerous_accept_invalid_hostnames,
        "`plugins.updater` must keep every `dangerous*` transport escape \
         off"
    );
    assert!(
        !parsed.allow_downgrades && !parsed.require_signed_version,
        "`plugins.updater` must keep `allowDowngrades` off (it removes the \
         only guard against installing an older validly signed release) \
         and `requireSignedVersion` at its default while the stub is \
         inert: both are release-task signing-policy decisions that must \
         be made consciously, not stub edits"
    );
}

#[test]
fn updater_plugin_refuses_to_initialize_without_a_config_section() {
    // Arrange: the mock runtime (the same `tauri::test` harness the IPC
    // tests use) with a context that carries no `plugins.updater`
    // section — what tauri would hand the plugin if the stub were
    // deleted from tauri.conf.json.
    let result = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(tauri::test::mock_context(tauri::test::noop_assets()));

    // Assert: plugin initialization (and with it app startup) fails, and
    // the failure is attributable to the updater plugin config — proving
    // the stub in tauri.conf.json is required, not decoration.
    let error = match result {
        Ok(_) => panic!(
            "the updater plugin must refuse to initialize without a \
             `plugins.updater` section"
        ),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("updater"),
        "the failure should be attributable to the updater plugin config, \
         got: {error}"
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
