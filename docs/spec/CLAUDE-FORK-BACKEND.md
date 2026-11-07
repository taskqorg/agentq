# Required Claude local held-fork backend contract

This is the unresolved external prerequisite for the required #300 Claude fork. It does not advertise an implemented native API, enable AgentQ's fork capability, replace #300 acceptance, or authorize a speculative control request. [SESSION-BRANCHING.md](SESSION-BRANCHING.md) keeps the binding runtime/ownership contract.

## Available source and edit boundary

The available implementation is inert shipped source embedded in official Claude Code 2.1.284, internal revision `2b8ce618c24de26410e4bdfc4e1d592accd61f61`. The provenance/index is `/tmp/agentq-300-claude-implementation-inquiry-evidence.json`, with source chunks indexed by `/tmp/agentq-300-cli-implementation-source/index.json`. These are external evidence, not an editable Claude build tree or a backend dependency AgentQ can rebuild. The public Claude Code tag is a separate documentation/plugin repository. AgentQ must not patch an installed binary, modify inert evidence, inject replacement JavaScript, execute the SDK, or introduce another transport/provider to fill this gap.

No complete held local-fork sequence is established in this source. This is version/path-specific, not proof of permanent native impossibility. Existing local resume/fork/cursor hydration is real and remains the intended native operation.

## Why existing controls do not complete the sequence

| Existing mechanism | Actual source behavior | Remaining missing cause/evidence |
|---|---|---|
| Local resume + fork-session + resume-session-at + session-id | Startup validates the requested UUID; `aC` hydrates and slices inclusively; `XAe` keeps a fresh invocation identity for a fork | Requested ID does not prove actual child identity or loaded retained prefix |
| await-initialize | Requires local stream-json; admits early initialization and plugin setup | Not an established held-fork phase surviving initialize; `ZP` can seat an initialPrompt and initialization releases/kicks the queue |
| await-claim / claim_session | Startup explicitly rejects await-claim combined with continue, resume **or** fork-session | Spare claiming cannot implement the required local native fork |
| initialize + get_settings + list_permission_rules | Actual `Zc` reports PID/current permission/state/proactivity; get_settings reports `st()` as applied.model; rules expose live policy facts | No actual session/retained-chain binding and no hold on other prompt producers |
| interrupt / cancel_async_message | Cancels current or identified queued work; optional cancel_queued operates on an existing queue snapshot | Cannot prevent model work before the request or future deferred/cron/agent/channel enqueue after it |
| end_session | Sets endSessionRequested, aborts, discards pending work and breaks the inbound loop | Does not leave a held, resumable child on the same writable transport |
| set_prompt_suggestions_paused | Pauses the suggestions subsystem | Does not hold native turn execution or all other queue sources |
| rewind_conversation | Checks current running/queue state, optionally interrupts, persists and changes a selected prefix | A mutation after startup cannot establish a causal hold before startup; no child identity/prefix admission receipt |
| poll_event / status / context / presentation export / replay | Observations, event handling, rendered sections, aggregates, presentation text or replayed input/control frames | No established authoritative retained-message chain plus causal hold/release contract |
| fork_conversation | Inspected `jc/U_/x_` success lane requires sdkUrl and first-party remote session/environment creation/upload | Does not supply the authorized local owned child transport |

Headless `QP` directly dispatches these controls. Deferred-tool resume, cron scheduler callbacks, seated agents and channels have separate enqueue paths. A queue-empty/idle observation or a delay does not stop their next enqueue. No cancellation, status observation or native filesystem appearance may be substituted for a held-start guarantee.

## Smallest external backend change

Extend the existing local print/headless fork boot and stdio protocol, with **one native startup hold**, rather than another fork implementation or transport. The backend maintainer may choose field names; the following state machine and response information are required. No proposed flag or field is sent by AgentQ until its actual implementation is available and source-proven.

1. **Held boot.** An explicit local fork startup option establishes a non-bypassable execution hold before any native startup/resume work can dispatch a prompt/model turn. Existing native loader performs resume/fork/cursor selection. Every execution entry, including deferred tools, cron, agents and channels, respects that hold; initialize cannot release it. A held process may service only its existing control protocol and observations. Startup work outside the ordinary queue must not bypass the same hold.
2. **Authoritative held receipt.** The existing correlated initialize response identifies the actual child session ID and process, parent session, exact source incarnation/generation and selected cursor. It supplies authoritative loaded retained messages or a complete equivalent witness: ordered actual UUID/parent chain, payload identity and exact mapping to the original source prefix. A digest of the requested source or a list copied from AgentQ is insufficient. Unknown/remapped/unverifiable entries fail the hold admission. The hold remains active during get_settings and list_permission_rules, which provide existing effective model/permission/rule readback. Requested and applied facts stay separate.
3. **Atomic first explicit input.** The existing typed user input, correlated to the same held incarnation, consumes the hold exactly once. Under that same synchronization, it proves no other runnable/pending/uncertain input precedes it and admits this exact caller input as the first turn. If an autonomous source queued work while held, the backend refuses release before executing any turn; it does not silently discard retained caller/native work. There is no release-then-send window and no autonomous first turn. Duplicate/foreign/stale release input never replays.
4. **Unknown outcomes.** Lost/partial/malformed replies keep the native hold or an explicit unknown outcome; they do not auto-release, resume, delete the child or retry the original action. The actual incarnation and operation ID must remain distinguishable for existing owner reconciliation.

A startup hold plus an authoritative initialize extension and atomic consumption on the existing input lane is the minimum semantic change. Merely adding a host-side field parser cannot create any of these native guarantees. Backend implementation must be supplied through legitimate editable upstream source/build ownership or a supported vendor release; that source/build ownership is currently unavailable here.

## AgentQ source already prepared

The current private `PreparedFork` constructs the existing producer-shaped durable intent from the exact selected Step and measured source generation: operation ID, parent session/owner, selected original Step, physical path/incarnation, requested_conditions and the Claude boundary descriptor. The descriptor itself has no raw message bodies. This is original-source evidence, never child-loaded-prefix evidence or a portable anchor.

The existing one owned `Session::boot`/`exchange`/`input` transport is retained. Parent authority, idle/native-process identity, scope, C285 request fences and source revalidation are checked before any future child stage. Recovered transports remain observation-only. Unsupported or absent Claude capability still returns fork-unsupported. No pending plan persists applied lineage or calls archive publication.

After the external contract exists, remaining AgentQ implementation is explicit:

- Add its source-proven startup option and validate the real held receipt through this same owned transport; perform ordinary child validation/preflight and process-hook/lifetime setup before native child creation.
- Persist parent branch_begin and distinct child branch_lineage admission before creation; bind measured child identity with existing ownership and refuse source-equal/foreign/malformed IDs before input. Revalidate the original native generation immediately before the native write and after readback.
- Compare the actual held child's loaded witness with the selected original prefix, then validate effective get_settings/permission/rule conditions. Expand tool/carrier/attachment/compaction and platform mappings only from exact source evidence; unsupported forms remain explicit gaps.
- Atomically submit exactly the explicit Launch input using the real backend hold-consumption mechanism. Preserve unknown fences and never retry on uncertain delivery.
- Only after verified application/readback, finish durable parent/child lineage and call the existing archive handle's lineage hook. Current archive publisher validates Codex thread acknowledgements; add a Claude-specific validated native receipt path when its actual schema exists. Never fabricate a thread.id shim. The native JSONL selection and stream archive remain separate physical domains; without an actual accepted capture bridge, parent anchor is unmapped and activation pending.

No build/test/check/lint/probe/hash verifier/static-review/native qualification is authorized by this source document. #300 remains incomplete until this native prerequisite and the listed AgentQ code are implemented, followed by the separately authorized coherent verification/native stages.
