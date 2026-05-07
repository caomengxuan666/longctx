use crate::benchmark::{Config, TestCase, SCHEMA_VERSION};
use crate::runner::{read_config, read_tests};
use anyhow::{bail, Context, Result};
use std::env;
use std::path::{Path, PathBuf};

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
    validate_config(&config, check_api_key)?;

    let tests = read_tests(bench_path)?;
    if tests.is_empty() {
        bail!("no benchmark tests found in {}", bench_path.display());
    }

    for test in &tests {
        validate_test(bench_path, test)?;
    }

    Ok(ValidationSummary {
        test_count: tests.len(),
        config_model: config.provider.model,
        api_key_checked: check_api_key,
    })
}

fn validate_config(config: &Config, check_api_key: bool) -> Result<()> {
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

fn validate_test(bench_dir: &Path, test: &TestCase) -> Result<()> {
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
    if test.expected.is_empty() {
        bail!("test {} expected answers must not be empty", test.id);
    }
    if !context_is_resolvable(bench_dir, &test.context) {
        bail!(
            "test {} context does not resolve to a file: {}",
            test.id,
            test.context
        );
    }
    Ok(())
}

fn context_is_resolvable(bench_dir: &Path, context: &str) -> bool {
    if context.contains('\n') || context.len() > 4096 {
        return true;
    }

    let path = PathBuf::from(context);
    if path.is_absolute() {
        return path.exists();
    }
    bench_dir.join(&path).exists() || path.exists()
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
}
