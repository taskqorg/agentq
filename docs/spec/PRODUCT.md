# AgentQ — internal organization package

Revision: 5. Product outcomes; decisions and the requirement map live in [agentq.md](../../agentq.md) § 11. [Acceptance](VERIFICATION.md).

## Purpose

Install once per machine/runtime. Manage our agents' optimization, updates and telemetry centrally. Replace optimization tools later without rebuilding onboarding, fleet control or observability. Users do not maintain a service per client. Upstream tools do the optimization; our small package manages their integration.

AgentQ also owns the reusable Codex/Claude/Antigravity worker runtime that TaskQ, PilotQ and other products use to run agent turns, along with repeatable worker benchmarks and a historical results viewer. A plain agent turn needs no optimizer installation; optimization is an explicit, optional choice per launch (R13).

## Requirements

- **R1 — One managed package.** Support macOS, native Windows and Linux; WSL has its own runtime on the same host. Manage Codex CLI/Desktop, Claude Code CLI/Desktop Code and qualified Hermes integrations. One local management entry point owns needed child processes; no manual per-agent service administration. Claude Desktop Chat supports explicit MCP only.
- **R2 — Retired document tools.** AgentQ no longer exposes index_document, search_document or get_original; measurement and return condition are in [OPTIMIZATION-RETIRED.md](OPTIMIZATION-RETIRED.md).
- **R3 — Retired optimization.** Native command output and context remain unchanged; AgentQ installs no optimization engine or provider proxy.
- **R4 — Preserve work.** Keep requests, native authentication, model, approvals, sandbox, hook trust and conversation history unchanged. Preserve the exact direct Codex provider definition for saved conversations. Retained data is not removed by qualification.
- **R5 — Easy onboarding.** One link starts agent-led discovery; offer all ready clients, allow deselection, reuse known TaskQ profiles. Request only missing input/required consent. Resolve actual config roots and user/project/environment precedence, including Desktop reload requirements. Explain changes, keep unrelated settings, verify real activity from each selected profile; configured is not verified. Team login uses device enrollment; employees receive no shared Langfuse secrets.
- **R6 — Transactional updates.** CoreQ owns release checksums, launcher, rollback and OS service updates. No optimization engine, proxy worker, port or connection drain is installed.
- **R7 — Ownership and removal.** Record each created/reused component and config edit before mutation, including native telemetry plugins. `uninstall --dry-run` previews; `uninstall` restores owned settings and removes owned dependencies, plugins, services and data. Preserve pre-existing/shared components, login and later unrelated edits; report conflicts. Repeat install/remove is safe. Disable/restore works even if proxy is down. Existing exported history remains.
- **R8 — Small, reliable state.** Durable local state holds ownership, identities, transactions, versions, incidents, original references and telemetry queue. Choose the simplest adequate storage in the technical plan; SQLite is an option, not a product requirement. No separate local DB server. Set explicit timeouts, memory/disk/queue caps, retention/freshness and recovery policy before qualification. Serialize lifecycle changes; survive interrupted transactions. Secrets stay in protected storage, never telemetry.
- **R9 — Consistent telemetry.** Native record hooks and runtime observations persist local model-call and turn metadata, with optional Langfuse export and enrollment. Unknown stays unknown; secrets and content stay outside default metadata.
- **R10 — Honest accounting.** Report observed model input, cache and output usage, latency, errors and attributable cost once. DashQ owns fleet presentation; retired optimization fields are unavailable, never zero savings.
- **R11 — Simple development/deploy.** One codebase and repeatable build/test/release workflow; small unavoidable OS adapters. Prefer ready upstream components, platform facilities and prebuilt artifacts; users do not compile or assemble toolchains. Added code/dependency/service/step requires a real benefit. Preserve effective features and reliability; no speculative framework. New runtime starts from an empty codebase, not inherited legacy implementation.
- **R12 — Spec-driven agents.** AI manager assigns bounded tasks against an approved spec revision. Independent acceptance verifies integrated outcomes before authorized merge/deploy. Behavior changes require discussion/evidence, approved spec amendment, then implementation. Minimum meaningful product scenarios plus targeted high-risk checks; no test-count/coverage quota or function-by-function duplication.
- **R13 — Reusable runtime.** Products launch, resume, cancel and observe native worker turns through AgentQ without a service installation. Preserve runtime modes, accounts, tools, policy, recovery and attributable observations. Optimization intent stays a compatible no-op until TaskQ, PilotQ and Pipe adopt it.
- **R14 — Reliable worker start.** Before a worker gets its task, the runtime checks everything that worker needs and repairs only what it owns. A worker starts ready, or it refuses before spending tokens, with the exact cause and next step. Nothing makes a worker silently wait for a person. Our optional observation parts never stop work. Unverified readiness stays visible, never reported as ready.

## Evidence and limits

Our experiments support noisy-command reduction and selective retrieval. They do **not** prove a released combined recipe or universal net savings. Tokenade saved input on noisy Claude tests but caused extra JSON reading; its live Codex probe did not compress. The experiment reports stay in git history before taskqorg/agentq#62.

Technology, operating limits and rollout policy live in [agentq.md](../../agentq.md). New code must meet this contract; old product deletion/deployment is a separate authorized step.

Accepted integration control interface: [CONFIGURATION A1–A5](CONFIGURATION.md), implementing R1/R4/R5/R7/R9 without model/provider/permission management.

Accepted R9/R10 accounting and effect definitions: [ACCOUNTING O1–O4](ACCOUNTING.md).
