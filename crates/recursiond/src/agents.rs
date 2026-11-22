//! Agent processes and the workspace hub (transcripts + routing to agents).
//!
//! Claude is driven through `claude -p --input-format stream-json --output-format stream-json`
//! with `--permission-prompt-tool stdio`, so permission prompts arrive as `control_request`
//! messages on stdout and are answered with `control_response` on stdin.

use crate::i18n::tr;
use crate::workspace::{display_tool_name, mentions, now_ms, summarize_tool, tool_result_text, AgentKind, Body, Member, MemberStatus, Message, PermMode, PermState, TurnUsage, Workspace};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::broadcast;

mod codex;
mod models;
mod polls;
mod threads;
pub use polls::tools as poll_tools;

pub use models::ModelOption;

/// PATH and binaries resolved from the user's login shell (GUI apps start with a bare PATH).
#[derive(Clone, Debug, Default)]
pub struct AgentEnv {
    pub path: String,
    pub claude: Option<String>,
    pub codex: Option<String>,
}

impl AgentEnv {
    pub async fn detect() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let out = Command::new(&shell)
            .args(["-l", "-c", "printf '%s\\n' \"$PATH\"; printf 'claude=%s\\n' \"$(command -v claude)\"; printf 'codex=%s\\n' \"$(command -v codex)\""])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut path = lines.first().filter(|l| !l.starts_with("claude=") && !l.starts_with("codex=")).map(|s| s.to_string()).unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
        let find = |k: &str| lines.iter().find_map(|l| l.strip_prefix(k)).map(|s| s.trim().to_string()).filter(|s| s.starts_with('/'));
        let (mut claude, mut codex) = (find("claude="), find("codex="));
        // A login shell skips ~/.zshrc, which is where Claude's installer (and nvm and friends) put
        // their PATH line on macOS: look in the usual install folders too.
        for (bin, slot) in [("claude", &mut claude), ("codex", &mut codex)] {
            if slot.is_none() {
                if let Some(found) = Self::known_dirs().into_iter().map(|d| d.join(bin)).find(|p| p.is_file()) {
                    let dir = found.parent().unwrap().to_string_lossy().to_string();
                    if !path.split(':').any(|p| p == dir) {
                        path = format!("{dir}:{path}");
                    }
                    *slot = Some(found.to_string_lossy().to_string());
                }
            }
        }
        AgentEnv { path, claude, codex }
    }

    fn known_dirs() -> Vec<std::path::PathBuf> {
        let home = dirs::home_dir().unwrap_or_default();
        let mut v: Vec<std::path::PathBuf> = [".local/bin", ".claude/local", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin"].iter().map(|d| home.join(d)).collect();
        v.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(std::path::PathBuf::from));
        v
    }
}

struct Proc {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    child: Arc<tokio::sync::Mutex<Child>>,
    /// Bumped on every spawn so a dying old reader can't clobber a new process.
    generation: u64,
    /// JSON-RPC state when the agent is Codex (app-server).
    codex: Option<Arc<codex::Rpc>>,
}

struct PendingPerm {
    panel: String,
    member: String,
    msg_id: String,
    input: Value,
    suggestions: Value,
    /// Codex: JSON-RPC id of the server request to answer.
    codex_request: Option<Value>,
}

/// How much history a newly joined agent gets.
const HISTORY_MSGS: usize = 60;
const HISTORY_CHARS: usize = 30_000;
/// Consecutive human messages within this window form one round.
const MERGE_WINDOW_MS: u64 = 1500;

/// Per-workspace round state (not persisted).
///
/// Turn-based chat: a human message opens a round; every addressed agent answers once, in
/// parallel; then everyone waits for the human again. Human messages sent while a round is
/// running are queued for the next round.
#[derive(Default)]
struct Runtime {
    /// Members that still owe a reply in the current round.
    speaking: HashSet<String>,
    /// Targets accumulated from human messages not yet delivered (None inside = everyone).
    queued_targets: Option<HashSet<String>>,
    queued_all: bool,
    /// Human messages waiting for the next round.
    queued_count: u32,
    /// Bumped on every human message; a delayed round start only fires if still current.
    send_gen: u64,
    /// Members that must be re-delivered when their in-flight turn ends (round started while busy).
    pending: HashSet<String>,
    /// Project folder of this workspace (set on every human message).
    cwd: String,
    roster_version: u64,
    roster_seen: HashMap<String, u64>,
    /// member → id of the first message of its turn in progress.
    turn_first: HashMap<String, String>,
    /// Members compacting their conversation → the chat's folder, to deliver what waited meanwhile.
    compacting: HashMap<String, String>,
    /// Compacting members a round skipped: they get their own round once the compact is done.
    missed: HashSet<String>,
}

pub struct Hub {
    runtime: Mutex<HashMap<String, Runtime>>,
    workspaces: Mutex<HashMap<String, Workspace>>,
    procs: Mutex<HashMap<(String, String), Proc>>,
    perms: Mutex<HashMap<String, PendingPerm>>,
    dirty: Mutex<HashSet<String>>,
    pub save_notify: tokio::sync::Notify,
    pub tx: broadcast::Sender<String>,
    env: tokio::sync::RwLock<Option<AgentEnv>>,
    /// Model lists per CLI, cached for a while (asking spawns the CLI).
    models: tokio::sync::Mutex<HashMap<&'static str, (std::time::Instant, Vec<ModelOption>)>>,
    generation: std::sync::atomic::AtomicU64,
    /// Daemon port, for the MCP URL handed to agents.
    port: std::sync::atomic::AtomicU16,
    /// MCP key → (panel, member): each agent gets its own URL, so tool calls know who is calling.
    mcp: Mutex<HashMap<String, (String, String)>>,
    pub accounts: crate::accounts::Accounts,
}

fn mid() -> String {
    crate::model::new_id()
}

fn msg(from: &str, body: Body, streaming: bool) -> Message {
    Message { id: mid(), ts: now_ms(), from: from.into(), seq: 0, to: vec![], streaming, turn: false, via: None, from_thread: None, reply_to: None, files: vec![], usage: None, body }
}

impl Hub {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(8192);
        Arc::new(Hub {
            runtime: Mutex::new(HashMap::new()),
            workspaces: Mutex::new(HashMap::new()),
            procs: Mutex::new(HashMap::new()),
            perms: Mutex::new(HashMap::new()),
            dirty: Mutex::new(HashSet::new()),
            save_notify: tokio::sync::Notify::new(),
            tx,
            env: tokio::sync::RwLock::new(None),
            models: Default::default(),
            generation: Default::default(),
            port: Default::default(),
            mcp: Default::default(),
            accounts: crate::accounts::Accounts::load(),
        })
    }

    pub fn emit_accounts(&self) {
        self.emit(json!({ "t": "accounts", "accounts": self.accounts.list() }));
    }

    /// Re-ask the CLIs who is logged in on every account (on start, and after a login).
    pub async fn refresh_accounts(&self, only: Option<&str>) -> bool {
        let env = self.env().await;
        let mut changed = false;
        for a in self.accounts.list().into_iter().filter(|a| only.is_none_or(|o| o == a.id)) {
            changed |= self.accounts.refresh(&a.id, &env.path, env.claude.as_deref(), env.codex.as_deref()).await;
        }
        if changed {
            self.emit_accounts();
        }
        changed
    }

    /// Ask the providers for every logged-in account's limits (see accounts::fetch_usage).
    pub async fn poll_usage(&self) {
        let env = self.env().await;
        let mut changed = false;
        for a in self.accounts.list().into_iter().filter(|a| a.logged_in) {
            if let Some((usage, resets)) = crate::accounts::fetch_usage(a.kind, &a.id, &env.path, env.codex.as_deref()).await {
                changed |= self.accounts.record_usage(&a.id, &usage, &resets);
            }
        }
        if changed {
            self.emit_accounts();
        }
    }

    /// After a login: the "log in again" cards for this account are done.
    pub fn resolve_relogin(&self, account: &str) {
        let panels: Vec<String> = self.workspaces.lock().unwrap().keys().cloned().collect();
        for p in panels {
            let ids: Vec<String> = self.with_ws(&p, |ws| {
                ws.messages.iter().filter(|m| matches!(&m.body, Body::Relogin { account: a, resolved: false, .. } if a == account)).map(|m| m.id.clone()).collect()
            });
            for id in ids {
                self.update_msg(&p, &id, |m| {
                    if let Body::Relogin { resolved, .. } = &mut m.body {
                        *resolved = true;
                    }
                });
            }
        }
    }

    /// Is any agent (in a chat that has been opened) using this account?
    pub fn account_in_use(&self, account: &str, chats: &[String]) -> Option<String> {
        chats.iter().find_map(|p| self.with_ws(p, |ws| ws.members.iter().find(|m| m.account.as_deref() == Some(account)).map(|m| format!("@{}", m.alias))))
    }

    pub async fn models(&self, kind: AgentKind) -> Result<Vec<ModelOption>, String> {
        let key = match kind {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
        };
        // Held across the fetch so opening two pickers at once spawns the CLI only once.
        let mut cache = self.models.lock().await;
        if let Some((at, list)) = cache.get(key) {
            if at.elapsed() < std::time::Duration::from_secs(600) {
                return Ok(list.clone());
            }
        }
        let list = models::list(&self.env().await, kind).await?;
        cache.insert(key, (std::time::Instant::now(), list.clone()));
        Ok(list)
    }

    /// Members of every given chat panel (loads transcripts that aren't in memory yet).
    pub fn roster(&self, panels: &[String]) -> Value {
        let map: serde_json::Map<String, Value> = panels.iter().map(|p| (p.clone(), self.with_ws(p, |ws| json!(ws.members)))).collect();
        json!({ "t": "ws_roster", "panels": map })
    }

    /// Close an agent's turn: record what it cost on the turn's first message.
    fn finish_turn(&self, panel: &str, member: &str, usage: Option<TurnUsage>) {
        let first = self.rt(panel, |r| r.turn_first.remove(member));
        if let (Some(id), Some(u)) = (first, usage) {
            self.update_msg(panel, &id, |m| m.usage = Some(u));
        }
    }

    pub fn set_port(&self, port: u16) {
        self.port.store(port, std::sync::atomic::Ordering::Relaxed);
    }

    /// The built-in MCP server's URL for one agent (stable for the daemon's lifetime).
    fn mcp_url(&self, panel: &str, member: &str) -> String {
        let mut map = self.mcp.lock().unwrap();
        let key = match map.iter().find(|(_, v)| v.0 == panel && v.1 == member) {
            Some((k, _)) => k.clone(),
            None => {
                use rand::Rng;
                let k: String = rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(32).map(char::from).collect();
                map.insert(k.clone(), (panel.to_string(), member.to_string()));
                k
            }
        };
        format!("http://127.0.0.1:{}/mcp/{key}", self.port.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn alias_of(&self, panel: &str, member: &str) -> Option<String> {
        self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == member).map(|m| m.alias.clone()))
    }

    /// Who an MCP key belongs to: (panel, member id, alias).
    pub fn mcp_caller(&self, key: &str) -> Option<(String, String, String)> {
        let (panel, member) = self.mcp.lock().unwrap().get(key).cloned()?;
        let alias = self.with_ws(&panel, |ws| ws.members.iter().find(|m| m.id == member).map(|m| m.alias.clone()))?;
        Some((panel, member, alias))
    }

    /// The card in the chat for a document an agent just wrote.
    /// Notes something an agent made in the chat's 產出 list.
    pub fn record_artifact(&self, panel: &str, kind: crate::workspace::ArtifactKind, target: &str, title: &str, by: &str) {
        let a = self.with_ws(panel, |ws| ws.record_artifact(kind, target, title, by, crate::workspace::now_ms()));
        self.emit(json!({ "t": "ws_artifact", "panel": panel, "artifact": a }));
        self.mark_dirty(panel);
    }

    /// Lists documents this chat made that its 產出 list doesn't have yet.
    pub fn adopt_docs(&self, panel: &str, docs: &[crate::model::Doc]) {
        let added = self.with_ws(panel, |ws| {
            let mut n = 0;
            for d in docs {
                if !ws.artifacts.iter().any(|a| a.kind == crate::workspace::ArtifactKind::Doc && a.target == d.id) {
                    let kind = if d.kind == crate::model::DocKind::Diagram { crate::workspace::ArtifactKind::Diagram } else { crate::workspace::ArtifactKind::Doc };
                    ws.record_artifact(kind, &d.id, &d.title, &d.member, d.updated);
                    n += 1;
                }
            }
            n
        });
        if added > 0 {
            self.mark_dirty(panel);
        }
    }

    /// Takes a thread, document, page or poll off the 產出 list. The chat keeps showing it.
    pub fn output_remove(&self, panel: &str, id: &str) -> Result<(), String> {
        let ev = self.with_ws(panel, |ws| {
            if let Some(t) = ws.threads.iter_mut().find(|t| t.id == id) {
                t.removed = true;
                return Some(json!({ "t": "ws_thread", "panel": panel, "thread": t }));
            }
            if let Some(a) = ws.artifacts.iter_mut().find(|a| a.id == id) {
                a.removed = true;
                return Some(json!({ "t": "ws_artifact", "panel": panel, "artifact": a }));
            }
            let p = ws.polls.iter_mut().find(|p| p.id == id)?;
            p.removed = true;
            Some(json!({ "t": "ws_poll", "panel": panel, "poll": p }))
        });
        let ev = ev.ok_or_else(|| tr!("err.noOutput"))?;
        self.emit(ev);
        self.mark_dirty(panel);
        Ok(())
    }

    pub fn artifact(&self, panel: &str, id: &str) -> Option<crate::workspace::Artifact> {
        self.with_ws(panel, |ws| ws.artifacts.iter().find(|a| a.id == id).cloned())
    }

    pub fn post_doc(&self, panel: &str, member: &str, doc: &crate::model::Doc, updated: bool) {
        self.push_msg(panel, msg(member, Body::Doc { doc: doc.id.clone(), title: doc.title.clone(), updated }, false));
    }

    async fn env(&self) -> AgentEnv {
        if let Some(e) = self.env.read().await.clone() {
            if e.claude.is_some() || e.codex.is_some() {
                return e;
            }
        }
        let e = AgentEnv::detect().await;
        *self.env.write().await = Some(e.clone());
        e
    }

    fn emit(&self, v: Value) {
        let _ = self.tx.send(v.to_string());
    }

    fn with_ws<R>(&self, panel: &str, f: impl FnOnce(&mut Workspace) -> R) -> R {
        let mut map = self.workspaces.lock().unwrap();
        let ws = map.entry(panel.to_string()).or_insert_with(|| Workspace::load(panel));
        f(ws)
    }

    fn mark_dirty(&self, panel: &str) {
        self.dirty.lock().unwrap().insert(panel.to_string());
        self.save_notify.notify_one();
    }

    pub fn flush(&self) {
        let dirty: Vec<String> = self.dirty.lock().unwrap().drain().collect();
        let map = self.workspaces.lock().unwrap();
        for p in dirty {
            if let Some(ws) = map.get(&p) {
                ws.save(&p);
            }
        }
    }

    pub fn snapshot(&self, panel: &str) -> Value {
        let round = self.round_state(panel);
        self.with_ws(panel, |ws| json!({ "t": "ws_state", "panel": panel, "ws": ws, "round": { "active": round["active"], "queued": round["queued"] } }))
    }

    fn push_msg(&self, panel: &str, mut msg: Message) -> Message {
        // The first thing an agent says in a turn opens it (the UI starts a new header there).
        let speaking = msg.from != "you" && !matches!(msg.body, Body::System { .. }) && self.with_ws(panel, |ws| ws.members.iter().any(|m| m.id == msg.from && m.status != MemberStatus::Idle));
        if speaking {
            msg.turn = self.rt(panel, |r| !r.turn_first.contains_key(&msg.from));
            if msg.turn {
                self.rt(panel, |r| r.turn_first.insert(msg.from.clone(), msg.id.clone()));
            }
        }
        let m = self.with_ws(panel, |ws| ws.push(msg));
        self.emit(json!({ "t": "ws_msg", "panel": panel, "msg": &m }));
        self.mark_dirty(panel);
        m
    }

    fn remove_msgs(&self, panel: &str, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        self.with_ws(panel, |ws| ws.messages.retain(|m| !ids.contains(&m.id)));
        for id in ids {
            self.emit(json!({ "t": "ws_msg_removed", "panel": panel, "id": id }));
        }
        self.mark_dirty(panel);
    }

    fn rt<R>(&self, panel: &str, f: impl FnOnce(&mut Runtime) -> R) -> R {
        let mut map = self.runtime.lock().unwrap();
        f(map.entry(panel.to_string()).or_default())
    }

    fn update_msg(&self, panel: &str, id: &str, f: impl FnOnce(&mut Message)) {
        let msg = self.with_ws(panel, |ws| ws.message_mut(id).map(|m| {
            f(m);
            m.clone()
        }));
        if let Some(m) = msg {
            self.emit(json!({ "t": "ws_msg", "panel": panel, "msg": m }));
            self.mark_dirty(panel);
        }
    }

    fn update_member(&self, panel: &str, member: &str, f: impl FnOnce(&mut Member)) {
        let m = self.with_ws(panel, |ws| ws.member_mut(member).map(|m| {
            f(m);
            m.clone()
        }));
        if let Some(m) = m {
            self.emit(json!({ "t": "ws_member", "panel": panel, "member": m }));
            self.mark_dirty(panel);
        }
    }

    fn system(&self, panel: &str, from: &str, text: impl Into<String>, error: bool) {
        let text = text.into();
        // A login problem gets a card with a way out instead of a bare error.
        if error && crate::accounts::is_auth_error(&text) {
            let m = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == from).cloned());
            if let Some(m) = m {
                let account = self.accounts.resolve(m.kind, m.account.as_deref());
                let name = self.accounts.get(&account).map(|a| a.name).unwrap_or_default();
                self.push_msg(panel, msg(from, Body::Relogin { account, name, text, resolved: false }, false));
                return;
            }
        }
        let body = if error { Body::Error { text } } else { Body::System { text } };
        self.push_msg(panel, msg(from, body, false));
    }

    // ---------- commands from clients ----------

    pub async fn add_agent(self: &Arc<Self>, panel: &str, kind: AgentKind, alias: String, model: Option<String>, account: Option<String>, mode: PermMode) -> Result<(), String> {
        let account = self.accounts.resolve(kind, account.as_deref());
        let acc = self.accounts.get(&account).ok_or_else(|| tr!("err.noAccount"))?;
        let env = self.env().await;
        match kind {
            AgentKind::Claude if env.claude.is_none() => {
                return Err(tr!("agent.err.noClaude"));
            }
            AgentKind::Codex => {
                let Some(bin) = env.codex.clone() else {
                    return Err(tr!("agent.err.noCodex"));
                };
                let mut cmd = Command::new(bin);
                cmd.args(["login", "status"]).env("PATH", &env.path);
                self.accounts.apply(&mut cmd, kind, Some(&account));
                let out = cmd.output().await.map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                if !out.status.success() || text.to_lowercase().contains("not logged in") {
                    return Err(tr!("agent.err.codexLoggedOut", name = acc.name));
                }
            }
            _ => {}
        }
        let alias = self.check_alias(panel, &alias, None)?;
        let account = (account != crate::accounts::default_id(kind)).then_some(account);
        let member = Member { id: mid(), kind, alias: alias.clone(), model: model.filter(|m| !m.is_empty()), account, cli_session_id: None, cost_usd: 0.0, status: MemberStatus::Idle, usage: Default::default(), context_used: 0, context_window: 0, seen: 0, mode };
        self.with_ws(panel, |ws| ws.members.push(member.clone()));
        self.rt(panel, |r| r.roster_version += 1);
        self.emit(json!({ "t": "ws_member", "panel": panel, "member": member }));
        let text = if mode == PermMode::default() { tr!("agent.joined", alias = alias) } else { tr!("agent.joinedMode", alias = alias, mode = mode.label()) };
        self.system(panel, &member.id, text, false);
        Ok(())
    }

    /// Lets this chat use files the human picked from their computer. One that is in the project
    /// after all comes back as its project path (nothing to grant); the others are granted and
    /// come back as canonical absolute paths. Missing files are left out.
    pub fn grant(&self, panel: &str, cwd: &str, paths: &[String]) -> Vec<String> {
        let root = std::fs::canonicalize(cwd).ok();
        let mut out = Vec::new();
        let mut granted = Vec::new();
        for p in paths.iter().filter_map(|p| std::fs::canonicalize(p).ok()).filter(|p| p.is_file()) {
            match root.as_ref().and_then(|r| p.strip_prefix(r).ok()) {
                Some(rel) => out.push(rel.to_string_lossy().to_string()),
                None => {
                    let s = p.to_string_lossy().to_string();
                    granted.push(s.clone());
                    out.push(s);
                }
            }
        }
        if !granted.is_empty() {
            self.with_ws(panel, |ws| {
                for p in granted {
                    if !ws.grants.contains(&p) {
                        ws.grants.push(p);
                    }
                }
            });
            self.mark_dirty(panel);
        }
        out
    }

    /// A file this chat may use: inside the project, or one the human granted.
    pub fn usable_file(&self, panel: &str, cwd: &str, path: &str) -> Option<std::path::PathBuf> {
        if std::path::Path::new(path).is_absolute() {
            let p = std::fs::canonicalize(path).ok()?;
            if std::fs::canonicalize(cwd).is_ok_and(|root| p.starts_with(root)) {
                return Some(p);
            }
            let s = p.to_string_lossy().to_string();
            return self.with_ws(panel, |ws| ws.grants.contains(&s)).then_some(p);
        }
        crate::files::resolve(cwd, path)
    }

    pub fn is_compacting(&self, panel: &str, member: &str) -> bool {
        self.rt(panel, |r| r.compacting.contains_key(member))
    }

    /// Has an idle agent summarise its own conversation (Claude's `/compact`, Codex's
    /// `thread/compact/start`) to free context. Rounds meanwhile go on without it; what it missed
    /// reaches it afterwards in a round of its own.
    pub async fn compact(self: &Arc<Self>, panel: &str, member: &str, cwd: String) -> Result<(), String> {
        let m = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == member).cloned()).ok_or_else(|| tr!("err.noAgent"))?;
        if m.status != MemberStatus::Idle {
            return Err(tr!("agent.err.busyCompact", alias = m.alias));
        }
        if m.cli_session_id.is_none() {
            return Err(tr!("agent.err.nothingToCompact", alias = m.alias));
        }
        let running = self.procs.lock().unwrap().contains_key(&(panel.to_string(), m.id.clone()));
        if !running {
            match m.kind {
                AgentKind::Claude => self.spawn(panel, &m, &cwd).await?,
                AgentKind::Codex => self.spawn_codex(panel, &m, &cwd).await?,
            }
        }
        self.rt(panel, |r| r.compacting.insert(m.id.clone(), cwd));
        self.update_member(panel, &m.id, |x| x.status = MemberStatus::Compacting);
        let sent = match m.kind {
            AgentKind::Claude => self.write(panel, &m.id, json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": "/compact" }] } })).await,
            AgentKind::Codex => {
                let rpc = self.procs.lock().unwrap().get(&(panel.to_string(), m.id.clone())).and_then(|p| p.codex.clone()).ok_or("Codex is not running")?;
                rpc.compact(self, panel, &m.id).await
            }
        };
        if let Err(e) = sent {
            self.rt(panel, |r| r.compacting.remove(&m.id));
            self.update_member(panel, &m.id, |x| x.status = MemberStatus::Idle);
            return Err(e);
        }
        Ok(())
    }

    /// The compact finished (or failed): say so, then give it what it missed.
    async fn compact_done(self: &Arc<Self>, panel: &str, member: &str, tokens: Option<(u64, u64)>, error: Option<String>) {
        let Some(cwd) = self.rt(panel, |r| r.compacting.remove(member)) else { return };
        // Claude's "after" count leaves out its system prompt and tools, so it reads far lower than
        // the context really is; the real figure comes with the next reply.
        self.update_member(panel, member, |m| m.status = MemberStatus::Idle);
        let alias = self.alias_of(panel, member).unwrap_or_default();
        match (error, tokens) {
            (Some(e), _) => self.system(panel, member, tr!("agent.compactFailed", alias = alias, error = e), true),
            (None, Some((before, _))) => self.system(panel, member, tr!("agent.compacted", alias = alias, before = before), false),
            (None, None) => self.system(panel, member, tr!("agent.compactedPlain", alias = alias), false),
        }
        // What it missed meanwhile comes as a round of its own, so it never held anyone else's up.
        if self.rt(panel, |r| r.missed.remove(member) | r.pending.remove(member)) {
            self.queue_round(panel, Some(vec![member.to_string()]), cwd);
        }
    }

    /// Changes what an agent may do without asking. Its process restarts (the conversation is
    /// resumed) so the next turn runs under the new mode; not while it is answering.
    pub async fn set_mode(&self, panel: &str, member: &str, mode: PermMode) -> Result<(), String> {
        let (alias, status, old) = self
            .with_ws(panel, |ws| ws.members.iter().find(|m| m.id == member).map(|m| (m.alias.clone(), m.status, m.mode)))
            .ok_or_else(|| tr!("err.noAgent"))?;
        if old == mode {
            return Ok(());
        }
        if status != MemberStatus::Idle {
            return Err(tr!("agent.err.busyMode", alias = alias));
        }
        self.update_member(panel, member, |m| m.mode = mode);
        self.kill(panel, member).await;
        self.system(panel, "system", tr!("agent.modeChanged", alias = alias, mode = mode.label()), false);
        Ok(())
    }

    /// Normalize a proposed alias and make sure it is usable in this chat (`except`: the member being renamed).
    fn check_alias(&self, panel: &str, alias: &str, except: Option<&str>) -> Result<String, String> {
        let alias = alias.trim().trim_start_matches('@').to_string();
        if alias.is_empty() {
            return Err(tr!("agent.err.emptyAlias"));
        }
        if alias.chars().any(|c| !(c.is_alphanumeric() || c == '_' || c == '-')) {
            return Err(tr!("agent.err.badAlias"));
        }
        let dup = self.with_ws(panel, |ws| ws.members.iter().any(|m| Some(m.id.as_str()) != except && m.alias.eq_ignore_ascii_case(&alias)));
        if dup {
            return Err(tr!("agent.err.duplicate", alias = alias));
        }
        Ok(alias)
    }

    /// Everyone sees the new name from the next round on (the roster is re-sent). An idle agent's
    /// process is restarted so its own instructions ("You are @…") carry the new name too; its
    /// conversation is resumed, so nothing is lost.
    pub async fn rename_agent(&self, panel: &str, member: &str, alias: &str) -> Result<(), String> {
        let alias = self.check_alias(panel, alias, Some(member))?;
        let (old, idle) = self
            .with_ws(panel, |ws| {
                let m = ws.members.iter_mut().find(|m| m.id == member)?;
                let old = std::mem::replace(&mut m.alias, alias.clone());
                Some((old, m.status == MemberStatus::Idle))
            })
            .ok_or_else(|| tr!("err.noAgent"))?;
        if old == alias {
            return Ok(());
        }
        if idle {
            self.kill(panel, member).await;
        }
        self.rt(panel, |r| r.roster_version += 1);
        if let Some(m) = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == member).cloned()) {
            self.emit(json!({ "t": "ws_member", "panel": panel, "member": m }));
        }
        self.system(panel, "system", tr!("agent.renamed", old = old, alias = alias), false);
        Ok(())
    }

    pub async fn remove_agent(&self, panel: &str, member: &str) {
        self.kill(panel, member).await;
        let alias = self.with_ws(panel, |ws| {
            let a = ws.members.iter().find(|m| m.id == member).map(|m| m.alias.clone());
            ws.members.retain(|m| m.id != member);
            a
        });
        self.emit(json!({ "t": "ws_member_removed", "panel": panel, "member": member }));
        self.rt(panel, |r| {
            r.roster_version += 1;
            r.pending.remove(member);
            r.speaking.remove(member);
        });
        self.emit_round(panel);
        if let Some(a) = alias {
            self.system(panel, "system", tr!("agent.left", alias = a), false);
        }
    }

    fn emit_round(&self, panel: &str) {
        let (active, queued) = self.rt(panel, |r| (!r.speaking.is_empty(), r.queued_count));
        self.emit(json!({ "t": "ws_round", "panel": panel, "active": active, "queued": queued }));
    }

    pub fn round_state(&self, panel: &str) -> Value {
        let (active, queued) = self.rt(panel, |r| (!r.speaking.is_empty(), r.queued_count));
        json!({ "t": "ws_round", "panel": panel, "active": active, "queued": queued })
    }

    /// A human message. It is shown right away; the round starts once the human has been quiet
    /// for MERGE_WINDOW_MS, or — if a round is running — right after that round ends.
    pub async fn send(self: &Arc<Self>, panel: &str, text: String, cwd: String, via: Option<crate::workspace::DocVia>, reply_to: Option<&str>, files: Vec<String>) -> Result<(), String> {
        let text = text.trim().to_string();
        // Only files in the project, or picked from the computer (granted).
        let files: Vec<String> = files.into_iter().filter(|f| self.usable_file(panel, &cwd, f).is_some()).collect();
        if text.is_empty() && files.is_empty() {
            return Ok(());
        }
        let members: Vec<Member> = self.with_ws(panel, |ws| ws.members.clone());
        if members.is_empty() {
            return Err(tr!("err.noAgents"));
        }
        let targeted = mentions(&text, &members, None);
        let all = targeted.is_empty() || targeted.len() == members.len();
        let mut m = msg("you", Body::Text { text }, false);
        m.to = if all { vec![] } else { targeted.clone() };
        m.files = files;
        m.via = via;
        m.reply_to = reply_to.and_then(|id| {
            self.with_ws(panel, |ws| {
                ws.messages.iter().find(|x| x.id == id).and_then(|x| match &x.body {
                    Body::Text { text } => Some(crate::workspace::ReplyRef {
                        msg: x.id.clone(),
                        from: x.from.clone(),
                        text: text.chars().take(crate::workspace::REPLY_EXCERPT).collect(),
                    }),
                    _ => None,
                })
            })
        });
        self.push_msg(panel, m);
        self.queue_round(panel, if all { None } else { Some(targeted) }, cwd);
        Ok(())
    }

    /// Something for the agents arrived (`targets` None = everyone): open a round for it once the
    /// merge window passes and nobody is still speaking.
    fn queue_round(self: &Arc<Self>, panel: &str, targets: Option<Vec<String>>, cwd: String) {
        let gen = self.rt(panel, |r| {
            r.cwd = cwd.clone();
            match targets {
                None => r.queued_all = true,
                Some(t) => r.queued_targets.get_or_insert_with(HashSet::new).extend(t),
            }
            r.queued_count += 1;
            r.send_gen += 1;
            r.send_gen
        });
        self.emit_round(panel);
        let (hub, panel, cwd) = (self.clone(), panel.to_string(), cwd);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(MERGE_WINDOW_MS)).await;
            let ready = hub.rt(&panel, |r| r.send_gen == gen && r.speaking.is_empty());
            if ready {
                hub.start_round(&panel, &cwd).await;
            }
        });
    }

    /// "Interrupt this round and send now": stop agents, then open a round with the queued messages.
    pub async fn send_now(self: &Arc<Self>, panel: &str, cwd: String) {
        self.interrupt_all(panel).await;
        self.rt(panel, |r| r.speaking.clear());
        self.start_round(panel, &cwd).await;
    }

    async fn start_round(self: &Arc<Self>, panel: &str, cwd: &str) {
        let members: Vec<String> = self.with_ws(panel, |ws| ws.members.iter().map(|m| m.id.clone()).collect());
        let targets: Vec<String> = self.rt(panel, |r| {
            if r.queued_count == 0 {
                return vec![];
            }
            let t: Vec<String> = if r.queued_all {
                members.clone()
            } else {
                let q = r.queued_targets.clone().unwrap_or_default();
                members.iter().filter(|m| q.contains(*m)).cloned().collect()
            };
            r.queued_all = false;
            r.queued_targets = None;
            r.queued_count = 0;
            // A compacting agent doesn't hold the round up; it catches up after.
            let (busy, t): (Vec<String>, Vec<String>) = t.into_iter().partition(|m| r.compacting.contains_key(m));
            r.missed.extend(busy);
            r.speaking = t.iter().cloned().collect();
            t
        });
        self.emit_round(panel);
        for t in targets {
            self.trigger(panel, &t, cwd).await;
        }
    }

    /// Deliver to an agent now, or after its in-flight (interrupted) turn ends.
    async fn trigger(self: &Arc<Self>, panel: &str, member: &str, cwd: &str) {
        let Some(m) = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == member).cloned()) else { return };
        if m.status != MemberStatus::Idle {
            self.rt(panel, |r| r.pending.insert(member.to_string()));
            return;
        }
        if let Err(e) = self.deliver(panel, &m, cwd).await {
            self.system(panel, &m.id, e, true);
            self.update_member(panel, &m.id, |m| m.status = MemberStatus::Idle);
            spawn_member_done(self.clone(), panel.to_string(), member.to_string(), cwd.to_string());
        }
    }

    /// An agent finished (or failed) its reply for this round.
    async fn member_done(self: &Arc<Self>, panel: &str, member: &str, cwd: &str) {
        let (round_over, next) = self.rt(panel, |r| {
            let was = r.speaking.remove(member);
            let over = was && r.speaking.is_empty();
            (over, over && r.queued_count > 0)
        });
        if round_over {
            self.emit_round(panel);
            if next {
                self.start_round(panel, cwd).await;
            }
        }
    }

    /// Called when an agent's turn ends: hide [PASS], then either re-deliver (it was interrupted
    /// into a new round) or mark it done for the round. @mentions never wake anyone.
    async fn turn_ended(self: &Arc<Self>, panel: String, member: String, text: String, text_ids: Vec<String>, cwd: String) {
        // The marker only ever opens the turn's first message; the rest of the turn's text stays.
        let marked = text_ids.first().is_some_and(|first| self.take_reply_marker(&panel, first).is_some());
        let text = if marked { parse_reply_marker(&text).map(|(_, rest)| rest.to_string()).or_else(|| leading_quote(&text).map(|(_, rest)| rest.to_string())).unwrap_or(text) } else { text };
        let trimmed = text.trim();
        let passed = trimmed == "[PASS]" || (trimmed.starts_with("[PASS]") && trimmed.len() < 40);
        if passed {
            self.remove_msgs(&panel, &text_ids);
            let alias = self.with_ws(&panel, |ws| ws.members.iter().find(|m| m.id == member).map(|m| m.alias.clone())).unwrap_or_default();
            self.system(&panel, &member, tr!("agent.passed", alias = alias), false);
        }
        let again = self.rt(&panel, |r| r.pending.remove(&member));
        if again {
            self.trigger(&panel, &member, &cwd).await;
        } else {
            self.member_done(&panel, &member, &cwd).await;
        }
    }

    /// An agent's reply that opens with `↩#12` answers message #12: drop the marker from the text
    /// and record the reply, so the chat shows the quote. Returns the number it pointed at.
    fn take_reply_marker(&self, panel: &str, msg_id: &str) -> Option<u64> {
        let m = self.with_ws(panel, |ws| ws.messages.iter().find(|x| x.id == msg_id).cloned())?;
        let Body::Text { text } = &m.body else { return None };
        // `↩#12`, or failing that a leading `>` quote of an earlier message's words.
        let (target, rest) = match parse_reply_marker(text) {
            Some((seq, rest)) => (self.with_ws(panel, |ws| ws.messages.iter().find(|x| x.seq == seq && x.id != m.id).cloned()), rest),
            None => {
                let (quote, rest) = leading_quote(text)?;
                let found = self.with_ws(panel, |ws| {
                    ws.messages.iter().rev().filter(|x| x.id != m.id && x.seq < m.seq).find(|x| matches!(&x.body, Body::Text { text } if squash(text).contains(&quote))).cloned()
                })?;
                (Some(found), rest)
            }
        };
        let seq = target.as_ref().map(|x| x.seq).unwrap_or(0);
        let reply = target.and_then(|x| match &x.body {
            Body::Text { text } => Some(crate::workspace::ReplyRef { msg: x.id.clone(), from: x.from.clone(), text: text.chars().take(crate::workspace::REPLY_EXCERPT).collect() }),
            _ => None,
        });
        let rest = rest.to_string();
        self.update_msg(panel, msg_id, |x| {
            x.body = Body::Text { text: rest };
            x.reply_to = reply;
        });
        Some(seq)
    }

    async fn interrupt_all(self: &Arc<Self>, panel: &str) {
        let members: Vec<String> = self.with_ws(panel, |ws| ws.members.iter().filter(|m| m.status != MemberStatus::Idle).map(|m| m.id.clone()).collect());
        for member in members {
            self.interrupt(panel, &member).await;
        }
    }

    /// Stop one agent's turn; the others keep going.
    pub async fn stop_member(self: &Arc<Self>, panel: &str, member: &str) {
        let Some(alias) = self.with_ws(panel, |ws| ws.members.iter().find(|m| m.id == member && m.status != MemberStatus::Idle).map(|m| m.alias.clone())) else {
            return;
        };
        self.rt(panel, |r| r.pending.remove(member));
        self.interrupt(panel, member).await;
        self.system(panel, "system", &tr!("agent.stopped", alias = alias), false);
    }

    /// Interrupt one agent's turn, killing it if it doesn't stop within a few seconds.
    async fn interrupt(self: &Arc<Self>, panel: &str, member: &str) {
        let member = member.to_string();
        let rpc = self.procs.lock().unwrap().get(&(panel.to_string(), member.clone())).and_then(|p| p.codex.clone());
        match rpc {
            Some(rpc) => rpc.interrupt(self, panel, &member).await,
            None => {
                let _ = self.write(panel, &member, json!({ "type": "control_request", "request_id": mid(), "request": { "subtype": "interrupt" } })).await;
            }
        }
        let (hub, panel, member) = (self.clone(), panel.to_string(), member.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            let busy = hub.with_ws(&panel, |ws| ws.members.iter().any(|m| m.id == member && m.status != MemberStatus::Idle));
            if busy {
                hub.kill(&panel, &member).await;
                hub.update_member(&panel, &member, |m| m.status = MemberStatus::Idle);
                // It never reported the end of its turn: hand it the next round now if one is waiting.
                let (again, cwd) = hub.rt(&panel, |r| (r.pending.remove(&member), r.cwd.clone()));
                if again {
                    spawn_trigger(hub.clone(), panel.clone(), member.clone(), cwd);
                } else {
                    spawn_member_done(hub.clone(), panel.clone(), member.clone(), cwd);
                }
            }
        });
    }

    pub async fn decide(&self, request_id: &str, decision: &str) -> Result<(), String> {
        let p = self.perms.lock().unwrap().remove(request_id).ok_or_else(|| tr!("perm.err.gone"))?;
        if let Some(rpc_id) = p.codex_request.clone() {
            let (state, d) = match decision {
                "deny" => (PermState::Denied, "decline"),
                "always" => (PermState::AlwaysAllowed, "acceptForSession"),
                _ => (PermState::Allowed, "accept"),
            };
            self.write(&p.panel, &p.member, json!({ "jsonrpc": "2.0", "id": rpc_id, "result": { "decision": d } })).await?;
            self.update_msg(&p.panel, &p.msg_id, |m| {
                if let Body::Permission { state: s, .. } = &mut m.body {
                    *s = state;
                }
            });
            self.update_member(&p.panel, &p.member, |m| m.status = MemberStatus::Thinking);
            return Ok(());
        }
        let (state, response) = match decision {
            "deny" => (PermState::Denied, json!({ "behavior": "deny", "message": "The user denied this action." })),
            "always" => {
                let rules: Vec<Value> = p.suggestions.as_array().map(|a| a.iter().filter(|s| s["type"] == "addRules").cloned().collect()).unwrap_or_default();
                (PermState::AlwaysAllowed, json!({ "behavior": "allow", "updatedInput": p.input, "updatedPermissions": rules }))
            }
            _ => (PermState::Allowed, json!({ "behavior": "allow", "updatedInput": p.input })),
        };
        self.write(&p.panel, &p.member, json!({ "type": "control_response", "response": { "subtype": "success", "request_id": request_id, "response": response } })).await?;
        self.update_msg(&p.panel, &p.msg_id, |m| {
            if let Body::Permission { state: s, .. } = &mut m.body {
                *s = state;
            }
        });
        self.update_member(&p.panel, &p.member, |m| m.status = MemberStatus::Thinking);
        Ok(())
    }

    /// Stop everything: interrupt agents, end the round, drop queued messages' delivery.
    pub async fn stop(self: &Arc<Self>, panel: &str) {
        self.interrupt_all(panel).await;
        self.rt(panel, |r| {
            r.pending.clear();
            r.speaking.clear();
            r.queued_all = false;
            r.queued_targets = None;
            r.queued_count = 0;
            r.send_gen += 1;
        });
        self.emit_round(panel);
        self.system(panel, "system", tr!("chat.stopped"), false);
    }

    /// Panel closed or switched to another kind: stop agents, optionally drop the transcript.
    pub async fn close_panel(&self, panel: &str, delete: bool) {
        let members: Vec<String> = self.procs.lock().unwrap().keys().filter(|(p, _)| p == panel).map(|(_, m)| m.clone()).collect();
        for m in members {
            self.kill(panel, &m).await;
        }
        if delete {
            self.workspaces.lock().unwrap().remove(panel);
            self.dirty.lock().unwrap().remove(panel);
            Workspace::delete(panel);
        }
    }

    pub async fn shutdown(&self) {
        let keys: Vec<(String, String)> = self.procs.lock().unwrap().keys().cloned().collect();
        for (p, m) in keys {
            self.kill(&p, &m).await;
        }
        self.flush();
    }

    // ---------- process plumbing ----------

    async fn kill(&self, panel: &str, member: &str) {
        let proc = self.procs.lock().unwrap().remove(&(panel.to_string(), member.to_string()));
        if let Some(p) = proc {
            let _ = p.child.lock().await.kill().await;
        }
        let stale: Vec<String> = self.perms.lock().unwrap().iter().filter(|(_, v)| v.panel == panel && v.member == member).map(|(k, _)| k.clone()).collect();
        for k in stale {
            if let Some(p) = self.perms.lock().unwrap().remove(&k) {
                self.update_msg(&p.panel, &p.msg_id, |m| {
                    if let Body::Permission { state, .. } = &mut m.body {
                        *state = PermState::Denied;
                    }
                });
            }
        }
    }

    async fn write(&self, panel: &str, member: &str, v: Value) -> Result<(), String> {
        let stdin = self.procs.lock().unwrap().get(&(panel.to_string(), member.to_string())).map(|p| p.stdin.clone());
        let stdin = stdin.ok_or("agent is not running")?;
        let mut line = v.to_string();
        line.push('\n');
        let mut w = stdin.lock().await;
        w.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
        w.flush().await.map_err(|e| e.to_string())
    }

    /// Send an agent everything it hasn't seen yet (other people's text), as one user turn.
    async fn deliver(self: &Arc<Self>, panel: &str, m: &Member, cwd: &str) -> Result<(), String> {
        let (batch, upto) = self.with_ws(panel, |ws| {
            let names: HashMap<String, String> = ws.members.iter().map(|x| (x.id.clone(), x.alias.clone())).collect();
            let unseen: Vec<&Message> = ws.messages.iter().filter(|x| x.seq > m.seen && (x.from != m.id || x.from_thread.is_some())).collect();
            let upto = ws.next_seq;
            // What a message answers, for its header: `, replying to @x's message: "…"`.
            let re = |x: &Message| {
                x.reply_to.as_ref().map(|r| {
                    let who = if r.from == "you" { "the human's message".to_string() } else if r.from == x.from { "their own earlier message".to_string() } else { format!("@{}'s message", names.get(&r.from).cloned().unwrap_or_else(|| "agent".into())) };
                    format!(", replying to {who}: \"{}{}\"", r.text.replace('\n', " "), if r.text.chars().count() >= crate::workspace::REPLY_EXCERPT { "…" } else { "" })
                }).unwrap_or_default()
            };
            // Every line carries the message's number, so a reply can point at it (`↩#12`).
            let lines: Vec<String> = unseen
                .iter()
                .filter_map(|x| match &x.body {
                    Body::Text { text } if x.from == "you" => {
                        let to = if x.to.is_empty() { "everyone".to_string() } else { x.to.iter().map(|t| format!("@{}", names.get(t).cloned().unwrap_or_default())).collect::<Vec<_>>().join(" ") };
                        let files = if x.files.is_empty() {
                            String::new()
                        } else {
                            format!("\n(attached files, read them yourself: {})", x.files.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", "))
                        };
                        Some(format!("[#{} human → {to}{}]: {text}{files}", x.seq, re(x)))
                    }
                    Body::Text { text } if x.from_thread.is_some() => Some(format!(
                        "[#{} @{} — conclusion of thread #{}, brought back to the chat]: {}",
                        x.seq,
                        names.get(&x.from).cloned().unwrap_or_else(|| "agent".into()),
                        x.from_thread.as_ref().map(|t| t.num).unwrap_or(0),
                        text
                    )),
                    Body::Text { text } => Some(format!("[#{} @{}{}]: {}", x.seq, names.get(&x.from).cloned().unwrap_or_else(|| "agent".into()), re(x), text)),
                    b @ (Body::Poll { .. } | Body::PollClosed { .. }) => Self::poll_line(b, &ws.polls, &names),
                    Body::Doc { doc, title, updated } => Some(format!(
                        "[@{} {} document `{doc}` \"{title}\" — read it with doc_read]",
                        names.get(&x.from).cloned().unwrap_or_else(|| "agent".into()),
                        if *updated { "updated" } else { "shared" }
                    )),
                    _ => None,
                })
                .collect();
            (lines, upto)
        });
        // Trim very long history (e.g. an agent joining late).
        let mut batch = batch;
        let mut omitted = 0;
        while batch.len() > HISTORY_MSGS || (batch.len() > 1 && batch.iter().map(|l| l.len()).sum::<usize>() > HISTORY_CHARS) {
            batch.remove(0);
            omitted += 1;
        }
        if batch.is_empty() {
            return Ok(());
        }
        let roster = self.with_ws(panel, |ws| {
            ws.members.iter().map(|x| format!("@{}{}", x.alias, x.model.as_ref().map(|mm| format!(" ({mm})")).unwrap_or_default())).collect::<Vec<_>>().join(", ")
        });
        let (ver, seen_ver) = self.rt(panel, |r| (r.roster_version, r.roster_seen.get(&m.id).copied()));
        let mut text = String::new();
        if seen_ver != Some(ver) {
            text.push_str(&format!("[Recursion] Members of this chat: {roster}. You are @{}.\n", m.alias));
            self.rt(panel, |r| r.roster_seen.insert(m.id.clone(), ver));
        }
        if omitted > 0 {
            text.push_str(&format!("[Recursion] ({omitted} earlier messages omitted)\n"));
        }
        text.push_str(&batch.join("\n\n"));

        self.rt(panel, |r| r.turn_first.remove(&m.id));
        let running = self.procs.lock().unwrap().contains_key(&(panel.to_string(), m.id.clone()));
        if !running {
            match m.kind {
                AgentKind::Claude => self.spawn(panel, m, cwd).await?,
                AgentKind::Codex => self.spawn_codex(panel, m, cwd).await?,
            }
        }
        self.update_member(panel, &m.id, |x| {
            x.status = MemberStatus::Thinking;
            x.seen = upto;
        });
        if m.kind == AgentKind::Codex {
            let rpc = self.procs.lock().unwrap().get(&(panel.to_string(), m.id.clone())).and_then(|p| p.codex.clone()).ok_or("Codex is not running")?;
            return rpc.start_turn(self, panel, &m.id, &text).await;
        }
        self.write(panel, &m.id, json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })).await
    }

    async fn spawn(self: &Arc<Self>, panel: &str, m: &Member, cwd: &str) -> Result<(), String> {
        let env = self.env().await;
        let bin = env.claude.clone().ok_or_else(|| tr!("err.noCli", cli = "claude"))?;
        let prompt = group_prompt(&m.alias);
        let mut cmd = Command::new(bin);
        cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
            .args(["--permission-prompt-tool", "stdio", "--permission-mode", m.mode.claude()])
            .args(["--append-system-prompt", &prompt]);
        let mcp = json!({ "mcpServers": { "recursion": { "type": "http", "url": self.mcp_url(panel, &m.id) } } });
        cmd.args(["--mcp-config", &mcp.to_string(), "--allowedTools", "mcp__recursion"]);
        if let Some(model) = &m.model {
            cmd.args(["--model", model]);
        }
        if let Some(sid) = &m.cli_session_id {
            cmd.args(["--resume", sid]);
        }
        let dir = if std::path::Path::new(cwd).is_dir() { cwd.to_string() } else { dirs::home_dir().unwrap_or_default().to_string_lossy().to_string() };
        cmd.current_dir(&dir).env("PATH", &env.path).env("CLAUDE_CODE_ENTRYPOINT", "recursion");
        self.accounts.apply(&mut cmd, m.kind, m.account.as_deref());
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        self.update_member(panel, &m.id, |m| m.status = MemberStatus::Starting);
        let mut child = cmd.spawn().map_err(|e| tr!("err.spawn", cli = "claude", error = e))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let generation = self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let child = Arc::new(tokio::sync::Mutex::new(child));
        self.procs.lock().unwrap().insert((panel.to_string(), m.id.clone()), Proc { stdin: Arc::new(tokio::sync::Mutex::new(stdin)), child: child.clone(), generation, codex: None });

        let base_cost = m.cost_usd;
        let (hub, panel_s, member_s, cwd_s) = (self.clone(), panel.to_string(), m.id.clone(), dir.clone());
        let cwd_done = dir.clone();
        let err_buf = Arc::new(Mutex::new(String::new()));
        {
            let err_buf = err_buf.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let mut b = err_buf.lock().unwrap();
                    if b.len() < 4000 {
                        b.push_str(&l);
                        b.push('\n');
                    }
                }
            });
        }
        tokio::spawn(async move {
            let mut reader = ClaudeReader::new(hub.clone(), panel_s.clone(), member_s.clone(), cwd_s, base_cost);
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    reader.handle(v);
                }
            }
            let status = child.lock().await.wait().await.ok();
            let current = hub.procs.lock().unwrap().get(&(panel_s.clone(), member_s.clone())).map(|p| p.generation);
            if current == Some(generation) {
                hub.procs.lock().unwrap().remove(&(panel_s.clone(), member_s.clone()));
                let was_busy = hub.with_ws(&panel_s, |ws| ws.members.iter().any(|m| m.id == member_s && m.status != MemberStatus::Idle));
                if was_busy {
                    let err = err_buf.lock().unwrap().trim().to_string();
                    let code = status.and_then(|s| s.code()).map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                    let detail = if err.is_empty() { String::new() } else { format!("\n\n```\n{}\n```", err.chars().take(1500).collect::<String>()) };
                    hub.system(&panel_s, &member_s, format!("{}{detail}", tr!("agent.crashed", cli = "Claude", code = code)), true);
                    hub.update_member(&panel_s, &member_s, |m| m.status = MemberStatus::Idle);
                    spawn_member_done(hub.clone(), panel_s.clone(), member_s.clone(), cwd_done.clone());
                }
            }
        });
        Ok(())
    }
}

/// Turns Claude's stream-json output into transcript messages.
struct ClaudeReader {
    hub: Arc<Hub>,
    panel: String,
    member: String,
    cwd: String,
    base_cost: f64,
    /// `total_cost_usd` is cumulative for the process; this is its value at the last turn end.
    last_cost: f64,
    /// content block index → our message id, for the message currently streaming
    blocks: HashMap<u64, String>,
    tools: HashMap<String, String>,
    streamed_api_ids: HashSet<String>,
    turn_text: String,
    turn_text_ids: Vec<String>,
    /// Tokens before and after the last `/compact`, from its `compact_boundary`.
    compacted: Option<(u64, u64)>,
}

impl ClaudeReader {
    fn new(hub: Arc<Hub>, panel: String, member: String, cwd: String, base_cost: f64) -> Self {
        Self { hub, panel, member, cwd, base_cost, last_cost: 0.0, blocks: HashMap::new(), tools: HashMap::new(), streamed_api_ids: HashSet::new(), turn_text: String::new(), turn_text_ids: Vec::new(), compacted: None }
    }

    fn new_msg(&mut self, body: Body, streaming: bool) -> String {
        let is_text = matches!(body, Body::Text { .. });
        if let Body::Text { text } = &body {
            if !self.turn_text.is_empty() {
                self.turn_text.push_str("\n\n");
            }
            self.turn_text.push_str(text);
        }
        let m = self.hub.push_msg(&self.panel, msg(&self.member, body, streaming));
        if is_text {
            self.turn_text_ids.push(m.id.clone());
        }
        m.id
    }

    fn handle(&mut self, v: Value) {
        let hub = self.hub.clone();
        let (panel, member) = (self.panel.clone(), self.member.clone());
        match v["type"].as_str().unwrap_or("") {
            "system" if v["subtype"] == "compact_boundary" => {
                let meta = &v["compact_metadata"];
                if let (Some(before), Some(after)) = (meta["pre_tokens"].as_u64(), meta["post_tokens"].as_u64()) {
                    self.compacted = Some((before, after));
                }
            }
            "system" if v["subtype"] == "init" => {
                if let Some(sid) = v["session_id"].as_str() {
                    let sid = sid.to_string();
                    hub.update_member(&panel, &member, |m| {
                        m.cli_session_id = Some(sid);
                        if m.status == MemberStatus::Starting {
                            m.status = MemberStatus::Thinking;
                        }
                    });
                }
            }
            "stream_event" => {
                let ev = &v["event"];
                if let Some(id) = v["api_message_id"].as_str() {
                    self.streamed_api_ids.insert(id.to_string());
                }
                match ev["type"].as_str().unwrap_or("") {
                    "message_start" => self.blocks.clear(),
                    "content_block_start" => {
                        let idx = ev["index"].as_u64().unwrap_or(0);
                        let block = &ev["content_block"];
                        match block["type"].as_str().unwrap_or("") {
                            "text" => {
                                let text = block["text"].as_str().unwrap_or("").to_string();
                                let id = self.new_msg(Body::Text { text }, true);
                                self.blocks.insert(idx, id);
                            }
                            "tool_use" => {
                                let tid = block["id"].as_str().unwrap_or("").to_string();
                                let name = display_tool_name(block["name"].as_str().unwrap_or("tool"));
                                let id = self.new_msg(Body::Tool { tool_use_id: tid.clone(), name, summary: String::new(), detail: String::new(), result: None, is_error: false }, true);
                                self.tools.insert(tid, id.clone());
                                self.blocks.insert(idx, id);
                            }
                            _ => {}
                        }
                    }
                    "content_block_delta" => {
                        let idx = ev["index"].as_u64().unwrap_or(0);
                        if ev["delta"]["type"] == "text_delta" {
                            if let (Some(id), Some(t)) = (self.blocks.get(&idx).cloned(), ev["delta"]["text"].as_str()) {
                                let t = t.to_string();
                                self.turn_text.push_str(&t);
                                hub.with_ws(&panel, |ws| {
                                    if let Some(Message { body: Body::Text { text }, .. }) = ws.message_mut(&id) {
                                        text.push_str(&t);
                                    }
                                });
                                hub.emit(json!({ "t": "ws_delta", "panel": panel, "id": id, "text": t }));
                            }
                        }
                    }
                    "content_block_stop" => {
                        let idx = ev["index"].as_u64().unwrap_or(0);
                        if let Some(id) = self.blocks.get(&idx).cloned() {
                            let is_text = hub.with_ws(&panel, |ws| matches!(ws.message_mut(&id).map(|m| &m.body), Some(Body::Text { .. })));
                            if is_text {
                                hub.update_msg(&panel, &id, |m| m.streaming = false);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                // What this model call had in context = everything it read plus what it wrote.
                let u = &v["message"]["usage"];
                let used = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens"].iter().map(|k| u[k].as_u64().unwrap_or(0)).sum::<u64>();
                if used > 0 && v["parent_tool_use_id"].is_null() {
                    hub.update_member(&panel, &member, |m| m.context_used = used);
                }
                let api_id = v["message"]["id"].as_str().unwrap_or("").to_string();
                let streamed = self.streamed_api_ids.contains(&api_id);
                for block in v["message"]["content"].as_array().cloned().unwrap_or_default() {
                    match block["type"].as_str().unwrap_or("") {
                        "tool_use" => {
                            let tid = block["id"].as_str().unwrap_or("").to_string();
                            let name = block["name"].as_str().unwrap_or("tool").to_string();
                            let (summary, detail) = summarize_tool(&name, &block["input"], &self.cwd);
                            match self.tools.get(&tid).cloned() {
                                Some(id) => hub.update_msg(&panel, &id, |m| {
                                    m.streaming = false;
                                    if let Body::Tool { summary: s, detail: d, .. } = &mut m.body {
                                        *s = summary;
                                        *d = detail;
                                    }
                                }),
                                None => {
                                    let id = self.new_msg(Body::Tool { tool_use_id: tid.clone(), name: display_tool_name(&name), summary, detail, result: None, is_error: false }, false);
                                    self.tools.insert(tid, id);
                                }
                            }
                        }
                        "text" if !streamed => {
                            let text = block["text"].as_str().unwrap_or("").to_string();
                            if !text.is_empty() {
                                self.new_msg(Body::Text { text }, false);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in v["message"]["content"].as_array().cloned().unwrap_or_default() {
                    if block["type"] == "tool_result" {
                        let tid = block["tool_use_id"].as_str().unwrap_or("");
                        if let Some(id) = self.tools.get(tid).cloned() {
                            let text = tool_result_text(&block["content"]);
                            let is_err = block["is_error"].as_bool().unwrap_or(false);
                            hub.update_msg(&panel, &id, |m| {
                                m.streaming = false;
                                if let Body::Tool { result, is_error, .. } = &mut m.body {
                                    *result = Some(text.chars().take(4000).collect());
                                    *is_error = is_err;
                                }
                            });
                        }
                    }
                }
            }
            "control_request" => {
                let req = &v["request"];
                if req["subtype"] == "can_use_tool" {
                    let request_id = v["request_id"].as_str().unwrap_or("").to_string();
                    // Reading a file the human picked for this chat was already their go-ahead.
                    let read = matches!(req["tool_name"].as_str(), Some("Read" | "Grep" | "Glob" | "LS"));
                    let target = req["input"]["file_path"].as_str().or(req["input"]["path"].as_str()).unwrap_or("");
                    if read && !target.is_empty() && hub.usable_file(&panel, &self.cwd, target).is_some() && std::path::Path::new(target).is_absolute() {
                        let (hub, panel, member, input) = (hub.clone(), panel.clone(), member.clone(), req["input"].clone());
                        tokio::spawn(async move {
                            let response = json!({ "behavior": "allow", "updatedInput": input });
                            let _ = hub.write(&panel, &member, json!({ "type": "control_response", "response": { "subtype": "success", "request_id": request_id, "response": response } })).await;
                        });
                        return;
                    }
                    let tool = req["display_name"].as_str().or(req["tool_name"].as_str()).unwrap_or("tool").to_string();
                    let (summary, detail) = summarize_tool(req["tool_name"].as_str().unwrap_or(""), &req["input"], &self.cwd);
                    let msg_id = self.new_msg(Body::Permission { request_id: request_id.clone(), tool, summary, detail, state: PermState::Pending }, false);
                    hub.perms.lock().unwrap().insert(request_id, PendingPerm { panel: panel.clone(), member: member.clone(), msg_id, input: req["input"].clone(), suggestions: req["permission_suggestions"].clone(), codex_request: None });
                    hub.update_member(&panel, &member, |m| m.status = MemberStatus::WaitingPermission);
                }
            }
            "rate_limit_event" => {
                if let Some(windows) = v["rate_limit_info"]["unifiedWindows"].as_object() {
                    let usage: Vec<(String, f64)> = windows.iter().filter_map(|(k, w)| w["utilization"].as_f64().map(|u| (k.clone(), u))).collect();
                    let resets: std::collections::BTreeMap<String, u64> = windows.iter().filter_map(|(k, w)| w["resetsAt"].as_u64().map(|t| (k.clone(), t))).collect();
                    hub.update_member(&panel, &member, |m| usage.into_iter().for_each(|(k, u)| {
                        m.usage.insert(k, u);
                    }));
                    let m = hub.with_ws(&panel, |ws| ws.members.iter().find(|m| m.id == member).cloned());
                    if let Some(m) = m {
                        if hub.accounts.record_usage(&hub.accounts.resolve(m.kind, m.account.as_deref()), &m.usage, &resets) {
                            hub.emit_accounts();
                        }
                    }
                }
            }
            "result" => {
                let cost = v["total_cost_usd"].as_f64();
                let base = self.base_cost;
                hub.update_member(&panel, &member, |m| {
                    m.status = MemberStatus::Idle;
                    if let Some(c) = cost {
                        m.cost_usd = base + c;
                    }
                });
                // modelUsage lists every model the turn used (subagents too); the main one did the most reading.
                let window = v["modelUsage"].as_object().and_then(|mu| {
                    mu.values()
                        .max_by_key(|x| x["inputTokens"].as_u64().unwrap_or(0) + x["cacheReadInputTokens"].as_u64().unwrap_or(0) + x["cacheCreationInputTokens"].as_u64().unwrap_or(0))
                        .and_then(|x| x["contextWindow"].as_u64())
                });
                if let Some(w) = window {
                    hub.update_member(&panel, &member, |m| m.context_window = w);
                }
                let u = &v["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let usage = TurnUsage {
                    input: n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens"),
                    cached: n("cache_read_input_tokens"),
                    output: n("output_tokens"),
                    cost_usd: cost.map(|c| (c - self.last_cost).max(0.0)),
                };
                if let Some(c) = cost {
                    self.last_cost = c;
                }
                if hub.is_compacting(&panel, &member) {
                    self.blocks.clear();
                    self.turn_text.clear();
                    self.turn_text_ids.clear();
                    let error = v["is_error"].as_bool().unwrap_or(false).then(|| v["result"].as_str().unwrap_or("").to_string());
                    spawn_compact_done(hub, panel, member, self.compacted.take(), error);
                    return;
                }
                hub.finish_turn(&panel, &member, (usage.input + usage.output > 0).then_some(usage));
                if v["is_error"].as_bool().unwrap_or(false) && v["subtype"] != "error_during_execution" {
                    let text = v["result"].as_str().unwrap_or("Claude reported an error").to_string();
                    hub.system(&panel, &member, text, true);
                }
                self.blocks.clear();
                let text = std::mem::take(&mut self.turn_text);
                let ids = std::mem::take(&mut self.turn_text_ids);
                let (hub, cwd) = (self.hub.clone(), self.cwd.clone());
                tokio::spawn(async move { hub.turn_ended(panel, member, text, ids, cwd).await });
            }
            _ => {}
        }
    }
}

/// `↩#12` (or `↩ #12`) opening a reply, then the rest of the text.
fn parse_reply_marker(text: &str) -> Option<(u64, &str)> {
    let t = text.trim_start();
    let t = t.strip_prefix('↩')?.trim_start().strip_prefix('#')?;
    let end = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let seq = t[..end].parse().ok()?;
    Some((seq, t[end..].trim_start()))
}

/// A reply that opens with a `>` blockquote: the quoted words (squashed) and what follows.
/// Short quotes are ignored; they are too likely to match by chance.
fn leading_quote(text: &str) -> Option<(String, &str)> {
    let t = text.trim_start();
    if !t.starts_with('>') {
        return None;
    }
    let end = t.split_inclusive('\n').take_while(|l| l.trim_start().starts_with('>')).map(str::len).sum::<usize>();
    let quote: String = t[..end].lines().map(|l| l.trim_start().trim_start_matches('>').trim()).collect::<Vec<_>>().join(" ");
    let quote = squash(&quote);
    (quote.chars().count() >= 6).then(|| (quote, t[end..].trim_start()))
}

/// Text with Markdown marks and runs of whitespace taken out, for comparing quotes.
fn squash(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '#')).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Instructions shared by every agent kind.
fn group_prompt(alias: &str) -> String {
    format!(
        "You are @{alias}, an agent in a Recursion group chat: a human working with one or more AI agents in the same project.\n\
         - The chat is turn-based. Each time the human speaks, every addressed agent replies exactly once, all at the same time; \
         then everyone waits for the human again. You will not see other agents' replies from the same round until the next round.\n\
         - New messages arrive prefixed with their speaker, e.g. `[human → everyone]: ...` or `[@other]: ...`. You only receive \
         messages you have not seen yet; your own earlier replies are already in your context.\n\
         - You may address another agent as @alias, but that does not make them reply now; they will read it next round.\n\
         - Each message starts with its number, e.g. `[#12 @other]`. To quote or answer one particular message, open your \
         reply with `↩#12` (that number) followed by your text: the chat then shows that message quoted in your bubble, \
         linked to the original. Don't copy its words into a `>` blockquote for this. Skip it when you simply answer the \
         latest message.\n\
         - If the human @mentioned you by name, you must respond. Only when a message was sent to everyone and you truly have \
         nothing useful to add may you reply with exactly [PASS] and nothing else.\n\
         - Your replies are rendered as Markdown, so use tables, lists and code blocks where they help.\n\
         - For anything long or visual (reports, big tables, diagrams, dashboards), call the `doc_write` tool (Recursion MCP server) \
         to save an HTML document: the human gets a card that opens it in its own panel, and every agent in the chat can read it \
         with `doc_read`. Then keep your chat reply short. Revise a document by passing its id instead of making a new one.\n\
         - For architecture, workflow or sequence diagrams, call `diagram_write` with the boxes and lines as JSON; the app lays \
         them out and draws them. Fix the layout problems it reports by moving boxes to other cells.\n\
         - Documents can talk back. Inside the page, `recursion.reply(data)` delivers `data` (string or JSON) to you as the \
         human's next message, and `recursion.sendPrompt(text)` posts to the whole chat. So when you want the human to choose, \
         fill in or answer something, make a document with a form or buttons that calls `recursion.reply(...)` on submit, \
         sending only what the human picked or typed (e.g. `{{\"database\": \"SQLite\"}}`); your advice comes after, in the chat. \
         Never ask them to copy text back into the chat.\n\
         - You have your own browser panel next to the chat, which the human can watch: `browse_open` a URL (any site, or the \
         project's dev server such as http://localhost:5173) to get the page as text with numbered links, buttons and fields, then \
         `browse_click`, `browse_fill`, `browse_read` and `browse_screenshot`. It shares the human's logins, so never buy, post, \
         send, delete or change settings there without the human's explicit go-ahead.\n\
         - The human may start a poll: vote with `poll_vote` (option letter and a one-sentence reason), then keep your reply \
         to a sentence. You don't see the others' votes until it closes; `poll_list` / `poll_read` show past results.\n\
         - Reply in the human's language."
    )
}

/// Plain fn so the readers' future types don't recurse into the round machinery.
fn spawn_compact_done(hub: Arc<Hub>, panel: String, member: String, tokens: Option<(u64, u64)>, error: Option<String>) {
    tokio::spawn(async move { hub.compact_done(&panel, &member, tokens, error).await });
}

/// Plain fn so callers' future types don't recurse into the round machinery.
fn spawn_member_done(hub: Arc<Hub>, panel: String, member: String, cwd: String) {
    tokio::spawn(async move { hub.member_done(&panel, &member, &cwd).await });
}

fn spawn_trigger(hub: Arc<Hub>, panel: String, member: String, cwd: String) {
    tokio::spawn(async move { hub.trigger(&panel, &member, &cwd).await });
}

#[cfg(test)]
mod reply_marker_tests {
    use super::parse_reply_marker;

    #[test]
    fn reads_the_reply_marker() {
        assert_eq!(parse_reply_marker("↩#12 對，這樣比較好"), Some((12, "對，這樣比較好")));
        assert_eq!(parse_reply_marker("  ↩ #3\n\nyes"), Some((3, "yes")));
        assert_eq!(parse_reply_marker("#12 no arrow"), None);
        assert_eq!(parse_reply_marker("↩ nothing"), None);
    }

    #[test]
    fn reads_a_leading_quote() {
        use super::leading_quote;
        let (q, rest) = leading_quote("> 問題一：台灣最高的山是哪座？\n\n台灣最高的山是**玉山**").unwrap();
        assert_eq!(q, "問題一：台灣最高的山是哪座？");
        assert_eq!(rest, "台灣最高的山是**玉山**");
        assert!(leading_quote("> ok\n\nyes").is_none(), "too short to trust");
        assert!(leading_quote("no quote").is_none());
    }
}
