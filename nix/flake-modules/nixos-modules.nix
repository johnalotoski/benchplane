{ ... }:
{
  flake.nixosModules = {
    default = import ../modules/nixos/default.nix;
    base = import ../modules/nixos/base.nix;
    runner = import ../modules/nixos/runner.nix;
    telemetry = import ../modules/nixos/telemetry.nix;
    artifacts = import ../modules/nixos/artifacts.nix;
    lifecycle = import ../modules/nixos/lifecycle.nix;
    aws = import ../modules/nixos/aws.nix;
    nvidia = import ../modules/nixos/nvidia.nix;
    runtime-vllm = import ../modules/nixos/runtime-vllm.nix;
    experiment-node = import ../modules/nixos/profile-experiment-node.nix;
  };
}
