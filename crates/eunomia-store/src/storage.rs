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
//! transaction.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

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

/// The busy handler timeout applied by [`open_db`] (spec DB-001).
///
/// Set through [`Connection::busy_timeout`], which programs the same SQLite
/// busy handler as the `busy_timeout` pragma; reading `PRAGMA busy_timeout`
/// back reports this value.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Errors that can occur while opening or migrating the database.
#[derive(Debug)]
pub enum StorageError {
    /// A migration failed to apply.
    Migration(rusqlite_migration::Error),
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
    /// Writing the pre-migration backup copy failed (spec DB-004).
    Backup {
        /// The backup path the run was producing (the final file name;
        /// its staged temporary is cleaned up best-effort), or the
        /// backup directory itself when it is missing.
        path: PathBuf,
        /// The step that failed.
        source: BackupError,
    },
}

/// The step of a pre-migration backup write that failed.
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
            StorageError::Migration(err) => write!(f, "database migration failed: {err}"),
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
                    "failed to write the pre-migration backup at {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Migration(err) => Some(err),
            StorageError::Sql(err) => Some(err),
            StorageError::Open { source, .. } => Some(source),
            StorageError::JournalMode { .. } => None,
            StorageError::Backup { source, .. } => Some(source),
        }
    }
}

impl From<rusqlite_migration::Error> for StorageError {
    fn from(err: rusqlite_migration::Error) -> Self {
        StorageError::Migration(err)
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

/// Opens the SQLite database file at `path` with the required pragma set.
///
/// The file is created when it does not exist yet, but parent directories
/// are never created: the app layer owns the data-directory layout (see
/// DECISIONS.md D-027) and must create `db/` before calling this function.
/// A missing parent surfaces as [`StorageError::Open`] naming the path.
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
/// pragma fails, and [`StorageError::JournalMode`] when SQLite reports a
/// journal mode other than `wal` after the update attempt.
pub fn open_db(path: &Path) -> Result<Connection, StorageError> {
    let conn = Connection::open(path).map_err(|source| open_failed(path, source))?;

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
/// back and the database stays at the last committed step.
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
/// read, the database is too far ahead, or a migration fails;
/// [`StorageError::Backup`] when a safety copy cannot be written.
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
        return Err(StorageError::Migration(
            rusqlite_migration::Error::MigrationDefinition(
                rusqlite_migration::MigrationDefinitionError::DatabaseTooFarAhead,
            ),
        ));
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
        sweep_stale_temporaries(backup_dir)?;
    }
    for target in (start + 1)..=latest {
        // A database that already carries a schema version is backed up
        // before every step; a run that starts from version 0 creates
        // the schema and has nothing to lose.
        if start > 0 {
            let stamp = utc_stamp(Timestamp::now());
            backups.push(vacuum_into_backup(conn, backup_dir, current, &stamp)?);
        }
        set.to_version(conn, target)?;
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

/// Removes leftover staging files of crashed earlier backup writes.
///
/// A crash between `VACUUM INTO` and the final link leaves a
/// `pre-migration-v*.sqlite3.tmp` file; the millisecond stamp makes the
/// next run land on a fresh name, so without this sweep the debris would
/// accumulate. Foreign `.tmp` files are deliberately untouched — only
/// this crate's own naming pattern is removed. Runs are serialized (see
/// [`migrate`]), so a `.tmp` matching this pattern can only be stale.
/// Individual removal failures are ignored: a held file must not block
/// the run, and the write path never reuses its exact name.
fn sweep_stale_temporaries(backup_dir: &Path) -> Result<(), StorageError> {
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
        if name.starts_with("pre-migration-v") && name.ends_with(".sqlite3.tmp") {
            let _ = retry_transient_hold(|| std::fs::remove_file(entry.path()));
        }
    }
    Ok(())
}

/// Runs `op`, retrying the transient Windows holds spec §9.5 prescribes
/// for file writes (an antivirus scanner briefly holding a freshly
/// created file): a sharing violation or access denied is retried up to
/// three more times after 100 ms, 300 ms and 900 ms pauses, then the
/// last error is returned.
fn retry_transient_hold<T>(mut op: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
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
/// into place with [`std::fs::hard_link`] (the temporary is removed
/// afterwards): unlike a rename, which silently replaces an existing
/// destination on every supported platform, a hard link fails if the
/// final name exists — so an existing copy is never overwritten even in
/// the window between the name search and the link. A crash mid-copy
/// leaves the `.tmp` file, which the next run removes (see
/// [`sweep_stale_temporaries`]); the final name only ever appears
/// complete.
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

        let mut tmp_name = final_path.clone().into_os_string();
        tmp_name.push(".tmp");
        let tmp_path = PathBuf::from(tmp_name);

        // A `.tmp` at this exact name can only be left over from a
        // crashed earlier run (the caller serializes migration runs);
        // `VACUUM INTO` refuses an existing file, so remove it first.
        if let Err(err) = retry_transient_hold(|| std::fs::remove_file(&tmp_path))
            && err.kind() != io::ErrorKind::NotFound
        {
            return Err(StorageError::Backup {
                path: final_path,
                source: BackupError::Io(err),
            });
        }

        let tmp_str = tmp_path.to_str().ok_or_else(|| StorageError::Backup {
            path: final_path.clone(),
            source: BackupError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "the backup path is not valid UTF-8",
            )),
        })?;
        if let Err(err) = conn.execute("VACUUM INTO ?1", [tmp_str]) {
            // Best-effort cleanup; a surviving `.tmp` stays recognizable.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(StorageError::Backup {
                path: final_path,
                source: BackupError::Sqlite(err),
            });
        }
        match retry_transient_hold(|| std::fs::hard_link(&tmp_path, &final_path)) {
            Ok(()) => {}
            // A file appeared under the name in the window between the
            // search and the link: take the next name and copy again.
            // This branch is defensive — under the serialized-runs
            // assumption nothing else creates files here — and is
            // deliberately not covered by a test.
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&tmp_path);
                attempt += 1;
                continue;
            }
            Err(err) => {
                // Best-effort cleanup of the staged temporary; a
                // survivor stays recognizable (the sweep removes it).
                let _ = std::fs::remove_file(&tmp_path);
                return Err(StorageError::Backup {
                    path: final_path,
                    source: BackupError::Io(err),
                });
            }
        }
        // The final name is complete now; the temporary link is pure
        // debris, so its removal is best-effort: a file a scanner keeps
        // holding must not fail an otherwise finished copy (the sweep
        // removes the leftover on a later run).
        let _ = retry_transient_hold(|| std::fs::remove_file(&tmp_path));
        return Ok(final_path);
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
            matches!(err, StorageError::Migration(_)),
            "expected StorageError::Migration, got: {err:?}"
        );
        assert_eq!(user_version(&conn), 2, "the version must be untouched");
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

        // Step 2 applies; step 3 is broken and must roll back alone.
        let set_v3 = set_of(&[
            "CREATE TABLE t1 (x TEXT)",
            "CREATE TABLE t2 (x TEXT)",
            "THIS IS NOT SQL",
        ]);
        let err = {
            let mut conn = Connection::open(&db_path).unwrap();
            migrate(&mut conn, &set_v3, &backup_dir).expect_err("the broken step must fail the run")
        };
        assert!(
            matches!(err, StorageError::Migration(_)),
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
}
