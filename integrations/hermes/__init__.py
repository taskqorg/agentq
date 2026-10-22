"""AgentQ terminal-result adapter and model-call reporter; decisions, originals and stats belong to the binary."""
import json
import subprocess

_AGENTQ = @AGENTQ_EXE@
_USAGE = ("input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens")


def transform(*, tool_name, args, result, session_id=None, turn_id=None, tool_call_id=None, **_):
    if tool_name != "terminal" or not isinstance(result, str):
        return None
    try:
        native = json.loads(result)
        if not isinstance(native, dict) or not isinstance(native.get("output"), str):
            return None
        payload = dict(adapter="hook", tool="terminal", command=args.get("command"),
                       text=native["output"], exit_code=native.get("exit_code"),
                       session=session_id, turn=turn_id, tool_call=tool_call_id)
        proc = subprocess.run([_AGENTQ, "compress", "hermes"], input=json.dumps(payload),
                              text=True, capture_output=True, timeout=4)
        if proc.returncode:
            return None
        compressed = json.loads(proc.stdout)
        if not isinstance(compressed.get("text"), str) or compressed["text"] == native["output"]:
            return None
        native["output"] = compressed["text"]
        return json.dumps(native, ensure_ascii=False)
    except (OSError, ValueError, TypeError, AttributeError, subprocess.TimeoutExpired):
        return None


def account(*, api_request_id=None, session_id=None, turn_id=None, model=None, response_model=None,
            usage=None, ended_at=None, **_):
    """One provider call: IDs, model and token integers only; no request, response or message content."""
    try:
        payload = dict(api_request_id=api_request_id, session_id=session_id, turn_id=turn_id, model=model,
                       response_model=response_model, ended_at=ended_at,
                       usage={k: usage.get(k) for k in _USAGE} if isinstance(usage, dict) else None)
        subprocess.run([_AGENTQ, "hook", "hermes", "post_api_request"], input=json.dumps(payload, default=str),
                       text=True, capture_output=True, timeout=5)
    except (OSError, ValueError, TypeError, subprocess.TimeoutExpired):
        pass


def register(ctx):
    ctx.register_hook("transform_tool_result", transform)
    ctx.register_hook("post_api_request", account)
