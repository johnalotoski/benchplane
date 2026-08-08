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
                    execution, validity, summaries, evidence writing,
                    and verification
        │
        ▼
benchplane CLI ───── thin command dispatch and human/JSON presentation
```

The executable local path dispatches directly on three concrete combinations: `localFake`/`localFake` runs in process, while `local`/`cpuProbe` and `local`/`llamaCpp` supervise fixed helper executables supplied by the same package. `llamaCpp` is tied to one packaged SmolLM2 model and one workload profile. The experiment cannot select a path, command, arbitrary arguments, environment, working directory, prompt, model location, URL, or address. The two supervised helpers share only a private bounded measurement-child supervisor for shell-free spawn, stdout/stderr bounds, deadlines, termination, reaping, and complete-protocol success; their identities, arguments, record checks, work limits, and failure codes remain concrete. No provider/runtime traits, public process API, plugin ABI, or general command runner was introduced. AWS and vLLM remain declarative and unimplemented.

There are now two execution envelopes around that same public CLI path:

```text
operator shell ───────────────────────────────┐
                                              ▼
systemctl start → NixOS oneshot service → benchplane run → benchplane-core
```

The service supplies a pinned package containing the CLI, both helpers, CPU-only llama.cpp, and the fixed GGUF model, plus a read-only experiment path, persistent output root, unprivileged identity, and systemd activation timeout. The CLI resolves helpers beside its own executable, so execution does not depend on ambient `PATH`; the inference helper reads its model through a compiled immutable Nix-store reference. The service does not introduce a daemon, RPC endpoint, queue, database, or controller-to-runner protocol. Explicit activation is an orchestration boundary: the unit is not boot-enabled, and every completed activation leaves the oneshot inactive so a later explicit start is a new run.

Run lifecycle, attempt provenance, attempt outcome, measurement validity, and evidence publication are separate concerns. After entering attempt preparation, Benchplane records a bounded allowlist of platform and software facts beneath that attempt before concrete execution starts. Provenance is not run-global: a future retry design could execute another attempt on a different host or package. An attempt becomes terminal when concrete execution returns; collection and finalization belong to the enclosing run. Evidence format v1 deliberately contains one attempt and provides neither retry nor resume semantics.

Infrastructure and node configuration are implementation surfaces around the core lifecycle:

```text
OpenTofu root → high-level experiment module → ephemeral node
                                               │
                                               ▼
                                 composed NixOS capability modules
```

Experiments and studies depend on the framework. Framework code does not import, embed, or otherwise depend on files under `experiments/` or `studies/`. Repository checks may run catalog examples through the packaged public CLI.
