//! Platform matrix tests for the write path (task T-070,
//! docs/spec.md 9.4/9.5, 12.1/15.3, NFR-001…003): the atomic-write
//! pipeline and its preflights over each platform's real filesystem
//! semantics.
//!
//! Groups:
//!
//! * common — every leg: a CRLF document round-trips byte for byte
//!   (§15.3's Windows "CRLF in repositories" row: whatever the
//!   platform, no layer of the pipeline rewrites line endings).
//! * Unix (the Linux and macOS legs; Linux runs it locally): the T-F6
//!   chain — resolve a symlinked target, gate it with the write-rights
//!   preflight, then write through the resolved path with the link
//!   kept (the write-through half re-pins the inline atomic suite; the
//!   added value is the chained gate and the whole-folder debris
//!   oracle); a read-only-marked target is stopped by the PREFLIGHT
//!   (T-F5's gate) while the raw rename would have landed — the
//!   division of labor the `write_access` docs state; writing into a
//!   read-only folder fails at the staging create with EACCES on the
//!   §10 log line; and the written file takes the setgid folder's
//!   group (§15.3's Linux "rights and owners" row).
//! * Windows: §15.3's "write rights in protected folders" row,
//!   arranged as an ACL write denial on an owned folder (the Windows
//!   analogue of the Unix 0o555 fixture) — the gate refuses with
//!   ERROR_ACCESS_DENIED and a write that skips the gate cannot stage;
//!   and a target beyond the 260-character MAX_PATH bound is written
//!   and verified in place (NFR-023's long-path support at the fs
//!   layer; the pipeline works through the verbatim spelling
//!   regardless of the app manifest's longPathAware flag).
//!
//! §15.3's Windows "locked files" row is T-F8; its criterion test
//! lives in the atomic module's inline suite (the D-099 three-level
//! ladder), not here. The read-only FILE ATTRIBUTE gate is likewise
//! pinned inline (`a_read_only_target_file_is_no_write_access_t_f5`,
//! on every leg). Unix symlink flavors are pinned inline; the Windows
//! link flavor (junctions) is covered in the sibling platform files.
//! The Windows group compiles on the other targets through the
//! cross-target checks (the D-099 precedent) and runs on the CI
//! Windows leg.

#![allow(clippy::print_stderr)] // test skip-notices and benchmark timings print deliberately

mod common;

use std::fs;
use std::path::Path;

use eunomia_core::hash::Sha256Hex;
use eunomia_fs::atomic::write_target_file;
use eunomia_fs::free_space::FreeSpaceError;
use tempfile::TempDir;

/// A free-space probe that always reports plenty (the D-089 seam:
/// these tests exercise the filesystem, not the volume).
fn plenty_free() -> impl FnOnce() -> Result<u64, FreeSpaceError> {
    || Ok(1 << 40)
}

/// The FIPS digest of `bytes`, in the crate's canonical spelling —
/// computed here the way the assertions want it, never taken from the
/// code under test.
fn sha256_hex_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256Hex::new();
    hasher.update(bytes);
    hasher.finalize_hex()
}

/// The sorted entry names of `folder` — the "what is on disk" oracle.
fn entry_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("reading the fixture folder must work")
        .map(|entry| {
            entry
                .expect("iterating the fixture folder must work")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn a_crlf_document_round_trips_byte_for_byte() {
    // §15.3's Windows row names CRLF in repositories; the platform
    // contract behind it is byte-exactness: the pipeline writes what
    // it was handed, so the row is pinned on every leg.
    let dir = TempDir::new().expect("a temporary directory must be creatable");
    let target = dir.path().join("AGENTS.md");
    fs::write(&target, b"first line\r\nsecond line\r\n")
        .expect("writing the fixture file must work");
    let replacement = b"- one\r\n- two\r\n- three\r\n";

    let written = write_target_file(&target, replacement, plenty_free())
        .expect("writing the CRLF document must succeed");

    assert_eq!(
        written.size,
        u64::try_from(replacement.len()).expect("a slice length fits u64")
    );
    assert_eq!(written.sha256_hex, sha256_hex_of(replacement));
    assert_eq!(
        &fs::read(&target).expect("reading back the document must work"),
        replacement,
        "no layer of the pipeline may rewrite line endings"
    );
    assert_eq!(
        entry_names(dir.path()),
        vec!["AGENTS.md".to_owned()],
        "a successful write leaves no staging debris"
    );
}

/// §15.3's Linux "rights and owners" row plus the Unix symlink rows,
/// through the real §9.4/§9.5 preflights and pipeline.
#[cfg(unix)]
mod unix_rights_and_links {
    use super::*;

    use eunomia_fs::atomic::{AtomicWriteError, WriteStep};
    use eunomia_fs::paths::resolve_target_path;
    use eunomia_fs::write_access::{WriteAccessError, check_write_access};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[test]
    fn the_sync_chain_resolves_gates_then_writes_through_a_symlinked_target() {
        // T-F6 at the fs integration level: the pipeline's own order —
        // resolve first (§9.5 step 1), gate with the write-rights
        // preflight (§9.4 step 3), then write over the RESOLVED path
        // (§9.5 step 4). The resolve and write halves are each pinned
        // by the inline suites; the chain itself — the gate answering
        // for the RESOLVED path between them, and the whole-folder
        // oracle — is what this test adds.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture root must work");
        fs::write(root.join("AGENTS.md"), b"original\n")
            .expect("writing the real target must work");
        std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md"))
            .expect("creating the fixture link must work");

        let resolved =
            resolve_target_path(&root, "CLAUDE.md").expect("the link target must resolve");
        assert_eq!(resolved.file_name(), "AGENTS.md");
        check_write_access(resolved.path()).expect("the real file behind the link is writable");
        write_target_file(resolved.path(), b"updated body\n", plenty_free())
            .expect("writing through the resolved path must succeed");

        // The write went to the real file, the link survived it, and
        // both spellings read the new content.
        assert_eq!(
            fs::read(root.join("AGENTS.md")).expect("reading the real file must work"),
            b"updated body\n"
        );
        assert_eq!(
            fs::read(root.join("CLAUDE.md")).expect("reading through the link must work"),
            b"updated body\n"
        );
        assert!(
            fs::symlink_metadata(root.join("CLAUDE.md"))
                .expect("the link must still be statted")
                .file_type()
                .is_symlink(),
            "the link itself must survive the write"
        );
        assert_eq!(
            entry_names(&root),
            vec!["AGENTS.md".to_owned(), "CLAUDE.md".to_owned()],
            "the chain leaves exactly the two spellings — no staging debris"
        );
    }

    #[test]
    fn a_read_only_target_is_gated_by_the_preflight_not_by_the_rename() {
        // The `write_access` docs' posture, pinned on a real
        // filesystem: on Unix a rename CAN replace a write-disabled
        // file — T-F5's protection is the preflight gate, not the OS.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"marked read-only\n").expect("writing the fixture file must work");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o444))
            .expect("marking the fixture read-only must work");

        // The gate honors the user's read-only mark (T-F5)...
        let refusal = check_write_access(&target)
            .expect_err("a read-only target must be refused by the preflight");
        assert!(
            matches!(refusal, WriteAccessError::FileReadOnly { .. }),
            "the preflight verdict is the read-only mark, got: {refusal:?}"
        );
        // ...while the raw pipeline does not consult the target's mode
        // bits and lands the replace:
        write_target_file(&target, b"replaced anyway\n", plenty_free())
            .expect("the §9.5 rename is not stopped by the file's own mode bits");
        assert_eq!(
            fs::read(&target).expect("reading the replaced file must work"),
            b"replaced anyway\n"
        );
        // §9.5 step 2's mode copy keeps the mark: the next preflight
        // gates again.
        let mode = fs::metadata(&target)
            .expect("the replaced file must be statted")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o444,
            "the read-only mark survives the replace"
        );
    }

    #[test]
    fn writing_into_a_read_only_folder_fails_at_the_staging_create_with_eacces() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture root must work");
        let target = root.join("AGENTS.md");
        fs::write(&target, b"original\n").expect("writing the fixture file must work");
        // Root-run mode bits are not enforced, so the read-only-folder
        // precondition cannot be set up (the house skip pattern).
        if running_as_root(&root) {
            eprintln!(
                "skipping writing_into_a_read_only_folder_fails_at_the_staging_create_\
                 with_eacces: running as root — mode bits are not enforced, the \
                 read-only-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&root, fs::Permissions::from_mode(0o555))
            .expect("removing the folder's write bits must work");
        let gate = check_write_access(&target);
        let write = write_target_file(&target, b"denied\n", plenty_free());
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755))
            .expect("restoring the folder's mode bits must work");

        // The pipeline's own order: the preflight gate refuses first
        // (T-F5's §10 fact)...
        let refusal = gate.expect_err("a target inside a read-only folder must be refused");
        assert!(
            matches!(refusal, WriteAccessError::FolderNotWritable { .. }),
            "the preflight verdict names the folder's rights, got: {refusal:?}"
        );
        assert_eq!(
            refusal.os_code(),
            Some(13),
            "EACCES is the OS code the §10 log line carries"
        );
        // ...and a write that skips the gate still cannot stage: the
        // staging create is the first step that touches the folder,
        // refused with EACCES, the original untouched, nothing staged.
        let AtomicWriteError::WriteFailed(failed) =
            &write.expect_err("staging inside a read-only folder must be refused")
        else {
            panic!("the refusal must be a step failure, got a different variant");
        };
        assert_eq!(failed.step, WriteStep::TempCreate);
        assert_eq!(failed.source.raw_os_error(), Some(13));
        assert!(failed.cleanup.is_none(), "nothing was ever staged");
        assert_eq!(
            fs::read(&target).expect("the original must survive"),
            b"original\n"
        );
        assert_eq!(entry_names(&root), vec!["AGENTS.md".to_owned()]);
    }

    #[test]
    fn the_written_file_takes_the_setgid_folder_group() {
        // §15.3's Linux "owners" row through the pipeline: the staging
        // file is created in the TARGET'S folder (§9.5 step 2), so a
        // group-shared (setgid) folder's group rides onto the written
        // file — the fact that keeps syncs into shared checkouts
        // group-owned.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("shared");
        fs::create_dir(&root).expect("creating the fixture folder must work");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o2775))
            .expect("setting the setgid bit must work");
        assert_eq!(
            fs::metadata(&root)
                .expect("the fixture folder must be statted")
                .permissions()
                .mode()
                & 0o2000,
            0o2000,
            "precondition: the filesystem must honor the setgid bit"
        );

        let target = root.join("AGENTS.md");
        write_target_file(&target, b"group-shared\n", plenty_free())
            .expect("writing into the setgid folder must succeed");

        let file_gid = fs::metadata(&target)
            .expect("the written file must be statted")
            .gid();
        let dir_gid = fs::metadata(&root)
            .expect("the folder must be statted")
            .gid();
        assert_eq!(
            file_gid, dir_gid,
            "the setgid folder's group rides onto the written file"
        );
    }

    /// The house root probe (the `running_as_root_is_untestable`
    /// idiom of the inline suites): mode-bit fixtures are vacuous
    /// under root.
    fn running_as_root(temp: &Path) -> bool {
        fs::metadata(temp)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }
}

/// §15.3's Windows "write rights in protected folders" and "long
/// paths" rows for the write path (NFR-023).
#[cfg(windows)]
mod windows_rights_and_long_path {
    use super::*;

    use common::{VERBATIM_PREFIX, deep_verbatim};
    use eunomia_fs::atomic::{AtomicWriteError, WriteStep};
    use eunomia_fs::write_access::{WriteAccessError, check_write_access};
    use std::process::Command;

    #[test]
    fn writing_into_a_write_denied_folder_fails_at_the_staging_create_with_access_denied() {
        // The Windows analogue of the Unix 0o555 fixture: an explicit
        // deny ACE for the running user blocks file creation in an
        // OWNED folder — "protected folder" as an arrangeable fact,
        // not a guess about system folders.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let root = dir.path().join("repo");
        fs::create_dir(&root).expect("creating the fixture root must work");
        let target = root.join("AGENTS.md");
        fs::write(&target, b"original\n").expect("writing the fixture file must work");

        let whoami = Command::new("whoami")
            .output()
            .expect("spawning whoami must work");
        assert!(
            whoami.status.success(),
            "whoami must name the running account"
        );
        let account = String::from_utf8(whoami.stdout)
            .expect("the account name must be UTF-8")
            .trim()
            .to_owned();
        assert!(!account.is_empty(), "whoami must not answer empty");

        // Deny create-file/append-data rights on the folder for the
        // running account (deletion still flows from the parent, so
        // the TempDir teardown stays possible).
        let root_text = root.to_string_lossy().into_owned();
        let deny = Command::new("icacls")
            .args([&root_text, "/deny", &format!("{account}:(WD,AD)")])
            .output()
            .expect("spawning icacls must work");
        assert!(
            deny.status.success(),
            "icacls must deny the write rights: {}{}",
            String::from_utf8_lossy(&deny.stdout),
            String::from_utf8_lossy(&deny.stderr)
        );

        let gate = check_write_access(&target);
        let write = write_target_file(&target, b"denied\n", plenty_free());
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        let restore = Command::new("icacls")
            .args([&root_text, "/remove:d", &account])
            .output()
            .expect("spawning icacls must work");
        assert!(
            restore.status.success(),
            "removing the deny ACE must work: {}{}",
            String::from_utf8_lossy(&restore.stdout),
            String::from_utf8_lossy(&restore.stderr)
        );

        // The pipeline's own order, the Unix test's exact shape with
        // the Windows code: the gate refuses with ERROR_ACCESS_DENIED
        // (5)...
        let refusal = gate.expect_err("a target inside a write-denied folder must be refused");
        assert!(
            matches!(refusal, WriteAccessError::FolderNotWritable { .. }),
            "the preflight verdict names the folder's rights, got: {refusal:?}"
        );
        assert_eq!(
            refusal.os_code(),
            Some(5),
            "ERROR_ACCESS_DENIED is the OS code the §10 log line carries"
        );
        // ...and a write that skips the gate cannot stage either.
        let AtomicWriteError::WriteFailed(failed) =
            &write.expect_err("staging inside a write-denied folder must be refused")
        else {
            panic!("the refusal must be a step failure, got a different variant");
        };
        assert_eq!(failed.step, WriteStep::TempCreate);
        assert_eq!(failed.source.raw_os_error(), Some(5));
        assert!(failed.cleanup.is_none(), "nothing was ever staged");
        assert_eq!(
            fs::read(&target).expect("the original must survive"),
            b"original\n"
        );
        assert_eq!(entry_names(&root), vec!["AGENTS.md".to_owned()]);
    }

    #[test]
    fn a_target_beyond_max_path_writes_and_verifies() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let deep = deep_verbatim(dir.path(), 7);
        fs::create_dir_all(&deep).expect("creating the deep fixture folder must work");
        let target = deep.join("AGENTS.md");
        let simplified = target
            .to_string_lossy()
            .trim_start_matches(VERBATIM_PREFIX)
            .len();
        assert!(
            simplified > 260,
            "precondition: the fixture must exceed MAX_PATH even simplified, \
             got {simplified} characters"
        );

        let written = write_target_file(&target, b"beyond max path\n", plenty_free())
            .expect("a verbatim long-path target must write");

        assert_eq!(
            written.path, target,
            "the written report names the verbatim target"
        );
        assert_eq!(
            fs::read(&target).expect("reading back the deep target must work"),
            b"beyond max path\n"
        );
        assert_eq!(
            entry_names(&deep),
            vec!["AGENTS.md".to_owned()],
            "the deep folder holds exactly the written file — no staging debris"
        );
    }
}
