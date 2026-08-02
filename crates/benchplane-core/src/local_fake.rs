// SPDX-License-Identifier: Apache-2.0

use crate::execution::ExecutionOutput;
use benchplane_schema::{
    FailureRecord, LocalFakeScenario, MeasurementPhase, MeasurementRecord, ResolvedExperiment,
    RunState, ERROR_LOCAL_FAKE_INTERRUPTED, ERROR_LOCAL_FAKE_RUNTIME_FAILURE,
    LOCAL_FAKE_GENERATOR_VERSION,
};
use sha2::{Digest, Sha256};

pub(crate) fn execute(
    plan: &ResolvedExperiment,
    seed: u64,
    scenario: LocalFakeScenario,
) -> ExecutionOutput {
    let spec = &plan.experiment.spec.measurement;
    let mut measurements = Vec::new();

    for index in 1..=spec.warmup_runs {
        measurements.push(measurement(plan, seed, MeasurementPhase::Warmup, index, 1));
    }

    let measured_count = match scenario {
        LocalFakeScenario::Success => spec.repetitions,
        LocalFakeScenario::InsufficientMeasurements => spec.repetitions.saturating_sub(1),
        LocalFakeScenario::RuntimeFailure | LocalFakeScenario::Interrupted => {
            spec.repetitions.saturating_sub(1).min(1)
        }
    };
    for index in 1..=measured_count {
        measurements.push(measurement(
            plan,
            seed,
            MeasurementPhase::Measured,
            index,
            1,
        ));
    }

    match scenario {
        LocalFakeScenario::Success | LocalFakeScenario::InsufficientMeasurements => {
            ExecutionOutput {
                measurements,
                terminal_state: RunState::Succeeded,
                failure: None,
            }
        }
        LocalFakeScenario::RuntimeFailure => ExecutionOutput {
            measurements,
            terminal_state: RunState::Failed,
            failure: Some(FailureRecord {
                phase: "running".to_owned(),
                code: ERROR_LOCAL_FAKE_RUNTIME_FAILURE.to_owned(),
                message: "deterministic local-fake runtime failure".to_owned(),
                retryable: false,
                attempt_number: 1,
            }),
        },
        LocalFakeScenario::Interrupted => ExecutionOutput {
            measurements,
            terminal_state: RunState::Interrupted,
            failure: Some(FailureRecord {
                phase: "running".to_owned(),
                code: ERROR_LOCAL_FAKE_INTERRUPTED.to_owned(),
                message: "deterministic local-fake interruption".to_owned(),
                retryable: false,
                attempt_number: 1,
            }),
        },
    }
}

fn measurement(
    plan: &ResolvedExperiment,
    seed: u64,
    phase: MeasurementPhase,
    repetition_index: u32,
    sample_index: u32,
) -> MeasurementRecord {
    let phase_tag = match phase {
        MeasurementPhase::Warmup => "warmup",
        MeasurementPhase::Measured => "measured",
    };
    let metric = |tag, minimum, span| {
        derive_value(
            &plan.resolved_plan_digest,
            seed,
            phase_tag,
            repetition_index,
            sample_index,
            tag,
            minimum,
            span,
        )
    };

    MeasurementRecord {
        generator: LOCAL_FAKE_GENERATOR_VERSION.to_owned(),
        attempt_number: 1,
        phase,
        repetition_index,
        sample_index,
        latency_micros: metric("latencyMicros", 10_000, 40_001),
        time_to_first_token_micros: metric("timeToFirstTokenMicros", 1_000, 9_001),
        throughput_milli_requests_per_second: metric(
            "throughputMilliRequestsPerSecond",
            10_000,
            90_001,
        ),
        successful_requests: plan.experiment.spec.workload.requests,
        failed_requests: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn derive_value(
    plan_digest: &str,
    seed: u64,
    phase: &str,
    repetition_index: u32,
    sample_index: u32,
    metric_tag: &str,
    minimum: u64,
    span: u64,
) -> u64 {
    let seed = seed.to_be_bytes();
    let repetition = repetition_index.to_be_bytes();
    let sample = sample_index.to_be_bytes();
    let components: [&[u8]; 7] = [
        LOCAL_FAKE_GENERATOR_VERSION.as_bytes(),
        plan_digest.as_bytes(),
        &seed,
        phase.as_bytes(),
        &repetition,
        &sample,
        metric_tag.as_bytes(),
    ];
    let mut hasher = Sha256::new();
    for component in components {
        hasher.update((component.len() as u64).to_be_bytes());
        hasher.update(component);
    }
    let digest = hasher.finalize();
    let value = u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix length"));
    minimum + value % span
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{evaluate_validity, summarize};
    use crate::resolve_experiment;
    use benchplane_schema::{Experiment, ValidityStatus};

    fn plan(seed: u64) -> ResolvedExperiment {
        let experiment: Experiment = serde_json::from_value(serde_json::json!({
            "apiVersion": "benchplane/v1alpha1",
            "kind": "Experiment",
            "metadata": { "name": "deterministic" },
            "spec": {
                "provider": { "kind": "localFake" },
                "runtime": { "kind": "localFake", "seed": seed, "scenario": "success" },
                "workload": { "profile": "smoke", "requests": 8 },
                "measurement": { "warmupRuns": 1, "repetitions": 3 },
                "budget": { "maximumCostUsd": 0 }
            }
        }))
        .expect("experiment should deserialize");
        resolve_experiment(experiment).expect("experiment should resolve")
    }

    #[test]
    fn same_plan_and_seed_produce_identical_measurements_and_summary() {
        let plan = plan(42);
        let first = execute(&plan, 42, LocalFakeScenario::Success);
        let second = execute(&plan, 42, LocalFakeScenario::Success);
        let first_validity = evaluate_validity(&plan, &first, "run-fixed");
        let second_validity = evaluate_validity(&plan, &second, "run-fixed");

        assert_eq!(first.measurements, second.measurements);
        assert_eq!(
            summarize(&plan, &first, &first_validity, "run-fixed"),
            summarize(&plan, &second, &second_validity, "run-fixed")
        );
    }

    #[test]
    fn different_seed_changes_measurements() {
        let first_plan = plan(42);
        let second_plan = plan(43);
        let first = execute(&first_plan, 42, LocalFakeScenario::Success);
        let second = execute(&second_plan, 43, LocalFakeScenario::Success);
        assert_ne!(first.measurements, second.measurements);
    }

    #[test]
    fn scenario_validity_is_separate_from_operational_status() {
        let plan = plan(42);
        let insufficient = execute(&plan, 42, LocalFakeScenario::InsufficientMeasurements);
        let validity = evaluate_validity(&plan, &insufficient, "run-fixed");
        assert_eq!(insufficient.terminal_state, RunState::Succeeded);
        assert_eq!(validity.status, ValidityStatus::Invalid);
        assert_eq!(validity.reasons[0].code, "insufficientSamples");

        for scenario in [
            LocalFakeScenario::RuntimeFailure,
            LocalFakeScenario::Interrupted,
        ] {
            let output = execute(&plan, 42, scenario);
            assert_eq!(
                evaluate_validity(&plan, &output, "run-fixed").status,
                ValidityStatus::Indeterminate
            );
        }
    }
}
