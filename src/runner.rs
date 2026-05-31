use crate::benchmark::{
    BenchmarkResult, Config, ErrorKind, GraderConfig, ProviderConfig, RoutingDecision, RunConfig,
    RunMetadata, SuiteManifest, TestCase, SCHEMA_VERSION,
};
use crate::context_index::{
    is_auto_context, load_or_build_context_index, route_context, validate_context_index,
    write_context_index, ContextIndex,
};
use crate::grader::{grade, grade_with_llm};
use crate::tokenizer::TokenCounter;
use crate::validator::validate_loaded_benchmark;
use anyhow::{anyhow, Context, Result};
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::task::JoinSet;
use tokio::time::sleep;
use walkdir::WalkDir;

pub async fn run_benchmarks(bench_dir: &str) -> Result<()> {
    run_benchmarks_with_options(bench_dir, RunOptions::default()).await
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    pub force: bool,
}

pub async fn run_benchmarks_with_options(bench_dir: &str, options: RunOptions) -> Result<()> {
    let bench_path = Path::new(bench_dir);
    let config = read_config(&bench_path.join("config.toml"))?;
    let tests = read_tests(bench_path)?;
    if tests.is_empty() {
        return Err(anyhow!("no benchmark .json files found in {bench_dir}"));
    }
    validate_loaded_benchmark(bench_path, &config, &tests, false)?;

    let api_key = env::var(&config.provider.api_key_env).with_context(|| {
        format!(
            "environment variable {} is not set",
            config.provider.api_key_env
        )
    })?;

    let context_index = if tests.iter().any(|test| is_auto_context(&test.context)) {
        let index = load_or_build_context_index(bench_path)?;
        validate_context_index(bench_path, &index)?;
        write_context_index(bench_path, &index)?;
        Some(index)
    } else {
        None
    };

    let results_path = bench_path.join("results.jsonl");
    let metadata_path = bench_path.join("run.json");
    let request_log_path = config
        .run
        .log_requests
        .then(|| bench_path.join(&config.run.request_log_path));
    ensure_output_paths_can_be_written(&results_path, request_log_path.as_deref(), options.force)?;

    let run_started = Instant::now();
    let started_at_unix_ms = unix_ms_now();
    let mut metadata = build_run_metadata(
        bench_path,
        &results_path,
        started_at_unix_ms,
        &config,
        &tests,
    );
    write_run_metadata(&metadata_path, &metadata)?;

    let mut results_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&results_path)
        .with_context(|| format!("failed to open {}", results_path.display()))?;

    let client = Client::builder()
        .build()
        .context("failed to build HTTP client")?;
    let counter = TokenCounter::cl100k();
    let tests_for_run = tests.clone();
    let results = run_all(
        client,
        config.provider.clone(),
        config.run.clone(),
        config.grader.clone(),
        api_key,
        bench_path.to_path_buf(),
        tests_for_run,
        context_index,
        &counter,
    )
    .await?;

    for result in &results {
        writeln!(results_file, "{}", serde_json::to_string(&result)?)?;
    }
    if config.run.log_requests {
        let log_path = request_log_path
            .as_deref()
            .unwrap_or_else(|| Path::new(&config.run.request_log_path));
        write_request_logs(bench_path, log_path, &tests, &results)?;
    }
    metadata.finished_at_unix_ms = Some(unix_ms_now());
    metadata.duration_ms = Some(run_started.elapsed().as_millis() as u64);
    write_run_metadata(&metadata_path, &metadata)?;

    Ok(())
}

fn ensure_output_paths_can_be_written(
    results_path: &Path,
    request_log_path: Option<&Path>,
    force: bool,
) -> Result<()> {
    if force {
        return Ok(());
    }
    if results_path.exists() {
        bail_existing_output(results_path)?;
    }
    if let Some(path) = request_log_path {
        if path.exists() {
            bail_existing_output(path)?;
        }
    }
    Ok(())
}

fn bail_existing_output(path: &Path) -> Result<()> {
    anyhow::bail!(
        "refusing to overwrite existing output {}; rerun with --force to replace it",
        path.display()
    )
}

fn build_run_metadata(
    bench_path: &Path,
    results_path: &Path,
    started_at_unix_ms: u64,
    config: &Config,
    tests: &[TestCase],
) -> RunMetadata {
    RunMetadata {
        schema_version: SCHEMA_VERSION,
        bench_dir: bench_path.to_string_lossy().into_owned(),
        results_path: results_path.to_string_lossy().into_owned(),
        started_at_unix_ms,
        finished_at_unix_ms: None,
        duration_ms: None,
        test_count: tests.len(),
        provider_model: config.provider.model.clone(),
        provider_base_url: config.provider.base_url.clone(),
        provider_request_style: config.provider.request_style.as_str().to_string(),
        request_timeout_secs: config.run.request_timeout_secs,
        max_retries: config.run.max_retries,
        retry_backoff_ms: config.run.retry_backoff_ms,
        concurrency: config.run.concurrency.max(1),
        log_requests: config.run.log_requests,
        request_log_path: if config.run.log_requests {
            Some(config.run.request_log_path.clone())
        } else {
            None
        },
        suites: suite_names(tests),
    }
}

fn write_run_metadata(path: &Path, metadata: &RunMetadata) -> Result<()> {
    let json = serde_json::to_string_pretty(metadata)?;
    fs::write(path, json).with_context(|| format!("failed to write {}", path.display()))
}

fn write_request_logs(
    bench_path: &Path,
    log_path: &Path,
    tests: &[TestCase],
    results: &[BenchmarkResult],
) -> Result<()> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory {}", parent.display()))?;
    }

    let mut log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)
        .with_context(|| format!("failed to open request log {}", log_path.display()))?;

    for (test, result) in tests.iter().zip(results.iter()) {
        let routing = result.routing.as_ref();
        let context_path = routing
            .and_then(|routing| routing.selected_context_path.as_deref())
            .unwrap_or(&test.context);
        let request_hash = if result.attempts > 0 {
            load_context(bench_path, context_path)
                .map(|context| build_prompt(&context, &test.question))
                .map(|prompt| sha256_hex(&prompt))
                .ok()
        } else {
            None
        };
        let response_hash = result
            .answer
            .as_ref()
            .or(result.error.as_ref())
            .map(|body| sha256_hex(body));
        let entry = HttpExchangeLog {
            schema_version: SCHEMA_VERSION,
            logged_at_unix_ms: unix_ms_now(),
            id: result.id.clone(),
            suite: result.suite.clone(),
            request_id: result.request_id.clone(),
            http_status: result.http_status,
            attempts: result.attempts,
            latency_ms: result.latency_ms,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            passed: result.passed,
            error_kind: result.error_kind.clone(),
            routing_status: routing.map(|routing| routing.status.clone()),
            routing_reason: routing.and_then(|routing| routing.reason.clone()),
            selected_context_path: routing
                .and_then(|routing| routing.selected_context_path.clone()),
            request_sha256: request_hash,
            response_sha256: response_hash,
        };
        writeln!(log_file, "{}", serde_json::to_string(&entry)?)?;
    }

    Ok(())
}

fn suite_names(tests: &[TestCase]) -> Vec<String> {
    let mut suites = tests
        .iter()
        .filter_map(suite_name)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if suites.is_empty() {
        suites.push("unknown".to_string());
    }
    suites
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[allow(clippy::too_many_arguments)]
async fn run_all(
    client: Client,
    provider: ProviderConfig,
    run: RunConfig,
    grader_config: GraderConfig,
    api_key: String,
    bench_path: PathBuf,
    tests: Vec<TestCase>,
    context_index: Option<ContextIndex>,
    counter: &TokenCounter,
) -> Result<Vec<BenchmarkResult>> {
    let concurrency = run.concurrency.max(1);
    let total = tests.len();
    let mut ordered = vec![None; total];
    let mut tasks = JoinSet::new();

    for (idx, test) in tests.into_iter().enumerate() {
        while tasks.len() >= concurrency {
            let (completed_idx, result) = tasks
                .join_next()
                .await
                .context("benchmark task set ended unexpectedly")?
                .context("benchmark task panicked")?;
            ordered[completed_idx] = Some(result);
        }

        let client = client.clone();
        let provider = provider.clone();
        let run = run.clone();
        let grader_config = grader_config.clone();
        let api_key = api_key.clone();
        let bench_path = bench_path.clone();
        let context_index = context_index.clone();
        let counter = counter.clone();
        tasks.spawn(async move {
            let result = run_one(
                &client,
                &provider,
                &run,
                &grader_config,
                &api_key,
                &bench_path,
                &test,
                context_index.as_ref(),
                &counter,
            )
            .await;
            (idx, result)
        });
    }

    while let Some(joined) = tasks.join_next().await {
        let (idx, result) = joined.context("benchmark task panicked")?;
        ordered[idx] = Some(result);
    }

    ordered
        .into_iter()
        .enumerate()
        .map(|(idx, result)| result.with_context(|| format!("missing result for test index {idx}")))
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn run_one(
    client: &Client,
    provider: &ProviderConfig,
    run: &RunConfig,
    grader_config: &GraderConfig,
    api_key: &str,
    bench_dir: &Path,
    test: &TestCase,
    context_index: Option<&ContextIndex>,
    counter: &TokenCounter,
) -> BenchmarkResult {
    let started = Instant::now();
    let (context_ref, routing) = match resolve_context_reference(test, context_index) {
        Ok(resolved) => resolved,
        Err(error) => {
            let (error, routing) = *error;
            return BenchmarkResult {
                schema_version: SCHEMA_VERSION,
                suite: suite_name(test),
                token_count: test_token_count(test),
                id: test.id.clone(),
                provider_model: Some(provider.model.clone()),
                provider_base_url: Some(provider.base_url.clone()),
                http_status: None,
                request_id: None,
                rate_limit_remaining: None,
                rate_limit_reset: None,
                passed: false,
                attempts: 0,
                latency_ms: started.elapsed().as_millis() as u64,
                input_tokens: 0,
                output_tokens: 0,
                answer: None,
                error: Some(error),
                error_kind: Some(ErrorKind::ContextRoute),
                routing,
                judge_latency_ms: None,
                judge_input_tokens: None,
                judge_output_tokens: None,
                judge_http_status: None,
                judge_attempts: None,
                judge_error: None,
                metadata: test.metadata.clone(),
            };
        }
    };
    let context = match load_context(bench_dir, &context_ref) {
        Ok(context) => context,
        Err(error) => {
            return BenchmarkResult {
                schema_version: SCHEMA_VERSION,
                suite: suite_name(test),
                token_count: test_token_count(test),
                id: test.id.clone(),
                provider_model: Some(provider.model.clone()),
                provider_base_url: Some(provider.base_url.clone()),
                http_status: None,
                request_id: None,
                rate_limit_remaining: None,
                rate_limit_reset: None,
                passed: false,
                attempts: 0,
                latency_ms: 0,
                input_tokens: 0,
                output_tokens: 0,
                answer: None,
                error: Some(error.to_string()),
                error_kind: Some(ErrorKind::ContextLoad),
                routing,
                judge_latency_ms: None,
                judge_input_tokens: None,
                judge_output_tokens: None,
                judge_http_status: None,
                judge_attempts: None,
                judge_error: None,
                metadata: test.metadata.clone(),
            };
        }
    };
    let prompt = build_prompt(&context, &test.question);
    let input_tokens = counter.count_tokens(&prompt);

    let response = match provider.request_style {
        crate::benchmark::ProviderRequestStyle::ChatCompletions => {
            send_chat_completion(
                client,
                run,
                provider,
                api_key,
                &prompt,
                input_tokens,
                counter,
            )
            .await
        }
        crate::benchmark::ProviderRequestStyle::Responses => {
            send_responses(
                client,
                run,
                provider,
                api_key,
                &prompt,
                input_tokens,
                counter,
            )
            .await
        }
    };

    match response {
        Ok(output) => {
            let (
                passed,
                judge_lat,
                judge_input_tok,
                judge_output_tok,
                judge_http_status,
                judge_attempts,
                judge_error,
            ) = if matches!(test.grader, crate::benchmark::Grader::LlmJudge) {
                let judge_model = grader_config
                    .judge_model
                    .as_deref()
                    .unwrap_or(&provider.model);
                let judge_result = grade_with_llm(
                    &output.answer,
                    test,
                    &context,
                    client,
                    &provider.base_url,
                    api_key,
                    judge_model,
                    run.request_timeout_secs,
                    counter,
                )
                .await;
                (
                    judge_result.passed,
                    Some(judge_result.latency_ms),
                    Some(judge_result.input_tokens),
                    Some(judge_result.output_tokens),
                    judge_result.http_status,
                    Some(judge_result.attempts),
                    judge_result.error,
                )
            } else {
                (
                    grade(&output.answer, test),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            };
            let judge_failed = judge_error.is_some();
            BenchmarkResult {
                schema_version: SCHEMA_VERSION,
                suite: suite_name(test),
                token_count: test_token_count(test),
                id: test.id.clone(),
                provider_model: Some(provider.model.clone()),
                provider_base_url: Some(provider.base_url.clone()),
                http_status: output.http_status,
                request_id: output.request_id,
                rate_limit_remaining: output.rate_limit_remaining,
                rate_limit_reset: output.rate_limit_reset,
                passed,
                attempts: output.attempts,
                latency_ms: started.elapsed().as_millis() as u64,
                input_tokens: output.input_tokens,
                output_tokens: output.output_tokens,
                answer: Some(output.answer),
                error: if passed {
                    None
                } else if let Some(error) = &judge_error {
                    Some(format!("LLM judge failed: {error}"))
                } else {
                    Some("answer did not satisfy grader".to_string())
                },
                error_kind: if passed {
                    None
                } else if judge_failed {
                    Some(ErrorKind::Judge)
                } else {
                    Some(ErrorKind::Validation)
                },
                routing,
                judge_latency_ms: judge_lat,
                judge_input_tokens: judge_input_tok,
                judge_output_tokens: judge_output_tok,
                judge_http_status,
                judge_attempts,
                judge_error,
                metadata: test.metadata.clone(),
            }
        }
        Err(failure) => error_result(ErrorResultInput {
            test,
            provider,
            started,
            input_tokens,
            attempts: failure.attempts,
            kind: failure.kind,
            http_status: failure.http_status,
            request_id: failure.request_id,
            rate_limit_remaining: failure.rate_limit_remaining,
            rate_limit_reset: failure.rate_limit_reset,
            routing,
            error: failure.error,
        }),
    }
}

async fn send_chat_completion(
    client: &Client,
    run: &RunConfig,
    provider: &ProviderConfig,
    api_key: &str,
    prompt: &str,
    input_tokens: u64,
    counter: &TokenCounter,
) -> std::result::Result<ProviderOutput, ProviderFailure> {
    let request = ChatCompletionRequest {
        model: provider.model.clone(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        temperature: 0.0,
    };
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let response = send_with_retries::<ChatCompletionRequest, ChatCompletionResponse>(
        client, run, api_key, &url, &request,
    )
    .await?;
    let answer = response
        .body
        .choices
        .first()
        .map(|choice| choice.message.content.trim().to_string())
        .unwrap_or_default();
    let output_tokens = response
        .body
        .usage
        .as_ref()
        .map_or_else(|| counter.count_tokens(&answer), |u| u.completion_tokens);
    let input_tokens = response
        .body
        .usage
        .as_ref()
        .map_or(input_tokens, |u| u.prompt_tokens);
    Ok(ProviderOutput {
        answer,
        input_tokens,
        output_tokens,
        attempts: response.attempts,
        http_status: response.http_status,
        request_id: response.request_id,
        rate_limit_remaining: response.rate_limit_remaining,
        rate_limit_reset: response.rate_limit_reset,
    })
}

async fn send_responses(
    client: &Client,
    run: &RunConfig,
    provider: &ProviderConfig,
    api_key: &str,
    prompt: &str,
    input_tokens: u64,
    counter: &TokenCounter,
) -> std::result::Result<ProviderOutput, ProviderFailure> {
    let request = ResponsesRequest {
        model: provider.model.clone(),
        input: prompt.to_string(),
        temperature: Some(0.0),
    };
    let url = format!("{}/responses", provider.base_url.trim_end_matches('/'));
    let response = send_with_retries::<ResponsesRequest, ResponsesResponse>(
        client, run, api_key, &url, &request,
    )
    .await?;
    let answer = responses_answer(&response.body);
    let input_tokens = response
        .body
        .usage
        .as_ref()
        .and_then(|usage| usage.input_tokens)
        .unwrap_or(input_tokens);
    let output_tokens = response
        .body
        .usage
        .as_ref()
        .and_then(|usage| usage.output_tokens)
        .unwrap_or_else(|| counter.count_tokens(&answer));
    Ok(ProviderOutput {
        answer,
        input_tokens,
        output_tokens,
        attempts: response.attempts,
        http_status: response.http_status,
        request_id: response.request_id,
        rate_limit_remaining: response.rate_limit_remaining,
        rate_limit_reset: response.rate_limit_reset,
    })
}

async fn send_with_retries<Request, Response>(
    client: &Client,
    run: &RunConfig,
    api_key: &str,
    url: &str,
    request: &Request,
) -> std::result::Result<ProviderSuccess<Response>, ProviderFailure>
where
    Request: Serialize + ?Sized,
    Response: DeserializeOwned,
{
    let max_attempts = run.max_retries.saturating_add(1).max(1);
    let mut attempts = 0;

    loop {
        attempts += 1;
        let response = client
            .post(url)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(run.request_timeout_secs))
            .json(request)
            .send()
            .await;

        match response {
            Ok(response) => {
                let status = response.status();
                let headers = response.headers();
                let request_id = provider_request_id(headers);
                let rate_limit_remaining = provider_rate_limit(headers, "x-ratelimit-remaining");
                let rate_limit_reset = provider_rate_limit(headers, "x-ratelimit-reset");
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    let error = anyhow!("HTTP {status} from provider: {}", body.trim());
                    if is_retryable_status(status) && attempts < max_attempts {
                        sleep(retry_delay(run, attempts)).await;
                        continue;
                    }
                    return Err(ProviderFailure {
                        error,
                        attempts,
                        kind: ErrorKind::Http,
                        http_status: Some(status.as_u16()),
                        request_id,
                        rate_limit_remaining,
                        rate_limit_reset,
                    });
                }

                match response.json::<Response>().await {
                    Ok(body) => {
                        return Ok(ProviderSuccess {
                            body,
                            attempts,
                            http_status: Some(status.as_u16()),
                            request_id,
                            rate_limit_remaining,
                            rate_limit_reset,
                        });
                    }
                    Err(error) => {
                        return Err(ProviderFailure {
                            error: anyhow!("failed to decode provider response: {error}"),
                            attempts,
                            kind: ErrorKind::ResponseDecode,
                            http_status: Some(status.as_u16()),
                            request_id,
                            rate_limit_remaining,
                            rate_limit_reset,
                        });
                    }
                }
            }
            Err(error) => {
                if is_retryable_error(&error) && attempts < max_attempts {
                    sleep(retry_delay(run, attempts)).await;
                    continue;
                }
                return Err(ProviderFailure {
                    error: error.into(),
                    attempts,
                    kind: ErrorKind::Transport,
                    http_status: None,
                    request_id: None,
                    rate_limit_remaining: None,
                    rate_limit_reset: None,
                });
            }
        }
    }
}

fn provider_request_id(headers: &reqwest::header::HeaderMap) -> Option<String> {
    for key in ["x-request-id", "x-openai-request-id", "openai-request-id"] {
        if let Some(value) = headers.get(key) {
            if let Ok(value) = value.to_str() {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn provider_rate_limit(headers: &reqwest::header::HeaderMap, key: &str) -> Option<String> {
    let candidates: &[&str] = match key {
        "x-ratelimit-remaining" => &[
            "x-ratelimit-remaining",
            "x-ratelimit-remaining-requests",
            "x-ratelimit-remaining-tokens",
        ],
        "x-ratelimit-reset" => &[
            "x-ratelimit-reset",
            "x-ratelimit-reset-requests",
            "x-ratelimit-reset-tokens",
        ],
        other => &[other],
    };

    for candidate in candidates {
        if let Some(value) = headers.get(*candidate) {
            if let Ok(value) = value.to_str() {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn retry_delay(run: &RunConfig, attempts_so_far: u32) -> Duration {
    let exponent = attempts_so_far.saturating_sub(1).min(10);
    let multiplier = 1u64 << exponent;
    Duration::from_millis(run.retry_backoff_ms.saturating_mul(multiplier))
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) || status.is_server_error()
}

fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

struct ErrorResultInput<'a, E> {
    test: &'a TestCase,
    provider: &'a ProviderConfig,
    started: Instant,
    input_tokens: u64,
    attempts: u32,
    kind: ErrorKind,
    http_status: Option<u16>,
    request_id: Option<String>,
    rate_limit_remaining: Option<String>,
    rate_limit_reset: Option<String>,
    routing: Option<RoutingDecision>,
    error: E,
}

fn error_result<E: std::fmt::Display>(input: ErrorResultInput<'_, E>) -> BenchmarkResult {
    BenchmarkResult {
        schema_version: SCHEMA_VERSION,
        suite: suite_name(input.test),
        token_count: test_token_count(input.test),
        id: input.test.id.clone(),
        provider_model: Some(input.provider.model.clone()),
        provider_base_url: Some(input.provider.base_url.clone()),
        http_status: input.http_status,
        request_id: input.request_id,
        rate_limit_remaining: input.rate_limit_remaining,
        rate_limit_reset: input.rate_limit_reset,
        passed: false,
        attempts: input.attempts,
        latency_ms: input.started.elapsed().as_millis() as u64,
        input_tokens: input.input_tokens,
        output_tokens: 0,
        answer: None,
        error: Some(input.error.to_string()),
        error_kind: Some(input.kind),
        routing: input.routing,
        judge_latency_ms: None,
        judge_input_tokens: None,
        judge_output_tokens: None,
        judge_http_status: None,
        judge_attempts: None,
        judge_error: None,
        metadata: input.test.metadata.clone(),
    }
}

type ContextResolution = (String, Option<RoutingDecision>);
type ContextResolutionError = Box<(String, Option<RoutingDecision>)>;

fn resolve_context_reference(
    test: &TestCase,
    context_index: Option<&ContextIndex>,
) -> std::result::Result<ContextResolution, ContextResolutionError> {
    if !is_auto_context(&test.context) {
        return Ok((test.context.clone(), None));
    }

    let Some(index) = context_index else {
        return Err(Box::new((
            "context routing requested but context index is unavailable".to_string(),
            None,
        )));
    };

    let decision = route_context(test, index);
    if let Some(path) = decision.selected_context_path.clone() {
        Ok((path, Some(decision)))
    } else {
        let message = format!(
            "context routing failed for {}: {}",
            test.id,
            decision
                .reason
                .as_deref()
                .unwrap_or("no context candidate selected")
        );
        Err(Box::new((message, Some(decision))))
    }
}

fn suite_name(test: &TestCase) -> Option<String> {
    test.metadata.get("suite").cloned()
}

fn test_token_count(test: &TestCase) -> Option<u64> {
    test.metadata
        .get("token_count")
        .and_then(|value| value.parse::<u64>().ok())
}

pub(crate) fn read_config(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    toml::from_str::<Config>(&text)
        .with_context(|| format!("failed to parse config {}", path.display()))
}

pub(crate) fn read_tests(bench_dir: &Path) -> Result<Vec<TestCase>> {
    let search_root = if bench_dir.join("manifests").is_dir() {
        bench_dir.join("manifests")
    } else {
        bench_dir.to_path_buf()
    };

    let mut paths = Vec::new();
    for entry in WalkDir::new(&search_root).min_depth(1) {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if is_generated_json_artifact(path) {
            continue;
        }
        paths.push(path.to_path_buf());
    }
    paths.sort();

    let mut tests = Vec::new();
    for path in paths {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read test file {}", path.display()))?;
        if let Ok(manifest) = serde_json::from_str::<SuiteManifest>(&text) {
            tests.extend(manifest.suites);
        } else {
            let test = serde_json::from_str::<TestCase>(&text)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            tests.push(test);
        }
    }
    Ok(tests)
}

fn is_generated_json_artifact(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("context.index.json" | "results.json" | "run.json")
    )
}

fn load_context(bench_dir: &Path, context: &str) -> Result<String> {
    let context_path = PathBuf::from(context);
    if context_path.is_absolute() && context_path.exists() {
        return fs::read_to_string(&context_path)
            .with_context(|| format!("failed to read context {}", context_path.display()));
    }

    let relative = bench_dir.join(&context_path);
    if relative.exists() {
        return fs::read_to_string(&relative)
            .with_context(|| format!("failed to read context {}", relative.display()));
    }

    if context_path.exists() {
        return fs::read_to_string(&context_path)
            .with_context(|| format!("failed to read context {}", context_path.display()));
    }

    Ok(context.to_string())
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Vec<ResponsesOutputItem>,
    #[serde(default)]
    usage: Option<ResponsesUsage>,
}

#[derive(Debug, Deserialize)]
struct ResponsesOutputItem {
    #[serde(default)]
    content: Vec<ResponsesContentItem>,
}

#[derive(Debug, Deserialize)]
struct ResponsesContentItem {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponsesUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[derive(Debug)]
struct ProviderSuccess<T> {
    body: T,
    attempts: u32,
    http_status: Option<u16>,
    request_id: Option<String>,
    rate_limit_remaining: Option<String>,
    rate_limit_reset: Option<String>,
}

#[derive(Debug)]
struct ProviderFailure {
    error: anyhow::Error,
    attempts: u32,
    kind: ErrorKind,
    http_status: Option<u16>,
    request_id: Option<String>,
    rate_limit_remaining: Option<String>,
    rate_limit_reset: Option<String>,
}

#[derive(Debug)]
struct ProviderOutput {
    answer: String,
    attempts: u32,
    http_status: Option<u16>,
    request_id: Option<String>,
    rate_limit_remaining: Option<String>,
    rate_limit_reset: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
}

fn responses_answer(response: &ResponsesResponse) -> String {
    if let Some(output_text) = &response.output_text {
        let trimmed = output_text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    let mut chunks = Vec::new();
    for item in &response.output {
        for content in &item.content {
            if content.kind.as_deref() == Some("output_text") {
                if let Some(text) = &content.text {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        chunks.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    chunks.join("\n")
}

#[derive(Debug, Serialize)]
struct HttpExchangeLog {
    schema_version: u32,
    logged_at_unix_ms: u64,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    suite: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    attempts: u32,
    latency_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
    passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<ErrorKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_context_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_sha256: Option<String>,
}

fn build_prompt(context: &str, question: &str) -> String {
    format!(
        "Use the context to answer the question.\n\nContext:\n{context}\n\nQuestion:\n{question}\n\nAnswer:"
    )
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::{Config, ProviderConfig, RunConfig, TestCase};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[test]
    fn config_uses_toml_parser_and_run_defaults() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        fs::write(
            &path,
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"
"#,
        )
        .unwrap();

        let config = read_config(&path).unwrap();
        assert_eq!(config.schema_version, SCHEMA_VERSION);
        assert_eq!(config.provider.model, "example-model");
        assert_eq!(config.run.request_timeout_secs, 120);
        assert_eq!(config.run.max_retries, 2);
        assert_eq!(config.run.concurrency, 1);
        assert!(!config.run.log_requests);
        assert_eq!(config.run.request_log_path, "reports/http-log.jsonl");
    }

    #[test]
    fn config_parses_response_request_style() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        fs::write(
            &path,
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"
request_style = "responses"
"#,
        )
        .unwrap();

        let config = read_config(&path).unwrap();
        assert_eq!(
            config.provider.request_style,
            crate::benchmark::ProviderRequestStyle::Responses
        );
    }

    #[test]
    fn read_tests_finds_new_manifest_layout() {
        let tmp = tempdir().unwrap();
        let manifests = tmp.path().join("manifests");
        fs::create_dir_all(&manifests).unwrap();
        fs::write(
            manifests.join("needle.json"),
            r#"
{
  "schema_version": 1,
  "name": "needle",
  "token_count": 100,
  "seed": 7,
  "suites": [{
    "schema_version": 1,
    "id": "needle-100",
    "context": "contexts/needle_context.txt",
    "question": "q",
    "expected": ["a"],
    "grader": "Exact",
    "metadata": {}
  }]
}
"#,
        )
        .unwrap();

        let tests = read_tests(tmp.path()).unwrap();
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].id, "needle-100");
    }

    #[test]
    fn read_tests_finds_flat_layout_compatibility() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("needle.json"),
            r#"
{
  "schema_version": 1,
  "id": "flat-needle",
  "context": "contexts/needle_context.txt",
  "question": "q",
  "expected": ["a"],
  "grader": "Exact",
  "metadata": {"suite": "needle", "token_count": "100000"}
}
"#,
        )
        .unwrap();

        let tests = read_tests(tmp.path()).unwrap();
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].id, "flat-needle");
        assert_eq!(
            tests[0].metadata.get("token_count"),
            Some(&"100000".to_string())
        );
    }

    #[test]
    fn read_tests_ignores_generated_root_json_artifacts_in_flat_layout() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("needle.json"),
            r#"
{
  "schema_version": 1,
  "id": "flat-needle",
  "context": "contexts/needle_context.txt",
  "question": "q",
  "expected": ["a"],
  "grader": "Exact",
  "metadata": {"suite": "needle", "token_count": "100000"}
}
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("context.index.json"),
            r#"{"schema_version":1,"bench_id":"bench","created_at_unix_ms":1,"contexts":[]}"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("run.json"),
            r#"{"schema_version":1,"bench_dir":"bench","results_path":"results.jsonl","started_at_unix_ms":1,"test_count":1,"provider_model":"m","provider_base_url":"u","provider_request_style":"chat-completions","request_timeout_secs":120,"max_retries":2,"retry_backoff_ms":500,"concurrency":1,"suites":["needle"]}"#,
        )
        .unwrap();

        let tests = read_tests(tmp.path()).unwrap();
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].id, "flat-needle");
    }

    #[test]
    fn build_run_metadata_captures_snapshot() {
        let bench_dir = tempdir().unwrap();
        let results_path = bench_dir.path().join("results.jsonl");
        let config = Config {
            schema_version: SCHEMA_VERSION,
            run: RunConfig {
                request_timeout_secs: 30,
                max_retries: 3,
                retry_backoff_ms: 250,
                concurrency: 4,
                log_requests: true,
                request_log_path: "reports/http-log.jsonl".to_string(),
            },
            provider: ProviderConfig {
                base_url: "https://api.example.test/v1".to_string(),
                api_key_env: "EXAMPLE_API_KEY".to_string(),
                model: "example-model".to_string(),
                request_style: crate::benchmark::ProviderRequestStyle::ChatCompletions,
            },
            grader: GraderConfig::default(),
        };
        let mut metadata = BTreeMap::new();
        metadata.insert("suite".to_string(), "needle".to_string());
        let tests = vec![TestCase {
            schema_version: SCHEMA_VERSION,
            id: "needle-100".to_string(),
            context: "contexts/needle_context.txt".to_string(),
            question: "q".to_string(),
            expected: vec!["a".to_string()],
            grader: crate::benchmark::Grader::Exact,
            metadata,
        }];

        let snapshot = build_run_metadata(bench_dir.path(), &results_path, 1234, &config, &tests);
        assert_eq!(snapshot.schema_version, SCHEMA_VERSION);
        assert_eq!(snapshot.test_count, 1);
        assert_eq!(snapshot.provider_model, "example-model");
        assert_eq!(snapshot.provider_request_style, "chat-completions");
        assert_eq!(snapshot.concurrency, 4);
        assert!(snapshot.log_requests);
        assert_eq!(
            snapshot.request_log_path,
            Some("reports/http-log.jsonl".to_string())
        );
        assert_eq!(snapshot.suites, vec!["needle"]);
        assert_eq!(snapshot.finished_at_unix_ms, None);
    }

    #[test]
    fn provider_headers_capture_request_and_rate_limit_metadata() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-request-id", "req_123".parse().unwrap());
        headers.insert("x-ratelimit-remaining", "42".parse().unwrap());
        headers.insert("x-ratelimit-reset", "1s".parse().unwrap());

        assert_eq!(provider_request_id(&headers), Some("req_123".to_string()));
        assert_eq!(
            provider_rate_limit(&headers, "x-ratelimit-remaining"),
            Some("42".to_string())
        );
        assert_eq!(
            provider_rate_limit(&headers, "x-ratelimit-reset"),
            Some("1s".to_string())
        );
    }

    #[test]
    fn responses_answer_prefers_output_text_and_falls_back_to_output_items() {
        let direct = ResponsesResponse {
            output_text: Some(" direct answer ".to_string()),
            output: Vec::new(),
            usage: None,
        };
        assert_eq!(responses_answer(&direct), "direct answer");

        let fallback = ResponsesResponse {
            output_text: None,
            output: vec![ResponsesOutputItem {
                content: vec![ResponsesContentItem {
                    kind: Some("output_text".to_string()),
                    text: Some(" fallback answer ".to_string()),
                }],
            }],
            usage: None,
        };
        assert_eq!(responses_answer(&fallback), "fallback answer");
    }

    #[test]
    fn request_logs_are_redacted_and_include_request_ids() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("contexts")).unwrap();
        fs::write(
            tmp.path().join("contexts/needle_context.txt"),
            "secret code orchid-123",
        )
        .unwrap();

        let mut metadata = BTreeMap::new();
        metadata.insert("suite".to_string(), "needle".to_string());
        metadata.insert("token_count".to_string(), "100000".to_string());
        let test = TestCase {
            schema_version: SCHEMA_VERSION,
            id: "needle-100000".to_string(),
            context: "contexts/needle_context.txt".to_string(),
            question: "What is the archive access code?".to_string(),
            expected: vec!["orchid-123".to_string()],
            grader: crate::benchmark::Grader::Exact,
            metadata,
        };
        let result = BenchmarkResult {
            schema_version: SCHEMA_VERSION,
            suite: Some("needle".to_string()),
            token_count: Some(100000),
            id: "needle-100000".to_string(),
            provider_model: Some("example-model".to_string()),
            provider_base_url: Some("https://api.example.test/v1".to_string()),
            http_status: Some(200),
            request_id: Some("req-abc".to_string()),
            rate_limit_remaining: None,
            rate_limit_reset: None,
            passed: true,
            attempts: 1,
            latency_ms: 9,
            input_tokens: 12,
            output_tokens: 4,
            answer: Some("orchid-123".to_string()),
            error: None,
            error_kind: None,
            routing: None,
            judge_latency_ms: None,
            judge_input_tokens: None,
            judge_output_tokens: None,
            judge_http_status: None,
            judge_attempts: None,
            judge_error: None,
            metadata: BTreeMap::new(),
        };
        let log_path = tmp.path().join("reports/http-log.jsonl");

        write_request_logs(tmp.path(), &log_path, &[test], &[result]).unwrap();
        let log = fs::read_to_string(log_path).unwrap();
        assert!(log.contains("req-abc"));
        assert!(log.contains("request_sha256"));
        assert!(log.contains("response_sha256"));
        assert!(!log.contains("archive access code"));
        assert!(!log.contains("orchid-123"));
    }

    #[test]
    fn request_logs_do_not_hash_unsent_context_route_failures() {
        let tmp = tempdir().unwrap();
        let test = TestCase {
            schema_version: SCHEMA_VERSION,
            id: "auto-1".to_string(),
            context: "auto".to_string(),
            question: "What is the code?".to_string(),
            expected: vec!["a".to_string()],
            grader: crate::benchmark::Grader::Exact,
            metadata: BTreeMap::new(),
        };
        let result = BenchmarkResult {
            schema_version: SCHEMA_VERSION,
            suite: Some("needle".to_string()),
            token_count: Some(100),
            id: "auto-1".to_string(),
            provider_model: Some("example-model".to_string()),
            provider_base_url: Some("https://api.example.test/v1".to_string()),
            http_status: None,
            request_id: None,
            rate_limit_remaining: None,
            rate_limit_reset: None,
            passed: false,
            attempts: 0,
            latency_ms: 1,
            input_tokens: 0,
            output_tokens: 0,
            answer: None,
            error: Some("context routing failed".to_string()),
            error_kind: Some(ErrorKind::ContextRoute),
            routing: Some(RoutingDecision {
                schema_version: SCHEMA_VERSION,
                test_id: "auto-1".to_string(),
                selected_context_id: None,
                selected_context_path: None,
                method: "none".to_string(),
                status: "ambiguous".to_string(),
                confidence: 0.0,
                candidates: vec![],
                llm_router_used: false,
                input_tokens: 0,
                output_tokens: 0,
                latency_ms: 0,
                reason: Some("local_router_confidence_below_threshold".to_string()),
            }),
            judge_latency_ms: None,
            judge_input_tokens: None,
            judge_output_tokens: None,
            judge_http_status: None,
            judge_attempts: None,
            judge_error: None,
            metadata: BTreeMap::new(),
        };
        let log_path = tmp.path().join("reports/http-log.jsonl");

        write_request_logs(tmp.path(), &log_path, &[test], &[result]).unwrap();
        let log = fs::read_to_string(log_path).unwrap();
        assert!(log.contains("routing_status"));
        assert!(log.contains("local_router_confidence_below_threshold"));
        assert!(!log.contains("request_sha256"));
    }

    #[test]
    fn run_output_guard_rejects_existing_results_without_force() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        fs::write(&results, "old results").unwrap();

        let error = ensure_output_paths_can_be_written(&results, None, false).unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        ensure_output_paths_can_be_written(&results, None, true).unwrap();
    }

    #[test]
    fn retry_delay_exponentiates() {
        let run = RunConfig {
            retry_backoff_ms: 100,
            ..Default::default()
        };
        assert_eq!(retry_delay(&run, 1), Duration::from_millis(100));
        assert_eq!(retry_delay(&run, 2), Duration::from_millis(200));
        assert_eq!(retry_delay(&run, 3), Duration::from_millis(400));
    }

    #[test]
    fn retry_delay_saturates_at_max() {
        let run = RunConfig {
            retry_backoff_ms: 100,
            ..Default::default()
        };
        let delay = retry_delay(&run, 20);
        assert_eq!(delay, Duration::from_millis(100 * 1024));
    }

    #[test]
    fn is_retryable_status_classifies_correctly() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::REQUEST_TIMEOUT));
        assert!(is_retryable_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_retryable_status(StatusCode::OK));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn load_context_reads_from_bench_dir() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("contexts")).unwrap();
        fs::write(tmp.path().join("contexts/test.txt"), "hello world").unwrap();
        let content = load_context(tmp.path(), "contexts/test.txt").unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn load_context_returns_inline_for_missing_file() {
        let tmp = tempdir().unwrap();
        let content = load_context(tmp.path(), "some inline text").unwrap();
        assert_eq!(content, "some inline text");
    }

    #[test]
    fn build_prompt_formats_correctly() {
        let prompt = build_prompt("some context", "some question");
        assert!(prompt.contains("some context"));
        assert!(prompt.contains("some question"));
        assert!(prompt.starts_with("Use the context"));
    }

    #[tokio::test]
    async fn wiremock_chat_completion_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "ORCHID-123"}}],
                "usage": {"prompt_tokens": 100, "completion_tokens": 5}
            })))
            .mount(&mock_server)
            .await;

        let client = Client::builder().build().unwrap();
        let provider = ProviderConfig {
            base_url: mock_server.uri(),
            api_key_env: "TEST_KEY".to_string(),
            model: "test-model".to_string(),
            request_style: crate::benchmark::ProviderRequestStyle::ChatCompletions,
        };
        let run = RunConfig::default();
        let counter = TokenCounter::cl100k();

        let result = send_chat_completion(
            &client,
            &run,
            &provider,
            "test-key",
            "test prompt",
            10,
            &counter,
        )
        .await
        .unwrap();

        assert_eq!(result.answer, "ORCHID-123");
        assert_eq!(result.input_tokens, 100);
        assert_eq!(result.output_tokens, 5);
        assert_eq!(result.attempts, 1);
        assert_eq!(result.http_status, Some(200));
    }

    #[tokio::test]
    async fn wiremock_retries_on_server_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .mount(&mock_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "ok"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 1}
            })))
            .mount(&mock_server)
            .await;

        let client = Client::builder().build().unwrap();
        let provider = ProviderConfig {
            base_url: mock_server.uri(),
            api_key_env: "TEST_KEY".to_string(),
            model: "test-model".to_string(),
            request_style: crate::benchmark::ProviderRequestStyle::ChatCompletions,
        };
        let run = RunConfig {
            max_retries: 3,
            retry_backoff_ms: 1,
            ..Default::default()
        };
        let counter = TokenCounter::cl100k();

        let result = send_chat_completion(
            &client,
            &run,
            &provider,
            "test-key",
            "test prompt",
            10,
            &counter,
        )
        .await
        .unwrap();

        assert_eq!(result.answer, "ok");
        assert_eq!(result.attempts, 3);
    }

    #[tokio::test]
    async fn run_records_llm_judge_failures_as_judge_errors() {
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_string_contains("Use the context to answer"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "candidate answer"}}],
                "usage": {"prompt_tokens": 20, "completion_tokens": 2}
            })))
            .mount(&mock_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_string_contains("You are grading an answer"))
            .respond_with(ResponseTemplate::new(500).set_body_string("judge unavailable"))
            .mount(&mock_server)
            .await;

        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("contexts")).unwrap();
        fs::create_dir_all(tmp.path().join("manifests")).unwrap();
        fs::write(tmp.path().join("contexts/test.txt"), "context").unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            format!(
                r#"
[provider]
base_url = "{}"
api_key_env = "LONGCTX_JUDGE_TEST_KEY"
model = "test-model"
"#,
                mock_server.uri()
            ),
        )
        .unwrap();
        fs::write(
            tmp.path().join("manifests/judge.json"),
            r#"
{
  "schema_version": 1,
  "name": "judge",
  "token_count": 100,
  "seed": 7,
  "suites": [{
    "schema_version": 1,
    "id": "judge-1",
    "context": "contexts/test.txt",
    "question": "q",
    "expected": ["a"],
    "grader": "LlmJudge",
    "metadata": {"suite": "judge", "token_count": "100"}
  }]
}
"#,
        )
        .unwrap();
        std::env::set_var("LONGCTX_JUDGE_TEST_KEY", "test-key");

        run_benchmarks_with_options(tmp.path().to_str().unwrap(), RunOptions { force: false })
            .await
            .unwrap();

        let result_line = fs::read_to_string(tmp.path().join("results.jsonl")).unwrap();
        let result: BenchmarkResult = serde_json::from_str(&result_line).unwrap();
        assert_eq!(result.error_kind, Some(ErrorKind::Judge));
        assert_eq!(result.judge_http_status, Some(500));
        assert_eq!(result.judge_attempts, Some(1));
        assert!(result
            .judge_error
            .as_deref()
            .unwrap_or_default()
            .contains("judge unavailable"));
        assert!(result.error.unwrap().contains("LLM judge failed"));
    }

    #[tokio::test]
    async fn run_preflight_rejects_stale_context_index_before_provider_request() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = "http://127.0.0.1:9/v1"
api_key_env = "LONGCTX_PREFLIGHT_KEY"
model = "test-model"
"#,
        )
        .unwrap();
        fs::create_dir_all(tmp.path().join("contexts")).unwrap();
        fs::create_dir_all(tmp.path().join("manifests")).unwrap();
        fs::write(tmp.path().join("contexts/needle_context.txt"), "changed").unwrap();
        fs::write(
            tmp.path().join("manifests/needle.json"),
            r#"
{
  "schema_version": 1,
  "name": "needle",
  "token_count": 100,
  "seed": 7,
  "suites": [{
    "schema_version": 1,
    "id": "needle-auto",
    "context": "auto",
    "question": "q",
    "expected": ["a"],
    "grader": "Exact",
    "metadata": {"suite": "needle", "token_count": "100"}
  }]
}
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("context.index.json"),
            r#"{
  "schema_version": 1,
  "bench_id": "bench",
  "created_at_unix_ms": 1,
  "contexts": [{
    "context_id": "needle_context",
    "path": "contexts/needle_context.txt",
    "sha256": "not-the-current-hash",
    "suite": "needle",
    "token_count": 100,
    "seed": 7,
    "source_manifests": ["manifests/needle.json"],
    "test_ids": ["needle-auto"],
    "title": "needle context",
    "summary": "needle",
    "keywords": ["needle"],
    "safe_anchors": [],
    "metadata": {}
  }]
}
"#,
        )
        .unwrap();
        std::env::set_var("LONGCTX_PREFLIGHT_KEY", "test-key");

        let error =
            run_benchmarks_with_options(tmp.path().to_str().unwrap(), RunOptions { force: false })
                .await
                .unwrap_err();
        assert!(error.to_string().contains("hash mismatch"));
        assert!(!tmp.path().join("results.jsonl").exists());
    }
}
