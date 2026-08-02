# Security model

Credentials remain ambient and short-lived:

- local operation: AWS IAM Identity Center or role assumption;
- GitHub Actions: OIDC to a narrowly scoped role;
- experiment node: EC2 instance profile;
- gated model access: runtime retrieval into tmpfs or systemd credentials.

Credentials must not appear in Nix derivations, the Nix store, OpenTofu variables/state, experiment YAML, logs, or public evidence.
