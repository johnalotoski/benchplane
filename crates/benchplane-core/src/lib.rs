// SPDX-License-Identifier: Apache-2.0

pub mod evidence;
mod lifecycle;
mod local_fake;
mod parsing;
mod resolution;

pub use benchplane_schema::{EvidenceManifest, RunResult};
pub use evidence::{verify_evidence_bundle, EvidenceError};
pub use lifecycle::{Lifecycle, LifecycleError};
pub use parsing::{parse_experiment, ParseError};
pub use resolution::{resolve_experiment, ResolutionError};
