// SPDX-License-Identifier: Apache-2.0

use crate::{
    child_supervisor::{self, ChildExecution, ChildProtocol, ExitCodeFailure},
    execution::ExecutionOutput,
    provenance::nvidia_driver_version_supported,
};
use benchplane_schema::{
    FailureRecord, LlamaCppTarget, MeasurementPhase, MeasurementRecord, NvidiaGpuProvenance,
    RunState, ERROR_LLAMA_CPP_DEADLINE_EXCEEDED, ERROR_LLAMA_CPP_EXIT_FAILED,
    ERROR_LLAMA_CPP_MODEL_INIT_FAILED, ERROR_LLAMA_CPP_OUTPUT_INVALID,
    ERROR_LLAMA_CPP_RESOURCE_ACCOUNTING_FAILED, ERROR_LLAMA_CPP_SPAWN_FAILED,
    LLAMA_CPP_GENERATOR_VERSION, MAX_LLAMA_CPP_RECORDS, MAX_LLAMA_CPP_REQUEST_OBSERVATIONS,
};
use serde::Deserialize;
use std::{path::Path, process::Command};

const MAX_SERIALIZED_RECORD_BYTES: usize = 16 * 1024;
// One NVIDIA metadata preamble plus the maximum 16 repetition records.
const MAX_STDOUT_BYTES: usize = 17 * MAX_SERIALIZED_RECORD_BYTES;
const MODEL_INIT_EXIT_CODE: i32 = 20;
const NVIDIA_METADATA_FORMAT: &str = "benchplane-llama-cpp-nvidia/v1";
const SPECIAL_EXIT_CODES: &[ExitCodeFailure] = &[ExitCodeFailure {
    exit_code: MODEL_INIT_EXIT_CODE,
    failure_code: ERROR_LLAMA_CPP_MODEL_INIT_FAILED,
    message: "packaged llama.cpp helper could not initialize its fixed model",
}];

#[derive(Debug, Clone, Copy)]
pub(crate) struct LlamaCppConfig {
    pub target: LlamaCppTarget,
    pub requests: u32,
    pub warmup_runs: u32,
    pub repetitions: u32,
    pub output_tokens: u32,
    pub maximum_runtime_seconds: u64,
}

pub(crate) fn execute(executable: &Path, config: LlamaCppConfig) -> ExecutionOutput {
    let mut command = Command::new(executable);
    // The fixed runtime needs no ambient configuration. In particular, clearing
    // the environment prevents loader and ggml backend variables from changing
    // the packaged executable/library boundary or measured work. The helper also
    // passes its compiled package-owned directory to ggml's explicit-path loader.
    command.env_clear();
    into_execution_output(execute_command(command, config), config)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NvidiaMetadataRecord {
    format: String,
    nvidia: NvidiaGpuProvenance,
}

fn into_execution_output(result: ChildExecution, config: LlamaCppConfig) -> ExecutionOutput {
    if result.failure.is_none() {
        let nvidia_gpu = match config.target {
            LlamaCppTarget::Cpu if result.metadata.is_empty() => None,
            LlamaCppTarget::NvidiaCuda if result.metadata.len() == 1 => {
                match parse_nvidia_metadata(&result.metadata[0]) {
                    Ok(metadata) => Some(metadata),
                    Err(message) => {
                        return ExecutionOutput {
                            measurements: Vec::new(),
                            resources: result.resources,
                            nvidia_gpu: None,
                            terminal_state: RunState::Failed,
                            failure: Some(protocol_failure(&message)),
                        };
                    }
                }
            }
            _ => {
                return ExecutionOutput {
                    measurements: Vec::new(),
                    resources: result.resources,
                    nvidia_gpu: None,
                    terminal_state: RunState::Failed,
                    failure: Some(protocol_failure(
                        "llama.cpp helper metadata does not match the requested target",
                    )),
                };
            }
        };
        return ExecutionOutput {
            measurements: result.measurements,
            resources: result.resources,
            nvidia_gpu,
            terminal_state: RunState::Succeeded,
            failure: None,
        };
    }
    let terminal_state = if result.failure.is_none() {
        RunState::Succeeded
    } else {
        RunState::Failed
    };
    ExecutionOutput {
        measurements: result.measurements,
        resources: result.resources,
        nvidia_gpu: None,
        terminal_state,
        failure: result.failure,
    }
}

fn parse_nvidia_metadata(bytes: &[u8]) -> Result<NvidiaGpuProvenance, String> {
    let record: NvidiaMetadataRecord = serde_json::from_slice(bytes)
        .map_err(|error| format!("llama.cpp helper emitted malformed NVIDIA metadata: {error}"))?;
    let gpu = record.nvidia;
    let bounded = |value: &str| {
        !value.is_empty()
            && value.trim() == value
            && value.len() <= 256
            && !value.chars().any(char::is_control)
    };
    let compute_capability_valid =
        gpu.compute_capability
            .split_once('.')
            .is_some_and(|(major, minor)| {
                !major.is_empty()
                    && !minor.is_empty()
                    && major.bytes().all(|byte| byte.is_ascii_digit())
                    && minor.bytes().all(|byte| byte.is_ascii_digit())
            });
    if record.format != NVIDIA_METADATA_FORMAT
        || gpu.vendor != "NVIDIA"
        || !bounded(&gpu.device_name)
        || gpu.logical_device_index != 0
        || gpu.total_vram_bytes == 0
        || !bounded(&gpu.nvidia_driver_version)
        || !nvidia_driver_version_supported(&gpu.nvidia_driver_version)
        || !bounded(&gpu.cuda_driver_version)
        || !bounded(&gpu.cuda_runtime_version)
        || !bounded(&gpu.cuda_toolkit_version)
        || !compute_capability_valid
        || gpu.offload.policy != "singleDeviceAllLayers"
        || gpu.offload.total_layers == 0
        || gpu.offload.offloaded_layers != gpu.offload.total_layers
    {
        return Err(
            "llama.cpp helper NVIDIA metadata is inconsistent with the fixed target".to_owned(),
        );
    }
    Ok(gpu)
}

fn execute_command(mut command: Command, config: LlamaCppConfig) -> ChildExecution {
    let expected_records = match expected_record_count(config) {
        Ok(count) => count,
        Err(failure) => return ChildExecution::failed(failure, None),
    };
    command
        .arg("--requests")
        .arg(config.requests.to_string())
        .arg("--warmup-runs")
        .arg(config.warmup_runs.to_string())
        .arg("--repetitions")
        .arg(config.repetitions.to_string())
        .arg("--output-tokens")
        .arg(config.output_tokens.to_string());

    child_supervisor::execute(
        command,
        ChildProtocol {
            runtime_name: "llama.cpp SmolLM2",
            expected_metadata_records: usize::from(config.target == LlamaCppTarget::NvidiaCuda),
            expected_records,
            max_record_bytes: MAX_SERIALIZED_RECORD_BYTES,
            max_stdout_bytes: MAX_STDOUT_BYTES,
            maximum_runtime_seconds: config.maximum_runtime_seconds,
            spawn_failure_code: ERROR_LLAMA_CPP_SPAWN_FAILED,
            exit_failure_code: ERROR_LLAMA_CPP_EXIT_FAILED,
            output_failure_code: ERROR_LLAMA_CPP_OUTPUT_INVALID,
            deadline_failure_code: ERROR_LLAMA_CPP_DEADLINE_EXCEEDED,
            resource_failure_code: ERROR_LLAMA_CPP_RESOURCE_ACCOUNTING_FAILED,
            special_exit_codes: SPECIAL_EXIT_CODES,
        },
        |record, position| validate_record(record, position, config),
    )
}

fn expected_record_count(config: LlamaCppConfig) -> Result<usize, FailureRecord> {
    let count = config
        .warmup_runs
        .checked_add(config.repetitions)
        .filter(|count| *count <= MAX_LLAMA_CPP_RECORDS)
        .ok_or_else(|| protocol_failure("llama.cpp requested excessive measurement records"))?;
    u64::from(count)
        .checked_mul(u64::from(config.requests))
        .filter(|observations| *observations <= MAX_LLAMA_CPP_REQUEST_OBSERVATIONS)
        .ok_or_else(|| protocol_failure("llama.cpp requested excessive request observations"))?;
    let count = usize::try_from(count)
        .map_err(|_| protocol_failure("llama.cpp measurement count is not representable"))?;
    count
        .checked_mul(MAX_SERIALIZED_RECORD_BYTES)
        .filter(|bytes| *bytes <= MAX_STDOUT_BYTES)
        .ok_or_else(|| {
            protocol_failure("llama.cpp requested output exceeds its transport bound")
        })?;
    Ok(count)
}

fn validate_record(
    record: &MeasurementRecord,
    position: usize,
    config: LlamaCppConfig,
) -> Result<(), String> {
    let (phase, repetition_index) = if position < config.warmup_runs as usize {
        (MeasurementPhase::Warmup, position as u32 + 1)
    } else {
        (
            MeasurementPhase::Measured,
            position as u32 - config.warmup_runs + 1,
        )
    };
    let valid = record.generator == LLAMA_CPP_GENERATOR_VERSION
        && record.attempt_number == 1
        && record.phase == phase
        && record.repetition_index == repetition_index
        && record.sample_index == 1
        && record.latency_micros > 0
        && record.time_to_first_token_micros > 0
        && record.time_to_first_token_micros <= record.latency_micros
        && record.throughput_milli_requests_per_second > 0
        && record.successful_requests == config.requests
        && record.failed_requests == 0
        && record.request_observations.len() == config.requests as usize
        && record
            .request_observations
            .iter()
            .enumerate()
            .all(|(position, observation)| {
                observation.request_index == position as u32 + 1
                    && observation.latency_micros > 0
                    && observation.time_to_first_token_micros > 0
                    && observation.time_to_first_token_micros <= observation.latency_micros
            });
    if valid {
        Ok(())
    } else {
        Err(format!(
            "llama.cpp record {} does not match the requested execution",
            position + 1
        ))
    }
}

fn protocol_failure(message: &str) -> FailureRecord {
    FailureRecord {
        phase: "running".to_owned(),
        code: ERROR_LLAMA_CPP_OUTPUT_INVALID.to_owned(),
        message: message.to_owned(),
        retryable: false,
        attempt_number: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use benchplane_schema::{ProcessResources, RequestObservation};
    use std::{
        fs,
        path::PathBuf,
        sync::OnceLock,
        time::{Duration, Instant},
    };

    fn config() -> LlamaCppConfig {
        LlamaCppConfig {
            target: LlamaCppTarget::Cpu,
            requests: 2,
            warmup_runs: 1,
            repetitions: 2,
            output_tokens: 4,
            maximum_runtime_seconds: 1,
        }
    }

    fn record(phase: MeasurementPhase, repetition_index: u32) -> MeasurementRecord {
        MeasurementRecord {
            generator: LLAMA_CPP_GENERATOR_VERSION.to_owned(),
            attempt_number: 1,
            phase,
            repetition_index,
            sample_index: 1,
            latency_micros: 20,
            time_to_first_token_micros: 10,
            throughput_milli_requests_per_second: 1000,
            successful_requests: 2,
            failed_requests: 0,
            request_observations: vec![
                RequestObservation {
                    request_index: 1,
                    latency_micros: 19,
                    time_to_first_token_micros: 9,
                },
                RequestObservation {
                    request_index: 2,
                    latency_micros: 21,
                    time_to_first_token_micros: 11,
                },
            ],
        }
    }

    fn nvidia_metadata() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "format": NVIDIA_METADATA_FORMAT,
            "nvidia": {
                "vendor": "NVIDIA",
                "deviceName": "Test NVIDIA GPU",
                "logicalDeviceIndex": 0,
                "totalVramBytes": 8589934592_u64,
                "nvidiaDriverVersion": "610.43.03",
                "cudaDriverVersion": "13.0",
                "cudaRuntimeVersion": "13.0",
                "cudaToolkitVersion": "13.0",
                "computeCapability": "8.9",
                "offload": {
                    "policy": "singleDeviceAllLayers",
                    "offloadedLayers": 31,
                    "totalLayers": 31
                }
            }
        }))
        .expect("serialize NVIDIA metadata")
    }

    #[test]
    fn nvidia_success_requires_bounded_complete_metadata() {
        let mut gpu_config = config();
        gpu_config.target = LlamaCppTarget::NvidiaCuda;
        let successful = ChildExecution {
            metadata: vec![nvidia_metadata()],
            measurements: vec![record(MeasurementPhase::Measured, 1)],
            resources: Some(ProcessResources {
                cpu_time_micros: 1,
                peak_rss_bytes: 1024,
            }),
            failure: None,
        };
        let output = into_execution_output(successful, gpu_config);
        assert_eq!(output.terminal_state, RunState::Succeeded);
        assert_eq!(
            output
                .nvidia_gpu
                .expect("NVIDIA provenance")
                .offload
                .offloaded_layers,
            31
        );

        let mut old_driver: serde_json::Value =
            serde_json::from_slice(&nvidia_metadata()).expect("parse NVIDIA metadata");
        old_driver["nvidia"]["nvidiaDriverVersion"] = serde_json::json!("595.84");
        assert!(parse_nvidia_metadata(
            &serde_json::to_vec(&old_driver).expect("serialize old-driver metadata")
        )
        .is_err());

        for metadata in [Vec::new(), vec![b"{}".to_vec()]] {
            let invalid = ChildExecution {
                metadata,
                measurements: vec![record(MeasurementPhase::Measured, 1)],
                resources: Some(ProcessResources {
                    cpu_time_micros: 1,
                    peak_rss_bytes: 1024,
                }),
                failure: None,
            };
            let output = into_execution_output(invalid, gpu_config);
            assert_eq!(output.terminal_state, RunState::Failed);
            assert!(output.measurements.is_empty());
            assert!(output.resources.is_some());
            assert_eq!(
                output.failure.expect("protocol failure").code,
                ERROR_LLAMA_CPP_OUTPUT_INVALID
            );
        }
    }

    #[test]
    fn validates_identity_order_metrics_and_counts() {
        assert!(validate_record(&record(MeasurementPhase::Warmup, 1), 0, config()).is_ok());
        assert!(validate_record(&record(MeasurementPhase::Measured, 1), 1, config()).is_ok());
        let mut wrong = record(MeasurementPhase::Measured, 2);
        wrong.successful_requests = 1;
        assert!(validate_record(&wrong, 2, config()).is_err());
        let mut missing = record(MeasurementPhase::Measured, 2);
        missing.request_observations.pop();
        assert!(validate_record(&missing, 2, config()).is_err());
        let mut out_of_order = record(MeasurementPhase::Measured, 2);
        out_of_order.request_observations.swap(0, 1);
        assert!(validate_record(&out_of_order, 2, config()).is_err());
    }

    #[test]
    fn maximum_request_observations_fit_the_bounded_transport() {
        let maximum_observations = usize::try_from(MAX_LLAMA_CPP_REQUEST_OBSERVATIONS)
            .expect("observation bound fits usize");
        let mut maximum_record = record(MeasurementPhase::Measured, 1);
        maximum_record.request_observations = (1..=maximum_observations)
            .map(|index| RequestObservation {
                request_index: index as u32,
                latency_micros: u64::MAX,
                time_to_first_token_micros: u64::MAX,
            })
            .collect();
        let serialized = serde_json::to_vec(&maximum_record)
            .expect("serialize maximum request-observation record");
        assert!(serialized.len() < MAX_SERIALIZED_RECORD_BYTES);

        let maximum = LlamaCppConfig {
            target: LlamaCppTarget::Cpu,
            requests: MAX_LLAMA_CPP_REQUEST_OBSERVATIONS as u32,
            warmup_runs: 0,
            repetitions: 1,
            output_tokens: 1,
            maximum_runtime_seconds: 1,
        };
        assert_eq!(expected_record_count(maximum).expect("maximum fits"), 1);
        assert!(
            (MAX_LLAMA_CPP_RECORDS as usize + 1) * MAX_SERIALIZED_RECORD_BYTES <= MAX_STDOUT_BYTES
        );
        let mut excessive = maximum;
        excessive.requests += 1;
        assert!(expected_record_count(excessive).is_err());
    }

    #[test]
    fn spawn_failure_is_a_runtime_failure() {
        let output = execute(&PathBuf::from("benchplane\0llama-cpp"), config());
        assert_eq!(output.terminal_state, RunState::Failed);
        assert_eq!(
            output.failure.expect("failure").code,
            ERROR_LLAMA_CPP_SPAWN_FAILED
        );
    }

    fn test_child() -> &'static Path {
        static CHILD: OnceLock<PathBuf> = OnceLock::new();
        CHILD
            .get_or_init(|| {
                let directory = std::env::temp_dir().join(format!(
                    "benchplane-llama-adapter-test-{}",
                    std::process::id()
                ));
                fs::create_dir_all(&directory).expect("create test child directory");
                let source = directory.join("child.rs");
                let executable = directory.join("child");
                fs::write(
                    &source,
                    r###"
use std::{env, thread, time::Duration};
fn burn_cpu() {
    let mut value = 1_u64;
    for index in 0..5_000_000_u64 {
        value = value.wrapping_mul(6364136223846793005).wrapping_add(index);
    }
    std::hint::black_box(value);
}
fn record(phase: &str, index: u32) {
    println!(r#"{{"generator":"benchplane-llama-cpp-smollm2/v2","attemptNumber":1,"phase":"{}","repetitionIndex":{},"sampleIndex":1,"latencyMicros":20,"timeToFirstTokenMicros":10,"throughputMilliRequestsPerSecond":1000,"successfulRequests":2,"failedRequests":0,"requestObservations":[{{"requestIndex":1,"latencyMicros":19,"timeToFirstTokenMicros":9}},{{"requestIndex":2,"latencyMicros":21,"timeToFirstTokenMicros":11}}]}}"#, phase, index);
}
fn invalid_observations(value: &str) {
    println!(r#"{{"generator":"benchplane-llama-cpp-smollm2/v2","attemptNumber":1,"phase":"warmup","repetitionIndex":1,"sampleIndex":1,"latencyMicros":20,"timeToFirstTokenMicros":10,"throughputMilliRequestsPerSecond":1000,"successfulRequests":2,"failedRequests":0,"requestObservations":{}}}"#, value);
}
fn main() {
    match env::var("LLAMA_TEST_MODE").as_deref() {
        Ok("success") => { burn_cpu(); record("warmup", 1); record("measured", 1); record("measured", 2); }
        Ok("missing") => { record("warmup", 1); record("measured", 1); }
        Ok("duplicate") => { record("warmup", 1); record("warmup", 1); record("measured", 2); }
        Ok("malformed") => println!("not-json"),
        Ok("missing-observations") => invalid_observations("[]"),
        Ok("extra-observation") => invalid_observations(r#"[{"requestIndex":1,"latencyMicros":19,"timeToFirstTokenMicros":9},{"requestIndex":2,"latencyMicros":21,"timeToFirstTokenMicros":11},{"requestIndex":3,"latencyMicros":22,"timeToFirstTokenMicros":12}]"#),
        Ok("observation-order") => invalid_observations(r#"[{"requestIndex":2,"latencyMicros":19,"timeToFirstTokenMicros":9},{"requestIndex":1,"latencyMicros":21,"timeToFirstTokenMicros":11}]"#),
        Ok("observation-metric") => invalid_observations(r#"[{"requestIndex":1,"latencyMicros":9,"timeToFirstTokenMicros":10},{"requestIndex":2,"latencyMicros":21,"timeToFirstTokenMicros":11}]"#),
        Ok("partial-malformed") => { record("warmup", 1); println!("not-json"); }
        Ok("partial-nonzero") => { record("warmup", 1); std::process::exit(7); }
        Ok("model-init") => std::process::exit(20),
        Ok("nonzero") => { eprintln!("private injected failure"); std::process::exit(7); }
        Ok("oversized") => println!("{}", "x".repeat(16385)),
        Ok("excessive-total") => print!("{}", "x".repeat(270000)),
        Ok("deadline") => { record("warmup", 1); thread::sleep(Duration::from_secs(5)); }
        _ => std::process::exit(9),
    }
}
"###,
                )
                .expect("write test child source");
                let status = Command::new("rustc")
                    .arg(&source)
                    .arg("-o")
                    .arg(&executable)
                    .status()
                    .expect("run rustc for test child");
                assert!(status.success(), "compile test child");
                executable
            })
            .as_path()
    }

    fn injected(mode: &str) -> ChildExecution {
        let mut command = Command::new(test_child());
        command.env("LLAMA_TEST_MODE", mode);
        execute_command(command, config())
    }

    #[test]
    fn complete_protocol_succeeds() {
        let outcome = injected("success");
        assert!(outcome.failure.is_none());
        assert_eq!(outcome.measurements.len(), 3);
        assert_eq!(outcome.measurements[0].phase, MeasurementPhase::Warmup);
        assert_eq!(outcome.measurements[0].request_observations.len(), 2);
        assert_eq!(
            outcome.measurements[0].request_observations[1].request_index,
            2
        );
        assert_eq!(outcome.measurements[2].repetition_index, 2);
        assert!(outcome.resources.is_some());
    }

    #[test]
    fn malformed_missing_duplicate_and_output_bounds_fail() {
        for mode in [
            "malformed",
            "missing",
            "duplicate",
            "missing-observations",
            "extra-observation",
            "observation-order",
            "observation-metric",
            "oversized",
            "excessive-total",
        ] {
            assert_eq!(
                injected(mode)
                    .failure
                    .expect("invalid protocol must fail")
                    .code,
                ERROR_LLAMA_CPP_OUTPUT_INVALID,
                "mode={mode}"
            );
        }
    }

    #[test]
    fn model_initialization_and_other_nonzero_exits_are_distinct() {
        assert_eq!(
            injected("model-init")
                .failure
                .expect("model init must fail")
                .code,
            ERROR_LLAMA_CPP_MODEL_INIT_FAILED
        );
        assert_eq!(
            injected("nonzero").failure.expect("nonzero must fail").code,
            ERROR_LLAMA_CPP_EXIT_FAILED
        );
    }

    #[test]
    fn failed_complete_protocol_discards_a_valid_prefix() {
        for (mode, code) in [
            ("partial-nonzero", ERROR_LLAMA_CPP_EXIT_FAILED),
            ("partial-malformed", ERROR_LLAMA_CPP_OUTPUT_INVALID),
        ] {
            let output = into_execution_output(injected(mode), config());
            assert_eq!(output.terminal_state, RunState::Failed, "mode={mode}");
            assert!(output.measurements.is_empty(), "mode={mode}");
            assert_eq!(output.failure.expect("failure").code, code, "mode={mode}");
        }
    }

    #[test]
    fn failed_and_timed_out_children_retain_exact_resources() {
        for mode in ["nonzero", "partial-malformed", "deadline"] {
            assert!(injected(mode).resources.is_some(), "mode={mode}");
        }
    }

    #[test]
    fn deadline_terminates_and_reaps_child() {
        let started = Instant::now();
        let output = into_execution_output(injected("deadline"), config());
        assert_eq!(output.terminal_state, RunState::Failed);
        assert!(output.measurements.is_empty());
        assert_eq!(
            output.failure.expect("failure").code,
            ERROR_LLAMA_CPP_DEADLINE_EXCEEDED
        );
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
