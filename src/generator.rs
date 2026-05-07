use crate::benchmark::{Grader, SuiteManifest, TestCase};
use anyhow::{bail, Context, Result};
use rand::{seq::SliceRandom, Rng};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn generate(suite_type: &str, token_count: u64, out_dir: &str) -> Result<()> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output directory {out_dir}"))?;

    let suite_type = suite_type.to_ascii_lowercase();
    let manifest = match suite_type.as_str() {
        "needle" | "needlesuite" => build_needle_suite(token_count, out_dir)?,
        "multi-needle" | "multineedle" | "multi_needle" | "multineedlesuite" => {
            build_multi_needle_suite(token_count, out_dir)?
        }
        "conflict" | "conflictsuite" => build_conflict_suite(token_count, out_dir)?,
        other => bail!("unknown suite type '{other}', expected needle, multi-needle, or conflict"),
    };

    let manifest_path = Path::new(out_dir).join(format!("{}.json", manifest.name));
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&manifest_path, json)
        .with_context(|| format!("failed to write manifest {}", manifest_path.display()))?;
    Ok(())
}

fn build_needle_suite(token_count: u64, out_dir: &str) -> Result<SuiteManifest> {
    let secret = format!("ORCHID-{}", rand::thread_rng().gen_range(100_000..999_999));
    let needle = format!("Needle fact: the archive access code is {secret}.");
    let context_path = write_context(out_dir, "needle_context.txt", token_count, &[needle])?;

    let mut metadata = HashMap::new();
    metadata.insert("suite".to_string(), "needle".to_string());

    Ok(SuiteManifest {
        name: "needle".to_string(),
        token_count,
        suites: vec![TestCase {
            id: format!("needle-{token_count}"),
            context: context_path,
            question: "What is the archive access code? Answer with only the code.".to_string(),
            expected: vec![secret],
            grader: Grader::Exact,
            metadata,
        }],
    })
}

fn build_multi_needle_suite(token_count: u64, out_dir: &str) -> Result<SuiteManifest> {
    let mut rng = rand::thread_rng();
    let count = rng.gen_range(3..=8);
    let names = [
        "Aster", "Beryl", "Cygnus", "Dahlia", "Ember", "Fjord", "Garnet", "Helio",
    ];
    let mut facts = Vec::with_capacity(count);
    let mut expected = Vec::with_capacity(count);

    for (idx, name) in names.choose_multiple(&mut rng, count).enumerate() {
        let value = rng.gen_range(10..99);
        facts.push(format!("Needle fact {idx}: project {name} has checkpoint value {value}."));
        expected.push(format!("{name}:{value}"));
    }

    let context_path = write_context(out_dir, "multi_needle_context.txt", token_count, &facts)?;
    let expected_answer = expected.join(", ");

    let mut metadata = HashMap::new();
    metadata.insert("suite".to_string(), "multi-needle".to_string());
    metadata.insert("fact_count".to_string(), count.to_string());

    Ok(SuiteManifest {
        name: "multi_needle".to_string(),
        token_count,
        suites: vec![TestCase {
            id: format!("multi-needle-{token_count}-{count}"),
            context: context_path,
            question: "List each project and checkpoint value as Name:value pairs, separated by commas."
                .to_string(),
            expected: vec![expected_answer],
            grader: Grader::Regex(expected.join(r"\s*,\s*")),
            metadata,
        }],
    })
}

fn build_conflict_suite(token_count: u64, out_dir: &str) -> Result<SuiteManifest> {
    let correct = format!("TAU-{}", rand::thread_rng().gen_range(1000..9999));
    let old = format!("LAMBDA-{}", rand::thread_rng().gen_range(1000..9999));
    let other_scope = format!("KAPPA-{}", rand::thread_rng().gen_range(1000..9999));
    let facts = vec![
        format!("For the 2023 east-region audit, the routing key was {old}."),
        format!("For the 2024 west-region audit, the routing key was {other_scope}."),
        format!("For the 2024 east-region audit, the routing key is {correct}."),
    ];
    let context_path = write_context(out_dir, "conflict_context.txt", token_count, &facts)?;

    let mut metadata = HashMap::new();
    metadata.insert("suite".to_string(), "conflict".to_string());
    metadata.insert("scope".to_string(), "2024 east-region audit".to_string());

    Ok(SuiteManifest {
        name: "conflict".to_string(),
        token_count,
        suites: vec![TestCase {
            id: format!("conflict-{token_count}"),
            context: context_path,
            question: "What is the routing key for the 2024 east-region audit? Answer with only the key."
                .to_string(),
            expected: vec![correct],
            grader: Grader::Exact,
            metadata,
        }],
    })
}

fn write_context(out_dir: &str, file_name: &str, token_count: u64, facts: &[String]) -> Result<String> {
    let mut rng = rand::thread_rng();
    let fact_tokens: u64 = facts.iter().map(|f| estimate_tokens(f)).sum();
    let target = token_count.max(fact_tokens);
    let filler = "This paragraph is ordinary benchmark filler text about documents, meetings, logs, summaries, and archived notes. It contains no answer to the benchmark question.";
    let filler_tokens = estimate_tokens(filler).max(1);
    let repeats = target
        .saturating_sub(facts.len() as u64 * 16)
        .checked_div(filler_tokens)
        .unwrap_or(0)
        .max(facts.len() as u64 + 1);
    let repeats = usize::try_from(repeats).context("token count is too large to allocate context")?;
    let mut chunks = vec![filler.to_string(); repeats];

    let mut positions: Vec<usize> = (0..chunks.len()).collect();
    positions.shuffle(&mut rng);
    positions.truncate(facts.len());
    positions.sort_unstable();

    for (offset, (position, fact)) in positions.into_iter().zip(facts.iter()).enumerate() {
        chunks.insert(position + offset, fact.clone());
    }

    let path = Path::new(out_dir).join(file_name);
    fs::write(&path, chunks.join("\n\n"))
        .with_context(|| format!("failed to write context {}", path.display()))?;
    Ok(path_to_string(&path))
}

fn estimate_tokens(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

fn path_to_string(path: &PathBuf) -> String {
    path.to_string_lossy().into_owned()
}
