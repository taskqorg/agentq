# Legacy removal and namespace cutover

Binding detail of [agentq.md § 4 Namespace cutover](../../agentq.md#namespace-cutover) and [agentq.md § 10](../../agentq.md#10-legacy-node-install). The contract holds the mandatory rules; this file holds the procedures. A change here is a decision change ([spec first](../../agentq.md#principles)).

## Namespace cutover

- `agentq migrate-names [--dry-run]` updates only unchanged owned registrations and their journal/suspended receipts under the lifecycle lock; onboarding and service startup run it automatically.
- It preserves native login, model, permissions, sandbox, preferences, retained originals and queued data. It never resumes or edits a conversation. Clients reload normally; active workers are not killed.
- Only exact managed provider/instruction blocks, owned MCP keys and owned Hermes plugin files are renamed. Foreign edits or target-name collisions stop before writes.
- The bounded owner-only `data/name-migration.json` retains the original ownership/control metadata for retry and rollback, without copying native configuration files.
- Native writes precede metadata activation; retry recognizes already-renamed values and files after an interrupted write.
- `agentq migrate-names --rollback` restores the recorded owned names only while ownership/control metadata still matches the cutover; later ownership/control metadata changes block rollback. Roll back names before selecting an older binary.
- Old-name literals remain only in migration/removal logic and historical evidence, not current registrations or aliases.

## Legacy detection

- `onboard` detects `~/.agent-context` and the global npm package `@alexkirs/agent-context`.
- The install is `<legacy>/launcher.mjs`: a folder without it (a still-running legacy process recreates `incidents/` and `machine-id.json`) counts as removed, as does a folder with `<legacy>/.retired`.
- `onboard`, `doctor` and `uninstall --legacy` name the pids of processes whose command line holds `<legacy>/` (Windows: `<legacy>\`, case-insensitive, read from `Win32_Process`).

## Legacy removal procedure

- `uninstall --legacy [--dry-run]` removes the legacy install without Node. `<legacy>` is `$AGENT_CONTEXT_HOME/.agent-context`, else `~/.agent-context`, and its parent is the home of the configs it edited.
- The Node journals (`<legacy>/backups/*/journal.json`) give the file list only, limited to the known Codex, Claude Code and Hermes files: a user edit made after the journal never blocks.
- Each listed file is copied to `<legacy>/config-backup/` (a rerun keeps the first copy), then only legacy entries are stripped: hooks and MCP servers that name `launcher.mjs` or `agent-context`, env values routed to the legacy proxy ports, Codex `model_provider = "agent_context"` and `[model_providers.agent_context]`, Hermes `supercompress` and `agent-context` plugin entries, instruction marker blocks.
- The Hermes `plugins/agent-context` files are removed.
- The `contextq-monitor` launch agent, Windows task or systemd unit is unloaded, the global npm package `@alexkirs/agent-context` removed, and `<legacy>` moved to `<legacy>.retired-<YYYY-MM-DD>`, so the backup ends in its `config-backup/`. Running legacy proxy pids are listed and left for the user.
- When the move fails (Windows: running proxies hold files, access denied), the folder is copied to the retired path, each copied file and emptied folder is removed where it can be, `<legacy>/.retired` names the retired path, and the remaining files are listed as locked; the removal succeeds.
- `doctor` lists those files and the legacy pids until the user stops the processes and deletes `<legacy>`.

## Retired repository parts

- The relay worker and its GitHub App are retired: their code is gone from this repository, and the deployed Worker and App are deleted outside it. Node clients that still default to the relay keep reports in their local queue.
- `npm-final/` is the final package `@alexkirs/agent-context` 0.5.0: every invocation prints the migration instruction to stderr and exits 1, nothing else, no install scripts. It is published as a release asset without `recipe.json`, so the legacy updater never selects it and `uninstall --legacy` keeps working.
- The Node code, gateway, its tests, benchmarks, Python adapters and recipe signing are removed from this repository; only `npm-final/` stays.
- Manual removal on a host without agentq: `node ~/.agent-context/launcher.mjs uninstall --dry-run`, then `uninstall`.
- Existing relay-filed issues and Langfuse traces stay.
