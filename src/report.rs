use crate::benchmark::BenchmarkResult;
use anyhow::{Context, Result};
use std::fs;

pub fn generate_html(results_path: &str, out_path: &str) -> Result<()> {
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

    let total = results.len();
    let passed = results.iter().filter(|result| result.passed).count();
    let pass_rate = if total == 0 {
        0.0
    } else {
        (passed as f64 / total as f64) * 100.0
    };
    let avg_latency = if total == 0 {
        0
    } else {
        results.iter().map(|result| result.latency_ms).sum::<u64>() / total as u64
    };

    let rows = results
        .iter()
        .map(|result| {
            let status = if result.passed { "pass" } else { "fail" };
            format!(
                "<tr><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape_html(&result.id),
                status,
                status,
                result.latency_ms,
                result.input_tokens,
                result.output_tokens,
                escape_html(result.error.as_deref().unwrap_or(""))
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
    main {{ max-width: 1100px; margin: 0 auto; }}
    h1 {{ font-size: 28px; margin-bottom: 18px; }}
    .summary {{ display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; margin-bottom: 24px; }}
    .metric {{ background: #fff; border: 1px solid #d9dee7; border-radius: 8px; padding: 14px; }}
    .metric span {{ display: block; color: #52606d; font-size: 13px; }}
    .metric strong {{ display: block; font-size: 24px; margin-top: 4px; }}
    table {{ width: 100%; border-collapse: collapse; background: #fff; border: 1px solid #d9dee7; }}
    th, td {{ padding: 10px 12px; border-bottom: 1px solid #e4e7ec; text-align: left; font-size: 14px; vertical-align: top; }}
    th {{ background: #eef1f5; font-weight: 650; }}
    .pass {{ color: #0b6b3a; font-weight: 650; }}
    .fail {{ color: #a61b1b; font-weight: 650; }}
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
  <table>
    <thead><tr><th>ID</th><th>Status</th><th>Latency ms</th><th>Input tokens</th><th>Output tokens</th><th>Error</th></tr></thead>
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

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
