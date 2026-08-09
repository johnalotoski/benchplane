// SPDX-License-Identifier: Apache-2.0

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
compile_error!(
    "benchplane-core supervised helper execution requires Linux wait4 resource accounting; supported systems are x86_64-linux and aarch64-linux"
);

mod child_supervisor;
mod cpu_probe;
pub mod evidence;
mod execution;
mod lifecycle;
mod llama_cpp;
mod local_fake;
mod parsing;
mod provenance;
mod resolution;
mod run;

pub use benchplane_schema::{EvidenceManifest, RunResult};
pub use evidence::{verify_evidence_bundle, EvidenceError};
pub use lifecycle::{Lifecycle, LifecycleError};
pub use parsing::{parse_experiment, ParseError};
pub use resolution::{resolve_experiment, ResolutionError};
pub use run::{run_experiment, RunError, RunOptions};
