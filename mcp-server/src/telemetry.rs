//! In-process telemetry and accounting for the MCP server.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use metrics::{counter, histogram};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ClientIdentity {
    pub kind: String,
    pub host: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ReadinessState {
    pub health: String,
    pub quiescent: Option<bool>,
    pub message: Option<String>,
    pub updated_at_ms: Option<u64>,
}

impl Default for ReadinessState {
    fn default() -> Self {
        Self {
            health: "unknown".to_string(),
            quiescent: None,
            message: None,
            updated_at_ms: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct BootstrapTelemetry {
    pub success_count: u64,
    pub failure_count: u64,
    pub last_outcome: Option<String>,
    pub last_error: Option<String>,
    pub updated_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ToolTelemetry {
    pub call_count: u64,
    pub success_count: u64,
    pub invalid_params_count: u64,
    pub timeout_count: u64,
    pub failure_count: u64,
    pub last_latency_ms: Option<u64>,
    pub last_error: Option<String>,
    pub last_error_code: Option<String>,
    pub updated_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TelemetrySnapshot {
    pub bootstrap: BootstrapTelemetry,
    pub tools: BTreeMap<String, ToolTelemetry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CompilerAccountingSnapshot {
    pub source: String,
    pub source_path: Option<String>,
    pub last_scan_at_ms: Option<u64>,
    pub compiler_artifact_count: u64,
    pub fresh_artifact_count: u64,
    pub rebuilt_artifact_count: u64,
    pub build_script_executed_count: u64,
    pub build_finished_count: u64,
    pub parse_error_count: u64,
}

impl Default for CompilerAccountingSnapshot {
    fn default() -> Self {
        Self {
            source: "cargo_json".to_string(),
            source_path: None,
            last_scan_at_ms: None,
            compiler_artifact_count: 0,
            fresh_artifact_count: 0,
            rebuilt_artifact_count: 0,
            build_script_executed_count: 0,
            build_finished_count: 0,
            parse_error_count: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolOutcome {
    Success,
    InvalidParams,
    Timeout,
    Failure,
}

impl ToolOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::InvalidParams => "invalid_params",
            Self::Timeout => "timeout",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone)]
pub struct TelemetryState {
    client: ClientIdentity,
    inner: Arc<RwLock<TelemetryInner>>,
}

#[derive(Default)]
struct TelemetryInner {
    bootstrap: BootstrapTelemetry,
    tools: BTreeMap<String, ToolTelemetry>,
    compiler_accounting: CompilerAccountingSnapshot,
    cached_accounting_path: Option<PathBuf>,
    cached_accounting_modified_ms: Option<u64>,
}

impl TelemetryState {
    #[must_use]
    pub fn from_env() -> Self {
        let process_id = std::process::id();
        let session_id = std::env::var("LSPMUX_SESSION_ID")
            .unwrap_or_else(|_| format!("pid-{process_id}-{}", now_unix_ms().unwrap_or(0)));
        let client = ClientIdentity {
            kind: std::env::var("LSPMUX_CLIENT_KIND").unwrap_or_else(|_| "generic_mcp".to_string()),
            host: std::env::var("LSPMUX_CLIENT_HOST").unwrap_or_else(|_| "generic".to_string()),
            session_id,
        };

        Self {
            client,
            inner: Arc::new(RwLock::new(TelemetryInner::default())),
        }
    }

    #[must_use]
    pub fn client_identity(&self) -> ClientIdentity {
        self.client.clone()
    }

    pub fn record_bootstrap_success(&self, service_mode: &str) {
        let updated_at_ms = now_unix_ms();
        {
            let mut inner = self.write_inner();
            inner.bootstrap.success_count += 1;
            inner.bootstrap.last_outcome = Some(service_mode.to_string());
            inner.bootstrap.last_error = None;
            inner.bootstrap.updated_at_ms = updated_at_ms;
        }

        counter!(
            "lspmux_cc_bootstrap_total",
            "service_mode" => service_mode.to_string()
        )
        .increment(1);
    }

    pub fn record_bootstrap_failure(&self, stage: &str, error: &str) {
        let updated_at_ms = now_unix_ms();
        {
            let mut inner = self.write_inner();
            inner.bootstrap.failure_count += 1;
            inner.bootstrap.last_outcome = Some("failed".to_string());
            inner.bootstrap.last_error = Some(error.to_string());
            inner.bootstrap.updated_at_ms = updated_at_ms;
        }

        counter!("lspmux_cc_bootstrap_failures_total", "stage" => stage.to_string()).increment(1);
    }

    pub fn record_tool_result(
        &self,
        tool: &str,
        outcome: ToolOutcome,
        latency_ms: u64,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) {
        let updated_at_ms = now_unix_ms();
        {
            let mut inner = self.write_inner();
            let tool_stats = inner.tools.entry(tool.to_string()).or_default();
            tool_stats.call_count += 1;
            tool_stats.last_latency_ms = Some(latency_ms);
            tool_stats.updated_at_ms = updated_at_ms;
            tool_stats.last_error_code = error_code.map(ToOwned::to_owned);
            tool_stats.last_error = error_message.map(ToOwned::to_owned);

            match outcome {
                ToolOutcome::Success => tool_stats.success_count += 1,
                ToolOutcome::InvalidParams => tool_stats.invalid_params_count += 1,
                ToolOutcome::Timeout => {
                    tool_stats.timeout_count += 1;
                    tool_stats.failure_count += 1;
                }
                ToolOutcome::Failure => tool_stats.failure_count += 1,
            }
            drop(inner);
        }

        counter!(
            "lspmux_cc_tool_requests_total",
            "tool" => tool.to_string(),
            "client_kind" => self.client.kind.clone(),
            "outcome" => outcome.as_str().to_string()
        )
        .increment(1);
        histogram!(
            "lspmux_cc_tool_latency_seconds",
            "tool" => tool.to_string(),
            "client_kind" => self.client.kind.clone()
        )
        .record(f64::from(u32::try_from(latency_ms).unwrap_or(u32::MAX)) / 1_000.0);
    }

    #[must_use]
    pub fn snapshot(&self) -> TelemetrySnapshot {
        let inner = self.read_inner();
        TelemetrySnapshot {
            bootstrap: inner.bootstrap.clone(),
            tools: inner.tools.clone(),
        }
    }

    #[must_use]
    pub fn compiler_accounting_snapshot(&self) -> CompilerAccountingSnapshot {
        self.read_inner().compiler_accounting.clone()
    }

    pub fn refresh_compiler_accounting(&self, workspace_root: Option<&str>) {
        let Some(workspace_root) = workspace_root else {
            return;
        };
        let Some(source_path) = latest_flycheck_stdout(Path::new(workspace_root)) else {
            return;
        };

        let modified_ms = file_modified_ms(&source_path);
        {
            let inner = self.read_inner();
            if inner.cached_accounting_path.as_ref() == Some(&source_path)
                && inner.cached_accounting_modified_ms == modified_ms
            {
                return;
            }
        }

        let snapshot = parse_cargo_json_output(&source_path);
        let fresh = snapshot.fresh_artifact_count;
        let rebuilt = snapshot.rebuilt_artifact_count;
        {
            let mut inner = self.write_inner();
            inner.cached_accounting_path = Some(source_path);
            inner.cached_accounting_modified_ms = modified_ms;
            inner.compiler_accounting = snapshot;
        }

        if fresh > 0 {
            counter!("lspmux_cc_artifact_reuse_total", "result" => "fresh".to_string())
                .increment(fresh);
        }
        if rebuilt > 0 {
            counter!("lspmux_cc_artifact_reuse_total", "result" => "rebuilt".to_string())
                .increment(rebuilt);
        }
    }

    fn read_inner(&self) -> RwLockReadGuard<'_, TelemetryInner> {
        match self.inner.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_inner(&self) -> RwLockWriteGuard<'_, TelemetryInner> {
        match self.inner.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn latest_flycheck_stdout(workspace_root: &Path) -> Option<PathBuf> {
    let target_dir = workspace_root.join("target");
    let entries = fs::read_dir(target_dir).ok()?;
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }

            let name = entry.file_name();
            if !name.to_string_lossy().starts_with("flycheck") {
                return None;
            }

            let stdout_path = entry.path().join("stdout");
            let modified_ms = file_modified_ms(&stdout_path)?;
            Some((modified_ms, stdout_path))
        })
        .max_by_key(|(modified_ms, _)| *modified_ms)
        .map(|(_, path)| path)
}

fn parse_cargo_json_output(path: &Path) -> CompilerAccountingSnapshot {
    let mut snapshot = CompilerAccountingSnapshot {
        source_path: Some(path.to_string_lossy().into_owned()),
        last_scan_at_ms: now_unix_ms(),
        ..CompilerAccountingSnapshot::default()
    };

    let Ok(contents) = fs::read_to_string(path) else {
        snapshot.parse_error_count = 1;
        return snapshot;
    };

    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(line) else {
            snapshot.parse_error_count += 1;
            continue;
        };

        match value.get("reason").and_then(Value::as_str) {
            Some("compiler-artifact") => {
                snapshot.compiler_artifact_count += 1;
                if value.get("fresh").and_then(Value::as_bool).unwrap_or(false) {
                    snapshot.fresh_artifact_count += 1;
                } else {
                    snapshot.rebuilt_artifact_count += 1;
                }
            }
            Some("build-script-executed") => snapshot.build_script_executed_count += 1,
            Some("build-finished") => snapshot.build_finished_count += 1,
            Some(_) | None => {}
        }
    }

    snapshot
}

fn file_modified_ms(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    system_time_ms(modified)
}

fn now_unix_ms() -> Option<u64> {
    system_time_ms(SystemTime::now())
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    let elapsed = time.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(elapsed.as_millis()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cargo_json_summary() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("stdout");
        fs::write(
            &path,
            r#"{"reason":"compiler-artifact","fresh":true}
{"reason":"compiler-artifact","fresh":false}
{"reason":"build-script-executed"}
{"reason":"build-finished"}
not-json
"#,
        )
        .unwrap();

        let snapshot = parse_cargo_json_output(&path);
        assert_eq!(snapshot.compiler_artifact_count, 2);
        assert_eq!(snapshot.fresh_artifact_count, 1);
        assert_eq!(snapshot.rebuilt_artifact_count, 1);
        assert_eq!(snapshot.build_script_executed_count, 1);
        assert_eq!(snapshot.build_finished_count, 1);
        assert_eq!(snapshot.parse_error_count, 1);
    }

    #[test]
    fn finds_latest_flycheck_stdout() {
        let temp_dir = tempfile::tempdir().unwrap();
        let older = temp_dir.path().join("target/flycheck0");
        let newer = temp_dir.path().join("target/flycheck1");
        fs::create_dir_all(&older).unwrap();
        fs::create_dir_all(&newer).unwrap();
        fs::write(older.join("stdout"), "{}\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(newer.join("stdout"), "{}\n").unwrap();

        let latest = latest_flycheck_stdout(temp_dir.path()).unwrap();
        assert_eq!(latest, newer.join("stdout"));
    }

    #[test]
    fn tool_result_updates_snapshot() {
        let telemetry = TelemetryState::from_env();
        telemetry.record_tool_result("rust_hover", ToolOutcome::Success, 12, None, None);
        telemetry.record_tool_result(
            "rust_hover",
            ToolOutcome::Failure,
            8,
            Some("internal_error"),
            Some("boom"),
        );

        let snapshot = telemetry.snapshot();
        let tool = snapshot.tools.get("rust_hover").unwrap();
        assert_eq!(tool.call_count, 2);
        assert_eq!(tool.success_count, 1);
        assert_eq!(tool.failure_count, 1);
        assert_eq!(tool.last_error.as_deref(), Some("boom"));
        assert_eq!(tool.last_error_code.as_deref(), Some("internal_error"));
    }
}
