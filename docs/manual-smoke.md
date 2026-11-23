# Manual Smoke Checklist

Checks that automation cannot prove. The CI legs prove compilation and
unit behavior; what a real OS refuses to let a test do — holding a
locked file, denying directory writes across a launch, watching a
multi-megabyte session rotate — is verified here, by hand, on a real
machine. Every item names its source (a DECISIONS.md entry and/or
ARCHITECTURE.md section); the pass criteria restate those sources and
add nothing.

The current items are the three Windows GUI smoke checks from D-143
(the file log's size half, T-255): what the unit suite cannot reach —
the real clock, the production worker-thread path of the degradation
warn, and Windows' GUI stderr and file locking. Items for other OSes
and other subsystems may join below.

This file is the in-repo home for manual smoke checks. It is not the
spec's release checklist: spec §15.5 conditions the release on "the
manual checklist for scenarios S1–S7, S9, S10 is done on each OS"
(spec §15.1) — those scenario runs are defined in the spec and are not
reproduced here. A release run walks both.

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
