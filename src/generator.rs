use crate::benchmark::{Grader, SuiteManifest, TestCase, SCHEMA_VERSION};
use anyhow::{bail, Context, Result};
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub fn generate(
    suite_type: &str,
    token_count: u64,
    seed: Option<u64>,
    out_dir: &str,
) -> Result<()> {
    let out_dir = Path::new(out_dir);
    let manifests_dir = out_dir.join("manifests");
    let contexts_dir = out_dir.join("contexts");
    fs::create_dir_all(&manifests_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            manifests_dir.display()
        )
    })?;
    fs::create_dir_all(&contexts_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            contexts_dir.display()
        )
    })?;

    let suite_type = suite_type.to_ascii_lowercase();
    let seed = seed.unwrap_or_else(|| rand::thread_rng().gen());
    let mut rng = StdRng::seed_from_u64(seed);
    let manifest = match suite_type.as_str() {
        "needle" | "needlesuite" => build_needle_suite(&mut rng, token_count, seed, &contexts_dir)?,
        "multi-needle" | "multineedle" | "multi_needle" | "multineedlesuite" => {
            build_multi_needle_suite(&mut rng, token_count, seed, &contexts_dir)?
        }
        "conflict" | "conflictsuite" => {
            build_conflict_suite(&mut rng, token_count, seed, &contexts_dir)?
        }
        other => bail!("unknown suite type '{other}', expected needle, multi-needle, or conflict"),
    };

    let manifest_path = manifests_dir.join(format!("{}.json", manifest.name));
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&manifest_path, json)
        .with_context(|| format!("failed to write manifest {}", manifest_path.display()))?;
    Ok(())
}

fn build_needle_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
) -> Result<SuiteManifest> {
    let secret = format!("ORCHID-{}", rng.gen_range(100_000..999_999));
    let needle = format!("Needle fact: the archive access code is {secret}.");
    let context_path = write_context(
        rng,
        contexts_dir,
        "needle_context.txt",
        token_count,
        &[needle],
    )?;

    let mut metadata = BTreeMap::new();
    metadata.insert("suite".to_string(), "needle".to_string());
    metadata.insert("token_count".to_string(), token_count.to_string());
    metadata.insert("seed".to_string(), seed.to_string());

    Ok(SuiteManifest {
        schema_version: SCHEMA_VERSION,
        name: "needle".to_string(),
        token_count,
        seed,
        suites: vec![TestCase {
            schema_version: SCHEMA_VERSION,
            id: format!("needle-{token_count}"),
            context: context_path,
            question: "What is the archive access code? Answer with only the code.".to_string(),
            expected: vec![secret],
            grader: Grader::Exact,
            metadata,
        }],
    })
}

fn build_multi_needle_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
) -> Result<SuiteManifest> {
    let count = rng.gen_range(3..=8);
    let names = [
        "Aster", "Beryl", "Cygnus", "Dahlia", "Ember", "Fjord", "Garnet", "Helio",
    ];
    let mut facts = Vec::with_capacity(count);
    let mut expected = Vec::with_capacity(count);

    for (idx, name) in names.choose_multiple(rng, count).enumerate() {
        let value = rng.gen_range(10..99);
        facts.push(format!(
            "Needle fact {idx}: project {name} has checkpoint value {value}."
        ));
        expected.push(format!("{name}:{value}"));
    }

    let context_path = write_context(
        rng,
        contexts_dir,
        "multi_needle_context.txt",
        token_count,
        &facts,
    )?;

    let mut metadata = BTreeMap::new();
    metadata.insert("suite".to_string(), "multi-needle".to_string());
    metadata.insert("token_count".to_string(), token_count.to_string());
    metadata.insert("fact_count".to_string(), count.to_string());
    metadata.insert("seed".to_string(), seed.to_string());

    Ok(SuiteManifest {
        schema_version: SCHEMA_VERSION,
        name: "multi_needle".to_string(),
        token_count,
        seed,
        suites: vec![TestCase {
            schema_version: SCHEMA_VERSION,
            id: format!("multi-needle-{token_count}-{count}"),
            context: context_path,
            question:
                "List each project and checkpoint value as Name:value pairs, separated by commas."
                    .to_string(),
            expected,
            grader: Grader::Set,
            metadata,
        }],
    })
}

fn build_conflict_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
) -> Result<SuiteManifest> {
    let correct = format!("TAU-{}", rng.gen_range(1000..9999));
    let old = format!("LAMBDA-{}", rng.gen_range(1000..9999));
    let other_scope = format!("KAPPA-{}", rng.gen_range(1000..9999));
    let facts = vec![
        format!("For the 2023 east-region audit, the routing key was {old}."),
        format!("For the 2024 west-region audit, the routing key was {other_scope}."),
        format!("For the 2024 east-region audit, the routing key is {correct}."),
    ];
    let context_path = write_context(
        rng,
        contexts_dir,
        "conflict_context.txt",
        token_count,
        &facts,
    )?;

    let mut metadata = BTreeMap::new();
    metadata.insert("suite".to_string(), "conflict".to_string());
    metadata.insert("token_count".to_string(), token_count.to_string());
    metadata.insert("scope".to_string(), "2024 east-region audit".to_string());
    metadata.insert("seed".to_string(), seed.to_string());

    Ok(SuiteManifest {
        schema_version: SCHEMA_VERSION,
        name: "conflict".to_string(),
        token_count,
        seed,
        suites: vec![TestCase {
            schema_version: SCHEMA_VERSION,
            id: format!("conflict-{token_count}"),
            context: context_path,
            question:
                "What is the routing key for the 2024 east-region audit? Answer with only the key."
                    .to_string(),
            expected: vec![correct],
            grader: Grader::Exact,
            metadata,
        }],
    })
}

fn write_context(
    rng: &mut StdRng,
    contexts_dir: &Path,
    file_name: &str,
    token_count: u64,
    facts: &[String],
) -> Result<String> {
    let fact_tokens: u64 = facts.iter().map(|f| estimate_tokens(f)).sum();
    let target = token_count.max(fact_tokens);
    let filler = "This paragraph is ordinary benchmark filler text about documents, meetings, logs, summaries, and archived notes. It contains no answer to the benchmark question.";
    let filler_tokens = estimate_tokens(filler).max(1);
    let repeats = target
        .saturating_sub(facts.len() as u64 * 16)
        .checked_div(filler_tokens)
        .unwrap_or(0)
        .max(facts.len() as u64 + 1);
    let repeats =
        usize::try_from(repeats).context("token count is too large to allocate context")?;
    let mut chunks = vec![filler.to_string(); repeats];

    let mut positions: Vec<usize> = (0..chunks.len()).collect();
    positions.shuffle(rng);
    positions.truncate(facts.len());
    positions.sort_unstable();

    for (offset, (position, fact)) in positions.into_iter().zip(facts.iter()).enumerate() {
        chunks.insert(position + offset, fact.clone());
    }

    let path = contexts_dir.join(file_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create context directory {}", parent.display()))?;
    }
    fs::write(&path, chunks.join("\n\n"))
        .with_context(|| format!("failed to write context {}", path.display()))?;
    let relative_path = Path::new("contexts").join(file_name);
    Ok(relative_path_to_string(&relative_path))
}

fn estimate_tokens(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

fn relative_path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::SuiteManifest;
    use tempfile::tempdir;

    #[test]
    fn generation_is_reproducible_for_the_same_seed() {
        let tmp_a = tempdir().unwrap();
        let tmp_b = tempdir().unwrap();

        generate("needle", 100, Some(42), tmp_a.path().to_str().unwrap()).unwrap();
        generate("needle", 100, Some(42), tmp_b.path().to_str().unwrap()).unwrap();

        let manifest_a = fs::read_to_string(tmp_a.path().join("manifests/needle.json")).unwrap();
        let manifest_b = fs::read_to_string(tmp_b.path().join("manifests/needle.json")).unwrap();

        let parsed_a: SuiteManifest = serde_json::from_str(&manifest_a).unwrap();
        let parsed_b: SuiteManifest = serde_json::from_str(&manifest_b).unwrap();
        assert_eq!(parsed_a.seed, 42);
        assert_eq!(parsed_a.name, parsed_b.name);
        assert_eq!(parsed_a.suites[0].expected, parsed_b.suites[0].expected);
        assert_eq!(manifest_a, manifest_b);
    }
}
