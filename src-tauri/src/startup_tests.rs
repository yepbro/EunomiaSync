//! T-132's startup-sequence suite: the §5.5 assembly — the order,
//! the two integration batteries, and the §12.2 window-ready budget —
//! verified end to end over the real open path.
//!
//! The card's essence: every step exists individually; this task
//! ASSEMBLES them. D-239 records the order map in prose — every
//! spec step's call site and its verdict; what this
//! suite pins in code:
//!
//! 1. **The order** (`the_startup_sequence_runs_the_spec_order`):
//!    the fourteen startup call sites of `lib.rs` — the `setup`
//!    hook's pre-open half and the shared
//!    [`crate::start_database_session`] tail — as one ordered source
//!    scan (the `config_tests.rs` wiring style). The
//!    load-bearing adjacencies the spec orders for safety — the daily
//!    copy before the invariants arm (the copy is the day's as-found
//!    snapshot, D-230), the interrupted sweep before the commit hook
//!    (D-147), the watch listener before the availability recheck —
//!    are pairs of this one scan. The open's own internal order (the
//!    version gate, the integrity gate, the pragma open, the
//!    migrations) is a second scan over `db.rs`
//!    (`the_open_gates_precede_the_data_touching_steps`); the
//!    pre-migration copy's before-migrations position lives inside
//!    the store's `run_migrations` and is pinned there, through the
//!    same public API `Db::open` calls
//!    (`crates/eunomia-store/tests/schema_v2.rs`:
//!    `upgrading_from_v1_writes_a_valid_pre_migration_copy_of_the_v1_state`
//!    — the copy's content is the source-version state, which only a
//!    copy taken before the step can be). The behavioral halves of
//!    the open gates — a too-new or damaged database rejected with
//!    the file byte-identical — are the existing `db_tests.rs` T-F11
//!    battery.
//! 2. **The empty-DB battery** (T-132's done-when): the whole
//!    sequence over a fresh data directory — the layout, `config.toml`,
//!    the log file, the open, the daily copy, the silent invariants,
//!    the publication, the services armed.
//! 3. **The existing-DB battery**: a seeded world (a project with
//!    its library and assignment, one gone folder, one corpse run
//!    row, an over-limit journal) — the sequence marks the corpse,
//!    writes the day's copy, prunes the journal to the settings
//!    limit, and the dirty/availability states converge.
//! 4. **The budget** (`startup_on_set_l_fits_the_ci_budget_bound`,
//!    §12.2/NFR-002): the full synchronous sequence over data set L,
//!    measured against the spec's 2 s row (with the documented CI
//!    margin — see [`WINDOW_READY_BUDGET`]) — plus the structure
//!    that makes the budget survivable, proven on the L world where
//!    the first dirty pass is slow enough to straddle the tail's
//!    return: the background passes are spawned, never awaited.
//! 5. **The memory row** (`memory_row::
//!    startup_rss_on_set_l_fits_the_memory_budget`, §12.2's
//!    "Memory (RSS, set L): no more than 400 MB"): the backend
//!    process's own footprint over the same L startup — measured in
//!    a FRESH process (this test binary re-executed, the
//!    `sync_kill_tests.rs` D-226 pattern), so the reading carries
//!    neither sibling tests' allocations nor the L generation's
//!    remnants. The child reports its own `/proc` checkpoints and
//!    the parent watches the child's `/proc` independently — the
//!    anti-stub cross-check that keeps the numbers honest. Linux
//!    only (`/proc`); the driver of both halves is
//!    `scripts/measure-startup.sh`, the card's repeatable
//!    measurement harness.
//!
//! # Honest boundaries
//!
//! * **`run()` is not drivable under a unit harness** (the house
//!   posture, D-147's note): the batteries drive the hook's healthy
//!   database half — layout → config → logging → open → the shared
//!   tail — exactly as the hook orders it, on the mock runtime's app
//!   with the session states the hook manages. The hook's abort and
//!   recovery arms (layout/config failure, `E_DB_CORRUPT`,
//!   `E_DB_TOO_NEW`) are `db_tests.rs`'s own batteries; the window
//!   itself is the webview's — what is measurable here is the
//!   backend's synchronous share, a necessary condition of the
//!   window-ready row.
//! * **`logging::init` installs the global subscriber**; the result
//!   of `set_global_default` is deliberately ignored in production
//!   and stays so here: under a multi-test process (`cargo test`,
//!   not nextest's process-per-test) the first caller wins the
//!   subscriber and later facades route to it. No assertion here
//!   reads log lines, so the cross-talk is benign; under nextest
//!   (the mandated chain) each test is its own process anyway.
//! * **Set L's roots are synthetic** (the store's own `data_sets.rs`
//!   suite precedent): the synchronous sequence never touches a
//!   project folder — the folder-touching passes (availability,
//!   watch) are spawned off-thread (SYS-001, NFR-002) — so synthetic
//!   roots measure the same synchronous work; the real-folder passes
//!   are the benches' and the e2e scripts' territory (§15.1's Load
//!   row).
//! * **The budget's reference machine** is the §12.2 reading the
//!   benches crate already recorded (D-135) and D-239 carries
//!   forward: the 2 s verdict is a release-profile rerun on the
//!   actual reference machine (the release gate, recorded as open);
//!   the CI assert is the spec number with a documented 2× margin —
//!   CI runs the debug profile on shared runners, several times the
//!   release cost — still falsifiable (a synchronous step growing
//!   past ~3 s goes red; a pass wrongly awaited is caught by the
//!   structure pin, deterministically, not by the clock).
//! * **The memory row's scope is the backend process.** The
//!   measurement child is the mock-runtime app — no webview exists
//!   under a unit harness — so its `/proc` reading is the backend's
//!   own footprint, a NECESSARY share of the row, not a sufficient
//!   one: the whole app adds the webview's processes, and the real
//!   app's main process adds the real runtime's share (the
//!   native host init the mock runtime replaces — neither is
//!   measured here, in either direction: the reading carries a
//!   ~60 MB harness + mock-runtime floor the real process does
//!   not, and the real runtime's own footprint is outside the
//!   measurement). What the assert therefore pins is the spec's
//!   number on this proxy, verbatim-bounded, with the direction
//!   unmeasured and the whole-app verdict (backend + webview,
//!   release profile, the reference machine) left to the release
//!   rerun, D-135's standing class, with its in-repo home in
//!   `docs/manual-smoke.md`; on macOS and Windows there is no
//!   `/proc`, and the row there is the release rerun's as well.

#![allow(clippy::print_stderr, clippy::print_stdout)] // the set-L budget
// and memory tests print their
// measurements deliberately (the
// `data_sets.rs` eprintln
// precedent; the memory row's
// child prints its report to the
// parent through the pipe)

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tauri::Manager;
use tauri::test::{mock_builder, mock_context, noop_assets};

use eunomia_store::data_sets::ProjectRoot;

// ---------------------------------------------------------------------------
// The harness and the sequence driver
// ---------------------------------------------------------------------------

/// A mock app in the state the `setup` hook leaves behind before the
/// open can fail into recovery mode: the session states the shared
/// tail arms over ([`crate::dirty_recalc::DirtyState`],
/// [`crate::file_watch::FileWatchState`],
/// [`crate::availability::AvailabilityState`]) with their buses'
/// receiving halves parked in [`crate::StateServices`], and the three
/// state events mounted the way the hook's first line mounts them (the
/// `db_verify_tests.rs` `session_app` shape — the emit/listen round
/// trip needs the `EventRegistry`). No command is registered: this
/// suite asserts state and files, not IPC.
fn session_app() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let temp = tempfile::tempdir().expect("failed to create a temp directory");
    let app = session_app_at(temp.path());
    (temp, app)
}

/// The [`session_app`] shell over an EXISTING data root — the same
/// states and events, no temp directory of its own: the memory
/// measurement's child process runs over a world another process
/// built (see `memory_row`), so it must not re-create the root, only
/// arm the shell the way the hook does over whatever is already
/// there. [`session_app`] itself is the two-line wrapper every other
/// test of this suite uses; the split is additive, the landed
/// callers' behavior byte-identical.
fn session_app_at(root: &Path) -> tauri::App<tauri::test::MockRuntime> {
    crate::data_dirs::ensure_data_layout(root)
        .expect("creating the data-directory layout must work");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app");
    let (dirty_state, dirty_triggers) = crate::dirty_recalc::DirtyState::new();
    app.manage(dirty_state);
    let (file_watch_state, file_watch_commands) = crate::file_watch::FileWatchState::new();
    app.manage(file_watch_state);
    app.manage(crate::availability::AvailabilityState::new());
    app.manage(crate::StateServices::parked(
        dirty_triggers,
        file_watch_commands,
    ));
    tauri_specta::Builder::<tauri::test::MockRuntime>::new()
        .events(tauri_specta::collect_events![
            crate::availability::ProjectAvailabilityEvent,
            crate::dirty_recalc::DirtyEvent,
            crate::file_watch::FileChangedEvent,
        ])
        .mount_events(&app);
    app
}

/// The `setup` hook's healthy database half, driven exactly as the
/// hook orders it (§5.5, SYS-011): layout → `config.toml` → file
/// logging → [`crate::db::Db::open`] → the shared tail. The `run()`
/// wrapper, the pre-open infallible state manages, and the abort/
/// recovery arms stay with the hook (see the module docs' honest
/// boundaries); what this driver is, is the ordered sequence itself —
/// the same calls, in the same order, over the harness's app.
fn startup_sequence(app: &tauri::App<tauri::test::MockRuntime>, data_dir: &Path) {
    crate::data_dirs::ensure_data_layout(data_dir)
        .expect("creating the data-directory layout must work");
    let (config, warnings) =
        crate::config::load_or_create(data_dir).expect("loading the config must work");
    app.manage(config);
    let level = app.state::<crate::config::AppConfig>().log_level;
    let logs_dir = data_dir.join(crate::data_dirs::LOGS_SUBDIRECTORY);
    let logger = crate::logging::init(&logs_dir, level);
    app.manage(logger);
    crate::config::emit_warnings(&warnings);
    crate::logging::startup(data_dir, level);
    let database = crate::db::Db::open(data_dir).expect("the database must open");
    crate::start_database_session(app.handle(), database);
}

/// The managed database (the `db_verify_tests.rs` shape).
fn database_of(app: &tauri::App<tauri::test::MockRuntime>) -> crate::db::Db {
    app.state::<crate::db::Db>().inner().clone()
}

/// The `db-backups/` directory of a data root.
fn backups_dir_of(root: &Path) -> PathBuf {
    root.join("db-backups")
}

/// The day's ERR-001 copy in `db-backups/`, if one exists — the
/// `daily-<YYYYMMDD>.sqlite3` scheme decided by the file name. The
/// name's stamp must be the copy's own UTC today (ERR-001's "if
/// there is none for today yet" is the store's naming rule, and this
/// helper refuses a wrongly dated name instead of accepting any
/// `daily-*` file).
fn daily_copy_of(root: &Path, now: jiff::Timestamp) -> Option<PathBuf> {
    let today = now.strftime("%Y%m%d").to_string();
    let mut found = None;
    for entry in std::fs::read_dir(backups_dir_of(root))
        .expect("the layout guarantees the db-backups directory is readable")
    {
        let name = entry
            .expect("reading a db-backups entry must work")
            .file_name()
            .into_string()
            .expect("the copy names are ASCII");
        if name.starts_with("daily-") && name.ends_with(".sqlite3") {
            assert_eq!(
                &name["daily-".len()..name.len() - ".sqlite3".len()],
                today,
                "the daily copy must be named for the UTC day it was written \
                 (ERR-001's same-day rule); found: {name}"
            );
            assert!(
                found.is_none(),
                "a single sequence run writes at most one daily copy (ERR-001's \
                 same-day rule); found a second: {name}"
            );
            found = Some(backups_dir_of(root).join(name));
        }
    }
    found
}

/// The journal's newest entries (up to 200) through the executor —
/// the assertions' window into what startup journaled.
fn journal_of(database: &crate::db::Db) -> Vec<eunomia_store::action_log::LogRecord> {
    tauri::async_runtime::block_on(database.run(|tx| {
        eunomia_store::action_log::recent(tx, 200)
            .map_err(crate::commands::activity_log_to_app_error)
    }))
    .expect("reading the journal must work")
}

/// How many rows the journal holds — raw SQL: the oracle is the
/// schema, and the retention task's observable is the count itself
/// (the `ipc_tests.rs` shape).
fn journal_size(database: &crate::db::Db) -> i64 {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        tx.query_row("SELECT COUNT(*) FROM action_log", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("counting the journal must work")
}

/// Polls the journal until it holds `expected` rows, or the deadline
/// passes; answers the last count read — the caller's `assert_eq!`
/// names the shortfall, never a swallowed timeout.
fn wait_for_journal_size(database: &crate::db::Db, expected: i64) -> i64 {
    let deadline = Instant::now() + EVENT_DEADLINE;
    let mut counted = journal_size(database);
    while counted != expected && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        counted = journal_size(database);
    }
    counted
}

/// Waits until at least `at_least` dirty passes ran (the
/// `ipc_tests.rs` shape) — the services' startup passes run on the
/// global runtime, off this thread.
fn wait_for_dirty_passes(app: &tauri::App<tauri::test::MockRuntime>, at_least: u64) {
    let state = app.state::<crate::dirty_recalc::DirtyState>();
    let deadline = Instant::now() + EVENT_DEADLINE;
    loop {
        let passes = state.pass_count();
        if passes >= at_least {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the dirty service ran only {passes} passes, waiting for {at_least}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until no availability recheck holds the in-flight slot —
/// the startup pass the tail spawns must have finished before the
/// test ends (or the temp directory would drop under a running
/// probe) — the `ipc_tests.rs` shape.
fn wait_for_availability_slot(app: &tauri::App<tauri::test::MockRuntime>) {
    let slot = app.state::<crate::availability::AvailabilityState>();
    let deadline = Instant::now() + EVENT_DEADLINE;
    while slot.running.is_taken() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !slot.running.is_taken(),
        "the startup availability recheck never released its slot"
    );
}

/// One expected `state://dirty` diff entry — the typed payload's own
/// shape, spelled out per test, never computed from the code under
/// test (the `ipc_tests.rs` shape).
fn dirty_change(project_id: &str, dirty: bool) -> crate::dirty_recalc::ProjectDirtyChange {
    crate::dirty_recalc::ProjectDirtyChange {
        project_id: project_id.to_owned(),
        dirty,
    }
}

/// Listens for `state://dirty` (the `ipc_tests.rs` shape) — the
/// receiver must exist before the sequence runs, or the session's
/// first diff would miss it.
fn listen_dirty(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> std::sync::mpsc::Receiver<crate::dirty_recalc::DirtyEvent> {
    let (sender, receiver) = std::sync::mpsc::channel();
    tauri_specta::Event::listen(app, move |event| {
        let _ = sender.send(event.payload);
    });
    receiver
}

/// Listens for `state://project-availability` (the `ipc_tests.rs`
/// shape) — registered before the sequence, for the same reason.
fn listen_availability(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> std::sync::mpsc::Receiver<crate::availability::ProjectAvailabilityEvent> {
    let (sender, receiver) = std::sync::mpsc::channel();
    tauri_specta::Event::listen(app, move |event| {
        let _ = sender.send(event.payload);
    });
    receiver
}

/// How long a test waits for a pass, an event or a prune it knows
/// must come — generous for a loaded runner and finite so a missing
/// service fails the test instead of hanging it (the `ipc_tests.rs`
/// value).
const EVENT_DEADLINE: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// The world seeders (the `ipc_tests.rs` / `db_tests.rs` shapes)
// ---------------------------------------------------------------------------

/// The `log.max_entries` value the seeded journal runs against — the
/// §6.5 range's floor, written through the settings repository (the
/// `ipc_tests.rs` recovery constants).
const JOURNAL_LIMIT: i64 = 1000;

/// How many journal rows the seeded world carries: over the limit by
/// a margin no off-by-one could hide (the `ipc_tests.rs` value).
const JOURNAL_SEEDED: i64 = 1050;

/// One project row's display facts, the `ipc_tests.rs`
/// `column_project` shape.
fn column_project<'a>(path: &'a str, path_key: &'a str) -> eunomia_store::projects::NewProject<'a> {
    use eunomia_store::projects::AddedVia;
    use eunomia_store::settings::{BaseName, ExtraFile, Position, Scheme};
    eunomia_store::projects::NewProject {
        path,
        path_key,
        name: "Startup world",
        label: "SW",
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

/// Seeds one project AND its first assigned instruction in ONE
/// transaction (the `ipc_tests.rs` `seed_assigned_project` shape): a
/// never-synced project with a non-empty build is dirty on its first
/// observation (DB-021's first clause) — the verdict the startup pass
/// must converge to. The title is a parameter because the library's
/// titles are unique (DB-010). Answers the project row id.
fn seed_assigned_project(
    database: &crate::db::Db,
    path: String,
    path_key: &'static str,
    title: &'static str,
    content: &'static str,
) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = column_project(&path, path_key);
        let project = eunomia_store::projects::create(tx, &new)
            .map_err(crate::commands::projects_store_to_app_error)?;
        let item = eunomia_store::library::create_item(
            tx,
            &eunomia_store::library::NewLibraryItem::Instruction { title },
        )
        .map_err(crate::dirty_recalc::library_seed_error_for_tests)?;
        let variant = eunomia_store::library::create_variant(
            tx,
            &eunomia_store::library::NewVariant {
                instruction_id: &item.id,
                name: "Main",
                content,
                is_main: true,
            },
        )
        .map_err(crate::dirty_recalc::library_seed_error_for_tests)?;
        eunomia_store::library::assign(
            tx,
            &eunomia_store::library::NewAssignment {
                project_id: &project.id,
                instruction_id: &item.id,
                variant_id: &variant.id,
            },
        )
        .map_err(crate::dirty_recalc::library_seed_error_for_tests)?;
        Ok(project.id)
    }))
    .expect("seeding the assigned project through the repositories must work")
}

/// Seeds one project whose folder does not exist — the DB-032/D-09
/// half the availability startup pass must report (the `ipc_tests.rs`
/// recovery test's gone folder). Answers the project row id.
fn seed_gone_project(database: &crate::db::Db, path: String, path_key: &'static str) -> String {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        let new = column_project(&path, path_key);
        let project = eunomia_store::projects::create(tx, &new)
            .map_err(crate::commands::projects_store_to_app_error)?;
        Ok(project.id)
    }))
    .expect("seeding the gone project through the repositories must work")
}

/// Plants the crash-shaped `sync_runs` rows by raw SQL (the
/// `db_tests.rs` sweep-test shape): one corpse a dead process left
/// `running`, one settled `success` row the sweep must not touch. The
/// oracle is the schema, not the store's DTOs.
fn plant_run_rows(database: &crate::db::Db) {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        tx.execute_batch(
            "INSERT INTO sync_runs (id, started_at, status) VALUES \
                 ('corpse', '2026-11-27T12:00:00.000Z', 'running'); \
             INSERT INTO sync_runs (id, started_at, finished_at, status) VALUES \
                 ('settled', '2026-11-27T11:00:00.000Z', \
                  '2026-11-27T11:01:00.000Z', 'success');",
        )
        .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(())
    }))
    .expect("planting the crash-shaped rows must work")
}

/// Reads the run rows back by raw SQL (the `db_tests.rs` shape):
/// `(id, status, finished_at, error_code)`, oldest first.
fn read_run_rows(
    database: &crate::db::Db,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(|tx| {
        let mut statement = tx
            .prepare(
                "SELECT id, status, finished_at, error_code FROM sync_runs \
                      ORDER BY started_at",
            )
            .map_err(crate::db::sqlite_error_to_app_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(crate::db::sqlite_error_to_app_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::db::sqlite_error_to_app_error)?;
        Ok(rows)
    }))
    .expect("reading the run rows back must work")
}

/// Leaves the journal over its limit (the `ipc_tests.rs` shape):
/// `log.max_entries` at the range floor through the settings
/// repository, then the user entries through the journal's own writer
/// in one transaction — the state LOG-006's startup run must prune.
fn seed_journal_over_the_limit(database: &crate::db::Db) {
    let setting_db = database.clone();
    tauri::async_runtime::block_on(setting_db.run(move |tx| {
        eunomia_store::settings::set(
            tx,
            eunomia_store::settings::SettingKey::LogMaxEntries,
            eunomia_store::settings::SettingValue::Int(JOURNAL_LIMIT),
        )
        .map_err(crate::commands::settings_write_to_app_error)?;
        Ok(())
    }))
    .expect("seeding the journal limit must work");
    let database = database.clone();
    tauri::async_runtime::block_on(database.run(move |tx| {
        use eunomia_store::action_log::{Action, Level, LogEntry, Outcome, Visibility, insert};
        for _ in 0..JOURNAL_SEEDED {
            insert(
                tx,
                &LogEntry::new(
                    Action::ProjectAdded,
                    Level::Info,
                    Visibility::User,
                    Outcome::Ok,
                ),
            )
            .map_err(crate::commands::activity_log_to_app_error)?;
        }
        Ok(())
    }))
    .expect("seeding the journal through its writer must work");
}

// ---------------------------------------------------------------------------
// The order pins
// ---------------------------------------------------------------------------

#[test]
fn the_startup_sequence_runs_the_spec_order() {
    // The §5.5 assembly, pinned as one ordered source scan of
    // `lib.rs` (the `config_tests.rs` wiring style — comments
    // stripped, live code only). Every marker is a spec step's call
    // site; the ascending positions are the order:
    //
    // * the hook: layout (§5.5) → config (SYS-011: "read at startup,
    //   before logging and the database are set up") → file logging
    //   (§5.5's logs subfolder) → `Db::open`;
    // * the shared tail: the §9.4 interrupted sweep (D-147: BEFORE
    //   the commit hook — the marking's row change must not drop a
    //   dirty trigger for a pass that reads projects, not runs) →
    //   the DB-022 commit seam → the ERR-001 daily copy → the §6.3
    //   invariant check (D-230: after the daily copy — the copy is
    //   the day's as-found snapshot — and after the hook, whose seam
    //   the check's journal writes ride) → the publication → the
    //   state services in their own order (the dirty recalc's
    //   startup pass first — its own startup permit, D-121; the
    //   watch listener BEFORE the availability recheck can emit; the
    //   LOG-006 retention task; DB-032's availability recheck last).
    //
    // Honest boundary, same as every wiring pin: this proves the
    // wiring exists and stays wired in this order, not that the OS
    // file opened — the behavioral halves are this suite's two
    // integration batteries and the `db_tests.rs` gate batteries.
    let lib = crate::config_tests::strip_comments(include_str!("lib.rs"));
    let markers = [
        "data_dirs::ensure_data_layout(&data_dir)?",
        "config::load_or_create(&data_dir)?",
        "logging::init(&logs_dir, level)",
        "db::Db::open(&data_dir)",
        "database.mark_interrupted_runs(",
        "database.set_commit_hook(",
        "database.ensure_daily_backup(",
        "db_verify::startup_check(",
        "app.manage(database);",
        "dirty_recalc::spawn_service(",
        "file_watch::listen_availability(",
        "file_watch::spawn_service(",
        "retention::spawn_daily_cleanup(",
        "availability::spawn_recheck(",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                lib.matches(marker).count(),
                1,
                "lib.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            lib.find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "lib.rs must run the §5.5 startup steps in the spec's order \
             (layout, config, logging, open; then the tail: sweep, commit \
             hook, daily copy, invariant check, publication, services — \
             the dirty pass, the watch listener and service, the retention \
             task, the availability recheck); marker positions: \
             {positions:?}"
        );
    }
}

#[test]
fn the_open_gates_precede_the_data_touching_steps() {
    // The open's internal order (§6.3, DB-004, T-F11), pinned as an
    // ordered source scan of `db.rs`: the too-new gate → the
    // quick_check gate → the pragma open → the migrations. Both
    // gates run on bare, read-only connections BEFORE `open_db`:
    // the DB-001 pragma set *writes* to the file (the
    // `journal_mode=WAL` switch rewrites the header of a
    // rollback-journal database), so §10's effects — `E_DB_TOO_NEW`:
    // "The app does not change the database"; `E_DB_CORRUPT`: the
    // file stays byte-identical — hold only in this order. The
    // version gate runs first (DB-004's own order note: the cheaper
    // `user_version` read rejects a newer database before the
    // page-walking `quick_check` pays for it) — behaviorally pinned
    // by `db_tests.rs`'s `a_too_new_and_damaged_database_answers_too_new_first`
    // and the T-F11 byte-identity batteries.
    //
    // The pre-migration copy's before-migrations position is NOT
    // here: it lives inside `run_migrations` (the store crate) and
    // is pinned there through the same public API this open calls —
    // `schema_v2.rs`'s
    // `upgrading_from_v1_writes_a_valid_pre_migration_copy_of_the_v1_state`
    // proves the copy's content is the source-version state, which
    // only a copy taken before the step can be.
    let db_module = crate::config_tests::strip_comments(include_str!("db.rs"));
    let markers = [
        "reject_too_new(&db_path)?;",
        "reject_corrupt(&db_path)?;",
        "eunomia_store::storage::open_db(&db_path)",
        "eunomia_store::storage::run_migrations(&mut connection, &backups_dir)",
    ];
    let positions: Vec<usize> = markers
        .iter()
        .map(|marker| {
            assert_eq!(
                db_module.matches(marker).count(),
                1,
                "db.rs must contain `{marker}` exactly once, live (a \
                 commented-out or duplicated line must not count)"
            );
            db_module
                .find(marker)
                .expect("the exactly-once check above guarantees one match")
        })
        .collect();

    for pair in positions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "db.rs must run both open gates (the too-new version gate, \
             then quick_check) before the pragma open and the migrations — \
             a write before the gates would change a database §10 orders \
             left untouched; marker positions: {positions:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The integration batteries
// ---------------------------------------------------------------------------

#[test]
fn startup_on_an_empty_database_runs_the_whole_sequence() {
    // T-132's done-when, the empty half: the whole sequence runs over
    // a fresh data directory — the layout, the config, the log, the
    // open (a fresh DB: the bootstrap run applies every migration
    // but writes no pre-migration copies — D-038's fresh-run rule;
    // no corpses, no
    // violations), the daily copy (ERR-001: "at every startup" — a
    // fresh first launch gets its copy too, D-046), the publication,
    // and the services armed over the published database.
    let (temp, app) = session_app();
    let dirty_events = listen_dirty(&app);
    let availability_events = listen_availability(&app);
    // The day the copy must be named for, captured BEFORE the
    // sequence runs (the copy is stamped at open time; a fresh
    // `now` at assertion time could straddle a UTC midnight and
    // fail spuriously).
    let day = jiff::Timestamp::now();

    startup_sequence(&app, temp.path());

    // The pre-database steps left their artifacts: the §5.5 layout
    // (config.toml created by SYS-011's missing-file rule), and the
    // log file in `logs/` (the naming grammar the module owns).
    assert!(
        temp.path().join("config.toml").exists(),
        "the config step must create config.toml in the data root"
    );
    let logs_dir = temp.path().join(crate::data_dirs::LOGS_SUBDIRECTORY);
    let log_names: Vec<String> = std::fs::read_dir(&logs_dir)
        .expect("the logs directory must be readable")
        .map(|entry| {
            entry
                .expect("reading a logs entry must work")
                .file_name()
                .into_string()
                .expect("the log names are ASCII")
        })
        .collect();
    assert!(
        log_names
            .iter()
            .any(|name| name.starts_with("eunomiasync.") && name.ends_with(".log")),
        "the logging step must leave the file log under logs/; found \
         {log_names:?}"
    );

    // The database is published — and it is the fresh one (§5.5's
    // "the app starts with an empty DB and says so" premise:
    // `created_fresh`).
    let database = database_of(&app);
    assert!(
        database.created_fresh(),
        "the first open over an empty data directory must report the \
         database as created fresh"
    );

    // ERR-001's first-launch copy: exactly one daily copy, a valid
    // database at the schema the open left behind.
    let copy =
        daily_copy_of(temp.path(), day).expect("a fresh startup must write the day's ERR-001 copy");
    let copy_connection = rusqlite::Connection::open(&copy).expect("the daily copy must open");
    let integrity: String = copy_connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("the copy's integrity must answer");
    assert_eq!(integrity, "ok", "the daily copy must be a valid database");
    let schema: i64 = copy_connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("the copy's schema version must answer");
    assert_eq!(
        schema,
        eunomia_store::storage::latest_schema_version() as i64,
        "the daily copy of a fresh open carries the fully migrated schema"
    );

    // The healthy startup is silent: no invariant entry (DB-017's
    // zero-case), no run rows (§9.4's sweep found nothing to mark).
    assert!(
        journal_of(&database).is_empty(),
        "an empty startup must journal nothing — no invariant entry"
    );
    let runs: i64 = tauri::async_runtime::block_on(database.clone().run(|tx| {
        tx.query_row("SELECT COUNT(*) FROM sync_runs", [], |row| row.get(0))
            .map_err(crate::db::sqlite_error_to_app_error)
    }))
    .expect("counting the runs must work");
    assert_eq!(runs, 0, "an empty startup has no run rows to mark");

    // The services are armed exactly once: the parked start is spent
    // (only the publishing call spends it), the dirty service's own
    // startup pass ran, and the availability pass's in-flight slot is
    // free — the teardown guard only: the slot is taken INSIDE the
    // spawned task, so a free slot cannot prove the pass was ever
    // polled. On this empty world that is benign by construction —
    // the pass has no project to probe, touches no path, and its
    // only work is one empty read; the battery that DOES have a
    // path-bearing pass (the existing-DB one) awaits the pass's own
    // observable, the event, instead of trusting the slot.
    assert!(
        app.state::<crate::StateServices>().take_parked().is_none(),
        "the session's one-shot service start must be spent by the \
         publishing call"
    );
    wait_for_dirty_passes(&app, 1);
    wait_for_availability_slot(&app);

    // The empty world is silent on both wire oracles: no dirty
    // verdict to report (empty diffs are not emitted), no
    // availability change (there is no project to probe).
    assert!(
        dirty_events.try_recv().is_err(),
        "an empty startup emits no state://dirty diff"
    );
    assert!(
        availability_events.try_recv().is_err(),
        "an empty startup emits no state://project-availability diff"
    );
    drop(app);
    drop(temp);
}

#[test]
fn startup_on_an_existing_database_recovers_and_converges() {
    // T-132's done-when, the existing half: a seeded world — an
    // assigned project with a live folder, a project whose folder is
    // gone, one corpse run row, an over-limit journal — runs the
    // same sequence and converges: the corpse is marked
    // `interrupted` (§9.4), the settled row keeps its terminal shape,
    // the day's copy exists and preserves the world, LOG-006's
    // startup run prunes the journal to the settings limit, the
    // invariants stay silent (a clean seeded world), and the two
    // state services converge over the seeded world: the assigned
    // project reads dirty (DB-021's first clause), the gone project
    // reports unavailable (DB-032).
    let (temp, app) = session_app();
    let dirty_events = listen_dirty(&app);
    let availability_events = listen_availability(&app);

    // Arrange: the world exists BEFORE the app starts. The seeding
    // session opens the database through the same `Db::open`, seeds
    // through the repositories, and is dropped — the sequence below
    // then re-opens the file as any restart would.
    let seeder = crate::db::Db::open(temp.path()).expect("opening the seeding session must work");
    let live_folder = temp.path().join("repos").join("live");
    // The probe's fourth check (D-084): the folder must hold a `.git`
    // entry — the honest shape of a registered project's root, so
    // the live project probes AVAILABLE and the gone one is the only
    // verdict that changes.
    std::fs::create_dir_all(live_folder.join(".git"))
        .expect("creating the live project folder must work");
    let assigned = seed_assigned_project(
        &seeder,
        live_folder.to_string_lossy().into_owned(),
        "seed-live",
        "Be brief",
        "Answer in one sentence.",
    );
    let gone = seed_gone_project(
        &seeder,
        temp.path()
            .join("repos")
            .join("gone") // never created — the DB-032/D-09 case
            .to_string_lossy()
            .into_owned(),
        "seed-gone",
    );
    plant_run_rows(&seeder);
    seed_journal_over_the_limit(&seeder);
    drop(seeder);

    // Act: the startup sequence, exactly as the hook orders it
    // (the day captured before it, for the copy's stamp).
    let day = jiff::Timestamp::now();
    startup_sequence(&app, temp.path());

    // The database is published — and NOT fresh (§5.5's existing-DB
    // premise).
    let database = database_of(&app);
    assert!(
        !database.created_fresh(),
        "a restart over an existing database must not report it fresh"
    );

    // §9.4's sweep: the corpse went `interrupted` with `finished_at`
    // stamped at the marking moment (D-147: the moment the row went
    // terminal — the stamp is the sweep's own now, so the assertion
    // is the shape, not the wall clock) and `error_code` left NULL;
    // the settled row keeps its own terminal shape verbatim (read by
    // raw SQL — the oracle is the schema, the `db_tests.rs` shape).
    let runs = read_run_rows(&database);
    assert_eq!(runs.len(), 2, "the sweep invents no rows and drops none");
    assert_eq!(
        (
            runs[0].0.as_str(),
            runs[0].1.as_str(),
            &runs[0].2,
            &runs[0].3
        ),
        (
            "settled",
            "success",
            &Some("2026-11-27T11:01:00.000Z".to_owned()),
            &None
        ),
        "the settled row keeps its own terminal shape"
    );
    assert_eq!(
        (runs[1].0.as_str(), runs[1].1.as_str(), &runs[1].3),
        ("corpse", "interrupted", &None),
        "the corpse is marked interrupted with no error code"
    );
    assert!(
        runs[1].2.is_some(),
        "the interrupted row carries the marking moment's finish stamp"
    );

    // ERR-001's copy for the day exists and preserves the seeded
    // world (a valid database holding both project rows — the copy
    // is content-agnostic about the marking's position relative to
    // it, which the spec does not order).
    let copy =
        daily_copy_of(temp.path(), day).expect("the restart must write the day's ERR-001 copy");
    let copy_connection = rusqlite::Connection::open(&copy).expect("the daily copy must open");
    let projects: i64 = copy_connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .expect("the copy's project count must answer");
    assert_eq!(
        projects, 2,
        "the daily copy preserves the seeded world's rows"
    );

    // The invariants stay silent over a clean seeded world: no
    // error-level entry (DB-017's diagnostic posture — violations
    // journal, and this world has none).
    assert!(
        journal_of(&database)
            .iter()
            .all(|entry| entry.level.as_str() != "error"),
        "a clean seeded world must journal no error-level entry at startup"
    );

    // The services armed: the parked start is spent.
    assert!(
        app.state::<crate::StateServices>().take_parked().is_none(),
        "the session's one-shot service start must be spent by the \
         publishing call"
    );

    // LOG-006's startup run: the over-limit journal is pruned back to
    // the settings limit by the retention task the tail spawned
    // (its startup pass is the task's own first tick).
    assert_eq!(
        wait_for_journal_size(&database, JOURNAL_LIMIT),
        JOURNAL_LIMIT,
        "the retention task's startup run must bring the journal back \
         under log.max_entries"
    );

    // The dirty state converges: the assigned project reads dirty on
    // its first observation (never synced, non-empty build — DB-021's
    // first clause), and the wire diff carries exactly it (the
    // gone-but-unassigned project builds empty and reads clean).
    wait_for_dirty_passes(&app, 1);
    let dirty_state = app.state::<crate::dirty_recalc::DirtyState>();
    assert!(
        dirty_state.last_verdict(&assigned),
        "the assigned, never-synced project must read dirty after the \
         startup pass"
    );
    assert!(
        !dirty_state.last_verdict(&gone),
        "the project with no assignments must read clean (DB-021's \
         empty-build exception)"
    );
    assert_eq!(
        dirty_events
            .try_recv()
            .expect("the startup pass must report its first diff")
            .changes,
        vec![dirty_change(&assigned, true)],
        "the session's first dirty diff carries exactly the newly dirty \
         project"
    );
    assert!(
        dirty_events.try_recv().is_err(),
        "no second diff — the prune's commit diff is empty (the world \
         changed nothing the pass reads)"
    );

    // The availability state converges: the gone project is reported
    // unavailable on the session's first probe (DB-032's startup
    // check point; the baseline rule — the live project emits
    // nothing). The event is AWAITED with a deadline, not polled:
    // `spawn_recheck` takes its in-flight slot inside the spawned
    // task, so a free slot proves nothing about the pass having
    // started — the event itself is the pass's observable (the slot
    // wait below stays as the teardown guard it is).
    let availability = availability_events
        .recv_timeout(EVENT_DEADLINE)
        .expect("the startup availability pass must report the gone folder")
        .changes;
    assert_eq!(
        availability.len(),
        1,
        "only the gone project changed its verdict"
    );
    assert_eq!(availability[0].project_id, gone);
    assert!(
        !availability[0].available,
        "the gone project is unavailable"
    );
    assert!(
        availability[0].unavailable_reason.is_some(),
        "the unavailable verdict carries its D-084 reason"
    );
    assert!(
        availability_events.try_recv().is_err(),
        "no second availability diff at startup"
    );
    // The teardown guard: the pass that emitted the event may still
    // hold its in-flight slot — the temp directory must not drop
    // under a running probe.
    wait_for_availability_slot(&app);
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// The §12.2 budget on set L
// ---------------------------------------------------------------------------

/// §12.2's window-ready budget, verbatim: "Startup until the window
/// is ready (set L): no more than 2 s". What a unit harness can
/// measure of that row is the backend's synchronous share — the
/// whole sequence this suite drives (layout, config, logging, the
/// open with its gates and the no-op migration run, the tail's
/// synchronous steps) — which is a *necessary condition* of the row:
/// the window cannot be ready before the backend is, and the
/// webview's own load is the e2e scripts' half (§15.1's Load row).
///
/// The assert carries a documented margin for where it runs
/// (§12.2's own wording settles this: the targets are "checked with
/// a benchmark **before release on a reference machine**" — CI is
/// not that machine): CI executes the DEBUG profile on shared,
/// loaded runners, where the sequence's measured share is several
/// times its release-profile cost, so the hard CI bound is the spec
/// number times [`CI_TOLERANCE`] — tight enough that a synchronous
/// step growing past ~3 s of work goes red, generous enough that a
/// slow runner does not flake the gate. The dev-box measurement and
/// the release-gate rerun obligation are recorded in the task's
/// DECISIONS.md entry of this task, D-239 (the D-135 "reference machine, honestly"
/// reading — the actual 2 s verdict is a release-profile rerun on
/// the reference machine, the same standing ruling the benches
/// crate's numbers carry).
const WINDOW_READY_BUDGET: Duration = Duration::from_secs(2);

/// The margin the CI assert grants over [`WINDOW_READY_BUDGET`] —
/// see its docs for the reading.
const CI_TOLERANCE: u32 = 2;

/// Builds data set L (§12.2: 500 projects, 1000 instructions, 100
/// headings, 3 variants per instruction, 100,000 assignments) as a
/// database file under the data root's `db/` — the store's own
/// `data_sets.rs` suite's synthetic-root spelling: the roots are path
/// strings, no folders, because the synchronous sequence never
/// touches a project folder (the folder-touching passes are spawned
/// — SYS-001/NFR-002). Answers the created row ids for addressing.
fn build_set_l(root: &Path) -> eunomia_store::data_sets::GeneratedDataSet {
    let started = Instant::now();
    let db_path = crate::data_dirs::db_file_path(root);
    let mut connection = eunomia_store::storage::open_db(&db_path)
        .expect("opening the set-L fixture database must work");
    eunomia_store::storage::run_migrations(
        &mut connection,
        &crate::data_dirs::db_backups_dir(root),
    )
    .expect("migrating the set-L fixture must work");
    let roots: Vec<(String, String)> = (0..500)
        .map(|index| {
            let path = format!("/set-l/repo-{index:04}");
            let path_key = path.to_lowercase();
            (path, path_key)
        })
        .collect();
    let borrowed: Vec<ProjectRoot<'_>> = roots
        .iter()
        .map(|(path, path_key)| ProjectRoot { path, path_key })
        .collect();
    let generated = eunomia_store::data_sets::generate(
        &mut connection,
        &eunomia_store::data_sets::L,
        &borrowed,
    )
    .expect("generating set L must work");
    eprintln!("set L generation took {:?}", started.elapsed());
    generated
}

#[test]
fn startup_on_set_l_fits_the_ci_budget_bound() {
    // §12.2's row, measured over the real sequence on the real
    // upper-limit data set — asserted at the CI bound the test is
    // named for (the spec's 2 s × the documented margin; the 2 s
    // verdict itself is the release-gate rerun, D-239) — and the
    // structure that protects the
    // budget, proven on the same world: the background passes are
    // spawned, never awaited, so the heavy work cannot hold the
    // launch (SYS-001; the NFR-002 posture every heavy step of the
    // tail already documents). Set L is the world where that is
    // observable: the first dirty pass — the whole snapshot read,
    // the 500 project builds, the hash comparisons — takes hundreds
    // of milliseconds (D-135 measured 383 ms in the bench profile),
    // while the tail's synchronous remainder after the spawn is
    // microseconds; if a future change makes the tail AWAIT the
    // pass, the pass completes before the tail returns and the
    // `pass_count` assertion below goes red (the deterministic
    // mutation — see D-239's mutation record).
    let (temp, app) = session_app();

    // Arrange: the L world exists before the app starts (the store's
    // own L-suite precedent — generation is setup, never measured).
    let generated = build_set_l(temp.path());
    assert_eq!(
        generated.assignment_count, 100_000,
        "the fixture is §12.2's set L (the generator's own suite pins \
         the deeper shape)"
    );

    // Act: the whole synchronous sequence, timed.
    let started = Instant::now();
    startup_sequence(&app, temp.path());
    let elapsed = started.elapsed();
    eprintln!("the set-L startup sequence took {elapsed:?}");

    // Assert 1 — the structure: the tail has returned while the first
    // dirty pass is still running (spawned, not awaited). Two
    // observables, one per await-class regression: the pass's own
    // effect (the first L verdict is not recorded yet — the pass
    // needs the whole snapshot read and 500 builds, seconds in this
    // profile, while the tail's synchronous remainder after the spawn
    // is microseconds; a tail that AWAITED the pass would return with
    // the verdict already recorded) and the service loop's counter
    // (zero passes completed — a tail that ran the service inline
    // would have counted one).
    let dirty_state = app.state::<crate::dirty_recalc::DirtyState>();
    assert!(
        !dirty_state.last_verdict(&generated.project_ids[0]),
        "the tail must return before the first dirty pass completes — \
         NFR-002's startup budget is the spawn, never the await (DB-022's \
         startup pass runs in the background)"
    );
    assert_eq!(
        dirty_state.pass_count(),
        0,
        "the dirty service must still be between passes when the tail \
         returns — its startup pass is spawned, not run inline"
    );

    // Assert 2 — the budget: the synchronous share fits the spec's
    // 2 s row (with the documented CI margin — see
    // [`WINDOW_READY_BUDGET`] for the reading; the dev-box
    // measurement and the release-gate rerun are the DECISIONS.md
    // entry's).
    let ci_bound = WINDOW_READY_BUDGET * CI_TOLERANCE;
    assert!(
        elapsed <= ci_bound,
        "§12.2: startup until the window is ready (set L) must be no \
         more than 2 s (the CI assert grants the documented tolerance \
         for the debug profile and the shared runner) — the backend's \
         synchronous share alone took {elapsed:?}"
    );

    // Assert 3 — the convergence: the spawned passes do run, and the
    // L world's honest startup verdict (every project has
    // assignments and no stored hash — DB-021's first clause, the
    // state D-121's probe measured) is what the session records.
    wait_for_dirty_passes(&app, 1);
    let converged = app.state::<crate::dirty_recalc::DirtyState>();
    assert!(
        converged.last_verdict(&generated.project_ids[0]),
        "the first L project must read dirty after the startup pass"
    );
    wait_for_availability_slot(&app);
    drop(app);
    drop(temp);
}

// ---------------------------------------------------------------------------
// The §12.2 memory row on set L (Linux: /proc)
// ---------------------------------------------------------------------------

/// §12.2's memory measurement on set L (task T-140), everything in
/// this module Linux-gated: the row is
///
/// ```text
/// | Memory (RSS, set L) | no more than 400 MB |
/// ```
///
/// and `/proc/<pid>/status` — Linux's own resident-set accounting —
/// is the only oracle this suite can read without a new dependency
/// (the `sysinfo`-style crates are an owner decision, D-035's
/// discipline; `/proc` is std `fs`). On macOS and Windows there is
/// no `/proc`: the row there is the release rerun's (D-135's
/// standing class), whose in-repo home is the manual smoke
/// checklist's §12.2 item.
///
/// # Why a child process, and two of them
///
/// The measurement must be of a process that carries the startup's
/// own memory and nothing else — but every test runs inside the test
/// binary's process, where `cargo test`'s parallel siblings and the
/// L fixture's generation remnants (glibc arenas keep freed pages
/// resident) would both inflate the reading without saying anything
/// about the app. So the parent test spawns two children, this same
/// test binary re-executed under its own `--exact` filter (the
/// `sync_kill_tests.rs` D-226 pattern, the spec env vars carried
/// process-local on the [`Command`]):
///
/// - the **world child** builds set L into a temp directory the
///   PARENT owns and exits — its allocations die with it;
/// - the **measure child** then starts VIRGIN over the pre-built
///   world — exactly the real app's position (the L database exists
///   on disk before the app launches) — arms the session shell
///   (`session_app_at`), runs the real sequence
///   ([`startup_sequence`], timing it), waits for the background
///   passes to settle, and reports its own `/proc` checkpoints on
///   stdout, which the parent reads through the pipe.
///
/// # The checkpoints
///
/// - **baseline** — the child's `VmRSS` after the shell is armed,
///   before the sequence: the harness + mock-runtime floor the
///   reading carries (the overstatement the memory assert absorbs).
/// - **ready** — right after the sequence returned: the backend's
///   synchronous share resident (the `Db::open` page walk, the
///   ERR-001 `VACUUM INTO` copy, the §6.3 invariant sweep — the
///   step costs D-239 measured at 916 ms debug on the dev box).
/// - **settled** — after the first dirty pass and the availability
///   recheck completed: the background services' footprint, the
///   dominant share being DB-022's first pass reading the WHOLE
///   ALG-001 snapshot (100 000 assignments, 3 000 variant texts)
///   into memory before the chunked verdicts (D-121's shape).
/// - **peak** — the child's `VmHWM` at the same moment: the
///   process's true high-water mark, the strongest number — what a
///   transient allocation between checkpoints cannot hide.
///
/// # What the parent asserts (and why each has teeth)
///
/// 1. Both children exit 0 — the world is §12.2's L (the world child
///    asserts the generator's own count), the measurement completed.
/// 2. The child's elapsed time fits the spec's 2 s row at the
///    documented CI margin — the SAME policy and the SAME constants
///    as the in-process budget test above (D-239's reading; not a
///    fork, a re-measurement in a fresh process).
/// 3. The child's settled and peak `VmRSS` fit the spec's 400 MiB
///    reading of the row — no extra margin: the assert pins the
///    spec's number on the measurable proxy (the reading carries a
///    ~60 MB harness + mock-runtime floor the real process does not;
///    the real runtime's own share is outside the measurement, so
///    the direction is unmeasured — a necessary, not sufficient,
///    check; see the module docs' honest boundaries).
/// 4. The anti-stub cross-check: the parent polls the child's
///    `/proc` while it runs and requires its own observed maximum to
///    sit in the neighborhood of the reported checkpoints — at least
///    the settled reading minus a small slack (a child inflating its
///    numbers fails it) and at most the reported peak plus the same
///    slack (a child understating them fails it — `VmRSS` never
///    exceeds `VmHWM` at any instant, so the upper bound is
///    structural). The numbers must describe the real live process,
///    not constants.
/// 5. The run's reality: exactly one day-carrying ERR-001 copy
///    exists in the shared root, its stamp bounded by the parent's
///    pre-spawn clock (the child's own day, or the next if UTC
///    midnight crossed between the spawn and the child's open —
///    the landed [`daily_copy_of`] rules spelled straddle-proof,
///    because the child stamps the copy with its own clock), the
///    root's database is L-sized (the [`L_DATABASE_FLOOR`] check),
///    and the copy's size is within half of that database's own —
///    the sequence's open+copy provably ran over the L world in
///    this root (the world child's count assert pins what the root
///    holds; a child that skipped the open, or measured some other
///    root, cannot fake either half).
///
/// The parent then prints one machine-readable line (the
/// `EUNOMIA-STARTUP-REPORT` prefix) carrying every reading — the
/// interface `scripts/measure-startup.sh` parses; the script is the
/// task card's "script", the repeatable measurement harness that
/// records both §12.2 numbers (this row and the startup row) on the
/// L set.
#[cfg(target_os = "linux")]
mod memory_row {
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{
        CI_TOLERANCE, WINDOW_READY_BUDGET, backups_dir_of, build_set_l, session_app_at,
        startup_sequence, wait_for_availability_slot, wait_for_dirty_passes,
    };

    /// §12.2's memory budget: "Memory (RSS, set L): no more than
    /// 400 MB" — read as 400 MiB (409 600 kB), the unit `/proc`
    /// reports (kB is kibibytes there); the MiB reading is the
    /// documented one (4.8 % above the strict decimal MB), recorded
    /// here so the two spellings cannot drift apart silently. No CI
    /// margin: the assert pins the spec's number on the measurable
    /// proxy (the reading carries a ~60 MB harness + mock-runtime
    /// floor the real process does not — the real runtime's own
    /// share is outside the measurement, so the direction is
    /// unmeasured and the whole-app verdict is the release rerun's,
    /// see the module docs' honest boundaries); a margin here would
    /// only blunt the assert without measuring anything more.
    const MEMORY_BUDGET_KB: u64 = 400 * 1024;

    /// The bound on the parent's wait for the WORLD child — the L
    /// generation, which has no internal deadline of its own. The
    /// dev box builds L in ~25 s under load; a slow shared runner
    /// needs a few times that, so the bound sits an order of
    /// magnitude above and still ends a hung child (a deadlock in
    /// the fixture path) in minutes instead of the CI job's whole
    /// timeout — the same finite-waiter posture as
    /// [`super::EVENT_DEADLINE`], scaled to the child's real cost.
    const WORLD_DEADLINE: Duration = Duration::from_secs(300);

    /// The bound on the parent's wait for the MEASURE child. The
    /// child's own waits are internally bounded (`EVENT_DEADLINE`,
    /// 20 s each), so its legitimate total is well under a minute
    /// even on a loaded runner; the parent's bound catches the hang
    /// class the child cannot catch itself (a deadlock inside the
    /// production path the child drives).
    const MEASURE_DEADLINE: Duration = Duration::from_secs(120);

    /// The floor the reality check holds the shared root's database
    /// to: the generator's L database is ~38 MB (D-239's fixture
    /// measurement) and an empty database tens of kB — 30 MiB sits
    /// an order of magnitude above the one and below the other, so
    /// the floor separates the L world from anything smaller without
    /// pinning the generator's exact output size (the world child's
    /// own count assert pins the deeper shape).
    const L_DATABASE_FLOOR: u64 = 30 * 1024 * 1024;

    /// The slack of both anti-stub neighborhood bounds. Upper side:
    /// the parent's observed `VmRSS` may exceed the child's reported
    /// `VmHWM` only by what the process can grow between the child's
    /// last read and its exit (the report printing and the harness
    /// teardown; structurally `VmRSS` never exceeds `VmHWM` at any
    /// instant, so the slack only absorbs post-report growth). Lower
    /// side: the post-pass state the child reads as settled persists
    /// for seconds, so the parent's 100 ms poll cadence cannot miss
    /// it — the slack only absorbs the checkpoint-to-poll gap and the
    /// run-to-run allocator variance (the observed gap on the dev box
    /// was ~2.5 MB). 16 MiB is an order of magnitude above both while
    /// still catching the constants a stubbed report could print.
    const CONSISTENCY_SLACK_KB: u64 = 16 * 1024;

    /// How often the parent polls the measurement child's
    /// `/proc/<pid>/status` while it runs — often enough that the
    /// multi-second settled phase is sampled many times over, cheap
    /// enough to be noise.
    const POLL_CADENCE: Duration = Duration::from_millis(100);

    /// The prefix of the report line both halves speak — the parent
    /// prints it after its asserts (the interface the script greps),
    /// the child prints it on stdout (the interface the parent
    /// parses). One spelling, two pipes.
    const REPORT_PREFIX: &str = "EUNOMIA-STARTUP-REPORT";

    /// The process-local role switch (the D-226 pattern: the vars
    /// ride the spawned `Command`, never the parent's own
    /// environment, so parallel tests in one process cannot see each
    /// other's children).
    const ROLE_VAR: &str = "EUNOMIA_STARTUP_MEASUREMENT_ROLE";
    /// The shared data root path the vars carry.
    const ROOT_VAR: &str = "EUNOMIA_STARTUP_MEASUREMENT_ROOT";
    /// The world-builder role's value.
    const ROLE_WORLD: &str = "world";
    /// The measurement role's value.
    const ROLE_MEASURE: &str = "measure";

    /// This test's own libtest name, derived from `module_path!` (the
    /// D-226 `self_test_path` shape, re-spelled here so this suite
    /// does not reach into a sibling module's private helpers): the
    /// children re-execute the binary under `--exact <name>`, so a
    /// renamed test fails loudly at the parent's report parse — the
    /// child would run zero tests and exit without a report line.
    fn self_test_path() -> String {
        let module = module_path!();
        let rest = module
            .split_once("::")
            .expect("the module path has a crate segment")
            .1;
        format!("{rest}::startup_rss_on_set_l_fits_the_memory_budget")
    }

    /// Reads one kilobyte-valued field of `/proc/<pid>/status`
    /// (`VmRSS`, `VmHWM`) — the kernel's own accounting of the
    /// process's resident set, the oracle this module cannot
    /// fabricate. Answers `None` when the file or the field is not
    /// there (a vanished process between polls, a non-Linux kernel):
    /// the callers decide what absence means.
    fn proc_status_field_kb(pid: u32, field: &str) -> Option<u64> {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let prefix = format!("{field}:");
        for line in status.lines() {
            let Some(rest) = line.strip_prefix(&prefix) else {
                continue;
            };
            // The fields look like `VmRSS:\t  61024 kB` — the value is
            // the first whitespace-separated token, the unit (kB,
            // kibibytes) is /proc's documented spelling.
            let value = rest.split_whitespace().next()?;
            return value.parse::<u64>().ok();
        }
        None
    }

    /// Reads one `key=value` number off the report line — the parse
    /// panics (naming the key and the whole line) when the field is
    /// missing or not a number, because a report that cannot be
    /// parsed is a broken measurement, not a skipped one.
    fn report_field(line: &str, key: &str) -> u64 {
        let prefix = format!("{key}=");
        for token in line.split_whitespace() {
            if let Some(value) = token.strip_prefix(&prefix) {
                return value
                    .parse()
                    .unwrap_or_else(|_| panic!("the report's {key} must be a number: {line}"));
            }
        }
        panic!("the report line must carry {key}: {line}");
    }

    /// Spawns one measurement half: this test binary re-executed
    /// under its own `--exact` filter (the D-226 pattern), carrying
    /// the role and the shared data root as process-local env vars.
    /// `--nocapture` is load-bearing for the measure child: libtest
    /// captures the test's stdout unless it is disabled, and the
    /// parent reads the child's report through the pipe. `--quiet`
    /// halves the harness's own lines (the report prefix is what the
    /// parent parses; the rest is diagnostics).
    /// Waits for a spawned child to exit within `deadline`, killing
    /// it on expiry: a hung child fails the test in minutes with the
    /// elapsed time named, instead of hanging the whole test binary
    /// until the CI job's timeout — the finite-waiter posture of
    /// [`super::EVENT_DEADLINE`], applied at the process level. The
    /// kill happens before the panic so no orphan outlives the
    /// failing parent.
    fn wait_within(child: &mut Child, deadline: Duration, who: &str) -> std::process::ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = child
                .try_wait()
                .unwrap_or_else(|error| panic!("polling {who} must work: {error}"))
            {
                return status;
            }
            if started.elapsed() > deadline {
                let _ = child.kill();
                panic!(
                    "{who} did not exit within {deadline:?} — a hang, not \
                     a slow run (killed at {:?}; the child's inherited \
                     stderr carries whatever it printed before the hang)",
                    started.elapsed()
                );
            }
            std::thread::sleep(POLL_CADENCE);
        }
    }

    fn spawn_role_child(role: &str, root: &Path, stdout: Stdio) -> Child {
        let exe = std::env::current_exe().expect("the test binary's own path must resolve");
        Command::new(exe)
            .arg("--exact")
            .arg(self_test_path())
            .arg("--quiet")
            .arg("--nocapture")
            .stdout(stdout)
            // stderr stays inherited — a child's panic text is the
            // useful failure diagnostic, and it rides stderr.
            .env(ROLE_VAR, role)
            .env(ROOT_VAR, root)
            .spawn()
            .expect("the startup measurement child must spawn")
    }

    /// The world child's half: build set L into the parent-owned root
    /// and return — the test passes in the child, the process exits,
    /// and every allocation of the generation dies with it (the
    /// measurement child starts with none of it in its RSS).
    fn build_the_l_world(root: &Path) {
        crate::data_dirs::ensure_data_layout(root)
            .expect("creating the data-directory layout must work");
        let generated = build_set_l(root);
        assert_eq!(
            generated.assignment_count, 100_000,
            "the world child must build §12.2's set L (the generator's \
             own suite pins the deeper shape)"
        );
    }

    /// The measure child's half: the virgin process over the
    /// pre-built world — the real app's position. Arms the shell,
    /// checkpoints its own `/proc`, runs the real sequence (timed),
    /// waits for the background passes to settle, checkpoints again,
    /// and prints the report line the parent parses. The convergence
    /// waits are the landed helpers: the pass-count wait proves the
    /// spawned services really ran (their work is in the readings),
    /// the availability slot wait releases the probe before the root
    /// can drop under it in the parent.
    fn run_the_startup_measurement(root: &Path) {
        let app = session_app_at(root);
        let baseline_kb = proc_status_field_kb(std::process::id(), "VmRSS")
            .expect("the measurement child must be able to read its own /proc status");
        let started = Instant::now();
        startup_sequence(&app, root);
        let elapsed = started.elapsed();
        let ready_kb = proc_status_field_kb(std::process::id(), "VmRSS")
            .expect("the measurement child must be able to read its own /proc status");
        wait_for_dirty_passes(&app, 1);
        wait_for_availability_slot(&app);
        let settled_kb = proc_status_field_kb(std::process::id(), "VmRSS")
            .expect("the measurement child must be able to read its own /proc status");
        let peak_kb = proc_status_field_kb(std::process::id(), "VmHWM")
            .expect("the measurement child must be able to read its own /proc status");
        eprintln!(
            "the set-L startup memory measurement: share {elapsed:?}, VmRSS baseline \
             {baseline_kb} kB, ready {ready_kb} kB, settled {settled_kb} kB, peak (VmHWM) \
             {peak_kb} kB"
        );
        println!(
            "{REPORT_PREFIX} elapsed_ms={} baseline_kb={baseline_kb} ready_kb={ready_kb} \
             settled_kb={settled_kb} peak_kb={peak_kb}",
            elapsed.as_millis()
        );
        drop(app);
    }

    #[test]
    fn startup_rss_on_set_l_fits_the_memory_budget() {
        // The child halves run when the role var is present (the
        // spawned `Command` set it); the parent half — everything
        // below — is what CI and the script run.
        if let Ok(role) = std::env::var(ROLE_VAR) {
            let root = PathBuf::from(
                std::env::var(ROOT_VAR).expect("the child spec carries the data root"),
            );
            match role.as_str() {
                ROLE_WORLD => build_the_l_world(&root),
                ROLE_MEASURE => run_the_startup_measurement(&root),
                other => panic!("unknown startup measurement role: {other:?}"),
            }
            return;
        }

        // The shared world, owned by THIS process so it outlives both
        // children and dies only after the asserts have read it.
        let temp = tempfile::tempdir().expect("failed to create a temp directory");
        let root = temp.path().to_path_buf();

        // The world child: set L, built out-of-process, its memory
        // never part of the measurement. The wait is bounded (see
        // [`WORLD_DEADLINE`]) — the generation has no internal
        // deadline of its own.
        let mut world = spawn_role_child(ROLE_WORLD, &root, Stdio::null());
        let world_status = wait_within(&mut world, WORLD_DEADLINE, "the world child");
        assert!(
            world_status.success(),
            "the world child must build set L and exit cleanly ({world_status}), \
             its stderr carries the generation diagnostics"
        );

        // The parent's clock, captured BEFORE the measure child
        // spawns: the child's ERR-001 copy is stamped by the child's
        // OWN now (at its open, seconds after this capture), so the
        // copy's day is this day or the next — the bound the reality
        // check below enforces instead of re-reading a later clock
        // (a UTC-midnight crossing between the capture and the
        // child's open names the next day, and reading the parent's
        // now AFTER the run would then refuse a perfectly valid
        // copy — the straddle class D-239's round-1 P3-3 closed for
        // the in-process batteries, closed here at the process
        // level by bounding the child's stamp from the pre-spawn
        // capture).
        let day0 = jiff::Timestamp::now();

        // The measurement child: a virgin process over that world.
        // The stdout pipe carries its report; the reader thread
        // drains it while the main thread polls the child's /proc
        // (the report is a few lines — the pipe cannot fill).
        let mut measure = spawn_role_child(ROLE_MEASURE, &root, Stdio::piped());
        let pid = measure.id();
        let mut report_pipe = measure
            .stdout
            .take()
            .expect("the measure child's stdout was piped");
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            report_pipe
                .read_to_string(&mut text)
                .expect("reading the measure child's report must work");
            text
        });

        // The parent's own /proc watch of the child — the
        // independent reading the anti-stub cross-check compares
        // against. The loop ends when the child exits, and is
        // bounded by [`MEASURE_DEADLINE`]: the child's own waits are
        // internally deadlined (`EVENT_DEADLINE`), so a legitimate
        // run cannot approach the bound — what it catches is the
        // hang class the child cannot catch itself (a deadlock
        // inside the production path it drives), killed and named
        // with the elapsed time and the last observed state. Polls
        // after the child's exit read nothing (the /proc entry
        // vanishes once the process is reaped), which the Option
        // absorbs.
        let mut max_seen_kb = 0u64;
        let mut polls = 0u64;
        let measure_started = Instant::now();
        let status = loop {
            if let Some(status) = measure
                .try_wait()
                .expect("polling the measure child must work")
            {
                break status;
            }
            if measure_started.elapsed() > MEASURE_DEADLINE {
                let _ = measure.kill();
                panic!(
                    "the measure child did not exit within {MEASURE_DEADLINE:?} \
                     — a hang inside the startup path it drives, not a slow \
                     run (killed at {:?}; {polls} /proc polls, max VmRSS \
                     observed {max_seen_kb} kB; the child's inherited stderr \
                     carries whatever it printed before the hang)",
                    measure_started.elapsed()
                );
            }
            if let Some(kb) = proc_status_field_kb(pid, "VmRSS") {
                max_seen_kb = max_seen_kb.max(kb);
                polls += 1;
            }
            std::thread::sleep(POLL_CADENCE);
        };
        assert!(
            status.success(),
            "the measure child must complete its run and exit cleanly ({status})"
        );
        let child_report = reader.join().expect("the report reader thread must finish");
        let report_line = child_report
            .lines()
            .find(|line| line.starts_with(REPORT_PREFIX))
            .unwrap_or_else(|| {
                panic!(
                    "the measure child must print its {REPORT_PREFIX} line on stdout; \
                     got: {child_report:?}"
                )
            });

        // Assert 1 — the timing, at the landed policy: the spec's 2 s
        // row with the documented CI margin (D-239's reading — the
        // constants are the budget test's own, this is the same
        // sequence re-measured in a fresh process, not a forked
        // policy). A synchronous step growing past ~3 s of work goes
        // red here exactly as it does above.
        let elapsed_ms = report_field(report_line, "elapsed_ms");
        assert!(
            Duration::from_millis(elapsed_ms) <= WINDOW_READY_BUDGET * CI_TOLERANCE,
            "§12.2: startup until the window is ready (set L) must be no more \
             than 2 s (the CI assert grants the documented tolerance for the \
             debug profile and the shared runner) — the fresh-process \
             re-measurement of the backend's synchronous share took \
             {elapsed_ms} ms"
        );

        // Assert 2 — the memory row, the spec's number on the
        // measurable proxy (the 400 MiB reading, see
        // [`MEMORY_BUDGET_KB`]): the reading carries the harness +
        // mock-runtime floor, the real runtime's share is outside
        // the measurement (a necessary, not sufficient, check — the
        // whole-app verdict is the release rerun's), and the peak
        // (VmHWM) closes the transient-allocation hole the settled
        // checkpoint alone would leave.
        let settled_kb = report_field(report_line, "settled_kb");
        let peak_kb = report_field(report_line, "peak_kb");
        assert!(
            settled_kb <= MEMORY_BUDGET_KB,
            "§12.2: Memory (RSS, set L) must be no more than 400 MB — the \
             measurement process's settled VmRSS is {settled_kb} kB (the \
             proxy reading: the harness + mock-runtime floor is inside it, \
             the real runtime's share is outside — the whole-app verdict \
             is the release rerun's)"
        );
        assert!(
            peak_kb <= MEMORY_BUDGET_KB,
            "§12.2: Memory (RSS, set L) must be no more than 400 MB — the \
             measurement process's peak (VmHWM) is {peak_kb} kB (the peak \
             is the strongest reading: no transient allocation between \
             checkpoints can hide from it)"
        );

        // Assert 3 — the anti-stub cross-check: the numbers must
        // describe the real live process. The parent's own /proc
        // polls of the child bound the reported readings from both
        // sides — its observed maximum must sit in the neighborhood
        // of the reported checkpoints: at least the settled reading
        // minus the slack (the post-pass state the child reads as
        // settled persists for seconds, so the parent's 100 ms
        // cadence cannot miss it — a report inflating the numbers
        // fails this side) and at most the reported peak plus the
        // same slack (VmRSS ≤ VmHWM at every instant, so a real
        // process cannot exceed an honest peak — a report
        // understating them fails that side).
        assert!(
            polls > 0,
            "the parent must have observed the measure child's /proc status \
             at least once — a child that exits before the first poll is a \
             broken measurement, not a fast one"
        );
        assert!(
            max_seen_kb + CONSISTENCY_SLACK_KB >= settled_kb,
            "the child's reported settled VmRSS ({settled_kb} kB) must be a \
             reading of the real process: the parent's own /proc polls never \
             saw it above {max_seen_kb} kB — the report overstates what the \
             kernel accounted"
        );
        assert!(
            max_seen_kb <= peak_kb + CONSISTENCY_SLACK_KB,
            "the child's reported peak ({peak_kb} kB) must be a reading of \
             the real process: the parent's own /proc polls saw {max_seen_kb} \
             kB resident — the report understates what the kernel accounted"
        );

        // Assert 4 — the run's reality: the sequence really ran over
        // the L world in THIS root. The copy is found by a scan of
        // the shared root's db-backups (the landed
        // [`daily_copy_of`] rules — the daily-* name shape, the
        // at-most-one copy — spelled here with the day bounded from
        // the PRE-SPAWN capture instead of a post-run now: the
        // child stamps the copy with its own clock, so a
        // UTC-midnight crossing between the spawn and the open
        // legitimately names the next day, and a post-run now would
        // refuse it). The size checks then establish what the
        // measurement ran over — the root's database is L-sized
        // (the floor: the generator's L database is ~38 MB per
        // D-239, an empty database tens of kB — the floor sits
        // orders of magnitude above the one and just below the
        // other; the world child's own count assert is what pins
        // the deeper L shape) and the copy is of it (VACUUM INTO of
        // a freshly generated, churn-free database preserves its
        // size to well within half).
        let day0_stamp = day0.strftime("%Y%m%d").to_string();
        let day1_stamp = (day0 + jiff::SignedDuration::from_hours(24))
            .strftime("%Y%m%d")
            .to_string();
        let mut copies = Vec::new();
        for entry in std::fs::read_dir(backups_dir_of(&root))
            .expect("the layout guarantees the db-backups directory is readable")
        {
            let name = entry
                .expect("reading a db-backups entry must work")
                .file_name()
                .into_string()
                .expect("the copy names are ASCII");
            if name.starts_with("daily-") && name.ends_with(".sqlite3") {
                let stamp = &name["daily-".len()..name.len() - ".sqlite3".len()];
                assert!(
                    stamp == day0_stamp || stamp == day1_stamp,
                    "the day's ERR-001 copy must be named for the day the                      sequence ran ({day0_stamp}, or {day1_stamp} if UTC midnight                      crossed between the spawn and the child's open): found                      {name}"
                );
                copies.push(backups_dir_of(&root).join(name));
            }
        }
        assert_eq!(
            copies.len(),
            1,
            "the measure child's startup must have written exactly one              day-carrying ERR-001 copy (ERR-001's same-day rule)"
        );
        let copy = copies
            .pop()
            .expect("the length assert guarantees exactly one copy");
        let copy_len = std::fs::metadata(&copy)
            .expect("the day's ERR-001 copy must be readable")
            .len();
        let db_len = std::fs::metadata(crate::data_dirs::db_file_path(&root))
            .expect("the set-L database must be readable")
            .len();
        assert!(
            db_len >= L_DATABASE_FLOOR,
            "the shared root's database must be the L world the world child \
             built: {db_len} bytes against the {L_DATABASE_FLOOR}-byte floor \
             (the generator's L database is ~38 MB, D-239; an empty database \
             is tens of kB)"
        );
        assert!(
            copy_len * 2 >= db_len,
            "the day's ERR-001 copy must be a copy of the set-L database: \
             {copy_len} bytes copied against the {db_len}-byte database — \
             the measurement provably ran over the L world in this root"
        );

        // The report — the script's interface: every reading of the
        // measurement, one line, grep-parsable.
        let baseline_kb = report_field(report_line, "baseline_kb");
        let ready_kb = report_field(report_line, "ready_kb");
        eprintln!(
            "{REPORT_PREFIX} elapsed_ms={elapsed_ms} baseline_kb={baseline_kb} \
             ready_kb={ready_kb} settled_kb={settled_kb} peak_kb={peak_kb} \
             parent_max_poll_kb={max_seen_kb} parent_polls={polls}"
        );
    }
}
