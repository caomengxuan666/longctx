use crate::benchmark::{Grader, ProviderRequestStyle, TestCase};
use crate::tokenizer::TokenCounter;
use regex::Regex;
use std::time::{Duration, Instant};

pub fn grade(answer: &str, test: &TestCase) -> bool {
    let answer = answer.trim();
    match &test.grader {
        Grader::Exact => test
            .expected
            .iter()
            .any(|expected| answer.eq_ignore_ascii_case(expected.trim())),
        Grader::Regex(pattern) => Regex::new(pattern)
            .map(|re| re.is_match(answer))
            .unwrap_or(false),
        Grader::Json => grade_json(answer, &test.expected),
        Grader::JsonFields => grade_json_fields(answer, &test.expected),
        Grader::Set => grade_set(answer, &test.expected),
        Grader::Contains => test
            .expected
            .iter()
            .all(|expected| compact(answer).contains(&compact(expected))),
        Grader::LlmJudge => false, // requires async; handled separately by runner
        Grader::ExpectRefusal => grade_expect_refusal(answer),
    }
}

pub fn grade_expect_refusal(answer: &str) -> bool {
    if answer.is_empty() {
        return true;
    }
    let lower = answer.to_lowercase();
    let refusal_signals = [
        "cannot find",
        "can't find",
        "not found",
        "no answer",
        "don't have",
        "do not have",
        "unable to find",
        "not mentioned",
        "not available",
        "no information",
        "i don't know",
        "i do not know",
        "cannot determine",
        "not specified",
        "not provided",
        "there is no",
        "not in the context",
        "not in the document",
        "not in the text",
        "no mention",
        "doesn't mention",
        "does not mention",
        "couldn't find",
        "could not find",
        "no data",
        "insufficient information",
    ];
    refusal_signals.iter().any(|signal| lower.contains(signal))
}

fn grade_set(answer: &str, expected: &[String]) -> bool {
    let normalized_answer = compact(answer);
    expected
        .iter()
        .all(|needle| normalized_answer.contains(&compact(needle)))
}

fn grade_json(answer: &str, expected: &[String]) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(answer) else {
        return false;
    };
    if expected.is_empty() {
        return true;
    }

    let rendered = value.to_string();
    expected
        .iter()
        .all(|needle| rendered.contains(needle) || answer.contains(needle))
}

fn grade_json_fields(answer: &str, expected: &[String]) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(answer) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };

    expected.iter().all(|needle| {
        let Some((key, expected_value)) = needle.split_once('=') else {
            return false;
        };
        let key = key.trim();
        let expected_value = expected_value.trim();
        let Some(actual) = object.get(key) else {
            return false;
        };
        json_value_matches(actual, expected_value)
    })
}

fn json_value_matches(actual: &serde_json::Value, expected: &str) -> bool {
    if let Ok(expected_json) = serde_json::from_str::<serde_json::Value>(expected) {
        return actual == &expected_json;
    }

    match actual {
        serde_json::Value::String(value) => {
            value.eq_ignore_ascii_case(expected) || compact(value) == compact(expected)
        }
        _ => {
            let actual_rendered = actual.to_string();
            actual_rendered == expected
        }
    }
}

fn compact(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Debug)]
pub struct LlmJudgeResult {
    pub passed: bool,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub http_status: Option<u16>,
    pub attempts: u32,
    pub error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn grade_with_llm(
    answer: &str,
    test: &TestCase,
    context: &str,
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    request_style: ProviderRequestStyle,
    timeout_secs: u64,
    max_retries: u32,
    retry_backoff_ms: u64,
    counter: &TokenCounter,
) -> LlmJudgeResult {
    let expected = test.expected.join(", ");
    let prompt = format!(
        "You are grading an answer to a question based on a context.\n\n\
         Context:\n{context}\n\n\
         Question:\n{}\n\n\
         Expected answer: {expected}\n\n\
         Student's answer: {answer}\n\n\
         Is the student's answer correct? Reply with only \"CORRECT\" or \"INCORRECT\".",
        test.question
    );

    let input_tokens = counter.count_tokens(&prompt);
    let started = Instant::now();

    let (url, body) = match request_style {
        ProviderRequestStyle::ChatCompletions => (
            format!("{}/chat/completions", base_url.trim_end_matches('/')),
            serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.0,
            }),
        ),
        ProviderRequestStyle::Responses => (
            format!("{}/responses", base_url.trim_end_matches('/')),
            serde_json::json!({
                "model": model,
                "input": prompt,
                "temperature": 0.0,
            }),
        ),
    };
    let max_attempts = max_retries.saturating_add(1).max(1);
    let mut attempts = 0;

    let resp = loop {
        attempts += 1;
        match client
            .post(&url)
            .bearer_auth(api_key)
            .json(&body)
            .timeout(Duration::from_secs(timeout_secs))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    if is_retryable_status(status) && attempts < max_attempts {
                        tokio::time::sleep(retry_delay(retry_backoff_ms, attempts)).await;
                        continue;
                    }
                    return LlmJudgeResult {
                        passed: false,
                        latency_ms: started.elapsed().as_millis() as u64,
                        input_tokens,
                        output_tokens: 0,
                        http_status: Some(status.as_u16()),
                        attempts,
                        error: Some(format!("HTTP {status} from judge: {}", body.trim())),
                    };
                }
                break resp;
            }
            Err(error) => {
                if is_retryable_error(&error) && attempts < max_attempts {
                    tokio::time::sleep(retry_delay(retry_backoff_ms, attempts)).await;
                    continue;
                }
                return LlmJudgeResult {
                    passed: false,
                    latency_ms: started.elapsed().as_millis() as u64,
                    input_tokens,
                    output_tokens: 0,
                    http_status: None,
                    attempts,
                    error: Some(error.to_string()),
                };
            }
        }
    };

    let latency_ms = started.elapsed().as_millis() as u64;
    let status = resp.status();
    let http_status = Some(status.as_u16());

    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return LlmJudgeResult {
            passed: false,
            latency_ms,
            input_tokens,
            output_tokens: 0,
            http_status,
            attempts,
            error: Some("failed to decode judge response JSON".to_string()),
        };
    };

    let judge_answer = match request_style {
        ProviderRequestStyle::ChatCompletions => json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string(),
        ProviderRequestStyle::Responses => responses_answer_from_value(&json).unwrap_or_default(),
    };

    let upper = judge_answer.to_uppercase();
    let passed = upper.contains("CORRECT") && !upper.contains("INCORRECT");
    let input_tokens = json
        .get("usage")
        .and_then(|usage| match request_style {
            ProviderRequestStyle::ChatCompletions => usage.get("prompt_tokens"),
            ProviderRequestStyle::Responses => usage.get("input_tokens"),
        })
        .and_then(|tokens| tokens.as_u64())
        .unwrap_or(input_tokens);
    let output_tokens = json
        .get("usage")
        .and_then(|usage| match request_style {
            ProviderRequestStyle::ChatCompletions => usage.get("completion_tokens"),
            ProviderRequestStyle::Responses => usage.get("output_tokens"),
        })
        .and_then(|tokens| tokens.as_u64())
        .unwrap_or_else(|| counter.count_tokens(&judge_answer));

    LlmJudgeResult {
        passed,
        latency_ms,
        input_tokens,
        output_tokens,
        http_status,
        attempts,
        error: None,
    }
}

fn responses_answer_from_value(json: &serde_json::Value) -> Option<String> {
    if let Some(text) = json
        .get("output_text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    json.get("output")
        .and_then(|output| output.as_array())
        .and_then(|items| {
            items
                .iter()
                .flat_map(|item| {
                    item.get("content")
                        .and_then(|content| content.as_array())
                        .into_iter()
                        .flatten()
                })
                .find_map(|content| {
                    content
                        .get("text")
                        .and_then(|text| text.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string)
                })
        })
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::REQUEST_TIMEOUT
            | reqwest::StatusCode::TOO_MANY_REQUESTS
            | reqwest::StatusCode::INTERNAL_SERVER_ERROR
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn retry_delay(retry_backoff_ms: u64, attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(10);
    Duration::from_millis(retry_backoff_ms.saturating_mul(1u64 << shift))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::{TestCase, SCHEMA_VERSION};
    use std::collections::BTreeMap;

    fn test_case(expected: Vec<&str>, grader: Grader) -> TestCase {
        TestCase {
            schema_version: SCHEMA_VERSION,
            id: "case".to_string(),
            context: "context".to_string(),
            question: "question".to_string(),
            expected: expected.into_iter().map(str::to_string).collect(),
            grader,
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn exact_grading_is_case_insensitive() {
        let test = test_case(vec!["ORCHID-123"], Grader::Exact);
        assert!(grade(" orchid-123 ", &test));
    }

    #[test]
    fn set_grading_accepts_items_in_any_order() {
        let test = test_case(vec!["Aster:12", "Beryl:34"], Grader::Set);
        assert!(grade("Beryl: 34, Aster: 12", &test));
    }

    #[test]
    fn json_grading_requires_valid_json() {
        let test = test_case(vec!["needle"], Grader::Json);
        assert!(!grade("needle", &test));
        assert!(grade(r#"{"value":"needle"}"#, &test));
    }

    #[test]
    fn contains_grading_accepts_substrings() {
        let test = test_case(vec!["archive code"], Grader::Contains);
        assert!(grade("The archive code is orchid-123.", &test));
    }

    #[test]
    fn json_fields_grading_matches_named_fields() {
        let test = test_case(
            vec!["name=Alpha", "count=3", "details={\"enabled\":true}"],
            Grader::JsonFields,
        );
        assert!(grade(
            r#"{"name":"alpha","count":3,"details":{"enabled":true}}"#,
            &test
        ));
    }

    #[test]
    fn json_fields_grading_rejects_missing_fields() {
        let test = test_case(vec!["name=Alpha", "count=3"], Grader::JsonFields);
        assert!(!grade(r#"{"name":"alpha"}"#, &test));
    }

    #[test]
    fn regex_grading_matches_pattern() {
        let test = test_case(
            vec![r"ORCHID-\d+"],
            Grader::Regex(r"ORCHID-\d+".to_string()),
        );
        assert!(grade("ORCHID-12345", &test));
    }

    #[test]
    fn regex_grading_rejects_non_matching() {
        let test = test_case(
            vec![r"ORCHID-\d+"],
            Grader::Regex(r"ORCHID-\d+".to_string()),
        );
        assert!(!grade("LAMBDA-12345", &test));
    }

    #[test]
    fn regex_grading_returns_false_for_invalid_regex() {
        let test = test_case(vec!["("], Grader::Regex("(".to_string()));
        assert!(!grade("anything", &test));
    }

    #[test]
    fn exact_grading_rejects_wrong_answer() {
        let test = test_case(vec!["ORCHID-123"], Grader::Exact);
        assert!(!grade("LAMBDA-456", &test));
    }

    #[test]
    fn json_fields_rejects_non_object() {
        let test = test_case(vec!["key=value"], Grader::JsonFields);
        assert!(!grade("[1, 2, 3]", &test));
    }

    #[test]
    fn json_fields_rejects_invalid_json() {
        let test = test_case(vec!["key=value"], Grader::JsonFields);
        assert!(!grade("not json", &test));
    }

    #[test]
    fn expect_refusal_passes_for_refusal_phrases() {
        let test = test_case(vec![], Grader::ExpectRefusal);
        assert!(grade(
            "I cannot find this information in the context.",
            &test
        ));
        assert!(grade("The answer is not mentioned in the document.", &test));
        assert!(grade("I don't know.", &test));
        assert!(grade("There is no answer provided.", &test));
    }

    #[test]
    fn expect_refusal_passes_for_empty_answer() {
        let test = test_case(vec![], Grader::ExpectRefusal);
        assert!(grade("", &test));
    }

    #[test]
    fn expect_refusal_rejects_specific_answer() {
        let test = test_case(vec![], Grader::ExpectRefusal);
        assert!(!grade("ORCHID-123456", &test));
        assert!(!grade("The archive access code is TAU-5738.", &test));
    }

    #[test]
    fn llm_judge_returns_false_from_sync_grade() {
        let test = test_case(vec!["answer"], Grader::LlmJudge);
        assert!(!grade("answer", &test));
    }

    #[test]
    fn expect_refusal_detects_more_signals() {
        let test = test_case(vec![], Grader::ExpectRefusal);
        assert!(grade("This is not in the context.", &test));
        assert!(grade("The document doesn't mention this.", &test));
        assert!(grade("Insufficient information to answer.", &test));
    }
}
