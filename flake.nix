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
                    docs = {
                      git.url = "https://example.invalid/maki.git";
                      port = 8080;
                      openFirewall = true;
                    };
                    observed = {
                      source = "/srv/maki/observed";
                      port = 8081;
                      metrics.port = 4041;
                    };
                  };
                };
              }
            ];
          };
          targetService = name: targetsModuleEval.config.systemd.services."maki-${name}";
          serviceExecStart =
            name: builtins.unsafeDiscardStringContext (targetService name).serviceConfig.ExecStart;
          docsExecStart = serviceExecStart "docs";
          observedExecStart = serviceExecStart "observed";
          docsStateDirectory = builtins.unsafeDiscardStringContext (targetService "docs")
            .serviceConfig.StateDirectory;
          targetsPorts = targetsModuleEval.config.networking.firewall.allowedTCPPorts;
          docsPortIsOpen = builtins.elem 8080 targetsPorts;
          wantedByMultiUser =
            builtins.all (name: builtins.elem "multi-user.target" (targetService name).wantedBy)
              [
                "docs"
                "observed"
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

          stack-ci-contract =
            pkgs.runCommand "maki-stack-ci-contract"
              {
                nativeBuildInputs = [ pkgs.python3 ];
              }
              ''
                bash ${source}/scripts/ci/test-check-stack.sh
                PYTHONDONTWRITEBYTECODE=1 python3 ${source}/scripts/ci/test_check_stack_metadata.py
                touch $out
              '';

          nixos-module-eval = pkgs.runCommand "maki-nixos-module-eval" { } ''
            docs_exec_start=${lib.escapeShellArg docsExecStart}
            case "$docs_exec_start" in
              *"serve --git https://example.invalid/maki.git --branch main --state-dir /var/lib/maki/docs --fetch-interval 60s --host 127.0.0.1 --port 8080"*) ;;
              *)
                echo "unexpected docs ExecStart: $docs_exec_start"
                exit 1
                ;;
            esac
            case "$docs_exec_start" in
              *"--index-redirect"*)
                echo "docs target should not override maki.toml home by default"
                exit 1
                ;;
            esac

            observed_exec_start=${lib.escapeShellArg observedExecStart}
            case "$observed_exec_start" in
              *"serve /srv/maki/observed --host 127.0.0.1 --port 8081 --metrics 127.0.0.1:4041"*) ;;
              *)
                echo "unexpected observed ExecStart: $observed_exec_start"
                exit 1
                ;;
            esac

            state_directory=${lib.escapeShellArg docsStateDirectory}
            if [ "$state_directory" != maki/docs ]; then
              echo "expected StateDirectory=maki/docs, got $state_directory"
              exit 1
            fi

            firewall_open=${lib.escapeShellArg (if docsPortIsOpen then "yes" else "no")}
            if [ "$firewall_open" != yes ]; then
              echo "expected firewall port 8080 to be open"
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
          stack-ci = pkgs.mkShell {
            packages = [ pkgs.python3 ];
          };

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
