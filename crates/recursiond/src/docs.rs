//! Session documents and the built-in MCP server that lets agents write them.
//!
//! Each agent process gets its own MCP URL (`/mcp/<key>`, see `Hub::mcp_url`), so a tool call
//! knows which chat and which agent it came from without the agent having to say. The server
//! speaks MCP's streamable-HTTP transport in its simplest form: one JSON-RPC POST, one JSON reply.

use crate::model::Doc;
use crate::workspace::now_ms;
use crate::App;
use axum::extract::{Path, State as AxState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_HTML: usize = 2 * 1024 * 1024;

fn dir(session: &str) -> PathBuf {
    crate::home().join("docs").join(session)
}

fn file(session: &str, doc: &str) -> PathBuf {
    dir(session).join(format!("{doc}.html"))
}

pub fn read(session: &str, doc: &str) -> Option<String> {
    // Ids are ours (alphanumeric), but they also arrive from clients and agents.
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric());
    if !ok(session) || !ok(doc) {
        return None;
    }
    std::fs::read_to_string(file(session, doc)).ok()
}

pub fn delete_session(session: &str) {
    let _ = std::fs::remove_dir_all(dir(session));
}

fn tools() -> Value {
    let mut all = doc_tools();
    all.as_array_mut().unwrap().extend(crate::browse::tools());
    all.as_array_mut().unwrap().extend(crate::agents::poll_tools().as_array().unwrap().clone());
    all
}

fn doc_tools() -> Value {
    json!([
        {
            "name": "doc_write",
            "description": "Save an HTML document to this chat's session and show it to the human as a card that opens in its own panel. \
Use it for anything long or visual: reports, big tables, comparisons, diagrams (inline SVG, or Mermaid from a CDN), dashboards, \
interactive explainers. Write one complete, self-contained HTML page; scripts and CDN libraries are allowed. \
Pass `id` to replace an existing document (same card, updated content) instead of creating a new one. \
The page can talk back: `recursion.reply(data)` sends `data` (string or JSON) to you as the human's next message, \
so send only what the human chose or entered (e.g. {\"database\": \"SQLite\"}), not your own text; you answer after it arrives. \
`recursion.sendPrompt(text)` posts `text` to the whole chat as the human. Call them from a button or form submit, \
e.g. to collect choices or answers from a form you designed. \
Keep your chat reply short and point to the document.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short title shown on the card and panel" },
                    "html": { "type": "string", "description": "A complete HTML document" },
                    "id": { "type": "string", "description": "Existing document id to replace; omit to create a new one" }
                },
                "required": ["title", "html"]
            }
        },
        {
            "name": "doc_read",
            "description": "Read the HTML of a document in this session (yours or another agent's).",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "doc_list",
            "description": "List the documents in this session: id, title, last author.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

fn text(t: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": is_error })
}

impl App {
    async fn call_tool(&self, panel: &str, member: &str, alias: &str, name: &str, args: &Value) -> Value {
        if name.starts_with("browse_") {
            return self.browse(panel, member, name, args).await;
        }
        if name.starts_with("poll_") {
            return match self.hub.poll_tool(panel, member, name, args) {
                Ok(t) => text(t, false),
                Err(e) => text(e, true),
            };
        }
        let Some(session) = self.state.lock().unwrap().session_of_panel(panel).map(|s| s.id.clone()) else {
            return text("This chat is no longer open.", true);
        };
        let docs: Vec<Doc> = self.state.lock().unwrap().sessions.iter().find(|s| s.id == session).map(|s| s.docs.clone()).unwrap_or_default();
        match name {
            "doc_list" => {
                if docs.is_empty() {
                    return text("No documents yet.", false);
                }
                text(docs.iter().map(|d| format!("{} — {} (by @{}, rev {})", d.id, d.title, d.author, d.rev)).collect::<Vec<_>>().join("\n"), false)
            }
            "doc_read" => {
                let id = args["id"].as_str().unwrap_or("");
                match docs.iter().any(|d| d.id == id).then(|| read(&session, id)).flatten() {
                    Some(html) => text(html, false),
                    None => text(format!("No document with id `{id}`. Use doc_list to see what exists."), true),
                }
            }
            "doc_write" => {
                let title = args["title"].as_str().unwrap_or("").trim().to_string();
                let html = args["html"].as_str().unwrap_or("");
                if title.is_empty() || html.trim().is_empty() {
                    return text("Both `title` and `html` are required.", true);
                }
                if html.len() > MAX_HTML {
                    return text(format!("Document too large ({} bytes, max {MAX_HTML}).", html.len()), true);
                }
                let id = args["id"].as_str().filter(|s| !s.is_empty());
                if let Some(id) = id {
                    if !docs.iter().any(|d| d.id == id) {
                        return text(format!("No document with id `{id}`. Omit `id` to create a new one."), true);
                    }
                }
                let written = self.state.lock().unwrap().write_doc(&session, id, title, alias.to_string(), panel, member, now_ms());
                let (doc, created) = match written {
                    Ok(r) => r,
                    Err(e) => return text(e.to_string(), true),
                };
                let path = file(&session, &doc.id);
                let saved = std::fs::create_dir_all(dir(&session)).and_then(|_| std::fs::write(&path, html));
                if let Err(e) = saved {
                    return text(format!("Could not save: {e}"), true);
                }
                if created {
                    let _ = self.state.lock().unwrap().open_doc(panel, &doc.id, false);
                }
                self.mark_changed();
                self.hub.post_doc(panel, member, &doc, !created);
                self.hub.record_artifact(panel, crate::workspace::ArtifactKind::Doc, &doc.id, &doc.title, member);
                let verb = if created { "Created" } else { "Updated" };
                text(
                    format!(
                        "{verb} document `{}` (rev {}). The human sees it as a card in the chat and it is open in a panel next to it. \
To revise it, call doc_write again with id \"{}\".",
                        doc.id, doc.rev, doc.id
                    ),
                    false,
                )
            }
            _ => text(format!("Unknown tool {name}"), true),
        }
    }

    async fn rpc(&self, panel: &str, member: &str, alias: &str, req: &Value) -> Option<Value> {
        let id = req.get("id")?.clone(); // notifications get no reply
        let params = &req["params"];
        let result = match req["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "recursion", "version": env!("CARGO_PKG_VERSION") }
            }),
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tools() }),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("");
                self.call_tool(panel, member, alias, name, &params["arguments"]).await
            }
            m => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {m}") } })),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }
}

const MAX_PAGE_MESSAGE: usize = 8000;
const PAGE_MESSAGE_GAP: std::time::Duration = std::time::Duration::from_secs(2);

impl App {
    /// A doc page called `recursion.reply` / `recursion.sendPrompt`. It is posted as the human
    /// (the human clicked something in it), labelled with the doc, so nothing happens unseen.
    pub async fn doc_action(&self, session: &str, doc: &str, action: &str, data: &str) -> Result<(), String> {
        let (d, cwd) = {
            let st = self.state.lock().unwrap();
            let s = st.sessions.iter().find(|s| s.id == session).ok_or("找不到 session")?;
            (s.docs.iter().find(|d| d.id == doc).cloned().ok_or("找不到文件")?, s.cwd.clone())
        };
        // A page script could loop; one message per doc every couple of seconds is plenty for clicks.
        {
            let mut last = self.doc_last.lock().unwrap();
            let now = std::time::Instant::now();
            if last.get(doc).is_some_and(|t| now.duration_since(*t) < PAGE_MESSAGE_GAP) {
                return Err(format!("文件「{}」送訊息太快了，已略過", d.title));
            }
            last.insert(doc.to_string(), now);
        }
        let data = data.trim();
        if data.is_empty() {
            return Ok(());
        }
        if data.chars().count() > MAX_PAGE_MESSAGE {
            return Err(format!("文件「{}」送出的內容太長（上限 {MAX_PAGE_MESSAGE} 字）", d.title));
        }
        let chat_alive = self.state.lock().unwrap().panels.get(&d.chat).is_some_and(|p| p.kind == crate::model::PanelKind::Workspace);
        if !chat_alive {
            return Err(format!("寫這份文件的群聊已經關掉了，「{}」的訊息沒有送出", d.title));
        }
        let body = if data.contains('\n') || data.starts_with('{') || data.starts_with('[') { format!("```\n{data}\n```") } else { data.to_string() };
        let text = match action {
            "reply" => {
                // To whoever wrote it, under their current name; everyone if they left.
                let to = self.hub.alias_of(&d.chat, &d.member).map(|a| format!("@{a} ")).unwrap_or_default();
                format!("{to}（來自文件「{}」）\n{body}", d.title)
            }
            "prompt" => format!("（來自文件「{}」）\n{body}", d.title),
            other => return Err(format!("未知的文件動作：{other}")),
        };
        let via = crate::workspace::DocVia { doc: d.id.clone(), title: d.title.clone(), data: data.to_string() };
        self.hub.send(&d.chat, text, cwd, Some(via), None).await
    }
}

pub async fn mcp(AxState(app): AxState<Arc<App>>, Path(key): Path<String>, body: String) -> Response {
    let Some((panel, member, alias)) = app.hub.mcp_caller(&key) else {
        return (StatusCode::NOT_FOUND, "unknown MCP key").into_response();
    };
    tracing::debug!(%alias, body = %body.chars().take(300).collect::<String>(), "mcp request");
    let Ok(req) = serde_json::from_str::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "bad JSON").into_response();
    };
    let reply = match &req {
        Value::Array(batch) => {
            let mut out = vec![];
            for r in batch {
                out.extend(app.rpc(&panel, &member, &alias, r).await);
            }
            (!out.is_empty()).then(|| Value::Array(out))
        }
        r => app.rpc(&panel, &member, &alias, r).await,
    };
    match reply {
        Some(v) => Json(v).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// No server-initiated stream: GET (SSE) is not offered.
pub async fn mcp_get() -> StatusCode {
    tracing::debug!("mcp GET (no SSE offered)");
    StatusCode::METHOD_NOT_ALLOWED
}
