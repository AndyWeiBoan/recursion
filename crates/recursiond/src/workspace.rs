//! Group-chat workspaces: members, transcript, persistence.

use crate::i18n::tr;
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
    /// Summarising its own conversation to free context (compact).
    Compacting,
}

/// How much an agent may do without asking. Same three levels for both CLIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermMode {
    /// Asks before editing files and before running commands.
    Ask,
    /// Edits files on its own, asks before running commands.
    #[default]
    Edits,
    /// Never asks; no sandbox.
    Auto,
}

impl PermMode {
    pub fn label(self) -> String {
        match self {
            PermMode::Ask => tr!("perm.ask"),
            PermMode::Edits => tr!("perm.edits"),
            PermMode::Auto => tr!("perm.auto"),
        }
    }
    /// Claude's `--permission-mode`.
    pub fn claude(self) -> &'static str {
        match self {
            PermMode::Ask => "default",
            PermMode::Edits => "acceptEdits",
            PermMode::Auto => "bypassPermissions",
        }
    }
    /// Codex's (approvalPolicy, sandbox).
    pub fn codex(self) -> (&'static str, &'static str) {
        match self {
            PermMode::Ask => ("untrusted", "workspace-write"),
            PermMode::Edits => ("on-request", "workspace-write"),
            PermMode::Auto => ("never", "danger-full-access"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub kind: AgentKind,
    pub alias: String,
    #[serde(default)]
    pub model: Option<String>,
    /// Account id it runs under (see accounts.rs); None = the default account of its kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
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
    #[serde(default)]
    pub mode: PermMode,
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
    /// The agent's account needs logging in again; the UI offers a button for it.
    Relogin { account: String, name: String, text: String, #[serde(default)] resolved: bool },
    /// Loop guard tripped; the UI offers "continue".
    Guard { text: String, #[serde(default)] resolved: bool },
    /// The human started a poll; the card shows its live state from `Workspace.polls`.
    Poll { poll: String },
    /// A poll ended: the point where agents are told the result.
    PollClosed { poll: String },
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
    /// A thread's conclusion brought back to the main chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_thread: Option<ThreadRef>,
    /// The message the human was answering (their "回覆").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyRef>,
    /// Project files the human attached (paths relative to the session's folder).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// What the whole turn cost; set on the turn's first message when the turn ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
    #[serde(flatten)]
    pub body: Body,
}

/// What a reply points at, copied when sent so it still reads right if the original changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplyRef {
    pub msg: String,
    /// "you" or a member id.
    pub from: String,
    /// The start of its text.
    pub text: String,
}

pub const REPLY_EXCERPT: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThreadRef {
    pub thread: String,
    pub num: u64,
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
    /// Side discussions about one message, each with a forked copy of its author's conversation.
    #[serde(default)]
    pub threads: Vec<Thread>,
    #[serde(default)]
    pub next_thread: u64,
    /// Things the agents made for this chat (documents, pages they opened), listed beside threads.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub polls: Vec<Poll>,
    /// Files outside the project the human picked from their computer: picking one lets the
    /// chat's agents read it (and the viewer show it). Absolute, canonical paths.
    #[serde(default)]
    pub grants: Vec<String>,
    #[serde(default)]
    pub next_poll: u64,
    #[serde(default)]
    pub settings: ChatSettings,
}

/// What the human set for one chat.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChatSettings {
    /// Shown instead of 群聊 in the tree and the chat's header.
    #[serde(default)]
    pub name: String,
    /// Standing instructions for every agent in this chat.
    #[serde(default)]
    pub rules: String,
    /// Stop taking messages once the agents' known spend reaches this (USD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PollStatus {
    #[default]
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    /// "you" or a member id.
    pub voter: String,
    pub option: usize,
    #[serde(default)]
    pub reason: String,
    pub ts: u64,
}

/// A question with fixed options. Who voted for what stays hidden from the agents until it closes,
/// so the later ones can't just follow the earlier ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Poll {
    pub id: String,
    /// Shown as #n, per chat.
    pub num: u64,
    pub question: String,
    /// Background for the question (optional).
    #[serde(default)]
    pub description: String,
    pub options: Vec<String>,
    /// Members asked to vote (everyone in the chat when it started).
    pub voters: Vec<String>,
    pub votes: Vec<Vote>,
    pub status: PollStatus,
    pub created: u64,
    #[serde(default)]
    pub closed: Option<u64>,
    /// The chat message that shows it.
    #[serde(default)]
    pub msg: String,
    /// Taken off the 產出 list by the human (the chat still shows it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
}

pub const POLL_OPTIONS_MAX: usize = 8;

/// A, B, C… for option `i`.
pub fn option_letter(i: usize) -> char {
    (b'A' + i as u8) as char
}

impl Poll {
    /// "B", "2" or the option's own text (any case) → its index.
    pub fn parse_option(&self, s: &str) -> Option<usize> {
        let s = s.trim().trim_end_matches([')', '.', '）']).trim();
        let n = self.options.len();
        if s.chars().count() == 1 {
            let c = s.chars().next()?.to_ascii_uppercase();
            if c.is_ascii_uppercase() && ((c as u8 - b'A') as usize) < n {
                return Some((c as u8 - b'A') as usize);
            }
        }
        if let Ok(k) = s.parse::<usize>() {
            return (1..=n).contains(&k).then(|| k - 1);
        }
        self.options.iter().position(|o| o.trim().eq_ignore_ascii_case(s))
    }

    /// Records (or changes) a vote while the poll is open.
    pub fn vote(&mut self, voter: &str, option: usize, reason: &str, ts: u64) -> Result<(), String> {
        if self.status != PollStatus::Open {
            return Err(format!("Poll #{} is already closed.", self.num));
        }
        if option >= self.options.len() {
            return Err(format!("Poll #{} has no option {}.", self.num, option + 1));
        }
        self.votes.retain(|v| v.voter != voter);
        self.votes.push(Vote { voter: voter.to_string(), option, reason: reason.trim().to_string(), ts });
        Ok(())
    }

    /// Every agent asked who is still in the chat has voted.
    pub fn agents_done(&self, members: &[String]) -> bool {
        self.voters.iter().filter(|v| members.contains(v)).all(|v| self.votes.iter().any(|x| &x.voter == v))
    }

    pub fn close(&mut self, ts: u64) -> bool {
        if self.status == PollStatus::Closed {
            return false;
        }
        self.status = PollStatus::Closed;
        self.closed = Some(ts);
        true
    }

    /// The question and options as agents read them.
    pub fn ballot(&self) -> String {
        let opts: Vec<String> = self.options.iter().enumerate().map(|(i, o)| format!("{}) {o}", option_letter(i))).collect();
        let about = if self.description.trim().is_empty() { String::new() } else { format!(" ({})", self.description.trim().replace('\n', " ")) };
        format!("poll #{} (id `{}`): \"{}\"{about} — options: {}", self.num, self.id, self.question, opts.join("; "))
    }

    /// The result with who voted what and why; `name` turns a voter into "@alias" / "the human".
    pub fn result(&self, name: &dyn Fn(&str) -> String) -> String {
        let mut out = format!("Poll #{} \"{}\" — {} vote(s):", self.num, self.question, self.votes.len());
        for (i, o) in self.options.iter().enumerate() {
            let who: Vec<String> = self.votes.iter().filter(|v| v.option == i).map(|v| name(&v.voter)).collect();
            out.push_str(&format!("\n  {}) {o}: {}{}", option_letter(i), who.len(), if who.is_empty() { String::new() } else { format!(" ({})", who.join(", ")) }));
        }
        let reasons: Vec<String> = self.votes.iter().filter(|v| !v.reason.is_empty()).map(|v| format!("\n  {} chose {}: {}", name(&v.voter), option_letter(v.option), v.reason)).collect();
        if !reasons.is_empty() {
            out.push_str("\nReasons:");
            out.push_str(&reasons.concat());
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// A document; `target` is its doc id.
    Doc,
    /// A web page the agent opened in its browser; `target` is the url.
    Page,
    /// A diagram (a document drawn from JSON); `target` is its doc id.
    Diagram,
}

/// One entry in a chat's 產出 list. Same kind and target = the same artifact, updated in place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub target: String,
    pub title: String,
    /// Member id of the agent that made it.
    pub by: String,
    pub created: u64,
    pub updated: u64,
    /// Taken off the 產出 list by the human (the chat still shows it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
}

impl Workspace {
    /// Adds an artifact, or refreshes the one with the same kind and target. Returns it.
    pub fn record_artifact(&mut self, kind: ArtifactKind, target: &str, title: &str, by: &str, at: u64) -> Artifact {
        if let Some(a) = self.artifacts.iter_mut().find(|a| a.kind == kind && a.target == target) {
            if !title.is_empty() {
                a.title = title.to_string();
            }
            a.by = by.to_string();
            a.updated = at;
            // Made again: it's news again.
            a.removed = false;
            return a.clone();
        }
        let a = Artifact {
            id: crate::model::new_id(),
            kind,
            target: target.to_string(),
            title: title.to_string(),
            by: by.to_string(),
            created: at,
            updated: at,
            removed: false,
        };
        self.artifacts.push(a.clone());
        a
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    /// The agent is answering.
    Replying,
    /// The agent answered; the human's turn.
    #[default]
    Waiting,
    /// Its conclusion was posted to the main chat.
    Brought,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    /// Short number shown as #n, per chat.
    pub num: u64,
    /// The main-chat message it is about, and the part of it that was quoted.
    pub msg: String,
    pub quote: String,
    /// About the whole message rather than a selected part (`quote` is then the message itself).
    #[serde(default)]
    pub whole: bool,
    /// Member who answers (the author of `msg`).
    pub agent: String,
    /// The CLI conversation of the fork (Claude session id / Codex thread id), once it exists.
    #[serde(default)]
    pub fork: Option<String>,
    pub messages: Vec<ThreadMsg>,
    #[serde(default)]
    pub status: ThreadStatus,
    pub updated: u64,
    /// Taken off the 產出 list by the human (the chat still shows it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMsg {
    /// "you", a member id, or "system" for errors.
    pub from: String,
    pub text: String,
    pub ts: u64,
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
                // A thread reply in flight died with the daemon.
                w.threads.iter_mut().filter(|t| t.status == ThreadStatus::Replying).for_each(|t| t.status = ThreadStatus::Waiting);
                // Threads saved before `whole` existed: a quote equal to its message meant the whole message.
                for t in w.threads.iter_mut().filter(|t| !t.whole) {
                    let text = w.messages.iter().find(|m| m.id == t.msg).and_then(|m| if let Body::Text { text } = &m.body { Some(text) } else { None });
                    t.whole = text.is_some_and(|x| x.chars().take(1200).collect::<String>() == t.quote);
                }
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
        Some("doc_write") => tr!("tool.docWrite"),
        Some("diagram_write") => format!("📐 {}", tr!("tool.diagramWrite")),
        Some("doc_read") => tr!("tool.docRead"),
        Some("doc_list") => tr!("tool.docList"),
        Some("browse_open") => format!("🌐 {}", tr!("tool.browseOpen")),
        Some("browse_read") => format!("🌐 {}", tr!("tool.browseRead")),
        Some("browse_click") => format!("🌐 {}", tr!("tool.browseClick")),
        Some("browse_fill") => format!("🌐 {}", tr!("tool.browseFill")),
        Some("browse_screenshot") => format!("🌐 {}", tr!("tool.browseScreenshot")),
        Some("poll_vote") => format!("📊 {}", tr!("tool.pollVote")),
        Some("poll_list") => format!("📊 {}", tr!("tool.pollList")),
        Some("poll_read") => format!("📊 {}", tr!("tool.pollRead")),
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
        "TodoWrite" => tr!("tool.todo"),
        _ if recursion_tool(name) == Some("diagram_write") => format!("「{}」{}", s("title"), if s("id").is_empty() { String::new() } else { tr!("tool.updatedSuffix") }),
        _ if recursion_tool(name) == Some("doc_write") => format!("「{}」{}", s("title"), if s("id").is_empty() { String::new() } else { tr!("tool.updatedSuffix") }),
        _ if recursion_tool(name) == Some("doc_read") => s("id"),
        _ if recursion_tool(name) == Some("doc_list") => String::new(),
        _ if recursion_tool(name) == Some("browse_open") => s("url"),
        _ if recursion_tool(name) == Some("browse_click") => format!("#{}", input.get("ref").map(|v| v.to_string()).unwrap_or_default()),
        _ if recursion_tool(name) == Some("browse_fill") => {
            format!("#{} ← {}", input.get("ref").map(|v| v.to_string()).unwrap_or_default(), s("value").chars().take(60).collect::<String>())
        }
        _ if recursion_tool(name).is_some_and(|t| t.starts_with("browse_")) => String::new(),
        // What it voted shows on the poll card when the poll closes, not in the tool line before.
        _ if recursion_tool(name).is_some_and(|t| t.starts_with("poll_")) => input.get("poll").and_then(|v| v.as_str()).map(|p| if p.starts_with('#') { p.to_string() } else { String::new() }).unwrap_or_default(),
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
        _ if recursion_tool(name) == Some("poll_vote") => String::new(),
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
        let m = Message { id: "1".into(), ts: 1, from: "you".into(), seq: 1, to: vec![], streaming: false, turn: false, via: None, from_thread: None, reply_to: None, files: vec![], usage: None, body: Body::Text { text: "hi".into() } };
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
        Member { id: id.into(), kind: AgentKind::Claude, alias: alias.into(), model: None, account: None, cli_session_id: None, cost_usd: 0.0, status: MemberStatus::Idle, usage: Default::default(), context_used: 0, context_window: 0, seen: 0, mode: PermMode::default() }
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

    #[test]
    fn same_target_is_the_same_artifact() {
        let mut ws = Workspace::default();
        let a = ws.record_artifact(ArtifactKind::Page, "https://a.test/", "A", "m1", 1);
        let b = ws.record_artifact(ArtifactKind::Page, "https://a.test/", "", "m2", 2);
        assert_eq!(a.id, b.id);
        assert_eq!((b.title.as_str(), b.by.as_str(), b.created, b.updated), ("A", "m2", 1, 2), "keeps the title when none is given");
        ws.record_artifact(ArtifactKind::Doc, "https://a.test/", "doc", "m1", 3);
        assert_eq!(ws.artifacts.len(), 2, "a different kind is a different artifact");
    }
    #[test]
    fn polls_parse_options_and_hide_nothing_after_closing() {
        let mut p = Poll {
            id: "p".into(),
            num: 1,
            question: "Q".into(),
            description: String::new(),
            options: vec!["Ship it".into(), "Wait".into()],
            voters: vec!["a".into(), "b".into()],
            votes: vec![],
            status: PollStatus::Open,
            created: 0,
            closed: None,
            msg: String::new(),
            removed: false,
        };
        assert_eq!(p.parse_option("b"), Some(1));
        assert_eq!(p.parse_option("A)"), Some(0));
        assert_eq!(p.parse_option("2"), Some(1));
        assert_eq!(p.parse_option("ship it"), Some(0));
        assert_eq!(p.parse_option("C"), None);
        assert_eq!(p.parse_option("3"), None);
        p.vote("a", 0, "fast", 1).unwrap();
        p.vote("a", 1, "changed my mind", 2).unwrap();
        assert_eq!(p.votes.len(), 1, "a second vote replaces the first");
        assert!(!p.agents_done(&["a".into(), "b".into()]));
        assert!(p.agents_done(&["a".into()]), "agents who left don't hold it open");
        assert!(p.close(3) && !p.close(4));
        assert!(p.vote("b", 0, "", 5).is_err());
        let r = p.result(&|v| format!("@{v}"));
        assert!(r.contains("B) Wait: 1 (@a)") && r.contains("@a chose B: changed my mind"), "{r}");
    }
}
