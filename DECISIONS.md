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

Addendum (2026-11-21, T-253/D-129): the conscious omission above is
decided now — the owner ruled a 5-file cap, and the builder carries
`.max_log_files(5)` (daily rotation unchanged; tracing-appender 0.2.5
has no size rotation, so the ruling's per-file 10 MB half is honestly
unenforced). The decision, its boundaries and the tests live in D-129;
this entry's rotation/name claims stand as recorded.

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

Addendum (2026-11-21, T-253/D-129): the app-layer retention this
entry deferred has landed — `retention.rs` retires pre-migration
copies older than 7 days at every successful `Db::open`, newest of
the kind protected (which is exactly this entry's "never delete the
copy UPD-004 points at" rule operationalized: the pointer is always
the newest pre-migration copy). This crate is untouched: the store
still never removes a final copy.

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

Addendum (2026-11-21, T-253/D-129): the future owner decision above
is made — pre-recovery copies retire by the 7-day age rule at every
successful `Db::open`, newest of the kind protected, never inside
`restore_files` itself (a restore's chosen copy must outlive a
failed publish — D-129's round-1 P1-1). "Renamed, never discarded"
for the damaged database stands for the preserve step; the preserved
copy is later subject to the age rule like any other.

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

### D-080 — The backup name builders: slug cleaning, hash8 truncation, the project-id gate, and the UTC stamp — 2026-11-19

Problem: T-053 implements spec §7.6's backup row (BKP-001, 002,
008, 011), §5.5's layout line and SEC-006 as pure string builders
in `eunomia-core`. The spec fixes the layout
`backups/<project_id>__<slug>/<file>.<UTC YYYYMMDDTHHMMSSmmmZ>.<hash8>.bak`,
"slug is the cleaned project name, up to 40 characters" ("the
main key is `project_id`"), and SEC-006's "Backup paths are built
from the ID and the cleaned slug, not from arbitrary user strings.
The target file name is only a value from the enum" — but leaves
five readings open: (1) the cleaning rules — charset, what happens
to non-ASCII, and to a name that cleans to nothing; (2) what
`hash8` is the first 8 of; (3) whether the opaque `ProjectId`
(D-042: core never validates the UUIDv7 form) may be trusted as a
path component; (4) which enum supplies the `<file>` name; (5) how
the stamp crosses core's no-time-library boundary (D-017; D-061
settled this for ALG-032's marker).

Solution: `eunomia-core/src/backup_name.rs` —
`project_slug`, `target_file_name`, `backup_dir_name`,
`backup_file_name`, `backup_relative_path`, all pure (no I/O,
D-006/D-017: `eunomia-fs` joins the returned §5.5-relative string
under the data directory and owns SEC-007). The rulings:

- SLUG: ASCII letters (lower-cased) and digits are kept; every
  MAXIMAL run of any other code point — spaces, punctuation,
  controls, all non-ASCII — collapses to ONE hyphen; edge hyphens
  are trimmed; the first 40 CODE POINTS of the cleaned string
  survive ("up to 40 characters" describes the RESULT — D-079's
  reading of the same wording in DB-042: clean first, cut second),
  and a cut landing right after a separator run drops the trailing
  hyphen it would leave. A result that is empty falls back to the
  literal `project` — BKP-001's "the main key is `project_id`"
  makes the slug display sugar, and the fallback is the spec's own
  common noun, no user bytes. Non-ASCII is DROPPED, not
  transliterated: the spec owns no transliteration table, and one
  would be invention ("Проект" → `project`). The output charset is
  exactly `[a-z0-9-]`, so `.`, `/`, `\`, NUL, `%` and space are
  unrepresentable and `..` is structurally impossible.
- SEC-006 reading: the sentence itself names the two blessed
  sources — "the ID and the cleaned slug" — so the slug IS the
  permitted derivative it describes; what the rule bars is RAW
  user text reaching a component and a typed file name. The file
  name therefore arrives as `(Scheme, BaseName)` and maps onto
  §5.7's closed list of watched target files: A|B →
  `AGENTS.md`/`CLAUDE.md` (the `BaseName` literal), C →
  `<STEM>.project.md` (D-20 — the main file is not edited).
- HASH8: the first 8 characters of the 64-character LOWERCASE-hex
  SHA-256 of the backed-up file's content — BKP-010's check value
  in DB-020's canonical rendering (the same spelling
  `hash::build_hash` produces). The builders take the FULL digest
  and truncate internally, so a caller cannot hand-pick the short
  id and the one format gate (length 64, ASCII lowercase hex) sits
  next to the truncation it guards; the database row keeps the
  full digest, the name only needs a readable short id.
- PROJECT ID: validated non-empty over `[0-9A-Za-z_-]` (bytes:
  any non-ASCII fails) and refused with the typed
  `BackupNameError::UnsafeProjectId` — a data condition, not a
  broken-invariant panic, because the column is opaque TEXT to
  core (D-042) and a corrupted row must not build a traversal
  path; dots are outside the charset, so `..` stays impossible in
  the joined `<id>__<slug>` name (which also always contains the
  `__` divider, so it can never equal a Windows reserved device
  word).
- STAMP: `BackupTime` is D-061's decomposed-civil-fields posture
  plus a `millisecond` field (domains asserted, panics documented);
  the format is the template verbatim, uncompressed and
  zero-padded — 18 digits-plus-`T` characters before the `Z`, 19
  total, always UTC — chosen by the spec for lexical sorting in a
  plain directory listing, and distinct from ALG-032's marker
  format (D-061) and the store's ISO-8601 spelling. Time is a
  parameter, never a clock.

Path safety is proven, not asserted: the tests drive `../`,
`..\`, `..`, percent spellings (`%2e%2e%2f` — never decoded),
NUL, tabs/newlines and Unicode payloads through the builders and
check the results both by charset and mechanically as
`std::path::Component` lists — single `Normal` components, the
whole relative path exactly three; `ParentDir` is unreachable.
Boundary: no `AppError` mapping in core (D-062's precedent — the
command layer maps `BackupNameError` to §8 codes when the backup
pipeline task lands); `lib.rs` gained exactly one
`pub mod backup_name;` line and ARCHITECTURE.md's already-stale
tree was deliberately left alone (D-062's known open item).

Tests: 36 in `backup_name.rs` — slug oracles (letters/digits/
spaces, specials-and-runs collapse, NUL/controls, traversal
battery, percent literalism, 40-cut at/over the bound,
clean-then-cut with the trailing-hyphen re-trim, non-ASCII
dropping, fallback battery, edge trimming, a property battery
of 20 hostile/ordinary names over the charset/length/shape
rules, and a full `0..=char::MAX` scalar scan of the charset
lemma — D-079's posture for total rules); the enum mapping;
stamp format pins (19 characters, `T` at index 8, zero-padding,
domain floors/ceilings) and ten `should_panic` domain tests;
hash8 truncation + nine refused digest shapes + the Display
pin; the id gate (passing shapes, eleven refused shapes,
Display); §5.5 whole-path oracles for schemes A and C, the
hostile-name end-to-end path, the whole hostile battery run
through the path builder (three `Normal` components each), and
the length bounds (file = `<file>.19.8.bak` + 3 dots; dir ≤
36+2+40).

### D-082 — Version snapshots and hashes: hand-rolled canonical JSON under the allow-list, exclusion by construction — 2026-11-19

Problem: T-055 implements ALG-070 (§9.8) and §7.3's closing sentence in
`eunomia-core`: an instruction version snapshot stores the name and state, a
variant version snapshot the name and text, each with its SHA-256. The spec
left five points open: (1) the exact JSON literal — ALG-070 says "a JSON
snapshot is built" without pinning keys, field order, or the escaping table;
(2) which crate emits that JSON — at runtime this normally means `serde_json`,
but §5.2 pins core to "only `serde`, `sha2`, `thiserror`" (D-017), and D-042
deliberately kept `serde_json` dev-only; (3) how the main-variant flag and the
library order stay out of snapshots — §7.3 excludes them, but nothing stopped
a future caller from threading them in; (4) whether the hash needs D-064-style
length framing like the build hash; (5) whether the builders take whole domain
values (`Instruction` carries id, state AND its variant list; `Variant`
carries an id) or the exact snapshot pieces.

Solution: `crates/eunomia-core/src/version_snapshot.rs` —
`instruction_snapshot`/`instruction_snapshot_hash`/`variant_snapshot`/
`variant_snapshot_hash`, pure and total (no I/O, no clock, no panics on any
`&str`).

- THE DEPENDENCY RULING (the task's open dilemma): §5.2's allow-list and the
  recorded posture (D-017's manifest comment, D-042's "the runtime allow-list
  stays serde/sha2/thiserror") make `serde_json` NOT sanctioned as a core
  runtime dependency — the spec gives a determinate answer, not an ambiguity,
  so nothing was added to `[dependencies]`. The alternative that remains inside
  the allow-list is a hand-rolled emitter, and the task brief's own conditional
  ("hand-rolled string JSON is not acceptable IF serde_json is sanctioned")
  accepts exactly that when it is not. The emitter is ~25 lines over two fixed
  two-field shapes — D-064's hand-rolled-hex precedent at JSON scale — and is
  explicitly NOT a general JSON writer: a third snapshot shape or a wire
  consumer is a DECISIONS.md event. `serde_json` stays dev-only, where it now
  does double duty as the tests' byte-compat oracle.
- CANONICAL JSON pinned: compact (no whitespace), fixed field order —
  `name`,`state` and `name`,`text`, the §7.3 sentence's own order; keys are
  the spec's snapshot words: `name` for BOTH the instruction title
  (`Instruction::title`) and the variant name, `state` carrying
  `InstructionState::as_str` (the §6.2 CHECK literal — the D-042 serde canon),
  `text` for `Variant::content` (the §2/DB-006 word, not the Rust field name).
  String escaping is exactly `serde_json::to_string`'s table (`\"`, `\\`, the
  five short escapes, other controls as lowercase `\u00xx`, every other code
  point verbatim in UTF-8), pinned in tests exhaustively over all 32 ASCII
  control characters plus structural, DEL and non-ASCII samples — so a
  store-side re-serialization of a stored snapshot with `serde_json` can never
  produce a different hash. That byte-compat is also why no D-064-style
  framing is layered on: quoting and commas already make data → JSON
  injective, where DB-020's plain field concatenation was ambiguous. The hash
  is plain SHA-256 over the JSON's UTF-8 bytes (ALG-070 names no tag), in the
  app's 64-character lowercase hex; a future snapshot-format change is a
  DECISIONS event precisely because stored hashes are compared across app
  versions (one spurious version per entity would be the price of a silent
  format drift).
- EXCLUSION BY CONSTRUCTION: the parameters ARE the snapshot — there is no
  flag, position, id or variant-list parameter, and `Variant` itself carries
  no `is_main` field (D-042), so the store's `is_main` column cannot reach a
  snapshot through this API. Pieces, not domain aggregates: taking
  `&Instruction` would hand the function three fields it must deliberately
  ignore and let a future `Instruction` field (a position, say) leak in with
  one careless edit; taking `&str` pieces plus the `InstructionState` enum
  (the one parameter where a domain type makes invalid states unrepresentable)
  pins the snapshot contract to the parameter list itself. Tests pin the
  consequence from the domain side: two `Variant`s differing in id (the
  store-side stand-in for the flag, which is unrepresentable) hash identically,
  and an instruction's variant-list changes cannot move its hash.
- OWNERSHIP BOUNDARY: the four functions only build and hash — ALG-070's
  second half (equal hash → no version created, D-16) is the store task's
  decision over the `revisions` table, not made here; content rules (§6.9
  lengths, DB-016 reserved lines, DB-006's 262,144 characters) are the store's
  write boundary, since a snapshot is a faithful record of what is stored,
  valid or not; no `AppError` mapping in core (D-062's posture).
  `lib.rs` gained exactly one `pub mod version_snapshot;` line
  (D-062's precedent); ARCHITECTURE.md's known-stale tree stays untouched.
  Instead of a second 16-line copy, `hash::to_lowercase_hex` was widened to
  `pub(crate)` — the renderer and its D-064 rationale travel with it.
  One note for the ALG-073 History task (LIB-028/029): a snapshot is ONE line
  of JSON, so its diff with another version's must be computed over the PARSED
  fields (name/state/text), never over the raw strings — a raw diff degenerates
  to a single "whole line changed" block. The snapshot reader joins this module
  then, so one module owns the format's both directions.

Tests: 13 in `version_snapshot.rs` — independent-oracle SHA-256 pins
(Python `hashlib` over the exact JSON bytes: both instruction states, the
plain and the empty-text variant); exact canonical-literal pins for both
shapes; the two acceptance properties (identical data → identical JSON and
hash; the main flag/identity/variant list provably cannot change a hash);
per-field triggers (name, state, text — including whitespace-only and
trailing-newline differences, snapshots being unnormalized); empty text vs a
space; a hand-written escaping literal (quote, backslash, line break, tab,
verbatim emoji/Cyrillic) with its oracle hash; the serde_json byte-compat
battery (all control characters, round-trip parse back to the same pair)
across both shapes; and the DB-006 boundary text building deterministically.

### D-083 — The `markdown_preview` pipeline: renderer in src-tauri, event-level raw-HTML escaping, ammonia-minus-`img`; inline render, no `spawn_blocking` — 2026-11-19

Problem: T-056 implements §7.10's `markdown_preview` (SEC-005, D-15,
the D-009 name): "renders Markdown into sanitized HTML (SEC-005)",
the command being editor-facing — "В сборке файлов не используется".
SEC-005 pins the policy in one sentence: "The Markdown preview is
built in Rust; the HTML is sanitized (`ammonia`); raw HTML from
instruction text is escaped. External resources are not loaded." Six
readings were open: (1) WHERE the renderer lives — D-006's original
vision had markdown logic in eunomia-core, but D-017 pinned core to
the spec 5.2 allow-list ("only `serde`, `sha2`, `thiserror`"), which
neither §5.3's `pulldown-cmark` nor SEC-005's `ammonia` belongs to;
(2) HOW "raw HTML … is escaped" is achieved — ammonia _strips_
disallowed tags (and drops `script`/`style` contents wholesale), it
does not turn them into visible text, so an ammonia-only pipeline
fails the "escaped" half of SEC-005; (3) which ammonia config makes
"External resources are not loaded" true (the crate default _allows_
`<img src=http…>` — `Builder::new()` alone violates the sentence);
(4) whether the command needs `spawn_blocking` (SYS-001 names DB,
files, scan, sync — not CPU renders; the generic rule "CPU work →
blocking pool" would disagree); (5) adopting two crates outside every
existing dependency family, with the license/advisory gate that adds;
(6) the response shape — the T-055/T-057 line puts warnings in this
command's response later, and the card demands a shape that can gain
a field without breaking.

Solution: `src-tauri/src/markdown.rs` (`preview_html`, pure
`&str → String`) behind the thin command in `commands.rs`.

- PLACEMENT: src-tauri, beside the command. Spec 5.2's crate table is
  explicit and closed about core — "only `serde`, `sha2`,
  `thiserror`" — and D-017 made that list enforceable at the manifest
  level precisely so a dependency question like this one has an
  answer; §5.3 names `pulldown-cmark` ("Preview and warnings. Not
  used when building files") and SEC-005 names `ammonia` for the app
  without assigning a crate. D-006's "storage, markdown, diff, sync,
  watch" phrasing predates the allow-list and its markdown clause is
  superseded by it. The app crate's dependency set is open (it
  already carries `rusqlite`/`jiff`/`tokio`/`tracing`/`toml`/specta
  beyond the table's "the other crates, `tauri`"), so the renderer
  lands there — a reviewer can check the whole policy by noting that
  `crates/eunomia-core` stayed untouched. The §5.3 "not used when
  building files" note is enforced structurally: `preview_html` has
  exactly one caller (the command), and no assembly/target-file code
  reaches the module.
- ESCAPING: at the pulldown-cmark event level, _before_ rendering —
  `Event::Html`/`Event::InlineHtml` (whole HTML blocks and inline
  tags) are re-typed as `Event::Text`, which `html::push_html`
  escapes into `&lt;…&gt;` visible literals; Markdown image syntax
  gets the same treatment one level up, its `Start/End(Image)` tag
  events dropped so the alt text (the parser's inner content) remains
  and no `<img>` — hence no `src` — is ever generated. Ammonia is the
  SECOND gate (SEC-005 names it as the sanitizer, and it stays as
  defense in depth), not the escaping mechanism: the two gates are
  pinned by separate tests, so removing one cannot silently disarm
  the other.
- AMMONIA CONFIG: crate defaults minus `img` (`rm_tags`) — with no
  `img` in the allow-list, no `src` attribute survives anywhere, so
  no relative/external image policy is even expressible (an `<img>`
  the sanitizer _does_ see — i.e. bypassing the escaping gate — is
  stripped whole). The defaults carry the rest — and they were
  verified against the resolved ammonia 4.2.5 source, not assumed:
  `script`/`style` contents dropped (`clean_content_tags`), HTML
  comments stripped (`strip_comments` already defaults to `true` in
  ammonia 4 — initially added as an explicit call under the wrong
  belief that the default kept comments; removed as a no-op after
  the round-1 review), URL schemes closed to the safe set
  (`javascript:` and friends unrepresentable), every link forced
  `rel="noopener noreferrer"` (`link_rel` — on every surviving
  anchor, href-less ones included), `class` not allowed (the
  code-fence language class is dropped — no highlighting in MVP).
  One deliberate addition: a closed task-list carve-out — `input`
  joins the tags; `disabled`/`checked` are whitelisted as plain
  attributes (booleans, no behavior), while `type` is whitelisted
  ONLY through the value set, because ammonia's attribute check is a
  disjunction (`tag_attributes` OR `tag_attribute_values`) and a
  `type` line in the plain set would pass with ANY value — the
  round-1 review caught exactly that configuration bug (the value
  pin was inert); with `type` value-pinned to `checkbox` (what
  TASKLISTS emits), the extension's checkboxes render, a
  non-checkbox input loses its `type` (and with it its usefulness),
  and only a bare, attribute-less `<input>` — which renders no
  executable or resource-loading behavior (a browser would show a
  default text field, nothing more) — could survive a caller that
  bypasses the escaping gate. Links
  are the one external reference kept: inert in the output (nothing
  loads until clicked), and click navigation is the frontend Preview
  tab's contract, outside this command. D-022's deferred live
  runtime check ("an external image in the preview does not load")
  stays with that frontend task — this task guarantees the content
  side (nothing remote survives sanitization; the IPC tests prove
  it) while the CSP already held the browser side.
- EXTENSIONS: the pulldown-cmark extension bits `ENABLE_TABLES`,
  `ENABLE_STRIKETHROUGH` and `ENABLE_TASKLISTS` — the
  GitHub-flavored basics an instruction text plausibly uses; §5.3 pins the crate, not a dialect. Footnotes,
  math, metadata, heading attributes (they would emit `id`/`class`
  the sanitizer strips anyway) and smart punctuation (it would
  rewrite the author's literal quotes) stay off.
- SPAWN_BLOCKING: no. The established SYS-001 policy (commands.rs's
  section, D-037) routes DB/file/scan/sync work — blocking I/O — to
  `tauri::async_runtime::spawn_blocking`; `app_info` (T-031) already
  established that a command with neither I/O nor state runs inline.
  The render is pure CPU with no lock and no `.await` inside, so
  there is nothing to starve; realistic inputs stay in the
  milliseconds range because the editor's §6.9/DB-006 text ceiling
  (262,144 characters) bounds what the UI sends — a caller-side
  bound the command deliberately does not enforce (the round-1
  review flagged the original wording, which claimed the command was
  bounded; it renders whatever it is handed). The generic "CPU work →
  blocking pool" guidance prices _unbounded_ work, which a live
  editor preview is not. The input is not §6.9-validated on purpose:
  the preview renders whatever the editor currently holds, and the
  save path's validating constructors are where length and reserved
  markers are enforced. The command is stateless (no `AppHandle` —
  the first such command; registered in the bare tauri-specta form),
  answers in any mode, and its `Result<T, AppError>` is §7.10's
  uniform shape, not a claim of fallibility: the pipeline is total,
  no `Err` is reachable, so no §10 log edge exists either.
- DEPENDENCIES: `pulldown-cmark = { version = "0.13", default-
features = false, features = ["html"] }` (0.13.6 resolves; the
  dropped default features take the `getopts` CLI machinery out of
  the build) and `ammonia = "4"` (4.2.5 resolves). Both are NEW
  crates in the graph — unlike the rusqlite/jiff/tokio precedents
  there was no existing version to re-use — and both pass
  `cargo deny check` (pulldown-cmark MIT, ammonia MIT OR Apache-2.0;
  no advisories). Manifest comments carry the spec anchors and the
  placement rationale.
- RESPONSE SHAPE: a named struct `MarkdownPreview { html: String }`,
  not a bare `String` — D-15's warnings list (T-057) joins as an
  additional field in a coordinated bindings regeneration (the
  frontend always regenerates with the Rust change in the same
  commit), never reshaping the wire the UI consumes. The bindings
  pin (`export_bindings_are_up_to_date`) and the lib.rs registration
  pin keep the surface honest in both directions.

Tests: 13 unit tests in `markdown.rs` — the acceptance pair (script
never markup, external image in BOTH spellings never
executable/referenceable; the Markdown spelling leaves no trace of
the URL, the raw spelling survives only as escaped text, upper-case
tags included), benign raw HTML also escaped, the heading/emphasis/
fenced-code render pins (the TAGGED fence exercises the no-class
policy for real — an untagged fence in the first revision never
produced a class for the sanitizer to strip, a round-1 review
catch), the link `rel="noopener noreferrer"` pin, `javascript:` and
`data:` links each reduced to their text (the scheme gate, end to
end — the `data:` leg added after the round-2 review noted the doc
claimed the scheme without a test), the checkbox carve-out, the
sanitizer's own gate driven directly (hostile fragment in — script
payloads, handler, comment, `javascript:` and `data:` hrefs, iframe,
non-checkbox input — and only text, href-less anchors and a bare
`<input>` out, proving the second gate and its carve-out's negative
half independently of the first), and the empty-input answer. 4
command-level tests in `ipc_tests.rs`: the acceptance pair over the
real mock-IPC envelope, escape-and-render over IPC, the empty
result, and the registered-exactly-once pin.

### D-084 — Project-folder availability: four metadata probes in `paths`, hand-rolled typed failure, and the no-drive-heuristic contract — 2026-11-19

Problem: T-060 implements §9.4 preflight step 1 (§6.9's project-path row,
DB-032, D-09): a registered project's folder must be verified available —
exists, is a directory, is readable, contains `.git` — with every failure
later mapping to `E_PROJECT_UNAVAILABLE` and the OS code as detail. Four
points were left open: (1) which `eunomia-fs` module owns the check — the
four skeleton modules (D-019) name none, and adding a fifth is a
module-boundary decision; (2) the error type's shape — the crate's pinned
dependency set (T-003) has no `thiserror`, and pulling one in is a manifest
decision; (3) how the DB-032 contract "network and removable drives are
allowed" can be TESTED when a unit test cannot mount an NFS share — an
assertion that the code "does not look at device numbers" proves nothing on
its own; (4) what the check promises about races — its probes are separate
OS calls, and the card mandates a vanished-`.git` test.

Solution: `check_project_available(root: &Path) -> Result<(),
ProjectUnavailableError>` in `crates/eunomia-fs/src/paths.rs`.

- PLACEMENT: `paths`. The §6.9 row IS a path validation, and the paths
  module's pinned responsibility (T-003 skeleton docs, lib.rs) is exactly
  "the path checks used when validating paths that arrive from the UI".
  `scan` is discovery — ALG-050..054 walk roots and, per ALG-052,
  DISTINGUISH the `.git`-directory from the `.git`-file form (skipping the
  file form); availability needs only "there is a `.git` entry", so the
  distinction stays out. No new module: the D-019 boundary stands, and
  lib.rs's skeleton sentence ("not implemented yet") was updated — this is
  the crate's first implementation — while the paths doc keeps the planned
  canonicalization (DB-030, `dunce`) and SEC-003 work visible.
  ARCHITECTURE.md's tree stays known-stale on purpose (D-082's posture —
  it is outside this card's scope and already carries the known-stale
  marker); lib.rs's module paragraph is the accurate record.
- FOUR PROBES, TOTAL: `stat` of the root (symlinks followed — a project
  root that is itself a link checks the repository), `is_dir`, one probe
  read (`read_dir`'s open is listability, which is what "readable" means
  for a directory — the stricter of the two readings: a search-only
  folder (Unix `x` without `r`) is reported unavailable even though
  named paths inside could be opened, because the app itself lists the
  folder — the repository scan reads entry names, ALG-054 — rather than
  poking fixed paths), then `symlink_metadata(<root>/.git)`. ANY `.git` entry
  counts: a directory, a file (worktree/submodule gitdir pointer), even a
  symlink — presence, not validity; whether the pointer leads to a healthy
  gitdir is git's and the later phases' business.
- CLASSIFICATION: io-backed failures keep an intentionally coarse bucket —
  `NotFound` → `Missing`, every other OS refusal (denied access on the
  folder or an ancestor, a dropped drive's EIO/ENODEV, a symlink loop,
  ENOTDIR) → `NotReadable`, with the `io::Error` itself carried as
  `source`, so `raw_os_error()`/`ErrorKind` ride along for the app layer's
  `ErrorDetails::from_io_error` (§10's log facts). `NotADirectory` and
  `NoGitEntry` carry no io error — no OS call failed. The command layer
  maps every variant to `E_PROJECT_UNAVAILABLE`; the reasons stay typed so
  the add-project flow's `E_NOT_A_REPO` can reuse the check. Accessors
  `io_error()`/`os_code()` expose the details; no `AppError` mapping here
  (D-062's posture). Write access is deliberately absent: that is
  `E_NO_WRITE_ACCESS` (§9.4 preflight step 3), a different check.
- NO-DRIVE-HEURISTIC CONTRACT BY CONSTRUCTION: the four probes are the
  entire decision — there is deliberately no seam for a drive-type filter
  to plug into. No device numbers (`MetadataExt::dev`), no mount tables,
  no volume flags, no path-shape sniffing (`/mnt/…`, `\\UNC\…`): NFS,
  USB, UNC and tmpfs repositories pass exactly like local ones.
- THE REMOTE-DRIVE TEST SEAM (why it is not tautological): a unit test
  cannot mount a network share, so the test pins the contract on the
  mechanism every "reject non-local volumes" shortcut must consult —
  mount identity. `/dev/shm` is a separate tmpfs mount with its own
  `st_dev`; the test stat-compares its device number against the temp
  dir's, builds a repository fixture there, and requires the PUBLIC check
  to accept it — a heuristic keyed on device numbers or mount identity
  would fail this test. It skips loudly when `/dev/shm` is missing or
  (exotically) sits on the same filesystem as the temp dir. tmpfs is not
  NFS, and no unit test can make it be; the residual gap is named here
  rather than papered over. The portable second half pins path-shape
  blindness with no skip at all: repositories under a `$RECYCLE.BIN/`
  parent (ALG-051's exclusion names are scan rules, not availability
  rules) and inside a `mnt/` component must pass.
- TOCTOU POSTURE, DOCUMENTED NOT PRETENDED: the probes are separate OS
  calls over a live filesystem; the function reports what they observed
  at their moment — no snapshot is taken and no atomicity is claimed.
  The card's vanished-`.git` test pins the flip (present → `Ok`, removed
  → `NoGitEntry`); availability is recomputed at every startup,
  window-open and sync (DB-032), so a flip between two checks is simply
  the next check's verdict. A plain file replacing the folder mid-check
  surfaces as `NotReadable` carrying ENOTDIR.
- ERROR TYPE HAND-ROLLED (the D-017 store precedent; the manifest is
  untouched): `#[derive(Debug)]` enum + `Display` (lowercase, no trailing
  punctuation) + `std::error::Error` whose `source()` is the carried
  io error. Tests reuse the already-pinned `tempfile` for fixtures — it
  was parked for the atomic module, and using it in tests adds nothing to
  the manifest.
- PERMISSION TESTS, ROOT-HONEST: the two chmod tests (mode 000 → the
  listing denied; mode 400 → listing ok but the `.git` stat denied — the
  two distinct `NotReadable` producers) skip loudly via `eprintln!` when
  the fixture's owner uid is 0. The uid check is std-only:
  `MetadataExt::uid()` on our own fixture equals the creating process's
  effective uid (the store's `running_as_root_is_untestable` precedent);
  a libc `geteuid` would be a new manifest decision. Mode bits are
  restored BEFORE the assertions run, so a panic cannot strand an
  unreadable fixture under TempDir's teardown.

Tests: 13 in `paths.rs` — the happy `.git`-directory; the `.git`-file form
accepted; a plain file → `NotADirectory` (with `os_code() == None`); a
missing path → `Missing` (kind `NotFound`, ENOENT = 2 pinned on unix);
mode-000 → `NotReadable` with EACCES = 13 and mode-400 readable-but-
unsearchable → `NotReadable` with EACCES = 13 (both root-skipped loudly;
the mode-400 test first proves the listing still succeeds, so its failure
can only come from the denied `.git` stat); the vanished `.git` flip; a
`.git` symlink accepted; a symlinked root checked through (and a dangling
root link → `Missing`); ENOTDIR = 20 riding in the typed error
(root-proof); the Display texts and the `source()` chain pinned (exact
unit-variant texts, io-variant prefixes, `source()` Some/None); the
cross-filesystem `/dev/shm` acceptance (skip-able, including an unusable
`/dev/shm`); and the `$RECYCLE.BIN`/`mnt` path-shape acceptance.

### D-085 — Markdown preview warnings: one parse, offsets-based unclosed-fence detection, the minimal taxonomy; additive response shape — 2026-11-19

Problem: T-057 implements the warnings half of D-15 ("validation gives
warnings only (for example, an unclosed code fence) and does not block
writing") and §7.3 LIB-002/009…016 ("Markdown warnings (for example, an
unclosed code block) that do not block saving"): `markdown_preview` must
return a warnings list beside the sanitized HTML. Five points were open:
(1) the taxonomy — the spec pins the category (warnings, non-blocking)
and names ONE kind by example, leaving the rest of the set open;
(2) DETECTION — pulldown-cmark's event stream does not mark an unclosed
fence: closed and unclosed blocks pair `Start(CodeBlock)` /
`Text…` / `End(CodeBlock)` identically (verified empirically against
0.13.6 before designing), so "detect it from the event stream" needed a
real invariant, not a read of event kinds; (3) the card's "no second
parser over the text" bites the moment the command needs HTML and
warnings from the same text; (4) the response shape must extend T-056's
named struct additively, with the bindings regenerated in the same
commit; (5) what "enough for the UI to point at the problem" means.

Solution: the pipeline in `markdown.rs` becomes
`preview(text) -> Preview { html, warnings }` — ONE parse serving both
outputs.

- TAXONOMY, MINIMAL AND SPEC-FAITHFUL: the single kind
  `MarkdownWarningKind::UnclosedCodeFence` (wire string
  `unclosed_code_fence` via `serde(rename_all)`, the ErrorCode
  wire-contract rule — the strings are a contract for the editor UI,
  pinned by the bindings). The spec names exactly one kind by example;
  inventing more kinds (unbalanced emphasis? undefined links?) would be
  guessing UI for problems nobody asked about — the enum is closed, and
  a future kind is an additive variant plus its own decision.
  Non-blocking BY CONSTRUCTION: a warning is response data; the save
  path reads none of it; the command keeps §7.10's `Result` shape with
  no reachable `Err` (D-083's totality holds — detection cannot fail),
  and `errors.rs` is untouched: warnings are not errors, no §10 code
  exists or is needed.
- DETECTION, OFFSETS NOT EVENT KINDS: the tracker observes the same
  stream the renderer consumes — `Parser::into_offset_iter()`, the
  same parser with the same options (the transformed event sequence
  `push_html` receives is bit-for-bit D-083's; the exact-HTML test
  with a warning present pins it). The invariant, in its final (gap)
  form after the round-1 review: a fenced block is unclosed UNLESS the
  source between its content's end and the block's own end holds a
  closing fence — a fence line is never whitespace, so an empty or
  whitespace-only gap means no closer was ever consumed: the block ran
  to end-of-input, or an enclosing container broke it first (a list
  item's next bullet, a blockquote's end — pulldown ends the block
  there with no closer). Furthest-end tracking survives CRLF input,
  whose in-fence content arrives as split text pieces. The
  closing-fence rules (same fence character, at least as many
  characters, nothing else on the line) are pulldown-cmark's and are
  NOT reimplemented: a too-short or wrong-character line stays
  _content_ and keeps the content running to the block's end — exactly
  the warning case, for free. Two pulldown specifics the gap form had
  to survive: the degenerate one-or-two-line blocks with no content
  events at all (`"```"`, `"```\n"`, `"```\n```"`, `"```\n```\n"`) are
  decided by a bounded read of the block's own source range (closed ⟺
  a line follows the opening fence line inside it), and — the round-1
  review's catch, reproduced before fixing — a final
  non-newline-terminated line of 1–3 spaces is a synthesized
  zero-length item with NO content event, so the first (equality)
  wording of the invariant missed the warning there; the gap form
  (whitespace-only gap ⟹ unclosed) covers it, with the
  closed-fence-with-trailing-whitespace-after-the-closer shape pinned
  as the no-false-warning half. Indented code blocks emit
  `CodeBlock(Indented)` and never warn — even when their lines look
  like fences, which CommonMark itself declares content.
- POSITION: `line: u32`, 1-based, of the OPENING fence — the thing an
  author fixes — counted over the raw text's bytes. A byte offset was
  rejected: the editor's textarea is line-addressable, and an offset
  would drag a second coordinate system (UTF-16 units in JS) into a
  wire contract for no requirement. The counting is cursor-based (the
  round-1 review measured the per-warning full rescan at ~483 ms on a
  max-size §6.9 draft full of container-broken fences): warnings
  arrive in source order, so each count folds only the bytes since the
  previous warning into a running line cursor — O(text) once in
  total, not per warning — and the counter saturates (`try_from`
  ceiling + `saturating_add`) instead of overflowing: the original
  +1 arithmetic was the one panic-capable spot on a documented-total
  path.
- RESPONSE SHAPE, ADDITIVE: `MarkdownPreview` gains
  `warnings: Vec<MarkdownWarning>` after the unchanged `html` field —
  the coordinated bindings regeneration D-083's named struct was
  shaped for; `MarkdownWarning { kind, line }` is specta-typed and
  travels as `MarkdownWarning[]`/`MarkdownWarningKind` in
  `src/bindings.ts`, drift-pinned as before. Nothing about the html
  wire the UI consumes moved.
- STRUCTURE: `preview_html` (`&str → String`, D-083) is renamed
  `preview` — the old name promised a bare string and the function no
  longer returns one; the rename is recorded here as part of the same
  decision. The T-056 unit tests now assert on `.html` of the same
  pipeline; `sanitize`, the escaping transform and the render are
  byte-for-byte unchanged.
- A BUG THE TESTS CAUGHT, RECORDED: the first implementation compared
  the absolute content end against the RELATIVE slice length
  (`text[start..end].len()`) — fences at source offset 0 passed
  (where absolute and relative coincide) and every fence opening after
  content silently produced no warning. The acceptance test (fence on
  line 3) went red before anything else touched the code; fixed by
  comparing against the absolute block end. The lesson is structural:
  offsets must never be mixed across coordinate systems in one
  comparison — the invariant's doc comment now names both offsets
  absolute explicitly.

Tests: 14 warning-half unit tests in `markdown.rs` — the unclosed fence
warning at its opening line 3 (with the swallowed text still rendering
as a code block); clean text (heading, emphasis, CLOSED fence,
paragraph) warning-free; unclosed at EOF without a trailing newline;
the shorter-closer and foreign-fence-character lines that cannot
close; the unclosed tilde fence; the four degenerate one-line forms in
both polarities; container breaks (list item and blockquote) warning
at the opening line with their closed-fence twins pinned quiet; the
whitespace-only tail line (the round-1 catch, with tab / 4-space /
trailing-newline boundary shapes and the
closed-with-trailing-whitespace no-false-warning half); two unclosed
fences in source order (lines 1 and 3); the serde wire literal pinned
directly (`unclosed_code_fence`, the errors.rs rule); indented code
blocks (plain and fence-shaped) never warning; CRLF input in both
polarities; one warning per unclosed fence wherever it opens
(closed-then-unclosed, opening line 7 pinned); and the exact-HTML pin
proving warnings and HTML are independent outputs (the render
byte-for-byte D-083's while a warning fires). The empty test now pins
the whole `Preview` (empty html, empty warnings). 2 new command-level
tests in `ipc_tests.rs`: the unclosed fence over the mock-IPC envelope
(one warning, kind and line intact on the wire, the command still
`Ok`) and clean text warning-free over IPC; the empty IPC test gained
the warnings-empty assertion. Review round 1
(opencode-go/deepseek-v4.1-flash, verdict approve-with-fixes): P2-1
(the whitespace-tail miss) fixed with the gap-form invariant and its
regression test; P2-2 (closed-fence-in-container coverage) added to
the container test; P3-1 (quadratic line counting, the +1 overflow)
fixed with the cursor and saturating arithmetic; P3-2 (doc wording and
the test count) corrected here; P3-3 (the wire literal) pinned by the
new serde test; P3-4 (lone-CR line endings) declined — §6.9
normalizes CRLF→LF on save and real editors emit LF/CRLF, so a lone-CR
document is not a shape the editor produces, and the warning still
fires with a correct kind there (only the line number could differ in
an input the save path would normalize anyway). Full suite: 798 tests
green; fmt, clippy `-D warnings`, `cargo deny check`, pnpm lint /
format:check / typecheck / test all green.

### D-086 — Project path canonicalization and the `path_key`: dunce stored verbatim, NFC + simple-fold key per OS, the two-escalation dependency saga, and the format-stability contract — 2026-11-19

Problem: T-058 implements DB-030/DB-031/D-07 in `crates/eunomia-fs::paths`
(`canonicalize`, `canonical_path_key`, `path_key`, `path_key_with_rule`,
`PathKeyRule`, `CanonicalizeError`). The spec pinned the mechanics
(`dunce::canonicalize`, `/` separators, no trailing separator, per-OS
normalization) but left five points open, three of which needed owner
rulings across two escalations: (1) the normalizer dependency — §5.2's fs
row named no Unicode crate while DB-030 mandated Unicode NFC and case
folding, and std has neither (no normalization, and `to_lowercase` is a
lowercase MAPPING, not folding: İ, ſ and the mathematical-alphanumerics
block differ); (2) WHICH folding — simple vs full case folding vs
lowercase, each provably different on real names; (3) whether NFC applies
to the stored path or only to the key; plus (4) two factual corrections
surfaced during verification, and (5) two unspecified corners (the fold/
NFC pipeline order, and Unix non-UTF-8 path bytes).

Solution:

- THE DEPENDENCY SAGA (two escalations, both resolved by owner rulings
  recorded on the INBOX card and then in spec commits): escalation 1 —
  §5.2 pinned the fs set without any normalization crate, and D-017/D-082
  forbid inventing deps → the owner ruled (2026-11-19): NFC + SIMPLE case
  folding per OS (the filesystem-faithful rule), stored path verbatim
  (reading A), and named `focaccia` as the fold crate (spec commit
  1e1b930). Verification killed that premise: focaccia 2.6.0 exposes only
  `CaseFold { Full, Ascii, Turkic, Lithuanian }` — no simple variant, no
  feature flags — and is comparison-only (`casecmp`/`case_eq`), so it
  cannot even materialize a folded string, which a stored TEXT key needs.
  Escalation 2 landed the swap the ruling's semantics require: the owner
  adopted `unicode-case-mapping` 1.0.0 (Apache-2.0, already allow-listed;
  zero deps; yeslogic) whose `case_folded(char) -> Option<NonZeroU32>` IS
  simple folding by construction (single-code-point signature; `None` =
  maps to itself), with `unicode-normalization` 0.1 (Unicode-3.0, already
  allow-listed) for NFC (spec commit 9a953d9; §5.2's fs row now names both
  and focaccia is out). Recess lesson recorded for crate recommendations:
  verify a candidate's actual API before naming it in a ruling — the
  focaccia detour cost one full escalation round.
- THE FOLD IS SIMPLE, AND THAT IS LOAD-BEARING (ruling 2, 2026-11-19):
  simple = exactly the 1:1 mappings (CaseFolding.txt statuses C and S).
  Full folding (F entries) would unfold ligatures — ﬁ→fi, ß→ss — and
  thereby MERGE folder names the target filesystems keep distinct (APFS
  simple-fold, NTFS `$UpCase`), colliding two real projects on one unique
  `path_key`. Both sides are pinned in tests: ﬁ≠fi, straße≠STRASSE,
  İstanbul≠Istanbul; AND ſ→s, ẞ→ß, KELVIN SIGN→k (1:1 mappings do fold).
  The spec's committed text ("1:1 mappings only") governs over the chat
  ruling's stray "ſ stays distinct" phrase — ſ carries a C entry
  (`017F; C; 0073`, verified against UCD CaseFolding.txt) and both
  filesystems fold it; the card comment is the erroneous one. ẞ (S entry,
  folds to ß while its full fold is ss) is the counterexample that kills
  the tempting "derive simple from full-fold-if-1-char" shortcut — the
  shortcut is provably NOT simple folding, which is also why `caseless`
  (full-fold iterators only) was rejected without escalation.
- READING A (ruling 1): the stored `projects.path` is the verbatim
  `dunce::canonicalize` output; NFC and folding apply to the KEY only.
  Pinned by a byte-level test: a decomposed folder name stays decomposed
  in the canonical path on Linux while the folding rules merge its
  spellings in the key. Dunce's own safety rule is part of the verbatim
  contract (review round 1): the `\\?\` prefix is stripped only where
  Win32 could open the stripped spelling — plain drive paths always
  strip; verbatim UNC shares (`\\?\UNC\…`), paths beyond 260 characters
  and reserved DOS component names keep it (dunce 1.0.5's
  `is_safe_to_strip`), so DB-030's parenthetical reads as dunce's
  best-effort rule, not an unconditional claim. Identity is unaffected
  (both spellings of such a folder canonicalize to the same verbatim
  form); the key of a verbatim UNC spelling is pinned (`//?/unc/…`).
- THE KEY PIPELINE: NFC → simple fold → NFC, per module docs. Folding
  does not preserve normalization formats (CaseFolding.txt's own header),
  so the folded text is normalized again; the leading NFC makes decomposed
  spellings fold like their precomposed twins (pinned: NFC and NFD
  spellings of café and École collapse under Windows/macOS rules and stay
  distinct under Linux, where they ARE two different folders). The
  pipeline is a pinned fixed point (key of a key = the key) and total over
  arbitrary text.
- SEPARATORS ARE RULE-SCOPED: `\` is a separator on Windows and a legal
  NAME byte on Unix, so the Windows rule splits on both `/` and `\` while
  Linux/macOS split on `/` only — a Unix folder `a\b` never collides with
  a sibling `a/b` (pinned). Leading markers survive: `/` keys to `/` (not
  the empty string) and a UNC `//server/share` keeps its double marker;
  interior/trailing runs collapse.
- TEXT KEYS OVER BYTE PATHS: Unix paths are bytes, the key is a SQLite
  TEXT column — non-UTF-8 bytes reach the key as U+FFFD via
  `to_string_lossy` (the same lossy form logs and dialogs show), pinned so
  a silent switch to panic/drop cannot slip in. The documented cost
  (review round 1): two byte names differing only in invalid-byte
  sequences collapse to one key, so the second add hits the UNIQUE column
  and a scan can classify such a folder as a known project — whether to
  reject non-UTF-8 paths at the add/scan seam instead is a
  format-contract decision left with the owner, not made here.
- FORMAT-STABILITY WARNING (the D-082 posture for snapshot hashes):
  `path_key` is stored (`projects.path_key`, `project_exclusions.path_key`,
  both UNIQUE per DB-015) and compared across app versions — the rule set
  (simple-fold flavor, NFC, separator shape, lossy-UTF-8 handling) is now
  a format contract. Changing any of it later silently splits or merges
  project identities; a change is a DECISIONS.md event, and existing rows
  would need a migration, not a rebuild-on-read.
- BOUNDARY AND OWNERSHIP: `canonicalize`/`canonical_path_key` are the
  entry points the add/relabel flows will call when they land (a later
  task); `path_key` keys an
  ALREADY-canonicalized path (the scan's use, ALG-053 — real paths by
  construction since the scan follows no links, D-07) and is total; the
  pure `path_key_with_rule` + public `PathKeyRule` keep the spec's per-OS
  wording visible in the API and give every rule unit coverage on every OS
  (Linux here, Windows/macOS literals on the CI matrix). `platform_rule()`
  stays pub(crate) — the selection is internal, the rule enum is the
  surface. `CanonicalizeError` is hand-rolled (path text + io source +
  `os_code()`), mirroring D-084's `ProjectUnavailableError` shape; no
  `thiserror` — the crate's §5.2 list has none (D-017 store precedent).
  Availability (D-084) is untouched; DB-031's aliasing lives in
  canonicalize-first keying, as D-084's symlink test comment anticipated.

Tests: 21 unit tests + 5 doctests in `paths.rs` — the dotted/relative
canonical form equality; symlink resolution to the target form and the
Windows-leg twin (skip only on capability absence: unsupported/EPERM/
ENOSYS/ENOTSUP on Unix, ERROR_PRIVILEGE_NOT_HELD = 1314 on Windows — any
other fixture failure panics, so DB-031 coverage cannot silently vanish);
the typed canonicalize failure (path text, ENOENT = 2, Display prefix,
source chain) and the ENOTDIR under-a-plain-file case; the verbatim stored
path (decomposed name survives byte-for-byte); the Windows-leg verbatim
prefix strip with the drive-letter key shape; ASCII folding per rule;
Windows `\`+`/` rewriting with trailing-separator drop; the Unix backslash
identity; separator-run collapse with `/` and UNC markers surviving plus
the verbatim-UNC key spelling dunce actually hands back; the NFC/NFD
collapse pairs under Windows/macOS and their Linux distinctness; the
simple-fold fidelity battery (ﬁ/ß/İ stay; ſ/ẞ/KELVIN fold); the
fixed-point property over a mixed battery; total posture (no trailing
separator, no `\` under the Windows rule, empty text OK); the Linux
platform-wrapper literal (Windows/macOS literals cfg-gated for the CI
matrix); the lossy U+FFFD contract; and the card's acceptance test —
one real folder, a symlink to it, ONE `path_key` through both
`canonical_path_key` and the canonicalize+key halves, with the
`assert_ne` proving only canonicalization collapses the alias.

### D-087 — The repository scan walker: probe-based ALG-052 classification, Quit-token cancellation, and parameters-not-configuration — 2026-11-19

Problem: T-061 implements the folder traversal of §9.6 (ALG-050..054,
PRJ-007, D-04..D-07) as the walker ONLY — path_key/classification is
T-075, progress events and the cancel command are T-076 — and four
design points were open: (1) how a folder is classified as repository /
submodule-or-worktree when `.git` is ON the default exclusion list
(D-05), which makes "watch for `.git` entries in the walk's own
listing" self-defeating; (2) whether ALG-054 ("reads only directory
entry names and does not open files") forbids that classification
syscall entirely; (3) the cancellation primitive — no tokio in the
allow-list, and SYS-003 names a token for BOTH scan and sync; (4) what
the walker yields when the parallel walk's order is unspecified, and
how PRJ-002's "folders checked" counter survives the walker-only scope.

Solution: `scan_repositories(root, options, cancel, on_event) ->
ScanOutcome` in `crates/eunomia-fs/src/scan.rs`, on `ignore`'s parallel
`WalkBuilder` with threads `min(4, available cores)`
(`available_parallelism`, std), depth passed one-for-one to
`max_depth` as `NonZeroUsize` (ruling out the unlimited-walk reading of
an unvalidated value), every ignore-file source AND the hidden filter
off (the spec's only skip rules are the explicit names of ALG-051; a
blanket hidden filter would hide repositories the spec wants found),
`follow_links(false)` (junctions surface as symlinks in std's file
type, so the same gate covers ALG-050's reparse points).

- PROBE-BASED CLASSIFICATION: every traversed, non-excluded directory
  gets exactly one `symlink_metadata(<dir>/.git)` (the D-084 idiom):
  `.git` DIRECTORY → Repository event, walk CONTINUES inside (ALG-051's
  nested-repositories rule); any other `.git` kind — file or symlink —
  → Skipped("submodule or worktree"), NOT descended (D-06: "you can add
  it by hand"; ALG-051's continue-inside rule is written for found
  repositories, not skips); NotFound → plain Directory event, walk
  descends; any other OS refusal → Skipped(Access) with the error, NOT
  descended. ALG-054 is honored, not violated, by the probe: it never
  opens a file and no content can surface through the API — and the
  listing-watching alternative is WRONG under default settings, because
  exclusion is a traversal rule: `.git` on D-05's list would stop the
  `.git` entry from ever being listed, so no parent could ever be
  classified a repository. Exclusion must gate the DESCENT, not the
  DETECTION; the probe keeps the two rules independent. One stat per
  directory (never per file) is the walker's only syscall beyond the
  walk itself; the unreadable-file test pins the observable half of
  ALG-054 (a chmod-000 FILE produces no event), and the syscall-negative
  is pinned by API shape — no event variant could carry contents.
- EXCLUSION PRECEDENCE: an excluded folder is skipped, not descended,
  and NOT probed — exclusion wins over detection even when the folder
  holds a `.git` directory (this is what makes D-05's list coherent).
  The scan ROOT is exempt from the name filters (the user chose it
  explicitly); comparison is exact and case-sensitive (the list is user
  data, the spec pins no folding). System folders (`$RECYCLE.BIN`,
  `System Volume Information`, `.Trash`, `.Trashes`) fold into the same
  `SkipReason::Excluded` — PRJ-002 shows both as "excluded".
- ACCESS ERRORS ARE DATA: every per-entry failure — walker-reported
  (unreadable root, failed listing) or probe refusal — becomes
  `ScanEvent::Skipped` with `SkipReason::Access { source: io::Error }`
  reconstructed from `ignore`'s error WITHOUT moving or unwrapping it
  (round-2 review F-1 records the mechanism the code actually uses,
  after the round-1 P3-3 fix removed the guard+expect pair): the
  borrowed `error.io_error()` is inspected and the source is rebuilt —
  `from_raw_os_error` for OS-backed errors (the exact `raw_os_error`,
  and with it std's kind and strerror text), `Error::new(kind, text)`
  otherwise; the non-io shapes are unreachable with all ignore sources
  off, but a synthetic `io::Error::other` fallback keeps the arm
  total. The traversal continues (PRJ-007); `scan_repositories` is
  total, no `Result`. `ignore::Error` never leaks into the API
  (boundary discipline; its errors carry no `path()` accessor — the
  path comes from a private recursive match over the public wrapper
  variants). Documented corner: a searchable-but-unlistable folder
  (Unix `x` without `r`) yields its classification AND an access skip
  for the same path — both facts are true (pinned by a test).
- TOKEN PRIMITIVE: `CancelToken` — a `Clone` `Arc<AtomicBool>` wrapper
  with `cancel()` (Release) / `is_cancelled()` (Acquire), the weakest
  correct pair since nothing else synchronizes on the flag. No new
  dependency; no tokio-util (not in the allow-list); nothing to reuse in
  the workspace (grepped). Checked FIRST on every entry — found, plain,
  skipped, and walker errors alike — and answered with
  `WalkState::Quit`, `ignore`'s whole-walk quit primitive, so
  termination is prompt and observable (`run` returning). The outcome
  is exact, not sampled: the visitor records its quit in a shared
  `quit_observed` flag at the moment it answers `Quit`, and the outcome
  derives from that — a flip racing with the last entry (after every
  check) reports `Completed`, because nothing was interrupted and every
  result is whole (round-1 review P3-1 ruled the naive final re-read of
  the token a doc-contract defect: it could cancel a fully completed
  walk). Keeping or dropping in-flight events is the command layer's
  PRJ-002 rule. The type lives in `scan.rs` but is named for SYS-003's
  both-consumer scope: the sync task should reuse or relocate it, not
  grow a second one.
- YIELD SHAPE: `ScanEvent::{Repository, Directory, Skipped}` — the
  `Directory` variant is the walker-only answer to PRJ-002's "folders
  checked" counter (the consumer counts events; no throttled progress
  here, that is T-076). Callback receives `&ScanEvent` (counters pay
  nothing; storers clone) and is `Fn + Send + Sync` (shared across
  walker threads via the per-thread visitor factory). Ordering is
  UNSPECIFIED beyond parent-before-own-contents (structural); tests
  assert sets, never sequences.
- PARAMETERS, NOT CONFIGURATION: `ScanOptions { max_depth:
NonZeroUsize, exclude_names: Vec<String> }` is built by the command
  layer from the validated §6.5 keys; no `Default` (duplicating §6.5's
  defaults here would drift); no `config.toml`/DB/settings reads in the
  module (scope boundary).
- MANIFEST UNTOUCHED: the parked `ignore = "0.4"` covers everything
  (WalkBuilder, WalkState::Quit, DirEntry::file_type); no feature flag
  needed; `walkdir`/`tempfile`/`dunce`/`eunomia-core` stay idle as
  pinned (tempfile already used by tests per the T-060 precedent). No
  `Cargo.lock` change. lib.rs's module-plan sentence ("scan ... still
  skeletons") is left stale on purpose: the file is shared with a
  concurrent sibling task, and scan.rs's own module docs are the
  accurate record (the D-082 known-stale posture); the orchestrator's
  merge can refresh the sentence.
- TESTS: 18 in `scan.rs`, colocated, through the public API on real
  temp trees: the two acceptance pairs — nested repositories found
  separately, unreadable folder does not stop the traversal (sibling
  repo still found, EACCES = 13 carried on the skip), pre-cancelled
  token yields nothing, mid-walk flip on the FIRST yielded entry
  interrupts with strictly fewer than all deliveries (deterministic:
  each thread checks per entry and delivers synchronously, so the
  in-flight window is bounded by the thread count) — plus the battery:
  `.git`-file skip with the exact reason and no descent, excluded names
  (skipped, not descended, not classified despite `.git`), the four
  system folders, the depth bound, symlink non-following (external repo
  not found, no event for the link), `.gitignore` blindness, root as a
  repository and root bypassing the name filters, unreadable root (one
  skip, clean completion), searchable-but-unlistable two-event corner,
  unreadable file never reported, Display/`os_code()`/`source()`
  contract. Permission tests skip loudly under root (the paths.rs
  uid-check precedent); assertions are on sets, never sequences
  (parallel delivery is unordered). Round-1 additions: the three
  untested builder switches pinned (`.ignore` files, the hidden filter,
  parent-level rules — the P2-1 finding that half the ALG-050
  configuration had no failing test) and the root-symlink exception
  pinned; the mid-walk assertion tightened to the exact
  `delivered <= walk_thread_count()` bound (the reviewer verified it
  against `ignore`'s synchronous per-thread delivery).

### D-088 — Stale app tmp cleanup: `temp_files` module, the pinned staging-name grammar, and the sweep's conservative deletion semantics — 2026-11-19

Problem: T-065 implements spec 9.4 phase 1 step 7 ("delete stale app
temporary files in the target folder (older than one hour)") — the
cleanup for the debris a killed process leaves behind (9.5's staging
file, T-F2: the original file is intact; on the next sync, the temp
file is deleted). Six readings the spec leaves open: where the module
lives (the T-003 module plan names scan/paths/backup/atomic, and the
atomic module is T-066's); the token alphabet for "8 random
characters"; whether the sweep keys on the target file's name; the
strictness of "older than one hour"; what a per-file failure and a
missing folder mean for the caller; and how symlinks and races are
handled.

Solution: a NEW module `crates/eunomia-fs/src/temp_files.rs` — a
deliberate D-019 boundary change (the plan's four modules own scan/
paths/backup/atomic concerns; the staging-file NAME convention is
neither of those, and it must exist before the T-066 writer lands so
the writer can only spell names the cleanup recognizes). `lib.rs`
gains exactly one `pub mod temp_files;` line; the `//!` prose list is
deliberately untouched (D-062's known-open-item posture — parallel
tasks edit that block only when it is next free).

The API is three functions and their types. (1) `tmp_file_name(
target_name, token)` — the ONLY spelling of a staging name: `.` +
target name + `.eunomia-` + token + `.tmp`, the token validated as
exactly 8 lowercase-hex characters. (2) `is_app_tmp_file(name,
target_name)` — the matcher, byte-exact and case-sensitive: the
writer only ever writes lowercase hex, so an uppercase spelling is
somebody else's file (a looser match would widen what a sweep may
delete). (3) `remove_stale_tmp_files(folder, target_name, now)` —
the sweep. The builder/matcher symmetry is pinned by tests: a name
the builder produces always matches its own target and never a
different one, so staging debris can never be unrecognizable to the
cleanup.

TOKEN ALPHABET: the spec's "8 random characters" is pinned to
lowercase hex `[0-9a-f]` (`TMP_TOKEN_ALPHABET`, `TMP_TOKEN_LEN = 8`),
derived from the repo's conventions — `hash.rs` renders every digest
as lowercase hex and D-080 already fixed the 8-char short-id form as
the first 8 lowercase-hex characters of a SHA-256; one alphabet
across all app-generated short identifiers, no second convention.
16^8 ≈ 4·10^9 tokens are ample because the writer creates with
`create_new` (9.5 step 2) — a collision is an error and a retry, not
corruption. Hex is also the safe deletion choice: it excludes
uppercase letters, so `.X.eunomia-ABCDEFGH.tmp` stays foreign.

KEYING: the sweep keys on the target file's own name — a project
removes only ITS target's staging files (`.CLAUDE.md.eunomia-*.tmp`
stays when `AGENTS.md` is swept). The spec's `<name>` in 9.5 step 2
is the file being written, and step 7's "app temporary files in the
target folder" is read through that same grammar; per-target keying
is the conservative reading and satisfies T-F2 (the next sync of the
same target deletes the debris).

STALENESS: strictly older than one hour — `now − mtime > 1h`; a file
exactly one hour old stays until the next sweep. "Older than"
is a strict inequality, and deletion ambiguity resolves toward not
deleting. The mtime is the entry's OWN (`DirEntry::metadata`, no
symlink traversal); a future mtime (clock skew) fails
`duration_since` and is never stale. `now` is an explicit
`SystemTime` parameter — time is a parameter, never a clock
(D-061's posture); the pipeline reads the clock once per run.

DELETION GATES: an entry is deleted only when the name matches for
that target AND the entry is a regular file AND it is stale. A
directory or a symlink with a tmp-shaped name was not created by the
writer (which makes regular files only) — foreign, left alone; the
symlink is never followed when reading metadata (D-07's not-followed
posture, ALG-054's names-and-metadata-only rule — the sweep never
opens file contents). One listing level, never recursive: 9.5 stages
in the same folder as the target, so deeper entries cannot be the
writer's. Non-UTF-8 entry names cannot match a `&str` grammar:
foreign by construction.

FAILURE SEMANTICS: a missing folder (`NotFound` on the listing) is a
successful no-op with an empty report — phase 1 step 7 is
housekeeping between the availability check (step 1) and the write
(phase 3), it must never fail a run, and "no stale tmp files in a
folder that does not exist" holds vacuously; a folder that cannot be
listed at all is the typed `TmpSweepError::ListingFailed` (OS code
riding in the source, `io_error()`/`os_code()` accessors in the
`paths.rs` shape); per-file failures (denied stat or unlink) are
collected in `TmpSweepReport::failed` and the sweep continues — one
unremovable file must not stop the cleanup (phase 1's "skipped
projects do not stop the run" spirit), and the caller logs the
paths. An entry that vanishes between listing, stat and unlink is
convergence, not failure: the goal state already holds, it is
silently skipped. TOCTOU is honest: a listing + stat + unlink over a
live filesystem pretends no snapshot; a staging file created after
the listing is at most one sweep away (the writer deletes its own
temporary on every error path, 9.5 step 7).

The atomic-write implementation itself is NOT here — `atomic.rs`
stays T-066's; this module only fixes the name it must use.
COORDINATION RULE FOR T-066: `target_name` is the RESOLVED target
file's name — 9.5 step 1 resolves symlinks before staging, so a
`CLAUDE.md` link stages (and is swept) as `AGENTS.md`; the sweep and
the writer must hand the same spelling to the grammar, or each
spelling's debris waits for the other target's sweep. Deliberately
no new dependencies (the crate manifest is untouched; `std::time::
SystemTime` is std, and `tempfile` is already a crate dependency —
pinned by D-019 for the atomic module — serving the tests). The
builder rejects a target name that is not a single plain file name
(separator, NUL, `.`/`..` — it would stage in another folder, which
the one-level sweep can never reach), and one shared predicate
(`is_plain_file_name`) gates BOTH the builder and the matcher, so
the matcher matches nothing for a target the builder could not name
— a contract-violating caller cannot widen what a sweep may delete.
Testing: 34 colocated test functions (33 run on Unix CI legs, 1
Windows-gated for the platform's backslash separator) — the matcher
near-miss battery (missing dot, extra
suffix, wrong length, non-hex, uppercase, another target, extra
prefix, empty token), builder validation and verbatim grammar
spelling, and real-folder sweeps for the card's acceptance (stale
deleted, fresh stays, foreign stays — with stale mtimes), the exact
1-hour boundary and 1-second-past, multiple stale files, missing and
empty folders, a subfolder not swept, a future mtime, and Unix-gated
permission-collection (two stale files failing in an r-x folder —
collecting BOTH proves no abort; loud root skip), symlink-and-dangling
skip with a stale target (fails any implementation that follows the
link), a stale tmp-shaped directory kept with its content, and a
non-UTF-8 name never deleted. Mutations verified the battery bites:
widening `STALE_AFTER`, case-folding the token alphabet, aborting at
the first per-file failure, following symlinks in metadata, and
dropping the regular-file gate each fail exactly their test.
Round-1 review (deepseek-v4.1-flash) forced the `#[cfg(unix)]` gate
the listing-failure test had missed (proven by a Windows-target
compile check), the target-name validation, the empty-target
matcher guard, the mid-stream report-discard documentation, and two
D-088 wording fixes; round-2 review (hy4-flash-free) approved with
the shared `is_plain_file_name` symmetry guard and this count
correction as its only outcomes.
`paths.rs`/`scan.rs`/`atomic.rs`, both manifests and `Cargo.lock`
untouched. Full suite: 831 tests green (33 new on Unix, 798
before); fmt,
clippy `-D warnings`, `cargo deny check`, pnpm lint / format:check /
typecheck / test all green, plus a Windows-target compile check of
the crate's tests (`cargo check --target x86_64-pc-windows-msvc
--tests`, round-1 review finding: the listing-failure test had
missed its `#[cfg(unix)]` gate). The separator validation follows
`std::path::is_separator` (platform-defined: a backslash is a
separator on Windows, an ordinary file-name character on Unix —
pinned per platform). Known open items, D-062's
posture: lib.rs's `//!` module list, ARCHITECTURE.md's `eunomia-fs`
tree (which also still shows `paths` as a skeleton) and the
`atomic` skeleton's doc block all deliberately did NOT gain a
`temp_files` entry — parallel tasks (T-058/T-061/T-066) edit those
blocks; reconcile the lists when the files are next free.

### D-089 — The data-directory free-space check: the rustix probe, the injected free-space source, and the Windows escalation — 2026-11-19

Problem: T-064 implements §9.4 phase 1's closing rule — "Free space for
backups is checked on the volume of the data directory: if there is less
space than the total size of the copies plus a 20% margin, the run ends
as `aborted` with `E_DISK_FULL`" — as the check ONLY: the run-abort
wiring is a later pipeline task, and the §10 `E_DISK_FULL` mapping
belongs to the command layer (the D-062 posture). Four design points
were open: (1) the production free-space probe — the pre-ruling named
`rustix` (sanctioned in the §5.2 fs row by docs commit f3db54f;
owner-approved in chat the same day, "одобряю", 2026-11-19), but the
exact call, the fields, and their cross-platform sufficiency had to be
verified against rustix 1.1.6's actual API; (2) which of the two statvfs
free-block counts and which of the two block-size fields answer "can
this process still write the copies"; (3) the exact margin arithmetic —
"20% of what, rounded how, and what happens near `u64::MAX`"; (4) the
card's acceptance seam: "a test with a substituted free-space source"
must drive the whole decision path without a filesystem.

Solution: `crates/eunomia-fs/src/free_space.rs` — `MARGIN_PERCENT`,
`required_for(copies) -> u64`, `check_space(free, required) ->
Result<(), FreeSpaceError>`, `check_backup_space(copies, free_space)`
(the seam), and `free_bytes(path)` (the production probe), with one
hand-rolled error enum (the D-017/D-084 precedent; no `thiserror`).

- RUSTIX PROMOTION, VERIFIED THEN PINNED: `rustix = { version = "1",
features = ["fs"] }` — exactly the locked 1.1.6 line (already in
  Cargo.lock as a transitive dependency of `tempfile`, so the promotion
  adds zero new crates to the graph; `fs` is the only feature needed and
  the only one pinned; `std` rides on rustix's defaults). The compile
  check corrected one API expectation: on the linux_raw backend
  `statvfs` answers with `rustix::io::Errno`, not `std::io::Error`, so
  the probe converts `Errno → io::Error` explicitly — `raw_os_error()`
  survives for §10's log line. The docs.rs target set confirmed the
  pre-ruled call exists on both remaining P0 unix platforms (Linux,
  macOS).
- THE WINDOWS GAP (ESCALATED, NOT SOLVED): **rustix 1.1.6 has no `fs`
  module on Windows at all** — `#[cfg(not(windows))] pub mod fs;` in its
  lib.rs — and the libc crate exposes no `statvfs` for
  `x86_64-pc-windows-msvc` either. Windows is a P0 platform (NFR-001…003)
  and D-060's CI matrix compiles the crate on a Windows leg, so the
  module carries a `#[cfg(windows)]` arm of `free_bytes` that returns a
  new typed variant `ProbeUnavailable` — no fabricated data, no unsafe,
  no substituted dependency — solely so the crate compiles there. This
  is an escalation to the orchestrator (the brief forbids substituting a
  crate on my own authority): the production call will likely be
  `GetDiskFreeSpaceExW`, for which `windows-sys` (already locked at
  0.61.2) is a zero-new-crates route but an unsafe-FFI decision — the
  project's first since D-058 — and is the owner's to sanction. Until
  then the caller-facing contract is unchanged: probe failure semantics
  are the caller's decision, on every platform.
- BAVAIL × FRSIZE: `f_bavail` ("available to unprivileged users"), not
  `f_bfree` (total free including the reserved blocks a healthy
  filesystem keeps and a non-root write must not consume); `f_frsize`
  (the fundamental block size the counts are denominated in), not
  `f_bsize` (the "optimal transfer size", which need not equal the
  unit). The multiplication saturates — an absurd statvfs yields the
  `u64` ceiling, never a wrapped number. Both rulings pinned against
  hand-built `StatVfs` fixtures (bfree-vs-bavail and bsize-vs-frsize
  each get a fixture where the wrong field gives a different answer),
  not a live volume.
- MARGIN ARITHMETIC, PINNED BYTE FOR BYTE: the margin is 20% OF the
  sum, added on top; the multiplication runs in `u128` (`sum × 20 /
100`) because a `saturating_mul`-then-divide would distort the margin
  near the ceiling, and the quotient is one fifth of the sum, so it
  always fits `u64`; the final `sum + margin` saturates via
  `u64::try_from(..).unwrap_or(u64::MAX)`, so a near-ceiling copy list
  demands the ceiling and never wraps into a "fits" number (the
  wrapping bug is pinned by name: `[u64::MAX - 10, 20]` summed with `+`
  would read as 9 bytes). The floor rule is pinned exactly
  (`required_for(&[101]) == 121`). The decision is strictly
  `free < required`: §9.4 aborts only when there is LESS space, so
  exact equality passes (`0 == 0` included).
- THE SEAM: `check_backup_space(copies, free_space: FnOnce() ->
Result<u64, FreeSpaceError>)` — a plain closure, the weakest contract
  that works (the scan's `on_event` posture), because the card's
  acceptance test substitutes a fake without a filesystem and must
  exercise sum, margin and decision in one call. The pure core
  (`required_for`, `check_space`) does no I/O, reads no clock, reads no
  configuration (`MARGIN_PERCENT` is a named spec-fixed const, not a
  setting); the production probe is the only I/O in the module.
- ERROR SHAPE (D-084's sibling shape): `#[derive(Debug)]` enum +
  `Display` (lowercase, no trailing punctuation) + `std::error::Error`
  with `source()` chaining the carried io error on the io-backed variant
  only, plus the `io_error()`/`os_code()` accessors the app layer's
  §10 log facts consume. `NotEnoughSpace { free, required }` carries
  both numbers for the "Free up space" message; `ProbeFailed` keeps the
  OS code; `ProbeUnavailable` carries none (no OS call was made).
  `From<io::Error>` feeds the probe conversion. Probe failures pass
  through `check_backup_space` verbatim — a failed query is never
  reported as "no space".
- PLACEMENT: a NEW module `free_space`, the crate's fifth (lib.rs gains
  exactly one `pub mod` line). The name is the spec's own term for the
  quantity (§9.4's "free space"), not the error code and not a broader
  "disk" scope the module does not have; the §5.2 skeleton named none of
  the module files, and the four-probe availability check (D-084) lives
  in `paths` because it is a path validation — space is a property of
  the volume, a different probe family. The shared lib.rs doc sentence
  ("scan, backup and atomic are still skeletons") is left stale on
  purpose (the D-087 merge-window posture); the orchestrator's merge can
  refresh it.

Tests: 18 in `free_space.rs`, colocated, the card's substituted-source
battery first: enough (400 vs 360), the exact-equality boundary
(including `0 == 0`), one byte short (fields `free: 359, required: 360`
pinned), way short (3.6 MB against 10 bytes, Display prefix pinned),
the 20%-of-the-sum margin (3_000_000 → 3_600_000), the floor rule
(101 → 121, 99 → 118), the empty list (0 required, passes on a full
volume), the saturation pair (near-ceiling demand fails at
`u64::MAX - 1`, passes only at the ceiling), the exact-multiplication
pin below the ceiling (a five-exabyte copy demands 6 EB, killing the
`saturating_mul` mutant, whose distorted margin would read 184 PB),
probe-error passthrough (ENOSPC = 28 surfaces as `ProbeFailed`, the
decision never runs on unfetched data), `ProbeUnavailable` passthrough
(the Windows verdict, `os_code() == None`), the probe-first order
(a failed query fails an empty-list check — reordering to skip the
probe when nothing is needed would flip the verdict), the
`From<io::Error>` mapping (EACCES = 13), the two statvfs fixtures
(bavail × frsize = 409_600 where bfree would give 4_096_000; frsize
where bsize would give 819_200; the absurd fixture saturates instead of
panicking or wrapping), the live-probe smoke test (tempdir, positive),
the missing-path probe (ENOENT = 2 on the typed error), and the
Display/source-chain contract (all three texts, the source() chain
Some on `ProbeFailed` only).

Addendum (2026-11-19, T-064/D-089): the Windows
escalation above is RESOLVED. The owner sanctioned the production probe
in chat, 2026-11-19: `windows-sys 0.61.2`, target-gated under
`[target.'cfg(windows)'.dependencies]` in `eunomia-fs`'s manifest —
already in Cargo.lock via the tauri stack, so the direct edge adds zero
new packages (the lock's diff is one line: the edge in `eunomia-fs`'s
dependency list) and Linux/macOS builds do not resolve the dependency
at all. `Win32_Storage_FileSystem` is the only feature gate needed:
docs.rs 0.61.2 places `GetDiskFreeSpaceExW` in
`windows_sys::Win32::Storage::FileSystem`, and the `BOOL`/`PCWSTR`
types it references live in `windows_sys::core`, which is not
feature-gated (the 0.61 generation moved them out of
`Win32::Foundation`). The §5.2 fs row as amended in docs commit 332fc8b
sanctions exactly this call. THE UNSAFE BLOCK: `free_bytes`'s Windows
arm is ONE `unsafe` block — the crate's first, the workspace's second
after D-058's `geteuid` — a direct `GetDiskFreeSpaceExW` call whose
SAFETY comment states every invariant against the Microsoft contract
(fileapi.h): the NUL-terminated wide buffer alive and unmutated across
the call (`wide_path` appends the terminator; a named local, not a
temporary); the out-pointer an aligned, initialized `u64` local
(windows-sys models ULARGE_INTEGER as `u64` — no 32-bit truncation, the
docs' own warning); the two volume totals passed NULL per the API's
`[out, optional]` (§9.4 needs only the available-to-caller figure — the
quota-aware analogue of the `f_bavail` ruling); the `BOOL`/`GetLastError`
convention mapped by a pure `probe_result` helper (`#[cfg(any(test,
windows))]`, the D-058 `is_root` gate precedent) so the Linux suite pins
the mapping deterministically. The path is passed verbatim as a
directory — the API "accepts any directory on a disk"; NULL would mean
"the current disk", never §9.4's data-directory volume; the trailing-
backslash rule binds only the bare `\\server\share` name form. No pure
u64 math exists to extract: windows-sys hands ULARGE_INTEGER over
already as one `u64`, so the unsafe surface moves bytes only — the
extraction clause's goal is met vacuously, and the testable pieces
(the BOOL mapping, the NUL-terminated wide shape) are pinned instead.
PROBEUNAVAILABLE REMOVED: with both platform arms real, no code path
can produce the variant (workspace grep: it lived only in this module;
`free_bytes` was its only producer, and the enum has no consumers
outside it — T-064 merged hours earlier), so the variant is deleted
rather than left dead to misstate the module ("a platform without a
probe" that no longer exists). Tests: the battery is 22 now — the
unavailable-passthrough test is replaced by the `probe_result` mapping
pair (any nonzero BOOL stays Ok even with a garbage last-error
argument; a zero BOOL becomes `ProbeFailed` carrying the raw code
unchanged), the Display/source contract test drops the third text, and
the Windows CI leg (D-060's test matrix) gains three concrete,
falsifiable pins: the live positive probe, the missing-path
`ProbeFailed`-with-OS-code, and the NUL-terminated UTF-16 path shape
against literal code units. Proof honesty: the Windows probe executes
only on the CI Windows leg — this Linux host proves the Linux suite
green, plus `cargo check`/`clippy -D warnings` for
`x86_64-pc-windows-msvc` compiling the real arm.

### D-090 — The Git repository root lookup: canonicalize-first walk-up in `paths`, the negative-lookup/io-refusal split, and the nearest-root consumer split — 2026-11-19

Problem: T-059 implements PRJ-003/D-08's add-project precondition — the
folder picked in the UI may sit anywhere inside a repository and the
app must offer the repository root; a folder with no `.git` anywhere
above is the future `E_NOT_A_REPO` (spec 10). The card pins the walk
("up from the chosen folder to the folder with `.git` (folder or
file)") and four acceptance cases, but leaves five points open: (1)
which module owns the walk; (2) the typed-error shape, given that a
NEGATIVE lookup ("no repository above") produces no io error at all
while a REFUSED probe (denied search, ENOTDIR) does — collapsing the
two would make "could not check" indistinguishable from "checked and
absent"; (3) whether ALG-052's dir-vs-file distinction applies to the
walk too; (4) how canonicalization composes with a walk that must
answer with a path the app can store; (5) what happens when `.git` is
itself a symlink.

Solution: `find_git_root(path: &Path) -> Result<std::path::PathBuf,
NotARepoError>` in `crates/eunomia-fs/src/paths.rs`, next to the
availability check it mirrors and the canonicalization it builds on.

- PLACEMENT: `paths`. The walk IS a path check on a UI-supplied folder
  (the module's pinned responsibility), it is the add-by-hand twin of
  `check_project_available`'s `.git` probe — the §6.9 row "folder with
  `.git` (folder or file)" checked one level at a time upward — and it
  consumes the module's own `canonicalize`. Not `scan`: ALG-050ff is
  discovery over a scan root with its own classification and skip
  reasons; this answers "is THIS chosen folder inside a repository, and
  where does it start". No new module (D-019's boundary stands), no new
  dependency (the walk is std `Path::ancestors` plus one
  `symlink_metadata` per level; the manifest and `Cargo.lock` are
  untouched).
- CANONICALIZE-FIRST, THE D-086 POSTURE: the input resolves through the
  module's `canonicalize` before any probe, so the returned root is the
  canonical stored form (DB-030) and the add flow gets "offer the root"
  and "store the root" from one answer. It also makes the walk SAFE by
  construction: every ancestor of a canonical path is a strict prefix
  of it, so `Path::ancestors` visits each level exactly once and
  terminates at the filesystem root (whose `.git` probe still runs) —
  no cycle is possible because symlinks are already resolved (D-07).
  The cost is honest and accepted: canonicalization requires the whole
  path to EXIST, so a vanished folder fails as the typed
  `CanonicalizeError` before the walk can say anything about
  repositories — the right answer anyway for a UI pick (the folder
  existed at pick time; if it is gone now, that is D-09's situation,
  not a repository question).
- TWO FAILURE CLASSES, TWO KINDS OF VARIANT: `NotARepoError` is named
  for the §10 code the add flow raises, but it is an enum because the
  failure modes differ in kind. `NoGitEntry { path }` — every level up
  to the filesystem root was probed and none has `.git`: a fact about
  the tree, and NO io error exists (a negative lookup is an absence,
  not a refusal — there is no syscall to carry), so the variant has no
  `io::Error`, `os_code()` answers `None`, and it is THE variant the
  future `E_NOT_A_REPO` mapping consumes. `NotReadable { path, source
}` — a probe REFUSED by the OS (denied search on a walked folder; the
  canonicalized input being a plain file → ENOTDIR; a drive dropped
  mid-walk): the refused path text and the exact `io::Error` ride along
  for §10's log facts, following D-084's precedent of a coarse access
  bucket (`NotReadable`, this module's established name) with the exact
  raw code carried. `Canonicalize { source }` — the input itself
  unresolvable, D-086's typed failure carried whole (so the missing-
  input case surfaces the existing typed failure instead of a second
  encoding of it). What the command layer maps the io-backed variants
  to is its call (D-062's posture keeps `AppError` out of this crate);
  the hand-rolled impls stay `thiserror`-free (D-017 store precedent).
- THE PERMISSION WALK IS DETERMINISTIC-TESTABLE ANYWAY: canonicalizing
  the FINAL component needs search permission only on its parent,
  while the `<folder>/.git` probe needs search ON the folder — so mode
  0o600 on the picked folder pins the refused-probe branch without any
  race (verified on the platform before writing: realpath succeeds,
  the probe answers EACCES = 13). An ancestor denied in the walk's
  window is the same variant by construction; the TOCTOU posture is
  documented like D-084's — probes are separate OS calls over a live
  tree, the verdict is what the probes observed, and `NotFound` on a
  level simply continues the climb.
- `.git` DIR, FILE, OR SYMLINK — ALL ROOT TERMINATORS: the walk serves
  the add-by-hand flow, where §6.9 and D-06 accept the file form ("you
  can add it by hand"); ALG-052's "submodule or worktree" skip is a
  SCAN rule with a different consumer (discovery over a whole tree),
  not a walk-up rule — a linked worktree resolves to the worktree's
  own root, never through the pointer into the main repository. The
  `.git`-symlink ruling follows D-084 (presence, not validity): the
  probe is `symlink_metadata`, which does not follow the final
  symlink, so even a DANGLING `.git` symlink terminates the walk;
  whether the pointer leads to a healthy gitdir is git's business.
- NEAREST ROOT WINS: bottom-up, the FIRST `.git` stops the walk — a
  folder inside a nested repository resolves to the inner one (D-06:
  nested repositories are registered as separate projects), and "the
  picked folder IS the root" is the walk's first iteration, not a
  special case.
- lib.rs STAYS UNTOUCHED (the D-087 posture, now stronger): TWO
  concurrent sibling tasks own new module lines in that file (T-064,
  T-065); `paths.rs`'s module docs carry the accurate
  implemented-so-far record, and the orchestrator's merge refreshes
  the shared file once for all three tasks.

Tests: 11 unit tests + 1 doctest in `paths.rs`, colocated, through the
public function on real temp fixtures — the card's four acceptance
cases first: the picked folder IS the root (the answer equals the
canonicalized input); a subfolder two levels deep finds the same root;
the typed negative lookup outside any repository (`path()` pinned to
the canonicalized input, `os_code()`/`io_error()`/`source()` all None;
the temp dir's REAL ancestors verified `.git`-free with independent fs
probes so the fixture cannot pass vacuously, loud skip if the host
really has a repository above); and the `.git` FILE terminates the
walk (worktree form, entered from a nested folder). Plus: the nearest
root wins over an outer nested repository (both sides asserted); a
symlinked subfolder resolves to the real root path (the
canonicalize-first observable; the alias spelling asserted absent); a
dangling `.git` symlink terminates the walk; a denied probe under mode
0o600 is `NotReadable` with EACCES = 13 and the refused path text
(loud root skip, mode restored before assertions, a
canonicalization-still-ok precondition captured while locked); a
plain-file input is `NotReadable` with ENOTDIR = 20 (root-proof; pins
that canonicalize accepts files and the walk classifies the refusal
honestly; pinned on Unix — on other platforms the classification of
this OUT-OF-CONTRACT input follows the OS answer and is documented on
the variant: the folder picker supplies folders, canonicalize merely
requires existence); a missing input is the `Canonicalize` variant (path text,
ENOENT = 2 pinned on unix); and the error-contract test pins the
Display texts, `path()` per variant, the io-backed split and the
`source()` chain.

### D-091 — Backup copy with verification: the fs→core edge activated, the plain SHA-256 widened in core, and the check/remove/dedup semantics — 2026-11-19

Problem: T-068 implements spec §7.6's copy row (BKP-001, 002, 008,
010, 011) and §9.4 phase 2 step 1 as `eunomia-fs`'s `backup` module:
copy the target file into `backups/<project_id>__<slug>/`, check size
and SHA-256 before the copy counts as created, and skip the copy when
the source's digest equals the file's last backup's (the record then
points at the existing copy). The card fixes the acceptance ("a
substituted damaged copy is caught by the check; an identical file
does not multiply copies") but leaves open: how `fs` reaches the
D-080 name builders and a SHA-256 without violating §5.2's
dependency rows; the copy/check ordering and its TOCTOU honesty; the
dedup contract's input; what a failed copy leaves on disk; folder
rights (SEC-007); and the API shape of an eight-input operation.

Solution: the `backup` module of `eunomia-fs` — one entry point
`backup_target_file(&BackupTarget)` plus `verify_backup_copy`, the
typed `BackupError` (hand-rolled, D-017/D-084 posture,
`io_error()`/`os_code()` accessors in the `paths.rs` shape), and
`BackupOutcome::{Created, Deduplicated}`. The rulings:

- FS→CORE: §5.2's `fs` row lists `core`, so the edge — pre-wired by
  T-003 (D-019) — is now first-activated: `backup` consumes
  `backup_name`'s builders and `hash`'s digest type. The manifest
  gained no dependency line; its comment's "the skeleton references
  none of core's types yet" was truthed instead.
- HASH VISIBILITY: `fs` never names `sha2` (§5.2 pins `sha2` to
  core's row; D-017's per-crate allow-lists). Core gained
  `hash::Sha256Hex` — an opaque incremental wrapper
  (`new`/`update`/`finalize_hex`, pure, no I/O) rendering the
  canonical 64-character lowercase hex — and `fs` streams file bytes
  through it. This is a deliberate, recorded API-surface widening
  made from an fs task because the spec's shape demands it: the hash
  lives in the crate that owns `sha2`, the streaming lives in the
  crate that owns files, and the wrapper keeps the digest's byte
  form and the hashing crate out of `fs`'s vocabulary. The DB-020
  build hash (`build_hash`'s `v1` framing) is untouched — the
  wrapper digests raw bytes only, the plain file-content hash
  BKP-010 compares.
- STREAMING: every pass moves a fixed 64 KiB chunk
  (`COPY_CHUNK_BYTES`, one named constant) — source hashing, copying
  and copy verification never hold more than one chunk. Three
  passes: hash the source (size from the open file's own metadata,
  fstat, taken once), copy, re-hash the copy. `std::fs::copy` is
  deliberately not used: it silently overwrites the destination and
  preserves the source's permission bits, neither of which the
  backup semantics want.
- DEDUP: `last_backup_sha256_hex: Option<&str>` is a PARAMETER — the
  store lookup belongs to the store tasks. Comparison is exact
  full-digest equality on the canonical lowercase spelling; both
  sides hash with the same core primitive, so equal strings mean
  equal content, and a non-canonical value simply compares unequal
  into a fresh copy. A dedup decision creates NOTHING (not even the
  folders) and returns the source's (size, digest) so the caller's
  record can reference the existing copy; whether that copy is still
  on disk is the store's record question, not this module's.
- CHECK AND REMOVAL: BKP-010's check is size first, then the digest
  of the copy's bytes (no digest computed for a size mismatch — the
  check has already failed). The copy is `sync_all`-ed before the
  check so the verification judges settled bytes. Any check failure
  REMOVES the corrupt copy — bytes under a verified-backup name that
  are not verified content must not linger (the card's damaged-copy
  acceptance) — and `VerifyFailed::cleanup` carries a refused
  removal for the run's `discarded` sweep (BKP-005) to retry.
  `verify_backup_copy` is the check as its own public function with
  the same remove-on-failure contract, so tests and future callers
  drive it directly. Write-path errors remove the partial copy the
  same way; handles are dropped before removal (Windows sharing
  violations).
- NEVER OVERWRITE: the copy is created with `create_new` — an
  existing file under the destination name (a same-millisecond
  collision in a retried run) is the typed `CopyFailed`
  (`AlreadyExists`), never a clobber: a backup is history.
- FOLDERS: the `backups` root and the project folder are created as
  needed (BKP-001/002: a first-ever backup must not fail for a
  missing folder); a missing DATA directory is the typed
  `FolderCreateFailed`, not silently repaired — app startup owns
  that folder. The root is born `0700` on Unix: created in ONE step
  with that mode (`DirBuilder::mode`), so no window exists in which
  a permissive root is on disk and there is no chmod to fail after
  creation (a umask can only remove bits, and `0700` has no
  group/other bits to remove); an existing root keeps its rights.
  The leaf copy's mode is the platform default and deliberately
  unpinned — the BKP specs pin no leaf rights, and traversal past
  the `0700` root is impossible for anyone but the owner regardless.
- CRASH DEBRIS AND DURABILITY: the copy is written straight to its
  final §5.5 name (`create_new`; no staging grammar for backups —
  that would be its own naming convention, its own task), so a kill
  mid-copy can leave a partial file under a final backup name; the
  debris is INERT (no record references it — the record follows a
  verified copy) and its sweeping belongs to the store tasks'
  run-level/retention work, the same relationship §9.5's staging
  files have to `temp_files`. The copy's bytes are `sync_all`-ed
  before the check, but the project folder's entry is not fsynced
  (§9.5 pins a directory fsync for the write phase's rename only;
  std cannot open directories portably on Windows); a crash in the
  copy→record window can lose the entry — the unreferenced/missing
  copy situation §10's restore path already models (`missing`
  status). Accepted here, recorded, owned downstream.
- TOCTOU: honest — hash → copy → re-hash copy, so a source change
  between passes surfaces as a verification mismatch (the copy is
  not the snapshot's bytes) and the corrupt copy is removed; a size
  change mid-hash fails against the fstat size. The one unclosable
  window — a same-size mutation DURING the hash read — is documented
  as out of the check's threat model (silent copy corruption, not
  adversarial mutation; §9.5 separately verifies what gets written).
  The source file is never modified.
- API SHAPE: the eight inputs are one operation's vocabulary —
  grouped in the `BackupTarget<'a>` struct (source, data_dir,
  project_id, project_name, scheme, base_name, time,
  last_backup_sha256_hex) instead of an eight-argument call; the
  `too_many_arguments` lint fired and the repo has no `#[allow]`
  precedent — restructure, don't allow. All identity fields stay
  typed (SEC-006): the builders own the gates, a hostile ID is
  `NameFailed` before any path exists, a hostile name is cleaned.
  The §9.4 RUN semantics (the `backup_records` row, the `discarded`
  sweep, the `aborted`/`E_BACKUP_FAILED` mapping) are the store and
  pipeline tasks' — this module is the per-file operation they call;
  no DB, no clock, no configuration reads.

Known-open items, D-062's posture: `eunomia-fs`'s `lib.rs` `//!`
prose still lists `backup` as a skeleton, and `eunomia-core`'s
`lib.rs` sha2 parenthetical does not yet mention the new wrapper —
both blocks are contested by live parallel tasks (T-059/T-064) and
are reconciled when the files are next free.

Testing: 20 colocated tests in `backup.rs` + 4 in core's `hash.rs`
(+ 1 doctest) — the happy path against independent oracles (FIPS
180-2 vectors; the created name pinned byte for byte to the builders'
spelling with the injected time), the dedup acceptance (same digest →
no copy and no folders; different digest → a copy; a last hash
matching only the hash8 PREFIX → a copy, pinning full-digest
equality against the short-id trap; an uppercase last hash → a
copy, pinning exact-byte equality), the substituted
same-size copy and the truncated copy caught by `verify_backup_copy`
and removed from disk, an untouched copy passing, the 8 MiB
streaming case (a Python-hashlib oracle over the exact byte pattern
— chunking cannot drift), the empty file, a missing source (typed,
OS code), folder creation with the `0700` root, a missing data
directory, the never-overwrite collision, the REFUSED corrupt-copy
removal (an r-x folder: `cleanup` carries EACCES, the Display says
"could not be removed", the copy stays for the `discarded` sweep —
Unix-gated with the loud root skip), hostile id and hostile
name (components stay `Normal`), and the error contract (Display,
source chain, OS codes). Mutations were run against the battery and
each bit exactly its test: weakening the digest comparison fails the
substitution test, removing the dedup branch fails the
identical-file test, dropping the size check fails the truncation
test, and skipping the corrupt-copy removal fails the removal
assertions. Windows-target and
macOS-target compile checks of the crate's tests pass (the cfg-gated
mode and permission code). Full suite: fmt, clippy
`--workspace --all-targets -D warnings`, `cargo deny check`,
`cargo test --workspace` (975 green) and the four pnpm checks all
green on the handed-back commit.

Round-1 review (deepseek-v4.1-flash) approved with five findings,
all addressed: the hash8-prefix and uppercase dedup boundary tests
were added (P2: full-digest equality now pinned), the `0700` root
creation became one-step (`DirBuilder::mode` — no permissive-root
window, P2), the `CopyFailed::cleanup` doc reference was corrected
(P3), the manifest comment now says `ignore` is active since T-061
(P3), and `verify_backup_copy`'s docs open with "destructive by
contract" (P3 — the destructive signature is kept deliberately: the
removal-on-failure is the module's pinned BKP-010 contract, and a
`&BackupCopy` parameter would not remove the footgun). The
crash-debris and directory-entry findings were answered with the
documented posture above rather than new machinery (staging names
and dir-fsync are unpinned by the spec and out of the card's scope).
Round-2 review (hy4-flash-free) approved, verifying all five round-1
fixes against the code and passing the test audit; its one finding —
the refused-removal (`cleanup = Some`) branches had no pinning test —
was fixed with the Unix-gated r-x-folder test above (mutation-checked:
dropping the cleanup reporting fails exactly that test).

### D-092 — Sync target-path resolution: whole-chain canonicalization, the component-wise containment with a Windows-only fold retry, and the uniform final-path check — 2026-11-19

Problem: T-062 implements §9.4 preflight step 2 / §9.5 step 1 (T-F6):
the sync's write target `<root>/<name>` must be resolved when it is a
symlink ("the write goes to the real file"; the common case
`CLAUDE.md` → `AGENTS.md`) and refused typed when it is dangling
(`E_TARGET_BROKEN_LINK`), resolves outside the root
(`E_TARGET_OUTSIDE_ROOT`), or is a folder (`E_TARGET_IS_DIR`). The
spec pins the verdicts but leaves open: (1) the resolution mechanism —
`dunce::canonicalize` on the link vs a `symlink_metadata` +
`read_link` composition; (2) how containment is checked on every OS
(component-wise vs string prefixes; what case-insensitivity means for
the ROOT comparison); (3) the typed-error shape, given that two
verdicts are tree facts with no OS failure and two are refusals; (4)
whether an absent target is an error (the spec implies a first sync
creates it); (5) what the function promises about the race between
resolution and the later write.

Solution: `resolve_target_path(root, file_name) -> Result<TargetPath,
TargetPathError>` in `crates/eunomia-fs/src/paths.rs`, next to the
canonicalization it builds on.

- MECHANISM: the module's own `canonicalize` (dunce) on the linked
  candidate — the WHOLE chain resolves in one OS call (the kernel
  follows links of any depth; a `read_link` composition would
  re-implement the walk with a depth bound and re-derive normalization),
  and the answer is the absolute, verbatim-spelled canonical form both
  the containment check and the writer want (the D-086 primitive). A
  dangling link is ENOENT from that call — the spec's own case falls
  out; a loop is ELOOP, classified broken like a dangling link (the
  resolution can never end at a real file; the constant is spelled per
  platform — Linux 40, macOS 62, verified against xnu's
  `bsd/sys/errno.h` where 32 is EPIPE — because `ErrorKind::FilesystemLoop`
  is still unstable; platforms whose spelling is unknown here fall to
  the coarse `NotReadable` bucket, code riding in the source). The
  resolved real path is statted to classify a link-to-directory as
  `IsDir` — canonicalize succeeds on directories, so the folder
  verdict needs its own probe. The D-086 verbatim ruling is about the
  PROJECT path's identity; the resolved target is a WRITE path, not a
  stored identity — its canonical form is what keeps containment
  honest and what the writer renames over.
- TARGETPATH, NOT A BARE PATHBUF: the success struct carries the write
  path AND the resolved file name, because the name crosses a second
  seam — the staging grammar `.<name>.eunomia-<token>.tmp` (9.5 step
  2, D-088) keys on the RESOLVED name (a `CLAUDE.md` link stages and
  is swept as `AGENTS.md`); both fields come from one resolution and
  cannot drift. The name is lossy text (the module's established
  U+FFFD posture); the write path carries the real bytes.
- CONTAINMENT: component-wise (`Path::strip_prefix`) between CANONICAL
  forms — never a string prefix (`/repo2` does not start with
  `/repo`); both sides are canonical because the ROOT ITSELF
  canonicalizes at entry (the D-086 posture ENFORCED here, not assumed
  of the caller: a root behind a symlinked ancestor — on macOS every
  temp fixture, `/var/…` = `/private/var/…` — must not produce a false
  OutsideRoot; a vanished root is the honest `NotReadable` refusal,
  the pipeline's availability check owns that situation) plus
  canonicalize on the link, so the shared prefix carries the
  filesystem's own spelling and the byte-exact comparison answers
  everywhere. The check is PHYSICAL on every branch, not lexical (the
  round-2 review caught the first draft judging the non-link branches
  on the joined spelling, where an INTERMEDIATE symlink — `sub/x.md`
  with `sub → ../outside` — is lexically inside but physically out):
  the regular-file branch canonicalizes its whole path (for a
  one-component contract name the answer is the same path; one extra
  realpath is the price of not trusting the name), and the absent
  branch resolves its parent chain ([`resolve_absent`]; a parent
  chain that does not exist has nothing to link through — the lexical
  join is the truth there, a future creation is plain). The check
  also demands ordinary NAMES in the remainder
  after the prefix: a `..` component strips-prefix TEXTUALLY while
  climbing out of the root (verified — the review round caught the
  first draft relying on the bare prefix), and an absolute name
  replaces the root; both are refused as
  `OutsideRoot`, while an interior `.` is inert (`components()`
  normalizes it away; the OS resolves the same inside file) — the fs
  layer does not silently trust the enum seam
  (§5.7, SEC-006). When byte-exact fails, the comparison retries
  case-folded on WINDOWS ONLY — the one platform whose filesystem
  folds case as unconditional platform behavior (NTFS/FAT), folded
  with the same rule as `PathKeyRule::Windows` per component (NFC →
  simple fold → NFC), defense-in-depth behind the root
  canonicalization. macOS is deliberately excluded: default APFS folds but case-sensitive APFS is supported, and a
  folded comparison there could accept a genuinely DIFFERENT directory
  — the false-inside direction, the dangerous one (a write outside the
  registered root); Linux keeps bytes (DB-030). The check runs on the
  FINAL write path whatever produced it: for contract inputs
  (`file_name` is one ordinary component from `target_file_name`,
  §5.7, SEC-006) it is a no-op, but a structurally escaping name
  (`..`, `.`, an absolute path) is refused as
  `OutsideRoot` instead of silently written.
- ERROR SHAPE: `TargetPathError`, one variant per §10 `E_TARGET_*`
  code named for it, plus the module's established `NotReadable`
  bucket (D-084/D-090) for refused probes — "could not check" is not
  any of §9.4's verdicts. `BrokenLink` and `NotReadable` carry the
  exact `io::Error` (source chain, `raw_os_error`); `OutsideRoot` and
  `IsDir` are tree facts no OS call failed for — `os_code()` is None,
  the D-090 negative-lookup/refusal split. `OutsideRoot` carries both
  paths (the resolved real path and the root it was checked against).
  Hand-rolled impls, no `thiserror` (D-017); accessors `path()`/
  `io_error()`/`os_code()` in the module's shape. The new consuming
  accessor `CanonicalizeError::into_io_error` lets typed errors
  classify a canonicalization refusal and re-raise the io error as
  their own variant (`NotARepoError` carries it whole instead — both
  patterns now exist deliberately).
- ABSENT TARGET IS `Ok`: the plain joined path (of the canonicalized
  root) is returned and nothing
  is created — a first sync CREATES the file (§9.4 implies it; pinned
  by a test asserting nothing was created, the preflight "changes
  nothing" rule). A regular-file target returns the joined path
  unchanged: the root is canonical and the name one component, so the
  join is already canonical and no resolution call is made.
- TOCTOU, DOCUMENTED: the probes are separate OS calls over a live
  filesystem; the verdict is what they observed at their moment (the
  D-084 posture; pinned by a remove-the-file flip test). The
  structural guarantee: the write target is the RESOLVED path, so the
  atomic write renames over the real file and never over the link
  spelling — the link survives (T-F6). Not guaranteed: the link can be
  re-pointed before the write (the write lands on the previously
  resolved file; the next sync re-resolves — a sync is idempotent,
  §10); an absent target can appear as a link (the rename replaces
  that link with a regular file; §9.4's skip semantics tolerate the
  flip); a resolved file can vanish (the write reports its own
  failure, 9.5 step 7). No snapshot, no pretended atomicity.
- No new dependencies (std `fs` probes + the crate's own canonicalize;
  D-017's allow-list untouched, Cargo.toml/Cargo.lock unchanged).
  `CanonicalizeError::into_io_error` is the only change to existing
  API; lib.rs's paths sentence gained the new capability (plus the
  D-090 lookup it had omitted under the concurrent-siblings posture).
- KNOWN OPEN (the D-088/D-090 posture): ARCHITECTURE.md's tree still
  lists the fs modules as skeletons — the concurrent-siblings posture
  leaves that shared tree to the orchestrator's refresh; the accurate
  record is this entry and the module docs. And a Windows link loop
  classifies as `NotReadable`, not `BrokenLink`: the Win32 spelling of
  the loop refusal is not pinned from this machine (a blind constant
  would be an unverified fact), so it falls to the documented
  unknown-platform bucket with the exact code riding in the source.

Tests: 17 unit tests + 1 doctest in `paths.rs` (18th cfg-gated for
Windows CI), colocated, through the public function on real temp
fixtures — the card's four acceptances first: the dangling link
(`BrokenLink`, ENOENT = 2, the link spelling named); a link to a REAL
file outside the fixture root (`OutsideRoot` carrying the resolved
path and the canonical root, no OS code); a plain folder target and a link
resolving THROUGH to a folder (both `IsDir`, the resolved path named);
and `CLAUDE.md` → `AGENTS.md` (the write path is the canonical real
file, the staging name is `AGENTS.md`, the link survives — T-F6).
Plus: the absent-target creation case (nothing created); the
regular-file pass-through; a depth-2 chain resolving to the final
file; a self-loop (`BrokenLink`, OS code riding along); a relative
multi-component link spelling resolving through the link's folder; the
structural escape refusals (`..` and absolute names are
`OutsideRoot` — pinned end to end AND in the pure core, where the
ParentDir remainder strips-prefix textually; an interior `.` pinned
inert and accepted); the intermediate-link refusals (a deeper name
reached through — and created behind — a link hiding in a middle
component judges by the real location, both branches pinned); the
containment posture — byte-exact pinned on Linux (a case-differing
sibling folder is OUTSIDE) and by the pure parameterized
`inside_root_components` core (both flavors, plus the `/repo2`
string-prefix trap), with the Windows-leg pin (a case-flipped root
spelling stays inside — the observable the platform owes, absorbed by
the root canonicalization with the fold retry behind it); a denied
target probe under a search-bit-less root
(`NotReadable`, EACCES = 13, loud root skip); the observed-tree flip
(resolves while the real file exists, broken after removal); and the
error-contract test pinning Display texts, `path()` per variant, the
io-backed/fact split and the `source()` chain.

### D-093 — The projects repository: TEXT in/out with the schema's full path_key uniqueness, a table-wide gapless sort_order, direct `ProjectsError` (the `StorageError` surface freeze), and the journaling boundary left with the producer — 2026-11-19

Problem: T-071 adds CRUD for projects and exclusions, the DB-015
`path_key` lookup, and the `sort_order` handling to `eunomia-store`
(spec §6.2, §6.4). Open questions: how the uniqueness scope of the
lookup relates to the schema's constraint; what rule the
`projects.sort_order` column follows, given that DB-005/DB-011 pin the
_library_ `position` explicitly and say nothing projects-specific; how
the typed UNIQUE violation is surfaced without widening
`StorageError`; whether project add/change/archive/restore journal
here (LOG-001…004 name them, D-051 exists); and whether the
store→core edge finally earns its keep.

Solution: `eunomia_store::projects`, one module for both path-bearing
tables (`projects`, `project_exclusions` — one feature boundary: the
§6.4 archive/restore flow composes exclusions, PRJ-017, and one error
shape), registered in `lib.rs`. Public API: `create`/`get`/
`find_by_path_key`/`list(scope)`/`update`/`archive`/`restore`/
`reorder` plus `create_exclusion`/`list_exclusions`/
`find_exclusion_by_path_key`/`delete_exclusion`, over
`NewProject`/`Project`/`ProjectUpdate`/`NewExclusion`/`Exclusion` and
the vocabularies `ProjectState`, `AddedVia`, `ExclusionReason`,
`ListScope`. The §6.5 enum types (`Scheme`, `BaseName`, `Position`,
`ExtraFile`) are reused from `settings` — no second definition of a
closed vocabulary (three private `parse_*` adapters decode them on
reads; the round trip is unit-pinned).

- _Uniqueness scope: the schema's, not a parameter._ The migration
  (T-016, source of truth) carries `path_key TEXT NOT NULL UNIQUE` on
  both tables — a _full_ unique index over active **and** archived
  rows, exactly DB-015's wording; there is no partial-index reality to
  reconcile. So `find_by_path_key` deliberately has no active-only
  scope: it returns the row in either state and the caller branches on
  `Project::state` (active = PRJ-001's "already added", archived =
  the "offer to restore" flow). The two tables are separate
  namespaces: a project row and an exclusion row may legally share a
  key — that coexistence is what archiving a scan-found project
  produces, and an integration test pins it.
- _sort_order: DB-005 over the whole table._ Creation appends
  `MAX(sort_order) + 1` (a scalar subquery _inside_ the single
  `INSERT` statement, so the append cannot interleave even outside a
  transaction); reads order by it (ALG-001's subset read);
  `reorder` renumbers the complete id set `1..=N` in one transaction
  and rejects a set that is not a permutation of the table
  (`ReorderMismatch` with unknown/missing/duplicates counted)
  _before_ the first write. Archive/restore keep a row's position:
  §6.4's archive row touches only the folder, the assignments and the
  scan exclusions — it says nothing about `sort_order`, so keeping the
  position is this repository's reading (the least destructive one),
  reinforced by v1 having no physical project delete (the table only
  grows) and PRJ-013…018's restore "bringing the project back" — with
  its place. The gapless
  invariant is therefore table-wide (`1..=N` over all rows), not
  active-subset-wide; DB-011's 1..=N stays the library's own pin and
  nothing here touches `library_items`.
- _Typed UNIQUE violation, deliberately typed._ The one runtime
  outcome between two individually legal rows — the `path_key`
  collision (PRJ-001's "already registered", PRJ-006's "skipped") —
  maps SQLite's extended result code `SQLITE_CONSTRAINT_UNIQUE` to
  `ProjectsError::DuplicatePathKey { path_key }`, never the message
  text. §6.9 field validation is NOT duplicated here: it lives in
  `eunomia_core` (`project_validators`, D-075/D-079) and runs at the
  command boundary; the schema CHECKs are the store's floor and
  surface as `ProjectsError::Sql`. `archive`/`restore` are idempotent
  (a repeated call returns the row unchanged, no `archived_at` drift —
  the make-operations-idempotent rule), and maintain
  `archived_at IS NOT NULL ⟺ state = 'archived'`; `update` writes
  exactly the fields present (PRJ-008's editable set: name, label,
  color, description, scheme, base_name, the two extra files — never
  path/path_key, PRJ-008), restamps `updated_at` only when it wrote,
  and a fieldless update is a verified no-op. Reads reject a stored
  vocabulary cell outside its CHECK vocabulary with
  `ProjectsError::Member` (the D-039 corrupt-cells policy; the tests
  simulate corruption through `PRAGMA ignore_check_constraints`, the
  only way such a cell can exist).
- _`ProjectsError` returns directly — the `StorageError` surface is
  frozen for this task._ The `Settings`/`ActionLog` precedent nests a
  typed error inside `StorageError`; that path was implemented first
  and then reverted, because `src-tauri/src/retention.rs` _exhaustively_
  matches `StorageError` and T-071's hard scope forbids touching
  `src-tauri/**`. So this module's functions return
  `Result<_, ProjectsError>` with a `Sql(rusqlite::Error)` variant
  (`From` impl lets `?` work), all inside `crates/eunomia-store/**`:
  `storage.rs` ends this task with a zero diff. Consequence for the
  first command task that consumes the repository: it maps
  `ProjectsError` beside the `StorageError` it already unwraps; the
  first store-side task allowed to edit `src-tauri` may normalize the
  two shapes (fold `ProjectsError` into a `StorageError::Projects`
  variant) — that is a deliberate follow-up, not drift.
- _Transactions: executor-agnostic, as D-051 settled._ Every function
  joins the caller's transaction (the `settings::set` shape); the
  one-transaction-per-operation boundary stays the app-layer `Db::run`
  (D-037). `reorder` is the only multi-statement writer and says so in
  its docs; its rollback atomicity is pinned by an integration test
  through `unchecked_transaction` (the `action_log::cleanup` test
  pattern).
- _Journaling: the producer's, not the repository's._ LOG-001…004
  does put project add/change/archive/restore in the journal, and the
  vocabulary already exists (`Action::ProjectAdded`…`ProjectRestored`,
  D-051) — but D-051's own rule assigns the entry to the _producer_
  at its `Db::run` boundary (the `settings::set` precedent: the store
  writes no log rows). The future project-commands task composes
  `create`/`update`/`archive`/`restore` with `action_log::insert` in
  its one transaction; nothing here touches `action_log`.
- _The store→core edge stays unused._ Every signature is TEXT and
  store-local enums (D-017's rule: the edge is added by the first task
  whose store code references core types). Canonicalization and
  `path_key` construction remain `eunomia_fs::paths`' job (DB-030/
  DB-031, D-086): this module stores and looks up the TEXT it is
  handed. `Cargo.toml`/`Cargo.lock` unchanged.
- _Tests._ 28 new tests: a unit pair in the module (vocabulary round
  trip through the schema literals; error Display shapes) and 26
  integration tests in `tests/projects.rs` on a real `open_db` +
  `run_migrations` database (the established store pattern): the
  active-duplicate and archived-duplicate conflicts as typed errors
  with the surviving row verified; cross-namespace coexistence;
  exclusion CRUD with physical delete; gapless append `1..=N` (after
  a reorder and with an archived row in the middle alike); scoped
  lists in `sort_order` order; archive/restore keeping positions;
  reorder renumbering, its three rejection shapes with the table
  verified untouched, and the transactional rollback (plus the twin
  pin that a failed duplicate insert inside a transaction keeps that
  transaction usable — the ABORT promise the command tasks build on);
  full-column round-trip with UUIDv7/DB-002 pins; partial update with
  a verified no-op, a same-value update succeeding (`changed == 0`
  means "no such id"), and a restamped `updated_at` discriminated by
  a millisecond-boundary sleep; lifecycle stamps and idempotence;
  `NotFound` typing on every by-id writer; the CHECK floor as `Sql`;
  corrupt `state`/`added_via`/`reason` cells as typed `Member` on
  `get` and both `list`s.

### D-094 — The write-access preflight: file-flag-first order, std's read-only definition, the staging-grammar probe with sweepable debris, and collisions as probe failures — 2026-11-20

Problem: T-063 implements §9.4 preflight step 3 (OUT-007/008, T-F5):
"Check rights: whether a temporary file can be created in the folder,
and whether the existing file is marked read-only. If not: `skipped`,
`E_NO_WRITE_ACCESS`." The spec pins the verdict but leaves open:
(1) the order of the two checks (the sentence lists the folder first);
(2) what "marked read-only" means cross-platform — the concept is
Windows' `FILE_ATTRIBUTE_READONLY` wearing a Unix costume; (3) whether
the probe's temporary file may use any convenient name; (4) what a
`create_new` collision means; (5) where probe refusals that are not
rights facts go (missing folder, full volume); (6) whether an absent
target is an error.

Solution: `crates/eunomia-fs/src/write_access.rs` —
`check_write_access(target) -> Result<(), WriteAccessError>`, next to
the preflight's step-1/step-2 modules.

- INPUT: the RESOLVED write path of `resolve_target_path` (D-092) —
  one argument is all step 3 needs: the flag is a fact about the path,
  the folder is its parent, the staging name is its file name.
  `fs::metadata` FOLLOWS a remaining link, so the flag judged belongs
  to the real file the write replaces.
- ORDER: the FILE flag first, then the folder probe. The spec sentence
  is a conjunction; this order keeps the check's only mutating step
  (the probe) from running once the verdict is decided (the phase's
  "changes nothing" spirit), and the file flag is the nearer cause — a
  fact about the exact file being replaced (D-092's precedence style).
  Pinned by a both-refuse test (read-only file + 0o555 folder answers
  `FileReadOnly`).
- READ-ONLY DEFINITION: std's `fs::Permissions::readonly()` — the
  cross-platform marked-read-only fact (Unix: `mode & 0o222 == 0`,
  what `chmod a-w` produces; Windows: `FILE_ATTRIBUTE_READONLY`, which
  std's own metadata probe reads — the idiomatic cfg handling lives in
  std, so the module carries no cfg). The flag is the MARKED state,
  not an OS prediction: a Unix `rename` could in fact replace a
  write-disabled file, but T-F5 pins the read-only target as a skip
  regardless — the user's mark is a "do not touch" signal honored
  before any bytes move. An absent target skips the flag check
  entirely (the creation case, D-092; the card: rights, not
  existence).
- THE PROBE IS REAL, and its name is the app staging grammar
  (`temp_files::tmp_file_name`, D-088): `.<name>.eunomia-<8 lowercase
hex>.tmp`, `create_new`, drop the handle (Windows cannot delete an
  open file), remove. The grammar is not decoration: debris that a
  crashed or refused cleanup leaves is recognized by the step-7 sweep
  (`remove_stale_tmp_files`) and deleted once an hour old — the
  probe's net effect converges to zero (the D-088 idempotency
  posture). A failed or raced removal never flips the verdict:
  creation, not removal, answered the rights question. Tokens: clock ×
  pid × attempt folded into 8 lowercase hex characters (no rand crate
  — freshness is all a staging name needs, and `create_new` plus the
  bounded retry is the real safety), `PROBE_ATTEMPTS = 3` — this
  module's own bound (one fresh token rules out an accidental
  collision, three cap the cost), mirroring rather than citing 9.5
  step 4's three-retry pattern (that clause is about rename sharing
  violations, not staging-name collisions).
- CLASSIFICATION: a pure `classify_creation_refusal`, pinned by
  synthetic-kind tests (the D-089 `probe_result` posture — the real
  probe cannot be made to fail on command). `PermissionDenied` |
  `ReadOnlyFilesystem` → `FolderNotWritable` (the EROFS reading: a
  read-only mount IS "no write rights to the folder", §10); the
  `ErrorKind` variants are stable on the 1.99 toolchain.
  `AlreadyExists` → retry with the next token; exhaustion of all
  attempts → `ProbeFailed` (a collision is never a fabricated rights
  verdict). Everything else — `NotFound` (step 1's availability check
  owns the missing-folder story), `StorageFull` (§10's `E_DISK_FULL`
  write-phase row), a dropped drive — is `ProbeFailed` with the exact
  `io::Error` riding along (the D-090 refusal split).
- ERROR SHAPE: `WriteAccessError` — `FolderNotWritable` and
  `FileReadOnly` (both the §10 `E_NO_WRITE_ACCESS` facts, mapped by
  the command layer to `ErrorCode::NoWriteAccess`; the registry stays
  out of this crate, D-062) plus `ProbeFailed`; every variant carries
  the lossy target path, the io-backed ones the `io::Error`
  (`path()`/`io_error()`/`os_code()` accessors, hand-rolled impls,
  D-017). Degenerate inputs (no parent folder or file name — a
  relative bare name, a root, `..`) are refused as
  `ProbeFailed` (Unsupported) BEFORE anything is created: a guessed
  folder (the process's working directory) must never be probed, and
  never littered.
- WINDOWS: no cfg anywhere in the production code — std's
  `Permissions` and `OpenOptions` are the portable facts;
  `cargo check --target x86_64-pc-windows-msvc --all-targets` is
  clean. The T-F5 fixture uses `set_readonly`, so the acceptance test
  itself runs on the Windows CI leg unchanged.
- KNOWN OPEN (the D-088/D-090/D-092 posture): lib.rs's `//!` module
  list and ARCHITECTURE.md's fs enumeration stay untouched (exactly
  one `pub mod write_access;` line added — parallel siblings are
  landing in `atomic.rs`); the orchestrator's refresh reconciles both
  lists.

Tests: 17 unit tests + 1 doctest, inline in the module, through the
public function on real temp fixtures (the private token seam drives
the collision paths deterministically): the T-F5 acceptance (read-only
file → `FileReadOnly`, `os_code` None, no probe debris — the order
itself is pinned by the both-refuse test); a writable target in a
writable folder passes with zero debris and its content untouched;
the absent-target creation case (nothing created); the symlink follow
(judged by the real file); a 0o555 folder → `FolderNotWritable` with
EACCES = 13 (Unix-gated, permissions restored before the assertions,
loud root skip — the D-088 test posture); an untraversable 0o644
folder → the refused metadata probe is `ProbeFailed` with EACCES
riding, never a fabricated rights verdict; the both-refuse order pin;
a missing folder → `ProbeFailed` with `NotFound` riding; the
degenerate relative name → `ProbeFailed` (Unsupported) with the
working directory scanned clean of staging-grammar debris; the
collision retry (a pre-created `...eunomia-00000000.tmp` passes on
`...00000001`, the fixture survives, the probe cleans up after
itself); token exhaustion → `ProbeFailed` with the last collision
riding; the refusal classification pinned kind by kind; the token
grammar over 100 draws; the error contract (Display texts, `path()`,
the io/fact split, the `source()` chain); and a Linux-gated non-UTF-8
target name (the lossy spelling probes its folder fine).

### D-096 — `projects_list`: the dirty flag served at its honest DB-022 boundary, availability probed after the transaction, and `ProjectsError` mapped at the command boundary — 2026-11-20

Problem: T-072 turns §7.10's `projects_list` into the first command
over the T-071 projects repository (the MAIN-028 column fields: label,
color, availability, dirty, `last_synced_at`). Five points were open:
(1) where the column's `dirty` fact comes from — DB-021 compares
`last_synced_hash` with the _current_ build hash, DB-022 assigns the
current hashes to a background pipeline that has not landed, and the
store exposes no library/assignment reads to build one from; (2) how
the command reaches the D-084 availability check — `eunomia-fs` owns
it, but src-tauri had no `eunomia-fs` edge; (3) how `ProjectsError`
maps onto the §7.10 wire, given D-093 froze the `StorageError`
surface and named this task as the mapper; (4) whether the command
logs failures when the logging facade has no `projects_list` function;
(5) what "archived by filter" means on the wire.

Solution: `projects_list(include_archived: Option<bool>)` in
`commands.rs`, registered in `collect_commands!`, returning
`Vec<ProjectColumnEntry>`.

- _dirty: served at its honest boundary, not computed._ DB-014's
  archived half is the one part of DB-021 the §6.2 row decides alone
  (archived is never dirty). For active projects, no database this
  app can produce holds unsynced changes yet: nothing writes
  assignments, library content or sync hashes (`last_synced_hash`
  starts NULL with no writer), so DB-021's exception sentence shields
  every reachable row and the honest verdict is `false`. The live
  fact's owner is DB-022's pipeline (`state://dirty`, MAIN-028's dot
  icon), whose inputs — the library and assignment repositories, the
  ALG-001 snapshot — do not exist in the store; hand-writing SQL over
  those tables from the command layer would violate D-010/D-017's
  schema ownership and duplicate the snapshot task. The seam is
  `dirty_verdict` in `commands.rs`: the DB-022 task (or the first
  assignments command) feeds `eunomia_core::dirty::is_dirty` the real
  build facts there — the DTO field is that pipeline's arrival point,
  and a silent constant past that point is what the seam's docs
  forbid. `eunomia-core` is deliberately NOT added to src-tauri:
  nothing honest can call the predicate with yet.
- _Availability: the D-084 check, read-only, after the transaction._
  `eunomia_fs::paths::check_project_available` per project (DB-032's
  window-open check point), in a `spawn_blocking` that runs AFTER the
  `Db::run` read: four metadata probes over live folders are blocking
  file I/O (SYS-001) and must never hold the one database lock — a
  dead drive delays only this response. An unavailable folder is the
  entry's `available: false` plus `unavailable_reason` (the wire's
  own snake_case names of the D-084 verdicts — `missing`,
  `not_a_directory`, `not_readable`, `no_git_entry`: D-09/MAIN-028's
  "icon and text" needs the class), never an error of the read. The
  cost: one `eunomia-fs` dependency edge in `src-tauri/Cargo.toml`
  (the rusqlite/jiff/tempfile precedent — an edge, not a version; the
  workspace member was already resolved), flagged to the orchestrator
  because Cargo.toml sits at this task's scope edge.
- _`ProjectsError` mapped at the boundary, as D-093 anticipated._
  `projects_store_to_app_error`: `Sql` → the shared base classifier;
  `Member` → the recorded D-037 fallback with a `ProjectsCorrupt`
  label and a class-only diagnostic naming the column (the T-027
  round-2 rule — a tampered cell is arbitrary database text and never
  reaches `params`/`os_message`); the write-path outcomes
  (`DuplicatePathKey`, `NotFound`, `ReorderMismatch` — unreachable
  from a read) keep the fallback with a `ProjectsError` label and
  class-only text, so a stored `path_key` or id never travels either.
  Folding the two store error families together stays D-093's
  recorded follow-up for a store-side task allowed to edit `src-tauri`.
- _No failure log edge — a documented gap, not an omission._ The
  facade's vocabulary grows per command task
  (`logging::activity_list_failed` is T-027's) and `logging.rs` is
  outside this card's file scope; the follow-up is whichever task
  adopts the Projects module's write commands into the facade. A
  successful read is never journaled (the `activity_list` rationale).
- _`include_archived` is include-semantics, one scope._ `None`/`false`
  → `ListScope::Active` (the matrix's columns); `true` → `ListScope::All`
  — the same ordered read widened, not a second query: §6.4's archive
  keeps a row's position, so archived entries appear at their old
  places carrying `state: "archived"` for the D-10 restore screen to
  filter on. A plain argument, not a query DTO: one boolean has no
  typo-surface to reject loudly (the `deny_unknown_fields` rationale)
  and tauri rejects unknown argument names itself.
- _The DTO carries what the column renders plus what it calls back
  with._ Beyond the card's five fields: `id` (every later project
  command addresses it), `name` (DB-041/NFR-020's tooltip), `state`
  (the filter's consumer) and `unavailable_reason` (D-09's text
  half). No DB type crosses the boundary (the `ActivityEntry` rule).
- _Tests._ 6 IPC tests over the mock runtime with projects seeded
  through the T-071 repository and real `.git` folders for the probe:
  the sort_order answer after a repository reorder plus the full
  field set and the unavailable-folder flag (not an error); the
  archived-filter widening with DB-014's never-dirty pin and the
  explicit-false twin; the empty-database empty list; the
  no-database honesty pin; the corrupt-cell fallback envelope (via
  the `ignore_check_constraints` seam, surfaced by the widened read —
  the default `state = 'active'` filter shields a tampered row from
  decoding); the registration pin. 2 unit tests pin the error mapper:
  the corrupt cell's label and class-only diagnostic, and the
  unreachable write outcomes keeping stored identifiers off every
  wire surface.

### D-095 — The atomic write: the §9.5 pipeline in `atomic`, the write-through gap in std's Windows rename, and the injected seams — 2026-11-20

Problem: T-066 implements spec §9.5 (OUT-007, ERR-002, ERR-003) as
`eunomia-fs`'s `atomic` module: stage in the same folder, copy the
Unix mode, write and `sync_all`, replace in one rename, `fsync` the
parent folder (Unix), re-read and verify SHA-256, delete the staging
file on every error while the original stays whole. The card fixes
the errors (`E_WRITE_FAILED`, `E_VERIFY_FAILED`, `E_DISK_FULL`) and
leaves open: (1) the Windows replace — the card allows "MoveFileExW
without retries or a std equivalent", and the two are NOT the same:
std's `fs::rename` on Windows is `MoveFileExW` with
`MOVEFILE_REPLACE_EXISTING` only (verified in the toolchain's
`sys/fs/windows.rs`), while §9.5 step 4 pins `MOVEFILE_WRITE_THROUGH`
too; (2) the staging token's entropy without a `rand` crate; (3) the
write-volume disk-full arithmetic — §9.4's 20% margin sentence is
about the BACKUP volume, nothing pins a margin for the write; (4) how
tests fail each pipeline step without touching other files; (5) what
"the original stays unchanged" (§9.5 step 7) can even mean for the
two steps that run AFTER the rename.

Solution: `write_target_file(target, contents, free_space)` plus the
typed `AtomicWriteError` (`NameFailed`/`DiskFull`/`WriteFailed`/
`VerifyFailed`, the D-017/D-084 hand-rolled shape with
`io_error()`/`os_code()`), `WrittenFile` (path/size/digest), and
`WriteStep` — the step fact §10's log line names next to the OS code
(`DiskFullFailure` carries it as `Option<WriteStep>` too, `None` for
the pre-write verdict; `NameFailed` maps onto `E_WRITE_FAILED` — a
contract-violation shape the intended D-092 caller cannot produce).

- WRITE-THROUGH, NOT STD: the Windows arm of the replace is a direct
  `MoveFileExW(REPLACE | WRITE_THROUGH)` through `windows-sys` — the
  crate's second owner-sanctioned FFI call after D-089's probe (the
  task brief pre-wires `Win32_Storage_FileSystem` for exactly this),
  one `unsafe` block with every fileapi.h invariant stated at the
  call site (the D-058/D-089 discipline). Unix uses `fs::rename`
  (rename(2) replaces atomically). The whole replace funnels through
  ONE cfg-split `replace_target`, and `WriteFailed` carries the raw
  OS error — T-067's sharing-violation loop (100/300/900 ms →
  `E_FILE_LOCKED`) drops into that single function with no new
  plumbing; this task ships no retries and no `E_FILE_LOCKED`, per
  the card boundary.
- TOKEN WITHOUT RAND: 8 lowercase-hex characters (D-088's alphabet)
  as the first 8 of a SHA-256 over (nanoseconds, pid, a process-wide
  `AtomicU64` counter), digested through core's `Sha256Hex` — D-017's
  allow-list stays intact; the token needs uniqueness, not
  unpredictability (`create_new` makes a collision an error, never
  corruption). A collision retries under a fresh token, at most 3
  attempts, then `WriteFailed`/`TempCreate` with the last attempted
  name and the `AlreadyExists` refusal — D-088's "an error and a
  retry" honored without unbounded looping.
- DISK-FULL, TWO SOURCES, NO MARGIN: the demand is exactly the
  content's byte count (the 20% margin is §9.4's backup-volume rule;
  no write margin is pinned anywhere), decided by
  `free_space::check_space` through the D-089 closure seam —
  production passes `|| free_bytes(folder)`, tests a number. Only the
  shortage verdict aborts; a FAILED probe does not block the write
  (the write itself is the authority, ERR-003's posture: a dropped
  drive reports its own failure). Mid-write `ENOSPC`
  (`io::ErrorKind::StorageFull`, std maps errno 28 on Unix) from any
  write-path step classifies into the same `DiskFull` — §10's row is
  about the situation, not the probe.
- STEP 7 AFTER STEP 4, HONESTLY: "the original stays unchanged" can
  only bind up to the replace. For the folder `fsync` and the verify
  (the two post-rename steps) the file is WHOLLY the new content —
  old-or-new, never partial (OUT-007's actual invariant) — and is
  never deleted: it is the user's data, §10's remedy for
  `E_VERIFY_FAILED` is the backup, and a failed check leaves the
  unverified new bytes plus the caller's backup rather than an empty
  path. The staging file's removal is attempted on every failure; a
  refused removal rides in `cleanup` (Some) and the leftover is
  grammar-conforming debris the `temp_files` sweep owns (T-F2,
  mutation-verified).
- FAULT INJECTION, cfg(test)-ONLY: spec 15.1 names "failure points in
  `fs`" — implemented as a thread-local armed fault (one-shot, per
  step: TempCreate/ModeCopy/WriteSync/Replace/DirSync/VerifyRead,
  plus a corrupt-target-before-verify action and a fixed token queue
  for the collision fixtures), consumed at the production step call
  sites under `#[cfg(test)]` statements. The shipped pipeline has no
  hooks (the `faults` seam does not even compile outside `cfg(test)`),
  the public API carries no test parameter, and the cleanup paths
  under test are the production ones. RAII guard (`Armed`) disarms on
  drop so a test cannot leak a fault.
- MODE COPY, FULL MASK, KERNEL LIMIT STATED: `chmod` on the staging
  file (no umask applies) before writing; a missing target is a
  first-sync creation and keeps the platform default. The mask is the
  full `& 0o7777` — with the honest limit that the Linux killpriv
  policy strips setuid/setgid from any file at write time (the
  staging file is written after the chmod), so those bits cannot
  survive a non-root atomic replace on modern kernels no matter what
  a writer does; sticky rides along. Handle discipline follows
  `backup`: the staging handle is dropped before the rename and every
  error-path removal (Windows sharing).
- SEAMS CONSUMED, NOT REBUILT: the staging name is
  `temp_files::tmp_file_name` (the one grammar the sweep matches),
  the expected digest is core's `Sha256Hex`, and the caller owns
  §9.5 step 1 — `write_target_file` receives the RESOLVED path
  (D-092) and re-derives the lossy file name for the grammar, so a
  `CLAUDE.md` link writes the real file and the link survives (T-F6,
  pinned by a caller-side test through `resolve_target_path`).

Tests: 26 colocated test functions in `atomic.rs` (25 run on the
Linux/macOS legs; 1 is Windows-only) — the happy paths (new,
overwrite, empty, a 64 KiB+7 multichunk write against a
Python-hashlib oracle, pinning the streaming verify's chunk
boundary) with folder-listing no-debris asserts,
the space decisions (short, exact boundary, probe-failure
passthrough, the real `free_bytes` closure smoke), `ENOSPC`→
`DiskFull` (target path + step pinned) with the staging file cleaned,
a failure at EVERY step (original intact + staging removed, or the
new content whole for the post-rename steps), the refused cleanup
(r-x folder, root-skipped, leftover proven sweepable by
`is_app_tmp_file`), the corrupted settled file caught with the
actual digest of the corrupted bytes (the check reads the disk, not
the buffer), collision retry/exhaustion with pre-created names
pinning the grammar, the mode copy (0o604 plus the sticky bit 0o1604
— the one special bit the kernel's killpriv policy lets survive a
write; setuid/setgid are stripped by the kernel at write time
whatever the module does, so the full-mask ruling is documented with
that limit and pinned through sticky), the symlink seam, the
degenerate shapes (bare relative target, NUL name → `NameFailed`),
and the error contract (Display/source/os_code).
Mutation-checked: skipping the cleanup, weakening the digest
comparison, dropping the mode copy and disabling the space check each
fail exactly their tests. Windows-target and macOS-target clippy of
the crate's tests pass (the MoveFileExW arm, the cfg-gated faults and
the unix-only steps); the Windows replace arm's behavior runs on the
CI Windows leg (`the_windows_replace_swaps_an_existing_file`).

Round-1 review (deepseek-v4.1-flash) — CHANGES REQUESTED with no
P0/P1, three P2 and four P3; all three P2s fixed plus P3-1..P3-3
(P3-4, the stale `lib.rs` skeleton sentence, is the recorded
D-087/D-089/D-092 concurrent-siblings posture for the orchestrator):

- P2-1: the mid-write `ENOSPC` verdict reported the STAGING path in
  `DiskFullFailure.path` (its own doc says the target; §10's path is
  the project fact and the staging file is already deleted) — `abort`
  now takes the target and `DiskFullFailure` gained
  `step: Option<WriteStep>`; the ENOSPC test pins both.
- P2-2: the module docs claimed a one-to-one §10 mapping with a
  fourth variant unaccounted — `NameFailed`'s folding into
  `E_WRITE_FAILED` is now documented in the error section and here.
- P2-3: the streaming verify was never exercised past one chunk —
  the 64 KiB+7 Python-oracle test above was added.
- P3-1: the mode-copy error path removed the staging file with its
  handle still open (against the module's own stated discipline) and
  its two failures disagreed on `path` — the handle is now dropped
  before that removal, and both report the staging path.
- P3-2/P3-3: the full-mask mode ruling is now pinned (sticky
  survives; setuid/setgid cannot — kernel killpriv, stated) and the
  test-count/wording errors in this entry were corrected.

Round-2 review (hy4-flash-free) — APPROVED, no P0/P1/P2; its one P3
(the multichunk test's comment claimed `write_all`'s partial-write
branch, which a single regular-file write need not exercise) was
taken: the comment now claims only the verifier's chunk boundary. The
reviewer independently recomputed the 64 KiB+7 oracle and verified
the `MoveFileExW` call against the pinned windows-sys 0.61.2 source.

### D-097 — The library repository: constructive DB-010, the position move as the DB-005 primitive, delete-with-replacement as one boundary, and `updated_at` aligned with the snapshot notion — 2026-11-20

Problem: T-081 adds CRUD for the three library tables
(`library_items`, `instruction_variants`, `project_instructions`) on
top of the v1 schema (T-016). Open questions: how the DB-010
"at least one active variant and exactly one active main" invariant —
only half of which the partial index `ux_variant_main` enforces — is
kept without a validator pass; which reordering primitive DB-005
implies for the library (a full-permutation `reorder` like projects,
or something else); where LIB-023's "replacement chosen per project,
main successor required, everything in one transaction" lives; and
which writes restamp `updated_at` when §9.8/D-16 explicitly exclude
the main flag and the library order from snapshots.

Solution: `eunomia_store::library`, one module for the three tables
(the projects/T-071 repository pattern: store-local vocabularies with
`as_str`/`from_member`, hand-rolled `LibraryError` returned directly —
the D-093 `StorageError` surface freeze — every function joining the
caller's transaction, ids/timestamps from `values`, Cargo.toml
untouched, D-017's no-core-edge rule intact).

- _DB-010 made constructive, not validated after the fact._ The API
  cannot talk its way out of the invariant: the first active variant
  of an instruction must be its main (`MainRequired`), a second
  active main is typed (`MainVariantExists`, the index stays the
  floor), `set_main_variant` demotes-then-promotes (never two active
  mains mid-sequence; promote-first could trip the partial index in
  either row order), and `delete_variant` refuses the last active
  variant (LIB-025). A bare `create_item` on an instruction
  deliberately leaves DB-010 unmet until the main variant lands: the
  LIB-002 create dialog produces the pair, and the command task
  composes both calls in its one transaction — the same
  producer-composition boundary as archive + exclusion (D-093); the
  transient window is invisible inside the transaction and is
  documented on `create_item`.
- _`move_item(id, to_position)` instead of a projects-style
  full-permutation `reorder`._ DB-005's own wording is "reordering
  renumbers the affected range" — a single-item move to a target
  position is the primitive the drag-and-drop UI (LIB-003/004) and
  the branch move (LIB-032, a command-layer composition) need, and it
  cannot express a non-permutation: the target range is validated
  `1..=COUNT(*)` (`PositionOutOfRange`, table untouched), a
  same-position move writes nothing. Creation appends `MAX+1` inside
  the single INSERT (the `projects::create` shape), a heading's
  physical delete closes the gap by shifting the tail, hide keeps the
  position (the archive precedent — DB-011's 1..=N is table-wide, not
  active-only).
- _LIB-023's dialog flow is one repository boundary._
  `delete_variant(id, replacement: Option<&str>)` moves the
  assignments to the replacement _before_ the soft delete (DB-012; no
  assignment disappears silently), clears `is_main` on the dead row
  (a deleted row must not count as the main; demoting first also
  keeps `ux_variant_main` satisfiable mid-sequence), and promotes the
  replacement when the deleted variant was main. `None` defaults to
  the current active main — the dialog's own default — and is
  rejected for a main variant (`ReplacementRequired`); a replacement
  that is not an active sibling is `ReplacementInvalid`; LIB-025
  (`LastVariant`) is checked first as the absolute refusal. The NOCASE
  name twin (`ux_variant_name`'s scope) is typed on create, rename
  and restore via pre-checks — the table's several unique indexes
  share one extended code, so SQLite's code alone cannot
  disambiguate them and message matching stays banned; the
  pre-check-plus-index safety net is the same one-connection argument
  as the rest of the crate (D-037).
- _`updated_at` follows the snapshot notion of a real change (D-16)._ Renames,
  content edits, heading-level changes and every state transition
  (hide/restore, delete/restore) restamp; a pure position move, the
  tail renumbering it causes, and a main-flag promotion restamp
  nothing — the `projects::reorder` precedent generalized into a
  rule, so the library's "change date" column (LIB-001) does not
  churn for hundreds of rows on one drag.
- _Assignments._ `assign` pre-checks and types what the composite PK
  and FK only floor: `AlreadyAssigned` (D-02, one variant of one
  instruction per project), `VariantNotAssignable` (DB-012: unknown,
  cross-instruction or soft-deleted variant), `NotAnInstruction`
  (MAIN-001: headings have no cells), `NotFound` for unknown
  project/instruction ids. `reassign` changes one project's variant
  only (MAIN-005), checks the assignment's existence before the
  variant, and re-assigning the held variant is a verified no-op;
  `unassign` removes without touching the variant (LIB-020); hidden
  instructions keep and gain assignments (DB-013 — the build filter
  is the caller's); `list_assignments_for_project` orders by
  `library_items.position` (OUT-001: the file order equals the
  library order).
- _The two §6.9 clauses D-076 assigns to "the library create/update
  STORE task" — this one — landed here (round-1 review)._ (a) The
  main-instruction-name uniqueness among _visible_ instructions,
  case-insensitive (a cross-row query the field validators cannot
  run): `DuplicateInstructionTitle`, probed on `create_item`,
  `update_item` and `restore_item` — a hidden twin does not block
  (not visible), and the conflict surfaces at restore, mirroring
  `restore_variant`'s name check; heading titles are exempt (the
  §6.9 row is the instruction name's). (b) The save-time CRLF→LF
  fold of variant text (CRLF only — the lone-CR/BOM/edge pass stays
  build-time with ALG-002): the stored form is the folded form, so
  the DB-006 CHECK measures what is actually stored — a raw text over
  the limit whose folded form fits is accepted (D-076's recorded
  asymmetry, pinned by a test).
- _Round-1 hardenings._ The `NoActiveMain` typed outcome replaces
  `delete_variant`'s default-replacement panic: the main flag is
  reachable by a plain out-of-band `UPDATE … SET is_main = 0` that
  violates no CHECK, so DB-010 can be broken without any corrupt
  cell — surfaced per DB-017 (no automatic repair) instead of guessed
  around. `delete_variant`'s replacement is documented as
  one-for-all: LIB-023's per-project choices compose `reassign` calls
  after the delete in the same transaction (pinned by a test).
  `restore_item` on a heading (always active) is the documented
  idempotent no-op, and the ABORT pin now exercises a genuinely
  rejected statement (the §6.2 CHECK floor), not only a typed
  pre-check refusal.
- _Tests._ 3 unit tests in the module (vocabulary round trip, the
  heading-level CHECK matrix decode, error Display shapes) and 46
  integration tests in `tests/library.rs` on a real `open_db` +
  `run_migrations` database: full-column round trips for all three
  tables with UUIDv7/DB-002 pins; the gapless sequence under append,
  middle/end moves, heading delete and hide; the constructive DB-010
  rules and every LIB-023/025 rejection shape with the surviving rows
  verified; assignment CRUD with the D-02 duplicate, the three
  DB-012 causes, MAIN-005 isolation and the position-ordered read;
  the DB-006 boundary through both create and update plus the
  CRLF-fold measurement; the §6.9 title-uniqueness NOCASE scope
  (create, rename, restore-into-twin, hidden twin, heading exempt);
  corrupt cells (kind/state/heading_level/variant state) as typed
  `Member` through the `ignore_check_constraints` seam; and the
  transaction contracts (move and delete-variant roll back as sets, a
  failed statement — CHECK-rejected, not pre-checked — keeps the
  transaction usable).

### D-098 — ALG-053 classification in `eunomia-fs::scan_classify`: the registry is an injected snapshot, the caller's canonical-root obligation, and set overlaps resolve by the fixed priority known > archived > excluded — 2026-11-20

Problem: T-075 implements the second half of ALG-053 (the first is
T-061's walker, `scan`; the consumer is PRJ-002/005/006's scan
window): for every found repository, compute the `path_key` and the
class `new`/`known` (active)/`archived`/`excluded`. Open questions:
where the membership data comes from, given the crate boundary (fs
must not depend on store); what to do when a `path_key` sits in
several registry sets at once — a typed error or a deterministic
answer; and what result shape the future command task (T-076)
should serialize into the throttled progress event.

Solution: `crates/eunomia-fs/src/scan_classify.rs` (registered in
`lib.rs`), pure and synchronous over the walker's output —
`classify_key` (the lookup), `classify_repository` (key via
`paths::path_key` + class; ALG-053's sentence in one call),
`classify_entry` (the envelope), over `ScanRegistry` /
`ScanClass` / `ClassifiedRepository` / `ClassifiedScanEvent`.

- _The registry is a parameter, not a dependency._ `ScanRegistry`
  holds the three membership sets — the `path_key`s of active
  projects, archived projects, and `project_exclusions` rows — as
  plain `String`s the caller (T-076) loads from `eunomia-store`
  (D-093's `list`/`list_exclusions`). The snapshot is taken per
  scan, so adds/archives committed mid-walk cannot flip a class
  mid-run; `new` is the fall-through when no set knows the key.
- _Overlaps are a priority, never an error._ ALG-053 says nothing
  about a key in two sets, and the honest reading is that overlaps
  are the NORMAL state: archiving a scan-found project keeps the
  archived row AND writes an exclusion for the same key (§6.4,
  PRJ-017; D-093 pins the coexistence with an integration test), so
  `archived ∩ excluded` is the steady state of every archived
  scan-found project — a typed error would turn the common case
  into a failure path. The fixed priority: `known` first (an active
  project is the truest fact; DB-015's full UNIQUE keeps a key from
  being both an active and an archived row, and D-10's restore
  removes the exclusion), `archived` over `excluded` (the project
  still exists; DB-015's restore offer answers, not the exclusions
  screen), `excluded` only for keys with no project row. Pinned by
  tests for each pair and for the three-way overlap.
- _The key is `paths::path_key` AS GIVEN, no canonicalization — and
  the canonical root is the CALLER'S obligation._ The walker does
  NOT canonicalize: it accepts any root verbatim, and a symlinked
  root is a documented, deliberately supported input that is
  traversed as the folder it names while every reported path KEEPS
  the link spelling (`scan`'s own root test pins that) — so "the
  scan's output paths are canonical by construction" is FALSE as a
  walker guarantee. What holds instead: classification is pure
  (pinned: a missing path classifies by its text key, no
  filesystem), and the IDENTITY it computes matches the stored one
  exactly when the command layer canonicalizes the scan root with
  `paths::canonicalize` BEFORE `scan_repositories` — the same
  function whose output the add flow keys, so both sides spell the
  identity alike. T-076 owns that obligation (it owns the root
  choice); this module documents it in its docs and pins the
  failure mode with a test (a symlinked root classifies `new`
  against the canonical registry) so it cannot be assumed away.
  Below a canonical root the no-follow rule (D-07) makes the
  descendant paths canonical text, and DB-031's canonical aliasing
  stays the add flow's canonicalize-first job; alias spellings the
  key itself collapses (separator runs, NFC/NFD under the folding
  rules) classify as one repository.
- _Two result shapes, both owned here._ `ClassifiedRepository` is
  OWNED (path verbatim per D-086, key, class) — found repositories
  are the payload T-076 accumulates for PRJ-002's result list.
  `ClassifiedScanEvent<'a>` is the BORROWED 1:1 envelope of every
  `ScanEvent`: repositories gain their facts, plain folders (the
  "folders checked" counter) and skips pass through by reference
  with the walker's reason verbatim (OS code intact) — zero
  allocation for most events, ready to serialize. The module docs
  also keep the two "excluded" facts apart: the walk's name-based
  `SkipReason::Excluded` (ALG-051, pre-classification) vs the
  registry's `ScanClass::Excluded` (a found repository in
  `project_exclusions`).
- _Scope._ No new dependencies (`Cargo.toml` untouched); `scan.rs`,
  `paths.rs`, store and app layers untouched — the throttle and
  cancellation of ALG-053's remaining clauses stay the walker's and
  T-076's.

Tests: 11 unit tests + 4 doctests in `scan_classify.rs`. The
acceptance criterion end to end (a real tree — two repositories
plus a nested one — scanned twice through `scan_repositories`:
empty registry → all `new`; the first scan's keys as the active set
→ the rescan classifies every repository `known`, none `new`; the
harvested keys pinned distinct, name-bearing and separator-clean so
a degenerate key function cannot pass); each set gives its class
(with the key pinned spelling-independently); the three overlap
pins (active wins; archived∩excluded → `archived`); alias spellings
(separator collapse; NFC/NFD under an explicitly-fed folding rule —
a doc-pin of the cross-module premise where the classifier consumes
it); purity (a missing path classifies); the symlinked-root failure
mode (Unix: the canonical registry key is missed, `new` — the
caller's obligation made concrete); empty registry; and the
envelope's pass-through for every `SkipReason` (reason kind, OS
code, and the shared `path()` accessor) and for plain folders.

### D-099 — The Windows replace retries: the 1+3 schedule on the raw-attempt seam, classification by raw Win32 code, and the three-level test ladder for a Windows-only behavior — 2026-11-20

Problem: T-067 layers §9.5 step 4's locked-replace discipline
("sharing violation or access denied → up to 3 retries with pauses of
100, 300 and 900 ms, then `E_FILE_LOCKED`") onto T-066's `atomic`
(D-095), which shipped the replace without retries and left the seam
deliberately single (`replace_target`). Open questions: (1) the
attempt arithmetic — "3 retries" is 4 attempts (initial + 3) with 3
pauses, not 3 attempts; (2) which OS errors retry — the spec names
two conditions, Win32 spells them 32/5, and MoveFileExW advice often
adds ERROR_LOCK_VIOLATION (33); (3) where the `E_FILE_LOCKED`
classification lives — §10 gives it its own code, but `WriteFailed`
was the pipeline's only replace failure shape; (4) how a
Windows-only behavior is tested where CI runs Linux (a real sharing
violation needs a Windows handle opened without FILE_SHARE_DELETE);
(5) how the 1.3 s of pauses stay out of the test suite without a
public test parameter.

Solution: a platform-neutral `replace_with_lock_retries(attempt,
pause)` around the raw OS call, plus a typed fs-level refusal —
`AtomicWriteError::FileLocked(FileLockedFailure)` with the module's
step/path/source/cleanup contract (and `retries: usize`), matching
the app layer's existing `ErrorCode::FileLocked` wire name.

- 1+3 SCHEDULE, NEUTRAL CORE: the retry loop takes the attempt and
  the pause as closures, so it is plain code both platforms compile
  and the Windows arm wires `perform_replace` (the cfg-split raw
  call: rename(2) / the D-095 MoveFileExW) plus `thread::sleep`. One
  initial attempt; each of the three pauses (100/300/900 ms) buys one
  retry; a success at any attempt returns at once and never waits.
- EXACTLY TWO RETRIED CODES, BY RAW VALUE: `ERROR_SHARING_VIOLATION`
  (32) and `ERROR_ACCESS_DENIED` (5), classified by
  `raw_os_error()` — spelled as local constants because the crate's
  windows-sys features are exactly `Win32_Storage_FileSystem`
  (D-095; the named ERROR_* constants live in the not-enabled
  `Win32_Foundation`) and Cargo.toml is outside this task's file
  scope. ERROR_LOCK_VIOLATION (33, byte-range locks) is deliberately
  NOT retried: the spec names two conditions, and 33 is neither.
- HONEST EXITS: any non-lock refusal ends the loop immediately as the
  plain OS fact — including one that arrives AFTER lock refusals (the
  lock lifting into a different failure is reported as itself, never
  masked as "still locked"; pinned at both the early and the
  post-schedule position). Exhausted locks exit as the private
  `ReplaceRefusal::Locked { source, retries }`, classified to
  `FileLocked` in the step-4 failure path (`abort_replace`, the
  sibling of `abort`); everything else flows through `abort`
  unchanged, so `E_DISK_FULL`/`E_WRITE_FAILED` semantics are intact.
  The variant is `#[cfg(any(windows, test))]`: Unix production never
  constructs it (§10 pins the row to Windows; rename(2) has no
  sharing violations) and a never-constructed variant would fail the
  workspace's `-D warnings`.
- THREE-LEVEL TEST LADDER: (1) mechanics — the loop itself against
  injected 32/5 refusals and a recording clock (attempt count 4,
  pauses exactly 100/300/900, success-after-N, non-lock immediate
  exit at both positions — no real sleeping anywhere); (2) pipeline —
  a cfg(test) fault (`LockedReplacePlan`: Exhaust or UnlockAfter+N)
  routes the replace through the REAL loop with a simulated locked
  attempt whose eventual success is the real `perform_replace`, so
  the typed `FileLocked` refusal, the staging cleanup and the intact
  original run on the Linux legs; (3) the real Windows criterion —
  T-F8's editor shape as a `std` handle with
  `share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)` (no
  FILE_SHARE_DELETE; OpenOptionsExt, no new FFI): the actual
  MoveFileExW refusals ride the actual schedule (pauses recorded by
  the same clock seam, so the CI leg does not sleep 1.3 s), then
  `E_FILE_LOCKED` with the original intact and the staging file
  gone, and the idempotent §10 retry lands the write after the
  holder closes. The one-shot fault hook still bypasses the loop
  (it refuses the STEP, not a syscall attempt) — pinned by a test.

Tests: 13 new test functions in `atomic.rs` — 12 compiled on the
Linux legs (one of them Unix-only: the refused-cleanup fixture) and 1
`#[cfg(windows)]` (the T-F8 criterion) — 39 total in the module (37
run on the Linux legs). Mutation-checked: a wrong pause value, a loop
that never retries, and a final-refusal masking-as-Locked each fail
their tests (the masking mutant survived the first battery and forced
the post-schedule honesty test); the round-1 review's two surviving
mutants — widening the retried set with ERROR_LOCK_VIOLATION (33) and
dropping the Locked arm's staging cleanup — forced the 33 pin inside
the non-lock test and the refused-cleanup fixture
(`ExhaustWithRefusedCleanup`, the FileLocked twin of the WriteFailed
folder-lock test).
Windows-target check and clippy (`--all-targets --target
x86_64-pc-windows-msvc -D warnings`) pass — the FILE_SHARE_* test
constants, the retry arm and the Windows-only faults items
(`arm_pause_recording`, `record_lock_pause_if_armed`, gated
`#[cfg(windows)]` so Linux test builds stay warning-free) all
compile on both legs.

Round-1 review (deepseek-v4.1-flash) — CHANGES REQUESTED, no P0/P1,
two P2, four P3; both P2s fixed plus P3-1/P3-2 (P3-3 documented,
P3-4 declined):

- P2-1: the deliberate non-retry of ERROR_LOCK_VIOLATION (33) was
  documented but unpinned — the reviewer's `| Some(33)` mutant
  survived all 201 tests. Fixed: `a_non_lock_refusal_is_not_retried`
  iterates `[13, 33]`; the mutant now dies.
- P2-2: `FileLockedFailure.cleanup = Some(_)` and its Display phrase
  were never executed — `cleanup: None` and Display-clause mutants
  both survived. Fixed: `LockedReplacePlan::ExhaustWithRefusedCleanup`
  (the FileLocked twin of the WriteFailed folder-lock fixture) plus
  the refused-cleanup test (root-skipped like its twin); both mutants
  now die.
- P3-1 taken: the real Windows test accepts `Some(32) | Some(5)` —
  the OS's choice between the two §9.5 codes, observed only on the CI
  leg, must not redden it. P3-2 taken: the entry's test counts were
  corrected. P3-3 documented only: the single-fixture discipline of
  `Armed`'s combined drop (predating this task) is now stated on the
  guard. P3-4 declined: `lib.rs`'s stale skeleton sentence is the
  recorded D-095 KNOWN OPEN posture, outside this task's file scope
  and inside the sibling task's window.

Round-2 review (hy4-flash-free) — APPROVED, no findings at any level;
it independently re-ran six mutants in a scratch copy (all die),
re-measured the test counts (13 new / 39 total / 37 on Linux legs),
and confirmed the root-skip is the documented house precedent, not
evasion.

### D-100 — `dialog_pick_folder` and the SEC-003 session allowlist — 2026-11-20

Problem: T-073 turns §7.10's System-module `dialog_pick_folder`
(D-009's fixed name) into the first Rust-side use of the dialog
plugin, and §5.6 SEC-003 ("a path must be a registered project or
must have been picked in the system dialog in this session") into
code. Five points were open: (1) where the "picked this session"
list lives and who owns it; (2) what "the path matches" means —
raw text, canonical form, or the DB-030 `path_key`; (3) which error
a non-allowlisted path gets, and which one a pick whose folder
vanished between the dialog and the bookkeeping gets; (4) how the
native dialog is tested without opening one; (5) whether the picker
works in recovery mode (no `Db` managed).

Solution: `session_paths.rs` (new module) + the command in
`commands.rs`, registered in `collect_commands!`.

- _The session list is managed state, not a static._
  `SessionPaths(Mutex<Vec<PathBuf>>)` holds the canonical forms of
  this process's picks — managed by `setup` before anything that can
  fail, so the picker answers in any mode. A `LazyLock` static was
  rejected because the IPC tests need per-app isolation (one test's
  pick must not validate in another's app). `std::sync::Mutex`, the
  inverse of D-037's async-mutex reasoning: the guard is held for a
  vector lookup/insert, never across an await or I/O; poison is
  recovered (`into_inner`) — the list is plain data. Duplicates
  collapse; there is deliberately no `clear` — a session is one app
  process, the process exit is the clear.
- _Matching is canonical-form equality; both halves go through
  DB-030's identity._ The UI path is canonicalized with
  `eunomia_fs::paths::canonicalize` (DB-030, read-only reuse — the
  crates untouched), so DB-031's aliasing (a symlink and its target
  are one folder) falls out before any comparison, and an aliased
  spelling of a pick (`…/sub/../picked`, or a real symlink) validates.
  Both halves then key the canonical form with
  `eunomia_fs::paths::path_key` — ONE folding rule (the round-1
  review finding: a byte-exact session half beside a folded project
  half would reject a case/NFD-differing spelling of a pick that the
  project half accepts — safe, but inconsistent). Inherited edge,
  recorded by the round-2 review: on a case-SENSITIVE volume under a
  case-FOLDING rule (macOS APFS), a never-picked sibling differing
  only in case folds onto a picked folder's key — DB-030's own
  semantics question (the registered-project half shares it), not a
  T-073 seam; `crates/**` is frozen for this task, so any tightening
  belongs to a store/fs-side task. The
  registered-project half asks
  `eunomia_store::projects::find_by_path_key` — DB-015 keeps the key
  unique across active AND archived rows, and an archived project is
  still a registered project. The boundary is the exact folder:
  a subfolder of a registered project, or of a pick, is NOT its
  folder (SEC-003 allowlists folders, not trees). That exact-folder
  rule binds UI-typed paths only: when `scan_start`/`scan_commit`/
  `project_relink` land, folders the app itself discovered inside an
  allowlisted root must not silently widen SEC-003 — that task adds
  them through a deliberate, recorded rule (the round-1 P3 note).
- _Two different rejections._ A path on neither half is
  `E_VALIDATION` in the settings shape (`field`/`value`/`reason`,
  value = the caller's own input echoed back — SEC-008 respected); a
  path that cannot canonicalize gets the SAME rejection, not an OS
  error: the dialog only returns existing folders and the repository
  only stores canonical paths, so such a path cannot be on either
  allowlist — "not allowlisted" is the honest answer whatever the OS
  reason. A pick whose folder vanished between the dialog answer and
  the bookkeeping is the other story: §10's `E_PROJECT_UNAVAILABLE`
  row is exactly that fact, with the picked path in `params` and the
  OS refusal in `details` — the first production consumer of
  `AppError::with_os_error` (its dead-code allow removed).
- _The dialog itself is an injected seam._ The command's whole flow
  lives in `dialog_pick_folder_with(app, pick)`; production wires
  `plugin_pick_folder` (`app.dialog().file().blocking_pick_folder()`,
  SEC-001 — the webview has no `dialog:` permission, which is what
  lets this command be the single UI road to the picker). The plugin
  call parks its thread for as long as the dialog is open, so it —
  and the fresh pick's canonicalization — run in `spawn_blocking`
  (SYS-001). The pick is canonicalized BEFORE it is remembered and
  answered, so the allowlist only holds canonical forms and the wire
  answer is the canonical path string (DB-030 — the form the future
  `project_add_manual`/`scan_start` will register); cancel answers
  `None` and grows nothing. The plugin call is the recorded
  untestable seam (the `app_open_data_folder` opener treatment); the
  wiring is pinned by the registration test. A `dialog_error_to_app_error`
  fallback (the opener mapping's shape) takes the plugin error's
  rendered text — the only reachable typed error is
  `tauri_plugin_fs::Error`, not nameable without adding a
  `tauri-plugin-fs` edge this command does not need; no new §10 code
  was introduced. The non-desktop twin of the picker answers the
  fallback (the plugin's folder picker is a desktop API; the
  `running_as_root` platform-twin pattern).
- _The check is session-first on purpose._
  `session_paths::validate_ui_path(app, path) -> Result<PathBuf,
AppError>` is the reusable boundary the upcoming path-consuming
  commands call; it returns the canonical form so a consumer never
  canonicalizes twice. The session half is consulted BEFORE the
  database half: the pick side needs no `Db`, so a picked folder
  validates in recovery mode too, where the picker itself still
  works. A path on neither half in that mode answers the recorded
  `NoDatabase` fallback (the honest state answer — every real
  consumer needs the database for its own operation and fails before
  validation would run). The command's failures log through a new
  `logging::dialog_pick_folder_failed` facade edge (§10's every-error
  rule); a successful pick logs nothing — the allowlist is
  in-memory bookkeeping, and the future `project.added` journal entry
  is the flow's real event.
- _Tests._ 9 IPC-suite tests over the mock runtime (the command
  driven through the picker seam, the check driven directly — it is
  a library function consumed by future commands, not a command):
  the card's criterion (an existing unpicked unregistered folder and
  a non-canonicalizable path both rejected with the full
  `E_VALIDATION` envelope), the session half end to end (canonical
  answer, dedupe, validation, dot-segment and real-symlink aliases
  passing, the pick's subfolder rejected — the session-side twin of
  the project boundary, pinned by the round-1 review), the empty and
  blank path rejections, the project half (active + archived passing,
  the subfolder boundary), cancel (null, nothing remembered), the
  vanish race (`E_PROJECT_UNAVAILABLE` + OS facts), the
  recovery-mode ordering, the missing-state honesty fallbacks, and
  the registration pin; the logging edge has its facade test
  (`the_dialog_failure_line_carries_the_code_and_payload`);
  2 unit tests pin the session state, 2 the command's error
  mappings. `src/bindings.ts` regenerated (`dialogPickFolder`:
  `Promise<string | null>`).
- _Known limitation, recorded against D-086._ The wire answer is the
  canonical path as a lossy `String`: a pick whose folder name is not
  valid UTF-8 (possible on Linux) reaches the UI with U+FFFD
  placeholders and can never validate on the way back — the picker
  succeeds, the folder stays unusable. D-086 left the non-UTF-8
  policy to the add/scan seam; whichever task builds that seam either
  rejects non-UTF-8 picks loudly there or widens the answer, and this
  entry is the pointer. The criterion test pins the rejection at the
  library boundary (`validate_ui_path`) because no registered command
  takes a folder path yet — the first consuming command
  (`project_add_manual`) must add the IPC-level twin (the round-1
  P3 note).

### D-102 — The version-recording service: the ALG-070 hash gate with a snapshot-blind carve-out, prune-to-limit in the caller's transaction, and the limit as a parameter (no core edge yet) — 2026-11-20

Problem: T-082 puts a version of an instruction or variant into the
`revisions` table on every real save (LIB-026). Open questions: where
the ALG-070 "same hash as the last version — no version" comparison
lives and what it compares against; whether the _op_ vocabulary
(create/update/hide/restore/delete) is hash-gated uniformly — a
variant's snapshot is name+text only (§7.3), so a soft delete or its
restore builds a byte-identical snapshot and a literal reading of the
card's "identical hash — no version" would make `op='delete'`
unreachable (instructions never delete in v1; §6.4); how the
`history.max_revisions` limit reaches the service and what "the
latest version is never deleted" (ALG-072) means at limit 0/1; and
whether the store finally takes the `eunomia-core` edge spec §5.2
plans (D-017's first-consumer rule).

Solution: `eunomia_store::library_versions`, a bookkeeping service
over the existing v1 `revisions` table (no schema change, no changes
to `library.rs`/`settings.rs`/`Cargo.toml`).

- _The gate compares the new pair with the entity's last recorded
  version, never with the library row._ ALG-070's wording ("when an
  instruction name, or a variant name or text, changes… if the hash
  equals the hash of the last version, no version is created") is a
  content-save rule; the service receives the finished T-055 pair
  (snapshot + SHA-256 as TEXT) and one typed op, and compares
  `snapshot_hash` with the last `revisions` row of the entity
  (`ORDER BY rev_no DESC LIMIT 1`). Non-snapshot facts therefore
  cannot create versions by construction: `updated_at` restamps,
  main-flag promotions, position moves and already-folded CRLF all
  hash the same and record nothing (pinned end to end through the
  real repository writers).
- _The snapshot-blind carve-out._ Variant `delete` and `restore`
  pass the gate whenever they represent a transition; everything else
  is hash-gated. §7.3 fixes the variant snapshot's fields (name,
  text) with no state column, so the two lifecycle transitions are
  structurally invisible to the hash comparison, while LIB-026's
  acceptance criterion says "each real change adds one version" and
  the History panel must show the operations (ALG-071's op list
  exists for them). A blind op is therefore suppressed only when the
  last version carries both the same `op` and the same hash: the
  repository's `delete_variant`/`restore_variant` are idempotent and
  return no "changed" signal (D-097), so a repeated command
  (double click, retry) re-records a transition that did not happen —
  and while a variant is deleted its content cannot change (the
  repository refuses to edit the dead), so an op-equal, hash-equal
  re-run is definitionally a re-run, while a genuine transition
  always differs (a second real delete follows a `restore`, a
  different op). The reasoning carries one caller invariant, stated
  in the module docs for the command tasks: every delete/restore
  transition is recorded through this seam — a state mutation that
  skips the recording would leave a stale `(op, hash)` as the last
  version and could wrongly absorb the next genuine transition.
  Instruction hide/restore need no such pairing:
  §7.3's instruction snapshot carries the state field, so a genuine
  transition always changes the hash — and the gate usefully absorbs
  idempotent re-runs on both sides (re-recording a hide or a delete
  that did not happen converges to `Unchanged`).
- _Two entry points, not one: type-level op pairing._
  `record_instruction` takes `InstructionOp` (create/update/hide/
  restore), `record_variant` takes `VariantOp` (create/update/
  delete/restore) — a hide version of a variant or a delete version
  of an instruction is a compile error, the §6.4 lifecycle table
  encoded in the API (the `NewLibraryItem` kind-carrying precedent).
  Both probe the caller's id first: unknown ids are `NotFound`, a
  heading id is `NotAnInstruction` (§6.2: headings have no
  versions), a corrupt `kind` cell surfaces as `Member` (D-039, the
  `assign` raw-probe precedent — this module's error type is its
  own, D-093 shape). `revert`/`reverted_from` are deliberately not
  expressible: they are the P1 LIB-028/029 flow.
- _ALG-071: `rev_no` = previous maximum plus one, as a scalar
  subquery inside the INSERT_ (the `create_item` shape), so numbers
  keep growing monotonically past prunes and never re-count from 1 —
  the UNIQUE(entity_type, entity_id, rev_no) constraint holds
  throughout.
- _ALG-072 prune: after the insert, in the same caller transaction,
  per entity._ `DELETE … WHERE rev_no <= MAX(rev_no) − keep` with
  `keep = max(max_revisions, 1)`: pure rev_no arithmetic over the
  UNIQUE index (no LIMIT-parameter subtleties), idempotent, touching
  only the recorded entity. The clamp makes "the latest version is
  never deleted" unconditional rather than a caller precondition —
  a zero/negative limit (a caller bug; §6.5 validates 10–1000)
  collapses the history to the latest version instead of wiping it.
  An unchanged record prunes nothing: ALG-072 ties the cleanup to an
  insertion, so a lowered setting takes effect at an entity's next
  real change (no standalone sweep exists in v1). The whole
  recording is multi-statement and atomic only inside the caller's
  transaction (D-037/D-051), pinned by a rollback test.
- _The limit is a parameter, not a settings read._ The
  `action_log::cleanup` `LogLimits` precedent: §6.5's range has one
  owner (the settings repository), the command layer reads
  `settings::get(SettingKey::HistoryMaxRevisions)` in the same
  transaction as the save and passes the value on. This keeps the
  module free of the settings boundary and makes the 0/1-limit
  invariant directly testable.
- _No `eunomia-core` edge (yet)._ D-017's rule — the edge is added
  by the first task whose store code references core types — is not
  triggered: T-055's module docs already place "the comparison
  against the last version and the no-version decision" in the
  store, the command tasks (T-084+) assemble the pair themselves
  with the core builders, and the P1 revert will need to pass a
  _historical_ snapshot through this same seam (no current row can
  supply it), so the TEXT pair is the forward-compatible boundary.
  The first store code that needs a core type adds the edge with its
  own entry.

Tests: 3 unit tests in the module (op members are exactly the §6.2
CHECK literals; error Display shapes; the prune over an already-pruned
state removing nothing, at the private seam on an in-memory migrated
database) and 19 integration tests in `tests/library_versions.rs` on
a real `open_db` + `run_migrations` database: both card acceptance
criteria verbatim (an unchanged save adds no version and consumes no
rev_no; a limit of 10 keeps exactly the ten newest, 6..=15 of 15),
every op of both entities with the correct literal and rev_no+1
(including the blind delete/restore carrying the unchanged pair),
non-snapshot changes (an updated_at restamp on a true no-op field
write, the main flag, a position move) adding nothing, the repeated
hide and the repeated blind operation converging to `Unchanged` while
a genuine delete→restore→delete still records, saving back to an
older snapshot still recording (the comparison is with the last
version only), the 0/1/negative-limit clamp keeping the latest, the
rolling window with rev_no continuing past deletions, the unchanged
no-cleanup rule, per-entity isolation, the transaction rollback
leaving neither insert nor prune, unknown ids (including each entry
point rejecting the other entity kind's id), headings, the corrupt
kind cell, and the T-055 example pairs stored byte for byte.

Round-1 review (deepseek-v4.1-flash) — REQUEST CHANGES, one P1, three
P2, three P3; all processed:

- P1-1 taken (the round's substance): the blind carve-out originally
  recorded delete/restore unconditionally, so a repeated command
  (the repository's idempotent no-ops hand the caller no "changed"
  signal) inserted a spurious version, consumed a `rev_no` and could
  prune a genuine one — a D-16 violation. Fixed by the op-plus-hash
  pairing described in the carve-out bullet above, with the
  regression pinned (`repeated_blind_operations_record_no_version`:
  delete×2 → one version; restore×2 → one version; the genuine
  second delete records).
- P2-1 taken: the "identical values" test actually changed the
  content (`"content 1"` → `"Be brief."`); it now updates with the
  row's own values and asserts the stored content survived, so the
  restamp — not an accidental content match — is what the gate
  ignores.
- P2-2 taken: "prune idempotency" was claimed but never exercised
  (the second record was hash-gated before the prune); the public
  test now claims only what it proves (no cleanup on an unchanged
  record), and the idempotent re-prune moved to the module's unit
  test at the private seam.
- P2-3 taken: no test returned to an older hash, so an
  any-version gate would have passed the suite;
  `saving_back_to_an_older_snapshot_still_records_a_version` pins
  the last-version-only comparison.
- P3-1 declined: `INSERT … RETURNING` would drop the read-back
  statement and the `expect`, but the insert-then-read shape is the
  house precedent (`create_item` et al.) and keeps the statement
  pattern uniform; not worth diverging.
- P3-2 taken: the probe contract gained cross-entity cases (a
  variant id at the instruction entry point and vice versa are
  `NotFound`); recording on a soft-deleted variant row was already
  exercised by the blind-ops test and is now stated in its comment.
- P3-3 superseded by the P1-1 fix: the blind-op re-run limitation no
  longer exists to document — the gate itself absorbs the re-runs.

Round-2 review (hy4-flash-free) — APPROVE, no P0/P1; one P2, two P3,
all processed:

- P2-1 taken: the hash half of the op-plus-hash blind gate was never
  exercised — every blind-op test reused the create's pair, so an
  op-only mutant (`last_op == op` alone) passed the whole suite. The
  repeated-blind test now ends with an op-equal, hash-different
  delete that must record (rev 5), pinning the `&&` semantics as the
  service's contract (through the repository the case cannot arise —
  content is frozen while the variant is dead — so the pin is
  defensive, matching the gate's own defensive posture).
- P3-1 taken: the blind gate's soundness rests on "every
  delete/restore transition is recorded through this seam" — now
  stated as a caller invariant in the module docs and in the
  carve-out bullet above, so the command and revert tasks inherit
  it.
- P3-2 declined: pinning `RevisionError::Sql`'s Display string — the
  `library` module's own Display test pins its typed variants and
  leaves the `Sql` wrapper unpinned (it renders rusqlite's message,
  not this crate's words); matching that precedent.

### D-101 — The fs fault-injection feature: the non-default `fault-injection` cargo feature, the self dev-dependency, and the cfg(test) seam generalized without loss — 2026-11-20

Problem: T-069 asks for spec 15.1's "failure points in `fs`" as ONE
infrastructure, drivable from integration tests and absent from the
release build. The existing seam (D-095/D-099) was `#[cfg(test)]`-
private inside `atomic.rs`: invisible to `tests/` (there the crate
is an external dependency, `cfg(test)` is false) and to the future
NFR-024 kill tests, which run the library inside a child process.
Open questions: (1) how a feature-gated seam stays out of the
release build even though cargo features are additive; (2) how
integration tests turn the feature on without a CI-fragile
command-line flag; (3) how to migrate the T-066/T-067 fixtures
without losing coverage; (4) where the NEW points live — the N-th
backup copy (T-F1's fs half), the pre-write disk-full verdict
(15.1's "out of space"), and the two kill-test pauses (T-F2 inside
a write, T-F3 between phases).

Solution: `eunomia-fs/src/fault_injection.rs`, compiled under
`#[cfg(any(test, feature = "fault-injection"))]`, holding the whole
armed-fault registry (thread-local slots + the RAII `Armed` guard,
D-095's discipline unchanged: any guard's drop disarms every
fixture on the thread — parallel tests stay isolated); atomic.rs
and backup.rs consume `pub(crate)` hooks at their production call
sites under the same cfg.

- THE RELEASE GUARANTEE IS STRUCTURAL: the feature is not in
  `default`, and NO `[dependencies]` edge in the workspace enables
  it. The only automatic enablement is the crate's own self
  dev-dependency (`[dev-dependencies] eunomia-fs = { path = ".",
features = ["fault-injection"] }`) — cargo's supported self-edge —
  which exists so a plain `cargo test` (and CI) exercises the
  points without a flag. `cargo build`/`--release` never resolve
  dev-dependencies, so every production artifact (including the
  Tauri app) compiles the crate with the feature off. The one
  unification hole cargo leaves open — a release build that also
  resolves dev-dependencies (`cargo build --release --all-targets`,
  `cargo test --release`) would link the points into release
  binaries — is closed by a `compile_error!` in the module under
  `#[cfg(not(debug_assertions))]`: the leak is unrepresentable, at
  the documented cost that release-profile TEST builds are
  impossible with the feature on (CI and the release path use none,
  verified against .github/workflows/ci.yml). Absence is verified
  with `strings`, NOT `nm` — the workspace strips release symbols
  (`[profile.release] strip = true`), so nm shows nothing in ANY
  release rlib and cannot discriminate: `cargo build --release -p
eunomia-fs` then `strings target/release/libeunomia_fs.rlib |
grep -c eunomia_fs15fault_injection` → 0, with a feature-enabled
  dev build of the same crate as the control (the scan finds the
  symbols there). Workspace test builds DO unify the feature on for
  every consumer in that build (dev-dependencies participate there)
  — by design, visible only to test binaries.
- MIGRATION WITHOUT LOSS: D-095's `faults` module moved verbatim
  (semantics, one-shot consumption, fixed-token queue, locked-
  replace plans, recording clock — renamed to a public vocabulary:
  `arm_atomic_fault`/`AtomicFault`/`AtomicStep`,
  `fix_staging_tokens`, `arm_locked_replace`, `recorded_lock_pauses`).
  All 39 atomic unit tests kept their logic, only the API names
  changed; `perform_replace` became `pub(crate)` (the simulated
  lock's eventual success is the REAL attempt), and the
  `cfg(any(windows, test))` items widened to
  `cfg(any(windows, test, feature = "fault-injection"))`. 214 crate
  unit tests green before and after.
- THE NEW POINTS: `arm_backup_copy_failure(n, io::Error)` — the
  N-th copy pass of `backup_target_file` refuses its write path
  once per copy (at the pass's start, after the destination exists,
  so the production partial-copy removal and `CopyFailed`
  classification run for real; the first attempt placed it inside
  the streaming loop, which counts chunks, not copies — caught by
  the integration test and moved); `arm_free_space_override(u64)` —
  a STATE, not an event: while armed, every successful pre-write
  probe of the armed thread reports the number, `check_space` still
  decides, and a FAILED probe passes through untouched (ERR-003's
  posture preserved and pinned) — the shape higher-layer tests need,
  since production's closure is not theirs to substitute;
  `arm_pause(PausePlan{point, skip})` — the kill-test substrate:
  `PausePoint::StagingSettled` parks between write+sync and the
  rename (T-F2's exact window; while parked, the disk holds the
  untouched original plus exactly one sweep-grammar staging file —
  what a kill there leaves), `PausePoint::WriteEntry` parks at the
  §9.5 entry (the seam between the backup phase and the write,
  T-F3's shape at file granularity). The pause is one-shot with a
  skip counter (T-F3 aims at a later boundary), signals its arrival
  through a channel (`PauseControl::wait_for_arrival` with timeout),
  parks until released, and a dropped release channel releases too
  (a panicked observer must not deadlock the operation). The
  read-only row of 15.1 stays fixture-driven at `write_access` (the
  real mark is cheaply arrangeable and the probe is the production
  check — injecting there would only test the injection); the write
  path's read-only-shaped refusals are injectable at the atomic
  steps (EROFS pinned), and `write_access.rs` itself was not touched
  (task file scope).
- TESTS: 11 integration tests in `tests/fault_injection.rs` drive
  every point from OUTSIDE the crate (the way later pipeline tasks
  and the kill tests will): N-th-copy selectivity + no partial
  debris + copies before/after land + no leak past the guard;
  injected replace refusal (original intact, staging cleaned, post-
  guard write lands); pre-write disk-full over the caller's own
  plentiful probe; failed-probe passthrough under an armed
  override; mid-write ENOSPC → DiskFull; the read-only gate by real
  mark (T-F5) and EROFS at TempCreate; locked-replace exhaustion
  (typed E_FILE_LOCKED, 3 retries, recorded 100/300/900 ms
  schedule); StagingSettled pause (disk state inside the window ==
  the kill contract, release completes the write, no cross-thread
  leak); WriteEntry pause between phases (backup complete, target
  untouched, nothing staged); skip-counter selectivity (the first
  arrival passes with its write landed — the discriminator a broken
  counter cannot fake) and one-shot liveness (a third write
  completes unparked). Cross-target clippy (`x86_64-pc-windows-
msvc`, `aarch64-apple-darwin`, lib and --all-targets, `-D
warnings`) passes: the windows arm's fault blocks and the
  cfg-gated `Locked` refusal compile on every leg that can reach
  them.

Round-1 review (deepseek-v4.1-flash) — CHANGES REQUESTED, no P0/P1,
three P2 and four P3; all fixed:

- P2-1: the release-absence proof was vacuous — the workspace strips
  release symbols (`[profile.release] strip = true`), so `nm` shows
  nothing in ANY release rlib, feature or not; the 527-vs-0 contrast
  compared an unstripped dev build against a stripped release one.
  The documented method is now `strings … | grep -c
eunomia_fs15fault_injection` (0 in the plain release rlib; the
  feature-enabled dev build is the control that the scan sees the
  symbols when they exist), restated in the module docs, the feature
  comment and above.
- P2-2: the guarantee had a hole — `cargo build --release
--all-targets` / `cargo test --release` resolve dev-dependencies,
  so the self edge's feature reached the lib unit that release
  binaries link (the reviewer reproduced both in the repo and in a
  minimal replica). Closed structurally: the module now carries
  `#[cfg(not(debug_assertions))] compile_error!` — the leak is
  unrepresentable; the cost (no release-profile test builds with the
  feature) is documented above and matches CI.
- P2-3: the StagingSettled test did not pin the "written and
  settled" half of T-F2 (a pause parked before the write would leave
  an empty staging file and pass). The test now reads the parked
  staging file and asserts the COMPLETE new content; all three pause
  tests also assert `done_rx.try_recv().is_err()` — the worker is
  parked, not merely past the signal.
- P3-1 taken: lib.rs's `[`fault_injection`]` link broke the
  feature-off rustdoc build — now plain text. P3-2 taken: the
  integration suite no longer self-disables via `#![cfg(feature)]`
  (a dropped self edge would silently skip 0 tests); it demands the
  feature with a `compile_error!` of its own instead. P3-3 taken:
  the honest check statement — the test targets always run with the
  feature (the dev edge provides it; there is no feature-off test
  run), the feature-off configuration is covered by
  `cargo build`/`clippy --lib`. P3-4 taken: `#[must_use]` on
  `Armed` ("dropping the guard disarms every fixture on this
  thread") and a second-probe assertion pinning the free-space
  override's STATE semantics.

Round-2 review (hy4-flash-free) — APPROVED, no P0/P1/P2. It
independently reproduced the strings proof (0 in the plain release
rlib, symbols present in the feature dev build), confirmed the
release+feature `compile_error!`, re-verified the migration is
lossless and all round-1 fixes present, and validated each
integration test as independently falsifiable. Its one P3 (low
confidence): the release guard keys on `debug_assertions`, a profile
flag — a release profile deliberately configured with
`debug-assertions = true` would bypass it, slightly overstating the
"unrepresentable" claim. Taken as documentation (the module docs now
state the keying and name the `strings` scan as the backstop); the
suggested CI assertion lives in `.github/workflows/ci.yml`, outside
this task's file scope — recorded here for the orchestrator.

Addendum (post-round-2): the release guard's macro invocation uses
the BRACE form (`compile_error! { … }`) in both fault_injection.rs
and its test suite. The app's SEC-008 pin
(`tracing_events_are_constructed_only_in_the_logging_module`) scans
every workspace `.rs` source RAW for the substring `error!(` — the
paren form of the compile-error macro contains that substring and
reddened the pin (the pin belongs to src-tauri, outside this task's
file scope, and its conservatism is deliberate: comments showing the
macro are meant to turn it red). The brace form is the same macro,
rustfmt-stable, and not a spelling the pin matches; the reason is
stated at both call sites. (The paren form of the panic-message
strings is unaffected — only the macro-invocation spelling matters.)

### D-103 — `project_add_manual`: the repo-root offer behind a confirm flag, typed non-add outcomes instead of error codes, the pre-generated D-29 tie-break id, and the §6.9 write warning as response data — 2026-11-20

Problem: T-074 turns §7.10's `project_add_manual` into the first
path-consuming command (PRJ-001/003/004/010/012, scenario S1). Six
points were open: (1) what PRJ-003's "the app offers the repository
root" means on the wire — a silent re-registration of a folder the
user did not pick, or a response the UI turns into a question; (2)
which §10 code a duplicate add is — or whether it is an error at all
(the card says a typed "message", not a 500-shaped refusal); (3)
D-29's tie-break hashes "the project ID", but the store's `create`
mints the row id internally and `crates/**` is frozen; (4) where
§6.9's add-time write-access warning lives, given the check runs
before any target file exists; (5) how a confirmed offer can pass
SEC-003 when the offered root is app-derived, not dialog-picked
(D-100's exact-folder rule); (6) the wire shape of a response with
four meanings.

Solution: `project_add_manual(path, accept_repo_root: Option<bool>)
-> ProjectAddResult` in `commands.rs`, registered in
`collect_commands!`, plus a `logging::project_add_failed` facade edge
(the follow-up D-096 recorded for the Projects module's write
commands).

- _The offer is a response, confirmed by re-sending the pick._ A
  subfolder pick answers the `"repo_root_offered"` arm with the
  canonical root `find_git_root` resolved (nearest root wins, D-06)
  and writes nothing; the UI's confirm re-sends the SAME picked path
  with `accept_repo_root: true`, and the command re-derives the root
  before adding. That keeps SEC-003's letter intact — the allowlist
  still holds exactly what the dialog chose, no app-derived path is
  remembered (D-100's rule that discovered folders must not silently
  widen the allowlist), and the confirm is idempotent (a replay hits
  the duplicate path). A silent auto-add was rejected: registering a
  different folder than the one the user picked is exactly the
  surprise G5-style consent exists for.
- _Non-add outcomes are typed answers, not error codes._
  `duplicate_active` (the "message is shown" case) and
  `archived_duplicate` (the restore offer, carrying the id T-109's
  `project_restore` will take) are success-shaped results: §10 has no
  duplicate row, the DB-015 conflict is a fact about the user's
  folder, and an error envelope would invite a "fix your input"
  message for a correct input (the D-096 refusal-not-500 shape). The
  duplicate check runs inside the operation's own transaction
  (`find_by_path_key`, either state), so the create cannot race past
  it (one connection, D-037); the refusal writes nothing — no row, no
  journal entry, the card's "the DB did not change" pinned by test.
- _The root lookup keeps D-090's split._ `NoGitEntry` is `E_NOT_A_REPO`
  (§10's row); a refused probe or canonicalization is
  `E_PROJECT_UNAVAILABLE` with the walked path in `params` and the OS
  refusal in `details` — "could not check" is not "checked and
  absent", and the user actions differ.
- _The name/label derivation and its recorded edge._ The name defaults
  to the root's folder name through `validate_name` (§6.9: trim, 1–100
  code points), the label to `default_label` + `validate_label`
  (DB-042). The `project_validators` counterexample — a control-
  character prefix deriving an unstorable label — surfaces as an
  `E_VALIDATION` field envelope, and the filesystem root itself (no
  file name) answers the same way: the app cannot register a nameless
  row the schema CHECKs would reject.
- _One transaction: duplicate check, defaults copy, color, create,
  journal (D-051)._ Everything the operation writes happens in one
  `Db::run`: the DB-015 check, the four `default.*` reads (PRJ-012:
  the copy is atomic with the create — a concurrent `settings_set`
  cannot split them), the D-29 color over the ACTIVE projects'
  colors, `create` (the repository appends `sort_order`), and the
  `project.added` entry — info/user/ok, entity project, no details
  (paths ride `Details::File` for file writes only). The
  `E_SYNC_RUNNING` guard of SYS-005 does not apply: that rule lists
  the project-_editing_ commands and no sync writer exists yet.
- _The D-29 tie-break id is a pre-generated sibling._ `create` mints
  its own row id and the store is frozen, so the tie-break hashes a
  fresh UUIDv7 generated in the transaction. Both ids are fresh at
  decision time — a re-add gets a new row id either way, which is the
  determinism D-29's parenthetical asks of a decision point — and the
  divergence is confined to the full-palette tie (≥12 active
  projects). If the store ever grows an id-taking create, wiring the
  row's own id in is a one-line change the color tests would not
  notice (they pin the count-0 rule, not tie values).
- _The write-access warning is response data, post-commit._ §6.9:
  "Write access is checked when adding (warning)" — the probe
  (`check_write_access` over the root's future target file) runs
  AFTER the transaction on the blocking pool (a dead drive delays
  only this response, never the one database lock — the D-096 probe
  order), and the D-094 verdicts travel as
  `write_access_warning: "folder_not_writable" | "file_read_only"`
  on the added answer; the project stays registered. A refused probe
  answers `None` — "could not check" is not "no rights" (D-090), and
  the sync preflight re-checks where a refusal is an
  `E_NO_WRITE_ACCESS` skip.
- _The response is a discriminant struct, not a tagged enum._
  `ProjectAddResult { outcome, project?, offered_root?, existing? }`
  in the `ActivityEntry` house shape: closed vocabularies as strings,
  wire primitives only, the arms as `Option` fields (no DB type
  crosses the boundary). `sort_order` is a saturating `u32` (the
  D-049 specta rule). `eunomia-core` joined `src-tauri/Cargo.toml`
  for `pick_color` and the validators — an edge, not a version (the
  D-096 `eunomia-fs` precedent), flagged to the orchestrator because
  the manifest sits at the task's scope edge.
- _Tests._ 10 IPC-suite tests over the mock runtime (picks through
  the real T-073 dialog seam, real `.git` fixtures, seeding through
  the repositories): the card's four AC cases by name — repository
  root (every field: canonical path, name, DB-042 label, palette
  color, defaults, sort_order 1→2, a different color on the second
  add by D-29's count-0 rule, the exact one-entry journal),
  subfolder (the offer writes nothing; the confirm registers the
  root; the nearest-root rule over nested repositories), adding
  again (the duplicate message and the archive offer with the
  database proven unchanged), non-Git folder (the full `E_NOT_A_REPO`
  envelope) — plus the SEC-003 gate ordering (an unpicked REAL
  repository answers the gate's `E_VALIDATION`, never `E_NOT_A_REPO`
  or an add; the lossy U+FFFD input is D-100's IPC-level twin), the
  PRJ-012 copy from changed `default.*`, both §6.9 warning classes
  over read-only fixtures (root-gated on the folder arm, the D-088
  posture), the honest no-database answer, and the registration pin;
  4 unit tests pin the pure seams (the two lookup mappings, the
  derived-name edges, the warning classes). `src/bindings.ts`
  regenerated: `projectAddManual`,
  `ProjectAddResult`/`AddedProject`/`ExistingProject`.
- _Round-1 review fixes (deepseek-v4.1-flash; 3×P2, 6×P3, all
  applied)._ (1) The write-access probe now targets the file the
  project's scheme actually writes — `target_file_name(scheme,
base_name)`, so scheme C probes `<STEM>.project.md` and stays silent
  about a main file it never touches (was: `base_name`, mis-targeting
  every scheme-C project both ways); pinned by two scheme-C fixtures
  (read-only target warns, read-only main file does not). (2) The
  D-100 non-UTF-8 seam resolved at this command: `validate_ui_path`'s
  canonicalize-failure rejection names the encoding ("the folder name
  is not valid UTF-8 … rename the folder") when the sent text carries
  U+FFFD, instead of the false "not picked" sentence for a folder that
  WAS picked — and the earlier synthetic U+FFFD test arm (a path that
  never existed, vacuously passing) is replaced by a real Linux
  non-UTF-8 fixture driven through the dialog seam. (3) The stale
  `#[allow(dead_code)]` and "no production caller" note dropped from
  `validate_ui_path` — this command is that caller. Plus the P3s: a
  `DuplicatePathKey` from `create` (the unreachable race) now
  converges to the typed duplicate answer instead of the
  read-command fallback under a retry-inviting code; PRJ-012's DEF-005
  half is pinned (re-seeded defaults leave the stored row unchanged);
  the confirm replay and the inner-root-picked-directly branches are
  pinned; `logging::project_add_failed` has its facade test;
  `managed_db` resolves before the offer branch so recovery mode
  answers `NoDatabase` consistently for a subfolder pick too; the
  canonicalization comment no longer overstates the gate's guarantee.
- _Round-2 review (hy4-flash-free: 0×P0/P1/P2, 1×P3, applied)._ The
  command's `E_VALIDATION` arm for D-079's counterexample — a legal
  repository name whose DB-042 derivation is an unstorable label —
  was implemented but unpinned; a Unix-gated IPC test now drives a
  control-named `.git` folder through the dialog seam and asserts the
  envelope names the `label` field with the derived spelling, leaving
  the database untouched.

### D-104 — The marker clock in core: jiff's tz features turn on, the UTC-name ruling, and a pure instant+zone seam — 2026-11-20

Problem: T-111 gives the ALG-032 marker its "now": local time and
zone, taken "once per run, at the start of the write phase, the same
for all projects", with D-23's `UTC±hh:mm` form when no IANA name is
available. Four decisions the card leaves implicit: (1) the card's
chrono + iana-time-zone wording is pre-migration text — D-035 replaced
chrono with jiff 1.x and predicted "jiff's tz features stay off until
the creation-marker task needs them (that task turns on what it
uses)"; (2) which jiff features that is, given core's D-017
allow-list posture (a manifest change needs a record); (3) what
"without an IANA name" means exactly — jiff reports `UTC` itself as a
name, and system-detection failure degrades to a nameless
`Etc/Unknown` that behaves like UTC; (4) how to test a clock at all —
the system time and zone are not test inputs.

Solution: `marker_clock` in eunomia-core. `MarkerClock` holds the
three things the marker prints — `MarkerTime` (D-061's decomposed
civil fields), `Option<String>` IANA name, `i32` offset minutes —
with private fields: they are one observation, and public fields
would let a caller stamp one zone's wall clock with another zone's
offset. Two constructors: `from_instant(Timestamp, TimeZone)` is the
pure seam every test drives (fixed RFC 3339 instants, explicitly
built zones — named via `TimeZone::get`, fixed via
`Offset::to_time_zone`, POSIX via `TimeZone::posix`, jiff's
`Etc/Unknown` via `TimeZone::unknown()`); `now_marker_clock()` is
that seam at `Timestamp::now()` + `TimeZone::system()`, the one
clock read of the run. `marker_line()` delegates to
`creation_marker_line`, so the template keeps its single owner
(D-061) and the "same marker for every file" property is type-level:
one immutable capture, identical lines on every ask (pinned by
tests, including through `clone`).

Dependency shape (the D-035 turn-on, in core this time):
`jiff = { version = "1", default-features = false, features =
["std", "tz-system", "tzdb-zoneinfo", "tzdb-bundle-platform"] }` —
`tz-system` for system-zone detection (TZ env var, /etc/localtime,
Windows registry), `tzdb-zoneinfo` for the Unix tzdb
(/usr/share/zoneinfo), `tzdb-bundle-platform` because Windows has no
system tzdb (jiff target-gates the bundle to Windows, so Linux/macOS
compile none of it). `tz-fat`/`tzdb-concatenated`/`logging`/`perf-inline`
stay off — outside this need. iana-time-zone never enters: jiff's
native IANA support is exactly why spec 5.3 dropped it (already
reflected in the tech table). Lockfile: two new rows,
jiff-tzdb 1.0.0 and jiff-tzdb-platform 1.0.0 (Unlicense OR MIT),
Windows-only edges that stay unused rows on Unix — D-035's
chrono-under-serde_with posture; `windows-link` was already in the
graph. `cargo deny check` green (advisories/bans/licenses/sources).

Spec-reading rulings, pinned by tests: (1) a machine actually set to
UTC stamps the name-and-offset form `UTC +00:00` — "UTC" is an IANA
identifier and D-23's compact form is for a zone whose name is
_unknown_; (2) jiff's `Etc/Unknown` fallback (detection failed)
reports no name and offsets zero, so the marker degrades honestly to
`UTC+00:00` instead of failing the run; (3) the offset is the one in
force at the captured instant (DST honored — the fall-back pair
2026-10-25 02:30+02:00/02:30+01:00 Europe/Berlin is an oracle), and
sub-second instants truncate (MarkerTime is second-granular); (4) a
sub-minute zone offset (only a POSIX rule or a synthetic fixed zone
can produce one today) truncates toward zero, never rounds; (5) a
zone beyond the marker's own ±24:00 domain (jiff reaches ±25:59)
panics at `marker_line()` — marker.rs's broken-invariant posture,
unreachable from real IANA zones (±14:00 max).

Round-1 review (deepseek-v4.1-flash) — APPROVED with 5×P2/4×P3, all
taken. What they changed:

- **The §5.2 exemption, said out loud (P2-2).** `marker_clock` is the
  first core module whose production path reads the environment (the
  process clock via `jiff`, the system tzdb via `TimeZone::system`) —
  a reversal of D-061's "the clock stays out of core / the sync
  pipeline captures it via jiff above core", which D-104's first
  draft did not own. Kept in core (the task card assigns the capture
  here and scoped the jiff edge to this crate's manifest; moving
  `now_marker_clock` to `src-tauri` would put the marker's clock read
  in the shell layer against D-010's thin-shell rule) and recorded as
  an exemption: clock and system tzdb are environment inputs, not
  application data; the read lives in one function
  (`now_marker_clock`); every test drives the pure seam instead.
  D-061's consumer note is amended by this entry: the capture is core
  code (`marker_clock`), not "above core".
- **The environment can name a zone beyond the marker's own domain
  (P2-5).** `TZ=AAA-24:59:59BBB,M3.2.0,M11.1.0` is a legal POSIX rule
  (verified: jiff parses it, offset 1499 min) that made
  `marker_line()` panic via `now_marker_clock()` — a run aborted over
  a stamp. `now_marker_clock` now guards: an out-of-domain capture
  re-resolves the _same_ instant (one clock read, never two) against
  jiff's nameless `Etc/Unknown`, degrading the marker to the honest
  `UTC+00:00` form. The panic posture stays for the pure seam
  (`from_instant` with a synthetic ≥24h zone), per marker.rs.
- **Criterion test made falsifiable (P2-3).** "One capture → every
  ask identical" over immutable fields is a tautology; the test now
  pins that `marker_line()` is exactly `creation_marker_line` over the
  three published accessors across every zone shape (named, UTC,
  fixed, POSIX, unknown) — the property that makes the identity mean
  anything. The many-files-one-capture oracle stays with the
  write-phase consumer.
- **Day-rollover fixtures (P2-4).** Tokyo (`2026-11-17T23:30Z` →
  `2026-11-18 08:30 +09:00`) and Honolulu (`T05:00Z` → `2026-11-16
19:00 -10:00`) pin that the _date_ comes from the zone's calendar;
  every earlier fixture shared UTC's date, so a UTC-date
  implementation was green.
- **Stale docs (P2-1).** marker.rs's "this crate has no time library"
  and lib.rs's dependency enumeration were false as of the edge; both
  reworded (D-017's list is extended by D-104). The spec's §5.2
  module table still reads "only serde, sha2, thiserror" for
  eunomia-core — that row is spec text outside this task's file scope
  (docs/spec.md is not in the T-111 change scope; owner ruling
  precedents c22cc46/f3db54f apply); left for the orchestrator.
- **Naming honesty (P3-1/3-2/3-3/3-4).** "Canonicalized" overstated —
  jiff reports the loaded identifier (case-normalized, aliases kept:
  `Etc/UTC` stamps `Etc/UTC +00:00`, now pinned by a test); the
  case-insensitive `TimeZone::get` trick left the tests (not a
  documented jiff guarantee); the sub-minute truncation test asserts
  full lines (wall clock and offset pinned together — the seconds the
  offset drops do survive in the wall clock, so "never overstates"
  wording is gone); and the capture moment is documented as §9.4's
  Phase 0 (content build), with ALG-032's "start of the write phase"
  quoted as the looser wording of the same moment.

Round-2 review (hy4-flash-free) — APPROVED, no P0/P1/P2. It verified
all nine round-1 fixes against the diff (no regressions), audited all
tests for falsifiability ("no oracle that would pass under a
UTC-date, real-clock, wrong-offset, second-clock-read or
wrong-formatter implementation"), checked jiff 1.0.1's API against
the vendored sources, and ran the crate suite, clippy and
`cargo deny check` itself — all green. Its one P3 (low confidence,
taken): the guard's exact ±24:00 pass-through boundary was not pinned
— a `<=`→`<` regression would silently degrade a legal ±24:00 zone.
Now pinned by
`the_run_guard_passes_exactly_twenty_four_hours_through` (both signs,
plus the full `2026-11-18 13:03:22 UTC+24:00` line a whole day over).

### D-105 — The build snapshot read: a caller-joined read transaction, the first store→core edge wired, and the SYS-005 isolation criterion pinned deterministically through WAL — 2026-11-20

Problem: T-100 turns ALG-001/SYS-005 into store code: one read
transaction that hands `eunomia-core`'s
`BuildSnapshot` the active projects in `sort_order` order, all
library items in `position` order, their active variants, and the
assignments. Open questions: who owns the transaction boundary; whether
the store→core edge D-017 reserved ("added by the first task whose
store code actually references core types") is finally wired, and
what it costs; how the acceptance criterion — a library edit
committed "during the read" stays out of the snapshot — is pinned
honestly rather than by racing a fast reader against a slow writer;
and which repository reads are reused versus added.

Solution: `eunomia_store::build_snapshot`, one function
`read(&Connection) -> Result<BuildSnapshot, BuildSnapshotError>`.

- _The transaction is joined, not owned_ (the D-037/D-051 rule, kept
  even though this is a read): `read` takes `&Connection` and runs
  its four SELECTs inside whatever transaction the caller holds; the
  production shape is `Db::run(|tx| build_snapshot::read(tx))`. WAL
  (DB-001, applied by `open_db`) is what makes that the SYS-005
  guarantee: a read transaction's view of the database is pinned at
  its **first** read statement, so everything committed afterwards —
  through any connection — is invisible until the transaction ends.
  The module docs record the calling contract a type cannot carry:
  called bare, each SELECT gets its own implicit read transaction and
  the four reads could tear; the integration suite calls it bare only
  where no write can interleave.
- _The core edge, wired by its first consumer._ D-017's rule executed
  to the letter: `Cargo.toml` gains the path-only
  `eunomia-core` edge with a comment naming this entry, the
  repositories stay TEXT in/out with store-local vocabularies
  (D-093/D-097), and the row→domain mapping lives in `build_snapshot`
  alone — `SnapshotProject` (id, name, the canonical root verbatim,
  write settings), `LibraryItem` (heading level via
  `HeadingLevel::try_from`, instruction state, variants),
  `Assignment`. The vocabulary translations are total
  member-by-member matches; a unit test pins them by schema literal
  in both directions (the literals are the oracle, not the variant
  names), so a swapped mapping fails the suite even though both sides
  compile. No `thiserror` rides the edge into the store:
  `BuildSnapshotError` keeps the house hand-rolled impls, returned
  directly (the D-093 boundary — not nested in `StorageError`).
- _Reads reused and reads added._ `projects::list(Active)` is reused
  unchanged — D-093 already calls it "ALG-001's subset read". The
  library repository gained two pinpoint reads because the existing
  ones are per-entity: `list_active_variants` (every active variant of
  every instruction, `ORDER BY instruction_id, created_at, id` — one
  statement instead of a `list_variants` call per instruction; the
  snapshot is hot: every sync and every DB-022 hash recompute reads
  it) and `list_assignments_for_active_projects` (a JOIN over
  `projects WHERE state = 'active'`, `ORDER BY p.sort_order,
i.position` — the DB-014 scoping lives in SQL, not in caller
  memory, because core's snapshot validation rejects an assignment of
  a project the snapshot does not hold). Hidden instructions' active
  variants are included: ALG-001 reads "all library items … active
  variants" with no visibility filter, and DB-013 assignments may
  still point at them.
- _The isolation test is deterministic, not a race._ WAL pins a
  transaction's view at its first read — so the test pins exactly
  that, through the public API only: `unchecked_transaction`, one
  full `read` (the snapshot is now pinned), a **second** `open_db`
  connection commits a variant-content edit in autocommit, a second
  full `read` inside the still-open transaction must equal the first
  (whole-value equality plus the named content), and only after
  `rollback` does a fresh read see the edit — the non-vacuous half
  proving the commit was real. The scenario's precondition
  (`journal_mode = wal`) is asserted up front: in rollback-journal
  mode the writer would block on the reader's lock instead of
  committing, and the scenario this task must prove would not exist.
- _Validation and corrupt cells._ `BuildSnapshot::new` runs at the
  boundary, so a committed bare instruction (DB-010's transient
  window escaping its producer transaction) surfaces as
  `Invalid(NoActiveVariants)` at sync start, and a corrupt vocabulary
  cell arrives wrapped as `Projects(Member)`/`Library(Member)` with
  column and value intact (D-039). One nuance recorded honestly: a
  corrupt `projects.state` cell **cannot** surface through this read —
  the Active scope's `WHERE state = 'active'` drops the row before
  any decode, so the project silently leaves the snapshot (and its
  assignments with it). That is D-093's scope semantics, not a new
  policy; the corrupt-cell propagation test therefore corrupts
  `scheme`, a cell the active-scoped read still selects and decodes.
  `list_active_variants`' `WHERE state = 'active'` has the same shape
  (round-1 review): a corrupt variant `state` cell drops the row
  silently — for an active instruction left with no returned variants
  the read then fails as `Invalid(NoActiveVariants)`, a misleading
  cause — and its `# Errors` docs say so instead of promising an
  unreachable `Member`. The crate-level "no core edge" claims that
  D-017's reserved edge made stale (`library.rs`, `library_versions.rs`
  module docs) were reworded to point at this module as the crate's
  one core consumer.
- _Tests._ 2 unit tests in the module (vocabulary literals; error
  Display shapes) and 10 integration tests in
  `tests/build_snapshot.rs` on a real `open_db` + `run_migrations`
  database: the empty database → empty snapshot; active projects in
  `sort_order` with archived excluded and every write-settings
  member mapped (scheme B, `CLAUDE.md`, both extra files on, one per
  position); all items in the **current** position order after a
  heading move, hidden instructions included and marked; only active
  variants in creation order, the soft-deleted twin gone; variant
  text exactly as stored (the save-time CRLF fold already applied,
  edge whitespace untouched — ALG-002 is downstream); assignments
  scoped to active projects in the snapshot's reading order; the
  isolation criterion; the bare-instruction and corrupt-cell error
  paths; and a cross-check that the snapshot's sets equal the
  repositories' own active/all reads (a filter drift in either
  direction fails).

### D-107 — The T-070 platform matrix suites: cfg-grouped integration tests per OS, shell-arranged Windows fixtures, and the cmd quoting trap — 2026-11-20

Problem: T-070 turns spec 15.3's platform matrix ("case-insensitive
NTFS, long paths, junctions, locked files, CRLF in repositories, write
rights in protected folders / case-insensitive APFS, NFD names … /
rights and owners, symlinks"; NFR-001..003, 12.1) into tests for
`eunomia-fs`, while CI develops on a Linux box and the inline suites
already pin the pure halves (the DB-030 key rules with literals, the
Unix symlink shapes, T-F8's locked-file criterion per D-099's ladder).
Three questions: (1) how to organize tests so each OS leg runs its own
real-filesystem facts, the Linux leg runs a real subset locally, and
the other groups still cannot rot; (2) how to build the two Windows
fixtures std cannot build (junctions, write-denied folders) without
leaving the crate's dependency list (frozen by this task's file
scope); (3) what the "protected folders" row means when the CI runner
is ephemeral and elevated.

Solution: three integration suites in `crates/eunomia-fs/tests/` —
`paths_platform.rs`, `scan_platform.rs`, `atomic_platform.rs` — each a
common group plus `#[cfg(windows)]` / `#[cfg(target_os = "macos")]` /
`#[cfg(unix)]` / `#[cfg(target_os = "linux")]` groups:

- THE LADDER EXTENDED TO INTEGRATION TESTS: every group drives the
  public API against the real filesystem (case variants resolve to the
  stored spelling and one key on NTFS/APFS; NFD/NFC spellings are one
  folder on APFS and TWO projects on Linux — the counter-truth that
  makes the folding rule meaningful; a junction aliases its target to
  one DB-031 identity and is not followed by the walk; verbatim
  `\\?\` paths past MAX_PATH canonicalize, key `//?/…`, scan and write;
  EACCES 13 / ERROR_ACCESS_DENIED 5 ride the typed skips and the
  WriteFailed step; the setgid group rides onto written files). The
  Windows/macOS groups are compile-checked by the cross-target runs
  (`cargo check -p eunomia-fs --all-targets --target
x86_64-pc-windows-msvc` / `aarch64-apple-darwin`, D-099's precedent)
  and executed by the ci.yml OS matrix (`cargo test --workspace` on
  ubuntu/windows/macos legs; clippy runs on ubuntu only, so per-target
  unused-import hygiene in cfg-gated modules matters).
- SHELL-ARRANGED WINDOWS FIXTURES, ZERO NEW DEPS: junctions via
  `cmd /C mklink /J` (no elevation; windows-sys has no enabled
  reparse-point feature and Cargo.toml is frozen), the write-denied
  folder via `icacls /deny <whoami>:(WD,AD)` — deny ACEs bind even
  elevated runners, deletion still flows from the parent, and
  `/remove:d` restores before the assertions (the house
  restore-before-assert pattern); long paths built as verbatim
  `\\?\` spellings, which no manifest dependency needs.
- THE CMD QUOTING TRAP (round-1 review P1, the entry's recess lesson):
  `Command`'s Windows argument escaping backslash-escapes embedded
  quotes MSVC-style, cmd does not honor backslash escapes — manually
  wrapping mklink's path arguments in quotes mangles them and breaks
  both junction tests. Plain `.arg(path)` values are correct: std
  quotes a token iff it needs it. The duplicated helpers that carried
  the bug were hoisted into `tests/common/mod.rs`
  (`#![allow(dead_code)]`: each test crate links its own copy and uses
  a different per-target subset) so the drift cannot recur.
- HONEST SCOPING: "protected folders" is arranged as an ACL denial on
  an OWNED temp folder (an arrangeable fact, not a guess about system
  folders); T-F8 and the read-only-attribute gate stay with their
  inline criterion tests (documented in the module headers); the macOS
  "permission prompts" (TCC) and Linux XDG/Wayland/AppImage rows are
  not filesystem-layer behaviors and are out of this crate's scope.

Tests: 18 tests — 8 run on the Linux leg (1 common paths + 2 linux
scan + 5 unix atomic), 9 on Windows, 9 on macOS (the shared common and
case-insensitive groups run on every leg). Mutation-checked: making
the Linux path_key rule NFC-normalize kills
`nfc_and_nfd_named_repositories_are_two_projects`. Round-1 review
(deepseek-v4.1-flash) — CHANGES REQUESTED (P1 quoting bug above, P2
duplication/scoping, P3 tautologies): all fixed and amended. Round-2
review (hy4-flash-free) — APPROVED, no findings; independently
confirmed the inline-duplication boundaries and the CI platform
assumptions.

### D-106 — `exclusions_list` and `exclusion_remove`: the D-10 screen's backend over the frozen journal vocabulary, an id-addressed remove, and the acceptance criterion composed with the T-075 classifier — 2026-11-20

Problem: T-078 turns §7.10's last two Projects-module names into the
D-10 "Excluded paths" screen's backend: `exclusions_list` (the
screen's rows) and `exclusion_remove` (its "Remove" button, §6.4's
one physical-delete entity — "the path is found by a scan again").
Five points were open: (1) what the list carries, given the exclusion
row's identity is a `path_key` but the screen shows a path;
(2) which §10 code answers an id that matches no row — the card says
a typed NotFound refusal, the registry has no not-found code;
(3) how LOG-001…004 journals a removal when the D-051 `Action`
vocabulary (frozen for this task) has no exclusion member and no
`EntityType` names an exclusion; (4) how the by-id remove reads its
row when the store's public face (D-093) has a by-key find but no
by-id read; (5) how the card's acceptance criterion — "after the
removal, a scan classifies the path as `new`" — is proven when the
scan command layer (T-076) does not exist yet.

Solution: two commands in `commands.rs` over the T-071 repository
(read-only) and the `Db::run` executor (D-037), registered in
`collect_commands!`, plus two `logging.rs` facade edges
(`exclusions_list_failed`, `exclusion_remove_failed` — the D-096
follow-up note for the Projects module's edges, adopted by the task
that owns them; a successful remove is the journal entry itself, the
D-051 unmirrored info+user+ok shape, and reads journal nothing).

- _The list mirrors the §6.2 row: id, path, path_key, reason,
  created_at._ The `ExclusionEntry` DTO carries every column as a
  wire primitive (the `ActivityEntry` rule), in the repository's own
  stable order — `created_at`, the UUIDv7 id second (oldest first,
  insertion order within a millisecond). `path_key` rides the wire
  deliberately: it is the string the scan registry's excluded set
  holds (ALG-053/D-098), so the scan UI can correlate a skipped
  repository with its exclusion row without re-deriving the key, and
  it is a pure text transform of the `path` that already travels
  (D-086) — no new fact leaves the database. A fresh database answers
  an empty list; recovery mode answers the honest `NoDatabase`
  fallback (the `projects_list` rule, never a pretend-empty list).
- _An unknown id is the `E_VALIDATION` refusal naming the field._ The
  frozen §10 registry has no generic not-found code; of the
  candidates, `E_VALIDATION` (params field/value/reason, the
  `rejected_field` shape) is the honest fit — the caller's input
  names no row, the whole operation is rejected, nothing is written —
  while the D-037 fallback would misname a stale screen as a database
  problem and invite a meaningless retry. Idempotency holds at the
  state level (the make-operations-idempotent rule): a refused or
  replayed remove leaves no partial state — the path is not excluded
  either way — and the store's own `delete_exclusion` carries the
  same convergence one layer down.
- _The journal entry is `project.changed` with the removed path as
  the `Details::File` payload._ LOG-001…004 does not name exclusion
  operations, but the card requires the entry, and the frozen D-051
  vocabularies leave exactly one defensible shape: the projects
  domain's generic change verb at info/user/ok, no entity (the
  exclusion id must not ride a `project` entity column — the Activity
  window's project filter would mis-correlate it), no `project_id`
  (no project row is involved; `manual` exclusions have none), and
  the path — SEC-008 permits paths, and it is exactly the fact the
  screen shows — as the one identifying fact. **Tracked follow-up:**
  a store-side task should grow an `exclusion.removed` `Action`
  member (with its `EntityType`) and take the entry over BEFORE the
  Activity UI ships — until then the Activity window renders these
  rows as a project change attributable to no project, which is the
  known cost of the frozen vocabulary; the rows are replaceable (the
  member's landing task may drop or migrate them, they carry no
  facts the exclusions table does not).
- _The by-id read composes the repository's ordered list._ The
  remove's transaction is: `list_exclusions` (the screen's own data,
  screen-sized) → find the id → the typed refusal when absent →
  `delete_exclusion` → the journal `insert` — all inside the one
  `Db::run` transaction (D-051's producer rule, the T-074 shape), so
  a failure after the DELETE rolls both back (pinned by the settings
  rollback test's trigger shape: the row survives, the journal claims
  nothing).
- _SEC-003: id-based, no new path vector._ `exclusions_list` takes no
  input at all; `exclusion_remove` takes an id that must come from
  the list. No UI-typed path can reach the filesystem or the store
  through either command — the screen's paths flow out of the
  database, never in. The wire DTO's `path_key` disclosure adds no
  capability (see above).
- _The acceptance criterion is proven by direct composition._ The
  T-078 IPC test builds the ALG-053 registry exactly the way T-076
  will (the three membership sets read through the repositories,
  D-093/D-098; §6.5's default depth and D-05's default exclusion
  names), canonicalizes the scan root (the caller obligation D-098
  records), runs the real walker + classifier over a real `.git`
  fixture: `excluded` before the command, `new` after it — with a
  never-excluded sibling pinned `new` throughout so the verdict flip
  is attributable to the removal alone. The flip is the
  no-project-row shape (a `manual` exclusion, or a `removed_scanned`
  leftover whose project row is gone); the PRJ-017 coexistence shape
  — an archived row plus its `removed_scanned` exclusion on one key —
  classifies `archived` before AND after the removal (D-098's
  priority; only `project_restore`, T-109, clears the last trace),
  pinned by its own test so the command never promises more than
  §6.4 does.

Tests: 10 IPC tests in `ipc_tests.rs` (the package total stays all
green): the acceptance pair (`removing_an_exclusion_makes_the_next_
scan_classify_the_path_new`), the PRJ-017 coexistence pin
(`removing_an_exclusion_of_an_archived_project_keeps_the_scan_verdict_
archived`), every field + stable order with a forced same-millisecond
tiebreak pin (both reason members, the DB-002 created_at shape), the
empty-database list, the honest no-database answers for both
commands, the corrupt `reason` cell's fallback envelope
(`ProjectsCorrupt`, class-only diagnostic, no raw cell on any SEC-008
surface), the removal's one-transaction delete + journal
(action/level/visibility/result, the `File` path detail, no entity),
the typed unknown-id refusal with rows and journal proven unchanged,
the rollback proof (a trigger rejecting the journal insert rolls the
DELETE back), and the registration pins for both §7.10 names.

- _Round-1 review (deepseek-v4.1-flash: 0×P0/P1, 2×P2, 5×P3, all
  applied)._ The T-074 journal comment's "paths ride `Details::File`
  for file writes only" contradicted the new non-file-write `File`
  detail — scoped to the add flow with a D-106 pointer; the
  `exclusion.removed` follow-up moved from prose to the tracked form
  above; the shared mapper's three write-outcome diagnostics
  reworded command-neutral (exclusion_remove is a write command, the
  old texts said "reached a read command"); the
  `exclusions_list_failed` doc now names the base classifier's SQL
  classes; the order test's id-tiebreak claim became a real pin (one
  forced shared `created_at` through the executor, re-read ordered by
  the id); the scan helper now passes D-05's seven-name §6.5 default
  exclusion list (T-076 fidelity); the PRJ-017 coexistence shape
  gained its own test (below) and the acceptance bullet's
  generalization was qualified to the no-project-row shape.
- _Round-2 review (hy4-flash-free: 0×P0/P1/P2, 2×P3, both
  applied)._ The `exclusion_remove_failed` doc gained the same SQL
  class enumeration its sibling received in round 1 (the remove path
  has three mapper seams: the list read, the delete, the journal
  insert); the two new facade edges gained the real-subscriber test
  the peer edges have (`the_exclusion_failure_lines_carry_the_code_
and_payload` in `logging.rs` — a wrong target or dropped field on
  either line now fails a test).

### D-108 — Frontend mock IPC layer behind one facade and the `VITE_IPC_MOCK` flag — 2026-11-20

Problem: spec §15.1 requires UI component tests and Playwright e2e to run
"against the Vite dev server with a mocked IPC backend", and UI development
in a plain browser has no Rust to talk to — but the frontend's only
sanctioned IPC path (D-003) is the generated `commands` in `bindings.ts`,
which need the Tauri webview. There was no mock layer, no fixture generator,
and no single point a future "no direct invoke" lint pin could guard.

Solution (T-141): `src/ipc/` is the app's single IPC access point.

- **Facade.** `src/ipc/index.ts` exports `ipc` (`backend` + `commands`);
  the ONLY module importing `src/bindings.ts` is `src/ipc/contract.ts`.
  Components/stores import `src/ipc`, never bindings — D-003's no-raw-
  `invoke()` line is now one lint rule away (recommended pin:
  `no-restricted-imports` on `../bindings`/`../../bindings` outside
  `src/ipc/**`; not added here — `eslint.config.js` was outside the task's
  file scope).
- **One flag.** `VITE_IPC_MOCK` (`1`/`true`) selects the mock backend at
  module load: `pnpm dev` in a browser, `pnpm e2e` (playwright.config.ts
  passes the flag to its webServer, so CI's plain `pnpm run e2e` is
  mock-backed), and Vitest (always on via `test.env` in `vite.config.ts` —
  the same mocks everywhere, per §15.1). Unset (production build,
  `tauri dev`) the real commands run. The selection in `src/ipc/index.ts`
  is a hand-inlined comparison, NOT a call into `mockEnabled()`: Vite
  replaces `import.meta.env.VITE_IPC_MOCK` with a literal at build time,
  the comparison folds, and the bundler drops the whole mock layer — a
  function call would keep the branch opaque and ship ~5 kB gzip of dead
  mock code (round-1 review P1-1 caught exactly that in the first cut;
  verified after the fix: no mock strings in `dist/`). `VITE_IPC_FIXTURES`
  (`s`/`m`/`l`) picks the fixture profile, default `s`; tests pin `s`
  explicitly so a developer's env cannot leak set L into the suite.
- **Mocks.** `src/ipc/mock/commands.ts` implements every generated command,
  annotated `IpcCommands` (= `typeof commands`): a regenerated `bindings.ts`
  with a new command FAILS `pnpm run typecheck` until a mock is written —
  loud drift, same principle as the Rust-side bindings pins. First
  exercise: T-078's `exclusions_list`/`exclusion_remove` joined
  `bindings.ts` and the typecheck demanded their mocks at the T-141 rebase
  — fixtures in the generator (S/M presets carry two rows each), the
  store's `(created_at, id)` stable order, the typed stale-id
  `E_VALIDATION`, and the D-106 journal shape (`project.changed` with the
  path in `details`, no `exclusion.removed` member yet). Writes mutate
  an in-memory store (`mock/state.ts`) that reads back honestly
  (add→list, set→get, journal→activity_list); error arms reject with the
  §7.10 `AppError` shape; read boundaries validate like the real ones
  (`activity_list`: limit 1..=500, offset ≥ 0, DB-002 shapes, non-inverted
  range, closed action/result vocabularies — the 21-member list mirrors
  `action_log.rs`, extended together with the store; `settings_set`: known
  key through a Map lookup — prototype names are not keys — §6.5 range
  shapes, canonical re-serialization, all-or-nothing). Known mock limits,
  documented in the module header: no filesystem (the `repo_root_offered`
  arm never triggers; SEC-003 session gate and §6.9 name-length validation
  not modelled), the write-access probe always answers "writable",
  recovery commands skip the recovery-mode guard, `markdown_preview` is a
  small escaper (SEC-005 policy) with the one D-15 warning, D-29's
  tie-break is palette order, not an id hash.
- **Fixtures.** `src/ipc/fixtures/generator.ts`: seeded mulberry32 PRNG
  (in-house, ~40 lines — no new dependency; new deps are an owner
  decision), fixed base timestamp, UUIDv7-shaped ids; fully parameterized
  profiles with the §12.2 presets S/M/L (L = 500/1000+100/100 000/3,
  opt-in only — the unit suite never builds it). Referential integrity and
  id uniqueness are asserted at generation time (a broken fixture throws,
  never reaches a test as a silent wrong fact). Library/assignment
  fixtures use §6.2-DDL-shaped local types until the Library/Assignments
  command groups generate wire DTOs; projects reuse the T-072 wire type.
- **Events.** `bindings.ts` generates NO events today (checked): the five
  §7.10/D-009 events arrive with the backend tasks that emit them, so the
  facade serves commands only and `src/ipc/mock/events.ts` provides the
  typed bus (subscribe/unsubscribe/emit) those wrappers and UI tests will
  key with the generated payload types.
- **Testing seam.** `resetMockState(profile)` seeds a custom world per
  test; Vitest runs everything (including `App.spec.ts`, which reads the
  version line through a real facade round-trip) against the same mocks.

Review: two rounds (deepseek-v4.1-flash, then hy4-flash-free) — findings
and fixes recorded in `.opencode/reviews/t141-round*.md` (owner-local).
Round 1 (2×P1, 6×P2, 11×P3) was fully applied: the tree-shaking fix above
(P1-1); the fixture journal's action vocabulary aligned to the store's
closed 21-member list (`sync.run_finished`, no invented spellings, P1-2);
DB-042's label derivation extracted as one shared `defaultLabel`
(upper-case first, 6 code points — `straße` → `STRASS`, P2-1); archived
rows never dirty (DB-014/DB-021, P2-2); `activity_list` boundary
validation mirrored (P2-3); the settings-key Map lookup (P2-4); real
hidden-instruction coverage — set S now includes one (P2-5); the label
test re-pinned to rule literals (P2-6); plus the substantive P3s
(archived-position ordering, D-29 least-used color, canonical setting
storage, `app_info` version from package.json, Playwright's webServer env
so plain `pnpm run e2e` is mock-backed, hermetic test fixture profile,
event-bus reentrancy). Declined: a "set L actually generates" unit test —
the task brief pins small sets in the suite, L stays opt-in.
Round 2 (hy4-flash-free) verified every round-1 fix in code and approved;
its P2 (the extra-hint shape must reject unknown fields like
`extra_file_from_json`, not silently drop them) and P3 (integer
`limit`/`offset`) were applied before the final amend.

### D-111 — Platform-true facts in tests: OS error codes and canonical path expectations — 2026-11-21

Problem: two Windows-only CI failures in a row (runs 44864444674,
44865217431), both invisible on the Linux dev machine and both in
test expectations, not production code. (1) The T-069 fault-injection
test injected `io::Error::from_raw_os_error(28)` ungated: on Unix 28
is ENOSPC (std maps it to `ErrorKind::StorageFull`), on Windows it is
`ERROR_OUT_OF_PAPER` — so the production `StorageFull` classification
was right to refuse `DiskFull` and the test failed. (2) Four T-074
IPC tests built expected paths with `std::fs::canonicalize`, which
prepends the `\\?\` verbatim prefix on Windows, while every command
returns the DB-030 canonical form (dunce, no prefix); the first run's
leg aborted at (1) before reaching them, masking (2) for one round.

Solution — two rules for every test that states a platform fact:

- A raw OS error is a platform statement. When a test injects or
  asserts one, it must either gate by platform (`#[cfg(unix)]` /
  `#[cfg(windows)]` with the platform's code — ENOSPC 28 vs
  `ERROR_DISK_FULL` 112, both `StorageFull` in std) or use
  `io::Error::new(ErrorKind::…)` when only the kind matters.
- A canonical-path expectation is built with
  `eunomia_fs::paths::canonicalize` (the DB-030 form the commands
  return), never `std::fs::canonicalize` — the two diverge by the
  verbatim prefix on Windows and are identical on Unix, which is
  exactly why Linux runs cannot catch it.

Service commits 799163f (T-069) and 430bf22 (T-074) carry the fixes;
subagent briefs for tasks with IPC/fs tests quote these two rules.

### D-109 — The §12.2 data set generator: repository-driven fill in one transaction, determinism defined over a stabilized view, and the two-crate split (store fill / fs temp Git roots) — 2026-11-20

Problem: T-136 needs the M and L benchmark databases of spec §12.2
(M: 50 projects, 200 instructions; L: 500 projects, 1,000
instructions, 100 headings, 3 variants per instruction, 100,000
assignments — the confirmed upper-limit scenario) plus the temporary
Git folders the future sync benchmarks will write into, produced
deterministically for T-137+. Open questions: where the generator
lives (the store cannot create folders, the fs crate cannot write
rows, and no new crate/dependency edge is allowed); what
determinism can even mean when the repositories own the id and
timestamp columns (UUIDv7 `new_id`, wall-clock `now_utc` — D-031);
how the L assignments spread; and what "generate again into the same
database" means.

Solution: two modules, one per crate, composed by the benchmark
consumer.

- **Store side — `eunomia_store::data_sets`.** `DataSetSpec`
  (projects / headings / instructions / variants-per-instruction /
  assignments-per-project) with the consts `M` and `L` carrying
  §12.2's exact numbers (M: 50/0/200/1/40 → 2,000 assignments; L:
  500/100/1000/3/200 → 100,000). `generate(&mut Connection, &spec,
&[ProjectRoot])` fills an **empty** database through the
  repositories' own create methods only — `projects::create`,
  `create_item`, `create_variant`, `assign` — inside one transaction
  it owns: the standalone-connection seam for benchmarks and
  fixtures, which is why it takes `&mut Connection` and cannot be
  composed under `Db::run` (D-037's executor composition stays the
  app layer's). Integrity is the repositories' guarantee, not a
  generator re-implementation: dense `position`/`sort_order` come
  from the append semantics, DB-010 from the constructive
  main-variant rules, D-02 from `assign`. Typed refusals before the
  first write: `NotEmpty`, `NotEnoughRoots`,
  `AssignmentsPerProjectTooHigh`, `InstructionsRequired`,
  `VariantsRequired`; repository/transaction failures roll the whole
  run back (empty database → a retry converges: the idempotency
  semantics are "fresh fixture or rejection", not merge — pinned by
  a duplicate-path-key rollback test).
- **Determinism = the stabilized view.** No RNG exists to seed —
  every value is a pure function of the profile counts and the row
  ordinal. The contract is defined over what the generator cannot
  control stripped away: `stabilized(conn, base)` reads the ALG-001
  collections back through the repositories with ids mapped to
  creation ordinals, timestamps dropped, and the base prefix
  stripped **by component count** (components split on `/` and `\`,
  empties ignored) from both `path` and `path_key`. Component
  counting — not text-prefix matching — is what makes the rule
  platform-true: DB-030's keying re-spells separators, NFC and case
  (folded keys on Windows/macOS) but never changes component
  structure, so one count strips both columns everywhere (round-1
  review's P1: a single text prefix silently never matches a folded
  `path_key`). Two runs with the same profile and equally named
  roots produce **equal stabilized views** while their raw ids
  differ — pinned as the falsifiable determinism test, with folded
  key spellings in the fixture roots and both the negative half
  (without `base`, the views differ: the stripping is part of the
  definition) and a unit pin of the strip's spelling-agnosticism.
- **The L spread.** Project `p`, assignment `k` → instruction
  `(37·p + k) mod 1000`, variant `(p + k) mod 3`. Distinctness needs
  no coprimality (round-2 review's correction: `k < 200 ≤ 1000`
  alone makes the k-window injective, so no stride check exists);
  37 is purely the spread constant — a hop big enough that
  consecutive projects' windows interleave across the whole library
  instead of stacking into bands. Coverage: all 1,000 instructions
  appear, ≈100 assignments each (90–110 exactly, by the formula).
  The closed form is recomputed in the tests by hand — never through
  the module's helpers — over all 100,000 L pairs and all 2,000 M
  pairs. Layout: headings partition the instructions into near-equal
  consecutive groups (exactly 10×100 for L; the 4/3/3 remainder
  branch and the empty-heading edge pinned separately), heading
  levels cycle `2..=6`, all rows active (§12.2 measures the plain
  upper-limit scenario). Variant names run `Main`, `Compact`,
  `Detailed`, then numbered `Alternative 03`… — distinct NOCASE
  names for any variant count (a cycling scheme would collide on the
  §6.9 active-sibling uniqueness). Variant texts are sized for
  §12.2's sync premise (≈52 characters average → a fully assigned
  set-L project builds ≈10 KB). `set L` also reads back through
  `build_snapshot::read` — the whole ALG-001 graph validates, so
  T-137 benchmarks start from a snapshot the sync pipeline accepts.
- **fs side — `eunomia_fs::temp_repos`.** `TempRepos::create(count,
label)` lays out `<tempdir>/<label>/repo-<i:04>` with a minimal but
  real `.git` (`HEAD`, `config`, `objects/`, `refs/` — git's own
  validity set; the scan's ALG-052 probe and `git rev-parse` both
  accept it), canonical paths + DB-030 keys ready for
  `ProjectRoot`, auto-removed with the guard (the crate's idle
  `tempfile` dependency's first staging use). Labels are validated as
  one safe component (no separators, no `..`) so nothing escapes the
  guard's cleanup. The two crates keep no edge between them —
  benchmarks compose (`TempRepos` → roots → `generate`), the store
  stays filesystem-free, fs stays database-free. Group directory
  stored canonical (macOS `/var` → `/private/var`), roots always
  canonical so `path`/`path_key`/scan root spellings agree.

Tests: 5 unit + 7 integration in `tests/data_sets.rs` (M sizes,
invariants and stabilized collections with M's distribution; L
sizes/distribution/snapshot — the one heavy L fill, ≈25 s in a debug
build; determinism both halves with folded key spellings; typed
rejections writing nothing; heading layout edges 4/3/3 and 1/1/1/0/0
with variant names beyond the third; the duplicate-key mid-fill
rollback; stabilized corrupt-cell propagation through the
`ignore_check_constraints` seam) and 2 unit + 4 integration in
`tests/temp_repos.rs` (naming, unsafe labels, layout, canonical
keys, the scan finding every root, teardown, empty fixture).

### D-110 — vue-i18n 11 with the in-repo `eunomia/no-raw-ui-text` lint gate; pipe plurals as UI-020's "ICU plurals" — 2026-11-20

Problem: UI-020 (spec §11.5) requires vue-i18n with an `en.json` catalog
of dotted feature keys, pluralization ("ICU plurals"), and — per the
card's acceptance criteria — a linter or test that catches a template
string that is not going through a key. Three decisions were open: the
vue-i18n version, the plural format reading, and the enforcement
mechanism (`vue/no-raw-text` was suggested, but eslint-plugin-vue 10.12.1
ships no such rule — verified against the installed package's 256-rule
list; the rule of that name lives in `@intlify/eslint-plugin-vue-i18n`,
an extra dependency outside the card's vue-i18n-only allow-list).

Solution (T-142):

- **Dependency.** `vue-i18n@^11.4.23` (dependencies) — the current v11
  stable line on the registry (`latest` dist-tag), peer `vue ^3.0.0`,
  `node >= 22` (the repo pins node 22/pnpm 12). v12 is alpha-only; v9/v10
  lines carry `legacy9`/`legacy10` dist-tags. Composition API mode
  (`legacy: false`), one instance in `src/i18n/index.ts`, installed in
  `main.ts`; components use `useI18n()`, no global `$t` reliance.
- **Catalog and keys.** `src/locales/en.json`, nested namespaces per UI
  feature reached as dotted paths (UI-020's `project.settings.title`
  convention): `app.*` for the shell today (title, version line,
  tagline, logo alts, tracked-projects plural), later screens add their
  own namespaces. The catalog generic (`MessageSchema = typeof en`)
  feeds vue-i18n's key autocomplete only — its `t()` accepts any string,
  so unknown keys are NOT a compile error; typo protection is behavioral:
  a catalog-coverage test asserts every `t('…')` literal in `.vue`
  templates exists (`te(key)`), and App.spec pins the rendered tagline
  and alts (a missing key renders as the literal key string, warned
  only). `en` is the single locale and the fallback; NFR-004…006's
  backend error texts will resolve against this same catalog when their
  `message_key` consumers land.
- **Plurals.** vue-i18n's pipe-separated plural forms (`zero | one |
other` for English, `{count}` interpolation, count passed as the
  plural choice) — vue-i18n's CLDR plural-rule message format, which is
  the tool-native reading of UI-020's "ICU plurals"; adopting ICU
  MessageFormat syntax literally would need an extra message-compiler
  dependency outside the allow-list. The working UI-visible example is
  the tracked-projects line in App.vue: count from `projects_list`
  through the ipc facade, rendered with
  `t('app.trackedProjects', { count: n(projectCount) }, projectCount)` —
  the explicit numeric argument selects the plural form, `n()` formats
  the interpolated count.
- **The lint gate.** A custom rule, `eunomia/no-raw-ui-text`
  (`src/i18n/no-raw-ui-text.ts`, registered in eslint.config.js for
  `**/*.vue`): a template text node containing any Unicode letter or
  digit is a violation — punctuation/symbols are layout — and so is the
  literal of a user-facing attribute (`alt`, `title`, `placeholder`,
  `label`, `aria-label`, `aria-placeholder`, `aria-description`,
  `aria-roledescription`, `aria-valuetext`, `aria-braillelabel`,
  `aria-brailleroledescription`, plus `value` on submit-style inputs,
  where the browser renders it as the control's label) and an
  interpolation or bound expression that shows bare literal copy —
  `{{ 'Save' }}`, `{{ 'Sa' + 've' }}`, ``{{ `Save` }}``,
  `{{ t('k') + ' — Beta' }}`, `:title="'Save'"`: only `+`
  concatenations, template literals and lone literals are walked, and
  everything inside a `t`/`n`/`$t` call counts as translation data, so
  comparisons (`x === 'en'`) and filter calls never fire; bound key
  expressions (`:alt="t('…')"`) are the sanctioned form. A copy prefix
  around an interpolation (`v{{ version }}` — text-node half — or
  ``{{ `v${version}` }}`` — expression half) is caught too, which is
  why the version line moved into the `app.versionLine` message with a
  `{version}` placeholder. Options `ignorePattern`/`ignoreText` exist
  for the day a genuinely text-like literal needs an exemption. Known
  limit, accepted: copy computed in `<script setup>` and rendered via
  `{{ label }}` is invisible to this template-only rule, and so is copy
  smuggled through non-copy expression shapes (method calls returning
  literals, ternary arms); the catalog-coverage test catches the typo
  half, and a script-side scan would be its own rule if real screens
  start doing it. eslint.config.js
  imports the rule with an explicit `.ts` extension — Node's type
  stripping (default since Node 22.18; CI floats `node-version: 22`,
  local is 22.23) loads it at lint time, and vue-tsc checks the rule
  file itself (under `src/`); the config file belongs to no tsconfig
  project, so its import is unchecked wiring (verified end to end:
  eslint red on a planted raw string, green after removal).
- **The keyed/unkeyed boundary.** Not keyed, by design: mock-layer and
  fixture data (`src/ipc/**` — project names, journal actions, error
  message keys — data the UI renders, not UI chrome), the generated
  `src/bindings.ts`, `index.html` (plain HTML, no Vue template; the
  window title is Tauri's), URLs, CSS classes, and code identifiers.
  The rule scans `.vue` templates only, which matches that boundary.
- **Tests.** `src/i18n/no-raw-ui-text.spec.ts` pins the rule red/green
  with ESLint RuleTester (the card's "red example": a raw template
  string must fail; also the copy-prefix, attribute-literal,
  expression-literal and submit-input `value` cases, plus both exemption
  valves and a non-matching `ignorePattern`) and the eslint.config.js
  wiring (rule present at error severity for `**/*.vue`, guarded by a
  `@ts-expect-error` import of the JS config — no declaration file by
  design); `src/i18n/i18n.spec.ts` pins nested keys, named
  interpolation, the 0/1/2/5 plural forms, the `n()`-formatted count at
  the message level, and the catalog coverage over every `t('…')`
  literal in `.vue` templates; `App.spec.ts` mounts through the i18n
  plugin and proves the plural line end to end over the mock facade with
  exactly 1/2/5/0 active projects, an active-vs-archived counting case
  (3 rows, 2 archived → "1 project tracked"), and the rendered tagline
  and alts (per-test `resetMockState` worlds). The T-141
  mock layer is untouched — all its suites stay green; e2e assertions
  (title, heading text, `v\d` version line) keep holding because the
  keyed rendering produces the same strings.

Review: two rounds — findings and fixes in `.opencode/reviews/t142-round*.md`
(owner-local). Round 1 (deepseek-v4.1-flash): no P0/P1; 2 × P2, 5 × P3,
all applied. P2-1 — the "typed catalog" claim was false (vue-i18n's `t`
accepts any string, so a key typo passed lint, typecheck and every
test): the claim is corrected here and in `src/i18n/index.ts`, and the
real guards added (the catalog-coverage `te()` test and the tagline/alt
assertions in App.spec). P2-2 — the rule missed string literals inside
template expressions (`{{ 'Save' }}`, `:title="'Save'"`): the
`VExpressionContainer` literal case added with red/green pins; the
script-side constant remains a documented known limit. P3s applied:
`aria-valuetext`/braille attributes and submit-input `value` joined the
attribute set (with text-input/option non-cases pinned); the
`ignorePattern`/`ignoreText` exemption valves are tested (including a
non-matching pattern); the plural tests gained an archived-rows case
that separates counting active from counting everything; the eslint
config comment now states the real Node floor (type stripping default
since 22.18) and drops the incorrect `allowImportingTsExtensions`
rationale (the config belongs to no tsconfig project); the count
interpolates through the composer's `n()` ("1,000 projects tracked",
pinned). Declined: reconciling spec §12's dangling NFR-004…006 IDs —
`docs/spec.md` is outside this task's file scope (a docs card material).
Round 2 (hy4-flash-free): no P0/P1/P2; 4 × P3, all applied. The
submit-input `value` check compared the element's source-cased
`rawName` (an uppercase `<INPUT type="submit">` slipped by) — it now
compares the parser's lowercased `name`, with an uppercase red pin;
the expression gate only fired on literal roots (`t('k') + ' — Beta'`,
`'Sa' + 've'`, `` `Save` `` passed clean) — the visitor now walks the
narrow copy-only shapes (`+` chains, template literals, lone literals,
skipping `t`/`n`/`$t` call subtrees), with red pins for all three forms
plus `` `v${version}` `` and green pins for comparisons/filters; the
catalog-coverage scan read raw sources (a `t('…')` mention inside a
comment counted as a usage) — comments are stripped before matching and
the raw-scan intent plus its false-negative surface (aliased `t`,
dynamic keys) documented in the spec; D-110's plural call form was a
wrong shorthand (`t(key, n)`) — it now quotes the real expression.

### D-112 — Project availability as a service: the session diff cache, the baseline rule, and the first specta event (a manual `Event` impl) — 2026-11-20

Problem: T-079 turns D-09/DB-032's three check points (startup,
window open/focus, before a sync) into the §7.10
`state://project-availability` event with "the list of changes".
Five points were open: (1) what the diff is taken against — nothing
in the spec defines a baseline, and an honest first check of 500
healthy projects is not 500 changes; (2) where the last known
verdicts live — the database is meaningless across restarts (a
folder's availability is a fact about the machine's current
filesystem) and no `projects` column exists for it; (3) how the
event reaches `src/bindings.ts` when tauri-specta's `Event` derive
macro is compiled out (the workspace pins
`tauri-specta = "=2.0.0-rc.29"` with the `typescript` feature only;
`derive` is off) and the builder had NO events registered yet; (4)
what a focus-storm does — `Focused(true)` can fire many times a
second and every check probes four metadata syscalls per project
over possibly dead drives; (5) what the future sync pipeline
(§9.4/T-118) actually consumes from this task.

Solution: `src-tauri/src/availability.rs` — a service module, not a
command (the §5.1 layering: the commands layer owns commands, the
service owns the check points), plus one wiring line each in
`lib.rs`.

- _The baseline rule._ The cache is session memory
  (`AvailabilityState`, managed next to `SessionPaths` — infallible,
  present in recovery mode too); a project never probed this session
  counts as `(available, no reason)`. Consequences: a healthy first
  check emits NOTHING (T-072's `projects_list` already probes fresh
  on every read — an all-healthy startup has nothing to tell the
  UI), a project first probed while its folder is gone IS a change
  (D-09's marking must not wait for a second pass), and any later
  contradiction of a recorded verdict is a change in both directions
  (unavailable → available included: the marking must clear). The
  event carries only the changed projects, in `sort_order` (probe
  order), and is not emitted at all for an empty diff — repeated
  checks with unchanged folders are silent, which is what makes
  rechecking on every focus safe. The payload is minimal by design:
  `project_id` + `available` + `unavailable_reason` per change — the
  UI already holds names keyed by id from `projects_list`, and a
  copy in the event could only drift.
- _At-least-once delivery._ The probes are recorded only AFTER the
  emit succeeded: a failed emit leaves the cache stale so the next
  check point re-detects and re-emits the same diff (a lost UI
  update would be permanent until the folder changed again; a
  duplicate — possible when the guarded spawn overlaps the sync
  pipeline's direct await — is a benign no-op for a listener that
  sets state keyed by id). The emit failure itself is a `warn`
  facade line (`availability_emit_failed`), not an error of the
  check.
- _The first specta event, by hand._ `impl tauri_specta::Event for
ProjectAvailabilityEvent` with `const NAME =
"state://project-availability"` — the spec's literal wire name
  (tauri permits its characters: alphanumeric, `-`, `/`, `:`, `_`).
  The derive macro would build the name from the struct's identifier
  and cannot produce the `state://` prefix anyway. Registered in
  `specta_builder().events(collect_events![…])`; `bindings.ts` now
  carries `events.stateProjectAvailability` (the camelCase accessor
  of the wire name) with the `makeEvent` runtime — the first events
  export in the file (D-108's "no events today" note is thereby
  superseded; the mock layer's typed event bus keys with the
  generated payload types as it planned). The verdict spellings
  (`missing`/`not_a_directory`/`not_readable`/`no_git_entry`) are
  single-sourced in `Verdict::probe`, and `commands.rs`'s
  `availability_of` (T-072) now delegates to it — the column and the
  event cannot drift. Two recorded follow-ups for the first frontend
  consumer (round-2 review P3-2/P3-3; both files are outside this
  task's scope): the webview cannot subscribe yet —
  `capabilities/default.json` grants nothing, and the generated
  `events.…listen()` needs `core:event:listen` (with the matching
  `config_tests.rs` pin) added by the frontend task that first
  listens; and `src/ipc/mock/events.ts`'s header still says the
  bindings carry commands only — stale since this commit, to refresh
  when the mock bus adopts `ProjectAvailabilityEvent` as its first
  key.
- _Triggers._ Startup: `availability::spawn_recheck` spawned in the
  setup hook AFTER the database is managed and the retention task —
  a background task, because dead-drive probes must not hold the
  launch (SYS-001, NFR-002). Window: every window event routes
  through `availability::on_window_event`; `Focused(true)` — which
  is also how a window's opening is delivered — is the trigger, and
  ANY window qualifies (every window renders project state; the slot
  makes extra passes harmless). Sync: the reusable entry is
  `recheck_and_emit` itself, awaited directly — the pipeline (T-118)
  calls it as its DB-032 checkpoint, gets the diff back to branch or
  log on, and keeps its own §9.4 phase-1 per-project probes (a
  verdict can go stale between the checkpoint and the write; the
  cache is never the pipeline's gate).
- _The in-flight slot._ `spawn_recheck` coalesces: an atomic flag
  taken by `compare_exchange` and released by an RAII guard's `Drop`
  — a `Send` guard (it owns an `Arc`), so it is held across the
  task's awaits for the whole pass, DB read to emit. Nobody ever
  waits on the flag, and it frees itself on panic (the unwind drops
  the guard) so a crashed recheck cannot silence every later one.
  (The first cut used `if slot.try_lock().is_err() { return; }` over
  a `std::sync::Mutex` — the guard was a temporary of the `if`
  condition, dropped at the condition's end, and being `!Send` it
  could not have crossed the await at all: the slot guarded nothing.
  Round-1 review P1-1; the regression is pinned by a test that parks
  a real pass mid-run on the gated database connection and observes
  the slot taken.)
- _Recovery mode._ No `Db` → the honest `NoDatabase` `AppError`
  through the same `managed_db` edge every database command uses,
  logged by the spawned triggers through
  `availability_check_failed` (the `activity_list_failed` shape); no
  event, no panic — the recovery screen has no project columns to
  mark.
- _Tests._ 9 unit (the pure diff semantics: baseline both halves,
  both directions, unchanged-both-ways, reason flip, prune-then-
  return-is-first-probe, probe order; the spec-literal event name;
  the slot's take-once/free-on-drop mechanics) + 8 over the mock
  runtime in `ipc_tests.rs`: the card's criterion end to end
  (`fs::rename` the real `.git` fixture away → the recheck answers
  exactly the one change and the typed listener receives it;
  unchanged recheck silent; rename back → the recovery change and
  event), the missing-at-startup half, the window seam
  (`Focused(true)` rechecks and emits, `Focused(false)`/`Resized`
  are ignored and emit nothing while a diff is pending — silence
  proves no recheck), the startup spawn awaited by its returned join
  handle (the deterministic seam; production drops it), the
  coalescing slot BOTH ways — a real pass parked mid-run on the
  gated database connection (observed taken, second spawn silent,
  gate released → the parked pass emits, slot frees) and the simpler
  externally-held slot — the empty active list, the honest
  no-database answer for both the direct entry and the spawn, and
  the registration pin (`lib.rs` source scan, the D-034 rationale)
  covering the `collect_events!` wiring AND both trigger call sites
  (round-1 review P2-1). Rust-side listeners deliver synchronously
  inside `emit`, so every no-event assertion is a `try_recv` after
  the call returned — no timeouts (the one bounded wait observes a
  pass enter its in-flight state).

### D-113 — The UI-011 AppError alert: honest message_key fallback, code-keyed §10 action groups, visible OS facts, and the copyable envelope — 2026-11-21

Problem: T-143 turns UI-011/NFR-018 (an error message answers what
happened, where, why, and what to do, with technical details collapsed
behind a copy button) into the reusable frontend component over the
T-013 `AppError` wire form (§7.10: `code`, `message_key`, `params`,
`details`). Open questions: the fallback when `message_key` is not in
the catalog (vue-i18n renders the literal key string, warned only —
D-110's P2-1 finding); what "where" renders given that param shapes
beyond path/project are not yet stable across producers; which OS facts
stay visible (BKP-005 vs the bindings' "os_message is envelope
material"); how the §10 user actions group; and how copying works
without a clipboard plugin dependency.

Solution (T-143): `src/components/AppErrorAlert.vue` — a pure
presentational component (`error: AppErrorWire` + optional `contextKey`
in, no IPC), plus `src/components/app-error-actions.ts` (the §10
grouping) and the `errors.*` namespace in `src/locales/en.json`.

- **Fallback policy.** The title resolves `te(message_key)` first:
  present → `t(message_key, params)` (params interpolated per UI-020,
  NFR-006); absent → `errors.fallbackTitle` ("Something went wrong").
  The raw key never reaches the UI. The code, where row, action row and
  envelope do not depend on the key and keep rendering — drift degrades
  to a generic title, not a broken alert. The optional `contextKey`
  follows the same `te()` discipline (unknown key → no context line).
- **"Where" and the message templates.** A labeled row for exactly the
  `path`/`project` params (§10's "where" facts), composed per what is
  present (`{project} — {path}` / `{path}` / `{project}`); absent both
  → no row. Message templates interpolate the params their producer has
  pinned — the D-113-first-cut made every message placeholder-free
  because `E_VALIDATION`'s param shapes are not stable across producers
  (`field`/`key`/`id` + `value`/`reason` variants); the round-1 review
  (P2-1) corrected that to the honest split: producer-STABLE keys
  interpolate (`errors.db_too_new` → schema versions,
  `errors.db_corrupt` → the quick_check `detail`,
  `errors.update_migration_failed` → covered by its action, all pinned
  by the db suite), while the still-unstable families (`E_VALIDATION`)
  stay placeholder-free until their producer task pins the shape — the
  component always passes `params` to `t()`, so a message growing
  placeholders needs no component change, and every param is always in
  the copyable envelope.
- **"Why" — BKP-005 stays visible (D-034's pointer honored).** The why
  row always shows the §10 code; `details.os_code` and `details.os_kind`
  join it when present (`E_WRITE_FAILED — OS error 13
(PermissionDenied)`; kind alone when there is no raw code — BKP-005's
  "OS code **or** error type"). `os_message` stays envelope-only, per
  the bindings' documented contract ("raw diagnostic material … never
  interpolated into the localized message"). The `os_kind` tags stay
  machine spellings (round-1 P3-8 declined as out of scope: humanizing
  them is a catalog task of its own when a producer family makes it
  matter).
- **"What to do" — groups keyed by `code`, not `message_key`, plus one
  sanctioned family-key override.** §10's action column is per-code and
  the `ErrorCode` union is closed, so the map in `app-error-actions.ts`
  is `satisfies Record<ErrorCode, string>`: a new code without a group
  is a compile error, and message-family keys ride their code's group
  for free (keying by the open `message_key` string would need a
  per-family lookup rule that can miss one). Twenty codes → sixteen
  code groups, merged only where §10's actions genuinely coincide:
  `fixTarget` (the three E_TARGET_* rows), `restoreFromBackup`
  (WRITE_FAILED + VERIFY_FAILED) and `retry` (DB_LOCKED's "retry" +
  UPDATE_FAILED's "retry later" — split from SYNC_RUNNING's `wait` by
  the round-1 review P3-1: waiting presumes a running operation the
  other two do not have); each remaining code keeps its §10-specific
  action — merging further would blur instructions the spec spells out.
  The one override (`ERROR_ACTION_OVERRIDES`, consulted on an exact
  own-property `message_key` match before the code map — round-2 P3-1:
  `in` would walk Object.prototype and turn a "toString" key into a
  bogus catalog key) is `errors.update_migration_failed` →
  `installPreviousVersion`: the family key `DbInitError::app_error`
  already emits for UPD-004, whose remedy ("install the previous
  version, pointing at the `db-backups/pre-migration-*` copy") is
  knowingly not §10's row for `E_UPDATE_FAILED`; the producing code
  documents that "the UI shows the UPD-004 instruction, not the §10
  row's prose". The override's action interpolates `{backup_path}`.
  Group texts live at `errors.actions.<group>`.
- **The envelope.** Native `<details>/<summary>` (the a11y disclosure
  pattern for free) holding the full §7.10 wire form as 2-space JSON in
  a fixed field order (`code`, `message_key`, `params`, `details`) and
  one Copy button. Copying uses `navigator.clipboard.writeText` with
  the `execCommand` textarea fallback for contexts without the async
  clipboard API — no clipboard-manager plugin (a new dependency is an
  owner decision). The feedback is a three-state machine
  (`idle`/`copied`/`failed`): the button label flips to "Copied" or
  "Copy failed" and reverts after 2.5 s; every failure path — writeText
  rejecting, execCommand returning false, execCommand throwing
  (round-2 P2-2: the throw is caught, so it lands in `failed` instead
  of escaping the click handler as an unhandled rejection) — reaches
  the failed state. The legacy path's textarea is removed in a
  `finally` and the pre-copy focus restored (round-1 P3-3).
- **Accessibility.** `role="alert"` wraps the message cluster only
  (context, title, where/why/action rows) — mounting the alert
  announces the error, while the collapsed disclosure, the Copy button
  and the copy-feedback live region are siblings outside it (round-1
  P3-2: a live region nested in another re-announces unpredictably).
  The feedback is announced by a visually hidden `role="status"`
  region and is never color-only (the label changes). Real `<button>`
  (keyboard + focus visible via `:focus-visible`, ≥24 px target); the
  error state is carried by icon + text, not color alone; dark scheme
  follows the OS.
- **The `E_VALIDATION` family question (round-1 P3-5, resolved by
  record).** vue-i18n cannot make `errors.validation` both a string and
  the parent of `errors.validation.<field>` family members. The default
  key stays the string leaf (T-013's contract, pinned both sides);
  family members are NOT added under it — when the first family member
  lands, the default leaf moves to `errors.validation.default` in the
  same coordinated change (D-034 already frames message-key renames
  that way, pinned by tests on both sides). Until then a family key
  honestly hits the title fallback while riding its code's action.
- **Catalog pins.** `AppErrorAlert.spec.ts` mirrors the twenty default
  `errors.<snake_case>` keys from `errors.rs`'s registry test and
  asserts each resolves in `en.json`, pins all twenty code→group
  mappings and the override, and asserts an action message for every
  group — the frontend half of the T-013 key contract; the Rust-side
  bindings pin is the other half. The component is not wired into a
  screen yet (callers catch the rejected `ipc.commands.*` promise and
  render it — the screens' tasks): the same tested-today/consumed-later
  shape as the registry itself. `src/ipc/index.ts` re-exports
  `AppErrorWire`/`ErrorCode` so UI code never imports `bindings.ts`
  (D-003).

Review: two rounds — reports in `.opencode/reviews/t143-round*.md`
(owner-local). Round 1 (deepseek-v4.1-flash, changes requested):
1×P1, 3×P2, 8×P3. Applied: P1-1 — the one non-default key the Rust
side already emits (`errors.update_migration_failed`, UPD-004) got its
catalog message + the family-key action override (the grouped "wait"
advice was knowingly wrong for a rolled-back migration); P2-1 —
producer-stable messages now interpolate their params (db_too_new,
db_corrupt) with title tests; P2-2 — the legacy-copy test pins the
copied payload (textarea value, `'copy'` argument, removal), plus a
writeText-rejects→legacy-succeeds case; P2-3 — a table-driven pin of
all twenty code→group mappings with exact-text assertions for the
merged groups; P3-1 — `wait` split from `retry`, `fixCauseAndRetry`
no longer points at the collapsed details; P3-2 — `role="alert"`
narrowed to the message cluster; P3-3 — try/finally + focus restore in
the legacy path; P3-4 — the three-state copy feedback with a failed
test; P3-6/P3-7 — per-test mounts, redundant assertion dropped.
Declined: P3-8 (humanizing `os_kind` tags — out of scope). Round 2
(hy4-flash-free, changes requested): 2×P2, 2×P3, all applied. P2-1 —
this entry's body updated to the applied design (the five stale
pre-fix passages); P2-2 — a throwing `execCommand` now lands in the
`failed` state instead of an unhandled rejection, with a test; P3-1 —
the override lookup is an own-property check (`hasOwnProperty.call`;
lib is ES2020, no `Object.hasOwn`), with a prototype-named-key test;
P3-2 — containment assertions (disclosure/copy/status outside the
alert), the failed-state revert test, and the payload assertion in the
writeText-rejects case.

### D-114 — Frontend date/time helpers on `Intl`: the `GMT±hh:mm` zone form, per-call locale, and the typed error boundary — 2026-11-21

Problem: UI-020 (spec §11.5) requires dates "formatted with `Intl` in
the system locale" and "time is always shown with the time zone";
MAIN-027's sync panel and PRJ-021's settings modal show the last-sync
time as "local time and time zone" — but the spec pins no zone FORM
(IANA name or offset), no default/override split for locale and zone,
and no behavior for bad inputs (an unparseable DB-002 string, an
unusable zone identifier). The backend marker precedent (ALG-032,
D-104) prints "name + offset" into file contents — a different surface
with different rules.

Solution (T-144): `src/date/` — pure helpers over
`Intl.DateTimeFormat`, no dependencies, no consumers wired yet (the
future UI column over the existing `projects.last_synced_at` wire
field and the sync panel import them).

- **Zone form: the offset alone.** `formatDateTime` renders
  `timeZoneName: 'longOffset'` — `Nov 17, 2026, 2:03:22 PM GMT+01:00`
  — uniform for every zone spelling including UTC (`GMT+00:00`, not
  bare `GMT`): explicit components make V8 print the zero offset.
  Rationale: the UI spec says "time zone", and its own offset habit is
  `UTC±hh:mm` (ALG-032's fallback arm, DB-002's `Z`); the
  IANA-name-plus-offset form is the file marker's contract (ALG-032,
  D-104), deliberately NOT reused for the UI. A UI column showing
  `Europe/Prague` on every row would repeat what one offset says once.
- **Locale and zone are parameters, never globals.** Both default to
  `undefined`; the locale flows to `Intl` as-is (its own system-locale
  default, per UI-020), and an undefined zone OMITS the option so the
  engine applies the system zone natively — which also degrades an
  unidentifiable system zone (`Etc/Unknown`) gracefully to UTC.
  `systemTimeZone()` exports the resolved name when Intl can both
  resolve and accept it, and otherwise a synthesized fixed-offset
  identifier (the round-1 POSIX-TZ fix, below) — the return value is
  always safe to display or re-pass; `Etc/Unknown` must be sanitized
  because V8 REJECTS it as an explicit `timeZone` while happily
  defaulting to it.
- **ECMA-402 constraint that shaped the code:** `dateStyle`/`timeStyle`
  cannot combine with ANY component option — `timeZoneName` included —
  (TypeError, verified on Node 22.23/V8); hence explicit
  year/month/day/hour/minute/second components for the zone-bearing
  form, `dateStyle: 'medium'` for the date-only form (`timeZone` is not
  a component, so it composes there).
- **Zone parameter accepts IANA names and `±hh:mm` fixed-offset
  identifiers** (`+05:45`), the engine's supported subset; the `UTC±h`
  spelling throws in V8 and is rejected like any unknown name. Bad
  inputs throw a typed `DateFormatError` with a `kind` discriminant
  (`'invalid-date'` | `'invalid-time-zone'`) and the engine's
  `RangeError` as `cause` — callers branch on `kind`, not message
  text. The zone is validated by a probe constructor with a FIXED valid
  locale, so a structurally invalid caller locale (e.g. `en_US`)
  surfaces as the engine's own unwrapped `RangeError`, never
  misattributed as a zone failure. No silent fallback: a wrong zone
  name must fail loudly (the system-zone default is a documented
  absence, not a catch).
- **`formatDate` takes a zone too.** A date-only rendering still needs
  the zone to pick the calendar day: `2026-11-17T23:30:00Z` is Nov 17
  in New York, Nov 18 in Tokyo (pinned). No zone NAME is rendered — a
  date carries no wall time to qualify.
- **Determinism (CI runners have different zones):** every expectation
  uses a fixed UTC instant (the DB-002 string form), an explicit zone
  and an explicit locale; nothing depends on the process TZ. The two
  default-behavior pins are runner-independent equivalences:
  `formatDateTime(t)` must equal the call with `systemTimeZone()`
  explicitly, and the parameter-less call must equal the call with
  `resolvedOptions().locale` — both hold on any runner because both
  sides read the same system default (verified for `TZ=`,
  `TZ=Invalid/Zone` and a named zone). DST coverage: the Prague
  fall-back pair (same wall clock 2:30 AM, offset `+02:00` → `+01:00`)
  and the New York spring-forward pair (wall clock jumps 1:30 → 3:30)
  pin that the offset is the one in force AT the instant; Tokyo/Accra
  pin fixed offsets; `+05:45` pins the offset-identifier arm. The
  always-with-zone criterion (UI-020) is pinned twice: exact strings
  (offset inside them) and a `/GMT[+-]\d{2}:\d{2}$/` property check
  across all zone spellings — dropping `timeZoneName` fails both.
  Expected strings verified against Node 22.23's ICU; locale pins use
  `en`, `cs-CZ`, `de-DE`.

Round-1 review (deepseek-v4.1-flash) — CHANGES REQUIRED: 1×P1, 1×P2,
5×P3; all taken. What they changed:

- **POSIX-TZ runners broke the system-zone contract (P1, reproduced).
  `TZ=UTC-3` makes the engine APPLY +03:00 while
  `resolvedOptions().timeZone` stays `undefined`, so `systemTimeZone()`
  returned `'UTC'` — the "safe to re-pass" promise was silently wrong
  and the default-equivalence pin failed on such a runner. Fix:
  `systemTimeZone()` now synthesizes a fixed-offset identifier
  (`offsetTimeZoneIdentifier(-new Date().getTimezoneOffset())`, a new
  exported pure helper) whenever the resolved name is missing or
  unusable. POSIX rules WITH transitions (`PST8PDT,…`, even with fake
  abbreviations) resolve to real IANA names in ICU — verified — so the
  synthesized arm only fires for fixed-offset POSIX zones, where it is
  exact at every instant; the call-moment caveat is documented anyway.
  The suite now passes under `TZ=UTC-3`, `TZ=`, `TZ=Invalid/Zone` and
  `TZ=PST8PDT,…` alike (the equivalence pins hold on each).
- **`longOffset` needs WebKit ≥ 15.4 (P2).** NFR-001's floor is macOS
  12, and the Tauri WKWebView on macOS 12.0–12.2 throws a bare
  RangeError on the option value — every `formatDateTime` call would
  die there. Fix: one-time feature detect at module load, degrading to
  `timeZoneName: 'short'` (`GMT+1`, `CEST`) — still an explicit zone
  marker per UI-020; only the exact `±hh:mm` shape is lost on those
  WebKits. The suite (Node 22) always exercises `longOffset`; the
  `'short'` arm is one line resting on Intl's own contract.
- **Formatter cache (P3).** Construction ≈ 0.36 ms vs ≈ 3.6 µs per
  cached `format()` (reviewer-measured, Node 22); the matrix may format
  hundreds of stamps per render. Formatters are now cached per (shape,
  locale, zone) in a Map capped at 64 and cleared wholesale on overflow
  — the overflow path trades speed, never output.
- **Test honesty (P3).** Two tautological assertions dropped
  (`instanceof Error` under an `instanceof DateFormatError` guard; a
  try/catch re-proving what `toThrow(RangeError)` already implied —
  `DateFormatError` extends `Error`, not `RangeError`). The cs-CZ/de-DE
  exact dateTime strings became structural pins (24-hour wall time +
  `GMT±hh:mm` suffix — the spec-relevant halves), because month
  abbreviations drift with CI's floating Node 22 ICU; the all-numeric
  cs-CZ date-only pin stays exact (D-111 spirit: exact pins only on
  platform-stable facts). `formatDate` gained its own default-zone
  equivalence pin. D-114's "future `last_synced_at` column" wording
  corrected — the wire field exists; the UI column is the future part.

Round-2 review (hy4-flash-free) — APPROVED, no P0/P1; 3×P2, all taken:
the locale cache key now JSON-encodes pre-stringified parts (a plain
comma join let the invalid string locale `'en,fr'` silently reuse the
`['en','fr']` array's formatter — call-order-dependent acceptance, now
impossible: string keys are quoted, array keys bracketed); the
invalid-locale test now passes a zone so the fixed-'en' validation
probe it describes actually runs in the call; D-114's body was brought
back in sync with the code (the pre-round-1 text still claimed a
`'UTC'` fallback for unusable names where the code synthesizes the
offset identifier — the entry contradicted its own round-1 record).

### D-116 — The event-listen capability and the typed mock bus — 2026-11-21

Problem: T-246 closes D-112's two recorded follow-ups. (1) The webview
cannot subscribe to `state://project-availability`: D-024's capability
grants nothing, and the generated `events.…listen()` reaches the core
event plugin's `plugin:event|listen` command, which the ACL gates. (2)
`src/ipc/mock/events.ts` still documents `src/bindings.ts` as
commands-only — stale since T-079 — and its bus is loose-keyed
(`Record<string, unknown>`), so the mock layer had no typed seam for the
event the backend now emits.

Solution:

- _The capability grant, spelled honestly._ The task card's shorthand
  `core:event:listen` is not an ACL identifier: tauri 2.12.6's core
  event permissions are `core:event:allow-listen`/`-unlisten`
  (plus `-emit`/`-emit-to` and the `default` set), and an unknown
  identifier fails the build (`permission … not found` in the ACL
  resolver — verified against the installed
  `tauri-utils-2.10.3`/`tauri-2.12.6` sources). The grant is therefore
  exactly `["core:event:allow-listen", "core:event:allow-unlisten"]`:
  the SUBSCRIPTION lifecycle, both halves — the JS `listen()` returns
  an unlisten function that invokes `plugin:event|unlisten`, so a
  listen-only grant would let the webview subscribe but never end a
  subscription (a listener leak per mount; the mock bus tests
  unsubscribe as part of the same seam, and the real side must support
  what the mock promises). Emits and everything else stay denied
  (SEC-001: events are emitted from Rust). The pin in
  `config_tests.rs` is renamed and repinned from "empty set" to this
  exact two-element list (proven: dropping the grant turns it red with
  the pin's message), the module docs' "no permissions at all" sentence
  is updated, and the no-app-ACL pins keep their meaning with corrected
  wording — application commands are still ungated and still granted
  nothing.
- _The typed mock bus._ `IpcEventMap` (in `mock/events.ts`) keys the
  bus by the §7.10 wire names with the payload types generated into
  `bindings.ts`, re-exported through `contract.ts` as the
  `*_Serialize` wire halves (`ProjectAvailabilityEventWire` — the
  shape the Rust side emits, the same rule the command wire types
  follow). `mockEventBus` is now `EventBus<IpcEventMap>` (strictly
  keyed; tests for names the backend does not emit yet instantiate
  local buses, the pattern the T-141 spec already used), and a small
  factory `busEvent(bus, name)` binds one event into a standalone
  `{ on, emit }` accessor — `mockEvents.stateProjectAvailability`,
  named after the generated camelCase accessor so the facade task can
  serve both backends behind one `ipc.events`. The accessors are
  deliberately synchronous over the mock bus, unlike the real
  promise-based `listen`/`emit`; bridging that difference is the
  facade task's line, not the mock's.
- _The mock event source._ `applyMockAvailability(verdicts)` in
  `mock/state.ts` — the twin of the backend's check pass
  (`availability::recheck_and_emit`): a per-session verdict cache
  (cleared by `resetMockState`, the mock twin of `AvailabilityState`)
  is what verdicts diff against, NOT the row — a project never probed
  this session counts as `(available, no reason)`, D-112's baseline
  rule, so a healthy first check is silent while a first probe of a
  gone folder IS a change (reproducible: a fixture-unavailable row's
  first CONFIRMING verdict emits). Only contradictions are changes
  (flip either direction, or a new reason on a still-unavailable
  project); available ⇒ `unavailable_reason: null` (the input is a
  discriminated union, so "available with a reason" is
  unrepresentable; reasons are the four D-084 spellings); one call =
  one pass = ONE event with the changes in `sort_order`, `null` and
  no emit for an empty diff (an empty input array included); the pass
  is all-or-nothing — an unknown id throws before any verdict is
  applied, cached, or emitted (a half-applied silent pass is the
  divergence the backend's at-least-once rule exists to prevent), and
  a mid-pass revert leaves no entry (the diff is the pass's net
  effect, last verdict per id); active rows only (the backend's pass
  reads the active list — an archived project is skipped, not
  recorded); verdicts are recorded on the cache and the rows, so
  `projects_list` reads them back. Like the backend seam, it returns
  the emitted diff (or `null`) — the observable the sync pipeline's
  checkpoint answers with. Known limit: the backend prunes a project's
  cache entry when it leaves the active list; the mock's cache is
  replaced only by `resetMockState` (no mock command mutates project
  state yet, so the archive/restore round-trip is a hand-edited row).
- _Deferred by file scope (round-1 review P2-1/P3s), for the owning
  tasks._ The capability grant makes these current-state statements
  stale, all in files this task must not touch: `ARCHITECTURE.md`
  ("the capability grants it no permissions at all"), `commands.rs`
  ("capabilities/default.json stays empty" ×2 and one "stays
  plugin-free" comment), `lib.rs` ("only application commands … cross
  the IPC boundary"), `recovery_tests.rs` ("The empty-capability
  contract" heading phrase), `docs/spec.md` §5.6 SEC-001 ("capabilities
  lists only the app commands"), and `src/ipc/index.ts` ("the facade
  serves commands only today"). Same for the mock's wire-name literal
  not being tied to the backend's `Event::NAME` (the generated
  bindings export no name constant; a Rust-side rename would leave the
  mock silently diverged — the facade/consumer task should pin the two
  together), and for `core:event:allow-listen` not being
  event-name-scoped (tauri 2.12.6 core event permissions carry no
  per-event scope; the webview may listen to any app event — accepted
  residual risk: every §7.10 payload is data the commands already
  return to the same window).

Review: two rounds on different models (reports in
`.opencode/reviews/t246-round1.md` / `t246-round2.md`, owner-local).
Round 1 `deepseek-v4.1-flash` — no P0/P1; P2-2 (the pass applied
verdicts before the unknown-id throw and emitted nothing) and P2-3
(the row-as-cache seed made D-112's baseline rule unreproducible)
fixed by the all-or-nothing rewrite above; P2-1 and the P3s resolved
by the deferral bullet, the `mockEvents` doc rewording, the unused
`ProjectAvailabilityChangeWire` re-export removal, and the spec
hardening (guaranteed unsubscription, empty-pass and revert-pair and
baseline tests, read-back through `projects_list`). Round 2
`hy4-flash-free` — APPROVE, no P0/P1/P2; its three optional P3s
applied: the `mockEvents` doc now says explicitly that only
`listen`/`once` are usable from the webview while the generated
`emit`/`emitTo` stay denied (SEC-001) and the mock `emit` is a
test-only convenience; the unknown-id test now also pins the cache
half of all-or-nothing (a valid follow-up pass must still emit); the
baseline test pins the fixture's `'not_readable'` literal instead of
casting the row value through the reason union.

### D-115 — `projects_reorder`: the active-order input spliced around the archived rows' own slots, the verified no-op, and the bulk `project.changed` journal — 2026-11-21

Problem: T-080 turns §7.10's `projects_reorder` into the PRJ-019/020
persistence ("the order is saved right away", AC "kept after a
restart") over the frozen T-071 repository (D-093), whose `reorder`
takes a permutation of EVERY id in the table. Four points were open:
(1) what the UI sends — the store wants the whole table, the drag
surface is the active columns alone; (2) where the archived rows land
in the composed permutation (D-093's `reorder` doc says "builds the
new order from `list(All)` … splices the move in, and passes the full
permutation", or "list(Active) plus the archived ids in their relative
order" — the parenthetical reads as concatenation, the main clause as
interleaving, and the two differ); (3) how the D-093 refusal forms
map onto §7.10 when they reach a command that CAN produce them; (4)
whether a reorder journals, and in what shape, given LOG-001…004's
"changing projects" and D-051's frozen vocabulary.

Solution: `projects_reorder(ordered_ids: Vec<String>) -> ()` in
`commands.rs`, registered in `collect_commands!`, plus a
`logging::projects_reorder_failed` facade edge (the D-096 follow-up
pattern D-103/D-106 adopted).

- _The input is the complete ACTIVE order._ PRJ-019/020 reorders
  columns — exactly the rows `projects_list`'s default active read
  serves; the archived screen is not a column. The command validates
  the list against the active set read in the same transaction, and
  composes the full permutation itself (the store stays frozen,
  D-093).
- _Archived rows keep their own slots._ The new active order is
  zipped into the stored table order: every slot the stored table
  fills with an active project takes the next input id, every
  archived row keeps its exact `sort_order`. The concatenation
  reading of the store doc's parenthetical was rejected: it MOVES
  archived rows (to the tail) on a drag that does not concern them,
  while the splice reading leaves their positions literally
  untouched — the same least-destructive posture D-093 already chose
  for archive/restore ("keeping the position… reinforced by
  PRJ-013…018's restore 'bringing the project back' — with its
  place"), and §6.4's archive row "says nothing about `sort_order`".
  The restore flow lands a restored project back at its own place
  among the columns under this reading, not at the tail.
- _The three refusal forms are the typed `E_VALIDATION`, mapped from
  both sides._ The command's own validation counts the D-093 forms at
  the active-set boundary — input ids that name no active project
  (unknown OR archived: a stale screen's list), active projects the
  list omits, and repeats — and refuses with the `rejected_field`
  shape (`field: "ordered_ids"`, the reason naming all three counts,
  the value echoing only the offending CALLER ids — capped at five
  ids and 64 code points each, or the sent count when nothing
  offends — never stored ones: stored ids stay off the params
  surfaces, the D-096 rule). Nothing is
  written. The store's `ReorderMismatch` cannot follow a passing
  validation (one connection serializes the read and the write,
  D-037) but is mapped through the SAME refusal if it ever fires
  anyway — convergence, never a masked conflict (the D-103
  duplicate-race rule); the shared `projects_store_to_app_error`
  keeps its fallback arm unchanged.
- _A reorder journals as ONE bulk `project.changed` entry._
  LOG-001…004 puts "changing projects" in the journal and D-051 maps
  it to `ProjectChanged`, so a reorder is journaled — at info/user/ok
  inside the operation's own transaction, with `Details::Bulk` (the
  enum's own "how many rows a bulk operation changed") carrying how
  many POSITIONS moved, and no entity and no `project_id`: the change
  is one fact about many projects at once, and the Activity window's
  project filter must not mis-attribute it to one of them (the
  `exclusion_remove` no-entity rationale). The `Bulk` field doc says
  "changed assignments" — the assignments producer's own reading; the
  enum-level wording is the authority this command relies on, and a
  store-side task may generalize the field doc.
- _The replayed drag is a verified no-op._ The composition returns
  how many positions actually change; zero means the re-sent order
  equals the stored one, and the command writes NOTHING — no
  renumbering, no journal entry (the journal must not claim a change
  that never happened, D-051; the store's fieldless-update no-op
  precedent). The shortcut yields only while the stored positions are
  already the gapless `1..=N` (DB-005): a tampered, non-gapless table
  falls through to the renumbering — the one writer that restores the
  invariant (the round-1 review's defense). A retry storm around one
  drag converges with zero side effects (the
  make-operations-idempotent rule).
- _Acceptance, at the reopen boundary._ The AC pair is proven at the
  layer the restart acts on: the IPC test invokes the command, then
  drops EVERY handle to the first connection (the webview and the
  test's own `Db` clone both keep the connection alive — D-037's
  `Arc<Mutex<…>>` — so `drop(webview)`, `drop(database)`,
  `drop(app)`), reopens the same data root through `Db::open`, and
  reads the stored order back — the dragged order with the gapless
  `1..=N` intact (the `app_info` second-open shape; a process-level
  restart adds nothing the reopen does not already exercise).

Tests: 4 unit tests pin the pure seam (`composed_reorder` /
`rejected_reorder`): the splice around archived slots with the moved
count, the no-op boundaries (current order, single column, empty
matrix), every mismatch form counted (unknown, archived-as-unknown,
missing, empty-list-vs-actives, repeats), and the envelope params
(caller ids echoed and capped, the sent count for the missing-only
form). 10 IPC tests over the mock runtime: the reopen criterion, the
archived-slots composition (an archived row among actives and one at
the tail), the one-transaction renumber + journal with its rollback
proof (the trigger shape), the replayed no-op (both before and after
a real reorder), every refusal form — including the pure-unknown
stale-screen shape (a complete active set plus one archived id) —
with the table and journal proven untouched, the corrupt-cell
fallback envelope on the command's own `list(All)` read (raw SQL
asserts the table was not renumbered), the empty/single boundaries,
the honest no-database answer, and the registration pin; plus a
logging pin for the `projects_reorder_failed` edge (the T-078 shape)
and a four-case mock suite in `commands.spec.ts` (splice + journal,
no-op, the archived refusal, empty/duplicate refusals). Bindings
regenerated: `projectsReorder`.

Scope note: the regenerated `src/bindings.ts` forced two frontend
mock-layer edits — `src/ipc/mock/commands.ts` (the D-141 loud-drift
rule: every bindings command needs a mock or `pnpm run typecheck`
fails) and `src/ipc/mode.spec.ts` (the explicit shipped-set pin). The
mock mirrors the command's semantics honestly (validation, splice,
renumber, bulk journal, no-op). Recorded for the orchestrator: the
brief's `src/**` freeze was overridden by its own green-checks
requirement; both edits are additive hunks in IPC infrastructure, not
the columns UI the parallel task owns.

### D-117 — OS-following light/dark theming: palette as CSS tokens in theme.css, a single `<html data-theme>` switch driven by useOsTheme(matchMedia), and a dependency-free WCAG contrast test — 2026-11-21

Problem: T-145 turns UI-021/NFR-019/NFR-022 (spec §11.5/§12.4) into the
app-wide theming base: light and dark themes follow the OS setting and
switch live without an app restart; text contrast ≥ WCAG AA 4.5:1 in
both themes; focus always visible; interactive controls ≥ 24 px. The
Tauri template's leftover styles in App.vue (unscoped `:root`
font/color block, logo drop-shadows, link colors) had to go. Open
questions: the switching mechanism (the card says "prefers-color-scheme
on standard Tailwind components"; Tailwind v4.3.5 is already wired
CSS-first via `@import 'tailwindcss'` + `@tailwindcss/vite`), where the
palette lives, and how the "automatic contrast check" runs without new
dependencies (package.json is frozen by the task scope).

Solution (T-145):

- **Palette.** `src/assets/theme.css` is the only place colors are
  defined: eight semantic tokens (`page`, `surface`, `ink`, `ink-muted`,
  `accent`, `danger`, `focus-ring`, `line`) in the `:root` light set and
  overridden 1:1 in `[data-theme='dark']` — zinc/gray neutrals with
  indigo accents and red-700/red-400 danger, every text pair ≥ 4.5:1
  with margin (the tightest checked text pair is accent on the dark
  surface at 4.99:1; ink on page is 16–18:1; the tightest non-text pair
  is line on the dark surface at 3.08:1). Both dark sets also declare
  `color-scheme` so native scrollbars/form controls follow, and
  index.html carries `<meta name="color-scheme" content="light dark">`.
- **Mechanism.** A single switch point: `useOsTheme()`
  (`src/assets/theme.ts`) applies the OS preference to
  `<html data-theme="light|dark">` at setup time and re-applies it on
  every matchMedia `(prefers-color-scheme: dark)` `change` event — an OS
  flip re-resolves the CSS variables live, no reload. A
  `@media (prefers-color-scheme: dark) { :root:not([data-theme='light'])
… }` fallback in theme.css carries the same dark values, so the FIRST
  paint on a dark OS is already dark (before the JS bundle runs) and the
  no-JS world themes too; the spec pins the fallback set equal to the
  attribute set (two hand-maintained dark sets would drift). This is an
  attribute-derived theme, not a bare CSS media-query swap, for three
  reasons: it is the one switch the whole app can share (tokens,
  Tailwind's `dark:` variant — re-keyed to the attribute via
  `@custom-variant dark` in main.css — and a future manual settings
  override), it is observable from JS so the no-restart criterion is
  testable (the spec stubs matchMedia and flips `matches`), and
  `light-dark()` was ruled out anyway (macOS 12 WKWebView, an NFR-001
  target, predates it). Deviation from the card's letter ("theme by
  prefers-color-scheme"): none in effect — the attribute is always
  derived from prefers-color-scheme; recorded here because the card
  reads like a media-query-only implementation. Known limit: the
  AppErrorAlert (T-143, outside this task's edit scope) keys its own
  dark block on the raw media query, not the attribute — identical
  while the attribute tracks the OS; if a persisted manual override ever
  lands, that component must move to the attribute first (follow-up for
  its owner).
- **Tailwind mapping.** main.css maps each token through
  `@theme inline --color-*: var(--*)`, giving standard utilities
  (`bg-page`, `text-ink`, `border-line`, …) that keep the `var()`
  reference in the generated class, so they re-resolve when the
  attribute flips; `@layer base` carries the app-wide norms: a global
  `:focus-visible` 2 px `--focus-ring` outline (UI-021; component styles
  may override the color, never remove it — AppErrorAlert's Copy button
  keeps its own), `min-height/min-width: 1.5rem` for
  button/[role='button']/summary/input/select/textarea (NFR-022's 24 px
  both axes; inline prose links stay exempt per WCAG 2.5.8's inline
  exception), a `100vh`+`100dvh` shell-min-height pair (dvh needs
  Safari 15.4; NFR-001 floors at macOS 12), system font stack,
  accent-colored links. App.vue now uses only these utilities — both
  Tauri template style blocks are deleted.
- **Contrast proof without dependencies.** `src/assets/theme.spec.ts`
  computes WCAG relative luminance/contrast in-test, first pinning the
  checker itself against reference pairs (21:1 black/white; #767676
  passes and #777777 fails 4.5:1 on white — a checker that cannot
  reproduce the boundary cannot be trusted), then parses the SHIPPED
  theme.css (comments stripped, blocks brace-matched, `--token: #rrggbb`
  literals collected — the stylesheet is the artifact the webview
  renders, so hardcoded spec-side copies would prove nothing) and
  asserts: every base text pair ≥ 4.5:1 and non-text boundaries
  (focus-ring, line) ≥ 3:1 in both themes; every token appears in at
  least one checked pair; the dark blocks override every token (no
  silent light fallback, the two dark sets equal); the AppErrorAlert's
  own surfaces (mirrored literals #fef2f2/#f3f4f6/#2c1a1a/#1f1f1f — the
  component is outside this task's edit scope) hold 4.5:1 against the
  matching theme's ink; main.css maps every token, imports theme.css
  (and main.ts imports main.css — the palette's only route into the
  app) and ships the focus and 24 px norms. The rendered loop is closed
  by one Playwright case (e2e/smoke.spec.ts): emulateMedia flips the OS
  color scheme light→dark→light and the shell's computed background
  follows live, no reload — real browser, real compiled CSS, real
  matchMedia. vitest's css handling stubs CSS imports (plain, `?raw`,
  `?inline` all come back empty) and the task scope froze
  vite.config/tsconfig, so the spec reads the two CSS files with
  node:fs — the one `@ts-expect-error` in the diff (src tsconfig has no
  node types; runtime is Node); once node types are added the directive
  goes unused and typecheck fails until it is removed (the cue).
- **Placement.** theme.ts lives under src/assets because T-145's
  parallel-window file scope allows `src/assets/**` (three sibling
  tasks own src-tauri, src/date, src/ipc/mock/events); moving it next to
  future composables later is a no-op (one import site: App.vue).

Review: two rounds — reports in `.opencode/reviews/t145-round*.md`
(owner-local). Round 1 (deepseek-v4.1-flash, changes requested): no
P0/P1; 2×P2, 7×P3 — all applied except one declined. P2-1 — the first
paint on a dark OS was light (the palette's light `:root` set applies
before the JS bundle runs; reviewer measured it in a built-app probe):
the `@media (prefers-color-scheme: dark) { :root:not([data-theme='light'])
}` fallback added, its token set pinned equal to the attribute block.
P2-2 — the main.css→theme.css→main.ts chain was unasserted (deleting
the `@import` kept all 43 tests green): structural pins added (imports
asserted) plus a rendered Playwright case (emulateMedia light→dark→
light, computed background follows live — the only test that would have
caught P2-1). P3s applied: `min-width: 1.5rem` joined the 24 px norm
(height-only before); a coverage test forces every token into at least
one contrast pair; the `@ts-expect-error` comment now says typecheck
fails until the directive is removed (not "self-deletes"); the alert
mirror check documents its query==attribute assumption; the shell uses
a `100vh`+`100dvh` base pair (min-h-dvh alone dropped on macOS
12.0–12.2 WKWebView); D-117's margin numbers corrected (4.99:1 accent/
dark-surface is the real binding pair). Declined: P3-5's "drop
@custom-variant dark, it has no consumer" — the variant is the
single-switch seam D-117 designs for the first theme-aware screen;
instead its pin carries an honest no-consumer-yet note. P3-4's
media-query-keyed AppErrorAlert is a follow-up for that component's
owner (recorded above), not editable in this window.
Round 2 (hy4-flash-free, no changes requested): no P0/P1/P2; 3×P3,
all applied. P3-1 — the AppErrorAlert mirror's `surfaces` data was
dead (the it.each rows duplicated it): the rows are now driven from the
single mirror table. P3-2 — the rendered e2e could not tell the JS
bridge from the CSS fallback (a dead bridge left it green): the live
case now asserts `html[data-theme]` at every step, and a second case
blocks all scripts on a dark OS and asserts the `:root` canvas is still
dark — the pre-JS fallback proven at the rendered level (the P2-1
regression guard). P3-3 — the fallback block's `color-scheme: dark`
joined the asserted declarations (native scrollbars/controls on the
dark first paint). The reviewer also re-verified the three-block
cascade under all four OS/attribute combinations, the D-117 contrast
numbers, and the alert's opacity-dimmed text (6.34:1 light / 7.98:1
dark — clear AA).

### D-118 — `logging::projects_list_failed`: the D-096 gap closed by its follow-up, and the forced `bindings.ts` doc-mirror — 2026-11-21

Problem: T-072 (D-096) shipped `projects_list` with a documented
scope exception — "no failure log edge yet" — deferring the §10
every-error rule to the task that owns the facade file. T-244 is that
follow-up, and it hit one question the brief did not anticipate: the
command's doc comment (which the edge's arrival made stale) is
mirrored verbatim into `src/bindings.ts` by tauri-specta on every
`cargo test`, and the `export_bindings_are_up_to_date` contract
(D-034, tightened T-013) keeps CI red until the regenerated file is
committed — while the T-244 brief froze `src/bindings.ts` ("the
command does not change — the signature is the same").

Solution: the `projects_list_failed` facade edge in `logging.rs` in
the exact `projects_reorder_failed` shape (error level, target
`eunomia::projects`, payload = the error's own §7.10 vocabulary:
code literal, params, `os_code`/`os_kind`/`os_message`), reached from
`projects_list` through the standard wrapper/`_impl` split so one
`.inspect_err` covers every failure branch (the `managed_db`
no-database fallback, the ordered store read, the blocking-pool join
failure); success logs nothing (reading is not an event — the
`activity_list` rationale, D-051/LOG-005). The pin is the T-078/T-080
real-subscriber facade test
(`the_projects_list_failure_line_carries_the_code_and_payload`), its
fixtures mirroring the two reachable classes the T-072 IPC tests
pinned (`NoDatabase`, `ProjectsCorrupt`); the success half stays
structural, as for every sibling edge.

- _The `bindings.ts` freeze vs the binding-sync contract._ The
  signature is indeed unchanged — only the stale "no failure log edge
  yet" doc paragraph was rewritten, and the exporter mirrored that
  paragraph into `bindings.ts` (4 insertions / 6 deletions, zero
  type or mock impact). D-034's red-until-committed contract wins
  over the brief's file freeze, the same way D-115's scope note
  recorded its own forced frontend edits: committing the mechanical
  mirror keeps CI green, and reverting it would re-open the gap the
  contract exists to close. Recorded for the orchestrator as the one
  deviation from the T-244 file scope.
- _No wrapper-call pin, on purpose (the round-1 review's P3-2)._ No
  command-level logging edge in this codebase has a textual
  wiring pin (checked: `activity_list`, the settings pair,
  `app_info`, `dialog_pick_folder`, `project_add_manual`, both
  exclusions, `projects_reorder`); the wiring argument is structural
  (one `inspect_err` on the single `_impl` every failure flows
  through) and the facade pin holds the line shape. Adding a pin for
  this one command alone would be the inconsistency.

### D-119 — The `exclusion.removed` journal member: the D-106 follow-up landed, the vocabulary's first beyond-LOG-001…004 members, and the legacy `project.changed` rows kept readable — 2026-11-21

Problem: T-245 closes D-106's tracked follow-up. The frozen D-051
vocabulary forced `exclusion_remove` to journal `project.changed`
with no entity — a row the future Activity window would render as a
project change attributable to no project (D-106's known cost,
recorded as "take the entry over BEFORE the Activity UI ships").
Growing the vocabulary raised three questions: (1) the member's form
and where it sits in an enum whose every member maps to a
LOG-001…004 category (the requirement does not name exclusion
operations); (2) whether the entry gains an `EntityType` of its own
or reuses a project shape; (3) where the removed path travels now
that the entity columns can carry the row's identity, and what
happens to rows already written in the T-078 shape.

Solution: two new members, one producer switch, and a compatibility
pin — the vocabulary's first consciously-beyond-requirement
extension.

- _The member: `Action::ExclusionRemoved` → `"exclusion.removed"`,
  placed after `ErrorReported` in a marked beyond-LOG-001…004
  group._ The `object.verb` lowercase shape is unchanged
  (`action_strings_are_dotted_and_unique` still pins it); the
  placement is honest history — every LOG-001…004 category keeps its
  documented members in their documented order, and the extension
  gets its own labeled section instead of hiding inside the
  requirement's list. The mapping oracle test grew a
  `beyond_requirement` list after the requirement's categories, so
  the enum's documented order stays the flattened category order and
  a future member added anywhere but the end fails the placement
  assert. `Action::ALL` is 22; the `exclusion.removed` spelling is
  pinned by a unit test that also rejects the near misses
  (`exclusion.added`, `exclusion.remove`, `exclusions.removed`) —
  the extension added exactly one member, not an open-ended family.
- _`EntityType::Exclusion` → `"exclusion"`, placed after `Project`._
  D-106's follow-up text called for the member "with its
  `EntityType`", and the placement follows the domain:
  `project_exclusions` is the projects-domain sibling table. The old
  D-106 reasoning — "the exclusion id must not ride a `project`
  entity column" — is exactly what the new member fixes: the id now
  rides an `exclusion` entity column, the honest kind for the row
  it names. `EntityType::ALL` is 9; the singular spelling is pinned
  (`exclusions` and friends rejected).
- _The path stays in `Details::File`; the entity carries the row id;
  `project_id` stays NULL._ The neighbors decide the payload:
  `BackupCreated` carries its path in `Details::File` (LOG-001…004's
  own "the project and path" convention, SEC-008 permits paths), and
  the T-078 entry already used exactly that detail — so the switch
  changes the verb and adds the entity, and rewrites nothing else.
  No `project_id`: no project row is involved, and `manual`
  exclusions have none (D-106's rule, unchanged). The shape stays
  info/user/ok — the D-051 unmirrored tracing shape, so
  `logging::action_logged` needed no change.
- _Compatibility: the legacy rows keep reading, proven against the
  real stored shape._ `project.changed` remains a member (its only
  current producer, `projects_reorder`, is untouched; the future
  update family will use the same member), so T-078-era rows parse
  exactly as written; the store integration test hand-INSERTs the
  exact legacy row (SQL literal, not a current producer's output)
  beside a new-shape row and reads both back — the read's backward
  compatibility is pinned against the historical bytes, immune to a
  producer-side drift. No migration was written, and D-106's
  "replaceable" premise no longer holds as stated: §6.4's delete is
  physical, so a legacy journal row is now the only remaining record
  of that removal — dropping it loses history, and migrating cannot
  recover the removed row's id (the T-078 entry never carried one).
  The follow-up therefore closes in two halves: the READ half is
  done (pinned above), while the RENDER half stays open for the
  Activity-UI task — until it maps the legacy shape at display time
  (`project.changed`, entity NULL, a `{"path":…}` detail ⇒ the
  "exclusion removed" label), those rows still render as a project
  change attributable to no object (D-106's known cost, bounded to
  pre-release data: the app is 0.1.0 and no shipped version holds
  T-078-era rows).
- _The command and its mirrors._ `exclusion_remove` writes the new
  member inside its one `Db::run` transaction (unchanged shape: the
  by-id list read, the typed refusal, the DELETE, the insert —
  D-106's composition), its doc comment now records the D-119 form
  and the legacy-read guarantee; the T-078 IPC test asserts the new
  verb, the `(exclusion, id)` entity, and the path detail. The
  frontend mock mirror follows the D-108 strict-coverage rule:
  `ACTION_MEMBERS` grows to 22 (the `activity_list` filter accepts
  the new member immediately), `exclusionRemove` journals the new
  shape with the entity columns, and the mock spec pins action,
  entity pair, and `project_id` NULL. `bindings.ts` regenerates
  mechanically (the command's doc comment is mirrored into it by the
  exporter; zero type or signature change — D-118's precedent).

Tests: store — the two new integration tests above plus the
`the_exclusion_members_round_trip_and_reject_near_misses` unit test
and the updated vocabulary oracles (`enums_round_trip_the_schema_
check_literals`, `the_action_vocabulary_covers_log_001_to_004_
member_by_member`); app — the T-078 journal asserts repointed (one
test, `exclusion_remove_deletes_the_row_and_journals_the_path_in_
one_transaction`); frontend — the mock journal test repointed
(`exclusionRemove journals exclusion.removed naming the removed row
(D-119)`). No other journal surface moved: the tracing mirror's
decision table is level/visibility/result-based and the new entry
keeps the unmirrored shape.

- _Round-1 review (deepseek-v4.1-flash: 0×P0/P1, 2×P2, 5×P3; verdict
  APPROVE)._ Applied: the compatibility bullet rewritten (the
  replaceability premise and the render half — P2-1; the producer
  wording — P3-4); the legacy test's timestamp moved to a far-past
  literal (clock-independent ordering, P3-1); the CHECK-literal
  oracle doc now says which columns the §6.2 DDL actually constrains
  (P3-2); `ACTION_MEMBERS` gained a length-and-uniqueness pin in its
  spec (P3-3). Declined with reason (P2-2): the stale
  "member-by-member onto LOG-001…004" sentence in ARCHITECTURE.md is
  real, but ARCHITECTURE.md is outside this task's frozen file
  scope — recorded here for the orchestrator (the D-118 deviation
  precedent). Recorded (P3-5): the one-way read policy is now
  reachable in production for the first time — an older build
  opening a database that holds an `exclusion.removed` row fails the
  whole `recent`/`list` read (the D-039/D-051 unknown-member policy,
  version skew surfaced by design); the accepted mitigation is the
  updater's no-rollback story (UPD-004), not a reader-side guess.

### D-120 — The D-086 addendum: non-UTF-8 paths refused at the add/scan boundaries; the lossy U+FFFD key deleted — 2026-11-21

Problem: D-086 left the non-UTF-8 corner as a lossy key (Unix path
bytes → U+FFFD via `to_string_lossy`) and explicitly deferred "reject
at the add/scan seam instead?" to the owner as a format-contract
decision. The escalation landed on 2026-11-20: variant 2 — reject
non-UTF-8 paths with a typed error at the input boundaries, so keys
and stored paths are always LOSSLESS; NFC + simple-fold apply to
valid UTF-8 only. The recorded cost of the lossy rule (two
byte-different names collapsing onto one UNIQUE key; a scan
classifying such a folder `known` against a lossy dev row) is what
the ruling removes.

Solution (T-243):

- _The identity seam refuses, typed._ `eunomia_fs::paths::canonicalize`
  now returns `CanonicalizeError::NotUtf8` (the struct became the enum
  `{Io, NotUtf8}`) when the CANONICAL FORM's text is not valid UTF-8 —
  the output is what becomes the stored TEXT path and the key's input,
  so the gate judges the output (a non-UTF-8 input resolving to a
  UTF-8 target — a UTF-8-named link to a non-UTF-8-named folder, the
  other way round — is ACCEPTED; the stored identity is the target's
  spelling). `path_key` returns the new `PathKeyError` (lossy path
  text for the log line only) instead of keying lossily;
  `canonical_path_key` folds the (compositionally unreachable) key
  refusal into the same `NotUtf8` variant — enforced, not assumed.
  The refusal is a TEXT fact (the D-090 split): no io error, no OS
  code, no `source()`.
- _The write-path machinery keeps D-092 byte-faithful._ D-092's
  contract is that the sync WRITE path carries the real bytes and
  only the staging NAME is lossy text (D-088) — a different surface
  from the identity seam. `resolve_target_path` and its helpers
  therefore moved to a private `canonicalize_raw` (dunce, io error
  raw, no UTF-8 gate); behavior and tests unchanged. The D-086
  addendum governs STORED identities, not resolved write targets.
- _The walk skips, classification never sees lossy._
  `SkipReason::NotUtf8` ("path is not valid UTF-8", `os_code()`
  None) is the walk's fourth reason: a path whose text is not valid
  UTF-8 — its own name or an ancestor's — is a skip, NOT a found
  repository, and the walk does not descend (every descendant's path
  inherits the invalid bytes). The gate runs before the name filters
  and at every depth INCLUDING the root: unlike the excluded-name
  policy, this is the TEXT boundary of the identity itself.
  `classify_repository`/`classify_entry` now answer `Result` with
  `PathKeyError` — unreachable for walk-driven events (the gate
  guarantees it, pinned end to end), honestly typed for hand-built
  inputs; no classified result can be lossy-known.
- _The app boundaries share one envelope._ `rejected_non_utf8_path`
  in `commands.rs` (the `rejected_field` shape): `E_VALIDATION`,
  field `path`, the sent spelling echoed (lossy — the only printable
  form, the caller's own input, SEC-008), reason "the folder name is
  not valid UTF-8: the project cannot be registered (rename the
  folder)". No new §10 code — the registry has none for this fact
  and the owner's card names the E_VALIDATION form. Consumers: the
  DIALOG boundary (`dialog_pick_folder_with` refuses the pick itself
  — nothing remembered, so no unrepresentable folder ever reaches
  the allowlist or the wire answer; an OS refusal stays
  `E_PROJECT_UNAVAILABLE`/`picked_folder_vanished`), the SEC-003
  gate (`validate_ui_path`: the typed `NotUtf8` branch, plus the
  D-100 lossy-answer-string heuristic kept for the wire spelling
  that contains U+FFFD), and `project_add_manual`
  (`repo_lookup_failed` maps `Canonicalize::NotUtf8` to the encoding
  envelope — an availability error would misname the fact — and the
  key/stored-text arms translate the fs refusal into the same
  envelope; the stored `projects.path` and `path_key` are now
  lossless by construction).
- _Format contract, no migration._ The key format's domain narrows
  to valid UTF-8 (D-086's format-stability warning named a change a
  DECISIONS event — this is it). The change landed BEFORE the first
  release: already-created dev rows with lossy keys are insignificant
  (delete and re-add), no stored-key migration ships.

Tests: fs — the lossy-key pin replaced by the refusal pins
(`non_utf8_path_text_is_refused_by_the_key_builder`:
`PathKeyError` contract, lossy diagnostic field, valid text keys as
before; `canonicalize_refuses_a_non_utf8_canonical_form`: a REAL
Linux fixture — UTF-8-named link to a `\xff`-named folder — refused
typed, raw spelling refused, composition refused, non-UTF-8 INPUT
resolving to a UTF-8 target accepted, valid sibling untouched);
`scan` — `a_non_utf8_repository_is_skipped_and_its_subtree_is_not_walked`
(own-name and ancestor-name fixtures, no Repository event, no
descent, sibling found) and the reason-text/os-code contract;
`scan_classify` — the pipeline pin
`a_non_utf8_repository_classifies_nothing_even_against_a_lossy_key`
(scan → classify over a registry holding the LOSSY spelling's key:
exactly one NotUtf8 skip, only the valid sibling classified, no
lossy-known identity; plus the honest typed Err for a hand-built
event). App — the IPC seam moved UP a boundary:
`a_non_utf8_folder_is_refused_at_the_dialog_and_add_boundaries`
(the pick through the real dialog seam is refused with the encoding
envelope, the allowlist does not grow, the hand-sent lossy spelling
gets the same answer at the add gate, the database stays empty, the
valid sibling picks and adds with its lossless canonical stored
path); the commands unit `a_non_utf8_root_lookup_failure_is_the_
encoding_validation_envelope`. The full existing `path_key` battery
stays green after the lossy branch's deletion; Linux-only fixtures
follow the `temp_files` EILSEQ posture (APFS/NTFS reject invalid-byte
names). `temp_repos` (fixture tooling) and `tests/scan_platform.rs`
were adjusted only for the new signatures — ASCII fixture paths,
unchanged behavior.

Round-1 review (deepseek-v4.1-flash; 0×P0, 0×P1, 2×P2, 5×P3,
APPROVE): all applied. P2-1 — docs/spec.md's skip-reason vocabulary
(PRJ-002's "Skipped" parenthetical, ALG-051/052) does not yet list
`not_utf8`; per the D-086 precedent spec edits ship as their own
`docs:` commits after the ruling, so this entry records the
deferral: the spec extension must land before the `scan_start` UI
consumes the reason vocabulary (a note to the same effect lives in
`paths.rs`'s module docs). P2-2 — the broken intra-doc link fixed
(`crate::scan::SkipReason::NotUtf8`) and the three new private-item
links turned into plain code spans. P3 — the typed SEC-003 arm now
pinned (a UTF-8-named alias onto the `repo\xff` fixture, real bytes
at the add gate); the reason wording now says "the folder path
contains a name…" (the gate judges the whole path); the dead `None`
arm removed (`picked_folder_vanished` takes the destructured
`&io::Error`); the unkeyable-probe and non-UTF-8-ROOT arms pinned by
unit tests; the over-broad classify sentence narrowed to "as a
repository event". Round 2 (hy4-flash-free; 0×P0/P1/P2, 1×P3-low,
APPROVE): the envelope's doc no longer claims the echoed value is
literally the caller's input at every seam — it is the lossy
rendering of the offending path (literally the input at the session
gate, the lossy spelling of the refused path at the others).

### D-121 — The dirty recalculation service: the commit seam fires on row changes, the capacity-1 bus is the debounce, chunked mini-snapshots hold the set-L budget, and the diff baseline is clean — 2026-11-21

Problem: T-101 turns DB-022 into the background service behind the
§7.10 `state://dirty` event (DB-021's rule, OUT-013, MAIN-028): hashes
of all active projects recalculated 150 ms after each committed
transaction, at startup, and after a sync. Six points were open: how
"after each committed transaction" reaches a background task from the
D-037 executor without re-triggering on its own reads; what the hash
input is now that ALG-030 has landed (D-064's seam sentence predates
it); what the event's diff is taken against on the first observation;
whether the §12.2 set-L budget (500 ms) holds over the T-100 snapshot;
and what the future sync pipeline consumes.

Solution: `src-tauri/src/dirty_recalc.rs`, the D-112 service shape.

- _The commit seam is a row-change predicate._ `Db` gains
  `commit_hook: Option<Arc<dyn Fn() + Send + Sync>>` +
  `set_commit_hook(&mut self)`; `attempt_once` fires it after a
  successful commit iff the guarded connection's `total_changes()`
  moved (read before `BEGIN`, read after `COMMIT`, guard still held).
  Read-only transactions — the service's own snapshot read included —
  and zero-row writes fire nothing (no self-retrigger loop); a
  busy-retry run fires exactly once (only its successful attempt
  commits); a rolled-back attempt never fires. The hook runs under
  the connection mutex, so its contract is "never block" — the
  installed closure only `try_send`s.
- _The bus is the debounce._ One `tauri::async_runtime` mpsc channel
  of capacity 1 (a tokio re-export — no new dependency): permits are
  triggers, a full channel swallows the next `try_send`, and that IS
  the coalescing. The single-consumer service task sleeps 150 ms on
  `Trigger::Commit`, drains collapsed permits, runs one pass, returns
  to `recv`; a commit landing during a pass leaves a fresh permit for
  the next window — no lost trigger, no queue-jump, no in-flight slot
  (the channel serializes). The startup trigger (`Immediate`, no
  debounce) is sent by `spawn_service` before its spawn, so it cannot
  race the task's first `recv`.
- _The hash input is the assembled managed text._ D-064 pinned the
  seam as "the assembled managed, marker excluded"; with ALG-030
  (D-065) landed that is no longer `build_body`'s output alone: scheme
  A wraps the body with the D-21 reading lines (the private
  `managed_block` join), B and C take the body alone (`assembly_b`'s
  own docs). `managed_text` mirrors the private helper byte-exactly
  (the public `build_content` panics on B by design), pinned by tests
  that strip `build_content`'s marker tail for A and C and check B
  against the body. The target path rides through
  `backup_name::target_file_name` (§9.4 phase 1 step 2's spelling).
- _The diff baseline is clean._ D-112's shape with the opposite
  implicit baseline: an unobserved project counts as clean, so the
  first pass reports exactly the dirty ones (OUT-013's startup
  warning — every dirty project IS a change; a fully-clean startup is
  silent), later contradictions arrive in both directions (dirty →
  clean after a sync included — MAIN-028's dot must clear), empty
  diffs are silent, and emit-then-record keeps delivery
  at-least-once. `recalc_and_emit` is also the after-sync seam §9.4
  phase 4 step 5 names: the pipeline awaits it once its DB-024 hash
  updates committed.
- _The set-L budget holds on chunked mini-snapshots._ `build_body` is
  per-project and rescans the snapshot's whole assignment list every
  call — its own docs assign the multi-project batching to the caller,
  and this service is that caller. Measured on set L (release, this
  8-core box under unrelated load ≈5): naive per-project calls cost
  ~725 ms compute (500 × 100,000 assignment comparisons — the
  quadratic the budget cannot hold); singleton mini-snapshots per
  project still cost ~600 ms (cloning and re-validating the
  1,100-item library 500 times — four million short-lived
  allocations); **chunks of 25 projects** bring the sequential compute
  to ~210 ms and the whole pass — one read transaction (~310 ms, the
  T-100 snapshot read + `projects::list`, the store's own §12.2 share)
  plus the 20 chunks spread over parallel `spawn_blocking` tasks plus
  the emit — to **~430–545 ms release / ~2.0 s debug**, with the
  verdict order deterministic (handles awaited in chunk order). The
  scoping is body-equivalent by construction: filtering assignments by
  project id is exactly what `selected_variants` does inside. The
  release-suite review catch worth keeping: the first chunked version
  dropped the last partial chunk (`get(start..start+25)` past the
  slice end) — every small fixture is one partial chunk, so the suite
  went red the moment it ran in release; the end is clamped now. The
  test asserts the exact verdict list (500 dirty, id-for-id against
  the sequential reference) and keeps a 10 s regression tripwire
  instead of asserting the 500 ms figure (CI runners vary; the budget
  row stays a reference-machine release measurement — the numbers
  above travel with this entry).
- _Deliberate logging gap._ This window's parallel task owns
  `logging.rs`, so the `dirty_check_failed`/`dirty_emit_failed`
  facade lines (the `availability_*` shape) could not be added; the
  failure paths degrade safely without them (a failed pass records
  nothing, the next trigger re-runs it), and the lines are a recorded
  follow-up for the first task allowed to edit `logging.rs`.

Testing: 19 unit tests in the module (the diff semantics over the
clean baseline — first-observation both halves, both directions,
unchanged silence, prune-then-return, order; the verdict computation —
never-synced both spellings, the no-assignments exception against
reading lines, stored-matching clean, stored-differing dirty, the
extra-file settings change, snapshot order, the crossing-wires scoping
proof; the managed mirror against `build_content` for A/C and the body
for B; the wire-name literal) + 9 mock-runtime tests in
`dirty_recalc_tests.rs` (the card's criteria end to end —
edit→dirty→event, sync-эталон→clean→event, hidden-instruction edit
silent while the pass counter proves the pass ran; the startup spawn;
the debounce floor and burst coalescing; read-only discrimination;
the executor seam's fire matrix — write fires once, read and zero-row
write and rollback stay silent; the honest no-database answer; the
set-L budget; and the `lib.rs` wiring source-scan pin).

Round-1 review addendum (D-121, same task): findings fixed here —
the stale-record guard (an older overlapping pass can no longer rewind
the cache; its event may still carry a superseded change, at-least-once,
next pass corrects — the docs' earlier "duplicates are benign" claim
sharpened to this), the clean-hash pins for scheme B, scheme C and
enabled extras (the `extra_file_lines` argument order and the C target
spelling were unpinned by any passing case), the reasoned `expect` over
`unwrap_or_default` in the chunk build, the partial-chunk order test,
and the read-split print in the L test (the reviewer's "duplicated
projects::list costs 341 ms" diagnosis is refuted by it: the list is
~1.6 ms; the ~330 ms lives inside the store's `build_snapshot::read`,
the T-100 §12.2 share). Findings routed to their owning tasks (fix
sites outside this task's window): `commands.rs`'s `dirty_verdict`
seam still answers the constant `false` — its own doc names this task
as the taker, but the file belongs to the parallel window (the UI's
initial dirty state must come from there or a full-set read API; a
one-shot startup diff cannot bootstrap a late subscriber); the
recovery restore/recreate path reopens a `Db` with no commit hook and
no service spawn (the same gap availability has there — the recovery
commands' task re-arms both); the mid-sync recalculation cadence
(§9.4 phase 3's per-project hash transactions will each trigger a
debounced ~0.5 s pass — the pipeline task should pause or coalesce the
bus during a run); and the logging facade lines remain the recorded
follow-up. The DB-022 payload reading is made explicit: the spec's
"list of dirty projects" arrives as the first event's full list (the
clean baseline makes every dirty project a first change) and as diffs
afterwards — D-112's event shape, now stated rather than implied.

### D-122 — Backup retention as a store service: the ok-only window per "project—file" pair, delete-first marking, and best-effort FS failures reported not raised — 2026-11-21

Problem: T-114 lands BKP-003/BKP-006 — after a run finishes, at
most N=backup.retention copies are kept per "project and file" pair,
the oldest extras deleted (§6.4: file deleted → record `pruned`).
Open questions: where the service lives and how `backup.retention`
reaches it; what "only if the new copies are checked" (BKP-006)
means against a DDL with no `unchecked` status; which N are "the
newest" and whether anything protects the latest copy at keep 0;
and what a filesystem failure mid-prune does to the record (mark
anyway, or not), given the marks must share one transaction while
file deletions cannot roll back.

Solution: `eunomia_store::backup_retention` — a bookkeeping service
over the existing v1 `backup_records` table (no schema change, the
`library_versions` shape), one entry point `apply(conn, data_dir,
keep)`.

- _The limit is a parameter (the LogLimits/D-102 precedent)._ §6.5's
  1–200 range has one owner — the settings repository (D-039); the
  command layer reads `backup.retention` and passes the value. A
  zero or negative `keep` (a caller bug, not a user setting) is
  clamped to 1 inside `apply`: the newest copy of a pair is never
  deleted — ALG-072's unconditional-latest guard, so a buggy zero
  cannot wipe a project's whole backup history. The spec names no
  "latest is protected" rule for backups; the clamp is this entry's
  ruling, adopted from the revisions precedent.
- _BKP-006 = the ok-only set._ §9.4 phase 2 inserts a record only
  after the BKP-010 size+SHA-256 check, so `ok` IS the "checked"
  marking and the other three statuses are history of gone copies.
  The retention window is exactly `status='ok'`: terminal rows
  (`pruned`, `missing`, `discarded`) consume no keep slot and are
  never rewritten. The run-level half of BKP-006 — retention runs
  only after the run's new copies were all checked — is the
  pipeline's calling contract (phase 4 is unreachable past an
  `E_BACKUP_FAILED` abort); the record-level half is the ok filter.
- _"Newest" by `created_at DESC, id DESC`._ DB-002 TEXT makes the
  lexicographic order chronological; UUIDv7 TEXT ids (D-031) break
  same-millisecond ties by true creation order. One window-function
  query (ROW_NUMBER per `(project_id, source_path)` partition)
  selects the prunable rows.
- _Delete-first, mark-after._ §6.4 ties `pruned` to the physical
  deletion, and the order is the self-healing one: a crash or
  rollback between removal and mark leaves an `ok` record without a
  file, which the next run converges (removal NotFound = success,
  D-091's `remove_copy` ruling) and marks `pruned`. The mirror order
  would leave a `pruned` record over a live file — a lying status
  the restore screen must never see.
- _FS failure = report, not error; record stays `ok`._ A removal
  that fails after the §9.5 transient-hold retries (`retry_transient_
hold`, now pub(crate) in `storage`) skips its record, lands in
  `RetentionOutcome::failures` with the record id, the stored cell
  and a typed reason, and the walk continues — the daily-copy
  prune's best-effort posture: phase 4 must not fail a successful
  sync over one held file, and the next run retries the skip
  (idempotent convergence). The marks ride the caller's connection
  (D-037/D-051): a DB failure mid-mark returns `StorageError::Sql`,
  the caller's rollback undoes every mark, and the already-removed
  files stay removed — converged by the next run exactly like the
  crash window above.
- _The deletion guard._ `backup_path` is D-091's relative layout
  string (`backups/<project_id>__<slug>/<file>.<UTC>.<hash8>.bak`),
  joined under the caller's `data_dir`. A destructive call does not
  trust the cell: it must be relative, rooted at `backups/`, free of
  `..` components and `.bak`-named (mid-path `.` is normalized away
  by `Path::components` and needs no rule), else the record is
  reported with `PathRejected` and never deleted — the daily-copy
  retention's strict-name discipline applied to deletion. The path
  convention (relative, not absolute) is this entry's contract for
  the future record-writer task; the UNIQUE column stores the
  data-dir-relative spelling so a relocated data directory keeps its
  records valid.
- _No store→fs edge._ `eunomia-fs`'s `remove_copy` is private and
  single-line; `storage.rs` already deletes database-backup files
  with `std::fs` best-effort (`prune_daily_backups`), so the copy
  removal uses the same posture in-store — D-017's edge economy:
  the first task that truly needs the other crate's types adds the
  edge, and one `remove_file` is not it.

Tests: 3 unit tests in the module (the guard's accept shape; the
rejection vocabulary incl. the std-normalization note on `.`; the
failure `Display` facts) and 11 integration tests in
`tests/backup_retention.rs` on a real `open_db`+`run_migrations`
database with real files, the layout paths built through the real
core builders (`backup_relative_path` — the 19-char BKP-001 stamp,
the cleaned slug): the BKP-003 criterion as a process (five syncs,
each followed by the phase-4 retention at N=3 → after every run
exactly the newest `min(n,3)` alive; after the fifth the three
newest, files and `ok` records, the two oldest deleted and
`pruned`), keep 1, the 0/negative clamp over both a fresh and an
already-pruned state, the same-millisecond id tie-break (real
`new_id` UUIDv7s, D-031), per-pair isolation (two projects; two
files in one project), terminal statuses (`pruned`, `missing`,
`discarded`) invisible and untouched, the vanished copy converging
(one real removal + one NotFound in one pass) plus the second full
no-op run, the refused removal (a directory squatting on the name:
skipped, reported, others pruned, next run converging after the
obstacle goes), hostile cells (`../` escape with a live file
outside the tree, absolute path) reported and never deleted, the
trigger-injected mark failure rolling every status back while the
removed files stay removed (then converging), and the empty-table /
within-limit no-ops.

Round-1 review (deepseek-v4.1-flash) — REQUEST CHANGES, no P0/P1,
four P2, eight P3; all processed:

- P2-1 taken (the round's substance): the docs claimed the tests
  pin the guard's accepted shape against the real builder output
  while the fixtures hand-spelled the layout (and the stamp was 18
  chars, not BKP-001's 19). The fixtures now build every layout
  path through `eunomia_core::backup_name::backup_relative_path`
  (the store→core edge D-105 already wired), with distinct hex
  digests per copy so the builders' `hash8` component differs too;
  the claims are now true.
- P2-2 taken: "the next run converges" was stated unconditionally —
  it holds only while the record still sits past the window. The
  module docs now state the residual (a limit raised before the
  next run leaves an `ok` record over a missing file, §10's
  restore/`missing` detection being the designated closer — no v1
  producer of `missing` exists yet).
- P2-3 taken: `mark_pruned` gained the structural `AND status =
'ok'` guard and counts only changed rows, making the
  "terminal rows are never rewritten" promise structural instead
  of snapshot-dependent (defensive today under the single-writer
  executor, D-037; §10's `missing` detection and BKP-005's sweep
  are future second writers).
- P2-4 taken: `PathRejected::Escape`'s `.` half is unreachable
  (`Path::components` normalizes mid-path `.` away; a leading `.`
  is `NotUnderBackupsRoot`) — the docs and the arm's comment now
  say so instead of implying a live rule.
- P3 taken (1, 2, 3, 4, 5, 7): the `discarded` status is now
  seeded and pinned untouched in the terminal-status test; the
  `created_at DESC, id DESC` tie-break is exercised (two real
  UUIDv7 ids sharing one stamp — the later-generated id keeps the
  slot); the vanished-copy fixture now writes copy 1's file so one
  pass exercises a real removal AND a NotFound convergence (its
  comment matches again); the criterion test applies retention
  after EACH of the five syncs (the card's process, not one batch
  apply); the guard's docs name the symlink boundary (the guard
  judges the stored cell, not the filesystem — a symlink inside a
  writable data directory is outside the machine-written-cell
  threat model); and the `source_path`-verbatim pair-key contract
  is stated in the module docs for the future record writer.
- P3-6 declined (index over `(status, project_id, source_path,
created_at)`): a schema/migration change outside this task's
  boundary — §6.4 keeps the history as status, the volumes are
  §12.2-bounded, and the module docs now name the index as the
  schema task for the day the full-window scan stops being cheap.
  The doc half of the finding (noting the intentional row growth)
  was taken.
- P3-8 declined (Unix errno meanings in `retry_transient_hold`'s
  Windows-only os-code matches): the helper is pre-existing
  D-038 machinery shared with the migration/daily-copy paths;
  changing its retry semantics belongs to its owner-task, and the
  worst case here is a 1.3 s pause before an already-failing
  removal is reported.

Round-2 review (hy4-flash-free) — APPROVE, no P0/P1/P2; two P3
(low confidence, both flagged non-blocking by the reviewer):

- P3-1 taken: the module docs described `apply` in the present
  tense as "called by the sync pipeline" while no production caller
  exists yet (the caller is the parallel src-tauri pipeline task) —
  the docs now say the service is FOR the pipeline to call and name
  the boundary.
- P3-2 declined (clamping `keep` to §6.5's upper bound 200 inside
  the service): the range has exactly one owner — the settings
  repository (D-039) — and D-102's parameter posture exists
  precisely so the service does not duplicate it; a hardcoded 200
  here would silently diverge the day the spec range changes, and
  an over-200 value only keeps more copies (harmless direction).

### D-124 — AGENTS.md gains Project Rules distilled from this file's recurring failure modes — 2026-11-21

Problem: the problems recorded in DECISIONS.md are strongly recurring —
spec prose open to several readings, unverified assumptions about
library APIs and platform behavior, false-green tests, platform blind
spots from the Linux dev machine, user content reaching diagnostic
surfaces, exit codes masked by pipes, guards protecting nothing — but
each was caught only after the fact (review rounds, CI, post-merge
fixes); the agent-facing instructions encoded nothing preventive, so
every new task re-faced the same classes from scratch.

Solution: AGENTS.md section 3 ("Project Rules") — eleven rules mapped
one-to-one onto the failure modes above: resolve spec ambiguity before
coding, with the chosen reading recorded here; verify facts against the
installed sources and cite file/line; platform facts behind cfg gates,
test expectations built with project helpers; every test falsifiable —
independent oracles plus a mutation check; green means exit code
(`$?`/`PIPESTATUS`, no `cmd | tail/grep &&` chains); stated linear
complexity for untrusted text/path input; a guard lives across its whole
protected region; user content never reaches diagnostics; dependencies
and unsafe are owner rulings with cargo-deny green; frozen zones of
parallel tasks stay untouched, forced deviations recorded here; docs
must not lie, and what automation cannot prove goes to the manual smoke
checklist. The rules deliberately duplicate nothing governed by the
global AGENTS.md (the DECISIONS duty, one commit per task, check
gates); each rule is phrased to be checkable at review. This entry is
the traceable link the section cites back to.

### D-123 — The global `sync_running` flag: take-once atomic + RAII guard as managed state, and the SYS-005 gate as a check (not a hold) in front of the three project-mutation commands — 2026-11-21

Problem: T-105 turns SYS-004/MAIN-030 (§5.4) into code: parallel
syncs are impossible — the global `sync_running` flag is taken
atomically, a second start answers `E_SYNC_RUNNING` (§9.4 phase 0
step 1, released at phase 4 step 5) — and SYS-005 makes the
project-changing commands answer the same typed §10 refusal while a
sync runs. Four points were open: who owns the flag and how it
survives a panicking or cancelled run; whether the gated commands
take the flag or only read it; where the gate sits in each command's
act order; and what the refusal's §7.10 envelope carries.

Solution: `src-tauri/src/sync_running.rs` + the gate in
`commands.rs`.

- _The flag is the availability slot, pointed at exclusion._
  `SyncRunning` (managed state, the D-112 service shape) wraps one
  `Arc<SyncFlag { AtomicBool }>`; `try_acquire` is a
  `compare_exchange(false, true, AcqRel, Acquire)` — the atomicity
  IS the mutual exclusion — and returns the RAII `SyncGuard`, whose
  `Drop` stores `false` with `Release`. The guard owns an `Arc`
  (`Send + Sync` by construction), so the future pipeline holds it
  across `.await`s for the whole run; a panic, an early `?` or a
  cancelled task unwinds through the same drop, so the flag can
  never stay "running" behind a dead pipeline. `is_running()` is
  the read-side status; `try_acquire` is the tested-today/
  consumed-later seam for T-112+ (the `errors.rs` posture, with
  `#[allow(dead_code)]`) — no sync writer exists yet.
- _Typed refusal, envelope at the boundary._ `try_acquire` answers
  `Result<SyncGuard, AlreadyRunning>` — a marker type, not a wire
  value; `commands::begin_sync(app)` is the app-layer start seam
  (§9.4 phase 0 step 1) the future `sync_run` command calls, and
  `commands::sync_running_refusal()` renders the §7.10 envelope
  (`AppError::new(SyncRunning)`) — the one mapping from the marker,
  shared by the seam and the gate so the two answers cannot drift.
  Params empty and details `None`, on purpose: the §10 row names no
  path, project or field for this situation, and the refusal is a
  policy answer, not an OS failure. `pub(crate)` so the IPC tests
  pin seam and wire form to each other.
- _The gate is a check, not a hold._ Only the sync pipeline ever
  holds the guard: the flag means "a sync is running", so a mutation
  holding it would make `is_running` lie and block a sync START
  behind an edit. `reject_during_sync` loads the flag and answers
  the refusal when it is up; a command that slips in just before a
  sync starts commits cleanly on either side of the snapshot — the
  D-037 executor serializes it against the snapshot read on the one
  connection, and SYS-005's "stays dirty" half is the dirty-recalc
  service's (D-121).
- _The guarded set._ Today exactly the three mutation commands that
  exist: `project_add_manual`, `exclusion_remove`, `projects_reorder`
  — reads (`projects_list`, `exclusions_list`, settings, history)
  are never gated (SYS-005 names the changing commands only); the
  spec's own `project_update`/`project_remove`/`project_relink`/
  `backup_restore` join the gate as those commands land. The T-074
  doc claim "the guard does not apply" is superseded here.
- _Order: the mode gate is the first act._ Before SEC-003 for the
  add flow and before `managed_db` for the other two: during a sync
  the refusal is the answer whatever the input is; SEC-003's
  invariant (a foreign folder never reaches the `.git` check, the
  database, the probes) holds either way — pinned by a test where a
  never-picked path meets `E_SYNC_RUNNING`, not `E_VALIDATION`.
- _Missing state is the `managed_session` answer._ The flag is
  managed by `setup` before anything that could fail (needed in
  every mode: the gate runs before the honest recovery-mode
  `NoDatabase` answer); its absence is an uninitialized-app bug —
  the D-037 fallback with a `SyncRunningMissing` diagnostic, never a
  panic and never a silently-permissive gate. The healthy-app and
  no-database IPC fixtures now manage the state (they model the app
  `setup` builds); recovery mode is not reinterpreted — with no
  database no sync can run, the flag stays down, the commands keep
  their `NoDatabase` answers.
- _Envelope rendering stays in the existing logging edges._ The
  refusal flows through each command's `inspect_err` edge
  (`project_add_failed`/`exclusion_remove_failed`/
  `projects_reorder_failed`); their facade docs now list
  `E_SYNC_RUNNING` among the reachable §10 answers.

Tests: 3 unit tests in `sync_running.rs` (take-once/hold/release;
the panic unwind frees the flag via `catch_unwind`; 8 parallel
starts elect exactly one winner — deterministic two-barrier overlap,
no sleep) and 7 IPC tests in `ipc_tests.rs` (the card's criterion
through the `begin_sync` production seam — first `Ok`, the second
start's refusal serialized against a spec-literal JSON, so the
marker→envelope mapping cannot drift; per-command refusal under an
externally-held `begin_sync` guard with the read side proven alive
(`exclusions_list` and `projects_list`), the tables proven
untouched, and the same call converging after the drop; the add
flow's gate-before-SEC-003 ordering (a never-picked path meets
`E_SYNC_RUNNING`, not `E_VALIDATION`); the gate's honest
missing-state answer (`SyncRunningMissing`) with the database
present, pinning the gate-before-`managed_db` ordering; and a
`lib.rs` manage-line marker pin — without it, dropping the setup
`manage` left every suite green). `src/bindings.ts` re-exported
(doc-mirror only, D-034/D-118 — no command signature changed).

Round-1 review (deepseek-v4.1-flash) — REQUEST CHANGES, no P0/P1,
three P2, two P3; all processed:

- P2-1 taken (missing-state branch and setup wiring untested): the
  `SyncRunningMissing` fallback had zero coverage and a deleted
  `manage` line in `lib.rs` stayed green. Added the honest-fallback
  test (healthy `Db`, no flag state — the `E_DB_LOCKED` can then
  only come from the gate, pinning the ordering) and the
  exactly-once `lib.rs` marker pin.
- P2-2 taken (the criterion was not pinned structurally): the test
  observed the seam's `is_err()` and separately compared the
  constructor's output — no production path mapped
  `AlreadyRunning` to the envelope. Added
  `commands::begin_sync(app) -> Result<SyncGuard, AppError>` — §9.4
  phase 0 step 1's app-layer seam for the future `sync_run`
  command, `#[allow(dead_code)]` like `try_acquire`, mapping the
  marker through `sync_running_refusal` (no `From`, the errors.rs
  rule) — and the criterion now drives it twice, the second
  refusal asserted against the spec literal. The per-command
  helpers hold the flag through the same seam.
- P2-3 taken (the parallel race was sleep-based and could flake):
  rewritten with two `Barrier`s — everyone attempts between them,
  the winner cannot drop before all have attempted. The rewrite
  immediately caught its own first draft reproducing the
  availability slot's round-1 trap verbatim: an `if let Ok(_guard)`
  body scope drops the winner's guard before the barrier (8
  winners); the guard binding now lives to an explicit `drop`
  after the barrier (project rule 7 — a guard protects its whole
  region).
- P3-1 taken (`projects_list` not proven alive under the flag):
  added the read-half twin of the `exclusions_list` pin.
- P3-2 taken (the serialization claim rested on an unpinned T-112
  property): the module docs now state the requirement explicitly —
  the phase-0 snapshot must be ONE `Db::run` read transaction on
  the shared connection, or the "either side of the snapshot"
  argument voids; recorded here as the T-112 calling contract.

Round-2 review (hy4-flash-free) — APPROVE, no P0/P1/P2/P3. All five
round-1 fixes verified in the committed tree (including the
`strip_comments` coverage of the marker pin and the happens-before
of the barrier join); the criterion's spec literal corroborated
against the pre-existing `errors.rs` pins; the check-not-hold
serialization argument verified sound against D-037's single
`Arc<Mutex<Connection>>`. One non-blocking low-confidence
observation (a future refactor could extract the spec-literal JSON
into one shared fixture to make the two-step test oracle's
independence explicit in code) — noted for the T-112 task, no
change now: the chain is sound while the suites live in one binary.
The round-2 reviewer artifact (`REVIEW-T-105-round2.md`, written
into the worktree despite the read-only brief) was moved into the
round-2 report file and deleted from the tree.

### D-125 — Owner rulings on the html-pages prototype audit — 2026-11-21

Problem: the audit of the `html-pages` prototype (manual main-window mock-up,
43 commits) against `docs/spec.md` 1.1 found 41 gaps — 13 beyond the spec
(A1–A13), 23 refinements (B1–B23), and 5 contradictions (C1–C5). Several
items (A1, B1, B20, C1–C3) were open model or UX decisions that the spec
could not resolve on its own. Source:
`.opencode/audits/html-pages-vs-spec-2026-11-21.md` (read-only audit; the
reverse gap D — the spec describing more than the mock-up — is normal for a
sketch and was ruled out of scope).

Solution: the owner ruled on 2026-11-21; all approved items were folded into
`docs/spec.md` 1.2 as extensions of existing requirement IDs (no new ID
prefixes introduced):

- **A1 — approved (model change):** a matrix cell may carry per-project
  custom text, bypassing the variant model. `project_instructions` now has
  a nullable `variant_id` plus `custom_text` with an exactly-one-target
  CHECK, and `instruction_id` a direct FK to `library_items` (the composite
  FK is skipped when `variant_id` is NULL, so the direct FK restores the
  referential guarantee; round-1 review P2-1) (§6.2); §6.9 gained the
  custom-text validation row; MAIN-007/LIB-018 rewritten from the
  fork-variant flow (a variant named after the project, command
  `variant_fork_for_project` — removed) to a custom-text editor
  (`assignment_set_custom`); cell behavior (select disabled and showing
  "Custom", checkbox active, edit/delete icons, confirmed deletion, bulk
  operations do not touch custom cells) in §7.3/§7.4/§11.2; the build uses
  custom text as the cell's text (§9.2). Chosen readings recorded (round-1
  review P1-2): deleting custom text (or clearing it via
  `assignment_set_custom`) assigns the main variant back to the cell — the
  prototype restored the previously selected variant from a UI-only cache,
  which the schema does not keep; and the disabled select of a custom cell
  shows "Custom", never a variant name.
- **A2–A11, A13 — approved:** per-cell "changed" state (§11.2); select-all
  checkbox + selection chip + contextual "Assign to projects…" (§7.4);
  hidden-instructions counter on "Show hidden" and search overriding
  collapse (§7.4); full toolbar composition (§11.1); "Unsynced changes"
  dialog from the dirty counter with per-project target files and Run sync
  (§7.4); scan-modal additions — active depth/exclusions line, Scan again,
  Add selected gating, skipped-reason hints (§7.1); three extra
  confirmations — restore archived project, delete excluded path, delete
  custom text (§11.4); the update block living in About (§7.9); entry
  points to Archived/Excluded from Settings (§7.2/§11.1); the settings
  pencil in the column header (§7.1, PRJ-008 entry point). Chosen reading
  for the "changed" marker recorded (round-1 review P1-1/P1-3): the marker
  is a session UI hint against a baseline captured at window open and
  recaptured after every sync run — not persisted state; the dirty flag
  stays authoritative and hash-based (DB-021) and the two may disagree
  (an identical-text variant switch marks a cell without dirtying the
  project; a reordered library dirties the project without marking a cell).
  The audit candidate's "dirty column = at least one changed cell" was
  rejected as a redefinition of DB-021.
- **A12 — declined:** the pickers keep the named `Main`; the `Main` naming
  rule in §6.9 is unchanged (§6.9 gained only A1's custom-text row). The
  prototype's pseudo-variant "Default" is a demo deviation.
- **B1–B23 — approved:** cell = checkbox + inline select, Enter opens the
  picker (MAIN-001/§11.2 rewrite); hide/restore via the row eye button with
  the confirmation text and the dashed "hidden" chip; heading-row tools
  (+/pencil/trash, level-6 limit) and the fixed heading-deletion
  confirmation text (UI-010); the four drag-and-drop outcomes; tooltip
  mechanics (reusable, clamped, keyboard-triggered, none for empty
  instructions); shortcut scope under open dialogs; roving-focus mechanics;
  editor structure (separate name fields, live validation, ARIA tabs);
  variant-row actions and the "Variant N" default name; project-settings
  details (live label preview, counters, scheme-conditional blocks, copy
  feedback); archive type-to-confirm mechanics and Archived/Excluded row
  content with focus retention; scan re-run; sync run presentation (outcome
  badge, counter chips, E_* hints, per-project time updates); settings
  without Save; Activity taxonomy; Help match counter; About details;
  MAIN-028 rewritten to icon + visible short text with the short sync time
  as text and the full name in tooltip/sr-only; sync-bar composition;
  sr-only "(empty instruction)"; cell accessible names "instruction —
  project: state" including "(changed)".
- **C1 — approved:** dropping a section on a heading (or the library
  header) auto-shifts the whole branch to target level + 1 (or 2) within
  2–6; "Change level" / "Shift branch level ±1" remain for manual cases
  (LIB-032, ALG-062 rewritten; consistent with ALG-060/061).
- **C2 — approved (simplification):** the main variant cannot be deleted —
  the button is disabled with "Make another variant main first" (LIB-024,
  §6.4). §3.2 D-12 keeps its original wording with a change note pointing
  here (history not silently rewritten).
- **C3 — approved (simplification):** deleting a variant moves its
  assignments to the main variant; the confirmation names the number of
  affected projects; the per-project replacement dialog is gone (LIB-023,
  DB-012, §6.4).
- **C4 — declined:** duplicate project names stay allowed with a warning
  (§6.9 is right); the prototype's blocking inline error is a demo
  deviation.
- **C5 — declined:** two buttons — "Add project" (system picker) and "Scan
  folder" — per UI-012/S1/S2; the prototype's single button that jumps
  straight into a simulated scan is a picker simulation.

Spec version bumped 1.1 → 1.2; the header names this ruling. Affected spec
IDs: MAIN-001, MAIN-006…009, MAIN-012…014, MAIN-024…028, MAIN-007/LIB-018,
LIB-002/009…016, LIB-005…007, LIB-017…025, LIB-030…034, ALG-020, ALG-062,
PRJ-002/005…006, PRJ-008…018, PRJ-021, DEF-001…004, UI-001…004, UI-010,
UPD-001/002/005, HLP-001…004, LOG UI row, DB-012, DB-016, §2 Assignment,
§6.2 DDL, §6.4, §6.9, §7.5 OUT-020, §7.10 commands, §8 S5, §9.2, §11.2,
§11.3, §15.1 unit row, §16 M3, §3.2 D-12 (change note). UI-012 was checked
and left unchanged (C5 declined).

Round-1 review (deepseek-v4.1-flash, docs-only; report
`.opencode/reviews/docs-spec-sync-round1.md`) — no P0, 3 P1, 8 P2, 6 P3;
all findings adjudicated and taken:

- P1-1/P1-3 taken as a rewording plus the recorded session-baseline
  reading (A2 above); the dirty flag stays hash-based (DB-021).
- P1-2 taken with the main-variant restore reading (A1 above).
- P2-1…P2-8 all taken: a direct FK on `instruction_id` (§6.2); the §11.2
  Main/Empty rows show the inline select; the "+" custom-text affordance
  named in MAIN-001/§11.2; this entry's A12 wording and the omitted B4
  fixed; DB-016 names custom text; "Unsynced changes" added to UI-002;
  MAIN-024's dirty wording follows DB-021 instead of overstating it.
- P3-1/P3-2/P3-3/P3-5/P3-6 taken: the Escape target is the cell's
  checkbox; S5 is listed under M3 as well; the custom-text build path is
  named in the §15.1 unit row; an OUT-020 row was added to §7.5; the two
  "Check for updates" buttons are disambiguated.
- P3-4 taken by making the "(changed)" suffix visible cell text (a
  non-color cue), not only an accessible-name suffix.

### D-126 — NUL rejected in every user-facing name field: the D-076 validators-wide promise, kept positionally — 2026-11-21

Problem: D-076 left an open item — the three §6.9 library name rows
and T-049's project name carry no NUL rule, so a name containing
`U+0000` validates while SQLite's `length()` (verified against SQLite
directly in D-076) counts code points only up to the first NUL: a
NUL-FIRST name slips past the validator's length check and the store's
own `length(trim(…))` CHECK then measures it as 0, failing the insert
with a raw database error instead of a field error. The sharper
consequence D-076's round-1 review added: the heading title is the one
name written into the target file verbatim (`## <title>` via the
body's heading block), so a NUL there puts a NUL byte into
`AGENTS.md`/`CLAUDE.md` — git then treats the file as binary (its
heuristic reads a blob's leading bytes, and the managed heading area
sits in them). T-252
(the 2026-11-21 DECISIONS audit, item 7) closes the promised
validators-wide gap.

Solution — the audit's actual list, not the card's summary one:

- **New rejections**, each a typed per-field error in the house form
  `"<field> contains NUL at index {index}"` (the `VariantTextNul`
  shape): `InstructionTitleNul`, `VariantNameNul`, `HeadingTitleNul`
  (`library_validators`) and `NameNul` (`project_validators`,
  `validate_name`). The project name is in scope because D-076 itself
  names "T-049's project fields" as carrying the same latent gap.
- **Already covered, pinned only**: the project LABEL — `U+0000` is a
  control character, so D-075's `LabelControlCharacter` rejection
  already refuses it wherever it sits (a NUL-only label fails the
  earlier visible check as `LabelEmpty`); the closed-list fields
  (color, file name) cannot match a NUL to a literal at all — both
  get boundary pins, no new rule.
- **Ordering, the one ruling this entry adds**: inside the three
  library names the NUL rides the EXISTING character scan — after the
  length bounds, one positional pass reports the FIRST offending
  character, whichever rule it breaks (`\n`/`\r` = §6.9's one-line
  rule, `\0` = this ruling): a name with a break at 1 and a NUL at 3
  says the break, the swapped one says the NUL. That is how the scan
  already worked for line breaks and how D-075's label control scan
  works; a rule-ordered pair of scans would report a later defect
  while an earlier one sits unreported. The project name (no
  one-line rule) keeps its own order: empty, too long, then the NUL
  scan. Length always wins over every character verdict.

§6.9 bans NUL in the variant TEXT only — grep-verified; nothing in the
spec's name rows reserves a control-character rule. These rejections
are therefore D-076-motivated EXTENSIONS stricter than the bare rows
(the same posture as D-076's variant-name one-line ruling), not
transcriptions; the spec should grow a matching one-line note in its
§6.9 name rows (reported to the orchestrator — the docs task owns
spec.md). Falsifiability: with the NUL arms mutated out, the new
per-field boundary tests and the extended precedence test fail; the
pre-existing variant-text and label NUL tests stay green (they pin
rules that predate T-252).

Deliberately NOT extended — the two text fields that are not names:
the project DESCRIPTION still carries no NUL rule (open question for a
follow-up card: a NUL there is inert for the target file but hits the
same stored-length arithmetic), and the variant text was already
covered. The extra-file names are fixed literals (`<STEM>.local.md`,
`<STEM>.project.md`), not user input — nothing to validate. No
migration or backfill is needed because no write path could ever have
stored a NUL (round-1 review's correction of this entry's first
draft): the project name's only writer derives it from the repository
FOLDER name (`derived_name` in the add-project command) — an OS path
component cannot carry `U+0000`, and a non-UTF-8 byte becomes U+FFFD
before `validate_name` sees it — and the library name fields have no
production write path at all (the command-layer wiring is a later
task, D-076/D-034). The schema itself does NOT forbid a NUL — a
mid-string NUL passes every name CHECK, because SQLite counts only up
to it — so the library editor, the SEC-009 import and any future
store-level writer MUST route names through these validators: this
gate, not the CHECKs, is what keeps the columns clean.

### D-128 — `projects_list` serves the dirty flag from the T-101 service cache: one truth, the clean baseline inherited, and DB-014 decided by the row — 2026-11-21

Problem: D-121's round-1 review routed a finding to this seam —
`commands.rs`'s `dirty_verdict` still answered the constant `false`
after the dirty service landed, so the wire's per-project `dirty`
(the initial state the UI must bootstrap its dots from; a one-shot
startup diff cannot bootstrap a late subscriber) and the
`state://dirty` event were two sources of truth that could only drift.

Solution: the command reads the service's managed
`DirtyState` cache (read-only; the recalculation mechanics untouched).

- _One truth, precisely._ New `DirtyState::last_verdict(&str)` is the
  read side of the exact map the service diffs its next
  `state://dirty` event against, so every verdict the wire serves is
  one the service truthfully recorded. The event itself is
  at-least-once and may carry a superseded change the cache no longer
  holds (D-121's overlap contract), so a consumer reconciling the two
  treats the read as the truth — the event updates, the read corrects;
  the docs say exactly that, not a stronger "agree by construction"
  (the round-1 review catch). The verdicts are read on the async
  runtime after the read transaction and AFTER the `spawn_blocking`
  availability probes (the round-1 ordering fix: a verdict snapshot
  taken before the probes could arrive seconds later, carrying dirt
  older than an event the webview had already applied; after the
  probes the race shrinks to the IPC boundary). The lookup itself is
  one short mutex guard + `HashMap::get` — no lock near an `await`,
  no lock held across the blocking pool. The lookups are per row, not
  one atomic snapshot: a concurrent recording between two rows can
  mix pre- and post-pass verdicts in one response — every value still
  a verdict the cache truthfully held, and cheaper than a map clone
  per read.
- _The clean baseline is inherited, not redefined._ Before the
  service's first pass the cache is empty and every verdict reads
  clean — D-121's own absent ≡ clean diff rule, now the command's
  startup semantics too. A missing state entirely (a theoretical
  broken setup: `setup` manages it before anything that could fail,
  recovery mode included) is the same answer via the command's
  `try_state` fallback: the honest degraded read is the clean
  baseline, not a failed command over a display flag — the opposite
  trade-off from `managed_db`'s honest `NoDatabase` error, because
  the list itself is still truthfully readable and the flag merely
  unobserved.
- _DB-014 stays row-decidable._ The archived arm is checked before
  the cache, so a stale `true` entry (archived after the last pass,
  before the next one prunes it — pinned by a test) can never leak a
  dot onto an archived column.
- _The `E_SYNC_RUNNING` gate (T-105) does not apply_: it covers the
  project-_changing_ commands; `projects_list` is a read, safe during
  a sync, answering the verdicts the running pass will refresh.

Testing: five mock-runtime tests in `dirty_recalc_tests.rs` driving
the real command over the harness (the cache seeded only through
production `recalc_and_emit` passes, never private helpers): both
verdicts reflected per project (synced-clean + never-synced dirty in
one cache), the empty-cache clean baseline, the archived half over a
stale cache entry (with the row's `state: "archived"` and its absence
from the active read pinned, so the dirty claim is about the archived
row), repeated-call stability (whole-vector equality over a dirty
verdict — not vacuous), and the missing-state fallback. All verified
falsifiable by mutation: the old constant fails the reflection test
and the stability test's dirty anchor, an inverted baseline fails the
empty-cache test, a cache-reading archived arm fails the stale-entry
test — every owning test red under its mutation. `src/bindings.ts`
regenerated (`export_bindings_are_up_to_date`, D-034): the doc mirror
now states the cache-backed boundary instead of the "pipeline has not
landed" one. D-121's routed finding is closed by this entry.

Round-1 review addendum (D-128, same task; report
`.opencode/reviews/t254-round1.md`): findings fixed here — the
"agree by construction" overstatement (docs in `commands.rs`,
`dirty_recalc.rs`, and this entry now state the reconciling-read
contract; the module doc's "the next pass corrects" sentence in
`dirty_recalc.rs` gained its missing condition — the correction fires
only when the world moved again), and the verdict-read-before-probes
ordering (moved after the probes). Findings routed elsewhere: the
recovery restore/recreate path still reopens a `Db` with no commit
hook and no service spawn, so after a recovery the dots can never
light this session — already D-121's routed follow-up, now with the
read side named (follow-up card: the recovery-commands task must
re-arm the service, not only the hook); no code change in this task's
frozen-zone-bound scope.

Round-2 review addendum (D-128, same task; report
`.opencode/reviews/t254-round2.md`, hy4-flash-free): findings fixed
here — the round-1 condition reached the module doc's diff-contract
paragraph but not two siblings, now fixed: `record`'s own doc
(`record` in `dirty_recalc.rs`) carries the same only-if-the-world-
moved condition, and `last_verdict`'s doc states the reconciling-read
contract instead of the refuted "cannot drift apart". This addendum
also narrows the round-1 claim above: the sentence that gained the
condition is the module doc's (in `dirty_recalc.rs`); D-121's own
DECISIONS addendum keeps its historical 2026-11-21 wording (history
is not silently rewritten — the sharpening lives here and in the
code docs). The round-2 question (the read can transiently lag a
just-emitted event by the emit→record window — the divergence is
bidirectional) is now stated in `dirty_verdict`'s docs; the future
consuming task reads that contract off `bindings.ts`.

### D-129 — T-253: protective DB copies retire by name-stamp age (7 days, newest of each kind protected); the file log caps at 5 daily files — constants, not config keys — 2026-11-21

Problem: the two protective-copy schemes in `db-backups/` accumulated
uncapped (D-046 recorded that the ERR-001 count-of-7 never touches
them; D-049 left `pre-recovery-*` "until an owner decides"), and the
file log rotated daily with no file-count bound at all. The owner
ruled on 2026-11-21: copies older than **7 days** go, by age alone
("независимо от количества; новые копии не удаляются никогда"); the
file log gets "ротация 5 файлов × 10 МБ (≈50 МБ максимум)". The card
left four decisions to this task: where the cleanup fires, the
age source, the limits' form (constants vs `config.toml` §5.8), and
how the log ruling maps onto `tracing-appender`.

Solution, four decisions:

- _Age source — the name stamp, never mtime._ Both schemes stamp the
  creation instant into the file name (`pre-migration-v<N>-<UTC>` with
  `<UTC>` = `%Y%m%dT%H%M%S%3fZ`, D-038's `utc_stamp`; `pre-recovery-<UTC>`
  with the same shape plus `-N` same-millisecond variants, T-024). The
  decisive fact: a pre-recovery copy is a **rename** of the damaged
  database, so its mtime is the damaged file's last-write time and can
  be arbitrarily older than the preserve moment — mtime-based age would
  judge a copy made today as "over a week old" and, with two of them in
  the directory, delete a fresh copy the day it was made, breaking the
  ruling's "новые не удаляются никогда" outright. The stamp is the
  creating scheme's own clock. Parsing (`retention.rs`): prefix →
  version digits (pre-migration only) → stamp via
  `Timestamp::strptime("%Y%m%dT%H%M%S%3fZ%z", stamp + "+0000")` — the
  appended offset is required because jiff 1.0.1's `Timestamp::strptime`
  refuses to produce a `Timestamp` without a parsed offset (verified
  empirically: "offset required to parse timestamp"; the schemes'
  trailing `Z` is a literal, not `%z`), and the stamp is UTC by
  construction of both producers. A name any part of which does not
  match — a hand-renamed copy, foreign files, `*.sqlite3.tmp` staging
  debris, sidecar siblings — is **never deleted**: a file whose age
  cannot be established has no age to judge (deleting it would be the
  count-based guess the ruling replaces). The `-N` variant orders
  within one stamp (`-2` written after `-1` after the plain name) —
  pinned by a dedicated test.
- _Semantics — strictly older than 7 days, per kind, newest
  protected._ `kind` is the name prefix (pre-migration across all
  versions is ONE kind — the ruling protects "one fresh copy of each
  вид", and a recovery must not cost the newest migration safety net).
  Delete iff `stamp < now − 7d` (a copy exactly at the edge stays; the
  test pins the millisecond beside it) AND the copy is not the newest
  of its kind (max stamp, variant breaking ties). This encodes both
  ruling halves: a sole 10-day-old copy survives exactly until a new
  one exists — the card's recorded edge — and `daily-*` copies stay
  under ERR-001's own count rule. The pass
  (`retention::cleanup_db_backups(backups_dir, now)`) is best-effort:
  an unreadable directory or an unremovable file logs a `warn` through
  the facade (`db_backup_retention_failed`) and the pass returns — it
  can never fail the operation that triggered it; a pass that removed
  something logs the per-kind counts at `info`
  (`db_backup_retention`), a no-op logs nothing (the D-046/D-054
  no-line policy).
- _Trigger points — one call site: `Db::open` after the migration
  run._ Every launch opens the database, so this is the startup pass
  the `retention.rs` pattern asked for, and an upgrade run's fresh
  pre-migration copies get their aged siblings retired immediately;
  every recovery operation (recreate/restore) reopens through
  `Db::open`, so the pass also fires right after a successful
  recovery. The pre-recovery creation site
  (`recovery::preserve_damaged`) is deliberately NOT a trigger — the
  round-1 review's P1-1: inside `restore_files` the preserve runs
  while the user's chosen copy is still the only durable source of the
  bytes being restored (the staged `db/*.tmp` is debris-swept by the
  next attempt, "debris, never data", D-049), so an age pass there
  could delete an aged, non-newest chosen copy before the publish
  commits and leave a failed publish unretryable. The chosen copy may
  still be retired later by the reopen's own pass, which is the
  "may itself be pruned by later retention" reading the frozen
  `commands.rs` comment already documents (a forced deviation from its
  neighboring "is left in place" phrasing that this entry records,
  Rule 10 — the file belongs to a parallel task and was not edited).
  The LOG-006 daily tick deliberately does NOT run this pass either:
  a long-running app cannot create a protective copy without passing
  through an open (pre-migration copies exist only inside `Db::open`,
  pre-recovery only via the recovery commands that reopen), and wiring
  `spawn_daily_cleanup` to the backups directory would need a `Db`
  getter the frozen-`lib.rs` signature cannot carry. Honest
  boundaries: a copy that crosses the 7-day line mid-run is retired at
  the next open, not at the crossing moment; a launch whose open
  FAILS (recovery mode) runs no pass — the aged copies wait for the
  recovery operation's reopen or the next successful launch (exactly
  the state in which the restore-keeps-its-source boundary above
  matters). A rerun is a no-op over a cleaned directory, and a pass
  interrupted mid-deletion converges on the rerun (both tested).
- _Limits' form — constants; the log ruling mapped to what
  tracing-appender has._ Spec §5.8 SYS-012 pins `config.toml`'s key
  list as exactly `log_level` (unknown keys warn), and D-029's
  bidirectional `KNOWN_KEYS` pin keeps the file at that list — so
  retention limits as config keys would contradict the spec this task
  may not edit. Constants it is: `PROTECTIVE_COPY_WINDOW = 7 days` in
  `retention.rs`, `MAX_LOG_FILES = 5` in `logging.rs`. The log half:
  the ruling's ideal was 5 × 10 MB, but tracing-appender 0.2.5 has no
  size-based rotation — its `Rotation` is NEVER/MINUTELY/HOURLY/DAILY
  only (verified in the vendored sources, `rolling.rs:501-509`) — and a
  custom size-rotating writer would break the §5.5-pinned
  `eunomiasync.<date>.log` name scheme D-032 recorded. The card's
  sanctioned alternative is taken: `Rotation::DAILY` stays and the
  builder gains `.max_log_files(5)` (the crate's own count cap; the
  prune runs at appender creation — every startup, which is the
  testable half — and at each rotation, rolling.rs:615/731). Honest
  boundary: the per-file 10 MB half is not enforced; the cap bounds
  days of history, and one chatty day can exceed 10 MB. If the owner
  wants the size half, that is a new task (a custom writer or a
  post-rotation size check) — follow-up material, not this one.

Tests: the pass's behavior suite lives in `retention.rs` (aged/recent
of both kinds, protected-newest past the window, the fresh-retires-
aged edge, the strict 7-day boundary, empty directory, idempotent
rerun and interrupted-pass convergence, sidecar sweep with a retired
copy, same-stamp variant ordering, undatable/foreign names untouched,
unreadable directory survived, a ~250-entry large directory reaching
the exact verdict); the parser probed directly (both schemes,
variants, all rejections including a non-numeric `-N`; instants
compared against independently parsed literals — jiff's `Display`
trims trailing fractional zeros); the wiring pinned at the call site
(`db_tests.rs` — `Db::open` retires the aged non-newest copy while the
daily scheme survives, stamps derived from the real clock because the
open call uses it) and the boundary pinned at the non-call-site
(`recovery_tests.rs` — a restore FROM an aged non-newest pre-migration
copy leaves the source and the aged neighbors in place until the
reopen, the round-1 P1-1 regression pin); the log cap in `logging.rs`
(building the appender over seven seeded dated files leaves exactly
five, the live file among them; below the cap nothing is deleted).
Mutation checks ran red→green: flipping the boundary comparison and
dropping the newest-protection each fail their test. Honest
boundaries: the rotation-time half of the log prune rides the
appender's own untestable clock, like D-032's midnight rollover; and
enabling `max_log_files` makes tracing-appender's own
`prune_old_logs` reachable, which reports a failed removal with an
`eprintln!` (rolling.rs:705-716) — under the D-032 Windows-GUI
no-stderr analysis an `eprintln!` there would abort the process; the
same crate already `eprintln!`s on the pre-existing daily-rotation
path, so this adds no new class of exposure, recorded here so the
Windows smoke run knows to look.

### D-130 — T-112: sync phase 0 as two transactions around a pure build — the flag-guarded start, the `running`/`not_started` record, and the SYS-005 criterion pinned at the composition level — 2026-11-21

Problem: T-112 turns §9.4 phase 0 into code: take `sync_running`
(T-105), record a `running` `sync_runs` row, snapshot the library
(T-100) and build every active project's content. Six points were
open: how the phase's steps compose (flag, read, build, write — how
many transactions, in what order); what the per-project rows carry at
phase 0 vs what phases 1–4 fill in; how the content build reuses the
D-121 mechanics when `dirty_recalc.rs` is a frozen zone; whether
phase 0 writes any final status; how the card's criterion (a mid-run
library edit stays out of the run AND leaves the project dirty) is
pinned without racing; and how a new src-tauri module registers when
`lib.rs` is held by the parallel T-102.

Solution: `eunomia_store::sync_runs` (new store repositories) +
`src-tauri/src/sync_phase0.rs` (the phase), composed as four steps.

- _Two transactions around a pure build._ Step 1:
  `commands::begin_sync` — the T-105 seam, typed `E_SYNC_RUNNING`
  refusal — and, the moment the flag is taken, the capture of the
  run's start timestamp: `sync_runs.started_at` names the run's
  START, not the later insert (§9.4 phase 0 step 2's "record the
  start time"; the round-1 review's P2-1 — the first spelling
  stamped at insert time and would have drifted by the whole build).
  Step
  2: ONE `Db::run` read transaction reads the T-100 snapshot (the
  SYS-005 shape the D-123 serialization argument names). Step 3: the
  content build on `spawn_blocking` AFTER the transaction (SYS-001,
  the D-096 ordering) — a pure function over the snapshot value, so
  an edit committing anywhere after step 2 structurally cannot enter
  the content. Step 4: ONE `Db::run` write transaction records the
  run (`sync_runs::create_running`). Every error path is total: the
  RAII `SyncGuard` travels inside the returned `Phase0Outcome`
  (guard, run row, per-project content in `sort_order`); an error
  before the outcome unwinds through the guard's drop — flag free,
  read wrote nothing, failed write rolled back.
- _Phase 0 records the fingerprint, not the verdict._ The run row:
  fresh UUIDv7 id, DB-002 `started_at`, `running`, `projects_total`,
  counters at their §6.2 zeros, `finished_at`/`error_code`/`details`
  NULL. Each `sync_run_projects` row: `not_started`, the built
  `target_path` (§9.4 phase-1-step-2 spelling,
  pre-symlink-resolution), and the DB-020 `build_hash` of the built
  content — the fingerprint phases 1–4 and the DB-024 writer read
  instead of rebuilding. Phase 0 deliberately writes NO final run
  status: §9.4's verdicts are phase 4's (T-113+), and a `running`
  row left by a dead process is the crash-recovery rule's
  `interrupted`, not phase 0's business — pinned by the whole-entry
  test (the row reads `running` after the outcome drops). The
  crash-window corollary (round-1 review, recorded): a process death
  BEFORE the record leaves no run row at all — the recovery rule has
  nothing to act on, which is the honest reading of "the run never
  started"; the row's existence is the run's existence, recorded
  once its content is built. Status transitions got no store writer
  yet for the same reason; the reads future phases need (`get`,
  `list_projects` — the latter in `sort_order` via the projects
  join) landed with the insert.
- _The D-121 mechanics, mirrored under the frozen-zone rule (AGENTS
  §3.10)._ `managed_text` (D-064/D-065's markerless seam) and the
  chunked mini-snapshots of 25 (the set-L budget) are duplicated into
  `sync_phase0.rs`, pinned independently against the same oracles
  (`build_content`'s real output for A/C, the body for B; a new
  26-project chunk-boundary crossing-wires test — a scope leak
  mutation was verified red against it). Consolidating the two
  spellings behind one shared builder is a follow-up card for the
  first task allowed to edit `dirty_recalc.rs`.
- _The criterion, pinned deterministically at the composition level._
  The steps are separately callable (`read_snapshot`,
  `build_projects`, `record_run`), so the test interleaves
  deterministically: synced-clean project (stored hash = DB-020 of
  `Original.`), snapshot read, EDIT of the assigned variant commits,
  build + record, then both halves — built content and recorded row
  still fingerprint `Original.`, and the live `recalc_and_emit`
  (D-121's service) reports the project dirty. Run twice: the writer
  is the app's own `Db` handle (the production serialization shape)
  and a second `open_db` connection (the T-100 out-of-band pattern —
  the guarantee holds against any writer).
- _The lib.rs boundary judgment._ The brief authorizes the two new
  src-tauri files but forbids `lib.rs` (held by T-102 in its own
  worktree); a Rust module file is dead code without its declaration,
  so the registration is the minimal mechanical cross the "allowed"
  list implies: exactly two declaration blocks
  (`pub(crate) mod sync_phase0;` plus the test-module declaration),
  nothing else in that file touched. The merge-ordering consequence
  is accepted deliberately: if T-102 lands first, the two blocks
  apply cleanly; a textual collision at the same anchor is a 4-line
  resolution the orchestrator performs.
- _Error mapping stays local._ `SyncRunsError → AppError` lives in
  `sync_phase0.rs` (`sync_runs_to_app_error`, the D-093/D-096
  boundary in the `projects_store_to_app_error` shape) because
  `commands.rs` is a frozen zone: SQLite through the shared base
  classifier, corrupt cells class-only (SEC-008), the by-id outcome
  the family fallback. The snapshot read reuses
  `dirty_recalc::build_snapshot_to_app_error` as-is. The UTF-8
  spelling of a built target path is total by construction (stored
  path TEXT + static ASCII file name) and refuses class-only rather
  than lossy (`TargetPathNotUtf8`).

Tests: store — 3 unit (vocabulary literal round trips, corrupt-cell
refusals, error Display) + 8 integration on a real migrated database(row shapes verbatim, UUIDv7/DB-002 id and timestamp, the
joined-transaction rollback under an FK refusal, the duplicate-input
floor, the empty run, `get` round trip, `sort_order` read order
against reversed insert order, the D-039 tampered-cell arm planted via
`ignore_check_constraints`); src-tauri — 12 (8 in
`sync_phase0_tests.rs` + 4 inline in `sync_phase0.rs`): the criterion
twice (app-handle writer, second-connection writer), the whole entry
(flag held while the outcome lives, content/rows verbatim against
independently spelled `build_hash` oracles, two schemes, the
crossing-wires rail, the row stays `running` after the drop, the
flag freed), the 26-project chunk boundary, the
settings-coverage matrix (scheme C + `CLAUDE.md` + enabled extras,
the round-1 fix), the empty project list
(recorded run, no rows), the second-start typed refusal (envelope:
`SyncRunning`, details `None`), and the failed start's flag release
(recovery-mode `NoDatabase` through the flag-first order). Mutation
check: the mini-snapshot scope leak (M1) failed the chunk-boundary
test red, green after revert.

Round-1 review addendum (D-130, same task; report
`.opencode/reviews/t112-round1.md`, deepseek-v4.1-flash — no P0/P1):
findings fixed here — P2-1 `started_at` now captured at the flag take
and passed through `create_running` (the store API takes the
caller's timestamp; the crash-window corollary recorded above);
P2-2 the settings-coverage test (scheme A with both extras at Start
pinning the §7.5 tie and the ALG-030 splice against the D-21
sentence literals, scheme C with `CLAUDE.md` and differing extra
positions pinning the target name and the hash's extras slots);
P3-1 the refusal test now counts `sync_runs` rows; P3-2 the dead
`SyncRunsError::NotFound` variant and its mapper arm deleted (`get`
answers `Ok(None)`); P3-3/P3-4 the `Aborted` and retention doc
wordings corrected (the cleanup orders by `started_at DESC, id
DESC`); P3-5 the broken code span in this entry reflowed; P3-6 the
module docs now state the `panic = "abort"` release caveat (the
dev/test unwind drops the guard; the release abort kills the flag
with the process). P3-7 (a cfg(test) interleave hook inside `run`)
declined: the exported steps ARE `run`'s body, and a test-only seam
in production code buys nothing the step-level drive does not
already pin.

Round-2 review addendum (D-130, same task; report
`.opencode/reviews/t112-round2.md`, hy4-flash-free — no P0/P1/P2):
findings fixed here — the stale src-tauri test count in this entry
(12, with the file breakdown); the mapper's `Member` arm documented
as the reads' forward seam (`create_running` cannot produce it
today); the cancelled-caller asymmetry documented in the module docs
(a dropped `run` future frees the guard but cannot stop an in-flight
write — `db.rs`'s own non-cancellability contract; the orphaned
`running` row is the startup `interrupted` recovery's case, and the
cancellation policy belongs to the `sync_run` command, T-118).

Follow-ups for the board (the orchestrator cards them): consolidate
the `managed_text`/chunking mirrors when `dirty_recalc.rs` unfreezes;
ARCHITECTURE.md grows the phase-0 module row (that file was outside
this task's allowed list); the `sync_runs` status-transition writers
arrive with their phase tasks (T-113+), and the `interrupted`
recovery pass is the startup task's.

### D-131 — T-113: sync phase 1 (preflight) over Phase0Outcome — the seven steps as one blocking-pool pass, the markerless write plan, and the E_DISK_FULL tail — 2026-11-21

Problem: T-113 turns §9.4 phase 1 into code: for every active
project, availability, resolved target path, write rights, the
existing file (scheme B: UTF-8 and markers), the ALG-033 `unchanged`
verdict, the D-25 confirmation list, the stale-tmp sweep — then the
free-space rule that aborts the run with `E_DISK_FULL`. Eight points
were open: how the phase composes over `Phase0Outcome` (how many
transactions, where the blocking pool sits); where the per-project
SCHEME comes from when `Phase0Outcome` does not carry it and
`sync_phase0.rs` is a frozen zone; what "build the final bytes"
(step 4) can mean when ALG-032 stamps the marker "once per run, at
the start of the write phase" — after preflight; the §9.4 step-5 vs
step-6 order when an empty build matches a marker-only file (D-24 vs
D-25 both fire); which §10 row carries the D-090 "could not check"
refusals (`resolve` NotReadable, `write_access` ProbeFailed, a
refused file read) that are not verdicts; what the free-space check
does about an empty copy list and a FAILED probe; which run-row
status an `E_DISK_FULL` abort writes and what happens to the counters
and per-project rows; and what the store's phase-1 writers look like
(the transitions D-130 deferred to "the phase tasks that cause them").

Solution: `src-tauri/src/sync_phase1.rs` (the phase) +
`eunomia_store::sync_runs` additions (`mark_project_skipped`,
`mark_project_unchanged`, `abort_run`) + `lib.rs` mod declarations
(the D-130 boundary judgment, same anchor).

- _Composition, phase-0-shaped._ ONE read transaction
  (`sync.skip_identical` + each run project's scheme from the
  projects table), the per-project seven-step preflight on
  `spawn_blocking` AFTER it (SYS-001, the D-096 ordering), ONE write
  transaction applying every `skipped`/`unchanged` verdict (`ready`
  rows stay `not_started` — phase 3's), and — only on the space
  shortage — a SECOND write transaction aborting the run. The
  free-space source and the sweep clock are injected parameters (the
  D-089/atomic.rs seam; production closes over
  `free_bytes(&data_dir)` and `SystemTime::now()`).
- _The scheme's second home._ `Phase0Outcome` carries content, hash
  and target path but not the scheme; reading it from the projects
  table in the phase's own transaction cannot observe an app-driven
  change: the run holds `sync_running` and D-123's SYS-005 gate
  rejects the project-mutation commands while it is up. Only an
  out-of-band writer on the database file could race it — the same
  residual T-100 window every post-snapshot read shares. The root
  and file name for steps 1–2 decompose from the built target path
  itself (`parent()` + `file_name()`): the stored root is canonical
  and the name one of four static ASCII literals (phase 0's own
  `target_path_text` argument).
- _The markerless write plan (step 4)._ ALG-032's marker does not
  exist yet at preflight, so the phase builds the markerless form of
  the write instead: A/C take ALG-030's shape minus the marker line
  (`managed` + one blank line, or the empty string) — exactly
  `compare::contents_match`'s documented new-side contract; B takes
  the ALG-031 splice minus the marker line, spelled by a private
  `splice_without_marker` — a deliberate byte-exact mirror of
  `assembly_b`'s private `build_block`/`dominant_terminator` (the
  D-130 `managed_text` posture), pinned against the REAL `splice`'s
  output over five file shapes × four managed shapes in the module's
  unit tests. The plan rides the outcome as an explicit `WritePlan`
  enum (the round-1 review's P2-1): `WholeFile { managed }` for A/C
  (phase 3 rebuilds the whole file around the run marker — no
  re-read, the write replaces everything), `Splice { managed,
content, span }` for B (phase 3 splices between the markers of the
  content phase 1 read — §9.4's reader is phase 1, and re-splicing
  the MARKERLESS form instead would replace the block with the
  marker-only body, corrupting the file). The variant is the scheme
  for phase 3's purposes (A and C share the whole-file shape), so no
  third home for the scheme column exists.
- _Step 5 before step 6 — the idempotence edge._ §9.4 runs the steps
  in order, so an ALG-033 match is `unchanged` even for an empty
  build over a marker-only file (the file the next write would
  produce byte for byte; D-24's own acceptance, not a D-25 wipe).
  Step 6 fires only on the ready path: empty build over an existing
  file with MORE THAN ZERO bytes — the raw size reading of D-25's
  "the file is not [empty]", which with `skip_identical` off puts a
  marker-only file on the confirmation list. Pinned by the D-25
  integration test both ways (the setting on: unchanged, off the
  list; off: on the list).
- _Refusals file under the unavailability row._ The three "could not
  check" shapes — `resolve_target_path`'s NotReadable,
  `check_write_access`'s ProbeFailed, and a non-NotFound read
  refusal — are not §9.4 verdicts (D-090) and not rights facts
  (D-094); each skips with `E_PROJECT_UNAVAILABLE`, whose §10
  situation list ("folder moved or deleted, drive disconnected, no
  read rights") is the nearest honest row, with the OS code riding
  in the message. The mapping table is unit-tested arm by arm with
  synthetic errors; the read-refusal branch is unreachable through
  mode bits (an unreadable file is read-only-flagged and skips at
  step 3) and documented as the total-function posture.
- _The regular-file gate (round-1 review's P2-4)._ Between steps 2
  and 3 the phase refuses a target entry that is not a regular file
  (a FIFO or device node — `resolve_target_path` accepts any
  non-directory, non-link entry by design, and an open-for-read on an
  empty FIFO BLOCKS, verified on the host, hanging the sequential
  preflight and its flag for the process's life; the fs crate is a
  frozen zone, so the gate lives in the app layer). No §10 code names
  "wrong entry kind": the skip files under `E_PROJECT_UNAVAILABLE`
  with the class-only message "the target entry is not a regular
  file" (E_TARGET_IS_DIR's literal says folder — using it would lie),
  pinned by a `mkfifo(1)`-fixture test.
- _The free-space tail._ The copies are the ready projects' existing
  files (phase 2's backup list); an EMPTY copy list skips the probe
  entirely (zero required bytes cannot come up short, and a failed
  probe must not abort a backupless run — pinned by a
  panic-if-called probe). A definitive `NotEnoughSpace` aborts the
  run (`aborted` + `E_DISK_FULL` + caller's `finished_at` + a
  numbers-only details line), leaves every per-project row at its
  phase-1 verdict, and answers the typed `E_DISK_FULL` envelope with
  free/required in `details`. A FAILED probe proceeds — the
  atomic.rs write-time posture (pinned by an injected
  `ProbeFailed` test); phase 2's `E_BACKUP_FAILED` is the honest
  reporter of a volume that cannot take the copies. The probe runs on
  `spawn_blocking` like every other syscall of the phase (the
  round-1 review's P2-5: production closes over a statvfs — SYS-001).
  The counters stay untouched on abort (phase 4's, the D-130 ownership
  rule): an aborted run's summary derives from the per-project rows.
- _The store writers._ Two named per-project transitions (skip
  carries code+message over an intact target_path/build_hash
  fingerprint; unchanged clears both error columns) and the
  run-level `abort_run`; every UPDATE is scoped to its source state
  (`not_started`, `running` — the round-1 review's P2-2: a
  transition driven twice, or against a settled row, matches zero
  rows and hears the typed `Sql` refusal instead of silently
  rewriting the run's audit record), and an UPDATE matching no row at
  all surfaces the same way — never a silent success (the
  `create_running` duplicate-input posture). `abort_run`'s doc names
  phase 1's tail rule only: phase 2's `E_BACKUP_FAILED` abort shares
  the row shape but adds the per-project `not_started` reset
  (§9.4 phase 2 step 2) — that writer is the phase-2 task's.
- _Sweep order._ Step 7 runs only for projects that reached it —
  unchanged and ready; a project skipped at steps 1–4 has no sweep
  (its steps stopped at the refusal), pinned by the sweep test's
  skipped-folder assertion. Best-effort: a refused listing never
  changes a verdict.

Tests: store — 4 new integration (skip shape over the fingerprint,
unchanged clears error columns, abort shape with counters/rows
untouched, the zero-row AND wrong-state typed refusals); src-tauri —
19 after round 1 (3 inline in `sync_phase1.rs`: the mirror pin
against real `splice`, the refusal tables' every arm, the class-only
invariant refusals; 16 in
`sync_phase1_tests.rs` on the mock-app harness with REAL repo
folders: criterion 1 twice (unavailable, read-only T-F5), criterion 2
(shortage abort with the exact 100+20%=120 margin oracle and
row/file assertions; the 119/120 boundary; the never-probing
backupless run), step 2 (three refusals with their codes + the
T-F6 symlink resolving and comparing through the real file),
the regular-file gate (the `mkfifo` FIFO skips class-only instead
of blocking the sequential pass — behind a 60 s watchdog so a gate
regression fails THIS test, not the harness timeout),
step 4 (E_NOT_UTF8, three marker shapes, SYS-009's deleted B target;
the B splice form with the CRLF-dominant markerless oracle), step 5
(A unchanged/different + the setting gate + the row transition),
step 6 (the D-25 list, the creation case, the idempotence edge both
ways), step 7 (stale deleted via std `set_modified` backdating,
fresh/foreign/.git kept, skipped folder untouched), the A/C creation
case with scheme C's `CLAUDE.project.md`, the sort_order order after
a reorder, and the recovery-mode NoDatabase refusal. Mutation check:
M1 (mirror terminator forced LF) failed the mirror pin and the B
splice test; M2 (setting ignored) failed four unchanged tests; M3
(tail disabled) failed the shortage test; M4 (D-25 without the size
guard) failed the confirmation test — all green after revert.

Follow-ups for the board (the orchestrator cards them):
ARCHITECTURE.md grows the phase-1 module row (outside this task's
allowed list, the D-130 precedent); the `interrupted` recovery pass
and the sync-run command's `NeedsConfirmation` gate are T-118+;
phase 3 consumes `ReadyProject` (the `WritePlan`, the B content and
span) when the write phase lands; the sweep's per-file failures get
their diagnostic line when the run's logging task lands.

Round-1 review addendum (D-131, same task; report
`.opencode/reviews/t113-round1.md`, deepseek-v4.1-flash — no P0/P1):
findings fixed here — P2-1 the ready product now carries an explicit
`WritePlan` (scheme-discriminated: `WholeFile { managed }` /
`Splice { managed, content, span }`), so phase 3 implements the
documented recipe without a third scheme home and without re-reading
the B file (the docs' splice wording was also literally corrupting —
splicing into the markerless form would drop the managed body);
P2-2 the three store writers scope their UPDATEs to the source state
(`not_started`/`running`) with wrong-state tests; P2-3 the
`SyncRunProjectStatus` vocabulary doc updated (phase 1 decides
`skipped` AND `unchanged`); P2-4 the regular-file gate (above) with
the `mkfifo` test; P2-5 the free-space probe moved to
`spawn_blocking`; P3-1 the failed-probe-proceeds test; P3-2 the
sweep test's healthy project is now `unchanged`, pinning the
step-7 call on that branch; P3-3 the sweep-comment now states the
report is not carried anywhere (the next sweep re-attempts; the
diagnostic line is the logging task's); P3-4 `abort_run`'s doc
narrowed to phase 1 with the phase-2 reset noted as that task's;
P3-5 the `Skipped` verdict carries structured
`details: Option<ErrorDetails>` (`io_error()` → `from_io_error`) so
T-118's log line has §10's OS facts — the store row keeps
code+message (the schema has no details column); P3-6 these
addenda are written from the actual reports.

Round-2 review addendum (D-131, same task; report
`.opencode/reviews/t113-round2.md`, hy4-flash-free — no P0/P1/P2):
the reviewer verified the round-1 fixes and found no regression in
the new paths (the regular-file gate's arm analysis, the blocking
probe's bounds and ownership, the WritePlan construction, the
state-scoped SQL, the B mirror's pin honesty, §9.4 order and §10
codes). Findings fixed here — P3-1 this round-2 addendum itself (the
pre-written stub replaced by this itemized text); P3-2 the Tests
enumeration above now lists the FIFO-gate test (the count was right,
the inventory summed one short); P3-3 the FIFO test gained a 60 s
watchdog (a worker thread + bounded channel receive) so a gate
regression fails the test loudly instead of hanging the suite to the
CI timeout.

### D-132 — T-115: sync phase 2 (backups) over Phase1Outcome — the store's `backup_records` writer, the BKP-010 dedup ruling under the UNIQUE column, and the E_BACKUP_FAILED abort — 2026-11-21

Problem: T-115 turns §9.4 phase 2 into code: for every project the
run will write whose target file exists, copy the file into the
§5.5 backups tree (BKP-010 size + SHA-256 check inside the fs
operation), record it, and on ANY error abort the run with
`E_BACKUP_FAILED` — this run's copies deleted (`discarded`), the
per-project rows reset (`not_started` except `skipped`), no target
byte moved. Eight points were open: what the `backup_records` writer
looks like and how it stays compatible with T-114's shipped reader
(D-122); what a dedup decision writes when the schema's UNIQUE
`backup_path` forbids a second row over the same file; how the abort
sweep orders file deletion against record marks; which timestamps the
records carry (D-061); where the failure injections live when the fs
crate's `fault-injection` feature is release-off; what BKP-005's
field list means for the envelope vs the run row under SEC-008; what
the phase-2 reset looks like in `sync_runs`; and how the phase
composes (transactions, blocking pool).

Solution: `eunomia_store::backup_records` (new repository) +
`eunomia_store::sync_runs::reset_projects_not_started` (the step-2
reset) + `src-tauri/src/sync_phase2.rs` (the phase), with `lib.rs`
mod blocks (the D-130 boundary judgment, same anchor).

- _The writer and the D-122 handshake._ `insert(conn, &NewBackupRecord)`
  takes the typed status (the full CHECK vocabulary owned here;
  phase 2 is the only v1 producer of `ok`-at-insert and
  `discarded`-at-insert), generates the UUIDv7 id inside the joined
  transaction, and stores exactly the conventions D-122 fixed:
  `source_path` = the §9.4 phase-1 RESOLVED path text (one stable
  spelling per file), `backup_path` = the §5.5 relative layout string
  the core builders spell, `sha256` = the full canonical digest.
  `last_checked(conn, project_id, source_path)` answers the pair's
  newest `ok` copy by `created_at DESC, id DESC` — the same "newest"
  the retention window uses — terminal statuses invisible. The
  integration suite pins the handshake end to end: rows written
  through THIS writer over real files are pruned by
  `backup_retention::apply` exactly the way T-114's seeded fixtures
  were.
- _The dedup ruling (round of the UNIQUE column)._ BKP-010's "the
  record points to the existing one" cannot mean a second row: the
  schema's UNIQUE `backup_path` is one row per copy, and a dedup
  decision creates no copy. The ruling: the EXISTING `ok` row stays
  the pair's one record (its own run, its own stamp), the phase
  writes NO row for a dedup decision, and the outcome carries the
  `Deduplicated { size, sha256_hex, backup_path }` fact so the run's
  log and phase 4 see where the pair's backup lives. First spelled
  the other way (a second row pointing at the same path), the
  integration run refused it with the UNIQUE constraint — the schema
  itself corrected the reading, and the test now pins it.
- _The composition, D-130-shaped._ ONE read transaction (each
  eligible project's name/scheme/base_name from the projects table —
  the D-131 second-home ruling — plus the dedup digest), the copy
  loop on `spawn_blocking` after it (SYS-001, the D-096 ordering),
  and ONE write transaction on success (every created copy's record
  `ok`, one `created_at` for the run). Eligibility is §9.4 step 1's
  two conditions off `Phase1Outcome`: `ready` AND `existing_size =
Some` (OUT-005's creation case, `unchanged`, `skipped` — nothing);
  an empty eligible list copies nothing, writes nothing, and the run
  continues (zero copies cannot fail). `now: jiff::Timestamp` is the
  run's ONE backup instant (D-061): it resolves into the `BackupTime`
  of every copy name, the DB-002 `created_at` of every record, and
  the aborted run's `finished_at` — record and name cannot disagree
  (the DB-002 spelling and the civil fields are pinned against
  literals in the module's unit tests; jiff 1.0's API is
  `to_zoned_datetime(TimeZone::UTC)`).
- _The abort path (§9.4 phase 2 step 2)._ Files first, marks second —
  D-122's delete-first order applied to `discarded` (a `discarded`
  record over a live file is the lying mark the prune ruling exists
  to avoid): the copies this run created are deleted on the blocking
  pool (NotFound = converged, D-091), then ONE write transaction
  writes the whole verdict — the deleted copies' records born
  terminal (`discarded` at insert: insert-`ok`-then-mark spends a
  transaction to reach the same cells), a deletion the filesystem
  REFUSES keeps that record `ok` over the live verified file (the
  never-lying-mark rule; the next run's dedup may even claim it),
  `abort_run` finishes the run `aborted`/`E_BACKUP_FAILED`, and the
  new `reset_projects_not_started` returns every non-`skipped`
  verdict to `not_started` with the error columns cleared (at this
  point in the pipeline only `unchanged` rows exist to reset; the
  sweep is over the whole vocabulary because `abort_run` is). The
  reset is a bulk sweep, not a state-machine edge: zero matched rows
  is `Ok(0)` (an all-creation-case backup failure legitimately has
  nothing to reset), unlike `mark_project_*`'s zero-row refusal.
- _The injections and the envelope._ The `backup` parameter is the
  copy operation (production passes `backup_target_file` itself; the
  D-089 seam) — the fs crate's `fault-injection` feature is
  non-default and stays release-off, so the app layer's seam is the
  honest injector: tests delegate to the real operation until the
  failing call, so the copies the abort sweep must delete are real
  files on disk. The typed `E_BACKUP_FAILED` envelope carries
  BKP-005's whole field list in `details`: project NAME (the one
  §10 message the spec explicitly demands it in — the recorded
  exception to the class-only default), source path, backups folder
  (the project's §5.5 folder via the pure builder), OS code
  (`os_code`) and error type (`os_kind`), cause (`BackupError`'s
  Display), and §10's recommendation. The run ROW's `details` stay
  class-only (project id + the error's paths/OS facts, no stored
  name text — SEC-008; the split is the envelope-vs-row ruling).
- _Store-failure postures._ A failed record transaction on the
  success path deletes the orphaned copies best-effort (the D-091
  sweep posture) and propagates the store error — the run row stays
  `running`, the crash-recovery rule's case (the D-131 posture);
  same for a failed abort transaction (files already gone, rows
  untouched). A process death between the deletions and the abort
  transaction leaves deleted files, no records, a `running` row —
  the startup `interrupted` recovery's case, the same window D-122
  recorded for its order.
- _Idempotence by convergence._ A retried run after an abort starts
  phase 0 anew; the aborted run's rows are history, and a copy that
  survived a refused deletion is simply the pair's newest checked
  copy — the retry's dedup lookup points at it. Nothing needs to
  recognize a previous attempt's state.

Tests: store — 3 unit (vocabulary round trip incl. the CHECK
literals, the corrupt-cell refusal, error Display) + 10 integration
in `tests/backup_records.rs` (the ok and discarded row shapes
verbatim with the UUIDv7 id pin, the UNIQUE/both-FK refusals, the
newest-`ok` lookup with the same-stamp id tiebreak, terminal
statuses invisible, per-pair scoping, the empty-table None, and the
T-114 handshake: three writer-made copies pruned by the shipped
retention at keep 1) + 2 integration in `tests/sync_runs.rs` for the
reset (the four-verdict mix with the `failed` row planted in plain
SQL — phase 3's writer is T-116+'s — pinning skipped-survives,
error-columns-cleared, fingerprints-intact, idempotence, and the
`Ok(0)` zero-match ruling); src-tauri — 3 inline in
`sync_phase2.rs` (the DB-002 stamp incl. truncation, the BKP-001
civil fields, the error-kind table) + 6 in `sync_phase2_tests.rs`
on the mock-app harness with real repo folders and a real §5.5
tree: the success path (builder-spelled names pinned as literals,
FIPS-oracle digests, resolved source_path, the run's one stamp, run
row `running`, targets verbatim), the dedup (a seeded `ok` record +
real earlier copy → no second file, no new row, the pair's record
intact), the card's criterion with the full verdict mix (healthy
copy swept + `discarded` history, failing project, skipped survives,
unchanged reset, every BKP-005 field asserted by name, targets
verbatim, `last_synced` untouched for all four), the size and digest
mismatches (aborted, VerifyFailed kind, no records, target
verbatim), the refused deletion (read-only backup folder, non-root:
record stays `ok` over the live copy, run still aborted), and the
empty-eligible run (creation case + unchanged + skipped: no copies,
no records, no folders, run continues). Mutation check: M1 dedup
input ignored — the dedup test red; M2 reset dropped — the unchanged
row stayed `Unchanged`; M3 deletion sweep dropped — the swept-copies
assertion red; all green after revert.

Round-1 review addendum (D-132, same task; report
`.opencode/reviews/t115-round1.md`, deepseek-v4.1-flash — no P0; 2×P1,
4×P2, P3s): findings fixed here —

- P1-1 (taken): BKP-005's field list reached only
  `details.os_message`, which the frozen error UI keeps inside the
  collapsed Details envelope (`AppErrorAlert.vue` renders the
  "Where" row from `params.path`/`params.project` only). The envelope
  now carries BOTH halves: `params` (`project`, `path` — UI-011's
  Where row) and the composed diagnostic line in `details`
  (folder, OS code + kind, cause, recommendation). The abort test
  asserts the params and every `os_message` field by name.
- P1-2 (taken): the dedup decision trusted the record and never
  checked the copy it points at still exists. The copy loop now
  verifies the recorded copy's file on the blocking pool before
  handing the digest to the fs operation; a vanished copy loses its
  dedup claim and a fresh copy is made (BKP-005's "no forced write
  without a backup" — the ruling is in the module docs and this
  entry). Pinned by `a_vanished_recorded_copy_does_not_dedup`.
- P2-1 (taken): the failed copy's own leftover partial bytes
  (`cleanup: Some`, the fs operation's retrier handshake, D-091) are
  now swept with the run — `lingering_copy_path` feeds the same
  blocking deletion pass. Pinned by
  `the_failed_copies_leftover_bytes_are_swept_with_the_run` (a real
  staged partial under a final-name shape).
- P2-2 (taken): the abort sweep's removals now run §9.5's
  transient-hold ladder (100/300/900 ms on sharing violation / access
  denied — os 32/5, the same gate `storage::retry_transient_hold`
  uses; the helper is `pub(crate)`, and `storage.rs` is outside this
  task's allowed list, so the ladder is mirrored in the phase with
  the same gate and pauses, documented).
- P2-3 (taken): the success-path transaction comment still claimed
  dedup claims write rows — corrected to the UNIQUE ruling.
- P2-4 (taken): (a) the record-transaction failure branch is now
  pinned (`a_record_transaction_failure_cleans_the_copies_and_
leaves_the_run_running`: a stale record claiming the run's exact
  §5.5 name under a historic source spelling → the copy is made, the
  insert hits UNIQUE, the orphan is swept, the store error
  propagates, the run row stays `running`); (b) the abort test also
  asserts the run row's details do NOT contain the project name (the
  SEC-008 split, now verified, not just documented).
- P3-3 (taken): a refused deletion is now a class-only count in the
  abort details ("N copy(ies) of this run could not be deleted").
- P3-9 (taken): the store constraint tests pin the constraint itself
  (`SqliteFailure { code: ConstraintViolation }` + the UNIQUE/FOREIGN
  KEY message) instead of `matches!(Sql(_))`, and the discarded-row
  test asserts the full verbatim shape.
- P3-1 (declined, recorded): `finished_at` stays the run's one
  injected instant — §6.2 pins no abort-stamp accuracy, a second
  clock parameter breaks the determinism the fixed instant buys, and
  in production both stamps come from the same real clock within one
  run; the phase-1 abort reads `now_utc()` because phase 1 owns no
  instant of its own.
- P3-2 (superseded by P1-1): the name now rides `params` for the UI;
  the composed `os_message` keeps it too (the acceptance criterion
  reads over the whole envelope).
- P3-4 (declined, recorded): the `db002_stamp` mirror stays (the
  store's helper reads the real clock; `values.rs` is outside this
  task's allowed list) — the unit test pins the literal, the D-130
  mirror posture.
- P3-5 (declined, recorded): the `expect` on a dedup outcome without
  a lookup names the INJECTED operation's contract — production's
  `backup_target_file` returns `Deduplicated` only for a digest it
  was handed, so the arm is unreachable outside a lying injector.
- P3-6/P3-7/P3-8 → follow-up cards for the board (below).

Follow-ups for the board (the orchestrator cards them): consolidate
the three `sync_runs_to_app_error` spellings (+ the new
`backup_records` mapper) when `commands.rs` unfreezes; the
`backup_records` index for the per-pair lookup (D-122's future schema
task, now N lookups per sync); an owner for pre-existing orphan `.bak`
debris (D-091's hand-off — neither retention nor this sweep knows
copies no record references); ARCHITECTURE.md rows for
`sync_phase2`/`backup_records` (outside this task's allowed list, the
D-130 precedent).

Round-2 review addendum (D-132, same task; report
`.opencode/reviews/t115-round2.md`, hy4-flash-free — no P0/P1): the
reviewer verified every round-1 fix in place and correct (the params
split, the dedup existence gate, the leftover sweep, the ladder's
gate and pauses against `storage::retry_transient_hold`, the
refused-deletion count), the spec/compat claims (eligibility, BKP-010
ordering, the abort verdict, the T-114 handshake, SEC-008, frozen
zones), and re-ran the suites. Findings fixed here —

- P2-1 (taken): the refused-deletion count now has its test — the
  refused-deletion test asserts the phrase in the run row's details,
  and the normal abort test asserts its ABSENCE (a fully swept run
  carries no refusal note).
- P2-2 (taken): the ladder is pinned through an injectable seam
  (`remove_copy_with(remove, path)`, the production wrapper closing
  over `std::fs::remove_file`): two transient holds (os 32) then
  success = three attempts; a non-transient refusal (os 28) = one
  attempt with the code preserved; NotFound = converged; an eternal
  hold (os 5) surfaces after the bounded ladder.
- P2-3 (taken): the dedup TOCTOU the round-1 gate left open — the
  recorded copy can vanish BETWEEN the pre-call gate and the digest
  decision (the source-hash window). A dedup answer is now
  re-checked against the disk; a vanished copy re-copies with no
  digest (`rebackup_without_digest`), so the write keeps its restore
  point. Pinned by
  `a_copy_vanishing_inside_the_hash_window_is_made_fresh` (the
  injector deletes the recorded copy and answers `Deduplicated` —
  the phase answers with a real copy).

The round-2 non-findings confirm the recorded rulings (the UNIQUE
dedup ruling, `finished_at`, the mirror, the expect posture) and the
follow-up list below.

### D-133 — T-116: sync phase 3 (the write) over Phase1Outcome — the run's one marker, the per-project commit, and DB-024's hash ruling — 2026-11-21

Problem: T-116 turns §9.4 phase 3 into code: every `ready` project,
one at a time in `sort_order`, gets its final bytes — ALG-030/031's
assembly around the run's creation marker — through §9.5's atomic
pipeline; success lands `written` plus the project's
`last_synced_at`/`last_synced_hash` "right away in a separate short
transaction"; a refused write lands `failed` with its §10 code while
the others continue and the backup stays. Six points were open: when
ALG-032's "taken once per run, at the start of the write phase"
capture happens now that phase 0 exists and took no marker; which
hash DB-024 stores — the DB-020 build hash or the digest of the
written bytes (the written file carries the marker; D-24/ALG-033
compare the markerless form); what "a separate short transaction"
separates (one transaction per project covering row and stamp, or
two); where the build hash comes from when `Phase1Outcome` does not
carry it; what the phase's own `Err` means when §9.4 says failures
are rows; and how the D-130 "read tx, blocking pass, write tx"
composition applies to a per-project loop.

Solution: `eunomia_store::sync_runs` phase-3 writers
(`mark_project_written`, `mark_project_failed`) +
`eunomia_store::projects::record_last_synced` (the point method —
no restamp existed; T-071's `update` deliberately leaves the two
columns to the sync tasks) + `src-tauri/src/sync_phase3.rs` (the
phase), with `lib.rs` mod blocks (the D-130 boundary judgment, same
anchor).

- _The marker, once, at the write phase's start._ The `clock`
  parameter is `FnOnce() -> MarkerClock` — production passes
  `now_marker_clock` (its >24h POSIX-zone guard included), tests fix
  the instant, and an empty `ready` list captures nothing at all.
  The phase formats `marker_line` ONCE from the one capture and
  hands the same `String` to every project's assembly — "every file
  of the run carries one marker" is structural (a `FnOnce` cannot be
  re-asked), and the behavior suite pins the byte-exact file forms
  sharing that one line (the T-111 criterion at the composition
  level; `marker_clock`'s own suite pins the capture side). Note:
  `marker_clock.rs`'s module docs mused the capture "belongs" at
  phase 0 — §9.4 phase 0 takes no marker and D-131's preflight
  explicitly cannot know it, so the ruling is ALG-032's own words:
  the write phase's start, i.e. here (core is a frozen zone; the
  doc line stays as written and this entry is the correction of
  record).
- _DB-024's hash ruling._ `last_synced_hash` is the run's own DB-020
  build hash of the MARKERLESS managed text — the very fingerprint
  phase 0 froze into `sync_run_projects.build_hash`, which phase 3
  reads back in its one up-front read transaction (D-130's "read
  the fingerprint instead of rebuilding" — no recomputation, no
  second build). It is NOT `WrittenFile::sha256_hex` (§9.5 step 6's
  digest of the written bytes, marker included): D-24/ALG-033
  compare the markerless form, so the stored value must be what the
  next comparison recomputes against. The outcome still carries the
  written-bytes digest as the §9.5 proof fact; the behavior test
  asserts the stamp equals the frozen fingerprint AND differs from
  the written digest (the concepts pinned apart). A `ready` project
  whose row holds no fingerprint is the class-only
  broken-invariant refusal (`Phase3FingerprintMissing`, the
  phase-1 `project_missing_refusal` family).
- _One commit per project._ §9.4 phase 3 step 2's "updated right
  away in a separate short transaction" is read as ONE transaction
  per project covering BOTH facts: `mark_project_written` (the
  state-machine edge `not_started → written`, error columns cleared
  — the unchanged edge's clearing rule) and `record_last_synced`
  (the run's ONE DB-002 instant as `last_synced_at`; the build hash
  as `last_synced_hash`; `updated_at` deliberately untouched — a
  sync stamp is not a project edit). "Separate" separates the
  verdict from the file write (not transactional) and from every
  other project's verdict (§9.4 step 3's "the other projects
  continue" rides on each verdict committing alone); a `written`
  row without its stamp, or a stamp without its row, would be half
  a verdict. The failure arm is one transaction too
  (`not_started → failed` with the §10 code and class-only
  Display message); the loop then takes the next project, and
  neither the backup (phase 2's committed history) nor the failed
  project's `last_synced_*` (OUT-020) is touched.
- _The composition is per-project, by spec._ D-130's
  read/blocking/write shape does not apply: §9.4 phase 3's own
  words are "one by one, in order", so the phase is ONE read
  transaction (the fingerprints) + a per-project [pure assembly →
  `spawn_blocking` write (SYS-001, the §9.5 syscall run) → one
  short verdict transaction] loop. The write operation rides an
  `Arc` (each blocking closure is `'static`) — hence the `Sync`
  bound on the seam. The `write` seam (production
  `production_write` closing over `write_target_file` + the real
  free-space probe; tests inject the typed refusals) is the
  phase-2 injector pattern; the §10 table maps arm by arm
  (`NameFailed` folds into `E_WRITE_FAILED`, the D-099 ruling).
- _The phase's own `Err`._ Never a write refusal (that is a `failed`
  row carried in the outcome, with the structured OS details for
  T-118's log line — the Skipped shape). `Err` is the store/executor
  family: the fingerprint read or a verdict transaction refusing, or
  `NoDatabase` — every one leaves the run row `running` (the
  crash-recovery rule's case, the D-131 posture). The recorded
  residual window mirrors D-132's orphaned copies: on the success
  path the FILE is already written when its transaction refuses —
  the file stays (§9.5's old-or-new; atomic's own docs forbid
  deleting it), the row stays `not_started`, and the next run's
  phase 1 sees the content (ALG-033 ignores the marker line) while
  the startup recovery recalculates hashes over what is on disk.
- _`whole_file` is the last mirror._ The WholeFile tail
  (`managed` + blank line + marker + break, or marker + break when
  empty) is spelled in the phase (the plan owns the assembled
  managed text; `build_content` consumes body + reading lines) and
  pinned against the real builder's own output — the D-130 posture.
  Splice calls the REAL `assembly_b::splice` (content, span,
  managed, marker): no mirror exists for the half the real builder
  owns, and D-131's plan carries the read `content`/`span` so phase
  3 never re-reads the file (§9.4's reader is phase 1).

Tests: store — `mark_project_written` (stale error cells cleared,
fingerprint intact), `mark_project_failed` (code + message,
fingerprint intact), both folded into the wrong-state refusal test
(unknown id and settled row are the typed `Sql` refusal);
`record_last_synced` (exactly the two sync columns — `updated_at`
pinned equal across a sleep past the millisecond boundary, state/
position/content verbatim, re-record overwrites, unknown id typed
NotFound). src-tauri — 4 unit in `sync_phase3.rs` (the ALG-030 tail
pinned against `build_content` over the builder's own assembly; the
§10 table arm by arm with synthetic errors; the DB-002 stamp incl.
truncation; the class-only refusal shape) + 5 in
`sync_phase3_tests.rs` on the mock-app harness with real repo
folders and the real §9.5 operation: the success path (both files
byte-exact around the ONE marker, exactly one marker per file,
`Written` digest against an independent hasher, rows `written`,
stamps = the frozen fingerprints ≠ the written digests, `updated_at`
unmoved, run row `running`, the creation case written), the scheme-B
splice (outside preserved byte for byte, marker inside the fence,
literal oracle), the card's criterion (first project's write
refused: `failed` row with code + message + OS details, second
project written, failed target and `last_synced_*` unchanged, both
backups still on disk), the E_VERIFY_FAILED injection (digest
mismatch: `failed` with the code, no OS fact — a decision, not an
OS failure), and the empty-ready list (zero write calls, no
verdicts, phase-1 rows untouched, run continues). Mutation check:
M2 stamp fed the written digest — the fingerprint pin red; M3 a
failed project stopped the loop — the continuation pin red; both
green after revert.

Follow-ups for the board (the orchestrator cards them):
ARCHITECTURE.md rows for `sync_phase3` (outside this task's allowed
list, the D-132 precedent); the marker_clock doc-line correction
("capture belongs at phase 0" → the write phase's start, D-133's
ruling) rides the first task allowed to touch `eunomia-core`.

Round-1 review addendum (D-133, same task; report
`.opencode/reviews/t116-round1.md`, deepseek-v4.1-flash — no
P0/P1/P2; 8×P3): findings fixed here —

- P3-1 (taken): the T-111 "marker taken once" pin was structural
  only (a fixed-instant clock cannot see a per-project re-capture).
  The success test now drives a SEQUENCED clock — the first capture
  is the fixed Prague instant, every later one a later minute — and
  asserts the capture counter is exactly 1, so a seam widened to
  `Fn` with per-project captures fails on the bytes AND the counter.
  The helper's closure is typed `Fn` precisely so that refactor
  compiles and dies on the assertions instead of on the bound.
  Mutation M4 verified red (re-capture per project →
  `captures == 2`), green after revert.
- P3-2 (taken): the missing-fingerprint invariant is now a
  PRECONDITION PASS over the whole `ready` list right after the
  fingerprint read — a broken phase-0 chain refuses the phase before
  the first byte moves (the empty-list "zero writes when the phase
  cannot proceed" posture) instead of mid-loop over a half-processed
  run; the in-loop lookup rides the reasoned-panic posture over that
  proof (D-061).
- P3-5 (taken): the "what the phase does NOT decide" list now names
  the D-25 precondition — the loop writes EVERY `ready` project, and
  the `confirm_empty` gate is T-118's, resolved through
  `Phase1Outcome::needs_confirmation` before this phase.
- P3-6 (taken): `production_write`'s dead no-parent arm collapsed
  into one code path — the frozen pipeline refuses a parent-less
  target before consulting the probe, so the `unwrap_or(target)`
  fallback only hands the closure a path; the comment says so.
- P3-7 (taken): the store-failure doc citation now states what the
  frozen crate actually states (never deletes a written target; the
  no-delete posture is its `VerifyFailed` docs') instead of
  attributing a general prohibition to it (house rule 11).
- P3-8 (taken): the duplicated comment line in the store test
  removed.
- P3-3 (declined, recorded): re-reading scheme-B targets before the
  splice is a spec ruling, not a task-level fix — §9.4's reader IS
  phase 1 (D-131's plan carries content+span so phase 3 never
  re-reads), the between-phases TOCTOU is §9.4's own design, and
  SYS-008's watcher owns external edits (the next sync converges;
  the backup covers the window). A follow-up card goes to the owner
  with the reviewer's re-read option.
- P3-4 (declined, recorded → follow-up): the terminal state of a
  `running` run behind a phase-level `Err` (flag released by the
  guard's drop, row `running` until the NEXT startup's
  `interrupted` recovery) is the recorded D-131 posture; the
  decision whether T-118's error path should finish the run instead
  is T-118's, and a follow-up card names it.

Round-2 review addendum (D-133, same task; report
`.opencode/reviews/t116-round2.md`, hy4-flash-free — no P0/P1/P2;
1×P3): the reviewer verified every round-1 fix in place and
discriminating (the sequenced clock's bytes-and-counter double pin,
the precondition pass's ordering and its `is_none_or(Option::
is_none)` double arm, the doc corrections), confirmed both round-1
declinations without new arguments, re-audited the loop fresh
(transaction boundaries, the `Arc<write>` sharing, the marker reuse,
the failure-vs-`Err` split, SEC-008, the §10 table against the
frozen enum) and re-ran the suites. Findings fixed here —

- P3-1 (taken): the round-1 P3-2 fix's own property — a broken
  phase-0 chain refuses the phase BEFORE the first byte moves — had
  no behavior test. Added
  `a_missing_fingerprint_refuses_the_phase_before_any_write`: the
  second `ready` project's `build_hash` planted NULL by direct SQL
  (the store suite's planting shape), the counting write sentinel,
  and the assertions — class-only `Phase3FingerprintMissing`
  refusal, ZERO write calls (the first project's intact fingerprint
  must not buy it a write), rows/files/stamps untouched. Mutation M5
  verified red (the pre-pass disabled → project 1 written, then the
  reasoned-panic `expect` fires), green after revert.
