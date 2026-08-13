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

This repository is an early modular monolith. Its local vertical slices parse and semantically validate `benchplane/v1alpha1` experiment YAML, materialize defaults into a deterministic resolved plan, capture bounded attempt-scoped execution provenance, execute deterministic local-fake work, a measured local CPU token probe, or real packaged llama.cpp model inference on CPU or one explicit NVIDIA/CUDA target, retain bounded llama.cpp request observations, account for supervised helper CPU time and peak RSS, evaluate validity, summarize measurements, and atomically publish a verified evidence bundle. It can also descriptively compare two verified, measurement-compatible current llama.cpp bundles. The same execution lifecycle runs either directly from the CLI or inside an unprivileged NixOS systemd service.

It also establishes the intended boundaries for:

- a typed experiment and evidence schema;
- a Rust CLI and reusable core;
- in-repository Nix flake modules and NixOS modules;
- persistent and ephemeral OpenTofu layers;
- experiment definitions, studies, and evidence references;
- CI, formatting, security, and architectural decisions.

The implementation is deliberately conservative: it does not provision AWS resources, execute the declarative vLLM variant, collect GPU telemetry, handle parent operating-system signals, resume a run, or retry an attempt. Every local runtime's records and work are explicitly bounded before a run ID is allocated.

## Local execution

```text
experiment YAML
→ strict parsing and semantic validation
→ deterministic resolution and plan identity
→ bounded attempt-scoped platform and packaged-software provenance
→ concrete local runtime execution
  (deterministic local-fake, measured CPU probe, or packaged CPU/NVIDIA llama.cpp inference)
→ bounded per-request latency and TTFT observations for llama.cpp
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
# x86_64-linux package plus a usable NVIDIA driver/device:
benchplane run experiments/examples/local-llama-cpp-nvidia-cuda.yaml
```

The CPU probe starts the fixed `benchplane-cpu-probe` executable from the same package without a shell. It performs deterministic, data-dependent CPU work and records observed request latency, first-output latency, and request throughput. Timing varies by host and proves only local CPU execution, subprocess supervision, and inference-shaped measurement plumbing—not model, GPU, vLLM, cross-host, or production inference performance.

The `llamaCpp` runtime starts a package-owned helper without a shell. Target `cpu` is the backward-compatible default and uses the existing CPU-only default package on `x86_64-linux` and `aarch64-linux`. Target `nvidiaCuda` is available only from the named `packages.x86_64-linux.benchplane-nvidia-cuda` package; its pinned CUDA 12.9 Update 1 build targets compute capabilities 7.5, 8.0, 8.6, 8.9, 9.0, 10.0, 10.3, 12.0, and 12.1 and requires an NVIDIA host driver at least version 575.57.08. This is Benchplane's conservative native-stack floor from the pinned CUDA 12.9.1 metadata, not CUDA 12.x's absolute compatibility minimum: NVIDIA documents minor-version compatibility with older 525-series drivers, but Benchplane does not currently claim or test that mode. Before loading ggml or initializing CUDA, the helper reads the fixed bounded driver-version source and rejects an older, missing, or malformed version. Passing the numeric gate does not guarantee a given GPU/driver combination works; successful evidence still requires CUDA/backend/device initialization, complete model offload and inference. It uses a separate CUDA-enabled b10133 helper, selects CUDA logical device 0, disables split mode, and requires every model layer to be reported as offloaded to that device. Neither target may select another device, backend, offload count, or helper through experiment input.

Both targets load the fixed [SmolLM2-135M-Instruct Q2_K GGUF](https://huggingface.co/QuantFactory/SmolLM2-135M-Instruct-GGUF) from the Nix store and perform greedy transformer decoding. The Apache-2.0 model file is exactly 88,201,792 bytes and is pinned at repository commit `c33bd7b3a0c1c5048af630f0198eb2a29977b422` with SHA-256 `55aa88ddac43adce6af0e9be8d6cdff2337a3835cd9b50bbcd7a894eb66dfc75`; llama.cpp b10133 is MIT-licensed. The helper loads the model once, executes configured requests sequentially at concurrency one, and retains bounded request latency and TTFT observations alongside each repetition aggregate. These requests share initialized model state and are not independent runs. No model download, network, home directory, credential, container, arbitrary model path, or arbitrary prompt is used at execution time. NVIDIA execution intentionally depends on the host kernel driver and `/dev/nvidia*` device interface outside the immutable Nix closure.

This tiny quantized fixture is suitable for routine CPU CI because it is a real but unusually small 135M-parameter model. It proves the execution, measurement, and accelerator-attribution path only. Its CPU or GPU numbers are not representative of production inference, model quality, realistic GPU scaling, cross-host performance, vLLM behavior, or cloud lifecycle; fixed model-size, launch, and transfer overhead can dominate.

The default output root is `.benchplane`; use `--output-root PATH` to select another same-filesystem staging and publication root, and `--json` for one machine-readable result object. The original YAML bytes and the normalized experiment and resolved-plan digests are all retained in the bundle.

New bundles also contain `attempts/0001/provenance.json`, an integrity-covered record of the attempt's bounded OS, kernel, architecture, CPU class/availability, Benchplane package, generator, and fixed llama.cpp/model/backend lineage. A successful NVIDIA run additionally records bounded device name, logical selection, CUDA-visible memory capacity, driver/CUDA/toolkit identities, compute capability, and the observed all-layer offload proof. It deliberately omits GPU UUID, serial and PCI identifiers as well as hostnames, user identity, machine identifiers, addresses, credentials, and arbitrary environment or inventory data. This context improves attribution but does not authenticate the host or make performance statistically reproducible or comparable across hosts. Historical `benchplane-evidence/v1` bundles without this additive payload remain verifiable.

Helper-backed CPU-probe and llama.cpp runs additionally contain `attempts/0001/resources.json`. It records exact Linux accounting for the supervised helper's full process lifetime: user-plus-system `cpuTimeMicros` and peak resident-set high-water `peakRssBytes`. Llama accounting therefore includes startup, model loading, warmups, measured repetitions, and teardown; it does not change latency or TTFT semantics. These values are not CPU utilization, per-request cost, system-wide or exclusive memory, energy, contention, or a basis for cross-host comparability. In-process `localFake` runs have no equivalent helper observation, and historical evidence-v1 bundles without resources remain verifiable.

For current llama.cpp evidence, every warmup and measured repetition aggregate contains one numeric request observation per configured request. Model initialization remains outside request latency and TTFT. Warmup observations remain excluded from validity and summaries, and the displayed p50/p95 values remain statistics over measured repetition aggregates—not request-tail percentiles. The retained observations enable later distribution analysis, but this slice computes no confidence intervals and makes no independence or cross-host-comparability claim. Historical aggregate-only llama evidence remains verifiable.

Compare two independently generated current llama.cpp bundles without executing a workload or changing either bundle:

```console
benchplane evidence compare BASELINE_BUNDLE CANDIDATE_BUNDLE
benchplane evidence compare BASELINE_BUNDLE CANDIDATE_BUNDLE --json
```

Both inputs first pass the same bounded semantic verification as `benchplane evidence verify`. Comparison requires successful, valid local llama.cpp v2 runs with equal execution target, model/engine/backend, workload profile, output-token target, request/concurrency counts, and warmup/repetition counts. CPU and NVIDIA runs are therefore explicitly incompatible rather than presented as a speedup comparison; two matching NVIDIA runs are eligible, while recorded device/driver and other platform differences are reported as context. Request latency/TTFT and repetition aggregate latency/TTFT/throughput are recomputed from measured, non-warmup records; helper CPU time and peak RSS remain host-process whole-helper-lifetime observations, not GPU telemetry. Results are deterministic descriptive arithmetic using floor means and nearest-rank p50/p95, not confidence, significance, independence, causality, or cross-host equivalence claims. Historical llama v1 remains verifiable but is not comparison-eligible.

The first cloud milestone may later extend this path to one single-GPU vLLM experiment, but AWS and vLLM execution remain intentionally unimplemented. Ordinary CI builds and semantically tests the CUDA package/evidence path without claiming real CUDA execution. The hardware path was additionally exercised on an NVIDIA GeForce RTX 4060 Laptop GPU with open driver 610.43.03 (CUDA driver API 13.3): the packaged CUDA 12.9 Update 1 helper generated tokens, reported all 31 model layers offloaded, published two verified bundles, compared those GPU runs as compatible, and rejected a missing-device run without CPU fallback. This is a validated configuration, not the minimum driver requirement, and proves the bounded path only on that configuration—not general NVIDIA compatibility or representative performance.

## NixOS runner service

The NixOS module defines an explicitly activated `benchplane-runner.service`. It invokes the configured package's public `benchplane run` command as an unprivileged system user and stores staging data and published runs beneath the systemd-managed `/var/lib/benchplane` state directory. It is not attached to `multi-user.target`: each intentional `systemctl start benchplane-runner.service` invocation creates one new run.

See [`docs/nixos-runner.md`](docs/nixos-runner.md) for module options, NVIDIA composition, operation, exit behavior, timeout semantics, and security boundaries. The flake includes a cost-free CPU NixOS VM integration test using only local compute resources; its model is already in the immutable package closure, so it needs no cloud account, GPU, container runtime, external runtime service, runtime model download, or runtime network access. That VM test does not claim CUDA execution coverage.

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
