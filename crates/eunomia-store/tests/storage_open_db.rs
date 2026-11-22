use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::storage::{StorageError, open_db, run_migrations};
use rusqlite::Connection;

/// Creates a unique temporary directory holding a `db/` subdirectory and
/// returns its path, mimicking the app-side data-directory layout (the
/// directory structure is owned by the app layer, D-027; `open_db` itself
/// must not create it).
fn temp_db_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-open-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp directory should be creatable");
    std::fs::create_dir_all(dir.join("db-backups")).expect("temp directory should be creatable");
    dir
}

fn db_path(dir: &Path) -> PathBuf {
    dir.join("db").join("eunomiasync.sqlite3")
}

fn backup_dir(dir: &Path) -> PathBuf {
    dir.join("db-backups")
}

/// The spec DB-001 pragma values read back from an open connection.
struct Pragmas {
    journal_mode: String,
    foreign_keys: i64,
    busy_timeout: i64,
    synchronous: i64,
}

/// Reads the spec DB-001 pragma set back from an open connection.
fn read_pragmas(conn: &Connection) -> Pragmas {
    Pragmas {
        journal_mode: conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("PRAGMA journal_mode should be readable"),
        foreign_keys: conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("PRAGMA foreign_keys should be readable"),
        busy_timeout: conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("PRAGMA busy_timeout should be readable"),
        synchronous: conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("PRAGMA synchronous should be readable"),
    }
}

/// Asserts the DB-001 values on the pragma set from [`read_pragmas`].
///
/// Honest limitation, verified empirically against the bundled build
/// (rusqlite 0.41.1, libsqlite3-sys 0.39.1): the defaults of a plain
/// `Connection::open` on a fresh file are `journal_mode="delete"`,
/// `foreign_keys=1` (the bundled SQLite compiles with
/// `SQLITE_DEFAULT_FOREIGN_KEYS=1`), `busy_timeout=5000` (rusqlite sets
/// it on every open) and `synchronous=2` (the SQLite compile-time
/// default, unchanged in WAL mode). The `foreign_keys`, `busy_timeout`
/// and `synchronous` assertions therefore pin the observable contract
/// and start failing the moment a library change moves those defaults;
/// only the `journal_mode` assertion currently detects its pragma being
/// removed from `open_db` (see DECISIONS.md D-030).
fn assert_db001_pragmas(pragmas: &Pragmas) {
    assert_eq!(pragmas.journal_mode, "wal", "journal_mode must be WAL");
    assert_eq!(pragmas.foreign_keys, 1, "foreign_keys must be ON");
    assert_eq!(pragmas.busy_timeout, 5000, "busy_timeout must be 5000 ms");
    assert_eq!(pragmas.synchronous, 2, "synchronous must be FULL (2)");
}

#[test]
fn open_db_applies_the_full_pragma_set() {
    let dir = temp_db_dir("fresh");
    let path = db_path(&dir);

    let conn = open_db(&path).expect("open_db should open a fresh database file");
    assert!(path.exists(), "the database file should be created");

    assert_db001_pragmas(&read_pragmas(&conn));

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn open_db_reapplies_the_pragma_set_on_every_open() {
    let dir = temp_db_dir("reopen");
    let path = db_path(&dir);

    {
        let conn = open_db(&path).expect("the first open should succeed");
        assert_db001_pragmas(&read_pragmas(&conn));
    }

    // The journal mode is persisted in the file header, so a plain reopen
    // would keep reporting `wal` no matter what the second open does, and
    // the other three pragmas equal the DB-001 values by library default
    // (D-030). Force the file back to a rollback journal first: the second
    // open_db call must then switch it to WAL again for the test to pass.
    {
        let plain = Connection::open(&path).expect("plain reopen should succeed");
        let mode: String = plain
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
            .expect("journal_mode should switch back to delete");
        assert_eq!(mode, "delete", "the file should no longer be in WAL mode");
    }

    {
        let conn = open_db(&path).expect("reopening the same file should succeed");
        assert_db001_pragmas(&read_pragmas(&conn));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn open_db_rejects_a_database_that_cannot_use_wal() {
    // An in-memory connection reports journal_mode "memory" after the
    // update attempt — the known trigger for the WAL verification guard:
    // the open must fail instead of returning a connection without the
    // NFR-007 crash-safety guarantees.
    let err =
        open_db(Path::new(":memory:")).expect_err("an in-memory database cannot switch to WAL");
    let rendered = err.to_string();
    assert!(
        rendered.contains(":memory:"),
        "the error message should name the path, got: {rendered}"
    );

    match err {
        StorageError::JournalMode { path, mode } => {
            assert_eq!(
                path,
                Path::new(":memory:"),
                "the error should carry the path"
            );
            assert_eq!(mode, "memory", "the reported mode should be echoed");
        }
        other => panic!("expected StorageError::JournalMode, got: {other:?}"),
    }
}

#[test]
fn open_db_fails_without_creating_parent_directories() {
    let dir = temp_db_dir("no-parent");
    // Skip the `db/` level the helper created: point at a parent that
    // deliberately does not exist.
    let parent = dir.join("missing-parent");
    let path = parent.join("eunomiasync.sqlite3");

    let err = open_db(&path)
        .expect_err("open_db must not create parent directories; a missing parent is an error");

    assert!(
        matches!(err, StorageError::Open { .. }),
        "a missing parent must surface as StorageError::Open, got: {err:?}"
    );
    assert!(
        err.to_string()
            .contains(&path.to_string_lossy().to_string()),
        "the error message must name the offending path, got: {err}"
    );
    assert!(
        !parent.exists(),
        "open_db must not create the missing parent directory"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn open_db_then_run_migrations_on_the_same_connection() {
    // The production startup sequence: open with the DB-001 pragma set,
    // then migrate the same connection (DB-004).
    let dir = temp_db_dir("migrate");
    let path = db_path(&dir);

    {
        let mut conn = open_db(&path).expect("open_db should succeed");
        let backups = run_migrations(&mut conn, &backup_dir(&dir))
            .expect("migrations should apply on the pragma-configured connection");
        assert!(
            backups.is_empty(),
            "the fresh database produces no backup copies"
        );

        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('schema', '0001', '2026-11-18T10:00:00.000Z')",
            [],
        )
        .expect("a settings write should succeed after migrations");
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version should be readable");
        assert_eq!(user_version, 2, "schema version must be exactly 2");
        assert_db001_pragmas(&read_pragmas(&conn));
    }

    // Reopening the migrated file: the pragma set is reapplied and the
    // schema stays migrated.
    {
        let mut conn = open_db(&path).expect("reopening the migrated file should succeed");
        run_migrations(&mut conn, &backup_dir(&dir))
            .expect("re-running migrations on a migrated database must be a no-op");
        assert_db001_pragmas(&read_pragmas(&conn));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}
