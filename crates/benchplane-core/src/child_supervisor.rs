// SPDX-License-Identifier: Apache-2.0

//! Private supervision for Benchplane's fixed measurement helpers.
//!
//! This is intentionally not a general command runner: callers supply an already fixed
//! executable and a complete, bounded `MeasurementRecord` protocol.

use benchplane_schema::{FailureRecord, MeasurementRecord, ProcessResources};
use std::{
    io::Read,
    mem::MaybeUninit,
    os::unix::process::ExitStatusExt,
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
    pub expected_metadata_records: usize,
    pub expected_records: usize,
    pub max_record_bytes: usize,
    pub max_stdout_bytes: usize,
    pub maximum_runtime_seconds: u64,
    pub spawn_failure_code: &'static str,
    pub exit_failure_code: &'static str,
    pub output_failure_code: &'static str,
    pub deadline_failure_code: &'static str,
    pub resource_failure_code: &'static str,
    pub special_exit_codes: &'static [ExitCodeFailure],
}

#[derive(Debug)]
pub(crate) struct ChildExecution {
    pub metadata: Vec<Vec<u8>>,
    pub measurements: Vec<MeasurementRecord>,
    pub resources: Option<ProcessResources>,
    pub failure: Option<FailureRecord>,
}

impl ChildExecution {
    pub(crate) fn failed(failure: FailureRecord, resources: Option<ProcessResources>) -> Self {
        Self {
            metadata: Vec::new(),
            measurements: Vec::new(),
            resources,
            failure: Some(failure),
        }
    }
}

struct ReapedChild {
    status: ExitStatus,
    resources: Result<ProcessResources, String>,
}

struct CompletedChild {
    reaped: ReapedChild,
    metadata: Vec<Vec<u8>>,
    measurements: Vec<MeasurementRecord>,
}

struct SupervisionFailure {
    failure: FailureRecord,
    resources: Option<Result<ProcessResources, String>>,
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
) -> ChildExecution {
    let accepted_bytes = protocol
        .expected_metadata_records
        .checked_add(protocol.expected_records)
        .and_then(|records| records.checked_mul(protocol.max_record_bytes))
        .filter(|bytes| *bytes <= protocol.max_stdout_bytes)
        .ok_or_else(|| output_failure(protocol, "requested output exceeds its transport bound"));
    let accepted_bytes = match accepted_bytes {
        Ok(bytes) => bytes,
        Err(failure) => return ChildExecution::failed(failure, None),
    };
    debug_assert!(accepted_bytes <= protocol.max_stdout_bytes);

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ChildExecution::failed(
                failure(
                    protocol.spawn_failure_code,
                    format!(
                        "could not start packaged {} helper: {error}",
                        protocol.runtime_name
                    ),
                ),
                None,
            );
        }
    };
    let stdout = child.stdout.take().expect("piped child stdout");
    let stderr = child.stderr.take().expect("piped child stderr");
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let stdout_protocol = protocol;
    let stdout_thread = thread::spawn(move || read_stdout(stdout, stdout_tx, stdout_protocol));
    let stderr_thread = thread::spawn(move || read_stderr(stderr));
    let deadline = Instant::now() + Duration::from_secs(protocol.maximum_runtime_seconds);

    let result = supervise(&child, &stdout_rx, protocol, deadline, validate);
    let result = match result {
        Ok(completed) => Ok(completed),
        Err(failure) => Err((
            failure.failure,
            failure
                .resources
                .unwrap_or_else(|| terminate_and_reap(&mut child)),
        )),
    };
    let _ = stdout_thread.join();
    let stderr = stderr_thread.join().unwrap_or_default();

    match result {
        Ok(completed) if completed.reaped.status.success() => completed_success(
            protocol,
            completed.reaped.resources,
            completed.metadata,
            completed.measurements,
        ),
        Ok(completed) => {
            let special = completed.reaped.status.code().and_then(|code| {
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
                            "packaged {} helper exited with {}",
                            protocol.runtime_name, completed.reaped.status
                        ),
                    )
                });
            failed_after_reap(
                failure(code, format!("{message}{}", format_stderr(&stderr))),
                completed.reaped.resources,
            )
        }
        Err((mut failure, resources)) => {
            if !stderr.is_empty() && failure.code != protocol.deadline_failure_code {
                failure.message.push_str(&format_stderr(&stderr));
                failure.message = bounded_message(&failure.message);
            }
            failed_after_reap(failure, resources)
        }
    }
}

fn completed_success(
    protocol: ChildProtocol,
    resources: Result<ProcessResources, String>,
    metadata: Vec<Vec<u8>>,
    measurements: Vec<MeasurementRecord>,
) -> ChildExecution {
    if metadata.len() != protocol.expected_metadata_records
        || measurements.len() != protocol.expected_records
    {
        return failed_after_reap(
            output_failure(
                protocol,
                format!(
                    "emitted {} metadata and {} measurement records; expected {} and {}",
                    metadata.len(),
                    measurements.len(),
                    protocol.expected_metadata_records,
                    protocol.expected_records,
                ),
            ),
            resources,
        );
    }
    match resources {
        Ok(resources) => ChildExecution {
            metadata,
            measurements,
            resources: Some(resources),
            failure: None,
        },
        Err(error) => ChildExecution::failed(resource_failure(protocol, error), None),
    }
}

fn supervise(
    child: &Child,
    stdout: &Receiver<StdoutEvent>,
    protocol: ChildProtocol,
    deadline: Instant,
    validate: impl Fn(&MeasurementRecord, usize) -> Result<(), String>,
) -> Result<CompletedChild, SupervisionFailure> {
    let mut metadata = Vec::with_capacity(protocol.expected_metadata_records);
    let mut measurements = Vec::with_capacity(protocol.expected_records);
    let mut stdout_done = false;
    let mut exit_status = None;

    while !stdout_done || exit_status.is_none() {
        let now = Instant::now();
        if now >= deadline {
            return Err(supervision_failure(
                failure(
                    protocol.deadline_failure_code,
                    format!(
                        "packaged {} helper exceeded the {} second experiment deadline",
                        protocol.runtime_name, protocol.maximum_runtime_seconds
                    ),
                ),
                &mut exit_status,
            ));
        }
        let wait = POLL_INTERVAL.min(deadline.saturating_duration_since(now));
        match stdout.recv_timeout(wait) {
            Ok(StdoutEvent::Line(line)) => {
                let emitted = metadata.len().saturating_add(measurements.len());
                let expected = protocol
                    .expected_metadata_records
                    .saturating_add(protocol.expected_records);
                if emitted >= expected {
                    return Err(supervision_failure(
                        output_failure(protocol, "emitted excessive protocol records"),
                        &mut exit_status,
                    ));
                }
                if line
                    .len()
                    .checked_add(1)
                    .is_none_or(|bytes| bytes > protocol.max_record_bytes)
                {
                    return Err(supervision_failure(
                        output_failure(
                            protocol,
                            format!(
                                "measurement record exceeded the {} byte protocol envelope",
                                protocol.max_record_bytes
                            ),
                        ),
                        &mut exit_status,
                    ));
                }
                if metadata.len() < protocol.expected_metadata_records {
                    metadata.push(line);
                    continue;
                }
                let record: MeasurementRecord = match serde_json::from_slice(&line) {
                    Ok(record) => record,
                    Err(error) => {
                        return Err(supervision_failure(
                            output_failure(protocol, format!("emitted malformed JSON: {error}")),
                            &mut exit_status,
                        ));
                    }
                };
                if let Err(message) = validate(&record, measurements.len()) {
                    return Err(supervision_failure(
                        output_failure(protocol, message),
                        &mut exit_status,
                    ));
                }
                measurements.push(record);
            }
            Ok(StdoutEvent::Error(message)) => {
                return Err(supervision_failure(
                    output_failure(protocol, message),
                    &mut exit_status,
                ));
            }
            Ok(StdoutEvent::Done) | Err(RecvTimeoutError::Disconnected) => stdout_done = true,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if exit_status.is_none() {
            exit_status = match wait4(child, libc::WNOHANG) {
                Ok(status) => status,
                Err(error) => {
                    return Err(supervision_failure(
                        output_failure(protocol, format!("could not wait for helper: {error}")),
                        &mut exit_status,
                    ));
                }
            };
        }
    }

    Ok(CompletedChild {
        reaped: exit_status.expect("child exit observed"),
        metadata,
        measurements,
    })
}

fn supervision_failure(
    failure: FailureRecord,
    reaped: &mut Option<ReapedChild>,
) -> SupervisionFailure {
    SupervisionFailure {
        failure,
        resources: reaped.take().map(|reaped| reaped.resources),
    }
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

fn terminate_and_reap(child: &mut Child) -> Result<ProcessResources, String> {
    match wait4(child, libc::WNOHANG) {
        Ok(Some(reaped)) => return reaped.resources,
        Ok(None) => {}
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    }
    let _ = child.kill();
    match wait4(child, 0) {
        Ok(Some(reaped)) => reaped.resources,
        Ok(None) => Err("blocking wait returned without reaping the helper".to_owned()),
        Err(error) => {
            let _ = child.wait();
            Err(error)
        }
    }
}

fn wait4(child: &Child, options: libc::c_int) -> Result<Option<ReapedChild>, String> {
    let pid = libc::pid_t::try_from(child.id())
        .map_err(|_| "helper process ID is not representable".to_owned())?;
    loop {
        let mut status = 0;
        let mut usage = MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: `pid` identifies the exact child owned by `child`; both output
        // pointers are valid for writes, and successful return initializes rusage.
        let waited = unsafe { libc::wait4(pid, &mut status, options, usage.as_mut_ptr()) };
        if waited == pid {
            // SAFETY: wait4 returned this child PID and therefore initialized rusage.
            let usage = unsafe { usage.assume_init() };
            return Ok(Some(ReapedChild {
                status: ExitStatus::from_raw(status),
                resources: convert_resource_usage(&usage),
            }));
        }
        if waited == 0 {
            return Ok(None);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(error.to_string());
    }
}

fn convert_resource_usage(usage: &libc::rusage) -> Result<ProcessResources, String> {
    resource_usage_from_components(
        usage.ru_utime.tv_sec,
        usage.ru_utime.tv_usec,
        usage.ru_stime.tv_sec,
        usage.ru_stime.tv_usec,
        usage.ru_maxrss,
    )
}

fn resource_usage_from_components(
    user_seconds: i64,
    user_micros: i64,
    system_seconds: i64,
    system_micros: i64,
    peak_rss_kib: i64,
) -> Result<ProcessResources, String> {
    let time_micros = |seconds: i64, micros: i64| {
        if seconds < 0 || !(0..1_000_000).contains(&micros) {
            return None;
        }
        u64::try_from(seconds)
            .ok()?
            .checked_mul(1_000_000)?
            .checked_add(u64::try_from(micros).ok()?)
    };
    let user = time_micros(user_seconds, user_micros)
        .ok_or_else(|| "helper user CPU time is not representable".to_owned())?;
    let system = time_micros(system_seconds, system_micros)
        .ok_or_else(|| "helper system CPU time is not representable".to_owned())?;
    let cpu_time_micros = user
        .checked_add(system)
        .ok_or_else(|| "helper total CPU time overflowed".to_owned())?;
    let peak_rss_bytes = u64::try_from(peak_rss_kib)
        .ok()
        .and_then(|value| value.checked_mul(1024))
        .ok_or_else(|| "helper peak RSS is not representable in bytes".to_owned())?;
    Ok(ProcessResources {
        cpu_time_micros,
        peak_rss_bytes,
    })
}

fn output_failure(protocol: ChildProtocol, message: impl AsRef<str>) -> FailureRecord {
    failure(
        protocol.output_failure_code,
        format!("{} protocol: {}", protocol.runtime_name, message.as_ref()),
    )
}

fn resource_failure(protocol: ChildProtocol, message: impl AsRef<str>) -> FailureRecord {
    failure(
        protocol.resource_failure_code,
        format!(
            "could not account for packaged {} helper: {}",
            protocol.runtime_name,
            message.as_ref()
        ),
    )
}

fn failed_after_reap(
    primary: FailureRecord,
    resources: Result<ProcessResources, String>,
) -> ChildExecution {
    ChildExecution::failed(primary, resources.ok())
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

    const TEST_PROTOCOL: ChildProtocol = ChildProtocol {
        runtime_name: "test",
        expected_metadata_records: 0,
        expected_records: 0,
        max_record_bytes: 1,
        max_stdout_bytes: 1,
        maximum_runtime_seconds: 1,
        spawn_failure_code: "test.spawnFailed",
        exit_failure_code: "test.exitFailed",
        output_failure_code: "test.outputInvalid",
        deadline_failure_code: "test.deadlineExceeded",
        resource_failure_code: "test.resourceAccountingFailed",
        special_exit_codes: &[],
    };

    #[test]
    fn converts_linux_wait4_units_with_checked_arithmetic() {
        assert_eq!(
            resource_usage_from_components(1, 250_000, 2, 750_000, 4096)
                .expect("valid resource usage"),
            ProcessResources {
                cpu_time_micros: 4_000_000,
                peak_rss_bytes: 4 * 1024 * 1024,
            }
        );
        for invalid in [
            resource_usage_from_components(-1, 0, 0, 0, 1),
            resource_usage_from_components(0, 1_000_000, 0, 0, 1),
            resource_usage_from_components(i64::MAX, 0, 0, 0, 1),
            resource_usage_from_components(0, 0, 0, 0, -1),
            resource_usage_from_components(0, 0, 0, 0, i64::MAX),
        ] {
            assert!(invalid.is_err());
        }
    }

    #[test]
    fn accounting_failure_does_not_mask_an_established_primary_failure() {
        let primary = failure("helper.primaryFailure", "primary failure");
        let outcome = failed_after_reap(
            primary.clone(),
            Err("injected accounting failure".to_owned()),
        );
        assert_eq!(outcome.failure, Some(primary));
        assert!(outcome.resources.is_none());
        assert!(outcome.measurements.is_empty());
    }

    #[test]
    fn otherwise_successful_child_requires_real_accounting_without_fabricated_zeroes() {
        let outcome = completed_success(
            TEST_PROTOCOL,
            Err("injected accounting failure".to_owned()),
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            outcome.failure.expect("accounting failure").code,
            TEST_PROTOCOL.resource_failure_code
        );
        assert!(outcome.resources.is_none());
        assert!(outcome.measurements.is_empty());
    }

    #[test]
    fn protocol_failure_retains_resources_if_the_child_was_already_reaped() {
        let expected = ProcessResources {
            cpu_time_micros: 17,
            peak_rss_bytes: 2048,
        };
        let mut reaped = Some(ReapedChild {
            status: ExitStatus::from_raw(0),
            resources: Ok(expected),
        });
        let observed = supervision_failure(
            failure("helper.outputInvalid", "trailing malformed output"),
            &mut reaped,
        );
        assert_eq!(
            observed.resources.expect("retained resources"),
            Ok(expected)
        );
        assert!(reaped.is_none());
    }

    #[test]
    fn termination_reaps_the_exact_child_and_returns_resources() {
        let mut child = Command::new("sleep")
            .arg("5")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn test child");
        let pid = libc::pid_t::try_from(child.id()).expect("representable PID");
        let resources = terminate_and_reap(&mut child).expect("reap with accounting");
        assert!(resources.peak_rss_bytes.is_multiple_of(1024));

        let mut status = 0;
        // SAFETY: this probes only the PID already reaped above, with a valid status pointer.
        let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert_eq!(waited, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}
