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
    LlmJudge,
    ExpectRefusal,
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
    #[serde(default)]
    pub judge_latency_ms: Option<u64>,
    #[serde(default)]
    pub judge_input_tokens: Option<u64>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub run: RunConfig,
    pub provider: ProviderConfig,
    #[serde(default)]
    pub grader: GraderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraderConfig {
    #[serde(default)]
    pub judge_model: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_result_serde_round_trip() {
        let result = BenchmarkResult {
            schema_version: SCHEMA_VERSION,
            suite: Some("needle".to_string()),
            token_count: Some(100000),
            id: "test-1".to_string(),
            provider_model: Some("gpt-4.1".to_string()),
            provider_base_url: Some("https://api.openai.com/v1".to_string()),
            http_status: Some(200),
            request_id: Some("req-abc".to_string()),
            rate_limit_remaining: Some("9".to_string()),
            rate_limit_reset: Some("60".to_string()),
            passed: true,
            attempts: 1,
            latency_ms: 150,
            input_tokens: 1000,
            output_tokens: 50,
            answer: Some("ORCHID-123".to_string()),
            error: None,
            error_kind: None,
            judge_latency_ms: None,
            judge_input_tokens: None,
            metadata: BTreeMap::new(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: BenchmarkResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.id, parsed.id);
        assert_eq!(result.passed, parsed.passed);
        assert_eq!(result.latency_ms, parsed.latency_ms);
        assert_eq!(result.input_tokens, parsed.input_tokens);
        assert_eq!(result.answer, parsed.answer);
    }

    #[test]
    fn suite_manifest_serde_round_trip() {
        let manifest = SuiteManifest {
            schema_version: SCHEMA_VERSION,
            name: "needle".to_string(),
            token_count: 100000,
            seed: 42,
            suites: vec![TestCase {
                schema_version: SCHEMA_VERSION,
                id: "needle-100000".to_string(),
                context: "contexts/needle_context.txt".to_string(),
                question: "What is the code?".to_string(),
                expected: vec!["ORCHID-123".to_string()],
                grader: Grader::Exact,
                metadata: BTreeMap::new(),
            }],
        };
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: SuiteManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest.name, parsed.name);
        assert_eq!(manifest.seed, parsed.seed);
        assert_eq!(manifest.suites.len(), parsed.suites.len());
        assert_eq!(manifest.suites[0].expected, parsed.suites[0].expected);
    }

    #[test]
    fn provider_request_style_serde_round_trip() {
        assert_eq!(
            serde_json::from_str::<ProviderRequestStyle>("\"chat-completions\"").unwrap(),
            ProviderRequestStyle::ChatCompletions,
        );
        assert_eq!(
            serde_json::from_str::<ProviderRequestStyle>("\"responses\"").unwrap(),
            ProviderRequestStyle::Responses,
        );
        assert_eq!(
            serde_json::from_str::<ProviderRequestStyle>("\"chat\"").unwrap(),
            ProviderRequestStyle::ChatCompletions,
        );
    }

    #[test]
    fn error_kind_serde_round_trip() {
        let kinds = vec![
            ErrorKind::ContextLoad,
            ErrorKind::Transport,
            ErrorKind::Http,
            ErrorKind::ResponseDecode,
            ErrorKind::Validation,
        ];
        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let parsed: ErrorKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, parsed);
        }
    }

    #[test]
    fn grader_llm_judge_serde_round_trip() {
        let json = serde_json::to_string(&Grader::LlmJudge).unwrap();
        let parsed: Grader = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Grader::LlmJudge));
    }

    #[test]
    fn grader_expect_refusal_serde_round_trip() {
        let json = serde_json::to_string(&Grader::ExpectRefusal).unwrap();
        let parsed: Grader = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Grader::ExpectRefusal));
    }

    #[test]
    fn grader_config_serde_round_trip() {
        let config = GraderConfig {
            judge_model: Some("gpt-4.1-mini".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: GraderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.judge_model, Some("gpt-4.1-mini".to_string()));
    }

    #[test]
    fn grader_config_default_has_no_judge_model() {
        let config = GraderConfig::default();
        assert!(config.judge_model.is_none());
    }
}
