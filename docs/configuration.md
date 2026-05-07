# Configuration

`longctx run` reads `config.toml` from the benchmark directory.

Example:

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

Fields:

- `provider.base_url`: OpenAI-compatible API root.
- `provider.api_key_env`: Environment variable that stores the API key.
- `provider.model`: Model name sent in each request.
- `provider.request_style`: Request style, either `chat-completions` or `responses`.
- `run.request_timeout_secs`: Per-request timeout in seconds.
- `run.max_retries`: Additional retry attempts for transient failures.
- `run.retry_backoff_ms`: Base delay between retries.
- `run.concurrency`: Number of benchmark requests to run in parallel.
- `run.log_requests`: Write opt-in redacted HTTP exchange logs under `reports/`.
- `run.request_log_path`: Relative or absolute path for the request log file.

Generated benchmark layout:

- `manifests/*.json`: Suite manifests.
- `contexts/*.txt`: Generated context files.
- `results.jsonl`: Benchmark run output with suite, token count, provider, request, latency, token, and error metadata.
- `reports/http-log.jsonl`: Optional redacted request log output when enabled.
- `run.json`: Run snapshot with timing and config metadata.
- `report.html` or `reports/*.html`: Human-readable report output.
