# Benchplane

**Reproducible AI systems experiments, from specification to evidence.**

Benchplane is an early-stage public portfolio project for declaratively describing an AI-systems experiment and carrying it through ephemeral infrastructure, a pinned NixOS/software environment, repeated measurement, provenance capture, cost/SLO analysis, evidence publication, and teardown.

The first workload target is vLLM on AWS GPU instances. The architecture is intentionally broader than “vLLM on NixOS” and should eventually support additional inference engines, training workloads, GPU architectures, providers, and experiment classes.

## Why this exists

Many individual pieces already exist: vLLM packaging, NixOS GPU support, cloud provisioning, load generators, Kubernetes serving stacks, and benchmark harnesses. Benchplane aims to connect those pieces into a whole-experiment reproducibility system with:

- pinned software and Nix closure provenance;
- ephemeral Spot and On-Demand lifecycle handling;
- repeated, uncertainty-aware measurements;
- machine-readable evidence bundles;
- effective cost per valid or SLO-compliant experiment;
- native-Nix versus container comparisons;
- cross-version regression studies;
- public reports backed by immutable evidence.

## Current status

This repository is an initial modular-monolith scaffold. It establishes the intended boundaries for:

- a typed experiment and evidence schema;
- a Rust CLI and reusable core;
- in-repository Nix flake modules and NixOS modules;
- persistent and ephemeral OpenTofu layers;
- experiment definitions, studies, and evidence references;
- CI, formatting, security, and architectural decisions.

The scaffold is deliberately conservative: it does not yet provision AWS resources or run a GPU workload.

## Planned first vertical slice

```text
experiment YAML
→ validation and deterministic resolution
→ local fake execution
→ miniature evidence bundle
→ evidence verification
→ deterministic report summary
```

The first cloud milestone will extend that path to one single-GPU vLLM experiment with artifact upload and automatic instance termination.

## Repository map

```text
crates/        Rust CLI, reusable core, and typed schema
schemas/       Generated, versioned public schemas
nix/           Flake modules, packages, NixOS modules, images, and VM tests
infra/tofu/    Persistent account foundation and ephemeral experiment root/module
experiments/   Reusable experiment specifications and smoke examples
studies/       Hypotheses, analyses, reports, and evidence locks
results/       Small checked-in examples only; real bundles live outside Git
docs/          Architecture, lifecycle, security, and ADRs
```

## Development

Enter the development shell:

```console
nix develop
```

Then run:

```console
just fmt
just check
```

## Design constraints

1. Framework code must not depend on `experiments/` or `studies/`.
2. Human-authored experiment files are YAML; canonical hashing uses JSON.
3. Requested specifications, resolved plans, runs, and attempts are distinct records.
4. AWS credentials and model tokens never enter Nix derivations, OpenTofu state, experiment files, or evidence bundles.
5. Ordinary CI must require neither AWS credentials nor a GPU.
6. Main should remain buildable and reviewable; feature branches may preserve useful incremental history.

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
