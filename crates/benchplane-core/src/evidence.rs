// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{
    AttemptProvenance, AttemptRecord, AttemptResources, AttemptStatus, DeviceClass,
    EvidenceManifest, LifecycleEvent, LocalFakeScenario, MeasurementPhase, MeasurementRecord,
    ProviderSpec, ResolvedExperiment, ResourceScope, RunRecord, RunState, RunSummary,
    RuntimeProvenance, RuntimeSpec, ValidityResult, ValidityStatus, ATTEMPT_PROVENANCE_FORMAT_V1,
    ATTEMPT_RESOURCES_FORMAT_V1, BENCHPLANE_SOFTWARE_NAME, CPU_PROBE_GENERATOR_VERSION,
    EVIDENCE_FORMAT_V1, LLAMA_CPP_BACKEND_IDENTITY, LLAMA_CPP_ENGINE_NAME,
    LLAMA_CPP_ENGINE_VERSION, LLAMA_CPP_GENERATOR_VERSION, LLAMA_CPP_GENERATOR_VERSION_V1,
    LLAMA_CPP_MODEL_IDENTITY, LLAMA_CPP_MODEL_SHA256, LOCAL_FAKE_GENERATOR_VERSION,
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
const MAX_RESOLVED_PLAN_BYTES: u64 = 64 * 1024;
const MAX_MEASUREMENTS_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MEASUREMENT_LINE_BYTES: usize = 64 * 1024;
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
    #[error("invalid resolved plan: {0}")]
    InvalidResolvedPlan(String),
    #[error("invalid measurements: {0}")]
    InvalidMeasurements(String),
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
    let mut verified_digests = BTreeMap::new();
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
        verified_digests.insert(relative.to_owned(), actual);
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
    validate_record_consistency(&root, &manifest, &retained_records, &verified_digests)?;

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
        "resolved-plan.json" => Some(MAX_RESOLVED_PLAN_BYTES),
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
    verified_digests: &BTreeMap<String, String>,
) -> Result<(), EvidenceError> {
    if root.file_name().and_then(|name| name.to_str()) != Some(manifest.run_id.as_str()) {
        return Err(EvidenceError::BundleRunIdMismatch);
    }

    let plan: ResolvedExperiment = parse_record(records, "resolved-plan.json")?;
    validate_resolved_plan(&plan, manifest)?;
    let measurement_digest = verified_digests
        .get("attempts/0001/measurements.jsonl")
        .ok_or_else(|| {
            EvidenceError::MissingRequiredPayload("attempts/0001/measurements.jsonl".to_owned())
        })?;
    let measurement_validation =
        validate_measurements(root, measurement_digest, &plan, manifest.run_status)?;

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
        validate_attempt_provenance(
            &provenance,
            manifest,
            &plan,
            measurement_validation.generator.as_deref(),
        )?;
    }
    if let Some(bytes) = records.get("attempts/0001/resources.json") {
        let resources: AttemptResources =
            serde_json::from_slice(bytes).map_err(|source| EvidenceError::InvalidRecord {
                path: "attempts/0001/resources.json".to_owned(),
                source,
            })?;
        validate_attempt_resources(&resources, manifest, &plan)?;
    }
    let validity: ValidityResult = parse_record(records, "validity.json")?;
    let expected_validity = match manifest.run_status {
        RunState::Succeeded
            if measurement_validation.measured_records
                >= plan.experiment.spec.measurement.repetitions =>
        {
            ValidityStatus::Valid
        }
        RunState::Succeeded => ValidityStatus::Invalid,
        RunState::Failed | RunState::Interrupted => ValidityStatus::Indeterminate,
        _ => unreachable!("manifest status was validated as terminal"),
    };
    if validity.run_id != manifest.run_id
        || validity.status != manifest.validity_status
        || validity.status != expected_validity
        || validity.required_samples != plan.experiment.spec.measurement.repetitions
        || validity.observed_samples != measurement_validation.measured_records
    {
        return Err(EvidenceError::InconsistentRecord(
            "validity.json".to_owned(),
        ));
    }
    let summary: RunSummary = parse_record(records, "summary.json")?;
    if summary.run_id != manifest.run_id
        || summary.run_status != manifest.run_status
        || summary.validity_status != manifest.validity_status
        || summary.attempt_count != 1
        || summary.sample_count != measurement_validation.measured_records
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

struct MeasurementValidation {
    measured_records: u32,
    generator: Option<String>,
}

fn validate_resolved_plan(
    plan: &ResolvedExperiment,
    manifest: &EvidenceManifest,
) -> Result<(), EvidenceError> {
    let expected = crate::resolution::resolve_experiment(plan.experiment.clone())
        .map_err(|error| EvidenceError::InvalidResolvedPlan(error.to_string()))?;
    if &expected != plan
        || plan.experiment_digest != manifest.experiment_digest
        || plan.resolved_plan_digest != manifest.resolved_plan_digest
    {
        return Err(EvidenceError::InvalidResolvedPlan(
            "typed content or deterministic digests do not match the bundle".to_owned(),
        ));
    }
    Ok(())
}

fn validate_measurements(
    root: &Path,
    expected_digest: &str,
    plan: &ResolvedExperiment,
    run_status: RunState,
) -> Result<MeasurementValidation, EvidenceError> {
    let relative = Path::new("attempts/0001/measurements.jsonl");
    let display = "attempts/0001/measurements.jsonl";
    let path = checked_regular_path(root, relative, display)?;
    let size = fs::metadata(&path)
        .map_err(|source| EvidenceError::Read {
            path: path.display().to_string(),
            source,
        })?
        .len();
    if size > MAX_MEASUREMENTS_BYTES {
        return invalid_measurements(&format!(
            "{display} exceeds the {MAX_MEASUREMENTS_BYTES}-byte parsing limit"
        ));
    }

    let (expected_warmups, expected_measured) = expected_measurement_counts(plan, run_status)?;
    let expected_total = expected_warmups
        .checked_add(expected_measured)
        .ok_or_else(|| EvidenceError::InvalidMeasurements("record count overflowed".to_owned()))?;
    let file = File::open(&path).map_err(|source| EvidenceError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut line = Vec::new();
    let mut total_bytes = 0_u64;
    let mut position = 0_u32;
    let mut generator: Option<String> = None;

    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|source| EvidenceError::Read {
                path: path.display().to_string(),
                source,
            })?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes
            .checked_add(read as u64)
            .ok_or_else(|| EvidenceError::InvalidMeasurements("file size overflowed".to_owned()))?;
        if total_bytes > MAX_MEASUREMENTS_BYTES {
            return invalid_measurements("measurement payload exceeds its parsing limit");
        }
        if read > MAX_MEASUREMENT_LINE_BYTES {
            return invalid_measurements("measurement record exceeds its line bound");
        }
        hasher.update(&line);
        if line.last() != Some(&b'\n') {
            return invalid_measurements("measurement JSONL ends with a truncated record");
        }
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.is_empty() || position >= expected_total {
            return invalid_measurements("measurement sequence has an empty or extra record");
        }
        let record: MeasurementRecord =
            serde_json::from_slice(&line).map_err(|source| EvidenceError::InvalidRecord {
                path: display.to_owned(),
                source,
            })?;
        validate_measurement_record(
            &record,
            position,
            expected_warmups,
            plan,
            generator.as_deref(),
        )?;
        if generator.is_none() {
            generator = Some(record.generator.clone());
        }
        position += 1;
    }

    if hex::encode(hasher.finalize()) != expected_digest {
        return Err(EvidenceError::ChecksumMismatch(display.to_owned()));
    }
    if position != expected_total {
        return invalid_measurements(&format!(
            "measurement sequence has {position} records; expected {expected_total}"
        ));
    }
    Ok(MeasurementValidation {
        measured_records: expected_measured,
        generator,
    })
}

fn expected_measurement_counts(
    plan: &ResolvedExperiment,
    run_status: RunState,
) -> Result<(u32, u32), EvidenceError> {
    let measurement = &plan.experiment.spec.measurement;
    match (
        &plan.experiment.spec.provider,
        &plan.experiment.spec.runtime,
    ) {
        (ProviderSpec::LocalFake, RuntimeSpec::LocalFake { scenario, .. }) => {
            let (expected_status, measured) = match scenario {
                LocalFakeScenario::Success => (RunState::Succeeded, measurement.repetitions),
                LocalFakeScenario::InsufficientMeasurements => (
                    RunState::Succeeded,
                    measurement.repetitions.saturating_sub(1),
                ),
                LocalFakeScenario::RuntimeFailure => (
                    RunState::Failed,
                    measurement.repetitions.saturating_sub(1).min(1),
                ),
                LocalFakeScenario::Interrupted => (
                    RunState::Interrupted,
                    measurement.repetitions.saturating_sub(1).min(1),
                ),
            };
            if run_status != expected_status {
                return invalid_measurements("localFake scenario does not match run status");
            }
            Ok((measurement.warmup_runs, measured))
        }
        (ProviderSpec::Local, RuntimeSpec::CpuProbe { .. } | RuntimeSpec::LlamaCpp { .. }) => {
            match run_status {
                RunState::Succeeded => Ok((measurement.warmup_runs, measurement.repetitions)),
                RunState::Failed => Ok((0, 0)),
                _ => invalid_measurements("helper-backed run has an unsupported terminal status"),
            }
        }
        _ => invalid_measurements("resolved provider/runtime combination is not executable"),
    }
}

fn validate_measurement_record(
    record: &MeasurementRecord,
    position: u32,
    warmup_count: u32,
    plan: &ResolvedExperiment,
    prior_generator: Option<&str>,
) -> Result<(), EvidenceError> {
    let (phase, repetition_index) = if position < warmup_count {
        (MeasurementPhase::Warmup, position + 1)
    } else {
        (MeasurementPhase::Measured, position - warmup_count + 1)
    };
    if record.attempt_number != 1
        || record.phase != phase
        || record.repetition_index != repetition_index
        || record.sample_index != 1
        || record.latency_micros == 0
        || record.time_to_first_token_micros == 0
        || record.time_to_first_token_micros > record.latency_micros
        || record.throughput_milli_requests_per_second == 0
        || record.successful_requests != plan.experiment.spec.workload.requests
        || record.failed_requests != 0
        || prior_generator.is_some_and(|generator| generator != record.generator)
    {
        return invalid_measurements(&format!(
            "aggregate record {} does not match the resolved execution",
            position + 1
        ));
    }

    match &plan.experiment.spec.runtime {
        RuntimeSpec::LocalFake { .. } => {
            if record.generator != LOCAL_FAKE_GENERATOR_VERSION
                || !record.request_observations.is_empty()
            {
                return invalid_measurements("localFake measurement contract is not supported");
            }
        }
        RuntimeSpec::CpuProbe { .. } => {
            if record.generator != CPU_PROBE_GENERATOR_VERSION
                || !record.request_observations.is_empty()
            {
                return invalid_measurements("cpuProbe measurement contract is not supported");
            }
        }
        RuntimeSpec::LlamaCpp { .. } => match record.generator.as_str() {
            LLAMA_CPP_GENERATOR_VERSION_V1 if record.request_observations.is_empty() => {}
            LLAMA_CPP_GENERATOR_VERSION => {
                if record.request_observations.len()
                    != plan.experiment.spec.workload.requests as usize
                {
                    return invalid_measurements(
                        "llamaCpp v2 request-observation cardinality does not match workload.requests",
                    );
                }
                for (index, observation) in record.request_observations.iter().enumerate() {
                    if observation.request_index != index as u32 + 1
                        || observation.latency_micros == 0
                        || observation.time_to_first_token_micros == 0
                        || observation.time_to_first_token_micros > observation.latency_micros
                    {
                        return invalid_measurements(
                            "llamaCpp v2 request observation is out of order or invalid",
                        );
                    }
                }
                validate_llama_v2_aggregate_consistency(record)?;
            }
            _ => return invalid_measurements("llamaCpp measurement generator is not supported"),
        },
        RuntimeSpec::Vllm { .. } => {
            return invalid_measurements("vLLM execution evidence is not supported")
        }
    }
    Ok(())
}

fn validate_llama_v2_aggregate_consistency(
    record: &MeasurementRecord,
) -> Result<(), EvidenceError> {
    let latency_bounds = rounded_request_mean_bounds(
        record
            .request_observations
            .iter()
            .map(|observation| observation.latency_micros),
    )
    .ok_or_else(|| {
        EvidenceError::InvalidMeasurements(
            "llamaCpp v2 latency aggregate bounds are not representable".to_owned(),
        )
    })?;
    let ttft_bounds = rounded_request_mean_bounds(
        record
            .request_observations
            .iter()
            .map(|observation| observation.time_to_first_token_micros),
    )
    .ok_or_else(|| {
        EvidenceError::InvalidMeasurements(
            "llamaCpp v2 TTFT aggregate bounds are not representable".to_owned(),
        )
    })?;
    if !(latency_bounds.0..=latency_bounds.1).contains(&record.latency_micros)
        || !(ttft_bounds.0..=ttft_bounds.1).contains(&record.time_to_first_token_micros)
    {
        return invalid_measurements(
            "llamaCpp v2 aggregate latency or TTFT contradicts its request observations",
        );
    }
    Ok(())
}

fn rounded_request_mean_bounds(mut values: impl Iterator<Item = u64>) -> Option<(u64, u64)> {
    let (count, rounded_micros) =
        values.try_fold((0_u128, 0_u128), |(count, total), value| -> Option<_> {
            Some((count.checked_add(1)?, total.checked_add(value.into())?))
        })?;
    if count == 0 || rounded_micros < count {
        return None;
    }

    // For an emitted observation m = ceil(raw_nanos / 1000), raw_nanos is in
    // ((m - 1) * 1000, m * 1000]. Summing those exact integer-nanosecond
    // intervals gives the tight range for the producer's ceil-of-the-raw-mean.
    let minimum_raw_nanos = rounded_micros
        .checked_sub(count)?
        .checked_mul(1000)?
        .checked_add(count)?;
    let maximum_raw_nanos = rounded_micros.checked_mul(1000)?;
    let divisor = count.checked_mul(1000)?;
    let minimum = ceil_div_u128(minimum_raw_nanos, divisor);
    let maximum = ceil_div_u128(maximum_raw_nanos, divisor);
    Some((minimum.try_into().ok()?, maximum.try_into().ok()?))
}

fn ceil_div_u128(dividend: u128, divisor: u128) -> u128 {
    dividend / divisor + u128::from(!dividend.is_multiple_of(divisor))
}

fn validate_attempt_resources(
    resources: &AttemptResources,
    manifest: &EvidenceManifest,
    plan: &ResolvedExperiment,
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
    if resources.scope != ResourceScope::HelperProcessLifetime
        || !matches!(
            (
                &plan.experiment.spec.provider,
                &plan.experiment.spec.runtime
            ),
            (
                ProviderSpec::Local,
                RuntimeSpec::CpuProbe { .. } | RuntimeSpec::LlamaCpp { .. }
            )
        )
    {
        return invalid_resources("resource scope does not apply to the resolved runtime");
    }
    Ok(())
}

fn validate_attempt_provenance(
    provenance: &AttemptProvenance,
    manifest: &EvidenceManifest,
    plan: &ResolvedExperiment,
    measurement_generator: Option<&str>,
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

    match (&provenance.software.runtime, &plan.experiment.spec.runtime) {
        (RuntimeProvenance::LocalFake { generator }, RuntimeSpec::LocalFake { .. }) => {
            if generator != LOCAL_FAKE_GENERATOR_VERSION {
                return invalid_provenance("localFake generator identity is not supported");
            }
            require_matching_generator(generator, measurement_generator)?;
        }
        (RuntimeProvenance::CpuProbe { generator }, RuntimeSpec::CpuProbe { .. }) => {
            if generator != CPU_PROBE_GENERATOR_VERSION {
                return invalid_provenance("cpuProbe generator identity is not supported");
            }
            require_matching_generator(generator, measurement_generator)?;
        }
        (
            RuntimeProvenance::LlamaCpp {
                generator,
                engine,
                model,
                backend,
            },
            RuntimeSpec::LlamaCpp {
                model: resolved_model,
                ..
            },
        ) => {
            if !matches!(
                generator.as_str(),
                LLAMA_CPP_GENERATOR_VERSION_V1 | LLAMA_CPP_GENERATOR_VERSION
            ) || engine.name != LLAMA_CPP_ENGINE_NAME
                || engine.version != LLAMA_CPP_ENGINE_VERSION
                || model.identity != LLAMA_CPP_MODEL_IDENTITY
                || &model.identity != resolved_model
                || model.sha256 != LLAMA_CPP_MODEL_SHA256
                || backend.identity != LLAMA_CPP_BACKEND_IDENTITY
                || backend.device_class != DeviceClass::Cpu
            {
                return invalid_provenance("llamaCpp software lineage is not supported");
            }
            require_matching_generator(generator, measurement_generator)?;
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
        _ => return invalid_provenance("runtime kind does not match the resolved plan"),
    }
    Ok(())
}

fn require_matching_generator(
    provenance_generator: &str,
    measurement_generator: Option<&str>,
) -> Result<(), EvidenceError> {
    if measurement_generator.is_some_and(|generator| generator != provenance_generator) {
        return invalid_provenance("runtime generator does not match measurements");
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

fn invalid_measurements<T>(message: &str) -> Result<T, EvidenceError> {
    Err(EvidenceError::InvalidMeasurements(message.to_owned()))
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
    use benchplane_schema::{
        Experiment, RequestObservation, RunState, ValidityStatus, EVIDENCE_FORMAT_V1,
    };
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
                    "kind": "localFake",
                    "generator": LOCAL_FAKE_GENERATOR_VERSION
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

    fn llama_plan(requests: u32) -> ResolvedExperiment {
        let experiment: Experiment = serde_json::from_value(serde_json::json!({
            "apiVersion": "benchplane/v1alpha1",
            "kind": "Experiment",
            "metadata": { "name": "llama-evidence" },
            "spec": {
                "provider": { "kind": "local" },
                "runtime": {
                    "kind": "llamaCpp",
                    "model": LLAMA_CPP_MODEL_IDENTITY,
                    "outputTokens": 4
                },
                "workload": {
                    "profile": "smollm2-chat-greedy-v1",
                    "requests": requests,
                    "concurrency": 1
                },
                "measurement": { "warmupRuns": 1, "repetitions": 2 },
                "budget": { "maximumCostUsd": 0 },
                "lifecycle": { "maximumRuntimeSeconds": 120 }
            }
        }))
        .expect("llama experiment");
        crate::resolution::resolve_experiment(experiment).expect("resolve llama experiment")
    }

    fn llama_measurement(
        generator: &str,
        phase: MeasurementPhase,
        repetition_index: u32,
        requests: u32,
    ) -> MeasurementRecord {
        let request_observations = if generator == LLAMA_CPP_GENERATOR_VERSION {
            (1..=requests)
                .map(|request_index| RequestObservation {
                    request_index,
                    latency_micros: 20,
                    time_to_first_token_micros: 10,
                })
                .collect()
        } else {
            Vec::new()
        };
        MeasurementRecord {
            generator: generator.to_owned(),
            attempt_number: 1,
            phase,
            repetition_index,
            sample_index: 1,
            latency_micros: 20,
            time_to_first_token_micros: 10,
            throughput_milli_requests_per_second: 1_000,
            successful_requests: requests,
            failed_requests: 0,
            request_observations,
        }
    }

    fn write_measurements(root: &Path, measurements: &[MeasurementRecord]) {
        let mut bytes = Vec::new();
        for measurement in measurements {
            serde_json::to_writer(&mut bytes, measurement).expect("serialize measurement");
            bytes.push(b'\n');
        }
        fs::write(root.join("attempts/0001/measurements.jsonl"), bytes)
            .expect("write measurements");
    }

    fn llama_provenance(generator: &str) -> AttemptProvenance {
        let mut provenance = valid_provenance();
        provenance.software.runtime = serde_json::from_value(serde_json::json!({
            "kind": "llamaCpp",
            "generator": generator,
            "engine": {
                "name": LLAMA_CPP_ENGINE_NAME,
                "version": LLAMA_CPP_ENGINE_VERSION,
                "nixStorePath": null
            },
            "model": {
                "identity": LLAMA_CPP_MODEL_IDENTITY,
                "sha256": LLAMA_CPP_MODEL_SHA256,
                "nixStorePath": null
            },
            "backend": {
                "identity": LLAMA_CPP_BACKEND_IDENTITY,
                "deviceClass": "cpu",
                "nixStorePath": null
            }
        }))
        .expect("llama provenance runtime");
        provenance
    }

    fn llama_fixture(label: &str, generator: &str) -> TestDirectory {
        llama_fixture_with_requests(label, generator, 2)
    }

    fn llama_fixture_with_requests(label: &str, generator: &str, requests: u32) -> TestDirectory {
        let directory = copy_fixture(label);
        let plan = llama_plan(requests);
        fs::write(
            directory.path.join("resolved-plan.json"),
            serde_json::to_vec_pretty(&plan).expect("serialize plan"),
        )
        .expect("write plan");

        let mut manifest: EvidenceManifest = serde_json::from_slice(
            &fs::read(directory.path.join("manifest.json")).expect("read manifest"),
        )
        .expect("parse manifest");
        manifest.experiment_digest = plan.experiment_digest.clone();
        manifest.resolved_plan_digest = plan.resolved_plan_digest.clone();
        fs::write(
            directory.path.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let mut run: RunRecord =
            serde_json::from_slice(&fs::read(directory.path.join("run.json")).expect("read run"))
                .expect("parse run");
        run.experiment_digest = plan.experiment_digest.clone();
        run.resolved_plan_digest = plan.resolved_plan_digest.clone();
        fs::write(
            directory.path.join("run.json"),
            serde_json::to_vec_pretty(&run).expect("serialize run"),
        )
        .expect("write run");

        let mut summary: RunSummary = serde_json::from_slice(
            &fs::read(directory.path.join("summary.json")).expect("read summary"),
        )
        .expect("parse summary");
        summary.sample_count = 2;
        summary.experiment_digest = plan.experiment_digest.clone();
        summary.resolved_plan_digest = plan.resolved_plan_digest.clone();
        fs::write(
            directory.path.join("summary.json"),
            serde_json::to_vec_pretty(&summary).expect("serialize summary"),
        )
        .expect("write summary");

        let mut validity: ValidityResult = serde_json::from_slice(
            &fs::read(directory.path.join("validity.json")).expect("read validity"),
        )
        .expect("parse validity");
        validity.required_samples = 2;
        validity.observed_samples = 2;
        fs::write(
            directory.path.join("validity.json"),
            serde_json::to_vec_pretty(&validity).expect("serialize validity"),
        )
        .expect("write validity");

        write_measurements(
            &directory.path,
            &[
                llama_measurement(generator, MeasurementPhase::Warmup, 1, requests),
                llama_measurement(generator, MeasurementPhase::Measured, 1, requests),
                llama_measurement(generator, MeasurementPhase::Measured, 2, requests),
            ],
        );
        write_provenance(&directory.path, &llama_provenance(generator));
        write_resources(&directory.path, &valid_resources());
        rewrite_checksums(&directory.path);
        directory
    }

    fn read_measurements(root: &Path) -> Vec<MeasurementRecord> {
        fs::read_to_string(root.join("attempts/0001/measurements.jsonl"))
            .expect("read measurements")
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse measurement"))
            .collect()
    }

    #[test]
    fn verifies_the_checked_in_fixture() {
        let manifest = verify_evidence_bundle(&fixture_root()).expect("fixture should verify");
        assert_eq!(manifest.format, EVIDENCE_FORMAT_V1);
    }

    #[test]
    fn verifies_historical_and_current_llama_measurement_lineage() {
        for (label, generator) in [
            ("v1", LLAMA_CPP_GENERATOR_VERSION_V1),
            ("v2", LLAMA_CPP_GENERATOR_VERSION),
        ] {
            let directory = llama_fixture(&format!("llama-{label}"), generator);
            verify_evidence_bundle(&directory.path).expect("known llama lineage should verify");
            let records = read_measurements(&directory.path);
            assert_eq!(records.len(), 3);
            let observations: usize = records
                .iter()
                .map(|record| record.request_observations.len())
                .sum();
            assert_eq!(
                observations,
                if generator == LLAMA_CPP_GENERATOR_VERSION {
                    6
                } else {
                    0
                }
            );
        }
    }

    #[test]
    fn verifies_single_and_multiple_request_llama_v2_repetitions() {
        for requests in [1, 2] {
            let directory = llama_fixture_with_requests(
                &format!("llama-v2-{requests}-requests"),
                LLAMA_CPP_GENERATOR_VERSION,
                requests,
            );
            verify_evidence_bundle(&directory.path).expect("llama v2 evidence should verify");
            let records = read_measurements(&directory.path);
            assert!(records.iter().all(|record| {
                record.request_observations.len() == requests as usize
                    && record.latency_micros == 20
                    && record.time_to_first_token_micros == 10
            }));
        }
    }

    #[test]
    fn rejects_rechecksummed_llama_v2_aggregate_mismatches() {
        for (label, latency, ttft) in [("latency", 100_u64, 10_u64), ("ttft", 20, 19)] {
            let directory = llama_fixture(
                &format!("llama-v2-{label}-aggregate-mismatch"),
                LLAMA_CPP_GENERATOR_VERSION,
            );
            let mut records = read_measurements(&directory.path);
            records[0].latency_micros = latency;
            records[0].time_to_first_token_micros = ttft;
            write_measurements(&directory.path, &records);
            rewrite_checksums(&directory.path);
            assert!(matches!(
                verify_evidence_bundle(&directory.path),
                Err(EvidenceError::InvalidMeasurements(_))
            ));
        }
    }

    #[test]
    fn llama_v2_aggregate_rounding_interval_is_tight() {
        for (aggregate, accepted) in [(2_u64, true), (3, true), (1, false), (4, false)] {
            let directory = llama_fixture(
                &format!("llama-v2-rounding-boundary-{aggregate}"),
                LLAMA_CPP_GENERATOR_VERSION,
            );
            let mut records = read_measurements(&directory.path);
            records[0].request_observations = vec![
                RequestObservation {
                    request_index: 1,
                    latency_micros: 2,
                    time_to_first_token_micros: 1,
                },
                RequestObservation {
                    request_index: 2,
                    latency_micros: 3,
                    time_to_first_token_micros: 1,
                },
            ];
            records[0].latency_micros = aggregate;
            records[0].time_to_first_token_micros = 1;
            write_measurements(&directory.path, &records);
            rewrite_checksums(&directory.path);
            assert_eq!(verify_evidence_bundle(&directory.path).is_ok(), accepted);
        }
    }

    #[test]
    fn rejects_invalid_llama_request_observation_sequences() {
        for mode in ["missing", "extra", "order", "zero", "ttft"] {
            let directory = llama_fixture(
                &format!("llama-observation-{mode}"),
                LLAMA_CPP_GENERATOR_VERSION,
            );
            let mut records = read_measurements(&directory.path);
            match mode {
                "missing" => {
                    records[0].request_observations.pop();
                }
                "extra" => {
                    records[0].request_observations.push(RequestObservation {
                        request_index: 3,
                        latency_micros: 22,
                        time_to_first_token_micros: 12,
                    });
                }
                "order" => {
                    records[0].request_observations.swap(0, 1);
                }
                "zero" => {
                    records[0].request_observations[0].latency_micros = 0;
                }
                "ttft" => {
                    records[0].request_observations[0].time_to_first_token_micros = 20;
                    records[0].request_observations[0].latency_micros = 19;
                }
                _ => unreachable!(),
            };
            write_measurements(&directory.path, &records);
            rewrite_checksums(&directory.path);
            assert!(
                matches!(
                    verify_evidence_bundle(&directory.path),
                    Err(EvidenceError::InvalidMeasurements(_))
                ),
                "mode={mode}"
            );
        }
    }

    #[test]
    fn rejects_missing_extra_malformed_and_mismatched_llama_measurements() {
        for mode in ["missing-record", "extra-record", "phase", "generator"] {
            let directory = llama_fixture(
                &format!("llama-measurement-{mode}"),
                LLAMA_CPP_GENERATOR_VERSION,
            );
            let mut records = read_measurements(&directory.path);
            match mode {
                "missing-record" => {
                    records.pop();
                }
                "extra-record" => records.push(llama_measurement(
                    LLAMA_CPP_GENERATOR_VERSION,
                    MeasurementPhase::Measured,
                    3,
                    2,
                )),
                "phase" => records[0].phase = MeasurementPhase::Measured,
                "generator" => records[0].generator = CPU_PROBE_GENERATOR_VERSION.to_owned(),
                _ => unreachable!(),
            }
            write_measurements(&directory.path, &records);
            rewrite_checksums(&directory.path);
            assert!(
                matches!(
                    verify_evidence_bundle(&directory.path),
                    Err(EvidenceError::InvalidMeasurements(_))
                ),
                "mode={mode}"
            );
        }

        let malformed = llama_fixture("llama-malformed-measurement", LLAMA_CPP_GENERATOR_VERSION);
        fs::write(
            malformed.path.join("attempts/0001/measurements.jsonl"),
            b"{not-json}\n",
        )
        .expect("write malformed measurement");
        rewrite_checksums(&malformed.path);
        assert!(matches!(
            verify_evidence_bundle(&malformed.path),
            Err(EvidenceError::InvalidRecord { path, .. })
                if path == "attempts/0001/measurements.jsonl"
        ));
    }

    #[test]
    fn rejects_resolved_plan_provenance_and_resource_runtime_mismatches() {
        let provenance_mismatch = llama_fixture(
            "llama-provenance-runtime-mismatch",
            LLAMA_CPP_GENERATOR_VERSION,
        );
        let mut provenance: AttemptProvenance = serde_json::from_slice(
            &fs::read(
                provenance_mismatch
                    .path
                    .join("attempts/0001/provenance.json"),
            )
            .expect("read provenance"),
        )
        .expect("parse provenance");
        provenance.software.runtime = RuntimeProvenance::CpuProbe {
            generator: CPU_PROBE_GENERATOR_VERSION.to_owned(),
        };
        write_provenance(&provenance_mismatch.path, &provenance);
        rewrite_checksums(&provenance_mismatch.path);
        assert!(matches!(
            verify_evidence_bundle(&provenance_mismatch.path),
            Err(EvidenceError::InvalidAttemptProvenance(_))
        ));

        let plan_mismatch = llama_fixture("llama-plan-mismatch", LLAMA_CPP_GENERATOR_VERSION);
        let mut plan: ResolvedExperiment = serde_json::from_slice(
            &fs::read(plan_mismatch.path.join("resolved-plan.json")).expect("read plan"),
        )
        .expect("parse plan");
        plan.experiment.spec.workload.requests = 3;
        fs::write(
            plan_mismatch.path.join("resolved-plan.json"),
            serde_json::to_vec_pretty(&plan).expect("serialize mismatched plan"),
        )
        .expect("write mismatched plan");
        rewrite_checksums(&plan_mismatch.path);
        assert!(matches!(
            verify_evidence_bundle(&plan_mismatch.path),
            Err(EvidenceError::InvalidResolvedPlan(_))
        ));
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
    fn rejects_helper_resources_for_local_fake_plan() {
        let directory = copy_fixture("attempt-resources");
        write_resources(&directory.path, &valid_resources());
        rewrite_checksums(&directory.path);

        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::InvalidAttemptResources(message))
                if message.contains("does not apply")
        ));
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
    fn rejects_rechecksummed_malformed_measurement_payload() {
        let directory = copy_fixture("streamed-payload");
        fs::write(
            directory.path.join("attempts/0001/measurements.jsonl"),
            vec![b'x'; 2 * 1024 * 1024],
        )
        .expect("write large measurement payload");
        rewrite_checksums(&directory.path);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::InvalidMeasurements(_))
        ));
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
