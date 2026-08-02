# Security model

Credentials remain ambient and short-lived:

- local operation: AWS IAM Identity Center or role assumption;
- GitHub Actions: OIDC to a narrowly scoped role;
- experiment node: EC2 instance profile;
- gated model access: runtime retrieval into tmpfs or systemd credentials.

Credentials must not appear in Nix derivations, the Nix store, OpenTofu variables/state, experiment YAML, logs, or public evidence.

Evidence checksums detect payload changes relative to the included checksum inventory and support internal consistency checks. They do not authenticate a publisher, host, or workflow, and a party able to replace the complete bundle can recompute the checksums. Signing and provenance attestations are outside the current local-fake lifecycle scope.

AWS and vLLM are declarative-only schema variants. The current executable path neither accesses cloud credentials nor starts those runtimes; unsupported combinations are rejected before run allocation.
