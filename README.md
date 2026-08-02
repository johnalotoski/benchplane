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

This repository is an early modular-monolith scaffold. Its first local vertical slice can parse and semantically validate `benchplane/v1alpha1` experiment YAML, materialize defaults into a deterministic resolved plan, export a generated JSON Schema, and verify a miniature checksummed local-fake evidence fixture.

It also establishes the intended boundaries for:

- a typed experiment and evidence schema;
- a Rust CLI and reusable core;
- in-repository Nix flake modules and NixOS modules;
- persistent and ephemeral OpenTofu layers;
- experiment definitions, studies, and evidence references;
- CI, formatting, security, and architectural decisions.

The scaffold is deliberately conservative: it does not yet execute the full local lifecycle, provision AWS resources, or run a GPU workload.

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
nix/           Flake modules, packages, and evaluated NixOS modules
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
just tofu-validate
```

`just check` is deterministic and includes OpenTofu formatting, but not provider-backed semantic validation. `just tofu-validate` initializes the resource-free experiment root with its read-only provider lock file and may download the pinned provider; it does not run a plan or contact AWS APIs.

The supported development, CI, and release environment is the Nix development shell pinned by `flake.lock`. Running Cargo directly outside that environment is currently best-effort.

Benchplane does not currently promise a minimum supported Rust version (MSRV). An MSRV may be introduced for an individual crate when it has a real Cargo-only downstream consumer or is prepared for publication, and the claimed version can be continuously tested in CI.

## Design constraints

1. Framework code must not depend on `experiments/` or `studies/`.
2. Human-authored experiment files are YAML. Digests use deterministic `serde_json` serialization of fully parsed, defaulted Rust types; this is not standardized canonical JSON.
3. Requested specifications, resolved plans, runs, and attempts are distinct records.
4. AWS credentials and model tokens never enter Nix derivations, OpenTofu state, experiment files, or evidence bundles.
5. Ordinary CI must require neither AWS credentials nor a GPU.
6. Main should remain buildable and reviewable; feature branches may preserve useful incremental history.

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
