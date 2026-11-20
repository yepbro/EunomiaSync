//! Behavior tests for the projects repository (spec §6.2
//! `projects`/`project_exclusions`, §6.4, DB-015; task T-071).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` — the production configuration
//! (DB-001 pragmas, D-030) — in a temporary directory, and drive the
//! repository through its public API:
//!
//! * `path_key` uniqueness is the schema's reality (DB-015, the full
//!   `UNIQUE` of the migration): an active duplicate and an *archived*
//!   duplicate are both typed `DuplicatePathKey` conflicts; the
//!   exclusions table enforces its own key independently, and a
//!   project row may legally share its key with an exclusion row
//!   (the PRJ-017 archive flow's shape);
//! * `sort_order` is the DB-005 order: creation appends gapless
//!   `1..=N` over the whole table, reads order by it, `reorder`
//!   renumbers the complete id set, archive/restore keep positions;
//! * CRUD round-trips pin every column, the generated ids (UUIDv7
//!   TEXT) and timestamps (DB-002), and the created-row defaults;
//! * update writes exactly the present fields, stamps `updated_at`,
//!   and a no-op update churns nothing;
//! * archive/restore are idempotent and maintain the
//!   `archived_at ⟺ archived` invariant;
//! * by-id writers type a missing id as `NotFound`; a reorder set that
//!   is not a permutation of the table is typed `ReorderMismatch` and
//!   touches nothing; `reorder` inside an explicit transaction rolls
//!   back as a set;
//! * a stored vocabulary cell outside its closed vocabulary fails the
//!   read as the typed `Member` error (D-039; written through the
//!   `ignore_check_constraints` test seam — the CHECKs make such a
//!   cell unreachable through the API, so it can only be corruption
//!   or a newer app version), and a CHECK rejection (a §6.9 value
//!   that bypassed the validators) surfaces as `Sql`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::projects::{
    AddedVia, ExclusionReason, ListScope, NewExclusion, NewProject, Project, ProjectState,
    ProjectUpdate, ProjectsError, archive, create, create_exclusion, delete_exclusion,
    find_by_path_key, find_exclusion_by_path_key, get, list, list_exclusions, reorder, restore,
    update,
};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::storage::{open_db, run_migrations};
use rusqlite::Connection;

/// A temporary database directory removed when the guard is dropped —
/// the same teardown every sibling suite does explicitly, made
/// impossible to forget at a call site. Best-effort: a directory that
/// cannot be removed (a file still held by an exotic platform) is
/// left behind rather than failing a green test.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies) and
/// returns the connection plus the directory guard. A fresh database
/// writes no backup copy, so `db-backups/` is not created (D-038).
fn projects_db(label: &str) -> (Connection, TempDir) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-projects-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, TempDir(dir))
}

/// Asserts the canonical UUIDv7 TEXT shape of a row id (DB-002/D-031):
/// 36 characters, lowercase hyphenated, version nibble `7`.
fn assert_uuidv7_text(id: &str) {
    let bytes = id.as_bytes();
    assert_eq!(bytes.len(), 36, "id must be 36 characters: {id}");
    for index in [8, 13, 18, 23] {
        assert_eq!(bytes[index], b'-', "hyphen at byte {index} of {id}");
    }
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            continue;
        }
        assert!(
            byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
            "lowercase hex digit at byte {index} of {id}"
        );
    }
    assert_eq!(bytes[14], b'7', "version nibble of {id}");
}

/// Asserts the DB-002 shape of a timestamp cell
/// (`YYYY-MM-DDTHH:MM:SS.sssZ`, 24 characters).
fn assert_db002_shape(value: &str) {
    let bytes = value.as_bytes();
    assert_eq!(bytes.len(), 24, "timestamp must be 24 characters: {value}");
    assert_eq!(bytes[10], b'T', "date/time separator of {value}");
    assert_eq!(bytes[19], b'.', "fractional separator of {value}");
    assert_eq!(bytes[23], b'Z', "UTC marker of {value}");
}

/// A full input for one project; every call gets a distinct `path_key`
/// (`/repos/repo-{n}`), so a set of these can be created together.
/// Calling it twice with the same `n` reproduces the same identity —
/// the duplicate-path_key cases below rely on that.
fn sample(n: usize) -> NewProject<'static> {
    NewProject {
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
    }
}

/// Creates `sample(n)` and panics on failure (the error branches are
/// the tests' own subject, not this helper's).
fn seed(conn: &Connection, n: usize) -> Project {
    create(conn, &sample(n)).unwrap_or_else(|err| panic!("seed {n} should create, got {err}"))
}

/// The ids of `list(All)`, in `sort_order` order — the shape most
/// assertions below read.
fn all_ids(conn: &Connection) -> Vec<String> {
    list(conn, ListScope::All)
        .expect("list should succeed")
        .into_iter()
        .map(|project| project.id)
        .collect()
}

/// The `sort_order` values of `list(All)`.
fn all_sort_orders(conn: &Connection) -> Vec<i64> {
    list(conn, ListScope::All)
        .expect("list should succeed")
        .into_iter()
        .map(|project| project.sort_order)
        .collect()
}

// ---- Creation pins every column and the generated values ----------------------

#[test]
fn create_pins_every_column_and_the_generated_values() {
    let (conn, _dir) = projects_db("create-round-trip");
    let input = NewProject {
        added_via: AddedVia::Scan,
        scheme: Scheme::B,
        base_name: BaseName::ClaudeMd,
        extra_local: ExtraFile {
            enabled: true,
            position: Position::Start,
        },
        extra_project: ExtraFile {
            enabled: true,
            position: Position::End,
        },
        description: "a pinned description",
        ..sample(1)
    };
    let created = create(&conn, &input).expect("create should succeed");

    assert_uuidv7_text(&created.id);
    assert_db002_shape(&created.created_at);
    // One stamp for both creation columns: the row is born consistent.
    assert_eq!(created.created_at, created.updated_at);
    assert_eq!(created.path, "/repos/repo-1");
    assert_eq!(created.path_key, "/repos/repo-1");
    assert_eq!(created.name, "Project 1");
    assert_eq!(created.label, "P1");
    assert_eq!(created.color, "blue");
    assert_eq!(created.description, "a pinned description");
    assert_eq!(created.state, ProjectState::Active);
    assert_eq!(created.added_via, AddedVia::Scan);
    assert_eq!(created.sort_order, 1, "the first project starts the order");
    assert_eq!(created.scheme, Scheme::B);
    assert_eq!(created.base_name, BaseName::ClaudeMd);
    assert_eq!(
        created.extra_local,
        ExtraFile {
            enabled: true,
            position: Position::Start
        }
    );
    assert_eq!(
        created.extra_project,
        ExtraFile {
            enabled: true,
            position: Position::End
        }
    );
    assert_eq!(created.last_synced_at, None, "OUT-020: no sync has run");
    assert_eq!(created.last_synced_hash, None);
    assert_eq!(created.archived_at, None, "a new row is not archived");

    // The returned row is exactly what a fresh read returns.
    let reread = get(&conn, &created.id)
        .expect("get should succeed")
        .expect("the created row should exist");
    assert_eq!(reread, created);
}

#[test]
fn get_returns_none_for_an_unknown_id() {
    let (conn, _dir) = projects_db("get-unknown");
    assert_eq!(get(&conn, "no-such-id").expect("get should succeed"), None);
}

// ---- path_key uniqueness: the schema's actual scope (DB-015) -------------------

#[test]
fn an_active_duplicate_path_key_is_a_typed_conflict_and_the_row_survives() {
    let (conn, _dir) = projects_db("duplicate-active");
    let first = seed(&conn, 1);

    // Same identity as the seeded row: the duplicate.
    let err = create(&conn, &sample(1))
        .map(|_| ())
        .expect_err("the same path_key must conflict");
    assert!(
        matches!(err, ProjectsError::DuplicatePathKey { ref path_key }
            if path_key == "/repos/repo-1"),
        "expected DuplicatePathKey for /repos/repo-1, got {err:?}"
    );
    // The failing statement must not have disturbed the existing row
    // (SQLite's ABORT policy rolls back the statement, not more).
    let rows = list(&conn, ListScope::All).expect("list should succeed");
    assert_eq!(rows.len(), 1, "only the original row exists");
    assert_eq!(rows[0].id, first.id);
}

#[test]
fn an_archived_project_still_blocks_its_path_key() {
    // DB-015's exact wording — "unique among active and archived
    // projects" — matches the migration's full UNIQUE: an archived row
    // keeps blocking the path until it is restored. This pins the REAL
    // constraint, not a partial one.
    let (conn, _dir) = projects_db("duplicate-archived");
    let original = seed(&conn, 1);
    archive(&conn, &original.id).expect("archive should succeed");

    let err = create(&conn, &sample(1))
        .map(|_| ())
        .expect_err("an archived row must still conflict");
    assert!(
        matches!(err, ProjectsError::DuplicatePathKey { .. }),
        "expected DuplicatePathKey, got {err:?}"
    );
}

#[test]
fn find_by_path_key_returns_the_row_in_either_state_and_none_for_a_free_key() {
    let (conn, _dir) = projects_db("find-by-key");
    let active = seed(&conn, 1);
    let found = find_by_path_key(&conn, "/repos/repo-1")
        .expect("find should succeed")
        .expect("the active row should be found");
    assert_eq!(found.id, active.id);

    archive(&conn, &active.id).expect("archive should succeed");
    let archived = find_by_path_key(&conn, "/repos/repo-1")
        .expect("find should succeed")
        .expect("the archived row must still be found (the restore flow)");
    assert_eq!(archived.id, active.id);
    assert_eq!(archived.state, ProjectState::Archived);

    assert_eq!(
        find_by_path_key(&conn, "/repos/free").expect("find should succeed"),
        None,
        "an unregistered path is free"
    );
}

// ---- Exclusions: their own namespace and CRUD ---------------------------------

#[test]
fn exclusions_enforce_their_own_uniqueness_but_may_share_a_projects_key() {
    let (conn, _dir) = projects_db("exclusion-uniqueness");
    create_exclusion(
        &conn,
        &NewExclusion {
            path: "/repos/excluded",
            path_key: "/repos/excluded",
            reason: ExclusionReason::Manual,
        },
    )
    .expect("the first exclusion should create");

    let err = create_exclusion(
        &conn,
        &NewExclusion {
            path: "/repos/excluded",
            path_key: "/repos/excluded",
            reason: ExclusionReason::RemovedScanned,
        },
    )
    .map(|_| ())
    .expect_err("a second exclusion with the same key must conflict");
    assert!(matches!(err, ProjectsError::DuplicatePathKey { .. }));

    // Separate namespaces (the PRJ-017 shape): the project's row and
    // its exclusions entry carry the same path_key on purpose, and the
    // project row is untouched by the exclusion.
    let project = seed(&conn, 1);
    let shared = create_exclusion(
        &conn,
        &NewExclusion {
            path: "/repos/repo-1",
            path_key: "/repos/repo-1",
            reason: ExclusionReason::RemovedScanned,
        },
    )
    .expect("an exclusion may share a project's path_key");
    assert_eq!(shared.path_key, "/repos/repo-1");
    let untouched = get(&conn, &project.id)
        .expect("get should succeed")
        .expect("the project row should exist");
    assert_eq!(untouched, project);
}

#[test]
fn exclusion_crud_round_trips_and_physical_delete_reopens_the_path() {
    let (conn, _dir) = projects_db("exclusion-crud");
    let created = create_exclusion(
        &conn,
        &NewExclusion {
            path: "/repos/scanned",
            path_key: "/repos/scanned",
            reason: ExclusionReason::RemovedScanned,
        },
    )
    .expect("create should succeed");
    assert_uuidv7_text(&created.id);
    assert_db002_shape(&created.created_at);
    assert_eq!(created.path, "/repos/scanned");
    assert_eq!(created.path_key, "/repos/scanned");
    assert_eq!(created.reason, ExclusionReason::RemovedScanned);

    // Both vocabulary members round-trip (§6.2 CHECK).
    let manual = create_exclusion(
        &conn,
        &NewExclusion {
            path: "/repos/manual",
            path_key: "/repos/manual",
            reason: ExclusionReason::Manual,
        },
    )
    .expect("create should succeed");
    assert_eq!(manual.reason, ExclusionReason::Manual);

    let listed = list_exclusions(&conn).expect("list should succeed");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, created.id, "oldest first");
    assert_eq!(listed[1].id, manual.id);

    let found = find_exclusion_by_path_key(&conn, "/repos/manual")
        .expect("find should succeed")
        .expect("the exclusion should be found");
    assert_eq!(found.id, manual.id);
    assert_eq!(
        find_exclusion_by_path_key(&conn, "/repos/free").expect("find should succeed"),
        None
    );

    // §6.4: the one physical delete — the path is a scan candidate
    // again.
    delete_exclusion(&conn, &created.id).expect("delete should succeed");
    assert_eq!(
        list_exclusions(&conn).expect("list should succeed").len(),
        1
    );
    assert_eq!(
        find_exclusion_by_path_key(&conn, "/repos/scanned").expect("find should succeed"),
        None,
        "the deleted path is no longer excluded"
    );
}

#[test]
fn deleting_an_unknown_exclusion_is_typed_not_found() {
    let (conn, _dir) = projects_db("exclusion-delete-unknown");
    let err = delete_exclusion(&conn, "no-such-id").expect_err("an unknown id must be rejected");
    assert!(matches!(
        err,
        ProjectsError::NotFound { ref id } if id == "no-such-id"
    ));
}

// ---- sort_order: the DB-005 order ------------------------------------------------

#[test]
fn creation_appends_a_gapless_order_over_the_whole_table() {
    let (conn, _dir) = projects_db("append-order");
    for n in 1..=3 {
        let project = seed(&conn, n);
        assert_eq!(
            project.sort_order,
            i64::try_from(n).unwrap(),
            "position of {n}"
        );
    }
    assert_eq!(all_sort_orders(&conn), vec![1, 2, 3], "gapless 1..=N");
}

#[test]
fn list_orders_by_sort_order_and_honors_the_scope() {
    let (conn, _dir) = projects_db("list-scope");
    let a = seed(&conn, 1);
    let b = seed(&conn, 2);
    let c = seed(&conn, 3);

    assert_eq!(
        all_ids(&conn),
        vec![a.id.as_str(), b.id.as_str(), c.id.as_str()],
        "All is the sort_order-ordered table"
    );

    archive(&conn, &b.id).expect("archive should succeed");
    assert_eq!(
        list(&conn, ListScope::Active)
            .expect("list should succeed")
            .into_iter()
            .map(|project| project.id)
            .collect::<Vec<_>>(),
        vec![a.id, c.id],
        "the active list is the sort_order-ordered subset"
    );
    assert_eq!(
        list(&conn, ListScope::Archived)
            .expect("list should succeed")
            .into_iter()
            .map(|project| project.id)
            .collect::<Vec<_>>(),
        vec![b.id],
        "the archived screen gets the archived rows"
    );
}

#[test]
fn archive_and_restore_keep_the_rows_position() {
    let (conn, _dir) = projects_db("archive-keeps-order");
    let a = seed(&conn, 1);
    let b = seed(&conn, 2);
    assert_eq!(all_sort_orders(&conn), vec![1, 2]);

    let archived = archive(&conn, &a.id).expect("archive should succeed");
    assert_eq!(archived.state, ProjectState::Archived);
    assert_eq!(archived.sort_order, 1, "archiving does not renumber");
    assert_eq!(
        all_sort_orders(&conn),
        vec![1, 2],
        "the table-wide order is intact"
    );

    let restored = restore(&conn, &a.id).expect("restore should succeed");
    assert_eq!(restored.state, ProjectState::Active);
    assert_eq!(restored.sort_order, 1, "restoring returns the old position");
    assert_eq!(all_ids(&conn), vec![a.id, b.id]);
}

#[test]
fn reorder_renumbers_the_whole_table_to_one_through_n() {
    let (conn, _dir) = projects_db("reorder");
    let a = seed(&conn, 1);
    let b = seed(&conn, 2);
    let c = seed(&conn, 3);

    reorder(&conn, &[&c.id, &a.id, &b.id]).expect("reorder should succeed");
    assert_eq!(all_ids(&conn), vec![c.id, a.id, b.id], "the new order");
    assert_eq!(
        all_sort_orders(&conn),
        vec![1, 2, 3],
        "gapless after renumbering"
    );
}

#[test]
fn reorder_rejects_a_set_that_is_not_a_permutation_and_touches_nothing() {
    let (conn, _dir) = projects_db("reorder-mismatch");
    let a = seed(&conn, 1);
    let b = seed(&conn, 2);
    let c = seed(&conn, 3);

    // Missing a project: `missing` counts it, nothing else.
    let err = reorder(&conn, &[&a.id, &b.id]).expect_err("must be rejected");
    assert!(
        matches!(
            err,
            ProjectsError::ReorderMismatch {
                ref unknown,
                missing: 1,
                duplicates: 0
            } if unknown.is_empty()
        ),
        "expected one missing id, got {err:?}"
    );
    // An unknown id: it is reported, nothing is missing.
    let err = reorder(&conn, &[&a.id, &b.id, &c.id, &String::from("ghost")])
        .expect_err("must be rejected");
    assert!(
        matches!(
            err,
            ProjectsError::ReorderMismatch {
                ref unknown,
                missing: 0,
                duplicates: 0
            } if unknown == &vec![String::from("ghost")]
        ),
        "expected the unknown ghost id, got {err:?}"
    );
    // A duplicate entry: counted, nothing missing or unknown.
    let err = reorder(&conn, &[&a.id, &b.id, &c.id, &a.id]).expect_err("must be rejected");
    assert!(
        matches!(
            err,
            ProjectsError::ReorderMismatch {
                missing: 0,
                duplicates: 1,
                ..
            }
        ),
        "expected one duplicate, got {err:?}"
    );

    // Every rejection left the table untouched.
    assert_eq!(all_sort_orders(&conn), vec![1, 2, 3]);
}

#[test]
fn reorder_inside_a_transaction_rolls_back_as_a_set() {
    let (conn, _dir) = projects_db("reorder-rollback");
    let a = seed(&conn, 1);
    let b = seed(&conn, 2);
    let c = seed(&conn, 3);

    {
        let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
        reorder(&tx, &[&c.id, &b.id, &a.id]).expect("reorder should succeed inside the tx");
        // The change is visible inside the transaction…
        assert_eq!(
            all_ids(&tx),
            vec![c.id.as_str(), b.id.as_str(), a.id.as_str()]
        );
        // …and vanishes with the rollback (the Transaction drop).
    }
    assert_eq!(
        all_ids(&conn),
        vec![a.id, b.id, c.id],
        "the rollback erased the whole renumbering"
    );
    assert_eq!(all_sort_orders(&conn), vec![1, 2, 3]);
}

// ---- Update semantics (PRJ-008/PRJ-012) -----------------------------------------

#[test]
fn update_writes_exactly_the_present_fields_and_restamps_updated_at() {
    let (conn, _dir) = projects_db("update-partial");
    let original = seed(&conn, 1);

    // Sleeping past a millisecond boundary before the update (the
    // D-031 test precedent) makes the stamp assertion below
    // discriminating: create and update cannot land on the same
    // millisecond, so an update that stopped writing the column would
    // keep `updated_at == created_at` and fail it.
    std::thread::sleep(std::time::Duration::from_millis(2));

    let updated = update(
        &conn,
        &original.id,
        &ProjectUpdate {
            name: Some("Renamed"),
            description: Some("new description"),
            scheme: Some(Scheme::C),
            extra_local: Some(ExtraFile {
                enabled: true,
                position: Position::Start,
            }),
            ..ProjectUpdate::default()
        },
    )
    .expect("update should succeed");

    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.description, "new description");
    assert_eq!(updated.scheme, Scheme::C);
    assert_eq!(
        updated.extra_local,
        ExtraFile {
            enabled: true,
            position: Position::Start
        }
    );
    // Absent fields keep their values.
    assert_eq!(updated.label, original.label);
    assert_eq!(updated.color, original.color);
    assert_eq!(updated.base_name, original.base_name);
    assert_eq!(updated.extra_project, original.extra_project);
    // The identity and lifecycle columns are not updatable.
    assert_eq!(updated.path, original.path);
    assert_eq!(updated.path_key, original.path_key);
    assert_eq!(updated.added_via, original.added_via);
    assert_eq!(updated.state, ProjectState::Active);
    assert_eq!(updated.sort_order, original.sort_order);
    assert_eq!(updated.created_at, original.created_at);
    // The stamp moved forward (DB-002 text order equals time order for
    // the fixed-width shape; the sleep above guarantees the update
    // cannot legitimately reuse the creation millisecond).
    assert!(
        updated.updated_at.as_str() > updated.created_at.as_str(),
        "updated_at {} must be strictly later than created_at {}",
        updated.updated_at,
        updated.created_at
    );
}

#[test]
fn a_no_op_update_changes_nothing_including_the_stamp() {
    let (conn, _dir) = projects_db("update-noop");
    let original = seed(&conn, 1);

    let unchanged = update(&conn, &original.id, &ProjectUpdate::default())
        .expect("a fieldless update should succeed");
    assert_eq!(unchanged, original, "identical row: no churn, no restamp");
}

#[test]
fn updating_an_unknown_id_is_typed_not_found() {
    let (conn, _dir) = projects_db("update-unknown");
    let err = update(
        &conn,
        "no-such-id",
        &ProjectUpdate {
            name: Some("x"),
            ..ProjectUpdate::default()
        },
    )
    .expect_err("an unknown id must be rejected");
    assert!(matches!(
        err,
        ProjectsError::NotFound { ref id } if id == "no-such-id"
    ));
}

// ---- Archive/restore lifecycle ----------------------------------------------------

#[test]
fn archive_stamps_archived_at_and_restore_clears_it() {
    let (conn, _dir) = projects_db("archive-timestamps");
    let project = seed(&conn, 1);

    let archived = archive(&conn, &project.id).expect("archive should succeed");
    assert_eq!(archived.state, ProjectState::Archived);
    let stamp = archived.archived_at.expect("archived_at must be set");
    assert_db002_shape(&stamp);

    let restored = restore(&conn, &project.id).expect("restore should succeed");
    assert_eq!(restored.state, ProjectState::Active);
    assert_eq!(
        restored.archived_at, None,
        "archived_at is not NULL only while archived"
    );
}

#[test]
fn archive_and_restore_are_idempotent_without_timestamp_drift() {
    let (conn, _dir) = projects_db("lifecycle-idempotent");
    let project = seed(&conn, 1);

    let first = archive(&conn, &project.id).expect("archive should succeed");
    let second = archive(&conn, &project.id).expect("a repeated archive should succeed");
    assert_eq!(
        second, first,
        "no restamp: the archived row is returned as-is"
    );

    let restored = restore(&conn, &project.id).expect("restore should succeed");
    let again = restore(&conn, &project.id).expect("a repeated restore should succeed");
    assert_eq!(again, restored, "no churn on the active row");
}

#[test]
fn archiving_or_restoring_an_unknown_id_is_typed_not_found() {
    let (conn, _dir) = projects_db("lifecycle-unknown");
    for err in [
        archive(&conn, "no-such-id").expect_err("must be rejected"),
        restore(&conn, "no-such-id").expect_err("must be rejected"),
    ] {
        assert!(matches!(
            err,
            ProjectsError::NotFound { ref id } if id == "no-such-id"
        ));
    }
}

/// The escaped SQL literal of an id (UUIDv7 TEXT: nothing to escape
/// today, but the helper keeps the seam honest if that ever changes).
fn sql_literal(id: &str) -> String {
    id.replace('\'', "''")
}

/// Writes one vocabulary-violating cell through the SQLite test seam
/// (`ignore_check_constraints`) — the simulation of the D-039
/// corruption/newer-version scenario the CHECKs otherwise make
/// unreachable.
fn corrupt_cell(conn: &Connection, update_sql: &str) {
    conn.execute_batch(&format!(
        "PRAGMA ignore_check_constraints = 1;\n{update_sql};\nPRAGMA ignore_check_constraints = 0;"
    ))
    .expect("the corruption should apply");
}

// ---- The schema floor and corrupt cells -------------------------------------------

#[test]
fn a_check_rejection_surfaces_as_sql_when_the_validators_are_bypassed() {
    // The §6.9 field rules belong to eunomia-core; the store's floor is
    // the schema CHECK, which must not let a bypassed value through.
    let (conn, _dir) = projects_db("check-floor");
    let mut input = sample(1);
    input.name = Box::leak("x".repeat(101).into_boxed_str());
    let err = create(&conn, &input).expect_err("a too-long name must be rejected");
    assert!(
        matches!(err, ProjectsError::Sql(_)),
        "expected the raw SQL (CHECK) error, got {err:?}"
    );
    assert_eq!(
        list(&conn, ListScope::All)
            .expect("list should succeed")
            .len(),
        0,
        "the rejected row left nothing behind"
    );
}

#[test]
fn a_corrupt_vocabulary_cell_fails_the_read_as_typed_member() {
    // The CHECKs make a vocabulary-violating cell unreachable through
    // the API — such a cell means corruption or a newer app version
    // (D-039). The test seam is the SQLite escape hatch for exactly
    // that simulation: `ignore_check_constraints`.
    let (conn, _dir) = projects_db("corrupt-cell");
    let project = seed(&conn, 1);

    corrupt_cell(
        &conn,
        &format!(
            "UPDATE projects SET state = 'bogus' WHERE id = '{}'",
            sql_literal(&project.id)
        ),
    );

    let err = get(&conn, &project.id)
        .map(|_| ())
        .expect_err("a corrupt state cell must fail the read");
    assert!(
        matches!(
            err,
            ProjectsError::Member {
                column: "state",
                ..
            }
        ),
        "expected Member(state), got {err:?}"
    );
    let err = list(&conn, ListScope::All)
        .map(|_| ())
        .expect_err("list must fail on the corrupt cell too");
    assert!(matches!(
        err,
        ProjectsError::Member {
            column: "state",
            ..
        }
    ));

    // A second vocabulary column, to pin the decode is per-column.
    corrupt_cell(
        &conn,
        &format!(
            "UPDATE projects SET state = 'active', added_via = 'bogus' WHERE id = '{}'",
            sql_literal(&project.id)
        ),
    );
    let err = get(&conn, &project.id)
        .map(|_| ())
        .expect_err("a corrupt added_via cell must fail the read");
    assert!(matches!(
        err,
        ProjectsError::Member {
            column: "added_via",
            ..
        }
    ));

    // And the exclusions side.
    let exclusion = create_exclusion(
        &conn,
        &NewExclusion {
            path: "/repos/x",
            path_key: "/repos/x",
            reason: ExclusionReason::Manual,
        },
    )
    .expect("create should succeed");
    corrupt_cell(
        &conn,
        &format!(
            "UPDATE project_exclusions SET reason = 'bogus' WHERE id = '{}'",
            sql_literal(&exclusion.id)
        ),
    );
    let err = list_exclusions(&conn)
        .map(|_| ())
        .expect_err("a corrupt reason cell must fail the read");
    assert!(matches!(
        err,
        ProjectsError::Member {
            column: "reason",
            ..
        }
    ));
}

// ---- Composition contracts (append after reorder/archives, transaction reuse) ----

#[test]
fn a_failed_statement_inside_a_transaction_keeps_it_usable() {
    // create's error docs promise SQLite's ABORT policy: the failing
    // statement rolls back alone and the surrounding transaction stays
    // usable. A change of that policy (e.g. to a full ROLLBACK) would
    // silently break every composing command task.
    let (conn, _dir) = projects_db("abort-keeps-transaction");
    let first = seed(&conn, 1);

    let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
    create(&tx, &sample(1))
        .map(|_| ())
        .expect_err("the duplicate must conflict inside the transaction too");
    // The same transaction goes on to do real work, and the work
    // commits.
    let second = create(&tx, &sample(2)).expect("the transaction must stay usable");
    tx.commit().expect("commit should succeed");

    assert_eq!(
        all_ids(&conn),
        vec![first.id.as_str(), second.id.as_str()],
        "the committed work survived the earlier failed statement"
    );
}

#[test]
fn creation_appends_after_a_reorder() {
    // The append is MAX+1 over the whole table — it must continue the
    // sequence the renumbering produced, not restart from the row
    // count or anything positional.
    let (conn, _dir) = projects_db("append-after-reorder");
    let a = seed(&conn, 1);
    let b = seed(&conn, 2);
    let c = seed(&conn, 3);
    reorder(&conn, &[&c.id, &a.id, &b.id]).expect("reorder should succeed");

    let fourth = seed(&conn, 4);
    assert_eq!(fourth.sort_order, 4, "appended after the reordered table");
    assert_eq!(
        all_ids(&conn),
        vec![
            c.id.as_str(),
            a.id.as_str(),
            b.id.as_str(),
            fourth.id.as_str()
        ]
    );
    assert_eq!(all_sort_orders(&conn), vec![1, 2, 3, 4], "gapless");
}

#[test]
fn creation_appends_with_an_archived_row_in_the_middle() {
    // Archived rows keep their positions, so MAX+1 must skip over
    // them — no position reuse, no gap.
    let (conn, _dir) = projects_db("append-after-archive");
    seed(&conn, 1);
    let middle = seed(&conn, 2);
    seed(&conn, 3);
    archive(&conn, &middle.id).expect("archive should succeed");

    let fourth = seed(&conn, 4);
    assert_eq!(fourth.sort_order, 4);
    assert_eq!(
        all_sort_orders(&conn),
        vec![1, 2, 3, 4],
        "archived row keeps 2"
    );
    assert_eq!(
        list(&conn, ListScope::Active)
            .expect("list should succeed")
            .into_iter()
            .map(|project| project.sort_order)
            .collect::<Vec<_>>(),
        vec![1, 3, 4],
        "the active subset is order-preserving but not contiguous"
    );
}

#[test]
fn an_update_that_writes_the_same_values_still_succeeds() {
    // `changed == 0` must mean "no such id", not "nothing differed":
    // a same-value update (PRJ-008 saves the whole modal) is a real
    // write and must not be typed NotFound.
    let (conn, _dir) = projects_db("update-same-value");
    let original = seed(&conn, 1);

    let updated = update(
        &conn,
        &original.id,
        &ProjectUpdate {
            name: Some(original.name.as_str()),
            ..ProjectUpdate::default()
        },
    )
    .expect("a same-value update should succeed");
    assert_eq!(updated.name, original.name);
    assert_eq!(updated.label, original.label);
}
