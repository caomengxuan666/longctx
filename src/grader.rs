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
    }
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
