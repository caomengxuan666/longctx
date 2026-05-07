# LongContextBench

LongContextBench is a long-context benchmark CLI for testing how well LLMs recall facts, compose multiple facts, and resolve conflicting information at large context sizes.

The `longctx` binary can generate synthetic benchmark suites, run them against OpenAI-compatible chat completion APIs, and turn JSONL results into a local HTML report.

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

```sh
longctx generate needle --tokens 100000 --out ./bench
longctx generate multi-needle --tokens 100000 --out ./bench
longctx generate conflict --tokens 100000 --out ./bench
```

### `run`

Run generated benchmark cases against an OpenAI-compatible provider. The benchmark directory must contain a `config.toml` file.

```sh
longctx run ./bench
```

Results are written to `./bench/results.jsonl`.

### `report`

Generate a standalone HTML report from a results JSONL file.

```sh
longctx report ./bench/results.jsonl --out report.html
```

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

Any provider that exposes an OpenAI-compatible `/chat/completions` endpoint can be used by changing `base_url`, `api_key_env`, and `model`.

## Generated Files

Each generated suite writes:

- A context text file, such as `needle_context.txt`.
- A suite manifest JSON file, such as `needle.json`.

The runner accepts generated suite manifests and writes newline-delimited JSON results to `results.jsonl`.

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
cargo build
```
