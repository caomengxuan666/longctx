use crate::benchmark::{BenchmarkResult, Config};
use crate::generator;
use crate::report::{self, ReportSummary};
use crate::runner::{self, RunOptions};
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CAPABILITY_SUITES: &[&str] = &[
    "multi-needle",
    "conflict",
    "multi-hop",
    "order-dependent",
    "position-sweep",
    "hallucination",
];

#[derive(Debug, Clone)]
pub struct ProbeOptions {
    pub min_tokens: u64,
    pub max_tokens: u64,
    pub resolution_tokens: u64,
    pub seed: u64,
    pub config_path: Option<PathBuf>,
    pub capability_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeSummary {
    pub run_dir: String,
    pub provider_model: String,
    pub provider_base_url: String,
    pub request_style: String,
    pub min_tokens: u64,
    pub max_tokens: u64,
    pub resolution_tokens: u64,
    pub seed: u64,
    pub best_success: Option<ProbeAttemptSummary>,
    pub first_failure: Option<ProbeAttemptSummary>,
    pub attempts: Vec<ProbeAttemptSummary>,
    pub capabilities: Option<CapabilityProbeSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeAttemptSummary {
    pub token_count: u64,
    pub bench_dir: String,
    pub passed: bool,
    pub http_status: Option<u16>,
    pub error_kind: Option<String>,
    pub attempts: u32,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub answer: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityProbeSummary {
    pub token_count: u64,
    pub bench_dir: String,
    pub report_path: String,
    pub summary: ReportSummary,
}

impl ProbeSummary {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("probe run: {}\n", self.run_dir));
        out.push_str(&format!(
            "provider: {} ({}, {})\n",
            self.provider_model, self.provider_base_url, self.request_style
        ));
        match (&self.best_success, &self.first_failure) {
            (Some(success), Some(failure)) => {
                out.push_str(&format!(
                    "max passing token_count: {} (input_tokens: {})\n",
                    success.token_count, success.input_tokens
                ));
                out.push_str(&format!(
                    "first failing token_count: {} (status: {}, error_kind: {})\n",
                    failure.token_count,
                    display_opt(failure.http_status),
                    failure.error_kind.as_deref().unwrap_or("")
                ));
            }
            (Some(success), None) => {
                out.push_str(&format!(
                    "all probes passed through token_count: {} (input_tokens: {})\n",
                    success.token_count, success.input_tokens
                ));
            }
            (None, Some(failure)) => {
                out.push_str(&format!(
                    "minimum token_count failed: {} (status: {}, error_kind: {})\n",
                    failure.token_count,
                    display_opt(failure.http_status),
                    failure.error_kind.as_deref().unwrap_or("")
                ));
            }
            (None, None) => out.push_str("no probe attempts were run\n"),
        }
        out.push_str("attempts:\n");
        for attempt in &self.attempts {
            let status = if attempt.passed { "pass" } else { "fail" };
            out.push_str(&format!(
                "  {status:4} token_count={} input_tokens={} http_status={} latency_ms={} error_kind={}\n",
                attempt.token_count,
                attempt.input_tokens,
                display_opt(attempt.http_status),
                attempt.latency_ms,
                attempt.error_kind.as_deref().unwrap_or("")
            ));
        }
        if let Some(capabilities) = &self.capabilities {
            out.push_str(&format!(
                "capabilities @ {} tokens: {}/{} passed ({:.1}%), report: {}\n",
                capabilities.token_count,
                capabilities.summary.passed,
                capabilities.summary.total,
                capabilities.summary.pass_rate,
                capabilities.report_path
            ));
        }
        out
    }
}

pub async fn probe_context(out_dir: &str, options: ProbeOptions) -> Result<ProbeSummary> {
    validate_probe_options(&options)?;
    let out_dir = Path::new(out_dir);
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create probe directory {}", out_dir.display()))?;
    let config_path = resolve_config_path(out_dir, options.config_path.as_deref())?;
    let config = read_config(&config_path)?;
    let run_dir = next_probe_run_dir(out_dir)?;
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create probe run directory {}", run_dir.display()))?;

    let mut attempts = Vec::new();
    let mut best_success: Option<ProbeAttemptSummary> = None;
    let mut first_failure: Option<ProbeAttemptSummary> = None;
    let mut token_count = options.min_tokens;

    loop {
        let attempt = run_needle_attempt(&run_dir, &config_path, token_count, options.seed).await?;
        let passed = attempt.passed;
        attempts.push(attempt.clone());
        if passed {
            best_success = Some(attempt);
            if token_count >= options.max_tokens {
                break;
            }
            token_count = token_count.saturating_mul(2).min(options.max_tokens);
        } else {
            first_failure = Some(attempt);
            break;
        }
    }

    while let (Some(success), Some(failure)) = (&best_success, &first_failure) {
        let Some(next) = midpoint_token(
            success.token_count,
            failure.token_count,
            options.resolution_tokens,
        ) else {
            break;
        };
        let attempt = run_needle_attempt(&run_dir, &config_path, next, options.seed).await?;
        attempts.push(attempt.clone());
        if attempt.passed {
            best_success = Some(attempt);
        } else {
            first_failure = Some(attempt);
        }
    }

    attempts.sort_by_key(|attempt| attempt.token_count);
    let capabilities = if let Some(tokens) = options.capability_tokens {
        Some(run_capability_probe(&run_dir, &config_path, tokens, options.seed).await?)
    } else {
        None
    };

    let summary = ProbeSummary {
        run_dir: run_dir.to_string_lossy().into_owned(),
        provider_model: config.provider.model,
        provider_base_url: config.provider.base_url,
        request_style: config.provider.request_style.as_str().to_string(),
        min_tokens: options.min_tokens,
        max_tokens: options.max_tokens,
        resolution_tokens: options.resolution_tokens,
        seed: options.seed,
        best_success,
        first_failure,
        attempts,
        capabilities,
    };
    let summary_path = run_dir.join("probe-summary.json");
    fs::write(&summary_path, summary.to_json()?)
        .with_context(|| format!("failed to write {}", summary_path.display()))?;
    Ok(summary)
}

async fn run_needle_attempt(
    run_dir: &Path,
    config_path: &Path,
    token_count: u64,
    seed: u64,
) -> Result<ProbeAttemptSummary> {
    let bench_dir = run_dir
        .join("attempts")
        .join(format!("needle-{token_count}"));
    fs::create_dir_all(&bench_dir)
        .with_context(|| format!("failed to create {}", bench_dir.display()))?;
    generator::generate("needle", token_count, Some(seed), &path_string(&bench_dir))?;
    copy_config(config_path, &bench_dir)?;
    run_probe_benchmark(&bench_dir).await?;
    let result = read_single_result(&bench_dir.join("results.jsonl"))?;
    Ok(result_to_attempt(token_count, &bench_dir, result))
}

async fn run_capability_probe(
    run_dir: &Path,
    config_path: &Path,
    token_count: u64,
    seed: u64,
) -> Result<CapabilityProbeSummary> {
    let bench_dir = run_dir.join("capabilities").join(format!("{token_count}"));
    fs::create_dir_all(&bench_dir)
        .with_context(|| format!("failed to create {}", bench_dir.display()))?;
    for suite in CAPABILITY_SUITES {
        generator::generate(suite, token_count, Some(seed), &path_string(&bench_dir))?;
    }
    copy_config(config_path, &bench_dir)?;
    run_probe_benchmark(&bench_dir).await?;
    let results_path = bench_dir.join("results.jsonl");
    let report_path = bench_dir.join("reports").join("report.html");
    report::generate_html(&path_string(&results_path), &path_string(&report_path))?;
    let summary = report::generate_summary(&path_string(&results_path))?;
    Ok(CapabilityProbeSummary {
        token_count,
        bench_dir: path_string(&bench_dir),
        report_path: path_string(&report_path),
        summary,
    })
}

async fn run_probe_benchmark(bench_dir: &Path) -> Result<()> {
    runner::run_benchmarks_with_options(
        &path_string(bench_dir),
        RunOptions {
            force: false,
            dry_run: false,
            filter: None,
            limit: None,
        },
    )
    .await
}

fn read_single_result(results_path: &Path) -> Result<BenchmarkResult> {
    let text = fs::read_to_string(results_path)
        .with_context(|| format!("failed to read {}", results_path.display()))?;
    let line = text
        .lines()
        .next()
        .with_context(|| format!("{} did not contain a result row", results_path.display()))?;
    serde_json::from_str(line)
        .with_context(|| format!("failed to parse result row in {}", results_path.display()))
}

fn result_to_attempt(
    token_count: u64,
    bench_dir: &Path,
    result: BenchmarkResult,
) -> ProbeAttemptSummary {
    ProbeAttemptSummary {
        token_count,
        bench_dir: path_string(bench_dir),
        passed: result.passed,
        http_status: result.http_status,
        error_kind: result.error_kind.map(|kind| format!("{kind:?}")),
        attempts: result.attempts,
        latency_ms: result.latency_ms,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
        answer: result.answer,
        error: result.error,
    }
}

fn validate_probe_options(options: &ProbeOptions) -> Result<()> {
    if options.min_tokens == 0 {
        anyhow::bail!("--min-tokens must be greater than 0");
    }
    if options.max_tokens < options.min_tokens {
        anyhow::bail!("--max-tokens must be greater than or equal to --min-tokens");
    }
    if options.resolution_tokens == 0 {
        anyhow::bail!("--resolution-tokens must be greater than 0");
    }
    if matches!(options.capability_tokens, Some(0)) {
        anyhow::bail!("--capability-tokens must be greater than 0");
    }
    Ok(())
}

fn resolve_config_path(out_dir: &Path, config_path: Option<&Path>) -> Result<PathBuf> {
    let path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| out_dir.join("config.toml"));
    if !path.exists() {
        anyhow::bail!(
            "config file {} does not exist; pass --config or place config.toml in the probe directory",
            path.display()
        );
    }
    Ok(path)
}

fn read_config(config_path: &Path) -> Result<Config> {
    let text = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", config_path.display()))
}

fn copy_config(config_path: &Path, bench_dir: &Path) -> Result<()> {
    fs::copy(config_path, bench_dir.join("config.toml"))
        .with_context(|| format!("failed to copy config from {}", config_path.display()))?;
    Ok(())
}

fn next_probe_run_dir(out_dir: &Path) -> Result<PathBuf> {
    let runs_dir = out_dir.join("probe-runs");
    fs::create_dir_all(&runs_dir)
        .with_context(|| format!("failed to create {}", runs_dir.display()))?;
    let base = unix_ms_now();
    for offset in 0..1000 {
        let candidate = runs_dir.join(format!("{}", base + offset));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("failed to allocate a unique probe run directory")
}

fn midpoint_token(low_success: u64, high_failure: u64, resolution: u64) -> Option<u64> {
    if high_failure <= low_success || high_failure - low_success <= resolution {
        return None;
    }
    let mut mid = low_success + (high_failure - low_success) / 2;
    if resolution > 1 {
        mid = (mid / resolution) * resolution;
    }
    if mid <= low_success {
        mid = low_success.saturating_add(resolution);
    }
    if mid >= high_failure {
        mid = high_failure.saturating_sub(resolution);
    }
    (mid > low_success && mid < high_failure).then_some(mid)
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn display_opt<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_respects_resolution() {
        assert_eq!(midpoint_token(256_000, 512_000, 8_000), Some(384_000));
        assert_eq!(midpoint_token(256_000, 272_000, 8_000), Some(264_000));
        assert_eq!(midpoint_token(264_000, 272_000, 8_000), None);
    }

    #[test]
    fn invalid_probe_options_are_rejected() {
        let options = ProbeOptions {
            min_tokens: 0,
            max_tokens: 1,
            resolution_tokens: 1,
            seed: 42,
            config_path: None,
            capability_tokens: None,
        };
        assert!(validate_probe_options(&options).is_err());
    }
}
