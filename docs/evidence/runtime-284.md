# Launch preparation: task 284

Task [#284](https://github.com/taskqorg/agentq/issues/284) makes a launched worker start ready or refuse with a named cause, and makes AgentQ's Codex hooks run. Decisions: [agentq.md § 4](../../agentq.md#codex-hook-trust), M12.15–M12.17 and [RUNTIME.md § Launch preparation](../spec/RUNTIME.md#launch-preparation).

## Defects found

| Defect | Before | After |
|---|---|---|
| AgentQ wrote no Codex hook trust | A real Codex turn on an isolated home ran no AgentQ hook; `hooks/list` read `untrusted` | Journaled trust for the two exact entries; the same turn stamps `codex.SessionStart` and `codex.Stop` |
| On Windows, Codex starts no hook whose command holds a quoted token | `"<exe>" hook codex Stop` never ran, trusted or bypassed, so AgentQ's Windows Codex hook records never arrived | Bare path, or the `cmd.exe --%` form when the path holds a space; both observed running |
| A logged-out client exits 1 with its state as text | `ready` read `unknown`, so nothing could refuse | `missing`; unreadable text stays `unknown` |
| A parent session's key and id reached the child | Inherited `ANTHROPIC_API_KEY`, `CLAUDECODE` and others | Removed; a caller's explicit value wins |
| git could wait for a person | With an inherited askpass program git was still waiting after 8 s | Fails in about 1 s with a named git error |

Failing before, for the child environment (`clean`, `ssh_guard` and the argv refusals restored to their old behavior in place, then put back):

```text
child_environment_is_scrubbed_and_guarded: panicked: ANTHROPIC_API_KEY inherited: {"ANTHROPIC_API_KEY":"invented-parent", ...}
launches_refuse_bypass_and_argv_secrets: panicked at runtime/tests/agent.rs: launch refused
```

## Native qualification

Command: `python3 scripts/qualify-launch-preparation.py`, after `cargo build --locked --bin agentq` and `cargo build --locked -p agentq-runtime --example qualify_launch`. It uses isolated homes under the OS temp dir and a loopback model stub. It reads no login except a read-only `ready` contrast, spends nothing, installs no service and edits no working profile.

### Windows

Windows 11 10.0.26300, Codex CLI 0.162.1, Claude Code 2.1.319, git 2.46.0.windows.1. All facts pass.

| Item | Observation |
|---|---|
| A1 | `hooks: codex SessionStart trusted (native)` and `hooks: codex Stop trusted (native)`. `config.toml` holds exactly two `[hooks.state.…]` tables. A real Codex turn stamps both hook events, also from an install path with a space. |
| A1 fallback | The computed hash equals the native `hooks/list` value for both entries. |
| A2 | Before trust: `agentq doctor` exits 1 with `hooks: codex Stop untrusted (native); run agentq onboard --yes codex`, `status` exits 1 with `hook_trust.state` `failed`, and the real turn stamps nothing. After trust: both read `trusted` and `ok`. |
| A3 Codex | `config/batchWrite` with `expectedVersion`. A project-layer hook is listed only after the grant, so real Codex honors the key. Repeat reads `Unchanged`. A stale version returns `configVersionConflict`. |
| A3 Claude | Fresh home: `trust-config-missing`. Held `.claude.json.lock`: `trust-locked`, lock kept. Then `Granted`, then `Unchanged`. A real `claude -p` in the folder keeps the key and adds no other. |
| A3 refusals | The user's home and the filesystem root read `trust-path-refused` on both clients. |
| A4 | On an isolated home both clients read version `ok` and login `missing`, while the same binaries read `ok` for their own login, so the read follows the account. Codex catalogue answers with 8 models. Codex limits on a logged-out home read `native-account-read-failed`, never zero. |
| A5 | Behind `agentq agent turn` with `GIT_ASKPASS` and `GIT_TERMINAL_PROMPT=1` inherited: `git ls-remote` against a 401 exits 128 in 0.8 s, `could not read Username … terminal prompts disabled`. `ANTHROPIC_API_KEY` and `CLAUDECODE` read absent in the child. The same git without the guard was still waiting after 8 s. |

Observed on this host: the first `claude.cmd` on `PATH` (an nvm npm shim) does not start, because its `claude.exe` is missing. `ready` reports version `unknown` with the exit reason, which is correct. The script names the shim and uses the next working Claude.

### macOS

Pending: this attempt ran on the Windows host only. Windows evidence does not qualify macOS.

## Gaps

- Real Claude's read of `hasTrustDialogAccepted` is not observed: `claude -p` has no trust gate and its gated paths need a login, which an isolated home lacks. The key form and retention are observed; the runtime still maps `Workspace not trusted` to `workspace-untrusted`.
- A real client self-update is not run; `version_cache_follows_the_binary_identity` covers the identity change with a fake client. npm shims read every time by design.
- A Claude background daemon that already runs keeps its own environment. The caller adds `agent::GIT_GUARD` to `Launch.settings_env` there. Argument and settings guards from earlier tasks keep `--bg` free of an implicit `--settings`.
- `GIT_SSH_COMMAND` is checked only as a value; no real ssh prompt was driven.
- Nothing here is deployed to a working profile.
