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
    #[error("evidence payload is not a regular file: {0}")]
    NonRegularFile(String),
    #[error("SHA256SUMS must contain a checksum for manifest.json")]
    MissingManifestChecksum,
    #[error("checksum mismatch for {0}")]
    ChecksumMismatch(String),
    #[error("unsupported evidence format: {0}")]
    UnsupportedFormat(String),
    #[error("manifest field {0} must not be empty")]
    EmptyManifestField(&'static str),
    #[error("manifest experimentDigest must be a lowercase sha256 digest")]
    InvalidExperimentDigest,
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
    let manifest: EvidenceManifest =
        serde_json::from_slice(manifest_bytes).map_err(EvidenceError::Manifest)?;
    validate_manifest(&manifest)?;

    Ok(manifest)
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
        return Err(EvidenceError::NonRegularFile(root.display().to_string()));
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
    if manifest.status.trim().is_empty() {
        return Err(EvidenceError::EmptyManifestField("status"));
    }
    if !is_sha256_digest(&manifest.experiment_digest) {
        return Err(EvidenceError::InvalidExperimentDigest);
    }
    Ok(())
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
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_nanos();
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "benchplane-{label}-{}-{nonce}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
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

    fn copy_fixture(label: &str) -> TestDirectory {
        let directory = TestDirectory::new(label);
        for name in ["manifest.json", "summary.json", "SHA256SUMS"] {
            fs::copy(fixture_root().join(name), directory.path.join(name))
                .expect("copy evidence fixture");
        }
        directory
    }

    fn write_checksums(root: &Path, names: &[&str]) {
        let mut contents = String::new();
        for name in names {
            let bytes = fs::read(root.join(name)).expect("read checksummed file");
            contents.push_str(&format!("{}  {name}\n", hex::encode(Sha256::digest(bytes))));
        }
        fs::write(root.join("SHA256SUMS"), contents).expect("write checksums");
    }

    fn rewrite_manifest(root: &Path, manifest: &EvidenceManifest) {
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        write_checksums(root, &["manifest.json", "summary.json"]);
    }

    fn valid_manifest() -> EvidenceManifest {
        EvidenceManifest {
            format: EVIDENCE_FORMAT_V1.to_owned(),
            run_id: "local-fake-0001".to_owned(),
            experiment_digest: format!("sha256:{}", "a".repeat(64)),
            status: "complete".to_owned(),
        }
    }

    #[test]
    fn verifies_the_checked_in_fixture() {
        let manifest = verify_evidence_bundle(&fixture_root()).expect("fixture should verify");
        assert_eq!(manifest.format, EVIDENCE_FORMAT_V1);
    }

    #[test]
    fn rejects_modified_payload() {
        let directory = copy_fixture("modified-payload");
        fs::write(directory.path.join("summary.json"), b"{}\n").expect("modify summary");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::ChecksumMismatch(path)) if path == "summary.json"
        ));
    }

    #[test]
    fn rejects_modified_manifest() {
        let directory = copy_fixture("modified-manifest");
        fs::write(directory.path.join("manifest.json"), b"{}\n").expect("modify manifest");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::ChecksumMismatch(path)) if path == "manifest.json"
        ));
    }

    #[test]
    fn rejects_malformed_checksum_line() {
        let directory = copy_fixture("malformed-line");
        fs::write(directory.path.join("SHA256SUMS"), b"not-a-checksum-line\n")
            .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::InvalidChecksumLine(_))
        ));
    }

    #[test]
    fn rejects_invalid_checksum_digest_text() {
        let directory = copy_fixture("invalid-checksum");
        fs::write(
            directory.path.join("SHA256SUMS"),
            format!("{}  manifest.json\n", "a".repeat(63)),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::InvalidChecksum(path)) if path == "manifest.json"
        ));
    }

    #[test]
    fn rejects_duplicate_path() {
        let directory = copy_fixture("duplicate-path");
        let manifest = fs::read(directory.path.join("manifest.json")).expect("read manifest");
        let digest = hex::encode(Sha256::digest(manifest));
        fs::write(
            directory.path.join("SHA256SUMS"),
            format!("{digest}  manifest.json\n{digest}  manifest.json\n"),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::DuplicatePath(path)) if path == "manifest.json"
        ));
    }

    #[test]
    fn rejects_missing_manifest_checksum() {
        let directory = copy_fixture("missing-manifest");
        write_checksums(&directory.path, &["summary.json"]);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::MissingManifestChecksum)
        ));
    }

    #[test]
    fn rejects_parent_traversal() {
        let directory = copy_fixture("parent-traversal");
        fs::write(
            directory.path.join("SHA256SUMS"),
            format!("{}  ../manifest.json\n", "0".repeat(64)),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::UnsafePath(path)) if path == "../manifest.json"
        ));
    }

    #[test]
    fn rejects_absolute_path() {
        let directory = copy_fixture("absolute-path");
        fs::write(
            directory.path.join("SHA256SUMS"),
            format!("{}  /tmp/manifest.json\n", "0".repeat(64)),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::UnsafePath(path)) if path == "/tmp/manifest.json"
        ));
    }

    #[test]
    fn rejects_sha256sums_as_payload() {
        let directory = copy_fixture("self-checksum");
        fs::write(
            directory.path.join("SHA256SUMS"),
            format!("{}  SHA256SUMS\n", "0".repeat(64)),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::UnsafePath(path)) if path == "SHA256SUMS"
        ));
    }

    #[test]
    fn rejects_unsupported_evidence_format() {
        let directory = copy_fixture("unsupported-format");
        let mut manifest = valid_manifest();
        manifest.format = "benchplane-evidence/v2".to_owned();
        rewrite_manifest(&directory.path, &manifest);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::UnsupportedFormat(format)) if format == "benchplane-evidence/v2"
        ));
    }

    #[test]
    fn rejects_empty_required_manifest_field() {
        let directory = copy_fixture("empty-field");
        let mut manifest = valid_manifest();
        manifest.run_id = "  ".to_owned();
        rewrite_manifest(&directory.path, &manifest);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::EmptyManifestField("runId"))
        ));
    }

    #[test]
    fn rejects_invalid_experiment_digest() {
        let directory = copy_fixture("invalid-experiment-digest");
        let mut manifest = valid_manifest();
        manifest.experiment_digest = "sha256:not-a-digest".to_owned();
        rewrite_manifest(&directory.path, &manifest);
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::InvalidExperimentDigest)
        ));
    }

    #[test]
    fn rejects_non_regular_payload() {
        let directory = copy_fixture("non-regular");
        fs::create_dir(directory.path.join("payload")).expect("create payload directory");
        fs::write(
            directory.path.join("SHA256SUMS"),
            format!("{}  payload\n", "0".repeat(64)),
        )
        .expect("modify checksums");
        assert!(matches!(
            verify_evidence_bundle(&directory.path),
            Err(EvidenceError::NonRegularFile(path)) if path == "payload"
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
    fn validates_sha256_digest_shape() {
        assert!(is_sha256_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_sha256_digest(&format!("sha256:{}", "a".repeat(63))));
        assert!(!is_sha256_digest(&format!("sha256:{}", "A".repeat(64))));
        assert!(!is_sha256_digest(&format!("sha512:{}", "a".repeat(64))));
    }
}
