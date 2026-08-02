# Experiment schema

The public schema distinguishes:

1. requested experiment — the user's YAML;
2. resolved experiment — defaults and immutable identities filled in;
3. run — one logical execution request;
4. attempt — each acquisition or execution attempt within a run.

Rust types are the initial source of truth. JSON Schema Draft 2020-12 is generated and checked in for external consumers.

## Structural and semantic restrictions

The generated JSON Schema expresses the document structure, required properties, primitive types, tagged provider/runtime variants, defaults, and closed objects. Unknown properties are rejected at the top level and at each nested object or tagged-variant boundary. `metadata.labels` is intentionally an open string-to-string map.

Benchplane semantic validation applies restrictions that are awkward or misleading to express in the generated schema alone. These include supported `apiVersion` and artifact-format values, nonempty names and provider/workload/runtime identities, positive requests and repetitions, the lifecycle upper bound, and provider-specific budget rules. A `localFake` budget may be zero; an AWS budget must be finite and greater than zero.

Consumers must not treat JSON Schema acceptance as a substitute for `benchplane validate`.

## Resolved-experiment digest

Resolution hashes `serde_json::to_vec()` output for the fully parsed, defaulted `Experiment`. Struct fields serialize in their Rust declaration order, and map-valued labels use `BTreeMap`, which orders keys lexically. YAML whitespace, presentation style, and mapping order therefore do not affect the digest after parsing.

This representation is deterministic for this implementation, but it is not a standardized canonical-JSON scheme. A future version must define canonicalization before promising byte-for-byte cross-implementation reproduction.
