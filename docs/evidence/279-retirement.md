# Optimization retirement qualification

Scope: issue [279](https://github.com/taskqorg/agentq/issues/279). No working profile, installed AgentQ, host service or TaskQ task was changed. No deployment is included.

## Document-tool measurement

Read-only scan of this Mac: 511 Codex session JSONL files, 11 archived Codex JSONL files, 5496 Claude transcript JSONL files, zero unreadable files. Counted native `function_call` and `tool_use` records only, never prose mentions, schemas or tool results.

Codex: zero index/search calls. Claude: five `index_document` and five `search_document` calls, all October 12–13 through the old `mcp__agent_context__` namespace. Current `agentq` namespace: zero. Retained local SQLite telemetry, November 1–6: zero index/search calls and one `get_original` call. Retention limits this observation; it is not a fleet lifetime count.

## Native saved-provider resume

Copied the existing Codex conversation `01a1ad84-a145-7f31-8681-081d23e66437` into a private temporary CODEX_HOME. The conversation's native session metadata identifies provider `agentq`. Used the exact `onboard::CODEX_DIRECT_PROVIDER` bytes, with no base URL or AgentQ dependency, and the native read-only sandbox. Asked only for a literal answer with no tools or work continuation.

Native `codex exec resume` exited 0, emitted `thread.started` with the same conversation ID, answered `AGENTQ_RESUME_OK`, then emitted `turn.completed`. Reported usage: 61282 input, 37120 cached input, 133 output tokens. Temporary authentication was removed after completion. No `agentq direct` command ran.

## Automated checks

The PR records the candidate commit and results for `cargo test --locked`, `cargo run --locked -- selftest`, the contract lint from specq `7e9adf57287553176d8b3be61c2b22565543dc51`, Hermes record-hook checks. Native fleet migration, Windows resume and long-running service qualification remain outside this change.

## Upgrade regression coverage

Service startup now calls the same locked migration exercised by `retirement_preserves_saved_provider_and_record_hooks`: an old routed config and agent-less ownership journal become the exact direct definition while SessionStart/Stop remain. `startup_retires_agentless_parents_and_refreshes_only_owned_plugin` covers pre-v0.8 parent MCP/hook receipts, a foreign MCP sibling, later foreign edits, idempotence, record-only Hermes source without new activation, and corrupt ownership refusal. `old_disabled_profile_ignores_retired_scope_and_never_restores_it` exercises repeated disable and enable of a saved old profile without returning retired fields. `legacy_route_problem_is_readonly_and_not_release_health` covers IPv4, localhost, IPv6, the direct definition and unreadable config. These isolated checks exercise the startup migration implementation; no installed host service or working profile was migrated.

DashQ handoff: its enrollment configuration UI still offers retired fields and its result renderer must handle the executor's `feature_retired` refusal. This PR changes no DashQ code.
