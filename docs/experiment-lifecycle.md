# Experiment lifecycle

Benchplane parses, semantically validates, and resolves an experiment before allocating a run ID. Invalid input is a rejected request, not a run.

A run is one logical invocation of a resolved plan. This first slice creates exactly one attempt, numbered `1` and stored under `attempts/0001`; it does not retry or resume. Re-running the same plan creates a new canonical `run-<lowercase hyphenated UUIDv7>` identity. A process crash may leave diagnostic staging data, but finalized bundles are immutable and are never reopened for continuation. Retry semantics will be designed only when a concrete acquisition or execution failure requires them.

## Run states

```text
created → preparing

preparing → running | finalizing
running → collecting | finalizing
collecting → finalizing
finalizing → succeeded | failed | interrupted
```

Every other transition is rejected with `lifecycle.invalidTransition`, and a rejected transition does not append an event. `succeeded`, `failed`, and `interrupted` are terminal states.

`events.jsonl` is the authoritative append-only chronology. Each event records the run ID, a monotonic sequence number, UTC RFC 3339 millisecond timestamp, previous and next state, attempt number, and optional failure. `run.json` and `attempt.json` are mutable convenience projections written through temporary sibling files and atomic rename.

Attempt status has the smaller `created → preparing → running → succeeded | failed | interrupted` model. The attempt becomes terminal as soon as local-fake execution returns. Run-level `collecting` and `finalizing` therefore occur only after the attempt is terminal; an evidence-publication failure does not retroactively change a successfully executed attempt. This preserves four distinct dimensions: attempt outcome, run outcome, measurement validity, and evidence publication outcome.

## Operational outcome and validity

Run state and measurement validity are separate:

| Local-fake scenario | Run state | Validity | Meaning |
|---|---|---|---|
| `success` | `succeeded` | `valid` | All configured measurements were emitted. |
| `runtimeFailure` | `failed` | `indeterminate` | A deterministic partial prefix may exist, but execution failed. |
| `interrupted` | `interrupted` | `indeterminate` | A deterministic partial prefix may exist, but execution was interrupted. |
| `insufficientMeasurements` | `succeeded` | `invalid` | Execution completed normally with fewer measured repetitions than required. |

Failures use stable codes and are recorded on the terminal event and snapshots. After run allocation, reported persistence and publication errors include the run ID, failure phase, and retained staging path. Diagnostic updates are best effort and never replace the original error.

This slice does not install signal handlers. The `interrupted` local-fake scenario exercises lifecycle and evidence semantics only; it does not demonstrate SIGINT, SIGTERM, process supervision, or host-shutdown behavior. Real signal handling is deferred to a later lifecycle-hardening milestone.
