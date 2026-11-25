//! Behavior tests for schema version 2 (task T-256): the v1→v2 upgrade
//! path and the `backup_records` per-pair lookup index.
//!
//! Version 2 is a single index, `ix_backup_records_pair_lookup` over
//! `(project_id, source_path, status, created_at)` — the owner-approved
//! column set (the card's decision; D-132's round-1 follow-up list):
//! the per-pair `last_checked` lookup of `backup_records` (BKP-010's
//! dedup input, §9.4 phase 2) issues one query per eligible project per
//! sync run, and without the index every one of them scanned the whole
//! `backup_records` table.
//!
//! Pinned behavior:
//!
//! * a version-1 database with rows upgrades through version 2 to the
//!   current version 3 and keeps every seeded row verbatim — v2 adds the
//!   index and v3 rebuilds only assignment rows;
//! * a fresh database reaches `PRAGMA user_version = 3` with the v2 index
//!   present, and `PRAGMA index_info` pins its columns to exactly the
//!   approved set, in order (an independent oracle: the task card's
//!   decision, not whatever the SQL happens to say). The same shape is
//!   pinned from the other side by schema_v1.rs's
//!   `schema_v1_contains_the_spec_indexes` — a deliberate second list
//!   (the owner's 2026-11-24 ruling, T-273): the two serve different
//!   contracts, and a change to the pair index edits both files;
//! * `EXPLAIN QUERY PLAN` for the exact `last_checked` statement
//!   searches through the index — no `SCAN backup_records` (this is
//!   the test that fails without the index; see the mirror note on
//!   [`LAST_CHECKED_SQL`]);
//! * the upgrade run writes D-038 pre-migration copies of both source
//!   states — `pre-migration-v1-<UTC>.sqlite3` and
//!   `pre-migration-v2-<UTC>.sqlite3` — each valid and intact.
//!
//! The version-1 fixture ([`seed_v1_schema`]) is the real thing: it
//! executes the actual `migrations/0001_init.sql` and sets
//! `PRAGMA user_version = 1` in one transaction — the same two steps
//! rusqlite_migration's per-step transaction performs (`goto_up`:
//! `execute_batch` of the SQL, then `PRAGMA user_version` inside the
//! transaction; rusqlite_migration 2.7, src/lib.rs 618–653) — so the
//! upgrade these tests drive starts from a state byte-equivalent in
//! schema and version to what a shipped version-1 app leaves behind.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::backup_records::{CheckedCopy, last_checked};
use eunomia_store::storage::{open_db, run_migrations};
use rusqlite::Connection;

/// The index this schema version adds, in the repo's `ix_<table>_<purpose>`
/// naming convention (`ix_items_position`, `ix_log_ts`).
const PAIR_INDEX: &str = "ix_backup_records_pair_lookup";

/// A canonical DB-002 timestamp literal (test data only).
const TS: &str = "'2026-11-21T10:00:00.000Z'";

/// The exact statement `backup_records::last_checked` runs
/// (crates/eunomia-store/src/backup_records.rs), mirrored here because
/// `EXPLAIN QUERY PLAN` needs the statement text and the repository
/// module keeps it private (the D-132 `db002_stamp` mirror posture).
/// The plan test cross-checks the mirror against the production
/// function on the same fixture below: a drift that changes the
/// query's answer on this data — e.g. dropping the `status = 'ok'`
/// predicate, which the fixture exposes (the pair's terminal row is
/// its NEWEST stamp) — fails the suite. Drifts that keep the answer
/// on this data (a purely syntactic rewrite, or dropping a term this
/// fixture cannot discriminate, such as the `created_at` ordering)
/// are the mirror's residual; the same-stamp `id` tiebreak is pinned
/// by `tests/backup_records.rs` and, end to end, by the upgrade
/// test's `last_checked` literal.
const LAST_CHECKED_SQL: &str = "SELECT sha256, backup_path FROM backup_records
  WHERE project_id = ?1 AND source_path = ?2 AND status = 'ok'
  ORDER BY created_at DESC, id DESC
  LIMIT 1";

/// The real version-1 schema, the same file `MIGRATIONS` embeds.
const V1_SQL: &str = include_str!("../migrations/0001_init.sql");

/// A temporary directory removed when the guard is dropped (the
/// `tests/backup_records.rs` pattern).
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a `db/` subdirectory
/// for the database file and a `db-backups/` subdirectory for the
/// migration safety copies, mimicking the app-side data-directory
/// layout (the layout is the app layer's, D-027; the store crate
/// receives both paths as parameters and creates neither). Returns the
/// two subdirectory paths and the teardown guard.
fn temp_data_dir(label: &str) -> (PathBuf, PathBuf, TempDir) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-schema-v2-{label}-{}-{nanos}",
        std::process::id(),
    ));
    let db = dir.join("db");
    let backup_dir = dir.join("db-backups");
    std::fs::create_dir_all(&db).expect("temp db directory should be creatable");
    std::fs::create_dir_all(&backup_dir).expect("temp backup directory should be creatable");
    (db, backup_dir, TempDir(dir))
}

/// Reads `PRAGMA user_version` from a connection.
fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version should be readable")
}

/// Builds an authentic version-1 database at `db_path`: the real
/// `0001_init.sql` executed and `PRAGMA user_version` set to 1 inside
/// one transaction — exactly what rusqlite_migration's per-step
/// transaction does for the first migration (see the module docs).
fn seed_v1_schema(db_path: &Path) {
    let conn = Connection::open(db_path).expect("the v1 database should open");
    conn.execute_batch("BEGIN")
        .expect("the fixture transaction should start");
    conn.execute_batch(V1_SQL)
        .expect("the real 0001_init.sql should apply");
    conn.pragma_update(None, "user_version", 1)
        .expect("user_version should be settable");
    conn.execute_batch("COMMIT")
        .expect("the fixture transaction should commit");
    assert_eq!(user_version(&conn), 1, "the fixture must be at version 1");
}

/// The id of the seeded project (the `project_id` of every seeded
/// record and lookup).
const PROJECT_ID: &str = "p-00000000-0000-7000-8000-000000000001";

/// The seeded pair's source path (the §9.4 phase-1 resolved spelling).
const SOURCE_PATH: &str = "/repos/alpha/AGENTS.md";

/// Seeds version-1 user data the upgrade must preserve: one project
/// (plus a second for pair scoping), one settings row, and five
/// `backup_records` rows — three `ok` copies of the pair under test
/// (two sharing one stamp, so the D-122 id tiebreak decides "newest",
/// plus one older), one terminal row seeded NEWER than every `ok` (so
/// a lookup that forgets the `ok` predicate answers wrong — the
/// property the mirror cross-check stands on), and one record of
/// another project. Plain SQL with literal values: the rows are data,
/// and the assertions below compare against the same literals (the
/// `tests/schema_v1.rs` fixture posture).
fn seed_v1_rows(conn: &Connection) {
    conn.execute_batch(&format!(
        "INSERT INTO projects (
             id, path, path_key, name, label, color, added_via,
             sort_order, scheme, base_name, created_at, updated_at
         ) VALUES (
             '{PROJECT_ID}', '/repos/alpha', '/repos/alpha', 'Alpha', 'A',
             '#3366cc', 'manual', 1, 'A', 'AGENTS.md', {TS}, {TS}
         );
         INSERT INTO projects (
             id, path, path_key, name, label, color, added_via,
             sort_order, scheme, base_name, created_at, updated_at
         ) VALUES (
             'p-00000000-0000-7000-8000-000000000002', '/repos/beta', '/repos/beta',
             'Beta', 'B', '#cc3366', 'scan', 2, 'B', 'CLAUDE.md', {TS}, {TS}
         );
         INSERT INTO settings (key, value, updated_at)
         VALUES ('ui.language', '\"en\"', {TS});
         INSERT INTO backup_records (
             id, project_id, sync_run_id, source_path, backup_path,
             size_bytes, sha256, status, created_at
         ) VALUES
             ('r-00000000-0000-7000-8000-000000000001', '{PROJECT_ID}', NULL,
              '{SOURCE_PATH}', 'alpha/2026/11/20261121T090000000Z-AGENTS.md',
              12, 'aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111',
              'ok', '2026-11-21T09:00:00.000Z'),
             ('r-00000000-0000-7000-8000-000000000002', '{PROJECT_ID}', NULL,
              '{SOURCE_PATH}', 'alpha/2026/11/20261121T100000000Z-AGENTS.md',
              34, 'bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222',
              'ok', '2026-11-21T10:00:00.000Z'),
             ('r-00000000-0000-7000-8000-000000000003', '{PROJECT_ID}', NULL,
              '{SOURCE_PATH}', 'alpha/2026/11/20261121T110000000Z-AGENTS.md',
              56, 'cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333',
              'pruned', '2026-11-21T11:00:00.000Z'),
             ('r-00000000-0000-7000-8000-000000000004',
              'p-00000000-0000-7000-8000-000000000002', NULL,
              '/repos/beta/AGENTS.md', 'beta/2026/11/20261121T100000000Z-AGENTS.md',
              78, 'dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444',
              'ok', '2026-11-21T10:00:00.000Z'),
             ('r-00000000-0000-7000-8000-000000000005', '{PROJECT_ID}', NULL,
              '{SOURCE_PATH}', 'alpha/2026/11/20261121T100000001Z-AGENTS.md',
              90, 'eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555',
              'ok', '2026-11-21T10:00:00.000Z');"
    ))
    .expect("the v1 rows should seed");
}

/// The seeded pair's newest `ok` copy — the row `last_checked` must
/// answer with on this fixture: `r-…0005` wins the same-stamp tie with
/// `r-…0002` by the higher id (the D-122 tiebreak), and the newer
/// `pruned` row must stay invisible. Derived from the stamps and ids
/// above (the requirement's "newest", not a value read back from the
/// database).
const NEWEST_OK_SHA256: &str = "eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555";
const NEWEST_OK_BACKUP_PATH: &str = "alpha/2026/11/20261121T100000001Z-AGENTS.md";

/// Reads the pair-index's columns from `PRAGMA index_info`, in index
/// order.
fn index_columns(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA index_info({PAIR_INDEX})"))
        .expect("index_info should be preparable");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(2))
        .expect("index_info rows should be readable");
    rows.map(|row| row.expect("every index_info row should decode"))
        .collect()
}

/// Whether `sqlite_master` holds the pair index.
fn pair_index_exists(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [PAIR_INDEX],
        |row| row.get::<_, i64>(0),
    )
    .expect("sqlite_master should be readable")
        == 1
}

#[test]
fn a_v1_database_with_rows_upgrades_to_v3_and_preserves_them() {
    let (db, backup_dir, _guard) = temp_data_dir("upgrade");
    let db_path = db.join("eunomiasync.sqlite3");
    seed_v1_schema(&db_path);
    {
        let conn = Connection::open(&db_path).unwrap();
        seed_v1_rows(&conn);
    }

    // The production upgrade shape: `open_db` (the DB-001 pragma set),
    // then `run_migrations` on the same connection (DB-004).
    let mut conn = open_db(&db_path).expect("the v1 database should open");
    run_migrations(&mut conn, &backup_dir).expect("the v1→v3 upgrade should apply cleanly");

    assert_eq!(user_version(&conn), 3, "the upgraded database is at v3");
    assert!(
        pair_index_exists(&conn),
        "the upgraded database has the pair index"
    );

    // Every seeded v1 row survives verbatim — v2 adds the index and v3
    // rebuilds only the assignment table. The record comparison covers every column
    // of every row (all nine of the DDL: `sync_run_id` is NULL in
    // every seed row and compared as that literal via COALESCE), not a
    // key subset: a migration that rewrote a digest, a path, a size,
    // a stamp or a link while keeping ids and statuses fails here.
    let project_name: String = conn
        .query_row(
            "SELECT name FROM projects WHERE id = ?1",
            [PROJECT_ID],
            |row| row.get(0),
        )
        .expect("the project row should survive the upgrade");
    assert_eq!(project_name, "Alpha");
    let setting: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'ui.language'",
            [],
            |row| row.get(0),
        )
        .expect("the settings row should survive the upgrade");
    assert_eq!(setting, "\"en\"");
    let records: Vec<[String; 9]> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, COALESCE(sync_run_id, 'NULL'), source_path, \
                 backup_path, size_bytes, sha256, status, created_at \
                 FROM backup_records ORDER BY id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok([
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get::<_, i64>(5)?.to_string(),
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ])
            })
            .unwrap();
        rows.map(|row| row.expect("every record row should decode"))
            .collect()
    };
    assert_eq!(
        records,
        [
            [
                "r-00000000-0000-7000-8000-000000000001",
                PROJECT_ID,
                "NULL",
                SOURCE_PATH,
                "alpha/2026/11/20261121T090000000Z-AGENTS.md",
                "12",
                "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111",
                "ok",
                "2026-11-21T09:00:00.000Z",
            ],
            [
                "r-00000000-0000-7000-8000-000000000002",
                PROJECT_ID,
                "NULL",
                SOURCE_PATH,
                "alpha/2026/11/20261121T100000000Z-AGENTS.md",
                "34",
                "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222",
                "ok",
                "2026-11-21T10:00:00.000Z",
            ],
            [
                "r-00000000-0000-7000-8000-000000000003",
                PROJECT_ID,
                "NULL",
                SOURCE_PATH,
                "alpha/2026/11/20261121T110000000Z-AGENTS.md",
                "56",
                "cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333",
                "pruned",
                "2026-11-21T11:00:00.000Z",
            ],
            [
                "r-00000000-0000-7000-8000-000000000004",
                "p-00000000-0000-7000-8000-000000000002",
                "NULL",
                "/repos/beta/AGENTS.md",
                "beta/2026/11/20261121T100000000Z-AGENTS.md",
                "78",
                "dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444",
                "ok",
                "2026-11-21T10:00:00.000Z",
            ],
            [
                "r-00000000-0000-7000-8000-000000000005",
                PROJECT_ID,
                "NULL",
                SOURCE_PATH,
                "alpha/2026/11/20261121T100000001Z-AGENTS.md",
                "90",
                "eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555",
                "ok",
                "2026-11-21T10:00:00.000Z",
            ],
        ]
        .map(|row| row.map(str::to_owned)),
        "all five records survive with every column verbatim"
    );

    // And the lookup the index exists for still answers correctly on
    // the upgraded database: the pair's newest `ok` copy — the
    // same-stamp id tiebreak included — terminal statuses invisible.
    let newest = last_checked(&conn, PROJECT_ID, SOURCE_PATH)
        .expect("last_checked should answer on the upgraded database");
    assert_eq!(
        newest,
        Some(CheckedCopy {
            sha256_hex: NEWEST_OK_SHA256.to_owned(),
            backup_path: NEWEST_OK_BACKUP_PATH.to_owned(),
        }),
        "the newest ok copy of the pair, unchanged by the upgrade"
    );
}

#[test]
fn a_fresh_database_reaches_user_version_three_with_the_pair_index() {
    let (db, _backup_dir, _guard) = temp_data_dir("fresh");
    let db_path = db.join("eunomiasync.sqlite3");
    let mut conn = open_db(&db_path).expect("a fresh database should open");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("a fresh bootstrap should apply every migration");

    assert_eq!(
        user_version(&conn),
        3,
        "a fully migrated database is at version 3"
    );
    assert!(
        pair_index_exists(&conn),
        "the pair index exists on a fresh database"
    );
    // The independent oracle: the owner-approved column set, in order —
    // an index silently created over different columns (or a different
    // order) is a regression a name-only check would miss.
    assert_eq!(
        index_columns(&conn),
        ["project_id", "source_path", "status", "created_at"]
            .map(str::to_owned)
            .to_vec(),
        "the index covers exactly the approved columns, in order"
    );
}

#[test]
fn the_last_checked_query_plan_uses_the_pair_index() {
    let (db, _backup_dir, _guard) = temp_data_dir("plan");
    let db_path = db.join("eunomiasync.sqlite3");
    let mut conn = open_db(&db_path).expect("a fresh database should open");
    run_migrations(&mut conn, Path::new("unused-backup-dir")).unwrap();
    seed_v1_rows(&conn);

    // The exact statement `last_checked` runs (see the mirror note on
    // the constant): its plan must search through the pair index, not
    // scan the table — the acceptance criterion that fails without the
    // index.
    let details: Vec<String> = {
        let mut stmt = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {LAST_CHECKED_SQL}"))
            .expect("the plan statement should prepare");
        let rows = stmt
            .query_map([PROJECT_ID, SOURCE_PATH], |row| row.get::<_, String>(3))
            .expect("plan rows should be readable");
        rows.map(|row| row.expect("every plan row should decode"))
            .collect()
    };
    let plan = details.join("; ");
    assert!(
        plan.contains(&format!("USING INDEX {PAIR_INDEX}")),
        "the plan must use the pair index, got: {plan}"
    );
    assert!(
        !plan.contains("SCAN backup_records"),
        "the plan must not scan the table, got: {plan}"
    );

    // The mirror cross-check: on this fixture the mirrored statement
    // and the production function must agree — a semantic drift of the
    // production SQL away from the mirrored text fails here.
    let via_mirror = conn
        .query_row(LAST_CHECKED_SQL, [PROJECT_ID, SOURCE_PATH], |row| {
            Ok(CheckedCopy {
                sha256_hex: row.get(0)?,
                backup_path: row.get(1)?,
            })
        })
        .expect("the mirrored statement should answer on the fixture");
    let via_production =
        last_checked(&conn, PROJECT_ID, SOURCE_PATH).expect("last_checked should answer");
    assert_eq!(
        Some(via_mirror),
        via_production,
        "the mirrored statement and the production function agree"
    );
}

#[test]
fn upgrading_from_v1_writes_a_valid_pre_migration_copy_of_the_v1_state() {
    let (db, backup_dir, _guard) = temp_data_dir("pre-copy");
    let db_path = db.join("eunomiasync.sqlite3");
    seed_v1_schema(&db_path);
    {
        let conn = Connection::open(&db_path).unwrap();
        seed_v1_rows(&conn);
    }

    let backups = {
        let mut conn = open_db(&db_path).expect("the v1 database should open");
        run_migrations(&mut conn, &backup_dir)
            .expect("the v1→v3 upgrade with its safety copies should succeed")
    };

    // Two pending steps, two copies, each named for the state it preserves
    // (D-038: `pre-migration-v1-…`, then `pre-migration-v2-…`).
    assert_eq!(backups.len(), 2, "two upgrade steps, two copies");
    let backup = &backups[0];
    let name = backup
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the copy name should be UTF-8");
    let prefix = "pre-migration-v1-";
    assert!(name.starts_with(prefix), "copy name: {name}");
    assert!(name.ends_with(".sqlite3"), "copy name: {name}");
    let stamp = &name[prefix.len()..name.len() - ".sqlite3".len()];
    assert_stamp_shape(stamp);

    // The copy is a valid, openable database preserving exactly the v1
    // state: version 1, no v2 objects, the user rows intact.
    let copy = Connection::open(backup).expect("the copy should open");
    let integrity: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert_eq!(user_version(&copy), 1, "the copy is the version-1 state");
    assert!(
        !pair_index_exists(&copy),
        "the v1 copy must not contain the v2 index"
    );
    let records: i64 = copy
        .query_row("SELECT COUNT(*) FROM backup_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(records, 5, "the copy preserves the v1 rows");

    let v2_name = backups[1]
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the v2 copy name should be UTF-8");
    assert!(
        v2_name.starts_with("pre-migration-v2-"),
        "copy name: {v2_name}"
    );
    let v2_copy = Connection::open(&backups[1]).expect("the v2 copy should open");
    assert_eq!(
        user_version(&v2_copy),
        2,
        "the second copy is the version-2 state"
    );
    assert!(
        pair_index_exists(&v2_copy),
        "the v2 copy already has the pair index"
    );
    let custom_columns: i64 = v2_copy
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('project_instructions') WHERE name = 'custom_text'",
            [],
            |row| row.get(0),
        )
        .expect("v2 table info reads");
    assert_eq!(
        custom_columns, 0,
        "the v2 copy predates the custom-target column"
    );

    // No staging debris survives the run.
    let mut names: Vec<String> = std::fs::read_dir(&backup_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![name.to_owned(), v2_name.to_owned()],
        "exactly the two pre-migration copies, no staging debris"
    );
}

/// Asserts the D-038 stamp shape of a backup file name:
/// `YYYYMMDDTHHMMSSSSSZ` — 19 characters: 8 of date, `T`, 6 of time,
/// 3 of milliseconds, `Z` (mirrored from `storage.rs`'s private
/// checker).
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
