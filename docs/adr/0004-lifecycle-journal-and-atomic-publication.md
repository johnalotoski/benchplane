# ADR 0004: lifecycle journal and atomic evidence publication

## Status

Accepted.

## Decision

Use an append-only lifecycle event journal as the authoritative chronology of a run. Maintain `run.json` and per-attempt JSON snapshots as convenience projections, written atomically through temporary sibling files.

Build evidence bundles in a staging directory, finalize and verify every payload there, and publish only a verified bundle through a same-filesystem atomic directory rename. Treat a finalized bundle as immutable.

Evidence format v1 records exactly one execution attempt. Attempts become terminal when execution returns; collection and finalization remain run-level phases and cannot mutate a terminal attempt outcome.

## Consequences

A final path is either absent or contains a complete verified bundle; consumers never observe an in-progress publication. Partial staging directories remain useful for diagnosis but are not evidence bundles. Runtime-failed and interrupted runs can still publish evidence when finalization succeeds. Checksums provide integrity, not publisher authenticity.

Atomic rename is an atomic-visibility guarantee, not a claim of power-loss durability. The current implementation does not promise persistence across sudden kernel or host failure because it does not synchronize every finalized file and parent directory. Retries, resumability, real signal handling, signing, and publisher attestation remain separate future decisions.
