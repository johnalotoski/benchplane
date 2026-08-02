# Architecture

Benchplane begins as a modular monolith.

```text
human-authored experiment
        │
        ▼
benchplane-schema ── strict public experiment and evidence records
        │
        ▼
benchplane-core ─── parsing, resolution, lifecycle, execution, validity,
                    summaries, evidence writing, and verification
        │
        ▼
benchplane CLI ───── thin command dispatch and human/JSON presentation
```

The executable local path dispatches directly on two concrete combinations: `localFake`/`localFake` runs in process, while `local`/`cpuProbe` supervises a fixed helper executable supplied by the same package. The experiment cannot select a path, command, arguments, environment, working directory, or address. This second implementation justified sharing only execution outcome, validity, and summary logic; it did not introduce provider/runtime traits, a plugin ABI, or a general command runner. AWS and vLLM remain declarative and unimplemented.

There are now two execution envelopes around that same public CLI path:

```text
operator shell ───────────────────────────────┐
                                              ▼
systemctl start → NixOS oneshot service → benchplane run → benchplane-core
```

The service supplies a pinned package containing both binaries, a read-only experiment path, a persistent output root, an unprivileged identity, and a systemd activation timeout. The CLI resolves its CPU-probe helper beside its own executable, so neither executable depends on ambient `PATH`. The service does not introduce a daemon, RPC endpoint, queue, database, or controller-to-runner protocol. Explicit activation is an orchestration boundary: the unit is not boot-enabled, and every completed activation leaves the oneshot inactive so a later explicit start is a new run.

Run lifecycle, attempt outcome, measurement validity, and evidence publication are separate concerns. An attempt becomes terminal when concrete execution returns; collection and finalization belong to the enclosing run. Evidence format v1 deliberately contains one attempt and provides neither retry nor resume semantics.

Infrastructure and node configuration are implementation surfaces around the core lifecycle:

```text
OpenTofu root → high-level experiment module → ephemeral node
                                               │
                                               ▼
                                 composed NixOS capability modules
```

Experiments and studies depend on the framework. Framework code does not import, embed, or otherwise depend on files under `experiments/` or `studies/`. Repository checks may run catalog examples through the packaged public CLI.
