//! Runtime bootstrap and service discovery for the shared lspmux service.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
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
    fn parse(raw: Option<String>) -> Result<Self> {
        match raw.as_deref() {
            None | Some("") | Some("auto") => Ok(Self::Auto),
            Some("require") => Ok(Self::Require),
            Some("off") => Ok(Self::Off),
            Some(other) => {
                bail!("invalid LSPMUX_BOOTSTRAP value {other:?}; expected auto, require, or off")
            }
        }
    }
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
}

impl RuntimeConfig {
    /// Discover runtime configuration from environment variables and platform defaults.
    pub fn discover() -> Result<Self> {
        let home = std::env::var("HOME").unwrap_or_default();
        let lspmux_path = std::env::var("LSPMUX_PATH").unwrap_or_else(|_| {
            which::which("lspmux").map_or_else(
                |_| {
                    let cargo_home = std::env::var("CARGO_HOME").unwrap_or_else(|_| {
                        if home.is_empty() {
                            ".cargo".to_string()
                        } else {
                            format!("{home}/.cargo")
                        }
                    });
                    format!("{cargo_home}/bin/lspmux")
                },
                |path| path.to_string_lossy().into_owned(),
            )
        });

        let server_path = std::env::var("RUST_ANALYZER_PATH").unwrap_or_else(|_| {
            if let Ok(path) = which::which(SERVER_NAME) {
                return path.to_string_lossy().into_owned();
            }

            let xdg_data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
                if home.is_empty() {
                    ".local/share".to_string()
                } else {
                    format!("{home}/.local/share")
                }
            });
            format!("{xdg_data_home}/lspmux-rust-analyzer/current/{SERVER_NAME}")
        });

        let workspace_root = std::env::var("WORKSPACE_ROOT").ok().or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|path| path.to_str().map(ToOwned::to_owned))
        });

        let config_path = std::env::var("LSPMUX_CONFIG_PATH").unwrap_or_else(|_| {
            default_config_path(&home, std::env::var("XDG_CONFIG_HOME").ok().as_deref())
        });
        let socket_path = std::env::var("LSPMUX_SOCKET_PATH").unwrap_or_else(|_| {
            default_socket_path(
                std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
                std::env::var("TMPDIR").ok().as_deref(),
            )
        });
        let bootstrap_mode = BootstrapMode::parse(std::env::var("LSPMUX_BOOTSTRAP").ok())?;

        Ok(Self {
            lspmux_path,
            server_path,
            workspace_root,
            config_path,
            socket_path,
            bootstrap_mode,
        })
    }

    /// Ensure the shared lspmux service is available according to the bootstrap policy.
    pub async fn ensure_service_running(&self) -> Result<RuntimeStatus> {
        self.validate_prerequisites()?;

        if self.bootstrap_mode == BootstrapMode::Off {
            return Ok(self.runtime_status(ServiceMode::Skipped));
        }

        if self.socket_ready() {
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

        self.start_direct_server().await?;
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

    fn socket_ready(&self) -> bool {
        Path::new(&self.socket_path).exists()
    }

    async fn wait_for_socket(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.socket_ready() {
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
                .status()
                .await
                .context("failed to run launchctl bootstrap")?;
            return Ok(status.success());
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

    async fn start_direct_server(&self) -> Result<()> {
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
        let home = std::env::var("HOME").unwrap_or_default();
        self.config_path
            == default_config_path(&home, std::env::var("XDG_CONFIG_HOME").ok().as_deref())
    }
}

fn nix_like_uid() -> u32 {
    std::env::var("UID")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(0)
}

fn default_config_path(home: &str, xdg_config_home: Option<&str>) -> String {
    if cfg!(target_os = "macos") {
        PathBuf::from(home)
            .join("Library/Application Support/lspmux/config.toml")
            .to_string_lossy()
            .into_owned()
    } else {
        let root = xdg_config_home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".config"));
        root.join("lspmux/config.toml")
            .to_string_lossy()
            .into_owned()
    }
}

fn default_socket_path(xdg_runtime_dir: Option<&str>, tmpdir: Option<&str>) -> String {
    let base = xdg_runtime_dir
        .map(PathBuf::from)
        .or_else(|| tmpdir.map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("lspmux/lspmux.sock")
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_mode_defaults_to_auto() {
        assert_eq!(BootstrapMode::parse(None).unwrap(), BootstrapMode::Auto);
    }

    #[test]
    fn bootstrap_mode_rejects_unknown_values() {
        assert!(BootstrapMode::parse(Some("weird".to_string())).is_err());
    }

    #[test]
    fn default_socket_path_prefers_runtime_dir() {
        let path = default_socket_path(Some("/run/user/123"), Some("/tmp/custom"));
        assert_eq!(path, "/run/user/123/lspmux/lspmux.sock");
    }

    #[test]
    fn default_socket_path_falls_back_to_tmpdir() {
        let path = default_socket_path(None, Some("/tmp/custom"));
        assert_eq!(path, "/tmp/custom/lspmux/lspmux.sock");
    }

    #[test]
    fn default_config_path_uses_platform_convention() {
        let path = default_config_path("/home/test", Some("/xdg/config"));
        if cfg!(target_os = "macos") {
            assert_eq!(
                path,
                "/home/test/Library/Application Support/lspmux/config.toml"
            );
        } else {
            assert_eq!(path, "/xdg/config/lspmux/config.toml");
        }
    }
}
