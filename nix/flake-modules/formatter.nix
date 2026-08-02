{ inputs, ... }:
{
  imports = [ inputs.treefmt-nix.flakeModule ];

  perSystem = { ... }: {
    treefmt = {
      projectRootFile = "flake.nix";
      programs = {
        nixfmt.enable = true;
        rustfmt = {
          enable = true;
          edition = "2021";
        };
        taplo.enable = true;
        terraform.enable = true;
        shellcheck.enable = true;
      };
    };
  };
}
