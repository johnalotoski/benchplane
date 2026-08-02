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

    shutdownAfterFinalization = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Set only after upload and terminal-state handling are implemented.";
    };
  };

  config = lib.mkIf (cfg.enable && cfg.lifecycle.enable) {
    assertions = [
      {
        assertion = cfg.lifecycle.maximumRuntimeSeconds <= 86400;
        message = "The initial Benchplane lifecycle guard rejects runtimes over 24 hours.";
      }
    ];
  };
}
