# Optimization results

Accepted under agentq.md §3/§6 and PRODUCT R9/R10. [DashQ presentation](https://github.com/taskqorg/dashq/blob/main/docs/design/optimization-results.md) consumes these definitions; design approval is not live accounting qualification.

## O1. Populations and provenance

Reuse the existing stats record, bounded queue and export path. One distinct core event ID contributes once to exactly one outcome: compressed, passed or failed. Calls are core optimization calls, not all model requests, tool calls or MCP retrievals. Preserve canonical reason codes; a normal pass-through is not an error, and optimization failure is not task failure. Legacy run/compress overlap is deduplicated only with proven event identity; otherwise show separate cohorts and incomplete coverage.

Use source event time in a half-open [start,end) interval; preserve installation/runtime, opaque profile identity where available, adapter, recipe/engine/config version and safe evidence IDs. Unknown profile is a separate unknown cohort, never inferred from host name. Publish only metadata through existing authorized paths: no prompts, arguments, raw output, paths or credentials. Compatible additive fields stay versioned; unsupported schemas and future/uncertain timestamps cannot silently enter totals.

## O2. Measured volume

Compare bytes_in with the actual delivered bytes_out, including recovery references, for the same compressed calls. Report paired sample count and total observed compressed count. Sum paired before/after bytes before computing reduction; do not average percentages or extrapolate missing pairs. Invalid/nonfinite/negative or incomparable measurements are excluded with coverage reasons. MiB means 2^20 bytes. Native RTK and post-execution headroom remain distinct recipes.

Estimated token fields remain estimates with their method/version; never relabel bytes or estimates as observed model tokens, saved cost or subscription savings. A larger result is not a gain. Original-output fallback must be supported by adapter evidence, not inferred from a failed count.

## O3. Coverage and task effect

Export/query source observation time, covered installations/intervals, caps, dropped records, resets and known delivery gaps. Complete empty windows are zero; unreadable/missing windows are unknown. Partial windows show observed counts and lower bounds only when valid; no complete-period rate. Bucket on event time; gaps are not zeros and delayed data revises its original bucket. Bound queries using existing retention/caps; no second collector, permanent supervisor or analytics database.

Whole-task effect requires an explicitly separate paired experiment cohort: same work and comparable model/recipe/config, baseline and optimized runs, quality criteria/results, attempts/retries, retrieval/rework, latency and available observed usage. Keep input/cache/output/reasoning fields and their sample coverage distinct; overlapping token categories are not summed. Cost carries source, currency and price basis; calculated cost is labelled estimated. Missing cost/quality/comparison stays unknown.

Show sample size, versions and evidence references; state the scope and uncertainty of the comparison. Include measured overhead and failed attempts. A small passing cohort proves only those runs; it does not establish fleet-wide benefit or universal causal savings. The selected live period is not the experiment cohort. Reuse available experiment evidence; do not add an experiment runner in this work.

## O4. Qualification

Publish schema/aggregation conformance fixtures for duplicate/late events, half-open boundaries, mixed versions/recipes, unknown profiles, valid zero, partial/offline sources, capped queries, malformed byte pairs and process resets. Verify exact outcome/reason sums, byte units and pair coverage. Test estimates and overlapping token fields cannot enter observed totals or cost savings.

Qualify a real supported client path against local stats and delivered events with exact source/consumer versions. Record evidence and remaining adapter/platform gaps. Read-only results ship independently of profile management; no live configuration or rollout permission is implied.

## Published schema

`agentq.accounting/1`. Producer: the [stats record](COMPRESSION.md#stats-record) of agentq.md § 3 and its `agentq.compress` span. Aggregate: `agentq/accounting.rs`, also `agentq stats --accounting`. Input, output and conformance cases: [accounting-fixtures.json](accounting-fixtures.json); a consumer conforms when every case's `expect` holds. Additive keys keep the version; a changed meaning takes a new one. Qualification record: [VERIFICATION](VERIFICATION.md#accounting-qualification).
