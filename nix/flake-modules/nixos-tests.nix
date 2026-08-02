{ ... }:
{
  perSystem =
    {
      config,
      pkgs,
      ...
    }:
    let
      benchplane = config.packages.default;
      experiment = pkgs.writeText "nixos-runner-smoke.yaml" ''
        apiVersion: benchplane/v1alpha1
        kind: Experiment
        metadata:
          name: nixos-runner-smoke
        spec:
          provider:
            kind: localFake
          runtime:
            kind: localFake
            seed: 42
            scenario: success
          workload:
            profile: nixos-runner-smoke
            requests: 8
          measurement:
            warmupRuns: 1
            repetitions: 3
          budget:
            maximumCostUsd: 0
          lifecycle:
            maximumRuntimeSeconds: 60
      '';
    in
    {
      checks.nixos-runner-vm = pkgs.testers.runNixOSTest {
        name = "benchplane-nixos-runner";

        nodes.machine =
          { ... }:
          {
            imports = [ ../modules/nixos/default.nix ];

            services.benchplane = {
              enable = true;
              runner = {
                enable = true;
                package = benchplane;
                experimentFile = experiment;
              };
              lifecycle.maximumRuntimeSeconds = 300;
            };

            networking.useDHCP = false;
            virtualisation = {
              cores = 1;
              memorySize = 1024;
            };

            # Model a new test system initially created on NixOS 26.05.
            system.stateVersion = "26.05";
          };

        testScript = ''
          import json
          import re
          import shlex

          experiment = ${builtins.toJSON (toString experiment)}
          benchplane = ${builtins.toJSON "${benchplane}/bin/benchplane"}

          machine.start()
          machine.wait_for_unit("multi-user.target")

          with subtest("runner is defined but not activated at boot"):
              machine.succeed("systemctl cat benchplane-runner.service >/dev/null")
              machine.fail("systemctl is-active --quiet benchplane-runner.service")
              machine.fail("systemctl list-dependencies --plain multi-user.target | grep -Fx benchplane-runner.service")
              machine.fail("test -e /var/lib/benchplane")
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=StateDirectory --value"
              ).strip() == "benchplane"
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=TimeoutStartUSec --value"
              ).strip() == "5min"
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=Restart --value"
              ).strip() == "no"

          with subtest("runner is unprivileged and conservatively hardened"):
              runner_uid = machine.succeed("id -u benchplane").strip()
              assert runner_uid != "0"
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=User --value"
              ).strip() == "benchplane"
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=ProtectSystem --value"
              ).strip() == "strict"
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=NoNewPrivileges --value"
              ).strip() == "yes"
              experiment_mode = int(machine.succeed(
                  f"stat -c %a {shlex.quote(experiment)}"
              ).strip(), 8)
              assert experiment_mode & 0o222 == 0
              machine.fail(f"runuser -u benchplane -- test -w {shlex.quote(experiment)}")

          with subtest("one explicit activation completes successfully"):
              machine.succeed("systemctl start benchplane-runner.service")
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=Result --value"
              ).strip() == "success"
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=ExecMainStatus --value"
              ).strip() == "0"
              assert machine.succeed(
                  "systemctl show benchplane-runner.service --property=ActiveState --value"
              ).strip() == "inactive"

          with subtest("state and publication ownership are restricted"):
              assert machine.succeed("stat -c %a /var/lib/benchplane").strip() == "750"
              run_ids = machine.succeed(
                  "find /var/lib/benchplane/runs -mindepth 1 -maxdepth 1 -type d -printf '%f\\n'"
              ).splitlines()
              assert len(run_ids) == 1, run_ids
              run_id = run_ids[0]
              assert re.fullmatch(
                  r"run-[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}",
                  run_id,
              ), run_id
              run_directory = f"/var/lib/benchplane/runs/{run_id}"
              assert machine.succeed(f"stat -c %U {shlex.quote(run_directory)}").strip() == "benchplane"
              assert machine.succeed(f"stat -c %a {shlex.quote(run_directory)}").strip() == "750"
              machine.fail("test -e /.benchplane")

          with subtest("published evidence is complete and verifies"):
              machine.succeed(
                  f"{shlex.quote(benchplane)} evidence verify {shlex.quote(run_directory)}"
              )
              manifest = json.loads(machine.succeed(
                  f"cat {shlex.quote(run_directory + '/manifest.json')}"
              ))
              run = json.loads(machine.succeed(
                  f"cat {shlex.quote(run_directory + '/run.json')}"
              ))
              attempt = json.loads(machine.succeed(
                  f"cat {shlex.quote(run_directory + '/attempts/0001/attempt.json')}"
              ))
              validity = json.loads(machine.succeed(
                  f"cat {shlex.quote(run_directory + '/validity.json')}"
              ))
              summary = json.loads(machine.succeed(
                  f"cat {shlex.quote(run_directory + '/summary.json')}"
              ))

              assert manifest["format"] == "benchplane-evidence/v1"
              assert manifest["runId"] == run_id
              assert manifest["runStatus"] == "succeeded"
              assert manifest["validityStatus"] == "valid"
              assert manifest["attemptCount"] == 1
              assert run["runStatus"] == "succeeded"
              assert attempt["status"] == "succeeded"
              assert validity["status"] == "valid"
              assert validity["observedSamples"] == 3
              assert summary["attemptCount"] == 1
              assert summary["sampleCount"] == 3
              digest_pattern = r"sha256:[0-9a-f]{64}"
              assert re.fullmatch(digest_pattern, manifest["experimentDigest"])
              assert re.fullmatch(digest_pattern, manifest["resolvedPlanDigest"])
              machine.succeed(f"test -s {shlex.quote(run_directory + '/SHA256SUMS')}")

              measurements = machine.succeed(
                  f"cat {shlex.quote(run_directory + '/attempts/0001/measurements.jsonl')}"
              ).splitlines()
              assert len(measurements) == 4
              phases = [json.loads(line)["phase"] for line in measurements]
              assert phases.count("warmup") == 1
              assert phases.count("measured") == 3

              machine.fail(
                  "find /var/lib/benchplane/staging -mindepth 1 -maxdepth 1 -type d | grep ."
              )
        '';
      };
    };
}
