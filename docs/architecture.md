# Architecture

Benchplane begins as a modular monolith.

```text
human-authored experiment
        │
        ▼
benchplane-schema ── validation and deterministic public data model
        │
        ▼
benchplane-core ─── resolution, lifecycle, providers, runtimes, evidence, analysis
        │
        ▼
benchplane CLI ───── user interaction and machine-readable output
```

Infrastructure and node configuration are implementation surfaces around the core lifecycle:

```text
OpenTofu root → high-level experiment module → ephemeral node
                                               │
                                               ▼
                                 composed NixOS capability modules
```

Experiments and studies depend on the framework. The framework does not depend on the catalog or reports.
