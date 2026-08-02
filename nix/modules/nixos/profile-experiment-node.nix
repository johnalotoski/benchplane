{ ... }:
{
  imports = [
    ./default.nix
    ./aws.nix
    ./nvidia.nix
    ./runtime-vllm.nix
  ];

  services.benchplane = {
    enable = true;
    runner.enable = true;
    telemetry.enable = true;
    artifacts.enable = true;
    lifecycle.enable = true;
  };
}
