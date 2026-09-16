# Binary resolution contracts

`lspmux-cc` deliberately resolves external binaries instead of downloading or
replacing them. An explicit environment override wins, then `PATH` is used when
applicable. The MCP server reports the selected paths at startup.

## lspmux

Set `LSPMUX_PATH` to select an executable; otherwise the runtime uses `PATH`
and then `$CARGO_HOME/bin/lspmux`. The selected path is trusted-operator
input: it is executed as the shared daemon, so point it at a genuine
`lspmux` binary. Startup requires the path to be executable and
`lspmux --version` to report exactly a SemVer release in `>=0.3.0, <0.4.0`.
This is a compatibility boundary: the project runs `lspmux server` without `--config`.

`./setup core` installs `lspmux` 0.3.0 only when it is missing. An existing
binary is validated, never silently replaced.

## rust-analyzer

`RUST_ANALYZER_PATH` wins over `PATH`; Nix development shells and the assembled
plugin wire the flake's locked Fenix `rust-analyzer` as the recommended reference
artifact. That lock makes Nix development and plugin builds reproducible.

An explicit override, a `PATH` binary, and rustup—including a nightly
rust-analyzer—remain supported user-owned choices. The MCP server probes
`rust-analyzer --version` and logs its resolved path/version. A failed version
probe is only degraded observability: it does not reject the selected binary.

There is no SHA256 downloader, automatic updater, or implicit replacement of a
user-selected rust-analyzer binary.
