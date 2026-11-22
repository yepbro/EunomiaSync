//! Write rights for one sync target: the probe pair of spec 9.4
//! phase 1 step 3 (OUT-007/008, T-F5).
//!
//! The spec sentence: "Check rights: whether a temporary file can be
//! created in the folder, and whether the existing file is marked
//! read-only. If not: `skipped`, `E_NO_WRITE_ACCESS`." — the write-
//! rights half of the OUT-007/008 preflight ("Before writing, folder
//! availability and write rights are checked"; the availability half
//! is step 1's `E_PROJECT_UNAVAILABLE`, D-084), with T-F5 as the
//! required failure scenario: "The target file is read-only →
//! `skipped`, `E_NO_WRITE_ACCESS`". §10's row names the two facts this
//! module reports: "No write rights to the folder, or the file is
//! read-only."
//!
//! # The seam
//!
//! [`check_write_access`] takes the RESOLVED write path — the output
//! of `paths::resolve_target_path` (9.4 preflight step 2, D-092): one
//! path is everything step 3 needs (the read-only flag is a fact about
//! that path, the folder is its parent, and the staging name is its
//! file name). The `skipped`/`E_NO_WRITE_ACCESS` wording is the
//! pipeline's, not this module's: the verdicts are
//! [`WriteAccessError::FolderNotWritable`] and
//! [`WriteAccessError::FileReadOnly`] (both map to the command layer's
//! `ErrorCode::NoWriteAccess`, the `"E_NO_WRITE_ACCESS"` §10 literal),
//! while a refused probe — "could not check" — is
//! [`WriteAccessError::ProbeFailed`] and stays the caller's call (the
//! D-062 posture keeps the `AppError` registry out of this crate; the
//! D-090 negative-lookup/refusal split keeps a refusal from masquerading
//! as a rights verdict).
//!
//! # Two probes, the file flag first
//!
//! The spec sentence lists the folder probe first, but the order of a
//! conjunction is not normative — both refusals end in the same §10
//! outcome. This module checks the FILE's read-only flag first:
//!
//! - it is the nearer cause (a fact about the exact file the write
//!   replaces — the D-092 precedence style), and
//! - the folder probe is the check's only mutating step; running it
//!   last means an already-decided read-only verdict never touches the
//!   folder at all (the phase's "changes nothing" spirit).
//!
//! An absent target skips the flag check entirely (a first sync CREATES
//! the file, D-092; the card pins that the check is about rights, not
//! existence) and answers on the folder probe alone.
//!
//! # What "read-only" means (and why there is no cfg)
//!
//! [`std::fs::Permissions::readonly`] IS the cross-platform definition
//! of "marked read-only": on Unix a file with no write bits (mode
//! `& 0o222 == 0`, what `chmod a-w` produces), on Windows the
//! `FILE_ATTRIBUTE_READONLY` attribute — std reads the attribute in
//! its Windows metadata probe, so the idiomatic cfg handling lives in
//! std, not in this module. The flag is the MARKED state, not an OS
//! prediction: on Unix a `rename` (9.5 step 4) could in fact replace a
//! write-disabled file, but the spec still pins the read-only target as
//! a skip (T-F5) — the user's read-only mark is a "do not touch"
//! signal this preflight honors before any bytes move.
//!
//! # The folder probe is real, and its debris is sweepable
//!
//! "Whether a temporary file can be created in the folder" is answered
//! by creating one: `create_new` refuses double-creation, so a fresh
//! file with a never-used name fails ONLY for environmental reasons —
//! permission refused, folder gone, volume full — which is exactly the
//! honesty an access(2)-style mode-bit shortcut lacks (ACLs, immutable
//! flags and read-only mounts all answer here without this module
//! re-deriving them). The probe file is spelled with the app's staging
//! grammar ([`crate::temp_files::tmp_file_name`], D-088):
//! `.<name>.eunomia-<8 lowercase hex>.tmp`. That is not decoration: a
//! probe file the cleanup cannot remove (see below) is debris the
//! step-7 sweep (`remove_stale_tmp_files`, spec 9.4 phase 1 step 7)
//! recognizes by that grammar and deletes once it is an hour old —
//! the probe converges to "no net change" even across a crash between
//! creation and cleanup, the D-088 idempotency posture.
//!
//! A probe that succeeds removes its file before returning (the
//! handle is dropped first: Windows refuses to delete an open file).
//! If the removal fails, the check still answers `Ok` — creation, not
//! removal, is the rights fact — and the file stays as sweepable
//! staging debris; a removal that finds the file already gone is
//! convergence, not failure.
//!
//! # What a probe refusal means, classified
//!
//! A COLLISION (`AlreadyExists`) is not a rights verdict: 16^8 tokens
//! make it vanishingly rare, and the honest response is a fresh token
//! ([`PROBE_ATTEMPTS`] tries, this module's own bound — one fresh
//! token rules out an accidental collision, three cap the cost) — only
//! the exhaustion of all attempts is an
//! answer, and that answer is "could not check" ([`ProbeFailed`]),
//! never a fabricated `FolderNotWritable`. A permission refusal
//! (`PermissionDenied`) or a read-only volume
//! (`ReadOnlyFilesystem`) IS the §10 fact: no write rights to the
//! folder. Everything else — the folder vanished mid-preflight
//! (`NotFound`; step 1's availability check owns that situation), a
//! full volume (`StorageFull`; §10's `E_DISK_FULL` row belongs to the
//! write phase), a dropped drive — is [`ProbeFailed`] with the exact
//! [`io::Error`] riding along. The classification is a pure function
//! ([`classify_creation_refusal`]) pinned by synthetic-kind tests, the
//! D-089 `probe_result` posture: the real probe cannot be made to fail
//! on command on this host.
//!
//! # Degenerate inputs are refused, not guessed
//!
//! The contract is the resolved path of D-092 (absolute, with a parent
//! and a file name). A target without a parent folder or a file name —
//! a relative bare name, a root, `..` — is refused as
//! [`WriteAccessError::ProbeFailed`] (Unsupported) BEFORE anything is
//! created: guessing a folder (the process's working directory) would
//! probe, and litter, a folder the caller never named.
//!
//! # Races are honest (TOCTOU)
//!
//! The probes are separate OS calls over a live filesystem; the
//! verdict is what they observed at their moment (the D-084 posture).
//! Rights can tighten or loosen between this check and the write; the
//! write then refuses on its own (9.5 steps 2 and 4 report the
//! creation/replacement failure), and the next sync re-checks — a
//! sync is idempotent (§10). Each probe answers the state IT observed:
//! a target that appears after the flag probe is judged by the folder
//! probe alone (its read-only mark is asked about at the next sync),
//! and a target that vanishes after the flag probe leaves the folder
//! probe to answer for a file the write phase will then report missing.
//!
//! # Testing plan
//!
//! The card's acceptance test is the T-F5 scenario itself: a read-only
//! existing file in a writable folder is [`WriteAccessError::FileReadOnly`]
//! (the same `set_readonly` fixture drives the Windows CI leg — the
//! flag is std's, so the test is platform-neutral). The folder half:
//! a `0o555` folder (Unix-gated, permissions restored before the
//! assertions, loud skip under root where mode bits are not enforced —
//! the D-088 test posture) is `FolderNotWritable` with EACCES riding;
//! a writable folder passes and leaves NO debris. The creation case
//! (absent target) passes and creates nothing. The order pin: a
//! read-only file answers even when the folder is also unwritable.
//! A folder that cannot be traversed (`0o644`) makes the target's own
//! metadata probe the refusal — `ProbeFailed`, the D-090 split. The
//! collision path is driven through the private token seam with
//! deterministic tokens (retry succeeds on the second name; exhaustion
//! is `ProbeFailed`, never `FolderNotWritable`), and the refusal
//! classification is pinned kind by kind against synthetic errors.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::temp_files::tmp_file_name;

/// How many staging names one folder probe tries before declaring the
/// check unanswered: a collision is not a rights verdict, so the probe
/// retries with a fresh token, and only the exhaustion of every
/// attempt is an answer — "could not check" ([`WriteAccessError::ProbeFailed`]),
/// never a fabricated rights verdict. Three is this module's own
/// bound: one fresh token rules out an accidental collision, and three
/// attempts cap the cost — mirroring (not mandated by) 9.5 step 4's
/// three-retry pattern for refused renames, a house precedent.
const PROBE_ATTEMPTS: usize = 3;

/// Checks the write rights of one sync target (spec 9.4 phase 1
/// step 3): whether the EXISTING target file is marked read-only, and
/// whether a temporary file can be created in its folder.
///
/// `target` is the RESOLVED write path — `paths::resolve_target_path`'s
/// output (9.4 preflight step 2, D-092): an absolute path naming a
/// regular file (or a file the write will create). A symlinked spelling
/// is followed (`fs::metadata`), so the flag judged is the real file's
/// — the one the atomic write renames over.
///
/// The file's flag is checked first (the module docs give the order
/// ruling); an absent target skips straight to the folder probe (the
/// creation case: the check is about rights, not existence). The
/// folder probe creates one staging-grammar file
/// (`.<name>.eunomia-<token>.tmp`, D-088) with `create_new` and
/// removes it again — see the module docs for the debris-convergence
/// story that makes the probe's net effect zero.
///
/// # Errors
///
/// [`WriteAccessError::FileReadOnly`] when the existing target is
/// marked read-only (T-F5); [`WriteAccessError::FolderNotWritable`]
/// when creating the probe file was refused for access reasons
/// (`PermissionDenied`, a read-only volume) — both are the §10
/// `E_NO_WRITE_ACCESS` facts for the command layer.
/// [`WriteAccessError::ProbeFailed`] when the check could not answer:
/// a degenerate input (no parent folder or file name), a refused
/// metadata probe, a vanished folder, or the exhaustion of all
/// [`PROBE_ATTEMPTS`] token attempts over collisions — a refusal is
/// never silently turned into a rights verdict (the D-090 split).
///
/// # Examples
///
/// ```
/// use eunomia_fs::write_access::check_write_access;
/// use std::fs;
///
/// let dir = tempfile::tempdir().unwrap();
/// let target = dir.path().join("AGENTS.md");
/// fs::write(&target, b"managed block").unwrap();
///
/// // A writable file in a writable folder passes, and the probe's
/// // temporary file is gone again:
/// check_write_access(&target).unwrap();
/// assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
///
/// // The T-F5 refusal: a read-only target file.
/// let mut perms = fs::metadata(&target).unwrap().permissions();
/// perms.set_readonly(true);
/// fs::set_permissions(&target, perms).unwrap();
/// assert!(check_write_access(&target).is_err());
/// ```
pub fn check_write_access(target: &Path) -> Result<(), WriteAccessError> {
    check_write_access_with_tokens(target, probe_token)
}

/// The token-parameterized core of [`check_write_access`]: everything
/// but the entropy source, so tests drive the collision path with
/// deterministic tokens instead of racing real entropy (the private-
/// seam posture of `free_space`'s pure helpers).
fn check_write_access_with_tokens<F>(target: &Path, token: F) -> Result<(), WriteAccessError>
where
    F: Fn(usize) -> String,
{
    let path = target.to_string_lossy().into_owned();
    // The degenerate-input guard (module docs): no parent folder or no
    // file name means the contract (D-092's resolved path) was broken —
    // refuse BEFORE creating anything, rather than probing a guessed
    // folder (a relative bare name would stage in the process's
    // working directory).
    let Some(folder) = target
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    else {
        return Err(degenerate(
            &path,
            "the target path must have a parent folder",
        ));
    };
    let Some(file_name) = target.file_name() else {
        return Err(degenerate(&path, "the target path must have a file name"));
    };

    // Probe one: the existing file's read-only flag. `fs::metadata`
    // FOLLOWS a remaining link (the input is already resolved, but a
    // re-pointed link must judge the file the write would replace —
    // the D-092 TOCTOU posture), and std's `readonly()` is the
    // cross-platform marked-read-only fact (module docs).
    match fs::metadata(target) {
        Ok(metadata) => {
            if metadata.permissions().readonly() {
                return Err(WriteAccessError::FileReadOnly { path });
            }
        }
        // Absent target: the creation case (D-092) — nothing to flag,
        // the folder probe answers alone.
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(WriteAccessError::ProbeFailed { path, source }),
    }

    // Probe two: create one staging-grammar file in the folder.
    let name = file_name.to_string_lossy().into_owned();
    probe_folder(folder, &name, &path, token)
}

/// Creates and removes one staging-grammar file in `folder`: the
/// spec's "whether a temporary file can be created in the folder",
/// answered by doing it. `path` is the lossy TARGET path the error
/// variants carry (the §10 log line's subject); `target_name` is the
/// target's file name the staging grammar keys on (D-088: the probe's
/// debris is sweepable by exactly that grammar).
fn probe_folder<F>(
    folder: &Path,
    target_name: &str,
    path: &str,
    token: F,
) -> Result<(), WriteAccessError>
where
    F: Fn(usize) -> String,
{
    let mut last_collision: Option<io::Error> = None;
    for attempt in 0..PROBE_ATTEMPTS {
        // A defensive arm (D-094): a path-derived file name cannot fail
        // the grammar in practice (no separators, never empty, no NUL
        // in a canonical path); mapping the impossible case to
        // ProbeFailed keeps the production path unwrap-free.
        let name = match tmp_file_name(target_name, &token(attempt)) {
            Ok(name) => name,
            Err(source) => {
                return Err(WriteAccessError::ProbeFailed {
                    path: path.to_owned(),
                    source: io::Error::new(io::ErrorKind::Unsupported, source.to_string()),
                });
            }
        };
        let probe = folder.join(&name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(file) => {
                // Drop the handle before removing: Windows refuses to
                // delete an open file.
                drop(file);
                // Best-effort cleanup: the CREATION answered the
                // rights question, so a failed removal changes nothing
                // about the verdict — and a NotFound means somebody
                // else removed it first (convergence either way). The
                // worst case leaves one staging file behind, and that
                // is exactly the debris the step-7 sweep
                // (`remove_stale_tmp_files`, D-088's grammar) deletes
                // an hour later: the probe's net effect converges to
                // zero even if this process dies right here.
                let _ = fs::remove_file(&probe);
                return Ok(());
            }
            Err(source) => match classify_creation_refusal(&source) {
                CreationRefusal::NotWritable => {
                    return Err(WriteAccessError::FolderNotWritable {
                        path: path.to_owned(),
                        source,
                    });
                }
                CreationRefusal::Collision => {
                    last_collision = Some(source);
                    continue;
                }
                CreationRefusal::Refused => {
                    return Err(WriteAccessError::ProbeFailed {
                        path: path.to_owned(),
                        source,
                    });
                }
            },
        }
    }
    // Every name collided: the folder is full of staging-shaped names,
    // which says nothing about rights — the honest answer is "could
    // not check", with the last collision riding in the source.
    Err(WriteAccessError::ProbeFailed {
        path: path.to_owned(),
        source: last_collision
            .unwrap_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no token was tried")),
    })
}

/// What a refused probe-file creation means for the rights question:
/// the pure classification the probe loop branches on, split out so
/// synthetic-kind tests can pin every arm (the `free_space`
/// `probe_result` posture — the real probe cannot be made to fail on
/// command on this host).
fn classify_creation_refusal(source: &io::Error) -> CreationRefusal {
    match source.kind() {
        // The §10 fact: the folder (or its volume) refuses new files.
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            CreationRefusal::NotWritable
        }
        // The name is taken: not a rights verdict, try the next token.
        io::ErrorKind::AlreadyExists => CreationRefusal::Collision,
        // Folder gone (step 1 owns availability), volume full (the
        // E_DISK_FULL write-phase row), a dropped drive: could not
        // check, the caller decides.
        _ => CreationRefusal::Refused,
    }
}

/// The tri-state answer of [`classify_creation_refusal`].
enum CreationRefusal {
    /// Access was refused: the no-write-rights verdict.
    NotWritable,
    /// The staging name already exists: retry with a fresh token.
    Collision,
    /// The probe could not answer; the caller decides the semantics.
    Refused,
}

/// One staging token for the folder probe: 8 lowercase hexadecimal
/// characters — exactly the grammar [`tmp_file_name`] validates
/// (D-088's alphabet). The token need not be cryptographic (a staging
/// name is no secret); it must only be fresh, so the entropy is the
/// clock folded with the process id (tests and sibling processes run
/// concurrently) and the attempt counter (a collision must not repeat
/// the colliding name). A clock before the epoch folds ZERO — still a
/// valid token, and `create_new` plus the bounded retry remains the
/// real safety.
fn probe_token(attempt: usize) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos();
    let mixed = (nanos as u32)
        ^ ((nanos >> 32) as u32)
        ^ std::process::id()
        ^ (attempt as u32).rotate_left(8);
    format!("{mixed:08x}")
}

/// The degenerate-input refusal of [`check_write_access_with_tokens`]:
/// an Unsupported probe failure with the contract message, no OS call
/// made.
fn degenerate(path: &str, message: &str) -> WriteAccessError {
    WriteAccessError::ProbeFailed {
        path: path.to_owned(),
        source: io::Error::new(io::ErrorKind::Unsupported, message),
    }
}

/// Why the write-access check said no.
///
/// One variant per §10 situation behind `E_NO_WRITE_ACCESS` (the
/// command layer maps both rights verdicts to `ErrorCode::NoWriteAccess`,
/// the `"E_NO_WRITE_ACCESS"` literal), plus the refused-probe bucket
/// for everything that is "could not check" rather than a verdict (the
/// D-090 negative-lookup/refusal split). Every variant carries the
/// TARGET path (lossy text — the same text the §10 log line shows);
/// the OS-backed variants carry the [`io::Error`] itself as their
/// `source`, whose `raw_os_error` and `ErrorKind` are §10's
/// first-class log facts (the shape `ErrorDetails::from_io_error`
/// consumes).
#[derive(Debug)]
pub enum WriteAccessError {
    /// Creating a file in the target's folder was refused for access
    /// reasons: `PermissionDenied` (EACCES/EPERM; Windows
    /// ERROR_ACCESS_DENIED) or a read-only volume (`ReadOnlyFilesystem`,
    /// EROFS) — §10's "No write rights to the folder".
    FolderNotWritable {
        /// The target path whose folder refused the probe (lossy form).
        path: String,
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
    /// The existing target file is marked read-only (Unix: no write
    /// bits; Windows: FILE_ATTRIBUTE_READONLY) — §10's "the file is
    /// read-only", the T-F5 scenario. A tree fact: no OS call failed,
    /// so there is no io error to carry.
    FileReadOnly {
        /// The read-only target path (lossy form).
        path: String,
    },
    /// The check could not answer: a degenerate input (no parent
    /// folder or file name), a refused metadata probe, a vanished
    /// folder, or every probe token collided. This is NOT a rights
    /// verdict; the caller decides the run semantics (the D-062
    /// posture keeps the §10 mapping out of this crate).
    ProbeFailed {
        /// The target path the check could not answer for (lossy form).
        path: String,
        /// The OS failure, carrying the raw error code; a synthetic
        /// Unsupported error for the degenerate-input cases.
        source: io::Error,
    },
}

impl WriteAccessError {
    /// The target path this failure is about (lossy form, the same
    /// text the §10 log line shows).
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            WriteAccessError::FolderNotWritable { path, .. }
            | WriteAccessError::FileReadOnly { path }
            | WriteAccessError::ProbeFailed { path, .. } => path,
        }
    }

    /// The I/O error behind this failure, when an OS call produced it;
    /// `None` for the verdicts that are plain facts about the tree
    /// ([`FileReadOnly`]) rather than OS failures.
    #[must_use]
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            WriteAccessError::FolderNotWritable { source, .. }
            | WriteAccessError::ProbeFailed { source, .. } => Some(source),
            WriteAccessError::FileReadOnly { .. } => None,
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`) — §10
    /// names it a first-class fact of every error log line. `None`
    /// where no OS call failed.
    #[must_use]
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl fmt::Display for WriteAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteAccessError::FolderNotWritable { path, source } => {
                write!(
                    f,
                    "no write rights to the folder of target {path}: {source}"
                )
            }
            WriteAccessError::FileReadOnly { path } => {
                write!(f, "target file {path} is marked read-only")
            }
            WriteAccessError::ProbeFailed { path, source } => {
                write!(f, "write-access probe failed for target {path}: {source}")
            }
        }
    }
}

impl std::error::Error for WriteAccessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::print_stderr)] // test skip-notices and benchmark timings print deliberately

    use super::*;
    use std::error::Error as _;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Marks a file read-only the portable way: std's `set_readonly`
    /// clears the Unix write bits and sets Windows
    /// FILE_ATTRIBUTE_READONLY — the same fact the check reads back,
    /// so the fixture and the probe share one definition.
    fn make_read_only(path: &Path) {
        let mut perms = fs::metadata(path)
            .expect("stating the fixture file must work")
            .permissions();
        perms.set_readonly(true);
        fs::set_permissions(path, perms).expect("marking the fixture read-only must work");
    }

    /// The names of all entries directly in `folder` (files, folders,
    /// links alike), sorted: what the "leaves no debris" assertions
    /// compare against.
    fn folder_entries(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .expect("listing the fixture folder must work")
            .map(|entry| {
                entry
                    .expect("reading a directory entry must work")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// A deterministic token source for the private seam: attempt 0 is
    /// `"00000000"`, attempt 1 `"00000001"`, … — valid grammar tokens
    /// (8 lowercase hex) that tests can pre-create on disk.
    fn counted_tokens(attempt: usize) -> String {
        format!("{attempt:08x}")
    }

    // ---- the file flag: T-F5 and its complements -----------------------

    #[test]
    fn a_read_only_target_file_is_no_write_access_t_f5() {
        // THE card acceptance and spec 15.2 T-F5: "The target file is
        // read-only → skipped, E_NO_WRITE_ACCESS". At this layer the
        // verdict is the typed FileReadOnly fact; the §10 literal is
        // the command layer's mapping. The folder is writable here, so
        // the file flag alone decides.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"managed block").expect("writing the fixture must work");
        make_read_only(&target);

        let err = check_write_access(&target)
            .expect_err("a read-only target must fail the write-access check");

        let expected = target.to_string_lossy().into_owned();
        assert!(
            matches!(&err, WriteAccessError::FileReadOnly { .. }),
            "the T-F5 verdict must be FileReadOnly, got: {err:?}"
        );
        assert_eq!(err.path(), expected);
        assert_eq!(
            err.os_code(),
            None,
            "the read-only mark is a tree fact, no OS call failed"
        );
        assert_eq!(
            err.to_string(),
            format!("target file {expected} is marked read-only"),
            "the Display text must name the file and the fact"
        );
        // The probe leaves no debris on this path. NOTE: this listing
        // alone does not pin the file-first ORDER — a probe that ran
        // and cleaned up would leave the same listing; the order pin
        // is the both-refuse test further down.
        assert_eq!(
            folder_entries(dir.path()),
            vec!["AGENTS.md".to_owned()],
            "a decided read-only verdict must not litter the folder"
        );
    }

    #[test]
    fn a_writable_target_in_a_writable_folder_passes_and_leaves_no_debris() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"managed block").expect("writing the fixture must work");

        check_write_access(&target).expect("a writable target in a writable folder must pass");

        // The probe's net effect is zero: the staging file it created
        // is gone, only the target remains, and the target's CONTENT is
        // untouched — the check never opens it (the phase 1 "changes
        // nothing" rule, §9.4).
        assert_eq!(
            fs::read(&target).expect("reading the fixture back must work"),
            b"managed block",
            "the check must not touch the target's content"
        );
        assert_eq!(
            folder_entries(dir.path()),
            vec!["AGENTS.md".to_owned()],
            "the probe must clean up its staging file"
        );
    }

    #[test]
    fn an_absent_target_passes_and_creates_nothing() {
        // The creation case (D-092): the check is about RIGHTS, not
        // existence — an absent target is Ok, and the preflight phase
        // still changes nothing: no target file, no staging debris.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");

        check_write_access(&target).expect("an absent target in a writable folder must pass");

        assert!(
            !target.exists(),
            "the check must not create the target file"
        );
        assert!(
            folder_entries(dir.path()).is_empty(),
            "the check must not leave staging debris"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_target_is_judged_by_the_real_file() {
        // The input contract is the RESOLVED path, but `fs::metadata`
        // follows any link anyway: the flag judged belongs to the file
        // the write would replace (the D-092 posture). A read-only
        // REAL file behind a link spelling is the T-F5 refusal for the
        // link's sync too; a writable one passes.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let real = dir.path().join("AGENTS.md");
        fs::write(&real, b"managed block").expect("writing the fixture must work");
        let link = dir.path().join("CLAUDE.md");
        std::os::unix::fs::symlink(&real, &link).expect("creating the symlink must work");

        check_write_access(&link).expect("a link to a writable real file must pass");

        make_read_only(&real);
        let err = check_write_access(&link).expect_err("a link to a read-only real file must fail");
        assert!(
            matches!(&err, WriteAccessError::FileReadOnly { .. }),
            "the verdict must follow the real file, got: {err:?}"
        );
        assert_eq!(
            err.path(),
            link.to_string_lossy().into_owned(),
            "the error names the path it was asked about, the link spelling"
        );
    }

    // ---- the folder probe ----------------------------------------------

    /// The fixture's owner uid is this process's effective uid (the
    /// fixture was created by it): uid 0 means the tests run as root,
    /// where mode bits are not enforced and an unwritable directory
    /// stays writable — the precondition cannot be set up, so the
    /// caller skips loudly (the D-088 test posture).
    #[cfg(unix)]
    fn running_as_root_is_untestable(path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_without_write_permission_is_no_write_access() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"managed block").expect("writing the fixture must work");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping a_folder_without_write_permission_is_no_write_access: \
                 running as root — mode bits are not enforced, the \
                 unwritable-folder precondition cannot be set up"
            );
            return;
        }

        // r-x for everyone: the folder is listable and the file inside
        // statable (so the flag check passes and the PROBE decides),
        // but no file may be created.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555))
            .expect("removing the write bits must work");
        let result = check_write_access(&target);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("an unwritable folder must fail the check");
        assert!(
            matches!(&err, WriteAccessError::FolderNotWritable { .. }),
            "a 0o555 folder must be FolderNotWritable, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES = 13 is the OS code the §10 log line carries"
        );
        assert!(
            err.to_string()
                .starts_with("no write rights to the folder of target "),
            "the Display text must state the §10 fact: {err}"
        );
        assert!(
            target.exists(),
            "the check must not touch the target itself"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_file_answers_even_when_the_folder_is_unwritable_too() {
        use std::os::unix::fs::PermissionsExt;

        // The order ruling, pinned: when BOTH facts hold, the file's
        // read-only flag is the verdict (the nearer cause, the mutating
        // probe never runs) — not the folder refusal.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"managed block").expect("writing the fixture must work");
        make_read_only(&target);
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping a_read_only_file_answers_even_when_the_folder_is_unwritable_too: \
                 running as root — mode bits are not enforced, the \
                 unwritable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555))
            .expect("removing the write bits must work");
        let result = check_write_access(&target);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("both refusals hold, so the check must fail");
        assert!(
            matches!(&err, WriteAccessError::FileReadOnly { .. }),
            "the file flag is the nearer cause and must win, got: {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_target_that_cannot_be_statted_is_a_probe_failure_not_a_rights_verdict() {
        use std::os::unix::fs::PermissionsExt;

        // The refused-metadata arm of the flag probe (the module docs'
        // "a refused metadata probe"): a folder that lost its execute
        // bit (0o755 → 0o644) lets the earlier preflight steps pass
        // but refuses the target's own stat — EACCES on the way to the
        // entry, which is "could not check" (the D-090 split), never a
        // fabricated FolderNotWritable. The discrimination matters: a
        // folder-probe-first implementation would answer
        // FolderNotWritable here (creating a file is refused for the
        // same missing bits) and fail this test.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"managed block").expect("writing the fixture must work");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping a_target_that_cannot_be_statted_is_a_probe_failure_not_a_rights_verdict: \
                 running as root — mode bits are not enforced, the \
                 untraversable-folder precondition cannot be set up"
            );
            return;
        }

        // rw-r--r--: readable, but no execute bit to search through, so
        // statting the file inside is refused.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o644))
            .expect("dropping the execute bits must work");
        let result = check_write_access(&target);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("an untraversable folder must fail the check");
        assert!(
            matches!(&err, WriteAccessError::ProbeFailed { .. }),
            "a refused metadata probe must be ProbeFailed, got: {err:?}"
        );
        assert_eq!(
            err.io_error().map(|source| source.kind()),
            Some(io::ErrorKind::PermissionDenied),
            "the EACCES refusal rides in the typed error"
        );
        assert_eq!(err.os_code(), Some(13), "EACCES = 13 for the §10 log line");
    }

    #[test]
    fn a_missing_folder_is_a_probe_failure_not_a_rights_verdict() {
        // The folder vanished (or was never there): the probe cannot
        // answer, and "no write rights" would be a fabricated verdict —
        // step 1's availability check owns the missing-folder story
        // (E_PROJECT_UNAVAILABLE), and the OS refusal rides along.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("never-created").join("AGENTS.md");

        let err = check_write_access(&target).expect_err("a missing folder must fail the check");

        assert!(
            matches!(&err, WriteAccessError::ProbeFailed { .. }),
            "a missing folder must be ProbeFailed, got: {err:?}"
        );
        assert_eq!(
            err.io_error().map(|source| source.kind()),
            Some(io::ErrorKind::NotFound),
            "the NotFound refusal must ride in the typed error"
        );
    }

    #[test]
    fn a_relative_target_without_a_folder_is_refused_without_touching_anything() {
        // The degenerate-input guard: a bare relative name has no
        // parent folder, so the contract (the resolved path of D-092)
        // is broken — refused BEFORE any creation, never staged in the
        // process's working directory.
        let err = check_write_access(Path::new("AGENTS.md"))
            .expect_err("a bare relative name must be refused");

        assert!(
            matches!(&err, WriteAccessError::ProbeFailed { .. }),
            "a degenerate target must be ProbeFailed, got: {err:?}"
        );
        assert_eq!(
            err.io_error().map(|source| source.kind()),
            Some(io::ErrorKind::Unsupported),
            "the degenerate refusal is a contract fact, not an OS failure"
        );
        assert_eq!(err.os_code(), None, "no OS call was made");
        // The litter shape a broken guard would leave is a staging
        // grammar name in the working directory — scan for it there
        // (the expect_err above already pins the refusal itself; this
        // catches a regression that drops the probe's cleanup).
        assert!(
            !folder_entries(Path::new("."))
                .iter()
                .any(|name| name.starts_with(".AGENTS.md.eunomia-")),
            "no staging debris may appear in the working directory"
        );
    }

    // ---- the collision path, driven with deterministic tokens ----------

    #[test]
    fn the_probe_collides_with_a_staging_file_and_retries_with_the_next_token() {
        // The probe's name follows the app staging grammar (D-088) —
        // pinned by MAKING it collide: the token-0 name exists on
        // disk, and the probe must answer on token 1. This also proves
        // the probe cleans up its own file and leaves the pre-existing
        // staging name (the sweep's business, not the probe's) alone.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"managed block").expect("writing the fixture must work");
        let colliding = dir.path().join(".AGENTS.md.eunomia-00000000.tmp");
        fs::write(&colliding, b"old debris").expect("writing the fixture must work");

        check_write_access_with_tokens(&target, counted_tokens)
            .expect("a collision is not a rights verdict; the retry must pass");

        assert_eq!(
            folder_entries(dir.path()),
            vec![
                ".AGENTS.md.eunomia-00000000.tmp".to_owned(),
                "AGENTS.md".to_owned(),
            ],
            "the pre-existing staging name stays; the probe's own file is cleaned up"
        );
    }

    #[test]
    fn probe_exhaustion_over_collisions_is_a_probe_failure_not_a_rights_verdict() {
        // Every attempt collides: the folder is full of staging-shaped
        // names, which says NOTHING about rights — the honest answer is
        // "could not check", never a fabricated FolderNotWritable.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        fs::write(&target, b"managed block").expect("writing the fixture must work");
        fs::write(
            dir.path().join(".AGENTS.md.eunomia-00000000.tmp"),
            b"the only staging name",
        )
        .expect("writing the fixture must work");
        let always_zero = |_: usize| "00000000".to_owned();

        let err = check_write_access_with_tokens(&target, always_zero)
            .expect_err("a probe that always collides must fail");

        assert!(
            matches!(&err, WriteAccessError::ProbeFailed { .. }),
            "exhaustion must be ProbeFailed, got: {err:?}"
        );
        assert_eq!(
            err.io_error().map(|source| source.kind()),
            Some(io::ErrorKind::AlreadyExists),
            "the last collision rides in the source"
        );
    }

    // ---- the refusal classification, pinned kind by kind ----------------

    #[test]
    fn permission_denied_and_read_only_volumes_are_the_no_write_rights_fact() {
        // Synthetic kinds (the free_space probe_result posture): the
        // real probe cannot be made to fail on command on this host,
        // but every classification arm must be pinned. EACCES/EPERM
        // and ERROR_ACCESS_DENIED all surface as PermissionDenied; a
        // read-only mount (EROFS) as ReadOnlyFilesystem — both are the
        // §10 "No write rights to the folder" fact.
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::ReadOnlyFilesystem,
        ] {
            assert!(
                matches!(
                    classify_creation_refusal(&io::Error::from(kind)),
                    CreationRefusal::NotWritable
                ),
                "{kind:?} must be the no-write-rights fact"
            );
        }
    }

    #[test]
    fn a_collision_is_classified_apart_from_verdicts() {
        assert!(
            matches!(
                classify_creation_refusal(&io::Error::from(io::ErrorKind::AlreadyExists)),
                CreationRefusal::Collision
            ),
            "AlreadyExists is a retry, not a verdict"
        );
    }

    #[test]
    fn every_other_refusal_is_could_not_check() {
        // NotFound (the folder vanished — step 1 owns availability),
        // StorageFull (§10's E_DISK_FULL write-phase row), a dropped
        // drive (Other): none of these is a rights verdict.
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::StorageFull,
            io::ErrorKind::Other,
        ] {
            assert!(
                matches!(
                    classify_creation_refusal(&io::Error::from(kind)),
                    CreationRefusal::Refused
                ),
                "{kind:?} must stay a probe failure, not a verdict"
            );
        }
    }

    #[test]
    fn the_probe_token_is_eight_lowercase_hex_characters_every_time() {
        // The grammar contract of tmp_file_name (D-088): whatever the
        // entropy folds, the token must stay valid — 100 draws over
        // shifting time pin the format, not the values.
        for attempt in 0..100 {
            let token = probe_token(attempt);
            assert_eq!(
                token.len(),
                8,
                "attempt {attempt} produced {token:?} of the wrong length"
            );
            assert!(
                token
                    .bytes()
                    .all(|byte| b"0123456789abcdef".contains(&byte)),
                "attempt {attempt} produced {token:?} outside the alphabet"
            );
        }
        // Consecutive attempts differ (a collision must not repeat the
        // colliding name) — with counted certainty for two draws at
        // least far enough apart in the counter.
        assert_ne!(probe_token(0), probe_token(1));
    }

    // ---- the error contract ---------------------------------------------

    #[test]
    fn error_contract_display_paths_and_source_chains() {
        // The command layer branches on the variants and logs the
        // Display text; these assertions pin the wording (lowercase,
        // no trailing punctuation — the house style), the path()
        // accessor per variant, the io-backed/fact split and the
        // source() chain without pinning std's strerror table.
        let target = PathBuf::from("/repo/AGENTS.md");
        let path = target.to_string_lossy().into_owned();

        let fact = WriteAccessError::FileReadOnly { path: path.clone() };
        assert_eq!(fact.path(), path);
        assert!(fact.io_error().is_none());
        assert_eq!(fact.os_code(), None);
        assert!(fact.source().is_none());
        assert_eq!(
            fact.to_string(),
            "target file /repo/AGENTS.md is marked read-only"
        );

        let denied = WriteAccessError::FolderNotWritable {
            path: path.clone(),
            source: io::Error::from_raw_os_error(13),
        };
        assert_eq!(denied.path(), path);
        assert_eq!(denied.os_code(), Some(13), "EACCES = 13 rides along");
        assert!(
            denied
                .to_string()
                .starts_with("no write rights to the folder of target /repo/AGENTS.md: "),
            "the Display text must state the fact, then the OS text: {denied}"
        );
        assert_eq!(
            denied.source().map(|source| source.to_string()),
            Some(io::Error::from_raw_os_error(13).to_string()),
            "source() chains exactly to the carried io error"
        );

        let refused = WriteAccessError::ProbeFailed {
            path,
            source: io::Error::from_raw_os_error(5),
        };
        assert_eq!(refused.os_code(), Some(5), "EIO = 5 rides along");
        assert!(
            refused
                .to_string()
                .starts_with("write-access probe failed for target /repo/AGENTS.md: "),
            "the Display text must name the target, then the OS text: {refused}"
        );
    }

    // Linux-only, and deliberately so: the fixture creates a file whose
    // name is invalid UTF-8, which only Linux filesystems accept — the
    // same platform posture as temp_files' non-UTF-8 sweep test.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_non_utf8_target_name_still_probes_its_folder() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        // The lossy-name posture (D-088/D-092): the staging grammar
        // takes lossy text, so a non-UTF-8 target name must not fail
        // the rights check — the probe stages under the lossy spelling
        // (sweepable debris) and the verdict is the folder's to give.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let name = OsStr::from_bytes(b"AGENTS\xff.md");
        let target = dir.path().join(name);
        fs::write(&target, b"managed block").expect("writing the fixture must work");

        check_write_access(&target).expect("a non-UTF-8 name must not fail the rights check");
        assert_eq!(
            folder_entries(dir.path()),
            vec!["AGENTS\u{fffd}.md".to_owned()],
            "the probe must leave no debris under the lossy spelling"
        );
    }
}
