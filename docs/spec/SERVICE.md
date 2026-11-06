# Service, status and host detail

Binding detail of [agentq.md § 5](../../agentq.md#5-install-layout-and-service). The contract holds the mandatory rules; this file holds layout, release artifacts, field lists, platform procedures and rationale. A change here is a decision change ([spec first](../../agentq.md#principles)).

Section references and P/M rule IDs below refer to [agentq.md](../../agentq.md).

## Installation and service layout

- Install: `~/.local/share/agentq`, Windows `%LOCALAPPDATA%\agentq`: `releases/<ver>/agentq`, `current`, `previous`, launcher `<install>/agentq`, `data/` (§ 8), `status.json`.
- Onboarding and `service install` make the stable launcher available as `agentq` for the installing OS user; `agentq setup-path` repairs it without changing integrations or restarting services. Uninstall removes only this launcher's registration ([procedure](SERVICE.md#command-registration)).
- Release CI on a `v*` tag from `main` builds `agentq-<ver>-<os>-<arch>` for macOS arm64/x64, Windows x64, Linux x64/arm64 (static musl), plus `SHA256SUMS` and `latest.json`. Each binary and `SHA256SUMS` carry a GitHub build provenance attestation (`gh attestation verify <file> --repo taskqorg/agentq`).
- The unit (coreq project `host`: `com.agentq.host`, `AgentQ-host`, `agentq-host.service`) runs `<install>/agentq service run` in `<install>`. It owns the queue sender, update checks and `status.json`.
- A `service run` process is one whose command line runs a binary under `<install>/releases/`; the Windows launcher is not counted. [Windows processes](SERVICE.md#windows-service-processes).
- Install and update use CoreQ `prepare_install` then `activate_install`; update first checks the sha256 and candidate selftest. The lifecycle lock covers checks, pruning, staging and activation. Durable originals restore a failed or interrupted activation before another install, update or service start ([detail](SERVICE.md#update-and-worker-swap)).
- Update: hourly; each update keeps only `current` and `previous`; a dashq release notice may trigger an early check. A service run restarts into a new `current` without a proxy drain ([detail](SERVICE.md#update-and-worker-swap)).
- `status.json` lives at `<install>/status.json`, written every 30 s; `service status` treats it as unknown after 90 s. It adds record hooks, queue, `model_backlog`, newest incident and per-profile `configuration`, without paths or file text ([fields](SERVICE.md#status-file-fields)).

## Command registration

- macOS/Linux use `~/.local/bin/agentq`, refuse a foreign file/link, and add an idempotent `~/.local/bin` PATH block to Bash/Zsh startup files (respecting `ZDOTDIR`).
- Windows adds the install directory to the user PATH without expanding or truncating existing entries, then broadcasts the environment change.
- New terminals load the change; existing processes keep their inherited PATH. Custom install roots still target their stable launcher.
- Uninstall removes only the Unix link whose target is this launcher's path, or this install directory's Windows user PATH entries. Shared Unix PATH blocks and other entries stay.
- Bash/Zsh are configured automatically; other Unix shells can add `~/.local/bin` themselves. Registration is per OS user, not a privileged machine-wide installation.

## Windows service processes

- On Windows `schtasks /End` ends only the launcher, so `stop`, `uninstall` and `restart` end every `service run` process tree, orphans included.
- `restart` ends the task first, starts the service only through the `AgentQ-host` task and succeeds only when exactly one `service run` remains and `status.json` names its pid within 90 s.
- `selftest` checks the count on a built-in Windows process list.
- `service install` never overwrites a running exe (Windows os error 32): identical release bytes are reused and the launcher is renamed aside, as in [update](#update-and-worker-swap). `selftest` checks this with its own running binary and the BOM strip on a built-in fixture.

## Update and worker swap

- `service install`, `onboard --yes` and update install through CoreQ `prepare_install`, then `activate_install`. The lifecycle lock covers checking (including `update --check`), pruning, download staging, preparation, activation and recovery. Preparation writes only `releases/<ver>/`; activation replaces the launchers, then durably writes `previous` and `current`.
- Update takes the version from CoreQ's check, downloads into `releases/<ver>.tmp/`, checks the sha256 and runs the candidate's `selftest` before preparation. Each failure leaves the active files unchanged and returns the error.
- Before activation, `data/install-recovery/` retains synced originals of `current`, `previous` and both platform launchers, including absence and permissions. A durable `ready` record commits that backup before any active file changes. An activation error restores and reads back those originals; an interrupted activation is restored before the next install, update or service start selects a release. Failed restoration reports uncertainty and retains the record for retry. Read-only commands, hooks and plain agent turns do not recover (M1.2, M1.3, M12.3).
- Successful activation removes the `ready` record durably before discarding the backup. A later check may prune an unactivated candidate only after recovery. A second install of the same version with different bytes, or an older binary over a newer `current`, refuses and keeps the active files.
- A service run whose `current` names another release (its own hourly check or a CLI `agentq update`) restarts into it: it exits 75 on the next service pass and the unit starts the new `current`. Startup runs the journal-owned retirement migration before senders or configuration reconciliation. Foreign edits remain conflicts; unreadable ownership means unknown.
- A service run not started by the launcher from `releases/<ver>/` never restarts itself.

## Status file fields

`<install>/status.json` holds the coreq keys (`host`, `version`, `pid`, `at`, `project`, `error`, `update`), published as [tests/fixtures/status.json](../../tests/fixtures/status.json) and as release asset `status.json` with the release version; a test keeps the fixture equal to the written keys. It adds:

- `legacy_route`: `absent`, `problem` (`legacy_agentq_route`, next `agentq direct`), or `unknown` (`unreadable_config`). `retirement` reports startup cleanup as `retired`, `conflict`, or `unknown` (`retirement_failed`); neither field proves a native turn.
- Record hooks, queue `{pending, oldest_age, last_delivery, dropped}`, `model_backlog` ([telemetry](TELEMETRY.md#claude-transcript-reading)) and per-profile configuration.
- `configuration` (CONFIGURATION A2, `agentq/profiles.rs`): per existing client root its kind, opaque `id`, `revision` (keyed hash of its files, owned journal entries and lifecycle intent), hooks, plugin, effective adapter/state, `owned`, read-only project `overrides` count (Codex `[profiles.*]`, Claude Code `.claude.json` projects with MCP servers or env; null when unknown or unsupported), `conflicts` and `incomplete` codes; no paths or file text.

## Host tools

agentq runs these external tools on a host ([specq host requirements](https://github.com/taskqorg/specq/blob/main/engineering/release-contract.md#host-requirements)). No optimization engine or Python runtime is required.

| Tool | Minimum | Needed by | Platforms |
|---|---|---|---|
| `curl` | 7.55.1 | release downloads, Langfuse export, DashQ enrollment and delivery | all |
| Windows PowerShell | 5.1 | process inventory, service registration check, user PATH | Windows |
| OS components | supplied by the OS release | service unit, process inventory, connection count, host name, hidden key input and protected native launch | macOS `launchctl ps id hostname stty sandbox-exec`; Linux `systemctl ps id hostname stty`; Windows `schtasks taskkill hostname whoami icacls` |
| `security`, `secret-tool` (optional) | none | Langfuse key storage; when absent, the `data/langfuse` file | macOS, Linux |

## Host requirement notes

- Minimums: curl 7.55.1 and bsdtar 3.3.2 are the copies Windows 10 1803 ships; every flag agentq and coreq pass is older (newest: curl `--proto`, 7.20.2). GNU tar 1.29 detects gzip on extraction.
- Native record integrations use the clients from agentq.md §4. Worker launches require the selected runtime CLI and its native login/version readiness (§12); no compression checker or command wrapper remains.
- `gh` is not called: incident delivery (agentq.md § 7) is not wired into this binary yet and adds its row when it is.
- The optional protected host uses macOS `launchctl bootstrap`, `kickstart`, `print` and `bootout`, `sandbox-exec -p`, native process/image identity, peer credentials, `KERN_BOOTTIME`, sandbox queries and resource-coalition accounting. Missing or unsupported native evidence refuses input and retains occupancy; Windows signature verification provides no containment.
- Wrapper exceptions: the run adapter keeps native stderr unchanged, so its RTK failures stay stats outcome `failed`; the Python finder probes candidates and its error names every candidate tried; a read of the optional key store falls back to the file without a log line, while a failed write is logged. coreq's own calls (update, enrollment, service unit) follow coreq.

## Release observation fields

- `selected`: CoreQ `Tool::current()` reads `<install>/current`; only a bounded release identifier crosses. Missing, unreadable or invalid selection is null with `selection_unknown`.
- `running`: the producer binary's `Tool.version`; `coreq_version`: the version in the built `Cargo.lock`, null when unreadable. No capture integration is required.
- `health`: the running release's existing `releases/<version>/healthy` regular-file marker, consumed by CoreQ launcher rollback. A readable empty marker with a nonfuture modification time yields `state=verified`, `scope=status_written`, `version` and source `at`. Missing, unreadable, nonempty or future markers remain unknown.
- `lifecycle`: `waiting_for_activation` when selected differs from running (the update drain rule still governs activation); `healthy` only for equal versions with this receipt; otherwise `starting` or `unknown`. Host disable intent yields `pending_disable`; unreadable lifecycle state yields `unknown`.
- Snapshot and heartbeat reuse the status observation, allowlist fields and codes, and set current health/lifecycle unknown when the status time is missing, future or older than 90 s. Historical versions and receipt time remain explicitly last-known. Older producers report `not_reported`.
