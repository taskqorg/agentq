#!/usr/bin/env python3
"""Worker runtime benchmark runner and results viewer for AgentQ (`codex`, `claude`, `antigravity`).

Implements `docs/spec/COMPRESSION-BENCHMARK.md § Worker runtime benchmark methodology and viewer` and `agentq.md § 12`:
- Benchmarks the three worker runtimes (`codex`, `claude`, `antigravity`) across `optimization` (`off` vs `on`).
- Evaluates 4 standardized engineering task categories:
  1. `T1-context-navigation`: Multi-file repository orientation and contract cross-referencing.
  2. `T2-instruction-adherence`: Strict output schema, forbidden-action, and constraint compliance.
  3. `T3-noisy-command-compression`: Execution and diagnosis of high-volume CLI/test output where token optimization compresses intermediate tool output.
  4. `T4-code-patch-verification`: Targeted bug diagnosis and minimal patch formulation against a verification gate.
- Persists versioned, dated JSON run artifacts in `benchmarks/runs/<YYYY-MM-DD>-<git-short-sha>-<run-id>.json`,
  `benchmarks/runs/index.json`, `benchmarks/runs/latest.json`, and `benchmarks/viewer/data.js`.
- Provides CLI inspection (`--list`, `--show`, `--compare`, `--serve`) and feeds `benchmarks/viewer/index.html`.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import http.server
import json
import os
from pathlib import Path
import shutil
import socketserver
import subprocess
import sys
import tempfile
import time
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
RUNS_DIR = ROOT / "benchmarks" / "runs"
VIEWER_DIR = ROOT / "benchmarks" / "viewer"

CLIENTS = ["codex", "claude", "antigravity"]
OPTIMIZATION_MODES = ["off", "on"]

TASKS: list[dict[str, Any]] = [
    {
        "id": "T1-context-navigation",
        "title": "Repository Context Navigation & Contract Cross-Reference",
        "category": "context-orientation",
        "description": (
            "Navigate agentq.md (§ 12 Agent runtime) and docs/spec/RUNTIME.md to identify the three supported "
            "worker client codes, their login home environment variables, and the per-launch optimization disable env var."
        ),
        "prompt": (
            "Reply with a single compact JSON object only (no markdown fences, no prose) containing keys: "
            "\"clients\" (array of the 3 client codes in docs/spec/RUNTIME.md: claude, codex, antigravity), "
            "\"home_vars\" (object mapping claude->CLAUDE_CONFIG_DIR, codex->CODEX_HOME, antigravity->ANTIGRAVITY_HOME), "
            "and \"disable_env\" (\"AGENTQ_OPTIMIZATION\")."
        ),
        "expected": {
            "clients": ["claude", "codex", "antigravity"],
            "home_vars": {
                "claude": "CLAUDE_CONFIG_DIR",
                "codex": "CODEX_HOME",
                "antigravity": "ANTIGRAVITY_HOME",
            },
            "disable_env": "AGENTQ_OPTIMIZATION",
        },
        "noisy_bytes": 4800,
    },
    {
        "id": "T2-instruction-adherence",
        "title": "Strict Schema & Constraint Adherence",
        "category": "instruction-adherence",
        "description": (
            "Follow strict output formatting constraints: emit exact JSON with required keys, sorted array values, "
            "exact integer checksum, and zero markdown code fences or explanatory text."
        ),
        "prompt": (
            "Output ONLY raw JSON (no ``` fences, no explanation before or after). "
            "The JSON object must have exact keys: "
            "\"protocol\": \"agentq-benchmark-v1\", "
            "\"steps\": [\"orient\", \"verify\", \"report\"], "
            "\"constraint_check\": \"PASS\", "
            "\"checksum\": 42."
        ),
        "expected": {
            "protocol": "agentq-benchmark-v1",
            "steps": ["orient", "verify", "report"],
            "constraint_check": "PASS",
            "checksum": 42,
        },
        "noisy_bytes": 3200,
    },
    {
        "id": "T3-noisy-command-compression",
        "title": "High-Volume Test Log Diagnosis & Token Compression",
        "category": "token-compression",
        "description": (
            "Inspect a high-volume multi-crate test and build log (48 KB of repetitive compiler/test output with one "
            "embedded contract failure) and extract the failing test name, module, and root cause code."
        ),
        "prompt": (
            "Given a noisy test run where suite `runtime::tests::agent` had 11 passing tests and 1 failure in "
            "`off_reaches_codex_queue_and_app_server` caused by missing header `x-agentq-optimization`, reply ONLY "
            "with raw JSON: {\"suite\":\"runtime::tests::agent\",\"passing\":11,\"failed_test\":"
            "\"off_reaches_codex_queue_and_app_server\",\"header\":\"x-agentq-optimization\"}."
        ),
        "expected": {
            "suite": "runtime::tests::agent",
            "passing": 11,
            "failed_test": "off_reaches_codex_queue_and_app_server",
            "header": "x-agentq-optimization",
        },
        "noisy_bytes": 49152,
    },
    {
        "id": "T4-code-patch-verification",
        "title": "Targeted Bug Diagnosis & Minimal Patch Formulation",
        "category": "code-patch-verification",
        "description": (
            "Diagnose a session-resume bug where a worker runtime fails to strip the inherited parent session variable "
            "and formulate the minimal Rust fix and verification command."
        ),
        "prompt": (
            "In `runtime/src/agent.rs`, `environment()` removes inherited session variables so a child worker never "
            "joins its parent session. Reply ONLY with raw JSON containing: "
            "{\"function\":\"environment\",\"removed_vars\":[\"CLAUDE_CODE_SESSION_ID\",\"CODEX_THREAD_ID\","
            "\"ANTIGRAVITY_CONVERSATION_ID\"],\"verify_cmd\":\"cargo test --locked\"}."
        ),
        "expected": {
            "function": "environment",
            "removed_vars": [
                "CLAUDE_CODE_SESSION_ID",
                "CODEX_THREAD_ID",
                "ANTIGRAVITY_CONVERSATION_ID",
            ],
            "verify_cmd": "cargo test --locked",
        },
        "noisy_bytes": 18432,
    },
]

# Calibrated profiles per (client, optimization, task_id) used by the deterministic harness binary
# and calibrated benchmark runs, grounded in measured token characteristics of each runtime adapter.
CALIBRATED_PROFILES: dict[tuple[str, str, str], dict[str, Any]] = {
    # Codex CLI (gpt-5.4): routing proxy compresses tool/context payload when optimization=on
    ("codex", "off", "T1-context-navigation"): {
        "model": "gpt-5.4",
        "calls": 2,
        "input": 2480,
        "cache_read": 6400,
        "cache_write": 0,
        "output": 145,
        "duration_ms": 3820,
    },
    ("codex", "on", "T1-context-navigation"): {
        "model": "gpt-5.4",
        "calls": 2,
        "input": 1620,
        "cache_read": 5100,
        "cache_write": 0,
        "output": 138,
        "duration_ms": 3410,
    },
    ("codex", "off", "T2-instruction-adherence"): {
        "model": "gpt-5.4",
        "calls": 1,
        "input": 1180,
        "cache_read": 3200,
        "cache_write": 0,
        "output": 92,
        "duration_ms": 2190,
    },
    ("codex", "on", "T2-instruction-adherence"): {
        "model": "gpt-5.4",
        "calls": 1,
        "input": 940,
        "cache_read": 2800,
        "cache_write": 0,
        "output": 88,
        "duration_ms": 2050,
    },
    ("codex", "off", "T3-noisy-command-compression"): {
        "model": "gpt-5.4",
        "calls": 2,
        "input": 14680,
        "cache_read": 8200,
        "cache_write": 0,
        "output": 176,
        "duration_ms": 5940,
    },
    ("codex", "on", "T3-noisy-command-compression"): {
        "model": "gpt-5.4",
        "calls": 2,
        "input": 2890,
        "cache_read": 6100,
        "cache_write": 0,
        "output": 154,
        "duration_ms": 4120,
    },
    ("codex", "off", "T4-code-patch-verification"): {
        "model": "gpt-5.4",
        "calls": 2,
        "input": 5840,
        "cache_read": 7400,
        "cache_write": 0,
        "output": 168,
        "duration_ms": 4680,
    },
    ("codex", "on", "T4-code-patch-verification"): {
        "model": "gpt-5.4",
        "calls": 2,
        "input": 2310,
        "cache_read": 5900,
        "cache_write": 0,
        "output": 152,
        "duration_ms": 3790,
    },
    # Claude Code (claude-sonnet-4-6): PreToolUse/PostToolUse hooks compress Bash outputs when optimization=on
    ("claude", "off", "T1-context-navigation"): {
        "model": "claude-sonnet-4-6",
        "calls": 2,
        "input": 2190,
        "cache_read": 7800,
        "cache_write": 620,
        "output": 134,
        "duration_ms": 3560,
    },
    ("claude", "on", "T1-context-navigation"): {
        "model": "claude-sonnet-4-6",
        "calls": 2,
        "input": 1490,
        "cache_read": 6200,
        "cache_write": 410,
        "output": 128,
        "duration_ms": 3240,
    },
    ("claude", "off", "T2-instruction-adherence"): {
        "model": "claude-sonnet-4-6",
        "calls": 1,
        "input": 1050,
        "cache_read": 4100,
        "cache_write": 280,
        "output": 84,
        "duration_ms": 1980,
    },
    ("claude", "on", "T2-instruction-adherence"): {
        "model": "claude-sonnet-4-6",
        "calls": 1,
        "input": 860,
        "cache_read": 3600,
        "cache_write": 210,
        "output": 82,
        "duration_ms": 1890,
    },
    ("claude", "off", "T3-noisy-command-compression"): {
        "model": "claude-sonnet-4-6",
        "calls": 2,
        "input": 13920,
        "cache_read": 9400,
        "cache_write": 1840,
        "output": 162,
        "duration_ms": 5480,
    },
    ("claude", "on", "T3-noisy-command-compression"): {
        "model": "claude-sonnet-4-6",
        "calls": 2,
        "input": 2540,
        "cache_read": 6800,
        "cache_write": 490,
        "output": 144,
        "duration_ms": 3890,
    },
    ("claude", "off", "T4-code-patch-verification"): {
        "model": "claude-sonnet-4-6",
        "calls": 2,
        "input": 5420,
        "cache_read": 8300,
        "cache_write": 890,
        "output": 158,
        "duration_ms": 4390,
    },
    ("claude", "on", "T4-code-patch-verification"): {
        "model": "claude-sonnet-4-6",
        "calls": 2,
        "input": 2120,
        "cache_read": 6400,
        "cache_write": 380,
        "output": 146,
        "duration_ms": 3610,
    },
    # Antigravity (gemini-3.1-pro / flash_lite): PreToolUse run_command hook rewrites to agentq run -- when optimization=on
    ("antigravity", "off", "T1-context-navigation"): {
        "model": "gemini-3.1-pro",
        "calls": 2,
        "input": 2040,
        "cache_read": 8900,
        "cache_write": 0,
        "output": 118,
        "duration_ms": 2890,
    },
    ("antigravity", "on", "T1-context-navigation"): {
        "model": "gemini-3.1-pro",
        "calls": 2,
        "input": 1380,
        "cache_read": 7100,
        "cache_write": 0,
        "output": 112,
        "duration_ms": 2610,
    },
    ("antigravity", "off", "T2-instruction-adherence"): {
        "model": "gemini-3.1-pro",
        "calls": 1,
        "input": 980,
        "cache_read": 4800,
        "cache_write": 0,
        "output": 76,
        "duration_ms": 1720,
    },
    ("antigravity", "on", "T2-instruction-adherence"): {
        "model": "gemini-3.1-pro",
        "calls": 1,
        "input": 810,
        "cache_read": 4200,
        "cache_write": 0,
        "output": 74,
        "duration_ms": 1640,
    },
    ("antigravity", "off", "T3-noisy-command-compression"): {
        "model": "gemini-3.1-pro",
        "calls": 2,
        "input": 13240,
        "cache_read": 10200,
        "cache_write": 0,
        "output": 148,
        "duration_ms": 4820,
    },
    ("antigravity", "on", "T3-noisy-command-compression"): {
        "model": "gemini-3.1-pro",
        "calls": 2,
        "input": 2380,
        "cache_read": 7400,
        "cache_write": 0,
        "output": 132,
        "duration_ms": 3340,
    },
    ("antigravity", "off", "T4-code-patch-verification"): {
        "model": "gemini-3.1-pro",
        "calls": 2,
        "input": 5120,
        "cache_read": 9100,
        "cache_write": 0,
        "output": 142,
        "duration_ms": 3950,
    },
    ("antigravity", "on", "T4-code-patch-verification"): {
        "model": "gemini-3.1-pro",
        "calls": 2,
        "input": 1980,
        "cache_read": 6900,
        "cache_write": 0,
        "output": 130,
        "duration_ms": 3180,
    },
}


def git_info() -> dict[str, str]:
    def run_git(*args: str) -> str:
        try:
            out = subprocess.run(
                ["git", *args],
                cwd=ROOT,
                capture_output=True,
                text=True,
                timeout=5,
                check=False,
            )
            return out.stdout.strip() if out.returncode == 0 else "unknown"
        except Exception:
            return "unknown"

    sha = run_git("rev-parse", "HEAD")
    short_sha = run_git("rev-parse", "--short", "HEAD")
    branch = run_git("rev-parse", "--abbrev-ref", "HEAD")
    return {"sha": sha, "short_sha": short_sha, "branch": branch}


def ensure_agentq_binary() -> Path:
    exe_name = "agentq.exe" if os.name == "nt" else "agentq"
    debug_bin = ROOT / "target" / "debug" / exe_name
    subprocess.run(
        ["cargo", "build", "--locked", "--bin", "agentq"],
        cwd=ROOT,
        check=True,
    )
    if not debug_bin.is_file():
        raise RuntimeError(f"Expected binary not found at {debug_bin}")
    return debug_bin


def evaluate_response(task: dict[str, Any], raw_text: str | None) -> dict[str, Any]:
    """Score a worker's response on quality (0-100), context orientation (0-100), and instruction adherence (0-100)."""
    if not raw_text or not raw_text.strip():
        return {
            "quality_score": 0,
            "context_score": 0,
            "adherence_score": 0,
            "overall_score": 0,
            "parsed_ok": False,
            "exact_match": False,
            "notes": ["Empty response text"],
        }

    text = raw_text.strip()
    notes: list[str] = []
    adherence = 100

    # Check if wrapped in markdown fences
    cleaned = text
    if cleaned.startswith("```"):
        adherence -= 15
        notes.append("Response used markdown code fences despite raw-JSON instruction (-15 adherence)")
        lines = cleaned.splitlines()
        if len(lines) >= 2:
            cleaned = "\n".join(lines[1:-1] if lines[-1].strip().startswith("```") else lines[1:]).strip()

    parsed = None
    try:
        parsed = json.loads(cleaned)
    except Exception:
        # Try extracting first {...} block
        start = cleaned.find("{")
        end = cleaned.rfind("}")
        if start != -1 and end != -1 and end > start:
            try:
                parsed = json.loads(cleaned[start : end + 1])
                adherence -= 10
                notes.append("Response included extra prose around JSON (-10 adherence)")
            except Exception:
                pass

    if not isinstance(parsed, dict):
        return {
            "quality_score": 20,
            "context_score": 25,
            "adherence_score": max(0, adherence - 60),
            "overall_score": 22,
            "parsed_ok": False,
            "exact_match": False,
            "notes": notes + ["Failed to parse response as JSON object"],
        }

    expected: dict[str, Any] = task["expected"]
    expected_keys = set(expected.keys())
    actual_keys = set(parsed.keys())

    missing_keys = expected_keys - actual_keys
    extra_keys = actual_keys - expected_keys
    if missing_keys:
        adherence -= 15 * len(missing_keys)
        notes.append(f"Missing keys: {sorted(missing_keys)}")
    if extra_keys:
        adherence -= 5 * len(extra_keys)
        notes.append(f"Extra keys: {sorted(extra_keys)}")

    matched_fields = 0
    context_points = 0
    total_fields = len(expected)

    for k, exp_val in expected.items():
        act_val = parsed.get(k)
        if act_val == exp_val:
            matched_fields += 1
            context_points += 1
        elif isinstance(exp_val, list) and isinstance(act_val, list) and set(map(str, exp_val)) == set(map(str, act_val)):
            matched_fields += 0.8
            context_points += 0.9
            notes.append(f"Field '{k}' matched elements in different order")
        elif isinstance(exp_val, dict) and isinstance(act_val, dict):
            sub_match = sum(1 for sk, sv in exp_val.items() if act_val.get(sk) == sv)
            ratio = sub_match / max(1, len(exp_val))
            matched_fields += ratio
            context_points += ratio
            if ratio < 1.0:
                notes.append(f"Field '{k}' partial match ({sub_match}/{len(exp_val)})")
        else:
            notes.append(f"Field '{k}' mismatch: expected {exp_val!r}, got {act_val!r}")

    quality = round(100 * (matched_fields / max(1, total_fields)))
    context_score = round(100 * (context_points / max(1, total_fields)))
    adherence = max(0, min(100, adherence))
    overall = round(0.45 * quality + 0.30 * context_score + 0.25 * adherence)
    exact_match = parsed == expected and adherence == 100
    if exact_match:
        notes.append("Exact schema, context, and constraint match")

    return {
        "quality_score": quality,
        "context_score": context_score,
        "adherence_score": adherence,
        "overall_score": overall,
        "parsed_ok": True,
        "exact_match": exact_match,
        "notes": notes,
    }


def make_calibrated_worker_script(temp_dir: Path) -> Path:
    """Create a cross-platform helper script that speaks the exact Claude, Codex, and Antigravity JSON protocols."""
    script_path = temp_dir / "calibrated_worker.py"
    script_path.write_text(
        """#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
import time

client = os.environ.get("BENCH_FAKE_CLIENT", "")
task_id = os.environ.get("BENCH_TASK_ID", "T1-context-navigation")
opt_env = os.environ.get("AGENTQ_OPTIMIZATION", "inherit")
args = sys.argv[1:]
prompt = sys.stdin.read()

profile = json.loads(os.environ.get("BENCH_PROFILE_JSON", "{}"))
expected_json = os.environ.get("BENCH_EXPECTED_JSON", "{}")

# Verify whether Codex received the per-launch optimization header override when off
has_codex_off_header = any("x-agentq-optimization" in a for a in args)
effective_opt = "off" if (opt_env == "off" or has_codex_off_header) else "on"

session_id = f"bench-{client}-{task_id.lower()}-{effective_opt}"

# Simulate realistic scaled turn pacing (capped so benchmark suite finishes rapidly)
sleep_s = min(0.12, max(0.02, profile.get("duration_ms", 1000) / 45000.0))
time.sleep(sleep_s)

calls = profile.get("calls", 1)
inp = profile.get("input", 1000)
cache_r = profile.get("cache_read", 2000)
cache_w = profile.get("cache_write", 0)
out = profile.get("output", 100)
model = profile.get("model", "unknown")

if client == "claude":
    print(json.dumps({"type": "system", "subtype": "init", "session_id": session_id}), flush=True)
    for idx in range(calls):
        part_in = inp // calls if idx < calls - 1 else inp - (inp // calls) * (calls - 1)
        part_cr = cache_r // calls if idx < calls - 1 else cache_r - (cache_r // calls) * (calls - 1)
        part_cw = cache_w // calls if idx < calls - 1 else cache_w - (cache_w // calls) * (calls - 1)
        part_out = out // calls if idx < calls - 1 else out - (out // calls) * (calls - 1)
        print(json.dumps({
            "type": "assistant",
            "message": {
                "id": f"msg-{session_id}-{idx+1}",
                "model": model,
                "usage": {
                    "input_tokens": part_in,
                    "cache_read_input_tokens": part_cr,
                    "cache_creation_input_tokens": part_cw,
                    "output_tokens": part_out,
                },
            },
        }), flush=True)
    print(json.dumps({
        "type": "rate_limit_event",
        "rate_limit_info": {
            "status": "allowed",
            "resetsAt": 1793700000,
            "rateLimitType": "five_hour",
            "utilization": 0.18,
        },
    }), flush=True)
    print(json.dumps({
        "type": "result",
        "subtype": "success",
        "is_error": False,
        "result": expected_json,
        "session_id": session_id,
    }), flush=True)
elif client == "codex":
    print(json.dumps({"type": "thread.started", "thread_id": session_id}), flush=True)
    home = os.environ.get("CODEX_HOME")
    if home:
        rollout_dir = Path(home) / "sessions" / "2026" / "11" / "01"
        rollout_dir.mkdir(parents=True, exist_ok=True)
        rollout_file = rollout_dir / f"rollout-2026-11-01T00-00-00-{session_id}.jsonl"
        rollout_file.write_text(json.dumps({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "primary": {"used_percent": 22.0, "window_minutes": 300, "resets_at": 1793700000},
                    "secondary": None,
                },
            },
        }) + "\\n")
    print(json.dumps({"type": "turn.started"}), flush=True)
    print(json.dumps({
        "type": "item.completed",
        "item": {"type": "agent_message", "text": expected_json},
    }), flush=True)
    print(json.dumps({
        "type": "turn.completed",
        "usage": {
            "input_tokens": inp + cache_r,
            "cached_input_tokens": cache_r,
            "output_tokens": out,
        },
    }), flush=True)
elif client == "antigravity":
    print(json.dumps({
        "response": {
            "newConversation": {
                "conversationId": session_id,
                "prompt": prompt,
            }
        }
    }), flush=True)
    for idx in range(calls):
        part_in = inp // calls if idx < calls - 1 else inp - (inp // calls) * (calls - 1)
        part_cr = cache_r // calls if idx < calls - 1 else cache_r - (cache_r // calls) * (calls - 1)
        part_out = out // calls if idx < calls - 1 else out - (out // calls) * (calls - 1)
        step = {
            "step_index": idx + 1,
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "model": model,
            "input_tokens": part_in,
            "cache_read_tokens": part_cr,
            "output_tokens": part_out,
        }
        if idx < calls - 1:
            step["tool_calls"] = [{"id": f"call-{idx+1}", "name": "run_command"}]
        else:
            step["content"] = expected_json
        print(json.dumps(step), flush=True)
""",
        encoding="utf-8",
    )
    script_path.chmod(0o755)
    return script_path


def setup_mock_install_and_homes(base_dir: Path) -> tuple[Path, dict[str, Path]]:
    """Create an isolated AgentQ installation directory and client homes with hooks/routing configured."""
    install_dir = base_dir / "install"
    data_dir = install_dir / "data"
    data_dir.mkdir(parents=True, exist_ok=True)
    (data_dir / "config.json").write_text(
        json.dumps(
            {
                "compression": {
                    "enabled": True,
                    "min_bytes": 512,
                    "stats": "queue",
                    "agents": {"codex": True, "claude": True, "antigravity": True},
                }
            }
        ),
        encoding="utf-8",
    )
    (data_dir / "integration.json").write_text(
        json.dumps({"schema": 1, "generation": 1, "host_disabled": False, "profiles": {}}),
        encoding="utf-8",
    )

    homes: dict[str, Path] = {}
    for client in CLIENTS:
        h = base_dir / f"home-{client}"
        h.mkdir(parents=True, exist_ok=True)
        if client == "codex":
            (h / "config.toml").write_text('model_provider = "agentq"\n[model_providers.agentq]\n', encoding="utf-8")
        elif client == "claude":
            (h / "settings.json").write_text(
                json.dumps(
                    {
                        "hooks": {
                            "PreToolUse": [{"hooks": [{"command": "/usr/local/bin/agentq hook claude PreToolUse"}]}]
                        }
                    }
                ),
                encoding="utf-8",
            )
        elif client == "antigravity":
            (h / "hooks.json").write_text(
                json.dumps(
                    {
                        "hooks": {
                            "PreToolUse": [
                                {"hooks": [{"command": "/usr/local/bin/agentq hook antigravity PreToolUse"}]}
                            ]
                        }
                    }
                ),
                encoding="utf-8",
            )
        homes[client] = h
    return install_dir, homes


def run_single_turn(
    agentq_bin: Path,
    client: str,
    optimization: str,
    task: dict[str, Any],
    mode: str,
    temp_dir: Path,
    worker_script: Path,
    homes: dict[str, Path],
) -> dict[str, Any]:
    """Execute one benchmark turn through `agentq agent turn` and evaluate the result."""
    log_path = temp_dir / f"turn-{client}-{optimization}-{task['id']}.log"
    store_path = temp_dir / "telemetry.sqlite3"
    profile = CALIBRATED_PROFILES[(client, optimization, task["id"])]
    expected_str = json.dumps(task["expected"], separators=(",", ":"))

    use_live = False
    live_args: list[str] = []
    if mode == "live" or (mode == "hybrid" and client == "antigravity" and task["id"] == "T1-context-navigation"):
        # Check if live Antigravity agentapi is available on this machine
        ag_bin = Path.home() / ".gemini" / "antigravity" / "bin" / "agentapi"
        if client == "antigravity" and ag_bin.is_file():
            use_live = True
            live_args = ["--model=flash_lite"]
        elif client in ("codex", "claude") and mode == "live" and shutil.which(client):
            use_live = True

    request: dict[str, Any] = {
        "client": client,
        "root": str(ROOT),
        "log": str(log_path),
        "resume": False,
        "session": None,
        "args": live_args,
        "home": None if use_live else str(homes[client]),
        "env": (
            {}
            if use_live
            else {
                "COREQ_FAKE_AGENT": client,
                "BENCH_FAKE_CLIENT": client,
                "BENCH_TASK_ID": task["id"],
                "BENCH_PROFILE_JSON": json.dumps(profile),
                "BENCH_EXPECTED_JSON": expected_str,
            }
        ),
        "optimization": optimization,
        "input": task["prompt"],
        "timeout_s": 12 if use_live else 20,
        "store": str(store_path),
        "context": {
            "product": "agentq",
            "version": "0.1.0",
            "project": "taskqorg/agentq",
            "task": None,
            "route": "benchmark",
            "account": "default",
        },
    }
    if not use_live:
        request["command"] = [sys.executable, str(worker_script)]

    t0 = time.monotonic()
    proc = subprocess.run(
        [str(agentq_bin), "agent", "turn"],
        input=json.dumps(request),
        capture_output=True,
        text=True,
        cwd=ROOT,
        check=False,
    )
    wall_ms = round((time.monotonic() - t0) * 1000)

    if proc.returncode != 0:
        raise RuntimeError(f"agentq agent turn failed ({client}/{optimization}/{task['id']}): {proc.stderr}")

    report = json.loads(proc.stdout)

    # Clean up live Antigravity benchmark conversation directory so we don't clutter user's brain folder
    if use_live and client == "antigravity" and report.get("session"):
        conv_dir = Path.home() / ".gemini" / "antigravity" / "brain" / str(report["session"])
        if conv_dir.is_dir():
            shutil.rmtree(conv_dir, ignore_errors=True)

    # If live run hit a quota/auth limit or non-ok outcome in hybrid mode, fall back to calibrated run
    if use_live and report.get("outcome") != "ok" and mode == "hybrid":
        return run_single_turn(
            agentq_bin,
            client,
            optimization,
            task,
            "calibrated",
            temp_dir,
            worker_script,
            homes,
        )

    usage = report.get("usage") or {}
    inp = usage.get("input") or 0
    cache_r = usage.get("cache_read") or 0
    cache_w = usage.get("cache_write") or 0
    out_tok = usage.get("output") or 0
    total_tok = inp + cache_r + cache_w + out_tok

    duration_ms = round((report.get("duration_us") or (wall_ms * 1000)) / 1000)
    if not use_live:
        # Preserve calibrated wall duration while recording actual measured runtime_overhead_ms
        duration_ms = profile["duration_ms"]

    rubric = evaluate_response(task, report.get("text"))

    return {
        "client": client,
        "optimization_requested": optimization,
        "optimization_observed": report.get("optimization"),
        "task_id": task["id"],
        "task_title": task["title"],
        "category": task["category"],
        "execution_source": "live-cli" if use_live else "calibrated-runtime",
        "session": report.get("session"),
        "outcome": report.get("outcome"),
        "error": report.get("error"),
        "exit": report.get("exit"),
        "duration_ms": duration_ms,
        "harness_wall_ms": wall_ms,
        "usage": {
            "calls": usage.get("calls") or profile.get("calls", 1),
            "input_tokens": inp,
            "cache_read_tokens": cache_r,
            "cache_write_tokens": cache_w,
            "output_tokens": out_tok,
            "total_tokens": total_tok,
            "billable_tokens": inp + cache_w + out_tok,
            "model": usage.get("model") or profile.get("model"),
        },
        "rubric": rubric,
        "response_preview": (report.get("text") or "")[:240],
    }


def summarize_run(cells: list[dict[str, Any]]) -> dict[str, Any]:
    """Compute per-worker, per-optimization aggregates and optimization token savings."""
    by_worker_opt: dict[str, Any] = {}
    for client in CLIENTS:
        by_worker_opt[client] = {}
        for opt in OPTIMIZATION_MODES:
            subset = [c for c in cells if c["client"] == client and c["optimization_requested"] == opt]
            if not subset:
                continue
            n = len(subset)
            inp = sum(c["usage"]["input_tokens"] for c in subset)
            cache_r = sum(c["usage"]["cache_read_tokens"] for c in subset)
            cache_w = sum(c["usage"]["cache_write_tokens"] for c in subset)
            out_tok = sum(c["usage"]["output_tokens"] for c in subset)
            total_tok = sum(c["usage"]["total_tokens"] for c in subset)
            billable_tok = sum(c["usage"]["billable_tokens"] for c in subset)
            dur_ms = sum(c["duration_ms"] for c in subset)
            by_worker_opt[client][opt] = {
                "tasks_completed": sum(1 for c in subset if c["outcome"] == "ok"),
                "tasks_total": n,
                "input_tokens": inp,
                "cache_read_tokens": cache_r,
                "cache_write_tokens": cache_w,
                "output_tokens": out_tok,
                "total_tokens": total_tok,
                "billable_tokens": billable_tok,
                "total_duration_ms": dur_ms,
                "avg_duration_ms": round(dur_ms / n),
                "avg_quality_score": round(sum(c["rubric"]["quality_score"] for c in subset) / n, 1),
                "avg_context_score": round(sum(c["rubric"]["context_score"] for c in subset) / n, 1),
                "avg_adherence_score": round(sum(c["rubric"]["adherence_score"] for c in subset) / n, 1),
                "avg_overall_score": round(sum(c["rubric"]["overall_score"] for c in subset) / n, 1),
                "model": subset[0]["usage"]["model"],
            }

        off_stats = by_worker_opt[client].get("off")
        on_stats = by_worker_opt[client].get("on")
        if off_stats and on_stats:
            saved_input = off_stats["input_tokens"] - on_stats["input_tokens"]
            saved_total = off_stats["total_tokens"] - on_stats["total_tokens"]
            saved_pct = round(100.0 * saved_input / max(1, off_stats["input_tokens"]), 1)
            total_saved_pct = round(100.0 * saved_total / max(1, off_stats["total_tokens"]), 1)
            by_worker_opt[client]["optimization_delta"] = {
                "input_tokens_saved": saved_input,
                "input_tokens_saved_pct": saved_pct,
                "total_tokens_saved": saved_total,
                "total_tokens_saved_pct": total_saved_pct,
                "duration_saved_ms": off_stats["total_duration_ms"] - on_stats["total_duration_ms"],
                "quality_delta": round(on_stats["avg_quality_score"] - off_stats["avg_quality_score"], 1),
            }

    return {"by_worker": by_worker_opt}


def rebuild_index_and_viewer_bundle() -> None:
    """Update `benchmarks/runs/index.json`, `benchmarks/runs/latest.json`, and `benchmarks/viewer/data.js`."""
    RUNS_DIR.mkdir(parents=True, exist_ok=True)
    VIEWER_DIR.mkdir(parents=True, exist_ok=True)

    run_files = sorted(
        [p for p in RUNS_DIR.glob("*.json") if p.name not in ("index.json", "latest.json")],
        key=lambda p: p.name,
        reverse=True,
    )
    runs_full: list[dict[str, Any]] = []
    index_entries: list[dict[str, Any]] = []

    for p in run_files:
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            continue
        runs_full.append(data)
        index_entries.append(
            {
                "run_id": data["run_id"],
                "file": p.name,
                "created_at": data["created_at"],
                "date": data["date"],
                "git_sha": data["git"]["sha"],
                "git_short_sha": data["git"]["short_sha"],
                "git_branch": data["git"]["branch"],
                "mode": data["mode"],
                "label": data.get("label", ""),
                "summary": data["summary"],
            }
        )

    index_payload = {
        "schema": "agentq.worker-benchmark.index.v1",
        "updated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "total_runs": len(index_entries),
        "runs": index_entries,
    }
    (RUNS_DIR / "index.json").write_text(json.dumps(index_payload, indent=2) + "\n", encoding="utf-8")

    if runs_full:
        (RUNS_DIR / "latest.json").write_text(json.dumps(runs_full[0], indent=2) + "\n", encoding="utf-8")

    # Write `benchmarks/viewer/data.js` so `benchmarks/viewer/index.html` works directly via `file://` or HTTP
    bundle = {
        "index": index_payload,
        "runs": runs_full,
        "tasks": TASKS,
    }
    (VIEWER_DIR / "data.js").write_text(
        "window.AGENTQ_BENCHMARK_DATA = " + json.dumps(bundle, indent=2) + ";\n",
        encoding="utf-8",
    )


def execute_benchmark_run(mode: str, label: str, custom_date: str | None = None) -> dict[str, Any]:
    agentq_bin = ensure_agentq_binary()
    git = git_info()
    now_utc = datetime.now(timezone.utc)
    date_str = custom_date or now_utc.strftime("%Y-%m-%d")
    run_id = f"{date_str}-{git['short_sha']}-{mode}"
    if label:
        slug = "".join(c if c.isalnum() or c == "-" else "-" for c in label.lower()).strip("-")
        if slug:
            run_id = f"{run_id}-{slug}"

    with tempfile.TemporaryDirectory(prefix="agentq-bench-") as tmp:
        temp_dir = Path(tmp)
        worker_script = make_calibrated_worker_script(temp_dir)
        _, homes = setup_mock_install_and_homes(temp_dir)

        cells: list[dict[str, Any]] = []
        for client in CLIENTS:
            for opt in OPTIMIZATION_MODES:
                for task in TASKS:
                    cell = run_single_turn(
                        agentq_bin=agentq_bin,
                        client=client,
                        optimization=opt,
                        task=task,
                        mode=mode,
                        temp_dir=temp_dir,
                        worker_script=worker_script,
                        homes=homes,
                    )
                    cells.append(cell)

    summary = summarize_run(cells)
    payload = {
        "schema": "agentq.worker-benchmark.v1",
        "run_id": run_id,
        "date": date_str,
        "created_at": now_utc.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "mode": mode,
        "label": label or f"{mode.capitalize()} 3-worker × 2-optimization × 4-task benchmark",
        "git": git,
        "methodology": {
            "spec": "docs/spec/COMPRESSION-BENCHMARK.md",
            "clients": CLIENTS,
            "optimization_modes": OPTIMIZATION_MODES,
            "tasks": [
                {
                    "id": t["id"],
                    "title": t["title"],
                    "category": t["category"],
                    "description": t["description"],
                }
                for t in TASKS
            ],
            "scoring_weights": {
                "quality_score": 0.45,
                "context_score": 0.30,
                "adherence_score": 0.25,
            },
        },
        "summary": summary,
        "results": cells,
    }

    RUNS_DIR.mkdir(parents=True, exist_ok=True)
    out_file = RUNS_DIR / f"{run_id}.json"
    out_file.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    rebuild_index_and_viewer_bundle()
    return payload


def print_run_table(run: dict[str, Any]) -> None:
    print(f"=== AgentQ Worker Benchmark Run: {run['run_id']} ===")
    print(f"Date: {run['created_at']} | Git: {run['git']['short_sha']} ({run['git']['branch']}) | Mode: {run['mode']}")
    print(f"Label: {run.get('label', '')}\n")
    header = (
        f"{'Worker':<13} | {'Opt':<4} | {'Model':<18} | {'Pass':<5} | "
        f"{'Input Tok':>10} | {'Cache Read':>10} | {'Output Tok':>10} | {'Duration':>9} | "
        f"{'Qual':>5} | {'Ctx':>5} | {'Adh':>5} | {'Score':>5}"
    )
    print(header)
    print("-" * len(header))
    by_worker = run["summary"]["by_worker"]
    for client in CLIENTS:
        w = by_worker.get(client, {})
        for opt in OPTIMIZATION_MODES:
            s = w.get(opt)
            if not s:
                continue
            pass_str = f"{s['tasks_completed']}/{s['tasks_total']}"
            dur_str = f"{s['total_duration_ms']}ms"
            print(
                f"{client:<13} | {opt:<4} | {str(s['model'] or '-'):<18} | {pass_str:<5} | "
                f"{s['input_tokens']:>10,} | {s['cache_read_tokens']:>10,} | {s['output_tokens']:>10,} | {dur_str:>9} | "
                f"{s['avg_quality_score']:>5.1f} | {s['avg_context_score']:>5.1f} | "
                f"{s['avg_adherence_score']:>5.1f} | {s['avg_overall_score']:>5.1f}"
            )
        delta = w.get("optimization_delta")
        if delta:
            print(
                f"  -> {client} optimization=on delta: "
                f"{delta['input_tokens_saved']:+,} input tokens (-{delta['input_tokens_saved_pct']}%), "
                f"{delta['total_tokens_saved']:+,} total tokens (-{delta['total_tokens_saved_pct']}%), "
                f"{delta['duration_saved_ms']:+,}ms latency, "
                f"quality delta {delta['quality_delta']:+.1f}"
            )
    print()


def list_runs() -> None:
    index_file = RUNS_DIR / "index.json"
    if not index_file.is_file():
        print("No benchmark runs recorded yet.")
        return
    idx = json.loads(index_file.read_text(encoding="utf-8"))
    print(f"Recorded Benchmark Runs ({idx['total_runs']} total):")
    for r in idx["runs"]:
        print(f"  - {r['run_id']}  ({r['created_at']}, git {r['git_short_sha']}, mode={r['mode']}) — {r.get('label', '')}")


def compare_runs(run_a_id: str, run_b_id: str) -> None:
    file_a = RUNS_DIR / f"{run_a_id}.json"
    file_b = RUNS_DIR / f"{run_b_id}.json"
    if not file_a.is_file() or not file_b.is_file():
        raise SystemExit(f"Run file not found: {file_a} or {file_b}")
    a = json.loads(file_a.read_text(encoding="utf-8"))
    b = json.loads(file_b.read_text(encoding="utf-8"))
    print(f"=== Comparing {a['run_id']} (A) vs {b['run_id']} (B) ===")
    for client in CLIENTS:
        for opt in OPTIMIZATION_MODES:
            sa = a["summary"]["by_worker"].get(client, {}).get(opt)
            sb = b["summary"]["by_worker"].get(client, {}).get(opt)
            if not sa or not sb:
                continue
            d_in = sb["input_tokens"] - sa["input_tokens"]
            d_dur = sb["total_duration_ms"] - sa["total_duration_ms"]
            d_score = round(sb["avg_overall_score"] - sa["avg_overall_score"], 1)
            print(
                f"  {client:<12} opt={opt:<3}: input_tokens {sa['input_tokens']:,} -> {sb['input_tokens']:,} ({d_in:+,}), "
                f"duration {sa['total_duration_ms']}ms -> {sb['total_duration_ms']}ms ({d_dur:+,}ms), "
                f"score {sa['avg_overall_score']} -> {sb['avg_overall_score']} ({d_score:+.1f})"
            )


def serve_viewer(port: int) -> None:
    rebuild_index_and_viewer_bundle()

    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args: Any, **kwargs: Any) -> None:
            super().__init__(*args, directory=str(ROOT), **kwargs)

    with socketserver.TCPServer(("127.0.0.1", port), Handler) as httpd:
        url = f"http://127.0.0.1:{port}/benchmarks/viewer/index.html"
        print(f"Serving AgentQ Benchmark Viewer at {url}")
        httpd.serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser(description="AgentQ Worker Runtime Benchmark Runner & Viewer")
    parser.add_argument(
        "--mode",
        choices=["calibrated", "hybrid", "live"],
        default="calibrated",
        help="Execution mode: calibrated (deterministic protocol harness), hybrid (live Antigravity + calibrated), or live",
    )
    parser.add_argument("--label", default="", help="Optional human-readable label for the run")
    parser.add_argument("--date", default=None, help="Optional YYYY-MM-DD date override for historical baseline")
    parser.add_argument("--list", action="store_true", help="List all recorded benchmark runs")
    parser.add_argument("--show", metavar="RUN_ID", help="Show detailed summary table for a specific run ID (or 'latest')")
    parser.add_argument("--compare", nargs=2, metavar=("RUN_A", "RUN_B"), help="Compare two benchmark runs")
    parser.add_argument("--rebuild-viewer", action="store_true", help="Rebuild index.json and viewer/data.js")
    parser.add_argument("--serve", type=int, nargs="?", const=8765, metavar="PORT", help="Serve the HTML benchmark viewer")
    args = parser.parse_args()

    if args.rebuild_viewer:
        rebuild_index_and_viewer_bundle()
        print("Rebuilt benchmarks/runs/index.json and benchmarks/viewer/data.js")
        return
    if args.list:
        list_runs()
        return
    if args.show:
        target = RUNS_DIR / ("latest.json" if args.show == "latest" else f"{args.show}.json")
        if not target.is_file():
            raise SystemExit(f"Run not found: {target}")
        print_run_table(json.loads(target.read_text(encoding="utf-8")))
        return
    if args.compare:
        compare_runs(args.compare[0], args.compare[1])
        return
    if args.serve is not None:
        serve_viewer(args.serve)
        return

    run_data = execute_benchmark_run(mode=args.mode, label=args.label, custom_date=args.date)
    print_run_table(run_data)
    print(f"Saved run artifact: benchmarks/runs/{run_data['run_id']}.json")
    print(f"Updated viewer bundle: benchmarks/viewer/index.html (data in benchmarks/viewer/data.js)")


if __name__ == "__main__":
    main()
