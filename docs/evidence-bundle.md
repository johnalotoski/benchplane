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
│       ├── provenance.json
│       ├── resources.json       # supervised helpers only
│       └── measurements.jsonl
├── validity.json
├── summary.json
├── manifest.json
└── SHA256SUMS
```

The original `experiment.yaml` bytes are preserved exactly. `resolved-plan.json`, lifecycle snapshots, the event journal, and measurements are written incrementally. Validity, summary, manifest, and checksums exist only after finalization.

The manifest records typed run and validity statuses, run ID, experiment and resolved-plan digests, and attempt count. `SHA256SUMS` covers the manifest and every regular payload recursively, but never itself. Paths are normalized and sorted lexicographically.

Evidence format v1 contains exactly one attempt, so `attemptCount` must equal `1`. The `attempts/` directory is a stable ownership boundary, not a promise that retries exist. Future retry support requires an evidence-format evolution or an explicitly reviewed relaxation of this invariant.

New runs write `attempts/0001/provenance.json` using record format `benchplane-attempt-provenance/v1`. The attempt-owned record contains matching run and attempt identities; bounded operating-system distribution/version, kernel name/release, architecture, CPU model/class and available logical-CPU count; Benchplane name/version and optional Nix-store package identity; and a tagged concrete runtime/generator identity. Packaged llama.cpp records additionally identify engine `b10133`, the fixed model identity and SHA-256, the CPU-only backend lineage, and their immutable Nix-store objects. Optional host facts are `null` when their fixed allowlisted source is unavailable. Non-Nix development builds may likewise have no meaningful store path.

`provenance.json` is an additive evidence-v1 extension: newly generated bundles always contain it, while historical v1 bundles without it remain valid. `SHA256SUMS` covers it like every regular payload. When present, the public verifier retains at most 16 KiB, strictly parses the typed record, checks bounded values and supported software identities, and requires its run ID and attempt number to match the bundle. Recomputed checksums do not make structurally malformed or semantically inconsistent provenance valid.

Supported supervised helper runs write `attempts/0001/resources.json` with record format `benchplane-attempt-resources/v1`. The record has matching run and attempt identities, scope `helperProcessLifetime`, unsigned `cpuTimeMicros`, and unsigned `peakRssBytes`. CPU time is total user plus system CPU time for the exact reaped helper. Peak RSS is Linux's helper-process high-water mark converted from KiB to bytes with checked arithmetic. The record is not part of the repetition sample population or summary.

For llama.cpp this lifetime starts with helper startup and includes backend/model initialization and model loading, warmup and measured repetitions, and teardown through exit/reaping. It intentionally does not subtract initialization or warmups and does not alter the request latency/TTFT definitions below. CPU time is not utilization or attributable per request/repetition; peak RSS is neither system-wide memory nor exclusive physical ownership or model-only memory. Neither field measures contention, energy, power, or GPU resources.

`resources.json` is another additive evidence-v1 extension. Historical bundles without it remain valid, including older bundles without provenance. `SHA256SUMS` covers the payload. When present, the verifier retains at most 4 KiB, strictly parses its format and known scope, checks run/attempt identity and exact Linux KiB-to-byte RSS units, and rejects malformed, oversized, or re-checksummed inconsistent records.

The record deliberately excludes environment variables, credentials, username, home directory, hostname, machine ID, network addresses, cloud identities, serial numbers, and arbitrary `/proc` or command output. These facts improve measurement attribution and software-lineage context but neither authenticate the producer nor establish performance equivalence, statistical reproducibility, or comparability across hosts.

For `benchplane-cpu-probe/v1`, each warmup or measured repetition contributes one aggregate `MeasurementRecord`. `latencyMicros` is mean completed-request latency, `timeToFirstTokenMicros` is mean first-output latency, `throughputMilliRequestsPerSecond` is successful requests divided by repetition wall time, and request counts report completed and failed requests. Summary latency and throughput continue to use only measured, non-warmup records; evidence-v1 gains no TTFT summary field. These timings are observed and nondeterministic even though the resolved plan and workload computation are deterministic.

For packaged llama.cpp, model/back-end loading occurs once before any repetition and is excluded from measurements. Within each repetition, requests execute sequentially at concurrency one. A request starts immediately before deterministic fixed-profile prompt construction and tokenization; its latency ends after greedy selection of the configured final generated token. TTFT uses that same start and ends after prompt prefill and greedy selection of the first generated token, so tokenization, context creation, and prompt evaluation are included. Aggregate `latencyMicros` and `timeToFirstTokenMicros` remain arithmetic means of the completed request durations, calculated from raw durations before conversion to integer microseconds. Throughput is configured requests divided by wall time from the beginning of the first request until the final request's context cleanup completes, expressed as milli-requests per second. Exactly configured requests completing the configured greedy decode count are successful; any tokenization, context, decode, sampling, or output failure aborts the helper, so a successful aggregate has `successfulRequests == workload.requests` and `failedRequests == 0`.

Historical generator `benchplane-llama-cpp-smollm2/v1` records only those repetition aggregates. Current generator `benchplane-llama-cpp-smollm2/v2` adds a `requestObservations` array to each aggregate record. It contains exactly `workload.requests` entries in one-based request-index order, each with positive integer `latencyMicros` and `timeToFirstTokenMicros` and TTFT no greater than latency. The observation inherits attempt number, warmup/measured phase, and repetition index from its enclosing aggregate. Individual durations are rounded up to microseconds with a minimum of one. The existing workload ceiling bounds a successful execution to at most 84 observations, and private helper record/stdout and verifier limits bound their transport and parsing.

Warmup records and observations use the same work and semantics but never contribute to validity sample counts or summary calculations. Current p50/p95 latency and throughput values remain statistics over measured repetition aggregates, not request-level tail summaries. All observations share one initialized model/helper lifetime and therefore are not independent runs. No generated model text or private prompt content is stored in the protocol or evidence. The resolved experiment supplies the fixed runtime, model, workload profile, and output-token identity; this additive typed field needs no evidence-format version change. The raw data can support later request-distribution analysis, but it provides no confidence interval, statistical independence, or cross-host comparability by itself.

Supervised CPU-probe and llama.cpp measurements are committed to the lifecycle only when the helper exits successfully, exact process accounting is available, and its complete record sequence validates. Records parsed before a nonzero exit, deadline, malformed trailing record, accounting failure, or other protocol failure are discarded; failed evidence therefore has indeterminate validity and no retained partial measurement prefix. A nonzero or timed-out child can still have honest attempt resource evidence when accounting was obtained during its final reap. Spawn failure has no child and therefore no resource record.

## Finalization and publication

Benchplane writes under `<output-root>/staging/<run-id>`, persists the terminal event and snapshots, writes finalization-only records, rejects symlinks and unsupported file types, writes checksums through `SHA256SUMS.tmp`, and verifies the complete staging bundle with the public verifier. Only then does it atomically rename the directory to `<output-root>/runs/<run-id>` on the same filesystem.

For the NixOS runner, `<output-root>` is the systemd-managed state directory, `/var/lib/benchplane` by default. Published bundles therefore appear beneath `/var/lib/benchplane/runs`; in-progress or retained diagnostic state appears beneath `/var/lib/benchplane/staging`. The service applies a `0027` umask and requests mode `0750` for the state directory so bundles are not world-readable or world-writable.

Finalized bundles are immutable. A partial staging directory is not an evidence bundle. If finalization, verification, or publication fails, no final directory is created and the staging directory is retained; failure details are updated best-effort because the underlying I/O failure may also prevent further persistence. An operator may inspect or delete retained staging data, but must not publish or cite it as authoritative evidence.

Same-filesystem rename provides atomic namespace visibility: consumers do not observe a partially renamed final directory. The current implementation does not synchronize every payload and parent directory and therefore makes no power-loss or kernel-crash durability promise. Atomic visibility is not the same guarantee as durable persistence after sudden host failure.

After publication, the CLI returns an `evidenceDigest` calculated from the exact `SHA256SUMS` bytes. It is intentionally not stored inside the checksummed bundle.

Checksum creation and verification hash payloads through bounded buffers rather than retaining whole-bundle contents. Verification bounds checksum lines, inventory entries, provenance, resources, the typed resolved plan, and measurement file/line sizes; measurement JSONL is parsed and re-hashed incrementally rather than retained wholesale. The verifier recomputes the repository's existing deterministic experiment and resolved-plan identities from the typed plan, then checks runtime/generator, phase/repetition order, counts, positive aggregate metrics, and the applicable request-observation contract. It accepts the known historical aggregate-only llama v1 lineage and the current observation-bearing v2 lineage, rejects unknown combinations, and links optional provenance/resources to the resolved runtime when those backward-compatible extensions are present. Historical evidence-v1 without provenance/resources remains valid. It also rejects malformed or duplicate checksums, missing or extra payloads, absolute or traversing paths, symlinks and canonical-path escapes, non-regular files, unsupported formats, invalid or inconsistent run/attempt identities, malformed typed payloads, and empty required manifest fields.

Checksums establish integrity and internal consistency relative to `SHA256SUMS`; they do not establish who or which machine produced the bundle, whether that producer was trusted, or whether an attacker changed payloads and then recomputed every checksum. Publisher authenticity requires a separate trust mechanism and is deliberately deferred; this format is not cryptographically authenticated.
