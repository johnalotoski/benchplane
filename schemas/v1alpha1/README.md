# Generated schemas

`experiment.schema.json` is generated from the Rust types in `benchplane-schema` and must be updated with:

```console
just schema
```

`just schema-check`, `just check`, and `nix flake check` fail when regeneration changes the checked-in schema. Closed objects reject unknown fields; intentional maps such as `metadata.labels` remain open. See `docs/experiment-schema.md` for the boundary between JSON Schema restrictions and Benchplane semantic validation.
