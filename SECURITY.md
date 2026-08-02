# Security policy

Benchplane handles cloud infrastructure, temporary credentials, runtime logs, and public evidence. Security mistakes can therefore become expensive or disclose sensitive data.

## Reporting a vulnerability

Please do not report security vulnerabilities through public GitHub issues.

Use GitHub's private vulnerability reporting feature by opening the repository's Security tab and selecting **Report a vulnerability**.

## Non-negotiable repository rules

- Never commit AWS access keys, session tokens, model-access tokens, `.env` files, OpenTofu state, SSH private keys, or pre-signed URLs.
- Treat anything placed in the Nix store as non-secret.
- Treat OpenTofu plans and state as potentially sensitive.
- Use short-lived local credentials, GitHub OIDC, and EC2 instance profiles.
- Sanitize public evidence bundles and test redaction in CI.
- Require explicit cost and lifetime limits before cloud execution.
