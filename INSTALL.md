# Install the AgentQ worker layer

AgentQ is the shared worker layer for preparation, launch, observation, control, history and archive. This procedure installs or removes its optional host recording and service for Codex, Claude Code, Hermes and Antigravity. A plain runtime-library or JSON turn needs no onboarding or service. Decisions: [agentq.md](agentq.md). Keep the user's login, model, approvals and sandbox. Never print keys, login files or whole configs.

## Install

1. Find the OS and runtime of the agent's user. Native Windows and WSL are separate installs; a Windows project opened in WSL does not make the Windows agent a Linux one.
2. Read `https://github.com/taskqorg/agentq/releases/latest/download/latest.json`. Download the file for the platform (`macos-aarch64`, `linux-x86_64`, `windows-x86_64`) and `SHA256SUMS` from `https://github.com/taskqorg/agentq/releases/latest/download/<name>` into a new temporary folder. Check the sha256; a mismatch stops the install. On macOS and Linux run `chmod +x`.
3. Run `<binary> onboard`. It lists ready agents and writes nothing. Detection is not installation.
4. Tell the user which agents were found and what changes, then ask once: "Found Codex and Claude Code. Connect both?" Skip the question if the user already named the agents.
5. On consent run `<binary> onboard --yes [codex|claude|hermes..]`. The command installs agentq into `~/.local/share/agentq` (Windows `%LOCALAPPDATA%\agentq`), applies agentq.md § 4 and installs the service. A named conflict stops that file; never replace a foreign optimizer or custom provider by guess.
6. Onboarding registers `agentq` on the current user's PATH. Open a new terminal and verify `agentq --version` and `agentq status` from outside the install directory. In the existing shell, use the installed launcher's absolute path until its environment is refreshed. To repair an existing install, update it and run its launcher with `setup-path` (0.8.2+); see [terminal setup](README.md#make-the-command-available-in-terminals). Optional Langfuse: the user runs `agentq langfuse` in a local terminal and types the host and keys there; never ask for keys in chat. Then `agentq telemetry verify`.
7. Check `agentq service status` and `agentq telemetry status`. Start a new session of each selected agent and finish one native turn and inspect its local record; reload clients normally, never kill other sessions.
8. Report: connected agents and version, service state, conflicts, telemetry, and what was verified by a real turn. Configured is not verified; unknown stays unknown.

## Remove

A user request to remove agentq allows this step.

1. Ask the user to close other sessions that use agentq.
2. Run `agentq uninstall --dry-run` and show what it restores and removes.
3. Run `agentq uninstall`. It restores owned edits whose value is unchanged, keeps later foreign edits and reports conflicts, removes the service, data and install folder. Logins, shared keys and exported history stay. A second run prints `already removed`.

## Temporarily disable and restore

Use [the controls](README.md#commands). Saved Codex chats retain their provider ID and the exact direct definition; never rewrite their history.

1. Read `agentq status --json`. Distinguish configured integration, runtime evidence and service health.
2. Run `agentq disable [codex|claude|hermes|claude-desktop|antigravity] --dry-run`, then the same command without `--dry-run`. Omit the agent for the installation. Explicit user intent is consent; do not add another approval round. Preview shared aliases.
3. Preserve active workers and conversations. Reload affected clients normally and repeat disable when pending; never kill them or force a timeout. Host disable suspends export, automatic activation and remote configuration. Foreign registrations or unknown scope remain conflicts.
4. Restore with `agentq enable [agent]`. Only saved supported record fields return, using current writers; retired routing, MCP and wrapping hooks never return. No onboarding or uninstall is invoked. Missing saved integration is unchanged.
5. Read status again. Configuration alone is not native verification. Preserve recovery state on partial/unknown results and report conflicts.

## DashQ connection target

The connection contract and qualification state are in [ENROLLMENT](docs/spec/ENROLLMENT.md). For a qualified release, use `agentq enroll <https-url>` and approve once in the browser (headless: link/code elsewhere). The command verifies a real dashboard snapshot. No file/key copying or repeated onboarding; use connection status and disconnect for recovery. Do not advertise this path as available before its release gate passes.
