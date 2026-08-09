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

This repository is an early modular monolith. Its local vertical slices parse and semantically validate `benchplane/v1alpha1` experiment YAML, materialize defaults into a deterministic resolved plan, capture bounded attempt-scoped execution provenance, execute deterministic local-fake work, a measured local CPU token probe, or real packaged CPU-only model inference, account for supervised helper CPU time and peak RSS, evaluate validity, summarize measurements, and atomically publish a verified evidence bundle. The same lifecycle runs either directly from the CLI or inside an unprivileged NixOS systemd service.

It also establishes the intended boundaries for:

- a typed experiment and evidence schema;
- a Rust CLI and reusable core;
- in-repository Nix flake modules and NixOS modules;
- persistent and ephemeral OpenTofu layers;
- experiment definitions, studies, and evidence references;
- CI, formatting, security, and architectural decisions.

The implementation is deliberately conservative: it does not provision AWS resources, execute the declarative vLLM variant, run a GPU workload, handle parent operating-system signals, resume a run, or retry an attempt. Every local runtime's records and work are explicitly bounded before a run ID is allocated.

## Local execution

```text
experiment YAML
→ strict parsing and semantic validation
→ deterministic resolution and plan identity
→ bounded attempt-scoped platform and packaged-software provenance
→ concrete local runtime execution
  (deterministic local-fake, measured CPU probe, or packaged llama.cpp inference)
→ exact helper-process lifetime CPU-time and peak-RSS observation, when applicable
→ lifecycle journal, records, measurements, validity, and summary
→ verified evidence finalization
→ same-filesystem atomic publication
```

Run the cost-free, self-contained smoke experiment with:

```console
benchplane run experiments/smoke/local-fake.yaml
benchplane run experiments/smoke/local-cpu-probe.yaml
benchplane run experiments/smoke/local-llama-cpp.yaml
```

The CPU probe starts the fixed `benchplane-cpu-probe` executable from the same package without a shell. It performs deterministic, data-dependent CPU work and records observed request latency, first-output latency, and request throughput. Timing varies by host and proves only local CPU execution, subprocess supervision, and inference-shaped measurement plumbing—not model, GPU, vLLM, cross-host, or production inference performance.

The `llamaCpp` runtime starts the package-owned `benchplane-llama-cpp` helper without a shell. It uses CPU-only llama.cpp `b10133` to load the fixed [SmolLM2-135M-Instruct Q2_K GGUF](https://huggingface.co/QuantFactory/SmolLM2-135M-Instruct-GGUF) from the Nix store and perform greedy transformer decoding. The Apache-2.0 model file is exactly 88,201,792 bytes and is pinned at repository commit `c33bd7b3a0c1c5048af630f0198eb2a29977b422` with SHA-256 `55aa88ddac43adce6af0e9be8d6cdff2337a3835cd9b50bbcd7a894eb66dfc75`; llama.cpp is MIT-licensed. No model download, network, home directory, credential, GPU, container, arbitrary model path, or arbitrary prompt is used at execution time.

This tiny quantized fixture is suitable for routine x86-64 and aarch64 CI because it is a real but unusually small 135M-parameter model. It proves actual local model execution and measurement plumbing only. It is not representative of production inference, model quality, cross-host performance, GPU behavior, vLLM behavior, or cloud lifecycle.

The default output root is `.benchplane`; use `--output-root PATH` to select another same-filesystem staging and publication root, and `--json` for one machine-readable result object. The original YAML bytes and the normalized experiment and resolved-plan digests are all retained in the bundle.

New bundles also contain `attempts/0001/provenance.json`, an integrity-covered record of the attempt's bounded OS, kernel, architecture, CPU class/availability, Benchplane package, generator, and fixed llama.cpp/model/backend lineage. It intentionally omits hostnames, user identity, machine identifiers, addresses, credentials, and arbitrary environment or inventory data. This context improves attribution but does not authenticate the host or make performance statistically reproducible or comparable across hosts. Historical `benchplane-evidence/v1` bundles without this additive payload remain verifiable.

Helper-backed CPU-probe and llama.cpp runs additionally contain `attempts/0001/resources.json`. It records exact Linux accounting for the supervised helper's full process lifetime: user-plus-system `cpuTimeMicros` and peak resident-set high-water `peakRssBytes`. Llama accounting therefore includes startup, model loading, warmups, measured repetitions, and teardown; it does not change latency or TTFT semantics. These values are not CPU utilization, per-request cost, system-wide or exclusive memory, energy, contention, or a basis for cross-host comparability. In-process `localFake` runs have no equivalent helper observation, and historical evidence-v1 bundles without resources remain verifiable.

The first cloud milestone may later extend this path to one single-GPU vLLM experiment, but AWS and vLLM execution remain intentionally unimplemented.

## NixOS runner service

The NixOS module defines an explicitly activated `benchplane-runner.service`. It invokes the configured package's public `benchplane run` command as an unprivileged system user and stores staging data and published runs beneath the systemd-managed `/var/lib/benchplane` state directory. It is not attached to `multi-user.target`: each intentional `systemctl start benchplane-runner.service` invocation creates one new run.

See [`docs/nixos-runner.md`](docs/nixos-runner.md) for module options, operation, exit behavior, timeout semantics, and security boundaries. The flake includes a cost-free NixOS VM integration test using only local compute resources; its model is already in the immutable package closure, so it needs no cloud account, GPU, container runtime, external runtime service, runtime model download, or runtime network access.

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
just cpu-probe-smoke
just llama-cpp-smoke
just nixos-runner-test
just tofu-validate
```

`just check` is deterministic and includes OpenTofu formatting, but not provider-backed semantic validation. `just nixos-runner-test` runs only the NixOS VM check; `nix flake check` includes it with the other flake checks. `just tofu-validate` initializes the cost-free experiment root with its read-only provider lock file and may download the pinned provider; it does not run a plan or contact AWS APIs.

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
