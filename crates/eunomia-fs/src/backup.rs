//! Backup copies of target files before sync writes: the per-file
//! operation of spec §7.6 (BKP-001, 002, 008, 010, 011) and §9.4
//! phase 2 step 1.
//!
//! One entry point, [`backup_target_file`]: it snapshots the source
//! file (size + SHA-256), compares the digest with the caller-supplied
//! digest of the file's last backup (BKP-010's dedup), and either
//! reports "no new copy — the record references the existing one" or
//! copies the file into the §5.5 layout and verifies the copy before
//! counting it as created. The §9.4 RUN semantics around it (the
//! backup-records row, the `discarded` sweep, the `aborted` /
//! `E_BACKUP_FAILED` mapping) belong to the store and pipeline tasks;
//! this module is the per-file operation they call — no database, no
//! clock, no configuration (the store lookup feeds
//! `last_backup_sha256_hex`, the pipeline resolves the wall clock into
//! [`BackupTime`] once per run and hands it down, D-061's posture).
//!
//! # The layout and the name (SEC-006)
//!
//! The copy lands at
//!
//! ```text
//! <data directory>/backups/<project_id>__<slug>/<file>.<UTC YYYYMMDDTHHMMSSmmmZ>.<hash8>.bak
//! ```
//!
//! and every name component comes from the pure builders of
//! `eunomia-core::backup_name` (spec §7.6, DECISIONS.md D-080): the
//! project folder from the typed [`ProjectId`] and the project name
//! (cleaned into the slug there — the raw name cannot reach a path
//! component), the file name from the typed [`Scheme`] + [`BaseName`]
//! pair, the stamp from the caller's [`BackupTime`] fields, and the
//! `hash8` from the truncation of the full digest this module
//! computed. No free-form string ever shapes a path (SEC-006); a
//! project ID the builders refuse surfaces as
//! [`BackupError::NameFailed`] — the path is not built at all.
//!
//! # The snapshot, the copy, the check (BKP-010)
//!
//! A copy counts as created only after the check passes, so the
//! operation runs three passes over the bytes:
//!
//! 1. **Snapshot** — open the source once, take its size from the
//!    open file's metadata, and stream its bytes through the
//!    incremental SHA-256 of `eunomia-core::hash` ([`Sha256Hex`]; the
//!    file-system layer hashes through core and never names the
//!    `sha2` crate — §5.2's `fs` row lists `core`, D-017). The
//!    snapshot's (size, digest) pair is the expected value for the
//!    check.
//! 2. **Copy** — stream the source into a freshly created copy
//!    (`create_new`: an existing file under the destination name is
//!    never overwritten — a backup is history, and a name collision
//!    means a retried run within the same millisecond, surfaced as
//!    [`BackupError::CopyFailed`]). The copy is `sync_all`-ed before
//!    the check so the verification judges settled bytes. Content
//!    only: the copy's permission bits are not pinned by the BKP
//!    specs, and §5.5 places the whole backups tree under the data
//!    directory, whose `backups` root is created `0700` on Unix
//!    (SEC-007) — traversal past it is impossible for anyone but the
//!    owner, so the leaf mode carries no reach beyond this user.
//!    (`std::fs::copy` is deliberately not used: it silently
//!    overwrites the destination and preserves the source's mode bits,
//!    neither of which the backup semantics want.)
//! 3. **Check** — re-open the copy, compare its size with the
//!    snapshot's size, then re-hash its bytes and compare the digest.
//!    Either mismatch fails the check. On any failure the corrupt
//!    bytes are REMOVED — a file under a verified-backup name that
//!    does not hold verified content must not linger (the card's
//!    acceptance: a substituted damaged copy is caught by the check);
//!    the removal failure itself is carried in
//!    [`VerifyFailed::cleanup`] for the run's `discarded` sweep to
//!    retry. [`verify_backup_copy`] is the check as its own function —
//!    the same removal-on-failure contract — so callers and tests can
//!    drive it directly.
//!
//! # Dedup (BKP-010)
//!
//! If the snapshot digest exactly equals `last_backup_sha256_hex` —
//! the full 64-character lowercase-hex digest of the file's LAST
//! backup, as the caller's store lookup recorded it — no copy is
//! made: not even the folders are created, and
//! [`BackupOutcome::Deduplicated`] tells the caller to point this
//! file's record at the existing copy. The comparison is exact
//! full-digest equality on the canonical lowercase spelling: both
//! sides hash with the same core primitive (`Sha256Hex`), so equal
//! strings mean equal content; an uppercase, truncated or otherwise
//! non-canonical value simply compares unequal and produces a fresh
//! copy. Whether the existing copy is still on disk is the store's
//! record question, not this module's.
//!
//! # Folders
//!
//! The `backups` root and the project's folder are created as needed
//! (BKP-001/002: backups ARE in the data directory — a project's
//! first backup must not fail for a missing folder). The data
//! directory itself must exist (app startup owns it): its absence is
//! [`BackupError::FolderCreateFailed`], not silently repaired. The
//! root is born `0700` on Unix — created in one step with that mode
//! (SEC-007), so no window ever exists in which a permissive root is
//! on disk; an existing root keeps its rights — the module does not
//! fight the owner's choices after the first creation. The leaf copy's
//! own mode is the platform default and deliberately unpinned: the
//! BKP specs pin no leaf rights, and traversal past the `0700` root is
//! impossible for anyone but the owner regardless.
//!
//! # Crash debris and durability posture
//!
//! The copy is written straight to its final §5.5 name (`create_new`):
//! the module's cleanup runs while the process lives, so a kill or
//! power loss mid-copy can leave a partial file under a final backup
//! name. That debris is INERT — no record was inserted for it (the
//! record follows a verified copy), so nothing references it — and its
//! sweeping belongs to the run-level/retention work of the store
//! tasks, the same relationship §9.5's staging files have to
//! `temp_files`. Durability: the copy's bytes are `sync_all`-ed before
//! the check, but the project folder's directory entry is not fsynced
//! (§9.5 pins a directory fsync for the write phase's rename only, and
//! the platform APIs for opening a directory are not portable through
//! std). A crash in the window between copy and record insert can
//! therefore lose the entry — an unreferenced/missing-copy situation
//! the store's restore path already models (§10's `missing` status).
//! The risk is accepted here and recorded in DECISIONS.md D-091.
//!
//! # Races are honest (TOCTOU)
//!
//! The source is a live file; the operation pretends no snapshot
//! atomicity it does not have. Ordering: hash the source, then copy,
//! then re-hash the copy — so a source change between the passes
//! surfaces as a verification mismatch (the copy simply is not the
//! snapshot's bytes), and the corrupt copy is removed. A source
//! changing size mid-hash likewise fails the check against the
//! metadata size taken at open. The one window no post-hoc check can
//! close is a same-size mutation of the source DURING the hash read
//! itself: the snapshot is then internally consistent but may never
//! have been the file's content; the pipeline bounds this (phase 2
//! backs up files phase 1 just read, and the written target is
//! verified separately in §9.5), and the check's threat model is
//! silent copy corruption, not adversarial concurrent mutation.
//! The source file is never modified by this module.
//!
//! # Testing plan
//!
//! Colocated tests over real temporary directories: the happy path
//! against independently computed SHA-256 oracles (FIPS 180-2
//! vectors), the exact builder spelling of the created name, the
//! dedup acceptance (same digest → no copy, no folders; different →
//! a copy), the substituted same-size corrupt copy and the truncated
//! copy caught by [`verify_backup_copy`] and removed, an untouched
//! copy passing the check, the 8 MiB streaming case, the empty file,
//! a missing source, folder creation including the SEC-007 mode, the
//! never-overwrite collision, the hostile project id and hostile
//! project name refused/contained, and the error contract (Display,
//! source chain, OS codes). The N-th-copy failure point of spec 15.1
//! (T-F1's fs-level half) is injected through the crate's
//! `fault_injection` module (`cfg(test)` or the non-default
//! `fault-injection` feature): the production removal and the
//! `CopyFailed` classification run under it.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use eunomia_core::backup_name::{BackupNameError, BackupTime, backup_relative_path};
use eunomia_core::hash::Sha256Hex;
use eunomia_core::project::{BaseName, ProjectId, Scheme};

#[cfg(any(test, feature = "fault-injection"))]
use crate::fault_injection;

/// The `backups` folder of the §5.5 layout, relative to the data
/// directory — the first component of every path the core builders
/// return. Kept here as the one spelling the folder creation uses; a
/// test pins it against the builders' output so the two cannot drift.
const BACKUPS_DIR: &str = "backups";

/// The read/write chunk of the streaming loops: 64 KiB, large enough
/// to keep syscall counts low for library-sized files, small enough
/// that the fixed stack buffer stays modest. No whole file is ever
/// loaded into memory: source hashing, copying and copy verification
/// all move exactly one chunk at a time.
const COPY_CHUNK_BYTES: usize = 64 * 1024;

/// What [`backup_target_file`] decided for one target file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupOutcome {
    /// The source's digest equals the caller-supplied digest of the
    /// file's last backup (BKP-010): no copy was created — not even
    /// the folders — and the caller's record references the existing
    /// copy it took the digest from.
    Deduplicated {
        /// The source file's size in bytes.
        size: u64,
        /// The source file's full SHA-256, 64 lowercase hex characters.
        sha256_hex: String,
    },
    /// A new copy exists under `backups/<project_id>__<slug>/` and
    /// passed the BKP-010 check (size + SHA-256 against the source
    /// snapshot): it counts as created.
    Created(BackupCopy),
}

/// The verified facts of one created backup copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupCopy {
    /// The §5.5 layout path of the copy, relative to the data
    /// directory, spelled by the core builders
    /// (`backups/<project_id>__<slug>/<file>.<UTC>.<hash8>.bak`,
    /// `/` separators — the spec string, not an OS path).
    pub relative_path: String,
    /// The copy's path on disk (the layout joined under the data
    /// directory that was handed in).
    pub path: PathBuf,
    /// The copy's size in bytes — equal to the source's size at
    /// snapshot time (that equality is the check that passed).
    pub size: u64,
    /// The copy's SHA-256 as the full 64-character lowercase hex
    /// digest — equal to the source's digest at snapshot time. The
    /// `hash8` in the name is its first 8 characters (D-080: the
    /// callers get the full digest; the name only truncates).
    pub sha256_hex: String,
}

/// Verifies a backup copy against the expected snapshot facts
/// (BKP-010): the copy's size must equal `expected_size` and its
/// SHA-256 must equal `expected_sha256_hex` — the size and digest
/// [`backup_target_file`] took from the source before copying (and
/// which [`BackupCopy`] carries back).
///
/// **Destructive by contract**: on a mismatch — size, digest, or a
/// copy that cannot be read at all — the file at `copy_path` is
/// REMOVED (a file under a backup name must not linger unverified;
/// see the module docs) and [`BackupError::VerifyFailed`] is returned
/// carrying what was actually measured and whether the removal itself
/// failed. This function is for freshly written backup copies whose
/// removal on failure is wanted; never hand it a path whose bytes
/// must survive a failed comparison. On success the copy stays,
/// untouched.
///
/// # Errors
///
/// [`BackupError::VerifyFailed`] for every check outcome that is not
/// an exact match; it never returns another variant (I/O trouble on
/// the copy is part of the failure data, not a separate error).
pub fn verify_backup_copy(
    copy_path: &Path,
    expected_size: u64,
    expected_sha256_hex: &str,
) -> Result<(), BackupError> {
    let outcome = check_copy_matches(copy_path, expected_size, expected_sha256_hex);
    if outcome.is_ok() {
        return Ok(());
    }
    // The corrupt bytes must not linger under a name that asserts
    // verified content; a vanished copy (raced away) is already
    // converged. `remove_copy` reports a refused removal so the
    // caller's discarded-copies sweep can retry it (BKP-005).
    let cleanup = remove_copy(copy_path);
    Err(BackupError::VerifyFailed(VerifyFailed {
        path: copy_path.to_owned(),
        expected_size,
        expected_sha256_hex: expected_sha256_hex.to_owned(),
        mismatch: outcome.expect_err("the failure branch is taken only on an error"),
        cleanup,
    }))
}

/// The inputs of one backup operation: the resolved source file, the
/// data directory the §5.5 layout joins under, the project/target
/// identity the builders shape into the name (SEC-006), the injected
/// run time (D-061's posture), and the caller's store lookup of the
/// file's last backup digest (BKP-010's dedup input). Grouped in one
/// struct rather than an eight-argument call: the fields are one
/// operation's vocabulary, and no two of them are guessable from a
/// bare position.
pub struct BackupTarget<'a> {
    /// The resolved target file to copy (§9.4 phase 1 step 2 resolves
    /// symlinks before phase 2; this module opens what it is given
    /// and never modifies it).
    pub source: &'a Path,
    /// The app data directory — the layout's `backups/…` prefix is
    /// joined under it.
    pub data_dir: &'a Path,
    /// The project's typed id; the builders gate it against the
    /// path-safe charset.
    pub project_id: &'a ProjectId,
    /// The project's raw name; the builders clean it into the slug,
    /// so a hostile name cannot reach a path component.
    pub project_name: &'a str,
    /// The write scheme of the target (picks the enum file name).
    pub scheme: Scheme,
    /// The target's base name (the other half of the enum file name).
    pub base_name: BaseName,
    /// The run's instant as UTC civil fields (time is a parameter,
    /// never a clock).
    pub time: BackupTime,
    /// The full 64-character lowercase-hex SHA-256 of the file's LAST
    /// backup, exactly as the caller's store recorded it (`None` when
    /// the file has no backup yet).
    pub last_backup_sha256_hex: Option<&'a str>,
}

/// Creates a backup copy of [`BackupTarget::source`] for one project
/// target, per the module docs: snapshot the source (size + SHA-256),
/// return [`BackupOutcome::Deduplicated`] when the digest equals
/// `last_backup_sha256_hex` (the caller's store lookup of the file's
/// last backup — [`None`] when there is none yet), otherwise copy into
/// `<data_dir>/backups/<project_id>__<slug>/…` under the name the
/// core builders spell for the target's identity, time and digest,
/// and verify the copy before counting it as created.
///
/// A hostile project ID fails the call with
/// [`BackupError::NameFailed`] before any path exists (SEC-006); a
/// hostile project NAME is cleaned into the slug by the builders.
///
/// # Errors
///
/// - [`BackupError::NameFailed`]: the builders refused an input (the
///   project-ID gate or the digest shape) — nothing was created.
/// - [`BackupError::FolderCreateFailed`]: the `backups` root or the
///   project folder could not be created (a missing data directory
///   included).
/// - [`BackupError::SourceFailed`]: the source could not be opened,
///   statted or read to the end; nothing was created.
/// - [`BackupError::CopyFailed`]: the copy could not be created
///   (an existing destination is never overwritten), written, or
///   synced; the partial bytes were removed unless even the removal
///   failed (the `cleanup` field of this variant carries the removal
///   error).
/// - [`BackupError::VerifyFailed`]: the copy was written but failed
///   the size/digest check and was removed.
pub fn backup_target_file(target: &BackupTarget<'_>) -> Result<BackupOutcome, BackupError> {
    let BackupTarget {
        source,
        data_dir,
        project_id,
        project_name,
        scheme,
        base_name,
        time,
        last_backup_sha256_hex,
    } = target;

    // Pass 1: the snapshot. The size comes from the open file's own
    // metadata; the digest streams through core's incremental SHA-256.
    let (size, sha256_hex) = hash_source(source)?;

    // Dedup (BKP-010): exact full-digest equality against the caller's
    // last-known copy digest. No folder, no copy, no name.
    if *last_backup_sha256_hex == Some(sha256_hex.as_str()) {
        return Ok(BackupOutcome::Deduplicated { size, sha256_hex });
    }

    // The name: only the builders (SEC-006). Nothing exists on disk
    // until they have accepted every input.
    let relative_path = backup_relative_path(
        project_id,
        project_name,
        *scheme,
        *base_name,
        *time,
        &sha256_hex,
    )
    .map_err(|source| BackupError::NameFailed { source })?;
    let copy_path = data_dir.join(&relative_path);
    ensure_backup_folders(data_dir, &copy_path)?;

    // Pass 2: the copy — fresh file, settled bytes; a partial write
    // removes itself.
    copy_file(source, &copy_path)?;

    // Pass 3: the check (BKP-010). A failed check removes the copy
    // inside `verify_backup_copy`.
    verify_backup_copy(&copy_path, size, &sha256_hex)?;

    Ok(BackupOutcome::Created(BackupCopy {
        relative_path,
        path: copy_path,
        size,
        sha256_hex,
    }))
}

/// Streams `source` through the incremental SHA-256, returning the
/// size taken from the open file's metadata and the full lowercase-hex
/// digest of the bytes actually read.
fn hash_source(source: &Path) -> Result<(u64, String), BackupError> {
    let mut file = File::open(source).map_err(|err| BackupError::SourceFailed {
        path: source.to_owned(),
        source: err,
    })?;
    let expected_size = file
        .metadata()
        .map_err(|err| BackupError::SourceFailed {
            path: source.to_owned(),
            source: err,
        })?
        .len();
    let mut hasher = Sha256Hex::new();
    let mut chunk = [0u8; COPY_CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|err| BackupError::SourceFailed {
                path: source.to_owned(),
                source: err,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok((expected_size, hasher.finalize_hex()))
}

/// Copies `source` to `copy_path` in one streaming pass and syncs the
/// copy to disk. The destination is created with `create_new`: an
/// existing file is never clobbered. On any write-path error the
/// partial copy is removed (a refused removal is reported in the
/// error's `cleanup`).
fn copy_file(source: &Path, copy_path: &Path) -> Result<(), BackupError> {
    let mut reader = File::open(source).map_err(|err| BackupError::SourceFailed {
        path: source.to_owned(),
        source: err,
    })?;
    // Open failure leaves nothing behind: cleanup is None. The most
    // realistic failure here is AlreadyExists — a same-millisecond
    // name collision — and the existing file is someone's real copy.
    let mut writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(copy_path)
        .map_err(|err| BackupError::CopyFailed {
            path: copy_path.to_owned(),
            source: err,
            cleanup: None,
        })?;

    let mut chunk = [0u8; COPY_CHUNK_BYTES];
    // The fault-injection point for spec 15.1's "failure of the
    // N-th backup" row (T-F1's fs-level half; the run-level sweep is
    // the pipeline's): an armed plan refuses the write path of the
    // N-th copy — once per copy, at the pass's start, after the
    // destination exists, so the partial-copy removal and the
    // `CopyFailed` classification below run for real.
    #[cfg(any(test, feature = "fault-injection"))]
    let injected = fault_injection::inject_backup_copy_failure();
    #[cfg(not(any(test, feature = "fault-injection")))]
    let injected: Option<io::Error> = None;
    let write_outcome = match injected {
        Some(source) => Err(source),
        None => loop {
            let read = match reader.read(&mut chunk) {
                Ok(read) => read,
                Err(source) => break Err(source),
            };
            if read == 0 {
                break match writer.sync_all() {
                    Ok(()) => Ok(()),
                    Err(source) => Err(source),
                };
            }
            if let Err(source) = writer.write_all(&chunk[..read]) {
                break Err(source);
            }
        },
    };
    // Close both handles BEFORE removing: on Windows an open handle
    // would make the removal fail with a sharing violation.
    drop(writer);
    drop(reader);

    if let Err(source) = write_outcome {
        let cleanup = remove_copy(copy_path);
        return Err(BackupError::CopyFailed {
            path: copy_path.to_owned(),
            source,
            cleanup,
        });
    }
    Ok(())
}

/// The BKP-010 check over the copy: size first, then the digest of
/// the copy's bytes, each compared with the expected snapshot. The
/// digest is not computed for a size mismatch — the size check alone
/// has already failed.
fn check_copy_matches(
    copy_path: &Path,
    expected_size: u64,
    expected_sha256_hex: &str,
) -> Result<(), VerifyMismatch> {
    let mut copy = File::open(copy_path).map_err(|source| VerifyMismatch::Unreadable { source })?;
    let actual_size = copy
        .metadata()
        .map_err(|source| VerifyMismatch::Unreadable { source })?
        .len();
    if actual_size != expected_size {
        return Err(VerifyMismatch::Size { actual_size });
    }
    let mut hasher = Sha256Hex::new();
    let mut chunk = [0u8; COPY_CHUNK_BYTES];
    loop {
        let read = copy
            .read(&mut chunk)
            .map_err(|source| VerifyMismatch::Unreadable { source })?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    let actual_sha256_hex = hasher.finalize_hex();
    if actual_sha256_hex != expected_sha256_hex {
        return Err(VerifyMismatch::Digest {
            actual_size,
            actual_sha256_hex,
        });
    }
    Ok(())
}

/// Creates the `backups` root (if missing) and the copy's project
/// folder (if missing). The root gets SEC-007's `0700` on Unix when
/// this call creates it; an existing root keeps its rights. The data
/// directory itself must exist — its absence is this typed failure,
/// not silently repaired: the app startup owns that folder, and a
/// backup silently landing somewhere new would hide the loss.
fn ensure_backup_folders(data_dir: &Path, copy_path: &Path) -> Result<(), BackupError> {
    let Some(project_folder) = copy_path.parent() else {
        // Unreachable: `copy_path` is the data directory joined with
        // the builders' `backups/<dir>/<file>` layout, which always
        // has a parent. Handled honestly rather than unwrapped.
        return Err(BackupError::FolderCreateFailed {
            path: copy_path.to_owned(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "the backup path has no parent folder",
            ),
        });
    };
    create_backups_root(&data_dir.join(BACKUPS_DIR))?;
    fs::create_dir_all(project_folder).map_err(|source| BackupError::FolderCreateFailed {
        path: project_folder.to_owned(),
        source,
    })?;
    Ok(())
}

/// Creates the `backups` root in ONE step with SEC-007's `0700` (Unix):
/// the folder is born private, so no window exists in which a
/// permissive root is on disk, and there is no chmod to fail after
/// creation. The OS still applies the umask to the requested mode, but
/// a umask can only REMOVE bits, and `0700` has no group/other bits to
/// remove. An existing root keeps its rights — the module does not
/// fight the owner's choices after the first creation.
#[cfg(unix)]
fn create_backups_root(backups_root: &Path) -> Result<(), BackupError> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new()
        .mode(0o700)
        .create(backups_root)
        .or_else(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists {
                Ok(())
            } else {
                Err(BackupError::FolderCreateFailed {
                    path: backups_root.to_owned(),
                    source,
                })
            }
        })
}

/// Windows variant: rights are inherited from the user profile
/// (SEC-007), so there is nothing to set; the root is created with the
/// platform default or accepted as existing.
#[cfg(not(unix))]
fn create_backups_root(backups_root: &Path) -> Result<(), BackupError> {
    match fs::create_dir(backups_root) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(BackupError::FolderCreateFailed {
            path: backups_root.to_owned(),
            source,
        }),
    }
}

/// Removes a failed copy. A copy that raced away (NotFound) is
/// converged, not a failure; a refused removal is returned so the
/// caller can retry (the run-level discarded sweep, BKP-005).
fn remove_copy(copy_path: &Path) -> Option<io::Error> {
    match fs::remove_file(copy_path) {
        Ok(()) => None,
        Err(source) if source.kind() == io::ErrorKind::NotFound => None,
        Err(source) => Some(source),
    }
}

/// What the BKP-010 check actually saw in the copy, when the copy did
/// not match the expected snapshot.
#[derive(Debug)]
pub enum VerifyMismatch {
    /// The copy's size differs from the snapshot's size (a truncated
    /// or grown copy); the digest was not computed.
    Size {
        /// The copy's size in bytes.
        actual_size: u64,
    },
    /// The copy's size matched but its bytes hash differently —
    /// substituted or corrupted content (the card's damaged-copy
    /// case).
    Digest {
        /// The copy's size in bytes.
        actual_size: u64,
        /// The digest of the copy's bytes, 64 lowercase hex characters.
        actual_sha256_hex: String,
    },
    /// The copy could not be opened, statted or read to the end for
    /// the check at all.
    Unreadable {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
}

impl VerifyMismatch {
    /// The I/O error behind the mismatch, when there is one.
    fn io_error(&self) -> Option<&io::Error> {
        match self {
            VerifyMismatch::Size { .. } | VerifyMismatch::Digest { .. } => None,
            VerifyMismatch::Unreadable { source } => Some(source),
        }
    }
}

/// The failure data of a verification that did not match (BKP-010):
/// the expected snapshot, what the copy actually showed, and whether
/// the corrupt copy was successfully removed.
#[derive(Debug)]
pub struct VerifyFailed {
    /// The copy's path (for the BKP-005 message's backups-folder and
    /// file facts).
    pub path: PathBuf,
    /// The expected size: the source's size at snapshot time.
    pub expected_size: u64,
    /// The expected digest: the source's SHA-256 at snapshot time, 64
    /// lowercase hex characters.
    pub expected_sha256_hex: String,
    /// What the copy actually showed.
    pub mismatch: VerifyMismatch,
    /// `None` when the corrupt copy was removed (or had already
    /// raced away); `Some` with the OS error when it could NOT be
    /// removed and the corrupt bytes are still on disk — the run's
    /// discarded-copies sweep (BKP-005) retries it.
    pub cleanup: Option<io::Error>,
}

/// Why a backup operation failed (typed, hand-rolled per the crate's
/// error posture, D-017/D-084: no error-crate dependency; the §10
/// `E_BACKUP_FAILED` mapping happens in the command layer from these
/// facts).
#[derive(Debug)]
pub enum BackupError {
    /// The core builders refused a backup-name input (the project-ID
    /// gate or the digest shape) — the path was never built (SEC-006:
    /// a path that would not be safe is not created).
    NameFailed {
        /// The builders' typed refusal, with the offending value.
        source: BackupNameError,
    },
    /// The `backups` root or the copy's project folder could not be
    /// created (a missing data directory included — the app startup
    /// owns that folder).
    FolderCreateFailed {
        /// The folder that could not be created.
        path: PathBuf,
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The source file could not be opened, statted or read to the
    /// end for its snapshot. Nothing was created.
    SourceFailed {
        /// The source file's path.
        path: PathBuf,
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The copy could not be created (an existing destination is
    /// never overwritten — a same-millisecond name collision is this
    /// error, not a clobber), written, or synced to disk. The partial
    /// bytes were removed; `cleanup` carries the removal error when
    /// even that failed.
    CopyFailed {
        /// The copy's path.
        path: PathBuf,
        /// The OS failure, carrying the raw error code.
        source: io::Error,
        /// The partial-copy removal outcome: `None` = removed (or
        /// nothing existed yet), `Some` = still on disk.
        cleanup: Option<io::Error>,
    },
    /// The copy was written but failed the size/SHA-256 check against
    /// the source snapshot, and was removed.
    VerifyFailed(VerifyFailed),
}

impl BackupError {
    /// The I/O error behind the failure, for the caller's log line
    /// (`ErrorDetails::from_io_error` in the app layer consumes the
    /// same shape as `paths`/`temp_files` errors).
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            BackupError::NameFailed { .. } => None,
            BackupError::FolderCreateFailed { source, .. }
            | BackupError::SourceFailed { source, .. }
            | BackupError::CopyFailed { source, .. } => Some(source),
            BackupError::VerifyFailed(failed) => failed.mismatch.io_error(),
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`), the §10
    /// log fact behind `E_BACKUP_FAILED`'s "OS code or error type".
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupError::NameFailed { source } => {
                write!(f, "the backup name was refused: {source}")
            }
            BackupError::FolderCreateFailed { path, source } => {
                write!(
                    f,
                    "cannot create the backup folder {}: {source}",
                    path.display()
                )
            }
            BackupError::SourceFailed { path, source } => {
                write!(
                    f,
                    "cannot read the source file {}: {source}",
                    path.display()
                )
            }
            BackupError::CopyFailed {
                path,
                source,
                cleanup,
            } => {
                write!(
                    f,
                    "cannot write the backup copy {}: {source}",
                    path.display()
                )?;
                if let Some(cleanup) = cleanup {
                    write!(f, "; the partial copy could not be removed: {cleanup}")?;
                }
                Ok(())
            }
            BackupError::VerifyFailed(failed) => {
                write!(
                    f,
                    "the backup copy {} failed the check",
                    failed.path.display()
                )?;
                match &failed.mismatch {
                    VerifyMismatch::Size { actual_size } => write!(
                        f,
                        ": {} bytes expected, {actual_size} found",
                        failed.expected_size
                    )?,
                    VerifyMismatch::Digest {
                        actual_sha256_hex, ..
                    } => write!(
                        f,
                        ": sha256 {} expected, {actual_sha256_hex} found",
                        failed.expected_sha256_hex
                    )?,
                    VerifyMismatch::Unreadable { source } => {
                        write!(f, ": the copy could not be read: {source}")?
                    }
                }
                if let Some(cleanup) = &failed.cleanup {
                    write!(f, "; the corrupt copy could not be removed: {cleanup}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for BackupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BackupError::NameFailed { source } => Some(source),
            BackupError::FolderCreateFailed { source, .. }
            | BackupError::SourceFailed { source, .. }
            | BackupError::CopyFailed { source, .. } => Some(source),
            BackupError::VerifyFailed(failed) => failed
                .mismatch
                .io_error()
                .map(|source| source as &(dyn std::error::Error + 'static)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::print_stderr)] // test skip-notices and benchmark timings print deliberately

    use std::error::Error as _;
    use std::path::Component;

    use tempfile::TempDir;

    use super::*;

    /// A canonical UUIDv7-shaped project id (BKP-001's ids).
    fn fixture_id() -> ProjectId {
        ProjectId::new("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d".to_owned())
    }

    /// The injected run time (D-061: time is a parameter).
    fn fixture_time() -> BackupTime {
        BackupTime {
            year: 2026,
            month: 11,
            day: 19,
            hour: 10,
            minute: 30,
            second: 0,
            millisecond: 123,
        }
    }

    /// The FIPS 180-2 vector for "abc" — the independent oracle for
    /// the fixture content (re-checked with Python's hashlib).
    const ABC_DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// The FIPS 180-2 vector for the empty input.
    const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// The source file name in the fixture project folder: arbitrary
    /// on purpose — the backup's name comes from the builders, not
    /// from the source's name (SEC-006).
    const SOURCE_NAME: &str = "AGENTS.md";

    /// Writes the fixture source file and returns its path.
    fn write_source(dir: &Path, content: &[u8]) -> PathBuf {
        let path = dir.join(SOURCE_NAME);
        fs::write(&path, content).expect("writing the fixture source must work");
        path
    }

    /// The one-call shape every test uses: scheme A, AGENTS.md, the
    /// fixture id/name/time.
    fn backup_call(
        source: &Path,
        data_dir: &Path,
        last_backup_sha256_hex: Option<&str>,
    ) -> Result<BackupOutcome, BackupError> {
        backup_target_file(&BackupTarget {
            source,
            data_dir,
            project_id: &fixture_id(),
            project_name: "My Project",
            scheme: Scheme::A,
            base_name: BaseName::AgentsMd,
            time: fixture_time(),
            last_backup_sha256_hex,
        })
    }

    /// The project's backup folder for the fixture id and name.
    fn fixture_backup_folder(data_dir: &Path) -> PathBuf {
        data_dir
            .join(BACKUPS_DIR)
            .join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project")
    }

    /// The fixture's owner uid is this process's effective uid (the
    /// fixture was created by it): uid 0 means the tests run as root,
    /// where mode bits are not enforced — the precondition of a
    /// permission test cannot be set up. The caller skips loudly
    /// instead of passing vacuously (house precedent: `paths.rs`,
    /// `temp_files.rs`).
    #[cfg(unix)]
    fn running_as_root_is_untestable(path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }

    /// The backup file name the builders must spell for the fixture
    /// inputs and the "abc" content (hash8 = the digest's first 8).
    fn expected_file_name(digest_prefix: &str) -> String {
        format!("AGENTS.md.20261119T103000123Z.{digest_prefix}.bak")
    }

    /// Every `.bak` entry in the fixture backup folder, sorted.
    fn backup_listing(data_dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(fixture_backup_folder(data_dir))
            .expect("the backup folder must exist")
            .map(|entry| {
                entry
                    .expect("reading a backup entry must work")
                    .file_name()
                    .to_str()
                    .expect("the backup names are ASCII")
                    .to_owned()
            })
            .collect();
        names.sort();
        names
    }

    // ---- the happy path ------------------------------------------------

    #[test]
    fn a_backup_copy_is_created_with_the_exact_builder_name_and_content() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let outcome = backup_call(&source, &data_dir, None).expect("the backup must be created");

        let BackupOutcome::Created(copy) = outcome else {
            panic!("the first backup must be Created, got: {outcome:?}");
        };
        // The name: the builders' spelling with the injected time and
        // the digest's own hash8 — byte for byte.
        assert_eq!(
            copy.relative_path,
            format!(
                "backups/0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__my-project/{}",
                expected_file_name(&ABC_DIGEST[..8])
            )
        );
        assert_eq!(
            copy.path,
            fixture_backup_folder(&data_dir).join(expected_file_name(&ABC_DIGEST[..8]))
        );
        // The facts: the source's size and its independently computed
        // digest (the FIPS "abc" vector — the module's own hashing
        // did not grade its own homework).
        assert_eq!(copy.size, 3);
        assert_eq!(copy.sha256_hex, ABC_DIGEST);
        // The copy's content is the source's content, byte for byte.
        assert_eq!(
            fs::read(&copy.path).expect("reading the copy must work"),
            b"abc"
        );
        // The source is never modified.
        assert_eq!(
            fs::read(&source).expect("reading the source must work"),
            b"abc"
        );
        // Exactly one copy exists.
        assert_eq!(
            backup_listing(&data_dir),
            vec![expected_file_name(&ABC_DIGEST[..8])]
        );
    }

    #[test]
    fn scheme_c_uses_the_project_md_target_name() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let outcome = backup_target_file(&BackupTarget {
            source: &source,
            data_dir: &data_dir,
            project_id: &fixture_id(),
            project_name: "My Project",
            scheme: Scheme::C,
            base_name: BaseName::ClaudeMd,
            time: fixture_time(),
            last_backup_sha256_hex: None,
        })
        .expect("the backup must be created");

        let BackupOutcome::Created(copy) = outcome else {
            panic!("the first backup must be Created, got: {outcome:?}");
        };
        assert!(
            copy.relative_path
                .ends_with("CLAUDE.project.md.20261119T103000123Z.ba7816bf.bak")
        );
    }

    #[test]
    fn an_empty_file_is_backed_up_with_the_empty_digest() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"");

        let outcome = backup_call(&source, &data_dir, None).expect("the backup must be created");

        let BackupOutcome::Created(copy) = outcome else {
            panic!("the first backup must be Created, got: {outcome:?}");
        };
        assert_eq!(copy.size, 0);
        assert_eq!(copy.sha256_hex, EMPTY_DIGEST);
        assert!(copy.path.exists(), "the zero-byte copy must exist");
        assert_eq!(
            fs::read(&copy.path).expect("reading the copy must work"),
            b""
        );
    }

    #[test]
    fn a_multimegabyte_file_streams_to_the_right_digest() {
        // 8 MiB of a deterministic 64-byte pattern: chunking is on the
        // duty here (the file spans many 64 KiB chunks), and the
        // expected digest is an independent Python-hashlib oracle
        // over the same pattern — an implementation that loaded the
        // file wrong, hashed the wrong range, or dropped chunks
        // cannot match it.
        let block: Vec<u8> = (0..64u8).collect();
        let repeats = 8 * 1024 * 1024 / 64;
        let content: Vec<u8> = block.repeat(repeats);
        let expected_digest = "bd24c5738c4417db37dd0c87ba50a2297835fde4f0a1bc35de38d9ef0064e9d4";

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, &content);

        let outcome = backup_call(&source, &data_dir, None).expect("the backup must be created");

        let BackupOutcome::Created(copy) = outcome else {
            panic!("the first backup must be Created, got: {outcome:?}");
        };
        assert_eq!(copy.size, 8 * 1024 * 1024);
        assert_eq!(copy.sha256_hex, expected_digest);
        assert_eq!(
            fs::read(&copy.path).expect("reading the copy must work"),
            content,
            "the copy must hold every byte of the 8 MiB"
        );
    }

    // ---- dedup (BKP-010) -------------------------------------------------

    #[test]
    fn an_identical_file_does_not_multiply_copies() {
        // The card's acceptance (b): the same content as the last
        // backup creates no second copy.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let first =
            backup_call(&source, &data_dir, None).expect("the first backup must be created");
        let BackupOutcome::Created(first_copy) = first else {
            panic!("the first backup must be Created, got: {first:?}");
        };
        assert_eq!(
            backup_listing(&data_dir),
            vec![expected_file_name(&ABC_DIGEST[..8])],
            "exactly one copy after the first backup"
        );

        let second = backup_call(&source, &data_dir, Some(&first_copy.sha256_hex))
            .expect("the dedup decision must not fail");

        assert_eq!(
            second,
            BackupOutcome::Deduplicated {
                size: 3,
                sha256_hex: ABC_DIGEST.to_owned(),
            },
            "the second call must dedup against the caller's last digest"
        );
        assert_eq!(
            backup_listing(&data_dir),
            vec![expected_file_name(&ABC_DIGEST[..8])],
            "the identical file must not create a second copy"
        );
    }

    #[test]
    fn a_dedup_decision_creates_no_folders() {
        // No copy, no layout: the dedup branch returns before any
        // path exists, so a project whose file is unchanged never
        // touches the backups tree at all.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let outcome = backup_call(&source, &data_dir, Some(ABC_DIGEST))
            .expect("the dedup decision must not fail");

        assert_eq!(
            outcome,
            BackupOutcome::Deduplicated {
                size: 3,
                sha256_hex: ABC_DIGEST.to_owned(),
            }
        );
        assert!(
            !data_dir.join(BACKUPS_DIR).exists(),
            "the backups tree must not be created for a dedup decision"
        );
    }

    #[test]
    fn a_different_last_hash_creates_a_fresh_copy() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");
        // A valid-shape digest that cannot equal the source's: the
        // comparison is exact full-digest equality.
        let other_digest = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let outcome = backup_call(&source, &data_dir, Some(other_digest))
            .expect("the backup must be created");

        assert!(
            matches!(outcome, BackupOutcome::Created(_)),
            "a different last digest must produce a fresh copy, got: {outcome:?}"
        );
        assert_eq!(
            backup_listing(&data_dir),
            vec![expected_file_name(&ABC_DIGEST[..8])],
            "exactly the new copy exists"
        );
    }

    #[test]
    fn a_last_hash_matching_only_the_hash8_prefix_still_creates_a_copy() {
        // The dedup comparison is FULL-digest equality (BKP-010: "the
        // SHA-256 of the source equals the SHA-256 of the last
        // backup"), not the name's 8-character short id: a last-hash
        // value sharing the hash8 PREFIX with the source's digest but
        // differing after it is a different digest and must produce a
        // fresh copy. An implementation comparing only the prefix —
        // the trap the name's hash8 invites — fails this test.
        let source_digest = ABC_DIGEST;
        let prefix_lookalike = format!("{}{}", &source_digest[..8], "0".repeat(56));
        assert_ne!(prefix_lookalike, source_digest);

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let outcome = backup_call(&source, &data_dir, Some(&prefix_lookalike))
            .expect("the backup must be created");

        assert!(
            matches!(outcome, BackupOutcome::Created(_)),
            "a prefix lookalike must not dedup, got: {outcome:?}"
        );
        assert_eq!(
            backup_listing(&data_dir),
            vec![expected_file_name(&ABC_DIGEST[..8])]
        );
    }

    #[test]
    fn an_uppercase_last_hash_is_not_equal_to_the_lowercase_digest() {
        // The comparison is exact byte equality on the canonical
        // LOWERCASE spelling (the docs' promise): the same digest
        // spelled in uppercase compares unequal and produces a fresh
        // copy rather than a silent dedup.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let outcome = backup_call(&source, &data_dir, Some(&ABC_DIGEST.to_uppercase()))
            .expect("the backup must be created");

        assert!(
            matches!(outcome, BackupOutcome::Created(_)),
            "an uppercase last hash must not dedup, got: {outcome:?}"
        );
        assert_eq!(
            backup_listing(&data_dir),
            vec![expected_file_name(&ABC_DIGEST[..8])]
        );
    }

    // ---- the check catches a damaged copy (BKP-010) ----------------------

    #[test]
    fn an_untouched_copy_passes_the_check() {
        // The control for the damaged-copy tests: the check accepts a
        // copy that was not tampered with.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");
        let BackupOutcome::Created(copy) =
            backup_call(&source, &data_dir, None).expect("the backup must be created")
        else {
            panic!("the first backup must be Created");
        };

        verify_backup_copy(&copy.path, copy.size, &copy.sha256_hex)
            .expect("an untouched copy must pass the check");
        assert!(
            copy.path.exists(),
            "a passing check leaves the copy in place"
        );
    }

    #[test]
    fn a_substituted_same_size_copy_is_caught_and_removed() {
        // The card's acceptance (a): a damaged copy substituted under
        // the right name — same length, different bytes — is caught
        // by the digest half of the check (the size half passes), and
        // the corrupt bytes are removed from disk.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");
        let BackupOutcome::Created(copy) =
            backup_call(&source, &data_dir, None).expect("the backup must be created")
        else {
            panic!("the first backup must be Created");
        };

        // Substitute: same 3-byte length, different content.
        fs::write(&copy.path, b"xyz").expect("substituting the copy bytes must work");

        let error = verify_backup_copy(&copy.path, copy.size, &copy.sha256_hex)
            .expect_err("a substituted copy must fail the check");

        let BackupError::VerifyFailed(failed) = &error else {
            panic!("the substitution must be VerifyFailed, got: {error:?}");
        };
        assert_eq!(failed.path, copy.path);
        assert_eq!(failed.expected_size, 3);
        assert_eq!(failed.expected_sha256_hex, ABC_DIGEST);
        let VerifyMismatch::Digest {
            actual_size,
            actual_sha256_hex,
        } = &failed.mismatch
        else {
            panic!(
                "the size matched, the digest did not: {:?}",
                failed.mismatch
            );
        };
        assert_eq!(*actual_size, 3, "the size half of the check passed");
        assert_eq!(
            actual_sha256_hex,
            // The real digest of b"xyz", computed with Python's
            // hashlib — an independent oracle.
            "3608bca1e44ea6c4d268eb6db02260269892c0b42b86bbf1e77a6fa16c3c9282"
        );
        assert!(
            failed.cleanup.is_none(),
            "the corrupt copy must be removable"
        );
        assert!(
            !copy.path.exists(),
            "the corrupt copy must be gone from disk"
        );
    }

    #[test]
    fn a_truncated_copy_is_caught_by_the_size_check() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");
        let BackupOutcome::Created(copy) =
            backup_call(&source, &data_dir, None).expect("the backup must be created")
        else {
            panic!("the first backup must be Created");
        };

        fs::write(&copy.path, b"a").expect("truncating the copy must work");

        let error = verify_backup_copy(&copy.path, copy.size, &copy.sha256_hex)
            .expect_err("a truncated copy must fail the check");

        let BackupError::VerifyFailed(failed) = &error else {
            panic!("the truncation must be VerifyFailed, got: {error:?}");
        };
        let VerifyMismatch::Size { actual_size } = failed.mismatch else {
            panic!("the size check fails first, got: {:?}", failed.mismatch);
        };
        assert_eq!(actual_size, 1, "the truncated copy reports its true size");
        assert!(
            failed.cleanup.is_none(),
            "the truncated copy must be removable"
        );
        assert!(!copy.path.exists(), "the truncated copy must be removed");
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_corrupt_copy_removal_is_reported_in_cleanup() {
        // The removal-on-failure contract's retry signal: when the
        // corrupt copy cannot be deleted (the folder refuses unlinks),
        // the error carries the OS failure in `cleanup`, the Display
        // says so (the BKP-005 message's "explanation of the cause"),
        // and the corrupt copy stays on disk for the run's
        // `discarded` sweep to retry — it is not silently dropped.
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");
        let BackupOutcome::Created(copy) =
            backup_call(&source, &data_dir, None).expect("the backup must be created")
        else {
            panic!("the first backup must be Created");
        };
        let project_folder = fixture_backup_folder(&data_dir);
        if running_as_root_is_untestable(&project_folder) {
            eprintln!(
                "skipping a_refused_corrupt_copy_removal_is_reported_in_cleanup: \
                 running as root — mode bits are not enforced, the \
                 unremovable-copy precondition cannot be set up"
            );
            return;
        }

        // Substitute corrupt bytes, then strip the folder's write bit
        // (r-x): every unlink inside is refused.
        fs::write(&copy.path, b"xyz").expect("substituting the copy bytes must work");
        fs::set_permissions(&project_folder, fs::Permissions::from_mode(0o500))
            .expect("removing the folder's write bit must work");
        let error = verify_backup_copy(&copy.path, copy.size, &copy.sha256_hex)
            .expect_err("the substituted copy must fail the check");
        // Restore before asserting, so TempDir's teardown can remove
        // the folder.
        fs::set_permissions(&project_folder, fs::Permissions::from_mode(0o700))
            .expect("restoring mode bits must work");

        let BackupError::VerifyFailed(failed) = &error else {
            panic!("the substitution must be VerifyFailed, got: {error:?}");
        };
        let Some(cleanup) = failed.cleanup.as_ref() else {
            panic!("a refused removal must be reported in cleanup, got None: {error:?}");
        };
        assert_eq!(
            cleanup.raw_os_error(),
            Some(13),
            "EACCES is the OS code the retry sweep will see"
        );
        assert!(
            copy.path.exists(),
            "the corrupt copy must stay on disk when its removal was refused"
        );
        assert!(
            error.to_string().contains("could not be removed"),
            "the Display must carry the removal failure: {error}"
        );
    }

    // ---- failure paths ----------------------------------------------------

    #[test]
    fn a_missing_source_is_a_typed_failure_with_the_os_code() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let missing = data_dir.join("never-created.md");

        let error = backup_call(&missing, &data_dir, None).expect_err("a missing source must fail");

        let BackupError::SourceFailed { path, source } = &error else {
            panic!("a missing source must be SourceFailed, got: {error:?}");
        };
        assert_eq!(path, &missing);
        // ENOENT on Unix and ERROR_FILE_NOT_FOUND on Windows share
        // the code 2.
        assert_eq!(error.os_code(), Some(2));
        assert!(source.raw_os_error().is_some());
        assert!(
            !data_dir.join(BACKUPS_DIR).exists(),
            "a failed snapshot must leave the backups tree uncreated"
        );
    }

    #[test]
    fn missing_backups_folders_are_created_and_the_root_is_private() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        assert!(
            !data_dir.join(BACKUPS_DIR).exists(),
            "the fixture starts without the backups tree"
        );
        let source = write_source(&data_dir, b"abc");

        let outcome = backup_call(&source, &data_dir, None).expect("the backup must be created");

        let BackupOutcome::Created(copy) = outcome else {
            panic!("the first backup must be Created, got: {outcome:?}");
        };
        assert!(copy.path.exists());
        assert!(fixture_backup_folder(&data_dir).is_dir());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(data_dir.join(BACKUPS_DIR))
                .expect("the backups root must exist")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "SEC-007: the backups root is 0700");
        }
    }

    #[test]
    fn a_missing_data_directory_is_a_folder_failure() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("never-created-data");
        let source = write_source(dir.path(), b"abc");

        let error =
            backup_call(&source, &data_dir, None).expect_err("the missing data dir must fail");

        assert!(
            matches!(error, BackupError::FolderCreateFailed { .. }),
            "a missing data directory must be FolderCreateFailed, got: {error:?}"
        );
        assert!(error.os_code().is_some(), "the OS code rides in the error");
    }

    #[test]
    fn an_existing_destination_is_never_overwritten() {
        // A backup is history: the create_new copy refuses to clobber
        // a file that already holds the destination name (a retried
        // run within the same millisecond collides here).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");
        let destination =
            fixture_backup_folder(&data_dir).join(expected_file_name(&ABC_DIGEST[..8]));
        fs::create_dir_all(destination.parent().expect("the destination has a parent"))
            .expect("creating the fixture folder must work");
        fs::write(&destination, b"KEEP").expect("writing the existing copy must work");

        let error = backup_call(&source, &data_dir, None).expect_err("the collision must fail");

        let BackupError::CopyFailed {
            path,
            cleanup,
            source: io_source,
        } = &error
        else {
            panic!("the collision must be CopyFailed, got: {error:?}");
        };
        assert_eq!(path, &destination);
        assert_eq!(io_source.kind(), io::ErrorKind::AlreadyExists);
        assert!(cleanup.is_none(), "nothing partial existed to remove");
        assert_eq!(
            fs::read(&destination).expect("reading the existing copy must work"),
            b"KEEP",
            "the existing copy's bytes must be untouched"
        );
    }

    // ---- SEC-006: paths come only from the builders -----------------------

    #[test]
    fn a_hostile_project_id_is_refused_through_the_fs_layer() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let error = backup_target_file(&BackupTarget {
            source: &source,
            data_dir: &data_dir,
            project_id: &ProjectId::new("bad/id".to_owned()),
            project_name: "My Project",
            scheme: Scheme::A,
            base_name: BaseName::AgentsMd,
            time: fixture_time(),
            last_backup_sha256_hex: None,
        })
        .expect_err("a hostile project id must be refused");

        let BackupError::NameFailed { source } = &error else {
            panic!("a hostile id must be NameFailed, got: {error:?}");
        };
        assert!(
            matches!(source, BackupNameError::UnsafeProjectId { .. }),
            "the builders' typed refusal must surface: {source:?}"
        );
        assert!(
            !data_dir.join(BACKUPS_DIR).exists(),
            "a refused name must not create the backups tree"
        );
    }

    #[test]
    fn a_hostile_project_name_cannot_escape_the_layout() {
        // The raw name is cleaned into the slug by the builders: the
        // created path stays inside backups/<id>__<slug>/, every
        // component a single Normal piece.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let source = write_source(&data_dir, b"abc");

        let outcome = backup_target_file(&BackupTarget {
            source: &source,
            data_dir: &data_dir,
            project_id: &fixture_id(),
            project_name: "../../etc/passwd",
            scheme: Scheme::A,
            base_name: BaseName::AgentsMd,
            time: fixture_time(),
            last_backup_sha256_hex: None,
        })
        .expect("a hostile NAME is cleaned, not refused");

        let BackupOutcome::Created(copy) = outcome else {
            panic!("the backup must be Created, got: {outcome:?}");
        };
        assert_eq!(
            copy.path,
            data_dir
                .join(BACKUPS_DIR)
                .join("0193f1a0-6d6d-7cce-a3c0-3d1f2a4b5c6d__etc-passwd")
                .join(expected_file_name(&ABC_DIGEST[..8])),
            "the copy lands in the slug folder the builders cleaned"
        );
        let relative: Vec<Component<'_>> = copy
            .path
            .strip_prefix(&data_dir)
            .expect("the copy must sit under the data directory")
            .components()
            .collect();
        assert_eq!(
            relative.len(),
            3,
            "the §5.5 layout is three components deep"
        );
        for component in &relative {
            assert!(
                matches!(component, Component::Normal(_)),
                "no traversal component may survive: {component:?}"
            );
        }
    }

    // ---- the error contract ------------------------------------------------

    #[test]
    fn the_error_display_and_source_chain_follow_the_contract() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).expect("creating the fixture data dir must work");
        let missing = data_dir.join("never-created.md");

        let source_error =
            backup_call(&missing, &data_dir, None).expect_err("a missing source must fail");
        assert!(
            source_error
                .to_string()
                .starts_with("cannot read the source file "),
            "the Display names the source file first: {source_error}"
        );
        assert!(
            source_error.source().is_some(),
            "the io error stays reachable through the source chain"
        );

        let name_source = write_source(&data_dir, b"abc");
        let name_error = backup_target_file(&BackupTarget {
            source: &name_source,
            data_dir: &data_dir,
            project_id: &ProjectId::new("../escape".to_owned()),
            project_name: "My Project",
            scheme: Scheme::A,
            base_name: BaseName::AgentsMd,
            time: fixture_time(),
            last_backup_sha256_hex: None,
        })
        .expect_err("a hostile id must be refused");
        assert!(
            name_error
                .to_string()
                .starts_with("the backup name was refused: "),
            "the Display defers to the builders' refusal: {name_error}"
        );
        assert!(
            name_error
                .source()
                .is_some_and(|source| source.is::<BackupNameError>()),
            "the builders' typed error rides in the source chain"
        );
        assert_eq!(
            name_error.os_code(),
            None,
            "a name refusal carries no OS code — nothing touched the disk"
        );

        let verify_error = BackupError::VerifyFailed(VerifyFailed {
            path: PathBuf::from("/data/backups/p__s/AGENTS.md.bak"),
            expected_size: 3,
            expected_sha256_hex: ABC_DIGEST.to_owned(),
            mismatch: VerifyMismatch::Size { actual_size: 1 },
            cleanup: None,
        });
        assert_eq!(
            verify_error.to_string(),
            "the backup copy /data/backups/p__s/AGENTS.md.bak failed the check: \
             3 bytes expected, 1 found"
        );
        assert!(
            verify_error.source().is_none(),
            "a size mismatch has no underlying io error"
        );
    }
}
