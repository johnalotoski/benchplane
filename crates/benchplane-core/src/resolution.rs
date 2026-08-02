// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{Experiment, ResolvedExperiment, ValidationError, API_VERSION};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResolutionError {
    #[error("experiment validation failed")]
    Validation(Vec<ValidationError>),
    #[error("failed to deterministically serialize the typed experiment: {0}")]
    DeterministicSerialization(serde_json::Error),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedPlanContent<'a> {
    api_version: &'a str,
    kind: &'a str,
    experiment: &'a Experiment,
    experiment_digest: &'a str,
}

pub fn resolve_experiment(experiment: Experiment) -> Result<ResolvedExperiment, ResolutionError> {
    experiment.validate().map_err(ResolutionError::Validation)?;

    let experiment_bytes =
        serde_json::to_vec(&experiment).map_err(ResolutionError::DeterministicSerialization)?;
    let experiment_digest = sha256_digest(&experiment_bytes);
    let content = ResolvedPlanContent {
        api_version: API_VERSION,
        kind: "ResolvedExperiment",
        experiment: &experiment,
        experiment_digest: &experiment_digest,
    };
    let plan_bytes =
        serde_json::to_vec(&content).map_err(ResolutionError::DeterministicSerialization)?;
    let resolved_plan_digest = sha256_digest(&plan_bytes);

    Ok(ResolvedExperiment {
        api_version: API_VERSION.to_owned(),
        kind: "ResolvedExperiment".to_owned(),
        experiment,
        experiment_digest,
        resolved_plan_digest,
    })
}

pub(crate) fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use benchplane_schema::{LocalFakeScenario, RuntimeSpec};

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
        assert_eq!(
            first.experiment.spec.runtime,
            RuntimeSpec::LocalFake {
                seed: 0,
                scenario: LocalFakeScenario::Success,
            }
        );
        assert_eq!(first.experiment_digest.len(), 71);
        assert_eq!(first.resolved_plan_digest.len(), 71);
        assert_ne!(first.experiment_digest, first.resolved_plan_digest);
    }

    #[test]
    fn plan_digest_excludes_its_own_field() {
        let resolved = resolve_experiment(experiment()).expect("resolution should succeed");
        let content = ResolvedPlanContent {
            api_version: &resolved.api_version,
            kind: &resolved.kind,
            experiment: &resolved.experiment,
            experiment_digest: &resolved.experiment_digest,
        };
        let bytes = serde_json::to_vec(&content).expect("serialize plan content");
        assert_eq!(resolved.resolved_plan_digest, sha256_digest(&bytes));
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
    fn equivalent_yaml_formatting_and_mapping_order_have_the_same_digests() {
        let first: Experiment = serde_saphyr::from_str(
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
        let second: Experiment = serde_saphyr::from_str(
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
        assert_eq!(first.resolved_plan_digest, second.resolved_plan_digest);
    }
}
