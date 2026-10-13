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

## Toolchain pin and proc-macro servers

`rust-toolchain.toml` pins one interactive toolchain (currently the same
rustc the flake's nixpkgs provides, so `nix develop`, crane builds, and
rustup-shim shells agree). rust-analyzer builds workspace metadata with the
cargo from its own environment: when that toolchain disagrees with whatever
built `target/`, every check rebuilds proc-macro dylibs under a different
rustc, and its proc-macro server — which enforces an exact rustc-version
match against the dylib — refuses to load them. The symptom is a
`macro-error` diagnostic (e.g. an unexpanded `#[tokio::main]`) cascading
into phantom syntax errors.

`./scripts/assert-toolchain.sh` compares the ambient rustc sysroot for identity
against the pin plus the ambient cargo release train. Inside a Nix shell,
Nix-store rustc must match the pinned version and cargo release train; stale
shells fail with a hint to exit and re-enter `nix develop`. The `just` build/test
recipes run it first and `./setup doctor` reports agreement.
When the flake's nixpkgs rev bumps rustc, bump the pin to match — CI is
unaffected (it invokes `cargo +stable` explicitly).

Recovery: a `macro-error` diagnostic names the exact rustc versions
involved (`expected` = the server's, `found` = the dylib's). If either
predates the pin, a stale rust-analyzer generation is serving the
workspace: rebuild with the pinned toolchain, then restart that
rust-analyzer instance (kill its PID; lspmux respawns it on the next
request) so it spawns the proc-macro server from the current sysroot.
Stale per-workspace servers from other sessions exit with their owners;
never kill processes you don't own.
