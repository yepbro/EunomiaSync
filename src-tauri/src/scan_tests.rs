//! Mock-runtime tests of the scan commands (T-076, §7.10 Projects
//! module, §9.6, PRJ-002/005/006, SYS-003, NFR-015, ALG-053,
//! ERR-002, D-153) — the `ipc_tests.rs` harness shape: a real
//! `Db` over a temp root, the SEC-003 picks driven through the real
//! dialog seam (`dialog_pick_folder_with`, so the session allowlist
//! holds exactly what a production pick leaves), real `.git`
//! fixtures, and the `scan://progress` event mounted the way
//! `lib.rs`'s setup mounts it.
//!
//! The card's acceptance pair leads: **a cancelled scan adds no
//! projects** (the mid-walk cancel is deterministic — the progress
//! listener latches the token from INSIDE the walk, the D-147
//! "three deterministic tests instead of one racy one"
//! composition: the first emission proves the walk is live, the
//! synchronous listener delivery proves the latch lands mid-walk,
//! and hundreds of remaining entries guarantee a later entry
//! observes it), and **the event rate is bounded** (pinned at the
//! throttle's own seam over a simulated clock in `scan.rs`'s unit
//! suite — no wall-second sleeps anywhere).
//!
//! The completed scan is pinned end to end: the four ALG-053
//! classes against a real seeded registry, the walker's skip
//! vocabulary on the wire, the counts' arithmetic (checked = every
//! walk event, found = repository events), the progress payloads'
//! monotone climb capped by the answer's terminal counts — and the
//! ERR-002 twin: the database is proven UNCHANGED (projects,
//! exclusions, journal — a scan writes nothing anywhere, by
//! construction; the census makes it a pinned fact). The remaining
//! pins: the pre-latched token, the slot lifecycle (empty → Some
//! mid-walk → empty on every answer), the second-start refusal,
//! SEC-003, the settings shaping the walk, the honest no-database
//! answer, and the registration pins.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use eunomia_fs::scan::CancelToken;
use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri_specta::Event as _;

use crate::scan::{
    ScanCancelSlot, ScanClassWire, ScanProgressEvent, ScanRunOutcome, ScanSkipReason,
};

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the scan commands serve: both
/// commands registered over the mock runtime, a fresh database in a
/// temp root, the SEC-003 session allowlist, the scan cancel slot
/// (the two states `lib.rs`'s setup manages for this feature), and
/// the `scan://progress` event mounted.
fn scan_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::scan::scan_start,
            crate::scan::scan_cancel
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::session_paths::SessionPaths::new());
    app.manage(ScanCancelSlot::new());
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![ScanProgressEvent])
        .mount_events(&app);
    (temp, app)
}

/// The mock `main` webview window the IPC requests go through (the
/// `ipc_tests.rs` shape).
fn main_webview(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("failed to create the mock `main` webview window")
}

/// One invoke request for `cmd` with a JSON body (the `ipc_tests.rs`
/// shape).
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

/// Invokes `scan_start` over the real IPC envelope, deserializing
/// the success payload into the typed outcome; errors come back as
/// the serialized `AppError` JSON.
fn invoke_scan_start(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    path: &str,
) -> Result<ScanRunOutcome, serde_json::Value> {
    match tauri::test::get_ipc_response(
        webview,
        invoke_request("scan_start", serde_json::json!({ "path": path })),
    ) {
        Ok(payload) => Ok(payload
            .deserialize::<ScanRunOutcome>()
            .expect("the success payload must deserialize into the outcome DTO")),
        Err(error) => Err(error),
    }
}

/// Invokes `scan_cancel` over the real IPC envelope.
fn invoke_scan_cancel(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> Result<(), serde_json::Value> {
    match tauri::test::get_ipc_response(
        webview,
        invoke_request("scan_cancel", serde_json::json!({})),
    ) {
        // The unit answer crosses as JSON `null`; an `Ok` response is
        // itself the shape proof — no unit value is passed through.
        Ok(_payload) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Picks `folder` through the REAL dialog seam (`dialog_pick_folder_
/// with` with a fixed picker answer), so the session allowlist holds
/// exactly what a production pick leaves — the canonical form the
/// SEC-003 gate then accepts. The `ipc_tests.rs` `pick_folder` shape.
fn pick_folder(app: &tauri::App<tauri::test::MockRuntime>, folder: &Path) -> String {
    // The picker closure must own its answer (`'static`, the seam's
    // bound), so the folder crosses as an owned copy.
    let picked = folder.to_path_buf();
    tauri::async_runtime::block_on(crate::commands::dialog_pick_folder_with(
        app.handle().clone(),
        move |_app| Ok(Some(picked)),
    ))
    .expect("the pick must succeed")
    .expect("a completed pick answers a path")
}

/// Subscribes the typed listener the frontend's generated
/// `events.scanProgress` mirrors, returning the receiver every
/// emitted payload lands on. Delivery to Rust listeners is
/// synchronous inside `emit` (the availability suite's rule) — which
/// is what makes the mid-walk cancel test deterministic.
fn listen_progress(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> std::sync::mpsc::Receiver<ScanProgressEvent> {
    let (sender, receiver) = std::sync::mpsc::channel();
    ScanProgressEvent::listen(app, move |event| {
        let _ = sender.send(event.payload);
    });
    receiver
}

/// The managed database handle.
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

/// Creates one `.git`-directory repository fixture directly under
/// `parent` (the canonical root form the answer reports).
fn repo_under(parent: &Path, name: &str) -> std::path::PathBuf {
    let repo = parent.join(name);
    std::fs::create_dir_all(repo.join(".git")).expect("the fixture repository must be creatable");
    eunomia_fs::paths::canonicalize(&repo).expect("the fresh fixture canonicalizes")
}

/// Creates one `.git`-FILE folder (the submodule/worktree form).
fn git_file_under(parent: &Path, name: &str) -> std::path::PathBuf {
    let folder = parent.join(name);
    std::fs::create_dir_all(&folder).expect("the fixture folder must be creatable");
    std::fs::write(folder.join(".git"), "gitdir: ../elsewhere\n")
        .expect("writing the gitdir pointer must work");
    eunomia_fs::paths::canonicalize(&folder).expect("the fresh fixture canonicalizes")
}

/// Seeds one ACTIVE project row for `repo` (the registry's `known`
/// set), through the repository the add flow writes.
fn seed_active_project(app: &tauri::App<tauri::test::MockRuntime>, repo: &Path, label: &str) {
    seed_project(app, repo, label, false);
}

/// Seeds one ARCHIVED project row for `repo` (the registry's
/// `archived` set).
fn seed_archived_project(app: &tauri::App<tauri::test::MockRuntime>, repo: &Path, label: &str) {
    seed_project(app, repo, label, true);
}

/// The shared seeding body: create, then archive when asked.
fn seed_project(
    app: &tauri::App<tauri::test::MockRuntime>,
    repo: &Path,
    label: &str,
    archived: bool,
) {
    let path = Box::leak(repo.to_string_lossy().into_owned().into_boxed_str());
    let key = Box::leak(
        eunomia_fs::paths::path_key(repo)
            .expect("the fixture path keys")
            .into_boxed_str(),
    );
    let label = Box::leak(label.to_owned().into_boxed_str());
    let database = database_of(app);
    let id = tauri::async_runtime::block_on(database.run(move |tx| {
        let project = eunomia_store::projects::create(
            tx,
            &eunomia_store::projects::NewProject {
                path,
                path_key: key,
                name: label,
                label,
                color: "blue",
                description: "",
                added_via: eunomia_store::projects::AddedVia::Manual,
                scheme: eunomia_store::settings::Scheme::A,
                base_name: eunomia_store::settings::BaseName::AgentsMd,
                extra_local: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
                extra_project: eunomia_store::settings::ExtraFile {
                    enabled: false,
                    position: eunomia_store::settings::Position::End,
                },
            },
        )
        .map_err(crate::commands::projects_store_to_app_error)?;
        if archived {
            eunomia_store::projects::archive(tx, &project.id)
                .map_err(crate::commands::projects_store_to_app_error)?;
        }
        Ok(project.id)
    }))
    .expect("seeding a project through the repository must work");
    let _ = id;
}

/// Seeds one `project_exclusions` row for `repo` (the registry's
/// `excluded` set).
fn seed_exclusion(app: &tauri::App<tauri::test::MockRuntime>, repo: &Path) {
    let path = Box::leak(repo.to_string_lossy().into_owned().into_boxed_str());
    let key = Box::leak(
        eunomia_fs::paths::path_key(repo)
            .expect("the fixture path keys")
            .into_boxed_str(),
    );
    let database = database_of(app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create_exclusion(
            tx,
            &eunomia_store::projects::NewExclusion {
                path,
                path_key: key,
                reason: eunomia_store::projects::ExclusionReason::Manual,
            },
        )
        .map(|_| ())
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding an exclusion through the repository must work");
}

/// Seeds one §6.5 setting through the store (the `ipc_tests.rs`
/// `seed_setting` shape).
fn seed_setting(
    app: &tauri::App<tauri::test::MockRuntime>,
    key: eunomia_store::settings::SettingKey,
    value: eunomia_store::settings::SettingValue,
) {
    let database = database_of(app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::settings::set(tx, key, value.clone())
            .map_err(crate::commands::settings_write_to_app_error)
    }))
    .expect("seeding a setting through the store must work");
}

/// The ERR-002 census: how many project rows, exclusion rows and
/// journal rows the database holds — the "nothing was persisted"
/// assertions compare this triple before and after a scan.
fn stored_counts(app: &tauri::App<tauri::test::MockRuntime>) -> (i64, i64, i64) {
    let database = database_of(app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        let projects: i64 = tx
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let exclusions: i64 = tx
            .query_row("SELECT COUNT(*) FROM project_exclusions", [], |row| {
                row.get(0)
            })
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let journal: i64 = tx
            .query_row("SELECT COUNT(*) FROM action_log", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok((projects, exclusions, journal))
    }))
    .expect("reading the stored counts must work")
}

/// The slot's occupancy — the lifecycle observable.
fn slot_token(app: &tauri::App<tauri::test::MockRuntime>) -> Option<Arc<CancelToken>> {
    app.state::<ScanCancelSlot>().token()
}

// ---------------------------------------------------------------------------
// The acceptance pair
// ---------------------------------------------------------------------------

#[test]
fn a_cancelled_scan_answers_cancelled_and_persists_nothing() {
    // THE card criterion: cancelling mid-walk adds no projects — and
    // the stronger ERR-002 sentence: an interrupted scan SAVES
    // NOTHING (no projects, no exclusions, not even a journal row).
    // The mid-walk cancel is deterministic (the module docs' D-147
    // composition): the first progress emission proves the walk is
    // live, the synchronous listener delivery latches the token from
    // inside the walk, and the hundreds of plain folders after the
    // first visited entry guarantee a later entry observes the latch.
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    std::fs::create_dir_all(&root).expect("the scan root fixture must be creatable");
    // One repository early in the walk's name space plus hundreds of
    // plain folders: whatever order the parallel walk visits them in,
    // entries remain after the first emission.
    let found_repo = repo_under(&root, "000-repo");
    for index in 0..400 {
        std::fs::create_dir_all(root.join(format!("{index:03}-folder")))
            .expect("the plain folder fixture must be creatable");
    }
    let picked = pick_folder(&app, &eunomia_fs::paths::canonicalize(&root).unwrap());
    let before = stored_counts(&app);

    // The listener: on the FIRST emission, capture that the slot was
    // occupied (the lifecycle's mid-walk fact) and latch the live
    // scan's own token through the slot — the exact read
    // `scan_cancel` makes.
    let cancel_from = app.handle().clone();
    let slot_held_during_walk = Arc::new(AtomicBool::new(false));
    let latched = Arc::new(AtomicBool::new(false));
    let seen_held = Arc::clone(&slot_held_during_walk);
    let seen_latch = Arc::clone(&latched);
    ScanProgressEvent::listen(&app, move |_event| {
        if seen_latch.swap(true, Ordering::SeqCst) {
            return; // latch once; later emissions (in flight) are noise
        }
        seen_held.store(true, Ordering::SeqCst);
        if let Some(token) = cancel_from
            .try_state::<ScanCancelSlot>()
            .and_then(|slot| slot.token())
        {
            token.cancel();
        }
    });

    let answer = tauri::async_runtime::block_on(crate::scan::scan_start_with(
        app.handle().clone(),
        picked,
        Arc::new(CancelToken::new()),
    ))
    .expect("a cancelled scan answers Ok — the user asked for it, no error envelope");

    // The Cancelled arm carries NO payload (the structural ERR-002
    // pin): there is no repositories field to read on it at all.
    assert!(
        matches!(answer, ScanRunOutcome::Cancelled),
        "the mid-walk latch must answer the Cancelled outcome; got {answer:?}"
    );
    assert!(
        latched.load(Ordering::SeqCst),
        "the listener must have observed a progress emission (the walk went live)"
    );
    assert!(
        slot_held_during_walk.load(Ordering::SeqCst),
        "the slot must hold the live scan's token while the walk runs"
    );
    assert!(
        slot_token(&app).is_none(),
        "the slot is empty again after the answer — the lease cleared it"
    );
    // ERR-002: nothing was persisted anywhere.
    assert_eq!(
        stored_counts(&app),
        before,
        "a cancelled scan must not touch projects, exclusions or the journal"
    );
    // The repository that WAS found before the latch: not in any
    // answer, and not in the store (the census above) — the walk's
    // accumulation was dropped, and `found_repo` exists only on
    // disk.
    let _ = found_repo;
}

#[test]
fn a_pre_cancelled_token_answers_cancelled_with_no_events() {
    // The deterministic twin of the cancel path (the walker's own
    // pinned behavior, composed through the command): a token
    // latched BEFORE the start makes the walk quit on its first
    // entry — zero events, zero emissions, the empty answer, and the
    // slot cleared on the way out.
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    let repo = repo_under(&root, "repo");
    let _ = repo;
    let picked = pick_folder(&app, &eunomia_fs::paths::canonicalize(&root).unwrap());
    let before = stored_counts(&app);
    let progress = listen_progress(&app);

    let token = Arc::new(CancelToken::new());
    token.cancel();
    let answer = tauri::async_runtime::block_on(crate::scan::scan_start_with(
        app.handle().clone(),
        picked,
        token,
    ))
    .expect("a pre-cancelled scan answers Ok(Cancelled), not Err");

    assert!(matches!(answer, ScanRunOutcome::Cancelled));
    assert!(
        progress.try_recv().is_err(),
        "a pre-latched token emits no progress events at all"
    );
    assert_eq!(stored_counts(&app), before, "nothing was persisted");
    assert!(
        slot_token(&app).is_none(),
        "the slot is empty after the answer"
    );
}

// ---------------------------------------------------------------------------
// The completed scan
// ---------------------------------------------------------------------------

#[test]
fn a_completed_scan_answers_the_four_classes_and_writes_nothing() {
    // The PRJ-002/ALG-053 answer over a real seeded registry: each
    // of the four classes lands on its repository (new twice — the
    // nested one is found separately, ALG-051), the walker's skip
    // vocabulary rides the wire (the D-05 default list skips each
    // found repository's own `.git` folder as `excluded`, and the
    // `.git`-file folder as `submodule_or_worktree`), and the
    // counters' arithmetic holds: `checked` counts EVERY walk event,
    // `found` the repository events. The ERR-002 twin: the census
    // proves the scan wrote nothing anywhere.
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    std::fs::create_dir_all(&root).expect("the scan root fixture must be creatable");
    let repo_new = repo_under(&root, "repo-new");
    let repo_known = repo_under(&root, "repo-known");
    let repo_archived = repo_under(&root, "repo-archived");
    let repo_excluded = repo_under(&root, "repo-excluded");
    let nested_parent = root.join("nested");
    let repo_nested = repo_under(&nested_parent, "repo-nested");
    std::fs::create_dir_all(root.join("plain"))
        .expect("the plain folder fixture must be creatable");
    let submodule = git_file_under(&root, "submodule");
    seed_active_project(&app, &repo_known, "Known");
    seed_archived_project(&app, &repo_archived, "Arch");
    seed_exclusion(&app, &repo_excluded);
    let picked = pick_folder(&app, &eunomia_fs::paths::canonicalize(&root).unwrap());
    let before = stored_counts(&app);
    let progress = listen_progress(&app);

    let webview = main_webview(&app);
    let answer = invoke_scan_start(&webview, &picked).expect("the completed scan answers Ok");

    let ScanRunOutcome::Completed {
        checked,
        found,
        repositories,
        skipped,
    } = answer
    else {
        panic!("the uncanceled scan must complete; got {answer:?}");
    };

    // The classified set: five found repositories, one per class
    // position (new twice), keyed and spelled exactly as walked.
    let by_path = |path: &Path| {
        let wanted = path.to_string_lossy().into_owned();
        repositories
            .iter()
            .find(|repo| repo.path == wanted)
            .unwrap_or_else(|| {
                panic!("the repository {wanted:?} must be reported; got {repositories:?}")
            })
    };
    assert_eq!(
        found, 5,
        "five repository events (all classes); got {found}"
    );
    assert_eq!(
        by_path(&repo_new).class,
        ScanClassWire::New,
        "an unregistered repository classifies new"
    );
    assert_eq!(
        by_path(&repo_known).class,
        ScanClassWire::Known,
        "an active project's repository classifies known"
    );
    assert_eq!(
        by_path(&repo_archived).class,
        ScanClassWire::Archived,
        "an archived project's repository classifies archived"
    );
    assert_eq!(
        by_path(&repo_excluded).class,
        ScanClassWire::Excluded,
        "an excluded path's repository classifies excluded"
    );
    assert_eq!(
        by_path(&repo_nested).class,
        ScanClassWire::New,
        "the nested repository is found separately (ALG-051)"
    );
    assert_eq!(
        by_path(&repo_new).path_key,
        eunomia_fs::paths::path_key(&repo_new).expect("the fixture keys"),
        "the reported key is the registered identity (DB-030)"
    );

    // The skip vocabulary: the `.git`-file folder with its exact
    // reason, and the D-05 default list skipping each found
    // repository's own `.git` folder as `excluded`.
    assert!(
        skipped
            .iter()
            .any(|skip| skip.path == submodule.to_string_lossy()
                && skip.reason == ScanSkipReason::SubmoduleOrWorktree
                && skip.os_code.is_none()),
        "the .git-file folder is skipped with the submodule-or-worktree reason; got {skipped:?}"
    );
    for repo in [
        &repo_new,
        &repo_known,
        &repo_archived,
        &repo_excluded,
        &repo_nested,
    ] {
        let git_dir = repo.join(".git").to_string_lossy().into_owned();
        assert!(
            skipped
                .iter()
                .any(|skip| skip.path == git_dir && skip.reason == ScanSkipReason::Excluded),
            "the default exclude list (D-05) skips {git_dir:?}; got {skipped:?}"
        );
    }

    // The counters' arithmetic: every walk event counts as checked —
    // 1 root + 5 repositories + 5 `.git` skips + 1 submodule skip +
    // 1 `nested` + 1 `plain` folder = 14; `found` counts repository
    // events only.
    assert_eq!(
        checked, 14,
        "checked counts every walk event (root, 5 repos, 6 skips, 2 plain folders); got {checked}"
    );

    // The progress wire: at least one emission (the first event
    // emits immediately), monotone counters, the spec's three
    // fields, capped by the answer's terminal counts.
    let mut emissions = Vec::new();
    while let Ok(payload) = progress.try_recv() {
        emissions.push(payload);
    }
    assert!(
        !emissions.is_empty(),
        "the walk emits scan://progress (the first event emits immediately)"
    );
    for pair in emissions.windows(2) {
        assert!(
            pair[0].checked <= pair[1].checked && pair[0].found <= pair[1].found,
            "the counters climb monotonically; saw {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    for payload in &emissions {
        assert!(
            payload
                .current_path
                .starts_with(root.to_string_lossy().as_ref())
        );
        assert!(payload.checked <= checked);
        assert!(payload.found <= found);
    }

    // ERR-002's twin over the happy path: a completed scan persists
    // nothing either — `scan_commit` (T-077) is the only writer.
    assert_eq!(
        stored_counts(&app),
        before,
        "a completed scan must not touch projects, exclusions or the journal"
    );
    assert!(
        slot_token(&app).is_none(),
        "the slot is empty after the answer"
    );
}

#[test]
fn the_settings_shape_the_walk() {
    // The §6.5 keys reach the walker: `scan.max_depth` (D-04) bounds
    // the descent (a repository two levels down is not visited with
    // depth 1), and `scan.exclude_names` (D-05) adds a caller-chosen
    // name to the skip list on top of the defaults.
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    std::fs::create_dir_all(&root).expect("the scan root fixture must be creatable");
    let near = repo_under(&root, "near");
    let far_parent = root.join("plain");
    let far = repo_under(&far_parent, "far");
    let secret = repo_under(&root, "secret");
    seed_setting(
        &app,
        eunomia_store::settings::SettingKey::ScanMaxDepth,
        eunomia_store::settings::SettingValue::Int(1),
    );
    seed_setting(
        &app,
        eunomia_store::settings::SettingKey::ScanExcludeNames,
        eunomia_store::settings::SettingValue::StrList(vec!["secret".to_owned()]),
    );
    let picked = pick_folder(&app, &eunomia_fs::paths::canonicalize(&root).unwrap());

    let webview = main_webview(&app);
    let answer =
        invoke_scan_start(&webview, &picked).expect("the scan over the seeded settings answers Ok");
    let ScanRunOutcome::Completed {
        repositories,
        skipped,
        ..
    } = answer
    else {
        panic!("the scan must complete; got {answer:?}");
    };

    let paths: Vec<&str> = repositories.iter().map(|repo| repo.path.as_str()).collect();
    assert!(
        paths.contains(&near.to_string_lossy().as_ref()),
        "the depth-1 repository is found; got {paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.contains("far")),
        "a repository at depth 2 is not visited with scan.max_depth 1; got {paths:?}"
    );
    assert!(
        skipped
            .iter()
            .any(|skip| skip.path == secret.to_string_lossy()
                && skip.reason == ScanSkipReason::Excluded),
        "the caller-chosen exclude name skips its folder; got {skipped:?}"
    );
    let _ = far;
}

// ---------------------------------------------------------------------------
// The cancel command and the slot
// ---------------------------------------------------------------------------

#[test]
fn scan_cancel_latches_the_live_token_and_is_a_no_op_when_no_scan_is_live() {
    // The command's two arms over the real IPC envelope: with
    // nothing live, the idempotent Ok (NFR-015's window-close race
    // must not error); with a token in the slot, the latch lands on
    // the caller's OWN Arc — the identity proof that `scan_cancel`
    // cancels the live scan's flag, not a copy.
    let (temp, app) = scan_app();
    let _ = temp;
    let webview = main_webview(&app);

    invoke_scan_cancel(&webview).expect("a cancel with no scan live is the idempotent no-op");

    let held = Arc::new(CancelToken::new());
    assert!(
        app.state::<ScanCancelSlot>().try_publish(&held),
        "the manual publication wins the empty slot (the externally-held-slot pattern)"
    );
    invoke_scan_cancel(&webview).expect("the cancel over a live token answers Ok");
    assert!(
        held.is_cancelled(),
        "the cancel latched the published token's shared flag"
    );
    assert!(
        slot_token(&app).is_some(),
        "the cancel itself does not clear the slot — the lease of the running scan does"
    );
}

#[test]
fn a_second_scan_start_while_one_is_live_is_refused() {
    // The single-live-scan rule (§10's E_SYNC_RUNNING row, the only
    // \"already running\" vocabulary the closed registry holds): a
    // start that finds the slot occupied answers the typed refusal
    // and publishes nothing — the live scan's token stays untouched
    // and uncancelled.
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    let repo = repo_under(&root, "repo");
    let _ = repo;
    let picked = pick_folder(&app, &eunomia_fs::paths::canonicalize(&root).unwrap());

    let held = Arc::new(CancelToken::new());
    assert!(app.state::<ScanCancelSlot>().try_publish(&held));

    let webview = main_webview(&app);
    let refusal = invoke_scan_start(&webview, &picked).expect_err("the second start is refused");

    assert_eq!(
        refusal.get("code").and_then(|code| code.as_str()),
        Some("E_SYNC_RUNNING"),
        "the refusal is the §10 envelope; got {refusal:?}"
    );
    assert!(
        !held.is_cancelled(),
        "the refused start did not touch the live scan's token"
    );
    assert!(
        slot_token(&app).is_some(),
        "the live scan's token stays published"
    );
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

#[test]
fn an_unpicked_path_is_rejected_by_sec003() {
    // SEC-003 at the scan boundary: a folder that exists on disk but
    // was never picked in the dialog and is no registered project is
    // a typed E_VALIDATION rejection — and the refusal publishes
    // nothing (the slot stays empty; nothing runs).
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    let repo = repo_under(&root, "repo");
    let _ = repo;
    let unpicked = eunomia_fs::paths::canonicalize(&root)
        .unwrap()
        .display()
        .to_string();

    let webview = main_webview(&app);
    let refusal = invoke_scan_start(&webview, &unpicked).expect_err("a foreign path is rejected");

    assert_eq!(
        refusal.get("code").and_then(|code| code.as_str()),
        Some("E_VALIDATION"),
        "the foreign path meets the SEC-003 rejection; got {refusal:?}"
    );
    assert_eq!(
        refusal.get("params").and_then(|params| params.get("field")),
        Some(&serde_json::json!("path")),
        "the rejection names the path field (the settings shape)"
    );
    assert!(
        slot_token(&app).is_none(),
        "the refused start publishes nothing"
    );
}

#[test]
fn the_honest_no_database_answer() {
    // Recovery mode: no `Db` managed. The pick works (the dialog
    // half of SEC-003 needs no database), and the scan answers the
    // recorded no-database fallback — never a panic, never a
    // pretend-empty result (the `managed_db` law).
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::scan::scan_start,
            crate::scan::scan_cancel
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::session_paths::SessionPaths::new());
    app.manage(ScanCancelSlot::new());
    let root = temp.path().join("scan-root");
    let repo = repo_under(&root, "repo");
    let _ = repo;
    let picked = pick_folder(&app, &eunomia_fs::paths::canonicalize(&root).unwrap());

    let webview = main_webview(&app);
    let refusal =
        invoke_scan_start(&webview, &picked).expect_err("a scan without a database is refused");

    assert_eq!(
        refusal.get("code").and_then(|code| code.as_str()),
        Some("E_DB_LOCKED"),
        "the no-database answer is the recorded fallback code; got {refusal:?}"
    );
    assert_eq!(
        refusal
            .get("details")
            .and_then(|details| details.get("os_kind"))
            .and_then(|kind| kind.as_str()),
        Some("NoDatabase"),
        "the fallback diagnostic names the missing state"
    );
    assert!(
        slot_token(&app).is_none(),
        "the lease cleared the publication"
    );
}

// ---------------------------------------------------------------------------
// The registration pins (the D-034 rationale)
// ---------------------------------------------------------------------------

#[test]
fn the_scan_commands_and_event_are_registered_with_the_specta_builder() {
    // Dropping any registration would leave `export_bindings_are_
    // up_to_date` green while silently rewriting the committed
    // bindings without the command/event — the D-034 wiring-pin
    // rationale; each marker exactly once, in the builder and in the
    // `setup` wiring.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    for marker in [
        "scan::scan_cancel",
        "scan::scan_start",
        "scan::ScanProgressEvent",
        "scan::ScanCancelSlot::new()",
    ] {
        assert_eq!(
            lib.matches(marker).count(),
            1,
            "lib.rs must wire {marker} exactly once (§7.10, T-076)"
        );
    }
}

// ---------------------------------------------------------------------------
// The mid-walk occupancy proof (the listener seam is already exercised
// above; this keeps the count observable explicit for future seams)
// ---------------------------------------------------------------------------

#[test]
fn the_progress_emission_count_over_a_real_walk_is_bounded_by_the_throttle_wiring() {
    // The wiring-level bound: over a real walk of thousands of
    // entries the number of emissions is bounded by the walk's wall
    // duration — ceil(duration/100ms) + 1 — which for any tree this
    // suite builds (well under a minute) is far below the event
    // count. The exact ≤10-per-second bound is the throttle unit's
    // simulated-clock pin (no wall-second sleeps here — the D-140
    // flaky-race posture); this pin proves the walk's callback
    // actually consults the throttle (mutation: bypassing it makes
    // this red by an order of magnitude).
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    std::fs::create_dir_all(&root).expect("the scan root fixture must be creatable");
    for index in 0..3000 {
        std::fs::create_dir_all(root.join(format!("folder-{index:04}")))
            .expect("the plain folder fixture must be creatable");
    }
    let picked = pick_folder(&app, &eunomia_fs::paths::canonicalize(&root).unwrap());
    let counter = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&counter);
    ScanProgressEvent::listen(&app, move |_event| {
        seen.fetch_add(1, Ordering::SeqCst);
    });

    let webview = main_webview(&app);
    let started = std::time::Instant::now();
    let answer = invoke_scan_start(&webview, &picked).expect("the walk completes");
    let walked_for = started.elapsed();

    assert!(matches!(answer, ScanRunOutcome::Completed { .. }));
    let emissions = counter.load(Ordering::SeqCst);
    // 3001 folders walked; unthrottled delivery would emit ≥3001
    // events. The throttle's honest ceiling for the observed
    // duration is duration/100ms + 1 — assert well under the
    // unthrottled count and at least one emission.
    assert!(emissions >= 1, "progress events reach the wire");
    let ceiling = (walked_for.as_millis() / 100) as usize + 1;
    assert!(
        emissions <= ceiling,
        "emissions ({emissions}) exceed the throttle's ceiling for a {walked_for:?} walk ({ceiling})"
    );
    assert!(
        emissions < 3000,
        "the walk consulted the throttle (unthrottled delivery would emit one event per folder)"
    );
}
