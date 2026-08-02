{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.benchplane.aws;
in
{
  options.services.benchplane.aws = {
    enable = lib.mkEnableOption "AWS-specific Benchplane node integration";
    enableSsm = lib.mkOption {
      type = lib.types.bool;
      default = true;
    };
  };

  config = lib.mkIf cfg.enable {
    services.amazon-ssm-agent.enable = cfg.enableSsm;
    environment.systemPackages = [ pkgs.awscli2 ];
  };
}
