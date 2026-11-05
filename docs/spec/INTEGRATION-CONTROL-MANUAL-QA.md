# Integration control: remaining native acceptance

This worksheet closes the Desktop and native macOS/Windows service gates in the [execution plan](INTEGRATION-CONTROL-PLAN.md). CLI, adapter and Linux-container evidence does not replace these observations.

## Environment

Use a disposable native host or VM with its own ordinary client authentication and configuration. Do not copy credentials. This worksheet describes full qualification; the later owner-authorized rollout to existing working installations is tracked in the plan and does not make unrun steps pass. Record the candidate SHA-256 from the latest gate, native OS/client versions, and the selected absolute installation/configuration roots. A changed candidate version or hash invalidates the run; automatic update must not substitute a release during qualification.

The test operator must first establish a supported isolated Desktop session. A CLI flag or a successful MCP loader is insufficient. If the app cannot select the isolated configuration, record that prerequisite as unavailable and stop that path. The Codex computer-use tool refused `com.openai.codex`; this worksheet does not authorize another automation route around that refusal.

The current Claude Desktop Local-environment editor was inspected through the allowed UI tool. It rejects `CLAUDE_CONFIG_DIR` for one environment with: `"CLAUDE_CONFIG_DIR" can only be set for all sessions, not for one environment.` The unsaved draft was discarded, and no model turn ran. Use an independently authenticated native user/VM or another vendor-supported isolation mechanism; do not change the global setting for working sessions to force this gate.

An agent can prepare the candidate, engines, fixtures, collectors and terminal commands inside this environment. The human operator only needs to perform the native Desktop interactions and ordinary approvals. Leave model, effort, permission mode, sandbox and authentication unchanged.

## Fixture

Create a dedicated QA project with pytest available on its normal test PATH. Generate these two files in that project:

```python
from pathlib import Path
import uuid

marker = "AGENTQ_DESKTOP_" + uuid.uuid4().hex
cases = "\n".join(
    f"def test_pass_{i:03}(): "
    + (f'print("{marker}"); ' if i == 0 else "")
    + f"assert {i} == {i}"
    for i in range(180)
) + "\n"
Path("test_success.py").write_text(cases)
Path("test_failure.py").write_text(
    cases + f'def test_expected_failure(): assert False, "{marker}"\n'
)
print(marker)
```

In each requested Desktop turn, use the ordinary native shell tool exactly once. Never ask the model to invoke `agentq run`, alter a permission rule, or retry through a disguised command.

Success command:

```sh
pytest -s -v -p no:cacheprovider --color=no test_success.py
```

Failure command, used only in its stated step:

```sh
pytest -s -v -p no:cacheprovider --color=no test_failure.py
```

Ask for the passed/failed counts, native exit code and the exact recovery reference visible in that result, or null. A successful compressed result must have a new session/tool-correlated local record, fewer output bytes, a model-visible reference, and complete original recovery containing all 180 cases and the unique marker. Compare the recovered bytes with the matching native pre-transformation output when available; missing evidence remains unknown.

## Codex Desktop and Claude Desktop Code

Run this sequence for each client. CLI/Desktop aliases sharing the selected configuration are one scope; the dry run must show that relationship.

| Step | Action | Required observation |
|---|---|---|
| 1 | Start a fresh isolated Desktop session and run the success command | Native execution, smaller result, correlated record and recoverable original. Record session/tool IDs and the active provider. |
| 2 | `agentq optimization off codex` or `agentq optimization off claude`; run again in the same session | Process/conversation survives. New result stays native. Integration/configuration bytes remain unchanged. |
| 3 | Switch optimization on for that agent; run again | New successful result compresses. Previously transmitted Codex history is unchanged. |
| 4 | Run the selected `agentq disable AGENT --dry-run`, then `agentq disable AGENT` | Owned integration is detached; foreign values remain. Existing session stays usable. Pending reload/drain is shown rather than forced shutdown. |
| 5 | Open a new native session after detach and run the success command | Native direct access and no selected integration transformation. A test fixture must not inherit another installation's provider or MCP registration. |
| 6 | `agentq enable AGENT`; open a new native session and run the success command | The saved baseline is restored. For an originally routed Codex baseline, the proxy is ready before routing returns. MCP and hooks load from the candidate. |
| 7 | In Claude auto mode, run the failure command | Exit 1, 180 passed / 1 failed and full native failure output. No wrapper or permission change. This API cannot replace failed-tool output, so no compression is expected. |

For Claude auto mode, successful results use PostToolUse; older PreToolUse-only profiles need explicit hook configuration first. Their missing PostToolUse field must not silently appear merely because enable ran. Record that baseline separately.

## Claude Desktop Chat

This profile is MCP-only. Compression must remain unsupported.

1. In an isolated Chat session, index the public fixture and recover it through `agentq`; record the registration's candidate identity.
2. Disable `claude-desktop`, reload Chat normally and start a fresh conversation. Its selected MCP tools must be absent. Claude Code's independent registration remains intact.
3. Enable `claude-desktop`, reload normally and recover the retained original. The restored registration uses `agentq mcp claude-desktop`.

An old cached `mcp claude` connection cannot prove Chat's independent identity; record pending reload until a fresh session loads the new role.

## Native service lifecycle

Do this only on the disposable host, after the active-client continuity checks.

1. Save a retained original and record existing grants/queue identities without printing their values. Close QA clients normally.
2. Run host `agentq disable`. Repeated disable must finish after known dependencies drain; it must preserve the first baseline. Unknown inventory remains pending, with no forced timeout.
3. Verify the actual OS manager: launchd `gui/UID/com.agentq.host` is unloaded, or Windows task `AgentQ-host` is absent. The saved recipe, data and originals remain. Local `agentq status --json` reports intentional stop.
4. `agentq service restart` must reject the disabled scope. Log out/in and reboot the disposable host; confirm the selected integration does not reconnect. Read status and recover the original while the service is stopped.
5. Run `agentq enable`. Verify native unit/task activation and candidate hash, then the originally routed provider/MCP/hooks. A native fresh Desktop turn must work. Confirm originals, grants and queued observations survive.
6. Repeat enable and disable once to check idempotence. Preserve later foreign edits and record conflicts rather than overwriting them.

## Record

For each row, retain: candidate hash, OS/client version, absolute QA roots, phase, native session/tool IDs, unchanged native policy, command/exit, local event/ref, original-byte comparison, status JSON, OS-manager readback and verdict with a reason. Keep these private. Do not put credentials, raw native configuration or arbitrary paths into telemetry.

Mark the whole plan complete only after the remaining rows have authoritative evidence. An unavailable Desktop session, missing native output or changed candidate is unknown, not pass.
