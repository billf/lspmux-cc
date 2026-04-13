//! Runtime bootstrap and service discovery for the shared lspmux service.

use std::fs;
use std::net::TcpStream;
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
    /// Reuse an installed service when available, otherwise start one directly.
    Auto,
    /// Require a pre-existing user service and fail if it is unavailable.
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

/// Runtime status surfaced through the MCP status tool.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RuntimeStatus {
    pub bootstrap_mode: BootstrapMode,
    pub service_mode: ServiceMode,
    pub lspmux_path: String,
    pub server_path: String,
    pub config_path: String,
    pub socket_path: String,
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
            return Ok(self.runtime_status(ServiceMode::Skipped));
        }

        if self.service_ready() {
            return Ok(self.runtime_status(ServiceMode::Reused));
        }

        if self.is_default_config_path()
            && self.try_start_via_manager().await?
            && self.wait_for_socket().await
        {
            return Ok(self.runtime_status(ServiceMode::StartedViaManager));
        }

        if self.bootstrap_mode == BootstrapMode::Require {
            bail!(
                "shared lspmux service is unavailable; run `./setup core` or set \
                 LSPMUX_BOOTSTRAP=auto to allow direct fallback"
            );
        }

        self.start_direct_server()?;
        if self.wait_for_socket().await {
            return Ok(self.runtime_status(ServiceMode::StartedDirectly));
        }

        bail!(
            "started lspmux directly but socket {} did not become ready",
            self.socket_path
        );
    }

    fn runtime_status(&self, service_mode: ServiceMode) -> RuntimeStatus {
        RuntimeStatus {
            bootstrap_mode: self.bootstrap_mode,
            service_mode,
            lspmux_path: self.lspmux_path.clone(),
            server_path: self.server_path.clone(),
            config_path: self.config_path.clone(),
            socket_path: self.socket_path.clone(),
        }
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

fn tcp_is_ready(host: &str, port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("{host}:{port}").parse().unwrap(),
        StdDuration::from_millis(500),
    )
    .is_ok()
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
}
