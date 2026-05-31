use crate::benchmark::BenchmarkResult;
use anyhow::{Context, Result};
use askama::Template;
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

#[derive(Template)]
#[template(path = "compare.html")]
struct CompareView<'a> {
    summary: &'a ComparisonSummary,
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
    let view = CompareView { summary };
    let html = view
        .render()
        .context("failed to render comparison template")?;
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

        let view = CompareView { summary: &summary };
        let html = view.render().unwrap();
        assert!(html.contains("Long Context Comparison Report"));
        assert!(html.contains("Improved IDs"));
        assert!(html.contains("Candidate total"));
        assert!(html.contains("a"));
    }

    #[test]
    fn compare_handles_empty_files() {
        let tmp = tempdir().unwrap();
        let baseline = tmp.path().join("baseline.jsonl");
        let candidate = tmp.path().join("candidate.jsonl");
        fs::write(&baseline, "").unwrap();
        fs::write(&candidate, "").unwrap();

        let summary =
            compare_results(baseline.to_str().unwrap(), candidate.to_str().unwrap()).unwrap();
        assert_eq!(summary.baseline_total, 0);
        assert_eq!(summary.candidate_total, 0);
        assert_eq!(summary.common_total, 0);
        assert_eq!(summary.avg_latency_delta_ms, 0);
    }

    #[test]
    fn compare_handles_no_common_ids() {
        let tmp = tempdir().unwrap();
        let baseline = tmp.path().join("baseline.jsonl");
        let candidate = tmp.path().join("candidate.jsonl");
        fs::write(&baseline, format!("{}\n", result_line("a", true))).unwrap();
        fs::write(&candidate, format!("{}\n", result_line("b", true))).unwrap();

        let summary =
            compare_results(baseline.to_str().unwrap(), candidate.to_str().unwrap()).unwrap();
        assert_eq!(summary.common_total, 0);
        assert!(summary.improved_ids.is_empty());
        assert!(summary.regressed_ids.is_empty());
        assert_eq!(summary.added_ids, vec!["b"]);
        assert_eq!(summary.removed_ids, vec!["a"]);
    }

    #[test]
    fn comparison_html_with_empty_lists() {
        let summary = ComparisonSummary {
            baseline_total: 0,
            candidate_total: 0,
            common_total: 0,
            improved_ids: vec![],
            regressed_ids: vec![],
            added_ids: vec![],
            removed_ids: vec![],
            avg_latency_delta_ms: 0,
            avg_input_tokens_delta: 0,
            avg_output_tokens_delta: 0,
        };

        let view = CompareView { summary: &summary };
        let html = view.render().unwrap();
        assert!(html.contains("none"));
    }

    #[test]
    fn average_delta_returns_zero_for_empty() {
        assert_eq!(average_delta(0, 0), 0);
    }
}
