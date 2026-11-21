//! Agents driving browser panels (the browse_* MCP tools).
//!
//! The pages live in the desktop app's OS webviews, not here, so every call travels
//! daemon → app (a client that announced itself with `browser_host`) → back as `browse_result`.
//! Each agent gets its own browser panel next to its chat, so the human sees what it is doing.

use crate::App;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

/// Longer than the app's own 60 s limit, so its error message wins.
const TIMEOUT: Duration = Duration::from_secs(75);
/// Largest screenshot passed on to an agent (base64 PNG).
const MAX_SHOT: usize = 6 * 1024 * 1024;

#[derive(Default)]
pub struct Bridge {
    hosts: Mutex<Vec<(u64, UnboundedSender<String>)>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next: AtomicU64,
}

impl Bridge {
    pub fn add_host(&self, client: u64, tx: UnboundedSender<String>) {
        let mut hosts = self.hosts.lock().unwrap();
        hosts.retain(|(c, _)| *c != client);
        hosts.push((client, tx));
    }

    pub fn remove_host(&self, client: u64) {
        self.hosts.lock().unwrap().retain(|(c, _)| *c != client);
    }

    pub fn resolve(&self, id: u64, result: Result<Value, String>) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(result);
        }
    }

    async fn request(&self, panel: &str, op: &str, args: Value) -> Result<Value, String> {
        // The most recently connected app window does the work.
        let host = self.hosts.lock().unwrap().last().map(|(_, tx)| tx.clone());
        let host = host.ok_or("The Recursion desktop app is not open, and the browser lives in it. Ask the human to open the app.")?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let req = json!({ "t": "browse_req", "id": id, "panel": panel, "op": op, "args": args });
        if host.send(req.to_string()).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err("The Recursion app just disconnected; try again.".into());
        }
        match tokio::time::timeout(TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("The browser went away before answering.".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("The browser did not answer in time.".into())
            }
        }
    }
}

pub fn tools() -> Vec<Value> {
    let rf = json!({ "type": "integer", "description": "Element number from the last browse_read / browse_open, e.g. 12 for [#12 …]" });
    vec![
        json!({
            "name": "browse_open",
            "description": "Open a URL in your own browser panel next to the chat (the human sees it) and return the page as text. \
Links, buttons and form fields appear as numbered items like [#3 link: Docs → https://…] or [#7 input text: Search]; \
use those numbers with browse_click and browse_fill. Works for any site and for the project's dev server (e.g. http://localhost:5173). \
The browser shares the human's logins, so never purchase, post, send, delete or change settings without the human's explicit go-ahead.",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string", "description": "Full URL; a bare host like example.com gets https://" } },
                "required": ["url"]
            }
        }),
        json!({
            "name": "browse_read",
            "description": "Read the page currently open in your browser panel again (after clicks, or when it changed). \
Numbers are refreshed on every read. Long pages come in parts: pass `from` to continue where the last part ended.",
            "inputSchema": { "type": "object", "properties": { "from": { "type": "integer", "description": "Character offset to continue from" } } }
        }),
        json!({
            "name": "browse_click",
            "description": "Click a numbered link or button on the page in your browser panel. Tells you where the page ended up; call browse_read to see it.",
            "inputSchema": { "type": "object", "properties": { "ref": rf }, "required": ["ref"] }
        }),
        json!({
            "name": "browse_fill",
            "description": "Type into a numbered field (input, textarea, editable area), pick an option of a select (by its text or value), \
or set a checkbox/radio (\"true\"/\"false\"). `submit: true` then presses Enter / submits its form.",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": rf, "value": { "type": "string" }, "submit": { "type": "boolean" } },
                "required": ["ref", "value"]
            }
        }),
        json!({
            "name": "browse_screenshot",
            "description": "Take a screenshot of what is visible in your browser panel, to check layout or visuals.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

fn text(t: impl Into<String>, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": is_error })
}

/// What an agent typed → a URL the webview accepts.
fn normalize(url: &str) -> Result<String, String> {
    let u = url.trim();
    if u.is_empty() {
        return Err("`url` is required.".into());
    }
    // "name:" that is not "host:port" means the URL already has a scheme.
    let has_scheme = u.split_once(':').is_some_and(|(s, rest)| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
            && !rest.starts_with(|c: char| c.is_ascii_digit())
    });
    let full = if has_scheme {
        u.to_string()
    } else if u.starts_with("localhost") || u.starts_with("127.0.0.1") || u.starts_with("[::1]") || u.starts_with("0.0.0.0") {
        format!("http://{u}")
    } else if u.starts_with('/') {
        format!("file://{u}")
    } else {
        format!("https://{u}")
    };
    let scheme = full.split(':').next().unwrap_or("").to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" | "file" | "about" => Ok(full),
        s => Err(format!("Only http, https and file URLs can be opened, not {s}:")),
    }
}

fn page_text(r: &Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
    let mut out = format!("# {}\n{}\n\n{}", s("title"), s("url"), s("text"));
    if let Some(more) = r["more"].as_u64() {
        let total = r["total"].as_u64().unwrap_or(0);
        out.push_str(&format!("\n\n(… page continues: {} of {total} characters shown. Call browse_read with from={more} for the next part.)", more));
    }
    out
}

impl App {
    pub async fn browse(&self, chat: &str, member: &str, name: &str, args: &Value) -> Value {
        let r = self.browse_inner(chat, member, name, args).await;
        r.unwrap_or_else(|e| text(e, true))
    }

    async fn browse_inner(&self, chat: &str, member: &str, name: &str, args: &Value) -> Result<Value, String> {
        let (panel, created) = self.state.lock().unwrap().agent_browser(chat, member).map_err(|e| e.to_string())?;
        if created {
            self.mark_changed();
        }
        let (op, req) = match name {
            "browse_open" => {
                let url = normalize(args["url"].as_str().unwrap_or(""))?;
                self.state.lock().unwrap().set_panel_url(&panel, url.clone()).map_err(|e| e.to_string())?;
                self.mark_changed();
                ("open", json!({ "url": url }))
            }
            _ => {
                let url = self.state.lock().unwrap().panels.get(&panel).and_then(|p| p.url.clone());
                let Some(url) = url else {
                    return Err("Your browser has no page open yet. Call browse_open first.".into());
                };
                match name {
                    "browse_read" => ("read", json!({ "from": args["from"], "url": url })),
                    "browse_click" => ("click", json!({ "ref": args["ref"], "url": url })),
                    "browse_fill" => ("fill", json!({ "ref": args["ref"], "value": args["value"], "submit": args["submit"], "url": url })),
                    "browse_screenshot" => ("screenshot", json!({ "url": url })),
                    _ => return Err(format!("Unknown tool {name}")),
                }
            }
        };
        let r = self.browser.request(&panel, op, req).await?;
        // Remember where the page ended up, so the panel reopens there.
        if let Some(url) = r["url"].as_str().filter(|u| !u.is_empty()) {
            let differs = self.state.lock().unwrap().panels.get(&panel).is_some_and(|p| p.url.as_deref() != Some(url));
            if differs {
                let _ = self.state.lock().unwrap().set_panel_url(&panel, url.to_string());
                self.mark_changed();
            }
        }
        // Each page it lands on goes in the chat's 產出 list.
        if let Some(url) = r["url"].as_str().filter(|u| u.starts_with("http")) {
            self.hub.record_artifact(chat, crate::workspace::ArtifactKind::Page, url, r["title"].as_str().unwrap_or(""), member);
        }
        Ok(match op {
            "open" | "read" => text(page_text(&r), false),
            "screenshot" => {
                let png = r["png"].as_str().unwrap_or("");
                if png.is_empty() || png.len() > MAX_SHOT {
                    return Err("The screenshot came back empty or too large.".into());
                }
                json!({ "content": [
                    { "type": "image", "data": png, "mimeType": "image/png" },
                    { "type": "text", "text": format!("{} — {}", r["title"].as_str().unwrap_or(""), r["url"].as_str().unwrap_or("")) }
                ], "isError": false })
            }
            _ => {
                let did = r["did"].as_str().unwrap_or("done");
                let moved = if r["navigated"].as_bool().unwrap_or(false) { "The page navigated. " } else { "" };
                text(
                    format!(
                        "{did}. {moved}Now at: {} — {}. Call browse_read to see the page.",
                        r["title"].as_str().unwrap_or(""),
                        r["url"].as_str().unwrap_or("")
                    ),
                    false,
                )
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn urls_get_a_scheme() {
        assert_eq!(normalize("example.com/a").unwrap(), "https://example.com/a");
        assert_eq!(normalize("localhost:5173").unwrap(), "http://localhost:5173");
        assert_eq!(normalize("http://x.test").unwrap(), "http://x.test");
        assert_eq!(normalize("/tmp/a.html").unwrap(), "file:///tmp/a.html");
        assert!(normalize("javascript:alert(1)").is_err());
        assert!(normalize("").is_err());
    }
}
