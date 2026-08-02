// SPDX-License-Identifier: Apache-2.0

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

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
    assert!(output_text(&verified.stdout).contains("run=local-fake-0001 status=complete"));

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let tampered = std::env::temp_dir().join(format!(
        "benchplane-evidence-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&tampered).expect("temporary evidence directory");
    for name in ["manifest.json", "summary.json", "SHA256SUMS"] {
        fs::copy(source.join(name), tampered.join(name)).expect("copy evidence fixture");
    }
    fs::write(tampered.join("summary.json"), b"{}\n").expect("tamper with summary");

    let rejected = benchplane(&[
        "evidence",
        "verify",
        tampered.to_str().expect("UTF-8 temporary path"),
    ]);
    fs::remove_dir_all(&tampered).expect("remove temporary evidence directory");

    assert!(!rejected.status.success(), "tampered evidence should fail");
    assert!(output_text(&rejected.stderr).contains("checksum mismatch for summary.json"));
}
