// SPDX-License-Identifier: Apache-2.0

use benchplane_core::{
    compare_evidence_bundles, parse_experiment, resolve_experiment, run_experiment,
    verify_evidence_bundle, EvidenceComparison, MetricComparison, ResolutionError, RunOptions,
};
use benchplane_schema::{Experiment, RunResult, RunState, ValidityStatus};
use clap::{Parser, Subcommand};
use schemars::schema_for;
use std::{fs, path::PathBuf, process::ExitCode};

const EXIT_SUCCESS: u8 = 0;
const EXIT_INTERNAL: u8 = 1;
const EXIT_REJECTED: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_OPERATIONAL_FAILURE: u8 = 4;
const EXIT_FINALIZATION_FAILURE: u8 = 5;

#[derive(Debug, Parser)]
#[command(name = "benchplane")]
#[command(about = "Reproducible AI systems experiments, from specification to evidence")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and semantically validate an experiment specification.
    Validate { experiment: PathBuf },

    /// Resolve defaults and print a deterministic machine-readable plan.
    Resolve { experiment: PathBuf },

    /// Execute one supported synchronous local experiment.
    Run {
        experiment: PathBuf,
        #[arg(long, default_value = ".benchplane")]
        output_root: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Work with public schemas.
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },

    /// Work with evidence bundles.
    Evidence {
        #[command(subcommand)]
        command: EvidenceCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SchemaCommand {
    /// Export the current experiment JSON Schema to standard output.
    Export,
}

#[derive(Debug, Subcommand)]
enum EvidenceCommand {
    /// Verify evidence checksums and bounded semantic consistency.
    Verify { bundle: PathBuf },

    /// Descriptively compare two verified current llama.cpp evidence bundles.
    Compare {
        baseline_bundle: PathBuf,
        candidate_bundle: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    ExitCode::from(execute(Cli::parse()))
}

fn execute(cli: Cli) -> u8 {
    match cli.command {
        Command::Validate { experiment } => validate_command(&experiment),
        Command::Resolve { experiment } => resolve_command(&experiment),
        Command::Run {
            experiment,
            output_root,
            json,
        } => run_command(&experiment, output_root, json),
        Command::Schema {
            command: SchemaCommand::Export,
        } => {
            let schema = schema_for!(Experiment);
            match serde_json::to_string_pretty(&schema) {
                Ok(schema) => {
                    println!("{schema}");
                    EXIT_SUCCESS
                }
                Err(error) => internal_error(error),
            }
        }
        Command::Evidence {
            command: EvidenceCommand::Verify { bundle },
        } => match verify_evidence_bundle(&bundle) {
            Ok(manifest) => {
                println!(
                    "verified: run={} status={} validity={} experiment={} plan={}",
                    manifest.run_id,
                    manifest.run_status.as_str(),
                    manifest.validity_status.as_str(),
                    manifest.experiment_digest,
                    manifest.resolved_plan_digest,
                );
                EXIT_SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                EXIT_FINALIZATION_FAILURE
            }
        },
        Command::Evidence {
            command:
                EvidenceCommand::Compare {
                    baseline_bundle,
                    candidate_bundle,
                    json,
                },
        } => match compare_evidence_bundles(&baseline_bundle, &candidate_bundle) {
            Ok(comparison) => {
                if json {
                    match serde_json::to_string(&comparison) {
                        Ok(output) => println!("{output}"),
                        Err(error) => return internal_error(error),
                    }
                } else {
                    print_human_comparison(&comparison);
                }
                EXIT_SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                EXIT_FINALIZATION_FAILURE
            }
        },
    }
}

fn print_human_comparison(comparison: &EvidenceComparison) {
    println!(
        "baseline: {} ({})",
        comparison.baseline.run_id, comparison.baseline.bundle_path
    );
    println!(
        "candidate: {} ({})",
        comparison.candidate.run_id, comparison.candidate.bundle_path
    );
    println!(
        "measurement compatible: {}",
        if comparison.compatible { "yes" } else { "no" }
    );
    if !comparison.compatible {
        println!(
            "incompatible dimensions: {}",
            comparison.incompatibilities.join(", ")
        );
        println!("{}", comparison.interpretation);
        return;
    }

    let differences: Vec<_> = comparison
        .environment
        .iter()
        .filter(|field| {
            !matches!(
                field.relationship,
                benchplane_core::EnvironmentRelationship::Same
            )
        })
        .collect();
    println!(
        "recorded environment differences/unknowns: {}",
        differences.len()
    );
    for field in differences {
        println!(
            "  {}: baseline={} candidate={} ({:?})",
            field.field,
            field.baseline.as_deref().unwrap_or("unknown"),
            field.candidate.as_deref().unwrap_or("unknown"),
            field.relationship
        );
    }
    let requests = comparison
        .requests
        .as_ref()
        .expect("compatible comparison has request metrics");
    println!(
        "measured requests: baseline={} candidate={}",
        requests.baseline_count, requests.candidate_count
    );
    print_metric("request latency mean (µs)", &requests.latency_micros.mean);
    print_metric("request latency p50 (µs)", &requests.latency_micros.p50);
    print_metric("request latency p95 (µs)", &requests.latency_micros.p95);
    print_metric(
        "request TTFT mean (µs)",
        &requests.time_to_first_token_micros.mean,
    );
    print_metric(
        "request TTFT p50 (µs)",
        &requests.time_to_first_token_micros.p50,
    );
    print_metric(
        "request TTFT p95 (µs)",
        &requests.time_to_first_token_micros.p95,
    );

    let repetitions = comparison
        .repetitions
        .as_ref()
        .expect("compatible comparison has repetition metrics");
    println!(
        "measured repetitions: baseline={} candidate={}",
        repetitions.baseline_count, repetitions.candidate_count
    );
    print_metric(
        "repetition aggregate latency mean (µs)",
        &repetitions.aggregate_latency_micros.mean,
    );
    print_metric(
        "repetition aggregate latency p50 (µs)",
        &repetitions.aggregate_latency_micros.p50,
    );
    print_metric(
        "repetition aggregate latency p95 (µs)",
        &repetitions.aggregate_latency_micros.p95,
    );
    print_metric(
        "repetition aggregate TTFT mean (µs)",
        &repetitions.aggregate_time_to_first_token_micros.mean,
    );
    print_metric(
        "repetition aggregate TTFT p50 (µs)",
        &repetitions.aggregate_time_to_first_token_micros.p50,
    );
    print_metric(
        "repetition aggregate TTFT p95 (µs)",
        &repetitions.aggregate_time_to_first_token_micros.p95,
    );
    print_metric(
        "repetition mean throughput (mreq/s)",
        &repetitions.mean_throughput_milli_requests_per_second,
    );

    if let Some(resources) = &comparison.attempt_resources {
        if let Some(cpu) = &resources.cpu_time_micros {
            print_metric("whole-helper CPU time (µs)", cpu);
        } else {
            println!("whole-helper CPU time: not comparable (missing resource evidence)");
        }
        if let Some(rss) = &resources.peak_rss_bytes {
            print_metric("whole-helper peak RSS (bytes)", rss);
        } else {
            println!("whole-helper peak RSS: not comparable (missing resource evidence)");
        }
    }
    println!("{}", comparison.interpretation);
}

fn print_metric(label: &str, metric: &MetricComparison) {
    let percentage = metric
        .delta
        .percentage_delta_milli_percent
        .map(|value| format!("{:.3}%", value as f64 / 1_000.0))
        .unwrap_or_else(|| "undefined".to_owned());
    println!(
        "{label}: baseline={} candidate={} delta={:+} ({percentage})",
        metric.baseline, metric.candidate, metric.delta.absolute_delta
    );
}

fn read_experiment_bytes(path: &PathBuf) -> Result<Vec<u8>, u8> {
    fs::read(path).map_err(|error| {
        eprintln!(
            "error: could not read experiment {}: {error}",
            path.display()
        );
        EXIT_INTERNAL
    })
}

fn validate_command(path: &PathBuf) -> u8 {
    let Ok(bytes) = read_experiment_bytes(path) else {
        return EXIT_INTERNAL;
    };
    let experiment = match parse_experiment(&bytes) {
        Ok(experiment) => experiment,
        Err(error) => return rejected_error(error),
    };
    if let Err(errors) = experiment.validate() {
        for error in errors {
            eprintln!("error: {error}");
        }
        return EXIT_REJECTED;
    }
    println!("valid: {}", experiment.metadata.name);
    EXIT_SUCCESS
}

fn resolve_command(path: &PathBuf) -> u8 {
    let Ok(bytes) = read_experiment_bytes(path) else {
        return EXIT_INTERNAL;
    };
    let experiment = match parse_experiment(&bytes) {
        Ok(experiment) => experiment,
        Err(error) => return rejected_error(error),
    };
    match resolve_experiment(experiment) {
        Ok(resolved) => match serde_json::to_string_pretty(&resolved) {
            Ok(resolved) => {
                println!("{resolved}");
                EXIT_SUCCESS
            }
            Err(error) => internal_error(error),
        },
        Err(ResolutionError::Validation(errors)) => {
            for error in errors {
                eprintln!("error: {error}");
            }
            EXIT_REJECTED
        }
        Err(error) => internal_error(error),
    }
}

fn run_command(path: &PathBuf, output_root: PathBuf, json: bool) -> u8 {
    let Ok(bytes) = read_experiment_bytes(path) else {
        return EXIT_INTERNAL;
    };
    match run_experiment(&bytes, &RunOptions { output_root }) {
        Ok(result) => {
            if json {
                match serde_json::to_string(&result) {
                    Ok(output) => println!("{output}"),
                    Err(error) => return internal_error(error),
                }
            } else {
                print_human_result(&result);
            }
            result_exit_code(&result)
        }
        Err(error) => {
            if let benchplane_core::RunError::Resolution(ResolutionError::Validation(errors)) =
                &error
            {
                for validation_error in errors {
                    eprintln!("error: {validation_error}");
                }
            } else {
                eprintln!("error: {error}");
            }
            if error.is_request_rejection() {
                EXIT_REJECTED
            } else if error.is_finalization_failure() {
                EXIT_FINALIZATION_FAILURE
            } else {
                EXIT_INTERNAL
            }
        }
    }
}

fn print_human_result(result: &RunResult) {
    println!("run ID: {}", result.run_id);
    println!("run state: {}", result.run_state.as_str());
    println!("validity: {}", result.validity_status.as_str());
    println!("attempt count: {}", result.attempt_count);
    println!("sample count: {}", result.sample_count);
    if let Some(latency) = &result.latency {
        println!(
            "repetition-aggregate latency (µs): mean={} p50={} p95={}",
            latency.mean_micros, latency.p50_micros, latency.p95_micros
        );
    }
    if let Some(throughput) = result.mean_throughput_milli_requests_per_second {
        println!("mean throughput (mreq/s): {throughput}");
    }
    if let Some(resources) = result.resources {
        println!("helper CPU time (µs): {}", resources.cpu_time_micros);
        println!("helper peak RSS (bytes): {}", resources.peak_rss_bytes);
    }
    println!("bundle path: {}", result.bundle_path);
    println!("experiment digest: {}", result.experiment_digest);
    println!("resolved-plan digest: {}", result.resolved_plan_digest);
    println!("evidence digest: {}", result.evidence_digest);
}

fn result_exit_code(result: &RunResult) -> u8 {
    match (result.run_state, result.validity_status) {
        (RunState::Succeeded, ValidityStatus::Valid) => EXIT_SUCCESS,
        (RunState::Succeeded, ValidityStatus::Invalid | ValidityStatus::Indeterminate) => {
            EXIT_INVALID
        }
        (RunState::Failed | RunState::Interrupted, _) => EXIT_OPERATIONAL_FAILURE,
        _ => EXIT_INTERNAL,
    }
}

fn rejected_error(error: impl std::fmt::Display) -> u8 {
    eprintln!("error: {error}");
    EXIT_REJECTED
}

fn internal_error(error: impl std::fmt::Display) -> u8 {
    eprintln!("error: {error}");
    EXIT_INTERNAL
}
