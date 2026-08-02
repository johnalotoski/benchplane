// SPDX-License-Identifier: Apache-2.0

mod cpu_probe;
pub mod evidence;
mod execution;
mod lifecycle;
mod local_fake;
mod parsing;
mod resolution;
mod run;

pub use benchplane_schema::{EvidenceManifest, RunResult};
pub use evidence::{verify_evidence_bundle, EvidenceError};
pub use lifecycle::{Lifecycle, LifecycleError};
pub use parsing::{parse_experiment, ParseError};
pub use resolution::{resolve_experiment, ResolutionError};
pub use run::{run_experiment, RunError, RunOptions};
