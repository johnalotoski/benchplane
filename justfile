set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

fmt:
    treefmt

# On failure, treefmt writes formatting fixes before exiting nonzero.
fmt-check:
    treefmt --fail-on-change

check: fmt-check schema-check
    actionlint
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
    cargo run --quiet -p benchplane -- validate experiments/smoke/local-fake.yaml
    cargo run --quiet -p benchplane -- validate experiments/examples/vllm-single-gpu.yaml

tofu-validate:
    tofu -chdir=infra/tofu/experiment init -backend=false -lockfile=readonly -input=false
    tofu -chdir=infra/tofu/experiment validate

schema:
    mkdir -p schemas/v1alpha1
    cargo run --quiet -p benchplane -- schema export > schemas/v1alpha1/experiment.schema.json

schema-check:
    tmp="$$(mktemp)"; trap 'rm -f "$$tmp"' EXIT; \
      cargo run --quiet -p benchplane -- schema export > "$$tmp"; \
      diff -u schemas/v1alpha1/experiment.schema.json "$$tmp"

bootstrap:
    cargo generate-lockfile
    just schema
