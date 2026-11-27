//! Tests for the update-feed module (T-133): the endpoint/channel truth
//! UPD-003 and REL-004 fix, and the end-to-end proof that the updater
//! initializes from the real configuration — the `EmptyEndpoints`
//! failure mode D-023 accepted is gone now that the config carries the
//! stable feed.
//!
//! Honest boundary, stated per test:
//!
//! - The endpoint/constant tests pin literals the spec fixes (REL-004's
//!   repository URL, the `stable` channel name) — an independent oracle
//!   (the spec text), not a mirror of the module.
//! - The end-to-end test builds a mock-runtime app from the *real*
//!   `tauri.conf.json` (`tauri::generate_context!(test = true)` —
//!   the same config the app runs with) and exercises the plugin
//!   through its public `UpdaterExt` seam: `updater()` returning
//!   `Ok` is precisely the observable that was `Err(EmptyEndpoints)`
//!   under the D-023 stub. It proves the plugin initialized from
//!   the real config and found the endpoints — the network itself
//!   (the manifest GET) is the plugin's internal behavior and is
//!   not exercised here; no test in this suite touches the network
//!   (NFR-012).
//!
//!   The `test = true` flag is the macro's documented escape for a
//!   context created inside a test binary (tauri-macros 2.7.5,
//!   `src/lib.rs`): it skips exactly one piece of code generation —
//!   the `Info.plist` embedding every plain `generate_context!()`
//!   emits on macOS development builds (tauri-codegen 2.7.5,
//!   `src/context.rs`: `embed_plist::embed_info_plist!`, a
//!   `#[no_mangle] _EMBED_INFO_PLIST` static). The lib test target
//!   also compiles `run()`'s plain expansion (`lib.rs`), and two
//!   embeddings in one binary are a duplicate symbol — CI run
//!   46046771869 failed both macOS legs on exactly that. Linux and
//!   Windows hosts generate no plist block at all (the codegen arm
//!   is `#[cfg(target_os = "macos")]`), which is why only macOS
//!   collided. The structural pin
//!   [`the_lib_test_target_has_one_plist_embedding_expansion`]
//!   keeps the one-expansion invariant true on every platform.
//!
//! The config-shape pins (pubkey well-formedness, the exact endpoint
//! list, the `dangerous*` escapes) live in `config_tests.rs` beside the
//! other `tauri.conf.json` pins, which own the config-reading helpers.

use tauri::test::mock_builder;
use tauri_plugin_updater::UpdaterExt;

use crate::updater::{STABLE_ENDPOINT, UpdateChannel};

/// The repository host and transport REL-004 fixes: https (the plugin's
/// endpoint validation rejects non-https in release builds) on
/// GitHub's release host.
const EXPECTED_ENDPOINT_PREFIX: &str = "https://github.com";

/// The stable manifest path under the REL-004 repository: GitHub's
/// `releases/latest/download/latest.json` redirect to the newest
/// non-prerelease release's manifest asset.
const EXPECTED_ENDPOINT_SUFFIX: &str = "/yepbro/EunomiaSync/releases/latest/download/latest.json";

#[test]
fn the_stable_endpoint_is_the_rel004_repository_release_feed() {
    // Arrange & Act: the module's endpoint constant.
    let endpoint = STABLE_ENDPOINT;

    // Assert: pinned against the spec text (REL-004), piece by piece —
    // scheme and host (https + github.com, the plugin's transport
    // requirement and the repo's host), the repository, and GitHub's
    // stable-redirect path to the latest release's manifest. A host,
    // repository or path drift (the endpoint moved, a channel suffix
    // appeared) turns this red.
    assert_eq!(
        endpoint,
        format!("{EXPECTED_ENDPOINT_PREFIX}{EXPECTED_ENDPOINT_SUFFIX}"),
        "the stable endpoint must be REL-004's repository release feed: \
         https host, the yepbro/EunomiaSync repository, GitHub's \
         latest-release manifest redirect"
    );
}

#[test]
fn the_endpoint_carries_no_request_shaping() {
    // Arrange & Act: the module's endpoint constant.
    //
    // Assert: NFR-013 — neither request contains project or instruction
    // data. The request is a plain GET of this URL; the plugin would
    // only append data through its `{{current_version}}`/`{{target}}`/
    // `{{arch}}`/`{{bundle_type}}` placeholders (in BOTH spellings —
    // literal and the percent-encoded `%7B%7B…%7D%7D` form the plugin
    // substitutes first, 2.14.1 `check`) or a query string. The URL
    // must carry neither: no placeholders in either spelling, no query,
    // no fragment. A future "helpful" tracking parameter added to the
    // URL turns this red.
    assert!(
        !STABLE_ENDPOINT.contains("{{"),
        "the endpoint must not carry plugin placeholders: the request \
         URL is sent verbatim (NFR-013)"
    );
    let encoded = STABLE_ENDPOINT.to_ascii_lowercase();
    assert!(
        !encoded.contains("%7b%7b") && !encoded.contains("%7d%7d"),
        "the endpoint must not carry the percent-encoded placeholder \
         spellings the plugin substitutes (`%7B%7Btarget%7D%7D` etc.): \
         the request URL is sent verbatim (NFR-013, round-1 P3-2)"
    );
    assert!(
        !STABLE_ENDPOINT.contains('?') && !STABLE_ENDPOINT.contains('#'),
        "the endpoint must carry no query or fragment: nothing of the \
         app's may enter the request (NFR-013)"
    );
}

#[test]
fn the_only_channel_is_stable() {
    // Arrange & Act: the channel type — a closed enum with one variant
    // by construction; `as_str` is the spec's spelling of it.
    //
    // Assert: REL-004 — one update channel, named `stable` (UPD-005's
    // About line spells it the same way). Two halves, stated honestly:
    // the enum having exactly one variant is a compile-time shape (a
    // second variant cannot appear without editing this file's imports
    // and the match inside `as_str` — a conscious diff this test
    // reviews), and this assertion pins the runtime half (the name).
    // The set staying CLOSED at one is not provable by an in-crate
    // assertion over the enum itself (any such check over the enum's
    // own variants is true by construction); its real guards are the
    // single-endpoint config pin in `config_tests.rs` and review —
    // stated here so nobody trusts a vacuous check later
    // (round-1 P3-1).
    assert_eq!(
        UpdateChannel::Stable.as_str(),
        "stable",
        "the channel name must be REL-004's `stable` (UPD-005's About \
         line shows the same string)"
    );
}

#[test]
fn the_updater_initializes_from_the_real_config() {
    // Arrange: the mock runtime carrying the REAL app context —
    // `tauri::generate_context!(test = true)` compiles in the same
    // `tauri.conf.json` the app runs with (the same macro `run()`
    // uses, so the config cannot drift from what these tests see),
    // and `mock_builder()` is tauri's documented pairing for it.
    // The plugin is registered exactly as `lib.rs` registers it.
    // The `test = true` flag changes exactly one thing (verified:
    // its only consumer in tauri-codegen 2.7.5 is the plist-block
    // gate): the macOS dev-build `Info.plist` embedding is skipped
    // — mandatory here, because the lib test target also compiles
    // `run()`'s unflagged expansion and two embeddings are a
    // duplicate `_EMBED_INFO_PLIST` symbol (CI run 46046771869,
    // both macOS legs; Linux/Windows generate no plist block, so
    // only macOS collided). The config itself — endpoints, key,
    // channel — is the real one, unchanged by the flag.
    //
    // Act: the plugin's public seam. `updater()` builds the `Updater`
    // from the state the plugin's initialization managed; under the
    // D-023 stub it failed with `EmptyEndpoints`.
    let updater = mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(tauri::generate_context!(test = true))
        .expect("the app must initialize with the real tauri.conf.json")
        .updater();

    // Assert: the updater builds — the endpoints reached the plugin
    // (this is the exact call that returned `Err(EmptyEndpoints)` while
    // `plugins > updater > endpoints` was the empty stub). The plugin
    // initializes from the real config, and the D-023 failure mode is
    // gone. Honest boundary: this proves initialization and endpoint
    // resolution, not a network round-trip — no test here contacts the
    // feed (NFR-012); the manifest GET is exercised only by the manual
    // release smoke once releases exist (T-234).
    assert!(
        updater.is_ok(),
        "the updater must build from the real config: `updater()` \
         failing here means the endpoints did not reach the plugin \
         (under the D-023 stub this was `EmptyEndpoints`), got: {:?}",
        updater.err().map(|error| error.to_string())
    );
}

// ---- the one-expansion structural pin --------------------------------

/// The single live `generate_context!` invocation the crate's binaries
/// may carry UNFLAGGED: `run()`'s context seam in `src-tauri/src/lib.rs`
/// — the production expansion, which on a macOS dev build is the one
/// that embeds the `Info.plist`. Matched by the full in-repo path (the
/// logging scans' rule: a bare `lib.rs` match would also exempt a
/// future `crates/<crate>/src/lib.rs`, which cannot have this seam).
const PRODUCTION_CONTEXT_SEAM: &str = "src-tauri/src/lib.rs";

/// The flagged spelling every OTHER invocation must use (the macro's
/// documented `test` argument, tauri-macros 2.7.5 `src/lib.rs`). Built
/// by `concat!` so this file's own source text does not carry the
/// needle: the scanner keeps string-literal contents, so a literal
/// needle here would count itself.
const TEST_FLAGGED_INVOCATION: &str = concat!("generate_context", "!(test = true)");

/// The plain invocation needle the pin counts, `concat!`-built for the
/// same self-counting reason.
const CONTEXT_INVOCATION: &str = concat!("generate_context", "!");

/// One `generate_context!` per binary is a tauri constraint, not a
/// style preference: every plain expansion on a macOS dev build emits
/// `embed_plist`'s `#[no_mangle] static _EMBED_INFO_PLIST`
/// (`embed_plist-1.2.2/src/lib.rs` — the crate's documented "accidental
/// reuse protection ... works even if these macros are reused in
/// different modules"), and a second unflagged expansion ANYWHERE in
/// the same binary is a duplicate symbol. The lib test target compiles
/// `lib.rs` (with `run()`'s unflagged expansion) plus every `cfg(test)`
/// module — CI run 46046771869 failed BOTH macOS legs exactly there
/// (T-133's end-to-end test above added the second expansion; Linux and
/// Windows legs never saw it because the codegen's plist arm is
/// `#[cfg(target_os = "macos")]` on the HOST). This pin turns that
/// macOS-only link error into a red test on EVERY platform: lib.rs
/// keeps exactly ONE unflagged invocation, every other file's
/// invocations are all the `test = true` spelling. Mutation-verified
/// both ways: reverting the e2e test's flag turns this red, and so does
/// planting a second plain invocation in any scanned source.
#[test]
fn the_lib_test_target_has_one_plist_embedding_expansion() {
    // Arrange & Act: every workspace `.rs` source with comments
    // stripped (the sibling scans' precision — prose explaining this
    // very pin lives in the module docs above and must not trip it,
    // while an actual invocation is live code). String-literal
    // contents stay visible (the stripper's safe direction:
    // over-detection, a red pin, a conscious edit).
    let files = crate::config_tests::workspace_rust_sources();
    assert!(
        !files.is_empty(),
        "the source walker must find the workspace sources"
    );
    for file in &files {
        let text = std::fs::read_to_string(file).expect("a listed source file must be readable");
        let stripped = crate::config_tests::strip_comments(&text);
        let invocations = stripped.matches(CONTEXT_INVOCATION).count();
        let flagged = stripped.matches(TEST_FLAGGED_INVOCATION).count();
        let path = file.display().to_string();
        if file.ends_with(PRODUCTION_CONTEXT_SEAM) {
            // The production seam: exactly one invocation, unflagged —
            // a second one (or flagging this one, which would strip
            // the app's dev-build `Info.plist` on macOS to dodge a
            // collision) is a red pin and a conscious edit.
            assert_eq!(
                invocations, 1,
                "`{path}` is the crate's one production context-macro \
                 seam: a second live invocation in the lib duplicates \
                 `_EMBED_INFO_PLIST` on macOS dev builds (CI run \
                 46046771869) — the seam stays exactly one; if it \
                 must move, that is this pin's conscious edit"
            );
            assert_eq!(
                flagged, 0,
                "`{path}` must keep the REAL embedding: `test = true` \
                 on the production seam strips the app's dev-build \
                 `Info.plist` on macOS — the flag belongs to test-side \
                 expansions only"
            );
        } else {
            // Everywhere else — the `*_tests.rs` modules especially,
            // which compile into the lib test target beside `run()`'s
            // expansion: every invocation must be the flagged
            // spelling, and the counts must be EQUAL — each flagged
            // spelling contains the plain needle, so the counts agree
            // only when every plain occurrence is a flagged one.
            assert_eq!(
                invocations, flagged,
                "`{path}` must spell every `generate_context` macro \
                 invocation as `{TEST_FLAGGED_INVOCATION}`: an \
                 unflagged expansion here compiles a second \
                 `_EMBED_INFO_PLIST` into the lib test target beside \
                 `run()`'s, which is the duplicate symbol that failed \
                 both macOS legs of CI run 46046771869 — if this is \
                 test-side code, add the flag; if you believe you need \
                 a second PRODUCTION expansion, one binary still holds \
                 only one embedding (the bin test target compiles the \
                 lib's seam too), so route it through `lib.rs`'s \
                 single seam instead of flagging production code"
            );
        }
    }
}
