// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{
    FailureRecord, LatencySummary, MeasurementPhase, MeasurementRecord, ResolvedExperiment,
    RunState, RunSummary, ValidityReason, ValidityResult, ValidityStatus,
};

#[derive(Debug)]
pub(crate) struct ExecutionOutput {
    pub measurements: Vec<MeasurementRecord>,
    pub terminal_state: RunState,
    pub failure: Option<FailureRecord>,
}

pub(crate) fn evaluate_validity(
    plan: &ResolvedExperiment,
    output: &ExecutionOutput,
    run_id: &str,
) -> ValidityResult {
    let observed = measured_records(&output.measurements).count() as u32;
    let required = plan.experiment.spec.measurement.repetitions;
    let (status, reasons) = match output.terminal_state {
        RunState::Succeeded if observed >= required => (ValidityStatus::Valid, Vec::new()),
        RunState::Succeeded => (
            ValidityStatus::Invalid,
            vec![ValidityReason {
                code: "insufficientSamples".to_owned(),
                message: format!("required {required} measured samples but observed {observed}"),
            }],
        ),
        RunState::Failed => (
            ValidityStatus::Indeterminate,
            vec![ValidityReason {
                code: "runtimeFailure".to_owned(),
                message: "measurement validity is indeterminate after runtime failure".to_owned(),
            }],
        ),
        RunState::Interrupted => (
            ValidityStatus::Indeterminate,
            vec![ValidityReason {
                code: "interrupted".to_owned(),
                message: "measurement validity is indeterminate after interruption".to_owned(),
            }],
        ),
        state => unreachable!("execution produced nonterminal state {state:?}"),
    };

    ValidityResult {
        run_id: run_id.to_owned(),
        status,
        required_samples: required,
        observed_samples: observed,
        reasons,
    }
}

pub(crate) fn summarize(
    plan: &ResolvedExperiment,
    output: &ExecutionOutput,
    validity: &ValidityResult,
    run_id: &str,
) -> RunSummary {
    let measured: Vec<_> = measured_records(&output.measurements).collect();
    let sample_count = measured.len() as u32;
    let (latency, mean_throughput) =
        if measured.is_empty() || validity.status == ValidityStatus::Indeterminate {
            (None, None)
        } else {
            let mut latencies: Vec<u64> = measured
                .iter()
                .map(|record| record.latency_micros)
                .collect();
            latencies.sort_unstable();
            let latency_sum: u64 = latencies.iter().sum();
            let throughput_sum: u64 = measured
                .iter()
                .map(|record| record.throughput_milli_requests_per_second)
                .sum();
            (
                Some(LatencySummary {
                    mean_micros: latency_sum / sample_count as u64,
                    p50_micros: nearest_rank(&latencies, 50),
                    p95_micros: nearest_rank(&latencies, 95),
                }),
                Some(throughput_sum / sample_count as u64),
            )
        };

    RunSummary {
        run_id: run_id.to_owned(),
        run_status: output.terminal_state,
        attempt_count: 1,
        sample_count,
        validity_status: validity.status,
        latency,
        mean_throughput_milli_requests_per_second: mean_throughput,
        experiment_digest: plan.experiment_digest.clone(),
        resolved_plan_digest: plan.resolved_plan_digest.clone(),
    }
}

fn measured_records(records: &[MeasurementRecord]) -> impl Iterator<Item = &MeasurementRecord> {
    records
        .iter()
        .filter(|record| record.phase == MeasurementPhase::Measured)
}

fn nearest_rank(sorted: &[u64], percentile: usize) -> u64 {
    let rank = (percentile * sorted.len()).div_ceil(100);
    sorted[rank.saturating_sub(1)]
}
