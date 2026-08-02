# Experiment schema

The public schema distinguishes:

1. requested experiment — the user's YAML;
2. resolved experiment — defaults and immutable identities filled in;
3. run — one logical execution request;
4. attempt — each acquisition or execution attempt within a run.

Rust types are the initial source of truth. JSON Schema Draft 2020-12 is generated and checked in for external consumers.

## Local-fake runtime controls

The executable cost-free local combination is provider `localFake` with runtime `localFake`. Its runtime controls are strict and versioned with the experiment schema:

```yaml
runtime:
  kind: localFake
  seed: 42
  scenario: success
```

`seed` defaults to `0`. `scenario` defaults to `success` and accepts only `success`, `runtimeFailure`, `interrupted`, or `insufficientMeasurements`. Unknown fields are rejected. Synthetic measurements identify the stable generator as `benchplane-local-fake/v1`.

The sum of `measurement.warmupRuns` and `measurement.repetitions` must not exceed 10,000 for the local-fake runtime. Benchplane performs this semantic check with overflow-safe arithmetic before allocating a run ID. The bound limits CPU, memory, and disk work; `maximumRuntimeSeconds` is not an execution guard for the synchronous fake runtime.

## Local CPU-probe controls

The measured local combination is provider `local` with runtime `cpuProbe`, and it supports exactly the `cpu-token-probe-v1` workload profile:

```yaml
provider: { kind: local }
runtime:
  kind: cpuProbe
  outputTokens: 8
  workUnitsPerToken: 256
workload:
  profile: cpu-token-probe-v1
  requests: 2
  concurrency: 1
```

`outputTokens` is 1–4,096 and `workUnitsPerToken` is 1–1,000,000. Only concurrency `1` is implemented. Warmups plus repetitions may not exceed 10,000, and their checked product with requests, output tokens, and work units may not overflow or exceed 100,000,000 work units. Positive requests and repetitions, the exact profile, these bounds, and a finite nonnegative local budget are validated before run allocation.

## Structural and semantic restrictions

The generated JSON Schema expresses the document structure, required properties, primitive types, tagged provider/runtime variants, defaults, and closed objects. Unknown properties are rejected at the top level and at each nested object or tagged-variant boundary. `metadata.labels` is intentionally an open string-to-string map.

Benchplane semantic validation applies restrictions that are awkward or misleading to express in the generated schema alone. These include supported `apiVersion` and artifact-format values, nonempty names and identities, positive requests and repetitions, local execution work bounds and compatibility, the lifecycle upper bound, and provider-specific budget rules. `localFake` and `local` budgets may be zero; an AWS budget must be finite and greater than zero.

Consumers must not treat JSON Schema acceptance as a substitute for `benchplane validate`.

## Resolved-experiment digest

Resolution hashes `serde_json::to_vec()` output for the fully parsed, defaulted `Experiment`. Struct fields serialize in their Rust declaration order, and map-valued labels use `BTreeMap`, which orders keys lexically. YAML whitespace, presentation style, and mapping order therefore do not affect the digest after parsing.

This representation is deterministic for this implementation, but it is not a standardized canonical-JSON scheme. A future version must define canonicalization before promising byte-for-byte cross-implementation reproduction.

## Resolved-plan digest

`resolvedPlanDigest` is SHA-256 over deterministic `serde_json` serialization of the resolved-plan content: API version, kind, fully parsed/defaulted experiment, and `experimentDigest`. The `resolvedPlanDigest` field itself is excluded from its input. Both digests are stored in the resolved plan, run record, evidence manifest, summary, and terminal CLI result.

CPU-probe plan identity is deterministic, but its monotonic-clock measurements are observations and intentionally vary between executions and hosts.
