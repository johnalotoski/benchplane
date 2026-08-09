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

Attempt status has the smaller `created → preparing → running → succeeded | failed | interrupted` model. During `preparing`, after the attempt snapshot exists and before the transition to `running`, Benchplane captures and persists bounded attempt-scoped execution provenance. An unavailable nonessential host fact is recorded as absent; a provenance persistence failure follows the existing allocated I/O failure path and does not create a runtime failure category. The attempt becomes terminal as soon as concrete execution returns. Run-level `collecting` and `finalizing` therefore occur only after the attempt is terminal; an evidence-publication failure does not retroactively change a successfully executed attempt. This preserves the distinct dimensions of attempt provenance and outcome, run outcome, measurement validity, and evidence publication outcome.

## Operational outcome and validity

Run state and measurement validity are separate:

| Local-fake scenario | Run state | Validity | Meaning |
|---|---|---|---|
| `success` | `succeeded` | `valid` | All configured measurements were emitted. |
| `runtimeFailure` | `failed` | `indeterminate` | A deterministic partial prefix may exist, but execution failed. |
| `interrupted` | `interrupted` | `indeterminate` | A deterministic partial prefix may exist, but execution was interrupted. |
| `insufficientMeasurements` | `succeeded` | `invalid` | Execution completed normally with fewer measured repetitions than required. |

Failures use stable codes and are recorded on the terminal event and snapshots. After run allocation, reported persistence and publication errors include the run ID, failure phase, and retained staging path. Diagnostic updates are best effort and never replace the original error.

A successful CPU probe follows `created → preparing → running → collecting → finalizing → succeeded`. Spawn failure, malformed or excessive output, nonzero child exit, or runtime deadline makes the attempt and run `failed`, validity `indeterminate`, and CLI status `4`; normal evidence finalization and publication still proceed when storage works. CPU-probe output is accepted only after the complete child protocol succeeds. If the child fails after emitting a valid prefix, that prefix is discarded and the finalized failed evidence contains no CPU-probe measurements.

The packaged llama.cpp path follows the same successful transition sequence and complete-protocol rule. Its stable runtime failures distinguish `llamaCpp.spawnFailed`, `llamaCpp.modelInitFailed`, `llamaCpp.outputInvalid`, `llamaCpp.exitFailed`, and `llamaCpp.deadlineExceeded`. Each produces a failed attempt/run and indeterminate validity after allocation, then proceeds through evidence-v1 finalization when storage remains operational. A later publication failure remains separate and never rewrites an already successful attempt.

This slice does not install signal handlers. The `interrupted` local-fake scenario exercises lifecycle and evidence semantics only; it does not demonstrate SIGINT, SIGTERM, process supervision, or host-shutdown behavior. Real signal handling is deferred to a later lifecycle-hardening milestone.

## NixOS service activation and timeout

`benchplane-runner.service` is an explicitly started oneshot. It is not wanted by `multi-user.target`, so boot does not imply retry, resume, or a new experiment. After a completed oneshot becomes inactive, each later `systemctl start benchplane-runner.service` invokes `benchplane run` again and allocates a new run ID.

The CPU-probe and llama.cpp adapters apply the experiment's resolved `lifecycle.maximumRuntimeSeconds` as an inner child deadline. They kill and wait for an overdue child, record a runtime failure, and finalize failed evidence when possible. This is child supervision, not parent signal handling or a graceful interruption.

Separately, the module maps its runner maximum to systemd `TimeoutStartSec`, which bounds the whole `ExecStart` while the oneshot is activating. That outer watchdog should be comfortably longer than the experiment's inner deadline. Benchplane does not yet handle SIGINT or SIGTERM; if systemd times out and terminates the parent, staging data may remain and no finalized interrupted evidence bundle is guaranteed. Only the local-fake `interrupted` scenario records an `interrupted` lifecycle outcome.
