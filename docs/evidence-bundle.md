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

Evidence format v1 contains exactly one attempt, so `attemptCount` must equal `1`. The `attempts/` directory is a stable ownership boundary, not a promise that retries exist. Future retry support requires an evidence-format evolution or an explicitly reviewed relaxation of this invariant.

For `benchplane-cpu-probe/v1`, each warmup or measured repetition contributes one aggregate `MeasurementRecord`. `latencyMicros` is mean completed-request latency, `timeToFirstTokenMicros` is mean first-output latency, `throughputMilliRequestsPerSecond` is successful requests divided by repetition wall time, and request counts report completed and failed requests. Summary latency and throughput continue to use only measured, non-warmup records; evidence-v1 gains no TTFT summary field. These timings are observed and nondeterministic even though the resolved plan and workload computation are deterministic.

CPU-probe measurements are committed to the lifecycle only when the helper exits successfully and its complete record sequence validates. Records parsed before a nonzero exit, deadline, malformed trailing record, or other protocol failure are discarded; failed evidence therefore has indeterminate validity and no retained partial CPU-probe measurement prefix.

## Finalization and publication

Benchplane writes under `<output-root>/staging/<run-id>`, persists the terminal event and snapshots, writes finalization-only records, rejects symlinks and unsupported file types, writes checksums through `SHA256SUMS.tmp`, and verifies the complete staging bundle with the public verifier. Only then does it atomically rename the directory to `<output-root>/runs/<run-id>` on the same filesystem.

For the NixOS runner, `<output-root>` is the systemd-managed state directory, `/var/lib/benchplane` by default. Published bundles therefore appear beneath `/var/lib/benchplane/runs`; in-progress or retained diagnostic state appears beneath `/var/lib/benchplane/staging`. The service applies a `0027` umask and requests mode `0750` for the state directory so bundles are not world-readable or world-writable.

Finalized bundles are immutable. A partial staging directory is not an evidence bundle. If finalization, verification, or publication fails, no final directory is created and the staging directory is retained; failure details are updated best-effort because the underlying I/O failure may also prevent further persistence. An operator may inspect or delete retained staging data, but must not publish or cite it as authoritative evidence.

Same-filesystem rename provides atomic namespace visibility: consumers do not observe a partially renamed final directory. The current implementation does not synchronize every payload and parent directory and therefore makes no power-loss or kernel-crash durability promise. Atomic visibility is not the same guarantee as durable persistence after sudden host failure.

After publication, the CLI returns an `evidenceDigest` calculated from the exact `SHA256SUMS` bytes. It is intentionally not stored inside the checksummed bundle.

Checksum creation and verification hash payloads through bounded buffers rather than retaining whole-bundle contents. Verification bounds checksum lines, inventory entries, and the size of records it must parse; large measurement payloads are never loaded wholesale. It parses `manifest.json` from the exact bounded bytes that passed checksum verification. It rejects malformed or duplicate checksums, missing or extra payloads, absolute or traversing paths, symlinks and canonical-path escapes, non-regular files, unsupported formats, invalid or inconsistent run identities, and empty required manifest fields.

Checksums establish integrity and internal consistency relative to `SHA256SUMS`; they do not establish who or which machine produced the bundle, whether that producer was trusted, or whether an attacker changed payloads and then recomputed every checksum. Publisher authenticity requires a separate trust mechanism and is deliberately deferred; this format is not cryptographically authenticated.
