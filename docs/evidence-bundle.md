# Evidence bundle

The initial executable evidence format is `benchplane-evidence/v1`:

```text
<run-id>/
├── experiment.yaml
├── resolved-plan.json
├── run.json
├── events.jsonl
├── attempts/
│   └── 0001/
│       ├── attempt.json
│       └── measurements.jsonl
├── validity.json
├── summary.json
├── manifest.json
└── SHA256SUMS
```

The original `experiment.yaml` bytes are preserved exactly. `resolved-plan.json`, lifecycle snapshots, the event journal, and measurements are written incrementally. Validity, summary, manifest, and checksums exist only after finalization.

The manifest records typed run and validity statuses, run ID, experiment and resolved-plan digests, and attempt count. `SHA256SUMS` covers the manifest and every regular payload recursively, but never itself. Paths are normalized and sorted lexicographically.

## Finalization and publication

Benchplane writes under `<output-root>/staging/<run-id>`, persists the terminal event and snapshots, writes finalization-only records, rejects symlinks and unsupported file types, writes checksums through `SHA256SUMS.tmp`, and verifies the complete staging bundle with the public verifier. Only then does it atomically rename the directory to `<output-root>/runs/<run-id>` on the same filesystem.

Finalized bundles are immutable. A partial staging directory is not an evidence bundle. If finalization, verification, or publication fails, no final directory is created and the staging directory is retained; failure details are updated best-effort because the underlying I/O failure may also prevent further persistence.

After publication, the CLI returns an `evidenceDigest` calculated from the exact `SHA256SUMS` bytes. It is intentionally not stored inside the checksummed bundle.

Verification parses `manifest.json` from the exact bytes that passed checksum verification. It rejects malformed or duplicate checksums, missing or extra payloads, absolute or traversing paths, symlinks and canonical-path escapes, non-regular files, unsupported formats, invalid identities, and empty required manifest fields.

Checksums establish bundle integrity relative to `SHA256SUMS`; they do not establish who published the bundle or whether the checksum file itself came from a trusted source. Publisher authenticity requires a separate trust mechanism.
