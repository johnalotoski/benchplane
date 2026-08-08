// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{
    AttemptProvenance, BackendProvenance, CpuProvenance, DeviceClass, KernelProvenance,
    ModelProvenance, OperatingSystemProvenance, PlatformProvenance, ResolvedExperiment,
    RuntimeProvenance, RuntimeSpec, SoftwareComponentProvenance, SoftwareProvenance,
    ATTEMPT_PROVENANCE_FORMAT_V1, BENCHPLANE_SOFTWARE_NAME, CPU_PROBE_GENERATOR_VERSION,
    LLAMA_CPP_BACKEND_IDENTITY, LLAMA_CPP_ENGINE_NAME, LLAMA_CPP_ENGINE_VERSION,
    LLAMA_CPP_GENERATOR_VERSION, LLAMA_CPP_MODEL_SHA256, LOCAL_FAKE_GENERATOR_VERSION,
};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_OS_RELEASE_BYTES: u64 = 16 * 1024;
const MAX_CPUINFO_BYTES: u64 = 256 * 1024;
const MAX_KERNEL_FILE_BYTES: u64 = 4 * 1024;
const MAX_PLATFORM_VALUE_BYTES: usize = 256;
const MAX_NIX_STORE_PATH_BYTES: usize = 512;

pub(crate) fn capture(run_id: &str, plan: &ResolvedExperiment) -> AttemptProvenance {
    let runtime = match &plan.experiment.spec.runtime {
        RuntimeSpec::LocalFake { .. } => RuntimeProvenance::LocalFake {
            generator: LOCAL_FAKE_GENERATOR_VERSION.to_owned(),
        },
        RuntimeSpec::CpuProbe { .. } => RuntimeProvenance::CpuProbe {
            generator: CPU_PROBE_GENERATOR_VERSION.to_owned(),
        },
        RuntimeSpec::LlamaCpp { model, .. } => RuntimeProvenance::LlamaCpp {
            generator: LLAMA_CPP_GENERATOR_VERSION.to_owned(),
            engine: SoftwareComponentProvenance {
                name: LLAMA_CPP_ENGINE_NAME.to_owned(),
                version: LLAMA_CPP_ENGINE_VERSION.to_owned(),
                nix_store_path: compiled_nix_store_path(option_env!(
                    "BENCHPLANE_LLAMA_CPP_NIX_STORE_PATH"
                )),
            },
            model: ModelProvenance {
                identity: model.clone(),
                sha256: LLAMA_CPP_MODEL_SHA256.to_owned(),
                nix_store_path: compiled_nix_store_path(option_env!(
                    "BENCHPLANE_SMOLLM2_NIX_STORE_PATH"
                )),
            },
            backend: BackendProvenance {
                identity: LLAMA_CPP_BACKEND_IDENTITY.to_owned(),
                device_class: DeviceClass::Cpu,
                nix_store_path: compiled_nix_store_path(option_env!(
                    "BENCHPLANE_LLAMA_CPP_NIX_STORE_PATH"
                )),
            },
        },
        RuntimeSpec::Vllm { .. } => {
            unreachable!("unsupported runtime is rejected before provenance capture")
        }
    };

    let os_release = read_bounded(Path::new("/etc/os-release"), MAX_OS_RELEASE_BYTES);
    let (distribution, version) = os_release
        .as_deref()
        .map(os_release_identity)
        .unwrap_or_default();
    let operating_system = OperatingSystemProvenance {
        family: std::env::consts::OS.to_owned(),
        distribution,
        version,
    };
    let kernel_name =
        read_single_value(Path::new("/proc/sys/kernel/ostype"), MAX_KERNEL_FILE_BYTES)
            .unwrap_or_else(|| std::env::consts::OS.to_owned());
    let kernel_release = read_single_value(
        Path::new("/proc/sys/kernel/osrelease"),
        MAX_KERNEL_FILE_BYTES,
    );
    let cpu_model = read_bounded(Path::new("/proc/cpuinfo"), MAX_CPUINFO_BYTES)
        .as_deref()
        .and_then(cpu_model_value);
    let logical_cpu_count = std::thread::available_parallelism()
        .ok()
        .and_then(|count| u32::try_from(count.get()).ok());

    AttemptProvenance {
        format: ATTEMPT_PROVENANCE_FORMAT_V1.to_owned(),
        run_id: run_id.to_owned(),
        attempt_number: 1,
        platform: PlatformProvenance {
            operating_system,
            kernel: KernelProvenance {
                name: kernel_name,
                release: kernel_release,
            },
            architecture: std::env::consts::ARCH.to_owned(),
            cpu: CpuProvenance {
                model: cpu_model,
                logical_cpu_count,
            },
        },
        software: SoftwareProvenance {
            benchplane: SoftwareComponentProvenance {
                name: BENCHPLANE_SOFTWARE_NAME.to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                nix_store_path: current_executable_nix_store_path(),
            },
            runtime,
        },
    }
}

fn read_bounded(path: &Path, limit: u64) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn read_single_value(path: &Path, limit: u64) -> Option<String> {
    read_bounded(path, limit).and_then(|value| bounded_value(&value))
}

fn release_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        if candidate != key {
            return None;
        }
        let trimmed = value.trim();
        let unquoted = if trimmed.len() >= 2
            && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
                || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
        {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };
        bounded_value(unquoted)
    })
}

fn os_release_identity(contents: &str) -> (Option<String>, Option<String>) {
    (
        release_value(contents, "ID"),
        release_value(contents, "VERSION_ID"),
    )
}

fn cpu_model_value(contents: &str) -> Option<String> {
    for accepted_key in ["model name", "Processor", "Hardware"] {
        if let Some(value) = contents.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == accepted_key)
                .then(|| bounded_value(value))
                .flatten()
        }) {
            return Some(value);
        }
    }
    None
}

fn bounded_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_PLATFORM_VALUE_BYTES
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value.to_owned())
}

fn compiled_nix_store_path(value: Option<&'static str>) -> Option<String> {
    value.and_then(|path| nix_store_object(Path::new(path)))
}

fn current_executable_nix_store_path() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .and_then(|path| nix_store_object(&path))
}

fn nix_store_object(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let rest = text.strip_prefix("/nix/store/")?;
    let entry = rest.split('/').next()?;
    if entry.is_empty()
        || entry.len() + "/nix/store/".len() > MAX_NIX_STORE_PATH_BYTES
        || entry.chars().any(|character| character.is_control())
    {
        return None;
    }
    Some(
        PathBuf::from("/nix/store")
            .join(entry)
            .display()
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_capture_is_explicitly_allowlisted_and_bounded() {
        let release = "ID=nixos\nVERSION_ID=\"26.05\"\nHOSTNAME=private-host\nTOKEN=secret\n";
        let identity = os_release_identity(release);
        assert_eq!(identity.0.as_deref(), Some("nixos"));
        assert_eq!(identity.1.as_deref(), Some("26.05"));
        assert!(!format!("{identity:?}").contains("private-host"));
        assert!(!format!("{identity:?}").contains("secret"));
        assert!(bounded_value(&"x".repeat(MAX_PLATFORM_VALUE_BYTES + 1)).is_none());
    }

    #[test]
    fn cpu_capture_ignores_serial_and_uses_a_model_class_field() {
        let cpuinfo = "processor : 0\nSerial : secret-serial\nmodel name : Example CPU 1\n";
        assert_eq!(cpu_model_value(cpuinfo).as_deref(), Some("Example CPU 1"));
        assert!(cpu_model_value("Serial : secret-serial\n").is_none());
    }

    #[test]
    fn nix_identity_is_reduced_to_one_store_object() {
        assert_eq!(
            nix_store_object(Path::new(
                "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-benchplane-0.1.0/bin/benchplane"
            ))
            .as_deref(),
            Some("/nix/store/0123456789abcdfghijklmnpqrsvwxyz-benchplane-0.1.0")
        );
        assert!(nix_store_object(Path::new("/usr/bin/benchplane")).is_none());
    }
}
