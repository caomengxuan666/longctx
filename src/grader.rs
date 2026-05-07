use crate::benchmark::{Grader, TestCase};
use regex::Regex;

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
    }
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
}
