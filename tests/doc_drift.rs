use std::fs;

#[test]
fn docs_cover_current_cli_and_config_surface() {
    let cargo_toml = fs::read_to_string("Cargo.toml").expect("Cargo.toml should be readable");
    assert!(cargo_toml.contains("rust-version = \"1.81\""));
    assert!(cargo_toml.contains("\"/report.html\""));
    assert!(cargo_toml.contains("\".codex.toml\""));

    let readme = fs::read_to_string("README.md").expect("README.md should be readable");
    for command in ["generate", "run", "validate", "report", "compare", "index"] {
        assert!(
            readme.contains(command),
            "README.md should mention the {command} command"
        );
    }
    for suite in [
        "needle",
        "multi-needle",
        "conflict",
        "multi-hop",
        "order-dependent",
        "position-sweep",
        "hallucination",
    ] {
        assert!(
            readme.contains(suite),
            "README.md should mention the {suite} suite type"
        );
    }
    assert!(readme.contains("log_requests"));
    assert!(readme.contains("request_log_path"));
    assert!(readme.contains("request_style"));
    assert!(readme.contains("judge_model"));
    assert!(readme.contains("error_kind = \"Judge\""));
    assert!(readme.contains("context = \"auto\""));
    assert!(readme.contains("context.index.json"));
    assert!(readme.contains("--force"));
    assert!(readme.contains("--dry-run"));
    assert!(readme.contains("--filter"));
    assert!(readme.contains("benchmark directory"));
    assert!(readme.contains("Minimum supported Rust version: 1.81"));
    assert!(readme.contains("cargo install longctx"));
    assert!(readme.contains("SHA256"));

    let config = fs::read_to_string("docs/configuration.md")
        .expect("docs/configuration.md should be readable");
    assert!(config.contains("provider.base_url"));
    assert!(config.contains("provider.request_style"));
    assert!(config.contains("run.log_requests"));
    assert!(config.contains("run.request_log_path"));
    assert!(config.contains("reports/http-log.jsonl"));
    assert!(config.contains("context.index.json"));
    assert!(config.contains("Duplicate test IDs"));
    assert!(config.contains("LLM judge auditing"));
    assert!(config.contains("--limit"));
    assert!(config.contains("Context file safety"));

    let roadmap = fs::read_to_string("docs/productionization.md")
        .expect("docs/productionization.md should be readable");
    assert!(roadmap.contains("schema_version"));
    assert!(roadmap.contains("request IDs"));
    assert!(roadmap.contains("license"));
    assert!(roadmap.contains("MSRV 1.81"));
}
