# Service, status and host detail

Binding detail of [agentq.md § 5](../../agentq.md#5-install-layout-and-service). The contract holds the mandatory rules; this file holds field lists, platform procedures and rationale. A change here is a decision change ([spec first](../../agentq.md#principles)).

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
- `service install` copies the running binary to `releases/<ver>/` and to the launcher only where the bytes differ: Windows refuses to overwrite a running exe (os error 32). A running launcher of an older version that cannot be replaced stays and says so; it only reads `current`. `selftest` checks the skip against its own running binary and the BOM strip on a built-in fixture.

## Update and worker swap

- An engine pin change starts a new headroom worker on a new port, routes new connections to it, keeps open ones on the old worker, retires the old one after 15 min plus 30 s grace, then kills its process tree with an incident; at most two workers.
- A service run whose `current` names another release (its own hourly check or a CLI `agentq update`) restarts into it: it exits 75 on the first pass with no open proxied connection, at most 15 min after it saw the change, and the unit starts the new `current`.
- While connections stay open it logs the deferral and its count to the service log, once per change of reason; an unknown count defers like an open one.
- A service run not started by the launcher from `releases/<ver>/` never restarts itself.

## Status file fields

`<install>/status.json` adds to the coreq keys:

- `agents`: per agent with a config root its `routing` (routed, direct, bypassed, unknown) and `reason`, `intended` and `intended_reason`.
- `intended` is `routed` for an agent with shipped routing, onboarded (an owned edit under its root) and not `agentq direct`; else `direct`. `intended_reason` is `onboarded`, `routing not shipped`, `` `agentq direct` `` or `not onboarded`.
- Intent is `unknown`, never direct, when `owned.json` (not a list of entries with a `file`) or `config.json` (not an object whose `direct` lists names) is unreadable or of another shape. Claude Code and Hermes are intended direct until their routing ships.
- `mcp`, `hooks`, `engines` (per engine its pin and `ready` or `unavailable`), queue `{pending, oldest_age, last_delivery, dropped}`, `model_backlog` ([telemetry](TELEMETRY.md#claude-transcript-reading)), newest incident.
- `configuration` (CONFIGURATION A2, `agentq/profiles.rs`): per existing client root its kind, opaque `id`, `revision` (keyed hash of its files, owned journal entries, port and preference), routing, MCP, hooks, plugin, optimization, effective adapter/state, `owned`, read-only project `overrides` count (Codex `[profiles.*]`, Claude Code `.claude.json` projects with MCP servers or env; null when unknown or unsupported), `conflicts` and `incomplete` codes; no paths or file text.

## Host tools

agentq runs these external tools on a host ([specq host requirements](https://github.com/taskqorg/specq/blob/main/engineering/release-contract.md#host-requirements)). The managed engines RTK and headroom are pinned, installed and checked under agentq.md § 3, not here.

| Tool | Minimum | Needed by | Platforms |
|---|---|---|---|
| `curl` | 7.55.1 | update and engine downloads, Langfuse export, DashQ enrollment and delivery | all |
| `tar` | bsdtar 3.3.2 or GNU tar 1.29 | RTK unpack; Windows bsdtar also reads the RTK zip | all |
| Python | 3.11, below 3.14, with `venv` and `ensurepip` | headroom venv, `check compression` | all except macOS x64 (no headroom wheels) |
| Windows PowerShell | 5.1 | process inventory, service registration check, user PATH | Windows |
| OS components | supplied by the OS release | service unit, process inventory, connection count, host name, hidden key input | macOS `launchctl ps pgrep lsof id hostname stty`; Linux `systemctl ps pgrep id hostname stty`; Windows `schtasks taskkill netstat hostname whoami icacls` |
| `security`, `secret-tool` (optional) | none | Langfuse key storage; when absent, the `data/langfuse` file | macOS, Linux |

## Host requirement notes

- Minimums: curl 7.55.1 and bsdtar 3.3.2 are the copies Windows 10 1803 ships; every flag agentq and coreq pass is older (newest: curl `--proto`, 7.20.2). GNU tar 1.29 detects gzip on extraction. The Python range is the headroom pin's; `check compression` alone accepts 3.9+.
- Not host requirements: Codex, Claude Code and Hermes run agentq, not the reverse; their configuration compatibility is agentq.md § 4, and `check compression` records their exact versions. Commands passed to `agentq run` belong to the user.
- `npm` runs only to remove the legacy package, which brought its own npm. `gh` is not called: incident delivery (agentq.md § 7) is not wired into this binary yet and adds its row when it is.
- Wrapper exceptions: the run adapter keeps native stderr unchanged, so its RTK failures stay stats outcome `failed`; the Python finder probes candidates and its error names every candidate tried; a read of the optional key store falls back to the file without a log line, while a failed write is logged. coreq's own calls (update, enrollment, service unit) follow coreq.

## Release observation fields

- `selected`: CoreQ 0.11.0 `Tool::current()` reads `<install>/current`; only a bounded release identifier crosses. Missing, unreadable or invalid selection is null with `selection_unknown`.
- `running`: the producer binary's `Tool.version`; `coreq_version`: the Cargo.lock pin, 0.11.0. No dependency upgrade or capture integration is required.
- `health`: the running release's existing `releases/<version>/healthy` regular-file marker, consumed by CoreQ launcher rollback. A readable empty marker with a nonfuture modification time yields `state=verified`, `scope=status_written`, `version` and source `at`. Missing, unreadable, nonempty or future markers remain unknown.
- `lifecycle`: `waiting_for_activation` when selected differs from running (the update drain rule still governs activation); `healthy` only for equal versions with this receipt; otherwise `starting` or `unknown`. Host disable intent yields `pending_disable`; unreadable lifecycle state yields `unknown`.
- Snapshot and heartbeat reuse the status observation, allowlist fields and codes, and set current health/lifecycle unknown when the status time is missing, future or older than 90 s. Historical versions and receipt time remain explicitly last-known. Older producers report `not_reported`.
