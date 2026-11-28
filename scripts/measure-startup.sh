#!/usr/bin/env bash
# §12.2's startup and memory rows on set L — the repeatable
# measurement harness (T-140; DECISIONS.md's entry of that task).
#
# What it measures: the backend process running the REAL startup
# sequence over the generated set-L world — the backend's
# synchronous startup share (the timing) and the process's RSS
# checkpoints (baseline, ready, settled, peak). The measurement
# itself is the Linux test of src-tauri/src/startup_tests.rs
# (memory_row); this script builds and runs it, parses the reported
# numbers, and prints the verdict against the spec's budgets.
#
# Usage:
#   scripts/measure-startup.sh
#
# Budget policy (the D-239/D-135 readings): the dev-profile run
# checks the startup row at the documented CI margin (2 s x 2 — the
# debug profile on a loaded machine is several times the release
# cost) and the memory row at the documented 400 MiB reading of the
# spec's 400 MB. The reading carries a ~60 MB harness + mock-runtime
# floor the real process does not, and the real runtime's own share
# is outside the measurement — a necessary, not sufficient, check;
# the whole-app verdict is the manual smoke checklist's §12.2 item.
#
# Why no --release mode: a release-profile test build of this crate
# is impossible by design — eunomia-fs's fault-injection feature
# (on for eunomia-sync's test builds via the D-226 dev-dependency
# edge) fails the build with a compile_error in any
# non-debug-assertions unit (D-101's deliberate gate). The
# release-profile verdict of both rows is therefore NOT a cargo-test
# rerun: it is the manual smoke checklist's §12.2 item
# (docs/manual-smoke.md) — the real app, webview included, on the
# reference machine, which is D-135's standing "reference machine,
# honestly" reading.
#
# Platform: Linux only (the measurement reads /proc; the test itself
# is cfg-gated to Linux). On macOS/Windows the rows are the release
# checklist's manual measurements.
#
# Exit status: 0 — both numbers within the budgets (and the test's
# own asserts green, which `cargo test`'s exit code proves); 1 — a
# script-level failure (bad arguments, the memory floor, a missing
# report line, or a budget breach); anything else — cargo's own exit
# status, propagated verbatim (the measurement test failed: a build
# error, a failed assert, a hung child killed by the parent's
# deadline).

set -euo pipefail

STARTUP_BUDGET_MS=4000 # 2 s x the documented CI margin (D-239)
RSS_BUDGET_KB=409600   # §12.2: "no more than 400 MB", read as 400 MiB (/proc kB)

if [[ $# -gt 0 ]]; then
  echo "usage: $0 (no arguments; see the header for the release-verdict story)" >&2
  exit 1
fi

# The repo's cargo resource policy: limited parallelism unless the
# caller already capped it (AGENTS: CARGO_BUILD_JOBS=2).
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

# The same policy's memory floor: a full single-crate test build
# needs headroom; stop before starting it rather than mid-way.
AVAILABLE_MB=$(free -m | awk '/^Mem:/ {print $7}')
if (( AVAILABLE_MB < 6000 )); then
  echo "FAIL: only ${AVAILABLE_MB} MB of memory available — the policy floor is 6 GB." >&2
  exit 1
fi

TEST=startup_tests::memory_row::startup_rss_on_set_l_fits_the_memory_budget

echo "[measure-startup] running the measurement test (dev profile, jobs=${CARGO_BUILD_JOBS})..."
echo

# `cargo test`'s exit status is the gate the script trusts first: the
# test's own asserts (the budgets, the anti-stub cross-check, the
# reality of the run) all fire inside it — never a grepped "ok".
set +e
OUTPUT=$(cargo test -p eunomia-sync --lib "$TEST" -- --exact --nocapture 2>&1)
STATUS=$?
set -e
echo "$OUTPUT"

if (( STATUS != 0 )); then
  echo "[measure-startup] FAIL: the measurement test failed (cargo exit ${STATUS})." >&2
  exit "$STATUS"
fi

# The report line the parent test prints after its asserts.
REPORT=$(grep -o "EUNOMIA-STARTUP-REPORT elapsed_ms=.*" <<<"$OUTPUT" | tail -n 1 || true)
if [[ -z "$REPORT" ]]; then
  echo "[measure-startup] FAIL: the measurement did not print its report line." >&2
  exit 1
fi

field() {
  grep -oE "$1=[0-9]+" <<<"$REPORT" | head -n 1 | cut -d= -f2
}
ELAPSED_MS=$(field elapsed_ms)
BASELINE_KB=$(field baseline_kb)
READY_KB=$(field ready_kb)
SETTLED_KB=$(field settled_kb)
PEAK_KB=$(field peak_kb)

echo
echo "[measure-startup] §12.2 on set L (dev profile; the release verdict is the"
echo "manual smoke checklist's §12.2 item):"
echo "  startup (backend's synchronous share): ${ELAPSED_MS} ms   (budget ${STARTUP_BUDGET_MS} ms)"
echo "  RSS baseline (harness floor):          $((BASELINE_KB / 1024)) MB"
echo "  RSS at ready (sequence returned):      $((READY_KB / 1024)) MB"
echo "  RSS settled (first pass + services):   $((SETTLED_KB / 1024)) MB"
echo "  RSS peak (VmHWM):                      $((PEAK_KB / 1024)) MB   (budget $((RSS_BUDGET_KB / 1024)) MB)"

BREACH=0
if (( ELAPSED_MS > STARTUP_BUDGET_MS )); then
  echo "  FAIL: the startup share exceeds its budget." >&2
  BREACH=1
fi
if (( PEAK_KB > RSS_BUDGET_KB )); then
  echo "  FAIL: the peak RSS exceeds the memory budget." >&2
  BREACH=1
fi
if (( BREACH == 0 )); then
  echo "  OK: both numbers within their budgets."
fi

exit "$BREACH"
