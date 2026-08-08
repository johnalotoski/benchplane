# Nix packages

The flake's default package builds the Benchplane Rust CLI using the checked-in `Cargo.lock` and combines it with both fixed helpers, CPU-only llama.cpp `b10133`, and the immutable SmolLM2-135M-Instruct Q2_K fixture. The model source is QuantFactory's Apache-2.0 GGUF at commit `c33bd7b3a0c1c5048af630f0198eb2a29977b422`; its 88,201,792-byte content is pinned by SHA-256 `55aa88ddac43adce6af0e9be8d6cdff2337a3835cd9b50bbcd7a894eb66dfc75`. The package exposes only Benchplane's CLI and helpers, not llama.cpp's general-purpose CLIs. The model and MIT-licensed engine increase the package closure but avoid every runtime download.

Keep future vLLM packaging separate from this Benchplane package so native-Nix and container runtime variants can share the surrounding node configuration.
