use crate::probe::{self, ProbeOptions, ProbeSummary};
use anyhow::{Context, Result};
use askama::Template;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScoreProfile {
    Quick,
    Standard,
    Deep,
    MaxContext,
}

impl ScoreProfile {
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "quick" => Ok(Self::Quick),
            "standard" => Ok(Self::Standard),
            "deep" => Ok(Self::Deep),
            "max-context" | "max_context" | "maxcontext" => Ok(Self::MaxContext),
            other => anyhow::bail!(
                "unknown score profile '{other}', expected quick, standard, deep, or max-context"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Deep => "deep",
            Self::MaxContext => "max-context",
        }
    }

    fn defaults(self) -> ProfileDefaults {
        match self {
            Self::Quick => ProfileDefaults {
                min_tokens: 8_000,
                max_tokens: 64_000,
                resolution_tokens: 8_000,
                capability_tokens: Some(8_000),
            },
            Self::Standard => ProfileDefaults {
                min_tokens: 8_000,
                max_tokens: 1_000_000,
                resolution_tokens: 8_000,
                capability_tokens: Some(32_000),
            },
            Self::Deep => ProfileDefaults {
                min_tokens: 8_000,
                max_tokens: 1_000_000,
                resolution_tokens: 4_000,
                capability_tokens: Some(128_000),
            },
            Self::MaxContext => ProfileDefaults {
                min_tokens: 8_000,
                max_tokens: 1_000_000,
                resolution_tokens: 8_000,
                capability_tokens: None,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScoreOptions {
    pub profile: ScoreProfile,
    pub config_path: Option<PathBuf>,
    pub min_tokens: Option<u64>,
    pub max_tokens: Option<u64>,
    pub resolution_tokens: Option<u64>,
    pub seed: u64,
    pub capability_tokens: Option<u64>,
    pub skip_capabilities: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreSummary {
    pub schema_version: u32,
    pub run_dir: String,
    pub profile: ScoreProfile,
    pub provider_model: String,
    pub provider_base_url: String,
    pub request_style: String,
    pub score: f64,
    pub score_fmt: String,
    pub max_context_tokens: Option<u64>,
    pub max_context_input_tokens: Option<u64>,
    pub first_failure_tokens: Option<u64>,
    pub first_failure_http_status: Option<u16>,
    pub first_failure_error_kind: Option<String>,
    pub categories: Vec<ScoreCategory>,
    pub artifacts: ScoreArtifacts,
    pub probe: ProbeSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreCategory {
    pub name: String,
    pub score: f64,
    pub score_fmt: String,
    pub weight: f64,
    pub passed: usize,
    pub total: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreArtifacts {
    pub score_json: String,
    pub score_html: String,
    pub probe_summary_json: String,
    pub capability_report_html: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct ProfileDefaults {
    min_tokens: u64,
    max_tokens: u64,
    resolution_tokens: u64,
    capability_tokens: Option<u64>,
}

#[derive(Template)]
#[template(path = "score.html")]
struct ScoreView<'a> {
    summary: &'a ScoreSummary,
    max_context_display: String,
    first_failure_display: String,
}

pub async fn score_provider(out_dir: &str, options: ScoreOptions) -> Result<ScoreSummary> {
    let defaults = options.profile.defaults();
    let min_tokens = options.min_tokens.unwrap_or(defaults.min_tokens);
    let max_tokens = options.max_tokens.unwrap_or(defaults.max_tokens);
    let resolution_tokens = options
        .resolution_tokens
        .unwrap_or(defaults.resolution_tokens);
    let capability_tokens = if options.skip_capabilities {
        None
    } else {
        options.capability_tokens.or(defaults.capability_tokens)
    };
    let probe = probe::probe_context(
        out_dir,
        ProbeOptions {
            min_tokens,
            max_tokens,
            resolution_tokens,
            seed: options.seed,
            config_path: options.config_path,
            capability_tokens,
        },
    )
    .await?;
    let run_dir = PathBuf::from(&probe.run_dir);
    let score_json = run_dir.join("score.json");
    let score_html = run_dir.join("score.html");
    let mut summary = build_score_summary(options.profile, max_tokens, &run_dir, probe);
    summary.artifacts.score_json = path_string(&score_json);
    summary.artifacts.score_html = path_string(&score_html);
    fs::write(&score_json, summary.to_json()?)
        .with_context(|| format!("failed to write {}", score_json.display()))?;
    write_score_html(&summary, &score_html)?;
    Ok(summary)
}

impl ScoreSummary {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("LongCtx Score: {} / 100\n", self.score_fmt));
        out.push_str(&format!(
            "Model: {} ({}, {})\n",
            self.provider_model, self.provider_base_url, self.request_style
        ));
        out.push_str(&format!("Profile: {}\n", self.profile.as_str()));
        match self.max_context_tokens {
            Some(tokens) => out.push_str(&format!(
                "Max context: {} tokens (input_tokens: {})\n",
                tokens,
                self.max_context_input_tokens.unwrap_or(0)
            )),
            None => out.push_str("Max context: no passing context probe\n"),
        }
        if let Some(tokens) = self.first_failure_tokens {
            out.push_str(&format!(
                "First failure: {} tokens (status: {}, error_kind: {})\n",
                tokens,
                display_opt(self.first_failure_http_status),
                self.first_failure_error_kind.as_deref().unwrap_or("")
            ));
        }
        out.push_str("Categories:\n");
        for category in &self.categories {
            out.push_str(&format!(
                "  {:26} {:>6} / 100  {} ({}/{})\n",
                category.name, category.score_fmt, category.detail, category.passed, category.total
            ));
        }
        out.push_str(&format!("HTML report: {}\n", self.artifacts.score_html));
        out.push_str(&format!("JSON report: {}\n", self.artifacts.score_json));
        if let Some(report) = &self.artifacts.capability_report_html {
            out.push_str(&format!("Capability report: {report}\n"));
        }
        out
    }
}

fn build_score_summary(
    profile: ScoreProfile,
    max_tokens: u64,
    run_dir: &Path,
    probe: ProbeSummary,
) -> ScoreSummary {
    let max_context_tokens = probe
        .best_success
        .as_ref()
        .map(|attempt| attempt.token_count);
    let max_context_input_tokens = probe
        .best_success
        .as_ref()
        .map(|attempt| attempt.input_tokens);
    let first_failure_tokens = probe
        .first_failure
        .as_ref()
        .map(|attempt| attempt.token_count);
    let first_failure_http_status = probe
        .first_failure
        .as_ref()
        .and_then(|attempt| attempt.http_status);
    let first_failure_error_kind = probe
        .first_failure
        .as_ref()
        .and_then(|attempt| attempt.error_kind.clone());
    let mut categories = Vec::new();
    categories.push(context_category(
        max_context_tokens,
        max_tokens,
        probe.best_success.is_some(),
    ));
    if let Some(capabilities) = &probe.capabilities {
        categories.push(suite_category(
            "Retrieval",
            0.20,
            &capabilities.summary,
            &["multi-needle", "position-sweep"],
        ));
        categories.push(suite_category(
            "Reasoning and Conflict",
            0.20,
            &capabilities.summary,
            &["conflict", "multi-hop", "order-dependent"],
        ));
        categories.push(suite_category(
            "Hallucination Resistance",
            0.20,
            &capabilities.summary,
            &["hallucination"],
        ));
        categories.push(latency_category(capabilities.summary.avg_latency_ms));
    }
    let weight_sum: f64 = categories.iter().map(|category| category.weight).sum();
    let score = if weight_sum == 0.0 {
        0.0
    } else {
        categories
            .iter()
            .map(|category| category.score * category.weight)
            .sum::<f64>()
            / weight_sum
    };
    let capability_report_html = probe
        .capabilities
        .as_ref()
        .map(|capabilities| capabilities.report_path.clone());
    ScoreSummary {
        schema_version: 1,
        run_dir: path_string(run_dir),
        profile,
        provider_model: probe.provider_model.clone(),
        provider_base_url: probe.provider_base_url.clone(),
        request_style: probe.request_style.clone(),
        score,
        score_fmt: fmt_score(score),
        max_context_tokens,
        max_context_input_tokens,
        first_failure_tokens,
        first_failure_http_status,
        first_failure_error_kind,
        categories,
        artifacts: ScoreArtifacts {
            score_json: String::new(),
            score_html: String::new(),
            probe_summary_json: path_string(&run_dir.join("probe-summary.json")),
            capability_report_html,
        },
        probe,
    }
}

fn context_category(
    max_context_tokens: Option<u64>,
    profile_max_tokens: u64,
    passed_any: bool,
) -> ScoreCategory {
    let tokens = max_context_tokens.unwrap_or(0);
    let score = if passed_any && profile_max_tokens > 0 {
        ((tokens as f64 / profile_max_tokens as f64) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    ScoreCategory {
        name: "Context Capacity".to_string(),
        score,
        score_fmt: fmt_score(score),
        weight: 0.35,
        passed: usize::from(passed_any),
        total: 1,
        detail: format!("{tokens}/{profile_max_tokens} tokens"),
    }
}

fn suite_category(
    name: &str,
    weight: f64,
    summary: &crate::report::ReportSummary,
    suite_names: &[&str],
) -> ScoreCategory {
    let mut passed = 0usize;
    let mut total = 0usize;
    for row in &summary.suites {
        if suite_names.contains(&row.label.as_str()) {
            passed += row.passed;
            total += row.total;
        }
    }
    let score = pass_rate_score(passed, total);
    ScoreCategory {
        name: name.to_string(),
        score,
        score_fmt: fmt_score(score),
        weight,
        passed,
        total,
        detail: if total == 0 {
            "not run".to_string()
        } else {
            format!("{passed}/{total} passed")
        },
    }
}

fn latency_category(avg_latency_ms: u64) -> ScoreCategory {
    let score = match avg_latency_ms {
        0..=5_000 => 100.0,
        5_001..=15_000 => 85.0,
        15_001..=30_000 => 65.0,
        30_001..=60_000 => 45.0,
        _ => 25.0,
    };
    ScoreCategory {
        name: "Latency".to_string(),
        score,
        score_fmt: fmt_score(score),
        weight: 0.05,
        passed: usize::from(avg_latency_ms > 0),
        total: 1,
        detail: format!("avg {avg_latency_ms} ms"),
    }
}

fn pass_rate_score(passed: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (passed as f64 / total as f64) * 100.0
    }
}

fn write_score_html(summary: &ScoreSummary, out_path: &Path) -> Result<()> {
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let view = ScoreView {
        summary,
        max_context_display: display_tokens(summary.max_context_tokens),
        first_failure_display: display_tokens(summary.first_failure_tokens),
    };
    fs::write(out_path, view.render()?)
        .with_context(|| format!("failed to write {}", out_path.display()))?;
    Ok(())
}

fn fmt_score(score: f64) -> String {
    format!("{score:.1}")
}

fn display_tokens(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn display_opt<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_parse() {
        assert_eq!(ScoreProfile::parse("quick").unwrap(), ScoreProfile::Quick);
        assert_eq!(
            ScoreProfile::parse("max_context").unwrap(),
            ScoreProfile::MaxContext
        );
        assert!(ScoreProfile::parse("unknown").is_err());
    }

    #[test]
    fn context_category_scores_against_profile_max() {
        let category = context_category(Some(500), 1_000, true);
        assert_eq!(category.score, 50.0);
        let category = context_category(None, 1_000, false);
        assert_eq!(category.score, 0.0);
    }
}
