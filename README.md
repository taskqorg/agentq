# agentq

agentq installs native record hooks, observes agents on one host and provides a reusable worker runtime. One static binary; no optimizer, proxy or Python engine. Decisions: [agentq.md](agentq.md).

## Install through an agent

> Install agentq from https://github.com/taskqorg/agentq. Read INSTALL.md, run onboarding, and report what works and what still needs setup.

Native Windows and WSL are separate installs. Procedure: [INSTALL.md](INSTALL.md).

## Make the command available in terminals

Onboarding and `agentq service install` register `agentq` for the current OS user automatically, without administrator rights. Open a new terminal after installation, then run `agentq --version` and `agentq status` from any directory.

For an existing installation, update AgentQ and repair its command registration:

```sh
# macOS / Linux
"$HOME/.local/share/agentq/agentq" update
"$HOME/.local/share/agentq/agentq" setup-path
```

```powershell
# Native Windows PowerShell
& "$env:LOCALAPPDATA\agentq\agentq.exe" update
& "$env:LOCALAPPDATA\agentq\agentq.exe" setup-path
```

`setup-path` requires 0.8.2 or later. With a custom install root, invoke that root's launcher. It repairs registration without onboarding, configuration changes or service restarts. macOS/Linux use `~/.local/bin/agentq` and configure Bash/Zsh startup files; other shells should add `~/.local/bin` to PATH. Windows adds the install directory to the user PATH. Existing terminals and applications retain their old environment: reopen them, or on Bash/Zsh run `export PATH="$HOME/.local/bin:$PATH"` for the current shell.

A foreign Unix file or link at `~/.local/bin/agentq` is preserved and reported as a conflict. Uninstall removes this installation's command registration; the shared Unix PATH block stays for other tools. Registration applies to one user account; each account and each native Windows/WSL environment has its own installation.

## Commands

- `agentq onboard` discovers profiles without writes; `onboard --yes [agent..]` installs record hooks and the service.
- `agentq status [--json]` separates integration configuration, runtime evidence and service health. `doctor` is read-only.
- `agentq disable [agent] --dry-run`, then `disable [agent]`, detaches owned integrations. `enable [agent]` restores saved supported record fields. Only enable clears disable intent; foreign edits remain conflicts.
- `agentq direct [agent..]` repairs journal-owned legacy routing. Saved Codex chats retain the exact direct OpenAI definition under provider ID `agentq`; history and native login stay unchanged.
- `agentq archive enroll <https-origin>` separately approves session content capture; manual capture stays off without `--manual` and separate approval. Inspect `archive status --json`; stop with `agentq disconnect`. [Archive](docs/spec/ARCHIVE.md).
- `agentq langfuse` reads optional export keys from hidden stdin; `telemetry status|verify|export` inspects delivery or exports retained metadata.
- `agentq profile [--json]` ranks retained operation measurements.
- `agentq update [--check]`, `selftest`, `service status` check the installation.
- `agentq uninstall --dry-run`, then `uninstall`, restores unchanged owned edits and removes the service/install.

Cleanup runs at service start; an update alone does not clean a stopped service or disabled host. Use `agentq direct` for explicit routing cleanup. Service startup removes unchanged journal-owned optimizer registrations and updates an owned Hermes plugin to record-only. Foreign changes remain reported conflicts. A selected loopback or missing Codex provider is a problem with next step `agentq direct`; unowned dead routes and unresolved conflicts exit nonzero with manual edits. Reload clients normally; never kill active work to clear pending status.

## Run agent turns

`agentq agent turn` runs a native Codex, Claude or Antigravity turn from JSON stdin. Rust callers use `agentq-runtime`; plain turns need no installation or service. Optimization intent remains a compatible no-op for pinned consumers. [Runtime contract](docs/spec/RUNTIME.md).

Local recording needs no Langfuse or server connection. Missing evidence remains unknown. [Optimization retirement and measurements](docs/spec/OPTIMIZATION-RETIRED.md).

Releases: [taskqorg/agentq](https://github.com/taskqorg/agentq/releases), one binary per platform plus `SHA256SUMS`; provenance: `gh attestation verify <file> --repo taskqorg/agentq`. Shared rules: [specq](https://github.com/taskqorg/specq).
