//! Behavior tests for the settings repository (spec §6.5; task T-028).
//!
//! The tests run against a real database opened through `open_db` and
//! migrated with `run_migrations` — the production configuration
//! (DB-001 pragmas, D-030) — in a temporary directory, and drive the
//! repository through its public API:
//!
//! * a fresh, empty database reads back the §6.5 defaults for every
//!   key (reads never write rows);
//! * `set` → `get` round-trips, with the canonical JSON text and a
//!   DB-002 `updated_at` pinned per key;
//! * writes outside a key's range are rejected and leave the stored
//!   state untouched;
//! * a stored cell that is not valid JSON — or valid JSON outside the
//!   key's contract — fails the read with an explicit error, never a
//!   silent default;
//! * rows with unknown keys do not disturb `get_all`;
//! * changing `default.*` settings leaves existing project rows
//!   untouched (DEF-005).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eunomia_store::settings::{
    BaseName, ExtraFile, Position, Scheme, SettingKey, SettingValue, SettingsError, get, get_all,
    set,
};
use eunomia_store::storage::{StorageError, open_db, run_migrations};
use rusqlite::Connection;

/// Creates a unique temporary directory holding a migrated database
/// (opened through `open_db`, so the DB-001 pragma set applies) and
/// returns the connection plus the directory for teardown. A fresh
/// database writes no backup copy, so `db-backups/` is not created
/// (D-038).
fn settings_db(label: &str) -> (Connection, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be past the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "eunomia-store-settings-{label}-{}-{nanos}",
        std::process::id(),
    ));
    std::fs::create_dir_all(dir.join("db")).expect("temp db directory should be creatable");
    let mut conn =
        open_db(&dir.join("db").join("eunomiasync.sqlite3")).expect("open_db should succeed");
    run_migrations(&mut conn, Path::new("unused-backup-dir"))
        .expect("migrations should apply to a fresh database");
    (conn, dir)
}

/// Asserts the DB-002 shape of an `updated_at` cell
/// (`YYYY-MM-DDTHH:MM:SS.sssZ`, 24 characters).
fn assert_db002_shape(value: &str) {
    let bytes = value.as_bytes();
    assert_eq!(bytes.len(), 24, "updated_at must be 24 characters: {value}");
    assert_eq!(bytes[10], b'T', "date/time separator of {value}");
    assert_eq!(bytes[19], b'.', "fractional separator of {value}");
    assert_eq!(bytes[23], b'Z', "UTC marker of {value}");
}

/// Reads the raw `value` cell of `key`.
fn stored_value(conn: &Connection, key: &str) -> String {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .expect("the row should exist")
}

/// Asserts the read succeeds and returns the value (`StorageError`
/// cannot be compared wholesale, so failures render through the
/// panic).
fn read(conn: &Connection, key: SettingKey) -> SettingValue {
    get(conn, key).unwrap_or_else(|err| panic!("{}: read should succeed, got {err}", key.as_str()))
}

#[test]
fn a_fresh_database_reads_back_the_spec_defaults() {
    let (conn, dir) = settings_db("defaults");

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "the fresh database holds no settings rows");

    let all = get_all(&conn).expect("get_all on an empty database should succeed");
    let keys: Vec<&str> = all.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "default.scheme",
            "default.base_name",
            "default.extra_local",
            "default.extra_project",
            "backup.retention",
            "sync.skip_identical",
            "scan.max_depth",
            "scan.exclude_names",
            "history.max_revisions",
            "log.max_entries",
            "log.max_days",
            "ui.language",
            "update.check_on_start",
        ],
        "every §6.5 key, in table order"
    );

    // Expected values hardcoded from the spec §6.5 table.
    let expected: Vec<SettingValue> = vec![
        SettingValue::Scheme(Scheme::A),
        SettingValue::BaseName(BaseName::AgentsMd),
        SettingValue::ExtraFile(ExtraFile {
            enabled: false,
            position: Position::End,
        }),
        SettingValue::ExtraFile(ExtraFile {
            enabled: false,
            position: Position::End,
        }),
        SettingValue::Int(10),
        SettingValue::Bool(true),
        SettingValue::Int(6),
        SettingValue::StrList(
            [
                "node_modules",
                "vendor",
                ".git",
                "dist",
                "build",
                "target",
                ".venv",
            ]
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        ),
        SettingValue::Int(200),
        SettingValue::Int(10000),
        SettingValue::Int(180),
        SettingValue::Str("en".to_owned()),
        SettingValue::Bool(true),
    ];
    for ((key, value), want) in all.iter().zip(expected) {
        assert_eq!(value, &want, "value of {}", key.as_str());
        assert_eq!(read(&conn, *key), want, "get of {}", key.as_str());
    }

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "reading must not write rows");

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn set_then_get_round_trips_and_stores_canonical_json() {
    let (conn, dir) = settings_db("roundtrip");

    let cases: Vec<(SettingKey, SettingValue, &str)> = vec![
        (
            SettingKey::DefaultScheme,
            SettingValue::Scheme(Scheme::C),
            "\"C\"",
        ),
        (
            SettingKey::DefaultBaseName,
            SettingValue::BaseName(BaseName::ClaudeMd),
            "\"CLAUDE.md\"",
        ),
        (
            SettingKey::DefaultExtraLocal,
            SettingValue::ExtraFile(ExtraFile {
                enabled: true,
                position: Position::Start,
            }),
            "{\"enabled\":true,\"position\":\"start\"}",
        ),
        (
            SettingKey::DefaultExtraProject,
            SettingValue::ExtraFile(ExtraFile {
                enabled: false,
                position: Position::End,
            }),
            "{\"enabled\":false,\"position\":\"end\"}",
        ),
        (SettingKey::BackupRetention, SettingValue::Int(200), "200"),
        (
            SettingKey::SyncSkipIdentical,
            SettingValue::Bool(false),
            "false",
        ),
        (SettingKey::ScanMaxDepth, SettingValue::Int(1), "1"),
        (
            SettingKey::ScanExcludeNames,
            SettingValue::StrList(vec![".cache".to_owned(), "tmp".to_owned()]),
            "[\".cache\",\"tmp\"]",
        ),
        (
            SettingKey::HistoryMaxRevisions,
            SettingValue::Int(1000),
            "1000",
        ),
        (
            SettingKey::LogMaxEntries,
            SettingValue::Int(100000),
            "100000",
        ),
        (SettingKey::LogMaxDays, SettingValue::Int(3650), "3650"),
        (
            SettingKey::UiLanguage,
            SettingValue::Str("cs".to_owned()),
            "\"cs\"",
        ),
        (
            SettingKey::UpdateCheckOnStart,
            SettingValue::Bool(false),
            "false",
        ),
    ];

    for (key, value, json) in &cases {
        set(&conn, *key, value.clone()).expect("a valid value should be storable");
        assert_eq!(
            read(&conn, *key),
            value.clone(),
            "read-back of {}",
            key.as_str()
        );
        assert_eq!(&stored_value(&conn, key.as_str()), json, "stored JSON");
        let updated_at: String = conn
            .query_row(
                "SELECT updated_at FROM settings WHERE key = ?1",
                [key.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_db002_shape(&updated_at);
    }

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 13, "one row per §6.5 key");

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn set_replaces_the_value_and_bumps_updated_at() {
    let (conn, dir) = settings_db("replace");

    set(&conn, SettingKey::BackupRetention, SettingValue::Int(10)).unwrap();
    let first: String = conn
        .query_row(
            "SELECT updated_at FROM settings WHERE key = 'backup.retention'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    // `thread::sleep` sleeps at least the requested duration, so the
    // millisecond-resolution DB-002 stamp necessarily differs.
    std::thread::sleep(std::time::Duration::from_millis(2));
    set(&conn, SettingKey::BackupRetention, SettingValue::Int(20)).unwrap();

    assert_eq!(
        read(&conn, SettingKey::BackupRetention),
        SettingValue::Int(20),
        "the second value wins"
    );
    let second: String = conn
        .query_row(
            "SELECT updated_at FROM settings WHERE key = 'backup.retention'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(first, second, "updated_at must move with the rewrite");

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn out_of_range_writes_are_rejected_and_leave_the_stored_state_intact() {
    let (conn, dir) = settings_db("ranges");

    let ranges: [(SettingKey, (i64, i64)); 5] = [
        (SettingKey::BackupRetention, (1, 200)),
        (SettingKey::ScanMaxDepth, (1, 20)),
        (SettingKey::HistoryMaxRevisions, (10, 1000)),
        (SettingKey::LogMaxEntries, (1000, 100000)),
        (SettingKey::LogMaxDays, (7, 3650)),
    ];
    for (key, (min, max)) in ranges {
        // A stored in-range value the rejected writes must not disturb.
        set(&conn, key, SettingValue::Int(min)).unwrap();
        for rejected in [min - 1, max + 1] {
            match set(&conn, key, SettingValue::Int(rejected)) {
                Err(StorageError::Settings {
                    key: errored,
                    source: SettingsError::Range { value, .. },
                }) => {
                    assert_eq!(errored, key.as_str());
                    assert_eq!(value, i128::from(rejected));
                }
                other => panic!(
                    "{}: {rejected} must be rejected as out of range, got {other:?}",
                    key.as_str()
                ),
            }
        }
        assert_eq!(
            read(&conn, key),
            SettingValue::Int(min),
            "a rejected write must leave the stored value untouched ({})",
            key.as_str()
        );
    }

    // A rejected write on a key with no row must not create one.
    match set(&conn, SettingKey::UiLanguage, SettingValue::Int(5)) {
        Err(StorageError::Settings {
            source: SettingsError::Type { .. },
            ..
        }) => {}
        other => panic!("a type-mismatched value must be rejected, got {other:?}"),
    }
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 5, "only the five in-range rows exist");

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_stored_value_that_is_not_json_fails_the_read_explicitly() {
    let (conn, dir) = settings_db("corrupt-json");

    conn.execute(
        "INSERT INTO settings (key, value, updated_at)
         VALUES ('scan.max_depth', 'six', '2026-11-18T10:00:00.000Z')",
        [],
    )
    .unwrap();

    for label in ["get", "get_all"] {
        let err = match label {
            "get" => get(&conn, SettingKey::ScanMaxDepth)
                .expect_err("a non-JSON cell must fail the read"),
            _ => get_all(&conn).expect_err("get_all must fail on the same cell"),
        };
        match &err {
            StorageError::Settings { key, source } => {
                assert_eq!(key, "scan.max_depth");
                assert!(
                    matches!(source, SettingsError::Json(_)),
                    "expected a JSON error, got {source:?}"
                );
            }
            other => panic!("expected StorageError::Settings, got {other:?}"),
        }
        let rendered = err.to_string();
        assert!(
            rendered.contains("scan.max_depth"),
            "the error names the key: {rendered}"
        );
    }

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_stored_value_outside_the_contract_fails_the_read_explicitly() {
    let (conn, dir) = settings_db("corrupt-values");

    /// Decides whether an error is the variant its case expects.
    type ExpectedError = fn(&SettingsError) -> bool;

    let cases: [(&str, &str, ExpectedError); 3] = [
        ("backup.retention", "500", |source| {
            matches!(source, SettingsError::Range { .. })
        }),
        ("sync.skip_identical", "\"yes\"", |source| {
            matches!(source, SettingsError::Type { .. })
        }),
        ("default.extra_local", "{\"enabled\":true}", |source| {
            matches!(source, SettingsError::Shape { .. })
        }),
    ];
    for (key, json, is_expected) in cases {
        conn.execute(
            "INSERT INTO settings (key, value, updated_at)
             VALUES (?1, ?2, '2026-11-18T10:00:00.000Z')",
            rusqlite::params![key, json],
        )
        .unwrap();
        let setting_key = SettingKey::from_key(key).expect("the test keys are known");
        let err = get(&conn, setting_key)
            .expect_err("a stored value outside the contract must fail the read");
        match &err {
            StorageError::Settings {
                key: errored,
                source,
            } => {
                assert_eq!(errored, key);
                assert!(is_expected(source), "{key}: unexpected error {source:?}");
            }
            other => panic!("expected StorageError::Settings, got {other:?}"),
        }
    }

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn set_rejects_a_value_variant_that_does_not_belong_to_the_key() {
    let (conn, dir) = settings_db("kind-mismatch");

    // Every case below would serialize to JSON its key could parse
    // (a scheme serializes to the string "A"); the variant pairing
    // must reject the call before that coercion happens.
    let mismatched: [(SettingKey, SettingValue, &str, &str); 3] = [
        (
            SettingKey::UiLanguage,
            SettingValue::Scheme(Scheme::A),
            "string",
            "scheme",
        ),
        (
            SettingKey::DefaultScheme,
            SettingValue::Str("A".to_owned()),
            "scheme",
            "string",
        ),
        (
            SettingKey::BackupRetention,
            SettingValue::Bool(true),
            "integer",
            "boolean",
        ),
    ];
    for (key, value, expected, found) in mismatched {
        match set(&conn, key, value) {
            Err(StorageError::Settings {
                key: errored,
                source:
                    SettingsError::Type {
                        expected: want,
                        found: got,
                    },
            }) => {
                assert_eq!(errored, key.as_str());
                assert_eq!(want, expected);
                assert_eq!(got, found);
            }
            other => panic!("{key:?}: expected a pairing error, got {other:?}"),
        }
    }

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "rejected writes must not create rows");

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_stored_keys_do_not_disturb_get_all() {
    let (conn, dir) = settings_db("unknown-keys");

    conn.execute(
        "INSERT INTO settings (key, value, updated_at)
         VALUES ('future.thing', '\"x\"', '2026-11-18T10:00:00.000Z')",
        [],
    )
    .unwrap();

    let all = get_all(&conn).expect("an unknown row must not fail get_all");
    assert_eq!(all.len(), SettingKey::ALL.len());
    assert!(
        all.iter().all(|(key, _)| key.as_str() != "future.thing"),
        "the unknown key is not part of the result"
    );

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn changing_default_settings_leaves_existing_projects_untouched() {
    // DEF-005/PRJ-012: `default.*` settings are copied into a project
    // at creation; changing them later must not rewrite project rows.
    // This pins the store-level half of that rule.
    let (conn, dir) = settings_db("def005");

    conn.execute(
        "INSERT INTO projects (id, path, path_key, name, label, color, added_via,
                               sort_order, scheme, base_name, created_at, updated_at)
         VALUES ('p1', '/repo/p1', '/repo/p1', 'Project', 'PRJ', 'blue', 'manual',
                 1, 'A', 'AGENTS.md', '2026-11-18T10:00:00.000Z',
                 '2026-11-18T10:00:00.000Z')",
        [],
    )
    .unwrap();

    set(
        &conn,
        SettingKey::DefaultScheme,
        SettingValue::Scheme(Scheme::C),
    )
    .unwrap();
    set(
        &conn,
        SettingKey::DefaultBaseName,
        SettingValue::BaseName(BaseName::ClaudeMd),
    )
    .unwrap();
    set(
        &conn,
        SettingKey::DefaultExtraLocal,
        SettingValue::ExtraFile(ExtraFile {
            enabled: true,
            position: Position::Start,
        }),
    )
    .unwrap();
    set(
        &conn,
        SettingKey::DefaultExtraProject,
        SettingValue::ExtraFile(ExtraFile {
            enabled: true,
            position: Position::Start,
        }),
    )
    .unwrap();

    let project: (String, String, i64, String, i64, String) = conn
        .query_row(
            "SELECT scheme, base_name, extra_local_enabled, extra_local_position,
                    extra_project_enabled, extra_project_position
             FROM projects WHERE id = 'p1'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        project,
        (
            "A".to_owned(),
            "AGENTS.md".to_owned(),
            0,
            "end".to_owned(),
            0,
            "end".to_owned()
        ),
        "no project column may follow a changed default"
    );

    drop(conn);
    std::fs::remove_dir_all(&dir).unwrap();
}
