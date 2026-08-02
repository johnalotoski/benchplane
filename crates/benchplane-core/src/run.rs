// SPDX-License-Identifier: Apache-2.0

use crate::{
    evidence::write_checksum_file,
    lifecycle::{Lifecycle, LifecycleError},
    local_fake::{evaluate_validity, execute, summarize},
    parse_experiment, resolve_experiment, verify_evidence_bundle, ParseError, ResolutionError,
};
use benchplane_schema::{
    AttemptRecord, AttemptStatus, EvidenceManifest, FailureRecord, LocalFakeScenario, ProviderSpec,
    ResolvedExperiment, RunRecord, RunResult, RunState, RuntimeSpec,
    ERROR_EVIDENCE_FINALIZATION_FAILED, ERROR_EXECUTION_UNSUPPORTED_COMBINATION,
    EVIDENCE_FORMAT_V1,
};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub output_root: PathBuf,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            output_root: PathBuf::from(".benchplane"),
        }
    }
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error("{code}: {message}")]
    UnsupportedCombination { code: &'static str, message: String },
    #[error("I/O operation failed for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Lifecycle(#[from] LifecycleError),
    #[error("could not serialize {record}: {source}")]
    Serialization {
        record: &'static str,
        source: serde_json::Error,
    },
    #[error(
        "evidence finalization failed for {run_id}; staging retained at {staging_path}: {message}"
    )]
    Finalization {
        run_id: String,
        staging_path: String,
        message: String,
    },
}

impl RunError {
    pub fn is_request_rejection(&self) -> bool {
        matches!(
            self,
            Self::Parse(_) | Self::Resolution(_) | Self::UnsupportedCombination { .. }
        )
    }

    pub fn is_finalization_failure(&self) -> bool {
        matches!(self, Self::Finalization { .. })
    }
}

trait Clock {
    fn now(&self) -> String;
}

trait RunIdGenerator {
    fn next_run_id(&self) -> String;
}

trait PublicationHook {
    fn before_publish(&self, staging: &Path, final_path: &Path) -> std::io::Result<()>;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

struct UuidV7Generator;

impl RunIdGenerator for UuidV7Generator {
    fn next_run_id(&self) -> String {
        format!("run-{}", uuid::Uuid::now_v7())
    }
}

struct NoopPublicationHook;

impl PublicationHook for NoopPublicationHook {
    fn before_publish(&self, _staging: &Path, _final_path: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

struct RunServices<'a> {
    clock: &'a dyn Clock,
    ids: &'a dyn RunIdGenerator,
    publication: &'a dyn PublicationHook,
}

pub fn run_experiment(
    experiment_bytes: &[u8],
    options: &RunOptions,
) -> Result<RunResult, RunError> {
    let clock = SystemClock;
    let ids = UuidV7Generator;
    let publication = NoopPublicationHook;
    run_experiment_with_services(
        experiment_bytes,
        options,
        &RunServices {
            clock: &clock,
            ids: &ids,
            publication: &publication,
        },
    )
}

fn run_experiment_with_services(
    experiment_bytes: &[u8],
    options: &RunOptions,
    services: &RunServices<'_>,
) -> Result<RunResult, RunError> {
    let experiment = parse_experiment(experiment_bytes)?;
    let plan = resolve_experiment(experiment)?;
    let (seed, scenario) = local_fake_controls(&plan)?;

    let run_id = services.ids.next_run_id();
    let staging_parent = options.output_root.join("staging");
    let runs_parent = options.output_root.join("runs");
    create_directory(&staging_parent)?;
    create_directory(&runs_parent)?;
    let staging = staging_parent.join(&run_id);
    let final_path = runs_parent.join(&run_id);
    create_new_directory(&staging)?;
    let attempt_directory = staging.join("attempts/0001");
    create_directory(&attempt_directory)?;

    let created_at = services.clock.now();
    let (mut lifecycle, initial_event) = Lifecycle::new(created_at.clone(), 1);
    let mut run_record = RunRecord {
        run_id: run_id.clone(),
        run_status: RunState::Created,
        created_at: created_at.clone(),
        updated_at: created_at.clone(),
        completed_at: None,
        experiment_digest: plan.experiment_digest.clone(),
        resolved_plan_digest: plan.resolved_plan_digest.clone(),
        attempt_count: 1,
        failure: None,
    };
    let mut attempt_record = AttemptRecord {
        run_id: run_id.clone(),
        attempt_number: 1,
        status: AttemptStatus::Created,
        started_at: created_at.clone(),
        updated_at: created_at,
        completed_at: None,
        failure: None,
    };

    write_bytes(&staging.join("experiment.yaml"), experiment_bytes)?;
    write_json_atomic(&staging.join("resolved-plan.json"), &plan, "resolved plan")?;
    let events_path = staging.join("events.jsonl");
    create_empty_file(&events_path)?;
    append_json_line(&events_path, &initial_event, "lifecycle event")?;
    let measurements_path = attempt_directory.join("measurements.jsonl");
    create_empty_file(&measurements_path)?;
    write_snapshots(&staging, &attempt_directory, &run_record, &attempt_record)?;

    persist_transition(
        &mut lifecycle,
        RunState::Preparing,
        None,
        services.clock,
        &events_path,
        &staging,
        &attempt_directory,
        &mut run_record,
        &mut attempt_record,
    )?;
    persist_transition(
        &mut lifecycle,
        RunState::Running,
        None,
        services.clock,
        &events_path,
        &staging,
        &attempt_directory,
        &mut run_record,
        &mut attempt_record,
    )?;

    let execution = execute(&plan, seed, scenario);
    for measurement in &execution.measurements {
        append_json_line(&measurements_path, measurement, "measurement")?;
    }

    if execution.terminal_state == RunState::Succeeded {
        persist_transition(
            &mut lifecycle,
            RunState::Collecting,
            None,
            services.clock,
            &events_path,
            &staging,
            &attempt_directory,
            &mut run_record,
            &mut attempt_record,
        )?;
    }
    let validity = evaluate_validity(&plan, &execution, &run_id);
    let summary = summarize(&plan, &execution, &validity, &run_id);

    let finalization = (|| -> Result<Vec<u8>, String> {
        persist_transition(
            &mut lifecycle,
            RunState::Finalizing,
            None,
            services.clock,
            &events_path,
            &staging,
            &attempt_directory,
            &mut run_record,
            &mut attempt_record,
        )
        .map_err(|error| error.to_string())?;
        persist_transition(
            &mut lifecycle,
            execution.terminal_state,
            execution.failure.clone(),
            services.clock,
            &events_path,
            &staging,
            &attempt_directory,
            &mut run_record,
            &mut attempt_record,
        )
        .map_err(|error| error.to_string())?;
        write_json_atomic(&staging.join("validity.json"), &validity, "validity result")
            .map_err(|error| error.to_string())?;
        write_json_atomic(&staging.join("summary.json"), &summary, "run summary")
            .map_err(|error| error.to_string())?;
        let manifest = EvidenceManifest {
            format: EVIDENCE_FORMAT_V1.to_owned(),
            run_id: run_id.clone(),
            run_status: execution.terminal_state,
            validity_status: validity.status,
            experiment_digest: plan.experiment_digest.clone(),
            resolved_plan_digest: plan.resolved_plan_digest.clone(),
            attempt_count: 1,
        };
        write_json_atomic(
            &staging.join("manifest.json"),
            &manifest,
            "evidence manifest",
        )
        .map_err(|error| error.to_string())?;
        let checksum_bytes = write_checksum_file(&staging).map_err(|error| error.to_string())?;
        verify_evidence_bundle(&staging).map_err(|error| error.to_string())?;
        if final_path.exists() {
            return Err(format!(
                "final bundle path already exists: {}",
                final_path.display()
            ));
        }
        services
            .publication
            .before_publish(&staging, &final_path)
            .map_err(|error| error.to_string())?;
        fs::rename(&staging, &final_path).map_err(|error| error.to_string())?;
        Ok(checksum_bytes)
    })();

    let checksum_bytes = match finalization {
        Ok(bytes) => bytes,
        Err(message) => {
            persist_finalization_failure(&staging, services.clock, &mut run_record, &message);
            return Err(RunError::Finalization {
                run_id,
                staging_path: staging.display().to_string(),
                message,
            });
        }
    };
    let evidence_digest = crate::resolution::sha256_digest(&checksum_bytes);

    Ok(RunResult {
        run_id,
        run_state: execution.terminal_state,
        validity_status: validity.status,
        attempt_count: 1,
        sample_count: summary.sample_count,
        latency: summary.latency,
        mean_throughput_milli_requests_per_second: summary
            .mean_throughput_milli_requests_per_second,
        bundle_path: final_path.display().to_string(),
        experiment_digest: plan.experiment_digest,
        resolved_plan_digest: plan.resolved_plan_digest,
        evidence_digest,
        failure: execution.failure,
    })
}

fn local_fake_controls(plan: &ResolvedExperiment) -> Result<(u64, LocalFakeScenario), RunError> {
    match (
        &plan.experiment.spec.provider,
        &plan.experiment.spec.runtime,
    ) {
        (ProviderSpec::LocalFake, RuntimeSpec::LocalFake { seed, scenario }) => {
            Ok((*seed, *scenario))
        }
        _ => Err(RunError::UnsupportedCombination {
            code: ERROR_EXECUTION_UNSUPPORTED_COMBINATION,
            message: "only provider localFake with runtime localFake is executable".to_owned(),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_transition(
    lifecycle: &mut Lifecycle,
    to: RunState,
    failure: Option<FailureRecord>,
    clock: &dyn Clock,
    events_path: &Path,
    staging: &Path,
    attempt_directory: &Path,
    run_record: &mut RunRecord,
    attempt_record: &mut AttemptRecord,
) -> Result<(), RunError> {
    let event = lifecycle.transition(to, clock.now(), failure.clone())?;
    append_json_line(events_path, &event, "lifecycle event")?;

    run_record.run_status = to;
    run_record.updated_at.clone_from(&event.recorded_at);
    attempt_record.status = attempt_status(to);
    attempt_record.updated_at.clone_from(&event.recorded_at);
    if to.is_terminal() {
        run_record.completed_at = Some(event.recorded_at.clone());
        attempt_record.completed_at = Some(event.recorded_at.clone());
        run_record.failure.clone_from(&failure);
        attempt_record.failure = failure;
    }
    write_snapshots(staging, attempt_directory, run_record, attempt_record)
}

fn attempt_status(state: RunState) -> AttemptStatus {
    match state {
        RunState::Created => AttemptStatus::Created,
        RunState::Preparing => AttemptStatus::Preparing,
        RunState::Running => AttemptStatus::Running,
        RunState::Collecting => AttemptStatus::Collecting,
        RunState::Finalizing => AttemptStatus::Finalizing,
        RunState::Succeeded => AttemptStatus::Succeeded,
        RunState::Failed => AttemptStatus::Failed,
        RunState::Interrupted => AttemptStatus::Interrupted,
    }
}

fn write_snapshots(
    staging: &Path,
    attempt_directory: &Path,
    run_record: &RunRecord,
    attempt_record: &AttemptRecord,
) -> Result<(), RunError> {
    write_json_atomic(&staging.join("run.json"), run_record, "run record")?;
    write_json_atomic(
        &attempt_directory.join("attempt.json"),
        attempt_record,
        "attempt record",
    )
}

fn persist_finalization_failure(
    staging: &Path,
    clock: &dyn Clock,
    run_record: &mut RunRecord,
    message: &str,
) {
    run_record.updated_at = clock.now();
    run_record.failure = Some(FailureRecord {
        phase: "finalizing".to_owned(),
        code: ERROR_EVIDENCE_FINALIZATION_FAILED.to_owned(),
        message: message.to_owned(),
        retryable: false,
        attempt_number: 1,
    });
    let _ = write_json_atomic(&staging.join("run.json"), run_record, "run record");
}

fn create_directory(path: &Path) -> Result<(), RunError> {
    fs::create_dir_all(path).map_err(|source| RunError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn create_new_directory(path: &Path) -> Result<(), RunError> {
    fs::create_dir(path).map_err(|source| RunError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn create_empty_file(path: &Path) -> Result<(), RunError> {
    fs::File::create(path)
        .map(|_| ())
        .map_err(|source| RunError::Io {
            path: path.display().to_string(),
            source,
        })
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), RunError> {
    fs::write(path, bytes).map_err(|source| RunError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn json_bytes<T: Serialize>(value: &T, record: &'static str) -> Result<Vec<u8>, RunError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|source| RunError::Serialization { record, source })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_json_atomic<T: Serialize>(
    path: &Path,
    value: &T,
    record: &'static str,
) -> Result<(), RunError> {
    let file_name = path
        .file_name()
        .expect("record path must have a file name")
        .to_string_lossy();
    let temporary = path.with_file_name(format!("{file_name}.tmp"));
    write_bytes(&temporary, &json_bytes(value, record)?)?;
    fs::rename(&temporary, path).map_err(|source| RunError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn append_json_line<T: Serialize>(
    path: &Path,
    value: &T,
    record: &'static str,
) -> Result<(), RunError> {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|source| RunError::Io {
            path: path.display().to_string(),
            source,
        })?;
    let bytes =
        serde_json::to_vec(value).map_err(|source| RunError::Serialization { record, source })?;
    file.write_all(&bytes)
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|source| RunError::Io {
            path: path.display().to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{verify_evidence_bundle, EvidenceError};
    use benchplane_schema::{LocalFakeScenario, MeasurementRecord, ValidityStatus};
    use std::{
        cell::Cell,
        collections::BTreeMap,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    const FIXED_RUN_ID: &str = "run-018f6f9a-7b3c-7abc-8def-0123456789ab";
    const FIXED_TIME: &str = "2026-01-02T03:04:05.006Z";

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "benchplane-run-{label}-{}-{counter}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> String {
            FIXED_TIME.to_owned()
        }
    }

    struct FixedIds;

    impl RunIdGenerator for FixedIds {
        fn next_run_id(&self) -> String {
            FIXED_RUN_ID.to_owned()
        }
    }

    struct AssertPublicationBoundary {
        observed: Cell<bool>,
        fail: bool,
    }

    impl PublicationHook for AssertPublicationBoundary {
        fn before_publish(&self, staging: &Path, final_path: &Path) -> std::io::Result<()> {
            assert!(staging.exists());
            assert!(!final_path.exists());
            verify_evidence_bundle(staging).expect("staging must verify before publication");
            self.observed.set(true);
            if self.fail {
                Err(std::io::Error::other("injected publication failure"))
            } else {
                Ok(())
            }
        }
    }

    fn experiment(seed: u64, scenario: LocalFakeScenario) -> Vec<u8> {
        let scenario = match scenario {
            LocalFakeScenario::Success => "success",
            LocalFakeScenario::RuntimeFailure => "runtimeFailure",
            LocalFakeScenario::Interrupted => "interrupted",
            LocalFakeScenario::InsufficientMeasurements => "insufficientMeasurements",
        };
        format!(
            "apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata:\n  name: test\nspec:\n  provider:\n    kind: localFake\n  runtime:\n    kind: localFake\n    seed: {seed}\n    scenario: {scenario}\n  workload:\n    profile: smoke\n    requests: 8\n  measurement:\n    warmupRuns: 1\n    repetitions: 3\n  budget:\n    maximumCostUsd: 0\n"
        )
        .into_bytes()
    }

    fn execute_fixed(
        root: &Path,
        seed: u64,
        scenario: LocalFakeScenario,
        hook: &dyn PublicationHook,
    ) -> Result<RunResult, RunError> {
        run_experiment_with_services(
            &experiment(seed, scenario),
            &RunOptions {
                output_root: root.to_owned(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &FixedIds,
                publication: hook,
            },
        )
    }

    fn read_measurements(bundle: &Path) -> Vec<MeasurementRecord> {
        fs::read_to_string(bundle.join("attempts/0001/measurements.jsonl"))
            .expect("read measurements")
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse measurement"))
            .collect()
    }

    fn directory_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(directory).expect("read fixture directory") {
                let entry = entry.expect("read fixture entry");
                if entry.file_type().expect("read fixture file type").is_dir() {
                    collect(root, &entry.path(), files);
                } else {
                    let relative = entry
                        .path()
                        .strip_prefix(root)
                        .expect("fixture file beneath root")
                        .to_owned();
                    files.insert(relative, fs::read(entry.path()).expect("read fixture file"));
                }
            }
        }

        let mut files = BTreeMap::new();
        collect(root, root, &mut files);
        files
    }

    #[test]
    fn successful_run_is_valid_verified_and_atomically_published() {
        let directory = TestDirectory::new("success");
        let hook = AssertPublicationBoundary {
            observed: Cell::new(false),
            fail: false,
        };
        let result = execute_fixed(&directory.0, 42, LocalFakeScenario::Success, &hook)
            .expect("run should succeed");
        let bundle = PathBuf::from(&result.bundle_path);

        assert!(hook.observed.get());
        assert_eq!(result.run_state, RunState::Succeeded);
        assert_eq!(result.validity_status, ValidityStatus::Valid);
        assert_eq!(result.sample_count, 3);
        assert!(bundle.exists());
        assert!(!directory.0.join("staging").join(FIXED_RUN_ID).exists());
        verify_evidence_bundle(&bundle).expect("published bundle must verify");
    }

    #[test]
    fn checked_in_fixture_matches_fixed_execution() {
        let directory = TestDirectory::new("fixture-parity");
        let result = execute_fixed(
            &directory.0,
            42,
            LocalFakeScenario::Success,
            &NoopPublicationHook,
        )
        .expect("fixed execution should succeed");
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/evidence/local-fake");

        assert_eq!(
            directory_files(&fixture),
            directory_files(Path::new(&result.bundle_path)),
            "checked-in evidence fixture must be regenerated from fixed execution"
        );
    }

    #[test]
    fn same_seed_is_deterministic_and_different_seed_changes_output() {
        let first = TestDirectory::new("same-seed-first");
        let second = TestDirectory::new("same-seed-second");
        let third = TestDirectory::new("different-seed");
        let noop = NoopPublicationHook;
        let first_result =
            execute_fixed(&first.0, 42, LocalFakeScenario::Success, &noop).expect("first run");
        let second_result =
            execute_fixed(&second.0, 42, LocalFakeScenario::Success, &noop).expect("second run");
        let third_result =
            execute_fixed(&third.0, 43, LocalFakeScenario::Success, &noop).expect("third run");

        let first_bundle = PathBuf::from(first_result.bundle_path);
        let second_bundle = PathBuf::from(second_result.bundle_path);
        let third_bundle = PathBuf::from(third_result.bundle_path);
        assert_eq!(
            fs::read(first_bundle.join("attempts/0001/measurements.jsonl")).unwrap(),
            fs::read(second_bundle.join("attempts/0001/measurements.jsonl")).unwrap()
        );
        assert_eq!(
            fs::read(first_bundle.join("summary.json")).unwrap(),
            fs::read(second_bundle.join("summary.json")).unwrap()
        );
        assert_ne!(
            read_measurements(&first_bundle),
            read_measurements(&third_bundle)
        );
    }

    #[test]
    fn failure_interruption_and_insufficient_measurements_finalize() {
        for (scenario, expected_state, expected_validity, expected_failure) in [
            (
                LocalFakeScenario::RuntimeFailure,
                RunState::Failed,
                ValidityStatus::Indeterminate,
                Some(benchplane_schema::ERROR_LOCAL_FAKE_RUNTIME_FAILURE),
            ),
            (
                LocalFakeScenario::Interrupted,
                RunState::Interrupted,
                ValidityStatus::Indeterminate,
                Some(benchplane_schema::ERROR_LOCAL_FAKE_INTERRUPTED),
            ),
            (
                LocalFakeScenario::InsufficientMeasurements,
                RunState::Succeeded,
                ValidityStatus::Invalid,
                None,
            ),
        ] {
            let directory = TestDirectory::new("terminal-scenario");
            let result = execute_fixed(&directory.0, 42, scenario, &NoopPublicationHook)
                .expect("scenario should finalize");
            assert_eq!(result.run_state, expected_state);
            assert_eq!(result.validity_status, expected_validity);
            assert_eq!(
                result.failure.as_ref().map(|failure| failure.code.as_str()),
                expected_failure
            );
            verify_evidence_bundle(Path::new(&result.bundle_path))
                .expect("terminal scenario bundle should verify");
        }
    }

    #[test]
    fn output_paths_with_spaces_and_unicode_work() {
        let directory = TestDirectory::new("path-parent");
        let root = directory.0.join("output with spaces λ");
        let result = execute_fixed(&root, 42, LocalFakeScenario::Success, &NoopPublicationHook)
            .expect("Unicode output path should work");
        verify_evidence_bundle(Path::new(&result.bundle_path)).expect("bundle should verify");
    }

    #[test]
    fn injected_publication_failure_leaves_staging_and_no_final_bundle() {
        let directory = TestDirectory::new("publication-failure");
        let hook = AssertPublicationBoundary {
            observed: Cell::new(false),
            fail: true,
        };
        let error = execute_fixed(&directory.0, 42, LocalFakeScenario::Success, &hook)
            .expect_err("publication should fail");
        assert!(error.is_finalization_failure());
        assert!(hook.observed.get());
        assert!(directory.0.join("staging").join(FIXED_RUN_ID).exists());
        assert!(!directory.0.join("runs").join(FIXED_RUN_ID).exists());
        let retained: RunRecord = serde_json::from_slice(
            &fs::read(
                directory
                    .0
                    .join("staging")
                    .join(FIXED_RUN_ID)
                    .join("run.json"),
            )
            .expect("read retained run record"),
        )
        .expect("parse retained run record");
        assert_eq!(
            retained.failure.expect("finalization failure record").code,
            ERROR_EVIDENCE_FINALIZATION_FAILED
        );
        assert!(matches!(
            verify_evidence_bundle(&directory.0.join("staging").join(FIXED_RUN_ID)),
            Err(EvidenceError::ChecksumMismatch(path)) if path == "run.json"
        ));
    }

    #[test]
    fn partial_staging_bundle_is_rejected() {
        let directory = TestDirectory::new("partial");
        let staging = directory.0.join("staging").join(FIXED_RUN_ID);
        fs::create_dir_all(&staging).expect("create partial staging");
        fs::write(
            staging.join("experiment.yaml"),
            experiment(0, LocalFakeScenario::Success),
        )
        .expect("write partial payload");
        assert!(verify_evidence_bundle(&staging).is_err());
    }

    #[test]
    fn unsupported_combinations_are_rejected_before_run_id_allocation() {
        struct PanickingIds;
        impl RunIdGenerator for PanickingIds {
            fn next_run_id(&self) -> String {
                panic!("run ID must not be allocated")
            }
        }

        let bytes = b"apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: { name: unsupported }\nspec:\n  provider: { kind: localFake }\n  runtime: { kind: vllm, model: example, revision: main }\n  workload: { profile: smoke, requests: 1 }\n  budget: { maximumCostUsd: 0 }\n";
        let directory = TestDirectory::new("unsupported");
        let error = run_experiment_with_services(
            bytes,
            &RunOptions {
                output_root: directory.0.clone(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &PanickingIds,
                publication: &NoopPublicationHook,
            },
        )
        .expect_err("unsupported combination should be rejected");
        assert!(error.is_request_rejection());
    }
}
