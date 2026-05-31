use crate::benchmark::{
    RoutingCandidate, RoutingDecision, SuiteManifest, TestCase, SCHEMA_VERSION,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

pub const CONTEXT_INDEX_FILE: &str = "context.index.json";
pub const AUTO_CONTEXT_VALUE: &str = "auto";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextIndex {
    #[serde(default = "crate::benchmark::default_schema_version")]
    pub schema_version: u32,
    pub bench_id: String,
    pub created_at_unix_ms: u64,
    #[serde(default)]
    pub contexts: Vec<ContextIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextIndexEntry {
    pub context_id: String,
    pub path: String,
    pub sha256: String,
    pub suite: String,
    pub token_count: u64,
    pub seed: u64,
    #[serde(default)]
    pub source_manifests: Vec<String>,
    #[serde(default)]
    pub test_ids: Vec<String>,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub safe_anchors: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

pub fn is_auto_context(context: &str) -> bool {
    context.trim().eq_ignore_ascii_case(AUTO_CONTEXT_VALUE)
}

pub fn load_or_build_context_index(bench_dir: &Path) -> Result<ContextIndex> {
    let index_path = bench_dir.join(CONTEXT_INDEX_FILE);
    if index_path.exists() {
        return read_context_index(&index_path);
    }
    let index = build_context_index(bench_dir)?;
    write_context_index(bench_dir, &index)?;
    Ok(index)
}

pub fn load_or_build_context_index_in_memory(bench_dir: &Path) -> Result<ContextIndex> {
    let index_path = bench_dir.join(CONTEXT_INDEX_FILE);
    if index_path.exists() {
        return read_context_index(&index_path);
    }
    build_context_index(bench_dir)
}

pub fn read_context_index(path: &Path) -> Result<ContextIndex> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read context index {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse context index {}", path.display()))
}

pub fn write_context_index(bench_dir: &Path, index: &ContextIndex) -> Result<()> {
    let path = bench_dir.join(CONTEXT_INDEX_FILE);
    let json = serde_json::to_string_pretty(index)?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))
}

pub fn build_context_index(bench_dir: &Path) -> Result<ContextIndex> {
    build_context_index_with_metadata(bench_dir, unix_ms_now(), bench_id_from_path(bench_dir))
}

pub fn build_reproducible_context_index(bench_dir: &Path) -> Result<ContextIndex> {
    build_context_index_with_metadata(bench_dir, 0, "bench".to_string())
}

fn build_context_index_with_metadata(
    bench_dir: &Path,
    created_at_unix_ms: u64,
    bench_id: String,
) -> Result<ContextIndex> {
    let manifests = read_manifests(bench_dir)?;
    let mut contexts: BTreeMap<String, ContextIndexEntry> = BTreeMap::new();

    for (manifest_path, manifest) in manifests {
        let source_manifest = relative_path(bench_dir, &manifest_path);
        let manifest_name = manifest.name.clone();
        for test in manifest.suites {
            if is_auto_context(&test.context) || looks_inline_context(&test.context) {
                continue;
            }

            let relative_context = normalize_relative_context_path(bench_dir, &test.context);
            let absolute_context = bench_dir.join(&relative_context);
            if !absolute_context.exists() {
                continue;
            }

            let suite = suite_for_index(&manifest_name, &test);
            let context_id = context_id_from_path(&relative_context);
            let entry =
                contexts
                    .entry(relative_context.clone())
                    .or_insert_with(|| ContextIndexEntry {
                        context_id,
                        path: relative_context.clone(),
                        sha256: String::new(),
                        suite: suite.clone(),
                        token_count: manifest.token_count,
                        seed: manifest.seed,
                        source_manifests: Vec::new(),
                        test_ids: Vec::new(),
                        title: context_title(&suite),
                        summary: context_summary(&suite),
                        keywords: Vec::new(),
                        safe_anchors: Vec::new(),
                        metadata: BTreeMap::new(),
                    });

            entry.source_manifests.push(source_manifest.clone());
            entry.test_ids.push(test.id.clone());
            entry.metadata.extend(router_safe_metadata(&test.metadata));
            entry
                .keywords
                .extend(keywords_for_test(&manifest_name, &suite, &test));
        }
    }

    let mut entries = contexts.into_values().collect::<Vec<_>>();
    for entry in &mut entries {
        let path = bench_dir.join(&entry.path);
        entry.sha256 = sha256_file(&path)?;
        entry.source_manifests.sort();
        entry.source_manifests.dedup();
        entry.test_ids.sort();
        entry.test_ids.dedup();
        entry.keywords = unique_sorted(entry.keywords.drain(..));
    }
    entries.sort_by(|a, b| a.context_id.cmp(&b.context_id));

    Ok(ContextIndex {
        schema_version: SCHEMA_VERSION,
        bench_id,
        created_at_unix_ms,
        contexts: entries,
    })
}

fn bench_id_from_path(bench_dir: &Path) -> String {
    bench_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bench")
        .to_string()
}

pub fn route_context(test: &TestCase, index: &ContextIndex) -> RoutingDecision {
    let mut candidates = index
        .contexts
        .iter()
        .map(|entry| score_candidate(test, entry))
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.context_id.cmp(&b.context_id))
    });
    candidates.truncate(5);

    let Some(best) = candidates.first().cloned() else {
        return RoutingDecision {
            schema_version: SCHEMA_VERSION,
            test_id: test.id.clone(),
            selected_context_id: None,
            selected_context_path: None,
            method: "none".to_string(),
            status: "error".to_string(),
            confidence: 0.0,
            candidates,
            llm_router_used: false,
            input_tokens: 0,
            output_tokens: 0,
            latency_ms: 0,
            reason: Some("empty_context_index".to_string()),
        };
    };

    let second_score = candidates.get(1).map_or(0.0, |candidate| candidate.score);
    let confidence = confidence(best.score, second_score, candidates.len());
    let accepted = best.score > 0.0 && confidence >= 0.55;
    let method = routing_method(&best);

    RoutingDecision {
        schema_version: SCHEMA_VERSION,
        test_id: test.id.clone(),
        selected_context_id: accepted.then(|| best.context_id.clone()),
        selected_context_path: accepted.then(|| best.path.clone()),
        method,
        status: if accepted { "selected" } else { "ambiguous" }.to_string(),
        confidence,
        candidates,
        llm_router_used: false,
        input_tokens: 0,
        output_tokens: 0,
        latency_ms: 0,
        reason: if accepted {
            Some("local_router_selected_top_candidate".to_string())
        } else if best.score > 0.0 && (best.score - second_score).abs() < f64::EPSILON {
            Some("local_router_top_candidates_tied".to_string())
        } else {
            Some("local_router_confidence_below_threshold".to_string())
        },
    }
}

pub fn validate_context_index(bench_dir: &Path, index: &ContextIndex) -> Result<()> {
    if index.schema_version > SCHEMA_VERSION {
        bail!(
            "context index schema_version {} is newer than supported schema_version {}",
            index.schema_version,
            SCHEMA_VERSION
        );
    }

    let bench_root = bench_dir
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", bench_dir.display()))?;

    for entry in &index.contexts {
        if entry.context_id.trim().is_empty() {
            bail!("context index entry has empty context_id");
        }
        let entry_path = Path::new(&entry.path);
        if entry_path.is_absolute() {
            bail!("indexed context path must be relative: {}", entry.path);
        }
        let path = bench_dir.join(&entry.path);
        if !path.exists() {
            bail!("indexed context does not exist: {}", entry.path);
        }
        let canonical_path = path
            .canonicalize()
            .with_context(|| format!("failed to canonicalize indexed context {}", entry.path))?;
        if !canonical_path.starts_with(&bench_root) {
            bail!(
                "indexed context escapes benchmark directory: {}",
                entry.path
            );
        }
        let actual = sha256_file(&path)?;
        if actual != entry.sha256 {
            bail!(
                "indexed context hash mismatch for {}: expected {}, got {}",
                entry.path,
                entry.sha256,
                actual
            );
        }
    }

    Ok(())
}

fn read_manifests(bench_dir: &Path) -> Result<Vec<(PathBuf, SuiteManifest)>> {
    let search_root = if bench_dir.join("manifests").is_dir() {
        bench_dir.join("manifests")
    } else {
        bench_dir.to_path_buf()
    };

    let mut manifests = Vec::new();
    for entry in WalkDir::new(&search_root).min_depth(1) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        if is_generated_json_artifact(path) {
            continue;
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest {}", path.display()))?;
        if let Ok(manifest) = serde_json::from_str::<SuiteManifest>(&text) {
            manifests.push((path.to_path_buf(), manifest));
        } else if let Ok(test) = serde_json::from_str::<TestCase>(&text) {
            manifests.push((
                path.to_path_buf(),
                suite_manifest_from_flat_test(path, test),
            ));
        }
    }
    manifests.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(manifests)
}

fn suite_manifest_from_flat_test(path: &Path, test: TestCase) -> SuiteManifest {
    let name = test
        .metadata
        .get("suite")
        .cloned()
        .unwrap_or_else(|| context_id_from_path(&path.to_string_lossy()));
    let token_count = test
        .metadata
        .get("token_count")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let seed = test
        .metadata
        .get("seed")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    SuiteManifest {
        schema_version: test.schema_version,
        name,
        token_count,
        seed,
        suites: vec![test],
    }
}

fn suite_for_index(manifest_name: &str, test: &TestCase) -> String {
    test.metadata
        .get("suite")
        .cloned()
        .unwrap_or_else(|| manifest_name.to_string())
}

fn score_candidate(test: &TestCase, entry: &ContextIndexEntry) -> RoutingCandidate {
    let mut score = 0.0;
    let mut reasons = Vec::new();
    let test_suite = test.metadata.get("suite").map(String::as_str);
    if test_suite == Some(entry.suite.as_str()) {
        score += 5.0;
        reasons.push("suite_match".to_string());
    }

    if let Some(token_count) = test
        .metadata
        .get("token_count")
        .and_then(|value| value.parse::<u64>().ok())
    {
        if token_count == entry.token_count {
            score += 2.0;
            reasons.push("token_count_match".to_string());
        }
    }

    let normalized_id = normalize_token(&test.id);
    for signal in [entry.context_id.as_str(), entry.suite.as_str()] {
        let signal = normalize_token(signal);
        if !signal.is_empty() && normalized_id.contains(&signal) {
            score += 2.0;
            reasons.push("id_prefix_match".to_string());
            break;
        }
    }

    let query_terms = tokenize_query(&format!("{} {}", test.id, test.question));
    let entry_terms = entry.keywords.iter().cloned().collect::<BTreeSet<_>>();
    let mut lexical_hits = 0;
    for term in &query_terms {
        if entry_terms.contains(term) {
            lexical_hits += 1;
        }
    }
    if lexical_hits > 0 {
        score += (lexical_hits as f64 * 0.5).min(4.0);
        reasons.push("keyword_overlap".to_string());
    }

    RoutingCandidate {
        context_id: entry.context_id.clone(),
        path: entry.path.clone(),
        score,
        reasons,
    }
}

fn confidence(top_score: f64, second_score: f64, candidate_count: usize) -> f64 {
    if top_score <= 0.0 {
        return 0.0;
    }
    if candidate_count <= 1 {
        return 1.0;
    }
    let margin = (top_score - second_score).max(0.0);
    let margin_factor = (margin / top_score.max(1.0)).clamp(0.0, 1.0);
    (0.35 + (0.65 * margin_factor)).clamp(0.0, 1.0)
}

fn routing_method(candidate: &RoutingCandidate) -> String {
    let has_metadata = candidate
        .reasons
        .iter()
        .any(|reason| reason == "suite_match" || reason == "token_count_match");
    let has_lexical = candidate
        .reasons
        .iter()
        .any(|reason| reason == "keyword_overlap" || reason == "id_prefix_match");
    match (has_metadata, has_lexical) {
        (true, true) => "hybrid",
        (true, false) => "metadata",
        (false, true) => "lexical",
        (false, false) => "none",
    }
    .to_string()
}

fn looks_inline_context(context: &str) -> bool {
    context.contains('\n') || context.len() > 4096
}

fn normalize_relative_context_path(bench_dir: &Path, context: &str) -> String {
    let path = PathBuf::from(context);
    if path.is_absolute() {
        return relative_path(bench_dir, &path);
    }
    path.to_string_lossy().replace('\\', "/")
}

fn relative_path(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn context_id_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(path)
        .replace('\\', "/")
}

fn context_title(suite: &str) -> String {
    format!("{} context", suite.replace(['_', '-'], " "))
}

fn context_summary(suite: &str) -> String {
    match suite {
        "needle" => "Single hidden fact retrieval task.",
        "multi-needle" | "multi_needle" => "Multiple hidden facts that must be returned together.",
        "conflict" => "Conflicting scoped facts where the current scoped answer must be selected.",
        "multi-hop" | "multi_hop" => "Facts requiring composition across multiple statements.",
        "order-dependent" | "order_dependent" => "Ordered facts where sequence matters.",
        "position-sweep" | "position_sweep" => {
            "Needle retrieval cases at different context positions."
        }
        "hallucination" => "Filler context with questions about absent facts.",
        _ => "Benchmark context descriptor.",
    }
    .to_string()
}

fn keywords_for_test(manifest_name: &str, suite: &str, test: &TestCase) -> Vec<String> {
    let mut keywords = Vec::new();
    keywords.extend(tokenize_query(manifest_name));
    keywords.extend(tokenize_query(suite));
    keywords.extend(tokenize_query(&test.id));
    keywords.extend(tokenize_query(&test.question));
    for (key, value) in &test.metadata {
        if is_router_safe_metadata_pair(key, value) {
            keywords.extend(tokenize_query(key));
            keywords.extend(tokenize_query(value));
        }
    }
    keywords
}

fn router_safe_metadata(metadata: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    metadata
        .iter()
        .filter(|(key, value)| is_router_safe_metadata_pair(key, value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn is_router_safe_metadata_pair(key: &str, value: &str) -> bool {
    is_router_safe_metadata_key(key) && !looks_like_answer_signal(value)
}

fn is_router_safe_metadata_key(key: &str) -> bool {
    let normalized = normalize_token(key);
    ![
        "answer",
        "answers",
        "expected",
        "expectedanswer",
        "secret",
        "secrets",
        "password",
        "passcode",
        "code",
        "value",
        "targetvalue",
    ]
    .iter()
    .any(|unsafe_key| normalized.contains(unsafe_key))
}

fn looks_like_answer_signal(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 6 {
        return false;
    }
    let has_digit = trimmed.chars().any(|ch| ch.is_ascii_digit());
    let has_upper = trimmed.chars().any(|ch| ch.is_ascii_uppercase());
    let has_separator = trimmed.contains('-') || trimmed.contains('_') || trimmed.contains(':');
    has_digit && has_upper && has_separator
}

fn tokenize_query(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize_token)
        .filter(|token| token.len() >= 3)
        .collect()
}

fn normalize_token(text: &str) -> String {
    text.to_ascii_lowercase().replace(['_', '-'], "")
}

fn unique_sorted(values: impl Iterator<Item = String>) -> Vec<String> {
    values
        .filter(|value| !value.trim().is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn is_generated_json_artifact(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("context.index.json" | "results.json" | "run.json")
    )
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::Grader;
    use tempfile::tempdir;

    #[test]
    fn builds_context_index_from_manifest_layout() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("contexts")).unwrap();
        fs::create_dir_all(tmp.path().join("manifests")).unwrap();
        fs::write(
            tmp.path().join("contexts/needle_context.txt"),
            "needle text",
        )
        .unwrap();
        fs::write(
            tmp.path().join("manifests/needle.json"),
            r#"{
  "schema_version": 1,
  "name": "needle",
  "token_count": 100,
  "seed": 7,
  "suites": [{
    "schema_version": 1,
    "id": "needle-100",
    "context": "contexts/needle_context.txt",
    "question": "What is the archive access code?",
    "expected": ["ORCHID-1"],
    "grader": "Exact",
    "metadata": {"suite": "needle", "token_count": "100"}
  }]
}"#,
        )
        .unwrap();

        let index = build_context_index(tmp.path()).unwrap();
        assert_eq!(index.contexts.len(), 1);
        assert_eq!(index.contexts[0].context_id, "needle_context");
        assert_eq!(index.contexts[0].suite, "needle");
        assert!(index.contexts[0].keywords.contains(&"archive".to_string()));
        validate_context_index(tmp.path(), &index).unwrap();
    }

    #[test]
    fn routes_auto_context_by_metadata_and_keywords() {
        let index = ContextIndex {
            schema_version: SCHEMA_VERSION,
            bench_id: "bench".to_string(),
            created_at_unix_ms: 1,
            contexts: vec![
                ContextIndexEntry {
                    context_id: "needle_context".to_string(),
                    path: "contexts/needle_context.txt".to_string(),
                    sha256: "hash".to_string(),
                    suite: "needle".to_string(),
                    token_count: 100,
                    seed: 7,
                    source_manifests: vec![],
                    test_ids: vec![],
                    title: "Needle context".to_string(),
                    summary: "Needle".to_string(),
                    keywords: vec![
                        "archive".to_string(),
                        "access".to_string(),
                        "code".to_string(),
                    ],
                    safe_anchors: vec![],
                    metadata: BTreeMap::new(),
                },
                ContextIndexEntry {
                    context_id: "conflict_context".to_string(),
                    path: "contexts/conflict_context.txt".to_string(),
                    sha256: "hash".to_string(),
                    suite: "conflict".to_string(),
                    token_count: 100,
                    seed: 7,
                    source_manifests: vec![],
                    test_ids: vec![],
                    title: "Conflict context".to_string(),
                    summary: "Conflict".to_string(),
                    keywords: vec![
                        "routing".to_string(),
                        "audit".to_string(),
                        "key".to_string(),
                    ],
                    safe_anchors: vec![],
                    metadata: BTreeMap::new(),
                },
            ],
        };
        let mut metadata = BTreeMap::new();
        metadata.insert("suite".to_string(), "needle".to_string());
        metadata.insert("token_count".to_string(), "100".to_string());
        let test = TestCase {
            schema_version: SCHEMA_VERSION,
            id: "needle-100".to_string(),
            context: AUTO_CONTEXT_VALUE.to_string(),
            question: "What is the archive access code?".to_string(),
            expected: vec!["ORCHID-1".to_string()],
            grader: Grader::Exact,
            metadata,
        };

        let decision = route_context(&test, &index);
        assert_eq!(decision.status, "selected");
        assert_eq!(
            decision.selected_context_path.as_deref(),
            Some("contexts/needle_context.txt")
        );
        assert!(decision.confidence >= 0.55);
    }

    #[test]
    fn route_context_rejects_tied_top_candidates_as_ambiguous() {
        let index = ContextIndex {
            schema_version: SCHEMA_VERSION,
            bench_id: "bench".to_string(),
            created_at_unix_ms: 1,
            contexts: vec![
                ContextIndexEntry {
                    context_id: "a_context".to_string(),
                    path: "contexts/a.txt".to_string(),
                    sha256: "hash".to_string(),
                    suite: "needle".to_string(),
                    token_count: 100,
                    seed: 7,
                    source_manifests: vec![],
                    test_ids: vec![],
                    title: "A".to_string(),
                    summary: "A".to_string(),
                    keywords: vec!["archive".to_string(), "access".to_string()],
                    safe_anchors: vec![],
                    metadata: BTreeMap::new(),
                },
                ContextIndexEntry {
                    context_id: "b_context".to_string(),
                    path: "contexts/b.txt".to_string(),
                    sha256: "hash".to_string(),
                    suite: "needle".to_string(),
                    token_count: 100,
                    seed: 7,
                    source_manifests: vec![],
                    test_ids: vec![],
                    title: "B".to_string(),
                    summary: "B".to_string(),
                    keywords: vec!["archive".to_string(), "access".to_string()],
                    safe_anchors: vec![],
                    metadata: BTreeMap::new(),
                },
            ],
        };
        let mut metadata = BTreeMap::new();
        metadata.insert("suite".to_string(), "needle".to_string());
        metadata.insert("token_count".to_string(), "100".to_string());
        let test = TestCase {
            schema_version: SCHEMA_VERSION,
            id: "needle-100".to_string(),
            context: AUTO_CONTEXT_VALUE.to_string(),
            question: "What is the archive access value?".to_string(),
            expected: vec!["ORCHID-1".to_string()],
            grader: Grader::Exact,
            metadata,
        };

        let decision = route_context(&test, &index);
        assert_eq!(decision.status, "ambiguous");
        assert_eq!(decision.selected_context_path, None);
        assert_eq!(
            decision.reason.as_deref(),
            Some("local_router_top_candidates_tied")
        );
    }

    #[test]
    fn context_index_does_not_use_answer_like_metadata_for_routing() {
        let mut metadata = BTreeMap::new();
        metadata.insert("suite".to_string(), "needle".to_string());
        metadata.insert("expected_answer".to_string(), "ORCHID-1".to_string());
        metadata.insert("secret_code".to_string(), "PINE-9".to_string());
        metadata.insert("label".to_string(), "LOTUS-7".to_string());
        metadata.insert("scope".to_string(), "east audit".to_string());
        let test = TestCase {
            schema_version: SCHEMA_VERSION,
            id: "needle-100".to_string(),
            context: "auto".to_string(),
            question: "What is the archive access code?".to_string(),
            expected: vec!["ORCHID-1".to_string()],
            grader: Grader::Exact,
            metadata,
        };

        let keywords = keywords_for_test("needle", "needle", &test);
        assert!(!keywords.contains(&"orchid".to_string()));
        assert!(!keywords.contains(&"pine".to_string()));
        assert!(!keywords.contains(&"lotus".to_string()));
        assert!(keywords.contains(&"east".to_string()));
    }

    #[test]
    fn validate_context_index_rejects_paths_outside_benchmark_dir() {
        let tmp = tempdir().unwrap();
        let outside = tmp.path().join("outside.txt");
        fs::write(&outside, "outside").unwrap();
        let bench = tmp.path().join("bench");
        fs::create_dir_all(&bench).unwrap();
        let index = ContextIndex {
            schema_version: SCHEMA_VERSION,
            bench_id: "bench".to_string(),
            created_at_unix_ms: 1,
            contexts: vec![ContextIndexEntry {
                context_id: "outside".to_string(),
                path: "../outside.txt".to_string(),
                sha256: sha256_file(&outside).unwrap(),
                suite: "needle".to_string(),
                token_count: 100,
                seed: 7,
                source_manifests: vec![],
                test_ids: vec![],
                title: "outside".to_string(),
                summary: "outside".to_string(),
                keywords: vec![],
                safe_anchors: vec![],
                metadata: BTreeMap::new(),
            }],
        };

        let error = validate_context_index(&bench, &index).unwrap_err();
        assert!(error.to_string().contains("escapes benchmark directory"));
    }

    #[test]
    fn build_context_index_supports_flat_testcase_layout() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("contexts")).unwrap();
        fs::write(
            tmp.path().join("contexts/needle_context.txt"),
            "needle text",
        )
        .unwrap();
        fs::write(
            tmp.path().join("needle.json"),
            r#"{
  "schema_version": 1,
  "id": "flat-needle",
  "context": "contexts/needle_context.txt",
  "question": "What is the archive access code?",
  "expected": ["ORCHID-1"],
  "grader": "Exact",
  "metadata": {"suite": "needle", "token_count": "100", "seed": "7"}
}"#,
        )
        .unwrap();

        let index = build_context_index(tmp.path()).unwrap();
        assert_eq!(index.contexts.len(), 1);
        assert_eq!(index.contexts[0].path, "contexts/needle_context.txt");
        assert_eq!(index.contexts[0].token_count, 100);
        assert_eq!(index.contexts[0].seed, 7);
    }
}
