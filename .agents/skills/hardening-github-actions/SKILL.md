---
name: hardening-github-actions
description: >
  Creates, modifies, and reviews GitHub Actions workflows and composite
  actions using secure defaults. Use whenever working with files under
  .github/workflows/ or .github/actions/, including CI, releases, AWS OIDC,
  GPU jobs, scheduled benchmarks, reusable workflows, artifact handling,
  container publication, and deployment automation.
---

# Hardening GitHub Actions

Apply this procedure before editing to establish the threat model and after
editing to audit the completed diff. Treat workflow security as a set of trust
boundaries, not a checklist that makes every job equally privileged.

## Review procedure

For every workflow or composite-action change:

1. Identify the workflow's purpose and the authority it needs.
2. Identify every trigger and whether an untrusted user can cause it to run.
3. Inventory untrusted inputs, including event data, caller inputs, matrix
   values, downloaded artifacts, caches, and checked-out code.
4. Inventory permissions, secrets, credentials, environments, and protected
   resources at workflow and job scope.
5. Identify each runner's trust, persistence, network reach, and cleanup model.
6. Map artifact, cache, reusable-workflow, and `workflow_run` transfers across
   trust or privilege boundaries.
7. Identify cost, timeout, interruption, and teardown risks.
8. Review every external action and reusable workflow, including its immutable
   pin, inputs, credentials, and effective permissions.
9. Make the smallest changes that address the actual risks without disabling
   useful unprivileged pull-request CI.
10. Run syntax, repository, and security-focused validation.
11. Reapply this procedure to the final diff and report residual risks and
    assumptions.

## Permissions and triggers

Declare least privilege at workflow scope. Prefer one of:

```yaml
permissions:
  contents: read
```

```yaml
permissions: {}
```

Grant additional authority only to the specific job that needs it. Explicitly
review every grant of `id-token: write`, `contents: write`, `packages: write`,
`pull-requests: write`, `issues: write`, `actions: write`, or
`security-events: write`. Confirm that the trigger and permissions are safe in
combination, not merely safe when reviewed separately.

Use `pull_request` for ordinary CI that tests untrusted fork code on ephemeral
GitHub-hosted runners with read-only permissions and no secrets. Do not add a
trust gate solely because such CI executes pull-request code. Add a job-level
trust gate before exposing repository or environment secrets, write-capable
tokens, AWS OIDC, deployment authority, persistent or self-hosted runners,
protected artifacts, production systems, or paid infrastructure beyond an
explicitly accepted public-CI allowance.

Do not use `pull_request_target` to check out or execute untrusted pull-request
code. If that trigger is unavoidable for metadata-only automation, operate on
trusted base-branch code and treat all pull-request content as data.

For manual, scheduled, issue-command, and API triggers, verify who can invoke
the workflow and who can change its inputs. Use protected GitHub environments
and approval where the job has publication, deployment, or cost-bearing
authority.

## Action and reusable-workflow pinning

Pin every third-party action and externally hosted reusable workflow to a full,
immutable commit SHA. Keep the intended release tag as a comment:

```yaml
- uses: actions/checkout@<full-commit-sha> # v4
```

Do not keep action SHAs in this skill. When editing, resolve the intended tag
from the authoritative upstream repository, account for annotated tags by
using the dereferenced commit, and verify that the full SHA matches the
expected release. Report the verification source. Review updates deliberately;
an immutable pin still runs third-party code.

For `actions/checkout`, set:

```yaml
with:
  persist-credentials: false
```

Retain credentials only when a narrowly scoped trusted job demonstrably needs
Git access after checkout. Set an explicit fetch depth when history or tags are
required; otherwise avoid fetching unnecessary history.

## Expressions, shells, and outputs

Treat GitHub contexts, workflow inputs, artifact metadata, branch and tag
names, issue and pull-request bodies or titles, comments, labels, and matrix
values as untrusted. Never interpolate untrusted expressions directly into a
shell block:

```yaml
# Unsafe
- run: command "${{ github.event.some.untrusted.value }}"
```

Pass the value through `env:` and quote the shell variable:

```yaml
- env:
    UNTRUSTED_VALUE: ${{ github.event.some.untrusted.value }}
  run: command "$UNTRUSTED_VALUE"
```

Expressions in `if:`, `with:`, `env:`, `run-name:`, and concurrency fields do
not undergo the same shell parsing, but still require validation appropriate to
their consumer. Choose an explicit shell when behavior matters. In multiline
Bash, normally begin with `set -euo pipefail`; strict mode improves failure
handling but does not prevent expression or shell injection.

Pass secrets through `env:` rather than expression substitution inside shell
source. Avoid printing derived secret values, passing secrets on command lines,
or writing them to broadly readable files. Remember that masking is not an
authorization boundary.

When untrusted text may contain newlines, write step outputs using the
documented multiline delimiter form with a collision-resistant delimiter.
Validate restricted values before using the simpler `name=value` form. Treat
values read from outputs as untrusted at the next step.

## Workflow inputs

Validate `workflow_dispatch`, reusable-workflow, issue-command, and similar
inputs before use. Prefer enumerated choices; validate lengths, character sets,
numeric ranges, and path containment. Reject shell fragments and unsafe paths.
Apply policy checks before accepting instance types, regions, commands, Nix
attributes, OpenTofu targets, artifact destinations, maximum cost, or maximum
runtime. Do not turn a nominally constrained input into an arbitrary execution
interface.

## AWS OIDC and cloud authority

For AWS workflows:

- Never grant `id-token: write` to ordinary untrusted pull-request jobs.
- Prefer OIDC to long-lived AWS access keys.
- Scope IAM trust to the intended repository and, as appropriate, workflow,
  branch, tag, and protected GitHub environment.
- Use protected environments for paid GPU runs and releases when appropriate.
- Keep assumed roles least-privileged and session durations short.
- Avoid publishing account IDs, role ARNs, and infrastructure identifiers
  without a reason.
- Never put cloud credentials in artifacts, caches, Nix derivations or store
  paths, logs, OpenTofu output, or evidence bundles.

Review both the workflow permissions and the external IAM trust policy. A
well-scoped GitHub token does not compensate for an overbroad cloud role.

## Cost-bearing infrastructure

For any workflow that can provision AWS, Spot or On-Demand instances, GPU
capacity, or another paid resource, require:

- a trusted manual trigger or another explicitly approved trigger;
- strict concurrency limits and a hard job timeout;
- experiment-level maximum runtime and maximum cost controls;
- resource TTL and ownership tags;
- unconditional cleanup in an `always()` step;
- an independent instance-side termination safeguard;
- clear failure reporting and an operator recovery path when cleanup fails;
- no provisioning from ordinary pull-request CI.

A GitHub Actions timeout can terminate the cleanup process too. It is not, by
itself, teardown protection. Do not cancel an older run if cancellation could
interrupt publication or infrastructure cleanup without a safe recovery path.
For ordinary stateless CI, use concurrency cancellation when it reduces stale
work safely.

## Self-hosted and GPU runners

Give self-hosted runners special scrutiny. Never run arbitrary fork code on a
persistent self-hosted or GPU runner. Prefer ephemeral, single-job runners and
destroy or reimage them after use. Assume credentials, files, containers,
processes, mounts, caches, and network access can survive a job unless the
runner lifecycle proves otherwise. Avoid privileged containers and host socket
exposure. Ensure cloud and model credentials cannot survive the job.

## Artifacts, evidence, and caches

Treat downloaded artifacts as untrusted, especially across workflow
boundaries. Do not execute an artifact produced by an untrusted job inside a
privileged job. Where practical, verify checksums, provenance, producer
workflow, repository, run, commit, and artifact identity. Extract archives with
path-traversal and symlink defenses.

Before upload or publication, exclude secrets, OpenTofu state and plans,
signed URLs, credentials, private host metadata, model tokens, and unnecessary
cloud identifiers. Set explicit retention periods. State accurately that a
checksum demonstrates integrity relative to the checksum source, not publisher
authenticity.

Treat caches as a possible poisoning boundary. Use narrowly scoped keys, never
cache secrets or credential-bearing directories, and do not consume
fork-controlled caches in privileged release or deployment jobs without
validation. Successful restoration does not make a cache trusted.

## `workflow_run` boundaries

Treat `workflow_run` as a possible privilege escalation: an untrusted producer
can feed artifacts or metadata to a consumer that has secrets or write
permissions. Validate the triggering workflow, repository, event type,
conclusion, branch or commit, artifact producer, and original run's trust
status. Never execute artifacts merely because the consuming run is
privileged.

## Reusable workflows

Pin external reusable workflows by immutable commit SHA. Declare their minimum
permissions, avoid broad `secrets: inherit`, and treat caller inputs as
untrusted. Ensure a caller cannot elevate the called workflow's effective
permissions. Document the expected callers, triggers, and trust assumptions.

## Releases, containers, evidence publication, and deployment

For publication or deployment:

- use protected environments or explicit approval where appropriate;
- keep immutable action pins and minimal job-level write permissions;
- validate version, tag, image name, destination, and other release inputs;
- build from the intended immutable commit, not an untrusted checkout;
- publish checksums and provenance appropriate to the artifact;
- keep release, registry, signing, and deployment secrets unavailable to fork
  pull requests.

Separate build and publish authority carefully. If a privileged job consumes a
previous build, apply the artifact-boundary rules rather than assuming the
producer was trusted.

## Nix-specific review

When a workflow evaluates a pull request's flake, inspect `nixConfig`,
substituter settings, builders, and any use of `accept-flake-config` or
`--accept-flake-config`. Do not let untrusted flake configuration redirect
trusted credentials or privileged builds. Keep secrets out of derivation
arguments and the Nix store. Pinning `nixpkgs` does not pin GitHub Actions, and
pinning actions does not make an untrusted derivation safe on a persistent
runner.

## Validation and final audit

Run the repository's normal formatting and checks plus the relevant workflow
checks. Prefer existing maintained tooling and evaluate maintenance cost before
adding a large security toolchain. At minimum, consider:

```console
actionlint
grep -RIn 'pull_request_target' .github
grep -RInE 'uses: .*@(main|master|v[0-9]+)$' .github
grep -RIn '\${{.*}}' .github/workflows
grep -RInE 'permissions:.*write|id-token: write|secrets: inherit' .github
```

Also review multiline `run:` blocks for direct expression interpolation,
checkout steps for `persist-credentials: false`, external reusable workflows
for immutable pins, permissions at both scopes, explicit timeouts for hanging
or costly jobs, concurrency semantics, and artifact/caching boundaries.
Interpret search results: not every expression is shell injection, and not
every write permission is unjustified.

After validation, re-read the final workflow and diff using the full review
procedure. Report commands run, failures or unavailable checks, action-pin
verification, trust assumptions, elevated authority, teardown behavior, and
remaining risks.
