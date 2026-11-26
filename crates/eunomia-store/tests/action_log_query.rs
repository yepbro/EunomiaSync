//! Behavior tests for the "Activity" query (spec §7.7 UI row, LOG-005;
//! task T-027) — [`eunomia_store::action_log::list`].
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` (the production configuration,
//! DB-001 pragmas, D-030) in a temporary directory, and drive the
//! query through its public API:
//!
//! * each filter alone, and the filters combined: date range (both
//!   bounds **inclusive**, pinned by entries stamped exactly at the
//!   bounds), project, actions, result, visibility;
//! * pagination: pages do not overlap, their union is the filtered
//!   set, `total` counts the filters' matches (not the page), an
//!   offset past the end is an honest empty page;
//! * determinism: the order is `ts DESC, id DESC` — an equal-`ts`
//!   block pages stably by id, and an entry whose `ts` contradicts
//!   its insert order sorts by `ts` (time is the primary key of the
//!   window, id only the tiebreaker);
//! * LOG-005's invariant at the store level: a `visibility = user`
//!   filter never returns a `diagnostic` entry, whatever the other
//!   filters narrow to;
//! * the boundary contract: timestamps that are not the exact DB-002
//!   shape are rejected (the lexicographic-ordering precondition),
//!   an inverted range is rejected, and filter values that look like
//!   SQL never change the query's shape (they are bound parameters,
//!   matched literally).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::action_log::{
    Action, ActionLogError, ActivityPage, ActivityQuery, Details, Entity, EntityType, FailureCode,
    Level, LogEntry, LogRecord, Outcome, Visibility, insert, list,
};
use eunomia_store::storage::{StorageError, open_db, run_migrations};
use rusqlite::Connection;

/// A fixed mid-day DB-002 timestamp the date fixtures build around.
const BASE_TS: &str = "2026-11-10T12:00:00.000Z";

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies) and
/// returns the connection plus the directory for teardown (the same
/// helper shape as the T-026 suite).
fn journal_db(label: &str) -> (Connection, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-activity-query-{label}-{}-{nanos}",
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

/// The row of an entry as `list` returns it: id plus action, enough
/// for order and membership assertions without noise.
fn ids(page: &ActivityPage) -> Vec<i64> {
    page.records.iter().map(|record| record.id).collect()
}

/// An unfiltered query for one page of up to `limit` entries.
fn page_query(limit: u32) -> ActivityQuery {
    ActivityQuery {
        from_ts: None,
        to_ts: None,
        project_id: None,
        actions: Vec::new(),
        result: None,
        visibility: None,
        limit,
        offset: 0,
    }
}

/// Inserts one entry with the store's own API and then pins its `ts`
/// cell to `ts` — the timestamp is `insert`'s own stamp (DB-002,
/// D-051) and the read filters must be testable against *chosen*
/// times, so the fixture sets the one column the API owns itself.
/// Returns the entry's autoincrement id.
fn seed(
    conn: &Connection,
    ts: &str,
    action: Action,
    visibility: Visibility,
    result: Outcome,
    project_id: Option<&str>,
) -> i64 {
    let mut entry = LogEntry::new(action, Level::Info, visibility, result);
    entry.project_id = project_id;
    let id = insert(conn, &entry).expect("insert should succeed");
    conn.execute(
        "UPDATE action_log SET ts = ?1 WHERE id = ?2",
        rusqlite::params![ts, id],
    )
    .expect("pinning the fixture ts should succeed");
    id
}

/// Reads `list` and unwraps, rendering failures through the panic.
fn query(conn: &Connection, query: &ActivityQuery) -> ActivityPage {
    list(conn, query).unwrap_or_else(|err| panic!("list should succeed, got {err}"))
}

// ---------------------------------------------------------------------------
// Filters, one at a time
// ---------------------------------------------------------------------------

#[test]
fn the_date_bounds_are_inclusive() {
    let (conn, dir) = journal_db("date-bounds");
    let before = seed(
        &conn,
        "2026-11-09T23:59:59.999Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let at_from = seed(
        &conn,
        "2026-11-10T00:00:00.000Z",
        Action::ProjectChanged,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let mid = seed(
        &conn,
        BASE_TS,
        Action::ProjectArchived,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let at_to = seed(
        &conn,
        "2026-11-10T23:59:59.999Z",
        Action::ProjectRestored,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let after = seed(
        &conn,
        "2026-11-11T00:00:00.000Z",
        Action::SettingsChanged,
        Visibility::User,
        Outcome::Ok,
        None,
    );

    // The whole day of Nov 10: both bounds land inside.
    let mut bounds = page_query(10);
    bounds.from_ts = Some("2026-11-10T00:00:00.000Z".to_owned());
    bounds.to_ts = Some("2026-11-10T23:59:59.999Z".to_owned());
    let page = query(&conn, &bounds);
    assert_eq!(
        ids(&page),
        vec![at_to, mid, at_from],
        "both bounds inclusive"
    );
    assert_eq!(page.total, 3);

    // from-only: everything from the lower bound on, `after` included.
    let mut from_only = page_query(10);
    from_only.from_ts = Some("2026-11-10T00:00:00.000Z".to_owned());
    assert_eq!(
        ids(&query(&conn, &from_only)),
        vec![after, at_to, mid, at_from]
    );

    // to-only: everything up to the upper bound, `before` included.
    let mut to_only = page_query(10);
    to_only.to_ts = Some("2026-11-10T00:00:00.000Z".to_owned());
    assert_eq!(ids(&query(&conn, &to_only)), vec![at_from, before]);

    // A single-millisecond range: an entry stamped exactly at it is
    // the whole result.
    let mut instant = page_query(10);
    instant.from_ts = Some(BASE_TS.to_owned());
    instant.to_ts = Some(BASE_TS.to_owned());
    let page = query(&conn, &instant);
    assert_eq!(ids(&page), vec![mid]);
    assert_eq!(page.total, 1);
    teardown(conn, dir);
}

#[test]
fn timestamps_outside_the_db002_shape_are_rejected_not_guessed() {
    let (conn, dir) = journal_db("ts-shape");
    seed(
        &conn,
        BASE_TS,
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );

    // Each of these parses as RFC 3339 (or is a plain date) yet none
    // has the fixed shape whose text order equals chronological order
    // — a silent text comparison over them would mis-order, so the
    // query rejects them (the `values::is_db002_shape` precondition).
    for (field, bad) in [
        ("from_ts", "2026-11-10T00:00:00Z"),
        ("from_ts", "2026-11-10"),
        ("from_ts", "2026-11-10T00:00:00.000+00:00"),
        ("to_ts", "2026-11-10T00:00:00.0Z"),
        ("to_ts", "not a timestamp at all"),
    ] {
        let mut bounds = page_query(10);
        if field == "from_ts" {
            bounds.from_ts = Some(bad.to_owned());
        } else {
            bounds.to_ts = Some(bad.to_owned());
        }
        match list(&conn, &bounds) {
            Err(StorageError::ActionLog {
                source:
                    ActionLogError::Timestamp {
                        field: column,
                        found,
                    },
            }) => {
                assert_eq!(column, field, "the error names the offending bound");
                assert_eq!(found, bad, "the error carries the rejected value");
            }
            other => panic!("a {bad:?} {field} must be rejected, got {other:?}"),
        }
    }
    teardown(conn, dir);
}

#[test]
fn an_inverted_date_range_is_rejected() {
    let (conn, dir) = journal_db("inverted");
    let mut bounds = page_query(10);
    bounds.from_ts = Some("2026-11-11T00:00:00.000Z".to_owned());
    bounds.to_ts = Some("2026-11-10T00:00:00.000Z".to_owned());
    match list(&conn, &bounds) {
        Err(StorageError::ActionLog {
            source: ActionLogError::Range { from, to },
        }) => {
            assert_eq!(from, "2026-11-11T00:00:00.000Z");
            assert_eq!(to, "2026-11-10T00:00:00.000Z");
        }
        other => panic!("an inverted range must be rejected, got {other:?}"),
    }
    teardown(conn, dir);
}

#[test]
fn the_project_filter_matches_exactly_one_id() {
    let (conn, dir) = journal_db("project");
    let alpha = "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f";
    let beta = "0193a2bb-2222-7333-9c6d-1a2b3c4d5e6f";
    let a1 = seed(
        &conn,
        BASE_TS,
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some(alpha),
    );
    let b1 = seed(
        &conn,
        "2026-11-10T12:00:00.001Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some(beta),
    );
    let no_project = seed(
        &conn,
        "2026-11-10T12:00:00.002Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let _ = (b1, no_project);

    let mut by_project = page_query(10);
    by_project.project_id = Some(alpha.to_owned());
    let page = query(&conn, &by_project);
    assert_eq!(ids(&page), vec![a1]);
    assert_eq!(page.total, 1);

    // A project id that matches nothing is an honest empty page.
    by_project.project_id = Some("0193a2bb-9999-7333-9c6d-1a2b3c4d5e6f".to_owned());
    let page = query(&conn, &by_project);
    assert_eq!(page.records.len(), 0);
    assert_eq!(page.total, 0);
    teardown(conn, dir);
}

#[test]
fn filter_values_never_change_the_querys_shape() {
    let (conn, dir) = journal_db("injection");
    let hostile = "x' OR '1'='1"; // classic SQL-injection payload as a project id
    let innocent = seed(
        &conn,
        BASE_TS,
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some("innocent"),
    );
    let hostile_row = seed(
        &conn,
        "2026-11-10T12:00:00.001Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some(hostile),
    );

    // Filtering by the hostile string selects exactly the row that
    // carries it as a literal value — the bound parameter matched the
    // column, the quote did not terminate anything.
    let mut by_hostile = page_query(10);
    by_hostile.project_id = Some(hostile.to_owned());
    let page = query(&conn, &by_hostile);
    assert_eq!(ids(&page), vec![hostile_row]);
    assert_eq!(page.total, 1);

    // And it does not widen any other filter: the injection-shaped
    // *action* string matches nothing (it is no member) instead of
    // matching everything.
    let mut by_hostile_action = page_query(10);
    by_hostile_action.actions = vec![Action::from_member("project.added").expect("a member")];
    by_hostile_action.project_id = Some("' OR 1=1 --".to_owned());
    let page = query(&conn, &by_hostile_action);
    assert_eq!(page.total, 0, "a bound parameter cannot widen the filter");
    let _ = innocent;
    teardown(conn, dir);
}

#[test]
fn the_action_filter_matches_any_of_the_selected_actions() {
    let (conn, dir) = journal_db("actions");
    let added = seed(
        &conn,
        BASE_TS,
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let changed = seed(
        &conn,
        "2026-11-10T12:00:00.001Z",
        Action::ProjectChanged,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let archived = seed(
        &conn,
        "2026-11-10T12:00:00.002Z",
        Action::ProjectArchived,
        Visibility::User,
        Outcome::Ok,
        None,
    );

    // A subset: only the selected actions, newest first.
    let mut subset = page_query(10);
    subset.actions = vec![Action::ProjectAdded, Action::ProjectArchived];
    assert_eq!(ids(&query(&conn, &subset)), vec![archived, added]);

    // An empty action list is no filter at all — every entry.
    let all = query(&conn, &page_query(10));
    assert_eq!(ids(&all), vec![archived, changed, added]);
    assert_eq!(all.total, 3);
    teardown(conn, dir);
}

#[test]
fn the_result_filter_matches_each_outcome() {
    let (conn, dir) = journal_db("result");
    let ok_row = seed(
        &conn,
        BASE_TS,
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let error_row = seed(
        &conn,
        "2026-11-10T12:00:00.001Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Error,
        None,
    );
    let skipped_row = seed(
        &conn,
        "2026-11-10T12:00:00.002Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Skipped,
        None,
    );

    for (result, expected) in [
        (Outcome::Ok, vec![ok_row]),
        (Outcome::Error, vec![error_row]),
        (Outcome::Skipped, vec![skipped_row]),
    ] {
        let mut by_result = page_query(10);
        by_result.result = Some(result);
        let page = query(&conn, &by_result);
        assert_eq!(ids(&page), expected, "the {result:?} filter");
        assert_eq!(page.total, 1);
    }
    teardown(conn, dir);
}

// ---------------------------------------------------------------------------
// The LOG-005 invariant (the card's acceptance criterion, store half)
// ---------------------------------------------------------------------------

#[test]
fn a_user_visibility_filter_never_returns_diagnostic_entries() {
    let (conn, dir) = journal_db("visibility-invariant");
    // Pairs of user/diagnostic entries stamped identically apart from
    // visibility, across several actions, projects and results — a
    // diagnostic twin exists for every filter combination below.
    let mut expected_user_ids = Vec::new();
    for (index, (action, project)) in [
        (
            Action::ProjectAdded,
            Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f"),
        ),
        (
            Action::SyncProjectResult,
            Some("0193a2bb-2222-7333-9c6d-1a2b3c4d5e6f"),
        ),
        (Action::BackupCreated, None),
    ]
    .into_iter()
    .enumerate()
    {
        let ts = format!("2026-11-10T12:00:{index:02}.000Z");
        let result = if index == 1 {
            Outcome::Error
        } else {
            Outcome::Ok
        };
        expected_user_ids.push(seed(&conn, &ts, action, Visibility::User, result, project));
        seed(&conn, &ts, action, Visibility::Diagnostic, result, project);
    }
    expected_user_ids.reverse(); // newest first

    // The invariant holds for the bare user filter…
    let mut user_only = page_query(10);
    user_only.visibility = Some(Visibility::User);
    let page = query(&conn, &user_only);
    assert_eq!(ids(&page), expected_user_ids);
    assert_eq!(page.total, 3);

    // …and it holds under every other filter narrowing the same rows:
    // a diagnostic entry matching the filter must still stay hidden.
    let mut dated = user_only.clone();
    dated.from_ts = Some("2026-11-10T00:00:00.000Z".to_owned());
    assert_eq!(query(&conn, &dated).total, 3, "the date filter");

    let mut by_project = user_only.clone();
    by_project.project_id = Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned());
    assert_eq!(query(&conn, &by_project).total, 1, "the project filter");

    let mut by_action = user_only.clone();
    by_action.actions = vec![Action::BackupCreated];
    assert_eq!(query(&conn, &by_action).total, 1, "the action filter");

    let mut by_result = user_only.clone();
    by_result.result = Some(Outcome::Error);
    assert_eq!(query(&conn, &by_result).total, 1, "the result filter");

    let mut combined = user_only;
    combined.from_ts = Some("2026-11-10T00:00:00.000Z".to_owned());
    combined.to_ts = Some("2026-11-10T23:59:59.999Z".to_owned());
    combined.actions = vec![
        Action::ProjectAdded,
        Action::SyncProjectResult,
        Action::BackupCreated,
    ];
    combined.result = Some(Outcome::Error);
    assert_eq!(
        query(&conn, &combined).total,
        1,
        "every filter combined: only the user twin survives"
    );

    // While `None` (the diagnostics section's query) sees both.
    let mut both = page_query(10);
    both.visibility = None;
    assert_eq!(query(&conn, &both).total, 6);
    teardown(conn, dir);
}

// ---------------------------------------------------------------------------
// Filters combined
// ---------------------------------------------------------------------------

#[test]
fn the_filters_combine_with_and() {
    let (conn, dir) = journal_db("combined");
    let alpha = "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f";
    let beta = "0193a2bb-2222-7333-9c6d-1a2b3c4d5e6f";
    // Four entries walking one field each away from the first.
    let match_all = seed(
        &conn,
        "2026-11-10T10:00:00.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some(alpha),
    );
    let wrong_project = seed(
        &conn,
        "2026-11-10T10:00:00.001Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some(beta),
    );
    let wrong_action = seed(
        &conn,
        "2026-11-10T10:00:00.002Z",
        Action::ProjectChanged,
        Visibility::User,
        Outcome::Ok,
        Some(alpha),
    );
    let wrong_result = seed(
        &conn,
        "2026-11-10T10:00:00.003Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Error,
        Some(alpha),
    );
    let out_of_range = seed(
        &conn,
        "2026-11-11T10:00:00.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        Some(alpha),
    );
    let _ = (wrong_project, wrong_action, wrong_result, out_of_range);

    let mut combined = ActivityQuery {
        from_ts: Some("2026-11-10T00:00:00.000Z".to_owned()),
        to_ts: Some("2026-11-10T23:59:59.999Z".to_owned()),
        project_id: Some(alpha.to_owned()),
        actions: vec![Action::ProjectAdded],
        result: Some(Outcome::Ok),
        visibility: Some(Visibility::User),
        limit: 10,
        offset: 0,
    };
    let page = query(&conn, &combined);
    assert_eq!(ids(&page), vec![match_all]);
    assert_eq!(
        page.total, 1,
        "AND semantics: one entry satisfies all filters"
    );

    // Dropping one filter at a time grows the result by exactly the
    // entry that filter excluded.
    combined.project_id = None;
    assert_eq!(
        query(&conn, &combined).total,
        2,
        "without the project filter"
    );
    combined.actions = Vec::new();
    assert_eq!(
        query(&conn, &combined).total,
        3,
        "without the action filter"
    );
    combined.result = None;
    assert_eq!(
        query(&conn, &combined).total,
        4,
        "without the result filter"
    );
    combined.to_ts = None;
    assert_eq!(
        query(&conn, &combined).total,
        5,
        "without the upper date bound"
    );
    teardown(conn, dir);
}

// ---------------------------------------------------------------------------
// Pagination and deterministic ordering
// ---------------------------------------------------------------------------

#[test]
fn pages_do_not_overlap_and_cover_the_whole_filtered_set() {
    let (conn, dir) = journal_db("pagination");
    let mut expected = Vec::new();
    for index in 0..7 {
        expected.push(seed(
            &conn,
            &format!("2026-11-10T12:00:{index:02}.000Z"),
            Action::ProjectAdded,
            Visibility::User,
            Outcome::Ok,
            None,
        ));
    }
    expected.reverse(); // newest first

    // Three pages of three over seven entries.
    let mut seen = Vec::new();
    for offset in [0, 3, 6] {
        let mut page = page_query(3);
        page.offset = offset;
        let result = query(&conn, &page);
        seen.extend(ids(&result));
        assert_eq!(result.total, 7, "total is the filtered count, not the page");
    }
    assert_eq!(seen, expected, "the pages tile the set, newest first");
    assert_eq!(seen.len(), 7, "no entry appears twice");

    // An offset past the end is an honest empty page, not an error.
    let mut past_end = page_query(3);
    past_end.offset = 60;
    let result = query(&conn, &past_end);
    assert_eq!(result.records.len(), 0);
    assert_eq!(result.total, 7, "the count is independent of the offset");

    // A limit larger than the set returns everything in one page.
    assert_eq!(ids(&query(&conn, &page_query(100))), expected);
    teardown(conn, dir);
}

#[test]
fn equal_ts_entries_page_stably_by_the_id_tiebreaker() {
    let (conn, dir) = journal_db("equal-ts");
    // Six entries sharing one timestamp: only the id tiebreaker can
    // order them, and it must order them the same way on every page.
    let mut expected = Vec::new();
    for _ in 0..6 {
        expected.push(seed(
            &conn,
            BASE_TS,
            Action::ProjectAdded,
            Visibility::User,
            Outcome::Ok,
            None,
        ));
    }
    expected.reverse(); // id DESC inside the equal-ts block

    let mut seen = Vec::new();
    for offset in [0, 2, 4] {
        let mut page = page_query(2);
        page.offset = offset;
        seen.extend(ids(&query(&conn, &page)));
    }
    assert_eq!(seen, expected, "the tiebreaker is stable across pages");
    teardown(conn, dir);
}

#[test]
fn ts_is_the_primary_order_key_the_id_only_breaks_ties() {
    let (conn, dir) = journal_db("ts-primary");
    // Insert order deliberately contradicts ts order: the third insert
    // carries the newest timestamp. The window sorts by time, so it
    // leads; `recent` (insert order) would put it last — the two seams
    // differ exactly here, and this pins which is which.
    let oldest = seed(
        &conn,
        "2026-11-10T12:00:00.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let middle = seed(
        &conn,
        "2026-11-10T12:00:01.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let newest = seed(
        &conn,
        "2026-11-10T12:00:02.000Z",
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );
    let _ = (middle, oldest);

    let page = query(&conn, &page_query(10));
    assert_eq!(
        page.records
            .iter()
            .map(|record| record.ts.clone())
            .collect::<Vec<_>>(),
        vec![
            "2026-11-10T12:00:02.000Z".to_owned(),
            "2026-11-10T12:00:01.000Z".to_owned(),
            "2026-11-10T12:00:00.000Z".to_owned(),
        ],
        "time is the primary order key"
    );
    assert_eq!(page.records[0].id, newest);
    teardown(conn, dir);
}

// ---------------------------------------------------------------------------
// The read-side contract shared with `recent`
// ---------------------------------------------------------------------------

#[test]
fn corrupt_cells_fail_the_list_read_explicitly() {
    let (conn, dir) = journal_db("corrupt-cells");
    seed(
        &conn,
        BASE_TS,
        Action::ProjectAdded,
        Visibility::User,
        Outcome::Ok,
        None,
    );

    // An action outside the closed vocabulary.
    conn.execute("UPDATE action_log SET action = 'project.exploded'", [])
        .expect("tampering should succeed");
    match list(&conn, &page_query(10)) {
        Err(StorageError::ActionLog {
            source: ActionLogError::Member { column, found },
        }) => {
            assert_eq!(column, "action");
            assert_eq!(found, "project.exploded");
        }
        other => panic!("an unknown action must fail the read, got {other:?}"),
    }

    // A `details` cell that is not JSON — even when the corrupt row is
    // outside the requested page, the whole read fails (the corrupt-
    // cells policy is about the rows the query returns; the row here
    // is the only one, so it is in the page).
    conn.execute(
        "UPDATE action_log SET action = 'project.added', details = 'not json'",
        [],
    )
    .expect("tampering should succeed");
    match list(&conn, &page_query(10)) {
        Err(StorageError::ActionLog {
            source: ActionLogError::Json(_),
        }) => {}
        other => panic!("a non-JSON details cell must fail the read, got {other:?}"),
    }

    // A `ts` cell outside the DB-002 shape: `ts` is the column the
    // ordering and range semantics stand on (text comparison equals
    // chronological comparison only for the exact shape), so a
    // shapeless stored value is a corrupt cell the read must surface —
    // not silently sort textually (the D-039 policy, extended to `ts`;
    // the cell variant is distinct from the bound variant so the
    // command layer can tell stored corruption from caller input).
    conn.execute(
        "UPDATE action_log SET details = NULL, ts = '2026-11-10T00:00:00Z'",
        [],
    )
    .expect("tampering should succeed");
    match list(&conn, &page_query(10)) {
        Err(StorageError::ActionLog {
            source: ActionLogError::TsCell { found },
        }) => {
            assert_eq!(found, "2026-11-10T00:00:00Z");
        }
        other => panic!("a shapeless ts cell must fail the read, got {other:?}"),
    }
    teardown(conn, dir);
}

#[test]
fn the_page_round_trips_every_column_of_a_full_entry() {
    let (conn, dir) = journal_db("roundtrip");
    let mut entry = LogEntry::new(
        Action::SyncProjectResult,
        Level::Warn,
        Visibility::Diagnostic,
        Outcome::Error,
    );
    entry.entity = Some(Entity {
        kind: EntityType::Project,
        id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f",
    });
    entry.project_id = Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f");
    entry.sync_run_id = Some("0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f");
    entry.details = Some(Details::Failure {
        code: FailureCode::new("E_SYNC_RUNNING").expect("the registry shape holds"),
        path: None,
        os_code: None,
    });
    let id = insert(&conn, &entry).expect("insert should succeed");

    // Ask for everything (visibility None = both audiences).
    let page = query(&conn, &page_query(10));
    assert_eq!(page.records.len(), 1);
    let record: &LogRecord = &page.records[0];
    assert_eq!(record.id, id);
    assert_eq!(record.level, Level::Warn);
    assert_eq!(record.visibility, Visibility::Diagnostic);
    assert_eq!(record.action, Action::SyncProjectResult);
    assert_eq!(
        record.entity,
        Some((
            EntityType::Project,
            "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned()
        ))
    );
    assert_eq!(
        record.project_id,
        Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f".to_owned())
    );
    assert_eq!(
        record.sync_run_id,
        Some("0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f".to_owned())
    );
    assert_eq!(record.result, Outcome::Error);
    assert_eq!(
        record.details,
        Some(serde_json::json!({ "code": "E_SYNC_RUNNING" }))
    );
    teardown(conn, dir);
}
