//! EunomiaSync Tauri application entry point.
//!
//! Commands are registered through `tauri-specta`, which generates the
//! typed TypeScript bindings in `src/bindings.ts` on debug builds. The
//! generated file is committed so the frontend can be type-checked
//! without a Rust toolchain.

use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::{Builder, collect_commands, collect_events};

// The IPC error contract (spec §7.10, §10): AppError and the MVP
// error-code registry, exported to the frontend through the specta
// builder below.
pub(crate) mod errors;

// The machine-level `config.toml` (spec §5.8, SYS-011…013) is needed on
// every platform, exactly like the app itself.
pub(crate) mod config;

// The data directory layout (spec §5.5) is needed on every platform,
// exactly like the app itself.
pub(crate) mod data_dirs;

// The application database state and the one-transaction command
// executor (SYS-001/SYS-002, NFR-008; DECISIONS.md D-037).
pub(crate) mod db;

// The LOG-006 retention task (spec §7.7, §6.4 "Log and runs"):
// journal/sync-run cleanup at startup and once a day, over the
// `Db::run` executor with the limits read from the settings table in
// the same transaction (DECISIONS.md D-054).
pub(crate) mod retention;

// The recovery-screen backend (spec §7.10 command notes, §10
// E_DB_CORRUPT row; T-024): the file mechanics behind
// `db_recreate`/`db_restore_backup` and the `db-backups/` listing.
pub(crate) mod recovery;

// The IPC commands (spec §7.10): the recovery-screen group of the
// System module (D-009) and the History module's `activity_list`
// (T-027) — the first registered commands since `greet` was removed
// (D-037); every later command task extends this module.
pub(crate) mod commands;

// The SEC-003 session allowlist and the reusable UI-path check (spec
// §5.6, T-073): the "picked in this session" half of the trust rule
// behind `dialog_pick_folder`, and the validator the upcoming
// path-consuming commands call at their boundary.
pub(crate) mod session_paths;

// The Markdown preview renderer (spec §7.10 Library module
// `markdown_preview`, §5.3, SEC-005, D-15; T-056): pulldown-cmark
// parse → raw-HTML escape → ammonia sanitize, editor-facing only —
// the §5.3 "not used when building files" note holds because the
// build path never calls it (its sole caller is the command).
pub(crate) mod markdown;

// The project-availability service (spec §7.10's
// `state://project-availability` event; D-09/DB-032's three check
// points; T-079): the D-084 re-probe of every active project, the
// session diff cache, and the startup/focus triggers wired below.
pub(crate) mod availability;

// The global `sync_running` flag (spec §5.4 SYS-004/MAIN-030, §9.4
// phase 0/phase 4, §10's `E_SYNC_RUNNING` row; T-105): the atomic
// take-once flag the future sync pipeline (T-112+) acquires at its
// start and releases by RAII at its finish, plus the status read the
// project-changing commands' SYS-005 gate checks.
pub(crate) mod sync_running;

// The dirty-flag recalculation service (spec §7.10's `state://dirty`
// event; DB-021/DB-022's three recalculation points; T-101): the
// DB-021 verdict over the T-100 snapshot, the session diff cache, the
// coalescing debounce bus behind `Db::run`'s commit seam, and the
// startup/sync triggers wired below.
pub(crate) mod dirty_recalc;

// The sync pipeline's phase 0 (spec §9.4 phase 0, §5.4 SYS-004/
// SYS-005, ALG-001; T-112): the flag-guarded start — one read
// transaction snapshots the library, the build assembles every active
// project's managed content and DB-020 hash from that snapshot alone,
// and one write transaction records the `running` run with its
// `not_started` per-project rows. Phases 1–4 (T-113+) pick up the
// prepared run; the flag lease travels in the outcome (RAII, D-123).
pub(crate) mod sync_phase0;

// The sync pipeline's phase 1 — preflight (spec §9.4 phase 1, §10's
// skip rows, D-09/D-24/D-25, OUT-008; T-113): the read-only
// per-project gate over phase 0's built content — availability,
// resolved target path, write rights, the existing file (scheme B:
// UTF-8 and marker checks), the ALG-033 `unchanged` verdict, the
// D-25 confirmation list, the stale-tmp sweep — and the closing
// free-space rule that aborts the whole run with `E_DISK_FULL`
// before anything is written. No target file is touched; the
// verdicts land in `sync_run_projects` (T-118's command consumes
// the outcome).
pub(crate) mod sync_phase1;

// The sync pipeline's phase 2 — backups (spec §9.4 phase 2, §7.6
// BKP-001…011 through eunomia-fs's verified copy, §10's
// E_BACKUP_FAILED row, OUT-006/OUT-019; T-115): for every `ready`
// project whose target file exists, one copy into the §5.5 backups
// tree (size + SHA-256 checked, BKP-010) and one `backup_records`
// row through the new store writer; any error aborts the run before
// a single target byte moves — this run's copies deleted with
// `discarded` history rows, the per-project rows reset to
// `not_started` (except `skipped`), and the typed envelope carrying
// BKP-005's field list. After this phase succeeds the run cannot be
// canceled (§9.4 phase 2 step 3).
pub(crate) mod sync_phase2;

// The sync pipeline's phase 3 — the write (spec §9.4 phase 3, §9.5
// through eunomia-fs's atomic pipeline, §9.3's ALG-030/031/032
// assembly, §10's four write rows, DB-024/OUT-020; T-116): one
// `ready` project at a time, in order — the run's ONE creation
// marker (ALG-032, T-111's criterion) around phase 0's managed text,
// laid down atomically. Success lands `written` plus the project's
// `last_synced_at`/`last_synced_hash` in one short transaction; a
// refused write lands `failed` with its §10 code and the run takes
// the next project (the backup stays). Phase 4 (T-117) owns the
// final run verdict; the flag lease stays with the caller's
// Phase0Outcome (D-123).
pub(crate) mod sync_phase3;

// The sync pipeline's phase 4 — the finish (spec §9.4 phase 4,
// §7.6's BKP-003 through the store retention service, §7.7's run
// journal, MAIN-027; T-117): the `unchanged` stamps, the backup
// retention sweep, the counted success/partial/failed verdict with
// `finished_at` (MAIN-027's derived last-success time), the D-051
// journal entries — all in ONE transaction — and the after-commit
// `state://dirty` recalculation (D-121's seam). Retention and
// journal refusals are reported, never the verdict's; the flag lease
// stays with the caller's Phase0Outcome (its RAII drop is §9.4
// phase 4 step 5's reset, D-123/T-118's to own).
pub(crate) mod sync_phase4;

// File logging with daily rotation (spec §5.3, §7.7; SEC-008) — needed on
// every platform, exactly like the app itself.
pub(crate) mod logging;

// The single-instance handoff handler (SYS-006) is desktop-only, exactly
// like the plugin whose callback calls it below.
#[cfg(desktop)]
pub(crate) mod single_instance;

/// Constructs the tauri-specta builder with all commands and events
/// registered. Shared by the app entry point and the binding export test.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        // The registered §7.10 commands: the recovery-screen group of
        // the System module (D-009), the History module's
        // `activity_list` (T-027), the Settings module's
        // `settings_get`/`settings_set` (T-030), the System
        // module's `app_info` (T-031), the Library module's
        // `markdown_preview` (T-056), the Projects module's
        // `projects_list` (T-072), the System module's
        // `dialog_pick_folder` (T-073), the Projects module's
        // `project_add_manual` (T-074), the Projects module's
        // `exclusions_list`/`exclusion_remove` (T-078) and the
        // Projects module's `projects_reorder` (T-080). Every later
        // command task (library, …) lists its own §7.10 names here.
        // The commands are runtime-generic (the tauri-specta shape for
        // commands that take an `AppHandle`), so the registration pins
        // the concrete runtime of the shipped app; `markdown_preview`
        // takes no state at all, hence the bare form.
        .commands(collect_commands![
            commands::activity_list::<tauri::Wry>,
            commands::app_info::<tauri::Wry>,
            commands::app_open_data_folder::<tauri::Wry>,
            commands::db_backups_list::<tauri::Wry>,
            commands::db_restore_backup::<tauri::Wry>,
            commands::db_recreate::<tauri::Wry>,
            commands::dialog_pick_folder::<tauri::Wry>,
            commands::exclusion_remove::<tauri::Wry>,
            commands::exclusions_list::<tauri::Wry>,
            commands::markdown_preview,
            commands::project_add_manual::<tauri::Wry>,
            commands::projects_list::<tauri::Wry>,
            commands::projects_reorder::<tauri::Wry>,
            commands::settings_get::<tauri::Wry>,
            commands::settings_set::<tauri::Wry>,
        ])
        // The registered §7.10 events: `state://project-availability`
        // (T-079, D-09/DB-032) and `state://dirty` (T-101,
        // DB-021/DB-022) — the events the builder carries, emitted by
        // `availability::recheck_and_emit` at DB-032's three check
        // points and by `dirty_recalc::recalc_and_emit` at DB-022's
        // three recalculation points. The wire names are the spec's
        // literal `state://…` spellings (pinned by the manual `Event`
        // implementations in `availability.rs` and `dirty_recalc.rs`);
        // the generated TypeScript accessors are their camelCase
        // forms (`stateProjectAvailability`, `stateDirty`). Every
        // later event task (sync://progress, state://file-changed, …)
        // lists its own §7.10 name here.
        .events(collect_events![
            availability::ProjectAvailabilityEvent,
            dirty_recalc::DirtyEvent,
        ])
        // `AppError` (§7.10: every command returns `Result<T, AppError>`)
        // is registered directly instead of through a command
        // signature: the type must stay reachable even when a command
        // set change would otherwise drop it from the export, and
        // `typ` is tauri-specta's documented mechanism for types not
        // reachable from commands or events; the type (and its
        // `ErrorCode`/`ErrorDetails` dependencies) reaches
        // `src/bindings.ts` through it, pinned by `errors_tests.rs`.
        .typ::<errors::AppError>()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // Export the typed bindings only on debug builds; the generated
    // file is committed and linted as part of the frontend toolchain.
    #[cfg(debug_assertions)]
    builder
        .export(Typescript::default(), "../src/bindings.ts")
        .expect("Failed to export typescript bindings");

    // Tauri plugins (spec §5.3 table, SEC-001): all four are invoked from
    // Rust only — the webview gets neither their JavaScript packages nor
    // any `opener:`/`dialog:`/`updater:` permission, so their commands are
    // unreachable from the UI. `capabilities/default.json` stays
    // plugin-free and only application commands (`dialog_pick_folder`,
    // `app_open_data_folder`, later `update_*`) cross the
    // IPC boundary. `config_tests.rs` pins the exact plugin set and the
    // registration order below.
    let tauri_builder = tauri::Builder::default();

    // `single-instance` (SYS-006: a second launch must not start a second
    // process — the window of the first one is brought to the front) — the
    // plugin REQUIRES being registered before every other plugin, so it is
    // wired first. Desktop-only: the crate is compiled out on mobile, hence
    // the cfg. The callback hands off to the `single_instance::focus_main_window`
    // handler (that module documents the show/unminimize/set_focus order
    // and the missing-window case); the returned outcome is a test seam —
    // there is nothing to branch on here. `_argv`/`_cwd` are deliberately
    // unused: deep links are out of v1 (SEC-004) and no CLI behavior
    // consumes them yet. Diagnostics go through the logging facade
    // (`logging.rs`) into the tracing file log; argv/cwd are never
    // logged — they can carry user paths and the handoff needs none of
    // them. (The plugin registers before the `setup` hook below runs,
    // so a handoff arriving in that startup window is emitted before
    // the subscriber exists and is dropped — no crash, one lost
    // diagnostic line, accepted.)
    #[cfg(desktop)]
    let tauri_builder =
        tauri_builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            single_instance::focus_main_window(app);
        }));

    // `dialog` (SEC-001, SEC-003): the native folder picker runs in Rust
    // behind the `dialog_pick_folder` app command (T-073), so the
    // picked path is tracked per session and every other path stays
    // registered-project-only. The webview has no `dialog:` permission.
    let tauri_builder = tauri_builder.plugin(tauri_plugin_dialog::init());

    // `updater` (D-005/D-023, SEC-001: network belongs to Rust): update
    // check and install run in Rust behind the `update_check` /
    // `update_install` app commands (upcoming tasks); the webview never
    // contacts the release feed. The plugin refuses to initialize without
    // a `plugins.updater` config section (2.14.1 `Config` has a required
    // `pubkey` field), so `tauri.conf.json` carries a disabled stub
    // (`pubkey: ""`, no endpoints) until the release task sets the real
    // key and endpoints; with it, `updater()` fails fast with
    // `EmptyEndpoints` instead of the app failing to start.
    let tauri_builder = tauri_builder.plugin(tauri_plugin_updater::Builder::new().build());

    // `opener` (SEC-001): "open the data folder" runs in Rust behind the
    // `app_open_data_folder` app command (upcoming task); the webview has
    // no `opener:` permission and cannot open arbitrary URLs/paths.
    let tauri_builder = tauri_builder.plugin(tauri_plugin_opener::init());

    // DB-032's window check point (D-09, T-079): every window event
    // routes through the availability trigger — a focus gain
    // (`Focused(true)`, which is also how "the window opened" is
    // delivered) rechecks all active projects in a fire-and-forget
    // task behind the in-flight slot; everything else is ignored.
    // Routed through the `on_window_event` seam (not per-window
    // listeners) so every window of the app qualifies, present and
    // future, and the handler never blocks the event loop's thread
    // (SYS-001 — the probes run on the spawned task's blocking pool).
    let tauri_builder = tauri_builder.on_window_event(|window, event| {
        if let availability::WindowEventReaction::Recheck(_pass) =
            availability::on_window_event(window.app_handle(), event)
        {
            // The pass's join handle is deliberately dropped here:
            // production never waits on a focus-triggered recheck —
            // the task runs on (the handle detaches on drop, tokio's
            // semantics); only the IPC tests await it.
        }
    });

    tauri_builder
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            // Data directory (spec §5.5): resolve `app_local_data_dir()`
            // and create the layout (`db/`, `db-backups/`, `backups/`,
            // `logs/`) before any of the app's own code reads or writes
            // the directory — `config.toml` (SYS-011, the config task),
            // the database and the logs all live inside. (Tauri builds
            // the config-declared windows before this hook runs and
            // their WebView data sits under this root on Windows and
            // Linux, so the webview layer may create the root first —
            // irrelevant here: creation is idempotent.) Creation
            // converges to the full layout even after a partially
            // deleted directory. A failure aborts the launch: without
            // the layout there is no database, no logs and no config to
            // run with — and SYS-011 orders the layout *before* the
            // config and the config *before* logging, so no diagnostics
            // channel exists at this point; the error propagates out of
            // `setup`, and tauri then panics in the event-loop `Ready`
            // handler with `Failed to setup app: …`, whose message
            // carries the offending path and cause (DECISIONS.md
            // D-027).
            let data_dir = app.path().app_local_data_dir()?;
            data_dirs::ensure_data_layout(&data_dir)?;
            // The resolved root, published for the recovery commands
            // (T-024): they run exactly when the database step below
            // fails, so the one path fact they need must be managed
            // before it can fail — and being a plain value (not a
            // resolver call) is what keeps the IPC tests on temp
            // roots instead of the real user data directory.
            app.manage(data_dirs::DataRoot(data_dir.clone()));

            // The SEC-003 session allowlist (§5.6, T-073): the folders
            // the user picks in the system dialog this session. Also
            // managed before anything that could fail — the state is
            // infallible, and `dialog_pick_folder` must answer in any
            // mode (recovery included: no database is involved in
            // opening a picker).
            app.manage(session_paths::SessionPaths::new());

            // The global sync-running flag (SYS-004/MAIN-030, T-105):
            // taken atomically by the future sync pipeline's start
            // (§9.4 phase 0 step 1), released by the guard's RAII drop
            // at its finish (phase 4 step 5), and read by the
            // project-changing commands' SYS-005 gate. Managed here
            // for the same reason as the allowlist — infallible,
            // needed in every mode: the gate runs before the
            // `managed_db` honest-recovery answer, so the flag state
            // must exist even in recovery mode, where no sync can run
            // (no database) and the flag simply stays down.
            app.manage(sync_running::SyncRunning::new());

            // The availability session cache (D-09/DB-032, T-079):
            // the last D-084 verdicts per active project, plus the
            // in-flight slot that coalesces overlapping rechecks.
            // Managed here for the same reason as the allowlist —
            // infallible, needed in every mode (window events start
            // only after this hook, and in recovery mode the recheck
            // answers the honest `NoDatabase` error over it instead
            // of panicking on a missing lookup).
            app.manage(availability::AvailabilityState::new());

            // The dirty-flag session cache (DB-021/DB-022, T-101):
            // the last known dirty verdicts per active project, the
            // capacity-1 trigger bus behind the 150 ms debounce, and
            // the pass counter. Managed here for the same reason as
            // the availability cache — infallible, needed in every
            // mode (the commit hook reads it through the managed
            // state at fire time, and in recovery mode it exists
            // unused: no service is spawned, no hook is ever set).
            // The bus's receiving half waits in `dirty_triggers` for
            // the service spawn below — the pair travels together
            // because the channel is the state's own coalescing seam.
            let (dirty_state, dirty_triggers) = dirty_recalc::DirtyState::new();
            app.manage(dirty_state);

            // Machine-level config (spec §5.8, SYS-011…013): read or
            // create `config.toml` from the data directory — before
            // logging (SYS-011: the log level comes from here) and
            // before the database (both spec §5.8; the failure policy is
            // DECISIONS.md D-028, the seam shape D-029). A missing file
            // is created with defaults; unknown keys are ignored with a
            // warning; unparseable content — invalid UTF-8, broken TOML
            // or an invalid `log_level` — falls back to in-memory
            // defaults with a warning and the file is never rewritten
            // (SYS-012). Only an unreadable/unwritable file aborts the
            // launch, like the layout above (still pre-logging, so
            // still no diagnostics channel to degrade into). The config
            // is published as managed state first; the logging step
            // below reads it back through `app.state::<AppConfig>()`.
            let (config, warnings) = config::load_or_create(&data_dir)?;
            app.manage(config);
            let level = app.state::<config::AppConfig>().log_level;

            // File logging (spec §5.3, §5.5, §7.7): initialized after
            // the config (SYS-011) with the `log_level` it holds, at
            // `<data dir>/logs/` explicitly — never Tauri's
            // `app_log_dir()` (macOS would move the logs out of the
            // data directory; pinned in `config_tests.rs`). `init`
            // never aborts the launch: logging is diagnostics, so a
            // broken log file degrades to a stderr subscriber with one
            // `error` event (module docs in `logging.rs`). The returned
            // `Logger` owns the non-blocking writer's guard; keeping it
            // as managed state holds the writer thread for the whole
            // run (dropping it early would silently stop the file log —
            // tauri exits the process without dropping managed state,
            // so the flush-at-exit path belongs to the worker's
            // per-batch flushing, not to this value's destructor). The
            // config warnings and the startup line are emitted after
            // `init`, so they land in the log file.
            let logs_dir = data_dir.join(data_dirs::LOGS_SUBDIRECTORY);
            let logger = logging::init(&logs_dir, level);
            app.manage(logger);
            config::emit_warnings(&warnings);
            logging::startup(&data_dir, level);

            // The database (SYS-002): one SQLite connection for the
            // whole app, opened from the data directory (the §5.5
            // paths — `db/eunomiasync.sqlite3` and the `db-backups/`
            // the migration run's safety copies go to — are derived
            // inside `db::Db::open`) with the DB-001 pragma set and
            // migrated to the latest schema, then published as managed
            // state. Every database command goes through its
            // `run` executor — one operation, one transaction, off
            // the async runtime (SYS-001, NFR-008; DECISIONS.md
            // D-037). A database newer than this app (DB-004,
            // NFR-010) is rejected before anything touches the file
            // and hands back the typed `E_DB_TOO_NEW` `AppError`
            // (`DbInitError::app_error`), logged here through the
            // facade with its §10 code. A migration step that failed
            // and was rolled back (UPD-004) is another: the error
            // names the pre-migration copy, and the typed
            // `E_UPDATE_FAILED` form is logged through its own facade
            // event. A migration failure *without* a copy (the run
            // created the database) deliberately gets no facade line:
            // not the UPD-004 situation, it keeps the D-027
            // anonymous-abort treatment — the panic `Display` is its
            // report (D-043). These failures still ABORT the launch
            // (D-027/D-028): their §10 effects ("Update the app",
            // "install the previous version") end the session, and
            // their screens are the future update task's.
            //
            // A database damaged enough to fail `PRAGMA quick_check`
            // (spec §6.3, §10 `E_DB_CORRUPT`, T-F11's damaged-DB
            // half) is the deliberate exception since T-024: the
            // verdict is logged through the facade and the hook
            // returns `Ok` with **no** `Db` managed — recovery mode.
            // §10's effect for this code is a screen *instead of* the
            // app ("data directory path, the list of `db-backups/`
            // copies, 'Open data folder' and 'Start with empty
            // database' buttons"), which requires a live process: the
            // recovery commands (`db_backups_list`,
            // `db_restore_backup`, `db_recreate`,
            // `app_open_data_folder`) work in exactly this state over
            // the managed `DataRoot`, and the recreate/restore ones
            // reopen and manage the database themselves (§7.10: "…
            // and reopens it"). The frontend screen itself is the
            // recovery-UI task's; this hook's contract is: corrupt →
            // logged, alive, database-less.
            let mut database = match db::Db::open(&data_dir) {
                Ok(database) => database,
                Err(error @ db::DbInitError::Corrupt { .. }) => {
                    // The typed verdict exists (the match arm owns
                    // it); log it through the facade and continue
                    // without the database. `app_error()` is `Some`
                    // for this shape by construction (D-045).
                    if let Some(app_error) = error.app_error() {
                        logging::db_corrupt(&app_error);
                    }
                    // Recovery mode: no `Db` is managed, the daily
                    // step below is skipped with it (there is no
                    // database to copy), and the window opens into
                    // the recovery UI. The command that recovers
                    // performs this step's daily+manage tail itself.
                    return Ok(());
                }
                Err(error) => {
                    match (&error, error.app_error()) {
                        (db::DbInitError::TooNew { .. }, Some(app_error)) => {
                            logging::db_too_new(&app_error);
                        }
                        (db::DbInitError::Migration { .. }, Some(app_error)) => {
                            logging::db_migration_failed(&app_error);
                        }
                        _ => {}
                    }
                    return Err(error.into());
                }
            };
            // The DB-022 commit seam (T-101): every transaction that
            // commits with row changes drops one trigger permit onto
            // the dirty-recalc bus, which debounces it 150 ms and
            // recalculates. Installed on the `mut` handle before the
            // database is managed and the first operation can run —
            // `set_commit_hook`'s `&mut self` is the proof nothing is
            // in flight while the hook appears.
            database.set_commit_hook(dirty_recalc::commit_hook(app.handle().clone()));
            // The ERR-001 daily backup (spec §10): after the database
            // is open and migrated, one `VACUUM INTO` copy per UTC day
            // (`db-backups/daily-YYYYMMDD.sqlite3`), the newest 7 kept
            // — idempotent within a day, decided by the file name. A
            // fresh first launch gets its copy too ("at every
            // startup"): a cheap copy of the empty database is an
            // honest baseline for the day. Degrade, never abort
            // (DECISIONS.md D-046): the daily copy is extra
            // reliability, not a precondition — a failure is logged
            // through the facade and the launch continues (unlike a
            // pre-migration copy failure, D-038). An existing same-day
            // copy gets no line: it is the idempotent no-op, and a
            // line per restart would only repeat itself.
            match database.ensure_daily_backup(jiff::Timestamp::now()) {
                Ok(eunomia_store::storage::DailyBackupOutcome::Written(path)) => {
                    logging::db_backup_daily(&path);
                }
                Ok(eunomia_store::storage::DailyBackupOutcome::AlreadyPresent(_)) => {}
                Err(error) => logging::db_backup_daily_failed(&error),
            }
            // The LOG-006 retention task (spec §7.7, §6.4): journal and
            // sync-run cleanup by the settings limits — the startup run
            // fires immediately inside the spawned task (the database is
            // open and managed by now), then once a day. Deliberately
            // *after* `app.manage` and after the ERR-001 daily copy
            // above: the copy is the day's recovery point of the state
            // as the app found it, housekeeping shrinks it afterwards,
            // and commands keep answering through the whole pass — the
            // executor serializes them on the one connection behind the
            // prune's single transaction, a bounded queue rather than a
            // hang. A failure degrades exactly like the daily copy's —
            // logged through the facade, the launch unaffected (D-054).
            let retention_db = database.clone();
            app.manage(database);
            // DB-022's startup recalculation point (T-101): the
            // service task over the trigger bus, spawned — not
            // awaited — so the set-L build pass cannot hold the
            // launch (SYS-001; NFR-002's startup budget). The spawn
            // sends the startup trigger itself, before the retention
            // task's first cleanup can occupy the bus's one permit
            // with a debounced Commit (either way exactly one pass
            // runs — the coalescing). The pass emits `state://dirty`
            // only for projects found dirty against the clean
            // baseline (OUT-013's startup warning; `dirty_recalc.rs`
            // module docs); a failure records nothing and waits for
            // the next trigger (at-least-once).
            dirty_recalc::spawn_service(app.handle().clone(), dirty_triggers);
            retention::spawn_daily_cleanup(retention_db);

            // DB-032's startup check point (D-09, T-079): one
            // availability recheck of every active project, spawned —
            // not awaited — so four metadata probes per project over
            // possibly dead drives cannot hold the launch (SYS-001;
            // NFR-002's startup budget). Deliberately the LAST startup
            // step: the event is mounted (the hook's first line), the
            // database is managed (the recheck reads the project list
            // through its executor), and the diff cache above exists.
            // The pass emits `state://project-availability` only when
            // some project is found unavailable (the baseline rule —
            // `availability.rs` module docs); a failure is logged
            // through the facade, never fatal to the launch.
            availability::spawn_recheck(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod config_tests;

#[cfg(test)]
mod db_tests;

#[cfg(test)]
mod errors_tests;

#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod ipc_tests;

#[cfg(test)]
mod dirty_recalc_tests;

#[cfg(test)]
mod sync_phase0_tests;

#[cfg(test)]
mod sync_phase1_tests;

#[cfg(test)]
mod sync_phase2_tests;

#[cfg(test)]
mod sync_phase3_tests;

#[cfg(test)]
mod sync_phase4_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Regenerates `src/bindings.ts` so `cargo test` keeps the committed
    /// bindings in sync with the Rust command definitions. Run
    /// `cargo test -p eunomia-sync` after changing any command, then commit
    /// the updated file.
    ///
    /// The export is followed by an equality check: the file is read back
    /// after the export and compared with the content `include_str!`
    /// embedded at compile time (before this test rewrote the working-tree
    /// copy). A developer who changed the Rust contract but has not
    /// regenerated `src/bindings.ts` sees the test red on that first run —
    /// the same run performs the regeneration, so repeating it is green;
    /// CI, which compiles the committed file before any export, stays red
    /// until the regenerated file is committed. Without the check the test
    /// would silently rewrite the file and stay green (D-034; tightened in
    /// the T-013 review).
    #[test]
    fn export_bindings_are_up_to_date() {
        specta_builder()
            .export(Typescript::default(), "../src/bindings.ts")
            .expect("Failed to export typescript bindings");
        let on_disk = std::fs::read_to_string("../src/bindings.ts")
            .expect("the exported bindings file must be readable");
        // EOL-normalized on both sides: a Windows checkout translates
        // text files to CRLF (the runner's git runs autocrlf), so the
        // compile-time `include_str!` bytes carry CRLF while the
        // exporter writes LF — the staleness check is about the
        // bindings CONTENT, not the checkout's line endings.
        assert_eq!(
            on_disk.replace("\r\n", "\n"),
            include_str!("../../src/bindings.ts").replace("\r\n", "\n"),
            "src/bindings.ts is stale: the committed bindings differ from \
             what the registered commands and types generate — commit the \
             regenerated file"
        );
    }
}
