{
  description = "Benchplane: reproducible AI systems experiments, from specification to evidence";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        ./nix/flake-modules/dev-shells.nix
        ./nix/flake-modules/formatter.nix
        ./nix/flake-modules/nixos-modules.nix
        ./nix/flake-modules/nixos-tests.nix
        ./nix/flake-modules/packages.nix
      ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
    };
}
