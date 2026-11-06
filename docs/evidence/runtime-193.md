# Protected receipt wire compatibility: task 193

Historical evidence at the cited revisions; optimization behavior below is retired. Current decision: [optimization retirement](../spec/OPTIMIZATION-RETIRED.md).

[Task #193](https://github.com/taskqorg/agentq/issues/193) requires consuming the published host authority receipt without weakening verification. This corrects the M12.8 wire lookup; the optional adapter, existing protected API and CoreQ-only runtime dependencies remain unchanged (M12.2–M12.3). Native composed/image qualification stays Open under A4.

## Producer and defect

The [AssetQ failure analysis](https://github.com/taskqorg/assetq/blob/13ae3b9dcbdef37ae65c57a04592dbfdb2d1e7fe/docs/blockers/4-agentq-receipt-wire.md) identifies the mismatch. Read-only inspection of TaskQ `56b1a1eba274bf88520e4d668461372fe3048b00` confirms that `policy::Effective::json` emits `runtime`, `requested`, `grant`, `effective`, `enforcement` and `refused`; `runtime::protected_app_server::resolve` stores that object as `authority.resolved`. AgentQ baseline `6585656b5966f6f5912f504b6bf30f5bc576acaa` instead requires `resolved.mode`. Its mode-only fake receipt hides the incompatibility.

Under P3/R13, the owning [runtime detail](../spec/RUNTIME.md#protected-app-server) was aligned before implementation. The validator now requires `launch.authority.resolved.effective == "workspace"`; it accepts no `mode` alias. All other authority, identity, incarnation and binding checks and fixed generic diagnostics remain unchanged. No raw receipt, path or key was added to public diagnostics.

## Failure before and pass after

Following specq Testing's development rule, change only the existing fake host's `resolved` object to the producer's six-field shape with explicitly invented values, leaving the validator unchanged. On Windows:

```text
cargo test --locked -p agentq-runtime --test agent -- protected_
exit 101
protected authority missing, unknown or mismatched; no native input
```

The process scenario fails at `server.verify`, before its initialize exchange, matching the reported authority refusal. This is a safe producer-shaped process-double reproduction, not a real host launch or proof about the PM's remaining native receipt fields.

After the one-field lookup correction, the same command exits 0 with its protected process scenario passing. The retained regression extends the existing refusal table with Full and null `effective`, absent `effective`, null `resolved` and mode-only evidence; every case refuses before fake native stdin. Existing stale/tampered grant, revision, approver, project/profile, account, image, adapter, operation and process checks, interrupted/unknown readback recovery, permanent changed-binding fence, consumed-operation no-replay, cancellation and one attributable observation across recovery remain intact. No assertion or authority guard was relaxed.

## Corrected source and acceptance boundary

Corrected immutable runtime source: `4a3613759514bca4366caacca60745eaa618ecf2`. The required pre-first-push rebase on `origin/main` completed without changes. Consume this Git revision only after TaskQ review/publication accepts it with green merge-result CI (§7/R12); source availability or portable success alone is not publication acceptance. Later evidence-only commits retain identical runtime source, tests and dependencies.

The dependency remains `agentq-runtime = { git = "https://github.com/taskqorg/agentq", rev = "4a3613759514bca4366caacca60745eaa618ecf2" }`. This task performs no consumer adoption or crate replay.

The Windows candidate gate on this source passed `cargo test --locked` and offline `cargo run --locked -- selftest` in 220.648 seconds total (Rust tests: 220.135 seconds). `AGENT_CONTEXT_HOME` and `npm_config_prefix` were isolated in OS temporary storage, so legacy-removal fixtures could not remove the real package. All 15 runtime process scenarios passed, including the protected gate and ordinary transport/optimization/consumer paths. The four workflow Python suites passed 9/11/2/4 cases. Contract lint from workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51` passed with zero errors and the existing contract headroom warnings. `git diff --check` passed. `git diff --exit-code 4a3613759514bca4366caacca60745eaa618ecf2 -- runtime/ Cargo.toml Cargo.lock` confirms the later evidence/documentation edits do not change gated runtime source or dependencies.

A read-only Astra security review inspected the immutable corrected source, full affected specification/validator/fixture, producer source and before/after logs. It found no blocking task-fit or implementation-quality findings. It reran no tests. No native containment or image capability was inferred from that review or these portable checks.

Open: green publication CI and reviewer acceptance; PM qualification of the composed native API and current image-tool/output capability after publication. Portable checks do not qualify Mac containment or image/3D generation. No native/model/paid call, grant/config/account/capacity change, deployment or retained AssetQ crate replay occurred.
