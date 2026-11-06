"""Run qualify_ownership against a supplied native Codex; all credentials and data are synthetic and temporary."""
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import time

repo = pathlib.Path(sys.argv[1]).resolve()
native = pathlib.Path(sys.argv[2]).resolve()
with tempfile.TemporaryDirectory(prefix="agentq-native287-") as temp:
    root = pathlib.Path(temp).resolve()
    for name in ("account", "workspace", "user"):
        (root / name).mkdir()
    release = threading.Event()

    class Stub(http.server.BaseHTTPRequestHandler):
        requests = 0

        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            Stub.requests += 1
            (root / "request").write_text(str(Stub.requests))
            while not release.is_set() and not (root / "release").exists():
                time.sleep(0.02)
            rows = [
                {"type": "response.created", "response": {"id": "r1"}},
                {"type": "response.completed", "response": {"id": "r1", "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}},
            ]
            body = "".join("event: " + row["type"] + "\ndata: " + json.dumps(row) + "\n\n" for row in rows).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    home = root / "account"
    config = 'model="stub"\nmodel_provider="fixture"\n[projects.' + json.dumps(str(root / "workspace")) + ']\ntrust_level="trusted"\n[model_providers.fixture]\nname="fixture"\nbase_url="http://127.0.0.1:' + str(server.server_port) + '/v1"\nwire_api="responses"\n'
    (home / "config.toml").write_text(config)
    (home / "auth.json").write_text(json.dumps({"OPENAI_API_KEY": "synthetic-loopback-only"}))
    env = dict(os.environ, HOME=str(root / "user"), USERPROFILE=str(root / "user"), CODEX_HOME=str(home), CLAUDE_CONFIG_DIR=str(root / "user" / ".claude"))
    for key in list(env):
        if key in ("OPENAI_API_KEY", "CODEX_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"):
            env.pop(key)
    subprocess.run(["git", "init", "-q", str(root / "workspace")], check=True, env=env)
    print(subprocess.check_output([str(native), "--version"], env=env, text=True).strip(), flush=True)
    suffix = ".exe" if os.name == "nt" else ""
    try:
        subprocess.run([str(repo / "target/debug/examples" / ("qualify_ownership" + suffix)), str(root), str(native)], env=env, check=True)
        assert Stub.requests == 1
    finally:
        release.set()
        server.shutdown()
