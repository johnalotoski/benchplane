# AGENTS.md

This file gives coding agents repository-specific operating instructions.

## Project intent

Benchplane is a reproducible AI-systems performance laboratory. Avoid narrowing the design to only vLLM, AWS, inference, NVIDIA, or NixOS even though those form the initial implementation path.

## Current milestone

Keep the attempt-scoped execution-provenance milestone small and reviewable. Every supported local execution path:

1. captures a bounded typed provenance record while attempt 1 is `preparing`, before concrete execution begins;
2. records only allowlisted OS, kernel, architecture, CPU model/class and logical-CPU availability fields, never generic host inventory or environment;
3. identifies the Benchplane package and concrete generator, including the pinned llama.cpp `b10133`, fixed SmolLM2 model digest, CPU-only backend lineage, and meaningful Nix-store identities for packaged inference;
4. stores the checksum-covered payload at `attempts/0001/provenance.json` and verifies its structure and attempt/run identity when present; and
5. preserves `benchplane-evidence/v1` compatibility by accepting historical bundles that predate the additive provenance payload.

This milestone improves attribution and reproducibility context; it does not authenticate a host or publisher, establish statistically reproducible or cross-host-comparable performance, measure CPU time or memory, add GPU/vLLM/AWS behavior, add signing or attestation, or introduce generic inventory, telemetry, process, provider, plugin, or controller abstractions. Preserve the fixed packaged inference, bounded child supervision, lifecycle, validity, and publication contracts.

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
