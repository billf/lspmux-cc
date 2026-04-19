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
    lspmux-src = {
      url = "git+https://codeberg.org/p2502/lspmux?ref=main";
      flake = false;
    };
  };

  outputs = inputs@{ self, nixpkgs, flake-parts, crane, fenix, lspmux-src, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-darwin" "aarch64-darwin" "x86_64-linux" "aarch64-linux" ];

      imports = [
        inputs.flake-parts.flakeModules.easyOverlay
      ];

      perSystem = { config, self', pkgs, system, lib, ... }:
        let
          craneLib = crane.mkLib pkgs;

          rust-analyzer = fenix.packages.${system}.rust-analyzer;
          rust-analyzer-nightly = rust-analyzer;

          mcpServerSrc = craneLib.cleanCargoSource ./mcp-server;

          commonArgs = {
            src = mcpServerSrc;
            strictDeps = true;

            buildInputs = lib.optionals pkgs.stdenv.isDarwin [
              pkgs.libiconv
            ];
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          jqBin = "${pkgs.jq}/bin";
        in
        {
          overlayAttrs = {
            inherit (config.packages) lspmux-cc-mcp lspmux rust-analyzer rust-analyzer-nightly plugin;
          };

          packages = {
            lspmux-cc-mcp = craneLib.buildPackage (commonArgs // {
              inherit cargoArtifacts;
              meta.mainProgram = "lspmux-cc-mcp";
            });
            lspmux = craneLib.buildPackage {
              src = craneLib.cleanCargoSource lspmux-src;
              strictDeps = true;
              buildInputs = lib.optionals pkgs.stdenv.isDarwin [
                pkgs.libiconv
              ];
              meta.mainProgram = "lspmux";
            };
            inherit rust-analyzer rust-analyzer-nightly;

            plugin = pkgs.runCommand "lspmux-rust-cc-plugin" {
              meta.description = "lspmux-rust-cc Claude Code plugin (Nix-assembled)";
            } ''
              mkdir -p $out/.claude-plugin $out/bin $out/hooks/scripts $out/skills

              # Static files
              cp ${./.claude-plugin/plugin.json} $out/.claude-plugin/plugin.json
              cp ${./.mcp.json}                  $out/.mcp.json
              cp ${./.lsp.json}                  $out/.lsp.json
              cp ${./hooks/hooks.json}           $out/hooks/hooks.json
              cp -r ${./skills}/*                $out/skills/

              # Bin symlinks -> Nix store binaries
              ln -s ${self'.packages.lspmux}/bin/lspmux                $out/bin/lspmux
              ln -s ${self'.packages.lspmux-cc-mcp}/bin/lspmux-cc-mcp  $out/bin/lspmux-cc-mcp
              ln -s ${self'.packages.rust-analyzer}/bin/rust-analyzer   $out/bin/rust-analyzer

              # Hook scripts with jq store-path injected onto PATH
              for script in session-start.sh post-file-edit.sh; do
                {
                  echo '#!/usr/bin/env bash'
                  echo 'export PATH="${jqBin}:''${PATH:-}"'
                  tail -n +2 ${./hooks/scripts}/$script
                } > $out/hooks/scripts/$script
                chmod +x $out/hooks/scripts/$script
              done
            '';

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
            plugin-structure = pkgs.runCommand "check-plugin-structure" {} ''
              test -f ${self'.packages.plugin}/.claude-plugin/plugin.json
              test -f ${self'.packages.plugin}/.mcp.json
              test -f ${self'.packages.plugin}/.lsp.json
              test -L ${self'.packages.plugin}/bin/lspmux
              test -L ${self'.packages.plugin}/bin/lspmux-cc-mcp
              test -L ${self'.packages.plugin}/bin/rust-analyzer
              test -x ${self'.packages.plugin}/hooks/scripts/session-start.sh
              test -x ${self'.packages.plugin}/hooks/scripts/post-file-edit.sh
              test -f ${self'.packages.plugin}/skills/diagnose-lspmux/SKILL.md
              test -f ${self'.packages.plugin}/skills/rust-diagnostics/SKILL.md
              test -x "$(readlink ${self'.packages.plugin}/bin/lspmux)"
              test -x "$(readlink ${self'.packages.plugin}/bin/lspmux-cc-mcp)"
              touch $out
            '';
          };

          devShells.default = pkgs.mkShell {
            inputsFrom = [ self'.packages.lspmux-cc-mcp ];
            packages = [
              # Rust
              pkgs.rustc pkgs.cargo pkgs.clippy pkgs.rustfmt
              self'.packages.rust-analyzer
              self'.packages.lspmux
              # Shell script deps
              pkgs.curl pkgs.jq
              # Dev tools
              pkgs.just pkgs.shellcheck
            ];
          };
        };
    };
}
