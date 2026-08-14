# AGENTS.md

This file gives coding agents repository-specific operating instructions.

## Project intent

Benchplane is a reproducible AI-systems performance laboratory. Avoid narrowing the design to only vLLM, AWS, inference, NVIDIA, or NixOS even though those form the initial implementation path.

## Current milestone

Keep the bounded NVIDIA llama.cpp slice concrete. The public `llamaCpp.target` distinguishes backward-compatible `cpu` from the single supported `nvidiaCuda` target. The NVIDIA package/helper exists only on `x86_64-linux`, requires host driver 575.57.08 or newer before backend initialization, selects logical CUDA device 0, uses no split mode, and requires observed all-layer offload before a run can succeed. Never expose device indices, offload counts, backend paths, CUDA environment, or general llama.cpp arguments.

Successful NVIDIA evidence must carry bounded attempt provenance for the package/runtime/model/backend plus device name, logical selection, CUDA-visible memory capacity, NVIDIA/CUDA identities, compute capability, and effective offload. Exclude UUIDs, serials, PCI identifiers, arbitrary inventory, and GPU telemetry. CPU/RSS resources remain exact host helper-process lifetime observations and are not GPU measurements. Comparison requires equal CPU/NVIDIA target, so CPU-versus-GPU is incompatible while matching GPU runs remain descriptively comparable. Preserve historical evidence, request/repetition measurement semantics, package-owned backend protection, exact supervision/accounting, lifecycle, validity, and atomic publication. Do not add GPU telemetry, multi-GPU, NCCL, ROCm, Vulkan, vLLM, AWS, orchestration, statistics, or accelerator/runtime abstractions.

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
