// SPDX-License-Identifier: Apache-2.0

use crate::evidence::{verify_evidence_bundle_data, EvidenceError, VerifiedEvidenceBundle};
use benchplane_schema::{
    AttemptProvenance, AttemptResourceComparison, AttemptResourceValues, AttemptResources,
    BundleIdentity, DistributionComparison, EnvironmentComparison, EnvironmentRelationship,
    EvidenceComparison, IntegerDelta, LlamaMeasurementContract, MeasurementPhase, MetricComparison,
    ProviderSpec, RepetitionScopeComparison, RequestScopeComparison, RuntimeProvenance,
    RuntimeSpec, ValidityStatus, EVIDENCE_COMPARISON_FORMAT_V1, LLAMA_CPP_GENERATOR_VERSION,
};
use std::path::Path;
use thiserror::Error;

const DESCRIPTIVE_NOTE: &str = "Descriptive only: requests and repetitions within each run share one initialized helper/model lifetime; no significance, independence, causality, or cross-host equivalence is claimed.";

#[derive(Debug, Error)]
pub enum ComparisonError {
    #[error("{role} evidence bundle is invalid: {source}")]
    InvalidEvidence {
        role: &'static str,
        #[source]
        source: EvidenceError,
    },
    #[error("{role} evidence bundle is not comparison-eligible: {reason}")]
    Ineligible { role: &'static str, reason: String },
    #[error("baseline and candidate must be distinct Benchplane runs")]
    SameRun,
}

pub fn compare_evidence_bundles(
    baseline_path: &Path,
    candidate_path: &Path,
) -> Result<EvidenceComparison, ComparisonError> {
    let baseline = verify_evidence_bundle_data(baseline_path).map_err(|source| {
        ComparisonError::InvalidEvidence {
            role: "baseline",
            source,
        }
    })?;
    let candidate = verify_evidence_bundle_data(candidate_path).map_err(|source| {
        ComparisonError::InvalidEvidence {
            role: "candidate",
            source,
        }
    })?;
    if baseline.manifest.run_id == candidate.manifest.run_id {
        return Err(ComparisonError::SameRun);
    }

    let baseline_contract = eligible_contract("baseline", &baseline)?;
    let candidate_contract = eligible_contract("candidate", &candidate)?;
    let incompatibilities = contract_differences(&baseline_contract, &candidate_contract);
    let compatible = incompatibilities.is_empty();
    let environment = compare_environment(
        baseline
            .provenance
            .as_ref()
            .expect("eligibility requires provenance"),
        candidate
            .provenance
            .as_ref()
            .expect("eligibility requires provenance"),
    );

    let (requests, repetitions, attempt_resources) = if compatible {
        (
            Some(compare_requests(&baseline, &candidate)),
            Some(compare_repetitions(&baseline, &candidate)),
            compare_resources(baseline.resources.as_ref(), candidate.resources.as_ref()),
        )
    } else {
        (None, None, None)
    };

    Ok(EvidenceComparison {
        format: EVIDENCE_COMPARISON_FORMAT_V1.to_owned(),
        baseline: bundle_identity(&baseline),
        candidate: bundle_identity(&candidate),
        compatible,
        incompatibilities,
        measurement_contract: compatible.then_some(baseline_contract),
        environment,
        requests,
        repetitions,
        attempt_resources,
        interpretation: DESCRIPTIVE_NOTE.to_owned(),
    })
}

fn eligible_contract(
    role: &'static str,
    bundle: &VerifiedEvidenceBundle,
) -> Result<LlamaMeasurementContract, ComparisonError> {
    if bundle.manifest.run_status != benchplane_schema::RunState::Succeeded
        || bundle.manifest.validity_status != ValidityStatus::Valid
    {
        return ineligible(role, "requires a successful run with valid measurements");
    }
    let (target, model, output_tokens) = match (
        &bundle.plan.experiment.spec.provider,
        &bundle.plan.experiment.spec.runtime,
    ) {
        (
            ProviderSpec::Local,
            RuntimeSpec::LlamaCpp {
                target,
                model,
                output_tokens,
            },
        ) => (*target, model.clone(), *output_tokens),
        _ => return ineligible(role, "requires the local llamaCpp runtime"),
    };
    if bundle
        .measurements
        .iter()
        .any(|record| record.generator != LLAMA_CPP_GENERATOR_VERSION)
    {
        return ineligible(
            role,
            "requires current request-level benchplane-llama-cpp-smollm2/v2 measurements",
        );
    }
    let Some(AttemptProvenance { software, .. }) = bundle.provenance.as_ref() else {
        return ineligible(role, "requires attempt provenance");
    };
    let RuntimeProvenance::LlamaCpp {
        generator,
        engine,
        model: provenance_model,
        backend,
    } = &software.runtime
    else {
        return ineligible(role, "requires llamaCpp attempt provenance");
    };
    if generator != LLAMA_CPP_GENERATOR_VERSION {
        return ineligible(role, "requires current llamaCpp v2 provenance");
    }

    Ok(LlamaMeasurementContract {
        provider: "local".to_owned(),
        runtime: "llamaCpp".to_owned(),
        target,
        generator: generator.clone(),
        model,
        model_sha256: provenance_model.sha256.clone(),
        engine_name: engine.name.clone(),
        engine_version: engine.version.clone(),
        backend: backend.identity.clone(),
        device_class: backend.device_class,
        workload_profile: bundle.plan.experiment.spec.workload.profile.clone(),
        output_tokens,
        requests_per_repetition: bundle.plan.experiment.spec.workload.requests,
        concurrency: bundle.plan.experiment.spec.workload.concurrency,
        warmup_runs: bundle.plan.experiment.spec.measurement.warmup_runs,
        measured_repetitions: bundle.plan.experiment.spec.measurement.repetitions,
    })
}

fn ineligible<T>(role: &'static str, reason: &str) -> Result<T, ComparisonError> {
    Err(ComparisonError::Ineligible {
        role,
        reason: reason.to_owned(),
    })
}

fn contract_differences(
    baseline: &LlamaMeasurementContract,
    candidate: &LlamaMeasurementContract,
) -> Vec<String> {
    let mut differences = Vec::new();
    macro_rules! compare {
        ($field:ident, $name:literal) => {
            if baseline.$field != candidate.$field {
                differences.push($name.to_owned());
            }
        };
    }
    compare!(provider, "provider");
    compare!(runtime, "runtime");
    compare!(target, "target");
    compare!(generator, "generator");
    compare!(model, "model");
    compare!(model_sha256, "modelSha256");
    compare!(engine_name, "engineName");
    compare!(engine_version, "engineVersion");
    compare!(backend, "backend");
    compare!(device_class, "deviceClass");
    compare!(workload_profile, "workloadProfile");
    compare!(output_tokens, "outputTokens");
    compare!(requests_per_repetition, "requestsPerRepetition");
    compare!(concurrency, "concurrency");
    compare!(warmup_runs, "warmupRuns");
    compare!(measured_repetitions, "measuredRepetitions");
    differences
}

fn bundle_identity(bundle: &VerifiedEvidenceBundle) -> BundleIdentity {
    BundleIdentity {
        bundle_path: bundle.root.display().to_string(),
        run_id: bundle.manifest.run_id.clone(),
        experiment_digest: bundle.manifest.experiment_digest.clone(),
        resolved_plan_digest: bundle.manifest.resolved_plan_digest.clone(),
    }
}

fn compare_requests(
    baseline: &VerifiedEvidenceBundle,
    candidate: &VerifiedEvidenceBundle,
) -> RequestScopeComparison {
    let (baseline_latency, baseline_ttft) = request_values(baseline);
    let (candidate_latency, candidate_ttft) = request_values(candidate);
    RequestScopeComparison {
        unit: "measuredRequest".to_owned(),
        baseline_count: baseline_latency.len() as u64,
        candidate_count: candidate_latency.len() as u64,
        latency_micros: compare_distribution(&baseline_latency, &candidate_latency),
        time_to_first_token_micros: compare_distribution(&baseline_ttft, &candidate_ttft),
    }
}

fn request_values(bundle: &VerifiedEvidenceBundle) -> (Vec<u64>, Vec<u64>) {
    bundle
        .measurements
        .iter()
        .filter(|record| record.phase == MeasurementPhase::Measured)
        .flat_map(|record| record.request_observations.iter())
        .map(|observation| {
            (
                observation.latency_micros,
                observation.time_to_first_token_micros,
            )
        })
        .unzip()
}

fn compare_repetitions(
    baseline: &VerifiedEvidenceBundle,
    candidate: &VerifiedEvidenceBundle,
) -> RepetitionScopeComparison {
    let (baseline_latency, baseline_ttft, baseline_throughput) = repetition_values(baseline);
    let (candidate_latency, candidate_ttft, candidate_throughput) = repetition_values(candidate);
    RepetitionScopeComparison {
        unit: "measuredRepetitionAggregate".to_owned(),
        baseline_count: baseline_latency.len() as u64,
        candidate_count: candidate_latency.len() as u64,
        aggregate_latency_micros: compare_distribution(&baseline_latency, &candidate_latency),
        aggregate_time_to_first_token_micros: compare_distribution(&baseline_ttft, &candidate_ttft),
        mean_throughput_milli_requests_per_second: compare_metric(
            crate::execution::mean_u64(&baseline_throughput)
                .expect("eligible evidence has measured repetitions"),
            crate::execution::mean_u64(&candidate_throughput)
                .expect("eligible evidence has measured repetitions"),
        ),
    }
}

fn repetition_values(bundle: &VerifiedEvidenceBundle) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let mut latency = Vec::new();
    let mut ttft = Vec::new();
    let mut throughput = Vec::new();
    for record in bundle
        .measurements
        .iter()
        .filter(|record| record.phase == MeasurementPhase::Measured)
    {
        latency.push(record.latency_micros);
        ttft.push(record.time_to_first_token_micros);
        throughput.push(record.throughput_milli_requests_per_second);
    }
    (latency, ttft, throughput)
}

fn compare_distribution(baseline: &[u64], candidate: &[u64]) -> DistributionComparison {
    let baseline =
        crate::execution::describe_micros(baseline).expect("eligible evidence has measured values");
    let candidate = crate::execution::describe_micros(candidate)
        .expect("eligible evidence has measured values");
    DistributionComparison {
        mean: compare_metric(baseline.mean_micros, candidate.mean_micros),
        p50: compare_metric(baseline.p50_micros, candidate.p50_micros),
        p95: compare_metric(baseline.p95_micros, candidate.p95_micros),
        baseline,
        candidate,
    }
}

fn compare_metric(baseline: u64, candidate: u64) -> MetricComparison {
    let absolute_delta = i128::from(candidate) - i128::from(baseline);
    let percentage_delta_milli_percent =
        (baseline != 0).then(|| absolute_delta * 100_000 / i128::from(baseline));
    MetricComparison {
        baseline,
        candidate,
        delta: IntegerDelta {
            absolute_delta,
            percentage_delta_milli_percent,
        },
    }
}

fn compare_resources(
    baseline: Option<&AttemptResources>,
    candidate: Option<&AttemptResources>,
) -> Option<AttemptResourceComparison> {
    if baseline.is_none() && candidate.is_none() {
        return None;
    }
    let baseline_values = baseline.map(resource_values);
    let candidate_values = candidate.map(resource_values);
    let (cpu_time_micros, peak_rss_bytes) = match (baseline, candidate) {
        (Some(baseline), Some(candidate)) => (
            Some(compare_metric(
                baseline.cpu_time_micros,
                candidate.cpu_time_micros,
            )),
            Some(compare_metric(
                baseline.peak_rss_bytes,
                candidate.peak_rss_bytes,
            )),
        ),
        _ => (None, None),
    };
    Some(AttemptResourceComparison {
        unit: "helperProcessLifetime".to_owned(),
        baseline: baseline_values,
        candidate: candidate_values,
        cpu_time_micros,
        peak_rss_bytes,
    })
}

fn resource_values(resources: &AttemptResources) -> AttemptResourceValues {
    AttemptResourceValues {
        scope: resources.scope,
        cpu_time_micros: resources.cpu_time_micros,
        peak_rss_bytes: resources.peak_rss_bytes,
    }
}

fn compare_environment(
    baseline: &AttemptProvenance,
    candidate: &AttemptProvenance,
) -> Vec<EnvironmentComparison> {
    let mut fields = vec![
        environment_field(
            "platform.architecture",
            Some(&baseline.platform.architecture),
            Some(&candidate.platform.architecture),
        ),
        environment_field(
            "platform.cpu.model",
            baseline.platform.cpu.model.as_ref(),
            candidate.platform.cpu.model.as_ref(),
        ),
        environment_field(
            "platform.cpu.logicalCpuCount",
            baseline
                .platform
                .cpu
                .logical_cpu_count
                .map(|value| value.to_string())
                .as_ref(),
            candidate
                .platform
                .cpu
                .logical_cpu_count
                .map(|value| value.to_string())
                .as_ref(),
        ),
        environment_field(
            "platform.operatingSystem.family",
            Some(&baseline.platform.operating_system.family),
            Some(&candidate.platform.operating_system.family),
        ),
        environment_field(
            "platform.operatingSystem.distribution",
            baseline.platform.operating_system.distribution.as_ref(),
            candidate.platform.operating_system.distribution.as_ref(),
        ),
        environment_field(
            "platform.operatingSystem.version",
            baseline.platform.operating_system.version.as_ref(),
            candidate.platform.operating_system.version.as_ref(),
        ),
        environment_field(
            "platform.kernel.name",
            Some(&baseline.platform.kernel.name),
            Some(&candidate.platform.kernel.name),
        ),
        environment_field(
            "platform.kernel.release",
            baseline.platform.kernel.release.as_ref(),
            candidate.platform.kernel.release.as_ref(),
        ),
        environment_field(
            "software.benchplane.version",
            Some(&baseline.software.benchplane.version),
            Some(&candidate.software.benchplane.version),
        ),
        environment_field(
            "software.benchplane.nixStorePath",
            baseline.software.benchplane.nix_store_path.as_ref(),
            candidate.software.benchplane.nix_store_path.as_ref(),
        ),
    ];
    if let (
        RuntimeProvenance::LlamaCpp {
            engine: baseline_engine,
            model: baseline_model,
            backend: baseline_backend,
            ..
        },
        RuntimeProvenance::LlamaCpp {
            engine: candidate_engine,
            model: candidate_model,
            backend: candidate_backend,
            ..
        },
    ) = (&baseline.software.runtime, &candidate.software.runtime)
    {
        fields.extend([
            environment_field(
                "software.runtime.engine.nixStorePath",
                baseline_engine.nix_store_path.as_ref(),
                candidate_engine.nix_store_path.as_ref(),
            ),
            environment_field(
                "software.runtime.model.nixStorePath",
                baseline_model.nix_store_path.as_ref(),
                candidate_model.nix_store_path.as_ref(),
            ),
            environment_field(
                "software.runtime.backend.nixStorePath",
                baseline_backend.nix_store_path.as_ref(),
                candidate_backend.nix_store_path.as_ref(),
            ),
        ]);
        match (&baseline_backend.nvidia, &candidate_backend.nvidia) {
            (Some(baseline), Some(candidate)) => fields.extend([
                environment_field(
                    "software.runtime.backend.nvidia.deviceName",
                    Some(&baseline.device_name),
                    Some(&candidate.device_name),
                ),
                environment_field(
                    "software.runtime.backend.nvidia.totalVramBytes",
                    Some(&baseline.total_vram_bytes.to_string()),
                    Some(&candidate.total_vram_bytes.to_string()),
                ),
                environment_field(
                    "software.runtime.backend.nvidia.nvidiaDriverVersion",
                    Some(&baseline.nvidia_driver_version),
                    Some(&candidate.nvidia_driver_version),
                ),
                environment_field(
                    "software.runtime.backend.nvidia.cudaDriverVersion",
                    Some(&baseline.cuda_driver_version),
                    Some(&candidate.cuda_driver_version),
                ),
                environment_field(
                    "software.runtime.backend.nvidia.cudaRuntimeVersion",
                    Some(&baseline.cuda_runtime_version),
                    Some(&candidate.cuda_runtime_version),
                ),
                environment_field(
                    "software.runtime.backend.nvidia.cudaToolkitVersion",
                    Some(&baseline.cuda_toolkit_version),
                    Some(&candidate.cuda_toolkit_version),
                ),
                environment_field(
                    "software.runtime.backend.nvidia.computeCapability",
                    Some(&baseline.compute_capability),
                    Some(&candidate.compute_capability),
                ),
                environment_field(
                    "software.runtime.backend.nvidia.offload",
                    Some(&format!(
                        "{}:{}/{}",
                        baseline.offload.policy,
                        baseline.offload.offloaded_layers,
                        baseline.offload.total_layers
                    )),
                    Some(&format!(
                        "{}:{}/{}",
                        candidate.offload.policy,
                        candidate.offload.offloaded_layers,
                        candidate.offload.total_layers
                    )),
                ),
            ]),
            (Some(_), None) | (None, Some(_)) => fields.push(EnvironmentComparison {
                field: "software.runtime.backend.nvidia".to_owned(),
                baseline: baseline_backend
                    .nvidia
                    .as_ref()
                    .map(|_| "present".to_owned()),
                candidate: candidate_backend
                    .nvidia
                    .as_ref()
                    .map(|_| "present".to_owned()),
                relationship: EnvironmentRelationship::Unknown,
            }),
            (None, None) => {}
        }
    }
    fields
}

fn environment_field(
    field: &str,
    baseline: Option<&String>,
    candidate: Option<&String>,
) -> EnvironmentComparison {
    let relationship = match (baseline, candidate) {
        (Some(baseline), Some(candidate)) if baseline == candidate => EnvironmentRelationship::Same,
        (Some(_), Some(_)) => EnvironmentRelationship::Different,
        _ => EnvironmentRelationship::Unknown,
    };
    EnvironmentComparison {
        field: field.to_owned(),
        baseline: baseline.cloned(),
        candidate: candidate.cloned(),
        relationship,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use benchplane_schema::DeviceClass;

    fn contract() -> LlamaMeasurementContract {
        LlamaMeasurementContract {
            provider: "local".to_owned(),
            runtime: "llamaCpp".to_owned(),
            target: benchplane_schema::LlamaCppTarget::Cpu,
            generator: LLAMA_CPP_GENERATOR_VERSION.to_owned(),
            model: "model".to_owned(),
            model_sha256: "sha256:model".to_owned(),
            engine_name: "llama.cpp".to_owned(),
            engine_version: "engine".to_owned(),
            backend: "backend".to_owned(),
            device_class: DeviceClass::Cpu,
            workload_profile: "profile".to_owned(),
            output_tokens: 4,
            requests_per_repetition: 2,
            concurrency: 1,
            warmup_runs: 1,
            measured_repetitions: 3,
        }
    }

    #[test]
    fn deterministic_integer_deltas_and_undefined_zero_baseline_percentage() {
        assert_eq!(
            compare_metric(100, 100).delta,
            IntegerDelta {
                absolute_delta: 0,
                percentage_delta_milli_percent: Some(0),
            }
        );
        assert_eq!(
            compare_metric(100, 125).delta,
            IntegerDelta {
                absolute_delta: 25,
                percentage_delta_milli_percent: Some(25_000),
            }
        );
        assert_eq!(
            compare_metric(0, 1).delta,
            IntegerDelta {
                absolute_delta: 1,
                percentage_delta_milli_percent: None,
            }
        );
    }

    #[test]
    fn distributions_use_floor_mean_and_nearest_rank() {
        let compared = compare_distribution(&[10, 20, 30], &[20, 30, 40]);
        assert_eq!(compared.baseline.mean_micros, 20);
        assert_eq!(compared.baseline.p50_micros, 20);
        assert_eq!(compared.baseline.p95_micros, 30);
        assert_eq!(compared.mean.delta.absolute_delta, 10);
    }

    #[test]
    fn missing_environment_values_are_unknown_even_on_both_sides() {
        let compared = environment_field("cpu.model", None, None);
        assert_eq!(compared.relationship, EnvironmentRelationship::Unknown);
    }

    #[test]
    fn compatibility_is_fieldwise_not_resolved_digest_equality() {
        let baseline = contract();
        let mut candidate = baseline.clone();
        candidate.output_tokens = 8;
        candidate.requests_per_repetition = 3;
        candidate.warmup_runs = 2;
        assert_eq!(
            contract_differences(&baseline, &candidate),
            ["outputTokens", "requestsPerRepetition", "warmupRuns"]
        );
    }
}
