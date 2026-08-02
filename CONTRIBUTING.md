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
nix develop
just fmt
just check
just tofu-validate
```

The first two commands are local and deterministic. `just tofu-validate` may download the locked provider, but it must not use AWS credentials, contact AWS APIs, run a plan, or create resources.

The supported development, CI, and release environment is the Nix shell pinned by `flake.lock`. Commands may instead be run as `nix develop -c <command>`. Direct Cargo or rustup use outside that environment is currently best-effort.

Cloud tests must be manually triggered, cost-bounded, and designed to tear down in an unconditional cleanup path.

## Architectural rule

Reusable code in `crates/`, `nix/`, and `infra/tofu/modules/` must not depend on experiment catalogs or study reports. Experiment- and study-specific material may depend on the reusable framework.
