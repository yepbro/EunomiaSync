//! Shared helpers of the T-070 platform matrix suites
//! (`paths_platform.rs`, `scan_platform.rs`, `atomic_platform.rs`).
//!
//! A plain `mod common;` module, not a test target: every helper that
//! more than one of the suites needs lives here once, so the two
//! copies of the junction builder cannot drift apart again (the
//! round-1 review's P1: both copies carried the same broken manual
//! quoting). Every item is `#[cfg(windows)]` — the suites' Linux and
//! macOS groups build their fixtures with plain `std` calls — so on
//! the other targets this module compiles empty.
//!
//! `#![allow(dead_code)]` because each test crate links its own copy
//! and uses a different subset on each target (the atomic suite never
//! builds junctions); an unused-per-crate helper is the intended
//! shape, not a lapsed fixture.

#![allow(dead_code)]

/// The verbatim prefix std hands back and only the OS accepts past
/// MAX_PATH for every spelling.
#[cfg(windows)]
pub const VERBATIM_PREFIX: &str = "\\\\?\\";

/// Creates a directory junction `link` → `target` through the shell's
/// `mklink /J` (junctions need no elevation; the crate's dependency
/// list has no reparse-point API, and the tests need none of one).
///
/// The paths go in as PLAIN arguments — no manual quotes: Rust's
/// Windows argument escaping already quotes a token that needs it,
/// and pre-quoting produces a backslash-escaped `\"` that cmd does
/// not honor, mangling the path (round-1 review's P1).
#[cfg(windows)]
pub fn make_junction(link: &std::path::Path, target: &std::path::Path) {
    use std::process::Command;

    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("spawning cmd for mklink must work");
    assert!(
        output.status.success(),
        "mklink /J must create the junction: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A verbatim (`\\?\`-prefixed) path `levels` components below
/// `base`, each component long enough that even the SIMPLIFIED
/// spelling stays beyond the 260-character MAX_PATH bound.
#[cfg(windows)]
pub fn deep_verbatim(base: &std::path::Path, levels: usize) -> std::path::PathBuf {
    use std::path::PathBuf;

    let mut path = PathBuf::from(format!("{VERBATIM_PREFIX}{}", base.display()));
    for index in 0..levels {
        path.push(format!("deep_component_{index:02}_with_a_long_spelling"));
    }
    path
}
