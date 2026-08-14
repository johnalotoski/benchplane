# Architecture

Benchplane begins as a modular monolith.

```text
human-authored experiment
        │
        ▼
benchplane-schema ── strict public experiment and evidence records
        │
        ▼
benchplane-core ─── parsing, resolution, lifecycle, attempt provenance,
                    helper resource accounting, execution, validity, summaries, evidence writing,
                    verification, and bounded descriptive comparison
        │
        ▼
benchplane CLI ───── thin command dispatch and human/JSON presentation
```

The executable local path dispatches directly on three concrete combinations: `localFake`/`localFake` runs in process, while `local`/`cpuProbe` and `local`/`llamaCpp` supervise fixed helper executables supplied by the same package. `llamaCpp` is tied to one packaged SmolLM2 model and one workload profile; its typed target selects either the backward-compatible CPU helper or the x86-64-only single-device NVIDIA/CUDA helper. The CUDA helper fixes logical device 0, no split mode, and all-layer offload. Its repetition records retain a bounded nested latency/TTFT observation for each sequential request; no generic accelerator, event, or telemetry stream was added. The experiment cannot select a physical device, backend, offload count, path, command, arbitrary arguments, environment, working directory, prompt, model location, URL, or address. The supervised helpers share only a private bounded measurement-child supervisor for shell-free spawn, stdout/stderr bounds, deadlines, termination, exact per-child Linux `wait4` accounting and reaping, and complete-protocol success; their identities, metadata, arguments, record checks, work limits, and failure codes remain concrete. No provider/runtime traits, public process API, plugin ABI, general command runner, polling thread, or telemetry service was introduced. AWS and vLLM remain declarative and unimplemented.

There are now two execution envelopes around that same public CLI path:

```text
operator shell ───────────────────────────────┐
                                              ▼
systemctl start → NixOS oneshot service → benchplane run → benchplane-core
```

The service supplies a pinned package containing the CLI, fixed helpers, CPU and (on x86-64) CUDA llama.cpp backend closures, and the fixed GGUF model, plus a read-only experiment path, persistent output root, unprivileged identity, and systemd activation timeout. The CLI resolves helpers beside its own executable, so execution does not depend on ambient `PATH`; each inference helper reads its model and backend directory through compiled immutable Nix-store references. The optional NVIDIA module grants only the first device/control/UVM nodes needed by this target. The service does not introduce a daemon, RPC endpoint, queue, database, or controller-to-runner protocol. Explicit activation is an orchestration boundary: the unit is not boot-enabled, and every completed activation leaves the oneshot inactive so a later explicit start is a new run.

Run lifecycle, attempt provenance, attempt-owned helper resources, attempt outcome, measurement validity, and evidence publication are separate concerns. After entering attempt preparation, Benchplane records a bounded allowlist of platform and immutable software facts beneath that attempt before concrete execution starts. A successful CUDA helper adds its bounded observed device/driver/CUDA/offload proof to that same attempt record before the attempt becomes terminal; missing proof converts the concrete execution to the established runtime failure path. When supervised execution finishes, the same private boundary that reaps that exact child obtains its process-lifetime CPU time and peak RSS; in-process local-fake execution has no such observation. Llama request observations remain owned by their enclosing warmup or measured repetition and share one initialized helper/model lifetime; they are not attempts or independent runs. Provenance and resources are not run-global: a future retry design could execute another attempt on a different host or package. An attempt becomes terminal when concrete execution returns; collection and finalization belong to the enclosing run. Evidence format v1 deliberately contains one attempt and provides neither retry nor resume semantics.

Infrastructure and node configuration are implementation surfaces around the core lifecycle:

```text
OpenTofu root → high-level experiment module → ephemeral node
                                               │
                                               ▼
                                 composed NixOS capability modules
```

Experiments and studies depend on the framework. Framework code does not import, embed, or otherwise depend on files under `experiments/` or `studies/`. Repository checks may run catalog examples through the packaged public CLI.

`benchplane evidence compare` is a read-only consumer of two independently verified bundles, not another execution lifecycle. It accepts only the concrete current local llama.cpp v2 measurement contract, compares measurement-affecting typed fields explicitly—including CPU versus NVIDIA target—and derives request/repetition statistics from verified raw records. Matching NVIDIA runs can compare while CPU-versus-NVIDIA is incompatible. Device/driver/platform/package differences remain contextual output rather than a generalized environment-equivalence policy. The command creates no run ID, helper process, evidence payload, study, or persistent state.
