//! Behavior tests for the action-log repository (spec §7.7; task T-026).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` — the production configuration
//! (DB-001 pragmas, D-030) — in a temporary directory, and drive the
//! repository through its public API:
//!
//! * `insert` → `recent` round-trips every field, with the DB-002
//!   timestamp shape pinned;
//! * an entry written inside an operation's transaction rolls back
//!   with it, and the fail-then-log error entry written in the next
//!   transaction survives — both halves of the journal's
//!   transactional semantics;
//! * `user` and `diagnostic` entries round-trip separately (LOG-005);
//! * the T-245 exclusion-removal shape round-trips with its entity
//!   and path, and rows the T-078 era wrote (`project.changed`, no
//!   entity — D-106's frozen-vocabulary fit) still read unchanged
//!   beside it;
//! * a tampered cell — a non-JSON `details`, an unknown action —
//!   fails the read explicitly (the D-039 corrupt-cells policy);
//! * the SEC-008 contract at the database boundary: the `details`
//!   cell of an entry built beside a realistic instruction text
//!   carries none of that text.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::action_log::{
    Action, ActionLogError, Details, Entity, EntityType, FailureCode, Level, LogEntry, Outcome,
    Visibility, insert, recent,
};
use eunomia_store::storage::{StorageError, open_db, run_migrations};
use rusqlite::Connection;

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies) and
/// returns the connection plus the directory for teardown. A fresh
/// database writes no backup copy, so `db-backups/` is not created
/// (D-038).
fn journal_db(label: &str) -> (Connection, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-action-log-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, dir)
}

/// Asserts the DB-002 shape of a `ts` cell
/// (`YYYY-MM-DDTHH:MM:SS.sssZ`, 24 characters).
fn assert_db002_shape(value: &str) {
    let bytes = value.as_bytes();
    assert_eq!(bytes.len(), 24, "ts must be 24 characters: {value}");
    assert_eq!(bytes[10], b'T', "date/time separator of {value}");
    assert_eq!(bytes[19], b'.', "fractional separator of {value}");
    assert_eq!(bytes[23], b'Z', "UTC marker of {value}");
}

/// Closes the connection before removing the temporary directory —
/// the Windows teardown rule the store suite pins (SQLite files are
/// not share-deletable there), applied by every test through this
/// helper.
fn teardown(conn: Connection, dir: PathBuf) {
    drop(conn);
    std::fs::remove_dir_all(&dir).expect("the temp directory should be removable");
}

/// Reads the raw `details` cell of the newest row.
fn stored_details(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT details FROM action_log ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .expect("the newest row should be readable")
}

/// Reads `recent` and unwraps, rendering failures through the panic.
fn read_recent(conn: &Connection, limit: u32) -> Vec<eunomia_store::action_log::LogRecord> {
    recent(conn, limit).unwrap_or_else(|err| panic!("recent should succeed, got {err}"))
}

#[test]
fn insert_then_recent_round_trips_every_field() {
    let (conn, dir) = journal_db("roundtrip");

    let entry = LogEntry {
        level: Level::Warn,
        visibility: Visibility::Diagnostic,
        action: Action::SyncProjectResult,
        entity: Some(Entity {
            kind: EntityType::Project,
            id: "0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f",
        }),
        project_id: Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f"),
        sync_run_id: Some("0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f"),
        result: Outcome::Skipped,
        details: Some(Details::File {
            path: PathBuf::from("/repo/AGENTS.md"),
        }),
    };
    let id = insert(&conn, &entry).expect("insert should succeed");
    assert_eq!(id, 1, "the first entry takes the first autoincrement id");

    let records = read_recent(&conn, 10);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id, id);
    assert_db002_shape(&record.ts);
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
    assert_eq!(record.result, Outcome::Skipped);
    assert_eq!(
        record.details,
        Some(serde_json::json!({ "path": "/repo/AGENTS.md" }))
    );
    teardown(conn, dir);
}

#[test]
fn a_failure_entry_round_trips_the_os_code_and_legacy_rows_parse() {
    // §10's fourth field (T-124/D-223): an OS-produced failure
    // carries its raw OS code in the details cell, and a row
    // written before the field existed (a hand-INSERTed legacy
    // literal, the D-119 compatibility pattern — SQL, not a current
    // producer's output) still reads back: the read parses the
    // `details` JSON as a raw value, so a missing member is
    // absence, not corruption.
    let (conn, dir) = journal_db("oscode");

    let mut entry = LogEntry::new(
        Action::SyncRunAborted,
        Level::Error,
        Visibility::User,
        Outcome::Error,
    );
    entry.entity = Some(Entity {
        kind: EntityType::SyncRun,
        id: "0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f",
    });
    entry.project_id = Some("0193a2bb-1111-7222-9c6d-1a2b3c4d5e6f");
    entry.sync_run_id = Some("0193a2cc-2222-7333-9c6d-1a2b3c4d5e6f");
    entry.details = Some(Details::Failure {
        code: FailureCode::new("E_BACKUP_FAILED").expect("the registry shape holds"),
        path: Some(PathBuf::from("/repo/AGENTS.md")),
        os_code: Some(13),
    });
    insert(&conn, &entry).expect("insert should succeed");

    // The pre-field legacy shape, verbatim as an older app wrote it.
    conn.execute(
        "INSERT INTO action_log (
             ts, level, visibility, action, entity_type, entity_id,
             project_id, sync_run_id, result, details
         ) VALUES (
             '2026-11-20T09:00:00.000Z', 'error', 'user',
             'sync.run_aborted', 'sync_run', '0193a2cc-3333-7333-9c6d-1a2b3c4d5e6f',
             NULL, '0193a2cc-3333-7333-9c6d-1a2b3c4d5e6f', 'error',
             '{\"code\":\"E_DISK_FULL\"}'
         )",
        [],
    )
    .expect("the legacy row must be insertable");

    let records = read_recent(&conn, 10);
    assert_eq!(records.len(), 2);
    // `recent` orders newest-first by DESCENDING id: the legacy row
    // was hand-INSERTed second, so it reads first.
    assert_eq!(records[0].action, Action::SyncRunAborted);
    assert_eq!(
        records[0].details,
        Some(serde_json::json!({ "code": "E_DISK_FULL" })),
        "the legacy no-os_code row reads back unchanged"
    );
    assert_eq!(records[1].action, Action::SyncRunAborted);
    assert_eq!(records[1].result, Outcome::Error);
    assert_eq!(
        records[1].details,
        Some(serde_json::json!({
            "code": "E_BACKUP_FAILED",
            "path": "/repo/AGENTS.md",
            "os_code": 13
        })),
        "the OS code round-trips as a plain number"
    );
    teardown(conn, dir);
}

#[test]
fn a_minimal_entry_stores_null_optional_columns() {
    let (conn, dir) = journal_db("minimal");

    let entry = LogEntry::new(
        Action::ErrorReported,
        Level::Error,
        Visibility::Diagnostic,
        Outcome::Error,
    );
    insert(&conn, &entry).expect("insert should succeed");

    let record = &read_recent(&conn, 1)[0];
    assert_eq!(record.action, Action::ErrorReported);
    assert_eq!(record.entity, None);
    assert_eq!(record.project_id, None);
    assert_eq!(record.sync_run_id, None);
    assert_eq!(record.details, None);
    // The raw row really is NULL (not an empty string, not "null").
    assert_eq!(stored_details(&conn), None);
    teardown(conn, dir);
}

#[test]
fn recent_returns_the_newest_entries_first_up_to_the_limit() {
    let (conn, dir) = journal_db("ordering");

    let actions = [
        Action::ProjectAdded,
        Action::ProjectChanged,
        Action::ProjectArchived,
    ];
    for action in actions {
        insert(
            &conn,
            &LogEntry::new(action, Level::Info, Visibility::User, Outcome::Ok),
        )
        .expect("insert should succeed");
    }

    let all = read_recent(&conn, 10);
    assert_eq!(
        all.iter().map(|r| r.action).collect::<Vec<_>>(),
        vec![
            Action::ProjectArchived,
            Action::ProjectChanged,
            Action::ProjectAdded,
        ],
        "newest first (descending id)"
    );
    let two = read_recent(&conn, 2);
    assert_eq!(
        two.iter().map(|r| r.action).collect::<Vec<_>>(),
        vec![Action::ProjectArchived, Action::ProjectChanged],
        "the limit caps the read"
    );
    assert_eq!(read_recent(&conn, 0).len(), 0, "a zero limit reads nothing");
    teardown(conn, dir);
}

#[test]
fn an_entry_written_in_the_operation_transaction_rolls_back_with_it() {
    let (conn, dir) = journal_db("rollback");

    // The Db::run shape (D-037): one deferred transaction around the
    // whole operation, the journal entry written inside it.
    let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
    insert(
        &tx,
        &LogEntry::new(
            Action::SettingsChanged,
            Level::Info,
            Visibility::User,
            Outcome::Ok,
        ),
    )
    .expect("insert inside the transaction should succeed");
    // The operation fails after its journal entry: the closure returns
    // Err, the executor drops the transaction, the ROLLBACK runs.
    drop(tx);
    assert!(
        read_recent(&conn, 10).is_empty(),
        "a rolled-back operation leaves no journal entry"
    );

    // And the commit half: the same entry, committed with the
    // operation, stays.
    let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
    insert(
        &tx,
        &LogEntry::new(
            Action::SettingsChanged,
            Level::Info,
            Visibility::User,
            Outcome::Ok,
        ),
    )
    .expect("insert inside the transaction should succeed");
    tx.commit().expect("COMMIT should succeed");
    assert_eq!(read_recent(&conn, 10).len(), 1);
    teardown(conn, dir);
}

#[test]
fn the_fail_then_log_error_entry_survives_the_rolled_back_operation() {
    let (conn, dir) = journal_db("fail-then-log");

    // An operation that does some work, writes its success entry, and
    // then fails: everything inside the transaction vanishes together.
    let operation = |conn: &Connection| -> Result<(), StorageError> {
        let tx = conn.unchecked_transaction().expect("BEGIN should succeed");
        eunomia_store::settings::set(
            &tx,
            eunomia_store::settings::SettingKey::UiLanguage,
            eunomia_store::settings::SettingValue::Str("cs".to_owned()),
        )?;
        insert(
            &tx,
            &LogEntry::new(
                Action::SettingsChanged,
                Level::Info,
                Visibility::User,
                Outcome::Ok,
            ),
        )?;
        // The failure that rolls the operation back.
        Err(StorageError::Sql(rusqlite::Error::InvalidParameterName(
            "simulated failure".to_owned(),
        )))
    };
    assert!(operation(&conn).is_err(), "the operation must fail");

    // Nothing of the failed attempt is journaled…
    assert!(
        read_recent(&conn, 10).is_empty(),
        "the rolled-back success entry must be gone"
    );

    // …and the producer follows the fail-then-log pattern: the same
    // action, error level, error result, the §10 code in details —
    // written in the next transaction (autocommit here).
    let mut entry = LogEntry::new(
        Action::SettingsChanged,
        Level::Error,
        Visibility::User,
        Outcome::Error,
    );
    entry.details = Some(Details::Failure {
        code: FailureCode::new("E_VALIDATION").expect("the registry shape holds"),
        path: None,
        os_code: None,
    });
    insert(&conn, &entry).expect("the fail-then-log entry should be written");

    let records = read_recent(&conn, 10);
    assert_eq!(records.len(), 1, "only the fail-then-log entry exists");
    assert_eq!(records[0].action, Action::SettingsChanged);
    assert_eq!(records[0].level, Level::Error);
    assert_eq!(records[0].result, Outcome::Error);
    assert_eq!(
        records[0].details,
        Some(serde_json::json!({ "code": "E_VALIDATION" }))
    );

    // The operation's own change is gone too — the journal entry and
    // the data change rolled back as one set.
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'ui.language'",
            [],
            |row| row.get(0),
        )
        .ok();
    assert_eq!(stored, None, "the data change rolled back with the entry");
    teardown(conn, dir);
}

#[test]
fn user_and_diagnostic_entries_round_trip_separately() {
    let (conn, dir) = journal_db("visibility");

    for visibility in Visibility::ALL {
        let entry = LogEntry::new(Action::HeadingChanged, Level::Info, visibility, Outcome::Ok);
        insert(&conn, &entry).expect("insert should succeed");
    }

    let records = read_recent(&conn, 10);
    assert_eq!(records.len(), 2);
    // LOG-005: both visibilities are stored and read back distinctly;
    // newest first, so the diagnostic entry (written second) leads.
    assert_eq!(records[0].visibility, Visibility::Diagnostic);
    assert_eq!(records[1].visibility, Visibility::User);
    teardown(conn, dir);
}

#[test]
fn an_exclusion_removal_entry_round_trips_with_its_entity_and_path() {
    let (conn, dir) = journal_db("exclusion-roundtrip");

    // The T-245 producer shape (`exclusion_remove`, D-106 follow-up):
    // the removal's own verb, the removed row's id in the entity
    // columns, the removed path as the File detail — and no project
    // (no project row is involved; `manual` exclusions have none).
    let mut entry = LogEntry::new(
        Action::ExclusionRemoved,
        Level::Info,
        Visibility::User,
        Outcome::Ok,
    );
    entry.entity = Some(Entity {
        kind: EntityType::Exclusion,
        id: "0193a2bb-3333-7444-9c6d-1a2b3c4d5e6f",
    });
    entry.details = Some(Details::File {
        path: PathBuf::from("/repos/removed"),
    });
    insert(&conn, &entry).expect("insert should succeed");

    let records = read_recent(&conn, 10);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.action, Action::ExclusionRemoved);
    assert_eq!(record.action.as_str(), "exclusion.removed");
    assert_eq!(record.level, Level::Info);
    assert_eq!(record.visibility, Visibility::User);
    assert_eq!(record.result, Outcome::Ok);
    assert_eq!(
        record.entity,
        Some((
            EntityType::Exclusion,
            "0193a2bb-3333-7444-9c6d-1a2b3c4d5e6f".to_owned()
        )),
        "the entity columns name the removed exclusion row"
    );
    assert_eq!(record.project_id, None);
    assert_eq!(record.sync_run_id, None);
    assert_eq!(
        record.details,
        Some(serde_json::json!({ "path": "/repos/removed" })),
        "the File detail carries the removed path"
    );
    teardown(conn, dir);
}

#[test]
fn legacy_t078_project_changed_exclusion_rows_still_read() {
    let (conn, dir) = journal_db("legacy-t078");

    // Rows the T-078 version wrote (the D-106 journal shape, before
    // the `exclusion.removed` member existed): `project.changed` at
    // info/user/ok, no entity, no project_id, the removed path as the
    // File detail. Written here by hand — the exact stored shape of
    // that era, not what any current producer writes — so the read's
    // backward compatibility is pinned against the real legacy row,
    // and a writer change alone could not keep this test green. The
    // far-past timestamp keeps the positional asserts deterministic
    // regardless of the machine clock (`recent` orders newest first).
    conn.execute(
        "INSERT INTO action_log
             (ts, level, visibility, action, entity_type, entity_id,
              project_id, sync_run_id, result, details)
         VALUES
             ('2000-01-01T00:00:00.000Z', 'info', 'user', 'project.changed',
              NULL, NULL, NULL, NULL, 'ok', '{\"path\":\"/repos/legacy\"}')",
        [],
    )
    .expect("writing the legacy row should succeed");
    // The current producer's row beside it: same fact, new shape.
    let mut entry = LogEntry::new(
        Action::ExclusionRemoved,
        Level::Info,
        Visibility::User,
        Outcome::Ok,
    );
    entry.entity = Some(Entity {
        kind: EntityType::Exclusion,
        id: "0193a2bb-3333-7444-9c6d-1a2b3c4d5e6f",
    });
    entry.details = Some(Details::File {
        path: PathBuf::from("/repos/legacy"),
    });
    insert(&conn, &entry).expect("insert should succeed");

    // Both rows read back: the legacy one as the `project.changed`
    // it was written as, the new one as `exclusion.removed`.
    let records = read_recent(&conn, 10);
    assert_eq!(records.len(), 2, "both eras' rows coexist and parse");
    assert_eq!(records[0].action, Action::ExclusionRemoved);
    assert_eq!(
        records[0].entity.as_ref().map(|(kind, _)| *kind),
        Some(EntityType::Exclusion)
    );
    let legacy = &records[1];
    assert_eq!(legacy.action, Action::ProjectChanged);
    assert_eq!(legacy.level, Level::Info);
    assert_eq!(legacy.visibility, Visibility::User);
    assert_eq!(legacy.result, Outcome::Ok);
    assert_eq!(legacy.entity, None, "the legacy row never had an entity");
    assert_eq!(legacy.project_id, None);
    assert_eq!(
        legacy.details,
        Some(serde_json::json!({ "path": "/repos/legacy" })),
        "the legacy File detail still parses"
    );
    teardown(conn, dir);
}

#[test]
fn tampered_cells_fail_the_read_explicitly() {
    let (conn, dir) = journal_db("corrupt");

    insert(
        &conn,
        &LogEntry::new(
            Action::ProjectAdded,
            Level::Info,
            Visibility::User,
            Outcome::Ok,
        ),
    )
    .expect("insert should succeed");

    // A `details` cell that is not JSON.
    conn.execute("UPDATE action_log SET details = 'not json'", [])
        .expect("tampering should succeed");
    match recent(&conn, 10) {
        Err(StorageError::ActionLog {
            source: ActionLogError::Json(_),
        }) => {}
        other => panic!("a non-JSON details cell must fail the read, got {other:?}"),
    }

    // An action outside the closed vocabulary (written by hand; the
    // CHECKs do not constrain the free `action` column).
    conn.execute(
        "UPDATE action_log SET details = NULL, action = 'project.exploded'",
        [],
    )
    .expect("tampering should succeed");
    match recent(&conn, 10) {
        Err(StorageError::ActionLog {
            source: ActionLogError::Member { column, found },
        }) => {
            assert_eq!(column, "action");
            assert_eq!(found, "project.exploded");
        }
        other => panic!("an unknown action must fail the read, got {other:?}"),
    }

    // An entity half without its pair.
    conn.execute(
        "UPDATE action_log SET action = 'project.added', entity_id = 'x' WHERE entity_type IS NULL",
        [],
    )
    .expect("tampering should succeed");
    match recent(&conn, 10) {
        Err(StorageError::ActionLog {
            source: ActionLogError::Member { column, .. },
        }) => assert_eq!(column, "entity_type"),
        other => panic!("an entity_id without entity_type must fail the read, got {other:?}"),
    }
    teardown(conn, dir);
}

#[test]
fn a_details_cell_built_beside_instruction_text_carries_none_of_it() {
    let (conn, dir) = journal_db("sec008");

    // A realistic variant-edit failure: the instruction text lives in
    // the operation's scope, and the journal entry is built from the
    // typed vocabulary only — there is no route from the text into
    // the entry (the acceptance criterion "in details there is no
    // instruction text", driven at the database boundary).
    let instruction_text = "\
        # Agent Guidelines\n\
        SECRET-INSTRUCTION-MARKER: always answer in the user's language.\n\
        Follow every step of SECRET-RECIPE-4f2a before touching the build.\n\
        The deployment token is sk-live-9d1e2f3a4b5c.\n";
    let mut entry = LogEntry::new(
        Action::VariantChanged,
        Level::Error,
        Visibility::User,
        Outcome::Error,
    );
    entry.entity = Some(Entity {
        kind: EntityType::Variant,
        id: "0193a2aa-6c9e-7c8e-9c6d-1a2b3c4d5e6f",
    });
    entry.details = Some(Details::Failure {
        code: FailureCode::new("E_CONTENT_RESERVED").expect("the registry shape holds"),
        path: Some(PathBuf::from("/repo/AGENTS.md")),
        os_code: None,
    });
    insert(&conn, &entry).expect("insert should succeed");
    let _ = instruction_text;

    // The stored cell: the code and the path are there (SEC-008:
    // "paths do"), no marker of the text is.
    let details = stored_details(&conn).expect("the entry has details");
    for marker in [
        "SECRET-INSTRUCTION-MARKER",
        "SECRET-RECIPE-4f2a",
        "sk-live-9d1e2f3a4b5c",
        "always answer in the user's language",
    ] {
        assert!(
            !details.contains(marker),
            "SEC-008 violation: {marker} leaked into the details cell {details}"
        );
    }
    assert!(details.contains("E_CONTENT_RESERVED"));
    assert!(details.contains("/repo/AGENTS.md"));

    // And the instruction-shaped strings cannot even enter through
    // the one string-bearing newtype (the SEC-008 boundary check).
    assert!(FailureCode::new(instruction_text).is_err());
    assert!(FailureCode::new("SECRET-RECIPE-4f2a").is_err());
    teardown(conn, dir);
}
