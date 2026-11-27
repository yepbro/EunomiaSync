//! Mock-IPC behavior tests for the Backups module (T-125/T-353):
//! `backups_list` (BKP-004) through the mock IPC harness, and
//! `backup_save` (BKP-012) through the `backup_save_with` seam — the
//! `dialog_pick_folder_with` pattern: the tests drive the whole flow
//! with their own picker closure (the one untestable step is the OS
//! dialog), so the byte-for-byte export, every refusal and the cancel
//! path run against a real database, a real data root and real files.

use eunomia_core::backup_name::{BackupTime, backup_relative_path};
use eunomia_core::hash::Sha256Hex;
use eunomia_core::project::{BaseName as CoreBaseName, ProjectId, Scheme as CoreScheme};
use eunomia_store::backup_records::{
    BackupRecordStatus, NewBackupRecord, get, insert, list_for_project,
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

/// The `backup_save` harness: a mock app whose `DataRoot` is the temp
/// root (the copy files live under it), the command registered with
/// the production wrapper — the tests drive [`backup_save_with`]
/// directly with their own picker (the `dialog_pick_folder` pattern),
/// and the refusals that never reach the seam also run over the IPC
/// wire through the registered wrapper.
fn save_app() -> (
    tempfile::TempDir,
    tauri::App<tauri::test::MockRuntime>,
    std::path::PathBuf,
) {
    let temp = tempfile::tempdir().expect("temp directory creates");
    crate::data_dirs::ensure_data_layout(temp.path()).expect("data layout creates");
    let database = crate::db::Db::open(temp.path()).expect("database opens");
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![crate::backups::backup_save,])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds");
    app.manage(crate::data_dirs::DataRoot(temp.path().to_path_buf()));
    app.manage(database);
    let exports = temp.path().join("exports");
    (temp, app, exports)
}

fn save_app_without_db() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![crate::backups::backup_save,])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds")
}

/// The FIPS 180-2 "abc" vector — the independent digest oracle this
/// suite's seeded copies use (the content and the expected digest are
/// both literals from the standard's test vectors, never values the
/// command computed).
const ABC_BYTES: &[u8] = b"abc";
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// Seeds one `ok` record whose copy file REALLY exists under the temp
/// data root with exactly `ABC_BYTES` content, the record's
/// `size_bytes`/`sha256` columns being that content's real facts (the
/// FIPS vector pinned first, so the row and the file agree the way a
/// pipeline-written row would). Returns the row's id, its §5.5
/// relative path, and the copy's absolute path.
fn seed_verifiable_copy(
    transaction: &rusqlite::Transaction<'_>,
    temp: &std::path::Path,
    project_id: &str,
    project_name: &str,
    run_id: &str,
) -> Result<(String, String, std::path::PathBuf), crate::errors::AppError> {
    // The oracle first: the digest of the vector content is the
    // vector's own expected value.
    let mut hasher = Sha256Hex::new();
    hasher.update(ABC_BYTES);
    let sha256 = hasher.finalize_hex();
    assert_eq!(sha256, ABC_SHA256, "the FIPS 180-2 'abc' vector must hold");

    let time = BackupTime {
        year: 2026,
        month: 11,
        day: 21,
        hour: 12,
        minute: 0,
        second: 1,
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
            sync_run_id: Some(run_id),
            source_path: "/repos/repo-1/AGENTS.md",
            backup_path: &backup_path,
            size_bytes: i64::try_from(ABC_BYTES.len()).expect("a small size"),
            sha256_hex: &sha256,
            status: BackupRecordStatus::Ok,
            created_at: "2026-11-21T12:00:01.000Z",
        },
    )
    .map_err(crate::db::backup_records_to_app_error)?;
    let copy_path = temp.join(&backup_path);
    std::fs::create_dir_all(copy_path.parent().expect("the §5.5 layout has a parent")).map_err(
        |source| {
            let mut error = crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
            error.details = Some(crate::errors::ErrorDetails {
                os_code: source.raw_os_error(),
                os_kind: Some("TestSetup".to_owned()),
                os_message: Some(source.to_string()),
            });
            error
        },
    )?;
    std::fs::write(&copy_path, ABC_BYTES).map_err(|source| {
        let mut error = crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
        error.details = Some(crate::errors::ErrorDetails {
            os_code: source.raw_os_error(),
            os_kind: Some("TestSetup".to_owned()),
            os_message: Some(source.to_string()),
        });
        error
    })?;
    Ok((id, backup_path, copy_path))
}

/// A test picker that records every consultation (the suggested names
/// it saw) and answers the pre-arranged destination: `None` for
/// `cancel`, `Some(path)` otherwise. The recording makes "the dialog
/// never opened" a falsifiable assertion on every refusal path.
struct RecordingPicker {
    seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    answer: Option<std::path::PathBuf>,
}

impl RecordingPicker {
    fn new(answer: Option<std::path::PathBuf>) -> Self {
        Self {
            seen: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            answer,
        }
    }

    /// The shared recording — hold it before [`Self::drive`] consumes
    /// the picker, read it after.
    fn recording(&self) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
        std::sync::Arc::clone(&self.seen)
    }

    /// Drives [`crate::backups::backup_save_with`] over `app` with
    /// this picker (the whole flow minus the OS dialog — the
    /// `dialog_pick_folder_with` pattern).
    fn drive(
        self,
        app: &tauri::App<tauri::test::MockRuntime>,
        id: String,
    ) -> Result<Option<String>, crate::errors::AppError> {
        let seen = std::sync::Arc::clone(&self.seen);
        let answer = self.answer;
        let handle = app.handle().clone();
        tauri::async_runtime::block_on(crate::backups::backup_save_with(
            handle,
            id,
            move |_app: &tauri::AppHandle<tauri::test::MockRuntime>, suggested: &str| {
                seen.lock()
                    .expect("the picker's recording lock")
                    .push(suggested.to_owned());
                Ok(answer)
            },
        ))
    }
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

// ---- `backup_save` (BKP-012, T-353) ------------------------------------

#[test]
fn backup_save_writes_the_copy_byte_for_byte_to_the_chosen_path() {
    let (temp, app, exports) = save_app();
    let data_root = temp.path().to_path_buf();
    std::fs::create_dir(&exports).expect("the exports folder creates");
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        let (id, backup_path, copy_path) =
            seed_verifiable_copy(tx, &data_root, &project, "Project 1", &run)?;
        Ok((project, id, backup_path, copy_path))
    }))
    .expect("the seed transaction commits");
    let (project, id, backup_path, copy_path) = seeded;

    let destination = exports.join("chosen-name.md");
    let picker = RecordingPicker::new(Some(destination.clone()));
    let seen = picker.recording();

    let answer = picker.drive(&app, id.clone()).expect("the export succeeds");

    // The answer: the path actually written.
    assert_eq!(
        answer.as_deref(),
        Some(destination.display().to_string().as_str()),
        "the answer is the written path"
    );
    // BKP-012's acceptance: the saved file is byte-identical to the
    // copy — read both from disk, never compare in-memory oracles.
    let saved = std::fs::read(&destination).expect("the saved file exists");
    let copy = std::fs::read(&copy_path).expect("the copy file still exists");
    assert_eq!(saved, copy, "the saved file is byte-for-byte the copy");
    assert_eq!(saved, ABC_BYTES, "both files hold the seeded content");
    // The source copy is untouched.
    assert_eq!(
        std::fs::read(&copy_path).expect("the copy survives the export"),
        ABC_BYTES
    );
    // The dialog saw exactly the suggested name BKP-012 spells: the
    // copy's file stem, its creation stamp, the target's extension.
    assert_eq!(
        seen.lock().expect("the recording lock").as_slice(),
        ["AGENTS.backup-20261121T120001000Z.md"],
        "the suggested name is <stem>.backup-<stamp>.<ext> of the copy's own name"
    );
    // An export journals nothing and changes no row (BKP-012: no
    // backup record change, no SYS-005, no journal entry).
    assert!(
        journal_of(&database).is_empty(),
        "the export journals nothing"
    );
    let row = tauri::async_runtime::block_on(
        database.run(move |tx| get(tx, &id).map_err(crate::db::backup_records_to_app_error)),
    )
    .expect("the row reads back")
    .expect("the row exists");
    assert_eq!(
        row.status,
        BackupRecordStatus::Ok,
        "the row keeps its status"
    );
    assert_eq!(row.backup_path, backup_path, "the row keeps its path");
    assert_eq!(row.size_bytes, 3, "the row keeps its size");
    assert_eq!(row.sha256, ABC_SHA256, "the row keeps its digest");
    // No second row appeared: the project's whole history still holds
    // exactly the one row the seed wrote (the round-1 P3-4 fix — a
    // by-id read cannot see a second row, the listing can).
    let count_project = project;
    let count = tauri::async_runtime::block_on(database.run(move |tx| {
        list_for_project(tx, &count_project).map_err(crate::db::backup_records_to_app_error)
    }))
    .expect("the listing reads back");
    assert_eq!(count.len(), 1, "the export added no row");
}

#[test]
fn backup_save_refuses_terminal_status_rows_before_the_dialog_opens() {
    // BKP-012: a terminal-status row refuses with
    // E_RESTORE_SOURCE_MISSING naming the record's status. The pruned
    // row's copy file is deliberately LEFT ON DISK (the lingering
    // shape retention cannot produce but tampering can): the STATUS is
    // the refusal's trigger — a mutation that skips it falls through
    // to the file arms and answers a different code, reddening here.
    let (temp, app, _exports) = save_app();
    let data_root = temp.path().to_path_buf();
    let database = database_of(&app);
    // All three terminal statuses (the round-1 P2-2 fix: BKP-012 names
    // every one, and each must be pinned — a mutation that collapses
    // two of them would stay green over a single-status test).
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        let mut rows = Vec::new();
        for (n, status) in [
            (2, BackupRecordStatus::Pruned),
            (3, BackupRecordStatus::Missing),
            (4, BackupRecordStatus::Discarded),
        ] {
            let row = seed_record(
                tx,
                &project,
                "Project 1",
                Some(&run),
                SeedRow {
                    n,
                    status,
                    created_at: "2026-11-21T12:00:04.000Z",
                    size_bytes: 64,
                },
            )?;
            // Each terminal row's own copy file lingers on disk: the
            // STATUS is the refusal's trigger — a mutation that skips
            // it falls through to the file arms and answers a
            // different code, reddening here.
            let lingering = data_root.join(&row.backup_path);
            std::fs::create_dir_all(lingering.parent().expect("the §5.5 layout has a parent"))
                .map_err(|source| {
                    let mut error =
                        crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
                    error.details = Some(crate::errors::ErrorDetails {
                        os_code: source.raw_os_error(),
                        os_kind: Some("TestSetup".to_owned()),
                        os_message: Some(source.to_string()),
                    });
                    error
                })?;
            std::fs::write(&lingering, b"leftover").map_err(|source| {
                let mut error = crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
                error.details = Some(crate::errors::ErrorDetails {
                    os_code: source.raw_os_error(),
                    os_kind: Some("TestSetup".to_owned()),
                    os_message: Some(source.to_string()),
                });
                error
            })?;
            rows.push((row.id, status));
        }
        Ok(rows)
    }))
    .expect("the seed transaction commits");

    for (row_id, status) in seeded {
        let picker = RecordingPicker::new(Some(temp.path().join("never.md")));
        let seen = picker.recording();
        let refusal = picker
            .drive(&app, row_id.clone())
            .expect_err("a terminal-status row refuses the export");

        assert_eq!(
            refusal.code,
            crate::errors::ErrorCode::RestoreSourceMissing,
            "{status:?} refuses with the source-missing code"
        );
        assert_eq!(
            refusal.params.get("status").map(String::as_str),
            Some(status.as_str()),
            "the refusal names the record's status (BKP-012): {status:?}"
        );
        // The wire envelope's full shape (the serialized §7.10 form),
        // pinned on the first status only — the shape is one mapping.
        if status == BackupRecordStatus::Pruned {
            let wire = serde_json::to_value(&refusal).expect("the refusal serializes");
            assert_eq!(wire["code"], serde_json::json!("E_RESTORE_SOURCE_MISSING"));
            assert_eq!(
                wire["message_key"],
                serde_json::json!("errors.restore_source_missing")
            );
            assert_eq!(wire["params"]["status"], serde_json::json!("pruned"));
        }
        // The dialog never opened.
        assert!(
            seen.lock().expect("the recording lock").is_empty(),
            "the {status:?} refusal precedes the dialog"
        );
    }
    assert!(
        journal_of(&database).is_empty(),
        "the refusal journals nothing"
    );
}

#[test]
fn backup_save_refuses_an_unknown_id() {
    let (_temp, app, _exports) = save_app();
    let picker = RecordingPicker::new(None);
    let seen = picker.recording();

    let refusal = picker
        .drive(&app, "no-record-has-this-id".to_owned())
        .expect_err("an unknown id refuses");

    assert_eq!(refusal.code, crate::errors::ErrorCode::Validation);
    assert_eq!(refusal.params.get("field").map(String::as_str), Some("id"));
    assert_eq!(
        refusal.params.get("reason").map(String::as_str),
        Some("no backup record has this id")
    );
    assert!(
        seen.lock().expect("the recording lock").is_empty(),
        "the refusal precedes the dialog"
    );
}

#[test]
fn backup_save_answers_none_when_the_user_cancels() {
    // The cancel answer is `dialog_pick_folder`'s "nothing on cancel"
    // shape: `Ok(None)`, nothing written, nothing journaled.
    let (temp, app, exports) = save_app();
    let data_root = temp.path().to_path_buf();
    std::fs::create_dir(&exports).expect("the exports folder creates");
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        seed_verifiable_copy(tx, &data_root, &project, "Project 1", &run)
    }))
    .expect("the seed transaction commits");
    let (id, _backup_path, _copy_path) = seeded;

    let picker = RecordingPicker::new(None);
    let seen = picker.recording();
    let answer = picker.drive(&app, id).expect("a cancel is not an error");

    assert_eq!(answer, None, "nothing was saved");
    assert_eq!(
        seen.lock().expect("the recording lock").as_slice(),
        ["AGENTS.backup-20261121T120001000Z.md"],
        "the dialog did open, with the suggested name"
    );
    assert!(
        std::fs::read_dir(&exports)
            .expect("the exports folder lists")
            .next()
            .is_none(),
        "cancel writes nothing"
    );
    assert!(journal_of(&database).is_empty(), "cancel journals nothing");
}

#[test]
fn backup_save_refuses_when_the_copy_is_not_the_records_bytes() {
    // The verify-mismatch reading (D-203 (2), §9.5 semantics): a copy
    // whose bytes are not the record's size/sha256 columns is
    // E_VERIFY_FAILED — measured before the dialog and before any
    // destination byte moves, so the user is never asked where to put
    // bytes that are not the record's.
    let (temp, app, _exports) = save_app();
    let data_root = temp.path().to_path_buf();
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        seed_verifiable_copy(tx, &data_root, &project, "Project 1", &run)
    }))
    .expect("the seed transaction commits");
    let (id, _backup_path, copy_path) = seeded;
    // The copy on disk is substituted: same length, wrong bytes.
    std::fs::write(&copy_path, b"xyz").expect("the substituted copy writes");

    let picker = RecordingPicker::new(Some(temp.path().join("never.md")));
    let seen = picker.recording();
    let refusal = picker
        .drive(&app, id.clone())
        .expect_err("a copy that is not the record's bytes refuses");

    assert_eq!(refusal.code, crate::errors::ErrorCode::VerifyFailed);
    assert_eq!(
        refusal.params.get("id").map(String::as_str),
        Some(id.as_str())
    );
    assert_eq!(
        refusal
            .details
            .as_ref()
            .and_then(|details| details.os_kind.as_deref()),
        Some("BackupExportVerifyFailed")
    );
    assert!(
        seen.lock().expect("the recording lock").is_empty(),
        "the refusal precedes the dialog — no destination was ever asked for"
    );
    // The substituted copy is left exactly as the disk holds it (the
    // export refuses; it does not "clean up" a copy it does not own).
    assert_eq!(
        std::fs::read(&copy_path).expect("the copy is untouched"),
        b"xyz"
    );
    assert!(
        journal_of(&database).is_empty(),
        "the refusal journals nothing"
    );
}

#[test]
fn backup_save_refuses_when_the_ok_rows_file_is_gone_from_disk() {
    // The honest arm §10's row names: the row still says `ok`, the
    // file is gone — the export refuses with E_RESTORE_SOURCE_MISSING
    // and the record KEEPS its stored status (no `missing` transition;
    // that detection is §10's future task).
    let (temp, app, _exports) = save_app();
    let data_root = temp.path().to_path_buf();
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        seed_verifiable_copy(tx, &data_root, &project, "Project 1", &run)
    }))
    .expect("the seed transaction commits");
    let (id, backup_path, copy_path) = seeded;
    std::fs::remove_file(&copy_path).expect("the copy is gone");

    let picker = RecordingPicker::new(Some(temp.path().join("never.md")));
    let seen = picker.recording();
    let refusal = picker
        .drive(&app, id.clone())
        .expect_err("a gone copy refuses");

    assert_eq!(refusal.code, crate::errors::ErrorCode::RestoreSourceMissing);
    assert_eq!(
        refusal.params.get("status").map(String::as_str),
        Some("ok"),
        "the record keeps its stored status, and the refusal names it"
    );
    assert!(
        refusal.params.contains_key("path"),
        "the gone copy's path travels in params (SEC-008: paths may)"
    );
    assert_eq!(
        refusal
            .details
            .as_ref()
            .and_then(|details| details.os_kind.as_deref()),
        Some("NotFound")
    );
    assert!(
        seen.lock().expect("the recording lock").is_empty(),
        "the refusal precedes the dialog"
    );
    let row = tauri::async_runtime::block_on(
        database.run(move |tx| get(tx, &id).map_err(crate::db::backup_records_to_app_error)),
    )
    .expect("the row reads back")
    .expect("the row exists");
    assert_eq!(row.status, BackupRecordStatus::Ok, "no status flip");
    assert_eq!(row.backup_path, backup_path, "no row change at all");
    assert!(
        journal_of(&database).is_empty(),
        "the refusal journals nothing"
    );
}

#[test]
#[cfg(unix)]
fn backup_save_refuses_a_save_path_the_wire_cannot_carry() {
    // The round-1 P2-3 fix's pin: a destination whose text is not
    // valid UTF-8 (Unix `FilePath::into_path` can hand one back) is
    // refused BEFORE the write — no file lands at a location the
    // answer could only misname (`display()` would silently replace
    // the offending bytes). The D-086 dialog-boundary posture,
    // `dialog_pick_folder`'s twin.
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let (temp, app, exports) = save_app();
    std::fs::create_dir(&exports).expect("the exports folder creates");
    let data_root = temp.path().to_path_buf();
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        seed_verifiable_copy(tx, &data_root, &project, "Project 1", &run)
    }))
    .expect("the seed transaction commits");
    let (id, _backup_path, _copy_path) = seeded;

    // An ABSOLUTE destination under the temp root with a non-UTF-8
    // file name (the round-2 P2-1 fix: a bare relative name would be
    // refused by the §9.5 no-parent check anyway, so the "nothing was
    // written" half could not see a write-first mutation's output — a
    // writable parent makes the write POSSIBLE, and the refusal's job
    // is to prevent it).
    let destination = exports.join(OsString::from_vec(b"chosen-\xff.md".to_vec()));
    let picker = RecordingPicker::new(Some(destination.clone()));
    let refusal = picker
        .drive(&app, id)
        .expect_err("a path the wire cannot carry refuses");

    assert_eq!(refusal.code, crate::errors::ErrorCode::Validation);
    assert_eq!(
        refusal.params.get("field").map(String::as_str),
        Some("path")
    );
    assert!(
        refusal.params.contains_key("reason"),
        "the refusal names why the path is refused"
    );
    // Nothing was written: the exact destination does not exist (the
    // write never ran — the P2-3 claim, now falsifiable)…
    assert!(
        !destination.exists(),
        "the refused path was never written: {}",
        destination.display()
    );
    // …and no file with those bytes exists anywhere under the temp
    // root.
    let mut written = Vec::new();
    for entry in walkdir(temp.path()) {
        if entry.is_file()
            && entry
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains("chosen"))
        {
            written.push(entry);
        }
    }
    assert!(
        written.is_empty(),
        "the refused path was never written: {written:?}"
    );
    assert!(
        journal_of(&database).is_empty(),
        "the refusal journals nothing"
    );
}

/// Every path under `root`, recursively — the refusal tests' "nothing
/// was written" oracle (a plain walk; the scan machinery is the
/// feature under test elsewhere).
fn walkdir(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the temp tree lists") {
            let path = entry.expect("the entry reads").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn backup_save_refuses_a_refused_write() {
    // §9.5's write failing maps through the shared `write_code` table
    // (D-203 (2)): a destination whose folder does not exist makes the
    // staging create fail — E_WRITE_FAILED, the destination path in
    // params, the OS facts in details.
    let (temp, app, _exports) = save_app();
    let data_root = temp.path().to_path_buf();
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        seed_verifiable_copy(tx, &data_root, &project, "Project 1", &run)
    }))
    .expect("the seed transaction commits");
    let (id, _backup_path, _copy_path) = seeded;

    // A destination in a folder that does not exist: §9.5's staging
    // create refuses (the T-F7 shape), and the module's own tests
    // prove the old-file-stays-whole half; this pins the command's
    // mapping of the refusal.
    let destination = temp.path().join("no-such-folder").join("chosen.md");
    let picker = RecordingPicker::new(Some(destination.clone()));
    let refusal = picker
        .drive(&app, id)
        .expect_err("a refused write refuses the export");

    assert_eq!(refusal.code, crate::errors::ErrorCode::WriteFailed);
    assert_eq!(
        refusal.params.get("path").map(String::as_str),
        Some(destination.display().to_string().as_str())
    );
    assert!(
        journal_of(&database).is_empty(),
        "the refusal journals nothing"
    );
}

#[test]
fn backup_save_refuses_a_corrupt_backup_path_cell() {
    // A stored `backup_path` that does not spell the §5.5 grammar is
    // the corrupt-cell envelope (the `backup_records` Member posture):
    // the layout is validated BEFORE the join, so a tampered absolute
    // path never reaches the filesystem.
    let (temp, app, _exports) = save_app();
    let data_root = temp.path().to_path_buf();
    let database = database_of(&app);
    let seeded = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = seed_project(tx, 1)?;
        let run = seed_run(tx, &project, 1)?;
        let (id, _path, _copy) = seed_verifiable_copy(tx, &data_root, &project, "Project 1", &run)?;
        tx.execute(
            "UPDATE backup_records SET backup_path = '/etc/passwd' WHERE id = ?1",
            rusqlite::params![&id],
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(id)
    }))
    .expect("the seed transaction commits");
    let id = seeded;

    let picker = RecordingPicker::new(Some(temp.path().join("never.md")));
    let seen = picker.recording();
    let refusal = picker
        .drive(&app, id)
        .expect_err("a corrupt backup_path cell refuses");

    assert_eq!(refusal.code, crate::errors::ErrorCode::DbLocked);
    assert_eq!(
        refusal
            .details
            .as_ref()
            .and_then(|details| details.os_kind.as_deref()),
        Some("BackupRecordsCorrupt")
    );
    assert!(
        seen.lock().expect("the recording lock").is_empty(),
        "the refusal precedes the dialog"
    );
}

#[test]
fn the_backup_save_command_answers_honestly_without_a_database() {
    let app = save_app_without_db();
    let webview = main_webview(&app);
    let saved: Result<Option<String>, serde_json::Value> =
        invoke(&webview, "backup_save", serde_json::json!({ "id": "any" }));
    let error = saved.expect_err("the export refuses without a database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"));
    assert_eq!(error["details"]["os_kind"], serde_json::json!("NoDatabase"));
}

#[test]
fn the_backup_save_unknown_id_refusal_travels_the_wire() {
    // The registered production wrapper over the mock IPC harness:
    // the unknown-id refusal never reaches the dialog seam, so the
    // full wire envelope — code, message_key, params — is pinned end
    // to end through the registration the app ships.
    let (_temp, app, _exports) = save_app();
    let webview = main_webview(&app);
    let saved: Result<Option<String>, serde_json::Value> = invoke(
        &webview,
        "backup_save",
        serde_json::json!({ "id": "missing" }),
    );
    let error = saved.expect_err("an unknown id refuses over the wire");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"));
    assert_eq!(error["message_key"], serde_json::json!("errors.validation"));
    assert_eq!(error["params"]["field"], serde_json::json!("id"));
    assert_eq!(
        error["params"]["reason"],
        serde_json::json!("no backup record has this id")
    );
}

#[test]
fn the_backup_save_command_is_registered_once_with_specta() {
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    assert_eq!(
        lib.matches("backups::backup_save").count(),
        1,
        "lib.rs registers `backups::backup_save` once"
    );
}
