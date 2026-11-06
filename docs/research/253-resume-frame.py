#!/usr/bin/env python3
"""Task 253: measure the native Codex thread/resume frame on invented rollouts.

Usage: python3 -I docs/research/253-resume-frame.py <codex binary> <empty scratch dir>

Each case runs `codex app-server` with CODEX_HOME and HOME in a new temp folder, a rollout of invented
text and a loopback mock Responses endpoint. It reads no login and no real session, makes no paid call
and prints only counts, sizes and native error codes.
"""
import http.server, json, os, select, subprocess, sys, tempfile, threading, time, uuid

LIMIT = 8 * 1024 * 1024  # the AgentQ per-record bound
REQUESTS = []


class Mock(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length") or 0))
        items = json.loads(body).get("input") or []
        REQUESTS.append({"bytes": len(body), "input_items": len(items), "invented_markers": sum(json.dumps(i).count("invented") for i in items)})
        events = [
            {"type": "response.created", "response": {"id": "r1"}},
            {"type": "response.output_item.done", "item": {"type": "message", "role": "assistant", "id": "m1", "content": [{"type": "output_text", "text": "answer"}]}},
            {"type": "response.completed", "response": {"id": "r1", "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}},
        ]
        data = "".join(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n" for e in events).encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def rollout(home, thread, turns, chars):
    folder = os.path.join(home, "sessions/2026/11/06")
    os.makedirs(folder, exist_ok=True)
    path = os.path.join(folder, f"rollout-2026-11-06T00-00-00-{thread}.jsonl")
    at = "2026-11-06T00:00:00.000Z"
    with open(path, "w") as f:
        def row(kind, payload):
            f.write(json.dumps({"timestamp": at, "type": kind, "payload": payload}) + "\n")
        row("session_meta", {"id": thread, "timestamp": at, "cwd": home, "originator": "fixture", "cli_version": "0.162.0", "source": "cli", "model_provider": "mock"})
        for n in range(turns):
            row("event_msg", {"type": "task_started", "turn_id": f"t{n}"})
            row("event_msg", {"type": "user_message", "message": f"invented {n}", "kind": "plain"})
            row("response_item", {"type": "message", "role": "user", "content": [{"type": "input_text", "text": f"invented {n}"}]})
            row("event_msg", {"type": "agent_message", "message": "y" * chars})
            row("response_item", {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "y" * chars}]})
            row("event_msg", {"type": "task_complete", "turn_id": f"t{n}", "last_agent_message": "done"})
    return path


def configure(home, port):
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write(f'model = "mock-model"\nmodel_provider = "mock"\n[model_providers.mock]\nname = "mock"\nbase_url = "http://127.0.0.1:{port}/v1"\nwire_api = "responses"\nrequires_openai_auth = false\nsupports_websockets = false\n')


class Server:
    def __init__(self, codex, home):
        env = {k: v for k, v in os.environ.items() if k in ("PATH", "TMPDIR", "LANG")}
        env.update(CODEX_HOME=home, HOME=home)
        self.p = subprocess.Popen([codex, "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, cwd=home)
        self.buf = b""
        self.largest = 0
        self.send({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "fixture", "version": "1"}, "capabilities": {"experimentalApi": True}}})
        self.until(lambda m: m.get("id") == 1)
        self.send({"method": "initialized"})

    def send(self, message):
        self.p.stdin.write((json.dumps(message) + "\n").encode())
        self.p.stdin.flush()

    def until(self, wanted, seconds=60):
        end = time.time() + seconds
        while time.time() < end:
            while b"\n" in self.buf:
                line, self.buf = self.buf.split(b"\n", 1)
                self.largest = max(self.largest, len(line) + 1)
                try:
                    message = json.loads(line)
                except ValueError:
                    continue
                if wanted(message):
                    return len(line) + 1, message
            if select.select([self.p.stdout], [], [], 0.5)[0]:
                chunk = os.read(self.p.stdout.fileno(), 1 << 20)
                if not chunk:
                    return None, None
                self.buf += chunk
        return None, None

    def stop(self):
        self.p.stdin.close()
        self.p.wait(timeout=15)  # EOF lets the native server release its writer lease


def resume(server, thread, home, exclude):
    params = {"threadId": thread, "cwd": home}
    if exclude:
        params["excludeTurns"] = True
    server.send({"id": 2, "method": "thread/resume", "params": params})
    return server.until(lambda m: m.get("id") == 2)


def frame_case(codex, scratch, port, turns, chars, exclude):
    with tempfile.TemporaryDirectory(dir=scratch) as home:
        thread = str(uuid.uuid4())
        configure(home, port)
        path = rollout(home, thread, turns, chars)
        before = open(path, "rb").read()
        server = Server(codex, home)
        try:
            size, reply = resume(server, thread, home, exclude)
            result = reply["result"]["thread"]
            server.send({"id": 3, "method": "thread/turns/list", "params": {"threadId": thread, "limit": 5}})
            page, listed = server.until(lambda m: m.get("id") == 3)
            largest = server.largest
        finally:
            server.stop()
        after = open(path, "rb").read()
        return {"case": "frame", "excludeTurns": exclude, "rollout_bytes": len(before), "resume_frame": size, "over_limit": size > LIMIT,
                "response_turns": len(result.get("turns") or []), "same_thread": result.get("id") == thread,
                "page_frame": page, "page_turns": len(listed["result"]["data"]), "largest_frame": largest,
                "prefix_kept": after.startswith(before), "rollout_growth": len(after) - len(before)}


def model_case(codex, scratch, port, exclude):
    REQUESTS.clear()
    with tempfile.TemporaryDirectory(dir=scratch) as home:
        thread = str(uuid.uuid4())
        rollout(home, thread, 20, 4096)
        configure(home, port)
        server = Server(codex, home)
        try:
            size, reply = resume(server, thread, home, exclude)
            server.send({"id": 3, "method": "turn/start", "params": {"threadId": thread, "input": [{"type": "text", "text": "invented probe"}]}})
            _, done = server.until(lambda m: m.get("method") == "turn/completed")
        finally:
            server.stop()
        return {"case": "model", "excludeTurns": exclude, "resume_frame": size, "response_turns": len(reply["result"]["thread"].get("turns") or []),
                "turn_status": done["params"]["turn"]["status"], "model_requests": list(REQUESTS)}


def refusal_case(codex, scratch, port):
    with tempfile.TemporaryDirectory(dir=scratch) as home:
        thread = str(uuid.uuid4())
        configure(home, port)
        server = Server(codex, home)
        try:
            _, absent = resume(server, thread, home, True)
        finally:
            server.stop()
        rollout(home, thread, 1, 8)
        first = Server(codex, home)
        second = Server(codex, home)
        try:
            resume(first, thread, home, True)
            _, busy = resume(second, thread, home, True)
        finally:
            first.stop()
            second.stop()
        scrub = lambda reply: {"code": reply["error"]["code"], "message": reply["error"]["message"].replace(thread, "<thread>").replace(home, "<home>")} if "error" in reply else "accepted"
        return {"case": "refusal", "absent_thread": scrub(absent), "second_writer": scrub(busy)}


def main():
    codex, scratch = sys.argv[1:3]
    mock = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Mock)
    threading.Thread(target=mock.serve_forever, daemon=True).start()
    port = mock.server_address[1]
    print(json.dumps({"codex": subprocess.run([codex, "--version"], capture_output=True, text=True).stdout.strip()}))
    for turns, chars in ((40, 26214), (40, 314572)):  # 1 MiB and 12 MiB of answers
        for exclude in (False, True):
            print(json.dumps(frame_case(codex, scratch, port, turns, chars, exclude)))
    for exclude in (False, True):
        print(json.dumps(model_case(codex, scratch, port, exclude)))
    print(json.dumps(refusal_case(codex, scratch, port)))


main()
