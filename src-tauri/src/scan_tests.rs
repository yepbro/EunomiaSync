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
//!
//! The commit suite (T-077, §7.10 `scan_commit`, PRJ-002, §8 S2,
//! LOG-001…004, D-179) drives `scan_commit` in the same app as the
//! scan it consumes, with the census of every "nothing was written"
//! claim read back through the store: the card's acceptance pair — a
//! repeated scan and commit create no duplicates; an error on one
//! project rolls back the whole batch (two staged shapes: a refused
//! row, a refused journal entry) — plus the SEC-003 discovered-set
//! rule (an unreported path, a cancelled scan's paths, a folder
//! swapped in under a reported name), the manual-add rules over a
//! batch (the PRJ-012 copy, the DB-042 label, D-29's color across
//! rows, `added_via = scan`, the appended order, one `project.added`
//! entry per row), the typed outcomes for registered and excluded
//! paths, the write-access warnings, the SYS-005 gate, the
//! missing-state answers, a large selection, and the registration
//! pins.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use eunomia_fs::scan::CancelToken;
use eunomia_store::action_log::LogRecord;
use eunomia_store::projects::{AddedVia, Project};
use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri_specta::Event as _;

use crate::scan::{
    ScanCancelSlot, ScanClassWire, ScanCommitResult, ScanFindings, ScanProgressEvent,
    ScanRunOutcome, ScanSkipReason, ScannedRepository,
};

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

/// A mock app in the healthy state the scan commands serve: the three
/// scan commands registered over the mock runtime, a fresh database in
/// a temp root, the SEC-003 session allowlist, the scan cancel slot and
/// the discovered set (the two scan states `lib.rs`'s setup manages),
/// the SYS-005 flag state and the watch session state the commit's
/// post-commit steps read, and the `scan://progress` event mounted.
fn scan_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let (temp, app, _no_service) = scan_app_with_watch_service();
    (temp, app)
}

/// [`scan_app`] with the watch service's bus receiver returned, so a
/// test spawns the REAL service — the commit's watch proofs need the
/// service task, not the stub (the `add_manual_watch_app` shape in
/// `ipc_tests.rs`).
fn scan_app_with_watch_service() -> (
    tempfile::TempDir,
    tauri::App<tauri::test::MockRuntime>,
    tauri::async_runtime::Receiver<crate::file_watch::Command>,
) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::scan::scan_commit,
            crate::scan::scan_start,
            crate::scan::scan_cancel
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::session_paths::SessionPaths::new());
    app.manage(ScanCancelSlot::new());
    app.manage(ScanFindings::new());
    // The SYS-005 gate state the healthy app manages at startup
    // (T-105): without it the commit would meet the gate's honest
    // missing-state fallback instead of the real path under test.
    app.manage(crate::sync_running::SyncRunning::new());
    // The watch session state the setup hook manages (T-257): the
    // commit's created rows await one watch refresh, which reads this
    // state. The bus's RECEIVING half travels back to the caller: the
    // plain `scan_app` drops it — no service is spawned there, so the
    // awaited seam answers its honest `FileWatchGone` refusal
    // immediately (never a hang) and every commit pins the
    // never-fatal posture — while the watch proofs spawn the real
    // service on it.
    let (watch_state, commands) =
        crate::file_watch::FileWatchState::with_budget(crate::file_watch::MUTATION_REFRESH_BUDGET);
    app.manage(watch_state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            ScanProgressEvent,
            crate::file_watch::FileChangedEvent
        ])
        .mount_events(&app);
    (temp, app, commands)
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
    // The walk root is the CANONICAL form the SEC-003 check returns
    // (the scan module doc's contract — walked paths spell the
    // identity that way), so every spelling this test compares
    // against is the canonical one, never the raw tempdir form: the
    // raw spelling diverges from the resolved one on macOS
    // (`/var` → `/private/var`) and on Windows (the a19fe45 class).
    let walk_root =
        eunomia_fs::paths::canonicalize(&root).expect("the scan root fixture canonicalizes");
    seed_active_project(&app, &repo_known, "Known");
    seed_archived_project(&app, &repo_archived, "Arch");
    seed_exclusion(&app, &repo_excluded);
    let picked = pick_folder(&app, &walk_root);
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
                .starts_with(walk_root.to_string_lossy().as_ref()),
            "the walk reports paths under the canonical walk root; got {:?} \
             against the root {:?}",
            payload.current_path,
            walk_root
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
// `scan_commit` (T-077, §7.10 Projects module, PRJ-002/005/006, §8 S2,
// LOG-001…004, §5.6 SEC-003, §6.9, DB-005/DB-015/DB-030/DB-042, D-29,
// D-051, D-100, D-179)
// ---------------------------------------------------------------------------

/// Invokes `scan_commit` over the real IPC envelope with the given
/// selection, deserializing the success payload into the typed answer;
/// errors come back as the serialized `AppError` JSON.
fn invoke_scan_commit(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    paths: &[String],
) -> Result<ScanCommitResult, serde_json::Value> {
    match tauri::test::get_ipc_response(
        webview,
        invoke_request("scan_commit", serde_json::json!({ "paths": paths })),
    ) {
        Ok(payload) => Ok(payload
            .deserialize::<ScanCommitResult>()
            .expect("the success payload must deserialize into the commit DTO")),
        Err(error) => Err(error),
    }
}

/// Runs one completed scan over the picked `root` and answers what it
/// reported.
fn scan_completed(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    root: &str,
) -> Vec<ScannedRepository> {
    let answer = invoke_scan_start(webview, root).expect("the scan completes");
    let ScanRunOutcome::Completed { repositories, .. } = answer else {
        panic!("the scan must complete; got {answer:?}");
    };
    repositories
}

/// One completed scan's `new` repositories, their paths SORTED — the
/// result view's display order (the walk's delivery order is
/// unspecified beyond parent-before-child), which is also the order the
/// tests commit in.
fn scan_new_paths(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    root: &str,
) -> Vec<String> {
    let mut paths: Vec<String> = scan_completed(webview, root)
        .into_iter()
        .filter(|repository| repository.class == ScanClassWire::New)
        .map(|repository| repository.path)
        .collect();
    paths.sort();
    paths
}

/// The outcomes of a commit's entries, in answer order.
fn outcomes_of(result: &ScanCommitResult) -> Vec<&str> {
    result
        .entries
        .iter()
        .map(|entry| entry.outcome.as_str())
        .collect()
}

/// The repository paths of a commit's entries, in answer order.
fn paths_of(result: &ScanCommitResult) -> Vec<&str> {
    result
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect()
}

/// Every project row in `sort_order` order, through the repository the
/// command writes.
fn stored_projects(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> Vec<eunomia_store::projects::Project> {
    let database = database_of(app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::list(tx, eunomia_store::projects::ListScope::All)
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("reading the projects through the repository must work")
}

/// The journal's entries in insertion order (oldest first) — `recent`
/// answers newest first.
fn journal_entries(app: &tauri::App<tauri::test::MockRuntime>) -> Vec<LogRecord> {
    let database = database_of(app);
    let mut records = tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::action_log::recent(tx, 1000)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("reading the journal through the executor must work");
    records.reverse();
    records
}

/// Stages raw SQL — a test trigger — on the command's own connection,
/// the rollback proofs' shape (`ipc_tests.rs`'s settings test).
fn stage_sql(app: &tauri::App<tauri::test::MockRuntime>, sql: &'static str) {
    let database = database_of(app);
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute_batch(sql)
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("staging the SQL must work");
}

/// The commit suite's standard fixture: a picked scan root holding one
/// `.git`-directory repository per name, answering the picked root's
/// canonical path and each repository's canonical path (in `names`
/// order).
fn picked_root_with_repos(
    app: &tauri::App<tauri::test::MockRuntime>,
    temp: &Path,
    names: &[&str],
) -> (String, Vec<PathBuf>) {
    let root = temp.join("scan-root");
    std::fs::create_dir_all(&root).expect("the scan root fixture must be creatable");
    let repos = names.iter().map(|name| repo_under(&root, name)).collect();
    let picked = pick_folder(
        app,
        &eunomia_fs::paths::canonicalize(&root).expect("the scan root fixture canonicalizes"),
    );
    (picked, repos)
}

#[test]
fn a_repeated_scan_and_commit_create_no_duplicates() {
    // THE card criterion ("a repeated scan and commit create no
    // duplicates"), end to end through both commands. The first scan
    // reports three new repositories and the commit registers each
    // exactly once — with `added_via = scan`. The repeated scan over the
    // same folder classifies all three `known` (PRJ-002's own
    // acceptance sentence), and the commit stays duplicate-free from
    // BOTH sides it could break: a RE-SENT selection (the first result
    // view's, or a double-clicked "Add selected") converges into typed
    // outcomes naming the rows that exist — neither an error nor a
    // second row nor a second journal entry — and the second result
    // view's "Add selected" is the empty selection, the typed no-op.
    let (temp, app) = scan_app();
    let (picked, repos) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta", "gamma"]);
    let webview = main_webview(&app);

    let selection = scan_new_paths(&webview, &picked);
    let expected: Vec<String> = repos
        .iter()
        .map(|repo| repo.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        selection, expected,
        "the scan reports the three repositories as new"
    );

    let committed =
        invoke_scan_commit(&webview, &selection).expect("the first commit must succeed");
    assert_eq!(outcomes_of(&committed), ["added", "added", "added"]);
    assert_eq!(
        paths_of(&committed),
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "the answer follows the sent order"
    );
    assert_eq!(
        stored_counts(&app),
        (3, 0, 3),
        "three rows, three journal entries"
    );
    assert!(
        stored_projects(&app)
            .iter()
            .all(|project| project.added_via == AddedVia::Scan),
        "every row was registered by the scan (§6.2 `added_via`)"
    );

    let again = scan_completed(&webview, &picked);
    assert_eq!(
        again.len(),
        3,
        "the repeated scan still finds the three repositories"
    );
    assert!(
        again
            .iter()
            .all(|repository| repository.class == ScanClassWire::Known),
        "scanning the same folder again classifies every repository known; got {again:?}"
    );

    let resent =
        invoke_scan_commit(&webview, &selection).expect("a re-sent selection converges, not errs");
    assert_eq!(
        outcomes_of(&resent),
        ["already_added", "already_added", "already_added"]
    );
    let existing_ids: Vec<&str> = resent
        .entries
        .iter()
        .filter_map(|entry| entry.existing.as_ref().map(|existing| existing.id.as_str()))
        .collect();
    let added_ids: Vec<&str> = committed
        .entries
        .iter()
        .filter_map(|entry| entry.project.as_ref().map(|project| project.id.as_str()))
        .collect();
    assert_eq!(
        existing_ids, added_ids,
        "each typed outcome names the row the first commit created"
    );
    assert_eq!(
        stored_counts(&app),
        (3, 0, 3),
        "no second row and no second journal entry for the re-sent selection"
    );

    assert!(
        scan_new_paths(&webview, &picked).is_empty(),
        "the second result view has nothing left to add"
    );
    let empty = invoke_scan_commit(&webview, &[]).expect("an empty selection is the typed no-op");
    assert_eq!(
        empty,
        ScanCommitResult {
            entries: Vec::new()
        }
    );
    assert_eq!(
        stored_counts(&app),
        (3, 0, 3),
        "the empty selection wrote nothing"
    );
}

#[test]
fn an_error_on_one_row_rolls_back_the_rows_before_it_with_their_journal_entries() {
    // THE card criterion ("an error on one project rolls back
    // everything"), in the trigger shape of the settings rollback test:
    // the THIRD repository's INSERT is refused by a staged trigger —
    // after the first two rows and their journal entries were already
    // written inside the transaction — so an unchanged census after the
    // failure is the proof that they rolled back with it (the one
    // `Db::run` transaction, D-037). The control at the end shows the
    // trigger bites on `gamma` alone: the same first two repositories
    // commit cleanly without it.
    let (temp, app) = scan_app();
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta", "gamma"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 3, "three new repositories to commit");
    stage_sql(
        &app,
        "CREATE TRIGGER reject_gamma BEFORE INSERT ON projects
         WHEN NEW.name = 'gamma'
         BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
    );

    let error = invoke_scan_commit(&webview, &selection)
        .expect_err("the staged abort must fail the whole commit");
    assert_eq!(
        error["code"],
        serde_json::json!("E_VALIDATION"),
        "the constraint abort maps through the base classifier: {error}"
    );
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert_eq!(
        stored_counts(&app),
        (0, 0, 0),
        "alpha and beta — written before gamma inside the transaction — rolled back with \
         their journal entries"
    );

    let first_two = &selection[..2];
    let committed = invoke_scan_commit(&webview, first_two)
        .expect("the trigger refuses gamma alone: the first two commit");
    assert_eq!(outcomes_of(&committed), ["added", "added"]);
    assert_eq!(
        stored_counts(&app),
        (2, 0, 2),
        "the control: the rows before the failing one commit cleanly without it"
    );
}

#[test]
fn an_error_on_one_journal_entry_rolls_back_the_rows_before_it() {
    // The criterion's other half — the journal shares the operation's
    // transaction (D-051): the SECOND row's `project.added` insert is
    // refused once a second project row exists, so the failure lands
    // AFTER one row and its entry were written inside the transaction.
    // Both must be gone after the failure: a rolled-back commit leaves
    // neither the row nor an entry claiming it. The control (the first
    // repository alone) commits — one row never trips the trigger.
    let (temp, app) = scan_app();
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 2);
    stage_sql(
        &app,
        "CREATE TRIGGER reject_second_journal_entry BEFORE INSERT ON action_log
         WHEN (SELECT COUNT(*) FROM projects) >= 2
         BEGIN SELECT RAISE(ABORT, 'test: reject'); END;",
    );

    let error = invoke_scan_commit(&webview, &selection)
        .expect_err("the refused journal entry must fail the whole commit");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ConstraintViolation")
    );
    assert_eq!(
        stored_counts(&app),
        (0, 0, 0),
        "the first row rolled back together with the entry that was already written"
    );

    let committed = invoke_scan_commit(&webview, &selection[..1])
        .expect("a single row never trips the trigger");
    assert_eq!(outcomes_of(&committed), ["added"]);
    assert_eq!(stored_counts(&app), (1, 0, 1));
}

#[cfg(unix)]
#[test]
fn an_unstorable_derived_label_on_one_repository_refuses_the_whole_selection() {
    // A REAL §6.9 error on one project, with no staged trigger: D-079's
    // counterexample — a repository folder named with a leading control
    // character is a legal name whose DB-042 derivation ("\u{7}REPO") is
    // a label §6.9 rejects. Inside a selection the refusal is the
    // manual add's envelope (the `E_VALIDATION` naming the LABEL field
    // with the derived spelling) plus the repository's own `path` —
    // the batch's answer to "which row do I uncheck" — and the two
    // sound repositories in the same selection are not added either.
    // Unix-gated: the fixture needs a control character in a directory
    // name.
    let (temp, app) = scan_app();
    let (picked, repos) =
        picked_root_with_repos(&app, temp.path(), &["alpha", "\u{7}repo", "gamma"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(
        selection.len(),
        3,
        "the scan reports the control-named repository too"
    );

    let error = invoke_scan_commit(&webview, &selection)
        .expect_err("an unstorable derived label must reject the whole selection");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(
        error["params"]["field"],
        serde_json::json!("label"),
        "the failed field is the derived label, not the name"
    );
    assert_eq!(error["params"]["value"], serde_json::json!("\u{7}REPO"));
    assert_eq!(
        error["params"]["path"],
        serde_json::json!(repos[1].to_string_lossy()),
        "the rejection names the repository so the result view can mark its row"
    );
    assert_eq!(
        stored_counts(&app),
        (0, 0, 0),
        "nothing of the selection was written"
    );

    let refused = repos[1].to_string_lossy().into_owned();
    let sound: Vec<String> = selection
        .iter()
        .filter(|path| path.as_str() != refused)
        .cloned()
        .collect();
    assert_eq!(sound.len(), 2, "alpha and gamma are the sound repositories");
    let committed = invoke_scan_commit(&webview, &sound).expect("the sound repositories commit");
    assert_eq!(outcomes_of(&committed), ["added", "added"]);
    assert_eq!(stored_counts(&app), (2, 0, 2));
}

#[test]
fn the_commit_registers_each_repository_with_the_manual_add_rules() {
    // "The same rules as manual add", field by field, read back through
    // the store: the canonical path (DB-030), the folder name, the
    // DB-042 label (the first six characters, upper case — literal
    // expectations, not the derivation helper's output), a palette
    // color (D-29), `added_via = scan` (§6.2), the four write settings
    // copied from the CHANGED `default.*` keys (PRJ-012 — the copy
    // reads the settings at the commit), the appended `sort_order`
    // after the row that already existed (DB-005), no §6.9 warning on
    // a writable fixture, and ONE `project.added` journal entry per
    // created row — info/user/ok, entity and project naming the row,
    // in the sent order (LOG-001…004, D-051).
    let (temp, app) = scan_app();
    let outside = repo_under(temp.path(), "already-here");
    seed_active_project(&app, &outside, "Prior");
    use eunomia_store::settings::{
        BaseName, ExtraFile, Position, Scheme, SettingKey, SettingValue,
    };
    seed_setting(
        &app,
        SettingKey::DefaultScheme,
        SettingValue::Scheme(Scheme::B),
    );
    seed_setting(
        &app,
        SettingKey::DefaultBaseName,
        SettingValue::BaseName(BaseName::ClaudeMd),
    );
    seed_setting(
        &app,
        SettingKey::DefaultExtraLocal,
        SettingValue::ExtraFile(ExtraFile {
            enabled: true,
            position: Position::Start,
        }),
    );
    seed_setting(
        &app,
        SettingKey::DefaultExtraProject,
        SettingValue::ExtraFile(ExtraFile {
            enabled: true,
            position: Position::End,
        }),
    );
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta", "gamma"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 3);

    let committed = invoke_scan_commit(&webview, &selection).expect("the commit must succeed");
    assert_eq!(outcomes_of(&committed), ["added", "added", "added"]);
    let entries = committed.entries;
    let palette: Vec<&str> = eunomia_core::project::Color::ALL
        .iter()
        .map(|color| color.as_str())
        .collect();
    for (index, (entry, path)) in entries.iter().zip(&selection).enumerate() {
        assert_eq!(entry.path, *path, "the entry is about the sent repository");
        let project = entry
            .project
            .as_ref()
            .expect("the added entry carries the row");
        assert_eq!(
            project.path, *path,
            "the stored path is the canonical root (DB-030)"
        );
        assert_eq!(
            project.name,
            ["alpha", "beta", "gamma"][index],
            "S1: the folder name"
        );
        assert_eq!(
            project.label,
            ["ALPHA", "BETA", "GAMMA"][index],
            "DB-042: the first six characters, upper case"
        );
        assert!(
            palette.contains(&project.color.as_str()),
            "{}",
            project.color
        );
        assert_eq!(
            project.added_via, "scan",
            "§6.2: the scan commit's spelling"
        );
        assert_eq!(
            project.sort_order,
            index as u32 + 2,
            "appended after the existing row"
        );
        assert_eq!(project.scheme, "B");
        assert_eq!(project.base_name, "CLAUDE.md");
        assert!(project.extra_local_enabled);
        assert_eq!(project.extra_local_position, "start");
        assert!(project.extra_project_enabled);
        assert_eq!(project.extra_project_position, "end");
        assert_eq!(
            project.write_access_warning, None,
            "a writable fixture carries no warning"
        );
        assert!(
            entry.existing.is_none(),
            "an added entry names no existing row"
        );
    }

    // The stored rows agree with the wire (the wire is a view, not a
    // second truth), and the pre-existing row is untouched.
    let stored = stored_projects(&app);
    assert_eq!(stored.len(), 4);
    let fresh: Vec<&Project> = stored
        .iter()
        .filter(|project| project.path != outside.to_string_lossy())
        .collect();
    assert_eq!(fresh.len(), 3);
    assert!(fresh.iter().all(|project| {
        project.added_via == AddedVia::Scan
            && project.scheme == Scheme::B
            && project.base_name == BaseName::ClaudeMd
            && project.extra_local.enabled
            && project.extra_local.position == Position::Start
            && project.extra_project.enabled
            && project.extra_project.position == Position::End
            && project.description.is_empty()
    }));

    // The journal: one entry per created row, oldest first == the sent
    // order, each naming its row.
    let journal = journal_entries(&app);
    assert_eq!(
        journal.len(),
        3,
        "one add, one entry — three adds, three entries"
    );
    for (record, entry) in journal.iter().zip(&entries) {
        let id = entry.project.as_ref().expect("added").id.as_str();
        assert_eq!(record.action.as_str(), "project.added");
        assert_eq!(record.level.as_str(), "info");
        assert_eq!(record.visibility.as_str(), "user");
        assert_eq!(record.result.as_str(), "ok");
        assert_eq!(
            record.entity,
            Some((
                eunomia_store::action_log::EntityType::Project,
                id.to_owned()
            ))
        );
        assert_eq!(record.project_id.as_deref(), Some(id));
        assert_eq!(
            record.details, None,
            "these flows write no file: no details"
        );
    }
}

#[test]
fn twelve_rows_in_one_commit_take_twelve_distinct_colors() {
    // PRJ-010's acceptance criterion ("For 12 projects in a row, colors
    // do not repeat") inside ONE commit: D-29 is evaluated per created
    // row against the active set as it stands INSIDE the transaction —
    // the batch's own earlier rows count — so a batch from an empty
    // library takes the whole palette. The thirteenth row then repeats
    // a color, the legal repeat DB-041 names. WHAT each row takes is
    // D-29's hash (D-077) and is deliberately not pinned.
    let (temp, app) = scan_app();
    let names: Vec<String> = (0..13).map(|n| format!("repo-{n:02}")).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &name_refs);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 13);

    let committed = invoke_scan_commit(&webview, &selection).expect("the commit must succeed");
    assert_eq!(outcomes_of(&committed).len(), 13);
    assert!(
        outcomes_of(&committed)
            .iter()
            .all(|outcome| *outcome == "added")
    );
    let colors: Vec<&str> = committed
        .entries
        .iter()
        .filter_map(|entry| entry.project.as_ref().map(|project| project.color.as_str()))
        .collect();
    assert_eq!(colors.len(), 13);
    let first_twelve: std::collections::HashSet<&str> = colors[..12].iter().copied().collect();
    assert_eq!(
        first_twelve.len(),
        12,
        "twelve rows in one commit take twelve distinct colors; got {colors:?}"
    );
    assert!(
        colors[..12].contains(&colors[12]),
        "the thirteenth row repeats a color once the palette is full; got {colors:?}"
    );
}

#[test]
fn registered_and_excluded_paths_answer_typed_outcomes_and_the_rest_are_added() {
    // The typed non-add outcomes (D-179): a path an ACTIVE project
    // already holds answers `already_added` naming that row; one an
    // ARCHIVED project holds answers `archived` with the same row —
    // the id DB-015's restore offer calls back with; one that sits in
    // `project_exclusions` with no project answers `excluded` (the scan
    // showed it under "Skipped"; §6.4's removal is the way back). None
    // of the three is an error and none writes a row or a journal
    // entry: the fresh repository in the same selection is still added.
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    std::fs::create_dir_all(&root).expect("the scan root fixture must be creatable");
    let archived = repo_under(&root, "repo-archived");
    let excluded = repo_under(&root, "repo-excluded");
    let fresh = repo_under(&root, "repo-fresh");
    let known = repo_under(&root, "repo-known");
    seed_active_project(&app, &known, "Known");
    seed_archived_project(&app, &archived, "Arch");
    seed_exclusion(&app, &excluded);
    let picked = pick_folder(
        &app,
        &eunomia_fs::paths::canonicalize(&root).expect("the scan root fixture canonicalizes"),
    );
    let webview = main_webview(&app);
    // The scan reports all four repositories with their ALG-053
    // classes — all four join the discovered set, since what the
    // commit does with each is its own typed answer.
    let reported = scan_completed(&webview, &picked);
    assert_eq!(reported.len(), 4);
    let class_of = |path: &Path| {
        reported
            .iter()
            .find(|repository| repository.path == path.to_string_lossy().as_ref())
            .map(|repository| repository.class)
    };
    assert_eq!(class_of(&known), Some(ScanClassWire::Known));
    assert_eq!(class_of(&archived), Some(ScanClassWire::Archived));
    assert_eq!(class_of(&excluded), Some(ScanClassWire::Excluded));
    assert_eq!(class_of(&fresh), Some(ScanClassWire::New));

    let selection: Vec<String> = [&archived, &excluded, &fresh, &known]
        .iter()
        .map(|repo| repo.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        selection,
        {
            let mut sorted = selection.clone();
            sorted.sort();
            sorted
        },
        "the selection is the result view's sorted display order"
    );

    let committed = invoke_scan_commit(&webview, &selection).expect("the commit must succeed");
    assert_eq!(
        outcomes_of(&committed),
        ["archived", "excluded", "added", "already_added"]
    );
    assert_eq!(
        paths_of(&committed),
        selection.iter().map(String::as_str).collect::<Vec<_>>()
    );

    let stored = stored_projects(&app);
    let id_of = |path: &Path| {
        stored
            .iter()
            .find(|project| project.path == path.to_string_lossy())
            .expect("the seeded row")
            .id
            .as_str()
    };
    let archived_entry = &committed.entries[0];
    assert_eq!(
        archived_entry.existing.as_ref().map(|existing| {
            (
                existing.id.as_str(),
                existing.state.as_str(),
                existing.path.as_str(),
            )
        }),
        Some((
            id_of(&archived),
            "archived",
            archived.to_string_lossy().as_ref()
        )),
        "the archived entry carries the restore offer's row"
    );
    assert!(archived_entry.project.is_none());
    let excluded_entry = &committed.entries[1];
    assert!(excluded_entry.project.is_none() && excluded_entry.existing.is_none());
    assert_eq!(excluded_entry.path, excluded.to_string_lossy());
    let fresh_entry = &committed.entries[2];
    assert_eq!(
        fresh_entry
            .project
            .as_ref()
            .map(|project| project.added_via.as_str()),
        Some("scan")
    );
    let known_entry = &committed.entries[3];
    assert_eq!(
        known_entry
            .existing
            .as_ref()
            .map(|existing| (existing.id.as_str(), existing.state.as_str())),
        Some((id_of(&known), "active"))
    );

    assert_eq!(
        stored_counts(&app),
        (3, 1, 1),
        "one row and one journal entry — the typed outcomes wrote nothing"
    );
    let journal = journal_entries(&app);
    assert_eq!(journal.len(), 1);
    assert_eq!(
        journal[0].project_id.as_deref(),
        fresh_entry
            .project
            .as_ref()
            .map(|project| project.id.as_str()),
        "the one entry is the fresh row's"
    );
}

#[test]
fn one_repository_sent_twice_is_one_entry_and_one_row() {
    // In-request duplicates: the same repository sent twice — once
    // under its trailing-separator spelling, which keys identically
    // (DB-030 strips trailing separators) — is ONE candidate, one
    // entry, one row, and keeps its FIRST position in the sent order
    // (the `sort_order` append order). The other alias the UI could
    // send, a dot segment, does NOT key like the reported path and is
    // the SEC-003 rejection: the commit matches the scan answer's
    // exact text by key, it does not resolve arbitrary spellings.
    let (temp, app) = scan_app();
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    let (alpha, beta) = (selection[0].clone(), selection[1].clone());

    let committed = invoke_scan_commit(
        &webview,
        &[
            alpha.clone(),
            format!("{alpha}/"),
            alpha.clone(),
            beta.clone(),
            beta.clone(),
        ],
    )
    .expect("the commit must succeed");
    assert_eq!(outcomes_of(&committed), ["added", "added"]);
    assert_eq!(paths_of(&committed), [alpha.as_str(), beta.as_str()]);
    let orders: Vec<u32> = committed
        .entries
        .iter()
        .filter_map(|entry| entry.project.as_ref().map(|project| project.sort_order))
        .collect();
    assert_eq!(orders, [1, 2], "first occurrence wins the append order");
    assert_eq!(stored_counts(&app), (2, 0, 2));

    let dotted = format!("{picked}/./alpha");
    let error = invoke_scan_commit(&webview, std::slice::from_ref(&dotted))
        .expect_err("a dot segment is not a reported path");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["params"]["field"], serde_json::json!("path"));
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(dotted),
        "the caller's own input is echoed"
    );
}

#[cfg(unix)]
#[test]
fn two_reported_names_that_resolve_to_one_folder_are_one_entry_and_one_row() {
    // The second collapse — by the RESOLVED identity, after the one by
    // the sent text's key: both names were reported, so both pass the
    // text gate, but since the scan `beta` was replaced by a symlink
    // onto `alpha`. They lead to ONE folder, and the commit registers
    // that folder once: one entry, one row — not an `added` followed
    // by an `already_added` for the same repository, which is what a
    // commit without the resolved-identity collapse answers (the
    // in-transaction duplicate check would still keep the store
    // clean, so only the answer's shape can tell the two apart).
    let (temp, app) = scan_app();
    let (picked, repos) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 2, "the scan reported both repositories");

    std::fs::remove_dir_all(&repos[1]).expect("removing the fixture repository must work");
    std::os::unix::fs::symlink(&repos[0], &repos[1])
        .expect("creating the symlink onto alpha must work");

    let committed = invoke_scan_commit(&webview, &selection).expect("both names are admitted");
    assert_eq!(outcomes_of(&committed), ["added"], "one folder, one entry");
    assert_eq!(paths_of(&committed), [repos[0].to_string_lossy().as_ref()]);
    assert_eq!(stored_counts(&app), (1, 0, 1), "one row, one journal entry");
}

#[test]
fn a_path_no_completed_scan_reported_is_rejected_before_anything_is_added() {
    // SEC-003's discovered-set rule (D-179): a REAL repository under
    // the picked root — on disk, canonical, a `.git` directory, created
    // AFTER the scan — is everything a containment rule would accept
    // and still refused, because no completed scan reported it. The
    // refusal is the typed `E_VALIDATION` of the path field naming the
    // policy, and the sound repository in the same selection is not
    // added either. A blank entry is the same field's rejection.
    let (temp, app) = scan_app();
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &["alpha"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 1);

    let late = repo_under(Path::new(&picked), "late-repo");
    let late_text = late.to_string_lossy().into_owned();
    let error = invoke_scan_commit(&webview, &[selection[0].clone(), late_text.clone()])
        .expect_err("a path no scan reported must be rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["params"]["field"], serde_json::json!("path"));
    assert_eq!(error["params"]["value"], serde_json::json!(late_text));
    assert!(
        error["params"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("SEC-003")),
        "the rejection names the policy, got: {error}"
    );
    assert_eq!(
        error["details"],
        serde_json::json!(null),
        "a policy answer carries no OS facts"
    );
    assert_eq!(
        stored_counts(&app),
        (0, 0, 0),
        "the selection was refused as a whole"
    );

    // The text gate runs BEFORE any disk probe: a path that exists
    // nowhere on disk is still the SEC-003 rejection, never the
    // availability error a canonicalization attempt would answer — the
    // webview cannot make the commit touch the filesystem for a path no
    // scan reported.
    let nowhere = "/nowhere/never-existed/repo".to_owned();
    let error = invoke_scan_commit(&webview, std::slice::from_ref(&nowhere))
        .expect_err("a path that exists nowhere is rejected as unreported, not unavailable");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["params"]["value"], serde_json::json!(nowhere));
    assert_eq!(
        error["details"],
        serde_json::json!(null),
        "no OS fact: nothing was probed"
    );

    let error =
        invoke_scan_commit(&webview, &["   ".to_owned()]).expect_err("a blank entry is rejected");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["params"]["field"], serde_json::json!("path"));
}

#[test]
fn a_cancelled_scan_publishes_nothing_and_keeps_the_previous_findings() {
    // The publication half of the rule (ERR-002): only a COMPLETED scan
    // publishes. Scan A completes; scan B is then cancelled before its
    // first entry (the pre-latched token) — B's repository is NOT
    // admitted afterwards, and A's still is: a cancelled scan saves
    // nothing AND clears nothing.
    let (temp, app) = scan_app();
    let root_a = temp.path().join("root-a");
    std::fs::create_dir_all(&root_a).expect("the first root fixture must be creatable");
    let alpha = repo_under(&root_a, "alpha");
    let root_b = temp.path().join("root-b");
    std::fs::create_dir_all(&root_b).expect("the second root fixture must be creatable");
    let beta = repo_under(&root_b, "beta");
    let picked_a = pick_folder(
        &app,
        &eunomia_fs::paths::canonicalize(&root_a).expect("the first root canonicalizes"),
    );
    let picked_b = pick_folder(
        &app,
        &eunomia_fs::paths::canonicalize(&root_b).expect("the second root canonicalizes"),
    );
    let webview = main_webview(&app);
    assert_eq!(
        scan_new_paths(&webview, &picked_a).len(),
        1,
        "scan A completes"
    );

    let token = Arc::new(CancelToken::new());
    token.cancel();
    let answer = tauri::async_runtime::block_on(crate::scan::scan_start_with(
        app.handle().clone(),
        picked_b,
        token,
    ))
    .expect("a cancelled scan answers Ok, not Err");
    assert!(matches!(answer, ScanRunOutcome::Cancelled));

    let beta_text = beta.to_string_lossy().into_owned();
    let error = invoke_scan_commit(&webview, std::slice::from_ref(&beta_text))
        .expect_err("a cancelled scan's repository is not admitted");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["params"]["value"], serde_json::json!(beta_text));

    let committed = invoke_scan_commit(&webview, &[alpha.to_string_lossy().into_owned()])
        .expect("the completed scan's repository is still admitted");
    assert_eq!(outcomes_of(&committed), ["added"]);
}

#[test]
fn a_repository_that_vanished_after_the_scan_refuses_the_whole_selection() {
    // The folder is gone by the commit (D-09's situation): the answer is
    // §10's `E_PROJECT_UNAVAILABLE` with the path in params and the OS
    // refusal in details — "could not check" is not "not a repo" (the
    // D-090 split) — and the still-present repository in the same
    // selection is not added: a result view holding a vanished row is
    // stale as a whole, and "Scan again" is the user's action.
    let (temp, app) = scan_app();
    let (picked, repos) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 2);

    std::fs::remove_dir_all(&repos[1]).expect("removing the fixture repository must work");
    let error = invoke_scan_commit(&webview, &selection)
        .expect_err("a vanished folder must refuse the commit");
    assert_eq!(
        error["code"],
        serde_json::json!("E_PROJECT_UNAVAILABLE"),
        "{error}"
    );
    assert_eq!(
        error["params"]["path"],
        serde_json::json!(repos[1].to_string_lossy())
    );
    assert!(
        error["details"]["os_code"].as_i64().is_some(),
        "the OS refusal travels in details (§10's first-class log fact): {error}"
    );
    assert_eq!(
        stored_counts(&app),
        (0, 0, 0),
        "alpha — still present — was not added"
    );
}

#[test]
fn a_folder_whose_own_git_is_gone_is_refused_even_inside_a_repository() {
    // The repository-root gate: the scan root is itself a repository,
    // so `find_git_root` still answers for `beta` — with the ANCESTOR's
    // root — after beta's own `.git` was removed since the scan. The
    // commit requires the folder ITSELF to be the root and refuses it
    // with §10's `E_NOT_A_REPO` (the manual add's envelope: the path in
    // params, no details — a fact about the tree): it never registers a
    // subfolder under a silently chosen ancestor (PRJ-003's offer is
    // the manual add's flow, not the scan commit's).
    let (temp, app) = scan_app();
    let root = temp.path().join("scan-root");
    std::fs::create_dir_all(root.join(".git"))
        .expect("the root repository fixture must be creatable");
    let beta = repo_under(&root, "beta");
    let picked = pick_folder(
        &app,
        &eunomia_fs::paths::canonicalize(&root).expect("the scan root fixture canonicalizes"),
    );
    let webview = main_webview(&app);
    let beta_text = beta.to_string_lossy().into_owned();
    assert!(
        scan_new_paths(&webview, &picked).contains(&beta_text),
        "the scan reported beta as a new repository"
    );

    std::fs::remove_dir_all(beta.join(".git")).expect("removing the fixture's .git must work");
    let error = invoke_scan_commit(&webview, std::slice::from_ref(&beta_text))
        .expect_err("a folder that is no longer a root must be refused");
    assert_eq!(error["code"], serde_json::json!("E_NOT_A_REPO"), "{error}");
    assert_eq!(error["message_key"], serde_json::json!("errors.not_a_repo"));
    assert_eq!(error["params"], serde_json::json!({ "path": beta_text }));
    assert_eq!(error["details"], serde_json::json!(null));
    assert_eq!(stored_counts(&app), (0, 0, 0));
}

#[cfg(unix)]
#[test]
fn a_folder_swapped_in_under_a_reported_name_is_refused_by_the_discovered_set() {
    // The canonical-form half of the rule: between the scan and the
    // commit, the reported folder is replaced by a symlink onto a
    // repository OUTSIDE the picked root. The sent text is still a
    // reported path, but what it resolves to is not — the commit
    // refuses it with the SEC-003 rejection naming the folder the name
    // leads to, and nothing outside the scan's findings is registered.
    let (temp, app) = scan_app();
    let (picked, repos) = picked_root_with_repos(&app, temp.path(), &["alpha"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 1);

    let victim = repo_under(temp.path(), "outside-victim");
    let alpha = &repos[0];
    let moved = temp.path().join("alpha-moved-away");
    std::fs::rename(alpha, &moved).expect("moving the fixture repository must work");
    std::os::unix::fs::symlink(&victim, alpha).expect("creating the swapped symlink must work");

    let error =
        invoke_scan_commit(&webview, &selection).expect_err("a swapped-in folder must be refused");
    assert_eq!(error["code"], serde_json::json!("E_VALIDATION"), "{error}");
    assert_eq!(error["params"]["field"], serde_json::json!("path"));
    assert_eq!(
        error["params"]["value"],
        serde_json::json!(victim.to_string_lossy()),
        "the rejection names the folder the reported name now leads to"
    );
    assert_eq!(
        stored_counts(&app),
        (0, 0, 0),
        "the repository outside the scan was not registered"
    );
}

#[test]
fn scan_commit_is_refused_during_a_sync_and_recovers_after_it() {
    // SYS-005 end to end over the real IPC path (the
    // `project_add_manual` gate test's shape): the commit creates
    // project rows, so while the flag is up it answers the §10
    // `E_SYNC_RUNNING` envelope — asserted against the spec-literal
    // JSON, not the constructor's output — writes nothing, and answers
    // the mode refusal for a path from nowhere too (the gate precedes
    // the input gates); the moment the run ends the same selection
    // lands.
    let (temp, app) = scan_app();
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &["alpha"]);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);

    let guard =
        crate::commands::begin_sync(app.handle()).expect("the test's flag must be free to take");
    let error = invoke_scan_commit(&webview, &selection)
        .expect_err("the commit must be refused during a sync");
    assert_eq!(
        error,
        serde_json::json!({
            "code": "E_SYNC_RUNNING",
            "message_key": "errors.sync_running",
            "params": {},
            "details": null,
        }),
        "the refusal's envelope is the §10/§7.10 wire form"
    );
    let foreign = invoke_scan_commit(&webview, &["/nowhere/never-scanned".to_owned()])
        .expect_err("a path from nowhere must meet the mode answer first");
    assert_eq!(
        foreign, error,
        "the mode gate precedes SEC-003 — the refusal is input-independent"
    );
    assert_eq!(
        stored_counts(&app),
        (0, 0, 0),
        "no project row was written while the flag was up"
    );

    drop(guard);
    let committed =
        invoke_scan_commit(&webview, &selection).expect("the commit works once the sync has ended");
    assert_eq!(
        outcomes_of(&committed),
        ["added"],
        "the retried commit after the run ended succeeded"
    );
}

#[test]
fn scan_commit_answers_honestly_without_a_database() {
    // Recovery mode: no `Db` managed. The honest answer is the recorded
    // `NoDatabase` fallback — not a pretend empty answer — and it comes
    // from the database lookup, which precedes every gate: the flag and
    // findings states ARE managed here so neither of their own
    // missing-state fallbacks can be the source.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![crate::scan::scan_commit])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(crate::session_paths::SessionPaths::new());
    app.manage(ScanFindings::new());
    app.manage(crate::sync_running::SyncRunning::new());

    let error = invoke_scan_commit(&main_webview(&app), &["/repos/anything".to_owned()])
        .expect_err("the commit needs the database");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("NoDatabase"),
        "the fallback diagnostic names the missing state"
    );
}

#[test]
fn a_completed_scan_and_a_commit_refuse_honestly_without_the_findings_state() {
    // The uninitialized-app bug the two commands answer honestly (the
    // `the_missing_session_state_answers_the_honest_fallback` shape):
    // `setup` manages the discovered set before anything that could
    // fail, so its absence cannot occur in a real launch — and neither
    // command may pretend: a walk that cannot publish its findings is
    // refused (never a completed answer whose commit could only say
    // "not reported"), and the commit refuses before admitting
    // anything. A cancelled scan needs no publication and is unaffected.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    crate::data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database = crate::db::Db::open(temp.path())
        .expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            crate::scan::scan_commit,
            crate::scan::scan_start,
            crate::scan::scan_cancel
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    app.manage(crate::session_paths::SessionPaths::new());
    app.manage(ScanCancelSlot::new());
    // Deliberately NOT managing `ScanFindings` — the state under test.
    app.manage(crate::sync_running::SyncRunning::new());
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![ScanProgressEvent])
        .mount_events(&app);
    let (root, repos) = {
        let root = temp.path().join("scan-root");
        std::fs::create_dir_all(&root).expect("the scan root fixture must be creatable");
        let repo = repo_under(&root, "alpha");
        (root, vec![repo])
    };
    let picked = pick_folder(
        &app,
        &eunomia_fs::paths::canonicalize(&root).expect("the scan root fixture canonicalizes"),
    );
    let webview = main_webview(&app);

    let error = invoke_scan_start(&webview, &picked)
        .expect_err("a completed scan must refuse honestly without its findings state");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ScanFindingsMissing")
    );
    assert!(
        slot_token(&app).is_none(),
        "the lease cleared the publication on the refusal too"
    );
    // A cancelled scan publishes nothing and needs no findings state.
    let token = Arc::new(CancelToken::new());
    token.cancel();
    let answer = tauri::async_runtime::block_on(crate::scan::scan_start_with(
        app.handle().clone(),
        picked.clone(),
        token,
    ))
    .expect("a cancelled scan answers Ok, not Err");
    assert!(matches!(answer, ScanRunOutcome::Cancelled));

    let error = invoke_scan_commit(&webview, &[repos[0].to_string_lossy().into_owned()])
        .expect_err("the commit must refuse honestly without its findings state");
    assert_eq!(error["code"], serde_json::json!("E_DB_LOCKED"), "{error}");
    assert_eq!(
        error["details"]["os_kind"],
        serde_json::json!("ScanFindingsMissing")
    );
    assert_eq!(stored_counts(&app), (0, 0, 0));
}

#[test]
fn the_write_access_warning_rides_its_own_added_entry() {
    // §6.9's project-path row ("Write access is checked when adding
    // (warning)") per row of the batch: an existing read-only target
    // file (T-F5's fact, portable) warns on ITS entry only. The
    // positions matter: the unwritable repositories sit SECOND and
    // LAST, each behind a writable one, so a warning that slid onto
    // the next row would show — the round-1 review found exactly that
    // slide (a compacted warning list handed out positionally), which
    // a first-row-only shape cannot distinguish from the right answer.
    // All four rows are registered: a warning never un-adds, and the
    // sync preflight re-checks before any write.
    let (temp, app) = scan_app();
    let (picked, repos) =
        picked_root_with_repos(&app, temp.path(), &["alpha", "beta", "gamma", "delta"]);
    // The selection is the SORTED display order — alpha, beta, delta,
    // gamma — so beta is the second row and gamma the last.
    for unwritable in [&repos[1], &repos[2]] {
        std::fs::write(unwritable.join("AGENTS.md"), b"hand-written")
            .expect("the fixture file must be creatable");
        let mut permissions = std::fs::metadata(unwritable.join("AGENTS.md"))
            .expect("the fixture file must be statable")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(unwritable.join("AGENTS.md"), permissions)
            .expect("marking the fixture read-only must work");
    }
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 4);

    let committed = invoke_scan_commit(&webview, &selection)
        .expect("a read-only target is a warning, not a rejection");
    assert_eq!(outcomes_of(&committed), ["added"; 4]);
    let warnings: Vec<Option<&str>> = committed
        .entries
        .iter()
        .map(|entry| {
            entry
                .project
                .as_ref()
                .and_then(|project| project.write_access_warning.as_deref())
        })
        .collect();
    assert_eq!(
        warnings,
        [None, Some("file_read_only"), None, Some("file_read_only")],
        "each warning names the row whose target is read-only; the writable rows before and          after stay silent"
    );
    assert_eq!(
        stored_counts(&app),
        (4, 0, 4),
        "all four rows are registered"
    );
}

#[test]
fn a_large_selection_commits_in_one_transaction() {
    // The batch path at scale — Rule 6's large input, no timing
    // assertion: several hundred repositories go through one commit
    // and come back as one row, one entry and one journal line each,
    // with the DB-005 append order following the sent order, distinct
    // identities, and D-29's least-used rule keeping the palette
    // balanced (a multiple of twelve rows over twelve colors is
    // exactly even — any re-count bug in the batch's color state would
    // skew it). The selection is sent with N >> R — every path five
    // times, cycled (3000 sent paths for 600 reported repositories):
    // the in-request duplicates cost nothing observable, one row,
    // entry and journal line per repository, the first occurrences'
    // order kept (the cycle's first lap is the sorted order itself).
    const ROWS: usize = 600;
    let (temp, app) = scan_app();
    let names: Vec<String> = (0..ROWS).map(|n| format!("repo-{n:04}")).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (picked, _) = picked_root_with_repos(&app, temp.path(), &name_refs);
    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(
        selection.len(),
        ROWS,
        "the scan reports every fixture repository"
    );

    let sent: Vec<String> = selection.iter().cycle().take(ROWS * 5).cloned().collect();
    assert_eq!(
        sent.len(),
        ROWS * 5,
        "N >> R: five spellings per repository"
    );
    let committed = invoke_scan_commit(&webview, &sent).expect("the large commit must succeed");
    assert_eq!(outcomes_of(&committed), vec!["added"; ROWS]);
    assert_eq!(
        paths_of(&committed),
        selection.iter().map(String::as_str).collect::<Vec<_>>(),
        "one entry per repository, in first-occurrence order"
    );

    let stored = stored_projects(&app);
    assert_eq!(stored.len(), ROWS);
    for (position, (row, path)) in stored.iter().zip(&selection).enumerate() {
        assert_eq!(
            row.sort_order,
            position as i64 + 1,
            "DB-005: the append follows the sent order"
        );
        assert_eq!(row.path, *path);
        assert_eq!(row.added_via, AddedVia::Scan);
    }
    let ids: std::collections::HashSet<&str> = stored.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids.len(), ROWS, "every row has its own identity");
    let mut per_color: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for row in &stored {
        *per_color.entry(row.color.as_str()).or_default() += 1;
    }
    assert_eq!(
        per_color.len(),
        12,
        "the whole palette is in use: {per_color:?}"
    );
    assert!(
        per_color.values().all(|count| *count == ROWS / 12),
        "the least-used rule spreads the batch evenly: {per_color:?}"
    );
    assert_eq!(
        journal_entries(&app).len(),
        ROWS,
        "one journal entry per row"
    );
}

// ---------------------------------------------------------------------------
// `scan_commit` and the watch service (§5.7 SYS-007, D-144, D-156) — the
// `ipc_tests.rs` `add_manual_watch_app` proofs, mirrored for the batch
// ---------------------------------------------------------------------------

#[test]
fn a_commit_joins_its_created_rows_to_the_watched_set_through_one_pass() {
    // §5.7 SYS-007 ("Watches are added when a project is added") for
    // the batch: the created rows join the watched set through ONE
    // awaited full pass on the bounded mutation seam — the same
    // refresh path (command, ack, service pass) the manual add and
    // the sync aftermath run — never one pass per row. The proof
    // needs a verdict only a pass that COVERED the new rows can
    // record: a PRE-EXISTING differing `AGENTS.md` in each committed
    // repository (SYS-008's literal reading). Without the wiring the
    // cache stays empty (absent ≡ clean).
    let (temp, app, commands) = scan_app_with_watch_service();
    let (picked, repos) = picked_root_with_repos(&app, temp.path(), &["alpha", "beta"]);
    for repo in &repos {
        std::fs::write(
            repo.join("AGENTS.md"),
            "A pre-existing file an editor left.\n",
        )
        .expect("the pre-existing target must be writable");
    }

    crate::file_watch::spawn_service(
        app.handle().clone(),
        commands,
        crate::ipc_tests::WATCH_DEBOUNCE,
    )
    .expect("the watcher must start");
    let _events = crate::ipc_tests::listen_file_changed(&app);
    crate::ipc_tests::wait_for_watch_passes(&app, 1); // the startup pass over an empty table

    let webview = main_webview(&app);
    let selection = scan_new_paths(&webview, &picked);
    assert_eq!(selection.len(), 2);
    let committed = invoke_scan_commit(&webview, &selection)
        .expect("the commit must succeed — the refresh is never-fatal aftermath");
    assert_eq!(outcomes_of(&committed), ["added", "added"]);
    let mut created_ids: Vec<String> = committed
        .entries
        .iter()
        .filter_map(|entry| entry.project.as_ref().map(|project| project.id.clone()))
        .collect();
    created_ids.sort();

    // The wiring's proof: the service's cache holds a verdict for
    // exactly the two created rows — recordable only by a full pass
    // that covered them after their commit.
    let mut snapshot = app.state::<crate::file_watch::FileWatchState>().snapshot();
    snapshot.sort_by(|left, right| left.project_id.cmp(&right.project_id));
    assert_eq!(
        snapshot,
        created_ids
            .iter()
            .map(|project_id| crate::file_watch::ProjectFileChange {
                project_id: project_id.clone(),
                external_changed: true,
                file_deleted: false,
            })
            .collect::<Vec<_>>(),
        "each created row's pre-existing differing file is reported"
    );
    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        2,
        "exactly one full pass served the whole batch, after the startup one"
    );
    // Display-only even through the mutation's refresh: the commit
    // writes no target file (OUT-005 — the sync's job).
    for repo in &repos {
        assert_eq!(
            std::fs::read_to_string(repo.join("AGENTS.md")).expect("the file must be readable"),
            "A pre-existing file an editor left.\n",
            "the commit (and its watch pass) never writes the target file"
        );
    }
}

#[test]
fn a_commit_that_creates_no_row_runs_no_watch_pass() {
    // The wiring's negative half (the `a_refused_add_runs_no_watch_pass`
    // twin): the typed outcomes and the empty selection write nothing,
    // so they must run no watch pass — the refresh belongs to the
    // committed rows alone. The pass counter is the deterministic
    // observable (an empty diff is silent on the wire by contract).
    let (temp, app, commands) = scan_app_with_watch_service();
    let (picked, repos) = picked_root_with_repos(&app, temp.path(), &["alpha"]);
    seed_active_project(&app, &repos[0], "Alpha");

    crate::file_watch::spawn_service(
        app.handle().clone(),
        commands,
        crate::ipc_tests::WATCH_DEBOUNCE,
    )
    .expect("the watcher must start");
    let _events = crate::ipc_tests::listen_file_changed(&app);
    crate::ipc_tests::wait_for_watch_passes(&app, 1);
    let after_startup = app
        .state::<crate::file_watch::FileWatchState>()
        .pass_count();
    assert_eq!(after_startup, 1);

    let webview = main_webview(&app);
    assert!(
        scan_new_paths(&webview, &picked).is_empty(),
        "the seeded row is not new"
    );
    let typed = invoke_scan_commit(&webview, &[repos[0].to_string_lossy().into_owned()])
        .expect("the typed outcome is not an error");
    assert_eq!(outcomes_of(&typed), ["already_added"]);
    let empty = invoke_scan_commit(&webview, &[]).expect("the empty selection is the no-op");
    assert!(empty.entries.is_empty());
    assert_eq!(
        app.state::<crate::file_watch::FileWatchState>()
            .pass_count(),
        after_startup,
        "no created row, no watch pass"
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
        "scan::scan_commit",
        "scan::scan_start",
        "scan::ScanProgressEvent",
        "scan::ScanCancelSlot::new()",
        "scan::ScanFindings::new()",
    ] {
        assert_eq!(
            lib.matches(marker).count(),
            1,
            "lib.rs must wire {marker} exactly once (§7.10, T-076/T-077)"
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
