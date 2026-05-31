use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::tempdir;

fn longctx() -> Command {
    Command::new(env!("CARGO_BIN_EXE_longctx"))
}

fn assert_success(output: Output) -> String {
    if !output.status.success() {
        panic!(
            "command failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_failure(output: Output) -> String {
    if output.status.success() {
        panic!(
            "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_config(bench_dir: &Path, api_key_env: &str) {
    fs::write(
        bench_dir.join("config.toml"),
        format!(
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "{api_key_env}"
model = "example-model"
"#
        ),
    )
    .unwrap();
}

fn result_line(id: &str, passed: bool, latency_ms: u64) -> String {
    serde_json::json!({
        "schema_version": 1,
        "suite": "needle",
        "token_count": 100,
        "id": id,
        "provider_model": "example-model",
        "provider_base_url": "https://api.example.test/v1",
        "http_status": 200,
        "request_id": "req_test",
        "passed": passed,
        "attempts": 1,
        "latency_ms": latency_ms,
        "input_tokens": 10,
        "output_tokens": 2,
        "answer": "A",
        "error": null,
        "error_kind": null
    })
    .to_string()
}

#[test]
fn cli_generate_validate_index_and_dry_run() {
    let tmp = tempdir().unwrap();
    let bench = tmp.path().join("bench");

    assert_success(
        longctx()
            .args([
                "generate",
                "needle",
                "--tokens",
                "100",
                "--seed",
                "42",
                "--out",
                bench.to_str().unwrap(),
            ])
            .output()
            .unwrap(),
    );
    assert!(bench.join("contexts/needle_context-100.txt").exists());
    assert!(bench.join("manifests/needle-100.json").exists());
    assert!(bench.join("context.index.json").exists());

    write_config(&bench, "LONGCTX_CLI_SMOKE_MISSING_KEY");
    let validate = assert_success(
        longctx()
            .args(["validate", bench.to_str().unwrap(), "--skip-api-key-check"])
            .output()
            .unwrap(),
    );
    assert!(validate.contains("validated 1 tests"));

    let index = assert_success(
        longctx()
            .args(["index", bench.to_str().unwrap()])
            .output()
            .unwrap(),
    );
    assert!(index.contains("indexed 1 contexts"));

    let dry_run = assert_success(
        longctx()
            .args([
                "run",
                bench.to_str().unwrap(),
                "--dry-run",
                "--filter",
                "needle",
                "--limit",
                "1",
            ])
            .output()
            .unwrap(),
    );
    assert!(dry_run.contains("dry run ok"));
    assert!(dry_run.contains("selected 1/1 tests"));
    assert!(!bench.join("results.jsonl").exists());
    assert!(!bench.join("run.json").exists());
}

#[test]
fn cli_run_refuses_existing_results_without_force() {
    let tmp = tempdir().unwrap();
    let bench = tmp.path().join("bench");
    assert_success(
        longctx()
            .args([
                "generate",
                "needle",
                "--tokens",
                "100",
                "--seed",
                "42",
                "--out",
                bench.to_str().unwrap(),
            ])
            .output()
            .unwrap(),
    );
    write_config(&bench, "LONGCTX_CLI_SMOKE_KEY");
    fs::write(bench.join("results.jsonl"), "old results").unwrap();

    let output = assert_failure(
        longctx()
            .args(["run", bench.to_str().unwrap()])
            .env("LONGCTX_CLI_SMOKE_KEY", "test-key")
            .output()
            .unwrap(),
    );
    assert!(output.contains("refusing to overwrite"));
}

#[test]
fn cli_generate_invalid_suite_has_no_filesystem_side_effects() {
    let tmp = tempdir().unwrap();
    let bench = tmp.path().join("bench");

    let output = assert_failure(
        longctx()
            .args([
                "generate",
                "not-a-suite",
                "--tokens",
                "100",
                "--out",
                bench.to_str().unwrap(),
            ])
            .output()
            .unwrap(),
    );
    assert!(output.contains("unknown suite type"));
    assert!(!bench.exists());
}

#[test]
fn cli_probe_context_help_is_available() {
    let help = assert_success(
        longctx()
            .args(["probe-context", "--help"])
            .output()
            .unwrap(),
    );
    assert!(help.contains("Automatically probe"));
    assert!(help.contains("--resolution-tokens"));
    assert!(help.contains("--skip-capabilities"));
}

#[test]
fn cli_score_help_is_available() {
    let help = assert_success(longctx().args(["score", "--help"]).output().unwrap());
    assert!(help.contains("Run a complete scoring profile"));
    assert!(help.contains("--profile"));
    assert!(help.contains("max-context"));
    assert!(help.contains("--json"));
}

#[test]
fn cli_report_and_compare_outputs() {
    let tmp = tempdir().unwrap();
    let baseline = tmp.path().join("baseline.jsonl");
    let candidate = tmp.path().join("candidate.jsonl");
    fs::write(&baseline, format!("{}\n", result_line("a", false, 20))).unwrap();
    fs::write(&candidate, format!("{}\n", result_line("a", true, 10))).unwrap();

    let report_json = assert_success(
        longctx()
            .args(["report", candidate.to_str().unwrap(), "--json"])
            .output()
            .unwrap(),
    );
    assert!(report_json.contains("\"total\": 1"));
    assert!(report_json.contains("\"passed\": 1"));

    assert_success(
        longctx()
            .args(["report", candidate.to_str().unwrap()])
            .output()
            .unwrap(),
    );
    assert!(tmp.path().join("reports/report.html").exists());

    let report_path: PathBuf = tmp.path().join("report.html");
    assert_success(
        longctx()
            .args([
                "report",
                candidate.to_str().unwrap(),
                "--out",
                report_path.to_str().unwrap(),
            ])
            .output()
            .unwrap(),
    );
    let html = fs::read_to_string(report_path).unwrap();
    assert!(html.contains("Long Context Benchmark Report"));

    let compare = assert_success(
        longctx()
            .args([
                "compare",
                baseline.to_str().unwrap(),
                candidate.to_str().unwrap(),
            ])
            .output()
            .unwrap(),
    );
    assert!(compare.contains("improved: 1"));
}
