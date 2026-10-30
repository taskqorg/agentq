# agentq

Shared rules: https://github.com/taskqorg/specq (binding for every task). Used owner pages: [principles](https://github.com/taskqorg/specq/blob/main/engineering/principles.md), [spec first](https://github.com/taskqorg/specq/blob/main/engineering/spec-first.md), [release contract](https://github.com/taskqorg/specq/blob/main/engineering/release-contract.md), [verification](https://github.com/taskqorg/specq/blob/main/engineering/verification.md), [documentation](https://github.com/taskqorg/specq/blob/main/caveman/rules/documentation.md), [project contracts](https://github.com/taskqorg/specq/blob/main/engineering/contracts.md).

Read [agentq.md](agentq.md) first; a decision change edits it or its linked detail before code. Product outcomes: [docs/spec/PRODUCT.md](docs/spec/PRODUCT.md); acceptance: [docs/spec/VERIFICATION.md](docs/spec/VERIFICATION.md).

Install, onboarding or removal at a user's request: [INSTALL.md](INSTALL.md).

Checks: `cargo test --locked`, `cargo run --locked -- selftest`, and the contract lint `python3 <specq>/engineering/scripts/contract_lint.py agentq.md` at the specq commit pinned in `.github/workflows/test.yml`. Test legacy removal (agentq.md § 10) only in an isolated `AGENT_CONTEXT_HOME` and `npm_config_prefix`, else it removes the real global npm package. Qualifying a candidate does not authorize deploying it to working profiles or calling it stable.
