# Evidence bundle

The target evidence format is `benchplane-evidence/v1`.

A complete bundle should eventually contain the requested specification, resolved plan, run and attempt records, Nix and source provenance, host/GPU/runtime metadata, request and telemetry measurements, validity decisions, uncertainty summaries, SLO and cost results, logs, reports, and checksums.

The initial local milestone uses a miniature deterministic fixture containing a top-level manifest, summary, and checksums. Verification requires the manifest itself to be checksummed and parses it from the exact bytes that passed checksum verification. It rejects malformed digests, duplicate entries, absolute or traversing paths, symlink escapes, non-regular payloads, unsupported formats, and empty required manifest fields.

Checksums establish bundle integrity relative to `SHA256SUMS`; they do not establish who published the bundle or whether the checksum file itself came from a trusted source. Publisher authenticity requires a separate trust mechanism.
