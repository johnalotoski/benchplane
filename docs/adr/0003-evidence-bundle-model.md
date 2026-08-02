# ADR 0003: immutable evidence bundles

## Status

Accepted.

## Decision

Store raw evidence outside Git in immutable, content-verifiable bundles. Commit reports, small summaries, analysis source, bundle locations, and digests.

## Rationale

Raw telemetry and logs grow quickly, while public conclusions must remain traceable to exact machine-readable evidence.
