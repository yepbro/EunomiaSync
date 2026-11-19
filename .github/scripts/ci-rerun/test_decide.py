#!/usr/bin/env python3
"""Unit tests for the CI auto-rerun decision function (T-242).

Runner choice: the standard library's unittest - pytest-like ergonomics
with zero dependencies, so the very same command verifies the logic
locally and as a workflow step on the runner:

    python3 .github/scripts/ci-rerun/test_decide.py

The fixtures under fixtures/ are shaped exactly like the responses of
GET /repos/yepbro/EunomiaSync/actions/runs/<id>/jobs (and of its
per-attempt form /runs/<id>/attempts/<n>/jobs) and carry the job sets
of real runs (ids, names and timestamps included) with conclusions
adjusted to the scenario each fixture isolates:

    cancelled-only            the end-state of run 44584364443 attempt 2
                              (build (macos-15) cancelled, gate green);
                              the cap-exhausted drill reuses this file
                              with --rerun-count 2 (the cap is state,
                              not a job shape - no duplicate fixture)
    has-failure               run 44531625527 as it actually failed
    derived-gate-failure      run 44584364443 attempt 1 as it actually
                              happened: test-matrix (macos-15) cancelled,
                              "test" gate failed, zero real failures
    mixed-failure-and-cancels run 44578261527 exactly as it happened
    timing-failure            the cancelled-only job set with the leg's
                              conclusion swapped to timing_failure
"""

import copy
import json
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import decide  # noqa: E402

FIXTURES = pathlib.Path(__file__).resolve().parent / "fixtures"


def load(fixture):
    data = json.loads((FIXTURES / (fixture + ".json")).read_text(encoding="utf-8"))
    assert isinstance(data.get("jobs"), list)
    return data["jobs"]


class DecideTests(unittest.TestCase):
    # (a) required fixture: successes + several cancellations -> rerun.
    def test_cancelled_only_reruns(self):
        d = decide.decide(load("cancelled-only"), rerun_count=0)
        self.assertEqual(d.action, "rerun")
        self.assertIn("build (macos-15)", d.reason)

    # (b) required fixture: a failure anywhere -> noop.
    def test_has_failure_noops(self):
        d = decide.decide(load("has-failure"), rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("test-matrix (windows-latest)", d.reason)
        self.assertIn("never auto-rerun", d.reason)

    # (c) required fixture: cap exhausted -> noop.
    def test_cap_exhausted_noops(self):
        d = decide.decide(load("cancelled-only"), rerun_count=2)
        self.assertEqual(d.action, "noop")
        self.assertIn("cap reached", d.reason)

    def test_cap_not_yet_exhausted_still_reruns(self):
        d = decide.decide(load("cancelled-only"), rerun_count=1)
        self.assertEqual(d.action, "rerun")
        self.assertIn("1/2", d.reason)

    # The roll-up gate "test" fails whenever a matrix leg is cancelled,
    # even with zero real failures - that derived failure must not block
    # the heal.
    def test_derived_gate_failure_reruns(self):
        d = decide.decide(load("derived-gate-failure"), rerun_count=0)
        self.assertEqual(d.action, "rerun")
        self.assertIn("excused as derived", d.reason)

    # The carve-out must fail closed: a gate failure with no cancelled
    # matrix leg is treated as a real failure.
    def test_gate_failure_without_cancelled_leg_noops(self):
        jobs = load("derived-gate-failure")
        for job in jobs:
            if job["name"].startswith("test-matrix ("):
                job["conclusion"] = "success"
        d = decide.decide(jobs, rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("treating the gate failure as real", d.reason)

    # The carve-out's twin fail-closed path (D-078): if ci.yml renames
    # the gate, the unknown gate failure must become a hard failure.
    def test_gate_renamed_makes_gate_failure_real(self):
        jobs = load("derived-gate-failure")
        for job in jobs:
            if job["name"] == "test":
                job["name"] = "unit-gate"
        d = decide.decide(jobs, rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("real failure(s) present: unit-gate", d.reason)

    # ...and if the matrix job id is renamed, the gate's legs are no
    # longer recognizable, so its failure must not be excused either.
    def test_matrix_legs_renamed_makes_gate_failure_real(self):
        jobs = load("derived-gate-failure")
        for job in jobs:
            if job["name"].startswith("test-matrix ("):
                job["name"] = job["name"].replace("test-matrix", "matrix", 1)
        d = decide.decide(jobs, rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("treating the gate failure as real", d.reason)
        self.assertIn("legs: none", d.reason)

    # Real-world shape of run 44578261527: cancellations AND a real
    # failure in the same run -> cancellations must not smooth it over.
    def test_mixed_failure_and_cancels_noops(self):
        d = decide.decide(load("mixed-failure-and-cancels"), rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("test-matrix (windows-latest)", d.reason)

    def test_timing_failure_noops(self):
        d = decide.decide(load("timing-failure"), rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("non-standard conclusion", d.reason)

    def test_skipped_conclusion_noops(self):
        jobs = load("cancelled-only")
        for job in jobs:
            if job["name"] == "e2e":
                job["conclusion"] = "skipped"
        d = decide.decide(jobs, rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("non-standard conclusion", d.reason)

    def test_incomplete_job_noops(self):
        jobs = load("cancelled-only")
        for job in jobs:
            if job["name"] == "e2e":
                job["status"] = "in_progress"
                job["conclusion"] = None
        d = decide.decide(jobs, rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("non-completed job", d.reason)

    def test_all_success_noops(self):
        jobs = load("cancelled-only")
        for job in jobs:
            job["conclusion"] = "success"
        d = decide.decide(jobs, rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("nothing to heal", d.reason)

    def test_empty_job_list_noops(self):
        d = decide.decide([], rerun_count=0)
        self.assertEqual(d.action, "noop")
        self.assertIn("no jobs reported", d.reason)


class CliTests(unittest.TestCase):
    """The CLI contract the workflow relies on (ACTION=/REASON= lines)."""

    def _run(self, argv):
        import contextlib
        import io

        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = decide.main(argv)
        return code, out.getvalue()

    def test_cli_prints_action_and_reason(self):
        fixture = str(FIXTURES / "cancelled-only.json")
        code, out = self._run(["--jobs", fixture, "--rerun-count", "0"])
        self.assertEqual(code, 0)
        self.assertIn("ACTION=rerun\n", out)
        self.assertTrue(out.split("REASON=", 1)[1].strip())

    def test_cli_accepts_bare_list(self):
        jobs = json.loads((FIXTURES / "has-failure.json").read_text())["jobs"]
        import os
        import tempfile

        fd, path = tempfile.mkstemp(suffix=".json")
        self.addCleanup(os.unlink, path)
        with os.fdopen(fd, "w") as fh:
            json.dump(jobs, fh)
        code, out = self._run(["--jobs", path, "--rerun-count", "0"])
        self.assertEqual(code, 0)
        self.assertIn("ACTION=noop\n", out)

    def test_cli_truncated_input_exits_two_without_action(self):
        data = copy.deepcopy(json.loads((FIXTURES / "cancelled-only.json").read_text()))
        data["jobs"] = data["jobs"][:3]  # keep total_count=12 -> truncated
        import os
        import tempfile

        fd, path = tempfile.mkstemp(suffix=".json")
        self.addCleanup(os.unlink, path)
        with os.fdopen(fd, "w") as fh:
            json.dump(data, fh)
        code, out = self._run(["--jobs", path, "--rerun-count", "0"])
        self.assertEqual(code, 2)
        self.assertNotIn("ACTION=", out)

    def test_cli_missing_file_exits_two(self):
        code, out = self._run(["--jobs", "/nonexistent/jobs.json"])
        self.assertEqual(code, 2)
        self.assertNotIn("ACTION=", out)


if __name__ == "__main__":
    unittest.main(verbosity=2)
