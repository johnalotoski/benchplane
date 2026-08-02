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

The executable local path dispatches directly to the concrete local-fake implementation. The declarative AWS provider and vLLM runtime remain unimplemented; Benchplane will not introduce generalized provider or runtime interfaces until another real lifecycle demonstrates the required boundary.

Run lifecycle, attempt outcome, measurement validity, and evidence publication are separate concerns. An attempt becomes terminal when concrete execution returns; collection and finalization belong to the enclosing run. Evidence format v1 deliberately contains one attempt and provides neither retry nor resume semantics.

Infrastructure and node configuration are implementation surfaces around the core lifecycle:

```text
OpenTofu root → high-level experiment module → ephemeral node
                                               │
                                               ▼
                                 composed NixOS capability modules
```

Experiments and studies depend on the framework. Framework code does not import, embed, or otherwise depend on files under `experiments/` or `studies/`. Repository checks may run catalog examples through the packaged public CLI.
