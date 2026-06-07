use anyhow::{Context, Result};
use eval_harness::{HarnessReport, load_fixture, run_case};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[tokio::main]
async fn main() -> Result<()> {
    let api_base = std::env::var("RUST_HARNESS_API_BASE")
        .unwrap_or_else(|_| "http://127.0.0.1:4000".to_owned());
    let fixtures_dir = std::env::var("RUST_HARNESS_FIXTURES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("crates/eval_harness/fixtures"));
    let suites = [
        ("v1", "chat-context-regression-cases.json"),
        ("v2", "chat-context-regression-v2-cases.json"),
        ("v3", "chat-context-regression-v3-cases.json"),
        ("v4", "chat-context-regression-v4-cases.json"),
        ("v5", "chat-context-regression-v5-cases.json"),
        ("v6", "chat-context-regression-v6-cases.json"),
    ];
    let selected_suites = selected_suites();
    let concurrency = std::env::var("RUST_HARNESS_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);

    let mut reports = Vec::new();
    for (suite, file_name) in suites {
        if !selected_suites.is_empty() && !selected_suites.contains(suite) {
            continue;
        }
        let fixture_path = fixtures_dir.join(file_name);
        let cases = load_fixture(&fixture_path).with_context(|| {
            format!("failed to load {suite} fixture {}", fixture_path.display())
        })?;
        let suite_reports = run_suite(&api_base, suite, cases, concurrency).await?;
        reports.extend(suite_reports);
    }

    print_summary(&reports);

    if reports.iter().any(|report| !report.passed) {
        std::process::exit(1);
    }

    Ok(())
}

fn selected_suites() -> HashSet<String> {
    std::env::var("RUST_HARNESS_SUITES")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

async fn run_suite(
    api_base: &str,
    suite: &'static str,
    cases: Vec<eval_harness::RegressionCase>,
    concurrency: usize,
) -> Result<Vec<HarnessReport>> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = tokio::task::JoinSet::new();

    for case in cases {
        let permit = semaphore.clone().acquire_owned().await?;
        let api_base = api_base.to_owned();
        tasks.spawn(async move {
            let _permit = permit;
            run_case(&api_base, suite, &case).await
        });
    }

    let mut reports = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let mut case_reports = result??;
        for report in &case_reports {
            println!("{}", serde_json::to_string(report)?);
        }
        reports.append(&mut case_reports);
    }
    Ok(reports)
}

fn print_summary(reports: &[HarnessReport]) {
    let total = reports.len();
    let passed = reports.iter().filter(|report| report.passed).count();
    eprintln!("summary: passed={passed}/{total}");

    for report in reports.iter().filter(|report| !report.passed).take(30) {
        eprintln!(
            "failed: {} {} turn {} expected={} actual={} type={} reply={} missing={} vector={} model={} tool={} message={}",
            report.suite,
            report.case,
            report.turn,
            report.expected_type,
            report.actual_type,
            report.type_check,
            report.reply_checks,
            report.missing_field_checks,
            report.vector_chunk_checks,
            report.model_call_checks,
            report.tool_call_checks,
            report.message
        );
    }
}
