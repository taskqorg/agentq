# AgentQ 298 blocked candidate

Base: `f0d129fc223c3db42a7273a68385b22f7a0e9140`. This is preparatory source, **not a complete canary implementation** or integration approval. Issue 298 body and all four owner comments were read through read-only `gh issue view`; the finish inventory was read from `/tmp/agentq-refactor-finish-inventory-56fd30e/report.md`.

The coordinator approved the finite authority/scenario/privacy contract through question `msg_d8d2590f8e72` (response `msg_176222741bb3`). No native execution or per-host approval was granted. No further design proposal was opened.

## Implemented boundary and callers

| Area | Source/caller | Observed result |
|---|---|---|
| Contract | agentq.md M7.2; VERSION-CANARY.md; VERIFICATION canary scenario | Exact revision 1 prompt, authority fields, independent dimensions, privacy and separate native/board gates |
| Explicit CLI | agentq/main.rs canary arm → canary::request | Reads binding on stdin; no registration, grants or default caller; all present native scenarios refuse unknown |
| Authority | canary.rs approved/request_with | Reads existing installation and owner-written receipt only; exact fields and equality, independent OS hostname; rejects before executable resolution; lifecycle lock and recheck before version probe |
| Version | canary.rs version/version_at → registry::require_cli → existing coreq process output; host::version_in | Only numeric dotted output survives; uses existing 5-second version-probe policy; never login/readiness or account files |
| Verdict | canary.rs evaluate | Synthetic independent terminal/events/history/hook-correlation/usage checks: false fail, new shape drift, missing unknown, otherwise pass |
| Privacy | canary.rs shape; version-canary-drift.json | Fixed keys and known discriminator values only; arbitrary names/discriminators hashed; content discarded before output/persistence |
| Finding | canary.rs finding → executor::lock/control::save | Synthetic-only caller; host/runtime/version/scenario identity, explicit open/not-submitted state, atomic private write, repeated finding replaces same file |
| Selftest | main.rs selftest → canary::selftest | Offline evaluator/privacy checks; no native calls |

No caller currently joins native evidence to evaluate/finding. These helpers are preparation with synthetic coverage, not a working failure-report path. The non-test build truthfully warns that `finding` is unused.

## Precise remaining source and evidence blockers

`canary::request_with` intentionally returns `unchanged-native-settings-witness-unavailable` after an authorized version observation; every scenario dimension stays null. The existing `Runtime::ready_for`/preflight checks installation/login/version/policy readiness, not equality of effective native model, permissions and session policy to the durable receipt. Calling it would add login probes without supplying the missing witness.

`runtime/src/cli.rs:57-67` builds model/policy/command from caller input. `runtime/src/agent/registry/codex.rs:41-68` maps policy and selected model into explicit native arguments. Native defaults can remain unset, but these seams provide no independent current-settings comparison. A receipt assertion alone is not that comparison. This candidate therefore does not launch, choose a policy, silently assume settings equality, or claim a native capability.

Completing the native runner still requires a content-free unchanged-settings witness per runtime, one isolated native invocation through the existing Runtime API without settings overrides, and real event/history/hook-correlation/usage collection for that exact run. Hook age or configured hooks alone cannot prove correlation. No generic runner, retry, cache or new resource gate was introduced to hide those gaps.

The supplied scope forbids helpers; no independent review occurred. **Independent review is required before integration.** No stopped parent/provider review was inspected or substituted.

Issue A3 requires a deduplicated board finding. Existing agentq.md section 7 only permits catalog update/queue-overflow reports; no authorized board API/target for canary findings was established. Local fixtures do not complete board delivery. A separately explicit delivery operation/API and owner approval remain necessary, with no actual submission in this task.

Non-Unix host identity is deliberately unavailable and refuses before any executable; Windows/native per-platform behavior was not qualified. Version suffixes/prereleases conservatively remain unknown. Detected version is not cached or persisted as tested capability. A changed registered executable between invocations is not a qualification claim.

## Synthetic verification

- `cargo test --locked -p agentq --bin agentq canary::tests:: -- --nocapture`: PASS, one test, 100 filtered; same selected check ran twice while finishing fixture wiring, both passed. It covers invalid/missing/mismatched/corrupt authority, each unknown/fail dimension, pass/drift evaluation, content-bearing arbitrary key/discriminator projection, actual private persisted evidence and repeated durable identity, plus owned temporary executable version output and unrecognized output.
- `AGENTQ_SELFTEST_OFFLINE=1 cargo run --locked -- selftest`: PASS (`selftest: ok 0.0.0-dev macos-aarch64`). Existing runtime dead-code warnings plus preparatory unused finding warning remain.
- Workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` lint extracted using local `git show`: PASS, 0 errors; existing contract line-headroom warning remains (230 lines / cap 300).
- Built CLI `canary` with `{}` stdin: exit 1, opt-in-unverified unknown, all five dimensions null. No native client was invoked.
- `git diff --check`: PASS.

No full suite, Windows tests, installed native client, auth/account/profile/config read, live or paid scenario, upgrade, grant, service/cron mutation, notification/issue/comment, push, PR, CI, tag, release or deployment occurred. Actual native upgrade proof, board delivery and independent review remain open; AgentQ298 is not complete.
