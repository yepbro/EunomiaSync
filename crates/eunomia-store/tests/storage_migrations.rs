use eunomia_store::storage::run_migrations;
use rusqlite::Connection;

#[test]
fn fresh_database_reaches_latest_migration() {
    let mut conn = Connection::open_in_memory().unwrap();
    run_migrations(&mut conn).expect("migrations should apply cleanly on a fresh database");

    // The `settings` table created by 0001_init.sql must be usable.
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

#[test]
fn migrations_run_on_file_backed_database() {
    let path =
        std::env::temp_dir().join(format!("eunomia-store-test-{}.sqlite3", std::process::id()));
    // A pre-existing file would leak state between runs; start fresh.
    let _ = std::fs::remove_file(&path);

    {
        let mut conn = Connection::open(&path).unwrap();
        run_migrations(&mut conn).unwrap();
    }

    // Reopening must report a fully migrated database: schema version 1
    // (0001_init.sql is the only migration so far, spec DB-004).
    {
        let mut conn = Connection::open(&path).unwrap();
        run_migrations(&mut conn)
            .expect("re-running migrations on a migrated database must be a no-op");
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_version, 1, "schema version must be exactly 1");
    }

    std::fs::remove_file(&path).unwrap();
}
