{ config, lib, ... }:
let
  cfg = config.services.benchplane.nvidia;
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
  };
}
