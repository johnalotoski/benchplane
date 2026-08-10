// SPDX-License-Identifier: Apache-2.0

use crate::{
    child_supervisor::{self, ChildExecution, ChildProtocol},
    execution::ExecutionOutput,
};
use benchplane_schema::{
    FailureRecord, MeasurementPhase, MeasurementRecord, RunState, CPU_PROBE_GENERATOR_VERSION,
    ERROR_CPU_PROBE_DEADLINE_EXCEEDED, ERROR_CPU_PROBE_EXIT_FAILED, ERROR_CPU_PROBE_OUTPUT_INVALID,
    ERROR_CPU_PROBE_RESOURCE_ACCOUNTING_FAILED, ERROR_CPU_PROBE_SPAWN_FAILED,
    MAX_CPU_PROBE_RECORDS,
};
use std::{path::Path, process::Command};

const MAX_STDOUT_BYTES: usize = 1024 * 1024;
/// Includes one serialized measurement record and its trailing newline.
const MAX_SERIALIZED_RECORD_BYTES: usize = 512;
const MAX_ACCEPTED_RECORD_OUTPUT_BYTES: usize =
    match (MAX_CPU_PROBE_RECORDS as usize).checked_mul(MAX_SERIALIZED_RECORD_BYTES) {
        Some(value) => value,
        None => panic!("CPU probe accepted output bound overflowed"),
    };
const _: () = assert!(MAX_ACCEPTED_RECORD_OUTPUT_BYTES <= MAX_STDOUT_BYTES);

#[derive(Debug, Clone, Copy)]
pub(crate) struct CpuProbeConfig {
    pub requests: u32,
    pub warmup_runs: u32,
    pub repetitions: u32,
    pub output_tokens: u32,
    pub work_units_per_token: u32,
    pub maximum_runtime_seconds: u64,
}

pub(crate) fn execute(executable: &Path, config: CpuProbeConfig) -> ExecutionOutput {
    into_execution_output(execute_inner(executable, config))
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

fn execute_inner(executable: &Path, config: CpuProbeConfig) -> ChildExecution {
    execute_command(Command::new(executable), config)
}

fn execute_command(mut command: Command, config: CpuProbeConfig) -> ChildExecution {
    let expected_count = match expected_record_count(config) {
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
        .arg(config.output_tokens.to_string())
        .arg("--work-units-per-token")
        .arg(config.work_units_per_token.to_string());

    child_supervisor::execute(
        command,
        ChildProtocol {
            runtime_name: "CPU probe",
            expected_records: expected_count,
            max_record_bytes: MAX_SERIALIZED_RECORD_BYTES,
            max_stdout_bytes: MAX_STDOUT_BYTES,
            maximum_runtime_seconds: config.maximum_runtime_seconds,
            spawn_failure_code: ERROR_CPU_PROBE_SPAWN_FAILED,
            exit_failure_code: ERROR_CPU_PROBE_EXIT_FAILED,
            output_failure_code: ERROR_CPU_PROBE_OUTPUT_INVALID,
            deadline_failure_code: ERROR_CPU_PROBE_DEADLINE_EXCEEDED,
            resource_failure_code: ERROR_CPU_PROBE_RESOURCE_ACCOUNTING_FAILED,
            special_exit_codes: &[],
        },
        |record, position| validate_record(record, position, config),
    )
}

fn expected_record_count(config: CpuProbeConfig) -> Result<usize, FailureRecord> {
    let count = config
        .warmup_runs
        .checked_add(config.repetitions)
        .filter(|count| *count <= MAX_CPU_PROBE_RECORDS)
        .ok_or_else(|| protocol_failure("CPU probe requested excessive measurement records"))?;
    let count = usize::try_from(count)
        .map_err(|_| protocol_failure("CPU probe measurement record count is not representable"))?;
    count
        .checked_mul(MAX_SERIALIZED_RECORD_BYTES)
        .filter(|bytes| *bytes <= MAX_STDOUT_BYTES)
        .ok_or_else(|| {
            protocol_failure("CPU probe requested output exceeds its transport bound")
        })?;
    Ok(count)
}

fn validate_record(
    record: &MeasurementRecord,
    position: usize,
    config: CpuProbeConfig,
) -> Result<(), String> {
    let (phase, repetition_index) = if position < config.warmup_runs as usize {
        (MeasurementPhase::Warmup, position as u32 + 1)
    } else {
        (
            MeasurementPhase::Measured,
            position as u32 - config.warmup_runs + 1,
        )
    };
    let valid = record.generator == CPU_PROBE_GENERATOR_VERSION
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
            "CPU probe record {} does not match the requested execution",
            position + 1
        ))
    }
}

fn protocol_failure(message: impl AsRef<str>) -> FailureRecord {
    FailureRecord {
        phase: "running".to_owned(),
        code: ERROR_CPU_PROBE_OUTPUT_INVALID.to_owned(),
        message: message.as_ref().chars().take(1024).collect(),
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

    fn config() -> CpuProbeConfig {
        CpuProbeConfig {
            requests: 2,
            warmup_runs: 1,
            repetitions: 2,
            output_tokens: 4,
            work_units_per_token: 32,
            maximum_runtime_seconds: 1,
        }
    }

    fn record(phase: MeasurementPhase, repetition_index: u32) -> MeasurementRecord {
        MeasurementRecord {
            generator: CPU_PROBE_GENERATOR_VERSION.to_owned(),
            attempt_number: 1,
            phase,
            repetition_index,
            sample_index: 1,
            latency_micros: 20,
            time_to_first_token_micros: 10,
            throughput_milli_requests_per_second: 1000,
            successful_requests: 2,
            failed_requests: 0,
            request_observations: Vec::new(),
        }
    }

    #[test]
    fn validates_expected_order_and_identity() {
        assert!(validate_record(&record(MeasurementPhase::Warmup, 1), 0, config()).is_ok());
        assert!(validate_record(&record(MeasurementPhase::Measured, 1), 1, config()).is_ok());
        assert!(validate_record(&record(MeasurementPhase::Measured, 2), 2, config()).is_ok());
        assert!(validate_record(&record(MeasurementPhase::Measured, 1), 0, config()).is_err());
        let mut duplicate = record(MeasurementPhase::Measured, 1);
        duplicate.generator = "another-generator".to_owned();
        assert!(validate_record(&duplicate, 1, config()).is_err());
    }

    #[test]
    fn maximum_accepted_records_fit_the_output_transport_bound() {
        let maximum_record = MeasurementRecord {
            generator: CPU_PROBE_GENERATOR_VERSION.to_owned(),
            attempt_number: 1,
            phase: MeasurementPhase::Measured,
            repetition_index: MAX_CPU_PROBE_RECORDS,
            sample_index: 1,
            latency_micros: u64::MAX,
            time_to_first_token_micros: u64::MAX,
            throughput_milli_requests_per_second: u64::MAX,
            successful_requests: u32::MAX,
            failed_requests: 0,
            request_observations: Vec::new(),
        };
        let serialized_bytes = serde_json::to_vec(&maximum_record)
            .expect("serialize maximum record")
            .len()
            .checked_add(1)
            .expect("include newline");
        assert!(serialized_bytes <= MAX_SERIALIZED_RECORD_BYTES);
        assert_eq!(MAX_ACCEPTED_RECORD_OUTPUT_BYTES, 512_000);

        let mut maximum = config();
        maximum.warmup_runs = 1;
        maximum.repetitions = MAX_CPU_PROBE_RECORDS - 1;
        assert_eq!(
            expected_record_count(maximum).expect("maximum must fit"),
            MAX_CPU_PROBE_RECORDS as usize
        );
        maximum.repetitions = MAX_CPU_PROBE_RECORDS;
        assert!(expected_record_count(maximum).is_err());
    }

    #[test]
    fn spawn_failure_is_a_runtime_failure() {
        let unspawnable = PathBuf::from("benchplane\0cpu-probe");
        let output = execute(&unspawnable, config());
        assert_eq!(output.terminal_state, RunState::Failed);
        assert_eq!(
            output.failure.expect("failure record").code,
            ERROR_CPU_PROBE_SPAWN_FAILED
        );
    }

    fn test_child() -> &'static Path {
        static CHILD: OnceLock<PathBuf> = OnceLock::new();
        CHILD
            .get_or_init(|| {
                let directory = std::env::temp_dir().join(format!(
                    "benchplane-probe-adapter-test-{}",
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
    println!(r#"{{"generator":"benchplane-cpu-probe/v1","attemptNumber":1,"phase":"{}","repetitionIndex":{},"sampleIndex":1,"latencyMicros":20,"timeToFirstTokenMicros":10,"throughputMilliRequestsPerSecond":1000,"successfulRequests":2,"failedRequests":0}}"#, phase, index);
}
fn main() {
    match env::var("PROBE_TEST_MODE").as_deref() {
        Ok("success") => { burn_cpu(); record("warmup", 1); record("measured", 1); record("measured", 2); }
        Ok("missing") => { record("warmup", 1); record("measured", 1); }
        Ok("duplicate") => { record("warmup", 1); record("warmup", 1); record("measured", 2); }
        Ok("malformed") => println!("not-json"),
        Ok("partial-malformed") => { record("warmup", 1); println!("not-json"); }
        Ok("partial-nonzero") => { record("warmup", 1); std::process::exit(7); }
        Ok("nonzero") => { eprintln!("private injected failure"); std::process::exit(7); }
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
        command.env("PROBE_TEST_MODE", mode);
        execute_command(command, config())
    }

    #[test]
    fn real_child_success_is_parsed_incrementally() {
        let outcome = injected("success");
        assert!(outcome.failure.is_none());
        assert_eq!(outcome.measurements.len(), 3);
        assert_eq!(outcome.measurements[0].phase, MeasurementPhase::Warmup);
        assert_eq!(outcome.measurements[2].repetition_index, 2);
        let resources = outcome.resources.expect("exact child resources");
        assert!(resources.cpu_time_micros > 0);
        assert!(resources.peak_rss_bytes.is_multiple_of(1024));
    }

    #[test]
    fn nonzero_malformed_missing_and_duplicate_outputs_fail() {
        assert_eq!(
            injected("nonzero").failure.expect("nonzero must fail").code,
            ERROR_CPU_PROBE_EXIT_FAILED
        );
        for mode in ["malformed", "missing", "duplicate"] {
            assert_eq!(
                injected(mode)
                    .failure
                    .expect("invalid output must fail")
                    .code,
                ERROR_CPU_PROBE_OUTPUT_INVALID,
                "mode={mode}"
            );
        }
    }

    #[test]
    fn failed_and_timed_out_children_retain_exact_resources() {
        for mode in ["nonzero", "partial-malformed", "deadline"] {
            assert!(injected(mode).resources.is_some(), "mode={mode}");
        }
    }

    #[test]
    fn incomplete_child_protocol_discards_a_valid_record_prefix() {
        for (mode, code) in [
            ("partial-nonzero", ERROR_CPU_PROBE_EXIT_FAILED),
            ("partial-malformed", ERROR_CPU_PROBE_OUTPUT_INVALID),
        ] {
            let output = into_execution_output(injected(mode));
            assert_eq!(output.terminal_state, RunState::Failed, "mode={mode}");
            assert!(output.measurements.is_empty(), "mode={mode}");
            assert_eq!(
                output.failure.expect("failure record").code,
                code,
                "mode={mode}"
            );
        }
    }

    #[test]
    fn deadline_terminates_and_reaps_child() {
        let started = Instant::now();
        let output = into_execution_output(injected("deadline"));
        assert_eq!(output.terminal_state, RunState::Failed);
        assert!(output.measurements.is_empty());
        assert_eq!(
            output.failure.expect("deadline failure").code,
            ERROR_CPU_PROBE_DEADLINE_EXCEEDED
        );
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
