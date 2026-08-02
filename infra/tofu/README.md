# OpenTofu layout

- `bootstrap/`: one-time remote-state foundation.
- `account/`: long-lived artifact, IAM/OIDC, budget, and optional shared-network resources.
- `modules/aws-experiment/`: one high-level ephemeral experiment module.
- `experiment/`: root module invoked per run with an isolated state key.

The initial scaffold intentionally creates no cloud resources. Add resources only with explicit lifetime, budget, state isolation, evidence finalization, and teardown behavior.
