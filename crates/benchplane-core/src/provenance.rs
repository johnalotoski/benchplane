// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{
    AttemptProvenance, BackendProvenance, CpuProvenance, DeviceClass, KernelProvenance,
    LlamaCppTarget, ModelProvenance, NvidiaGpuProvenance, OperatingSystemProvenance,
    PlatformProvenance, ResolvedExperiment, RuntimeProvenance, RuntimeSpec,
    SoftwareComponentProvenance, SoftwareProvenance, ATTEMPT_PROVENANCE_FORMAT_V1,
    BENCHPLANE_SOFTWARE_NAME, CPU_PROBE_GENERATOR_VERSION, LLAMA_CPP_BACKEND_IDENTITY,
    LLAMA_CPP_CUDA_BACKEND_IDENTITY, LLAMA_CPP_ENGINE_NAME, LLAMA_CPP_ENGINE_VERSION,
    LLAMA_CPP_GENERATOR_VERSION, LLAMA_CPP_MODEL_SHA256, LOCAL_FAKE_GENERATOR_VERSION,
};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

const MAX_OS_RELEASE_BYTES: u64 = 16 * 1024;
// Preserve the former inspection range as a prefix without making discovery
// depend on the size of the complete per-CPU file.
const MAX_CPUINFO_SCAN_BYTES: u64 = 256 * 1024;
const MAX_KERNEL_FILE_BYTES: u64 = 4 * 1024;
const MAX_PLATFORM_VALUE_BYTES: usize = 256;
const MAX_NIX_STORE_PATH_BYTES: usize = 512;
const MIN_NVIDIA_DRIVER_VERSION: (u32, u32, u32) = (575, 57, 8);

pub(crate) fn nvidia_driver_version_supported(value: &str) -> bool {
    let mut components = value.split('.');
    let Some(major) = components
        .next()
        .and_then(|component| component.parse::<u32>().ok())
    else {
        return false;
    };
    let Some(minor) = components
        .next()
        .and_then(|component| component.parse::<u32>().ok())
    else {
        return false;
    };
    let patch = match components.next() {
        Some(component) => match component.parse::<u32>() {
            Ok(component) => component,
            Err(_) => return false,
        },
        None => 0,
    };
    components.next().is_none() && (major, minor, patch) >= MIN_NVIDIA_DRIVER_VERSION
}

pub(crate) fn capture(run_id: &str, plan: &ResolvedExperiment) -> AttemptProvenance {
    let runtime = match &plan.experiment.spec.runtime {
        RuntimeSpec::LocalFake { .. } => RuntimeProvenance::LocalFake {
            generator: LOCAL_FAKE_GENERATOR_VERSION.to_owned(),
        },
        RuntimeSpec::CpuProbe { .. } => RuntimeProvenance::CpuProbe {
            generator: CPU_PROBE_GENERATOR_VERSION.to_owned(),
        },
        RuntimeSpec::LlamaCpp { target, model, .. } => RuntimeProvenance::LlamaCpp {
            generator: LLAMA_CPP_GENERATOR_VERSION.to_owned(),
            engine: SoftwareComponentProvenance {
                name: LLAMA_CPP_ENGINE_NAME.to_owned(),
                version: LLAMA_CPP_ENGINE_VERSION.to_owned(),
                nix_store_path: compiled_nix_store_path(match target {
                    LlamaCppTarget::Cpu => option_env!("BENCHPLANE_LLAMA_CPP_NIX_STORE_PATH"),
                    LlamaCppTarget::NvidiaCuda => {
                        option_env!("BENCHPLANE_LLAMA_CPP_CUDA_NIX_STORE_PATH")
                    }
                }),
            },
            model: ModelProvenance {
                identity: model.clone(),
                sha256: LLAMA_CPP_MODEL_SHA256.to_owned(),
                nix_store_path: compiled_nix_store_path(option_env!(
                    "BENCHPLANE_SMOLLM2_NIX_STORE_PATH"
                )),
            },
            backend: Box::new(BackendProvenance {
                identity: match target {
                    LlamaCppTarget::Cpu => LLAMA_CPP_BACKEND_IDENTITY,
                    LlamaCppTarget::NvidiaCuda => LLAMA_CPP_CUDA_BACKEND_IDENTITY,
                }
                .to_owned(),
                device_class: match target {
                    LlamaCppTarget::Cpu => DeviceClass::Cpu,
                    LlamaCppTarget::NvidiaCuda => DeviceClass::NvidiaCuda,
                },
                nix_store_path: compiled_nix_store_path(match target {
                    LlamaCppTarget::Cpu => option_env!("BENCHPLANE_LLAMA_CPP_NIX_STORE_PATH"),
                    LlamaCppTarget::NvidiaCuda => {
                        option_env!("BENCHPLANE_LLAMA_CPP_CUDA_NIX_STORE_PATH")
                    }
                }),
                nvidia: None,
            }),
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
    let cpu_model = read_cpu_model(Path::new("/proc/cpuinfo"));
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

pub(crate) fn attach_nvidia(
    provenance: &mut AttemptProvenance,
    nvidia: NvidiaGpuProvenance,
) -> Result<(), &'static str> {
    match &mut provenance.software.runtime {
        RuntimeProvenance::LlamaCpp { backend, .. }
            if backend.device_class == DeviceClass::NvidiaCuda && backend.nvidia.is_none() =>
        {
            backend.nvidia = Some(Box::new(nvidia));
            Ok(())
        }
        _ => Err("NVIDIA execution provenance does not match the prepared runtime"),
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

fn read_cpu_model(path: &Path) -> Option<String> {
    cpu_model_from_reader(File::open(path).ok()?)
}

fn cpu_model_from_reader(reader: impl Read) -> Option<String> {
    let mut reader = BufReader::new(reader.take(MAX_CPUINFO_SCAN_BYTES + 1));
    let mut line = Vec::new();
    let mut scanned_bytes = 0_u64;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).ok()?;
        if read == 0 {
            return None;
        }
        scanned_bytes = scanned_bytes.checked_add(read as u64)?;
        if scanned_bytes > MAX_CPUINFO_SCAN_BYTES {
            return None;
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            continue;
        };
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if ["model name", "Processor", "Hardware"].contains(&key.trim()) {
            return bounded_value(value);
        }
    }
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
    use std::io::{Cursor, Error};

    #[test]
    fn nvidia_driver_version_has_a_strict_supported_boundary() {
        for version in ["575.57.08", "575.57.9", "595.84"] {
            assert!(nvidia_driver_version_supported(version), "{version}");
        }
        for version in [
            "575.57",
            "575.57.07",
            "575",
            "575.57.08.1",
            "not-a-version",
            "4294967296.0.0",
            " 575.57.08",
        ] {
            assert!(!nvidia_driver_version_supported(version), "{version}");
        }
    }

    struct ErrorAfterChunk(Cursor<Vec<u8>>);

    impl Read for ErrorAfterChunk {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.0.read(buffer)?;
            if read == 0 {
                Err(Error::other("trailing read was attempted"))
            } else {
                Ok(read)
            }
        }
    }

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
    fn cpu_scan_recognizes_existing_keys_and_stops_at_the_first_match() {
        for key in ["model name", "Processor", "Hardware"] {
            let input = format!("{key} : Example CPU\n").into_bytes();
            assert_eq!(
                cpu_model_from_reader(ErrorAfterChunk(Cursor::new(input))).as_deref(),
                Some("Example CPU")
            );
        }
    }

    #[test]
    fn cpu_scan_finds_an_early_model_in_input_larger_than_the_old_limit() {
        let mut input = b"processor : 0\nmodel name : Large Host CPU\n".to_vec();
        input.resize(256 * 1024 + 1, b'x');
        assert_eq!(
            cpu_model_from_reader(Cursor::new(input)).as_deref(),
            Some("Large Host CPU")
        );
    }

    #[test]
    fn cpu_scan_returns_none_without_an_accepted_field() {
        let input = b"processor : 0\nSerial : secret-serial\nflags : private details\n";
        assert!(cpu_model_from_reader(Cursor::new(input)).is_none());
    }

    #[test]
    fn cpu_scan_does_not_search_past_the_prefix_bound() {
        let mut input = vec![b'x'; MAX_CPUINFO_SCAN_BYTES as usize + 1];
        input.extend_from_slice(b"\nmodel name : Too Late CPU\n");
        assert!(cpu_model_from_reader(Cursor::new(input)).is_none());
    }

    #[test]
    fn cpu_scan_preserves_the_model_value_bound() {
        let input = format!(
            "model name : {}\n",
            "x".repeat(MAX_PLATFORM_VALUE_BYTES + 1)
        );
        assert!(cpu_model_from_reader(Cursor::new(input)).is_none());
    }

    #[test]
    fn cpu_scan_skips_malformed_and_non_utf8_lines_without_panicking() {
        let input = b"unexpected line\nmodel name without colon\ninvalid: \xff\xfe\nHardware : Example ARM CPU\n";
        assert_eq!(
            cpu_model_from_reader(Cursor::new(input)).as_deref(),
            Some("Example ARM CPU")
        );
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
