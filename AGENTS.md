# AGENTS.md

This file gives coding agents repository-specific operating instructions.

## Project intent

Benchplane is a reproducible AI-systems performance laboratory. Avoid narrowing the design to only vLLM, AWS, inference, NVIDIA, or NixOS even though those form the initial implementation path.

## Current milestone

Keep the packaged local CPU model-inference milestone small and reviewable. The concrete `local`/`llamaCpp` path:

1. loads the package-fixed SmolLM2-135M-Instruct Q2_K model with pinned CPU-only llama.cpp;
2. performs bounded greedy transformer inference in a package-owned child and emits no model text;
3. records one aggregate warmup or measured evidence-v1 record with observed latency, TTFT, throughput, and request counts;
4. accepts only the fixed `smollm2-chat-greedy-v1` workload at concurrency one and bounds records, tokens, child output, and runtime independently in parent and helper;
5. discards every parsed record unless the complete child protocol and exit status succeed; and
6. runs offline through the existing CLI and unprivileged NixOS oneshot on x86-64 and aarch64 Linux.

This milestone demonstrates real local model execution and measurement plumbing, not representative production performance, cross-host comparability, GPU or vLLM behavior, model quality, arbitrary model/prompt selection, runtime download, a generic subprocess framework, AWS execution, or a controller-to-runner protocol. Persistent runner state and evidence belong beneath the systemd-managed state directory; `PrivateTmp=true` intentionally provides private writable temporary directories.

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
