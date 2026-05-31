# LongContextBench

LongContextBench is a long-context benchmark CLI for testing how well LLMs recall facts, compose multiple facts, and resolve conflicting information at large context sizes.

The `longctx` binary can generate synthetic benchmark suites, run them against OpenAI-compatible chat completion APIs, and turn JSONL results into a local HTML report.

Design and configuration notes live in [docs/configuration.md](docs/configuration.md), [docs/productionization.md](docs/productionization.md), and [docs/schema-migration.md](docs/schema-migration.md).

## Install

```sh
cargo install --git https://github.com/LibSkills/longctx.git
```

## Commands

### `generate`

Generate synthetic benchmark cases and context files.

Supported suites:

- `needle`: hides one target fact in a long filler context.
- `multi-needle`: hides several facts that must be returned together.
- `conflict`: includes older or adjacent facts and asks for the scoped current answer.
- `multi-hop`: chains two related facts (e.g. project lead and badge code) and asks for a transitive answer.
- `order-dependent`: generates sequenced facts with timestamps and asks for a value at a specific step.
- `position-sweep`: places a needle at 5 positions (start/early/middle/late/end) to measure the "lost in the middle" effect.
- `hallucination`: pure filler context with questions about non-existent facts; tests whether the model fabricates answers.

```sh
longctx generate needle --tokens 100000 --out ./bench
longctx generate multi-needle --tokens 100000 --out ./bench
longctx generate conflict --tokens 100000 --out ./bench
longctx generate multi-hop --tokens 100000 --out ./bench
longctx generate order-dependent --tokens 100000 --out ./bench
longctx generate position-sweep --tokens 100000 --out ./bench
longctx generate hallucination --tokens 100000 --out ./bench
```

Add `--seed <value>` to make generation reproducible.

### `run`

Run generated benchmark cases against an OpenAI-compatible provider. The benchmark directory must contain a `config.toml` file.

```sh
longctx run ./bench
```

Results are written to `./bench/results.jsonl`.

### `validate`

Validate a benchmark directory before running it.

```sh
longctx validate ./bench
```

Use `--skip-api-key-check` when you only want to check files and schema.

### `index`

Build or refresh the context index used by automatic context routing.

```sh
longctx index ./bench
```

Generated suites write this index automatically. Run `index` after hand-editing manifests or context files.

### `report`

Generate a standalone HTML report from a results JSONL file.

```sh
longctx report ./bench/results.jsonl --out report.html
longctx report ./bench/results.jsonl --json
longctx report ./bench/results.jsonl --json --out report.json
```

The report includes per-suite summaries, per-token-count summaries, trend charts, and failure groups.
When result rows include automatic routing decisions, the report shows the selected context, routing method, status, and confidence.
`--json` emits a machine-readable summary.

### `compare`

Compare two result files.

```sh
longctx compare baseline.jsonl candidate.jsonl
longctx compare baseline.jsonl candidate.jsonl --json
longctx compare baseline.jsonl candidate.jsonl --html-out compare.html
longctx compare baseline.jsonl candidate.jsonl --fail-on-regression
```

`compare --json` emits machine-readable deltas, including average latency and token changes.
`compare --html-out` writes a standalone comparison report.

## Quick Start

Generate a 100K-token needle retrieval test:

```sh
longctx generate needle --tokens 100000 --out ./bench
```

Create `./bench/config.toml`:

```toml
[provider]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model = "gpt-4.1"

[run]
request_timeout_secs = 120
max_retries = 2
retry_backoff_ms = 500
concurrency = 4
```

Run the benchmark:

```sh
export OPENAI_API_KEY="sk-..."
longctx run ./bench
```

Generate the report:

```sh
longctx report ./bench/results.jsonl --out report.html
```

## Config File Format

`longctx run` reads `config.toml` from the benchmark directory.

```toml
[provider]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model = "gpt-4.1"
```

Fields:

- `base_url`: Provider base URL. The runner posts to `{base_url}/chat/completions`.
- `api_key_env`: Name of the environment variable containing the API key.
- `model`: Model name sent in the chat completion request.
- `request_timeout_secs`: Per-request timeout in seconds.
- `max_retries`: Retry budget for transient failures.
- `retry_backoff_ms`: Base retry delay in milliseconds.
- `concurrency`: Number of benchmark requests to run in parallel.
- `request_style`: Provider request style, either `chat-completions` or `responses`.
- `log_requests`: Write opt-in redacted HTTP exchange logs under `reports/`.
- `request_log_path`: Path for the request log file, defaulting to `reports/http-log.jsonl`.

The config also supports an optional `[grader]` section for LLM-as-judge grading:

```toml
[grader]
judge_model = "gpt-4.1-mini"  # optional, defaults to the same provider model
```

When a test case uses `Grader::LlmJudge`, the runner sends the answer to the judge model for evaluation instead of using exact string matching.

Any provider that exposes an OpenAI-compatible `/chat/completions` endpoint can be used by changing `base_url`, `api_key_env`, and `model`.

## Generated Files

Each generated suite writes:

- A context text file under `contexts/`.
- A suite manifest JSON file under `manifests/`.
- A context routing index at `context.index.json`.

The runner accepts generated suite manifests and writes newline-delimited JSON results to `results.jsonl`.
Result rows include the suite name, token count, provider model, HTTP status, provider request ID, rate-limit headers, attempt count, structured error kind when a run fails, and optional routing audit details.
Each run also writes a `run.json` snapshot with config and timing metadata.
When `run.log_requests` is enabled, redacted HTTP exchange logs are written to `reports/http-log.jsonl`.

## Automatic Context Routing

Set a test case's `context = "auto"` to let the runner select a context file from `context.index.json` instead of naming a file directly.

The first implementation is a zero-token local router. It scores context-level index entries using safe manifest metadata, test IDs, suite names, token counts, and lexical overlap with the question. It does not call an LLM router and does not inspect `expected` answers. If routing is ambiguous, the result fails with `error_kind = "ContextRoute"` instead of silently choosing a weak candidate.

Routing decisions are written into each result row under `routing`, including selected context path, candidate scores, method, status, confidence, token usage, and latency fields. The token and latency fields are currently zero because the local router does not spend model tokens.

## Contributing

Contributions are welcome. Before opening a pull request:

1. Run `cargo fmt`.
2. Run `cargo test`.
3. Keep changes focused on one behavior or feature.
4. Include documentation updates when CLI behavior, config, or output formats change.

Useful local checks:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
```
