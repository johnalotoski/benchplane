// SPDX-License-Identifier: Apache-2.0

use crate::{
    child_supervisor::{self, ChildExecution, ChildProtocol, ExitCodeFailure},
    execution::ExecutionOutput,
};
use benchplane_schema::{
    FailureRecord, MeasurementPhase, MeasurementRecord, RunState,
    ERROR_LLAMA_CPP_DEADLINE_EXCEEDED, ERROR_LLAMA_CPP_EXIT_FAILED,
    ERROR_LLAMA_CPP_MODEL_INIT_FAILED, ERROR_LLAMA_CPP_OUTPUT_INVALID,
    ERROR_LLAMA_CPP_RESOURCE_ACCOUNTING_FAILED, ERROR_LLAMA_CPP_SPAWN_FAILED,
    LLAMA_CPP_GENERATOR_VERSION, MAX_LLAMA_CPP_RECORDS,
};
use std::{path::Path, process::Command};

const MAX_SERIALIZED_RECORD_BYTES: usize = 512;
const MAX_STDOUT_BYTES: usize = 64 * 1024;
const MODEL_INIT_EXIT_CODE: i32 = 20;
const SPECIAL_EXIT_CODES: &[ExitCodeFailure] = &[ExitCodeFailure {
    exit_code: MODEL_INIT_EXIT_CODE,
    failure_code: ERROR_LLAMA_CPP_MODEL_INIT_FAILED,
    message: "packaged llama.cpp helper could not initialize its fixed model",
}];

#[derive(Debug, Clone, Copy)]
pub(crate) struct LlamaCppConfig {
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
    into_execution_output(execute_command(command, config))
}

fn into_execution_output(result: ChildExecution) -> ExecutionOutput {
    let terminal_state = if result.failure.is_none() {
        RunState::Succeeded
    } else {
        RunState::Failed
    };
    ExecutionOutput {
        measurements: result.measurements,
        resources: result.resources,
        terminal_state,
        failure: result.failure,
    }
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
    config
        .warmup_runs
        .checked_add(config.repetitions)
        .filter(|count| *count <= MAX_LLAMA_CPP_RECORDS)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| protocol_failure("llama.cpp requested excessive measurement records"))
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
        && record.failed_requests == 0;
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
    use std::{
        fs,
        path::PathBuf,
        sync::OnceLock,
        time::{Duration, Instant},
    };

    fn config() -> LlamaCppConfig {
        LlamaCppConfig {
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
        }
    }

    #[test]
    fn validates_identity_order_metrics_and_counts() {
        assert!(validate_record(&record(MeasurementPhase::Warmup, 1), 0, config()).is_ok());
        assert!(validate_record(&record(MeasurementPhase::Measured, 1), 1, config()).is_ok());
        let mut wrong = record(MeasurementPhase::Measured, 2);
        wrong.successful_requests = 1;
        assert!(validate_record(&wrong, 2, config()).is_err());
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
    println!(r#"{{"generator":"benchplane-llama-cpp-smollm2/v1","attemptNumber":1,"phase":"{}","repetitionIndex":{},"sampleIndex":1,"latencyMicros":20,"timeToFirstTokenMicros":10,"throughputMilliRequestsPerSecond":1000,"successfulRequests":2,"failedRequests":0}}"#, phase, index);
}
fn main() {
    match env::var("LLAMA_TEST_MODE").as_deref() {
        Ok("success") => { burn_cpu(); record("warmup", 1); record("measured", 1); record("measured", 2); }
        Ok("missing") => { record("warmup", 1); record("measured", 1); }
        Ok("duplicate") => { record("warmup", 1); record("warmup", 1); record("measured", 2); }
        Ok("malformed") => println!("not-json"),
        Ok("partial-malformed") => { record("warmup", 1); println!("not-json"); }
        Ok("partial-nonzero") => { record("warmup", 1); std::process::exit(7); }
        Ok("model-init") => std::process::exit(20),
        Ok("nonzero") => { eprintln!("private injected failure"); std::process::exit(7); }
        Ok("oversized") => println!("{}", "x".repeat(513)),
        Ok("excessive-total") => print!("{}", "x".repeat(70000)),
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
        assert_eq!(outcome.measurements[2].repetition_index, 2);
        assert!(outcome.resources.is_some());
    }

    #[test]
    fn malformed_missing_duplicate_and_output_bounds_fail() {
        for mode in [
            "malformed",
            "missing",
            "duplicate",
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
            let output = into_execution_output(injected(mode));
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
        let output = into_execution_output(injected("deadline"));
        assert_eq!(output.terminal_state, RunState::Failed);
        assert!(output.measurements.is_empty());
        assert_eq!(
            output.failure.expect("failure").code,
            ERROR_LLAMA_CPP_DEADLINE_EXCEEDED
        );
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
