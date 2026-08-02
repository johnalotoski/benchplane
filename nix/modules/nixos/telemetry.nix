{ config, lib, ... }:
let
  cfg = config.services.benchplane;
in
{
  options.services.benchplane.telemetry = {
    enable = lib.mkEnableOption "Benchplane host and accelerator telemetry";

    sampleIntervalSeconds = lib.mkOption {
      type = lib.types.ints.positive;
      default = 1;
    };
  };

  config = lib.mkIf (cfg.enable && cfg.telemetry.enable) {
    assertions = [
      {
        assertion = cfg.telemetry.sampleIntervalSeconds > 0;
        message = "Benchplane telemetry interval must be positive.";
      }
    ];
  };
}
