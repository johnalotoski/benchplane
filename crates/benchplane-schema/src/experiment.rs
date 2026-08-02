// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const API_VERSION: &str = "benchplane/v1alpha1";
pub const EXPERIMENT_KIND: &str = "Experiment";
pub const EVIDENCE_FORMAT_V1: &str = "benchplane-evidence/v1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Experiment {
    pub api_version: String,
    pub kind: String,
    pub metadata: Metadata,
    pub spec: ExperimentSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExperimentSpec {
    pub provider: ProviderSpec,
    pub runtime: RuntimeSpec,
    pub workload: WorkloadSpec,
    #[serde(default)]
    pub measurement: MeasurementSpec,
    pub budget: BudgetSpec,
    #[serde(default)]
    pub lifecycle: LifecycleSpec,
    #[serde(default)]
    pub artifacts: ArtifactSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProviderSpec {
    LocalFake,
    Aws {
        region: String,
        #[serde(default)]
        capacity: AwsCapacity,
        #[schemars(rename = "instanceTypes")]
        instance_types: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AwsCapacity {
    #[default]
    Spot,
    OnDemand,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RuntimeSpec {
    LocalFake,
    Vllm {
        model: String,
        revision: String,
        #[serde(default)]
        arguments: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkloadSpec {
    pub profile: String,
    pub requests: u32,
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
}

fn default_concurrency() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasurementSpec {
    #[serde(default = "default_warmups")]
    pub warmup_runs: u32,
    #[serde(default = "default_repetitions")]
    pub repetitions: u32,
}

impl Default for MeasurementSpec {
    fn default() -> Self {
        Self {
            warmup_runs: default_warmups(),
            repetitions: default_repetitions(),
        }
    }
}

fn default_warmups() -> u32 {
    1
}

fn default_repetitions() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BudgetSpec {
    pub maximum_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleSpec {
    #[serde(default = "default_runtime_seconds")]
    pub maximum_runtime_seconds: u64,
    #[serde(default = "default_true")]
    pub destroy_on_completion: bool,
}

impl Default for LifecycleSpec {
    fn default() -> Self {
        Self {
            maximum_runtime_seconds: default_runtime_seconds(),
            destroy_on_completion: true,
        }
    }
}

fn default_runtime_seconds() -> u64 {
    3600
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactSpec {
    #[serde(default)]
    pub public: bool,
    #[serde(default = "default_artifact_format")]
    pub format: String,
}

impl Default for ArtifactSpec {
    fn default() -> Self {
        Self {
            public: false,
            format: default_artifact_format(),
        }
    }
}

fn default_artifact_format() -> String {
    EVIDENCE_FORMAT_V1.to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedExperiment {
    pub api_version: String,
    pub kind: String,
    pub experiment: Experiment,
    pub experiment_digest: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("unsupported apiVersion: {0}")]
    ApiVersion(String),
    #[error("unsupported kind: {0}")]
    Kind(String),
    #[error("metadata.name must not be empty")]
    EmptyName,
    #[error("workload.profile must not be empty")]
    EmptyWorkloadProfile,
    #[error("workload.requests must be greater than zero")]
    NoRequests,
    #[error("workload.concurrency must be greater than zero")]
    NoConcurrency,
    #[error("measurement.repetitions must be greater than zero")]
    NoRepetitions,
    #[error("budget.maximumCostUsd must be finite and nonnegative for localFake experiments")]
    InvalidLocalBudget,
    #[error("budget.maximumCostUsd must be finite and greater than zero for AWS experiments")]
    InvalidAwsBudget,
    #[error("lifecycle.maximumRuntimeSeconds must be between 1 and 86400")]
    InvalidRuntime,
    #[error("AWS provider requires at least one instance type")]
    MissingInstanceTypes,
    #[error("AWS provider region must not be empty")]
    EmptyAwsRegion,
    #[error("AWS provider instance types must not contain empty values")]
    EmptyAwsInstanceType,
    #[error("vLLM model and revision must not be empty")]
    InvalidVllmIdentity,
    #[error("unsupported artifact format: {0}")]
    UnsupportedArtifactFormat(String),
}

impl Experiment {
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if self.api_version != API_VERSION {
            errors.push(ValidationError::ApiVersion(self.api_version.clone()));
        }
        if self.kind != EXPERIMENT_KIND {
            errors.push(ValidationError::Kind(self.kind.clone()));
        }
        if self.metadata.name.trim().is_empty() {
            errors.push(ValidationError::EmptyName);
        }
        if self.spec.workload.profile.trim().is_empty() {
            errors.push(ValidationError::EmptyWorkloadProfile);
        }
        if self.spec.workload.requests == 0 {
            errors.push(ValidationError::NoRequests);
        }
        if self.spec.workload.concurrency == 0 {
            errors.push(ValidationError::NoConcurrency);
        }
        if self.spec.measurement.repetitions == 0 {
            errors.push(ValidationError::NoRepetitions);
        }
        if !(1..=86400).contains(&self.spec.lifecycle.maximum_runtime_seconds) {
            errors.push(ValidationError::InvalidRuntime);
        }

        match &self.spec.provider {
            ProviderSpec::LocalFake => {
                if !self.spec.budget.maximum_cost_usd.is_finite()
                    || self.spec.budget.maximum_cost_usd < 0.0
                {
                    errors.push(ValidationError::InvalidLocalBudget);
                }
            }
            ProviderSpec::Aws {
                region,
                instance_types,
                ..
            } => {
                if !self.spec.budget.maximum_cost_usd.is_finite()
                    || self.spec.budget.maximum_cost_usd <= 0.0
                {
                    errors.push(ValidationError::InvalidAwsBudget);
                }
                if region.trim().is_empty() {
                    errors.push(ValidationError::EmptyAwsRegion);
                }
                if instance_types.is_empty() {
                    errors.push(ValidationError::MissingInstanceTypes);
                } else if instance_types.iter().any(|value| value.trim().is_empty()) {
                    errors.push(ValidationError::EmptyAwsInstanceType);
                }
            }
        }

        if let RuntimeSpec::Vllm {
            model, revision, ..
        } = &self.spec.runtime
        {
            if model.trim().is_empty() || revision.trim().is_empty() {
                errors.push(ValidationError::InvalidVllmIdentity);
            }
        }

        if self.spec.artifacts.format != EVIDENCE_FORMAT_V1 {
            errors.push(ValidationError::UnsupportedArtifactFormat(
                self.spec.artifacts.format.clone(),
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_experiment() -> Experiment {
        Experiment {
            api_version: API_VERSION.to_owned(),
            kind: EXPERIMENT_KIND.to_owned(),
            metadata: Metadata {
                name: "local-smoke".to_owned(),
                labels: BTreeMap::new(),
            },
            spec: ExperimentSpec {
                provider: ProviderSpec::LocalFake,
                runtime: RuntimeSpec::LocalFake,
                workload: WorkloadSpec {
                    profile: "deterministic-smoke".to_owned(),
                    requests: 10,
                    concurrency: 1,
                },
                measurement: MeasurementSpec::default(),
                budget: BudgetSpec {
                    maximum_cost_usd: 0.01,
                },
                lifecycle: LifecycleSpec::default(),
                artifacts: ArtifactSpec::default(),
            },
        }
    }

    #[test]
    fn valid_document_passes_validation() {
        assert_eq!(valid_experiment().validate(), Ok(()));
    }

    #[test]
    fn zero_cost_is_valid_for_local_fake() {
        let mut experiment = valid_experiment();
        experiment.spec.budget.maximum_cost_usd = 0.0;

        assert_eq!(experiment.validate(), Ok(()));
    }

    #[test]
    fn zero_cost_is_invalid_for_aws() {
        let mut experiment = valid_experiment();
        experiment.spec.provider = ProviderSpec::Aws {
            region: "us-east-2".to_owned(),
            capacity: AwsCapacity::Spot,
            instance_types: vec!["g6.xlarge".to_owned()],
        };
        experiment.spec.budget.maximum_cost_usd = 0.0;

        assert_eq!(
            experiment.validate(),
            Err(vec![ValidationError::InvalidAwsBudget])
        );
    }

    #[test]
    fn unsupported_artifact_format_is_invalid() {
        let mut experiment = valid_experiment();
        experiment.spec.artifacts.format = "benchplane-evidence/v2".to_owned();

        assert_eq!(
            experiment.validate(),
            Err(vec![ValidationError::UnsupportedArtifactFormat(
                "benchplane-evidence/v2".to_owned()
            )])
        );
    }

    #[test]
    fn obvious_empty_strings_are_invalid() {
        let mut experiment = valid_experiment();
        experiment.spec.provider = ProviderSpec::Aws {
            region: " ".to_owned(),
            capacity: AwsCapacity::Spot,
            instance_types: vec![String::new()],
        };
        experiment.spec.workload.profile = " ".to_owned();

        assert_eq!(
            experiment.validate(),
            Err(vec![
                ValidationError::EmptyWorkloadProfile,
                ValidationError::EmptyAwsRegion,
                ValidationError::EmptyAwsInstanceType,
            ])
        );
    }

    #[test]
    fn invalid_values_are_reported_together() {
        let mut experiment = valid_experiment();
        experiment.spec.workload.requests = 0;
        experiment.spec.budget.maximum_cost_usd = -0.01;
        experiment.spec.lifecycle.maximum_runtime_seconds = 86_401;

        assert_eq!(
            experiment.validate(),
            Err(vec![
                ValidationError::NoRequests,
                ValidationError::InvalidRuntime,
                ValidationError::InvalidLocalBudget,
            ])
        );
    }
}
