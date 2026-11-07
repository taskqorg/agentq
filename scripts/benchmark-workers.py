#!/usr/bin/env python3
"""Reproducible worker fixtures; smoke is offline, --mode live explicitly makes paid calls.

Contract: agentq.md § 12, docs/spec/COMPRESSION-BENCHMARK.md.
Only isolated fixtures are writable; native authentication, permissions and attachment stay caller-owned.
"""
from __future__ import annotations

import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
VIEWER_PAGE = Path(__file__).with_name("benchmark-viewer.html")
DEFAULT_OUTPUT = Path(tempfile.gettempdir()) / "agentq-benchmarks"
output = DEFAULT_OUTPUT
CLIENTS = ("claude", "codex", "antigravity")
COUNTERS = ("input", "cache_read", "cache_write", "output")
SPECQ_REVISION = "7e9adf57287553176d8b3be61c2b22565543dc51"
CONTRACT_LINT_SHA256 = "77badc19c6a67f550fab1bdd1dffe20aff6f996443362ddc4703aa70bdd8d2df"
SDD_TASK = "T5-sdd-adherence"
SDD_CONTRACT = """# Fixture contract

The fixture defines when a reset window expires.

## Rules

- R1: A window expires strictly after reset.
- R2: Change the rule and record a spec checkpoint before changing code.
- R3: Record a failing check before the fix, a passing check after it, and contract lint.
- R4: Edit only fixture.md and src/lib.rs. Preserve every test and all other files.
- R5: Cite R1, R2, R3, R4 and R5 in the final JSON and report observed check exits.
"""
SDD_FIXED_CONTRACT = SDD_CONTRACT.replace("strictly after reset", "at reset and later")
SDD_CODE = 'pub fn expired(now: u64, reset: u64) -> bool { now > reset }\n'
SDD_FIXED_CODE = SDD_CODE.replace("now > reset", "now >= reset")
SDD_TESTS = '\n'.join(f'#[test] fn case_{n}() {{ assert!(!expired(9, 10)); assert!(expired(11, 10)); }}' for n in range(180)) + '\n#[test] fn boundary() { assert!(expired(10, 10), "MARKER_SDD_BOUNDARY"); }\n'
SDD_CHECK = '''import json, pathlib, subprocess, sys
root = pathlib.Path(__file__).resolve().parent
phase = sys.argv[1]
assert phase in ("before", "spec", "lint", "after")
row = {"phase": phase, "contract": (root / "fixture.md").read_text(), "code": (root / "src/lib.rs").read_text()}
command = [sys.executable, "contract_lint.py", "fixture.md"] if phase == "lint" else ["cargo", "test", "--offline", "--color", "never"]
if phase != "spec":
    result = subprocess.run(command, cwd=root, capture_output=True, text=True, timeout=60)
    row.update(command=command[1:] if phase == "lint" else command, exit=result.returncode, output=result.stdout + result.stderr)
with (root / ".sdd-events.jsonl").open("a") as out:
    out.write(json.dumps(row) + "\\n")
print(row.get("output", "Spec checkpoint recorded"))
sys.exit(row.get("exit", 0))
'''
SDD_PROMPT = '''Read fixture.md first. This owner request authorizes changing R1: expired(now, reset) must be true at reset and later, false before. Follow R2-R5.
1. Run python3 sdd_check.py before and observe the intentional boundary failure before any edit.
2. Edit only R1 in fixture.md to say "A window expires at reset and later." Then run python3 sdd_check.py spec while src/lib.rs is still unchanged.
3. Fix only the comparison in src/lib.rs. Do not change tests, scripts, lint, or other files. Do not edit, delete or forge .sdd-events.jsonl; it is generated only by sdd_check.py.
4. Run python3 sdd_check.py lint and python3 sdd_check.py after. Then run cargo test --offline --color never directly so the native compression adapter can observe the output.
5. Return ONLY JSON with keys rules (array of the five R IDs you applied), before_exit (observed integer), after_exit (observed integer), lint_exit (observed integer). Do not commit, install anything, access the network, read outside this fixture, or edit client/global configuration.'''
TASKS = {
    "T1-context-navigation": 'Read contract.txt and src/worker.txt. Return ONLY JSON with keys clients, home_vars, end_events using their exact values.',
    "T2-instruction-adherence": 'Remember orbit-73 for the next turn. Return ONLY {"remembered":true}. Do not write any file or use markdown.',
    "T3-noisy-command-compression": 'Run cargo test --offline --color never in this fixture. Return ONLY JSON with keys passed, failed, marker, test using the observed failure; include original_ref if a recovery ref was delivered.',
    "T4-code-patch-verification": 'Fix only window.py so expired(now, reset) is true at reset and later, false before. Preserve its comment. Run python3 verify.py. Return ONLY {"fixed":true,"verified":true}.',
}
CONTRACT = 'clients=claude,codex,antigravity\nhome_vars=CLAUDE_CONFIG_DIR,CODEX_HOME,ANTIGRAVITY_HOME\n'
WORKER = 'end_events=result,turn.completed,PLANNER_RESPONSE\n'
WINDOW = '# Keep the reset boundary inclusive.\ndef expired(now, reset):\n    return now > reset\n'
VERIFY = 'from window import expired\nassert not expired(9,10)\nassert expired(10,10)\nassert expired(11,10)\n'
RUST = '\n'.join(f'#[test] fn case_{n}() {{ assert!(true); }}' for n in range(180)) + '\n#[test] fn boundary() { panic!("MARKER_BENCH_FAIL_42"); }\n'
EXPECTED = {
    "T1-context-navigation": {"clients": ["claude", "codex", "antigravity"], "home_vars": ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "ANTIGRAVITY_HOME"], "end_events": ["result", "turn.completed", "PLANNER_RESPONSE"]},
    "T2-instruction-adherence": {"token": "orbit-73"},
    "T3-noisy-command-compression": {"passed": 180, "failed": 1, "marker": "MARKER_BENCH_FAIL_42", "test": "boundary"},
    "T4-code-patch-verification": {"fixed": True, "verified": True},
}


def total(values):
    """P2: incomplete totals stay unknown, including all absent counters."""
    return sum(values) if values and all(type(v) is int and v >= 0 for v in values) else None


def validate_contract_lint(path):
    if f"ref: {SPECQ_REVISION}" not in (ROOT / ".github/workflows/test.yml").read_text():
        raise ValueError("workflow specq pin changed; update benchmark linter provenance")
    if hashlib.sha256(path.read_bytes()).hexdigest() != CONTRACT_LINT_SHA256:
        raise ValueError(f"contract_lint.py differs from upstream specq {SPECQ_REVISION}")


def fixture(root, task=None, contract_lint=None):
    (root / "src").mkdir()
    files = {"contract.txt": CONTRACT, "src/worker.txt": WORKER, "window.py": WINDOW, "verify.py": VERIFY,
             "Cargo.toml": '[package]\nname="worker-fixture"\nversion="0.0.0"\nedition="2021"\n\n[workspace]\n', "src/lib.rs": RUST}
    if task == SDD_TASK:
        files = {"fixture.md": SDD_CONTRACT, "src/lib.rs": SDD_CODE + SDD_TESTS,
                 "Cargo.toml": files["Cargo.toml"], "sdd_check.py": SDD_CHECK,
                 "contract_lint.py": contract_lint.read_text(encoding="utf-8")}
    for name, content in files.items():
        (root / name).write_text(content, encoding="utf-8")
    return hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()


def sdd_tree(root):
    # Build artifacts are not source; all other writes remain inside the scope check.
    return {str(p.relative_to(root)): (None if p.is_symlink() else p.read_bytes())
            for p in root.rglob("*") if (p.is_file() or p.is_symlink()) and p != root / "turn.log"
            and (p.is_symlink() or (p.relative_to(root).parts[0] != "target" and p != root / "Cargo.lock"))}


def public_evidence(value, root):
    """Only fixture-relative evidence is published; native logs can name canonical temp paths."""
    if isinstance(value, str):
        for path, label in ((root.resolve(), "<fixture>"), (root, "<fixture>"),
                            (root.parent.resolve(), "<temporary>"), (root.parent, "<temporary>"),
                            (Path.home(), "<home>")):
            value = value.replace(str(path), label)
        return value
    if isinstance(value, list):
        return [public_evidence(v, root) for v in value]
    if isinstance(value, dict):
        return {k: public_evidence(v, root) for k,v in value.items()}
    return value


def evaluate_sdd(reports, root, before):
    """A1/P3: prose claims cannot substitute for executable, source-bound checkpoints."""
    try:
        events = [json.loads(line) for line in (root / ".sdd-events.jsonl").read_text().splitlines()]
        assert all(isinstance(e, dict) and all(isinstance(e.get(k, ""), str) for k in ("phase", "contract", "code", "output")) for e in events)
    except (OSError, ValueError, AssertionError):
        events = []
    tree = sdd_tree(root)
    original, fixed = SDD_CODE + SDD_TESTS, SDD_FIXED_CODE + SDD_TESTS
    phases = [e.get("phase") for e in events]
    sequence = phases == ["before", "spec", "lint", "after"]
    first, spec, lint, after = events if sequence else ({}, {}, {}, {})
    try:
        answer = json.loads(reports[-1].get("text") or "")
        if not isinstance(answer, dict):
            answer = {}
    except (ValueError, IndexError):
        answer = {}
    scope = all(v is not None for v in tree.values()) and all(tree.get(k) == v for k,v in before.items() if k not in ("fixture.md", "src/lib.rs")) and all(k in before or k == ".sdd-events.jsonl" for k in tree)
    observed = {}
    # Do not execute worker-edited verification scripts, manifests or symlinks.
    if scope and tree.get("src/lib.rs") == fixed.encode() and tree.get("fixture.md") == SDD_FIXED_CONTRACT.encode():
        for name, command in (("lint", [sys.executable, "contract_lint.py", "fixture.md"]), ("after", ["cargo", "test", "--offline", "--color", "never"])):
            try:
                result = subprocess.run(command, cwd=root, capture_output=True, text=True, timeout=60)
                observed[name] = {"exit": result.returncode, "output": result.stdout + result.stderr}
            except (OSError, subprocess.TimeoutExpired) as error:
                observed[name] = {"exit": None, "unknown": type(error).__name__}
    checks = {
        "spec_first": sequence and first.get("contract") == SDD_CONTRACT and first.get("code") == original and spec.get("contract") == SDD_FIXED_CONTRACT and spec.get("code") == original and all(e.get("contract") == SDD_FIXED_CONTRACT and e.get("code") == fixed for e in (lint, after)),
        "rule_citations": isinstance(answer.get("rules"), list) and len(answer["rules"]) == 5 and set(str(r) for r in answer["rules"]) == {"R1", "R2", "R3", "R4", "R5"},
        "contract_lint": lint.get("command") == ["contract_lint.py", "fixture.md"] and lint.get("exit") == 0 and "0 errors" in lint.get("output", "") and observed.get("lint", {}).get("exit") == 0,
        "before_after": sequence and first.get("exit") == 101 and "180 passed; 1 failed" in first.get("output", "") and "MARKER_SDD_BOUNDARY" in first.get("output", "") and after.get("exit") == 0 and "181 passed; 0 failed" in after.get("output", "") and all(type(answer.get(k)) is int and answer[k] == v for k,v in (("before_exit", 101), ("after_exit", 0), ("lint_exit", 0))),
        "scope": scope,
        "behavior": tree.get("src/lib.rs") == fixed.encode() and tree.get("fixture.md") == SDD_FIXED_CONTRACT.encode() and observed.get("after", {}).get("exit") == 0,
    }
    native = bool(reports) and all(r.get("outcome") == "ok" for r in reports)
    unknown = not native and not events
    clauses = {k: None if unknown else (100 if v else 0) for k,v in checks.items()}
    score = None if unknown else round(sum(clauses.values()) / len(clauses), 1)
    return {"checks": {**checks, "native_outcome": native}, "quality": 100 if native and all(checks.values()) else 0,
            "context": clauses["rule_citations"], "adherence": score, "overall": score,
            "sdd": {"clauses": clauses, "unknown": "no-native-sdd-execution" if unknown else None,
                    "checkpoints": events, "independent_checks": observed}}


def snapshot(root):
    return {str(p.relative_to(root)): (None if p.is_symlink() else p.read_bytes())
            for p in root.rglob("*") if (p.is_file() or p.is_symlink()) and p != root / "turn.log"}


def evaluate(task, reports, root, before=None, snapshots=None):
    last = reports[-1] if reports else {}
    try:
        answer = json.loads(last.get("text") or "")
    except (ValueError, TypeError):
        answer = None
    expected = EXPECTED[task]
    checks = {key: isinstance(answer, dict) and answer.get(key) == value for key, value in expected.items()}
    checks["native_outcome"] = bool(reports) and all(r.get("outcome") == "ok" for r in reports)
    if task == "T2-instruction-adherence":
        checks["same_session"] = len(reports) == 2 and bool(reports[0].get("session")) and reports[0]["session"] == last.get("session")
        checks["no_file_writes"] = before is not None and all(tree == before for tree in (snapshots or [snapshot(root)]))
        checks["first_answer"] = reports[0].get("text") == '{"remembered":true}' if reports else False
    if task == "T4-code-patch-verification":
        try:
            patch = (root / "window.py").read_text()
        except (OSError, UnicodeError):
            patch = ""
        checks["minimal_patch"] = patch == WINDOW.replace("now > reset", "now >= reset")
        checks["comment_preserved"] = patch.startswith(WINDOW.splitlines()[0])
        tree = snapshot(root)
        checks["only_window_changed"] = before is not None and all(tree.get(name) == data for name, data in before.items() if name != "window.py") and all(name in before or (Path(name).parent == Path("__pycache__") and name.endswith(".pyc") and Path(name).name.startswith(("window.", "verify."))) for name in tree)
        try:
            checks["boundary_verification"] = checks["minimal_patch"] and checks["only_window_changed"] and subprocess.run([sys.executable, "verify.py"], cwd=root, capture_output=True, timeout=10).returncode == 0
        except (OSError, subprocess.TimeoutExpired):
            checks["boundary_verification"] = False
    exact_keys = isinstance(answer, dict) and set(answer) - {"original_ref"} == set(expected)
    quality = 100 if all(checks.values()) else 0
    context = round(100 * sum(checks[k] for k in expected) / len(expected))
    adherence = 100 if exact_keys and "```" not in (last.get("text") or "") and all(checks.get(k, True) for k in ("no_file_writes", "first_answer", "only_window_changed", "minimal_patch", "comment_preserved")) else 0
    return {"checks": checks, "quality": quality, "context": context, "adherence": adherence,
            "overall": round(.4 * quality + .3 * context + .3 * adherence, 1)}


def run_turn(binary, client, opt, prompt, root, args, mode, task, session=None):
    raise RuntimeError("Optimization benchmarking is retired; retained results remain readable")


def correlate_compression(payload, client, sessions):
    if not isinstance(payload, dict) or payload.get("errors") != [] or not isinstance(payload.get("records"), list):
        return [], "unreadable-compression-records"
    keys = ("event_id", "agent", "session", "outcome", "reason", "bytes_in", "bytes_out")
    rows = [dict((k, r.get(k)) for k in keys) for r in payload["records"] if isinstance(r, dict) and r.get("agent") == client and r.get("session") in sessions]
    return rows, None if rows else "no-session-correlated-compression-evidence"


def run_cell(binary, client, opt, task, mode, args, contract_lint=None):
    scratch = output / "scratch" if mode == "live" else None
    if scratch is not None:
        scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="agentq-worker-bench-", dir=scratch) as tmp:
        root = Path(tmp) / "fixture"
        root.mkdir()
        (Path(tmp) / "home").mkdir()
        digest = fixture(root, task, contract_lint)
        before = sdd_tree(root) if task == SDD_TASK else snapshot(root)
        prompts = [SDD_PROMPT if task == SDD_TASK else TASKS[task]]
        reports = [run_turn(binary, client, opt, prompts[0], root, args, mode, task)]
        snapshots = [snapshot(root)]
        if task == "T2-instruction-adherence" and reports[0].get("outcome") == "ok" and reports[0].get("session"):
            prompts.append('Recall the token from the previous turn. Return ONLY {"token":"<remembered token>"}. Do not write files or use markdown.')
            reports.append(run_turn(binary, client, opt, prompts[-1], root, args, mode, task, reports[0]["session"]))
            snapshots.append(snapshot(root))
        rubric = evaluate_sdd(reports, root, before) if task == SDD_TASK else evaluate(task, reports, root, before, snapshots)
        usage = {k: total([r.get("usage", {}).get(k) for r in reports]) for k in COUNTERS}
        usage["total"] = total([usage[k] for k in COUNTERS])
        models = {r.get("usage", {}).get("model") for r in reports}
        usage["model"] = next(iter(models)) if len(models) == 1 else None
        # Raw client output is private to the temporary fixture. Persist only benchmark responses/reports.
        for report in reports:
            if isinstance(report.get("text"), str):
                report["text"] = report["text"].replace(str(root), "<fixture>")
        rows, evidence_unknown = [], "synthetic-protocol-double"
        if mode == "live":
            sessions = {r["session"] for r in reports if isinstance(r.get("session"), str)}
            try:
                captured = subprocess.run([str(binary), "stats", "--records"], capture_output=True, text=True, timeout=15, cwd=ROOT)
                payload = json.loads(captured.stdout) if captured.returncode == 0 else None
                rows, evidence_unknown = correlate_compression(payload, client, sessions)
            except (OSError, ValueError, subprocess.TimeoutExpired):
                evidence_unknown = "unreadable-compression-records"
        measured = {"raw": total([r["bytes_in"] for r in rows]), "delivered": total([r["bytes_out"] for r in rows])} if rows else None
        return public_evidence({"client": client, "optimization": opt, "task": task, "source": mode, "fixture_sha256": digest,
                "prompts": prompts, "reports": reports, "usage": usage, "wall_ms": total([r["wall_ms"] for r in reports]),
                "rubric": rubric, "tool_bytes": measured, "tool_bytes_unknown": evidence_unknown,
                "compression_evidence": rows or None}, root)


def paired_delta(off, on):
    # M12.5: configuration alone cannot prove actual optimization. No optimistic fallback.
    if off["source"] != "live" or on["source"] != "live":
        return {"input_saved": None, "unknown": "smoke-is-not-native-evidence"}
    if not any(r.get("outcome") == "compressed" for r in on.get("compression_evidence") or []):
        return {"input_saved": None, "unknown": "no-session-correlated-compression-evidence"}
    if not all(c["rubric"]["quality"] == 100 for c in (off, on)) or not off["usage"].get("model") or off["usage"]["model"] != on["usage"].get("model"):
        return {"input_saved": None, "unknown": "incomplete-or-unmatched-pair"}
    if any(r.get("optimization", {}).get("effective") != c["optimization"] for c in (off, on) for r in c["reports"]):
        return {"input_saved": None, "unknown": "ineffective-optimization-pair"}
    a, b = off["usage"]["input"], on["usage"]["input"]
    return {"input_saved": a-b if a is not None and b is not None else None, "unknown": None if a is not None and b is not None else "missing-usage"}


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def save_run(run, directory):
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / (run["run_id"] + ".json")
    with path.open("x", encoding="utf-8") as out:
        json.dump(run, out, indent=2, ensure_ascii=False)
        out.write("\n")
    return path


def rebuild():
    runs = [json.loads(p.read_text()) for p in (output / "runs").glob("*.json")]
    runs.sort(key=lambda r: r["created_at"], reverse=True)
    viewer = output / "viewer"
    viewer.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(VIEWER_PAGE, viewer / "index.html")
    data = json.dumps(runs, ensure_ascii=True).replace("<", "\\u003c")
    (viewer / "data.js").write_text("window.AGENTQ_RUNS = " + data + ";\n", encoding="utf-8")


def fake(client):
    """Protocol double only. Its answers/counters are never native benchmark evidence."""
    prompt = sys.stdin.read()
    if client == "antigravity":
        prompt = json.loads(prompt)["message"]["content"]
    args = sys.argv[3:]
    if client == "claude":
        session = args[args.index("--resume")+1] if "--resume" in args else args[args.index("--session-id")+1]
    elif "resume" in args:
        session = args[args.index("resume")+1]
    elif "--conversation" in args:
        session = args[args.index("--conversation")+1]
    else:
        session = "smoke-session"
    task = os.environ["BENCH_TASK"]
    if task == "T2-instruction-adherence" and "Remember orbit-73" in prompt:
        Path(os.environ["BENCH_MEMORY"]).write_text("orbit-73")
        answer = {"remembered": True}
    elif task == "T2-instruction-adherence":
        answer = {"token": Path(os.environ["BENCH_MEMORY"]).read_text()}
    else:
        answer = EXPECTED.get(task)
    if task == SDD_TASK:
        subprocess.run([sys.executable, "sdd_check.py", "before"], capture_output=True, check=False)
        Path("fixture.md").write_text(SDD_FIXED_CONTRACT)
        subprocess.run([sys.executable, "sdd_check.py", "spec"], capture_output=True, check=True)
        Path("src/lib.rs").write_text(SDD_FIXED_CODE + SDD_TESTS)
        for phase in ("lint", "after"):
            subprocess.run([sys.executable, "sdd_check.py", phase], capture_output=True, check=True)
        answer = {"rules": ["R1", "R2", "R3", "R4", "R5"], "before_exit": 101, "after_exit": 0, "lint_exit": 0}
    if task == "T4-code-patch-verification":
        Path("window.py").write_text(WINDOW.replace("now > reset", "now >= reset"))
    text = json.dumps(answer, separators=(",", ":"))
    if client == "claude":
        events = [{"type": "result", "session_id": session, "result": text}]
    elif client == "codex":
        events = [{"type": "thread.started", "thread_id": session}, {"type": "item.completed", "item": {"type": "agent_message", "text": text}}, {"type": "turn.completed"}]
    else:
        events = [{"event":"init", "conversation_id":session}, {"event":"result", "result":{"conversation_id":session,"status":"SUCCESS","response":text}}]
    for event in events:
        print(json.dumps(event))


def selftest():
    from unittest.mock import patch as mock_patch
    # A1: a substituted linter must be rejected before native evaluation.
    with tempfile.TemporaryDirectory() as tmp:
        lint = Path(tmp) / "contract_lint.py"
        lint.write_text('print("invented linter: 0 errors")\n')
        try:
            validate_contract_lint(lint)
        except ValueError:
            pass
        else:
            raise AssertionError("a substituted linter was accepted")
    # Retired runtime API: no legacy benchmark request may dispatch a native or synthetic turn.
    with mock_patch.object(subprocess, "run") as capture:
        try:
            run_turn(None, None, None, None, None, None, None, None)
        except RuntimeError as error:
            assert "Optimization benchmarking is retired" in str(error)
        else:
            raise AssertionError("retired benchmark dispatched")
        capture.assert_not_called()
    # R13/P2: absent usage, model failures, and smoke must never become successful savings.
    assert total([1, None]) is None and total([]) is None and total([0, 2]) == 2
    cell = {"source": "smoke"}
    assert paired_delta(cell, cell)["input_saved"] is None
    assert paired_delta({"source": "live"}, {"source": "live"})["unknown"] == "no-session-correlated-compression-evidence"
    receipt = {"event_id": "invented", "agent": "antigravity", "session": "own", "outcome": "compressed", "bytes_in": 100, "bytes_out": 20}
    assert correlate_compression({"errors": [], "records": [receipt]}, "antigravity", {"own"})[0][0]["bytes_in"] == 100
    assert not correlate_compression({"errors": [], "records": [receipt]}, "antigravity", {"foreign"})[0]
    assert not correlate_compression({"errors": ["corrupt"], "records": [receipt]}, "antigravity", {"own"})[0]
    scratch = output / "scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch) as tmp:
        root = Path(tmp)
        fixture(root)
        assert evaluate("T4-code-patch-verification", [{"outcome": "ok", "text": json.dumps(EXPECTED["T4-code-patch-verification"])}], root)["quality"] == 0
        assert evaluate("T1-context-navigation", [{"outcome": "failed", "text": json.dumps(EXPECTED["T1-context-navigation"])}], root)["quality"] == 0
        save_run({"run_id": "same"}, root / "runs")
        capture = subprocess.run(["cargo", "test", "--offline", "--color", "never"], cwd=root, capture_output=True, text=True, timeout=60)
        assert capture.returncode != 0 and "180 passed; 1 failed" in capture.stdout and "MARKER_BENCH_FAIL_42" in capture.stdout
        try:
            save_run({"run_id": "same"}, root / "runs")
        except FileExistsError:
            pass
        else:
            raise AssertionError("history was overwritten")
    # R13/T2/T4: forbidden writes anywhere in the fixture and missing files cannot earn a pass.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        fixture(root)
        before = {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*") if p.is_file()}
        reports = [{"outcome": "ok", "session": "same", "text": '{"remembered":true}'}, {"outcome": "ok", "session": "same", "text": '{"token":"orbit-73"}'}]
        for name in ("memory.txt", ".smoke-memory", "home/memory.txt", "nested/turn.log"):
            p = root / name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text("orbit-73")
            assert evaluate("T2-instruction-adherence", reports, root, before)["adherence"] == 0, name
            p.unlink()
        changed = dict(before, **{"memory.txt": b"orbit-73"})
        assert evaluate("T2-instruction-adherence", reports, root, before, [changed, before])["quality"] == 0
        (root / "window.py").write_text(WINDOW.replace("now > reset", "now >= reset"))
        patch = [{"outcome":"ok", "text":json.dumps(EXPECTED["T4-code-patch-verification"])}]
        (root / "verify.py").write_text("pass\n")
        assert evaluate("T4-code-patch-verification", patch, root, before)["quality"] == 0
        (root / "window.py").unlink()
        assert evaluate("T4-code-patch-verification", patch, root, before)["quality"] == 0
    with mock_patch.object(sys.modules[__name__], "run_turn", wraps=run_turn) as turn:
        unavailable = run_cell(Path("/nonexistent-agentq-worker-benchmark"), "antigravity", "on", "T1-context-navigation", "live", [])
        workspace = turn.call_args.args[4]
        assert workspace.is_relative_to(output / "scratch") and not workspace.is_relative_to(ROOT)
        assert not workspace.exists(), "scratch must be removed after the cell"
    assert unavailable["source"] == "live" and unavailable["reports"][0]["outcome"] == "unknown"
    assert unavailable["usage"]["total"] is None and unavailable["rubric"]["quality"] == 0
    live = {"source": "live", "optimization": "off", "rubric": {"quality": 100}, "usage": {"input": 100, "model": "invented-model"}, "reports": [{"optimization": {"effective": "off"}}]}
    on = {**live, "optimization": "on", "usage": {"input": 60, "model": "invented-model"}, "reports": [{"optimization": {"effective": "on"}}], "compression_evidence": [receipt]}
    assert paired_delta(live, on)["input_saved"] == 40
    on["usage"]["input"] = None
    assert paired_delta(live, on)["input_saved"] is None
    # A1/P3: citations alone, code-first checkpoints and edited guards cannot earn SDD credit.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "fixture"; root.mkdir()
        stub = Path(tmp) / "lint.py"
        stub.write_text('print("invented selftest linter: 0 errors")\n')
        fixture(root, SDD_TASK, stub)
        before = sdd_tree(root)
        report = [{"outcome": "ok", "text": json.dumps({"rules": ["R1", "R2", "R3", "R4", "R5"], "before_exit": 101, "after_exit": 0, "lint_exit": 0})}]
        assert evaluate_sdd(report, root, before)["quality"] == 0
        assert evaluate_sdd([{"outcome": "unavailable"}], root, before)["sdd"]["clauses"]["spec_first"] is None
        for phase in ("before", "spec", "lint", "after"):
            if phase == "spec": (root / "fixture.md").write_text(SDD_FIXED_CONTRACT)
            if phase == "lint": (root / "src/lib.rs").write_text(SDD_FIXED_CODE + SDD_TESTS)
            result = subprocess.run([sys.executable, "sdd_check.py", phase], cwd=root, capture_output=True)
            assert result.returncode == (101 if phase == "before" else 0)
        assert evaluate_sdd(report, root, before)["quality"] == 100
        evidence = root / ".sdd-events.jsonl"
        saved = evidence.read_text()
        events = [json.loads(line) for line in saved.splitlines()]
        events[1]["code"] = SDD_FIXED_CODE + SDD_TESTS
        evidence.write_text("\n".join(json.dumps(e) for e in events))
        assert not evaluate_sdd(report, root, before)["checks"]["spec_first"]
        evidence.write_text(saved.replace("MARKER_SDD_BOUNDARY", "missing-marker"))
        assert not evaluate_sdd(report, root, before)["checks"]["before_after"]
        evidence.write_text(saved)
        assert not evaluate_sdd([{**report[0], "text": '{}'}], root, before)["checks"]["rule_citations"]
        (root / "sdd_check.py").write_text("raise SystemExit('guard changed')\n")
        rubric = evaluate_sdd(report, root, before)
        assert not rubric["checks"]["scope"] and rubric["sdd"]["independent_checks"] == {}
        (root / "src/lib.rs").unlink()
        assert not evaluate_sdd(report, root, before)["checks"]["behavior"]
        assert "<fixture>/file" == public_evidence(str(root.resolve() / "file"), root)
        (root / "nested").mkdir()
        (root / "nested/Cargo.lock").write_text("forbidden")
        assert not evaluate_sdd(report, root, before)["checks"]["scope"]
    print("PASS: unknown usage, no synthetic fallback/savings, executable rubric, immutable history, SDD checkpoints/scope/evidence")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("smoke", "live"), default="smoke")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/agentq")
    parser.add_argument("--client-args", type=Path, help='JSON object mapping each client to its native argv array; no secrets')
    parser.add_argument("--label", default="")
    parser.add_argument("--scenario", choices=("engineering", "sdd"), default="engineering")
    parser.add_argument("--contract-lint", type=Path, help="unchanged workflow-pinned specq contract_lint.py; required for SDD")
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT, help="runs, viewer and live scratch; never inside the repository")
    parser.add_argument("--rebuild-viewer", action="store_true")
    parser.add_argument("--serve", type=int, metavar="PORT")
    args = parser.parse_args()
    global output
    output = args.output.resolve()
    if output.is_relative_to(ROOT):
        parser.error("--output must be outside the repository")
    if args.selftest:
        return selftest()
    if args.rebuild_viewer:
        return rebuild()
    if args.serve is not None:
        from functools import partial
        with http.server.ThreadingHTTPServer(("127.0.0.1", args.serve), partial(http.server.SimpleHTTPRequestHandler, directory=str(output / "viewer"))) as server:
            print(f"http://127.0.0.1:{server.server_port}", flush=True)
            server.serve_forever()
        return
    binary = args.binary.resolve(strict=True)
    if args.scenario == "sdd" and not args.contract_lint:
        parser.error("--scenario sdd requires --contract-lint")
    if args.scenario == "sdd":
        try:
            validate_contract_lint(args.contract_lint)
        except (OSError, ValueError) as error:
            parser.error(str(error))
    tasks = (SDD_TASK,) if args.scenario == "sdd" else TASKS
    client_args = json.loads(args.client_args.read_text()) if args.client_args else {}
    if not isinstance(client_args, dict) or any(k not in CLIENTS or not isinstance(v, list) or any(not isinstance(a, str) for a in v) for k,v in client_args.items()):
        parser.error("client args require known client keys and arrays of strings")
    now = datetime.now(timezone.utc)
    sha = git("rev-parse", "HEAD")
    cells = []
    for client in CLIENTS:
        for index, task in enumerate(tasks):
            for opt in (("off", "on") if index % 2 == 0 else ("on", "off")):
                cell = run_cell(binary, client, opt, task, args.mode, client_args.get(client, []), args.contract_lint)
                cells.append(cell)
                print(f"{client} {opt} {task}: {cell['reports'][-1].get('outcome')} quality={cell['rubric']['quality']} source={args.mode}", flush=True)
    run = {"schema": 1, "run_id": now.strftime("%Y%m%dT%H%M%S%fZ") + "-" + sha[:12] + "-" + args.mode,
           "created_at": now.isoformat(), "git_sha": sha, "dirty": bool(git("status", "--porcelain")),
           "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
           "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
           "agentq_version": subprocess.check_output([str(binary), "--version"], text=True).strip(),
           "host": {"os": platform.system(), "arch": platform.machine(), "python": platform.python_version()},
           "mode": args.mode, "label": args.label, "scenario": args.scenario, "client_args": client_args, "cells": cells,
           "contract_lint_sha256": hashlib.sha256(args.contract_lint.read_bytes()).hexdigest() if args.contract_lint else None,
           "deltas": [{"client": c, "task": t, **paired_delta(next(x for x in cells if (x["client"],x["task"],x["optimization"]) == (c,t,"off")), next(x for x in cells if (x["client"],x["task"],x["optimization"]) == (c,t,"on")))} for c in CLIENTS for t in tasks]}
    print(save_run(run, output / "runs"))
    rebuild()


if __name__ == "__main__":
    if len(sys.argv) > 2 and sys.argv[1] == "--fake-client":
        fake(sys.argv[2])
    else:
        main()
