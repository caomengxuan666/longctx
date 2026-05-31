use crate::benchmark::{Config, Grader, TestCase, SCHEMA_VERSION};
use crate::context_index::{
    is_auto_context, load_or_build_context_index, load_or_build_context_index_in_memory,
    read_context_index, validate_context_index, CONTEXT_INDEX_FILE,
};
use crate::runner::{read_config, read_tests};
use anyhow::{bail, Context, Result};
use reqwest::Url;
use std::collections::BTreeSet;
use std::env;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationSummary {
    pub test_count: usize,
    pub config_model: String,
    pub api_key_checked: bool,
}

pub fn validate_benchmark_dir(bench_dir: &str, check_api_key: bool) -> Result<ValidationSummary> {
    let bench_path = Path::new(bench_dir);
    if !bench_path.is_dir() {
        bail!(
            "benchmark directory does not exist: {}",
            bench_path.display()
        );
    }

    let config = read_config(&bench_path.join("config.toml"))?;
    let tests = read_tests(bench_path)?;
    if tests.is_empty() {
        bail!("no benchmark tests found in {}", bench_path.display());
    }
    validate_loaded_benchmark(bench_path, &config, &tests, check_api_key)?;

    Ok(ValidationSummary {
        test_count: tests.len(),
        config_model: config.provider.model,
        api_key_checked: check_api_key,
    })
}

pub(crate) fn validate_loaded_benchmark(
    bench_path: &Path,
    config: &Config,
    tests: &[TestCase],
    check_api_key: bool,
) -> Result<()> {
    validate_loaded_benchmark_with_options(bench_path, config, tests, check_api_key, true)
}

pub(crate) fn validate_loaded_benchmark_with_options(
    bench_path: &Path,
    config: &Config,
    tests: &[TestCase],
    check_api_key: bool,
    write_missing_context_index: bool,
) -> Result<()> {
    validate_config(config, check_api_key)?;
    validate_unique_test_ids(tests)?;
    for test in tests {
        validate_test(bench_path, test)?;
    }
    if tests.iter().any(|test| is_auto_context(&test.context)) {
        let index = if write_missing_context_index {
            load_or_build_context_index(bench_path)?
        } else {
            load_or_build_context_index_in_memory(bench_path)?
        };
        validate_context_index(bench_path, &index)?;
    } else {
        let index_path = bench_path.join(CONTEXT_INDEX_FILE);
        if index_path.exists() {
            let index = read_context_index(&index_path)?;
            validate_context_index(bench_path, &index)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_config(config: &Config, check_api_key: bool) -> Result<()> {
    if config.schema_version > SCHEMA_VERSION {
        bail!(
            "config schema_version {} is newer than supported schema_version {}",
            config.schema_version,
            SCHEMA_VERSION
        );
    }
    if config.provider.base_url.trim().is_empty() {
        bail!("provider.base_url must not be empty");
    }
    validate_provider_base_url(&config.provider.base_url)?;
    if config.provider.api_key_env.trim().is_empty() {
        bail!("provider.api_key_env must not be empty");
    }
    if config.provider.model.trim().is_empty() {
        bail!("provider.model must not be empty");
    }
    if config.run.request_timeout_secs == 0 {
        bail!("run.request_timeout_secs must be greater than 0");
    }
    if config.run.concurrency == 0 {
        bail!("run.concurrency must be greater than 0");
    }
    validate_request_log_path(&config.run.request_log_path)?;
    if check_api_key {
        env::var(&config.provider.api_key_env).with_context(|| {
            format!(
                "environment variable {} is not set",
                config.provider.api_key_env
            )
        })?;
    }
    Ok(())
}

fn validate_provider_base_url(base_url: &str) -> Result<()> {
    let parsed = Url::parse(base_url.trim())
        .with_context(|| "provider.base_url must be an absolute http(s) URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        bail!("provider.base_url must be an absolute http(s) URL with a host");
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        bail!("provider.base_url must not include query strings or fragments");
    }
    Ok(())
}

pub(crate) fn validate_test(bench_dir: &Path, test: &TestCase) -> Result<()> {
    if test.schema_version > SCHEMA_VERSION {
        bail!(
            "test {} schema_version {} is newer than supported schema_version {}",
            test.id,
            test.schema_version,
            SCHEMA_VERSION
        );
    }
    if test.id.trim().is_empty() {
        bail!("test id must not be empty");
    }
    if test.question.trim().is_empty() {
        bail!("test {} question must not be empty", test.id);
    }
    if test.expected.is_empty() && !matches!(test.grader, Grader::ExpectRefusal) {
        bail!("test {} expected answers must not be empty", test.id);
    }
    if is_auto_context(&test.context) {
        return Ok(());
    }
    if let Err(error) = resolve_context_path(bench_dir, &test.context) {
        bail!("test {} context is invalid: {error}", test.id);
    }
    Ok(())
}

fn validate_unique_test_ids(tests: &[TestCase]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for test in tests {
        if !seen.insert(test.id.as_str()) {
            bail!("duplicate test id: {}", test.id);
        }
    }
    Ok(())
}

fn validate_request_log_path(path: &str) -> Result<()> {
    let path = Path::new(path);
    if path.is_absolute() {
        bail!("run.request_log_path must be relative and stay under reports/");
    }
    let mut components = path.components();
    if components.next() != Some(Component::Normal("reports".as_ref())) {
        bail!("run.request_log_path must stay under reports/");
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("run.request_log_path must not escape reports/");
    }
    Ok(())
}

pub(crate) fn resolve_context_path(bench_dir: &Path, context: &str) -> Result<Option<PathBuf>> {
    if looks_inline_context(context) {
        return Ok(None);
    }
    let bench_dir = bench_dir.canonicalize().with_context(|| {
        format!(
            "failed to resolve benchmark directory {}",
            bench_dir.display()
        )
    })?;
    let path = PathBuf::from(context);
    let candidate = if path.is_absolute() {
        path
    } else {
        bench_dir.join(path)
    };
    let resolved = candidate
        .canonicalize()
        .with_context(|| format!("context does not resolve to a file: {}", context))?;
    if !resolved.starts_with(&bench_dir) {
        bail!(
            "context file must stay under benchmark directory: {}",
            context
        );
    }
    Ok(Some(resolved))
}

fn looks_inline_context(context: &str) -> bool {
    context.contains('\n') || context.len() > 4096
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_valid_fixture(root: &Path) {
        fs::write(
            root.join("config.toml"),
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("contexts")).unwrap();
        fs::create_dir_all(root.join("manifests")).unwrap();
        fs::write(root.join("contexts/needle_context.txt"), "answer is A").unwrap();
        fs::write(
            root.join("manifests/needle.json"),
            r#"
{
  "schema_version": 1,
  "name": "needle",
  "token_count": 100,
  "seed": 7,
  "suites": [{
    "schema_version": 1,
    "id": "needle-100",
    "context": "contexts/needle_context.txt",
    "question": "q",
    "expected": ["a"],
    "grader": "Exact",
    "metadata": {"suite": "needle"}
  }]
}
"#,
        )
        .unwrap();
    }

    #[test]
    fn validate_accepts_generated_layout() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());

        let summary = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap();
        assert_eq!(summary.test_count, 1);
        assert_eq!(summary.config_model, "example-model");
        assert!(!summary.api_key_checked);
    }

    #[test]
    fn validate_rejects_missing_context_file() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::remove_file(tmp.path().join("contexts/needle_context.txt")).unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("context does not resolve"));
    }

    #[test]
    fn validate_rejects_nonexistent_benchmark_dir() {
        let error = validate_benchmark_dir("/nonexistent/path/12345", false).unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn validate_rejects_empty_base_url() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = ""
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("base_url"));
    }

    #[test]
    fn validate_rejects_invalid_base_url() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = "api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("absolute http(s) URL"));
    }

    #[test]
    fn validate_rejects_base_url_with_query_or_fragment() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = "https://api.example.test/v1?debug=true"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("query strings or fragments"));
    }

    #[test]
    fn validate_rejects_empty_model() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = ""
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("model"));
    }

    #[test]
    fn validate_rejects_zero_timeout() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"

[run]
request_timeout_secs = 0
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("request_timeout_secs"));
    }

    #[test]
    fn validate_rejects_zero_concurrency() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"

[run]
concurrency = 0
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("concurrency"));
    }

    #[test]
    fn validate_rejects_duplicate_test_ids() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("manifests/needle.json"),
            r#"
{
  "schema_version": 1,
  "name": "needle",
  "token_count": 100,
  "seed": 7,
  "suites": [
    {
      "schema_version": 1,
      "id": "duplicate",
      "context": "contexts/needle_context.txt",
      "question": "q1",
      "expected": ["a"],
      "grader": "Exact",
      "metadata": {"suite": "needle"}
    },
    {
      "schema_version": 1,
      "id": "duplicate",
      "context": "contexts/needle_context.txt",
      "question": "q2",
      "expected": ["a"],
      "grader": "Exact",
      "metadata": {"suite": "needle"}
    }
  ]
}
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("duplicate test id"));
    }

    #[test]
    fn validate_rejects_request_log_path_outside_reports() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("config.toml"),
            r#"
[provider]
base_url = "https://api.example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
model = "example-model"

[run]
request_log_path = "../http-log.jsonl"
"#,
        )
        .unwrap();

        let error = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap_err();
        assert!(error.to_string().contains("request_log_path"));
    }

    #[test]
    fn validate_accepts_expect_refusal_without_expected_answers() {
        let tmp = tempdir().unwrap();
        write_valid_fixture(tmp.path());
        fs::write(
            tmp.path().join("manifests/needle.json"),
            r#"
{
  "schema_version": 1,
  "name": "hallucination",
  "token_count": 100,
  "seed": 7,
  "suites": [{
    "schema_version": 1,
    "id": "hallucination-100",
    "context": "contexts/needle_context.txt",
    "question": "What is the missing code?",
    "expected": [],
    "grader": "ExpectRefusal",
    "metadata": {"suite": "hallucination"}
  }]
}
"#,
        )
        .unwrap();

        let summary = validate_benchmark_dir(tmp.path().to_str().unwrap(), false).unwrap();
        assert_eq!(summary.test_count, 1);
    }

    #[test]
    fn resolve_context_path_accepts_inline_content() {
        let tmp = tempdir().unwrap();
        assert_eq!(
            resolve_context_path(tmp.path(), "some inline text\nwith newline").unwrap(),
            None
        );
        let long_text = "x".repeat(5000);
        assert_eq!(resolve_context_path(tmp.path(), &long_text).unwrap(), None);
    }

    #[test]
    fn resolve_context_path_accepts_absolute_path_under_bench_dir() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("test.txt");
        fs::write(&file, "content").unwrap();
        let resolved = resolve_context_path(tmp.path(), &file.to_string_lossy()).unwrap();
        assert_eq!(resolved.unwrap(), file.canonicalize().unwrap());
    }

    #[test]
    fn resolve_context_path_rejects_paths_outside_bench_dir() {
        let tmp = tempdir().unwrap();
        let bench = tmp.path().join("bench");
        fs::create_dir_all(&bench).unwrap();
        let outside_file = tmp.path().join("secret.txt");
        fs::write(&outside_file, "do not read").unwrap();

        let absolute_error =
            resolve_context_path(&bench, &outside_file.to_string_lossy()).unwrap_err();
        assert!(absolute_error.to_string().contains("benchmark directory"));

        let relative_error = resolve_context_path(&bench, "../secret.txt").unwrap_err();
        assert!(relative_error.to_string().contains("benchmark directory"));
    }
}
