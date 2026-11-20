//! Agent processes and the workspace hub (transcripts + routing to agents).
//!
//! Claude is driven through `claude -p --input-format stream-json --output-format stream-json`
//! with `--permission-prompt-tool stdio`, so permission prompts arrive as `control_request`
//! messages on stdout and are answered with `control_response` on stdin.

use crate::workspace::{mentions, now_ms, summarize_tool, tool_result_text, AgentKind, Body, Member, MemberStatus, Message, PermState, Workspace};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::broadcast;

mod codex;
mod models;

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
        let path = lines.first().filter(|l| !l.starts_with("claude=") && !l.starts_with("codex=")).map(|s| s.to_string()).unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
        let find = |k: &str| lines.iter().find_map(|l| l.strip_prefix(k)).map(|s| s.trim().to_string()).filter(|s| s.starts_with('/'));
        AgentEnv { path, claude: find("claude="), codex: find("codex=") }
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
}

fn mid() -> String {
    crate::model::new_id()
}

fn msg(from: &str, body: Body, streaming: bool) -> Message {
    Message { id: mid(), ts: now_ms(), from: from.into(), seq: 0, to: vec![], streaming, body }
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
        })
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

    /// Who an MCP key belongs to: (panel, member id, alias).
    pub fn mcp_caller(&self, key: &str) -> Option<(String, String, String)> {
        let (panel, member) = self.mcp.lock().unwrap().get(key).cloned()?;
        let alias = self.with_ws(&panel, |ws| ws.members.iter().find(|m| m.id == member).map(|m| m.alias.clone()))?;
        Some((panel, member, alias))
    }

    /// The card in the chat for a document an agent just wrote.
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

    fn push_msg(&self, panel: &str, msg: Message) -> Message {
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
        let body = if error { Body::Error { text } } else { Body::System { text } };
        self.push_msg(panel, msg(from, body, false));
    }

    // ---------- commands from clients ----------

    pub async fn add_agent(self: &Arc<Self>, panel: &str, kind: AgentKind, alias: String, model: Option<String>) -> Result<(), String> {
        let env = self.env().await;
        match kind {
            AgentKind::Claude if env.claude.is_none() => {
                return Err("找不到 claude 指令。請先安裝 Claude Code 並執行 claude auth login".into());
            }
            AgentKind::Codex => {
                let Some(bin) = env.codex.clone() else {
                    return Err("找不到 codex 指令。請先安裝 Codex CLI（npm i -g @openai/codex）並執行 codex login".into());
                };
                let out = Command::new(bin).args(["login", "status"]).env("PATH", &env.path).output().await.map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                if !out.status.success() || text.to_lowercase().contains("not logged in") {
                    return Err("Codex 還沒登入，請先在終端執行 codex login".into());
                }
            }
            _ => {}
        }
        let alias = alias.trim().trim_start_matches('@').to_string();
        if alias.is_empty() {
            return Err("別名不能是空的".into());
        }
        let dup = self.with_ws(panel, |ws| ws.members.iter().any(|m| m.alias.eq_ignore_ascii_case(&alias)));
        if dup {
            return Err(format!("這個群聊已經有 @{alias} 了"));
        }
        if alias.chars().any(|c| !(c.is_alphanumeric() || c == '_' || c == '-')) {
            return Err("別名只能用英數字、中文、底線或連字號".into());
        }
        let member = Member { id: mid(), kind, alias: alias.clone(), model: model.filter(|m| !m.is_empty()), cli_session_id: None, cost_usd: 0.0, status: MemberStatus::Idle, usage: Default::default(), seen: 0 };
        self.with_ws(panel, |ws| ws.members.push(member.clone()));
        self.rt(panel, |r| r.roster_version += 1);
        self.emit(json!({ "t": "ws_member", "panel": panel, "member": member }));
        self.system(panel, &member.id, format!("@{alias} 加入了群聊"), false);
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
            self.system(panel, "system", format!("@{a} 離開了群聊"), false);
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
    pub async fn send(self: &Arc<Self>, panel: &str, text: String, cwd: String) -> Result<(), String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        let members: Vec<Member> = self.with_ws(panel, |ws| ws.members.clone());
        if members.is_empty() {
            return Err("這個群聊還沒有 agent".into());
        }
        let targeted = mentions(&text, &members, None);
        let all = targeted.is_empty() || targeted.len() == members.len();
        let mut m = msg("you", Body::Text { text }, false);
        m.to = if all { vec![] } else { targeted.clone() };
        self.push_msg(panel, m);
        let gen = self.rt(panel, |r| {
            r.cwd = cwd.clone();
            if all {
                r.queued_all = true;
            } else {
                r.queued_targets.get_or_insert_with(HashSet::new).extend(targeted);
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
        Ok(())
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
        let trimmed = text.trim();
        let passed = trimmed == "[PASS]" || (trimmed.starts_with("[PASS]") && trimmed.len() < 40);
        if passed {
            self.remove_msgs(&panel, &text_ids);
            let alias = self.with_ws(&panel, |ws| ws.members.iter().find(|m| m.id == member).map(|m| m.alias.clone())).unwrap_or_default();
            self.system(&panel, &member, format!("@{alias} 略過（沒有要補充的）"), false);
        }
        let again = self.rt(&panel, |r| r.pending.remove(&member));
        if again {
            self.trigger(&panel, &member, &cwd).await;
        } else {
            self.member_done(&panel, &member, &cwd).await;
        }
    }

    async fn interrupt_all(self: &Arc<Self>, panel: &str) {
        let members: Vec<String> = self.with_ws(panel, |ws| ws.members.iter().filter(|m| m.status != MemberStatus::Idle).map(|m| m.id.clone()).collect());
        for member in members {
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
                    }
                }
            });
        }
    }

    pub async fn decide(&self, request_id: &str, decision: &str) -> Result<(), String> {
        let p = self.perms.lock().unwrap().remove(request_id).ok_or("這個核准請求已經結束了")?;
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
            "deny" => (PermState::Denied, json!({ "behavior": "deny", "message": "使用者拒絕了這個動作。" })),
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
        self.system(panel, "system", "已停止", false);
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
        let stdin = stdin.ok_or("agent 沒有在執行")?;
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
            let unseen: Vec<&Message> = ws.messages.iter().filter(|x| x.seq > m.seen && x.from != m.id).collect();
            let upto = ws.next_seq;
            let lines: Vec<String> = unseen
                .iter()
                .filter_map(|x| match &x.body {
                    Body::Text { text } if x.from == "you" => {
                        let to = if x.to.is_empty() { "everyone".to_string() } else { x.to.iter().map(|t| format!("@{}", names.get(t).cloned().unwrap_or_default())).collect::<Vec<_>>().join(" ") };
                        Some(format!("[human → {to}]: {text}"))
                    }
                    Body::Text { text } => Some(format!("[@{}]: {}", names.get(&x.from).cloned().unwrap_or_else(|| "agent".into()), text)),
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
            let rpc = self.procs.lock().unwrap().get(&(panel.to_string(), m.id.clone())).and_then(|p| p.codex.clone()).ok_or("Codex 沒有在執行")?;
            return rpc.start_turn(self, panel, &m.id, &text).await;
        }
        self.write(panel, &m.id, json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })).await
    }

    async fn spawn(self: &Arc<Self>, panel: &str, m: &Member, cwd: &str) -> Result<(), String> {
        let env = self.env().await;
        let bin = env.claude.clone().ok_or("找不到 claude 指令")?;
        let prompt = group_prompt(&m.alias);
        let mut cmd = Command::new(bin);
        cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
            .args(["--permission-prompt-tool", "stdio", "--permission-mode", "acceptEdits"])
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
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        self.update_member(panel, &m.id, |m| m.status = MemberStatus::Starting);
        let mut child = cmd.spawn().map_err(|e| format!("無法啟動 claude：{e}"))?;
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
                    hub.system(&panel_s, &member_s, format!("Claude 意外結束（exit {code}）。下一則訊息會自動接續對話。{detail}"), true);
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
    /// content block index → our message id, for the message currently streaming
    blocks: HashMap<u64, String>,
    tools: HashMap<String, String>,
    streamed_api_ids: HashSet<String>,
    turn_text: String,
    turn_text_ids: Vec<String>,
}

impl ClaudeReader {
    fn new(hub: Arc<Hub>, panel: String, member: String, cwd: String, base_cost: f64) -> Self {
        Self { hub, panel, member, cwd, base_cost, blocks: HashMap::new(), tools: HashMap::new(), streamed_api_ids: HashSet::new(), turn_text: String::new(), turn_text_ids: Vec::new() }
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
                                let name = block["name"].as_str().unwrap_or("tool").to_string();
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
                                    let id = self.new_msg(Body::Tool { tool_use_id: tid.clone(), name, summary, detail, result: None, is_error: false }, false);
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
                    hub.update_member(&panel, &member, |m| usage.into_iter().for_each(|(k, u)| {
                        m.usage.insert(k, u);
                    }));
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
                if v["is_error"].as_bool().unwrap_or(false) && v["subtype"] != "error_during_execution" {
                    let text = v["result"].as_str().unwrap_or("Claude 回報錯誤").to_string();
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

/// Instructions shared by every agent kind.
fn group_prompt(alias: &str) -> String {
    format!(
        "You are @{alias}, an agent in a Recursion group chat: a human working with one or more AI agents in the same project.\n\
         - The chat is turn-based. Each time the human speaks, every addressed agent replies exactly once, all at the same time; \
         then everyone waits for the human again. You will not see other agents' replies from the same round until the next round.\n\
         - New messages arrive prefixed with their speaker, e.g. `[human → everyone]: ...` or `[@other]: ...`. You only receive \
         messages you have not seen yet; your own earlier replies are already in your context.\n\
         - You may address another agent as @alias, but that does not make them reply now; they will read it next round.\n\
         - If the human @mentioned you by name, you must respond. Only when a message was sent to everyone and you truly have \
         nothing useful to add may you reply with exactly [PASS] and nothing else.\n\
         - Your replies are rendered as Markdown, so use tables, lists and code blocks where they help.\n\
         - For anything long or visual (reports, big tables, diagrams, dashboards), call the `doc_write` tool (Recursion MCP server) \
         to save an HTML document: the human gets a card that opens it in its own panel, and every agent in the chat can read it \
         with `doc_read`. Then keep your chat reply short. Revise a document by passing its id instead of making a new one.\n\
         - Reply in the human's language."
    )
}

/// Plain fn so callers' future types don't recurse into the round machinery.
fn spawn_member_done(hub: Arc<Hub>, panel: String, member: String, cwd: String) {
    tokio::spawn(async move { hub.member_done(&panel, &member, &cwd).await });
}

fn spawn_trigger(hub: Arc<Hub>, panel: String, member: String, cwd: String) {
    tokio::spawn(async move { hub.trigger(&panel, &member, &cwd).await });
}
