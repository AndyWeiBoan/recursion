//! Group-chat workspaces: members, transcript, persistence.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    #[default]
    Idle,
    Starting,
    Thinking,
    WaitingPermission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub kind: AgentKind,
    pub alias: String,
    #[serde(default)]
    pub model: Option<String>,
    /// CLI conversation id, used to resume after restarts.
    #[serde(default)]
    pub cli_session_id: Option<String>,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub status: MemberStatus,
    /// Latest rate-limit utilisation reported by the CLI (0–1), e.g. {"five_hour": 0.1}.
    #[serde(default)]
    pub usage: std::collections::BTreeMap<String, f64>,
    /// Tokens in the agent's context as of its latest model call, and the model's limit (0 = unknown).
    #[serde(default)]
    pub context_used: u64,
    #[serde(default)]
    pub context_window: u64,
    /// Highest message `seq` this agent has been shown.
    #[serde(default)]
    pub seen: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermState {
    Pending,
    Allowed,
    AlwaysAllowed,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Text { text: String },
    Tool { tool_use_id: String, name: String, summary: String, #[serde(default)] detail: String, #[serde(default)] result: Option<String>, #[serde(default)] is_error: bool },
    Permission { request_id: String, tool: String, summary: String, detail: String, state: PermState },
    System { text: String },
    Error { text: String },
    /// An agent created (`updated: false`) or replaced a session document.
    Doc { doc: String, title: String, #[serde(default)] updated: bool },
    /// Loop guard tripped; the UI offers "continue".
    Guard { text: String, #[serde(default)] resolved: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub ts: u64,
    /// "you" for the human, otherwise a member id.
    pub from: String,
    /// Monotonic per workspace; agents track what they've seen by seq.
    #[serde(default)]
    pub seq: u64,
    /// Addressed members (empty = everyone). Only set on human messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    #[serde(default)]
    pub streaming: bool,
    /// First message of an agent's turn: starts a new header row in the UI.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub turn: bool,
    /// Sent from a doc page (`recursion.reply` / `sendPrompt`) rather than typed: shown as a card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<DocVia>,
    /// What the whole turn cost; set on the turn's first message when the turn ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
    #[serde(flatten)]
    pub body: Body,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocVia {
    pub doc: String,
    pub title: String,
    /// What the page sent, as it sent it (the message text adds the label and @mention for agents).
    pub data: String,
}

/// Tokens used by one agent turn. `input` includes cached tokens; `cached` is the cache-read part.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TurnUsage {
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    /// Claude reports a price; Codex (subscription) does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Workspace {
    pub members: Vec<Member>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub next_seq: u64,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn dir() -> PathBuf {
    crate::home().join("workspaces")
}

impl Workspace {
    pub fn load(panel: &str) -> Self {
        std::fs::read(dir().join(format!("{panel}.json")))
            .ok()
            .and_then(|b| serde_json::from_slice::<Workspace>(&b).ok())
            .map(|mut w| {
                // Older files had no seq: number messages in order.
                if w.messages.iter().any(|m| m.seq == 0) {
                    for (i, m) in w.messages.iter_mut().enumerate() {
                        m.seq = i as u64 + 1;
                    }
                    w.next_seq = w.messages.len() as u64;
                }
                // Nothing is running right after a load.
                w.members.iter_mut().for_each(|m| m.status = MemberStatus::Idle);
                w.messages.iter_mut().for_each(|m| {
                    m.streaming = false;
                    if let Body::Permission { state, .. } = &mut m.body {
                        if *state == PermState::Pending {
                            *state = PermState::Denied;
                        }
                    }
                });
                w
            })
            .unwrap_or_default()
    }

    pub fn save(&self, panel: &str) {
        let _ = std::fs::create_dir_all(dir());
        let path = dir().join(format!("{panel}.json"));
        let tmp = path.with_extension("json.tmp");
        if let Ok(b) = serde_json::to_vec(self) {
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    pub fn delete(panel: &str) {
        let _ = std::fs::remove_file(dir().join(format!("{panel}.json")));
    }

    pub fn push(&mut self, mut m: Message) -> Message {
        self.next_seq += 1;
        m.seq = self.next_seq;
        self.messages.push(m.clone());
        m
    }

    pub fn member_mut(&mut self, id: &str) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.id == id)
    }

    pub fn message_mut(&mut self, id: &str) -> Option<&mut Message> {
        self.messages.iter_mut().rev().find(|m| m.id == id)
    }
}

/// One-line summary of a tool call for the transcript, plus a fuller detail string.
/// Our own MCP tools, however the CLI spells them (`mcp__recursion__doc_write`, `recursion.doc_write`).
fn recursion_tool(name: &str) -> Option<&str> {
    name.strip_prefix("mcp__recursion__").or_else(|| name.strip_prefix("recursion."))
}

/// The tool name as shown in the chat.
pub fn display_tool_name(name: &str) -> String {
    match recursion_tool(name) {
        Some("doc_write") => "寫文件".into(),
        Some("doc_read") => "讀文件".into(),
        Some("doc_list") => "列出文件".into(),
        _ => name.to_string(),
    }
}

pub fn summarize_tool(name: &str, input: &serde_json::Value, cwd: &str) -> (String, String) {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let rel = |p: String| {
        let prefix = format!("{}/", cwd.trim_end_matches('/'));
        p.strip_prefix(&prefix).map(|r| r.to_string()).unwrap_or(p)
    };
    let summary = match name {
        "Bash" => s("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => rel(s("file_path")),
        "Grep" => format!("{}{}", s("pattern"), if s("path").is_empty() { String::new() } else { format!(" in {}", rel(s("path"))) }),
        "Glob" => s("pattern"),
        "WebFetch" => s("url"),
        "WebSearch" => s("query"),
        "Task" | "Agent" => s("description"),
        "TodoWrite" => "更新待辦清單".into(),
        _ if recursion_tool(name) == Some("doc_write") => format!("「{}」{}", s("title"), if s("id").is_empty() { "" } else { "（更新）" }),
        _ if recursion_tool(name) == Some("doc_read") => s("id"),
        _ if recursion_tool(name) == Some("doc_list") => String::new(),
        _ => {
            let j = input.to_string();
            j.chars().take(120).collect()
        }
    };
    let detail = match name {
        "Bash" => {
            let d = s("description");
            if d.is_empty() { s("command") } else { format!("{}\n\n{}", d, s("command")) }
        }
        "Write" => s("content"),
        "Edit" => format!("- {}\n+ {}", s("old_string"), s("new_string")),
        _ if recursion_tool(name) == Some("doc_write") => s("html"),
        _ => serde_json::to_string_pretty(input).unwrap_or_default(),
    };
    let one_line: String = summary.lines().next().unwrap_or("").chars().take(200).collect();
    (one_line, detail.chars().take(8000).collect())
}

/// Text of a tool_result content (string or list of blocks).
pub fn tool_result_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarizes_common_tools() {
        let (s, _) = summarize_tool("Bash", &json!({"command": "ls -la\necho hi"}), "/p");
        assert_eq!(s, "ls -la");
        let (s, _) = summarize_tool("Edit", &json!({"file_path": "/p/src/a.rs", "old_string": "a", "new_string": "b"}), "/p");
        assert_eq!(s, "src/a.rs");
        let (s, _) = summarize_tool("Grep", &json!({"pattern": "foo", "path": "/p/src"}), "/p");
        assert_eq!(s, "foo in src");
    }

    #[test]
    fn tool_result_blocks() {
        assert_eq!(tool_result_text(&json!("x")), "x");
        assert_eq!(tool_result_text(&json!([{"type":"text","text":"a"},{"type":"text","text":"b"}])), "a\nb");
    }

    #[test]
    fn message_roundtrip_flattens_body() {
        let m = Message { id: "1".into(), ts: 1, from: "you".into(), seq: 1, to: vec![], streaming: false, turn: false, via: None, usage: None, body: Body::Text { text: "hi".into() } };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["kind"], "text");
        assert_eq!(v["text"], "hi");
        let back: Message = serde_json::from_value(v).unwrap();
        assert!(matches!(back.body, Body::Text { .. }));
    }
}

/// Text with fenced and inline code removed, so `@names` inside code don't count as mentions.
fn strip_code(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut in_tick = false;
        for ch in line.chars() {
            if ch == '`' {
                in_tick = !in_tick;
            } else if !in_tick {
                out.push(ch);
            }
        }
        out.push('\n');
    }
    out
}

/// Member ids mentioned with @alias (case-insensitive). `@all` / `@全部` mean everyone.
pub fn mentions(text: &str, members: &[Member], exclude: Option<&str>) -> Vec<String> {
    let clean = strip_code(text);
    let mut found: Vec<String> = Vec::new();
    let chars: Vec<char> = clean.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-' || chars[j] == '.') {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect::<String>().trim_end_matches('.').to_string();
            if name.eq_ignore_ascii_case("all") || name == "全部" || name == "所有人" {
                for m in members {
                    if Some(m.id.as_str()) != exclude && !found.contains(&m.id) {
                        found.push(m.id.clone());
                    }
                }
            } else if let Some(m) = members.iter().find(|m| m.alias.eq_ignore_ascii_case(&name)) {
                if Some(m.id.as_str()) != exclude && !found.contains(&m.id) {
                    found.push(m.id.clone());
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

#[cfg(test)]
mod mention_tests {
    use super::*;

    fn m(id: &str, alias: &str) -> Member {
        Member { id: id.into(), kind: AgentKind::Claude, alias: alias.into(), model: None, cli_session_id: None, cost_usd: 0.0, status: MemberStatus::Idle, usage: Default::default(), context_used: 0, context_window: 0, seen: 0 }
    }

    #[test]
    fn finds_mentions_case_insensitive_and_skips_code() {
        let ms = vec![m("a", "claude"), m("b", "Opus")];
        assert_eq!(mentions("hey @Claude and @opus.", &ms, None), vec!["a", "b"]);
        assert_eq!(mentions("`@claude` and\n```\n@opus\n```\nno", &ms, None), Vec::<String>::new());
        assert_eq!(mentions("mail me at x@claude.com", &ms, None), Vec::<String>::new());
        assert_eq!(mentions("@all go", &ms, Some("a")), vec!["b"]);
        assert_eq!(mentions("@全部 開始", &ms, None), vec!["a", "b"]);
    }

}
