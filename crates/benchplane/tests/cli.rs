// SPDX-License-Identifier: Apache-2.0

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "benchplane-cli-{label}-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create test directory");
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(relative: &str) -> PathBuf {
    repository_root().join("tests/fixtures").join(relative)
}

fn benchplane(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_benchplane"))
        .args(arguments)
        .output()
        .expect("benchplane should execute")
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create destination directory");
    for entry in fs::read_dir(source).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("read entry type").is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn scenario_experiment(scenario: &str) -> String {
    format!(
        "apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata:\n  name: cli-{scenario}\nspec:\n  provider:\n    kind: localFake\n  runtime:\n    kind: localFake\n    seed: 42\n    scenario: {scenario}\n  workload:\n    profile: smoke\n    requests: 8\n  measurement:\n    warmupRuns: 1\n    repetitions: 3\n  budget:\n    maximumCostUsd: 0\n"
    )
}

fn run_scenario(scenario: &str, json: bool) -> (TestDirectory, Output) {
    let directory = TestDirectory::new(scenario);
    let experiment = directory.path.join("experiment.yaml");
    let output_root = directory.path.join("output");
    fs::write(&experiment, scenario_experiment(scenario)).expect("write experiment");
    let mut arguments = vec![
        "run",
        experiment.to_str().expect("UTF-8 experiment path"),
        "--output-root",
        output_root.to_str().expect("UTF-8 output path"),
    ];
    if json {
        arguments.push("--json");
    }
    let output = benchplane(&arguments);
    (directory, output)
}

#[test]
fn validate_accepts_both_tagged_yaml_examples() {
    for relative in [
        "experiments/smoke/local-fake.yaml",
        "experiments/examples/vllm-single-gpu.yaml",
    ] {
        let path = repository_root().join(relative);
        let output = benchplane(&["validate", path.to_str().expect("UTF-8 fixture path")]);
        assert!(
            output.status.success(),
            "{relative} should validate: {}",
            output_text(&output.stderr)
        );
    }
}

#[test]
fn validate_rejects_required_invalid_fixtures() {
    let cases = [
        ("invalid/api-version.yaml", "unsupported apiVersion"),
        (
            "invalid/zero-requests.yaml",
            "requests must be greater than zero",
        ),
        (
            "invalid/empty-aws-instance-types.yaml",
            "requires at least one instance type",
        ),
        ("invalid/invalid-budget.yaml", "finite and nonnegative"),
        (
            "invalid/excessive-runtime.yaml",
            "must be between 1 and 86400",
        ),
    ];

    for (relative, expected_error) in cases {
        let path = fixture(relative);
        let output = benchplane(&["validate", path.to_str().expect("UTF-8 fixture path")]);
        assert!(!output.status.success(), "{relative} should be rejected");
        assert!(
            output_text(&output.stderr).contains(expected_error),
            "{relative} did not report {expected_error:?}: {}",
            output_text(&output.stderr)
        );
    }
}

#[test]
fn validate_rejects_unknown_fields_at_declarative_boundaries() {
    let cases = [
        ("invalid/unknown-top-level.yaml", "unexpectedSetting"),
        (
            "invalid/misspelled-lifecycle-key.yaml",
            "maximumRuntimeSecond",
        ),
        (
            "invalid/unknown-aws-provider-field.yaml",
            "availabilityZone",
        ),
        (
            "invalid/unknown-vllm-runtime-field.yaml",
            "tensorParallelSize",
        ),
        ("invalid/unknown-local-fake-runtime-field.yaml", "scenaro"),
    ];

    for (relative, unknown_field) in cases {
        let path = fixture(relative);
        let output = benchplane(&["validate", path.to_str().expect("UTF-8 fixture path")]);
        assert!(!output.status.success(), "{relative} should be rejected");
        let stderr = output_text(&output.stderr);
        assert!(
            stderr.contains("unknown field") && stderr.contains(unknown_field),
            "{relative} did not identify {unknown_field:?} as unknown: {stderr}"
        );
    }
}

#[test]
fn validate_rejects_duplicate_yaml_keys() {
    let path = fixture("invalid/duplicate-key.yaml");
    let output = benchplane(&["validate", path.to_str().expect("UTF-8 fixture path")]);

    assert!(!output.status.success(), "duplicate key should be rejected");
    assert!(
        output_text(&output.stderr).contains("duplicate"),
        "duplicate-key error was not reported: {}",
        output_text(&output.stderr)
    );
}

#[test]
fn resolve_is_deterministic_and_materializes_defaults() {
    let path = fixture("valid/minimal-local-fake.yaml");
    let path = path.to_str().expect("UTF-8 fixture path");
    let first = benchplane(&["resolve", path]);
    let second = benchplane(&["resolve", path]);

    assert!(first.status.success(), "{}", output_text(&first.stderr));
    assert_eq!(first.stdout, second.stdout);

    let resolved: Value = serde_json::from_slice(&first.stdout).expect("valid resolved JSON");
    assert_eq!(resolved["experiment"]["spec"]["workload"]["concurrency"], 1);
    assert_eq!(
        resolved["experiment"]["spec"]["measurement"]["repetitions"],
        3
    );
    assert_eq!(
        resolved["experiment"]["spec"]["lifecycle"]["maximumRuntimeSeconds"],
        3600
    );
    assert_eq!(
        resolved["experimentDigest"]
            .as_str()
            .expect("string digest")
            .len(),
        71
    );
    assert_eq!(
        resolved["resolvedPlanDigest"]
            .as_str()
            .expect("string digest")
            .len(),
        71
    );
}

#[test]
fn schema_export_matches_the_checked_in_schema() {
    let output = benchplane(&["schema", "export"]);
    assert!(output.status.success(), "{}", output_text(&output.stderr));
    let exported = output_text(&output.stdout);
    assert!(exported.contains("\"instanceTypes\""));
    assert!(!exported.contains("\"instance_types\""));

    let checked_in = fs::read(repository_root().join("schemas/v1alpha1/experiment.schema.json"))
        .expect("checked-in schema");
    assert_eq!(output.stdout, checked_in, "generated schema has drifted");
}

#[test]
fn evidence_verify_accepts_fixture_and_detects_tampering() {
    let source = fixture("evidence/local-fake");
    let verified = benchplane(&[
        "evidence",
        "verify",
        source.to_str().expect("UTF-8 fixture path"),
    ]);
    assert!(
        verified.status.success(),
        "{}",
        output_text(&verified.stderr)
    );
    let stdout = output_text(&verified.stdout);
    assert!(stdout
        .contains("run=run-018f6f9a-7b3c-7abc-8def-0123456789ab status=succeeded validity=valid"));

    let temporary = TestDirectory::new("evidence-tamper");
    let tampered = temporary.path.join("bundle");
    copy_directory(&source, &tampered);
    fs::write(tampered.join("summary.json"), b"{}\n").expect("tamper with summary");

    let rejected = benchplane(&[
        "evidence",
        "verify",
        tampered.to_str().expect("UTF-8 temporary path"),
    ]);
    assert_eq!(rejected.status.code(), Some(5));
    assert!(output_text(&rejected.stderr).contains("checksum mismatch for summary.json"));
}

#[test]
fn run_human_output_reports_the_terminal_result() {
    let (_directory, output) = run_scenario("success", false);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        output_text(&output.stderr)
    );
    let stdout = output_text(&output.stdout);
    for expected in [
        "run ID: run-",
        "run state: succeeded",
        "validity: valid",
        "attempt count: 1",
        "sample count: 3",
        "latency (µs):",
        "mean throughput (mreq/s):",
        "bundle path:",
        "experiment digest: sha256:",
        "resolved-plan digest: sha256:",
        "evidence digest: sha256:",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in {stdout}"
        );
    }
}

#[test]
fn run_json_output_is_one_object_with_no_stdout_diagnostics() {
    let (_directory, output) = run_scenario("success", true);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        output_text(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected diagnostics: {}",
        output_text(&output.stderr)
    );
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("run result JSON");
    assert_eq!(result["runState"], "succeeded");
    assert_eq!(result["validityStatus"], "valid");
    assert_eq!(result["attemptCount"], 1);
    assert_eq!(result["sampleCount"], 3);
    let bundle = Path::new(result["bundlePath"].as_str().expect("bundle path"));
    assert!(bundle.is_dir());
}

#[test]
fn run_exit_codes_cover_rejection_invalidity_and_operational_outcomes() {
    let missing = benchplane(&["run", "/definitely/missing/benchplane-experiment.yaml"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());

    let invalid = fixture("invalid/unknown-local-fake-runtime-field.yaml");
    let rejected = benchplane(&[
        "run",
        invalid.to_str().expect("UTF-8 fixture path"),
        "--json",
    ]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());

    let (_directory, insufficient) = run_scenario("insufficientMeasurements", true);
    assert_eq!(insufficient.status.code(), Some(3));
    let result: Value = serde_json::from_slice(&insufficient.stdout).expect("invalid run JSON");
    assert_eq!(result["runState"], "succeeded");
    assert_eq!(result["validityStatus"], "invalid");

    for (scenario, state) in [("runtimeFailure", "failed"), ("interrupted", "interrupted")] {
        let (_directory, output) = run_scenario(scenario, true);
        assert_eq!(output.status.code(), Some(4));
        let result: Value = serde_json::from_slice(&output.stdout).expect("terminal run JSON");
        assert_eq!(result["runState"], state);
        assert_eq!(result["validityStatus"], "indeterminate");
        assert!(Path::new(result["bundlePath"].as_str().expect("bundle path")).is_dir());
    }
}
