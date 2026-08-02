// SPDX-License-Identifier: Apache-2.0

use crate::execution::ExecutionOutput;
use benchplane_schema::{
    FailureRecord, MeasurementPhase, MeasurementRecord, RunState, CPU_PROBE_GENERATOR_VERSION,
    ERROR_CPU_PROBE_DEADLINE_EXCEEDED, ERROR_CPU_PROBE_EXIT_FAILED, ERROR_CPU_PROBE_OUTPUT_INVALID,
    ERROR_CPU_PROBE_SPAWN_FAILED,
};
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_STDOUT_BYTES: usize = 1024 * 1024;
const MAX_STDERR_BYTES: usize = 16 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_DIAGNOSTIC_CHARS: usize = 1024;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CpuProbeConfig {
    pub requests: u32,
    pub warmup_runs: u32,
    pub repetitions: u32,
    pub output_tokens: u32,
    pub work_units_per_token: u32,
    pub maximum_runtime_seconds: u64,
}

enum StdoutEvent {
    Line(Vec<u8>),
    Error(String),
    Done,
}

pub(crate) fn execute(executable: &Path, config: CpuProbeConfig) -> ExecutionOutput {
    match execute_inner(executable, config) {
        Ok(measurements) => ExecutionOutput {
            measurements,
            terminal_state: RunState::Succeeded,
            failure: None,
        },
        Err(failure) => ExecutionOutput {
            measurements: Vec::new(),
            terminal_state: RunState::Failed,
            failure: Some(failure),
        },
    }
}

fn execute_inner(
    executable: &Path,
    config: CpuProbeConfig,
) -> Result<Vec<MeasurementRecord>, FailureRecord> {
    execute_command(Command::new(executable), config)
}

fn execute_command(
    mut command: Command,
    config: CpuProbeConfig,
) -> Result<Vec<MeasurementRecord>, FailureRecord> {
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
        .arg(config.work_units_per_token.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| {
        failure(
            ERROR_CPU_PROBE_SPAWN_FAILED,
            format!("could not start the packaged CPU probe: {error}"),
        )
    })?;
    let stdout = child.stdout.take().expect("piped child stdout");
    let stderr = child.stderr.take().expect("piped child stderr");
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let stdout_thread = thread::spawn(move || read_stdout(stdout, stdout_tx));
    let stderr_thread = thread::spawn(move || read_stderr(stderr));
    let deadline = Instant::now() + Duration::from_secs(config.maximum_runtime_seconds);

    let result = supervise(&mut child, &stdout_rx, config, deadline);
    if result.is_err() {
        terminate_and_reap(&mut child);
    }
    let _ = stdout_thread.join();
    let stderr = stderr_thread.join().unwrap_or_default();

    match result {
        Ok((status, measurements)) if status.success() => {
            let expected = (config.warmup_runs + config.repetitions) as usize;
            if measurements.len() == expected {
                Ok(measurements)
            } else {
                Err(output_failure(format!(
                    "CPU probe emitted {} records; expected {expected}",
                    measurements.len()
                )))
            }
        }
        Ok((status, _)) => Err(failure(
            ERROR_CPU_PROBE_EXIT_FAILED,
            format!(
                "packaged CPU probe exited with {status}{}",
                format_stderr(&stderr)
            ),
        )),
        Err(mut failure) => {
            if !stderr.is_empty() && failure.code != ERROR_CPU_PROBE_DEADLINE_EXCEEDED {
                failure.message.push_str(&format_stderr(&stderr));
                failure.message = bounded_message(&failure.message);
            }
            Err(failure)
        }
    }
}

fn supervise(
    child: &mut Child,
    stdout: &Receiver<StdoutEvent>,
    config: CpuProbeConfig,
    deadline: Instant,
) -> Result<(std::process::ExitStatus, Vec<MeasurementRecord>), FailureRecord> {
    let expected_count = (config.warmup_runs + config.repetitions) as usize;
    let mut measurements = Vec::with_capacity(expected_count);
    let mut stdout_done = false;
    let mut exit_status = None;

    while !stdout_done || exit_status.is_none() {
        let now = Instant::now();
        if now >= deadline {
            terminate_and_reap(child);
            return Err(failure(
                ERROR_CPU_PROBE_DEADLINE_EXCEEDED,
                format!(
                    "packaged CPU probe exceeded the {} second experiment deadline",
                    config.maximum_runtime_seconds
                ),
            ));
        }
        let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(now));
        match stdout.recv_timeout(wait) {
            Ok(StdoutEvent::Line(line)) => {
                if measurements.len() >= expected_count {
                    return Err(output_failure(
                        "CPU probe emitted excessive measurement records",
                    ));
                }
                let record: MeasurementRecord = serde_json::from_slice(&line).map_err(|error| {
                    output_failure(format!("CPU probe emitted malformed JSON: {error}"))
                })?;
                validate_record(&record, measurements.len(), config)?;
                measurements.push(record);
            }
            Ok(StdoutEvent::Error(message)) => return Err(output_failure(message)),
            Ok(StdoutEvent::Done) | Err(RecvTimeoutError::Disconnected) => stdout_done = true,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if exit_status.is_none() {
            exit_status = child.try_wait().map_err(|error| {
                output_failure(format!("could not wait for packaged CPU probe: {error}"))
            })?;
        }
    }

    Ok((exit_status.expect("child exit observed"), measurements))
}

fn validate_record(
    record: &MeasurementRecord,
    position: usize,
    config: CpuProbeConfig,
) -> Result<(), FailureRecord> {
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
        Err(output_failure(format!(
            "CPU probe record {} does not match the requested execution",
            position + 1
        )))
    }
}

fn read_stdout(mut reader: impl Read, sender: Sender<StdoutEvent>) {
    let mut buffer = [0_u8; 8192];
    let mut line = Vec::new();
    let mut total = 0_usize;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                let _ = sender.send(StdoutEvent::Error(format!(
                    "could not read CPU probe output: {error}"
                )));
                return;
            }
        };
        total = total.saturating_add(count);
        if total > MAX_STDOUT_BYTES {
            let _ = sender.send(StdoutEvent::Error(
                "CPU probe output exceeded the 1048576 byte limit".to_owned(),
            ));
            return;
        }
        for byte in &buffer[..count] {
            if *byte == b'\n' {
                if sender
                    .send(StdoutEvent::Line(std::mem::take(&mut line)))
                    .is_err()
                {
                    return;
                }
            } else {
                line.push(*byte);
                if line.len() > MAX_LINE_BYTES {
                    let _ = sender.send(StdoutEvent::Error(
                        "CPU probe output line exceeded the 16384 byte limit".to_owned(),
                    ));
                    return;
                }
            }
        }
    }
    if !line.is_empty() {
        let _ = sender.send(StdoutEvent::Line(line));
    }
    let _ = sender.send(StdoutEvent::Done);
}

fn read_stderr(mut reader: impl Read) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 4096];
    while let Ok(count) = reader.read(&mut buffer) {
        if count == 0 {
            break;
        }
        let remaining = MAX_STDERR_BYTES.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..count.min(remaining)]);
    }
    retained
}

fn terminate_and_reap(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn output_failure(message: impl Into<String>) -> FailureRecord {
    failure(ERROR_CPU_PROBE_OUTPUT_INVALID, message.into())
}

fn failure(code: &str, message: impl AsRef<str>) -> FailureRecord {
    FailureRecord {
        phase: "running".to_owned(),
        code: code.to_owned(),
        message: bounded_message(message.as_ref()),
        retryable: false,
        attempt_number: 1,
    }
}

fn bounded_message(message: &str) -> String {
    message.chars().take(MAX_DIAGNOSTIC_CHARS).collect()
}

fn format_stderr(stderr: &[u8]) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!("; stderr: {}", String::from_utf8_lossy(stderr).trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, sync::OnceLock};

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
    fn bounded_reader_rejects_oversized_line_and_total_output() {
        let (sender, receiver) = mpsc::channel();
        read_stdout(&vec![b'x'; MAX_LINE_BYTES + 1][..], sender);
        assert!(matches!(receiver.recv(), Ok(StdoutEvent::Error(_))));

        let mut many_lines = Vec::with_capacity(MAX_STDOUT_BYTES + 1);
        let chunk = vec![b'x'; MAX_LINE_BYTES];
        while many_lines.len() <= MAX_STDOUT_BYTES {
            many_lines.extend_from_slice(&chunk);
            many_lines.push(b'\n');
        }
        let (sender, receiver) = mpsc::channel();
        read_stdout(&many_lines[..], sender);
        assert!(receiver
            .into_iter()
            .any(|event| matches!(event, StdoutEvent::Error(_))));
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
fn record(phase: &str, index: u32) {
    println!(r#"{{"generator":"benchplane-cpu-probe/v1","attemptNumber":1,"phase":"{}","repetitionIndex":{},"sampleIndex":1,"latencyMicros":20,"timeToFirstTokenMicros":10,"throughputMilliRequestsPerSecond":1000,"successfulRequests":2,"failedRequests":0}}"#, phase, index);
}
fn main() {
    match env::var("PROBE_TEST_MODE").as_deref() {
        Ok("success") => { record("warmup", 1); record("measured", 1); record("measured", 2); }
        Ok("missing") => { record("warmup", 1); record("measured", 1); }
        Ok("duplicate") => { record("warmup", 1); record("warmup", 1); record("measured", 2); }
        Ok("malformed") => println!("not-json"),
        Ok("nonzero") => { eprintln!("private injected failure"); std::process::exit(7); }
        Ok("deadline") => thread::sleep(Duration::from_secs(5)),
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

    fn injected(mode: &str) -> Result<Vec<MeasurementRecord>, FailureRecord> {
        let mut command = Command::new(test_child());
        command.env("PROBE_TEST_MODE", mode);
        execute_command(command, config())
    }

    #[test]
    fn real_child_success_is_parsed_incrementally() {
        let records = injected("success").expect("test child succeeds");
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].phase, MeasurementPhase::Warmup);
        assert_eq!(records[2].repetition_index, 2);
    }

    #[test]
    fn nonzero_malformed_missing_and_duplicate_outputs_fail() {
        assert_eq!(
            injected("nonzero").expect_err("nonzero must fail").code,
            ERROR_CPU_PROBE_EXIT_FAILED
        );
        for mode in ["malformed", "missing", "duplicate"] {
            assert_eq!(
                injected(mode).expect_err("invalid output must fail").code,
                ERROR_CPU_PROBE_OUTPUT_INVALID,
                "mode={mode}"
            );
        }
    }

    #[test]
    fn deadline_terminates_and_reaps_child() {
        let started = Instant::now();
        let error = injected("deadline").expect_err("deadline must fail");
        assert_eq!(error.code, ERROR_CPU_PROBE_DEADLINE_EXCEEDED);
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
