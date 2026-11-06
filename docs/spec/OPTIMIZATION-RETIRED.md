# Retired context optimization

AgentQ removes context optimization, Headroom, RTK, provider routing, compression and document MCP tools. Native Stop and SessionStart record hooks, service, telemetry, enrollment and runtime remain. Existing retained data stays untouched; no fleet deployment is authorized by qualification.

The 14-day ceiling of qualified runner output, as share of input plus cache tokens:

| Host | Codex | Claude |
|---|---|---|
| Mac | 0.30–0.77% | 0.14–0.30% |
| Windows | 0.38–0.69% | 0.69–0.71% |
| WSL | 0.02–0.03% | unknown |

Compressed calls in 7 days: Mac 1 of 5; pds18 0 of 79; Windows unknown. Headroom costs a 724 MB venv and about 658 MB memory on Mac. Return condition: a paired test shows real token savings with no quality loss.

## Compatibility

The exact direct Codex provider definition in `onboard::CODEX_DIRECT_PROVIDER` stays unchanged, so saved conversations keep their provider ID and native login. No conversation history is rewritten. Native resume qualification needs an isolated profile and a real saved-provider session; local code checks alone do not prove it.

`agentq_runtime::optimization::Intent`, its parsing and report shape remain compatible no-ops. TaskQ, PilotQ and Pipe pin earlier runtime revisions and must adopt the replacement before API removal. A request reports `effective=off`, `reason=optimization_retired`; it adds no provider headers or environment overrides.

DashQ is the enrolled snapshot consumer (`snapshot.rs`, ENROLLMENT and CONFIGURATION). Schema 1 retains unrelated identity, release, resource, queue and configuration fields; retired fields are omitted rather than reporting healthy optimizer state. Old snapshots remain readable as historical evidence; consumers must treat absent retired fields as unavailable. The inspected DashQ consumer accepts every `agentq-snapshot/1` in `src/enroll.rs::renders`; `src/version.rs` reads engine data as optional and `src/page.rs` renders absent library versions as `Not reported`. No consumer deployment is implied.

## Local document-tool usage

Measurement reads only this Mac's retained telemetry and actual native transcript tool-call records; prose mentions, tool definitions and test fixtures do not count. The retained telemetry has zero `index_document` and zero `search_document` calls. The scan covered 511 Codex session files, 11 archived Codex files and 5496 Claude transcript files, with no unreadable files. Codex had zero calls; Claude had 5 index and 5 search calls, all on October 12–13 through the retired `agent_context` namespace. Telemetry covered November 1–6 and retained one `get_original` call. No current `agentq` index/search calls were found. Near-zero measured use retires both tools; `get_original` retires with compression. This is a local retained-data observation, not a fleet-wide lifetime claim.

## Upgrade migration and diagnostics

Before the service starts export or configuration reconciliation, its existing lifecycle lock guards retirement of journal-owned routing, MCP registrations, PreToolUse/PostToolUse and instruction blocks, including receipts without an agent field. Only values still equal to their recorded after-value change; conflicts remain visible. Codex routing becomes the exact direct definition above. Already-owned Hermes plugin source follows the record-only release without installing new activation or changing disabled intent. Splitting old parent receipts preserves their saved recovery-source links. Successful retirement removes its receipts, so repeated starts are safe. Corrupt ownership state fails visibly and never resets ownership.

Saved disable receipts ignore retired routing, MCP, wrapping hooks and instructions when validating or enabling current fields. Enable restores only record hooks or the record plugin. Doctor, fresh status, profile conflicts and heartbeat identify a selected `agentq` loopback endpoint as `legacy_agentq_route`, next step `agentq direct`; unreadable configuration remains unknown. Migration errors are reported separately from release health.

Plans block retired routing, optimization and MCP operations as unsupported; the executor defensively refuses retired writes with `feature_retired`. DashQ still offers these fields and must remove the controls and render that refusal in its own stream; this PR changes no DashQ code.
