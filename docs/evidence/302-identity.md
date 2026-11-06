# Product identity revision

Issue [302](https://github.com/taskqorg/agentq/issues/302), rebased onto main after native protocol extraction [312](https://github.com/taskqorg/agentq/pull/312), observation [319](https://github.com/taskqorg/agentq/pull/319), platform metadata [321](https://github.com/taskqorg/agentq/pull/321) and preflight [320](https://github.com/taskqorg/agentq/pull/320).

PRODUCT revision 6 and the root requirement map identify AgentQ as the single worker layer for preparation, launch/resume, observation, control, history and archive. R1–R14 retain their outcome identities; R2 and R3 are explicitly retired. Current implementation and accepted targets remain distinct: worker preflight is implemented; archive support and native qualification retain their stated limits. README, INSTALL and package descriptions follow that role. Products retain admission, grants, scheduling and UI.

Historical optimizer procedures, research, evidence and released changelog entries are marked historical; their recorded results remain intact. The sole parked decision, measurements and return condition remain in `docs/spec/OPTIMIZATION-RETIRED.md`. Active local SQLite constraints move to the current telemetry owner page with their existing values unchanged.

The shared specq runtime/history/archive and telemetry pages already assign these responsibilities to AgentQ. Current product pages link to those owner pages; no specq repository edit or duplicate shared rule is needed. Changed Markdown file targets and local anchors are checked, with GitHub file targets verified through the API.

Canonical GitHub repository description: `Single worker layer for native agents: prepare, launch, observe, control, history and archive`. The repository metadata was updated and read back as this exact text; this metadata result is separate from code and CI evidence.

This change updates documentation and package metadata only. No runtime behavior, installed binary, working profile, host service or TaskQ state changes; no deployment or native model call is part of this identity revision. This docs-only refresh uses documentation/link checks, locked Cargo metadata and contract lint at workflow-pinned specq `7e9adf57287553176d8b3be61c2b22565543dc51`; full runtime gates run in CI.
