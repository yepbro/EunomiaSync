//! Atomic file writes: the seven-step pipeline of spec 9.5.
//!
//! One entry point, [`write_target_file`]: it replaces a target file's
//! content so that a reader (or a crash, OUT-007) only ever sees the
//! whole old file or the whole new one, never a partial mix. The
//! pipeline is spec 9.5 verbatim:
//!
//! 1. **Resolve the target path** — the CALLER's step (§9.5 step 1,
//!    `paths::resolve_target_path`, DECISIONS.md D-092): this module
//!    receives the resolved write path and never resolves anything
//!    itself. A `CLAUDE.md` link has therefore already become the real
//!    `AGENTS.md` path — renaming over the resolved path keeps the
//!    link (T-F6), and the staging file is spelled with the resolved
//!    name (D-088's coordination rule), which this module re-derives
//!    from the path's own file name (lossy text, the D-092 form; for a
//!    resolved target that is `TargetPath::file_name()` by
//!    construction).
//! 2. **Stage** — a temporary file in the SAME folder as the target
//!    (same folder = same volume = the final swap is a rename, never a
//!    cross-device copy), named `.<name>.eunomia-<token>.tmp` by
//!    `temp_files::tmp_file_name` (the one grammar the stale-file
//!    sweep matches, D-088) and created with `create_new` — a name
//!    collision is a typed error under a fresh token, never a
//!    clobber. The rights of the existing target are copied onto the
//!    staging file (Unix mode, §9.5 step 2).
//! 3. **Write and settle** — all bytes, then `sync_all`, so the
//!    verification judges bytes the disk has accepted.
//! 4. **Replace** — Unix `rename(2)`; Windows `MoveFileExW` with
//!    `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH` (§9.5
//!    step 4). A locked Windows target (sharing violation or access
//!    denied) is retried after 100, 300 and 900 ms — three retries
//!    after the initial attempt, then
//!    [`AtomicWriteError::FileLocked`] (`E_FILE_LOCKED`); any other
//!    refusal fails immediately.
//! 5. **Sync the folder entry** — `fsync` on the parent folder, Unix
//!    only (§9.5 step 5 pins the platform; std cannot open a directory
//!    on Windows, and `MOVEFILE_WRITE_THROUGH` already flushes the
//!    rename there).
//! 6. **Verify** — re-open the target and compare its SHA-256 with the
//!    digest of the bytes that were handed in (§9.5 step 6). The check
//!    reads what is ON DISK, not the input buffer: content that
//!    changed or corrupted after the swap is caught as
//!    [`AtomicWriteError::VerifyFailed`].
//! 7. **Clean up** — on an error at any step the staging file is
//!    deleted (§9.5 step 7). After a successful replace there is no
//!    staging file left to delete; for the two post-replace steps
//!    (folder sync, verify) the file therefore stays WHOLLY the new
//!    content — old-or-new, never partial — and is never deleted by
//!    this module: it is the user's data, and §10's remedy for a
//!    failed verification is the backup, not an empty path.
//!
//! # Errors (§10)
//!
//! The typed variants map onto the §10 codes the command layer logs
//! (four codes, five variants — [`AtomicWriteError::NameFailed`]
//! folds into `E_WRITE_FAILED`; the D-062 posture: the code mapping
//! itself belongs to the app layer):
//!
//! - [`AtomicWriteError::DiskFull`] — `E_DISK_FULL`, "no space … on
//!   the project volume during the write". Two sources feed it: the
//!   pre-write check (below) and the OS refusing a write-path step
//!   with `ENOSPC` (`io::ErrorKind::StorageFull`).
//! - [`AtomicWriteError::WriteFailed`] — `E_WRITE_FAILED` for every
//!   other refusal of a pipeline step (staging create, mode copy,
//!   write/sync, replace, folder sync), each carrying which step
//!   failed and the raw OS error.
//! - [`AtomicWriteError::VerifyFailed`] — `E_VERIFY_FAILED`, the
//!   re-read digest mismatch or an unreadable target.
//! - [`AtomicWriteError::NameFailed`] — also `E_WRITE_FAILED`: a
//!   contract violation (the caller handed a path whose file name
//!   cannot shape a staging name — the resolved targets of D-092
//!   never can), kept as its own variant so the refusal is a typed
//!   fact, not an invented OS error. The command layer logs it with
//!   the write-failed code; no §10 row of its own exists.
//!
//! - [`AtomicWriteError::FileLocked`] — `E_FILE_LOCKED`, Windows
//!   only: §9.5 step 4's locked-replace endgame. The Windows replace
//!   arm retries a sharing violation (`ERROR_SHARING_VIOLATION`, 32)
//!   or access denial (`ERROR_ACCESS_DENIED`, 5) after 100, 300 and
//!   900 ms — one initial attempt plus three retries; a target still
//!   locked after the third retry fails typed here. The Unix arm
//!   never produces the variant: §10 pins the row to Windows, and
//!   rename(2) has no sharing violations to retry.
//!
//! The retry loop lives in the one seam D-095 established: every
//! platform funnels the replace through [`replace_target`], and the
//! Windows arm wraps the raw `MoveFileExW` attempt in the §9.5
//! schedule. Its refusal type ([`ReplaceRefusal`]) distinguishes the
//! exhausted lock from every other OS error, and the pipeline's
//! step-4 failure path classifies it — no new plumbing beyond that
//! one call site.
//!
//! # The pre-write space check
//!
//! §10's `E_DISK_FULL` row names the project volume during the write,
//! so before anything is created the caller-supplied free-space source
//! is consulted once — the same seam `free_space::check_backup_space`
//! established (D-089): production passes
//! `|| free_space::free_bytes(folder)`, tests substitute a plain
//! number. The demand is exactly the content's byte count: §9.4's
//! "plus a 20% margin" arithmetic belongs to the BACKUP volume check,
//! and no margin is pinned for the write. Only the definitive
//! shortage verdict aborts: a FAILED probe does not block the write
//! (a dropped drive then reports its own honest failure from the
//! write itself, ERR-003's posture) — the write, not the probe, is
//! the authority on whether the bytes can be written. The exact
//! boundary passes (`free == required`), inheriting
//! `free_space::check_space`'s strict-less-than ruling.
//!
//! # The staging token
//!
//! The spec asks for "8 random characters"; the alphabet is pinned by
//! D-088 (lowercase hex, `TMP_TOKEN_ALPHABET`). The entropy source is
//! the wall clock's nanoseconds, the process id and a process-wide
//! counter, digested through `eunomia-core::hash` — no `rand`
//! dependency (D-017's allow-list; the token needs uniqueness, not
//! unpredictability: `create_new` makes a collision an error under
//! the next token, never corruption, and the sweep recognizes any
//! grammar-conforming name). A collision retries with a fresh token,
//! at most [`TOKEN_ATTEMPTS`] times, then fails typed.
//!
//! # Crash safety and idempotence (OUT-007, ERR-002)
//!
//! The invariant is old-or-new: before the rename the target is the
//! untouched old file and the staging file is private debris; the
//! rename is the single atomic transition; after it the target is
//! wholly the new content. A kill at any instant leaves at most a
//! staging file under the sweep's grammar (`temp_files`, T-F2) — never
//! a half-written target. Re-running the write converges: same
//! content, fresh token, same end state (a sync is idempotent, §10).
//!
//! # Races are honest (TOCTOU)
//!
//! The pipeline is a sequence of OS calls over a live filesystem; no
//! snapshot, no pretended atomicity between the steps. The space
//! verdict is a fact about the moment it was taken; the mode copy
//! reads the target's metadata at its own instant; the verification
//! re-reads whatever the target holds after the swap — an external
//! editor writing between the rename and the verification surfaces as
//! a digest mismatch (`E_VERIFY_FAILED`), which is the honest report
//! of "the file does not hold what this run wrote".
//!
//! # Testing plan
//!
//! Colocated tests over real temporary directories drive the public
//! function only; failures at each pipeline step are injected through
//! the crate's fault-injection module (spec 15.1's "failure points
//! in `fs`": `fault_injection`, compiled for `cfg(test)` and for the
//! non-default `fault-injection` feature — the pause and override
//! points of T-069 live there too), so the cleanup paths run in the
//! production code under test. The
//! battery: the new-file and overwrite happy paths against the FIPS
//! "abc" oracle, the empty write, the mode copy (Unix), the
//! disk-full shortage/exact-boundary/probe-failure decisions with a
//! substituted source, `ENOSPC` mapped to `E_DISK_FULL`, a failure at
//! EVERY step leaving the original intact and the staging file
//! removed (or the refusal carried), the refused cleanup, the digest
//! check catching a corrupted settled file (it reads the disk, not
//! the buffer), token collision retry and exhaustion, the missing
//! folder and degenerate target shapes, the caller-side symlink
//! resolution keeping the link (T-F6's shape), and the error contract
//! (Display, source chain, OS codes). The Windows lock retries
//! (§9.5 step 4) run at three levels: mechanics tests against the
//! retry loop itself (injected `ERROR_SHARING_VIOLATION` /
//! `ERROR_ACCESS_DENIED` refusals and a recording clock pin the
//! 100/300/900 ms schedule, the attempt count and the
//! non-lock-refusal exits — no real sleeping anywhere), pipeline
//! tests through a simulated locked replace driving the SAME loop
//! (the typed `E_FILE_LOCKED` refusal, the staging cleanup and the
//! intact original run on the Linux legs), and one real Windows test
//! holding the target open without `FILE_SHARE_DELETE` (T-F8's
//! editor shape, on the CI Windows leg).

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use eunomia_core::hash::Sha256Hex;

#[cfg(any(test, feature = "fault-injection"))]
use crate::fault_injection;
use crate::free_space::{FreeSpaceError, check_space};
use crate::temp_files::{TMP_TOKEN_LEN, TmpNameError, tmp_file_name};

/// How many distinct staging names one write may try before giving
/// up: a `create_new` collision (same folder, same token — a retried
/// run racing a debris file) tries the next token; three attempts
/// bound the loop, and the last refusal is reported typed. D-088's
/// ruling: a collision is an error and a retry, never corruption.
const TOKEN_ATTEMPTS: usize = 3;

/// The read chunk of the verification pass: 64 KiB, the same size
/// `backup` streams with — the target is re-read in fixed chunks and
/// never loaded whole into memory.
const VERIFY_CHUNK_BYTES: usize = 64 * 1024;

/// What a successful [`write_target_file`] proved about the file on
/// disk: the target now holds exactly `size` bytes whose SHA-256 is
/// `sha256_hex` — the digest was measured by re-reading the settled
/// file (§9.5 step 6), not by trusting the write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrittenFile {
    /// The target path that was replaced (the resolved write path the
    /// caller handed in).
    pub path: PathBuf,
    /// The content's size in bytes.
    pub size: u64,
    /// The content's SHA-256, 64 lowercase hex characters — the
    /// expected digest the re-read check confirmed against the disk.
    pub sha256_hex: String,
}

/// Atomically replaces `target`'s content with `contents` following
/// the §9.5 pipeline (see the module docs): stage in the same folder
/// under the D-088 grammar with `create_new`, copy the existing
/// target's Unix mode onto the staging file, write and `sync_all`,
/// replace the target in one rename, `fsync` the parent folder (Unix),
/// then re-read the target and verify its SHA-256.
///
/// `target` is the RESOLVED write path (§9.5 step 1 is the caller's —
/// `paths::resolve_target_path`, D-092); this module renames over
/// exactly the path it is given. `free_space` is the D-089 seam: the
/// free bytes of the target's volume, consulted once before anything
/// is created (production passes `|| free_space::free_bytes(folder)`,
/// tests substitute a number); only a definitive shortage aborts the
/// write as [`AtomicWriteError::DiskFull`] — a failed probe does not
/// block it.
///
/// The operation is idempotent: re-running it with the same content
/// converges to the same file (a fresh staging token each time), so a
/// retried sync after an interrupted run is safe (§10).
///
/// # Errors
///
/// - [`AtomicWriteError::NameFailed`]: `target`'s file name cannot
///   shape a staging name (not a single plain file name) — nothing
///   was created.
/// - [`AtomicWriteError::DiskFull`]: the volume of `target` cannot
///   hold the content (the pre-write check's shortage verdict), or a
///   write-path step was refused with no space (`ENOSPC` /
///   `StorageFull`). Before the staging file exists nothing was
///   touched; after it, the staging file was removed unless even the
///   removal failed (`cleanup`).
/// - [`AtomicWriteError::WriteFailed`]: any other refusal of a
///   pipeline step — staging create (including a missing folder, the
///   T-F7 shape, and exhausted token collisions), mode copy,
///   write/sync, replace, folder sync. The staging file was removed
///   unless the removal was refused (`cleanup`); the target is
///   untouched for every step up to and including the replace, and
///   wholly the new content for the folder sync (the rename already
///   happened — old-or-new, never partial).
/// - [`AtomicWriteError::FileLocked`]: Windows only — the replace
///   was refused with a sharing violation or access denied through
///   all of §9.5 step 4's retries (100/300/900 ms). The staging file
///   was removed unless the removal was refused (`cleanup`); the
///   target keeps its old content whole.
/// - [`AtomicWriteError::VerifyFailed`]: the re-read digest did not
///   match, or the target could not be read back. The file holds the
///   new content (unverified); it is deliberately NOT deleted — §10's
///   remedy is the backup copy.
pub fn write_target_file<S>(
    target: &Path,
    contents: &[u8],
    free_space: S,
) -> Result<WrittenFile, AtomicWriteError>
where
    S: FnOnce() -> Result<u64, FreeSpaceError>,
{
    // A bare relative name ("AGENTS.md") has no folder to stage in —
    // the resolved targets of D-092 are absolute; refuse the shape
    // instead of staging into the process's working directory.
    let Some(folder) = target.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Err(AtomicWriteError::WriteFailed(WriteFailed {
            step: WriteStep::TempCreate,
            path: target.to_owned(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "the target path has no parent folder to stage in",
            ),
            cleanup: None,
        }));
    };
    // The staging name keys on the target's own file name, lossy text
    // (the D-092 form — `TargetPath::file_name()` for a resolved
    // target).
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());

    // The seam between the backup phase (§9.4 phase 2) and the write
    // (§9.5), under the fault-injection cfg: the caller's backups
    // are made and nothing of this write has touched the disk yet —
    // an armed pause parks here for the between-phases kill tests
    // (T-F3's shape at single-file granularity, spec 15.1's
    // "force-killing the process between phases").
    #[cfg(any(test, feature = "fault-injection"))]
    fault_injection::pause_if_armed(fault_injection::PausePoint::WriteEntry);

    // §10's write-time E_DISK_FULL: only the definitive shortage
    // aborts. A failed probe falls through — the write itself reports
    // a dropped drive far more precisely (ERR-003). An armed
    // fault-injection override replaces what a SUCCESSFUL probe
    // observed (never the decision, which stays `check_space`'s, and
    // never a failed probe): 15.1's "out of space" row, drivable by
    // integration tests that cannot reach into the caller-supplied
    // closure.
    let required = u64::try_from(contents.len()).unwrap_or(u64::MAX);
    #[cfg(any(test, feature = "fault-injection"))]
    let probe_outcome = fault_injection::override_free_space(free_space());
    #[cfg(not(any(test, feature = "fault-injection")))]
    let probe_outcome = free_space();
    if let Ok(reported_free) = probe_outcome
        && let Err(FreeSpaceError::NotEnoughSpace {
            free,
            required: needed,
        }) = check_space(reported_free, required)
    {
        return Err(AtomicWriteError::DiskFull(DiskFullFailure {
            path: target.to_owned(),
            step: None,
            free: Some(free),
            required: needed,
            source: None,
            cleanup: None,
        }));
    }

    // The expected digest of the §9.5 step-6 check: computed from the
    // handed-in bytes, so the check can only pass when the disk holds
    // exactly what this run meant to write.
    let expected_sha256_hex = sha256_hex_of(contents);

    // Step 2: stage. `create_new` under the sweep's grammar; a name
    // collision retries under a fresh token, at most TOKEN_ATTEMPTS
    // times in total.
    let (staging_file, staging_path) = create_staging_file(folder, file_name.as_deref(), target)?;

    // Step 2 (Unix): the existing target's mode rides along — the
    // replaced file keeps the rights its owner gave it.
    #[cfg(unix)]
    if let Err(failure) = copy_existing_mode(target, &staging_path) {
        // The stated handle discipline: close before the error-path
        // removal (the removal runs inside `abort`).
        drop(staging_file);
        return Err(abort(failure, target, &staging_path, required));
    }

    // Step 3: write every byte, then settle them.
    #[cfg(any(test, feature = "fault-injection"))]
    let write_outcome =
        match fault_injection::inject_atomic(fault_injection::AtomicStep::WriteSync, target) {
            Some(source) => Err(source),
            None => write_and_sync(&staging_file, contents),
        };
    #[cfg(not(any(test, feature = "fault-injection")))]
    let write_outcome = write_and_sync(&staging_file, contents);
    // Close the handle BEFORE the rename and the error-path removal:
    // an open handle is a Windows sharing violation waiting to happen
    // (the backup module's precedent).
    drop(staging_file);
    if let Err(source) = write_outcome {
        return Err(abort(
            WriteFailed {
                step: WriteStep::WriteSync,
                path: staging_path.clone(),
                source,
                cleanup: None,
            },
            target,
            &staging_path,
            required,
        ));
    }

    // T-F2's kill window (spec 15.2), under the fault-injection cfg:
    // the staging file is written and settled, the rename has not
    // happened — an armed pause parks here (spec 15.1's
    // "force-killing the process inside a write"), so a kill test
    // can take the process down inside the window and check what is
    // on disk: the untouched original plus exactly one sweepable
    // staging file.
    #[cfg(any(test, feature = "fault-injection"))]
    fault_injection::pause_if_armed(fault_injection::PausePoint::StagingSettled);

    // Step 4: the single atomic transition. Failures here leave the
    // old file intact and the staging file removable; on Windows the
    // arm has already ridden §9.5's lock-retry schedule, so an
    // exhausted lock classifies as E_FILE_LOCKED (abort_replace).
    if let Err(refusal) = replace_target(&staging_path, target) {
        return Err(abort_replace(refusal, target, &staging_path, required));
    }

    // Step 5 (Unix): make the folder entry itself durable. The rename
    // already happened, so a failure reports the step with the file
    // wholly new — and the staging path no longer exists, which the
    // removal treats as converged.
    #[cfg(unix)]
    if let Err(source) = sync_parent_folder(folder, target) {
        return Err(abort(
            WriteFailed {
                step: WriteStep::DirSync,
                path: folder.to_owned(),
                source,
                cleanup: None,
            },
            target,
            &staging_path,
            required,
        ));
    }

    // Step 6: verify what the disk holds, not what the buffer held.
    #[cfg(any(test, feature = "fault-injection"))]
    fault_injection::corrupt_target_if_armed(target);
    match read_back_digest(target) {
        Ok(actual_sha256_hex) if actual_sha256_hex == expected_sha256_hex => {}
        Ok(actual_sha256_hex) => {
            return Err(AtomicWriteError::VerifyFailed(VerifyFailed {
                path: target.to_owned(),
                expected_sha256_hex,
                mismatch: VerifyMismatch::Digest { actual_sha256_hex },
            }));
        }
        Err(source) => {
            return Err(AtomicWriteError::VerifyFailed(VerifyFailed {
                path: target.to_owned(),
                expected_sha256_hex,
                mismatch: VerifyMismatch::Unreadable { source },
            }));
        }
    }

    Ok(WrittenFile {
        path: target.to_owned(),
        size: required,
        sha256_hex: expected_sha256_hex,
    })
}

/// Step 2's staging create: tries up to [`TOKEN_ATTEMPTS`] distinct
/// tokens under the D-088 grammar, `create_new` every time — an
/// existing file under a candidate name is a collision (a retried run
/// racing that run's debris), answered with the next token, never a
/// clobber. Any other refusal, or an exhausted budget, fails typed
/// with nothing left behind.
fn create_staging_file(
    folder: &Path,
    file_name: Option<&str>,
    target: &Path,
) -> Result<(File, PathBuf), AtomicWriteError> {
    let Some(file_name) = file_name else {
        return Err(AtomicWriteError::WriteFailed(WriteFailed {
            step: WriteStep::TempCreate,
            path: target.to_owned(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "the target path does not end in a file name",
            ),
            cleanup: None,
        }));
    };
    let mut last_collision: Option<(PathBuf, io::Error)> = None;
    for _ in 0..TOKEN_ATTEMPTS {
        let token = next_token();
        // The name is the sweep's grammar or nothing: the builder
        // refuses anything a one-level sweep could not clean (and a
        // refused name does not depend on the token, so the first
        // attempt settles it).
        let name = tmp_file_name(file_name, &token)
            .map_err(|source| AtomicWriteError::NameFailed { source })?;
        let path = folder.join(name);
        #[cfg(any(test, feature = "fault-injection"))]
        if let Some(source) =
            fault_injection::inject_atomic(fault_injection::AtomicStep::TempCreate, target)
        {
            return Err(AtomicWriteError::WriteFailed(WriteFailed {
                step: WriteStep::TempCreate,
                path,
                source,
                cleanup: None,
            }));
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            // Somebody holds this token's name — a collision, not a
            // conflict with the target: try the next token.
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some((path, source));
            }
            Err(source) => {
                return Err(AtomicWriteError::WriteFailed(WriteFailed {
                    step: WriteStep::TempCreate,
                    path,
                    source,
                    cleanup: None,
                }));
            }
        }
    }
    // The budget ran out over collisions: the last attempted name and
    // its OS refusal are the facts. (The expect is an invariant, not
    // an input check: the loop ran at least once, and only collisions
    // reach this point.)
    let (path, source) = last_collision.expect("an exhausted token budget has seen a collision");
    Err(AtomicWriteError::WriteFailed(WriteFailed {
        step: WriteStep::TempCreate,
        path,
        source,
        cleanup: None,
    }))
}

/// Step 3: write every byte and `sync_all` — the bytes the later
/// verification judges must be bytes the disk has accepted, not page
/// cache promises.
fn write_and_sync(mut staging_file: &File, contents: &[u8]) -> io::Result<()> {
    staging_file.write_all(contents)?;
    staging_file.sync_all()
}

/// The next staging token: 8 lowercase hex characters. Production
/// digests the wall clock, the process id and a process-wide counter
/// through core's SHA-256 (see the module docs for why no `rand`);
/// tests may fix the sequence to pin exact names.
fn next_token() -> String {
    #[cfg(any(test, feature = "fault-injection"))]
    {
        if let Some(token) = fault_injection::next_fixed_token() {
            return token;
        }
    }
    production_token()
}

/// The production token source: uniqueness without a `rand`
/// dependency — nanoseconds (monotonic enough across a process's
/// life), the pid (separates concurrent app instances) and a
/// process-wide counter (separates back-to-back calls within one
/// nanosecond) digested into the D-088 alphabet.
fn production_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|passed| u64::try_from(passed.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    let pid = u64::from(std::process::id());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);

    let mut hasher = Sha256Hex::new();
    hasher.update(&nanos.to_le_bytes());
    hasher.update(&pid.to_le_bytes());
    hasher.update(&sequence.to_le_bytes());
    // Lowercase hex is one byte per character: the byte range at 8 is
    // exactly the first 8 characters.
    hasher.finalize_hex()[..TMP_TOKEN_LEN].to_owned()
}

/// Step 2's Unix mode copy: the replaced file keeps the rights of the
/// file it replaces (`chmod`, which no umask applies to — the copied
/// mode lands exactly). A target that does not exist is a first sync's
/// creation: nothing to copy, the platform default stands (the BKP
/// specs pin no rights for created targets either). The copy is the
/// full `& 0o7777` mask, faithfully — with the kernel's own limits,
/// not the module's: the Linux killpriv policy strips setuid/setgid
/// from any file at write time (a security guarantee no writer can
/// bypass — the staging file is written after the chmod), while the
/// sticky bit and the plain rwx bits ride along.
#[cfg(unix)]
fn copy_existing_mode(target: &Path, staging_path: &Path) -> Result<(), WriteFailed> {
    use std::os::unix::fs::PermissionsExt;

    let mode = match fs::metadata(target) {
        Ok(metadata) => metadata.permissions().mode() & 0o7777,
        // A vanished or never-existing target: a first sync creates
        // the file; the default mode is correct (and a target that
        // vanished mid-run fails honestly at the replace instead).
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(WriteFailed {
                step: WriteStep::ModeCopy,
                path: staging_path.to_owned(),
                source,
                cleanup: None,
            });
        }
    };
    #[cfg(any(test, feature = "fault-injection"))]
    if let Some(source) =
        fault_injection::inject_atomic(fault_injection::AtomicStep::ModeCopy, target)
    {
        return Err(WriteFailed {
            step: WriteStep::ModeCopy,
            path: staging_path.to_owned(),
            source,
            cleanup: None,
        });
    }
    fs::set_permissions(staging_path, fs::Permissions::from_mode(mode)).map_err(|source| {
        WriteFailed {
            step: WriteStep::ModeCopy,
            path: staging_path.to_owned(),
            source,
            cleanup: None,
        }
    })
}

/// How §9.5 step 4 refused: the plain OS error of a non-lock failure
/// — and of EVERY failure the Unix arm produces (the retry discipline
/// is Windows-only) — or the lock that outlasted the whole §9.5
/// schedule. The distinction is §10's: `E_WRITE_FAILED` vs
/// `E_FILE_LOCKED`.
#[derive(Debug)]
enum ReplaceRefusal {
    /// One OS refusal, unretried: any non-lock error, and every Unix
    /// replace error whatever its raw code.
    Os(io::Error),
    /// Windows: the target stayed locked (sharing violation or access
    /// denied) through all of §9.5 step 4's retries. `source` is the
    /// last OS refusal, `retries` how many retries the schedule made.
    /// Test builds compile the variant on Unix too — the simulated
    /// locked replace drives the real loop there (see `faults`).
    #[cfg(any(windows, test, feature = "fault-injection"))]
    Locked { source: io::Error, retries: usize },
}

/// Step 4, the Unix arm: one `rename(2)`, no retries — §10's
/// `E_FILE_LOCKED` row is pinned to Windows, and rename(2) has no
/// sharing violations to retry. Whatever the raw code, the refusal is
/// the plain OS fact.
#[cfg(unix)]
fn replace_target(staging_path: &Path, target: &Path) -> Result<(), ReplaceRefusal> {
    #[cfg(any(test, feature = "fault-injection"))]
    if fault_injection::locked_replace_is_armed() {
        // The retry-mechanics test channel: the SAME §9.5 loop the
        // Windows arm runs, driven by a simulated locked replace and
        // the recording clock, so the pipeline-level `E_FILE_LOCKED`
        // path (typed refusal, staging cleanup, original intact) runs
        // on the Linux legs — a real sharing violation needs a
        // Windows handle, and T-F8's test owns that on the CI leg.
        return replace_with_lock_retries(
            || fault_injection::simulated_lock_attempt(staging_path, target),
            fault_injection::record_lock_pause,
        );
    }
    #[cfg(any(test, feature = "fault-injection"))]
    if let Some(source) =
        fault_injection::inject_atomic(fault_injection::AtomicStep::Replace, target)
    {
        return Err(ReplaceRefusal::Os(source));
    }
    perform_replace(staging_path, target).map_err(ReplaceRefusal::Os)
}

/// Step 4, the Windows arm: §9.5's retry discipline around one raw
/// `MoveFileExW` attempt — a sharing violation
/// (`ERROR_SHARING_VIOLATION`) or access denial
/// (`ERROR_ACCESS_DENIED`) is retried after the pauses of
/// [`LOCK_RETRY_PAUSES_MS`] (100, 300, 900 ms; three retries after
/// the initial attempt), a target still locked after the third retry
/// is [`ReplaceRefusal::Locked`] (§10 `E_FILE_LOCKED`), and any other
/// refusal surfaces immediately.
#[cfg(windows)]
fn replace_target(staging_path: &Path, target: &Path) -> Result<(), ReplaceRefusal> {
    #[cfg(any(test, feature = "fault-injection"))]
    if fault_injection::locked_replace_is_armed() {
        // The simulated-lock channel (see the Unix arm): the schedule
        // and its classification run against injected refusals with
        // the recording clock, no real sleeping.
        return replace_with_lock_retries(
            || fault_injection::simulated_lock_attempt(staging_path, target),
            fault_injection::record_lock_pause,
        );
    }
    #[cfg(any(test, feature = "fault-injection"))]
    if let Some(source) =
        fault_injection::inject_atomic(fault_injection::AtomicStep::Replace, target)
    {
        return Err(ReplaceRefusal::Os(source));
    }
    replace_with_lock_retries(
        || perform_replace(staging_path, target),
        |pause| {
            #[cfg(any(test, feature = "fault-injection"))]
            if fault_injection::record_lock_pause_if_armed(pause) {
                return;
            }
            std::thread::sleep(pause);
        },
    )
}

/// §9.5 step 4's retry schedule: the pause before each of the three
/// retries of a locked Windows replace, in milliseconds.
#[cfg(any(windows, test, feature = "fault-injection"))]
const LOCK_RETRY_PAUSES_MS: [u64; 3] = [100, 300, 900];

/// `ERROR_SHARING_VIOLATION` (32): another process holds the target
/// open without `FILE_SHARE_DELETE` — the editor/antivirus shape of
/// §10's `E_FILE_LOCKED` row. Spelled here as a number because the
/// crate's windows-sys feature set is exactly
/// `Win32_Storage_FileSystem` (D-095); the named constant lives in
/// the not-enabled `Win32_Foundation`.
#[cfg(any(windows, test, feature = "fault-injection"))]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// `ERROR_ACCESS_DENIED` (5): the target's ACL or attributes refuse
/// the delete-on-replace; §9.5 step 4 retries it alongside the
/// sharing violation. Same spelling note as
/// [`ERROR_SHARING_VIOLATION`].
#[cfg(any(windows, test, feature = "fault-injection"))]
const ERROR_ACCESS_DENIED: i32 = 5;

/// Whether `source` is one of §9.5 step 4's two retried lock
/// refusals, by raw Win32 code. Deliberately exactly the spec's two:
/// `ERROR_LOCK_VIOLATION` (33, byte-range locks) is not in the set
/// the spec names.
#[cfg(any(windows, test, feature = "fault-injection"))]
fn is_lock_refusal(source: &io::Error) -> bool {
    matches!(
        source.raw_os_error(),
        Some(ERROR_SHARING_VIOLATION) | Some(ERROR_ACCESS_DENIED)
    )
}

/// §9.5 step 4's retry discipline, written platform-neutral so the
/// Linux legs can drive it: `attempt` runs once, and while it refuses
/// with a lock code, again after each pause of
/// [`LOCK_RETRY_PAUSES_MS`] — one initial attempt plus at most three
/// retries. `pause` is the sleep seam: production sleeps the calling
/// thread, tests record the duration (the mock clock).
///
/// A refusal that is not a lock code ends the loop immediately as
/// [`ReplaceRefusal::Os`] — including one that arrives after lock
/// refusals: the honest latest fact, never masked as exhausted locks.
/// A lock that outlasts the whole schedule is
/// [`ReplaceRefusal::Locked`] with the last OS refusal; a success at
/// any attempt returns at once and waits no further.
#[cfg(any(windows, test, feature = "fault-injection"))]
fn replace_with_lock_retries(
    mut attempt: impl FnMut() -> io::Result<()>,
    mut pause: impl FnMut(std::time::Duration),
) -> Result<(), ReplaceRefusal> {
    let mut outcome = attempt();
    for pause_ms in LOCK_RETRY_PAUSES_MS {
        match outcome {
            Ok(()) => return Ok(()),
            Err(source) if is_lock_refusal(&source) => {}
            Err(source) => return Err(ReplaceRefusal::Os(source)),
        }
        pause(std::time::Duration::from_millis(pause_ms));
        outcome = attempt();
    }
    match outcome {
        Ok(()) => Ok(()),
        Err(source) if is_lock_refusal(&source) => Err(ReplaceRefusal::Locked {
            source,
            retries: LOCK_RETRY_PAUSES_MS.len(),
        }),
        Err(source) => Err(ReplaceRefusal::Os(source)),
    }
}

/// One raw §9.5 step-4 attempt, no retries: Unix `rename(2)`, which
/// replaces atomically. `pub(crate)`: the fault-injection module's
/// simulated locked replace calls the REAL attempt as its eventual
/// success (`fault_injection::simulated_lock_attempt`).
#[cfg(unix)]
pub(crate) fn perform_replace(staging_path: &Path, target: &Path) -> io::Result<()> {
    fs::rename(staging_path, target)
}

/// One raw §9.5 step-4 attempt, no retries — the direct
/// `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` call. std's own
/// `fs::rename` is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`
/// only (verified in the toolchain's std source,
/// `sys/fs/windows.rs`), so the spec-pinned write-through needs the
/// direct call — the crate's second owner-sanctioned FFI call after
/// D-089's free-space probe, same discipline (D-095). The retry loop
/// around it is [`replace_target`]'s Windows arm; this function is
/// the single unsynchronized syscall. `pub(crate)` for the same
/// reason as the Unix arm (the simulated locked replace).
#[cfg(windows)]
pub(crate) fn perform_replace(staging_path: &Path, target: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    // The wide buffers must outlive the call (see the SAFETY
    // comment): named locals, never temporaries in the call.
    let old_w = wide_path(staging_path);
    let new_w = wide_path(target);
    // SAFETY: `MoveFileExW(lpExistingFileName: LPCWSTR,
    // lpNewFileName: LPCWSTR, dwFlags: DWORD) -> BOOL` — one call, no
    // retained state, no callbacks (`extern "system"` ABI,
    // Kernel32). The Microsoft contract (fileapi.h) and how each
    // invariant is met:
    // 1. Both name parameters must be valid NUL-terminated wide
    //    strings for the duration of the call — `old_w`/`new_w` are
    //    exactly that (`wide_path` appends the terminator), their
    //    buffers stay alive and unmutated across the call (borrowed
    //    `as_ptr()` only), and `PCWSTR` is `*const u16`, the element
    //    type.
    // 2. `dwFlags` is a plain value: the two §9.5-pinned constants,
    //    nothing to validate.
    // 3. The function writes no caller memory and retains no
    //    handles; the staging file's own handle was dropped before
    //    this step, so no self-inflicted sharing violation exists.
    // 4. On failure the reason is in `GetLastError` — read in the
    //    very next statement (`io::Error::last_os_error()`) before
    //    anything can overwrite it, and returned whole for the retry
    //    loop's lock classification.
    let ok = unsafe {
        MoveFileExW(
            old_w.as_ptr(),
            new_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The path as a NUL-terminated UTF-16 buffer: the `LPCWSTR` shape
/// `MoveFileExW` consumes. Verbatim code units plus the terminator;
/// a named local so its lifetime covers the call (the free-space
/// module's twin helper, local to keep this module self-contained).
#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    wide
}

/// Step 5 (Unix): `fsync` the parent folder so the rename's directory
/// entry survives a crash in the window after the replace — the
/// durability §9.5 step 5 pins for Unix. Opening a directory
/// read-only and `sync_all`-ing it is the portable-on-Unix shape; on
/// Windows std cannot open a directory at all, and the spec pins the
/// step to Unix (`MOVEFILE_WRITE_THROUGH` carries the intent there).
#[cfg(unix)]
fn sync_parent_folder(folder: &Path, _target: &Path) -> io::Result<()> {
    #[cfg(any(test, feature = "fault-injection"))]
    if let Some(source) =
        fault_injection::inject_atomic(fault_injection::AtomicStep::DirSync, _target)
    {
        return Err(source);
    }
    File::open(folder)?.sync_all()
}

/// Step 6's read side: streams the target back in fixed chunks and
/// returns the digest of the bytes found on disk. An injected refusal
/// of the read surfaces as [`VerifyMismatch::Unreadable`] at the call
/// site, which owns the §9.5 step it failed.
fn read_back_digest(target: &Path) -> io::Result<String> {
    #[cfg(any(test, feature = "fault-injection"))]
    if let Some(source) =
        fault_injection::inject_atomic(fault_injection::AtomicStep::VerifyRead, target)
    {
        return Err(source);
    }
    let mut file = File::open(target)?;
    let mut hasher = Sha256Hex::new();
    let mut chunk = [0u8; VERIFY_CHUNK_BYTES];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hasher.finalize_hex())
}

/// The SHA-256 of the bytes handed in — the §9.5 step-6 expected
/// value. Pure: no I/O, so it can only describe the input.
fn sha256_hex_of(contents: &[u8]) -> String {
    let mut hasher = Sha256Hex::new();
    hasher.update(contents);
    hasher.finalize_hex()
}

/// Removes a failed run's staging file (§9.5 step 7): a staging file
/// that raced away is converged, not a failure; a refused removal is
/// returned so the caller carries it (the sweep will retry the
/// leftover under the same grammar, T-F2).
fn remove_staging(staging_path: &Path) -> Option<io::Error> {
    match fs::remove_file(staging_path) {
        Ok(()) => None,
        Err(source) if source.kind() == io::ErrorKind::NotFound => None,
        Err(source) => Some(source),
    }
}

/// Fails the pipeline after a write-path step refused: removes the
/// staging file, then classifies — a no-space refusal
/// (`io::ErrorKind::StorageFull`) is the §10 write-time `E_DISK_FULL`
/// even though it came from the OS mid-write, everything else is
/// `E_WRITE_FAILED` with the step named. The mid-write `DiskFull`
/// verdict reports the TARGET (§10's "path" is the project fact, and
/// the staging file it just deleted must not be the logged path);
/// `WriteFailed` keeps the path the failed step operated on.
fn abort(
    failure: WriteFailed,
    target: &Path,
    staging_path: &Path,
    required: u64,
) -> AtomicWriteError {
    let cleanup = remove_staging(staging_path);
    let WriteFailed {
        step,
        path,
        source,
        cleanup: _,
    } = failure;
    if source.kind() == io::ErrorKind::StorageFull {
        return AtomicWriteError::DiskFull(DiskFullFailure {
            path: target.to_owned(),
            step: Some(step),
            free: None,
            required,
            source: Some(source),
            cleanup,
        });
    }
    AtomicWriteError::WriteFailed(WriteFailed {
        step,
        path,
        source,
        cleanup,
    })
}

/// Step 4's failure path: removes the staging file, then classifies —
/// an exhausted Windows lock schedule is §10's `E_FILE_LOCKED`
/// ([`AtomicWriteError::FileLocked`]); any other refusal goes through
/// [`abort`] (a no-space refusal is the write-time `E_DISK_FULL`,
/// everything else `E_WRITE_FAILED` with the step named).
fn abort_replace(
    refusal: ReplaceRefusal,
    target: &Path,
    staging_path: &Path,
    required: u64,
) -> AtomicWriteError {
    match refusal {
        ReplaceRefusal::Os(source) => abort(
            WriteFailed {
                step: WriteStep::Replace,
                path: target.to_owned(),
                source,
                cleanup: None,
            },
            target,
            staging_path,
            required,
        ),
        #[cfg(any(windows, test, feature = "fault-injection"))]
        ReplaceRefusal::Locked { source, retries } => {
            let cleanup = remove_staging(staging_path);
            AtomicWriteError::FileLocked(FileLockedFailure {
                step: WriteStep::Replace,
                path: target.to_owned(),
                retries,
                source,
                cleanup,
            })
        }
    }
}

/// Which §9.5 pipeline step refused — the fact §10's log line carries
/// next to the OS error ("every error is written to the log with its
/// code, project, path, and OS code").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteStep {
    /// Creating the staging file (§9.5 step 2): a missing folder (the
    /// T-F7 shape), an unwritable one, or an exhausted token budget.
    TempCreate,
    /// Copying the existing target's Unix mode onto the staging file
    /// (§9.5 step 2; Unix only — the variant is never produced on
    /// Windows).
    ModeCopy,
    /// Writing the bytes and `sync_all`-ing them (§9.5 step 3).
    WriteSync,
    /// The replace rename itself (§9.5 step 4).
    Replace,
    /// The parent-folder `fsync` (§9.5 step 5; Unix only).
    DirSync,
}

impl fmt::Display for WriteStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            WriteStep::TempCreate => "creating the staging file",
            WriteStep::ModeCopy => "copying the file mode",
            WriteStep::WriteSync => "writing the staging file",
            WriteStep::Replace => "replacing the target",
            WriteStep::DirSync => "syncing the folder",
        };
        f.write_str(text)
    }
}

/// The failure data of a write-path step that refused (§10
/// `E_WRITE_FAILED`): the step, the path it was operating on (the
/// staging path for the staging steps, the target for the replace,
/// the folder for the folder sync — one meaning per step, both
/// mode-copy failures name the staging path), the OS error, and
/// whether the §9.5 step-7 cleanup succeeded.
#[derive(Debug)]
pub struct WriteFailed {
    /// Which pipeline step refused.
    pub step: WriteStep,
    /// The path the failed step was operating on.
    pub path: PathBuf,
    /// The OS failure, carrying the raw error code.
    pub source: io::Error,
    /// The staging-file removal outcome: `None` = removed (or nothing
    /// existed yet — the staging create's own failures), `Some` = the
    /// staging file is still on disk and the sweep's retry owns it.
    pub cleanup: Option<io::Error>,
}

/// The failure data of §10's `E_FILE_LOCKED`: a Windows target that
/// stayed locked through all of §9.5 step 4's retries.
#[derive(Debug)]
pub struct FileLockedFailure {
    /// The pipeline step that refused — always [`WriteStep::Replace`]:
    /// the lock is a replace-step fact, and the field keeps the
    /// module's step/path contract uniform.
    pub step: WriteStep,
    /// The locked target path the replace was retried on.
    pub path: PathBuf,
    /// How many §9.5 retries were made before giving up — the
    /// schedule's three (after 100, 300 and 900 ms).
    pub retries: usize,
    /// The last OS refusal (`ERROR_SHARING_VIOLATION` or
    /// `ERROR_ACCESS_DENIED`), carrying the raw code for the §10 log
    /// line.
    pub source: io::Error,
    /// The staging-file removal outcome, identical in meaning to
    /// [`WriteFailed::cleanup`].
    pub cleanup: Option<io::Error>,
}

/// The failure data of the §10 write-time `E_DISK_FULL`: what was
/// demanded, what was available (when the pre-write check could
/// answer), and the OS refusal when the verdict came from the write
/// path instead.
#[derive(Debug)]
pub struct DiskFullFailure {
    /// The target file the write demanded space for (§10's log fact —
    /// never the staging file, even when the OS refusal came from a
    /// step that was operating on it).
    pub path: PathBuf,
    /// The step the OS refusal came from, when the verdict came from
    /// the write path; `None` for the pre-write check's shortage
    /// verdict (no step had run yet).
    pub step: Option<WriteStep>,
    /// The free bytes the pre-write source reported, when it could
    /// answer; `None` when the verdict came from the OS refusing a
    /// write-path step (`source`) and no probe number exists.
    pub free: Option<u64>,
    /// The byte count the write demanded: exactly the content's
    /// length (§9.4's 20% margin belongs to the backup-volume check;
    /// no margin is pinned for the write).
    pub required: u64,
    /// The OS no-space refusal (`ENOSPC` / `StorageFull`) when the
    /// verdict came from the write path; `None` for the pre-write
    /// check's shortage verdict.
    pub source: Option<io::Error>,
    /// The staging-file removal outcome, identical in meaning to
    /// [`WriteFailed::cleanup`] (`None` for the pre-write check —
    /// nothing existed yet).
    pub cleanup: Option<io::Error>,
}

/// The failure data of the §9.5 step-6 check: what was expected, and
/// what the settled file actually showed.
#[derive(Debug)]
pub struct VerifyFailed {
    /// The target path that was re-read.
    pub path: PathBuf,
    /// The expected digest: the SHA-256 of the bytes handed in, 64
    /// lowercase hex characters.
    pub expected_sha256_hex: String,
    /// What the re-read actually measured.
    pub mismatch: VerifyMismatch,
}

/// What the §9.5 step-6 re-read saw, when it did not match.
#[derive(Debug)]
pub enum VerifyMismatch {
    /// The file was read whole and its bytes hash differently — the
    /// settled content is not what this run wrote (corruption, or an
    /// external editor inside the rename→verify window).
    Digest {
        /// The digest of the bytes found on disk, 64 lowercase hex
        /// characters.
        actual_sha256_hex: String,
    },
    /// The target could not be opened or read to the end for the
    /// check at all.
    Unreadable {
        /// The OS failure, carrying the raw error code.
        source: io::Error,
    },
}

impl VerifyMismatch {
    /// The I/O error behind the mismatch, when there is one.
    fn io_error(&self) -> Option<&io::Error> {
        match self {
            VerifyMismatch::Digest { .. } => None,
            VerifyMismatch::Unreadable { source } => Some(source),
        }
    }
}

/// Why an atomic write failed (typed, hand-rolled per the crate's
/// error posture, D-017/D-084: no error-crate dependency; the §10
/// code mapping happens in the command layer from these facts).
#[derive(Debug)]
pub enum AtomicWriteError {
    /// The target's file name cannot shape a staging name (not a
    /// single plain file name — separators, NUL, `.`/`..`): nothing
    /// was created, and no spelling the sweep matches could ever
    /// have been produced.
    NameFailed {
        /// The staging-name builder's typed refusal.
        source: TmpNameError,
    },
    /// No space on the project volume (§10's write-time
    /// `E_DISK_FULL`): the pre-write check's shortage verdict, or the
    /// OS refusing a write-path step with no space.
    DiskFull(DiskFullFailure),
    /// A pipeline step refused (§10 `E_WRITE_FAILED`); the staging
    /// file was removed unless the removal itself was refused.
    WriteFailed(WriteFailed),
    /// The target is locked by another process (§10 `E_FILE_LOCKED`;
    /// Windows only): the §9.5 step-4 replace was refused with a
    /// sharing violation or access denied through all three retries
    /// (100/300/900 ms). The staging file was removed unless the
    /// removal itself was refused (`cleanup`); the target keeps its
    /// old content whole. Unix never produces this variant — §10
    /// pins the row to Windows.
    FileLocked(FileLockedFailure),
    /// The written file failed the §9.5 step-6 re-read check (§10
    /// `E_VERIFY_FAILED`). The file holds the new, unverified
    /// content and is deliberately not deleted — the remedy is the
    /// backup copy.
    VerifyFailed(VerifyFailed),
}

impl AtomicWriteError {
    /// The I/O error behind the failure, for the caller's log line
    /// (`ErrorDetails::from_io_error` in the app layer consumes the
    /// same shape as the sibling modules' errors). `None` where no
    /// OS call failed (the name refusal; the pre-write shortage
    /// verdict; the digest mismatch).
    pub fn io_error(&self) -> Option<&io::Error> {
        match self {
            AtomicWriteError::NameFailed { .. } => None,
            AtomicWriteError::DiskFull(failed) => failed.source.as_ref(),
            AtomicWriteError::WriteFailed(failed) => Some(&failed.source),
            AtomicWriteError::FileLocked(failed) => Some(&failed.source),
            AtomicWriteError::VerifyFailed(failed) => failed.mismatch.io_error(),
        }
    }

    /// The raw OS error code (`std::io::Error::raw_os_error`), the
    /// §10 log fact. `None` where no OS call failed.
    pub fn os_code(&self) -> Option<i32> {
        self.io_error().and_then(io::Error::raw_os_error)
    }
}

impl fmt::Display for AtomicWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtomicWriteError::NameFailed { source } => {
                write!(f, "the staging name was refused: {source}")
            }
            AtomicWriteError::DiskFull(failed) => {
                match (failed.free, &failed.source) {
                    // The pre-write verdict: both numbers are facts.
                    (Some(free), None) => write!(
                        f,
                        "not enough space on the volume of {}: {free} bytes available, {} required",
                        failed.path.display(),
                        failed.required
                    ),
                    // The mid-write verdict has no probe numbers: the
                    // OS error is the fact.
                    (_, Some(source)) => write!(
                        f,
                        "no space left while writing {}: {source}",
                        failed.path.display()
                    ),
                    // Both absent is not a shape the pipeline produces
                    // (one of the two is always set); the arm keeps
                    // the Display total without panicking on a
                    // hand-built value.
                    (None, None) => write!(
                        f,
                        "no space left for {}: {} bytes required",
                        failed.path.display(),
                        failed.required
                    ),
                }?;
                if let Some(cleanup) = &failed.cleanup {
                    write!(f, "; the staging file could not be removed: {cleanup}")?;
                }
                Ok(())
            }
            AtomicWriteError::WriteFailed(failed) => {
                write!(
                    f,
                    "the atomic write failed while {} {}: {}",
                    failed.step,
                    failed.path.display(),
                    failed.source
                )?;
                if let Some(cleanup) = &failed.cleanup {
                    write!(f, "; the staging file could not be removed: {cleanup}")?;
                }
                Ok(())
            }
            AtomicWriteError::FileLocked(failed) => {
                write!(
                    f,
                    "the atomic write failed while {} {}: the file is locked by another \
                     process, {} retries failed: {}",
                    failed.step,
                    failed.path.display(),
                    failed.retries,
                    failed.source
                )?;
                if let Some(cleanup) = &failed.cleanup {
                    write!(f, "; the staging file could not be removed: {cleanup}")?;
                }
                Ok(())
            }
            AtomicWriteError::VerifyFailed(failed) => {
                write!(
                    f,
                    "the written file {} failed the check",
                    failed.path.display()
                )?;
                match &failed.mismatch {
                    VerifyMismatch::Digest { actual_sha256_hex } => write!(
                        f,
                        ": sha256 {} expected, {actual_sha256_hex} found",
                        failed.expected_sha256_hex
                    )?,
                    VerifyMismatch::Unreadable { source } => {
                        write!(f, ": the file could not be read back: {source}")?
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for AtomicWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AtomicWriteError::NameFailed { source } => Some(source),
            AtomicWriteError::DiskFull(failed) => failed
                .source
                .as_ref()
                .map(|source| source as &(dyn std::error::Error + 'static)),
            AtomicWriteError::WriteFailed(failed) => Some(&failed.source),
            AtomicWriteError::FileLocked(failed) => Some(&failed.source),
            AtomicWriteError::VerifyFailed(failed) => failed
                .mismatch
                .io_error()
                .map(|source| source as &(dyn std::error::Error + 'static)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;
    use crate::free_space::free_bytes;

    /// The FIPS 180-2 vector for "abc" (re-checked with Python's
    /// hashlib) — the independent oracle for the fixture content.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// The FIPS 180-2 vector for the empty input.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// A fake free-space source for the substituted-source seam: a
    /// closure over a plain number, no filesystem anywhere.
    fn fake_free(free: u64) -> impl FnOnce() -> Result<u64, FreeSpaceError> {
        move || Ok(free)
    }

    /// A plentiful fake source: no space trouble in the happy-path
    /// fixtures.
    fn plenty_free() -> impl FnOnce() -> Result<u64, FreeSpaceError> {
        fake_free(1 << 40)
    }

    /// The sorted entry names of `folder` — the "no debris" oracle:
    /// what the write left behind is exactly observable.
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

    /// The fixture's owner uid is this process's effective uid (the
    /// fixture was created by it): uid 0 means the tests run as root,
    /// where mode bits are not enforced and an unwritable directory
    /// stays writable — the precondition of a permission test cannot
    /// be set up. The caller skips loudly instead of passing
    /// vacuously (house precedent: `temp_files`).
    #[cfg(unix)]
    fn running_as_root_is_untestable(folder: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;

        fs::metadata(folder)
            .expect("the fixture directory must be statable")
            .uid()
            == 0
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;

        fs::metadata(path)
            .expect("stating the fixture must work")
            .permissions()
            .mode()
            & 0o7777
    }

    /// A target file with `contents`, for the overwrite fixtures.
    fn existing(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).expect("writing the fixture file must work");
        path
    }

    // ---- the happy paths -----------------------------------------------

    #[test]
    fn a_new_file_is_written_settled_and_verified() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");

        let written = write_target_file(&target, b"abc", plenty_free())
            .expect("creating a new file must succeed");

        assert_eq!(fs::read(&target).expect("the file must exist"), b"abc");
        assert_eq!(written.path, target);
        assert_eq!(written.size, 3);
        // The reported digest is the FIPS oracle of the content, not
        // a self-derived value.
        assert_eq!(written.sha256_hex, ABC_SHA256);
        // Nothing else is in the folder: no staging debris.
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn an_existing_file_is_replaced_whole_with_no_debris() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"the old content");

        write_target_file(&target, b"the new content", plenty_free())
            .expect("replacing an existing file must succeed");

        assert_eq!(
            fs::read(&target).expect("the file must still exist"),
            b"the new content",
            "the target holds exactly the new bytes, never a mix"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn an_empty_write_creates_an_empty_verified_file() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");

        let written =
            write_target_file(&target, b"", plenty_free()).expect("an empty write is legal");

        assert_eq!(fs::read(&target).expect("the file must exist"), b"");
        assert_eq!(written.size, 0);
        assert_eq!(written.sha256_hex, EMPTY_SHA256);
    }

    #[test]
    fn a_multichunk_write_streams_to_the_right_digest() {
        // The §9.5 step-6 re-read streams in 64 KiB chunks; a full
        // chunk plus a tail forces the verifier through a real chunk
        // boundary (a single `write_all` of this size to a regular
        // file is normally one syscall, so the writer's partial path
        // is NOT what is pinned here — the read side is). The
        // expected digest is a Python-hashlib oracle over the exact
        // byte pattern (the house precedent: backup's multimegabyte
        // test), not a value derived from the code under test — a
        // chunk-accumulation bug in the re-read hasher mismatches the
        // check and fails this write.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        let contents: Vec<u8> = (0..VERIFY_CHUNK_BYTES + 7)
            .map(|index| (index % 251) as u8)
            .collect();

        let written = write_target_file(&target, &contents, plenty_free())
            .expect("a multichunk write must verify like a small one");

        assert_eq!(written.size, contents.len() as u64);
        assert_eq!(
            written.sha256_hex,
            "48e84525bcb5c9dcd67fa7a692ed20f31909ba8f0dd24c3ada9bd54b057468b0"
        );
        assert_eq!(fs::read(&target).expect("the file must exist"), contents);
    }

    // ---- the space check (§10 E_DISK_FULL, pre-write) -------------------

    #[test]
    fn a_short_volume_fails_disk_full_before_anything_is_touched() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let contents = b"thirteen bytes";
        let target = existing(dir.path(), "AGENTS.md", b"untouched");

        let err = write_target_file(&target, contents, fake_free(contents.len() as u64 - 1))
            .expect_err("one byte short of the content must refuse the write");

        let AtomicWriteError::DiskFull(failed) = &err else {
            panic!("a shortage must be DiskFull, got: {err:?}");
        };
        assert_eq!(failed.free, Some(contents.len() as u64 - 1));
        assert_eq!(failed.required, contents.len() as u64);
        assert_eq!(failed.step, None, "no step had run yet");
        assert!(failed.source.is_none(), "no OS call failed");
        assert!(
            failed.cleanup.is_none(),
            "nothing existed to clean up — the check runs first"
        );
        assert_eq!(err.os_code(), None, "a decision, not an OS failure");
        // The card's invariant: the original is intact, nothing was
        // staged.
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn a_volume_with_exactly_the_needed_space_passes() {
        // `check_space`'s strict-less-than ruling, inherited: the
        // spec aborts only when there is LESS space than needed.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");

        write_target_file(&target, b"abc", fake_free(3)).expect("the exact boundary passes");
    }

    #[test]
    fn a_failing_space_probe_does_not_block_the_write() {
        // ERR-003's posture: the write, not the probe, is the
        // authority. A dropped drive fails the probe here and the
        // actual write call later; a flaky probe must not refuse a
        // write that would succeed.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");
        let probe = || {
            Err(FreeSpaceError::ProbeFailed {
                source: io::Error::from_raw_os_error(2),
            })
        };

        write_target_file(&target, b"abc", probe).expect("a failed probe must not abort the write");
        assert_eq!(fs::read(&target).expect("the file must exist"), b"abc");
    }

    #[test]
    fn the_real_free_space_probe_drives_a_successful_write() {
        // The wiring check: production's closure shape over the real
        // probe (the substituted-source tests above are the
        // contract).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("AGENTS.md");

        write_target_file(&target, b"abc", || free_bytes(dir.path()))
            .expect("a live volume must pass the check");
    }

    // ---- ENOSPC on the write path (§10 E_DISK_FULL, mid-write) ----------

    #[cfg(unix)]
    #[test]
    fn enospc_from_the_write_step_is_disk_full_and_cleans_the_staging_file() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        // ENOSPC = 28: std maps it to ErrorKind::StorageFull, the
        // mid-write verdict the module classifies by.
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::WriteSync,
            io::Error::from_raw_os_error(28),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("a no-space refusal must fail the write");

        let AtomicWriteError::DiskFull(failed) = &err else {
            panic!("ENOSPC must be classified DiskFull, got: {err:?}");
        };
        assert_eq!(
            failed.path, target,
            "§10's path fact is the target, never the deleted staging file"
        );
        assert_eq!(failed.step, Some(WriteStep::WriteSync));
        assert_eq!(failed.free, None, "no probe number exists mid-write");
        assert_eq!(failed.required, 3);
        assert_eq!(
            err.os_code(),
            Some(28),
            "the OS code rides for the §10 log line"
        );
        assert!(
            failed.cleanup.is_none(),
            "the staging file must be removed, not carried"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    // ---- the Windows lock-retry schedule (§9.5 step 4, T-F8) ---------

    #[test]
    fn a_sharing_violation_rides_the_full_schedule_then_refuses_locked() {
        // ERROR_SHARING_VIOLATION = 32: the editor/antivirus shape.
        // The loop runs against injected refusals with a recording
        // clock — the schedule and attempt count are pinned without
        // any real sleeping.
        let mut attempts = 0;
        let mut pauses = Vec::new();
        let refusal = replace_with_lock_retries(
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(32))
            },
            |pause| pauses.push(pause),
        )
        .expect_err("a permanently locked target must refuse the write");

        assert_eq!(attempts, 4, "one initial attempt plus three retries");
        assert_eq!(
            pauses,
            [
                Duration::from_millis(100),
                Duration::from_millis(300),
                Duration::from_millis(900),
            ],
            "the §9.5 pauses, in order and no more"
        );
        let ReplaceRefusal::Locked { source, retries } = refusal else {
            panic!("exhausted locks are the typed refusal, got: {refusal:?}");
        };
        assert_eq!(retries, 3);
        assert_eq!(
            source.raw_os_error(),
            Some(32),
            "the last OS refusal rides for the log line"
        );
    }

    #[test]
    fn an_access_denied_refusal_rides_the_same_schedule() {
        // ERROR_ACCESS_DENIED = 5: §9.5 step 4 retries it alongside
        // the sharing violation — the same discipline, the same end.
        let mut attempts = 0;
        let mut pauses = Vec::new();
        let refusal = replace_with_lock_retries(
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(5))
            },
            |pause| pauses.push(pause),
        )
        .expect_err("a permanently denied target must refuse the write");

        assert_eq!(attempts, 4);
        assert_eq!(
            pauses,
            [
                Duration::from_millis(100),
                Duration::from_millis(300),
                Duration::from_millis(900),
            ]
        );
        let ReplaceRefusal::Locked { source, .. } = refusal else {
            panic!("access denied exhausts into the same typed refusal: {refusal:?}");
        };
        assert_eq!(source.raw_os_error(), Some(5));
    }

    #[test]
    fn a_lock_that_lifts_mid_schedule_succeeds_after_the_pauses_so_far() {
        // The holder closes the file between retries: the write must
        // land, having waited exactly the pauses before the winning
        // attempt.
        let mut attempts = 0;
        let mut pauses = Vec::new();
        replace_with_lock_retries(
            || {
                attempts += 1;
                if attempts <= 2 {
                    Err(io::Error::from_raw_os_error(32))
                } else {
                    Ok(())
                }
            },
            |pause| pauses.push(pause),
        )
        .expect("a lock that lifts must let the write through");

        assert_eq!(attempts, 3, "the initial attempt and two retries");
        assert_eq!(
            pauses,
            [Duration::from_millis(100), Duration::from_millis(300),],
            "no pause after the successful attempt"
        );
    }

    #[test]
    fn a_non_lock_refusal_is_not_retried() {
        // Only the two §9.5 lock codes wait: any other refusal fails
        // at once, exactly as before the retry loop existed. 33 is
        // ERROR_LOCK_VIOLATION — byte-range locks, the one code
        // MoveFileExW retry advice habitually adds — and D-099 rules
        // it OUT: the spec names two conditions, and 33 is neither
        // (a mutant widening the set dies here).
        for code in [13, 33] {
            let mut attempts = 0;
            let mut pauses = Vec::new();
            let refusal = replace_with_lock_retries(
                || {
                    attempts += 1;
                    Err(io::Error::from_raw_os_error(code))
                },
                |pause| pauses.push(pause),
            )
            .expect_err("a non-lock refusal must fail immediately");

            assert_eq!(attempts, 1, "code {code}: exactly one attempt");
            assert!(pauses.is_empty(), "code {code}: no pause was taken");
            let ReplaceRefusal::Os(source) = refusal else {
                panic!("code {code}: a plain OS refusal stays the plain shape");
            };
            assert_eq!(source.raw_os_error(), Some(code));
        }
    }

    #[test]
    fn a_different_refusal_after_locks_is_reported_as_itself() {
        // The lock lifts into a DIFFERENT failure (say the folder
        // vanished mid-schedule): the honest latest fact is reported,
        // never masked as "still locked".
        let mut attempts = 0;
        let mut pauses = Vec::new();
        let refusal = replace_with_lock_retries(
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(io::Error::from_raw_os_error(32))
                } else {
                    Err(io::Error::from_raw_os_error(13))
                }
            },
            |pause| pauses.push(pause),
        )
        .expect_err("the second refusal must fail the replace");

        assert_eq!(attempts, 2);
        assert_eq!(pauses, [Duration::from_millis(100)]);
        let ReplaceRefusal::Os(source) = refusal else {
            panic!("the latest refusal is the fact, got: {refusal:?}");
        };
        assert_eq!(source.raw_os_error(), Some(13));
    }

    #[test]
    fn a_different_refusal_at_the_last_scheduled_attempt_is_reported_as_itself() {
        // The same honesty at the schedule's END: three lock refusals
        // ride all three pauses, the fourth attempt refuses with a
        // different code — and that fact, not "locked", is the
        // verdict (a mutant classifying the final refusal as Locked
        // dies here).
        let mut attempts = 0;
        let mut pauses = Vec::new();
        let refusal = replace_with_lock_retries(
            || {
                attempts += 1;
                if attempts <= 3 {
                    Err(io::Error::from_raw_os_error(32))
                } else {
                    Err(io::Error::from_raw_os_error(13))
                }
            },
            |pause| pauses.push(pause),
        )
        .expect_err("the fourth refusal must fail the replace");

        assert_eq!(attempts, 4, "the whole schedule ran first");
        assert_eq!(
            pauses,
            [
                Duration::from_millis(100),
                Duration::from_millis(300),
                Duration::from_millis(900),
            ]
        );
        let ReplaceRefusal::Os(source) = refusal else {
            panic!("the latest refusal is the fact, got: {refusal:?}");
        };
        assert_eq!(source.raw_os_error(), Some(13));
    }

    #[test]
    fn an_exhausted_locked_replace_fails_file_locked_and_cleans_the_staging_file() {
        // The pipeline-level endgame on the Linux legs: the simulated
        // locked replace drives the REAL loop, so the typed refusal,
        // the staging cleanup and the intact original all run.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault =
            fault_injection::arm_locked_replace(fault_injection::LockedReplacePlan::Exhaust {
                code: 32,
            });

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("a permanently locked target must fail the write");

        let AtomicWriteError::FileLocked(failed) = &err else {
            panic!("exhausted lock retries are E_FILE_LOCKED, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::Replace);
        assert_eq!(failed.path, target);
        assert_eq!(failed.retries, 3, "the whole §9.5 schedule ran");
        assert_eq!(
            err.os_code(),
            Some(32),
            "the last OS refusal rides for the §10 log line"
        );
        assert!(
            failed.cleanup.is_none(),
            "the staging file must be removed, not carried"
        );
        assert_eq!(
            fault_injection::recorded_lock_pauses(),
            [
                Duration::from_millis(100),
                Duration::from_millis(300),
                Duration::from_millis(900),
            ],
            "the real schedule ran, in order"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn an_exhausted_access_denied_replace_carries_code_five() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault =
            fault_injection::arm_locked_replace(fault_injection::LockedReplacePlan::Exhaust {
                code: 5,
            });

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("a permanently denied target must fail the write");

        let AtomicWriteError::FileLocked(_) = &err else {
            panic!("access denied exhausts into E_FILE_LOCKED too, got: {err:?}");
        };
        assert_eq!(err.os_code(), Some(5));
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn a_locked_replace_that_unlocks_mid_schedule_lands_the_write() {
        // T-F8's happy neighbor: the editor closes the file while the
        // schedule is still running — the retry succeeds, the write
        // lands whole.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"old");
        let _fault =
            fault_injection::arm_locked_replace(fault_injection::LockedReplacePlan::UnlockAfter {
                refusals: 2,
                code: 32,
            });

        write_target_file(&target, b"new", plenty_free())
            .expect("a lock that lifts must let the write land");

        assert_eq!(fs::read(&target).expect("the write must land"), b"new");
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
        assert_eq!(
            fault_injection::recorded_lock_pauses(),
            [Duration::from_millis(100), Duration::from_millis(300),],
            "two refusals, two pauses — nothing after the success"
        );
    }

    #[test]
    fn an_unlocked_replace_waits_not_at_all() {
        // §9.5's schedule spends time only on refusals: a target free
        // from the first attempt must not wait a single millisecond.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"old");
        let _fault =
            fault_injection::arm_locked_replace(fault_injection::LockedReplacePlan::UnlockAfter {
                refusals: 0,
                code: 32,
            });

        write_target_file(&target, b"new", plenty_free())
            .expect("a free target must replace immediately");

        assert_eq!(fs::read(&target).expect("the write must land"), b"new");
        assert!(
            fault_injection::recorded_lock_pauses().is_empty(),
            "a first-attempt success never pauses"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_cleanup_after_an_exhausted_lock_is_carried_and_the_debris_stays_sweepable() {
        // The FileLocked twin of the WriteFailed refused-removal
        // fixture: the lock exhausts AND the folder refuses the
        // §9.5 step-7 removal — the refusal must ride in `cleanup`
        // (the sweep owns the leftover), and the Display must say the
        // leftover exists.
        use std::os::unix::fs::PermissionsExt;

        use crate::temp_files::is_app_tmp_file;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping a_refused_cleanup_after_an_exhausted_lock_is_carried_and_the_debris_stays_sweepable: \
                 running as root — mode bits are not enforced, the refused-removal \
                 precondition cannot be set up"
            );
            return;
        }
        let _fault = fault_injection::arm_locked_replace(
            fault_injection::LockedReplacePlan::ExhaustWithRefusedCleanup { code: 32 },
        );

        let result = write_target_file(&target, b"abc", plenty_free());
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("the exhausted lock must fail the write");
        let AtomicWriteError::FileLocked(failed) = &err else {
            panic!("the exhausted lock is still E_FILE_LOCKED, got: {err:?}");
        };
        let cleanup = failed
            .cleanup
            .as_ref()
            .expect("the refused removal must be carried, not dropped");
        assert_eq!(
            cleanup.raw_os_error(),
            Some(13),
            "the removal refusal is EACCES"
        );
        assert!(
            err.to_string().contains("could not be removed"),
            "the Display must say the leftover exists: {err}"
        );
        assert_eq!(
            failed.retries, 3,
            "the whole schedule ran before the verdict"
        );
        // The leftover is exactly one staging file under the sweep's
        // grammar — T-F2's debris contract, unchanged by the retries.
        let leftover: Vec<String> = entry_names(dir.path())
            .into_iter()
            .filter(|name| name != "AGENTS.md")
            .collect();
        assert_eq!(
            leftover.len(),
            1,
            "exactly one staging file stays: {leftover:?}"
        );
        assert!(
            is_app_tmp_file(&leftover[0], "AGENTS.md"),
            "the leftover must be sweepable debris: {}",
            leftover[0]
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
    }

    #[test]
    fn an_injected_replace_refusal_bypasses_the_retry_schedule() {
        // The one-shot hook refuses the STEP, not a syscall attempt,
        // so no retries ride it — pinning both the seam's semantics
        // and the platform boundary: on Unix the arm never classifies
        // a lock at all (raw 32 is just a number there — EPIPE), and
        // even on Windows the injection precedes the loop.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::Replace,
            io::Error::from_raw_os_error(32),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("the injected refusal must fail the write");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("a one-shot refusal stays E_WRITE_FAILED, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::Replace);
        assert_eq!(err.os_code(), Some(32));
        assert!(
            fault_injection::recorded_lock_pauses().is_empty(),
            "the schedule never ran"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    // ---- a failure at every step: original intact, staging removed ------

    #[test]
    fn a_staging_create_failure_keeps_the_original_intact() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::TempCreate,
            io::Error::from_raw_os_error(13),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("an injected create refusal must fail the write");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("an EACCES create refusal must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::TempCreate);
        assert_eq!(err.os_code(), Some(13));
        assert!(
            failed.cleanup.is_none(),
            "the create itself failed: nothing existed to remove"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn a_missing_folder_fails_at_the_staging_create_with_the_os_error() {
        // T-F7's shape: the folder vanished between preflight and the
        // write.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = dir.path().join("never-created").join("AGENTS.md");

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("a missing folder must fail the staging create");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("a missing folder must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::TempCreate);
        assert!(
            err.os_code().is_some(),
            "the OS refusal (ENOENT on Unix) must ride along: {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_mode_copy_failure_cleans_the_staging_file() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::ModeCopy,
            io::Error::from_raw_os_error(13),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("an injected chmod refusal must fail the write");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("a mode-copy refusal must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::ModeCopy);
        assert!(
            failed.cleanup.is_none(),
            "the staging file must be removed, not carried"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn a_write_step_failure_cleans_the_staging_file_and_keeps_the_original() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        // EACCES, not ENOSPC: the plain write-failure classification.
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::WriteSync,
            io::Error::from_raw_os_error(13),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("an injected write refusal must fail the write");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("an EACCES write refusal must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::WriteSync);
        assert!(
            failed.cleanup.is_none(),
            "the staging file must be removed, not carried"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn a_replace_failure_cleans_the_staging_file_and_keeps_the_original() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::Replace,
            io::Error::from_raw_os_error(13),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("an injected rename refusal must fail the write");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("a replace refusal must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::Replace);
        assert!(
            failed.cleanup.is_none(),
            "the staging file must be removed, not carried"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_sync_failure_reports_the_step_with_the_new_content_whole() {
        // Post-replace, old-or-new is the invariant: the rename
        // already happened, so the file is WHOLLY the new content and
        // the staging path no longer exists (converged cleanup).
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::DirSync,
            io::Error::from_raw_os_error(5),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("an injected fsync refusal must fail the write");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("a folder-sync refusal must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::DirSync);
        assert_eq!(err.os_code(), Some(5), "EIO = 5 rides for the log line");
        assert!(
            failed.cleanup.is_none(),
            "the rename consumed the staging path: converged, not carried"
        );
        assert_eq!(
            fs::read(&target).expect("the target must hold the new bytes"),
            b"abc",
            "the file is wholly the new content, never a mix"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn a_verify_read_failure_is_verify_failed_with_the_new_content_whole() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::Fail(
            fault_injection::AtomicStep::VerifyRead,
            io::Error::from_raw_os_error(13),
        ));

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("an injected read-back refusal must fail the write");

        let AtomicWriteError::VerifyFailed(failed) = &err else {
            panic!("an unreadable target must be VerifyFailed, got: {err:?}");
        };
        assert_eq!(failed.expected_sha256_hex, ABC_SHA256);
        assert!(
            matches!(&failed.mismatch, VerifyMismatch::Unreadable { .. }),
            "the refusal is the unreadable shape: {:?}",
            failed.mismatch
        );
        assert_eq!(err.os_code(), Some(13));
        // The file is NOT deleted (it is the user's data now); it is
        // wholly the new content.
        assert_eq!(fs::read(&target).expect("the file must stay"), b"abc");
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[test]
    fn a_corrupted_settled_file_is_caught_by_the_digest_check() {
        // The check reads what is ON DISK: appending one byte to the
        // settled target before the re-read must be caught, with the
        // actual digest of the corrupted bytes — proving the verdict
        // comes from the file, not the input buffer.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let _fault = fault_injection::arm_atomic_fault(fault_injection::AtomicFault::CorruptTarget);

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("a corrupted settled file must fail the check");

        let AtomicWriteError::VerifyFailed(failed) = &err else {
            panic!("a digest mismatch must be VerifyFailed, got: {err:?}");
        };
        let VerifyMismatch::Digest { actual_sha256_hex } = &failed.mismatch else {
            panic!(
                "the mismatch must carry the actual digest: {:?}",
                failed.mismatch
            );
        };
        // Independent oracle: the test hashes the file's bytes with
        // its own read; the module streamed the same file.
        let mut hasher = Sha256Hex::new();
        hasher.update(&fs::read(&target).expect("the corrupted file must be readable"));
        assert_eq!(actual_sha256_hex, &hasher.finalize_hex());
        assert_eq!(failed.expected_sha256_hex, ABC_SHA256);
        // The corrupted file is kept, not deleted — §10's remedy is
        // the backup.
        assert_eq!(
            fs::read(&target).expect("the file must stay"),
            b"abcX",
            "the corrupted bytes must not be removed by the module"
        );
        assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_staging_removal_is_carried_and_the_file_stays_sweepable() {
        use std::os::unix::fs::PermissionsExt;

        use crate::temp_files::is_app_tmp_file;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        if running_as_root_is_untestable(dir.path()) {
            eprintln!(
                "skipping a_refused_staging_removal_is_carried_and_the_file_stays_sweepable: \
                 running as root — mode bits are not enforced, the refused-removal \
                 precondition cannot be set up"
            );
            return;
        }
        let _fault = fault_injection::arm_atomic_fault(
            fault_injection::AtomicFault::FailAndLockFolder(fault_injection::AtomicStep::WriteSync),
        );

        let result = write_target_file(&target, b"abc", plenty_free());
        // Restore even when the assertions below panic, so TempDir's
        // teardown can remove the directory.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755))
            .expect("restoring mode bits must work");

        let err = result.expect_err("the injected failure must fail the write");
        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("the refusal must be WriteFailed, got: {err:?}");
        };
        let cleanup = failed
            .cleanup
            .as_ref()
            .expect("the refused removal must be carried, not dropped");
        assert_eq!(
            cleanup.raw_os_error(),
            Some(13),
            "the removal refusal is EACCES"
        );
        assert!(
            err.to_string().contains("could not be removed"),
            "the Display must say the leftover exists: {err}"
        );
        // The leftover is exactly one staging file under the sweep's
        // grammar — T-F2's debris contract.
        let leftover: Vec<String> = entry_names(dir.path())
            .into_iter()
            .filter(|name| name != "AGENTS.md")
            .collect();
        assert_eq!(
            leftover.len(),
            1,
            "exactly one staging file stays: {leftover:?}"
        );
        assert!(
            is_app_tmp_file(&leftover[0], "AGENTS.md"),
            "the leftover must be sweepable debris: {}",
            leftover[0]
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
    }

    // ---- the staging token and its collisions ---------------------------

    #[test]
    fn a_token_collision_retries_with_a_fresh_name_and_keeps_the_foreign_file() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        // The exact name the first token will spell — pinned by
        // pre-creating it: the writer must hit the collision and move
        // on to the second token.
        let colliding = dir.path().join(".AGENTS.md.eunomia-abcd1234.tmp");
        fs::write(&colliding, b"foreign debris").expect("writing the fixture must work");
        let _tokens = fault_injection::fix_staging_tokens(&["abcd1234", "0f1e2d6c"]);

        write_target_file(&target, b"abc", plenty_free())
            .expect("a single collision must be retried, not fatal");

        assert_eq!(fs::read(&target).expect("the write must land"), b"abc");
        assert_eq!(
            fs::read(&colliding).expect("the foreign file must never be touched"),
            b"foreign debris",
            "a collision is an error under the next token, never a clobber"
        );
        let mut names = entry_names(dir.path());
        names.retain(|name| name != "AGENTS.md");
        assert_eq!(
            names,
            [".AGENTS.md.eunomia-abcd1234.tmp"],
            "the foreign file stays, no second staging file remains"
        );
    }

    #[test]
    fn exhausted_token_attempts_fail_the_write_with_the_last_attempted_name() {
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        let colliding = dir.path().join(".AGENTS.md.eunomia-abcd1234.tmp");
        fs::write(&colliding, b"foreign debris").expect("writing the fixture must work");
        // Every attempt collides on the same token.
        let _tokens = fault_injection::fix_staging_tokens(&["abcd1234", "abcd1234", "abcd1234"]);

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("an exhausted token budget must fail the write");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("an exhausted budget must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::TempCreate);
        assert_eq!(
            failed.path, colliding,
            "the last attempted name is the fact"
        );
        assert_eq!(
            failed.source.kind(),
            io::ErrorKind::AlreadyExists,
            "the carried refusal is the collision itself"
        );
        assert_eq!(
            fs::read(&target).expect("the target must survive"),
            b"untouched"
        );
        assert_eq!(
            fs::read(&colliding).expect("the foreign file must never be touched"),
            b"foreign debris"
        );
    }

    // ---- the Unix mode copy (§9.5 step 2) --------------------------------

    #[cfg(unix)]
    #[test]
    fn the_replaced_file_inherits_the_targets_unix_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"old");
        // A mode no platform default produces, with no write bit for
        // group and a stray read bit for other.
        fs::set_permissions(&target, fs::Permissions::from_mode(0o604))
            .expect("chmod on the fixture must work");

        write_target_file(&target, b"new", plenty_free()).expect("the write must succeed");

        assert_eq!(
            mode_of(&target),
            0o604,
            "the replaced file keeps the owner's mode, exactly"
        );

        // The full-mask ruling, pinned through the sticky bit: the
        // copy is chmod `& 0o7777`, and a `0o777` mask mutant loses
        // sticky — the one special bit the kernel's killpriv policy
        // lets survive a write (setuid/setgid are stripped from ANY
        // file at write time by the kernel itself, whatever the
        // module does; sticky rides along).
        fs::set_permissions(&target, fs::Permissions::from_mode(0o1604))
            .expect("chmod on the fixture must work");
        write_target_file(&target, b"newer", plenty_free()).expect("the second write must succeed");
        assert_eq!(mode_of(&target), 0o1604);
    }

    // ---- the caller seam: resolution happened before (§9.5 step 1) -------

    #[cfg(unix)]
    #[test]
    fn a_resolved_symlink_target_writes_the_real_file_and_keeps_the_link() {
        // T-F6's shape, driven from the caller side: the pipeline
        // resolves (T-062), this module renames over the resolved
        // path — the link itself is never the write target.
        use std::os::unix::fs::symlink;

        use crate::paths::resolve_target_path;

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        existing(dir.path(), "AGENTS.md", b"old");
        symlink("AGENTS.md", dir.path().join("CLAUDE.md")).expect("creating the symlink must work");

        let resolved = resolve_target_path(dir.path(), "CLAUDE.md").expect("the link must resolve");
        assert_eq!(resolved.file_name(), "AGENTS.md");

        write_target_file(resolved.path(), b"new", plenty_free())
            .expect("the write must land on the real file");

        assert_eq!(
            fs::read(dir.path().join("AGENTS.md")).expect("the real file"),
            b"new"
        );
        assert!(
            fs::symlink_metadata(dir.path().join("CLAUDE.md"))
                .expect("the link must still exist")
                .file_type()
                .is_symlink(),
            "the link survives the write"
        );
    }

    // ---- degenerate target shapes ----------------------------------------

    #[test]
    fn a_bare_relative_target_is_refused_without_staging_anything() {
        // The resolved targets of D-092 are absolute; a bare name has
        // no folder to stage in and must be refused, not staged into
        // the process's working directory.
        let err = write_target_file(Path::new("AGENTS.md"), b"abc", plenty_free())
            .expect_err("a bare relative target must be refused");

        let AtomicWriteError::WriteFailed(failed) = &err else {
            panic!("the refusal must be WriteFailed, got: {err:?}");
        };
        assert_eq!(failed.step, WriteStep::TempCreate);
        assert_eq!(failed.source.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(err.os_code(), None, "no OS call was made");
    }

    #[test]
    fn a_degenerate_file_name_is_refused_by_the_staging_grammar() {
        // A name the grammar (and the sweep) refuse: NUL can never be
        // a real file name, but it can be constructed — the builder
        // must refuse it before anything exists on disk. (`.`/`..`
        // never reach the builder: std's `file_name` normalizes them
        // away into the no-file-name refusal above.)
        let target = PathBuf::from("/repo/AGENTS\0.md");

        let err = write_target_file(&target, b"abc", plenty_free())
            .expect_err("a degenerate name must be refused");

        assert!(
            matches!(err, AtomicWriteError::NameFailed { .. }),
            "the staging-name builder must refuse it, got: {err:?}"
        );
        assert!(
            matches!(
                err.source(),
                Some(source) if source.to_string() == TmpNameError::InvalidTargetName.to_string()
            ),
            "the refusal is the builder's own typed error: {err:?}"
        );
    }

    // ---- the Windows replace arm -----------------------------------------

    #[cfg(windows)]
    #[test]
    fn the_windows_replace_swaps_an_existing_file() {
        // Runs on the CI Windows leg: the MoveFileExW arm replaces an
        // existing target with the staging file's bytes, in one call,
        // no retries.
        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"old");
        let staging = existing(dir.path(), "staging.tmp", b"new");

        replace_target(&staging, &target).expect("the replace must succeed");

        assert_eq!(
            fs::read(&target).expect("the target must hold the new bytes"),
            b"new"
        );
        assert!(!staging.exists(), "the staging file is gone");
    }

    #[cfg(windows)]
    #[test]
    fn a_target_open_without_share_delete_is_retried_then_file_locked() {
        // T-F8, the acceptance criterion: an editor holds the target
        // open without FILE_SHARE_DELETE — the REAL MoveFileExW
        // refuses (32 for this share-mode shape; 5 if the OS twists
        // through an ACL check — either of the two §9.5 codes), the
        // schedule runs between the real refusals, and the write ends
        // E_FILE_LOCKED with the original intact and the staging file
        // removed. The recording clock keeps the CI leg from really
        // sleeping 1.3 s; the refusals themselves are the OS's own.
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let dir = TempDir::new().expect("a temporary directory must be creatable");
        let target = existing(dir.path(), "AGENTS.md", b"untouched");
        {
            // FILE_SHARE_READ | FILE_SHARE_WRITE, deliberately NO
            // FILE_SHARE_DELETE: the replace must delete the target,
            // and the holder does not allow it.
            let _holder = fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(&target)
                .expect("the fixture must be able to hold the target open");
            let _clock = fault_injection::arm_pause_recording();

            let err = write_target_file(&target, b"abc", plenty_free())
                .expect_err("a held-open target must fail the write");

            let AtomicWriteError::FileLocked(failed) = &err else {
                panic!("the exhausted retries must be E_FILE_LOCKED, got: {err:?}");
            };
            assert_eq!(failed.step, WriteStep::Replace);
            assert_eq!(failed.path, target);
            assert_eq!(failed.retries, 3);
            assert!(
                matches!(err.os_code(), Some(32) | Some(5)),
                "a real §9.5 lock refusal rides for the log line: {:?}",
                err.os_code()
            );
            assert!(failed.cleanup.is_none(), "the staging file was removed");
            assert_eq!(
                fault_injection::recorded_lock_pauses(),
                [
                    Duration::from_millis(100),
                    Duration::from_millis(300),
                    Duration::from_millis(900),
                ],
                "the real schedule ran between the real refusals"
            );
            assert_eq!(
                fs::read(&target).expect("the target must survive"),
                b"untouched"
            );
            assert_eq!(entry_names(dir.path()), ["AGENTS.md"]);
        }
        // The holder is gone — §10's remedy is the idempotent retry:
        // the same write must now land.
        write_target_file(&target, b"abc", plenty_free())
            .expect("the write must land once the holder closed");
        assert_eq!(fs::read(&target).expect("the file must exist"), b"abc");
    }

    // ---- the error contract ----------------------------------------------

    #[test]
    fn display_source_chain_and_os_codes_follow_the_contract() {
        // WriteFailed: the step, the path and the OS error in the
        // text; the io error in the source chain.
        let write_failed = AtomicWriteError::WriteFailed(WriteFailed {
            step: WriteStep::Replace,
            path: PathBuf::from("/repo/AGENTS.md"),
            source: io::Error::from_raw_os_error(13),
            cleanup: None,
        });
        assert!(
            write_failed.to_string().starts_with(
                "the atomic write failed while replacing the target /repo/AGENTS.md: "
            ),
            "the Display must name the step and the path: {write_failed}"
        );
        assert_eq!(write_failed.os_code(), Some(13));
        assert_eq!(
            write_failed.source().map(|source| source.to_string()),
            Some(io::Error::from_raw_os_error(13).to_string())
        );

        // The exhausted Windows lock: the step, the path, the retry
        // count and the OS error in the text; the io error in the
        // source chain.
        let file_locked = AtomicWriteError::FileLocked(FileLockedFailure {
            step: WriteStep::Replace,
            path: PathBuf::from("/repo/AGENTS.md"),
            retries: 3,
            source: io::Error::from_raw_os_error(32),
            cleanup: None,
        });
        assert!(
            file_locked.to_string().starts_with(
                "the atomic write failed while replacing the target /repo/AGENTS.md: \
                 the file is locked by another process, 3 retries failed: "
            ),
            "the Display must name the step, the path and the retries: {file_locked}"
        );
        assert_eq!(file_locked.os_code(), Some(32));
        assert_eq!(
            file_locked.source().map(|source| source.to_string()),
            Some(io::Error::from_raw_os_error(32).to_string())
        );

        // The pre-write disk-full verdict: both numbers, no OS code.
        let pre_check = AtomicWriteError::DiskFull(DiskFullFailure {
            path: PathBuf::from("/repo/AGENTS.md"),
            step: None,
            free: Some(10),
            required: 12,
            source: None,
            cleanup: None,
        });
        assert_eq!(
            pre_check.to_string(),
            "not enough space on the volume of /repo/AGENTS.md: 10 bytes available, 12 required"
        );
        assert_eq!(pre_check.os_code(), None);
        assert!(pre_check.source().is_none());

        // The mid-write verdict: the OS error is the fact, the step
        // rides along.
        let mid_write = AtomicWriteError::DiskFull(DiskFullFailure {
            path: PathBuf::from("/repo/AGENTS.md"),
            step: Some(WriteStep::WriteSync),
            free: None,
            required: 12,
            source: Some(io::Error::from_raw_os_error(28)),
            cleanup: None,
        });
        assert!(
            mid_write
                .to_string()
                .starts_with("no space left while writing /repo/AGENTS.md: "),
            "the Display must lead with the OS fact: {mid_write}"
        );
        assert_eq!(mid_write.os_code(), Some(28));

        // The verify failures: digest and unreadable shapes.
        let digest = AtomicWriteError::VerifyFailed(VerifyFailed {
            path: PathBuf::from("/repo/AGENTS.md"),
            expected_sha256_hex: ABC_SHA256.to_owned(),
            mismatch: VerifyMismatch::Digest {
                actual_sha256_hex: EMPTY_SHA256.to_owned(),
            },
        });
        assert_eq!(
            digest.to_string(),
            format!(
                "the written file /repo/AGENTS.md failed the check: sha256 {ABC_SHA256} expected, {EMPTY_SHA256} found"
            )
        );
        assert_eq!(digest.os_code(), None);

        let unreadable = AtomicWriteError::VerifyFailed(VerifyFailed {
            path: PathBuf::from("/repo/AGENTS.md"),
            expected_sha256_hex: ABC_SHA256.to_owned(),
            mismatch: VerifyMismatch::Unreadable {
                source: io::Error::from_raw_os_error(13),
            },
        });
        assert!(
            unreadable.to_string().contains("could not be read back"),
            "the Display must state the refusal: {unreadable}"
        );
        assert_eq!(unreadable.os_code(), Some(13));

        // The name refusal chains to the builder's typed error.
        let name_failed = AtomicWriteError::NameFailed {
            source: TmpNameError::InvalidTargetName,
        };
        assert_eq!(
            name_failed.to_string(),
            format!(
                "the staging name was refused: {}",
                TmpNameError::InvalidTargetName
            )
        );
        assert!(name_failed.source().is_some());
    }
}
