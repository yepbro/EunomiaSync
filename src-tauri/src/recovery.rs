//! The database recovery operations behind the §10 recovery screen
//! (spec §7.10 command notes, §10 `E_DB_CORRUPT` row; DECISIONS.md
//! T-024).
//!
//! The screen appears exactly when the startup database open failed the
//! quick_check gate (D-045): at that moment there is no [`crate::db::Db`]
//! in managed state and no connection to run anything through — so these
//! operations deliberately do **not** use the `Db::run` executor (the
//! boundary D-037 drew: the executor serves commands over a *working*
//! database). Recovery is plain filesystem work plus the same bare,
//! read-only startup gates the open chain runs (`reject_too_new`,
//! `reject_corrupt` — re-used, not re-implemented: one detection
//! mechanic for the live file and for the copies it may be replaced
//! from), all over paths derived from the data root the `setup` hook
//! resolved and published ([`crate::data_dirs::DataRoot`]).
//!
//! Three properties are load-bearing:
//!
//! - **The damaged file is never deleted.** Both `db_recreate` and
//!   `db_restore_backup` first *rename* the damaged database into
//!   `db-backups/` as `pre-recovery-<UTC>.sqlite3` — the exact name the
//!   §7.10 command notes fix — with its `-journal`/`-wal`/`-shm`
//!   siblings renamed alongside (they may hold state of the damaged
//!   database, and a stale sibling left in `db/` would be presented to
//!   the *next* database file SQLite opens there). Renaming the
//!   siblings first and the main file last makes the main rename the
//!   commit point of the preserve step: a failure halfway leaves the
//!   damaged file in place and the step re-runnable.
//! - **A chosen copy is validated before anything is applied.** The
//!   copy's name must be a plain `.sqlite3` file name (no separators,
//!   no `..`, no `.tmp` staging debris, no `-wal`/`-shm`/`-journal`
//!   sibling — the UI sends a base name, never a path, SEC-009's
//!   spirit), and the file itself must pass both startup gates: a copy
//!   made by a newer app answers `E_DB_TOO_NEW`, a damaged copy
//!   answers `E_DB_CORRUPT`, *before* the live file is touched.
//! - **The replacement is staged before the preserve and published
//!   after it.** The copy is staged next to the final path
//!   (`db/eunomiasync.sqlite3.tmp`, mode `0600` per D-048, fsynced)
//!   while the live file is still in place — so the most failure-prone
//!   step (copying bytes: a full disk, a permission, an unreadable
//!   source) cannot leave `db/` without a database — then the preserve
//!   runs, then one rename publishes the staged copy (§9.5's staging
//!   pattern, parent-directory fsync included). The chosen copy in
//!   `db-backups/` is never opened for writing, and the restore itself
//!   never removes it (T-253's age-based retention may retire an aged
//!   copy later, at the reopen — never while the restore is in flight).
//!
//! The commands in `commands.rs` compose these operations with
//! `Db::open` (which re-runs the full gate chain over the replaced
//! file) and publish the reopened database as managed state; this
//! module owns only the file-level mechanics, so its behavior is
//! testable against real files without a Tauri app.

use std::io;
use std::path::{Component, Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::data_dirs;
use crate::db::{self, DbInitError};
use crate::errors::{AppError, ErrorCode, ErrorDetails};

/// The suffix that marks staging files of every copy scheme (the
/// store's pre-migration/daily staging, this module's restore staging).
/// [`backup_path`] rejects them as a selection — they are half-written
/// artifacts of a crashed run, not copies a user may pick. (The
/// listing needs no separate check: a staging name does not end in
/// `.sqlite3`, which is the listing's own filter.)
const STAGING_SUFFIX: &str = ".tmp";

/// The extension of every database copy in `db-backups/` — and the
/// listing's membership rule: only `*.sqlite3` files are copies
/// (pre-migration, daily, pre-recovery), never the `-wal`/`-shm`/
/// `-journal` siblings a preserve renames alongside, never a user's
/// foreign file.
const COPY_EXTENSION: &str = ".sqlite3";

/// The prefix of a preserved damaged database (§7.10 command notes:
/// `db-backups/pre-recovery-<UTC>.sqlite3`).
const PRESERVED_PREFIX: &str = "pre-recovery-";

/// The reason param of a [`RecoveryError::RefusedHealthy`] rejection.
const REFUSED_HEALTHY_REASON: &str = "a working database is already in place — restart the app";

/// The maximum number of name attempts for one preserved file (the
/// millisecond stamp plus `-1`…`-<CAP>` suffixed variants) before the
/// operation gives up — the same guard against a pathological
/// directory the store's backup naming uses (D-038).
const PRESERVED_NAME_ATTEMPT_CAP: u32 = 1000;

/// One row of `db_backups_list` — a copy in `db-backups/` as the §10
/// recovery screen renders it: the file's base name (what
/// `db_restore_backup` takes back as `backup_name`), its size, and its
/// modification time in UTC ISO-8601. Fields are the screen's own
/// facts; the copy's *kind* (pre-migration, daily, pre-recovery) is
/// readable from the name and deliberately not duplicated as a field.
///
/// `size_bytes` is a `u32`, not the file system's `u64`: specta
/// forbids BigInt-style types on the TypeScript wire (precision
/// loss), and a database copy beyond 4 GiB is far outside this app's
/// envelope — a bigger copy reports the saturated `u32::MAX`, an
/// honest display fact for a file the screen could not meaningfully
/// render anyway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub(crate) struct DbBackupInfo {
    /// The copy's file name inside `db-backups/` — a plain base name,
    /// the exact string `db_restore_backup` expects back.
    pub(crate) name: String,
    /// The copy's size in bytes (saturated at `u32::MAX`; see the
    /// struct docs).
    pub(crate) size_bytes: u32,
    /// The copy's modification time, UTC ISO-8601 (`RFC 3339`, `Z`
    /// suffix) — the string the screen shows; parsing stays the
    /// frontend's choice.
    pub(crate) modified_utc: String,
}

/// Why a selected backup name was rejected before any file was touched.
/// A closed vocabulary (the §10 log line and the UI's message params
/// carry it); every variant maps to `E_VALIDATION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvalidBackupName {
    /// Not a plain file name: empty, `.`, `..`, a path with separators,
    /// or any other multi-component path spelling.
    NotAFileName,
    /// A staging artifact of some copy scheme (`*.tmp`), not a copy.
    StagingFile,
    /// Not a database copy name (`*.sqlite3`): a `-wal`/`-shm`/
    /// `-journal` sibling of some preserve, or a foreign file. The
    /// listing never offers these; the restore must not accept them
    /// either — a non-empty sibling fails the gates, but a 0-byte one
    /// would pass both (the gates skip empty files) and silently
    /// bootstrap an empty database.
    NotADatabaseCopy,
    /// No regular file with this name exists in `db-backups/`.
    Missing,
}

impl InvalidBackupName {
    /// The word the `reason` interpolation param carries.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            InvalidBackupName::NotAFileName => "not a plain file name",
            InvalidBackupName::StagingFile => "a staging file, not a copy",
            InvalidBackupName::NotADatabaseCopy => "not a database copy",
            InvalidBackupName::Missing => "no such copy",
        }
    }
}

/// Why a recovery operation failed. Kept as data (the `DataLayoutError`
/// shape): the wire form is [`RecoveryError::app_error`], constructed at
/// the command boundary where the §10 code choice is contextual (the
/// D-034 policy — no blanket `From`).
#[derive(Debug)]
pub(crate) enum RecoveryError {
    /// The selected backup name was rejected before anything was
    /// touched. Wire form: `E_VALIDATION` (a field the UI supplied
    /// failed validation — the §10 row's own situation).
    InvalidBackupName {
        /// The rejected name, as received.
        name: String,
        /// Which validation failed.
        reason: InvalidBackupName,
    },
    /// `db_recreate` refused because the database path already holds a
    /// file this app can open (both startup gates pass over it):
    /// "start with an empty database" is the screen's last resort, and
    /// silently discarding a *working* unmanaged database (the state a
    /// failed reopen after a successful restore leaves behind) is the
    /// one way this command could destroy data for nothing. Wire form:
    /// `E_VALIDATION` — nothing was touched.
    RefusedHealthy {
        /// The healthy database file the operation refused to discard.
        path: PathBuf,
    },
    /// A filesystem operation of the preserve/replace mechanics failed.
    /// The wire code is contextual on the OS cause: permission failures
    /// are `E_NO_WRITE_ACCESS`, a full volume `E_DISK_FULL`, and
    /// everything else lands on the recorded D-037 fallback
    /// (`E_DB_LOCKED`, "operation rejected, retry") with the true
    /// reason in `details` — the §10 registry has no generic I/O code.
    Io {
        /// The step that failed ("secure", "preserve", "copy", …).
        op: &'static str,
        /// The path the step was working on.
        path: PathBuf,
        /// The filesystem failure.
        source: io::Error,
    },
    /// A startup gate rejected the selected copy (too-new/corrupt), or
    /// the reopen after the replacement failed. The wire form is the
    /// typed `DbInitError::app_error()` where one exists; a generic
    /// init failure keeps the D-037 fallback like [`RecoveryError::Io`]
    /// — its startup shape is the D-027 abort, which a command cannot
    /// perform, so the honest command-level equivalent is a retryable
    /// error with the reason attached.
    DbInit(Box<DbInitError>),
}

impl RecoveryError {
    /// Builds the §7.10 wire form of the failure (the command boundary
    /// calls this; see the variant docs for the code choices).
    pub(crate) fn app_error(&self) -> AppError {
        match self {
            RecoveryError::InvalidBackupName { name, reason } => {
                let mut error = AppError::new(ErrorCode::Validation);
                error.params.insert("name".to_owned(), name.clone());
                error
                    .params
                    .insert("reason".to_owned(), reason.as_str().to_owned());
                error
            }
            RecoveryError::RefusedHealthy { path } => {
                let mut error = AppError::new(ErrorCode::Validation);
                error
                    .params
                    .insert("reason".to_owned(), REFUSED_HEALTHY_REASON.to_owned());
                error
                    .params
                    .insert("path".to_owned(), path.display().to_string());
                error
            }
            RecoveryError::Io { op, path, source } => {
                let code = match source.kind() {
                    io::ErrorKind::PermissionDenied => ErrorCode::NoWriteAccess,
                    io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
                        ErrorCode::DiskFull
                    }
                    _ => ErrorCode::DbLocked,
                };
                let mut error = AppError::new(code);
                error.params.insert("op".to_owned(), (*op).to_owned());
                error
                    .params
                    .insert("path".to_owned(), path.display().to_string());
                error.with_os_error(source)
            }
            RecoveryError::DbInit(init) => init.app_error().unwrap_or_else(|| {
                // The generic init failure (an open error that is not a
                // typed §10 situation): the D-037 fallback shape — the
                // least harmful of the twenty codes — with the init
                // failure's own text as the diagnostic.
                let mut error = AppError::new(ErrorCode::DbLocked);
                error.details = Some(ErrorDetails {
                    os_code: None,
                    os_kind: Some("DbInitError".to_owned()),
                    os_message: Some(init.to_string()),
                });
                error
            }),
        }
    }
}

impl From<DbInitError> for RecoveryError {
    fn from(init: DbInitError) -> Self {
        RecoveryError::DbInit(Box::new(init))
    }
}

/// Where a damaged database was preserved: renamed from `from` (the
/// `db/eunomiasync.sqlite3` path) to `to`
/// (`db-backups/pre-recovery-<UTC>.sqlite3`). Returned so the command
/// can log the audit line with both paths (SEC-008 facade).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Preserved {
    pub(crate) from: PathBuf,
    pub(crate) to: PathBuf,
}

/// Lists every copy in `db-backups/` for the recovery screen (§10):
/// the `*.sqlite3` regular files — pre-migration, daily and
/// pre-recovery copies alike — newest modification time first (ties
/// broken by name, so the order is deterministic). Directories,
/// symlinks, non-UTF-8 names, staging debris (`*.tmp` — not
/// `.sqlite3`-shaped) and the `-wal`/`-shm`/`-journal` siblings a
/// preserve renames alongside are all skipped: `db-backups/` is
/// app-written, and only a database file is a copy this screen can
/// act on (a 0-byte sibling would even pass both restore gates —
/// they skip empty files — and silently bootstrap an empty database).
///
/// # Errors
///
/// [`RecoveryError::Io`] when the directory cannot be read — with the
/// directory as the `path` param, the honest "nothing to list from"
/// rather than a silent empty list.
pub(crate) fn list_backups(backups_dir: &Path) -> Result<Vec<DbBackupInfo>, RecoveryError> {
    let mut entries: Vec<(std::time::SystemTime, String, u64)> = Vec::new();
    for entry in std::fs::read_dir(backups_dir).map_err(|source| RecoveryError::Io {
        op: "list",
        path: backups_dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| RecoveryError::Io {
            op: "list",
            path: backups_dir.to_path_buf(),
            source,
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.ends_with(COPY_EXTENSION) {
            continue;
        }
        // The entry's own metadata (`DirEntry::metadata` does not
        // follow symlinks — the no-follow rule the restore selection
        // applies too): a symlink to a database is not a copy this app
        // wrote, and following it would let a planted link answer for
        // a file elsewhere.
        let metadata = entry.metadata().map_err(|source| RecoveryError::Io {
            op: "list",
            path: backups_dir.join(&name),
            source,
        })?;
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().map_err(|source| RecoveryError::Io {
            op: "list",
            path: backups_dir.join(&name),
            source,
        })?;
        entries.push((modified, name, metadata.len()));
    }
    // Newest first; ties by name — both pinned by tests.
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(entries
        .into_iter()
        .map(|(modified, name, size_bytes)| DbBackupInfo {
            name,
            size_bytes: u32::try_from(size_bytes).unwrap_or(u32::MAX),
            modified_utc: iso_utc(modified),
        })
        .collect())
}

/// The file mechanics of `db_recreate` (§7.10: "keeps the damaged
/// database as `db-backups/pre-recovery-<UTC>.sqlite3` … and starts
/// with an empty database"): refuse when the database path already
/// holds a healthy file (see [`RecoveryError::RefusedHealthy`] — the
/// empty start is the screen's last resort, never a way to silently
/// discard a working unmanaged database), then preserve the damaged
/// file, and nothing else — the empty database is created by the
/// subsequent `Db::open` over the now-missing path (the store's
/// `open_db` bootstraps a fresh file; a fresh bootstrap is exactly
/// the "start with an empty database" the spec names).
///
/// Returns where the damaged file was preserved, or `None` when there
/// was no database file at all (a hand-deleted `db/` — nothing to
/// preserve, the fresh bootstrap is still the right outcome).
///
/// # Errors
///
/// [`RecoveryError::RefusedHealthy`] when the live file passes both
/// startup gates (the wire form is `E_VALIDATION`; nothing was
/// touched); [`RecoveryError::Io`] on any preserve step (see
/// [`preserve_damaged`] for the ordering that keeps the step
/// re-runnable).
pub(crate) fn recreate_files(
    data_dir: &Path,
    now: Timestamp,
) -> Result<Option<Preserved>, RecoveryError> {
    let db_path = data_dirs::db_file_path(data_dir);
    refuse_healthy(&db_path)?;
    preserve_damaged(&db_path, &data_dirs::db_backups_dir(data_dir), now)
}

/// The `db_recreate` guard: when the database path holds an existing,
/// non-empty file both startup gates accept (not too new, quick_check
/// clean), "start with an empty database" must refuse — the honest
/// states of recovery mode all carry a file the gates *reject* (the
/// screen exists because of that verdict); a healthy unmanaged file
/// means a restore placed a copy and the reopen failed, and the right
/// answers are retrying the restore or restarting the app, not
/// discarding a working database. A missing, empty, too-new or
/// damaged file proceeds — the guard must never block the genuine
/// recovery paths.
fn refuse_healthy(db_path: &Path) -> Result<(), RecoveryError> {
    let exists_non_empty = std::fs::metadata(db_path).is_ok_and(|metadata| metadata.len() > 0);
    if exists_non_empty
        && db::reject_too_new(db_path).is_ok()
        && db::reject_corrupt(db_path).is_ok()
    {
        return Err(RecoveryError::RefusedHealthy {
            path: db_path.to_path_buf(),
        });
    }
    Ok(())
}

/// The file mechanics of `db_restore_backup` (§7.10: "keeps the damaged
/// database as `db-backups/pre-recovery-<UTC>.sqlite3`, replaces it
/// with the chosen copy, and reopens it"): validate the selection,
/// **stage the copy first**, preserve the damaged file, then publish
/// the staged copy at the database path. The reopen is the command's
/// next step, not this function's.
///
/// Staging before preserving is the load-bearing order (round-1
/// review finding): the most failure-prone step — copying the chosen
/// backup, which fails on a full disk, a permission, an unreadable
/// source — runs while the live file is still in place, so its
/// failure leaves `db/` exactly as it was. The window in which the
/// database path is empty shrinks to the two renames of the preserve
/// and the publish; `on_preserved` runs inside that window, after the
/// damaged file has moved and before anything replaces it — the
/// command passes its audit line there, so a crash between the renames
/// still leaves the "where did my old database go?" trail.
///
/// Returns where the damaged file was preserved (`None` when there was
/// no database file — the copy still becomes the database).
///
/// # Errors
///
/// [`RecoveryError::InvalidBackupName`] when the selection is not a
/// plain `.sqlite3` copy name or names no copy; the gate rejections
/// (`E_DB_TOO_NEW`/`E_DB_CORRUPT` wire forms) when the copy itself is
/// not applicable; [`RecoveryError::Io`] for the stage/preserve/
/// publish mechanics. No error path deletes or overwrites anything
/// the user could not reproduce: a failure before the preserve leaves
/// the damaged file in place, one after it leaves it preserved under
/// its `pre-recovery-` name.
pub(crate) fn restore_files<F>(
    data_dir: &Path,
    backup_name: &str,
    now: Timestamp,
    on_preserved: F,
) -> Result<Option<Preserved>, RecoveryError>
where
    F: FnOnce(&Preserved),
{
    let backups_dir = data_dirs::db_backups_dir(data_dir);
    let source = backup_path(&backups_dir, backup_name)?;
    validate_backup(&source)?;
    let db_path = data_dirs::db_file_path(data_dir);
    stage_copy(&source, &db_path)?;
    let preserved = preserve_damaged(&db_path, &backups_dir, now)?;
    if let Some(preserved) = &preserved {
        on_preserved(preserved);
    }
    publish_copy(&db_path)?;
    Ok(preserved)
}

/// Runs the startup gates over a chosen copy (T-018/T-020 mechanics,
/// BYO as the card requires): a copy this app cannot open must never
/// replace the live file — the preserve step of the restore is
/// destructive-adjacent, so validation completes before anything is
/// staged or moved. The gates open bare, read-only connections and
/// skip missing/0-byte files (a 0-byte daily copy of a fresh database
/// is a valid empty database — D-046's recorded case).
///
/// # Errors
///
/// The gate rejections in their typed wire forms: too-new →
/// `E_DB_TOO_NEW`, damaged → `E_DB_CORRUPT` (both via
/// [`RecoveryError::DbInit`]).
pub(crate) fn validate_backup(source: &Path) -> Result<(), RecoveryError> {
    db::reject_too_new(source)?;
    db::reject_corrupt(source)?;
    Ok(())
}

/// Resolves a UI-supplied selection to a copy path inside
/// `db-backups/`: the name must be a single normal path component — no
/// separators, no `.`, no `..`, so the joined path cannot leave the
/// directory (the escape attempts §10's recovery screen must survive) —
/// must not be staging debris, and must be a `.sqlite3` copy name (a
/// `-wal`/`-shm`/`-journal` sibling or a foreign file is not a copy).
/// The existence check uses `symlink_metadata` (no following): a
/// symlink planted in the app-owned directory is not a copy this app
/// wrote.
pub(crate) fn backup_path(backups_dir: &Path, name: &str) -> Result<PathBuf, RecoveryError> {
    let invalid = |reason: InvalidBackupName| RecoveryError::InvalidBackupName {
        name: name.to_owned(),
        reason,
    };
    let mut components = Path::new(name).components();
    let plain_name = matches!(
        components.next(),
        Some(Component::Normal(_)) if components.next().is_none()
    );
    if !plain_name {
        return Err(invalid(InvalidBackupName::NotAFileName));
    }
    if name.ends_with(STAGING_SUFFIX) {
        return Err(invalid(InvalidBackupName::StagingFile));
    }
    if !name.ends_with(COPY_EXTENSION) {
        return Err(invalid(InvalidBackupName::NotADatabaseCopy));
    }
    let path = backups_dir.join(name);
    if !path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_file())
    {
        return Err(invalid(InvalidBackupName::Missing));
    }
    Ok(path)
}

/// Preserves the damaged database (§7.10's "keeps the damaged
/// database"): chmod it to `0600` (D-048 — this step creates a new
/// artifact in `db-backups/`, and the layer that creates the artifact
/// enforces its mode), rename any `-journal`/`-wal`/`-shm` siblings
/// alongside the target name, then rename the main file — the commit
/// point of the step. Sibling renames happen *first* (and happen even
/// when the main file is missing: a sibling left in `db/` would be
/// presented to whatever database file opens there next — a stale
/// `-wal` applied to a fresh bootstrap, a hot `-journal` rolled back
/// into it), so a failure leaves the damaged file at its original
/// path and the whole step re-runnable with a fresh target name.
/// Sibling chmods are best-effort (they are transient SQLite-owned
/// artifacts, and a mode repair must not fail the preserve over data
/// that is already being moved); the main file's chmod is required.
///
/// The target name is `pre-recovery-<UTC>.sqlite3` with the store's
/// stamp shape (`YYYYMMDDTHHMMSSSSSZ`, millisecond resolution,
/// filesystem-safe) and `-N` suffix variants on a same-millisecond
/// collision — unique on every platform (renaming onto an existing
/// target fails on Windows, so uniqueness is a correctness requirement
/// there, not polish; the name probe checks the sibling spellings
/// too, so a half-completed earlier attempt cannot break a retry).
pub(crate) fn preserve_damaged(
    db_path: &Path,
    backups_dir: &Path,
    now: Timestamp,
) -> Result<Option<Preserved>, RecoveryError> {
    let stamp = now.strftime("%Y%m%dT%H%M%S%3fZ").to_string();
    let target = free_preserved_path(backups_dir, &stamp)?;
    // The siblings first, the main file last — see the method docs.
    // `-journal` shares the hazard (a hot rollback journal would be
    // rolled back into the *next* database file), so it travels with
    // the same mechanics.
    for suffix in SIDECAR_SUFFIXES {
        let sibling = append_suffix(db_path, suffix);
        if sibling.exists() {
            secure_owner_only_best_effort(&sibling);
            rename(&sibling, &append_suffix(&target, suffix))?;
        }
    }
    if !db_path.exists() {
        // No main file (hand-deleted `db/`, or an earlier preserve
        // that moved it and died before the caller retried): the
        // siblings above are still cleared out of `db/`, and there is
        // nothing to preserve.
        return Ok(None);
    }
    secure_owner_only(db_path)?;
    rename(db_path, &target)?;
    // No retention pass here, deliberately (T-253, D-126): a preserve
    // inside `restore_files` runs while the user's chosen copy is still
    // the only durable source of the bytes being restored — the staged
    // `.tmp` is debris-swept by the next attempt, so deleting an aged
    // chosen copy at this point would make a failed publish unrecover-
    // able. The reopen every recovery operation performs (`Db::open`)
    // runs the pass instead, once the restore has succeeded.
    Ok(Some(Preserved {
        from: db_path.to_path_buf(),
        to: target,
    }))
}

/// The sibling spellings SQLite creates next to a database file
/// (spec §5.5 names `-wal`/`-shm`; `-journal` is the rollback-journal
/// sibling of a database not in WAL mode).
const SIDECAR_SUFFIXES: [&str; 3] = ["-journal", "-wal", "-shm"];

/// Finds an unoccupied preserve target under `backups_dir` for the
/// given stamp: `pre-recovery-<stamp>.sqlite3`, then
/// `pre-recovery-<stamp>-<N>.sqlite3` up to
/// [`PRESERVED_NAME_ATTEMPT_CAP`] attempts — unoccupied including the
/// sibling spellings (`-journal`/`-wal`/`-shm`), so a half-completed
/// earlier attempt at the same stamp cannot make a sibling rename
/// collide. A pathological directory that exhausts the cap is an I/O
/// failure ("preserve"), not a silent overwrite of an earlier
/// preserve.
fn free_preserved_path(backups_dir: &Path, stamp: &str) -> Result<PathBuf, RecoveryError> {
    for attempt in 0..=PRESERVED_NAME_ATTEMPT_CAP {
        let name = match attempt {
            0 => format!("{PRESERVED_PREFIX}{stamp}{COPY_EXTENSION}"),
            n => format!("{PRESERVED_PREFIX}{stamp}-{n}{COPY_EXTENSION}"),
        };
        let candidate = backups_dir.join(name);
        if !candidate.exists()
            && SIDECAR_SUFFIXES.iter().all(|suffix| {
                let sibling = append_suffix(&candidate, suffix);
                !sibling.exists()
            })
        {
            return Ok(candidate);
        }
    }
    Err(RecoveryError::Io {
        op: "preserve",
        path: backups_dir.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free pre-recovery name found",
        ),
    })
}

/// Stages a copy of `source` next to `db_path` as
/// `<db_path>.tmp` (the §9.5 staging pattern the store's copy schemes
/// use — same directory, so the final rename is atomic): copy, enforce
/// `0600` on the staged file (D-048: the staging name must never carry
/// wider bits than the published one), fsync it (a crash after the
/// publish must not expose a torn database). A stale staging file of
/// an earlier crashed run is removed first — it is debris, never
/// data. Staging deliberately touches nothing at `db_path` itself: it
/// is the step that runs *before* the preserve, so its failures (a
/// full disk, a permission, an unreadable copy) leave the live file
/// exactly where it was.
///
/// The chosen copy in `db-backups/` is only ever read: the staged
/// bytes are a copy, not a hard link, because a link would make every
/// later SQLite write mutate the backup it came from.
pub(crate) fn stage_copy(source: &Path, db_path: &Path) -> Result<(), RecoveryError> {
    let staging = staging_path(db_path);
    if staging.exists() {
        std::fs::remove_file(&staging).map_err(|source_error| RecoveryError::Io {
            op: "clean staging",
            path: staging.clone(),
            source: source_error,
        })?;
    }
    std::fs::copy(source, &staging).map_err(|source_error| RecoveryError::Io {
        op: "copy",
        path: staging.clone(),
        source: source_error,
    })?;
    secure_owner_only(&staging)?;
    // The durability fsync goes through a write handle: Windows'
    // FlushFileBuffers requires GENERIC_WRITE and fails with
    // ERROR_ACCESS_DENIED (os error 5) on the read-only handle
    // `File::open` yields, while Unix fsyncs read-only descriptors
    // fine (DECISIONS.md D-074, T-241). The handle only flushes;
    // nothing is written through it, so the staged bytes are
    // untouched.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&staging)
        .map_err(|source_error| RecoveryError::Io {
            op: "sync",
            path: staging.clone(),
            source: source_error,
        })?;
    file.sync_all().map_err(|source_error| RecoveryError::Io {
        op: "sync",
        path: staging.clone(),
        source: source_error,
    })
}

/// Publishes the staged copy: rename `<db_path>.tmp` into place (the
/// atomic step), then fsync the parent directory — §9.5's step 5: on
/// a crash the directory entry of the rename must survive with the
/// file, or the next startup finds no database and bootstraps fresh.
/// The parent fsync is best-effort (`File::open` of a directory is
/// not available everywhere): an exotic filesystem that refuses it
/// must not turn a completed replacement into a reported failure —
/// the staged file's own fsync already ran. On Windows this arm is a
/// de-facto no-op (a directory cannot be opened for a flushing handle
/// this way; D-074) — the durability price is one rename whose
/// directory entry the OS flushes on its own schedule.
pub(crate) fn publish_copy(db_path: &Path) -> Result<(), RecoveryError> {
    if db_path.exists() {
        return Err(RecoveryError::Io {
            op: "publish",
            path: db_path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the database path is still occupied; preserve runs first",
            ),
        });
    }
    std::fs::rename(staging_path(db_path), db_path).map_err(|source_error| RecoveryError::Io {
        op: "publish",
        path: db_path.to_path_buf(),
        source: source_error,
    })?;
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
    }
    Ok(())
}

/// The staging path of a publish: `<db_path>.tmp` (the same spelling
/// every copy scheme's staging uses).
fn staging_path(db_path: &Path) -> PathBuf {
    let mut name = db_path.as_os_str().to_os_string();
    name.push(STAGING_SUFFIX);
    PathBuf::from(name)
}

/// Renames `from` to `to` with the op/path vocabulary of
/// [`RecoveryError::Io`].
fn rename(from: &Path, to: &Path) -> Result<(), RecoveryError> {
    std::fs::rename(from, to).map_err(|source| RecoveryError::Io {
        op: "preserve",
        path: from.to_path_buf(),
        source,
    })
}

/// Appends `suffix` to a path's file name.
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Sets `0600` on Unix (SEC-007/D-048 — a preserve target and a staged
/// copy are database data at rest), no-op on Windows where the rights
/// come from the user profile. A failure aborts the calling step
/// before any rename: continuing would move user data into a backups
/// directory with wider bits than every other copy in it.
#[cfg(unix)]
fn secure_owner_only(path: &Path) -> Result<(), RecoveryError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        RecoveryError::Io {
            op: "secure",
            path: path.to_path_buf(),
            source,
        }
    })
}

/// The Windows half of [`secure_owner_only`]: nothing to enforce
/// (SEC-007 assigns the rights to the user profile).
#[cfg(not(unix))]
fn secure_owner_only(_path: &Path) -> Result<(), RecoveryError> {
    Ok(())
}

/// The best-effort sibling of [`secure_owner_only`]: the same `0600`
/// on Unix, but a failure is swallowed — used for the `-journal`/
/// `-wal`/`-shm` siblings of a preserve (transient SQLite-owned
/// artifacts already on their way out of `db/`; the D-048-critical
/// modes are the main file's and the staged copy's, both enforced
/// hard).
#[cfg(unix)]
fn secure_owner_only_best_effort(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// The non-Unix half of [`secure_owner_only_best_effort`]: nothing to
/// enforce (SEC-007 assigns the rights to the user profile).
#[cfg(not(unix))]
fn secure_owner_only_best_effort(_path: &Path) {}

/// Formats a file's modification time as UTC ISO-8601 (`RFC 3339`,
/// `Z` suffix) — jiff's own `Timestamp` rendering, reached through the
/// `TryFrom<SystemTime>` conversion. Unrepresentable times (before the
/// Unix epoch on a platform with a coarse clock) fall back to the
/// epoch's rendering rather than failing the listing: the mtime is a
/// display fact, not a gate.
fn iso_utc(modified: std::time::SystemTime) -> String {
    Timestamp::try_from(modified)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_string()
}
