# Architecture

EunomiaSync keeps `AGENTS.md` files consistent across many projects. This
document describes how the codebase is laid out and how the pieces talk to
each other. Everything marked **planned** is not implemented yet.

## Overview

- **Backend:** Rust, organized as a Cargo workspace.
- **Desktop shell:** Tauri 2 (`src-tauri`).
- **Frontend:** Vue 3 + TypeScript, built with Vite, styled with Tailwind CSS
  v4, state managed with Pinia.
- **Storage:** SQLite via `rusqlite` with embedded SQL migrations.
- **Typed IPC:** `tauri-specta` generates TypeScript bindings from Rust
  commands; the generated `src/bindings.ts` is committed.

## Workspace layout

```text
crates/
  eunomia-core/                # Core library, no Tauri/SQLite/fs dependency
    src/assembly_b.rs          #   Scheme-B assembly: block spliced into the user's markers (ALG-031) (working)
    src/assignment.rs          #   The project ↔ instruction ↔ variant link (working)
    src/backup_name.rs         #   Backup path grammar: pure string half of §7.6/§5.5 (working)
    src/body.rs                #   ALG-020…022 project body assembly (working)
    src/color_pick.rs          #   New-project palette color: least-used, hash tie-break (D-29) (working)
    src/compare.rs             #   ALG-033 compare: file vs build, the unchanged verdict (working)
    src/content_build.rs       #   Chunked multi-project content pass over one snapshot (working)
    src/dirty.rs               #   DB-021 dirty predicate: build hash vs stored (working)
    src/extra_files.rs         #   Scheme-A extra-file reading lines (D-01/D-21) (working)
    src/gaps.rs                #   LIB-032 heading-level skips, facts for the UI warning (working)
    src/hash.rs                #   DB-020 build hash (SHA-256, D-064) (working)
    src/heading_range.rs       #   ALG-060 branch range of a heading, as index range (working)
    src/levels.rs              #   ALG-062 level ops: point change, branch shift ±1 (working)
    src/library.rs             #   Library items: instructions, headings, variants (working)
    src/library_validators.rs  #   §6.9 library field validators: names, variant text (working)
    src/marker.rs              #   Creation-marker line format (ALG-032, D-061) (working)
    src/marker_clock.rs        #   Once-per-run marker clock: instant + zone capture (D-104) (working)
    src/markers.rs             #   Scheme-B marker parser: exactly one D-19 pair (working)
    src/normalize.rs           #   ALG-002 variant text normalization (working)
    src/project.rs             #   Project write vocabulary: scheme, base name, extra files, palette color (working)
    src/project_validators.rs  #   §6.9 project field validators + DB-042 default label (working)
    src/reorder.rs             #   ALG-061 move: cut, insert, renumber positions from 1 (working)
    src/snapshot.rs            #   Immutable build snapshot, ALG-001 (working)
    src/target_file.rs         #   ALG-030 managed text seam + whole-file content for A/C (working)
    src/version_snapshot.rs    #   ALG-070 version snapshots of instructions/variants + SHA-256 (working)
    markdown                   #   Markdown processing (planned)
    diff                       #   Text diffing (planned)
    sync                       #   AGENTS.md sync engine (planned)
  eunomia-fs/                  # File-system layer (scan/preflight/backup/atomic/watcher), no Tauri dependency
    src/atomic.rs              #   Atomic writes with retries (working)
    src/backup.rs              #   Backup copies before writes (working)
    src/fault_injection.rs     #   Spec 15.1 failure points (test builds, fault-injection feature)
    src/free_space.rs          #   §9.4 preflight: free space on the data volume (working)
    src/paths.rs               #   Path normalization and validation (working)
    src/scan.rs                #   Repository tree scanning (working)
    src/scan_classify.rs       #   ALG-053 classification: path_key, New/Known/Archived/Excluded (working)
    src/temp_files.rs          #   Staging temp-file names + stale-file sweep (working)
    src/temp_repos.rs          #   Temp .git repo roots for benchmark fixtures (fixture tooling)
    src/watcher.rs             #   Target-file watching (spec 5.7)    (working)
    src/write_access.rs        #   §9.4 preflight: write-rights probe (tmp create, read-only) (working)
  eunomia-store/               # Database layer (SQLite), no Tauri dependency
    src/action_log.rs          #   Journal repository: LOG-001…005 + LOG-006 cleanup (working)
    src/backup_records.rs      #   Per-run backup records, the BKP-010 dedup lookup, the BKP-004 listing (working)
    src/backup_retention.rs    #   BKP-003 retention: newest N copies per pair (working)
    src/build_snapshot.rs      #   ALG-001 snapshot read: one read transaction (SYS-005) (working)
    src/data_sets.rs           #   Deterministic benchmark data sets M and L (working)
    src/library.rs             #   Library repository: items, variants, assignments (working)
    src/library_versions.rs    #   Revisions bookkeeping: ALG-070…072 + prune + history reads (working)
    src/projects.rs            #   Projects + exclusions repository (working)
    src/settings.rs            #   Typed settings repository, §6.5 keys (working)
    src/storage.rs             #   SQLite storage + migrations   (working)
    src/sync_runs.rs           #   sync_runs / sync_run_projects repositories (working)
    src/values.rs              #   UUIDv7 ids, ISO-8601 UTC times (working)
    migrations/                #   Embedded SQL migration files
benches/
  eunomia-load/           # §12.2 load benchmarks (criterion; local tool — CI runs its tests, never the benches)
    src/lib.rs            #   Fixtures (TempRepos + data_sets) + the reads + the parallel bench substrate + the assignment toggle + the composed §9.4 sync pass + the scan tree
    benches/              #   project_build, dirty_recalc, assignment_toggle, sync_pipeline, scan_walk (the five §12.2 budget rows)
src-tauri/
  src/availability.rs      # Availability service: re-probes + state://project-availability (working)
  src/assignments.rs       # assignment_set / assignments_bulk / assignment_set_custom (working)
  src/backup_orphans.rs    # Orphaned .bak report at Db::open (read-only, T-332) (working)
  src/backups.rs           # backups_list: the Backups tab read (BKP-004, T-125) (working)
  src/commands.rs          # IPC commands, the §7.10 registered set (working)
  src/config.rs            # config.toml load-or-create; broken file falls back to defaults (working)
  src/data_dirs.rs         # Data-directory layout + SEC-007 Unix modes (working)
  src/db.rs                # DB state + one-transaction executor   (working)
  src/dirty_recalc.rs      # Background dirty recalc, state://dirty events (working)
  src/errors.rs            # AppError + MVP error-code registry  (working)
  src/file_watch.rs        # Target-file watch service, state://file-changed (working)
  src/logging.rs           # File log: daily + 10 MB rotation, 5-file cap (working)
  src/markdown.rs          # Markdown preview renderer (pulldown-cmark; preview only) (working)
  src/project_archive.rs   # project_archive command: §6.4 row transition + PRJ-017 exclusion (working)
  src/project_update.rs    # project_update command: PRJ-008 field patch + project.changed journal (working)
  src/recovery.rs          # Recovery-screen file mechanics       (working)
  src/revisions.rs         # revisions_list / revision_get: History reads (working)
  src/retention.rs         # LOG-006 retention task: journal/runs prune daily (working)
  src/scan.rs              # scan_start/scan_cancel/scan_commit commands, scan://progress (working)
  src/session_paths.rs     # SEC-003 session allowlist + UI path check (working)
  src/single_instance.rs   # Single-instance handoff: restore + focus (SYS-006) (working)
  src/sync_phase0.rs       # Sync phase 0: flag, run record, built content (working)
  src/sync_phase1.rs       # Sync phase 1 preflight: ready/unchanged/skipped (working)
  src/sync_phase2.rs       # Sync phase 2: §7.6 backups + records (working)
  src/sync_phase3.rs       # Sync phase 3: the §9.5 atomic writes (working)
  src/sync_phase4.rs       # Sync phase 4 finish: stamps, retention, verdict, journal (working)
  src/sync_running.rs      # The global sync_running flag (SYS-004) (working)
  src/variants.rs          # variant_* commands: the Library save surface (working)
  src/headings.rs          # heading lifecycle + items_move: Library structure (working)
  src/lib.rs               # App builder, specta registration   (working)
src/                      # Frontend
  bindings.ts              # GENERATED by tauri-specta, committed
  ipc/                     # Typed IPC facade: only bindings importer, mock/real backend (D-003) (working)
  main.ts                  # Frontend entry: createApp + Pinia + i18n (working)
  i18n/                    # vue-i18n setup + the no-raw-ui-text lint rule (working)
  locales/en.json          # Message catalog, nested feature namespaces (working)
  date/                    # Date/time formatting helpers (UI-020) (working)
  assets/                  # Tailwind theme tokens + global styles (working)
  components/              # Reusable UI components               (working)
    AppErrorAlert.vue      # UI-011 AppError alert, copyable details (working)
  App.vue                  # Root view                           (working)
  views/, stores/          # Vue views / Pinia stores            (planned)
```

`eunomia-core` is deliberately independent of Tauri so that markdown,
diff, and sync logic can be tested without a UI layer (see
DECISIONS.md D-006); it is also free of database dependencies, so the
"core = pure logic, no I/O" rule holds at the dependency level. The
domain vocabulary now lives here (T-033, DECISIONS.md D-042): the
project write settings (scheme A/B/C, base name, extra files), the
library model (instructions, headings, variants), assignments, and the
immutable `BuildSnapshot` of spec ALG-001 — assembled by the store
layer in one read transaction, validated by core's
`BuildSnapshot::new`, and handed to the build as a plain value
(ordering carried by collection order; every assignment resolves inside
the value, so the builder is a total function). Variant texts ride the
snapshot raw and are normalized per ALG-002 before reaching a file
(`normalize::variant_text`, a pure function — CR/CRLF become LF, the
BOM and the block's leading empty lines and trailing whitespace are
dropped, everything inside the text is untouched; spec D-22, recorded
in DECISIONS.md D-044). The project body assembly of spec §9.2 lives in
`body::build_body` (ALG-020…022): a pure function over the snapshot and
a project id that walks the library order once, keeps the headings whose
section holds an assigned active instruction (a hidden instruction keeps
no heading, DB-013; an empty variant keeps it, D-13), normalizes each
selected variant's text, and joins the blocks with exactly one empty
line — the same body for schemes A, B and C, with no trailing line
break. SQLite storage lives in `eunomia-store` (see
DECISIONS.md D-010), which is free of Tauri dependencies for the same
reason. File-system work — repository scan, path normalization and
validation, backup copies, atomic writes — lives in `eunomia-fs` (see
DECISIONS.md D-019): the D-019 module plan is implemented today, from
scan and the §9.4 preflight seams to backup, atomic write and watcher,
with the spec-15.1 fault-injection seams feature-gated for tests.
`src-tauri` is the app shell: commands, managed state, specta wiring,
and the services that drive them (sync pipeline, watch, availability,
dirty recalc, retention).

## IPC contract

Rust commands are registered through `tauri-specta`:

1. A command in a `src-tauri/src` module (`src-tauri/src/commands.rs`,
   added back by the recovery task after D-037's removal of the
   template demo) is annotated with `#[tauri::command]` and
   `#[specta::specta]`. Commands that take an `AppHandle` are generic
   over `R: Runtime` — the documented tauri-specta shape — and the
   registration pins the concrete runtime (`::<tauri::Wry>`).
2. The command is added to `collect_commands![...]` in
   `src-tauri/src/lib.rs`.
   The registered set is forty-one commands — the §7.10 names that
   are implemented so far, matching the `mode.spec` surface list. It comprises the
   recovery-screen group of
   the System module (spec §7.10/D-009): `app_open_data_folder`,
   `db_backups_list`, `db_restore_backup`, `db_recreate` — plus the
   History module's `activity_list` (T-027): the "Activity" window's
   filtered, paginated journal query over the `Db::run` executor, with
   LOG-005's visibility rule (user entries by default, `diagnostic`
   only on the explicit `include_diagnostic` request) — the
   Settings module's `settings_get`/`settings_set` (T-030): the
   §6.5 settings map read/written atomically through the store's
   single validation path, each changed key journaled
   `settings.changed` inside the operation's own transaction — and
   the System module's `app_info` (T-031): the About window's fact
   sheet (version, identifier, author, license, repository, data
   directory, and whether this launch created the database from
   scratch), plus the NFR-023 Linux root-launch flag of T-032
   (`running_as_root`: whether the launch runs as root, answered from
   the effective uid through a pure `is_root` seam — `false` on every
   platform but Linux, where the spec scopes the warning), a
   non-database command that answers from compile-time constants and
   managed state in any mode, recovery included — the Library
   module's `markdown_preview` (T-056): the editor's live preview,
   Markdown rendered into sanitized HTML (SEC-005) with the D-15
   warnings computed in the same parse pass — a stateless,
   non-database command reading no managed state, hence the bare
   non-generic form in the registration — the Projects module's
   `projects_list` (T-072): the matrix columns read (an optional
   archived include) whose per-row availability and staleness
   verdicts double as the cold-start carrier for the events a fresh
   webview never received — the Projects module's
   `project_add_manual` (T-074): the SEC-003-gated hand add of a
   project (a path that neither canonicalizes onto a registered
   project nor onto a folder picked this session is refused before
   anything runs) whose post-add watch refresh is never-fatal
   aftermath — the Projects module's
   `project_archive` (T-108, `src-tauri/src/project_archive.rs`,
   DECISIONS.md D-194): the §6.4 archive — one id-addressed
   `Db::run` transaction carrying the row transition, the PRJ-017
   exclusion row for a scan-born project (converging over an
   existing one) and the single `project.archived` journal entry;
   assignments, the stored sync hash and the folder are untouched
   (DB-014), and the SYS-005 gate answers `E_SYNC_RUNNING` while a
   sync holds the flag — the Projects module's
   `project_update` (T-106, `src-tauri/src/project_update.rs`,
   DECISIONS.md D-201): the PRJ-008 settings save — an id-addressed
   patch over the four §6.9-validated editable fields (name, label,
   color, description; absent fields unchanged, a duplicate name
   saved) whose one `Db::run` transaction carries the store update and
   the single `project.changed` journal entry; the path and the origin
   are immutable by the command's own surface, an archived or unknown
   id is the stale-screen `E_VALIDATION` refusal, and the SYS-005 gate
   answers `E_SYNC_RUNNING` while a sync holds the flag — the Projects
   module's `exclusions_list`/`exclusion_remove` (T-078): the §6.4
   path-exclusion read in the repository's stable oldest-first order
   and the by-id removal journaled in its own transaction — the
   Projects module's `projects_reorder` (T-080): the drag reorder
   whose full-list validation refuses before the first write and
   which refuses with `E_SYNC_RUNNING` while a sync holds the flag
   (SYS-005) — the Projects module's
   `scan_start`/`scan_cancel`/`scan_commit` (T-076/T-077,
   `src-tauri/src/scan.rs`): the PRJ-002 scan that validates the
   root through SEC-003 and walks the tree on the blocking pool with
   the throttled `scan://progress` events, answering the found
   candidate set that persists nothing (a COMPLETED scan publishes
   its reported keys into the session's discovered set — the SEC-003
   rule the commit admits against, DECISIONS.md D-179), the token
   latch that cancels it — an idempotent no-op when no scan is live —
   and the commit that adds the checked repositories in ONE
   transaction through the new-project rules `commands.rs` shares
   with `project_add_manual` (typed outcomes for paths already
   registered; a commit over an excluded path lifts the exclusion and
   adds the project, D-179/D-196; any error rolls back the whole
   batch, and a failed post-commit probe answers the typed partial
   success `probe_failed` instead of an error) —
   the Library module's instruction lifecycle
   `instruction_create`/`instruction_update`/`instruction_hide`/
   `instruction_restore` (T-328, DECISIONS.md D-188): the create
   dialog's pair (one instruction, its `Main` variant, one `create`
   version per entity and one `instruction.created` journal entry)
   placed at the end of the picked section — `create_item` +
   `move_item` in one transaction, §6.9/DB-016 validation first
   (the marker line answers `E_CONTENT_RESERVED`, its own §10 row) —
   the rename with its `update` version and the verified same-name
   no-op, and the hide/restore pair that keeps variants and
   assignments (D-11/DB-013) recording one version and one journal
   entry per real transition, idempotent on re-runs; no SYS-005 gate
   — §5.4's own sentence allows library edits during a sync, the
   DB-022 commit hook recalculates the dirty set after every
   committed transaction —
   and the System module's `dialog_pick_folder` (T-073): the native
   folder picker invoked from Rust, its canonical picked path
   remembered in the SEC-003 session allowlist (`None` on cancel,
   nothing remembered) — and the Sync
   module's `sync_run` (T-118, DECISIONS.md D-138): the §9.4 pipeline
   over the finished phase modules — phase 0's flag-guarded start (a
   second start refuses with `E_SYNC_RUNNING`; the flag resets by
   RAII on every exit), DB-032's availability recheck (after the
   flag, before phase 1), phase 1's preflight, the D-25 empty-build
   gate (the run row terminal `aborted`, the answer
   `NeedsConfirmation` with the project ids, the UI's confirm
   re-call a full new run), the one cancel check point before phase 2
   (the live run's token is published through the managed
   `SyncCancelSlot` (T-258, D-147) for the future `sync_cancel`
   command; after phase 2 no stop exists), phases 2/3/4, the
   never-fatal after-sync refresh and finish reports, and the summary
   read back from the committed rows — emitting the §7.10
   `sync://progress` event throughout (T-119, SYS-003, D-202): the
   live "k of N" (terminal §9.4 verdicts over phase 0's project
   count) and §9.4's current phase word, one report before each of
   phases 1–4 begins (only when the run reaches the phase) plus one
   per write through the wrapped write seam, bounded at ten
   emissions per second by the `SyncRunSeams.progress_clock` seam
   (D-197's clock-injection pattern). A post-phase-0 `Err` whose run
   row is still `running` gets that row a terminal `aborted` status
   with the error's own §10 code, decided by the row's state inside
   one transaction (D-138's wrapper) — and the Library
   module's `library_get` (T-083, DECISIONS.md D-187): the §7.3
   instruction library's tree — instructions (name, variant count,
   dates, state, the variants inline with LIB-014's emptiness flag
   and DB-010's main flag, the assignment labels of the ACTIVE
   projects) and headings (level, the LIB-032 skip flag from the
   `eunomia_core::gaps` walk) — one flat `position`-ordered vector
   read in ONE `Db::run` transaction, the `include_hidden` flag
   widening it the `projects_list` way, and the answer carrying the
   D-14 `hidden_count` (the hidden instructions, counted in the same
   transaction, D-200) — and the Library module's
   variant commands `variant_create`/`variant_update`/
   `variant_set_main`/`variant_delete`/`variant_restore` (T-329, the
   batch absorbing T-087…T-091, `src-tauri/src/variants.rs`): the
   §6.9-validated save surface over the `eunomia-store` library
   repository — each command ONE `Db::run` transaction carrying the
   row write, the §9.8 version row (ALG-070's hash gate decides
   whether one is written; `variant_set_main` writes none — D-16's
   metadata ruling) and the `variant.*` journal entry together;
   `variant_delete` moves the deleted variant's assignments to the
   replacement (the instruction's main by default, LIB-023) inside
   the same transaction, refusing the last active variant (LIB-025)
   and the main without a named successor; `variant_create` copies
   the main's current text or starts empty (LIB-017…022); §5.4
   allows library edits during a sync, so none of the five takes the
   SYS-005 gate the project-changing commands do — and the Library
   structure commands `heading_create`/`heading_update`/
   `heading_delete`/`items_move` (T-330, the batch absorbing
   T-092…T-095, `src-tauri/src/headings.rs`): validated heading
   creation/rename, point-wise level change, whole-branch ±1 shift,
   physical delete that keeps instructions in order, and semantic
   item/branch moves. Each runs one `Db::run` transaction over the
   existing core ALG-060/061/062 algorithms and the store's whole-order
   persistence; a heading operation journals `heading.*`, a moved
   instruction journals `instruction.changed`, and no heading writes a
   revision. Automatic drop alignment validates every new level before
   any order or level write; self/descendant drops and levels outside
   2–6 are `E_VALIDATION` — and the Assignments module's
   `assignment_set`/`assignments_bulk`/`assignment_set_custom` (T-331,
   `src-tauri/src/assignments.rs`): a single toggle/select, one-transaction
   row/column products, and D-125's per-project custom-text target. A real
   write carries one `assignment.changed` entry with its changed count;
   existing pairs (including custom cells) are returned as already assigned
   and never replaced by bulk work. Custom text is validated by §6.9, lives
   on the assignment rather than a forked variant, and clearing it restores
   the active main. No assignment command touches a project file. The
   History module's `revisions_list`/`revision_get` (T-331,
   `src-tauri/src/revisions.rs`) read instruction+variant summaries
   newest-first without materializing snapshots, or one full revision by id,
   through the store's typed history repository — and the Backups
   module's `backups_list` (T-125, `src-tauri/src/backups.rs`,
   BKP-004): one project's whole `backup_records` history — date,
   file, size, sync run id and status, every §6.2 CHECK status (§6.4
   keeps the history as status), newest
   first (D-122's ordering) — through the store's per-project read
   (`backup_records::list_for_project`) on one `Db::run` read
   transaction; an unknown project id is the typed `E_VALIDATION`
   refusal, a known project without rows the honest empty list, and
   the no-database answer is the `E_DB_LOCKED`/`NoDatabase` refusal
   the other read commands share. Every further command name is fixed by §7.10
   and belongs to its own task.
3. On debug builds the specta `Builder` exports `src/bindings.ts`
   (`specta_typescript::Typescript`). The file is **committed** so the
   frontend type-checks and lints without a Rust toolchain; it is excluded
   from ESLint and Prettier because it is generated.
4. The frontend calls the generated `commands` — never a raw `invoke()`
   (see DECISIONS.md D-003).

**Rule: no raw `invoke()` calls in the frontend.** Always use the generated
`commands` (see DECISIONS.md D-003). Every command returns
`Result<T, AppError>`; database commands reach the one-transaction
executor (`db::Db::run`, see the Storage section), which is what makes
SYS-001/SYS-002 hold for every command alike — the deliberate
exceptions are the recovery group, which by design runs when no
database exists (see the Storage section's recovery paragraph and
D-049), and `app_info`, which has no database work at all (constants,
the process's effective-uid fact, and managed state only, any mode).

The error half of the contract — `AppError` (spec §7.10: every command
returns `Result<T, AppError>`) — lives in `src-tauri/src/errors.rs`: the
twenty-code MVP registry of the spec §10 table (`E_SYNC_RUNNING` …
`E_UPDATE_FAILED`; the two P1 codes of that table are deliberately
excluded until their tasks land), the stable default `message_key` per
code, interpolation `params`, and structured `details` carrying the OS
error code/kind/message. It reaches `src/bindings.ts` through the specta
builder's `typ` registration (kept beside the command registration so a
command-set change cannot drop it silently); the recovery commands
return it directly — the first registered fallible commands, with the
§7.10 rejection envelope pinned by `ipc_tests.rs`. `errors_tests.rs`
pins the registration and the exported literal set. DECISIONS.md D-034
records the field design; the SQLite→registry classifier lives in
`db.rs` (D-037), and the store-to-wire mapping for the sync-run
tables is that module's one `sync_runs_to_app_error` (T-264 — the
seven per-module copies consolidated; the corrupt-cell arm stays
class-only under SEC-008, and the `BackupRecordsCorrupt` sibling
over the backups table keeps its own label on purpose).

## Plugins

Four Tauri plugins are wired in `src-tauri/src/lib.rs` (spec §5.3): dialog,
updater, single-instance, and opener. SEC-001 keeps every one of them on
the Rust side:

- The webview gets no plugin JavaScript packages (`package.json` carries
  only `@tauri-apps/api`, pinned by a `config_tests.rs` test) and the
  capability grants it exactly one conscious extension, the
  event-subscription pair `core:event:allow-listen`/`-unlisten` (the
  T-246/D-116 follow-up to D-024's original empty grant) — application
  commands are not capability-gated, so apart from subscribing to
  backend events this leaves the UI exactly its typed command surface.
- `dialog`, `opener`, and `updater` are invoked from Rust behind the app
  commands (`app_open_data_folder` since T-024 opens the data directory
  through the opener's Rust API; `dialog_pick_folder` and `update_*` are
  planned separately); no `fs` or `shell` plugin is registered or granted
  (`tauri-plugin-fs` reaches the binary only as an unregistered
  transitive dependency of `dialog`, never initialized as a plugin), the
  tray and autostart are out of v1 (D-28), and there are no deep links
  (SEC-004).
- `single-instance` (SYS-006) is registered first — the plugin requires
  it — and is desktop-only (`#[cfg(desktop)]`). Its callback runs inside
  the first instance and restores the `main` window (show if hidden,
  unminimize if minimized) before focusing it; a closed window is logged
  and skipped, never a panic (DECISIONS.md D-025). The inter-process
  mechanism itself (instance lock, second process exiting before any app
  setup, callback dispatch) is a process-level facility and is covered by
  manual smoke runs and the CI build matrix; the handler's window
  selection and missing-window case are unit-tested on the mock runtime.

The updater registers from a disabled `plugins.updater` stub
(`pubkey: ""`, no endpoints) because the plugin refuses to initialize
without the section (DECISIONS.md D-023); it stays network-inert until the
release task sets the real key and feed.

## Data directory

The data directory is the Tauri default `app_local_data_dir()` —
`local_data_dir/pro.yepbro.eunomiasync` (spec §5.5). At startup, before
any of the app's own code touches the directory, `src-tauri/src/data_dirs.rs`
(`ensure_data_layout`) creates the root and its four subdirectories —
`db/`, `db-backups/`, `backups/`, `logs/` — idempotently, so a first run,
a restart and a partially deleted layout all converge to the same
structure. (Tauri builds the config-declared windows before the `setup`
hook runs, and their WebView data sits under this root on Windows and
Linux, so the webview layer may create the root first; the layout step is
idempotent, so that is harmless.) The per-OS path table is pinned by
`config_tests.rs` (bundle identifier plus the unset
`appDirectoriesOverride`), and the `setup` wiring — including that the
fallible steps (path resolution, layout creation, config loading)
propagate with `?` — by a text pin there. `config.toml` is
not created by the layout — it is the config task's job (spec 5.8,
SYS-011), which only needs the directory to exist first. Logs live in
`logs/` inside the data directory on every OS: on macOS the Tauri
`app_log_dir()` default (`~/Library/Logs/…`) is deliberately not used,
and the logging module (`src-tauri/src/logging.rs`) points its writer
at `<data dir>/logs/` explicitly; a `config_tests.rs` absence pin
(comments stripped, live code only) makes a future `app_log_dir()`
wiring fail a test instead of passing silently (see the Logging section).
If the layout cannot be created (permissions, an occupied path), the
startup aborts: the error propagates out of the `setup` hook and tauri
panics with `Failed to setup app: …`, carrying the path and cause — there
is no meaningful run without it (DECISIONS.md D-027).

On Unix the same layout step enforces spec §13 SEC-007's directory
modes: `backups/` and `db-backups/` are held at exactly `0700`,
re-asserted at every startup (not only set at creation), so an
existing directory with wider bits — restored from a copy, loosened by
a tool — converges back on the next run; a chmod failure aborts the
launch like a creation failure. The other layout directories (`db/`,
`logs/`, the root) keep the OS defaults: SEC-007 names the backups
folders and the DB file only. On Windows nothing is chmod'ed — the
rights are inherited from the user profile (SEC-007).

Manual check, pending until the first GUI smoke run on each OS: on a
clean profile, launch the app once and confirm the four subdirectories
exist under the §5.5 path — Linux
`~/.local/share/pro.yepbro.eunomiasync/`, macOS
`~/Library/Application Support/pro.yepbro.eunomiasync/`, Windows
`%LOCALAPPDATA%\pro.yepbro.eunomiasync\` — that a second launch adds
nothing and changes nothing, and that the first launch creates
`logs/eunomiasync.<date>.log` inside the data directory (see the
Logging section — since T-255 a chatty day continues into
`eunomiasync.<date>.2.log` parts at 10 MB each, at most five log files
in total). On Linux and macOS also confirm the SEC-007 modes
with `stat`: `backups/` and `db-backups/` at `0700`, and — while the
app is running, so the sidecars exist — `db/eunomiasync.sqlite3`, its
`-wal`/`-shm` siblings and the day's `db-backups/daily-*.sqlite3` at
`0600` (or owner-only for the siblings). On Windows there is nothing
to check: the rights are inherited from the user profile (SEC-007).
CI does not execute this assertion on macOS (its leg only builds), so
the manual run is the macOS half of the criterion. The first launch
also writes the day's ERR-001 copy
`db-backups/daily-YYYYMMDD.sqlite3` (see the Storage section), and
pays one full-copy `VACUUM INTO` of I/O doing it — not a hang; a
second launch the same day must leave the copy untouched. The same
first launch must create `config.toml` with the default content
(`log_level = "info"`, SYS-011),
a second launch must leave it untouched, and a deliberately broken
`config.toml` must not prevent startup: the app starts with defaults and
records a warning in the log file (SYS-012), with the file's bytes
preserved.

## Storage

SQLite is accessed with `rusqlite` (feature `bundled`, so no system SQLite
is required) and schema evolution uses `rusqlite_migration`
(DECISIONS.md D-002). The database file is opened by
`eunomia_store::storage::open_db(&Path)`, which applies the spec DB-001
pragma set (`journal_mode=WAL` — verified against the mode SQLite
reports back, `foreign_keys=ON`, `busy_timeout=5000`,
`synchronous=FULL`) on every open, never creates parent directories
(the app-layer data-directory layout, D-027, owns them) and fails the
open when WAL cannot be used (D-030). On Unix the same open sets the
file to owner-only `0600` (spec §13 SEC-007) — immediately after
creation and again on every open, so both a fresh file and a drifted
one (restored with wider bits) converge, and the WAL/-shm siblings
SQLite creates on the WAL switch inherit the mode; every backup copy
is `0600` from the staged temporary onward (one enforcement point both
copy schemes share). On Windows the profile's inherited rights stand.
A mode that cannot be set fails the open (the copy path reports it as
a backup-write failure). Migration files live in
`crates/eunomia-store/migrations/*.sql`, are embedded with
`include_str!`, and applied by
`eunomia_store::storage::run_migrations(&mut Connection, &Path)`, which
brings any database to the latest schema version — each migration in its
own transaction, and, before every migration applied to an existing
database (`user_version > 0`), a `VACUUM INTO` safety copy
`pre-migration-v<N>-<UTC>.sqlite3` into the caller-provided backup
directory (`db-backups/`; D-038). Schema version 1 — the eleven
§6.2 tables with their full constraint set — lives in
`migrations/0001_init.sql` (see DECISIONS.md D-036 for the two
strengthened constraints and the testing contract); version 2
(`migrations/0002_backup_records_pair_index.sql`, T-256) appends the
index-only `ix_backup_records_pair_lookup` over `backup_records
(project_id, source_path, status, created_at)` — the owner-approved
column set serving the per-pair `last_checked` dedup lookup (§9.4
phase 2), which previously scanned the table once per project per
sync run (D-132's follow-up); version 3
(`migrations/0003_assignment_custom_text.sql`, T-331) rebuilds
`project_instructions` with the direct instruction FK, nullable variant,
nullable custom text and the exactly-one-target CHECK (DB-012/MAIN-007),
copying existing variant-backed rows verbatim. Tests run against in-memory
SQLite and file-backed databases through the public API, including the real
v1→v3 upgrade path and both pre-migration copies (`tests/schema_v2.rs` and
`tests/schema_v3.rs` build their source databases from the real 0001/0002
SQL); broken-step paths go through a private seam with synthetic migration
sets (D-038).

The app side of that storage is `src-tauri/src/db.rs` (DECISIONS.md
D-037): `Db` — the managed state — holds the **one** connection
(SYS-002) behind a `std::sync::Mutex` and is opened by the `setup` hook
from the data directory right after logging — `Db::open` derives both
§5.5 paths through the `data_dirs` helpers (`db/eunomiasync.sqlite3`
and the `db-backups/` the migration run's safety copies go to, D-038)
and, as its **first** step, rejects a database newer than the app
(DB-004/NFR-010): `PRAGMA user_version` is read on a bare connection
and compared with the store's `latest_schema_version()` — before
`open_db`, because the DB-001 WAL switch writes to the file and §10's
`E_DB_TOO_NEW` effect is "The app does not change the database"
(the T-F11 byte-identity criterion). The rejection is a typed
`DbInitError::TooNew` whose `app_error()` is the §7.10 wire form of
`E_DB_TOO_NEW` (path and both schema versions as params); it aborts
the launch like every other init failure (D-027/D-028 — the
"update the app" screen is the future recovery/update task's) and is
logged through the facade (`logging::db_too_new`, §10 code + params)
before the abort. The gate's read leaves the file byte-identical, no
`-wal`/`-shm` debris and no safety copies — with the recorded D-040
exceptions, all of them SQLite's own crash recovery during the read
(a WAL file with pending frames is checkpointed; a hot rollback
journal is rolled back), which move bytes while preserving content,
and an "unknown" verdict (unreadable version) degrading the
rejection to the post-pragma backstop. A migration-run failure on a
file that became newer mid-open is classified from the version fact
the same way (the app layer's mapping of the store's
`DatabaseTooFarAhead`, D-038). As the **second** step, the integrity
gate (spec §6.3, DECISIONS.md D-045) runs `PRAGMA quick_check` on an
existing, non-empty file — still on a bare, read-only connection,
still before `open_db`, so a damaged database is rejected as the
typed `DbInitError::Corrupt` (wire form `E_DB_CORRUPT` with the path
and SQLite's own finding as params) with the file byte-identical
(T-F11's damaged-DB half); a missing or 0-byte file skips the check
(a valid empty database — nothing to verify, so a first launch pays
nothing), garbage fails it with `SQLITE_NOTADB`, a malformed image
with `SQLITE_CORRUPT`, and any non-corruption read failure
(busy/locked, I/O) degrades to the canonical failure one step later
instead of a false corrupt verdict. §10's "start in recovery mode"
is, since T-024, this typed verdict plus continuation: the hook logs
`logging::db_corrupt` and returns `Ok` with no `Db` managed — see the
recovery paragraph below (DECISIONS.md D-049). Only then do `open_db` +
`run_migrations` run (the store stays the only SQLite/schema owner).
A migration step that fails is rolled back by its own transaction;
the failure surfaces as the typed `DbInitError::Migration` (UPD-004,
D-043): the store's error carries the pre-migration copy of the
unchanged state, the abort `Display` names that copy and the
"install the previous app version" instruction, the wire form
(`app_error()`) is `E_UPDATE_FAILED` with the
`errors.update_migration_failed` key and both paths as params, and
the `setup` hook logs it through `logging::db_migration_failed`
before the abort — a failure on a database the run itself created
has no copy and keeps the generic abort. Every
database command runs through `Db::run`: the operation closure executes
on `tauri::async_runtime::spawn_blocking` (SYS-001 — blocking SQLite
work never touches the async workers), inside exactly one deferred
transaction (`unchecked_transaction` … `commit`, rusqlite's `Drop`
sending the `ROLLBACK` on any error, NFR-008), and receives a
`&Transaction`, so its statements join the one transaction by
default and cannot escape the boundary by accident (a deliberate
`execute_batch("COMMIT")` through the Deref remains possible, as
`db.rs` documents). The mutex
serializes operations on the connection and is never held across an
`.await` (the `std` mutex makes that unrepresentable). Errors map to
the §10 contract through the base classifier in the same module
(busy→`E_DB_LOCKED`, corrupt→`E_DB_CORRUPT`, full→`E_DISK_FULL`,
read-only→`E_NO_WRITE_ACCESS`, constraint→`E_VALIDATION`, and the
recorded fallback for the rest); a panicked operation rolls back, the
poisoned guard is recovered (the rollback ran during unwinding), and
the next operation proceeds. `db_tests.rs` proves the acceptance
behavior against real database files: mid-operation failures roll back
everything, concurrent operations serialize on the connection, long
operations leave the async runtime responsive, and a panicking
operation neither commits nor poisons. A busy/locked failure
(`busy_timeout` expired under another connection's lock) is retried
by the executor itself — §10 `E_DB_LOCKED`'s "up to 3 automatic
retries": the whole operation re-runs in a fresh transaction (the
failed attempt rolled back, so the retry is a fresh run — the
operation signature is `FnMut` for exactly that), pausing §9.5's
100/300/900 ms ladder on the blocking thread between attempts and
dropping the connection lock for the pause; after the third retry the
failure surfaces as the `E_DB_LOCKED` §7.10 wire value. Only
SQLite's busy/locked class enters the cycle — the operation's own
errors, constraint violations and the recorded fallback (which also
lands on `E_DB_LOCKED`) surface on their first attempt, their
`details.os_kind` keeping the producers separable — and each retry is
logged through the facade (`logging::db_busy_retry`, a numeric-only
`warn`; the one thing the executor logs itself, because a retry is
invisible to every other layer).

The same `db-backups/` also holds the daily startup copies (spec §10
ERR-001, DECISIONS.md D-046): after the database is open and
migrated — and before the state is managed — the `setup` hook calls
`Db::ensure_daily_backup(now)`, which locks the one connection and
runs `eunomia_store::storage::daily_backup` into the directory
`Db::open` derived once (`jiff::Timestamp` is the injectable "today"
seam; only its UTC date names the copy `daily-YYYYMMDD.sqlite3`).
At most one copy exists per UTC day (a run that finds the name
present changes nothing — decided by the file name, the copy never
opened), and after a copy is written the newest 7 daily names are
kept, oldest deleted — never the copy just written, so a
clock-set-back directory cannot cost the day its copy; pruning is
best-effort throughout and never turns a successful copy into a
reported failure. `pre-migration-*` names are outside this daily cap;
their retention is the separate T-253 age rule below, not a count. A fresh
first launch gets its copy of the empty
database too ("at every startup" — a cheap, honest baseline). The
publication mechanics are the pre-migration scheme's (D-038): `.tmp`
staging, `hard_link` no-clobber publication, §9.5 retries, and a
per-scheme sweep of stale `.tmp` debris. Failure policy — degrade,
never abort (the deliberate difference from a pre-migration copy
failure, which aborts): the daily copy is extra reliability over a
healthy database, so a failure is returned to the `setup` hook,
logged through the facade (`logging::db_backup_daily_failed`, a
`warn` — the launch continues; a written copy gets one
`logging::db_backup_daily` `info` line, an existing same-day copy
gets none), and every later command runs normally. The step runs
synchronously in `setup`, so the first launch of a day pays one
full-copy `VACUUM INTO` of I/O before the window exists — ERR-001's
own cost, not a hang. These
file-log lines remain the whole journal of daily-copy creation: the
`action_log` journal exists (T-026), but LOG-001..004's "backup
creation" is the BKP-001 target-file family, not this database copy
(see the boundary note on `logging::db_backup_daily`).

Right after the daily copy and `app.manage`, the `setup` hook spawns
the LOG-006 retention task (spec §7.7, §6.4 "Log and runs",
DECISIONS.md D-054): `src-tauri/src/retention.rs` runs one cleanup
pass immediately (asynchronously — `setup` does not wait) and then
once every 24 h, on `tauri::async_runtime` with `tokio::time::sleep`
as the timer. One pass is one `Db::run` operation: both `log.*`
settings are read and the prune executes in the **same** transaction
(SYS-002 — the limits and the deletes describe one consistent
state). The semantics live in the store
(`eunomia_store::action_log::cleanup`): a journal entry is deleted
when it is outside the newest `log.max_entries` entries (`ts DESC,
id DESC` — the Activity window's ordering) **or** strictly older than
`now − log.max_days` days (an entry exactly at the boundary
survives), and `sync_runs` keeps the last 500 by `started_at DESC,
id DESC` — older runs go with their `sync_run_projects` rows (the
schema's `ON DELETE CASCADE`), `backup_records` rows keep theirs
with a NULL `sync_run_id` (`ON DELETE SET NULL`), and journal
entries naming a pruned run keep the id string (no FK there). The
pass is idempotent and joins its caller's transaction, so a rollback
restores everything. Failure policy — the D-046 shape: a failed pass
is logged through the facade (`logging::db_cleanup_failed`, a `warn`)
and the loop sleeps on to retry next day; a pass that removed
something gets one `logging::db_cleanup` `info` line with the counts,
a no-op gets none. The cleanup writes no `action_log` entry itself
(LOG-001..004 has no housekeeping member — the facade lines are its
whole journal). Honest boundaries, recorded in D-054: the 24 h tick
counts from launch, not UTC midnight; the task has no cancellation
token — process exit drops it, an interrupted pass rolls back with
its transaction, and the next startup re-runs it; the runs cap keeps
the newest 500 regardless of status, so an app whose newest 500 runs
are all failed/aborted loses the rows LOG-007's "overall last
successful sync" would read from the runs table — §6.4 mandates the
cap, so the task that builds LOG-007 must read the overall time from
`projects.last_synced_at`, not from the runs; and the schema's own
comment on `action_log` still says "append-only" — the migration
file is frozen (D-010/D-036), and since T-029 this retention cleanup
is the one deliberate deleter. The startup wiring (open → the §9.4
interrupted-runs sweep → the DB-022 commit hook → daily copy →
manage → retention spawn) is pinned by the two text pins — the
extended `db_tests.rs` one for open → daily copy → manage →
retention spawn, and the `ipc_tests.rs` pin added with the sweep
(T-258) for open → the sweep → the commit hook → manage.

**Recovery mode** (spec §10 `E_DB_CORRUPT` row, §7.10 command notes;
DECISIONS.md D-049): when the startup open fails the quick_check gate,
the `setup` hook logs the verdict and continues **without** managing a
`Db` — that state (layout up, `DataRoot` and the session states
published, the two service buses parked and unarmed, no database) is
recovery mode, and the §10 screen that replaces the app is what it
exists for; the too-new, rolled-back-migration and generic-open
failures keep their abort. The recovery commands
(`src-tauri/src/commands.rs`, the first registered command group since
D-037) run in exactly that state: they are file operations over the
managed `DataRoot` (`src-tauri/src/recovery.rs`), not `Db::run`
executor work — there is no connection to run anything through. The
read-only pair: `db_backups_list` (the `*.sqlite3` regular files of
`db-backups/` — `name`, `size_bytes`, `modified_utc` UTC ISO-8601,
newest first; staging debris, preserve siblings, foreign files and
symlinks are not copies) and `app_open_data_folder`
(the opener plugin's Rust API; SEC-001 — the webview holds no
`opener:` permission). The destructive pair — `db_recreate`,
`db_restore_backup(backup_name)`
— is guarded three times (a concurrent recovery → `E_VALIDATION` over
an async-mutex gate, taken FIRST; a healthy `Db` already managed →
`E_VALIDATION`, checked under that gate so no recovery can slip
between the check and the publication; and for `db_recreate` alone,
a live file both startup gates accept → `E_VALIDATION` "a working
database is already in place": the empty start never silently
discards a working unmanaged database — the state a restore whose
reopen failed leaves)
and then, inside one `spawn_blocking`: validate the selection (a
plain `*.sqlite3` base name only — no separators, `.`/`..`, `.tmp`
staging debris or `-wal`/`-shm`/`-journal` siblings, a 0-byte
sibling would pass both gates and silently bootstrap an empty
database; missing → `E_VALIDATION`; symlinks not followed), re-run
the startup gates over the chosen copy (`E_DB_TOO_NEW`/`E_DB_CORRUPT`
before anything is touched), **stage the copy**
(`db/eunomiasync.sqlite3.tmp`:
a byte copy, never a hard link — a link would let later writes
mutate the backup — `0600` before publication, fsynced, while the
live file is still in place, so a full disk or a permission leaves `db/`
exactly as it was), preserve the damaged database as
`db-backups/pre-recovery-<UTC>.sqlite3` (renamed, not discarded —
though the T-253 age rule may retire it as an aged copy at a later
open; chmod `0600` before the rename, stale `-journal`/`-wal`/`-shm`
siblings renamed alongside first — though a gate read over the
damaged file already lets SQLite's own crash recovery remove bogus
siblings, the preserve catches what those reads leave — the main
rename as the commit point), publish the staged copy by one rename
(§9.5's staging pattern, parent-directory fsync best-effort), reopen
through the full `Db::open` chain, and run `start_database_session`
(`lib.rs`) — the same tail a healthy startup runs, since T-322/D-186: sweep
any `running` rows a dead process left into `interrupted` (the §9.4 startup
crash recovery, T-258 — recovery mode is the one state where no run can be
live), install the DB-022 commit hook, write the day's ERR-001 copy,
`app.manage` the reopened database, and start the state services over it
(dirty recalculation, target-file watch, availability startup pass, daily
retention). §7.10's "…and reopens it": the app continues without a
restart, armed like a launch. The two service buses' receiving halves are
parked in the managed `StateServices` slot from the `setup` hook until
that tail takes them — once per session; until T-322 the recovery
published a database and nothing else, and the recovered session ran
without its services until a restart. The window in which the database
path is empty is the two renames; the preserve audit line rides the
`on_preserved` callback into it. Every recovery step is audited
through the facade (`recovery_preserved`/
`recovery_recreated`/`recovery_restored`/`recovery_failed`, plus the
opener's two lines — the failure lines carry the code literal, the
params, and the OS facts when the failure had any).
`recovery_tests.rs` pins both card criteria through the real seams,
the escape/gate rejections, the failed-stage window and the
healthy-refusal; `ipc_tests.rs` drives the commands over mock IPC,
including the §7.10 error envelope of a rejected invoke — the D-034
pin. Honest boundaries (D-049): a crash between the two renames
leaves the next startup to bootstrap fresh while the original sits
under its `pre-recovery-` name; and the frontend screen itself is the
recovery-UI task's.

The protective copies both schemes leave behind are governed by the
T-253 age rule (owner ruling 2026-11-21, `src-tauri/src/retention.rs`):
`pre-migration-*` and `pre-recovery-*` files older than 7 days — aged
by the creation stamp in the file name, never by count or mtime — are
deleted by a best-effort pass that runs once, at the end of a
successful `Db::open` (`db.rs`) — which covers every startup, the
moment a migration run has written its copies, and the reopen after a
recovery. `preserve_damaged` is deliberately not a trigger: a
restore's chosen copy must outlive a failed publish (D-129). The
newest copy of each kind is protected even past the window, the
`daily-*` scheme stays under ERR-001's own count rule beside them, and
a copy deleted by the pass takes its `-journal`/`-wal`/`-shm` sidecar
siblings with it.

Orphaned project-backup copies — `.bak` files under `backups/` that no
`backup_records` row claims (the crash window D-091 records: the copy
is written straight to its final name, the record follows) — are
reported by a read-only pass at the same end-of-open point, never
deleted (the owner ruling of 2026-11-24, T-332, D-190):
`src-tauri/src/backup_orphans.rs` reads every claimed `backup_path`
(through `eunomia_store::backup_records::all_paths`, all statuses),
lists each `backups/<project folder>/` one level deep, and logs one
`logging::backup_orphans` `warn` line with numeric fields only —
`orphan_files`, `orphan_folders`, `unreadable_folders`; no paths and
no folder names, because a folder name carries the project-name slug
(SEC-008). The line is emitted when an orphan was counted or a
project folder could not be listed; when nothing was counted but a
folder could not be listed, it carries a distinct message (the two
counts are a lower bound then), while a mixed pass keeps the orphan
message with `unreadable_folders` beside it. A tree with no orphans
and every folder readable logs nothing. The rule the line
belongs to: the user inspects the folder and deletes a file by hand
when its content is not needed; the app never reclaims the space (a
copy with no row may be the only surviving evidence of a file). A
failed records read or an unlistable root degrades to its own `warn`
line and never fails the open; a project folder that cannot be listed
is counted and the readable ones are still judged.

The `settings` table has a typed repository, `eunomia_store::settings`
(spec §6.5, DECISIONS.md D-039): the thirteen §6.5 keys as a
`SettingKey` enum, values validated per key as JSON
(`SettingValue::from_json` — one parse for reads, writes and the
future IPC), §6.5 defaults applied by reads when a row is missing, and
an explicit `StorageError::Settings` when a stored cell is corrupt —
never a silent default (the table is machine-written, unlike the
hand-editable `config.toml`, SYS-012). `default.*` keys are read only
by project creation; changing them never touches existing projects
(DEF-005). The Settings module's IPC commands (T-030,
DECISIONS.md D-055) serve this repository through the `Db::run`
executor: `settings_get` answers the full map in §6.5 table order
(defaults applied), `settings_set` applies a partial update — the
screen's dirty set — atomically in one transaction, validating every
key/value through the store's own parse and rejecting anything
invalid with `E_VALIDATION` naming the key, the sent value and the
reason. The wire DTO stays string-shaped (dotted key + canonical JSON
text; the `details` precedent) — no type duplication, and
`specta/serde_json` stays disabled. Every changed key writes one
`settings.changed` journal entry inside the operation's transaction
(LOG-001…004, D-051), its entity id the dotted key and no value in
the payload. "Applies without a restart" (DEF-001…004) holds by
construction — no layer caches settings, so the next reader (the
`settings_get` round trip today, the future scanner through the same
repository) sees the committed value.

The `action_log` table has the journal repository,
`eunomia_store::action_log` (spec §7.7 LOG-001…005, DECISIONS.md
D-051): a closed `Action` enum mapped member-by-member onto
LOG-001…004's own list, `Level`/`Visibility`/`Outcome` matching the
§6.2 CHECK vocabularies, and a typed `Details` payload (`Bulk`,
`File`, `Failure`) whose fields are numbers, paths and the
shape-validated `FailureCode` newtype — SEC-008 is structural: no
variant of `Details` accepts free text, so instruction content has no
route into the column, and `FailureCode::new` rejects every
instruction-shaped string (the form check against the §10 registry
shape, not the membership — the member list stays in `errors.rs`).
Transactional semantics, resolved explicitly: a successful operation
writes its entry inside the operation's own transaction (the
`settings::set` shape — a rolled-back operation leaves no entry), and
a failed operation's entry is written **after** the rollback, in the
next transaction (fail-then-log: the error entry must survive the
transaction it describes). Reads come in two seams with two
orderings: `recent` (newest by insert order — journal replay) and
`list` (the Activity window's query, T-027: filters by inclusive
DB-002 date range, project, actions, result and visibility, paginated
with limit/offset plus a filter-matching `total`, ordered `ts DESC,
id DESC` so equal-millisecond entries page deterministically — both
seams fail with `StorageError::ActionLog` on any cell outside its
vocabulary, a non-JSON `details`, a stored `ts` cell outside the
DB-002 shape, or a date bound that is not the exact DB-002 shape;
that shape is the precondition under which text
comparison over `ts` equals chronological comparison, which is what
makes the range filters correct). The `activity_list` command
(§7.10 History module) serves that query through the `Db::run`
executor with LOG-005 enforced at the IPC boundary: user entries by
default, `diagnostic` only on the explicit `include_diagnostic`
request (D-052 records the DTO design). The journal reaches the file
log through one facade function, `logging::action_logged`, which owns
the selection policy: an `error` or `warn` row mirrors whatever its
visibility (an `info` row whose result is an error included), a
diagnostic `info` mirrors, and the one shape that does not is an
`info` + `user` + non-error row — the Activity window's own data,
already in the database journal. The payload is codes and ids only
(the policy reasoning lives with the function). LOG-006's retention/cleanup
lives in the same module since T-029: `cleanup(conn, limits, now)`
enforces the §6.4 "Log and runs" row over the caller's transaction
(see the Database section above for the full semantics and the
startup wiring). The first producer is wired (T-030): `settings_set`
writes its `settings.changed` entries through `insert` inside the
operation's transaction and mirrors them through
`logging::action_logged` (a no-op for the info+user+ok shape, per
the policy above); the fail-then-log half stays with the first
operation that fails after writing.

## Config

Configuration format is TOML (serde + `toml`), per DECISIONS.md D-001.
At startup, right after the data-directory layout and before logging and
the database are set up (spec 5.8, SYS-011), the `setup` hook calls
`src-tauri/src/config.rs` (`load_or_create`): a missing `config.toml` is
created with the pinned default content (`log_level = "info"`) — staged
in a `.tmp` sibling renamed into place, so a crash mid-write cannot tear
the file — and an existing file is parsed and **never rewritten**: a
broken TOML, invalid UTF-8 bytes or an invalid `log_level` fall back to
in-memory defaults with a warning and the app still starts (SYS-012),
unknown keys are ignored with a warning naming the key (manual diff of
the parsed `toml::Value`'s top-level keys against the `KNOWN_KEYS` list
— no `serde_ignored` dependency), and only an unreadable/unwritable file
aborts the launch like the layout step does (DECISIONS.md D-028).
Warnings come back as a `Vec<String>` from the seam (the config runs
before any log subscriber exists, SYS-011) and are re-emitted into the
tracing file log after logging initializes; the config itself
is published as managed state, and the logging step reads it via
`app.state::<AppConfig>()` to filter the log — applying `log_level` to
the log output (DECISIONS.md D-029). The
startup wiring is pinned by a `config_tests.rs` text pin (layout →
config → `app.manage` → level → logging → guard → warnings); the seam
behavior is unit-tested in
`config.rs` over `tempfile` roots. User-managed settings stay in SQLite
(spec 6.5) and no library data is ever written to the file.

## Logging

The app writes its `tracing` file log — `tracing` +
`tracing-subscriber`'s fmt layer over a custom writer of
`src-tauri/src/logging.rs` on `tracing-appender`'s non-blocking worker
thread — wired in `logging.rs` (spec §5.3; see DECISIONS.md for the
dependency entry). The file lands
in `<data dir>/logs/eunomiasync.<UTC-date>.log` on every OS (spec §5.5;
the `app_log_dir()` default is deliberately not used — see the Data
directory section). Rotation is two-triggered (owner ruling
2026-11-21, "5 files × 10 MB ≈ 50 MB max"; T-253 shipped the count
half, T-255 the size half): a new UTC day starts a new file, and a
file that can hold no more than 10 MB rolls into a same-day part
(`eunomiasync.<date>.2.log`, `.3.log`, … — the first file of a day
keeps the exact §5.5 name; the ruling is unsatisfiable without
same-day files, and the §5.5 structure block lists primary shapes
only, the T-024 `pre-recovery-*` precedent — see the DECISIONS.md
T-255 entry). The ruling needed an own writer because
`tracing-appender` 0.2.5 rotates on time only and offers no way to
force a mid-day file switch; the writer also runs the retention:
before every rotation (and at every startup — to five when today's
file already exists, to four when it is about to be created) it
prunes the oldest
files beyond **five**, ordering by the (date, part) stamps parsed from
the file names — never mtime — and never touching a file whose name is
not its own canonical grammar. Restarts are idempotent: today's
highest existing part is continued at its true length, and a part
already at the cap rolls on its first write. The maximum level comes
from `config.toml`'s
`log_level` (SYS-012), applied by initializing the subscriber _after_
the config is loaded (SYS-011) — `app.state::<AppConfig>()` is the seam
that hands the level over (D-029). The writer guard is kept alive as
managed state so the non-blocking writer never shuts down early; tauri
exits the process without dropping managed state, so what persists log
lines is the writer thread's per-batch flushing (only lines queued in
the final moments of the process can be lost).

Failure policy: logging is diagnostics, so a broken log file (read-only
directory, full disk) never aborts the launch — `logging::init` degrades
to a stderr subscriber with the same level filter and logs one `error`
event naming the failing path. Both subscribers are built with
`log_internal_errors(false)`: the fmt layer's default writer-error
fallback is an `eprintln!` of its own, which panics on an unusable
stderr (a Windows GUI release build has none) — and the workspace's
`panic = "abort"` would turn that into a process abort; a log line that
cannot be written is dropped instead. The same policy governs the
writer's rotation mid-run: a prune that cannot read or delete, or a
next part file that cannot be opened, is reported once through the
facade and then ignored — the writer keeps writing into the current
file (the 10 MB cap is exceeded, never the launch) and retries every
following write, so a healed disk heals the log. This is deliberately
weaker than the layout/config abort policy (D-027/D-028): those guard
data the app cannot run without. Taking the rolling appender out of
the path (T-255) also retired its `eprintln!`-on-failure sites — the
Windows GUI abort exposure D-129 flagged; the crate's one remaining
`eprintln!` (the worker's final flush at guard-drop shutdown) is
unreachable in the app, which never drops the guard, and every
workspace crate's production code is kept `eprintln!`-free by the
workspace-level clippy gate (`[workspace.lints.clippy]`
`print_stderr`/`print_stdout` in the root `Cargo.toml`, inherited by
each member manifest).

SEC-008 is enforced structurally: every tracing event of the app is
constructed in `logging.rs` from typed parameters (paths, operation
codes, numeric IDs) — the event macros and `#[instrument]` are banned
everywhere else by a `config_tests.rs` text pin, and the facade's
`file_event` primitive has no parameter that could carry instruction
text or file content. The primitive is a tested, still-unused facet
(`#[allow(dead_code)]`, the keep ruling D-163): the sync and backup
tasks landed without it — sync journals its file-write outcomes in
the `action_log` journal mirrored by `action_logged` (D-051) plus
the `sync_*` diagnostic facets, and the phase-2 backups record
theirs in `backup_records` rows — and the facet's prospective
consumers are the pending file-operation producers outside the sync
journal (the variant library tasks, `backup_restore`/BKP-007); the
first such producer either adopts the primitive or retires it. The
`logging.rs` tests prove the contract end to end: a file event carries
the path and the ID while the operation's instruction text stays out of
the log. One deliberate exception: config warnings may quote the
offending bytes of `config.toml` itself (D-028 — the machine config is
user-written diagnostics input, not library content). When the first
library crate adopts `tracing`, its task must extend this discipline —
the pin already covers `crates/**` and will force that decision.

Honest testing boundaries: the midnight rollover and the global
subscriber install are not unit-testable (the date cannot be moved in a
test; the global default is process-wide and the suite runs in
parallel) — the dated file name, the per-level filtering, the SEC-008
payload, the size-rotation arithmetic (a small-cap seam over the same
production path, plus a real-constants pin that writes past 10 MB and
asserts the parts), the restart idempotency, the prune verdict and the
degradation policy are (driven through a real subscriber over
`tempfile` roots and directly through the writer, asserted by reading
the real files back); the rollover itself, the production path of the
rotation-degradation warn (it fires on the worker thread, where the
tests' thread-local subscriber does not exist) and the file
location on each OS rest on the manual smoke run — the steps live in
`docs/manual-smoke.md`. A Windows GUI
smoke should confirm: a normal session logs into
`logs/eunomiasync.<date>.log` and rolls into `.2.log` parts past 10 MB
with at most five files total; a read-only `logs/` directory starts
the app in the stderr degradation without aborting; a locked log file
does not crash the prune.

## Watcher

File watching uses `notify` with debouncing through `notify-debouncer-full`
(DECISIONS.md D-004, D-137). Scope: the target files of active, available
projects, display-only — external edits are shown in the UI and overwritten
by the next sync (spec 5.7); a watch event never writes a file. The mechanic
half lives in `eunomia-fs` (`watcher`: one debounced, non-recursive parent-
directory watch per set of target files, name-filtered — the inode-trap
rationale in the module docs), the semantic half in `src-tauri`
(`file_watch`: the SYS-008 re-read-and-compare verdicts behind the
`state://file-changed` event; subscriptions refresh at startup, on
availability events, through the after-sync seam the sync pipeline
calls, and through that seam's BOUNDED twin after the project
mutations that change the watched set — a wait budget (10 s,
D-156) so the watch pass can no longer hang an interactive answer
on a stuck mount; the mutation flow's own older probe/canonicalize
exposure stays D-144's recorded boundary;
`project_add_manual`'s committed arm today,
the scheme/name/archive/restore commands as they land, D-144; the
verdicts are readable by any caller through the `projects_list`
column carrier). On Linux, the `fs.inotify.max_user_watches` limit must be
respected: an event-queue overflow degrades to one full re-check
through the `Backend` event (SYS-010), while a REFUSED watch (an
unwatchable root, the watch limit itself, or a refused re-arm the
backend's refusal class proves fatal to the watch — D-148's liveness
attribute) leaves the file unwatched — silently for events, but
recorded per full pass as the wanted-but-unwatched set WITH the
refusal class that explains each entry (`unwatched_kinds`, D-155):
a non-empty changed recording warns once through the facade
(`logging::file_watch_refused`, one line per degradation episode
with the per-class counts), and the SYS-010 non-blocking notice is
a poll — the `unwatched_reason` field on the `projects_list`
carrier, `None` for watched/archived/unavailable rows, never an
event (D-137's P2-4 loop ruling); neither case ever crashes. The
watcher's other diagnostic edges — the init-failure warn, the
pass-failure error (fire-and-forget triggers only; an awaited
failure's line is the caller's) and the emit-failure warn — flow
through the same facade (D-155).

## Updater

Auto-updates use `tauri-plugin-updater`, wired in `src-tauri/src/lib.rs`
(DECISIONS.md D-005, D-023). Signing keys and the update endpoint are
separate (release) tasks; until they land the plugin stays registered but
unconfigured — every update check fails fast in Rust with
`EmptyEndpoints`, and the webview has no path to the release feed. The
updater is part of the MVP scope (spec 7.9).

## Testing strategy

| Level       | Scope                                                                                                                 | Tooling / command                                                           |
| ----------- | --------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| Unit        | Rust crates and Vue components                                                                                        | `cargo test --workspace` / `pnpm run test` (Vitest)                         |
| Integration | `eunomia-store` storage & migrations                                                                                  | `cargo test -p eunomia-store` (in-memory + file DB)                         |
| API         | Tauri runtime behavior (updater init, wiring)                                                                         | `cargo test -p eunomia-sync` (`tauri::test` mocks)                          |
| Executor    | DB transaction/rollback/runtime paths                                                                                 | `cargo test -p eunomia-sync` (`db_tests.rs`)                                |
| Load        | Spec §12.2 budgets on generated data (five rows: build, dirty, toggle, sync pipeline, scan walk; set L today; M open) | `cargo bench -p eunomia-load` (criterion; local, DECISIONS.md D-135, D-159) |
| E2E         | Browser scenarios against Vite server                                                                                 | `pnpm run e2e` (Playwright, DECISIONS.md D-007)                             |
| Smoke       | App builds on all three platforms                                                                                     | CI `build` matrix: `tauri build --debug --no-bundle`                        |

## CI

GitHub Actions workflow (`.github/workflows/ci.yml`) runs fourteen jobs
(DECISIONS.md D-060, D-074, D-168):

- **lint** — `cargo fmt --check`, `cargo clippy -D warnings`, ESLint,
  Prettier, TypeScript.
- **test-matrix** — `cargo test --workspace` on five OS legs
  (`ubuntu-latest`, `ubuntu-24.04`, `windows-latest`, `macos-15`,
  `macos-15-intel`), `fail-fast: false`; the frontend unit tests
  (Vitest) run on the two Linux legs only (ubuntu-latest and the
  pinned ubuntu-24.04). On the Windows leg a staging step copies the app
  manifest beside every test executable as `<exe>.manifest` — without it
  the manifest-less test harnesses die at load with
  `STATUS_ENTRYPOINT_NOT_FOUND` (D-074; the same note in
  `CONTRIBUTING.md` covers running the tests on a local Windows
  machine).
- **test** — a roll-up gate, not a suite: it fails unless every
  `test-matrix` leg succeeded, keeping the branch-protection context
  literally named `test` stable while the matrix reports one check per
  leg.
- **deny** — `cargo-deny` (licenses, advisories, duplicate crates).
- **e2e** — Playwright against the Vite dev server.
- **build** — `tauri build --debug --no-bundle` on the same five OS
  legs as the test matrix (a compile/link smoke; the macOS legs cover
  both arm64 and x64).

## Platform notes

- **Linux:** requires `webkit2gtk-4.1` and other GTK packages
  (see `CONTRIBUTING.md`); the watcher must handle `inotify` watch limits.
- **Windows:** renders through WebView2 (bundled with Windows 10+). The
  application manifest (`src-tauri/windows/app.manifest`, embedded by
  `src-tauri/build.rs` via `WindowsAttributes::app_manifest`) declares
  `longPathAware` (NFR-023) and `requestedExecutionLevel="asInvoker"`
  (SEC-012/NFR-011); long paths additionally require the OS
  `LongPathsEnabled` policy, which the manifest flag alone cannot turn on.
  The custom manifest replaces tauri-build's default wholesale, so it also
  reproduces the default's Common-Controls v6 dependency (needed by the
  dialog plugin). Automated coverage stops at compile+link for the
  application binary itself: `rc.exe` embeds the manifest as opaque bytes
  (no XML validation), and the CI build job never runs the binary — the
  two runtime criteria are verified
  by a manual smoke run: (1) with `LongPathsEnabled=1`, add a project
  whose path exceeds 260 characters and confirm load and save succeed
  (and note the expected failure with the policy off); (2) confirm no UAC
  prompt on launch and that the embedded manifest reports `asInvoker` and
  `longPathAware` (`sigcheck64 -m <exe>` or
  `mt.exe -inputresource:<exe>;#1 -out:manifest.xml`). The CI test
  matrix, however, does exercise manifest loading at runtime on Windows
  — the test executables run with the externally staged v6 manifest
  (D-074), so a manifest regression that breaks process startup is
  caught by the `test-matrix` leg, not only by the manual smoke.
- **macOS:** requires the Xcode Command Line Tools for the Rust/TKG build.

## Pending decisions

The rules model, managed blocks format, and conflict policy are not
decided yet — see DECISIONS.md for settled decisions and follow-up tasks.
