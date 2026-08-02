# Experiment lifecycle

Proposed high-level states:

```text
requested
→ validated
→ resolved
→ acquiring
→ provisioned
→ preparing
→ warming
→ measuring
→ finalizing
→ uploaded
→ terminating
→ complete
```

Terminal alternatives include invalid, acquisition-failed, interrupted, runtime-failed, budget-exceeded, upload-failed, and teardown-failed. Attempts are distinct from runs so failed Spot acquisition and interrupted capacity remain part of the economic record.
