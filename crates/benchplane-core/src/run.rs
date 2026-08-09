// SPDX-License-Identifier: Apache-2.0

use crate::{
    cpu_probe::{self, CpuProbeConfig},
    evidence::write_checksum_file,
    execution::{evaluate_validity, summarize},
    lifecycle::{Lifecycle, LifecycleError},
    llama_cpp::{self, LlamaCppConfig},
    local_fake, parse_experiment, provenance, resolve_experiment, verify_evidence_bundle,
    EvidenceError, ParseError, ResolutionError,
};
use benchplane_schema::{
    AttemptRecord, AttemptResources, AttemptStatus, EvidenceManifest, FailureRecord,
    LocalFakeScenario, ProviderSpec, ResolvedExperiment, ResourceScope, RunRecord, RunResult,
    RunState, RuntimeSpec, ATTEMPT_RESOURCES_FORMAT_V1, ERROR_EVIDENCE_FINALIZATION_FAILED,
    ERROR_EXECUTION_UNSUPPORTED_COMBINATION, EVIDENCE_FORMAT_V1,
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
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    #[error("publication destination already exists: {0}")]
    PublicationConflict(String),
    #[error("run {run_id} failed during {phase}; staging retained at {staging_path}: {source}")]
    Allocated {
        run_id: String,
        phase: &'static str,
        staging_path: String,
        #[source]
        source: Box<RunError>,
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
        matches!(self, Self::Allocated { .. })
    }
}

trait Clock {
    fn now(&self) -> String;
}

trait RunIdGenerator {
    fn next_run_id(&self) -> String;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunCheckpoint {
    MeasurementAppend,
    Checksum,
    Verification,
    Publication,
}

trait RunHook {
    fn check(
        &self,
        checkpoint: RunCheckpoint,
        staging: &Path,
        final_path: &Path,
    ) -> std::io::Result<()>;
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

struct NoopRunHook;

impl RunHook for NoopRunHook {
    fn check(
        &self,
        _checkpoint: RunCheckpoint,
        _staging: &Path,
        _final_path: &Path,
    ) -> std::io::Result<()> {
        Ok(())
    }
}

struct RunServices<'a> {
    clock: &'a dyn Clock,
    ids: &'a dyn RunIdGenerator,
    hook: &'a dyn RunHook,
    cpu_probe_executable: Option<&'a Path>,
    llama_cpp_executable: Option<&'a Path>,
}

pub fn run_experiment(
    experiment_bytes: &[u8],
    options: &RunOptions,
) -> Result<RunResult, RunError> {
    let clock = SystemClock;
    let ids = UuidV7Generator;
    let hook = NoopRunHook;
    run_experiment_with_services(
        experiment_bytes,
        options,
        &RunServices {
            clock: &clock,
            ids: &ids,
            hook: &hook,
            cpu_probe_executable: None,
            llama_cpp_executable: None,
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
    let execution_kind = execution_kind(&plan)?;

    let run_id = services.ids.next_run_id();
    let staging_parent = options.output_root.join("staging");
    let runs_parent = options.output_root.join("runs");
    create_directory(&staging_parent)?;
    create_directory(&runs_parent)?;
    let staging = staging_parent.join(&run_id);
    let final_path = runs_parent.join(&run_id);
    create_new_directory(&staging)?;
    let attempt_directory = staging.join("attempts/0001");

    let created_at = services.clock.now();
    let (mut lifecycle, initial_event) = Lifecycle::new(run_id.clone(), created_at.clone(), 1);
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
    let events_path = staging.join("events.jsonl");
    let measurements_path = attempt_directory.join("measurements.jsonl");

    let outcome = (|| -> Result<RunResult, AllocatedFailure> {
        at(create_directory(&attempt_directory), "initializing")?;
        at(
            write_bytes(&staging.join("experiment.yaml"), experiment_bytes),
            "initializing",
        )?;
        at(
            write_json_atomic(&staging.join("resolved-plan.json"), &plan, "resolved plan"),
            "initializing",
        )?;
        at(create_empty_file(&events_path), "initializing")?;
        at(
            append_json_line(&events_path, &initial_event, "lifecycle event"),
            "initializing",
        )?;
        at(create_empty_file(&measurements_path), "initializing")?;
        at(
            write_snapshots(&staging, &attempt_directory, &run_record, &attempt_record),
            "initializing",
        )?;

        at(
            persist_run_transition(
                &mut lifecycle,
                RunState::Preparing,
                None,
                services.clock,
                &events_path,
                &staging,
                &mut run_record,
            ),
            "preparing",
        )?;
        at(
            persist_attempt_status(
                AttemptStatus::Preparing,
                None,
                services.clock,
                &attempt_directory,
                &mut attempt_record,
            ),
            "preparing",
        )?;
        let provenance = provenance::capture(&run_id, &plan);
        at(
            write_json_atomic(
                &attempt_directory.join("provenance.json"),
                &provenance,
                "attempt provenance",
            ),
            "preparing",
        )?;
        at(
            persist_run_transition(
                &mut lifecycle,
                RunState::Running,
                None,
                services.clock,
                &events_path,
                &staging,
                &mut run_record,
            ),
            "running",
        )?;
        at(
            persist_attempt_status(
                AttemptStatus::Running,
                None,
                services.clock,
                &attempt_directory,
                &mut attempt_record,
            ),
            "running",
        )?;

        let execution = match execution_kind {
            ExecutionKind::LocalFake { seed, scenario } => {
                local_fake::execute(&plan, seed, scenario)
            }
            ExecutionKind::CpuProbe(config) => {
                let executable = services
                    .cpu_probe_executable
                    .map(Path::to_path_buf)
                    .unwrap_or_else(packaged_cpu_probe_executable);
                cpu_probe::execute(&executable, config)
            }
            ExecutionKind::LlamaCpp(config) => {
                let executable = services
                    .llama_cpp_executable
                    .map(Path::to_path_buf)
                    .unwrap_or_else(packaged_llama_cpp_executable);
                llama_cpp::execute(&executable, config)
            }
        };
        let attempt_terminal = match execution.terminal_state {
            RunState::Succeeded => AttemptStatus::Succeeded,
            RunState::Failed => AttemptStatus::Failed,
            RunState::Interrupted => AttemptStatus::Interrupted,
            _ => unreachable!("execution returns a terminal state"),
        };
        at(
            persist_attempt_status(
                attempt_terminal,
                execution.failure.clone(),
                services.clock,
                &attempt_directory,
                &mut attempt_record,
            ),
            "running",
        )?;
        if let Some(resources) = execution.resources {
            let attempt_resources = AttemptResources {
                format: ATTEMPT_RESOURCES_FORMAT_V1.to_owned(),
                run_id: run_id.clone(),
                attempt_number: 1,
                scope: ResourceScope::HelperProcessLifetime,
                cpu_time_micros: resources.cpu_time_micros,
                peak_rss_bytes: resources.peak_rss_bytes,
            };
            at(
                write_json_atomic(
                    &attempt_directory.join("resources.json"),
                    &attempt_resources,
                    "attempt resources",
                ),
                "recordingResources",
            )?;
        }
        at(
            services
                .hook
                .check(RunCheckpoint::MeasurementAppend, &staging, &final_path)
                .map_err(|source| RunError::Io {
                    path: measurements_path.display().to_string(),
                    source,
                }),
            "recordingMeasurements",
        )?;
        for measurement in &execution.measurements {
            at(
                append_json_line(&measurements_path, measurement, "measurement"),
                "recordingMeasurements",
            )?;
        }

        if execution.terminal_state == RunState::Succeeded {
            at(
                persist_run_transition(
                    &mut lifecycle,
                    RunState::Collecting,
                    None,
                    services.clock,
                    &events_path,
                    &staging,
                    &mut run_record,
                ),
                "collecting",
            )?;
        }
        let validity = evaluate_validity(&plan, &execution, &run_id);
        let summary = summarize(&plan, &execution, &validity, &run_id);

        at(
            persist_run_transition(
                &mut lifecycle,
                RunState::Finalizing,
                None,
                services.clock,
                &events_path,
                &staging,
                &mut run_record,
            ),
            "finalizing",
        )?;
        at(
            persist_run_transition(
                &mut lifecycle,
                execution.terminal_state,
                execution.failure.clone(),
                services.clock,
                &events_path,
                &staging,
                &mut run_record,
            ),
            "finalizing",
        )?;
        at(
            write_json_atomic(&staging.join("validity.json"), &validity, "validity result"),
            "finalizing",
        )?;
        at(
            write_json_atomic(&staging.join("summary.json"), &summary, "run summary"),
            "finalizing",
        )?;
        let manifest = EvidenceManifest {
            format: EVIDENCE_FORMAT_V1.to_owned(),
            run_id: run_id.clone(),
            run_status: execution.terminal_state,
            validity_status: validity.status,
            experiment_digest: plan.experiment_digest.clone(),
            resolved_plan_digest: plan.resolved_plan_digest.clone(),
            attempt_count: 1,
        };
        at(
            write_json_atomic(
                &staging.join("manifest.json"),
                &manifest,
                "evidence manifest",
            ),
            "finalizing",
        )?;
        at(
            services
                .hook
                .check(RunCheckpoint::Checksum, &staging, &final_path)
                .map_err(|source| RunError::Io {
                    path: staging.display().to_string(),
                    source,
                }),
            "checksumming",
        )?;
        let pending_evidence_digest = at(
            write_checksum_file(&staging).map_err(RunError::from),
            "checksumming",
        )?;
        at(
            services
                .hook
                .check(RunCheckpoint::Verification, &staging, &final_path)
                .map_err(|source| RunError::Io {
                    path: staging.display().to_string(),
                    source,
                }),
            "verifying",
        )?;
        at(
            verify_evidence_bundle(&staging)
                .map(|_| ())
                .map_err(RunError::from),
            "verifying",
        )?;
        if final_path.exists() {
            return Err(AllocatedFailure {
                phase: "publishing",
                source: Box::new(RunError::PublicationConflict(
                    final_path.display().to_string(),
                )),
            });
        }
        at(
            services
                .hook
                .check(RunCheckpoint::Publication, &staging, &final_path)
                .map_err(|source| RunError::Io {
                    path: final_path.display().to_string(),
                    source,
                }),
            "publishing",
        )?;
        at(
            fs::rename(&staging, &final_path).map_err(|source| RunError::Io {
                path: final_path.display().to_string(),
                source,
            }),
            "publishing",
        )?;
        let evidence_digest = pending_evidence_digest.finish();

        Ok(RunResult {
            run_id: run_id.clone(),
            run_state: execution.terminal_state,
            validity_status: validity.status,
            attempt_count: 1,
            sample_count: summary.sample_count,
            latency: summary.latency,
            mean_throughput_milli_requests_per_second: summary
                .mean_throughput_milli_requests_per_second,
            resources: execution.resources,
            bundle_path: final_path.display().to_string(),
            experiment_digest: plan.experiment_digest.clone(),
            resolved_plan_digest: plan.resolved_plan_digest.clone(),
            evidence_digest,
            failure: execution.failure,
        })
    })();

    match outcome {
        Ok(result) => Ok(result),
        Err(failure) => {
            persist_allocated_failure(
                &staging,
                &attempt_directory,
                &events_path,
                services.clock,
                &mut lifecycle,
                &mut run_record,
                &attempt_record,
                failure.phase,
                &failure.source.to_string(),
            );
            Err(RunError::Allocated {
                run_id,
                phase: failure.phase,
                staging_path: staging.display().to_string(),
                source: failure.source,
            })
        }
    }
}

struct AllocatedFailure {
    phase: &'static str,
    source: Box<RunError>,
}

fn at<T>(result: Result<T, RunError>, phase: &'static str) -> Result<T, AllocatedFailure> {
    result.map_err(|source| AllocatedFailure {
        phase,
        source: Box::new(source),
    })
}

#[derive(Debug, Clone, Copy)]
enum ExecutionKind {
    LocalFake {
        seed: u64,
        scenario: LocalFakeScenario,
    },
    CpuProbe(CpuProbeConfig),
    LlamaCpp(LlamaCppConfig),
}

fn execution_kind(plan: &ResolvedExperiment) -> Result<ExecutionKind, RunError> {
    match (
        &plan.experiment.spec.provider,
        &plan.experiment.spec.runtime,
    ) {
        (ProviderSpec::LocalFake, RuntimeSpec::LocalFake { seed, scenario }) => {
            Ok(ExecutionKind::LocalFake {
                seed: *seed,
                scenario: *scenario,
            })
        }
        (
            ProviderSpec::Local,
            RuntimeSpec::CpuProbe {
                output_tokens,
                work_units_per_token,
            },
        ) => Ok(ExecutionKind::CpuProbe(CpuProbeConfig {
            requests: plan.experiment.spec.workload.requests,
            warmup_runs: plan.experiment.spec.measurement.warmup_runs,
            repetitions: plan.experiment.spec.measurement.repetitions,
            output_tokens: *output_tokens,
            work_units_per_token: *work_units_per_token,
            maximum_runtime_seconds: plan
                .experiment
                .spec
                .lifecycle
                .maximum_runtime_seconds,
        })),
        (
            ProviderSpec::Local,
            RuntimeSpec::LlamaCpp { output_tokens, .. },
        ) => Ok(ExecutionKind::LlamaCpp(LlamaCppConfig {
            requests: plan.experiment.spec.workload.requests,
            warmup_runs: plan.experiment.spec.measurement.warmup_runs,
            repetitions: plan.experiment.spec.measurement.repetitions,
            output_tokens: *output_tokens,
            maximum_runtime_seconds: plan
                .experiment
                .spec
                .lifecycle
                .maximum_runtime_seconds,
        })),
        _ => Err(RunError::UnsupportedCombination {
            code: ERROR_EXECUTION_UNSUPPORTED_COMBINATION,
            message: "executable combinations are provider localFake with runtime localFake, provider local with runtime cpuProbe, and provider local with runtime llamaCpp".to_owned(),
        }),
    }
}

fn packaged_cpu_probe_executable() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| {
            executable
                .parent()
                .map(|directory| directory.join("benchplane-cpu-probe"))
        })
        .unwrap_or_else(|| PathBuf::from("/benchplane-package-missing/bin/benchplane-cpu-probe"))
}

fn packaged_llama_cpp_executable() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| {
            executable
                .parent()
                .map(|directory| directory.join("benchplane-llama-cpp"))
        })
        .unwrap_or_else(|| PathBuf::from("/benchplane-package-missing/bin/benchplane-llama-cpp"))
}

fn persist_run_transition(
    lifecycle: &mut Lifecycle,
    to: RunState,
    failure: Option<FailureRecord>,
    clock: &dyn Clock,
    events_path: &Path,
    staging: &Path,
    run_record: &mut RunRecord,
) -> Result<(), RunError> {
    let event = lifecycle.transition(to, clock.now(), failure.clone())?;
    append_json_line(events_path, &event, "lifecycle event")?;

    run_record.run_status = to;
    run_record.updated_at.clone_from(&event.recorded_at);
    if to.is_terminal() {
        run_record.completed_at = Some(event.recorded_at.clone());
        run_record.failure.clone_from(&failure);
    }
    write_json_atomic(&staging.join("run.json"), run_record, "run record")
}

fn persist_attempt_status(
    status: AttemptStatus,
    failure: Option<FailureRecord>,
    clock: &dyn Clock,
    attempt_directory: &Path,
    attempt_record: &mut AttemptRecord,
) -> Result<(), RunError> {
    let recorded_at = clock.now();
    attempt_record.status = status;
    attempt_record.updated_at.clone_from(&recorded_at);
    if matches!(
        status,
        AttemptStatus::Succeeded | AttemptStatus::Failed | AttemptStatus::Interrupted
    ) {
        attempt_record.completed_at = Some(recorded_at);
        attempt_record.failure = failure;
    }
    write_json_atomic(
        &attempt_directory.join("attempt.json"),
        attempt_record,
        "attempt record",
    )
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

#[allow(clippy::too_many_arguments)]
fn persist_allocated_failure(
    staging: &Path,
    attempt_directory: &Path,
    events_path: &Path,
    clock: &dyn Clock,
    lifecycle: &mut Lifecycle,
    run_record: &mut RunRecord,
    attempt_record: &AttemptRecord,
    phase: &'static str,
    message: &str,
) {
    let failure = FailureRecord {
        phase: phase.to_owned(),
        code: if matches!(
            phase,
            "finalizing" | "checksumming" | "verifying" | "publishing"
        ) {
            ERROR_EVIDENCE_FINALIZATION_FAILED.to_owned()
        } else {
            benchplane_schema::ERROR_IO_OPERATION_FAILED.to_owned()
        },
        message: message.to_owned(),
        retryable: false,
        attempt_number: 1,
    };

    if matches!(
        lifecycle.state(),
        RunState::Preparing | RunState::Running | RunState::Collecting
    ) {
        let _ = persist_run_transition(
            lifecycle,
            RunState::Finalizing,
            None,
            clock,
            events_path,
            staging,
            run_record,
        );
    }
    if lifecycle.state() == RunState::Finalizing {
        let _ = persist_run_transition(
            lifecycle,
            RunState::Failed,
            Some(failure.clone()),
            clock,
            events_path,
            staging,
            run_record,
        );
    }
    run_record.updated_at = clock.now();
    run_record.failure = Some(failure);
    let _ = write_json_atomic(&staging.join("run.json"), run_record, "run record");
    let _ = write_json_atomic(
        &attempt_directory.join("attempt.json"),
        attempt_record,
        "attempt record",
    );
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
    use benchplane_schema::{
        AttemptProvenance, AttemptResources, LocalFakeScenario, MeasurementRecord, ResourceScope,
        RuntimeProvenance, ValidityStatus, ATTEMPT_PROVENANCE_FORMAT_V1,
        ATTEMPT_RESOURCES_FORMAT_V1, CPU_PROBE_GENERATOR_VERSION, LLAMA_CPP_ENGINE_VERSION,
        LLAMA_CPP_GENERATOR_VERSION, LLAMA_CPP_MODEL_IDENTITY, LLAMA_CPP_MODEL_SHA256,
        LOCAL_FAKE_GENERATOR_VERSION,
    };
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

    struct FailAt(RunCheckpoint);

    impl RunHook for FailAt {
        fn check(
            &self,
            checkpoint: RunCheckpoint,
            _staging: &Path,
            _final_path: &Path,
        ) -> std::io::Result<()> {
            if checkpoint == self.0 {
                Err(std::io::Error::other("injected run failure"))
            } else {
                Ok(())
            }
        }
    }

    impl RunHook for AssertPublicationBoundary {
        fn check(
            &self,
            checkpoint: RunCheckpoint,
            staging: &Path,
            final_path: &Path,
        ) -> std::io::Result<()> {
            if checkpoint != RunCheckpoint::Publication {
                return Ok(());
            }
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

    fn cpu_experiment() -> Vec<u8> {
        b"apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: { name: cpu-probe }\nspec:\n  provider: { kind: local }\n  runtime: { kind: cpuProbe, outputTokens: 4, workUnitsPerToken: 32 }\n  workload: { profile: cpu-token-probe-v1, requests: 2, concurrency: 1 }\n  measurement: { warmupRuns: 1, repetitions: 2 }\n  budget: { maximumCostUsd: 0 }\n  lifecycle: { maximumRuntimeSeconds: 1 }\n".to_vec()
    }

    fn llama_experiment() -> Vec<u8> {
        b"apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: { name: llama-cpp }\nspec:\n  provider: { kind: local }\n  runtime: { kind: llamaCpp, model: smollm2-135m-instruct-q2-k-v1, outputTokens: 4 }\n  workload: { profile: smollm2-chat-greedy-v1, requests: 2, concurrency: 1 }\n  measurement: { warmupRuns: 1, repetitions: 2 }\n  budget: { maximumCostUsd: 0 }\n  lifecycle: { maximumRuntimeSeconds: 1 }\n".to_vec()
    }

    fn execute_fixed(
        root: &Path,
        seed: u64,
        scenario: LocalFakeScenario,
        hook: &dyn RunHook,
    ) -> Result<RunResult, RunError> {
        run_experiment_with_services(
            &experiment(seed, scenario),
            &RunOptions {
                output_root: root.to_owned(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &FixedIds,
                hook,
                cpu_probe_executable: None,
                llama_cpp_executable: None,
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

    fn read_attempt(bundle: &Path) -> AttemptRecord {
        serde_json::from_slice(
            &fs::read(bundle.join("attempts/0001/attempt.json")).expect("read attempt record"),
        )
        .expect("parse attempt record")
    }

    fn read_provenance(bundle: &Path) -> AttemptProvenance {
        serde_json::from_slice(
            &fs::read(bundle.join("attempts/0001/provenance.json"))
                .expect("read attempt provenance"),
        )
        .expect("parse attempt provenance")
    }

    fn read_resources(bundle: &Path) -> AttemptResources {
        serde_json::from_slice(
            &fs::read(bundle.join("attempts/0001/resources.json")).expect("read attempt resources"),
        )
        .expect("parse attempt resources")
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
        assert!(result.resources.is_none());
        assert!(!bundle.join("attempts/0001/resources.json").exists());
        assert_eq!(read_attempt(&bundle).status, AttemptStatus::Succeeded);
        let provenance = read_provenance(&bundle);
        assert_eq!(provenance.format, ATTEMPT_PROVENANCE_FORMAT_V1);
        assert_eq!(provenance.run_id, FIXED_RUN_ID);
        assert_eq!(provenance.attempt_number, 1);
        assert!(!provenance.platform.operating_system.family.is_empty());
        assert!(!provenance.platform.kernel.name.is_empty());
        assert!(!provenance.platform.architecture.is_empty());
        assert_eq!(
            provenance
                .platform
                .cpu
                .logical_cpu_count
                .map(|count| count > 0),
            Some(true)
        );
        assert!(matches!(
            provenance.software.runtime,
            RuntimeProvenance::LocalFake { ref generator }
                if generator == LOCAL_FAKE_GENERATOR_VERSION
        ));
        assert_eq!(
            result.evidence_digest,
            crate::resolution::sha256_digest(
                &fs::read(bundle.join("SHA256SUMS")).expect("read checksum inventory")
            )
        );
        assert!(bundle.exists());
        assert!(!directory.0.join("staging").join(FIXED_RUN_ID).exists());
        verify_evidence_bundle(&bundle).expect("published bundle must verify");
    }

    #[test]
    fn checked_in_fixture_matches_fixed_execution() {
        let directory = TestDirectory::new("fixture-parity");
        let result = execute_fixed(&directory.0, 42, LocalFakeScenario::Success, &NoopRunHook)
            .expect("fixed execution should succeed");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/evidence/run-018f6f9a-7b3c-7abc-8def-0123456789ab");

        let mut historical = directory_files(&fixture);
        historical.remove(Path::new("SHA256SUMS"));
        let mut generated = directory_files(Path::new(&result.bundle_path));
        generated.remove(Path::new("SHA256SUMS"));
        assert!(generated
            .remove(Path::new("attempts/0001/provenance.json"))
            .is_some());
        assert_eq!(
            historical, generated,
            "checked-in evidence fixture must be regenerated from fixed execution"
        );
    }

    #[test]
    fn same_seed_is_deterministic_and_different_seed_changes_output() {
        let first = TestDirectory::new("same-seed-first");
        let second = TestDirectory::new("same-seed-second");
        let third = TestDirectory::new("different-seed");
        let noop = NoopRunHook;
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
        for (scenario, expected_state, expected_validity, expected_attempt, expected_failure) in [
            (
                LocalFakeScenario::RuntimeFailure,
                RunState::Failed,
                ValidityStatus::Indeterminate,
                AttemptStatus::Failed,
                Some(benchplane_schema::ERROR_LOCAL_FAKE_RUNTIME_FAILURE),
            ),
            (
                LocalFakeScenario::Interrupted,
                RunState::Interrupted,
                ValidityStatus::Indeterminate,
                AttemptStatus::Interrupted,
                Some(benchplane_schema::ERROR_LOCAL_FAKE_INTERRUPTED),
            ),
            (
                LocalFakeScenario::InsufficientMeasurements,
                RunState::Succeeded,
                ValidityStatus::Invalid,
                AttemptStatus::Succeeded,
                None,
            ),
        ] {
            let directory = TestDirectory::new("terminal-scenario");
            let result = execute_fixed(&directory.0, 42, scenario, &NoopRunHook)
                .expect("scenario should finalize");
            assert_eq!(result.run_state, expected_state);
            assert_eq!(result.validity_status, expected_validity);
            assert_eq!(
                read_attempt(Path::new(&result.bundle_path)).status,
                expected_attempt
            );
            assert_eq!(
                result.failure.as_ref().map(|failure| failure.code.as_str()),
                expected_failure
            );
            verify_evidence_bundle(Path::new(&result.bundle_path))
                .expect("terminal scenario bundle should verify");
        }
    }

    #[test]
    fn cpu_probe_spawn_failure_publishes_failed_evidence() {
        let directory = TestDirectory::new("cpu-probe-spawn-failure");
        let unspawnable = PathBuf::from("benchplane\0cpu-probe");
        let result = run_experiment_with_services(
            &cpu_experiment(),
            &RunOptions {
                output_root: directory.0.clone(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &FixedIds,
                hook: &NoopRunHook,
                cpu_probe_executable: Some(&unspawnable),
                llama_cpp_executable: None,
            },
        )
        .expect("runtime failure should finalize");
        assert_eq!(result.run_state, RunState::Failed);
        assert_eq!(result.validity_status, ValidityStatus::Indeterminate);
        assert!(result.resources.is_none());
        assert!(!Path::new(&result.bundle_path)
            .join("attempts/0001/resources.json")
            .exists());
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some(benchplane_schema::ERROR_CPU_PROBE_SPAWN_FAILED)
        );
        assert_eq!(
            read_attempt(Path::new(&result.bundle_path)).status,
            AttemptStatus::Failed
        );
        assert!(matches!(
            read_provenance(Path::new(&result.bundle_path)).software.runtime,
            RuntimeProvenance::CpuProbe { ref generator }
                if generator == CPU_PROBE_GENERATOR_VERSION
        ));
        verify_evidence_bundle(Path::new(&result.bundle_path)).expect("failed bundle verifies");
    }

    #[test]
    fn cpu_probe_nonzero_exit_publishes_failed_resource_evidence() {
        let directory = TestDirectory::new("cpu-probe-nonzero-resources");
        let exits_nonzero = PathBuf::from("false");
        let result = run_experiment_with_services(
            &cpu_experiment(),
            &RunOptions {
                output_root: directory.0.clone(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &FixedIds,
                hook: &NoopRunHook,
                cpu_probe_executable: Some(&exits_nonzero),
                llama_cpp_executable: None,
            },
        )
        .expect("runtime failure should finalize");
        assert_eq!(result.run_state, RunState::Failed);
        assert_eq!(result.validity_status, ValidityStatus::Indeterminate);
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some(benchplane_schema::ERROR_CPU_PROBE_EXIT_FAILED)
        );
        let observed = result.resources.expect("reaped child resources");
        let resources = read_resources(Path::new(&result.bundle_path));
        assert_eq!(resources.format, ATTEMPT_RESOURCES_FORMAT_V1);
        assert_eq!(resources.run_id, FIXED_RUN_ID);
        assert_eq!(resources.attempt_number, 1);
        assert_eq!(resources.scope, ResourceScope::HelperProcessLifetime);
        assert_eq!(resources.cpu_time_micros, observed.cpu_time_micros);
        assert_eq!(resources.peak_rss_bytes, observed.peak_rss_bytes);
        assert!(read_measurements(Path::new(&result.bundle_path)).is_empty());
        verify_evidence_bundle(Path::new(&result.bundle_path))
            .expect("failed resource bundle verifies");
    }

    #[test]
    fn llama_cpp_spawn_failure_publishes_failed_evidence() {
        let directory = TestDirectory::new("llama-cpp-spawn-failure");
        let unspawnable = PathBuf::from("benchplane\0llama-cpp");
        let result = run_experiment_with_services(
            &llama_experiment(),
            &RunOptions {
                output_root: directory.0.clone(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &FixedIds,
                hook: &NoopRunHook,
                cpu_probe_executable: None,
                llama_cpp_executable: Some(&unspawnable),
            },
        )
        .expect("runtime failure should finalize");
        assert_eq!(result.run_state, RunState::Failed);
        assert_eq!(result.validity_status, ValidityStatus::Indeterminate);
        assert!(result.resources.is_none());
        assert!(!Path::new(&result.bundle_path)
            .join("attempts/0001/resources.json")
            .exists());
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some(benchplane_schema::ERROR_LLAMA_CPP_SPAWN_FAILED)
        );
        assert_eq!(
            read_attempt(Path::new(&result.bundle_path)).status,
            AttemptStatus::Failed
        );
        let provenance = read_provenance(Path::new(&result.bundle_path));
        assert!(matches!(
            provenance.software.runtime,
            RuntimeProvenance::LlamaCpp {
                ref generator,
                ref engine,
                ref model,
                ..
            } if generator == LLAMA_CPP_GENERATOR_VERSION
                && engine.version == LLAMA_CPP_ENGINE_VERSION
                && model.identity == LLAMA_CPP_MODEL_IDENTITY
                && model.sha256 == LLAMA_CPP_MODEL_SHA256
        ));
        verify_evidence_bundle(Path::new(&result.bundle_path)).expect("failed bundle verifies");
    }

    #[test]
    fn output_paths_with_spaces_and_unicode_work() {
        let directory = TestDirectory::new("path-parent");
        let root = directory.0.join("output with spaces λ");
        let result = execute_fixed(&root, 42, LocalFakeScenario::Success, &NoopRunHook)
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
        assert!(error.to_string().contains(FIXED_RUN_ID));
        assert!(error.to_string().contains(
            &directory
                .0
                .join("staging")
                .join(FIXED_RUN_ID)
                .display()
                .to_string()
        ));
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
        assert_eq!(
            read_attempt(&directory.0.join("staging").join(FIXED_RUN_ID)).status,
            AttemptStatus::Succeeded
        );
        assert!(matches!(
            verify_evidence_bundle(&directory.0.join("staging").join(FIXED_RUN_ID)),
            Err(EvidenceError::ChecksumMismatch(path)) if path == "run.json"
        ));
    }

    #[test]
    fn failures_before_and_during_finalization_retain_allocated_run_context() {
        for checkpoint in [
            RunCheckpoint::MeasurementAppend,
            RunCheckpoint::Verification,
        ] {
            let directory = TestDirectory::new("allocated-failure-context");
            let error = execute_fixed(
                &directory.0,
                42,
                LocalFakeScenario::Success,
                &FailAt(checkpoint),
            )
            .expect_err("injected failure must be reported");
            let staging = directory.0.join("staging").join(FIXED_RUN_ID);
            assert!(error.is_finalization_failure());
            assert!(error.to_string().contains(FIXED_RUN_ID));
            assert!(error.to_string().contains(&staging.display().to_string()));
            assert!(staging.exists());
            assert!(!directory.0.join("runs").join(FIXED_RUN_ID).exists());
            assert!(verify_evidence_bundle(&staging).is_err());
            assert_eq!(read_attempt(&staging).status, AttemptStatus::Succeeded);
        }
    }

    #[test]
    fn publication_destination_conflict_retains_staging() {
        let directory = TestDirectory::new("publication-conflict");
        let destination = directory.0.join("runs").join(FIXED_RUN_ID);
        fs::create_dir_all(&destination).expect("create conflicting destination");
        fs::write(destination.join("marker"), b"preexisting\n").expect("write marker");

        let error = execute_fixed(&directory.0, 42, LocalFakeScenario::Success, &NoopRunHook)
            .expect_err("destination conflict must fail publication");
        let staging = directory.0.join("staging").join(FIXED_RUN_ID);
        assert!(error.is_finalization_failure());
        assert!(error.to_string().contains(FIXED_RUN_ID));
        assert!(staging.exists());
        assert!(destination.join("marker").exists());
        assert!(!destination.join("manifest.json").exists());
        assert!(verify_evidence_bundle(&staging).is_err());
        assert_eq!(read_attempt(&staging).status, AttemptStatus::Succeeded);
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
                hook: &NoopRunHook,
                cpu_probe_executable: None,
                llama_cpp_executable: None,
            },
        )
        .expect_err("unsupported combination should be rejected");
        assert!(error.is_request_rejection());
    }

    #[test]
    fn excessive_local_fake_work_is_rejected_before_run_id_allocation() {
        struct PanickingIds;
        impl RunIdGenerator for PanickingIds {
            fn next_run_id(&self) -> String {
                panic!("run ID must not be allocated")
            }
        }

        let bytes = b"apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: { name: excessive }\nspec:\n  provider: { kind: localFake }\n  runtime: { kind: localFake }\n  workload: { profile: smoke, requests: 1 }\n  measurement: { warmupRuns: 10000, repetitions: 1 }\n  budget: { maximumCostUsd: 0 }\n";
        let directory = TestDirectory::new("excessive");
        let error = run_experiment_with_services(
            bytes,
            &RunOptions {
                output_root: directory.0.clone(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &PanickingIds,
                hook: &NoopRunHook,
                cpu_probe_executable: None,
                llama_cpp_executable: None,
            },
        )
        .expect_err("excessive work should be rejected");
        assert!(error.is_request_rejection());
        assert!(!directory.0.join("staging").exists());
    }

    #[test]
    fn excessive_cpu_probe_work_is_rejected_before_run_id_allocation() {
        struct PanickingIds;
        impl RunIdGenerator for PanickingIds {
            fn next_run_id(&self) -> String {
                panic!("run ID must not be allocated")
            }
        }

        let bytes = b"apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: { name: excessive-cpu }\nspec:\n  provider: { kind: local }\n  runtime: { kind: cpuProbe, outputTokens: 100, workUnitsPerToken: 100000 }\n  workload: { profile: cpu-token-probe-v1, requests: 11, concurrency: 1 }\n  measurement: { warmupRuns: 1, repetitions: 3 }\n  budget: { maximumCostUsd: 0 }\n";
        let directory = TestDirectory::new("excessive-cpu");
        let error = run_experiment_with_services(
            bytes,
            &RunOptions {
                output_root: directory.0.clone(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &PanickingIds,
                hook: &NoopRunHook,
                cpu_probe_executable: None,
                llama_cpp_executable: None,
            },
        )
        .expect_err("excessive work should be rejected");
        assert!(error.is_request_rejection());
        assert!(!directory.0.join("staging").exists());
    }

    #[test]
    fn excessive_cpu_probe_records_are_rejected_before_run_id_allocation() {
        struct PanickingIds;
        impl RunIdGenerator for PanickingIds {
            fn next_run_id(&self) -> String {
                panic!("run ID must not be allocated")
            }
        }

        let bytes = b"apiVersion: benchplane/v1alpha1\nkind: Experiment\nmetadata: { name: excessive-cpu-records }\nspec:\n  provider: { kind: local }\n  runtime: { kind: cpuProbe, outputTokens: 1, workUnitsPerToken: 1 }\n  workload: { profile: cpu-token-probe-v1, requests: 1, concurrency: 1 }\n  measurement: { warmupRuns: 1, repetitions: 1000 }\n  budget: { maximumCostUsd: 0 }\n";
        let directory = TestDirectory::new("excessive-cpu-records");
        let error = run_experiment_with_services(
            bytes,
            &RunOptions {
                output_root: directory.0.clone(),
            },
            &RunServices {
                clock: &FixedClock,
                ids: &PanickingIds,
                hook: &NoopRunHook,
                cpu_probe_executable: None,
                llama_cpp_executable: None,
            },
        )
        .expect_err("excessive records should be rejected");
        assert!(error.is_request_rejection());
        assert!(!directory.0.join("staging").exists());
    }
}
