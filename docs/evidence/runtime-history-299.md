# Live history facts (#299)

The runtime derives context, reported limit windows, per-request usage and native
cost provenance, plan/todos and per-turn file rollups through the same history
boundary. Content remains local history; telemetry receives numeric metadata only.
The existing agent.turn/agent.call path and immutable request IDs remain authoritative
for accounting. Identified Codex token_usage_record requests supersede legacy
snapshots within their native turn; older legacy-only turns remain readable.

## Native Mac proof

On macOS aarch64, Claude Code 2.1.319 ran one short interactive image-read turn with
claude-fable-5[1m]. A per-launch --settings file selected the built
`agentq agent statusline --output <temporary-file>` command. No working settings,
profiles or client configuration were edited. Native callbacks appended five rows
with the same session ID. The final callback reported:

- context input: 19,668 tokens (2 input + 19,557 cache-read + 109 cache-write);
- context window: 1,000,000 tokens;
- seven-day use: 88%, with native reset time;
- five-hour use: absent, preserved as unknown rather than zero.

The native source remains read-only. The projected fixture
`runtime/tests/fixtures/history/claude-statusline.json` scrubs session identity and
reset time; it retains native model/context/window fields. An earlier print-mode
probe did not invoke the statusline and its invalid tiny PNG was rejected; it is
not counted as statusline or successful image proof. The interactive turn read a
valid 16x16 PNG and completed; that source can also qualify media decoding in #291.

The built `agentq agent history` live read joined the native transcript and feed
with exit 0, returning used=19668, window=1000000, seven_day=0.88 and an explicit
unknown five_hour window; two native model calls remained separate.

## Portable proof and review follow-ups

`history_live` covers exact native-session joins, partial trailing feeds, unknown
fields, two successful file turns with a denied edit excluded, replay metadata,
latest native request usage, resend idempotency and metadata-only telemetry.
One turn plus one request remains two events after repeated append. Native-reported
cost uses integer nanodollars and an explicit native source; content, paths and
private caller keys never enter these events.

Steps now carry native model, effort, CLI version and per-record sidechain/meta
flags without requiring TaskQ to parse native records. Model changes clear stale
effort. Hook success/errors/context, stop-hook summaries, truncation notices,
diagnostics, queue operations and PR links are typed. Removed queued text remains a
queue operation, never a user message. Metadata replay never re-emits old tools.
Server tools are web steps; tool-search results are tool steps.

Resolved source and family associations travel in the private session checkpoint.
A regression inserts an unrelated invalid source after opening the session and
successfully resumes its cursor, proving recovery does not rediscover all homes.
The source prefix and native binding still validate. No derived-history cache,
timeout, cap, budget or retry policy was added.

The #282 source proof is clarified: one logical parent has three physical copies
and remains ambiguous by native session ID; its opaque source IDs open individually.
Verified process fencing for advisory resume descriptions still belongs to #287.

Windows native statusline execution remains unproved; portable fixtures and CI do
not substitute for a real Windows CLI callback. No deployment is qualified here.

Validation: `cargo test --locked`, focused history tests after the presentation
revision fix, offline selftest, the workflow-pinned contract lint and `git diff
--check` passed locally. Native Windows callback proof remains for the owner.

The initial Windows CI run exposed a cleanup-exit race in an existing fake-turn
test. Main PR #309 subsequently fixed production exit provenance: a kill requested
by the runtime is not reported as a native client exit. This stack now keeps main's
exit/unknown tests unchanged and removes the interim test relaxation. Native result
authority and immutable terminal observations remain independently tested.

After main #319 moved native parsing into runtime modules, the rebase keeps that
boundary: Claude and Codex revision selection, native cost/reasoning fields and
Codex rollout request extraction now live in their native modules. Live history
selects the registered runtime and call parser instead of guessing from format
prefixes. Existing alias/parent-binding regressions remain intact with the resolved
session checkpoint. Final rebase qualification uses focused history/capture tests,
offline selftest and pinned lint locally; the full suite belongs to CI.
