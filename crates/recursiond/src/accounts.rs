//! Several logins per CLI. Each extra account is a config folder (`~/.recursion/accounts/<id>/`)
//! handed to the CLI as `CLAUDE_CONFIG_DIR` / `CODEX_HOME`; only the login lives there, the rest
//! (settings, CLAUDE.md, skills, …) is linked back to `~/.claude` / `~/.codex`. The default account
//! of each kind is the CLI's own folder, untouched. An agent keeps the account it was created with:
//! its conversation is stored in that folder.

use crate::workspace::{now_ms, AgentKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: AgentKind,
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Subscription, e.g. "pro", "max", "plus".
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub logged_in: bool,
    /// Latest rate-limit utilisation (0–1) seen from any agent on this account, e.g. {"five_hour": 0.3}.
    #[serde(default)]
    pub usage: BTreeMap<String, f64>,
    #[serde(default)]
    pub usage_at: u64,
    /// When each window resets (seconds since epoch), e.g. {"five_hour": 1795245000}.
    #[serde(default)]
    pub resets: BTreeMap<String, u64>,
}

pub fn default_id(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "claude-default",
        AgentKind::Codex => "codex-default",
    }
}

fn file() -> PathBuf {
    crate::home().join("accounts.json")
}

/// The config folder of a non-default account.
pub fn dir(id: &str) -> Option<PathBuf> {
    (!id.ends_with("-default") && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| crate::home().join("accounts").join(id))
}

fn env_var(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "CLAUDE_CONFIG_DIR",
        AgentKind::Codex => "CODEX_HOME",
    }
}

/// Shared from the CLI's own folder into every extra account.
fn shared(kind: AgentKind) -> (PathBuf, &'static [&'static str]) {
    let home = dirs::home_dir().unwrap_or_default();
    match kind {
        AgentKind::Claude => (home.join(".claude"), &["settings.json", "settings.local.json", "CLAUDE.md", "skills", "agents", "commands", "plugins", "output-styles"]),
        AgentKind::Codex => (home.join(".codex"), &["config.toml", "AGENTS.md", "prompts", "skills"]),
    }
}

/// A shell-safe `VAR='dir' ` prefix for commands typed into a terminal ("" for a default account).
pub fn env_prefix(kind: AgentKind, id: &str) -> String {
    match dir(id) {
        Some(d) => format!("{}='{}' ", env_var(kind), d.to_string_lossy().replace('\'', r"'\''")),
        None => String::new(),
    }
}

pub fn login_command(kind: AgentKind, id: &str) -> String {
    let p = env_prefix(kind, id);
    match kind {
        AgentKind::Claude => format!("{p}claude auth login"),
        AgentKind::Codex => format!("{p}codex login"),
    }
}

#[derive(Default)]
pub struct Accounts {
    list: Mutex<Vec<Account>>,
}

impl Accounts {
    pub fn load() -> Self {
        let mut list: Vec<Account> = std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            if !list.iter().any(|a| a.id == default_id(kind)) {
                list.insert(0, Account { id: default_id(kind).into(), kind, name: "預設".into(), email: None, plan: None, logged_in: false, usage: Default::default(), usage_at: 0, resets: Default::default() });
            }
        }
        Accounts { list: Mutex::new(list) }
    }

    fn save(&self) {
        let json = serde_json::to_vec_pretty(&*self.list.lock().unwrap()).unwrap();
        let tmp = file().with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, file());
        }
    }

    pub fn list(&self) -> Vec<Account> {
        self.list.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Account> {
        self.list.lock().unwrap().iter().find(|a| a.id == id).cloned()
    }

    /// The account an agent runs under: its own, or the default of its kind.
    pub fn resolve(&self, kind: AgentKind, id: Option<&str>) -> String {
        id.filter(|i| self.get(i).is_some_and(|a| a.kind == kind)).unwrap_or(default_id(kind)).to_string()
    }

    /// Point a CLI command at the account's folder (nothing to do for a default account).
    pub fn apply(&self, cmd: &mut Command, kind: AgentKind, id: Option<&str>) {
        if let Some(d) = dir(&self.resolve(kind, id)) {
            cmd.env(env_var(kind), d);
        }
    }

    pub fn create(&self, kind: AgentKind, name: &str) -> Result<Account, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("帳號名稱不能是空的".into());
        }
        if self.list.lock().unwrap().iter().any(|a| a.kind == kind && a.name == name) {
            return Err(format!("已經有叫「{name}」的帳號了"));
        }
        let id = crate::model::new_id();
        let d = dir(&id).ok_or("帳號 id 無效")?;
        std::fs::create_dir_all(&d).map_err(|e| format!("無法建立帳號資料夾：{e}"))?;
        let (src, items) = shared(kind);
        for item in items {
            let from = src.join(item);
            if from.exists() {
                #[cfg(unix)]
                let _ = std::os::unix::fs::symlink(&from, d.join(item));
            }
        }
        let a = Account { id, kind, name: name.into(), email: None, plan: None, logged_in: false, usage: Default::default(), usage_at: 0, resets: Default::default() };
        self.list.lock().unwrap().push(a.clone());
        self.save();
        Ok(a)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("帳號名稱不能是空的".into());
        }
        let mut list = self.list.lock().unwrap();
        let a = list.iter_mut().find(|a| a.id == id).ok_or("找不到這個帳號")?;
        a.name = name.into();
        drop(list);
        self.save();
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let d = dir(id).ok_or("預設帳號不能刪除")?;
        self.list.lock().unwrap().retain(|a| a.id != id);
        self.save();
        let _ = std::fs::remove_dir_all(d);
        Ok(())
    }

    /// Latest limits seen from an agent on this account. Returns whether anything changed.
    pub fn record_usage(&self, id: &str, usage: &BTreeMap<String, f64>, resets: &BTreeMap<String, u64>) -> bool {
        let mut list = self.list.lock().unwrap();
        let Some(a) = list.iter_mut().find(|a| a.id == id) else { return false };
        if &a.usage == usage && (resets.is_empty() || &a.resets == resets) {
            return false;
        }
        a.usage = usage.clone();
        if !resets.is_empty() {
            a.resets = resets.clone();
        }
        a.usage_at = now_ms();
        drop(list);
        self.save();
        true
    }

    /// Ask the CLI who is logged in on this account. Returns whether anything changed.
    pub async fn refresh(&self, id: &str, path: &str, claude: Option<&str>, codex: Option<&str>) -> bool {
        let Some(a) = self.get(id) else { return false };
        let (logged_in, email, plan) = match a.kind {
            AgentKind::Claude => {
                let Some(bin) = claude else { return false };
                let mut cmd = Command::new(bin);
                cmd.args(["auth", "status"]).env("PATH", path);
                self.apply(&mut cmd, a.kind, Some(id));
                let out = tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output()).await;
                let Ok(Ok(out)) = out else { return false };
                let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
                (v["loggedIn"].as_bool().unwrap_or(false), v["email"].as_str().map(String::from), v["subscriptionType"].as_str().map(String::from))
            }
            AgentKind::Codex => {
                let Some(bin) = codex else { return false };
                let mut cmd = Command::new(bin);
                cmd.args(["login", "status"]).env("PATH", path);
                self.apply(&mut cmd, a.kind, Some(id));
                let out = tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output()).await;
                let Ok(Ok(out)) = out else { return false };
                let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                let ok = out.status.success() && !text.to_lowercase().contains("not logged in");
                let (email, plan) = codex_identity(id);
                (ok, email, plan)
            }
        };
        let mut list = self.list.lock().unwrap();
        let Some(a) = list.iter_mut().find(|a| a.id == id) else { return false };
        let changed = a.logged_in != logged_in || a.email != email || a.plan != plan;
        a.logged_in = logged_in;
        if logged_in {
            a.email = email;
            a.plan = plan;
        }
        drop(list);
        if changed {
            self.save();
        }
        changed
    }
}

/// When the account's login file last changed: a fresh login rewrites it, even if the CLI
/// already counted the account as logged in (an expired token still "is" a login).
pub fn credentials_mtime(kind: AgentKind, id: &str) -> Option<std::time::SystemTime> {
    let home = dirs::home_dir().unwrap_or_default();
    let base = dir(id).unwrap_or_else(|| home.join(if kind == AgentKind::Claude { ".claude" } else { ".codex" }));
    let f = base.join(if kind == AgentKind::Claude { ".credentials.json" } else { "auth.json" });
    std::fs::metadata(f).and_then(|m| m.modified()).ok()
}

/// Ask the provider for this account's limits right now, without waiting for an agent to report
/// them. Codex: its app-server's `account/rateLimits/read`. Claude: the endpoint behind Claude
/// Code's `/usage` (not publicly documented), with the login the CLI already has; the token is only
/// read, never refreshed, so an expired one just means "no data yet".
pub async fn fetch_usage(kind: AgentKind, id: &str, path: &str, codex: Option<&str>) -> Option<(BTreeMap<String, f64>, BTreeMap<String, u64>)> {
    let r = match kind {
        AgentKind::Claude => claude_usage(id).await,
        AgentKind::Codex => codex_usage(id, path, codex?).await,
    };
    if r.is_none() {
        tracing::warn!(account = id, "could not read usage from the provider");
    }
    r
}

async fn claude_usage(id: &str) -> Option<(BTreeMap<String, f64>, BTreeMap<String, u64>)> {
    let base = dir(id).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"));
    let raw = std::fs::read(base.join(".credentials.json")).ok()?;
    let creds: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let oauth = &creds["claudeAiOauth"];
    let token = oauth["accessToken"].as_str()?;
    if oauth["expiresAt"].as_u64().is_some_and(|t| t <= now_ms()) {
        return None;
    }
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| tracing::warn!(%e, "http client")).ok()?;
    let v: serde_json::Value = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|e| tracing::warn!(%e, "claude usage request"))
        .ok()?
        .error_for_status()
        .map_err(|e| tracing::warn!(%e, "claude usage status"))
        .ok()?
        .json()
        .await
        .ok()?;
    let (mut usage, mut resets) = (BTreeMap::new(), BTreeMap::new());
    for k in ["five_hour", "seven_day"] {
        if let Some(u) = v[k]["utilization"].as_f64() {
            usage.insert(k.to_string(), u / 100.0);
        }
        if let Some(t) = v[k]["resets_at"].as_str().and_then(parse_rfc3339) {
            resets.insert(k.to_string(), t);
        }
    }
    (!usage.is_empty()).then_some((usage, resets))
}

async fn codex_usage(id: &str, path: &str, bin: &str) -> Option<(BTreeMap<String, f64>, BTreeMap<String, u64>)> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut cmd = Command::new(bin);
    cmd.arg("app-server").env("PATH", path).current_dir(dirs::home_dir().unwrap_or_default());
    if let Some(d) = dir(id) {
        cmd.env(env_var(AgentKind::Codex), d);
    }
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let mut child = cmd.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    let mut lines = BufReader::new(child.stdout.take()?).lines();
    let ask = async {
        let init = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "clientInfo": { "name": "recursion", "version": env!("CARGO_PKG_VERSION") } } });
        stdin.write_all(format!("{init}\n").as_bytes()).await.ok()?;
        let read = serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read", "params": {} });
        stdin.write_all(format!("{read}\n").as_bytes()).await.ok()?;
        while let Ok(Some(line)) = lines.next_line().await {
            let m: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            if m["id"] == 2 {
                return Some(m["result"]["rateLimits"].clone());
            }
        }
        None
    };
    let rl = tokio::time::timeout(std::time::Duration::from_secs(20), ask).await.ok().flatten()?;
    let _ = child.kill().await;
    let (mut usage, mut resets) = (BTreeMap::new(), BTreeMap::new());
    for w in [&rl["primary"], &rl["secondary"]] {
        let Some(pct) = w["usedPercent"].as_f64() else { continue };
        let name = match w["windowDurationMins"].as_u64() {
            Some(300) => "five_hour",
            Some(10080) => "seven_day",
            _ => continue,
        };
        usage.insert(name.to_string(), pct / 100.0);
        if let Some(t) = w["resetsAt"].as_u64() {
            resets.insert(name.to_string(), t);
        }
    }
    (!usage.is_empty()).then_some((usage, resets))
}

/// "2026-11-21T07:10:00.61+00:00" → seconds since epoch.
fn parse_rfc3339(s: &str) -> Option<u64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>());
    let (y, mo, da) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let (clock, offset) = match rest.find(['+', '-', 'Z']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let mut c = clock.split(':');
    let (h, mi) = (c.next()?.parse::<i64>().ok()?, c.next()?.parse::<i64>().ok()?);
    let sec = c.next().and_then(|x| x.split('.').next()).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    let off = match offset.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let (oh, om) = offset[1..].split_once(':').unwrap_or((&offset[1..], "0"));
            let mins = oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?;
            if sign == '+' { mins * 60 } else { -mins * 60 }
        }
        _ => 0,
    };
    // Days from 1970-01-01 to y-mo-da (proleptic Gregorian).
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    u64::try_from(days * 86400 + h * 3600 + mi * 60 + sec - off).ok()
}

/// Email and plan from the Codex login's ID token (a JWT; only its payload is read, not verified).
fn codex_identity(id: &str) -> (Option<String>, Option<String>) {
    use base64::Engine;
    let base = dir(id).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"));
    let Ok(raw) = std::fs::read(base.join("auth.json")) else { return (None, None) };
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap_or_default();
    let Some(payload) = v["tokens"]["id_token"].as_str().and_then(|t| t.split('.').nth(1)) else { return (None, None) };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')) else { return (None, None) };
    let claims: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (
        claims["email"].as_str().map(String::from),
        claims["https://api.openai.com/auth"]["chatgpt_plan_type"].as_str().map(String::from),
    )
}

/// Does an agent's error mean its login is gone or expired?
pub fn is_auth_error(text: &str) -> bool {
    let t = text.to_lowercase();
    ["/login", "invalid api key", "oauth token", "not logged in", "authentication_error", "401 unauthorized", "token has expired", "refresh token", "please log in", "login required"]
        .iter()
        .any(|p| t.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_accounts_have_no_folder() {
        assert!(dir("claude-default").is_none());
        assert!(dir("../etc").is_none());
        assert!(dir("abc123").is_some());
        assert_eq!(env_prefix(AgentKind::Claude, "claude-default"), "");
        assert!(login_command(AgentKind::Codex, "abc123").starts_with("CODEX_HOME='"));
    }

    #[test]
    fn reset_times_parse() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-21T07:10:00.613843+00:00"), Some(1795245000));
        assert_eq!(parse_rfc3339("2026-11-21T15:10:00+08:00"), Some(1795245000));
    }

    #[test]
    fn auth_errors_are_recognised() {
        assert!(is_auth_error("Invalid API key · Please run /login"));
        assert!(is_auth_error("OAuth token has expired. Please obtain a new token"));
        assert!(!is_auth_error("Claude 意外結束（exit 1）"));
    }
}
