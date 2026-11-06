# Agent registry refactor

Issue [292](https://github.com/taskqorg/agentq/issues/292), stacked on optimization retirement [304](https://github.com/taskqorg/agentq/pull/304). No deployment, working profile or installed AgentQ change is included.

`runtime/src/agent/registry/` contains one registration module each for Codex, Claude Code, Antigravity, Hermes and Claude Desktop Chat. Each owns its native policy/login mapping, home and executable lookup, descriptor/aliases, profile files, setup requests, hook actions and profile inspection where applicable. Shared runtime code retains process/transport ownership, recovery, observation and the Runtime trait; binary code retains journal/CAS/privacy/revision handling. Native protocol parsers stay private and unchanged.

`runtime/build.rs` discovers modules and generates references; it runs no agent and introduces no dependency. Ordering is registration data, preserving existing runtime, onboarding and profile order. The runtime and optimization compatibility APIs remain unchanged. Registry validation checks required metadata, identities, aliases, ordering and capability methods; Rust requires every callback and Runtime method. A compile-fail example proves an incomplete entry cannot compile.

`examples/fake.rs` is a complete fake runtime and integration entry, compiled through the same discovery path in a separate test catalog. No core list mentions it. Its native operations refuse explicitly, and its missing CLI prevents setup. Copying a module into the production catalog requires only that module's own unique identity and ordering data, never a core-list edit.

Onboarding checks CLI presence before installation/setup; configure and enable use the same pre-write setup gate. Pure journal fixture construction and owned upgrade cleanup remain separate from new setup. Existing behavior assertions are retained; setup fixtures now supply explicit native-presence files, never execute them and restore their temporary PATH. A missing-native regression proves no journal or profile is created.

Validation: `cargo test --locked`, `cargo run --locked -- selftest`, contract lint at workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51`, and Hermes record-hook checks. CI and the candidate commit are recorded in the PR. No live native runtime requalification is claimed for this source refactor.

Review correction: shared native parsers still contain Client branches; the catalog example proves discovery, not a new working native protocol. Their extraction and a running scripted CLI are a separate follow-up before identity #302. Capability checks now call operation paths in both directions for every registered worker, including native option mapping, modes, session IDs/URLs, image input, account, catalogue and quota observations; an unprobed declared capability fails. Required Runtime methods remain compile-checked by the fake implementation. Metadata validation no longer claims that a name whitelist proves behavior.

Automatic onboarding skips missing CLIs per profile; explicitly named missing clients refuse. Configure/enable still check all selected setup writes. Hook aliases keep their old no-op result, and the `agentq` hook key applies only to its registered owner. An npm fixture proves presence and launch share Codex's native resolution across a symlinked codex.js and sibling platform dependency; the requested resolver was already shared, so no alternate launcher fallback was added.

The #304 second-review fixes were already in this stack at ca2720a (parent 53611e1), including the selected-but-missing provider diagnosis in the Codex module, direct cleanup, ownership release and created-hook uninstall regression. The rebase onto main retains them.
