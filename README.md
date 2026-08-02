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

This repository is an early modular monolith. Its first zero-cost vertical slice parses and semantically validates `benchplane/v1alpha1` experiment YAML, materializes defaults into a deterministic resolved plan, executes one deterministic local-fake attempt, evaluates validity, summarizes measurements, and atomically publishes a verified evidence bundle.

It also establishes the intended boundaries for:

- a typed experiment and evidence schema;
- a Rust CLI and reusable core;
- in-repository Nix flake modules and NixOS modules;
- persistent and ephemeral OpenTofu layers;
- experiment definitions, studies, and evidence references;
- CI, formatting, security, and architectural decisions.

The implementation is deliberately conservative: it does not provision AWS resources, execute the declarative vLLM variant, run a GPU workload, resume a run, or retry an attempt.

## Local-fake vertical slice

```text
experiment YAML
→ strict parsing and semantic validation
→ deterministic resolution and plan identity
→ deterministic local-fake execution
→ lifecycle journal, records, measurements, validity, and summary
→ verified evidence finalization
→ same-filesystem atomic publication
```

Run the resource-free smoke experiment with:

```console
benchplane run experiments/smoke/local-fake.yaml
```

The default output root is `.benchplane`; use `--output-root PATH` to select another same-filesystem staging and publication root, and `--json` for one machine-readable result object. The original YAML bytes and the normalized experiment and resolved-plan digests are all retained in the bundle.

The first cloud milestone may later extend this path to one single-GPU vLLM experiment, but AWS and vLLM execution remain intentionally unimplemented.

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
just local-smoke
just tofu-validate
```

`just check` is deterministic and includes OpenTofu formatting, but not provider-backed semantic validation. `just tofu-validate` initializes the resource-free experiment root with its read-only provider lock file and may download the pinned provider; it does not run a plan or contact AWS APIs.

The supported development, CI, and release environment is the Nix development shell pinned by `flake.lock`. Running Cargo directly outside that environment is currently best-effort.

Benchplane does not currently promise a minimum supported Rust version (MSRV). An MSRV may be introduced for an individual crate when it has a real Cargo-only downstream consumer or is prepared for publication, and the claimed version can be continuously tested in CI.

## Design constraints

1. Framework code must not depend on `experiments/` or `studies/`.
2. Human-authored experiment files are YAML. Digests use deterministic `serde_json` serialization of fully parsed, defaulted Rust types; this is not standardized canonical JSON.
3. Requested specifications, resolved plans, runs, and attempts are distinct records; validation and resolution happen before a run ID is allocated.
4. AWS credentials and model tokens never enter Nix derivations, OpenTofu state, experiment files, or evidence bundles.
5. Ordinary CI must require neither AWS credentials nor a GPU.
6. Main should remain buildable and reviewable; feature branches may preserve useful incremental history.

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
