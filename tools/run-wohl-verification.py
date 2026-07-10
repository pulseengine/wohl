#!/usr/bin/env python3
"""
Rivet-driven verification gate for wohl.

Extract and run the executable verification steps recorded on wohl's rivet
verification artifacts, so the right side of the V is *driven*, not narrated
(pulseengine/wohl#50, Track E; mirrors relay's run-falcon-verification.py).

  1. List verification artifacts (unit-verification + sw-verification).
  2. For each, `rivet get` the full JSON and read fields.steps[].run.
  3. Skip commands whose shape needs infra the ubuntu-latest gate lacks
     (cargo kani, bazel/Verus, spar, wasmtime, sigil, developer abs paths) —
     those have their OWN dedicated CI jobs; the gate runs the rest.
  4. Run each remaining command; a `cargo test -p X <name>` that names a test
     must actually run >=1 test (renamed/removed test = evidence drift).
  5. Aggregate pass/fail per artifact; emit a Markdown PR summary.

Usage:
    python3 tools/run-wohl-verification.py
    python3 tools/run-wohl-verification.py --dry-run
    python3 tools/run-wohl-verification.py --markdown
    python3 tools/run-wohl-verification.py --filter '(has-tag "matter")'
"""

import argparse
import json
import re
import shlex
import subprocess
import sys
import time
from typing import Any

# Command shapes that need infra the ubuntu-latest gate doesn't provide.
# Each of these already has a dedicated CI job (Kani matrix, Verus bazel job,
# bazel-build, sigil-attest) — the gate must not re-run or fail on them.
BENCH_PATTERNS = [
    re.compile(r"\bcargo\s+kani\b"),               # Kani matrix job
    re.compile(r"\bbazel\s+(test|build|run)\b"),   # Verus / bazel-build jobs
    re.compile(r"\bspar\s+\w"),                     # spar not on the gate runner
    re.compile(r"\bwasmtime\b"),                    # composed-wasm exec (bazel job)
    re.compile(r"\bcargo\s+component\b"),
    re.compile(r"--target\s+wasm32-"),
    re.compile(r"target/wasm32-"),
    re.compile(r"\bsigil\b"),                       # sigil-attest job
    re.compile(r"\bgh\s+attestation\s+verify\b"),   # needs sigstore TUF init
    re.compile(r"\bcargo\s+fuzz\b"),                # cargo-fuzz smoke job
    re.compile(r"/Users/[^/]+/"),                    # developer-machine abs path
    re.compile(r"^\s*cd\s+~"),                       # non-portable tilde path
]

TYPES = ["unit-verification", "sw-verification"]


def is_bench_only(cmd: str) -> bool:
    return any(p.search(cmd) for p in BENCH_PATTERNS)


def rivet_list(artifact_type: str, filter_expr: str | None) -> list[str]:
    cmd = ["rivet", "list", "--type", artifact_type, "--format", "json"]
    if filter_expr:
        cmd += ["--filter", filter_expr]
    data = json.loads(subprocess.check_output(cmd))
    # Exclude cross-repo (prefixed) ids like `relay:UV-CCSDS-001` — the `relay`
    # rivet external federates its whole graph in; this gate runs only wohl's
    # own verification, not the supplier's.
    return [a["id"] for a in data.get("artifacts", []) if ":" not in a["id"]]


def rivet_get(artifact_id: str) -> dict[str, Any]:
    return json.loads(subprocess.check_output(
        ["rivet", "get", artifact_id, "--format", "json"]))


def cargo_test_names_a_filter(cmd: str) -> bool:
    """True if `cmd` is `cargo test ...` naming a specific test filter — a bare
    token that isn't a flag, `-p <pkg>`, or a `--` harness arg."""
    try:
        toks = shlex.split(cmd)
    except ValueError:
        return False
    if toks[:2] != ["cargo", "test"]:
        return False
    i = 2
    while i < len(toks):
        t = toks[i]
        if t == "--":
            return False
        if t in ("-p", "--package", "--manifest-path", "--features"):
            i += 2
            continue
        if t.startswith("-"):
            i += 1
            continue
        return True
    return False


def cargo_tests_passed(output: str) -> int:
    return sum(int(m) for m in re.findall(r"test result: ok\. (\d+) passed", output))


def run_steps(artifact: dict[str, Any], dry_run: bool) -> tuple[bool, list[dict]]:
    aid = artifact["id"]
    steps = artifact.get("fields", {}).get("steps") or []
    results = []
    artifact_pass = True
    for i, step in enumerate(steps):
        cmd = step["run"]
        if is_bench_only(cmd):
            print(f"  [ skip-bench-only] {aid}: {cmd}")
            results.append({"cmd": cmd, "pass": True, "skipped": True, "rc": 0, "duration": 0.0})
            continue
        if dry_run:
            print(f"  [dry-run] {aid} step {i+1}: {cmd}")
            results.append({"cmd": cmd, "pass": True, "skipped": False, "rc": 0, "duration": 0.0})
            continue
        start = time.monotonic()
        proc = subprocess.run(cmd, shell=True, capture_output=True, text=True)
        rc = proc.returncode
        duration = time.monotonic() - start
        passed = rc == 0
        note = ""
        if passed and cargo_test_names_a_filter(cmd) and cargo_tests_passed(proc.stdout) == 0:
            passed = False
            note = " — named test ran 0 (renamed/removed? evidence drift)"
        artifact_pass = artifact_pass and passed
        status = "PASS" if passed else (f"FAIL (rc={rc})" if rc != 0 else "FAIL (0 tests)")
        print(f"  [{status:>14}] ({duration:6.2f}s) {aid}: {cmd}{note}")
        if not passed and (proc.stdout or proc.stderr):
            for line in (proc.stdout + proc.stderr).strip().splitlines()[-15:]:
                print(f"        | {line}")
        results.append({"cmd": cmd, "pass": passed, "skipped": False, "rc": rc, "duration": duration})
    if not steps:
        print(f"  [   skip-no-steps] {aid}: (no steps defined)")
    return artifact_pass, results


def emit_markdown(report: list[dict]) -> str:
    total = len(report)
    skipped_no_steps = sum(1 for r in report if not r["steps"])
    passed = 0
    bench_only = 0
    for r in report:
        executed = [s for s in r["steps"] if not s.get("skipped")]
        if r["steps"] and not executed:
            bench_only += 1
        elif executed and r["pass"]:
            passed += 1
    failed = total - passed - skipped_no_steps - bench_only
    icon = "✅" if failed == 0 else "❌"
    lines = [
        f"## {icon} Rivet verification gate — wohl",
        "",
        f"**{passed}/{total - skipped_no_steps - bench_only} passed** "
        f"(of artifacts with runnable steps)",
        "",
        "| | count |",
        "|---|---|",
        f"| Passed | {passed} |",
        f"| Failed | {failed} |",
        f"| Skipped (bench-only — own CI job / needs infra) | {bench_only} |",
        f"| No executable steps yet | {skipped_no_steps} |",
        "",
    ]
    if failed:
        lines.append("### Failed artifacts")
        for r in report:
            fails = [s for s in r["steps"] if not s.get("skipped") and not s["pass"]]
            if fails:
                lines.append(f"- `{r['id']}` — {r['title']}")
                for s in fails:
                    lines.append(f"  - `{s['cmd']}` (rc={s['rc']})")
        lines.append("")
    lines.append(
        "Source of truth: `fields.steps` on `artifacts/verification/*.yaml`. "
        "Coverage grows as steps are added to more artifacts (wohl#50, Track E)."
    )
    return "\n".join(lines)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--filter", default=None,
                   help="rivet S-expression filter (default: all verification artifacts)")
    p.add_argument("--dry-run", action="store_true", help="print commands without executing")
    p.add_argument("--markdown", action="store_true", help="emit Markdown summary (for PR comment)")
    args = p.parse_args()

    ids: list[str] = []
    for t in TYPES:
        ids += rivet_list(t, args.filter)
    print(f"# wohl verification gate — {len(ids)} artifact(s): {', '.join(ids)}\n")

    report = []
    overall_pass = True
    for aid in ids:
        a = rivet_get(aid)
        ok, step_results = run_steps(a, args.dry_run)
        overall_pass = overall_pass and ok
        report.append({"id": aid, "title": a.get("title", ""), "pass": ok, "steps": step_results})

    if args.markdown:
        print("\n" + emit_markdown(report))
    return 0 if overall_pass else 1


if __name__ == "__main__":
    sys.exit(main())
