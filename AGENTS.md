# AGENTS.md

This file gives coding agents repository-specific operating instructions.

## Project intent

Benchplane is a reproducible AI-systems performance laboratory. Avoid narrowing the design to only vLLM, AWS, inference, NVIDIA, or NixOS even though those form the initial implementation path.

## Current milestone

Keep the attempt-scoped helper-resource-accounting milestone small and reviewable. Every supported supervised local helper path:

1. obtains exact per-child Linux accounting at the private reap boundary, never from a process-global child-usage delta;
2. records helper-process-lifetime user-plus-system CPU time in microseconds and peak RSS in bytes without attributing either value to requests or repetitions;
3. retains accounting on nonzero-exit and deadline failures when the exact reap observation is available, while preserving the established primary failure and discarding partial measurement output;
4. stores the checksum-covered payload at `attempts/0001/resources.json` and verifies its bounded type, format, scope, and attempt/run identity when present; and
5. leaves resources absent for in-process `localFake` and preserves `benchplane-evidence/v1` compatibility for historical bundles without the additive payload.

For llama.cpp the resource scope includes startup, backend/model initialization and loading, warmups, measured repetitions, and teardown. Existing latency/TTFT semantics remain unchanged. This milestone does not measure utilization, system-wide or exclusive memory, energy, contention, GPU resources, or per-request/per-repetition resources; establish statistically reproducible or cross-host-comparable performance; add GPU/vLLM/AWS behavior; or introduce generic telemetry, process, provider, plugin, or controller abstractions. Preserve attempt provenance, package-owned execution, bounded child supervision, lifecycle, validity, and publication contracts.

## Boundaries

- `benchplane` is the thin CLI crate.
- `benchplane-schema` owns public data types and validation.
- `benchplane-core` owns parsing reuse, resolution, lifecycle, execution, validity, summaries, evidence writing, and verification.
- Framework code must not import or embed files from `experiments/` or `studies/`.
- Keep AWS and vLLM as concrete modules until a second real implementation proves an extraction boundary.

## Safety and cost

- Never add credentials, tokens, OpenTofu state, pre-signed URLs, or private account identifiers.
- Never place secrets in Nix derivations or the Nix store.
- Do not add an AWS apply path without explicit maximum runtime, maximum cost, run tags, state isolation, artifact finalization, and unconditional teardown.
- Ordinary pull-request CI must not contact AWS.

## Change discipline

- Make the smallest coherent change that advances the current milestone.
- Add tests for behavior, not only structure.
- Prefer explicit tagged schema variants over speculative plugin systems.
- Do not add placeholder CLI commands that only print “not implemented.”
- Do not introduce Kubernetes, a web UI, a database, multi-cloud support, or a plugin ABI during the initial milestone.
- Update an ADR when changing an architectural decision.

## Expected checks

Use the supported Nix environment for Rust development and checks:

```console
nix develop
just fmt
just check
just local-smoke
just cpu-probe-smoke
just llama-cpp-smoke
just nixos-runner-test
just tofu-validate
```

Commands may instead be invoked explicitly with `nix develop -c <command>`. Direct Cargo use outside the pinned Nix environment is currently best-effort; when Nix is unavailable, clearly report which supported checks could not be executed.

## GitHub Actions

Whenever creating, modifying, or reviewing files under `.github/workflows/` or
`.github/actions/`, use the `$hardening-github-actions` skill before editing to
establish the threat model and after editing to audit the completed diff. This
applies to CI, AWS OIDC, GPU experiments, scheduled benchmarks, releases,
reusable workflows, artifact publication, self-hosted runners, and deployment
automation.
