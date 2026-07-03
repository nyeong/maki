{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.maki;
in
{
  options.services.maki = {
    enable = lib.mkEnableOption "Maki personal wiki server";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.maki;
      defaultText = lib.literalExpression "self.packages.\${pkgs.stdenv.hostPlatform.system}.maki";
      description = "Maki package to run.";
    };

    source = lib.mkOption {
      type = lib.types.str;
      example = "/srv/hanassig";
      description = "Runtime path to the Maki note directory.";
    };

    host = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      example = "0.0.0.0";
      description = "Host address for maki serve to bind.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 4000;
      example = 8080;
      description = "TCP port for maki serve to bind.";
    };

    indexRedirect = lib.mkOption {
      type = lib.types.str;
      default = "README";
      example = "index";
      description = "Document route that / redirects to.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "maki";
      description = "User account that runs the maki service.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "maki";
      description = "Group account that runs the maki service.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether to open the configured TCP port in the NixOS firewall.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasPrefix "/" cfg.source;
        message = "services.maki.source must be an absolute runtime path string, e.g. \"/srv/hanassig\".";
      }
    ];

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];

    users.groups = lib.mkIf (cfg.group == "maki") {
      maki = { };
    };

    users.users = lib.mkIf (cfg.user == "maki") {
      maki = {
        isSystemUser = true;
        inherit (cfg) group;
      };
    };

    systemd.services.maki = {
      description = "Maki personal wiki server";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        ExecStart = lib.escapeShellArgs [
          "${cfg.package}/bin/maki"
          "serve"
          cfg.source
          "--host"
          cfg.host
          "--port"
          (toString cfg.port)
          "--index-redirect"
          cfg.indexRedirect
        ];
        Restart = "on-failure";
        RestartSec = "2s";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.source;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ReadOnlyPaths = [ cfg.source ];
      };
    };
  };
}
