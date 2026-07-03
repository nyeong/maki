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
          moduleEval = lib.nixosSystem {
            system = moduleEvalSystem;
            modules = [
              self.nixosModules.maki
              {
                services.maki = {
                  enable = true;
                  package = fakeMaki;
                  source = "/srv/hanassig";
                  host = "0.0.0.0";
                  port = 8080;
                  indexRedirect = "index";
                  openFirewall = true;
                };
              }
            ];
          };
          execStart = builtins.unsafeDiscardStringContext moduleEval.config.systemd.services.maki.serviceConfig.ExecStart;
          portIsOpen = builtins.elem 8080 moduleEval.config.networking.firewall.allowedTCPPorts;
          wantedByMultiUser = builtins.elem "multi-user.target" moduleEval.config.systemd.services.maki.wantedBy;
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
            exec_start=${lib.escapeShellArg execStart}
            case "$exec_start" in
              *"serve /srv/hanassig --host 0.0.0.0 --port 8080 --index-redirect index"*) ;;
              *)
                echo "unexpected ExecStart: $exec_start"
                exit 1
                ;;
            esac

            firewall_open=${lib.escapeShellArg (if portIsOpen then "yes" else "no")}
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
          default = pkgs.mkShell {
            inherit shellHook;
            buildInputs = enabledPackages;
            packages = with pkgs; [
              cargo
              clippy
              rust-analyzer
              rustc
              rustfmt
              cargo-llvm-cov
            ];

            LLVM_COV = "${pkgs.llvmPackages.llvm}/bin/llvm-cov";
            LLVM_PROFDATA = "${pkgs.llvmPackages.llvm}/bin/llvm-profdata";
            RUST_BACKTRACE = "1";
          };
        }
      );
    };
}
