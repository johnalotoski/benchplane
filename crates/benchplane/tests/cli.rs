// SPDX-License-Identifier: Apache-2.0

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const SENSITIVE_ENV_SENTINEL: &str = "must-not-enter-benchplane-evidence";

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

fn comparison_fixtures() -> (PathBuf, PathBuf) {
    (
        fixture("evidence-compare/run-019fe9ab-d56f-7a70-a199-bfbd92aa3cd3"),
        fixture("evidence-compare/run-019fe9ab-efa8-7d31-b631-25ab14491fb8"),
    )
}

fn benchplane(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_benchplane"))
        .env("BENCHPLANE_TEST_SECRET", SENSITIVE_ENV_SENTINEL)
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
fn validate_accepts_all_tagged_yaml_examples() {
    for relative in [
        "experiments/smoke/local-fake.yaml",
        "experiments/smoke/local-cpu-probe.yaml",
        "experiments/smoke/local-llama-cpp.yaml",
        "experiments/examples/local-llama-cpp-nvidia-cuda.yaml",
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
fn cpu_probe_runs_through_public_cli_and_publishes_observed_measurements() {
    let directory = TestDirectory::new("cpu-probe");
    let experiment = repository_root().join("experiments/smoke/local-cpu-probe.yaml");
    let output_root = directory.path.join("output");
    let output = benchplane(&[
        "run",
        experiment.to_str().expect("UTF-8 experiment path"),
        "--output-root",
        output_root.to_str().expect("UTF-8 output root"),
        "--json",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        output_text(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("run result JSON");
    assert_eq!(result["runState"], "succeeded");
    assert_eq!(result["validityStatus"], "valid");
    assert!(result["resources"]["cpuTimeMicros"].as_u64().is_some());
    assert!(result["resources"]["peakRssBytes"]
        .as_u64()
        .unwrap()
        .is_multiple_of(1024));
    let bundle = Path::new(result["bundlePath"].as_str().expect("bundle path"));
    let records: Vec<Value> = fs::read_to_string(bundle.join("attempts/0001/measurements.jsonl"))
        .expect("measurements")
        .lines()
        .map(|line| serde_json::from_str(line).expect("measurement JSON"))
        .collect();
    assert_eq!(records.len(), 4);
    for record in records {
        assert_eq!(record["generator"], "benchplane-cpu-probe/v1");
        let latency = record["latencyMicros"].as_u64().unwrap();
        let ttft = record["timeToFirstTokenMicros"].as_u64().unwrap();
        assert!(latency > 0 && ttft > 0 && ttft <= latency);
        assert!(record["throughputMilliRequestsPerSecond"].as_u64().unwrap() > 0);
        assert_eq!(record["successfulRequests"], 2);
        assert_eq!(record["failedRequests"], 0);
    }
    let provenance: Value = serde_json::from_slice(
        &fs::read(bundle.join("attempts/0001/provenance.json")).expect("attempt provenance"),
    )
    .expect("attempt provenance JSON");
    assert_eq!(provenance["format"], "benchplane-attempt-provenance/v1");
    assert_eq!(provenance["runId"], result["runId"]);
    assert_eq!(provenance["attemptNumber"], 1);
    assert_eq!(provenance["software"]["runtime"]["kind"], "cpuProbe");
    assert_eq!(
        provenance["software"]["runtime"]["generator"],
        "benchplane-cpu-probe/v1"
    );
    let platform = provenance["platform"].as_object().expect("platform object");
    assert!(!platform.contains_key("hostname"));
    assert!(!platform.contains_key("machineId"));
    assert!(!platform.contains_key("username"));
    let provenance_text = serde_json::to_string(&provenance).expect("serialize provenance value");
    assert!(!provenance_text.contains("BENCHPLANE_TEST_SECRET"));
    assert!(!provenance_text.contains(SENSITIVE_ENV_SENTINEL));
    let resources: Value = serde_json::from_slice(
        &fs::read(bundle.join("attempts/0001/resources.json")).expect("attempt resources"),
    )
    .expect("attempt resources JSON");
    assert_eq!(resources["format"], "benchplane-attempt-resources/v1");
    assert_eq!(resources["runId"], result["runId"]);
    assert_eq!(resources["attemptNumber"], 1);
    assert_eq!(resources["scope"], "helperProcessLifetime");
    assert_eq!(
        resources["cpuTimeMicros"],
        result["resources"]["cpuTimeMicros"]
    );
    assert_eq!(
        resources["peakRssBytes"],
        result["resources"]["peakRssBytes"]
    );
    assert!(fs::read_to_string(bundle.join("SHA256SUMS"))
        .expect("checksum inventory")
        .lines()
        .any(|line| line.ends_with("  attempts/0001/provenance.json")));
    assert!(fs::read_to_string(bundle.join("SHA256SUMS"))
        .expect("checksum inventory")
        .lines()
        .any(|line| line.ends_with("  attempts/0001/resources.json")));
    let verified = benchplane(&[
        "evidence",
        "verify",
        bundle.to_str().expect("UTF-8 bundle path"),
    ]);
    assert!(
        verified.status.success(),
        "{}",
        output_text(&verified.stderr)
    );
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
        ("invalid/llama-cpp-wrong-model.yaml", "requires model"),
        (
            "invalid/llama-cpp-invalid-target.yaml",
            "unknown variant `arbitraryGpu`",
        ),
        (
            "invalid/llama-cpp-wrong-profile.yaml",
            "requires workload.profile",
        ),
        (
            "invalid/llama-cpp-concurrency.yaml",
            "supports only workload.concurrency 1",
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
        ("invalid/unknown-llama-cpp-runtime-field.yaml", "modelPath"),
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
fn llama_cpp_resolution_is_deterministic_and_materializes_fixed_identity() {
    let path = fixture("valid/minimal-llama-cpp.yaml");
    let path = path.to_str().expect("UTF-8 fixture path");
    let first = benchplane(&["resolve", path]);
    let second = benchplane(&["resolve", path]);
    assert!(first.status.success(), "{}", output_text(&first.stderr));
    assert_eq!(first.stdout, second.stdout);
    let resolved: Value = serde_json::from_slice(&first.stdout).expect("resolved JSON");
    assert_eq!(
        resolved["experiment"]["spec"]["runtime"]["kind"],
        "llamaCpp"
    );
    assert_eq!(
        resolved["experiment"]["spec"]["runtime"]["model"],
        "smollm2-135m-instruct-q2-k-v1"
    );
    assert_eq!(resolved["experiment"]["spec"]["runtime"]["outputTokens"], 4);
    assert_eq!(
        resolved["experiment"]["spec"]["workload"]["profile"],
        "smollm2-chat-greedy-v1"
    );
    assert!(resolved["experiment"]["spec"]["runtime"]
        .get("target")
        .is_none());

    let nvidia = fixture("valid/minimal-llama-cpp-nvidia-cuda.yaml");
    let nvidia = benchplane(&[
        "resolve",
        nvidia.to_str().expect("UTF-8 NVIDIA fixture path"),
    ]);
    assert!(nvidia.status.success(), "{}", output_text(&nvidia.stderr));
    let nvidia: Value = serde_json::from_slice(&nvidia.stdout).expect("resolved NVIDIA JSON");
    assert_eq!(
        nvidia["experiment"]["spec"]["runtime"]["target"],
        "nvidiaCuda"
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
    let source = fixture("evidence/run-018f6f9a-7b3c-7abc-8def-0123456789ab");
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
    let tampered = temporary
        .path
        .join("run-018f6f9a-7b3c-7abc-8def-0123456789ab");
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
fn evidence_compare_reports_verified_llama_metrics_in_json_and_human_forms() {
    let (baseline, candidate) = comparison_fixtures();
    let arguments = [
        "evidence",
        "compare",
        baseline.to_str().expect("UTF-8 baseline"),
        candidate.to_str().expect("UTF-8 candidate"),
        "--json",
    ];
    let first = benchplane(&arguments);
    let second = benchplane(&arguments);
    assert!(first.status.success(), "{}", output_text(&first.stderr));
    assert_eq!(
        first.stdout, second.stdout,
        "JSON output must be deterministic"
    );
    assert_eq!(
        first.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    let result: Value = serde_json::from_slice(&first.stdout).expect("comparison JSON");
    assert_eq!(result["format"], "benchplane-evidence-comparison/v1");
    assert_eq!(result["compatible"], true);
    assert_eq!(
        result["measurementContract"]["generator"],
        "benchplane-llama-cpp-smollm2/v2"
    );
    assert_eq!(result["requests"]["baselineCount"], 6);
    assert_eq!(result["requests"]["candidateCount"], 6);
    assert_eq!(result["repetitions"]["baselineCount"], 3);
    assert_eq!(result["repetitions"]["candidateCount"], 3);
    assert_eq!(result["attemptResources"]["unit"], "helperProcessLifetime");
    assert!(
        result["requests"]["latencyMicros"]["mean"]["delta"]["absoluteDelta"]
            .as_i64()
            .is_some()
    );
    assert!(result["environment"]
        .as_array()
        .expect("environment list")
        .iter()
        .any(|field| {
            field["field"] == "platform.operatingSystem.distribution"
                && field["relationship"] == "unknown"
        }));

    let human = benchplane(&[
        "evidence",
        "compare",
        baseline.to_str().expect("UTF-8 baseline"),
        candidate.to_str().expect("UTF-8 candidate"),
    ]);
    assert!(human.status.success(), "{}", output_text(&human.stderr));
    let stdout = output_text(&human.stdout);
    for expected in [
        "measurement compatible: yes",
        "measured requests: baseline=6 candidate=6",
        "request latency p95 (µs):",
        "request TTFT p95 (µs):",
        "measured repetitions: baseline=3 candidate=3",
        "repetition aggregate TTFT mean (µs):",
        "whole-helper CPU time (µs):",
        "whole-helper peak RSS (bytes):",
        "Descriptive only:",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in {stdout}"
        );
    }
}

#[test]
fn evidence_compare_rejects_same_run_identity_and_invalid_input() {
    let (baseline, candidate) = comparison_fixtures();
    let same_path = benchplane(&[
        "evidence",
        "compare",
        baseline.to_str().expect("UTF-8 baseline"),
        baseline.to_str().expect("UTF-8 baseline"),
        "--json",
    ]);
    assert_eq!(same_path.status.code(), Some(5));
    assert!(output_text(&same_path.stderr)
        .contains("baseline and candidate must be distinct Benchplane runs"));
    assert!(same_path.stdout.is_empty());

    let same_run_copy_root = TestDirectory::new("comparison-same-run-copy");
    let same_run_copy = same_run_copy_root
        .path
        .join(baseline.file_name().expect("bundle directory name"));
    copy_directory(&baseline, &same_run_copy);
    let copied = benchplane(&[
        "evidence",
        "compare",
        baseline.to_str().expect("UTF-8 baseline"),
        same_run_copy.to_str().expect("UTF-8 copied bundle"),
    ]);
    assert_eq!(copied.status.code(), Some(5));
    assert!(output_text(&copied.stderr)
        .contains("baseline and candidate must be distinct Benchplane runs"));
    assert!(copied.stdout.is_empty());

    let temporary = TestDirectory::new("comparison-invalid-candidate");
    let invalid = temporary
        .path
        .join(candidate.file_name().expect("bundle directory name"));
    copy_directory(&candidate, &invalid);
    fs::write(invalid.join("summary.json"), b"{}\n").expect("tamper summary");
    let rejected = benchplane(&[
        "evidence",
        "compare",
        baseline.to_str().expect("UTF-8 baseline"),
        invalid.to_str().expect("UTF-8 candidate"),
    ]);
    assert_eq!(rejected.status.code(), Some(5));
    assert!(output_text(&rejected.stderr).contains("candidate evidence bundle is invalid"));
    assert!(rejected.stdout.is_empty());
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
        "repetition-aggregate latency (µs):",
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
    assert!(!stdout.contains("helper CPU time"));
    assert!(!stdout.contains("helper peak RSS"));
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
    assert!(result["resources"].is_null());
    let bundle = Path::new(result["bundlePath"].as_str().expect("bundle path"));
    assert!(bundle.is_dir());
    assert!(!bundle.join("attempts/0001/resources.json").exists());
}

#[test]
fn helper_resources_are_reported_in_human_output() {
    let directory = TestDirectory::new("cpu-probe-human-resources");
    let experiment = repository_root().join("experiments/smoke/local-cpu-probe.yaml");
    let output = benchplane(&[
        "run",
        experiment.to_str().expect("UTF-8 experiment path"),
        "--output-root",
        directory.path.to_str().expect("UTF-8 output root"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        output_text(&output.stderr)
    );
    let stdout = output_text(&output.stdout);
    assert!(stdout.contains("helper CPU time (µs):"), "{stdout}");
    assert!(stdout.contains("helper peak RSS (bytes):"), "{stdout}");
}

#[test]
fn run_rejects_excessive_local_fake_work_before_allocation() {
    let directory = TestDirectory::new("excessive-work");
    let experiment = directory.path.join("experiment.yaml");
    let output_root = directory.path.join("output");
    fs::write(
        &experiment,
        "apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: { name: excessive }\nspec:\n  provider: { kind: localFake }\n  runtime: { kind: localFake }\n  workload: { profile: smoke, requests: 1 }\n  measurement: { warmupRuns: 10000, repetitions: 1 }\n  budget: { maximumCostUsd: 0 }\n",
    )
    .expect("write experiment");
    let output = benchplane(&[
        "run",
        experiment.to_str().expect("UTF-8 experiment path"),
        "--output-root",
        output_root.to_str().expect("UTF-8 output root"),
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(output_text(&output.stderr)
        .contains("runtime.localFake total generated records must not exceed 10000"));
    assert!(!output_root.exists());
}

#[test]
fn run_rejects_invalid_llama_cpp_work_and_provider_before_allocation() {
    for (label, provider, requests, expected) in [
        (
            "wrong-provider",
            "localFake",
            1,
            "execution.unsupportedCombination",
        ),
        ("excessive-work", "local", 100, "exceeds 8192 tokens"),
    ] {
        let directory = TestDirectory::new(label);
        let experiment = directory.path.join("experiment.yaml");
        let output_root = directory.path.join("output");
        fs::write(
            &experiment,
            format!(
                "apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: {{ name: {label} }}\nspec:\n  provider: {{ kind: {provider} }}\n  runtime: {{ kind: llamaCpp }}\n  workload: {{ profile: smollm2-chat-greedy-v1, requests: {requests} }}\n  budget: {{ maximumCostUsd: 0 }}\n"
            ),
        )
        .expect("write experiment");
        let output = benchplane(&[
            "run",
            experiment.to_str().expect("UTF-8 experiment path"),
            "--output-root",
            output_root.to_str().expect("UTF-8 output root"),
            "--json",
        ]);
        assert_eq!(output.status.code(), Some(2), "label={label}");
        assert!(output.stdout.is_empty(), "label={label}");
        assert!(
            output_text(&output.stderr).contains(expected),
            "label={label}"
        );
        assert!(!output_root.exists(), "label={label}");
    }
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
