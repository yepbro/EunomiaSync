# DECISIONS

Problems the project ran into and how they were solved.
Every entry has a unique stable identifier `D-NNN` (sequential, never reused);
reference entries by it from commits, reviews, task comments and other entries.

### D-001 — Configuration format: TOML — 2026-11-17

Problem: the tech stack listed both YAML and TOML at once — a single format had to be chosen.

Solution: a single TOML format (serde + toml); YAML is excluded.

### D-002 — SQLite access: rusqlite + rusqlite_migration — 2026-11-17

Problem: no database access layer was chosen (options: rusqlite, sqlx, tauri-plugin-sql).

Solution: rusqlite (the `bundled` feature) + rusqlite_migration, synchronous (Tauri commands
already run on a thread pool); migrations are SQL files embedded via `include_str!`.

### D-003 — Typed IPC: tauri-specta — 2026-11-17

Problem: the Rust command contract and TypeScript can drift apart from the code.

Solution: tauri-specta generates `src/bindings.ts`; raw `invoke()` is forbidden in the frontend —
the frontend only calls the generated `commands`.

### D-004 — File watching: notify-debouncer-full — 2026-11-17

Problem: raw notify sends a flood of events for a single change.

Solution: debounce via notify-debouncer-full; on Linux, account for
`fs.inotify.max_user_watches` and handle its overflow with a clear message.

### D-005 — Auto-update: tauri-plugin-updater — 2026-11-17

Problem: app distribution requires an update mechanism.

Solution: tauri-plugin-updater at the distribution stage (signing keys, endpoint —
a separate task); the updater is part of the MVP (spec 7.9).

### D-006 — Cargo workspace: crates/eunomia-core + src-tauri — 2026-11-17

Problem: logic inside src-tauri is hard to test without a UI.

Solution: the core is a separate `crates/eunomia-core` library with no Tauri dependencies
(storage, markdown, diff, sync, watch); src-tauri holds commands, state, and specta.

### D-007 — E2E strategy: Playwright against the Vite dev server — 2026-11-17

Problem: tauri-driver + WebdriverIO is fragile (msedgedriver on Windows).

Solution: UI scenarios run via Playwright against the Vite dev server; tauri-driver comes later,
smoke tests only.

### D-008 — Agent skills: adapted pstack subset in .opencode/skills — 2026-11-17

Problem: the pstack plugin (github.com/cursor/plugins, MIT) targets Cursor/Claude Code, and
its core (poteto-mode) conflicts with this repo's cycle: no plan-first, optional tests,
autonomous push/board/cleanup, multi-model fan-out unavailable in OpenCode.

Solution: no wholesale install. Nine self-contained skills (seven principles,
typescript-best-practices, create-verification-skill) were copied into `.opencode/skills/`
(gitignored, owner-local) with adaptations: output paths `.cursor/skills/` → `.opencode/skills/`;
cleanup scoped to what a run creates (everything else still requires operator confirmation);
cargo resource policy embedded; the DECISIONS.md duty carved out in
encode-lessons-in-structure; Vue/tauri-specta notes in typescript-best-practices.
Invocation points are fixed in AGENTS.local.md ("Skill Invocations").
poteto-mode, arena, swarm, interrogate and tdd were not installed.

### D-009 — IPC command names for the missing groups — 2026-11-17

Problem: spec 7.10 had no commands for the system folder picker, the Markdown
preview, restoring a variant, and the database recovery screen (E_DB_CORRUPT).

Solution: `dialog_pick_folder` (the system folder picker opened from Rust; feeds
the SEC-003 session allowlist), `markdown_preview` (sanitized HTML + warnings,
SEC-005/D-15), `variant_restore`; for the recovery screen: `db_backups_list`,
`db_restore_backup`, `db_recreate`, plus `app_open_data_folder`. The watcher adds
the `state://file-changed` event. All names are listed in spec 7.10.

### D-010 — Crate layout: split per the spec (core/store/fs/app) — 2026-11-17

Problem: the repo keeps SQLite inside `eunomia-core` (D-006), while the spec (5.2)
and the task plan assume separate `eunomia-store` and `eunomia-fs` crates.

Solution: the spec's four-crate layout wins — it enforces "core = pure logic, no IO"
at the dependency level, and the planned split is mechanical (`storage.rs` and
`migrations/` move to `eunomia-store`). D-006's rule (no Tauri dependencies in the
library crates) stays unchanged.

### D-011 — Spec revision: watcher, TOML config, updater in MVP; e2e via Vite — 2026-11-17

Problem: docs/spec.md contradicted the recorded decisions: it had no TOML config
(D-18 said "TOML is not used"), no file watcher (OUT-014), e2e via tauri-driver, and
D-005 said the updater was not part of the MVP while spec 7.9 made it P0.

Solution: spec v1.1 brings the decisions in and closes the gaps: a machine-level
`config.toml` (TOML per D-001; user settings stay in SQLite, spec 5.8), a target-file
watcher in the MVP (notify-debouncer-full per D-004, display-only, sync semantics
unchanged, spec 5.7), the updater confirmed as MVP scope, and e2e confirmed as
Playwright against the Vite dev server with a mocked IPC (tauri-driver is out of v1,
spec 15.1).

### D-012 — Agent skills: rust-skills (leonardomso) installed — 2026-11-17

Problem: Rust guidance for the workspace was ad hoc. The rust-skills set (265 rules,
26 categories, MIT) fits the context budget — an index SKILL.md plus per-rule files
loaded on demand — but its recommendations must not silently override project decisions.

Solution: installed as `.opencode/skills/rust-skills` (index + `rules/`; the source
repo's own checks/ crate, AGENTS.md/CLAUDE.md/meta files excluded). An "EunomiaSync
Notes" section in the index pins the overlays: clippy `-D warnings` beats rule advice;
new crates (thiserror/anyhow/smallvec/…) only via a task + DECISIONS.md entry
(cargo-deny applies); the project stays unsafe-free; no preemptive tokio; resource
policy overrides parallelism advice. Invocation: the description auto-gates it to Rust
tasks; the moment is pinned in AGENTS.local.md (any task touching `crates/**` or
`src-tauri/**`); Rust subagent briefs carry the skill path.

### D-013 — Tauri skills: fxckcode set + LevyBytes router; full-stack set rejected — 2026-11-17

Problem: Tauri 2 knowledge was scattered; three candidate skill sets had different
shapes and overlaps (fxckcode 9 compact gate-skills MIT; LevyBytes one router with deep
references Apache-2.0; full-stack-skills 52 per-plugin skills Apache-2.0).

Solution: installed the fxckcode set minus `tauri-mobile` (desktop-only project) as
eight workflow gates: setup, architecture, frontend, rust, security, plugins, testing,
release; its repo-level `references/` (official docs map, version-sensitive notes) went
to `.opencode/references/` so the setup skill's relative link resolves. Installed the
LevyBytes router as `tauri-develop` (SKILL.md + references/ + LICENSE + NOTICE) for the
deep topics the gates lack: IPC mocks (matches the D-007/D-011 mocked-IPC e2e),
Rust/WebView debugging, sidecars/resources, icons, webdriver. Precedence is pinned in
AGENTS.local.md: fx gates decide, `tauri-develop` supplies detail. Overlays applied:
D-003 (no raw `invoke()`, tauri-specta `commands`) onto tauri-frontend and the router;
Playwright-over-Vite e2e onto tauri-testing; updater MVP scope onto tauri-release.
The 52-skill full-stack set was rejected: registry pollution, and its updater skill is
a thin scope blurb with links, covered by `tauri-release` + official docs.

### D-014 — Vue skills: 7 of 17 from KIMJINWOO4/vue-skills — 2026-11-17

Problem: the frontend layer had no framework-level guidance (only the TS layer via
typescript-best-practices and the IPC layer via tauri-frontend). The upstream set ships
17 skills, several irrelevant to this repo (Nuxt, VitePress, antfu conventions,
component/query/validation libraries, pnpm/Turborepo tooling).

Solution: installed `vue` (core: reactivity, SFC, Pinia, testing references),
`vue-debug`, `vue-review`, `vue-performance`, `vue-clean-components`, `vue-a11y`,
`build-tooling`; each with provenance LICENSE and overlay footers (no Nuxt/SSR; D-003
no raw `invoke()`; npm not pnpm — no pnpm workspaces/Turborepo/tsdown; cargo resource
policy). Skipped: `vue-data` (query layer — no HTTP API; data flows via IPC per D-003),
`vue-ui` (no component library decided), `vue-forms` (validation library = dependency
decision), `design` (UnoCSS-centric), `vue-ci` (CI stable), `vue-upgrade` (add when a
Vue upgrade task lands), `nuxt`/`vitepress`/`antfu`/`find-skills` (out of scope).
Layering pinned in AGENTS.local.md: typescript-best-practices (TS) → `vue` (framework)
→ `tauri-frontend` (IPC gate); subagent briefs for Vue tasks carry the core skill path.

### D-015 — Overlay cleanup: provenance/restating footers removed, guards kept — 2026-11-17

Problem: skill files accumulated overlay footers restating facts already authoritative
elsewhere (AGENTS.local.md layering notes, ARCHITECTURE.md, DECISIONS.md) — duplication
that also complicates future upstream syncs.

Solution: stripped provenance and application-site footers from 15 files (seven pstack
principles, five fx tauri gates, five vue skills; typescript-best-practices lost only
its provenance line). Kept the load-bearing set: create-verification-skill intact
(section 0 policies govern the generated skill; .opencode paths); rust-skills
"EunomiaSync Notes" (qualifies 265 rules: clippy wins, new crates via DECISIONS.md,
unsafe-free, no preemptive tokio, resource policy); typescript-best-practices notes
(.vue script blocks despite the paths filter, generated bindings.ts, no zod adopted);
one-line guards where upstream text actively contradicts repo rules — D-003 no-raw-
invoke on tauri-frontend/tauri-develop/vue, Playwright e2e on tauri-testing, updater
MVP scope on tauri-release, npm-not-pnpm on build-tooling — and the DECISIONS.md
carve-out in encode-lessons-in-structure (upstream corollary must not suppress the
record). Rationale: guards must live inside the skill file because auto-triggered
reads and subagent briefs reach the skill without AGENTS.local.md context.

### D-016 — Architecture-patterns skill installed (operator-provided) — 2026-11-17

Problem: no installed skill covered the language-agnostic design layer — planning
decisions (state modeling, invariants, side effects, ownership) and architecture-level
reviews. The pstack principles cover specific techniques; rust-skills/vue are
language-level.

Solution: installed the operator-provided `architecture-patterns` skill (English
original, unmodified; no upstream repository tracked) as `.opencode/skills/architecture-
patterns/`: 9 patterns, 11 anti-patterns, decision rules, and an evidence-based review
format. Wired as the design/review gate in AGENTS.local.md — applied when planning
features or reviewing structure, before language-level skills; precedence over the
overlapping pstack principles pinned there (idempotent expands its pattern 4, boundary
discipline its pattern 6, type-system discipline its invariants/primitive-obsession
guidance). Its audit rules match the repo discipline: reviews never modify code unless
implementation was explicitly requested.

### D-017 — eunomia-store extracted; core keeps the spec dependency allow-list — 2026-11-17

Problem: D-010 planned the store split but left open three details: which crate owns the
rusqlite/rusqlite_migration version-alignment constraint, whether `eunomia-store` gets a
`core` dependency right away (spec 5.2 lists it), and how core's dependency allow-list
(serde, sha2, thiserror) becomes enforceable before the domain modules exist.

Solution: the split was executed (T-002) as a mechanical move — `storage.rs`, `migrations/`
and their tests moved to `crates/eunomia-store` with the public API renamed to
`eunomia_store::storage`. The alignment comment travels with the dependencies into the
store manifest (it constrains whoever depends on rusqlite + rusqlite_migration).
`eunomia-store` does NOT depend on `eunomia-core` yet: nothing in the moved code uses core
types, so the edge would be an unused dependency; it is added by the first task whose store
code actually references core types. Core's manifest now carries the spec-pinned allow-list
serde, sha2, thiserror ahead of use (per the T-002 card and spec 5.2): an empty
`[dependencies]` would let the next task silently add anything, while the pinned list makes
"core = pure logic, these three deps only" reviewable at the dependency level; they stay
idle until the domain-type tasks land, and `eunomia-store` deliberately keeps hand-rolled
error impls instead of pulling thiserror. The allow-list covers the three deps only:
spec 5.3 additionally names `similar` for text diffing, which is outside it — whether
`similar` joins the allow-list or diff logic lives in another crate is a decision for the
diff task. `src-tauri` swapped its path dependency
`eunomia-core` → `eunomia-store` (it had no code referencing core; core comes back with the
first command that needs its types).

### D-018 — Frontend package manager switched from npm to pnpm — 2026-11-17

Problem: npm made every worktree expensive. Each `git worktree add` needed a full
`npm install` (~9 s, ~267 MB per worktree: npm copies a complete flat node_modules even
when the packages are already on disk elsewhere), and the pre-commit hook only worked
after that copy completed. The npm-not-pnpm stance (D-014/D-015 build-tooling guard)
predated the multi-worktree agent workflow and priced pnpm at its monorepo features
(workspaces/Turborepo), which this single-package repo still does not need.

Solution: switched the whole toolchain to pnpm, pinned via `"packageManager":
"pnpm@12.6.0"` (read by corepack locally and pnpm/action-setup in CI). `pnpm-lock.yaml`
replaces `package-lock.json`; CI installs with `pnpm/action-setup` +
`actions/setup-node` (`cache: pnpm`) + `pnpm install --frozen-lockfile`; the pre-commit
hook, CONTRIBUTING, ARCHITECTURE, `tauri.conf.json` (beforeDevCommand/beforeBuildCommand)
and docs now run scripts through pnpm. Worktree bootstrap is the convention documented in
AGENTS.md: run `pnpm install` in a fresh worktree — it hardlinks packages from the global
content-addressable store, measured at ~0.5 s and ~8 MiB of new disk (df delta; du shows
~213 MB only because hardlinked content is shared with the store; on another
filesystem the saving is lost — a /tmp tmpfs install fell back to copying at
~8.5 s / ~204 MiB, npm-like), and it runs the
`prepare` script, so `core.hooksPath=.githooks` is set and the hook works
immediately.
This supersedes the npm-not-pnpm guard of D-014/D-015; the no-workspaces/Turborepo/tsdown
part stays (nothing here adopts monorepo tooling, and pnpm settings would go into
`pnpm-workspace.yaml` if ever needed). Three toolchain details had to be
accommodated. First, pnpm passes the `--` separator through to the script
(npm swallowed it), so the CI build job calls `pnpm run tauri build --debug
--no-bundle` without `--` — with it, tauri forwards the flags to cargo and the
build dies on `unexpected argument '--debug'`. Second, pnpm does not forward
SIGTERM to script children: a Playwright webServer started as `pnpm run dev`
left vite running after the pnpm wrapper died, holding the stdio pipes and
port 1420 open, so the e2e run passed its test but never exited — locally and
in CI; `playwright.config.ts` therefore starts the dev server directly
(`node node_modules/vite/bin/vite.js --port 1420 --strictPort`). Third,
`.prettierignore` already existed on main (it excluded the npm lockfile and
`src/bindings.ts`, so main's `format:check` was green — there was no
pre-existing failure); the migration only swaps the ignored lockfile entry
`package-lock.json` → `pnpm-lock.yaml` and documents both entries: the
lockfile is machine-written, and `src/bindings.ts` is Tauri Specta output
regenerated by `cargo test` (src-tauri/src/lib.rs), so formatting it manually
would be overwritten by the next regeneration. Caveat for the owner: the
`npm (not pnpm)` guard line inside the owner-local build-tooling skill copy is
now outdated and needs a manual footer edit (skills are gitignored and
read-only for agent tasks).

### D-019 — eunomia-fs crate added as a skeleton — 2026-11-17

Problem: the spec (5.2) and D-010 plan the fs crate, but the workspace had no
`crates/eunomia-fs`: the scan/backup/atomic tasks have nothing to land in, and
the file-system dependencies (ignore/walkdir, tempfile, dunce) would otherwise
ride in silently with the first implementing task.

Solution: added the crate as a skeleton per the T-003 card — four modules
declared empty, each with a doc-commented planned responsibility and testing
plan, no implementation: `scan` (repository traversal per spec 9.6,
ALG-050..054: `ignore`'s parallel WalkBuilder with `.gitignore` reading
disabled — a repository under an ignored directory must stay visible — and
`walkdir` for plain recursive traversal outside the repository-scan
algorithm, e.g. the data directory), `paths` (normalization and validation;
`dunce` keeps canonical Windows paths free of verbatim `\\?\` prefixes;
SEC-003 checks land here, fed by app/DB-layer allowlists), `backup` (copies
before writes and copy checks, spec 5.5 layout, BKP-010/SEC-006), `atomic`
(write-then-rename via `tempfile` with the spec 9.5 invariants: same-folder
staging, sync/fsync points, hash verification, 3 retries 100/300/900 ms then
`E_FILE_LOCKED`). The four external dependencies follow the
spec-pinned-ahead-of-use pattern of D-017 (core's own allow-list): each is
pinned by the card and the spec and stays idle until its module lands. The
`eunomia-core` path edge deliberately departs from D-017's other rule (the
store adds core only with its first consumer): the T-003 card fixes this
crate's dependency set at creation, so the edge is wired now — the skeleton
references none of core's types yet, which is expected, but the edge is
compiled and covered by clippy/deny from day one. Deliberately absent:
`notify-debouncer-full` (spec 5.7 watching) — it is outside the T-003 card's
dependency set and the skeleton has no watch module; watching joins with its
own task, matching ARCHITECTURE.md. Tests: none in the crate yet — the
skeleton declares no public items with behavior to assert on, so each module
doc carries the testing plan and T-003 verification is the workspace check
suite (build/fmt/clippy/test/deny); empty `#[test] fn it_works()` markers
were rejected as proving nothing. The module boundaries are the crate's
public contract: implementing tasks fit their code into the declared
modules, and a boundary change requires its own DECISIONS.md entry.

### D-020 — App identifier set to pro.yepbro.eunomiasync — 2026-11-17

Problem: the scaffold carried `com.yepbro.eunomiasync` in `tauri.conf.json`,
while spec 5.5, REL-004 and UPD-005 fix the bundle ID as `pro.yepbro.eunomiasync`.
The identifier is also the last path component of the data directory
(`app_local_data_dir()` = `<local_data_dir>/<identifier>`), so changing it
relocates the folder that holds the database, backups, config, and logs.

Solution: the identifier in `src-tauri/tauri.conf.json` is set to
`pro.yepbro.eunomiasync`, and the `authors` field of `src-tauri/Cargo.toml`
names the owner (`Den Brooks <denis@yepbro.pro>`). Compatibility: the app is
pre-release with no existing installations, so the old `com.yepbro.eunomiasync`
data folder needs no migration; on every OS the data directory ends with
`pro.yepbro.eunomiasync` per the spec 5.5 table (Windows
`%LOCALAPPDATA%/pro.yepbro.eunomiasync/`, macOS
`~/Library/Application Support/pro.yepbro.eunomiasync/`, Linux
`$XDG_DATA_HOME/pro.yepbro.eunomiasync/`, default `~/.local/share/...`).
`src-tauri/src/config_tests.rs` pins the identifier — and with it
`app_local_data_dir()` on all OSes — against the spec value.
