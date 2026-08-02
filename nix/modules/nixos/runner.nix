{
  config,
  lib,
  utils,
  ...
}:
let
  cfg = config.services.benchplane;
  runnerCfg = cfg.runner;
  outputRoot = "/var/lib/${cfg.stateDirectory}";
in
{
  options.services.benchplane.runner = {
    enable = lib.mkEnableOption "the Benchplane systemd runner";

    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      description = "Benchplane package whose public CLI the runner executes.";
    };

    experimentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Read-only experiment specification executed by each explicit service
        activation.
      '';
    };
  };

  config = lib.mkIf (cfg.enable && runnerCfg.enable) {
    assertions = [
      {
        assertion = runnerCfg.package != null;
        message = "services.benchplane.runner.package must be set when the runner is enabled.";
      }
      {
        assertion = runnerCfg.experimentFile != null;
        message = "services.benchplane.runner.experimentFile must be set when the runner is enabled.";
      }
    ];

    systemd.services.benchplane-runner = {
      description = "Benchplane experiment runner";

      serviceConfig = {
        Type = "oneshot";
        Restart = "no";
        User = cfg.user;
        Group = cfg.group;
        StateDirectory = cfg.stateDirectory;
        StateDirectoryMode = "0750";
        UMask = "0027";
        TimeoutStartSec = "${toString cfg.lifecycle.maximumRuntimeSeconds}s";
        ExecStart = utils.escapeSystemdExecArgs [
          "${runnerCfg.package}/bin/benchplane"
          "run"
          runnerCfg.experimentFile
          "--output-root"
          outputRoot
        ];

        StandardOutput = "journal";
        StandardError = "journal";

        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        CapabilityBoundingSet = "";
        AmbientCapabilities = "";
        RestrictSUIDSGID = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        LockPersonality = true;
      };
    };
  };
}
