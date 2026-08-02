# Nix packages

The flake's default package builds the Benchplane Rust CLI using the checked-in `Cargo.lock`. Keep vLLM packaging separate from the Benchplane CLI package so native-Nix and container runtime variants can share the surrounding node configuration.
