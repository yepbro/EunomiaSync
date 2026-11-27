//! Behavior tests for schema version 4 (T-356): MAIN-034's extend arm in
//! the `project_instructions` CHECK.
//!
//! The oracle is the current spec §6.2/DB-012 shape, not the migration's
//! own text: an assignment is one of three shapes — a same-instruction
//! variant alone (MAIN-001), custom text alone (MAIN-007's replace mode),
//! or a base variant BESIDE custom text (MAIN-034's extend cell) — and
//! "neither" (both columns NULL) is the one illegal target. Existing v3
//! rows — variant-only and custom-only — must survive the v3→v4 rebuild
//! verbatim; a v3 database cannot hold an extend row (its CHECK rejects
//! the pair), so the upgrade has nothing to widen and only widens the
//! constraint.

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
        "eunomia-store-schema-v4-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("temp directory creates");
    TempDir(path)
}

const V1_SQL: &str = include_str!("../migrations/0001_init.sql");
const V2_SQL: &str = include_str!("../migrations/0002_backup_records_pair_index.sql");
const V3_SQL: &str = include_str!("../migrations/0003_assignment_custom_text.sql");
const TS: &str = "2026-11-27T12:00:00.000Z";

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version must be readable")
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

/// Plants one raw `project_instructions` row with the given target
/// columns (the seeding path every check-bypassing migration test needs).
fn seed_target_row(
    conn: &Connection,
    project_id: &str,
    instruction_id: &str,
    variant_id: Option<&str>,
    custom_text: Option<&str>,
) {
    conn.execute(
        "INSERT INTO project_instructions (
            project_id, instruction_id, variant_id, custom_text, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![project_id, instruction_id, variant_id, custom_text, TS],
    )
    .expect("target seed inserts");
}

/// Builds a v3 database: the three real migrations applied by hand (the
/// way `schema_v3.rs` builds v2) plus one row in each v3-legal shape —
/// a variant-only target and a custom-only target.
fn seed_v3(conn: &Connection) {
    conn.execute_batch("BEGIN")
        .expect("fixture transaction starts");
    conn.execute_batch(V1_SQL).expect("v1 schema applies");
    conn.execute_batch(V2_SQL).expect("v2 index applies");
    conn.execute_batch(V3_SQL).expect("v3 rebuild applies");
    conn.pragma_update(None, "user_version", 3)
        .expect("fixture version sets");
    seed_project(conn, "p1");
    seed_project(conn, "p2");
    seed_project(conn, "p3");
    seed_instruction(conn, "i1", "v1");
    seed_instruction(conn, "i2", "v2");
    seed_target_row(conn, "p1", "i1", Some("v1"), None);
    seed_target_row(conn, "p2", "i1", None, Some("custom text"));
    conn.execute_batch("COMMIT")
        .expect("fixture transaction commits");
    assert_eq!(user_version(conn), 3);
}

#[test]
fn v3_assignments_upgrade_verbatim_to_the_v4_extend_aware_shape() {
    let temp = temp_dir("upgrade");
    let db_path = temp.path().join("db.sqlite3");
    let backup_dir = temp.path().join("db-backups");
    std::fs::create_dir_all(&backup_dir).expect("backup directory creates");
    let mut conn = open_db(&db_path).expect("database opens");
    seed_v3(&conn);

    run_migrations(&mut conn, &backup_dir).expect("the v3→v4 upgrade applies");

    assert_eq!(
        user_version(&conn),
        4,
        "the spec's extend-aware schema is v4"
    );
    // Both v3 shapes survive byte-for-byte: variant-only …
    assert_eq!(
        conn.query_row(
            "SELECT project_id, instruction_id, variant_id, custom_text, created_at, updated_at
               FROM project_instructions WHERE project_id = 'p1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .expect("variant-backed row reads"),
        (
            "p1".to_owned(),
            "i1".to_owned(),
            Some("v1".to_owned()),
            None,
            TS.to_owned(),
            TS.to_owned()
        ),
        "the existing variant target is preserved byte-for-byte"
    );
    // … and custom-only.
    assert_eq!(
        conn.query_row(
            "SELECT project_id, instruction_id, variant_id, custom_text
               FROM project_instructions WHERE project_id = 'p2'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .expect("custom row reads"),
        (
            "p2".to_owned(),
            "i1".to_owned(),
            None,
            Some("custom text".to_owned())
        ),
        "the existing custom target is preserved byte-for-byte"
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
    // The rebuild recreated §6.2's own index names.
    let mut indexes: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'index' AND tbl_name = 'project_instructions' AND name NOT LIKE 'sqlite_%'")
        .expect("index query prepares")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("index query runs")
        .collect::<Result<Vec<_>, _>>()
        .expect("index rows decode");
    indexes.sort();
    assert_eq!(
        indexes,
        vec![
            "ix_project_instructions_instruction_id".to_owned(),
            "ix_project_instructions_variant_id".to_owned()
        ],
        "the two §6.2 extra indexes exist (both sides sorted)"
    );
}

#[test]
fn v4_allows_the_three_target_shapes_and_still_refuses_neither() {
    let temp = temp_dir("shapes");
    let mut conn = open_db(&temp.path().join("db.sqlite3")).expect("database opens");
    run_migrations(&mut conn, &temp.path().join("db-backups")).expect("fresh database migrates");
    seed_project(&conn, "p1");
    seed_project(&conn, "p2");
    seed_project(&conn, "p3");
    seed_project(&conn, "p4");
    seed_instruction(&conn, "i1", "v1");
    seed_instruction(&conn, "i2", "v2");

    // The three legal §6.2 shapes all insert after migration 0004.
    seed_target_row(&conn, "p1", "i1", Some("v1"), None);
    seed_target_row(&conn, "p2", "i1", None, Some("replace text"));
    seed_target_row(&conn, "p3", "i1", Some("v1"), Some("supplement"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM project_instructions", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("assignment count reads"),
        3,
        "all three target shapes are legal"
    );

    // "Neither" stays the one illegal shape, and the pre-existing floors
    // (the composite FK's same-instruction half, DB-006) still refuse.
    for (project_id, instruction_id, variant_id, custom_text) in [
        ("p4", "i1", None, None),
        ("p4", "i1", Some("v2"), Some("cross-instruction base")),
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
        conn.query_row(
            "SELECT COUNT(*) FROM project_instructions WHERE project_id = 'p4'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("refused count reads"),
        0,
        "every refused target leaves no row"
    );
}
