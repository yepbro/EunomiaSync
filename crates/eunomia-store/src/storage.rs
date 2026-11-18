//! SQLite storage for EunomiaSync.
//!
//! The database schema is managed with SQL migration files embedded into
//! the binary via [`include_str!`]. `rusqlite_migration` tracks the applied
//! schema version through the SQLite `user_version` pragma, so upgrading an
//! existing database only requires adding the next migration file.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

/// The ordered list of embedded SQL migrations applied by [`run_migrations`].
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

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
///    [`Connection::busy_timeout`], which programs the same SQLite busy
///    handler as the pragma does;
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

/// Migrates `conn` to the latest schema version.
///
/// On a fresh database this applies every migration in order; on an
/// existing database it only applies the pending ones.
///
/// The connection's pragmas are left as-is. With `open_db`'s
/// `foreign_keys=ON` this is safe for migrations that only create
/// objects, but a future rebuild-style migration (SQLite's twelve-step
/// `ALTER TABLE` procedure) cannot switch foreign keys off inside
/// `rusqlite_migration`'s per-migration transactions — that migration
/// needs its own foreign-keys policy decision before it lands (see
/// DECISIONS.md D-030).
pub fn run_migrations(conn: &mut Connection) -> Result<(), StorageError> {
    let migrations = Migrations::new(MIGRATIONS.iter().map(|sql| M::up(sql)).collect());
    migrations.to_latest(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_migrations_creates_meta_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        run_migrations(&mut conn).unwrap();
    }
}
