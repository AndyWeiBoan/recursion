//! Session documents and the built-in MCP server that lets agents write them.
//!
//! Each agent process gets its own MCP URL (`/mcp/<key>`, see `Hub::mcp_url`), so a tool call
//! knows which chat and which agent it came from without the agent having to say. The server
//! speaks MCP's streamable-HTTP transport in its simplest form: one JSON-RPC POST, one JSON reply.

use crate::i18n::tr;
use crate::model::{Doc, DocKind};
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

/// A diagram's JSON, next to the SVG drawn from it.
fn spec_file(session: &str, doc: &str) -> PathBuf {
    dir(session).join(format!("{doc}.json"))
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

/// A session's docs join another session's (a closed session reopened into one on its folder).
pub fn move_session(from: &str, to: &str) {
    let Ok(files) = std::fs::read_dir(dir(from)) else { return };
    let _ = std::fs::create_dir_all(dir(to));
    for f in files.flatten() {
        let _ = std::fs::rename(f.path(), dir(to).join(f.file_name()));
    }
    let _ = std::fs::remove_dir(dir(from));
}

fn tools() -> Value {
    let mut all = doc_tools();
    all.as_array_mut().unwrap().extend(crate::browse::tools());
    all.as_array_mut().unwrap().extend(crate::agents::poll_tools().as_array().unwrap().clone());
    all.as_array_mut().unwrap().extend(crate::agents::todo_tools().as_array().unwrap().clone());
    all
}

fn doc_tools() -> Value {
    json!([
        {
            "name": "doc_write",
            "description": "Save an HTML document to this chat's session and show it to the human as a card that opens in its own panel. \
Use it for anything long or visual: reports, big tables, comparisons, diagrams (inline SVG, or Mermaid from a CDN), dashboards, \
interactive explainers. Write one complete, self-contained HTML page; scripts and CDN libraries are allowed. \
Not for handing over a file (a CSV, a spreadsheet, a PDF): save the file in the project and call `file_share` instead. \
Pass `id` to replace an existing document (same card, updated content) instead of creating a new one. \
The page can talk back: `recursion.reply(data)` sends `data` (string or JSON) to you as the human's next message, \
so send only what the human chose or entered (e.g. {\"database\": \"SQLite\"}), not your own text; you answer after it arrives. \
`recursion.sendPrompt(text)` posts `text` to the whole chat as the human. Call them from a button or form submit, \
e.g. to collect choices or answers from a form you designed. \
Keep your chat reply short and point to the document. \
For architecture, workflow or sequence diagrams use diagram_write instead.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short title shown on the card and panel" },
                    "html": { "type": "string", "description": "A complete HTML document" },
                    "file": { "type": "string", "description": "Path of an HTML file to show instead of `html` (absolute, or relative to the chat's folder)" },
                    "id": { "type": "string", "description": "Existing document id to replace; omit to create a new one" }
                },
                "required": ["title"]
            }
        },
        {
            "name": "diagram_write",
            "description": crate::diagram::GUIDE,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short title shown on the card and panel" },
                    "diagram": { "type": "object", "description": "The diagram as JSON: `type` plus its boxes and lines (see the description)" },
                    "id": { "type": "string", "description": "Existing diagram id to replace; omit to create a new one" }
                },
                "required": ["title", "diagram"]
            }
        },
        {
            "name": "doc_read",
            "description": "Read a document in this session (yours or another agent's): the HTML of a page, or the JSON of a diagram.",
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
        },
        {
            "name": "file_share",
            "description": "Give the human a file of the project (a CSV, a spreadsheet, a PDF, a picture, any file you made or found): \
it goes into the chat's materials as a card, and opens in the app's own viewer (a CSV as a table they can sort, with its raw text a click away). \
Save the file in the project first, then share its path. Use this rather than writing a document that only shows or wraps a file's contents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file's path, relative to the project folder (or absolute inside it)" },
                    "title": { "type": "string", "description": "What to call it on the card; the file's name if left out" }
                },
                "required": ["path"]
            }
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
        if name.starts_with("todo_") {
            return match self.hub.todo_tool(panel, member, name, args) {
                Ok(t) => text(t, false),
                Err(e) => text(e, true),
            };
        }
        if name.starts_with("poll_") {
            return match self.hub.poll_tool(panel, member, name, args) {
                Ok(t) => text(t, false),
                Err(e) => text(e, true),
            };
        }
        if name == "file_share" {
            let path = args["path"].as_str().unwrap_or("").trim().to_string();
            return match self.usable_file(panel, &path) {
                Some(full) if full.is_file() => {
                    let title = args["title"].as_str().filter(|t| !t.trim().is_empty()).map(str::to_string).unwrap_or_else(|| full.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path.clone()));
                    self.hub.record_artifact(panel, crate::workspace::ArtifactKind::File, &path, &title, member);
                    text(format!("Shared `{path}` in the chat's materials as \"{title}\"."), false)
                }
                Some(_) => text(format!("`{path}` is not a file."), true),
                None => text(format!("`{path}` is not a file of this project (or one the human gave this chat)."), true),
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
                text(
                    docs.iter()
                        .map(|d| format!("{} — {}{} (by @{}, rev {})", d.id, d.title, if d.kind == DocKind::Diagram { " [diagram]" } else { "" }, d.author, d.rev))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    false,
                )
            }
            "doc_read" => {
                let id = args["id"].as_str().unwrap_or("");
                // A diagram is read as the JSON it was drawn from: that's what an agent edits.
                if docs.iter().any(|d| d.id == id && d.kind == DocKind::Diagram) {
                    return match std::fs::read_to_string(spec_file(&session, id)) {
                        Ok(spec) => text(format!("Diagram `{id}` (edit this JSON and pass it to diagram_write with id \"{id}\"):\n{spec}"), false),
                        Err(_) => text(format!("Diagram `{id}` has no saved JSON."), true),
                    };
                }
                match docs.iter().any(|d| d.id == id).then(|| read(&session, id)).flatten() {
                    Some(html) => text(html, false),
                    None => text(format!("No document with id `{id}`. Use doc_list to see what exists."), true),
                }
            }
            "doc_write" => {
                let title = args["title"].as_str().unwrap_or("").trim().to_string();
                // A file the agent made (archify and the like), or the HTML itself.
                let from_file = match args["file"].as_str().map(str::trim).filter(|f| !f.is_empty()) {
                    Some(f) => {
                        let cwd = self.state.lock().unwrap().panel_cwd(panel).unwrap_or_default();
                        let path = std::path::Path::new(&cwd).join(f);
                        if !matches!(path.extension().and_then(|e| e.to_str()), Some("html" | "htm")) {
                            return text("`file` must be an .html file.", true);
                        }
                        match std::fs::read_to_string(&path) {
                            Ok(h) => Some(h),
                            Err(e) => return text(format!("Could not read {}: {e}", path.display()), true),
                        }
                    }
                    None => None,
                };
                let html = from_file.as_deref().unwrap_or_else(|| args["html"].as_str().unwrap_or(""));
                if title.is_empty() || html.trim().is_empty() {
                    return text("`title` and either `html` or `file` are required.", true);
                }
                // A document named like a data file is a file being handed over: that is file_share's.
                let ext = title.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
                if matches!(ext.as_str(), "csv" | "tsv" | "xlsx" | "xls" | "pdf" | "json" | "txt" | "parquet") {
                    return text(format!("Not saved: \"{title}\" is a file, not a document. Save the file in the project and call `file_share` with its path; the human opens it in the app's own viewer (a CSV as a sortable table)."), true);
                }
                if html.len() > MAX_HTML {
                    return text(format!("Document too large ({} bytes, max {MAX_HTML}).", html.len()), true);
                }
                let id = args["id"].as_str().filter(|s| !s.is_empty());
                match self.save_doc(&session, &docs, panel, member, alias, id, DocKind::Html, title, html, None) {
                    Ok((doc, created)) => text(
                        format!(
                            "{} document `{}` (rev {}). The human sees it as a card in the chat and it is open in a panel next to it. \
To revise it, call doc_write again with id \"{}\".",
                            if created { "Created" } else { "Updated" },
                            doc.id,
                            doc.rev,
                            doc.id
                        ),
                        false,
                    ),
                    Err(e) => text(e, true),
                }
            }
            "diagram_write" => {
                let title = args["title"].as_str().unwrap_or("").trim().to_string();
                if title.is_empty() {
                    return text("`title` is required.", true);
                }
                // Some clients send the object as a JSON string.
                let spec = match &args["diagram"] {
                    Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
                    v => v.clone(),
                };
                let drawn = match crate::diagram::render(&spec) {
                    Ok(d) => d,
                    Err(problems) => return text(format!("Not drawn; fix these and call again:\n- {}", problems.join("\n- ")), true),
                };
                let id = args["id"].as_str().filter(|s| !s.is_empty());
                let pretty = serde_json::to_string_pretty(&spec).unwrap_or_default();
                match self.save_doc(&session, &docs, panel, member, alias, id, DocKind::Diagram, title, &drawn.svg, Some(&pretty)) {
                    Ok((doc, created)) => {
                        let mut t = format!(
                            "{} diagram `{}` (rev {}); it is open in a panel next to the chat. To change it, call diagram_write again with id \"{}\".",
                            if created { "Created" } else { "Updated" },
                            doc.id,
                            doc.rev,
                            doc.id
                        );
                        if !drawn.warnings.is_empty() {
                            t += &format!("\nIt was drawn, but has layout problems worth fixing:\n- {}", drawn.warnings.join("\n- "));
                        }
                        text(t, false)
                    }
                    Err(e) => text(e, true),
                }
            }
            _ => text(format!("Unknown tool {name}"), true),
        }
    }

    /// Saves a new or revised document, opens it next to the chat and posts its card.
    #[allow(clippy::too_many_arguments)]
    fn save_doc(&self, session: &str, docs: &[Doc], panel: &str, member: &str, alias: &str, id: Option<&str>, kind: DocKind, title: String, content: &str, spec: Option<&str>) -> Result<(Doc, bool), String> {
        if let Some(id) = id {
            if !docs.iter().any(|d| d.id == id) {
                return Err(format!("No document with id `{id}`. Omit `id` to create a new one."));
            }
        }
        let (doc, created) = self.state.lock().unwrap().write_doc(session, id, kind, title, alias.to_string(), panel, member, now_ms()).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir(session)).map_err(|e| format!("Could not save: {e}"))?;
        std::fs::write(file(session, &doc.id), content).map_err(|e| format!("Could not save: {e}"))?;
        match spec {
            Some(s) => std::fs::write(spec_file(session, &doc.id), s).map_err(|e| format!("Could not save: {e}"))?,
            None => {
                let _ = std::fs::remove_file(spec_file(session, &doc.id));
            }
        }
        if created {
            let _ = self.state.lock().unwrap().open_doc(panel, &doc.id, false);
        }
        self.mark_changed();
        self.hub.post_doc(panel, member, &doc, !created);
        let art = if kind == DocKind::Diagram { crate::workspace::ArtifactKind::Diagram } else { crate::workspace::ArtifactKind::Doc };
        self.hub.record_artifact(panel, art, &doc.id, &doc.title, member);
        Ok((doc, created))
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
            let s = st.sessions.iter().find(|s| s.id == session).ok_or_else(|| tr!("err.noSession"))?;
            (s.docs.iter().find(|d| d.id == doc).cloned().ok_or_else(|| tr!("err.noDoc"))?, s.cwd.clone())
        };
        // A page script could loop; one message per doc every couple of seconds is plenty for clicks.
        {
            let mut last = self.doc_last.lock().unwrap();
            let now = std::time::Instant::now();
            if last.get(doc).is_some_and(|t| now.duration_since(*t) < PAGE_MESSAGE_GAP) {
                return Err(tr!("doc.tooFast", title = d.title));
            }
            last.insert(doc.to_string(), now);
        }
        let data = data.trim();
        if data.is_empty() {
            return Ok(());
        }
        if data.chars().count() > MAX_PAGE_MESSAGE {
            return Err(tr!("doc.tooLong", title = d.title, max = MAX_PAGE_MESSAGE));
        }
        let chat_alive = self.state.lock().unwrap().panels.get(&d.chat).is_some_and(|p| p.kind == crate::model::PanelKind::Workspace);
        if !chat_alive {
            return Err(tr!("doc.chatGone", title = d.title));
        }
        let body = if data.contains('\n') || data.starts_with('{') || data.starts_with('[') { format!("```\n{data}\n```") } else { data.to_string() };
        let text = match action {
            "reply" => {
                // To whoever wrote it, under their current name; everyone if they left.
                let to = self.hub.alias_of(&d.chat, &d.member).map(|a| format!("@{a} ")).unwrap_or_default();
                format!("{to}{}\n{body}", tr!("doc.fromDoc", title = d.title))
            }
            "prompt" => format!("{}\n{body}", tr!("doc.fromDoc", title = d.title)),
            other => return Err(format!("Unknown document action: {other}")),
        };
        let via = crate::workspace::DocVia { doc: d.id.clone(), title: d.title.clone(), data: data.to_string() };
        self.hub.send(&d.chat, text, cwd, Some(via), None, vec![], vec![], vec![]).await
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
