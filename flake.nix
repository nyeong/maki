{
  description = "Maki Rust development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    git-hooks.url = "github:cachix/git-hooks.nix";
    git-hooks.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { self, nixpkgs, ... }@inputs:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      forAllSystems = nixpkgs.lib.genAttrs systems;

      source = nixpkgs.lib.cleanSourceWith {
        src = ./.;
        filter =
          path: type:
          let
            name = builtins.baseNameOf path;
          in
          !(
            type == "directory"
            && builtins.elem name [
              ".direnv"
              "result"
              "target"
            ]
          );
      };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        rec {
          maki = pkgs.rustPlatform.buildRustPackage {
            pname = "maki";
            version = "0.1.0";
            src = source;
            cargoLock.lockFile = ./Cargo.lock;

            meta = {
              description = "File based personal wiki runtime";
              mainProgram = "maki";
              platforms = systems;
            };
          };

          default = maki;
        }
      );

      nixosModules = {
        maki = import ./nix/modules/maki.nix { inherit self; };
        default = self.nixosModules.maki;
      };

      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          inherit (nixpkgs) lib;
          moduleEvalSystem = if lib.hasSuffix "-linux" system then system else "x86_64-linux";
          moduleEvalPkgs = nixpkgs.legacyPackages.${moduleEvalSystem};
          fakeMaki = moduleEvalPkgs.writeShellScriptBin "maki" "exit 0";
          targetsModuleEval = lib.nixosSystem {
            system = moduleEvalSystem;
            modules = [
              self.nixosModules.maki
              {
                services.maki = {
                  enable = true;
                  package = fakeMaki;
                  targets = {
                    hanassig = {
                      git.url = "https://git.eska.nyeong.me/nyeong/hanassig";
                      host = "0.0.0.0";
                      port = 8080;
                      openFirewall = true;
                    };
                    docs = {
                      git.url = "https://git.eska.nyeong.me/nyeong/maki";
                      git.fetchInterval = "5m";
                      port = 8081;
                      openFirewall = true;
                    };
                    local = {
                      source = "/srv/local-maki";
                      port = 8082;
                      indexRedirect = "index";
                    };
                  };
                };
              }
            ];
          };
          targetService = name: targetsModuleEval.config.systemd.services."maki-${name}";
          serviceExecStart =
            name: builtins.unsafeDiscardStringContext (targetService name).serviceConfig.ExecStart;
          hanassigExecStart = serviceExecStart "hanassig";
          docsExecStart = serviceExecStart "docs";
          localExecStart = serviceExecStart "local";
          hanassigStateDirectory = builtins.unsafeDiscardStringContext (targetService "hanassig")
            .serviceConfig.StateDirectory;
          targetsPorts = targetsModuleEval.config.networking.firewall.allowedTCPPorts;
          targetsPortsAreOpen = builtins.all (port: builtins.elem port targetsPorts) [
            8080
            8081
          ];
          localPortIsClosed = !(builtins.elem 8082 targetsPorts);
          wantedByMultiUser =
            builtins.all (name: builtins.elem "multi-user.target" (targetService name).wantedBy)
              [
                "hanassig"
                "docs"
                "local"
              ];
        in
        {
          pre-commit-check = inputs.git-hooks.lib.${system}.run {
            src = ./.;
            settings.rust.check.cargoDeps = pkgs.rustPlatform.importCargoLock {
              lockFile = ./Cargo.lock;
            };
            hooks = {
              nixfmt.enable = true;
              rustfmt.enable = true;
              clippy = {
                enable = true;
                settings = {
                  allFeatures = true;
                  denyWarnings = true;
                };
              };
              statix.enable = true;
              deadnix.enable = true;
              cargo-test = {
                enable = true;
                name = "cargo test";
                entry = "cargo test";
                language = "system";
                pass_filenames = false;
                stages = [
                  "pre-commit"
                  "pre-push"
                ];
                always_run = true;
              };
            };
          };

          maki = self.packages.${system}.maki;

          package-smoke =
            pkgs.runCommand "maki-package-smoke" { nativeBuildInputs = [ self.packages.${system}.maki ]; }
              ''
                maki build ${./docs/index.maki} > page.html
                grep -q '<!doctype html>' page.html
                touch $out
              '';

          nixos-module-eval = pkgs.runCommand "maki-nixos-module-eval" { } ''
            hanassig_exec_start=${lib.escapeShellArg hanassigExecStart}
            case "$hanassig_exec_start" in
              *"serve --git https://git.eska.nyeong.me/nyeong/hanassig --branch main --state-dir /var/lib/maki/hanassig --fetch-interval 60s --host 0.0.0.0 --port 8080"*) ;;
              *)
                echo "unexpected hanassig ExecStart: $hanassig_exec_start"
                exit 1
                ;;
            esac
            case "$hanassig_exec_start" in
              *"--index-redirect"*)
                echo "hanassig target should not override maki.toml home by default"
                exit 1
                ;;
            esac

            docs_exec_start=${lib.escapeShellArg docsExecStart}
            case "$docs_exec_start" in
              *"serve --git https://git.eska.nyeong.me/nyeong/maki --branch main --state-dir /var/lib/maki/docs --fetch-interval 5m --host 127.0.0.1 --port 8081"*) ;;
              *)
                echo "unexpected docs ExecStart: $docs_exec_start"
                exit 1
                ;;
            esac

            local_exec_start=${lib.escapeShellArg localExecStart}
            case "$local_exec_start" in
              *"serve /srv/local-maki --host 127.0.0.1 --port 8082 --index-redirect index"*) ;;
              *)
                echo "unexpected local ExecStart: $local_exec_start"
                exit 1
                ;;
            esac

            state_directory=${lib.escapeShellArg hanassigStateDirectory}
            if [ "$state_directory" != maki/hanassig ]; then
              echo "expected StateDirectory=maki/hanassig, got $state_directory"
              exit 1
            fi

            firewall_open=${
              lib.escapeShellArg (if targetsPortsAreOpen && localPortIsClosed then "yes" else "no")
            }
            if [ "$firewall_open" != yes ]; then
              echo "expected firewall ports 8080 and 8081 to be open, and 8082 to stay closed"
              exit 1
            fi

            wanted_by_multi_user=${lib.escapeShellArg (if wantedByMultiUser then "yes" else "no")}
            if [ "$wanted_by_multi_user" != yes ]; then
              echo "expected maki.service to be wanted by multi-user.target"
              exit 1
            fi

            touch $out
          '';
        }
      );
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          inherit (self.checks.${system}.pre-commit-check) shellHook enabledPackages;
        in
        {
          default = pkgs.mkShell {
            inherit shellHook;
            buildInputs = enabledPackages;
            packages = with pkgs; [
              cargo
              cargo-llvm-cov
              clippy
              rust-analyzer
              rustc
              rustfmt

              eslint
              nodejs
              prettier
              typescript
              typescript-language-server
            ];

            LLVM_COV = "${pkgs.llvmPackages.llvm}/bin/llvm-cov";
            LLVM_PROFDATA = "${pkgs.llvmPackages.llvm}/bin/llvm-profdata";
            RUST_BACKTRACE = "1";
          };
        }
      );
    };
}
