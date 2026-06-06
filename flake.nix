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
    in
    {
      checks = forAllSystems (system: {
        pre-commit-check = inputs.git-hooks.lib.${system}.run {
          src = ./.;
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
      });
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
