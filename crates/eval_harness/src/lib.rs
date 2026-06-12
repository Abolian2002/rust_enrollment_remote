use anyhow::{Context, Result};
use domain::ChatRequest;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionCase {
    pub name: String,
    #[serde(default, rename = "requiresLiveProvider")]
    pub requires_live_provider: bool,
    pub turns: Vec<RegressionTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionTurn {
    pub message: String,
    pub expect: RegressionExpectation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionExpectation {
    #[serde(rename = "type")]
    pub structured_type: String,
    #[serde(default, rename = "bundleIncludes")]
    pub bundle_includes: Vec<String>,
    #[serde(default, rename = "missingFields")]
    pub missing_fields: Vec<String>,
    #[serde(default, rename = "replyIncludes")]
    pub reply_includes: Vec<String>,
    #[serde(default, rename = "replyExcludes")]
    pub reply_excludes: Vec<String>,
    #[serde(default, rename = "minVectorChunks")]
    pub min_vector_chunks: Option<usize>,
    #[serde(default, rename = "minModelCalls")]
    pub min_model_calls: Option<usize>,
    #[serde(default, rename = "minToolCalls")]
    pub min_tool_calls: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessReport {
    pub suite: String,
    pub case: String,
    pub turn: usize,
    pub message: String,
    pub expected_type: String,
    pub actual_type: String,
    pub type_check: String,
    pub reply_checks: String,
    pub missing_field_checks: String,
    pub vector_chunk_checks: String,
    pub model_call_checks: String,
    pub tool_call_checks: String,
    pub passed: bool,
    pub tool_calls: Vec<String>,
    pub latency_ms: u128,
    pub compression: String,
    pub reply: String,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope {
    success: bool,
    data: Value,
    error: Option<Value>,
}

pub fn load_fixture(path: impl AsRef<Path>) -> Result<Vec<RegressionCase>> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read fixture {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse fixture {}", path.display()))
}

pub async fn run_case(
    api_base: &str,
    suite: &str,
    case: &RegressionCase,
) -> Result<Vec<HarnessReport>> {
    let http = Client::new();
    let run_id = uuid::Uuid::new_v4();
    let mut conversation_id = format!(
        "rust_harness_{}_{}_{}",
        suite,
        case.name.replace(' ', "_"),
        run_id
    );
    let mut reports = Vec::new();

    for (turn_index, turn) in case.turns.iter().enumerate() {
        let started_at = Instant::now();
        let response = http
            .post(format!("{}/api/v1/chat", api_base.trim_end_matches('/')))
            .json(&ChatRequest {
                conversation_id: Some(conversation_id.clone()),
                message: turn.message.clone(),
                profile: None,
            })
            .send()
            .await?
            .error_for_status()?
            .json::<ApiEnvelope>()
            .await?;

        if !response.success {
            anyhow::bail!("case {} failed: {:?}", case.name, response.error);
        }
        conversation_id = response
            .data
            .get("conversationId")
            .and_then(Value::as_str)
            .unwrap_or(&conversation_id)
            .to_owned();
        let structured = response
            .data
            .get("structuredResult")
            .cloned()
            .unwrap_or(Value::Null);
        let actual_type = structured
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let reply = response
            .data
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let type_passed = actual_type == turn.expect.structured_type;
        let reply_passed = turn
            .expect
            .reply_includes
            .iter()
            .all(|expected| reply.contains(expected))
            && turn
                .expect
                .reply_excludes
                .iter()
                .all(|forbidden| !reply.contains(forbidden))
            && default_reply_excludes()
                .iter()
                .all(|forbidden| !reply.contains(forbidden));
        let missing_fields = structured
            .get("missingFields")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let missing_passed =
            turn.expect.missing_fields.is_empty() || missing_fields == turn.expect.missing_fields;
        let vector_chunk_count = structured
            .get("vectorChunks")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let vector_passed = turn
            .expect
            .min_vector_chunks
            .is_none_or(|minimum| vector_chunk_count >= minimum);
        let tool_calls = response
            .data
            .get("diagnostics")
            .and_then(|value| value.get("trace"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("toolName")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let model_call_count = response
            .data
            .get("diagnostics")
            .and_then(|value| value.get("modelCallCount"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let tool_call_count = response
            .data
            .get("diagnostics")
            .and_then(|value| value.get("toolCallCount"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let model_calls_passed = turn
            .expect
            .min_model_calls
            .is_none_or(|minimum| model_call_count >= minimum);
        let tool_calls_passed = turn
            .expect
            .min_tool_calls
            .is_none_or(|minimum| tool_call_count >= minimum);
        let compression = response
            .data
            .get("diagnostics")
            .and_then(|value| value.get("compression"))
            .and_then(|value| value.get("level"))
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_owned();

        reports.push(HarnessReport {
            suite: suite.to_owned(),
            case: case.name.clone(),
            turn: turn_index + 1,
            message: turn.message.clone(),
            expected_type: turn.expect.structured_type.clone(),
            actual_type,
            type_check: pass_fail(type_passed),
            reply_checks: pass_fail(reply_passed),
            missing_field_checks: pass_fail(missing_passed),
            vector_chunk_checks: pass_fail(vector_passed),
            model_call_checks: pass_fail(model_calls_passed),
            tool_call_checks: pass_fail(tool_calls_passed),
            passed: type_passed
                && reply_passed
                && missing_passed
                && vector_passed
                && model_calls_passed
                && tool_calls_passed,
            tool_calls,
            latency_ms: started_at.elapsed().as_millis(),
            compression,
            reply: reply.to_owned(),
        });
    }

    Ok(reports)
}

fn pass_fail(passed: bool) -> String {
    if passed { "passed" } else { "failed" }.to_owned()
}

fn default_reply_excludes() -> &'static [&'static str] {
    &[
        "0451-88060176",
        "0451–88060176",
        "0451-88060177",
        "0451–88060177",
        "文化课×40%",
        "文化课 x 40%",
        "文化课*40%",
        "文化课成绩×40%",
        "文化课成绩*40%",
        "专业课×60%",
        "专业课*60%",
        "专业课成绩×60%",
        "专业课成绩*60%",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_v1_fixture() {
        let cases = load_fixture("fixtures/chat-context-regression-cases.json").unwrap();
        assert!(!cases.is_empty());
        assert!(cases.iter().any(|case| !case.turns.is_empty()));
    }

    #[test]
    fn loads_v5_fixture() {
        let cases = load_fixture("fixtures/chat-context-regression-v5-cases.json").unwrap();
        assert_eq!(cases.len(), 5);
        assert_eq!(cases.iter().map(|case| case.turns.len()).sum::<usize>(), 20);
    }

    #[test]
    fn loads_v6_fixture() {
        let cases = load_fixture("fixtures/chat-context-regression-v6-cases.json").unwrap();
        assert_eq!(cases.len(), 8);
        assert_eq!(cases.iter().map(|case| case.turns.len()).sum::<usize>(), 27);
    }
}
