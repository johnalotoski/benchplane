{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.benchplane;
in
{
  options.services.benchplane = {
    enable = lib.mkEnableOption "the Benchplane experiment runner";

    user = lib.mkOption {
      type = lib.types.str;
      default = "benchplane";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "benchplane";
    };

    stateDirectory = lib.mkOption {
      type = lib.types.str;
      default = "benchplane";
    };
  };

  config = lib.mkIf cfg.enable {
    users.groups.${cfg.group} = { };
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
    };

    environment.systemPackages = with pkgs; [
      curl
      jq
    ];
  };
}
