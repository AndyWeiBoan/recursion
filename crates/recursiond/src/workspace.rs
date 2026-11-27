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
    /// The model the CLI says it is running: the one in `model`, or, with none picked, the CLI's
    /// own default (which only the CLI knows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_model: Option<String>,
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
    /// Things from the chat's 產出 the human pointed at (dragged into the box).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<OutputRef>,
    /// What the whole turn cost; set on the turn's first message when the turn ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
    #[serde(flatten)]
    pub body: Body,
}

/// One of the chat's outputs a human message points at.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputRef {
    pub kind: RefKind,
    /// The artifact's, poll's, thread's or todo list's id.
    pub id: String,
    /// A todo item's id, when it is one item of the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Its title when sent, so the message still reads right once it is gone.
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Doc,
    Diagram,
    Page,
    Poll,
    Todo,
    Thread,
}

impl RefKind {
    fn word(self) -> &'static str {
        match self {
            RefKind::Doc => "document",
            RefKind::Diagram => "diagram",
            RefKind::Page => "web page",
            RefKind::Poll => "poll",
            RefKind::Todo => "todo list",
            RefKind::Thread => "side thread",
        }
    }
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
    /// The chat's todo lists, kept by the human and the agents alike.
    #[serde(default)]
    pub todo_lists: Vec<TodoList>,
    #[serde(default)]
    pub next_list: u64,
    /// Before named lists there was one list: read only, to move it into `todo_lists`.
    #[serde(default, skip_serializing)]
    todos: Vec<Todo>,
    #[serde(default)]
    pub settings: ChatSettings,
    /// Agents that left, by member id: their messages, votes and threads keep their names.
    #[serde(default)]
    pub former: std::collections::BTreeMap<String, FormerMember>,
    /// Each agent's turns, oldest first: what lets a rewind take its conversation back.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub turns: std::collections::BTreeMap<String, Vec<TurnMark>>,
    /// Rewinds an agent's conversation still has to take, done when it next starts: the point
    /// to go back to (Claude: the message to resume at; Codex: the first turn to drop).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub rewinds: std::collections::BTreeMap<String, String>,
}

/// One turn of an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TurnMark {
    /// The last message seq it was shown for this turn.
    pub upto: u64,
    /// The last seq in the chat when the turn ended; None while it runs (or if it never ended).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    /// Claude: its last message in the turn (to resume at). Codex: the turn's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<String>,
}

/// Where a rewind takes an agent's conversation.
#[derive(Debug, PartialEq)]
pub enum Back {
    /// It has nothing from the target on: leave it.
    Keep,
    /// Go back to `point` (see `Workspace::rewinds`) and keep the first `turns` marks;
    /// it has then been shown messages up to `seen`.
    To { point: String, turns: usize, seen: u64 },
    /// Nothing to go back to: a new conversation, fed the chat from the start.
    Fresh,
}

impl Back {
    /// Taking the chat back to before message `seq`: keep the turns that ended before it was sent.
    pub fn plan(kind: AgentKind, marks: &[TurnMark], seq: u64) -> Back {
        // A turn that never recorded its end (the daemon stopped during it) was over by the time
        // the next one was handed what it was shown: without that, one such turn would make
        // everything after it look newer than the target.
        let ended_before = |i: usize| marks[i].end.or_else(|| marks.get(i + 1).map(|n| n.upto)).is_some_and(|e| e < seq);
        let Some(cut) = (0..marks.len()).position(|i| !ended_before(i)) else { return Back::Keep };
        match kind {
            // Claude resumes at the last message of the last turn kept.
            AgentKind::Claude => match marks[..cut].iter().rposition(|t| t.point.is_some()) {
                Some(i) => Back::To { point: marks[i].point.clone().unwrap(), turns: i + 1, seen: marks[i].upto },
                None => Back::Fresh,
            },
            // Codex drops the first turn not kept and everything after it.
            AgentKind::Codex => match (&marks[cut].point, cut) {
                (Some(turn), 1..) => Back::To { point: turn.clone(), turns: cut, seen: marks[cut - 1].upto },
                _ => Back::Fresh,
            },
        }
    }
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
pub enum TodoState {
    #[default]
    Todo,
    Doing,
    Done,
}

/// One item of a todo list, in the order the human arranged them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    /// Shown as #n, per list.
    pub num: u64,
    pub text: String,
    /// The item's card: details in Markdown (steps, links), written by the human or an agent.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub state: TodoState,
    /// Who is on it while it's being done ("you" or a member id). An agent holds it until it
    /// finishes, lets go, or disconnects, so two agents never take the same item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// "you" or a member id.
    pub by: String,
    /// Who ticked it ("you" or a member id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_by: Option<String>,
    /// Put away in the list's 完成區, ticked or not (not ticked: it won't be done).
    #[serde(default)]
    pub archived: bool,
    pub created: u64,
}

/// A named todo list of a chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoList {
    pub id: String,
    /// Shown as #n, per chat.
    pub num: u64,
    pub name: String,
    #[serde(default)]
    pub items: Vec<Todo>,
    #[serde(default)]
    pub next_item: u64,
    pub created: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormerMember {
    pub alias: String,
    pub kind: AgentKind,
}

impl Workspace {
    /// A reference as sent by the UI, checked against this chat and titled from it; None if
    /// the chat has no such thing.
    pub fn resolve_ref(&self, r: &OutputRef) -> Option<OutputRef> {
        let title = match r.kind {
            RefKind::Doc | RefKind::Diagram | RefKind::Page => self.artifacts.iter().find(|a| a.id == r.id && !a.removed).map(|a| a.title.clone())?,
            RefKind::Poll => self.polls.iter().find(|p| p.id == r.id && !p.removed).map(|p| p.question.clone())?,
            RefKind::Thread => self.threads.iter().find(|t| t.id == r.id && !t.removed).map(|t| t.quote.clone())?,
            RefKind::Todo => {
                let list = self.todo_lists.iter().find(|l| l.id == r.id)?;
                match &r.item {
                    Some(i) => list.items.iter().find(|t| &t.id == i).map(|t| t.text.clone())?,
                    None => list.name.clone(),
                }
            }
        };
        Some(OutputRef { title, ..r.clone() })
    }

    /// How a reference reads for the agents: what it is and how to look at it.
    pub fn ref_line(&self, r: &OutputRef) -> String {
        const EXCERPT: usize = 600;
        let gone = || format!("(reference: {} \"{}\" — the human has since removed it; it is no longer available)", r.kind.word(), r.title);
        match r.kind {
            RefKind::Doc | RefKind::Diagram | RefKind::Page => match self.artifacts.iter().find(|a| a.id == r.id && !a.removed) {
                Some(a) if r.kind == RefKind::Page => format!("(reference: web page \"{}\" {})", a.title, a.target),
                Some(a) => format!("(reference: {} `{}` \"{}\" — read it with doc_read, id \"{}\")", r.kind.word(), a.target, a.title, a.target),
                None => gone(),
            },
            RefKind::Poll => match self.polls.iter().find(|p| p.id == r.id && !p.removed) {
                Some(p) => format!("(reference: poll #{} \"{}\" — read it with poll_read, poll \"#{}\")", p.num, p.question, p.num),
                None => gone(),
            },
            RefKind::Todo => match self.todo_lists.iter().find(|l| l.id == r.id) {
                Some(l) => match r.item.as_ref().map(|i| l.items.iter().find(|t| &t.id == i)) {
                    None => format!("(reference: todo list #{} \"{}\" — read it with todo_read, list \"#{}\")", l.num, l.name, l.num),
                    Some(Some(t)) => format!("(reference: item #{} \"{}\" of todo list #{} \"{}\" — todo_read, list \"#{}\", shows the list)", t.num, t.text, l.num, l.name, l.num),
                    Some(None) => gone(),
                },
                None => gone(),
            },
            // Threads have no tool: what it reached comes along.
            RefKind::Thread => match self.threads.iter().find(|t| t.id == r.id && !t.removed) {
                Some(t) => {
                    let last = t.messages.iter().rev().find(|m| m.from != "you" && m.from != "system").map(|m| m.text.chars().take(EXCERPT).collect::<String>());
                    let about = if t.whole { "a whole message".to_string() } else { format!("\"{}\"", t.quote.replace('\n', " ")) };
                    match last {
                        Some(l) => format!("(reference: side thread #{} about {about}; where it ended up: {})", t.num, l.replace('\n', " ")),
                        None => format!("(reference: side thread #{} about {about}; nothing said in it yet)", t.num),
                    }
                }
                None => gone(),
            },
        }
    }

    /// Member id → alias, for everyone who has been in the chat (current names win).
    pub fn names(&self) -> std::collections::HashMap<String, String> {
        let mut n: std::collections::HashMap<String, String> = self.former.iter().map(|(id, f)| (id.clone(), f.alias.clone())).collect();
        n.extend(self.members.iter().map(|m| (m.id.clone(), m.alias.clone())));
        n
    }

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
                // The single list from before named lists.
                if !w.todos.is_empty() && w.todo_lists.is_empty() {
                    let items = std::mem::take(&mut w.todos);
                    w.next_list = 1;
                    w.todo_lists.push(TodoList { id: crate::model::new_id(), num: 1, name: tr!("todo.defaultList"), next_item: items.iter().map(|t| t.num).max().unwrap_or(0), items, created: now_ms() });
                }
                // No agent is connected yet: what they were on is free again.
                for t in w.todo_lists.iter_mut().flat_map(|l| l.items.iter_mut()) {
                    if t.state == TodoState::Doing && t.worker.as_deref() != Some("you") {
                        t.state = TodoState::Todo;
                        t.worker = None;
                    }
                }
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
        Some("todo_lists") | Some("todo_read") => format!("☑ {}", tr!("tool.todoList")),
        Some("todo_add") => format!("☑ {}", tr!("tool.todoAdd")),
        Some("todo_claim") => format!("☑ {}", tr!("tool.todoClaim")),
        Some("todo_note") => format!("☑ {}", tr!("tool.todoNote")),
        Some("todo_done") => format!("☑ {}", tr!("tool.todoDone")),
        Some("todo_release") => format!("☑ {}", tr!("tool.todoRelease")),
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
        _ if recursion_tool(name) == Some("todo_add") => format!("{} · {}", s("list"), s("text")),
        _ if recursion_tool(name).is_some_and(|t| t.starts_with("todo_")) => [s("list"), s("todo")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" "),
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
        let m = Message { id: "1".into(), ts: 1, from: "you".into(), seq: 1, to: vec![], streaming: false, turn: false, via: None, from_thread: None, reply_to: None, files: vec![], refs: vec![], usage: None, body: Body::Text { text: "hi".into() } };
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
        Member { id: id.into(), kind: AgentKind::Claude, alias: alias.into(), model: None, running_model: None, account: None, cli_session_id: None, cost_usd: 0.0, status: MemberStatus::Idle, usage: Default::default(), context_used: 0, context_window: 0, seen: 0, mode: PermMode::default() }
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

#[cfg(test)]
mod former_tests {
    use super::*;

    #[test]
    fn departed_members_keep_their_names() {
        let mut ws = Workspace::default();
        ws.members.push(Member { id: "a".into(), kind: AgentKind::Codex, alias: "codex".into(), model: None, running_model: None, account: None, cli_session_id: None, cost_usd: 0.0, status: MemberStatus::Idle, usage: Default::default(), context_used: 0, context_window: 0, seen: 0, mode: PermMode::default() });
        ws.former.insert("b".into(), FormerMember { alias: "claude".into(), kind: AgentKind::Claude });
        let n = ws.names();
        assert_eq!(n.get("a").map(String::as_str), Some("codex"));
        assert_eq!(n.get("b").map(String::as_str), Some("claude"));
    }
}

#[cfg(test)]
mod rewind_tests {
    use super::*;

    fn mark(upto: u64, end: Option<u64>, point: &str) -> TurnMark {
        TurnMark { upto, end, point: (!point.is_empty()).then(|| point.to_string()) }
    }

    #[test]
    fn claude_goes_back_to_the_last_turn_that_ended_before_the_target() {
        let marks = [mark(4, Some(6), "u1"), mark(8, Some(11), "u2"), mark(12, Some(14), "u3")];
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 9), Back::To { point: "u1".into(), turns: 1, seen: 4 });
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 12), Back::To { point: "u2".into(), turns: 2, seen: 8 });
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 15), Back::Keep, "it has nothing from the target on");
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 3), Back::Fresh);
    }

    #[test]
    fn a_turn_still_answering_when_the_target_came_is_dropped() {
        // Shown up to 8, but its answer ran past the target (10).
        let marks = [mark(4, Some(6), "u1"), mark(8, Some(12), "u2")];
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 10), Back::To { point: "u1".into(), turns: 1, seen: 4 });
        let unfinished = [mark(4, Some(6), "u1"), mark(8, None, "")];
        assert_eq!(Back::plan(AgentKind::Claude, &unfinished, 20), Back::To { point: "u1".into(), turns: 1, seen: 4 });
    }

    #[test]
    fn a_turn_cut_short_by_a_restart_does_not_hide_the_turns_after_it() {
        // The daemon stopped during the second turn: no end. The turns after it still count.
        let marks = [mark(4, Some(6), "u1"), mark(8, None, "u2"), mark(12, Some(14), "u3"), mark(20, Some(22), "u4")];
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 18), Back::To { point: "u3".into(), turns: 3, seen: 12 });
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 10), Back::To { point: "u1".into(), turns: 1, seen: 4 });
    }

    #[test]
    fn claude_skips_turns_with_nothing_to_resume_at() {
        let marks = [mark(4, Some(6), "u1"), mark(7, Some(8), ""), mark(12, Some(14), "u3")];
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 10), Back::To { point: "u1".into(), turns: 1, seen: 4 });
    }

    #[test]
    fn codex_drops_from_the_first_turn_not_kept() {
        let marks = [mark(4, Some(6), "t1"), mark(8, Some(11), "t2"), mark(12, Some(14), "t3")];
        assert_eq!(Back::plan(AgentKind::Codex, &marks, 9), Back::To { point: "t2".into(), turns: 1, seen: 4 });
        assert_eq!(Back::plan(AgentKind::Codex, &marks, 5), Back::Fresh, "no turn kept");
        assert_eq!(Back::plan(AgentKind::Codex, &marks, 15), Back::Keep);
    }

    #[test]
    fn an_agent_that_joined_after_the_target_starts_fresh() {
        let marks = [mark(12, Some(14), "u3")];
        assert_eq!(Back::plan(AgentKind::Claude, &marks, 10), Back::Fresh);
        assert_eq!(Back::plan(AgentKind::Claude, &[], 10), Back::Keep);
    }
}

#[cfg(test)]
mod ref_tests {
    use super::*;
    use serde_json::json;

    fn ws() -> Workspace {
        serde_json::from_value(json!({
            "members": [], "messages": [],
            "artifacts": [
                { "id": "a1", "kind": "doc", "target": "d9", "title": "BBU-4398 合併審查", "by": "m", "created": 1, "updated": 1 },
                { "id": "a2", "kind": "doc", "target": "d8", "title": "舊的", "by": "m", "created": 1, "updated": 1, "removed": true }
            ],
            "todo_lists": [{ "id": "l1", "num": 2, "name": "十題二分搜尋", "items": [{ "id": "i1", "num": 3, "text": "寫測試", "state": "todo", "by": "you", "created": 1 }], "next_item": 4, "created": 1 }]
        }))
        .unwrap()
    }

    fn r(kind: RefKind, id: &str, item: Option<&str>) -> OutputRef {
        OutputRef { kind, id: id.into(), item: item.map(Into::into), title: String::new() }
    }

    #[test]
    fn only_this_chats_outputs_are_taken_and_titled_from_it() {
        let w = ws();
        assert_eq!(w.resolve_ref(&r(RefKind::Doc, "a1", None)).unwrap().title, "BBU-4398 合併審查");
        assert_eq!(w.resolve_ref(&r(RefKind::Todo, "l1", Some("i1"))).unwrap().title, "寫測試");
        assert!(w.resolve_ref(&r(RefKind::Doc, "elsewhere", None)).is_none());
        assert!(w.resolve_ref(&r(RefKind::Doc, "a2", None)).is_none(), "taken off the list");
    }

    #[test]
    fn agents_are_told_what_it_is_and_how_to_read_it() {
        let w = ws();
        let line = w.ref_line(&w.resolve_ref(&r(RefKind::Doc, "a1", None)).unwrap());
        assert!(line.contains("`d9`") && line.contains("doc_read"), "{line}");
        let line = w.ref_line(&w.resolve_ref(&r(RefKind::Todo, "l1", Some("i1"))).unwrap());
        assert!(line.contains("item #3") && line.contains("todo list #2"), "{line}");
        let gone = OutputRef { title: "舊的".into(), ..r(RefKind::Doc, "a2", None) };
        assert!(w.ref_line(&gone).contains("no longer available"));
    }
}
