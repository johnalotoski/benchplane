{ config, lib, ... }:
let
  cfg = config.services.benchplane;
in
{
  options.services.benchplane.artifacts = {
    enable = lib.mkEnableOption "Benchplane evidence staging and upload";

    stagingDirectory = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/${cfg.stateDirectory}/artifacts";
    };
  };

  config = lib.mkIf (cfg.enable && cfg.artifacts.enable) {
    systemd.tmpfiles.rules = [
      "d ${cfg.artifacts.stagingDirectory} 0750 ${cfg.user} ${cfg.group} -"
    ];
  };
}
