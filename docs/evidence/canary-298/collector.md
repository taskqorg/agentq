# AgentQ 298 source collector candidate

This isolated descendant of `ae78c84af03069b55a766446dc3633ee0d6ffa15` replaces its deliberately blocked CLI with actual Runtime collection. The original blocked commit and [report](report.md) remain unchanged. The original base is `f0d129fc223c3db42a7273a68385b22f7a0e9140`.

The dispatch clarification authorizes exact requested Launch parameters, not a blanket pre-execution witness of all effective settings. That earlier inferred blocker is removed. Requested model/Policy/session policy/account home are durable authority; observed native conditions are independent evidence, never inferred from request equality. No native or paid execution was authorized or performed.

## Source and caller inventory

| Boundary | Caller and implementation | Evidence/unknown behavior |
|---|---|---|
| Explicit entry | Existing main canary arm → canary::request → request_with → collect::run | No default workflow, scheduler, service or hook invocation; stdin/request and owner-written receipt must agree exactly |
| Protected authority | canary::approved/protected_receipt; collect::parameters/executable | Existing installation and OS host identity, registered runtime, explicit canonical account home, model or explicit null, complete typed Policy, exact lifetime/readiness, scenario 1/native-turn and canonical executable path/SHA-256; no serde-defaulted Policy fields |
| Refusal before executable | request_with and collect::run entry | Missing/malformed/mismatched/unprotected authority is unknown; no native executable lookup or process occurs |
| Revalidation | Version observations before/after preflight, exact executable byte check, receipt recheck under existing lifecycle lock, final Configure spawn check | Changed version/bytes/receipt refuses; Configure is infallible, so an empty program makes spawn fail without executing a substitute; no retry, resume or fresh fallback |
| Launch | collect::run → Runtime preflight/security guards → Runtime::launch | One isolated workspace/session, literal prompt, exact approved home/model/Policy/lifetime/readiness; no auth or global configuration edits, no repair |
| Optional infrastructure | Explicit empty temporary Launch.install; no telemetry Store | Archive bookkeeping never falls back to host installation in the frozen candidate; no enrollment/grants or exporter are created |
| Native collection | Runtime progress/poll/report | Terminal and exact answer, event boundary and usage read independently; observer errors remain unknown; no cancellation because an observation failed |
| Canonical history | Runtime::history → typed steps, one full read of the newly launched session | Missing, partial or failed history is unknown; no reread loop over an incomplete tail; no product-native parser or transcript-copy fallback |
| Typed record detail | Runtime::transcript/log_records | Native decoding remains in Runtime; arbitrary record contents never survive diagnostic projection |
| Exact hook correlation | Existing model::session_start session-hash record, newly present after baseline and matching the actual Run session | Missing, preexisting or uncorrelated evidence is unknown; hook configuration/mtime alone is not proof; no new hook/trust file format |
| Conditions | Report usage model + history presentation model, typed recognized approval spellings | Observed model mismatch fails; recognized approval mismatch fails; full Policy, session policy and native account identity remain unknown where not observed; matching approval alone does not qualify a full Policy |
| Diagnostics | Fixed keys/discriminator allowlist → hashed unknown keys/types, counts and nullable numeric metadata | Arbitrary strings/keys/content never copied; recursive unknown fields detected; CLI report and persisted finding are separately projected |
| Verdict/finding | Production request_with → evaluate → finding → executor::lock/control::save | Fail/drift collection now reaches the durable helper; host/runtime/version/scenario identity keeps one open/not-submitted finding; persistence error cannot change native work |
| Metadata classification | Runtime history steps decoder + RUNTIME specification | Existing Codex rollout session_meta/turn_context are Metadata rather than false Unknown; unknown records still drift |

## Synthetic before/after proof

Before collection, no actual Runtime evidence caller reached evaluate/finding and the CLI always returned an unchanged-settings placeholder. After collection, a private fake executable is driven by the real registered Codex Runtime through version/login/preflight, one launch, poll/report, native-session history and exact session-hash hook evidence. No installed client is invoked.

The normal fixture yields five true scenario dimensions but overall unknown because applied native conditions are not all observable. Missing usage or exact hook correlation stays null/unknown. Wrong final answer and observed model mismatch yield fail; unknown native record/discriminator and a new nested field yield drift. A partial history tail stays unknown without a poll/retry loop.

All fixture turns assert the exact prompt and permitted native argv prefix. The seven normal/missing/drift/failure/partial cases each create exactly one native exec invocation. A separate model-mismatch case also runs once. Existing Runtime workspace refusal and changed detected version create no native turn. Changed executable bytes, missing/mismatched receipt and unprotected receipt refuse. Repeated fail/drift outcomes leave one durable finding. Both report and persisted evidence exclude the synthetic private strings and arbitrary field/type names.

The first collector fixture exposed a false drift on known session_meta; the native decoder correction makes the same normal fixture unknown with all five scenario dimensions true, while genuine new-record/new-field cases still drift. Source raw content is not a fixture export.

## Selected verification

- `cargo test --locked -p agentq --bin agentq canary:: -- --nocapture`: PASS, 2 tests, 100 filtered; final source run passed in 8.10 seconds after compilation. Development failures and subsequent corrections were limited to this selected synthetic chain.
- `cargo test --locked -p agentq-runtime --test history history_preserves_sources_and_recovers_without_replay -- --exact`: PASS, 1 test, 22 filtered; retained multi-format source/readback/recovery guard, no native executable.
- `AGENTQ_SELFTEST_OFFLINE=1 cargo run --locked -- selftest`: PASS on the final source, existing offline selftest only.
- Workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` contract lint: PASS, zero errors; preexisting line-headroom warning remains at 230 / 300 lines.
- `git diff --check`: PASS.

Only existing runtime dead-code/build-target warnings remain. No new unused-finding warning remains: production collection calls it. No full suite, Windows execution, CI, push, PR, release, tag, cron/service change or helper worker was run.

## Isolation limitation found during development

Early development fake runs set Launch.install to None. Source inspection then showed that archive::Handle::launch falls back through optimization::install_root to the host installation, and its asynchronous registration checks connection/connection.json before returning when no archive connection exists. Those early runs could therefore have attempted host archive-grant lookup and, if already enrolled, local capture registration. Actual host access or local side effects were not inspected because host/profile/config inspection was outside this task; their absence is **not proven**.

The frozen candidate explicitly binds optional archive bookkeeping to an empty owned temporary installation, and the final synthetic checks used that corrected path. No archive exporter, network notification, board delivery, service, installed native client, login flow or paid model call was invoked by this task. This limitation does not authorize inspecting, deleting or changing host state.

## Remaining external gates

Independent review is required before integration; none was performed because helpers/review workers were forbidden. No stopped provider/parent review was inspected or replaced. This commit is a source candidate, not integrated or stable.

Native qualification still needs separate explicit owner approval, protected per-host opt-in, one real client upgrade and the exact native scenario. No native version/platform was qualified by fixtures. Windows host identity remains unknown and refuses before executable; no invented host identity is used. Unobserved full Policy/session/account conditions remain unknown, not pass.

Issue 298's deduplicated board finding remains a separate explicit operator-approved delivery/API gate. Local files do not complete that requirement. No new generic workflow engine, cache, timeout, budget, retry limit, configuration key, trust file, telemetry-content export or automatic client update was added.
