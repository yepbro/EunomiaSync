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
//!   `tauri.conf.json` (`tauri::generate_context!()` — the same context
//!   the app runs with) and exercises the plugin through its public
//!   `UpdaterExt` seam: `updater()` returning `Ok` is precisely the
//!   observable that was `Err(EmptyEndpoints)` under the D-023 stub.
//!   It proves the plugin initialized from the real config and found
//!   the endpoints — the network itself (the manifest GET) is the
//!   plugin's internal behavior and is not exercised here; no test in
//!   this suite touches the network (NFR-012).
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
    // `tauri::generate_context!()` compiles in the same
    // `tauri.conf.json` the app runs with (the same macro `run()` uses,
    // so the config cannot drift from what these tests see), and
    // `mock_builder()` is tauri's documented pairing for it. The plugin
    // is registered exactly as `lib.rs` registers it.
    //
    // Act: the plugin's public seam. `updater()` builds the `Updater`
    // from the state the plugin's initialization managed; under the
    // D-023 stub it failed with `EmptyEndpoints`.
    let updater = mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(tauri::generate_context!())
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
