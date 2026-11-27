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
//! only — the frontend depends on no plugin JavaScript package — and
//! `single-instance` stays the first registered plugin (plugin
//! requirement), while the app keeps defining no ACL manifest of its own
//! (the precondition that leaves application commands ungated). The
//! capability's grant stayed empty until the first event consumer: since
//! T-246 (D-112's follow-up) it is exactly the event subscription
//! lifecycle — `core:event:allow-listen` plus `core:event:allow-unlisten`
//! — pinned by its own test below; everything else stays denied. The
//! updater initializes from the real `plugins.updater` section (T-133,
//! D-229): the embedded minisign public key — checked through the
//! plugin's own decode chain (base64 →
//! `minisign_verify::PublicKey::decode`) — and the stable channel's
//! single endpoint, pinned against the `updater` module's constant so
//! the config and the module cannot drift apart; the `dangerous*`
//! transport escapes and the signing-policy flags stay at their
//! defaults, and a section-less app must still fail to build (the
//! negative control D-023 keeps proving the section is required).
//!
//! The Windows application manifest (NFR-023, SEC-012/NFR-011) is pinned
//! the same way, with an explicit honesty boundary: on Linux the build
//! script compiles and runs, but tauri-build's Windows-resource branch
//! never executes (it is gated on the target triple at runtime), so what
//! is verifiable here is the manifest text and its wiring in `build.rs`.
//! No automated check validates the embedded result: `rc.exe` embeds the
//! manifest as opaque bytes without parsing the XML, and the CI `build`
//! matrix job on Windows (`tauri build --debug --no-bundle`) only proves
//! the manifest compiles and links — it never runs the binary. The two
//! runtime criteria — long paths accepted, no UAC prompt — are manual
//! Windows smoke facts with a written procedure (ARCHITECTURE.md,
//! Platform notes). `WindowsAttributes::app_manifest` replaces
//! tauri-build's default manifest wholesale (the default carries only
//! the Common-Controls v6 dependency), so the pins cover both halves:
//! the reproduced dependency (needed by the dialog plugin) and the
//! added `longPathAware`/`asInvoker` settings.
//!
//! The config startup wiring (SYS-011, §5.8) is pinned the same way: the
//! `setup` hook must load `config.toml` after the layout (the file lives
//! in the data directory), publish the config as managed state, then
//! initialize the tracing file log (after the config — SYS-011's order —
//! reading `log_level` back through `app.state::<AppConfig>()`), keep the
//! writer guard alive as managed state, and only then re-emit the config
//! warnings so they land in the log file. The seam's own behavior
//! (defaults, unknown keys, broken-file policy) is unit-tested in
//! `config.rs`, and the subscriber's behavior in `logging.rs`; the pins
//! here keep the wiring from silently unwiring.
//!
//! The logging security posture (§5.3, SEC-008) is pinned the same way,
//! with two text scans over the workspace sources. First, every tracing
//! event of the app is constructed in `logging.rs` alone — the event
//! macros and `#[instrument]` (whose auto-captured fields would happily
//! serialize a `content: String` argument into the log) may not appear
//! anywhere else, so a future task cannot interpolate instruction text
//! into a message without consciously editing the pin. Second, Tauri's
//! `app_log_dir()` stays out of the codebase (live code, comments
//! stripped): the log path must be derived from the data directory
//! (§5.5), not from the OS-specific default that macOS points at
//! `~/Library/Logs`.
//!
//! The network isolation posture (NFR-012, NFR-013, SEC-004) is pinned
//! the same way: a text scan over the same sources (comments stripped)
//! rejects every HTTP/socket client spelling, so the app's own code
//! contains no network code at all — the only network client the app
//! ever reaches is the one inside `tauri-plugin-updater`, invoked
//! through the plugin's public API from the `updater` module (which is
//! NFR-013's "separate module"). The webview side of the same fact is
//! already pinned by the CSP and capability tests above.

use serde_json::Value;

// The pubkey well-formedness pin decodes through base64 (the plugin's
// own first decode step); the dev-dependency edge is declared in
// Cargo.toml with its justification.
use base64::Engine;

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

/// Returns the Windows application manifest, embedded at compile time
/// with `include_str!` from the same file `build.rs` wires in. A moved
/// or renamed file fails this compile (and the build script's own
/// `include_str!`), so the pins below are consciously updated instead of
/// silently checking nothing — the same policy as [`crate_manifest`].
///
/// The pins over this text carry a residual gap, accepted like the other
/// file-text pins: they cannot prove that the manifest actually reached
/// the PE binary or that Windows accepts it — `rc.exe` embeds the file as
/// opaque bytes without validating the XML, the Windows CI build job only
/// proves compile+link (it never runs the binary), so the runtime facts
/// (embedded content, long paths accepted, no UAC prompt) rest on the
/// manual Windows smoke run described in ARCHITECTURE.md (see the module
/// docs).
fn windows_app_manifest() -> &'static str {
    include_str!("../windows/app.manifest")
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
fn setup_resolves_the_data_directory_and_creates_its_layout() {
    // Arrange & Act: `lib.rs`, compile-time constant, with line and
    // block comments stripped so prose about the data directory cannot
    // satisfy the pin. The markers are the `setup` closure itself, the
    // propagating path resolution, and the propagating layout call: the
    // `?` is part of each marker, so wiring that swallows the error
    // (`let _ = …`, `unwrap_or_default()`, a gated-off call) fails here
    // too, not only at the seam's own tests. The order is the one
    // startup needs (spec §5.5: the directory exists before the app's
    // own `config.toml` and database code runs).
    let lib = strip_comments(include_str!("lib.rs"));
    let markers = [
        ".setup(move |app|",
        "app.path().app_local_data_dir()?",
        "data_dirs::ensure_data_layout(&data_dir)?",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    // Assert: the layout must be created inside `setup`, from the
    // resolved data directory, with both fallible steps propagated
    // (`?`). Honest boundary, same as the plugin-registration pin: this
    // proves the wiring exists and stays wired, not that the OS
    // directories appeared — the runtime effect is covered by the
    // `data_dirs.rs` seam tests plus a manual first launch on a clean
    // system.
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must resolve `app_local_data_dir()` inside `setup` \
             and create the spec §5.5 layout from it, in that order; \
             marker positions: {positions:?}"
        );
    }
}

#[test]
fn setup_loads_config_then_logging_then_warnings() {
    // Arrange & Act: `lib.rs`, compile-time constant, with line and
    // block comments stripped so prose about the config cannot satisfy
    // the pin. SYS-011 orders the config before logging and the
    // database, and the file lives in the data directory (spec §5.5),
    // so the markers pin the full startup order: layout first, then the
    // config seam, then the managed state, then the level read back
    // through it, then the logging init from the data directory's
    // `logs/` subfolder, then the guard-keeping `app.manage(logger)`,
    // and only then the warning re-emission (after the subscriber
    // exists, the warnings land in the log file). The `?` is part of
    // the config marker, so wiring that swallows the error (`let _ =
    // …`, `unwrap_or_default()`, a gated-off call) fails here too.
    // Dropping `app.manage(config)` would strand the level read; dropping
    // `app.manage(logger)` would drop the non-blocking writer guard and
    // silently stop the file log mid-run.
    let lib = strip_comments(include_str!("lib.rs"));
    let markers = [
        "data_dirs::ensure_data_layout(&data_dir)?",
        "config::load_or_create(&data_dir)?",
        "app.manage(config);",
        "app.state::<config::AppConfig>().log_level",
        "logging::init(&logs_dir, level)",
        "app.manage(logger);",
        "config::emit_warnings(&warnings);",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    // Assert: the config is loaded from the data directory inside
    // `setup`, after the layout; logging initializes after the config
    // (SYS-011) from the managed state; the warnings are re-emitted
    // after both. Honest boundary, same as the layout and plugin pins:
    // this proves the wiring exists and stays wired and ordered, not
    // that the OS file appeared — the runtime effect is covered by the
    // `config.rs`/`logging.rs` seam tests plus a manual first launch on
    // a clean system.
    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must run the startup steps in the SYS-011 order \
             (layout → config → manage → level → logging → guard → \
             warnings); marker positions: {positions:?}"
        );
    }
}

/// Recursively collects every `.rs` file under `dir` into `files`.
///
/// Shared by the logging pins: both must see sources wherever they land
/// inside `src-tauri/src` (subdirectories included) and the library
/// crates — a macro call or an `app_log_dir()` wiring hidden in a new
/// module of any crate must not escape the pin.
fn collect_rust_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("reading `{}` must work: {error}", dir.display()))
    {
        let path = entry
            .expect("every directory entry should be readable")
            .path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// Every `.rs` source of the workspace the logging pins scan:
/// `src-tauri/src` (subdirectories included), the `src-tauri` build
/// script, the library crates under `crates/`, and the bench crate
/// under `benches/` (a workspace member — the round-1 P3-5 fix: a
/// network spelling planted there was outside the walker while the
/// pins' prose claimed "every workspace source"). A macro call, an
/// `app_log_dir()` wiring or a network client in any of them must not
/// escape the pin. A future top-level source directory (e.g. a
/// `src-tauri/tests/` integration suite) must be added here when it
/// appears — the floor check in the scans (`files` non-empty) cannot
/// notice its absence.
fn workspace_rust_sources() -> Vec<std::path::PathBuf> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rust_files(&manifest_dir.join("src"), &mut files);
    collect_rust_files(&manifest_dir.join("../crates"), &mut files);
    collect_rust_files(&manifest_dir.join("../benches"), &mut files);
    // The build script is outside `src/` but compiles into the same
    // app; a missing file fails the pins' read below, so a rename is a
    // conscious edit, not a silent narrowing of the scan.
    files.push(manifest_dir.join("build.rs"));
    files.sort();
    files
}

#[test]
fn tracing_events_are_constructed_only_in_the_logging_module() {
    // Arrange & Act: every workspace `.rs` source, scanned raw (comments
    // included — deliberately conservative, like the devtools and tray
    // pins: even a comment showing a macro call turns this red and
    // forces a conscious edit). `logging.rs` is the owner of event
    // construction (its SEC-008 facade — see that module's docs), and
    // `*_tests.rs` files never ship: cfg(test) code runs without the
    // production subscriber and is exempt.
    let files = workspace_rust_sources();
    assert!(
        !files.is_empty(),
        "the source walker must find the workspace sources"
    );
    let macro_needles = [
        "error!(",
        "warn!(",
        "info!(",
        "debug!(",
        "trace!(",
        "event!(",
        // Substring of span!/info_span!/debug_span!/…_span! — one needle
        // covers every span macro spelling; spans are banned with events
        // because a span's fields would carry the same content an event
        // field could (and `FmtSpan::NONE`, the fmt default, only makes
        // them *unprinted*, not uncollected).
        "span!(",
        "tracing::instrument",
        "#[instrument",
    ];
    for file in &files {
        // `logging.rs` is exempted by its full in-repo path, not its
        // bare name: a future `crates/<crate>/src/logging.rs` must NOT
        // inherit the exemption — the library crates cannot import this
        // module and need their own SEC-008 discipline when they start
        // logging (the logging module docs say exactly that).
        if file.ends_with("src-tauri/src/logging.rs") {
            continue;
        }
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file names must be UTF-8");
        if name.ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("a listed source file must be readable");
        for needle in macro_needles {
            assert!(
                !text.contains(needle),
                "`{}` must not use `{needle}` directly: every tracing \
                 event is constructed in `src-tauri/src/logging.rs` from \
                 typed, SEC-008-safe parameters (paths, codes, IDs — never \
                 instruction text or file content). Need a new event? Add \
                 a typed helper there (SEC-008; see the logging module docs)",
                file.display()
            );
        }
    }
}

#[test]
fn app_log_dir_is_never_called() {
    // Arrange & Act: every workspace `.rs` source with line and block
    // comments stripped — the honest precision for this pin, in both
    // directions: `data_dirs.rs` and `logging.rs` legitimately *explain*
    // the rejection of `app_log_dir()` in their docs (prose, stripped),
    // while an actual wiring would be live code
    // (`app.path().app_log_dir()`), which stripping keeps visible. `logging.rs`
    // itself is scanned — it is exactly where a "convenience" switch to
    // the Tauri default would land.
    let files = workspace_rust_sources();
    for file in &files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file names must be UTF-8");
        if name.ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("a listed source file must be readable");
        let stripped = strip_comments(&text);
        assert!(
            !stripped.contains("app_log_dir"),
            "`{}` must not call `app_log_dir()`: the file log is written \
             to `<data dir>/logs/` (spec §5.5) on every OS — on macOS the \
             Tauri default would move it to `~/Library/Logs/…`, away from \
             the data directory the spec pins (D-027; the manual \
             per-OS smoke run checks the file location)",
            file.display()
        );
    }
}

#[test]
fn workspace_sources_contain_no_network_client_code() {
    // Arrange & Act: every workspace `.rs` source (comments stripped —
    // the honest precision of the sibling scans: prose explaining the
    // network ban in a module's docs must not trip the pin, while an
    // actual wiring is live code). `*_tests.rs` files never ship
    // (cfg(test)) and are exempt, like every other scan in this module.
    // No file is exempt otherwise: not the `updater` module (the plugin
    // is reached through its API — `UpdaterExt` — which is not a client
    // spelling), not the build script, not the library crates.
    let files = workspace_rust_sources();
    assert!(
        !files.is_empty(),
        "the source walker must find the workspace sources"
    );
    // The needles: the common spellings that open or drive a network
    // client — the HTTP client crates by name (the plugin's own
    // `reqwest` included: if app code ever touches the client crate
    // directly, that is network code leaving the plugin's API), the
    // raw std sockets and their module path, the alternative client
    // crates' namespaces, and the libc socket syscalls (libc itself is
    // a legitimate direct dependency here). Not an exhaustive blacklist
    // of everything socket-shaped: a NEW networked crate is a
    // dependency decision behind rule 9 and cargo-deny, and this pin's
    // job is to keep the app's own code free of the known clients — a
    // needle planted in a dependency's sources is out of scope either
    // way. A `Url::parse` or an `http::` type alone is NOT a needle:
    // parsing and header types are not connections; the pin trips on
    // code that can move bytes to the network.
    let network_needles = [
        "reqwest",
        "ureq",
        "isahc",
        "attohttpc",
        "surf::",
        "curl::",
        "hyper::",
        "hyper_util::",
        "socket2::",
        "tungstenite::",
        "quinn::",
        "tonic::",
        "libc::socket",
        "libc::connect",
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "SocketAddr",
        "std::net",
    ];
    for file in &files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file names must be UTF-8");
        if name.ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("a listed source file must be readable");
        let stripped = strip_comments(&text);
        for needle in network_needles {
            assert!(
                !stripped.contains(needle),
                "`{}` must not use `{needle}` (live code): NFR-012/013 — \
                 the app's only network use is the updater plugin's own \
                 check/download of the signed artifact, reached through \
                 the plugin API from `src-tauri/src/updater.rs`; any \
                 other network code belongs there or nowhere (SEC-004)",
                file.display()
            );
        }
    }
}

#[test]
fn log_internal_errors_stays_off_in_the_shared_subscriber_builder() {
    // Arrange & Act: `logging.rs`, compile-time constant, with comments
    // stripped. The fmt layer's writer-error fallback is an `eprintln!`
    // that panics on an unusable stderr (a Windows GUI release build has
    // none), and `panic = "abort"` turns that into a process abort —
    // the no-panic contract is a vendored-source fact that a unit test
    // cannot reproduce (the test harness keeps stderr writable), so the
    // flag is pinned textually instead, like the Windows manifest facts.
    // Exactly once: all three construction sites go through the one
    // shared `fmt_subscriber`, so a second occurrence means someone
    // built a subscriber outside the shared path.
    let logging = strip_comments(include_str!("logging.rs"));
    assert_eq!(
        logging.matches(".log_internal_errors(false)").count(),
        1,
        "the shared subscriber builder must keep `.log_internal_errors(false)`: \
         the fmt layer's default writer-error fallback is an `eprintln!` that \
         panics on an unusable stderr — fatal under `panic = \"abort\"` \
         exactly in the degraded states this code exists for (D-032)"
    );
}

#[test]
fn config_load_warning_keeps_its_single_caller() {
    // Arrange & Act: every workspace `.rs` source (raw text). The
    // `config_load_warning` helper is the facade's one `&str`-carrying
    // exception (config diagnostics, the D-028 deliberate carve-out);
    // Rust visibility cannot restrict its callers within the crate, so
    // the caller set is pinned instead: the definition/test site
    // (`logging.rs`) and the config seam's `emit_warnings`
    // (`config.rs`). A new caller anywhere else — e.g. a future task
    // funneling some other free text through the exception — must
    // consciously edit this pin and justify itself against SEC-008.
    // `*_tests.rs` files are exempt: they never ship, and this pin's
    // own needle lives in one.
    let files = workspace_rust_sources();
    for file in &files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file names must be UTF-8");
        if name.ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("a listed source file must be readable");
        if !text.contains("config_load_warning") {
            continue;
        }
        assert!(
            file.ends_with("src-tauri/src/logging.rs") || file.ends_with("src-tauri/src/config.rs"),
            "`{}` must not reference `config_load_warning`: it is the \
             config-seam-only exception to the typed SEC-008 facade \
             (D-028/D-032) — new warning kinds get their own typed helper \
             in `logging.rs`, never free text through this one",
            file.display()
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
fn capability_grants_the_webview_only_event_listening() {
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

    // Assert: SEC-001 / D-024 kept the grant empty until the first
    // frontend event consumer; T-246 (D-112's follow-up) extends it
    // consciously by exactly the event SUBSCRIPTION lifecycle. The
    // generated `events.stateProjectAvailability.listen()` reaches the
    // core event plugin's `plugin:event|listen` command, and the
    // unsubscribe handle it returns reaches `plugin:event|unlisten` —
    // a webview that could subscribe but not unsubscribe would leak a
    // listener per subscription, so the pin carries both halves. Note
    // the identifiers' real spelling: tauri 2.12.6's core event
    // permissions are `core:event:allow-listen`/`-unlisten` (there is no
    // `core:event:listen`; an unknown identifier fails the build).
    // Everything else — `core:event:allow-emit`, `-emit-to`, any
    // `opener:`/`dialog:`/`fs:`/`shell:` permission — stays denied:
    // plugins and emits belong to the Rust side, and application
    // commands are not capability-gated. Extend consciously — together
    // with this pin and D-024 — only when the frontend actually needs
    // one more permission.
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json"),
    )
    .expect("capabilities/default.json should be readable");
    let capability: Value =
        serde_json::from_str(&raw).expect("capabilities/default.json should be valid JSON");
    let permissions: Vec<&str> = capability["permissions"]
        .as_array()
        .expect("capabilities/default.json should list `permissions` as an array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("every capability permission must be a string identifier")
        })
        .collect();
    assert_eq!(
        permissions,
        ["core:event:allow-listen", "core:event:allow-unlisten"],
        "capabilities/default.json must grant the webview exactly the \
         event subscription lifecycle — `core:event:allow-listen` plus \
         `core:event:allow-unlisten` (T-246, D-112) — and nothing else: \
         SEC-001 keeps plugins and event emits on the Rust side, and \
         application commands are not capability-gated"
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
    // tauri-build, and an explicit `Attributes::app_manifest` call in
    // `build.rs`. The Windows application manifest (T-009/NFR-023)
    // legitimately uses `WindowsAttributes::app_manifest` — the same
    // method name on an unrelated type — so the build-script pin below
    // distinguishes the two APIs instead of banning the bare substring
    // it once banned.
    let permissions_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("permissions");
    let build_script = include_str!("../build.rs");

    // Assert: the listen-only capability (D-024, extended by T-246
    // without touching application commands) only leaves application
    // commands callable while the app defines no ACL manifest of its
    // own. With one, tauri-build inserts `__app-acl__`, `has_app_acl`
    // flips true, and the invoke path starts gating every application
    // command the capability does not grant — the whole IPC surface
    // would die at runtime while every other test stays green. Granting
    // app commands consciously (permissions directory plus capability
    // entries) must update this test and D-024 together.
    assert!(
        !permissions_dir.exists(),
        "src-tauri/permissions/ must not exist: defining an app ACL \
         manifest would capability-gate every application command while \
         the capability grants no application-command permissions (D-024)"
    );
    // The ACL form needs an `AppManifest` value, and every spelling that
    // provides one names the struct (`AppManifest::new()`, `use
    // tauri_build::AppManifest`, ...). The one inference-based spelling
    // that would not (`.app_manifest(Default::default())`) is caught by
    // `build_script_wires_the_windows_manifest`, which requires exactly
    // one `.app_manifest(` call — the Windows one.
    assert!(
        !build_script.contains("AppManifest"),
        "build.rs must not declare an app ACL manifest \
         (tauri_build::Attributes::app_manifest with the AppManifest \
         struct): it would capability-gate every application command the \
         capability does not grant (D-024)"
    );
}

/// Strips `//` line comments and `/* ... */` block comments (nested
/// blocks included, as Rust nests them), leaving string, byte-string,
/// char and raw-string literals intact — the scanner is literal-aware
/// (D-229, the round-1 P2-1 fix).
///
/// Why literal-awareness is load-bearing: the previous delimiter-only
/// stripper treated any `//` or `/*` as a comment start — including
/// ones *inside* string literals. A URL literal cut the line at
/// `https:`, and a glob literal like `"**/*.md"` opened a phantom
/// block comment with no closing `*/`, silently dropping the rest of
/// the file: live network code after such a line was invisible to the
/// NFR-012/013 pin while it stayed green (reproduced by the round-1
/// reviewer with an exact replica, and now pinned red by the
/// `strip_comments` unit tests below).
///
/// Safety direction, deliberately: string and char literal *contents
/// are kept verbatim* (a marker or needle inside a literal is visible
/// to every scan — over-detection, a red pin and a conscious edit),
/// comment contents are dropped, and malformed literals fall back to
/// treating the opening quote as plain code (a raw newline inside a
/// `"…"` string, which valid Rust cannot contain, ends the literal
/// state) — no VALID RUST input shape can make the stripper swallow
/// live text silently (round-2 P3-3: the claim is scoped to
/// compiling sources, which is all the pins ever read; a malformed,
/// non-compiling input — an unterminated raw string or block comment
/// — keeps the scanner in that state to EOF and drops the tail, a
/// shape no walked file can be in). Raw strings `r"…"`/`r#"…"#` (and
/// the `br` byte form) keep their contents and may span newlines; a
/// `'` that does not close a char literal within the escape/scalar
/// bound is a lifetime and passes through.
///
/// Shared with the other test modules (`errors_tests.rs` pins its wiring
/// markers over comment-stripped sources the same way).
pub(crate) fn strip_comments(source: &str) -> String {
    /// The scanner state. `Block` carries the nesting depth, `Raw` the
    /// number of `#`s that must follow the closing quote.
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Code,
        Line,
        Block(usize),
        Str,
        Raw(usize),
    }

    let bytes = source.as_bytes();
    let len = bytes.len();
    // The output is assembled byte-wise and converted at the end: byte
    // pushes never panic on multi-byte UTF-8 (which only ever reaches
    // the catch-all arms, since every state delimiter is ASCII), and a
    // malformed-escape split of a scalar degrades to a lossy
    // replacement instead of a panic.
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut state = State::Code;
    let mut i = 0;

    /// A byte that may appear inside an identifier — a literal prefix
    /// (`r`, `br`) is only a prefix when the previous byte is not one
    /// of these, so `result` never starts a raw string.
    fn is_identifier_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_'
    }

    while i < len {
        match state {
            State::Code => {
                match bytes[i] {
                    b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                        state = State::Line;
                        i += 2;
                    }
                    b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                        state = State::Block(1);
                        i += 2;
                    }
                    b'"' => {
                        out.push(b'"');
                        state = State::Str;
                        i += 1;
                    }
                    b'\'' => {
                        // A char literal is `'` + one escape (`\n`,
                        // `\xNN`, `\u{…}` — at most 11 content bytes) or
                        // one scalar (at most 4 bytes) + `'`. A `'` whose
                        // closing partner is farther away is a lifetime
                        // and stays code — the bound keeps `<'a>(… &'a`
                        // from being swallowed as one giant "literal".
                        let closing = bytes[i + 1..]
                            .iter()
                            .position(|byte| *byte == b'\'')
                            .map(|position| i + 1 + position);
                        let is_char_literal = match closing {
                            Some(j) if bytes[i + 1] == b'\\' => j - i <= 12,
                            Some(j) => j - i <= 5,
                            None => false,
                        };
                        if let Some(j) = closing.filter(|_| is_char_literal) {
                            out.extend_from_slice(&bytes[i..=j]);
                            i = j + 1;
                        } else {
                            out.push(b'\'');
                            i += 1;
                        }
                    }
                    b'r' | b'b' if i == 0 || !is_identifier_byte(bytes[i - 1]) => {
                        // A raw-string prefix at an identifier boundary:
                        // `r` (or `br`) followed by zero or more `#` and a
                        // `"`. `b"…"` alone is an escaped byte string —
                        // the quote itself enters `Str` on its own match
                        // arm, so only the raw forms take this path.
                        let mut j = i + if bytes[i] == b'b' { 1 } else { 0 };
                        let starts_with_r = j < len && bytes[j] == b'r';
                        if starts_with_r {
                            j += 1;
                            let mut hashes = 0usize;
                            while j < len && bytes[j] == b'#' {
                                hashes += 1;
                                j += 1;
                            }
                            if j < len && bytes[j] == b'"' {
                                out.extend_from_slice(&bytes[i..=j]);
                                state = State::Raw(hashes);
                                i = j + 1;
                                continue;
                            }
                        }
                        // Not a raw literal after all — plain identifier
                        // text; one byte passes through and scanning
                        // continues in `Code`.
                        out.push(bytes[i]);
                        i += 1;
                    }
                    _ => {
                        // Any other byte — including multi-byte UTF-8,
                        // whose continuation bytes never equal an ASCII
                        // delimiter — passes through as live text.
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            State::Line => {
                if bytes[i] == b'\n' {
                    out.push(b'\n');
                    state = State::Code;
                }
                i += 1;
            }
            State::Block(depth) => {
                if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    state = State::Block(depth + 1);
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    state = if depth > 1 {
                        State::Block(depth - 1)
                    } else {
                        State::Code
                    };
                    i += 2;
                } else {
                    i += 1;
                }
            }
            State::Str => {
                match bytes[i] {
                    b'\\' if i + 1 < len => {
                        out.extend_from_slice(&bytes[i..i + 2]);
                        i += 2;
                    }
                    b'"' => {
                        out.push(b'"');
                        state = State::Code;
                        i += 1;
                    }
                    b'\n' => {
                        // A raw newline cannot appear inside a normal
                        // string literal; ending the literal state here
                        // keeps the scanner from ever swallowing the rest
                        // of the file on malformed input.
                        out.push(b'\n');
                        state = State::Code;
                        i += 1;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            State::Raw(hashes) => {
                // The literal ends at `"` followed by exactly `hashes`
                // `#`s; any other byte (newlines included) is content.
                let close_end = i + hashes;
                if bytes[i] == b'"'
                    && close_end < len
                    && (1..=hashes).all(|offset| bytes[i + offset] == b'#')
                {
                    out.extend_from_slice(&bytes[i..=close_end]);
                    state = State::Code;
                    i = close_end + 1;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn a_url_literal_does_not_blind_comment_stripping() {
    // Arrange & Act: the exact P2-1 shape — live code on the same line
    // after a `//` INSIDE a string literal, plus live text after a
    // `/*`-inside-a-string literal.
    let source = r#"let url = "https://api.example.com"; ureq::get(url);
let pattern = "**/*.md";
network_code_after_the_glob();
// a real comment
let live = "state://project-availability"; more_live_code();"#;

    // Assert: every live fragment stays visible — the old
    // delimiter-only stripper cut the first line at `https:` (dropping
    // `ureq::get(url);`) and treated the glob's `/*` as an unterminated
    // block comment, silently dropping the REST OF THE FILE; both made
    // the network-isolation pin blind while staying green.
    let stripped = strip_comments(source);
    assert!(
        stripped.contains("ureq::get(url);"),
        "live code after a `//` inside a string literal must stay visible: {stripped}"
    );
    assert!(
        stripped.contains("network_code_after_the_glob();"),
        "live code after a `/*` inside a string literal must stay visible (the old \
         stripper dropped the whole rest of the file here): {stripped}"
    );
    assert!(
        stripped.contains("more_live_code();"),
        "live code after a scheme-style string literal must stay visible: {stripped}"
    );
    assert!(
        stripped.contains("https://api.example.com"),
        "string literal contents are kept verbatim: {stripped}"
    );
}

#[test]
fn comments_are_still_stripped_including_nested_blocks() {
    // Arrange & Act: every comment form the pins rely on being removed.
    let source = "// ureq\nlet a = 1; /// doc ureq\n//! module ureq\n/* block ureq */ let b = 2;\nlet c = 3; /* outer /* nested */ still outer */ let d = 4;";

    // Assert: none of the needle-like text inside comments survives;
    // the code between them does; nested block comments close at their
    // real end, not at the inner `*/` (the old stripper ended early and
    // leaked ` still outer */` as visible text).
    let stripped = strip_comments(source);
    assert!(
        !stripped.contains("ureq"),
        "comment text must not survive stripping: {stripped}"
    );
    for live in ["let a = 1;", "let b = 2;", "let c = 3;", "let d = 4;"] {
        assert!(
            stripped.contains(live),
            "live code must survive stripping: {stripped}"
        );
    }
    assert!(
        !stripped.contains("still outer"),
        "the nested block's tail is comment text (Rust nests block comments) and \
         must not leak as live text: {stripped}"
    );
}

#[test]
fn raw_and_escaped_literals_keep_their_contents() {
    // Arrange & Act: raw strings (with and without hashes, spanning a
    // newline), escaped quotes, char literals and lifetimes — none of
    // these may be mangled, and none may blind the scanner.
    let source = r##"let doc = r#"an inner /* not a comment */ stays"#;
let re = r"line one
line two // still the string";
let path = "escaped \" quote; ureq::get(url)";
let nl = '\n'; let tab = '\t';
fn borrow<'a>(text: &'a str) -> &'a str { text }"##;

    // Assert: contents intact, code after them visible, lifetimes not
    // eaten by the char-literal logic.
    let stripped = strip_comments(source);
    assert!(
        stripped.contains("an inner /* not a comment */ stays"),
        "raw string contents are kept verbatim: {stripped}"
    );
    assert!(
        stripped.contains("line two // still the string"),
        "a raw string spanning a newline keeps its contents and does not \
         open a line comment: {stripped}"
    );
    assert!(
        stripped.contains("ureq::get(url)"),
        "live code after an escaped-quote string literal stays visible: {stripped}"
    );
    assert!(
        stripped.contains("let nl = '\\n'; let tab = '\\t';"),
        "char literals pass through unchanged: {stripped}"
    );
    assert!(
        stripped.contains("fn borrow<'a>(text: &'a str) -> &'a str { text }"),
        "lifetimes must not be parsed as char literals (a `'a` with no \
         near closing quote is a lifetime): {stripped}"
    );
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
fn updater_config_carries_the_stable_feed_and_a_well_formed_public_key() {
    // Arrange & Act: the real `plugins.updater` value from
    // `tauri.conf.json`, deserialized with the actual config type the
    // plugin initializes from.
    let config = tauri_conf();
    let section = config["plugins"]["updater"].clone();

    // Assert (negative control, kept from D-023): tauri feeds a plugin
    // `Value::Null` when its config section is absent, and the updater
    // refuses that — the companion
    // `updater_plugin_refuses_to_initialize_without_a_config_section`
    // test proves it end-to-end. If a future plugin version starts
    // accepting an absent section, this assertion fails and the section
    // becomes removable: decide consciously, then.
    assert!(
        serde_json::from_value::<tauri_plugin_updater::Config>(serde_json::Value::Null).is_err(),
        "the updater config must keep rejecting an absent `plugins.updater` \
         section; once it accepts one, the section is dead weight and \
         should be removed"
    );

    // Assert: the section deserializes into the real plugin config with
    // the live update feed — UPD-003's embedded public key and exactly
    // one endpoint, the stable channel's (REL-004). The endpoint is
    // pinned against the `updater` module's `STABLE_ENDPOINT` constant
    // (T-133, D-229): the plugin reads the endpoint from the config, the
    // module owns the typed truth the upcoming commands (T-134/T-135)
    // build on, and this pin keeps the two from drifting apart. The
    // config type stores `url::Url`s; stringifying them keeps this test
    // free of any direct `url` dependency.
    let parsed: tauri_plugin_updater::Config = serde_json::from_value(section).expect(
        "`plugins.updater` must deserialize into the plugin config: \
         without a valid section the app fails to start (the plugin \
         config has a required `pubkey` field)",
    );
    let endpoints: Vec<String> = parsed.endpoints.iter().map(|url| url.to_string()).collect();
    assert_eq!(
        endpoints,
        [crate::updater::STABLE_ENDPOINT.to_string()],
        "`plugins.updater` must carry exactly one endpoint — the stable \
         channel's GitHub Releases manifest (REL-004, D-229): a second \
         endpoint or a different URL is a channel/feed change and must \
         land in the `updater` module's constant together with this pin"
    );

    // Assert: the public key is well-formed through the plugin's own
    // decode chain (2.14.1 `verify_signature`: base64 STANDARD decode →
    // UTF-8 string → `minisign_verify::PublicKey::decode`) — the same
    // steps the plugin runs at artifact-verification time, so a key the
    // plugin could never use fails here at test time. The two line-shape
    // checks give the failure a readable cause; `PublicKey::decode` is
    // the oracle (an independent implementation of the minisign format,
    // not a mirror of this app's code).
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&parsed.pubkey)
        .unwrap_or_else(|_| panic!("the updater pubkey must be base64 (the plugin decodes it with base64 STANDARD before `PublicKey::decode`)"));
    let key_text = std::str::from_utf8(&decoded)
        .unwrap_or_else(|_| panic!("the updater pubkey must decode to UTF-8 minisign key text"));
    let lines: Vec<&str> = key_text.lines().collect();
    assert!(
        lines.len() == 2 && lines[0].starts_with("untrusted comment: minisign public key"),
        "the updater pubkey must be the two-line minisign public key the \
         `tauri signer generate` `.pub` file contains (comment line + key \
         line), got {} line(s)",
        lines.len()
    );
    assert!(
        lines[1].starts_with("RW"),
        "the updater pubkey's key line must be a minisign Ed25519 public \
         key (starts with `RW`), got: {}",
        lines[1]
    );
    assert!(
        minisign_verify::PublicKey::decode(key_text).is_ok(),
        "the updater pubkey must decode with minisign-verify — the \
         library the plugin verifies artifacts through (UPD-003)"
    );

    // Assert: the escapes stay off. The `dangerous*` transport flags
    // would weaken the https/TLS posture of the feed (the plugin's
    // `validate_endpoints` already rejects non-https endpoints in
    // release builds; the flags reach further, into certificate
    // validation); `allowDowngrades` removes the only guard against
    // installing an older validly signed release. `requireSignedVersion`
    // (2.14.1's anti-downgrade-pairing hardening) stays at its default
    // false: turning it on is a release-policy decision that belongs to
    // the signing/release tasks (T-229/T-234, D-229's follow-up), not a
    // config drive-by.
    assert!(
        !parsed.dangerous_insecure_transport_protocol
            && !parsed.dangerous_accept_invalid_certs
            && !parsed.dangerous_accept_invalid_hostnames,
        "`plugins.updater` must keep every `dangerous*` transport escape \
         off (NFR-012/SEC-004: the one network use is the https update \
         feed, with normal certificate validation)"
    );
    assert!(
        !parsed.allow_downgrades && !parsed.require_signed_version,
        "`plugins.updater` must keep `allowDowngrades` off (it removes the \
         only guard against installing an older validly signed release) \
         and `requireSignedVersion` at its default: both are release-task \
         signing-policy decisions (T-229/T-234), not config edits"
    );
}

#[test]
fn updater_plugin_refuses_to_initialize_without_a_config_section() {
    // Arrange: the mock runtime (the same `tauri::test` harness the IPC
    // tests use) with a context that carries no `plugins.updater`
    // section — what tauri would hand the plugin if the section were
    // removed from tauri.conf.json.
    let result = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(tauri::test::mock_context(tauri::test::noop_assets()));

    // Assert: plugin initialization (and with it app startup) fails, and
    // the failure is attributable to the updater plugin config — proving
    // the `plugins.updater` section itself is required, not decoration
    // (D-023 kept the negative control when its empty stub became the
    // real key-and-feed section; round-2 P3-2 removed the stale "stub"
    // wording).
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

#[test]
fn windows_manifest_keeps_the_common_controls_dependency() {
    // Arrange & Act: the manifest `build.rs` embeds, compile-time text.
    let manifest = windows_app_manifest();

    // Assert: `WindowsAttributes::app_manifest` replaces tauri-build's
    // default manifest wholesale, and that default carries exactly one
    // entry — the Common-Controls v6 dependency the tauri-build docs
    // require for dialog APIs (tauri-plugin-dialog is wired in
    // `lib.rs`, T-007) — so the custom manifest must reproduce it
    // verbatim. The whole `assemblyIdentity` is pinned attribute by
    // attribute: `processorArchitecture="*"` in particular is what
    // keeps the dependency valid on both Windows x64 and arm64
    // (NFR-001), and a drift there would break the dialog plugin on one
    // architecture with every other test green.
    let identity = [
        "type=\"win32\"",
        "name=\"Microsoft.Windows.Common-Controls\"",
        "version=\"6.0.0.0\"",
        "processorArchitecture=\"*\"",
        "publicKeyToken=\"6595b64144ccf1df\"",
        "language=\"*\"",
    ];
    for attribute in identity {
        assert!(
            manifest.contains(attribute),
            "the custom Windows manifest must reproduce tauri-build's \
             default Common-Controls v6 dependency verbatim; `{attribute}` \
             is missing: the dialog plugin needs the dependency and the \
             custom manifest replaces the default wholesale"
        );
    }
}

#[test]
fn windows_manifest_has_the_required_structure() {
    // Arrange & Act: the manifest `build.rs` embeds, compile-time text.
    let manifest = windows_app_manifest();

    // Assert: the pins are textual, so the document-level facts must be
    // pinned too: no XML declaration (tauri-winres' RC embedding puts a
    // space before the first line's content, and the declaration must be
    // the very first bytes of the document — the file deliberately ships
    // without one, like tauri-build's own default), at most a leading
    // comment before the asm.v1 root with `manifestVersion="1.0"`, and
    // the nesting order of the two settings blocks — a
    // `<ws2016:longPathAware>` or `<requestedExecutionLevel>` moved
    // outside its parent silently disables NFR-023 / SEC-012 while every
    // substring pin stays green.
    assert!(
        !manifest.contains("<?xml"),
        "the Windows manifest must not carry an XML declaration: the \
         RC embedding prepends a space and the declaration is required \
         to be the first bytes of the document"
    );
    let first_content = manifest
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .expect("the manifest should not be empty");
    assert!(
        first_content.starts_with("<!--") || first_content.starts_with("<assembly"),
        "the Windows manifest must start with (at most) a comment and \
         then the assembly root, got: {first_content}"
    );
    assert!(
        manifest.contains("manifestVersion=\"1.0\""),
        "the Windows manifest must declare manifestVersion=\"1.0\""
    );
    let position = |marker: &str| {
        manifest
            .find(marker)
            .unwrap_or_else(|| panic!("the manifest must contain {marker}"))
    };
    let root = position("<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\"");
    assert!(
        root < position("<dependency")
            && root < position("<trustInfo")
            && root < position("<application"),
        "the asm.v1 assembly root must precede every other element: \
         anything else first means the manifest structure was broken"
    );
    assert!(
        position("<trustInfo") < position("<requestedPrivileges")
            && position("<requestedPrivileges") < position("<requestedExecutionLevel"),
        "requestedExecutionLevel must stay nested inside \
         trustInfo/security/requestedPrivileges: moved out, the \
         no-elevation requirement (SEC-012 / NFR-011) silently stops \
         holding"
    );
    assert!(
        position("<application") < position("<windowsSettings")
            && position("<windowsSettings") < position("<ws2016:longPathAware"),
        "longPathAware must stay nested inside \
         application/windowsSettings: moved out, the long-path setting \
         (NFR-023) silently stops applying"
    );
}

#[test]
fn windows_manifest_never_requests_elevation() {
    // Arrange & Act: the manifest `build.rs` embeds, compile-time text.
    let manifest = windows_app_manifest();

    // Assert: SEC-012 / NFR-011 — the app runs with user rights and does
    // not ask for elevation. Exactly one `requestedExecutionLevel` must
    // exist, pinned as the whole element so a changed `level` or
    // `uiAccess` value fails the test; every elevated or
    // elevation-adjacent spelling is banned outright.
    assert_eq!(
        manifest.matches("<requestedExecutionLevel").count(),
        1,
        "the Windows manifest must declare exactly one \
         requestedExecutionLevel: a second one could override or \
         duplicate the execution-level declaration (SEC-012 / NFR-011)"
    );
    assert!(
        manifest.contains("<requestedExecutionLevel level=\"asInvoker\" uiAccess=\"false\" />"),
        "the Windows manifest must declare requestedExecutionLevel \
         level=\"asInvoker\" uiAccess=\"false\" so the app never asks \
         for UAC elevation (SEC-012 / NFR-011)"
    );
    for banned in [
        "requireAdministrator",
        "highestAvailable",
        "uiAccess=\"true\"",
        "autoElevate",
    ] {
        assert!(
            !manifest.contains(banned),
            "the Windows manifest must not request or enable elevation \
             (`{banned}` found): the app runs with user rights \
             (SEC-012 / NFR-011)"
        );
    }
}

#[test]
fn windows_manifest_enables_long_path_support() {
    // Arrange & Act: the manifest `build.rs` embeds, compile-time text.
    let manifest = windows_app_manifest();

    // Assert: NFR-023 — long path support is turned on through the
    // manifest. `longPathAware` is only recognized inside the SMI/2016
    // settings namespace, so the pin covers the namespace binding and
    // the flag itself. Honest limit of this pin and the manifest flag:
    // the OS side must also have the LongPathsEnabled policy on (an
    // environment fact, off by default on Windows 10/11; spec §15 tests
    // long paths on Windows), and the flag's runtime effect is verified
    // only by the manual smoke run (the CI build matrix compiles and
    // links the manifest but never runs the binary), not by these pins.
    assert!(
        manifest.contains("http://schemas.microsoft.com/SMI/2016/WindowsSettings"),
        "the Windows manifest must bind the SMI/2016 settings \
         namespace: longPathAware is only recognized inside it \
         (NFR-023)"
    );
    assert!(
        manifest.contains("<ws2016:longPathAware>true</ws2016:longPathAware>"),
        "the Windows manifest must enable longPathAware so project \
         paths longer than 260 characters are accepted (NFR-023); \
         flipping it to false, removing it or renaming the element \
         must fail here"
    );
}

#[test]
fn build_script_wires_the_windows_manifest() {
    // Arrange & Act: `build.rs`, compile-time text.
    let build_script = include_str!("../build.rs");

    // Assert: the manifest must actually be embedded — the
    // `WindowsAttributes` wiring, reading the file by its exact path
    // (a moved or renamed file fails `windows_app_manifest` at compile
    // time, so the two `include_str!`s together keep path and file in
    // sync) — and edits to the file must re-run the build script:
    // tauri-build emits its own narrower rerun-if-changed set (config
    // files, resources, icons) that does not cover the manifest, and
    // `include_str!` is opaque to Cargo, so a missing directive would
    // mean a silently stale manifest.
    assert!(
        build_script.contains("windows_attributes("),
        "build.rs must pass WindowsAttributes to tauri_build: without \
         the wiring the custom manifest is never embedded (NFR-023, \
         SEC-012)"
    );
    assert!(
        build_script.contains("include_str!(\"windows/app.manifest\")"),
        "build.rs must embed the manifest from windows/app.manifest \
         verbatim (NFR-023, SEC-012)"
    );
    assert!(
        build_script.contains("cargo:rerun-if-changed=windows/app.manifest"),
        "build.rs must emit cargo:rerun-if-changed for the manifest \
         file: tauri-build's rerun set does not cover it, and without \
         the directive manifest edits would not rebuild the binary"
    );
    assert_eq!(
        build_script.matches(".app_manifest(").count(),
        1,
        "build.rs must contain exactly one .app_manifest( call — the \
         Windows application manifest; a missing call drops the \
         NFR-023/SEC-012 settings, a second call would be the banned \
         ACL manifest (D-024)"
    );
}

#[test]
fn every_workspace_member_inherits_the_lint_gate() {
    // Arrange: the root manifest declares the print_stderr/print_stdout
    // deny pair and the member list; each member manifest must opt in
    // through `[lints] workspace = true` (T-275, the D-143 follow-up).
    // Reading the CURRENT member list from the root manifest is the
    // point: cargo's inheritance is opt-in per manifest, so a future
    // crate added to the workspace without the stanza would silently
    // escape the gate — this pin turns that red (review round 1, P3-3).
    let root_manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml");
    let root_text =
        std::fs::read_to_string(&root_manifest).expect("the root manifest must be readable");
    let root: toml::Value = toml::from_str(&root_text).expect("the root manifest must parse");

    // Act & Assert, part 1: the workspace-level gate itself must stay —
    // both lints, both deny (the structural encoding of the D-032/D-129
    // Windows analysis: a GUI release build has no stderr, and an
    // `eprintln!` panic under `panic = "abort"` aborts the process).
    let clippy_lints = root
        .get("workspace")
        .and_then(|workspace| workspace.get("lints"))
        .and_then(|lints| lints.get("clippy"))
        .expect("the root manifest must define [workspace.lints.clippy]");
    for lint in ["print_stderr", "print_stdout"] {
        assert_eq!(
            clippy_lints.get(lint).and_then(|level| level.as_str()),
            Some("deny"),
            "`{lint}` must stay `deny` in [workspace.lints.clippy]: the \
             workspace-wide gate (D-143, T-275) keeps production code \
             free of `eprintln!`/`println!` — logs go through the \
             logging facade only"
        );
    }

    // Act & Assert, part 2: every member the root lists must inherit.
    // The floor keeps the loop non-vacuous (a parse accident or a
    // silently shrunken member list must not read as compliance).
    let members = root
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(|members| members.as_array())
        .expect("the root manifest must list its workspace members");
    assert!(
        members.len() >= 5,
        "the member list read from the root manifest must cover the \
         whole workspace (five members as of T-275), not a parse \
         accident"
    );
    let member_dir = |path: &str| -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(path)
    };
    let mut parsed: Vec<(&str, toml::Value)> = Vec::new();
    for member in members {
        let path = member
            .as_str()
            .expect("every workspace member must be a path string");
        let manifest_path = member_dir(path).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
            panic!("reading `{}` must work: {error}", manifest_path.display())
        });
        let manifest: toml::Value = toml::from_str(&text)
            .unwrap_or_else(|error| panic!("`{}` must parse: {error}", manifest_path.display()));
        assert_eq!(
            manifest
                .get("lints")
                .and_then(|lints| lints.get("workspace"))
                .and_then(|workspace| workspace.as_bool()),
            Some(true),
            "`{path}` must inherit the workspace lints (`[lints] workspace \
             = true` in its Cargo.toml): the print_stderr/print_stdout \
             deny is workspace-wide, and a member without the stanza \
             would silently escape the gate (D-143, T-275)"
        );
        parsed.push((path, manifest));
    }

    // Act & Assert, part 3: cargo auto-adds every path dependency that
    // lies inside the workspace root as an implicit member — such a
    // crate never appears in the `members` array, so the explicit-list
    // loop above cannot see it and it, too, must carry the stanza
    // (round-2 review, P3-3). The dependency tables of every parsed
    // manifest — direct, dev, build, and each `target.'cfg'` arm — are
    // walked; dep targets outside the root (none exist today) and the
    // workspace root itself are not auto-members.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("the workspace root must canonicalize");
    let mut implicit_members: Vec<std::path::PathBuf> = Vec::new();
    for (member_path, manifest) in &parsed {
        let member_dir = member_dir(member_path);
        for deps in dependency_tables(manifest) {
            collect_root_path_deps(deps, &member_dir, &workspace_root, &mut implicit_members);
        }
    }
    let explicit: Vec<std::path::PathBuf> = parsed
        .iter()
        .map(|(path, _)| member_dir(path).canonicalize().expect("member dirs exist"))
        .collect();
    for member in implicit_members
        .iter()
        .filter(|path| !explicit.contains(path))
    {
        let manifest_path = member.join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
            panic!("reading `{}` must work: {error}", manifest_path.display())
        });
        let manifest: toml::Value = toml::from_str(&text)
            .unwrap_or_else(|error| panic!("`{}` must parse: {error}", manifest_path.display()));
        assert_eq!(
            manifest
                .get("lints")
                .and_then(|lints| lints.get("workspace"))
                .and_then(|workspace| workspace.as_bool()),
            Some(true),
            "`{}` is an implicit workspace member (a path dependency \
             inside the workspace root, cargo's auto-membership rule) \
             and must inherit the workspace lints (`[lints] workspace = \
             true` in its Cargo.toml): without the stanza it silently \
             escapes the print_stderr/print_stdout gate (D-143, T-275; \
             round-2 review P3-3)",
            manifest_path.display()
        );
    }
}

/// The dependency tables of one manifest: the direct, dev and build
/// sets, plus the same three under every `target.'cfg'` arm.
fn dependency_tables(manifest: &toml::Value) -> Vec<&toml::Table> {
    const DEP_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut tables = Vec::new();
    for name in DEP_TABLES {
        if let Some(deps) = manifest.get(name).and_then(|deps| deps.as_table()) {
            tables.push(deps);
        }
    }
    if let Some(targets) = manifest
        .get("target")
        .and_then(|targets| targets.as_table())
    {
        for platform in targets.values() {
            for name in DEP_TABLES {
                if let Some(deps) = platform.get(name).and_then(|deps| deps.as_table()) {
                    tables.push(deps);
                }
            }
        }
    }
    tables
}

/// Collects every path dependency of `deps` that resolves strictly
/// inside `workspace_root` (cargo's implicit-membership rule) into
/// `found`. Canonicalized, deduplicated by the caller's filter.
fn collect_root_path_deps(
    deps: &toml::Table,
    member_dir: &std::path::Path,
    workspace_root: &std::path::Path,
    found: &mut Vec<std::path::PathBuf>,
) {
    for (name, spec) in deps {
        let Some(path) = spec.get("path").and_then(|path| path.as_str()) else {
            continue;
        };
        let resolved = member_dir
            .join(path)
            .canonicalize()
            .unwrap_or_else(|error| {
                panic!(
                    "the path dependency `{name}` (`{path}`) must resolve: \
                     {error}"
                )
            });
        if resolved != workspace_root && resolved.starts_with(workspace_root) {
            found.push(resolved);
        }
    }
}
