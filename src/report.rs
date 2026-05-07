use crate::benchmark::BenchmarkResult;
use anyhow::{Context, Result};
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
    pub failure_groups: Vec<FailureGroupSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AggregateSummary {
    pub label: String,
    pub total: usize,
    pub passed: usize,
    pub pass_rate: f64,
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

pub fn generate_html(results_path: &str, out_path: &str) -> Result<()> {
    let results = read_results(results_path)?;
    let summary = build_summary(&results);

    let total = summary.total;
    let passed = summary.passed;
    let pass_rate = summary.pass_rate;
    let avg_latency = summary.avg_latency_ms;
    let suite_rows = render_suite_rows(&results);
    let token_rows = render_token_rows(&results);
    let failure_rows = render_failure_rows(&results);
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

    let rows = results
        .iter()
        .map(|result| {
            let status = if result.passed { "pass" } else { "fail" };
            let error_kind = result
                .error_kind
                .as_ref()
                .map(|kind| format!("{kind:?}"))
                .unwrap_or_default();
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"answer\">{}</td><td class=\"error\">{}</td></tr>",
                escape_html(&result.id),
                escape_html(result.suite.as_deref().unwrap_or("")),
                result
                    .token_count
                    .map(|token_count| token_count.to_string())
                    .unwrap_or_default(),
                escape_html(result.provider_model.as_deref().unwrap_or("")),
                status,
                status,
                result.attempts,
                result.latency_ms,
                result.input_tokens,
                result
                    .http_status
                    .map(|status| status.to_string())
                    .unwrap_or_default(),
                escape_html(result.request_id.as_deref().unwrap_or("")),
                escape_html(result.rate_limit_remaining.as_deref().unwrap_or("")),
                escape_html(result.rate_limit_reset.as_deref().unwrap_or("")),
                escape_html(&error_kind),
                escape_html(result.answer.as_deref().unwrap_or("")),
                escape_html(result.error.as_deref().unwrap_or("")),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Long Context Benchmark Report</title>
  <style>
    body {{ font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 32px; color: #1f2933; background: #f7f8fa; }}
    main {{ max-width: 1280px; margin: 0 auto; }}
    h1 {{ font-size: 28px; margin-bottom: 18px; }}
    h2 {{ font-size: 18px; margin: 0 0 10px; }}
    .summary {{ display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; margin-bottom: 24px; }}
    .split {{ display: grid; grid-template-columns: minmax(0, 2fr) minmax(260px, 1fr); gap: 18px; margin-bottom: 24px; }}
    .metric {{ background: #fff; border: 1px solid #d9dee7; border-radius: 8px; padding: 14px; }}
    .metric span {{ display: block; color: #52606d; font-size: 13px; }}
    .metric strong {{ display: block; font-size: 24px; margin-top: 4px; }}
    table {{ width: 100%; border-collapse: collapse; background: #fff; border: 1px solid #d9dee7; table-layout: fixed; }}
    th, td {{ padding: 10px 12px; border-bottom: 1px solid #e4e7ec; text-align: left; font-size: 14px; vertical-align: top; word-break: break-word; }}
    th {{ background: #eef1f5; font-weight: 650; }}
    .trends {{ display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 18px; margin-bottom: 24px; }}
    .trends svg {{ width: 100%; height: auto; background: #fff; border: 1px solid #d9dee7; border-radius: 8px; }}
    .pass {{ color: #0b6b3a; font-weight: 650; }}
    .fail {{ color: #a61b1b; font-weight: 650; }}
    .answer {{ white-space: pre-wrap; }}
    .error {{ white-space: pre-wrap; color: #52606d; }}
  </style>
</head>
<body>
<main>
  <h1>Long Context Benchmark Report</h1>
  <section class="summary">
    <div class="metric"><span>Total</span><strong>{total}</strong></div>
    <div class="metric"><span>Passed</span><strong>{passed}</strong></div>
    <div class="metric"><span>Pass rate</span><strong>{pass_rate:.1}%</strong></div>
    <div class="metric"><span>Avg latency</span><strong>{avg_latency} ms</strong></div>
  </section>
  <section class="trends">
    <div>
      <h2>Latency Trend</h2>
      {latency_trend}
    </div>
    <div>
      <h2>Input Tokens Trend</h2>
      {token_trend}
    </div>
  </section>
  <section class="split">
    <div>
      <h2>Suites</h2>
      <table>
        <thead><tr><th>Suite</th><th>Total</th><th>Passed</th><th>Pass rate</th><th>Avg latency ms</th></tr></thead>
        <tbody>
{suite_rows}
        </tbody>
      </table>
      <h2 style="margin-top:18px;">Token Counts</h2>
      <table>
        <thead><tr><th>Token count</th><th>Total</th><th>Passed</th><th>Pass rate</th><th>Avg latency ms</th></tr></thead>
        <tbody>
{token_rows}
        </tbody>
      </table>
    </div>
    <div>
      <h2>Failure Groups</h2>
      <table>
        <thead><tr><th>Group</th><th>Count</th></tr></thead>
        <tbody>
{failure_rows}
        </tbody>
      </table>
    </div>
  </section>
  <table>
    <thead><tr><th>ID</th><th>Suite</th><th>Token count</th><th>Model</th><th>Status</th><th>Attempts</th><th>Latency ms</th><th>Input tokens</th><th>HTTP</th><th>Request ID</th><th>Rate remaining</th><th>Rate reset</th><th>Error kind</th><th>Answer</th><th>Error</th></tr></thead>
    <tbody>
{rows}
    </tbody>
  </table>
</main>
</body>
</html>
"#
    );

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

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_suite_rows(results: &[BenchmarkResult]) -> String {
    let mut suites: BTreeMap<String, (usize, usize, u64)> = BTreeMap::new();
    for result in results {
        let suite = result
            .suite
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let entry = suites.entry(suite).or_insert((0, 0, 0));
        entry.0 += 1;
        if result.passed {
            entry.1 += 1;
        }
        entry.2 += result.latency_ms;
    }

    suites
        .into_iter()
        .map(|(suite, (total, passed, latency_sum))| {
            let pass_rate = if total == 0 {
                0.0
            } else {
                (passed as f64 / total as f64) * 100.0
            };
            let avg_latency = if total == 0 {
                0
            } else {
                latency_sum / total as u64
            };
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1}%</td><td>{}</td></tr>",
                escape_html(&suite),
                total,
                passed,
                pass_rate,
                avg_latency
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_token_rows(results: &[BenchmarkResult]) -> String {
    let mut tokens: BTreeMap<String, (usize, usize, u64)> = BTreeMap::new();
    for result in results {
        let token_key = result
            .token_count
            .map(|token_count| token_count.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let entry = tokens.entry(token_key).or_insert((0, 0, 0));
        entry.0 += 1;
        if result.passed {
            entry.1 += 1;
        }
        entry.2 += result.latency_ms;
    }

    tokens
        .into_iter()
        .map(|(token_count, (total, passed, latency_sum))| {
            let pass_rate = if total == 0 {
                0.0
            } else {
                (passed as f64 / total as f64) * 100.0
            };
            let avg_latency = if total == 0 {
                0
            } else {
                latency_sum / total as u64
            };
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1}%</td><td>{}</td></tr>",
                escape_html(&token_count),
                total,
                passed,
                pass_rate,
                avg_latency
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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
        escape_html(&min_label),
        escape_html(&max_label),
    )
}

fn render_failure_rows(results: &[BenchmarkResult]) -> String {
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    for result in results.iter().filter(|result| !result.passed) {
        let group = failure_group(result);
        *groups.entry(group).or_insert(0) += 1;
    }

    if groups.is_empty() {
        return "<tr><td colspan=\"2\">No failures</td></tr>".to_string();
    }

    groups
        .into_iter()
        .map(|(group, count)| {
            format!(
                "<tr><td>{}</td><td>{}</td></tr>",
                escape_html(&group),
                count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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
}
