// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ERROR_LOCAL_FAKE_RUNTIME_FAILURE: &str = "localFake.runtimeFailure";
pub const ERROR_LOCAL_FAKE_INTERRUPTED: &str = "localFake.interrupted";
pub const ERROR_EVIDENCE_FINALIZATION_FAILED: &str = "evidence.finalizationFailed";
pub const ERROR_LIFECYCLE_INVALID_TRANSITION: &str = "lifecycle.invalidTransition";
pub const ERROR_IO_OPERATION_FAILED: &str = "io.operationFailed";
pub const ERROR_EXECUTION_UNSUPPORTED_COMBINATION: &str = "execution.unsupportedCombination";
pub const ERROR_CPU_PROBE_SPAWN_FAILED: &str = "cpuProbe.spawnFailed";
pub const ERROR_CPU_PROBE_EXIT_FAILED: &str = "cpuProbe.exitFailed";
pub const ERROR_CPU_PROBE_OUTPUT_INVALID: &str = "cpuProbe.outputInvalid";
pub const ERROR_CPU_PROBE_DEADLINE_EXCEEDED: &str = "cpuProbe.deadlineExceeded";

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    Created,
    Preparing,
    Running,
    Collecting,
    Finalizing,
    Succeeded,
    Failed,
    Interrupted,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Collecting => "collecting",
            Self::Finalizing => "finalizing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Interrupted)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AttemptStatus {
    Created,
    Preparing,
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ValidityStatus {
    Valid,
    Invalid,
    Indeterminate,
}

impl ValidityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Indeterminate => "indeterminate",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureRecord {
    pub phase: String,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub attempt_number: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRecord {
    pub run_id: String,
    pub run_status: RunState,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub experiment_digest: String,
    pub resolved_plan_digest: String,
    pub attempt_count: u32,
    pub failure: Option<FailureRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptRecord {
    pub run_id: String,
    pub attempt_number: u32,
    pub status: AttemptStatus,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub failure: Option<FailureRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleEvent {
    pub run_id: String,
    pub sequence: u64,
    pub recorded_at: String,
    pub from_state: Option<RunState>,
    pub to_state: RunState,
    pub attempt_number: u32,
    pub failure: Option<FailureRecord>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeasurementPhase {
    Warmup,
    Measured,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasurementRecord {
    pub generator: String,
    pub attempt_number: u32,
    pub phase: MeasurementPhase,
    pub repetition_index: u32,
    pub sample_index: u32,
    pub latency_micros: u64,
    pub time_to_first_token_micros: u64,
    pub throughput_milli_requests_per_second: u64,
    pub successful_requests: u32,
    pub failed_requests: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidityReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidityResult {
    pub run_id: String,
    pub status: ValidityStatus,
    pub required_samples: u32,
    pub observed_samples: u32,
    pub reasons: Vec<ValidityReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LatencySummary {
    pub mean_micros: u64,
    pub p50_micros: u64,
    pub p95_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSummary {
    pub run_id: String,
    pub run_status: RunState,
    pub attempt_count: u32,
    pub sample_count: u32,
    pub validity_status: ValidityStatus,
    pub latency: Option<LatencySummary>,
    pub mean_throughput_milli_requests_per_second: Option<u64>,
    pub experiment_digest: String,
    pub resolved_plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceManifest {
    pub format: String,
    pub run_id: String,
    pub run_status: RunState,
    pub validity_status: ValidityStatus,
    pub experiment_digest: String,
    pub resolved_plan_digest: String,
    pub attempt_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunResult {
    pub run_id: String,
    pub run_state: RunState,
    pub validity_status: ValidityStatus,
    pub attempt_count: u32,
    pub sample_count: u32,
    pub latency: Option<LatencySummary>,
    pub mean_throughput_milli_requests_per_second: Option<u64>,
    pub bundle_path: String,
    pub experiment_digest: String,
    pub resolved_plan_digest: String,
    pub evidence_digest: String,
    pub failure: Option<FailureRecord>,
}
