# NixOS runner service

The Benchplane NixOS module runs the existing public `benchplane run EXPERIMENT` command inside a systemd oneshot. It proves an execution envelope for the local-fake lifecycle; it is not a daemon or a controller-to-runner transport.

## Configuration

Import `nixosModules.default` and configure a secret-free experiment path:

```nix
{
  inputs,
  pkgs,
  ...
}:
{
  imports = [ inputs.benchplane.nixosModules.default ];

  services.benchplane = {
    enable = true;
    runner = {
      enable = true;
      package = inputs.benchplane.packages.${pkgs.system}.default;
      experimentFile = ./experiment.yaml;
    };
    lifecycle.maximumRuntimeSeconds = 300;
  };
}
```

The relevant options are:

| Option | Meaning |
|---|---|
| `services.benchplane.enable` | Creates the unprivileged service account and enables shared module configuration. |
| `services.benchplane.runner.enable` | Defines `benchplane-runner.service`. |
| `services.benchplane.runner.package` | Required package containing `bin/benchplane`; the unit uses its absolute Nix store path. |
| `services.benchplane.runner.experimentFile` | Required read-only experiment path copied to the Nix store. |
| `services.benchplane.lifecycle.maximumRuntimeSeconds` | Positive activation timeout, at most 86,400 seconds; defaults to 3,600. |
| `services.benchplane.stateDirectory` | One safe `StateDirectory` component; defaults to `benchplane`. |

The existing `user`, `group`, and `stateDirectory` names remain configurable for module composition, but each accepts only a restricted single-component value. The standard configuration uses the `benchplane` system user and group and `/var/lib/benchplane`.

An experiment passed as a Nix path enters the Nix store. Never put credentials or secrets in that file. This local-fake configuration needs none.

## Operation

The service is intentionally not attached to a boot target. Start one run explicitly:

```console
sudo systemctl start benchplane-runner.service
```

The command blocks until the oneshot succeeds or fails. After completion, inspect its status and journal:

```console
systemctl status benchplane-runner.service
journalctl -u benchplane-runner.service
```

Systemd sends the CLI's human-readable standard output and diagnostics to the journal. The unit uses no ambient `PATH`: `ExecStart` references the configured package and experiment by absolute Nix store paths and supplies `/var/lib/benchplane` as `--output-root`.

Each explicit start after the prior oneshot finishes is a new `benchplane run` invocation and creates a new canonical UUIDv7 run ID. Boot is not an automatic run, retry, or resume. The service has `Restart=no`, performs no automatic shutdown, and does not upload or tear down anything.

## Evidence and exit status

Successful bundles are published beneath:

```text
/var/lib/benchplane/runs/<run-id>
```

Verify one with the same packaged CLI:

```console
benchplane evidence verify /var/lib/benchplane/runs/<run-id>
```

While a run is active, or after some process/finalization failures, diagnostic data may exist beneath `/var/lib/benchplane/staging/<run-id>`. A staging directory is not finalized evidence and must not be uploaded, cited, or treated as a successful run.

Systemd preserves the Benchplane process status. It does not reinterpret nonzero terminal outcomes:

| CLI status | Meaning | Unit result |
|---:|---|---|
| 0 | Run succeeded and evidence is valid. | Success |
| 1 | Internal error, including an unreadable experiment. | Failure |
| 2 | Experiment request rejected during parsing, validation, or resolution. | Failure |
| 3 | Execution succeeded but evidence validity is invalid or indeterminate. | Failure |
| 4 | Runtime failed or the scenario recorded interruption, even if evidence was published. | Failure |
| 5 | Evidence finalization, publication, or verification failed. | Failure |

## Runtime limit and security boundary

`maximumRuntimeSeconds` configures `TimeoutStartSec`, the systemd timeout that bounds a oneshot while its `ExecStart` is activating. This watchdog may terminate Benchplane forcibly. Benchplane does not yet implement operating-system signal handling, so a timeout is not a graceful `interrupted` lifecycle outcome: it may leave diagnostic staging data and may produce no final bundle.

The service runs as a static, unprivileged system account because the existing NixOS capability modules share that identity. Systemd owns the persistent state directory with mode `0750`; the process receives a `0027` umask and no writable home. The module also applies a conservative hardening baseline described in the [security model](security-model.md).

These controls do not define a fully isolated sandbox. The module leaves network, subprocess, model-file, and device policy available for later concrete runtime composition. The current runner defines no controller protocol, provider abstraction, RPC service, queue, retry state, upload destination, credential map, hook, or plugin interface.

## Integration test

Run the focused cost-free NixOS VM integration test with local compute:

```console
just nixos-runner-test
```

The test is also part of `nix flake check`. It builds its own local-fake experiment into the store, boots one NixOS VM, explicitly starts the service, verifies the published bundle with the packaged binary, and checks identity, ownership, status, counts, digests, checksums, and empty staging state. It requires no cloud account, AWS API access, GPU, model download, container runtime, or external runtime service; after its Nix closure is available, the VM uses no external network access.
