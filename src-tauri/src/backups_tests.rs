//! Mock-IPC behavior tests for `backups_list` (T-125, BKP-004).

use eunomia_core::backup_name::{BackupTime, backup_relative_path};
use eunomia_core::project::{BaseName as CoreBaseName, ProjectId, Scheme as CoreScheme};
use eunomia_store::backup_records::{
    BackupRecordStatus, NewBackupRecord, insert, list_for_project,
};
use eunomia_store::projects::{AddedVia, NewProject};
use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
use eunomia_store::sync_runs::{NewSyncRunProject, create_running};
use tauri::Manager;

use crate::backups::BackupCopyRecord;

fn backups_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("temp directory creates");
    crate::data_dirs::ensure_data_layout(temp.path()).expect("data layout creates");
    let database = crate::db::Db::open(temp.path()).expect("database opens");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![crate::backups::backups_list,])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds");
    app.manage(database);
    (temp, app)
}

fn backups_app_without_db() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![crate::backups::backups_list,])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds")
}

fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("mock main webview builds")
}

fn invoke_request(cmd: &str, body: serde_json::Value) -> tauri::webview::InvokeRequest {
    tauri::webview::InvokeRequest {
        cmd: cmd.into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost".parse().unwrap()
        } else {
            "tauri://localhost".parse().unwrap()
        },
        body: tauri::ipc::InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.to_string(),
    }
}

fn invoke<T: serde::de::DeserializeOwned>(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    body: serde_json::Value,
) -> Result<T, serde_json::Value> {
    match tauri::test::get_ipc_response(webview, invoke_request(cmd, body)) {
        Ok(payload) => Ok(payload
            .deserialize::<T>()
            .expect("success payload deserializes into the backup DTO")),
        Err(error) => Err(error),
    }
}

fn seed_error(error: eunomia_store::projects::ProjectsError) -> crate::errors::AppError {
    crate::commands::projects_store_to_app_error(error)
}

/// One full `backup_records` fixture row, distinct per `n`, with an
/// explicit DB-002 `created_at` so the listing's `created_at DESC`
/// order is observable (not only the id tiebreak).
#[derive(Clone)]
struct SeededRecord {
    id: String,
    sync_run_id: Option<String>,
    backup_path: String,
    size_bytes: i64,
    status: BackupRecordStatus,
    created_at: String,
}

/// One row's seed facts: `n` is the record's whole identity — the
/// digest, the §5.5 name's second field and the `created_at` stamp's
/// own second (the name and the record agree about when the copy was
/// made, the module's invariant) — while the status, the stamp text
/// and the size vary per row.
struct SeedRow {
    n: usize,
    status: BackupRecordStatus,
    created_at: &'static str,
    size_bytes: i64,
}

fn seed_record(
    transaction: &rusqlite::Transaction<'_>,
    project_id: &str,
    project_name: &str,
    run_id: Option<&str>,
    row: SeedRow,
) -> Result<SeededRecord, crate::errors::AppError> {
    let SeedRow {
        n,
        status,
        created_at,
        size_bytes,
    } = row;
    let sha256 = format!("{n:0>8}{}", "0".repeat(56));
    // The §5.5 layout path through the REAL core builders (the store
    // suite's D-122 fixture discipline), with the record's own
    // instant: the name's time component and `created_at` say the
    // same second.
    let time = BackupTime {
        year: 2026,
        month: 11,
        day: 21,
        hour: 12,
        minute: 0,
        second: u8::try_from(n).expect("a small second field"),
        millisecond: 0,
    };
    let backup_path = backup_relative_path(
        &ProjectId::new(project_id.to_owned()),
        project_name,
        CoreScheme::A,
        CoreBaseName::AgentsMd,
        time,
        &sha256,
    )
    .expect("the builder accepts DB-shaped ids and 64-hex digests");
    let id = insert(
        transaction,
        &NewBackupRecord {
            project_id,
            sync_run_id: run_id,
            source_path: "/repos/repo-1/AGENTS.md",
            backup_path: &backup_path,
            size_bytes,
            sha256_hex: &sha256,
            status,
            created_at,
        },
    )
    .map_err(crate::db::backup_records_to_app_error)?;
    Ok(SeededRecord {
        id,
        sync_run_id: run_id.map(str::to_owned),
        backup_path,
        size_bytes,
        status,
        created_at: created_at.to_owned(),
    })
}

fn seed_project(
    transaction: &rusqlite::Transaction<'_>,
    n: usize,
) -> Result<String, crate::errors::AppError> {
    let path: &'static str = Box::leak(format!("/repos/repo-{n}").into_boxed_str());
    eunomia_store::projects::create(
        transaction,
        &NewProject {
            path,
            path_key: path,
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
    .map(|project| project.id)
    .map_err(seed_error)
}

fn seed_run(
    transaction: &rusqlite::Transaction<'_>,
    project_id: &str,
    n: usize,
) -> Result<String, crate::errors::AppError> {
    create_running(
        transaction,
        Box::leak(format!("2026-11-21T12:00:0{n}.000Z").into_boxed_str()),
        &[NewSyncRunProject {
            project_id,
            target_path: "/repos/repo-1/AGENTS.md",
            build_hash: &"a".repeat(64),
        }],
    )
    .map(|run| run.id)
    .map_err(crate::db::sync_runs_to_app_error)
}

fn journal_of(database: &crate::db::Db) -> Vec<eunomia_store::action_log::LogRecord> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 1000)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("journal reads")
}

#[test]
fn backups_list_returns_the_projects_history_newest_first() {
    let (_temp, app) = backups_app();
    let database = database_of(&app);
    // The seed: one project with a run, another project's row that
    // must not leak, and one row whose run is swept afterwards (the
    // §6.4 run cleanup — `ON DELETE SET NULL` leaves the honest
    // `None` link). The insert order mixes the timeline so an
    // insertion-ordered answer is caught.
    let seeded = tauri::async_runtime::block_on(database.run(|tx| {
        let project = seed_project(tx, 1)?;
        let other = seed_project(tx, 2)?;
        let run = seed_run(tx, &project, 1)?;
        let mut rows = Vec::new();
        for row in [
            SeedRow {
                n: 3,
                status: BackupRecordStatus::Pruned,
                created_at: "2026-11-21T12:00:03.000Z",
                size_bytes: 64,
            },
            SeedRow {
                n: 4,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-21T12:00:04.000Z",
                size_bytes: 64,
            },
            SeedRow {
                n: 2,
                status: BackupRecordStatus::Missing,
                created_at: "2026-11-21T12:00:02.000Z",
                size_bytes: 64,
            },
            SeedRow {
                n: 1,
                status: BackupRecordStatus::Discarded,
                created_at: "2026-11-21T12:00:01.000Z",
                size_bytes: 64,
            },
            // The saturation arm: an absurd size beyond u32::MAX still
            // answers the wire's u32 ceiling (D-049), newest overall.
            SeedRow {
                n: 5,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-21T12:00:05.000Z",
                size_bytes: 4_294_967_296,
            },
        ] {
            rows.push(seed_record(tx, &project, "Project 1", Some(&run), row)?);
        }
        seed_record(
            tx,
            &other,
            "Project 2",
            None,
            SeedRow {
                n: 9,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-21T12:00:09.000Z",
                size_bytes: 64,
            },
        )?;
        // The run of the oldest row is swept: its record keeps the
        // history with a NULL link (§6.4, the FK's SET NULL).
        let swept_run = seed_run(tx, &project, 2)?;
        let mut swept = seed_record(
            tx,
            &project,
            "Project 1",
            Some(&swept_run),
            SeedRow {
                n: 0,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-21T12:00:00.000Z",
                size_bytes: 64,
            },
        )?;
        tx.execute(
            "DELETE FROM sync_runs WHERE id = ?1",
            rusqlite::params![&swept_run],
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        // The FK's `ON DELETE SET NULL` is the contract the row now
        // carries: the expectation follows it, not the stale seed.
        swept.sync_run_id = None;
        Ok((project, run, rows, swept))
    }))
    .expect("the seed transaction commits");
    let (project, run, rows, swept) = seeded;
    // The oracle: the fixture's own stamps, newest first (BKP-004's
    // newest-first listing, D-122's ordering).
    let mut expected = rows.clone();
    expected.push(swept.clone());
    expected.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    let webview = main_webview(&app);

    let listed: Vec<BackupCopyRecord> = invoke(
        &webview,
        "backups_list",
        serde_json::json!({ "projectId": project }),
    )
    .expect("the listing succeeds");

    // The full row shape, every field, newest first — the swept row's
    // run link is the honest `None`, the foreign project's row absent.
    assert_eq!(
        listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        expected
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        "the whole history is newest first and strictly scoped"
    );
    for (row, want) in listed.iter().zip(expected.iter()) {
        assert_eq!(row.id, want.id);
        assert_eq!(row.sync_run_id, want.sync_run_id);
        assert_eq!(row.source_path, "/repos/repo-1/AGENTS.md");
        assert_eq!(row.backup_path, want.backup_path);
        assert_eq!(
            row.size_bytes,
            u32::try_from(want.size_bytes).unwrap_or(u32::MAX)
        );
        assert_eq!(row.status, want.status.as_str());
        assert_eq!(row.created_at, want.created_at);
    }
    assert_eq!(
        listed
            .iter()
            .map(|row| row.status.as_str())
            .collect::<Vec<_>>(),
        vec!["ok", "ok", "pruned", "missing", "discarded", "ok"],
        "every §6.2 CHECK status is listed"
    );
    assert_eq!(listed[0].size_bytes, u32::MAX, "the absurd size saturates");
    assert_eq!(
        listed[5].sync_run_id, None,
        "the swept run's row keeps its history with no run link"
    );
    assert_eq!(
        listed[0].sync_run_id.as_deref(),
        Some(run.as_str()),
        "the live run's rows name their run"
    );
    // The wire key set: exactly the seven BKP-004 columns, no extras.
    let raw: serde_json::Value = invoke(
        &webview,
        "backups_list",
        serde_json::json!({ "projectId": project }),
    )
    .expect("the raw listing succeeds");
    let mut keys = raw
        .as_array()
        .expect("the wire answer is an array")
        .first()
        .expect("the seeded rows list")
        .as_object()
        .expect("the row is a JSON object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "backup_path",
            "created_at",
            "id",
            "size_bytes",
            "source_path",
            "status",
            "sync_run_id",
        ],
        "the row carries exactly the BKP-004 field set"
    );
    assert!(
        journal_of(&database).is_empty(),
        "the listing journals nothing"
    );
}

#[test]
fn backups_list_refuses_unknown_ids_and_answers_empty_for_known_or_archived_projects() {
    let (_temp, app) = backups_app();
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(|tx| {
        let project = seed_project(tx, 1)?;
        let archived = seed_project(tx, 2)?;
        let run = seed_run(tx, &archived, 1)?;
        let record = seed_record(
            tx,
            &archived,
            "Project 2",
            Some(&run),
            SeedRow {
                n: 1,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-21T12:00:01.000Z",
                size_bytes: 64,
            },
        )?;
        eunomia_store::projects::archive(tx, &archived).map_err(seed_error)?;
        Ok((project, archived, record))
    }))
    .expect("the seed transaction commits");
    let (project, archived, record) = seeded;
    let webview = main_webview(&app);

    let unknown: Result<Vec<BackupCopyRecord>, serde_json::Value> = invoke(
        &webview,
        "backups_list",
        serde_json::json!({ "projectId": "missing" }),
    );
    let unknown = unknown.expect_err("an unknown project id refuses");
    assert_eq!(unknown["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(unknown["params"]["field"], serde_json::json!("project_id"));
    assert_eq!(
        unknown["params"]["reason"],
        serde_json::json!("no project has this id")
    );

    let empty: Vec<BackupCopyRecord> = invoke(
        &webview,
        "backups_list",
        serde_json::json!({ "projectId": project }),
    )
    .expect("a known project with no rows answers an empty list");
    assert_eq!(empty, Vec::<BackupCopyRecord>::new());

    let archived_rows: Vec<BackupCopyRecord> = invoke(
        &webview,
        "backups_list",
        serde_json::json!({ "projectId": archived }),
    )
    .expect("an archived project's history still lists (DB-014)");
    assert_eq!(archived_rows.len(), 1);
    assert_eq!(archived_rows[0].id, record.id);
    assert_eq!(archived_rows[0].status, "ok");
    assert!(
        journal_of(&database).is_empty(),
        "the listing journals nothing"
    );
}

#[test]
fn the_backups_command_answers_honestly_without_a_database() {
    let app = backups_app_without_db();
    let webview = main_webview(&app);
    let listed: Result<Vec<BackupCopyRecord>, serde_json::Value> = invoke(
        &webview,
        "backups_list",
        serde_json::json!({ "projectId": "p" }),
    );
    let error = listed.expect_err("the listing refuses without a database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn the_backups_command_is_registered_once_with_specta() {
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("backups::backups_list").count(),
        1,
        "lib.rs registers `backups::backups_list` once"
    );
}

/// The command maps the store's listing row onto the wire untouched:
/// every store column the tab reaches survives the mapping (the
/// scoping and error arms are the command's own, above).
#[test]
fn the_command_maps_the_store_listing_untouched() {
    let (_temp, app) = backups_app();
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(|tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        seed_record(
            tx,
            &project,
            "Project 1",
            Some(&run),
            SeedRow {
                n: 1,
                status: BackupRecordStatus::Pruned,
                created_at: "2026-11-21T12:00:01.000Z",
                size_bytes: 64,
            },
        )?;
        seed_record(
            tx,
            &project,
            "Project 1",
            Some(&run),
            SeedRow {
                n: 2,
                status: BackupRecordStatus::Ok,
                created_at: "2026-11-21T12:00:02.000Z",
                size_bytes: 64,
            },
        )?;
        Ok(project)
    }))
    .expect("the seed transaction commits");
    let store_project = seeded.clone();
    let store_rows = tauri::async_runtime::block_on(database.run(move |tx| {
        list_for_project(tx, &store_project).map_err(crate::db::backup_records_to_app_error)
    }))
    .expect("the store listing succeeds");
    assert_eq!(store_rows.len(), 2);
    let webview = main_webview(&app);
    let listed: Vec<BackupCopyRecord> = invoke(
        &webview,
        "backups_list",
        serde_json::json!({ "projectId": seeded }),
    )
    .expect("the listing succeeds");
    assert_eq!(listed.len(), store_rows.len());
    for (row, store_row) in listed.iter().zip(store_rows.iter()) {
        assert_eq!(row.id, store_row.id);
        assert_eq!(row.sync_run_id, store_row.sync_run_id);
        assert_eq!(row.source_path, store_row.source_path);
        assert_eq!(row.backup_path, store_row.backup_path);
        assert_eq!(
            row.size_bytes,
            u32::try_from(store_row.size_bytes).unwrap_or(u32::MAX)
        );
        assert_eq!(row.status, store_row.status.as_str());
        assert_eq!(row.created_at, store_row.created_at);
    }
}
