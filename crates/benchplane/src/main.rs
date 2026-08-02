// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};
use benchplane_core::{resolve_experiment, verify_evidence_bundle, ResolutionError};
use benchplane_schema::Experiment;
use clap::{Parser, Subcommand};
use schemars::schema_for;
use std::{fs, path::PathBuf};

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

fn read_experiment(path: &PathBuf) -> Result<Experiment> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("could not read experiment {}", path.display()))?;
    serde_yaml::from_str(&source)
        .with_context(|| format!("could not parse experiment YAML {}", path.display()))
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { experiment } => {
            let experiment = read_experiment(&experiment)?;
            if let Err(errors) = experiment.validate() {
                for error in errors {
                    eprintln!("error: {error}");
                }
                bail!("experiment validation failed");
            }
            println!("valid: {}", experiment.metadata.name);
        }
        Command::Resolve { experiment } => {
            let experiment = read_experiment(&experiment)?;
            let resolved = resolve_experiment(experiment).inspect_err(|error| {
                if let ResolutionError::Validation(errors) = error {
                    for validation_error in errors {
                        eprintln!("error: {validation_error}");
                    }
                }
            })?;
            println!("{}", serde_json::to_string_pretty(&resolved)?);
        }
        Command::Schema {
            command: SchemaCommand::Export,
        } => {
            let schema = schema_for!(Experiment);
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
        Command::Evidence {
            command: EvidenceCommand::Verify { bundle },
        } => {
            let manifest = verify_evidence_bundle(&bundle)?;
            println!(
                "verified: run={} status={} experiment={}",
                manifest.run_id, manifest.status, manifest.experiment_digest
            );
        }
    }

    Ok(())
}
