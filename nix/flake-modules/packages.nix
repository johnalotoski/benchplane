{ inputs, ... }:
{
  perSystem =
    { pkgs, system, ... }:
    let
      cudaSupported = system == "x86_64-linux";
      cudaPkgs = import inputs.nixpkgs {
        inherit system;
        config.allowUnfree = true;
      };
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
        rpcSupport = false;
      };

      llamaCppCuda =
        if cudaSupported then
          cudaPkgs.llama-cpp.override {
            cudaSupport = true;
            cudaPackages = cudaPkgs.cudaPackages;
            rocmSupport = false;
            openclSupport = false;
            vulkanSupport = false;
            metalSupport = false;
            rpcSupport = false;
          }
        else
          null;

      ambientBackendSentinel =
        pkgs.runCommand "benchplane-ambient-ggml-backend-sentinel"
          { nativeBuildInputs = [ pkgs.stdenv.cc ]; }
          ''
            mkdir -p $out/bin $out/lib
            $CC -shared -fPIC ${../packages/ambient-ggml-backend-sentinel.c} \
              -o $out/lib/libggml-cuda.so
            $CXX -std=c++17 -Wall -Wextra -Werror \
              -I${llamaCppCpu.dev}/include \
              -L${llamaCppCpu}/lib -Wl,-rpath,${llamaCppCpu}/lib \
              ${../packages/ambient-ggml-default-loader.cpp} -lggml \
              -o $out/bin/ambient-ggml-default-loader
          '';

      llamaCppHelper = pkgs.stdenv.mkDerivation {
        pname = "benchplane-llama-cpp-helper";
        version = "1";
        dontUnpack = true;
        buildPhase = ''
          runHook preBuild
          $CXX -std=c++17 -O2 -Wall -Wextra -Werror \
            -DBENCHPLANE_MODEL_PATH='"${smolLm2Model}"' \
            -DBENCHPLANE_BACKEND_PATH='"${llamaCppCpu}/bin"' \
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

      llamaCppCudaHelper =
        if cudaSupported then
          cudaPkgs.cudaPackages.backendStdenv.mkDerivation {
            pname = "benchplane-llama-cpp-nvidia-cuda-helper";
            version = "1";
            dontUnpack = true;
            buildInputs = [
              cudaPkgs.cudaPackages.cuda_cudart
              cudaPkgs.cudaPackages.cuda_nvcc
            ];
            buildPhase = ''
              runHook preBuild
              $CXX -std=c++17 -O2 -Wall -Wextra -Werror \
                -DBENCHPLANE_TARGET_NVIDIA_CUDA=1 \
                -DBENCHPLANE_MODEL_PATH='"${smolLm2Model}"' \
                -DBENCHPLANE_BACKEND_PATH='"${llamaCppCuda}/bin"' \
                -I${llamaCppCuda.dev}/include \
                -I${cudaPkgs.cudaPackages.cuda_cudart}/include \
                -I${cudaPkgs.cudaPackages.cuda_nvcc}/include \
                -L${llamaCppCuda}/lib -Wl,-rpath,${llamaCppCuda}/lib \
                -L${cudaPkgs.cudaPackages.cuda_cudart}/lib \
                ${../packages/benchplane-llama-cpp.cpp} \
                -lllama -lggml -lggml-base -lcudart -pthread \
                -o benchplane-llama-cpp-nvidia-cuda
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              install -Dm755 benchplane-llama-cpp-nvidia-cuda \
                $out/bin/benchplane-llama-cpp-nvidia-cuda
              runHook postInstall
            '';
          }
        else
          null;

      mkBenchplaneRust =
        cudaEnabled:
        pkgs.rustPlatform.buildRustPackage {
          pname = if cudaEnabled then "benchplane-nvidia-cuda" else "benchplane";
          version = "0.1.0";
          src = source;

          cargoLock.lockFile = ../../Cargo.lock;

          env = {
            BENCHPLANE_LLAMA_CPP_NIX_STORE_PATH = toString llamaCppCpu;
            BENCHPLANE_SMOLLM2_NIX_STORE_PATH = toString smolLm2Model;
            BENCHPLANE_LLAMA_CPP_CUDA_AVAILABLE = if cudaEnabled then "1" else "0";
          }
          // inputs.nixpkgs.lib.optionalAttrs cudaEnabled {
            BENCHPLANE_LLAMA_CPP_CUDA_NIX_STORE_PATH = toString llamaCppCuda;
          };

          meta = {
            description = "Reproducible AI systems experiments, from specification to evidence";
            homepage = "https://github.com/johnalotoski/benchplane";
            license = inputs.nixpkgs.lib.licenses.asl20;
            mainProgram = "benchplane";
          };
        };

      mkBenchplane =
        rustPackage: cudaEnabled:
        pkgs.symlinkJoin {
          name = if cudaEnabled then "benchplane-nvidia-cuda-0.1.0" else "benchplane-0.1.0";
          paths = [
            rustPackage
            llamaCppHelper
          ]
          ++ inputs.nixpkgs.lib.optionals cudaEnabled [ llamaCppCudaHelper ];
          postBuild = ''
            # `current_exe` must remain inside this combined package so sibling
            # helper discovery cannot resolve back through a symlink to the Rust-only output.
            cp --remove-destination ${rustPackage}/bin/benchplane $out/bin/benchplane
            cp --remove-destination ${rustPackage}/bin/benchplane-cpu-probe $out/bin/benchplane-cpu-probe
            cp --remove-destination ${llamaCppHelper}/bin/benchplane-llama-cpp $out/bin/benchplane-llama-cpp
            ${inputs.nixpkgs.lib.optionalString cudaEnabled ''
              cp --remove-destination \
                ${llamaCppCudaHelper}/bin/benchplane-llama-cpp-nvidia-cuda \
                $out/bin/benchplane-llama-cpp-nvidia-cuda
            ''}
            mkdir -p $out/share/benchplane/models
            ln -s ${smolLm2Model} $out/share/benchplane/models/SmolLM2-135M-Instruct.Q2_K.gguf
            install -Dm444 ${../packages/THIRD_PARTY_NOTICES.md} \
              $out/share/doc/benchplane/THIRD_PARTY_NOTICES.md
            install -Dm444 ${../../LICENSE} \
              $out/share/licenses/benchplane/Apache-2.0.txt
            install -Dm444 ${../../NOTICE} $out/share/doc/benchplane/NOTICE
          '';
          meta = rustPackage.meta // {
            # Benchplane and the model are Apache-2.0; llama.cpp is MIT.
            license = with inputs.nixpkgs.lib.licenses; [
              asl20
              mit
            ];
          };
        };

      benchplaneRust = mkBenchplaneRust false;
      benchplane = mkBenchplane benchplaneRust false;
      benchplaneNvidiaCuda = if cudaSupported then mkBenchplane (mkBenchplaneRust true) true else null;

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

      evaluatedNvidiaModule =
        if cudaSupported then
          inputs.nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              ../modules/nixos/default.nix
              ../modules/nixos/nvidia.nix
              {
                nixpkgs.config.allowUnfree = true;
                services.benchplane = {
                  enable = true;
                  runner = {
                    enable = true;
                    package = benchplaneNvidiaCuda;
                    experimentFile = evaluationExperiment;
                  };
                  nvidia.enable = true;
                };
                system.stateVersion = "26.05";
              }
            ];
          }
        else
          null;

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
      packages = {
        default = benchplane;
        inherit benchplane;
      }
      // inputs.nixpkgs.lib.optionalAttrs cudaSupported {
        benchplane-nvidia-cuda = benchplaneNvidiaCuda;
      };

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
            '.runState == "succeeded" and .validityStatus == "valid" and
             .resources == null' \
            result.json > /dev/null
          ${pkgs.jq}/bin/jq -e \
            --arg runId "$(${pkgs.jq}/bin/jq -er '.runId' result.json)" \
            --arg benchplaneStore '${benchplane}' \
            '.format == "benchplane-attempt-provenance/v1" and
             .runId == $runId and .attemptNumber == 1 and
             (.platform.operatingSystem.family | length > 0) and
             (.platform.kernel.name | length > 0) and
             (.platform.architecture | length > 0) and
             (.platform.cpu.logicalCpuCount == null or .platform.cpu.logicalCpuCount > 0) and
             .software.benchplane.name == "benchplane" and
             .software.benchplane.version == "0.1.0" and
             .software.benchplane.nixStorePath == $benchplaneStore and
             .software.runtime.kind == "localFake" and
             .software.runtime.generator == "benchplane-local-fake/v1"' \
            "$bundle/attempts/0001/provenance.json" > /dev/null
          grep -F '  attempts/0001/provenance.json' "$bundle/SHA256SUMS" > /dev/null
          test ! -e "$bundle/attempts/0001/resources.json"
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
            '.runState == "succeeded" and .validityStatus == "valid" and
             (.resources.cpuTimeMicros | type == "number") and
             (.resources.peakRssBytes | type == "number") and
             (.resources.peakRssBytes % 1024 == 0)' \
            result.json > /dev/null
          ${pkgs.jq}/bin/jq -e -s \
            'length == 4 and all(.generator == "benchplane-cpu-probe/v1")' \
            "$bundle/attempts/0001/measurements.jsonl" > /dev/null
          ${pkgs.jq}/bin/jq -e \
            --arg runId "$(${pkgs.jq}/bin/jq -er '.runId' result.json)" \
            --arg benchplaneStore '${benchplane}' \
            '.format == "benchplane-attempt-provenance/v1" and
             .runId == $runId and .attemptNumber == 1 and
             (.platform.operatingSystem.family | length > 0) and
             (.platform.kernel.name | length > 0) and
             (.platform.architecture | length > 0) and
             (.platform.cpu.logicalCpuCount == null or .platform.cpu.logicalCpuCount > 0) and
             .software.benchplane.name == "benchplane" and
             .software.benchplane.version == "0.1.0" and
             .software.benchplane.nixStorePath == $benchplaneStore and
             .software.runtime.kind == "cpuProbe" and
             .software.runtime.generator == "benchplane-cpu-probe/v1"' \
            "$bundle/attempts/0001/provenance.json" > /dev/null
          grep -F '  attempts/0001/provenance.json' "$bundle/SHA256SUMS" > /dev/null
          ${pkgs.jq}/bin/jq -e \
            --arg runId "$(${pkgs.jq}/bin/jq -er '.runId' result.json)" \
            '.format == "benchplane-attempt-resources/v1" and
             .runId == $runId and .attemptNumber == 1 and
             .scope == "helperProcessLifetime" and
             (.cpuTimeMicros | type == "number") and
             (.peakRssBytes | type == "number") and
             (.peakRssBytes % 1024 == 0)' \
            "$bundle/attempts/0001/resources.json" > /dev/null
          grep -F '  attempts/0001/resources.json' "$bundle/SHA256SUMS" > /dev/null
          ${benchplane}/bin/benchplane evidence verify "$bundle" > verified.txt
          mkdir $out
          cp result.json verified.txt $out/
        '';

        llama-cpp-package-assets = pkgs.runCommand "benchplane-llama-cpp-package-assets" { } ''
          test -x ${benchplane}/bin/benchplane
          test -x ${benchplane}/bin/benchplane-cpu-probe
          test -x ${benchplane}/bin/benchplane-llama-cpp
          test ! -e ${benchplane}/bin/llama-cli
          for packagedBackend in ${benchplane}/bin/libggml-*.so; do
            test ! -e "$packagedBackend"
          done
          backendCount=0
          sawBlas=0
          sawCpu=0
          for backend in ${llamaCppCpu}/bin/libggml-*.so; do
            backendCount=$((backendCount + 1))
            case "$(basename "$backend")" in
              libggml-blas.so) sawBlas=1 ;;
              libggml-cpu*.so) sawCpu=1 ;;
              *)
                echo "unexpected non-CPU ggml backend: $backend" >&2
                exit 1
                ;;
            esac
          done
          test "$backendCount" -gt 0
          test "$sawBlas" = 1
          test "$sawCpu" = 1
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
          resultFile="$TMPDIR/result.json"
          ambientCwd="$TMPDIR/ambient-backend"
          mkdir "$ambientCwd"
          ln -s ${ambientBackendSentinel}/lib/libggml-cuda.so \
            "$ambientCwd/libggml-cuda.so"
          # Calibrate the sentinel against b10133's default CWD search before
          # proving the packaged explicit-path invocation ignores it.
          set +e
          (
            cd "$ambientCwd"
            ${ambientBackendSentinel}/bin/ambient-ggml-default-loader
          )
          sentinelStatus=$?
          set -e
          test "$sentinelStatus" = 86
          (
            cd "$ambientCwd"
            GGML_BACKEND_PATH="$ambientCwd/libggml-cuda.so" \
              ${benchplane}/bin/benchplane-llama-cpp \
                --requests 1 --warmup-runs 0 --repetitions 1 --output-tokens 1 \
                > "$TMPDIR/direct-helper.jsonl"
          )
          ${pkgs.jq}/bin/jq -e -s \
            'length == 1 and all(.generator == "benchplane-llama-cpp-smollm2/v2") and all(.phase == "measured" and .repetitionIndex == 1 and .latencyMicros > 0 and .timeToFirstTokenMicros > 0 and .timeToFirstTokenMicros <= .latencyMicros and .throughputMilliRequestsPerSecond > 0 and .successfulRequests == 1 and .failedRequests == 0 and (.requestObservations | length) == 1 and .requestObservations[0].requestIndex == 1 and .requestObservations[0].latencyMicros > 0 and .requestObservations[0].timeToFirstTokenMicros > 0 and .requestObservations[0].timeToFirstTokenMicros <= .requestObservations[0].latencyMicros)' \
            "$TMPDIR/direct-helper.jsonl" > /dev/null
          (
            cd "$ambientCwd"
            GGML_BACKEND_PATH="$ambientCwd/libggml-cuda.so" \
              ${benchplane}/bin/benchplane run ${../../experiments/smoke/local-llama-cpp.yaml} \
                --output-root "$outputRoot" --json > "$resultFile"
          )
          bundle="$(${pkgs.jq}/bin/jq -er '.bundlePath' "$resultFile")"
          ${pkgs.jq}/bin/jq -e \
            '.runState == "succeeded" and .validityStatus == "valid" and
             (.resources.cpuTimeMicros | type == "number") and
             (.resources.peakRssBytes | type == "number") and
             (.resources.peakRssBytes % 1024 == 0)' \
            "$resultFile" > /dev/null
          ${pkgs.jq}/bin/jq -e -s \
            'length == 4 and all(.generator == "benchplane-llama-cpp-smollm2/v2") and all(.latencyMicros > 0 and .timeToFirstTokenMicros > 0 and .timeToFirstTokenMicros <= .latencyMicros and .throughputMilliRequestsPerSecond > 0 and .successfulRequests == 2 and .failedRequests == 0 and (.requestObservations | length) == 2 and .requestObservations[0].requestIndex == 1 and .requestObservations[1].requestIndex == 2 and all(.requestObservations[]; .latencyMicros > 0 and .timeToFirstTokenMicros > 0 and .timeToFirstTokenMicros <= .latencyMicros)) and ([.[] | .requestObservations[]] | length) == 8 and ([.[] | select(.phase == "warmup") | .requestObservations[]] | length) == 2 and ([.[] | select(.phase == "measured") | .requestObservations[]] | length) == 6' \
            "$bundle/attempts/0001/measurements.jsonl" > /dev/null
          ${pkgs.jq}/bin/jq -e \
            --arg runId "$(${pkgs.jq}/bin/jq -er '.runId' "$resultFile")" \
            --arg benchplaneStore '${benchplane}' \
            --arg llamaStore '${llamaCppCpu}' \
            --arg modelStore '${smolLm2Model}' \
            '.format == "benchplane-attempt-provenance/v1" and
             .runId == $runId and .attemptNumber == 1 and
             (.platform.operatingSystem.family | length > 0) and
             (.platform.kernel.name | length > 0) and
             (.platform.architecture | length > 0) and
             (.platform.cpu.logicalCpuCount == null or .platform.cpu.logicalCpuCount > 0) and
             .software.benchplane.name == "benchplane" and
             .software.benchplane.version == "0.1.0" and
             .software.benchplane.nixStorePath == $benchplaneStore and
             .software.runtime.kind == "llamaCpp" and
             .software.runtime.generator == "benchplane-llama-cpp-smollm2/v2" and
             .software.runtime.engine.name == "llama.cpp" and
             .software.runtime.engine.version == "b10133" and
             .software.runtime.engine.nixStorePath == $llamaStore and
             .software.runtime.model.identity == "smollm2-135m-instruct-q2-k-v1" and
             .software.runtime.model.sha256 == "sha256:55aa88ddac43adce6af0e9be8d6cdff2337a3835cd9b50bbcd7a894eb66dfc75" and
             .software.runtime.model.nixStorePath == $modelStore and
             .software.runtime.backend.identity == "nixpkgs-llama-cpp-cpu-only-dynamic/v1" and
             .software.runtime.backend.deviceClass == "cpu" and
             .software.runtime.backend.nixStorePath == $llamaStore' \
            "$bundle/attempts/0001/provenance.json" > /dev/null
          grep -F '  attempts/0001/provenance.json' "$bundle/SHA256SUMS" > /dev/null
          ${pkgs.jq}/bin/jq -e \
            --arg runId "$(${pkgs.jq}/bin/jq -er '.runId' "$resultFile")" \
            '.format == "benchplane-attempt-resources/v1" and
             .runId == $runId and .attemptNumber == 1 and
             .scope == "helperProcessLifetime" and
             (.cpuTimeMicros | type == "number") and
             (.peakRssBytes | type == "number") and
             (.peakRssBytes % 1024 == 0)' \
            "$bundle/attempts/0001/resources.json" > /dev/null
          grep -F '  attempts/0001/resources.json' "$bundle/SHA256SUMS" > /dev/null
          ${benchplane}/bin/benchplane evidence verify "$bundle" > verified.txt
          fixtureBundle="$TMPDIR/run-019fe9ab-efa8-7d31-b631-25ab14491fb8"
          mkdir "$fixtureBundle"
          cp -R \
            ${../../tests/fixtures/evidence-compare/run-019fe9ab-efa8-7d31-b631-25ab14491fb8}/. \
            "$fixtureBundle/"
          ${benchplane}/bin/benchplane evidence compare \
            "$bundle" \
            "$fixtureBundle" \
            --json > comparison.json
          ${pkgs.jq}/bin/jq -e \
            '.format == "benchplane-evidence-comparison/v1" and
             .compatible == true and
             .baseline.runId != .candidate.runId and
             .requests.baselineCount == 6 and .requests.candidateCount == 6 and
             .repetitions.baselineCount == 3 and .repetitions.candidateCount == 3 and
             (.requests.latencyMicros.mean.delta.absoluteDelta | type == "number") and
             (.repetitions.meanThroughputMilliRequestsPerSecond.delta.absoluteDelta |
               type == "number") and
             .attemptResources.unit == "helperProcessLifetime" and
             (.attemptResources.cpuTimeMicros.delta.absoluteDelta | type == "number") and
             (.attemptResources.peakRssBytes.delta.absoluteDelta | type == "number")' \
            comparison.json > /dev/null
          mkdir $out
          cp "$resultFile" $out/result.json
          cp verified.txt $out/
          cp comparison.json $out/
        '';
      }
      // inputs.nixpkgs.lib.optionalAttrs cudaSupported {
        llama-cpp-nvidia-cuda-package-assets =
          pkgs.runCommand "benchplane-llama-cpp-nvidia-cuda-package-assets" { }
            ''
              test -x ${benchplaneNvidiaCuda}/bin/benchplane-llama-cpp-nvidia-cuda
              ${pkgs.binutils}/bin/strings \
                ${benchplaneNvidiaCuda}/bin/benchplane-llama-cpp-nvidia-cuda \
                | grep -F 'host NVIDIA driver must be 610.43.03 or newer' > /dev/null
              backendCount=0
              sawCuda=0
              sawCpu=0
              for backend in ${llamaCppCuda}/bin/libggml-*.so; do
                backendCount=$((backendCount + 1))
                case "$(basename "$backend")" in
                  libggml-cuda.so) sawCuda=1 ;;
                  libggml-blas.so|libggml-cpu*.so) sawCpu=1 ;;
                  *)
                    echo "unexpected ggml backend in CUDA package: $backend" >&2
                    exit 1
                    ;;
                esac
              done
              test "$backendCount" -gt 0
              test "$sawCuda" = 1
              test "$sawCpu" = 1
              cudaRpath="$(${pkgs.patchelf}/bin/patchelf --print-rpath \
                ${llamaCppCuda}/bin/libggml-cuda.so)"
              case ":$cudaRpath:" in
                *":/run/opengl-driver/lib:"*) ;;
                *)
                  echo "CUDA backend lacks the fixed NixOS host-driver lookup path" >&2
                  exit 1
                  ;;
              esac

              ambientCwd="$TMPDIR/ambient-backend"
              mkdir "$ambientCwd"
              ln -s ${ambientBackendSentinel}/lib/libggml-cuda.so \
                "$ambientCwd/libggml-cuda.so"
              set +e
              (
                cd "$ambientCwd"
                GGML_BACKEND_PATH="$ambientCwd/libggml-cuda.so" \
                  GGML_CUDA_DEVICES=99 \
                  CUDA_VISIBLE_DEVICES=99 \
                  CUDA_DEVICE_ORDER=PCI_BUS_ID \
                  ${benchplaneNvidiaCuda}/bin/benchplane-llama-cpp-nvidia-cuda \
                    --requests 1 --warmup-runs 0 --repetitions 1 --output-tokens 1 \
                    > "$TMPDIR/helper-out" 2> "$TMPDIR/helper-err"
              )
              helperStatus=$?
              set -e
              # Nix builds expose no NVIDIA device. The helper must fail at its
              # bounded model/backend initialization boundary, not load the
              # hostile sentinel (86) or silently execute on CPU (0).
              test "$helperStatus" = 20
              test ! -s "$TMPDIR/helper-out"

              set +e
              ${benchplaneNvidiaCuda}/bin/benchplane run \
                ${../../experiments/examples/local-llama-cpp-nvidia-cuda.yaml} \
                --output-root "$TMPDIR/gpu-output" --json > "$TMPDIR/gpu-result.json"
              runStatus=$?
              set -e
              test "$runStatus" = 4
              bundle="$(${pkgs.jq}/bin/jq -er '.bundlePath' "$TMPDIR/gpu-result.json")"
              ${pkgs.jq}/bin/jq -e \
                '.runState == "failed" and .validityStatus == "indeterminate" and
                 .failure.code == "llamaCpp.modelInitFailed"' \
                "$TMPDIR/gpu-result.json" > /dev/null
              ${benchplaneNvidiaCuda}/bin/benchplane evidence verify "$bundle" > /dev/null
              touch $out
            '';

        nixos-nvidia-module-evaluation =
          let
            nvidiaRunner = evaluatedNvidiaModule.config.systemd.services.benchplane-runner;
            nvidiaUser =
              evaluatedNvidiaModule.config.users.users.${evaluatedNvidiaModule.config.services.benchplane.user};
          in
          pkgs.runCommand "benchplane-nixos-nvidia-module-evaluation"
            {
              devicePolicy = nvidiaRunner.serviceConfig.DevicePolicy;
              deviceAllow = builtins.concatStringsSep "|" nvidiaRunner.serviceConfig.DeviceAllow;
              extraGroups = builtins.concatStringsSep "|" nvidiaUser.extraGroups;
            }
            ''
              test "$devicePolicy" = closed
              case "|$deviceAllow|" in *"|/dev/nvidia0 rw|"*) ;; *) exit 1 ;; esac
              case "|$deviceAllow|" in *"|/dev/nvidiactl rw|"*) ;; *) exit 1 ;; esac
              case "|$deviceAllow|" in *"|/dev/nvidia-uvm rw|"*) ;; *) exit 1 ;; esac
              case "|$extraGroups|" in *"|video|"*) ;; *) exit 1 ;; esac
              case "|$extraGroups|" in *"|render|"*) ;; *) exit 1 ;; esac
              touch $out
            '';
      };
    };
}
