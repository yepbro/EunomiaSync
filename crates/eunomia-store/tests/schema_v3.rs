//! Behavior tests for schema version 3 (T-331): the MAIN-007/D-125
//! per-project custom-text assignment target.
//!
//! The oracle is the current spec §6.2/DB-012 shape, not the migration's
//! own text: an assignment has a direct instruction FK, exactly one of a
//! same-instruction variant or custom text, and custom text obeys DB-006.
//! Existing variant-backed rows must survive the v2→v3 rebuild verbatim.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::storage::{open_db, run_migrations};
use rusqlite::Connection;

struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates this test's scratch directory.
///
/// `label` is the uniqueness namespace inside this binary and must stay
/// unique per test: the directory name is `label` + pid + clock nanos,
/// and the clock is only microsecond-fine on some CI runners, so two
/// tests sharing a label can land on one clock tick and share one
/// directory — CI run 45542135360 (macos-15-intel) failed exactly that
/// way, its two tests colliding on one `db.sqlite3` (D-195). Distinct
/// labels make the directories disjoint by construction.
fn temp_dir(label: &str) -> TempDir {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is past the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "eunomia-store-schema-v3-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("temp directory creates");
    TempDir(path)
}

const V1_SQL: &str = include_str!("../migrations/0001_init.sql");
const V2_SQL: &str = include_str!("../migrations/0002_backup_records_pair_index.sql");
const TS: &str = "2026-11-24T12:00:00.000Z";

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version must be readable")
}

fn seed_v2(conn: &Connection) {
    conn.execute_batch("BEGIN")
        .expect("fixture transaction starts");
    conn.execute_batch(V1_SQL).expect("v1 schema applies");
    conn.execute_batch(V2_SQL).expect("v2 index applies");
    conn.pragma_update(None, "user_version", 2)
        .expect("fixture version sets");
    seed_project(conn, "p1");
    seed_instruction(conn, "i1", "v1");
    seed_assignment(conn, "p1", "i1", "v1");
    conn.execute_batch("COMMIT")
        .expect("fixture transaction commits");
    assert_eq!(user_version(conn), 2);
}

fn seed_project(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO projects (
            id, path, path_key, name, label, color, state, added_via,
            sort_order, scheme, base_name, created_at, updated_at
         ) VALUES (?1, ?2, ?2, ?1, 'P', 'blue', 'active', 'manual', 1,
                   'A', 'AGENTS.md', ?3, ?3)",
        rusqlite::params![id, format!("/repos/{id}"), TS],
    )
    .expect("project seed inserts");
}

fn seed_instruction(conn: &Connection, instruction_id: &str, variant_id: &str) {
    conn.execute(
        "INSERT INTO library_items (
            id, kind, position, heading_level, title, state, created_at, updated_at
         ) VALUES (?1, 'instruction', 1, NULL, ?1, 'active', ?2, ?2)",
        rusqlite::params![instruction_id, TS],
    )
    .expect("instruction seed inserts");
    conn.execute(
        "INSERT INTO instruction_variants (
            id, instruction_id, name, content, is_main, state, created_at, updated_at
         ) VALUES (?1, ?2, 'Main', 'shared text', 1, 'active', ?3, ?3)",
        rusqlite::params![variant_id, instruction_id, TS],
    )
    .expect("variant seed inserts");
}

fn seed_assignment(conn: &Connection, project_id: &str, instruction_id: &str, variant_id: &str) {
    conn.execute(
        "INSERT INTO project_instructions (
            project_id, instruction_id, variant_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?4)",
        rusqlite::params![project_id, instruction_id, variant_id, TS],
    )
    .expect("assignment seed inserts");
}

#[test]
fn v2_assignments_upgrade_verbatim_to_the_v3_target_shape() {
    let temp = temp_dir("upgrade");
    let db_path = temp.path().join("db.sqlite3");
    let backup_dir = temp.path().join("db-backups");
    std::fs::create_dir_all(&backup_dir).expect("backup directory creates");
    let mut conn = open_db(&db_path).expect("database opens");
    seed_v2(&conn);

    run_migrations(&mut conn, &backup_dir).expect("the v2→v3 upgrade applies");

    assert_eq!(
        user_version(&conn),
        3,
        "the spec's custom-target schema is v3"
    );
    let columns: Vec<(String, i64)> = conn
        .prepare("PRAGMA table_info(project_instructions)")
        .expect("table-info query prepares")
        .query_map([], |row| Ok((row.get(1)?, row.get(3)?)))
        .expect("table-info query runs")
        .collect::<Result<Vec<_>, _>>()
        .expect("table-info rows decode");
    assert!(columns.contains(&("instruction_id".to_owned(), 1)));
    assert!(columns.contains(&("variant_id".to_owned(), 0)));
    assert!(columns.contains(&("custom_text".to_owned(), 0)));
    assert_eq!(
        conn.query_row(
            "SELECT project_id, instruction_id, variant_id, custom_text, created_at, updated_at
               FROM project_instructions",
            [],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            )),
        )
        .expect("upgraded assignment reads"),
        (
            "p1".to_owned(),
            "i1".to_owned(),
            "v1".to_owned(),
            None,
            TS.to_owned(),
            TS.to_owned()
        ),
        "the existing assignment is preserved byte-for-byte"
    );
    let foreign_key_violations = conn
        .prepare("PRAGMA foreign_key_check")
        .expect("foreign-key check prepares")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("foreign-key check runs")
        .collect::<Result<Vec<_>, _>>()
        .expect("foreign-key check rows decode");
    assert!(
        foreign_key_violations.is_empty(),
        "no FK violation survives"
    );
}

#[test]
fn v3_rejects_illegal_targets_cross_instruction_variants_and_oversize_custom_text() {
    let temp = temp_dir("rejects");
    let mut conn = open_db(&temp.path().join("db.sqlite3")).expect("database opens");
    run_migrations(&mut conn, &temp.path().join("db-backups")).expect("fresh database migrates");
    seed_project(&conn, "p1");
    seed_project(&conn, "p2");
    seed_project(&conn, "p3");
    seed_project(&conn, "p4");
    seed_instruction(&conn, "i1", "v1");
    seed_instruction(&conn, "i2", "v2");

    for (project_id, instruction_id, variant_id, custom_text) in [
        ("p1", "i1", None, None),
        ("p2", "i1", Some("v1"), Some("both targets")),
        ("p3", "i1", Some("v2"), None),
        ("p4", "i1", None, Some(&"x".repeat(262_145))),
    ] {
        let error = conn
            .execute(
                "INSERT INTO project_instructions (
                    project_id, instruction_id, variant_id, custom_text, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                rusqlite::params![project_id, instruction_id, variant_id, custom_text, TS],
            )
            .expect_err("the spec's illegal assignment target must be refused");
        assert!(
            matches!(error, rusqlite::Error::SqliteFailure(_, _)),
            "the independent database constraint refuses the row: {error}"
        );
    }
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM project_instructions", [], |row| row
            .get::<_, i64>(
            0
        ))
        .expect("assignment count reads"),
        0,
        "every refused target leaves no row"
    );

    conn.execute(
        "INSERT INTO project_instructions (
            project_id, instruction_id, variant_id, custom_text, created_at, updated_at
         ) VALUES ('p1', 'i1', NULL, '', ?1, ?1)",
        [TS],
    )
    .expect("empty custom text is a legal assigned target (D-13/MAIN-007)");
    assert_eq!(
        conn.query_row(
            "SELECT custom_text FROM project_instructions WHERE project_id = 'p1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("custom assignment reads"),
        ""
    );
}
