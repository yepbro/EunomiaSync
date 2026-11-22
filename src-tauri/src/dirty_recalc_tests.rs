//! Behavior tests of the dirty-flag recalculation service (T-101,
//! DB-021/DB-022, §7.10's `state://dirty`).
//!
//! The suite drives the service the ways production reaches it:
//!
//! * the **direct entry** `dirty_recalc::recalc_and_emit` — the pass
//!   the future sync pipeline awaits as §9.4 phase 4 step 5's "send
//!   `state://dirty`";
//! * the **service task** over the trigger bus — the startup spawn
//!   and the debounced commit trigger, observed through the pass
//!   counter (an empty diff is silent by contract, so the event alone
//!   cannot prove a pass ran) and the typed event receiver (Rust
//!   listeners deliver synchronously inside `emit`, so post-call
//!   `try_recv` assertions are deterministic — the availability
//!   suite's rule);
//! * the **executor seam** `Db::set_commit_hook` on a real database —
//!   the row-change discrimination: a write commits and fires, a
//!   read-only transaction and a zero-row write stay silent.
//! * the **read side** — `commands::projects_list`'s `dirty` served
//!   from the same cache (T-254, closing D-121's routed follow-up):
//!   the command is driven over the mock app against a cache seeded
//!   by real passes, pinning the clean baseline before the first
//!   pass, the archived half over a stale entry, call stability, and
//!   the missing-state fallback.
//!
//! The mock app is the `availability_app` shape (mock runtime, real
//! database in a temp root, events mounted through a specta builder);
//! the harness here owns its copy because the ipc suite's helpers are
//! private to it. One deliberate deviation from the production wiring,
//! documented at the harness: the commit hook is installed per test on
//! a local `Db` clone (managed state answers reads, but only a handle
//! the test runs writes through needs the hook), while the production
//! install-before-`manage` order is pinned by the `lib.rs` source
//! scan.
//!
//! The card's three acceptance criteria run end to end here: an edit
//! affecting the build (the assigned variant's text) makes the project
//! dirty; an edit to a hidden instruction without assignments does
//! not; and the set-L recalculation answers the full dirty list
//! within §12.2's budget (measured and printed in the T-136 style,
//! with a generous regression tripwire instead of the reference
//! machine's 500 ms — CI runners vary).
#![allow(clippy::print_stderr)] // test skip-notices and benchmark timings print deliberately

use std::path::Path;
use std::time::{Duration, Instant};

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_core::hash::build_hash;
use eunomia_core::project::{ExtraFile, ExtraFilePosition, Scheme};
use eunomia_store::projects::{ListScope, NewProject};
use eunomia_store::storage::{open_db, run_migrations};

use crate::data_dirs;
use crate::db::Db;
use crate::dirty_recalc::{
    self, DirtyEvent, DirtyState, ProjectDirtyChange, Trigger, recalc_and_emit,
};

// ---- the harness ------------------------------------------------------

/// A mock app in the healthy state the dirty service serves: a fresh
/// database in a temp root (the `projects_app` shape), the
/// [`DirtyState`] managed, and the event mounted — the registration
/// `lib.rs`'s setup performs for `tauri::Wry`, pinned here for the
/// mock runtime the emit/listen round trip needs (the specta builder
/// is runtime-generic; the `EventRegistry` mount is what makes `emit`
/// resolve the spec's wire name instead of panicking on a lookup).
///
/// Returns the bus's receiving half alongside the app: the test
/// decides whether and when the service task runs
/// ([`dirty_recalc::spawn_service`]) — the production spawn is pinned
/// by the `lib.rs` source scan instead.
fn dirty_app() -> (
    tempfile::TempDir,
    tauri::App<tauri::test::MockRuntime>,
    tauri::async_runtime::Receiver<Trigger>,
) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database =
        Db::open(temp.path()).expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    let (state, triggers) = DirtyState::new();
    app.manage(state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![DirtyEvent])
        .mount_events(&app);
    (temp, app, triggers)
}

/// Subscribes the typed listener the frontend's generated
/// `events.stateDirty` mirrors, returning the receiver every emitted
/// payload lands on. Delivery to Rust listeners is synchronous inside
/// `emit`, so once a recalc call has returned, the receiver already
/// holds everything that pass emitted.
fn listen_dirty(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> std::sync::mpsc::Receiver<DirtyEvent> {
    let (sender, receiver) = std::sync::mpsc::channel();
    tauri_specta::Event::listen(app, move |event| {
        let _ = sender.send(event.payload);
    });
    receiver
}

/// A local `Db` handle with the commit hook installed — the test's
/// write path. A clone of the managed `Db` (same connection; the hook
/// field is per-clone), so writes through it fire exactly the seam
/// production installs before `manage`; reads through the managed
/// clone behave identically either way (a read never fires).
fn hooked_handle(app: &tauri::App<tauri::test::MockRuntime>) -> Db {
    let mut database = app.state::<Db>().inner().clone();
    database.set_commit_hook(dirty_recalc::commit_hook(app.handle().clone()));
    database
}

/// The managed database handle — the reads and service passes.
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> Db {
    app.state::<Db>().inner().clone()
}

/// Waits until the service ran `at_least` passes, or fails the test
/// after a generous deadline. The pass counter is the deterministic
/// observable for "a pass happened" — an empty diff is silent on the
/// wire by contract.
fn wait_for_passes(app: &tauri::App<tauri::test::MockRuntime>, at_least: u64) {
    let state = app.state::<DirtyState>();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let passes = state.pass_count();
        if passes >= at_least {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the service ran only {passes} passes, waiting for {at_least}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---- seeding ----------------------------------------------------------

/// The DB-024 simulation: the sync pipeline's per-project hash store,
/// spelled as the raw column write that pipeline will make after a
/// `written`/`unchanged` status (the store has no writer for it yet —
/// projects.rs leaves the column to the sync tasks). The ids arrive
/// owned — the executor's closure is `'static`.
fn simulate_sync(database: &Db, project_id: String, hash: String) {
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.execute(
            "UPDATE projects SET last_synced_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, project_id],
        )
        .map(|_| ())
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("storing the synced hash must work");
}

/// Maps a library seeding failure onto the wire shape the executor
/// needs — test-local, the D-093 fallback family.
fn library_seed_error(error: eunomia_store::library::LibraryError) -> crate::errors::AppError {
    match error {
        eunomia_store::library::LibraryError::Sql(sql) => crate::db::sqlite_error_to_app_error(sql),
        other => {
            let mut app_error = crate::errors::AppError::new(crate::errors::ErrorCode::DbLocked);
            app_error.details = Some(crate::errors::ErrorDetails {
                os_code: None,
                os_kind: Some("LibrarySeed".to_owned()),
                os_message: Some(other.to_string()),
            });
            app_error
        }
    }
}

/// Seeds one scheme-A project with both extra files off (the unit
/// fixture's settings), returning its row id. The path arrives owned
/// — the executor's closure is `'static`.
fn seed_project(database: &Db, path: String, path_key: &'static str) -> String {
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path: &path,
                path_key,
                name: "Demo",
                label: "DEM",
                color: "red",
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
        .map(|project| project.id)
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("seeding a project through the repository must work")
}

/// Seeds one active instruction with its main variant, returning
/// `(instruction_id, variant_id)`. The spellings arrive as `'static`
/// literals — the executor's closure is `'static`, so nothing may
/// borrow the test's stack.
fn seed_instruction(database: &Db, title: &'static str, content: &'static str) -> (String, String) {
    tauri::async_runtime::block_on(database.run(move |tx| {
        let item = eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Instruction { title },
        )
        .map_err(library_seed_error)?;
        let variant = eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content,
                is_main: true,
            },
        )
        .map_err(library_seed_error)?;
        Ok((item.id, variant.id))
    }))
    .expect("seeding an instruction with its main variant must work")
}

/// Assigns `variant_id` of `instruction_id` to the project. The ids
/// arrive owned — the executor's closure is `'static`.
fn seed_assignment(database: &Db, project_id: String, instruction_id: String, variant_id: String) {
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project_id,
                instruction_id: &instruction_id,
                variant_id: &variant_id,
            },
        )
        .map(|_| ())
        .map_err(library_seed_error)
    }))
    .expect("seeding an assignment through the repository must work")
}

/// Hides an instruction (DB-013: the row keeps its assignments, the
/// build excludes it).
fn hide_instruction(database: &Db, instruction_id: String) {
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::hide_item(tx, &instruction_id)
            .map(|_| ())
            .map_err(library_seed_error)
    }))
    .expect("hiding the instruction must work")
}

/// Rewrites a variant's text — the edit of the library. Owned inputs,
/// the executor's `'static` rule.
fn edit_variant(database: &Db, variant_id: String, content: String) {
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::library::update_variant(
            tx,
            &variant_id,
            &eunomia_store::library::VariantUpdate {
                name: None,
                content: Some(&content),
            },
        )
        .map(|_| ())
        .map_err(library_seed_error)
    }))
    .expect("editing the variant must work")
}

/// The DB-024 hash of the plain one-block body over the fixture
/// project's target path — the value a sync of that build stores,
/// spelled directly through the public core seam (the same
/// composition the pipeline will hold), never through the code under
/// test. With both extra files off the managed block IS the body
/// (ALG-030 joins no reading lines around it), so the body string is
/// the hash's managed input verbatim.
fn synced_hash_of(path: &str, body: &str) -> String {
    build_hash(Scheme::A, Path::new(path), off(), off(), body)
}

/// The extras-off option of the fixture settings.
fn off() -> ExtraFile {
    ExtraFile {
        enabled: false,
        position: ExtraFilePosition::End,
    }
}

// ---- the direct entry: the card's criteria ---------------------------

#[test]
fn an_edit_affecting_the_build_makes_the_project_dirty_and_a_sync_clears_it() {
    // The card's criterion (а) over the whole pass, plus OUT-013's
    // story both ways: a synced-clean project goes dirty when an edit
    // changes the assigned variant's text (the event carries exactly
    // that change), stays dirty on an unchanged re-pass
    // (idempotence), and goes clean again once a sync stores the new
    // hash (DB-024) — the dirty→clean direction MAIN-028's dot must
    // clear on. This direct await is also the after-sync seam's shape
    // (§9.4 phase 4 step 5).
    let (temp, app, _triggers) = dirty_app();
    let database = database_of(&app);
    // The project's stored path is its repository ROOT (DB-030); the
    // hash's target path is the root joined with the base name — the
    // fixture spells both so the stored hash and the pass's computed
    // hash describe the same build.
    let root = temp.path().join("p1");
    let target = root.join("AGENTS.md");
    let project_id = seed_project(&database, root.display().to_string(), "/p1");
    let (instruction_id, variant_id) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(
        &database,
        project_id.clone(),
        instruction_id,
        variant_id.clone(),
    );
    simulate_sync(
        &database,
        project_id.clone(),
        synced_hash_of(&target.display().to_string(), "Be brief."),
    );
    let events = listen_dirty(&app);

    // Synced and clean: no change against the clean baseline, nothing
    // on the wire.
    let clean = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the clean pass must succeed");
    assert_eq!(clean, Vec::new());
    assert!(events.try_recv().is_err(), "a clean pass emits nothing");

    // The edit: the assigned variant's text changes, the build
    // follows, the hash differs — dirty.
    edit_variant(&database, variant_id, "Be very brief.".to_owned());
    let dirty = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the dirty pass must succeed");
    assert_eq!(
        dirty,
        vec![ProjectDirtyChange {
            project_id: project_id.clone(),
            dirty: true,
        }]
    );
    assert_eq!(
        events.try_recv().expect("the change must be emitted"),
        DirtyEvent {
            changes: vec![ProjectDirtyChange {
                project_id: project_id.clone(),
                dirty: true,
            }]
        }
    );

    // Idempotence: an unchanged re-pass is silent.
    let still = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the re-pass must succeed");
    assert_eq!(still, Vec::new());
    assert!(
        events.try_recv().is_err(),
        "an unchanged pass emits nothing"
    );

    // The sync stores the new build's hash (DB-024): clean again, and
    // the clearing IS a change the event carries.
    simulate_sync(
        &database,
        project_id.clone(),
        synced_hash_of(&target.display().to_string(), "Be very brief."),
    );
    let cleared = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the after-sync pass must succeed");
    assert_eq!(
        cleared,
        vec![ProjectDirtyChange {
            project_id: project_id.clone(),
            dirty: false,
        }]
    );
    assert_eq!(
        events.try_recv().expect("the clearing must be emitted"),
        DirtyEvent {
            changes: vec![ProjectDirtyChange {
                project_id,
                dirty: false,
            }]
        }
    );
}

#[test]
fn editing_a_hidden_instruction_without_assignments_leaves_every_project_clean() {
    // The card's criterion (б), through the full trigger chain: a
    // hidden instruction no project is assigned to contributes
    // nothing to any build (ALG-020's `included` requires the active
    // state), so editing its variant text moves no hash, no verdict,
    // and no event — while the committed write still fires the seam
    // and a debounced pass still RUNS (the counter proves it; the
    // empty receiver proves the silence).
    let (temp, app, triggers) = dirty_app();
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let target = root.join("AGENTS.md");
    let project_id = seed_project(&database, root.display().to_string(), "/p1");
    let (assigned_instruction, assigned_variant) =
        seed_instruction(&database, "Assigned", "Be brief.");
    seed_assignment(
        &database,
        project_id.clone(),
        assigned_instruction,
        assigned_variant,
    );
    let (hidden_instruction, hidden_variant) =
        seed_instruction(&database, "Buried", "Hidden text.");
    hide_instruction(&database, hidden_instruction);
    simulate_sync(
        &database,
        project_id,
        synced_hash_of(&target.display().to_string(), "Be brief."),
    );

    // The service with the seam: the startup pass runs clean and
    // silent, the edit triggers a debounced pass that stays silent.
    let writer = hooked_handle(&app);
    dirty_recalc::spawn_service(app.handle().clone(), triggers);
    let events = listen_dirty(&app);
    wait_for_passes(&app, 1);
    edit_variant(&writer, hidden_variant, "Hidden text, edited.".to_owned());
    wait_for_passes(&app, 2);
    // Both passes are done and nothing was ever emitted: the receiver
    // is empty because no verdict changed, not because no pass ran.
    assert!(
        events.try_recv().is_err(),
        "an edit outside every build must not mark any project dirty"
    );
}

// ---- the read side: projects_list serves the cache (T-254) -----------

/// Calls the real command over the mock app — the read the
/// frontend's generated `commands.projectsList` mirrors.
fn list_projects(
    app: &tauri::App<tauri::test::MockRuntime>,
    include_archived: bool,
) -> Vec<crate::commands::ProjectColumnEntry> {
    tauri::async_runtime::block_on(crate::commands::projects_list(
        app.handle().clone(),
        Some(include_archived),
    ))
    .expect("projects_list must succeed over the mock app")
}

/// The `dirty` flag of one seeded project's entry.
fn dirty_of(entries: &[crate::commands::ProjectColumnEntry], project_id: &str) -> bool {
    entries
        .iter()
        .find(|entry| entry.id == project_id)
        .unwrap_or_else(|| panic!("the seeded project {project_id} must be listed"))
        .dirty
}

#[test]
fn projects_list_serves_the_service_cache_verdicts() {
    // The T-254 core: the command's `dirty` is the service's last
    // known verdict, not a constant. The cache is seeded by ONE real
    // pass over a world with both verdicts in it — a synced-clean
    // project (DB-024 stored the matching hash) and a never-synced
    // project with an assignment (DB-021's first clause) — and the
    // list must reflect each project's own verdict.
    let (temp, app, _triggers) = dirty_app();
    let database = database_of(&app);
    let clean_root = temp.path().join("clean");
    let clean_target = clean_root.join("AGENTS.md");
    let clean_id = seed_project(&database, clean_root.display().to_string(), "/clean");
    let unsynced_root = temp.path().join("unsynced");
    let unsynced_id = seed_project(&database, unsynced_root.display().to_string(), "/unsynced");
    let (instruction_id, variant_id) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(
        &database,
        clean_id.clone(),
        instruction_id.clone(),
        variant_id.clone(),
    );
    seed_assignment(&database, unsynced_id.clone(), instruction_id, variant_id);
    simulate_sync(
        &database,
        clean_id.clone(),
        synced_hash_of(&clean_target.display().to_string(), "Be brief."),
    );
    let changes = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the cache-seeding pass must succeed");
    assert_eq!(
        changes,
        vec![ProjectDirtyChange {
            project_id: unsynced_id.clone(),
            dirty: true,
        }],
        "the pass found exactly the never-synced project dirty — the \
         cache now holds both verdicts"
    );

    let entries = list_projects(&app, false);
    assert!(
        !dirty_of(&entries, &clean_id),
        "the synced project serves its clean verdict"
    );
    assert!(
        dirty_of(&entries, &unsynced_id),
        "the never-synced project serves its dirty verdict"
    );
}

#[test]
fn projects_list_before_the_first_pass_answers_the_clean_baseline() {
    // The startup semantics, now the command's too (one truth): an
    // empty cache — no pass has run yet — reads every project clean,
    // a would-be-dirty one included; the first pass's event corrects
    // the dots afterwards (D-121's clean baseline).
    let (temp, app, _triggers) = dirty_app();
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let project_id = seed_project(&database, root.display().to_string(), "/p1");
    let (instruction_id, variant_id) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(&database, project_id.clone(), instruction_id, variant_id);

    let entries = list_projects(&app, false);
    assert!(
        !dirty_of(&entries, &project_id),
        "an unobserved project reads clean — the cache's absent ≡ \
         clean baseline"
    );
}

#[test]
fn an_archived_project_is_never_dirty_even_with_a_stale_cache_entry() {
    // DB-014's half, decided by the row whatever the cache holds:
    // the project went dirty (a real pass recorded `true`), was then
    // archived, and NO pass has pruned the cache yet — the archived
    // screen's entry must still read clean.
    let (temp, app, _triggers) = dirty_app();
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let project_id = seed_project(&database, root.display().to_string(), "/p1");
    let (instruction_id, variant_id) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(&database, project_id.clone(), instruction_id, variant_id);
    let changes = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the dirtying pass must succeed");
    assert_eq!(
        changes,
        vec![ProjectDirtyChange {
            project_id: project_id.clone(),
            dirty: true,
        }],
        "the cache holds the project's dirty verdict"
    );

    // Archive without another pass: the cache keeps the stale `true`
    // for the id (the next pass would prune it — the guard is the
    // row's state, not the cache's content).
    let archived_id = project_id.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::archive(tx, &archived_id)
            .map(|_| ())
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("archiving the project must work");

    let active = list_projects(&app, false);
    assert!(
        active.iter().all(|entry| entry.id != project_id),
        "the archived row left the default active read"
    );
    let archived = list_projects(&app, true);
    let entry = archived
        .iter()
        .find(|entry| entry.id == project_id)
        .expect("the archived read must list the archived row");
    assert_eq!(
        entry.state, "archived",
        "the listed row is the archived one — the dirty claim below is \
         about it"
    );
    assert!(
        !entry.dirty,
        "DB-014: an archived project is never counted as dirty, stale \
         cache entry or not"
    );
}

#[test]
fn repeated_projects_list_calls_are_stable() {
    // The read is pure against the cache: repeated calls over the
    // same seeded state answer identically — nothing is consumed or
    // reordered by a call.
    let (temp, app, _triggers) = dirty_app();
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let project_id = seed_project(&database, root.display().to_string(), "/p1");
    let (instruction_id, variant_id) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(&database, project_id.clone(), instruction_id, variant_id);
    let _ = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect("the cache-seeding pass must succeed");

    let first = list_projects(&app, false);
    let second = list_projects(&app, false);
    assert_eq!(
        first, second,
        "two reads over the same cache must answer identically"
    );
    assert!(
        dirty_of(&first, &project_id),
        "the anchored verdict is dirty — the stability claim is about \
         real values, not vacuous equality"
    );
}

#[test]
fn projects_list_without_the_managed_dirty_state_answers_the_clean_baseline() {
    // The missing-state fallback (the theoretical broken setup:
    // `setup` manages the state before anything that could fail, so
    // its absence is an uninitialized-app bug): the read still
    // succeeds — a display flag must not fail the whole column list
    // — and every verdict reads the clean baseline, exactly like an
    // empty cache (one truth: absent ≡ clean).
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let database =
        Db::open(temp.path()).expect("opening a fresh database in a temp directory must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    // No `DirtyState` managed — deliberately.
    app.manage(database);
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let project_id = seed_project(&database, root.display().to_string(), "/p1");
    let (instruction_id, variant_id) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(&database, project_id.clone(), instruction_id, variant_id);

    let entries = list_projects(&app, false);
    assert!(
        !dirty_of(&entries, &project_id),
        "no managed cache — the honest degraded verdict is the clean \
         baseline, not a failed read"
    );
}

// ---- the service task: startup, debounce, coalescing ------------------

#[test]
fn the_startup_pass_reports_never_synced_projects_with_assignments_dirty() {
    // DB-021's first clause through the startup spawn: a never-synced
    // project with an assignment and a non-empty build is dirty, and
    // the startup pass (the Immediate trigger, no debounce) says so
    // on the wire.
    let (temp, app, triggers) = dirty_app();
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let project_id = seed_project(&database, root.display().to_string(), "/p1");
    let (instruction_id, variant_id) = seed_instruction(&database, "Style", "Be brief.");
    seed_assignment(&database, project_id.clone(), instruction_id, variant_id);

    let events = listen_dirty(&app);
    let startup = Instant::now();
    dirty_recalc::spawn_service(app.handle().clone(), triggers);
    wait_for_passes(&app, 1);
    assert!(
        startup.elapsed() < Duration::from_secs(5),
        "the startup trigger is not debounced — a generous bound only, \
         the no-debounce half is pinned by the Commit direction below"
    );
    assert_eq!(
        events
            .try_recv()
            .expect("the startup dirty list must be emitted"),
        DirtyEvent {
            changes: vec![ProjectDirtyChange {
                project_id,
                dirty: true,
            }]
        }
    );
    // The service is still alive; dropping the app tears it down with
    // the runtime.
}

#[test]
fn a_write_burst_coalesces_into_one_debounced_pass() {
    // DB-022's window, both halves: the pass a committed write
    // schedules waits the debounce (the floor is guaranteed by the
    // sleep, so asserting it is safe on any runner), and a burst of
    // commits behind one window collapses into exactly one pass. The
    // window is widened past the production 150 ms so the coalescing
    // half is deterministic on a loaded runner (CI 44947235666: the
    // three `block_on` writes straddled a 150 ms window there).
    const BURST_WINDOW: std::time::Duration = std::time::Duration::from_millis(10_000);
    let (temp, app, triggers) = dirty_app();
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let _project_id = seed_project(&database, root.display().to_string(), "/p1");

    // The service settles after its startup pass and any residual
    // permit drains; the baseline count is taken after two extra
    // windows of quiet.
    dirty_recalc::spawn_service_with_window(app.handle().clone(), triggers, BURST_WINDOW);
    let _events = listen_dirty(&app);
    wait_for_passes(&app, 1);
    std::thread::sleep(BURST_WINDOW * 3);
    let settled = app.state::<DirtyState>().pass_count();

    // Three committed writes in quick succession — one permit's worth
    // of passes.
    let writer = hooked_handle(&app);
    let burst_start = Instant::now();
    for index in 0..3 {
        let title: &'static str = if index == 0 {
            "First"
        } else if index == 1 {
            "Second"
        } else {
            "Third"
        };
        tauri::async_runtime::block_on(writer.run(move |tx| {
            eunomia_store::library::create_item(
                tx,
                &eunomia_store::library::NewLibraryItem::Instruction { title },
            )
            .map(|_| ())
            .map_err(library_seed_error)
        }))
        .expect("the burst write must work");
    }
    wait_for_passes(&app, settled + 1);
    assert!(
        burst_start.elapsed() >= BURST_WINDOW,
        "the pass must wait the debounce window, not fire on the commit"
    );
    // No second pass follows: the burst's permits collapsed, and two
    // more windows of quiet prove it (a lower-bound style wait — the
    // channel holds no permit once the drain in the service loop ran).
    std::thread::sleep(BURST_WINDOW * 3);
    assert_eq!(
        app.state::<DirtyState>().pass_count(),
        settled + 1,
        "the burst must coalesce into exactly one pass"
    );
}

#[test]
fn read_only_transactions_do_not_trigger_a_pass() {
    // The seam's discrimination, end to end: reads through the hooked
    // handle commit transactions too (every `Db::run` does), but no
    // row changes — no permit, no pass.
    let (temp, app, triggers) = dirty_app();
    let database = database_of(&app);
    let root = temp.path().join("p1");
    let _project_id = seed_project(&database, root.display().to_string(), "/p1");

    dirty_recalc::spawn_service(app.handle().clone(), triggers);
    wait_for_passes(&app, 1);
    std::thread::sleep(dirty_recalc::DEBOUNCE * 3);
    let settled = app.state::<DirtyState>().pass_count();

    let reader = hooked_handle(&app);
    for _ in 0..3 {
        tauri::async_runtime::block_on(reader.run(move |tx| {
            eunomia_store::projects::list(tx, eunomia_store::projects::ListScope::Active)
                .map(|_| ())
                .map_err(crate::commands::projects_store_to_app_error)
        }))
        .expect("the read must work");
    }
    std::thread::sleep(dirty_recalc::DEBOUNCE * 3);
    assert_eq!(
        app.state::<DirtyState>().pass_count(),
        settled,
        "read-only transactions must not schedule a recalculation"
    );
}

// ---- the executor seam -------------------------------------------------

#[test]
fn the_commit_seam_fires_only_for_row_changing_transactions() {
    // The Db-level mechanics, no app: a write commits and fires once;
    // a read-only transaction and a zero-row UPDATE commit and stay
    // silent (the `total_changes` discrimination); a failing
    // transaction rolls back and never fires.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let mut database =
        Db::open(temp.path()).expect("opening a fresh database in a temp directory must work");
    let (fired, fired_receiver) = std::sync::mpsc::channel::<()>();
    database.set_commit_hook(std::sync::Arc::new(move || {
        let _ = fired.send(());
    }));

    // A write: one row changed, one fire.
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::create(
            tx,
            &NewProject {
                path: "/repos/demo",
                path_key: "/demo",
                name: "Demo",
                label: "DEM",
                color: "red",
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
        .map(|_| ())
        .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("the seeding write must work");
    assert_eq!(
        fired_receiver.try_recv(),
        Ok(()),
        "a write must fire the seam"
    );
    assert!(
        fired_receiver.try_recv().is_err(),
        "exactly one fire per committed write"
    );

    // A read: commits, changes nothing — silent.
    tauri::async_runtime::block_on(database.run(move |tx| {
        eunomia_store::projects::list(tx, eunomia_store::projects::ListScope::Active)
            .map(|_| ())
            .map_err(crate::commands::projects_store_to_app_error)
    }))
    .expect("the read must work");
    assert!(fired_receiver.try_recv().is_err(), "a read must not fire");

    // A zero-row write: commits, changes no row — silent.
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute(
            "UPDATE projects SET label = label WHERE id = 'no-such-row'",
            [],
        )
        .map(|_| ())
        .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("the no-op write must work");
    assert!(
        fired_receiver.try_recv().is_err(),
        "a zero-row write must not fire"
    );

    // A failing write: the whole transaction rolls back — no commit,
    // no row change, no fire (the path column is NOT NULL, so the
    // insert cannot succeed).
    let failing = tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute(
            "INSERT INTO projects (id, path_key, path, name, label, color, description) \
             VALUES ('x', NULL, NULL, NULL, NULL, NULL, NULL)",
            [],
        )
        .map(|_| ())
        .map_err(crate::db::sqlite_error_to_app_error)
    }));
    assert!(failing.is_err(), "the failing write must fail");
    assert!(
        fired_receiver.try_recv().is_err(),
        "a rolled-back transaction must not fire"
    );
}

// ---- recovery mode ------------------------------------------------------

#[test]
fn the_direct_entry_answers_honestly_without_a_database() {
    // The recovery-mode contract (the `availability` precedent): no
    // `Db` managed — the honest `NoDatabase` fallback, no event, no
    // panic. The service itself is never spawned in that mode (the
    // `lib.rs` setup returns before the spawn), pinned by the source
    // scan below.
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    let (state, _triggers) = DirtyState::new();
    app.manage(state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![DirtyEvent])
        .mount_events(&app);
    let events = listen_dirty(&app);

    let error = tauri::async_runtime::block_on(recalc_and_emit(app.handle()))
        .expect_err("a database-less pass must fail honestly");
    assert_eq!(
        error
            .details
            .as_ref()
            .and_then(|details| details.os_kind.as_deref()),
        Some("NoDatabase"),
        "the honest no-database answer, the managed_db edge every \
         database command shares"
    );
    assert!(events.try_recv().is_err(), "no event without a database");
}

// ---- set L: §12.2's budget ---------------------------------------------

/// Synthetic roots in the T-136 style: plain path strings, no folders
/// on disk — the dirty pass reads the database and builds text, it
/// never touches the file system (availability probing is another
/// service's concern).
fn synthetic_roots(prefix: &str, count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|index| {
            let path = format!("{prefix}/repo-{index:04}");
            let key = format!("{prefix}/repo-{index:04}");
            (path, key)
        })
        .collect()
}

#[test]
fn set_l_recalculation_answers_the_dirty_list_within_the_budget() {
    // The card's criterion (в): §12.2's set L — 500 projects, 1,000
    // instructions, 100 headings, 3 variants each, 100,000
    // assignments — recalculated by one pass. The functional oracle
    // is exact: every set-L project is never synced with 200
    // assignments and a non-empty build, so all 500 are dirty and the
    // first pass (clean baseline) reports exactly 500 changes. The
    // timing is measured and printed in the T-136 style; the assert
    // is a generous tripwire (the reference machine's 500 ms budget
    // is a release-build figure — the recorded debug number travels
    // in DECISIONS.md), wide enough never to flake on a slow CI
    // runner and tight enough to catch an accidental quadratic walk.
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    data_dirs::ensure_data_layout(temp.path())
        .expect("creating the data-directory layout in a temp directory must work");
    let db_path = data_dirs::db_file_path(temp.path());
    let backups_dir = data_dirs::db_backups_dir(temp.path());
    {
        let mut connection = open_db(&db_path).expect("opening the generator database");
        run_migrations(&mut connection, &backups_dir)
            .expect("migrating the generator database must work");
        let owned = synthetic_roots("/bench/l", 500);
        let roots: Vec<eunomia_store::data_sets::ProjectRoot<'_>> = owned
            .iter()
            .map(|(path, key)| eunomia_store::data_sets::ProjectRoot {
                path,
                path_key: key,
            })
            .collect();
        let start = Instant::now();
        eunomia_store::data_sets::generate(&mut connection, &eunomia_store::data_sets::L, &roots)
            .expect("generating set L must work");
        eprintln!("set L generation took {:?}", start.elapsed());
    }
    let database = Db::open(temp.path()).expect("opening the set-L database must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    app.manage(database);
    let (state, _triggers) = DirtyState::new();
    app.manage(state);
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![DirtyEvent])
        .mount_events(&app);
    let events = listen_dirty(&app);

    // The pass, timed by half: the read transaction (snapshot +
    // hashes) and the CPU half (build + hash) — the numbers the
    // DECISIONS entry records. The emit over a synchronous listener
    // is noise.
    let database = database_of(&app);
    let read_start = Instant::now();
    let (snapshot, rows) = tauri::async_runtime::block_on(database.run(move |transaction| {
        let t_snapshot = Instant::now();
        let snapshot = eunomia_store::build_snapshot::read(transaction)
            .map_err(crate::dirty_recalc::build_snapshot_to_app_error)?;
        let snapshot_elapsed = t_snapshot.elapsed();
        let t_list = Instant::now();
        let rows = eunomia_store::projects::list(transaction, ListScope::Active)
            .map_err(crate::commands::projects_store_to_app_error)?;
        let list_elapsed = t_list.elapsed();
        eprintln!(
            "set L read split: build_snapshot::read {snapshot_elapsed:?}, \
             projects::list {list_elapsed:?}"
        );
        Ok((snapshot, rows))
    }))
    .expect("the set-L read must work");
    let read_elapsed = read_start.elapsed();
    let stored_hashes: std::collections::HashMap<String, Option<String>> = rows
        .into_iter()
        .map(|project| (project.id, project.last_synced_hash))
        .collect();
    let compute_start = Instant::now();
    let precomputed =
        eunomia_core::content_build::compute_dirty_verdicts(&snapshot, &stored_hashes);
    let compute_elapsed = compute_start.elapsed();
    eprintln!(
        "set L phases: read {read_elapsed:?}, compute (sequential reference) \
         {compute_elapsed:?}; the budget row is the service pass below \
         (set M/L row of spec 12.2)"
    );
    assert_eq!(precomputed.len(), 500);
    assert!(precomputed.iter().all(|(_, dirty)| *dirty));

    // The service pass over the same database — the number the budget
    // is about (read + parallel compute + emit through the real
    // entry), and the sequential reference above must agree with it.
    let start = Instant::now();
    let changes =
        tauri::async_runtime::block_on(recalc_and_emit(app.handle())).expect("the set-L pass");
    let elapsed = start.elapsed();
    eprintln!("set L dirty recalculation took {elapsed:?} (budget: 500 ms)");
    assert_eq!(
        changes.len(),
        500,
        "every set-L project is never-synced with assignments and a build — all dirty"
    );
    assert!(changes.iter().all(|change| change.dirty));
    assert_eq!(
        changes
            .iter()
            .map(|change| change.project_id.as_str())
            .collect::<Vec<_>>(),
        precomputed
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        "the parallel pass and the sequential reference agree, id by id"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "the regression tripwire: an order of magnitude over the budget \
         means an accidental quadratic walk, not a slow runner"
    );
    // The event carries the same 500 changes (the wire half of the
    // same pass).
    let event = events.try_recv().expect("the dirty list must be emitted");
    assert_eq!(event.changes.len(), 500);
    assert!(event.changes.iter().all(|change| change.dirty));
}

// ---- the production wiring pin -----------------------------------------

#[test]
fn the_dirty_event_and_its_wiring_are_registered_with_the_specta_builder() {
    // The D-034/D-112 registration-pin rationale: the registration
    // cannot be exercised over the mock runtime (the builder serves
    // `tauri::Wry`), so the source is scanned — the event in
    // `collect_events!`, and every production wiring call site exactly
    // once.
    let lib = include_str!("lib.rs");
    assert_eq!(
        lib.matches("dirty_recalc::DirtyEvent").count(),
        1,
        "lib.rs must register the `state://dirty` event exactly once (§7.10)"
    );
    assert_eq!(
        lib.matches("dirty_recalc::DirtyState::new").count(),
        1,
        "lib.rs must manage the session state exactly once"
    );
    assert_eq!(
        lib.matches("dirty_recalc::commit_hook").count(),
        1,
        "lib.rs must install the executor's commit seam exactly once"
    );
    assert_eq!(
        lib.matches("dirty_recalc::spawn_service").count(),
        1,
        "lib.rs must spawn the service (DB-022's startup point) exactly once"
    );
}
