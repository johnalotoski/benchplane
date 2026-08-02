# Task 001: make the scaffold green

Status: completed for the bootstrap scaffold. This document is retained as the original acceptance checklist.

## Objective

Turn the supplied starter scaffold into a clean first public commit without broadening scope.

## Required work

1. Inspect `AGENTS.md`, the ADRs, and the complete repository before editing.
2. Replace `OWNER` in Cargo metadata with the actual GitHub owner or remove the repository field until known.
3. Restore the full canonical Apache-2.0 license text.
4. Run `cargo generate-lockfile` and commit `Cargo.lock`.
5. Build, test, format, and lint the Rust workspace. Fix all compiler and Clippy issues rather than weakening checks.
6. Verify the YAML enum representation used by both smoke examples.
7. Generate `schemas/v1alpha1/experiment.schema.json` from the Rust types; remove the placeholder comment.
8. Add invalid schema fixtures covering API version, zero requests, empty AWS instance types, invalid budgets, and excessive runtime.
9. Add CLI integration tests for `validate`, `resolve`, `schema export`, and tamper-resistant `evidence verify`.
10. Implement the smallest deterministic local-fake bundle fixture needed to prove verification, but do not yet add a public `run` command unless it performs a real complete local lifecycle.
11. Make the flake expose the Rust package and meaningful checks once `Cargo.lock` exists.
12. Validate Nix syntax/module evaluation and OpenTofu formatting/validation.
13. Keep ordinary CI independent of AWS and GPUs.
14. Update README status accurately and add no claims that have not been tested.

## Explicit non-goals

Do not add AWS resources, vLLM packaging, CUDA, containers, Kubernetes, a web UI, multi-cloud abstractions, a plugin ABI, a database, or benchmark conclusions.

## Completion report

At the end, report:

- files and behavior changed;
- commands executed and their results;
- anything that could not be tested in the environment;
- remaining risks before the first public push;
- a suggested coherent commit sequence rather than one giant commit.
