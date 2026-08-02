# ADR 0004: lifecycle journal and atomic evidence publication

## Status

Accepted.

## Decision

Use an append-only lifecycle event journal as the authoritative chronology of a run. Maintain `run.json` and per-attempt JSON snapshots as convenience projections, written atomically through temporary sibling files.

Build evidence bundles in a staging directory, finalize and verify every payload there, and publish only a verified bundle through a same-filesystem atomic directory rename. Treat a finalized bundle as immutable.

## Consequences

A final path is either absent or contains a complete verified bundle; consumers never observe an in-progress publication. Partial staging directories remain useful for diagnosis but are not evidence bundles. Runtime-failed and interrupted runs can still publish evidence when finalization succeeds. Checksums provide integrity, not publisher authenticity.
