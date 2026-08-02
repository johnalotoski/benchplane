# Security model

Credentials remain ambient and short-lived:

- local operation: AWS IAM Identity Center or role assumption;
- GitHub Actions: OIDC to a narrowly scoped role;
- experiment node: EC2 instance profile;
- gated model access: runtime retrieval into tmpfs or systemd credentials.

Credentials must not appear in Nix derivations, the Nix store, OpenTofu variables/state, experiment YAML, logs, or public evidence.

Evidence checksums detect payload changes relative to the included checksum inventory and support internal consistency checks. They do not authenticate a publisher, host, or workflow, and a party able to replace the complete bundle can recompute the checksums. Signing and provenance attestations are outside the current local-fake lifecycle scope.

AWS and vLLM are declarative-only schema variants. The current executable path neither accesses cloud credentials nor starts those runtimes; unsupported combinations are rejected before run allocation.

## NixOS runner boundary

The NixOS runner uses the module-managed `benchplane` system user and group, has no writable home, and receives its persistent output directory through systemd `StateDirectory=`. The experiment is a read-only Nix store path. Because Nix copies that file into the world-readable store, it must never contain credentials, tokens, private account identifiers, or other secrets.

The oneshot enables `NoNewPrivileges`, private temporary storage, protected system and home paths, empty capability sets, set-user-ID restrictions, protected kernel tunables/modules/control groups, and a restrictive umask. This is a conservative baseline, not a claim that the runner is fully sandboxed. It deliberately does not prohibit all network access, subprocesses, model-file access, or devices, because later concrete runtimes may need narrowly reviewed access to those facilities.

The current local-fake service performs no credential retrieval, upload, host shutdown, or cloud teardown. Its VM integration check uses an isolated test network and no external runtime network access. Future credentials must use an out-of-store runtime delivery mechanism rather than declarative environment variables, unit text, derivations, or experiment files.
