//! Agent processes and the workspace hub (transcripts + routing to agents).
//!
//! Claude is driven through `claude -p --input-format stream-json --output-format stream-json`
//! with `--permission-prompt-tool stdio`, so permission prompts arrive as `control_request`
//! messages on stdout and are answered with `control_response` on stdin.

use crate::workspace::{now_ms, summarize_tool, tool_result_text, AgentKind, Body, Member, MemberStatus, Message, PermState, Workspace};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::broadcast;

/// PATH and binaries resolved from the user's login shell (GUI apps start with a bare PATH).
#[derive(Clone, Debug, Default)]
pub struct AgentEnv {
    pub path: String,
    pub claude: Option<String>,
}

impl AgentEnv {
    pub async fn detect() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let out = Command::new(&shell)
            .args(["-l", "-c", "printf '%s\\n' \"$PATH\"; command -v claude || true"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let path = lines.next().map(|s| s.to_string()).unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
        let claude = lines.next().map(|s| s.trim().to_string()).filter(|s| s.starts_with('/'));
        AgentEnv { path, claude }
    }
}

struct Proc {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    child: Arc<tokio::sync::Mutex<Child>>,
    /// Bumped on every spawn so a dying old reader can't clobber a new process.
    generation: u64,
}

struct PendingPerm {
    panel: String,
    member: String,
    msg_id: String,
    input: Value,
    suggestions: Value,
}

pub struct Hub {
    workspaces: Mutex<HashMap<String, Workspace>>,
    procs: Mutex<HashMap<(String, String), Proc>>,
    perms: Mutex<HashMap<String, PendingPerm>>,
    dirty: Mutex<HashSet<String>>,
    pub save_notify: tokio::sync::Notify,
    pub tx: broadcast::Sender<String>,
    env: tokio::sync::RwLock<Option<AgentEnv>>,
    generation: std::sync::atomic::AtomicU64,
}

fn mid() -> String {
    crate::model::new_id()
}

impl Hub {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(8192);
        Arc::new(Hub {
            workspaces: Mutex::new(HashMap::new()),
            procs: Mutex::new(HashMap::new()),
            perms: Mutex::new(HashMap::new()),
            dirty: Mutex::new(HashSet::new()),
            save_notify: tokio::sync::Notify::new(),
            tx,
            env: tokio::sync::RwLock::new(None),
            generation: Default::default(),
        })
    }

    async fn env(&self) -> AgentEnv {
        if let Some(e) = self.env.read().await.clone() {
            if e.claude.is_some() {
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
        self.with_ws(panel, |ws| json!({ "t": "ws_state", "panel": panel, "ws": ws }))
    }

    fn push_msg(&self, panel: &str, msg: Message) {
        self.emit(json!({ "t": "ws_msg", "panel": panel, "msg": &msg }));
        self.with_ws(panel, |ws| ws.messages.push(msg));
        self.mark_dirty(panel);
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
        self.push_msg(panel, Message { id: mid(), ts: now_ms(), from: from.into(), streaming: false, body });
    }

    // ---------- commands from clients ----------

    pub async fn add_agent(self: &Arc<Self>, panel: &str, kind: AgentKind, alias: String, model: Option<String>) -> Result<(), String> {
        let env = self.env().await;
        if env.claude.is_none() {
            return Err("找不到 claude 指令。請先安裝 Claude Code（npm i -g @anthropic-ai/claude-code）並執行 claude auth login".into());
        }
        let alias = alias.trim().trim_start_matches('@').to_string();
        if alias.is_empty() {
            return Err("別名不能是空的".into());
        }
        let dup = self.with_ws(panel, |ws| ws.members.iter().any(|m| m.alias.eq_ignore_ascii_case(&alias)));
        if dup {
            return Err(format!("這個群聊已經有 @{alias} 了"));
        }
        let member = Member { id: mid(), kind, alias: alias.clone(), model: model.filter(|m| !m.is_empty()), cli_session_id: None, cost_usd: 0.0, status: MemberStatus::Idle, usage: Default::default() };
        self.with_ws(panel, |ws| ws.members.push(member.clone()));
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
        if let Some(a) = alias {
            self.system(panel, "system", format!("@{a} 離開了群聊"), false);
        }
    }

    pub async fn send(self: &Arc<Self>, panel: &str, text: String, cwd: String) -> Result<(), String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        let members: Vec<Member> = self.with_ws(panel, |ws| ws.members.clone());
        if members.is_empty() {
            return Err("這個群聊還沒有 agent".into());
        }
        self.push_msg(panel, Message { id: mid(), ts: now_ms(), from: "you".into(), streaming: false, body: Body::Text { text: text.clone() } });
        for m in members {
            if let Err(e) = self.deliver(panel, &m, &text, &cwd).await {
                self.system(panel, &m.id, e, true);
                self.update_member(panel, &m.id, |m| m.status = MemberStatus::Idle);
            }
        }
        Ok(())
    }

    pub async fn decide(&self, request_id: &str, decision: &str) -> Result<(), String> {
        let p = self.perms.lock().unwrap().remove(request_id).ok_or("這個核准請求已經結束了")?;
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

    pub async fn stop(self: &Arc<Self>, panel: &str) {
        let members: Vec<String> = self.with_ws(panel, |ws| ws.members.iter().filter(|m| m.status != MemberStatus::Idle).map(|m| m.id.clone()).collect());
        for member in members {
            let _ = self.write(panel, &member, json!({ "type": "control_request", "request_id": mid(), "request": { "subtype": "interrupt" } })).await;
            let (hub, panel, member) = (self.clone(), panel.to_string(), member.clone());
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                let busy = hub.with_ws(&panel, |ws| ws.members.iter().any(|m| m.id == member && m.status != MemberStatus::Idle));
                if busy {
                    hub.kill(&panel, &member).await;
                    hub.update_member(&panel, &member, |m| m.status = MemberStatus::Idle);
                }
            });
        }
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

    async fn deliver(self: &Arc<Self>, panel: &str, m: &Member, text: &str, cwd: &str) -> Result<(), String> {
        let running = self.procs.lock().unwrap().contains_key(&(panel.to_string(), m.id.clone()));
        if !running {
            self.spawn(panel, m, cwd).await?;
        }
        self.update_member(panel, &m.id, |m| m.status = MemberStatus::Thinking);
        self.write(panel, &m.id, json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })).await
    }

    async fn spawn(self: &Arc<Self>, panel: &str, m: &Member, cwd: &str) -> Result<(), String> {
        let env = self.env().await;
        let bin = env.claude.clone().ok_or("找不到 claude 指令")?;
        let prompt = format!(
            "You are running inside Recursion, a desktop workspace where a human works with coding agents. \
             Your alias in this chat is @{}. Your replies are rendered as Markdown in a chat panel, so use Markdown \
             (tables, lists, code blocks) where it helps. Reply in the user's language.",
            m.alias
        );
        let mut cmd = Command::new(bin);
        cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
            .args(["--permission-prompt-tool", "stdio", "--permission-mode", "acceptEdits"])
            .args(["--append-system-prompt", &prompt]);
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
        self.procs.lock().unwrap().insert((panel.to_string(), m.id.clone()), Proc { stdin: Arc::new(tokio::sync::Mutex::new(stdin)), child: child.clone(), generation });

        let base_cost = m.cost_usd;
        let (hub, panel_s, member_s, cwd_s) = (self.clone(), panel.to_string(), m.id.clone(), dir.clone());
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
}

impl ClaudeReader {
    fn new(hub: Arc<Hub>, panel: String, member: String, cwd: String, base_cost: f64) -> Self {
        Self { hub, panel, member, cwd, base_cost, blocks: HashMap::new(), tools: HashMap::new(), streamed_api_ids: HashSet::new() }
    }

    fn new_msg(&mut self, body: Body, streaming: bool) -> String {
        let id = mid();
        self.hub.push_msg(&self.panel, Message { id: id.clone(), ts: now_ms(), from: self.member.clone(), streaming, body });
        id
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
                    hub.perms.lock().unwrap().insert(request_id, PendingPerm { panel: panel.clone(), member: member.clone(), msg_id, input: req["input"].clone(), suggestions: req["permission_suggestions"].clone() });
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
            }
            _ => {}
        }
    }
}
