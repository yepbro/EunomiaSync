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
