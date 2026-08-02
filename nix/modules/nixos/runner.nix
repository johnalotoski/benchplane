{
  config,
  lib,
  pkgs,
  utils,
  ...
}:
let
  cfg = config.services.benchplane;
in
{
  options.services.benchplane.runner = {
    enable = lib.mkEnableOption "the Benchplane systemd runner";

    command = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "${pkgs.coreutils}/bin/true" ];
      description = "Command executed for the initial runner skeleton.";
    };
  };

  config = lib.mkIf (cfg.enable && cfg.runner.enable) {
    systemd.services.benchplane-runner = {
      description = "Benchplane experiment runner";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "oneshot";
        User = cfg.user;
        Group = cfg.group;
        StateDirectory = cfg.stateDirectory;
        ExecStart = utils.escapeSystemdExecArgs cfg.runner.command;
      };
    };
  };
}
