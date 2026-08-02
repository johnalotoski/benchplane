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

      evaluatedModule = inputs.nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          ../modules/nixos/profile-experiment-node.nix
          {
            services.benchplane.runner.command = [
              "${pkgs.coreutils}/bin/printf"
              "contains space"
              "single'quote"
              "double\"quote"
              "literal$variable"
              "unit%n"
              ";"
            ];
            system.stateVersion = "24.11";
          }
        ];
      };

      expectedRunnerCommand = "\"${pkgs.coreutils}/bin/printf\" \"contains space\" \"single'quote\" \"double\\\"quote\" \"literal$$variable\" \"unit%%n\" \";\"";
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
              runnerCommand = evaluatedModule.config.systemd.services.benchplane-runner.serviceConfig.ExecStart;
              inherit expectedRunnerCommand;
              maximumRuntime = toString evaluatedModule.config.services.benchplane.lifecycle.maximumRuntimeSeconds;
            }
            ''
              test "$runnerCommand" = "$expectedRunnerCommand"
              test "$maximumRuntime" = 3600
              touch $out
            '';
      };
    };
}
