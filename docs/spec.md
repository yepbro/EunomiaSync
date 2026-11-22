# Technical Specification: EunomiaSync

Spec version 1.2 · 21.11.2026 · based on: Product Requirements v0.3 and the customer's answers · includes the approved refinements from the html-pages prototype audit (D-125)

## 1. General Information

### 1.1. Purpose

EunomiaSync is a cross-platform desktop app (Windows, macOS, Linux). It stores instructions for AI agents in one local library and, on an explicit command, builds an `AGENTS.md` or `CLAUDE.md` file from them in each selected Git project. You write a rule once and assign it to the projects that need it. When you change it, the change goes out only to the places where it is used.

### 1.2. Goals

| ID  | Goal                                                                                      |
| --- | ----------------------------------------------------------------------------------------- |
| G1  | One place to store and edit instructions                                                  |
| G2  | Reuse one instruction in several projects                                                 |
| G3  | Several variants of an instruction, and a choice of variant for each project              |
| G4  | Roll out new rules to many selected projects at once                                      |
| G5  | Project files change only on the explicit "Run sync" command                              |
| G6  | Predictable full overwrite of managed content, and protection from data loss with backups |
| G7  | Traceable changes and a way to go back to earlier states                                  |

### 1.3. Scope of the First Version

**In scope:** local instruction library, projects, assignment matrix, three write schemes, sync, backups, change history (version records), updates, watching of target files for external changes, English UI.

**Next stage (P1):** diff between versions and version rollback, restoring a file from a backup in the UI, and JSON export and import of the library and settings.

**Out of scope for v1:** cloud sync and a server side; roles and multi-user work; merging external edits to target files (external changes are detected and shown, section 5.7, and the next sync overwrites them); Git operations (commit, push, editing `.gitignore`); automatic sync runs; telemetry and automatic error reports; a portable build; a tray icon and autostart; any localization other than English; a command-line interface.

### 1.4. Sources and Priority

If sources conflict, this order applies: (1) the customer's answers from 17.11.2026, (2) Product Requirements v0.3, (3) the default decisions from section 3.2. All decisions in item (3) are confirmed by the customer.

### 1.5. Notation

"Must" is a required requirement, "should" is a desired one, and "may" is an allowed one. Priority **P0** means in the MVP, **P1** means the next stage. IDs from Product Requirements are kept (PRJ, DEF, LIB, MAIN, OUT, BKP, LOG, HLP, UPD, NFR). New requirements use the prefixes SYS, DB, ALG, UI, SEC, ERR.

## 2. Terms and Definitions

| Term                  | Definition                                                                                                                                              |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Project               | A local folder that is the root of a Git repository and is registered in the app                                                                        |
| Library item          | An entry in the shared ordered structure: an instruction or a section heading                                                                           |
| Instruction           | A library item with a main name and one or more variants                                                                                                |
| Variant               | A version of an instruction with its own name and text. Exactly one variant of an instruction is the main one                                           |
| Assignment            | A "project — instruction — selected variant or per-project custom text" link (MAIN-007). One instruction in one project has no more than one assignment |
| Section heading       | A library item of level `##`…`######`; it has no assignment cells                                                                                       |
| Heading section       | The items after a heading, up to the next heading of the same or a higher level                                                                         |
| Build                 | Creating the text of a project's target file from the assignments and the shared structure                                                              |
| Target file           | The file that the app writes in the project                                                                                                             |
| File name (base name) | A choice from a list: `AGENTS.md` or `CLAUDE.md`. The stem is `AGENTS` or `CLAUDE`                                                                      |
| Write scheme          | A — full overwrite of the main file; B — writing to a marked area; C — writing to a separate `<STEM>.project.md` file                                   |
| Managed content       | The content that the app fully overwrites during a sync                                                                                                 |
| Creation marker       | The `Created by EunomiaSync …` line at the end of the managed content                                                                                   |
| Sync (sync run)       | One click of the "Run sync" button for all active projects                                                                                              |
| Preflight             | Checks and backup preparation before writing starts                                                                                                     |
| Build hash            | SHA-256 of the build text without the creation marker                                                                                                   |
| Dirty                 | A project state where the current build hash differs from the hash of the last successful sync                                                          |
| Data directory        | The app folder with the database, backups, and logs (section 5.5)                                                                                       |

| Externally modified | A project state where the target file on disk no longer matches the project's current build by the ALG-033 comparison; detected by the watcher (5.7) |

## 3. Decisions Made

### 3.1. Customer Answers (17.11.2026)

| ID   | Question                                                            | Decision                                                                                                                                          |
| ---- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| D-01 | Scheme A: what to do with `AGENTS.local.md` and `AGENTS.project.md` | The app only adds a "read this file" line to the main file. It does not create, read, or change these files. Closes OUT-A-004 and questions 16–17 |
| D-02 | Several variants of one instruction in a project                    | Not allowed. A project has no more than one variant of an instruction. Closes question 8                                                          |
| D-03 | Target files                                                        | One target file per project, chosen from a list (`AGENTS.md`, `CLAUDE.md`). Closes question 20                                                    |

### 3.2. Default Decisions (confirmed)

| ID   | Topic (PR question)                            | Decision                                                                                                                                                                                                                                                                                                                        |
| ---- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D-04 | Scan depth (1)                                 | 6 levels by default, configurable from 1 to 20                                                                                                                                                                                                                                                                                  |
| D-05 | Scan exclusions (2)                            | By default, folders named `node_modules`, `vendor`, `.git`, `dist`, `build`, `target`, `.venv` are skipped. The list is editable                                                                                                                                                                                                |
| D-06 | What counts as a repository (3, 4)             | A folder with a `.git` directory. If `.git` is a file (submodule or linked worktree), the folder is skipped with the status "skipped" and a reason; you can add it by hand. Nested regular repositories are registered as separate projects                                                                                     |
| D-07 | Symlinks (PRJ)                                 | Not followed during a scan. When a project is added by hand, the path is converted to its canonical form                                                                                                                                                                                                                        |
| D-08 | Non-Git folders (6)                            | Not supported. If the user picks a subfolder of a repository, the app offers the repository root (PRJ-003)                                                                                                                                                                                                                      |
| D-09 | Moved project (5)                              | The "unavailable" status is computed at startup and before a sync. Actions: "Set new path" or "Archive". The sync skips such a project and does not stop                                                                                                                                                                        |
| D-10 | Archive and exclusions (7)                     | An "Archived projects" screen with a "Restore" button, and an "Excluded paths" screen where you can delete an entry. Restore removes the path from the exclusions                                                                                                                                                               |
| D-11 | Hidden instruction (9, 10)                     | Assignments are kept, but it is not included in the build. Physical deletion is not in v1 (P1: purge hidden instructions with confirmation)                                                                                                                                                                                     |
| D-12 | Main variant (11)                              | When the main variant is deleted, the user picks a new main variant in a dialog. Deleting the only variant is not allowed (LIB-025). Changed by the owner ruling of 2026-11-21 (D-125): the main variant cannot be deleted at all — "Make another variant main first" (LIB-024)                                                 |
| D-13 | Empty variant (15)                             | Outputs no text. Counts as assigned for the purpose of including headings. The instruction name does not go into the file                                                                                                                                                                                                       |
| D-14 | Search, tags (12, 32)                          | Search and filters by name in the library and by project. No categories or tags. No dependencies between instructions (13)                                                                                                                                                                                                      |
| D-15 | Markdown (14)                                  | Preview in the editor (pulldown-cmark). Validation gives warnings only (for example, an unclosed code fence) and does not block writing                                                                                                                                                                                         |
| D-16 | History (22–24)                                | A version is created only on a real change. Limit of 200 versions per entity; old ones are deleted. Diff between versions and version rollback are not in the MVP; they are the next stage (P1)                                                                                                                                 |
| D-17 | Backups (25–27)                                | Default limit is 10 per file (1–200). Restoring a file from a backup in the UI is not in the MVP; it is the next stage (P1). A database copy is made before a migration                                                                                                                                                         |
| D-18 | Storage (28–31)                                | SQLite is the only store for library data, and user-managed settings are in it too (6.5). Machine-level app configuration lives in the `config.toml` file (5.8), not in the database. Export and import of the library and default settings in JSON (projects are not transferred) is not in the MVP; it is the next stage (P1) |
| D-19 | Scheme B markers (19)                          | `<!-- EUNOMIASYNC:BEGIN -->` and `<!-- EUNOMIASYNC:END -->`, one pair, each on its own line                                                                                                                                                                                                                                     |
| D-20 | Scheme C (18)                                  | The app does not edit the main file. It shows a ready snippet with a "Copy" button. Names depend on the stem: `<STEM>.project.md` and `<STEM>.local.md`                                                                                                                                                                         |
| D-21 | Text inserted into files                       | English. For "read this file" lines: `If <file> exists in the repository root, read it before starting work and follow its instructions.` This wording has been checked with the target agents and works                                                                                                                        |
| D-22 | Encoding and line breaks                       | UTF-8 without BOM, LF. In scheme B, the file's BOM and line breaks are kept for the content outside the block                                                                                                                                                                                                                   |
| D-23 | Creation marker                                | HTML comment `<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->`. Without an IANA name: `UTC+01:00`                                                                                                                                                                                                      |
| D-24 | Identical content                              | If the new content matches the existing file, ignoring the marker line, the file is not overwritten (status "unchanged"). This setting is on by default                                                                                                                                                                         |
| D-25 | Empty result                                   | If the build is empty but the file is not, a confirmation with a list of such projects is shown before the sync                                                                                                                                                                                                                 |
| D-26 | Platforms (40)                                 | Windows 10 22H2+ (x64, arm64), macOS 12+ (arm64, x64), Ubuntu 22.04+ (x64; arm64 if possible)                                                                                                                                                                                                                                   |
| D-27 | Release (38, 39)                               | GitHub Releases and the Tauri updater with signed updates. Installers are not signed for now (no Authenticode, no notarization)                                                                                                                                                                                                 |
| D-28 | Telemetry (43), portable build (41), tray (36) | Not doing these                                                                                                                                                                                                                                                                                                                 |
| D-29 | New project color (PRJ-010)                    | The least used palette color among active projects; on a tie, chosen by the hash of the project ID (deterministic)                                                                                                                                                                                                              |

## 4. Users and Scope

The app is single-user and local. There are no roles, accounts, or access control; data is protected by OS file system permissions. In v1, there is no built-in way to move the library between computers; JSON export and import (D-18) is the next stage (P1). Syncing the database through cloud folders is not supported: one database must not be opened by two computers at the same time.

## 5. Architecture

### 5.1. Overview

```
Vue 3 UI (WebView)
   │  Tauri IPC: invoke(command) / events
Rust backend
   ├─ commands   — thin layer of Tauri commands, input validation
   ├─ services   — sync, watch, history, dirty-tracker, scan, backup-restore
   ├─ core       — pure logic without IO: builder, schemes, marker parser, hashes
   ├─ store      — SQLite, migrations, repositories
   └─ fs         — scanner, path normalization, backup, atomic write
```

The UI does not access the file system. Rust does all file and database operations.

### 5.2. Modules

| Module (crate)  | Responsibility                                                                                              | Dependencies                                                                                                                                                      |
| --------------- | ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `eunomia-core`  | Domain types, text build, rendering schemes A/B/C, marker parser, build hash, validators                    | only `serde`, `sha2`, `thiserror`, `jiff` (time/zone for the marker, D-035/D-104)                                                                                 |
| `eunomia-store` | DB schema, migrations, repositories, transactions                                                           | `rusqlite`, `core`                                                                                                                                                |
| `eunomia-fs`    | Repository scan, path canonicalization, backup and copy checks, atomic write, retries, target-file watching | `ignore`/`walkdir`, `tempfile`, `dunce`, `notify-debouncer-full`, `unicode-normalization`, `unicode-case-mapping`, `rustix`, `windows-sys` (Windows only), `core` |
| `eunomia-app`   | Services (sync, watch, dirty-tracker), Tauri commands and events, app state                                 | the other crates, `tauri`                                                                                                                                         |
| `ui/`           | Vue app, Pinia stores, i18n, matrix, modal windows                                                          | Vue, Pinia, Tailwind                                                                                                                                              |

Rule: `core` does no input/output and is tested without the file system or the database.

### 5.3. Technologies

| Area             | Choice                                                                                             | Note                                                                                                                   |
| ---------------- | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| Shell            | Tauri 2.x                                                                                          | Versions are pinned when the project starts; versions were not checked for being current as of the spec date           |
| Backend language | Rust stable                                                                                        | `rust-toolchain.toml`, MSRV is pinned                                                                                  |
| DB               | SQLite via `rusqlite` (feature `bundled`) and `rusqlite_migration`                                 | WAL, `foreign_keys=ON`, `busy_timeout=5000`, `synchronous=FULL`                                                        |
| IDs              | `uuid` v7                                                                                          | Text IDs, ordered by time                                                                                              |
| Hashes, diff     | `sha2`, `similar`                                                                                  |                                                                                                                        |
| Time             | `jiff`                                                                                             | Timestamps (DB-002) and the creation marker; built-in IANA time zone support replaces a separate tz crate              |
| Logs             | `tracing`, `tracing-appender`                                                                      | File rotation                                                                                                          |
| Markdown         | `pulldown-cmark`                                                                                   | Preview and warnings. Not used when building files                                                                     |
| Frontend         | Vue 3, TypeScript (strict), Vite, Pinia, vue-i18n, Tailwind CSS                                    |                                                                                                                        |
| UI components    | `@tanstack/vue-virtual`, `sortablejs` (Vue wrapper)                                                | Matrix virtualization, drag-and-drop                                                                                   |
| Tauri plugins    | dialog, updater, single-instance, opener                                                           | No other plugins are used; `dialog` and `opener` are called from Rust only (SEC-001)                                   |
| Tests            | `cargo test`, `proptest`, Vitest, Playwright (against the Vite dev server with a mocked IPC, 15.1) |                                                                                                                        |
| License checks   | `cargo-deny`, `license-checker`                                                                    | App license: dual license, MIT License and Apache License, Version 2.0; dependency licenses must be compatible with it |

| File watching | `notify` + `notify-debouncer-full` | Target files of active projects (5.7) |
| Config file | `serde` + `toml` | Machine-level `config.toml` (5.8); user settings live in SQLite (6.5) |

### 5.4. Execution Threads

- **SYS-001 (P0).** Tauri commands run asynchronously. Blocking operations (DB, files, scan, sync) go to `spawn_blocking`.
- **SYS-002 (P0).** DB access goes through one connection behind a `Mutex`. Each user operation runs in one transaction (NFR-008).
- **SYS-003 (P0).** Scan and sync get a cancellation token and send progress with the `scan://progress` and `sync://progress` events, no more than 10 times per second.
- **SYS-004 (P0).** Parallel syncs are not allowed: the global `sync_running` flag is taken atomically; a second start returns the `E_SYNC_RUNNING` error (MAIN-030).
- **SYS-005 (P0).** A sync first builds all texts from a DB snapshot (one read transaction). Library edits during a sync are allowed; they do not go into the current run and stay "dirty". Commands that change projects (`project_update`, `project_remove`, `project_relink`, and `backup_restore` once it exists in P1) return `E_SYNC_RUNNING` during a sync.
- **SYS-006 (P0).** A second app instance does not start (`single-instance`): the window of the first one is brought to the front.

### 5.5. Data Directory

The data directory is the Tauri 2 default `app_local_data_dir()`, which is `local_data_dir/<bundle identifier>`, with the identifier `pro.yepbro.eunomiasync`. We use the local (not roaming) folder on Windows because the database, backups, and logs belong to this computer and must not be synced through a roaming profile.

| OS      | Path                                                                                        |
| ------- | ------------------------------------------------------------------------------------------- |
| Windows | `%LOCALAPPDATA%/pro.yepbro.eunomiasync/`                                                    |
| macOS   | `~/Library/Application Support/pro.yepbro.eunomiasync/`                                     |
| Linux   | `$XDG_DATA_HOME/pro.yepbro.eunomiasync/` (default `~/.local/share/pro.yepbro.eunomiasync/`) |

Structure:

```
config.toml
db/eunomiasync.sqlite3 (+ -wal, -shm)
db-backups/pre-migration-v<N>-<UTC>.sqlite3
backups/<project_id>__<slug>/<file>.<UTC>.<hash8>.bak
logs/eunomiasync.<date>.log
```

`config.toml` is the app configuration file (5.8). Logs are kept in the `logs/` subfolder of this directory on all OSes. This matches the Tauri `app_log_dir()` default on Windows and Linux; on macOS the Tauri default would be `~/Library/Logs/pro.yepbro.eunomiasync/`, so the app sets the log path itself. On Windows and Linux, Tauri also keeps the WebView data in this directory by default.

Tip: if the user deletes the data, the app starts with an empty DB and says so (question 31); backups are not picked up without the DB.

### 5.6. Shell Security

- **SEC-001 (P0).** The frontend does not get the `fs` and `shell` plugins. `capabilities` grants plugins nothing except one conscious extension — the event-subscription pair `core:event:allow-listen`/`core:event:allow-unlisten`, so the webview can subscribe to §7.10 events (application commands are not capability-gated and need no grant); the updater, dialog, and opener plugins are invoked from Rust (`update_*`, `dialog_pick_folder`, `app_open_data_folder`), never from the webview, and emitting events is Rust-only as well. This is also what lets SEC-003 track paths picked in the system dialog.
- **SEC-002 (P0).** CSP: `default-src 'self'`; external scripts, styles, and images are blocked; DevTools are off in the release build.
- **SEC-003 (P0).** All paths that come from the UI are checked in Rust: a path must be a registered project or must have been picked in the system dialog in this session.

### 5.7. File Watching

The app watches the target files of active, available projects (`AGENTS.md` and `CLAUDE.md`; for scheme C also `<STEM>.project.md`) so that external changes are visible before the next sync overwrites them. Watching is display-only: a watch event never writes a file, and there is no merge and no conflict dialog.

- **SYS-007 (P0).** Watching uses `notify` with `notify-debouncer-full` and a 300 ms debounce. Only the target files above are watched, one watch per file. Watches are added when a project is added or its scheme or file name changes, and removed when a project is archived or becomes unavailable.
- **SYS-008 (P0).** A project is externally modified when its target file on disk does not match the project's current build by the ALG-033 comparison. The state is computed at startup, right after a sync for projects that were written or left unchanged, and after debounced watch events (the file is re-read and re-checked to filter out noise). The state is shown in the matrix column (MAIN-028) and sent as the `state://file-changed` event. It is independent of the dirty flag (DB-021): a project can be dirty, externally modified, or both.
- **SYS-009 (P0).** If the target file is deleted externally, the project shows a "target deleted" hint. The next sync recreates the file in schemes A and C; scheme B skips it with `E_MARKERS_INVALID` as usual.
- **SYS-010 (P0).** On Linux, the watcher respects `fs.inotify.max_user_watches`. When the limit is reached or the event queue overflows, the watcher logs a warning, re-checks all target files once, and shows a non-blocking notice; it does not crash and does not lose the state silently.

### 5.8. Configuration File (TOML)

- **SYS-011 (P0).** Machine-level configuration is the `config.toml` file in the data directory (5.5). It is read at startup, before logging and the database are set up; if it is missing, it is created with default values. The file holds only machine-level settings; user-managed settings stay in the database (6.5), and no library data is ever written to it.
- **SYS-012 (P0).** Keys: `log_level` — one of `trace`, `debug`, `info`, `warn`, `error`; default `info`. Unknown keys are ignored with a warning. A file that cannot be parsed falls back to defaults with a warning, and the app still starts.
- **SYS-013 (P0).** The file is parsed with `serde` + `toml` (decision D-001). YAML is not used anywhere in the app.

## 6. Data Model

### 6.1. General Rules

- **DB-001 (P0).** Storage is one SQLite file in the data directory. On every open, `journal_mode=WAL`, `foreign_keys=ON`, `busy_timeout=5000`, `synchronous=FULL` are set.
- **DB-002 (P0).** IDs are UUIDv7 as TEXT. Time is UTC, an ISO-8601 string with milliseconds, for example `2026-11-17T13:03:22.123Z`.
- **DB-003 (P0).** Booleans are INTEGER 0/1 with CHECK; enums are TEXT with CHECK.
- **DB-004 (P0).** The schema version is stored in `PRAGMA user_version`. Migrations are numbered, go forward only, and each runs in its own transaction. Before a migration, `VACUUM INTO` writes to `db-backups/`. A database with a newer version than the app knows is not opened: the app shows a message suggesting an app update (NFR-009, NFR-010).
- **DB-005 (P0).** Item order is set by an integer `position` starting from 1, with no gaps. Reordering renumbers the affected range in one transaction.
- **DB-006 (P0).** Variant text is up to 262,144 characters.

### 6.2. Schema (DDL)

Below is the conceptually approved version 1 schema. Column names can be adjusted during implementation; constraints and relations cannot.

```sql
CREATE TABLE settings (
  key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  path_key TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 100),
  label TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 6),
  color TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 1000),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','archived')),
  added_via TEXT NOT NULL CHECK (added_via IN ('manual','scan')),
  sort_order INTEGER NOT NULL,
  scheme TEXT NOT NULL CHECK (scheme IN ('A','B','C')),
  base_name TEXT NOT NULL CHECK (base_name IN ('AGENTS.md','CLAUDE.md')),
  extra_local_enabled INTEGER NOT NULL DEFAULT 0 CHECK (extra_local_enabled IN (0,1)),
  extra_local_position TEXT NOT NULL DEFAULT 'end' CHECK (extra_local_position IN ('start','end')),
  extra_project_enabled INTEGER NOT NULL DEFAULT 0 CHECK (extra_project_enabled IN (0,1)),
  extra_project_position TEXT NOT NULL DEFAULT 'end' CHECK (extra_project_position IN ('start','end')),
  last_synced_at TEXT,
  last_synced_hash TEXT,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, archived_at TEXT);

CREATE TABLE project_exclusions (
  id TEXT PRIMARY KEY, path TEXT NOT NULL, path_key TEXT NOT NULL UNIQUE,
  reason TEXT NOT NULL CHECK (reason IN ('removed_scanned','manual')),
  created_at TEXT NOT NULL);

CREATE TABLE library_items (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('instruction','heading')),
  position INTEGER NOT NULL,
  heading_level INTEGER,
  title TEXT NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 200),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','hidden')),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, hidden_at TEXT,
  CHECK ((kind='heading' AND heading_level BETWEEN 2 AND 6)
      OR (kind='instruction' AND heading_level IS NULL)),
  CHECK (kind='instruction' OR state='active'));
CREATE INDEX ix_items_position ON library_items(position);

CREATE TABLE instruction_variants (
  id TEXT PRIMARY KEY,
  instruction_id TEXT NOT NULL REFERENCES library_items(id) ON DELETE RESTRICT,
  name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 100),
  content TEXT NOT NULL DEFAULT '' CHECK (length(content) <= 262144),
  is_main INTEGER NOT NULL DEFAULT 0 CHECK (is_main IN (0,1)),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','deleted')),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT,
  UNIQUE (instruction_id, id));
CREATE UNIQUE INDEX ux_variant_main ON instruction_variants(instruction_id)
  WHERE is_main = 1 AND state = 'active';
CREATE UNIQUE INDEX ux_variant_name ON instruction_variants(instruction_id, name COLLATE NOCASE)
  WHERE state = 'active';

CREATE TABLE project_instructions (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
  instruction_id TEXT NOT NULL REFERENCES library_items(id) ON DELETE RESTRICT,
  variant_id TEXT,
  custom_text TEXT CHECK (custom_text IS NULL OR length(custom_text) <= 262144),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  PRIMARY KEY (project_id, instruction_id),
  FOREIGN KEY (instruction_id, variant_id)
    REFERENCES instruction_variants(instruction_id, id) ON DELETE RESTRICT,
  CHECK ((variant_id IS NOT NULL AND custom_text IS NULL)
      OR (variant_id IS NULL AND custom_text IS NOT NULL)));

CREATE TABLE revisions (
  id TEXT PRIMARY KEY,
  entity_type TEXT NOT NULL CHECK (entity_type IN ('instruction','variant')),
  entity_id TEXT NOT NULL,
  rev_no INTEGER NOT NULL,
  op TEXT NOT NULL CHECK (op IN ('create','update','hide','restore','delete','revert')),
  snapshot TEXT NOT NULL,
  snapshot_hash TEXT NOT NULL,
  reverted_from INTEGER,
  created_at TEXT NOT NULL,
  UNIQUE (entity_type, entity_id, rev_no));

CREATE TABLE sync_runs (
  id TEXT PRIMARY KEY, started_at TEXT NOT NULL, finished_at TEXT,
  status TEXT NOT NULL CHECK (status IN
    ('running','success','partial','aborted','failed','interrupted')),
  projects_total INTEGER NOT NULL DEFAULT 0, projects_written INTEGER NOT NULL DEFAULT 0,
  projects_unchanged INTEGER NOT NULL DEFAULT 0, projects_skipped INTEGER NOT NULL DEFAULT 0,
  projects_failed INTEGER NOT NULL DEFAULT 0, error_code TEXT, details TEXT);

CREATE TABLE sync_run_projects (
  sync_run_id TEXT NOT NULL REFERENCES sync_runs(id) ON DELETE CASCADE,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
  status TEXT NOT NULL CHECK (status IN
    ('written','unchanged','skipped','failed','not_started')),
  target_path TEXT, build_hash TEXT, error_code TEXT, error_message TEXT,
  PRIMARY KEY (sync_run_id, project_id));

CREATE TABLE backup_records (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
  sync_run_id TEXT REFERENCES sync_runs(id) ON DELETE SET NULL,
  source_path TEXT NOT NULL, backup_path TEXT NOT NULL UNIQUE,
  size_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('ok','pruned','missing','discarded')),
  created_at TEXT NOT NULL);

CREATE TABLE action_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL,
  level TEXT NOT NULL CHECK (level IN ('info','warn','error')),
  visibility TEXT NOT NULL CHECK (visibility IN ('user','diagnostic')),
  action TEXT NOT NULL, entity_type TEXT, entity_id TEXT,
  project_id TEXT, sync_run_id TEXT,
  result TEXT NOT NULL CHECK (result IN ('ok','error','skipped')),
  details TEXT);
CREATE INDEX ix_log_ts ON action_log(ts);
```

For `project_instructions`, extra indexes on `instruction_id` and `variant_id` are also created. Section headings have no revisions: renaming and moving them is recorded only in `action_log`.

### 6.3. Invariants

Invariants are enforced by DB constraints and by the service layer. They are checked at startup (`PRAGMA quick_check` and an invariant check) and on demand in the diagnostics section.

- **DB-010.** Each active instruction has at least one active variant and exactly one active main variant.
- **DB-011.** Library item positions go in a row from 1 to N.
- **DB-012.** An assignment points to an active variant of the same instruction or carries per-project custom text instead of a variant reference (MAIN-007). A soft-deleted variant cannot be assigned, and before a variant is deleted, all its assignments are moved to the main variant (LIB-023).
- **DB-013.** A hidden instruction can have assignments, but it is not included in the build (D-11).
- **DB-014.** An archived project keeps its assignments, does not take part in syncs, and is not counted as dirty.
- **DB-015.** `path_key` is unique among active and archived projects. When the user adds the path of an archived project, the app offers to restore it.
- **DB-016.** Variant text and per-project custom text (MAIN-007) cannot contain a line equal to the `<!-- EUNOMIASYNC:BEGIN -->` or `<!-- EUNOMIASYNC:END -->` marker. Saving is rejected with the `E_CONTENT_RESERVED` error.
- **DB-017.** An invariant violation found during a check is written to the log at the error level and shown in diagnostics. There is no automatic data repair.

### 6.4. Deleting and Archiving

| Entity         | Action                     | Result                                                                                                                                                                                                                           |
| -------------- | -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Project        | Archive (`state=archived`) | The project folder is not touched (PRJ-015). Assignments are kept. If the project was found by a scan, its path goes to `project_exclusions` (PRJ-017)                                                                           |
| Instruction    | Hide (`state=hidden`)      | Data, variants, and assignments stay. There is no physical deletion in v1                                                                                                                                                        |
| Variant        | Soft delete                | The assignments of the variant move to the main variant; the confirmation names the number of affected projects (LIB-023). The last variant cannot be deleted (LIB-025), and the main variant cannot be deleted either (LIB-024) |
| Heading        | Physical delete            | The section's instructions stay in place and become part of the previous section                                                                                                                                                 |
| Path exclusion | Physical delete            | The path is found by a scan again                                                                                                                                                                                                |
| Revisions      | Cleanup by limit           | No more than 200 are kept per entity; the latest one is never deleted                                                                                                                                                            |
| Backup         | File deleted by limit      | The record gets the `pruned` status                                                                                                                                                                                              |
| Log and runs   | Cleanup by limits          | 10,000 entries or 180 days for the log; the last 500 runs                                                                                                                                                                        |

### 6.5. Settings

All keys are in the `settings` table (values in JSON). Default values are listed in the table below.

| Key                                     | Type               | Default                                                              | Range                    |
| --------------------------------------- | ------------------ | -------------------------------------------------------------------- | ------------------------ |
| `default.scheme`                        | A, B, C            | A                                                                    |                          |
| `default.base_name`                     | list               | `AGENTS.md`                                                          | `AGENTS.md`, `CLAUDE.md` |
| `default.extra_local` / `extra_project` | enabled + position | off, `end`                                                           | `start`, `end`           |
| `backup.retention`                      | integer            | 10                                                                   | 1–200                    |
| `sync.skip_identical`                   | boolean            | true                                                                 |                          |
| `scan.max_depth`                        | integer            | 6                                                                    | 1–20                     |
| `scan.exclude_names`                    | list of strings    | `node_modules`, `vendor`, `.git`, `dist`, `build`, `target`, `.venv` |                          |
| `history.max_revisions`                 | integer            | 200                                                                  | 10–1000                  |
| `log.max_entries`, `log.max_days`       | integers           | 10000, 180                                                           | 1000–100000, 7–3650      |
| `ui.language`                           | string             | `en`                                                                 |                          |
| `update.check_on_start`                 | boolean            | true                                                                 |                          |

Changing `default.*` does not affect existing projects (DEF-005).

### 6.6. Build Hash and the Dirty Flag

- **DB-020 (P0).** Project build hash: `sha256('v1' + scheme + target path + extra-file flags and positions + build text without the creation marker)`.
- **DB-021 (P0).** A project is dirty if it is active and either `last_synced_hash` is empty while the build text is not empty, or the hash differs from the current one. A project with no assignments that has never been synced is not counted as dirty.
- **DB-022 (P0).** Hashes of all active projects are recalculated in the background 150 ms after each committed transaction, at startup, and after a sync. The app sends the `state://dirty` event with the list of dirty projects (OUT-013, MAIN-028).
- **DB-023 (P0).** Changing the scheme or the file name makes the project dirty. The old file with the previous name is not deleted. Project settings show a hint about this.
- **DB-024 (P0).** A project's `last_synced_hash` and `last_synced_at` are updated only after a successful write or the `unchanged` status for this project (OUT-020).

### 6.7. Paths

- **DB-030 (P0).** A project path is converted to canonical form (`dunce::canonicalize`, without the Windows extended-length path prefix); the stored path is the verbatim canonicalize output. The `path_key` key: `/` separators, no trailing separator; on Windows and macOS also Unicode NFC and **simple** case folding — the filesystem-faithful rule: 1:1 mappings only, no ligature unfolding (`ﬁ` stays distinct from `fi`), while 1:1 mappings do fold (`ſ` folds to `s` — NTFS/APFS behave the same); on Windows this simple folding is the faithful stand-in for the NTFS `$UpCase` comparison; on Linux, case is kept and no normalization is applied. NFC and folding apply to the key only, never to the stored path (owner ruling, 2026-11-19).
- **DB-031 (P0).** Two paths that lead to the same folder through a symlink count as one project.
- **DB-032 (P0).** Network and removable drives are allowed. Unavailability is checked at startup, when the window opens, and before a sync (PRJ-004, D-09).

### 6.8. Project Palette and Label

- **DB-040.** The palette is fixed: 12 colors (`red`, `orange`, `amber`, `lime`, `green`, `teal`, `cyan`, `blue`, `indigo`, `violet`, `pink`, `slate`). Each one passes a label text contrast check of at least 4.5:1 in the light and dark themes.
- **DB-041.** The color of a new project is chosen by D-29. Color is not the only sign of a project: the label is always shown next to it, and the name is shown in a tooltip (NFR-020).
- **DB-042.** The default label is the first up to 6 characters of the name, in upper case. An empty label is not allowed.

### 6.9. Validation Rules

| Field                              | Rule                                                               | Limits                                                                                       |
| ---------------------------------- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| Project path                       | Exists, is a folder with `.git` (folder or file), is readable      | Write access is checked when adding (warning) and before a sync (error)                      |
| Project name                       | 1–100 characters after trimming spaces                             | Duplicates are allowed with a warning                                                        |
| Label                              | 1–6 visible characters, case is kept                               |                                                                                              |
| Description                        | Up to 1000 characters                                              |                                                                                              |
| Color                              | A value from the palette                                           |                                                                                              |
| Main instruction name              | 1–200 characters, one line                                         | Unique among visible instructions, case-insensitive                                          |
| Variant name                       | 1–100 characters                                                   | Unique within the instruction, case-insensitive. The main variant is named `Main` by default |
| Variant text                       | Markdown as plain text, up to 262,144 characters, empty is allowed | CRLF is changed to LF on save; `NUL` and reserved markers are rejected                       |
| Per-project custom text (MAIN-007) | Markdown as plain text, up to 262,144 characters, empty is allowed | Same rules as variant text; empty behaves like an empty variant (D-13)                       |
| Heading name                       | 1–200 characters, one line                                         | Level 2–6                                                                                    |
| File name                          | A value from the list                                              | Path and extension are not typed in                                                          |
| Backup limit                       | Integer 1–200                                                      | Zero is not allowed: writing without a backup is forbidden (BKP-005)                         |

A name field also rejects `NUL` — project name, instruction name, variant name, heading name — an extension stricter than the rows above, which ban it only in variant text.

Validation errors are shown at the field or in a general message. An entity is never saved partly: the whole operation runs in one transaction.

## 7. Functional Requirements

IDs are taken from Product Requirements v0.3. The tables give a refined wording and an acceptance criterion (AC). Requirements not listed here apply as worded in the PR.

### 7.1. Projects

| ID                     | P   | Requirement                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Acceptance criterion                                                                                                                     |
| ---------------------- | --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| PRJ-001, 003, 004      | P0  | The "Add project" button opens the system folder picker. If the folder is inside a repository, the app offers the repository root. If the path is already registered, a message is shown; if the project is archived, the app offers to restore it. A folder without `.git` is rejected with the `E_NOT_A_REPO` error                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Four cases are tested: repository root, subfolder, adding again, non-Git folder                                                          |
| PRJ-002, 005, 006      | P0  | Scan of a selected folder or drive. The app shows the number of folders checked, repositories found, and the current path, and there is a "Cancel" button. The modal also shows the active scan configuration (depth and exclusions from the settings) and an indeterminate progress bar. The result has three groups: "New" (with checkboxes, checked by default), "Already added", "Skipped" (with a reason: excluded, submodule or worktree, no access, non-UTF-8 path, archived or in exclusions); each skipped row also carries a short hint on how to fix it (for example, "os error 13 — permission denied; fix the rights and rescan"). From the result view, "Scan again" starts a fresh pass. "Add selected" is enabled only when at least one row is checked and adds the checked ones in one transaction                                                          | Scanning the same folder again creates no duplicates. Canceling a scan adds no projects                                                  |
| PRJ-007                | P0  | A folder access error goes to "Skipped" with the OS code, and the scan continues                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | A folder without read rights does not stop the scan of the others                                                                        |
| NFR-015                | P0  | The scan runs in the background, and the window stays responsive. Closing the scan window means cancel                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | During a scan of 100,000 folders, the UI responds to input                                                                               |
| PRJ-008, 009, 011, 021 | P0  | "Project settings" modal window, opened by the pencil icon in the project's column header. Name, label, color, and description are editable. Path and how the project was added are visible but not editable. The time of the last successful sync is shown in the local time zone. The label is previewed live as a chip; the description carries a `0 / 1000` counter; colors are radio swatches; the extra-file position selects are disabled while the include flag is off, and the derived `<STEM>.local.md` / `<STEM>.project.md` names follow the base name. The scheme-conditional blocks: extra files only in scheme A; a marker hint with "Copy marker block" in scheme B; the ready snippet with "Copy" in scheme C. Copy buttons report "Copied" / "Copy failed" for 1.5 s. Enter saves; saving updates the column header (color dot, label, tooltips) right away | Fields are saved in a transaction. The path cannot be changed                                                                            |
| PRJ-010                | P0  | If no color is chosen, it is assigned by rule D-29                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | For 12 projects in a row, colors do not repeat                                                                                           |
| PRJ-012                | P0  | A project has its own scheme, file name, and extra files. For a new project, values are copied from the default settings                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Changing the default settings does not change existing projects                                                                          |
| PRJ-013…018            | P0  | "Archive project" needs confirmation by typing the project name. The dialog explains what happens: the folder is not touched, assignments are kept, the project does not take part in syncs. A project found by a scan gets an entry in the exclusions. The confirm button stays disabled until the typed name matches exactly; Enter in the field confirms; the text names the way back through "Archived projects". Screens "Archived projects" ("Restore" button) and "Excluded paths" ("Remove" button): archived rows show the color dot, name, path, "Archived <date>", and the origin (manual or found by scan); excluded rows show the path, a human-readable reason badge ("Removed by archiving" / "Added manually"), and the date. After a row is deleted, focus stays inside the dialog                                                                           | Archiving does not change files on disk. Restoring brings the project back with its assignments and removes its path from the exclusions |
| D-09                   | P0  | An unavailable project (folder moved, deleted, or drive disconnected) is marked with an icon and text. Actions: "Locate…" (new path with a `.git` check) and "Archive". The sync skips such a project                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Moving the folder: the project stays in the list, and the sync finishes without errors for the others                                    |
| PRJ-019, 020           | P0  | The project order is changed by dragging a column and through the "Move left / Move right" menu (keyboard alternative). The order is saved right away                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | The order is kept after a restart                                                                                                        |

### 7.2. Settings Window and Default Settings

| ID          | P   | Requirement                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Acceptance criterion                                                               |
| ----------- | --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| DEF-001…004 | P0  | "Settings" modal window with sections: "Defaults for new projects" (scheme, file name, extra files with position for scheme A), "Backups" (limit), "Scanning" (depth, exclusions), "History and logs" (limits), "Updates and About". There is no "Save" button: every control applies on change (a status line says "Settings are applied immediately"). A number outside its 6.5 range shows an inline error, and the previous value stays until it is fixed. The exclusions list is edited as chips — add with the button or Enter, remove with the chip's cross; empty values and duplicates are rejected. Reopening the window syncs the controls with the stored settings. The extra-files block is visible only for scheme A. The "Updates and About" section holds the link buttons "Archived projects…" and "Excluded paths…" (D-10) | Each setting applies without a restart                                             |
| DEF-005     | P0  | Changing the defaults does not affect existing projects                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | AC same as PRJ-012                                                                 |
| D-18        | P1  | Export and import of the library and settings in JSON with a format version. Projects, assignments, and history are not transferred. Import is a merge: items with a matching name are not overwritten but get a suffix; a summary is shown before the import                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Export and import on another computer reproduce the structure, variants, and order |

### 7.3. Instruction Library

| ID                          | P   | Requirement                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Acceptance criterion                                                                                                   |
| --------------------------- | --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| LIB-001, 008                | P0  | The list shows the name, number of variants, creation and change dates, and colored labels of the projects where at least one variant is assigned                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Labels match the assignments                                                                                           |
| LIB-002, 009…016            | P0  | Create dialog: main name and text of the main variant (named `Main`). Editor: multi-line field with a monospace font, "Preview" tab (pulldown-cmark), character counter, Markdown warnings (for example, an unclosed code block) that do not block saving. The editor has separate "Instruction name" and "Variant name" fields; validation is inline and live (duplicate instruction names case-insensitive among visible ones, variant name 1–100 and unique within the instruction, reserved markers from DB-016 mark the field `aria-invalid`). The text field shows a `0 / 262,144` counter; Enter in a text field saves. "Edit" and "Preview" are ARIA tabs (arrows, Home, End); closing with unsaved changes offers "Keep editing" or "Discard". Variants are a list on the left, shown even when there is only one variant; the main variant is always first. An empty variant is marked with an icon and a dimmed style (LIB-014) and can be assigned (LIB-015)                         | Validation per table 6.9. An empty variant can be assigned                                                             |
| LIB-003, 004                | P0  | Instruction order is changed by drag-and-drop and saved right away                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | The order is kept after a restart                                                                                      |
| LIB-030, 031, 033           | P0  | Headings of levels 2–6 can be added, renamed, deleted, and moved. There is one structure for all projects. A "+" button in the library column header adds a root `##` section at the end of the tree; each heading row has "+" (a child, level + 1), a pencil (rename), and a trash (delete) buttons. At level 6, "+" is disabled with an explanation. Add and rename share one dialog with 1–200 name validation. Deleting a heading does not delete the section's instructions: the dialog says they will move to the previous section                                                                                                                                                                                                                                                                                                                                                                                                                                                         | The heading is deleted; the instructions stay                                                                          |
| LIB-032, 034, MAIN-018, 019 | P0  | Moving a heading moves its whole section (the heading and all items up to the next heading of the same or a higher level). Drag-and-drop outcomes: an instruction dropped on a row goes right after it; an instruction dropped on a heading goes to the end of that section; a section dropped on a heading becomes the last child of the target; a section dropped on the library column header becomes the first root section. In the last two cases the levels of the whole branch shift automatically to the target level + 1 (or to 2 for the header), and the move is rejected with `E_VALIDATION` if any new level would leave 2–6 (ALG-062). A drop into the branch's own range is blocked. The "Change level" and "Shift branch level ±1" commands remain for manual re-leveling outside a move (allowed only if levels stay within 2–6). Skipping a level (for example, `##` then `####`) is allowed and marked with a warning icon. Single instructions can be moved between sections | Moving a branch keeps the order of nested items. The order in the final file equals the order in the library (LIB-035) |
| LIB-005…007, 012            | P0  | "Hide" with confirmation. The "Show hidden" filter and the "Restore" command bring the instruction back with its assignments (D-11)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | A hidden instruction is not in the file after the next sync                                                            |
| LIB-017…022                 | P0  | "Add variant" (a copy of the main one or empty). One variant can be assigned to several projects. "Make main" changes the main variant. An assignment can be removed without deleting the variant. A project can have only one variant of an instruction (D-02). Variant actions live in the variant's list row: "Make main" (disabled for the current main) and Delete (disabled for the last variant, LIB-025, and for the main variant, LIB-024). A new variant starts as a copy of the main one or empty, with the first free name "Variant N"                                                                                                                                                                                                                                                                                                                                                                                                                                               | Changing the variant in one project does not change other projects (MAIN-005)                                          |
| LIB-023, 024, 025           | P0  | Deleting a variant is confirmed by a dialog that names the number of affected projects; the assignments of the deleted variant move to the main variant. The last variant cannot be deleted (LIB-025). The main variant cannot be deleted either: the Delete button is disabled with the hint "Make another variant main first" (LIB-024). Everything runs in one transaction                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | After the deletion, no assignment disappeared silently                                                                 |
| MAIN-007, LIB-018           | P0  | The "Edit for this project" command in the matrix opens the per-project custom editor. The cell's "+" shows the editor with a text field only; saving stores per-project custom text in the assignment itself, bypassing the variant model (6.2). While a cell carries custom text, its select is disabled and shows "Custom", the checkbox stays active, and the cell gets edit and delete icons; deletion is confirmed (UI-010) and assigns the main variant back to the cell. Bulk operations ("Clear column", "Assign all visible", "Assign to projects…") do not touch cells with custom text. In a build, the custom text is used as the cell's text (9.2). Replaces the earlier fork-variant flow (a variant named after the project) by the owner ruling of 2026-11-21 (D-125)                                                                                                                                                                                                           | The shared variant does not change when edited from a project context; the built file uses the custom text             |
| LIB-026, 027                | P0  | The app records versions of instructions and variants. A version is created only on a real change of the snapshot (D-16). The instruction "History" panel shows the list of versions (time, operation, entity). A deleted variant can be brought back with the "Restore variant" command                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Each real change adds one version; saving without changes adds none                                                    |
| LIB-028, 029                | P1  | In the "History" panel: view a version, diff with the previous or any selected version, "Restore this version". Restoring creates a new version with `op=revert` and `reverted_from`; old versions are not erased                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | After a restore, the history has a new entry and all the earlier ones                                                  |

An instruction version snapshot stores the name and state (active or hidden). A variant version snapshot stores the name and text. The main-variant flag and the library order are not part of snapshots.

### 7.4. Main Window: Assignment Matrix

| ID                         | P   | Requirement                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Acceptance criterion                                                                                       |
| -------------------------- | --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| MAIN-001…005               | P0  | Rows are library items shown as a tree; columns are active projects in the saved order. A click or Space turns the main variant assignment on and off. An assigned cell carries a checkbox and an inline select with the chosen variant; an unassigned cell shows no select. Every cell carries the "+" affordance for per-project custom text (MAIN-007). Enter opens the select's variant picker. Headings have no cells                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | All cell states (11.2) can be told apart without relying on color (NFR-020)                                |
| MAIN-006, 008, 009         | P0  | The icon next to the name opens the editor. A tooltip with the text (first 600 characters, 400 ms delay): one reusable tooltip element that clamps to the viewport, truncates the text with an ellipsis, hides on mouse leave, focus leave, scroll, resize, and Escape, and also opens on keyboard focus in the name cell. An empty instruction has a special dashed icon, a dimmed name, and an sr-only "(empty instruction)"; it gets no text tooltip                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | An empty instruction can be assigned                                                                       |
| MAIN-010, NFR-017, NFR-022 | P0  | Virtualization of rows and columns (`@tanstack/vue-virtual`). The first column and the header row are pinned. Minimum row height is 28 px. Arrow key navigation. Empty states: no projects, no instructions, no search results                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | A matrix of 500 projects by 1000 rows scrolls smoothly (targets in section 12)                             |
| D-14, UI                   | P0  | Search by instruction name and a project filter above the matrix, and a "Show hidden" toggle that shows the current number of hidden instructions. While the search is active, section collapse states are ignored: a heading row is visible if and only if one of its descendants matches; clearing the search restores the previous collapse state (UI-004)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | The filter does not change data                                                                            |
| UI (bulk operations)       | P0  | Rows are selected with per-row checkboxes; the checkbox column ends in a header "Select all" checkbox (indeterminate when the selection is partial). With at least one row selected, the toolbar shows an "N rows selected" chip and the "Assign to projects…" button. The command opens a dialog with multi-select of projects. If one instruction is selected, you can pick a variant. A column has an "Assign all visible" and "Clear column" menu. If 10 or more assignments are affected, a confirmation with the number of changes is shown. Bulk operations do not touch cells with per-project custom text (MAIN-007). Each bulk action is one transaction                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | 100 assignments are applied in one operation; on an error, nothing is applied                              |
| MAIN-011, 029, OUT-010     | P0  | Any change is saved to the DB right away; project files are not touched                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Files do not change until a sync is run                                                                    |
| MAIN-012…014               | P0  | "Hide" from the matrix is the eye button in the row; its confirmation says the instruction is excluded from built files, assignments are kept, and files change only after the next sync. Restore is the same button, without confirmation, when "Show hidden" is on; a hidden row shows a dashed "hidden" chip and a dimmed name. Files saved earlier do not change until a sync                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |                                                                                                            |
| MAIN-015…023               | P0  | Adding, renaming, deleting, and moving headings and instructions between sections. The tree shows nesting with indents. A heading appears in a project file by rule ALG-020                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | See section 9.2                                                                                            |
| MAIN-024…027, 030, 031     | P0  | The sync panel is pinned outside the table's scroll area and is always visible. From the left: the "Run sync" button, the progress area, the time of the last successful full sync (local time and time zone), and a counter pill of projects with unsynced changes with singular and plural wording (MAIN-026). Clicking the counter opens the "Unsynced changes" dialog: one row per dirty project with the project and its target file (for scheme C, `<STEM>.project.md`), an "Everything is synced" empty state, and a "Run sync" button to start the run from the dialog. During a sync, the button is disabled, and an animation (off under "reduce motion", UI-022) and a live "k of N" region are shown. When it finishes, a summary opens with the run's outcome badge ("success" or "partial" with a short explanation) and "Written", "Unchanged", "Skipped", "Failed" counter chips, with project, path, and reason; skip and error rows carry their `E_*` code and a short what-to-do hint (UI-011). After the run, per-project times update only for "written" and "unchanged" (OUT-020); skipped and failed projects keep their previous sync state, and the dirty flag follows DB-021 | The button is visible at any scroll position and window size. A second start while running is not possible |
| MAIN-028                   | P0  | A project column header shows its state as an icon with a visible short text: unavailable, dirty ("unsynced changes"), externally modified, or target deleted (5.7). A clean column shows the short time of its last sync as visible text (for example, "19 Nov, 09:14"). The full project name is in the header tooltip and in an sr-only span; the 1–6 character label sits next to the color dot (DB-041, DB-042)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | States can be told apart without color                                                                     |

### 7.5. Building and Writing Files

The algorithms are described in section 9. This section has the requirements and acceptance.

| ID                    | P   | Requirement                                                                                                                                                                                    | Acceptance criterion                                                                                                 |
| --------------------- | --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| OUT-001…003, 004      | P0  | For each project, the build is made from the assigned variants, in the order of the shared library, by the project's scheme. The file name is `AGENTS.md` or `CLAUDE.md`                       | The order in the file equals the order in the library (DoD 5)                                                        |
| OUT-005, 015          | P0  | An existing target file is fully replaced (except the area outside the markers in scheme B). A missing file is created (schemes A and C)                                                       | Content added to the file by hand disappears after a sync (scheme A), and a copy stays in the backup                 |
| OUT-006, 019, BKP-005 | P0  | Before writing starts, backups of the existing target files are created and checked for all projects. An error in even one backup cancels writing in all projects of the run                   | Simulated backup error: no file is changed, and there is a detailed error                                            |
| OUT-007, 008          | P0  | The write is atomic (section 9.5). Before writing, folder availability and write rights are checked                                                                                            | Killing the process during a write does not leave a partly written file                                              |
| OUT-009, MAIN-031     | P0  | After a run, a summary by project and file is shown, with reasons for skips and errors                                                                                                         | Each project in the summary has exactly one status                                                                   |
| OUT-011, 012          | P0  | Files change only with the "Run sync" command, which covers all active projects                                                                                                                | No library operation changes files                                                                                   |
| OUT-013, DB-021       | P0  | The app detects unsynced changes by the build hash and warns the user                                                                                                                          | Any edit that affects a project's build makes it dirty; an edit to a hidden instruction with no assignments does not |
| OUT-014               | P0  | External edits to target files are detected and displayed (5.7): the app never merges them and never writes a file because of a watch event. The next sync overwrites them as before (OUT-015) | An external edit flips the state without any dialog; a sync still overwrites the file and makes a backup             |
| OUT-016…018, D-23     | P0  | A creation marker at the end of each built content, local time with time zone. In scheme B, the marker is at the end of the managed block, before the closing marker                           | The format matches the template in section 9.3                                                                       |
| D-24                  | P0  | A file that did not change compared to the new build (ignoring the marker line) is not overwritten and not backed up; status `unchanged`. The project then counts as synced                    | A second sync with no edits changes no files and creates no backups                                                  |
| OUT-020               | P0  | Per project, the time and hash of the last sync (`projects.last_synced_at`, `last_synced_hash`) are updated only for the `written` and `unchanged` statuses (DB-024)                           | A skipped or failed project keeps its previous sync state (MAIN-024, S7)                                             |

**Scheme A.** Implements OUT-A-001…004 as worded in D-01.

- For `<STEM>.local.md` and `<STEM>.project.md`, the project has an "Include" flag and a "Start" or "End" position.
- When included, the line from D-21 with the file name is added to the main file. The app does not create, read, or change these files.
- If both lines are in the same position, the line for `.project.md` goes first, then the one for `.local.md` (local rules are read last and take priority).
- Project settings show a hint that `.local.md` is usually not added to Git. The app does not edit `.gitignore`.

**Scheme B.** Implements OUT-B-001…006.

- The target file must exist and contain exactly one pair of markers from D-19. Each marker is on its own line (spaces at the ends are allowed).
- The app does not create the file and does not insert the markers by itself (otherwise principle G5 is broken). The "Copy marker block" button copies a ready block to paste by hand.
- No markers, several of them, reversed order, or one of them missing: the project gets the `skipped` status with the `E_MARKERS_INVALID` error, nothing is written, and the other projects continue.
- Content outside the markers, including the file's BOM and line breaks, is kept byte for byte.

**Scheme C.** Implements OUT-C-001…004 as worded in D-20.

- Content is written to `<STEM>.project.md` in the project root and fully overwritten.
- Project settings show a ready snippet for the main file, for example: `If AGENTS.project.md exists in the repository root, read it before starting work and follow its instructions.` The file name in it comes from the settings. The typo in the PR example (mentioning `AGENTS.local.md`) is fixed.
- The app does not open the main file.

### 7.6. Backups and Restore

| ID                     | P   | Requirement                                                                                                                                                                                                                                                                                               | Acceptance criterion                                              |
| ---------------------- | --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| BKP-001, 002, 008, 011 | P0  | Backups are in the data directory: `backups/<project_id>__<slug>/<file name>.<UTC YYYYMMDDTHHMMSSmmmZ>.<hash8>.bak`. `slug` is the cleaned project name, up to 40 characters; the main key is `project_id`. The full file is copied before writing                                                        | The backup name shows the project, file, and time                 |
| BKP-010                | P0  | A copy counts as created after a check: size and SHA-256 match the source file. If the SHA-256 of the source file equals the SHA-256 of the last backup of this file, no new backup is created, and the record points to the existing one                                                                 | A replaced damaged backup is caught by the check                  |
| BKP-003, 006           | P0  | Up to N copies are kept per "project and file" pair (10 by default). The oldest extra ones are deleted after the run finishes, and only if the new copies are checked                                                                                                                                     | With N=3, three copies remain after five syncs                    |
| BKP-004                | P0  | In project settings, the "Backups" tab shows the date, file, size, and sync run number                                                                                                                                                                                                                    |                                                                   |
| BKP-005, 009           | P0  | A backup error stops writing in all projects. The message includes: project name, source file path, backups folder path, OS code or error type, an explanation of the cause, and a recommendation. There is no forced write without a backup. Copies created in this run are deleted (status `discarded`) | The message contains all the listed fields                        |
| BKP-007                | P1  | Restore from a backup in the UI: pick a copy, view the diff with the current file, confirm. Before restoring, a backup of the current file is made by the same rules, and the write is atomic. Restoring does not change the project's sync status: the file will be overwritten again on the next sync   | The file matches the copy byte for byte, and there is a log entry |

### 7.7. Action Log

| ID          | P   | Requirement                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ----------- | --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| LOG-001…004 | P0  | The `action_log` records: adding, changing, archiving, and restoring projects; changing settings; creating, editing, hiding, and restoring instructions and variants; changing assignments (one entry with the number of changes for a bulk operation); heading operations; sync runs and their results per project; backup creation; errors. An entry has the time, operation type, object, result, and, for file writes, the project and path                                                                                                            |
| LOG-005     | P0  | The `visibility` field: `user` is shown in the "Activity" window; `diagnostic` only in the file log and in diagnostics                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| LOG-006     | P0  | Log limits come from settings (10,000 entries or 180 days by default); cleanup runs at startup and once a day                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| LOG-007     | P0  | The time of the last successful sync is stored for each project (`projects.last_synced_at`) and overall (the latest of the runs with `success` status; for `partial`, by the rule in section 9.4)                                                                                                                                                                                                                                                                                                                                                          |
| UI          | P0  | "Activity" window with filters by date, project, type, and result. Type groups: Projects, Library, Assignments, Sync, Backups; date presets: All time, Last 7 days, Today; results are shown as ok / skipped / error chips. File-write entries show the path under the project; a bulk operation is one entry with the number of changes (LOG-001). Entries are newest-first, the filters survive reopening, and a hint names the log limits (LOG-006). The `tracing` file log is written with rotation; instruction and file content never goes into logs |

### 7.8. Help

- **HLP-001…004 (P0).** "Help" modal window with a list of articles on the left and the text on the right. Text search runs on the client; matches are highlighted in the list and in the text. The search shows an "N of M articles match" counter (live-announced); with no matches, the list shows the empty state (UI-012) and the open article stays. The selected article survives reopening; the search resets when the window opens. Minimum set of articles: quick start, projects and scanning, instructions and variants, schemes A/B/C, sync and dirty, backups, history, limits and common errors.
- **HLP-005 (P0).** The FAQ is stored in the app resources as Markdown in English and is updated together with the app.

### 7.9. Updates and Product Info

- **UPD-001, 002 (P0).** Update check at startup (if turned on in settings) and with the "Check for updates" button in the "About" window; the same-named button in Settings opens About and starts the check. The result (version chip, change list, "Download and install") and the install confirmation are shown in About. After an install, About shows "Installed — restart to apply". Closing the window cancels a running check. The version and the list of changes from the release metadata are shown.
- **UPD-003, 006, D-27 (P0).** `tauri-plugin-updater` is used; artifact signatures are checked with a public key built into the app. Download and install start only after confirmation. Installing is not available during a sync.
- **UPD-004 (P0).** Rolling back an installed version is not automated. If the new version could not run a migration, the app does not change the database and says that the previous version must be installed, pointing to the `db-backups/pre-migration-*` copy.
- **UPD-005 (P0).** "About" window: version, app identifier (`pro.yepbro.eunomiasync`), author (Den Brooks, denis@yepbro.pro), license (dual license: MIT License and Apache License, Version 2.0), repository link (https://github.com/yepbro/EunomiaSync). The brand line carries the version chip; an "Update channel: stable — signed updates via the Tauri updater (D-27)" line is shown, and the dependency licenses are a collapsible section (REL-004).
- **NFR-013 (P0).** The app's only network request is the update check. It contains no project or instruction data and runs in a separate module.

### 7.10. IPC Contract

All commands return `Result<T, AppError>`, where `AppError` has `code`, `message_key`, `params`, and optional `details`. The UI shows localized text by `message_key` (NFR-006, NFR-018).

| Module      | Commands                                                                                                                                                                                                                                                                                  |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Projects    | `projects_list`, `project_add_manual`, `scan_start`, `scan_cancel`, `scan_commit`, `project_update`, `project_archive`, `project_restore`, `project_relink`, `projects_reorder`, `exclusions_list`, `exclusion_remove`                                                                    |
| Settings    | `settings_get`, `settings_set`; P1: `library_export`, `library_import`                                                                                                                                                                                                                    |
| Library     | `library_get`, `instruction_create`, `instruction_update`, `instruction_hide`, `instruction_restore`, `variant_create`, `variant_update`, `variant_delete`, `variant_restore`, `variant_set_main`, `markdown_preview`, `heading_create`, `heading_update`, `heading_delete`, `items_move` |
| Assignments | `assignment_set`, `assignment_set_custom`, `assignments_bulk`                                                                                                                                                                                                                             |
| Sync        | `sync_status`, `sync_run`, `sync_cancel`, `sync_last_result`                                                                                                                                                                                                                              |
| Backups     | `backups_list`; P1: `backup_diff`, `backup_restore`                                                                                                                                                                                                                                       |
| History     | `revisions_list`, `revision_get`, `activity_list`; P1: `revision_diff`, `revision_restore`                                                                                                                                                                                                |
| System      | `update_check`, `update_install`, `app_info`, `app_open_data_folder`, `dialog_pick_folder`, `db_verify`, `db_backups_list`, `db_restore_backup`, `db_recreate`                                                                                                                            |

Events: `scan://progress`, `sync://progress`, `state://dirty`, `state://project-availability`, `state://file-changed`.

Command notes: `dialog_pick_folder` opens the system folder picker from Rust and returns the picked path (nothing on cancel); adding a project by hand and a scan start with it, and it feeds the SEC-003 session allowlist. `markdown_preview` renders Markdown into sanitized HTML (SEC-005) and returns Markdown warnings (D-15). `assignment_set_custom` sets per-project custom text in a cell; clearing it assigns the main variant back (MAIN-007). `variant_restore` brings back a soft-deleted variant (7.3). `db_backups_list`, `db_restore_backup`, and `db_recreate` back the recovery screen for `E_DB_CORRUPT` (section 10): the first lists the copies in `db-backups/`; the second keeps the damaged database as `db-backups/pre-recovery-<UTC>.sqlite3`, replaces it with the chosen copy, and reopens it; the third does the same but starts with an empty database. `app_open_data_folder` shows the data directory in the OS file manager.

## 8. User Scenarios

Each scenario lists the main flow (Main), alternatives and errors (Alt), and the success criterion (Success).

**S1. Adding a project by hand.** Main: "Add project", pick a folder, `.git` check, duplicate check, create the entry with default settings and a color by D-29, "Project settings" opens. Alt: the path already exists, the project is archived, not Git, no read rights. Success: the project is visible as a matrix column.

**S2. Scan.** Main: "Scan folder", pick the root, progress, result list, selection, "Add selected". Alt: cancel, unavailable folders, scanning again, a found project is in the exclusions. Success: no duplicates, skipped items are marked with reasons.

**S3. Creating an instruction and assigning it.** Main: "New instruction", name and text, pick a section, save, check the project cells. Alt: validation errors, duplicate name, reserved marker in the text. Success: the assignment is in the DB right away, files are not changed, the project became dirty.

**S4. Choosing a variant.** Main: open the variant list in a cell, pick a variant. Alt: the variant is empty, the variant was deleted by another action (the list refreshes). Success: other projects are not affected.

**S5. Global edit and edit for a project.** Main: "Edit" from the library changes the variant for everyone; "Edit for this project" from the matrix opens the per-project custom editor and stores the text in the cell's assignment (section 7.3). Success: the shared variant is not changed when edited from a project context, and the built file uses the custom text.

**S6. Adding an instruction to several projects at once.** Main: select rows, "Assign to projects…", check the projects, confirm. Alt: a project that is already assigned is not duplicated; if a project already has another variant of this instruction, the variant is not replaced, and the project is shown in an "already assigned" warning. Success: one transaction.

**S7. Sync.** Main: "Run sync"; build from a snapshot; preflight; backups; atomic write; summary (section 9.4). Alt: unavailable projects and scheme B errors are skipped; a backup error stops writing in all projects; a write error in one project marks only that project. Success: each project has one status; times are updated per OUT-020.

**S8. History and restoring a version (P1).** Main: "History" panel, pick a version, diff, "Restore this version". Success: a new `revert` version is created; earlier versions are kept.

**S9. Warning about unsynced changes.** Main: 150 ms after an edit, the panel shows "N projects have unsynced changes"; it disappears after a sync. Success: the counter matches the number of dirty projects.

**S10. Backup failure.** Main: the sync reaches the backups, one is not created, writing does not start, a detailed error is shown, the created copies are discarded, and the log has a cancel entry. Success: no project file is changed.

**S11. Restoring a file from a backup (P1).** Main: "Project settings, Backups tab", pick a copy, diff, confirm, back up the current file, atomic write of the copy. Alt: the copy is missing on disk (status `missing`), the project is unavailable. Success: the file equals the copy byte for byte.

## 9. Algorithms

### 9.1. Data Snapshot

- **ALG-001 (P0).** The build works with an immutable snapshot read in one read transaction: active projects in `sort_order` order; all library items by `position`; active variants; assignments. The snapshot is passed to `eunomia-core`, which does not access the DB or the file system.
- **ALG-002 (P0).** Variant text normalization: CRLF and a single CR are replaced with LF, the BOM is removed, and leading empty lines and all trailing whitespace of the block are dropped. Spaces inside the text are not touched (they matter in Markdown).

### 9.2. Building the Project Body

```
function build_body(items, assignments):
  included = set of instruction I where
      I.state = active AND assignments has an entry for I
  blocks = []
  for k, item in enumerate(items):
    if item is heading with level L:
      end = first index j > k where items[j] is a heading AND level <= L
            (otherwise len(items))
      if any item in items[k+1 .. end) belongs to included:
          blocks.append(repeat('#', L) + ' ' + item.title)
    else if item in included:
      text = normalize(text_of(assignment))   // custom text if set (MAIN-007),
                                              // else the variant content
      if text is not empty: blocks.append(text)
  return join(blocks, BLANK_LINE)      // BLANK_LINE = two line breaks
```

- **ALG-020 (P0).** Heading inclusion rule (MAIN-022, MAIN-023): a heading goes into the file if its section, including nested subheadings, has at least one instruction assigned to the project. Parent headings are included automatically, because nested sections are inside the parent's range. An empty variant counts as assigned and keeps the heading (D-13); an assignment with per-project custom text counts as assigned the same way (MAIN-007).
- **ALG-021 (P0).** An instruction belongs to the nearest heading above it. Instructions before the first heading go at the start of the file, without a heading.
- **ALG-022 (P0).** The instruction name is not written to the file. Blocks are separated by exactly one empty line. An empty body is allowed.

Example. Library: `## Style` → instructions A, B; `### Tests` → instruction C; `## Release` → instruction D. A project with only C assigned gets `## Style`, `### Tests`, and the text of C. The `## Release` section does not go into the file.

### 9.3. Building the File by Scheme, and the Creation Marker

- **ALG-030 (P0).** Schemes A and C: `managed = join(parts, BLANK_LINE)`, where parts are the extra-file reading lines with position `start` (A only), the body (if not empty), and the reading lines with position `end` (A only). File: `managed`, an empty line, the marker, one line break. If `managed` is empty, the file has only the marker.
- **ALG-031 (P0).** Scheme B: the `managed` block and the marker (by the same rules) are inserted between the marker lines. The marker lines and everything outside them stay unchanged. The block's line breaks are converted to the ones most used in the file (LF on a tie).
- **ALG-032 (P0).** Marker format: `<!-- Created by EunomiaSync YYYY-MM-DD HH:MM:SS <IANA name> <offset> -->`, for example `<!-- Created by EunomiaSync 2026-11-17 14:03:22 Europe/Prague +01:00 -->`. If the IANA name cannot be found, `UTC+01:00` is written. The time is taken once per run, at the start of the write phase, and is the same for all projects.
- **ALG-033 (P0).** The build hash (DB-020) is computed on `managed` without the marker. Comparison for D-24: in the existing file, the last non-empty line that starts with `<!-- Created by EunomiaSync ` and ends with ` -->` is dropped; the rest is compared with the new content byte for byte.

### 9.4. Sync Pipeline

**Phase 0. Start.**

1. Atomically take `sync_running`; otherwise `E_SYNC_RUNNING`.
2. Create a `sync_runs` record with the `running` status, record the start time and the snapshot (ALG-001), and build the content for all active projects.

**Phase 1. Preflight (changes nothing).** For each active project, in order:

1. Check that the folder is available. No access: `skipped`, `E_PROJECT_UNAVAILABLE`.
2. Find the target path: `<root>/<base_name>` (A, B) or `<root>/<STEM>.project.md` (C). If it is a symbolic link, the path is resolved and the write goes to the real file (a common case: `CLAUDE.md` points to `AGENTS.md`). A dangling link or a real path outside the project root: `skipped`, `E_TARGET_BROKEN_LINK` or `E_TARGET_OUTSIDE_ROOT`. The target path is a folder: `E_TARGET_IS_DIR`.
3. Check rights: whether a temporary file can be created in the folder, and whether the existing file is marked read-only. If not: `skipped`, `E_NO_WRITE_ACCESS`.
4. Read the existing file. Scheme B: check UTF-8 (`E_NOT_UTF8`) and the markers (`E_MARKERS_INVALID`). Build the final bytes.
5. If `sync.skip_identical` is on and the content matches by ALG-033, the status is `unchanged`.
6. If the build is empty but the existing file is not, the project goes into the confirmation list (D-25). In this case, the `sync_run` command without the `confirm_empty` flag returns `NeedsConfirmation` with the list of projects and changes nothing. After confirmation, the UI calls the command again with `confirm_empty=true`. Declining cancels the whole run.
7. Delete stale app temporary files in the target folder (older than one hour).

Projects with the `skipped` status do not stop the run. Free space for backups is checked on the volume of the data directory: if there is less space than the total size of the copies plus a 20% margin, the run ends as `aborted` with `E_DISK_FULL`.

**Phase 2. Backups.**

1. For each project that will be written and whose target file exists, copy the file to the backups folder (7.6), check the size and SHA-256, and add a record to `backup_records`.
2. Any error: the run gets the `aborted` status with `E_BACKUP_FAILED`; the copies of this run get the `discarded` status and are deleted; projects in `sync_run_projects` get `not_started` (except `skipped`); the last sync of every project stays unchanged. No target file is touched.
3. After phase 2 succeeds, the run cannot be canceled.

**Phase 3. Write.** Projects are processed one by one, in order:

1. Atomic write (9.5).
2. Success: status `written`; the project's `last_synced_at` and `last_synced_hash` are updated right away in a separate short transaction.
3. Error: status `failed` with a code and message; the other projects continue. The backup stays.

**Phase 4. Finish.**

1. For projects with the `unchanged` status, `last_synced_at` and `last_synced_hash` are updated.
2. The backup retention policy is applied (BKP-003).
3. Final run status: `success` if every active project got `written` or `unchanged`; `partial` if there is at least one `skipped` or `failed` and at least one `written` or `unchanged`; `failed` if no project was written or left unchanged. If there are no active projects, the run ends as `success` with a message.
4. The overall time of the last successful sync (MAIN-027) is the `finished_at` of the last run with the `success` status. For `partial`, the overall time is not updated, and the UI shows "Last attempt: partial" and the per-project times.
5. Reset `sync_running`, send `state://dirty`, show the summary.

**Recovery after a crash.** At startup, `sync_runs` records with the `running` status get the `interrupted` status, events are written to the log, and hashes are recalculated. Files are intact thanks to atomic writes, and the project status already reflects what was written at the time of the crash.

**Closing the window during a sync.** A confirmation is shown. The app finishes the current atomic write and then exits.

### 9.5. Atomic File Write

1. Resolve the target path (symlinks).
2. Create a temporary file in the same folder: `.<name>.eunomia-<8 random characters>.tmp` (`create_new` mode). The rights of the existing file are copied (Unix mode).
3. Write all bytes and call `sync_all`.
4. Replace the target: on Unix with `rename`, on Windows with `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` and `MOVEFILE_WRITE_THROUGH`. On a sharing violation or access denied, retry up to 3 times with pauses of 100, 300, and 900 ms, then `E_FILE_LOCKED`.
5. On Unix, run `fsync` on the parent folder.
6. Read the file and check its SHA-256 against the expected one. Mismatch: `failed`, `E_VERIFY_FAILED`.
7. On an error at any step, the temporary file is deleted and the original stays unchanged.

### 9.6. Repository Scan

- **ALG-050 (P0).** Walk: `ignore::WalkBuilder` without reading `.gitignore`; symlinks are not followed; Windows reparse points (junctions) are not followed; depth from settings; no more than `min(4, number of cores)` threads.
- **ALG-051 (P0).** Folders with names from `scan.exclude_names` and system folders are skipped: `$RECYCLE.BIN`, `System Volume Information`, `.Trash`, `.Trashes`. The scan continues inside a found repository (nested repositories are found separately).
- **ALG-052 (P0).** A folder with a `.git` directory is a found repository. A folder with a `.git` file is skipped with the reason "submodule or worktree".
- **ALG-053 (P0).** For a found path, the `path_key` and class are computed: `new`, `known` (active), `archived`, `excluded`. Progress is sent no more than 10 times per second; cancellation is checked on every entry.
- **ALG-054 (P0).** The scan reads only directory entry names and does not open files.
- **ALG-055 (P0).** A path whose text is not valid UTF-8 — the folder's own name or an ancestor's — is skipped with the reason "path is not valid UTF-8" and never reported as a repository; the walk does not descend into it (every descendant's path inherits the invalid bytes). The check runs before the name filters and at every visited depth, including the scan root; there is no OS code — a fact about the text, not an access refusal (D-120).

### 9.7. Reordering Library Items

- **ALG-060 (P0).** The branch range of a level-L heading goes from the heading itself to the item before the next heading of level L or higher (hidden instructions included). Moving a branch inside its own range is not allowed.
- **ALG-061 (P0).** A move runs in one transaction: cut the range, insert it at the target position, renumber `position` from 1. Moving one instruction is a special case of a one-item range.
- **ALG-062 (P0).** When a branch is dropped on a heading (to become its last child) or on the library column header (to become the first root section, LIB-032), the levels of all headings in the branch shift automatically to the target level + 1 (for the header, to 2); the move is rejected with `E_VALIDATION` if any new level would leave 2–6. The manual "Shift branch level ±1" command changes the levels of all headings in the branch by one under the same 2–6 rule; otherwise it is rejected with `E_VALIDATION`. Automatic shifting on drop replaces the earlier "levels never move with a branch" behavior by the owner ruling of 2026-11-21 (D-125).

### 9.8. Versions of Instructions and Variants

- **ALG-070 (P0).** When an instruction name, or a variant name or text, changes, a JSON snapshot is built and its SHA-256 is computed. If the hash equals the hash of the last version, no version is created.
- **ALG-071 (P0).** `rev_no` grows by 1 within an entity. Operations: `create`, `update`, `hide`, `restore`, `delete`, `revert`.
- **ALG-072 (P0).** After a version is inserted, the oldest extra ones are deleted until `history.max_revisions` remain. The latest version is never deleted.
- **ALG-073 (P1).** Diff between versions, and between a file and a backup, is built line by line (`similar`); the result goes to the UI as a list of "equal", "insert", "delete" blocks.

## 10. Errors and Conflicts

General rules. Every error is written to the log with its code, project, path, and OS code. A sync is idempotent, so running it again after fixing the cause is safe. User confirmation is needed in only two cases: an empty result over a non-empty file (D-25), and destructive UI actions (NFR-021).

| Code                                                               | Situation                                                                             | Effect                                                                                                                                        | User action                                                                                      |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `E_SYNC_RUNNING`                                                   | A second run, or a project edit during a sync                                         | Command rejected                                                                                                                              | Wait until it finishes                                                                           |
| `E_NOT_A_REPO`                                                     | A folder without `.git` is being added                                                | Project not added                                                                                                                             | Pick the repository root                                                                         |
| `E_PROJECT_UNAVAILABLE`                                            | Folder moved or deleted, drive disconnected, no read rights                           | Project skipped; the others continue                                                                                                          | "Locate…" or "Archive"                                                                           |
| `E_NO_WRITE_ACCESS`                                                | No write rights to the folder, or the file is read-only                               | Project skipped                                                                                                                               | Change the rights and retry                                                                      |
| `E_TARGET_IS_DIR`, `E_TARGET_BROKEN_LINK`, `E_TARGET_OUTSIDE_ROOT` | The target is a folder, a dangling link, or outside the project root                  | Project skipped                                                                                                                               | Fix the target                                                                                   |
| `E_MARKERS_INVALID`                                                | In scheme B, markers are missing, repeated, or in the wrong order                     | Project skipped                                                                                                                               | Paste the block with "Copy marker block"                                                         |
| `E_NOT_UTF8`                                                       | The scheme B file is not UTF-8                                                        | Project skipped                                                                                                                               | Re-encode the file                                                                               |
| `E_BACKUP_FAILED`                                                  | A backup was not created or failed the check                                          | Run `aborted`; files not touched                                                                                                              | Fix the cause from the error text and retry                                                      |
| `E_DISK_FULL`                                                      | No space: on the backups volume in phase 1, or on the project volume during the write | In phase 1, run `aborted`; during the write, project `failed`                                                                                 | Free up space                                                                                    |
| `E_FILE_LOCKED`                                                    | The file is locked by another process (Windows); 3 retries failed                     | Project `failed`; backup kept                                                                                                                 | Close the program and retry                                                                      |
| `E_WRITE_FAILED`, `E_VERIFY_FAILED`                                | Write error or hash check error                                                       | Project `failed`; backup kept                                                                                                                 | If needed, restore the file by hand from the backup in the data directory (in-app restore is P1) |
| `E_CONTENT_RESERVED`                                               | The text contains a scheme B marker line                                              | Save rejected                                                                                                                                 | Remove the line                                                                                  |
| `E_VALIDATION`                                                     | A field failed validation (table 6.9)                                                 | The whole operation rejected                                                                                                                  | Fix the field                                                                                    |
| `E_DB_LOCKED`                                                      | The database is busy; `busy_timeout` expired                                          | Operation rejected; up to 3 automatic retries                                                                                                 | Retry                                                                                            |
| `E_DB_CORRUPT`                                                     | `quick_check` failed                                                                  | Start in recovery mode: the data directory path, the list of `db-backups/` copies, "Open data folder" and "Start with empty database" buttons | Pick a copy or an empty database                                                                 |
| `E_DB_TOO_NEW`                                                     | The database was created by a newer version                                           | The app does not change the database                                                                                                          | Update the app                                                                                   |
| `E_RESTORE_SOURCE_MISSING`                                         | (P1) The backup file to restore from is missing                                       | The record gets the `missing` status                                                                                                          | Pick another copy                                                                                |
| `E_IMPORT_INVALID`                                                 | (P1) The import file has an unknown format or version                                 | Import canceled; data not changed                                                                                                             | Use an export from this version                                                                  |
| `E_UPDATE_FAILED`                                                  | No network, or the update signature is invalid                                        | Update not installed; work continues                                                                                                          | Retry later                                                                                      |

Extra reliability requirements:

- **ERR-001 (P0).** At every startup, the app makes a DB copy with `VACUUM INTO` (`db-backups/daily-YYYYMMDD.sqlite3`) if there is none for today yet, and keeps the last 7 (question 27).
- **ERR-002 (P0).** An interrupted scan saves nothing. An interrupted DB edit is rolled back by the transaction. An interrupted file write leaves the old file (9.5).
- **ERR-003 (P0).** A disconnected drive or lost network during a write ends as `E_WRITE_FAILED` for that project and does not break the integrity of the others.

## 11. Interface and UX

### 11.1. Window Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ EunomiaSync [Search…] [Projects ▾] [Show hidden (2)]  [+ Add project] │
│                                 [+ New instruction] [Activity] [⚙] [?] │
├───────────────────────────┬──────────────────────────────────────────┤
│ Library (tree)            │ WEB   API   OPS   …   (project columns)  │
│ ## Style                  │                                          │
│   Code style          [i] │  ✔     ✔▾     ☐                           │
│   Naming              [i] │  ☐     ✔     ✔                           │
│ ### Tests                 │                                          │
│   Unit tests          [i] │  ✔     ☐     ☐                           │
├───────────────────────────┴──────────────────────────────────────────┤
│ [Run sync]  Last sync 2026-11-17 14:03   3 projects have unsynced…    │
└──────────────────────────────────────────────────────────────────────┘
```

- **UI-001 (P0).** One main window. Minimum size 960×600. The bottom sync panel is outside the scrollable area (MAIN-025). The toolbar, left to right: brand block; search (D-14); project filter; "Show hidden" toggle with the hidden count; with rows selected, the "N rows selected" chip and the "Assign to projects…" button; on the right, "+ Add project", "+ New instruction", icon-only Activity and Settings buttons, and "?" (Help).
- **UI-002 (P0).** Modal windows: "Project settings" (PRJ-008, question 37: modal only), "Settings", "Instruction editor", "History", "Scan", "Sync summary", "Unsynced changes" (MAIN-026), "Archived projects", "Excluded paths", "Activity", "Help", "About". The "Archived projects" and "Excluded paths" windows open from the link buttons in the "Updates and About" section of Settings (DEF-001, D-10).
- **UI-003 (P0).** The instruction editor is a large modal window: the variant list on the left; the instruction name, variant name, and text fields on the right; "Edit" and "Preview" tabs; "Save", "Cancel", "History" buttons at the bottom. Closing with unsaved changes needs confirmation ("Keep editing" / "Discard").
- **UI-004 (P1).** Collapsing sections in the tree and saving that state between launches.

### 11.2. Matrix Cell States

| State                  | Display                                                                                                                              | Screen reader label                     |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------- |
| Not assigned           | Empty square                                                                                                                         | Not assigned                            |
| Main variant           | Square with a check mark and an inline select showing `Main`                                                                         | Assigned, main variant                  |
| Alternative variant    | Check mark and an inline select with the variant name                                                                                | Assigned, variant with the variant name |
| Custom text (MAIN-007) | Check mark, a disabled select showing "Custom", and edit and delete icons                                                            | Assigned, custom text                   |
| Empty variant          | Check mark, a dashed border, and an inline select with the variant's name                                                            | Assigned, empty variant                 |
| Changed                | Dimmed cell of any state above, a visible "(changed)" suffix, and a tooltip: "Assignment changed — will be written on the next sync" | The state's label, then "changed"       |
| Unavailable project    | Column with a warning icon                                                                                                           | Project unavailable                     |

Every cell, assigned or not, also carries the "+" affordance that opens the per-project custom editor (MAIN-007).

A cell is **changed** when its assignment — checkbox, chosen variant, or custom text (MAIN-007) — differs from the session baseline: the assignment state captured when the main window opens and recaptured after every sync run. The marker is a session UI hint, not persisted state. It is the per-cell counterpart of the dirty flag, which stays authoritative and hash-based (DB-021): the two can disagree — a switch to a variant with identical text marks the cell but not the project, and a reordered library dirties the project without marking any cell. Each cell's accessible name is "instruction — project: state", including the "(changed)" suffix (NFR-019).

### 11.3. Keyboard

| Action                       | Shortcut                 |
| ---------------------------- | ------------------------ |
| Navigate the matrix          | Arrows                   |
| Turn an assignment on or off | Space                    |
| Open the variant picker      | Enter                    |
| Select a range               | Shift + arrows           |
| Search                       | Ctrl/Cmd + F             |
| New instruction              | Ctrl/Cmd + N             |
| Add project                  | Ctrl/Cmd + Shift + N     |
| Run sync                     | Ctrl/Cmd + Shift + Enter |
| Settings                     | Ctrl/Cmd + ,             |
| Help                         | F1                       |
| Close window                 | Esc                      |

Tray, autostart, and global hotkeys are not in v1 (D-28).

Shortcut scope: the library shortcuts — search (Ctrl/Cmd+F), New instruction, Add project, and Run sync — do not fire while any dialog is open; Settings (Ctrl/Cmd+,) and Help (F1) open their windows on top of an open dialog.

Matrix focus is a roving tabindex over the assignment cells: exactly one cell has `tabindex=0`, the rest `-1`; arrow keys skip heading rows (they have no cells), filtered-out rows, and disabled columns, and do not wrap at the edges. Space toggles the checkbox natively. Shift+↑/↓ selects a range of rows from the anchor; a checkbox click resets the anchor. Escape from an open variant select returns focus to the cell's checkbox.

### 11.4. Confirmations and Messages

- **UI-010 (P0).** A confirmation that explains what will happen is required for: archiving a project (typing the name), hiding an instruction, deleting a heading (the dialog says: "The section's instructions stay in place and become part of the previous section"), deleting a variant, an empty sync result, bulk actions on 10 or more assignments (NFR-021), restoring an archived project, deleting an excluded path, and deleting per-project custom text (MAIN-007). In P1, the same is required for restoring a file from a backup, restoring a version, and import.
- **UI-011 (P0).** An error message answers four questions: what happened, where (path, project), why (code and OS reason), and what to do (NFR-018). Technical details are collapsed into a "Details" section with a copy button.
- **UI-012 (P0).** Empty states: no projects ("Add project" and "Scan folder" buttons), no instructions ("New instruction"), empty search.

### 11.5. Localization and Look

- **UI-020 (P0).** `vue-i18n`, file `en.json`, keys like `project.settings.title`, ICU plurals. Backend error texts come as `message_key` and `params` (NFR-004…006). Dates are formatted with `Intl` in the system locale; time is always shown with the time zone.
- **UI-021 (P0).** Light and dark themes follow the OS setting. Text contrast is at least WCAG AA (4.5:1); focus is always visible.
- **UI-022 (P0).** Animations (the sync spinner) are turned off when the OS "reduce motion" setting is on.

Design: the UI uses standard Tailwind components.

## 12. Non-Functional Requirements

### 12.1. Platforms

- **NFR-001…003 (P0).** Windows 10 22H2 and newer (x64, arm64), macOS 12 and newer (arm64, x64), Ubuntu 22.04 and newer (x64; arm64 if possible). File system work (rights, paths, atomic replace, symlinks) is tested separately on each platform (section 15).
- **NFR-023 (P0).** Windows: long path support is turned on through the manifest. Linux: running as root shows a warning, because the created files will be owned by root.

### 12.2. Performance

The targets are checked with a benchmark before release on a reference machine (4 cores, 16 GB RAM, SSD). Data set **L**: 500 projects, 1000 instructions and 100 headings, 100,000 assignments, 3 variants per instruction. Data set **M**: 50 projects, 200 instructions. Data set L is confirmed as the upper-limit scenario.

| Metric                                       | Target                                                            |
| -------------------------------------------- | ----------------------------------------------------------------- |
| Startup until the window is ready (set L)    | no more than 2 s                                                  |
| Saving an assignment toggle (p95, set L)     | no more than 50 ms                                                |
| Building one project with up to 1000 items   | no more than 20 ms                                                |
| Dirty recalculation for all projects (set L) | no more than 500 ms, in the background                            |
| Matrix scrolling (set L)                     | at least 50 frames per second; no more than 3000 matrix DOM nodes |
| Sync of 100 projects, files up to 10 KB, SSD | no more than 10 s, not counting antivirus locks                   |
| Scan of 100,000 folders on an SSD            | no more than 30 s; UI input response no more than 100 ms          |
| Memory (RSS, set L)                          | no more than 400 MB                                               |

Maximum supported scale for v1: 2000 projects, 5000 library items.

### 12.3. Reliability

- **NFR-007…010 (P0).** A process crash does not damage the DB (WAL, `synchronous=FULL`) and does not leave partly written target files (9.5). Migrations are described in DB-004. A damaged or newer database is handled by `E_DB_CORRUPT` and `E_DB_TOO_NEW`.
- **NFR-024 (P0).** For each step of pipeline phases 2–3, there is a test that force-kills the process (`SIGKILL` or `TerminateProcess`) and checks file integrity.

### 12.4. Accessibility

- **NFR-019, 020, 022 (P0).** The target is WCAG 2.1 AA for the main scenarios. The matrix is `role=grid` with `aria-rowindex` and `aria-colindex`; each cell has a text label (11.2). The tap target size is at least 24 px. Color is never the only sign of a state.

### 12.5. Localization

- **NFR-004…006 (P0).** Backend error texts reach the UI as `message_key` and `params` (§7.10's error envelope) and are rendered localized from the frontend catalog (UI-020).

## 13. Security and Privacy

Requirements SEC-001…003 are in 5.6. In addition:

- **SEC-004 (P0).** The app stores no secrets. Network use is only checking for and downloading updates (NFR-012, NFR-013). There are no listening ports and no deep links.
- **SEC-005 (P0).** The Markdown preview is built in Rust; the HTML is sanitized (`ammonia`); raw HTML from instruction text is escaped. External resources are not loaded.
- **SEC-006 (P0).** Backup paths are built from the ID and the cleaned slug, not from arbitrary user strings. The target file name is only a value from the enum.
- **SEC-007 (P0).** Rights: backups folder `0700`, DB file `0600` on Unix. On Windows, the user profile rights are inherited.
- **SEC-008 (P0).** Instruction text and file content never go into logs. Paths do, so a reminder about this is shown when the logs folder is opened.
- **SEC-009 (P1).** An import file is treated as untrusted: schema and version check, 50 MB limit, field length limits from 6.9, no executable content.
- **SEC-010 (P0).** CI runs `cargo audit`, `cargo-deny`, `npm audit`. Lock files are kept in the repository.
- **SEC-011 (P0).** The private update signing key is stored outside the repository (a CI secret). A key rotation procedure is described.
- **SEC-012 (P0).** The app runs with user rights and does not ask for elevation (NFR-011).

## 14. Build, Installation, Updates

| Platform | Artifacts                                                                  | Notes                                                                                                                    |
| -------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Windows  | NSIS installer (per-user install without admin rights), WebView2 evergreen | Installers are not signed for now (Authenticode is P1). Without a signature, a SmartScreen warning is possible           |
| macOS    | `.dmg`, `.app` for arm64 and x64                                           | Installers are not signed or notarized for now (P1). Until then, the app must be opened by hand through the context menu |
| Linux    | `.deb` and `.AppImage`                                                     | Depends on `webkit2gtk-4.1`. The `.rpm` format is optional                                                               |

- **REL-001 (P0).** CI (GitHub Actions) on Windows, macOS (arm64 and x64), and Ubuntu (the `ubuntu-latest` runner image; 26.04 at the time of writing — the label migrated from 24.04 in November 2026, and GitHub's 22.04 images are deprecated): formatting, `clippy -D warnings`, `cargo test`, `cargo-deny`, frontend linter and type check, Vitest, e2e, artifact build, signing of update artifacts, a release draft with `latest.json`.
- **REL-002 (P0).** Versioning follows SemVer. Release notes come from `CHANGELOG.md` and are shown in the update dialog (UPD-002).
- **REL-003 (P0).** Uninstalling the app does not delete the data directory. The Windows installer offers a "Remove user data" checkbox (off by default).
- **REL-004 (P0).** There is one update channel (stable). Project license: dual license, MIT License and Apache License, Version 2.0. Repository, releases, and updates: https://github.com/yepbro/EunomiaSync. Author: Den Brooks, denis@yepbro.pro. App identifier (bundle ID): `pro.yepbro.eunomiasync`. The "About" window shows the list of dependency licenses.

## 15. Testing and Acceptance

### 15.1. Test Levels

| Level                              | What is tested                                                                                                                                                                                                                                                                                                                                                                                              | Tools                                                                                   |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Unit (`core`)                      | Building the body and files by scheme, the heading inclusion rule, the custom-text build path (a cell with custom text contributes that text, MAIN-007), normalization, the marker parser, the hash, path normalization, the branch range. Property tests: for random trees, the order in the file equals the library order, the build is idempotent, a heading with no assigned instructions is not output | `cargo test`, `proptest`                                                                |
| Integration (`store`, `fs`, `app`) | Migrations from each schema version, DB invariants, the sync pipeline on temporary Git repositories in all schemes, backups, retention, scan, watcher state transitions (external edit, target deletion)                                                                                                                                                                                                    | `cargo test`, `tempfile`                                                                |
| Fault injection                    | Failure of the N-th backup, `rename` failure, out of space, read-only file, dangling symlink, locked file (Windows), force-killing the process between phases and inside a write                                                                                                                                                                                                                            | failure points in `fs`, a child process with `SIGKILL` or `TerminateProcess`            |
| UI (components)                    | Matrix, editor, confirmation dialogs, validation, keyboard                                                                                                                                                                                                                                                                                                                                                  | Vitest, Testing Library                                                                 |
| E2E                                | Scenarios S1–S7, S9, S10 (P1: S8, S11)                                                                                                                                                                                                                                                                                                                                                                      | Playwright against the Vite dev server with a mocked IPC backend, identical on all OSes |
| Load                               | Section 12.2 metrics on sets M and L                                                                                                                                                                                                                                                                                                                                                                        | `criterion` benchmarks and e2e scripts                                                  |

E2E runs the same way on all three OSes (D-007): Playwright drives the UI in a browser against the Vite dev server with a mocked IPC backend. The IPC contract itself is covered by Rust-side tests over the mock Tauri runtime, and the committed `bindings.ts` keep the frontend type-checked against the real commands. The real shell (native dialogs, updater, window behavior) is covered by the CI build smoke check, and a manual run of scenarios S1–S7, S9, S10 on each OS stays in the release checklist. `tauri-driver` is not used in v1; it may be revisited after 1.0 if real-shell e2e becomes necessary.

### 15.2. Required Failure Scenarios

| ID    | Scenario                                                              | Expected                                                                                             |
| ----- | --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| T-F1  | The backup of the second of three files is not created                | None of the three files is changed, created copies are deleted, run status `aborted`                 |
| T-F2  | The process is killed after the temp file is written, before `rename` | The original file is intact; on the next sync, the temp file is deleted                              |
| T-F3  | The process is killed between writing project 1 and project 2         | Project 1 has the current sync timestamp, project 2 has the old one, the run is marked `interrupted` |
| T-F4  | Scheme B with no markers, with two pairs, with reversed order         | `skipped`, `E_MARKERS_INVALID`, the other projects are written                                       |
| T-F5  | The target file is read-only                                          | `skipped`, `E_NO_WRITE_ACCESS`                                                                       |
| T-F6  | `CLAUDE.md` is a symlink to `AGENTS.md`                               | The write goes to the real file; the link is kept                                                    |
| T-F7  | The project folder is deleted between preflight and the write         | Project `failed`; the others continue                                                                |
| T-F8  | The file is locked by an editor (Windows)                             | 3 retries, then `E_FILE_LOCKED`; backup kept                                                         |
| T-F9  | A second sync with no changes                                         | All projects `unchanged`; no backups created                                                         |
| T-F10 | Backup limit 3 and five syncs with changes                            | The last 3 copies remain                                                                             |
| T-F11 | DB newer than the app, damaged DB                                     | `E_DB_TOO_NEW` and `E_DB_CORRUPT`; data not changed                                                  |
| T-F12 | A scheme B file with BOM and CRLF                                     | The area outside the markers is kept byte for byte                                                   |

| T-F13 | A target file is edited externally between syncs | The project shows the externally modified state; the next sync overwrites the file and makes a backup |

### 15.3. Platform Matrix

| Platform | Special checks                                                                                                            |
| -------- | ------------------------------------------------------------------------------------------------------------------------- |
| Windows  | Case-insensitive NTFS, long paths, junctions, locked files, CRLF in repositories, write rights in protected folders       |
| macOS    | Case-insensitive APFS, NFD names, permission prompts for Desktop, Documents, and external volumes during a scan, symlinks |
| Linux    | Rights and owners, symlinks, XDG folders, Wayland and X11 (smoke), running the AppImage                                   |

### 15.4. Mapping to the Definition of Done from the PR

| #      | Criterion (short)                                                  | Check                                |
| ------ | ------------------------------------------------------------------ | ------------------------------------ |
| 1, 2   | Adding by hand, scan, no duplicates                                | Integration, E2E S1, S2              |
| 3      | Instruction, variants, different variants in projects              | E2E S3, S4                           |
| 4      | Heading hierarchy and moving a branch                              | Unit (ALG-060…062), E2E              |
| 5, 6   | File order and contents                                            | Property tests, unit for ALG-020…022 |
| 7      | Saved right away, files not changed                                | Integration (OUT-010, OUT-011)       |
| 8      | Button visible when scrolling, disabled with a spinner             | E2E (MAIN-025, MAIN-026)             |
| 9      | Warning about unsynced changes                                     | Integration, E2E S9                  |
| 10     | One command for all projects                                       | E2E S7                               |
| 11, 12 | Backup in the project's folder, backup failure blocks writing      | Integration, T-F1                    |
| 13, 14 | Full build with marker, marker at the end of the block in scheme B | Unit and integration for ALG-030…033 |
| 15     | External edits are overwritten                                     | Integration (OUT-015)                |
| 16     | Backup limit                                                       | T-F10                                |
| 17     | Last sync times                                                    | Integration (OUT-020)                |
| 18     | History, summary, backups, errors                                  | Integration (LOG-001…004)            |
| 19     | Unavailable project                                                | Integration, E2E (D-09)              |
| 20     | Basic scenarios on all OSes                                        | CI matrix (REL-001)                  |

### 15.5. Release Conditions

- All P0 tests pass on the three OSes.
- The section 12.2 metrics are confirmed on set L.
- There are no known defects that lead to data loss or partial writes.
- The manual checklist for scenarios S1–S7, S9, S10 is done on each OS.
- Help, `CHANGELOG.md`, and the dependency license list are up to date.

## 16. Implementation Plan

Estimates are rough, for one developer who knows Rust and Vue. They are refined based on the customer's answers to the open questions.

| Stage                | Contents                                                                                           | Result                               | Depends on | Estimate |
| -------------------- | -------------------------------------------------------------------------------------------------- | ------------------------------------ | ---------- | -------- |
| M0 Skeleton          | Repository, workspace, CI, Tauri, Vue, SQLite migrations, error contract, config file (TOML), i18n | An empty app builds on three OSes    | —          | 1.5 wk   |
| M1 Projects          | Adding by hand, scan, card, archive, exclusions, order                                             | Scenarios S1, S2                     | M0         | 2 wk     |
| M2 Library           | Items, headings, variants, editor, hiding, version history records                                 | Scenarios S3, S5                     | M0         | 2.5 wk   |
| M3 Matrix            | Virtualization, assignments, bulk operations, per-project custom text (MAIN-007), search           | Scenarios S4–S6                      | M1, M2     | 2.5 wk   |
| M4 Build             | Builder, schemes A, B, C, marker, hash and dirty, target-file watcher                              | Scenario S9, unit and property tests | M2         | 1.5 wk   |
| M5 Sync              | Pipeline, atomic write, backups, summary, fault injection                                          | Scenarios S7, S10                    | M3, M4     | 3 wk     |
| M6 Service features  | Log, help, settings, updates, About                                                                | All P0 screens                       | M5         | 2 wk     |
| M7 Stabilization     | Cross-platform checks, accessibility, performance, packages                                        | Confirmed section 12 targets         | M6         | 2.5 wk   |
| M8 Release candidate | Fixes, documentation, release checklist                                                            | Version 1.0.0                        | M7         | 1 wk     |

Total: about 18.5 person-weeks. Stages M1 and M2 can run in parallel if there is a second developer. The estimates were made before diff and version rollback, restore from backup, and export and import moved to P1, and before the target-file watcher and the TOML config file were added to the scope; they need to be re-estimated.

**Next stage (P1), after 1.0.0:** diff between versions and version rollback (LIB-028, LIB-029, ALG-073, S8), restoring a file from a backup in the UI (BKP-007, S11), JSON export and import (D-18, SEC-009).

### Risks

| Risk                                              | Impact                       | Mitigation                                                                |
| ------------------------------------------------- | ---------------------------- | ------------------------------------------------------------------------- |
| Different `webkit2gtk` versions on Linux          | Layout and UI failures       | Testing on Ubuntu 22.04 and 24.04, a minimal set of CSS features          |
| File locks by antivirus and editors on Windows    | Write errors                 | Retries with pauses, a clear `E_FILE_LOCKED` error, backup before writing |
| Browser-only e2e does not exercise the real shell | IPC regressions slip through | Mock-runtime IPC tests in Rust and the manual release checklist (15.1)    |
| Matrix performance on set L                       | UI lag                       | Virtualization on both axes, an early benchmark in M3                     |
| macOS permissions during a scan                   | Skipped folders              | Clear messages in "Skipped", help                                         |
| Loss of the update signing key                    | Cannot release updates       | A backup of the key and a rotation procedure (SEC-011)                    |
