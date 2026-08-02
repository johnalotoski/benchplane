{
  config,
  lib,
  ...
}:
let
  cfg = config.services.benchplane;
in
{
  options.services.benchplane = {
    enable = lib.mkEnableOption "the Benchplane experiment runner";

    user = lib.mkOption {
      type = lib.types.strMatching "[a-z_][a-z0-9_-]*";
      default = "benchplane";
      description = "Unprivileged system user used by the Benchplane runner.";
    };

    group = lib.mkOption {
      type = lib.types.strMatching "[a-z_][a-z0-9_-]*";
      default = "benchplane";
      description = "System group used by the Benchplane runner.";
    };

    stateDirectory = lib.mkOption {
      type = lib.types.strMatching "[A-Za-z0-9][A-Za-z0-9_.-]*";
      default = "benchplane";
      description = ''
        Single systemd StateDirectory component used for persistent runner
        output beneath /var/lib.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    users.groups.${cfg.group} = { };
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = "/var/empty";
      createHome = false;
    };
  };
}
