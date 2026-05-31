use crate::benchmark::{BenchmarkResult, SCHEMA_VERSION};
use anyhow::{Context, Result};
use askama::Template;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportSummary {
    pub total: usize,
    pub passed: usize,
    pub pass_rate: f64,
    pub avg_latency_ms: u64,
    pub suites: Vec<AggregateSummary>,
    pub token_counts: Vec<AggregateSummary>,
    pub positions: Vec<AggregateSummary>,
    pub failure_groups: Vec<FailureGroupSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AggregateSummary {
    pub label: String,
    pub total: usize,
    pub passed: usize,
    pub pass_rate: f64,
    pub pass_rate_fmt: String,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FailureGroupSummary {
    pub label: String,
    pub count: usize,
}

impl ReportSummary {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub fn generate_summary(results_path: &str) -> Result<ReportSummary> {
    let results = read_results(results_path)?;
    Ok(build_summary(&results))
}

struct ResultRow {
    id: String,
    suite_display: String,
    token_count_display: String,
    model_display: String,
    status_class: &'static str,
    status_text: &'static str,
    attempts: u32,
    latency_ms: u64,
    input_tokens: u64,
    http_status_display: String,
    request_id_display: String,
    rate_limit_remaining_display: String,
    rate_limit_reset_display: String,
    error_kind_display: String,
    routing_display: String,
    routing_context_display: String,
    judge_display: String,
    answer_display: String,
    error_display: String,
}

#[derive(Template)]
#[template(path = "report.html")]
struct ReportView<'a> {
    total: usize,
    passed: usize,
    pass_rate_fmt: String,
    avg_latency_ms: u64,
    latency_trend: String,
    token_trend: String,
    suite_rows: &'a [AggregateSummary],
    token_counts: &'a [AggregateSummary],
    position_rows: &'a [AggregateSummary],
    failure_groups: &'a [FailureGroupSummary],
    results: Vec<ResultRow>,
}

pub fn generate_html(results_path: &str, out_path: &str) -> Result<()> {
    let results = read_results(results_path)?;
    let summary = build_summary(&results);

    let latency_trend = render_line_chart(
        &results
            .iter()
            .map(|result| result.latency_ms as f64)
            .collect::<Vec<_>>(),
        "#2563eb",
    );
    let token_trend = render_line_chart(
        &results
            .iter()
            .map(|result| result.input_tokens as f64)
            .collect::<Vec<_>>(),
        "#0f766e",
    );

    let result_rows: Vec<ResultRow> = results
        .iter()
        .map(|result| {
            let (status_class, status_text) = if result.passed {
                ("pass", "pass")
            } else {
                ("fail", "fail")
            };
            let routing_display = result.routing.as_ref().map_or_else(String::new, |routing| {
                format!(
                    "{} / {} / {:.2}",
                    routing.status, routing.method, routing.confidence
                )
            });
            let routing_context_display = result
                .routing
                .as_ref()
                .and_then(|routing| routing.selected_context_path.clone())
                .unwrap_or_default();
            let judge_display = render_judge_display(result);
            ResultRow {
                id: result.id.clone(),
                suite_display: result.suite.clone().unwrap_or_default(),
                token_count_display: result
                    .token_count
                    .map(|tc| tc.to_string())
                    .unwrap_or_default(),
                model_display: result.provider_model.clone().unwrap_or_default(),
                status_class,
                status_text,
                attempts: result.attempts,
                latency_ms: result.latency_ms,
                input_tokens: result.input_tokens,
                http_status_display: result
                    .http_status
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                request_id_display: result.request_id.clone().unwrap_or_default(),
                rate_limit_remaining_display: result
                    .rate_limit_remaining
                    .clone()
                    .unwrap_or_default(),
                rate_limit_reset_display: result.rate_limit_reset.clone().unwrap_or_default(),
                error_kind_display: result
                    .error_kind
                    .as_ref()
                    .map(|k| format!("{k:?}"))
                    .unwrap_or_default(),
                routing_display,
                routing_context_display,
                judge_display,
                answer_display: result.answer.clone().unwrap_or_default(),
                error_display: result.error.clone().unwrap_or_default(),
            }
        })
        .collect();

    let view = ReportView {
        total: summary.total,
        passed: summary.passed,
        pass_rate_fmt: format!("{:.1}", summary.pass_rate),
        avg_latency_ms: summary.avg_latency_ms,
        latency_trend,
        token_trend,
        suite_rows: &summary.suites,
        token_counts: &summary.token_counts,
        position_rows: &summary.positions,
        failure_groups: &summary.failure_groups,
        results: result_rows,
    };

    let html = view.render().context("failed to render report template")?;
    fs::write(out_path, html).with_context(|| format!("failed to write report {out_path}"))?;
    Ok(())
}

fn read_results(results_path: &str) -> Result<Vec<BenchmarkResult>> {
    let text = fs::read_to_string(results_path)
        .with_context(|| format!("failed to read results file {results_path}"))?;
    let mut results = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let result = serde_json::from_str::<BenchmarkResult>(line)
            .with_context(|| format!("failed to parse JSONL line {}", idx + 1))?;
        if result.schema_version > SCHEMA_VERSION {
            anyhow::bail!(
                "results file schema_version {} on line {} is newer than supported schema_version {}",
                result.schema_version,
                idx + 1,
                SCHEMA_VERSION
            );
        }
        if let Some(routing) = &result.routing {
            if routing.schema_version > SCHEMA_VERSION {
                anyhow::bail!(
                    "routing schema_version {} on line {} is newer than supported schema_version {}",
                    routing.schema_version,
                    idx + 1,
                    SCHEMA_VERSION
                );
            }
        }
        results.push(result);
    }
    Ok(results)
}

fn build_summary(results: &[BenchmarkResult]) -> ReportSummary {
    let total = results.len();
    let passed = results.iter().filter(|result| result.passed).count();
    let pass_rate = if total == 0 {
        0.0
    } else {
        (passed as f64 / total as f64) * 100.0
    };
    let avg_latency_ms = if total == 0 {
        0
    } else {
        results.iter().map(|result| result.latency_ms).sum::<u64>() / total as u64
    };

    ReportSummary {
        total,
        passed,
        pass_rate,
        avg_latency_ms,
        suites: render_aggregate_summary(results, |result| {
            result
                .suite
                .clone()
                .unwrap_or_else(|| "unknown".to_string())
        }),
        token_counts: render_aggregate_summary(results, |result| {
            result
                .token_count
                .map(|token_count| token_count.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        }),
        positions: render_aggregate_summary(results, |result| {
            result
                .metadata
                .get("position_label")
                .cloned()
                .unwrap_or_default()
        })
        .into_iter()
        .filter(|agg| !agg.label.is_empty())
        .collect(),
        failure_groups: render_failure_summary(results),
    }
}

fn render_aggregate_summary<F>(results: &[BenchmarkResult], label_fn: F) -> Vec<AggregateSummary>
where
    F: Fn(&BenchmarkResult) -> String,
{
    let mut groups: BTreeMap<String, (usize, usize, u64)> = BTreeMap::new();
    for result in results {
        let label = label_fn(result);
        let entry = groups.entry(label).or_insert((0, 0, 0));
        entry.0 += 1;
        if result.passed {
            entry.1 += 1;
        }
        entry.2 += result.latency_ms;
    }

    groups
        .into_iter()
        .map(|(label, (total, passed, latency_sum))| {
            let pass_rate = if total == 0 {
                0.0
            } else {
                (passed as f64 / total as f64) * 100.0
            };
            let avg_latency_ms = if total == 0 {
                0
            } else {
                latency_sum / total as u64
            };
            AggregateSummary {
                label,
                total,
                passed,
                pass_rate,
                pass_rate_fmt: format!("{:.1}", pass_rate),
                avg_latency_ms,
            }
        })
        .collect()
}

fn render_failure_summary(results: &[BenchmarkResult]) -> Vec<FailureGroupSummary> {
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    for result in results.iter().filter(|result| !result.passed) {
        let group = failure_group(result);
        *groups.entry(group).or_insert(0) += 1;
    }

    groups
        .into_iter()
        .map(|(label, count)| FailureGroupSummary { label, count })
        .collect()
}

fn render_judge_display(result: &BenchmarkResult) -> String {
    if result.judge_latency_ms.is_none()
        && result.judge_http_status.is_none()
        && result.judge_error.is_none()
    {
        return String::new();
    }

    let mut parts = Vec::new();
    if let Some(status) = result.judge_http_status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(attempts) = result.judge_attempts {
        parts.push(format!("{attempts} attempt(s)"));
    }
    if let Some(latency) = result.judge_latency_ms {
        parts.push(format!("{latency} ms"));
    }
    if let Some(input_tokens) = result.judge_input_tokens {
        parts.push(format!("{input_tokens} in"));
    }
    if let Some(output_tokens) = result.judge_output_tokens {
        parts.push(format!("{output_tokens} out"));
    }
    if let Some(error) = &result.judge_error {
        parts.push(error.clone());
    }
    parts.join(" / ")
}

fn render_line_chart(values: &[f64], color: &str) -> String {
    const WIDTH: f64 = 600.0;
    const HEIGHT: f64 = 180.0;
    const PADDING: f64 = 16.0;

    if values.is_empty() {
        return "<svg viewBox=\"0 0 600 180\" role=\"img\" aria-label=\"Empty trend chart\"><text x=\"24\" y=\"96\" fill=\"#52606d\">No data</text></svg>".to_string();
    }

    let min = values
        .iter()
        .fold(f64::INFINITY, |acc, value| acc.min(*value));
    let max = values
        .iter()
        .fold(f64::NEG_INFINITY, |acc, value| acc.max(*value));
    let span = if (max - min).abs() < f64::EPSILON {
        1.0
    } else {
        max - min
    };
    let x_step = if values.len() <= 1 {
        0.0
    } else {
        (WIDTH - 2.0 * PADDING) / (values.len() as f64 - 1.0)
    };

    let mut points = Vec::with_capacity(values.len());
    let mut circles = Vec::with_capacity(values.len());
    for (idx, value) in values.iter().enumerate() {
        let x = PADDING + idx as f64 * x_step;
        let normalized = (value - min) / span;
        let y = HEIGHT - PADDING - normalized * (HEIGHT - 2.0 * PADDING);
        points.push(format!("{x:.1},{y:.1}"));
        circles.push(format!(
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3\" fill=\"{color}\" />"
        ));
    }

    let min_label = format!("{min:.0}");
    let max_label = format!("{max:.0}");
    let label_y = HEIGHT - 6.0;

    format!(
        r##"<svg viewBox="0 0 600 180" role="img" aria-label="Trend chart">
  <line x1="16" y1="16" x2="16" y2="164" stroke="#d9dee7" />
  <line x1="16" y1="164" x2="584" y2="164" stroke="#d9dee7" />
  <polyline fill="none" stroke="{color}" stroke-width="3" points="{}" />
  {}
  <text x="20" y="{label_y}" fill="#52606d" font-size="12">{}</text>
  <text x="540" y="{label_y}" fill="#52606d" font-size="12" text-anchor="end">{}</text>
</svg>"##,
        points.join(" "),
        circles.join("\n  "),
        min_label,
        max_label,
    )
}

fn failure_group(result: &BenchmarkResult) -> String {
    match result.error_kind.as_ref() {
        Some(kind) => format!("{kind:?}"),
        None => {
            if result.passed {
                "Pass".to_string()
            } else {
                "Grading".to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn report_includes_suite_and_failure_summary() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        let out = tmp.path().join("report.html");
        fs::write(
            &results,
            r#"{"schema_version":1,"suite":"needle","token_count":100000,"id":"a","provider_model":"gpt-4.1","provider_base_url":"https://api.openai.com/v1","http_status":200,"request_id":"req_1","rate_limit_remaining":"9","rate_limit_reset":"60","passed":true,"attempts":1,"latency_ms":10,"input_tokens":100,"output_tokens":5,"answer":"A","error":null,"error_kind":null}
{"schema_version":1,"suite":"needle","token_count":100000,"id":"b","provider_model":"gpt-4.1","provider_base_url":"https://api.openai.com/v1","http_status":500,"request_id":"req_2","rate_limit_remaining":"8","rate_limit_reset":"60","passed":false,"attempts":1,"latency_ms":20,"input_tokens":110,"output_tokens":0,"answer":null,"error":"boom","error_kind":"Http"}
"#,
        )
        .unwrap();

        generate_html(results.to_str().unwrap(), out.to_str().unwrap()).unwrap();
        let html = fs::read_to_string(out).unwrap();
        assert!(html.contains("Suites"));
        assert!(html.contains("Latency Trend"));
        assert!(html.contains("Token Counts"));
        assert!(html.contains("Failure Groups"));
        assert!(html.contains("needle"));
        assert!(html.contains("Http"));
    }

    #[test]
    fn report_includes_judge_audit_details() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        let out = tmp.path().join("report.html");
        fs::write(
            &results,
            r#"{"schema_version":1,"suite":"judge","token_count":100,"id":"judge-1","provider_model":"gpt-4.1","provider_base_url":"https://api.openai.com/v1","http_status":200,"request_id":"req_1","passed":false,"attempts":1,"latency_ms":10,"input_tokens":100,"output_tokens":5,"answer":"A","error":"LLM judge failed: HTTP 500","error_kind":"Judge","judge_latency_ms":7,"judge_input_tokens":20,"judge_output_tokens":0,"judge_http_status":500,"judge_attempts":1,"judge_error":"HTTP 500 from judge"}
"#,
        )
        .unwrap();

        generate_html(results.to_str().unwrap(), out.to_str().unwrap()).unwrap();
        let html = fs::read_to_string(out).unwrap();
        assert!(html.contains("Judge"));
        assert!(html.contains("HTTP 500"));
        assert!(html.contains("HTTP 500 from judge"));
    }

    #[test]
    fn report_includes_routing_decision_columns() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        let out = tmp.path().join("report.html");
        fs::write(
            &results,
            r#"{"schema_version":1,"suite":"needle","token_count":100000,"id":"a","provider_model":"gpt-4.1","provider_base_url":"https://api.openai.com/v1","http_status":200,"request_id":"req_1","rate_limit_remaining":"9","rate_limit_reset":"60","passed":true,"attempts":1,"latency_ms":10,"input_tokens":100,"output_tokens":5,"answer":"A","error":null,"error_kind":null,"routing":{"schema_version":1,"test_id":"a","selected_context_id":"needle_context","selected_context_path":"contexts/needle_context.txt","method":"hybrid","status":"selected","confidence":0.9,"candidates":[],"llm_router_used":false,"input_tokens":0,"output_tokens":0,"latency_ms":0,"reason":"local_router_selected_top_candidate"}}
"#,
        )
        .unwrap();

        generate_html(results.to_str().unwrap(), out.to_str().unwrap()).unwrap();
        let html = fs::read_to_string(out).unwrap();
        assert!(html.contains("Routing"));
        assert!(html.contains("selected / hybrid / 0.90"));
        assert!(html.contains("contexts/needle_context.txt"));
    }

    #[test]
    fn summary_json_includes_aggregates() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        fs::write(
            &results,
            r#"{"schema_version":1,"suite":"needle","token_count":100000,"id":"a","provider_model":"gpt-4.1","provider_base_url":"https://api.openai.com/v1","http_status":200,"request_id":"req_1","rate_limit_remaining":"9","rate_limit_reset":"60","passed":true,"attempts":1,"latency_ms":10,"input_tokens":100,"output_tokens":5,"answer":"A","error":null,"error_kind":null}
{"schema_version":1,"suite":"needle","token_count":100000,"id":"b","provider_model":"gpt-4.1","provider_base_url":"https://api.openai.com/v1","http_status":500,"request_id":"req_2","rate_limit_remaining":"8","rate_limit_reset":"60","passed":false,"attempts":1,"latency_ms":20,"input_tokens":110,"output_tokens":0,"answer":null,"error":"boom","error_kind":"Http"}
"#,
        )
        .unwrap();

        let summary = generate_summary(results.to_str().unwrap()).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.suites.len(), 1);
        assert_eq!(summary.token_counts.len(), 1);
        assert_eq!(summary.failure_groups[0].label, "Http");
        assert!(summary.to_json().unwrap().contains("failure_groups"));
    }

    #[test]
    fn report_handles_empty_results_file() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        let out = tmp.path().join("report.html");
        fs::write(&results, "").unwrap();

        generate_html(results.to_str().unwrap(), out.to_str().unwrap()).unwrap();
        let html = fs::read_to_string(out).unwrap();
        assert!(html.contains("Long Context Benchmark Report"));
        assert!(html.contains(">0<"));
    }

    #[test]
    fn report_rejects_newer_result_schema_version() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        fs::write(
            &results,
            r#"{"schema_version":999,"id":"a","passed":true,"latency_ms":1,"input_tokens":1,"output_tokens":1,"answer":"A","error":null}
"#,
        )
        .unwrap();

        let error = generate_summary(results.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("newer than supported"));
    }

    #[test]
    fn report_rejects_newer_routing_schema_version() {
        let tmp = tempdir().unwrap();
        let results = tmp.path().join("results.jsonl");
        fs::write(
            &results,
            r#"{"schema_version":1,"id":"a","passed":true,"latency_ms":1,"input_tokens":1,"output_tokens":1,"answer":"A","error":null,"routing":{"schema_version":999,"test_id":"a","selected_context_id":"ctx","selected_context_path":"contexts/a.txt","method":"hybrid","status":"selected","confidence":1.0,"candidates":[],"llm_router_used":false,"input_tokens":0,"output_tokens":0,"latency_ms":0}}
"#,
        )
        .unwrap();

        let error = generate_summary(results.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("routing schema_version 999"));
    }

    #[test]
    fn line_chart_with_empty_values() {
        let svg = render_line_chart(&[], "#2563eb");
        assert!(svg.contains("No data"));
    }

    #[test]
    fn line_chart_with_single_value() {
        let svg = render_line_chart(&[42.0], "#2563eb");
        assert!(svg.contains("svg"));
        assert!(svg.contains("42"));
    }

    #[test]
    fn line_chart_with_all_same_values() {
        let svg = render_line_chart(&[5.0, 5.0, 5.0], "#2563eb");
        assert!(svg.contains("svg"));
        assert!(svg.contains("5"));
    }

    #[test]
    fn failure_group_classifies_by_error_kind() {
        let mut result = BenchmarkResult {
            schema_version: 1,
            suite: None,
            token_count: None,
            id: "test".to_string(),
            provider_model: None,
            provider_base_url: None,
            http_status: None,
            request_id: None,
            rate_limit_remaining: None,
            rate_limit_reset: None,
            passed: false,
            attempts: 1,
            latency_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            answer: None,
            error: None,
            error_kind: Some(crate::benchmark::ErrorKind::Http),
            routing: None,
            judge_latency_ms: None,
            judge_input_tokens: None,
            judge_output_tokens: None,
            judge_http_status: None,
            judge_attempts: None,
            judge_error: None,
            metadata: BTreeMap::new(),
        };
        assert_eq!(failure_group(&result), "Http");

        result.error_kind = Some(crate::benchmark::ErrorKind::Transport);
        assert_eq!(failure_group(&result), "Transport");

        result.error_kind = None;
        assert_eq!(failure_group(&result), "Grading");

        result.passed = true;
        assert_eq!(failure_group(&result), "Pass");
    }
}
