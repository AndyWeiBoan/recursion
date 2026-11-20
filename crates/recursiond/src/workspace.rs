//! Group-chat workspaces: members, transcript, persistence.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub ts: u64,
    /// "you" for the human, otherwise a member id.
    pub from: String,
    #[serde(default)]
    pub streaming: bool,
    #[serde(flatten)]
    pub body: Body,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Workspace {
    pub members: Vec<Member>,
    pub messages: Vec<Message>,
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

    pub fn member_mut(&mut self, id: &str) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.id == id)
    }

    pub fn message_mut(&mut self, id: &str) -> Option<&mut Message> {
        self.messages.iter_mut().rev().find(|m| m.id == id)
    }
}

/// One-line summary of a tool call for the transcript, plus a fuller detail string.
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
        let m = Message { id: "1".into(), ts: 1, from: "you".into(), streaming: false, body: Body::Text { text: "hi".into() } };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["kind"], "text");
        assert_eq!(v["text"], "hi");
        let back: Message = serde_json::from_value(v).unwrap();
        assert!(matches!(back.body, Body::Text { .. }));
    }
}
