use crate::benchmark::BenchmarkResult;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComparisonSummary {
    pub baseline_total: usize,
    pub candidate_total: usize,
    pub common_total: usize,
    pub improved_ids: Vec<String>,
    pub regressed_ids: Vec<String>,
    pub added_ids: Vec<String>,
    pub removed_ids: Vec<String>,
    pub avg_latency_delta_ms: i64,
    pub avg_input_tokens_delta: i64,
    pub avg_output_tokens_delta: i64,
}

impl ComparisonSummary {
    pub fn render(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("baseline: {}\n", self.baseline_total));
        output.push_str(&format!("candidate: {}\n", self.candidate_total));
        output.push_str(&format!("common: {}\n", self.common_total));
        output.push_str(&format!("improved: {}\n", self.improved_ids.len()));
        output.push_str(&format!("regressed: {}\n", self.regressed_ids.len()));
        output.push_str(&format!("added: {}\n", self.added_ids.len()));
        output.push_str(&format!("removed: {}\n", self.removed_ids.len()));
        output.push_str(&format!(
            "avg latency delta ms: {}\n",
            self.avg_latency_delta_ms
        ));
        output.push_str(&format!(
            "avg input tokens delta: {}\n",
            self.avg_input_tokens_delta
        ));
        output.push_str(&format!(
            "avg output tokens delta: {}\n",
            self.avg_output_tokens_delta
        ));

        if !self.improved_ids.is_empty() {
            output.push_str(&format!("improved ids: {}\n", self.improved_ids.join(", ")));
        }
        if !self.regressed_ids.is_empty() {
            output.push_str(&format!(
                "regressed ids: {}\n",
                self.regressed_ids.join(", ")
            ));
        }
        if !self.added_ids.is_empty() {
            output.push_str(&format!("added ids: {}\n", self.added_ids.join(", ")));
        }
        if !self.removed_ids.is_empty() {
            output.push_str(&format!("removed ids: {}\n", self.removed_ids.join(", ")));
        }

        output
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

pub fn compare_results(baseline_path: &str, candidate_path: &str) -> Result<ComparisonSummary> {
    let baseline = read_results(baseline_path)?;
    let candidate = read_results(candidate_path)?;

    let baseline_total = baseline.len();
    let candidate_total = candidate.len();

    let mut improved_ids = Vec::new();
    let mut regressed_ids = Vec::new();
    let mut common_total = 0;
    let mut latency_delta_sum = 0i128;
    let mut input_tokens_delta_sum = 0i128;
    let mut output_tokens_delta_sum = 0i128;

    for (id, baseline_result) in &baseline {
        if let Some(candidate_result) = candidate.get(id) {
            common_total += 1;
            latency_delta_sum +=
                candidate_result.latency_ms as i128 - baseline_result.latency_ms as i128;
            input_tokens_delta_sum +=
                candidate_result.input_tokens as i128 - baseline_result.input_tokens as i128;
            output_tokens_delta_sum +=
                candidate_result.output_tokens as i128 - baseline_result.output_tokens as i128;
            match (baseline_result.passed, candidate_result.passed) {
                (false, true) => improved_ids.push(id.clone()),
                (true, false) => regressed_ids.push(id.clone()),
                _ => {}
            }
        }
    }

    let added_ids = candidate
        .keys()
        .filter(|id| !baseline.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let removed_ids = baseline
        .keys()
        .filter(|id| !candidate.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();

    Ok(ComparisonSummary {
        baseline_total,
        candidate_total,
        common_total,
        improved_ids,
        regressed_ids,
        added_ids,
        removed_ids,
        avg_latency_delta_ms: average_delta(latency_delta_sum, common_total),
        avg_input_tokens_delta: average_delta(input_tokens_delta_sum, common_total),
        avg_output_tokens_delta: average_delta(output_tokens_delta_sum, common_total),
    })
}

pub fn write_comparison_html(summary: &ComparisonSummary, out_path: &str) -> Result<()> {
    let html = summary_to_html(summary);
    fs::write(out_path, html)
        .with_context(|| format!("failed to write comparison report {out_path}"))
}

fn average_delta(sum: i128, count: usize) -> i64 {
    if count == 0 {
        return 0;
    }
    let average = sum / count as i128;
    average.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn summary_to_html(summary: &ComparisonSummary) -> String {
    let rows = [
        ("Baseline total", summary.baseline_total.to_string()),
        ("Candidate total", summary.candidate_total.to_string()),
        ("Common total", summary.common_total.to_string()),
        ("Improved", summary.improved_ids.len().to_string()),
        ("Regressed", summary.regressed_ids.len().to_string()),
        ("Added", summary.added_ids.len().to_string()),
        ("Removed", summary.removed_ids.len().to_string()),
        (
            "Avg latency delta ms",
            summary.avg_latency_delta_ms.to_string(),
        ),
        (
            "Avg input tokens delta",
            summary.avg_input_tokens_delta.to_string(),
        ),
        (
            "Avg output tokens delta",
            summary.avg_output_tokens_delta.to_string(),
        ),
    ];

    let list_block = |title: &str, items: &[String]| {
        if items.is_empty() {
            format!("<p>{title}: none</p>")
        } else {
            format!(
                "<h3>{title}</h3><pre>{}</pre>",
                escape_html(&items.join(", "))
            )
        }
    };

    let metrics = rows
        .into_iter()
        .map(|(label, value)| {
            format!(
                "<tr><th>{}</th><td>{}</td></tr>",
                escape_html(label),
                escape_html(&value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Long Context Comparison Report</title>
  <style>
    body {{ font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 32px; color: #1f2933; background: #f7f8fa; }}
    main {{ max-width: 1100px; margin: 0 auto; }}
    h1 {{ font-size: 28px; margin-bottom: 18px; }}
    h2 {{ font-size: 18px; margin: 24px 0 10px; }}
    table {{ width: 100%; border-collapse: collapse; background: #fff; border: 1px solid #d9dee7; }}
    th, td {{ padding: 10px 12px; border-bottom: 1px solid #e4e7ec; text-align: left; vertical-align: top; }}
    th {{ width: 240px; background: #eef1f5; }}
    pre {{ white-space: pre-wrap; margin: 0; background: #fff; border: 1px solid #d9dee7; padding: 12px; }}
  </style>
</head>
<body>
<main>
  <h1>Long Context Comparison Report</h1>
  <table>
{metrics}
  </table>
  <h2>Improved IDs</h2>
  {}
  <h2>Regressed IDs</h2>
  {}
  <h2>Added IDs</h2>
  {}
  <h2>Removed IDs</h2>
  {}
</main>
</body>
</html>
"#,
        list_block("Improved IDs", &summary.improved_ids),
        list_block("Regressed IDs", &summary.regressed_ids),
        list_block("Added IDs", &summary.added_ids),
        list_block("Removed IDs", &summary.removed_ids),
    )
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn read_results(path: &str) -> Result<BTreeMap<String, BenchmarkResult>> {
    let text = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let mut results = BTreeMap::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let result = serde_json::from_str::<BenchmarkResult>(line)
            .with_context(|| format!("failed to parse JSONL line {}", idx + 1))?;
        results.insert(result.id.clone(), result);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn result_line(id: &str, passed: bool) -> String {
        result_line_with_metrics(id, passed, 10, 100, 5)
    }

    fn result_line_with_metrics(
        id: &str,
        passed: bool,
        latency_ms: u64,
        input_tokens: u64,
        output_tokens: u64,
    ) -> String {
        serde_json::json!({
            "schema_version": 1,
            "suite": "needle",
            "id": id,
            "provider_model": "gpt-4.1",
            "provider_base_url": "https://api.openai.com/v1",
            "http_status": 200,
            "request_id": "req_test",
            "passed": passed,
            "attempts": 1,
            "latency_ms": latency_ms,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "answer": "A",
            "error": null,
            "error_kind": null
        })
        .to_string()
    }

    #[test]
    fn compare_detects_improvements_and_regressions() {
        let tmp = tempdir().unwrap();
        let baseline = tmp.path().join("baseline.jsonl");
        let candidate = tmp.path().join("candidate.jsonl");
        fs::write(
            &baseline,
            format!("{}\n{}\n", result_line("a", false), result_line("b", true)),
        )
        .unwrap();
        fs::write(
            &candidate,
            format!("{}\n{}\n", result_line("a", true), result_line("c", true)),
        )
        .unwrap();

        let summary =
            compare_results(baseline.to_str().unwrap(), candidate.to_str().unwrap()).unwrap();
        assert_eq!(summary.baseline_total, 2);
        assert_eq!(summary.candidate_total, 2);
        assert_eq!(summary.common_total, 1);
        assert_eq!(summary.improved_ids, vec!["a"]);
        assert_eq!(summary.regressed_ids, Vec::<String>::new());
        assert_eq!(summary.added_ids, vec!["c"]);
        assert_eq!(summary.removed_ids, vec!["b"]);
    }

    #[test]
    fn compare_reports_average_metric_deltas() {
        let tmp = tempdir().unwrap();
        let baseline = tmp.path().join("baseline.jsonl");
        let candidate = tmp.path().join("candidate.jsonl");
        fs::write(
            &baseline,
            format!(
                "{}\n{}\n",
                result_line_with_metrics("a", true, 10, 100, 5),
                result_line_with_metrics("b", true, 20, 120, 10)
            ),
        )
        .unwrap();
        fs::write(
            &candidate,
            format!(
                "{}\n{}\n",
                result_line_with_metrics("a", true, 20, 90, 7),
                result_line_with_metrics("b", true, 30, 110, 12)
            ),
        )
        .unwrap();

        let summary =
            compare_results(baseline.to_str().unwrap(), candidate.to_str().unwrap()).unwrap();
        assert_eq!(summary.avg_latency_delta_ms, 10);
        assert_eq!(summary.avg_input_tokens_delta, -10);
        assert_eq!(summary.avg_output_tokens_delta, 2);
        assert!(summary.to_json().unwrap().contains("avg_latency_delta_ms"));
    }

    #[test]
    fn comparison_html_includes_summary() {
        let summary = ComparisonSummary {
            baseline_total: 2,
            candidate_total: 3,
            common_total: 1,
            improved_ids: vec!["a".to_string()],
            regressed_ids: vec!["b".to_string()],
            added_ids: vec!["c".to_string()],
            removed_ids: vec!["d".to_string()],
            avg_latency_delta_ms: 5,
            avg_input_tokens_delta: -1,
            avg_output_tokens_delta: 2,
        };

        let html = summary_to_html(&summary);
        assert!(html.contains("Long Context Comparison Report"));
        assert!(html.contains("Improved IDs"));
        assert!(html.contains("Candidate total"));
        assert!(html.contains("a"));
    }
}
