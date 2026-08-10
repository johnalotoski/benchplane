# AGENTS.md

This file gives coding agents repository-specific operating instructions.

## Project intent

Benchplane is a reproducible AI-systems performance laboratory. Avoid narrowing the design to only vLLM, AWS, inference, NVIDIA, or NixOS even though those form the initial implementation path.

## Current milestone

Keep verified two-bundle llama.cpp comparison small and descriptive. `benchplane evidence compare` must:

1. pass both inputs through the full bounded semantic evidence verifier before analysis;
2. accept only successful, valid current local llama.cpp v2 bundles and compare concrete measurement-affecting model/runtime/workload dimensions rather than whole resolved-plan digests;
3. calculate measured request and repetition statistics from raw non-warmup records, never copied summary numbers;
4. keep request, repetition, independently initialized run, and whole-helper resource scopes explicit; and
5. report recorded environment differences as context without claiming that they caused a performance delta.

Use deterministic floor means and the existing nearest-rank percentile convention. Requests and repetitions inside one helper lifetime are not independent runs. Helper CPU time and peak RSS include initialization, model loading, warmups, measured work, and teardown and must never be attributed per request. Historical llama v1 remains verifiable but is not eligible for this first comparison contract. Do not add confidence intervals, significance tests, causal claims, run orchestration/groups, persistent comparison artifacts, generic analytics/equivalence abstractions, GPU/vLLM/AWS execution, or new lifecycle/evidence/experiment versions. Preserve the package-owned execution, supervision, provenance, resources, lifecycle, validity, and atomic publication contracts.

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
