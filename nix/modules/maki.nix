{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.maki;

  gitSourceModule = lib.types.submodule {
    options = {
      url = lib.mkOption {
        type = lib.types.str;
        example = "https://git.example.com/me/wiki.git";
        description = "Git repository URL to serve.";
      };

      branch = lib.mkOption {
        type = lib.types.str;
        default = "main";
        example = "main";
        description = "Git branch to serve.";
      };

      fetchInterval = lib.mkOption {
        type = lib.types.str;
        default = "60s";
        example = "5m";
        description = "How often maki fetches the repository.";
      };

      stateDir = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/var/lib/maki/docs";
        description = ''
          Runtime state directory for the git mirror and checked-out releases.
          When unset, the NixOS module uses /var/lib/maki/<target-name>.
        '';
      };
    };
  };

  targetModule = lib.types.submodule {
    options = {
      source = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/srv/hanassig";
        description = "Runtime path to a local Maki project or note directory.";
      };

      git = lib.mkOption {
        type = lib.types.nullOr gitSourceModule;
        default = null;
        description = "Git repository source for this Maki target.";
      };

      host = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        example = "0.0.0.0";
        description = "Host address for this maki serve instance to bind.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        example = 8080;
        description = "TCP port for this maki serve instance to bind.";
      };

      indexRedirect = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "index";
        description = ''
          Optional document route that / redirects to. When unset, maki.toml
          decides the home route.
        '';
      };

      openFirewall = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Whether to open this target's TCP port in the NixOS firewall.";
      };
    };
  };

  instances = lib.mapAttrsToList (name: target: {
    unitName = "maki-${name}";
    targetName = name;
    inherit target;
  }) cfg.targets;

  isGitTarget = target: target.git != null;
  isLocalTarget = target: target.source != null;

  gitStateDir =
    name: target: if target.git.stateDir == null then "/var/lib/maki/${name}" else target.git.stateDir;

  optionalArg =
    option: value:
    lib.optionals (value != null) [
      option
      value
    ];

  sourceArgs =
    name: target:
    if isGitTarget target then
      [
        "--git"
        target.git.url
        "--branch"
        target.git.branch
        "--state-dir"
        (gitStateDir name target)
        "--fetch-interval"
        target.git.fetchInterval
      ]
    else
      [ target.source ];

  serveArgs =
    name: target:
    [
      "${cfg.package}/bin/maki"
      "serve"
    ]
    ++ sourceArgs name target
    ++ [
      "--host"
      target.host
      "--port"
      (toString target.port)
    ]
    ++ optionalArg "--index-redirect" target.indexRedirect;

  serviceFor =
    instance:
    lib.nameValuePair instance.unitName (
      let
        inherit (instance) target;
      in
      {
        description = "Maki personal wiki server (${instance.targetName})";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ] ++ lib.optionals (isGitTarget target) [ "network-online.target" ];
        wants = lib.optionals (isGitTarget target) [ "network-online.target" ];
        path = lib.optionals (isGitTarget target) [ pkgs.gitMinimal ];

        serviceConfig = {
          ExecStart = lib.escapeShellArgs (serveArgs instance.targetName target);
          Restart = "on-failure";
          RestartSec = "2s";
          User = cfg.user;
          Group = cfg.group;
          WorkingDirectory =
            if isGitTarget target then gitStateDir instance.targetName target else target.source;
          NoNewPrivileges = true;
          PrivateTmp = true;
          ProtectSystem = "strict";
        }
        // lib.optionalAttrs (isLocalTarget target) {
          ReadOnlyPaths = [ target.source ];
        }
        // lib.optionalAttrs (isGitTarget target && target.git.stateDir == null) {
          StateDirectory = "maki/${instance.targetName}";
        }
        // lib.optionalAttrs (isGitTarget target && target.git.stateDir != null) {
          ReadWritePaths = [ target.git.stateDir ];
        };
      }
    );

  targetAssertions = lib.concatMap (
    instance:
    let
      inherit (instance) target;
    in
    [
      {
        assertion = builtins.match "[A-Za-z0-9_-]+" instance.targetName != null;
        message = "services.maki target name '${instance.targetName}' must contain only letters, numbers, '_' or '-'.";
      }
      {
        assertion = isLocalTarget target != isGitTarget target;
        message = "services.maki target '${instance.targetName}' must set exactly one of source or git.";
      }
      {
        assertion = target.source == null || lib.hasPrefix "/" target.source;
        message = "services.maki target '${instance.targetName}' source must be an absolute runtime path string.";
      }
      {
        assertion = !isGitTarget target || target.git.url != "";
        message = "services.maki target '${instance.targetName}' git.url must not be empty.";
      }
      {
        assertion =
          !isGitTarget target || target.git.stateDir == null || lib.hasPrefix "/" target.git.stateDir;
        message = "services.maki target '${instance.targetName}' git.stateDir must be an absolute runtime path string.";
      }
    ]
  ) instances;

  bindAddresses = map (
    instance: "${instance.target.host}:${toString instance.target.port}"
  ) instances;
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

    targets = lib.mkOption {
      type = lib.types.attrsOf targetModule;
      default = { };
      example = lib.literalExpression ''
        {
          hanassig = {
            git.url = "https://git.example.com/me/hanassig.git";
            port = 4000;
          };
          docs = {
            git.url = "https://git.example.com/me/maki.git";
            port = 4001;
          };
        }
      '';
      description = "Named Maki serve instances.";
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

  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.targets != { };
        message = "services.maki requires at least one named target.";
      }
      {
        assertion = builtins.length bindAddresses == builtins.length (lib.unique bindAddresses);
        message = "services.maki targets must not bind the same host:port more than once.";
      }
    ]
    ++ targetAssertions;

    networking.firewall.allowedTCPPorts = lib.unique (
      lib.concatMap (
        instance: lib.optionals instance.target.openFirewall [ instance.target.port ]
      ) instances
    );

    users.groups = lib.mkIf (cfg.group == "maki") {
      maki = { };
    };

    users.users = lib.mkIf (cfg.user == "maki") {
      maki = {
        isSystemUser = true;
        inherit (cfg) group;
      };
    };

    systemd.services = lib.listToAttrs (map serviceFor instances);
  };
}
