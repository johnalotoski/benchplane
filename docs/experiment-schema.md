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

`outputTokens` is 1–4,096 and `workUnitsPerToken` is 1–1,000,000. Only concurrency `1` is implemented. Warmups plus repetitions may not exceed 1,000, and their checked product with requests, output tokens, and work units may not overflow or exceed 100,000,000 work units. The adapter reserves a 512-byte serialized envelope per record, so the maximum accepted record set occupies at most 512,000 bytes beneath its 1,048,576-byte stdout limit. Positive requests and repetitions, the exact profile, these bounds, and a finite nonnegative local budget are validated before run allocation. The packaged helper independently enforces the same record and work limits.

## Packaged llama.cpp controls

The real-model combination is provider `local` with runtime `llamaCpp`. It accepts only model identity `smollm2-135m-instruct-q2-k-v1` and workload profile `smollm2-chat-greedy-v1`:

```yaml
provider: { kind: local }
runtime:
  kind: llamaCpp
  target: cpu
  model: smollm2-135m-instruct-q2-k-v1
  outputTokens: 4
workload:
  profile: smollm2-chat-greedy-v1
  requests: 2
  concurrency: 1
```

`target` accepts only `cpu` or `nvidiaCuda` and defaults to `cpu`; the default is omitted from deterministic serialization to preserve existing CPU resolved-plan identities. The explicit NVIDIA target is executable only from the CUDA-bearing `x86_64-linux` package. It does not select a user GPU: the package-owned policy always uses logical CUDA device 0, no split mode, and all-layer offload. A package/platform that cannot contain this target rejects it before UUID/output allocation; a missing, malformed, or older-than-610.43.03 host driver or device unavailability discovered by the concrete helper follows the allocated failed-run path.

`model` defaults to the fixed identity and `outputTokens` defaults to `4`; explicit values are retained in the resolved experiment. Generated tokens must be 1–32 and concurrency must equal `1`. Warmups plus measured repetitions may not exceed 16. The checked product `(warmups + repetitions) × requests × (96 maximum fixed-profile prompt tokens + outputTokens)` may not overflow or exceed 8,192 prompt-plus-generated tokens. That existing work ceiling also limits one successful execution to at most 84 numeric request observations; no enable/disable field is needed. Requests and repetitions must be positive, the lifecycle maximum remains 1–86,400 seconds, and the local budget must be finite and nonnegative. All semantic checks and provider/runtime compatibility selection occur before UUIDv7 allocation or output-root creation. Both package-owned helpers independently enforce the record, observation, output-token, prompt-token, and total-token bounds.

No public field can select an executable, physical GPU/index, backend, offload count, CUDA path/environment, model path, model URL/repository/revision, prompt, working directory, network address, sampler, thread count, or general llama.cpp option. Request-index variation is derived deterministically inside the fixed profile.

## Structural and semantic restrictions

The generated JSON Schema expresses the document structure, required properties, primitive types, tagged provider/runtime variants, defaults, and closed objects. Unknown properties are rejected at the top level and at each nested object or tagged-variant boundary. `metadata.labels` is intentionally an open string-to-string map.

Benchplane semantic validation applies restrictions that are awkward or misleading to express in the generated schema alone. These include supported `apiVersion` and artifact-format values, nonempty names and identities, positive requests and repetitions, local execution work bounds and compatibility, the lifecycle upper bound, and provider-specific budget rules. `localFake` and `local` budgets may be zero; an AWS budget must be finite and greater than zero.

Consumers must not treat JSON Schema acceptance as a substitute for `benchplane validate`.

Attempt provenance, supervised-helper resource accounting, and llama request observations are observed execution evidence, not workload configuration. They add no enable/disable switch, threshold, budget, or sampling interval. The only new public control is the concrete llama execution `target`, because CPU and CUDA must never be selected silently by package/host state.

## Resolved-experiment digest

Resolution hashes `serde_json::to_vec()` output for the fully parsed, defaulted `Experiment`. Struct fields serialize in their Rust declaration order, and map-valued labels use `BTreeMap`, which orders keys lexically. YAML whitespace, presentation style, and mapping order therefore do not affect the digest after parsing.

This representation is deterministic for this implementation, but it is not a standardized canonical-JSON scheme. A future version must define canonicalization before promising byte-for-byte cross-implementation reproduction.

## Resolved-plan digest

`resolvedPlanDigest` is SHA-256 over deterministic `serde_json` serialization of the resolved-plan content: API version, kind, fully parsed/defaulted experiment, and `experimentDigest`. The `resolvedPlanDigest` field itself is excluded from its input. Both digests are stored in the resolved plan, run record, evidence manifest, summary, and terminal CLI result.

CPU-probe and llama.cpp plan identities are deterministic, but their monotonic-clock measurements are observations and intentionally vary between executions and hosts.
