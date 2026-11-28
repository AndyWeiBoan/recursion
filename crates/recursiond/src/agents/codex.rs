//! Codex driver: `codex app-server` over stdio JSON-RPC.
//!
//! Flow: initialize → initialized → thread/start (or thread/resume) → turn/start per delivery.
//! Approvals arrive as server requests (`item/commandExecution/requestApproval`,
//! `item/fileChange/requestApproval`) and are answered from the permission cards.

use crate::i18n::tr;
use super::{group_prompt, msg, Hub, PendingPerm, Proc};
use crate::workspace::{display_tool_name, AgentKind, summarize_tool, tool_result_text, Body, Member, MemberStatus, PermState, TurnUsage};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

pub struct Rpc {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    pub thread_id: Mutex<Option<String>>,
    pub(super) turn_id: Mutex<Option<String>>,
}

impl Rpc {
    fn new() -> Self {
        Rpc { next_id: AtomicU64::new(1), pending: Mutex::new(HashMap::new()), thread_id: Mutex::new(None), turn_id: Mutex::new(None) }
    }

    async fn call(&self, hub: &Hub, panel: &str, member: &str, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        hub.write(panel, member, json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await?;
        let resp = tokio::time::timeout(std::time::Duration::from_secs(60), rx).await.map_err(|_| format!("Codex did not answer {method}"))?.map_err(|_| "Codex exited".to_string())?;
        if let Some(err) = resp.get("error") {
            return Err(format!("Codex {method} failed: {}", err["message"].as_str().unwrap_or(&err.to_string())));
        }
        Ok(resp["result"].clone())
    }

    /// What Codex runs when nothing is picked: its config's model and effort, else the catalog's
    /// default model and that model's default effort. `model`: the one picked, if any.
    pub async fn defaults(&self, hub: &Hub, panel: &str, member: &str, model: Option<&str>) -> (Option<String>, Option<String>) {
        let cfg = self.call(hub, panel, member, "config/read", json!({})).await.unwrap_or_default();
        let str_of = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let mut name = model.map(str::to_string).or_else(|| str_of(&cfg["config"]["model"]));
        let mut effort = str_of(&cfg["config"]["model_reasoning_effort"]);
        if name.is_none() || effort.is_none() {
            let list = self.call(hub, panel, member, "model/list", json!({})).await.unwrap_or_default();
            let all = list["data"].as_array().cloned().unwrap_or_default();
            let pick = match &name {
                Some(n) => all.iter().find(|x| x["model"].as_str() == Some(n) || x["id"].as_str() == Some(n)),
                None => all.iter().find(|x| x["isDefault"] == true),
            };
            if let Some(x) = pick {
                name = name.or_else(|| str_of(&x["model"]).or_else(|| str_of(&x["id"])));
                effort = effort.or_else(|| str_of(&x["defaultReasoningEffort"]));
            }
        }
        (if model.is_some() { None } else { name }, effort)
    }

    /// Asks Codex to summarise the thread; it runs as a turn and ends with `turn/completed`.
    pub async fn compact(&self, hub: &Hub, panel: &str, member: &str) -> Result<(), String> {
        let thread = self.thread_id.lock().unwrap().clone().ok_or("Codex thread not started yet")?;
        self.call(hub, panel, member, "thread/compact/start", json!({ "threadId": thread })).await.map(|_| ())
    }

    fn resolve(&self, id: u64, resp: Value) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(resp);
        }
    }

    /// `images`: pictures the human attached, shown to Codex with the text.
    pub async fn start_turn(&self, hub: &Arc<Hub>, panel: &str, member: &str, text: &str, images: &[std::path::PathBuf]) -> Result<(), String> {
        let thread = self.thread_id.lock().unwrap().clone().ok_or("Codex thread not started yet")?;
        let mut input = vec![json!({ "type": "text", "text": text, "text_elements": [] })];
        input.extend(images.iter().map(|p| json!({ "type": "localImage", "path": p })));
        let r = self.call(hub, panel, member, "turn/start", json!({ "threadId": thread, "input": input })).await?;
        *self.turn_id.lock().unwrap() = r["turn"]["id"].as_str().map(|s| s.to_string());
        Ok(())
    }

    pub async fn interrupt(&self, hub: &Arc<Hub>, panel: &str, member: &str) {
        let thread = self.thread_id.lock().unwrap().clone();
        let turn = self.turn_id.lock().unwrap().clone();
        if let (Some(thread), Some(turn)) = (thread, turn) {
            let _ = self.call(hub, panel, member, "turn/interrupt", json!({ "threadId": thread, "turnId": turn })).await;
        }
    }
}

/// `/bin/zsh -lc 'node -e …'` → `node -e …`
fn display_command(item: &Value) -> String {
    if let Some(c) = item["commandActions"].as_array().and_then(|a| a.first()).and_then(|a| a["command"].as_str()) {
        return c.to_string();
    }
    let c = item["command"].as_str().unwrap_or("");
    for prefix in ["/bin/zsh -lc ", "/bin/bash -lc ", "bash -lc ", "zsh -lc "] {
        if let Some(rest) = c.strip_prefix(prefix) {
            return rest.trim_matches('\'').to_string();
        }
    }
    c.to_string()
}

fn file_changes(item: &Value, cwd: &str) -> (String, String) {
    let prefix = format!("{}/", cwd.trim_end_matches('/'));
    let changes = item["changes"].as_array().cloned().unwrap_or_default();
    let paths: Vec<String> = changes
        .iter()
        .filter_map(|c| c["path"].as_str())
        .map(|p| p.strip_prefix(&prefix).unwrap_or(p).to_string())
        .collect();
    let diff: String = changes.iter().filter_map(|c| c["diff"].as_str()).collect::<Vec<_>>().join("\n");
    (paths.join(", "), diff.chars().take(8000).collect())
}

impl Hub {
    pub(super) async fn spawn_codex(self: &Arc<Self>, panel: &str, m: &Member, cwd: &str) -> Result<(), String> {
        let env = self.env().await;
        let bin = env.codex.clone().ok_or_else(|| tr!("err.noCli", cli = "codex"))?;
        let dir = if std::path::Path::new(cwd).is_dir() { cwd.to_string() } else { dirs::home_dir().unwrap_or_default().to_string_lossy().to_string() };
        let enabled = self.member_mcp(panel, &m.id);
        // Skills: the ones picked for it (a folder given after `initialize`), and none of those
        // the human put in Codex's own folders (what Codex comes with stays).
        let skills = self.member_skills(panel, &m.id);
        let skills_dir = crate::library::agent_skills_dir(&format!("{panel}-{}", m.id), &skills)?;
        let codex_dir = crate::library::codex_dir(crate::accounts::dir(&self.accounts.resolve(AgentKind::Codex, m.account.as_deref())));
        let own = crate::library::codex_own_skills(&codex_dir, cwd);
        let mut cmd = Command::new(bin);
        // Guidance files reach it from Recursion, only those the human ticked (guides.rs).
        cmd.args(["-c", crate::guides::CODEX_NO_DOCS]);
        if !own.is_empty() {
            // By path: an imported Skill of the same name stays on. (TOML takes JSON's strings.)
            let off: Vec<String> = own.iter().map(|p| format!("{{path={},enabled=false}}", serde_json::to_string(&p.to_string_lossy()).unwrap_or_default())).collect();
            cmd.args(["-c", &format!("skills.config=[{}]", off.join(","))]);
        }
        cmd.arg("app-server").current_dir(&dir).env("PATH", &env.path);
        self.accounts.apply(&mut cmd, m.kind, m.account.as_deref());
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        self.update_member(panel, &m.id, |m| m.status = MemberStatus::Starting);
        let mut child = cmd.spawn().map_err(|e| tr!("err.spawn", cli = "codex", error = e))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let child = Arc::new(tokio::sync::Mutex::new(child));
        let rpc = Arc::new(Rpc::new());
        self.procs.lock().unwrap().insert(
            (panel.to_string(), m.id.clone()),
            Proc { stdin: Arc::new(tokio::sync::Mutex::new(stdin)), child: child.clone(), generation, codex: Some(rpc.clone()), mcp: enabled.clone(), spec: super::spec(m), skills: skills.clone() },
        );

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
        {
            let (hub, panel_s, member_s, rpc) = (self.clone(), panel.to_string(), m.id.clone(), rpc.clone());
            let mut reader = Reader::new(hub.clone(), panel_s.clone(), member_s.clone(), dir.clone(), rpc.clone());
            let cwd_done = dir.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        reader.handle(v).await;
                    }
                }
                // Fail any in-flight calls.
                rpc.pending.lock().unwrap().clear();
                let status = child.lock().await.wait().await.ok();
                let current = hub.procs.lock().unwrap().get(&(panel_s.clone(), member_s.clone())).map(|p| p.generation);
                if current == Some(generation) {
                    hub.procs.lock().unwrap().remove(&(panel_s.clone(), member_s.clone()));
                    hub.todo_release_all(&panel_s, &member_s);
                    let busy = hub.with_ws(&panel_s, |ws| ws.members.iter().any(|m| m.id == member_s && m.status != MemberStatus::Idle));
                    if busy {
                        let err = err_buf.lock().unwrap().trim().to_string();
                        let code = status.and_then(|s| s.code()).map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                        let detail = if err.is_empty() { String::new() } else { format!("\n\n```\n{}\n```", err.chars().take(1500).collect::<String>()) };
                        hub.system(&panel_s, &member_s, format!("{}{detail}", tr!("agent.crashed", cli = "Codex", code = code)), true);
                        hub.update_member(&panel_s, &member_s, |m| m.status = MemberStatus::Idle);
                        super::spawn_member_done(hub.clone(), panel_s.clone(), member_s.clone(), cwd_done.clone());
                    }
                }
            });
        }

        // Handshake + thread.
        rpc.call(self, panel, &m.id, "initialize", json!({ "clientInfo": { "name": "recursion", "title": "Recursion", "version": env!("CARGO_PKG_VERSION") }, "capabilities": null }))
            .await?;
        self.write(panel, &m.id, json!({ "jsonrpc": "2.0", "method": "initialized" })).await?;
        rpc.call(self, panel, &m.id, "skills/extraRoots/set", json!({ "extraRoots": [skills_dir.join("skills")] })).await?;
        let (approval, sandbox) = m.mode.codex();
        // Recursion's own server and the ones the session turned on; Codex's own others are switched off.
        let mut mcp = self.mcps.codex_config(&enabled, crate::accounts::dir(&self.accounts.resolve(AgentKind::Codex, m.account.as_deref())));
        mcp.insert("recursion".into(), json!({ "url": self.mcp_url(panel, &m.id) }));
        let mut params = json!({
            "cwd": dir,
            "approvalPolicy": approval,
            "sandbox": sandbox,
            "developerInstructions": group_prompt(&m.alias),
            "config": { "mcp_servers": mcp },
        });
        if let Some(model) = &m.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = &m.effort {
            params["config"]["model_reasoning_effort"] = json!(effort);
        }
        let result = match &m.cli_session_id {
            Some(tid) => {
                let mut p = params.clone();
                // A thread keeps the model and effort it last ran with: back to "default" has
                // to name what the default is.
                if m.model.is_none() || m.effort.is_none() {
                    let (model, effort) = rpc.defaults(self, panel, &m.id, m.model.as_deref()).await;
                    if let (None, Some(model)) = (&m.model, model) {
                        p["model"] = json!(model);
                    }
                    if let (None, Some(effort)) = (&m.effort, effort) {
                        p["config"]["model_reasoning_effort"] = json!(effort);
                    }
                }
                p["threadId"] = json!(tid);
                p["excludeTurns"] = json!(true);
                match rpc.call(self, panel, &m.id, "thread/resume", p).await {
                    Ok(r) => r,
                    // The old thread may be gone (e.g. Codex data cleared): start fresh.
                    Err(_) => rpc.call(self, panel, &m.id, "thread/start", params.clone()).await?,
                }
            }
            None => rpc.call(self, panel, &m.id, "thread/start", params.clone()).await?,
        };
        let tid = result["thread"]["id"].as_str().ok_or("Codex returned no thread id")?.to_string();
        self.codex_started(panel, &m.id, &result, enabled.clone());
        if let Some(model) = result["model"].as_str().filter(|s| !s.is_empty()) {
            let model = model.to_string();
            let effort = result["reasoningEffort"].as_str().map(str::to_string);
            self.update_member(panel, &m.id, |m| {
                m.running_model = Some(model);
                m.running_effort = effort;
            });
        }
        // Rewound: drop the turns from that one on. If Codex can't, it starts a new thread
        // and is fed the chat again.
        let mut tid = tid;
        if let Some(turn) = self.rewind_point(panel, &m.id).filter(|_| m.cli_session_id.as_deref() == Some(&tid)) {
            let reverted = rpc.call(self, panel, &m.id, "thread/revert", json!({ "threadId": tid, "beforeTurnId": turn })).await;
            if reverted.is_err() {
                let r = rpc.call(self, panel, &m.id, "thread/start", params).await?;
                tid = r["thread"]["id"].as_str().ok_or("Codex returned no thread id")?.to_string();
            }
            self.rewind_taken(panel, &m.id, reverted.is_err());
        }
        *rpc.thread_id.lock().unwrap() = Some(tid.clone());
        self.update_member(panel, &m.id, |m| m.cli_session_id = Some(tid));
        Ok(())
    }
}

struct Reader {
    hub: Arc<Hub>,
    panel: String,
    member: String,
    cwd: String,
    rpc: Arc<Rpc>,
    /// Codex item id → our message id
    items: HashMap<String, String>,
    turn_text: String,
    turn_text_ids: Vec<String>,
    /// Sum of per-request usage (`tokenUsage.last`) since the turn started.
    turn_usage: TurnUsage,
}

impl Reader {
    fn new(hub: Arc<Hub>, panel: String, member: String, cwd: String, rpc: Arc<Rpc>) -> Self {
        Reader { hub, panel, member, cwd, rpc, items: HashMap::new(), turn_text: String::new(), turn_text_ids: Vec::new(), turn_usage: TurnUsage::default() }
    }

    fn push(&mut self, item_id: &str, body: Body, streaming: bool) -> String {
        let is_text = matches!(body, Body::Text { .. });
        let m = self.hub.push_msg(&self.panel, msg(&self.member, body, streaming));
        if is_text {
            self.turn_text_ids.push(m.id.clone());
        }
        self.items.insert(item_id.to_string(), m.id.clone());
        m.id
    }

    async fn handle(&mut self, v: Value) {
        let hub = self.hub.clone();
        let (panel, member) = (self.panel.clone(), self.member.clone());
        let method = v["method"].as_str().unwrap_or("").to_string();
        tracing::trace!(line = %v.to_string().chars().take(300).collect::<String>(), "codex");

        // Response to one of our calls.
        if method.is_empty() {
            if let Some(id) = v["id"].as_u64() {
                self.rpc.resolve(id, v);
            }
            return;
        }
        let params = &v["params"];

        // Request from the server (needs an answer).
        if !v["id"].is_null() {
            let rpc_id = v["id"].clone();
            tracing::debug!(%method, params = %params.to_string().chars().take(400).collect::<String>(), "codex server request");
            match method.as_str() {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    let item_id = params["itemId"].as_str().unwrap_or("").to_string();
                    let (tool, summary, mut detail) = if method.starts_with("item/command") {
                        ("Shell".to_string(), display_command(params), String::new())
                    } else {
                        let existing = self.items.get(&item_id).cloned();
                        let (sum, det) = existing
                            .and_then(|id| hub.with_ws(&panel, |ws| ws.message_mut(&id).map(|m| m.body.clone())))
                            .and_then(|b| if let Body::Tool { summary, detail, .. } = b { Some((summary, detail)) } else { None })
                            .unwrap_or_default();
                        ("Edit".to_string(), if sum.is_empty() { tr!("tool.editFiles") } else { sum }, det)
                    };
                    if let Some(reason) = params["reason"].as_str() {
                        detail = if detail.is_empty() { reason.to_string() } else { format!("{reason}\n\n{detail}") };
                    }
                    let request_id = crate::model::new_id();
                    let waiting = if summary.is_empty() { tool.clone() } else { summary.clone() };
                    let msg_id = hub.push_msg(&panel, msg(&member, Body::Permission { request_id: request_id.clone(), tool, summary, detail, state: PermState::Pending }, false)).id;
                    hub.perms.lock().unwrap().insert(
                        request_id,
                        PendingPerm { panel: panel.clone(), member: member.clone(), msg_id, input: Value::Null, suggestions: Value::Null, codex_request: Some(rpc_id) },
                    );
                    hub.update_member(&panel, &member, |m| {
                        m.status = MemberStatus::WaitingPermission;
                        m.waiting_for = Some(waiting);
                    });
                }
                // Codex asks before each MCP tool call. Our own server only touches this session's
                // documents, and the others are the ones the human turned on for it: say yes.
                "mcpServer/elicitation/request" => {
                    let _ = hub.write(&panel, &member, json!({ "jsonrpc": "2.0", "id": rpc_id, "result": { "action": "accept", "content": {} } })).await;
                }
                _ => {
                    // Not supported yet (user input prompts, MCP elicitations…): decline so the turn can continue.
                    let _ = hub.write(&panel, &member, json!({ "jsonrpc": "2.0", "id": rpc_id, "error": { "code": -32601, "message": "not supported by Recursion" } })).await;
                }
            }
            return;
        }

        match method.as_str() {
            "item/started" => {
                let item = &params["item"];
                let id = item["id"].as_str().unwrap_or("").to_string();
                match item["type"].as_str().unwrap_or("") {
                    "agentMessage" => {
                        if !self.turn_text.is_empty() {
                            self.turn_text.push_str("\n\n");
                        }
                        self.push(&id, Body::Text { text: item["text"].as_str().unwrap_or("").to_string() }, true);
                    }
                    "commandExecution" => {
                        self.push(&id, Body::Tool { tool_use_id: id.clone(), name: "Shell".into(), summary: display_command(item), detail: item["command"].as_str().unwrap_or("").to_string(), result: None, is_error: false }, true);
                    }
                    "fileChange" => {
                        let (summary, detail) = file_changes(item, &self.cwd);
                        self.push(&id, Body::Tool { tool_use_id: id.clone(), name: "Edit".into(), summary, detail, result: None, is_error: false }, true);
                    }
                    "mcpToolCall" => {
                        let name = format!("{}.{}", item["server"].as_str().unwrap_or("mcp"), item["tool"].as_str().unwrap_or("tool"));
                        let (summary, detail) = summarize_tool(&name, &item["arguments"], &self.cwd);
                        self.push(&id, Body::Tool { tool_use_id: id.clone(), name: display_tool_name(&name), summary, detail, result: None, is_error: false }, true);
                    }
                    "webSearch" => {
                        self.push(&id, Body::Tool { tool_use_id: id.clone(), name: "WebSearch".into(), summary: item["query"].as_str().unwrap_or("").to_string(), detail: String::new(), result: None, is_error: false }, true);
                    }
                    _ => {}
                }
            }
            "item/agentMessage/delta" => {
                let item = params["itemId"].as_str().unwrap_or("");
                let delta = params["delta"].as_str().unwrap_or("").to_string();
                if let Some(id) = self.items.get(item).cloned() {
                    self.turn_text.push_str(&delta);
                    hub.with_ws(&panel, |ws| {
                        if let Some(crate::workspace::Message { body: Body::Text { text }, .. }) = ws.message_mut(&id) {
                            text.push_str(&delta);
                        }
                    });
                    hub.emit(json!({ "t": "ws_delta", "panel": panel, "id": id, "text": delta }));
                }
            }
            "item/completed" => {
                let item = &params["item"];
                let item_id = item["id"].as_str().unwrap_or("");
                let Some(id) = self.items.get(item_id).cloned() else { return };
                match item["type"].as_str().unwrap_or("") {
                    "agentMessage" => {
                        let full = item["text"].as_str().unwrap_or("").to_string();
                        hub.update_msg(&panel, &id, |m| {
                            m.streaming = false;
                            if let Body::Text { text } = &mut m.body {
                                if !full.is_empty() {
                                    *text = full;
                                }
                            }
                        });
                    }
                    "commandExecution" => {
                        let out = item["aggregatedOutput"].as_str().unwrap_or("").to_string();
                        let code = item["exitCode"].as_i64();
                        let status = item["status"].as_str().unwrap_or("");
                        hub.update_msg(&panel, &id, |m| {
                            m.streaming = false;
                            if let Body::Tool { result, is_error, .. } = &mut m.body {
                                *result = Some(if out.is_empty() && status == "declined" { tr!("perm.denied") } else { out.chars().take(4000).collect() });
                                *is_error = code.map(|c| c != 0).unwrap_or(false) || status == "failed" || status == "declined";
                            }
                        });
                    }
                    "fileChange" => {
                        let status = item["status"].as_str().unwrap_or("").to_string();
                        let (summary, detail) = file_changes(item, &self.cwd);
                        hub.update_msg(&panel, &id, |m| {
                            m.streaming = false;
                            if let Body::Tool { result, is_error, summary: s, detail: d, .. } = &mut m.body {
                                if !summary.is_empty() {
                                    *s = summary;
                                    *d = detail;
                                }
                                *result = Some(match status.as_str() { "completed" => tr!("tool.applied"), "declined" => tr!("perm.denied"), other => other.to_string() });
                                *is_error = status == "failed" || status == "declined";
                            }
                        });
                    }
                    _ => {
                        // MCP results are {content: [{type: "text", text}]}; show the text, not the envelope.
                        let r = &item["result"];
                        let text = if r["content"].is_array() { tool_result_text(&r["content"]) } else if r.is_null() { item["error"]["message"].as_str().unwrap_or("").to_string() } else { r.to_string() };
                        let text: String = text.chars().take(2000).collect();
                        // Failed: the call itself (status / error) or the tool's own answer (isError).
                        let failed = matches!(item["status"].as_str(), Some("failed" | "declined")) || !item["error"].is_null() || r["isError"].as_bool().unwrap_or(false);
                        hub.update_msg(&panel, &id, |m| {
                            m.streaming = false;
                            if let Body::Tool { result, is_error, .. } = &mut m.body {
                                *result = Some(text);
                                *is_error = failed;
                            }
                        });
                    }
                }
            }
            "account/rateLimits/updated" => {
                let rl = &params["rateLimits"];
                let mut usage = Vec::new();
                let mut resets = std::collections::BTreeMap::new();
                for (key, w) in [("primary", &rl["primary"]), ("secondary", &rl["secondary"])] {
                    if let Some(pct) = w["usedPercent"].as_f64() {
                        let mins = w["windowDurationMins"].as_u64().unwrap_or(0);
                        let name = if mins == 300 { "five_hour" } else if mins == 10080 { "seven_day" } else { key };
                        usage.push((name.to_string(), pct / 100.0));
                        if let Some(t) = w["resetsAt"].as_u64() {
                            resets.insert(name.to_string(), t);
                        }
                    }
                }
                hub.update_member(&panel, &member, |m| usage.into_iter().for_each(|(k, u)| {
                    m.usage.insert(k, u);
                }));
                // The same numbers belong to the account (Claude's path does this too).
                let m = hub.with_ws(&panel, |ws| ws.members.iter().find(|m| m.id == member).cloned());
                if let Some(m) = m {
                    if hub.accounts.record_usage(&hub.accounts.resolve(m.kind, m.account.as_deref()), &m.usage, &resets) {
                        hub.emit_accounts();
                    }
                }
            }
            "error" => {
                let text = params["error"]["message"].as_str().or(params["message"].as_str()).unwrap_or("Codex reported an error").to_string();
                hub.system(&panel, &member, text, true);
            }
            "thread/tokenUsage/updated" => {
                let last = &params["tokenUsage"]["last"];
                let n = |k: &str| last[k].as_u64().unwrap_or(0);
                let (used, window) = (n("totalTokens"), params["tokenUsage"]["modelContextWindow"].as_u64());
                hub.update_member(&panel, &member, |m| {
                    if used > 0 {
                        m.context_used = used;
                    }
                    if let Some(w) = window {
                        m.context_window = w;
                    }
                });
                self.turn_usage.input += n("inputTokens");
                self.turn_usage.cached += n("cachedInputTokens");
                self.turn_usage.output += n("outputTokens");
            }
            "turn/completed" if hub.is_compacting(&panel, &member) => {
                *self.rpc.turn_id.lock().unwrap() = None;
                self.items.clear();
                let error = (params["turn"]["status"] == "failed").then(|| params["turn"]["error"]["message"].as_str().unwrap_or("Codex turn failed").to_string());
                super::spawn_compact_done(hub, panel, member, None, error);
            }
            "turn/completed" => {
                let turn = &params["turn"];
                let usage = std::mem::take(&mut self.turn_usage);
                hub.finish_turn(&panel, &member, (usage.input + usage.output > 0).then_some(usage));
                *self.rpc.turn_id.lock().unwrap() = None;
                if turn["status"] == "failed" {
                    let text = turn["error"]["message"].as_str().unwrap_or("Codex turn failed").to_string();
                    hub.system(&panel, &member, text, true);
                }
                // Anything still marked streaming from this turn is done.
                let open: Vec<String> = self.items.values().cloned().collect();
                for id in open {
                    let streaming = hub.with_ws(&panel, |ws| ws.message_mut(&id).map(|m| m.streaming).unwrap_or(false));
                    if streaming {
                        hub.update_msg(&panel, &id, |m| m.streaming = false);
                    }
                }
                self.items.clear();
                hub.update_member(&panel, &member, |m| m.status = MemberStatus::Idle);
                let text = std::mem::take(&mut self.turn_text);
                let ids = std::mem::take(&mut self.turn_text_ids);
                spawn_turn_ended(hub, panel, member, text, ids, self.cwd.clone());
            }
            _ => {}
        }
    }
}

/// Kept as a plain fn so the reader's future type doesn't recurse into `turn_ended` → `deliver` → `spawn_codex`.
fn spawn_turn_ended(hub: Arc<Hub>, panel: String, member: String, text: String, ids: Vec<String>, cwd: String) {
    tokio::spawn(async move { hub.turn_ended(panel, member, text, ids, cwd).await });
}

// From issues #6 and #7 (fault injection, no CLI or account needed).
#[cfg(test)]
mod issue_regressions {
    use super::*;

    #[tokio::test]
    async fn rejected_turn_does_not_consume_unseen_messages() {
        let hub = Hub::new();
        let panel = crate::model::new_id();
        let member: Member = serde_json::from_value(json!({"id":"audit","alias":"audit","kind":"codex"})).unwrap();
        hub.with_ws(&panel, |ws| ws.members.push(member.clone()));
        hub.push_msg(&panel, msg("you", Body::Text { text: "Remember the audit marker 7429".into() }, false));
        let mut child = Command::new("/bin/cat").stdin(Stdio::piped()).stdout(Stdio::null()).kill_on_drop(true).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let child = Arc::new(tokio::sync::Mutex::new(child));
        let rpc = Arc::new(Rpc::new());
        *rpc.thread_id.lock().unwrap() = Some("test-thread".into());
        hub.procs.lock().unwrap().insert((panel.clone(), member.id.clone()), Proc { stdin: Arc::new(tokio::sync::Mutex::new(stdin)), child: child.clone(), generation: 1, codex: Some(rpc.clone()), mcp: vec![], spec: crate::agents::spec(&member), skills: vec![] });
        let responder = tokio::spawn(async move {
            loop {
                let id = rpc.pending.lock().unwrap().keys().next().copied();
                if let Some(id) = id {
                    rpc.resolve(id, json!({"id": id, "error": {"code": -32000, "message": "test: turn rejected"}}));
                    break;
                }
                tokio::task::yield_now().await;
            }
        });
        let result = hub.deliver(&panel, &member, "/tmp").await;
        responder.await.unwrap();
        child.lock().await.kill().await.unwrap();
        assert!(result.unwrap_err().contains("turn rejected"));
        let seen = hub.with_ws(&panel, |ws| ws.members[0].seen);
        assert_eq!(seen, 0, "a rejected turn must leave the human's message to be delivered again");
    }

    async fn check_mcp_failure(result: Value, error: Value) {
        let hub = Hub::new();
        let panel = crate::model::new_id();
        let mut reader = Reader::new(hub.clone(), panel.clone(), "audit".into(), "/tmp".into(), Arc::new(Rpc::new()));
        reader.handle(json!({"method":"item/started","params":{"item":{"type":"mcpToolCall","id":"tool1","server":"recursion","tool":"doc_read","arguments":{"id":"missing"}}}})).await;
        reader.handle(json!({"method":"item/completed","params":{"item":{"type":"mcpToolCall","id":"tool1","status":"failed","result":result,"error":error}}})).await;
        let state = hub.snapshot(&panel);
        let row = state["ws"]["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(row["streaming"], false);
        assert_eq!(row["is_error"], true, "failed MCP row: {row}");
    }

    #[tokio::test]
    async fn mcp_transport_failure_is_marked_error() {
        check_mcp_failure(Value::Null, json!({"message":"test transport failure"})).await;
    }

    #[tokio::test]
    async fn mcp_result_error_is_marked_error() {
        check_mcp_failure(json!({"content":[{"type":"text","text":"document not found"}],"isError":true}), Value::Null).await;
    }
}
