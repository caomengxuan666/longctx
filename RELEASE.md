# LongContextBench v0.2.0

This release adds a unified scoring CLI for long-context model evaluation.

## Install

Download the archive for your platform from the release assets, extract it, and run `longctx --help`.

Windows:

```powershell
.\longctx.exe --help
```

Linux and macOS:

```sh
./longctx --help
```

## Configure a Provider

Create a `config.toml` file:

```toml
[provider]
base_url = "https://api.example.com/v1"
api_key_env = "OPENAI_API_KEY"
model = "your-model"
request_style = "responses" # or "chat-completions"

[run]
request_timeout_secs = 900
max_retries = 0
retry_backoff_ms = 1000
concurrency = 1
```

Set the API key environment variable named by `api_key_env`.

Windows PowerShell:

```powershell
$env:OPENAI_API_KEY = "sk-..."
```

Linux and macOS:

```sh
export OPENAI_API_KEY="sk-..."
```

## Score a Model

Quick smoke score:

```sh
longctx score ./score --config ./config.toml --profile quick
```

Recommended comparable score:

```sh
longctx score ./score --config ./config.toml --profile standard
```

Maximum-context-only probe:

```sh
longctx score ./score --config ./config.toml --profile max-context
```

The command prints a terminal summary and writes:

- `score/probe-runs/<id>/score.html`
- `score/probe-runs/<id>/score.json`
- `score/probe-runs/<id>/probe-summary.json`
- Per-attempt benchmark artifacts under the same run directory

## Profiles

- `quick`: probes 8K to 64K and runs 8K capability suites.
- `standard`: probes up to 1M with 8K resolution and runs 32K capability suites.
- `deep`: probes up to 1M with 4K resolution and runs 128K capability suites.
- `max-context`: only finds the context boundary.

## Lower-Level Commands

Use `probe-context` for direct boundary probing:

```sh
longctx probe-context ./probe --config ./config.toml --max-tokens 1000000
```

Use `generate`, `run`, `report`, and `compare` for custom benchmark workflows.
