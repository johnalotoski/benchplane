{ ... }:
{
  perSystem = { config, pkgs, ... }: {
    devShells.default = pkgs.mkShell {
      packages =
        (with pkgs; [
          actionlint
          cargo
          clippy
          git
          jq
          just
          openssl
          opentofu
          pkg-config
          rustc
          rustfmt
          shellcheck
        ])
        ++ [ config.treefmt.build.wrapper ];

      shellHook = ''
        export RUST_BACKTRACE=1
      '';
    };
  };
}
