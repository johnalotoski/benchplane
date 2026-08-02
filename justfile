set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

fmt:
    treefmt

# On failure, treefmt writes formatting fixes before exiting nonzero.
fmt-check:
    treefmt --fail-on-change

check: fmt-check schema-check local-smoke
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
      jq -e '.runState == "succeeded" and .validityStatus == "valid"' \
        "$smoke_dir/result.json" > /dev/null; \
      cargo run --quiet -p benchplane -- evidence verify "$bundle" > /dev/null

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
