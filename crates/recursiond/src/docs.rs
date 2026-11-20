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
    json!([
        {
            "name": "doc_write",
            "description": "Save an HTML document to this chat's session and show it to the human as a card that opens in its own panel. \
Use it for anything long or visual: reports, big tables, comparisons, diagrams (inline SVG, or Mermaid from a CDN), dashboards, \
interactive explainers. Write one complete, self-contained HTML page; scripts and CDN libraries are allowed. \
Pass `id` to replace an existing document (same card, updated content) instead of creating a new one. \
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
                let written = self.state.lock().unwrap().write_doc(&session, id, title, alias.to_string(), now_ms());
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
