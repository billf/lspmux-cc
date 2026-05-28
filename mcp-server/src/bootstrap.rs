//! Runtime bootstrap and service discovery for the shared lspmux service.

use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;

use anyhow::{bail, Context, Result};
use directories::BaseDirs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{sleep, Duration, Instant};

/// The managed LSP backend exposed by this package.
pub const SERVER_NAME: &str = "rust-analyzer";

/// Environment-controlled bootstrap behavior.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapMode {
    /// Reuse a running daemon if reachable; otherwise spawn one directly.
    Auto,
    /// Require a daemon to already be running; do not spawn. Fails fast if
    /// the daemon is unreachable.
    Require,
    /// Do not attempt to start a shared service.
    Off,
}

impl BootstrapMode {
    fn parse(raw: Option<&str>) -> Result<Self> {
        match raw {
            None | Some("" | "auto") => Ok(Self::Auto),
            Some("require") => Ok(Self::Require),
            Some("off") => Ok(Self::Off),
            Some(other) => {
                bail!("invalid LSPMUX_BOOTSTRAP value {other:?}; expected auto, require, or off")
            }
        }
    }
}

/// Transport address parsed from the lspmux config's `connect` field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectAddr {
    /// TCP host and port (e.g. `connect = ["127.0.0.1", 27631]`).
    Tcp(String, u16),
    /// Unix domain socket path (e.g. `connect = "/run/lspmux.sock"`).
    Unix(String),
}

/// Parse the `connect` field from a lspmux TOML config string.
///
/// Returns `None` if the field is missing or has an unrecognized shape.
fn parse_connect_addr(config_toml: &str) -> Option<ConnectAddr> {
    let table: toml::Table = config_toml.parse().ok()?;
    let connect = table.get("connect")?;
    parse_connect_value(connect)
}

fn parse_connect_value(value: &toml::Value) -> Option<ConnectAddr> {
    match value {
        toml::Value::String(raw) => parse_connect_string(raw),
        toml::Value::Array(arr) if arr.len() == 2 => {
            let host = arr[0].as_str()?;
            let port = arr[1].as_integer()?;
            let port = u16::try_from(port).ok()?;
            Some(ConnectAddr::Tcp(host.to_string(), port))
        }
        _ => None,
    }
}

fn parse_connect_string(raw: &str) -> Option<ConnectAddr> {
    if let Some(addr) = raw.strip_prefix("tcp://") {
        return parse_tcp_host_port(addr);
    }

    if raw.starts_with('/') {
        return Some(ConnectAddr::Unix(raw.to_string()));
    }

    parse_tcp_host_port(raw).or_else(|| Some(ConnectAddr::Unix(raw.to_string())))
}

fn parse_tcp_host_port(raw: &str) -> Option<ConnectAddr> {
    let (host, port) = raw.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    Some(ConnectAddr::Tcp(host.to_string(), port))
}

/// How the shared lspmux service was obtained.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceMode {
    Reused,
    StartedViaManager,
    StartedDirectly,
    Skipped,
}

/// One rust-analyzer instance the lspmux daemon is currently hosting.
///
/// Sourced from `lspmux status --json`. The schema is best-effort — every
/// field except `pid` and `workspace_root` may be missing on older daemon
/// versions or under unexpected output. Treat absence as "unknown."
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, JsonSchema)]
pub struct InstanceRecord {
    pub pid: u32,
    /// Workspace root as reported by the daemon (not necessarily canonical).
    pub workspace_root: String,
    /// Canonical form of `workspace_root` if the path resolves on disk.
    #[serde(default)]
    pub canonical_workspace_root: Option<String>,
    /// Milliseconds since the instance last handled an LSP request.
    #[serde(default)]
    pub idle_for_ms: Option<u64>,
    /// Number of LSP clients currently connected to this instance.
    #[serde(default)]
    pub client_count: usize,
}

/// Dynamic workspace-status fields recomputed per status query.
///
/// `RuntimeStatus` carries both static (path-resolved-at-startup) and
/// dynamic (daemon-state-now) fields. This struct isolates the dynamic
/// half so the MCP `rust_server_status` tool can refresh just those
/// fields per call without rebuilding the whole `RuntimeStatus`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceFields {
    pub served_workspaces: Vec<String>,
    pub requested_workspace: Option<String>,
    pub workspace_match: Option<bool>,
    pub daemon_pid: Option<u32>,
    pub daemon_idle_for_ms: Option<u64>,
}

/// Runtime status surfaced through the MCP status tool.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RuntimeStatus {
    pub bootstrap_mode: BootstrapMode,
    pub service_mode: ServiceMode,
    pub lspmux_path: String,
    pub server_path: String,
    pub config_path: String,
    pub socket_path: String,
    /// Canonical workspace paths the running daemon is currently serving.
    /// Empty when the daemon is unreachable or its status output can't be parsed.
    #[serde(default)]
    pub served_workspaces: Vec<String>,
    /// Canonical form of the requested `workspace_root`, when it exists on disk.
    #[serde(default)]
    pub requested_workspace: Option<String>,
    /// Whether `requested_workspace` appears in `served_workspaces`. `None` when
    /// the daemon's served workspaces couldn't be determined (e.g. daemon down,
    /// `lspmux status --json` failed, output schema unexpected).
    #[serde(default)]
    pub workspace_match: Option<bool>,
    /// PID of the rust-analyzer instance serving `requested_workspace`. `None`
    /// when there's no match or the daemon's status couldn't be read.
    #[serde(default)]
    pub daemon_pid: Option<u32>,
    /// Milliseconds since the matched instance last handled an LSP request.
    #[serde(default)]
    pub daemon_idle_for_ms: Option<u64>,
    /// True if the legacy `com.lspmux.server` launchd plist (or systemd unit)
    /// is still loaded. Post-M5, the recommended path is on-demand spawn —
    /// when this is true, the user should run `./setup migrate` to remove it.
    #[serde(default)]
    pub legacy_global_daemon_detected: bool,
}

/// Resolved runtime configuration for the MCP server.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub lspmux_path: String,
    pub server_path: String,
    pub workspace_root: Option<String>,
    pub config_path: String,
    pub socket_path: String,
    pub bootstrap_mode: BootstrapMode,
    /// Transport address parsed from the config's `connect` field, if available.
    pub connect_addr: Option<ConnectAddr>,
}

impl RuntimeConfig {
    /// Discover runtime configuration from environment variables and platform defaults.
    ///
    /// # Errors
    ///
    /// Returns an error if environment-controlled bootstrap mode is invalid.
    pub fn discover() -> Result<Self> {
        let base_dirs = BaseDirs::new();
        let home = home_dir_string(base_dirs.as_ref());
        let lspmux_path = std::env::var("LSPMUX_PATH").unwrap_or_else(|_| {
            which::which("lspmux").map_or_else(
                |_| {
                    let cargo_home =
                        std::env::var("CARGO_HOME").unwrap_or_else(|_| cargo_home_path(&home));
                    format!("{cargo_home}/bin/lspmux")
                },
                |path| path.to_string_lossy().into_owned(),
            )
        });

        let server_path = resolve_server_path(
            std::env::var("RUST_ANALYZER_PATH").ok(),
            which::which(SERVER_NAME).ok(),
        );

        let workspace_root = std::env::var("WORKSPACE_ROOT").ok().or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|path| path.to_str().map(ToOwned::to_owned))
        });

        let config_path = std::env::var("LSPMUX_CONFIG_PATH")
            .unwrap_or_else(|_| default_config_path(base_dirs.as_ref(), &home));
        let socket_path = std::env::var("LSPMUX_SOCKET_PATH").unwrap_or_else(|_| {
            default_socket_path(
                std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
                base_dirs.as_ref(),
                std::env::var("TMPDIR").ok().as_deref(),
            )
        });
        let connect_hint = std::env::var("LSPMUX_CONNECT")
            .ok()
            .or_else(|| std::env::var("LSPMUX_SOCKET_PATH").ok());
        let bootstrap_mode =
            BootstrapMode::parse(std::env::var("LSPMUX_BOOTSTRAP").ok().as_deref())?;

        let connect_addr = fs::read_to_string(&config_path)
            .ok()
            .and_then(|contents| parse_connect_addr(&contents))
            .or_else(|| connect_hint.as_deref().and_then(parse_connect_string));

        Ok(Self {
            lspmux_path,
            server_path,
            workspace_root,
            config_path,
            socket_path,
            bootstrap_mode,
            connect_addr,
        })
    }

    /// Ensure the shared lspmux service is available according to the bootstrap policy.
    ///
    /// # Errors
    ///
    /// Returns an error if prerequisites are missing or the configured bootstrap policy
    /// cannot make the shared service available.
    pub async fn ensure_service_running(&self) -> Result<RuntimeStatus> {
        self.validate_prerequisites()?;

        if self.bootstrap_mode == BootstrapMode::Off {
            return Ok(self.runtime_status(ServiceMode::Skipped).await);
        }

        if self.service_ready() {
            return Ok(self.runtime_status(ServiceMode::Reused).await);
        }

        // Service-manager bootstrap (launchd/systemd) is opt-in post-M5.
        // Default behavior is on-demand spawn via `start_direct_server`. Users
        // who explicitly want the legacy auto-start path set
        // `LSPMUX_ALLOW_MANAGER_BOOTSTRAP=1` and keep the plist/unit installed.
        if std::env::var("LSPMUX_ALLOW_MANAGER_BOOTSTRAP").as_deref() == Ok("1")
            && self.is_default_config_path()
            && self.try_start_via_manager().await?
            && self.wait_for_socket().await
        {
            return Ok(self.runtime_status(ServiceMode::StartedViaManager).await);
        }

        if self.bootstrap_mode == BootstrapMode::Require {
            bail!(
                "lspmux daemon is unreachable at {} and BootstrapMode::Require forbids spawning. \
                 Start it manually, set LSPMUX_BOOTSTRAP=auto to allow spawn, or set \
                 LSPMUX_ALLOW_MANAGER_BOOTSTRAP=1 if you still rely on the launchd/systemd unit.",
                self.socket_path
            );
        }

        self.start_direct_server()?;
        if self.wait_for_socket().await {
            return Ok(self.runtime_status(ServiceMode::StartedDirectly).await);
        }

        bail!(
            "started lspmux directly but socket {} did not become ready",
            self.socket_path
        );
    }

    /// Compute the dynamic workspace-status fields by querying the daemon now.
    ///
    /// Used by both startup-time `runtime_status` and per-call refresh from
    /// the `rust_server_status` MCP tool. The MCP tool's caller wants current
    /// state — after the `LspClient` connects and the daemon spawns an instance
    /// for the workspace — not the bootstrap snapshot, which can lag by the
    /// LSP handshake duration.
    pub async fn refresh_workspace_fields(&self, service_mode: ServiceMode) -> WorkspaceFields {
        let requested_workspace = self
            .workspace_root
            .as_deref()
            .and_then(canonicalize_workspace);
        let instances = match service_mode {
            ServiceMode::Skipped => Vec::new(),
            _ => self.discover_status().await.unwrap_or_default(),
        };
        let matched_instance = requested_workspace.as_ref().and_then(|req| {
            instances
                .iter()
                .find(|i| i.canonical_workspace_root.as_ref() == Some(req))
        });
        let workspace_match = match (&requested_workspace, service_mode) {
            (_, ServiceMode::Skipped) => None,
            (Some(_), _) if !instances.is_empty() => Some(matched_instance.is_some()),
            // No requested workspace, or daemon returned no instances:
            // can't make a determination.
            _ => None,
        };
        let daemon_pid = matched_instance.map(|i| i.pid);
        let daemon_idle_for_ms = matched_instance.and_then(|i| i.idle_for_ms);
        let served_workspaces = instances
            .into_iter()
            .filter_map(|i| i.canonical_workspace_root)
            .collect();
        WorkspaceFields {
            served_workspaces,
            requested_workspace,
            workspace_match,
            daemon_pid,
            daemon_idle_for_ms,
        }
    }

    async fn runtime_status(&self, service_mode: ServiceMode) -> RuntimeStatus {
        let fields = self.refresh_workspace_fields(service_mode).await;
        let legacy_global_daemon_detected = detect_legacy_service_manager().await;
        let status = RuntimeStatus {
            bootstrap_mode: self.bootstrap_mode,
            service_mode,
            lspmux_path: self.lspmux_path.clone(),
            server_path: self.server_path.clone(),
            config_path: self.config_path.clone(),
            socket_path: self.socket_path.clone(),
            served_workspaces: fields.served_workspaces,
            requested_workspace: fields.requested_workspace,
            workspace_match: fields.workspace_match,
            daemon_pid: fields.daemon_pid,
            daemon_idle_for_ms: fields.daemon_idle_for_ms,
            legacy_global_daemon_detected,
        };
        tracing::info!(
            event = "daemon_workspace_match",
            service_mode = ?status.service_mode,
            workspace_match = ?status.workspace_match,
            served_count = status.served_workspaces.len(),
            daemon_pid = ?status.daemon_pid,
            requested_workspace = ?status.requested_workspace,
        );
        status
    }

    /// Ask the running lspmux daemon for its full instance list.
    ///
    /// Shells out to `lspmux status --json` and parses `instances[]`.
    ///
    /// Returns:
    /// - `None` when the daemon is unreachable: the `lspmux status` command
    ///   failed to spawn, exited non-zero, timed out, or its output couldn't
    ///   be parsed.
    /// - `Some(vec)` when the daemon responded; the vector is the parsed
    ///   `instances[]`. An empty vector is the valid "daemon up but hosting
    ///   no instances" case — distinct from "daemon down."
    pub async fn discover_status(&self) -> Option<Vec<InstanceRecord>> {
        let invocation = Command::new(&self.lspmux_path)
            .arg("status")
            .arg("--json")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = match tokio::time::timeout(Duration::from_secs(2), invocation).await {
            Ok(Ok(output)) if output.status.success() => output,
            _ => return None,
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_status_instances(&stdout)
    }

    fn validate_prerequisites(&self) -> Result<()> {
        if !Path::new(&self.lspmux_path).exists() {
            bail!(
                "lspmux binary not found at {}; install it or set LSPMUX_PATH",
                self.lspmux_path
            );
        }
        if !Path::new(&self.server_path).exists() {
            bail!(
                "{SERVER_NAME} binary not found at {}; install it or set RUST_ANALYZER_PATH",
                self.server_path
            );
        }
        if !Path::new(&self.config_path).exists() {
            bail!(
                "lspmux config not found at {}; run `./setup core` or set LSPMUX_CONFIG_PATH",
                self.config_path
            );
        }
        Ok(())
    }

    fn service_ready(&self) -> bool {
        match &self.connect_addr {
            Some(ConnectAddr::Tcp(host, port)) => tcp_is_ready(host, *port),
            Some(ConnectAddr::Unix(path)) => socket_is_ready(path),
            None => socket_is_ready(&self.socket_path),
        }
    }

    async fn wait_for_socket(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.service_ready() {
                return true;
            }
            sleep(Duration::from_millis(200)).await;
        }
        false
    }

    async fn try_start_via_manager(&self) -> Result<bool> {
        #[cfg(target_os = "macos")]
        {
            let label = "com.lspmux.server";
            let plist = PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join("Library/LaunchAgents")
                .join(format!("{label}.plist"));
            if !plist.exists() {
                return Ok(false);
            }

            let status = Command::new("launchctl")
                .arg("bootstrap")
                .arg(format!("gui/{}", nix_like_uid()))
                .arg(&plist)
                .stderr(std::process::Stdio::null())
                .status()
                .await
                .context("failed to run launchctl bootstrap")?;
            // Exit code 5 means the service is already loaded, which is fine.
            let already_loaded = status.code() == Some(5);
            return Ok(status.success() || already_loaded);
        }

        #[cfg(target_os = "linux")]
        {
            let status = Command::new("systemctl")
                .args(["--user", "start", "lspmux.service"])
                .status()
                .await
                .context("failed to run systemctl --user start lspmux.service")?;
            return Ok(status.success());
        }

        #[allow(unreachable_code)]
        Ok(false)
    }

    fn start_direct_server(&self) -> Result<()> {
        let mut command = Command::new(&self.lspmux_path);
        command
            .arg("server")
            .arg("--config")
            .arg(&self.config_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        command
            .spawn()
            .context("failed to spawn lspmux server directly")?;
        Ok(())
    }

    fn is_default_config_path(&self) -> bool {
        let base_dirs = BaseDirs::new();
        self.config_path
            == default_config_path(base_dirs.as_ref(), &home_dir_string(base_dirs.as_ref()))
    }
}

fn home_dir_string(base_dirs: Option<&BaseDirs>) -> String {
    base_dirs.map_or_else(
        || std::env::var("HOME").unwrap_or_default(),
        |dirs| dirs.home_dir().to_string_lossy().into_owned(),
    )
}

fn cargo_home_path(home: &str) -> String {
    if home.is_empty() {
        ".cargo".to_string()
    } else {
        format!("{home}/.cargo")
    }
}

fn nix_like_uid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: `getuid` is a side-effect-free libc call.
        unsafe { libc::getuid() }
    }

    #[cfg(not(unix))]
    {
        0
    }
}

fn default_config_path(base_dirs: Option<&BaseDirs>, home: &str) -> String {
    if cfg!(target_os = "macos") {
        let config_root = base_dirs.map_or_else(
            || PathBuf::from(home).join("Library/Application Support"),
            |dirs| dirs.config_dir().to_path_buf(),
        );
        config_root
            .join("lspmux/config.toml")
            .to_string_lossy()
            .into_owned()
    } else {
        let root = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| base_dirs.map(|dirs| dirs.config_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from(home).join(".config"));
        root.join("lspmux/config.toml")
            .to_string_lossy()
            .into_owned()
    }
}

/// Canonicalize a workspace path. Returns `None` when the path doesn't exist
/// or isn't convertible to UTF-8.
fn canonicalize_workspace(raw: &str) -> Option<String> {
    fs::canonicalize(raw)
        .ok()
        .and_then(|p| p.to_str().map(ToOwned::to_owned))
}

/// Extract instance records from `lspmux status --json` output.
///
/// Strict at the envelope, lenient per instance. Returns `None` when the
/// output isn't the expected shape (invalid JSON, or no `instances[]` array)
/// so callers can tell "couldn't parse the response" apart from "daemon up,
/// serving nothing" (`Some(vec![])`). Within a valid envelope, individual
/// instances missing `pid` or `workspaceRoot.path` are skipped rather than
/// failing the whole parse, so one drifted entry can't blind us to the rest.
fn parse_status_instances(json: &str) -> Option<Vec<InstanceRecord>> {
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let instances = value.get("instances")?.as_array()?;
    let records = instances
        .iter()
        .filter_map(|instance| {
            let pid = u32::try_from(instance.get("pid")?.as_u64()?).ok()?;
            let workspace_root = instance
                .get("workspaceRoot")
                .and_then(|wr| wr.get("path"))
                .and_then(|p| p.as_str())?
                .to_owned();
            let canonical_workspace_root = canonicalize_workspace(&workspace_root);
            let idle_for_ms = instance.get("idleFor").and_then(serde_json::Value::as_u64);
            let client_count = instance
                .get("clients")
                .and_then(|v| v.as_array())
                .map_or(0, Vec::len);
            Some(InstanceRecord {
                pid,
                workspace_root,
                canonical_workspace_root,
                idle_for_ms,
                client_count,
            })
        })
        .collect();
    Some(records)
}

fn default_socket_path(
    xdg_runtime_dir: Option<&str>,
    base_dirs: Option<&BaseDirs>,
    tmpdir: Option<&str>,
) -> String {
    let base = xdg_runtime_dir
        .map(PathBuf::from)
        .or_else(|| base_dirs.and_then(|dirs| dirs.runtime_dir().map(PathBuf::from)))
        .or_else(|| tmpdir.map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("lspmux/lspmux.sock")
        .to_string_lossy()
        .into_owned()
}

fn resolve_server_path(configured_path: Option<String>, path_lookup: Option<PathBuf>) -> String {
    configured_path.unwrap_or_else(|| {
        path_lookup.map_or_else(
            || SERVER_NAME.to_string(),
            |path| path.to_string_lossy().into_owned(),
        )
    })
}

/// Probe for a loaded legacy `com.lspmux.server` (macOS) or `lspmux.service`
/// (Linux). Best-effort: any error returns `false` and is non-fatal. Used to
/// surface migration guidance after M5, not to gate behavior.
async fn detect_legacy_service_manager() -> bool {
    #[cfg(target_os = "macos")]
    {
        let uid = nix_like_uid();
        let target = format!("gui/{uid}/com.lspmux.server");
        let invocation = Command::new("launchctl")
            .arg("print")
            .arg(&target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .status();
        let Ok(status) = tokio::time::timeout(Duration::from_secs(2), invocation).await else {
            return false;
        };
        matches!(status, Ok(s) if s.success())
    }

    #[cfg(target_os = "linux")]
    {
        let invocation = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", "lspmux.service"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .status();
        let Ok(status) = tokio::time::timeout(Duration::from_secs(2), invocation).await else {
            return false;
        };
        matches!(status, Ok(s) if s.success())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

fn tcp_is_ready(host: &str, port: u16) -> bool {
    // `(host, port).to_socket_addrs()` accepts both IPs and hostnames
    // (resolving via /etc/hosts and DNS), so `tcp://localhost:27631` correctly
    // probes the loopback listener instead of being reported down.
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs.into_iter().any(|addr| {
        TcpStream::connect_timeout(&addr, StdDuration::from_millis(500)).is_ok()
    })
}

fn socket_is_ready(path: &str) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };

    #[cfg(unix)]
    {
        metadata.file_type().is_socket() && UnixStream::connect(path).is_ok()
    }

    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_is_ready_returns_false_for_missing_path() {
        let tempdir = tempfile::tempdir().unwrap();
        let missing = tempdir.path().join("missing.sock");
        assert!(!socket_is_ready(missing.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn nix_like_uid_matches_os_uid() {
        assert_eq!(nix_like_uid(), unsafe { libc::getuid() });
    }

    #[cfg(unix)]
    #[test]
    fn socket_is_ready_detects_unix_socket() {
        use std::os::unix::net::UnixListener;

        let tempdir = tempfile::tempdir().unwrap();
        let socket_path = tempdir.path().join("lspmux.sock");
        let _listener = UnixListener::bind(&socket_path).unwrap();

        assert!(socket_is_ready(socket_path.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn socket_ready_requires_connectable() {
        use std::os::unix::net::UnixListener;

        let tempdir = tempfile::tempdir().unwrap();
        let socket_path = tempdir.path().join("stale.sock");

        // Bind a listener to create the socket file, then drop it immediately.
        let listener = UnixListener::bind(&socket_path).unwrap();
        drop(listener);
        // On macOS under concurrent test load, the kernel can briefly serve
        // accept() from the dropped listener's queue. Yield to let close(2)
        // settle before probing — flake-stabilizer, not behavior.
        std::thread::sleep(std::time::Duration::from_millis(25));

        // The socket file still exists on disk but nobody is listening.
        assert!(!socket_is_ready(socket_path.to_str().unwrap()));
    }

    #[test]
    fn bootstrap_mode_defaults_to_auto() {
        assert_eq!(BootstrapMode::parse(None).unwrap(), BootstrapMode::Auto);
    }

    #[test]
    fn bootstrap_mode_rejects_unknown_values() {
        assert!(BootstrapMode::parse(Some("weird")).is_err());
    }

    #[test]
    fn default_socket_path_prefers_runtime_dir() {
        let path = default_socket_path(Some("/run/user/123"), None, Some("/tmp/custom"));
        assert_eq!(path, "/run/user/123/lspmux/lspmux.sock");
    }

    #[test]
    fn default_socket_path_falls_back_to_tmpdir() {
        let path = default_socket_path(None, None, Some("/tmp/custom"));
        assert_eq!(path, "/tmp/custom/lspmux/lspmux.sock");
    }

    #[test]
    fn default_config_path_uses_platform_convention() {
        let path = default_config_path(None, "/home/test");
        if cfg!(target_os = "macos") {
            assert_eq!(
                path,
                "/home/test/Library/Application Support/lspmux/config.toml"
            );
        } else {
            assert_eq!(path, "/home/test/.config/lspmux/config.toml");
        }
    }

    #[test]
    fn resolve_server_path_prefers_explicit_env() {
        let resolved = resolve_server_path(
            Some("/nix/store/pinned-rust-analyzer/bin/rust-analyzer".to_string()),
            Some(PathBuf::from("/usr/bin/rust-analyzer")),
        );
        assert_eq!(
            resolved,
            "/nix/store/pinned-rust-analyzer/bin/rust-analyzer"
        );
    }

    #[test]
    fn resolve_server_path_uses_path_lookup_when_env_missing() {
        let resolved = resolve_server_path(
            None,
            Some(PathBuf::from("/run/current-system/sw/bin/rust-analyzer")),
        );
        assert_eq!(resolved, "/run/current-system/sw/bin/rust-analyzer");
    }

    #[test]
    fn resolve_server_path_falls_back_to_binary_name() {
        let resolved = resolve_server_path(None, None);
        assert_eq!(resolved, SERVER_NAME);
    }

    #[test]
    fn parse_connect_addr_tcp() {
        let config = r#"
listen = ["127.0.0.1", 27631]
connect = ["127.0.0.1", 27631]
"#;
        assert_eq!(
            parse_connect_addr(config),
            Some(ConnectAddr::Tcp("127.0.0.1".to_string(), 27631))
        );
    }

    #[test]
    fn parse_connect_addr_unix() {
        let config = r#"connect = "/run/lspmux/lspmux.sock""#;
        assert_eq!(
            parse_connect_addr(config),
            Some(ConnectAddr::Unix("/run/lspmux/lspmux.sock".to_string()))
        );
    }

    #[test]
    fn parse_connect_addr_tcp_string() {
        let config = r#"connect = "127.0.0.1:27631""#;
        assert_eq!(
            parse_connect_addr(config),
            Some(ConnectAddr::Tcp("127.0.0.1".to_string(), 27631))
        );
    }

    #[test]
    fn parse_connect_addr_tcp_url() {
        let config = r#"connect = "tcp://127.0.0.1:27631""#;
        assert_eq!(
            parse_connect_addr(config),
            Some(ConnectAddr::Tcp("127.0.0.1".to_string(), 27631))
        );
    }

    #[test]
    fn parse_connect_addr_missing() {
        let config = r#"listen = ["127.0.0.1", 27631]"#;
        assert_eq!(parse_connect_addr(config), None);
    }

    #[test]
    fn parse_connect_string_prefers_tcp_socket_path_override() {
        assert_eq!(
            parse_connect_string("tcp://127.0.0.1:27631"),
            Some(ConnectAddr::Tcp("127.0.0.1".to_string(), 27631))
        );
    }

    #[test]
    fn parse_connect_string_accepts_unix_socket_path_override() {
        assert_eq!(
            parse_connect_string("/tmp/lspmux/lspmux.sock"),
            Some(ConnectAddr::Unix("/tmp/lspmux/lspmux.sock".to_string()))
        );
    }

    #[test]
    fn tcp_is_ready_detects_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(tcp_is_ready("127.0.0.1", port));
    }

    #[test]
    fn tcp_is_ready_returns_false_for_closed_port() {
        // Bind then immediately drop to get a port that's definitely not listening.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!tcp_is_ready("127.0.0.1", port));
    }

    #[test]
    fn tcp_is_ready_resolves_hostnames_to_listening_port() {
        // A non-IP host (e.g. `connect = "localhost:27631"`) must be resolved
        // and probed, not declared down. Bind on 127.0.0.1 and probe via
        // `localhost`: the loopback hostname resolves through /etc/hosts.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(tcp_is_ready("localhost", port));
    }

    #[test]
    fn tcp_is_ready_returns_false_for_unresolvable_host() {
        // RFC 2606 reserves the `.invalid` TLD specifically for names that must
        // never resolve. Confirms resolution failure flows to a clean `false`,
        // not a panic.
        assert!(!tcp_is_ready("nonexistent.invalid", 27631));
    }

    #[test]
    fn parse_status_instances_extracts_full_schema_from_live_output() {
        // Fixture copied from real `lspmux status --json` output. Uses /tmp so
        // canonicalize succeeds; the schema shape is the load-bearing bit.
        let tmpdir = tempfile::tempdir().unwrap();
        let ws_a = tmpdir.path().join("wt-a");
        let ws_b = tmpdir.path().join("wt-b");
        std::fs::create_dir(&ws_a).unwrap();
        std::fs::create_dir(&ws_b).unwrap();
        let json = format!(
            r#"{{
                "instances": [
                    {{
                        "pid": 81886,
                        "workspaceRoot": {{ "path": {:?}, "deviceId": 1, "fileId": 2 }},
                        "idleFor": 100,
                        "clients": [{{"id": 1, "files": []}}, {{"id": 2, "files": []}}]
                    }},
                    {{
                        "pid": 31630,
                        "workspaceRoot": {{ "path": {:?} }},
                        "clients": []
                    }}
                ]
            }}"#,
            ws_a.to_str().unwrap(),
            ws_b.to_str().unwrap()
        );
        let got = parse_status_instances(&json).expect("valid envelope parses");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].pid, 81886);
        assert_eq!(got[0].idle_for_ms, Some(100));
        assert_eq!(got[0].client_count, 2);
        assert_eq!(got[1].pid, 31630);
        assert_eq!(got[1].idle_for_ms, None);
        assert_eq!(got[1].client_count, 0);
        let canon_a = std::fs::canonicalize(&ws_a)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(got[0].canonical_workspace_root, Some(canon_a));
    }

    #[test]
    fn parse_status_instances_returns_none_for_undecipherable_envelope() {
        // Envelope shape we can't read → None, so callers report the daemon as
        // unreachable rather than "up, serving nothing."
        assert!(parse_status_instances("not json").is_none());
        assert!(parse_status_instances("{}").is_none());
        assert!(parse_status_instances(r#"{"instances": "wrong type"}"#).is_none());
    }

    #[test]
    fn parse_status_instances_returns_some_empty_for_idle_daemon() {
        // A valid envelope with no instances is "daemon up, serving nothing" —
        // distinct from the undecipherable case above.
        assert_eq!(parse_status_instances(r#"{"instances": []}"#), Some(vec![]));
    }

    #[test]
    fn parse_status_instances_skips_instances_missing_required_fields() {
        // No pid → skipped. No workspaceRoot.path → skipped. The envelope is
        // valid, so the result is Some([]) (all entries skipped), not None.
        let json = r#"{"instances": [
            {"workspaceRoot": {"path": "/tmp"}},
            {"pid": 1, "workspaceRoot": {}},
            {"pid": 2}
        ]}"#;
        assert_eq!(parse_status_instances(json), Some(vec![]));
    }

    #[test]
    fn canonicalize_workspace_returns_none_for_missing_path() {
        assert!(canonicalize_workspace("/nonexistent/path/xyzzy").is_none());
    }

    #[test]
    fn canonicalize_workspace_resolves_existing_dir() {
        let tmpdir = tempfile::tempdir().unwrap();
        let raw = tmpdir.path().to_str().unwrap();
        let canon = canonicalize_workspace(raw).unwrap();
        // Result should be canonical (idempotent under a second pass).
        assert_eq!(canonicalize_workspace(&canon).unwrap(), canon);
    }

    #[tokio::test]
    async fn detect_legacy_service_manager_returns_false_for_missing_unit() {
        // On CI without a legacy launchd/systemd unit, this is deterministic.
        // Local devs who happen to have com.lspmux.server loaded will see this
        // fail with a clear message rather than a silent pass.
        assert!(
            !detect_legacy_service_manager().await,
            "expected no legacy lspmux service-manager unit; install ./setup migrate to remove one"
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_workspace_resolves_symlinks() {
        let tmpdir = tempfile::tempdir().unwrap();
        let target = tmpdir.path().join("real");
        let link = tmpdir.path().join("link");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let canon_target = canonicalize_workspace(target.to_str().unwrap()).unwrap();
        let canon_link = canonicalize_workspace(link.to_str().unwrap()).unwrap();
        assert_eq!(canon_target, canon_link);
    }
}
