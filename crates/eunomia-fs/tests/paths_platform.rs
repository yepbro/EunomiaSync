//! Platform matrix tests for project-path identity (task T-070,
//! docs/spec.md 12.1/15.3, NFR-001…003): the DB-030/DB-031 behavior
//! of `paths` exercised against each platform's REAL filesystem —
//! the facts the pure rule tests inside `src/paths.rs` cannot see.
//!
//! Groups:
//!
//! * common — every CI leg runs it, Linux locally included: a folder
//!   created with an NFD (decomposed Unicode) name survives the
//!   canonicalize round-trip byte for byte. That is the stored-path
//!   half of DB-030's owner ruling (NFC and folding belong to the KEY
//!   only); the inline `paths` test pins it on the Unix legs, this
//!   file adds the Windows/NTFS leg.
//! * case-insensitive volumes (`cfg(any(windows, target_os =
//!   "macos"))`): NTFS (§15.3's Windows row) and the default APFS
//!   (§15.3's macOS row) resolve every case-variant spelling of one
//!   folder to the STORED spelling, so all spellings carry one
//!   `path_key` (the class-level consequence — one identity — is
//!   pinned end-to-end by `scan_platform`'s macOS group), and a
//!   case-variant file name resolves to the on-disk case.
//! * Windows only: a junction aliases its target to one project
//!   identity (DB-031's Windows flavor of the symlink rule), and a
//!   path beyond the 260-character MAX_PATH bound keeps its verbatim
//!   `\\?\` prefix in the canonical form (dunce's safety rule,
//!   DECISIONS.md D-086) while still keying cleanly.
//! * macOS only: NFD and NFC spellings of one name are ONE folder
//!   (APFS compares names by canonical equivalence): the NFC spelling
//!   resolves to the stored NFD form — §15.3's "NFD names" row,
//!   keeping a decomposed folder name from multiplying project
//!   identities.
//!
//! The Windows and macOS groups are compile-checked by the
//! cross-target runs (`cargo check -p eunomia-fs --all-targets
//! --target x86_64-pc-windows-msvc` / `aarch64-apple-darwin`, the
//! D-099 precedent) and executed by the matching CI matrix legs; the
//! Linux leg runs the common group plus the Linux/Unix groups of the
//! sibling platform files.

mod common;

use std::fs;

use eunomia_fs::paths::canonicalize;
use tempfile::TempDir;

#[test]
fn an_nfd_named_folder_canonicalizes_verbatim_on_every_filesystem() {
    // §15.3's macOS "NFD names" row has a platform-independent half:
    // the canonical form the app STORES must carry the bytes the
    // filesystem holds (DB-030's reading A), whatever the platform
    // does to name comparison. NTFS, APFS and Linux filesystems all
    // preserve the stored bytes, so the round-trip is byte-exact on
    // every leg.
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    // "cafe" + COMBINING ACUTE ACCENT — the NFD spelling of café,
    // decomposed by hand, never composed by us:
    let decomposed = "caf\u{65}\u{301}";
    let folder = dir.path().join(decomposed);
    fs::create_dir(&folder).expect("creating the fixture folder must work");

    let canonical = canonicalize(&folder).expect("the fixture folder must canonicalize");
    let stored = canonical
        .file_name()
        .expect("a canonical path to a folder has a file name");
    // `as_encoded_bytes` is the platform's native OsStr encoding; for
    // this name (plain valid Unicode, no lone surrogates) it is UTF-8
    // on all three platforms, so the byte comparison is exact.
    assert_eq!(
        stored.as_encoded_bytes(),
        decomposed.as_bytes(),
        "DB-030's stored path is the verbatim filesystem spelling: the \
         decomposed bytes must come back decomposed on every platform"
    );
}

/// §15.3's "case-insensitive NTFS" (Windows) and "case-insensitive
/// APFS" (macOS) rows, run on the volumes that actually fold: every
/// case-variant spelling of one folder is ONE project.
#[cfg(any(windows, target_os = "macos"))]
mod case_insensitive_volume {
    use super::*;

    use eunomia_fs::paths::canonical_path_key;

    #[test]
    fn case_variant_spellings_resolve_to_the_stored_spelling_and_one_key() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = dir.path().join("MyProject");
        fs::create_dir_all(repo.join(".git")).expect("creating the fixture repository must work");

        // Every case variant resolves to the SAME canonical form, and
        // the form carries the folder's stored spelling ("MyProject"),
        // not the spelling asked with:
        let stored = canonicalize(&repo).expect("the on-disk spelling must canonicalize");
        for variant in ["MYPROJECT", "myproject", "mYpRoJeCt"] {
            let resolved = canonicalize(&dir.path().join(variant))
                .expect("a case-variant spelling must resolve to the same folder");
            assert_eq!(
                resolved, stored,
                "the {variant} spelling resolves to the stored folder"
            );
        }
        assert_eq!(
            stored
                .file_name()
                .expect("a folder path has a file name")
                .to_string_lossy(),
            "MyProject",
            "the canonical form restores the on-disk case"
        );

        // One key for every spelling (DB-030's folding rule keyed on a
        // real case-insensitive volume): the key of a variant is the
        // key of the stored spelling — one identity, so the project
        // cannot be added twice.
        let key = canonical_path_key(&repo).expect("the stored spelling must key");
        for variant in ["MYPROJECT", "myproject", "mYpRoJeCt"] {
            assert_eq!(
                canonical_path_key(&dir.path().join(variant))
                    .expect("a case-variant spelling must key"),
                key,
                "the {variant} spelling carries the one identity"
            );
        }
    }

    #[test]
    fn resolve_target_path_restores_the_on_disk_case_of_the_file_name() {
        use eunomia_fs::paths::resolve_target_path;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture root must work");
        fs::write(root.join("AGENTS.md"), b"stored spelling")
            .expect("writing the fixture file must work");

        let target = resolve_target_path(&root, "agents.md")
            .expect("a case-variant file name must resolve on a case-insensitive volume");
        assert_eq!(
            target.file_name(),
            "AGENTS.md",
            "the resolved name is the stored spelling — the staging grammar \
             and the sweep key on it"
        );
        assert_eq!(
            target.path(),
            canonicalize(&root)
                .expect("the root must canonicalize")
                .join("AGENTS.md")
        );
    }
}

/// §15.3's Windows "junctions" row: the reparse-point flavor of
/// DB-031's aliasing rule, on a real junction.
#[cfg(windows)]
mod windows_junction {
    use super::*;

    use eunomia_fs::paths::canonical_path_key;

    #[test]
    fn a_junction_and_its_target_are_one_project_identity() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("real-repo");
        fs::create_dir_all(target.join(".git")).expect("creating the fixture repository must work");
        let junction = dir.path().join("linked-repo");
        common::make_junction(&junction, &target);

        // DB-031's Windows flavor: the junction resolves THROUGH to
        // the target — one canonical form, one key, one project:
        let target_key = canonical_path_key(&target).expect("the target spelling must key");
        assert_eq!(
            canonical_path_key(&junction).expect("the junction spelling must canonicalize"),
            target_key,
            "a folder and a junction to it are one project (DB-031)"
        );
        // And the resolution is real, not textual: the canonical form
        // of the junction spelling is the TARGET's path.
        assert_eq!(
            canonicalize(&junction).expect("the junction must resolve through"),
            canonicalize(&target).expect("the target must canonicalize")
        );
    }
}

/// §15.3's Windows "long paths" row (NFR-023): beyond the
/// 260-character MAX_PATH bound.
#[cfg(windows)]
mod windows_long_path {
    use super::*;

    use common::{VERBATIM_PREFIX, deep_verbatim};
    use eunomia_fs::paths::canonical_path_key;

    #[test]
    fn a_long_path_keeps_its_verbatim_prefix_and_keys_cleanly() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let repo = deep_verbatim(dir.path(), 7).join("Repository");
        fs::create_dir_all(repo.join(".git")).expect("creating the deep fixture must work");
        let simplified = repo
            .to_string_lossy()
            .trim_start_matches(VERBATIM_PREFIX)
            .len();
        assert!(
            simplified > 260,
            "precondition: the fixture must exceed MAX_PATH even simplified, \
             got {simplified} characters"
        );

        // D-086: dunce strips the verbatim prefix only where Win32
        // could open the stripped spelling — past MAX_PATH it cannot,
        // so the STORED canonical form keeps the prefix.
        let canonical = canonicalize(&repo).expect("the deep folder must canonicalize");
        let text = canonical.to_string_lossy();
        assert!(
            text.starts_with(VERBATIM_PREFIX),
            "the canonical form keeps the verbatim prefix past MAX_PATH, got: {text}"
        );

        // The key still applies its full rule to the verbatim form
        // (D-086's pinned `//?/…` shape): `/` separators, folded case,
        // no trailing separator.
        let key = canonical_path_key(&repo).expect("the deep folder must key");
        assert!(
            key.starts_with("//?/"),
            "the verbatim prefix keys in the pinned D-086 shape: {key}"
        );
        assert!(
            !key.contains('\\'),
            "the Windows rule rewrites every separator to '/': {key}"
        );
        assert!(
            key.ends_with("/repository"),
            "the folded final component rides the long key: {key}"
        );
    }
}

/// §15.3's macOS "NFD names" row: the filesystem-level half. APFS
/// compares names by canonical equivalence while preserving the
/// stored bytes, so the two spellings of one name are ONE folder.
#[cfg(target_os = "macos")]
mod macos_normalization_insensitive_apfs {
    use super::*;

    #[test]
    fn nfd_and_nfc_spellings_are_one_folder_on_apfs() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        // The two spellings of café: precomposed and decomposed.
        let nfc = "caf\u{e9}";
        let nfd = "caf\u{65}\u{301}";
        assert_ne!(nfc, nfd);
        let stored = dir.path().join(nfd);
        fs::create_dir(&stored).expect("creating the fixture folder must work");

        // APFS resolves the NFC spelling to the stored NFD folder...
        let canonical_nfc = canonicalize(&dir.path().join(nfc))
            .expect("the NFC spelling must resolve to the stored NFD folder");
        let canonical_nfd = canonicalize(&stored).expect("the NFD spelling must canonicalize");
        assert_eq!(
            canonical_nfc, canonical_nfd,
            "the two spellings are one folder on APFS"
        );
        // ...and answers with the STORED bytes, not the spelling asked
        // with (DB-030's reading A on a normalization-insensitive
        // volume). One canonical form is one `path_key` by
        // construction — the app keys the canonical form, so the
        // decomposed name cannot multiply identities.
        assert_eq!(
            canonical_nfc
                .file_name()
                .expect("a folder path has a file name")
                .as_encoded_bytes(),
            nfd.as_bytes(),
            "the canonical form carries the stored (decomposed) bytes"
        );
    }
}
