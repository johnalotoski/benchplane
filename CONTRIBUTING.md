# Contributing

Benchplane is currently a design-first early-stage project.

## Workflow

- Create a focused feature branch.
- Commit frequently when the commits capture useful checkpoints.
- Open a draft pull request early for non-trivial work.
- Preserve coherent commits; squash only noisy or misleading intermediate work.
- Keep `main` green.

## Required checks

```console
just fmt
just check
```

Cloud tests must be manually triggered, cost-bounded, and designed to tear down in an unconditional cleanup path.

## Architectural rule

Reusable code in `crates/`, `nix/`, and `infra/tofu/modules/` must not depend on experiment catalogs or study reports. Experiment- and study-specific material may depend on the reusable framework.
