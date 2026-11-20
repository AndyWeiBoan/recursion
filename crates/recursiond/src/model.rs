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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub sessions: Vec<Session>,
    pub panels: HashMap<String, Panel>,
    pub active_session: Option<String>,
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

    /// Split the leaf holding `target`, putting `new_panel` after it in direction `dir`.
    /// Returns true if the target was found.
    fn split(&mut self, target: &str, dir: Dir, new_panel: &str) -> bool {
        match self {
            Node::Leaf { panel } if panel == target => {
                let old = Node::Leaf { panel: panel.clone() };
                *self = Node::Split {
                    id: new_id(),
                    dir,
                    children: vec![old, Node::Leaf { panel: new_panel.to_string() }],
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
                        sizes[i] = half;
                        sizes.insert(i + 1, half);
                        children.insert(i + 1, Node::Leaf { panel: new_panel.to_string() });
                        return true;
                    }
                }
                children.iter_mut().any(|c| c.split(target, dir, new_panel))
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
        self.panels.insert(id.clone(), Panel { id: id.clone(), kind, url: None });
        id
    }

    fn new_window(&mut self, kind: PanelKind, name: String) -> Window {
        let panel = self.new_panel(kind);
        Window { id: new_id(), name, root: Node::Leaf { panel: panel.clone() }, focus: panel }
    }

    pub fn create_session(&mut self, name: String, cwd: String, kind: PanelKind) -> String {
        let window = self.new_window(kind, String::new());
        let id = new_id();
        let session = Session {
            id: id.clone(),
            group: group_from_cwd(&cwd),
            name,
            cwd,
            active_window: Some(window.id.clone()),
            windows: vec![window],
        };
        self.sessions.push(session);
        self.active_session = Some(id.clone());
        id
    }

    /// Returns the ids of panels that were removed (so the caller can kill their processes).
    pub fn delete_session(&mut self, id: &str) -> Res<Vec<String>> {
        let idx = self.sessions.iter().position(|s| s.id == id).ok_or(ModelError::NotFound("session"))?;
        let s = self.sessions.remove(idx);
        let removed: Vec<String> = s.windows.iter().flat_map(|w| w.panel_ids()).collect();
        removed.iter().for_each(|p| {
            self.panels.remove(p);
        });
        if self.active_session.as_deref() == Some(id) {
            self.active_session = self.sessions.first().map(|s| s.id.clone());
        }
        Ok(removed)
    }

    pub fn rename_session(&mut self, id: &str, name: String) -> Res<()> {
        self.session_mut(id)?.name = name;
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
    fn delete_session_returns_panels() {
        let (mut st, sid) = one_session();
        let a = first_panel(&st);
        let b = st.split_panel(&a, Dir::Column, PanelKind::Terminal).unwrap();
        let mut removed = st.delete_session(&sid).unwrap();
        removed.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(removed, want);
        assert!(st.panels.is_empty());
        assert!(st.active_session.is_none());
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
}
