#!/usr/bin/env python3
"""#284 A6 native qualification of launch preparation (docs/evidence/runtime-284.md).

Runs the real `codex`, `claude` and `git` on PATH against isolated homes under the OS temp dir. It reads no login and
edits no working profile; it installs no service. Model calls go to a local stub, so it needs no account and spends nothing.

    cargo build --locked --bin agentq --example qualify_launch -p agentq -p agentq-runtime
    python3 scripts/qualify-launch-preparation.py
"""
import http.server
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if os.name == "nt" else ""
FAILED = []


def fact(name, ok, detail=""):
    if not ok:
        FAILED.append(name)
    print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)


def run(cmd, env=None, timeout=120, input=None, cwd=None):
    """Output goes through files: a client's helper that outlives it keeps a pipe open and would hold this script."""
    merged = dict(os.environ)
    merged.update(env or {})
    merged = {k: v for k, v in merged.items() if v is not None}
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err, tempfile.TemporaryFile() as stdin:
        stdin.write((input or "").encode())
        stdin.seek(0)
        child = subprocess.Popen(cmd, stdin=stdin, stdout=out, stderr=err, env=merged, cwd=cwd)
        try:
            code = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            if os.name == "nt":
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(child.pid)], capture_output=True)
            child.kill()
            child.wait()
            raise
        out.seek(0)
        err.seek(0)
        return subprocess.CompletedProcess(cmd, code, out.read().decode("utf-8", "replace"), err.read().decode("utf-8", "replace"))


def which(name):
    return shutil.which(name + ".cmd") or shutil.which(name) or sys.exit(f"{name} not on PATH")


def working_claude():
    """The first Claude that answers. A host may hold a stale npm shim first on PATH; it is named, not hidden."""
    candidates = [shutil.which("claude.exe"), shutil.which("claude.cmd"), shutil.which("claude"), os.environ.get("CLAUDE_CODE_EXECPATH")]
    candidates += [str(Path.home() / ".local" / "bin" / f"claude{EXE}"), str(Path(os.environ.get("APPDATA", "")) / "npm" / "claude.cmd")]
    for path in filter(None, candidates):
        if not Path(path).is_file():
            continue
        try:
            if run([path, "--version"], timeout=30).returncode == 0:
                return path
        except OSError:
            pass
        print(f"claude candidate does not start: {path}", flush=True)
    sys.exit("no working Claude")


class Stub(http.server.BaseHTTPRequestHandler):
    """A Responses endpoint that ends every turn at once, and a remote that demands credentials."""
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        body = "".join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in [
            {"type": "response.created", "response": {"id": "r1"}},
            {"type": "response.completed", "response": {"id": "r1", "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}}])
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        self.wfile.write(body.encode())

    def do_GET(self):
        self.send_response(401)
        self.send_header("WWW-Authenticate", 'Basic realm="invented"')
        self.send_header("content-length", "0")
        self.end_headers()

    def log_message(self, *_):
        pass


def codex_round(base, name, port, agentq, trust):
    """One isolated home: owned hooks, optional trust through the real onboarding code, then a real Codex turn."""
    home = base / name
    codex_home, install = home / "codex", home / "install"
    codex_home.mkdir(parents=True)
    (install / "data").mkdir(parents=True)
    installed = install / f"agentq{EXE}"
    shutil.copy(agentq, installed)
    (codex_home / "config.toml").write_text(
        f'model = "stub"\nmodel_provider = "stub"\n\n[model_providers.stub]\nname = "stub"\nbase_url = "http://127.0.0.1:{port}/v1"\nwire_api = "responses"\n')
    env = {"AGENTQ_QUAL_CODEX_HOME": str(codex_home), "AGENTQ_QUAL_EXE": str(installed), "CODEX_HOME": str(codex_home)}
    if trust:
        env["AGENTQ_QUAL_TRUST"] = "1"
    out = run(["cargo", "test", "--locked", "-p", "agentq", "--bin", "agentq", "native_onboard_trust", "--", "--ignored", "--nocapture"], env=env, cwd=ROOT, timeout=600)
    print(out.stdout.replace(str(base), "<base>")[:1500], flush=True)
    if out.returncode:
        sys.exit(out.stdout + out.stderr)
    project = home / "project"
    project.mkdir()
    run(["git", "init", "-q"], cwd=project)
    # AGENTQ_INSTALL keeps the hook's evidence and records in this home, away from a real installation.
    turn = run([which("codex"), "exec", "--json", "-C", str(project), "-"], env={"CODEX_HOME": str(codex_home), "AGENTQ_INSTALL": str(install), "AGENTQ_INCIDENT_REPORTS": "off"}, input="hi", timeout=120)
    evidence = install / "data" / "evidence"
    seen = sorted(p.name for p in evidence.iterdir()) if evidence.exists() else []
    print(f"{name}: codex exit {turn.returncode}; hook evidence {seen}", flush=True)
    if turn.returncode:
        print(turn.stdout[-400:], turn.stderr[-400:])
    return codex_home, out.stdout, seen


def doctor(base, name):
    """The real `doctor` and `status` of one isolated round: read-only, no service touched."""
    home = base / name
    env = {"AGENTQ_INSTALL": str(home / "install"), "CODEX_HOME": str(home / "codex"), "HOME": str(home), "USERPROFILE": str(home), "LOCALAPPDATA": str(home), "APPDATA": str(home),
           "CLAUDE_CONFIG_DIR": str(home / "claude"), "HERMES_HOME": str(home / "hermes"), "AGENTQ_INCIDENT_REPORTS": "off"}
    out = run([str(home / "install" / f"agentq{EXE}"), "doctor"], env=env, timeout=120)
    status = run([str(home / "install" / f"agentq{EXE}"), "status", "--json"], env=env, timeout=120)
    lines = [l.replace(str(base), "<base>") for l in out.stdout.splitlines() if l.startswith("hooks: codex")]
    print(f"{name}: doctor exit {out.returncode}: {lines}", flush=True)
    trust = json.loads(status.stdout).get("hook_trust")
    print(f"{name}: status exit {status.returncode}: {trust}", flush=True)
    return out.returncode, lines, status.returncode, trust


def git_prompt(base, port, agentq):
    """A real git behind `agentq agent turn`: the child environment must fail a credential prompt at once."""
    work = base / "git"
    work.mkdir()
    slow = work / ("slow-askpass.cmd" if os.name == "nt" else "slow-askpass")
    # A window waiting for a person: answers after 60 s, far past the bound below.
    slow.write_text("@ping -n 60 127.0.0.1 >NUL\r\n@echo invented\r\n" if os.name == "nt" else "#!/bin/sh\nsleep 60\necho invented\n")
    slow.chmod(0o755)
    child = work / "child.py"
    child.write_text(
        "import json, os, subprocess, sys, time\n"
        "s = time.time()\n"
        f"r = subprocess.run(['git', 'ls-remote', 'http://127.0.0.1:{port}/invented.git'], capture_output=True, text=True, timeout=90)\n"
        "names = ['GIT_TERMINAL_PROMPT', 'GIT_ASKPASS', 'SSH_ASKPASS_REQUIRE', 'GCM_INTERACTIVE', 'GIT_SSH_COMMAND', 'ANTHROPIC_API_KEY', 'CLAUDECODE']\n"
        f"open(r'{work / 'result.json'}', 'w').write(json.dumps({{'exit': r.returncode, 'seconds': round(time.time() - s, 1), 'stderr': r.stderr[-200:], 'env': {{n: os.environ.get(n) for n in names}}}}))\n")
    # Without the guard: the same git and askpass, so the guard is what changes the result.
    start = time.time()
    try:
        bare = run(["git", "ls-remote", f"http://127.0.0.1:{port}/invented.git"], env={"GIT_ASKPASS": str(slow), "GIT_TERMINAL_PROMPT": "1"}, timeout=8)
        before = f"exit {bare.returncode} in {time.time() - start:.1f} s"
    except subprocess.TimeoutExpired:
        before = "still waiting on the askpass program after 8 s"
    print(f"git without the guard: {before}")
    request = {"client": "claude", "root": str(work), "log": str(work / "turn.log"), "session": "00000000-0000-4000-8000-000000000284", "input": "x",
               "command": [sys.executable, str(child)], "optimization": "off", "timeout_s": 90,
               "context": {"product": "qualify", "version": "0"}}
    turn = run([str(agentq), "agent", "turn"], input=json.dumps(request), timeout=120, env={"GIT_ASKPASS": str(slow), "GIT_TERMINAL_PROMPT": "1", "ANTHROPIC_API_KEY": "invented-parent", "CLAUDECODE": "1"})
    result = json.loads((work / "result.json").read_text()) if (work / "result.json").exists() else {}
    print(f"git behind the runtime: {result or turn.stdout[-300:] + turn.stderr[-300:]}")
    fact("A5 a credential prompt fails at once", result.get("exit") not in (None, 0) and result.get("seconds", 99) < 8, str(result.get("seconds")))
    env = result.get("env", {})
    fact("A5 inherited auth and session stay out", env.get("ANTHROPIC_API_KEY") is None and env.get("CLAUDECODE") is None)
    fact("A5 guard variables reach the child", env.get("GIT_TERMINAL_PROMPT") == "0" and env.get("GIT_ASKPASS") == "agentq-no-askpass" and env.get("SSH_ASKPASS_REQUIRE") == "never" and env.get("GCM_INTERACTIVE") == "never", str(env))


def main():
    print("platform:", platform.platform())
    claude = working_claude()
    print("claude:", run([claude, "--version"]).stdout.strip())
    for name in ("codex", "git"):
        print(f"{name}:", run([which(name), "--version"]).stdout.strip())
    target = ROOT / "target" / "debug"
    agentq = target / f"agentq{EXE}"
    example = target / "examples" / f"qualify_launch{EXE}"
    base = Path(tempfile.mkdtemp(prefix="agentq-284-")).resolve()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]

    # A1/A2: the same real turn with and without trust. The first run is the defect the task names.
    _, status, seen = codex_round(base, "untrusted", port, agentq, False)
    fact("A2 before trust: Codex skips the hooks", not any(n.startswith("codex.") for n in seen), str(seen))
    fact("A2 before trust: status fails with a next step", '"state":"failed"' in status and "agentq onboard --yes codex" in status)
    code, lines, status_code, trust = doctor(base, "untrusted")
    fact("A2 real doctor fails each untrusted hook with the next step", code == 1 and len(lines) == 2 and all("untrusted (native); run `agentq onboard --yes codex`" in l for l in lines))
    fact("A2 real status fails and names the state", status_code == 1 and trust["state"] == "failed" and trust["next"])
    home, status, seen = codex_round(base, "trusted", port, agentq, True)
    fact("A1 trust comes from native hooks/list", "hooks: codex Stop trusted (native)" in status and "hooks: codex SessionStart trusted (native)" in status)
    fact("A1 only AgentQ's two entries", (home / "config.toml").read_text().count("[hooks.state.") == 2)
    fact("A1 the real turn ran both hooks", "codex.SessionStart" in seen and "codex.Stop" in seen, str(seen))
    fact("A2 after trust: status ok", '"state":"ok"' in status)
    _, lines, _, trust = doctor(base, "trusted")
    fact("A2 real doctor and status read trusted", len(lines) == 2 and all("trusted (native)" in l and "onboard" not in l for l in lines) and trust["state"] == "ok")
    _, status, seen = codex_round(base, "trusted spaced", port, agentq, True)
    fact("A1 a path with a space runs both hooks", "codex.SessionStart" in seen and "codex.Stop" in seen, str(seen))

    # A3/A4: real clients through the trait.
    out = run([str(example), str(base / "scope")], timeout=300, env={"AGENTQ_QUAL_CLAUDE": claude})
    print(out.stdout)
    if out.returncode:
        print(out.stderr[-800:])
    text = out.stdout
    fact("A3 Codex trust loads the project layer", "listed before trust: false" in text and "listed after trust: true" in text)
    fact("A3 Codex and Claude grant once", text.count("Ok(Granted)") == 2 and text.count("Ok(Unchanged)") == 2)
    fact("A3 home and root refuse", text.count('Some("trust-path-refused")') == 3)
    fact("A3 Claude lock and first run read as causes", 'held lock: Some("trust-locked")' in text and 'first run: Some("trust-config-missing")' in text)
    fact("A3 Claude key uses the client's form", '"hasTrustDialogAccepted": true' in text)
    fact("A4 an isolated home reads its own login", text.count("login missing") == 2 and "codex own login contrast: ok" in text and "claude own login contrast: ok" in text)
    fact("A4 both versions read from the account's binary", "codex isolated ready: install ok version ok" in text and "claude isolated ready: install ok version ok" in text)

    # A3: what real Claude does with the seeded file: it keeps the key under its own form and its own fields.
    claude_home, project = base / "scope" / "claude-account", base / "scope" / "project"
    seeded = json.loads((claude_home / ".claude.json").read_text())["projects"]
    run([claude, "-p", "hi"], cwd=project, timeout=120, env={"CLAUDE_CONFIG_DIR": str(claude_home), "ANTHROPIC_API_KEY": "invented", "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{port}", "CLAUDECODE": None, "CLAUDE_CODE_SESSION_ID": None})
    after = json.loads((claude_home / ".claude.json").read_text())["projects"]
    key = next(iter(seeded))
    print("claude keys after a real run:", [k.replace(str(base).replace(chr(92), "/"), "<base>") for k in after], flush=True)
    fact("A3 a real Claude run keeps the seeded key and adds no other", set(after) == set(seeded) and after[key].get("hasTrustDialogAccepted") is True)

    git_prompt(base, port, agentq)
    server.shutdown()
    shutil.rmtree(base, ignore_errors=True)
    print("result:", "FAILED " + ", ".join(FAILED) if FAILED else "all facts pass")
    return 1 if FAILED else 0


if __name__ == "__main__":
    sys.exit(main())
