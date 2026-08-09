set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

fmt:
    treefmt

# On failure, treefmt writes formatting fixes before exiting nonzero.
fmt-check:
    treefmt --fail-on-change

check: fmt-check schema-check local-smoke cpu-probe-smoke llama-cpp-smoke
    actionlint
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
    cargo run --quiet -p benchplane -- validate experiments/smoke/local-fake.yaml
    cargo run --quiet -p benchplane -- validate experiments/examples/vllm-single-gpu.yaml

local-smoke:
    smoke_dir="$(mktemp -d)"; trap 'rm -rf "$smoke_dir"' EXIT; \
      cargo run --quiet -p benchplane -- run experiments/smoke/local-fake.yaml \
        --output-root "$smoke_dir/output" --json > "$smoke_dir/result.json"; \
      bundle="$(jq -er '.bundlePath' "$smoke_dir/result.json")"; \
      jq -e '.runState == "succeeded" and .validityStatus == "valid" \
        and .resources == null' \
        "$smoke_dir/result.json" > /dev/null; \
      cargo run --quiet -p benchplane -- evidence verify "$bundle" > /dev/null

cpu-probe-smoke:
    smoke_dir="$(mktemp -d)"; trap 'rm -rf "$smoke_dir"' EXIT; \
      cargo build --quiet -p benchplane --bins; \
      cargo run --quiet -p benchplane -- run experiments/smoke/local-cpu-probe.yaml \
        --output-root "$smoke_dir/output" --json > "$smoke_dir/result.json"; \
      bundle="$(jq -er '.bundlePath' "$smoke_dir/result.json")"; \
      jq -e '.runState == "succeeded" and .validityStatus == "valid" \
        and (.resources.cpuTimeMicros | type == "number") \
        and (.resources.peakRssBytes % 1024 == 0)' \
        "$smoke_dir/result.json" > /dev/null; \
      jq -e --arg runId "$(jq -er '.runId' "$smoke_dir/result.json")" \
        '.format == "benchplane-attempt-resources/v1" \
          and .runId == $runId and .attemptNumber == 1 \
          and .scope == "helperProcessLifetime" \
          and (.cpuTimeMicros | type == "number") \
          and (.peakRssBytes % 1024 == 0)' \
        "$bundle/attempts/0001/resources.json" > /dev/null; \
      jq -e -s 'length == 4 and all(.generator == "benchplane-cpu-probe/v1") \
        and all(.latencyMicros > 0 and .timeToFirstTokenMicros > 0 \
          and .timeToFirstTokenMicros <= .latencyMicros \
          and .throughputMilliRequestsPerSecond > 0)' \
        "$bundle/attempts/0001/measurements.jsonl" > /dev/null; \
      grep -F '  attempts/0001/resources.json' "$bundle/SHA256SUMS" > /dev/null; \
      cargo run --quiet -p benchplane -- evidence verify "$bundle" > /dev/null

llama-cpp-smoke:
    system="$(nix eval --impure --raw --expr builtins.currentSystem)"; \
      nix build ".#checks.$system.llama-cpp-lifecycle-smoke" --print-build-logs

nixos-runner-test:
    system="$(nix eval --impure --raw --expr builtins.currentSystem)"; \
      nix build ".#checks.$system.nixos-runner-vm" --print-build-logs

nixos-runner-interactive:
    system="$(nix eval --impure --raw --expr builtins.currentSystem)"; \
      nix run ".#checks.$system.nixos-runner-vm.driverInteractive" --print-build-logs

tofu-validate:
    tofu -chdir=infra/tofu/experiment init -backend=false -lockfile=readonly -input=false
    tofu -chdir=infra/tofu/experiment validate

schema:
    mkdir -p schemas/v1alpha1
    cargo run --quiet -p benchplane -- schema export > schemas/v1alpha1/experiment.schema.json

schema-check:
    schema_file="$(mktemp)"; trap 'rm -f "$schema_file"' EXIT; \
      cargo run --quiet -p benchplane -- schema export > "$schema_file"; \
      diff -u schemas/v1alpha1/experiment.schema.json "$schema_file"

bootstrap:
    cargo generate-lockfile
    just schema
