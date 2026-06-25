use crate::model::{
    AccountInfo, CreditsInfo, LimitWindow, RateLimitBucket, UsageSummary, WidgetSnapshot,
};
use chrono::Local;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Error)]
pub enum AppServerError {
    #[error("Codex command was not found on PATH")]
    CodexNotFound,
    #[error("Could not start Codex app-server: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("Could not send request to Codex app-server: {0}")]
    Write(#[source] std::io::Error),
    #[error("Codex app-server took too long to respond")]
    Timeout,
    #[error("Codex app-server returned an error for {method}: {message}")]
    Rpc {
        method: &'static str,
        message: String,
    },
    #[error("Codex app-server returned invalid data for {method}: {message}")]
    Decode {
        method: &'static str,
        message: String,
    },
    #[error("Codex app-server did not provide an output stream")]
    MissingStdout,
    #[error("Codex app-server did not provide an input stream")]
    MissingStdin,
}

#[derive(Debug, Clone)]
pub struct CodexAppServerClient {
    command_override: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppServerCommand {
    program: PathBuf,
    args: Vec<OsString>,
}

impl AppServerCommand {
    fn direct(program: PathBuf) -> Self {
        Self {
            program,
            args: Vec::new(),
        }
    }
}

impl CodexAppServerClient {
    pub fn new() -> Self {
        Self {
            command_override: env::var_os("CODEX_WIN_WIDGET_CODEX").map(PathBuf::from),
        }
    }

    pub fn fetch_snapshot(&self) -> WidgetSnapshot {
        match self.fetch_snapshot_result() {
            Ok(snapshot) => snapshot,
            Err(error) => WidgetSnapshot::error(error.to_string()),
        }
    }

    pub fn fetch_snapshot_result(&self) -> Result<WidgetSnapshot, AppServerError> {
        let command = self.resolve_codex_command()?;
        let mut child = spawn_app_server(&command)?;
        let stdout = child.stdout.take().ok_or(AppServerError::MissingStdout)?;
        let mut stdin = child.stdin.take().ok_or(AppServerError::MissingStdin)?;
        let (tx, rx) = mpsc::channel();

        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });

        write_json(
            &mut stdin,
            json!({
                "method": "initialize",
                "id": 0,
                "params": {
                    "clientInfo": {
                        "name": "codex_win_widget",
                        "title": "Codex Windows Widget",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
        )?;
        write_json(&mut stdin, json!({ "method": "initialized", "params": {} }))?;
        write_json(
            &mut stdin,
            json!({ "method": "account/read", "id": 1, "params": { "refreshToken": false } }),
        )?;
        write_json(
            &mut stdin,
            json!({ "method": "account/rateLimits/read", "id": 2 }),
        )?;
        write_json(
            &mut stdin,
            json!({ "method": "account/usage/read", "id": 3 }),
        )?;
        let responses = collect_responses(rx, REQUEST_TIMEOUT);
        drop(stdin);
        let _ = terminate_child(&mut child);
        let _ = reader.join();
        let responses = responses?;

        let account = decode_account(response_result(&responses, 1, "account/read")?)?;
        let (buckets, reset_credit_count) =
            decode_rate_limits(response_result(&responses, 2, "account/rateLimits/read")?)?;
        let usage = decode_usage(response_result(&responses, 3, "account/usage/read")?)?;

        Ok(WidgetSnapshot {
            account,
            buckets,
            reset_credit_count,
            usage,
            fetched_at: Local::now(),
            error: None,
        })
    }

    fn resolve_codex_command(&self) -> Result<AppServerCommand, AppServerError> {
        if let Some(path) = &self.command_override {
            return Ok(command_from_codex_path(path.clone()));
        }

        find_on_path("codex.cmd")
            .map(command_from_codex_path)
            .or_else(|| find_on_path("codex.exe").map(AppServerCommand::direct))
            .or_else(|| find_on_path("codex").map(AppServerCommand::direct))
            .ok_or(AppServerError::CodexNotFound)
    }
}

impl Default for CodexAppServerClient {
    fn default() -> Self {
        Self::new()
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let output = Command::new("where.exe").arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

fn command_from_codex_path(path: PathBuf) -> AppServerCommand {
    if is_cmd_path(&path) {
        if let Some(command) = node_command_from_codex_cmd(&path) {
            return command;
        }
    }
    AppServerCommand::direct(path)
}

fn is_cmd_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
}

fn node_command_from_codex_cmd(path: &Path) -> Option<AppServerCommand> {
    let directory = path.parent()?;
    let script = directory
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("bin")
        .join("codex.js");
    if !script.is_file() {
        return None;
    }

    let local_node = directory.join("node.exe");
    let node = if local_node.is_file() {
        local_node
    } else {
        find_on_path("node.exe").or_else(|| find_on_path("node"))?
    };

    Some(AppServerCommand {
        program: node,
        args: vec![script.into_os_string()],
    })
}

fn spawn_app_server(command: &AppServerCommand) -> Result<Child, AppServerError> {
    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(AppServerError::Spawn)
}

fn write_json(stdin: &mut impl Write, value: Value) -> Result<(), AppServerError> {
    writeln!(stdin, "{value}").map_err(AppServerError::Write)?;
    stdin.flush().map_err(AppServerError::Write)
}

fn collect_responses(
    rx: mpsc::Receiver<String>,
    timeout: Duration,
) -> Result<HashMap<u64, RpcResponse>, AppServerError> {
    let deadline = Instant::now() + timeout;
    let mut responses = HashMap::new();

    while !responses.contains_key(&1) || !responses.contains_key(&2) || !responses.contains_key(&3)
    {
        let now = Instant::now();
        if now >= deadline {
            return Err(AppServerError::Timeout);
        }
        let remaining = deadline.saturating_duration_since(now);
        let line = rx
            .recv_timeout(remaining)
            .map_err(|_| AppServerError::Timeout)?;
        if let Ok(response) = serde_json::from_str::<RpcResponse>(&line)
            && let Some(id) = response.id
        {
            responses.insert(id, response);
        }
    }

    Ok(responses)
}

fn response_result(
    responses: &HashMap<u64, RpcResponse>,
    id: u64,
    method: &'static str,
) -> Result<Value, AppServerError> {
    let response = responses.get(&id).ok_or(AppServerError::Timeout)?;
    if let Some(error) = &response.error {
        return Err(AppServerError::Rpc {
            method,
            message: error.message.clone(),
        });
    }
    response
        .result
        .clone()
        .ok_or_else(|| AppServerError::Decode {
            method,
            message: "missing result".to_string(),
        })
}

fn decode_account(value: Value) -> Result<Option<AccountInfo>, AppServerError> {
    let result = serde_json::from_value::<AccountReadResult>(value).map_err(|error| {
        AppServerError::Decode {
            method: "account/read",
            message: error.to_string(),
        }
    })?;

    Ok(Some(AccountInfo {
        auth_type: result
            .account
            .as_ref()
            .and_then(|account| account.kind.clone()),
        plan_type: result
            .account
            .as_ref()
            .and_then(|account| account.plan_type.clone()),
        requires_openai_auth: result.requires_openai_auth,
    }))
}

fn decode_rate_limits(value: Value) -> Result<(Vec<RateLimitBucket>, Option<u64>), AppServerError> {
    let result = serde_json::from_value::<RateLimitsReadResult>(value).map_err(|error| {
        AppServerError::Decode {
            method: "account/rateLimits/read",
            message: error.to_string(),
        }
    })?;

    let mut buckets = Vec::new();
    if let Some(map) = result.rate_limits_by_limit_id {
        let mut pairs = map.into_iter().collect::<Vec<_>>();
        pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (_, bucket) in pairs {
            buckets.push(bucket.into());
        }
    } else if let Some(bucket) = result.rate_limits {
        buckets.push(bucket.into());
    }

    Ok((
        buckets,
        result
            .rate_limit_reset_credits
            .and_then(|credits| credits.available_count),
    ))
}

fn decode_usage(value: Value) -> Result<Option<UsageSummary>, AppServerError> {
    let result = serde_json::from_value::<UsageReadResult>(value).map_err(|error| {
        AppServerError::Decode {
            method: "account/usage/read",
            message: error.to_string(),
        }
    })?;

    let latest_daily_tokens = result
        .daily_usage_buckets
        .as_ref()
        .and_then(|buckets| buckets.last())
        .and_then(|bucket| bucket.tokens);

    Ok(result.summary.map(|summary| UsageSummary {
        lifetime_tokens: summary.lifetime_tokens,
        peak_daily_tokens: summary.peak_daily_tokens,
        longest_running_turn_sec: summary.longest_running_turn_sec,
        current_streak_days: summary.current_streak_days,
        longest_streak_days: summary.longest_streak_days,
        latest_daily_tokens,
    }))
}

fn terminate_child(child: &mut Child) -> std::io::Result<()> {
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    let _ = child.wait();
    Ok(())
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    id: Option<u64>,
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct AccountReadResult {
    account: Option<AccountRaw>,
    #[serde(rename = "requiresOpenaiAuth")]
    requires_openai_auth: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AccountRaw {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "planType")]
    plan_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RateLimitsReadResult {
    #[serde(rename = "rateLimits")]
    rate_limits: Option<RateLimitBucketRaw>,
    #[serde(rename = "rateLimitsByLimitId")]
    rate_limits_by_limit_id: Option<HashMap<String, RateLimitBucketRaw>>,
    #[serde(rename = "rateLimitResetCredits")]
    rate_limit_reset_credits: Option<ResetCreditsRaw>,
}

#[derive(Debug, Deserialize)]
struct ResetCreditsRaw {
    #[serde(rename = "availableCount")]
    available_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RateLimitBucketRaw {
    #[serde(rename = "limitId")]
    limit_id: String,
    #[serde(rename = "limitName")]
    limit_name: Option<String>,
    primary: Option<LimitWindowRaw>,
    secondary: Option<LimitWindowRaw>,
    credits: Option<CreditsRaw>,
    #[serde(rename = "planType")]
    plan_type: Option<String>,
    #[serde(rename = "rateLimitReachedType")]
    rate_limit_reached_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LimitWindowRaw {
    #[serde(rename = "usedPercent")]
    used_percent: f64,
    #[serde(rename = "windowDurationMins")]
    window_duration_mins: Option<u64>,
    #[serde(rename = "resetsAt")]
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CreditsRaw {
    #[serde(rename = "hasCredits")]
    has_credits: Option<bool>,
    unlimited: Option<bool>,
    balance: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageReadResult {
    summary: Option<UsageSummaryRaw>,
    #[serde(rename = "dailyUsageBuckets")]
    daily_usage_buckets: Option<Vec<DailyUsageBucketRaw>>,
}

#[derive(Debug, Deserialize)]
struct UsageSummaryRaw {
    #[serde(rename = "lifetimeTokens")]
    lifetime_tokens: Option<u64>,
    #[serde(rename = "peakDailyTokens")]
    peak_daily_tokens: Option<u64>,
    #[serde(rename = "longestRunningTurnSec")]
    longest_running_turn_sec: Option<u64>,
    #[serde(rename = "currentStreakDays")]
    current_streak_days: Option<u64>,
    #[serde(rename = "longestStreakDays")]
    longest_streak_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DailyUsageBucketRaw {
    tokens: Option<u64>,
}

impl From<RateLimitBucketRaw> for RateLimitBucket {
    fn from(value: RateLimitBucketRaw) -> Self {
        Self {
            limit_id: value.limit_id,
            limit_name: value.limit_name,
            primary: value.primary.map(Into::into),
            secondary: value.secondary.map(Into::into),
            credits: value.credits.map(Into::into),
            plan_type: value.plan_type,
            rate_limit_reached_type: value.rate_limit_reached_type,
        }
    }
}

impl From<LimitWindowRaw> for LimitWindow {
    fn from(value: LimitWindowRaw) -> Self {
        Self {
            used_percent: value.used_percent,
            window_duration_mins: value.window_duration_mins,
            resets_at: value.resets_at,
        }
    }
}

impl From<CreditsRaw> for CreditsInfo {
    fn from(value: CreditsRaw) -> Self {
        Self {
            has_credits: value.has_credits,
            unlimited: value.unlimited,
            balance: value.balance,
        }
    }
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(not(windows))]
trait CommandExtCompat {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}

#[cfg(not(windows))]
impl CommandExtCompat for Command {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rate_limit_shape() -> Result<(), AppServerError> {
        let value = json!({
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "limitName": null,
                    "primary": {
                        "usedPercent": 31,
                        "windowDurationMins": 60,
                        "resetsAt": 1730947200
                    },
                    "secondary": null,
                    "credits": {
                        "hasCredits": true,
                        "unlimited": false,
                        "balance": "10.5"
                    },
                    "planType": "plus",
                    "rateLimitReachedType": null
                }
            },
            "rateLimitResetCredits": { "availableCount": 2 }
        });

        let (buckets, reset_credits) = decode_rate_limits(value)?;
        assert_eq!(reset_credits, Some(2));
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].limit_id, "codex");
        assert_eq!(
            buckets[0]
                .primary
                .as_ref()
                .map(|window| window.used_percent),
            Some(31.0)
        );
        assert_eq!(
            buckets[0].credits.as_ref().map(CreditsInfo::display),
            Some("10.5".to_string())
        );
        Ok(())
    }

    #[test]
    fn decodes_usage_summary() -> Result<(), AppServerError> {
        let value = json!({
            "summary": {
                "lifetimeTokens": 1234567,
                "peakDailyTokens": 45678,
                "longestRunningTurnSec": 540,
                "currentStreakDays": 8,
                "longestStreakDays": 14
            },
            "dailyUsageBuckets": [
                { "startDate": "2026-06-18", "tokens": 12345 },
                { "startDate": "2026-06-19", "tokens": 45678 }
            ]
        });

        let usage = decode_usage(value)?.ok_or_else(|| AppServerError::Decode {
            method: "account/usage/read",
            message: "missing usage".to_string(),
        })?;
        assert_eq!(usage.lifetime_tokens, Some(1_234_567));
        assert_eq!(usage.latest_daily_tokens, Some(45_678));
        Ok(())
    }

    #[test]
    fn resolves_codex_cmd_to_node_script() -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("codex-win-widget-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        std::fs::create_dir_all(&bin)?;
        let cmd = root.join("codex.cmd");
        let node = root.join("node.exe");
        let script = bin.join("codex.js");
        std::fs::write(&cmd, "")?;
        std::fs::write(&node, "")?;
        std::fs::write(&script, "")?;

        let command = command_from_codex_path(cmd);

        assert_eq!(command.program, node);
        assert_eq!(command.args, vec![script.into_os_string()]);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[ignore]
    fn fetches_local_app_server_snapshot() -> Result<(), AppServerError> {
        let snapshot = CodexAppServerClient::new().fetch_snapshot_result()?;
        assert!(snapshot.account.is_some());
        assert!(snapshot.display_bucket().is_some());
        Ok(())
    }
}
