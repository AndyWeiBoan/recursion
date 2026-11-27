//! Several logins per CLI. Each extra account is a config folder (`~/.recursion/accounts/<id>/`)
//! handed to the CLI as `CLAUDE_CONFIG_DIR` / `CODEX_HOME`; only the login lives there, the rest
//! (settings, CLAUDE.md, skills, …) is linked back to `~/.claude` / `~/.codex`. The login the CLI
//! already had on this computer (its own folder, untouched) can be added too, as `<kind>-default`;
//! nothing is added on its own. An agent keeps the account it was created with: its conversation
//! is stored in that folder.

use crate::i18n::tr;
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
    /// Logged in, but the provider turned the login down since: it needs logging in again. The
    /// CLI can't tell (`auth status` only looks at the saved login), so this is what we saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expired: Option<Expired>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expired {
    /// When it was seen (ms since epoch).
    pub at: u64,
    /// "agent" (an agent was refused) or "usage" (the limits request was).
    pub why: String,
}

/// What asking the provider for an account's limits gave.
pub enum Fetched {
    Limits(BTreeMap<String, f64>, BTreeMap<String, u64>),
    /// The provider refused the login itself: it was logged out, revoked, or the like.
    Refused,
    /// Nothing to go by: no fresh token to ask with, the network, or an answer we don't know.
    Nothing,
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

/// `bin` is the CLI's full path when known: one installed a moment ago is not on the terminal's
/// PATH yet.
pub fn login_command(kind: AgentKind, id: &str, bin: Option<&str>) -> String {
    let p = env_prefix(kind, id);
    let bin = bin.map(|b| format!("'{}'", b.replace('\'', r"'\''"))).unwrap_or_else(|| if kind == AgentKind::Claude { "claude".into() } else { "codex".into() });
    match kind {
        AgentKind::Claude => format!("{p}{bin} auth login"),
        AgentKind::Codex => format!("{p}{bin} login"),
    }
}

#[derive(Default)]
pub struct Accounts {
    list: Mutex<Vec<Account>>,
}

impl Accounts {
    pub fn load() -> Self {
        let mut list: Vec<Account> = std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // Older versions put the CLI's own login in the list unasked, named 預設 ("default"): keep
        // it only if it was really logged in, under its email.
        list.retain(|a| !a.id.ends_with("-default") || a.logged_in || a.email.is_some());
        for a in list.iter_mut().filter(|a| a.name == "預設") {
            a.name = a.email.clone().unwrap_or_default();
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

    /// A new, empty login. Without a name it is named after its email once logged in.
    pub fn create(&self, kind: AgentKind, name: &str) -> Result<Account, String> {
        let name = name.trim();
        if !name.is_empty() && self.list.lock().unwrap().iter().any(|a| a.kind == kind && a.name == name) {
            return Err(tr!("acct.err.duplicate", name = name));
        }
        let id = crate::model::new_id();
        let d = dir(&id).ok_or("invalid account id")?;
        std::fs::create_dir_all(&d).map_err(|e| tr!("acct.err.mkdir", error = e))?;
        let (src, items) = shared(kind);
        for item in items {
            let from = src.join(item);
            if from.exists() {
                #[cfg(unix)]
                let _ = std::os::unix::fs::symlink(&from, d.join(item));
            }
        }
        let a = Account { id, kind, name: name.into(), email: None, plan: None, logged_in: false, usage: Default::default(), usage_at: 0, resets: Default::default(), expired: None };
        self.list.lock().unwrap().push(a.clone());
        self.save();
        Ok(a)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(tr!("acct.err.emptyName"));
        }
        let mut list = self.list.lock().unwrap();
        let a = list.iter_mut().find(|a| a.id == id).ok_or_else(|| tr!("err.noAccount"))?;
        a.name = name.into();
        drop(list);
        self.save();
        Ok(())
    }

    /// Add the login the CLI already has on this computer (its own folder).
    pub fn adopt(&self, kind: AgentKind) -> Account {
        let id = default_id(kind);
        if let Some(a) = self.get(id) {
            return a;
        }
        let a = Account { id: id.into(), kind, name: String::new(), email: None, plan: None, logged_in: false, usage: Default::default(), usage_at: 0, resets: Default::default(), expired: None };
        self.list.lock().unwrap().push(a.clone());
        self.save();
        a
    }

    /// Take an account out of Recursion. An added CLI login stays logged in on the computer; an
    /// account made here loses its folder (and the login in it).
    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.get(id).ok_or_else(|| tr!("err.noAccount"))?;
        self.list.lock().unwrap().retain(|a| a.id != id);
        self.save();
        if let Some(d) = dir(id) {
            let _ = std::fs::remove_dir_all(d);
        }
        Ok(())
    }

    /// Latest limits seen from an agent on this account. Returns whether anything changed. Limits
    /// only come with a working login, so it is not expired after all.
    pub fn record_usage(&self, id: &str, usage: &BTreeMap<String, f64>, resets: &BTreeMap<String, u64>) -> bool {
        let cleared = self.clear_expired(id);
        let mut list = self.list.lock().unwrap();
        let Some(a) = list.iter_mut().find(|a| a.id == id) else { return cleared };
        if &a.usage == usage && (resets.is_empty() || &a.resets == resets) {
            return cleared;
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

    /// The provider refused this account's login. Returns whether that is news.
    pub fn mark_expired(&self, id: &str, why: &str) -> bool {
        let mut list = self.list.lock().unwrap();
        let Some(a) = list.iter_mut().find(|a| a.id == id && a.logged_in && a.expired.is_none()) else { return false };
        a.expired = Some(Expired { at: now_ms(), why: why.into() });
        drop(list);
        self.save();
        true
    }

    /// It works again. Returns whether it was marked.
    pub fn clear_expired(&self, id: &str) -> bool {
        let mut list = self.list.lock().unwrap();
        let Some(a) = list.iter_mut().find(|a| a.id == id && a.expired.is_some()) else { return false };
        a.expired = None;
        drop(list);
        self.save();
        true
    }

    /// Is the account marked as needing a new login? A login saved since it was marked (in a
    /// terminal, or the CLI renewing it) clears the mark. Returns (expired, mark cleared just now).
    pub fn check_expired(&self, id: &str) -> (bool, bool) {
        let Some(a) = self.get(id) else { return (false, false) };
        let Some(e) = a.expired else { return (false, false) };
        let since = credentials_mtime(a.kind, id).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).is_some_and(|t| t.as_millis() as u64 > e.at);
        if since {
            return (false, self.clear_expired(id));
        }
        (true, false)
    }

    /// Ask the CLI who is logged in on this account. Returns whether anything changed.
    pub async fn refresh(&self, id: &str, path: &str, claude: Option<&str>, codex: Option<&str>) -> bool {
        let Some(a) = self.get(id) else { return false };
        let Some((logged_in, email, plan)) = status(a.kind, id, path, claude, codex).await else { return false };
        let mut list = self.list.lock().unwrap();
        let Some(a) = list.iter_mut().find(|a| a.id == id) else { return false };
        let mut changed = a.logged_in != logged_in || a.email != email || a.plan != plan;
        a.logged_in = logged_in;
        if logged_in {
            a.email = email;
            a.plan = plan;
            if a.name.is_empty() {
                if let Some(e) = &a.email {
                    a.name = e.clone();
                    changed = true;
                }
            }
        }
        drop(list);
        if changed {
            self.save();
        }
        changed
    }
}

/// Who is logged in on an account's folder (`<kind>-default`: the CLI's own): (logged in, email,
/// plan), or `None` when the CLI is missing or did not answer.
pub async fn status(kind: AgentKind, id: &str, path: &str, claude: Option<&str>, codex: Option<&str>) -> Option<(bool, Option<String>, Option<String>)> {
    let mut cmd = match kind {
        AgentKind::Claude => {
            let mut c = Command::new(claude?);
            c.args(["auth", "status"]);
            c
        }
        AgentKind::Codex => {
            let mut c = Command::new(codex?);
            c.args(["login", "status"]);
            c
        }
    };
    cmd.env("PATH", path).stdin(std::process::Stdio::null());
    if let Some(d) = dir(id) {
        cmd.env(env_var(kind), d);
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output()).await.ok()?.ok()?;
    Some(match kind {
        AgentKind::Claude => {
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
            (v["loggedIn"].as_bool().unwrap_or(false), v["email"].as_str().map(String::from), v["subscriptionType"].as_str().map(String::from))
        }
        AgentKind::Codex => {
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            let ok = out.status.success() && !text.to_lowercase().contains("not logged in");
            let (email, plan) = codex_identity(id);
            (ok, email, plan)
        }
    })
}

/// The official installer, run in a terminal: puts the CLI in ~/.local/bin, no root needed.
pub fn install_command(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "curl -fsSL https://claude.ai/install.sh | bash",
        AgentKind::Codex => "curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh",
    }
}

/// When the account's login file last changed: a fresh login rewrites it, even if the CLI
/// already counted the account as logged in (an expired token still "is" a login).
pub fn credentials_mtime(kind: AgentKind, id: &str) -> Option<std::time::SystemTime> {
    #[cfg(target_os = "macos")]
    if kind == AgentKind::Claude {
        return keychain::modified(&keychain::claude_service(id));
    }
    let home = dirs::home_dir().unwrap_or_default();
    let base = dir(id).unwrap_or_else(|| home.join(if kind == AgentKind::Claude { ".claude" } else { ".codex" }));
    let f = base.join(if kind == AgentKind::Claude { ".credentials.json" } else { "auth.json" });
    std::fs::metadata(f).and_then(|m| m.modified()).ok()
}

/// Ask the provider for this account's limits right now, without waiting for an agent to report
/// them. Codex: its app-server's `account/rateLimits/read`. Claude: the endpoint behind Claude
/// Code's `/usage` (not publicly documented), with the login the CLI already has; the token is only
/// read, never refreshed, so an expired one just means "no data yet".
pub async fn fetch_usage(kind: AgentKind, id: &str, path: &str, codex: Option<&str>) -> Fetched {
    let r = match (kind, codex) {
        (AgentKind::Claude, _) => claude_usage(id).await,
        (AgentKind::Codex, Some(bin)) => codex_usage(id, path, bin).await,
        (AgentKind::Codex, None) => Fetched::Nothing,
    };
    match &r {
        Fetched::Nothing => tracing::warn!(account = id, "could not read usage from the provider"),
        Fetched::Refused => tracing::warn!(account = id, "the provider refused the login"),
        Fetched::Limits(..) => {}
    }
    r
}

/// The login Claude Code saved for this account: a file on Linux, the login keychain on macOS.
fn claude_credentials(id: &str) -> Option<serde_json::Value> {
    let base = dir(id).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"));
    if let Some(v) = std::fs::read(base.join(".credentials.json")).ok().and_then(|raw| serde_json::from_slice(&raw).ok()) {
        return Some(v);
    }
    #[cfg(target_os = "macos")]
    if let Some(v) = keychain::secret(&keychain::claude_service(id)).and_then(|raw| serde_json::from_str(&raw).ok()) {
        return Some(v);
    }
    None
}

/// macOS keeps Claude Code's login in the login keychain instead of `.credentials.json`. Read
/// through `security`, as Claude Code itself does, so no keychain prompt appears.
#[cfg(target_os = "macos")]
mod keychain {
    use std::process::Command;

    /// "Claude Code-credentials"; an account with its own config folder adds the first 8 hex
    /// digits of that folder's SHA-256.
    pub fn claude_service(id: &str) -> String {
        let Some(dir) = super::dir(id) else { return "Claude Code-credentials".into() };
        let out = Command::new("/bin/sh").args(["-c", "printf %s \"$1\" | /usr/bin/shasum -a 256", "sh"]).arg(dir).output().ok();
        let hash = out.map(|o| String::from_utf8_lossy(&o.stdout).chars().take(8).collect::<String>()).unwrap_or_default();
        format!("Claude Code-credentials-{hash}")
    }

    fn user() -> String {
        std::env::var("USER").unwrap_or_default()
    }

    pub fn secret(service: &str) -> Option<String> {
        let out = Command::new("/usr/bin/security").args(["find-generic-password", "-a", &user(), "-s", service, "-w"]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
    }

    /// When the item last changed (its `mdat`), without reading the secret.
    pub fn modified(service: &str) -> Option<std::time::SystemTime> {
        let out = Command::new("/usr/bin/security").args(["find-generic-password", "-a", &user(), "-s", service]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        // "mdat"<timedate>=0x… "20261122041722Z\000"
        let line = text.lines().find(|l| l.contains("\"mdat\""))?;
        let stamp = line.split('"').nth(3)?.get(..14)?;
        let t = super::parse_rfc3339(&format!("{}-{}-{}T{}:{}:{}Z", &stamp[0..4], &stamp[4..6], &stamp[6..8], &stamp[8..10], &stamp[10..12], &stamp[12..14]))?;
        Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(t))
    }
}

async fn claude_usage(id: &str) -> Fetched {
    let Some(creds) = claude_credentials(id) else { return Fetched::Nothing };
    let oauth = &creds["claudeAiOauth"];
    let Some(token) = oauth["accessToken"].as_str() else { return Fetched::Nothing };
    // An expired access token is normal (the CLI renews it when next used): it says nothing.
    if oauth["expiresAt"].as_u64().is_some_and(|t| t <= now_ms()) {
        return Fetched::Nothing;
    }
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build() else { return Fetched::Nothing };
    let res = match client.get("https://api.anthropic.com/api/oauth/usage").bearer_auth(token).header("anthropic-beta", "oauth-2025-04-20").send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(%e, "claude usage request");
            return Fetched::Nothing;
        }
    };
    let status = res.status();
    let Ok(body) = res.text().await else { return Fetched::Nothing };
    if !status.is_success() {
        tracing::warn!(%status, body = %body.chars().take(300).collect::<String>(), "claude usage status");
        // Only the API saying the login itself is bad counts: a still-valid token turned down.
        return if status == reqwest::StatusCode::UNAUTHORIZED && body.contains("authentication_error") { Fetched::Refused } else { Fetched::Nothing };
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else { return Fetched::Nothing };
    let (mut usage, mut resets) = (BTreeMap::new(), BTreeMap::new());
    for k in ["five_hour", "seven_day"] {
        if let Some(u) = v[k]["utilization"].as_f64() {
            usage.insert(k.to_string(), u / 100.0);
        }
        if let Some(t) = v[k]["resets_at"].as_str().and_then(parse_rfc3339) {
            resets.insert(k.to_string(), t);
        }
    }
    if usage.is_empty() { Fetched::Nothing } else { Fetched::Limits(usage, resets) }
}

async fn codex_usage(id: &str, path: &str, bin: &str) -> Fetched {
    match codex_limits(id, path, bin).await {
        Some(Ok(l)) => l,
        Some(Err(text)) if is_auth_error(&text) => Fetched::Refused,
        Some(Err(text)) => {
            tracing::warn!(error = %text, "codex rateLimits/read");
            Fetched::Nothing
        }
        None => Fetched::Nothing,
    }
}

/// The limits, or the error the app-server answered with.
async fn codex_limits(id: &str, path: &str, bin: &str) -> Option<Result<Fetched, String>> {
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
                if !m["error"].is_null() {
                    return Some(Err(m["error"]["message"].as_str().map(String::from).unwrap_or_else(|| m["error"].to_string())));
                }
                return Some(Ok(m["result"]["rateLimits"].clone()));
            }
        }
        None
    };
    let rl = tokio::time::timeout(std::time::Duration::from_secs(20), ask).await.ok().flatten();
    let _ = child.kill().await;
    let rl = match rl? {
        Ok(rl) => rl,
        Err(e) => return Some(Err(e)),
    };
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
    Some(Ok(if usage.is_empty() { Fetched::Nothing } else { Fetched::Limits(usage, resets) }))
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
    ["/login", "invalid api key", "oauth token", "not logged in", "authentication_error", "401 unauthorized", "token has expired", "refresh token", "please log in", "login required", "failed to authenticate", "could not be refreshed"]
        .iter()
        .any(|p| t.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_errors_are_recognised() {
        // What Claude Code says when its login was revoked (seen on 2.1.x).
        assert!(is_auth_error("Failed to authenticate: OAuth session expired and could not be refreshed"));
        assert!(is_auth_error("API Error: 401 Unauthorized"));
        // A network problem is not a login problem.
        assert!(!is_auth_error("API Error: Connection error."));
        assert!(!is_auth_error("request timed out"));
    }

    #[test]
    fn default_accounts_have_no_folder() {
        assert!(dir("claude-default").is_none());
        assert!(dir("../etc").is_none());
        assert!(dir("abc123").is_some());
        assert_eq!(env_prefix(AgentKind::Claude, "claude-default"), "");
        assert!(login_command(AgentKind::Codex, "abc123", None).starts_with("CODEX_HOME='"));
        assert!(login_command(AgentKind::Codex, "abc123", Some("/a b/codex")).ends_with("'/a b/codex' login"));
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
