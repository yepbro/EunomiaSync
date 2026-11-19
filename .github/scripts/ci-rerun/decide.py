#!/usr/bin/env python3
"""Pure decision logic for the CI auto-rerun healer (T-242).

The function `decide(jobs, rerun_count, cap)` decides whether a COMPLETED
CI run whose job list is `jobs` should be re-run automatically. It is the
only place where that policy lives; the workflow (.github/workflows/ci-rerun.yml)
only fetches data, calls this, and acts on the printed ACTION.

Policy (strict cancelled-only discriminator):

* A run is healable only if EVERY non-success job has conclusion
  "cancelled" and at least one job is cancelled. Infrastructure capacity
  cancellations look exactly like that (macos arm64 legs; precedents:
  runs 44584364443, 44123571125, 44147164960).
* ANY other outcome - failure, timing_failure, startup_failure, skipped,
  action_required, or a job that is not completed yet - makes the healer
  no-op. Red belongs to a human; the healer never masks it.
* One provably-safe exception ("derived gate failure"): the roll-up gate
  job named "test" (ci.yml: `if: always()` + `test "$MATRIX_RESULT" =
  "success"`) concludes "failure" whenever a test-matrix leg was
  cancelled, even when no leg actually failed - the gate failure is then
  an artifact of the cancellation, not of the code. That failure is
  excused IF AND ONLY IF every "test-matrix (...)" leg is success or
  cancelled with at least one cancelled leg. Any real leg failure is
  still a hard failure (blocked below), so the exception cannot mask
  red.
* The cap: no more than `cap` (default 2) auto-reruns per head SHA. The
  workflow counts marker artifacts `ci-rerun-marker-<full sha>` before
  calling this function and passes the count in `rerun_count`.

Note that the RUN-level conclusion is deliberately ignored: GitHub
reports an infra-cancelled run sometimes as "failure" (44123571125) and
sometimes as "cancelled" (44147164960), so only per-job conclusions are
trustworthy.

Usage (CLI): decide.py --jobs <file|-> [--rerun-count N] [--cap 2]
Input: the JSON response of GET /actions/runs/<id>/jobs ({"total_count",
"jobs": [...]}) or a bare list of jobs. If total_count disagrees with the
list length the input is treated as truncated -> noop (fail closed).
Output (stdout): "ACTION=rerun|noop" then "REASON=<one line>"; exit 0
both ways - the decision itself is not an error. Bad input exits 2.
"""

import argparse
import json
import sys
from collections import namedtuple

# ci.yml job names this policy knows about. If ci.yml renames either,
# the carve-out fails CLOSED: an unknown gate failure is a hard failure.
GATE_JOB_NAME = "test"
MATRIX_LEG_PREFIX = "test-matrix ("

# Conclusions that this healer is allowed to reason about at all.
STANDARD_CONCLUSIONS = ("success", "failure", "cancelled")

CAP_DEFAULT = 2

Decision = namedtuple("Decision", ["action", "reason"])


def _noop(reason):
    return Decision("noop", reason)


def _rerun(reason):
    return Decision("rerun", reason)


def decide(jobs, rerun_count=0, cap=CAP_DEFAULT):
    """Return Decision(action="rerun"|"noop", reason=...) for a job list.

    `jobs` is the `jobs` array of GET /repos/<repo>/actions/runs/<id>/jobs:
    a list of objects with at least "name", "status" and "conclusion".
    """
    if not isinstance(jobs, list):
        raise TypeError("jobs must be a list of job objects")
    if not jobs:
        return _noop("no jobs reported for the completed run - refusing to act")

    def name_of(job):
        return str(job.get("name", "<unnamed>"))

    def conclusion_of(job):
        return job.get("conclusion")

    # 1. The run must be fully judged: a job that has not completed makes
    #    any decision premature.
    incomplete = [name_of(j) for j in jobs if j.get("status") != "completed"]
    if incomplete:
        return _noop(
            "non-completed job(s) present: "
            + ", ".join(incomplete)
            + " - refusing to judge an unfinished run"
        )

    # 2. Only conclusions with unambiguous meaning may be healed. This
    #    blocks timing_failure, startup_failure, skipped, action_required
    #    and anything else that is not a plain success/failure/cancelled.
    foreign = sorted(
        {
            "%s=%s" % (name_of(j), conclusion_of(j))
            for j in jobs
            if conclusion_of(j) not in STANDARD_CONCLUSIONS
        }
    )
    if foreign:
        return _noop(
            "non-standard conclusion(s): "
            + ", ".join(foreign)
            + " - only plain cancellations are healable"
        )

    # 3. Any real failure blocks the heal (red = human).
    hard_failures = [
        name_of(j)
        for j in jobs
        if conclusion_of(j) == "failure" and name_of(j) != GATE_JOB_NAME
    ]
    if hard_failures:
        return _noop(
            "real failure(s) present: "
            + ", ".join(sorted(hard_failures))
            + " - failures are never auto-rerun"
        )

    # 4. Derived-gate-failure carve-out (see module docstring). Fails
    #    closed: no legs found, or any leg not success/cancelled, or no
    #    cancelled leg at all -> the gate failure counts as real.
    gate_excused = False
    gate = next((j for j in jobs if name_of(j) == GATE_JOB_NAME), None)
    if gate is not None and conclusion_of(gate) == "failure":
        legs = [j for j in jobs if name_of(j).startswith(MATRIX_LEG_PREFIX)]
        leg_conclusions = {conclusion_of(j) for j in legs}
        if legs and leg_conclusions <= {"success", "cancelled"} and "cancelled" in leg_conclusions:
            gate_excused = True
        else:
            return _noop(
                "gate job '%s' failed without cancelled matrix legs - "
                "treating the gate failure as real (legs: %s)"
                % (GATE_JOB_NAME, ", ".join(sorted("%s=%s" % (name_of(l), conclusion_of(l)) for l in legs)) or "none")
            )

    # 5. There must be something to heal.
    cancelled = sorted(name_of(j) for j in jobs if conclusion_of(j) == "cancelled")
    if not cancelled:
        return _noop("nothing to heal: no cancelled jobs")

    # 6. The cap.
    if rerun_count >= cap:
        return _noop(
            "cap reached: %d auto-rerun(s) already recorded for this head SHA (cap %d)"
            % (rerun_count, cap)
        )

    reason = "infra-only cancellations: " + ", ".join(cancelled)
    if gate_excused:
        reason += (
            "; gate '%s' failure excused as derived from cancelled matrix legs"
            % GATE_JOB_NAME
        )
    reason += "; auto-reruns of this SHA so far: %d/%d" % (rerun_count, cap)
    return _rerun(reason)


def _extract_jobs(data):
    """Accept the API response object or a bare jobs list; fail closed on truncation."""
    if isinstance(data, dict):
        total = data.get("total_count")
        jobs = data.get("jobs")
        if not isinstance(jobs, list):
            raise ValueError("input object has no 'jobs' list")
        if isinstance(total, int) and total != len(jobs):
            raise ValueError(
                "job list truncated: got %d of %d jobs (the jobs fetch is "
                "single-page, per_page=100 is the API maximum - paginate and "
                "merge pages in the collect step if CI ever grows past 100 jobs)"
                % (len(jobs), total)
            )
        return jobs
    if isinstance(data, list):
        return data
    raise ValueError("input is neither a jobs list nor an API response object")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--jobs", required=True, help="path to the jobs JSON file, or - for stdin")
    parser.add_argument("--rerun-count", type=int, default=0, help="auto-reruns already recorded for this head SHA")
    parser.add_argument("--cap", type=int, default=CAP_DEFAULT, help="max auto-reruns per head SHA")
    args = parser.parse_args(argv)

    try:
        if args.jobs == "-":
            raw = sys.stdin.read()
        else:
            with open(args.jobs, encoding="utf-8") as fh:
                raw = fh.read()
        data = json.loads(raw)
        jobs = _extract_jobs(data)
        decision = decide(jobs, rerun_count=args.rerun_count, cap=args.cap)
    except (OSError, ValueError, TypeError) as exc:
        # Input errors are loud: no ACTION line is printed, the exit code
        # is 2, and the workflow step fails visibly instead of silently
        # "deciding" noop on data it could not even read.
        print("decide.py: input rejected: %s" % exc, file=sys.stderr)
        return 2

    print("ACTION=%s" % decision.action)
    print("REASON=%s" % decision.reason)
    return 0


if __name__ == "__main__":
    sys.exit(main())
