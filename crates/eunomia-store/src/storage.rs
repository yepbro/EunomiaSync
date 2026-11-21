//! SQLite storage for EunomiaSync.
//!
//! The database schema is managed with SQL migration files embedded into
//! the binary via [`include_str!`]. `rusqlite_migration` tracks the applied
//! schema version through the SQLite `user_version` pragma (spec DB-004),
//! so upgrading an existing database only requires adding the next
//! migration file. Schema version 1 — the eleven tables of spec §6.2 — is
//! defined by `migrations/0001_init.sql`.
//!
//! Before every migration applied to an existing database,
//! [`run_migrations`] writes a safety copy of the current state into a
//! caller-provided backup directory with SQLite's `VACUUM INTO` (spec
//! DB-004, UPD-004), and each migration is committed in its own
//! transaction. A migration that fails is rolled back alone and the
//! failure carries the path of the copy that preserves the state the
//! database is still at — the app layer's "install the previous
//! version" pointer (spec UPD-004).
//!
//! The same directory also holds the daily startup copies of spec §10
//! ERR-001 (`daily-YYYYMMDD.sqlite3`): [`daily_backup`] writes at most
//! one copy per UTC day and keeps the last 7, reusing the
//! pre-migration copy mechanics (staging, publication, retries — see
//! [`run_migrations`] and DECISIONS.md D-038).
//!
//! Every file this crate creates is owner-only on Unix (spec §13
//! SEC-007): the database file is set to `0600` in [`open_db`] before
//! the pragma chain — so no data is written through looser modes — and
//! each backup copy in [`stage_and_link_copy`] is `0600` before the
//! link publishes it under its final name (the copies hold the same
//! data as the database, so they share its sensitivity; DECISIONS.md
//! D-048). On non-Unix platforms nothing is chmod'ed: the rights are
//! inherited from the user profile (SEC-007). The backup
//! *directory's* `0700` is the app layer's to enforce — directories
//! belong to the data-directory layout (DECISIONS.md D-027), which
//! this crate never creates.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

use crate::action_log::ActionLogError;
use crate::settings::SettingsError;

/// The ordered list of embedded SQL migrations applied by [`run_migrations`].
///
/// Version 1 (`migrations/0001_init.sql`) creates the full §6.2 schema:
/// `settings`, `projects`, `project_exclusions`, `library_items`,
/// `instruction_variants`, `project_instructions`, `revisions`,
/// `sync_runs`, `sync_run_projects`, `backup_records` and `action_log`,
/// with every CHECK, UNIQUE and foreign-key constraint plus the indexes
/// the spec names (including the extra `project_instructions` indexes on
/// `instruction_id` and `variant_id`). After it runs,
/// `PRAGMA user_version` is 1.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

/// Builds the [`Migrations`] set from the embedded SQL files.
fn migrations() -> Migrations<'static> {
    Migrations::new(MIGRATIONS.iter().map(|sql| M::up(sql)).collect())
}

/// The schema version of a fully migrated database (spec DB-004): the
/// number of migrations in the embedded set, which is the
/// `PRAGMA user_version` value [`run_migrations`] leaves behind
/// (versions are the 1-based positions in [`MIGRATIONS`], applied in
/// order by `Migrations::to_version`).
///
/// The app layer reads this to reject a database *newer* than the app
/// before opening it with the DB-001 pragma set ([`open_db`]): the WAL
/// switch writes to the file, and a too-new database must not be
/// touched at all (spec §10 `E_DB_TOO_NEW`: "The app does not change
/// the database"). Schema knowledge stays in this crate — the number
/// travels, the set does not.
pub fn latest_schema_version() -> usize {
    MIGRATIONS.len()
}

/// The busy handler timeout applied by [`open_db`] (spec DB-001).
///
/// Set through [`Connection::busy_timeout`], which programs the same SQLite
/// busy handler as the `busy_timeout` pragma; reading `PRAGMA busy_timeout`
/// back reports this value.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Errors that can occur while opening or migrating the database.
#[derive(Debug)]
pub enum StorageError {
    /// A migration failed to apply. The failed step's own transaction
    /// was rolled back, so the database keeps the schema and data of
    /// its last committed step. `backup` is the pre-migration copy of
    /// exactly that state — the copy written immediately before the
    /// failed step, spec UPD-004's pointer for the app layer's
    /// "install the previous version" message — or `None` when the run
    /// wrote no copy: a fresh bootstrap (the database was created by
    /// this run) or a rejection before any step ran.
    Migration {
        /// The failed migration step.
        source: rusqlite_migration::Error,
        /// The copy of the state the database is still at, when the
        /// run wrote one.
        backup: Option<PathBuf>,
    },
    /// A SQL statement failed.
    Sql(rusqlite::Error),
    /// Opening the database file or applying its configuration failed.
    Open {
        /// The database file path.
        path: PathBuf,
        /// The SQLite error that caused the failure.
        source: rusqlite::Error,
    },
    /// The database could not be switched to write-ahead logging.
    ///
    /// `PRAGMA journal_mode=WAL` reports the achieved mode; anything other
    /// than `wal` means the file cannot use WAL (for example on a network
    /// filesystem) and the crash-safety guarantees (spec NFR-007) are not
    /// met.
    JournalMode {
        /// The database file path.
        path: PathBuf,
        /// The journal mode SQLite reported after the update attempt.
        mode: String,
    },
    /// Writing a backup copy failed (spec DB-004 pre-migration, spec
    /// §10 ERR-001 daily). Which scheme failed is visible from the
    /// path's name and the call site.
    Backup {
        /// The backup path the run was producing (the final file name;
        /// its staged temporary is cleaned up best-effort), or the
        /// backup directory itself when it is missing.
        path: PathBuf,
        /// The step that failed.
        source: BackupError,
    },
    /// Reading or writing a setting failed (spec §6.5): the value does
    /// not fit the key's type or range, or the stored JSON text is
    /// corrupt.
    Settings {
        /// The dotted settings key, e.g. `backup.retention`.
        key: String,
        /// The validation or decoding failure.
        source: SettingsError,
    },
    /// Reading an action-log entry failed (spec §7.7): a stored cell
    /// is outside its closed vocabulary, or a `details` cell is not
    /// valid JSON — corruption or a row written by a newer app
    /// version (the D-039 corrupt-cells policy, applied to the
    /// journal). Only the read path constructs this variant; a
    /// failing write surfaces as [`StorageError::Sql`].
    ActionLog {
        /// The validation or decoding failure.
        source: ActionLogError,
    },
    /// Setting the required owner-only permissions on the database
    /// file failed (spec §13 SEC-007, Unix): without them the data
    /// would sit readable by other local users, so the open fails
    /// instead of continuing with looser modes. The staged backup
    /// copies report the same failure as [`StorageError::Backup`]
    /// with [`BackupError::Io`] — a failed copy is not a failed
    /// open — so this variant is only produced by [`open_db`].
    Permissions {
        /// The file whose mode could not be set.
        path: PathBuf,
        /// The filesystem error.
        source: io::Error,
    },
}

/// The step of a backup write that failed (pre-migration or daily).
#[derive(Debug)]
pub enum BackupError {
    /// The backup directory does not exist or is not a directory. The
    /// data-directory layout — including `db-backups/` — is owned by the
    /// app layer (spec §5.5, DECISIONS.md D-027), so this crate reports
    /// the missing directory instead of creating it.
    Directory,
    /// The `VACUUM INTO` statement failed (for example: no space left,
    /// unwritable directory).
    Sqlite(rusqlite::Error),
    /// A filesystem operation around the copy failed (removing a stale
    /// temporary file, linking the finished copy into place).
    Io(io::Error),
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupError::Directory => {
                write!(
                    f,
                    "the backup directory does not exist or is not a directory"
                )
            }
            BackupError::Sqlite(err) => write!(f, "VACUUM INTO failed: {err}"),
            BackupError::Io(err) => write!(f, "filesystem operation failed: {err}"),
        }
    }
}

impl std::error::Error for BackupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BackupError::Directory => None,
            BackupError::Sqlite(err) => Some(err),
            BackupError::Io(err) => Some(err),
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Migration { source, backup } => {
                // Wording true for every producer of this variant: a
                // failed step contributes nothing (its transaction
                // rolled back), and no committed step is lost — the
                // database keeps the state of its last committed
                // migration, whatever version that is.
                write!(
                    f,
                    "database migration failed: {source}; the database \
                     keeps the state of its last committed migration"
                )?;
                match backup {
                    Some(path) => {
                        write!(
                            f,
                            "; the pre-migration copy of that state is at {}",
                            path.display()
                        )
                    }
                    None => Ok(()),
                }
            }
            StorageError::Sql(err) => write!(f, "database error: {err}"),
            StorageError::Open { path, source } => {
                write!(
                    f,
                    "failed to open or configure database at {}: {source}",
                    path.display()
                )
            }
            StorageError::JournalMode { path, mode } => write!(
                f,
                "could not enable write-ahead logging for {}: SQLite reported journal mode {mode:?}",
                path.display()
            ),
            StorageError::Backup { path, source } => {
                write!(
                    f,
                    "failed to write the database backup at {}: {source}",
                    path.display()
                )
            }
            StorageError::Settings { key, source } => {
                write!(f, "invalid setting {key}: {source}")
            }
            StorageError::ActionLog { source } => {
                write!(f, "invalid action-log entry: {source}")
            }
            StorageError::Permissions { path, source } => {
                write!(
                    f,
                    "failed to set owner-only permissions (0600) on {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Migration { source, .. } => Some(source),
            StorageError::Sql(err) => Some(err),
            StorageError::Open { source, .. } => Some(source),
            StorageError::JournalMode { .. } => None,
            StorageError::Backup { source, .. } => Some(source),
            StorageError::Settings { source, .. } => Some(source),
            StorageError::ActionLog { source } => Some(source),
            StorageError::Permissions { source, .. } => Some(source),
        }
    }
}

impl From<rusqlite_migration::Error> for StorageError {
    fn from(err: rusqlite_migration::Error) -> Self {
        // This conversion serves the version reads (`current_version`,
        // `pending_migrations`) and the too-new rejection: no migration
        // step ran, so there is no copy to point at. The stepping loop
        // attaches the copy itself when a step actually fails.
        StorageError::Migration {
            source: err,
            backup: None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        StorageError::Sql(err)
    }
}

/// Wraps a rusqlite error from the [`open_db`] path with the file path.
fn open_failed(path: &Path, source: rusqlite::Error) -> StorageError {
    StorageError::Open {
        path: path.to_owned(),
        source,
    }
}

/// Sets owner-only permissions (`0600`) on `path` — SEC-007's Unix
/// requirement for the database file and every backup copy of it (the
/// copies hold the same data, so they share the file's sensitivity).
/// The mode is a plain overwrite, not masked by the process umask, and
/// is re-applied on every call: a file that drifted — restored from a
/// copy, moved in with wider bits — converges back the next time its
/// owner opens or rewrites it. On non-Unix platforms (Windows) the
/// rights are inherited from the user profile (SEC-007) and this is
/// deliberately a no-op.
///
/// Returns the raw [`io::Error`] so each call site maps it into its own
/// error shape: [`StorageError::Permissions`] on the open path,
/// [`BackupError::Io`] around a staged copy.
#[cfg(unix)]
fn set_owner_only_mode(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    const OWNER_ONLY_FILE: u32 = 0o600;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_ONLY_FILE))
}

/// The non-Unix half of [`set_owner_only_mode`]: SEC-007 inherits the
/// user profile's rights on Windows, so nothing is chmod'ed.
#[cfg(not(unix))]
fn set_owner_only_mode(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Whether `path` addresses a database SQLite keeps in a real file —
/// the SEC-007 mode step applies to those only. The two non-file
/// spellings of [`open_db`]'s path: `":memory:"` (an in-memory
/// database, the WAL-verification guard's own test fixture) and an
/// empty path (SQLite's private on-disk temporary, deleted on close).
fn is_file_backed(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path != Path::new(":memory:")
}

/// Best-effort [`set_owner_only_mode`] on one of the database file's
/// sidecars (`suffix` is `"-wal"` or `"-shm"`), repairing a sidecar
/// that pre-dates this open and was therefore created without the
/// creation-time mode copy (a crash leftover, a restored data
/// directory). Errors — including the sidecar not existing — are
/// deliberately ignored: this is extra coverage on top of the main
/// file's mode, not a load-bearing guarantee (see [`open_db`]). On
/// non-Unix platforms (Windows) the profile's rights stand and this is
/// a no-op.
#[cfg(unix)]
fn repair_sidecar_mode(db_path: &Path, suffix: &str) {
    let mut name = db_path.as_os_str().to_os_string();
    name.push(suffix);
    let _ = set_owner_only_mode(Path::new(&name));
}

/// The non-Unix half of [`repair_sidecar_mode`] (see there).
#[cfg(not(unix))]
fn repair_sidecar_mode(_db_path: &Path, _suffix: &str) {}

/// Opens the SQLite database file at `path` with the required pragma set.
///
/// The file is created when it does not exist yet, but parent directories
/// are never created: the app layer owns the data-directory layout (see
/// DECISIONS.md D-027) and must create `db/` before calling this function.
/// A missing parent surfaces as [`StorageError::Open`] naming the path.
///
/// On Unix the file is set to owner-only permissions (`0600`, spec §13
/// SEC-007) immediately after the open — before the pragma chain below —
/// for two reasons: the WAL/-shm siblings SQLite creates on the
/// `journal_mode=WAL` switch copy the main file's mode, so they are
/// covered by construction, and a re-open repairs drift (a file restored
/// or copied with wider bits converges back, D-030's recorded gap). A
/// file whose mode cannot be set fails the open as
/// [`StorageError::Permissions`]: continuing would leave the data
/// readable by other local users. Siblings that *pre-date* the open are
/// not created here and see no mode copy; a best-effort pass at the end
/// of the pragma chain re-tightens them when they exist. In-memory
/// databases (`:memory:`) and SQLite's private temporaries (an empty
/// path) have no file and skip the step. On Windows the profile's
/// inherited rights stand (SEC-007).
///
/// The spec DB-001 pragma set is applied on every call, in this order:
///
/// 1. `busy_timeout=5000` — so every later step waits on a lock instead of
///    failing immediately; the timeout is set through
///    [`Connection::busy_timeout`], which programs the same SQLite
///    busy handler as the pragma does;
/// 2. `journal_mode=WAL` — verified against the mode SQLite reports, so a
///    file that cannot use WAL fails the open instead of silently running
///    without the NFR-007 crash-safety guarantees;
/// 3. `synchronous=FULL` — required for crash safety in WAL mode;
/// 4. `foreign_keys=ON` — the SQLite default depends on build flags, so it
///    is set explicitly on every connection.
///
/// Schema migrations are deliberately separate: call [`run_migrations`] on
/// the returned connection (spec DB-004).
///
/// # Errors
///
/// Returns [`StorageError::Open`] when the file cannot be opened or a
/// pragma fails, [`StorageError::Permissions`] when its mode cannot be
/// set to owner-only (SEC-007, Unix), and [`StorageError::JournalMode`]
/// when SQLite reports a journal mode other than `wal` after the
/// update attempt.
pub fn open_db(path: &Path) -> Result<Connection, StorageError> {
    let conn = Connection::open(path).map_err(|source| open_failed(path, source))?;

    // SEC-007: owner-only database file, set at the open and before the
    // pragma chain — the `journal_mode=WAL` switch below makes SQLite
    // create the `-wal`/`-shm` siblings, and they copy this file's
    // mode, so ordering here covers them too (see the method docs).
    // In-memory databases (`:memory:`) and SQLite's private temporary
    // databases (an empty path) have no file to protect, so the mode
    // step — and its failure — does not apply to them.
    if is_file_backed(path) {
        set_owner_only_mode(path).map_err(|source| StorageError::Permissions {
            path: path.to_owned(),
            source,
        })?;
    }

    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|source| open_failed(path, source))?;

    let mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|source| open_failed(path, source))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::JournalMode {
            path: path.to_owned(),
            mode,
        });
    }

    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(|source| open_failed(path, source))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|source| open_failed(path, source))?;

    // SEC-007, drift repair for sidecars that pre-date this open:
    // SQLite copies the main file's mode only when it *creates* the
    // `-wal`/`-shm` files, so a leftover from a crash or a restored
    // data directory is reused at whatever mode it carries — and a
    // `-wal` holds committed database frames. Best-effort on purpose:
    // the steady state is already guaranteed by the creation-time copy
    // above and the main file's mode, and a repair that cannot happen
    // (an exotic filesystem, the sidecar gone again) must not fail the
    // open over coverage the primary path already provides.
    repair_sidecar_mode(path, "-wal");
    repair_sidecar_mode(path, "-shm");

    Ok(conn)
}

/// Migrates `conn` to the latest schema version (spec DB-004).
///
/// On a fresh database this applies every migration in order; on an
/// existing database it only applies the pending ones. Each migration is
/// committed in its own transaction: the steps are applied one at a time
/// through `Migrations::to_version`, because `Migrations::to_latest`
/// would batch all pending migrations into a single transaction
/// (rusqlite_migration 2.7, `goto_up`). A migration that fails is rolled
/// back and the database stays at the last committed step; the returned
/// [`StorageError::Migration`] carries the pre-migration copy of that
/// state, when the run wrote one (spec UPD-004).
///
/// # Pre-migration backup
///
/// Before every migration applied to an existing database (`PRAGMA
/// user_version > 0` when the run starts), a copy of the current state is
/// written into `backup_dir` with `VACUUM INTO` (spec DB-004, UPD-004):
///
/// * the file is named `pre-migration-v<N>-<UTC>.sqlite3`, where `N` is
///   the schema version the database is at — the copy preserves exactly
///   that state — and `<UTC>` is a filesystem-safe UTC timestamp
///   (`YYYYMMDDTHHMMSSSSSZ`, millisecond resolution); if that name is
///   already taken (two copies within one millisecond, a clock step
///   back), `-1`, `-2`, … is inserted before the extension;
/// * a run that starts from a fresh database (`user_version = 0`, nothing
///   applied yet) writes no copies at all — there is no user state to
///   lose — including when it applies several migrations in one run;
/// * the copy is written to a `.tmp` sibling and linked into place with
///   a hard link (which fails instead of replacing an existing file), so
///   a crash can never leave a half-written file under a final name and
///   an existing copy is never overwritten; leftover `.tmp` files of
///   crashed runs are swept at the start of the next upgrade run.
///
/// The backup directory is a parameter, not a global: this crate does not
/// know the data directory. The app layer creates `db-backups/` as part
/// of the data-directory layout (spec §5.5, DECISIONS.md D-027). The
/// directory is required — and a missing or non-directory path fails the
/// run with [`StorageError::Backup`] before any migration of that run is
/// applied — exactly when a copy must be written: a fresh bootstrap
/// writes none and does not consult the directory at all.
///
/// A database whose `user_version` is newer than this binary's migration
/// set is rejected (the app layer turns this into the "update the app"
/// message, spec DB-004/NFR-010).
///
/// The connection's pragmas are left as-is. With `open_db`'s
/// `foreign_keys=ON` this is safe for migrations that only create
/// objects, but a future rebuild-style migration (SQLite's twelve-step
/// `ALTER TABLE` procedure) cannot switch foreign keys off inside
/// `rusqlite_migration`'s per-migration transactions — that migration
/// needs its own foreign-keys policy decision before it lands (see
/// DECISIONS.md D-030).
///
/// # Returns
///
/// The copies this run wrote, in application order (ascending source
/// version) — empty for a fresh bootstrap and for a no-op run.
///
/// # Errors
///
/// Returns [`StorageError::Migration`] when the schema version cannot be
/// read, the database is too far ahead, or a migration fails — the
/// variant's `backup` names the copy of the unchanged state when the
/// failed step had one written (an existing database; spec UPD-004) and
/// is `None` otherwise; [`StorageError::Backup`] when a safety copy
/// cannot be written, which fails the run before the migration of that
/// step is applied, so nothing changed.
pub fn run_migrations(
    conn: &mut Connection,
    backup_dir: &Path,
) -> Result<Vec<PathBuf>, StorageError> {
    migrate(conn, &migrations(), backup_dir)
}

/// Applies the pending migrations of `set` one step at a time, writing a
/// pre-migration backup copy for every step of an upgrade run.
///
/// Split out from [`run_migrations`] so the backup and per-step semantics
/// can be tested against synthetic multi-step migration sets — the
/// embedded set defines a single migration today, so no upgrade path
/// exists through the public API yet.
///
/// Migration runs are expected to be serialized by the caller (the app
/// opens one connection behind a mutex, spec SYS-002); the backup naming
/// relies on that single writer.
///
/// Behavior delta from the replaced `to_latest` call, recorded for
/// honesty: an empty migration set is a successful no-op here, where
/// `to_latest` errored with `NoMigrationsDefined`. The embedded set is
/// compile-time non-empty, so the difference is unobservable through
/// [`run_migrations`].
fn migrate(
    conn: &mut Connection,
    set: &Migrations<'_>,
    backup_dir: &Path,
) -> Result<Vec<PathBuf>, StorageError> {
    let start = usize::from(set.current_version(conn)?);
    let pending = set.pending_migrations(conn)?;
    if pending < 0 {
        // Preserve `Migrations::to_latest`'s verdict on a database newer
        // than the set instead of silently declaring it up to date.
        return Err(StorageError::Migration {
            source: rusqlite_migration::Error::MigrationDefinition(
                rusqlite_migration::MigrationDefinitionError::DatabaseTooFarAhead,
            ),
            // No migration step ran, so no copy was written.
            backup: None,
        });
    }
    let latest = start + pending as usize;

    let mut backups = Vec::new();
    let mut current = start;
    if start > 0 && latest > start {
        // The directory is required as soon as an upgrade run starts —
        // validated before the debris sweep so a broken layout is
        // reported as `BackupError::Directory`, not as a read failure.
        if !backup_dir.is_dir() {
            return Err(StorageError::Backup {
                path: backup_dir.to_owned(),
                source: BackupError::Directory,
            });
        }
        // Debris of crashed earlier runs (see `sweep_stale_temporaries`);
        // safe under the serialized-runs assumption.
        sweep_stale_temporaries(backup_dir, |name| {
            name.starts_with("pre-migration-v") && name.ends_with(".sqlite3.tmp")
        })?;
    }
    for target in (start + 1)..=latest {
        // A database that already carries a schema version is backed up
        // before every step; a run that starts from version 0 creates
        // the schema and has nothing to lose.
        if start > 0 {
            let stamp = utc_stamp(Timestamp::now());
            backups.push(vacuum_into_backup(conn, backup_dir, current, &stamp)?);
        }
        if let Err(source) = set.to_version(conn, target) {
            // The failed step rolled back alone (its own transaction,
            // DB-004): the database keeps the state of its last
            // committed step, which is exactly what the copy written
            // just before this step preserves — `backups.last()`, the
            // pointer spec UPD-004 has the app layer show the user.
            // A run that started from version 0 wrote no copies.
            return Err(StorageError::Migration {
                source,
                backup: backups.last().cloned(),
            });
        }
        current = target;
    }
    Ok(backups)
}

/// Formats `ts` as the filesystem-safe UTC stamp of a backup file name:
/// `YYYYMMDDTHHMMSSSSSZ` — compact, millisecond resolution, no `:` (which
/// Windows forbids in file names) and no other punctuation to escape.
fn utc_stamp(ts: Timestamp) -> String {
    ts.strftime("%Y%m%dT%H%M%S%3fZ").to_string()
}

/// The backup file name for a migration away from `source_version`.
///
/// `attempt` is 0 for the plain timestamped name and counts up when that
/// name is already taken. Note for future listers: a `-k` suffixed name
/// sorts *before* its unsuffixed sibling (`-` precedes `.` in ASCII), so
/// a name-ordered listing does not put the later-written copy last when a
/// same-millisecond collision occurred.
fn backup_file_name(source_version: usize, stamp: &str, attempt: u32) -> String {
    match attempt {
        0 => format!("pre-migration-v{source_version}-{stamp}.sqlite3"),
        n => format!("pre-migration-v{source_version}-{stamp}-{n}.sqlite3"),
    }
}

/// The maximum number of name attempts for one backup copy (the plain
/// timestamped name plus `-1`…`-<CAP>` suffixed variants) before the
/// write gives up — a guard against a pathological directory.
const BACKUP_NAME_ATTEMPT_CAP: u32 = 1000;

/// Removes leftover staging files of crashed earlier backup writes —
/// the `.tmp` debris this scheme's names leave behind (`is_own_debris`
/// decides what belongs to the scheme: the pre-migration copies match
/// their prefix, the daily copies their strict name pattern).
///
/// A crash between `VACUUM INTO` and the final link leaves a
/// `<name>.sqlite3.tmp` file; the next run of the same kind lands
/// on a fresh name (or finds its copy already present), so without
/// this sweep the debris would accumulate. Foreign `.tmp` files —
/// anything the predicate does not claim — are deliberately untouched.
/// Runs are serialized (see [`migrate`] and [`daily_backup`]), so a
/// `.tmp` matching the pattern can only be stale. Individual removal
/// failures are ignored: a held file must not block the run, and the
/// write path never reuses its exact name.
fn sweep_stale_temporaries(
    backup_dir: &Path,
    is_own_debris: impl Fn(&str) -> bool,
) -> Result<(), StorageError> {
    for entry in std::fs::read_dir(backup_dir).map_err(|err| StorageError::Backup {
        path: backup_dir.to_owned(),
        source: BackupError::Io(err),
    })? {
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_own_debris(name) {
            let _ = retry_transient_hold(|| std::fs::remove_file(entry.path()));
        }
    }
    Ok(())
}

/// Runs `op`, retrying the transient Windows holds spec §9.5 prescribes
/// for file writes (an antivirus scanner briefly holding a freshly
/// created file): a sharing violation or access denied is retried up to
/// three more times after 100 ms, 300 ms and 900 ms pauses, then the
/// last error is returned. Shared with `backup_retention`, whose copy
/// removals ride the same §9.5 backoff.
pub(crate) fn retry_transient_hold<T>(
    mut op: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    const PAUSES_MS: [u64; 3] = [100, 300, 900];
    let mut attempt = 0;
    loop {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) if is_transient_hold(&err) && attempt < PAUSES_MS.len() => {
                std::thread::sleep(Duration::from_millis(PAUSES_MS[attempt]));
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Whether `err` is a transient hold on Windows: a sharing violation
/// (os error 32) or access denied (os error 5).
fn is_transient_hold(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(32) | Some(5))
}

/// Writes a `VACUUM INTO` copy of the database into `backup_dir`.
///
/// The copy is staged under a `.tmp` sibling of the final name and linked
/// into place with [`std::fs::hard_link`] (see [`stage_and_link_copy`]):
/// unlike a rename, which silently replaces an existing destination on
/// every supported platform, a hard link fails if the final name exists
/// — so an existing copy is never overwritten even in the window between
/// the name search and the link. A crash mid-copy leaves the `.tmp`
/// file, which the next run removes (see [`sweep_stale_temporaries`]);
/// the final name only ever appears complete.
fn vacuum_into_backup(
    conn: &Connection,
    backup_dir: &Path,
    source_version: usize,
    stamp: &str,
) -> Result<PathBuf, StorageError> {
    if !backup_dir.is_dir() {
        return Err(StorageError::Backup {
            path: backup_dir.to_owned(),
            source: BackupError::Directory,
        });
    }

    let mut attempt = 0u32;
    loop {
        let final_path = backup_dir.join(backup_file_name(source_version, stamp, attempt));
        if final_path.exists() {
            attempt += 1;
            if attempt > BACKUP_NAME_ATTEMPT_CAP {
                return Err(StorageError::Backup {
                    path: final_path,
                    source: BackupError::Io(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "too many backup files share this timestamp",
                    )),
                });
            }
            continue;
        }
        match stage_and_link_copy(conn, &final_path)? {
            CopyPublication::Linked => return Ok(final_path),
            // A file appeared under the name in the window between the
            // search and the link: take the next name and copy again.
            CopyPublication::NameTaken => attempt += 1,
        }
    }
}

/// What [`stage_and_link_copy`] found at the final name: the staged
/// copy was linked into place, or the name was already occupied (the
/// link refuses to replace an existing file) — each caller decides
/// what an occupied name means for its scheme (the pre-migration
/// copies take the next name; the daily copies take it as their
/// same-day answer).
enum CopyPublication {
    /// The copy is linked under the final name, complete.
    Linked,
    /// The final name existed at the link — nothing was overwritten.
    NameTaken,
}

/// Stages one `VACUUM INTO` copy of `conn`'s database at
/// `<final_path>.tmp` and links it into place — the publication
/// mechanics both backup schemes share (the pre-migration copies,
/// DB-004, and the daily copies, ERR-001; DECISIONS.md D-038).
///
/// The staging and link discipline: the copy is written to the `.tmp`
/// sibling (a leftover `.tmp` at that exact name can only be stale
/// debris of a crashed earlier run — the callers serialize their runs
/// — and is removed first, `VACUUM INTO` refusing an existing file);
/// the finished temporary is set to owner-only permissions (`0600`,
/// SEC-007, Unix) *before* the link, so the published copy — and even
/// a crash-left `.tmp` — never carries wider bits; the link under the
/// final name then fails rather than replaces an occupied destination,
/// so the final name only ever appears complete and an existing copy
/// is never overwritten. Filesystem mutations retry the spec §9.5
/// backoff (see [`retry_transient_hold`]); once the link is in place
/// the temporary's removal is best-effort — the copy is complete, a
/// file some scanner keeps holding must not fail it, and the sweep
/// collects the leftover.
fn stage_and_link_copy(
    conn: &Connection,
    final_path: &Path,
) -> Result<CopyPublication, StorageError> {
    let mut tmp_name = final_path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = PathBuf::from(tmp_name);

    if let Err(err) = retry_transient_hold(|| std::fs::remove_file(&tmp_path))
        && err.kind() != io::ErrorKind::NotFound
    {
        return Err(StorageError::Backup {
            path: final_path.to_owned(),
            source: BackupError::Io(err),
        });
    }

    let tmp_str = tmp_path.to_str().ok_or_else(|| StorageError::Backup {
        path: final_path.to_owned(),
        source: BackupError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "the backup path is not valid UTF-8",
        )),
    })?;
    if let Err(err) = conn.execute("VACUUM INTO ?1", [tmp_str]) {
        // Best-effort cleanup; a surviving `.tmp` stays recognizable.
        let _ = std::fs::remove_file(&tmp_path);
        return Err(StorageError::Backup {
            path: final_path.to_owned(),
            source: BackupError::Sqlite(err),
        });
    }
    // SEC-007: the copy holds the database's data, so it is owner-only
    // (`0600`) from the moment it exists — set on the staged temporary
    // *before* the link publishes it (the link shares the inode, so the
    // published copy carries this mode), so even a crash-left `.tmp`
    // never exposes wider bits. Failure is a backup-write failure, not
    // an open failure, hence `BackupError::Io` rather than
    // `StorageError::Permissions`; the staged temporary is removed
    // best-effort like every other failure path of this function, so
    // a chmod-rejecting filesystem does not leave a umask-mode copy of
    // the database behind.
    set_owner_only_mode(&tmp_path).map_err(|source| {
        let _ = std::fs::remove_file(&tmp_path);
        StorageError::Backup {
            path: final_path.to_owned(),
            source: BackupError::Io(source),
        }
    })?;
    match retry_transient_hold(|| std::fs::hard_link(&tmp_path, final_path)) {
        Ok(()) => {}
        // A file appeared under the name in the window between the
        // caller's check and the link. This branch is defensive — under
        // the serialized-runs assumption nothing else creates files
        // here — and is deliberately not covered by a test.
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&tmp_path);
            return Ok(CopyPublication::NameTaken);
        }
        Err(err) => {
            // Best-effort cleanup of the staged temporary; a survivor
            // stays recognizable (the sweep removes it).
            let _ = std::fs::remove_file(&tmp_path);
            return Err(StorageError::Backup {
                path: final_path.to_owned(),
                source: BackupError::Io(err),
            });
        }
    }
    // The final name is complete now; the temporary link is pure
    // debris, so its removal is best-effort: a file a scanner keeps
    // holding must not fail an otherwise finished copy (the sweep
    // removes the leftover on a later run).
    let _ = retry_transient_hold(|| std::fs::remove_file(&tmp_path));
    Ok(CopyPublication::Linked)
}

// ---- Daily startup copies (spec §10 ERR-001) --------------------------------
//
// One `VACUUM INTO` copy per UTC day, `daily-YYYYMMDD.sqlite3`, at most
// 7 kept — the redundancy half of the backup directory, next to the
// migration-safety copies above. The two schemes share the publication
// mechanics (`stage_and_link_copy`) but never each other's names: the
// retention below counts and deletes daily names only.

/// How many daily copies are kept (spec §10 ERR-001: "keeps the last
/// 7"). The cap applies to the `daily-*.sqlite3` names only — the
/// pre-migration copies are outside it and accumulate uncapped (their
/// retention is a separate concern, D-038).
const DAILY_BACKUPS_KEPT: usize = 7;

/// The daily copy name for the UTC day of `now`:
/// `daily-YYYYMMDD.sqlite3` (spec §10 ERR-001). The date is the same
/// filesystem-safe shape the pre-migration stamps use for their date
/// part — 8 ASCII digits, no separators (nothing to escape on any
/// supported platform, D-038) — and its fixed width is what makes a
/// name-ordered listing of daily copies chronological.
fn daily_file_name(now: Timestamp) -> String {
    format!("daily-{}.sqlite3", now.strftime("%Y%m%d"))
}

/// Whether `name` is a daily copy of this scheme: exactly
/// `daily-<8 ASCII digits>.sqlite3`. Everything else is not this
/// scheme's to count or delete — other prefixes (`pre-migration-v…`),
/// suffixed variants of this prefix (`daily-20261118-1.sqlite3`), and
/// dates of any other width (`daily-2026111.sqlite3`,
/// `daily-notes.sqlite3`).
fn is_daily_copy_name(name: &str) -> bool {
    name.strip_prefix("daily-")
        .and_then(|rest| rest.strip_suffix(".sqlite3"))
        .is_some_and(|date| date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The outcome of a [`daily_backup`] run.
#[derive(Debug, PartialEq, Eq)]
pub enum DailyBackupOutcome {
    /// The day's copy was written and linked into place; the path.
    Written(PathBuf),
    /// A copy for the day already existed, so nothing was written —
    /// ERR-001's "if there is none for today yet". Decided by the file
    /// name alone: the existing copy is never opened, read or
    /// validated, and never replaced; the path.
    AlreadyPresent(PathBuf),
}

/// Writes the daily startup copy of the database (spec §10 ERR-001):
/// `db-backups/daily-YYYYMMDD.sqlite3`, at most one per UTC day, the
/// newest [`DAILY_BACKUPS_KEPT`] kept.
///
/// `now` is the injection seam for "today" (system time is not
/// mockable at the call site): only the UTC date of `now` is used, so
/// every instant of one UTC day maps to exactly one file name, and the
/// day boundary is the UTC midnight the stamp encodes.
///
/// Semantics:
///
/// * **Idempotency** — a run whose day already has a copy changes
///   nothing (not even pruning): a restart on the same day must not
///   create a second copy. The check is the file name; the copy is
///   never opened.
/// * **Publication** — the copy is staged and linked by the shared
///   D-038 mechanics ([`stage_and_link_copy`]): `.tmp` staging,
///   `hard_link` publication that refuses to replace an existing file,
///   §9.5 retries, and a sweep of this scheme's stale `.tmp` debris at
///   the start of the run (the sweep is the one thing a no-op run may
///   still remove — crashed-run debris, never a copy).
/// * **Retention** — after a copy is *written*, daily copies beyond
///   the newest [`DAILY_BACKUPS_KEPT`] are removed, oldest first, by
///   name order (fixed-width dates: lexicographic = chronological) —
///   **but never the copy this run just wrote**: a clock-set-back
///   directory may hold seven names newer than today, and deleting by
///   age alone would delete today's fresh copy, leave the day without
///   one and repeat a full `VACUUM INTO` on every restart. A run that
///   found its copy present does not prune at all: it wrote nothing,
///   so the cap from the previous write still holds; pathological
///   states (clock steps, files restored by hand) self-heal on the
///   next write. Pruning is best-effort throughout — the retention
///   runs *after* the copy is complete, so a listing that cannot be
///   read or a file that stays held (after its §9.5 retries) must not
///   turn a successful copy into a reported failure; the next write
///   prunes again. Pre-migration names are never counted or deleted.
///
/// The backup directory is a parameter and is required to exist, like
/// for the pre-migration copies: the app layer's layout creates
/// `db-backups/` (spec §5.5, D-027) and this crate creates nothing.
///
/// # Errors
///
/// Returns [`StorageError::Backup`] when the directory is missing or
/// not a directory, or the copy cannot be written. Retention failures
/// are *not* errors (see above). ERR-001 frames the daily copy as
/// extra reliability, not a precondition — the caller logs the
/// failure and continues (unlike a failed *pre-migration* copy, which
/// aborts the run, D-038); that degradation policy is the caller's to
/// apply.
pub fn daily_backup(
    conn: &Connection,
    backup_dir: &Path,
    now: Timestamp,
) -> Result<DailyBackupOutcome, StorageError> {
    if !backup_dir.is_dir() {
        return Err(StorageError::Backup {
            path: backup_dir.to_owned(),
            source: BackupError::Directory,
        });
    }
    sweep_stale_temporaries(backup_dir, is_daily_debris)?;
    let name = daily_file_name(now);
    let final_path = backup_dir.join(&name);
    if final_path.exists() {
        return Ok(DailyBackupOutcome::AlreadyPresent(final_path));
    }
    match stage_and_link_copy(conn, &final_path)? {
        CopyPublication::Linked => {
            // Retention runs on the write path only, and never deletes
            // the copy this run just wrote (see `prune_daily_backups`)
            // — a clock-set-back directory must not cost the day its
            // copy and thrash a full `VACUUM INTO` into every restart.
            prune_daily_backups(backup_dir, &name);
            Ok(DailyBackupOutcome::Written(final_path))
        }
        // The name was taken between the existence check and the link
        // (nothing else does, under the serialized-runs assumption):
        // the day already has its copy, and a run that wrote nothing
        // deletes nothing.
        CopyPublication::NameTaken => Ok(DailyBackupOutcome::AlreadyPresent(final_path)),
    }
}

/// Whether `name` is stale `.tmp` debris of a crashed daily write: the
/// `.tmp`-stripped name is a full daily copy name of this scheme
/// (`daily-<8 digits>.sqlite3`). Stricter than a prefix match on
/// purpose: `daily-notes.sqlite3.tmp` is not this scheme's file and
/// stays untouched, exactly like the retention's name policy.
fn is_daily_debris(name: &str) -> bool {
    name.strip_suffix(".tmp").is_some_and(is_daily_copy_name)
}

/// Removes the oldest daily copies beyond [`DAILY_BACKUPS_KEPT`],
/// never `keep` — the name this run just wrote (a clock-set-back
/// directory may hold seven names newer than today; deleting by age
/// alone would delete today's fresh copy, leave the day without one
/// and repeat a full `VACUUM INTO` on every restart).
///
/// Best-effort throughout, on purpose: this runs *after* the copy is
/// linked and complete, so a failure here must not turn a successful
/// copy into a reported one — the listing that cannot be read and the
/// file that stays held (after its §9.5 retries) are both left for
/// the next write, which prunes again.
fn prune_daily_backups(backup_dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(backup_dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str().map(str::to_owned))
        .filter(|name| is_daily_copy_name(name))
        .collect();
    names.sort_unstable();
    let excess = names.len().saturating_sub(DAILY_BACKUPS_KEPT);
    let mut removed = 0;
    for name in names {
        if removed >= excess {
            break;
        }
        if name == keep {
            continue;
        }
        let _ = retry_transient_hold(|| std::fs::remove_file(backup_dir.join(&name)));
        removed += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Creates a unique temporary directory for one test.
    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be past the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "eunomia-store-backup-{label}-{}-{nanos}",
            std::process::id(),
        ));
        fs::create_dir_all(&dir).expect("temp directory should be creatable");
        dir
    }

    /// Builds a synthetic migration set (real SQL, applied to real
    /// databases) — the embedded set defines a single migration, so the
    /// upgrade paths these tests need cannot go through it.
    fn set_of(sqls: &[&'static str]) -> Migrations<'static> {
        Migrations::new(sqls.iter().map(|sql| M::up(sql)).collect())
    }

    /// Asserts the `YYYYMMDDTHHMMSSSSSZ` shape of a backup stamp
    /// (19 characters: 8 of date, `T`, 6 of time, 3 of milliseconds, `Z`).
    fn assert_stamp_shape(stamp: &str) {
        let bytes = stamp.as_bytes();
        assert_eq!(bytes.len(), 19, "stamp must be 19 characters: {stamp}");
        assert_eq!(bytes[8], b'T', "the date/time separator of {stamp}");
        assert_eq!(bytes[18], b'Z', "the UTC marker of {stamp}");
        for (index, byte) in bytes.iter().enumerate() {
            if matches!(index, 8 | 18) {
                continue;
            }
            assert!(
                byte.is_ascii_digit(),
                "byte {index} of {stamp} must be a digit"
            );
        }
    }

    /// Reads `PRAGMA user_version` from a connection.
    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version should be readable")
    }

    /// The backup-directory argument for in-memory databases, which are
    /// always fresh or up to date and therefore never write a copy: the
    /// path is never touched (D-038). It deliberately does not exist, so
    /// a future rule change fails these tests loudly instead of writing
    /// into the working tree.
    const UNUSED_BACKUP_DIR: &str = "unused-backup-dir";

    #[test]
    fn run_migrations_creates_the_settings_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        let backups = run_migrations(&mut conn, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        assert!(
            backups.is_empty(),
            "a fresh database must not produce backup copies"
        );

        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('ui.language', '\"en\"', '2026-11-18T10:00:00.000Z')",
            [],
        )
        .unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ui.language'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "\"en\"");
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        let backups = run_migrations(&mut conn, Path::new(UNUSED_BACKUP_DIR))
            .expect("re-running migrations on a migrated database must be a no-op");
        assert!(backups.is_empty(), "a no-op run must not produce copies");
    }

    #[test]
    fn run_migrations_rejects_a_database_newer_than_the_set() {
        // A database whose user_version exceeds the embedded set must be
        // rejected (spec DB-004), not silently declared up to date.
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        conn.pragma_update(None, "user_version", 2)
            .expect("user_version should be settable");

        let err = run_migrations(&mut conn, Path::new(UNUSED_BACKUP_DIR))
            .expect_err("a too-new database must be rejected");
        assert!(
            matches!(err, StorageError::Migration { backup: None, .. }),
            "expected StorageError::Migration without a copy (no step ran), got: {err:?}"
        );
        assert_eq!(user_version(&conn), 2, "the version must be untouched");
    }

    #[test]
    fn latest_schema_version_matches_a_fully_migrated_database() {
        // The public version number must equal the `user_version` a
        // successful run leaves behind — the fact the app layer's
        // too-new gate compares against (spec DB-004).
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        assert_eq!(
            user_version(&conn) as usize,
            latest_schema_version(),
            "latest_schema_version must be the version of a fully migrated database"
        );
    }

    #[test]
    fn utc_stamp_is_compact_and_filesystem_safe() {
        // Every stamp must be exactly `YYYYMMDDTHHMMSSSSSZ` — no `:` (a
        // Windows file name cannot contain one), no dot, no offset.
        for _ in 0..100 {
            assert_stamp_shape(&utc_stamp(Timestamp::now()));
        }
    }

    #[test]
    fn backup_file_name_spellings() {
        assert_eq!(
            backup_file_name(1, "20261118T101500123Z", 0),
            "pre-migration-v1-20261118T101500123Z.sqlite3"
        );
        assert_eq!(
            backup_file_name(3, "20261118T101500123Z", 1),
            "pre-migration-v3-20261118T101500123Z-1.sqlite3"
        );
        assert_eq!(
            backup_file_name(3, "20261118T101500123Z", 12),
            "pre-migration-v3-20261118T101500123Z-12.sqlite3"
        );
    }

    #[test]
    fn upgrading_an_existing_database_writes_a_valid_pre_migration_copy() {
        let dir = temp_dir("upgrade");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        // A version-1 database with data, produced by the first step alone.
        let set_v1 = set_of(&["CREATE TABLE marker (value TEXT NOT NULL)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            let backups = migrate(&mut conn, &set_v1, &backup_dir).unwrap();
            assert!(backups.is_empty(), "the bootstrap run must not copy");
            conn.execute("INSERT INTO marker (value) VALUES ('before-migration')", [])
                .unwrap();
        }

        // The same database, one version later — opened through `open_db`,
        // the production configuration (WAL + synchronous=FULL + busy
        // timeout, DB-001/D-030): the copy must be valid from exactly
        // that state, un-checkpointed WAL frames included.
        let set_v2 = set_of(&[
            "CREATE TABLE marker (value TEXT NOT NULL)",
            "ALTER TABLE marker ADD COLUMN added_by_v2 TEXT",
        ]);
        let backups = {
            let mut conn = open_db(&db_path).unwrap();
            migrate(&mut conn, &set_v2, &backup_dir).expect("the upgrade run should succeed")
        };

        assert_eq!(backups.len(), 1, "one migration step, one copy");
        let backup = &backups[0];
        assert!(backup.is_file(), "the copy should exist as a file");

        let name = backup.file_name().unwrap().to_str().unwrap();
        let prefix = "pre-migration-v1-";
        assert!(name.starts_with(prefix), "copy name: {name}");
        assert!(name.ends_with(".sqlite3"), "copy name: {name}");
        let stamp = &name[prefix.len()..name.len() - ".sqlite3".len()];
        assert_stamp_shape(stamp);

        // The copy opens as a valid database that preserves the exact
        // pre-migration state (the acceptance criterion of the task card).
        let copy = Connection::open(backup).unwrap();
        let integrity: String = copy
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert_eq!(user_version(&copy), 1, "the copy is the version-1 state");
        let value: String = copy
            .query_row("SELECT value FROM marker", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "before-migration");

        // The upgraded original is at version 2 with the v2 column.
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 2);
        conn.execute(
            "INSERT INTO marker (value, added_by_v2) VALUES ('after', 'x')",
            [],
        )
        .expect("the upgraded schema should be usable");
        drop(conn);
        // Windows: close every handle on the directory before removing it
        // (SQLite opens files without FILE_SHARE_DELETE there).
        drop(copy);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_step_of_an_upgrade_is_backed_up_at_its_source_version() {
        let dir = temp_dir("multistep");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        let set_v1 = set_of(&["CREATE TABLE marker (value TEXT NOT NULL)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, &backup_dir).unwrap();
            conn.execute("INSERT INTO marker (value) VALUES ('v1-state')", [])
                .unwrap();
        }

        let set_v3 = set_of(&[
            "CREATE TABLE marker (value TEXT NOT NULL)",
            "CREATE TABLE v2_table (id INTEGER PRIMARY KEY)",
            "CREATE TABLE v3_table (id INTEGER PRIMARY KEY)",
        ]);
        let backups = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v3, &backup_dir).unwrap()
        };

        assert_eq!(backups.len(), 2, "two pending steps, two copies");
        let first = backups[0].file_name().unwrap().to_str().unwrap();
        let second = backups[1].file_name().unwrap().to_str().unwrap();
        assert!(
            first.starts_with("pre-migration-v1-"),
            "first copy captures the version-1 state: {first}"
        );
        assert!(
            second.starts_with("pre-migration-v2-"),
            "second copy captures the version-2 state: {second}"
        );

        // Each copy is valid and holds its source version's schema.
        let copy1 = Connection::open(&backups[0]).unwrap();
        let integrity1: String = copy1
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity1, "ok");
        assert_eq!(user_version(&copy1), 1);
        let tables1: i64 = copy1
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'v2_table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables1, 0, "the v1 copy must not contain v2 objects");

        let copy2 = Connection::open(&backups[1]).unwrap();
        let integrity2: String = copy2
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity2, "ok");
        assert_eq!(user_version(&copy2), 2);
        let tables2: i64 = copy2
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('v2_table', 'v3_table')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables2, 1, "the v2 copy has v2_table but not v3_table");

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 3);
        drop(conn);
        // Windows: close every handle on the directory before removing it.
        drop(copy1);
        drop(copy2);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_fresh_bootstrap_writes_no_copies_even_over_several_steps() {
        let dir = temp_dir("fresh");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        let set_v2 = set_of(&["CREATE TABLE t1 (x TEXT)", "CREATE TABLE t2 (x TEXT)"]);
        let mut conn = Connection::open(&db_path).unwrap();
        let backups = migrate(&mut conn, &set_v2, &backup_dir).unwrap();

        assert!(
            backups.is_empty(),
            "a database created by this run has nothing to back up"
        );
        let entries: Vec<_> = fs::read_dir(&backup_dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        assert!(entries.is_empty(), "the backup directory stays empty");

        // Windows: close the handle before removing the directory.
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_step_rolls_back_only_itself() {
        let dir = temp_dir("failing-step");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        let set_v1 = set_of(&["CREATE TABLE t1 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, &backup_dir).unwrap();
        }

        // Step 2 applies; step 3 is broken — with partial work first,
        // so only its rollback explains what is absent afterwards —
        // and must roll back alone.
        let set_v3 = set_of(&[
            "CREATE TABLE t1 (x TEXT)",
            "CREATE TABLE t2 (x TEXT)",
            "CREATE TABLE t3_probe (x TEXT); THIS IS NOT SQL",
        ]);
        let err = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v3, &backup_dir).expect_err("the broken step must fail the run")
        };
        assert!(
            matches!(err, StorageError::Migration { .. }),
            "expected StorageError::Migration, got: {err:?}"
        );

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            user_version(&conn),
            2,
            "step 2 stays committed; the failed step 3 is rolled back"
        );
        let t1: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 't1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let t2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 't2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((t1, t2), (1, 1), "both committed steps' objects exist");
        let t3_probe: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 't3_probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            t3_probe, 0,
            "the failed step's partial work must be rolled back"
        );
        drop(conn);

        // The copies for the steps that ran (and the attempted one) are
        // complete files — the failure never leaves a half-written copy.
        let names: Vec<String> = fs::read_dir(&backup_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(names.len(), 2, "copies for steps 2 and 3: {names:?}");
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("pre-migration-v1-")),
            "the v1 copy exists: {names:?}"
        );
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("pre-migration-v2-")),
            "the v2 copy exists: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.ends_with(".tmp")),
            "no temporary files survive: {names:?}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    /// Reads the full schema of `conn` as comparable rows — the
    /// "schema unchanged" proof compares the complete `sqlite_master`
    /// inventory (object type, name, root page, DDL), not a proxy.
    /// `sql` is `COALESCE`d: internal auto-index rows carry NULL.
    fn schema_dump(conn: &Connection) -> Vec<(String, String, i64, String)> {
        let mut statement = conn
            .prepare(
                "SELECT type, name, rootpage, COALESCE(sql, '') \
                 FROM sqlite_master ORDER BY type, name",
            )
            .expect("sqlite_master must be readable");
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .expect("sqlite_master rows must be readable");
        rows.map(|row| row.expect("every sqlite_master row must decode"))
            .collect()
    }

    /// Reads the `marker` table's rows in rowid order — the test
    /// fixture's user data.
    fn marker_rows(conn: &Connection) -> Vec<String> {
        let mut statement = conn
            .prepare("SELECT value FROM marker ORDER BY rowid")
            .expect("the marker table must be readable");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("marker rows must be readable");
        rows.map(|row| row.expect("every marker row must decode"))
            .collect()
    }

    #[test]
    fn a_failed_migration_changes_neither_schema_nor_data_and_names_the_copy() {
        // The T-019 acceptance criterion, on the private seam (the
        // embedded set has a single working migration, so an
        // intentionally broken one exists only here): a migration that
        // fails must leave the database exactly as it was and the
        // error must carry the path to the copy of that state
        // (spec UPD-004).
        let dir = temp_dir("failed-migration");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        // A version-1 database with data, produced by the first step alone.
        let set_v1 = set_of(&["CREATE TABLE marker (value TEXT NOT NULL)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, &backup_dir).unwrap();
            conn.execute("INSERT INTO marker (value) VALUES ('user data')", [])
                .unwrap();
        }

        // The state the broken migration must preserve, read from the
        // settled file (not from the connection that wrote it).
        let (schema_before, data_before) = {
            let conn = Connection::open(&db_path).unwrap();
            (schema_dump(&conn), marker_rows(&conn))
        };

        // The one pending step is intentionally broken — and it does
        // real work before dying (a table with a row), so only the
        // rollback can explain the state afterwards: without the
        // per-step transaction the `probe` table would survive the
        // failure. The run goes through `open_db`, the production
        // WAL/synchronous=FULL configuration (same shape as the
        // upgrade acceptance test).
        let broken = set_of(&[
            "CREATE TABLE marker (value TEXT NOT NULL)",
            "CREATE TABLE probe (x TEXT); INSERT INTO probe VALUES ('partial work'); THIS IS NOT SQL",
        ]);
        let err = {
            let mut conn = open_db(&db_path).unwrap();
            migrate(&mut conn, &broken, &backup_dir)
                .expect_err("the broken migration must fail the run")
        };
        let StorageError::Migration { backup, .. } = &err else {
            panic!("expected StorageError::Migration, got: {err:?}");
        };

        // (a) Schema, data and version are unchanged. The explicit
        // `probe` check names what the schema comparison proves: the
        // failed step's partial work is gone — the rollback, not an
        // inert statement, is what preserved the state.
        {
            let conn = Connection::open(&db_path).unwrap();
            assert_eq!(
                schema_dump(&conn),
                schema_before,
                "the failed migration must not change the schema"
            );
            assert!(
                !schema_dump(&conn)
                    .iter()
                    .any(|(kind, name, _, _)| kind == "table" && name == "probe"),
                "the failed step's partial work must be rolled back"
            );
            assert_eq!(
                marker_rows(&conn),
                data_before,
                "the failed migration must not change the data"
            );
            assert_eq!(user_version(&conn), 1, "the version must be unchanged");
        }

        // (b) The error names the copy of that state (UPD-004), the
        // copy exists, is valid and preserves exactly the state.
        let backup = backup
            .as_ref()
            .expect("an existing database gets a copy before the step");
        assert!(
            backup.starts_with(&backup_dir),
            "the copy must live in the backup directory: {}",
            backup.display()
        );
        assert!(
            backup.is_file(),
            "the copy must exist: {}",
            backup.display()
        );
        let copy = Connection::open(backup).unwrap();
        let integrity: String = copy
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert_eq!(user_version(&copy), 1, "the copy is the version-1 state");
        assert_eq!(
            marker_rows(&copy),
            data_before,
            "the copy preserves the user data"
        );
        drop(copy);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_migration_error_names_the_copy_of_the_failed_steps_own_state() {
        // A three-step run whose last step breaks: steps 2 and 3 each
        // wrote their pre-step copy, and the error must point at the
        // copy of the state the *failed* step started from (version 2,
        // what the database still is after the rollback) — not the
        // run's first copy (version 1).
        let dir = temp_dir("which-copy");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        let set_v1 = set_of(&["CREATE TABLE t1 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, &backup_dir).unwrap();
        }

        // The last step is broken and does partial work first — its
        // rollback is what the assertions below stand on.
        let broken = set_of(&[
            "CREATE TABLE t1 (x TEXT)",
            "CREATE TABLE t2 (x TEXT)",
            "CREATE TABLE probe (x TEXT); THIS IS NOT SQL",
        ]);
        let err = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &broken, &backup_dir)
                .expect_err("the broken last step must fail the run")
        };
        let StorageError::Migration {
            backup: Some(backup),
            ..
        } = err
        else {
            panic!("the failing step must carry its pre-step copy: {err:?}");
        };
        let name = backup.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("pre-migration-v2-"),
            "the copy of the failed step's source state: {name}"
        );

        let copy = Connection::open(&backup).unwrap();
        assert_eq!(
            user_version(&copy),
            2,
            "the named copy holds the state the database is still at"
        );
        drop(copy);

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 2, "the database stayed at version 2");
        let probe: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(probe, 0, "the failed step's partial work is rolled back");
        drop(conn);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_first_migration_of_a_fresh_database_reports_no_copy() {
        // A fresh bootstrap holds no user state and writes no copies;
        // the failure must say so honestly (`backup: None`) instead of
        // naming a nonexistent path — the app layer's UPD-004 message
        // degrades the same way for this shape.
        let dir = temp_dir("fresh-fail");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        // The broken bootstrap step does partial work before dying;
        // nothing of it may survive.
        let broken = set_of(&["CREATE TABLE probe (x TEXT); THIS IS NOT SQL"]);
        let err = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &broken, &backup_dir)
                .expect_err("the broken bootstrap migration must fail")
        };
        match err {
            StorageError::Migration { backup, .. } => {
                assert!(
                    backup.is_none(),
                    "a fresh run has no copy to name: {backup:?}"
                );
            }
            other => panic!("expected StorageError::Migration, got: {other:?}"),
        }

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 0, "nothing was applied");
        let probe: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(probe, 0, "the failed step's partial work is rolled back");
        drop(conn);
        let entries = fs::read_dir(&backup_dir).unwrap().count();
        assert_eq!(entries, 0, "no copies and no debris: {entries} entries");

        fs::remove_dir_all(&dir).unwrap();
    }

    /// Skips the test unless the process may fail on permission bits:
    /// root (uid 0) bypasses them, so a chmod-based fixture proves
    /// nothing there (the same gate as the config suite). The temp
    /// directory was created by this process, so its owner uid is the
    /// effective uid.
    #[cfg(unix)]
    fn running_as_root_is_untestable(temp: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(temp)
            .expect("the temp directory must be statable")
            .uid()
            == 0
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_backup_directory_surfaces_as_a_backup_error() {
        // The pre-step copy failing must abort the run as
        // `StorageError::Backup` (D-038: never silently continue, and
        // no migration of that step runs) — the failure keeps its own
        // shape instead of degrading into a migration error.
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("unwritable-backups");
        if running_as_root_is_untestable(&dir) {
            fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        let set_v1 = set_of(&["CREATE TABLE t1 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        }

        // No write rights in the backup directory: the `VACUUM INTO`
        // of the pre-step copy cannot create its staging file.
        fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o555))
            .expect("removing write rights must work");
        let set_v2 = set_of(&["CREATE TABLE t1 (x TEXT)", "CREATE TABLE t2 (x TEXT)"]);
        let result = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v2, &backup_dir)
        };
        // Restore the rights even when the assertion below panics, so
        // the teardown can remove the directory.
        fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o755))
            .expect("restoring write rights must work");
        let err = result.expect_err("the unwritable backup directory must fail the run");
        assert!(
            matches!(err, StorageError::Backup { .. }),
            "a failed copy write must surface as StorageError::Backup, got: {err:?}"
        );

        // No migration was applied by the failed run.
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 1, "no migration was applied");
        drop(conn);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_taken_file_name_gets_a_numeric_suffix() {
        let dir = temp_dir("collision");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE t (x TEXT)", []).unwrap();

        // Occupy the first two names this exact stamp would produce.
        let stamp = "20261118T101500123Z";
        let occupied: Vec<PathBuf> = [
            "pre-migration-v1-20261118T101500123Z.sqlite3",
            "pre-migration-v1-20261118T101500123Z-1.sqlite3",
        ]
        .iter()
        .map(|name| {
            let path = backup_dir.join(name);
            fs::write(&path, b"occupied").unwrap();
            path
        })
        .collect();

        let backup =
            vacuum_into_backup(&conn, &backup_dir, 1, stamp).expect("the copy should be written");
        assert_eq!(
            backup,
            backup_dir.join("pre-migration-v1-20261118T101500123Z-2.sqlite3"),
            "the third name is used"
        );

        // Existing copies are never overwritten.
        for path in &occupied {
            assert_eq!(
                fs::read(path).unwrap(),
                b"occupied",
                "{} must be untouched",
                path.display()
            );
        }

        // Windows: close the handle before removing the directory.
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stale_temp_file_is_removed_before_the_copy_is_written() {
        let dir = temp_dir("stale-tmp");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE t (x TEXT)", []).unwrap();

        // The debris of a crashed earlier run: a `.tmp` under the exact
        // name this stamp produces.
        let stale_tmp = backup_dir.join("pre-migration-v1-20261118T101500123Z.sqlite3.tmp");
        fs::write(&stale_tmp, b"half-written debris").unwrap();

        let backup = vacuum_into_backup(&conn, &backup_dir, 1, "20261118T101500123Z")
            .expect("the stale temporary file must not block the copy");
        assert_eq!(
            backup,
            backup_dir.join("pre-migration-v1-20261118T101500123Z.sqlite3")
        );
        assert!(
            !stale_tmp.exists(),
            "the stale temporary file is gone after the run"
        );

        let copy = Connection::open(&backup).unwrap();
        let integrity: String = copy
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");

        // Windows: close every handle before removing the directory.
        drop(copy);
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_backup_directory_fails_before_any_migration() {
        let dir = temp_dir("missing-dir");
        let db_path = dir.join("db.sqlite3");
        // `db-backups` is deliberately not created: the app layer owns the
        // layout (spec §5.5), and a broken layout must stop the run.
        let backup_dir = dir.join("db-backups");

        let set_v1 = set_of(&["CREATE TABLE t1 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        }

        let set_v2 = set_of(&["CREATE TABLE t1 (x TEXT)", "CREATE TABLE t2 (x TEXT)"]);
        let err = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v2, &backup_dir)
                .expect_err("a missing backup directory must fail the run")
        };
        match err {
            StorageError::Backup { path, source } => {
                assert_eq!(path, backup_dir, "the error names the directory");
                assert!(
                    matches!(source, BackupError::Directory),
                    "expected BackupError::Directory, got: {source:?}"
                );
            }
            other => panic!("expected StorageError::Backup, got: {other:?}"),
        }

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 1, "no migration was applied");
        drop(conn);
        assert!(
            !backup_dir.exists(),
            "the crate must not create the directory"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_regular_file_at_the_backup_directory_path_is_rejected() {
        let dir = temp_dir("dir-is-file");
        let db_path = dir.join("db.sqlite3");
        // Not a directory: a regular file occupies the backup path. The
        // run must fail the same way as with a missing directory.
        let backup_dir = dir.join("db-backups");
        fs::write(&backup_dir, b"not a directory").unwrap();

        let set_v1 = set_of(&["CREATE TABLE t1 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        }

        let set_v2 = set_of(&["CREATE TABLE t1 (x TEXT)", "CREATE TABLE t2 (x TEXT)"]);
        let err = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v2, &backup_dir)
                .expect_err("a non-directory backup path must fail the run")
        };
        assert!(
            matches!(
                err,
                StorageError::Backup {
                    source: BackupError::Directory,
                    ..
                }
            ),
            "expected BackupError::Directory, got: {err:?}"
        );

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 1, "no migration was applied");
        drop(conn);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_retry_after_a_failed_step_writes_a_fresh_copy_and_keeps_the_old_ones() {
        let dir = temp_dir("retry");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        let set_v1 = set_of(&["CREATE TABLE t1 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, &backup_dir).unwrap();
        }

        // The broken upgrade leaves the database at version 2 with two
        // copies (see `a_failing_step_rolls_back_only_itself`).
        let broken = set_of(&[
            "CREATE TABLE t1 (x TEXT)",
            "CREATE TABLE t2 (x TEXT)",
            "THIS IS NOT SQL",
        ]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &broken, &backup_dir).unwrap_err();
        }

        // The next run (a fixed third step shipped) upgrades from
        // version 2: a fresh pre-migration-v2 copy joins the earlier
        // ones; nothing is overwritten or half-written.
        let fixed = set_of(&[
            "CREATE TABLE t1 (x TEXT)",
            "CREATE TABLE t2 (x TEXT)",
            "CREATE TABLE t3 (x TEXT)",
        ]);
        let backups = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &fixed, &backup_dir).expect("the fixed upgrade should succeed")
        };
        assert_eq!(backups.len(), 1, "one pending step, one fresh copy");
        assert!(
            backups[0]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("pre-migration-v2-"),
            "the fresh copy captures the version-2 state"
        );

        let names: Vec<String> = fs::read_dir(&backup_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names.len(),
            3,
            "two old copies plus the fresh one: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.ends_with(".tmp")),
            "no temporary files survive: {names:?}"
        );

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 3);
        drop(conn);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_upgrade_run_sweeps_stale_pre_migration_temporaries() {
        let dir = temp_dir("sweep");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        // Debris of a crashed earlier run, plus a foreign `.tmp` the
        // sweep must not touch (it is not this crate's naming pattern).
        let stale = backup_dir.join("pre-migration-v1-20000101T000000000Z.sqlite3.tmp");
        fs::write(&stale, b"half-written debris").unwrap();
        let foreign = backup_dir.join("user-notes.txt.tmp");
        fs::write(&foreign, b"user data").unwrap();

        let set_v1 = set_of(&["CREATE TABLE t1 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, &backup_dir).unwrap();
        }

        let set_v2 = set_of(&["CREATE TABLE t1 (x TEXT)", "CREATE TABLE t2 (x TEXT)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v2, &backup_dir)
                .expect("the stale temporary must not block the upgrade");
        }

        assert!(!stale.exists(), "the crate's stale temporary is swept");
        assert!(foreign.exists(), "a foreign .tmp file is untouched");

        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(user_version(&conn), 2);
        drop(conn);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn transient_hold_retries_three_times_and_then_succeeds() {
        // Three sharing violations (Windows os error 32) then success:
        // the helper must absorb all of them and return the value. The
        // pauses are the spec §9.5 backoff (100/300/900 ms).
        let mut calls = 0;
        let result = retry_transient_hold(|| {
            calls += 1;
            if calls <= 3 {
                Err(std::io::Error::from_raw_os_error(32))
            } else {
                Ok(calls)
            }
        });
        assert_eq!(result.expect("fourth attempt succeeds"), 4);
        assert_eq!(calls, 4);
    }

    #[test]
    fn transient_hold_gives_up_after_the_prescribed_retries() {
        let mut calls = 0;
        let err = retry_transient_hold(|| {
            calls += 1;
            Err::<(), _>(std::io::Error::from_raw_os_error(32))
        })
        .expect_err("permanent holds must surface");
        assert_eq!(calls, 4, "one try plus the three §9.5 retries");
        assert_eq!(err.raw_os_error(), Some(32), "the last error is returned");
    }

    #[test]
    fn non_hold_errors_are_not_retried() {
        let mut calls = 0;
        let err = retry_transient_hold(|| {
            calls += 1;
            Err::<(), _>(std::io::Error::from_raw_os_error(28)) // ENOSPC
        })
        .expect_err("non-hold errors must surface immediately");
        assert_eq!(calls, 1, "no retries for other error classes");
        assert_eq!(err.raw_os_error(), Some(28));
        assert!(!is_transient_hold(&err));
        assert!(is_transient_hold(&std::io::Error::from_raw_os_error(32)));
        assert!(is_transient_hold(&std::io::Error::from_raw_os_error(5)));
    }

    // ---- daily startup copies (spec §10 ERR-001) ------------------------------

    /// Parses an RFC 3339 instant — the date seam's inputs are fixed
    /// timestamps, never the wall clock, so the day boundaries below
    /// are deterministic.
    fn ts(text: &str) -> Timestamp {
        text.parse()
            .unwrap_or_else(|err| panic!("a fixed test timestamp must parse: {text}: {err}"))
    }

    /// A real database with one marker row, opened through `open_db`
    /// (the production WAL/`synchronous=FULL` configuration, D-030).
    /// The writer connection is returned alive and must outlive the
    /// `daily_backup` call: with the last connection to a WAL database
    /// closed, SQLite checkpoints and deletes the `-wal` file — the
    /// writer staying open is what keeps the marker row in
    /// un-checkpointed WAL frames, so the copy is proven to capture
    /// them. Drop it before the teardown (the Windows rule: no open
    /// handles under a removed directory).
    fn database_with_marker(dir: &Path) -> (PathBuf, Connection) {
        let db_path = dir.join("db.sqlite3");
        let mut conn = open_db(&db_path).unwrap();
        run_migrations(&mut conn, Path::new(UNUSED_BACKUP_DIR)).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) \
             VALUES ('daily.marker', '\"present\"', '2026-11-18T09:00:00.000Z')",
            [],
        )
        .unwrap();
        (db_path, conn)
    }

    /// The `daily-*.sqlite3` names currently in `dir`, sorted.
    fn daily_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
            .filter(|name| name.starts_with("daily-"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn daily_file_name_is_the_utc_day() {
        // Any instant of one UTC day maps to the same name; the day
        // boundary is UTC midnight.
        assert_eq!(
            daily_file_name(ts("2026-11-18T23:59:59.999Z")),
            "daily-20261118.sqlite3"
        );
        assert_eq!(
            daily_file_name(ts("2026-11-19T00:00:00.000Z")),
            "daily-20261119.sqlite3"
        );
        assert_eq!(
            daily_file_name(ts("2026-11-18T00:00:00.000Z")),
            "daily-20261118.sqlite3"
        );
        assert_eq!(
            daily_file_name(ts("2026-11-18T12:00:00.000Z")),
            "daily-20261118.sqlite3"
        );
    }

    #[test]
    fn the_daily_name_pattern_admits_only_eight_digit_dates() {
        assert!(is_daily_copy_name("daily-20261118.sqlite3"));
        for foreign in [
            "daily-20261118-1.sqlite3",
            "daily-2026111.sqlite3",
            "daily-202611181.sqlite3",
            "daily-notes.sqlite3",
            "daily-.sqlite3",
            "daily-20261118.sqlite3.tmp",
            "pre-migration-v1-20261118T101500123Z.sqlite3",
            "daily-20261118.sqlite",
            "daily-20261118",
        ] {
            assert!(
                !is_daily_copy_name(foreign),
                "the pattern must not claim `{foreign}`"
            );
        }
    }

    #[test]
    fn daily_backup_writes_one_valid_copy_named_for_the_utc_day() {
        let dir = temp_dir("daily-first");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        let (_db_path, writer) = database_with_marker(&dir);

        let outcome = daily_backup(&writer, &backup_dir, ts("2026-11-18T15:30:00.000Z"))
            .expect("the daily copy should be written");
        let DailyBackupOutcome::Written(backup) = outcome else {
            panic!("a day without a copy must be Written, got {outcome:?}");
        };
        assert_eq!(
            backup,
            backup_dir.join("daily-20261118.sqlite3"),
            "the copy is named for the UTC day"
        );

        // The copy is a valid database holding exactly the state at the
        // moment of the copy — un-checkpointed WAL frames included: the
        // writer connection is still open, so the marker row lives in
        // the `-wal` file, not the main database file.
        assert!(
            dir.join("db.sqlite3-wal").is_file(),
            "the fixture must hold un-checkpointed WAL frames"
        );
        let copy = Connection::open(&backup).unwrap();
        let integrity: String = copy
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert_eq!(user_version(&copy), 1, "the copy carries the schema");
        let marker: String = copy
            .query_row(
                "SELECT value FROM settings WHERE key = 'daily.marker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, "\"present\"", "the copy preserves the data");
        drop(copy);

        // No staging debris survives.
        assert_eq!(daily_names(&backup_dir), vec!["daily-20261118.sqlite3"]);

        // Windows: close the handles before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_same_day_rerun_writes_no_second_copy_and_changes_nothing() {
        // The ERR-001 acceptance criterion: a second run on the same
        // day must not create a second copy.
        let dir = temp_dir("daily-same-day");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        let (db_path, writer) = database_with_marker(&dir);

        let first = daily_backup(&writer, &backup_dir, ts("2026-11-18T08:00:00.000Z")).unwrap();
        let DailyBackupOutcome::Written(first_path) = &first else {
            panic!("the morning run writes the copy: {first:?}");
        };
        let bytes_before = fs::read(first_path).unwrap();
        let listing_before = fs::read_dir(&backup_dir).unwrap().count();

        // A later instant of the same UTC day, a different connection —
        // the restart shape.
        let second = {
            let conn = open_db(&db_path).unwrap();
            daily_backup(&conn, &backup_dir, ts("2026-11-18T21:45:00.000Z")).unwrap()
        };
        assert_eq!(
            second,
            DailyBackupOutcome::AlreadyPresent(backup_dir.join("daily-20261118.sqlite3")),
            "the evening run must find the copy and write nothing"
        );
        assert_eq!(
            fs::read(first_path).unwrap(),
            bytes_before,
            "the existing copy is never replaced"
        );
        assert_eq!(
            fs::read_dir(&backup_dir).unwrap().count(),
            listing_before,
            "no second copy, no debris: the directory is unchanged"
        );
        assert_eq!(daily_names(&backup_dir), vec!["daily-20261118.sqlite3"]);

        // Windows: close the handles before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Seeds `backup_dir` with synthetic daily copies for the given
    /// dates — the accumulated history of past startups (plain files
    /// suffice: the retention counts names, it never opens a copy).
    fn seed_daily_copies(backup_dir: &Path, dates: &[&str]) {
        for date in dates {
            fs::write(
                backup_dir.join(format!("daily-{date}.sqlite3")),
                b"synthetic history",
            )
            .unwrap();
        }
    }

    #[test]
    fn the_eighth_daily_copy_deletes_the_oldest_and_keeps_seven() {
        // The ERR-001 acceptance criterion: eight copies are one too
        // many — the oldest goes.
        let dir = temp_dir("daily-eighth");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        seed_daily_copies(
            &backup_dir,
            &[
                "20261112", "20261113", "20261114", "20261115", "20261116", "20261117", "20261119",
            ],
        );
        // A pre-migration copy and a foreign file: outside the cap's
        // pattern, they must both survive it.
        let pre_migration = backup_dir.join("pre-migration-v1-20261110T101500123Z.sqlite3");
        fs::write(&pre_migration, b"safety copy").unwrap();
        let foreign = backup_dir.join("user-notes.txt");
        fs::write(&foreign, b"user data").unwrap();
        let (_db_path, writer) = database_with_marker(&dir);

        let outcome = daily_backup(&writer, &backup_dir, ts("2026-11-18T10:00:00.000Z")).unwrap();
        assert!(
            matches!(outcome, DailyBackupOutcome::Written(_)),
            "the day had no copy: {outcome:?}"
        );

        // Eight daily names existed; the oldest (20261112) is gone and
        // exactly the newest seven remain.
        assert_eq!(
            daily_names(&backup_dir),
            vec![
                "daily-20261113.sqlite3",
                "daily-20261114.sqlite3",
                "daily-20261115.sqlite3",
                "daily-20261116.sqlite3",
                "daily-20261117.sqlite3",
                "daily-20261118.sqlite3",
                "daily-20261119.sqlite3",
            ],
            "the eighth copy deletes the oldest daily copy"
        );
        assert!(
            pre_migration.is_file(),
            "the cap never deletes pre-migration copies"
        );
        assert!(foreign.is_file(), "the cap never deletes foreign files");

        // Windows: close the handle before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_prune_never_deletes_the_copy_it_just_wrote() {
        // The clock-set-back state: seven daily copies whose names are
        // all *newer* than "today". Writing today's copy must keep it
        // — the prune drops the oldest of the others instead — or the
        // day would lose its copy immediately and every restart would
        // repeat a full `VACUUM INTO`.
        let dir = temp_dir("daily-keep-written");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        seed_daily_copies(
            &backup_dir,
            &[
                "20261119", "20261120", "20261121", "20261122", "20261123", "20261124", "20261125",
            ],
        );
        let (_db_path, writer) = database_with_marker(&dir);

        let outcome = daily_backup(&writer, &backup_dir, ts("2026-11-18T10:00:00.000Z")).unwrap();
        let DailyBackupOutcome::Written(backup) = outcome else {
            panic!("today had no copy: {outcome:?}");
        };
        assert!(
            backup.is_file(),
            "the copy just written must survive the prune"
        );
        let names = daily_names(&backup_dir);
        assert_eq!(names.len(), 7, "the cap still holds: {names:?}");
        assert_eq!(
            names.first(),
            Some(&"daily-20261118.sqlite3".to_owned()),
            "today's copy is the oldest name and still kept: {names:?}"
        );
        assert!(
            !backup_dir.join("daily-20261119.sqlite3").exists(),
            "the oldest of the *others* is the one deleted"
        );

        // Windows: close the handle before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_seventh_daily_copy_deletes_nothing() {
        // The cap boundary from below: seven copies fit.
        let dir = temp_dir("daily-seventh");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        seed_daily_copies(
            &backup_dir,
            &[
                "20261113", "20261114", "20261115", "20261116", "20261117", "20261118",
            ],
        );
        let (_db_path, writer) = database_with_marker(&dir);

        daily_backup(&writer, &backup_dir, ts("2026-11-19T10:00:00.000Z")).unwrap();
        assert_eq!(
            daily_names(&backup_dir),
            vec![
                "daily-20261113.sqlite3",
                "daily-20261114.sqlite3",
                "daily-20261115.sqlite3",
                "daily-20261116.sqlite3",
                "daily-20261117.sqlite3",
                "daily-20261118.sqlite3",
                "daily-20261119.sqlite3",
            ],
            "seven copies: nothing is deleted"
        );

        // Windows: close the handle before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_no_op_daily_run_does_not_prune() {
        // A run that finds its copy present changes nothing — even
        // when the directory holds more daily copies than the cap
        // (clock stepped back, files restored by hand): the cap is
        // enforced by writes, and the next write restores it.
        let dir = temp_dir("daily-no-prune");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        seed_daily_copies(
            &backup_dir,
            &[
                "20261118", "20261119", "20261120", "20261121", "20261122", "20261123", "20261124",
                "20261125",
            ],
        );
        let (db_path, writer) = database_with_marker(&dir);

        let outcome = daily_backup(&writer, &backup_dir, ts("2026-11-18T10:00:00.000Z")).unwrap();
        assert!(
            matches!(outcome, DailyBackupOutcome::AlreadyPresent(_)),
            "today's copy exists: {outcome:?}"
        );
        assert_eq!(
            daily_names(&backup_dir).len(),
            8,
            "a no-op run deletes nothing"
        );

        // And the self-heal: the next *write* prunes back to seven.
        {
            let conn = open_db(&db_path).unwrap();
            daily_backup(&conn, &backup_dir, ts("2026-11-26T10:00:00.000Z")).unwrap();
        }
        assert_eq!(
            daily_names(&backup_dir).len(),
            7,
            "the next write restores the cap"
        );

        // Windows: close the handles before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_daily_run_sweeps_its_own_stale_temporaries_only() {
        let dir = temp_dir("daily-sweep");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        // Debris of a crashed daily run (a full daily name plus the
        // staging suffix), near-miss names that must NOT be claimed
        // (a daily-shaped word, another scheme's debris), and a
        // finished daily copy that must not be touched.
        let stale = backup_dir.join("daily-20260101.sqlite3.tmp");
        fs::write(&stale, b"half-written debris").unwrap();
        let near_miss_word = backup_dir.join("daily-notes.sqlite3.tmp");
        fs::write(&near_miss_word, b"a user file in daily clothing").unwrap();
        let foreign_scheme = backup_dir.join("pre-migration-v1-20000101T000000000Z.sqlite3.tmp");
        fs::write(&foreign_scheme, b"other scheme's debris").unwrap();
        let existing = backup_dir.join("daily-20261118.sqlite3");
        fs::write(&existing, b"today's copy").unwrap();
        let (_db_path, writer) = database_with_marker(&dir);

        let outcome = daily_backup(&writer, &backup_dir, ts("2026-11-18T10:00:00.000Z")).unwrap();
        assert!(
            matches!(outcome, DailyBackupOutcome::AlreadyPresent(_)),
            "today's copy exists: {outcome:?}"
        );
        assert!(
            !stale.exists(),
            "the daily run sweeps its own `.tmp` debris"
        );
        assert!(
            near_miss_word.exists(),
            "a `.tmp` that is not a full daily name is not this scheme's to delete"
        );
        assert!(
            foreign_scheme.exists(),
            "the pre-migration scheme's `.tmp` is the migration sweep's, not the daily run's"
        );
        assert_eq!(
            fs::read(&existing).unwrap(),
            b"today's copy",
            "an existing copy is never replaced or removed"
        );

        // Windows: close the handle before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_backup_directory_fails_the_daily_copy_without_creating_it() {
        let dir = temp_dir("daily-missing-dir");
        let (_db_path, writer) = database_with_marker(&dir);
        // `db-backups` deliberately not created: the layout is the app
        // layer's (D-027).
        let backup_dir = dir.join("db-backups");

        let err = daily_backup(&writer, &backup_dir, ts("2026-11-18T10:00:00.000Z"))
            .expect_err("a missing backup directory must fail the daily copy");
        match err {
            StorageError::Backup { path, source } => {
                assert_eq!(path, backup_dir, "the error names the directory");
                assert!(
                    matches!(source, BackupError::Directory),
                    "expected BackupError::Directory, got: {source:?}"
                );
            }
            other => panic!("expected StorageError::Backup, got: {other:?}"),
        }
        assert!(
            !backup_dir.exists(),
            "the crate must not create the directory"
        );

        // Windows: close the handle before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_backup_directory_fails_the_daily_copy_and_leaves_no_debris() {
        // The failure the degradation policy exists for: the copy
        // cannot be written, the error keeps its own shape
        // (`StorageError::Backup`), and no half-written file survives.
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("daily-unwritable");
        if running_as_root_is_untestable(&dir) {
            fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        let (_db_path, writer) = database_with_marker(&dir);

        fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o555))
            .expect("removing write rights must work");
        let result = daily_backup(&writer, &backup_dir, ts("2026-11-18T10:00:00.000Z"));
        fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o755))
            .expect("restoring write rights must work");
        let err = result.expect_err("the unwritable directory must fail the copy");
        assert!(
            matches!(err, StorageError::Backup { .. }),
            "a failed daily copy must surface as StorageError::Backup, got: {err:?}"
        );
        assert!(
            daily_names(&backup_dir).is_empty(),
            "no copy and no debris survive the failure"
        );

        // Windows: close the handle before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    // ---- owner-only permissions (spec §13 SEC-007, Unix) --------------------
    //
    // The card's acceptance criterion is a Linux/macOS test of the modes
    // after the first run; the app-layer end-to-end version of it lives
    // in `db_tests.rs`. Here the store's own artifacts are pinned at
    // their producing seams: the database file at `open_db`, every
    // backup copy at the shared staging step, and the WAL siblings
    // through the mode inheritance the `open_db` ordering builds on.

    /// Reads a path's permission bits masked to the standard `0o777` —
    /// what a mode assertion compares, without the file-type noise the
    /// raw `st_mode` carries.
    #[cfg(unix)]
    fn file_mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .unwrap_or_else(|err| panic!("stat of {} must work: {err}", path.display()))
            .permissions()
            .mode()
            & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn open_db_creates_the_database_file_owner_only() {
        // SEC-007: the database file is `0600` on Unix — exact mode,
        // not merely owner-writable, so the assertion catches a
        // umask-dependent `0644` default as well as a dropped chmod.
        let dir = temp_dir("mode-fresh");
        let db_path = dir.join("db.sqlite3");

        let conn = open_db(&db_path).expect("a fresh database must open");

        assert_eq!(
            file_mode(&db_path),
            0o600,
            "the database file must be owner-only (SEC-007)"
        );

        // Windows: close the handle before removing the directory.
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn open_db_repairs_a_loosened_database_file_mode() {
        // The mode is re-asserted on every open (the convergence
        // policy): a file that drifted — here loosened by hand to the
        // OS default — converges back on the next open, not only on
        // the first creation.
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("mode-repair");
        let db_path = dir.join("db.sqlite3");
        drop(open_db(&db_path).unwrap());
        fs::set_permissions(&db_path, fs::Permissions::from_mode(0o644))
            .expect("loosening the mode must work");

        drop(open_db(&db_path).expect("the re-open must succeed"));

        assert_eq!(
            file_mode(&db_path),
            0o600,
            "a re-open must repair a loosened mode (SEC-007)"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_wal_siblings_of_an_open_database_are_owner_only() {
        // The `-wal`/`-shm` siblings hold the same data as the database
        // (un-checkpointed committed frames included), so SEC-007's
        // spirit covers them even though the spec names only the file.
        // The mechanism is `open_db`'s ordering: the file is `0600`
        // *before* the WAL switch, and SQLite copies the main file's
        // mode when it creates the siblings — this test pins that
        // empirically; an exact `0600` is not demanded because the
        // owner bits are SQLite's choice, only the absence of group
        // and other access is ours.
        let dir = temp_dir("mode-sidecars");
        let db_path = dir.join("db.sqlite3");

        let conn = open_db(&db_path).expect("a fresh database must open");
        // A write forces real WAL frames, so both siblings exist while
        // the connection is open (a clean close would checkpoint and
        // delete them).
        conn.execute("CREATE TABLE probe (x TEXT)", [])
            .expect("the probe write must work");

        for sibling in ["db.sqlite3-wal", "db.sqlite3-shm"] {
            let path = dir.join(sibling);
            assert!(
                path.is_file(),
                "`{sibling}` must exist under an open WAL connection"
            );
            assert_eq!(
                file_mode(&path) & 0o077,
                0,
                "`{sibling}` must not be accessible by group or other (SEC-007)"
            );
        }

        // Windows: close the handle before removing the directory.
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_loosened_pre_existing_wal_sibling_is_repaired_on_the_next_open() {
        // The mode copy covers siblings SQLite *creates*; one that
        // already exists (a crash leftover, a restored directory —
        // here loosened by hand while the first connection keeps it
        // alive) is reused untouched, so `open_db` repairs it
        // best-effort: the second open must converge it back even
        // though nothing recreates it.
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("mode-sidecar-repair");
        let db_path = dir.join("db.sqlite3");
        let writer = open_db(&db_path).expect("the first open must work");
        writer
            .execute("CREATE TABLE probe (x TEXT)", [])
            .expect("the probe write must work");
        let wal = dir.join("db.sqlite3-wal");
        assert!(wal.is_file(), "the WAL sibling must exist");
        fs::set_permissions(&wal, fs::Permissions::from_mode(0o644))
            .expect("loosening the WAL mode must work");

        // A second connection while the first stays open: the sibling
        // exists, so no creation-time mode copy applies.
        drop(open_db(&db_path).expect("the second open must succeed"));

        assert_eq!(
            file_mode(&wal) & 0o077,
            0,
            "a pre-existing loosened `-wal` must be re-tightened on the \
             next open (SEC-007)"
        );

        // Windows: close the handles before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_pre_migration_copy_is_owner_only() {
        // The copy is a full image of the database's data (DB-004), so
        // it shares the file's SEC-007 mode; the producing seam is the
        // staging step both copy schemes go through.
        let dir = temp_dir("mode-premigration");
        let db_path = dir.join("db.sqlite3");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();

        let set_v1 = set_of(&["CREATE TABLE marker (value TEXT NOT NULL)"]);
        {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v1, &backup_dir).unwrap();
        }
        let set_v2 = set_of(&[
            "CREATE TABLE marker (value TEXT NOT NULL)",
            "ALTER TABLE marker ADD COLUMN added_by_v2 TEXT",
        ]);
        let backups = {
            let mut conn = open_db(&db_path).unwrap();
            migrate(&mut conn, &set_v2, &backup_dir).expect("the upgrade run should succeed")
        };

        assert_eq!(backups.len(), 1, "one pending step, one copy");
        assert_eq!(
            file_mode(&backups[0]),
            0o600,
            "the pre-migration copy must be owner-only (SEC-007)"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_daily_copy_is_owner_only() {
        // Same data, same mode (ERR-001's copies share the staging
        // step with the pre-migration ones, so one mechanism covers
        // both schemes).
        let dir = temp_dir("mode-daily");
        let backup_dir = dir.join("db-backups");
        fs::create_dir(&backup_dir).unwrap();
        let (_db_path, writer) = database_with_marker(&dir);

        let outcome = daily_backup(&writer, &backup_dir, ts("2026-11-18T10:00:00.000Z"))
            .expect("the daily copy should be written");
        let DailyBackupOutcome::Written(backup) = outcome else {
            panic!("a day without a copy must be Written, got {outcome:?}");
        };

        assert_eq!(
            file_mode(&backup),
            0o600,
            "the daily copy must be owner-only (SEC-007)"
        );

        // Windows: close the handle before removing the directory.
        drop(writer);
        fs::remove_dir_all(&dir).unwrap();
    }
}
