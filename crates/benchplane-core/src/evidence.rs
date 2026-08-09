// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{
    AttemptProvenance, AttemptRecord, AttemptResources, AttemptStatus, DeviceClass,
    EvidenceManifest, LifecycleEvent, RunRecord, RunState, RunSummary, RuntimeProvenance,
    ValidityResult, ATTEMPT_PROVENANCE_FORMAT_V1, ATTEMPT_RESOURCES_FORMAT_V1,
    BENCHPLANE_SOFTWARE_NAME, CPU_PROBE_GENERATOR_VERSION, EVIDENCE_FORMAT_V1,
    LLAMA_CPP_BACKEND_IDENTITY, LLAMA_CPP_ENGINE_NAME, LLAMA_CPP_ENGINE_VERSION,
    LLAMA_CPP_GENERATOR_VERSION, LLAMA_CPP_MODEL_IDENTITY, LLAMA_CPP_MODEL_SHA256,
    LOCAL_FAKE_GENERATOR_VERSION,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

const MAX_CHECKSUM_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CHECKSUM_LINE_BYTES: usize = 4 * 1024;
const MAX_CHECKSUM_ENTRIES: usize = 10_000;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_IDENTITY_RECORD_BYTES: u64 = 1024 * 1024;
const MAX_ATTEMPT_PROVENANCE_BYTES: u64 = 16 * 1024;
const MAX_ATTEMPT_RESOURCES_BYTES: u64 = 4 * 1024;
const MAX_PROVENANCE_VALUE_BYTES: usize = 256;
const MAX_NIX_STORE_PATH_BYTES: usize = 512;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("could not read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("SHA256SUMS is not valid UTF-8")]
    InvalidChecksumFileEncoding,
    #[error("SHA256SUMS exceeds the {MAX_CHECKSUM_FILE_BYTES}-byte size limit")]
    ChecksumFileTooLarge,
    #[error("SHA256SUMS line exceeds the {MAX_CHECKSUM_LINE_BYTES}-byte size limit")]
    ChecksumLineTooLong,
    #[error("SHA256SUMS exceeds the {MAX_CHECKSUM_ENTRIES}-entry limit")]
    TooManyChecksumEntries,
    #[error("manifest.json exceeds the {MAX_MANIFEST_BYTES}-byte size limit")]
    ManifestTooLarge,
    #[error("invalid manifest JSON: {0}")]
    Manifest(serde_json::Error),
    #[error("invalid evidence record {path}: {source}")]
    InvalidRecord {
        path: String,
        source: serde_json::Error,
    },
    #[error("invalid SHA256SUMS line: {0}")]
    InvalidChecksumLine(String),
    #[error("invalid SHA-256 digest for {0}")]
    InvalidChecksum(String),
    #[error("unsafe evidence path in SHA256SUMS: {0}")]
    UnsafePath(String),
    #[error("duplicate evidence path in SHA256SUMS: {0}")]
    DuplicatePath(String),
    #[error("evidence path escapes the bundle root: {0}")]
    PathEscape(String),
    #[error("evidence bundle root is not a directory: {0}")]
    BundleRootNotDirectory(String),
    #[error("evidence payload is not a regular file: {0}")]
    NonRegularFile(String),
    #[error("evidence payload path is not valid UTF-8: {0}")]
    NonUtf8Path(String),
    #[error("SHA256SUMS must contain a checksum for manifest.json")]
    MissingManifestChecksum,
    #[error("SHA256SUMS is missing payload: {0}")]
    MissingPayloadChecksum(String),
    #[error("SHA256SUMS declares a missing payload: {0}")]
    DeclaredPayloadMissing(String),
    #[error("required evidence payload is missing: {0}")]
    MissingRequiredPayload(String),
    #[error("checksum mismatch for {0}")]
    ChecksumMismatch(String),
    #[error("unsupported evidence format: {0}")]
    UnsupportedFormat(String),
    #[error("manifest field {0} must not be empty")]
    EmptyManifestField(&'static str),
    #[error("manifest runId must be run-<canonical lowercase RFC 4122 UUIDv7>")]
    InvalidRunId,
    #[error("manifest runStatus must be terminal")]
    NonTerminalRunStatus,
    #[error("manifest attemptCount must equal 1 for benchplane-evidence/v1")]
    InvalidAttemptCount,
    #[error("manifest experimentDigest must be a lowercase sha256 digest")]
    InvalidExperimentDigest,
    #[error("manifest resolvedPlanDigest must be a lowercase sha256 digest")]
    InvalidResolvedPlanDigest,
    #[error("SHA256SUMS already exists")]
    AlreadyFinalized,
    #[error("bundle directory name must match manifest runId")]
    BundleRunIdMismatch,
    #[error("evidence record {0} has inconsistent run identity or status")]
    InconsistentRecord(String),
    #[error("invalid attempt provenance: {0}")]
    InvalidAttemptProvenance(String),
    #[error("invalid attempt resources: {0}")]
    InvalidAttemptResources(String),
}

pub fn verify_evidence_bundle(root: &Path) -> Result<EvidenceManifest, EvidenceError> {
    let root = canonical_bundle_root(root)?;
    let payloads = gather_payload_paths(&root)?;
    let sums_path = checked_regular_path(&root, Path::new("SHA256SUMS"), "SHA256SUMS")?;
    let sums_size = fs::metadata(&sums_path)
        .map_err(|source| EvidenceError::Read {
            path: sums_path.display().to_string(),
            source,
        })?
        .len();
    if sums_size > MAX_CHECKSUM_FILE_BYTES {
        return Err(EvidenceError::ChecksumFileTooLarge);
    }
    let sums_file = File::open(&sums_path).map_err(|source| EvidenceError::Read {
        path: sums_path.display().to_string(),
        source,
    })?;
    let mut sums = BufReader::new(sums_file);

    let mut checked_names = BTreeSet::new();
    let mut checked_targets = BTreeSet::new();
    let mut retained_records = BTreeMap::new();
    let mut line = String::new();
    let mut total_checksum_bytes = 0_u64;
    loop {
        line.clear();
        let bytes_read = sums
            .read_line(&mut line)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::InvalidData => EvidenceError::InvalidChecksumFileEncoding,
                _ => EvidenceError::Read {
                    path: sums_path.display().to_string(),
                    source: error,
                },
            })?;
        if bytes_read == 0 {
            break;
        }
        total_checksum_bytes = total_checksum_bytes.saturating_add(bytes_read as u64);
        if total_checksum_bytes > MAX_CHECKSUM_FILE_BYTES {
            return Err(EvidenceError::ChecksumFileTooLarge);
        }
        if bytes_read > MAX_CHECKSUM_LINE_BYTES {
            return Err(EvidenceError::ChecksumLineTooLong);
        }
        let line = line.trim_end_matches(&['\r', '\n'][..]);
        if line.trim().is_empty() {
            continue;
        }
        if checked_names.len() >= MAX_CHECKSUM_ENTRIES {
            return Err(EvidenceError::TooManyChecksumEntries);
        }
        let (expected, relative) = line
            .split_once("  ")
            .ok_or_else(|| EvidenceError::InvalidChecksumLine(line.to_owned()))?;

        if expected.len() != 64
            || !expected
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(EvidenceError::InvalidChecksum(relative.to_owned()));
        }

        let relative_path = validate_relative_path(relative)?;
        if normalized_relative_path(relative_path)? != relative {
            return Err(EvidenceError::UnsafePath(relative.to_owned()));
        }
        if !checked_names.insert(relative.to_owned()) {
            return Err(EvidenceError::DuplicatePath(relative.to_owned()));
        }
        if !payloads.contains_key(relative) {
            return Err(EvidenceError::DeclaredPayloadMissing(relative.to_owned()));
        }

        let retained_limit = retained_limit(relative);
        let (canonical_path, actual, bytes) =
            hash_regular_file(&root, relative_path, relative, retained_limit)?;
        if !checked_targets.insert(canonical_path) {
            return Err(EvidenceError::DuplicatePath(relative.to_owned()));
        }
        if actual != expected {
            return Err(EvidenceError::ChecksumMismatch(relative.to_owned()));
        }
        if let Some(bytes) = bytes {
            retained_records.insert(relative.to_owned(), bytes);
        }
    }

    let manifest_bytes = retained_records
        .get("manifest.json")
        .ok_or(EvidenceError::MissingManifestChecksum)?;
    for relative in payloads.keys() {
        if !checked_names.contains(relative) {
            return Err(EvidenceError::MissingPayloadChecksum(relative.clone()));
        }
    }

    let manifest: EvidenceManifest =
        serde_json::from_slice(manifest_bytes).map_err(EvidenceError::Manifest)?;
    validate_manifest(&manifest)?;
    validate_required_payloads(&checked_names)?;
    validate_record_consistency(&root, &manifest, &retained_records)?;

    Ok(manifest)
}

pub(crate) struct PendingEvidenceDigest(Sha256);

impl PendingEvidenceDigest {
    pub(crate) fn finish(self) -> String {
        format!("sha256:{}", hex::encode(self.0.finalize()))
    }
}

pub(crate) fn write_checksum_file(root: &Path) -> Result<PendingEvidenceDigest, EvidenceError> {
    if root.join("SHA256SUMS").exists() {
        return Err(EvidenceError::AlreadyFinalized);
    }

    let root = canonical_bundle_root(root)?;
    let payloads = gather_payload_paths(&root)?;

    let temporary = root.join("SHA256SUMS.tmp");
    let final_path = root.join("SHA256SUMS");
    let file = File::create(&temporary).map_err(|source| EvidenceError::Write {
        path: temporary.display().to_string(),
        source,
    })?;
    let mut writer = BufWriter::new(file);
    let mut sums_hasher = Sha256::new();
    for (relative, path) in payloads {
        let (_, digest, _) = hash_regular_file(&root, &path, &relative, None)?;
        let line = format!("{digest}  {relative}\n");
        writer
            .write_all(line.as_bytes())
            .map_err(|source| EvidenceError::Write {
                path: temporary.display().to_string(),
                source,
            })?;
        sums_hasher.update(line.as_bytes());
    }
    writer.flush().map_err(|source| EvidenceError::Write {
        path: temporary.display().to_string(),
        source,
    })?;
    drop(writer);
    fs::rename(&temporary, &final_path).map_err(|source| EvidenceError::Write {
        path: final_path.display().to_string(),
        source,
    })?;
    Ok(PendingEvidenceDigest(sums_hasher))
}

fn gather_payload_paths(root: &Path) -> Result<BTreeMap<String, PathBuf>, EvidenceError> {
    let mut payloads = BTreeMap::new();
    let mut entries_seen = 0_usize;
    gather_directory(root, root, &mut payloads, &mut entries_seen)?;
    Ok(payloads)
}

fn gather_directory(
    root: &Path,
    directory: &Path,
    payloads: &mut BTreeMap<String, PathBuf>,
    entries_seen: &mut usize,
) -> Result<(), EvidenceError> {
    let entries = fs::read_dir(directory).map_err(|source| EvidenceError::Read {
        path: directory.display().to_string(),
        source,
    })?;
    for entry in entries {
        *entries_seen += 1;
        if *entries_seen > MAX_CHECKSUM_ENTRIES {
            return Err(EvidenceError::TooManyChecksumEntries);
        }
        let entry = entry.map_err(|source| EvidenceError::Read {
            path: directory.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("entry must be under root");
        let display = normalized_relative_path(relative)?;
        if display == "SHA256SUMS" {
            continue;
        }

        let metadata = fs::symlink_metadata(&path).map_err(|source| EvidenceError::Read {
            path: path.display().to_string(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(EvidenceError::NonRegularFile(display));
        }
        if metadata.is_dir() {
            gather_directory(root, &path, payloads, entries_seen)?;
        } else if metadata.is_file() {
            payloads.insert(display, relative.to_owned());
        } else {
            return Err(EvidenceError::NonRegularFile(display));
        }
    }
    Ok(())
}

fn normalized_relative_path(path: &Path) -> Result<String, EvidenceError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(EvidenceError::UnsafePath(path.display().to_string()));
    }
    let components: Result<Vec<_>, _> = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| EvidenceError::NonUtf8Path(path.display().to_string())),
            _ => unreachable!("components were validated"),
        })
        .collect();
    Ok(components?.join("/"))
}

fn canonical_bundle_root(root: &Path) -> Result<PathBuf, EvidenceError> {
    let canonical = fs::canonicalize(root).map_err(|source| EvidenceError::Read {
        path: root.display().to_string(),
        source,
    })?;
    let metadata = fs::metadata(&canonical).map_err(|source| EvidenceError::Read {
        path: canonical.display().to_string(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(EvidenceError::BundleRootNotDirectory(
            root.display().to_string(),
        ));
    }
    Ok(canonical)
}

fn validate_relative_path(relative: &str) -> Result<&Path, EvidenceError> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative == "SHA256SUMS"
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(EvidenceError::UnsafePath(relative.to_owned()));
    }
    Ok(path)
}

fn checked_regular_path(
    root: &Path,
    relative: &Path,
    display: &str,
) -> Result<PathBuf, EvidenceError> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|source| EvidenceError::Read {
        path: path.display().to_string(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(EvidenceError::NonRegularFile(display.to_owned()));
    }

    let canonical = fs::canonicalize(&path).map_err(|source| EvidenceError::Read {
        path: path.display().to_string(),
        source,
    })?;
    if !canonical.starts_with(root) {
        return Err(EvidenceError::PathEscape(display.to_owned()));
    }

    Ok(canonical)
}

fn hash_regular_file(
    root: &Path,
    relative: &Path,
    display: &str,
    retain_limit: Option<u64>,
) -> Result<(PathBuf, String, Option<Vec<u8>>), EvidenceError> {
    let canonical = checked_regular_path(root, relative, display)?;
    let metadata = fs::metadata(&canonical).map_err(|source| EvidenceError::Read {
        path: canonical.display().to_string(),
        source,
    })?;
    if let Some(limit) = retain_limit {
        if metadata.len() > limit {
            return if display == "manifest.json" {
                Err(EvidenceError::ManifestTooLarge)
            } else {
                Err(EvidenceError::InconsistentRecord(format!(
                    "{display} exceeds its parsing limit"
                )))
            };
        }
    }

    let file = File::open(&canonical).map_err(|source| EvidenceError::Read {
        path: canonical.display().to_string(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut retained =
        retain_limit.map(|limit| Vec::with_capacity(metadata.len().min(limit) as usize));
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|source| EvidenceError::Read {
                path: canonical.display().to_string(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        if let Some(bytes) = &mut retained {
            let limit = retain_limit.expect("retained payload has a limit") as usize;
            if bytes.len().saturating_add(count) > limit {
                return if display == "manifest.json" {
                    Err(EvidenceError::ManifestTooLarge)
                } else {
                    Err(EvidenceError::InconsistentRecord(format!(
                        "{display} exceeds its parsing limit"
                    )))
                };
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
    }
    Ok((canonical, hex::encode(hasher.finalize()), retained))
}

fn retained_limit(relative: &str) -> Option<u64> {
    match relative {
        "manifest.json" => Some(MAX_MANIFEST_BYTES),
        "run.json"
        | "attempts/0001/attempt.json"
        | "validity.json"
        | "summary.json"
        | "events.jsonl" => Some(MAX_IDENTITY_RECORD_BYTES),
        "attempts/0001/provenance.json" => Some(MAX_ATTEMPT_PROVENANCE_BYTES),
        "attempts/0001/resources.json" => Some(MAX_ATTEMPT_RESOURCES_BYTES),
        _ => None,
    }
}

fn validate_manifest(manifest: &EvidenceManifest) -> Result<(), EvidenceError> {
    if manifest.format != EVIDENCE_FORMAT_V1 {
        return Err(EvidenceError::UnsupportedFormat(manifest.format.clone()));
    }
    if manifest.run_id.trim().is_empty() {
        return Err(EvidenceError::EmptyManifestField("runId"));
    }
    if !is_run_id(&manifest.run_id) {
        return Err(EvidenceError::InvalidRunId);
    }
    if !manifest.run_status.is_terminal() {
        return Err(EvidenceError::NonTerminalRunStatus);
    }
    if manifest.attempt_count != 1 {
        return Err(EvidenceError::InvalidAttemptCount);
    }
    if !is_sha256_digest(&manifest.experiment_digest) {
        return Err(EvidenceError::InvalidExperimentDigest);
    }
    if !is_sha256_digest(&manifest.resolved_plan_digest) {
        return Err(EvidenceError::InvalidResolvedPlanDigest);
    }
    Ok(())
}

fn validate_required_payloads(names: &BTreeSet<String>) -> Result<(), EvidenceError> {
    for required_path in [
        "experiment.yaml",
        "resolved-plan.json",
        "run.json",
        "events.jsonl",
        "validity.json",
        "summary.json",
        "manifest.json",
        "attempts/0001/attempt.json",
        "attempts/0001/measurements.jsonl",
    ] {
        if !names.contains(required_path) {
            return Err(EvidenceError::MissingRequiredPayload(
                required_path.to_owned(),
            ));
        }
    }
    Ok(())
}

fn parse_record<T: serde::de::DeserializeOwned>(
    records: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<T, EvidenceError> {
    let bytes = records
        .get(path)
        .ok_or_else(|| EvidenceError::MissingRequiredPayload(path.to_owned()))?;
    serde_json::from_slice(bytes).map_err(|source| EvidenceError::InvalidRecord {
        path: path.to_owned(),
        source,
    })
}

fn validate_record_consistency(
    root: &Path,
    manifest: &EvidenceManifest,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<(), EvidenceError> {
    if root.file_name().and_then(|name| name.to_str()) != Some(manifest.run_id.as_str()) {
        return Err(EvidenceError::BundleRunIdMismatch);
    }

    let run: RunRecord = parse_record(records, "run.json")?;
    if run.run_id != manifest.run_id
        || run.run_status != manifest.run_status
        || run.attempt_count != 1
        || run.experiment_digest != manifest.experiment_digest
        || run.resolved_plan_digest != manifest.resolved_plan_digest
    {
        return Err(EvidenceError::InconsistentRecord("run.json".to_owned()));
    }
    let attempt: AttemptRecord = parse_record(records, "attempts/0001/attempt.json")?;
    let expected_attempt_status = match manifest.run_status {
        RunState::Succeeded => AttemptStatus::Succeeded,
        RunState::Failed => AttemptStatus::Failed,
        RunState::Interrupted => AttemptStatus::Interrupted,
        _ => unreachable!("manifest status was validated as terminal"),
    };
    if attempt.run_id != manifest.run_id
        || attempt.attempt_number != 1
        || attempt.status != expected_attempt_status
    {
        return Err(EvidenceError::InconsistentRecord(
            "attempts/0001/attempt.json".to_owned(),
        ));
    }
    if let Some(bytes) = records.get("attempts/0001/provenance.json") {
        let provenance: AttemptProvenance =
            serde_json::from_slice(bytes).map_err(|source| EvidenceError::InvalidRecord {
                path: "attempts/0001/provenance.json".to_owned(),
                source,
            })?;
        validate_attempt_provenance(&provenance, manifest)?;
    }
    if let Some(bytes) = records.get("attempts/0001/resources.json") {
        let resources: AttemptResources =
            serde_json::from_slice(bytes).map_err(|source| EvidenceError::InvalidRecord {
                path: "attempts/0001/resources.json".to_owned(),
                source,
            })?;
        validate_attempt_resources(&resources, manifest)?;
    }
    let validity: ValidityResult = parse_record(records, "validity.json")?;
    if validity.run_id != manifest.run_id || validity.status != manifest.validity_status {
        return Err(EvidenceError::InconsistentRecord(
            "validity.json".to_owned(),
        ));
    }
    let summary: RunSummary = parse_record(records, "summary.json")?;
    if summary.run_id != manifest.run_id
        || summary.run_status != manifest.run_status
        || summary.validity_status != manifest.validity_status
        || summary.attempt_count != 1
        || summary.experiment_digest != manifest.experiment_digest
        || summary.resolved_plan_digest != manifest.resolved_plan_digest
    {
        return Err(EvidenceError::InconsistentRecord("summary.json".to_owned()));
    }

    let events = records
        .get("events.jsonl")
        .ok_or_else(|| EvidenceError::MissingRequiredPayload("events.jsonl".to_owned()))?;
    for line in events
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let event: LifecycleEvent =
            serde_json::from_slice(line).map_err(|source| EvidenceError::InvalidRecord {
                path: "events.jsonl".to_owned(),
                source,
            })?;
        if event.run_id != manifest.run_id || event.attempt_number != 1 {
            return Err(EvidenceError::InconsistentRecord("events.jsonl".to_owned()));
        }
    }
    Ok(())
}

fn validate_attempt_resources(
    resources: &AttemptResources,
    manifest: &EvidenceManifest,
) -> Result<(), EvidenceError> {
    if resources.format != ATTEMPT_RESOURCES_FORMAT_V1 {
        return invalid_resources("unsupported format");
    }
    if resources.run_id != manifest.run_id || resources.attempt_number != 1 {
        return invalid_resources("runId or attemptNumber does not match the bundle");
    }
    if !resources.peak_rss_bytes.is_multiple_of(1024) {
        return invalid_resources("peakRssBytes is not an exact Linux KiB-to-byte conversion");
    }
    Ok(())
}

fn validate_attempt_provenance(
    provenance: &AttemptProvenance,
    manifest: &EvidenceManifest,
) -> Result<(), EvidenceError> {
    if provenance.format != ATTEMPT_PROVENANCE_FORMAT_V1 {
        return invalid_provenance("unsupported format");
    }
    if provenance.run_id != manifest.run_id || provenance.attempt_number != 1 {
        return invalid_provenance("runId or attemptNumber does not match the bundle");
    }

    require_provenance_value(
        &provenance.platform.operating_system.family,
        "operatingSystem.family",
    )?;
    optional_provenance_value(
        provenance.platform.operating_system.distribution.as_deref(),
        "operatingSystem.distribution",
    )?;
    optional_provenance_value(
        provenance.platform.operating_system.version.as_deref(),
        "operatingSystem.version",
    )?;
    require_provenance_value(&provenance.platform.kernel.name, "kernel.name")?;
    optional_provenance_value(
        provenance.platform.kernel.release.as_deref(),
        "kernel.release",
    )?;
    require_provenance_value(&provenance.platform.architecture, "architecture")?;
    optional_provenance_value(provenance.platform.cpu.model.as_deref(), "cpu.model")?;
    if provenance.platform.cpu.logical_cpu_count == Some(0) {
        return invalid_provenance("cpu.logicalCpuCount must be positive when present");
    }

    let benchplane = &provenance.software.benchplane;
    if benchplane.name != BENCHPLANE_SOFTWARE_NAME {
        return invalid_provenance("software.benchplane.name is not supported");
    }
    require_provenance_value(&benchplane.version, "software.benchplane.version")?;
    optional_nix_store_path(
        benchplane.nix_store_path.as_deref(),
        "software.benchplane.nixStorePath",
    )?;

    match &provenance.software.runtime {
        RuntimeProvenance::LocalFake { generator } => {
            if generator != LOCAL_FAKE_GENERATOR_VERSION {
                return invalid_provenance("localFake generator identity is not supported");
            }
        }
        RuntimeProvenance::CpuProbe { generator } => {
            if generator != CPU_PROBE_GENERATOR_VERSION {
                return invalid_provenance("cpuProbe generator identity is not supported");
            }
        }
        RuntimeProvenance::LlamaCpp {
            generator,
            engine,
            model,
            backend,
        } => {
            if generator != LLAMA_CPP_GENERATOR_VERSION
                || engine.name != LLAMA_CPP_ENGINE_NAME
                || engine.version != LLAMA_CPP_ENGINE_VERSION
                || model.identity != LLAMA_CPP_MODEL_IDENTITY
                || model.sha256 != LLAMA_CPP_MODEL_SHA256
                || backend.identity != LLAMA_CPP_BACKEND_IDENTITY
                || backend.device_class != DeviceClass::Cpu
            {
                return invalid_provenance("llamaCpp software lineage is not supported");
            }
            optional_nix_store_path(
                engine.nix_store_path.as_deref(),
                "runtime.engine.nixStorePath",
            )?;
            optional_nix_store_path(
                model.nix_store_path.as_deref(),
                "runtime.model.nixStorePath",
            )?;
            optional_nix_store_path(
                backend.nix_store_path.as_deref(),
                "runtime.backend.nixStorePath",
            )?;
            let store_paths_present = [
                engine.nix_store_path.is_some(),
                model.nix_store_path.is_some(),
                backend.nix_store_path.is_some(),
            ];
            if store_paths_present.iter().any(|present| *present)
                && !store_paths_present.iter().all(|present| *present)
            {
                return invalid_provenance(
                    "llamaCpp Nix store lineage must be wholly present or absent",
                );
            }
            if engine.nix_store_path != backend.nix_store_path {
                return invalid_provenance(
                    "llamaCpp engine and backend must identify the same Nix store object",
                );
            }
        }
    }
    Ok(())
}

fn require_provenance_value(value: &str, field: &'static str) -> Result<(), EvidenceError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_PROVENANCE_VALUE_BYTES
        || value.chars().any(char::is_control)
    {
        return invalid_provenance(&format!(
            "{field} is empty, oversized, or contains controls"
        ));
    }
    Ok(())
}

fn optional_provenance_value(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), EvidenceError> {
    if let Some(value) = value {
        require_provenance_value(value, field)?;
    }
    Ok(())
}

fn optional_nix_store_path(value: Option<&str>, field: &'static str) -> Result<(), EvidenceError> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(entry) = value.strip_prefix("/nix/store/") else {
        return invalid_provenance(&format!("{field} is not a Nix store object path"));
    };
    let (hash, name) = entry.split_once('-').unwrap_or_default();
    const NIX_BASE32: &str = "0123456789abcdfghijklmnpqrsvwxyz";
    if value.len() > MAX_NIX_STORE_PATH_BYTES
        || entry.contains('/')
        || hash.len() != 32
        || !hash.chars().all(|character| NIX_BASE32.contains(character))
        || name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "+-._?=".contains(character))
    {
        return invalid_provenance(&format!("{field} is not a bounded Nix store object path"));
    }
    Ok(())
}

fn invalid_provenance<T>(message: &str) -> Result<T, EvidenceError> {
    Err(EvidenceError::InvalidAttemptProvenance(message.to_owned()))
}

fn invalid_resources<T>(message: &str) -> Result<T, EvidenceError> {
    Err(EvidenceError::InvalidAttemptResources(message.to_owned()))
}

fn is_run_id(value: &str) -> bool {
    let Some(uuid_text) = value.strip_prefix("run-") else {
        return false;
    };
    let Ok(uuid) = uuid::Uuid::parse_str(uuid_text) else {
        return false;
    };
    uuid_text == uuid.hyphenated().to_string()
        && uuid.get_version_num() == 7
        && uuid.get_variant() == uuid::Variant::RFC4122
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use benchplane_schema::{RunState, ValidityStatus, EVIDENCE_FORMAT_V1};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
        cleanup_root: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "benchplane-{label}-{}-{counter}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self {
                cleanup_root: path.clone(),
                path,
            }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.cleanup_root);
        }
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/evidence/run-018f6f9a-7b3c-7abc-8def-0123456789ab")
    }

    fn copy_directory(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).expect("create destination directory");
        for entry in fs::read_dir(source).expect("read source directory") {
            let entry = entry.expect("read source entry");
            let target = destination.join(entry.file_name());
            if entry.file_type().expect("entry file type").is_dir() {
                copy_directory(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("copy fixture file");
            }
        }
    }

    fn copy_fixture(label: &str) -> TestDirectory {
        let mut directory = TestDirectory::new(label);
        directory.path = directory
            .cleanup_root
            .join("run-018f6f9a-7b3c-7abc-8def-0123456789ab");
        copy_directory(&fixture_root(), &directory.path);
        directory
    }

    fn rewrite_checksums(root: &Path) {
        fs::remove_file(root.join("SHA256SUMS")).expect("remove existing checksums");
        write_checksum_file(root).expect("rewrite checksums");
    }

    fn rewrite_manifest(root: &Path, manifest: &EvidenceManifest) {
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        rewrite_checksums(root);
    }

    fn valid_manifest() -> EvidenceManifest {
        EvidenceManifest {
            format: EVIDENCE_FORMAT_V1.to_owned(),
            run_id: "run-018f6f9a-7b3c-7abc-8def-0123456789ab".to_owned(),
            run_status: RunState::Succeeded,
            validity_status: ValidityStatus::Valid,
            experiment_digest: format!("sha256:{}", "a".repeat(64)),
            resolved_plan_digest: format!("sha256:{}", "b".repeat(64)),
            attempt_count: 1,
        }
    }

    fn valid_provenance() -> AttemptProvenance {
        serde_json::from_value(serde_json::json!({
            "format": ATTEMPT_PROVENANCE_FORMAT_V1,
            "runId": "run-018f6f9a-7b3c-7abc-8def-0123456789ab",
            "attemptNumber": 1,
            "platform": {
                "operatingSystem": {
                    "family": "linux",
                    "distribution": "nixos",
                    "version": "26.05"
                },
                "kernel": { "name": "Linux", "release": "6.12.1" },
                "architecture": "x86_64",
                "cpu": { "model": "Example CPU", "logicalCpuCount": 2 }
            },
            "software": {
                "benchplane": {
                    "name": BENCHPLANE_SOFTWARE_NAME,
                    "version": "0.1.0",
                    "nixStorePath": null
                },
                "runtime": {
                    "kind": "llamaCpp",
                    "generator": LLAMA_CPP_GENERATOR_VERSION,
                    "engine": {
                        "name": LLAMA_CPP_ENGINE_NAME,
                        "version": LLAMA_CPP_ENGINE_VERSION,
                        "nixStorePath": "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-llama-cpp-b10133"
                    },
                    "model": {
                        "identity": LLAMA_CPP_MODEL_IDENTITY,
                        "sha256": LLAMA_CPP_MODEL_SHA256,
                        "nixStorePath": "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-smollm2.gguf"
                    },
                    "backend": {
                        "identity": LLAMA_CPP_BACKEND_IDENTITY,
                        "deviceClass": "cpu",
                        "nixStorePath": "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-llama-cpp-b10133"
                    }
                }
            }
        }))
        .expect("construct valid attempt provenance")
    }

    fn write_provenance(root: &Path, provenance: &AttemptProvenance) {
        fs::write(
            root.join("attempts/0001/provenance.json"),
            serde_json::to_vec_pretty(provenance).expect("serialize provenance"),
        )
        .expect("write provenance");
    }

    fn valid_resources() -> AttemptResources {
        serde_json::from_value(serde_json::json!({
            "format": ATTEMPT_RESOURCES_FORMAT_V1,
            "runId": "run-018f6f9a-7b3c-7abc-8def-0123456789ab",
            "attemptNumber": 1,
            "scope": "helperProcessLifetime",
            "cpuTimeMicros": 12345,
            "peakRssBytes": 4194304
        }))
        .expect("construct valid attempt resources")
    }

    fn write_resources(root: &Path, resources: &AttemptResources) {
        fs::write(
            root.join("attempts/0001/resources.json"),
            serde_json::to_vec_pretty(resources).expect("serialize resources"),
        )
        .expect("write resources");
    }

    #[test]
    fn verifies_the_checked_in_fixture() {
        let manifest = verify_evidence_bundle(&fixture_root()).expect("fixture should verify");
        assert_eq!(manifest.format, EVIDENCE_FORMAT_V1);
    }

    #[test]
    fn verifies_optional_attempt_provenance_and_checksums_it() {
        let directory = copy_fixture("attempt-provenance");
        write_provenance(&directory.path, &valid_provenance());
        rewrite_checksums(&directory.path);

        verify_evidence_bundle(&directory.path).expect("provenance extension should verify");
        assert!(fs::read_to_string(directory.path.join("SHA256SUMS"))
            .expect("read checksums")
            .lines()
            .any(|line| line.ends_with("  attempts/0001/provenance.json")));
    }

    #[test]
    fn rejects_tampered_and_semantically_invalid_attempt_provenance() {
        let tampered = copy_fixture("tampered-attempt-provenance");
        write_provenance(&tampered.path, &valid_provenance());
        rewrite_checksums(&tampered.path);
        fs::write(tampered.path.join("attempts/0001/provenance.json"), b"{}\n")
            .expect("tamper with provenance");
        assert!(matches!(
            verify_evidence_bundle(&tampered.path),
            Err(EvidenceError::ChecksumMismatch(path))
                if path == "attempts/0001/provenance.json"
        ));

        let inconsistent = copy_fixture("inconsistent-attempt-provenance");
        let mut provenance = valid_provenance();
        provenance.run_id = "run-018f6f9a-7b3c-7abc-8def-0123456789ac".to_owned();
        write_provenance(&inconsistent.path, &provenance);
        rewrite_checksums(&inconsistent.path);
        assert!(matches!(
            verify_evidence_bundle(&inconsistent.path),
            Err(EvidenceError::InvalidAttemptProvenance(_))
        ));

        let malformed = copy_fixture("malformed-attempt-provenance");
        fs::write(
            malformed.path.join("attempts/0001/provenance.json"),
            b"{}\n",
        )
        .expect("write malformed provenance");
        rewrite_checksums(&malformed.path);
        assert!(matches!(
            verify_evidence_bundle(&malformed.path),
            Err(EvidenceError::InvalidRecord { path, .. })
                if path == "attempts/0001/provenance.json"
        ));
    }

    #[test]
    fn bounds_attempt_provenance_parsing() {
        let directory = copy_fixture("oversized-attempt-provenance");
        fs::write(
            directory.path.join("attempts/0001/provenance.json"),
            vec![b' '; MAX_ATTEMPT_PROVENANCE_BYTES as usize + 1],
        )
        .expect("write oversized provenance");
        rewrite_checksums(&directory.path);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::InconsistentRecord(path))
                if path.contains("attempts/0001/provenance.json exceeds")
        ));
    }

    #[test]
    fn verifies_optional_attempt_resources_and_checksums_them() {
        let directory = copy_fixture("attempt-resources");
        write_resources(&directory.path, &valid_resources());
        rewrite_checksums(&directory.path);

        verify_evidence_bundle(&directory.path).expect("resource extension should verify");
        assert!(fs::read_to_string(directory.path.join("SHA256SUMS"))
            .expect("read checksums")
            .lines()
            .any(|line| line.ends_with("  attempts/0001/resources.json")));
    }

    #[test]
    fn rejects_tampered_or_inconsistent_attempt_resources() {
        let tampered = copy_fixture("tampered-attempt-resources");
        write_resources(&tampered.path, &valid_resources());
        rewrite_checksums(&tampered.path);
        fs::write(tampered.path.join("attempts/0001/resources.json"), b"{}\n")
            .expect("tamper with resources");
        assert!(matches!(
            verify_evidence_bundle(&tampered.path),
            Err(EvidenceError::ChecksumMismatch(path))
                if path == "attempts/0001/resources.json"
        ));

        for (label, mutate) in [
            ("format", 0_u8),
            ("run-id", 1),
            ("attempt", 2),
            ("rss-units", 3),
        ] {
            let directory = copy_fixture(&format!("invalid-attempt-resources-{label}"));
            let mut resources = valid_resources();
            match mutate {
                0 => resources.format = "benchplane-attempt-resources/v2".to_owned(),
                1 => resources.run_id = "run-018f6f9a-7b3c-7abc-8def-0123456789ac".to_owned(),
                2 => resources.attempt_number = 2,
                3 => resources.peak_rss_bytes += 1,
                _ => unreachable!(),
            }
            write_resources(&directory.path, &resources);
            rewrite_checksums(&directory.path);
            assert!(matches!(
                verify_evidence_bundle(&directory.path),
                Err(EvidenceError::InvalidAttemptResources(_))
            ));
        }
    }

    #[test]
    fn rejects_unknown_malformed_and_oversized_attempt_resources() {
        let unknown_scope = copy_fixture("unknown-resource-scope");
        fs::write(
            unknown_scope.path.join("attempts/0001/resources.json"),
            serde_json::to_vec(&serde_json::json!({
                "format": ATTEMPT_RESOURCES_FORMAT_V1,
                "runId": "run-018f6f9a-7b3c-7abc-8def-0123456789ab",
                "attemptNumber": 1,
                "scope": "systemWide",
                "cpuTimeMicros": 1,
                "peakRssBytes": 1024
            }))
            .expect("serialize unknown scope"),
        )
        .expect("write unknown scope");
        rewrite_checksums(&unknown_scope.path);
        assert!(matches!(
            verify_evidence_bundle(&unknown_scope.path),
            Err(EvidenceError::InvalidRecord { path, .. })
                if path == "attempts/0001/resources.json"
        ));

        let malformed = copy_fixture("malformed-attempt-resources");
        fs::write(malformed.path.join("attempts/0001/resources.json"), b"{}\n")
            .expect("write malformed resources");
        rewrite_checksums(&malformed.path);
        assert!(matches!(
            verify_evidence_bundle(&malformed.path),
            Err(EvidenceError::InvalidRecord { path, .. })
                if path == "attempts/0001/resources.json"
        ));

        let oversized = copy_fixture("oversized-attempt-resources");
        fs::write(
            oversized.path.join("attempts/0001/resources.json"),
            vec![b' '; MAX_ATTEMPT_RESOURCES_BYTES as usize + 1],
        )
        .expect("write oversized resources");
        rewrite_checksums(&oversized.path);
        assert!(matches!(
            verify_evidence_bundle(&oversized.path),
            Err(EvidenceError::InconsistentRecord(path))
                if path.contains("attempts/0001/resources.json exceeds")
        ));
    }

    #[test]
    fn accepts_crlf_checksum_inventory() {
        let directory = copy_fixture("crlf-checksums");
        let sums_path = directory.path.join("SHA256SUMS");
        let sums = fs::read_to_string(&sums_path).expect("read checksum inventory");
        fs::write(&sums_path, sums.replace('\n', "\r\n")).expect("write CRLF checksum inventory");

        verify_evidence_bundle(&directory.path).expect("CRLF checksum inventory should verify");
    }

    #[test]
    fn rejects_non_directory_bundle_root() {
        let directory = TestDirectory::new("non-directory-root");
        let root = directory.path.join("bundle");
        fs::write(&root, b"not a directory\n").expect("write bundle root file");

        assert!(matches!(
            verify_evidence_bundle(&root),
            Err(EvidenceError::BundleRootNotDirectory(path)) if path == root.display().to_string()
        ));
    }

    #[test]
    fn rejects_modified_payload_and_manifest() {
        let payload = copy_fixture("modified-payload");
        fs::write(payload.path.join("summary.json"), b"{}\n").expect("modify summary");
        assert!(matches!(
            verify_evidence_bundle(&payload.path),
            Err(EvidenceError::ChecksumMismatch(path)) if path == "summary.json"
        ));

        let manifest = copy_fixture("modified-manifest");
        fs::write(manifest.path.join("manifest.json"), b"{}\n").expect("modify manifest");
        assert!(matches!(
            verify_evidence_bundle(&manifest.path),
            Err(EvidenceError::ChecksumMismatch(path)) if path == "manifest.json"
        ));
    }

    #[test]
    fn rejects_malformed_checksum_line_and_digest() {
        let malformed = copy_fixture("malformed-line");
        fs::write(malformed.path.join("SHA256SUMS"), b"not-a-checksum-line\n")
            .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&malformed.path),
            Err(EvidenceError::InvalidChecksumLine(_))
        ));

        let invalid = copy_fixture("invalid-checksum");
        fs::write(
            invalid.path.join("SHA256SUMS"),
            format!("{}  manifest.json\n", "a".repeat(63)),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&invalid.path),
            Err(EvidenceError::InvalidChecksum(path)) if path == "manifest.json"
        ));
    }

    #[test]
    fn rejects_duplicate_and_missing_paths() {
        let duplicate = copy_fixture("duplicate-path");
        let manifest = fs::read(duplicate.path.join("manifest.json")).expect("read manifest");
        let digest = hex::encode(Sha256::digest(manifest));
        fs::write(
            duplicate.path.join("SHA256SUMS"),
            format!("{digest}  manifest.json\n{digest}  manifest.json\n"),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&duplicate.path),
            Err(EvidenceError::DuplicatePath(path)) if path == "manifest.json"
        ));

        let missing = copy_fixture("missing-manifest");
        let checksums =
            fs::read_to_string(missing.path.join("SHA256SUMS")).expect("read checksum fixture");
        let without_manifest = checksums
            .lines()
            .filter(|line| !line.ends_with("  manifest.json"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            missing.path.join("SHA256SUMS"),
            format!("{without_manifest}\n"),
        )
        .expect("rewrite checksums");
        assert!(matches!(
            verify_evidence_bundle(&missing.path),
            Err(EvidenceError::MissingManifestChecksum)
        ));
    }

    #[test]
    fn evidence_v1_rejects_every_attempt_count_other_than_one() {
        for attempt_count in [0, 2, u32::MAX] {
            let directory = copy_fixture(&format!("attempt-count-{attempt_count}"));
            let mut manifest = valid_manifest();
            manifest.attempt_count = attempt_count;
            rewrite_manifest(&directory.path, &manifest);
            assert!(matches!(
                verify_evidence_bundle(&directory.path),
                Err(EvidenceError::InvalidAttemptCount)
            ));
        }
    }

    #[test]
    fn rejects_undeclared_and_declared_but_missing_payloads() {
        let undeclared = copy_fixture("undeclared-payload");
        fs::write(undeclared.path.join("extra.txt"), b"not declared\n").expect("write extra");
        assert!(matches!(
            verify_evidence_bundle(&undeclared.path),
            Err(EvidenceError::MissingPayloadChecksum(path)) if path == "extra.txt"
        ));

        let missing = copy_fixture("declared-missing-payload");
        fs::remove_file(missing.path.join("summary.json")).expect("remove declared payload");
        assert!(matches!(
            verify_evidence_bundle(&missing.path),
            Err(EvidenceError::DeclaredPayloadMissing(path)) if path == "summary.json"
        ));
    }

    #[test]
    fn rejects_manifest_over_the_bounded_parse_limit() {
        let directory = copy_fixture("large-manifest");
        fs::write(
            directory.path.join("manifest.json"),
            vec![b' '; MAX_MANIFEST_BYTES as usize + 1],
        )
        .expect("write oversized manifest");
        rewrite_checksums(&directory.path);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::ManifestTooLarge)
        ));
    }

    #[test]
    fn verifies_large_nonparsed_payload_by_streaming() {
        let directory = copy_fixture("streamed-payload");
        fs::write(
            directory.path.join("attempts/0001/measurements.jsonl"),
            vec![b'x'; 2 * 1024 * 1024],
        )
        .expect("write large measurement payload");
        rewrite_checksums(&directory.path);
        verify_evidence_bundle(&directory.path).expect("large payload should be hashable");
    }

    #[test]
    fn rejects_unsafe_paths() {
        for (label, path) in [
            ("empty-path", ""),
            ("parent-traversal", "../manifest.json"),
            ("absolute-path", "/tmp/manifest.json"),
            ("self-checksum", "SHA256SUMS"),
        ] {
            let directory = copy_fixture(label);
            fs::write(
                directory.path.join("SHA256SUMS"),
                format!("{}  {path}\n", "0".repeat(64)),
            )
            .expect("modify checksums");
            assert!(matches!(
                verify_evidence_bundle(&directory.path),
                Err(EvidenceError::UnsafePath(rejected)) if rejected == path
            ));
        }
    }

    #[test]
    fn validates_manifest_fields() {
        type ErrorPredicate = fn(EvidenceError) -> bool;
        let cases: Vec<(EvidenceManifest, ErrorPredicate)> = vec![
            (
                {
                    let mut manifest = valid_manifest();
                    manifest.format = "benchplane-evidence/v2".to_owned();
                    manifest
                },
                |error| matches!(error, EvidenceError::UnsupportedFormat(_)),
            ),
            (
                {
                    let mut manifest = valid_manifest();
                    manifest.run_id = "  ".to_owned();
                    manifest
                },
                |error| matches!(error, EvidenceError::EmptyManifestField("runId")),
            ),
            (
                {
                    let mut manifest = valid_manifest();
                    manifest.experiment_digest = "sha256:not-a-digest".to_owned();
                    manifest
                },
                |error| matches!(error, EvidenceError::InvalidExperimentDigest),
            ),
            (
                {
                    let mut manifest = valid_manifest();
                    manifest.resolved_plan_digest = "sha256:not-a-digest".to_owned();
                    manifest
                },
                |error| matches!(error, EvidenceError::InvalidResolvedPlanDigest),
            ),
        ];

        for (index, (manifest, predicate)) in cases.into_iter().enumerate() {
            let directory = copy_fixture(&format!("manifest-case-{index}"));
            rewrite_manifest(&directory.path, &manifest);
            let error = verify_evidence_bundle(&directory.path).expect_err("manifest must fail");
            assert!(predicate(error));
        }
    }

    #[test]
    fn rejects_non_regular_payload() {
        let directory = copy_fixture("non-regular");
        fs::create_dir(directory.path.join("payload")).expect("create payload directory");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            directory.path.join("manifest.json"),
            directory.path.join("payload/link"),
        )
        .expect("create payload symlink");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::NonRegularFile(path)) if path == "payload/link"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let parent = TestDirectory::new("symlink-escape");
        let bundle = parent.path.join("bundle");
        let outside = parent.path.join("outside");
        fs::create_dir(&bundle).expect("create bundle");
        fs::create_dir(&outside).expect("create outside directory");
        fs::write(outside.join("payload"), b"outside\n").expect("write outside payload");
        symlink(&outside, bundle.join("escape")).expect("create escaping symlink");
        let digest = hex::encode(Sha256::digest(b"outside\n"));
        fs::write(
            bundle.join("SHA256SUMS"),
            format!("{digest}  escape/payload\n"),
        )
        .expect("write checksums");

        assert!(matches!(
            verify_evidence_bundle(&bundle),
            Err(EvidenceError::PathEscape(path)) | Err(EvidenceError::NonRegularFile(path))
                if path == "escape/payload" || path == "escape"
        ));
    }

    #[test]
    fn validates_digest_and_run_id_shapes() {
        assert!(is_sha256_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_sha256_digest(&format!("sha256:{}", "a".repeat(63))));
        assert!(!is_sha256_digest(&format!("sha256:{}", "A".repeat(64))));
        assert!(is_run_id("run-018f6f9a-7b3c-7abc-8def-0123456789ab"));
        for invalid in [
            "run-018F6F9A-7B3C-7ABC-8DEF-0123456789AB",
            "run-018f6f9a7b3c7abc8def0123456789ab",
            "run-018f6f9a-7b3c-4abc-8def-0123456789ab",
            "run-018f6f9a-7b3c-7abc-0def-0123456789ab",
            " run-018f6f9a-7b3c-7abc-8def-0123456789ab",
            "run-018f6f9a-7b3c-7abc-8def-0123456789ab ",
            "runs-018f6f9a-7b3c-7abc-8def-0123456789ab",
        ] {
            assert!(!is_run_id(invalid), "accepted invalid run ID {invalid}");
        }
    }

    #[test]
    fn rejects_internal_run_id_disagreement() {
        let directory = copy_fixture("run-id-disagreement");
        let path = directory.path.join("run.json");
        let mut run: RunRecord = serde_json::from_slice(&fs::read(&path).expect("read run record"))
            .expect("parse run record");
        run.run_id = "run-018f6f9a-7b3c-7abc-8def-0123456789ac".to_owned();
        fs::write(
            &path,
            serde_json::to_vec_pretty(&run).expect("serialize run"),
        )
        .expect("write run record");
        rewrite_checksums(&directory.path);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::InconsistentRecord(path)) if path == "run.json"
        ));
    }
}
