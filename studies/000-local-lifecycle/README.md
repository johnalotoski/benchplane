# Study 000: local lifecycle

## Question

Can Benchplane deterministically move a local fake experiment from specification through validation, resolution, evidence generation, finalization, and verification without cloud credentials or accelerator hardware?

## Exit criteria

- schema generation is reproducible;
- the same requested document resolves to the same digest;
- local execution creates a complete miniature evidence bundle;
- interrupted and failed fake attempts remain representable;
- bundle verification rejects tampering;
- ordinary CI exercises the entire path.
