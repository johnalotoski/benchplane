# Security model

Credentials remain ambient and short-lived:

- local operation: AWS IAM Identity Center or role assumption;
- GitHub Actions: OIDC to a narrowly scoped role;
- experiment node: EC2 instance profile;
- gated model access: runtime retrieval into tmpfs or systemd credentials.

Credentials must not appear in Nix derivations, the Nix store, OpenTofu variables/state, experiment YAML, logs, or public evidence.

Evidence checksums detect payload changes relative to the included checksum inventory and support internal consistency checks. They do not authenticate a publisher, host, or workflow, and a party able to replace the complete bundle can recompute the checksums. Signing and provenance attestations are outside the current local execution scope.

AWS and vLLM are declarative-only schema variants. The executable local paths neither access cloud credentials nor start those runtimes; unsupported combinations are rejected before run allocation.

The CPU probe is a fixed executable located beside the public CLI in the selected package. Experiment input cannot choose executable paths, shell commands, arbitrary arguments, environment variables, working directories, files, or network addresses. Benchplane spawns it without a shell, accepts only validated numeric controls, incrementally bounds and validates its output, and kills and reaps it at the inner deadline. This proves a narrow supervised local child boundary, not a public process ABI or general sandbox.

The llama.cpp runtime uses the same package-sibling selection boundary but remains runtime-specific. The package pins CPU-only llama.cpp `b10133` and an Apache-2.0 SmolLM2-135M-Instruct Q2_K GGUF at commit `c33bd7b3a0c1c5048af630f0198eb2a29977b422`, SHA-256 `55aa88ddac43adce6af0e9be8d6cdff2337a3835cd9b50bbcd7a894eb66dfc75`. Its helper has a compiled immutable model reference and accepts only bounded requests, records, and output-token counts. Benchplane starts this child with an empty environment so dynamic-loader or ggml backend variables cannot redirect the packaged boundary or alter measured work. Experiments cannot select model/prompt paths, URLs, repositories, revisions, sampler options, threads, environment, working directory, network address, or arbitrary llama.cpp arguments. The helper emits metrics only, never generated text or prompt contents, and independently enforces prompt and total-token ceilings.

The shared private supervisor performs shell-free spawn with null stdin, bounded stdout and retained stderr, deadline polling, termination, reaping, and complete-sequence success. A malformed record, inconsistent identity/order/count, excessive output, nonzero exit, or deadline discards the entire parsed prefix. A distinct helper exit status maps model-load failure to `llamaCpp.modelInitFailed`; diagnostics remain bounded. This internal reuse is not exported as a process API or plugin surface.

## NixOS runner boundary

The NixOS runner uses the module-managed `benchplane` system user and group, has no writable home, and receives its persistent output directory through systemd `StateDirectory=`. The experiment is a read-only Nix store path. Because Nix copies that file into the world-readable store, it must never contain credentials, tokens, private account identifiers, or other secrets.

The oneshot enables `NoNewPrivileges`, private temporary storage, protected system and home paths, empty capability sets, set-user-ID restrictions, protected kernel tunables/modules/control groups, and a restrictive umask. This is a conservative baseline, not a claim that the runner is fully sandboxed. It deliberately does not prohibit all network access, subprocesses, model-file access, or devices, because later concrete runtimes may need narrowly reviewed access to those facilities.

The current local service performs no credential retrieval, upload, host shutdown, or cloud teardown. Its CPU probe performs no file or network I/O beyond its inherited standard streams. Packaged inference reads only the public Nix-store model and linked libraries plus its inherited standard streams; it performs no execution-time network access. The VM integration check uses an isolated test network and no external runtime network access. Future credentials must use an out-of-store runtime delivery mechanism rather than declarative environment variables, unit text, derivations, or experiment files.
