{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.benchplane.nvidia;
  benchplaneCfg = config.services.benchplane;
in
{
  options.services.benchplane.nvidia = {
    enable = lib.mkEnableOption "NVIDIA GPU support for Benchplane";
  };

  config = lib.mkIf cfg.enable {
    services.xserver.videoDrivers = [ "nvidia" ];
    hardware.nvidia = {
      modesetting.enable = true;
      nvidiaPersistenced = true;
    };

    assertions = [
      {
        assertion = benchplaneCfg.enable && benchplaneCfg.runner.enable;
        message = "services.benchplane.nvidia requires the Benchplane runner.";
      }
      {
        assertion = pkgs.stdenv.hostPlatform.system == "x86_64-linux";
        message = "services.benchplane.nvidia supports only x86_64-linux.";
      }
    ];

    users.users = lib.mkIf benchplaneCfg.enable {
      ${benchplaneCfg.user}.extraGroups = [
        "video"
        "render"
      ];
    };

    systemd.services.benchplane-runner.serviceConfig = lib.mkIf benchplaneCfg.runner.enable {
      DevicePolicy = "closed";
      DeviceAllow = [
        "/dev/nvidia0 rw"
        "/dev/nvidiactl rw"
        "/dev/nvidia-uvm rw"
        "/dev/nvidia-uvm-tools rw"
      ];
    };
  };
}
