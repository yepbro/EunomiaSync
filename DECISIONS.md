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

### D-021 — Main window opens at 1280×800 with a clamped overflow, minimum 960×600 — 2026-11-17

Problem: UI-001 (spec §11.1) pins only the minimum window size (960×600),
not the startup size, while the scaffold opened at 800×600 — below the
required minimum, so the startup size had to change too. The replacement
value had to stay usable on small and scaled displays: at 1920×1080 with
150% scaling (the most common laptop setup) the logical work area is only
~1280×688, and tauri does not clamp an oversized startup window to the work
area on its own (`preventOverflow` must be set; verified in
tauri-runtime-wry `src/lib.rs` `config.prevent_overflow` → tao
`prevent_overflow()`).

Solution: the `main` window starts at 1280×800 — comfortably above the
960×600 minimum and fully visible on ≥1080p displays at 100%/125% scaling —
with `preventOverflow: true` so smaller work areas (150% scaling,
1366×768) clamp the startup frame instead of pushing the always-visible
sync panel (MAIN-025) off-screen; the 960×600 minimum still wins over the
clamp. Window positioning (`center`) is deliberately not set: OS default
placement is fine, and centering is not part of the T-005 card.
`src-tauri/src/config_tests.rs` pins the minimum, the "startup ≥ minimum"
floor, and locates the window by its label.

### D-022 — CSP: strict static policy, `'self'`/`'none'` sources only — 2026-11-17

Problem: SEC-002 (spec §5.6) requires `default-src 'self'` with external
scripts, styles, and images blocked, but does not spell out the full CSP
string. The scaffold shipped `"csp": null`, so every origin was allowed.
The policy had to stay strict enough to survive later relaxations and
must not break the Vite production bundle or tauri's own injected code.

Solution: `src-tauri/tauri.conf.json` sets
`default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self';
connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri
'self'; form-action 'self'; frame-src 'none'`.
No `devCsp` is set (pinned by a test). Verified against the installed
sources (tauri 2.12.6, tauri-utils 2.10.3, wry 0.57.2), directive by
directive:

- `default-src 'self'` — the SEC-002 baseline and fallback for every
  directive not spelled out.
- `script-src 'self'`, `style-src 'self'` — pinned beyond the fallback so
  relaxing `default-src` later cannot re-open remote code or styles.
  tauri appends the nonces/hashes of its own injected scripts and styles
  to these two directives at serve time (`replace_csp_nonce`, tauri
  manager; `inject_nonce_token`/inline-script hashes, tauri-codegen), so
  no `'unsafe-inline'` is needed; the built `dist/index.html` contains no
  inline scripts or styles (Vite emits `<script src>` + `<link>`), and
  the Vite production CSS/SFC styles are same-origin files. Desktop dev
  is unaffected: the config CSP only reaches pages served through the
  tauri custom protocol, and on desktop `tauri dev` loads the Vite
  server URL directly (`PROXY_DEV_SERVER = cfg!(all(dev, mobile))`).
- `img-src 'self'` — SEC-002's "external images are blocked"; this is the
  CSP-level enforcement for the future Markdown preview (SEC-005). The
  live runtime check ("an external image in the preview does not load")
  lands with the preview task; until then the policy guarantees the
  browser side of it. Because `data:` URIs are not allowed here,
  `vite.config.ts` sets `build.assetsInlineLimit: 0`: Vite's default
  would inline assets under 4 KB into `data:` URIs inside the JS bundle
  (the 496-byte `src/assets/vue.svg` shipped that way), where this
  directive would block them.
- `object-src 'none'` — no legitimate `<object>`/`<embed>`/`<applet>`,
  blocked outright.
- `base-uri 'self'`, `form-action 'self'` — neither falls back to
  `default-src`; without them an injected `<base>` could re-root relative
  URLs to a remote host and forms could post off-origin.
- `frame-src 'none'` — no embedded browsing contexts, ever (remote
  content ban; the preview renders sanitized HTML, not iframes).

`connect-src ipc: http://ipc.localhost` — the tauri-internal IPC
transports, the one deliberate exception to the no-remote-sources rule.
Tauri's IPC script (tauri 2.12 `scripts/ipc-protocol.js`) fetches
`ipc://localhost/<command>` (`http://ipc.localhost/<command>` on
Windows) as its primary transport and only latches the `postMessage`
fallback after a failure; `fetch` is governed by `connect-src`, so
without these two sources every `invoke` first fails a CSP-blocked
fetch round-trip and then settles into the fallback, which answers
responses through `eval` instead of the response channel
(`can_use_channel_for_response` in tauri `ipc/protocol.rs`) — a silent,
slower path for exactly the large payloads (matrix, preview HTML) this
app plans. Both endpoints are tauri-internal loopback transports, not
internet hosts; apart from them the webview performs no network
requests (SEC-004: network belongs to the Rust updater). The first
revision of this decision wrongly claimed IPC is `postMessage`-based on
all three desktop engines and shipped without `connect-src`; the
round-1 review disproved that against `scripts/ipc-protocol.js` and the
policy was corrected. DevTools are release-off by
tauri's defaults (the `devtools` cargo feature stays disabled, no window
sets `devtools: true`) and no deep links are registered (SEC-004);
`src-tauri/src/config_tests.rs` pins all of it, including absence tests
for the `devtools` feature, deep-link crates/config, the unset
`devCsp`, the `dangerous*` security key, a positive pin on the allowed
IPC `connect-src`, and exact-value pins on the four hardening
directives so their silent removal fails the suite too.

### D-023 — The updater registers from a disabled `plugins.updater` stub — 2026-11-17

Problem: T-007 wires `tauri-plugin-updater` before the release task exists,
so there are no real endpoints or signing key yet. The plugin cannot simply
be registered without configuration: tauri hands every plugin the value of
its `plugins.<name>` section and passes `Value::Null` when the section is
absent (tauri 2.12.6 `plugin.rs`, `initialize` → `unwrap_or_default()`), and
the updater's `Config` rejects that at plugin initialization — its custom
`Deserialize` requires the `pubkey` field (no `#[serde(default)]`) and a
`Null` value is not a map at all — so the whole app fails to start
(tauri-plugin-updater 2.14.1 `config.rs`; proven end-to-end by
`updater_plugin_refuses_to_initialize_without_a_config_section` on the mock
runtime). A stub with a fake endpoint was rejected: a release build shipped
by mistake would then probe a dead host on every check instead of failing
fast in Rust.

Solution: `tauri.conf.json` carries `"plugins": { "updater": { "pubkey": "",
"endpoints": [] } }` — the minimal value that deserializes, with the
updater effectively disabled. With it the plugin initializes, manages its
state, and every later `updater()` fails fast with `EmptyEndpoints` until
the release task sets the real key and feed — the failure stays inside the
upcoming `update_check` command instead of at app startup.
`config_tests.rs` pins both halves: the stub must deserialize into the real
`Config` with empty endpoints/pubkey and every `dangerous*` transport
escape off, and an absent section must keep failing plugin initialization.
Inline comments are impossible in `tauri.conf.json`, so the rationale lives
here and in the `lib.rs` registration comment.

### D-024 — The main-window capability grants the webview nothing — 2026-11-17

Problem: with the scaffold's `permissions: ["core:default", "opener:default"]`
the webview could call the whole `core:default` bundle — event, path, and
window/webview state APIs (including `core:webview:allow-internal-toggle-
devtools`), plus the opener plugin — while SEC-001 (§5.6) says the updater,
dialog, and opener plugins are invoked from Rust only and application
commands are not capability-gated. Which permissions, if any, the webview
honestly needs had to be decided on facts.

Solution: `capabilities/default.json` grants `permissions: []` — the empty
set — because the frontend's entire Tauri surface today is `invoke` of
application commands (`src/bindings.ts`, imports `@tauri-apps/api/core`
only). In tauri 2.12.6 the ACL check applies to `plugin:`-prefixed
invocations, to apps that define their own ACL manifest
(`has_app_manifest()`, set by custom permissions), or to non-local origins
(`webview/mod.rs`, the invoke path) — none of which hold here, so
application commands keep working with an empty grant, verified against the
installed source. `config_tests.rs` pins the whole invariant, not one file:
the capability file set itself is pinned to exactly `default.json` — the
same recursive, multi-format discovery tauri-build performs
(`capabilities/**/*`, `json`/`toml`/`json5`, referenced or not — so a
second capability file at any depth in any accepted format cannot slip
in) — and that one file must grant nothing; inline capabilities under
`app > security > capabilities` must stay unset or empty, and the app must
keep defining no ACL manifest of its own (`permissions/` directory and
`build.rs` `app_manifest` both pinned) — with one, every application
command would become capability-gated while the grant is empty, and the
IPC surface would die at runtime with all other tests green. Any future
need (e.g. `core:event:allow-listen` for the SYS-003 progress events) is
added consciously, together with a `config_tests.rs` pin, when the
consuming task lands; the plugin JS packages are banned from `package.json`
by a sibling test.

### D-025 — The single-instance callback restores then focuses the `main` window — 2026-11-17

Problem: T-007 left the `single-instance` callback as a logging
placeholder, so a second launch correctly died but the first instance's
window stayed wherever the user left it — spec SYS-006 (P0) requires the
window of the first instance to be brought to the front. The obvious
`set_focus()`-only handler is wrong on every desktop platform: tao gates
`set_focus` on visibility and the not-minimized state (tao 0.37.2,
`platform_impl/windows/window.rs`, `platform_impl/linux/window.rs`,
`platform_impl/macos/window.rs`), so a hidden or minimized window would
simply stay hidden/minimized and SYS-006 would silently not hold.

Solution: the callback hands off to `single_instance::focus_main_window`
(`src-tauri/src/single_instance.rs`), which resolves the window by the
label `main` (created from `tauri.conf.json` from app start, UI-001) and
runs show → unminimize → set_focus in that order; each restore step is
harmless when the window is already in the requested state, so the
sequence is safe to run unconditionally. On macOS `set_focus` additionally
activates the whole application (`makeKeyAndOrderFront` +
`activateIgnoringOtherApps`), which is the platform-correct reading of
"brought to the front". Two platform nuances are accepted, not engineered
around: on Linux the window manager reports the deiconified state
asynchronously, so a `set_focus` racing the `unminimize` result can be
dropped by tao (most window managers activate a window the app itself
deiconifies; the manual Linux smoke run covers the minimized case), and
an `Ok` from the sequence is not a guarantee the OS applied focus — the
handler logs what was attempted and what failed, not what the window
manager eventually did. A missing window (label absent — with tauri's
defaults closing the last window exits the process, so this arises on
shutdown races or, with a future tray icon, in a windowless running
instance) is logged and returns `NoMainWindow` — no panic: the callback
runs inside the healthy first instance, and a window-state failure must
not crash it, so individual operation errors are logged and skipped as
well, and diagnostics go to stderr lossily (`eprintln!` would panic on a
broken stderr, and on Windows the callback runs in an `extern "system"`
window procedure where a panic aborts the process). Unit tests run on the
tauri mock runtime and pin the handler's own seam — window selection by
label in both directions (the tests create windows from the literals
`"main"` and `"preview"`, never from the constant, so a mutated lookup
cannot satisfy its own test, and a lookup that picks any existing window
fails the non-main-window test) and the no-panic missing-window case; the
mock dispatchers make the window operations no-ops, so the inter-process
mechanism (instance lock, second-process exit, callback dispatch) and the
real focus effects are process/OS-level facts covered by manual smoke
runs and the CI build matrix, not by unit tests.

### D-026 — Windows manifest embedded via `WindowsAttributes::app_manifest`, default reproduced — 2026-11-17

Problem: NFR-023 (P0) requires long path support to be turned on through
the Windows manifest, and SEC-012/NFR-011 (P0) requires the app to run
with user rights without asking for elevation; the scaffold's `build.rs`
called plain `tauri_build::build()`, so the binary carried whatever
tauri-build embeds by default. Before writing any code the tauri-build
2.7.5 sources were read to settle the key question — replace or merge:
`WindowsAttributes::app_manifest` REPLACES the default manifest
wholesale (`self.app_manifest = Some(...)`, tauri-build 2.7.5
`src/lib.rs`), and the vendored default
(`windows-app-manifest.xml`) carries exactly one entry — the
Common-Controls v6 dependency (`Microsoft.Windows.Common-Controls`,
`publicKeyToken=6595b64144ccf1df`), which the tauri-build docs flag as
required for dialog APIs. There is no merge mode, and nothing else
(dpiAware et al.) lives in the default — DPI is handled by Tauri at
runtime, not by its manifest.

Solution: a dedicated file `src-tauri/windows/app.manifest` (ASCII-only,
one element per line, no XML declaration — tauri-winres embeds it
line-by-line as quoted RC strings, so the embedded bytes start with a
space and the declaration, which must be the first bytes of the document,
would risk rejection by the Windows manifest parser; tauri-build's own
default manifest ships without one too) reproduces the Common-Controls
dependency verbatim (the dialog plugin is wired, T-007) and adds the two
T-009 settings: `<requestedExecutionLevel
level="asInvoker" uiAccess="false" />` (SEC-012/NFR-011 — an explicit
level also disables the Windows "installer detection" heuristic) and
`<ws2016:longPathAware>true</ws2016:longPathAware>` inside the SMI/2016
settings namespace (NFR-023). `build.rs` wires it with
`WindowsAttributes::new().app_manifest(include_str!(...))` and
`tauri_build::try_build(Attributes::new().windows_attributes(...))`,
plus `cargo:rerun-if-changed=windows/app.manifest`: tauri-build emits
narrower rerun directives (config files, capabilities, resources — the
icon watch lives behind the `codegen` feature this crate does not
enable) that do not cover the manifest file, and `include_str!` is
opaque to Cargo, so without the directive manifest edits would never
rebuild the binary. Config pin notes: `config_tests.rs` previously
banned the substring `app_manifest` in `build.rs` (D-024 — it guards
against an app ACL manifest, i.e. `Attributes::app_manifest` with the
`AppManifest` struct); the Windows manifest legitimately uses the same
method name on an unrelated type, so that pin is narrowed to what D-024
actually means — no `AppManifest` naming plus exactly one
`.app_manifest(` call (the Windows one) — while the `permissions/`
directory check stays untouched. Honest testing boundary: on Linux the
build script compiles and runs, but tauri-build's Windows-resource
branch never executes (gated on the target triple at runtime), so the
Linux suite can only pin manifest text and build.rs wiring
(mutation-checked tests do exactly that). No automated check validates
the embedded result anywhere: `rc.exe` embeds the manifest as opaque
bytes without parsing the XML, and the CI `build` job on Windows
(`tauri build --debug --no-bundle`) only proves compile+link — it never
runs the binary — so the two runtime criteria — >260-char project paths
accepted, no UAC prompt — rest on the manual Windows smoke run; its
two-step procedure (project beyond 260 characters with
`LongPathsEnabled=1`; no UAC prompt plus `sigcheck64 -m`/`mt.exe`
extraction of the embedded manifest) is written down in
ARCHITECTURE.md, Platform notes. A CI step doing that extraction
automatically would close the gap; it is consciously deferred — CI
workflow changes belong to a release/CI task, not a manifest task.
One environment caveat is accepted, not engineered around:
`longPathAware` is the app-side half only — the OS must also have the
`LongPathsEnabled` policy on (off by default on Windows 10/11; spec §15
tests long paths per platform), which no manifest can switch by itself.

### D-027 — Data directory layout created at every startup; failure aborts the launch — 2026-11-17

Problem: spec §5.5 requires `db/`, `db-backups/`, `backups/` and `logs/`
under `app_local_data_dir()`, and the creation runs at every startup —
it has to survive restarts, crashes halfway through the creation loop and
users deleting part of the tree — while it also had to be decided what
happens when the directories cannot be created at all (permissions, a
regular file occupying the path).

Solution: one idempotent seam, `data_dirs::ensure_data_layout(root)` in
`src-tauri/src/data_dirs.rs` — `std::fs::create_dir_all` for the root and
each of the four subdirectories. Existing directories are a no-op and
missing ones are created, so every startup converges to the full layout
regardless of what a previous run left behind (spec 5.5 tip: deleted data
is simply recreated as the fresh, empty state). `config.toml` is
deliberately not part of the layout step: SYS-011 assigns it to the
config task, which runs after the directory exists. When creation fails,
the error propagates out of the `setup` hook and tauri panics in the
event-loop `Ready` handler with `Failed to setup app: error encountered
during setup hook: …` — the panic prints the `Display` chain, so the
offending path and OS cause reach the console: without the layout there
is no database, no logs and no config to run with, and no diagnostics
channel exists yet (tracing lands with the logging task), so a clear
startup failure beats degrading silently. The seam lives in src-tauri
rather than eunomia-fs for now: it is app-startup orchestration with a
single call site and src-tauri has no eunomia-fs edge yet — move it when
the first second consumer (backup or DB tasks) appears. Logs stay in
`logs/` inside the data directory on every OS — the macOS Tauri
`app_log_dir()` default (`~/Library/Logs/…`) must not be used; the
tracing task will point the appender at `<data dir>/logs/` explicitly
and should pin that choice against a future `app_log_dir()` regression
(spec §5.5). Testing: five seam tests over `tempfile` roots with missing
nested ancestors, mirroring the clean-system state (first run creates
everything; the root holds exactly the four directories and nothing
else; a rerun is a no-op and leaves the exact set; a partially deleted
layout is completed; an uncreatable path fails and names the full path),
mutation-checked (a dropped or extra directory, a swallowed error,
non-recursive creation over the nested root, an unwired or
error-swallowing `setup` call — each caught by its test), plus a
`config_tests.rs` text pin for the `setup` wiring whose markers include
the `?` of both fallible steps, so a swallowed wiring error fails the
pin too (same honesty boundary as the plugin-registration pin).
`tempfile` joins as a dev-only dependency of src-tauri — no new crate in
the dependency graph, it is already resolved workspace-wide via
eunomia-fs. The per-OS "first launch on a clean system" GUI check is
recorded as pending in ARCHITECTURE.md (Data directory section) until
the first smoke run.

### D-028 — Config file failure policy: broken file → defaults + warning, never rewritten; IO failure aborts — 2026-11-17

Problem: SYS-012 says an unparseable `config.toml` falls back to defaults
with a warning and the app still starts, but leaves three things open:
whether the broken file gets rewritten with the defaults, what happens
when the file cannot be _read_ or _created_ at all (OS-level failure, not
a parse failure), and whether a partial file (missing `log_level`) is an
error.

Solution: `config::load_or_create` (`src-tauri/src/config.rs`) writes the
file exactly once — when it is missing, with the pinned default content,
staged in a `.tmp` sibling and renamed into place so a crash mid-write
cannot leave a torn config file that the never-rewrite policy would
preserve forever (a stale `.tmp` from such a crash is simply overwritten
by the next attempt) — and never touches an existing file's bytes,
whatever they contain: the file is user data, a typo must not cost them
the rest of the file, and rewriting would hide the problem from whoever
comes to fix it (the warning points at the exact bytes that failed). A
parse failure or an invalid `log_level` value (including a non-string)
returns in-memory defaults plus a warning (SYS-012); a missing key falls
back to its default silently (`#[serde(default)]`) — a partial file is a
valid config. The same fallback covers invalid UTF-8 bytes (e.g. a
UTF-16 file written by a Windows shell redirect or an editor): the
content cannot be parsed, which is SYS-012's domain, not the data
directory's. OS-level failures — the existing file unreadable (e.g.
mode 000), the missing file unwritable (e.g. a read-only directory), the
data dir not a directory — are `ConfigError` and abort the launch by
propagating out of `setup` (the tauri `Failed to setup app: …` panic
prints the path and cause, same as the layout step, D-027): a data
directory that cannot hand over its config file is broken the same way
as one that cannot hold the database, and there is no diagnostics
channel yet to degrade into. The seam deliberately does not create
parent directories: `ensure_data_layout` runs first and owns that (the
order is pinned in `config_tests.rs`), so a wiring regression that calls
the config seam before the layout fails loudly instead of silently
re-deriving the layout's responsibility. Testing: thirteen seam tests
over `tempfile` (defaults created and pinned byte-for-byte, rerun no-op,
all five SYS-012 spellings, partial file, unknown keys, broken TOML,
invalid value, non-UTF-8 bytes, not-a-directory read failure, read-only
directory write failure and unreadable file — the last two unix-gated
and skipped as root, which bypasses permission bits — plus the
known-keys/diff agreement and a failing-writer test for the
no-panic warning writer), mutation-checked (mutated default content,
wrong fallback level, suppressed unknown-key warning, dropped
`app.manage`, swapped layout/config order, UTF-8 fallback turned into a
hard error, write errors swallowed into pretended success, creation
branch removed, emptied known-key list, panicking warning writer — each
caught by its test), plus the wiring text pin.

### D-029 — Config seam shape: warning Vec + lossy stderr before tracing; unknown keys via `toml::Value` diff; `AppConfig` as managed state — 2026-11-17

Problem: the config is read before any log subscriber exists (tracing is
the next startup task, T-012), so its warnings — unknown keys ignored,
broken-file fallbacks (SYS-012) — had no channel; unknown-key detection
needed a mechanism (`serde_ignored` crate vs a manual diff); and the
logging task needed a defined way to read the loaded config back.

Solution: three pieces. First, the seam returns
`Result<(AppConfig, Vec<String>), ConfigError>` — warnings are data, so
they are unit-testable; the `setup` wiring prints them to stderr lossily
(`writeln!` with the error ignored — a broken stderr must not abort the
launch, the same policy as `single_instance.rs`; the writer is an
extracted `write_warnings` so the no-panic guarantee has a failing-writer
test) until the tracing subscriber gives warnings a real destination,
and the criterion "`log_level` is applied to the log" is therefore
deferred to that task, which reads `app.state::<AppConfig>()`. Second,
unknown keys are found by diffing the parsed `toml::Value`'s top-level
keys against the `KNOWN_KEYS` list — kept in sync with `AppConfig`'s
fields by a dedicated test that serializes the default config and
compares the field-name set to the list in both directions (the config
types derive `Serialize` for exactly this check; the diff-side half
parses a known-keys-only document expecting zero warnings) — and
warning per unknown key —
`serde_ignored` was rejected as an
extra dependency for a check a handful of lines covers (new crates are
task decisions, D-012 policy); only top-level keys are named, the
content of an unknown table is not enumerated, and the same parsed value
then deserializes into `AppConfig`, so an invalid enum value lands in
the same fallback path. Third, the config is published with
`app.manage(config)` — tauri's state container is the idiomatic seam a
Tauri command or handler reads from (`app.state::<AppConfig>()`),
survives the whole app lifetime, and needs no global static; the wiring
is pinned in `config_tests.rs`. The `toml = "1"` dependency reuses the
1.2.1 already resolved in the lockfile (via tauri), so no new crate
version enters the graph (cargo-deny green). The on-disk key spelling is
the spec's literal `log_level` (snake_case); the `LogLevel` enum is
lowercase-serialized so the five SYS-012 words are the only accepted
values.

### D-030 — `open_db` applies the DB-001 pragma set; pragma defaults make three of four mutations unobservable — 2026-11-17

Problem: spec DB-001 requires `journal_mode=WAL`, `foreign_keys=ON`,
`busy_timeout=5000` and `synchronous=FULL` on every database open, but
the store crate only had `run_migrations` (which takes an already-opened
connection). Three details had to be settled: how `busy_timeout` is
applied (rusqlite offers both the pragma and a method), who creates the
parent directory of the database file, and how the acceptance test
("reads the pragmas after opening and sees all four values") relates to
mutation checking.

Solution: `eunomia_store::storage::open_db(&Path) -> Result<Connection,
StorageError>` applies the full set on every call, in a deliberate
order: `busy_timeout` first (so the WAL switch and every later step wait
on a lock instead of failing immediately), then `journal_mode=WAL`
(verified against the mode SQLite reports back via
`pragma_update_and_check` — a file that cannot use WAL, e.g. on a
network filesystem, fails the open with the new
`StorageError::JournalMode` instead of silently running without the
NFR-007 crash-safety guarantees), then `synchronous=FULL`,
`foreign_keys=ON`. Two more decisions: (1) `busy_timeout` is set via
`Connection::busy_timeout(Duration)` rather than the pragma — it is
type-safe (a `Duration`, not a `ToSql` value in a SQL string) and
programs the same SQLite busy handler the pragma does, so
`PRAGMA busy_timeout` still reads back 5000; (2) `open_db` never creates
parent directories — the directory layout is the app layer's job
(`data_dirs::ensure_data_layout`, D-027), and a missing parent surfaces
as the new `StorageError::Open` variant, which names the offending path
(the same contract as `Connection::open` itself, made explicit by a test
that asserts both the error variant and the path in the message).
`StorageError` grew the two variants (`Open { path, source }`,
`JournalMode { path, mode }`); `run_migrations` and its existing
variants are untouched.

Honest testing boundary, verified empirically against the bundled build
(rusqlite 0.41.1, libsqlite3-sys 0.39.1 as locked by the workspace): a
plain `Connection::open` on
a fresh file already reports `foreign_keys=1` (the bundled SQLite
compiles with `SQLITE_DEFAULT_FOREIGN_KEYS=1`), `busy_timeout=5000`
(rusqlite programs it on every open) and `synchronous=2` (the SQLite
compile-time default; `SQLITE_DEFAULT_WAL_SYNCHRONOUS` equals it, so
entering WAL does not change the value). Consequently, of the four
pragma-removal mutations only `journal_mode` is caught by the
integration test (a fresh file defaults to `journal_mode="delete"`);
removing `foreign_keys`, `busy_timeout` or `synchronous` keeps the
suite green because the observable state is identical to the required
one. The read-back assertions still pin the observable DB-001 contract
and start failing the moment a library change moves those defaults —
which is exactly when a missing explicit pragma would silently regress
the spec. The limitation is documented in the test file itself; no
implementation-coupled (source-grepping) test was added to fake the
remaining mutations, per the behavior-not-implementation principle.

Migration interaction, recorded for the first schema-change task:
`run_migrations` runs on the connection `open_db` returned, i.e. with
`foreign_keys=ON`, and rusqlite_migration 2.7 wraps every migration in
its own transaction, where `PRAGMA foreign_keys` is a no-op (its
documentation tells callers to switch the pragma OFF around the run and
offers `Migrations::foreign_key_check()` for validation). That is safe
for `0001_init.sql` (object creation only, no foreign keys), but it
blocks the standard twelve-step table-rebuild pattern: the first
rebuild-style migration must decide its own foreign-keys policy (switch
the pragma OFF around the migration run, optionally with a foreign-key
check) — that decision and the `run_migrations` behavior change belong
to that task, not to T-014; the documented contract lives in the
`run_migrations` doc comment.

Addendum (2026-11-18, T-017/D-038): the "wraps every migration in its
own transaction" claim above holds only when a single migration is
applied per call — `to_latest` batches ALL pending migrations into one
transaction (`goto_up`), and D-038's stepping loop is what actually
guarantees DB-004's per-migration transaction for multi-step hops.

Also recorded: `open_db` creates a new database file with the OS
default mode (`0644` under a typical umask on Unix), so the SEC-007
requirement (DB file `0600`) is not implemented here and needs a
dedicated task before release.

(D-028 and D-029 are held by the parallel configuration task, T-011.)

### D-031 — UUIDv7 and UTC timestamp helpers: uuid `now_v7` monotonicity, minimal chrono features — 2026-11-18

Problem: DB-002 (spec §6.1) fixes row identifiers as UUIDv7 TEXT and
timestamps as UTC ISO-8601 strings with milliseconds
(`2026-11-17T13:03:22.123Z`), and the tech table (§5.3) names `uuid` v7 and
`chrono` — adding both is a dependency decision (D-012 policy). The card
(T-015) also demands a monotonicity guarantee that survives
same-millisecond generation: RFC 9562 orders UUIDv7 across milliseconds
by construction, but within one millisecond the order depends on the
generator (a counter per RFC 9562 §6.2, "Monotonicity and Counters" —
not every library ships one).

Solution: three parts. (1) The helpers live in the new `eunomia-store`
module `values` — `new_id()` and `now_utc()`, both returning `String`:
TEXT is the storage contract, and no newtype wrapper is invented before a
first consumer exists. (2) Monotonicity rides on the `uuid` crate's
documented contract instead of a hand-rolled counter:
`Uuid::now_v7()` (uuid 1.29.3) guarantees "all UUIDs generated through
this method by the same process are guaranteed to be ordered by their
creation", implemented with a process-wide `Mutex<ContextV7>` shared
context — a 42-bit counter reseeded with 41 random bits at each new
millisecond, incremented within the millisecond, with backward clock
jitter absorbed (the context keeps the maximum observed timestamp and the
counter keeps running) and counter overflow handled by bumping the
timestamp (the RFC 9562 §6.2 counter method; verified in the crate
sources, `src/v7.rs` and `src/timestamp.rs`). The guarantee is per-process
only; the module docs state that nothing may rely on cross-process
ordering, and that after a backward clock step an id's embedded time can
run ahead of the wall clock — the guarantee covers ordering, so an id is
never a time source. The monotonicity tests are deterministic, not
timing-dependent. A 1000-generation tight loop completes inside one
millisecond on any realistic machine, so it pins the same-millisecond
counter path; a companion test sleeps 2 ms between two generations
(`thread::sleep` sleeps at least the requested duration, so a
millisecond boundary necessarily passes) and pins the cross-millisecond
timestamp-bits path. Both regimes are ordered by the same per-process
contract. (3) Formatting:
`Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)` — always
exactly three fractional digits and always the `Z` suffix (the UTC offset
is zero by construction); chrono formats `nanos / 1_000_000`, so
sub-millisecond precision truncates toward zero rather than rounding.
That truncation edge is documented in the helper's doc comment but not
pinned by a test — pinning it would require clock injection and a seam
for it is not invented here (the same honest-boundary reasoning as
D-030's pragma defaults).

Dependencies: `uuid = "1"` with feature `v7` (rng-backed counter
reseeding on top of default `std`), and `chrono = "0.4"` with
`default-features = false, features = ["now"]` — `now` (= `std` +
`alloc`) covers `Utc::now()` and `to_rfc3339_opts`; the `clock` feature
(which would pull `iana-time-zone` for local time — that serves the
app-layer creation marker per spec §5.3), the wasm bindings, and the
empty legacy `oldtime` feature are not requested. Lockfile facts,
corrected during the round-1 review: uuid 1.29.3 was already an active
dependency workspace-wide (via tauri's ecosystem), so that edge adds
nothing to the build; chrono 0.4.45 sat in Cargo.lock only as an unused
optional edge of serde_with — nothing compiled it before, so this task
puts chrono into the build graph for the first time (still no new crate
_version_ — the same lockfile-reuse pattern as D-029's `toml = "1"`).
One cargo-deny consequence: chrono carries RUSTSEC-2026-0543
(unmaintained, no safe upgrade — all versions affected; the advisory
recommends jiff or `time`). With no previous workspace dependent there
was nothing being tolerated before; the new direct edge is what trips
deny.toml's `unmaintained = "workspace"` rule, so deny.toml carries a
scoped `ignore` for that advisory ID with this entry as its reason.
Switching the time library is a spec-level decision (§5.3 names chrono),
not a task-level one; the ignore keeps the gate meaningful for every
other unmaintained direct dependency.

Addendum (2026-11-18, T-239): the ignore above turned out to be
temporary — chrono was replaced with jiff per D-035, the direct
dependency (and with it the deny.toml ignore and its comment block) is
gone; see D-035 for the lockfile detail.

### D-032 — File logging: tracing-appender daily rotation in `<data dir>/logs/`; SEC-008 enforced by a logging facade — 2026-11-18

Problem: T-012 turns the spec §5.3 "Logs = `tracing`, `tracing-appender`,
file rotation" row into a startup step, and several decisions were open:
which crates enter the graph and with what features; where the file lands
and how it is named (§5.5 fixes `logs/eunomiasync.<date>.log`, while
Tauri's `app_log_dir()` would move it to `~/Library/Logs/…` on macOS);
what happens when the log file cannot be created (the layout and config
steps abort the launch — D-027/D-028); where the non-blocking writer's
guard lives; and how SEC-008 ("instruction text and file content never
go into logs; paths do") can be _enforced_ rather than remembered, given
that the variant-save and sync features the card's acceptance test names
do not exist yet.

Solution: `src-tauri/src/logging.rs` owns everything. Dependencies:
`tracing = "0.1"` reuses the 0.1.44 already resolved via tauri (no new
version in the graph, same pattern as `toml = "1"` in D-029);
`tracing-subscriber = { 0.3, default-features = false, features =
["fmt"] }` — no ANSI escapes in log files and no `env-filter`, because
SYS-012 pins the level to `config.toml`, not `RUST_LOG`;
`tracing-appender = "0.2"` (0.2.5) for the rotation. File name: the
builder API with prefix `eunomiasync` + suffix `log` under
`Rotation::DAILY` yields exactly `eunomiasync.<YYYY-MM-DD>.log` (the date
stamp and the rotation boundary are UTC — `tracing-appender` formats
`now_utc()`); the appender is pointed at `<data dir>/logs/` explicitly,
never at Tauri's `app_log_dir()` (§5.5), and a `config_tests.rs` absence
pin over comment-stripped sources keeps that API out of live code.
Ordering (SYS-011): the config loads first, `logging::init` runs after
it and reads the level through `app.state::<AppConfig>()` — the seam
D-029 documented — so applying `log_level` to the log output is behavior
driven end to end by tests. Failure policy: logging is diagnostics, so
`init` never aborts the launch — on an appender failure it degrades to a
stderr fmt subscriber with the same level filter and emits one `error`
event naming the failing path (deliberately weaker than the
layout/config abort: those guard data the app cannot run without). The
non-blocking writer's `WorkerGuard` is returned inside a `Logger` value
that `setup` stores as managed state: it keeps the writer thread alive
for the whole run (dropping it early would silently stop the file log —
see the honest-boundaries paragraph below for why the flush-at-exit
intuition is wrong). The config warnings
keep flowing as the `Vec<String>` D-029 introduced (the config still runs
before any subscriber exists); `config::emit_warnings` re-emits them
through the facade after `init`, replacing the interim lossy-stderr
channel, and `single_instance.rs` switched its stderr diagnostics to the
facade the same way.

SEC-008 enforcement is structural, in three layers. First, every tracing
event of the app is constructed _in_ `logging.rs`: a `config_tests.rs`
pin scans `src-tauri/src` and `crates/**` (raw text, comments included —
conservative like the devtools/tray pins; `logging.rs` itself and
`*_tests.rs` files are exempt, the latter because cfg(test) code never
ships without the production subscriber) and fails on `error!(`/`warn!(`
/`info!(`/`debug!(`/`trace!(`/`event!(`/`tracing::instrument`/
`#[instrument` anywhere else — so a future task cannot interpolate
content into a message, and `#[instrument]`'s auto-captured fields (which
would happily serialize a `content: String` argument) are banned with it.
Second, the facade's signatures are the vocabulary: paths, closed-vocab
operation codes, numeric IDs. Third, the card's acceptance criterion
("at variant save and sync, text stays out of the log") is honestly
adapted to today's code: those features do not exist, so the facade
ships their logging primitive now — `file_event(FileOp, &Path,
Option<u64>)` with `FileOp ∈ {variant_saved, variant_restored, synced,
backed_up}` — marked `#[allow(dead_code)]` until the first of those
tasks calls it (tested today, consumed later), and the test drives it
exactly the way those tasks will: with a realistic instruction text
living in the same scope, the event carries the path, the ID and the
operation code while every marker of the text stays out of the file.
The one deliberate content exception: config warnings may quote the
offending bytes of `config.toml` itself (D-028's "the warning points at
the exact bytes that failed") — the machine-level config is
user-written diagnostics input, not instruction text or library file
content, which is what SEC-008 protects; the facade documents this.

Honest boundaries, mirrored in `ARCHITECTURE.md` (Logging section): the
midnight rollover cannot be moved into a unit test — the dated file name
proves the rotation wiring, the crossing itself rests on the crate plus
the manual smoke run; the global `set_global_default` install is
process-wide and thus not installable from the parallel test suite —
tests drive the identical subscriber via `with_default` over `tempfile`
roots and read the real file back, and the `setup` wiring is pinned
textually instead (the pin's markers now cover the full SYS-011 order:
layout → config → `app.manage` → level read → `logging::init` →
`app.manage(logger)` → warnings). The round-1 review tightened three
more edges, recorded here with their rationale: both subscribers set
`log_internal_errors(false)` (the fmt layer's writer-error fallback is
an `eprintln!` that panics on an unusable stderr — fatal under
`panic = "abort"` in a Windows GUI build, i.e. exactly in the degraded
state the fallback exists for; a failing-writer test pins the no-panic
contract); the degradation event is `error`, not `warn` (at
`log_level = "error"` a warn would be filtered by the very filter it
reports about); and the "guard flushes at exit" intuition is recorded
as false — tauri's event loop ends in `std::process::exit`, which skips
destructors, so the managed `Logger` never drops and the worker's
per-batch flushing is what persists lines (the guard's real job is
staying alive so the writer never stops early). The macros pin exempts
`src-tauri/src/logging.rs` by full path (a bare-name exemption would
silently exempt a future `crates/*/src/logging.rs`, which cannot import
this module and needs its own discipline), includes the `span!(` family
(one substring needle covers every `*_span!` spelling) and scans the
`src-tauri` build script alongside `src/` and `crates/**`. No retention
limit is configured for
the file log (`max_log_files` unset): spec sets none for the tracing log
(LOG-006's limits govern the DB `action_log`), so daily files accumulate
until a task decides otherwise — a conscious omission, not an oversight.
`cargo-deny` is green for the two new crates.

### D-034 — AppError: the §7.10 wire DTO with a 20-code registry, exported via `Builder::typ` — 2026-11-18

Problem: T-013 turns §7.10's `AppError` (every command returns
`Result<T, AppError>`) into code, and left open: the field shapes
(`params` vs `details`, where the OS error code lands); the form and
ownership of `message_key`; how the type reaches `src/bindings.ts` while
the only registered command (`greet`) cannot fail; whether `thiserror`
joins; and how the two P1 codes of the §10 table relate to the enum.

Solution: `src-tauri/src/errors.rs` (pub(crate) — the app shell owns the
IPC contract; `eunomia-core` stays pure logic with no specta edge).
Four decisions inside it. (1) `ErrorCode` is a closed 20-variant enum —
exactly the card's MVP list — with per-variant `#[serde(rename)]`, so
the wire form IS the spec §10 literal (`"E_SYNC_RUNNING"` …); explicit
renames keep the contract greppable, and the two P1 codes
(`E_RESTORE_SOURCE_MISSING`, `E_IMPORT_INVALID`) are deliberately absent
until their tasks land (documented in the module docs, pinned absent).
(2) `message_key` is a `String` field, not an enum: one code can reach
several messages (the `E_VALIDATION` family names the failed field), so
the key is per-error; `ErrorCode::message_key()` supplies the stable
default (`errors.<snake_case>`, the UI-020 key style), installed by
`AppError::new`. The key strings are the frontend contract (NFR-006:
the UI localizes by key and interpolates params — never backend prose);
both the defaults and the code literals are pinned by tests with
hardcoded spec-derived strings. (3) `params` is a `BTreeMap<String,
String>` — interpolation values (UI-011's "where": path, project),
string-typed for locale interpolation, ordered for deterministic wire
form and logs. The string-only value type is a recorded policy, not an
oversight: UI-020's ICU plurals take the count as an interpolation
argument and vue-i18n accepts digit strings there, so numeric params
travel as strings rather than widening the wire type to a JSON value
union (a weaker TS record, and no MVP message needs one; widening later
is a coordinated contract change, exactly like renaming a key). The OS
facts do NOT live there: `details` is a typed
`ErrorDetails { os_code, os_kind, os_message }` — UI-011's collapsed
"why" section — attached by `AppError::with_os_error` from
`raw_os_error()` with `ErrorKind`'s Debug name as the portable fallback
(BKP-005: "OS code or error type"). §10 makes the OS code a first-class
fact of every error log line; burying it among interpolation params
would force the UI to fish diagnostics out of message placeholders.
`ErrorDetails` is deliberately OS-shaped for the MVP; future technical
facts of non-OS failures (the expected/actual hash of `E_VERIFY_FAILED`,
`quick_check` output of `E_DB_CORRUPT`) extend it additively — optional
fields are null on old producers and ignored by old consumers — and the
error-UI task must render `details` inside the message block (not only
behind the collapsed section) where BKP-005 demands the OS code in the
visible message.
(4) No `From<std::io::Error>`: which code an OS failure maps to is
context-dependent (a permission error is `E_NO_WRITE_ACCESS` in
preflight but `E_BACKUP_FAILED` in the backup phase), so conversion is
an explicit constructor call at the future command boundaries — and no
`thiserror`: the type is a serialization DTO, neither `Display` nor
`std::error::Error`, nothing propagates it with `?` (the crate's
internal failures keep their own enums, like `ConfigError`/`StorageError`).

Reachability: the specta builder registers the type directly —
`Builder::typ::<AppError>()`, tauri-specta's documented mechanism for
types not reachable from a command signature. The alternative (faking
`Result<String, AppError>` on `greet`) was rejected: it would advertise
an error path that never occurs, and `greet` keeps its honest `String`
return until real fallible commands replace it. A side effect recorded
for consumers: specta-typescript emits each serde struct as an
`X | X_Serialize | X_Deserialize` alias pair (the sides differ on
optional fields), so `AppError` appears in bindings.ts as three type
declarations sharing the doc comment.

Enforcement: `errors_tests.rs` pins the wiring (`.typ::<…>()` exactly
once in comment-stripped `lib.rs` — without the pin, dropping the
registration would leave `export_bindings_are_up_to_date` green while it
silently rewrites the committed bindings without the type; verified by
mutation) and the committed `src/bindings.ts` (embedded at compile
time): the `AppError`/`ErrorCode`/`ErrorDetails` declarations with the
four §7.10 field spellings, all twenty literals present in the
`ErrorCode` union, exactly twenty `E_*` literals in the union (both
directions), and the P1 literals absent (over doc-stripped text —
specta exports Rust doc comments into the bindings, and they legitimately
contain `;` (the `E_DB_CORRUPT` doc) which would otherwise truncate the
union, and may legitimately mention codes which must not trip the
absence pin). Behavior tests in `errors.rs` cover the serde round-trip
(params/details/null-details), the literal↔variant mapping for all
twenty codes, the stable default keys, and the card's OS criterion end
to end (errno 13 → serialized `details.os_code == 13`, kind fallback for
code-less I/O errors). `ErrorCode::as_str()` gives the producing tasks
the §10 literal without going through serde, pinned equal to both the
serde output and the spec table; the default-key mapping is matched
exhaustively inside the test module, so a new variant fails compilation
of the suite until the registries are extended (round-1 review
tightening; the same review also made `export_bindings_are_up_to_date`
compare the regenerated file against the compile-time-embedded one: the
first run after a contract change is red and performs the regeneration,
and CI stays red on a stale committed file). The
module has no production caller yet — `#[allow(dead_code)]` with the
tested-today/consumed-later note, the same shape as the `logging.rs`
SEC-008 facade; error logging itself stays with the producing tasks
(D-032), and the first fallible-command task adds the `ipc_tests.rs`
case asserting the rejected invoke payload equals the §7.10 shape (the
DTO's serde behavior is proven today; the Tauri rejection envelope is
not, because no command returns it yet). No new dependencies: serde,
specta, serde_json were already resolved.

### D-035 — Time library: chrono replaced with jiff 1.x — 2026-11-18

Problem: chrono is RUSTSEC-2026-0543 (unmaintained, all versions affected,
no safe upgrade; the advisory names jiff and `time` as the recommended
replacements). D-031 had introduced it as a direct `eunomia-store`
dependency under a scoped deny.toml ignore and declared the library switch
a spec-level decision; T-239 executes that switch (spec §5.3 pinned chrono
plus iana-time-zone, so the tech-table row changes too).

Solution: `jiff = { version = "1", default-features = false,
features = ["std"] }`, chosen over `time` on facts checked 2026-11-18:

- Maintenance: jiff 1.0.0 was published 2026-11-15 and 1.0.1 on 2026-11-18,
  ten releases in the first eighteen days of November (BurntSushi, 288M
  downloads); `time` 0.3.55 was published 2026-08-01 — no release in the
  3.5 months since. Both lines are alive; jiff is the demonstrably more
  active one at the moment of the choice.
- Advisories: jiff has no RUSTSEC entry (no `crates/jiff/` in the advisory
  database). `time` has two: RUSTSEC-2020-0071 (patched in 0.2.23, 0.3.x
  unaffected) and RUSTSEC-2026-0009 (stack-exhaustion DoS, affects
  0.3.6–0.3.46, patched in 0.3.47 — the lock-resolved 0.3.55 is past that
  patch). For jiff — the crate actually in the graph — the verification is
  the green `cargo deny check` with no ignores left in deny.toml. Both
  libraries are the two replacements the chrono advisory itself
  recommends.
- DB-002 format: `Timestamp::now().strftime("%Y-%m-%dT%H:%M:%S%.3fZ")`
  formats in UTC with always exactly three fractional digits and the `Z`
  suffix; jiff documents that a precision setting truncates
  ("When using a precision setting, truncation is used" — `fmt::strtime`
  module docs, verified in the 1.0.1 sources), preserving the truncation
  semantics D-031 documented for chrono's `SecondsFormat::Millis`. The
  values contract is unchanged; of its five tests only the parse inside
  `now_utc_returns_the_current_time` moved from
  `chrono::DateTime::parse_from_rfc3339` to `Timestamp`'s RFC 3339
  `FromStr` — the checked behavior (RFC 3339 round-trip, millisecond
  resolution, ±1 s window) is the same.
- Time zones (spec §5.3's iana-time-zone perspective): jiff has native
  IANA time zone support (TZif parsing and system-zone detection via the
  `tz-system`/`tzdb-*` features), so the creation-marker need the spec
  attributed to `iana-time-zone` is covered by jiff alone and the
  separate crate leaves the tech table. `time` has no IANA support (UTC
  offsets only) and would have kept the extra-crate dependency.
- Features and graph: `features = ["std"]` compiles exactly two crates —
  jiff and jiff-core, pure Rust, no proc-macros (jiff's
  portable-atomic(-util) dependencies are gated on
  `not(target_has_atomic = "ptr")` targets and do not build on the
  project's desktop targets). A `time` equivalent for the same fixed
  format needs std, formatting and macros, compiling `time`,
  `time-core`, the `time-macros` proc-macro, `deranged`, `num-conv` and
  `powerfmt`.
  jiff's tz features stay off until the creation-marker task needs them
  (minimal features now; that task turns on what it uses).
- Line choice: jiff 1.0 over the already lock-resolved 0.2.46. 0.2 is the
  legacy line — its last feature release shipped simultaneously with
  1.0.0 — and pinning a new permanent dependency to a superseded line
  would schedule a repeat of this very migration. The cost is three new
  Cargo.lock entries (jiff 1.0.1, jiff-core 1.0.1, jiff-static 1.0.1, the
  last an unused optional edge that never compiles), all
  Unlicense OR MIT, license-checked by the same green deny run.

Lockfile facts: removing the direct dependency removes chrono from the
build graph (`cargo tree -i chrono` matches nothing) but not from
Cargo.lock — chrono 0.4.45 remains as serde_with's unused optional edge,
the exact pre-D-031 status quo: resolved, never compiled. With no
workspace member depending on chrono, deny.toml's
`unmaintained = "workspace"` rule no longer fires, so the
RUSTSEC-2026-0543 ignore and its comment block are deleted rather than
left as a dead exception; `cargo deny check` passes with zero ignores.

### D-036 — Schema v1: the §6.2 DDL kept verbatim, two constraints strengthened — 2026-11-18

Problem: T-016 turns the `meta`-table stub of `0001_init.sql` into the
full version-1 schema of spec §6.2 (eleven tables). The spec preamble
allows adjusting column names but not constraints and relations, and two
places needed a real decision: which half of DB-005 ("integer `position`
starting from 1, with no gaps") is enforceable in DDL, and a
three-valued-logic hole in the reference `library_items` CHECK.

Solution: the migration implements the §6.2 DDL verbatim — no column
renames were needed — with two deliberate deviations, both strengthening
(not weakening) constraints. (1) `library_items.position` carries
`CHECK (position >= 1)` as the DB-enforceable floor of DB-005; the
no-gaps invariant (DB-011) stays in the service layer, which renumbers
the affected range in one transaction. (2) The first table-level
`library_items` CHECK spells out `heading_level IS NOT NULL` before
`heading_level BETWEEN 2 AND 6`: under SQL three-valued logic the §6.2
expression alone accepts a heading with a NULL level (`NULL BETWEEN 2
AND 6` is NULL, and a CHECK only rejects FALSE), defeating the
constraint's evident intent — the heading/level matrix is now fully
enforced. The two extra `project_instructions` indexes the spec's
closing paragraph requires (on `instruction_id` and `variant_id`) are
named `ix_project_instructions_instruction_id` /
`ix_project_instructions_variant_id` (index names are an implementation
choice). The stub's `meta` table is gone: application key/value state
lives in `settings` (§6.5); the three tests that used `meta` moved to
`settings`.

Testing contract: the negative tests in `tests/schema_v1.rs` cover every
CHECK, UNIQUE, PK and FK of the schema and pin the SQLite extended
result code of each rejection (`SQLITE_CONSTRAINT_CHECK`, `_UNIQUE`,
`_PRIMARYKEY`, `_FOREIGNKEY`, `_NOTNULL` via the `rusqlite::ffi`
re-export — no new dependency), so a broken statement, a wrong
constraint class or a silently missing constraint all fail the suite
(the schema's NOT NULL columns are deliberately only sampled — one
representative rejection — since the acceptance matrix names
CHECK/UNIQUE/FK, not NOT NULL).
Empirical SQLite facts the tests encode (bundled build, rusqlite 0.41.1
/ libsqlite3-sys 0.39.1): a RESTRICT-blocked parent DELETE reports
`SQLITE_CONSTRAINT_TRIGGER` (1811) while an FK violation on INSERT (and
on parent-key UPDATE under the default NO ACTION) reports
`SQLITE_CONSTRAINT_FOREIGNKEY` (787); for `instruction_variants` a
duplicate id surfaces through the composite `UNIQUE (instruction_id, id)`
index (`_UNIQUE`, 2067) rather than the PK autoindex; `PRAGMA
foreign_key_list` returns one row per referenced _column_, so a
composite FK counts once via `COUNT(DISTINCT id)`; `AUTOINCREMENT` adds
`sqlite_sequence` to `sqlite_master` (excluded from the table-set check
by the `sqlite_%` filter, as any schema introspection should).
Foreign-key tests set `PRAGMA foreign_keys=ON` explicitly instead of
leaning on the bundled default — the pragma is per-connection and the
production path (`open_db`) sets it on every connection (D-030).
Mutations verified the tests bite: removing the `scheme` CHECK,
downgrading the sync-run CASCADE to NO ACTION, and dropping the
`variant_id` index each failed exactly the corresponding test. The
round-1 review tightened the suite with its own mutation runs: the
`instruction_variants` PK is exercised across two different instructions
(the composite UNIQUE masks a same-instruction duplicate), `action_log`
pins the AUTOINCREMENT behavior (sequential ids, no reuse of deleted
ids, the `sqlite_sequence` counter) and its insert-reachable duplicate-id
PK violation, an accept-side matrix covers every member of every enum
CHECK (a narrowed enum fails it even when every reject test still
passes), the six explicit indexes are pinned by table and columns via
`sqlite_master`/`PRAGMA index_info`, and the DB-006 boundary includes a
262144-two-byte-character row so a byte-semantics regression
(`CAST(content AS BLOB)`) cannot pass unnoticed. One migration-mechanics
note: the stub is replaced in place — same file, still version 1 —
which is safe only because the app is pre-release with no installed or
shared database (D-020); a developer database created from the old stub
reports `user_version = 1` and would keep the `meta` table, so it has
to be deleted by hand.

### D-037 — DB executor: one connection behind a std Mutex, one deferred transaction per operation, spawn_blocking; greet removed — 2026-11-18

Problem: T-023 turns SYS-001/SYS-002/NFR-008 into the shared command
infrastructure: the app needs managed state with the one SQLite
connection, a helper that runs each operation inside exactly one
transaction off the async runtime and maps failures into the §7.10
`AppError`, and the template's `greet` demo command removed. Open
questions: which mutex, `spawn_blocking` vs `block_in_place`, where
the transaction boundary sits and what the operation sees, how SQLite
failures map onto the closed twenty-code registry (no generic
"infrastructure failed" code exists), what a poisoned mutex means for
the one connection, and whether a carrier command is needed to prove
the executor.

Solution: `src-tauri/src/db.rs`. (1) `Db` holds
`Arc<std::sync::Mutex<Connection>>` — the `std` mutex, not
`tokio::sync::Mutex`: the guard is locked and released inside the
blocking closure and never crosses an `.await`, and the `std` guard
being `!Send` makes holding it across a suspension unrepresentable,
while the async mutex would invite exactly that (an operation parked on
the runtime holding the only connection). (2) `Db::run` moves the
operation onto `tauri::async_runtime::spawn_blocking` — SYS-001's own
word; `block_in_place` was rejected because it panics on
current-thread runtimes and converts a worker instead of using the
blocking pool. (3) The boundary is `BEGIN DEFERRED` (via
`unchecked_transaction` — `&self`, so it works through the guard) …
`COMMIT`; the operation receives `&Transaction`, so every statement
joins the one transaction by default and a nested `BEGIN` is rejected
by SQLite at runtime (a deliberate `execute_batch("COMMIT")` escape
stays reachable through the Deref to `Connection` — the type prevents
accidents, not sabotage, and the executor's final `COMMIT` would then
fail); on `Err` the `Transaction` drop sends the `ROLLBACK`. Deferred begin keeps
`busy_timeout` meaningful: a write collision waits instead of failing
instantly (the §10 `E_DB_LOCKED` situation is "busy_timeout expired";
the retry cycle around it is T-025). (4) The base classifier
`sqlite_error_to_app_error` (shared with future command tasks, which
keep D-034's contextual refinement): busy/locked→`E_DB_LOCKED`,
corrupt/not-a-db→`E_DB_CORRUPT`, full→`E_DISK_FULL`,
readonly/perm/cannot-open→`E_NO_WRITE_ACCESS`,
constraint→`E_VALIDATION`; everything else falls back to `E_DB_LOCKED`
with the true reason preserved in `details.os_message`. The fallback is
a recorded registry gap, not a fit: `E_DB_CORRUPT` (the plausible
alternative) forces the destructive recovery screen on what may be a
transient I/O failure, while "operation rejected, retry" is the least
harmful effect the twenty codes offer; revisit when §10 grows a
generic code (with T-025). The round-1 review tightened the
diagnostics: `details.os_kind` carries the exact rusqlite category
(`SystemIoFailure` no longer collapses into an "other" label) and
`os_message` appends the numeric result code — rusqlite's `Display`
prints only SQLite's message text, which would lose
`SQLITE_CONSTRAINT_UNIQUE` vs `_FOREIGNKEY`. (5) Poisoning: an
operation panic unwinds through the `Transaction` drop first — the
rollback runs before the guard poisons — so the executor recovers the
guard with `into_inner` _and clears the poison_, and the next
operation starts from a clean connection; the panic itself surfaces
through the same fallback as an `AppError` whose `details` carry the
panic payload's own text (a cancelled task is reported as cancelled,
not panicked); release builds abort on panic before this path exists.
(6) Startup: the `setup` hook opens `Db` from the data directory
after logging — both §5.5 paths are derived inside `Db::open` through
the new `data_dirs::db_file_path`/`db_backups_dir` helpers, so the
file and its backup directory cannot drift apart at a call site — the
SYS-011 order pin in `config_tests.rs` still ends at
`config::emit_warnings`, and a new pin in `db_tests.rs` continues the
chain (logging::startup → `Db::open` → `app.manage(database)`) — and
a failure aborts the launch like the layout/config steps
(D-027/D-028), but with logging already up. Named §10 situations
recorded honestly: a corrupt file aborts as an open/migration failure
until the recovery task lands
(`db_verify`/`db_restore_backup`/`db_recreate`); a database newer
than the app already errors in the store (D-038), but the §10
`E_DB_TOO_NEW` "update the app" screen is still the app layer's to
build. Pre-migration safety copies are the store's, taken inside
`run_migrations(conn, backup_dir)` (D-038); the returned list of
copies is dropped here until the startup-log task consumes it through
the facade — and the ERR-001 _daily_ backup remains its own open
task, untouched by this change. The executor does not log: §10's
error log line needs the project/path context only the producing
command has (SEC-008 facade vocabulary, D-032).

Dependencies: src-tauri gains a direct `rusqlite = "0.41"` edge — not
a new crate or version in the graph (already built via eunomia-store;
Cargo.lock only records the new edge): the executor owns
`Connection`/`Transaction` types directly. No features are restated:
the store stays the single declaration point for the build shape
(bundled SQLite), which its version comment pins against
rusqlite_migration.

Addendum (2026-11-18, T-018/D-040): the "corrupt/too-new abort" half
of this entry's startup paragraph is narrowed by D-040 — a database
newer than the app is now rejected by a typed gate before `open_db`
(`DbInitError::TooNew` → the `E_DB_TOO_NEW` AppError, logged through
the facade, launch still aborted); a corrupt file keeps the plain
open/migration abort described here until the recovery task lands.

Addendum (2026-11-18, T-020/D-045): the corrupt half is narrowed too —
a damaged database now fails the typed `quick_check` gate
(`DbInitError::Corrupt` → the `E_DB_CORRUPT` AppError, logged through
the facade, launch still aborted); see D-045.

greet: removed entirely — command, registration, mock-IPC tests
(`ipc_tests.rs`), the demo form in `App.vue`, and the regenerated
`src/bindings.ts` (which now carries only types; tauri-specta emits no
`commands` object for an empty command set). No carrier command was
added: §7.10/D-009 fix every name, none belongs to this task (the
`db_*` names are the recovery-screen task's, `settings_*` the settings
task's), and both acceptance criteria are operation-level facts — so
they are proven by `db_tests.rs` against real database files (the
D-034/D-032 tested-today/consumed-later shape): a mid-operation error
(own and SQLite-classified) rolls back every change; concurrent
operations serialize on the connection without deadlocking or
half-interleaving (the one-connection half of SYS-002 is structural —
`Connection` is `!Sync`, so sharing it without the mutex does not
compile); while twice
the worker count of long operations are in flight, an async round-trip
probe between spawned tasks completes in a fraction of their duration
— inline work on the workers would park every worker inside the
sleep and blow the budget; a panicking operation commits nothing and
leaves the connection usable. Two wiring pins keep the mechanism
(spawn_blocking) and the startup order from silently unwiring.

### D-038 — Pre-migration `VACUUM INTO` backups; per-migration transactions require stepping `to_version` — 2026-11-18

Problem: DB-004 (spec §6.1) requires a `VACUUM INTO` copy in
`db-backups/` before a migration and its own transaction for every
migration, but `run_migrations` called `Migrations::to_latest` with no
backup at all. Five decisions were open: what counts as an "existing"
database, which version `<N>` in the file name refers to, the timestamp
format (the canonical `now_utc()` contains `:`, which a Windows file name
cannot), how the crate learns the backup directory (§5.5 puts the
data-directory layout in the app layer), and — discovered while reading
the rusqlite_migration 2.7 sources — that `to_latest` does NOT give each
migration its own transaction: `goto_up` (lib.rs 618–654) wraps the whole
`current_version..target_version` range in one transaction, so a
multi-step hop v1→v3 would commit all steps together. D-030's
"rusqlite_migration 2.7 wraps every migration in its own transaction" is
true only for one-step calls; this entry supersedes that claim.

Solution: `run_migrations(conn, backup_dir)` — the signature change is
free: every caller lived in the store crate's tests, and src-tauri had
none yet. The backup directory is an explicit parameter (the crate knows
no data directory and creates none, mirroring `open_db`'s no-parent
policy of D-027/D-030): a run needing a copy with a missing or
non-directory `backup_dir` fails with the new
`StorageError::Backup { path, source: BackupError }`
(`Directory`/`Sqlite`/`Io`) _before_ any migration is applied. The run
returns the `Vec<PathBuf>` of copies it wrote (empty for fresh and no-op
runs) so the app layer can log them through the D-032 facade without
re-listing the directory.

Semantics, decided per the card's guidance:

- _Existing_ = `PRAGMA user_version > 0` **at the start of the run**. A
  run from version 0 writes no copies at all — including when it applies
  several migrations — because a database created by this run holds no
  user state. A crash mid-bootstrap restarts as "existing" at the last
  committed step and then backs up before the next step: correctness
  over one noise copy.
- Per-step: before EACH migration of an upgrade run, one copy named
  `pre-migration-v<N>-<UTC>.sqlite3` with `N` = the **source** version —
  the copy preserves exactly that state, which is what UPD-004's rollback
  pointer and `db_restore_backup` consume. A v1→v3 hop yields
  `pre-migration-v1-*` and `pre-migration-v2-*`.
- Each migration is committed in its own transaction by stepping
  `Migrations::to_version` one version at a time (each call opens its own
  transaction, `goto_up`); this loop, not a library guarantee, is what
  satisfies DB-004's per-migration transaction. A failing step rolls back
  alone and leaves the database at the last committed step with the
  earlier steps' copies intact.
- `<UTC>` is `YYYYMMDDTHHMMSSSSSZ` — 19 characters, millisecond
  resolution, from jiff `Timestamp::now()` formatted
  `"%Y%m%dT%H%M%S%3fZ"` (jiff's `%3f` prints the fractional seconds
  without the leading dot; same jiff dependency as `values`, D-035, no
  new crate). Name collisions — two copies within one millisecond, a
  backward clock step — get `-1`, `-2`, … before the extension (capped
  at 1000 attempts), and existing copies are never overwritten, so a
  recovery copy survives a retry. The future app-layer backup names
  (ERR-001's `daily-YYYYMMDD.sqlite3`, §7.10's
  `pre-recovery-<UTC>.sqlite3`) should reuse this stamp format; the
  helpers stay private until that task lifts them.
- Crash safety: the copy is staged under `<final>.tmp` and linked into
  place with `hard_link` (the temporary is removed afterwards) — a
  rename would silently replace an existing destination on every
  supported platform, while the link fails on an occupied name, closing
  the check-then-act window of the name search. The final name thus only
  ever appears complete. The millisecond stamp makes the next run land
  on a fresh name, so leftover `.tmp` debris is swept at the start of
  every upgrade run (`pre-migration-v*.sqlite3.tmp` only — foreign
  `.tmp` files are not the crate's to delete); the same-name removal
  inside the write path covers the fixed-name tests. Filesystem
  mutations (unlink, link) retry the spec §9.5 backoff — a sharing
  violation or access denied is retried three times with 100/300/900 ms
  pauses — because an antivirus scanner briefly holding a freshly
  created file must not abort the run; other errors surface immediately,
  and once the hard link is in place the temporary's removal is
  best-effort (the copy is complete; the sweep collects the debris).
  Deliberately omitted: fsync of the staged file
  and the backup directory — not portable through `std`, and the
  producing connection already runs SQLite's own `synchronous=FULL`
  discipline (D-030); a power-loss window is accepted and the §9.5
  fsync points belong to the `eunomia-fs` atomic module (D-019) if that
  boundary ever absorbs backup writes. A retry after a crash between
  copy and migration simply writes a second, equally valid copy.
- A database newer than the set still errors
  (`MigrationDefinitionError::DatabaseTooFarAhead`, reconstructed
  explicitly because the stepping loop would otherwise treat the database
  as up to date) — the "update the app" screen (NFR-009/010) stays the
  app layer's job. One more delta from the replaced `to_latest` call,
  recorded for honesty: an empty migration set is a successful no-op
  here, where `to_latest` errored `NoMigrationsDefined`; the embedded
  set is compile-time non-empty, so the difference is unobservable
  through `run_migrations`.
- Retention: pre-migration copies accumulate without a cap (spec caps
  only the daily copies, ERR-001). Pruning belongs to the app layer's
  backup-management task (the `db_backups_list` family, §7.10) and must
  never delete the copy UPD-004 points at; nothing in this crate removes
  a final copy.

Testing boundary: the embedded set defines a single migration, so no
upgrade from an old version is reachable through the public API until a
second migration ships. The stepping/backup driver is therefore a
private `migrate(conn, &Migrations, backup_dir)` seam; its unit tests
drive it with synthetic multi-step sets (real SQL, real file databases)
through the card's acceptance criterion — the copy exists, opens with
`PRAGMA integrity_check = ok`, and preserves the source version's
`user_version` and data — plus multi-step, fresh-bootstrap, failing-step
rollback (and retry-after-failure), collision suffix, stale-`.tmp` sweep,
§9.5 retry, non-directory backup path, and missing-directory cases. The
acceptance path opens the database through `open_db`, so the copy is
produced from the production WAL/`synchronous=FULL` configuration, not
only from a rollback-journal connection. Integration tests pin the
public API's observable behavior (fresh run leaves `db-backups/` empty,
no-op rerun writes nothing, too-new rejected). Mutations verified the
suite bites: removing the backup (4 tests), batching via `to_latest`
(2 tests — one of them proving the "v2" copy would silently hold the v3
state), skipping stale-`.tmp` removal (1 test). Round-1 review
(deepseek-v4.1-flash) additionally forced the hard-link no-clobber, the
debris sweep, the §9.5 retries, Windows-safe test teardown (connections
closed before `remove_dir_all` — SQLite files are not share-deletable
there), and the WAL-configuration test. Round-2 review (hy4-flash-free)
tightened the §9.5 backoff to the full three retries (100/300/900 ms,
four attempts), made the post-link temporary removal genuinely
best-effort, cleaned up the staged temporary on link failure, and
flagged the `AlreadyExists` race branch as defensive and deliberately
untested (nothing else creates files in the directory under the
serialized-runs assumption).

Addendum (2026-11-18, T-019/D-043): the `StorageError::Migration`
variant this entry introduced is now a struct variant
`{ source, backup: Option<PathBuf> }` — a failing step attaches the
copy written immediately before it (the state the rollback leaves
the database at, UPD-004's pointer); the semantics recorded above
are unchanged, only the error grew a field. A failed copy write
keeps this entry's `StorageError::Backup` shape.

### D-039 — Settings repository: typed §6.5 keys, one JSON validation path, corrupt cells error instead of defaulting — 2026-11-18

Problem: T-028 turns the §6.5 `settings` table into a repository. Open
questions: how to type keys and values without primitive obsession and
without speculative layers (the IPC commands are T-030, not this task);
where the ranges live; what a read does when a stored cell is corrupt;
and whether `serde_json` joins the crate (a dependency decision, D-012
policy).

Solution: `eunomia_store::settings`. Four decisions inside it.

(1) Typing. A runtime `SettingKey` enum — the thirteen §6.5 keys, `ALL`
in table order, string literals both ways (`as_str`/`from_key`) —
because a runtime set is required anyway by get-all and by the future
string-keyed IPC, so a type-level key→value mapping (ZST keys plus a
trait) would duplicate the canon. Values are a `SettingValue` enum
whose variants carry domain enums (`Scheme`, `BaseName`, `Position`,
`ExtraFile`) instead of raw strings. Two validation gates: `set` first
rejects a variant that does not belong to the key (a kind check before
serialization, so a Rust-side caller mistake like pairing `Scheme`
with `ui.language` cannot be silently coerced through the JSON form),
and `SettingValue::from_json(key, &Value)` is the single place that
knows the §6.5 types and ranges: integers with
inclusive bounds 1–200 (`backup.retention`, spec D-17), 1–20
(`scan.max_depth`, D-04), 10–1000, 1000–100000, 7–3650; enum members
case-sensitive, matching the schema CHECK vocabularies of §6.2; the
extra files one JSON object with exactly `enabled` (boolean) and
`position` (`start`/`end`); `scan.exclude_names` an array of strings
(§6.5 pins no content rules — an empty list is valid). Integers that
fit u64 but not i64 are range-rejected, not type-rejected
(`as_i128`; anything larger is already a float inside serde_json and
therefore a type error). `set` validates by re-parsing the value's
canonical JSON through the same function, so write validation ≡ read
validation ≡ future IPC validation — a stored value can never fail its
own read unless the table changed behind the app's back.

(2) Corrupt-cell policy: a stored value that is not JSON, or valid
JSON outside the key's contract, fails the read with the new
`StorageError::Settings { key, source: SettingsError }` (variants
Type/Range/Member/Shape/Json — an explicit error, never a silent
default). The settings table is machine-written (only `set` writes
it), so a bad cell means corruption or version skew and must surface;
this deliberately differs from `config.toml` (SYS-012/D-028), where a
hand-edited broken file degrades to defaults with a warning. Rows with
keys outside the §6.5 set are ignored by `get_all` (forward
compatibility: a newer app version may add keys; this repository
serves the set it knows).

(3) `default.*` semantics (DEF-005/PRJ-012): these are defaults _for
new projects_. The repository stores and returns them like any key and
never propagates them — pinned by a test that changes all four
`default.*` settings in a database holding a project row and asserts
the project's scheme/base_name/extra-file columns untouched; copying
defaults into a newly created project belongs to the future
project-creation task.

(4) `serde_json = "1"` joins as the crate's fifth dependency, for the
JSON `value` column (§6.5). It is already resolved workspace-wide
(1.0.154, via src-tauri/tauri), so no new crate version enters the
graph — the same lockfile-reuse pattern as `toml` (D-029) and `uuid`
(D-031); `cargo deny check` stays green with no new ignores. No
`serde` derive dependency: values are parsed from `serde_json::Value`
manually, keeping the surface at one crate.

Layering unchanged (D-017): the store knows no `AppError`; mapping
`StorageError::Settings` to `E_VALIDATION` is the future command
layer's job (T-030). Testing split: in-module unit tests pin the §6.5
canon (key literals both directions, defaults, ranges with both bounds
and one-past-each-side rejections, non-integer type errors, enum
member case sensitivity, the extra-file shape matrix, canonical JSON
forms round-tripping) — with expected defaults, bounds and the
seven-name exclusion list hardcoded as spec literals, so the tests are
oracles independent of the production constants (a mutated constant
fails them; round-1 review finding). `tests/settings.rs` (tempdir +
`open_db` + `run_migrations`, the production configuration) pins
repository behavior: defaults on a fresh empty database (zero rows,
and reads write nothing), set→get round-trips with the stored JSON
text and a DB-002 `updated_at` pinned per key, `updated_at` moving on
rewrite, out-of-range, type-mismatched and wrong-variant writes
rejected with the stored state untouched (including no row created
where none existed), corrupt stored cells failing `get` and `get_all`
explicitly, unknown rows ignored, and the DEF-005 pin.

### D-040 — E_DB_TOO_NEW: the too-new gate runs before the DB-001 pragma set; typed at the seam, screen deferred — 2026-11-18

Problem: DB-004/NFR-010/T-F11 (the "DB newer than the app" half)
require that a database with `PRAGMA user_version` above what the app
knows is not opened: `E_DB_TOO_NEW`, §10 effect "The app does not
change the database", user action "Update the app". The store half was
done (D-038: `run_migrations` rejects with `DatabaseTooFarAhead`), but
at the app layer `Db::open` ran `open_db` + `run_migrations` and every
failure — too-new included — aborted the launch as an anonymous
`DbInitError` (D-037's recorded gap). Two subtleties drove the design:
`open_db` applies the DB-001 pragma set _before_ `run_migrations`
could reject anything, and the `journal_mode=WAL` switch **writes to
the file** (it rewrites the header of a rollback-journal database —
bytes 18/19 — and creates `-wal`/`-shm` siblings), so a gate that runs
after `open_db` would already have modified the very file §10 says the
app must not change; and the byte-identity criterion has to hold for a
database created _outside_ the app (T-F11 creates one by hand), which
arrives in the default DELETE journal mode.

Solution: the gate is the first step of `Db::open`, before `open_db`.
It opens a bare `rusqlite::Connection` (no pragmas — reading one
pragma writes nothing, no journal is created; src-tauri owning
`Connection` directly is D-037's shape) and reads `PRAGMA
user_version`, comparing it with the store's new
`latest_schema_version()` — the one, additive store change this task
makes in T-028's zone (three lines plus an invariant test): the number
must come from the store or the app would own schema knowledge twice,
and hardcoding it in src-tauri would drift at the first migration.
Every failure of the pre-check read is deliberately treated as
"version unknown — proceed": a missing file (a bare open creates the
same 0-byte file `open_db` would), a missing parent, a corrupt file or
a lock all have their canonical failure one step later (D-030's
`StorageError::Open` names the path; a corrupt file keeps its D-037
abort — it must not masquerade as too-new). Only a definitive
`version > latest` produces the rejection.

The rejection is typed at the seam: `DbInitError` became an enum with
a `TooNew { path, database_schema_version, app_schema_version }`
variant (everything else keeps the old `Open` shape and Display
unchanged) and an `app_error()` method that builds the §7.10 wire form
of `E_DB_TOO_NEW` — default message key `errors.db_too_new`, params
`path`/`database_schema_version`/`app_schema_version` (strings, per
D-034's string-typed params), `details: null` (a verdict, not an OS
failure — the facts travel in params). `run_migrations` failures are
classified by `classify_migration_failure`, which re-reads
`user_version` on the open connection and maps the too-new **fact** to
`TooNew` — that is the app-layer mapping of D-038's
`DatabaseTooFarAhead`, expressed without a src-tauri dependency on
`rusqlite_migration` (the store stays its only consumer); after the
gate passed, that branch can only fire when the file changed under the
open (swapped/rewritten between the two steps — accepted TOCTOU: the
single-instance lock leaves only external actors, and `open_db` may
already have touched a swapped-in file before the backstop rejects).

Startup behavior: the abort policy is UNCHANGED (D-027/D-028 — without
the database there is nothing to run); what changed is honesty of the
failure. The `setup` hook calls `app_error()` and, for the too-new
rejection, logs it through the facade before returning the error:
`logging::db_too_new(&AppError)` emits one `error`-level event
(`code="E_DB_TOO_NEW"`, the params as its payload, target
`eunomia::db`) — error level because the launch aborts on the verdict
and the line must survive `log_level = "error"` (same reasoning as
D-032's degradation event); logging is up by then, unlike the
layout/config steps (SYS-011). The §10 "Update the app" screen is the
future recovery/update task's to build — the typed error, params and
log line are its contract; until it exists the panic's `Display` text
(names the path, both versions and the code literal) is the console
diagnostic. D-037's "corrupt/too-new abort" phrasing is thereby
narrowed: too-new is typed and logged, corrupt keeps the plain abort
until the recovery task.

Addendum (2026-11-18, T-020/D-045): the corrupt half of that narrowing
is itself narrowed — a damaged database is now rejected by the typed
`quick_check` gate as the second step of `Db::open`, still before
`open_db` (D-045); "until the recovery task" now means until the
recovery _screen_ task (T-024), not the corrupt detection.

Byte-level facts, verified empirically and pinned by tests: a
rollback-journal (hand-made) database gains nothing from the gate's
read — byte-identical, no sidecars (this is exactly why the gate must
precede `open_db`; the mutation "gate after open_db" fails the
byte-identity test on the WAL header rewrite). For a database already
in WAL mode (the shape a real newer app leaves), SQLite creates the
`-shm` and possibly `-wal` siblings for the bare read and removes them
on the clean drop — main file byte-identical, no debris (asserted;
the tests also pin that `db/` holds exactly the one file and
`db-backups/` stays empty). The gate connection carries the DB-001
`busy_timeout` (the same 5000 ms `open_db` applies; the constant lives
in the store and is duplicated here for that one call — a momentary
writer lock must yield a verdict, not defer the decision past the
pragma set). Accepted boundaries, both SQLite's own crash recovery
running during the read — bytes move, logical content does not, and
learning the version without opening the file would mean
re-implementing SQLite's WAL/journal formats: (a) a WAL file with
_pending frames_ (a newer app killed before checkpointing) is
checkpointed by the clean drop; (b) a _hot rollback journal_ (a newer
app killed mid-write in DELETE mode) is rolled back, consuming the
`-journal`. Deliberately untested: simulating either requires
leaking an open connection (`std::mem::forget`) to keep frames/journal
pending, and a leaked handle breaks the Windows test teardown rule the
store suite pinned (connections closed before `remove_dir_all`) — the
facts are SQLite's documented close-time recovery, and the settled
states on both sides are asserted. One more recorded degradation: a
version that cannot be read even after the busy timeout (a lock that
outlasts it, a transient I/O error) yields "unknown → proceed", and
what happens next depends on the lock: if it clears within
`open_db`'s own 5 s timeout, the pragmas are applied (on a
rollback-mode file the header rewrite already happened) and the
rejection arrives from the backstop; if it persists, `open_db` itself
fails and the launch aborts with the generic `Open` error — correct,
just not the typed verdict. With the timeout, either branch needs an
external writer holding the file across a startup in progress.

Testing: the T-F11 new-DB half through the real `Db::open` seam —
hand-made DELETE-mode file at latest+1 and latest+5 (wire form pinned:
code/message_key/all three params/`details: null`; file bytes asserted
equal; `db/` holds exactly the one file, `db-backups/` stays empty;
the abort `Display` names the path, version and code literal), the
WAL-mode production-shaped variant (made via the store's own
`open_db`+`run_migrations`, version bumped; bytes equal, no sidecar
debris, nothing else written), the boundary case — an up-to-date
database (version == latest, every existing user's normal state)
opens normally, which a `>=` mutation of the gate's comparison would
reject — a garbage file keeping the generic abort (`app_error()` None,
path in `Display`, bytes untouched), the backstop classifier in both
directions (too-new fact → `TooNew`, in-range → `Open` with the store
error preserved), the facade line (code literal, path, versions, §10
action; survives `log_level = "error"`), a lib.rs wiring pin (facade
log before the abort return), and the store invariant
(`latest_schema_version()` == `user_version` after a full run).
Mutations verified the suite bites: removing the gate fails the two
DELETE-mode tests (and only them — the WAL variant legitimately passes
through the backstop, which is that path's own proof); moving the gate
after `open_db` fails the byte-identity assertion exactly.

Addendum (2026-11-18, T-019/D-043): "`None` for every other init
failure" and "everything else keeps the old `Open` shape" are narrowed
by D-043 — a migration step that failed and was rolled back now has
its own typed `DbInitError::Migration` (UPD-004) whose `app_error()`
is the `E_UPDATE_FAILED` wire form when the run wrote a copy; the
abort policy this entry records is unchanged, and failures without a
copy still return `None`.

### D-041 — Check chains verify exit codes explicitly; output pipes must not swallow them — 2026-11-18

Problem: the orchestrator's post-merge check chain ran
`cargo clippy … 2>&1 | tail -1 && …`. In a pipeline the chain's status
is the LAST command's, so `tail`'s 0 masked clippy's failure: after
merging T-028 a combination-only `result_large_err` (T-023's
`DbInitError` wrapping the `StorageError` variant T-028 added — each
task green on its own base) failed clippy, yet the chain printed a
false "ALL GREEN" and continued. Any `cmd | tail`/`cmd | grep` shape
in a `&&` chain has the same hole; grep -c output ("13") can likewise
read as success while tests fail.

Solution: check chains gate on the real exit code. Either run the
command bare in the chain, or capture it immediately:

```bash
cmd > /tmp/opencode/<slug>.log 2>&1; code=$?; echo "exit=$code"
```

and branch on `$code` (or check `PIPESTATUS[0]` when a pipe is
unavoidable). Line-count summaries like `grep -c` are informational
only, never the gate. This applies to the orchestrator's merge
pipeline (worktree pre-merge checks and post-merge main-tree checks)
and to executor-subagent briefs, which now carry the same rule. The
failure that exposed it was fixed by a service commit (boxing
`StorageError` in `DbInitError`); the false-green itself is what this
entry prevents from recurring.

### D-042 — Domain types and the ALG-001 build snapshot in eunomia-core — 2026-11-18

Problem: T-033 turns the spec vocabulary (§2) and the snapshot contract
(ALG-001) into the first real modules of `eunomia-core`, and left open:
the snapshot's exact composition (flat DB-shaped collections vs a
resolved per-project tree), where ordering lives, how identifiers are
typed without the `uuid` crate, whether the snapshot validates its own
graph, which fields are deliberately absent, and how the crate's serde
allow-list entry (D-017) is put to use without inventing wire formats.

Solution: four flat modules — `project` (ProjectId; Scheme A/B/C;
BaseName with `stem()` for scheme C; ExtraFilePosition; ExtraFile;
ProjectWriteSettings), `library` (ItemId, VariantId; HeadingLevel as a
closed H2–H6 enum with `TryFrom<u8>` for the integer column;
InstructionState active/hidden; Variant; Heading; Instruction with its
variants; LibraryItem), `assignment` (Assignment), `snapshot`
(SnapshotProject, SnapshotError, BuildSnapshot).

Composition follows ALG-001's own decomposition rather than a
per-project resolved tree: active projects in `sort_order` order, ALL
library items in `position` order (hidden instructions and headings
included — ALG-020 consults `state`, and D-11 keeps hidden
assignments), the ACTIVE variants nested under their instruction
(matching §2's hierarchical "Instruction — one or more variants"
definition; the DB's flat `instruction_variants` table folds into it at
the store boundary), and the assignments of the active projects as a
flat list (DB-014: archived projects keep assignments, so the assembly
must scope them — the flat `Assignment` keeps `project_id` and remains
usable outside the snapshot). Ordering is carried by collection order
only: DB-005/DB-011 make positions a gapless 1..=N row, so the vector
preserves the position order (`index + 1` is the position) and no
numeric field is stored to drift out of sync.
Per-project resolution (instruction→variant maps) is the builder's
(T-035) internal step, not snapshot state.

`BuildSnapshot::new(projects, items, assignments)` validates the whole
graph once and keeps the fields private behind slice accessors, so a
constructed snapshot is immutable and every assignment provably
resolves: unknown project, unknown item, assignment to a heading,
variant not among the instruction's active variants (DB-012 shape),
duplicate (project, instruction) pair (D-02), duplicate ids, and —
mirroring DB-010's wording — an active instruction with no variants
(hidden instructions are not held to it). The schema enforces most of
these facts for a healthy database; the checks exist because the
snapshot is assembled by hand-written queries, and a wrong filter or
join there (e.g. an archived project's assignment reaching the
snapshot, or an assignment pointing at a variant the active-variant
query filtered out of its instruction — the DB-012 shape) must fail at
sync start with a named `SnapshotError` (thiserror) instead of
surfacing as a wrong write. What the checks cannot re-cover is the
active-variant filter itself: `Variant` deliberately carries no state,
so whether soft-deleted variants were excluded is the store query's own
guarantee. This
is what lets the builder be a total function with no lookup-miss
branches.

Deliberately absent: `is_main` on Variant (nothing in the build reads
it; DB-010's exactly-one-main invariant is store-checked — carrying it
would invite representing a state core cannot enforce), any `Default`
impl (the §6.5 default values have one owner, the settings repository,
D-039), UI-only project attributes (label, color), and housekeeping
columns (timestamps, last-synced hash — the snapshot is a build input,
not a project listing; `root`, `name` and `settings` stay because the
DB-020 hash, sync summaries and the D-25 confirmation list consume
them). `PathBuf` in SnapshotProject is a value, not fs access.

Serde: the vocabulary types derive Serialize/Deserialize with the
canonical literals pinned (`"A"`/`"AGENTS.md"`/`"start"`/`"end"`/
`"active"`/`"hidden"`, heading level as the schema integer 2–6 under
its §6.2 column key `heading_level`, the
`{"enabled":…,"position":…}` §6.5 JSON shape with unknown members
denied (`deny_unknown_fields`), the `"instruction"`/
`"heading"` kind word, identifiers as plain strings) — unit-tested in
both directions, case-sensitive rejections included. The aggregates
(BuildSnapshot, SnapshotProject) carry NO serde derives: a Deserialize
impl would bypass `new()`'s validation, and both types have no wire
consumer. Identifier newtypes are opaque (`String` inside, no UUIDv7
validation): generation and validation are the store's (D-031), and
`uuid` is outside the crate's allow-list (D-017) — the newtypes' value
is that project/item/variant ids cannot be mixed at call sites.
A dev-only `serde_json = "1"` (1.0.154, already resolved workspace-wide)
pins the literals in tests; the runtime allow-list stays
serde/sha2/thiserror, with serde and thiserror now actually used and
sha2 still idle until the build-hash task.

Recorded duplication: `eunomia_store::settings` already carries its own
Scheme/BaseName/Position/ExtraFile enums (D-039, written before core
had a home for them). Core's types are now the canonical vocabulary;
swapping the store's settings (and adding the store→core edge per
D-017's first-consumer rule) belongs to the next task that touches the
store's settings module, not to T-033. First consumers of the snapshot:
T-034 (ALG-002 normalization over the carried variant texts) and T-035
(the builder); variant texts ride the snapshot raw — normalization is a
build-pipeline step, not a snapshot mutation.

### D-043 — UPD-004: a failed migration is typed with its rollback copy; the wire carrier is E_UPDATE_FAILED (a recorded registry gap) — 2026-11-18

Problem: T-019 turns UPD-004 ("If the new version could not run a
migration, the app does not change the database and says that the
previous version must be installed, pointing to the
`db-backups/pre-migration-*` copy") into behavior. The rollback half
was already D-038's (per-step transactions, `to_version` stepping,
`a_failing_step_rolls_back_only_itself`), but the failure carried no
copy path: `StorageError::Migration(rusqlite_migration::Error)` had
nowhere for one, and at the app layer every `run_migrations` failure
collapsed into the anonymous `DbInitError::Open`. Open questions:
which copy the error points at; how the path travels store →
src-tauri without giving src-tauri a `rusqlite_migration` edge
(D-040); and which of the twenty §10 codes (registry fixed by
T-013/D-034) carries the situation — §10 has no migration-failure
row at all.

Solution: three pieces.

(1) Store: `StorageError::Migration` became a struct variant
`{ source, backup: Option<PathBuf> }`. The stepping loop attaches
`backups.last()` when a step fails — the copy written immediately
before the failed step, i.e. of the exact state the rollback leaves
the database at (the failed step's source version; for a v1→v3 run
failing at step 3 that is `pre-migration-v2-*`, not the run's first
copy). `backup` is `None` when no copy exists: a fresh bootstrap
(start version 0 — no user state), the version-read failures (the
`From` conversion, which serves `current_version`/`pending_migrations`)
and the too-new rejection (no step ran). A failed copy _write_ keeps
being `StorageError::Backup` — D-038's own shape; it is not the
UPD-004 situation (no migration ran), and it already carries the
attempted path and cause, so it does not degrade into silence or
into the new variant (pinned by a unix-gated read-only-directory
test, skipped as root like the config suite's).

(2) src-tauri: `classify_migration_failure` maps a
`StorageError::Migration` (with the too-new _fact_ still winning
first — the D-040 backstop) to the new `DbInitError::Migration
{ path, source, backup }`. `Display` (the text the startup panic
prints) states, in the copy-carrying arm, the rollback and the
UPD-004 instruction "install the previous app version" plus the copy
path (printed once: the arm renders the underlying migration
failure, `source.source()`, because the store error's own `Display`
already carries the path); both arms claim only the one fact true
for every producer of the variant — "the database keeps the state of
its last committed migration" (for a multi-step run that may be an
earlier step of the same run; for a run that created the database,
nothing) — and without a copy the text says exactly that ("no
pre-migration copy was written") instead of inventing a pointer,
claiming who created the file, or asserting a rollback for the
version-read failures where no step ever ran (round-2 review). `app_error()` returns the typed
wire form only when a copy exists: code `E_UPDATE_FAILED`, message
key `errors.update_migration_failed` (a deliberate member of the
update family, per D-034's per-instance keys), params `path` +
`backup_path`, `details: null` (a verdict, like `TooNew`; the store
reason stays in `Display`). A migration failure _without_ a copy
returns `None`: not the UPD-004 situation, the generic D-027 abort —
and by the same policy it gets no facade log line (the panic
`Display` is its report; its §10 line belongs to the recovery task
like every generic init failure's).

Why `E_UPDATE_FAILED`: §10 (fixed by T-013) has no
migration-failure code, and the plausible carriers lie about the
facts in their §10 rows — `E_DB_CORRUPT` (the database is not
damaged; the row's effect steers the user toward the destructive
"start with empty database" recovery, exactly the misclassification
D-037 recorded rejecting), `E_DB_TOO_NEW` (no version skew exists;
its action points the app version the wrong way) and `E_DB_LOCKED`
(not busy; retrying the same binary fails deterministically).
`E_UPDATE_FAILED` is the least-harmful carrier: UPD-004 is the
Updates section's own rollback rule, its §10 effect "Update not
installed; work continues" matches the remedy (stay on/return to the
previous version), and its action "Retry later" is the complementary
one (re-update once a fixed version ships). The UI localizes by
`message_key` (NFR-006), so the user sees the UPD-004 text, not the
§10 row's prose. This is a recorded registry gap in D-037's shape,
to revisit when §10 grows a migration-specific code. The `setup`
hook logs the typed form through the new `logging::db_migration_failed`
facade event (error level — the launch aborts on the verdict, the
same reasoning as `db_too_new`) before the abort; the abort policy
itself is unchanged (D-027/D-028).

(3) Proof, matching the card's criterion ("a test with an
intentionally broken migration: schema and data unchanged, the error
contains the copy path"). Store, on the private `migrate` seam with
real SQL and file databases (the D-038 boundary: the embedded set
has one working migration, so a broken step exists only there): the
broken single pending step on an existing database — it deliberately
does real work first (`CREATE TABLE probe …; INSERT …; THIS IS NOT
SQL`), so only the rollback can explain the state afterwards, not an
inert statement — leaves `sqlite_master` (type/name/rootpage/DDL,
auto-index NULLs coalesced) and the table data and `user_version`
equal to the pre-run snapshot, and the error's copy exists, passes
`integrity_check` and holds that state; a multi-step run's error
names the _failed step's_ copy; a fresh bootstrap failure reports
`backup: None` with an empty backup directory. src-tauri, through
the real `Db::open` seam: a hand-made database whose `settings`
table collides with migration 1 (the only intentionally broken
migration reachable through the public API — a fresh-bootstrap
failure, `backup: None`) aborts with the honest `Display` and an
unchanged schema/data/version; the copy-carrying shape is proven on
`classify_migration_failure` with a real store error (src-tauri
cannot synthesize `rusqlite_migration::Error` without a dependency
edge, so the fixture drives the store on the colliding database and
re-shapes the `backup` field — that pins the path plumbing and the
wire form; the copy file's own validity is the store suite's proof,
unreachable through `Db::open` until a second embedded migration
exists) and pins the wire form (code, key, both params,
`details: null`) plus the `Display` contents; a backup failure keeps
the generic `Open` abort; facade and `lib.rs` wiring pins mirror the
too-new ones. Mutations verified the suite bites: dropping the copy
from the store error fails both store acceptance tests; dropping the
copy/instruction from `Display` and the key override from
`app_error()` each fail their src-tauri test. Round-1 review
(deepseek-v4.1-flash) forced the partial-work broken steps, the
universally-true `Display` wording, the single copy-path printing
and these recorded policies; round-2 review (hy4-flash-free) removed
the last false claim (the None arm no longer says "was rolled back"
— version-read failures never opened a transaction) and restored the
`ARCHITECTURE.md` step-order sentence.

No new dependencies. Boundaries unchanged: the store knows no
`AppError` (D-017), src-tauri still depends on `rusqlite_migration`
not at all (D-040).

### D-044 — ALG-002 normalization: ASCII whitespace class, blank-line edges, BOM run, fixpoint idempotency — 2026-11-18

Problem: T-034 implements ALG-002 ("CRLF and a single CR are replaced
with LF, the BOM is removed, and leading empty lines and all trailing
whitespace of the block are dropped. Spaces inside the text are not
touched"), and the spec fixes the rules but not their boundaries: which
characters count as "whitespace" (ASCII vs Unicode White_Space),
whether a "leading empty line" includes whitespace-only lines, whether
"the BOM" is one character or a run, at which position a BOM counts as
removed (a BOM after a leading empty line is not at the text start),
and where the function should live relative to `Variant` and the
snapshot.

Solution: `eunomia_core::normalize::variant_text(&str) -> String` in a
new `normalize` module — a pure function over the text, not a method on
`Variant` and not a snapshot mutation (D-042: texts ride the snapshot
raw; the build calls this per variant, and ALG-002 names it an algorithm
of §9.1 in its own right). Four boundary decisions:

1. Whitespace class = ASCII whitespace per Rust
   `char::is_ascii_whitespace` (WhatWG: space, `\t`, `\n`, `\f`,
   `\r`) — one class for both edge rules. These are the characters
   that shape lines and blank lines in a UTF-8/LF file (D-22). Unicode
   whitespace (NBSP `U+00A0`, the `U+2000`…`U+200A` block, `U+3000`,
   the Unicode line/paragraph separators) and the vertical tab
   `U+000B` are Markdown text, not line structure — for the leading
   rule CommonMark's own blank line is space/tab only, and ALG-002's
   protection sentence itself anchors on Markdown — so they survive
   even at the block edges. (The Unicode separators are also not line
   endings here: ALG-002 converts CR/CRLF only, so they stay ordinary
   characters.) The chosen blank-line class is deliberately one notch
   broader than CommonMark's space/tab — a form-feed-only line counts
   as empty here — because one ASCII class serves both edge rules.
   Accepted consequence: a variant whose last line ends in
   NBSP builds a line with invisible trailing "whitespace" — content
   fidelity beats cosmetics there; ALG-002 does not fix it.
2. "Leading empty lines" = leading lines that are empty or hold only
   characters of that ASCII class — the Markdown blank line. The
   strict reading (zero-length lines only) would leave whitespace-only
   lines at the join points where ALG-022 separates blocks by "exactly
   one empty line", and the spec's own trailing rule ("all trailing
   whitespace") shows the edges are meant whitespace-clean. Spaces
   after the first content character of a line are inside the text and
   preserved (the first line's indentation survives).
3. Trailing rule = trim the ASCII-whitespace suffix of the whole block
   (final line breaks, trailing empty lines, trailing spaces/tabs of
   the last content line). A two-space Markdown hard break at the very
   end of a block is therefore dropped — "all trailing whitespace" is
   unconditional; hard breaks on inner lines keep their spaces.
4. BOM = a leading RUN of `U+FEFF`, and the rule is positional:
   leading means the very start of the block or immediately behind its
   leading empty lines (`"\n\u{feff}text"` → `"text"` in one call; the
   same holds after a lone CR); a `U+FEFF` anywhere else — after the
   first line's own characters, or inside the text — is content and
   preserved. The leading edge is consumed by one linear scan that
   repeats {strip the BOM run, drop empty lines} until neither
   applies, because the two rules interleave (an empty line can hide a
   BOM). The first draft re-ran the whole pass to a fixed point
   instead; the round-1 review measured that formulation at 13.6 s on
   a crafted 131k-character `"\u{feff}\n"`-chain — quadratic, and
   SEC-009 treats imports as untrusted — so the scan was rewritten as
   the linear equivalent (the result of either is the same contiguous
   middle slice of the CR-converted text: trailing edge =
   `trim_end`, leading edge = the scan's stopping point). The output
   is already normalized, so the function is idempotent by
   construction.

Also recorded: §6.9 already folds CRLF→LF at save time, but a lone CR,
a BOM and edge whitespace can still arrive (import per SEC-009, older
databases, direct SQL), and D-22 keeps a scheme-B file's own BOM and
line breaks only OUTSIDE the managed block — so the build-time pass is
unconditional and this module is the rule's single owner. No new
dependencies (the D-017 allow-list is untouched — a pure function over
`&str`). Tests: 27 unit tests + a doctest pin every ALG-002 rule, the
inner-whitespace invariants (indentation, inner empty lines, inner
trailing spaces/tabs), the class boundaries (Unicode whitespace and
`\v` as content, a form-feed-only line blank at the leading edge), the
BOM position rules (behind an empty line: removed; after the first
line's indentation: kept), degenerate inputs (empty, BOM-only,
CR-only, whitespace-only), idempotency (including the
BOM-after-empty-line and double-BOM cases) and a linear-time guard
over the crafted 65,536×`"\u{feff}\n"` chain; five behavior mutations
verified the suite bites (BOM strip removed → 5 unit failures,
trailing trim removed → 5, leading-edge drop removed → 9, BOM-run
strip alone removed → 5, CRLF-pair handling removed → 3 — unit-test
counts throughout; the doctest fails on top of the first three).

### D-045 — E_DB_CORRUPT: the quick_check gate runs second, before the pragma set; garbage and malformed files are both corrupt — 2026-11-18

Problem: T-020 turns spec §6.3/§10/NFR-007…010/T-F11's damaged-DB half
into behavior: at startup the app must run `PRAGMA quick_check` and, on
failure, surface `E_DB_CORRUPT` and head for recovery mode. Open
questions: where in the `Db::open` chain the check runs (T-018's
too-new gate is fixed first by DB-004, D-030's pragma set _writes_, and
D-038's migrations write more); what happens to files quick_check
cannot meaningfully check (garbage, empty, missing, locked); what
"recovery mode" means before the recovery screen task (T-024) exists;
and what the quick_check evidence is on the wire, given D-034 reserved
`params` for user-visible interpolation and `details` for OS facts.

Solution: `reject_corrupt(db_path)` in `src-tauri/src/db.rs` is the
**second** step of `Db::open` — after `reject_too_new`, before
`open_db`. Both directions of that placement are load-bearing. After
the too-new gate: DB-004 fixes that gate as the first step (D-040),
its verdict stays decisive on a file that is both too-new and damaged
(the version is a header fact answered cheaper than a full-file scan,
and a too-new file never pays the scan — it is presumably healthy).
Before `open_db`: the DB-001 pragma set writes to the file (the WAL
switch rewrites the header of a rollback-journal database), and §10's
`E_DB_CORRUPT` effect starts recovery over a database the app has not
touched — T-F11's "data not changed", asserted on the bytes (the
mutation "gate after `open_db`" fails the byte-identity test exactly;
removing the gate fails the three detection tests). The check runs on
a bare connection (the D-037/D-040 gate shape) with the gate's own
5000 ms `busy_timeout`; `quick_check` is read-only, so the D-040
sidecar facts and accepted boundaries apply verbatim: no `-wal`/`-shm`
debris on a clean drop, and SQLite's own crash recovery during the
read (a pending WAL checkpointed, a hot journal rolled back) may move
bytes while preserving content — learning corruption without opening
the file would mean re-implementing SQLite's formats.

Skip and degrade conditions, so the gate costs nothing where there is
nothing to verify and claims nothing it cannot prove: a **missing**
file and a **0-byte** file (a valid empty SQLite database; the gate
connection the too-new gate opened may have just created the 0-byte
one) are not checked at all. `quick_check` reads every page of the
file, so the skip is also what keeps that cost inside spec §12.2's
startup budget ("startup until the window is ready", 2 s for set L):
existing databases pay one full read of the file at every startup —
mandated by §6.3's own "checked at startup" wording and well inside
the budget at the §12.2 data-set scale on the reference machine, with
the release benchmark of §12.2 as the enforcing measurement — and
fresh launches pay nothing. A read that
fails for a **non-corruption** reason (busy/locked beyond the timeout,
an I/O error, an unopenable path) is "cannot verify", not "verified
corrupt": the gate proceeds and the canonical failure surfaces in
`open_db`/`run_migrations` — the too-new gate's "unknown → proceed"
policy (D-040) mirrored here, because a false corrupt verdict would
steer the user to the destructive empty-database recovery on what may
be a transient lock (the misclassification D-037 recorded rejecting).

Detection shapes, verified empirically against the bundled build
(the sqlite3 3.46 CLI plus a first-row C probe; each shape pinned by a
test): a **garbage** file fails the pragma query at prepare with
`SQLITE_NOTADB` ("file is not a database") — it is typed `Corrupt`
too, the same verdict D-037's statement classifier already gives
notadb (`E_DB_CORRUPT`); subtler damage makes the query **return
finding rows** instead of erroring — a shrunk in-header page count
yields "Tree 2 page 2 … invalid page number 3", and a smashed b-tree
page a row that itself carries the b-tree init error ("*** in
database main *** / Tree 2 page 2: btreeInitPage() returns error code
11"), both with the `***` prefix and an embedded newline, carried
verbatim (the CLI's bare "database disk image is malformed" arrives
only on rows after the first, which the first-row read never reaches;
a `SQLITE_CORRUPT` prepare/step failure is nevertheless classified the
same way — the defensive arm for build-dependent shapes). Only
`NotADatabase` and `DatabaseCorrupt` failures and non-`ok` first rows
reject; the shapes are pinned apart by their detail wording ("not a
database" / "btreeInitPage" vs the page-count finding), so neither
classification arm can drift into the other silently. One
honest limitation recorded: quick_check skips index-correlation
checks (a flipped byte in an index page passed it in verification) —
that is SQLite's trade-off for the speed §6.3 names, not a gate
defect; `integrity_check` is the diagnostics screen's on-demand tool,
not a startup requirement.

The typed error: `DbInitError::Corrupt { path, detail }` — `detail` is
SQLite's own evidence (finding row or error text), never prose this
layer composed. `app_error()` is `E_DB_CORRUPT` with the default key
(`errors.db_corrupt`, no per-instance override needed) and params
`path` + `detail`, `details: null` (a verdict like TooNew; the
quick_check evidence is an interpolation value for the recovery
screen, not an OS fact — D-034's ErrorDetails stays OS-shaped; the
alternative of growing a quick_check field into ErrorDetails was
rejected as a wire-contract change the screen task does not need yet).
`Display` (the startup panic text) names the path, the evidence, the
§10 effect and the code literal. Startup behavior: the abort policy is
UNCHANGED (D-027/D-028); "recovery mode" today is honestly the typed
error + the `logging::db_corrupt` facade line (error level, §10 code +
params, wired in `setup` before the abort) — the §10 screen (data
directory path, `db-backups/` list, "Open data folder"/"Start with
empty database" buttons, the `db_backups_list`/`db_restore_backup`/
`db_recreate` commands) is T-024's, blocked on exactly this contract;
the §6.3 "invariant check" (DB-010…) is not this gate either — those
invariants need the service layer and belong to the diagnostics task.
This narrows D-037/D-040's "a corrupt file keeps the plain abort until
the recovery task lands" to "until the recovery _screen_ task lands".

Testing, all through the real `Db::open` seam: three detection shapes
(finding-rows fixture via a shrunk in-header page count — header and
`user_version` stay readable so the too-new gate demonstrably passes
first; malformed-page fixture via a smashed b-tree root page; the
garbage fixture), each asserting the `E_DB_CORRUPT` wire form (code,
default key, path + non-empty detail params, `details: null`), the
byte-identical file, no sidecars, nothing else written and the
`Display` contents, with the malformed and finding-row details pinned
to their shape's wording; the same rejection on a **WAL-mode** damaged
database (the production shape: store-made, settled, then damaged) —
main file byte-identical and no `-wal`/`-shm` debris, the D-040
sidecar facts exercised on the rejection path (a corrupt file with
_pending_ WAL frames stays deliberately untested for D-040's own
reason: simulating one requires leaking an open connection, which
breaks the Windows teardown rule; the checkpoint-on-drop is SQLite's
documented close-time recovery); the empty-file skip (opens as a
fresh bootstrap); the gate order (too-new wins on a file that is
both); the honesty guard (an unopenable path — a directory where the
database belongs — keeps the generic `Open` abort, no typed verdict:
the same "cannot verify → proceed" branch a busy/locked file takes,
untestable there without a multi-second stall); the facade line (code
literal, path, evidence, §10 action; survives `log_level = "error"`);
and a `lib.rs` wiring pin (facade log before the abort return).
Mutations verified the suite bites: removing the gate fails exactly
the three detection tests; moving it after `open_db` fails the
byte-identity assertions exactly.

No new dependencies. Boundaries unchanged: the store knows no
`AppError` (D-017) and no quick_check gate — the check is a startup
gate of the app shell, like the too-new gate before it (D-040);
src-tauri still depends on `rusqlite_migration` not at all.

### D-046 — ERR-001 daily backups: same-day by name, last 7 kept, degrade on failure; the date is an injected `Timestamp` — 2026-11-18

Problem: ERR-001 (spec §10) requires a `VACUUM INTO` copy
`db-backups/daily-YYYYMMDD.sqlite3` at every startup when none exists
for today yet, keeping the last 7. Open questions: how to reuse the
D-038 pre-migration mechanics without duplicating them; how "today"
is injected (system time is not mockable); whether the cap touches
`pre-migration-*` files; where the call sits (store vs `Db::open` vs
the `setup` hook); whether a fresh first launch copies its empty
database; and what a failed daily copy does to the launch (a failed
pre-migration copy aborts, D-038).

Solution: `eunomia_store::storage::daily_backup(conn, backup_dir,
now)`, next to the pre-migration code it shares mechanics with. The
D-038 staging/publication path was extracted into one helper,
`stage_and_link_copy(conn, final_path)` (`.tmp` staging, `hard_link`
no-clobber publication, §9.5 retries, best-effort temporary removal),
which both schemes call — `vacuum_into_backup` keeps only its
attempt-loop name search; the debris sweep was parameterized by a
per-scheme predicate (`sweep_stale_temporaries(dir, is_own_debris)`):
the pre-migration scheme matches `pre-migration-v*.sqlite3.tmp`, the
daily scheme requires the `.tmp`-stripped name to be a full
`daily-<8 digits>.sqlite3` (`is_daily_debris`), so each run cleans
its own scheme's debris and `daily-notes.sqlite3.tmp` stays
untouched. Seven decisions inside:

- _Date seam_: `now: jiff::Timestamp` is a parameter; only its UTC
  date (`strftime("%Y%m%d")`, the same UTC-rendering `utc_stamp`
  already relies on) names the copy, so every instant of one UTC day
  maps to one name and the day boundary is UTC midnight. The setup
  hook passes `Timestamp::now()`; tests pass fixed instants. src-tauri
  gains a direct `jiff = "1"` edge — no new crate version (already
  built via eunomia-store, D-035; the lockfile-reuse pattern of
  D-029's `toml` and D-037's `rusqlite`).
- _Same-day idempotency by name, without opening the copy_: the run
  checks `daily-<today>.sqlite3` exists → `AlreadyPresent(path)` and
  writes nothing. Doubly enforced: even with the check removed, the
  `hard_link` publication refuses the occupied name (`NameTaken` →
  `AlreadyPresent`), the same race-closing property D-038 built.
- _Cap semantics_: after a copy is _written_, daily names beyond the
  newest 7 are deleted, oldest first — name order is chronological
  because the date is fixed-width 8 digits — **but never the copy that
  run just wrote** (round-1 review finding): a clock-set-back
  directory may hold seven names newer than today, and deleting by age
  alone would delete today's fresh copy, leave the day without one and
  repeat a full `VACUUM INTO` on every restart. The pattern is strict
  (`daily-<exactly 8 ASCII digits>.sqlite3`): suffixed variants,
  other widths and other prefixes are neither counted nor deleted,
  and `pre-migration-*` files are outside the cap entirely (ERR-001's
  "keeps the last 7" is about the daily copies; pre-migration
  retention stays uncapped per D-038). A run that found its copy
  present does not prune (it changed nothing; the cap from the last
  write still holds) — pathological states (clock stepped back, files
  restored by hand) self-heal on the next write. Pruning is
  best-effort throughout, and deliberately so (round-1 review): it
  runs _after_ the copy is linked and complete, so a listing that
  cannot be read or a file that stays held after its §9.5 retries
  must not turn a successful copy into a reported failure — the next
  write prunes again.
- _Fresh first launch copies too_: ERR-001 says "at every startup",
  and a copy of the just-created empty database is cheap and an
  honest baseline for the day (restoring it equals the "start with
  an empty database" option). No special case exists for it.
- _Call site_: `Db::ensure_daily_backup(now)` — a method on the
  managed state, called by the `setup` hook after `Db::open` and
  before `app.manage`. `Db` gained a `backups_dir` field (derived
  once in `Db::open` through `data_dirs`, so the file and its
  backups cannot drift at a call site — D-037's reasoning); the
  method locks the connection like any operation. A separate method,
  not a step inside `Db::open`, keeps the open chain's
  error contract untouched (`DbInitError` gains nothing) and lets the
  degradation decision live at the orchestration layer, where
  logging already sits; the wiring is pinned by a `db_tests.rs` text
  pin (open → daily → manage, facade calls present).
- _Degrade, never abort_: the deliberate difference from D-038's
  pre-migration abort. The daily copy is extra reliability over a
  database that opened and migrated cleanly; failing the launch over
  a redundant copy would trade a working app for nothing. A failure
  keeps its own shape (`StorageError::Backup`), is logged by the
  setup hook through `logging::db_backup_daily_failed` (a `warn`, not
  `error` — the launch continues, and an `error` line would read as
  a failed launch in log triage), and every later command runs
  normally (pinned: an operation commits on the same connection
  after a failed copy). A written copy gets one
  `logging::db_backup_daily` `info` line; an existing same-day copy
  gets none (the idempotent no-op would repeat on every restart).
  §10's error-code registry is not involved — this is not a §10 user
  situation, and the D-037 gap stays as recorded.
- _action_log boundary_: spec §7.7 LOG-001..004 has backup creation
  recorded in `action_log`; that service is T-026's and does not
  exist yet, so the facade lines above are the whole journal of
  daily-copy creation until that task wires the same event into
  `action_log` (visibility per LOG-005 is its decision).

SEC-007 boundary, extended by this task (round-1 review): the D-030
addendum already records that the database file is created with the
OS default mode instead of the required `0600`; the copies in
`db-backups/` — pre-migration (D-038) and now up to 8 daily ones —
inherit the same gap, and `db-backups/` itself is `0755`-under-umask
rather than `0700`. Fixing it per-file here would leave the two
schemes inconsistent; it belongs to the dedicated SEC-007 task, which
now owns the directory mode and every copy's mode together.

Testing: the store suite covers the name/UTC-day mapping (fixed
instants at the 23:59:59.999Z/00:00:00.000Z boundary), the strict
pattern (unit test over near-miss names), the valid-copy acceptance
(`integrity_check = ok`, schema version and marker row preserved,
produced through `open_db`'s WAL configuration with the writer
connection held open, so the marker really lives in un-checkpointed
WAL frames when the copy is taken), the two card
criteria (same-day rerun writes no second copy and changes nothing —
bytes compared; the eighth copy deletes the oldest, with
`pre-migration-*` and foreign files surviving), the
prune-keeps-what-it-wrote contract (seven names newer than today),
the below-cap
boundary (7 copies, nothing deleted), the no-op-no-prune contract
plus its next-write self-heal, the per-scheme sweep (a daily run
removes only debris whose `.tmp`-stripped name is a full daily name —
`daily-notes.sqlite3.tmp` survives like the migration scheme's and a
user's `.tmp`), the missing directory (fails, creates nothing), and
the unwritable directory (unix, skipped as root: fails as
`StorageError::Backup`, no debris). `db_tests.rs` repeats the two
acceptance criteria and the degradation contract through the `Db`
seam over the §5.5 layout, plus the wiring pin (open → daily →
manage, the facade calls present, and no `?`/`return` between the
daily call and the manage step — the never-abort half pinned as
text); `logging.rs` tests pin both facade lines' payloads and levels.
Mutations verified the suite bites: disabling the prune fails the
eighth-copy and no-op tests; disabling the same-day existence check
fails the no-op test (the rerun test stays green through the
hard-link no-clobber — the double enforcement working as designed);
widening the name pattern fails the pattern unit test.
