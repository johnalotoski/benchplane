{ config, lib, ... }:
let
  cfg = config.services.benchplane.runtime.vllm;
in
{
  options.services.benchplane.runtime.vllm = {
    enable = lib.mkEnableOption "the vLLM runtime adapter";
    model = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.model != null;
        message = "services.benchplane.runtime.vllm.model must be set when enabled.";
      }
    ];
  };
}
