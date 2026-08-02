# ADR 0001: begin with one repository

## Status

Accepted.

## Decision

Begin with one modular-monolith repository containing CLI, schema, core lifecycle, Nix, OpenTofu, experiments, and studies.

## Rationale

The initial boundaries will evolve together, cross-cutting changes should remain atomic, and a single commit should identify the framework and study inputs used for a result.

## Extraction rule

Extract a component only after a second real consumer exists and the interface has demonstrated stability.
