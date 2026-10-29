# agentq

agentq installs, routes and observes context optimization for Codex, Claude Code and Hermes on one host. One static binary per OS; no Node, npm or toolchain on the host. Decisions: [agentq.md](agentq.md). Product outcomes: [docs/spec/PRODUCT.md](docs/spec/PRODUCT.md).

## Install through an agent

Paste into a supported agent:

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

## Upgrade to the current names

Version 0.9.0 uses `agentq` for the MCP server, Codex provider, instruction markers and Hermes plugin. There is one current namespace; no retired-name aliases are registered.

Existing Codex conversations bound to the retired provider are intentionally not migrated. Their history remains on disk, but start new Codex chats after the cutover. Reload the other clients normally so they load the new MCP registration; active workers are not killed. New conversations use the current namespace and continue to work after temporary disable/enable.

Onboarding and service startup apply the owned registration change automatically. To apply or preview it after a binary update:

```sh
agentq migrate-names --dry-run
agentq migrate-names
agentq status
```

The transition preserves native login, model, permissions, sandbox, compression preferences, local originals and queued observations. A target-name collision or changed owned value is reported and kept. Repeating the command finishes an interrupted transition. `agentq migrate-names --rollback` restores the recorded owned names only while the ownership/control metadata still matches the cutover; select an older binary only after that rollback. Private rollback metadata stays in the installation data folder. Historical transcripts, evidence and retired-package removal rules keep their original names.

## Commands

- `agentq compress <agent>` reads one tool-result JSON object (`adapter`, `tool`, `text`) and returns `text` and a recovery `ref`. Unqualified hook/provider recipes pass unchanged. `agentq stats [--json]` shows local call outcomes and bytes; missing or corrupt records are unknown.
- `agentq profile [--json]` ranks compression and export operations by time, failure rate and variance against their budgets over the last 7 days; operations without readable samples, and proxy requests, read unknown. `agentq profile trace on [<minutes>]|off` records per-step traces for at most 60 minutes.
- `agentq onboard` finds ready agents and a legacy install; writes nothing. `agentq onboard --yes [codex|claude|hermes..]` applies config edits, installs the service and prints what to verify by a real turn.
- `agentq langfuse` reads the Langfuse host and keys from hidden stdin into protected storage. `agentq telemetry status|verify` shows the queue and checks delivery.
- `agentq service status`, `agentq update [--check]`, `agentq selftest` check the install.
- `agentq setup-path` repairs the current user's command registration.
- `agentq doctor` reads installation, host tools with their minimum versions, service, routing intent, engines and local telemetry health. A missing or old host tool fails with its found version, minimum and fix. Intentional detach is distinct from drift; runtime verification remains separate.
- `agentq uninstall --dry-run`, then `agentq uninstall`, restores owned edits and removes the service and install folder.

Releases: [taskqorg/agentq releases](https://github.com/taskqorg/agentq/releases), one `agentq-<ver>-<os>-<arch>` binary per platform plus `SHA256SUMS`. Check a download with `sha256sum -c SHA256SUMS --ignore-missing`; trace it to the workflow run that built it with `gh attestation verify <file> --repo taskqorg/agentq`.

## Temporarily disable optimization or integrations

Use AgentQ 0.9.0 or later for the current namespace. The supported commands preserve resume of Codex conversations created through the current AgentQ provider. Run commands on the host whose installation you want to control; Mac, Windows, FRA and WSL have separate state.

Start with status:

```sh
agentq --version
agentq status
agentq status --json                 # machine-readable details
```

Choose the mode:

| Goal | Turn off | Restore |
|---|---|---|
| Pause compression for the host; keep integrations connected | `agentq optimization off` | `agentq optimization on` |
| Pause compression for one agent | `agentq optimization off claude` | `agentq optimization on claude` |
| Detach every owned integration on this host | `agentq disable` | `agentq enable` |
| Detach one agent's integration | `agentq disable codex` | `agentq enable codex` |

Compression switches apply to new results immediately, without a client restart. The host pause takes precedence over individual preferences: `optimization on` preserves individually paused agents. To resume Claude after both pauses, run both `agentq optimization on` and `agentq optimization on claude`.

For a full detach, preview the affected registrations and shared profiles first:

```sh
agentq disable --dry-run             # preview the whole installation
agentq disable                      # detach owned integrations
agentq status                       # inspect configuration and pending work
agentq enable                       # restore the saved setup
agentq status

agentq disable codex --dry-run       # preview one agent instead
agentq disable codex
agentq enable codex
```

Targets are `codex`, `claude`, `hermes` and `claude-desktop`. `claude-desktop` means Desktop Chat and supports integration detach only. Codex CLI/Desktop share a profile; Claude Code CLI/Desktop Code share another. Workers using the same installation and profile switch together. There is no per-worker switch.

Read status as separate observations:

- `integration: on|off|mixed|unknown` describes owned configuration. `on` alone does not prove that a running client loaded it.
- `optimization: on|off|mixed|unknown` summarizes effective compression; per-profile preferences and reasons explain a host pause or disabled integration.
- Service state describes the background process separately. A service can remain running during detach to preserve active work and original recovery.
- `pending_reload` means a client may still have cached settings. Reload or close that client normally. `pending_drain`, or unknown dependencies, means safe transport shutdown is not yet proved. Repeat `disable` after clients finish; never kill workers to clear the status. After `enable`, open or reload affected clients normally and check status again.

Full detach keeps the installation, engines, local data, originals, credentials, grants and queued observations under their existing retention limits. Enable restores the saved supported registrations, prior Direct state and independent compression preferences. Later foreign edits remain conflicts. Repeating a command preserves the first recovery baseline. Only `enable` clears disable intent; onboarding, configure and service activation cannot reconnect a disabled integration. Local status and enable work with the service stopped.

Codex stores its provider ID in each conversation. The current provider ID is `agentq`. Disable, Direct and uninstall retain a marked direct OpenAI definition under that same ID, without an AgentQ URL, hook, MCP or service dependency. Existing conversations created with this namespace can therefore resume; enable restores the owned loopback definition only when its direct definition is unchanged.

If `agentq` is not on PATH, use the installed launcher. On macOS/Linux: `"$HOME/.local/share/agentq/agentq" status`. In Windows PowerShell: `& "$env:LOCALAPPDATA\agentq\agentq.exe" status`. Replace `status` with the desired command and arguments.

Claude auto mode compresses successful Bash output through PostToolUse without changing native commands or permissions; failures remain native. Older PreToolUse-only profiles need explicit hook configuration; enable preserves their saved registrations. `direct` is a routing-only emergency bypass and does not pause Claude's hook.

Qualification: Mac Codex/Claude CLI controls and isolated Linux systemd lifecycle/retention have native evidence. Desktop, native Mac/Windows reboot/logon and Codex originally routed service-baseline restoration are tracked separately; an unrun check remains unknown. See the [qualification record](docs/spec/INTEGRATION-CONTROL-PLAN.md#implementation-candidate-and-remaining-qualification) and [control contract](agentq.md#temporary-controls).

## Check installed client compression

With Python 3.9+, cargo, the installed client and its usual login:

```sh
agentq check compression both
agentq check compression codex --fixture pytest
# Or: codex / claude. --codex PATH / --claude PATH select a specific version.
# The source checker remains scripts/check-client-compression.py.
```

This opt-in live check uses the current client settings and paid model access. It creates a temporary dependency-free crate (180 passing tests and one intentional failure), asks each real CLI to run ordinary `cargo test` once, and checks session-correlated compression stats, a model-visible recovery reference and complete original recovery. It never asks the client to invoke `agentq run` manually or changes routing, hooks, credentials or services.

For a read-only Codex CLI, use `--fixture pytest` with real pytest on PATH. The same 180 passing tests and one failure run without pytest cache writes. The checker never installs dependencies or changes sandbox policy. When code-mode tool calls are absent from CLI stdout, it reads only that session's native local transcript; missing evidence stays unknown.

Exit 0: both integrations proved compression. Exit 1: failed or unproved; `report.json` distinguishes `fail` (native tool ran without compression proof) from `unknown` (missing prerequisite, client did not run, timeout or unreadable stats). The managed Headroom Python is reused when present. Windows can locate Codex's embedded CLI; this does not qualify the Desktop UI. Private logs and fixture stay in the printed directory. `--output DIR` selects it; each client has a 120 s deadline (`--timeout 30..300`). A green `selftest`, healthy service or proxy traffic alone cannot pass this check. Ordinary CI runs only the verdict checks, never paid clients.

## Local telemetry

Collection needs no Langfuse/server connection. Compression events, bounded profiling and existing producer events live in installation-local SQLite/WAL. `agentq stats --records` exports compression metadata for qualification; `agentq telemetry export` emits versioned JSONL for analysis. `agentq profile --json` ranks retained durations and failures. Local completeness and missing native usage remain unknown; byte reduction is not billed-token savings. Legacy files are preserved for migration/rollback. See [transition](docs/spec/TELEMETRY-QUALIFICATION.md#agentq-sqlite-transition).

## Legacy Node package

The Node package `@alexkirs/agent-context` (`agent-context` CLI, gateway) is retired under [agentq.md § 10](agentq.md#10-legacy-node-install); its code is gone from this repository. `agentq onboard --yes` removes an installed copy through its own uninstall; `agentq uninstall --legacy` does the same alone.

Shared rules: [specq](https://github.com/taskqorg/specq).

Compression qualification: [benchmark and native-client checks](docs/spec/COMPRESSION-BENCHMARK.md).
