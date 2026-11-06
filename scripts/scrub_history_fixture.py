#!/usr/bin/env python3
"""Scrub one real native transcript into a step fixture (docs/spec/RUNTIME.md § Steps).

Usage: scrub_history_fixture.py --source <file> --out <dir> --name <fixture> [--cap <n>] [--first-n <n>]

The script keeps record structure, numbers and protocol values. It replaces every
other string, maps IDs consistently and fails when a private marker survives.
Run it only on your own transcripts; review the output before committing it.
"""
import argparse
import datetime
import hashlib
import json
import pathlib
import re
import sys

# String values under these keys are protocol values, not content.
PROTOCOL_KEYS = {
    "type", "role", "status", "subtype", "phase", "stop_reason", "hookEvent", "trigger", "operation", "commandMode",
    "thread_source", "originator", "cli_version", "version", "model", "model_provider", "permissionMode", "mode",
    "history_mode", "kind", "source", "reasonType", "decision", "taskType", "turn_trigger", "state", "step_type",
    "event", "method", "media_type", "action", "userType", "entrypoint", "sessionKind", "turnOrigin", "promptSource",
    "tool", "server", "agentType", "resolvedModel", "behavior", "name", "tool_name",
}
# A value under `name` or `tool` is protocol only for these known native names.
KNOWN_NAMES = {
    "Bash", "PowerShell", "Monitor", "Read", "Write", "Edit", "MultiEdit", "NotebookEdit", "Grep", "Glob", "Agent", "Task",
    "WebSearch", "WebFetch", "AskUserQuestion", "TaskStop", "ToolSearch", "exec", "wait", "sleep", "spawn_agent", "send_message",
    "followup_task", "wait_agent", "list_agents", "run_command",
}
ID_RE = re.compile(
    r"^(toolu_|msg_|call_|exec-|rs_|ctc_|ctco_|fc_|resp_|cse_|agent-)?"
    r"([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}|[0-9a-zA-Z]{16,})$"
)
ID_KEYS = {
    "id", "uuid", "parentUuid", "logicalParentUuid", "sessionId", "session_id", "thread_id", "threadId", "turn_id", "turnId",
    "root_turn_id", "parent_thread_id", "forked_from_id", "agent_thread_id", "agentId", "agentID", "tool_use_id", "toolUseID", "toolUseId",
    "call_id", "promptId", "itemId", "response_id", "backgroundTaskId", "taskId", "task_id", "shellId", "request_id", "requestId",
    "sender_thread_id", "receiver_thread_ids", "window_id", "first_window_id", "previous_window_id", "source_uuid", "delivery_id",
    "sourceToolAssistantUUID", "conversation_id", "tool_call_id",
}
TIME_KEYS = {"timestamp", "created_at"}
PRIVATE = re.compile(r"alexk|alex\.kirs|C:\\\\Users|/Users/|/home/|gmail|Bangkok", re.I)
BASE = datetime.datetime(2000, 1, 1, tzinfo=datetime.timezone.utc)


class Scrubber:
    def __init__(self):
        self.ids = {}
        self.texts = 0
        self.epoch = None

    def mapped(self, value):
        known = self.ids.get(value)
        if known:
            return known
        match = ID_RE.match(value)
        prefix = (match.group(1) or "") if match else ""
        known = f"{prefix}fixture-{len(self.ids) + 1}"
        self.ids[value] = known
        return known

    def text(self, value):
        if value == "":
            return ""
        self.texts += 1
        kept = []
        exit_code = re.match(r"(Error: )?Exit code -?\d+", value)
        if exit_code:
            kept.append(exit_code.group(0))
        background = re.search(r"(with ID: |\(task )([0-9a-zA-Z_-]+)", value)
        if background:
            kept.append(background.group(1) + self.mapped(background.group(2)))
        return " ".join(kept + [f"scrubbed text {self.texts}"])

    def notification(self, value):
        def tag(match):
            name, body = match.group(1), match.group(2)
            if name in ("task-id", "tool-use-id"):
                return f"<{name}>{self.mapped(body)}</{name}>"
            if name == "status":
                return match.group(0)
            if name == "summary":
                code = re.search(r"\(exit code -?\d+\)", body)
                return f"<summary>{self.text('x')}{' ' + code.group(0) if code else ''}</summary>"
            return f"<{name}>{self.text('x')}</{name}>"
        inner = re.search(r"<task-notification>(.*?)</task-notification>", value, re.S)
        body = re.sub(r"<([a-z-]+)>(.*?)</\1>", tag, inner.group(1), flags=re.S)
        return f"<task-notification>{body}</task-notification>"

    def time(self, value):
        try:
            at = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            return self.text(value)
        self.epoch = self.epoch or at
        moved = BASE + (at - self.epoch)
        return moved.strftime("%Y-%m-%dT%H:%M:%S.") + f"{moved.microsecond // 1000:03d}Z"

    def string(self, key, value):
        if key in TIME_KEYS:
            return self.time(value)
        if value.startswith("<task-notification>") and "</task-notification>" in value:
            return self.notification(value)
        if key in ID_KEYS:
            return self.mapped(value)
        if key in ("name", "tool", "tool_name", "server"):
            return value if value in KNOWN_NAMES else self.text(value)
        if key == "source" and not re.fullmatch(r"[a-z_A-Z]+", value):
            return self.text(value)
        if key == "method" and re.fullmatch(r"[A-Za-z/]+", value):
            return value
        if key in PROTOCOL_KEYS and len(value) <= 64 and not PRIVATE.search(value) and not re.search(r"[\\/ ]", value):
            return value
        if key in ("cwd", "path", "file_path", "filePath", "filename", "move_path", "displayPath", "persistedOutputPath", "outputFilePath", "output-file"):
            self.texts += 1
            prefix = "file:///" if value.startswith("file://") else "/"
            return f"{prefix}fixture/path-{self.texts}"
        if re.fullmatch(r"-[A-Za-z]+", value):
            return value
        if ID_RE.match(value) and len(value) <= 80:
            return self.mapped(value)
        return self.text(value)

    def value(self, key, value):
        if isinstance(value, str):
            return self.string(key, value)
        if isinstance(value, list):
            return [self.value(key, v) for v in value]
        if isinstance(value, dict):
            if key == "changes":
                return {self.string("path", k): self.value(k, v) for k, v in value.items()}
            return {k: self.value(k, v) for k, v in value.items()}
        return value


def record_key(row):
    p = row.get("payload") if isinstance(row.get("payload"), dict) else {}
    kind = str(row.get("type") or row.get("method"))
    if kind == "event_msg":
        item = p.get("item")
        return f"event:{p.get('type')}:{item.get('type') if isinstance(item, dict) else ''}"
    if kind == "response_item":
        return f"response:{p.get('type')}"
    if kind in ("user", "assistant"):
        content = (row.get("message") or {}).get("content")
        first = content[0] if isinstance(content, list) and content else {}
        name = first.get("name", "") if isinstance(first, dict) else ""
        error = first.get("is_error", "") if isinstance(first, dict) else ""
        if isinstance(first, dict) and str(first.get("content", "")).startswith("Exit code"):
            error = "exit"
        background = "bg" if isinstance(row.get("toolUseResult"), dict) and row["toolUseResult"].get("backgroundTaskId") else ""
        return f"{kind}:{first.get('type') if isinstance(first, dict) else 'text'}:{name}:{error}:{background}"
    if kind == "attachment":
        return f"attachment:{(row.get('attachment') or {}).get('type')}"
    if kind == "system":
        return f"system:{row.get('subtype')}"
    return kind


def links(text):
    return set(re.findall(r"toolu_[0-9A-Za-z]+|call_[0-9A-Za-z]+|exec-[0-9a-f-]{36}", text))


def select(rows, cap, first_n):
    counts = {}
    keep = set(range(min(first_n, len(rows))))
    for index, (_, row) in enumerate(rows):
        key = record_key(row)
        counts[key] = counts.get(key, 0) + 1
        if counts[key] <= cap:
            keep.add(index)
    related = set()
    for index in keep:
        related |= links(rows[index][0])
    for index, (raw, _) in enumerate(rows):
        if links(raw) & related:
            keep.add(index)
    return [rows[i][1] for i in sorted(keep)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--cap", type=int, default=2)
    parser.add_argument("--first-n", type=int, default=0)
    parser.add_argument("--version", default="")
    args = parser.parse_args()
    source = pathlib.Path(args.source).read_bytes()
    rows = []
    for line in source.decode("utf8").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(row, dict):
            rows.append((line, row))
    scrubber = Scrubber()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    kept = select(rows, args.cap, args.first_n)
    lines = [json.dumps(scrubber.value("", row), ensure_ascii=False) for row in kept]
    body = "\n".join(lines) + "\n"
    leak = PRIVATE.search(body)
    if leak:
        sys.exit(f"private marker survived: {leak.group(0)}")
    (out / f"{args.name}.jsonl").write_text(body, encoding="utf8", newline="\n")
    provenance_path = out / "provenance.json"
    provenance = json.loads(provenance_path.read_text(encoding="utf8")) if provenance_path.exists() else []
    provenance = [p for p in provenance if p["fixture"] != f"{args.name}.jsonl"]
    provenance.append({
        "fixture": f"{args.name}.jsonl",
        "origin": "real native transcript",
        "native_version": args.version,
        "source_sha256": hashlib.sha256(source).hexdigest(),
        "source_rows": len(rows),
        "rows": len(kept),
        "redaction": "scripts/scrub_history_fixture.py: protocol values, numbers and structure kept; IDs mapped consistently; other strings replaced; times moved to 2000-01-01.",
    })
    provenance.sort(key=lambda p: p["fixture"])
    provenance_path.write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf8", newline="\n")
    print(f"{args.name}: {len(kept)} of {len(rows)} rows")


if __name__ == "__main__":
    main()
