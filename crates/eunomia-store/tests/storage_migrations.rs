use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::storage::run_migrations;
use rusqlite::Connection;

/// Creates a unique temporary directory holding a `db/` (the database
/// file) and a `db-backups/` subdirectory, mimicking the app-side
/// data-directory layout (the directory structure is owned by the app
/// layer, DECISIONS.md D-027; the store crate receives the paths as
/// parameters and creates neither).
fn temp_data_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-migrations-{label}-{}-{nanos}",
        std::process::id(),
    ));
    fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    fs::create_dir(dir.join("db-backups")).expect("temp backup directory should be creatable");
    dir
}

fn db_path(dir: &Path) -> PathBuf {
    dir.join("db").join("eunomiasync.sqlite3")
}

fn backup_dir(dir: &Path) -> PathBuf {
    dir.join("db-backups")
}

/// The names of the files currently in the backup directory.
fn backup_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(backup_dir(dir))
        .expect("the backup directory should be listable")
        .map(|entry| {
            entry
                .expect("directory entry should be readable")
                .file_name()
                .to_str()
                .expect("file names are ASCII")
                .to_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn fresh_database_reaches_latest_migration_without_backups() {
    let dir = temp_data_dir("fresh");
    let path = db_path(&dir);

    let backups = {
        let mut conn = Connection::open(&path).unwrap();
        run_migrations(&mut conn, &backup_dir(&dir))
            .expect("migrations should apply cleanly on a fresh database")
    };

    // Schema v0 → v1 on an empty database is a bootstrap, not an upgrade:
    // there is no state to lose, so no pre-migration copy is written
    // (spec DB-004, DECISIONS.md D-038).
    assert!(
        backups.is_empty(),
        "a fresh database must not produce backup copies"
    );
    assert!(
        backup_files(&dir).is_empty(),
        "the backup directory stays empty"
    );

    // The `settings` table created by 0001_init.sql must be usable.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('schema', '0001', '2026-11-18T10:00:00.000Z')",
            [],
        )
        .unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'schema'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "0001");
    }

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn migrations_run_on_file_backed_database() {
    let dir = temp_data_dir("noop");
    let path = db_path(&dir);

    {
        let mut conn = Connection::open(&path).unwrap();
        run_migrations(&mut conn, &backup_dir(&dir)).unwrap();
    }

    // Reopening must report a fully migrated database: schema version 1
    // (0001_init.sql is the only migration so far, spec DB-004). The
    // no-op rerun writes no backup copy.
    {
        let mut conn = Connection::open(&path).unwrap();
        let backups = run_migrations(&mut conn, &backup_dir(&dir))
            .expect("re-running migrations on a migrated database must be a no-op");
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_version, 1, "schema version must be exactly 1");
        assert!(backups.is_empty(), "a no-op run must not produce copies");
        assert!(
            backup_files(&dir).is_empty(),
            "the backup directory stays empty"
        );
    }

    fs::remove_dir_all(&dir).unwrap();
}
