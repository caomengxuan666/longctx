use std::fs;

#[test]
fn docs_cover_current_cli_and_config_surface() {
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

    let config = fs::read_to_string("docs/configuration.md")
        .expect("docs/configuration.md should be readable");
    assert!(config.contains("provider.request_style"));
    assert!(config.contains("run.log_requests"));
    assert!(config.contains("run.request_log_path"));
    assert!(config.contains("reports/http-log.jsonl"));
    assert!(config.contains("context.index.json"));
    assert!(config.contains("Duplicate test IDs"));
    assert!(config.contains("LLM judge auditing"));

    let roadmap = fs::read_to_string("docs/productionization.md")
        .expect("docs/productionization.md should be readable");
    assert!(roadmap.contains("schema_version"));
    assert!(roadmap.contains("request IDs"));
}
