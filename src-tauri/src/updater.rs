//! Update feed (UPD-003, UPD-006, NFR-012/NFR-013, SEC-004, REL-004;
//! DECISIONS.md D-005, D-023, D-229): the app's one and only network
//! surface, and the module boundary that owns it.
//!
//! What runs where (NFR-012/NFR-013, SEC-004): the HTTP client itself
//! lives *inside* `tauri-plugin-updater` 2.14.1 — `Updater::check` builds
//! a `reqwest` client and GETs the manifest, `Update::download`/`install`
//! fetch and verify the artifact (`updater.rs` of the plugin: the client
//! is constructed in `check`, verification in `verify_signature`). The
//! app's own code never touches a socket or a client crate (pinned by
//! `config_tests.rs`'s network-code scan). The plugin is reached from
//! here — `UpdaterExt::updater()` on the app handle — so every future
//! network-touching call site (`update_check`/`update_install`, T-134/
//! T-135) lands in this module, which is what NFR-013's "both run in a
//! separate module" pins. The webview keeps no updater permission
//! (capabilities/default.json, D-024) and no network CSP source
//! (SEC-001: network belongs to Rust).
//!
//! What the request carries (NFR-013): nothing of the app's. The check
//! is a GET of the fixed manifest URL below; the plugin would expand
//! `{{current_version}}`/`{{target}}`/`{{arch}}`/`{{bundle_type}}`
//! placeholders (in the literal and the percent-encoded spelling) if
//! the URL had them (plugin `updater.rs` `check`), but this URL has
//! none — no query parameters, no body, no project or instruction
//! data in either request. The plugin sends only fixed transport
//! metadata of its own (its `Accept` headers and its
//! `tauri-plugin-updater/<version>` user-agent); nothing about this
//! app or its data enters either request.
//!
//! The channel (REL-004): there is exactly one, `stable`. The plugin has
//! no channel mechanism of its own (2.14.1 `Config` has no channel
//! field; the `RemoteRelease` manifest has no `channels` key — verified
//! against the installed sources), so "one channel" is expressed the
//! only way the plugin supports: exactly one endpoint — the stable
//! channel's manifest URL — and nothing that could ever select a second
//! feed. [`UpdateChannel`] is the typed form of that fact; the endpoint
//! itself is carried by `tauri.conf.json` `plugins > updater` (the
//! plugin reads it there at initialization), and this module's
//! [`STABLE_ENDPOINT`] is the single source of truth that the config is
//! pinned to (`updater_tests.rs`).
//!
//! The public key (UPD-003): `tauri.conf.json`'s `pubkey` is the
//! base64-encoded two-line minisign public key (the exact content of
//! the `.key.pub` file `tauri signer generate` writes) — the format the
//! plugin decodes at verification time (`verify_signature`: base64 →
//! string → `minisign_verify::PublicKey::decode`). Artifact signatures
//! cannot be turned off, and no configuration switch skips the check —
//! a valid pubkey in the config is what makes it pass. The exact
//! ordering, stated for the future `update_install` implementer:
//! `Update::install` verifies the bytes before installing them;
//! `Update::download` verifies the fully assembled buffer before
//! returning it — but the download progress callback observes chunks
//! BEFORE verification, so callback bytes must be treated as
//! unverified until the buffer passes (round-1 P3-3). The current key
//! is dev-grade, passwordless, and replaceable by the release task
//! without compatibility constraints (nothing is signed yet —
//! DECISIONS.md D-229 records the ruling and the swap path; the
//! private key lives outside the repository, SEC-011).
//!
//! This card wires the plugin, the key, the endpoint and the channel;
//! the confirmation flow and the sync gate of UPD-003, plus the
//! startup check and the About window integration (UPD-001/UPD-002/
//! UPD-005), are the upcoming command tasks that land here.

/// The app's single update channel (REL-004: "There is one update
/// channel (stable)").
///
/// One variant on purpose: the channel set is closed at one, and the
/// type says so. The plugin's 2.14.1 API has no channel mechanism (see
/// the module docs), so this type is the app-side truth — the About
/// line's channel name (UPD-005) and any future channel-aware logic
/// read it from here instead of a bare string.
#[allow(dead_code)]
// constructed only by tests today; UPD-005's About line and T-134's check result are the future production consumers (the sync_phase2.rs/phase4.rs future-card precedent)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateChannel {
    /// The one channel this app ships: `stable` (REL-004). Its feed is
    /// [`STABLE_ENDPOINT`].
    Stable,
}

impl UpdateChannel {
    /// The channel's name as spec text spells it (UPD-005's "Update
    /// channel: stable" About line; REL-004's channel table row).
    #[allow(dead_code)] // UPD-005's About line and T-134's check result are future consumers; pinned by updater_tests.rs today
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
        }
    }
}

/// The stable channel's update-manifest endpoint (REL-004: repository,
/// releases and updates live at `https://github.com/yepbro/EunomiaSync`).
///
/// `releases/latest/download/latest.json` is GitHub's stable redirect to
/// the newest non-prerelease release's `latest.json` asset — the
/// manifest the T-234 release workflow will publish. The URL carries no
/// placeholders, so the request the plugin sends is exactly this string
/// (no version, target, or arch leaks into it — see the module docs).
///
/// `tauri.conf.json`'s `plugins > updater > endpoints` must list exactly
/// this URL (the plugin reads its endpoints from there, not from here);
/// `config_tests.rs` pins the two against each other, so they cannot
/// drift apart.
#[allow(dead_code)] // the plugin reads the endpoint from tauri.conf.json; production readers (T-134/T-135 diagnostics, the About line) are future cards — today this constant is the module-owned truth the config pin checks against
pub(crate) const STABLE_ENDPOINT: &str =
    "https://github.com/yepbro/EunomiaSync/releases/latest/download/latest.json";
