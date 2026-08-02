{ inputs, ... }:
{
  perSystem =
    { pkgs, system, ... }:
    let
      source = inputs.nixpkgs.lib.fileset.toSource {
        root = ../..;
        fileset = inputs.nixpkgs.lib.fileset.unions [
          ../../Cargo.toml
          ../../Cargo.lock
          ../../crates
          ../../experiments
          ../../schemas
          ../../tests
        ];
      };

      benchplane = pkgs.rustPlatform.buildRustPackage {
        pname = "benchplane";
        version = "0.1.0";
        src = source;

        cargoLock.lockFile = ../../Cargo.lock;

        meta = {
          description = "Reproducible AI systems experiments, from specification to evidence";
          homepage = "https://github.com/johnalotoski/benchplane";
          license = inputs.nixpkgs.lib.licenses.asl20;
          mainProgram = "benchplane";
        };
      };

      evaluationExperiment = pkgs.writeText "benchplane-module-evaluation-experiment.yaml" ''
        apiVersion: benchplane/v1alpha1
        kind: Experiment
        metadata:
          name: module-evaluation
        spec:
          provider:
            kind: localFake
          runtime:
            kind: localFake
          workload:
            profile: evaluation
            requests: 1
          budget:
            maximumCostUsd: 0
      '';

      evaluatedModule = inputs.nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          ../modules/nixos/profile-experiment-node.nix
          {
            services.benchplane.runner = {
              package = benchplane;
              experimentFile = evaluationExperiment;
            };
            # Model a new test system initially created on NixOS 26.05.
            system.stateVersion = "26.05";
          }
        ];
      };

      invalidRunnerEvaluation =
        runnerConfiguration: lifecycleConfiguration:
        builtins.tryEval (
          (inputs.nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              ../modules/nixos/default.nix
              {
                services.benchplane = {
                  enable = true;
                  runner = {
                    enable = true;
                  }
                  // runnerConfiguration;
                  lifecycle = lifecycleConfiguration;
                };
                system.stateVersion = "26.05";
              }
            ];
          }).config.system.build.toplevel.drvPath
        );

      missingPackageEvaluation = invalidRunnerEvaluation {
        experimentFile = evaluationExperiment;
      } { };
      missingExperimentEvaluation = invalidRunnerEvaluation {
        package = benchplane;
      } { };
      zeroRuntimeEvaluation = invalidRunnerEvaluation {
        package = benchplane;
        experimentFile = evaluationExperiment;
      } { maximumRuntimeSeconds = 0; };
      unsafeStateDirectoryEvaluation = builtins.tryEval (
        (inputs.nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [
            ../modules/nixos/default.nix
            {
              services.benchplane = {
                enable = true;
                stateDirectory = "../benchplane";
                runner = {
                  enable = true;
                  package = benchplane;
                  experimentFile = evaluationExperiment;
                };
              };
              system.stateVersion = "26.05";
            }
          ];
        }).config.system.build.toplevel.drvPath
      );

      runnerService = evaluatedModule.config.systemd.services.benchplane-runner;
    in
    {
      packages.default = benchplane;

      checks = {
        inherit benchplane;

        schema = pkgs.runCommand "benchplane-schema-check" { } ''
          ${benchplane}/bin/benchplane schema export > generated.json
          ${pkgs.diffutils}/bin/diff -u ${../../schemas/v1alpha1/experiment.schema.json} generated.json
          touch $out
        '';

        nixos-module-evaluation =
          pkgs.runCommand "benchplane-nixos-module-evaluation"
            {
              runnerCommand = runnerService.serviceConfig.ExecStart;
              runnerPackage = toString benchplane;
              stateDirectory = runnerService.serviceConfig.StateDirectory;
              timeoutStart = runnerService.serviceConfig.TimeoutStartSec;
              wantedBy = builtins.concatStringsSep " " runnerService.wantedBy;
              maximumRuntime = toString evaluatedModule.config.services.benchplane.lifecycle.maximumRuntimeSeconds;
              missingPackageAccepted = toString missingPackageEvaluation.success;
              missingExperimentAccepted = toString missingExperimentEvaluation.success;
              zeroRuntimeAccepted = toString zeroRuntimeEvaluation.success;
              unsafeStateDirectoryAccepted = toString unsafeStateDirectoryEvaluation.success;
            }
            ''
              case "$runnerCommand" in
                \"$runnerPackage/bin/benchplane\"\ \"run\"\ *\ \"--output-root\"\ \"/var/lib/benchplane\") ;;
                *) exit 1 ;;
              esac
              test "$stateDirectory" = benchplane
              test "$timeoutStart" = 3600s
              test -z "$wantedBy"
              test "$maximumRuntime" = 3600
              test -z "$missingPackageAccepted"
              test -z "$missingExperimentAccepted"
              test -z "$zeroRuntimeAccepted"
              test -z "$unsafeStateDirectoryAccepted"
              touch $out
            '';

        local-lifecycle-smoke = pkgs.runCommand "benchplane-local-lifecycle-smoke" { } ''
          outputRoot="$TMPDIR/benchplane-output"
          ${benchplane}/bin/benchplane run ${../../experiments/smoke/local-fake.yaml} \
            --output-root "$outputRoot" --json > result.json
          bundle="$(${pkgs.jq}/bin/jq -er '.bundlePath' result.json)"
          ${pkgs.jq}/bin/jq -e \
            '.runState == "succeeded" and .validityStatus == "valid"' \
            result.json > /dev/null
          test -d "$bundle"
          ${benchplane}/bin/benchplane evidence verify "$bundle" > verified.txt
          mkdir $out
          cp result.json verified.txt $out/
        '';
      };
    };
}
