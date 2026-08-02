# Security policy

Benchplane handles cloud infrastructure, temporary credentials, runtime logs, and public evidence. Security mistakes can therefore become expensive or disclose sensitive data.

## Reporting a vulnerability

Do not open a public issue containing credentials, exploitable details, private cloud identifiers, or sensitive logs. Until a private reporting channel is configured, contact the repository owner through the private contact method listed on their GitHub profile.

## Non-negotiable repository rules

- Never commit AWS access keys, session tokens, model-access tokens, `.env` files, OpenTofu state, SSH private keys, or pre-signed URLs.
- Treat anything placed in the Nix store as non-secret.
- Treat OpenTofu plans and state as potentially sensitive.
- Use short-lived local credentials, GitHub OIDC, and EC2 instance profiles.
- Sanitize public evidence bundles and test redaction in CI.
- Require explicit cost and lifetime limits before cloud execution.
