# Install and remove through an agent

This file is the procedure for Codex, Claude Code and Hermes when a user asks to install or remove agentq. Decisions: [agentq.md](agentq.md). Keep the user's login, model, approvals and sandbox. Never print keys, login files or whole configs.

## Install

1. Find the OS and runtime of the agent's user. Native Windows and WSL are separate installs; a Windows project opened in WSL does not make the Windows agent a Linux one.
2. Read `https://github.com/taskqorg/agentq/releases/latest/download/latest.json`. Download the file for the platform (`macos-aarch64`, `macos-x86_64`, `linux-x86_64`, `linux-aarch64`, `windows-x86_64`) and `SHA256SUMS` from `https://github.com/taskqorg/agentq/releases/latest/download/<name>` into a new temporary folder. Check the sha256; a mismatch stops the install. On macOS and Linux run `chmod +x`.
3. Run `<binary> onboard`. It lists ready agents and writes nothing. Detection is not installation.
4. Tell the user which agents were found and what changes, then ask once: "Found Codex and Claude Code. Connect both?" Skip the question if the user already named the agents.
5. On consent run `<binary> onboard --yes [codex|claude|hermes..]`. The command installs agentq into `~/.local/share/agentq` (Windows `%LOCALAPPDATA%\agentq`), applies agentq.md § 4 and installs the service. A named conflict stops that file; never replace a foreign optimizer or custom provider by guess.
6. Onboarding registers `agentq` on the current user's PATH. Open a new terminal and verify `agentq --version` and `agentq status` from outside the install directory. In the existing shell, use the installed launcher's absolute path until its environment is refreshed. To repair an existing install, update it and run its launcher with `setup-path` (0.8.2+); see [terminal setup](README.md#make-the-command-available-in-terminals). Optional Langfuse: the user runs `agentq langfuse` in a local terminal and types the host and keys there; never ask for keys in chat. Then `agentq telemetry verify`.
7. Check `agentq service status` and `agentq telemetry status`. Start a new session of each selected agent and call `agentq` `search_document`; reload clients normally, never kill other sessions.
8. Report: connected agents and version, service state, conflicts, telemetry, and what was verified by a real turn. Configured is not verified; unknown stays unknown.

## Remove

A user request to remove agentq allows this step.

1. Ask the user to close other sessions that use agentq.
2. Run `agentq uninstall --dry-run` and show what it restores and removes.
3. Run `agentq uninstall`. It restores owned edits whose value is unchanged, keeps later foreign edits and reports conflicts, removes the service, engines, data and install folder. Logins, shared keys and exported history stay. A second run prints `already removed`.

## Temporarily disable and restore

Use the commands from [README](README.md#temporarily-disable-optimization-or-integrations) with 0.9.0 or later for the current namespace. The current direct OpenAI provider definition supports resume after detach without an active AgentQ integration. Existing chats bound to the retired provider are intentionally not migrated: start new Codex chats after the owner-approved name cutover. Onboarding/service startup migrates owned registrations; `agentq migrate-names [--dry-run]` applies it after an explicit update. See [upgrade instructions](README.md#upgrade-to-the-current-names). Keep AgentQ enabled when the user requested that state. Claude auto mode uses successful-result PostToolUse replacement; older PreToolUse-only profiles need explicit hook configuration. Enable preserves the saved baseline.

1. Read `agentq status --json`. Distinguish a compression preference, configured integration, runtime evidence and service health; none implies the others.
2. For compression only, run `agentq optimization off [codex|claude|hermes]`; omit the agent for the host. Resume with `on`. The host switch preserves individually disabled preferences. No native config edits or client restart.
3. For complete integration detach, run `agentq disable [codex|claude|hermes|claude-desktop] --dry-run`, then the same command without `--dry-run`. Omit the agent for the installation. Explicit user intent is consent; do not add another approval round. Preview aliases because CLI and Desktop can share a profile.
4. Keep active workers and conversations. If status is pending, close/reload affected clients normally and repeat disable; never kill them or force a timeout. Host disable suspends export, automatic activation and remote configuration while needed transport drains. Foreign registrations or unknown scope remain conflicts.
5. Restore with `agentq enable [agent]`. It uses the saved supported fields and current canonical writers, preserves Direct/compression preferences and later foreign edits, and prepares the existing proxy before routing. It never runs uninstall or onboarding. A missing saved integration is unchanged.
6. Read status again. Matching configuration is configured; a required native client check remains unverified until it actually runs. Preserve data and recovery records on a partial/unknown result and report the conflict. Remote management gains no disable/enable permission from these local commands.

## DashQ connection target

Accepted, pending implementation: [ENROLLMENT](docs/spec/ENROLLMENT.md). Once qualified, use `agentq enroll <https-url>` and approve once in the browser (headless: link/code elsewhere). The command verifies a real dashboard snapshot. No file/key copying or repeated onboarding; use connection status and disconnect for recovery. Do not advertise this path as available before its release gate passes.
