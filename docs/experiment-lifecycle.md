# Experiment lifecycle

Benchplane parses, semantically validates, and resolves an experiment before allocating a run ID. Invalid input is a rejected request, not a run.

A run is one logical invocation of a resolved plan. This first slice creates exactly one attempt, numbered `1` and stored under `attempts/0001`; it does not retry or resume. Re-running the same plan creates a new `run-<lowercase UUIDv7>` identity.

## Run states

```text
created → preparing

preparing → running | finalizing
running → collecting | finalizing
collecting → finalizing
finalizing → succeeded | failed | interrupted
```

Every other transition is rejected with `lifecycle.invalidTransition`, and a rejected transition does not append an event. `succeeded`, `failed`, and `interrupted` are terminal states.

`events.jsonl` is the authoritative append-only chronology. Each event records a monotonic sequence number, UTC RFC 3339 millisecond timestamp, previous and next state, attempt number, and optional failure. `run.json` and `attempt.json` are mutable convenience projections written through temporary sibling files and atomic rename.

## Operational outcome and validity

Run state and measurement validity are separate:

| Local-fake scenario | Run state | Validity | Meaning |
|---|---|---|---|
| `success` | `succeeded` | `valid` | All configured measurements were emitted. |
| `runtimeFailure` | `failed` | `indeterminate` | A deterministic partial prefix may exist, but execution failed. |
| `interrupted` | `interrupted` | `indeterminate` | A deterministic partial prefix may exist, but execution was interrupted. |
| `insufficientMeasurements` | `succeeded` | `invalid` | Execution completed normally with fewer measured repetitions than required. |

Failures use stable codes and are recorded on the terminal event and snapshots. This slice does not install signal handlers; interruption is an explicit local-fake scenario used to exercise the terminal path.
