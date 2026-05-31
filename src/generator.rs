use crate::benchmark::{Grader, SuiteManifest, TestCase, SCHEMA_VERSION};
use crate::context_index::{build_context_index, write_context_index};
use crate::tokenizer::TokenCounter;
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
    let counter = TokenCounter::cl100k();
    let manifest = match suite_type.as_str() {
        "needle" | "needlesuite" => build_needle_suite(&mut rng, token_count, seed, &contexts_dir, &counter)?,
        "multi-needle" | "multineedle" | "multi_needle" | "multineedlesuite" => {
            build_multi_needle_suite(&mut rng, token_count, seed, &contexts_dir, &counter)?
        }
        "conflict" | "conflictsuite" => {
            build_conflict_suite(&mut rng, token_count, seed, &contexts_dir, &counter)?
        }
        "multi-hop" | "multihop" | "multi_hop" => {
            build_multi_hop_suite(&mut rng, token_count, seed, &contexts_dir, &counter)?
        }
        "order-dependent" | "orderdependent" | "order_dependent" => {
            build_order_dependent_suite(&mut rng, token_count, seed, &contexts_dir, &counter)?
        }
        "position-sweep" | "positionsweep" | "position_sweep" => {
            build_position_sweep_suite(&mut rng, token_count, seed, &contexts_dir, &counter)?
        }
        "hallucination" | "hallucinationsuite" => {
            build_hallucination_suite(&mut rng, token_count, seed, &contexts_dir, &counter)?
        }
        other => bail!("unknown suite type '{other}', expected needle, multi-needle, conflict, multi-hop, order-dependent, position-sweep, or hallucination"),
    };

    let manifest_path = manifests_dir.join(format!("{}.json", manifest.name));
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&manifest_path, json)
        .with_context(|| format!("failed to write manifest {}", manifest_path.display()))?;
    let index = build_context_index(out_dir)?;
    write_context_index(out_dir, &index)?;
    Ok(())
}

fn build_needle_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
    counter: &TokenCounter,
) -> Result<SuiteManifest> {
    let secret = format!("ORCHID-{}", rng.gen_range(100_000..999_999));
    let needle = format!("Needle fact: the archive access code is {secret}.");
    let context_path = write_context(
        rng,
        contexts_dir,
        "needle_context.txt",
        token_count,
        &[needle],
        counter,
        None,
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
    counter: &TokenCounter,
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
        counter,
        None,
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
    counter: &TokenCounter,
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
        counter,
        None,
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

fn build_multi_hop_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
    counter: &TokenCounter,
) -> Result<SuiteManifest> {
    let projects = [
        "Project Alpha",
        "Project Beta",
        "Project Gamma",
        "Project Delta",
        "Project Sigma",
        "Project Omega",
    ];
    let project = projects.choose(rng).unwrap();
    let person = format!("Agent-{}", rng.gen_range(100..999));
    let badge = format!("CODE-{}", rng.gen_range(1000..9999));

    let facts = vec![
        format!("{project}'s lead is {person}."),
        format!("{person}'s badge code is {badge}."),
    ];

    let context_path = write_context(
        rng,
        contexts_dir,
        "multi_hop_context.txt",
        token_count,
        &facts,
        counter,
        None,
    )?;

    let mut metadata = BTreeMap::new();
    metadata.insert("suite".to_string(), "multi-hop".to_string());
    metadata.insert("token_count".to_string(), token_count.to_string());
    metadata.insert("seed".to_string(), seed.to_string());

    Ok(SuiteManifest {
        schema_version: SCHEMA_VERSION,
        name: "multi_hop".to_string(),
        token_count,
        seed,
        suites: vec![TestCase {
            schema_version: SCHEMA_VERSION,
            id: format!("multi-hop-{token_count}"),
            context: context_path,
            question: format!(
                "What is the badge code of {project}'s lead? Answer with only the code."
            ),
            expected: vec![badge],
            grader: Grader::Exact,
            metadata,
        }],
    })
}

fn build_order_dependent_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
    counter: &TokenCounter,
) -> Result<SuiteManifest> {
    let step_count = rng.gen_range(3..=6u32);
    let target_step = rng.gen_range(1..=step_count);
    let param_names = [
        "temperature",
        "pressure",
        "voltage",
        "humidity",
        "frequency",
    ];
    let param = param_names.choose(rng).unwrap();

    let mut facts = Vec::new();
    let mut target_value = String::new();
    for step in 1..=step_count {
        let value = rng.gen_range(10..100);
        facts.push(format!("At step {step}, the {param} was {value}."));
        if step == target_step {
            target_value = value.to_string();
        }
    }

    let context_path = write_context(
        rng,
        contexts_dir,
        "order_dependent_context.txt",
        token_count,
        &facts,
        counter,
        None,
    )?;

    let mut metadata = BTreeMap::new();
    metadata.insert("suite".to_string(), "order-dependent".to_string());
    metadata.insert("token_count".to_string(), token_count.to_string());
    metadata.insert("seed".to_string(), seed.to_string());
    metadata.insert("step_count".to_string(), step_count.to_string());
    metadata.insert("target_step".to_string(), target_step.to_string());

    Ok(SuiteManifest {
        schema_version: SCHEMA_VERSION,
        name: "order_dependent".to_string(),
        token_count,
        seed,
        suites: vec![TestCase {
            schema_version: SCHEMA_VERSION,
            id: format!("order-dependent-{token_count}"),
            context: context_path,
            question: format!(
                "What was the {param} at step {target_step}? Answer with only the number."
            ),
            expected: vec![target_value],
            grader: Grader::Exact,
            metadata,
        }],
    })
}

fn build_position_sweep_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
    counter: &TokenCounter,
) -> Result<SuiteManifest> {
    let positions = [
        ("start", 0.05),
        ("early", 0.25),
        ("middle", 0.50),
        ("late", 0.75),
        ("end", 0.95),
    ];

    let mut test_cases = Vec::with_capacity(positions.len());
    for (label, pos) in &positions {
        let secret = format!("ORCHID-{}", rng.gen_range(100_000..999_999));
        let needle = format!("Needle fact: the archive access code is {secret}.");
        let file_name = format!("position_sweep_{label}.txt");
        let context_path = write_context(
            rng,
            contexts_dir,
            &file_name,
            token_count,
            &[needle],
            counter,
            Some(*pos),
        )?;

        let mut metadata = BTreeMap::new();
        metadata.insert("suite".to_string(), "position-sweep".to_string());
        metadata.insert("token_count".to_string(), token_count.to_string());
        metadata.insert("seed".to_string(), seed.to_string());
        metadata.insert("position".to_string(), format!("{pos:.2}"));
        metadata.insert("position_label".to_string(), label.to_string());

        test_cases.push(TestCase {
            schema_version: SCHEMA_VERSION,
            id: format!("position-sweep-{token_count}-{label}"),
            context: context_path,
            question: "What is the archive access code? Answer with only the code.".to_string(),
            expected: vec![secret],
            grader: Grader::Exact,
            metadata,
        });
    }

    Ok(SuiteManifest {
        schema_version: SCHEMA_VERSION,
        name: "position_sweep".to_string(),
        token_count,
        seed,
        suites: test_cases,
    })
}

fn build_hallucination_suite(
    rng: &mut StdRng,
    token_count: u64,
    seed: u64,
    contexts_dir: &Path,
    counter: &TokenCounter,
) -> Result<SuiteManifest> {
    let questions = [
        (
            "code",
            "What is the archive access code? Answer with only the code.",
        ),
        (
            "name",
            "What is the project lead's name? Answer with only the name.",
        ),
        (
            "date",
            "What date was the audit completed? Answer with only the date.",
        ),
    ];

    let mut test_cases = Vec::with_capacity(questions.len());
    for (label, question) in &questions {
        let file_name = format!("hallucination_{label}.txt");
        let context_path = write_context(
            rng,
            contexts_dir,
            &file_name,
            token_count,
            &[], // no facts — pure filler
            counter,
            None,
        )?;

        let mut metadata = BTreeMap::new();
        metadata.insert("suite".to_string(), "hallucination".to_string());
        metadata.insert("token_count".to_string(), token_count.to_string());
        metadata.insert("seed".to_string(), seed.to_string());

        test_cases.push(TestCase {
            schema_version: SCHEMA_VERSION,
            id: format!("hallucination-{token_count}-{label}"),
            context: context_path,
            question: question.to_string(),
            expected: vec![],
            grader: Grader::ExpectRefusal,
            metadata,
        });
    }

    Ok(SuiteManifest {
        schema_version: SCHEMA_VERSION,
        name: "hallucination".to_string(),
        token_count,
        seed,
        suites: test_cases,
    })
}

fn write_context(
    rng: &mut StdRng,
    contexts_dir: &Path,
    file_name: &str,
    token_count: u64,
    facts: &[String],
    counter: &TokenCounter,
    position: Option<f64>,
) -> Result<String> {
    let fact_tokens: u64 = facts.iter().map(|f| counter.count_tokens(f)).sum();
    let target = token_count.max(fact_tokens);
    let filler = "This paragraph is ordinary benchmark filler text about documents, meetings, logs, summaries, and archived notes. It contains no answer to the benchmark question.";
    let filler_tokens = counter.count_tokens(filler).max(1);
    let repeats = target
        .saturating_sub(facts.len() as u64 * 16)
        .checked_div(filler_tokens)
        .unwrap_or(0)
        .max(facts.len() as u64 + 1);
    let repeats =
        usize::try_from(repeats).context("token count is too large to allocate context")?;
    let mut chunks = vec![filler.to_string(); repeats];

    if let Some(pos) = position {
        // Insert all facts at the specified position (0.0 = start, 1.0 = end)
        let clamped = pos.clamp(0.0, 1.0);
        let insert_at = (clamped * chunks.len() as f64).round() as usize;
        let insert_at = insert_at.min(chunks.len());
        for (offset, fact) in facts.iter().enumerate() {
            chunks.insert(insert_at + offset, fact.clone());
        }
    } else {
        let mut positions: Vec<usize> = (0..chunks.len()).collect();
        positions.shuffle(rng);
        positions.truncate(facts.len());
        positions.sort_unstable();

        for (offset, (position, fact)) in positions.into_iter().zip(facts.iter()).enumerate() {
            chunks.insert(position + offset, fact.clone());
        }
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

    #[test]
    fn multi_needle_generation_is_reproducible() {
        let tmp_a = tempdir().unwrap();
        let tmp_b = tempdir().unwrap();

        generate(
            "multi-needle",
            100,
            Some(42),
            tmp_a.path().to_str().unwrap(),
        )
        .unwrap();
        generate(
            "multi-needle",
            100,
            Some(42),
            tmp_b.path().to_str().unwrap(),
        )
        .unwrap();

        let manifest_a =
            fs::read_to_string(tmp_a.path().join("manifests/multi_needle.json")).unwrap();
        let manifest_b =
            fs::read_to_string(tmp_b.path().join("manifests/multi_needle.json")).unwrap();
        assert_eq!(manifest_a, manifest_b);
    }

    #[test]
    fn conflict_generation_is_reproducible() {
        let tmp_a = tempdir().unwrap();
        let tmp_b = tempdir().unwrap();

        generate("conflict", 100, Some(42), tmp_a.path().to_str().unwrap()).unwrap();
        generate("conflict", 100, Some(42), tmp_b.path().to_str().unwrap()).unwrap();

        let manifest_a = fs::read_to_string(tmp_a.path().join("manifests/conflict.json")).unwrap();
        let manifest_b = fs::read_to_string(tmp_b.path().join("manifests/conflict.json")).unwrap();
        assert_eq!(manifest_a, manifest_b);
    }

    #[test]
    fn multi_hop_generation_is_reproducible() {
        let tmp_a = tempdir().unwrap();
        let tmp_b = tempdir().unwrap();

        generate("multi-hop", 100, Some(42), tmp_a.path().to_str().unwrap()).unwrap();
        generate("multi-hop", 100, Some(42), tmp_b.path().to_str().unwrap()).unwrap();

        let manifest_a = fs::read_to_string(tmp_a.path().join("manifests/multi_hop.json")).unwrap();
        let manifest_b = fs::read_to_string(tmp_b.path().join("manifests/multi_hop.json")).unwrap();
        assert_eq!(manifest_a, manifest_b);
    }

    #[test]
    fn order_dependent_generation_is_reproducible() {
        let tmp_a = tempdir().unwrap();
        let tmp_b = tempdir().unwrap();

        generate(
            "order-dependent",
            100,
            Some(42),
            tmp_a.path().to_str().unwrap(),
        )
        .unwrap();
        generate(
            "order-dependent",
            100,
            Some(42),
            tmp_b.path().to_str().unwrap(),
        )
        .unwrap();

        let manifest_a =
            fs::read_to_string(tmp_a.path().join("manifests/order_dependent.json")).unwrap();
        let manifest_b =
            fs::read_to_string(tmp_b.path().join("manifests/order_dependent.json")).unwrap();
        assert_eq!(manifest_a, manifest_b);
    }

    #[test]
    fn unknown_suite_type_returns_error() {
        let tmp = tempdir().unwrap();
        let err = generate("nonexistent", 100, Some(1), tmp.path().to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("unknown suite type"));
    }

    #[test]
    fn position_sweep_generation_is_reproducible() {
        let tmp_a = tempdir().unwrap();
        let tmp_b = tempdir().unwrap();

        generate(
            "position-sweep",
            100,
            Some(42),
            tmp_a.path().to_str().unwrap(),
        )
        .unwrap();
        generate(
            "position-sweep",
            100,
            Some(42),
            tmp_b.path().to_str().unwrap(),
        )
        .unwrap();

        let manifest_a =
            fs::read_to_string(tmp_a.path().join("manifests/position_sweep.json")).unwrap();
        let manifest_b =
            fs::read_to_string(tmp_b.path().join("manifests/position_sweep.json")).unwrap();
        assert_eq!(manifest_a, manifest_b);
    }

    #[test]
    fn position_sweep_generates_five_test_cases() {
        let tmp = tempdir().unwrap();
        generate(
            "position-sweep",
            100,
            Some(42),
            tmp.path().to_str().unwrap(),
        )
        .unwrap();

        let manifest =
            fs::read_to_string(tmp.path().join("manifests/position_sweep.json")).unwrap();
        let parsed: SuiteManifest = serde_json::from_str(&manifest).unwrap();
        assert_eq!(parsed.suites.len(), 5);

        let labels: Vec<&str> = parsed
            .suites
            .iter()
            .filter_map(|t| t.metadata.get("position_label").map(|s| s.as_str()))
            .collect();
        assert_eq!(labels, vec!["start", "early", "middle", "late", "end"]);
    }

    #[test]
    fn hallucination_generation_is_reproducible() {
        let tmp_a = tempdir().unwrap();
        let tmp_b = tempdir().unwrap();

        generate(
            "hallucination",
            100,
            Some(42),
            tmp_a.path().to_str().unwrap(),
        )
        .unwrap();
        generate(
            "hallucination",
            100,
            Some(42),
            tmp_b.path().to_str().unwrap(),
        )
        .unwrap();

        let manifest_a =
            fs::read_to_string(tmp_a.path().join("manifests/hallucination.json")).unwrap();
        let manifest_b =
            fs::read_to_string(tmp_b.path().join("manifests/hallucination.json")).unwrap();
        assert_eq!(manifest_a, manifest_b);
    }

    #[test]
    fn hallucination_generates_three_test_cases() {
        let tmp = tempdir().unwrap();
        generate("hallucination", 100, Some(42), tmp.path().to_str().unwrap()).unwrap();

        let manifest = fs::read_to_string(tmp.path().join("manifests/hallucination.json")).unwrap();
        let parsed: SuiteManifest = serde_json::from_str(&manifest).unwrap();
        assert_eq!(parsed.suites.len(), 3);

        for test in &parsed.suites {
            assert!(test.expected.is_empty());
            assert!(matches!(test.grader, Grader::ExpectRefusal));
        }
    }
}
