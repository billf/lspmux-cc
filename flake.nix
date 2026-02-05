{
  description = "lspmux-cc: LSP multiplexing for Claude Code";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
  };

  outputs = inputs@{ self, nixpkgs, flake-parts, crane, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-darwin" "aarch64-darwin" "x86_64-linux" "aarch64-linux" ];

      perSystem = { config, self', pkgs, system, lib, ... }:
        let
          craneLib = crane.mkLib pkgs;

          mcpServerSrc = lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              (lib.hasInfix "/mcp-server/" path)
              || (craneLib.filterCargoSources path type);
          };

          commonArgs = {
            src = mcpServerSrc;
            pname = "lspmux-cc-mcp";
            strictDeps = true;

            buildInputs = lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.Security
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
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
            packages = with pkgs; [
              # Rust
              rustc cargo clippy rustfmt rust-analyzer
              # Shell script deps
              curl jq
              # Dev tools
              just shellcheck
            ];
          };
        };
    };
}
