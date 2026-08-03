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

      smolLm2Model = pkgs.fetchurl {
        name = "SmolLM2-135M-Instruct.Q2_K.gguf";
        url = "https://huggingface.co/QuantFactory/SmolLM2-135M-Instruct-GGUF/resolve/c33bd7b3a0c1c5048af630f0198eb2a29977b422/SmolLM2-135M-Instruct.Q2_K.gguf?download=true";
        hash = "sha256-VaqI3axDrc5q8Om+jWzf8jN6ODXNm1C7zXqJTrZt/HU=";
      };

      llamaCppCpu = pkgs.llama-cpp.override {
        cudaSupport = false;
        rocmSupport = false;
        openclSupport = false;
        vulkanSupport = false;
        metalSupport = false;
      };

      llamaCppHelper = pkgs.stdenv.mkDerivation {
        pname = "benchplane-llama-cpp-helper";
        version = "1";
        dontUnpack = true;
        buildPhase = ''
          runHook preBuild
          $CXX -std=c++17 -O2 -Wall -Wextra -Werror \
            -DBENCHPLANE_MODEL_PATH='"${smolLm2Model}"' \
            -I${llamaCppCpu.dev}/include \
            -L${llamaCppCpu}/lib -Wl,-rpath,${llamaCppCpu}/lib \
            ${../packages/benchplane-llama-cpp.cpp} -lllama -lggml -pthread \
            -o benchplane-llama-cpp
          runHook postBuild
        '';
        installPhase = ''
          runHook preInstall
          install -Dm755 benchplane-llama-cpp $out/bin/benchplane-llama-cpp
          runHook postInstall
        '';
      };

      benchplaneRust = pkgs.rustPlatform.buildRustPackage {
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

      benchplane = pkgs.symlinkJoin {
        name = "benchplane-0.1.0";
        paths = [
          benchplaneRust
          llamaCppHelper
        ];
        postBuild = ''
          # `current_exe` must remain inside this combined package so sibling
          # helper discovery cannot resolve back through a symlink to the Rust-only output.
          cp --remove-destination ${benchplaneRust}/bin/benchplane $out/bin/benchplane
          cp --remove-destination ${benchplaneRust}/bin/benchplane-cpu-probe $out/bin/benchplane-cpu-probe
          cp --remove-destination ${llamaCppHelper}/bin/benchplane-llama-cpp $out/bin/benchplane-llama-cpp
          # llama.cpp dynamically discovers only its CPU/BLAS backend libraries
          # beside the fixed helper; do not expose the upstream general-purpose CLIs.
          ln -s ${llamaCppCpu}/bin/libggml-*.so $out/bin/
          mkdir -p $out/share/benchplane/models
          ln -s ${smolLm2Model} $out/share/benchplane/models/SmolLM2-135M-Instruct.Q2_K.gguf
          install -Dm444 ${../packages/THIRD_PARTY_NOTICES.md} \
            $out/share/doc/benchplane/THIRD_PARTY_NOTICES.md
          install -Dm444 ${../../LICENSE} \
            $out/share/licenses/benchplane/Apache-2.0.txt
          install -Dm444 ${../../NOTICE} $out/share/doc/benchplane/NOTICE
        '';
        meta = benchplaneRust.meta // {
          # Benchplane and the model are Apache-2.0; llama.cpp is MIT.
          license = with inputs.nixpkgs.lib.licenses; [
            asl20
            mit
          ];
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

        cpu-probe-lifecycle-smoke = pkgs.runCommand "benchplane-cpu-probe-lifecycle-smoke" { } ''
          outputRoot="$TMPDIR/benchplane-output"
          ${benchplane}/bin/benchplane run ${../../experiments/smoke/local-cpu-probe.yaml} \
            --output-root "$outputRoot" --json > result.json
          bundle="$(${pkgs.jq}/bin/jq -er '.bundlePath' result.json)"
          ${pkgs.jq}/bin/jq -e \
            '.runState == "succeeded" and .validityStatus == "valid"' \
            result.json > /dev/null
          ${pkgs.jq}/bin/jq -e -s \
            'length == 4 and all(.generator == "benchplane-cpu-probe/v1")' \
            "$bundle/attempts/0001/measurements.jsonl" > /dev/null
          ${benchplane}/bin/benchplane evidence verify "$bundle" > verified.txt
          mkdir $out
          cp result.json verified.txt $out/
        '';

        llama-cpp-package-assets = pkgs.runCommand "benchplane-llama-cpp-package-assets" { } ''
          test -x ${benchplane}/bin/benchplane
          test -x ${benchplane}/bin/benchplane-cpu-probe
          test -x ${benchplane}/bin/benchplane-llama-cpp
          test ! -e ${benchplane}/bin/llama-cli
          test -r ${benchplane}/share/benchplane/models/SmolLM2-135M-Instruct.Q2_K.gguf
          test -r ${benchplane}/share/doc/benchplane/THIRD_PARTY_NOTICES.md
          test -r ${benchplane}/share/licenses/benchplane/Apache-2.0.txt
          test -r ${benchplane}/share/doc/benchplane/NOTICE
          test "$(stat -Lc %s ${benchplane}/share/benchplane/models/SmolLM2-135M-Instruct.Q2_K.gguf)" = 88201792
          if ${benchplane}/bin/benchplane-llama-cpp --requests 100 --warmup-runs 1 --repetitions 3 --output-tokens 4; then
            exit 1
          fi
          touch $out
        '';

        llama-cpp-lifecycle-smoke = pkgs.runCommand "benchplane-llama-cpp-lifecycle-smoke" { } ''
          outputRoot="$TMPDIR/benchplane-output"
          ${benchplane}/bin/benchplane run ${../../experiments/smoke/local-llama-cpp.yaml} \
            --output-root "$outputRoot" --json > result.json
          bundle="$(${pkgs.jq}/bin/jq -er '.bundlePath' result.json)"
          ${pkgs.jq}/bin/jq -e \
            '.runState == "succeeded" and .validityStatus == "valid"' \
            result.json > /dev/null
          ${pkgs.jq}/bin/jq -e -s \
            'length == 4 and all(.generator == "benchplane-llama-cpp-smollm2/v1") and all(.latencyMicros > 0 and .timeToFirstTokenMicros > 0 and .timeToFirstTokenMicros <= .latencyMicros and .throughputMilliRequestsPerSecond > 0 and .successfulRequests == 2 and .failedRequests == 0)' \
            "$bundle/attempts/0001/measurements.jsonl" > /dev/null
          ${benchplane}/bin/benchplane evidence verify "$bundle" > verified.txt
          mkdir $out
          cp result.json verified.txt $out/
        '';
      };
    };
}
