// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ATTEMPT_PROVENANCE_FORMAT_V1: &str = "benchplane-attempt-provenance/v1";
pub const ATTEMPT_RESOURCES_FORMAT_V1: &str = "benchplane-attempt-resources/v1";
pub const EVIDENCE_COMPARISON_FORMAT_V1: &str = "benchplane-evidence-comparison/v1";
pub const BENCHPLANE_SOFTWARE_NAME: &str = "benchplane";
pub const LLAMA_CPP_ENGINE_NAME: &str = "llama.cpp";
pub const LLAMA_CPP_ENGINE_VERSION: &str = "b10133";
pub const LLAMA_CPP_MODEL_SHA256: &str =
    "sha256:55aa88ddac43adce6af0e9be8d6cdff2337a3835cd9b50bbcd7a894eb66dfc75";
pub const LLAMA_CPP_BACKEND_IDENTITY: &str = "nixpkgs-llama-cpp-cpu-only-dynamic/v1";
pub const LLAMA_CPP_CUDA_BACKEND_IDENTITY: &str = "nixpkgs-llama-cpp-nvidia-cuda-dynamic/v1";

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
pub const ERROR_CPU_PROBE_RESOURCE_ACCOUNTING_FAILED: &str = "cpuProbe.resourceAccountingFailed";
pub const ERROR_LLAMA_CPP_SPAWN_FAILED: &str = "llamaCpp.spawnFailed";
pub const ERROR_LLAMA_CPP_MODEL_INIT_FAILED: &str = "llamaCpp.modelInitFailed";
pub const ERROR_LLAMA_CPP_EXIT_FAILED: &str = "llamaCpp.exitFailed";
pub const ERROR_LLAMA_CPP_OUTPUT_INVALID: &str = "llamaCpp.outputInvalid";
pub const ERROR_LLAMA_CPP_DEADLINE_EXCEEDED: &str = "llamaCpp.deadlineExceeded";
pub const ERROR_LLAMA_CPP_RESOURCE_ACCOUNTING_FAILED: &str = "llamaCpp.resourceAccountingFailed";

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
pub struct AttemptProvenance {
    pub format: String,
    pub run_id: String,
    pub attempt_number: u32,
    pub platform: PlatformProvenance,
    pub software: SoftwareProvenance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceScope {
    HelperProcessLifetime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessResources {
    pub cpu_time_micros: u64,
    pub peak_rss_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptResources {
    pub format: String,
    pub run_id: String,
    pub attempt_number: u32,
    pub scope: ResourceScope,
    pub cpu_time_micros: u64,
    pub peak_rss_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformProvenance {
    pub operating_system: OperatingSystemProvenance,
    pub kernel: KernelProvenance,
    pub architecture: String,
    pub cpu: CpuProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatingSystemProvenance {
    pub family: String,
    pub distribution: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KernelProvenance {
    pub name: String,
    pub release: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CpuProvenance {
    pub model: Option<String>,
    pub logical_cpu_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SoftwareProvenance {
    pub benchplane: SoftwareComponentProvenance,
    pub runtime: RuntimeProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SoftwareComponentProvenance {
    pub name: String,
    pub version: String,
    pub nix_store_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RuntimeProvenance {
    LocalFake {
        generator: String,
    },
    CpuProbe {
        generator: String,
    },
    LlamaCpp {
        generator: String,
        engine: SoftwareComponentProvenance,
        model: ModelProvenance,
        backend: Box<BackendProvenance>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProvenance {
    pub identity: String,
    pub sha256: String,
    pub nix_store_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackendProvenance {
    pub identity: String,
    pub device_class: DeviceClass,
    pub nix_store_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nvidia: Option<Box<NvidiaGpuProvenance>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceClass {
    Cpu,
    NvidiaCuda,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NvidiaGpuProvenance {
    pub vendor: String,
    pub device_name: String,
    pub logical_device_index: u32,
    pub total_vram_bytes: u64,
    pub nvidia_driver_version: String,
    pub cuda_driver_version: String,
    pub cuda_runtime_version: String,
    pub cuda_toolkit_version: String,
    pub compute_capability: String,
    pub offload: NvidiaOffloadProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NvidiaOffloadProvenance {
    pub policy: String,
    pub offloaded_layers: u32,
    pub total_layers: u32,
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
pub struct RequestObservation {
    pub request_index: u32,
    pub latency_micros: u64,
    pub time_to_first_token_micros: u64,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_observations: Vec<RequestObservation>,
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
    pub resources: Option<ProcessResources>,
    pub bundle_path: String,
    pub experiment_digest: String,
    pub resolved_plan_digest: String,
    pub evidence_digest: String,
    pub failure: Option<FailureRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceComparison {
    pub format: String,
    pub baseline: BundleIdentity,
    pub candidate: BundleIdentity,
    pub compatible: bool,
    pub incompatibilities: Vec<String>,
    pub measurement_contract: Option<LlamaMeasurementContract>,
    pub environment: Vec<EnvironmentComparison>,
    pub requests: Option<RequestScopeComparison>,
    pub repetitions: Option<RepetitionScopeComparison>,
    pub attempt_resources: Option<AttemptResourceComparison>,
    pub interpretation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleIdentity {
    pub bundle_path: String,
    pub run_id: String,
    pub experiment_digest: String,
    pub resolved_plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlamaMeasurementContract {
    pub provider: String,
    pub runtime: String,
    pub target: crate::LlamaCppTarget,
    pub generator: String,
    pub model: String,
    pub model_sha256: String,
    pub engine_name: String,
    pub engine_version: String,
    pub backend: String,
    pub device_class: DeviceClass,
    pub workload_profile: String,
    pub output_tokens: u32,
    pub requests_per_repetition: u32,
    pub concurrency: u32,
    pub warmup_runs: u32,
    pub measured_repetitions: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentRelationship {
    Same,
    Different,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentComparison {
    pub field: String,
    pub baseline: Option<String>,
    pub candidate: Option<String>,
    pub relationship: EnvironmentRelationship,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegerDelta {
    pub absolute_delta: i128,
    /// Thousandths of one percent. Absent when the baseline is zero.
    pub percentage_delta_milli_percent: Option<i128>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricComparison {
    pub baseline: u64,
    pub candidate: u64,
    pub delta: IntegerDelta,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DistributionComparison {
    pub baseline: LatencySummary,
    pub candidate: LatencySummary,
    pub mean: MetricComparison,
    pub p50: MetricComparison,
    pub p95: MetricComparison,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestScopeComparison {
    pub unit: String,
    pub baseline_count: u64,
    pub candidate_count: u64,
    pub latency_micros: DistributionComparison,
    pub time_to_first_token_micros: DistributionComparison,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepetitionScopeComparison {
    pub unit: String,
    pub baseline_count: u64,
    pub candidate_count: u64,
    pub aggregate_latency_micros: DistributionComparison,
    pub aggregate_time_to_first_token_micros: DistributionComparison,
    pub mean_throughput_milli_requests_per_second: MetricComparison,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptResourceValues {
    pub scope: ResourceScope,
    pub cpu_time_micros: u64,
    pub peak_rss_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptResourceComparison {
    pub unit: String,
    pub baseline: Option<AttemptResourceValues>,
    pub candidate: Option<AttemptResourceValues>,
    pub cpu_time_micros: Option<MetricComparison>,
    pub peak_rss_bytes: Option<MetricComparison>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aggregate_json() -> serde_json::Value {
        serde_json::json!({
            "generator": "benchplane-llama-cpp-smollm2/v1",
            "attemptNumber": 1,
            "phase": "measured",
            "repetitionIndex": 1,
            "sampleIndex": 1,
            "latencyMicros": 20,
            "timeToFirstTokenMicros": 10,
            "throughputMilliRequestsPerSecond": 1000,
            "successfulRequests": 2,
            "failedRequests": 0
        })
    }

    #[test]
    fn historical_measurements_default_to_no_request_observations() {
        let record: MeasurementRecord =
            serde_json::from_value(aggregate_json()).expect("historical aggregate record");
        assert!(record.request_observations.is_empty());
        assert!(serde_json::to_value(record)
            .expect("serialize historical shape")
            .get("requestObservations")
            .is_none());
    }

    #[test]
    fn request_observations_round_trip_as_bounded_numeric_data() {
        let mut json = aggregate_json();
        json["generator"] = serde_json::json!("benchplane-llama-cpp-smollm2/v2");
        json["requestObservations"] = serde_json::json!([
            {
                "requestIndex": 1,
                "latencyMicros": 19,
                "timeToFirstTokenMicros": 9
            },
            {
                "requestIndex": 2,
                "latencyMicros": 21,
                "timeToFirstTokenMicros": 11
            }
        ]);
        let record: MeasurementRecord = serde_json::from_value(json).expect("v2 measurement");
        assert_eq!(record.request_observations.len(), 2);
        assert_eq!(record.request_observations[1].request_index, 2);
        let serialized = serde_json::to_vec(&record).expect("serialize v2 measurement");
        assert_eq!(
            serde_json::from_slice::<MeasurementRecord>(&serialized).expect("round trip"),
            record
        );
    }
}
