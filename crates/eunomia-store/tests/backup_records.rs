//! Behavior tests for the `backup_records` repository (spec §6.2 DDL,
//! §7.6 BKP-010, §9.4 phase 2; task T-115, the listing added by
//! T-125, the id-addressed read by T-353).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` (the `tests/sync_runs.rs` pattern)
//! and drive the repository through its public API — [`insert`],
//! [`last_checked`], `all_paths` (T-332's orphan-report claim set),
//! [`list_for_project`] (T-125's per-project listing) and [`get`]
//! (T-353's id-addressed export lookup) — reading
//! the `backup_records` table back with this module's own SQL: the
//! table IS the specified artifact (§6.2), so its cells are the
//! observable behavior.
//!
//! Pinned behavior:
//!
//! * the checked-copy row shape: every column lands verbatim (the
//!   caller's `created_at`, the §5.5 relative `backup_path`, the
//!   resolved `source_path`, the size and digest the BKP-010 check
//!   passed), the id is the DB-002 UUIDv7 TEXT spelling, and the
//!   `sync_run_id` link references the run that made the copy;
//! * the discarded history row: the abort sweep's landing status,
//!   same shape, different CHECK member;
//! * the constraint floor: a `backup_path` another record claims
//!   (UNIQUE), a `project_id` no project backs, and a `sync_run_id`
//!   no run backs are all the typed `Sql` refusal, never a silent
//!   row;
//! * the BKP-010 dedup input: `last_checked` answers the NEWEST `ok`
//!   copy of the pair (`created_at DESC, id DESC` — the same
//!   "newest" D-122 fixed for the retention window, including the
//!   same-millisecond id tiebreak), stays invisible to terminal
//!   statuses (`pruned`/`missing`/`discarded` are history, not
//!   copies), scopes strictly by the "project and file" pair, and
//!   answers `None` honestly when the pair has no checked copy;
//! * the BKP-004 listing (T-125): `list_for_project` answers one
//!   project's whole row history — every §6.2 CHECK status, rows
//!   never deleted (§6.4) — newest first (D-122's ordering, the id
//!   tiebreak included), scoped strictly to the caller's project,
//!   `Ok(vec![])` for a project without rows, and the typed `Member`
//!   refusal for a corrupt status cell (D-039);
//! * the BKP-012 export lookup (T-353): `get` answers the id-addressed
//!   row verbatim (every column, the status decoded), `None` for an id
//!   naming no row (the command layer's typed refusal), and the same
//!   `Member` refusal for a corrupt status cell;
//! * the T-114 handshake, end to end: rows written through THIS
//!   writer over REAL files in a real data directory are pruned by
//!   `backup_retention::apply` — the new writer and the shipped
//!   reader agree on the layout, the statuses and the "newest"
//!   order, not just coexist.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_core::backup_name::{BackupTime, backup_relative_path};
use eunomia_core::project::{BaseName as CoreBaseName, ProjectId, Scheme as CoreScheme};
use eunomia_store::backup_records::{
    BackupRecordStatus, BackupRecordsError, CheckedCopy, NewBackupRecord, all_paths, get, insert,
    last_checked, list_for_project,
};
use eunomia_store::backup_retention;
use eunomia_store::projects::{AddedVia, NewProject};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::storage::{open_db, run_migrations};
use eunomia_store::sync_runs::{NewSyncRunProject, create_running};
use rusqlite::Connection;

/// A temporary directory removed when the guard is dropped (the
/// `tests/sync_runs.rs` pattern).
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set — foreign keys
/// included — applies) plus an empty `data/` root with a `backups/`
/// folder, and returns the connection and the data-directory path.
fn records_db(label: &str) -> (Connection, TempDir, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-backup-records-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    std::fs::create_dir_all(dir.join("data").join("backups"))
        .expect("temp data backups directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    let data_dir = dir.join("data");
    (conn, TempDir(dir), data_dir)
}

/// Registers project `n` and returns its id (the link table's FK
/// target, not these tests' subject).
fn seed_project(conn: &Connection, n: usize) -> String {
    eunomia_store::projects::create(
        conn,
        &NewProject {
            path: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
            path_key: Box::leak(format!("/repos/repo-{n}").into_boxed_str()),
            name: Box::leak(format!("Project {n}").into_boxed_str()),
            label: Box::leak(format!("P{n}").into_boxed_str()),
            color: "blue",
            description: "",
            added_via: AddedVia::Manual,
            scheme: Scheme::A,
            base_name: BaseName::AgentsMd,
            extra_local: ExtraFile {
                enabled: false,
                position: Position::End,
            },
            extra_project: ExtraFile {
                enabled: false,
                position: Position::End,
            },
        },
    )
    .unwrap_or_else(|err| panic!("seed project {n} should create, got {err}"))
    .id
}

/// Records a `running` run for one project and returns its id (the
/// `sync_run_id` FK target).
fn seed_run(conn: &Connection, project_id: &str) -> String {
    create_running(
        conn,
        "2026-11-21T12:00:00.000Z",
        &[NewSyncRunProject {
            project_id,
            target_path: "/repos/repo-1/AGENTS.md",
            build_hash: &"a".repeat(64),
        }],
    )
    .expect("seeding the run should work")
    .id
}

/// The caller's DB-002 backup timestamp — an explicit literal (an
/// oracle, not `now_utc` output), so the row's `created_at` is pinned
/// verbatim: the value the caller passed is the value stored.
const CREATED_AT: &str = "2026-11-21T12:00:01.123Z";

/// The digest of copy `n` as full lowercase hex — copy-distinct
/// (the first 8 characters differ, so the name's `hash8` component
/// does too) and shape-true (64 hex characters) without pretending
/// to be a real hash of anything.
fn digest(n: usize) -> String {
    format!("{n:0>8}{}", "0".repeat(56))
}

/// The §5.5 layout path of copy `n` of one project, built through the
/// REAL core builders (the D-122 fixture discipline: the writer's
/// stored `backup_path` is pinned against the builder's spelling, not
/// a hand-made string) with a copy-distinct digest so the `hash8`
/// component differs too.
fn layout_path(
    project_id: &str,
    project_name: &str,
    base_name: CoreBaseName,
    copy: usize,
    time: BackupTime,
) -> String {
    backup_relative_path(
        &ProjectId::new(project_id.to_owned()),
        project_name,
        CoreScheme::A,
        base_name,
        time,
        &digest(copy),
    )
    .expect("the builder accepts DB-shaped ids and 64-hex digests")
}

/// The `AGENTS.md` layout shorthand every same-file fixture uses.
fn agents_path(project_id: &str, project_name: &str, copy: usize, time: BackupTime) -> String {
    layout_path(project_id, project_name, CoreBaseName::AgentsMd, copy, time)
}

/// Asserts the canonical UUIDv7 TEXT shape on an id — the DB-002
/// spelling `new_id` produces, pinned by structure (36 chars, hyphens
/// at the fixed offsets, lowercase hex elsewhere, version nibble `7`,
/// variant nibble `8`..=`b`), the oracle `tests/sync_runs.rs` uses.
fn assert_uuidv7_text(id: &str) {
    let bytes = id.as_bytes();
    assert_eq!(bytes.len(), 36, "id must be 36 characters: {id}");
    for index in [8, 13, 18, 23] {
        assert_eq!(bytes[index], b'-', "byte {index} of {id}");
    }
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            continue;
        }
        assert!(
            byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
            "byte {index} of {id} must be a lowercase hex digit"
        );
    }
    assert_eq!(bytes[14], b'7', "version nibble of {id}");
    assert!(
        matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
        "variant nibble of {id}"
    );
}

/// Reads one row's cells straight back, in the table's column order —
/// the verbatim-shape oracle every insert test asserts against.
#[allow(clippy::type_complexity)]
fn row_cells(
    conn: &Connection,
    id: &str,
) -> (
    String,
    Option<String>,
    String,
    String,
    i64,
    String,
    String,
    String,
) {
    conn.query_row(
        "SELECT project_id, sync_run_id, source_path, backup_path, \
                size_bytes, sha256, status, created_at \
           FROM backup_records WHERE id = ?1",
        [id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        },
    )
    .unwrap_or_else(|err| panic!("reading the row {id} back should work, got {err}"))
}

/// One ready-to-insert record with per-call distinct literals, so a
/// swapped column binding fails the verbatim assertions.
fn sample_record<'a>(
    project_id: &'a str,
    run_id: &'a str,
    source_path: &'a str,
    backup_path: &'a str,
    sha256_hex: &'a str,
    status: BackupRecordStatus,
) -> NewBackupRecord<'a> {
    NewBackupRecord {
        project_id,
        sync_run_id: Some(run_id),
        source_path,
        backup_path,
        size_bytes: sha256_hex.len() as i64,
        sha256_hex,
        status,
        created_at: CREATED_AT,
    }
}

/// The backup stamp of copy `n`: distinct millisecond fields, so the
/// created_at/id ordering fixtures have a real timeline.
fn stamp_time(n: usize) -> BackupTime {
    BackupTime {
        year: 2026,
        month: 11,
        day: 21,
        hour: 12,
        minute: 0,
        second: 1,
        millisecond: u16::try_from(100 + n).expect("a small millisecond field"),
    }
}

// ---- the row shapes ---------------------------------------------------

#[test]
fn insert_writes_the_checked_copy_row_verbatim() {
    let (conn, _guard, _data) = records_db("ok-shape");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let backup_path = agents_path(&project, "Project 1", 1, stamp_time(1));
    let sha256 = digest(1);

    let id = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &backup_path,
            &sha256,
            BackupRecordStatus::Ok,
        ),
    )
    .expect("inserting the checked-copy row should work");

    // The id: the DB-002 UUIDv7 TEXT spelling.
    assert_uuidv7_text(&id);
    // Every column: the caller's values, verbatim.
    assert_eq!(
        row_cells(&conn, &id),
        (
            project,
            Some(run),
            "/repos/repo-1/AGENTS.md".to_owned(),
            backup_path,
            64,
            sha256,
            "ok".to_owned(),
            CREATED_AT.to_owned(),
        )
    );
}

#[test]
fn insert_writes_the_discarded_history_row_verbatim() {
    // §9.4 phase 2 step 2's landing status: the abort sweep's row is
    // the history of a copy this run created and removed, so it is
    // born terminal — the §6.2 CHECK member is the only difference
    // from the checked-copy shape.
    let (conn, _guard, _data) = records_db("discarded-shape");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let backup_path = agents_path(&project, "Project 1", 1, stamp_time(1));

    let id = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &backup_path,
            &digest(1),
            BackupRecordStatus::Discarded,
        ),
    )
    .expect("inserting the discarded row should work");

    // The full verbatim shape — only the status cell differs from
    // the checked-copy row (the same assertion family the ok-shape
    // test applies, so a swapped binding fails here too).
    let backup_path = agents_path(&project, "Project 1", 1, stamp_time(1));
    assert_eq!(
        row_cells(&conn, &id),
        (
            project,
            Some(run),
            "/repos/repo-1/AGENTS.md".to_owned(),
            backup_path,
            64,
            digest(1),
            "discarded".to_owned(),
            CREATED_AT.to_owned(),
        )
    );
}

// ---- the constraint floor ---------------------------------------------

#[test]
fn a_claimed_backup_path_is_the_typed_refusal() {
    // Two copies cannot share a §5.5 name — the UNIQUE column is the
    // database half of the file side's `create_new` never-overwrite
    // rule (D-091).
    let (conn, _guard, _data) = records_db("unique-path");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let backup_path = agents_path(&project, "Project 1", 1, stamp_time(1));
    insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &backup_path,
            &digest(1),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the first record should insert");

    let refusal = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &backup_path,
            &digest(2),
            BackupRecordStatus::Ok,
        ),
    )
    .expect_err("a claimed backup_path must be refused");

    // The constraint itself, not just "some SQL error": a broken
    // column binding would also answer `Sql(_)` and pass a loose
    // assertion.
    assert!(
        matches!(
            &refusal,
            BackupRecordsError::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                    ..
                },
                Some(message),
            )) if message.contains("backup_path")
        ),
        "the refusal is the UNIQUE constraint on backup_path, got {refusal:?}"
    );
}

#[test]
fn an_unbacked_project_or_run_is_the_typed_refusal() {
    let (conn, _guard, _data) = records_db("fk-floor");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);

    let project_refusal = insert(
        &conn,
        &NewBackupRecord {
            project_id: "0193f1a0-0000-7000-8000-000000000000",
            sync_run_id: Some(&run),
            source_path: "/repos/repo-1/AGENTS.md",
            backup_path: &agents_path(&project, "Project 1", 1, stamp_time(1)),
            size_bytes: 1,
            sha256_hex: &digest(1),
            status: BackupRecordStatus::Ok,
            created_at: CREATED_AT,
        },
    )
    .expect_err("an unknown project_id must be refused");
    assert!(
        matches!(
            &project_refusal,
            BackupRecordsError::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                    ..
                },
                Some(message),
            )) if message.contains("FOREIGN KEY")
        ),
        "the project FK refusal is the constraint itself, got {project_refusal:?}"
    );

    let run_refusal = insert(
        &conn,
        &NewBackupRecord {
            project_id: &project,
            sync_run_id: Some("0193f1a0-0000-7000-8000-000000000001"),
            source_path: "/repos/repo-1/AGENTS.md",
            backup_path: &agents_path(&project, "Project 1", 2, stamp_time(2)),
            size_bytes: 1,
            sha256_hex: &digest(2),
            status: BackupRecordStatus::Ok,
            created_at: CREATED_AT,
        },
    )
    .expect_err("an unknown sync_run_id must be refused");
    assert!(
        matches!(
            &run_refusal,
            BackupRecordsError::Sql(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                    ..
                },
                Some(message),
            )) if message.contains("FOREIGN KEY")
        ),
        "the run FK refusal is the constraint itself, got {run_refusal:?}"
    );
}

// ---- the BKP-010 dedup input ------------------------------------------

#[test]
fn last_checked_answers_the_newest_ok_copy_of_the_pair() {
    let (conn, _guard, _data) = records_db("newest");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let source = "/repos/repo-1/AGENTS.md";
    // Three copies on a real timeline: the newest (stamp 3) must win.
    for n in 1..=3 {
        insert(
            &conn,
            &sample_record(
                &project,
                &run,
                source,
                &agents_path(&project, "Project 1", n, stamp_time(n)),
                &digest(n),
                BackupRecordStatus::Ok,
            ),
        )
        .expect("the fixture record should insert");
    }

    assert_eq!(
        last_checked(&conn, &project, source).expect("the lookup should work"),
        Some(CheckedCopy {
            sha256_hex: digest(3),
            backup_path: agents_path(&project, "Project 1", 3, stamp_time(3)),
        })
    );
}

#[test]
fn a_same_stamp_pair_resolves_newest_by_the_id() {
    // D-122's tiebreak, pinned from the writer's side: same
    // `created_at`, and the LATER insert (a greater UUIDv7, D-031's
    // creation order) is the newest copy.
    let (conn, _guard, _data) = records_db("tiebreak");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let source = "/repos/repo-1/AGENTS.md";
    let time = stamp_time(1);
    let first = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            source,
            &agents_path(&project, "Project 1", 1, time),
            &digest(1),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the first record should insert");
    let second = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            source,
            &agents_path(&project, "Project 1", 2, time),
            &digest(2),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the second record should insert");
    assert!(second > first, "UUIDv7 TEXT ids sort by creation");

    assert_eq!(
        last_checked(&conn, &project, source)
            .expect("the lookup should work")
            .map(|copy| copy.sha256_hex),
        Some(digest(2)),
        "the later insert wins the same-stamp tie"
    );
}

#[test]
fn last_checked_is_invisible_to_terminal_statuses() {
    // A `pruned`/`missing`/`discarded` row is history of a gone copy,
    // not a copy (D-122's ok-only set): the dedup must never point a
    // record at one — with only terminal rows, the pair has no
    // checked copy.
    let (conn, _guard, _data) = records_db("terminal");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let source = "/repos/repo-1/AGENTS.md";
    for (n, status) in [
        (1, BackupRecordStatus::Pruned),
        (2, BackupRecordStatus::Missing),
        (3, BackupRecordStatus::Discarded),
    ] {
        insert(
            &conn,
            &sample_record(
                &project,
                &run,
                source,
                &agents_path(&project, "Project 1", n, stamp_time(n)),
                &digest(n),
                status,
            ),
        )
        .expect("the terminal fixture record should insert");
    }

    assert_eq!(
        last_checked(&conn, &project, source).expect("the lookup should work"),
        None,
        "terminal rows are not copies"
    );

    // And the newest OK copy still wins when older terminal rows
    // exist alongside it (the go-live history shape: an old pruned
    // copy, a fresh checked one).
    insert(
        &conn,
        &sample_record(
            &project,
            &run,
            source,
            &agents_path(&project, "Project 1", 4, stamp_time(4)),
            &digest(4),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the checked record should insert");
    assert_eq!(
        last_checked(&conn, &project, source)
            .expect("the lookup should work")
            .map(|copy| copy.sha256_hex),
        Some(digest(4))
    );
}

#[test]
fn last_checked_scopes_by_the_project_and_file_pair() {
    let (conn, _guard, _data) = records_db("pairs");
    let first = seed_project(&conn, 1);
    let second = seed_project(&conn, 2);
    let run = seed_run(&conn, &first);
    // Two pairs: same project, different files (A and B names), plus
    // another project on the same source spelling — each pair's
    // newest is its own. The digests carry the pair index in their
    // first digit so the winner is identifiable; the layout paths
    // differ per project folder and copy number.
    for (pair, project, source) in [
        (1usize, &first, "/repos/repo-1/AGENTS.md"),
        (2, &first, "/repos/repo-1/CLAUDE.md"),
        (3, &second, "/repos/repo-1/AGENTS.md"),
    ] {
        let name = if pair == 3 { "Project 2" } else { "Project 1" };
        // The pair's own file kind: the layout path carries it, so two
        // pairs of one project cannot collide on the UNIQUE column.
        let base = if pair == 2 {
            CoreBaseName::ClaudeMd
        } else {
            CoreBaseName::AgentsMd
        };
        for copy in 1..=2 {
            insert(
                &conn,
                &sample_record(
                    project,
                    &run,
                    source,
                    &layout_path(project, name, base, copy, stamp_time(copy)),
                    &format!("{pair}{copy}{}", "0".repeat(62)),
                    BackupRecordStatus::Ok,
                ),
            )
            .expect("the pair fixture record should insert");
        }
    }

    for (project, source, expected) in [
        (&first, "/repos/repo-1/AGENTS.md", "12"),
        (&first, "/repos/repo-1/CLAUDE.md", "22"),
        (&second, "/repos/repo-1/AGENTS.md", "32"),
    ] {
        let checked = last_checked(&conn, project, source)
            .expect("the lookup should work")
            .unwrap_or_else(|| panic!("the pair ({project}, {source}) has a copy"));
        assert!(
            checked.sha256_hex.starts_with(expected),
            "the pair's own newest copy wins, got {}",
            checked.sha256_hex
        );
    }
}

#[test]
fn last_checked_answers_none_where_no_checked_copy_exists() {
    let (conn, _guard, _data) = records_db("none");
    let project = seed_project(&conn, 1);
    assert_eq!(
        last_checked(&conn, &project, "/repos/repo-1/AGENTS.md").expect("the lookup should work"),
        None,
        "an empty table answers None"
    );
}

/// The orphan report's known-set read (T-332): every claimed path comes
/// back, **whatever** its status — the terminal rows still claim the
/// names they refer to, which is what keeps a `pruned` copy's absent
/// file out of the report and a `discarded` one from ever being called
/// an orphan.
#[test]
fn all_paths_returns_every_claimed_path_across_statuses() {
    let (conn, _guard, _data) = records_db("all-paths");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let source = "/repos/repo-1/AGENTS.md";
    let mut expected: Vec<String> = Vec::new();
    for (n, status) in [
        (1, BackupRecordStatus::Ok),
        (2, BackupRecordStatus::Pruned),
        (3, BackupRecordStatus::Missing),
        (4, BackupRecordStatus::Discarded),
    ] {
        let backup_path = agents_path(&project, "Project 1", n, stamp_time(n));
        insert(
            &conn,
            &sample_record(&project, &run, source, &backup_path, &digest(n), status),
        )
        .expect("the fixture record should insert");
        expected.push(backup_path);
    }
    expected.sort();

    assert_eq!(
        all_paths(&conn).expect("the read should work"),
        expected,
        "every status's path is a claim, in stored-path order"
    );

    // The empty table is the honest empty answer, not an error.
    let (empty, _guard, _data) = records_db("all-paths-empty");
    assert_eq!(
        all_paths(&empty).expect("the read should work"),
        Vec::<String>::new()
    );
}

// ---- the T-114 handshake ----------------------------------------------
#[test]
fn the_retention_service_prunes_rows_written_by_this_repository() {
    // The D-122 compatibility criterion, end to end: three checked
    // copies of one pair written through THIS writer over REAL files
    // in a real data directory, then `backup_retention::apply` at
    // keep 1 — the shipped reader (T-114) must see the writer's rows
    // exactly the way it saw its own fixtures: the two oldest files
    // deleted with their records `pruned`, the newest file alive and
    // its record still `ok`.
    let (conn, _guard, data_dir) = records_db("handshake");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let source = "/repos/repo-1/AGENTS.md";
    for n in 1..=3 {
        let backup_path = agents_path(&project, "Project 1", n, stamp_time(n));
        let copy = data_dir.join(&backup_path);
        std::fs::create_dir_all(
            copy.parent()
                .expect("the builder's layout always has a parent"),
        )
        .expect("the copy's folder should be creatable");
        std::fs::write(&copy, format!("copy {n}")).expect("the copy file should be writable");
        insert(
            &conn,
            &sample_record(
                &project,
                &run,
                source,
                &backup_path,
                &digest(n),
                BackupRecordStatus::Ok,
            ),
        )
        .expect("the handshake record should insert");
    }

    let outcome = backup_retention::apply(&conn, &data_dir, 1)
        .expect("the retention pass over the writer's rows should work");

    assert_eq!(outcome.pruned, 2, "exactly the two oldest records prune");
    assert!(
        outcome.failures.is_empty(),
        "no prune failures: {:?}",
        outcome.failures
    );
    let statuses = {
        let mut statement = conn
            .prepare(
                "SELECT status, backup_path FROM backup_records
                  WHERE project_id = ?1 AND source_path = ?2
                  ORDER BY created_at, id",
            )
            .expect("the statuses query should prepare");
        statement
            .query_map(rusqlite::params![project, source], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("the statuses query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("reading the statuses should work")
    };
    assert_eq!(statuses.len(), 3, "no row is ever deleted (§6.4)");
    assert_eq!(statuses[0].0, "pruned");
    assert!(
        !data_dir.join(&statuses[0].1).exists(),
        "the oldest file is gone"
    );
    assert_eq!(statuses[1].0, "pruned");
    assert!(
        !data_dir.join(&statuses[1].1).exists(),
        "the middle file is gone"
    );
    assert_eq!(statuses[2].0, "ok", "the newest copy stays checked");
    assert!(
        data_dir.join(&statuses[2].1).exists(),
        "the newest file stays"
    );
}

// ---- the per-project listing (T-125, BKP-004) --------------------------

/// One record with an explicit `created_at` cell: unlike
/// [`sample_record`] (the fixed writer-side literal), the listing's
/// timeline needs row-distinct DB-002 stamps so `created_at DESC` is
/// observable on its own, not only through the id tiebreak.
fn stamped_record<'a>(
    project_id: &'a str,
    run_id: &'a str,
    source_path: &'a str,
    backup_path: &'a str,
    sha256_hex: &'a str,
    status: BackupRecordStatus,
    created_at: &'a str,
) -> NewBackupRecord<'a> {
    NewBackupRecord {
        project_id,
        sync_run_id: Some(run_id),
        source_path,
        backup_path,
        size_bytes: sha256_hex.len() as i64,
        sha256_hex,
        status,
        created_at,
    }
}

#[test]
fn list_for_project_returns_every_status_newest_first_and_scoped() {
    // The Backups tab's read (BKP-004): the row set is the project's
    // whole `backup_records` history — every §6.2 CHECK status, rows
    // never deleted (§6.4) — newest first by the D-122 ordering, and
    // strictly scoped to the caller's project.
    let (conn, _guard, _data) = records_db("list");
    let project = seed_project(&conn, 1);
    let other = seed_project(&conn, 2);
    let run = seed_run(&conn, &project);
    let run_two = seed_run(&conn, &other);
    let source = "/repos/repo-1/AGENTS.md";
    // One row per status on a real timeline: stamp 4 is newest, 1 the
    // oldest. The INSERT order mixes the timeline (3 first, 2 last), so
    // an answer ordered by insertion is caught.
    let wanted: [(usize, BackupRecordStatus, &str); 4] = [
        (4, BackupRecordStatus::Ok, "2026-11-21T12:00:04.000Z"),
        (3, BackupRecordStatus::Pruned, "2026-11-21T12:00:03.000Z"),
        (2, BackupRecordStatus::Missing, "2026-11-21T12:00:02.000Z"),
        (1, BackupRecordStatus::Discarded, "2026-11-21T12:00:01.000Z"),
    ];
    let inserted = [
        (3, BackupRecordStatus::Pruned, "2026-11-21T12:00:03.000Z"),
        (4, BackupRecordStatus::Ok, "2026-11-21T12:00:04.000Z"),
        (1, BackupRecordStatus::Discarded, "2026-11-21T12:00:01.000Z"),
        (2, BackupRecordStatus::Missing, "2026-11-21T12:00:02.000Z"),
    ];
    for (n, status, created_at) in inserted {
        insert(
            &conn,
            &stamped_record(
                &project,
                &run,
                source,
                &agents_path(&project, "Project 1", n, stamp_time(n)),
                &digest(n),
                status,
                created_at,
            ),
        )
        .expect("the fixture record should insert");
    }
    // A foreign project's row: it must not leak into the listing.
    insert(
        &conn,
        &stamped_record(
            &other,
            &run_two,
            "/repos/repo-2/AGENTS.md",
            &agents_path(&other, "Project 2", 9, stamp_time(9)),
            &digest(9),
            BackupRecordStatus::Ok,
            "2026-11-21T12:00:09.000Z",
        ),
    )
    .expect("the foreign record should insert");

    let rows = list_for_project(&conn, &project).expect("the listing should work");
    assert_eq!(
        rows.iter().map(|row| row.status).collect::<Vec<_>>(),
        wanted.map(|(_, status, _)| status).to_vec(),
        "every status is listed, newest first (created_at DESC)"
    );
    for (row, (n, _, created_at)) in rows.iter().zip(wanted.iter()) {
        assert_eq!(row.project_id, project);
        assert_eq!(row.sync_run_id, Some(run.clone()));
        assert_eq!(row.source_path, source);
        assert_eq!(
            row.backup_path,
            agents_path(&project, "Project 1", *n, stamp_time(*n))
        );
        assert_eq!(row.size_bytes, digest(*n).len() as i64);
        assert_eq!(row.sha256, digest(*n));
        assert_eq!(row.created_at, *created_at);
        assert_uuidv7_text(&row.id);
    }
}

#[test]
fn list_for_project_breaks_the_same_stamp_tie_by_id_desc() {
    // D-122's tiebreak on the listing's side: same `created_at`, and
    // the LATER insert (a greater UUIDv7, D-031's creation order)
    // is the newer row.
    let (conn, _guard, _data) = records_db("list-tie");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let source = "/repos/repo-1/AGENTS.md";
    let time = stamp_time(1);
    let first = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            source,
            &agents_path(&project, "Project 1", 1, time),
            &digest(1),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the first record should insert");
    let second = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            source,
            &agents_path(&project, "Project 1", 2, time),
            &digest(2),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the second record should insert");
    assert!(second > first, "UUIDv7 TEXT ids sort by creation");

    let rows = list_for_project(&conn, &project).expect("the listing should work");
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        vec![second.as_str(), first.as_str()],
        "the later insert wins the same-stamp tie"
    );
}

#[test]
fn list_for_project_answers_empty_for_a_project_without_records() {
    let (conn, _guard, _data) = records_db("list-empty");
    let project = seed_project(&conn, 1);

    let rows = list_for_project(&conn, &project).expect("the listing should work");
    assert_eq!(rows, Vec::new(), "no rows is the honest empty list");
}

#[test]
fn list_for_project_refuses_a_corrupt_status_cell() {
    // D-039's corrupt-cells policy on the read path: a status cell
    // outside the CHECK (a newer app version or tampering) is the
    // typed `Member` refusal naming the column — never a guess. The
    // schema's own CHECK refuses the tampering write, so the cell is
    // planted with `ignore_check_constraints` for the duration of the
    // statement (the `sync_runs` suite's corrupt-cell pattern) — the
    // read under test never bypasses anything.
    let (conn, _guard, _data) = records_db("list-corrupt");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let id = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &agents_path(&project, "Project 1", 1, stamp_time(1)),
            &digest(1),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the fixture record should insert");
    conn.pragma_update(None, "ignore_check_constraints", true)
        .expect("relaxing the CHECK for the tampering write should work");
    conn.execute(
        "UPDATE backup_records SET status = 'pending' WHERE id = ?1",
        [&id],
    )
    .expect("tampering with the status cell should apply");
    conn.pragma_update(None, "ignore_check_constraints", false)
        .expect("restoring the CHECK should work");

    assert_eq!(
        list_for_project(&conn, &project),
        Err(BackupRecordsError::Member {
            column: "status",
            found: "pending".to_owned(),
        })
    );
}

// ---- the id-addressed read (BKP-012, T-353) ---------------------------

#[test]
fn get_answers_the_id_addressed_row_verbatim() {
    // The export command's lookup (D-193's row form): every column the
    // record carries, read back by the row's own id — the same verbatim
    // discipline as the insert-shape oracle, so a swapped column
    // binding fails here too.
    let (conn, _guard, _data) = records_db("get-row");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let backup_path = agents_path(&project, "Project 1", 1, stamp_time(1));
    let sha256 = digest(1);
    let id = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &backup_path,
            &sha256,
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the fixture record should insert");

    let row = get(&conn, &id)
        .expect("the id-addressed read should work")
        .expect("the row the id names exists");
    assert_eq!(row.id, id);
    assert_eq!(row.project_id, project);
    assert_eq!(row.sync_run_id.as_deref(), Some(run.as_str()));
    assert_eq!(row.source_path, "/repos/repo-1/AGENTS.md");
    assert_eq!(row.backup_path, backup_path);
    assert_eq!(row.size_bytes, 64);
    assert_eq!(row.sha256, sha256);
    assert_eq!(row.status, BackupRecordStatus::Ok);
    assert_eq!(row.created_at, CREATED_AT);
}

#[test]
fn get_answers_none_for_an_unknown_id() {
    // `None` is the honest absence (the command layer's typed
    // E_VALIDATION refusal) — never a guessed or nearest row.
    let (conn, _guard, _data) = records_db("get-unknown");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &agents_path(&project, "Project 1", 1, stamp_time(1)),
            &digest(1),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the fixture record should insert");

    let row = get(&conn, "no-row-has-this-id").expect("the read itself should work");
    assert_eq!(row, None, "an id naming no row is None");
}

#[test]
fn get_refuses_a_corrupt_status_cell() {
    // The same D-039 posture as the listing: a status cell outside the
    // CHECK is the typed `Member` refusal, planted the same way.
    let (conn, _guard, _data) = records_db("get-corrupt");
    let project = seed_project(&conn, 1);
    let run = seed_run(&conn, &project);
    let id = insert(
        &conn,
        &sample_record(
            &project,
            &run,
            "/repos/repo-1/AGENTS.md",
            &agents_path(&project, "Project 1", 1, stamp_time(1)),
            &digest(1),
            BackupRecordStatus::Ok,
        ),
    )
    .expect("the fixture record should insert");
    conn.pragma_update(None, "ignore_check_constraints", true)
        .expect("relaxing the CHECK for the tampering write should work");
    conn.execute(
        "UPDATE backup_records SET status = 'pending' WHERE id = ?1",
        [&id],
    )
    .expect("tampering with the status cell should apply");
    conn.pragma_update(None, "ignore_check_constraints", false)
        .expect("restoring the CHECK should work");

    assert_eq!(
        get(&conn, &id),
        Err(BackupRecordsError::Member {
            column: "status",
            found: "pending".to_owned(),
        })
    );
}
