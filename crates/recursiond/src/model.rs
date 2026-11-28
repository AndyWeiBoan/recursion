//! Persistent workspace model: Session → Window → Panel, with a split-tree layout per window.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelKind {
    Workspace,
    Terminal,
    Browser,
}

/// `Row` lays children out side by side (left → right); `Column` stacks them (top → bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Row,
    Column,
}

/// Where a dragged panel goes, relative to the one it is dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    Split { id: String, dir: Dir, children: Vec<Node>, sizes: Vec<f32> },
    Leaf { panel: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Panel {
    pub id: String,
    pub kind: PanelKind,
    #[serde(default)]
    pub url: Option<String>,
    /// The agent (member id) whose browser this is; it opens pages here with the browse_* tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// A chat's own name (its settings), shown instead of the kind.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    pub id: String,
    pub name: String,
    pub root: Node,
    pub focus: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub group: String,
    pub windows: Vec<Window>,
    pub active_window: Option<String>,
    /// Documents agents wrote for this session (content lives in `docs/<session>/<id>.html`).
    #[serde(default)]
    pub docs: Vec<Doc>,
    /// What the add-agent form starts from in this session.
    #[serde(default)]
    pub defaults: AgentDefaults,
    /// The human's MCP servers (by name, from `mcp.json`) this session's agents get.
    #[serde(default)]
    pub mcp: Vec<String>,
    /// What every agent of the project is told (see guides.rs); a chat's rules go above it.
    #[serde(default)]
    pub prompt: String,
    /// The guidance files in the folder ticked for the agents; None = not chosen yet, the folder's
    /// own AGENTS.md and CLAUDE.md (and the human's own files), as the CLIs would read them.
    #[serde(default)]
    pub guides: Option<Vec<String>>,
    /// Skills and rules of Recursion's library (library.rs) this project's agents get.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
}

/// Starting values for agents added in a session; empty fields keep the form's own defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::workspace::AgentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<crate::workspace::PermMode>,
}

/// An HTML document attached to a session. Rewriting it replaces it (same id, higher rev).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Doc {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub kind: DocKind,
    /// Alias of the agent that last wrote it.
    pub author: String,
    pub rev: u64,
    pub updated: u64,
    /// Chat panel and member id of the last writer: where the page's `recursion.reply` goes.
    #[serde(default)]
    pub chat: String,
    #[serde(default)]
    pub member: String,
}

/// A page of HTML, or a diagram we drew from the agent's JSON (its file holds the SVG).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    #[default]
    Html,
    Diagram,
}

/// Browser panels show a doc when their url is `doc:<id>`.
pub fn doc_url(id: &str) -> String {
    format!("doc:{id}")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub sessions: Vec<Session>,
    pub panels: HashMap<String, Panel>,
    pub active_session: Option<String>,
    /// The human's language (a `locales/` code); `None` follows the system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Sessions and chats the human closed, newest last: they can be opened again until deleted.
    #[serde(default)]
    pub closed: Vec<Closed>,
}

/// Something closed, kept (with its chats' files and its docs) until it is deleted from the list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Closed {
    pub id: String,
    /// When it was closed (ms since epoch).
    pub at: u64,
    #[serde(flatten)]
    pub what: ClosedWhat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClosedWhat {
    /// A whole session, with the panels of its windows.
    Session { session: Session, panels: Vec<Panel> },
    /// A chat closed on its own, and the session it was in (which may be gone by now).
    Chat { panel: Panel, session: String, session_name: String, cwd: String },
    /// Agents removed from a chat (one, or all at once), whole: they go back into it, their
    /// conversations resumed. `chat_name`: what the chat was called, for when it is closed.
    Agents { panel: String, chat_name: String, members: Vec<crate::workspace::Member> },
}

/// Where opening a closed session put its docs, when they have to move: they live in a folder
/// named after the session, and a session already open on that folder takes them in.
pub struct Reopened {
    pub docs_from: Option<(String, String)>,
}

/// Two paths name the same folder (symlinks and a trailing `/` aside).
pub fn same_dir(a: &str, b: &str) -> bool {
    let norm = |p: &str| std::fs::canonicalize(p).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| p.trim_end_matches('/').to_string());
    norm(a) == norm(b)
}

#[derive(Debug)]
pub enum ModelError {
    NotFound(&'static str),
}

pub type Res<T> = Result<T, ModelError>;

impl std::error::Error for ModelError {}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::NotFound(what) => write!(f, "{what} not found"),
        }
    }
}

fn group_from_cwd(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "others".into())
}

impl Node {
    fn leaves(&self, out: &mut Vec<String>) {
        match self {
            Node::Leaf { panel } => out.push(panel.clone()),
            Node::Split { children, .. } => children.iter().for_each(|c| c.leaves(out)),
        }
    }

    /// The same splits with the panels put in its leaves in `order` (the same panels, in order).
    fn refill(&mut self, order: &mut std::slice::Iter<'_, String>) {
        match self {
            Node::Leaf { panel } => {
                if let Some(p) = order.next() {
                    *panel = p.clone();
                }
            }
            Node::Split { children, .. } => children.iter_mut().for_each(|c| c.refill(order)),
        }
    }

    /// Split the leaf holding `target`, putting `new_panel` after it in direction `dir`.
    /// Returns true if the target was found.
    fn split(&mut self, target: &str, dir: Dir, new_panel: &str) -> bool {
        self.insert(target, dir, new_panel, false)
    }

    /// Like `split`, with `new_panel` before the target (left of it, or above) when `before`.
    fn insert(&mut self, target: &str, dir: Dir, new_panel: &str, before: bool) -> bool {
        match self {
            Node::Leaf { panel } if panel == target => {
                let old = Node::Leaf { panel: panel.clone() };
                let new = Node::Leaf { panel: new_panel.to_string() };
                *self = Node::Split {
                    id: new_id(),
                    dir,
                    children: if before { vec![new, old] } else { vec![old, new] },
                    sizes: vec![0.5, 0.5],
                };
                true
            }
            Node::Leaf { .. } => false,
            Node::Split { dir: sdir, children, sizes, .. } => {
                // Same-direction split of a direct leaf child: insert as a sibling (tmux-like flattening).
                if *sdir == dir {
                    if let Some(i) = children
                        .iter()
                        .position(|c| matches!(c, Node::Leaf { panel } if panel == target))
                    {
                        let half = sizes[i] / 2.0;
                        let at = if before { i } else { i + 1 };
                        sizes[i] = half;
                        sizes.insert(at, half);
                        children.insert(at, Node::Leaf { panel: new_panel.to_string() });
                        return true;
                    }
                }
                children.iter_mut().any(|c| c.insert(target, dir, new_panel, before))
            }
        }
    }

    /// Add `panel` at the end (right, or bottom) of this whole tree.
    fn append(&mut self, dir: Dir, panel: &str) {
        let leaf = Node::Leaf { panel: panel.to_string() };
        match self {
            Node::Split { dir: d, children, sizes, .. } if *d == dir => {
                let n = children.len() as f32;
                sizes.iter_mut().for_each(|s| *s *= n / (n + 1.0));
                sizes.push(1.0 / (n + 1.0));
                children.push(leaf);
            }
            _ => {
                let old = std::mem::replace(self, Node::Leaf { panel: String::new() });
                *self = Node::Split { id: new_id(), dir, children: vec![old, leaf], sizes: vec![0.5, 0.5] };
            }
        }
    }

    /// Remove the leaf holding `target`. Returns Some(true) if this node itself became empty.
    fn remove(&mut self, target: &str) -> Option<bool> {
        match self {
            Node::Leaf { panel } => (panel == target).then_some(true),
            Node::Split { children, sizes, .. } => {
                for i in 0..children.len() {
                    if let Some(empty) = children[i].remove(target) {
                        if empty {
                            let freed = sizes.remove(i);
                            children.remove(i);
                            let total: f32 = sizes.iter().sum::<f32>() + freed;
                            if !sizes.is_empty() {
                                let scale = total / (total - freed).max(f32::EPSILON);
                                sizes.iter_mut().for_each(|s| *s *= scale);
                            }
                        }
                        if children.is_empty() {
                            return Some(true);
                        }
                        if children.len() == 1 {
                            let only = children.remove(0);
                            *self = only;
                        }
                        return Some(false);
                    }
                }
                None
            }
        }
    }

    /// Exchange the places of two leaves.
    fn swap(&mut self, a: &str, b: &str) {
        match self {
            Node::Leaf { panel } if panel == a => *panel = b.to_string(),
            Node::Leaf { panel } if panel == b => *panel = a.to_string(),
            Node::Leaf { .. } => {}
            Node::Split { children, .. } => children.iter_mut().for_each(|c| c.swap(a, b)),
        }
    }

    fn resize(&mut self, split_id: &str, new_sizes: &[f32]) -> bool {
        match self {
            Node::Leaf { .. } => false,
            Node::Split { id, children, sizes, .. } => {
                if id == split_id {
                    if new_sizes.len() == sizes.len() && new_sizes.iter().all(|s| s.is_finite() && *s > 0.0) {
                        let total: f32 = new_sizes.iter().sum();
                        *sizes = new_sizes.iter().map(|s| s / total).collect();
                        return true;
                    }
                    return false;
                }
                children.iter_mut().any(|c| c.resize(split_id, new_sizes))
            }
        }
    }
}

impl Window {
    pub fn panel_ids(&self) -> Vec<String> {
        let mut v = vec![];
        self.root.leaves(&mut v);
        v
    }
}

impl State {
    fn session_mut(&mut self, id: &str) -> Res<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id).ok_or(ModelError::NotFound("session"))
    }

    fn new_panel(&mut self, kind: PanelKind) -> String {
        let id = new_id();
        self.panels.insert(id.clone(), Panel { id: id.clone(), kind, url: None, agent: None, title: String::new() });
        id
    }

    fn new_window(&mut self, kind: PanelKind, name: String) -> Window {
        let panel = self.new_panel(kind);
        Window { id: new_id(), name, root: Node::Leaf { panel: panel.clone() }, focus: panel }
    }

    /// The open session on this folder, if there is one.
    pub fn session_at(&self, cwd: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| same_dir(&s.cwd, cwd))
    }

    /// One session per folder: opening a folder that already has one goes to it.
    pub fn create_session(&mut self, name: String, cwd: String, kind: PanelKind) -> String {
        if let Some(id) = self.session_at(&cwd).map(|s| s.id.clone()) {
            self.active_session = Some(id.clone());
            return id;
        }
        let window = self.new_window(kind, String::new());
        let id = new_id();
        let session = Session {
            id: id.clone(),
            group: group_from_cwd(&cwd),
            name,
            cwd,
            active_window: Some(window.id.clone()),
            windows: vec![window],
            docs: vec![],
            defaults: AgentDefaults::default(),
            mcp: vec![],
            prompt: String::new(),
            guides: None,
            skills: vec![],
            rules: vec![],
        };
        self.sessions.push(session);
        self.active_session = Some(id.clone());
        id
    }

    /// Close a session into the closed list. Returns its panels' ids (so the caller can stop
    /// their processes).
    pub fn close_session(&mut self, id: &str, now: u64) -> Res<Vec<String>> {
        let idx = self.sessions.iter().position(|s| s.id == id).ok_or(ModelError::NotFound("session"))?;
        let s = self.sessions.remove(idx);
        let removed: Vec<String> = s.windows.iter().flat_map(|w| w.panel_ids()).collect();
        let panels = removed.iter().filter_map(|p| self.panels.remove(p)).collect();
        if self.active_session.as_deref() == Some(id) {
            self.active_session = self.sessions.first().map(|s| s.id.clone());
        }
        self.closed.push(Closed { id: new_id(), at: now, what: ClosedWhat::Session { session: s, panels } });
        Ok(removed)
    }

    /// The chats among `panels`, as closed-list entries (taken before they are closed).
    pub fn closing_chats(&self, panels: &[String], now: u64) -> Vec<Closed> {
        panels
            .iter()
            .filter_map(|p| {
                let panel = self.panels.get(p).filter(|x| x.kind == PanelKind::Workspace)?.clone();
                let s = self.session_of_panel(p)?;
                Some(Closed { id: new_id(), at: now, what: ClosedWhat::Chat { panel, session: s.id.clone(), session_name: s.name.clone(), cwd: s.cwd.clone() } })
            })
            .collect()
    }

    /// Open a closed session or chat again and make it the active one.
    /// Removed agents go back by the chat (see `Hub::restore_agents`), not here.
    pub fn reopen(&mut self, id: &str) -> Res<Reopened> {
        let idx = self.closed.iter().position(|c| c.id == id && !matches!(c.what, ClosedWhat::Agents { .. })).ok_or(ModelError::NotFound("closed"))?;
        let c = self.closed.remove(idx);
        let mut docs_from = None;
        match c.what {
            ClosedWhat::Session { mut session, panels } => {
                panels.into_iter().for_each(|p| {
                    self.panels.insert(p.id.clone(), p);
                });
                // Its folder was opened again meanwhile: bring the windows and docs over to that session.
                if let Some(i) = self.sessions.iter().position(|s| same_dir(&s.cwd, &session.cwd)) {
                    let into = &mut self.sessions[i];
                    into.active_window = session.windows.first().map(|w| w.id.clone()).or(into.active_window.take());
                    into.windows.append(&mut session.windows);
                    if !session.docs.is_empty() {
                        into.docs.append(&mut session.docs);
                        docs_from = Some((session.id.clone(), into.id.clone()));
                    }
                    self.active_session = Some(into.id.clone());
                } else {
                    self.active_session = Some(session.id.clone());
                    self.sessions.push(session);
                }
            }
            ClosedWhat::Chat { panel, session, session_name, cwd } => {
                let sid = match self.sessions.iter().find(|s| s.id == session).or_else(|| self.session_at(&cwd)) {
                    Some(s) => s.id.clone(),
                    None => {
                        let id = new_id();
                        self.sessions.push(Session { id: id.clone(), group: group_from_cwd(&cwd), name: session_name, cwd, windows: vec![], active_window: None, docs: vec![], defaults: AgentDefaults::default(), mcp: vec![], prompt: String::new(), guides: None, skills: vec![], rules: vec![] });
                        id
                    }
                };
                let pid = panel.id.clone();
                self.panels.insert(pid.clone(), panel);
                let w = Window { id: new_id(), name: String::new(), root: Node::Leaf { panel: pid.clone() }, focus: pid };
                let s = self.session_mut(&sid)?;
                s.active_window = Some(w.id.clone());
                s.windows.push(w);
                self.active_session = Some(sid);
            }
            ClosedWhat::Agents { .. } => {}
        }
        Ok(Reopened { docs_from })
    }

    /// Take an entry off the closed list for good (the caller deletes its files).
    pub fn forget(&mut self, id: &str) -> Res<Closed> {
        let idx = self.closed.iter().position(|c| c.id == id).ok_or(ModelError::NotFound("closed"))?;
        Ok(self.closed.remove(idx))
    }

    pub fn rename_session(&mut self, id: &str, name: String) -> Res<()> {
        self.session_mut(id)?.name = name;
        Ok(())
    }

    /// A session's project folder (new terminals and agents start there) and agent defaults.
    pub fn set_session_settings(&mut self, id: &str, cwd: String, defaults: AgentDefaults) -> Res<()> {
        let s = self.session_mut(id)?;
        s.cwd = cwd;
        s.defaults = defaults;
        Ok(())
    }

    pub fn set_panel_title(&mut self, panel: &str, title: String) -> Res<()> {
        self.panels.get_mut(panel).ok_or(ModelError::NotFound("panel"))?.title = title;
        Ok(())
    }

    pub fn set_group(&mut self, id: &str, group: String) -> Res<()> {
        self.session_mut(id)?.group = group;
        Ok(())
    }

    pub fn select_session(&mut self, id: &str) -> Res<()> {
        self.session_mut(id)?;
        self.active_session = Some(id.into());
        Ok(())
    }

    pub fn create_window(&mut self, session: &str, kind: PanelKind) -> Res<String> {
        self.session_mut(session)?;
        let w = self.new_window(kind, String::new());
        let wid = w.id.clone();
        let s = self.session_mut(session)?;
        s.windows.push(w);
        s.active_window = Some(wid.clone());
        Ok(wid)
    }

    pub fn select_window(&mut self, session: &str, window: &str) -> Res<()> {
        let s = self.session_mut(session)?;
        if !s.windows.iter().any(|w| w.id == window) {
            return Err(ModelError::NotFound("window"));
        }
        s.active_window = Some(window.into());
        Ok(())
    }

    pub fn rename_window(&mut self, session: &str, window: &str, name: String) -> Res<()> {
        let s = self.session_mut(session)?;
        let w = s.windows.iter_mut().find(|w| w.id == window).ok_or(ModelError::NotFound("window"))?;
        w.name = name;
        Ok(())
    }

    pub fn close_window(&mut self, session: &str, window: &str) -> Res<Vec<String>> {
        let s = self.session_mut(session)?;
        let idx = s.windows.iter().position(|w| w.id == window).ok_or(ModelError::NotFound("window"))?;
        let w = s.windows.remove(idx);
        if s.active_window.as_deref() == Some(window) {
            s.active_window = s.windows.get(idx.saturating_sub(1)).or(s.windows.first()).map(|w| w.id.clone());
        }
        let removed = w.panel_ids();
        removed.iter().for_each(|p| {
            self.panels.remove(p);
        });
        Ok(removed)
    }

    fn locate_panel(&mut self, panel: &str) -> Res<(usize, usize)> {
        for (si, s) in self.sessions.iter().enumerate() {
            for (wi, w) in s.windows.iter().enumerate() {
                if w.panel_ids().iter().any(|p| p == panel) {
                    return Ok((si, wi));
                }
            }
        }
        Err(ModelError::NotFound("panel"))
    }

    pub fn split_panel(&mut self, panel: &str, dir: Dir, kind: PanelKind) -> Res<String> {
        let (si, wi) = self.locate_panel(panel)?;
        let new_panel = self.new_panel(kind);
        let w = &mut self.sessions[si].windows[wi];
        w.root.split(panel, dir, &new_panel);
        w.focus = new_panel.clone();
        Ok(new_panel)
    }

    /// Move a panel next to another one in the same window (`side` of it), or swap the two (`Center`).
    /// The chats of a session, in its windows' order.
    pub fn chats_of(&self, session: &str) -> Vec<String> {
        let Some(s) = self.sessions.iter().find(|s| s.id == session) else { return vec![] };
        let mut out = vec![];
        for w in &s.windows {
            let mut l = vec![];
            w.root.leaves(&mut l);
            out.extend(l.into_iter().filter(|p| self.panels.get(p).is_some_and(|x| x.kind == PanelKind::Workspace)));
        }
        out
    }

    /// A project to place `to` in the list (the sidebar's order).
    pub fn move_session(&mut self, id: &str, to: usize) -> Res<()> {
        let i = self.sessions.iter().position(|s| s.id == id).ok_or(ModelError::NotFound("session"))?;
        let s = self.sessions.remove(i);
        self.sessions.insert(to.min(self.sessions.len()), s);
        Ok(())
    }

    /// A window to place `to` among its project's (the tabs' order, and the sidebar's).
    pub fn move_window(&mut self, session: &str, window: &str, to: usize) -> Res<()> {
        let s = self.session_mut(session)?;
        let i = s.windows.iter().position(|w| w.id == window).ok_or(ModelError::NotFound("window"))?;
        let w = s.windows.remove(i);
        s.windows.insert(to.min(s.windows.len()), w);
        Ok(())
    }

    /// A window's panels in another order, its splits as they are: each place in the layout gets
    /// the next panel of `order`, which must be the same panels.
    pub fn order_panels(&mut self, window: &str, order: &[String]) -> Res<()> {
        let w = self.sessions.iter_mut().flat_map(|s| s.windows.iter_mut()).find(|w| w.id == window).ok_or(ModelError::NotFound("window"))?;
        let mut now = vec![];
        w.root.leaves(&mut now);
        let (mut a, mut b) = (now.clone(), order.to_vec());
        a.sort();
        b.sort();
        if a != b {
            return Err(ModelError::NotFound("panel"));
        }
        w.root.refill(&mut order.iter());
        Ok(())
    }

    pub fn move_panel(&mut self, panel: &str, target: &str, side: Side) -> Res<()> {
        let (si, wi) = self.locate_panel(panel)?;
        let w = &mut self.sessions[si].windows[wi];
        if panel == target || !w.panel_ids().iter().any(|p| p == target) {
            return Err(ModelError::NotFound("panel"));
        }
        match side {
            Side::Center => w.root.swap(panel, target),
            _ => {
                w.root.remove(panel);
                let dir = if matches!(side, Side::Left | Side::Right) { Dir::Row } else { Dir::Column };
                w.root.insert(target, dir, panel, matches!(side, Side::Left | Side::Top));
            }
        }
        w.focus = panel.into();
        Ok(())
    }

    /// Take a panel out of its window: into a new window of its own right after it (`into` None),
    /// or to the right edge of another window of the session. A window left empty goes away.
    /// Returns the window the panel is in now, which becomes the active one.
    pub fn panel_to_window(&mut self, panel: &str, into: Option<&str>) -> Res<String> {
        let (si, wi) = self.locate_panel(panel)?;
        let s = &mut self.sessions[si];
        let alone = s.windows[wi].panel_ids().len() == 1;
        let to = match into {
            Some(w) if w == s.windows[wi].id => return Err(ModelError::NotFound("window")),
            Some(w) => Some(s.windows.iter().position(|x| x.id == w).ok_or(ModelError::NotFound("window"))?),
            None if alone => return Err(ModelError::NotFound("window")),
            None => None,
        };
        let wid = match to {
            Some(ti) => {
                let w = &mut s.windows[ti];
                w.root.append(Dir::Row, panel);
                w.focus = panel.into();
                w.id.clone()
            }
            None => {
                let w = Window { id: new_id(), name: String::new(), root: Node::Leaf { panel: panel.into() }, focus: panel.into() };
                let id = w.id.clone();
                s.windows.insert(wi + 1, w);
                id
            }
        };
        if alone {
            s.windows.remove(wi);
        } else {
            let w = &mut s.windows[wi];
            w.root.remove(panel);
            if w.focus == panel {
                w.focus = w.panel_ids()[0].clone();
            }
        }
        s.active_window = Some(wid.clone());
        Ok(wid)
    }

    /// Close a panel. If it was the last one in its window, the window closes too.
    pub fn close_panel(&mut self, panel: &str) -> Res<Vec<String>> {
        let (si, wi) = self.locate_panel(panel)?;
        if self.sessions[si].windows[wi].panel_ids().len() == 1 {
            let (sid, wid) = (self.sessions[si].id.clone(), self.sessions[si].windows[wi].id.clone());
            return self.close_window(&sid, &wid);
        }
        let w = &mut self.sessions[si].windows[wi];
        let before = w.panel_ids();
        let pos = before.iter().position(|p| p == panel).unwrap_or(0);
        w.root.remove(panel);
        if w.focus == panel {
            let after = w.panel_ids();
            w.focus = after[pos.saturating_sub(1).min(after.len() - 1)].clone();
        }
        self.panels.remove(panel);
        Ok(vec![panel.to_string()])
    }

    pub fn focus_panel(&mut self, panel: &str) -> Res<()> {
        let (si, wi) = self.locate_panel(panel)?;
        let s = &mut self.sessions[si];
        s.windows[wi].focus = panel.into();
        s.active_window = Some(s.windows[wi].id.clone());
        self.active_session = Some(s.id.clone());
        Ok(())
    }

    pub fn set_panel_kind(&mut self, panel: &str, kind: PanelKind) -> Res<()> {
        self.panels.get_mut(panel).ok_or(ModelError::NotFound("panel"))?.kind = kind;
        Ok(())
    }

    pub fn set_panel_url(&mut self, panel: &str, url: String) -> Res<()> {
        self.panels.get_mut(panel).ok_or(ModelError::NotFound("panel"))?.url = Some(url);
        Ok(())
    }

    pub fn resize(&mut self, window: &str, split: &str, sizes: &[f32]) -> Res<()> {
        let w = self
            .sessions
            .iter_mut()
            .flat_map(|s| s.windows.iter_mut())
            .find(|w| w.id == window)
            .ok_or(ModelError::NotFound("window"))?;
        if w.root.resize(split, sizes) {
            Ok(())
        } else {
            Err(ModelError::NotFound("split"))
        }
    }

    pub fn session_of_panel(&self, panel: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.windows.iter().any(|w| w.panel_ids().iter().any(|p| p == panel)))
    }

    /// Create or replace a doc's metadata. `id: None` (or an unknown id) creates a new doc.
    /// Returns the doc and whether it is new.
    #[allow(clippy::too_many_arguments)]
    pub fn write_doc(&mut self, session: &str, id: Option<&str>, kind: DocKind, title: String, author: String, chat: &str, member: &str, now: u64) -> Res<(Doc, bool)> {
        let s = self.session_mut(session)?;
        if let Some(d) = id.and_then(|id| s.docs.iter_mut().find(|d| d.id == id)) {
            d.kind = kind;
            d.title = title;
            d.author = author;
            d.rev += 1;
            d.updated = now;
            d.chat = chat.into();
            d.member = member.into();
            return Ok((d.clone(), false));
        }
        let d = Doc { id: new_id(), title, kind, author, rev: 1, updated: now, chat: chat.into(), member: member.into() };
        s.docs.push(d.clone());
        Ok((d, true))
    }

    /// Show a doc next to `from` (a panel in the same session): focus the panel already showing
    /// it, otherwise split `from` and open it there. With `focus: false` the keyboard stays put.
    pub fn open_doc(&mut self, from: &str, doc: &str, focus: bool) -> Res<String> {
        let (si, _) = self.locate_panel(from)?;
        if !self.sessions[si].docs.iter().any(|d| d.id == doc) {
            return Err(ModelError::NotFound("doc"));
        }
        self.open_beside(from, doc_url(doc), focus)
    }

    /// A project file in the viewer next to `from` (`path` relative to the session's folder).
    pub fn open_file(&mut self, from: &str, path: &str, focus: bool) -> Res<String> {
        self.open_beside(from, format!("file:{path}"), focus)
    }

    /// Shows `url` (a document or a file) next to `from`: focus the panel already showing it,
    /// else switch a viewer panel (this window's first), else split `from`.
    fn open_beside(&mut self, from: &str, url: String, focus: bool) -> Res<String> {
        let (si, wi) = self.locate_panel(from)?;
        let existing = self.sessions[si]
            .windows
            .iter()
            .flat_map(|w| w.panel_ids())
            .find(|p| self.panels.get(p).is_some_and(|x| x.url.as_deref() == Some(url.as_str())));
        // Not open yet: switch a panel that shows another document or file (this window's first),
        // so they take turns in one panel instead of piling up.
        let is_doc = |p: &String| self.panels.get(p).is_some_and(|x| x.kind == PanelKind::Browser && x.url.as_deref().is_some_and(|u| u.starts_with("doc:") || (u.starts_with("file:") && !u.starts_with("file://"))));
        let windows = &self.sessions[si].windows;
        let reuse = existing.clone().or_else(|| {
            windows[wi].panel_ids().into_iter().find(|p| is_doc(p)).or_else(|| windows.iter().flat_map(|w| w.panel_ids()).find(|p| is_doc(p)))
        });
        if let Some(p) = reuse {
            if existing.is_none() {
                self.set_panel_url(&p, url)?;
            }
            if focus {
                self.focus_panel(&p)?;
            }
            return Ok(p);
        }
        let prev = self.sessions[si].windows[wi].focus.clone();
        let p = self.split_panel(from, Dir::Row, PanelKind::Browser)?;
        self.set_panel_url(&p, url)?;
        if !focus {
            self.sessions[si].windows[wi].focus = prev;
        }
        Ok(p)
    }

    /// The browser panel an agent of chat `from` drives, opened next to the chat the first time
    /// (without taking the focus). Returns it and whether it was just created.
    pub fn agent_browser(&mut self, from: &str, member: &str) -> Res<(String, bool)> {
        let (si, wi) = self.locate_panel(from)?;
        let existing = self.sessions[si]
            .windows
            .iter()
            .flat_map(|w| w.panel_ids())
            .find(|p| self.panels.get(p).is_some_and(|x| x.kind == PanelKind::Browser && x.agent.as_deref() == Some(member)));
        if let Some(p) = existing {
            return Ok((p, false));
        }
        let prev = self.sessions[si].windows[wi].focus.clone();
        let p = self.split_panel(from, Dir::Row, PanelKind::Browser)?;
        self.panels.get_mut(&p).ok_or(ModelError::NotFound("panel"))?.agent = Some(member.to_string());
        self.sessions[si].windows[wi].focus = prev;
        Ok((p, true))
    }

    /// cwd of the session that owns a panel.
    pub fn panel_cwd(&self, panel: &str) -> Option<String> {
        self.sessions
            .iter()
            .find(|s| s.windows.iter().any(|w| w.panel_ids().iter().any(|p| p == panel)))
            .map(|s| s.cwd.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_reordered_keep_the_splits() {
        let leaf = |p: &str| Node::Leaf { panel: p.into() };
        let mut root = Node::Split { id: "s".into(), dir: Dir::Row, children: vec![leaf("a"), Node::Split { id: "t".into(), dir: Dir::Column, children: vec![leaf("b"), leaf("c")], sizes: vec![0.3, 0.7] }], sizes: vec![0.5, 0.5] };
        root.refill(&mut ["c".to_string(), "a".to_string(), "b".to_string()].iter());
        let mut out = vec![];
        root.leaves(&mut out);
        assert_eq!(out, ["c", "a", "b"]);
        let Node::Split { children, sizes, .. } = &root else { panic!() };
        assert_eq!(sizes, &vec![0.5, 0.5]);
        assert!(matches!(&children[1], Node::Split { sizes, .. } if sizes == &vec![0.3, 0.7]));
    }

    fn one_session() -> (State, String) {
        let mut st = State::default();
        let sid = st.create_session("t".into(), "/tmp/proj".into(), PanelKind::Terminal);
        (st, sid)
    }

    fn first_panel(st: &State) -> String {
        st.sessions[0].windows[0].panel_ids()[0].clone()
    }

    #[test]
    fn new_session_has_one_terminal_window() {
        let (st, _) = one_session();
        let s = &st.sessions[0];
        assert_eq!(s.group, "proj");
        assert_eq!(s.windows.len(), 1);
        let p = first_panel(&st);
        assert_eq!(st.panels[&p].kind, PanelKind::Terminal);
        assert_eq!(s.windows[0].focus, p);
    }

    #[test]
    fn same_direction_splits_flatten() {
        let (mut st, _) = one_session();
        let a = first_panel(&st);
        let b = st.split_panel(&a, Dir::Row, PanelKind::Terminal).unwrap();
        let _c = st.split_panel(&b, Dir::Row, PanelKind::Browser).unwrap();
        match &st.sessions[0].windows[0].root {
            Node::Split { children, sizes, .. } => {
                assert_eq!(children.len(), 3);
                assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-5);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn cross_direction_split_nests() {
        let (mut st, _) = one_session();
        let a = first_panel(&st);
        let b = st.split_panel(&a, Dir::Row, PanelKind::Terminal).unwrap();
        st.split_panel(&b, Dir::Column, PanelKind::Terminal).unwrap();
        match &st.sessions[0].windows[0].root {
            Node::Split { children, .. } => assert!(matches!(children[1], Node::Split { dir: Dir::Column, .. })),
            _ => panic!(),
        }
    }

    #[test]
    fn closing_collapses_and_moves_focus() {
        let (mut st, _) = one_session();
        let a = first_panel(&st);
        let b = st.split_panel(&a, Dir::Row, PanelKind::Terminal).unwrap();
        let removed = st.close_panel(&b).unwrap();
        assert_eq!(removed, vec![b.clone()]);
        assert_eq!(st.sessions[0].windows[0].root, Node::Leaf { panel: a.clone() });
        assert_eq!(st.sessions[0].windows[0].focus, a);
        assert!(!st.panels.contains_key(&b));
    }

    #[test]
    fn closing_last_panel_closes_window() {
        let (mut st, sid) = one_session();
        let w2 = st.create_window(&sid, PanelKind::Terminal).unwrap();
        let a = first_panel(&st);
        st.close_panel(&a).unwrap();
        assert_eq!(st.sessions[0].windows.len(), 1);
        assert_eq!(st.sessions[0].windows[0].id, w2);
    }

    #[test]
    fn moving_panels() {
        let (mut st, _) = one_session();
        let a = first_panel(&st);
        let b = st.split_panel(&a, Dir::Row, PanelKind::Terminal).unwrap();
        let c = st.split_panel(&b, Dir::Row, PanelKind::Terminal).unwrap();
        let ids = |st: &State| st.sessions[0].windows[0].panel_ids();
        // [a b c] → c to the left of a → [c a b]
        st.move_panel(&c, &a, Side::Left).unwrap();
        assert_eq!(ids(&st), vec![c.clone(), a.clone(), b.clone()]);
        // swap c and b → [b a c]
        st.move_panel(&c, &b, Side::Center).unwrap();
        assert_eq!(ids(&st), vec![b.clone(), a.clone(), c.clone()]);
        // b below c → [a [c / b]]
        st.move_panel(&b, &c, Side::Bottom).unwrap();
        match &st.sessions[0].windows[0].root {
            Node::Split { children, sizes, .. } => {
                assert_eq!(children.len(), 2);
                assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-5);
                assert!(matches!(&children[1], Node::Split { dir: Dir::Column, children, .. } if children.len() == 2));
            }
            _ => panic!(),
        }
        assert_eq!(ids(&st), vec![a.clone(), c.clone(), b.clone()]);
        assert_eq!(st.sessions[0].windows[0].focus, b);
        assert!(st.move_panel(&a, &a, Side::Left).is_err());
    }

    #[test]
    fn panels_between_windows() {
        let (mut st, _) = one_session();
        let a = first_panel(&st);
        let b = st.split_panel(&a, Dir::Row, PanelKind::Terminal).unwrap();
        let c = st.split_panel(&b, Dir::Column, PanelKind::Terminal).unwrap();
        let w1 = st.sessions[0].windows[0].id.clone();
        // Out of a split window into a window of its own, right after it.
        let w2 = st.panel_to_window(&c, None).unwrap();
        let s = &st.sessions[0];
        assert_eq!(s.windows.iter().map(|w| w.id.clone()).collect::<Vec<_>>(), vec![w1.clone(), w2.clone()]);
        assert_eq!(s.windows[0].panel_ids(), vec![a.clone(), b.clone()]);
        assert_eq!(s.windows[1].panel_ids(), vec![c.clone()]);
        assert_eq!(s.active_window.as_deref(), Some(w2.as_str()));
        assert!(st.panels.contains_key(&c));
        // A window's only panel can't get a window of its own: it already has one.
        assert!(st.panel_to_window(&c, None).is_err());
        assert!(st.panel_to_window(&c, Some(&w2)).is_err());
        // Back into the first window, at its right edge; its own window goes away.
        assert_eq!(st.panel_to_window(&c, Some(&w1)).unwrap(), w1);
        let s = &st.sessions[0];
        assert_eq!(s.windows.len(), 1);
        assert_eq!(s.windows[0].panel_ids(), vec![a.clone(), b.clone(), c.clone()]);
        assert_eq!(s.windows[0].focus, c);
        match &s.windows[0].root {
            Node::Split { sizes, .. } => assert!((sizes.iter().sum::<f32>() - 1.0).abs() < 1e-5),
            _ => panic!(),
        }
        assert_eq!(s.active_window.as_deref(), Some(w1.as_str()));
    }

    #[test]
    fn resize_normalizes() {
        let (mut st, _) = one_session();
        let a = first_panel(&st);
        st.split_panel(&a, Dir::Row, PanelKind::Terminal).unwrap();
        let (wid, sid) = match &st.sessions[0].windows[0].root {
            Node::Split { id, .. } => (st.sessions[0].windows[0].id.clone(), id.clone()),
            _ => panic!(),
        };
        st.resize(&wid, &sid, &[3.0, 1.0]).unwrap();
        match &st.sessions[0].windows[0].root {
            Node::Split { sizes, .. } => assert_eq!(sizes, &vec![0.75, 0.25]),
            _ => panic!(),
        }
        assert!(st.resize(&wid, &sid, &[1.0]).is_err());
    }

    #[test]
    fn close_session_returns_panels_and_keeps_them() {
        let (mut st, sid) = one_session();
        let a = first_panel(&st);
        let b = st.split_panel(&a, Dir::Column, PanelKind::Terminal).unwrap();
        let mut removed = st.close_session(&sid, 1).unwrap();
        removed.sort();
        let mut want = vec![a.clone(), b];
        want.sort();
        assert_eq!(removed, want);
        assert!(st.panels.is_empty());
        assert!(st.active_session.is_none());
        let cid = st.closed[0].id.clone();
        st.reopen(&cid).unwrap();
        assert!(st.closed.is_empty());
        assert_eq!(st.active_session.as_deref(), Some(sid.as_str()));
        assert_eq!(st.panels.len(), 2);
        assert_eq!(first_panel(&st), a);
    }

    #[test]
    fn one_session_per_folder() {
        let (mut st, sid) = one_session();
        let again = st.create_session("x".into(), "/tmp/proj/".into(), PanelKind::Workspace);
        assert_eq!(again, sid);
        assert_eq!(st.sessions.len(), 1);
    }

    #[test]
    fn closed_chat_reopens_in_its_session_or_a_new_one() {
        let (mut st, sid) = one_session();
        let a = first_panel(&st);
        let chat = st.split_panel(&a, Dir::Row, PanelKind::Workspace).unwrap();
        let ids = vec![chat.clone()];
        let closing = st.closing_chats(&ids, 1);
        assert_eq!(closing.len(), 1);
        assert!(st.closing_chats(&[a.clone()], 1).is_empty(), "terminals are not kept");
        st.close_panel(&chat).unwrap();
        st.closed.extend(closing);
        let cid = st.closed[0].id.clone();
        st.reopen(&cid).unwrap();
        assert_eq!(st.session_of_panel(&chat).unwrap().id, sid);
        assert_eq!(st.sessions[0].windows.len(), 2);

        // Its session closed meanwhile: it comes back in a new session on the same folder.
        let closing = st.closing_chats(&[chat.clone()], 2);
        st.close_panel(&chat).unwrap();
        st.closed.extend(closing);
        st.close_session(&sid, 3).unwrap();
        let cid = st.closed[0].id.clone();
        st.reopen(&cid).unwrap();
        let s = st.session_of_panel(&chat).unwrap();
        assert_ne!(s.id, sid);
        assert_eq!(s.cwd, "/tmp/proj");
        // Then the closed session comes back into that one instead of opening the folder twice.
        let cid = st.closed[0].id.clone();
        st.reopen(&cid).unwrap();
        assert_eq!(st.sessions.len(), 1);
        assert_eq!(st.sessions[0].windows.len(), 2);
    }

    #[test]
    fn roundtrip_json() {
        let (mut st, _) = one_session();
        let a = first_panel(&st);
        st.split_panel(&a, Dir::Row, PanelKind::Browser).unwrap();
        let s = serde_json::to_string(&st).unwrap();
        let back: State = serde_json::from_str(&s).unwrap();
        assert_eq!(back.sessions[0].windows[0].root, st.sessions[0].windows[0].root);
    }

    #[test]
    fn agent_browser_opens_beside_the_chat_once() {
        let (mut st, _) = one_session();
        let chat = first_panel(&st);
        let (b, created) = st.agent_browser(&chat, "m1").unwrap();
        assert!(created);
        assert_eq!(st.panels[&b].kind, PanelKind::Browser);
        assert_eq!(st.sessions[0].windows[0].focus, chat, "the chat keeps the focus");
        assert_eq!(st.agent_browser(&chat, "m1").unwrap(), (b.clone(), false));
        let (other, _) = st.agent_browser(&chat, "m2").unwrap();
        assert_ne!(other, b, "each agent gets its own browser");
        st.set_panel_kind(&b, PanelKind::Terminal).unwrap();
        assert!(st.agent_browser(&chat, "m1").unwrap().1, "a panel turned into something else no longer counts");
    }

    #[test]
    fn closing_an_agent_browser_keeps_its_neighbours() {
        let (mut st, _) = one_session();
        let chat = first_panel(&st);
        let doc = st.split_panel(&chat, Dir::Row, PanelKind::Browser).unwrap();
        let term = st.split_panel(&doc, Dir::Column, PanelKind::Terminal).unwrap();
        let (b, _) = st.agent_browser(&chat, "m1").unwrap();
        st.close_panel(&b).unwrap();
        let mut left = st.sessions[0].windows[0].panel_ids();
        left.sort();
        let mut want = vec![chat, doc, term];
        want.sort();
        assert_eq!(left, want);
    }

    #[test]
    fn docs_open_beside_the_chat_once() {
        let mut st = State::default();
        let sid = st.create_session("s".into(), "/tmp".into(), PanelKind::Workspace);
        let chat = st.sessions[0].windows[0].focus.clone();
        let (d, created) = st.write_doc(&sid, None, DocKind::Html, "T".into(), "claude".into(), &chat, "m1", 1).unwrap();
        assert!(created);
        let p = st.open_doc(&chat, &d.id, false).unwrap();
        assert_eq!(st.panels[&p].url.as_deref(), Some(doc_url(&d.id).as_str()));
        assert_eq!(st.sessions[0].windows[0].focus, chat, "focus stays on the chat");
        assert_eq!(st.open_doc(&chat, &d.id, true).unwrap(), p, "reuses the open panel");
        assert_eq!(st.sessions[0].windows[0].focus, p);
        let (d2, created) = st.write_doc(&sid, Some(&d.id), DocKind::Html, "T2".into(), "codex".into(), &chat, "m2", 2).unwrap();
        assert!(!created);
        assert_eq!((d2.rev, d2.author.as_str(), st.sessions[0].docs.len()), (2, "codex", 1));
        let (other, _) = st.write_doc(&sid, None, DocKind::Html, "Other".into(), "claude".into(), &chat, "m1", 3).unwrap();
        assert_eq!(st.open_doc(&chat, &other.id, false).unwrap(), p, "another document switches the same panel");
        assert_eq!(st.panels[&p].url.as_deref(), Some(doc_url(&other.id).as_str()));
        assert_eq!(st.sessions[0].windows[0].panel_ids().len(), 2, "no new split");
    }
}
