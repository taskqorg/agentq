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
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
RUNS = ROOT / "benchmarks/runs"
VIEWER = ROOT / "benchmarks/viewer"
CLIENTS = ("claude", "codex", "antigravity")
COUNTERS = ("input", "cache_read", "cache_write", "output")
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


def fixture(root):
    (root / "src").mkdir()
    files = {"contract.txt": CONTRACT, "src/worker.txt": WORKER, "window.py": WINDOW, "verify.py": VERIFY,
             "Cargo.toml": '[package]\nname="worker-fixture"\nversion="0.0.0"\nedition="2021"\n', "src/lib.rs": RUST}
    for name, content in files.items():
        (root / name).write_text(content, encoding="utf-8")
    return hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()


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
    if mode == "live" and client == "antigravity" and "--dangerously-skip-permissions" not in args:
        args = [*args, "--dangerously-skip-permissions"]
    request = {"client": client, "root": str(root), "log": str(root / "turn.log"), "input": prompt,
               "session": session, "resume": bool(session), "optimization": opt, "args": args,
               "timeout_s": 120, "context": {"product": "worker-benchmark", "version": "1"}}
    if mode == "smoke":
        request.update(command=[sys.executable, str(Path(__file__).resolve()), "--fake-client", client],
                       home=str(root.parent / "home"), env={"BENCH_TASK": task, "BENCH_MEMORY": str(root.parent / "home/memory"), "COREQ_FAKE_AGENT": client, "HOME": str(root / "home"), "USERPROFILE": str(root / "home")})
    environment = os.environ.copy()
    if mode == "smoke":
        environment.update(HOME=str(root / "home"), USERPROFILE=str(root / "home"), LOCALAPPDATA=str(root / "home"), APPDATA=str(root / "home"))
    start = time.monotonic()
    try:
        proc = subprocess.run([str(binary), "agent", "turn"], input=json.dumps(request), text=True, capture_output=True, timeout=135, cwd=ROOT, env=environment)
        if proc.returncode:
            report = {"outcome": "unavailable", "error": "runtime-command-failed", "detail": proc.stderr[-1000:].replace(str(root), "<fixture>")}
        else:
            report = json.loads(proc.stdout)
            if not isinstance(report, dict) or report.get("schema") != 1:
                raise ValueError("invalid runtime report")
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        report = {"outcome": "unknown", "error": type(error).__name__, "detail": "No resubmission; native execution may still be active."}
    # Do not persist fake usage as observed native data.
    if mode == "smoke":
        report["usage"] = {**dict.fromkeys((*COUNTERS, "calls", "model")), "unknown": "synthetic-protocol-double"}
    report["wall_ms"] = round((time.monotonic() - start) * 1000)
    return report


def correlate_compression(payload, client, sessions):
    if not isinstance(payload, dict) or payload.get("errors") != [] or not isinstance(payload.get("records"), list):
        return [], "unreadable-compression-records"
    keys = ("event_id", "agent", "session", "outcome", "reason", "bytes_in", "bytes_out")
    rows = [dict((k, r.get(k)) for k in keys) for r in payload["records"] if isinstance(r, dict) and r.get("agent") == client and r.get("session") in sessions]
    return rows, None if rows else "no-session-correlated-compression-evidence"


def run_cell(binary, client, opt, task, mode, args):
    scratch = ROOT / "benchmarks/.tmp" if mode == "live" else None
    if scratch is not None:
        scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="agentq-worker-bench-", dir=scratch) as tmp:
        root = Path(tmp) / "fixture"
        root.mkdir()
        (Path(tmp) / "home").mkdir()
        digest = fixture(root)
        before = snapshot(root)
        prompts = [TASKS[task]]
        reports = [run_turn(binary, client, opt, prompts[0], root, args, mode, task)]
        snapshots = [snapshot(root)]
        if task == "T2-instruction-adherence" and reports[0].get("outcome") == "ok" and reports[0].get("session"):
            prompts.append('Recall the token from the previous turn. Return ONLY {"token":"<remembered token>"}. Do not write files or use markdown.')
            reports.append(run_turn(binary, client, opt, prompts[-1], root, args, mode, task, reports[0]["session"]))
            snapshots.append(snapshot(root))
        rubric = evaluate(task, reports, root, before, snapshots)
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
        return {"client": client, "optimization": opt, "task": task, "source": mode, "fixture_sha256": digest,
                "prompts": prompts, "reports": reports, "usage": usage, "wall_ms": total([r["wall_ms"] for r in reports]),
                "rubric": rubric, "tool_bytes": measured, "tool_bytes_unknown": evidence_unknown,
                "compression_evidence": rows or None}


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


def rebuild(directory=RUNS, viewer=VIEWER):
    runs = [json.loads(p.read_text()) for p in directory.glob("*.json")]
    runs.sort(key=lambda r: r["created_at"], reverse=True)
    viewer.mkdir(parents=True, exist_ok=True)
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
        answer = EXPECTED[task]
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
    # #192 A3: inspect the actual runtime request; permission bypass is limited to live agy fixtures.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for client, mode, args, expected in (
            ("antigravity", "live", [], ["--dangerously-skip-permissions"]),
            ("antigravity", "live", ["--model=pro", "--dangerously-skip-permissions"], ["--model=pro", "--dangerously-skip-permissions"]),
            ("antigravity", "smoke", [], []),
            ("codex", "live", [], []),
        ):
            with mock_patch.object(subprocess, "run") as capture:
                capture.return_value.returncode = 0
                capture.return_value.stdout = '{"schema":1}'
                run_turn(Path("invented-binary"), client, "off", "invented prompt", root, args, mode, "T1-context-navigation")
                assert json.loads(capture.call_args.kwargs["input"])["args"] == expected
    # R13/P2: absent usage, model failures, and smoke must never become successful savings.
    assert total([1, None]) is None and total([]) is None and total([0, 2]) == 2
    cell = {"source": "smoke"}
    assert paired_delta(cell, cell)["input_saved"] is None
    assert paired_delta({"source": "live"}, {"source": "live"})["unknown"] == "no-session-correlated-compression-evidence"
    receipt = {"event_id": "invented", "agent": "antigravity", "session": "own", "outcome": "compressed", "bytes_in": 100, "bytes_out": 20}
    assert correlate_compression({"errors": [], "records": [receipt]}, "antigravity", {"own"})[0][0]["bytes_in"] == 100
    assert not correlate_compression({"errors": [], "records": [receipt]}, "antigravity", {"foreign"})[0]
    assert not correlate_compression({"errors": ["corrupt"], "records": [receipt]}, "antigravity", {"own"})[0]
    with tempfile.TemporaryDirectory() as tmp:
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
        assert workspace.is_relative_to(ROOT / "benchmarks/.tmp")
        assert not workspace.exists(), "scratch must be removed after the cell"
    assert unavailable["source"] == "live" and unavailable["reports"][0]["outcome"] == "unknown"
    assert unavailable["usage"]["total"] is None and unavailable["rubric"]["quality"] == 0
    live = {"source": "live", "optimization": "off", "rubric": {"quality": 100}, "usage": {"input": 100, "model": "invented-model"}, "reports": [{"optimization": {"effective": "off"}}]}
    on = {**live, "optimization": "on", "usage": {"input": 60, "model": "invented-model"}, "reports": [{"optimization": {"effective": "on"}}], "compression_evidence": [receipt]}
    assert paired_delta(live, on)["input_saved"] == 40
    on["usage"]["input"] = None
    assert paired_delta(live, on)["input_saved"] is None
    print("PASS: unknown usage, no synthetic fallback/savings, executable rubric, immutable history")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("smoke", "live"), default="smoke")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/agentq")
    parser.add_argument("--client-args", type=Path, help='JSON object mapping each client to its native argv array; no secrets')
    parser.add_argument("--label", default="")
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--rebuild-viewer", action="store_true")
    parser.add_argument("--serve", type=int, metavar="PORT")
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    if args.rebuild_viewer:
        return rebuild()
    if args.serve is not None:
        from functools import partial
        with http.server.ThreadingHTTPServer(("127.0.0.1", args.serve), partial(http.server.SimpleHTTPRequestHandler, directory=str(VIEWER))) as server:
            print(f"http://127.0.0.1:{server.server_port}", flush=True)
            server.serve_forever()
        return
    binary = args.binary.resolve(strict=True)
    client_args = json.loads(args.client_args.read_text()) if args.client_args else {}
    if not isinstance(client_args, dict) or any(k not in CLIENTS or not isinstance(v, list) or any(not isinstance(a, str) for a in v) for k,v in client_args.items()):
        parser.error("client args require known client keys and arrays of strings")
    now = datetime.now(timezone.utc)
    sha = git("rev-parse", "HEAD")
    cells = []
    for client in CLIENTS:
        for index, task in enumerate(TASKS):
            for opt in (("off", "on") if index % 2 == 0 else ("on", "off")):
                cell = run_cell(binary, client, opt, task, args.mode, client_args.get(client, []))
                cells.append(cell)
                print(f"{client} {opt} {task}: {cell['reports'][-1].get('outcome')} quality={cell['rubric']['quality']} source={args.mode}", flush=True)
    run = {"schema": 1, "run_id": now.strftime("%Y%m%dT%H%M%S%fZ") + "-" + sha[:12] + "-" + args.mode,
           "created_at": now.isoformat(), "git_sha": sha, "dirty": bool(git("status", "--porcelain")),
           "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
           "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
           "agentq_version": subprocess.check_output([str(binary), "--version"], text=True).strip(),
           "host": {"os": platform.system(), "arch": platform.machine(), "python": platform.python_version()},
           "mode": args.mode, "label": args.label, "client_args": client_args, "cells": cells,
           "deltas": [{"client": c, "task": t, **paired_delta(next(x for x in cells if (x["client"],x["task"],x["optimization"]) == (c,t,"off")), next(x for x in cells if (x["client"],x["task"],x["optimization"]) == (c,t,"on")))} for c in CLIENTS for t in TASKS]}
    print(save_run(run, RUNS).relative_to(ROOT))
    rebuild()


if __name__ == "__main__":
    if len(sys.argv) > 2 and sys.argv[1] == "--fake-client":
        fake(sys.argv[2])
    else:
        main()
