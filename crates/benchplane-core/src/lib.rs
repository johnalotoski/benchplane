// SPDX-License-Identifier: Apache-2.0

pub mod evidence;

pub use benchplane_schema::EvidenceManifest;
pub use evidence::{verify_evidence_bundle, EvidenceError};

use benchplane_schema::{Experiment, ResolvedExperiment, ValidationError, API_VERSION};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResolutionError {
    #[error("experiment validation failed")]
    Validation(Vec<ValidationError>),
    #[error("failed to deterministically serialize the typed experiment: {0}")]
    DeterministicSerialization(serde_json::Error),
}

pub fn resolve_experiment(experiment: Experiment) -> Result<ResolvedExperiment, ResolutionError> {
    experiment.validate().map_err(ResolutionError::Validation)?;

    let bytes =
        serde_json::to_vec(&experiment).map_err(ResolutionError::DeterministicSerialization)?;
    let digest = Sha256::digest(bytes);

    Ok(ResolvedExperiment {
        api_version: API_VERSION.to_owned(),
        kind: "ResolvedExperiment".to_owned(),
        experiment,
        experiment_digest: format!("sha256:{}", hex::encode(digest)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn experiment() -> Experiment {
        serde_json::from_value(serde_json::json!({
            "apiVersion": "benchplane/v1alpha1",
            "kind": "Experiment",
            "metadata": { "name": "deterministic" },
            "spec": {
                "provider": { "kind": "localFake" },
                "runtime": { "kind": "localFake" },
                "workload": { "profile": "smoke", "requests": 1 },
                "budget": { "maximumCostUsd": 0.01 }
            }
        }))
        .expect("test experiment should deserialize")
    }

    #[test]
    fn resolution_is_deterministic_and_contains_defaults() {
        let first = resolve_experiment(experiment()).expect("first resolution should succeed");
        let second = resolve_experiment(experiment()).expect("second resolution should succeed");

        assert_eq!(first, second);
        assert_eq!(first.experiment.spec.workload.concurrency, 1);
        assert_eq!(first.experiment.spec.measurement.repetitions, 3);
        assert_eq!(
            first.experiment.spec.lifecycle.maximum_runtime_seconds,
            3600
        );
        assert!(first.experiment_digest.starts_with("sha256:"));
        assert_eq!(first.experiment_digest.len(), 71);
    }

    #[test]
    fn resolution_rejects_invalid_experiments() {
        let mut invalid = experiment();
        invalid.spec.workload.requests = 0;

        assert!(matches!(
            resolve_experiment(invalid),
            Err(ResolutionError::Validation(errors))
                if errors == vec![ValidationError::NoRequests]
        ));
    }

    #[test]
    fn equivalent_yaml_formatting_and_mapping_order_have_the_same_digest() {
        let first: Experiment = serde_yaml::from_str(
            r#"
apiVersion: benchplane/v1alpha1
kind: Experiment
metadata:
  name: mapping-order
  labels:
    zed: last
    alpha: first
spec:
  provider:
    kind: localFake
  runtime:
    kind: localFake
  workload:
    profile: smoke
    requests: 1
  budget:
    maximumCostUsd: 0
"#,
        )
        .expect("first YAML should deserialize");
        let second: Experiment = serde_yaml::from_str(
            r#"
kind: Experiment
metadata: { labels: { alpha: first, zed: last }, name: mapping-order }
spec:
  budget: { maximumCostUsd: 0.0 }
  workload: { requests: 1, profile: smoke }
  runtime: { kind: localFake }
  provider: { kind: localFake }
apiVersion: benchplane/v1alpha1
"#,
        )
        .expect("second YAML should deserialize");

        let first = resolve_experiment(first).expect("first experiment should resolve");
        let second = resolve_experiment(second).expect("second experiment should resolve");
        assert_eq!(first.experiment, second.experiment);
        assert_eq!(first.experiment_digest, second.experiment_digest);
    }
}
