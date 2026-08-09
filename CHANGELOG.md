# Changelog

All notable changes will be documented here once versioned releases begin.

## Unreleased

- Establish initial modular-monolith repository scaffold.
- Add the deterministic cost-free local-fake lifecycle and atomically published evidence bundles.
- Add the explicitly activated, hardened NixOS runner service and native x86-64/aarch64 VM integration coverage.
- Add the bounded local CPU token probe with observed latency, time-to-first-token, and throughput measurements.
- Add fixed, offline CPU-only llama.cpp inference over the packaged SmolLM2-135M-Instruct Q2_K model, with bounded supervised measurements through evidence-v1 and the NixOS oneshot.
- Add integrity-covered attempt-scoped platform and packaged-runtime provenance as a backward-compatible evidence-v1 extension, plus direct-helper protection against ambient llama.cpp backend redirection.
- Add exact attempt-scoped helper-process lifetime CPU-time and peak-RSS accounting for supervised CPU-probe and llama.cpp execution as a backward-compatible evidence-v1 extension.
- Retain bounded per-request latency and TTFT observations for packaged llama.cpp inference under a new generator contract, with streaming semantic evidence verification and historical aggregate-only compatibility.
