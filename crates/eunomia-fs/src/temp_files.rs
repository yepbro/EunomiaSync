//! The app's temporary files in target folders: the name convention and
//! the stale-file cleanup.
//!
//! Spec 9.5 step 2 stages every atomic write as a same-folder temporary
//! file named `.<name>.eunomia-<8 random characters>.tmp`, and spec 9.4
//! phase 1 step 7 tells the sync pipeline to "delete stale app temporary
//! files in the target folder (older than one hour)" — the debris a
//! killed process leaves behind (the T-F2 scenario: the original file is
//! intact; on the next sync, the temp file is deleted). This module pins
//! both halves of that convention in one place: [`tmp_file_name`] is the
//! only way the writer (the future atomic-write module) spells a staging
//! name, and [`is_app_tmp_file`] is the matcher the cleanup trusts —
//! one grammar, so the writer can never produce debris the sweep would
//! not recognize.
//!
//! # The name grammar (pinned)
//!
//! `.` + target file name + `.eunomia-` + 8 characters + `.tmp`,
//! compared byte for byte (case-sensitive) against the folder entry:
//!
//! - the leading dot keeps the staging file hidden and distinguishes it
//!   from every ordinary file;
//! - the target file name is the file being replaced (`AGENTS.md`,
//!   `CLAUDE.md`, `<STEM>.project.md` — no path separators, it comes
//!   from the pipeline's own target path); the sweep keys on it, so a
//!   project only ever removes staging files of its own target;
//! - `.eunomia-` is the app marker — a name without it is never ours;
//! - the 8 characters are LOWERCASE HEX (`[0-9a-f]`,
//!   [`TMP_TOKEN_ALPHABET`]). The spec says "8 random characters" and
//!   pins no alphabet; this module derives it from the repo's
//!   conventions: `hash.rs` renders every digest as lowercase hex and
//!   D-080 already fixed the 8-character short-id form as the first 8
//!   lowercase-hex characters of a SHA-256 — one alphabet across all
//!   app-generated short identifiers, no second convention invented.
//!   Hex is also the safe choice for deletion: it excludes uppercase
//!   letters, so a foreign `.X.eunomia-ABCDEFGH.tmp` stays foreign.
//!   16^8 ≈ 4·10^9 tokens are ample for staging names that are created
//!   with `create_new` (a collision is an error and a retry, not
//!   corruption);
//! - `.tmp` is the suffix, and nothing may follow it.
//!
//! Everything else in the folder is foreign and is never touched.
//!
//! # What the sweep deletes
//!
//! [`remove_stale_tmp_files`] lists the target folder (one level, never
//! recursive — spec 9.5 creates staging files in the same folder as the
//! target, so deeper entries cannot be the writer's) and deletes an
//! entry only when ALL of the following hold:
//!
//! 1. the entry name matches [`is_app_tmp_file`] for that target;
//! 2. the entry IS a regular file (`DirEntry::metadata`, which does not
//!    traverse symlinks) — the writer only ever creates regular files,
//!    so a directory or a symlink bearing a tmp-shaped name is not an
//!    app temporary file and stays (a symlink is additionally never
//!    followed when reading its metadata: D-07's not-followed posture,
//!    ALG-054's names-not-contents rule);
//! 3. the entry's own modification time is strictly older than one
//!    hour before `now` — "older than one hour" (spec 9.4) read as a
//!    strict inequality, and ambiguity about deletion resolves toward
//!    not deleting: a file exactly one hour old stays until the next
//!    sweep, which costs nothing and cannot lose data.
//!
//! The sweep reads entry names and metadata only; it never opens or
//! reads file contents (the ALG-054 posture).
//!
//! # Failure semantics
//!
//! - A missing folder is a successful no-op (an empty report): phase 1
//!   step 7 is housekeeping between the availability check (step 1)
//!   and the write (phase 3), so the sweep must never fail a sync run
//!   for it, and "no stale tmp files in a folder that does not exist"
//!   holds vacuously — the write phase reports a vanished folder far
//!   more precisely than this step could.
//! - A folder that cannot be listed at all is
//!   [`TmpSweepError::ListingFailed`] with the OS error; the caller
//!   logs it and continues the run.
//! - A per-file failure (the entry cannot be statted, or deletion is
//!   refused — e.g. the folder is not writable) is recorded in
//!   [`TmpSweepReport::failed`] and the sweep continues with the rest:
//!   phase 1's spirit is that one project's trouble does not stop the
//!   run, and one unremovable file must not stop the cleanup either.
//! - An entry that vanishes between the listing and its own removal
//!   (another cleanup, the writer's error path) is convergence, not a
//!   failure: the goal state "the stale file is gone" already holds,
//!   so it is silently skipped and never invented into the report.
//!
//! # Races are honest (TOCTOU)
//!
//! The sweep is a listing, a stat and an unlink per candidate over a
//! live filesystem — no snapshot, no atomicity pretended. A staging
//! file the writer creates after the listing is simply not in it (and
//! is at most one hour from its own sweep — the writer deletes its own
//! temporary on every error path, spec 9.5 step 7). An mtime in the
//! future (clock skew) is never stale: `now.duration_since(mtime)`
//! fails there, and the entry stays. The one-hour window is what the
//! spec chose exactly so that a file written moments ago is never a
//! deletion candidate.
//!
//! # Time is a parameter, never a clock
//!
//! `now` is an explicit [`SystemTime`] argument (D-061's posture): the
//! sync pipeline reads the clock once per run and hands it down, so
//! the sweep is deterministic and testable without sleeping.
//!
//! # Testing plan
//!
//! The matcher and the name builder are pure and tested against a
//! near-miss battery (missing dot, extra suffix, wrong token length,
//! non-hex and uppercase tokens, another target's staging name, extra
//! prefix text) plus builder/matcher round trips. The sweep is tested
//! over real temporary folders: stale deleted, fresh kept, foreign kept
//! (with stale mtimes too), the exact one-hour boundary, multiple stale
//! files, a missing folder, a subfolder not swept, a future mtime, and
//! — Unix-gated where mode bits or symlinks are involved — a permission
//! failure collected without aborting the sweep (skipped loudly on
//! root, where mode bits are not enforced), a symlink and a directory
//! with a tmp-shaped name left alone, and a non-UTF-8 entry name never
//! deleted.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The length of the random token in a staging-file name: the spec's
/// "8 random characters" (9.5 step 2).
pub const TMP_TOKEN_LEN: usize = 8;

/// The only characters a staging-file token may contain: lowercase
/// hexadecimal. See the module docs for why this alphabet (the repo's
/// `hash.rs`/D-080 lowercase-hex convention; the tightest safe choice
/// for deletion matching).
pub const TMP_TOKEN_ALPHABET: &str = "0123456789abcdef";

/// Whether `target_name` is a single plain file name: not empty, no
/// path separators or NUL (per the running platform's separator rules),
/// and not `.`/`..`. The builder refuses anything else, and the matcher
/// matches nothing for such a target — one predicate, so the two
/// halves of the grammar cannot disagree about what a target may be.
fn is_plain_file_name(target_name: &str) -> bool {
    !target_name.is_empty()
        && !target_name.bytes().any(|byte| byte == 0)
        && !target_name.chars().any(std::path::is_separator)
        && !matches!(target_name, "." | "..")
}

/// The leading dot of the staging-name grammar.
const TMP_LEADING_DOT: char = '.';

/// The app marker of the staging-name grammar: `.eunomia-`.
const TMP_MARKER: &str = ".eunomia-";

/// The suffix of the staging-name grammar: `.tmp`.
const TMP_SUFFIX: &str = ".tmp";

/// How old a staging file must be before the sweep deletes it: one
/// hour, strictly (spec 9.4 phase 1 step 7, "older than one hour").
const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// Builds the staging-file name for a target (spec 9.5 step 2):
/// `.<target_name>.eunomia-<token>.tmp`.
///
/// `target_name` is the name of the file being replaced — the pipeline
/// takes it from the RESOLVED target path's own file name (9.5 step 1
/// resolves symlinks before staging: a `CLAUDE.md` link stages as
/// `.AGENTS.md.eunomia-*.tmp`), so it is a single file name with no
/// path separators; `token` is the caller's random token, which must
/// be [`TMP_TOKEN_LEN`] characters from [`TMP_TOKEN_ALPHABET`] — the
/// writer picks its characters from that alphabet, and every name this
/// function returns is recognized by [`is_app_tmp_file`] for the same
/// target name (pinned by tests), so a staging file can never be
/// written under a spelling the cleanup would not delete.
///
/// # Errors
///
/// [`TmpNameError::EmptyTargetName`] when `target_name` is empty (a
/// real file name never is), [`TmpNameError::InvalidTargetName`] when
/// `target_name` is not a single plain file name (a path separator,
/// NUL, or `.`/`..` — such a "name" would stage the file in another
/// folder, which the one-level sweep can never reach),
/// [`TmpNameError::InvalidToken`] when `token` is not 8 lowercase hex
/// characters — a malformed name would be created fine by the OS and
/// then live forever as debris the sweep never matches.
#[must_use = "the staging name must reach the file creation call"]
pub fn tmp_file_name(target_name: &str, token: &str) -> Result<String, TmpNameError> {
    if target_name.is_empty() {
        return Err(TmpNameError::EmptyTargetName);
    }
    if !is_plain_file_name(target_name) {
        return Err(TmpNameError::InvalidTargetName);
    }
    if !is_valid_tmp_token(token) {
        return Err(TmpNameError::InvalidToken);
    }
    Ok(format!(
        "{TMP_LEADING_DOT}{target_name}{TMP_MARKER}{token}{TMP_SUFFIX}"
    ))
}

/// Whether `token` is a valid staging-file token: exactly
/// [`TMP_TOKEN_LEN`] characters, all from [`TMP_TOKEN_ALPHABET`]
/// (lowercase hex, case-sensitive).
///
/// The writer validates each token candidate with this before paying
/// for the name; [`is_app_tmp_file`] validates with the same function,
/// so the two can never disagree about the grammar.
#[must_use = "an unvalidated token produces names the sweep never matches"]
pub fn is_valid_tmp_token(token: &str) -> bool {
    token.len() == TMP_TOKEN_LEN
        && token
            .bytes()
            .all(|byte| TMP_TOKEN_ALPHABET.as_bytes().contains(&byte))
}

/// Whether a folder entry named `name` is an app staging file for the
/// target file named `target_name` — the exact grammar of
/// [`tmp_file_name`], compared byte for byte (case-sensitive).
///
/// Total and pure: it reads nothing but the two strings, and every
/// non-match is "foreign" — the sweep deletes only what this matcher
/// accepts, so any near-miss shape (a missing leading dot, text after
/// the `.tmp` suffix, a token of the wrong length or alphabet, another
/// target's staging name) is structurally safe. A `target_name` that
/// is not a single plain file name (empty, separators, NUL, `.`/`..`)
/// matches NOTHING: the builder refuses to build a staging name for
/// such a target, so a contract-violating caller can never widen what
/// a sweep may delete.
#[must_use = "an ignored verdict deletes nothing or the wrong thing"]
pub fn is_app_tmp_file(name: &str, target_name: &str) -> bool {
    if !is_plain_file_name(target_name) {
        return false;
    }
    let Some(body) = name
        .strip_prefix(TMP_LEADING_DOT)
        .and_then(|rest| rest.strip_suffix(TMP_SUFFIX))
    else {
        return false;
    };
    let Some(token) = body
        .strip_prefix(target_name)
        .and_then(|rest| rest.strip_prefix(TMP_MARKER))
    else {
        return false;
    };
    is_valid_tmp_token(token)
}

/// What one sweep pass did.
///
/// `deleted` carries the full paths of the staging files this pass
/// removed (paths are log-safe: SEC-008 keeps file content out of logs,
/// paths are the log vocabulary); `failed` carries the entries the
/// sweep wanted to delete but could not, each with its OS error —
/// the caller logs them and the next sync's sweep retries.
#[derive(Debug, Default)]
pub struct TmpSweepReport {
    /// Staging files removed by this pass.
    pub deleted: Vec<PathBuf>,
    /// Staging files that matched and were stale, but could not be
    /// removed (or statted); each kept on disk, each with its OS error.
    pub failed: Vec<TmpDeleteFailure>,
}

/// One entry the sweep could not delete, with the OS error that
/// refused it.
#[derive(Debug)]
pub struct TmpDeleteFailure {
    /// The entry's full path (still on disk).
    pub path: PathBuf,
    /// The OS failure: `raw_os_error` and `ErrorKind` are the §10 log
    /// facts, the same shape `ErrorDetails::from_io_error` consumes.
    pub source: io::Error,
}

/// Why a sweep could not run at all.
#[derive(Debug)]
pub enum TmpSweepError {
    /// The folder cannot be listed (access denied on it or an ancestor,
    /// the path is a plain file, the drive dropped mid-run). A missing
    /// folder is deliberately NOT this: it is a successful no-op (see
    /// the module docs). The exact OS code rides in the source error.
    ListingFailed {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
}

impl TmpSweepError {
    /// The I/O error behind the failure, for the caller's log line.
    pub fn io_error(&self) -> &io::Error {
        match self {
            TmpSweepError::ListingFailed { source } => source,
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`), the §10
    /// log fact.
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().raw_os_error()
    }
}

impl fmt::Display for TmpSweepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TmpSweepError::ListingFailed { source } => {
                write!(
                    f,
                    "cannot list the target folder for stale tmp files: {source}"
                )
            }
        }
    }
}

impl std::error::Error for TmpSweepError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TmpSweepError::ListingFailed { source } => Some(source),
        }
    }
}

/// Why a staging-file name could not be built.
#[derive(Debug)]
pub enum TmpNameError {
    /// The target file name is empty — no real file name is.
    EmptyTargetName,
    /// The target file name is not a single plain file name: it
    /// carries a path separator or NUL, or is `.`/`..`. Such a name
    /// would stage the file in another folder — debris the one-level
    /// sweep can never reach.
    InvalidTargetName,
    /// The token is not [`TMP_TOKEN_LEN`] characters of
    /// [`TMP_TOKEN_ALPHABET`]; such a name would be debris the sweep
    /// never recognizes.
    InvalidToken,
}

impl fmt::Display for TmpNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TmpNameError::EmptyTargetName => {
                write!(f, "the target file name must not be empty")
            }
            TmpNameError::InvalidTargetName => write!(
                f,
                "the target file name must be a single file name, without path separators"
            ),
            TmpNameError::InvalidToken => write!(
                f,
                "the tmp token must be exactly {TMP_TOKEN_LEN} lowercase hexadecimal characters"
            ),
        }
    }
}

impl std::error::Error for TmpNameError {}

/// Deletes the target's stale staging files in `folder` (spec 9.4
/// phase 1 step 7): every entry matching [`is_app_tmp_file`] for
/// `target_name` whose own modification time is strictly older than
/// one hour before `now`.
///
/// `target_name` must be the same name the writer stages under — the
/// RESOLVED target file's name (9.5 step 1 resolves symlinks before
/// staging, so a `CLAUDE.md` link is swept as `AGENTS.md`); calling
/// the sweep with a different spelling than the writer used would
/// leave that spelling's debris for the other target's own sweep.
///
/// One listing level, never recursive (spec 9.5 stages in the same
/// folder as the target). Foreign entries and fresh staging files are
/// never touched; symlinks and directories with a tmp-shaped name are
/// skipped (the writer creates regular files only); per-file failures
/// are collected in the report and the sweep continues. A missing
/// folder is a successful no-op; a folder that cannot be listed is
/// [`TmpSweepError::ListingFailed`]. See the module docs for the full
/// failure, race and staleness semantics.
///
/// # Errors
///
/// [`TmpSweepError::ListingFailed`] when the folder itself cannot be
/// read — including a mid-run break of the directory stream, in which
/// case the deletions already performed are deliberately not reported
/// (the next sync's sweep retries whatever is left). Per-file trouble
/// never fails the call — it lands in [`TmpSweepReport::failed`]
/// instead.
pub fn remove_stale_tmp_files(
    folder: &Path,
    target_name: &str,
    now: SystemTime,
) -> Result<TmpSweepReport, TmpSweepError> {
    let mut report = TmpSweepReport::default();
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        // A folder that is not there has no stale files: converged,
        // and far better reported by the write phase than failed here
        // (module docs, "Failure semantics").
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(source) => return Err(TmpSweepError::ListingFailed { source }),
    };
    for entry in entries {
        // The directory stream itself broke (mid-run I/O error): the
        // listing cannot continue, so this is the same folder-level
        // failure as a refused open, not a per-file one. Deletions
        // already performed before the break are deliberately NOT
        // reported (the error carries no report) — the sweep is
        // idempotent housekeeping, and the next sync's sweep simply
        // retries whatever is left.
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => return Err(TmpSweepError::ListingFailed { source }),
        };
        let path = entry.path();
        // Non-UTF-8 names cannot be the writer's (the grammar is
        // `&str`-shaped): foreign by construction.
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !is_app_tmp_file(name, target_name) {
            continue;
        }
        // The entry's OWN metadata: symlinks are not traversed
        // (D-07/ALG-054 posture), so the staleness judgment never
        // depends on what a tmp-shaped link points at.
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                // Raced away between listing and stat: already gone,
                // the goal state holds.
                continue;
            }
            Err(source) => {
                report.failed.push(TmpDeleteFailure { path, source });
                continue;
            }
        };
        if !metadata.is_file() {
            // A directory or a symlink with a tmp-shaped name was not
            // created by the writer; foreign, left alone.
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            report.failed.push(TmpDeleteFailure {
                path,
                source: io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the platform cannot report the file's modification time",
                ),
            });
            continue;
        };
        // Strictly older than one hour; a future mtime (clock skew)
        // makes `duration_since` fail and stays.
        let stale = now
            .duration_since(modified)
            .is_ok_and(|age| age > STALE_AFTER);
        if !stale {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => report.deleted.push(path),
            // Raced away between stat and unlink: converged.
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => report.failed.push(TmpDeleteFailure { path, source }),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::print_stderr)] // test skip-notices and benchmark timings print deliberately

    use super::*;
    use std::error::Error as _;
    use std::time::Duration;
    use tempfile::TempDir;

    /// One hour plus one second, hardcoded from the spec (not derived
    /// from `STALE_AFTER`): the tests are oracles independent of the
    /// production constant (D-039's posture).
    const HOUR_PLUS: Duration = Duration::from_secs(60 * 60 + 1);

    /// One hour minus one minute: comfortably fresh, and safely away
    /// from the boundary the dedicated tests pin.
    const FRESH_AGE: Duration = Duration::from_secs(60 * 60 - 60);

    /// Writes a file and backdates its modification time, relative to
    /// the `now` the sweep will be handed.
    fn stale_file(dir: &Path, name: &str, now: SystemTime, age: Duration) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"stale").expect("writing the fixture file must work");
        set_mtime(&path, now - age);
        path
    }

    /// Backdates an existing fixture's modification time. The open mode
    /// follows the platform's needs: files open with write access because
    /// Windows' `SetFileTime` requires FILE_WRITE_ATTRIBUTES, which a
    /// read-only handle lacks (CI failure 44692465967, Windows leg:
    /// `Access is denied`, 2026-11-20); directory fixtures — which only
    /// the Linux-gated tests create, since Windows cannot open a directory
    /// by handle at all — keep a read handle, which is enough for
    /// `futimens` on Linux and cannot open for writing anywhere (EISDIR).
    fn set_mtime(path: &Path, mtime: SystemTime) {
        let open = if path.is_dir() {
            fs::File::open(path)
        } else {
            fs::OpenOptions::new().write(true).open(path)
        };
        let file = open.expect("opening the fixture for set_times must work");
        file.set_times(std::fs::FileTimes::new().set_modified(mtime))
            .expect("setting the fixture's modification time must work");
    }

    /// The fixture's owner uid is this process's effective uid (the
    /// fixture was created by it): uid 0 means the tests run as root,
    /// where mode bits are not enforced and an unwritable directory
    /// stays writable — the precondition of a permission test cannot
    /// be set up. The caller skips loudly instead of passing vacuously
    /// (house precedent: `paths.rs`).
    #[cfg(unix)]
    fn running_as_root_is_untestable(path: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }

    // ---- the name grammar: the matcher --------------------------------

    #[test]
    fn a_canonical_staging_name_matches_its_target() {
        assert!(is_app_tmp_file(
            ".AGENTS.md.eunomia-abcd1234.tmp",
            "AGENTS.md"
        ));
        assert!(is_app_tmp_file(
            ".demo.project.md.eunomia-0f1e2d3c.tmp",
            "demo.project.md"
        ));
    }

    #[test]
    fn every_name_the_builder_produces_is_matched_for_its_own_target() {
        // The builder/matcher symmetry the module promises: for valid
        // tokens the writer can never produce a spelling the sweep
        // would not recognize.
        let targets = ["AGENTS.md", "CLAUDE.md", "demo.project.md"];
        let tokens = ["00000000", "abcdef01", "9f8e7d6c"];
        for target in targets {
            for token in tokens {
                let name = tmp_file_name(target, token).expect("valid inputs must build");
                assert!(
                    is_app_tmp_file(&name, target),
                    "{name} must match its own target {target}"
                );
                let other = if target == "AGENTS.md" {
                    "CLAUDE.md"
                } else {
                    "AGENTS.md"
                };
                assert!(
                    !is_app_tmp_file(&name, other),
                    "{name} must not match a different target {other}"
                );
            }
        }
    }

    #[test]
    fn a_name_without_the_leading_dot_is_foreign() {
        assert!(!is_app_tmp_file(
            "AGENTS.md.eunomia-abcd1234.tmp",
            "AGENTS.md"
        ));
    }

    #[test]
    fn a_name_with_text_after_the_tmp_suffix_is_foreign() {
        assert!(!is_app_tmp_file(
            ".AGENTS.md.eunomia-abcd1234.tmp.bak",
            "AGENTS.md"
        ));
        assert!(!is_app_tmp_file(
            ".AGENTS.md.eunomia-abcd1234.tmpX",
            "AGENTS.md"
        ));
    }

    #[test]
    fn a_token_of_the_wrong_length_is_foreign() {
        assert!(!is_app_tmp_file(
            ".AGENTS.md.eunomia-abcd123.tmp",
            "AGENTS.md"
        ));
        assert!(!is_app_tmp_file(
            ".AGENTS.md.eunomia-abcd12345.tmp",
            "AGENTS.md"
        ));
        assert!(!is_app_tmp_file(".AGENTS.md.eunomia-.tmp", "AGENTS.md"));
    }

    #[test]
    fn a_token_with_a_non_hex_character_is_foreign() {
        assert!(!is_app_tmp_file(
            ".AGENTS.md.eunomia-abcd123g.tmp",
            "AGENTS.md"
        ));
        assert!(!is_app_tmp_file(
            ".AGENTS.md.eunomia-abcd12-4.tmp",
            "AGENTS.md"
        ));
    }

    #[test]
    fn an_uppercase_token_is_foreign() {
        // Case-sensitive, byte for byte: the writer only ever writes
        // lowercase hex, so an uppercase spelling is somebody else's
        // file and must stay deletable by no one but its owner.
        assert!(!is_app_tmp_file(
            ".AGENTS.md.eunomia-ABCD1234.tmp",
            "AGENTS.md"
        ));
    }

    #[test]
    fn another_targets_staging_name_is_foreign() {
        assert!(!is_app_tmp_file(".other.eunomia-abcd1234.tmp", "AGENTS.md"));
        assert!(!is_app_tmp_file(
            ".CLAUDE.md.eunomia-abcd1234.tmp",
            "AGENTS.md"
        ));
    }

    #[test]
    fn text_before_the_target_name_is_foreign() {
        // The grammar is prefix-exact: the entry name must start with
        // the target's own name right after the leading dot.
        assert!(!is_app_tmp_file(
            ".x.AGENTS.md.eunomia-abcd1234.tmp",
            "AGENTS.md"
        ));
    }

    #[test]
    fn the_token_alphabet_and_length_follow_the_repo_convention() {
        // Spec-derived oracles, hardcoded: "8 random characters"
        // (spec 9.5) pinned to the repo's lowercase-hex convention
        // (hash.rs, D-080). Widening the alphabet is a decision that
        // must revisit this module, not a silent edit.
        assert_eq!(TMP_TOKEN_LEN, 8);
        assert_eq!(TMP_TOKEN_ALPHABET, "0123456789abcdef");
        assert!(is_valid_tmp_token("abcd1234"));
        assert!(is_valid_tmp_token("00000000"));
        assert!(!is_valid_tmp_token(""));
        assert!(!is_valid_tmp_token("abc"));
        assert!(!is_valid_tmp_token("abcd12345"));
        assert!(!is_valid_tmp_token("ABCD1234"));
        assert!(!is_valid_tmp_token("abcd123g"));
    }

    // ---- the name builder ---------------------------------------------

    #[test]
    fn the_builder_spells_the_grammar_verbatim() {
        let name = tmp_file_name("AGENTS.md", "abcd1234").expect("valid inputs must build");
        assert_eq!(name, ".AGENTS.md.eunomia-abcd1234.tmp");
        let scheme_c =
            tmp_file_name("demo.project.md", "0f1e2d3c").expect("valid inputs must build");
        assert_eq!(scheme_c, ".demo.project.md.eunomia-0f1e2d3c.tmp");
    }

    #[test]
    fn the_builder_rejects_an_empty_target_name() {
        let err = tmp_file_name("", "abcd1234").expect_err("an empty target name must be refused");
        assert!(
            matches!(err, TmpNameError::EmptyTargetName),
            "must be EmptyTargetName, got: {err:?}"
        );
        assert_eq!(err.to_string(), "the target file name must not be empty");
    }

    #[test]
    fn the_builder_rejects_a_target_name_that_is_not_a_single_file_name() {
        // A separator would stage the file in another folder — debris
        // the one-level sweep can never reach; `.`/`..` and NUL are
        // the same hazard in their own shapes. (`std::path::is_separator`
        // is platform-defined: `/` on Unix, `/` and `\` on Windows —
        // the builder validates against the running platform's rules,
        // and the backslash case is pinned separately per platform.)
        for bad in ["sub/AGENTS.md", "AGENTS.md/sub", ".", "..", "a\u{0}b"] {
            let err =
                tmp_file_name(bad, "abcd1234").expect_err("a non-file-name target must be refused");
            assert!(
                matches!(err, TmpNameError::InvalidTargetName),
                "target {bad:?} must be InvalidTargetName, got: {err:?}"
            );
        }
        assert_eq!(
            tmp_file_name("sub/AGENTS.md", "abcd1234")
                .unwrap_err()
                .to_string(),
            "the target file name must be a single file name, without path separators"
        );
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_a_backslash_is_a_separator_in_the_target_name() {
        let err = tmp_file_name("sub\\AGENTS.md", "abcd1234")
            .expect_err("a backslash path must be refused on Windows");
        assert!(
            matches!(err, TmpNameError::InvalidTargetName),
            "must be InvalidTargetName, got: {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn on_unix_a_backslash_is_an_ordinary_file_name_character() {
        // `std::path::is_separator` is `/` only on Unix: `sub\AGENTS.md`
        // is a LEGAL single file name there, stages inside the same
        // folder, and the grammar keeps recognizing it — pinning that
        // the validation follows the platform, not a hardcoded set.
        let name = tmp_file_name("sub\\AGENTS.md", "abcd1234")
            .expect("a backslash name is a legal Unix file name");
        assert_eq!(name, ".sub\\AGENTS.md.eunomia-abcd1234.tmp");
        assert!(is_app_tmp_file(&name, "sub\\AGENTS.md"));
    }

    #[test]
    fn an_empty_target_name_matches_nothing() {
        // The builder refuses to build a staging name for any target
        // that is not a single plain file name, so the matcher must
        // not admit one either — the two halves of the grammar cannot
        // disagree about what the writer can spell.
        assert!(!is_app_tmp_file(".eunomia-abcd1234.tmp", ""));
        assert!(!is_app_tmp_file("..eunomia-abcd1234.tmp", ""));
        assert!(!is_app_tmp_file("AGENTS.md.eunomia-abcd1234.tmp", ""));
        assert!(!is_app_tmp_file("...eunomia-abcd1234.tmp", "."));
        assert!(!is_app_tmp_file(". ..eunomia-abcd1234.tmp", ".."));
    }

    #[test]
    fn the_builder_rejects_malformed_tokens() {
        for token in ["", "abc", "abcd12345", "ABCD1234", "abcd123g"] {
            let err =
                tmp_file_name("AGENTS.md", token).expect_err("a malformed token must be refused");
            assert!(
                matches!(err, TmpNameError::InvalidToken),
                "token {token:?} must be InvalidToken, got: {err:?}"
            );
        }
        assert_eq!(
            tmp_file_name("AGENTS.md", "short").unwrap_err().to_string(),
            "the tmp token must be exactly 8 lowercase hexadecimal characters"
        );
    }

    // ---- the sweep: real folders ---------------------------------------

    #[test]
    fn a_stale_staging_file_is_deleted() {
        // The card's acceptance (a) and the T-F2 scenario: the debris
        // of a killed process is gone on the next sweep.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let stale = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-abcd1234.tmp",
            now,
            HOUR_PLUS,
        );

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert_eq!(
            report.deleted,
            vec![stale.clone()],
            "the stale file must be deleted"
        );
        assert!(
            report.failed.is_empty(),
            "no failures expected, got: {:?}",
            report.failed
        );
        assert!(
            !stale.exists(),
            "the stale file must be gone from the folder"
        );
    }

    #[test]
    fn a_fresh_staging_file_stays() {
        // The card's acceptance (b): a staging file younger than the
        // window is somebody's live write, never a deletion candidate.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let fresh = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-abcd1234.tmp",
            now,
            FRESH_AGE,
        );

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(
            report.deleted.is_empty(),
            "a fresh file must not be deleted"
        );
        assert!(report.failed.is_empty());
        assert!(fresh.exists(), "the fresh staging file must stay");
    }

    #[test]
    fn foreign_files_stay_even_when_stale() {
        // The card's acceptance (c): every near-miss shape is foreign,
        // and staleness never rescues a foreign file into deletion.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let target = stale_file(dir.path(), "AGENTS.md", now, FRESH_AGE);
        let foreign: Vec<PathBuf> = [
            // another target's properly-shaped staging file, stale
            stale_file(
                dir.path(),
                ".CLAUDE.md.eunomia-abcd1234.tmp",
                now,
                HOUR_PLUS,
            ),
            // an arbitrary file that happens to carry the marker
            stale_file(dir.path(), ".other.eunomia-abcd1234.tmp", now, HOUR_PLUS),
            // wrong token length / alphabet / case
            stale_file(dir.path(), ".AGENTS.md.eunomia-abcd123.tmp", now, HOUR_PLUS),
            stale_file(
                dir.path(),
                ".AGENTS.md.eunomia-abcd123g.tmp",
                now,
                HOUR_PLUS,
            ),
            stale_file(
                dir.path(),
                ".AGENTS.md.eunomia-ABCD1234.tmp",
                now,
                HOUR_PLUS,
            ),
            // missing leading dot / text after the suffix
            stale_file(dir.path(), "AGENTS.md.eunomia-abcd1234.tmp", now, HOUR_PLUS),
            stale_file(
                dir.path(),
                ".AGENTS.md.eunomia-abcd1234.tmp.bak",
                now,
                HOUR_PLUS,
            ),
            // ordinary folder content
            stale_file(dir.path(), "notes.txt", now, HOUR_PLUS),
            stale_file(dir.path(), ".hidden", now, HOUR_PLUS),
        ]
        .into();

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(
            report.deleted.is_empty(),
            "no foreign file may be deleted, got: {:?}",
            report.deleted
        );
        assert!(
            report.failed.is_empty(),
            "foreign files are skips, not failures"
        );
        for path in foreign {
            assert!(path.exists(), "{} must stay", path.display());
        }
        assert!(target.exists(), "the target file itself must stay");
    }

    #[test]
    fn an_exactly_one_hour_old_file_is_kept() {
        // "Older than one hour" is strict: at exactly the boundary the
        // file is not older than the window, and deletion ambiguity
        // resolves toward not deleting.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let boundary = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-abcd1234.tmp",
            now,
            Duration::from_secs(60 * 60),
        );

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(report.deleted.is_empty(), "the boundary file must be kept");
        assert!(boundary.exists(), "a file exactly one hour old must stay");
    }

    #[test]
    fn a_file_one_second_past_the_hour_is_deleted() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let past = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-abcd1234.tmp",
            now,
            HOUR_PLUS,
        );

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert_eq!(report.deleted, vec![past.clone()]);
        assert!(!past.exists());
    }

    #[test]
    fn a_future_modification_time_keeps_the_file() {
        // Clock skew: an mtime ahead of `now` is never "older than the
        // window", whatever the wall clock says.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let skewed = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-abcd1234.tmp",
            now + Duration::from_secs(3_600),
            Duration::ZERO,
        );

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(
            report.deleted.is_empty(),
            "a future-mtime file must be kept"
        );
        assert!(report.failed.is_empty());
        assert!(skewed.exists());
    }

    #[test]
    fn multiple_stale_files_are_all_deleted() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let first = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-00000001.tmp",
            now,
            HOUR_PLUS,
        );
        let second = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-00000002.tmp",
            now,
            Duration::from_secs(2 * 60 * 60),
        );
        let third = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-00000003.tmp",
            now,
            Duration::from_secs(48 * 60 * 60),
        );
        let fresh = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-00000004.tmp",
            now,
            FRESH_AGE,
        );

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        let mut deleted = report.deleted.clone();
        deleted.sort();
        let mut expected = vec![first, second, third];
        expected.sort();
        assert_eq!(deleted, expected, "every stale file must be deleted");
        assert!(report.failed.is_empty());
        assert!(fresh.exists(), "the one fresh file must stay");
    }

    #[test]
    fn a_missing_folder_is_a_successful_no_op() {
        // Phase 1 step 7 is housekeeping between the availability check
        // and the write: a folder that is not there has nothing to
        // sweep, and the write phase reports the loss precisely.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let gone = dir.path().join("never-created");
        let now = SystemTime::now();

        let report = remove_stale_tmp_files(&gone, "AGENTS.md", now)
            .expect("a missing folder must not fail the sweep");

        assert!(report.deleted.is_empty());
        assert!(report.failed.is_empty());
    }

    #[test]
    fn a_degenerate_target_name_deletes_nothing() {
        // A contract-violating caller handing the sweep a target that
        // is not a single plain file name must not widen deletion:
        // even a contrived file shaped for that degenerate target
        // stays, because the builder could never have produced it.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let contrived = stale_file(dir.path(), "...eunomia-abcd1234.tmp", now, HOUR_PLUS);
        let real = stale_file(
            dir.path(),
            ".AGENTS.md.eunomia-abcd1234.tmp",
            now,
            FRESH_AGE,
        );

        for degenerate in ["", ".", "..", "sub/AGENTS.md"] {
            let report = remove_stale_tmp_files(dir.path(), degenerate, now)
                .expect("a degenerate target sweeps nothing, it does not fail");
            assert!(
                report.deleted.is_empty(),
                "target {degenerate:?} must delete nothing, deleted: {:?}",
                report.deleted
            );
            assert!(report.failed.is_empty());
        }

        assert!(contrived.exists(), "the contrived file must stay");
        assert!(
            real.exists(),
            "the real staging file must stay for its own target"
        );
    }

    #[test]
    fn an_empty_folder_produces_an_empty_report() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(report.deleted.is_empty());
        assert!(report.failed.is_empty());
    }

    #[test]
    fn a_staging_file_in_a_subfolder_is_not_swept() {
        // Spec 9.5 stages in the SAME folder as the target, so the
        // sweep is one listing level; deeper entries are not the
        // writer's, whatever their name.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).expect("creating the fixture subfolder must work");
        let deep = stale_file(&nested, ".AGENTS.md.eunomia-abcd1234.tmp", now, HOUR_PLUS);

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(report.deleted.is_empty());
        assert!(deep.exists(), "a subfolder entry must never be swept");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_listed_is_a_typed_listing_failure() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let folder = dir.path().join("locked");
        fs::create_dir(&folder).expect("creating the fixture folder must work");
        if running_as_root_is_untestable(&folder) {
            eprintln!(
                "skipping a_folder_that_cannot_be_listed_is_a_typed_listing_failure: \
                 running as root — mode bits are not enforced, the \
                 unreadable-folder precondition cannot be set up"
            );
            return;
        }
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o000))
            .expect("removing all mode bits must work");
        let result = remove_stale_tmp_files(&folder, "AGENTS.md", SystemTime::now());
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("an unlistable folder must be a ListingFailed error");
        assert!(
            matches!(&err, TmpSweepError::ListingFailed { .. }),
            "a 000-mode folder must be ListingFailed, got: {err:?}"
        );
        assert_eq!(
            err.os_code(),
            Some(13),
            "EACCES is the OS code the log line carries for denied access"
        );
        assert!(
            err.to_string()
                .starts_with("cannot list the target folder for stale tmp files: ")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_permission_failure_is_collected_and_the_sweep_continues() {
        use std::os::unix::fs::PermissionsExt;

        // r-x on the folder: listable, but every unlink inside fails.
        // TWO stale files are the point: an implementation that
        // aborted at the first failure would report one — collecting
        // both proves the sweep continued past it.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let folder = dir.path().join("locked");
        fs::create_dir(&folder).expect("creating the fixture folder must work");
        let first = stale_file(&folder, ".AGENTS.md.eunomia-00000001.tmp", now, HOUR_PLUS);
        let second = stale_file(&folder, ".AGENTS.md.eunomia-00000002.tmp", now, HOUR_PLUS);
        let fresh = stale_file(&folder, ".AGENTS.md.eunomia-00000003.tmp", now, FRESH_AGE);
        let foreign = stale_file(&folder, "notes.txt", now, HOUR_PLUS);
        if running_as_root_is_untestable(&folder) {
            eprintln!(
                "skipping a_permission_failure_is_collected_and_the_sweep_continues: \
                 running as root — mode bits are not enforced, the \
                 unwritable-folder precondition cannot be set up"
            );
            return;
        }

        fs::set_permissions(&folder, fs::Permissions::from_mode(0o500))
            .expect("removing the write bits must work");
        let result = remove_stale_tmp_files(&folder, "AGENTS.md", now);
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let report = result.expect("per-file failures are collected, not fatal");
        assert!(
            report.deleted.is_empty(),
            "nothing can be deleted in an r-x folder"
        );
        let mut failed_paths: Vec<PathBuf> = report.failed.iter().map(|f| f.path.clone()).collect();
        failed_paths.sort();
        let mut expected = vec![first.clone(), second.clone()];
        expected.sort();
        assert_eq!(
            failed_paths, expected,
            "BOTH stale files must be attempted: the failure must not abort the sweep"
        );
        for failure in &report.failed {
            assert_eq!(
                failure.source.raw_os_error(),
                Some(13),
                "EACCES is the OS code each collected failure carries"
            );
        }
        assert!(
            first.exists() && second.exists(),
            "the refused files must stay"
        );
        assert!(fresh.exists(), "the fresh file must stay");
        assert!(foreign.exists(), "the foreign file must stay");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_staging_name_is_skipped_and_never_followed() {
        // A tmp-shaped SYMLINK was not created by the writer (which
        // makes regular files only): it stays, and its staleness is
        // judged on the LINK's own metadata. The target here is an
        // old foreign file — an implementation that followed the link
        // would read the target's mtime, call it stale, and delete
        // the LINK; this test fails exactly there.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let target = stale_file(dir.path(), "old-notes.txt", now, HOUR_PLUS);
        let link = dir.path().join(".AGENTS.md.eunomia-abcd1234.tmp");
        std::os::unix::fs::symlink(&target, &link).expect("creating the symlink must work");
        let dangling = dir.path().join(".AGENTS.md.eunomia-9f8e7d6c.tmp");
        std::os::unix::fs::symlink(dir.path().join("nowhere"), &dangling)
            .expect("creating the dangling symlink must work");

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(
            report.deleted.is_empty(),
            "symlinks are never deletion candidates"
        );
        assert!(report.failed.is_empty());
        assert!(link.is_symlink(), "the tmp-shaped link must stay");
        assert!(target.exists(), "the link's target must stay");
        assert!(
            dangling.is_symlink(),
            "a dangling tmp-shaped link must stay"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_with_a_staging_name_is_kept_even_when_stale() {
        // The writer never creates directories: a directory with a
        // tmp-shaped name is foreign even at a stale mtime — and
        // deleting it would mean a recursive removal of content this
        // module does not own.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let shaped = dir.path().join(".AGENTS.md.eunomia-abcd1234.tmp");
        fs::create_dir(&shaped).expect("creating the fixture directory must work");
        fs::write(shaped.join("payload.txt"), b"not ours")
            .expect("writing into the fixture directory must work");
        set_mtime(&shaped, now - HOUR_PLUS);

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(report.deleted.is_empty());
        assert!(report.failed.is_empty());
        assert!(shaped.is_dir(), "a tmp-shaped directory must stay");
        assert!(
            shaped.join("payload.txt").exists(),
            "its content must be untouched"
        );
    }

    // Linux-only, and deliberately so: the fixture creates an entry whose
    // name is invalid UTF-8, which only Linux filesystems accept — APFS
    // (macOS) and NTFS reject such names outright with EILSEQ (CI failure
    // 44691597845, macOS-15 leg: `Illegal byte sequence` at fixture setup,
    // 2026-11-20). On those platforms the swept-world state the test pins
    // cannot exist, so the non-deletion guarantee is untestable there and
    // the sweep's name matcher never sees such bytes to begin with.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_non_utf8_entry_name_is_never_deleted() {
        // The grammar is `&str`-shaped; an entry whose name is not
        // valid UTF-8 cannot be the writer's and must never reach the
        // deletion path, however stale it looks.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let now = SystemTime::now();
        let weird = dir
            .path()
            .join(OsStr::from_bytes(b".AGENTS.md.eunomia-a\xff\xfe34.tmp"));
        fs::write(&weird, b"foreign bytes").expect("writing the fixture file must work");
        set_mtime(&weird, now - HOUR_PLUS);

        let report = remove_stale_tmp_files(dir.path(), "AGENTS.md", now)
            .expect("an existing folder must sweep");

        assert!(report.deleted.is_empty());
        assert!(report.failed.is_empty());
        assert!(weird.exists(), "a non-UTF-8 name must stay");
    }

    #[test]
    fn the_sweep_error_display_and_source_chain_follow_the_contract() {
        // The command layer logs the Display text; these assertions pin
        // the wording (lowercase, no trailing punctuation — the house
        // style) and the source() chain without pinning std's strerror
        // table.
        let err = TmpSweepError::ListingFailed {
            source: io::Error::from_raw_os_error(13),
        };
        assert!(
            err.to_string()
                .starts_with("cannot list the target folder for stale tmp files: ")
        );
        assert_eq!(err.io_error().raw_os_error(), Some(13));
        assert!(err.source().is_some());
    }
}
