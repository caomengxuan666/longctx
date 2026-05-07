use crate::benchmark::{BenchmarkResult, Config, ProviderConfig, SuiteManifest, TestCase};
use crate::grader::grade;
use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use walkdir::WalkDir;

pub async fn run_benchmarks(bench_dir: &str) -> Result<()> {
    let bench_path = Path::new(bench_dir);
    let config = read_config(&bench_path.join("config.toml"))?;
    let api_key = env::var(&config.provider.api_key_env).with_context(|| {
        format!(
            "environment variable {} is not set",
            config.provider.api_key_env
        )
    })?;

    let tests = read_tests(bench_path)?;
    if tests.is_empty() {
        return Err(anyhow!("no benchmark .json files found in {bench_dir}"));
    }

    let results_path = bench_path.join("results.jsonl");
    let mut results_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&results_path)
        .with_context(|| format!("failed to open {}", results_path.display()))?;

    let client = Client::new();
    for test in tests {
        let result = run_one(&client, &config.provider, &api_key, bench_path, &test).await;
        writeln!(results_file, "{}", serde_json::to_string(&result)?)?;
    }

    Ok(())
}

async fn run_one(
    client: &Client,
    provider: &ProviderConfig,
    api_key: &str,
    bench_dir: &Path,
    test: &TestCase,
) -> BenchmarkResult {
    let started = Instant::now();
    let context = match load_context(bench_dir, &test.context) {
        Ok(context) => context,
        Err(error) => {
            return BenchmarkResult {
                id: test.id.clone(),
                passed: false,
                latency_ms: 0,
                input_tokens: 0,
                output_tokens: 0,
                answer: None,
                error: Some(error.to_string()),
            };
        }
    };
    let prompt = format!(
        "Use the context to answer the question.\n\nContext:\n{context}\n\nQuestion:\n{}\n\nAnswer:",
        test.question
    );
    let input_tokens = estimate_tokens(&prompt);

    let request = ChatCompletionRequest {
        model: provider.model.clone(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }],
        temperature: 0.0,
    };

    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    match client
        .post(url)
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await
    {
        Ok(response) => match response.error_for_status() {
            Ok(response) => match response.json::<ChatCompletionResponse>().await {
                Ok(body) => {
                    let answer = body
                        .choices
                        .first()
                        .map(|choice| choice.message.content.trim().to_string())
                        .unwrap_or_default();
                    BenchmarkResult {
                        id: test.id.clone(),
                        passed: grade(&answer, test),
                        latency_ms: started.elapsed().as_millis() as u64,
                        input_tokens: body.usage.as_ref().map_or(input_tokens, |u| u.prompt_tokens),
                        output_tokens: body
                            .usage
                            .as_ref()
                            .map_or_else(|| estimate_tokens(&answer), |u| u.completion_tokens),
                        answer: Some(answer),
                        error: None,
                    }
                }
                Err(error) => error_result(test, started, input_tokens, error),
            },
            Err(error) => error_result(test, started, input_tokens, error),
        },
        Err(error) => error_result(test, started, input_tokens, error),
    }
}

fn error_result<E: std::fmt::Display>(
    test: &TestCase,
    started: Instant,
    input_tokens: u64,
    error: E,
) -> BenchmarkResult {
    BenchmarkResult {
        id: test.id.clone(),
        passed: false,
        latency_ms: started.elapsed().as_millis() as u64,
        input_tokens,
        output_tokens: 0,
        answer: None,
        error: Some(error.to_string()),
    }
}

fn read_config(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let mut in_provider = false;
    let mut base_url = None;
    let mut api_key_env = None;
    let mut model = None;

    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_provider = line == "[provider]";
            continue;
        }
        if !in_provider {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "base_url" => base_url = Some(value),
            "api_key_env" => api_key_env = Some(value),
            "model" => model = Some(value),
            _ => {}
        }
    }

    Ok(Config {
        provider: ProviderConfig {
            base_url: base_url.context("missing provider.base_url in config.toml")?,
            api_key_env: api_key_env.context("missing provider.api_key_env in config.toml")?,
            model: model.context("missing provider.model in config.toml")?,
        },
    })
}

fn read_tests(bench_dir: &Path) -> Result<Vec<TestCase>> {
    let mut tests = Vec::new();
    for entry in WalkDir::new(bench_dir).min_depth(1).max_depth(1) {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("results.json") {
            continue;
        }
        let text = fs::read_to_string(path)
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

fn load_context(bench_dir: &Path, context: &str) -> Result<String> {
    let context_path = PathBuf::from(context);
    if context_path.exists() {
        return fs::read_to_string(&context_path)
            .with_context(|| format!("failed to read context {}", context_path.display()));
    }
    let relative = bench_dir.join(context);
    if relative.exists() {
        return fs::read_to_string(&relative)
            .with_context(|| format!("failed to read context {}", relative.display()));
    }
    Ok(context.to_string())
}

fn estimate_tokens(text: &str) -> u64 {
    text.split_whitespace().count() as u64
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
