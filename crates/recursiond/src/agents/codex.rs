//! Codex driver: `codex app-server` over stdio JSON-RPC.
//!
//! Flow: initialize → initialized → thread/start (or thread/resume) → turn/start per delivery.
//! Approvals arrive as server requests (`item/commandExecution/requestApproval`,
//! `item/fileChange/requestApproval`) and are answered from the permission cards.

use super::{group_prompt, msg, Hub, PendingPerm, Proc};
use crate::workspace::{Body, Member, MemberStatus, PermState};
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
    turn_id: Mutex<Option<String>>,
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
        let resp = tokio::time::timeout(std::time::Duration::from_secs(60), rx).await.map_err(|_| format!("Codex 沒有回應 {method}"))?.map_err(|_| "Codex 已結束".to_string())?;
        if let Some(err) = resp.get("error") {
            return Err(format!("Codex {method} 失敗：{}", err["message"].as_str().unwrap_or(&err.to_string())));
        }
        Ok(resp["result"].clone())
    }

    fn resolve(&self, id: u64, resp: Value) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(resp);
        }
    }

    pub async fn start_turn(&self, hub: &Arc<Hub>, panel: &str, member: &str, text: &str) -> Result<(), String> {
        let thread = self.thread_id.lock().unwrap().clone().ok_or("Codex thread 尚未建立")?;
        let r = self
            .call(hub, panel, member, "turn/start", json!({ "threadId": thread, "input": [{ "type": "text", "text": text, "text_elements": [] }] }))
            .await?;
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
        let bin = env.codex.clone().ok_or("找不到 codex 指令")?;
        let dir = if std::path::Path::new(cwd).is_dir() { cwd.to_string() } else { dirs::home_dir().unwrap_or_default().to_string_lossy().to_string() };
        let mut cmd = Command::new(bin);
        cmd.arg("app-server").current_dir(&dir).env("PATH", &env.path);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        self.update_member(panel, &m.id, |m| m.status = MemberStatus::Starting);
        let mut child = cmd.spawn().map_err(|e| format!("無法啟動 codex：{e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let child = Arc::new(tokio::sync::Mutex::new(child));
        let rpc = Arc::new(Rpc::new());
        self.procs.lock().unwrap().insert(
            (panel.to_string(), m.id.clone()),
            Proc { stdin: Arc::new(tokio::sync::Mutex::new(stdin)), child: child.clone(), generation, codex: Some(rpc.clone()) },
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
                    let busy = hub.with_ws(&panel_s, |ws| ws.members.iter().any(|m| m.id == member_s && m.status != MemberStatus::Idle));
                    if busy {
                        let err = err_buf.lock().unwrap().trim().to_string();
                        let code = status.and_then(|s| s.code()).map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                        let detail = if err.is_empty() { String::new() } else { format!("\n\n```\n{}\n```", err.chars().take(1500).collect::<String>()) };
                        hub.system(&panel_s, &member_s, format!("Codex 意外結束（exit {code}）。下一則訊息會自動接續對話。{detail}"), true);
                        hub.update_member(&panel_s, &member_s, |m| m.status = MemberStatus::Idle);
                    }
                }
            });
        }

        // Handshake + thread.
        rpc.call(self, panel, &m.id, "initialize", json!({ "clientInfo": { "name": "recursion", "title": "Recursion", "version": env!("CARGO_PKG_VERSION") }, "capabilities": null }))
            .await?;
        self.write(panel, &m.id, json!({ "jsonrpc": "2.0", "method": "initialized" })).await?;
        let mut params = json!({
            "cwd": dir,
            "approvalPolicy": "on-request",
            "sandbox": "workspace-write",
            "developerInstructions": group_prompt(&m.alias),
        });
        if let Some(model) = &m.model {
            params["model"] = json!(model);
        }
        let result = match &m.cli_session_id {
            Some(tid) => {
                let mut p = params.clone();
                p["threadId"] = json!(tid);
                p["excludeTurns"] = json!(true);
                match rpc.call(self, panel, &m.id, "thread/resume", p).await {
                    Ok(r) => r,
                    // The old thread may be gone (e.g. Codex data cleared): start fresh.
                    Err(_) => rpc.call(self, panel, &m.id, "thread/start", params).await?,
                }
            }
            None => rpc.call(self, panel, &m.id, "thread/start", params).await?,
        };
        let tid = result["thread"]["id"].as_str().ok_or("Codex 沒有回傳 thread id")?.to_string();
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
}

impl Reader {
    fn new(hub: Arc<Hub>, panel: String, member: String, cwd: String, rpc: Arc<Rpc>) -> Self {
        Reader { hub, panel, member, cwd, rpc, items: HashMap::new(), turn_text: String::new(), turn_text_ids: Vec::new() }
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
                        ("Edit".to_string(), if sum.is_empty() { "修改檔案".into() } else { sum }, det)
                    };
                    if let Some(reason) = params["reason"].as_str() {
                        detail = if detail.is_empty() { reason.to_string() } else { format!("{reason}\n\n{detail}") };
                    }
                    let request_id = crate::model::new_id();
                    let msg_id = hub.push_msg(&panel, msg(&member, Body::Permission { request_id: request_id.clone(), tool, summary, detail, state: PermState::Pending }, false)).id;
                    hub.perms.lock().unwrap().insert(
                        request_id,
                        PendingPerm { panel: panel.clone(), member: member.clone(), msg_id, input: Value::Null, suggestions: Value::Null, codex_request: Some(rpc_id) },
                    );
                    hub.update_member(&panel, &member, |m| m.status = MemberStatus::WaitingPermission);
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
                        let args: String = item["arguments"].to_string().chars().take(120).collect();
                        self.push(&id, Body::Tool { tool_use_id: id.clone(), name, summary: args, detail: item["arguments"].to_string(), result: None, is_error: false }, true);
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
                                *result = Some(if out.is_empty() && status == "declined" { "已拒絕".into() } else { out.chars().take(4000).collect() });
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
                                *result = Some(match status.as_str() { "completed" => "已套用".into(), "declined" => "已拒絕".into(), other => other.to_string() });
                                *is_error = status == "failed" || status == "declined";
                            }
                        });
                    }
                    _ => {
                        let text: String = item["result"].to_string().chars().take(2000).collect();
                        hub.update_msg(&panel, &id, |m| {
                            m.streaming = false;
                            if let Body::Tool { result, .. } = &mut m.body {
                                *result = Some(text);
                            }
                        });
                    }
                }
            }
            "account/rateLimits/updated" => {
                let rl = &params["rateLimits"];
                let mut usage = Vec::new();
                for (key, w) in [("primary", &rl["primary"]), ("secondary", &rl["secondary"])] {
                    if let Some(pct) = w["usedPercent"].as_f64() {
                        let mins = w["windowDurationMins"].as_u64().unwrap_or(0);
                        let name = if mins == 300 { "five_hour" } else if mins == 10080 { "seven_day" } else { key };
                        usage.push((name.to_string(), pct / 100.0));
                    }
                }
                hub.update_member(&panel, &member, |m| usage.into_iter().for_each(|(k, u)| {
                    m.usage.insert(k, u);
                }));
            }
            "error" => {
                let text = params["error"]["message"].as_str().or(params["message"].as_str()).unwrap_or("Codex 回報錯誤").to_string();
                hub.system(&panel, &member, text, true);
            }
            "turn/completed" => {
                let turn = &params["turn"];
                *self.rpc.turn_id.lock().unwrap() = None;
                if turn["status"] == "failed" {
                    let text = turn["error"]["message"].as_str().unwrap_or("Codex 這一輪失敗了").to_string();
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
