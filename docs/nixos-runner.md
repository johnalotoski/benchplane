# NixOS runner service

The Benchplane NixOS module runs the existing public `benchplane run EXPERIMENT` command inside a systemd oneshot. It exercises both local execution implementations, including the package-owned CPU-probe child; it is not a daemon or a controller-to-runner transport.

## Configuration

Import `nixosModules.default` and configure a secret-free experiment path:

```nix
{
  inputs,
  pkgs,
  ...
}:
let
  benchplanePackage = inputs.benchplane.packages.${pkgs.system}.default;
in
{
  imports = [ inputs.benchplane.nixosModules.default ];

  environment.systemPackages = [ benchplanePackage ];

  services.benchplane = {
    enable = true;
    runner = {
      enable = true;
      package = benchplanePackage;
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
| `services.benchplane.runner.package` | Required package containing `bin/benchplane` and its sibling CPU-probe helper; the unit uses its absolute Nix store path. |
| `services.benchplane.runner.experimentFile` | Required read-only experiment path copied to the Nix store. |
| `services.benchplane.lifecycle.maximumRuntimeSeconds` | Positive activation timeout, at most 86,400 seconds; defaults to 3,600. |
| `services.benchplane.stateDirectory` | One safe `StateDirectory` component; defaults to `benchplane`. |

The existing `user`, `group`, and `stateDirectory` names remain configurable for module composition, but each accepts only a restricted single-component value. The standard configuration uses the `benchplane` system user and group and `/var/lib/benchplane`.

The example binds the selected package once: the service invokes that package by absolute Nix store path, while `environment.systemPackages` makes the same CLI available to operators. An experiment passed as a Nix path enters the Nix store. Never put credentials or secrets in that file. Local-fake and CPU-probe configurations need none.

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
sudo benchplane evidence verify /var/lib/benchplane/runs/<run-id>
```

Administrative access is required because the managed state directory and published run directories are deliberately mode `0750` and owned by the Benchplane service identity. Do not weaken those permissions for interactive verification.

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

For CPU-probe experiments, the experiment's resolved lifecycle maximum is an inner child deadline: Benchplane kills and reaps an overdue helper and can finalize failed evidence. The module's `maximumRuntimeSeconds` separately configures `TimeoutStartSec`, the outer systemd timeout that bounds the entire oneshot while `ExecStart` is activating. Configure the outer timeout comfortably above the inner deadline. The outer watchdog may terminate Benchplane forcibly; without parent signal handling it is not a graceful `interrupted` lifecycle outcome and may leave diagnostic staging data without a final bundle.

The service runs as a static, unprivileged system account because the existing NixOS capability modules share that identity. Systemd owns the persistent state directory with mode `0750`; the process receives a `0027` umask and no writable home. The module also applies a conservative hardening baseline described in the [security model](security-model.md).

These controls do not define a fully isolated sandbox. The module leaves network, subprocess, model-file, and device policy available for later concrete runtime composition. The current runner defines no controller protocol, provider abstraction, RPC service, queue, retry state, upload destination, credential map, hook, or plugin interface.

## Integration test

Run the focused cost-free NixOS VM integration test with local compute:

```console
just nixos-runner-test
```

The test is also part of `nix flake check`. It builds its own small CPU-probe experiment into the store, boots one NixOS VM, explicitly starts the service, verifies the published bundle with the packaged binary, and checks identity, ownership, status, counts, generator identity, positive timing/throughput relationships, digests, checksums, and empty staging state. It uses semantic assertions rather than performance thresholds. It requires no cloud account, AWS API access, GPU, model download, container runtime, or external runtime service; after its Nix closure is available, the VM uses no external network access.

The flake declares and evaluates this check for `x86_64-linux` and `aarch64-linux`, but declaration or successful evaluation alone does not mean a VM was executed. The primary CI job executes the flake checks on `x86_64-linux`; a separate `NixOS runner VM (aarch64)` job builds `checks.aarch64-linux.nixos-runner-vm` on GitHub's native `ubuntu-24.04-arm` runner. The test uses KVM when it is available but permits same-architecture QEMU system emulation when it is not; this changes execution speed, not the packaged binary, guest architecture, or assertions. This establishes native CI coverage for this NixOS runner VM test on those two architectures, not broader ARM support for Benchplane. `just nixos-runner-test` selects and runs only the current host system's check; validation reports should still name the exact architecture on which each VM test actually ran.

For interactive inspection, launch the test driver's interactive output:

```console
just nixos-runner-interactive
```

At the Python prompt, either run the complete integration test with `test_script()`, or boot and inspect the VM manually:

```python
machine.start()
machine.wait_for_unit("multi-user.target")
print(machine.succeed("systemctl start benchplane-runner.service"))
print(machine.succeed("journalctl -u benchplane-runner.service --no-pager"))
print(machine.succeed("find /var/lib/benchplane -maxdepth 3 -print"))
```

Exit the driver with Control-D and confirm the prompt; it will terminate the VM and clean up its temporary state. Without KVM access, QEMU falls back to software emulation and boot can be substantially slower. This interactive driver is a development and debugging interface and does not replace the hermetic `nixos-runner-test` check.
