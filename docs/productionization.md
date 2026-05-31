# Productionization Roadmap

This project already has the basic loop:

1. Generate benchmark data.
2. Run it against an OpenAI-compatible provider.
3. Grade the answers.
4. Render a report.

The next work should make the loop dependable enough for repeat use.

## Completed baseline

- Deterministic generation with an explicit `seed`.
- Standard TOML parsing for `config.toml`.
- A `validate` command for config and manifest preflight checks.
- Retry and timeout controls in the runner.
- Versioned benchmark/result schema.
- Stable benchmark directory layout.
- Better report output with raw answers and attempt counts.
- Provider model/base URL, token count, and structured error kind in result rows.
- HTTP status, provider request ID, and rate-limit headers in result rows.
- Opt-in redacted HTTP exchange logs with request IDs and content hashes.
- Result-set comparison with optional failure on regression.
- Standalone comparison reports between two `results.jsonl` files.
- Comparison deltas for latency and token usage.
- Per-suite report summaries and failure grouping.
- Per-token-count report summaries.
- Latency and input-token trend charts in reports.
- Partial-match, generic JSON, and schema-aware JSON field graders.
- Schema migration notes for `schema_version`.
- Run snapshot file with config and timing metadata.
- Parallel execution with a configurable concurrency limit.
- CI checks for formatting, linting, unit tests, builds, and doc drift.
- Machine-readable report summary exports.
- Provider-specific adapters for chat completions and responses APIs.
- Zero-token automatic context routing with a reproducible `context.index.json`.
- Per-result routing audit metadata and report columns for routed runs.

## Next priorities

### P0

- No remaining P0 items.

### P1

- No remaining P1 items.

### P2

- No remaining P2 items.
