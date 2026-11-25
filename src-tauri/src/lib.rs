//! EunomiaSync Tauri application entry point.
//!
//! Commands are registered through `tauri-specta`, which generates the
//! typed TypeScript bindings in `src/bindings.ts` on debug builds. The
//! generated file is committed so the frontend can be type-checked
//! without a Rust toolchain.

use specta_typescript::Typescript;
use tauri::{AppHandle, Manager, Runtime};
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

// The orphaned project-backup report (T-332, the owner ruling of
// 2026-11-24): the read-only listing of `backups/` against the
// `backup_records` rows, run at the end of every successful
// `Db::open`. Never deletes — the debris is reported and the user
// decides (D-190).
pub(crate) mod backup_orphans;

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
// behind `dialog_pick_folder`, and the validator the path-consuming
// commands call at their boundary.
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
// take-once flag the sync pipeline (T-112+) acquires at its
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
// The target-file watch service (spec §5.7, SYS-007…009, OUT-014,
// §7.10's `state://file-changed` event; T-102): the debounced watch
// over the target files of active, available projects, the SYS-008
// re-read-and-compare verdicts, and the startup/availability/after-
// sync triggers wired below. Display-only: a watch event never
// writes a file.
pub(crate) mod file_watch;

// The scan commands and the `scan://progress` event (spec §7.10
// Projects module `scan_start`/`scan_cancel`/`scan_commit`, §9.6,
// PRJ-002, SYS-003/NFR-015, ALG-053, ERR-002; T-076, D-153; T-077,
// D-179): the command layer over the eunomia-fs walker and
// classifier — the SEC-003-validated root, the per-scan
// settings/registry snapshot, the blocking-pool walk with the
// ≥100 ms-throttled progress events, the managed cancel slot, and the
// found/candidate-set answer; the scan itself persists nothing. The
// module's one WRITER is `scan_commit` (T-077): the checked
// repositories of a completed scan's answer become project rows in
// one transaction through the new-project rules `commands.rs` owns,
// admitted only through the session's discovered set (the SEC-003
// rule D-100 reserved, fixed in D-179).
pub(crate) mod scan;

// The five Library-module variant commands (spec §7.10, T-329; the
// batch absorbing T-087…T-091, D-189): `variant_create`,
// `variant_update`, `variant_set_main`, `variant_delete`,
// `variant_restore` — the command layer over the `eunomia-store`
// library repository, the §6.9 library validators, the §9.8 version
// recorder and the `variant.*` journal members. One `Db::run`
// transaction per command (the write, the version row and the
// journal entry commit or roll back together); no SYS-005 gate (§5.4
// allows library edits during a sync). Its own module beside
// `scan.rs` — the §7.10 command-group shape.
pub(crate) mod variants;

// The four Library-structure commands (spec §7.10, T-330; the batch
// absorbing T-092…T-095): `heading_create`, `heading_update`,
// `heading_delete`, `items_move` — core-driven hierarchy changes over
// the library repository, one transaction and one journal entry per
// real operation, with no heading revisions and no SYS-005 gate.
pub(crate) mod headings;

// The three Assignments-module commands (spec §7.10, T-331): variant
// toggles, one-transaction bulk row/column writes, and D-125's per-project
// custom-text editor target. No command touches project files.
pub(crate) mod assignments;

// The History module's revision reads (spec §7.10, T-331):
// `revisions_list` and `revision_get` over the store's version repository.
pub(crate) mod revisions;

// The Backups module's project-copy listing (spec §7.10, BKP-004,
// T-125): `backups_list` — one project's whole `backup_records`
// history, every §6.2 CHECK status (§6.4 keeps the history as
// status), newest first, through the store's
// per-project read on the one `Db::run` transaction executor.
pub(crate) mod backups;

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
        // `markdown_preview` (T-056), the Library module's
        // `library_get` (T-083, D-187), its instruction
        // lifecycle `instruction_create`/`instruction_update`/
        // `instruction_hide`/`instruction_restore` (T-328) and its
        // variant commands `variant_create`/`variant_update`/
        // `variant_set_main`/`variant_delete`/`variant_restore`
        // (T-329, the batch absorbing T-087…T-091, D-189), the
        // Library-structure commands `heading_create`/
        // `heading_update`/`heading_delete`/`items_move` (T-330,
        // the batch absorbing T-092…T-095, D-191), the Assignments
        // module's `assignment_set`/`assignment_set_custom`/
        // `assignments_bulk` and the History module's
        // `revisions_list`/`revision_get` (T-331, D-192), the
        // Projects module's `projects_list` (T-072), the System
        // module's `dialog_pick_folder` (T-073), the Projects
        // module's `project_add_manual` (T-074), the Projects
        // module's `exclusions_list`/`exclusion_remove` (T-078), the
        // Projects module's `projects_reorder` (T-080), the
        // Projects module's `scan_start`/`scan_cancel`/`scan_commit`
        // (T-076/T-077), the Sync module's `sync_run` (T-118,
        // D-138) and the Backups module's `backups_list` (T-125,
        // D-193 — the project-settings "Backups" tab's read). Every
        // later command task lists its own §7.10 names here.
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
            commands::library_get::<tauri::Wry>,
            commands::instruction_create::<tauri::Wry>,
            commands::instruction_hide::<tauri::Wry>,
            commands::instruction_restore::<tauri::Wry>,
            commands::instruction_update::<tauri::Wry>,
            commands::markdown_preview,
            commands::project_add_manual::<tauri::Wry>,
            commands::projects_list::<tauri::Wry>,
            commands::projects_reorder::<tauri::Wry>,
            scan::scan_cancel::<tauri::Wry>,
            scan::scan_commit::<tauri::Wry>,
            scan::scan_start::<tauri::Wry>,
            commands::settings_get::<tauri::Wry>,
            commands::settings_set::<tauri::Wry>,
            commands::sync_run::<tauri::Wry>,
            variants::variant_create::<tauri::Wry>,
            variants::variant_delete::<tauri::Wry>,
            variants::variant_restore::<tauri::Wry>,
            variants::variant_set_main::<tauri::Wry>,
            variants::variant_update::<tauri::Wry>,
            headings::heading_create::<tauri::Wry>,
            headings::heading_delete::<tauri::Wry>,
            headings::heading_update::<tauri::Wry>,
            headings::items_move::<tauri::Wry>,
            assignments::assignment_set::<tauri::Wry>,
            assignments::assignment_set_custom::<tauri::Wry>,
            assignments::assignments_bulk::<tauri::Wry>,
            revisions::revision_get::<tauri::Wry>,
            revisions::revisions_list::<tauri::Wry>,
            backups::backups_list::<tauri::Wry>,
        ])
        // The registered §7.10 events: `state://project-availability`
        // (T-079, D-09/DB-032), `state://dirty` (T-101,
        // DB-021/DB-022), `state://file-changed` (T-102, §5.7
        // SYS-007…009) and `scan://progress` (T-076, SYS-003/ALG-053)
        // — the events the builder carries, emitted by
        // `availability::recheck_and_emit` at DB-032's three check
        // points, by `dirty_recalc::recalc_and_emit` at DB-022's
        // three recalculation points, by `file_watch`'s service
        // at §5.7's watch points, and by the scan command's walk
        // callback through its throttled seam. The wire names are
        // the spec's literal `state://…`/`scan://…` spellings
        // (pinned by the manual `Event` implementations in
        // `availability.rs`, `dirty_recalc.rs`, `file_watch.rs` and
        // `scan.rs`); the generated TypeScript accessors are
        // their camelCase forms (`stateProjectAvailability`,
        // `stateDirty`, `stateFileChanged`, `scanProgress`). Every
        // later event task (sync://progress, …) lists its own §7.10
        // name here.
        .events(collect_events![
            availability::ProjectAvailabilityEvent,
            dirty_recalc::DirtyEvent,
            file_watch::FileChangedEvent,
            scan::ScanProgressEvent,
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
    // unreachable from the UI. `capabilities/default.json` carries one
    // conscious grant, the event-subscription pair
    // `core:event:allow-listen`/`-unlisten` (T-246/D-116); application
    // commands are not capability-gated, §7.10 events cross the IPC
    // boundary as Rust-emitted notifications the webview may subscribe
    // to, and only application commands (`dialog_pick_folder`,
    // `app_open_data_folder`, later `update_*`) cross it as calls.
    // `config_tests.rs` pins the exact plugin set and the registration
    // order below.
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
    // `app_open_data_folder` app command (T-024); the webview has
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
            // taken atomically by the sync pipeline's start
            // (§9.4 phase 0 step 1), released by the guard's RAII drop
            // at its finish (phase 4 step 5), and read by the
            // project-changing commands' SYS-005 gate. Managed here
            // for the same reason as the allowlist — infallible,
            // needed in every mode: the gate runs before the
            // `managed_db` honest-recovery answer, so the flag state
            // must exist even in recovery mode, where no sync can run
            // (no database) and the flag simply stays down.
            app.manage(sync_running::SyncRunning::new());

            // The live run's published cancel token (§7.10's future
            // `sync_cancel`, §9.4's one cancel point; T-258, D-138's
            // recorded follow-up): `sync_run` publishes the token it
            // drives the pipeline with for the run's lifetime and
            // clears it on every exit (the RAII lease inside
            // `sync_run_with`). Managed beside the flag for the same
            // reason — infallible, needed in every mode: the future
            // cancel command must FIND the state, and in recovery
            // mode the slot simply stays empty (a sync refuses at
            // phase 0, so there is never a live run to cancel).
            app.manage(commands::SyncCancelSlot::new());

            // The live scan's published cancel token (§7.10's
            // `scan_cancel`, T-076): `scan_start` publishes the token
            // it drives the walk with for the scan's lifetime and
            // clears it on every exit (the RAII lease inside
            // `scan_start_with`) — the D-147 slot shape, one step (the
            // slot IS the guard). Managed beside the sync slot for
            // the same reason — infallible, needed in every mode: the
            // cancel command must FIND the state, and in recovery
            // mode the slot simply stays empty (a scan refuses at the
            // settings read — no database).
            app.manage(scan::ScanCancelSlot::new());

            // The session's discovered set (§7.10's `scan_commit`,
            // T-077/D-179): every COMPLETED scan publishes the keys of
            // the repositories it reported here, and the commit admits
            // only those paths — SEC-003's rule for a path the UI
            // sends back from a scan answer (D-100 reserved it). An
            // in-memory set, never persisted; the commit's transaction
            // is the only writer. Managed beside the cancel slot for
            // the same reason — infallible, needed in every mode: the
            // commit and the completed scan must FIND the state, and in
            // recovery mode it simply stays empty (a scan refuses at
            // the settings read — no database — before it could
            // publish, and the commit refuses at the database).
            app.manage(scan::ScanFindings::new());

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
            // unused until the recovery arms it). The bus's
            // receiving half is parked in `StateServices` (below)
            // for the shared tail — the pair travels together
            // because the channel is the state's own coalescing
            // seam.
            let (dirty_state, dirty_triggers) = dirty_recalc::DirtyState::new();
            app.manage(dirty_state);

            // The file-watch session cache (§5.7, T-102): the last
            // externally-changed/target-deleted verdicts per project
            // and the command bus's sending half. Managed here for
            // the same reason as the dirty cache — infallible, needed
            // in every mode (the availability listener and the pump
            // thread clone the sending half through the managed
            // state, and in recovery mode it exists unused until the
            // recovery arms it). The bus's receiving half is parked
            // with the dirty one.
            let (file_watch_state, file_watch_commands) = file_watch::FileWatchState::new();
            app.manage(file_watch_state);
            // The parked start of the state services (T-322): both
            // receiving halves wait here as managed state until a
            // database is published — on the healthy path a few steps
            // below, in recovery mode until `db_recreate`/
            // `db_restore_backup` reopens one. Managed with the
            // states that own the buses, before anything that can
            // fail: the tail reads it, and a hook returning early must
            // not drop it.
            app.manage(StateServices::parked(dirty_triggers, file_watch_commands));

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
            let database = match db::Db::open(&data_dir) {
                Ok(database) => database,
                Err(error @ db::DbInitError::Corrupt { .. }) => {
                    // The typed verdict exists (the match arm owns
                    // it); log it through the facade and continue
                    // without the database. `app_error()` is `Some`
                    // for this shape by construction (D-045).
                    if let Some(app_error) = error.app_error() {
                        logging::db_corrupt(&app_error);
                    }
                    // Recovery mode: no `Db` is managed, the shared
                    // tail below is skipped with it (there is no
                    // database to sweep, copy, publish or arm), and
                    // the window opens into the recovery UI. The
                    // command that recovers runs that tail itself
                    // over the reopened database — the parked service
                    // start above is exactly what it arms (T-322).
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
            // Everything after a successful open is the shared tail —
            // `start_database_session` (below): the §9.4 sweep, the
            // DB-022 commit seam, the ERR-001 daily copy, the
            // publication, and the state services over the published
            // database. The recovery commands run the same function
            // over the database they reopen (T-322), so the two open
            // paths cannot drift apart again.
            start_database_session(app.handle(), database);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// The session's state services — the dirty recalculation, the
/// target-file watch, the availability recheck and the daily
/// retention task — and their one-shot start.
///
/// The two bus-driven services (dirty, watch) are started from the
/// RECEIVING halves of the buses their managed states own
/// ([`dirty_recalc::DirtyState`], [`file_watch::FileWatchState`]).
/// Those states are managed before anything that can fail, in every
/// mode, so the halves must outlive the `setup` hook's recovery-mode
/// return: they are parked here, as managed state, until a database
/// is published for them — on the healthy path moments later, in
/// recovery mode until `db_recreate`/`db_restore_backup` reopens one
/// (T-322: before it, the halves died with the hook, and a recovered
/// session ran without its services until a restart).
///
/// The take is the session's "armed exactly once" fact:
/// [`start_database_session`] spends the pair in the call that
/// publishes the session's database — and only in that call — and
/// arms the services over it; a later call finds a database already
/// managed (tauri keeps the first of a type) and stops before the
/// take. The shape is tauri's own recommendation — a managed value
/// cannot be safely replaced (`Manager::manage` keeps the first of a
/// type and answers `false`; `unmanage` is unsafe, and its docs name
/// `Mutex<Option<_>>` + `Option::take` as the supported way; tauri
/// 2.12.6, `src/state.rs` `StateManager::set` and `src/lib.rs`
/// `Manager::unmanage`).
///
/// The lock guards only the swap-out: a short synchronous critical
/// section with no `.await` to cross (both callers run the tail
/// synchronously — the hook before the event loop serves a command,
/// the recovery command under its own `RECOVERY_OPERATION` gate).
pub(crate) struct StateServices {
    /// The parked receiving halves of the dirty-trigger bus and the
    /// watch command bus — `None` once the session is armed.
    parked: std::sync::Mutex<
        Option<(
            tauri::async_runtime::Receiver<dirty_recalc::Trigger>,
            tauri::async_runtime::Receiver<file_watch::Command>,
        )>,
    >,
}

impl StateServices {
    /// Parks the two receiving halves for the shared tail — called
    /// where the buses are created, before anything that can fail.
    pub(crate) fn parked(
        dirty_triggers: tauri::async_runtime::Receiver<dirty_recalc::Trigger>,
        file_watch_commands: tauri::async_runtime::Receiver<file_watch::Command>,
    ) -> Self {
        Self {
            parked: std::sync::Mutex::new(Some((dirty_triggers, file_watch_commands))),
        }
    }

    /// Takes the parked halves: `Some` on the session's first — and
    /// only — start, `None` once the services are armed.
    fn take_parked(
        &self,
    ) -> Option<(
        tauri::async_runtime::Receiver<dirty_recalc::Trigger>,
        tauri::async_runtime::Receiver<file_watch::Command>,
    )> {
        self.parked
            .lock()
            .expect("the parked service start is locked only inside this short synchronous section")
            .take()
    }
}

/// Brings a freshly opened database into the session — the one tail
/// every open path runs after a successful [`db::Db::open`]: the
/// `setup` hook for a healthy launch, and the recovery commands for
/// the database `db_recreate`/`db_restore_backup` reopened (T-322;
/// the same function, so neither path can lag the other again).
///
/// The order is load-bearing and pinned by the `lib.rs` source scans
/// (`db_tests.rs`, `ipc_tests.rs`):
///
/// 1. **The §9.4 crash-recovery sweep** ("Recovery after a crash",
///    T-258): every `running` run row a dead process left is marked
///    `interrupted` — synchronously, before the event loop can serve
///    a command. On the startup path this is the moment every
///    `running` row is a corpse by definition (no command of this
///    process holds the flag yet), which a deferred pass could not
///    promise (a spawned sweep could race this process's own first
///    sync and mark a LIVE run); on the recovery path no run can be
///    live either — no database was managed until this command. The
///    per-project rows and the project stamps stay exactly as the
///    crash left them (§9.4: "the project status already reflects
///    what was written"; T-F3), and the hashes' recalculation is the
///    DB-022 startup pass below (T-101). Deliberately BEFORE the
///    commit hook: the marking's row change would otherwise drop a
///    dirty trigger for a pass that reads projects, not runs. A
///    refusal degrades like the housekeeping steps (logged through
///    the facade, the session continues — D-046/D-054): the corpse
///    rows stay `running`, the next start retries the idempotent
///    sweep. Zero corpses (the normal case) logs nothing.
/// 2. **The DB-022 commit seam** (T-101): every transaction that
///    commits with row changes drops one trigger permit onto the
///    dirty-recalc bus, which debounces it 150 ms and recalculates.
///    Installed on the `mut` handle before the database is managed
///    and the first operation can run — `set_commit_hook`'s
///    `&mut self` is the proof nothing is in flight while the hook
///    appears.
/// 3. **The ERR-001 daily backup** (spec §10): after the database is
///    open and migrated, one `VACUUM INTO` copy per UTC day
///    (`db-backups/daily-YYYYMMDD.sqlite3`), the newest 7 kept —
///    idempotent within a day, decided by the file name. A fresh
///    first launch gets its copy too ("at every startup"): a cheap
///    copy of the empty database is an honest baseline for the day.
///    Degrade, never abort (DECISIONS.md D-046): the daily copy is
///    extra reliability, not a precondition — a failure is logged
///    through the facade and the session continues (unlike a failed
///    pre-migration copy, D-038). An existing same-day copy gets no
///    line: it is the idempotent no-op, and a line per restart would
///    only repeat itself.
/// 4. **The publication** — `app.manage(database)`: from here every
///    database command finds the handle.
/// 5. **The state services** over the published database, in an order
///    of their own (each step's reason is at the step).
///
/// Infallible by contract: nothing here can fail the open that
/// succeeded — every refusal degrades through the facade. One-shot
/// per session: the callers' own guards make a second call
/// unreachable (the hook runs before any command; the recovery
/// command takes its operation gate first and refuses under it while
/// a database runs), and the tail holds that line on its own — a
/// call that finds a database already managed stops there, dropping
/// the handle it was handed unpublished and arming nothing, because
/// the parked start is spent only by the call that publishes.
///
/// Blocking by content (SQL, `VACUUM INTO`, the watcher's start):
/// the hook runs it on the main thread before the event loop, the
/// recovery command inside its `spawn_blocking` closure (SYS-001).
///
/// # Panics
///
/// Panics if [`StateServices`], [`dirty_recalc::DirtyState`],
/// [`file_watch::FileWatchState`] or
/// [`availability::AvailabilityState`] is not managed — impossible
/// in production: the `setup` hook manages all four before the open
/// can fail into recovery mode, and a test harness that forgets one
/// is a broken fixture, not a state to run on. Also panics if the
/// parked start is already spent at a successful publication — only
/// this function spends it, right after publishing, so that is a
/// broken composition of the module, not a data condition.
pub(crate) fn start_database_session<R: Runtime>(app: &AppHandle<R>, mut database: db::Db) {
    match database.mark_interrupted_runs(&eunomia_store::values::now_utc()) {
        Ok(0) => {}
        Ok(count) => logging::sync_runs_interrupted(count),
        Err(error) => logging::sync_runs_interrupt_failed(error.code.as_str()),
    }
    database.set_commit_hook(dirty_recalc::commit_hook(app.clone()));
    match database.ensure_daily_backup(jiff::Timestamp::now()) {
        Ok(eunomia_store::storage::DailyBackupOutcome::Written(path)) => {
            logging::db_backup_daily(&path);
        }
        Ok(eunomia_store::storage::DailyBackupOutcome::AlreadyPresent(_)) => {}
        Err(error) => logging::db_backup_daily_failed(&error),
    }
    // The retention task's handle is cloned AFTER the hook, the
    // shape the hook always had: the hook is a per-clone field of `Db`
    // (`Db::set_commit_hook`), so a clone taken earlier would run the
    // task's writes outside the commit seam. Nothing observable rides
    // on it today — the prunes touch the journal and the runs, not the
    // build's inputs, so the pass they trigger diffs empty — but every
    // writer of the session's database goes through the same seam.
    let retention_db = database.clone();
    let published = app.manage(database);
    if !published {
        // A database is already managed — unreachable through both
        // callers: the hook runs once, and the recovery command
        // refuses while a database runs, under the operation gate it
        // takes first, so no recovery can slip between that refusal's
        // check and this publication. tauri kept the FIRST handle of
        // the type; this one is dropped here, unpublished, and the
        // services the first call armed stay the session's.
        return;
    }
    // A successful publication is the session's first, and only this
    // function takes the parked start — right here — so the halves
    // are there. (Taken AFTER the publication, never before: a start
    // is spent only by the call that publishes a database and arms
    // over it.)
    let (dirty_triggers, file_watch_commands) = app.state::<StateServices>().take_parked().expect(
        "the parked service start is spent only by the call that published the session's database",
    );
    // DB-022's startup recalculation point (T-101): the service task
    // over the trigger bus, spawned — not awaited — so the set-L
    // build pass cannot hold the launch (SYS-001; NFR-002's startup
    // budget). The spawn sends the startup trigger itself, before
    // the retention task's first cleanup can occupy the bus's one
    // permit with a debounced Commit (either way exactly one pass
    // runs — the coalescing). The pass emits `state://dirty` only
    // for projects found dirty against the clean baseline (OUT-013's
    // startup warning; `dirty_recalc.rs` module docs); a failure
    // records nothing and waits for the next trigger (at-least-once).
    dirty_recalc::spawn_service(app.clone(), dirty_triggers);
    // §5.7's watch service (T-102): subscribe to the availability
    // event FIRST — the listener must exist before the startup
    // recheck below can emit, or the session's first verdicts would
    // miss it — then spawn the service with the SYS-007 debounce. A
    // watcher backend that fails to start degrades: no watching this
    // session, the launch unaffected (SYS-010's no-crash posture; the
    // facade warning line is logged in the arm below, D-155).
    file_watch::listen_availability(app);
    if let Err(init) =
        file_watch::spawn_service(app.clone(), file_watch_commands, file_watch::DEBOUNCE)
    {
        // The degrade is total but no longer silent: the bus's
        // receiving half was dropped with the refusal, later
        // `try_send`s fail harmlessly, and the app runs without
        // external-change detection until restart — the one-line warn
        // the logging follow-up owed (D-137's follow-up 1, D-155).
        logging::file_watch_init_failed(&init);
    }
    // The LOG-006 retention task (spec §7.7, §6.4): journal and
    // sync-run cleanup by the settings limits — the startup run fires
    // immediately inside the spawned task, then once a day.
    // Deliberately *after* the publication and after the ERR-001
    // daily copy above: the copy is the day's recovery point of the
    // state as the app found it, housekeeping shrinks it afterwards,
    // and commands keep answering through the whole pass — the
    // executor serializes them on the one connection behind the
    // prune's single transaction, a bounded queue rather than a
    // hang. A failure degrades exactly like the daily copy's — logged
    // through the facade, the launch unaffected (D-054).
    retention::spawn_daily_cleanup(retention_db);

    // DB-032's startup check point (D-09, T-079): one availability
    // recheck of every active project, spawned — not awaited — so
    // four metadata probes per project over possibly dead drives
    // cannot hold the launch (SYS-001; NFR-002's startup budget).
    // Deliberately the LAST step: the event is mounted (the hook's
    // first line), the database is managed (the recheck reads the
    // project list through its executor), the diff cache exists, and
    // the watch listener above is registered. The pass emits
    // `state://project-availability` only when some project is found
    // unavailable (the baseline rule — `availability.rs` module
    // docs); a failure is logged through the facade, never fatal.
    availability::spawn_recheck(app.clone());
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

// T-126's cross-phase suite: the whole §9.4 pipeline on temporary Git
// repositories in all three schemes. Registered here, beside the phase
// suites, since T-341; it lived under a `#[path]` attribute inside
// `sync_phase4_tests` only because this block was a sibling task's
// frozen zone when the suite landed (DECISIONS.md D-183).
#[cfg(test)]
mod sync_pipeline_tests;

#[cfg(test)]
mod scan_tests;

#[cfg(test)]
mod library_tests;

#[cfg(test)]
mod variants_tests;

#[cfg(test)]
mod headings_tests;

#[cfg(test)]
mod assignments_tests;

#[cfg(test)]
mod revisions_tests;

#[cfg(test)]
mod backups_tests;

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
