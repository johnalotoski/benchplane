// SPDX-License-Identifier: Apache-2.0

//! Private supervision for Benchplane's fixed measurement helpers.
//!
//! This is intentionally not a general command runner: callers supply an already fixed
//! executable and a complete, bounded `MeasurementRecord` protocol.

use benchplane_schema::{FailureRecord, MeasurementRecord};
use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

const MAX_STDERR_BYTES: usize = 16 * 1024;
const MAX_DIAGNOSTIC_CHARS: usize = 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExitCodeFailure {
    pub exit_code: i32,
    pub failure_code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChildProtocol {
    pub runtime_name: &'static str,
    pub expected_records: usize,
    pub max_record_bytes: usize,
    pub max_stdout_bytes: usize,
    pub maximum_runtime_seconds: u64,
    pub spawn_failure_code: &'static str,
    pub exit_failure_code: &'static str,
    pub output_failure_code: &'static str,
    pub deadline_failure_code: &'static str,
    pub special_exit_codes: &'static [ExitCodeFailure],
}

enum StdoutEvent {
    Line(Vec<u8>),
    Error(String),
    Done,
}

pub(crate) fn execute(
    mut command: Command,
    protocol: ChildProtocol,
    validate: impl Fn(&MeasurementRecord, usize) -> Result<(), String>,
) -> Result<Vec<MeasurementRecord>, FailureRecord> {
    let accepted_bytes = protocol
        .expected_records
        .checked_mul(protocol.max_record_bytes)
        .filter(|bytes| *bytes <= protocol.max_stdout_bytes)
        .ok_or_else(|| output_failure(protocol, "requested output exceeds its transport bound"))?;
    debug_assert!(accepted_bytes <= protocol.max_stdout_bytes);

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        failure(
            protocol.spawn_failure_code,
            format!(
                "could not start packaged {} helper: {error}",
                protocol.runtime_name
            ),
        )
    })?;
    let stdout = child.stdout.take().expect("piped child stdout");
    let stderr = child.stderr.take().expect("piped child stderr");
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let stdout_protocol = protocol;
    let stdout_thread = thread::spawn(move || read_stdout(stdout, stdout_tx, stdout_protocol));
    let stderr_thread = thread::spawn(move || read_stderr(stderr));
    let deadline = Instant::now() + Duration::from_secs(protocol.maximum_runtime_seconds);

    let result = supervise(&mut child, &stdout_rx, protocol, deadline, validate);
    if result.is_err() {
        terminate_and_reap(&mut child);
    }
    let _ = stdout_thread.join();
    let stderr = stderr_thread.join().unwrap_or_default();

    match result {
        Ok((status, measurements)) if status.success() => {
            if measurements.len() == protocol.expected_records {
                Ok(measurements)
            } else {
                Err(output_failure(
                    protocol,
                    format!(
                        "emitted {} records; expected {}",
                        measurements.len(),
                        protocol.expected_records
                    ),
                ))
            }
        }
        Ok((status, _)) => {
            let special = status.code().and_then(|code| {
                protocol
                    .special_exit_codes
                    .iter()
                    .find(|failure| failure.exit_code == code)
            });
            let (code, message) = special
                .map(|special| (special.failure_code, special.message.to_owned()))
                .unwrap_or_else(|| {
                    (
                        protocol.exit_failure_code,
                        format!(
                            "packaged {} helper exited with {status}",
                            protocol.runtime_name
                        ),
                    )
                });
            Err(failure(
                code,
                format!("{message}{}", format_stderr(&stderr)),
            ))
        }
        Err(mut failure) => {
            if !stderr.is_empty() && failure.code != protocol.deadline_failure_code {
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
    protocol: ChildProtocol,
    deadline: Instant,
    validate: impl Fn(&MeasurementRecord, usize) -> Result<(), String>,
) -> Result<(ExitStatus, Vec<MeasurementRecord>), FailureRecord> {
    let mut measurements = Vec::with_capacity(protocol.expected_records);
    let mut stdout_done = false;
    let mut exit_status = None;

    while !stdout_done || exit_status.is_none() {
        let now = Instant::now();
        if now >= deadline {
            terminate_and_reap(child);
            return Err(failure(
                protocol.deadline_failure_code,
                format!(
                    "packaged {} helper exceeded the {} second experiment deadline",
                    protocol.runtime_name, protocol.maximum_runtime_seconds
                ),
            ));
        }
        let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(now));
        match stdout.recv_timeout(wait) {
            Ok(StdoutEvent::Line(line)) => {
                if measurements.len() >= protocol.expected_records {
                    return Err(output_failure(
                        protocol,
                        "emitted excessive measurement records",
                    ));
                }
                if line
                    .len()
                    .checked_add(1)
                    .is_none_or(|bytes| bytes > protocol.max_record_bytes)
                {
                    return Err(output_failure(
                        protocol,
                        format!(
                            "measurement record exceeded the {} byte protocol envelope",
                            protocol.max_record_bytes
                        ),
                    ));
                }
                let record: MeasurementRecord = serde_json::from_slice(&line).map_err(|error| {
                    output_failure(protocol, format!("emitted malformed JSON: {error}"))
                })?;
                validate(&record, measurements.len())
                    .map_err(|message| output_failure(protocol, message))?;
                measurements.push(record);
            }
            Ok(StdoutEvent::Error(message)) => return Err(output_failure(protocol, message)),
            Ok(StdoutEvent::Done) | Err(RecvTimeoutError::Disconnected) => stdout_done = true,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if exit_status.is_none() {
            exit_status = child.try_wait().map_err(|error| {
                output_failure(protocol, format!("could not wait for helper: {error}"))
            })?;
        }
    }

    Ok((exit_status.expect("child exit observed"), measurements))
}

fn read_stdout(mut reader: impl Read, sender: Sender<StdoutEvent>, protocol: ChildProtocol) {
    let mut buffer = [0_u8; 8192];
    let mut line = Vec::new();
    let mut total = 0_usize;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                let _ = sender.send(StdoutEvent::Error(format!(
                    "could not read output: {error}"
                )));
                return;
            }
        };
        total = total.saturating_add(count);
        if total > protocol.max_stdout_bytes {
            let _ = sender.send(StdoutEvent::Error(format!(
                "output exceeded the {} byte limit",
                protocol.max_stdout_bytes
            )));
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
                if line.len() >= protocol.max_record_bytes {
                    let _ = sender.send(StdoutEvent::Error(format!(
                        "output line exceeded the {} byte limit",
                        protocol.max_record_bytes
                    )));
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

fn output_failure(protocol: ChildProtocol, message: impl AsRef<str>) -> FailureRecord {
    failure(
        protocol.output_failure_code,
        format!("{} protocol: {}", protocol.runtime_name, message.as_ref()),
    )
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
