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
dedicated task before release. (Done by T-022/D-048: the mode is now
set at the open, before the pragma chain.)

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

Addendum (2026-11-18, T-025/D-050): the retry cycle this entry
deferred ("the retry cycle around it is T-025") has landed — busy /
locked attempts now replay up to three times before `E_DB_LOCKED`
reaches the wire, and the recorded fallback's claim on the same code
is re-examined; see D-050 for both halves. This entry's "the executor
does not log" rule gained one deliberate exception with it: the
executor logs the retry attempts themselves (`logging::db_busy_retry`,
a numeric-only warn) because no other layer can see them — see D-050.

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

Addendum (2026-11-18, T-024/D-049): the abort half of this entry's
startup paragraph is narrowed — the corrupt verdict no longer aborts
the launch. The `setup` hook logs this entry's facade line and
continues without managing a `Db` (recovery mode), which is what
makes the §10 screen and its `db_backups_list`/`db_restore_backup`/
`db_recreate`/`app_open_data_folder` commands reachable; see D-049.

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

### D-048 — SEC-007 modes: both backups dirs 0700, DB file and every copy 0600; the layer that creates the artifact enforces its mode, re-asserted on every run — 2026-11-18

Problem: spec §13 SEC-007 ("Rights: backups folder `0700`, DB file
`0600` on Unix. On Windows, the user profile rights are inherited")
left four questions open. Which folders "backups" means — the layout
has two backup directories, `backups/` and `db-backups/`; which layer
sets the DB file's mode (the file is created by SQLite inside
`open_db`, D-030); what an already-existing path with wrong modes
costs (D-027's layout creation is deliberately a no-op on existing
directories); and what the spec's silence means for the WAL/-shm
siblings and the backup copies (D-046 parked exactly this at the
SEC-007 task's door: the `db-backups/` mode and "every copy's mode
together").

Solution, four decisions:

- _Objects covered_: both backup directories (`backups/` for project
  file backups, `db-backups/` for the DB copies — D-046's assignment;
  the copies are the same sensitive data as the database, and the
  card's narrow "backups/" reading is satisfied alongside it), the
  database file, every backup copy (pre-migration and daily — one
  enforcement point at the shared staging step covers both schemes),
  and the WAL/-shm siblings. The siblings are not in the spec's text;
  they are covered by its spirit (they hold un-checkpointed committed
  database frames) through ordering rather than chmod: `open_db` sets
  the main file `0600` _before_ the `journal_mode=WAL` switch, and
  SQLite (≥ 3.22, bundled build verified empirically by a pinned test)
  creates the siblings with the main file's mode — so no per-sibling
  chmod exists on the creation path to drift or race. A sibling that
  _pre-dates_ the open (a crash leftover, a restored data directory)
  sees no creation-time copy — SQLite reuses it at whatever mode it
  carries — so the open ends with a best-effort re-tighten of
  `-wal`/`-shm` when they exist (round-1 review finding, reproduced
  empirically: a loosened `-wal` stayed `0644` for a whole session
  before the fix); best-effort because it is extra coverage on top of
  the creation-time guarantee, and an exotic filesystem refusing a
  repair must not fail the open over it. `db/`, `logs/` and the
  data-dir root deliberately keep the OS defaults: SEC-007 names the
  backups folders and the DB file only (their contents are protected
  by the file modes inside `db/`, and the logs by SEC-008's own
  rules), and widening the requirement beyond the spec is a spec
  change, not a task decision.
- _Layer: whoever creates the artifact enforces its mode_. The
  directories belong to the app layer, so `ensure_data_layout` (src-tauri)
  chmods both backups dirs to `0700` — a plain `set_permissions`, not a
  `DirBuilder` mode, because the exact bits are required and `mkdir`
  modes are umask-filtered. The database file belongs to the store, so
  `open_db` sets `0600` right after `Connection::open`, before the
  pragma chain (the ordering that covers the siblings); every consumer
  of the store gets the guarantee, the same reasoning that put the
  DB-001 pragma set inside `open_db` (D-030). The copies belong to the
  store's staging step, so `stage_and_link_copy` sets the `.tmp` to
  `0600` after `VACUUM INTO` and _before_ the `hard_link` — the link
  shares the inode, so the published name and even a crash-left `.tmp`
  never carry wider bits. Error shapes follow the same split: a failed
  chmod of the DB file is the new `StorageError::Permissions` (an open
  failure — the launch aborts, D-027's policy, because continuing
  would leave the data world-readable); a failed chmod around a copy
  is `BackupError::Io` inside `StorageError::Backup` (a backup-write
  failure, keeping D-038/D-046's per-scheme abort/degrade policies
  untouched); a failed directory chmod is the existing
  `DataLayoutError` with its Display generalized to "failed to create
  or secure". One addition from the round-1 review: the staging
  chmod's own failure branch removes the staged `.tmp` best-effort
  like every other failure path of the staging function — a chmod
  failure must not leave a umask-mode full copy of the database under
  the staging name. That one branch is deliberately unpinned by a
  test (round-2 review, recorded): a chmod of a file the test process
  itself just owned cannot be made to fail portably — permission bits
  do not stop the owner, and root bypasses them — and an injection
  seam exists for nothing else, so the pin would test the seam, not
  the behavior.
- _Existing paths converge, they are not skipped_: every enforcement
  point re-asserts its mode on every run — the backups dirs at every
  startup, the DB file at every open, copies at every write. A
  directory restored at `0755` or a file copied in at `0644` converges
  back on the next run (the idempotency principle over D-027's
  "existing directories are left alone", which is about _creation_,
  not modes; the layout doc contract now says both). One deliberate
  exception: a same-day daily copy that already exists is _not_
  re-chmod'ed — D-046's no-op contract ("writes nothing, changes
  nothing, the copy is never opened") wins over convergence; the
  directory's `0700` already shields such a copy at rest, and no
  released binary has ever written a wider-mode copy.
- _Windows inherits_: all enforcement is `#[cfg(unix)]` with trivial
  no-op `#[cfg(not(unix))]` halves (macOS is Unix and takes the same
  path); nothing is chmod'ed on Windows, where SEC-007 assigns the
  rights to the user profile. The no-op halves are compile-checked by
  CI's Windows leg (REL-001), not by the Linux test run.

Honest boundary recorded: the bare gate connections in `Db::open`
(`reject_too_new`, `reject_corrupt`) can create a 0-byte database file
at the OS default mode microseconds before `open_db` tightens it (the
window contains an empty file, never data); a first-run window exists
between `VACUUM INTO` and the staging chmod where a `.tmp` still has
the umask default — bounded to the staging name, swept by the next
run, and invisible under the already-`0700` directory; and the mode
step is skipped for the two non-file spellings of the path —
`":memory:"` (the store's own WAL-verification fixture) and an empty
path (SQLite's private temporary) — where no file exists to protect.

Testing: the store suite pins its own artifacts (fresh file `0600`,
loosened `0644` repaired on re-open, pre-migration and daily copies
`0600`, the WAL/-shm siblings owner-only under an open connection, a
loosened pre-existing `-wal` re-tightened by the next open — the
round-1 fix's own pin, with the first connection held open so the
sibling is genuinely reused, not recreated); the `data_dirs` suite
pins the directories (`0700` exact after the first run, a loosened
`0755` converged on the rerun, stable across reruns); `db_tests.rs`
carries the card's acceptance criterion — the real first-run seam
(`ensure_data_layout` + `Db::open` + the daily copy, the setup hook's
order) leaves every required mode exactly, and a second startup
re-tightens loosened modes. Mutations verified the suite bites:
disabling each of the three enforcement points fails its own test
(dirs `0755`, file/copies `0644`). Honest coverage boundary, noted by
the round-1 review: CI runs `cargo test --workspace` on Linux only
(the macOS and Windows legs build but do not test), so the mode
assertions execute on Linux and the macOS half of the card's
"Linux/macOS" criterion rides on the per-OS manual smoke checklist in
ARCHITECTURE.md; the Windows no-op halves are compile-checked there.

(The D-047 number is not used by this entry: it was issued inside
another task's window and belongs to that task's record.)

### D-049 — Recovery mode: the corrupt startup continues without a Db; the recovery commands are file operations over a published DataRoot — 2026-11-18

Problem: T-024 builds the backend of the §10 recovery screen: four
commands (`db_backups_list`, `db_restore_backup`, `db_recreate`,
`app_open_data_folder` — names fixed by §7.10/D-009) must work exactly
when the startup database open has failed, yet every command
infrastructure so far assumes the managed `Db`. Open questions: what
the process does after the `E_DB_CORRUPT` verdict (D-027 aborts every
init failure — a dead process shows no screen); where recovery
operations sit relative to the `Db::run` executor (D-037); how the
commands get their paths without touching the real user data
directory from tests; what the damaged file's preserved name is and
how the replacement is applied atomically; which §10 codes the
recovery's own failures map to; and what the list DTO carries.

Solution, six decisions:

- _Lifecycle — the corrupt startup continues, database-less._ The
  `setup` hook's corrupt arm now logs through `logging::db_corrupt`
  and returns `Ok` with **no** `Db` managed: recovery mode. §10's
  `E_DB_CORRUPT` effect is a screen _instead of_ the app (data
  directory path, `db-backups/` list, "Open data folder", "Start with
  empty database"), which requires a live process; every other init
  failure (too-new, rolled-back migration, generic open) keeps the
  D-027 abort — their §10 effects end the session and their screens
  are the update task's. The frontend screen is the recovery-UI
  task's; the hook's backend contract is "corrupt → logged, alive,
  database-less", narrowed from D-045's "typed error + facade line +
  abort". After a successful recreate/restore the command itself
  performs the hook's tail — `Db::open` over the replaced path
  (§7.10's own "…and reopens it"), the ERR-001 daily copy, then
  `app.manage` — so the app continues without a restart.
- _Boundary — recovery is NOT `Db::run`._ The executor serves
  commands over a _working_ database; recovery is plain filesystem
  work plus the bare read-only startup gates re-run over a chosen
  copy (`reject_too_new`/`reject_corrupt`, made `pub(crate)` — one
  detection mechanic for the live file and the copies it may be
  replaced from, T-018/T-020 BYO as the card asked). The mechanics
  live in `recovery.rs` (`recreate_files`/`restore_files`/
  `list_backups`), composed with `Db::open` by `commands.rs` inside
  one `spawn_blocking` closure (SYS-001's policy applied to file
  copies and quick_check scans, not just SQLite). Paths come from a
  new managed `data_dirs::DataRoot` — the root the hook resolved
  _before_ the open could fail; a plain value, so the mock-runtime
  IPC tests point it at temp roots instead of the real
  `~/.local/share/pro.yepbro.eunomiasync` that a call-time
  `app_local_data_dir()` would resolve.
- _Preserve mechanics — renamed, never deleted, siblings first._
  Both destructive commands first preserve the damaged database under
  the §7.10-fixed name `db-backups/pre-recovery-<UTC>.sqlite3`
  (stamp `YYYYMMDDTHHMMSSSSSZ`, the store's shape, with `-N` suffix
  variants on a same-millisecond collision — uniqueness is a Windows
  correctness requirement: renaming onto an existing target fails
  there; the free-name probe checks the sibling spellings too, so a
  half-completed earlier attempt cannot break a retry). Stale
  `-journal`/`-wal`/`-shm` siblings are renamed _before_ the main
  file and alongside it (`pre-recovery-…-wal`): a sibling left in
  `db/` would be presented to the _next_ database file that opens
  there, and renaming siblings first makes the main rename the commit
  point — a halfway failure leaves the damaged file in place and the
  step re-runnable; siblings move even when the main file is missing
  (a hand-deleted `db/` still must not feed a stale WAL to the next
  bootstrap). The preserved file is chmod'ed `0600` _before_ the
  rename and the siblings best-effort on the way (D-048: this step
  creates new artifacts in `db-backups/`, and the creating layer
  enforces its mode). Round-1-review discovery, verified empirically:
  a bare gate read over a damaged rollback-mode database makes
  SQLite's own crash recovery _remove_ siblings it considers bogus
  (fake `-journal`/`-wal`/`-shm` do not survive `reject_too_new`) —
  the D-040/D-045 accepted boundary doing this code's pre-work; the
  preserve's sibling handling is the belt-and-suspenders for what
  those reads leave behind (a `-wal` SQLite could not checkpoint), so
  its contract is pinned at the `preserve_damaged` seam, not through
  the composed commands.
- _Replacement order — stage before the preserve, publish after it_
  (round-1 review finding, now the recorded shape): the chosen copy is
  staged as `db/eunomiasync.sqlite3.tmp` (a stale staging file swept
  first, `0600` before publication, fsynced) **while the live file is
  still in place** — the most failure-prone step (copying bytes: a
  full disk, a permission, an unreadable source) runs with the
  damaged database still at its path, so its failure leaves `db/`
  exactly as it was; then the preserve renames the damaged file away,
  and one rename publishes the staged copy (§9.5's staging pattern,
  parent-directory fsync included, best-effort). The window in which
  the database path is empty is the two renames; the preserve audit
  line rides an `on_preserved` callback into that window, so a crash
  between the renames still leaves the "where did my old database
  go?" trail in the log. The copy in `db-backups/` is only ever read:
  the published database is a byte copy, never a hard link (a link
  would make every later SQLite write mutate the backup it came
  from), and it is never consumed.
- _Validation — base names of database copies, gates before touch._
  The UI sends a base name; anything with separators, `.`/`..`, or a
  `.tmp` staging suffix is `E_VALIDATION` (reason params: "not a
  plain file name" / "a staging file, not a copy" / "not a database
  copy" / "no such copy"), as is a name with no regular file behind
  it — the escape attempts the screen must survive (SEC-009's
  spirit). The "not a database copy" reason carries round-1 review
  finding 2: only `*.sqlite3` names list and restore — the listing
  and the selection share the rule, because a 0-byte `-wal` sibling
  would pass both gates (they skip empty files) and silently
  bootstrap an empty database; both halves also refuse to follow
  symlinks. The copy itself must pass both startup gates _before_
  anything is staged: too-new copy → `E_DB_TOO_NEW`, damaged copy →
  `E_DB_CORRUPT` (a 0-byte daily copy of a fresh database stays a
  valid restore target — D-046's own case). File-mechanics failures
  map contextually on the OS cause: permission → `E_NO_WRITE_ACCESS`,
  full → `E_DISK_FULL`, everything else → the recorded D-037 fallback
  with the reason in `details`; a reopen failure after the
  replacement keeps the `DbInitError::app_error()` typed form where
  one exists (the fallback otherwise — a command cannot perform the
  D-027 abort). Three command-level guards are `E_VALIDATION` too: a
  call while a healthy `Db` is managed ("a database is already
  running" — the §10 screen is not shown in that state), a concurrent
  recovery ("already in progress" — both end in `app.manage`, which
  panics on duplicates; the gate is a `LazyLock`-held async mutex so
  its `Send` guard covers the `.await`, the deliberate inverse of
  D-037's std-mutex rule), and — round-1 review finding 9, scoped to
  `db_recreate` only — a live file both gates accept ("a working
  database is already in place — restart the app"): the empty start
  is the screen's last resort, and the one state with a healthy
  unmanaged file is a restore whose placement succeeded but whose
  reopen failed, where the honest answers are retrying the restore
  (the copy is still there) or restarting, never silently discarding
  a working database. `db_restore_backup` stays exempt: swapping one
  validated copy for another with the previous file preserved loses
  nothing.
- _DTO and the opener seam._ `db_backups_list` returns
  `DbBackupInfo { name, size_bytes, modified_utc }` — the screen's
  own facts; the copy's kind (pre-migration/daily/pre-recovery)
  stays readable from the name, not duplicated as a field. All
  regular files are listed (a preserved damaged copy included —
  picking it answers with the honest gate rejection, not a hidden
  row), staging debris excluded, newest mtime first, ties by name.
  `size_bytes` is `u32` (saturating): specta forbids BigInt-style
  types on the TS wire, and a >4 GiB copy is outside this app's
  envelope. `modified_utc` is jiff's RFC 3339 UTC rendering.
  `app_open_data_folder` calls the opener plugin's Rust API
  (`OpenerExt::open_path`) — SEC-001's Rust-only plugin rule, with
  `capabilities/default.json` still `permissions: []` (pinned). Its
  real call is the one untestable seam (spawning the desktop file
  manager from a test is a side effect): the error mapping is
  unit-tested, the wiring is text-pinned, the call itself is the
  manual smoke runs'.
- _Audit._ Recovery is audit-important (SEC-008): six facade lines —
  `recovery_preserved` (both paths; emitted before the replacement is
  published so a crash between them still leaves the trail),
  `recovery_recreated`/`recovery_restored` (what the database now is,
  where the copy came from), `recovery_failed` (§10's every-error
  rule, code + params), `data_folder_opened`/
  `data_folder_open_failed`. The `db_backups_list` failures log
  through `recovery_failed` too.

Testing: `recovery_tests.rs` covers both card criteria through the
real seams (recreate over a quick_check-failing file → the original
preserved byte-identically under the pinned name at `0600`, `Db::open`
bootstraps a working empty database; restore from a copy holding a
marker → the marker readable through the reopened `Db`, the damaged
file preserved, the copy itself untouched and still readable), the
listing (DTO fields, newest-first, tie-by-name, debris/directory
exclusion of staging debris, preserve siblings, foreign files and
directories, the honest empty list), the escape/non-copy matrix
(eleven spellings rejected as `E_VALIDATION` with nothing touched —
bytes compared, no preserve file), the gate rejections over the copy
(`E_DB_CORRUPT` with SQLite's own evidence, `E_DB_TOO_NEW` with the
fixture version), sibling preservation at the owning seam (content
and the best-effort re-tightened mode, `-journal` included) and the
missing-main-file sibling sweep, same-millisecond name uniqueness,
the missing-file bootstrap, staging sweep, the failed-stage window
(an unwritable `db/` fails as `E_NO_WRITE_ACCESS` with the live file
byte-identical and nothing preserved — the round-1 criterion), the
recreate refusal over a healthy unmanaged database, and the restored
file's mode; `ipc_tests.rs` (recreated in the removed greet tests'
harness) drives the commands over real mock IPC — the DTO wire form,
the §7.10 error envelope of a rejected escape (the D-034 pin for the
first fallible commands), full recreate and restore round trips
(preserved + managed `Db` afterwards, the copy's data readable
through it), the stale-screen rejection, and the wrong-payload
rejection.
before the open, corrupt → log → `Ok`, the others' abort) and the
registration (four names, opener from Rust, empty capabilities)
from unwiring. The commands are runtime-generic (`R: Runtime` — the
tauri-specta-documented shape), which is what makes the mock-IPC
drive possible. No new dependencies; `jiff`'s and `rusqlite`'s edges
predate this task, and tauri-specta's generic-command support is
already in the pinned RC.

Honest boundaries: a crash between the preserve rename and the
publication rename leaves the database path empty (the two-rename
window the staging order shrank to its minimum) — the next startup
bootstraps fresh while the original sits under its `pre-recovery-`
name (the data is never lost, but the user must know to look there;
the preserve audit line, logged inside the window, names it); the
copy/publish steps carry no §9.5 transient-hold retry loop (a
Windows sharing violation surfaces as the retryable D-037 fallback —
polish for a follow-up, like the store's `retry_transient_hold`);
the daily-cap prune deliberately never touches `pre-recovery-*`
files, so repeated recoveries accumulate (retention is a future
owner decision); `db_backups_list` may briefly list a copy mid-write
of a concurrent startup (the name exists before its content is
linked — the screen picks from names, and the restore's own gates
re-verify); and the recovery commands do not exist for the
too-new/migration aborts — those keep the abort until their own
screen tasks.

### D-050 — Busy-retry cycle in `Db::run`: three replays in fresh transactions, §9.5 pauses, the fallback re-examined — 2026-11-18

Problem: T-025 turns §10's `E_DB_LOCKED` row ("busy_timeout expired …
up to 3 automatic retries") into behavior. Open questions: which layer
owns the cycle; what exactly is "the operation" that gets retried,
given the executor hands the operation a `&Transaction` and the
transaction is per-attempt; how the executor recognizes a busy
failure that surfaced _inside_ the operation body, where only the
classified `AppError` is visible; which backoff; how an acceptance
test holds a real lock without sleeping through 4 × 5 s of the DB-001
`busy_timeout`; and — recorded in D-037 as this task's business —
what happens to the gap-fallback ("everything else → `E_DB_LOCKED`")
now that the code's meaning grows "we retried three times".

Solution, in `src-tauri/src/db.rs`:

- _Layer — the executor itself._ The cycle lives inside `Db::run`'s
  `spawn_blocking` closure: the transaction boundary, the busy
  classification and the replay are all executor facts, and every
  future command through `run` inherits the §10 policy without
  per-command retry code. One attempt is the whole D-037 executor
  factored into `attempt_once` (lock → `BEGIN DEFERRED` → operation →
  `COMMIT`), returning a tri-state: `Success` / `Busy` (SQLite's
  busy/locked class — a retry candidate) / `Failed` (everything
  else, surfaced on the first attempt). The mutex guard is local to
  `attempt_once`, so the connection is free during every pause and
  each attempt re-locks.
- _The operation is `FnMut`, not `FnOnce._ A retry re-runs the whole
  body in a fresh transaction. That is safe because the failed
  attempt rolled back completely (the rollback is the transaction
  drop D-037 already documents), so the retry is a fresh run over
  whatever is committed when it starts — another operation may commit
  during a pause, which is precisely the contention the cycle exists
  for. The transaction is the idempotency mechanism for everything
  the body does through it; the body must read its inputs inside the
  transaction (never cache them across attempts), and a body with
  side effects outside the transaction must make them idempotent
  itself (§9.5 staging being the house pattern). The signature change
  compiled without touching a single existing closure.
- _Policy._ Up to `BUSY_RETRY_COUNT = 3` automatic retries after the
  original attempt (so at most 4 invocations — §10's "up to 3
  automatic retries" counts retries, not attempts), pausing
  `BUSY_RETRY_PAUSES_MS = [100, 300, 900]` between them: §9.5's own
  ladder, already the codebase's backoff (the store's
  `retry_transient_hold` for file-copy retries) — the same pattern,
  both citing §9.5; the literal is duplicated deliberately (the
  store's copy is function-local and not visible here, like the
  5000 ms value `schema_version_of` duplicates). The sleep is `std::thread::sleep`
  inside the blocking closure: the whole cycle already runs on
  tauri's blocking pool, and no async worker is ever parked (the
  existing responsive-runtime test keeps proving that end to end).
- _What retries — the class, not the code._ Only rusqlite's
  `DatabaseBusy`/`DatabaseLocked` primary categories (the extended
  codes — `SQLITE_BUSY_SNAPSHOT`, `SQLITE_LOCKED_SHAREDCACHE`, … —
  fold into them; shared cache is not enabled here, but the class is
  covered uniformly). Two detection edges: the executor's own
  `BEGIN`/`COMMIT` see the raw rusqlite error; a statement inside
  the body sees only the classified `AppError`, so the busy kinds are
  matched on `details.os_kind` against the same `Debug` names the
  classifier wrote — both strings derived from the rusqlite enum at
  runtime, so a variant rename moves producer and matcher together.
  Everything else — the operation's own `E_VALIDATION`, constraint
  violations, the fallback `E_DB_LOCKED` of an I/O failure, an error
  with no details — returns on its first attempt without pausing.
  The wire form of the verdict is unchanged §7.10 shape: the §10
  literal, the default `errors.db_locked` key, the SQLite facts in
  `details`, and **no params** — UI-011's params are "where" values
  (a path, a project); the busy situation has none, and the attempt
  count belongs to the log lines, not the interpolation contract.
- _Observability._ Each retry emits one `warn` through a new facade
  function, `logging::db_busy_retry(attempt, pause_ms)` — numeric
  payload only (SEC-008). This is the executor's single deliberate
  exception to D-037's "the executor does not log": a retry is
  invisible to every other layer (a successful replay never becomes
  an error; the exhausted verdict is the producing command's own §10
  line), so nobody else could log it. The call site is pinned by a
  wiring test in the `config_tests.rs` style.
- _The D-037 fallback, re-examined and kept._ With the cycle landed,
  `E_DB_LOCKED` from a genuine busy now additionally means "retried
  three times, still busy" — which a fallback producer (an I/O
  failure, a panic) did not do, so the fallback lies slightly more
  than before. It stays anyway: of the twenty codes its §10 effect
  ("operation rejected", user action "Retry") remains the least
  harmful for an unclassifiable failure — `E_DB_CORRUPT` forces the
  destructive recovery screen, `E_DISK_FULL`/`E_NO_WRITE_ACCESS`
  prescribe unrelated user actions, and no generic
  infrastructure-failed code exists (the recorded gap). Two
  mitigations bound the lie: fallback errors never enter the retry
  cycle (only the busy/locked class does — pinned by a test that
  counts one invocation for an I/O-class failure classified onto
  `E_DB_LOCKED`), and `details.os_kind` separates the producers on
  the wire (`DatabaseBusy` = genuinely busy and retried;
  `SystemIoFailure`/`OperationPanicked`/… = fallback). Revisit when
  the registry grows a fitting code, as before.
- _Test seam for `busy_timeout`._ The acceptance tests hold a real
  external lock (`BEGIN EXCLUSIVE` on a second rusqlite connection —
  in WAL mode the immediate write lock a competing writer needs) and
  let every attempt wait out the timeout, so at the DB-001 production
  value one test would sleep 4 × 5 s of timeouts plus §9.5 pauses.
  `Db::set_busy_timeout_for_tests(Duration)` — a `#[cfg(test)]`
  method — replaces the connection's busy timeout with the test
  value (25 ms). DB-001 is untouched in production: the method does
  not exist outside `cfg(test)`, and `open_db` still programs 5000 ms
  on every open. Determinism comes from the lock, not from timing:
  every attempt deterministically collides while the holder lives,
  and the release-between-attempts test synchronizes through channels
  (the body reports its busy failure and waits for the release before
  returning it, so the retry starts against a genuinely free lock —
  correctness never depends on the 100 ms pause being long enough).
  The busy tests were run three times in a row for stability
  (~1.4 s each). Accepted production cost, recorded with the seam: a
  lock that outlasts everything makes one `Db::run` occupy the
  executor for up to ~21 s (four 5 s DB-001 timeouts plus 1.3 s of
  §9.5 pauses), during which other operations queue on the connection
  mutex (the guard is held per attempt, freed for every pause —
  pinned by a dedicated test); the values are spec-mandated
  (DB-001/§9.5/§10), not tuned here. The cycle is also not
  cancellable: dropping the caller's future does not stop the
  `spawn_blocking` run, whose retries (and a possible commit) run to
  completion — a pre-existing property of one attempt, widened to
  the worst case; recorded in `Db::run`'s docs for the future
  command tasks.

The acceptance behavior is pinned in `db_tests.rs`: exactly four
invocations then the `E_DB_LOCKED` wire verdict with
`os_kind = DatabaseBusy` and a wall-clock floor of the summed §9.5
pauses (a no-pause loop cannot undercut it; no upper bound to stay
flake-free on loaded machines); nothing of the failed attempts
committed and the same connection serves the next operation once the
lock is gone; the synchronized-release test commits on the first
retry; an operation's own error, a real constraint violation and the
fallback `E_DB_LOCKED` all run exactly once; both busy discriminators
are driven directly on synthesized primary and extended result codes
(the acceptance tests cannot reach the `BEGIN`/`COMMIT` edges or the
`DatabaseLocked` member); the connection lock's freedom during the
pauses has its own synchronized test (a queued operation provably
runs mid-cycle while the retried operation is still in flight); and
the facade test pins the warn payload. No new dependencies (no
async-sleep crate, no retry framework — the loop is 30 lines over
std).

### D-051 — Action-log repository: in-transaction success entries, fail-then-log error entries; a closed typed `details`; the LOG-001…004 action vocabulary — 2026-11-18

Problem: T-026 turns the `action_log` table (schema D-036) into the
journal service of spec §7.7. Three open questions. (1) Transactional
semantics — the card says "the entry is written in the same
transaction as the operation", but LOG-001…004 also names "errors" as
journaled, and an error usually means the operation's transaction
_rolled back_: an entry written inside it would vanish with it. (2)
SEC-008 — `details` is a JSON column, and the acceptance criterion is
"in details there is no instruction text": the mechanism had to be
structural (the D-032 facade spirit), not a remembered rule. (3) The
`action` column is free TEXT in the schema — the closed vocabulary
LOG-001…004 implies had to be encoded somewhere, with the mapping to
the requirement's own wording fixed.

Solution: `eunomia_store::action_log`, the second repository after
`settings` (D-039's shape).

- _Transactional semantics, resolved explicitly._ One function,
  `insert(&Connection, &LogEntry)`, serves both halves. A successful
  operation writes its entry **inside the operation's own
  transaction** (the `settings::set` shape: a single statement that
  joins the caller's transaction), so an operation that rolls back
  leaves no entry — the journal must not claim an action that never
  happened. A _failed_ operation cannot use that transaction (the
  rollback would erase the entry), so its producer follows
  **fail-then-log**: let the operation's transaction roll back, then
  write the entry in the **next** transaction — the same action, now
  `Level::Error`/`Outcome::Error`, the §10 code in details. The
  module docs record the pattern; the integration suite pins both
  halves (a rolled-back operation leaves no entry; the fail-then-log
  entry survives, and the operation's data change is gone with the
  success entry). The store API stays executor-agnostic: nothing here
  knows `Db::run`; the future command tasks apply the pattern at
  their own `run` boundaries.
- _SEC-008, structural._ `Details` is a closed enum — `Bulk{changed}`,
  `File{path}`, `Failure{code, path?}` — whose fields are numbers,
  paths and a validated newtype; there is no variant accepting free
  text, so the variant content a failed operation holds in scope has
  nowhere to go (the `file_event` argument, applied to the database
  journal). The one string-bearing field, `FailureCode`, is a newtype
  whose `new` validates the §10 registry shape (`E_` + uppercase
  alphanumeric groups, single underscores): every instruction-shaped
  string — prose, lowercase, spaces, markers — fails the shape. Form,
  not membership: the §10 member list lives in the app layer's
  `errors.rs` registry (D-034), and duplicating it into the store
  would create a second source of truth that drifts at the first new
  code (D-017 keeps the crates decoupled); a new §10 code therefore
  needs no store change. The acceptance criterion is pinned from
  three sides: unit tests of the shape check (registry literals
  accepted, instruction markers rejected), an integration test that
  builds an entry beside a realistic instruction text and reads the
  stored cell back marker-free, and the facade test below.
- _The action vocabulary._ `Action` is a 21-member enum; `as_str` is
  the `object.verb` lowercase shape the schema suite already pins
  (`project.added`). The mapping to LOG-001…004's own list is
  member-by-member and pinned by a unit test that hardcodes the
  requirement's categories as an oracle: adding/changing/archiving/
  restoring projects; changing settings; creating/editing/hiding/
  restoring instructions; creating/editing/restoring variants plus
  their "hiding" half — for variants that is the §6.4 soft delete
  (`VariantDeleted`, since variants have no hidden state);
  assignments (bulk = one entry, the count in `Details::Bulk` — the
  requirement's own parenthetical); heading operations
  (created/changed/deleted, §6.4's physical delete); sync runs
  (`SyncRunFinished`) and their per-project results
  (`SyncProjectResult`) — the requirement's "and" reads as two
  entries; backup creation (`BackupCreated`, the BKP-001 target-file
  copies); errors (`ErrorReported`, e.g. a DB-017 invariant
  violation). A _failed_ operation deliberately has no member of its
  own: it is journaled as its own action with `result = error` (the
  fail-then-log semantics). Reads reject unknown stored actions with
  the D-039 corrupt-cells policy (`StorageError::ActionLog`) — every
  writer goes through `insert`, so an unknown member means corruption
  or version skew, not a value to guess at; the same error covers a
  non-JSON `details` cell and a vocabulary violation in any other
  column. The `entity_type` column gets the same treatment: a closed
  `EntityType` enum (project, instruction, heading, variant,
  assignment, sync_run, backup, setting), and an `entity_id` without
  its `entity_type` (or vice versa) fails the read instead of
  silently dropping the half that exists.
- _Timestamps and ids._ `insert` stamps `ts` itself with `now_utc()`
  (DB-002, D-031/D-035) — one timestamp convention regardless of
  producer, pinned by the round-trip test's shape assertion. The
  autoincrement id is returned; no UUIDv7 here (the schema's own
  `INTEGER PRIMARY KEY AUTOINCREMENT`, D-036).
- _Reads._ `recent(conn, limit)` — newest first, descending id — is
  the minimal seam: the "Activity" window's filters and pagination
  (by date, project, type, result) are T-027's and are not designed
  here; `details` comes back as parsed JSON, because interpreting the
  payload is the reading task's business. LOG-006's retention
  (10 000/180 from settings, startup + daily cleanup) is explicitly
  T-029's: nothing in the module deletes rows.
- _The tracing mirror._ The card's "duplicated into tracing without
  content" is one new facade function, `logging::action_logged(&LogEntry)`
  (D-032's pin keeps tracing macros in `logging.rs` alone), with the
  decision table split into `mirror_level` for direct testing. Policy,
  decided by LOG-005 and §10: **everything mirrors except the
  info+user+non-error shape.** Warn and error entries mirror at their
  own level regardless of visibility — including `diagnostic` ones,
  whose only other surface is diagnostics, so not mirroring would
  leave the database journal as their single home. An info entry
  mirrors in two cases: `result = error` — §10's "every error is
  written to the log" must hold even when a producer forgot the
  documented error-level pairing (the round-1 review's defense
  suggestion) — and `visibility = diagnostic`, per LOG-005's "only in
  the file log and in diagnostics". The one unmirrored shape — info +
  user + ok/skipped — is the "Activity" window's own data (the
  database journal already holds it, and the per-event facade
  functions already write the file log's routine lines; mirroring
  every user entry would double the file's volume with duplicates).
  The policy lives in the facade, not at callers: they call once per
  written entry. The mirrored line carries action/result codes,
  entity type/id, project and sync-run ids — SEC-008's vocabulary;
  `details` stays in the database alone (its paths and codes are
  already represented where they matter). No production caller yet:
  the first command task that writes the journal calls it (the
  `file_event` tested-today/consumed-later shape), and
  `db_backup_daily`'s boundary comment is updated to match — the
  journal exists, but LOG-001…004's "backup creation" is the BKP-001
  target-file family, not the ERR-001 daily database copy, whose
  journaling is a visibility decision for the startup-wiring task.

Dependencies: none new — `serde_json` (D-039) and `rusqlite` cover
everything. Tests: 7 unit (schema CHECK literals as an oracle, the
LOG-001…004 mapping table with placement, dotted-unique action
strings, the §10 shape check both ways over the full spec table, the
canonical details JSON forms, the beside-instruction-text unit pin,
`LogEntry::new`) and 8 integration over real migrated databases
(full-field round-trip with the DB-002 `ts`, minimal-entry NULL
columns, newest-first with limit, the rollback half, the fail-then-log
half, the visibility pair, the tampered-cell matrix, the
database-side SEC-008 pin; every test tears its temp directory down),
plus two facade tests in `logging.rs` (the mirror decision driven
end-to-end through a real subscriber — every shape mirrored at its
level, the info+user+ok shape absent, no instruction marker in the
file — and the `mirror_level` table directly). The round-1 review
(deepseek-v4.1-flash, APPROVE, no P0/P1) tightened the mirror policy
(the error-result defense and the diagnostic-info mirror), the
mapping test (placement, not just coverage), the temp-dir teardown,
four broken rustdoc lead-ins, and several doc-boundary notes recorded
above. The round-2 review (hy4-flash-free, APPROVE, no P0/P1) caught
one real oracle defect in that fix — the §10 sample had transcribed
`E_NOT_UTF` for the table's `E_NOT_UTF8` and included two §9.2
`MOVEFILE_*` flag names that are not registry codes — and the oracle
now carries exactly the table's 22 codes; it also tightened the
`StorageError::ActionLog` doc to read-path-only wording.

### D-052 — `activity_list`: inclusive exact-shape date bounds, `ts DESC, id DESC` ordering, user-by-default visibility at the IPC boundary — 2026-11-18

Problem: T-027 turns the "Activity" window's read (spec §7.7 UI row:
"filters by date, project, type, and result"; §7.10 History module's
`activity_list`) into the store's query and the first History IPC
command. Open questions: what the date filters compare as (the `ts`
column is TEXT); which order a paginated window needs when entries
share one millisecond; how LOG-005's "user in the window, diagnostic
only in diagnostics" becomes a wire contract whose default cannot leak
diagnostic entries; how the closed store vocabularies reach
TypeScript; and what the response's pagination meta carries.

Solution, six decisions:

- _Date bounds: inclusive both ends, exact DB-002 shape only._ The
  bounds are compared as **text** against `ts` (`ts >= ? AND ts <= ?`),
  which is equivalent to chronological comparison only because the
  new public `values::is_db002_shape` predicate holds: the format is
  fixed-width with fixed offsets and always the `Z` suffix, so its
  lexicographic order IS its chronological order. A bound that merely
  parses as RFC 3339 breaks that ("…:00Z" > "…:00.000Z" as text while
  naming the same instant), so both the command and the store
  (`ActionLogError::Timestamp`) reject every non-exact shape —
  mis-ordering is not guessable at. Both ends inclusive: with
  millisecond resolution a whole day is `…T00:00:00.000Z` to
  `…T23:59:59.999Z`, and an entry stamped exactly at a bound is inside
  the range (pinned by entries at both bounds). An inverted range
  (`from > to`) is `ActionLogError::Range` — an accidental empty match
  must not look like "nothing happened that day" (the D-039
  explicit-over-silent policy, applied to a query). The rule lives
  once (`is_db002_shape`); the command checks it for named-field
  `E_VALIDATION` params, the store for its own contract — the
  predicate, not the rule, is shared.
- _Ordering: `ts DESC, id DESC`._ Time is the window's user-visible
  axis; the autoincrement id is the tiebreaker that makes an
  equal-millisecond block page deterministically (equal `ts` is common
  — `insert` stamps its own `now_utc()` and a fast producer crosses
  entries inside one millisecond). `recent` keeps its `id DESC`
  insert-order semantics deliberately: it answers "the last N things
  written" (journal replay), `list` answers "when does this say it
  happened" — the two diverge only after a backward clock step, and
  each seam's tests pin its own ordering. The `ix_log_ts` index serves
  the range scans; LOG-006's retention (10 000 rows) bounds every
  scan.
- _LOG-005 at the IPC boundary: user by default, diagnostic only on
  the explicit request._ The store's `ActivityQuery.visibility` is a
  plain filter (`None` = both audiences) — the store is the executor,
  not the policy holder. The policy lives in the command: the DTO's
  `include_diagnostic` (absent/false → `Some(User)`, only `true` →
  `None`) is the single switch, so an untrusted caller cannot reach a
  diagnostic entry by omission or by filter combination — the
  acceptance criterion is pinned at both levels (the store test loops
  every filter over twin pairs; the IPC test drives the default call,
  an explicit false, filtered calls and the explicit true). A
  diagnostics-ONLY view (no user entries) is not built: no consumer
  exists; the diagnostics section can filter client-side by the
  `visibility` field the entries carry.
- _Pagination: limit/offset + filter-matching `total`._ The journal is
  retention-bounded and filterable by every axis the UI offers;
  offset pagination is sufficient and simplest, and the response
  carries `total` (COUNT over the same WHERE, both inside the one
  `Db::run` transaction, so page and count describe one consistent
  state) — the "page N of M" the window renders. Bounds:
  `DEFAULT_PAGE_LIMIT = 50`, `MAX_PAGE_LIMIT = 500` (a response-size
  guard, not a data-set guard), offset ≥ 0; the DTO fields are `i32`
  so a negative value reaches _our_ `E_VALIDATION` instead of dying in
  serde (a JSON number too large for i32 fails deserialization at the
  invoke layer — a malformed payload, like a string where a number
  belongs).
- _Wire DTO: stored spellings as strings, `u32` ids, `details` as
  JSON text._ The closed vocabularies (`action`, `result`, `level`,
  `visibility`, `entity_type`) travel as their stored member strings;
  the request side is parsed at the boundary through the store's own
  now-public `Action::from_member`/`Outcome::from_member` — one owner
  per vocabulary, unknown wire values are `E_VALIDATION` naming the
  field/value/reason, never a silent match-all. Mirroring the 21
  members into a src-tauri specta enum was rejected: a second list to
  keep in sync for no behavioral gain (the backend writes every value;
  TS gets `string`). `id` and `total` are `u32` (saturating): specta
  forbids 64-bit integers on the TS wire (the D-049 `size_bytes`
  precedent), and an autoincrement id or row count past 2^32 is
  outside this app's envelope. `details` is the canonical JSON text of
  the store's parsed value — not `serde_json::Value` on the wire,
  because `specta/serde_json` (the feature that implements `Type` for
  `Value`) is deliberately not enabled; parsing stays the frontend's
  choice (the `modified_utc` precedent). Field names are snake_case on
  the wire, the `DbBackupInfo`/`AppError` house convention.
- _SQL shape safety and failure mapping._ The `list` statement text is
  assembled from fixed literals only; every value — including the
  generated `?` list of the `action IN (…)` filter — travels as a
  bound parameter, so no filter value can change the query's shape
  (pinned by injection-shaped project ids matching literally and
  matching nothing). Failures map contextually (D-034):
  `StorageError::ActionLog` → `E_VALIDATION` (a corrupt cell names
  its column and value), SQLite failures → the shared base classifier,
  anything else (unreachable from a read) keeps the recorded fallback
  shape. The command logs every failure through the new
  `logging::activity_list_failed` (error level, §10 code + params +
  OS facts — the `recovery_failed` rationale); a successful read is
  never journaled (the journal feeding itself) and gets no routine
  log line. Without a managed `Db` (recovery mode) the command answers
  the fallback code with a `NoDatabase` diagnostic rather than
  pretending an empty journal.

Testing: 14 store integration tests over real migrated databases
(filter matrix each-alone and combined with drop-one-grow-by-one
semantics, inclusive bounds with entries stamped exactly at them,
shape rejection matrix, inverted range, injection-shaped values,
visibility invariant across every filter, pagination tiling with a
disjoint-union check, equal-`ts` page stability, `ts`-primary
ordering with insert order deliberately contradicted, corrupt cells,
full-column round-trip), `values` unit tests for the shape predicate
and the lexicographic-ordering property (including the motivating
near-miss counter-example), five IPC tests over the mock runtime
(filtered wire DTO round-trip, the LOG-005 default/false/filtered/
explicit-true matrix, the eight-case validation envelope, the
no-database answer, the registration pin), the facade line test, and
the regenerated `src/bindings.ts`. No new dependencies (enabling
`specta/serde_json` was considered and rejected above).

The round-1 review (deepseek-v4.1-flash, APPROVE) tightened six
edges, all fixed in the same commit: (1) the wire `actions` array is
de-duplicated during parsing, bounding the generated `IN (…)`
placeholder list by the vocabulary's own size (21) — an unbounded
array could otherwise push the statement past SQLite's variable limit
and surface a frontend bug as the fallback `E_DB_LOCKED` (pinned: a
5000-entry duplicate array succeeds and filters exactly); (2) the
read path now validates stored `ts` cells against the same
`is_db002_shape` predicate as the bounds (the D-039 corrupt-cells
policy, extended to the one column the ordering claim stands on — a
tampered shapeless `ts` fails the read instead of sorting textually;
both seams, `recent` and `list`, share the decode and the guarantee);
(3) the command logs every failure path — the early validation and
no-database returns previously bypassed the single
`logging::activity_list_failed` edge the entry claimed (the body is
split so one `inspect_err` covers validation, state and executor
alike); (4) a corrupt journal cell maps to the D-037 fallback with an
`ActionLogCorrupt` diagnostic, not `E_VALIDATION` — the §10 row of
that code tells the user to fix their input, a lie about database
corruption, and `E_DB_CORRUPT` is the other lie D-037 already
rejected (one bad cell must not force the destructive recovery
screen); the raw cell deliberately does not travel in `params` (a
tampered cell is arbitrary database text, and `params` feeds the log
line and the UI message — SEC-008's surfaces), while bound failures
(`Timestamp`/`Range`, caller input) keep `E_VALIDATION`; (5) the DTO
carries `deny_unknown_fields`, so a typo'd filter key fails
deserialization loudly instead of silently widening the query to the
whole journal, and the empty-`actions`-means-no-filter contract is
pinned at the IPC level (the unset-dropdown reading — the opposite
choice would make an accidental `[]` blank the window); (6) the IPC
tests close the app's managed `Db` before removing their temp
directories (the Windows teardown rule the store suite pins).

The round-2 review (hy4-flash-free, REQUEST CHANGES) caught the
incomplete half of fix (4) — fixed in the same commit: the stored-`ts`
validation had reused the bound variant `Timestamp`, so a tampered
`ts` cell was still classified as caller input (`E_VALIDATION`) with
its raw value in `params`. The store now separates the two producers:
`Timestamp` stays the bound variant (caller input), and the read path
raises the new `TsCell` variant (stored data), which the command maps
into the `ActionLogCorrupt` fallback like every other corrupt cell —
pinned end to end over IPC (fallback envelope, empty params, the
fixed class-only diagnostic). The same review's second finding closed
the last raw-cell surface: the fallback's `os_message` is no longer
the variant's own `Display` (which embeds the stored value) but the
fixed `corrupt_cell_diagnostic` text naming the column and the
failure class — a tampered cell's arbitrary text now reaches neither
`params` nor `os_message`, the two surfaces the §10 log line and the
UI-011 message render (whoever debugs a tampered database reads the
cell from the database).

### D-053 — Project body assembly: `body::build_body` over the snapshot, scheme-independent, §9.2 transcribed — 2026-11-18

Problem: T-035 turns the §9.2 pseudocode (`build_body`) and its rules
ALG-020…022 into eunomia-core code on top of the T-033 snapshot
(D-042) and the T-034 normalization (D-044). Open questions: the
module's name (the obvious `build.rs` shares its name with cargo's
build-script convention); the function's contract for a project that
is not in the snapshot versus a project that builds nothing; how the
scheme (A/B/C) and the file skeleton relate to the body; and how the
spec's per-decision wording maps onto the pseudocode where the prose
is looser than the code (heading inclusion says "assigned" without
repeating "active").

Solution: module `body` (`crates/eunomia-core/src/body.rs`), one public
function, `build_body(&BuildSnapshot, &ProjectId) -> Option<String>` —
a direct transcription of the §9.2 pseudocode (the canon per the task
brief), with these boundary decisions:

- _Module name._ `body`, not `build`: a `src/build.rs` inside a crate
  is technically just a module, but the name is cargo's build-script
  convention at the package root and invites confusion; the crate names
  modules by domain noun (`snapshot`, `normalize`, `library`) and §9.2's
  own noun is "the project body".
- _`None` vs `Some("")`._ An unknown project id returns `None` (the
  caller mixed identifiers); a project with nothing to build returns
  `Some("")` — a legitimate build result that the sync pipeline runs
  the spec D-25 empty-build confirmation for. Collapsing the two would
  make a caller bug indistinguishable from an empty library.
- _Scheme independence._ The §9.2 pseudocode has no scheme parameter,
  and §9.3 consumes its result: ALG-030 joins the extra-file lines
  (A only), this body and the creation marker into the managed block;
  ALG-031 wraps that block in scheme B's markers. `build_body` therefore
  reads no project settings beyond the identity — the extra-file
  reading lines, the marker, the trailing newline of the managed block,
  the base-name/`<STEM>.project.md` skeleton and the build hash
  (ALG-030…033) are all later pipeline steps, explicitly out of scope
  here. The body carries no trailing line break: ALG-030 appends its
  own tail.
- _Hidden instructions (DB-013/D-11)._ The pseudocode's `included` set
  requires `state = active` AND an assignment; the prose of ALG-020
  ("at least one instruction assigned") omits the state word, but the
  pseudocode is the canon and OUT-013 agrees (edits to hidden
  instructions never affect a build). A hidden instruction therefore
  keeps no heading either: hiding an instruction empties its section
  for inclusion purposes, exactly as if its assignment were gone.
- _Empty variants (D-13)._ An assigned active instruction whose variant
  text is empty (or normalizes to empty — the rule reads the
  ALG-002-normalized text, so whitespace-only or BOM-only content
  behaves the same) stays in `included`: its heading survives (including
  every ancestor heading, since the empty-variant instruction lies in
  their ranges), but no text block is emitted.
- _Total-function posture (D-042's)._ The per-project resolution —
  assignments scoped to the project, joined with active instructions,
  each mapped to its assigned variant — is the builder's internal step
  (`selected_variants`). The variant lookup `expect`s to find the
  assigned variant among the instruction's variants:
  `BuildSnapshot::new` rejects the mismatch (DB-012) at assembly, so
  the panic marks a broken invariant, not a data condition — the same
  "validate once at the boundary" reasoning D-042 used. An assignment
  to a heading is likewise unrepresentable in a snapshot
  (`AssignmentToHeading`), so the builder needs no branch for it; that
  guarantee is pinned by the snapshot suite's own rejection test.
- _Performance._ The heading check scans each section forward to its
  close (first heading of level ≤ L, §9.2 verbatim) instead of using a
  stack: heading levels run 2–6 only, so an item lies in at most five
  sections and the whole walk is linear — well inside §12.2's 20 ms
  budget for 1000 items (5000 is the v1 ceiling), with no benchmark
  needed to defend it.

No new dependencies (a pure function; D-017's allow-list untouched).
Testing: the §9.2 example is the oracle test with the expected string
hardcoded from the spec (`## Style`, `### Tests`, C's text, no
`## Release`) and doubles as the module doctest; every rule has its own
test — section inclusion (parent kept by a nested section, empty
sibling dropped, section closed by a same-or-higher heading, skipped
levels nest), the preamble before the first heading (ALG-021), hidden
exclusion (no block, no heading, all-hidden project builds empty),
empty-variant retention (raw and normalizes-to-empty), names never
entering the file, exactly-one-empty-line separators with no trailing
break, library order winning over assignment-list order (OUT-001, DoD
5), normalization applied (BOM/CRLF/edge whitespace), the `None` and
`Some("")` boundary pair, per-project variant selection, byte-identical
bodies across schemes A/B/C and every settings field (the boundary's
own guard), and a combined scenario (S2 kept through an empty variant
in a nested subsection while hidden and unassigned neighbors leave no
trace). Five behavior mutations verified the suite bites: disabling the
active-state check (3 tests), the one-LF separator (12), skipping
normalization (3), a `<` instead of `<=` in the section-close rule (2),
and dropping the empty-text guard (3). Round-1 review
(deepseek-v4.1-flash, APPROVE, no P0/P1) added the scheme/setting
identity test, the single `level` binding, and the linear-cost doc
note; round-2 review (hy4-flash-free, APPROVE) found nothing further.

### D-054 — LOG-006 retention: the disjunction of the two log limits, strictly-older age, last 500 runs by (started_at, id); a 24 h spawned ticker, settings read in the prune's own transaction — 2026-11-18

Problem: T-029 turns LOG-006 ("Log limits come from settings (10,000
entries or 180 days by default); cleanup runs at startup and once a
day") and §6.4's "Log and runs" row ("10,000 entries or 180 days for
the log; the last 500 runs") into the retention mechanism. Open
questions: what the "or" of the two limits means for a single row;
which boundary the age criterion cuts; what "the last N" means when
`ts` and insert order disagree; whether the 500 is a setting; what
happens to `sync_run_projects` and `backup_records` when runs are
deleted; how "once a day" reaches a long-running process (T-021's
daily copy is a startup check, not a timer); and what a failed
cleanup does to the launch.

Solution, nine decisions:

- _Where the code lives._ The semantics are the store's —
  `eunomia_store::action_log::cleanup(conn, limits, now)`, extending
  the journal's own module (the module docs' "nothing here deletes
  rows" paragraph is replaced by the retention section). §6.4 names
  "Log and runs" as one housekeeping row, LOG-006 draws both log
  limits from settings, and the runs cap is fixed — one function
  keeping the three together is the shape that cannot drift apart.
- _The disjunction._ A journal row is deleted when **either** limit
  alone rejects it: it is outside the newest `max_entries` rows, or
  its `ts` is strictly older than the cutoff `now − max_days` days.
  Equivalently, surviving = inside the newest N **and** not older
  than the cutoff — tightening either setting prunes immediately
  (each criterion is pinned deleting an entry the other would keep).
  One `DELETE … WHERE id NOT IN (newest N) OR ts < cutoff` expresses
  it; §6.4's wording "10,000 entries or 180 days" reads naturally
  as the disjunction of removal conditions.
- _The age boundary is strict._ "Older than max_days" deletes
  `ts < cutoff` only: an entry stamped exactly `max_days` before
  `now` is at the limit, not past it, and survives (pinned at
  millisecond precision). The cutoff is compared as DB-002-shaped
  text — the same precondition `list`'s range filters stand on
  ([`values::is_db002_shape`]: that exact shape is where text
  comparison equals chronological comparison). Days are exactly
  86 400 s (`SignedDuration`, not a calendar `Span` — `ts` cells are
  UTC instants; no timezone exists that could make a day anything
  else for this comparison).
- _"The last N" is time-primary._ The keep-set orders by
  `ts DESC, id DESC` — T-027's `list` ordering, not `recent`'s
  insert order: a backward clock step can write a high-id entry with
  an old `ts`, and retention keeps what the Activity window would
  show as newest (pinned: a late-inserted old entry is deleted while
  lower-id newer entries survive). `sync_runs` gets the run twin,
  `started_at DESC, id DESC` — the UUIDv7 text id (D-031) breaking
  `started_at` ties deterministically (pinned with a tied pair at
  the keep-window boundary).
- _500 is a constant, not a setting._ §6.5 has no key for runs and
  LOG-006 draws only the log limits from settings;
  `action_log::SYNC_RUNS_KEPT = 500` carries the §6.4 number next to
  the query that enforces it. The log limits travel as a
  `LogLimits { max_entries, max_days }` parameter whose validation
  owner stays the settings repository (D-039): re-checking the ranges
  in the cleanup would duplicate the §6.5 canon; the precondition
  (§6.5-validated values) is documented on the type, and the app
  seam honors it by reading both keys through `settings::get` in the
  prune's own transaction.
- _Cascade, SET NULL, and the journal's own references._ Deleting old
  runs rides the schema (D-036): `sync_run_projects` follows through
  `ON DELETE CASCADE` (the connection's `PRAGMA foreign_keys=ON`,
  D-030), `backup_records` rows survive with a NULL `sync_run_id`
  (`ON DELETE SET NULL` — a file backup outlives its run), and
  journal entries naming a pruned run keep their `sync_run_id`
  string (§6.2 puts no FK there; the journal is a historical record
  — an id that names a pruned run still names what happened). All
  three are pinned; no explicit child DELETEs were added (the
  cascade is the schema's own contract, exercised here through the
  production open). Recorded boundary (round-1 review): the cap
  keeps the newest 500 runs **regardless of status**, so an app
  whose newest 500 runs are all failed/aborted loses the rows
  LOG-007's "overall last successful sync" would derive from the
  runs table — §6.4 mandates the cap, so the task that builds
  LOG-007 must read the overall time from `projects.last_synced_at`
  (or another denormalized field), never by scanning `sync_runs`.
  The schema's own `action_log` comment still says "append-only":
  the migration file is frozen (D-010/D-036), and this cleanup is
  the one deliberate deleter since T-029 — ARCHITECTURE.md says so
  next to the retention description.
- _One transaction, idempotent, caller-owned._ Both DELETEs join the
  caller's transaction (the `insert`/`settings::set` shape): the app
  seam (`src-tauri/src/retention.rs`) reads the two settings and
  prunes inside **one** `Db::run` operation (D-037), so the limits
  and the deletes describe one consistent state and a mid-operation
  failure rolls the whole pass back (pinned by an explicit rollback).
  A second pass over the pruned state removes nothing (pinned) —
  which is also why the daily tick needs no same-day guard: unlike
  the ERR-001 copy (D-046) there is no day-named artifact to key on,
  and an extra pass is a no-op, not a duplicate.
- _Startup + a 24 h spawned ticker._ `retention::spawn_daily_cleanup`
  spawns one task on tauri's async runtime: the startup pass runs
  immediately (right after `app.manage` — LOG-006's "at startup"
  fired off the `setup` thread; commands keep answering through the
  whole pass, the executor serializing them on the one connection
  behind the prune's single transaction — a bounded queue, not
  concurrency), then the task sleeps `DAILY_INTERVAL` (24 h) between
  passes. Honest
  boundaries: the tick counts from launch, not UTC midnight — an app
  kept running prunes once per day on its own schedule, which is
  the natural reading of "once a day" for a limit that idempotence
  already bounds; and the task deliberately has no cancellation
  token — process exit drops it, a pass interrupted by exit rolls
  back with its transaction (SQLite atomicity), and the next
  startup's pass re-runs. `tokio` (feature `time` only) joins
  src-tauri for the timer: tauri's `async_runtime` re-exports its
  task API but not its timers, and its docs point at tokio for the
  missing items — no new crate version (already resolved via tauri;
  the D-029 lockfile-reuse pattern), `cargo deny` green. The
  startup ordering is deliberate: after the ERR-001 daily copy (the
  copy is the day's recovery point of the state as the app found
  it, housekeeping shrinks it afterwards) and after `app.manage`
  (pinned by extending the existing `db_tests.rs` wiring pin: open →
  daily → manage → retention spawn).
- _Degrade, keep ticking, journal nothing._ A failed pass is logged
  through the facade (`logging::db_cleanup_failed`, a `warn` — the
  D-046 policy: the launch continues, an `error` line would read as
  a failed launch) and the loop sleeps on: the failure may be
  transient, the next day retries. A pass that removed something
  gets one `logging::db_cleanup` `info` line with the two counts
  (SEC-008: numeric only); a no-op gets none (the idempotent common
  case, D-046's no-op-no-line policy). The cleanup writes no
  `action_log` entry: LOG-001…004's vocabulary has no housekeeping
  member and extending the closed enum beyond the requirement's list
  is not this task's call — the facade lines are its whole journal
  (D-046's boundary note, mirrored). A corrupt `log.*` cell fails
  the settings read explicitly (the D-039 corrupt-cells policy) and
  the pass surfaces the D-037 fallback wire shape with a class-only
  diagnostic in `details.os_message` — the failing key and the
  failure class, never the stored cell's bytes (the T-027
  `corrupt_cell_diagnostic` rule; the registry has no
  "housekeeping failed" code; the error is logged, never shown).

Testing: the store suite (`tests/action_log_cleanup.rs`, 14 tests
over real migrated databases seeded by direct SQL — the history
under test is accumulated rows with explicit timestamps, which the
production writers stamp with the wall clock) pins the card's two
acceptance criteria (1200 entries → exactly the newest 1000; 501
runs → 500 with zero orphaned `sync_run_projects`), the strict age
boundary (the exactly-`max_days`-old entry survives, one millisecond
older goes), each half of the disjunction deleting alone, the
combined mix, the ts-primary determinism and the run tiebreak, the
empty-database no-op, `backup_records` SET NULL, journal references
surviving, idempotency, the explicit-rollback transactionality, and
the `StorageError::Sql` error shape. The app side
(`retention.rs`' test module) pins the seam over a real `Db` in a
temp data directory: stored settings govern the prune over the
defaults (1001 entries + a stored limit of 1000 → 1000), a stored
`log.max_days` governs the age criterion, `read_limits` returns the
§6.5 defaults on a fresh database, the fresh first launch is a clean
no-op, a corrupt settings cell fails with the fallback shape
carrying a class-only diagnostic, the fallback builder's wire shape,
and the ticker's lifecycle (a bounded-loop seam: the first pass
fires with no external trigger; a failed tick does not end the
loop — the second tick of the same run prunes after the fault is
repaired). `logging.rs` gains one test for the two facade lines
(counts on the `info`, code + reason on the `warn`).

The round-1 review (deepseek-v4.1-flash, APPROVE, no P0/P1; all
findings fixed in the same commit) tightened five edges: (1) the
`LogLimits` doc claimed `max_entries <= 0` "keeps everything" — true
only for negative values (SQLite treats a negative `LIMIT` as
unlimited); `0` empties the keep-set and deletes the whole journal,
and the doc now says exactly that asymmetry; (2) the fallback's
`os_message` used the storage error's own `Display`, which for some
`SettingsError` variants embeds the stored cell's text — the T-027
`corrupt_cell_diagnostic` rule now applies here too: class and key
travel, stored text does not (pinned: the planted cell's bytes reach
no log surface); (3) the ticker itself had no behavioral coverage —
the loop is now a bounded-tick seam (`cleanup_loop`) with two
lifecycle tests; (4) the app seam pinned only `log.max_entries` — a
`log.max_days` seam test and a `read_limits`-defaults test close the
copy-paste hole; (5) the wiring pin's failure message, the §6.2→§6.4
mis-cite, the LOG-007 runs-cap boundary and the "commands keep
working" queueing wording (the boundaries recorded above and in
ARCHITECTURE.md). The review also confirmed the truncating `%.3f`
cutoff is exact at the system's own millisecond resolution (both
sides of the comparison truncate — `now_utc` stamps cells the same
way), now documented on `age_cutoff`.

The round-2 review (hy4-flash-free, APPROVE, no P0/P1) caught three
stale docstrings still describing the fallback `os_message` as
"the storage error's own text" — the class-only rewrite of round-1
fix 2 had missed the `run_once` docs, the `db_cleanup_failed` docs
and one D-054 sentence; all three now say class-only, so no future
reader "restores" the `Display`-based message and the SEC-008 leak
with it. Its P3 widened the failed-tick test's timing margin (the
tick interval is 100 ms now, the repair's ~95 ms margin and the
weaken-not-flake boundary recorded in the test).

Dependencies: `tokio = { version = "1", default-features = false,
features = ["time"] }` joins src-tauri (already resolved
workspace-wide via tauri — an edge, not a new version; deny stays
green with no new ignores). Everything else is covered by
`rusqlite`, `jiff` and the existing crates.

### D-055 — `settings_get`/`settings_set`: string-shaped wire, partial atomic save, per-key journal entries in the operation's transaction — 2026-11-18

Problem: T-030 turns the §7.10 Settings module (`settings_get`,
`settings_set`) into commands over the D-039 repository. Open
questions: how the thirteen keys and their seven value shapes reach
TypeScript without duplicating the store's types (the store has no
specta dependency, D-017); whether `settings_set` updates one key or
a set per call (§7.10 names no argument shape); what the
`settings.changed` journal entry carries (LOG-001…004) when `Details`
is closed at three variants (D-051) and the store crate is frozen in
this task's window (T-029 owns it); and how the DEF-001…005
acceptance criteria ("applies without a restart", "default.* changes
no existing project") are proven when the scanner and the project
commands do not exist yet.

Solution, five decisions:

- _Wire DTO: dotted keys + canonical JSON text, no type duplication._
  Keys travel as their §6.5 dotted strings, parsed at the boundary
  through `SettingKey::from_key`; values travel as canonical JSON
  **text**, parsed at the boundary and validated through
  `SettingValue::from_json` — D-039's single validation path, so the
  store stays the one owner of the vocabulary. Mirroring the keys or
  the value enum in a src-tauri specta type was rejected for the
  D-052 reason (a second list to keep in sync for no behavioral
  gain), and `serde_json::Value` on the wire stays impossible by the
  same entry's recorded choice (`specta/serde_json` deliberately
  disabled — the `details`-as-text precedent). The response is a
  **list in §6.5 table order** (the store's `get_all` order), not a
  JSON object: an object would impose sorted or arbitrary key order
  on the Settings window's sections.
- _Granularity: one call saves the screen's whole dirty set._ The
  changes list carries only the changed keys; everything applies in
  one `Db::run` transaction (§6.9: "the whole operation runs in one
  transaction" — the settings-save shape D-039's module docs
  anticipated). One key per call was rejected: the screen's Save
  would be N transactions, and a failure mid-save would leave the
  screen half applied. Validation is validate-all-first (unknown key,
  duplicate key, non-JSON value text, every §6.5
  type/range/member/shape violation → `E_VALIDATION` with
  key/value/reason params, the T-027 field/value/reason shape with
  the setting key as the failed field), then the store's own `set`
  re-validation as the defense-in-depth twin. An **empty list is
  rejected**, not treated as a no-op: an empty save that "succeeds"
  would hide a frontend bug that computes the dirty set as
  always-empty (the loud-choice policy). A valid list is bounded by
  the vocabulary itself — at most thirteen distinct keys, the 14th is
  unknown and any repeat is a duplicate — so the write loop cannot be
  made unbounded. The response is the fresh full map read in the same
  transaction: the saving screen gets the applied state without a
  second round trip. A resent unchanged value is **not** filtered out
  (round-1 review question, recorded): comparing against the current
  row would need a raw-row seam the store does not expose (`get`
  applies defaults, and treating "no row + value equals the default"
  as a no-op would unpin the user's explicit choice against a future
  default change), and the caller sends the dirty set by contract — a
  saved key that changed nothing is a caller anomaly the journal
  honestly shows as the save operation it was. `Db::run`'s busy-retry
  cannot double-journal: the failed attempt rolls back its entries
  before the replay.
- _Journal: one `settings.changed` entry per changed key, entity id
  the dotted key, no details._ The store's `Entity` type documents
  "the dotted key for a settings entry" as the settings entity id —
  the designed carrier of _which_ setting changed; `Details` is
  closed (Bulk/File/Failure, D-051) and the store is not this task's
  to extend, so no keys-list variant exists; a single `Bulk`-count
  entry would lose the which-keys fact. Per-key entries record
  exactly that, carry no value (keys are identifiers, not content —
  SEC-008's structural line; the count is the entry count, bounded by
  13), and the `insert` runs inside the operation's own transaction
  (D-051): a rolled-back save leaves no entry claiming it happened —
  pinned through the real command by a staged `RAISE(ABORT)` trigger
  that kills the second key's journal insert after the first key's
  set and entry both succeeded (constraint abort → base classifier →
  `E_VALIDATION`; both keys and the entry gone). Each entry also goes
  through `logging::action_logged` — the D-051 "call once per written
  entry" contract's first production caller (a no-op for the
  info+user+ok mirror shape).
- _Corrupt cells: read failures are corruption, write failures are
  input._ `StorageError::Settings` on a read (`settings_get`, and the
  `get_all` tail of `settings_set`) is a stored cell outside its
  contract — the D-052 corrupt-cell rule, applied to the settings
  table: the recorded D-037 fallback with a `SettingsCorrupt`
  diagnostic, empty params, and class-only `os_message`
  (`SettingsError`'s own `Display` embeds the stored value and key,
  so the diagnostic is a fixed per-class text). On the write path the
  same error can only be about the caller's value (`set` validates
  before writing), so it stays `E_VALIDATION` — the store-contract
  twin of the command's own validation. §10 error logging rides two
  new facade functions (`logging::settings_get_failed`/
  `settings_set_failed`, the `activity_list_failed` shape — a minimal
  additive edit to `logging.rs`, outside this task's named zones but
  required by the D-032 facade rule and §10's every-error line;
  flagged to the operator). A successful read logs nothing; a
  successful save's record is its journal entries.
- _Acceptance criteria, honestly adapted (no scanner, no project
  commands yet)._ "Applies without a restart" (DEF-001…004) is proven
  the T-012/D-032 way: `settings_set` → a **new** `settings_get` call
  over the same session's `Db` returns the new value — nothing
  between the boundary and the table caches settings, and the future
  scanner reading through the same repository path inherits the
  guarantee (pinned end to end over IPC). "Changing `default.*` does
  not affect existing projects" (DEF-005/PRJ-012) is pinned at the
  IPC level in D-039's store-suite shape: a real `projects` row
  seeded directly over the executor (no project command exists — the
  recorded boundary) survives a change of all four `default.*` keys
  with its scheme/base name/extra-file columns untouched and the row
  count unchanged; the structural half is that the settings path can
  only address the thirteen §6.5 keys (unknown-key rejection) and
  writes only `settings`/`action_log` rows.

Testing: two unit tests for the mapping seams (the write twin's
`E_VALIDATION`, the five-class corrupt-cell matrix with class-only
diagnostics), ten IPC tests over the mock runtime (§6.5 defaults
in table order as canonical JSON text — hardcoded spec-literal
oracle; the set→get round trip with the per-key journal; both
inclusive bounds of every ranged key; the nine-case rejection
envelope; all-or-nothing on a rejected list; the trigger-staged
rollback proof; the DEF-005 project pin over all six scheme/base
name/extra-file columns; the corrupt-cell fallback for both commands;
the no-database answers; the registration pin), one facade test for
the two §10 failure lines, plus the regenerated `src/bindings.ts`. No
new dependencies.

The round-1 review (deepseek-v4.1-flash, APPROVE, no P0/P1) tightened
six edges, all fixed in the same commit: the DEF-005 pin now asserts
all six project columns (not three of six); the two §10 failure
facades gained their end-to-end line test (the per-facade pattern the
other commands follow); the journal `insert` failure maps through
`action_log_to_app_error`, not the settings fallback — the settings
catch-all's `other.to_string()` would carry a hypothetical `ActionLog`
failure's stored text into `os_message` (SEC-008; unreachable today
because `insert` yields `Sql` only, but the two error families are
not interchangeable); the unreachable fallback's `os_kind` is the
plain family name `"StorageError"` (the `activity_log_to_app_error`
precedent, not a per-callsite suffix); and two doc truths (the
`journal_entries` helper's phantom `limit`; the store-twin envelope's
two-param shape). The resent-unchanged-value question was recorded as
the decision above instead of a code change. The round-2 review
(hy4-flash-free, APPROVE, no P0/P1/P2) re-verified the round-1 fixes
and the zone boundaries, and caught the one stale half of the
`action_logged` consumption: the facade's own "no production caller
yet" doc paragraph and the two dead `#[allow(dead_code)]` attributes
(`action_logged`, `mirror_level`) — removed, the doc now names
`settings_set` as the first producer.

### D-056 — Property tests of the body builder: proptest through the validating constructor, a backward-walking oracle, a pinned seed — 2026-11-18

Problem: T-036 turns spec §15.1's property-test row — for random
trees, the order in the file equals the library order (DoD 5–6: order
and contents), the build is idempotent, a heading with no assigned
instructions is not output — into code on top of T-035's
`body::build_body`. Open questions: how proptest enters the graph
given D-017's core allow-list; how random trees are generated without
ever bypassing `BuildSnapshot::new`; how an order oracle can check the
file order without restating the implementation's own walk; and how
property runs stay stable across CI's three operating systems.

Solution: a `#[cfg(test)] mod property_tests` beside the unit suite in
`crates/eunomia-core/src/body.rs` (the unit tests of T-035 are
untouched), with these decisions:

- _Dependency._ `proptest = "1"` (locked 1.12.0) as an eunomia-core
  dev-dependency: spec §5.3 pins it in the test toolchain, and
  dev-dependencies are outside D-017's runtime allow-list
  (serde/sha2/thiserror untouched) — the same policy D-042 applied to
  its `serde_json` dev-dep. Features kept minimal (`std`, `bit-set`);
  the default `fork`/`timeout` machinery (rusty-fork, tempfile) is
  off — the properties are pure and finish in ~0.1 s. Nine new locked
  packages (proptest, rand 0.10 stack, chacha20, rand_xorshift,
  unarray, core_detect, borsh/borsh-derive as proptest's transitive
  graph), licenses MIT/Apache-2.0, cargo-deny green.

- _Generator._ `raw_tree()` builds raw trees — 1–3 projects, 0–15
  items (headings of level 2–6, instructions with 0–3 variants whose
  texts come from a ten-entry pool of ALG-002 edges: empty,
  whitespace-only, CRLF-only, BOM behind leading blank lines, BOM+CRLF
  plus trailing junk, and plain multi-line bodies), plus one
  assignment choice per (project, instruction) pair. `assemble()`
  maps that onto the domain types and builds the snapshot **only
  through `BuildSnapshot::new`**, remapping the two shapes `new`
  rejects: an active instruction with no variants gains the empty
  variant (DB-010 — that empty variant is itself D-13's case), and a
  choice pointing at a hidden variant-less instruction is dropped (an
  assignment needs a variant to name, DB-012). A generator bug fails
  `assemble`'s `expect` and shows up as a failing property. Heading
  titles get an index suffix (`"<raw> [<index>]"`) and the text pool
  holds no `'#'`, so heading blocks are unique and can hide inside no
  variant text — the absence assertion cannot be fooled.

- _Oracle._ The order property compares against an expectation built
  from the same data by the opposite route: the implementation scans
  forward from each heading to its section's close (first heading of
  level ≤ L) and emits while walking; the oracle walks backward from
  each included instruction, marking its ancestor headings with a
  running minimum level (a heading is an ancestor iff its level is
  strictly below every heading between it and the instruction), then
  emits per item from the raw tree — a different representation, a
  different geometry direction, no shared walk. The two sides share
  only what the spec fixes verbatim: `normalize::variant_text`
  (ALG-002's own unit-tested module, T-034), the `repeat('#', L)` +
  `' '` + `title` block form and the `"\n\n"` separator. The
  comparison is full-body equality, so the test pins order and
  contents together
  (DoD 5–6, OUT-001/LIB-035). The three §15.1 properties are three
  tests; "assigned" in the third carries the §9.2 canon reading
  (assigned AND active, D-053 — a hidden instruction keeps no
  heading), and no properties beyond §15.1 were invented.

- _Determinism._ The module config pins `cases = 256` and
  `RngSeed::Fixed(36)` (the task's number — only its fixedness
  matters), so every run on every OS explores the same trees: a green
  run cannot flake on a slow runner and a red run reproduces byte for
  byte, which is what the 3-OS CI criterion needs from the test
  itself. proptest's standard failure persistence stays on: a failure
  lands in `crates/eunomia-core/proptest-regressions/body.txt` (the
  crate-root sibling directory of `src/`), replays on every later run,
  and would be committed as part of the suite.
  `PROPTEST_CASES`/`PROPTEST_RNG_SEED` override the pins for local
  exploration only.

Honest boundaries, recorded (round-1 review): CI runs
`cargo test --workspace` on Linux only — the three-OS matrix job
builds the app binary and never compiles, let alone runs, the test
code — so the card's "tests pass in CI on 3 OSes" is NOT closed by
this task; cross-OS sameness rests on the pinned ChaCha seed (a
platform-independent algorithm, no time, environment or file-system
dependence in the suite), and the actual execution on the other two
OSes lands with REL-001's fuller matrix. Normalization correctness is
out of this suite's reach by construction: the oracle calls
`normalize::variant_text` itself, so an ALG-002 regression turns both
sides wrong together — that risk is carried by the module's own unit
suite (T-034) and the unit test pinning that `build_body` calls it at
all. The idempotence property, §15.1-verbatim, can only fail on
non-determinism leaking into the pure function (there is none to see:
the per-project map is lookup-only); it is kept for the spec's
wording, not counted as independent DoD 5–6 coverage.

Five behavior mutations verified the suite bites, each failing the
property it targets: section-close `<=` → `<` (order + heading
absence), dropping the active-state check (order + heading absence),
separator `"\n\n"` → `"\n"` (order), skipping normalization (order),
reversed emission (order); idempotence stayed green under every
deterministic mutation — its scope is §15.1's plain wording, nothing
deeper.

### D-057 — `app_info`: compile-time facts, the DataRoot path, and the fresh-database flag observed inside `Db::open` — 2026-11-18

Problem: T-031 turns the §7.10 System-module `app_info` into the About
window's backend (UPD-005/REL-004: version, identifier, author,
license, repository link; the task card adds the §5.5 pair — the data
directory and whether this launch created the database from scratch).
Open questions: the source of every field (several candidates exist
per field: crate macros, `tauri.conf.json`, the runtime config, spec
literals — each needed one owner, or the same fact would live in two
places and drift); whether the command goes through `Db::run` (D-037)
or the `DataRoot` pattern (D-049); and where the launch-time fact
"the database was created from scratch at this run" lives — `Db::open`
knows it, but neither the setup hook nor the store was available to
record it (the setup region of `lib.rs` and `crates/eunomia-store/**`
were frozen in this task's window, T-029).

Solution, four decisions:

- _One source per fact, each pinned against its second declaration._
  `version` is `env!("CARGO_PKG_VERSION")` — the src-tauri crate
  version, which is the app version (`version.workspace = true`; the
  `tauri.conf.json` `version` mirrors it, and a unit test pins the two
  equal, so a bump of either alone fails the suite). `identifier` and
  `repository` are spec literals in `commands.rs` (UPD-005/REL-004
  fix both; no `repository` field exists in Cargo or the tauri config,
  so `CARGO_PKG_REPOSITORY` would be empty); the identifier is pinned
  equal to the `tauri.conf.json` field (D-020's home — the config that
  actually locates the data directory), and reading
  `app.config().identifier` at runtime was rejected: the mock runtime
  of the IPC tests carries its own default config, which would make
  the spec value unpinnable over IPC. `author` is
  `env!("CARGO_PKG_AUTHORS")` (the `src-tauri/Cargo.toml` field D-020
  names; the single-author manifest makes the macro exactly
  `Den Brooks <denis@yepbro.pro>` — the Cargo `Name <email>` form of
  UPD-005's "Den Brooks, denis@yepbro.pro", both facts on the wire).
  `license` is `env!("CARGO_PKG_LICENSE")`: the workspace's
  `MIT OR Apache-2.0`, the SPDX expression for UPD-005/REL-004's
  "dual license: MIT License and Apache License, Version 2.0"
  (the `LICENSE-MIT`/`LICENSE-APACHE` pair in the repo root), carried
  as the machine form the About window renders from. The tests use
  hardcoded spec literals as oracles for identifier/author/license/
  repository; the version alone is asserted against the same macro the
  command reads — deliberately not a literal (the value is defined by
  the build and changes every release; a literal oracle would turn
  each release into a red test), so that assertion pins the plumbing,
  and the crate↔config drift risk is pinned by the unit test instead.
- _Non-Db path, D-049's pattern._ Every field is a compile-time
  constant or a managed-state read: there is no blocking work, so
  nothing goes through `Db::run` and nothing needs `spawn_blocking`
  (SYS-001's executor policy serves DB/file operations; this command
  has neither). `data_dir` is the managed `data_dirs::DataRoot` — the
  same seam the recovery commands read, and the same property: tests
  point it at temp roots instead of the real
  `~/.local/share/pro.yepbro.eunomiasync`. The path renders through
  `display()` (a lossy display string for the About window; the
  OS-level open action belongs to `app_open_data_folder`). The command
  answers in any mode, recovery included — the About window must not
  depend on a healthy database. Its only reachable failure is the
  missing `DataRoot` state (an uninitialized-app bug; the state is
  managed before anything that can fail), answered with the D-037
  fallback shape and a `DataRootMissing` diagnostic like the recovery
  commands, and logged through one new facade line
  (`logging::app_info_failed`, the `activity_list_failed` shape — a
  minimal additive edit to `logging.rs` outside the task's named
  zones, the D-055 precedent, flagged to the operator).
- _The fresh flag lives inside `Db`, observed as the open's first
  statement._ `Db::open` records `created_fresh` from one metadata
  check — the §5.5 database file is missing (NotFound) or 0-byte —
  fresh iff the path is definitively absent or definitively empty
  (any other metadata failure reads as not fresh; the state fails the
  open itself a moment later, so the flag never reaches the wire
  wrongly). The load-bearing boundary of the placement is `open_db`,
  which writes the database header and schema: the gates before it
  leave at most a 0-byte file (D-040), and the predicate still reads
  a 0-byte file as fresh, so the observation's position ahead of the
  gates is belt-and-suspenders against a future inserted step, not a
  present-day hazard (a round-1 review correction of this entry's
  first draft, which wrongly claimed the gate's 0-byte file "would
  erase the fact"). Missing-or-empty is the §5.5 tip's own reading
  ("the app starts with an empty DB and says so": deleted data = no
  previous database), and it matches D-038's bootstrap semantics for
  every state the app itself produces. The flag is a field on `Db`
  (plus a `created_fresh()` accessor) rather than a second managed
  state or a `Db::open` return value: the setup hook and both
  recovery reopens go through `Db::open`, so every path that brings a
  database into existence publishes the fact without orchestration
  changes — and the frozen setup region (T-029) needed no edit at
  all. Consequences that fell out for free, pinned by assertions in
  the recovery IPC tests: after `db_recreate` the reopened handle
  answers `true` (§7.10's "starts with an empty database" is a fresh
  bootstrap), after `db_restore_backup` it answers `false` (the copy
  held a database) unless the copy was 0-byte (D-046's valid empty
  daily copy — the bootstrap then genuinely created the schema).
  Without a managed `Db` (recovery mode) `app_info` answers `false`
  — no fresh database was created, and the recovery screen, not this
  flag, is the state display. This edit touched `db.rs`, which the
  task brief did not list among its zones (it lists commands.rs,
  ipc_tests.rs, bindings.ts, the lib.rs registration region,
  data_dirs.rs) — but the brief itself names "Db::open returns?" as a
  candidate home for the flag, and the change is strictly additive
  (field, accessor, one observation line); neither parallel task
  (T-029: eunomia-store + setup region; T-036: eunomia-core) touches
  `db.rs`, so the window's merge risk is nil. Recorded here so the
  zone deviation is visible.
- _Wire shape._ `AppInfo` carries snake_case fields
  (`version`, `identifier`, `author`, `license`, `repository`,
  `data_dir`, `db_created_fresh`), strings plus the one bool — the
  house convention of `DbBackupInfo`/`AppError`; `db_created_fresh` is
  a plain `bool`, not `Option` (the card names a flag; "no database
  running" is expressed by `false` plus the state the rest of the app
  already displays).

Testing: six IPC tests over the mock runtime (the full DTO against
spec-literal oracles with the version-macro rationale above and
`db_created_fresh = true` over a clean root; the 0-byte-file half of
the predicate — the shape a crashed first launch leaves; the
not-fresh half driven through the same real `Db::open` seam —
bootstrap, close, reopen — asserting only the flag differs; the
no-database answer with `data_dir` still served from `DataRoot` and
the database file provably not created by the call; the
missing-`DataRoot` fallback envelope with its diagnostic; the lib.rs
registration pin), `created_fresh` assertions inside the two existing
recovery round-trip tests (recreate → true, restore → false — the
consequences recorded above), two unit tests pinning the field
sources against `tauri.conf.json` (identifier and version), one
facade line test in `logging.rs` (`app_info_failed`'s code +
diagnostic), and the regenerated `src/bindings.ts`. No new
dependencies (macros and managed state only).

Two requirements deliberately left to the About-window UI task,
recorded so they do not fall between tasks: REL-004's "the 'About'
window shows the list of dependency licenses" (the command serves the
app's own license; the dependency list is a screen of `cargo deny`
output, not an `app_info` field), and the human rendering of the
machine forms — the SPDX expression and the Cargo `Name <email>`
author spelling are what travels on the wire; turning them into
UPD-005's prose ("dual license: MIT License and Apache License,
Version 2.0") is the frontend's localization choice.

The round-1 review (deepseek-v4.1-flash, APPROVE, no P0/P1) tightened
five edges, all fixed in the same commit: the facade's log target is
`eunomia::system` (the §7.10 System-module group, not a new
`eunomia::app`); the 0-byte half of the predicate gained its own IPC
test; the no-database test now proves the command creates no database
file; the two recovery round-trip tests pin the flag consequences;
and the observation-order rationale was corrected (this entry's
fresh-flag paragraph above carries the corrected wording — the first
draft wrongly claimed the too-new gate's 0-byte file would change the
answer).

### D-058 — `running_as_root`: the effective-uid seam behind app_info, the crate's first unsafe block, and a Linux-only target-gated libc — 2026-11-18

Problem: T-032 adds the NFR-023 flag to `app_info` ("Linux: running as
root shows a warning, because the created files will be owned by
root"). Open questions: which uid answers "running as root" (real vs
effective, and the task brief's own `getuid()` suggestion); which
platforms carry the flag at all; how to read the uid when std exports
no such API and the project is unsafe-free (rust-skills EunomiaSync
Notes) with new crates gated behind D-012; and how to test a fact the
suite cannot produce (the test process cannot become root).

Solution, four decisions:

- _Platform scope is the spec's letter: Linux only._ NFR-023 prefixes
  the requirement with "Linux:", so the flag is answered by a
  compile-time pair: on `target_os = "linux"` it is the uid predicate,
  on every other platform it is `false` by a `cfg(not(…))` twin
  definition — a structural fact, not an untested convention. macOS
  could detect root the same way (`geteuid() == 0` works there), but
  the spec assigns no warning outside Linux, and a `true` flag there
  would drive a UI warning the spec does not define. The Windows half
  of NFR-023 (long-path manifest) is a packaging fact, not a runtime
  one — nothing to serve on the wire.
- _The effective uid, not the real one._ The card's detection hint
  said `getuid() == 0`; this task serves `libc::geteuid()` instead,
  because NFR-023's rationale is ownership — files a process creates
  are owned by its effective (fs) uid, and `id -u`, the conventional
  "am I root" check, prints the effective uid. The two readings
  coincide for every launch path a desktop app really has (direct
  launch, `sudo`, `pkexec` all set ruid = euid = 0); they differ only
  for setuid-root binaries, which EunomiaSync is not and cannot become
  accidentally. The deviation from the brief's letter is recorded
  here deliberately.
- _`libc` as a direct, Linux-target-gated dependency — and the
  crate's first `unsafe` block._ std exposes no uid API; the safe
  std-only alternatives are folklore (parsing `/proc/self/status`,
  stat-ing `/proc/self` or a temp file and reading its owner) whose
  semantics are undocumented and side-effectful — rejected as a
  foundation for a correctness flag. Between the wrapper crates,
  `nix`/`uzers` would be genuinely new crates in the graph for one
  call; `libc` 0.2 is already built workspace-wide (gtk/tao pull it on
  Linux), so the direct edge adds no version and no code — the
  rusqlite/jiff precedent of an edge, not a crate. The dependency is
  declared under `[target.'cfg(target_os = "linux")'.dependencies]`,
  so Windows/macOS builds do not even resolve it. This introduces the
  workspace's first `unsafe` block, which the EunomiaSync Notes pin
  down as a DECISIONS-first act: the block is one call,
  `unsafe { libc::geteuid() }`, inside `commands::current_uid` —
  async-signal-safe, no arguments to validate, no failure mode, no
  state held across the call; the wrapper is one function whose
  invariant is trivial by inspection, exactly the minimal shape that
  note demands. A `// SAFETY:` comment states it at the block.
- _The seam: a pure predicate over a raw `u32`, one thin reader, an
  injected uid._ `is_root(uid: u32) -> bool` (pure, platform-
  independent; gated `#[cfg(any(test, target_os = "linux"))]` —
  production uses it on Linux alone, and a non-Linux release build
  drops it rather than carrying dead code) + `current_uid() -> u32`
  (cfg pair: the real `geteuid` read on Linux, an inert `0` off Linux
  where no uid is ever read — the placeholder keeps the command body
  platform-uniform, it is not a sentinel) + `running_as_root(uid)` (the
  cfg pair holding the platform policy: the root predicate on Linux,
  `false` regardless of uid elsewhere) + `app_info_impl(app, uid)` —
  the uid enters the sheet builder as a **parameter the tests can
  drive**. The suite cannot become root, so the acceptance pair
  ("under root true, under a regular user false") is exercised two
  ways: the pure truth table (`is_root(0)`, `is_root(1000)`,
  `is_root(65534)`), and the driven sheet — `app_info_impl(&app, 0)`
  must carry `true`, `app_info_impl(&app, 1000)` must carry `false`
  (direct literals, Linux-gated; a hardcoded flag fails one half in
  either direction, no privilege needed). The IPC equality pin
  (`info.running_as_root == running_as_root(current_uid())`) is
  honestly scoped: it bites under a root run; in an unprivileged run
  both sides read false (the vacuity the round-1 review caught in the
  first draft's no-arg pin — see below), so the falsifiable coverage
  is the driven test, and the IPC test's if/else asserts whichever
  half the environment produces. The under-root half WAS executed
  privileged: `sudo -A <test binary> --exact …` ran the flag's IPC
  test as uid 0 during review and the final proof — it passes with
  the flag true on the wire. The non-Linux half keeps a dormant unit
  test (`#[cfg(not(target_os = "linux"))]`, asserting even uid 0
  answers false): it executes only under a non-Linux test build, which
  today's CI matrix does not run (it builds but does not test the
  Windows/macOS legs) — recorded here as deliberately unpinnable on
  this machine and the Linux CI. A local cross-check was attempted
  and is not feasible: `cargo check --target x86_64-pc-windows-msvc`
  dies in rusqlite's bundled-SQLite build script (`cc` wants MSVC
  `lib.exe`), and the Apple targets need an SDK; the compile-side
  guarantee for the non-Linux branches is the CI build matrix, which
  compiles both legs.

Testing: two unit tests (the truth table through the seam, the
dormant non-Linux pin) and two IPC tests over the mock runtime (the
wire flag with its plumbing equality and the environment's half
asserted by if/else; the driven sheet falsifying both halves without
privilege), plus the regenerated `src/bindings.ts`
(`running_as_root: boolean` with the platform semantics in its docs).
No new failure paths: the uid read cannot fail, so `app_info` keeps
its single reachable failure (missing `DataRoot`) and no logging edge
is added.

The round-1 review (deepseek-v4.1-flash, APPROVE, no P0/P1; it
independently ran the suite, clippy, the bindings export check and —
via `sudo -A` — the flag's tests as uid 0) tightened four edges, all
fixed in the same commit: `is_root` warned `dead_code` on non-Linux
release builds (the plugin-pair precedent: the platform twin never
calls it there) — fixed with the `any(test, linux)` gate; the first
draft's no-arg plumbing pin (`running_as_root() ==
is_root(current_uid())`) was vacuous in an unprivileged run, and its
comments plus this entry overstated what it catches — replaced by the
uid-parameterized `app_info_impl` and the driven-sheet test above,
with the IPC equality honestly scoped and the under-root branch made
a positive assertion; the dormant non-Linux test was accepted as
documented (no change); and "launch-time uid" wording became "the
process's effective uid" — the flag is computed per call, not
recorded at launch.

The round-2 review (hy4-flash-free, APPROVE, no P0/P1/P2) found
nothing to fix; its two P3 notes are the same recorded limits: the
off-Linux test pins stay dormant until CI runs tests on those legs
(the build matrix already compiles the twins), and the one mutation
left to unprivileged CI — the command passing a constant non-root uid
instead of `current_uid()` — is caught only by the privileged run
(the one-line call site was instead verified under root, recorded
above). Adding a privileged or non-Linux test leg to CI is a pipeline
decision left to its owner.

### D-059 — T-240 docs: prettier's single trailing newline beats the owner's verbatim byte — 2026-11-19

Problem: T-240 lands the owner-provided `CLA.md` and the full replacement
`CONTRIBUTING.md` as verbatim copies (the only intended edits: the three
CLA placeholders — effective date, governing-law jurisdiction ×2, contact
email — values fixed by the owner on the card, mid-task amended for the
jurisdiction). The owner's `CONTRIBUTING.md` source ends with a blank
line (`licenses.\n\n`), while prettier gates root markdown through both
`format:check` and the pre-commit hook and normalizes every file to a
single trailing newline. A byte-verbatim copy and green checks cannot
both hold.

Solution: the checks win. The trailing blank line is dropped
(`pnpm exec prettier --write CONTRIBUTING.md`; −1 byte — the only
deviation from the owner's source besides the three placeholder lines;
`CLA.md` passes prettier untouched). Rationale: `format:check` runs in
CI on every push, so a verbatim file would keep the pipeline red; the
dropped byte is semantically empty in Markdown (identical rendering);
and the check is the repo's enforceable contract, while verbatim-ness
has no enforcement and its only witness — this entry — now records the
exact deviation. The placeholder values remain owner legal decisions
recorded on the task card, outside this entry's scope.

### D-060 — CI test matrix on four OS legs behind a stable `test` gate; frontend tests pinned to the Linux leg — 2026-11-19

Problem: REL-001 (P0) and spec §14 require CI on Windows, macOS arm64 and
x64, and Ubuntu, but the `test` job ran `cargo test --workspace` on Linux
only — the boundary D-056 recorded as "lands with REL-001's fuller matrix",
and D-058's dormant non-Linux test pins never executed anywhere. Three
design questions: matrix the existing `test` job or add a separate matrix
job; whether every leg needs the Linux job's pnpm/Node setup; and what a
matrix does to `main`'s branch protection, whose required checks name the
literal context `test` (verified via the protection API: lint, test, deny,
e2e, build × 3 legs).

Solution: a new `test-matrix` job (the card's "new test matrix" option)
runs the four legs `[ubuntu-latest, windows-latest, macos-15,
macos-15-intel]` with `fail-fast: false` and `timeout-minutes: 45`, and the
`test` job id becomes a tiny `if: always()` gate that fails unless
`needs.test-matrix.result == 'success'`. Matrixing `test` itself would have
renamed its check contexts to `test (ubuntu-latest)` & co and orphaned the
required context `test` — the first PR would sit on "Expected — Waiting for
status" forever (round-1 review P0). The gate keeps the documented merge
gate satisfiable with zero settings churn and survives future leg changes;
`build (macos-15-intel)` still has to be added to the required list by hand
(per-leg build contexts cannot be aggregated away without the same rename
problem) — flagged to the operator at merge time. Per-leg dependencies
mirror the build matrix's OS-conditional approach: the apt step runs only
on `runner.os == 'Linux'` (Windows/macOS compile the Tauri stack from
system libraries, exactly as the build legs already do); pnpm/Node/Vitest
steps run only on the Linux leg, gated on `runner.os == 'Linux'` rather
than the label string (a future `ubuntu-22.04` pin must not silently
switch the frontend tests off), because `cargo test` never invokes Node
(debug-profile `generate_context!` serves `devUrl`, not `frontendDist`, so
no `dist/` is needed) and REL-001's frontend stack is platform-independent
— the Linux leg runs exactly today's sequence of steps.
`Swatinem/rust-cache@v2` keys per OS automatically, as in the build matrix.
The build matrix gains `macos-15-intel` for the §14 macOS x64 leg, and
`timeout-minutes: 60` bounds its hung-leg worst case. Runner facts,
verified against actions/runner-images#13045, the runner-images README and
the GitHub runners reference on 2026-11-19: `macos-15-intel` is the macOS
15 x64 image on the standard free public-repo class (4 CPU / 14 GB RAM;
the same image's `-large` alias is the paid larger runner, not this
class); the label is officially available until August 2027, the
successor `macos-26-intel` already provides the standard x64 image, and
that 2025 announcement also signaled x86_64's eventual end — watch
runner-images announcements when planning beyond macos-26 — all encoded
in a workflow comment
together with the queue note (Intel legs queue longer; one run now starts
4 macOS jobs against the free plan's 5 concurrent macOS slots). Spec
honestly notes: REL-001 says "Ubuntu 22.04" while the matrix keeps
`ubuntu-latest` (24.04 today), because GitHub's own 22.04 image is in its
deprecation window — flagged for a future spec revision, not silently
changed here; REL-001's artifact build stays a debug smoke build (no
bundled .dmg/.app yet), and its signing, release draft and
`latest.json` items stay outside CI by design — they belong to the
release/packaging task that owns the updater keys (D-005), and no
workflow job covers them yet. Known
boundary: whether the suite is green on Windows/macOS is provable only by
a real run after the push (CI-monitoring rule); the pinned proptest seed
(D-056) keeps the legs comparable once they run.

### D-061 — Creation-marker formatting in core: decomposed civil fields, no time library, no clock — 2026-11-19

Problem: T-038 formats the ALG-032 marker line
`<!-- Created by EunomiaSync YYYY-MM-DD HH:MM:SS <IANA name> <offset> -->`
in `eunomia-core`. Three decisions the spec leaves implicit: (1) which
types the instant crosses the boundary as — core has no time library
(jiff is `eunomia-store`'s dependency, D-035, and core's allow-list is
serde/thiserror/sha2, D-017), so `jiff::Timestamp` cannot be a
parameter without a DECISIONS-level dependency addition; (2) how the
UTC fallback of D-23/ALG-032 ("Without an IANA name: `UTC+01:00`")
reads — as a replacement of the name-and-offset pair or as an extra
field next to them; (3) the sign of a zero offset.

Solution: `marker::creation_marker_line(time: MarkerTime, iana_zone:
Option<&str>, offset_minutes: i32) -> String` — plain types only, the
allow-list untouched. `MarkerTime` is the six civil fields
(year..second) with pub fields in the crate's domain-type style:
decomposed integers are the least-coupled honest form (no jiff, no
preformatted string whose `YYYY-MM-DD HH:MM:SS` shape core could
neither check nor own), and the zero-padding template then has exactly
one implementation — this module — instead of one per caller. The
clock stays out on purpose (ALG-032 takes the time "once per run, at
the start of the write phase" — the sync pipeline captures it via
jiff above core and hands the fields down); the function is pure and
unit-tested with the spec's example as the oracle. Spec-reading
rulings, pinned by tests: the UTC form replaces the whole
`<IANA name> <offset>` pair (D-23's `UTC+01:00` is UTC+offset as one
field — `… 14:03:22 UTC+01:00 -->`, never `UTC+01:00 +01:00`), and a
zero offset prints `+00:00` (the template's `±hh:mm` always carries
the sign; RFC 3339 reserves `-00:00` for an _unknown_ offset, which a
stamped marker never has). Field domains are asserted with the crate's
broken-invariant panic posture (values arrive from a time library's
civil accessors and are always in range; month lengths stay the
calendar's truth, not the marker's); the offset must be whole minutes
within ±24:00 — no present-day zone exceeds ±14:00, and historical
sub-minute LMT offsets are inexpressible by design, irrelevant for a
marker stamped at run time. Consumers (T-039/T-041): resolve the run
instant once, pass `jiff`'s civil fields + zone name + offset minutes;
when the zone cannot be resolved, pass `None` and the offset still
tells the truth.

### D-062 — Scheme-B marker parser in eunomia-core: padding class, byte-0 BOM, lines()-CR semantics, error precedence, and the E_MARKERS_INVALID boundary — 2026-11-19

Problem: T-040 (`markers::parse`) transcribes spec §7.5/D-19 ("exactly one
pair, each on its own line, spaces at the ends allowed"), but four points the
spec fixes only in prose needed an explicit reading before they could be
coded and tested, plus one boundary against the app error registry.

Solution: `crates/eunomia-core/src/markers.rs` records the readings in its
module docs and this entry pins them:

- "Spaces at the ends" = ASCII space and tab, either side, nothing else
  (editor indentation). Other ASCII whitespace (`\f`, `\v`) and Unicode
  spaces (NBSP …) are content: a line carrying them is not a marker line.
- CRLF files are first-class (D-22 keeps the file's line breaks outside the
  block): lines are split with `str::lines()` semantics — on `\n`, with a
  `\r` directly before it belonging to the terminator; a lone `\r` anywhere
  else (including the last line of a file without a final `\n`) is content.
- A BOM at byte 0 is an encoding artifact, not line 1's content (the only
  place it can precede the begin marker; D-22 lets scheme-B files keep it).
  A BOM anywhere else is content and blocks marker recognition on its line.
  All span offsets stay absolute, so the BOM survives the ALG-031 splice.
- A marker written inside a longer line — before text, after text, or
  both markers on one line — is _not_ a marker line ("each on its own
  line"): the file is judged on its marker lines only, so a valid pair
  plus an inline mention (prose quoting the markers) stays valid; the
  mention is content.
- Failure classification precedence when several rules fail at once:
  duplicates outrank a missing half, which outranks reversed order — the
  `DuplicateMarkers` variant carries begin/end counts, so no fact is lost.
- Boundary: `E_MARKERS_INVALID` lives in the §7.10 `AppError` registry
  (src-tauri `errors.rs`, D-034). Core's `MarkersError` (one variant per
  §7.5 failure) deliberately does not know it; the sync pipeline (§9.4
  phase 1 step 4, which also owns the upstream `E_NOT_UTF8` check over the
  raw bytes) does the mapping in a later task. DB-016's reserved-marker-line
  validation reuses `classify_line` without dragging the registry in.

The parser returns `MarkerSpan { begin_line, end_line, managed_start,
managed_end }` — 1-based line numbers for diagnostics, absolute byte offsets
as the ALG-031 splice points (T-041) and the ALG-033 hash base. Known
open item: lib.rs's `//!` module list and ARCHITECTURE.md's `eunomia-core`
module enumeration both intentionally did not gain a `markers` entry (the
T-040 brief capped the lib.rs change at exactly one `pub mod markers;` line,
to keep three parallel tasks out of one doc block; T-035 updated
ARCHITECTURE.md for `body.rs` in its own commit, but that file was outside
this task's zone); reconcile both lists when those files are next free.

### D-063 — Scheme-A extra-file reading lines: `extra_files::extra_file_lines`, position-split output — 2026-11-19

Problem: T-037 needed the scheme-A "read this file" lines (spec §7.5,
D-01, D-21, OUT-A-001…004) as a pure core function over the T-033
domain types. Three design questions: the output shape for the
by-scheme builder (T-039, ALG-030 splices the body BETWEEN the start
and end lines, so one interleaved `Vec<String>` would force the builder
to re-split by position); whether the function should consult
`ProjectWriteSettings::scheme`; and how to keep the two same-type
`ExtraFile` arguments from being swapped silently.

Solution: `extra_files::extra_file_lines(base_name, extra_project,
extra_local) -> ExtraFileLines { start: Vec<String>, end: Vec<String> }`
— the split the consumer needs is the type itself, each list already in
the §7.5 order (`.project.md` before `.local.md` when both share a
position; implemented by visiting the project pair first). The line is
the D-21 sentence verbatim with `<file>` bound to
`<STEM>.project.md` / `<STEM>.local.md`. A disabled option contributes
nothing; its position is ignored while `enabled` is false, mirroring
`project::ExtraFile`. The function is deliberately scheme-blind:
ALG-030 marks the reading lines "(A only)", and the by-scheme builder
owns that gate — passing `&ProjectWriteSettings` would have dragged the
scheme question into a pure shaping step. The swap risk is covered by
tests, not types: the arguments are ordered project-then-local to match
the §7.5 tie order (the reverse of `ProjectWriteSettings`' field
order), and the oracle tests use four distinct hardcoded D-21 literals,
so an internal swap of the two pairs fails them; a caller that swaps
the two arguments still compiles — T-039's own tests own that check.
`ExtraFileLines` derives
no serde: it is a build artifact, not domain vocabulary, and D-017's
dependency surface is untouched. Testing: the card's full acceptance
matrix is exhaustively enumerated — enabled/disabled ×2 and start/end
×2 for both stems, 16 combinations per stem (32 cases) — against the
hardcoded literals, plus scenario tests for the tie, the split and the
disabled rules, plus doc tests.

Boundary: ALG-030's `join(parts, BLANK_LINE)`, the creation marker and
the scheme gate stay with the by-scheme builder (T-039); the §7.5 hint
about `.local.md` and Git and the scheme-C snippet (same D-21 sentence,
§7.5) are UI concerns.

### D-064 — Project build hash: DB-020's fields in spec order, length-framed; the marker exclusion is structural; hand-rolled lowercase hex — 2026-11-19

Problem: T-042 turns DB-020 (`sha256('v1' + scheme + target path +
extra-file flags and positions + build text without the creation
marker)`) and ALG-033 ("computed on `managed` without the marker") into
`eunomia-core` code. The spec fixes the field list and its order but
leaves open: the separators — a `+` chain over two arbitrary-length
fields (path, text) and four fixed-vocabulary tokens is not injective:
a path ending `…falseend` with a managed text starting
`trueendtrueend…` re-splits into a different valid input (a concrete
pair is pinned by test); the path encoding and normalization; the
extra-file field order and spelling; whether the extras count under
schemes B and C (ALG-030 adds reading lines under A only); and the
output format (`last_synced_hash` is bare TEXT).

Solution: `hash::build_hash(scheme, target_path, extra_local,
extra_project, managed_without_marker) -> String` in the new `hash`
module — a pure function, no clock, and the first user of `sha2`,
which D-017 pinned in the allow-list ahead of exactly this task. Five
decisions inside it:

1. _Framing._ Every field is fed as an 8-byte little-endian `u64` byte
   length followed by the bytes, in DB-020's field order, behind the
   literal `v1` tag (a constant prefix needs no framing and versions
   the formula — a future `v2` tag cannot collide with `v1` hashes).
   The framing makes input → byte stream injective, so distinct inputs
   cannot share a hash short of breaking SHA-256 itself; plain
   concatenation was rejected on the collision pair above (the test
   asserts the pair's naive `+` streams are byte-identical, then that
   their hashes differ).
2. _Path._ The platform's own bytes (`as_os_str().as_encoded_bytes()`),
   no normalization: the snapshot's project root is canonical (DB-030)
   and the target path derives from it, so one project yields the same
   bytes on every run of the same platform — cross-platform spelling
   equality is not this hash's job. The base name reaches the hash
   through the target path only (`<root>/<base_name>` /
   `<root>/<STEM>.project.md`), exactly the formula's field list; that
   is how DB-023's "changing the file name makes the project dirty" is
   satisfied (pinned with a base-name-change path test).
3. _Extras._ Both extra files in the §6.2 column order (`extra_local`,
   then `extra_project`), each as its enabled flag (`"true"`/`"false"`,
   the §6.5 JSON spelling) followed by its position
   (`"start"`/`"end"`), included under EVERY scheme: the formula has no
   scheme qualifier, and the hash fingerprints the project's build
   configuration, not just the bytes one scheme writes (the same
   reading that makes the scheme itself an input, DB-023); a scheme-B
   test pins the case where the built file ignores the extras yet the
   hash still moves.
4. _Marker exclusion is structural._ The signature has no time, zone
   or marker parameter — ALG-032's run time physically cannot reach
   the hash. Today the markerless managed text is `build_body`'s
   output (the §9.2 body never contains the marker, D-053); when the
   ALG-030 assembly joins the extra-file reading lines around the
   body, its `managed` string (still without the marker) is what gets
   passed here — the seam is "the assembled managed, marker excluded".
   Proof test: full contents in the ALG-030 shape (managed, an empty
   line, the marker, one line break) formatted through the real
   `creation_marker_line` with four marker variants (another time,
   another date, the D-23 UTC fallback, a western zone) differ as
   files, reduce to one and the same markerless remainder — the
   managed text plus the empty line's break; the drop rule removes
   the marker line alone, and the test pins those remaining bytes
   exactly — and hash identically.
5. _Output._ Sixty-four lowercase hex characters, hand-rolled (sixteen
   lines): a `hex` crate would leave D-017's allow-list, and the
   spelling is shared by `projects.last_synced_hash` (DB-020) and the
   BKP-010 backup SHA-256 comparisons.

Testing: nine unit tests plus the doctest — an oracle vector hardcoded
from an independent Python-hashlib computation over the exact framed
stream (one value pinning the tag, the field order, the framing and
every field literal at once), the 64-lowercase-hex format, stability,
the per-input change matrix (schemes A/B/C; root, base-name and
scheme-C path variants; each of the four extra fields alone; managed
text including the empty build and a trailing-newline difference), the
marker-time non-input proof, and the framing collision pair. Five
behavior mutations verified the suite bites — dropping the `v1` tag,
dropping the path field, swapping the extras' order, uppercasing the
hex alphabet, and removing the length framing each fail the suite,
with different watches: the unframed and uppercase-hex mutations fail
their dedicated tests, the extras-order swap fails the oracle and the
extras change matrix, and the dropped `v1` tag fails the oracle alone
— the change-matrix tests mutate both sides of their comparisons
identically, so only the value that pins the exact byte stream catches
what moves every input's hash the same way.

Boundaries: DB-021's dirty computation and DB-024's `last_synced_hash`
updates belong to the dirty-flag task; ALG-033's existing-file scanner
(drop the last marker line, byte-compare the rest) belongs to the
comparison task — the test-local strip mirror of the drop rule is a
proof device, not a production seam. No new dependencies (`sha2`
activated from D-017's list; `serde`/`thiserror` untouched). One
documented lag, deliberate: the "sha2 stays idle until the build-hash
task" comments in `lib.rs`/`Cargo.toml` and the lib.rs module index
now trail the landed module (as they already trail the parallel
tasks' `marker`/`markers`/`extra_files`) — T-042's zone constraint
allows exactly one line in lib.rs while neighboring core tasks run,
so the prose reconciliation belongs to the orchestrator's docs pass
over all four modules at once.

### D-065 — Whole-file assembly for schemes A and C: `target_file::build_content`, the scheme gate and the marker-only file — 2026-11-19

Problem: T-039 turns ALG-030 into code — the whole target-file content
of the full-overwrite schemes (A and C). Four decisions the spec prose
leaves to the reader: (1) what "parts" are in
`managed = join(parts, BLANK_LINE)` — is each reading line its own
part, or is a position's line group one part; (2) whether the
empty-managed sentence "the file has only the marker" keeps the final
line break; (3) where the ALG-030 "(A only)" gate on the reading lines
lives; (4) how scheme B relates to a function named "file content".

Solution: `target_file::build_content(scheme, body: &str,
extra_lines: &ExtraFileLines, marker_line: &str) -> String` — one
public function in a new module `target_file` (the §2 glossary's own
term for "the file that the app writes in the project"; the first
draft's `assembly` was rejected in review — a near-homograph of the
pre-existing `assignment` module that would blur imports, greps and
IDE completion in public API; `file` would suggest I/O in a crate
that has none, `file_body` collides with §9.2's `body`). The private
`managed_block` helper owns the join and the gate behind an
exhaustive `match` on `Scheme`, so a scheme variant added later is a
compile error here, not a silent fall-in with C.

- _Parts are position groups._ ALG-030 enumerates exactly three parts
  — the start lines, the body, the end lines. A position's lines form
  ONE part of adjacent lines (`.project.md` before `.local.md`, §7.5),
  and only neighbouring parts are separated by one empty line; two
  start lines are two lines of one block, never two blocks. The
  D-063 split shape (`ExtraFileLines { start, end }`) maps onto this
  directly, as D-063 intended.
- _Empty managed keeps the final line break._ "The file has only the
  marker" = the marker line WITH its terminating `\n`, no managed
  block, no leading empty line. Reading: §2 and ALG-033 treat the
  marker as a line ("the last non-empty line that starts with …"), a
  line in a D-22 LF file ends in `\n`, and every other file the app
  writes ends with exactly one line break — the sentence overrides
  which lines precede the marker, not the marker's own terminator.
  Pinned by an oracle test; both A and C produce it identically.
- _The scheme gate is here._ `extra_file_lines` stayed scheme-blind
  (D-063) precisely so this builder could own the choice: scheme A
  splices the lines, scheme C drops them even when every option is
  enabled (the card's criterion; D-20 — the app does not edit a
  scheme-C main file, and the §7.5 snippet is settings-UI text, not
  file content). With no reading lines A and C build the byte-identical
  file; the schemes differ in the target name only, which is the
  caller's concern.
- _Scheme B panics._ ALG-031 splices the managed block into an
  existing marked file — a different builder with different inputs
  (the existing bytes, the D-19 markers, the line-break conversion),
  so there is no whole scheme-B file to build; a B here is a caller
  bug in the crate's broken-invariant panic posture (D-061). The
  managed-block join lives in the private `managed_block` helper;
  the scheme-B task can promote it or reimplement it per ALG-031's
  own tail rules.

The marker arrives as a parameter, never formatted here (ALG-032's
once-per-run capture is the sync pipeline's clock; D-061), asserted
non-empty and single-line; the body is asserted free of framing line
breaks (a leading or trailing `\n` would turn ALG-030's one empty
line into two) — the two byte-level contracts the file layout depends
on. Encoding mechanics per D-22: every separator added is `\n`, no
BOM is ever prepended (a Rust String is UTF-8), and no input text is
normalized — the body arrived ALG-002-normalized from `build_body`,
the reading lines are D-21 sentences; an empty body part simply does
not exist (a whitespace-only body would pass through verbatim, but
`build_body` cannot produce one).

Note for the ALG-033 task (found in review): this module pins the
non-empty layout to `<managed>\n\n<marker>\n`. ALG-033 drops "the
last non-empty line" — the marker line — from the existing file and
compares "the rest" with the new content byte for byte; dropping only
the marker text leaves `<managed>\n\n` (or `<managed>\n`), which
never equals `<managed>`, so the D-24 comparison must either drop the
separator empty line together with the marker line or drop the marker
from the new content the same way before comparing. Whatever it
chooses, the scanner must expect this module's exact tail.

No new dependencies (D-017 untouched). Testing: whole-file oracles
hardcoded from the spec's own D-21/ALG-032 literals — the full ALG-030
composition (start/body/end), the adjacent-lines grouping at both
positions, the empty-body splices, the marker-only file (including
scheme C with every option enabled and an empty body — where the gate
and the empty branch meet), the C gate fed through the real
`extra_file_lines` with everything enabled, the A≡C identity without
lines, the position matrix, the exactly-one final line break, BOM
absence, verbatim body splicing, and the five panic postures (B,
empty marker, marker with `\n`, marker with `\r`, body with a framing
break) — plus doc tests.

### D-067 — The ALG-033 existing-file comparison: byte-exact drop of the last marker line's full span — 2026-11-19

Task T-043 (depends on T-038's marker format). Problem: ALG-033/D-24
say "in the existing file, the last non-empty line that starts with
`<!-- Created by EunomiaSync ` and ends with ` -->` is dropped; the
rest is compared with the new content byte for byte", and leaves four
readings open when turning it into `compare::contents_match` in
`eunomia-core`.

Solution — the four readings, fixed:

1. _Which line._ A content match over every line, position-independent:
   the scan keeps the LAST line anywhere in the file whose content
   carries the exact prefix and suffix — the same reading the merged
   D-064 test mirror already lives by, and the one that behaves right
   on both realistic shapes (a stray marker copy inside the managed
   text loses to the real trailing one; a user edit below the marker
   stays in the rest and differs). The spec's "non-empty" holds by
   construction (the prefix is non-empty); the timestamp between
   prefix and suffix is NOT re-validated against ALG-032 — recognition
   is the two strings only.
2. _What the drop removes._ The line's span: content bytes plus its
   terminator (`\r\n` or `\n`; the bare content when the final line is
   unterminated). This is deliberately byte-exact where the D-064
   test-local `lines().remove().join()` mirror is lossy (a proof
   device, as D-064 records): the mirror eats one adjacent break,
   which a production seam must not guess at. The caller's contract
   mirrors it: `new_without_marker_line` is the new build with its own
   marker line's span removed — for the ALG-030 shape (managed, an
   empty line, the marker, one line break) that is `managed + "\n\n"`,
   and the empty string when managed is empty. The new side is not
   scanned: the build knows where its marker line is.
3. _The terminator's `\r`._ One `\r` before a `\n` belongs to the
   terminator, not the content — `str::lines()` semantics, on bytes.
   Needed because ALG-031 lets the app itself write CRLF marker lines
   into CRLF scheme-B files; without the absorption a second sync of
   such a file could never be `unchanged`, against D-24's acceptance.
   A `\r` anywhere else is an ordinary byte and breaks the suffix.
4. _No marker, nothing dropped._ The whole existing bytes are then
   compared as is; a file from which the marker line was removed by
   hand matches exactly when what is left is already the markerless
   build.

Input types: `existing: &[u8]` — the comparison is byte for byte and
only scheme B promises UTF-8 (§9.4 checks `E_NOT_UTF8` there), so the
A/C reader may hand over raw disk bytes; a file whose compared rest
holds invalid bytes never matches (they stay in the rest while the
build is UTF-8; invalid bytes inside the dropped marker line go with
the line, and only the prefix and the suffix decide), which the
`&[u8]` parameter absorbs without caller-side logic.
`new_without_marker_line: &str` — the build is text. Pure, allocation-
free: the rest is compared as prefix-plus-suffix slices of the new
bytes (length, then `starts_with`, then `ends_with`). One published
precondition (review R1, P2): `existing` is the bytes of a file that
exists — a missing target is OUT-005's write case, never a
comparison, because an empty-managed build's markerless form is the
empty string and empty bytes standing in for a missing file would
read `unchanged` and the file would never be created.

Testing: sixteen unit tests plus the doctest, markers formatted by the
real `creation_marker_line`. The criterion's match case: files whose
only difference is the marker (another time, another date and zone,
the D-23 UTC fallback, a negative offset) all match one markerless
build; the scheme-B placement — the creation marker between the
managed block and the END line, not at the file's end, LF and CRLF —
matches its markerless twin through the real BEGIN/END constants. The difference cases: a body edit (down to one trailing byte),
an extra line before the managed text and a blank line after the
marker, a BOM outside the marker, CRLF breaks against an LF build,
near-miss marker lines (double space in the phrase, leading indent,
trailing space, the prefix's trailing space and the suffix's leading
space removed), content added after the marker, non-UTF-8 bytes
outside the marker, a missing marker with and without the separator
breaks, an empty existing file, and a marker-only file (with and
without its final break). Four behavior mutations verified the suite
bites: the prefix without its trailing space fails the damaged-marker
test, first-match-instead-of-last fails the last-marker test, no `\r`
absorption fails the CRLF-recognition test, and a span without the
terminator fails five match tests at once.

Boundaries: the return value is the bare D-24 answer; SYS-008's
externally-modified state, the DB-021 dirty flag and any journal
detail are later consumers (T-044) and are not shaped here. Reading
the file and assembling the build (ALG-030/031) are other tasks'
seams. No new dependencies (D-017's list untouched; pure code). The
lib.rs module-index prose does not yet list `compare` — the same
deliberate lag D-064 recorded: T-043's zone constraint allows exactly
one `pub mod` line while neighboring core tasks run, and the prose
reconciliation belongs to the orchestrator's docs pass.

### D-068 — The dirty predicate: an empty stored hash never reaches the difference comparison; the no-assignments exception is absolute — 2026-11-19

Problem: T-044 turns DB-021 into a pure eunomia-core predicate. The
rule names three facts-compound clauses (archived never dirty per
DB-014; active ∧ [empty `last_synced_hash` while the build text is
not empty ∨ hash differs]; no-assignments ∧ never-synced not dirty),
but leaves their intersections open. The load-bearing one: what "the
hash differs from the current one" means when `last_synced_hash` IS
empty. Read as a bare inequality, "" ≠ the current hash holds for
EVERY project — `build_hash` always returns sixty-four hex characters
(D-064) — so the second disjunct alone would mark every never-synced
active project dirty, the first disjunct ("empty while the build text
is not empty") would be fully redundant, and a project whose
assignments all point at hidden instructions (DB-013: real
assignments, empty build) would be dirty with nothing to write. The
exception sentence would then exist only to rescue the narrower
no-assignments case — an asymmetry between two project shapes that
both build nothing.

Solution: module `dirty`
(`crates/eunomia-core/src/dirty.rs`), `is_dirty(state,
last_synced_hash, current_hash, build_text, has_assignments) -> bool`
with `ProjectState { Active, Archived }` as the module's own input
enum (the snapshot carries no state — D-042 holds active projects
only and keeps housekeeping columns out — while DB-014 still has to
be encoded; the schema's two CHECK literals, no serde until a wire
consumer exists, per the crate's consumed-vocabulary practice). Two
interpretations, both pinned by tests that fail under the rejected
reading:

1. _The empty stored hash never reaches the comparison._ "Differs"
   presupposes a stored hash; with none stored (`None` or `Some("")`
   — the §6.2 column is nullable, DB-024 fills it only after a
   `written`/`unchanged` status, and the only writer, `build_hash`,
   never stores an empty string, D-064 — so both spellings mean
   "never synced"), the first clause alone decides: dirty iff the
   build text is non-empty. This keeps the first clause meaningful,
   makes never-synced empty builds clean with or without assignments
   (the symmetric outcome), and matches how OUT-013 frames the flag —
   unsynced changes that affect a build, not a vacant comparison.
2. _The exception is absolute._ DB-021's second sentence ("is not
   counted as dirty") is phrased exactly like DB-014's archived rule,
   so it overrides the first clause too: a never-synced project with
   no assignments stays clean EVEN when the build text is non-empty —
   reachable today only through extra-file reading lines, which
   ALG-030 joins without a single assignment. With a stored hash
   present the exception stops applying ("has never been synced"
   fails) and the comparison alone decides — including dirty for a
   synced project whose assignments were later removed or all went
   hidden: the emptied build moved the hash, and the sync pipeline
   has a D-25 empty write to confirm.

DB-023 needs no rule of its own here: the scheme and the target path
are `build_hash` inputs (D-064), so either change moves the current
hash and the difference clause fires — pinned by a composition test
through the real hash function. The inputs stay facts, not handles:
state and stored hash come from the store row, the build text and its
hash from the build pipeline (DB-022's recalculation is the future
caller), so the predicate keeps the crate's no-IO, no-clock posture.

Testing: the full oracle matrix — state × stored hash (`None`,
`Some("")`, equal, differing) × build text (non-empty, empty) ×
assignments — each row's verdict fixed by the clause cited for its
group (with inline notes where a row needs its own),
plus four focused tests (the guarded comparison, the absolute
exception, the DB-023 composition, and the OUT-013
edit→sync→edit lifecycle over real hashes) and the doc example. Three
behavior mutations verified the suite bites: dropping the exception
(two tests fail), the literal empty-hash inequality (two fail), and
dropping the archived gate (one fails).

Card-vs-spec: no divergence — the card's wording is a faithful
condensation of DB-021/DB-014/DB-023; the intersections it asks
about are exactly the ones resolved above.

### D-069 — Scheme-B file assembly in eunomia-core: ALG-030's layout inside the splice, dominant line breaks counted over the preserved outside, String output — 2026-11-19

Problem: T-041 turns ALG-031 ("the `managed` block and the marker (by
the same rules) are inserted between the marker lines. The marker lines
and everything outside them stay unchanged. The block's line breaks are
converted to the ones most used in the file (LF on a tie)") into code on
top of `markers::parse` (D-062). Five points the spec fixes only in
prose needed an explicit reading: the inserted block's exact layout, where
scheme B's creation marker lives, what "most used in the file" counts,
whether the result can be a `String`, and the boundary with the parallel
A/C builder (T-039).

Solution: `crates/eunomia-core/src/assembly_b.rs` — `assemble(content,
managed, marker_line) -> Result<String, MarkersError>` (parses through
T-040 and proxies its error for the pipeline's `E_MARKERS_INVALID`
mapping) plus the span-taking `splice(content, &MarkerSpan, managed,
marker_line) -> String` for a caller that already parsed. The readings:

- The splice is exactly D-062's
  `content[..managed_start] + block + content[managed_end..]`, so the
  outside — marker lines with their padding, a byte-0 BOM, mixed outside
  line breaks, a tail without a final break — is copied, never
  recomputed (T-F12's byte-for-byte criterion, §7.5, D-22).
- The block's layout is ALG-030's, carried by ALG-031's "by the same
  rules": `managed⏎⏎marker⏎` when the body has lines, `marker⏎` when it
  is empty ("the file has only the marker"). OUT-016…018/D-23 fix the
  marker's place: end of the managed block, before the closing marker —
  scheme B has no other marker location. The trailing `⏎` is D-062's
  precondition (the end marker must stay on its own line); every output
  re-parses with the begin marker unmoved, pinned by tests.
- "Most used in the file" counts the file's **preserved** breaks —
  everything outside the marker lines, the marker lines' own
  terminators included — and excludes the region being replaced: each
  `\r\n` is one CRLF, each unpaired `\n` one LF, a lone `\r` is
  content (D-062's `str::lines` semantics); CRLF only on a strict
  majority, ties and break-less outsides take LF. The exclusion is
  what the R1 review forced: whole-file counting is not a fixed point
  — a stale hand-made block's style can outvote the surviving file,
  flip the chosen style between two syncs of the same build, and make
  the second sync rewrite an unchanged file (violating D-24's
  "unchanged", §15.2 T-F9; reproduced on the real functions during
  review). Counting the preserved outside makes the choice provably
  stable: the outside never changes, so neither does the style. The
  managed body's own breaks are converted by a lines()-split +
  dominant-join, so one trailing break is absorbed, not doubled
  (`build_body` emits none), and further trailing blank lines are
  kept as lines.
- The result is a `String`, losslessly: every splice piece is UTF-8 text
  and the BOM is the character `\u{feff}`, which `String` carries
  unchanged; writing the string's bytes reproduces the outside exactly.
  The raw-bytes `E_NOT_UTF8` check stays upstream in the pipeline
  (D-062's boundary), as does atomic writing (the file layer).
- Scheme B has no extra-file reading lines (§7.5 marks them "(A only)";
  OUT-B names none): the block is the `build_body` body plus the marker
  line handed in verbatim from `creation_marker_line` — no clock, no
  formatting, and no overlap with T-039's A/C builder. DB-016's
  reserved-marker-line policing is likewise not here: a managed body
  carrying a marker line would make the written file grow a second pair
  (next sync: T-F4 skip); rejecting that text is the save-time
  `E_CONTENT_RESERVED` check's job.

Tests: T-F12 verbatim with a fully hardcoded oracle (BOM + CRLF, outside
byte for byte, output re-parses with the end marker on its computed
line), CRLF/LF majority and a genuine outside tie that the old block's
own CRLF cannot tip (the markers' own terminators count — a lesson this
entry's author relearned three times), first insertion into an empty
region, whole replacement, empty body, blank-line body, trailing-break
tolerance, all four parser verdicts proxied, padded markers + BOM
outside, END without a final break, mixed outside breaks untouched,
second-sync idempotence plus the R1 regression (a stale LF block in a
CRLF file chooses CRLF outright, first write already stable), and the
output's managed region reading back as exactly the block (ALG-033's
base). The marker-line single-line contract is a release `assert!`
(matching `marker.rs`'s posture for its inputs' domains), not a
debug-only check. Known open item, as in D-062: lib.rs's `//!` module
list and ARCHITECTURE.md still gain no `assembly_b` entry — the T-041
brief capped the lib.rs change at one `pub mod assembly_b;` line while
three tasks share that file.

### D-070 — The ALG-060 heading branch range as an index-range function — 2026-11-19

Problem: T-045 turns ALG-060 ("the branch range of a level-L heading
goes from the heading itself to the item before the next heading of
level L or higher, hidden instructions included") into eunomia-core
code — the foundation T-046 (ALG-061 move) and T-047 (ALG-062 level
shift) build on. Open questions: what the function returns (a copied
item list vs. positions); how "level L or higher" maps to the schema
integers (H2 is the highest level, so "higher" is a SMALLER number);
how the §2 term "heading section" (items after the heading) relates to
ALG-060's branch (heading included); and how the module coexists with
`body.rs`'s private `section_end` (D-053), which walks the same
geometry for ALG-020 heading inclusion.

Solution: module `heading_range`
(`crates/eunomia-core/src/heading_range.rs`), pure functions over
`&[LibraryItem]` — a snapshot's `items()` slice or any collection in
library order (the order is the canon, DB-005/D-042; no `position`
number is read):

- `heading_branch(items, heading_index)` returns
  `HeadingBranch { level, range }` where `range` is `Range<usize>`:
  start = the heading's own index (ALG-060: "from the heading
  itself"), end = the closing heading's index or `items.len()` at the
  end of the list. INDICES, not a copied list: ALG-061's cut-and-
  insert and ALG-062's "all headings in the branch" consume positions
  directly, and "moving a branch inside its own range" becomes an
  index check on `range` — no second traversal to map items back to
  positions.
- `heading_branch_by_id(items, &ItemId)` — the same branch addressed
  by heading id, the form a reorder command naturally holds; errors
  with `HeadingNotFound` (an instruction id is rejected too).
- `HeadingBranch::section()` — the §2 "heading section" as
  `range.start + 1..range.end`, so callers never re-derive the
  off-by-one; the spec defines both terms and they share the close.
- Errors: `IndexOutOfBounds`, `NotAHeading` (only headings root a
  branch), `HeadingNotFound` — thiserror, `ItemId` carried in the
  not-found variant for diagnostics.

The closing predicate is `heading.level.as_u8() <= L` ("L or higher" =
number less than or equal; `HeadingLevel` has no `Ord` impl and
library.rs is frozen, hence `as_u8`). Hidden instructions need no
code: they are ordinary items of the ordered structure (D-11), the
walk never reads instruction state — the tests pin that with hidden
items placed at branch edges. Level skips are the same rule read
twice: an H4 after an H2 with no H3 between is a branch inside the H2
(deeper never closes higher), and its own branch closes at the first
heading of level 2–4. `body.rs` stays untouched (out of the task's
zone, D-053): the two modules share the §9.2/ALG-060 closing rule,
not code; its `section_end` keeps serving ALG-020's per-heading
inclusion. Follow-up for whoever opens `body.rs`'s zone next: replace
its private `section_end` with this module's walk, so ALG-020 and
ALG-060 cannot drift apart silently.

Tests: the DoD list as unit tests — nested levels (an H3 branch as a
subrange of its H2), level skips in both directions (H2→H4 nesting;
H4 closed by an H3 before any same-level H4), the deeper-branch close
on "L or higher" (including H3 NOT closed by a following H4), H6
closing only on H6-or-higher, end of list (branch to `len`, heading
as the last item, instruction as the last item), H2 at index 0,
adjacent same-level branches (empty branch, empty section), hidden
instructions inside and at the last position, all three errors, and
the id form resolving to the index form's answer. Plus one proptest
over random trees (256 cases, fixed seed 45 — body.rs's run policy,
D-053): for every heading of every tree, nothing of level L or higher
sits strictly inside the range, and the item at `end` — when the list
has one — is a heading of level L or higher. Known open item, as in
D-053/D-062: lib.rs's `//!` module list and ARCHITECTURE.md gain no
`heading_range` entry — the T-045 brief capped the lib.rs change at
one `pub mod heading_range;` line while neighbors work in the crate.

### D-073 — The LIB-032 heading-level skips: adjacent-heading pairs, one report per edge — 2026-11-19

Problem: T-048 asks core to find the heading-level skips the UI marks
with the LIB-032 warning icon. The card names "ALG-063 (§9.7)", but
§9.7 ends at ALG-062 and no ALG-063 exists in docs/spec.md — the
rule's canon is the §7.3 acceptance row LIB-032/034/MAIN-018,019:
"Skipping a level (for example, `##` then `####`) is allowed and
marked with a warning icon" (the same sentence D-072 cites for "the
icon is UI, not validation"). Left open: whether a skip reads
branches or adjacent headings, what one report covers when several
levels are skipped at once, and which facts the DTO carries.

Solution: `gaps.rs` with one pure function,
`heading_level_gaps(&[LibraryItem]) -> Vec<HeadingLevelGap>` — a
single walk that remembers the previous heading only. The pair is
ADJACENT HEADINGS: the example is two heading markers (`##` then
`####`), instructions render no heading line (ALG-020) and never
close a branch (ALG-060), so they are transparent — an H2 followed
by instructions followed by an H4 is the same skip as H2 directly
above H4. Every pair the rule reports is a parent edge under
ALG-060 (a deeper heading never closes the branch above it, so the
earlier heading is the later one's parent), but the readings are
NOT equivalent: a heading that closes a deeper sub-branch and then
hangs under an ancestor across a level gap — the H4 in `H2, H5,
H4` — is not reported, because its immediate predecessor is deeper
than it (a close); the adjacent-heading reading is the implemented
one, so the icon marks the heading that ARRIVES too deep, not every
descendant of a skipped level (review R1 pinned the divergent
shapes as named tests). Rule: report iff `next_level > prev_level +
1`; one
report per offending edge however many levels it skips (H2 → H5 is
one skip, not two). Not skips: closes and siblings (level L or
higher closes, ALG-060), the first heading (no pair), the end of
the list (no pair), instruction-only and empty lists. The function
rejects nothing — skips are legal input (D-072); it supplies facts.

The DTO is the minimal fact a warning icon needs: `from_index` (the
parent), `to_index` (the row the icon marks — the heading that
arrives too deep), `from_level`, `to_level`, plus two derived
helpers, `jump()` (observed depth, ≥ 2 for a real report; skipped
levels = jump − 1) and `expected_level()` (from_level + 1, the
shallowest legal next step — a real report's from_level is at most
an H4, so the answer is H3…H5; a hand-built pair rooted at an H6
saturates at H6, keeping the helper total the way `jump`'s
saturating_sub does). Following the `HeadingBranch` precedent
(D-070) the report derives no serde — mapping it over IPC is the
command layer's task.

Tests: the DoD as unit tests — dense trees (the full H2→H6 ladder
and a mixed close/sibling tree) report nothing; the LIB-032 example
H2→H4 with the exact DTO; H2→H5 and H2→H6-after-an-empty-branch as
single reports with the right jumps; several skips in one list in
collection order; a skip deep in the tree naming its real parent
(the H3, not the H2 root); active and hidden instructions transparent
(bare vs interleaved ladders, indices shifted only); closes/siblings/
equal levels never reported; the end of the list creating no report;
a deep first heading, a single heading, instructions only, empty;
DTO well-formedness (indices address those very headings, no heading
strictly between, jump ≥ 2, expected = from + 1); purity and
determinism; the T-047 integration (change-level creating a skip,
a −1 shift closing one) and the ALG-060 cross-check (the pair's later
heading lies inside the earlier one's branch); the divergent shapes
where the adjacent and parent-edge readings split (`H2, H5, H4` and
`H2, H4, H4`: the closing/sibling heading under a gap is NOT
re-reported) and `expected_level` saturating for a hand-built
degenerate pair. Four proptest properties (256 cases, fixed
seed 48 — the D-053/D-070/D-071/D-072 run policy, which this entry
joins): the reports equal the independently recomputed too-deep
adjacent-heading pairs; every report is a real parent edge; interleaved
instructions change only the indices; determinism.

### D-074 — Windows test binaries load through an externally staged Common-Controls v6 manifest; the staged copy fsyncs through a write handle — 2026-11-19

Problem: the `test-matrix (windows-latest)` leg (D-060) died at process
start with STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) before any test
executed. The PE import table showed `comctl32.dll` statically imported
by the `eunomia_sync_lib` test exe but by none of the store exes: tao and
tauri import `TaskDialogIndirect`, an export that exists only in Common
Controls v6, while every cargo-test harness exe linked with no manifest
at all — tauri-build embeds the app manifest through embed-resource's
`cargo:rustc-link-arg-bins`, which reaches `[[bin]]` targets only
(upstream tauri-apps/tauri#13419; the same diagnosis and fix shape are
merged in dohooo/helmor@386f103, libnyanpasu/clash-nyanpasu#4830,
Xoshbin/asyar#499 and Psychotoxical/psysonic#1257). With no manifest the
loader binds comctl32 5.82 and the import cannot resolve, so the exe
dies before `main`.

Solution: CI stages `src-tauri/windows/app.manifest` as an external
`<exe>.manifest` beside every eunomia-sync test exe, in the staging step
between `cargo test --no-run` and the run (next to the WebView2Loader
DLL copy; the step throws when no exe matches, so a cargo layout change
fails loudly instead of degrading back to the loader crash). The loader's
documented precedence makes the mechanism precise: an exe without an
embedded manifest picks the external file up (the lib test harnesses —
Common-Controls v6 binds and `TaskDialogIndirect` resolves), while an exe
with an embedded manifest ignores it (the bin harness keeps tauri's
RC-embedded one; Vista+ embedded-wins, "Specifying a Default Activation
Context" on Microsoft Learn). Four delivery routes were measured dead on
the way there and are kept out of future retries: RUSTFLAGS cannot carry
`/MANIFESTDEPENDENCY` at all (cargo word-splits RUSTFLAGS shell-style; a
quoted space-bearing value cannot survive the cargo→rustc hop);
build-script `cargo:rustc-link-arg` `/MANIFEST:EMBED` +
`/MANIFESTINPUT` compiled clean but changed nothing at runtime on cargo
1.100.0 (the brand-new stable whose build-dir layout v2 places test exes
in `build/<pkg>/<hash>/out`; every published precedent predates it); the
same flags through RUSTFLAGS embed fine but collide with tauri's
resource.lib on the bin test harness (CVT1100 duplicate resource
MANIFEST/1 — cargo applies `-bins` args to a bin's test-mode link too);
and `/MANIFESTINPUT` without `/MANIFEST:EMBED` is refused outright
(LNK1220). Known boundary: the staging is CI-only, so a local Windows
`cargo test --workspace` still dies with 0xc0000139 unless the manifest
is copied beside the exes the same way — CONTRIBUTING.md documents the
workaround; revisit when cargo's build-script link-arg behavior on
1.100+ is understood or upstream lands a durable fix. The same task's
second Windows find, fixed in `stage_copy`: the staged
`*.sqlite3.tmp` was fsynced through the read-only handle `File::open`
yields — Windows' FlushFileBuffers requires GENERIC_WRITE and returns
ERROR_ACCESS_DENIED there (Unix fsyncs read-only descriptors fine), so
every restore/recreate staging step died with os error 5; the handle is
now opened `.write(true)`, flush-only, with no bytes written through it.
The best-effort parent-directory fsync after the publish rename remains
a de-facto Unix-only arm (Windows cannot open a directory for a flushing
handle that way) — accepted, since the staged file's own fsync already
ran.

### D-075 — The §6.9 project field validators: trim-then-count names, the visible-characters ruling for labels, and the palette as the color column's only gate — 2026-11-19

Problem: T-049 asks for the §6.9 validators of the five editable
project fields (the PRJ-008/009/011, 021 settings modal; the file
name rides PRJ-012's per-project copy). §6.9 states the rows tersely
and leaves three readings open: "1–100 characters after trimming
spaces" (trim what, and is the trimmed value the stored one?),
"1–6 visible characters, case is kept" (what is a visible
character?), and "a value from the palette" (the §6.2 `color` column
turns out to have NO CHECK — see below — so nothing at the database
enforces it). Also open: which "file name" — the base-name list or a
typed target path.

Solution: `project_validators.rs` — five pure functions over the
stored string forms, plus one typed error enum. The `Color` palette
enum itself joins the domain vocabulary in `project.rs` (DB-040's
twelve literals, `ALL` in spec order, serde pinned both directions),
because that module is the crate's vocabulary home (D-042) — the
validators consume it like they consume `BaseName`.

Name: trim FIRST, then count — `str::trim` (all Unicode
whitespace), then code points 1–100, returning the trimmed string
for the caller to persist. That return contract is what keeps the
app rule and the schema CHECK in exact agreement: SQLite's `trim()`
strips only U+0020, a SUBSET of Rust's strip set, so the stored
returned value satisfies `length(trim(name)) BETWEEN 1 AND 100` at
the same length the validator measured. The one verdict divergence —
a name made entirely of non-U+0020 whitespace (a tab, an NBSP) is
Empty here while the bare CHECK would pass it — is deliberate and
stricter: the row's trim exists so a name is not stored as
nothing-but-whitespace (the row reserves no control-character rule
for names — that concern is the variant-text row's, which rejects
`NUL` and reserved markers). Counting unit is the code point
(`chars().count()`), the unit SQLite's `length()` uses on TEXT —
not bytes, not graphemes.

Label — the visible-characters ruling (the spec never defines
"visible"): a character is VISIBLE iff it is neither Unicode
whitespace (`char::is_whitespace`) nor a control
(`char::is_control`). Rules, in check order: total length 1–6
INCLUDING spaces (exactly the schema CHECK's own bound, so what
passes here passes the database), at least one visible character
(DB-042: "An empty label is not allowed" — an empty or
whitespace-only label renders nothing next to the color, DB-041),
and no control character anywhere (Cc renders nothing and breaks the
one-line tag; the variant carries index and character). Inner
spaces are legal ON PURPOSE: DB-042 derives the default label from
the first up to 6 characters of the name in upper case — "My
Project" → "MY PRO" — so rejecting spaces would invalidate the
spec's own derivation. Case is kept: `validate_label` returns its
input slice unchanged — validation, not normalization; the
upper-case derivation is the UI's default for a NEW label, not a
rule stored labels follow. Recorded limitation: std's predicates
stop at Cc and White_Space, so three near-invisible families ride
along as legal input, each exactly as it passes the schema's bare
length CHECK — format characters (Cf: a zero-width space, a ZWJ)
count as visible; the line and paragraph separators (U+2028/U+2029)
ride as whitespace like a space; a lone combining mark counts as
visible. Separating them needs a Unicode data crate outside the
allow-list (D-017) and waits for a consumer that needs it.

Description: verbatim ≤ 1000 — no trim (the row names none, the
CHECK has none), empty legal (the column defaults to `''`), and the
input returned unchanged.

Color: the palette literal must match exactly, case-sensitive like
every schema literal. The §6.2 column is `color TEXT NOT NULL` with
NO CHECK — `validate_color` is the only gate a stored color ever
passes, which is why the palette lives in typed code rather than
schema hope.

File name: the §2 base-name list, not a typed path — §6.9's "a
value from the list" with "path and extension are not typed in" is
picker guidance; validation is exact membership in
`AGENTS.md`/`CLAUDE.md` (D-03), returning the parsed `BaseName`.
Out of scope, recorded: the §6.9 project PATH row (`.git` and
readability are fs I/O — the fs crate, per D-006) and the name's
"duplicates allowed with a warning" clause (a cross-row store query
with warning severity, not a field rule).

Errors: one `ProjectFieldError` enum, a variant per field and
failure (`NameEmpty`, `NameTooLong { length }`, `LabelEmpty`,
`LabelTooLong { length }`, `LabelControlCharacter { index,
character }` — the index counts code points, the module's unit —
`DescriptionTooLong { length }`, `UnknownColor
{ value }`, `UnknownFileName { value }`) — never a string — so the
command layer can map any of them to `E_VALIDATION` (§8) on the
D-062/D-071/D-072 precedent of typed errors at core boundaries.

Tests: the DoD as boundary pins — name 0-after-trim (empty, spaces,
Unicode whitespace mixes) / 1 / 100 / 101, trim-before-count (100
padded with edges), inner spaces kept, code points not bytes or
graphemes ("é"×100 vs 101, "e+accent"×51 = 102 code points); label
0 / 1 / 6 / 7 total, case kept verbatim (no folding, no trimming),
inner space legal (the DB-042 "MY PRO" shape, visible-plus-padding
to 6 on both edges), controls rejected with index at any position
(BEL, NUL, ESC, newline), the fixed precedence too-long →
no-visible → control (7 spaces is TooLong; a lone tab is Empty),
and the Cf boundary pinned as accepted; description 0 / 1000 /
1001 verbatim; color all twelve literals plus case and near-miss
rejections with the value kept; file name both literals plus paths,
case variants and other stems; the five fields validating together
(the modal's save), and every failure mode being a typed variant.
No proptest: the rules are total boundary arithmetic with nothing
random to explore beyond the exact pins.

### D-076 — The §6.9 library field validators: the validation/normalization split, the reserved-line scan over the built form, and the variant-name one-line ruling — 2026-11-19

Problem: T-050 transcribes the §6.9 library rows (instruction name,
variant name, variant text, heading name + level; DB-006, DB-016) into
`eunomia-core`, and the card's clauses left five readings open: what
"CRLF→LF" does inside a VALIDATOR given that ALG-002
(`normalize::variant_text`, D-044) already owns CR/CRLF→LF + BOM +
edges; which lines DB-016's "a line equal to the marker" reserves,
given `classify_line` (D-062) also matches padded marker lines;
whether the variant NAME row — the one name row that does not say
"one line" — may contain line breaks; which fields the NUL rejection
applies to; and whether a heading-level validator is still needed at
all.

Solution: `library_validators.rs` — four pure functions plus one
typed error enum (`LibraryFieldError`), in the T-049/D-075 shape
(code-point counting, trim-then-count names, typed per-field errors,
schema-CHECK agreement, boundaries pinned at 0/1/limit/limit+1). The
rulings:

1. Validation-versus-normalization split. The REJECTION rules are
   this module's: the three length bounds (200/100/262,144 — §6.9,
   DB-006, the §6.2 CHECKs), the NUL rejection, and DB-016's reserved
   marker lines. The TRANSFORMATIONS stay where they already live:
   the save-time "CRLF is changed to LF" fold is the STORE layer's
   own §6.9 clause, and the build-time form — lone CR, BOM, block
   edges — is ALG-002's unconditional pass, single-owned by
   `normalize::variant_text` (D-044). `validate_variant_text` returns
   its input UNCHANGED: validation, not normalization; nothing here
   folds, trims or strips variant text (unlike the name fields, the
   text row and its CHECK have no trim — "empty is allowed" means
   the empty string, and a whitespace-only text is legal and simply
   builds to an empty block, D-13). The store order that preserves
   spec fidelity is fold FIRST, then validate — the length then
   measures exactly the stored form, which is what the schema CHECK
   sees. Validating the raw form is conservative in one direction
   only (the round-1 review's correction of this entry's first
   draft): it never lets a database violation through (raw length ≥
   folded length), but it can reject a text §6.9 would allow, by one
   code point per CRLF — a raw `"x\r\n"` paste keeps two thirds of
   its character count once folded (measured: raw 262,146 → folded
   174,764), so the raw form's rejection at 262,145 may be a legal
   stored text.
   The save-time fold itself owns no code yet — `eunomia-store` has
   no library save path as of this task, so the card's "CRLF→LF"
   clause lives only in this decision: the library create/update
   STORE task owns it, with fold-before-validate as an explicit
   acceptance criterion, and it must decide its fold's scope there
   (§6.9's clause names CRLF only; the lone-CR conversion stays
   build-time with ALG-002) without re-implementing the build form.
2. The reserved-line scan runs over the text AS THE BUILD WRITES IT:
   one `normalize::variant_text` call (reusing D-044's owner), then a
   `classify_line` scan of the result's lines (reusing D-062's).
   Rationale: DB-016 exists so a saved variant can never re-appear
   as a second marker pair in a scheme-B file, and the line structure
   a parse will see is post-ALG-002 — where a lone CR has become LF
   and the leading BOM/empty-line edge is gone. A marker hidden
   behind a leading BOM (`"\u{feff}\n<!-- … -->"`) or exposed by a
   lone CR (`"x\r<!-- … -->"`) is therefore rejected too, and the
   verdict never depends on whether the caller already folded CRLF.
   The widening from DB-016's literal "a line EQUAL to the marker" to
   `classify_line`'s padded match is the same safety argument: a
   padded marker line in the managed block WOULD parse as a marker
   line (D-062's padding class), so exact equality alone would let
   the corruption through. The error's 1-based line number counts the
   built form — identical to the stored text's numbering except
   where ALG-002 dropped the leading edge. The creation-marker line
   (D-061) is NOT reserved: DB-016 names exactly the two D-19
   literals, the creation line never matches `classify_line`, and its
   timestamp makes an exact-match reservation meaningless anyway — a
   pasted copy is inert prose.
3. Variant name is one line. §6.9 writes "one line" for the
   instruction and heading names but only "1–100 characters" for the
   variant name. The omission is read as editorial (the row's Limits
   column already carries the uniqueness and Main-default notes): a
   variant name is the same kind of thing — a list label (the
   LIB-002 variant list) — so `validate_variant_name` rejects inner
   `\n`/`\r` exactly like the other two name fields. The module's one
   deliberate divergence STRICTER than the bare row; pinned by tests.
   Accepted consequence (round-1 review): SEC-009 re-applies §6.9
   limits to imports, so a hand-edited library JSON (D-18) carrying a
   newline in a variant name is rejected where the literal row would
   permit it — no legitimate export contains one, because the editor
   itself cannot save it.
   Edge breaks in all three names are trimmed away, not rejected:
   the STORED name (the returned, trimmed value) is what must be one
   line. The Unicode separators U+2028/U+2029 are whitespace, not
   `\n`/`\r` — they ride along as content, the std-tables boundary
   D-075 already recorded for labels.
4. NUL fields: variant text only — that is where §6.9 and DB-016 put
   it. The three name fields carry no NUL rule (the rows reserve no
   control-character rule for names; the variant-text row owns that
   concern, as D-075 ruled for the project fields), so a name
   containing `\0` validates here. Known open item, verified against
   SQLite directly: `length()` counts code points only up to the
   first U+0000 (`length('a'||char(0)||'bc')` = 1), so a name whose
   FIRST character is NUL passes this validator while the store's
   own `length(trim(name))` CHECK measures 0 — the insert then fails
   the CHECK with a raw database error instead of a field error. The
   same latent gap sits in T-049's project fields; closing it belongs
   to a validators-wide task, not to T-050's zone. The round-1
   review added a sharper consequence for one field: the heading
   title is the one name that reaches the target file verbatim
   (`## <title>` via the body's heading block), so a NUL in it would
   put a NUL byte into `AGENTS.md`/`CLAUDE.md` — git then treats the
   file as binary. That, not the SQLite arithmetic, is the same
   reason the variant-text NUL rule exists; it belongs on the same
   validators-wide task's list.
5. Heading level needs no validator: the closed `HeadingLevel` type
   (D-042) already makes 1 and 7 unrepresentable, with `TryFrom<u8>`
   as the store's boundary conversion — there is no free integer left
   to validate. Also deliberately absent: the two case-insensitive
   uniqueness clauses (cross-row store queries; the schema's NOCASE
   index backs one), the `Main` default name (a UI default), and the
   backup-limit row (a settings field, not a library field).

Names trim-then-count exactly like D-075 (Rust's `str::trim` ⊇
SQLite's U+0020-only `trim()`; the trimmed string is returned for the
caller to persist; whitespace-only is Empty). Check order inside the
text validator is fixed for a deterministic error code: length, then
NUL, then the reserved scan; inside a name: length, then the
one-line scan. Boundary: `ReservedContent` is core's variant for the
future `E_CONTENT_RESERVED` mapping in the src-tauri registry
(D-034), exactly as `MarkersError` relates to `E_MARKERS_INVALID`
(D-062) — the mapping itself is a later command-layer task. The
lib.rs module-list entry for `library_validators` was added; the
older list's missing entries (markers, marker, assembly_b, compare,
dirty, extra_files, hash, target_file — D-062's known open item)
remain untouched: ARCHITECTURE.md, the other half of that
reconciliation, is outside this task's zone.

### D-077 — The D-29 tie-break: sha256 of the ID, first 8 bytes big-endian, remainder over palette order — 2026-11-19

Problem: T-051 implements the new-project color rule (DB-041 → D-29,
PRJ-010). D-29 states "The least used palette color among active
projects; on a tie, chosen by the hash of the project ID
(deterministic)" — and names nothing else. Three load-bearing thirds
are unspecified: WHICH hash, HOW the digest folds to a choice, and
HOW the folded value maps onto the tied candidates. The first half
carries PRJ-010's acceptance alone (a count-0 color always beats a
count ≥ 1 color, so twelve consecutive adds from an empty library
are twelve distinct colors), so the openness touches only the
tie-break — but an unpinned tie-break is an unpinned test oracle.

Solution: `eunomia_core::color_pick::pick_color(&[Color],
&ProjectId) -> Color` counts the occupied slice per palette color
(DB-040 order = `Color::ALL`, T-049/D-075 — the palette is reused,
not restated), takes the colors at the minimum count as candidates
in palette order, and folds the identifier to an index among them:

1. _Hash_ — SHA-256 over the ID's UTF-8 bytes. `sha2` is already in
   core's allow-list (D-017, pinned ahead of exactly such tasks) and
   already has the D-064 build-hash precedent; a cryptographic hash
   makes the fold's low bits indifferent to project-name structure.
2. _Fold_ — the digest's first 8 bytes, big-endian, as one `u64`.
   Eight bytes dwarf the ≤ 12-way choice; big-endian is pinned for
   cross-build stability (either byte order works; only writing one
   down does — the same once-only spirit as D-064's little-endian
   length framing).
3. _Mapping_ — `scalar % candidate_count` indexes the candidates in
   DB-040 palette order. Deterministic by construction, and
   set-size-relative by nature: two IDs that agree over eleven
   candidates need not over ten, which is fine — D-29 asks for
   determinism per decision point, and a pure function delivers
   exactly that.

The "among ACTIVE projects" filter lives one seam away, in the
store's query that fills the slice (`WHERE state = 'active'`): the
`projects.color` column is NOT NULL with no CHECK (§6.2), every
value parses through `validate_color` (D-075's gate), and DB-014's
"an archived project … does not take part" therefore reduces to
"its color stops being passed in" — archiving frees the color as
the unique count-0 candidate for the next add, restoring re-seats
it. When all twelve colors are in use, every color ties at the
minimum and the hash decides among all of them — a repeat, which
DB-041 explicitly blesses ("color is not the only sign of a
project", the label rides next to it, NFR-020).

The test oracle is external, not mirror-on-mirror (the D-064 review
lesson): the FIPS 180-4 published sha256("abc") digest pins the
whole chain — hash function, byte order, fold width and palette-
order indexing — in one assertion (`0xba7816bf8f01cfea % 12 == 6`
→ cyan), and the same published scalar re-folds over a shrunk
candidate set (`% 11 == 10`). PRJ-010's twelve-in-a-row scenario is
asserted directly, as are least-count membership, multiset order
independence, and the archive-frees-the-color seam.

The round-1 review hardened that oracle: ONE vector's remainders
are coincidentally reproduced by degenerate folds (the abc scalar's
`%12`/`%11` are also `first-byte % 12`/`first-byte % 11` for
0xba = 186, and a constant 54 passes everything), so the suite now
carries the second FIPS 180-4 vector (Appendix B.2, the two-block
message: `0x248d6a61d20638b8 % 12 == 8` → indigo, which the
one-byte fold's 36 and the constant 54 both miss) plus a
many-identifiers spread assertion no constant-scalar implementation
can satisfy. The same review found and removed the module's one
restated palette fact — a local `PALETTE_SIZE = 12` constant, now
sized from `Color::ALL` itself.

Two deliberate lags, recorded per the D-064 precedent: lib.rs's
"sha2 idle" parenthetical is updated in this task's already-touched
lib.rs (it had been stale since D-064 landed), while
ARCHITECTURE.md's workspace-layout module list — incomplete since
before this task — stays out of T-051's zone and belongs to the
orchestrator's docs pass. And the active-only seam stays prose for
now: `&[Color]` cannot distinguish "active projects' colors" from
"all projects' colors", and making it explicit (an
active-colors-only newtype, or a store-side test that the query
filters `state = 'active'`) belongs to the store task that first
builds the slice.

### D-078 — The cancelled-CI auto-rerun: workflow_run, a job-level cancelled-only discriminator with a derived-gate carve-out, and an artifact-marker cap — 2026-11-19

Problem: macos arm64 CI legs (build (macos-15), occasionally
test-matrix (macos-15)) are regularly cancelled by runner capacity
constraints while the code is green; every incident cost a manual
`gh run rerun --failed` (runs 44584364443 ×2, 44574620012, 44123571125,
44147164960). T-242 automates that with two hard requirements: never
mask real red, and never rerun one commit endlessly.

Solution (ci-rerun.yml + .github/scripts/ci-rerun/decide.py):

- _Trigger_ — `workflow_run: workflows: [CI], types: [completed],
branches: [main]`: fires exactly once per completed CI run of main
  (ci.yml pushes are main-only) and hands over the run id; a schedule
  would need its own state and a race-prone scan. `workflow_run` only
  runs the default-branch copy of the file, so the live path is
  untestable pre-merge — hence the workflow_dispatch drill modes
  (fixture + read-only dry-run over a real run id) and the honest
  boundary below.
- _Discriminator_ — job-level only, because the run-level conclusion is
  unreliable for infra cancels (44123571125 rolled up to "failure",
  44147164960 to "cancelled"). Rerun iff every non-success job has
  conclusion `cancelled`, ≥1 cancelled, every job `completed`, and no
  conclusion outside {success, failure, cancelled} — timing_failure,
  startup_failure, skipped and friends are never healed. Any real
  failure is a no-op (red = human). One provably-safe carve-out: the
  roll-up gate `test` (ci.yml: `if: always()` over
  needs.test-matrix.result) fails whenever a matrix leg was cancelled
  even with zero real failures (real shape: 44578261527, where the gate
  failed at 17:47 after the macos-15 leg was cancelled at 17:47). That
  derived failure is excused only when every `test-matrix (` leg is
  success/cancelled with ≥1 cancelled; without it the healer could
  never heal the test-matrix cancels it exists for. The carve-out fails
  closed: any leg failure, no cancelled leg, a renamed gate (an unknown
  failing job is a hard failure), or renamed matrix legs (legs: none)
  each keep the gate failure "real" — all four paths are unit-tested.
  Its safety additionally rests on the `test` job staying a pure
  roll-up; that obligation is recorded as an INVARIANT comment on the
  job in ci.yml itself, where a future editor would trip over it.
- _Cap_ — at most 2 auto-reruns per head SHA, counted from artifact
  markers `ci-rerun-marker-<full sha>`: reruns bump `run_attempt` on
  the same run row (`gh run list --commit <sha>` returns ONE row) and
  conflate with the owner's manual reruns, so run history cannot count
  our reruns. Each heal uploads the marker BEFORE issuing
  `gh run rerun --failed` — a marker without a rerun wastes one slot
  (visible, conservative), while a rerun without a marker could exceed
  the cap. Markers expire after 90 days, far beyond a SHA's CI life.
- _Blast radius_ — permissions `actions: write` (rerun; same scope
  reads jobs/artifacts) + `contents: read` (checkout of the script and
  the main-tip ref lookup); concurrency keyed by the completed run's
  head SHA, so the self-heal loop AND any same-SHA replay serialize —
  the marker count -> upload window can never be raced for one SHA;
  `gh`-side job list is guarded by a total_count-vs-length check
  (truncation = loud exit 2, never a decision), and the marker counter
  awk refuses anything but per-line integers (an empty/malformed
  counter fails the job — the cap must never fail open). The policy
  lives only in decide.py's `decide()`, unit-tested on 5 fixtures —
  two of them verbatim per-attempt job lists of run 44584364443
  (attempt 1 = the derived-gate shape, attempt 2 = the cancelled-only
  shape) — 18 tests, stdlib unittest, zero dependencies: the same
  command runs locally and on every invocation of the workflow
  (immediately after checkout). Python carries no dedicated linter
  here, deliberately: the scripts are stdlib-only and ~300 LoC total,
  and the D-041 chain's py_compile plus the unittest import surface
  already gate what a linter would catch at this size; pulling in
  ruff/pylint for two files was judged cost without coverage.
- _Superseded commits_ — a workflow-level precondition outside the
  pure function: a rerun is issued only while the completed run's head
  SHA is still the tip of main. ci.yml's own `cancel-in-progress`
  produces exactly the healable job shape (a push B landing mid-run
  cancels run A's in-flight legs), and reviving a dead commit would
  burn the macOS capacity this healer protects. The tip lookup happens
  in the collect step (contents: read), a mismatch downgrades rerun to
  noop with an explicit reason line, and a failed lookup fails the job
  rather than guessing. Residual race (a push landing in the seconds
  between the tip check and `gh run rerun`): accepted — bounded by the
  cap, one wasted heal at worst. Known unfixable edge: the API exposes
  no cancellation actor or reason, so a HUMAN cancelling a still-tip
  run is indistinguishable from a capacity cancel and will be rerun;
  the cap is the backstop, and the healer never masks red either way.
  The downgrade path is drillable post-merge via the dispatch input
  `force_tip_mismatch` (and was exercised pre-merge by running the
  decide step's exact bash locally over the derived-gate fixture with
  TIP_OK=false).

Drill boundary, stated honestly: GitHub refuses to dispatch a workflow
that exists only on a feature branch (`gh workflow run ci-rerun.yml
--ref t-242` → "HTTP 404: workflow not found on the default branch" —
captured verbatim during the T-242 drill), and `workflow_run` likewise
runs only the default-branch copy, so NO live GitHub-runner execution
of this file is possible before merge. What WAS verified, locally and
end-to-end: the exact collect-step bash extracted from the workflow,
run against the live API in both modes (live-mode shape over green run
44584364443 → noop "nothing to heal", tip check passing; dry-run mode
→ real jobs + real marker count 0 via --paginate); decide.py's CLI
over all five fixtures; 18 unit tests; actionlint 1.7.7 + shellcheck
clean; prettier/eslint/cargo-fmt/typecheck green. The rerun MECHANISM
itself is proven by this repo's own history via the per-attempt jobs
endpoint (GET /actions/runs/<id>/attempts/<n>/jobs): run 44584364443
attempt 1 ended with build (macos-15) + test-matrix (macos-15)
cancelled and the gate failed at 18:22 with zero real failures —
exactly the derived-gate fixture; attempt 2 restarted precisely those
jobs; attempt 3 restarted the one still-cancelled build with zero
failed jobs present. NOT verifiable before merge: this workflow's own
wiring (the workflow_run trigger, the upload-artifact marker path, the
concurrency group under back-to-back completions). The first real
cancelled macos leg on main after landing is the live confirmation;
until then the live path rests on the drilled pieces, the mechanism's
historical proof, and the discriminator's tests.

### D-079 — The DB-042 default label: upper-case-then-slice, the stored (trimmed) name as input, and the derivation/validator split for "an empty label is not allowed" — 2026-11-19

Problem: T-052 implements DB-042 — "The default label is the first up
to 6 characters of the name, in upper case. An empty label is not
allowed." The sentence leaves four readings open. (1) ORDER: does "in
upper case" apply before or after the slice? The spec's own example
("My Project" → "MY PRO") cannot decide it — ASCII upper-casing is
length-preserving, both orders give six. (2) UNIT: "characters" as
code points, bytes or graphemes? (3) INPUT: the raw typed name or the
stored one? (4) Who enforces the empty prohibition — the derivation
(erroring on empty input?) or the validator?

Solution: `default_label(name: &str) -> String` in
`project_validators.rs` (not a new module: the derivation's contract
is stated in that module's own terms — its input domain is
`validate_name`'s output, its output obligation is `validate_label`'s
input; a separate file would split the label story in two):

```rust
name.trim().chars().flat_map(char::to_uppercase).take(6).collect()
```

(1) UPPER FIRST, slice second. "Up to 6 characters" describes the
RESULT — the default label is a thing of at most 6 characters, upper
case — so the bound must hold after the upper-casing, and
upper-casing EXPANDS (`"ß".to_uppercase()` = `"SS"`, `"ΐ"` = three code points).
Slice-first
lets six name characters surface as up to eighteen label code
points (`"ΐΐΐΐΐΐ"` → 18, `"ßßßßßß"` → `"SSSSSSSSSSSS"` = 12) — a
"default" the §6.9 rule and the
schema's `length(label) BETWEEN 1 AND 6` both reject: an unstorable
default the spec cannot mean. Upper-then-slice loses nothing of the
letter — an upper-cased string is its characters' upper-casings
concatenated, so its first 6 code points are exactly
slice-then-upper clipped back to 6 (the two orders are extensionally
identical once the result is clipped, which "up to 6" demands).
`"straße"` pins the order: → "STRASS" (6), not "STRASSE" (7).
(2) The unit is the code point (D-075's ruling for the label column,
SQLite's `length()` on TEXT): `chars()`, `flat_map`, `take` — never a
byte or grapheme slice. Six astral emoji derive six characters (24
bytes); a combining mark rides its base letter through the mapping.
(3) The input is the STORED name — the trimmed string `validate_name`
returns (its return contract, D-075) — and the function trims its
input itself, so `default_label(raw)` == `default_label(validated)`
and a raw caller cannot spend slice budget on edge whitespace
(`"  My Project"` would otherwise derive `"  MY P"`).
(4) Derivation produces, validator gates: the function is TOTAL —
"" (or pure whitespace) in, "" out, no panic, no Result — and for a
name `validate_name` accepted the derived STRING is never empty (at
least one code point survives): upper-casing never deletes a
character or maps it to whitespace or control (a test scanning
EVERY scalar value pins the lemma against future Unicode tables).
The result is VISIBLE unless the consumed prefix has NO visible
character: after the trim the first character is never whitespace,
but it can be a control, and the rest of the prefix can be
controls or inner spaces — all invisible (names reserve no control
rule, D-075, and trim removes only the edges). The "not allowed"
half of DB-042 stays `validate_label`'s `LabelEmpty`.

Recorded edge, pinned by test: names reserve NO control-character
rule (D-075), so both of these are legal names whose derived
defaults the label's own rules reject — "\u{7}abc" derives
"\u{7}ABC" and fails the control check, while a name whose
consumed prefix has no visible character — controls only
("\u{7}") or a control-and-space mix ("\u{7} \u{8}") — derives a
label rejected as `LabelEmpty` (the verdict DB-042's second
sentence names). The
derivation stays literal — filtering would invent a rule DB-042
does not state — so `validate_label(&default_label(valid_name))` is
Ok for every name whose first six characters carry no control
character, and the modal shows the field error for the user to edit
in the two cases it is not. The edge exists only WHILE names
reserve no control rule: a future §6.9 name-row change that rejects
controls dissolves it with no change here. Cf. adjacent field: the
new-project COLOR needs no such caveat (D-29 picks from the closed
palette, D-077).

Tests: lengths 1–5 (each pinned) / exactly 6 / longer (DB-042's
example verbatim); the order pin ("straße" → "STRASS", "ßßßßßß" →
"SSSSSS", the three-code-point "ΐΐΐΐΐΐ" → six, "ß" → "SS",
each result re-validated); code points not bytes/graphemes
("ПРОЕКТ" = 12 bytes / 6 chars, seven emoji → six, "e\u{301}"×4 →
"E\u{301}"×3); trim-first (padded and Unicode-whitespace edges, ""
and " " → "" which `validate_label` then rejects); the property
battery (15 names — Cyrillic, ß, İ, combining marks, emoji, Latin
accents, padded — each `validate_name`-Ok input deriving a
`validate_label`-Ok default, trim-neutrality asserted, and the
100-character name ceiling asserted alongside); the
control-character counterexamples (the control check; the
invisible prefix, incl. a control-and-space mix); the full-scan
lemma; and the new-project composition (validate the name, derive
the default, validate the label). No proptest, per D-075's
ruling that these rules are total boundary arithmetic — the
full scalar scan covers what a random char sample would approximate.
