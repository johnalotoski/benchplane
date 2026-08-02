// SPDX-License-Identifier: Apache-2.0

use benchplane_core::{
    parse_experiment, resolve_experiment, run_experiment, verify_evidence_bundle, ResolutionError,
    RunOptions,
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
    /// Verify an evidence bundle manifest and checksums.
    Verify { bundle: PathBuf },
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
    }
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
            "latency (µs): mean={} p50={} p95={}",
            latency.mean_micros, latency.p50_micros, latency.p95_micros
        );
    }
    if let Some(throughput) = result.mean_throughput_milli_requests_per_second {
        println!("mean throughput (mreq/s): {throughput}");
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
