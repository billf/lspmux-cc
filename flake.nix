{
  description = "lspmux-cc: LSP multiplexing for Claude Code";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs@{ self, nixpkgs, flake-parts, crane, fenix, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-darwin" "aarch64-darwin" "x86_64-linux" "aarch64-linux" ];

      perSystem = { config, self', pkgs, system, lib, ... }:
        let
          craneLib = crane.mkLib pkgs;

          rust-analyzer-nightly = fenix.packages.${system}.rust-analyzer;

          mcpServerSrc = craneLib.cleanCargoSource ./mcp-server;

          commonArgs = {
            src = mcpServerSrc;
            strictDeps = true;

            buildInputs = lib.optionals pkgs.stdenv.isDarwin [
              pkgs.libiconv
            ];
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          packages = {
            lspmux-cc-mcp = craneLib.buildPackage (commonArgs // {
              inherit cargoArtifacts;
            });
            inherit rust-analyzer-nightly;
            default = self'.packages.lspmux-cc-mcp;
          };

          checks = {
            clippy = craneLib.cargoClippy (commonArgs // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- -W clippy::nursery -W clippy::pedantic";
            });
            fmt = craneLib.cargoFmt { src = mcpServerSrc; };
            tests = craneLib.cargoTest (commonArgs // {
              inherit cargoArtifacts;
            });
          };

          devShells.default = pkgs.mkShell {
            inputsFrom = [ self'.packages.lspmux-cc-mcp ];
            packages = [
              # Rust
              pkgs.rustc pkgs.cargo pkgs.clippy pkgs.rustfmt
              rust-analyzer-nightly
              # Shell script deps
              pkgs.curl pkgs.jq
              # Dev tools
              pkgs.just pkgs.shellcheck
            ];
          };
        };
    };
}
