//! Behavior tests for the LOG-006 retention cleanup (spec §7.7,
//! §6.4 "Log and runs", §6.5; task T-029).
//!
//! The tests run against real migrated databases opened through
//! `open_db` (the production configuration — DB-001 pragmas including
//! `PRAGMA foreign_keys=ON`, D-030) and drive [`cleanup`] through its
//! public API. The journal and the runs tables are seeded with direct
//! SQL: the history under test is *accumulated* rows with explicit
//! timestamps, which the production writers stamp with the wall clock
//! — a test that needed `insert`'s own `now_utc` could not place an
//! entry three days in the past.
//!
//! The covered contract, claim by claim:
//!
//! * the count criterion — at a limit of 1000 entries exactly the
//!   1000 newest remain (the card's acceptance criterion), where
//!   "newest" is `ts DESC, id DESC` (T-027's ordering: a late insert
//!   carrying an old `ts` is not one of the newest);
//! * the age criterion and its boundary — strictly older than the
//!   cutoff goes, an entry exactly `max_days` old stays;
//! * the disjunction of LOG-006 — each limit alone removes an entry;
//! * `sync_runs` keeps the last 500, their `sync_run_projects` rows
//!   go with them (the schema's `ON DELETE CASCADE`), and a
//!   `backup_records` row survives its run's deletion with a NULL
//!   `sync_run_id` (`ON DELETE SET NULL`);
//! * journal entries naming a pruned run keep their `sync_run_id`
//!   string (no foreign key on the column, §6.2);
//! * idempotency — a second run removes nothing;
//! * the caller's transaction — a rollback keeps every row.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::action_log::{LogLimits, cleanup};
use eunomia_store::storage::{StorageError, open_db, run_migrations};
use jiff::Timestamp;
use rusqlite::Connection;

/// A fixed "now" for the age tests: 2026-11-19T00:00:00Z, chosen so
/// the cutoffs land on clean whole-day boundaries.
const NOW_MILLIS: i64 = 1_795_046_400_000;

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set — including
/// `PRAGMA foreign_keys=ON` — applies) and returns the connection
/// plus the directory for teardown. The same helper shape as the
/// other store integration suites.
fn retention_db(label: &str) -> (Connection, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-action-log-cleanup-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, dir)
}

/// Closes the connection before removing the temporary directory —
/// the Windows teardown rule the store suite pins.
fn teardown(conn: Connection, dir: PathBuf) {
    drop(conn);
    std::fs::remove_dir_all(&dir).expect("the temp directory should be removable");
}

/// Runs [`cleanup`] with a fixed `now`, unwrapping failures through
/// the panic (every test's arrange/act would be meaningless on an
/// error the tests do not target).
fn cleanup_at(
    conn: &Connection,
    limits: &LogLimits,
    now_millis: i64,
) -> eunomia_store::action_log::CleanupCounts {
    let now = Timestamp::from_millisecond(now_millis).expect("a fixed instant must parse");
    cleanup(conn, limits, now).unwrap_or_else(|err| panic!("cleanup should succeed, got {err}"))
}

/// The DB-002 text of an instant offset from [`NOW_MILLIS`] by
/// whole days plus milliseconds — an oracle built from calendar
/// arithmetic, not from the code under test.
fn ts(days_back: i64, millis: i64) -> String {
    Timestamp::from_millisecond(NOW_MILLIS - days_back * 86_400_000 + millis)
        .expect("a fixed instant must parse")
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Inserts one journal row with an explicit `ts` and returns its
/// autoincrement id. The vocabulary cells carry minimal valid
/// members (the schema CHECKs accept them).
fn seed_log(conn: &Connection, ts: &str) -> i64 {
    conn.query_row(
        "INSERT INTO action_log (ts, level, visibility, action, result)
         VALUES (?1, 'info', 'user', 'project.added', 'ok')
         RETURNING id",
        [ts],
        |row| row.get(0),
    )
    .expect("seeding an action_log row should work")
}

/// Inserts one journal row naming `sync_run_id` and returns its id.
fn seed_log_for_run(conn: &Connection, ts: &str, sync_run_id: &str) -> i64 {
    conn.query_row(
        "INSERT INTO action_log (ts, level, visibility, action, result, sync_run_id)
         VALUES (?1, 'info', 'user', 'sync.run_finished', 'ok', ?2)
         RETURNING id",
        rusqlite::params![ts, sync_run_id],
        |row| row.get(0),
    )
    .expect("seeding an action_log row with a sync_run_id should work")
}

/// How many journal rows exist.
fn log_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM action_log", [], |row| row.get(0))
        .expect("counting action_log should work")
}

/// The journal ids that exist, ascending — the surviving set a test
/// compares against.
fn log_ids(conn: &Connection) -> Vec<i64> {
    let mut statement = conn
        .prepare("SELECT id FROM action_log ORDER BY id")
        .expect("preparing the action_log listing should work");
    statement
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("listing action_log ids should work")
        .collect::<Result<Vec<_>, _>>()
        .expect("every action_log row should be readable")
}

/// Inserts one `projects` row (the parent `sync_run_projects` and
/// `backup_records` reference, RESTRICT) with a fixed id.
fn seed_project(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO projects (id, path, path_key, name, label, color, added_via,
                               sort_order, scheme, base_name, created_at, updated_at)
         VALUES (?1, '/repo/x', ?1, 'x', 'X', 'blue', 'manual', 1, 'A', 'AGENTS.md',
                 '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
        [id],
    )
    .expect("seeding a projects row should work");
}

/// Inserts one `sync_runs` row with an explicit `started_at`.
fn seed_run(conn: &Connection, id: &str, started_at: &str) {
    conn.execute(
        "INSERT INTO sync_runs (id, started_at, status) VALUES (?1, ?2, 'success')",
        rusqlite::params![id, started_at],
    )
    .expect("seeding a sync_runs row should work");
}

/// Inserts one `sync_run_projects` row.
fn seed_run_project(conn: &Connection, run_id: &str, project_id: &str) {
    conn.execute(
        "INSERT INTO sync_run_projects (sync_run_id, project_id, status)
         VALUES (?1, ?2, 'written')",
        rusqlite::params![run_id, project_id],
    )
    .expect("seeding a sync_run_projects row should work");
}

/// Inserts one `backup_records` row referencing a run.
fn seed_backup_record(conn: &Connection, id: &str, project_id: &str, run_id: &str) {
    conn.execute(
        "INSERT INTO backup_records (id, project_id, sync_run_id, source_path,
                                     backup_path, size_bytes, sha256, status, created_at)
         VALUES (?1, ?2, ?3, '/repo/x/AGENTS.md', ?4, 1, 'hash', 'ok',
                 '2026-01-01T00:00:00.000Z')",
        rusqlite::params![id, project_id, run_id, format!("/backups/{id}.bak")],
    )
    .expect("seeding a backup_records row should work");
}

/// How many `sync_runs` rows exist.
fn run_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
        .expect("counting sync_runs should work")
}

/// The `sync_runs` ids that exist, ascending.
fn run_ids(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT id FROM sync_runs ORDER BY id")
        .expect("preparing the sync_runs listing should work");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("listing sync_runs ids should work")
        .collect::<Result<Vec<_>, _>>()
        .expect("every sync_runs row should be readable")
}

/// Limits that take the age criterion out of play: entries within a
/// decade stay unless the count criterion removes them (3650 is the
/// §6.5 ceiling).
fn entries_only(max_entries: i64) -> LogLimits {
    LogLimits {
        max_entries,
        max_days: 3650,
    }
}

/// Limits that take the count criterion out of play: up to 100 000
/// entries (the §6.5 ceiling) stay unless the age criterion removes
/// them.
fn days_only(max_days: i64) -> LogLimits {
    LogLimits {
        max_entries: 100_000,
        max_days,
    }
}

#[test]
fn count_limit_keeps_exactly_the_newest_entries() {
    // The card's acceptance criterion, at the §6.5 range floor: 1200
    // accumulated entries, limit 1000 — exactly 1000 remain, and they
    // are the newest 1000 by `ts DESC, id DESC`.
    let (conn, dir) = retention_db("count");
    for hour in 0..1200 {
        // One entry per hour back from the fixed now: ts decreases as
        // the loop runs, ids increase — the two orders agree, so the
        // newest 1000 by ts are the newest 1000 by id.
        seed_log(&conn, &ts(0, -hour * 3_600_000));
    }
    assert_eq!(log_count(&conn), 1200);

    let counts = cleanup_at(&conn, &entries_only(1000), NOW_MILLIS);

    assert_eq!(log_count(&conn), 1000, "exactly the limit must remain");
    assert_eq!(counts.log_entries, 200, "the removed count is reported");
    assert_eq!(counts.sync_runs, 0);
    assert_eq!(
        log_ids(&conn),
        (1..=1000).collect::<Vec<_>>(),
        "the newest 1000 by ts DESC, id DESC must survive"
    );
    teardown(conn, dir);
}

#[test]
fn newest_means_ts_first_not_insert_order() {
    // T-027's ordering, not insert order: a backward clock step can
    // write an entry whose id is high but whose ts is old. With a
    // limit of 2 among 4 entries, the kept two are the two newest by
    // `ts DESC, id DESC` — the late-inserted old entry is deleted
    // even though its id is the newest.
    let (conn, dir) = retention_db("ts-first");
    seed_log(&conn, &ts(3, 0)); // id 1, ts oldest
    let second_newest = seed_log(&conn, &ts(1, 0)); // id 2
    let newest = seed_log(&conn, &ts(0, 0)); // id 3
    seed_log(&conn, &ts(3, -1)); // id 4, written last, ts below the oldest

    let counts = cleanup_at(&conn, &entries_only(2), NOW_MILLIS);

    assert_eq!(counts.log_entries, 2);
    assert_eq!(
        log_ids(&conn),
        vec![second_newest, newest],
        "the survivors are the newest by ts, not by insert id"
    );
    teardown(conn, dir);
}

#[test]
fn age_limit_is_strict_and_the_boundary_entry_survives() {
    // "Older than max_days" is strict: with max_days = 7, the cutoff
    // is exactly 7 days before now. An entry stamped exactly at the
    // cutoff is at the limit, not past it, and survives; one
    // millisecond older goes; a newer one stays untouched.
    let (conn, dir) = retention_db("age");
    seed_log(&conn, &ts(7, -1)); // 1 ms past the cutoff — deleted
    let boundary = seed_log(&conn, &ts(7, 0)); // exactly max_days old
    let recent = seed_log(&conn, &ts(1, 0));

    let counts = cleanup_at(&conn, &days_only(7), NOW_MILLIS);

    assert_eq!(counts.log_entries, 1, "only the strictly older entry goes");
    assert_eq!(
        log_ids(&conn),
        vec![boundary, recent],
        "the exact-boundary entry survives (strictly-older semantics)"
    );
    teardown(conn, dir);
}

#[test]
fn the_age_limit_alone_removes_an_entry_inside_the_count_limit() {
    // Half of LOG-006's disjunction: an entry that the count limit
    // would keep (it is among the newest N) is still deleted when it
    // is past the age cutoff — "10,000 entries **or** 180 days".
    // Two fresh entries plus two old ones with N = 3: the 390-day
    // entry sits inside the newest-3 window yet past the 365-day
    // cutoff, and goes on the age limit alone; the 400-day entry
    // fails both limits.
    let (conn, dir) = retention_db("age-alone");
    let fresh_a = seed_log(&conn, &ts(0, -1));
    let fresh_b = seed_log(&conn, &ts(0, -2));
    seed_log(&conn, &ts(390, 0)); // inside the newest-3 window, past the cutoff
    seed_log(&conn, &ts(400, 0)); // outside the window and past the cutoff

    let counts = cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 3, // the window holds both fresh entries and the 390-day one
            max_days: 365,
        },
        NOW_MILLIS,
    );

    assert_eq!(counts.log_entries, 2);
    assert_eq!(
        log_ids(&conn),
        vec![fresh_a, fresh_b],
        "the 390-day entry was inside the count window and still went (age alone)"
    );
    teardown(conn, dir);
}

#[test]
fn the_count_limit_alone_removes_a_fresh_entry_outside_the_window() {
    // The other half of the disjunction: fresh entries — well inside
    // the age cutoff — are still deleted when the count window is
    // smaller than the fresh bulk. Both deletions here (the fourth
    // fresh entry and the old one) are count-alone: the age limit of
    // 3650 days (the §6.5 ceiling) rejects nothing in this journal.
    let (conn, dir) = retention_db("count-alone");
    let fresh_a = seed_log(&conn, &ts(0, -1));
    let fresh_b = seed_log(&conn, &ts(0, -2));
    let fresh_c = seed_log(&conn, &ts(0, -3));
    seed_log(&conn, &ts(0, -4)); // the fourth fresh entry — outside the window
    seed_log(&conn, &ts(400, 0)); // the old entry — also outside the window

    let counts = cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 3, // the window holds the three newest fresh entries
            max_days: 3650,
        },
        NOW_MILLIS,
    );

    assert_eq!(counts.log_entries, 2);
    assert_eq!(
        log_ids(&conn),
        vec![fresh_a, fresh_b, fresh_c],
        "the fourth fresh entry went on the count limit alone (the age limit rejected nothing)"
    );
    teardown(conn, dir);
}

#[test]
fn combined_limits_prune_a_realistic_mix() {
    // A journal with a realistic shape — recent bulk, a middle-age
    // tail, an ancient tail — against the §6.5 defaults as a strict
    // but combined pair (small enough to prune every group).
    let (conn, dir) = retention_db("combined");
    // Five recent entries (today).
    for i in 0..5 {
        seed_log(&conn, &ts(0, -(i + 1) * 60_000));
    }
    // Three middle entries (10 days old — inside 30 days).
    for i in 0..3 {
        seed_log(&conn, &ts(10, -i * 60_000));
    }
    // Two ancient entries (100 days old — past a 30-day cutoff).
    for i in 0..2 {
        seed_log(&conn, &ts(100, -i * 60_000));
    }

    // Limit 6 by count, 30 by age: the 5 recent + 3 middle = 8 are
    // inside the age limit, but only the newest 6 of them are inside
    // the count limit; both ancient entries fail the age limit.
    let counts = cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 6,
            max_days: 30,
        },
        NOW_MILLIS,
    );

    assert_eq!(counts.log_entries, 4, "2 oldest of the 8 fresh + 2 ancient");
    assert_eq!(log_count(&conn), 6);
    // The survivors: all 5 recent plus the newest middle entry (id 6).
    assert_eq!(log_ids(&conn), (1..=6).collect::<Vec<_>>());
    teardown(conn, dir);
}

#[test]
fn empty_journal_and_no_runs_is_a_clean_noop() {
    let (conn, dir) = retention_db("empty");

    let counts = cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 1000,
            max_days: 180,
        },
        NOW_MILLIS,
    );

    assert_eq!(counts.log_entries, 0);
    assert_eq!(counts.sync_runs, 0);
    assert_eq!(log_count(&conn), 0);
    assert_eq!(run_count(&conn), 0);
    teardown(conn, dir);
}

#[test]
fn runs_beyond_five_hundred_go_and_take_their_project_rows() {
    // The card's acceptance criterion: 501 runs, each with its
    // per-project outcome row — the oldest run is deleted together
    // with its `sync_run_projects` row, leaving 500 runs and no
    // orphaned outcome rows.
    let (conn, dir) = retention_db("runs");
    seed_project(&conn, "proj-1");
    for day in 0..501 {
        // Started one day apart, ids ascending with time: id order
        // and started_at order agree.
        let run_id = format!("run-{day:03}");
        seed_run(&conn, &run_id, &ts(day, 0));
        seed_run_project(&conn, &run_id, "proj-1");
    }
    assert_eq!(run_count(&conn), 501);

    let counts = cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 1000,
            max_days: 180,
        },
        NOW_MILLIS,
    );

    assert_eq!(counts.sync_runs, 1, "only the 501st run goes");
    assert_eq!(run_count(&conn), 500);
    let survivors = run_ids(&conn);
    assert!(
        !survivors.contains(&"run-500".to_owned()),
        "the oldest run is gone"
    );
    assert_eq!(
        survivors.first().map(String::as_str),
        Some("run-000"),
        "the second-oldest run survives as the oldest"
    );
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_run_projects
              WHERE sync_run_id NOT IN (SELECT id FROM sync_runs)",
            [],
            |row| row.get(0),
        )
        .expect("counting orphaned sync_run_projects should work");
    assert_eq!(orphans, 0, "the deleted run's outcome rows went with it");
    let outcomes: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_run_projects", [], |row| {
            row.get(0)
        })
        .expect("counting sync_run_projects should work");
    assert_eq!(outcomes, 500, "exactly the surviving runs' rows remain");
    teardown(conn, dir);
}

#[test]
fn run_ties_on_started_at_break_on_id_desc() {
    // Two runs share one `started_at` at the contested boundary of
    // the keep window: 499 newer runs plus the tied pair as the
    // oldest two = 501 runs, and the kept 500 must include the
    // *higher-id* twin of the pair (the `id DESC` tiebreaker —
    // deterministic "last 500" even when the clock stalls).
    let (conn, dir) = retention_db("run-tie");
    seed_project(&conn, "proj-1");
    for day in 0..499 {
        seed_run(&conn, &format!("run-old-{day:03}"), &ts(day, 0));
    }
    // The tied pair: both started exactly 499 days back — older than
    // every run above, newer than nothing. One slot is contested.
    seed_run(&conn, "run-tie-a", &ts(499, 0));
    seed_run(&conn, "run-tie-z", &ts(499, 0));
    assert_eq!(run_count(&conn), 501);

    let counts = cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 1000,
            max_days: 3650,
        },
        NOW_MILLIS,
    );

    assert_eq!(counts.sync_runs, 1);
    let survivors = run_ids(&conn);
    assert_eq!(survivors.len(), 500);
    assert!(
        !survivors.contains(&"run-tie-a".to_owned()),
        "the lower-id twin of the tied pair must be the one deleted"
    );
    assert!(survivors.contains(&"run-tie-z".to_owned()));
    teardown(conn, dir);
}

#[test]
fn backup_records_survive_run_deletion_with_a_null_run_id() {
    // `backup_records.sync_run_id` is `ON DELETE SET NULL` (§6.2): a
    // file backup outlives its run, its row kept with the reference
    // nulled — cleanup must not lose BKP history.
    let (conn, dir) = retention_db("backup-null");
    seed_project(&conn, "proj-1");
    for day in 0..500 {
        seed_run(&conn, &format!("run-keep-{day:03}"), &ts(day, 0));
    }
    let oldest_id = "run-z-oldest";
    seed_run(&conn, oldest_id, &ts(500, 0)); // 501st, below the tie line
    seed_backup_record(&conn, "bak-1", "proj-1", oldest_id);

    let counts = cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 1000,
            max_days: 3650,
        },
        NOW_MILLIS,
    );

    assert_eq!(counts.sync_runs, 1);
    let (count, run_ref): (i64, Option<String>) = conn
        .query_row(
            "SELECT COUNT(*), MAX(sync_run_id) FROM backup_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("reading the backup record should work");
    assert_eq!(count, 1, "the backup record survives the run's deletion");
    assert_eq!(run_ref, None, "its sync_run_id is NULL now (SET NULL)");
    teardown(conn, dir);
}

#[test]
fn journal_entries_naming_a_pruned_run_keep_their_string() {
    // `action_log.sync_run_id` has no foreign key (§6.2): the journal
    // is a historical record, and an entry naming a pruned run keeps
    // the id string — what happened still names the run it happened
    // in, even though the run's own row is gone.
    let (conn, dir) = retention_db("journal-run-ref");
    seed_project(&conn, "proj-1");
    for day in 0..500 {
        seed_run(&conn, &format!("run-keep-{day:03}"), &ts(day, 0));
    }
    seed_run(&conn, "run-z-oldest", &ts(500, 0));
    let entry = seed_log_for_run(&conn, &ts(0, 0), "run-z-oldest");

    cleanup_at(
        &conn,
        &LogLimits {
            max_entries: 1000,
            max_days: 3650,
        },
        NOW_MILLIS,
    );

    let stored: Option<String> = conn
        .query_row(
            "SELECT sync_run_id FROM action_log WHERE id = ?1",
            [entry],
            |row| row.get(0),
        )
        .expect("reading the entry back should work");
    assert_eq!(
        stored.as_deref(),
        Some("run-z-oldest"),
        "the journal entry keeps its sync_run_id string after the run's deletion"
    );
    teardown(conn, dir);
}

#[test]
fn a_second_run_removes_nothing() {
    // Idempotency: the pruned state already satisfies both limits, so
    // a replay (the next startup, the next daily tick) is a no-op.
    let (conn, dir) = retention_db("idempotent");
    seed_project(&conn, "proj-1");
    for hour in 0..30 {
        seed_log(&conn, &ts(0, -hour * 3_600_000));
    }
    for day in 0..503 {
        let run_id = format!("run-{day:03}");
        seed_run(&conn, &run_id, &ts(day, 0));
        seed_run_project(&conn, &run_id, "proj-1");
    }
    let limits = LogLimits {
        max_entries: 10,
        max_days: 180,
    };
    let first = cleanup_at(&conn, &limits, NOW_MILLIS);
    assert_eq!(first.log_entries, 20);
    assert_eq!(first.sync_runs, 3);

    let second = cleanup_at(&conn, &limits, NOW_MILLIS);

    assert_eq!(
        second.log_entries, 0,
        "a replay must not delete more entries"
    );
    assert_eq!(second.sync_runs, 0, "a replay must not delete more runs");
    assert_eq!(log_count(&conn), 10);
    assert_eq!(run_count(&conn), 500);
    teardown(conn, dir);
}

#[test]
fn cleanup_joins_the_callers_transaction_and_rolls_back_with_it() {
    // The two statements run inside whatever transaction the caller
    // has open (the `insert` shape, D-037's executor): a caller that
    // rolls back keeps every row. This is the whole atomicity claim —
    // a `cleanup` that committed on its own would survive the
    // rollback and fail this test.
    let (conn, dir) = retention_db("rollback");
    seed_project(&conn, "proj-1");
    for hour in 0..8 {
        seed_log(&conn, &ts(0, -hour * 3_600_000));
    }
    for day in 0..502 {
        let run_id = format!("run-{day:03}");
        seed_run(&conn, &run_id, &ts(day, 0));
        seed_run_project(&conn, &run_id, "proj-1");
    }

    let transaction = conn
        .unchecked_transaction()
        .expect("opening a transaction should work");
    let limits = LogLimits {
        max_entries: 3,
        max_days: 180,
    };
    let now = Timestamp::from_millisecond(NOW_MILLIS).expect("a fixed instant must parse");
    let counts = cleanup(&transaction, &limits, now)
        .unwrap_or_else(|err| panic!("cleanup inside the transaction should succeed, got {err}"));
    assert_eq!(counts.log_entries, 5);
    assert_eq!(counts.sync_runs, 2);
    drop(transaction); // rusqlite's Drop sends the ROLLBACK

    assert_eq!(
        log_count(&conn),
        8,
        "the rollback restored every journal row"
    );
    assert_eq!(run_count(&conn), 502, "the rollback restored every run");
    teardown(conn, dir);
}

#[test]
fn cleanup_failure_maps_to_a_storage_sql_error() {
    // The error shape callers branch on: a failing statement surfaces
    // as `StorageError::Sql`, not a new variant (a dropped table is
    // the cheapest deterministic failure to stage).
    let (conn, dir) = retention_db("failure");
    conn.execute("DROP TABLE sync_runs", [])
        .expect("dropping sync_runs for the failure staging should work");

    let now = Timestamp::from_millisecond(NOW_MILLIS).expect("a fixed instant must parse");
    let error = cleanup(
        &conn,
        &LogLimits {
            max_entries: 1000,
            max_days: 180,
        },
        now,
    )
    .expect_err("cleanup over a missing sync_runs table must fail");

    assert!(
        matches!(error, StorageError::Sql(_)),
        "the failure must surface as StorageError::Sql, got {error:?}"
    );
    teardown(conn, dir);
}
