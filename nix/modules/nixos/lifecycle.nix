{ config, lib, ... }:
let
  cfg = config.services.benchplane;
in
{
  options.services.benchplane.lifecycle = {
    enable = lib.mkEnableOption "Benchplane lifecycle and deadline enforcement";

    maximumRuntimeSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 3600;
    };

  };

  config = lib.mkIf (cfg.enable && (cfg.lifecycle.enable || cfg.runner.enable)) {
    assertions = [
      {
        assertion = cfg.lifecycle.maximumRuntimeSeconds <= 86400;
        message = "The initial Benchplane lifecycle guard rejects runtimes over 24 hours.";
      }
    ];
  };
}
