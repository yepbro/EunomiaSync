//! SQLite storage for EunomiaSync.
//!
//! The database schema is managed with SQL migration files embedded into
//! the binary via [`include_str!`]. `rusqlite_migration` tracks the applied
//! schema version through the SQLite `user_version` pragma, so upgrading an
//! existing database only requires adding the next migration file.

use std::fmt;

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

/// The ordered list of embedded SQL migrations applied by [`run_migrations`].
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

/// Errors that can occur while opening or migrating the database.
#[derive(Debug)]
pub enum StorageError {
    /// A migration failed to apply.
    Migration(rusqlite_migration::Error),
    /// A SQL statement failed.
    Sql(rusqlite::Error),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Migration(err) => write!(f, "database migration failed: {err}"),
            StorageError::Sql(err) => write!(f, "database error: {err}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Migration(err) => Some(err),
            StorageError::Sql(err) => Some(err),
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

/// Migrates `conn` to the latest schema version.
///
/// On a fresh database this applies every migration in order; on an
/// existing database it only applies the pending ones.
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
