// SPDX-License-Identifier: Apache-2.0

use benchplane_schema::{EvidenceManifest, EVIDENCE_FORMAT_V1};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

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
    #[error("invalid manifest JSON: {0}")]
    Manifest(serde_json::Error),
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
    #[error("required evidence payload is missing: {0}")]
    MissingRequiredPayload(String),
    #[error("checksum mismatch for {0}")]
    ChecksumMismatch(String),
    #[error("unsupported evidence format: {0}")]
    UnsupportedFormat(String),
    #[error("manifest field {0} must not be empty")]
    EmptyManifestField(&'static str),
    #[error("manifest runId must be run-<lowercase UUIDv7>")]
    InvalidRunId,
    #[error("manifest runStatus must be terminal")]
    NonTerminalRunStatus,
    #[error("manifest attemptCount must be greater than zero")]
    InvalidAttemptCount,
    #[error("manifest experimentDigest must be a lowercase sha256 digest")]
    InvalidExperimentDigest,
    #[error("manifest resolvedPlanDigest must be a lowercase sha256 digest")]
    InvalidResolvedPlanDigest,
    #[error("SHA256SUMS already exists")]
    AlreadyFinalized,
}

pub fn verify_evidence_bundle(root: &Path) -> Result<EvidenceManifest, EvidenceError> {
    let root = canonical_bundle_root(root)?;
    let (_, sums_bytes) = read_regular_file(&root, Path::new("SHA256SUMS"), "SHA256SUMS")?;
    let sums =
        std::str::from_utf8(&sums_bytes).map_err(|_| EvidenceError::InvalidChecksumFileEncoding)?;

    let mut checked_names = BTreeSet::new();
    let mut checked_targets = BTreeSet::new();
    let mut verified_bytes = BTreeMap::new();

    for line in sums.lines().filter(|line| !line.trim().is_empty()) {
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
        if !checked_names.insert(relative.to_owned()) {
            return Err(EvidenceError::DuplicatePath(relative.to_owned()));
        }

        let (canonical_path, bytes) = read_regular_file(&root, relative_path, relative)?;
        if !checked_targets.insert(canonical_path) {
            return Err(EvidenceError::DuplicatePath(relative.to_owned()));
        }

        let actual = hex::encode(Sha256::digest(&bytes));
        if actual != expected {
            return Err(EvidenceError::ChecksumMismatch(relative.to_owned()));
        }
        verified_bytes.insert(relative.to_owned(), bytes);
    }

    let manifest_bytes = verified_bytes
        .get("manifest.json")
        .ok_or(EvidenceError::MissingManifestChecksum)?;

    let payloads = gather_payloads(&root)?;
    for relative in payloads.keys() {
        if !checked_names.contains(relative) {
            return Err(EvidenceError::MissingPayloadChecksum(relative.clone()));
        }
    }

    let manifest: EvidenceManifest =
        serde_json::from_slice(manifest_bytes).map_err(EvidenceError::Manifest)?;
    validate_manifest(&manifest)?;
    validate_required_payloads(&checked_names, manifest.attempt_count)?;

    Ok(manifest)
}

pub(crate) fn write_checksum_file(root: &Path) -> Result<Vec<u8>, EvidenceError> {
    if root.join("SHA256SUMS").exists() {
        return Err(EvidenceError::AlreadyFinalized);
    }

    let root = canonical_bundle_root(root)?;
    let payloads = gather_payloads(&root)?;
    let mut sums = Vec::new();
    for (relative, bytes) in payloads {
        let line = format!("{}  {relative}\n", hex::encode(Sha256::digest(bytes)));
        sums.extend_from_slice(line.as_bytes());
    }

    let temporary = root.join("SHA256SUMS.tmp");
    let final_path = root.join("SHA256SUMS");
    fs::write(&temporary, &sums).map_err(|source| EvidenceError::Write {
        path: temporary.display().to_string(),
        source,
    })?;
    fs::rename(&temporary, &final_path).map_err(|source| EvidenceError::Write {
        path: final_path.display().to_string(),
        source,
    })?;
    Ok(sums)
}

fn gather_payloads(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, EvidenceError> {
    let mut payloads = BTreeMap::new();
    gather_directory(root, root, &mut payloads)?;
    Ok(payloads)
}

fn gather_directory(
    root: &Path,
    directory: &Path,
    payloads: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), EvidenceError> {
    let entries = fs::read_dir(directory).map_err(|source| EvidenceError::Read {
        path: directory.display().to_string(),
        source,
    })?;
    for entry in entries {
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
            gather_directory(root, &path, payloads)?;
        } else if metadata.is_file() {
            let bytes = fs::read(&path).map_err(|source| EvidenceError::Read {
                path: path.display().to_string(),
                source,
            })?;
            payloads.insert(display, bytes);
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

fn read_regular_file(
    root: &Path,
    relative: &Path,
    display: &str,
) -> Result<(PathBuf, Vec<u8>), EvidenceError> {
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

    let bytes = fs::read(&canonical).map_err(|source| EvidenceError::Read {
        path: canonical.display().to_string(),
        source,
    })?;
    Ok((canonical, bytes))
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
    if manifest.attempt_count == 0 {
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

fn validate_required_payloads(
    names: &BTreeSet<String>,
    attempt_count: u32,
) -> Result<(), EvidenceError> {
    let mut required = vec![
        "experiment.yaml".to_owned(),
        "resolved-plan.json".to_owned(),
        "run.json".to_owned(),
        "events.jsonl".to_owned(),
        "validity.json".to_owned(),
        "summary.json".to_owned(),
        "manifest.json".to_owned(),
    ];
    for attempt in 1..=attempt_count {
        required.push(format!("attempts/{attempt:04}/attempt.json"));
        required.push(format!("attempts/{attempt:04}/measurements.jsonl"));
    }
    for required_path in required {
        if !names.contains(&required_path) {
            return Err(EvidenceError::MissingRequiredPayload(required_path));
        }
    }
    Ok(())
}

fn is_run_id(value: &str) -> bool {
    let Some(uuid_text) = value.strip_prefix("run-") else {
        return false;
    };
    if uuid_text != uuid_text.to_ascii_lowercase() {
        return false;
    }
    uuid::Uuid::parse_str(uuid_text)
        .ok()
        .is_some_and(|uuid| uuid.as_bytes()[6] >> 4 == 7)
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
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "benchplane-{label}-{}-{counter}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/evidence/local-fake")
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
        let directory = TestDirectory::new(label);
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

    #[test]
    fn verifies_the_checked_in_fixture() {
        let manifest = verify_evidence_bundle(&fixture_root()).expect("fixture should verify");
        assert_eq!(manifest.format, EVIDENCE_FORMAT_V1);
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
            Err(EvidenceError::PathEscape(path)) if path == "escape/payload"
        ));
    }

    #[test]
    fn validates_digest_and_run_id_shapes() {
        assert!(is_sha256_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_sha256_digest(&format!("sha256:{}", "a".repeat(63))));
        assert!(!is_sha256_digest(&format!("sha256:{}", "A".repeat(64))));
        assert!(is_run_id("run-018f6f9a-7b3c-7abc-8def-0123456789ab"));
        assert!(!is_run_id("run-018f6f9a-7b3c-4abc-8def-0123456789ab"));
    }
}
