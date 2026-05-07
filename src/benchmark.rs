use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub context: String,
    pub question: String,
    pub expected: Vec<String>,
    pub grader: Grader,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Grader {
    Exact,
    Regex(String),
    Json,
    JsonFields,
    Set,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorKind {
    ContextLoad,
    Transport,
    Http,
    ResponseDecode,
    Validation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub suite: Option<String>,
    #[serde(default)]
    pub token_count: Option<u64>,
    pub id: String,
    #[serde(default)]
    pub provider_model: Option<String>,
    #[serde(default)]
    pub provider_base_url: Option<String>,
    #[serde(default)]
    pub http_status: Option<u16>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub rate_limit_remaining: Option<String>,
    #[serde(default)]
    pub rate_limit_reset: Option<String>,
    pub passed: bool,
    #[serde(default)]
    pub attempts: u32,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub answer: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub error_kind: Option<ErrorKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub run: RunConfig,
    pub provider: ProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub bench_dir: String,
    pub results_path: String,
    pub started_at_unix_ms: u64,
    #[serde(default)]
    pub finished_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    pub test_count: usize,
    pub provider_model: String,
    pub provider_base_url: String,
    pub provider_request_style: String,
    pub request_timeout_secs: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub concurrency: usize,
    #[serde(default)]
    pub log_requests: bool,
    #[serde(default)]
    pub request_log_path: Option<String>,
    #[serde(default)]
    pub suites: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub log_requests: bool,
    #[serde(default = "default_request_log_path")]
    pub request_log_path: String,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            request_timeout_secs: default_request_timeout_secs(),
            max_retries: default_max_retries(),
            retry_backoff_ms: default_retry_backoff_ms(),
            concurrency: default_concurrency(),
            log_requests: false,
            request_log_path: default_request_log_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key_env: String,
    pub model: String,
    #[serde(default = "default_provider_request_style")]
    pub request_style: ProviderRequestStyle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderRequestStyle {
    #[serde(
        rename = "chat-completions",
        alias = "chat_completions",
        alias = "chat"
    )]
    ChatCompletions,
    #[serde(rename = "responses", alias = "response")]
    Responses,
}

impl ProviderRequestStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat-completions",
            Self::Responses => "responses",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub name: String,
    pub token_count: u64,
    #[serde(default)]
    pub seed: u64,
    pub suites: Vec<TestCase>,
}

pub fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

fn default_request_timeout_secs() -> u64 {
    120
}

fn default_max_retries() -> u32 {
    2
}

fn default_retry_backoff_ms() -> u64 {
    500
}

fn default_concurrency() -> usize {
    1
}

fn default_request_log_path() -> String {
    "reports/http-log.jsonl".to_string()
}

fn default_provider_request_style() -> ProviderRequestStyle {
    ProviderRequestStyle::ChatCompletions
}
