"""AgentQ model-call and session-end reporter."""
import json
import subprocess

_AGENTQ = @AGENTQ_EXE@
_USAGE = ("input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens", "reasoning_tokens")


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


def turn_end(*, session_id=None, completed=None, interrupted=None, **_):
    """One `run_conversation` ended: its session and end state only; no reason text or content."""
    try:
        payload = dict(session_id=session_id, completed=completed, interrupted=interrupted)
        subprocess.run([_AGENTQ, "hook", "hermes", "on_session_end"], input=json.dumps(payload, default=str),
                       text=True, capture_output=True, timeout=5)
    except (OSError, ValueError, TypeError, subprocess.TimeoutExpired):
        pass


def register(ctx):
    ctx.register_hook("post_api_request", account)
    ctx.register_hook("on_session_end", turn_end)
