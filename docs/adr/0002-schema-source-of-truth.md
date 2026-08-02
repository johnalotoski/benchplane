# ADR 0002: Rust types are the initial schema source of truth

## Status

Accepted.

## Decision

Author public experiment types in `benchplane-schema`, generate versioned JSON Schema, and accept YAML as the primary human-authored representation.

## Consequences

Generated schemas are checked in and CI must detect drift. Resolved-experiment hashing uses deterministic `serde_json` serialization of fully parsed, defaulted typed data. This is not a standardized canonical-JSON format; defining one for cross-implementation reproduction is future work.
