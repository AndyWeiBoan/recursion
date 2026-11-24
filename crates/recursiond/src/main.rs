//! recursiond — the long-running Recursion service.
//!
//! Owns the workspace model and all terminal processes. UI clients connect over a
//! localhost WebSocket (port + token are written to ~/.recursion/daemon.json).

mod accounts;
mod agents;
mod browse;
mod diagram;
mod docs;
mod files;
mod i18n;
mod mcp;
mod model;
mod pty;
mod workspace;

use crate::i18n::tr;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State as AxState};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use model::{Dir, PanelKind, State};
use pty::{TermEvent, TermInfo, Terms};
use workspace::AgentKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, Notify};

/// Bump whenever the client↔daemon protocol changes; the app restarts an older daemon.
pub const PROTOCOL: u32 = 28;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn home() -> PathBuf {
    if let Ok(dir) = std::env::var("RECURSION_HOME") {
        return PathBuf::from(dir);
    }
    dirs::home_dir().expect("no home dir").join(".recursion")
}

#[derive(Serialize, Deserialize)]
struct Discovery {
    port: u16,
    token: String,
    pid: u32,
    version: String,
    #[serde(default)]
    protocol: u32,
    /// When it started (ms since epoch): the app replaces it if its binary was rebuilt since.
    #[serde(default)]
    started: u64,
}

struct App {
    state: Mutex<State>,
    terms: Terms,
    token: String,
    /// Fires after every model change: broadcasts a snapshot and schedules a save.
    changed: broadcast::Sender<()>,
    save: Notify,
    term_infos: Mutex<HashMap<String, TermInfo>>,
    hub: Arc<agents::Hub>,
    /// Last time each doc page sent a message (rate limit).
    doc_last: Mutex<HashMap<String, std::time::Instant>>,
    /// Agents' browse_* calls, carried out by the desktop app.
    browser: browse::Bridge,
    /// A command to type into a terminal once it starts (account login windows).
    pending_cmd: Mutex<HashMap<String, String>>,
    /// When this daemon started (ms since epoch).
    started: u64,
    usage_polled: Mutex<Option<std::time::Instant>>,
}

/// Messages from a UI client.
#[derive(Deserialize, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
enum In {
    SessionCreate { name: String, cwd: String, kind: Option<PanelKind> },
    /// Close a session into the closed list.
    SessionClose { session: String },
    /// Open a closed session or chat again.
    ClosedReopen { id: String },
    /// Delete a closed session or chat for good, with its chats' files and docs.
    ClosedDelete { id: String },
    SessionRename { session: String, name: String },
    SessionGroup { session: String, group: String },
    /// A session's project folder and the defaults for agents added to it.
    SessionSettings { session: String, cwd: String, #[serde(default)] defaults: model::AgentDefaults, #[serde(default)] mcp: Option<Vec<String>> },
    /// The MCP settings page: our servers, their last test, what the CLIs have.
    McpList,
    /// Add or change a server (`old`: its name before, when editing).
    McpSave { name: String, #[serde(default)] old: Option<String>, server: mcp::Edit },
    McpRemove { name: String },
    /// Copy servers from Claude's or Codex's settings: (name, where from).
    McpImport { picks: Vec<(String, String)> },
    /// Connect to a server and list its tools.
    McpTest { name: String },
    /// A chat's name, rules and budget.
    WsSettings { panel: String, settings: workspace::ChatSettings },
    SessionSelect { session: String },
    WindowCreate { session: String, kind: Option<PanelKind> },
    WindowSelect { session: String, window: String },
    WindowRename { session: String, window: String, name: String },
    WindowClose { session: String, window: String },
    PanelSplit { panel: String, dir: Dir, kind: Option<PanelKind> },
    PanelClose { panel: String },
    PanelMove { panel: String, target: String, side: model::Side },
    PanelFocus { panel: String },
    PanelKind { panel: String, kind: PanelKind },
    PanelUrl { panel: String, url: String },
    LayoutResize { window: String, split: String, sizes: Vec<f32> },
    TermAttach { panel: String, cols: u16, rows: u16 },
    TermInput { panel: String, data: String },
    TermResize { panel: String, cols: u16, rows: u16 },
    TermRestart { panel: String, cols: u16, rows: u16 },
    WsAttach { panel: String },
    WsAddAgent { panel: String, kind: AgentKind, alias: String, model: Option<String>, account: Option<String>, #[serde(default)] mode: workspace::PermMode },
    WsSetMode { panel: String, member: String, mode: workspace::PermMode },
    /// Have an idle agent compact (summarise) its conversation.
    WsCompact { panel: String, member: String },
    /// The human's language; `None` follows the system.
    SetLang { lang: Option<String> },
    WsRemoveAgent { panel: String, member: String },
    WsRenameAgent { panel: String, member: String, alias: String },
    WsSend { panel: String, text: String, #[serde(default)] reply_to: Option<String>, #[serde(default)] files: Vec<String> },
    /// The project's files, for the 📎 picker.
    FilesList { panel: String },
    /// One project file's content, for the viewer.
    FileGet { panel: String, path: String },
    /// Show a project file next to a panel.
    FileOpen { panel: String, path: String },
    /// Files the human picked from their computer: this chat may use them.
    FilesGrant { panel: String, paths: Vec<String> },
    /// A picture pasted into the composer (base64): kept, granted to the chat, path sent back.
    ImagePaste { panel: String, data: String, mime: String },
    WsPerm { request_id: String, decision: String },
    WsStop { panel: String },
    /// Start a poll; `vote` is the human's own pick (option index), if any.
    PollCreate { panel: String, question: String, #[serde(default)] description: String, options: Vec<String>, #[serde(default)] vote: Option<usize> },
    PollVote { panel: String, poll: String, option: usize },
    PollClose { panel: String, poll: String },
    /// The chat's todo lists, from the human. Lists and items by id.
    TodoListCreate { panel: String, name: String },
    TodoListRename { panel: String, list: String, name: String },
    TodoListRemove { panel: String, list: String },
    TodoAdd { panel: String, list: String, text: String },
    /// Only what's given changes.
    TodoUpdate { panel: String, list: String, id: String, #[serde(default)] text: Option<String>, #[serde(default)] state: Option<workspace::TodoState> },
    /// Dragged to `index` among the list's items, into the 完成區 or out of it.
    TodoMove { panel: String, list: String, id: String, index: usize, archived: bool },
    TodoRemove { panel: String, list: String, id: String },
    /// Stop one agent's reply; the rest of the round goes on.
    WsStopAgent { panel: String, member: String },
    WsSendNow { panel: String },
    ModelsList { kind: AgentKind },
    DocOpen { panel: String, doc: String },
    /// Open an entry of a chat's 產出 list: a document in its panel, a page in the agent's browser.
    ArtifactOpen { panel: String, artifact: String },
    WsRoster,
    ThreadOpen { panel: String, msg: String, quote: String, text: String },
    ThreadSend { panel: String, thread: String, text: String },
    ThreadBring { panel: String, thread: String },
    /// Take a thread, document, page or poll off the 產出 list.
    OutputRemove { panel: String, id: String },
    DocGet { session: String, doc: String },
    DocAction { session: String, doc: String, action: String, data: String },
    AccountsList,
    /// Fetch every account's limits now (the usage panel or settings just opened).
    AccountsRefresh,
    /// For the settings' "about" page.
    DaemonInfo,
    /// Exit cleanly; the desktop app starts a fresh daemon (the current binary) when it reconnects.
    DaemonRestart,
    /// `name` may be empty: the account is then named after its email once logged in.
    AccountCreate { kind: AgentKind, #[serde(default)] name: String },
    AccountLogin { account: String },
    /// Add the login a CLI already has on this computer.
    AccountAdopt { kind: AgentKind },
    /// Logins the CLIs already have here that are not in Recursion yet (setup, add account).
    LoginsDetect,
    /// Which CLIs are installed; `fresh` looks again instead of using what is known.
    CliStatus { #[serde(default)] fresh: bool },
    /// Run a CLI's official installer in a setup terminal.
    CliInstall { kind: AgentKind },
    /// The setup terminal was closed.
    SetupClose { panel: String },
    AccountRename { account: String, name: String },
    AccountRemove { account: String },
    /// This client is the desktop app and can run agents' browser steps.
    BrowserHost,
    BrowseResult { id: u64, ok: bool, result: Option<serde_json::Value>, error: Option<String> },
}

/// Messages to a UI client.
#[derive(Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Out<'a> {
    State { state: &'a State, terms: &'a HashMap<String, TermInfo> },
    Replay { panel: &'a str, data: String },
    Out { panel: &'a str, data: String },
    Exit { panel: &'a str },
    Error { msg: String },
    Models { kind: AgentKind, models: Vec<agents::ModelOption>, error: Option<String> },
    Doc { session: &'a str, doc: &'a str, html: Option<String> },
}

impl App {
    fn snapshot(&self) -> String {
        let st = self.state.lock().unwrap();
        let infos = self.term_infos.lock().unwrap();
        serde_json::to_string(&Out::State { state: &st, terms: &infos }).unwrap()
    }

    fn mark_changed(&self) {
        let _ = self.changed.send(());
        self.save.notify_one();
    }

    fn save_now(&self) {
        let json = serde_json::to_vec_pretty(&*self.state.lock().unwrap()).unwrap();
        let path = home().join("state.json");
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Test a server in the background; the settings page sees "testing", then the result.
    fn mcp_test(&self, name: String) {
        let Some(server) = self.hub.mcps.get(&name) else { return };
        let hub = self.hub.clone();
        let status = |state, tools, error| mcp::Status { state, tools, error, at: workspace::now_ms() };
        hub.mcps.set_status(&name, status("testing", None, None));
        hub.broadcast(hub.mcps.view());
        tokio::spawn(async move {
            let path = hub.env().await.path;
            let s = match mcp::test(&server, &path).await {
                Ok(n) => status("ok", Some(n), None),
                Err(e) => status("failed", None, Some(e)),
            };
            // Changed or removed meanwhile: that result is stale.
            if hub.mcps.get(&name).as_ref() == Some(&server) {
                hub.mcps.set_status(&name, s);
                hub.broadcast(hub.mcps.view());
            }
        });
    }

    /// Stop what runs in these panels (terminals, agents), keeping chats' files.
    async fn stop_panels(&self, panels: &[String]) {
        for p in panels {
            self.terms.kill(p);
            self.hub.close_panel(p, false).await;
        }
    }

    /// Closed panels: chats with something in them go to the closed list, empty ones are deleted.
    async fn close_panels(&self, panels: &[String], chats: Vec<model::Closed>) {
        self.stop_panels(panels).await;
        for c in chats {
            let model::ClosedWhat::Chat { panel, .. } = &c.what else { continue };
            if self.hub.chat_is_empty(&panel.id) {
                self.hub.close_panel(&panel.id, true).await;
            } else {
                self.state.lock().unwrap().closed.push(c);
            }
        }
    }

    /// A file `panel` may show: what its own chat may use, or, for a viewer panel, what any chat
    /// of the same session may use (a file picked from the computer is granted to the chat).
    fn usable_file(&self, panel: &str, path: &str) -> Option<std::path::PathBuf> {
        let (cwd, chats) = {
            let st = self.state.lock().unwrap();
            let s = st.session_of_panel(panel)?;
            let chats: Vec<String> = s.windows.iter().flat_map(|w| w.panel_ids()).filter(|p| st.panels.get(p).is_some_and(|x| x.kind == model::PanelKind::Workspace)).collect();
            (s.cwd.clone(), chats)
        };
        std::iter::once(panel.to_string()).chain(chats).find_map(|p| self.hub.usable_file(&p, &cwd, path))
    }

    /// A setup terminal (not part of any session) that runs `cmd` once the UI shows it. Returns
    /// the message telling the UI to show it, and whether it was started now: one that is still
    /// running is shown again, never started twice (a second CLI login wipes the first).
    fn setup_terminal(&self, panel: String, title: String, cmd: String) -> (String, bool) {
        let running = self.terms.infos().get(&panel).is_some_and(|i| i.alive);
        if !running {
            self.terms.kill(&panel);
            self.pending_cmd.lock().unwrap().insert(panel.clone(), cmd);
        }
        (serde_json::json!({ "t": "setup_panel", "panel": panel, "title": title }).to_string(), !running)
    }

    /// Run the CLI's own login for this account in a setup terminal, then watch until it lands.
    async fn account_login(&self, account: &str) -> Result<String, String> {
        let a = self.hub.accounts.get(account).ok_or_else(|| tr!("err.noAccount"))?;
        let kind_name = if a.kind == AgentKind::Claude { "Claude" } else { "Codex" };
        let title = tr!("acct.loginWindow", kind = kind_name, name = if a.name.is_empty() { tr!("acct.newAccount") } else { a.name.clone() });
        let (reply, started) = self.setup_terminal(format!("setup-login-{}", a.id), title, accounts::login_command(a.kind, &a.id, self.hub.cli_bin(a.kind).await.as_deref()));
        if !started {
            return Ok(reply);
        }
        let hub = self.hub.clone();
        let (id, kind) = (a.id.clone(), a.kind);
        let before = accounts::credentials_mtime(kind, &id);
        tokio::spawn(async move {
            // The human has ten minutes to finish in the browser.
            for _ in 0..200 {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if accounts::credentials_mtime(kind, &id) == before {
                    continue;
                }
                hub.refresh_accounts(Some(&id)).await;
                if hub.accounts.get(&id).is_some_and(|a| a.logged_in) {
                    hub.resolve_relogin(&id);
                    return;
                }
            }
        });
        Ok(reply)
    }

    /// Run a CLI's official installer in a setup terminal; tell the UI when the CLI shows up.
    fn cli_install(&self, kind: AgentKind) -> String {
        let name = if kind == AgentKind::Claude { "Claude Code" } else { "Codex" };
        let (reply, started) = self.setup_terminal(format!("setup-install-{}", if kind == AgentKind::Claude { "claude" } else { "codex" }), tr!("setup.installing", name = name), accounts::install_command(kind).into());
        if started {
            let hub = self.hub.clone();
            tokio::spawn(async move {
                for _ in 0..200 {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let env = hub.redetect().await;
                    if (kind == AgentKind::Claude && env.claude.is_some()) || (kind == AgentKind::Codex && env.codex.is_some()) {
                        hub.broadcast(hub.cli_status(false).await);
                        return;
                    }
                }
            });
        }
        reply
    }

    /// Apply one client command. Returns an optional direct reply for the sender.
    async fn handle(&self, msg: In) -> Result<Option<String>, String> {
        use In::*;
        let e = |e: model::ModelError| e.to_string();
        let mut changed = true;
        let mut reply = None;
        match msg {
            SessionCreate { name, cwd, kind } => {
                let mut st = self.state.lock().unwrap();
                let before = st.sessions.len();
                let id = st.create_session(name, cwd, kind.unwrap_or(model::PanelKind::Terminal));
                // A new one starts with the MCP servers the last session saved turned on.
                if st.sessions.len() > before {
                    if let Some(s) = st.sessions.iter_mut().find(|s| s.id == id) {
                        s.mcp = self.hub.mcps.last();
                    }
                }
            }
            SessionClose { session } => {
                let removed = self.state.lock().unwrap().close_session(&session, workspace::now_ms()).map_err(e)?;
                self.stop_panels(&removed).await;
            }
            ClosedReopen { id } => {
                let r = self.state.lock().unwrap().reopen(&id).map_err(e)?;
                if let Some((from, to)) = r.docs_from {
                    docs::move_session(&from, &to);
                }
            }
            ClosedDelete { id } => {
                let c = self.state.lock().unwrap().forget(&id).map_err(e)?;
                match c.what {
                    model::ClosedWhat::Session { session, panels } => {
                        for p in panels.iter().filter(|p| p.kind == model::PanelKind::Workspace) {
                            self.hub.close_panel(&p.id, true).await;
                        }
                        docs::delete_session(&session.id);
                    }
                    model::ClosedWhat::Chat { panel, .. } => self.hub.close_panel(&panel.id, true).await,
                }
            }
            SessionRename { session, name } => self.state.lock().unwrap().rename_session(&session, name).map_err(e)?,
            SessionGroup { session, group } => self.state.lock().unwrap().set_group(&session, group).map_err(e)?,
            SessionSettings { session, cwd, defaults, mcp } => {
                let cwd = cwd.trim().to_string();
                if !std::path::Path::new(&cwd).is_dir() {
                    return Err(i18n::tr!("session.err.noFolder", path = cwd));
                }
                let mut st = self.state.lock().unwrap();
                if let Some(other) = st.sessions.iter().find(|s| s.id != session && model::same_dir(&s.cwd, &cwd)) {
                    return Err(i18n::tr!("session.err.folderOpen", name = other.name));
                }
                st.set_session_settings(&session, cwd, defaults).map_err(e)?;
                if let Some(names) = mcp {
                    let known = self.hub.mcps.names();
                    let names: Vec<String> = names.into_iter().filter(|n| known.contains(n)).collect();
                    self.hub.mcps.set_last(&names);
                    st.sessions.iter_mut().filter(|s| s.id == session).for_each(|s| s.mcp = names.clone());
                }
            }
            WsSettings { panel, settings } => {
                self.state.lock().unwrap().set_panel_title(&panel, settings.name.trim().to_string()).map_err(e)?;
                self.hub.set_chat_settings(&panel, settings);
            }
            SessionSelect { session } => self.state.lock().unwrap().select_session(&session).map_err(e)?,
            WindowCreate { session, kind } => {
                self.state.lock().unwrap().create_window(&session, kind.unwrap_or(model::PanelKind::Terminal)).map_err(e)?;
            }
            WindowSelect { session, window } => self.state.lock().unwrap().select_window(&session, &window).map_err(e)?,
            WindowRename { session, window, name } => {
                self.state.lock().unwrap().rename_window(&session, &window, name).map_err(e)?
            }
            WindowClose { session, window } => {
                let (removed, chats) = {
                    let mut st = self.state.lock().unwrap();
                    let ids = st.sessions.iter().find(|s| s.id == session).and_then(|s| s.windows.iter().find(|w| w.id == window)).map(|w| w.panel_ids()).unwrap_or_default();
                    let chats = st.closing_chats(&ids, workspace::now_ms());
                    (st.close_window(&session, &window).map_err(e)?, chats)
                };
                self.close_panels(&removed, chats).await;
            }
            PanelSplit { panel, dir, kind } => {
                self.state.lock().unwrap().split_panel(&panel, dir, kind.unwrap_or(model::PanelKind::Terminal)).map_err(e)?;
            }
            PanelClose { panel } => {
                let (removed, chats) = {
                    let mut st = self.state.lock().unwrap();
                    let chats = st.closing_chats(std::slice::from_ref(&panel), workspace::now_ms());
                    (st.close_panel(&panel).map_err(e)?, chats)
                };
                self.close_panels(&removed, chats).await;
            }
            PanelMove { panel, target, side } => self.state.lock().unwrap().move_panel(&panel, &target, side).map_err(e)?,
            PanelFocus { panel } => self.state.lock().unwrap().focus_panel(&panel).map_err(e)?,
            PanelKind { panel, kind } => {
                self.state.lock().unwrap().set_panel_kind(&panel, kind).map_err(e)?;
                if kind != model::PanelKind::Terminal {
                    self.terms.kill(&panel);
                }
                if kind != model::PanelKind::Workspace {
                    self.hub.close_panel(&panel, false).await;
                }
            }
            PanelUrl { panel, url } => self.state.lock().unwrap().set_panel_url(&panel, url).map_err(e)?,
            LayoutResize { window, split, sizes } => self.state.lock().unwrap().resize(&window, &split, &sizes).map_err(e)?,
            TermAttach { panel, cols, rows } | TermRestart { panel, cols, rows } => {
                let cwd = if panel.starts_with("setup-") {
                    dirs::home_dir().unwrap_or_default().to_string_lossy().to_string()
                } else {
                    self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?
                };
                let back = self.terms.ensure(&panel, &cwd, cols.max(2), rows.max(2)).map_err(|e| e.to_string())?;
                if let Some(cmd) = self.pending_cmd.lock().unwrap().remove(&panel) {
                    self.terms.write(&panel, format!("{cmd}\r").as_bytes());
                }
                reply = Some(serde_json::to_string(&Out::Replay { panel: &panel, data: B64.encode(back) }).unwrap());
                changed = false;
            }
            TermInput { panel, data } => {
                self.terms.write(&panel, data.as_bytes());
                changed = false;
            }
            TermResize { panel, cols, rows } => {
                self.terms.resize(&panel, cols.max(2), rows.max(2));
                changed = false;
            }
            WsAttach { panel } => {
                // Documents from before the 產出 list existed.
                let docs: Vec<model::Doc> = self.state.lock().unwrap().session_of_panel(&panel).map(|s| s.docs.iter().filter(|d| d.chat == panel).cloned().collect()).unwrap_or_default();
                self.hub.adopt_docs(&panel, &docs);
                reply = Some(self.hub.snapshot(&panel).to_string());
                changed = false;
            }
            ThreadOpen { panel, msg, quote, text } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                let id = self.hub.thread_open(&panel, &msg, &quote, &text, cwd).await?;
                reply = Some(serde_json::json!({ "t": "thread_opened", "panel": panel, "thread": id }).to_string());
                changed = false;
            }
            ThreadSend { panel, thread, text } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                self.hub.thread_send(&panel, &thread, &text, cwd).await?;
                changed = false;
            }
            OutputRemove { panel, id } => {
                self.hub.output_remove(&panel, &id)?;
                changed = false;
            }
            ThreadBring { panel, thread } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                self.hub.thread_bring(&panel, &thread, cwd).await?;
                changed = false;
            }
            WsRoster => {
                let panels: Vec<String> = {
                    let st = self.state.lock().unwrap();
                    st.panels.values().filter(|p| p.kind == model::PanelKind::Workspace).map(|p| p.id.clone()).collect()
                };
                reply = Some(self.hub.roster(&panels).to_string());
                changed = false;
            }
            DocOpen { panel, doc } => {
                self.state.lock().unwrap().open_doc(&panel, &doc, true).map_err(e)?;
            }
            ArtifactOpen { panel, artifact } => {
                let a = self.hub.artifact(&panel, &artifact).ok_or_else(|| tr!("err.noOutput"))?;
                let mut st = self.state.lock().unwrap();
                match a.kind {
                    workspace::ArtifactKind::Doc | workspace::ArtifactKind::Diagram => {
                        st.open_doc(&panel, &a.target, true).map_err(e)?;
                    }
                    workspace::ArtifactKind::Page => {
                        let (p, _) = st.agent_browser(&panel, &a.by).map_err(e)?;
                        st.set_panel_url(&p, a.target).map_err(e)?;
                        st.focus_panel(&p).map_err(e)?;
                    }
                }
            }
            DocAction { session, doc, action, data } => {
                self.doc_action(&session, &doc, &action, &data).await?;
                changed = false;
            }
            BrowserHost => changed = false, // handled per connection in `client`
            BrowseResult { id, ok, result, error } => {
                let r = if ok { Ok(result.unwrap_or_default()) } else { Err(error.unwrap_or_else(|| "browser error".into())) };
                self.browser.resolve(id, r);
                changed = false;
            }
            DocGet { session, doc } => {
                reply = Some(serde_json::to_string(&Out::Doc { session: &session, doc: &doc, html: docs::read(&session, &doc) }).unwrap());
                changed = false;
            }
            ModelsList { kind } => {
                let (models, error) = match self.hub.models(kind).await {
                    Ok(m) => (m, None),
                    Err(err) => (vec![], Some(err)),
                };
                reply = Some(serde_json::to_string(&Out::Models { kind, models, error }).unwrap());
                changed = false;
            }
            DaemonInfo => {
                reply = Some(
                    serde_json::json!({
                        "t": "daemon_info",
                        "version": env!("CARGO_PKG_VERSION"),
                        "protocol": PROTOCOL,
                        "pid": std::process::id(),
                        "home": home(),
                        "exe": std::env::current_exe().ok(),
                        "started": self.started,
                    })
                    .to_string(),
                );
                changed = false;
            }
            DaemonRestart => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(std::process::id() as i32, libc::SIGTERM);
                }
                changed = false;
            }
            AccountsRefresh => {
                // Opening the panel twice in a row must not hammer the providers.
                let due = {
                    let mut last = self.usage_polled.lock().unwrap();
                    let due = last.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(30));
                    if due {
                        *last = Some(std::time::Instant::now());
                    }
                    due
                };
                if due {
                    let hub = self.hub.clone();
                    tokio::spawn(async move { hub.poll_usage().await });
                }
                changed = false;
            }
            AccountsList => {
                reply = Some(serde_json::json!({ "t": "accounts", "accounts": self.hub.accounts.list() }).to_string());
                changed = false;
            }
            AccountCreate { kind, name } => {
                let a = self.hub.accounts.create(kind, &name)?;
                self.hub.emit_accounts();
                reply = Some(self.account_login(&a.id).await?);
                changed = false;
            }
            AccountLogin { account } => {
                reply = Some(self.account_login(&account).await?);
                changed = false;
            }
            AccountAdopt { kind } => {
                let a = self.hub.accounts.adopt(kind);
                self.hub.refresh_accounts(Some(&a.id)).await;
                self.hub.emit_accounts();
                changed = false;
            }
            LoginsDetect => {
                reply = Some(self.hub.detect_logins().await.to_string());
                changed = false;
            }
            McpList => {
                reply = Some(self.hub.mcps.view().to_string());
                // First look since the daemon started: see whether they connect.
                for name in self.hub.mcps.untested() {
                    self.mcp_test(name);
                }
                changed = false;
            }
            McpSave { name, old, server } => {
                let name = name.trim().to_string();
                self.hub.mcps.save_server(&name, old.as_deref(), server)?;
                if let Some(o) = old.filter(|o| *o != name) {
                    self.state.lock().unwrap().sessions.iter_mut().flat_map(|s| s.mcp.iter_mut()).filter(|n| **n == o).for_each(|n| *n = name.clone());
                }
                self.mcp_test(name);
            }
            McpRemove { name } => {
                self.hub.mcps.remove(&name);
                self.state.lock().unwrap().sessions.iter_mut().for_each(|s| s.mcp.retain(|n| *n != name));
                self.hub.broadcast(self.hub.mcps.view());
            }
            McpImport { picks } => {
                for name in self.hub.mcps.import(&picks) {
                    self.mcp_test(name);
                }
                changed = false;
            }
            McpTest { name } => {
                self.mcp_test(name);
                changed = false;
            }
            CliStatus { fresh } => {
                reply = Some(self.hub.cli_status(fresh).await.to_string());
                changed = false;
            }
            CliInstall { kind } => {
                reply = Some(self.cli_install(kind));
                changed = false;
            }
            SetupClose { panel } => {
                if panel.starts_with("setup-") {
                    self.terms.kill(&panel);
                    self.pending_cmd.lock().unwrap().remove(&panel);
                }
                changed = false;
            }
            AccountRename { account, name } => {
                self.hub.accounts.rename(&account, &name)?;
                self.hub.emit_accounts();
                changed = false;
            }
            AccountRemove { account } => {
                let chats: Vec<String> = {
                    let st = self.state.lock().unwrap();
                    st.panels.values().filter(|p| p.kind == model::PanelKind::Workspace).map(|p| p.id.clone()).collect()
                };
                if let Some(who) = self.hub.account_in_use(&account, &chats) {
                    return Err(tr!("acct.err.inUse", who = who));
                }
                self.hub.accounts.remove(&account)?;
                self.hub.emit_accounts();
                changed = false;
            }
            WsAddAgent { panel, kind, alias, model, account, mode } => {
                self.hub.add_agent(&panel, kind, alias, model, account, mode).await?;
                changed = false;
            }
            WsCompact { panel, member } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                self.hub.compact(&panel, &member, cwd).await?;
                changed = false;
            }
            SetLang { lang } => {
                i18n::set_lang(lang.as_deref());
                self.state.lock().unwrap().lang = lang;
            }
            WsSetMode { panel, member, mode } => {
                self.hub.set_mode(&panel, &member, mode).await?;
                changed = false;
            }
            WsRenameAgent { panel, member, alias } => {
                self.hub.rename_agent(&panel, &member, &alias).await?;
                changed = false;
            }
            WsRemoveAgent { panel, member } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel);
                self.hub.remove_agent(&panel, &member, cwd).await;
                changed = false;
            }
            WsSend { panel, text, reply_to, files } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                self.hub.send(&panel, text, cwd, None, reply_to.as_deref(), files).await?;
                changed = false;
            }
            FilesList { panel } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                reply = Some(serde_json::json!({ "t": "files", "panel": panel, "files": files::list(&cwd).await }).to_string());
                changed = false;
            }
            FileGet { panel, path } => {
                reply = Some(serde_json::json!({ "t": "file", "panel": panel, "path": path, "file": files::read(self.usable_file(&panel, &path)) }).to_string());
                changed = false;
            }
            FilesGrant { panel, paths } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                reply = Some(serde_json::json!({ "t": "files_granted", "panel": panel, "paths": self.hub.grant(&panel, &cwd, &paths) }).to_string());
                changed = false;
            }
            ImagePaste { panel, data, mime } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                let ext = match mime.as_str() {
                    "image/png" => "png",
                    "image/jpeg" => "jpg",
                    "image/gif" => "gif",
                    "image/webp" => "webp",
                    _ => return Err(i18n::t("image.err.type")),
                };
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| e.to_string())?;
                if bytes.len() > 20 * 1024 * 1024 {
                    return Err(i18n::t("image.err.big"));
                }
                let dir = home().join("images");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(format!("{}.{ext}", model::new_id()));
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                let paths = self.hub.grant(&panel, &cwd, &[path.to_string_lossy().to_string()]);
                reply = Some(serde_json::json!({ "t": "files_granted", "panel": panel, "paths": paths }).to_string());
                changed = false;
            }
            FileOpen { panel, path } => {
                self.usable_file(&panel, &path).ok_or_else(|| i18n::t("file.err.outside"))?;
                self.state.lock().unwrap().open_file(&panel, &path, true).map_err(e)?;
            }
            WsPerm { request_id, decision } => {
                self.hub.decide(&request_id, &decision).await?;
                changed = false;
            }
            WsStop { panel } => {
                self.hub.stop(&panel).await;
                changed = false;
            }
            PollCreate { panel, question, description, options, vote } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                self.hub.poll_create(&panel, &question, &description, options, vote, cwd).await?;
                changed = false;
            }
            PollVote { panel, poll, option } => {
                self.hub.poll_vote(&panel, &poll, "you", option, "")?;
                changed = false;
            }
            PollClose { panel, poll } => {
                self.hub.poll_close(&panel, &poll);
                changed = false;
            }
            TodoListCreate { panel, name } => {
                self.hub.todo_list_create(&panel, &name)?;
                changed = false;
            }
            TodoListRename { panel, list, name } => {
                self.hub.todo_list_rename(&panel, &list, &name)?;
                changed = false;
            }
            TodoListRemove { panel, list } => {
                self.hub.todo_list_remove(&panel, &list);
                changed = false;
            }
            TodoAdd { panel, list, text } => {
                self.hub.todo_add(&panel, &list, "you", &text)?;
                changed = false;
            }
            TodoUpdate { panel, list, id, text, state } => {
                self.hub.todo_update(&panel, &list, &id, text, state)?;
                changed = false;
            }
            TodoMove { panel, list, id, index, archived } => {
                self.hub.todo_move(&panel, &list, &id, index, archived)?;
                changed = false;
            }
            TodoRemove { panel, list, id } => {
                self.hub.todo_remove(&panel, &list, &id)?;
                changed = false;
            }
            WsStopAgent { panel, member } => {
                self.hub.stop_member(&panel, &member).await;
                changed = false;
            }
            WsSendNow { panel } => {
                let cwd = self.state.lock().unwrap().panel_cwd(&panel).ok_or("panel not found")?;
                self.hub.send_now(&panel, cwd).await;
                changed = false;
            }
        }
        if changed {
            self.mark_changed();
        }
        Ok(reply)
    }
}

#[derive(Deserialize)]
struct Auth {
    token: String,
}

async fn health(AxState(app): AxState<Arc<App>>, Query(q): Query<Auth>) -> impl IntoResponse {
    if q.token == app.token {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::UNAUTHORIZED, "bad token")
    }
}

async fn ws(ws: WebSocketUpgrade, AxState(app): AxState<Arc<App>>, Query(q): Query<Auth>) -> impl IntoResponse {
    if q.token != app.token {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }
    ws.on_upgrade(move |socket| client(socket, app)).into_response()
}

async fn client(socket: WebSocket, app: Arc<App>) {
    static NEXT_CLIENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let me = NEXT_CLIENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut tx, mut rx) = socket.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let _ = out_tx.send(app.snapshot());

    // Fan-in: model changes + terminal output → this client.
    let mut changed = app.changed.subscribe();
    let mut term_rx = app.terms.tx.subscribe();
    let mut hub_rx = app.hub.tx.subscribe();
    let fan = {
        let (app, out_tx) = (app.clone(), out_tx.clone());
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    r = changed.recv() => match r {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => { let _ = out_tx.send(app.snapshot()); }
                        Err(_) => break,
                    },
                    r = hub_rx.recv() => match r {
                        Ok(s) => { let _ = out_tx.send(s); }
                        Err(broadcast::error::RecvError::Lagged(n)) => { tracing::warn!(n, "client lagged on workspace events"); }
                        Err(_) => break,
                    },
                    r = term_rx.recv() => match r {
                        Ok(TermEvent::Output { panel, data }) => {
                            let _ = out_tx.send(serde_json::to_string(&Out::Out { panel: &panel, data: B64.encode(data) }).unwrap());
                        }
                        Ok(TermEvent::Exit { panel }) => {
                            let _ = out_tx.send(serde_json::to_string(&Out::Exit { panel: &panel }).unwrap());
                            app.mark_changed();
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => break,
                    },
                }
            }
        })
    };
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if tx.send(Message::Text(m.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = rx.next().await {
        let Message::Text(text) = msg else { continue };
        let reply = match serde_json::from_str::<In>(&text) {
            Ok(In::BrowserHost) => {
                app.browser.add_host(me, out_tx.clone());
                None
            }
            Ok(cmd) => app.handle(cmd).await.unwrap_or_else(|e| Some(serde_json::to_string(&Out::Error { msg: e }).unwrap())),
            Err(e) => Some(serde_json::to_string(&Out::Error { msg: format!("bad message: {e}") }).unwrap()),
        };
        if let Some(r) = reply {
            let _ = out_tx.send(r);
        }
    }
    app.browser.remove_host(me);
    fan.abort();
    writer.abort();
}

fn load_state() -> State {
    let st: State = std::fs::read(home().join("state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    i18n::set_lang(st.lang.as_deref());
    st
}

/// Is another daemon already serving? (discovery file + live health check)
async fn existing_daemon() -> bool {
    let Ok(raw) = std::fs::read(home().join("daemon.json")) else { return false };
    let Ok(d) = serde_json::from_slice::<Discovery>(&raw) else { return false };
    if d.protocol != PROTOCOL {
        // Older daemon still running: ask it to exit so this one can take over.
        #[cfg(unix)]
        unsafe {
            libc::kill(d.pid as i32, libc::SIGTERM);
        }
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        return false;
    }
    let url = format!("127.0.0.1:{}", d.port);
    let Ok(Ok(mut s)) = tokio::time::timeout(std::time::Duration::from_millis(500), tokio::net::TcpStream::connect(&url)).await else {
        return false;
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let req = format!("GET /health?token={} HTTP/1.0\r\nHost: localhost\r\n\r\n", d.token);
    if s.write_all(req.as_bytes()).await.is_err() {
        return false;
    }
    let mut buf = vec![0u8; 256];
    let n = tokio::time::timeout(std::time::Duration::from_millis(500), s.read(&mut buf)).await.ok().and_then(|r| r.ok()).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).contains(" 200 ")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())).init();
    std::fs::create_dir_all(home())?;
    if existing_daemon().await {
        tracing::info!("recursiond already running");
        return Ok(());
    }

    let token: String = {
        use rand::Rng;
        rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(32).map(char::from).collect()
    };
    let (changed, _) = broadcast::channel(64);
    let app = Arc::new(App {
        state: Mutex::new(load_state()),
        terms: Terms::new(),
        token: token.clone(),
        changed,
        save: Notify::new(),
        term_infos: Mutex::new(HashMap::new()),
        hub: agents::Hub::new(),
        doc_last: Default::default(),
        browser: Default::default(),
        pending_cmd: Default::default(),
        started: workspace::now_ms(),
        usage_polled: Default::default(),
    });
    // Who is logged in where (emails, plans), without holding up the start.
    {
        let hub = app.hub.clone();
        tokio::spawn(async move {
            hub.refresh_accounts(None).await;
            // Then the limits, every five minutes.
            loop {
                hub.poll_usage().await;
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });
    }

    // Debounced saver.
    {
        let app = app.clone();
        tokio::spawn(async move {
            loop {
                app.save.notified().await;
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                app.save_now();
            }
        });
    }
    // Debounced transcript saver.
    {
        let hub = app.hub.clone();
        tokio::spawn(async move {
            loop {
                hub.save_notify.notified().await;
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                hub.flush();
            }
        });
    }
    // Foreground-process poller (lets the UI hand Ctrl+H/J/K/L to nvim).
    {
        let app = app.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                let infos = app.terms.infos();
                let differs = *app.term_infos.lock().unwrap() != infos;
                if differs {
                    *app.term_infos.lock().unwrap() = infos;
                    let _ = app.changed.send(());
                }
            }
        });
    }

    let port: u16 = std::env::var("RECURSION_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let port = listener.local_addr()?.port();
    let disc = Discovery { port, token, pid: std::process::id(), version: env!("CARGO_PKG_VERSION").into(), protocol: PROTOCOL, started: app.started };
    let disc_path = home().join("daemon.json");
    std::fs::write(&disc_path, serde_json::to_vec_pretty(&disc)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&disc_path, std::fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!(port, "recursiond listening");
    app.hub.set_port(port);
    {
        let app = Arc::downgrade(&app);
        app.upgrade().unwrap().hub.set_mcp_lookup(move |panel| {
            app.upgrade().and_then(|a| a.state.lock().unwrap().session_of_panel(panel).map(|s| s.mcp.clone())).unwrap_or_default()
        });
    }

    let router = Router::new().route("/health", get(health)).route("/ws", get(ws))
        .route("/mcp/{key}", post(docs::mcp).get(docs::mcp_get)).with_state(app.clone());
    let shutdown = {
        let app = app.clone();
        async move {
            #[cfg(unix)]
            {
                let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            app.save_now();
            app.terms.kill_all();
            app.hub.shutdown().await;
            let _ = std::fs::remove_file(home().join("daemon.json"));
        }
    };
    axum::serve(listener, router).with_graceful_shutdown(shutdown).await?;
    Ok(())
}
