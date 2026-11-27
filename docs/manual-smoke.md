# Manual Smoke Checklist

Checks that automation cannot prove — or proves only against a mock.
The CI legs prove compilation and unit behavior; what a real OS
refuses to let a test do — holding a locked file, denying directory
writes across a launch, watching a multi-megabyte session rotate — is
verified here, by hand, on a real machine. The scenario runs at the
bottom are the complementary half: e2e (spec §15.1) walks those
against a mocked IPC backend in a browser, and the real shell — native
folder dialogs, real file writes, window behavior — stays for the
manual run. Every item names its source (a DECISIONS.md entry, an
ARCHITECTURE.md section, or the spec sections it restates); the pass
criteria restate those sources and add nothing.

The file carries two parts. Part one: the three Windows GUI smoke
checks from D-143 (the file log's size half, T-255) — what the unit
suite cannot reach: the real clock, the production worker-thread path
of the degradation warn, and Windows' GUI stderr and file locking.
Section 4 (D-186) is the first any-OS item that joined them: the
recovery path's re-arm of the state services; section 5 joined it in
the same shape — the externally-modified display half of OUT-014 — and
section 6 (T-128) rejoins the Windows family: the real editor lock of
§15.2 T-F8 inside a real sync run. Part two: the scenario
runs S1–S7, S9, S10 that spec §15.5 conditions the release on (added
by T-315). Items for other OSes and other subsystems may join below.

This file is the in-repo home for manual smoke checks — it is not the
spec's release checklist itself; a release run walks both. Since T-315
it carries the release-checklist scenario runs too: §15.5 conditions
the release on "The manual checklist for scenarios S1–S7, S9, S10 is
done on each OS, and the manual smoke checks tracked in
`docs/manual-smoke.md` pass" (§15.1 explains why the scenario half
stays manual — e2e drives a mocked IPC, not the real shell). S8 and
S11 are P1 (spec §15.1's e2e row, §16's post-1.0 stage) and are not in
§15.5's manual list; they are not included here.

## Common preparation

- A Windows release build of the app, installed and launched at least
  once, so the data directory exists:
  `%LOCALAPPDATA%\pro.yepbro.eunomiasync\` (ARCHITECTURE.md, Data
  directory). The log files live in its `logs\` subdirectory.
- The config file `<data dir>\config.toml` is created with defaults on
  first run; to make a session chatty enough to rotate, set
  `log_level = "trace"` in it (values: `trace`, `debug`, `info`,
  `warn`, `error`; default `info`) and restart the app. Heavy scan and
  sync runs over a large project generate the traffic; expect several
  runs, not seconds.
- Log file names carry the **UTC** date, not the local one
  (`eunomiasync.<date>.log`) — near local midnight the two dates
  differ. This checklist writes `<date>` for that UTC stamp.
- A plain (non-elevated) PowerShell window — the lock and ACL commands
  touch only the user's own profile. In it, set the abbreviation used
  below: `$d = "$env:LOCALAPPDATA\pro.yepbro.eunomiasync"`.
- After every item, restore what was locked or denied, delete the
  seeded files if any, and set `log_level` back to `info`.

## 1. Rotation past 10 MiB — same-day parts, at most five files

Source: D-143 (T-255), ARCHITECTURE.md §Logging. The per-file cap is
10 MiB = 10,485,760 bytes; the directory holds at most five canonical
log files (`MAX_LOG_FILE_BYTES`, `MAX_LOG_FILES` in
`src-tauri/src/logging.rs`).

**Prepare.** A project large enough that trace-logged scan and sync
runs generate megabytes of log lines.

**Do.**

1. Set `log_level = "trace"`, start the app, note the UTC date.
2. Run scan and sync repeatedly. Watch `$d\logs` in Explorer.
3. Keep going after the first part appears — a full five-slot day is
   the point.

**Pass when.**

- The session logs into `logs\eunomiasync.<date>.log` — the first file
  of the day keeps the exact spec §5.5 name.
- When the current file reaches the cap, the next write rolls into a
  same-day part `eunomiasync.<date>.2.log`, then `.3.log`, … — the
  inserted number is decimal, from 2, no leading zeros.
- No file exceeds 10,485,760 bytes — a file lands at the cap exactly
  and the write after it rolls (check sizes in Explorer properties;
  the one documented exception: a single line larger than the cap
  lands alone in its part, impossible at trace-level line sizes).
- The number of canonical log files — `eunomiasync.<date>.log` and
  `eunomiasync.<date>.<N>.log` names, nothing else — never exceeds
  five. When a sixth would appear, the oldest by name (date, then
  part) is deleted. Files whose names are not the app's own grammar
  are never counted or touched — drop a `notes.txt` into `logs\` and
  confirm it survives.
- The app stays up and responsive the whole time.

## 2. A read-only `logs\` degrades without aborting

Source: D-143 (the startup variant), ARCHITECTURE.md §Logging (the
failure policy and the mid-run variant). Two variants — the policy has
a startup half and a rotation half.

### 2a. Directory blocked before launch — stderr degradation

**Prepare.** The app not running, and `logs\` holding no file for the
current UTC day — delete the parts an earlier item left
(`Remove-Item "$d\logs\eunomiasync.<today's UTC date>*.log"`). The
deny below blocks only the _creation_ of a new file; an existing day
file would be reopened in place, the file log would come up normally,
and the degradation would never fire. Then deny file creation in
`logs\`:

```powershell
icacls "$d\logs" /deny "${env:USERNAME}:(WD,AD)"
```

**Do.**

1. Launch the app from a terminal with stderr redirected to a file (a
   GUI-subsystem executable still inherits redirected file handles, so
   this is the one way to see the degradation on Windows):
   `& "<path to EunomiaSync.exe>" 2> stderr-capture.txt`
2. Use the app normally for a minute.

**Pass when.**

- The app starts and stays interactive — a broken log never aborts the
  launch. (If the launch aborts outright, the deny reached the
  data-layout step instead of the log — record the error text; that is
  a different failure than the one this item proves.)
- `logs\` gains no `eunomiasync.*.log` file during the session.
- `stderr-capture.txt` contains one `error` event naming the failing
  path ("file log unavailable; logging to stderr"). This line is the
  criterion, not decoration: with an unusable stderr the same line
  would be dropped, and the observable half would be only the missing
  file.

**Restore.** Remove the deny, restart the app, and confirm the file
log returns.

```powershell
icacls "$d\logs" /remove:d "${env:USERNAME}"
```

### 2b. Directory blocked mid-session — rotation degrades in place

**Prepare.** As item 1: `log_level = "trace"`, and sync traffic has
brought the current file near the cap. Deny creation in `logs\` while
the app is running (the already-open file keeps its handle; the deny
blocks only new files):

```powershell
icacls "$d\logs" /deny "${env:USERNAME}:(WD,AD)"
```

**Do.** Keep syncing until the current file crosses the cap, so the
writer tries to open the next part.

**Pass when.**

- The app does not abort, and no new part file appears.
- Lines keep landing in the current (over-cap) file — the cap is
  exceeded, never the launch. The current file's size grows past
  10,485,760 bytes.
- Exactly one warning appears in that file:
  "the file log could not rotate; writing continues into the current
  file", with `step=open`, the path and the OS error. One line, not
  one per write — the report fires once per degradation episode and
  retries silently.
- Remove the deny (`icacls … /remove:d …`) and trigger another
  rotation: the next part appears and the log heals by itself.

## 3. A locked log file does not crash the prune

Source: D-143, ARCHITECTURE.md §Logging. The retention prune runs at
startup and before every rotation; a locked file must not take it — or
the app — down. Lock mid-run: a prune failure at startup is silent by
design (no subscriber exists yet to report through), so only the
mid-run prune can show its verdict.

**Prepare.** With the app already running (the startup prune is the
silent one, so the seeds and the hold must exist after a launch, not
before one): `log_level = "trace"`, the current file near the cap, and
`logs\` holding more than four canonical log files (seed copies named
with older UTC dates if needed, e.g. `eunomiasync.2026-01-01.log`), so
the prune has something to delete. Open a hold on one of the oldest
files — the file the prune will try to delete:

```powershell
$h = [IO.File]::Open("$d\logs\eunomiasync.<old-date>.log",
    'Open', 'Read', 'None')
```

Share mode `None` is an exclusive hold: deletions fail while it is
open. Do not lock today's file or the part the rotation is about to
open.

**Do.** Sync until the writer rotates.

**Pass when.**

- The app does not crash; the session continues.
- The rotation still happens: the next part appears
  (`eunomiasync.<date>.<N+1>.log`, where `N` is the part the current
  file carries — part numbers continue per day, they do not restart)
  and new lines land in it.
- Exactly one warning names the locked file: "the file log could not
  rotate; writing continues into the current file", `step=prune`, the
  locked path, an OS error (a sharing violation). Once per episode.
  Grep the old part and the new one — the warn rides the async log
  queue, so it can land on either side of the switch.
- Every unlocked file beyond the retention is deleted as usual.
- Release the hold (`$h.Close()`), trigger another rotation (or
  restart the app): the previously locked file is now deleted — a
  healed disk heals the log, and the retention converges without a
  restart dedicated to it.

## 4. Recovery re-arms the state services (any OS)

Source: D-186, ARCHITECTURE.md's "Recovery mode" paragraph, spec §10's
`E_DB_CORRUPT` row. The recovery commands run the same session tail a
healthy launch runs. The IPC suite proves it over the mock runtime
(`ipc_tests.rs`, the T-322 batteries); the one thing it cannot reach is
the real `setup` hook's recovery-mode return followed by the real
screen's command — this run. Pending the recovery screen (the
recovery-UI task): until it lands, the item waits.

**Prepare.** App closed. A project with one assigned instruction,
synced once — its unsynced-changes dot clear, no external-change mark.
Overwrite `<data dir>\db\eunomiasync.sqlite3` with bytes that are not
a database (any text file's content). For the second run keep a
`daily-*` copy in `<data dir>\db-backups\` — every earlier launch
leaves one per UTC day.

**Do.** Launch the app and pick "Start with empty database" on the
recovery screen (then: add the project again, create an instruction,
assign it). Repeat from the preparation with a copy instead — pick
the `daily-*` copy — and skip the re-add: the restored projects are
the ones to watch. In both runs, once the matrix shows the project,
edit its target file in an editor outside the app.

**Pass when.**

- No restart in between: the project's unsynced-changes dot lights
  after the assignment by itself — no focus change, no manual refresh.
- The external-change mark appears once the editor saves the target
  file.
- The log carries the recovery's own audit lines — "recovery: the
  damaged database was preserved (renamed, not deleted)", then
  "recovery: started with an empty database …" or "recovery: the
  chosen copy replaced the database …" — and after them neither the
  "watching is disabled until the next launch" warning nor a
  "state not managed" panic.

## 5. An external target-file change is displayed and not written back (any OS)

Source: spec §5.7 SYS-007/008, §7.5 OUT-014 (its display half — the
column state, no merge, no write on a watch event), OUT-015; §7.4
MAIN-028. Not a §8 scenario: this entry was added for the follow-up
recorded at D-183's P3-5 (T-126) — the watcher suite pins the service
half of OUT-014; the display half is UI and has no automation —
written in the same entry shape as the scenario runs below. An
external edit here means a change made by anything but the app — an
editor, another tool.

Pending: the matrix column headers (MAIN-028) — spec §16 M3. The
watching backend is landed (§5.7: the debounced watcher, the re-check,
`state://file-changed`); the column state icon and text are not. Until
the matrix lands, the item waits.

**Prepare.** One throwaway scheme-A repo (the scenario section's common
preparation: `git init` plus one commit — never a live repository),
one project added and synced once — its column is clean and shows the
short last-sync time (MAIN-028), and the target file
`<repo>/AGENTS.md` exists. An external editor.

**Do.**

1. In the editor, outside the app, add a line to `<repo>/AGENTS.md`
   and save. Watch the project's column header for a minute — the
   debounce is 300 ms (SYS-007) and the re-check follows it (SYS-008),
   so watch, don't stopwatch.
2. While the mark is showing, read the file again.
3. Restart the app without touching the file; read the column.
4. Run sync; then read the file and `<data dir>/backups/`.

**Pass when.**

- Step 1 flips the column header to the "externally modified" state
  (MAIN-028) by itself — no restart, no manual refresh, and no dialog,
  merge view or conflict prompt anywhere (OUT-014: an external edit
  flips the state without any dialog; the app never merges).
- The unsynced-changes counter does not light: the externally-modified
  state is independent of the dirty flag (SYS-008, DB-021) — a file
  edit is not a library change, and the build hash is unmoved.
- Step 2 sees the editor's bytes, unchanged by the app (OUT-014: the
  app never writes a file because of a watch event).
- Step 3 shows the mark once the matrix first renders after the launch
  — the state is computed at startup (SYS-008), not only from live
  watch events.
- Step 4 overwrites the file: the editor's line is gone (OUT-015; the
  acceptance column: content added by hand disappears after a sync),
  and `<data dir>/backups/<project_id>__<slug>/` gains one copy of the
  externally modified file (OUT-014's acceptance: a sync still
  overwrites the file and makes a backup; BKP-001). The column returns
  to clean and shows the new last-sync time (MAIN-028).

**Restore.** No rights or locks to undo; the scratch project goes with
the scenario section's common-preparation cleanup (the step-4 backup
copy stays under `<data dir>/backups/` by design).

## 6. A locked target file inside a real sync run (Windows)

Source: spec §15.2 T-F8 ("The file is locked by an editor (Windows) →
3 retries, then `E_FILE_LOCKED`; backup kept"), §9.5 step 4, §10's
`E_FILE_LOCKED` row, §9.4 phase 3 step 3; §15.3's Windows row ("locked
files"). The ladder and the run-level verdict are automated — the
retry schedule and its classification are pinned in `eunomia-fs`
(atomic.rs and `tests/fault_injection.rs`, D-099; the REAL
Windows-handle criterion runs on the CI Windows leg), and the
run-level half (the `failed` row with `E_FILE_LOCKED`, the kept
backup, the continued run) is pinned in `sync_pipeline_tests.rs`
(T-128) over the command's write seam. What no automation covers is
the composition this item walks: a REAL held-open target inside a
REAL app run — the app's own watch service, progress and finish
included.

Pending: the "Run sync" button, the progress region and the summary
modal — spec §16 M3/M5, the same dependency S7 names (the backend the
item drives is landed and pinned); until the UI lands, the item waits
beside S7.

**Prepare.** Two throwaway scheme-A repos (the scenario section's
common preparation), each added as a project and synced once, so both
target files exist and hold the build. One instruction assigned to
both projects; edit its variant text so the next sync has something to
write and back up for both. Hold `<repo1>\AGENTS.md` open WITHOUT
delete sharing — the deterministic holder, the same share-mode shape
the CI test builds, from PowerShell:

```powershell
$f = [System.IO.File]::Open("<repo1>\AGENTS.md", 'Open', 'Read', 'Read')
```

(FileShare.Read alone — no FILE_SHARE_DELETE, so the replace must
refuse.) A real editor that provably locks the same way works too;
verify by the outcome below, not by assumption. Leave the handle
open.

**Do.**

1. With the file held open, run sync (§9.4; the §7.10 `sync_run`
   command; MAIN-024…027's "Run sync" button).
2. Read the summary's per-project rows, the target file, the backups
   folder, the file log; release the holder (`$f.Close()`).
3. Run sync again.

**Pass when.**

- Step 1 does not hang or crash: the run takes a little over 1.3 s
  longer than usual for the held project (§9.5 step 4's 100/300/900 ms
  pauses), then answers — the app's own UI stays responsive
  throughout (SYS-001: the write rides the blocking pool).
- The held project's row reads `failed` with `E_FILE_LOCKED` and a
  message naming the file, the lock and "3 retries failed"; the run's
  verdict is `partial` — the OTHER project is `written` (§9.4 phase 3
  step 3: a failed write stops one project, never the run).
- The held file is intact: the bytes on disk are exactly what they
  were before the run (§9.5: a refused replace leaves the original
  alone), and no staging file of the D-088 grammar
  (`.<name>.eunomia-<8 hex>.tmp`) remains in the repo (§9.5 step 7
  removed it).
- The backup is kept: `<data dir>\backups\<held project's
id>__<slug>/` gained this run's copy of the pre-sync bytes — the
  failed write does not sweep phase 2's committed history (§9.4 phase
  3 step 3).
- Step 3 writes the file: the held project goes `written`, its file
  holds the new build — §10's remedy for a lock is the idempotent
  retry once the holder closed. No new copy appears: the source still
  equals the copy the locked run made, so BKP-010's dedup claims it
  (the pair's one copy is the locked run's own).

**Restore.** Release the holder; the scratch repos go with the
scenario section's common-preparation cleanup (the backup copy stays
by design).

## Scenario runs S1–S7, S9, S10

The release condition (spec §15.5): "The manual checklist for
scenarios S1–S7, S9, S10 is done on each OS, and the manual smoke
checks tracked in `docs/manual-smoke.md` pass." The entries below
restate the scenario runs the spec owns (spec §8) in the same
Prepare/Do/Pass-when shape, with pass criteria tied to the spec's
requirement IDs; the platform mechanics an entry adds (rights
changes, folder renames) are only means to reach the spec's own
states, never new claims.

**Pending surfaces.** The entries are written against the spec's
surfaces (§7, §8, §11), because the main-window UI has not landed:
as of 2026-11-24, `src/App.vue` renders the shell only — no toolbar
buttons, library, matrix, scan modal or sync panel. The
scenario-relevant backend commands (`project_add_manual`,
`project_archive` (T-108), `project_restore` (T-109),
`project_relink` (T-110),
`project_update` (T-106),
`scan_start`/`scan_cancel`/`scan_commit`, `sync_run`,
`sync_cancel` (T-120),
`sync_confirm_exit` (T-123),
`sync_status`/`sync_last_result` (T-121),
`settings_get`/`settings_set`, the library read `library_get`, the
instruction lifecycle
`instruction_create`/`instruction_update`/`instruction_hide`/
`instruction_restore`, the variant commands
`variant_create`/`variant_update`/`variant_set_main`/
`variant_delete`/`variant_restore`, the structure commands
`heading_create`/`heading_update`/`heading_delete`/`items_move`, the
assignment commands `assignment_set`/`assignment_set_custom`/
`assignments_bulk` (T-331), the history reads
`revisions_list`/`revision_get` (T-331), the Backups read
`backups_list` (T-125), and the Backups export
`backup_save` (T-353)) are registered
(`src-tauri/src/lib.rs`). No entry below is executable until its surfaces
land; each entry's Pending line names them, with the spec §16
milestone that owns them (the milestone table maps S1/S2 → M1,
S3/S5 → M2, S4–S6 → M3, S9 → M4, S7/S10 → M5). An entry may name a
surface another milestone owns — the Activity window is M6 ("Service
features"), the matrix columns are M3 even to S1/S2 — so an entry
becomes executable when every surface it names has landed; the Pending
line names the entry's own milestone, not every one it touches.

**S8 and S11 are deliberately absent** — P1, post-1.0 (spec §15.1's
e2e row, §16's next stage), not in §15.5's list.

### Common preparation

Per OS — the release condition is per OS:

- The release build installed and launched at least once, so the data
  directory exists: Linux `~/.local/share/pro.yepbro.eunomiasync/`,
  macOS `~/Library/Application Support/pro.yepbro.eunomiasync/`,
  Windows `%LOCALAPPDATA%\pro.yepbro.eunomiasync\` (spec §5.5;
  ARCHITECTURE.md, Data directory). The entries write `<data dir>` for
  that path.
- Throwaway Git repositories in a scratch folder — `git init` plus one
  commit each; a repository is any folder with `.git` (ALG-052). Sync
  writes real files into the projects: never point the app at live
  repositories during a checklist run.
- Settings at their §6.5 defaults unless an entry says otherwise
  (scheme A, `AGENTS.md`, `sync.skip_identical` on, scan depth 6 with
  the default exclusions).
- After every entry: restore anything that was denied or read-only,
  and clean the scratch projects out of the app if the list should
  stay empty for the next item.

### S1. Adding a project by hand

Source: spec §8 S1; §7.1 PRJ-001…004, PRJ-010 (D-29), PRJ-012; §6.8
DB-040…042.

Pending: the "+ Add project" button, the system folder picker flow and
the "Project settings" modal — spec §16 M1. (The backend
`project_add_manual`, `dialog_pick_folder` and the modal's save arm
`project_update` are registered — the §6.9 fields and the PRJ-012
scheme/file-name/extra-file half both.)

**Prepare.** Four throwaway repos (`r1`–`r4`), one plain non-Git
folder, one Git folder with read access denied (Windows: `icacls
"<folder>" /deny "${env:USERNAME}:(RD)"`; Unix: `chmod 000
"<folder>"`), defaults untouched (scheme A, `AGENTS.md`). For the
archived branch of step 5, archive one added project first
(type-the-name confirmation, UI-010).

**Do.**

1. "+ Add project" (Ctrl/Cmd+Shift+N): pick the non-Git folder.
2. Pick the read-denied folder.
3. Pick `r1`'s root; repeat for `r2`–`r4`.
4. Pick a subfolder inside `r1`.
5. Pick `r1` again; then pick the archived project's path.
6. Read the new columns' headers in the matrix.

**Pass when.**

- The non-Git folder is rejected with `E_NOT_A_REPO` and no project
  appears (PRJ-001's four cases: repo root, subfolder, again, non-Git).
- The read-denied folder is rejected too — §6.9's project-path
  validation (exists, is a folder with `.git`, is readable), S1's Alt
  "no read rights" — and nothing is added.
- The subfolder pick offers the repository root before anything is
  created.
- The repeat pick shows the already-registered message; the archived
  pick offers to restore the project (DB-015).
- Each completed add opens "Project settings" on the new entry; the
  settings show the defaults copied at creation — scheme A,
  `AGENTS.md` (PRJ-012).
- The color rule: from a clean project list, `r1`–`r4` get
  pairwise-different palette colors (D-29 assigns the least-used
  color; PRJ-010's full criterion is 12 projects in a row — the
  four-repo pass is its smoke half). Each header shows the color dot
  plus the default label: the first up to 6 characters of the name,
  upper case (DB-042).
- S1's success line: the project is visible as a matrix column.

**Restore.** Remove the deny (`icacls … /remove:d "${env:USERNAME}"`)
or `chmod 755` the denied folder.

### S2. Scan

Source: spec §8 S2; §7.1 PRJ-002, PRJ-005…007, NFR-015; §9.6
ALG-051…055; §6.5 scan defaults.

Pending: the Scan modal (UI-002) and its result view — spec §16 M1.
(The backend `scan_start`/`scan_cancel`/`scan_commit` are registered.)

**Prepare.** A scratch root containing: two plain repos; a repo nested
one level inside another repo; a repo under a folder named
`node_modules`; a folder whose `.git` is a file, not a directory
(`echo "gitdir: /somewhere" > f/.git` — the submodule/worktree
stand-in, ALG-052 keys on the file form); a folder with read access
denied (Windows: `icacls "<folder>" /deny "${env:USERNAME}:(RD)"`;
Unix: `chmod 000 "<folder>"`); one repo already added by hand (S1).

**Do.**

1. Start a scan from the scratch root.
2. Watch the modal while it walks.
3. Let it finish; read the three groups; check "Add selected" with
   nothing checked, then add the checked "New" rows.
4. Rescan the same root; then scan a big root and Cancel mid-walk.

**Pass when.**

- The modal showed live progress — folders checked, repositories
  found, current path — plus the active depth and exclusions, an
  indeterminate bar, and a working Cancel (PRJ-002; the visible
  liveness is ALG-053's progress, humanly checked).
- The groups are right: the two plain repos and the nested repo in
  "New" (the scan continues inside a found repository, ALG-051); the
  by-hand repo in "Already added"; the `node_modules` repo skipped
  with reason "excluded"; the `.git`-file folder skipped with
  "submodule or worktree" (ALG-052); the denied folder skipped with
  the OS code and a fix hint — and the scan continued past it
  (PRJ-007). No duplicate rows.
- "Add selected" is disabled until at least one row is checked; the
  add is one transaction (PRJ-002): the added rows appear as columns
  with the defaults, color and label a manual add would have given
  them (S1), and the project settings modal names their origin as
  found by scan (PRJ-008).
- Canceling the big scan added nothing (PRJ-002's criterion), and the
  window stayed responsive during the walk (NFR-015).
- The rescan produced no duplicates — the earlier adds sit in
  "Already added".

**Restore.** Remove the deny / `chmod 755` the denied folder.

### S3. Creating an instruction and assigning it

Source: spec §8 S3; §7.3 LIB-002…016; §6.9 validation; DB-016
(`E_CONTENT_RESERVED`); OUT-010/011; DB-021/022.

Pending: "+ New instruction", the instruction editor (UI-003) and the
matrix cells — spec §16 M2/M3. The create/rename/lifecycle backend is
in place — the registered
`instruction_create`/`instruction_update`/`instruction_hide`/
`instruction_restore` (T-328) — so the dialog's rejections below
(the case duplicate, the marker line, the §6.9 field rules) answer
with the commands' own typed envelopes once the editor lands;
`assignment_set` (T-331) is registered, so the assignment step and its
DB-022 dirty flip have their backend in place.

**Prepare.** One project added, never synced. A project with no
assignments that has never been synced is not dirty (DB-021) — note
the pill shows zero. Note the target file's state
(`<repo>/AGENTS.md` should not exist yet under scheme A). One
instruction already in the library — the duplicate probe's target
(create a throwaway one first if the library is empty).

**Do.**

1. "+ New instruction". Try the rejections first: a name duplicating
   the existing instruction's, different case only; a text containing
   a line equal to `<!-- EUNOMIASYNC:BEGIN -->`.
2. Fix the fields (name, main-variant text — the main variant is
   named `Main`), pick a section (create a heading first if none),
   save.
3. In the same instruction's editor, try renaming the variant to 101
   characters — rejected inline; restore the name.
4. Assign it to the project: click the cell or press Space.
5. Without running sync, watch the sync panel and the target folder
   for a minute.

**Pass when.**

- Every invalid input is rejected inline and live (§6.9) — the case
  duplicate among visible instructions, the marker line with
  `E_CONTENT_RESERVED` ("Save rejected"), the 101-character variant
  name — and nothing is half-saved: an entity is never saved partly
  (§6.9's tail).
- The save lands in the DB immediately and no file changed:
  `<repo>/AGENTS.md` is still absent/untouched (OUT-010/011 — files
  change only with "Run sync").
- Within ~150 ms of the assignment the project turns dirty (DB-022):
  the pill counts it, the column header shows "unsynced changes"
  (MAIN-028), and the cell shows the assigned state — check mark plus
  the `Main` select (§11.2).
- S3's success line, verbatim in effect: the assignment is in the DB
  right away, files are not changed, the project became dirty.

### S3a. Heading hierarchy, deletion and library moves

Source: spec §7.3 LIB-030…034; §7.4 MAIN-015…023; §6.2/6.4;
ALG-060…062; DB-005/DB-011; §7.10 `heading_create`,
`heading_update`, `heading_delete`, `items_move`.

Pending: the library heading tools, drag-and-drop interactions and
confirmation dialog — spec §16 M2/M3. Their backend commands are
registered (T-330); the checks below become executable when those UI
surfaces land.

**Prepare.** One project added, never synced. Build this library order:
`## A`, instruction A1, `### B`, instruction B1, `## C`, instruction
C1. Note the target file's state (under scheme A, `<repo>/AGENTS.md`
should not exist yet).

**Do.**

1. Use the header "+" to add a root `##` and B's "+" to add a child.
   Rename a heading. Try an empty/201-character name and levels 1/7.
2. Change B's level, then use "Shift branch level +1" and "-1" to
   restore it. Try shifting an H2 down or an H6-containing branch up.
3. Delete B and confirm the dialog. Its instructions must remain.
4. Drag A1 onto C1 (right after it), then onto heading A (the end of
   A's section). Drag A's whole branch onto C (its last child), then
   onto the library header (the first root at level 2).
5. Try dropping A's branch onto A or B itself. Try a drop whose
   automatic alignment would move a heading past H6.
6. Restart the app and reread the library.

**Pass when.**

- Create/rename/level changes appear immediately; invalid names and
  levels are refused with `E_VALIDATION` and nothing is half-saved.
- A branch shift moves every heading in the branch by one and no
  instruction; a refused shift changes neither levels nor order.
- B is physically gone, B1 keeps its relative order among the
  instructions, and all positions are dense from 1 with no gaps.
- Instruction drops implement "after the row" and "at the section
  end"; branch drops keep nested order and align levels to target + 1
  or root level 2. Self/descendant drops and a level past H6 are
  `E_VALIDATION` with the library unchanged.
- The moved order and levels survive restart. Activity contains the
  heading/instruction journal entries; heading history contains no
  revisions. No project file changed before "Run sync".

### S3b. Instruction and variant history reads

Source: spec §7.3 LIB-026/027; §7.10 `revisions_list` / `revision_get`.

Pending: the instruction "History" panel and version view — spec §16 M2.
The two backend reads are registered (T-331); automated mock-IPC tests pin
their order and full-row content.

**Prepare.** One instruction with Main and a second variant. Rename the
instruction, edit Main, then soft-delete and restore the second variant so
its history contains create/delete/restore operations.

**Do.**

1. Open the instruction's History panel.
2. Read the version list, then open one instruction version and one
   variant version.

**Pass when.**

- The list names time, operation and entity for the instruction and every
  variant, including the soft-deleted variant; newest versions come first.
- Each real change has its own version and a no-change save adds none
  (LIB-026/D-16).
- The version view shows the selected revision's complete snapshot; merely
  listing or opening versions adds no Activity entry and changes no file.

### S4. Choosing a variant

Source: spec §8 S4; §7.3 LIB-017…025; §7.4 MAIN-001…005; D-02; D-13;
§11.2 cell states.

Pending: the matrix cell selects and the editor's variant list —
spec §16 M2/M3. The variant commands and `assignment_set` backend are
registered (T-329/T-331).

**Prepare.** One instruction, two projects, assigned to both (Main).
Add a second variant (a copy) and an empty one ("Add variant" →
empty).

**Do.**

1. In project 1's cell, open the select, pick the copy variant.
2. In project 2's cell, pick the empty variant.
3. From the editor's variant list, delete the copy variant — the
   confirmation names the number of affected projects (LIB-023) —
   while the matrix shows project 1's cell.
4. Try deleting the main variant, then the last remaining one.

**Pass when.**

- Step 1 changed only project 1: its select shows the picked variant
  while project 2's cell still shows `Main` (check after step 1,
  before step 2) — the other projects are not affected (S4's success
  line; MAIN-005; D-02's one variant per project).
- The empty variant is assignable (step 2): project 2's cell shows the
  dashed-border empty-variant state (§11.2; LIB-014/015). D-13's build
  half — an empty variant still counts as assigned and keeps its
  heading — lands in project 2's file at its next sync: the heading
  survives with no text block under it.
- After the deletion the assignments moved to the main variant
  (LIB-023), project 1's cell refreshed to the main variant, and the
  select no longer offers the dead variant (S4's Alt: the list
  refreshes).
- The main variant's Delete is disabled with the "Make another variant
  main first" hint (LIB-024) — which also keeps LIB-025's last-variant
  guard out of sight: a removable variant is never the last one, so
  that disable never fires through the UI.

### S5. Global edit and edit for a project

Source: spec §8 S5; §7.3 MAIN-007/LIB-018; §9.2 (the custom text is
the cell's text); §11.2.

Pending: the matrix "+" affordance, the per-project custom editor and
the library editor — spec §16 M2/M3. `assignment_set_custom` (T-331) is
registered; clearing custom text restores the active main, and bulk clear
leaves custom cells untouched.

**Prepare.** One instruction, two projects, both assigned (Main); one
sync done (a clean baseline) — or fold the file half into S7.

**Do.**

1. "Edit" from the library: change the main variant's text; save.
2. In project 2's cell, press "+": write per-project custom text
   (different from the variant text); save.
3. Open the library editor again and read the variant text.
4. From project 2's column menu, run "Clear column"; watch the custom
   cell.

**Pass when.**

- The library edit changed the variant for everyone: project 1's
  project turns dirty (the build hash changed, DB-021) but its cell is
  NOT marked "(changed)" — §11.2 marks assignment changes only
  (checkbox, variant, custom text), and a variant-text edit leaves the
  assignment untouched; the new text reaches the file at the next
  sync.
- The per-project editor stored the text in the assignment itself:
  project 2's select shows "Custom", is disabled, edit and delete
  icons appeared (§11.2; MAIN-007).
- The variant text in the library editor is untouched by step 2 — the
  shared variant did not change when edited from a project context
  (S5's success line).
- "Clear column" leaves the custom cell assigned — bulk operations do
  not touch cells with custom text (MAIN-007); deleting the custom
  text is its own confirmation (UI-010) and assigns the main variant
  back.
- After a sync, project 2's file carries the custom text and project
  1's the variant text (§9.2).

### S6. Adding an instruction to several projects at once

Source: spec §8 S6; §7.4 the bulk-operations row; UI-010; MAIN-007;
LOG-001.

Pending: the row checkboxes, the "N rows selected" chip and the
"Assign to projects…" dialog — spec §16 M3. `assignments_bulk` (T-331) is
registered and applies the whole row/column product in one transaction.

**Prepare.** Three projects: project 1 assigned (Main); project 2
carrying a different variant of the same instruction (S4's setup
reused); project 3 carrying per-project custom text (S5's setup
reused). One more instruction unassigned anywhere.

**Do.**

1. Select the first instruction's row; "Assign to projects…"; check
   all three; confirm.
2. Repeat with a bulk that affects 10 or more assignments (10 rows,
   or fewer rows against many projects) — UI-010's threshold.
3. Open the Activity window, filter type Assignments.

**Pass when.**

- Project 1 is not duplicated; project 2 keeps its variant and appears
  in the "already assigned" warning (S6's Alt: the variant is not
  replaced); project 3's custom cell is untouched (MAIN-007).
- The applied bulk is one transaction — no partially applied state is
  visible after the confirm (§7.4's AC: on an error nothing is
  applied; forcing the error half is fault-injection territory, not a
  manual check).
- The ≥10 bulk asked for a confirmation carrying the number of changes
  (UI-010).
- The Activity window shows ONE entry for the bulk with the number of
  changes (LOG-001), not one per project.

### S7. Sync

Source: spec §8 S7; §9.4; §7.5 OUT-006…020; ALG-031…033; D-24; §10
(the `E_*` table).

Pending: "Run sync", the progress region, the summary modal and the
per-column state icons — spec §16 M3/M5. (The backend `sync_run`, the
phase modules, the `sync://progress` event (T-119) and `sync_cancel`
(T-120) are registered; the panel — including any Cancel control that
would call `sync_cancel` mid-run — is not.)

The manual check the Cancel control gains when the panel lands
(T-337/D-234): cancel a run mid-flight (before the backups) and open
the Activity window — the run's bracket shows the `sync.run_started`
entry (info, ok) closed by the `sync.run_cancelled` entry (info,
skipped), the same window a backup failure closes with its
`sync.run_aborted` entry (S10 below); the run's summary answers
`aborted` with no target file changed. Until the control exists the
backend half is covered by the automated battery (the real-command
cancel test in `ipc_tests.rs` pins the entry's shape); this line
waits with the panel.

**§9.4's "Closing the window during a sync" — the dialog half is the
pending line (T-123).** The backend half is registered (the
`sync://close-confirmation` ask, the `sync_confirm_exit` command, the
write-in-flight wait and the exit); the CONFIRMATION DIALOG the ask
triggers is the unlanded UI wave's (UI-010's pattern, spec §16). The
manual check this scenario gains when the dialog lands: start a sync
over a slow volume (or a large project), close the window mid-run —
the close is intercepted and the dialog appears; confirm — the app
exits only after the current project's file and its row are written
(the run row becomes `interrupted` at the next startup, §9.4's
recovery paragraph); cancel — the window stays and the run finishes.
With no sync running, closing the window exits the app as usual (no
dialog). Until the dialog exists this line is NOT exercisable
end-to-end (the app exposes no dev-tools event hook —
`window.__TAURI__` is not injected, `withGlobalTauri` is off), so the
backend half stays covered by its automated battery and this line
waits with the UI wave.

**Prepare.** Four projects, each assigned an instruction of its own
(shared instructions would couple their dirty states): (a) a healthy
scheme-A repo; (b) a scheme-B repo whose target file exists with the
marker pair — `<!-- EUNOMIASYNC:BEGIN -->` and
`<!-- EUNOMIASYNC:END -->`, each on its own line (D-19; "Copy marker
block" in the project's settings is the ready source, PRJ-008) — plus
a hand-written line outside the markers, e.g. a title line above the
BEGIN (the app never creates (b)'s file or inserts the markers, §7.5
scheme B, and an unassigned scheme-B project would trip D-25's
empty-build confirmation instead of writing); (c) a healthy repo to be
made unavailable; (d) a healthy scheme-A repo whose instruction step 2
never touches, so it stays clean and shows the `unchanged` half. Give
(a), (c) and (d) a hand-written `AGENTS.md` each (any line of text):
scheme A fully replaces an existing file and backs the original up, so
run 1 has something to back up for every project (OUT-005; with no
existing file there is nothing to back up — phase 2 copies existing
targets only).

**Do.**

1. First sync, everything healthy: watch the button disable during the
   run, the live "k of N" region, then the summary.
2. Break things: edit (a)'s variant text; delete (b)'s closing marker
   line; rename (c)'s folder on disk. Run sync again.
3. Repair without editing content: restore (b)'s closing marker line
   byte for byte, rename (c)'s folder back. Third sync — nothing was
   edited, only repaired.

**Pass when.**

- Run 1: badge "success"; (b)'s file keeps everything outside the
  markers byte for byte, including BOM and line breaks, and gains the
  block between them (§7.5 scheme B; ALG-031); every scheme-A file
  ends with the creation marker line
  `<!-- Created by EunomiaSync YYYY-MM-DD HH:MM:SS <IANA name> <offset> -->`
  (ALG-032 — the time is taken once per run and is the same in all
  projects of the run); `<data dir>/backups/` holds a
  `<project_id>__<slug>` folder per written project, one copy each
  (BKP-001 — possible because every target file existed before the
  write).
- Run 2's summary: exactly one status per project (OUT-009) — (a)
  `written`; (b) `skipped` with `E_MARKERS_INVALID` and the fix hint
  (UI-011; §10: paste the block with "Copy marker block"); (c)
  `skipped` with `E_PROJECT_UNAVAILABLE`; (d) `unchanged`. The overall
  time does not advance — a partial run shows "Last attempt: partial"
  and the per-project times (§9.4 phase 4); (b) and (c) keep their
  run-1 times (OUT-020/DB-024).
- Run 3: every project `unchanged` — (a) and (d) trivially, (b) and
  (c) because a byte-identical repair matches under ALG-033 (the
  comparison drops only the creation marker line); no new backups
  anywhere (D-24; `sync.skip_identical` default on, §6.5); the files'
  bytes are unmoved. That is §10's "running it again after fixing the
  cause is safe", demonstrated.

### S9. Warning about unsynced changes

Source: spec §8 S9; DB-020…023; MAIN-024…028.

Pending: the sync panel's counter pill and the "Unsynced changes"
dialog — spec §16 M4 (the table maps S9 to it). (The dirty-recalc
backend — DB-022, `state://dirty` — is landed.)

**Prepare.** Two projects, one instruction assigned to both; one sync
done — the pill is empty, the dialog's empty state is "Everything is
synced" (MAIN-026).

**Do.**

1. Edit the instruction's variant text; save; watch the panel — the
   recalc runs 150 ms after the committed change (DB-022), so watch,
   don't stopwatch.
2. Click the pill.
3. Sync from the dialog's "Run sync" button; then dirty ONE project
   only — assign a second instruction to project 1 alone and edit its
   variant — and read the pill again.
4. Open the pill's dialog and run the sync from it (the state from
   step 3).

**Pass when.**

- The pill showed "2 projects have unsynced changes" after step 1 and
  the singular form after step 3 — both MAIN-026 wordings; the column
  headers show the "unsynced changes" state (MAIN-028).
- The dialog lists one row per dirty project with the project and its
  target file (scheme C shows `<STEM>.project.md`); the counter equals
  the number of dirty projects (S9's success line).
- The counter follows the DB-021 rule: an edit that cannot change a
  build — hiding an instruction with no assignments, for example —
  adds nothing to it.
- After the sync the pill is gone.
- Optional cross-check: changing a project's scheme or file name in
  its settings dirties it too (DB-023), and the old file under the
  previous name is not deleted. (The save command behind the modal is
  registered — backend-verified by tests — the modal itself is spec
  §16 M1.)

### S10. Backup failure

Source: spec §8 S10; §7.5 OUT-006/019; §7.6 BKP-005/009/010; §9.4
phase 2; §15.2 T-F1; §10 `E_BACKUP_FAILED`.

Pending: the summary's detailed-error presentation — spec §16 M5. (The
phase-2 abort semantics are landed backend and pinned by the run-level
T-F1 test in `sync_pipeline_tests.rs` over the injected copy seam,
T-127; the mechanics below are real-OS — an actually refused folder,
the production retrier, the log line.)

**Prepare.** Three scheme-A projects sharing one assigned instruction.
Sync once — it creates the target files, and phase 2 backs up existing
files only, so no backups exist yet. Edit the shared instruction's
variant text and sync again: each project now holds exactly one backup
copy under `<data dir>/backups/<project_id>__<slug>/`, and each file
on disk differs from its copy — BKP-010's dedup must not blur the run
below (a project whose file equals its last copy would dedup,
creating nothing for the abort sweep to delete). Then block ONLY the
second project's backup subfolder — Windows:
`icacls "<data dir>\backups\<id2>__<slug2>" /deny "${env:USERNAME}:(WD,AD)"`;
Unix: `chmod a-w "<data dir>/backups/<id2>__<slug2>"`. Projects 1 and
3 stay writable, so the failure lands mid-batch (T-F1's shape: the
second of three).

**Do.**

1. Edit the shared instruction's variant text — all three dirty.
2. Run sync.
3. Inspect the error, the three backup subfolders, the three target
   files' bytes, the per-project times, the file log.
4. Restore the rights (`icacls … /remove:d "${env:USERNAME}"` /
   `chmod u+w …`) and sync again.

**Pass when.**

- The run aborted with `E_BACKUP_FAILED` (§10). Project 2's error
  carries all the BKP-005 fields: the project name, the source file
  path, the backups folder path, the OS code or error type, an
  explanation of the cause, and a recommendation (UI-011's four
  questions).
- Project 1's subfolder holds exactly what it held before the run: the
  copy this run created was deleted (the copies of the run are
  discarded and removed, §9.4 phase 2 step 2), and copies predating
  the run are never touched.
- Project 3 was never attempted — the copy loop stops at the first
  error; its subfolder and target file are unmoved (`not_started`,
  phase 2 step 2).
- No project file changed: all three target files keep their pre-run
  bytes (T-F1: none of the three is changed; S10's success line). No
  project shows `written` or `unchanged`, and no last-sync time moved
  (OUT-020/DB-024).
- The file log carries the failure: `<data dir>/logs/eunomiasync.<date>.log`
  has an `error` line with code `E_BACKUP_FAILED`, the project's
  params and the OS code/kind/message (§10: every error is written to
  the log with its code, project, path, and OS code).
- The journal carries the spec's S10 "cancel entry" (T-124/D-223):
  the Activity window shows this run's bracket — the
  `sync.run_started` entry (info, ok) and the `sync.run_aborted`
  entry (error, error) naming the failing project, its Details
  carrying the code `E_BACKUP_FAILED`, the failed copy's source path
  and the OS code. No `backup.created` entries for this run — the
  swept copies never committed a record (an entry claiming a created
  backup for a born-deleted copy would lie).
- After step 4: all three `written`, one new copy per project named
  `AGENTS.md.<UTC YYYYMMDDTHHMMSSmmmZ>.<hash8>.bak` (BKP-001).

**Restore.** The rights restore in step 4; nothing else persists.
