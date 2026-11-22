//! Behavior tests for schema version 1 (spec §6.2; task T-016).
//!
//! These tests drive the schema through the public migration API
//! (`run_migrations`) and assert observable SQLite behavior:
//!
//! * the migration applies to an empty database and creates exactly the
//!   eleven spec tables plus its indexes — the v1 schema of
//!   `0001_init.sql` and the v2 `backup_records` pair-lookup index —
//!   with `PRAGMA user_version = 2` (spec DB-004);
//! * every CHECK, UNIQUE and foreign-key constraint rejects invalid rows,
//!   identified by the SQLite extended result code
//!   (`SQLITE_CONSTRAINT_CHECK`, `_UNIQUE`, `_PRIMARYKEY`,
//!   `_FOREIGNKEY`) — not merely "some error", so a broken statement or a
//!   wrong constraint fails the test the same way a missing one does;
//! * the `ON DELETE` rules of §6.2 hold (RESTRICT blocks the delete,
//!   CASCADE removes the children, SET NULL clears the reference). SQLite
//!   reports a RESTRICT-blocked delete as `SQLITE_CONSTRAINT_TRIGGER` and
//!   a foreign-key INSERT violation as `SQLITE_CONSTRAINT_FOREIGNKEY`, so
//!   the two classes are pinned by their exact extended codes.
//!
//! Foreign-key tests set `PRAGMA foreign_keys=ON` explicitly: the pragma
//! is a per-connection setting, and the production path (`open_db`) sets
//! it on every connection — the tests must not lean on the bundled
//! build's default (DECISIONS.md D-030).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::storage::run_migrations;
use rusqlite::Connection;
use rusqlite::ffi::{
    SQLITE_CONSTRAINT_CHECK as ERR_CHECK, SQLITE_CONSTRAINT_FOREIGNKEY as ERR_FOREIGNKEY,
    SQLITE_CONSTRAINT_NOTNULL as ERR_NOTNULL, SQLITE_CONSTRAINT_PRIMARYKEY as ERR_PRIMARYKEY,
    SQLITE_CONSTRAINT_TRIGGER as ERR_RESTRICT, SQLITE_CONSTRAINT_UNIQUE as ERR_UNIQUE,
};

/// A row expressed as (column, SQL literal) pairs.
type Row = Vec<(&'static str, String)>;

/// A canonical DB-002 timestamp literal.
const TS: &str = "'2026-11-18T10:00:00.000Z'";

/// Quotes a string as a SQL text literal (test values never contain `'`).
fn s(value: &str) -> String {
    format!("'{value}'")
}

/// Returns `row` with `column` set to `value`, appending it when the base
/// row omits the column.
fn set(mut row: Row, column: &'static str, value: String) -> Row {
    if let Some(slot) = row.iter_mut().find(|(name, _)| *name == column) {
        slot.1 = value;
    } else {
        row.push((column, value));
    }
    row
}

/// Builds an `INSERT` statement for `table` from a [`Row`].
fn insert_sql(table: &str, row: &[(&'static str, String)]) -> String {
    let columns: Vec<&str> = row.iter().map(|(name, _)| *name).collect();
    let values: Vec<&str> = row.iter().map(|(_, value)| value.as_str()).collect();
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        columns.join(", "),
        values.join(", ")
    )
}

/// Extracts the SQLite extended result code from a rusqlite error.
fn extended_code(err: &rusqlite::Error) -> Option<i32> {
    match err {
        rusqlite::Error::SqliteFailure(failure, _) => Some(failure.extended_code),
        _ => None,
    }
}

/// Asserts that `result` failed with the expected constraint class.
fn expect_violation(result: rusqlite::Result<usize>, expected: i32, label: &str) {
    let err = match result {
        Ok(count) => panic!(
            "{label}: expected a constraint violation, but the statement succeeded ({count} row)"
        ),
        Err(err) => err,
    };
    let code = extended_code(&err);
    assert_eq!(
        code,
        Some(expected),
        "{label}: expected SQLite extended code {expected}, got {code:?} from {err:?}"
    );
}

/// Runs `sql` and asserts it fails with the given constraint class.
fn rejects(conn: &Connection, sql: &str, expected: i32, label: &str) {
    expect_violation(conn.execute(sql, []), expected, label);
}

/// Runs `sql` and asserts it succeeds.
fn accepts(conn: &Connection, sql: &str, label: &str) {
    conn.execute(sql, [])
        .unwrap_or_else(|err| panic!("{label}: statement should succeed, got {err}"));
}

/// An in-memory database migrated to the current schema version with
/// foreign-key enforcement switched on (mirroring `open_db`). The backup
/// directory argument is never touched: a fresh in-memory database
/// produces no backup copy (D-038), and the path deliberately does not
/// exist.
fn migrated() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to an empty database");
    conn.pragma_update(None, "foreign_keys", true)
        .expect("foreign_keys pragma should be settable");
    conn
}

// --- Base rows: minimal valid rows for each table (everything with a
// --- DEFAULT in the DDL is omitted).

fn project_row(id: &str) -> Row {
    vec![
        ("id", s(id)),
        ("path", s(&format!("/repo/{id}"))),
        ("path_key", s(&format!("/repo/{id}"))),
        ("name", s("Project")),
        ("label", s("PRJ")),
        ("color", s("blue")),
        ("added_via", s("manual")),
        ("sort_order", "1".into()),
        ("scheme", s("A")),
        ("base_name", s("AGENTS.md")),
        ("created_at", TS.into()),
        ("updated_at", TS.into()),
    ]
}

fn exclusion_row(id: &str) -> Row {
    vec![
        ("id", s(id)),
        ("path", s(&format!("/repo/{id}"))),
        ("path_key", s(&format!("/repo/{id}"))),
        ("reason", s("manual")),
        ("created_at", TS.into()),
    ]
}

fn instruction_row(id: &str, position: i64) -> Row {
    vec![
        ("id", s(id)),
        ("kind", s("instruction")),
        ("position", position.to_string()),
        ("title", s("Instruction")),
        ("created_at", TS.into()),
        ("updated_at", TS.into()),
    ]
}

fn heading_row(id: &str, position: i64, level: i64) -> Row {
    vec![
        ("id", s(id)),
        ("kind", s("heading")),
        ("position", position.to_string()),
        ("heading_level", level.to_string()),
        ("title", s("Heading")),
        ("created_at", TS.into()),
        ("updated_at", TS.into()),
    ]
}

fn variant_row(id: &str, instruction_id: &str) -> Row {
    vec![
        ("id", s(id)),
        ("instruction_id", s(instruction_id)),
        ("name", s("Main")),
        ("content", s("text")),
        ("is_main", "1".into()),
        ("created_at", TS.into()),
        ("updated_at", TS.into()),
    ]
}

fn assignment_row(project_id: &str, instruction_id: &str, variant_id: &str) -> Row {
    vec![
        ("project_id", s(project_id)),
        ("instruction_id", s(instruction_id)),
        ("variant_id", s(variant_id)),
        ("created_at", TS.into()),
        ("updated_at", TS.into()),
    ]
}

fn revision_row(id: &str, rev_no: i64) -> Row {
    vec![
        ("id", s(id)),
        ("entity_type", s("instruction")),
        ("entity_id", s("i1")),
        ("rev_no", rev_no.to_string()),
        ("op", s("create")),
        ("snapshot", s("{}")),
        ("snapshot_hash", s("00hash")),
        ("created_at", TS.into()),
    ]
}

fn sync_run_row(id: &str) -> Row {
    vec![
        ("id", s(id)),
        ("started_at", TS.into()),
        ("status", s("running")),
    ]
}

fn run_project_row(sync_run_id: &str, project_id: &str) -> Row {
    vec![
        ("sync_run_id", s(sync_run_id)),
        ("project_id", s(project_id)),
        ("status", s("written")),
    ]
}

fn backup_row(id: &str, project_id: &str, sync_run_id: Option<&str>) -> Row {
    vec![
        ("id", s(id)),
        ("project_id", s(project_id)),
        (
            "sync_run_id",
            sync_run_id.map(s).unwrap_or_else(|| "NULL".into()),
        ),
        ("source_path", s(&format!("/repo/{project_id}/AGENTS.md"))),
        ("backup_path", s(&format!("/data/backups/{id}.bak"))),
        ("size_bytes", "42".into()),
        ("sha256", s("00sha")),
        ("status", s("ok")),
        ("created_at", TS.into()),
    ]
}

fn log_row(action: &str) -> Row {
    vec![
        ("ts", TS.into()),
        ("level", s("info")),
        ("visibility", s("user")),
        ("action", s(action)),
        ("result", s("ok")),
    ]
}

// --- Seed helpers.

fn seed_project(conn: &Connection, id: &str) {
    accepts(
        conn,
        &insert_sql("projects", &project_row(id)),
        &format!("seed project {id}"),
    );
}

fn seed_instruction(conn: &Connection, id: &str, position: i64) {
    accepts(
        conn,
        &insert_sql("library_items", &instruction_row(id, position)),
        &format!("seed instruction {id}"),
    );
}

fn seed_variant(conn: &Connection, id: &str, instruction_id: &str) {
    accepts(
        conn,
        &insert_sql("instruction_variants", &variant_row(id, instruction_id)),
        &format!("seed variant {id}"),
    );
}

fn seed_sync_run(conn: &Connection, id: &str) {
    accepts(
        conn,
        &insert_sql("sync_runs", &sync_run_row(id)),
        &format!("seed sync run {id}"),
    );
}

// --- Schema shape.

#[test]
fn schema_v1_contains_exactly_the_spec_tables() {
    let conn = migrated();

    let mut tables: Vec<String> = Vec::new();
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table'
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    for row in rows {
        tables.push(row.unwrap());
    }

    let mut expected: Vec<&str> = [
        "action_log",
        "backup_records",
        "instruction_variants",
        "library_items",
        "project_exclusions",
        "project_instructions",
        "projects",
        "revisions",
        "settings",
        "sync_run_projects",
        "sync_runs",
    ]
    .into();
    expected.sort_unstable();
    assert_eq!(
        tables, expected,
        "the schema must create exactly the eleven §6.2 tables (and no `meta` stub)"
    );
}

#[test]
fn schema_v1_contains_the_spec_indexes() {
    let conn = migrated();

    // Explicit indexes have a non-NULL `sql`; UNIQUE and PRIMARY KEY
    // constraints create implicit `sqlite_autoindex_*` entries instead.
    // The check pins name, owning table and indexed column(s) — an index
    // silently created on the wrong column is a regression a name-only
    // comparison would miss. The partial-WHERE clause and the COLLATE
    // NOCASE of the two ux_ indexes are pinned behaviorally by the
    // main-variant and variant-name tests. The pair-lookup index is
    // schema version 2's addition (T-256).
    let expected: &[(&str, &str, &[&str])] = &[
        (
            "ix_backup_records_pair_lookup",
            "backup_records",
            &["project_id", "source_path", "status", "created_at"],
        ),
        ("ix_items_position", "library_items", &["position"]),
        ("ix_log_ts", "action_log", &["ts"]),
        (
            "ix_project_instructions_instruction_id",
            "project_instructions",
            &["instruction_id"],
        ),
        (
            "ix_project_instructions_variant_id",
            "project_instructions",
            &["variant_id"],
        ),
        (
            "ux_variant_main",
            "instruction_variants",
            &["instruction_id"],
        ),
        (
            "ux_variant_name",
            "instruction_variants",
            &["instruction_id", "name"],
        ),
    ];

    let mut actual: Vec<(String, String)> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT name, tbl_name FROM sqlite_master
                 WHERE type = 'index' AND sql IS NOT NULL ORDER BY name",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap();
        for row in rows {
            actual.push(row.unwrap());
        }
    }
    let expected_pairs: Vec<(&str, &str)> = expected
        .iter()
        .map(|(name, table, _)| (*name, *table))
        .collect();
    let actual_pairs: Vec<(&str, &str)> = actual
        .iter()
        .map(|(name, table)| (name.as_str(), table.as_str()))
        .collect();
    assert_eq!(
        actual_pairs, expected_pairs,
        "the schema must create the §6.2 indexes, the two project_instructions lookups, \
         and the v2 pair-lookup index, on the right tables"
    );

    for (name, _, columns) in expected {
        let mut indexed: Vec<String> = Vec::new();
        let mut stmt = conn.prepare(&format!("PRAGMA index_info({name})")).unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(2)).unwrap();
        for row in rows {
            indexed.push(row.unwrap());
        }
        let expected_columns: Vec<&str> = columns.to_vec();
        let actual_columns: Vec<&str> = indexed.iter().map(String::as_str).collect();
        assert_eq!(
            actual_columns, expected_columns,
            "index {name} must cover exactly these columns, in order"
        );
    }
}

#[test]
fn schema_v1_declares_the_spec_foreign_keys() {
    let conn = migrated();

    // (table, expected FK count) per §6.2. `foreign_key_list` returns one
    // row per referenced column, so the composite assignment FK counts
    // once through its DISTINCT constraint id.
    let expected: [(&str, usize); 11] = [
        ("action_log", 0),
        ("backup_records", 2),
        ("instruction_variants", 1),
        ("library_items", 0),
        ("project_exclusions", 0),
        ("project_instructions", 2),
        ("projects", 0),
        ("revisions", 0),
        ("settings", 0),
        ("sync_run_projects", 2),
        ("sync_runs", 0),
    ];
    for (table, count) in expected {
        let actual: i64 = conn
            .query_row(
                &format!("SELECT COUNT(DISTINCT id) FROM pragma_foreign_key_list('{table}')"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(actual as usize, count, "FK count of {table}");
    }

    // The composite assignment FK must tie (instruction_id, variant_id) to
    // instruction_variants(instruction_id, id) with ON DELETE RESTRICT.
    let mut links: Vec<(String, String, String, String)> = Vec::new();
    let mut stmt = conn
        .prepare("PRAGMA foreign_key_list(project_instructions)")
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            // Columns: 0 id, 1 seq, 2 table, 3 "from", 4 "to",
            // 5 on_update, 6 on_delete.
            Ok((
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(6)?,
            ))
        })
        .unwrap();
    for row in rows {
        links.push(row.unwrap());
    }
    assert!(links.contains(&(
        "instruction_variants".into(),
        "instruction_id".into(),
        "instruction_id".into(),
        "RESTRICT".into()
    )));
    assert!(links.contains(&(
        "instruction_variants".into(),
        "variant_id".into(),
        "id".into(),
        "RESTRICT".into()
    )));
    assert!(links.contains(&(
        "projects".into(),
        "project_id".into(),
        "id".into(),
        "RESTRICT".into()
    )));
}

#[test]
fn a_fully_migrated_database_records_user_version_two() {
    let conn = migrated();
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 2, "schema version must be exactly 2");
}

#[test]
fn schema_v1_applies_to_a_file_backed_database() {
    // Same hand-rolled temp-dir pattern as storage_open_db.rs: the app
    // layer owns the directory layout, migrations only need a writable
    // file (DECISIONS.md D-027/D-030).
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "eunomia-store-schema-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(&dir).expect("temp directory should be creatable");
    let backup_dir = dir.join("db-backups");
    std::fs::create_dir_all(&backup_dir).expect("temp backup directory should be creatable");
    let path = dir.join("eunomiasync.sqlite3");

    {
        let mut conn = Connection::open(&path).unwrap();
        run_migrations(&mut conn, &backup_dir)
            .expect("migrations should apply to an empty file-backed database");
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_version, 2);
    }

    // The applied version persists in the file: a reopened database is
    // already at version 2 and re-running migrations is a no-op.
    {
        let mut conn = Connection::open(&path).unwrap();
        run_migrations(&mut conn, &backup_dir).expect("re-running migrations must be a no-op");
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(user_version, 2);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 11, "all eleven tables must persist in the file");
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_table_accepts_a_valid_row() {
    let conn = migrated();

    seed_project(&conn, "p1");
    accepts(
        &conn,
        &insert_sql(
            "settings",
            &[
                ("key", s("ui.language")),
                ("value", s("\"en\"")),
                ("updated_at", TS.into()),
            ],
        ),
        "valid settings row",
    );
    accepts(
        &conn,
        &insert_sql("project_exclusions", &exclusion_row("x1")),
        "valid project_exclusions row",
    );
    seed_instruction(&conn, "i1", 1);
    accepts(
        &conn,
        &insert_sql("library_items", &heading_row("h1", 2, 2)),
        "valid heading row",
    );
    seed_variant(&conn, "v1", "i1");
    accepts(
        &conn,
        &insert_sql("project_instructions", &assignment_row("p1", "i1", "v1")),
        "valid project_instructions row",
    );
    accepts(
        &conn,
        &insert_sql("revisions", &revision_row("r1", 1)),
        "valid revisions row",
    );
    seed_sync_run(&conn, "run1");
    accepts(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p1")),
        "valid sync_run_projects row",
    );
    accepts(
        &conn,
        &insert_sql("backup_records", &backup_row("b1", "p1", Some("run1"))),
        "valid backup_records row",
    );
    accepts(
        &conn,
        &insert_sql("action_log", &log_row("project.added")),
        "valid action_log row",
    );

    for (table, expected) in [
        ("settings", 1),
        ("projects", 1),
        ("project_exclusions", 1),
        ("library_items", 2),
        ("instruction_variants", 1),
        ("project_instructions", 1),
        ("revisions", 1),
        ("sync_runs", 1),
        ("sync_run_projects", 1),
        ("backup_records", 1),
        ("action_log", 1),
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or_else(|e| panic!("counting {table} should succeed, got {e}"));
        assert_eq!(count, expected, "row count of {table}");
    }
}

// --- settings.

#[test]
fn every_enum_member_is_accepted() {
    // The reject-side tests prove invalid values fail; this matrix proves
    // no valid member was accidentally left out of a CHECK — a narrowed
    // enum (`state IN ('active')` dropping `archived`, which DB-014 and
    // PRJ-015 depend on) fails here even though every reject test still
    // passes.
    let conn = migrated();
    seed_project(&conn, "p1");
    accepts(
        &conn,
        &insert_sql("project_exclusions", &exclusion_row("x1")),
        "seed exclusion",
    );
    seed_instruction(&conn, "i1", 1);
    seed_variant(&conn, "v1", "i1");
    accepts(
        &conn,
        &insert_sql("revisions", &revision_row("r1", 1)),
        "seed revision",
    );
    seed_sync_run(&conn, "run1");
    accepts(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p1")),
        "seed run outcome",
    );
    accepts(
        &conn,
        &insert_sql("backup_records", &backup_row("b1", "p1", Some("run1"))),
        "seed backup",
    );
    accepts(
        &conn,
        &insert_sql("action_log", &log_row("project.added")),
        "seed log row",
    );

    let cases: &[(&str, &str)] = &[
        (
            "projects.state archived",
            "UPDATE projects SET state = 'archived'",
        ),
        (
            "projects.added_via scan",
            "UPDATE projects SET added_via = 'scan'",
        ),
        ("projects.scheme B", "UPDATE projects SET scheme = 'B'"),
        ("projects.scheme C", "UPDATE projects SET scheme = 'C'"),
        (
            "projects.base_name CLAUDE.md",
            "UPDATE projects SET base_name = 'CLAUDE.md'",
        ),
        (
            "projects.extra_local_enabled 1",
            "UPDATE projects SET extra_local_enabled = 1",
        ),
        (
            "projects.extra_local_position start",
            "UPDATE projects SET extra_local_position = 'start'",
        ),
        (
            "projects.extra_project_enabled 1",
            "UPDATE projects SET extra_project_enabled = 1",
        ),
        (
            "projects.extra_project_position start",
            "UPDATE projects SET extra_project_position = 'start'",
        ),
        (
            "project_exclusions.reason removed_scanned",
            "UPDATE project_exclusions SET reason = 'removed_scanned'",
        ),
        (
            "library_items.state hidden (an instruction may be hidden)",
            "UPDATE library_items SET state = 'hidden' WHERE id = 'i1'",
        ),
        (
            "library_items.kind heading (the row turned into an active heading)",
            "UPDATE library_items SET kind = 'heading', heading_level = 2, state = 'active' WHERE id = 'i1'",
        ),
        (
            "instruction_variants.is_main 0",
            "UPDATE instruction_variants SET is_main = 0",
        ),
        (
            "instruction_variants.state deleted",
            "UPDATE instruction_variants SET state = 'deleted'",
        ),
        (
            "revisions.entity_type variant",
            "UPDATE revisions SET entity_type = 'variant'",
        ),
        ("revisions.op create", "UPDATE revisions SET op = 'create'"),
        ("revisions.op update", "UPDATE revisions SET op = 'update'"),
        ("revisions.op hide", "UPDATE revisions SET op = 'hide'"),
        (
            "revisions.op restore",
            "UPDATE revisions SET op = 'restore'",
        ),
        ("revisions.op delete", "UPDATE revisions SET op = 'delete'"),
        ("revisions.op revert", "UPDATE revisions SET op = 'revert'"),
        (
            "sync_runs.status success",
            "UPDATE sync_runs SET status = 'success'",
        ),
        (
            "sync_runs.status partial",
            "UPDATE sync_runs SET status = 'partial'",
        ),
        (
            "sync_runs.status aborted",
            "UPDATE sync_runs SET status = 'aborted'",
        ),
        (
            "sync_runs.status failed",
            "UPDATE sync_runs SET status = 'failed'",
        ),
        (
            "sync_runs.status interrupted",
            "UPDATE sync_runs SET status = 'interrupted'",
        ),
        (
            "sync_run_projects.status unchanged",
            "UPDATE sync_run_projects SET status = 'unchanged'",
        ),
        (
            "sync_run_projects.status skipped",
            "UPDATE sync_run_projects SET status = 'skipped'",
        ),
        (
            "sync_run_projects.status failed",
            "UPDATE sync_run_projects SET status = 'failed'",
        ),
        (
            "sync_run_projects.status not_started",
            "UPDATE sync_run_projects SET status = 'not_started'",
        ),
        (
            "backup_records.status pruned",
            "UPDATE backup_records SET status = 'pruned'",
        ),
        (
            "backup_records.status missing",
            "UPDATE backup_records SET status = 'missing'",
        ),
        (
            "backup_records.status discarded",
            "UPDATE backup_records SET status = 'discarded'",
        ),
        (
            "action_log.level warn",
            "UPDATE action_log SET level = 'warn'",
        ),
        (
            "action_log.level error",
            "UPDATE action_log SET level = 'error'",
        ),
        (
            "action_log.visibility diagnostic",
            "UPDATE action_log SET visibility = 'diagnostic'",
        ),
        (
            "action_log.result error",
            "UPDATE action_log SET result = 'error'",
        ),
        (
            "action_log.result skipped",
            "UPDATE action_log SET result = 'skipped'",
        ),
    ];
    for (label, sql) in cases {
        accepts(&conn, sql, label);
    }
}

#[test]
fn settings_reject_a_duplicate_key() {
    let conn = migrated();
    accepts(
        &conn,
        &insert_sql(
            "settings",
            &[
                ("key", s("ui.language")),
                ("value", s("\"en\"")),
                ("updated_at", TS.into()),
            ],
        ),
        "first settings row",
    );
    rejects(
        &conn,
        &insert_sql(
            "settings",
            &[
                ("key", s("ui.language")),
                ("value", s("\"ru\"")),
                ("updated_at", TS.into()),
            ],
        ),
        ERR_PRIMARYKEY,
        "duplicate settings key",
    );
}

// --- projects: CHECK constraints.

#[test]
fn projects_reject_rows_violating_column_checks() {
    let conn = migrated();
    let cases: Vec<(&str, Row)> = vec![
        ("empty name", set(project_row("p2"), "name", s("   "))),
        (
            "name over 100 characters",
            set(project_row("p2"), "name", s(&"x".repeat(101))),
        ),
        ("empty label", set(project_row("p2"), "label", s(""))),
        (
            "label over 6 characters",
            set(project_row("p2"), "label", s("ABCDEFG")),
        ),
        (
            "description over 1000 characters",
            set(project_row("p2"), "description", s(&"d".repeat(1001))),
        ),
        (
            "state outside the enum",
            set(project_row("p2"), "state", s("paused")),
        ),
        (
            "added_via outside the enum",
            set(project_row("p2"), "added_via", s("import")),
        ),
        (
            "scheme outside the enum",
            set(project_row("p2"), "scheme", s("D")),
        ),
        (
            "base_name outside the enum",
            set(project_row("p2"), "base_name", s("README.md")),
        ),
        (
            "extra_local_enabled of 2",
            set(project_row("p2"), "extra_local_enabled", "2".into()),
        ),
        (
            "extra_local_enabled of -1",
            set(project_row("p2"), "extra_local_enabled", "-1".into()),
        ),
        (
            "extra_local_position outside the enum",
            set(project_row("p2"), "extra_local_position", s("middle")),
        ),
        (
            "extra_project_enabled of 2",
            set(project_row("p2"), "extra_project_enabled", "2".into()),
        ),
        (
            "extra_project_position outside the enum",
            set(project_row("p2"), "extra_project_position", s("middle")),
        ),
    ];
    for (label, row) in cases {
        rejects(
            &conn,
            &insert_sql("projects", &row),
            ERR_CHECK,
            &format!("projects: {label}"),
        );
    }
}

#[test]
fn projects_accept_boundary_length_values() {
    let conn = migrated();
    let cases: Vec<(&str, Row)> = vec![
        (
            "name of exactly 100 characters",
            set(project_row("p1"), "name", s(&"x".repeat(100))),
        ),
        (
            "label of exactly 6 characters",
            set(project_row("p2"), "label", s("ABCDEF")),
        ),
        (
            "description of exactly 1000 characters",
            set(project_row("p3"), "description", s(&"d".repeat(1000))),
        ),
    ];
    for (label, row) in cases {
        accepts(&conn, &insert_sql("projects", &row), label);
    }
}

#[test]
fn projects_reject_duplicate_ids_and_path_keys() {
    let conn = migrated();
    seed_project(&conn, "p1");

    rejects(
        &conn,
        &insert_sql(
            "projects",
            &set(project_row("p1"), "path_key", s("/other/p1")),
        ),
        ERR_PRIMARYKEY,
        "projects: duplicate id (path_key kept distinct so only the PK conflicts)",
    );
    rejects(
        &conn,
        &insert_sql(
            "projects",
            &set(project_row("p2"), "path_key", s("/repo/p1")),
        ),
        ERR_UNIQUE,
        "projects: duplicate path_key",
    );
}

// --- project_exclusions.

#[test]
fn project_exclusions_reject_invalid_reason_and_duplicates() {
    let conn = migrated();
    accepts(
        &conn,
        &insert_sql("project_exclusions", &exclusion_row("x1")),
        "first exclusion",
    );

    rejects(
        &conn,
        &insert_sql(
            "project_exclusions",
            &set(exclusion_row("x2"), "reason", s("other")),
        ),
        ERR_CHECK,
        "project_exclusions: reason outside the enum",
    );
    rejects(
        &conn,
        &insert_sql(
            "project_exclusions",
            &set(exclusion_row("x1"), "path_key", s("/other/x1")),
        ),
        ERR_PRIMARYKEY,
        "project_exclusions: duplicate id (path_key kept distinct)",
    );
    rejects(
        &conn,
        &insert_sql(
            "project_exclusions",
            &set(exclusion_row("x2"), "path_key", s("/repo/x1")),
        ),
        ERR_UNIQUE,
        "project_exclusions: duplicate path_key",
    );
}

// --- library_items.

#[test]
fn library_items_reject_rows_violating_checks() {
    let conn = migrated();
    let cases: Vec<(&str, Row)> = vec![
        (
            "kind outside the enum",
            set(instruction_row("i9", 2), "kind", s("note")),
        ),
        ("position below 1", instruction_row("i9", 0)),
        (
            "empty title",
            set(instruction_row("i9", 2), "title", s("  ")),
        ),
        (
            "title over 200 characters",
            set(instruction_row("i9", 2), "title", s(&"t".repeat(201))),
        ),
        (
            "state outside the enum",
            set(instruction_row("i9", 2), "state", s("draft")),
        ),
        (
            "instruction with a heading level",
            set(instruction_row("i9", 2), "heading_level", "3".into()),
        ),
        ("heading with level 1", heading_row("i9", 2, 1)),
        ("heading with level 7", heading_row("i9", 2, 7)),
        (
            "heading without a level",
            set(heading_row("i9", 2, 2), "heading_level", "NULL".into()),
        ),
        (
            "hidden heading",
            set(heading_row("i9", 2, 2), "state", s("hidden")),
        ),
    ];
    for (label, row) in cases {
        rejects(
            &conn,
            &insert_sql("library_items", &row),
            ERR_CHECK,
            &format!("library_items: {label}"),
        );
    }
}

#[test]
fn library_items_accept_boundary_values() {
    let conn = migrated();
    accepts(
        &conn,
        &insert_sql("library_items", &instruction_row("i1", 1)),
        "instruction at position 1",
    );
    accepts(
        &conn,
        &insert_sql(
            "library_items",
            &set(instruction_row("i2", 2), "title", s(&"t".repeat(200))),
        ),
        "title of exactly 200 characters",
    );
    accepts(
        &conn,
        &insert_sql("library_items", &heading_row("h1", 3, 2)),
        "heading at level 2",
    );
    accepts(
        &conn,
        &insert_sql("library_items", &heading_row("h2", 4, 6)),
        "heading at level 6",
    );
}

#[test]
fn library_items_reject_a_duplicate_id() {
    let conn = migrated();
    seed_instruction(&conn, "i1", 1);
    rejects(
        &conn,
        &insert_sql("library_items", &instruction_row("i1", 2)),
        ERR_PRIMARYKEY,
        "library_items: duplicate id",
    );
}

// --- instruction_variants.

#[test]
fn instruction_variants_reject_rows_violating_checks() {
    let conn = migrated();
    seed_instruction(&conn, "i1", 1);

    let cases: Vec<(&str, Row)> = vec![
        ("empty name", set(variant_row("v9", "i1"), "name", s(" "))),
        (
            "name over 100 characters",
            set(variant_row("v9", "i1"), "name", s(&"n".repeat(101))),
        ),
        (
            "is_main of 2",
            set(variant_row("v9", "i1"), "is_main", "2".into()),
        ),
        (
            "state outside the enum",
            set(variant_row("v9", "i1"), "state", s("archived")),
        ),
    ];
    for (label, row) in cases {
        rejects(
            &conn,
            &insert_sql("instruction_variants", &row),
            ERR_CHECK,
            &format!("instruction_variants: {label}"),
        );
    }

    // The unknown-instruction case must be an FK violation, not a CHECK.
    rejects(
        &conn,
        &insert_sql("instruction_variants", &variant_row("v9", "nobody")),
        ERR_FOREIGNKEY,
        "instruction_variants: FK to a missing library item",
    );
}

#[test]
fn instruction_variants_content_limit_is_262144_characters() {
    // Spec DB-006: variant text is up to 262144 characters.
    let conn = migrated();
    seed_instruction(&conn, "i1", 1);

    accepts(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(variant_row("v1", "i1"), "content", s(&"c".repeat(262144))),
        ),
        "content of exactly 262144 characters",
    );
    rejects(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(
                    set(variant_row("v2", "i1"), "name", s("Over")),
                    "is_main",
                    "0".into(),
                ),
                "content",
                s(&"c".repeat(262145)),
            ),
        ),
        ERR_CHECK,
        "content of 262145 characters",
    );
    // The empty variant is valid (spec §6.9: "empty is allowed").
    accepts(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(variant_row("v3", "i1"), "name", s("Empty")),
                "is_main",
                "0".into(),
            ),
        ),
        "empty content",
    );
    // `length()` on TEXT counts characters, not bytes: 262144 two-byte
    // characters (524288 bytes) are within the limit, because §6.9 counts
    // characters. A byte-semantics regression (CAST to BLOB) would reject
    // this row.
    accepts(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(
                    set(variant_row("v4", "i1"), "name", s("Wide")),
                    "is_main",
                    "0".into(),
                ),
                "content",
                s(&"ж".repeat(262144)),
            ),
        ),
        "content of 262144 two-byte characters (524288 bytes)",
    );
}

#[test]
fn instruction_variants_reject_a_duplicate_id() {
    let conn = migrated();
    seed_instruction(&conn, "i1", 1);
    seed_variant(&conn, "v1", "i1");
    rejects(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(variant_row("v1", "i1"), "name", s("Other")),
                "is_main",
                "0".into(),
            ),
        ),
        // The row duplicates both the id (PK) and the
        // (instruction_id, id) pair; SQLite checks the composite UNIQUE
        // index before the PK autoindex and reports UNIQUE (verified
        // against the bundled build).
        ERR_UNIQUE,
        "instruction_variants: duplicate id within one instruction",
    );

    // Across two different instructions the composite UNIQUE cannot fire,
    // so this is the case that exercises the PRIMARY KEY itself.
    seed_instruction(&conn, "i2", 2);
    rejects(
        &conn,
        &insert_sql("instruction_variants", &variant_row("v1", "i2")),
        ERR_PRIMARYKEY,
        "instruction_variants: duplicate id under another instruction (PK only)",
    );
}

#[test]
fn only_one_active_main_variant_is_allowed_per_instruction() {
    let conn = migrated();
    seed_instruction(&conn, "i1", 1);
    seed_variant(&conn, "v1", "i1");

    rejects(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(variant_row("v2", "i1"), "name", s("Draft")),
        ),
        ERR_UNIQUE,
        "second active main variant of one instruction",
    );

    // A soft-deleted main variant leaves the partial index, so a new
    // active main variant may take over (DB-010 counts active variants).
    accepts(
        &conn,
        "UPDATE instruction_variants SET state = 'deleted' WHERE id = 'v1'",
        "soft-delete the main variant",
    );
    accepts(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(variant_row("v2", "i1"), "name", s("New main")),
        ),
        "second main variant while the first is deleted",
    );

    // Independent instructions keep independent main variants.
    seed_instruction(&conn, "i2", 2);
    accepts(
        &conn,
        &insert_sql("instruction_variants", &variant_row("v3", "i2")),
        "main variant of another instruction",
    );
}

#[test]
fn variant_names_are_unique_per_instruction_while_active() {
    let conn = migrated();
    seed_instruction(&conn, "i1", 1);
    seed_variant(&conn, "v1", "i1"); // name "Main"

    rejects(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(variant_row("v2", "i1"), "is_main", "0".into()),
                "name",
                s("Main"),
            ),
        ),
        ERR_UNIQUE,
        "duplicate active variant name",
    );
    rejects(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(variant_row("v3", "i1"), "is_main", "0".into()),
                "name",
                s("mAIN"),
            ),
        ),
        ERR_UNIQUE,
        "duplicate active variant name is caught case-insensitively",
    );
    // A differently named second variant is fine...
    accepts(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(variant_row("v2", "i1"), "is_main", "0".into()),
                "name",
                s("Draft"),
            ),
        ),
        "second variant with a distinct name",
    );
    // ...and renaming into a collision is caught on UPDATE as well (a row
    // never collides with itself, so the collision must be with v1).
    rejects(
        &conn,
        "UPDATE instruction_variants SET name = 'main' WHERE id = 'v2'",
        ERR_UNIQUE,
        "rename into a case-insensitive collision",
    );

    // A deleted variant frees the name; another instruction may reuse it.
    accepts(
        &conn,
        "UPDATE instruction_variants SET state = 'deleted' WHERE id = 'v1'",
        "soft-delete the variant",
    );
    accepts(
        &conn,
        &insert_sql(
            "instruction_variants",
            &set(
                set(variant_row("v4", "i1"), "is_main", "0".into()),
                "name",
                s("Main"),
            ),
        ),
        "name of a deleted variant is free again",
    );
    seed_instruction(&conn, "i2", 2);
    accepts(
        &conn,
        &insert_sql("instruction_variants", &variant_row("v5", "i2")),
        "same name under another instruction",
    );
}

// --- project_instructions.

#[test]
fn project_instructions_reject_rows_violating_foreign_keys() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_instruction(&conn, "i1", 1);
    seed_instruction(&conn, "i2", 2);
    seed_variant(&conn, "v1", "i1");
    seed_variant(&conn, "v2", "i2");

    rejects(
        &conn,
        &insert_sql(
            "project_instructions",
            &assignment_row("nobody", "i1", "v1"),
        ),
        ERR_FOREIGNKEY,
        "assignment with an unknown project",
    );
    rejects(
        &conn,
        &insert_sql(
            "project_instructions",
            &assignment_row("p1", "i1", "nobody"),
        ),
        ERR_FOREIGNKEY,
        "assignment with an unknown variant",
    );
    rejects(
        &conn,
        &insert_sql("project_instructions", &assignment_row("p1", "i1", "v2")),
        ERR_FOREIGNKEY,
        "assignment pairing an instruction with another instruction's variant",
    );
}

#[test]
fn project_instructions_reject_duplicate_assignments() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_instruction(&conn, "i1", 1);
    seed_variant(&conn, "v1", "i1");
    accepts(
        &conn,
        &insert_sql("project_instructions", &assignment_row("p1", "i1", "v1")),
        "first assignment",
    );

    rejects(
        &conn,
        &insert_sql("project_instructions", &assignment_row("p1", "i1", "v1")),
        ERR_PRIMARYKEY,
        "duplicate (project_id, instruction_id) assignment",
    );
}

// --- revisions.

#[test]
fn revisions_reject_rows_violating_checks() {
    let conn = migrated();

    rejects(
        &conn,
        &insert_sql(
            "revisions",
            &set(revision_row("r1", 1), "entity_type", s("heading")),
        ),
        ERR_CHECK,
        "revisions: headings have no history (entity_type outside the enum)",
    );
    rejects(
        &conn,
        &insert_sql("revisions", &set(revision_row("r1", 1), "op", s("rename"))),
        ERR_CHECK,
        "revisions: op outside the enum",
    );
}

#[test]
fn revisions_reject_duplicate_ids_and_revision_numbers() {
    let conn = migrated();
    accepts(
        &conn,
        &insert_sql("revisions", &revision_row("r1", 1)),
        "first revision",
    );

    rejects(
        &conn,
        &insert_sql(
            "revisions",
            &set(revision_row("r1", 2), "rev_no", "2".into()),
        ),
        ERR_PRIMARYKEY,
        "revisions: duplicate id",
    );
    rejects(
        &conn,
        &insert_sql(
            "revisions",
            &set(revision_row("r2", 1), "snapshot_hash", s("01hash")),
        ),
        ERR_UNIQUE,
        "revisions: duplicate (entity_type, entity_id, rev_no)",
    );
    // The next revision number of the same entity is fine (ALG-071).
    accepts(
        &conn,
        &insert_sql("revisions", &set(revision_row("r2", 2), "op", s("update"))),
        "second revision of the entity",
    );
}

// --- sync_runs / sync_run_projects.

#[test]
fn sync_runs_reject_an_invalid_status() {
    let conn = migrated();
    rejects(
        &conn,
        &insert_sql(
            "sync_runs",
            &set(sync_run_row("run1"), "status", s("pending")),
        ),
        ERR_CHECK,
        "sync_runs: status outside the enum",
    );
    accepts(
        &conn,
        &insert_sql("sync_runs", &sync_run_row("run1")),
        "first sync run",
    );
    rejects(
        &conn,
        &insert_sql("sync_runs", &sync_run_row("run1")),
        ERR_PRIMARYKEY,
        "sync_runs: duplicate id",
    );
}

#[test]
fn sync_run_projects_reject_invalid_rows() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_sync_run(&conn, "run1");

    rejects(
        &conn,
        &insert_sql(
            "sync_run_projects",
            &set(run_project_row("run1", "p1"), "status", s("pending")),
        ),
        ERR_CHECK,
        "sync_run_projects: status outside the enum",
    );
    rejects(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("norun", "p1")),
        ERR_FOREIGNKEY,
        "sync_run_projects: unknown sync run",
    );
    rejects(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "noproject")),
        ERR_FOREIGNKEY,
        "sync_run_projects: unknown project",
    );
    accepts(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p1")),
        "first per-project outcome",
    );
    rejects(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p1")),
        ERR_PRIMARYKEY,
        "sync_run_projects: duplicate (sync_run_id, project_id)",
    );
}

// --- backup_records.

#[test]
fn backup_records_reject_invalid_rows() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_sync_run(&conn, "run1");

    rejects(
        &conn,
        &insert_sql(
            "backup_records",
            &set(backup_row("b9", "p1", None), "status", s("copied")),
        ),
        ERR_CHECK,
        "backup_records: status outside the enum",
    );
    rejects(
        &conn,
        &insert_sql("backup_records", &backup_row("b9", "noproject", None)),
        ERR_FOREIGNKEY,
        "backup_records: unknown project",
    );
    rejects(
        &conn,
        &insert_sql("backup_records", &backup_row("b9", "p1", Some("norun"))),
        ERR_FOREIGNKEY,
        "backup_records: unknown sync run",
    );

    accepts(
        &conn,
        &insert_sql("backup_records", &backup_row("b1", "p1", Some("run1"))),
        "first backup record",
    );
    rejects(
        &conn,
        &insert_sql(
            "backup_records",
            &set(
                backup_row("b1", "p1", None),
                "backup_path",
                s("/data/backups/other.bak"),
            ),
        ),
        ERR_PRIMARYKEY,
        "backup_records: duplicate id (backup_path kept distinct)",
    );
    rejects(
        &conn,
        &insert_sql(
            "backup_records",
            &set(
                backup_row("b2", "p1", None),
                "backup_path",
                s("/data/backups/b1.bak"),
            ),
        ),
        ERR_UNIQUE,
        "backup_records: duplicate backup_path",
    );
}

// --- action_log.

#[test]
fn action_log_reject_invalid_rows() {
    let conn = migrated();

    for (label, column, value) in [
        ("level outside the enum", "level", s("debug")),
        ("visibility outside the enum", "visibility", s("public")),
        ("result outside the enum", "result", s("partial")),
    ] {
        rejects(
            &conn,
            &insert_sql("action_log", &set(log_row("project.added"), column, value)),
            ERR_CHECK,
            &format!("action_log: {label}"),
        );
    }
    accepts(
        &conn,
        &insert_sql("action_log", &log_row("project.added")),
        "valid log row",
    );
    // An explicit duplicate id is the insert-reachable PRIMARY KEY
    // violation for this table (the autoincrement counter starts at 1).
    rejects(
        &conn,
        &insert_sql(
            "action_log",
            &set(log_row("project.archived"), "id", "1".into()),
        ),
        ERR_PRIMARYKEY,
        "action_log: duplicate explicit id",
    );
}

#[test]
fn action_log_ids_are_autoincrement() {
    let conn = migrated();
    accepts(
        &conn,
        &insert_sql("action_log", &log_row("project.added")),
        "first log row",
    );
    accepts(
        &conn,
        &insert_sql("action_log", &log_row("project.archived")),
        "second log row",
    );

    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT id FROM action_log ORDER BY id")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };
    assert_eq!(
        ids,
        vec![1, 2],
        "autoincrement ids start at 1 and grow by 1"
    );

    // AUTOINCREMENT never reuses the id of a deleted row (a plain
    // INTEGER PRIMARY KEY would hand out 2 again here), and its
    // high-water mark is visible in sqlite_sequence.
    accepts(&conn, "DELETE FROM action_log WHERE id = 2", "delete a row");
    accepts(
        &conn,
        &insert_sql("action_log", &log_row("settings.changed")),
        "row after a delete",
    );
    let id: i64 = conn
        .query_row("SELECT MAX(id) FROM action_log", [], |row| row.get(0))
        .unwrap();
    assert_eq!(id, 3, "the deleted id must not be reused");

    let seq: i64 = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'action_log'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(seq, 3, "sqlite_sequence tracks the AUTOINCREMENT counter");
}

// --- NOT NULL (representative sample).

#[test]
fn not_null_columns_reject_null_values() {
    let conn = migrated();

    // NOT NULL is not part of the acceptance matrix, but the sample pins
    // the constraint class for the schema's NOT NULL columns generally.
    rejects(
        &conn,
        &insert_sql("projects", &set(project_row("p1"), "path", "NULL".into())),
        ERR_NOTNULL,
        "projects: NULL path",
    );
}

// --- UPDATE-path enforcement (SQLite checks constraints on UPDATE too).

#[test]
fn constraints_hold_on_updates() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_project(&conn, "p2");
    seed_instruction(&conn, "i1", 1);
    seed_variant(&conn, "v1", "i1");

    rejects(
        &conn,
        "UPDATE projects SET state = 'paused' WHERE id = 'p1'",
        ERR_CHECK,
        "update into an invalid enum value",
    );
    rejects(
        &conn,
        "UPDATE projects SET path_key = '/repo/p2' WHERE id = 'p1'",
        ERR_UNIQUE,
        "update into a duplicate path_key",
    );
    // Updating a parent key that children still reference is an FK
    // violation (the default ON UPDATE action is NO ACTION).
    rejects(
        &conn,
        "UPDATE library_items SET id = 'i-renamed' WHERE id = 'i1'",
        ERR_FOREIGNKEY,
        "update of a referenced parent key",
    );
}

// --- ON DELETE rules of §6.2.

#[test]
fn deleting_a_referenced_parent_is_restricted() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_instruction(&conn, "i1", 1);
    seed_variant(&conn, "v1", "i1");
    accepts(
        &conn,
        &insert_sql("project_instructions", &assignment_row("p1", "i1", "v1")),
        "assignment referencing the project and the variant",
    );
    seed_sync_run(&conn, "run1");
    accepts(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p1")),
        "run outcome referencing the project",
    );
    accepts(
        &conn,
        &insert_sql("backup_records", &backup_row("b1", "p1", Some("run1"))),
        "backup referencing the project",
    );

    rejects(
        &conn,
        "DELETE FROM projects WHERE id = 'p1'",
        ERR_RESTRICT,
        "delete of a project with an assignment (RESTRICT)",
    );
    rejects(
        &conn,
        "DELETE FROM library_items WHERE id = 'i1'",
        ERR_RESTRICT,
        "delete of an instruction with variants (RESTRICT)",
    );
    rejects(
        &conn,
        "DELETE FROM instruction_variants WHERE id = 'v1'",
        ERR_RESTRICT,
        "delete of an assigned variant (RESTRICT)",
    );

    // Removing the child rows lifts the restriction.
    accepts(
        &conn,
        "DELETE FROM project_instructions",
        "remove the assignments",
    );
    accepts(
        &conn,
        "DELETE FROM instruction_variants WHERE id = 'v1'",
        "variant delete after its assignment is gone",
    );
    accepts(
        &conn,
        "DELETE FROM library_items WHERE id = 'i1'",
        "instruction delete after its variants are gone",
    );
    accepts(
        &conn,
        "DELETE FROM sync_run_projects",
        "remove the run outcomes",
    );
    accepts(
        &conn,
        "DELETE FROM backup_records",
        "remove the backup records",
    );
    accepts(
        &conn,
        "DELETE FROM projects WHERE id = 'p1'",
        "project delete is unrestricted once children are gone",
    );
}

#[test]
fn deleting_a_project_with_only_outcomes_or_backups_is_restricted() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_sync_run(&conn, "run1");
    accepts(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p1")),
        "run outcome",
    );
    rejects(
        &conn,
        "DELETE FROM projects WHERE id = 'p1'",
        ERR_RESTRICT,
        "delete of a project with a run outcome (RESTRICT)",
    );

    let conn = migrated();
    seed_project(&conn, "p1");
    seed_sync_run(&conn, "run1");
    accepts(
        &conn,
        &insert_sql("backup_records", &backup_row("b1", "p1", Some("run1"))),
        "backup",
    );
    rejects(
        &conn,
        "DELETE FROM projects WHERE id = 'p1'",
        ERR_RESTRICT,
        "delete of a project with a backup record (RESTRICT)",
    );
}

#[test]
fn deleting_a_sync_run_cascades_to_its_project_outcomes() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_project(&conn, "p2");
    seed_sync_run(&conn, "run1");
    accepts(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p1")),
        "outcome 1",
    );
    accepts(
        &conn,
        &insert_sql("sync_run_projects", &run_project_row("run1", "p2")),
        "outcome 2",
    );

    accepts(
        &conn,
        "DELETE FROM sync_runs WHERE id = 'run1'",
        "delete of the run",
    );
    let outcomes: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_run_projects", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(outcomes, 0, "the run's outcomes must be cascaded away");
    let projects: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(projects, 2, "the projects themselves must stay");
}

#[test]
fn deleting_a_sync_run_nulls_backup_references() {
    let conn = migrated();
    seed_project(&conn, "p1");
    seed_sync_run(&conn, "run1");
    accepts(
        &conn,
        &insert_sql("backup_records", &backup_row("b1", "p1", Some("run1"))),
        "backup with a run reference",
    );

    accepts(
        &conn,
        "DELETE FROM sync_runs WHERE id = 'run1'",
        "delete of the run",
    );
    let (count, null_runs): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COUNT(sync_run_id) FROM backup_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1, "the backup record must survive");
    assert_eq!(
        null_runs, 0,
        "COUNT(sync_run_id) counts non-NULL values, so it must be 0 after SET NULL"
    );
}
