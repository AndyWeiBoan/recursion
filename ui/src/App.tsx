import { useEffect, useMemo, useState } from "react";
import { t } from "./i18n";
import { getSnapshot, isMac, pickFolder, send, useAccounts, useRoster, useSnapshot, useToast } from "./client";
import { KindPicker, type PickRequest } from "./KindPicker";
import { LayoutView, useDragWidth } from "./Layout";
import { closePicker, pickerOpen, registerPicker, requestKind } from "./picker";
import { pruneTerminals } from "./terminals";
import { TodoCardLayer } from "./TodoCard";
import { listenZoomKeys, restoreZoom, zoom } from "./zoom";
import { listenForDrops } from "./drop";
import { overlay, pruneBrowsers, viewerPath, zoomBrowser } from "./browsers";
import { startTabDrag, usePanelDrag } from "./panelDrag";
import { describe, InlineEdit, PanelContextMenu, panelNumbers, SessionTree } from "./SessionTree";
import { ago, openSettings, SettingsModal } from "./Settings";
import { openSessionSettings, SessionSettingsModal } from "./ScopeSettings";
import { UsagePanel } from "./Usage";
import { UpdateNotice } from "./Update";
import { Onboarding, SetupTerminal } from "./Setup";
import { closeThreads, useZoomed } from "./threadsUI";
import { chatsOf, leaves, type Closed, type PanelKind, type Session, type State, type TermInfo } from "./types";
import { Icon } from "./icons";
import { closeChecked } from "./closing";

const VIM_LIKE = new Set(["nvim", "vim", "vi", "view", "nvim-qt"]);

function activeOf(state: State | null) {
  const session = state?.sessions.find((s) => s.id === state.active_session) ?? state?.sessions[0];
  const window = session?.windows.find((w) => w.id === session.active_window) ?? session?.windows[0];
  return { session, window };
}

async function newSession() {
  const cwd = await pickFolder();
  if (!cwd) return;
  // One session per folder: go to the one already open there.
  const trim = (p: string) => p.replace(/\/+$/, "");
  const open = getSnapshot().state?.sessions.find((s) => trim(s.cwd) === trim(cwd));
  if (open) return send({ t: "session_select", session: open.id });
  const name = cwd.replace(/\/+$/, "").split("/").pop() || cwd;
  requestKind(t("session.openIn", { name }), null, (kind) => send({ t: "session_create", name, cwd, kind }));
}

/** Move focus to the nearest panel in a direction, by on-screen geometry. */
function focusNeighbor(dir: "h" | "j" | "k" | "l") {
  const { window } = activeOf(getSnapshot().state);
  if (!window) return;
  const cur = document.querySelector<HTMLElement>(`[data-panel="${window.focus}"]`);
  if (!cur) return;
  const a = cur.getBoundingClientRect();
  let best: { id: string; score: number } | null = null;
  document.querySelectorAll<HTMLElement>("[data-panel]").forEach((el) => {
    const id = el.dataset.panel!;
    if (id === window.focus) return;
    const b = el.getBoundingClientRect();
    const ok =
      dir === "h" ? b.right <= a.left + 2 : dir === "l" ? b.left >= a.right - 2 : dir === "k" ? b.bottom <= a.top + 2 : b.top >= a.bottom - 2;
    if (!ok) return;
    const horizontal = dir === "h" || dir === "l";
    const gap = horizontal ? Math.abs((dir === "h" ? a.left - b.right : b.left - a.right)) : Math.abs(dir === "k" ? a.top - b.bottom : b.top - a.bottom);
    const overlap = horizontal
      ? Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top))
      : Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left));
    const score = gap * 10 - overlap;
    if (!best || score < best.score) best = { id, score };
  });
  if (best) send({ t: "panel_focus", panel: (best as { id: string }).id });
}

function useShortcuts(toggleSidebar: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (pickerOpen()) return;
      const { state, terms } = getSnapshot();
      const { session, window } = activeOf(state);
      const k = e.key.toLowerCase();
      const mod = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && e.shiftKey;
      const stop = () => {
        e.preventDefault();
        e.stopPropagation();
      };

      // Ctrl+H/J/K/L: move between panels, unless nvim/vim is in front of the focused terminal.
      if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && ["h", "j", "k", "l"].includes(k)) {
        const panel = window && state?.panels[window.focus];
        const fg = panel?.kind === "terminal" ? terms[panel.id]?.fg ?? "" : "";
        if (VIM_LIKE.has(fg)) return;
        stop();
        focusNeighbor(k as "h" | "j" | "k" | "l");
        return;
      }
      // ⌘= ⌘- ⌘0 / Ctrl+= Ctrl+- Ctrl+0: zoom the whole app, like a browser (and ghostty on Linux).
      // By the character as well as the key's place: other layouts (and typed-in keys) have other codes.
      const zoomStep = ["=", "+"].includes(e.key) || ["Equal", "NumpadAdd"].includes(e.code) ? 1 : ["-", "_"].includes(e.key) || ["Minus", "NumpadSubtract"].includes(e.code) ? -1 : e.key === "0" || e.code === "Digit0" || e.code === "Numpad0" ? 0 : null;
      if ((isMac ? e.metaKey : e.ctrlKey) && !e.altKey && zoomStep !== null) {
        stop();
        zoom(zoomStep as -1 | 0 | 1);
        return;
      }
      // ⌘, / Ctrl+,: settings (no Shift on Linux, like most apps).
      if (e.code === "Comma" && (isMac ? e.metaKey : e.ctrlKey) && !e.altKey && !e.shiftKey) {
        stop();
        openSettings();
        return;
      }
      if (!mod) return;
      if (k === "n" && (isMac ? e.shiftKey : true)) {
        stop();
        newSession();
        return;
      }
      if (k === "b") {
        stop();
        toggleSidebar();
        return;
      }
      if (!session) return;
      if (k === "t") {
        stop();
        requestKind(t("window.new"), null, (kind) => send({ t: "window_create", session: session.id, kind }));
      } else if (k === "d" && window) {
        stop();
        const down = isMac ? e.shiftKey : e.altKey;
        const panel = window.focus;
        requestKind(down ? t("panel.splitDown") : t("panel.splitRight"), null, (kind) => send({ t: "panel_split", panel, dir: down ? "column" : "row", kind }));
      } else if (k === "w" && window) {
        stop();
        const panel = window.focus;
        closeChecked(`panel:${panel}`, [panel], () => send({ t: "panel_close", panel }));
      } else if (/^Digit[1-9]$/.test(e.code)) {
        const w = session.windows[Number(e.code.slice(5)) - 1];
        if (w) {
          stop();
          send({ t: "window_select", session: session.id, window: w.id });
        }
      } else if (e.code === "BracketLeft" || e.code === "BracketRight") {
        stop();
        const i = session.windows.findIndex((w) => w.id === window?.id);
        const n = session.windows.length;
        const next = session.windows[(i + (e.code === "BracketRight" ? 1 : n - 1)) % n];
        if (next) send({ t: "window_select", session: session.id, window: next.id });
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [toggleSidebar]);
}

/** Sidebar width: dragged on its right edge, double-click there to reset, remembered. */
const useSidebarWidth = () => useDragWidth({ key: "recursion.sidebarWidth", initial: 220, min: 180, max: 480, edge: "right" });

export default function App() {
  const { state, terms, status, error } = useSnapshot();
  const [sidebar, setSidebar] = useState(true);
  const [pick, setPick] = useState<PickRequest | null>(null);
  useEffect(() => registerPicker(setPick), []);
  useEffect(restoreZoom, []);
  useEffect(() => void listenZoomKeys(zoomBrowser), []);
  useEffect(() => void listenForDrops(), []);
  // The webview's own menu (Back, Forward, Reload) means nothing here: keep it only where it
  // helps, on text being edited or selected (and in terminals, which have their own handling).
  useEffect(() => {
    const own = (e: MouseEvent) => {
      const el = e.target as HTMLElement;
      if (el.closest?.("input, textarea, [contenteditable=''], [contenteditable='true'], .xterm") || String(document.getSelection() ?? "")) return;
      e.preventDefault();
    };
    document.addEventListener("contextmenu", own);
    return () => document.removeEventListener("contextmenu", own);
  }, []);
  const toggleSidebar = useMemo(() => () => setSidebar((v) => !v), []);
  useShortcuts(toggleSidebar);
  const side = useSidebarWidth();
  const { session, window } = activeOf(state);

  useEffect(() => {
    if (!state) return;
    const alive = new Set(Object.values(state.panels).filter((p) => p.kind === "terminal").map((p) => p.id));
    pruneTerminals(alive);
    pruneBrowsers(new Set(Object.values(state.panels).filter((p) => p.kind === "browser" && !p.url?.startsWith("doc:") && viewerPath(p.url) === null).map((p) => p.id)));
  }, [state]);
  useEffect(() => (pick ? overlay() : undefined), [pick]);
  // A drawer's chat was closed or turned into something else: drop the zoom with it.
  const zoomed = useZoomed();
  useEffect(() => {
    if (zoomed && state && state.panels[zoomed]?.kind !== "workspace") closeThreads(zoomed);
  }, [state, zoomed]);

  return (
    <div
      className={`app${sidebar ? "" : " no-sidebar"}${side.dragging ? " side-resizing" : ""}`}
      style={{ "--side-w": `${side.width}px` } as React.CSSProperties}
    >
      <div className="corner" data-tauri-drag-region>
        <button className="icon-btn" title={`${t("side.title")}（${isMac ? "⌘B" : "Ctrl+Shift+B"}）`} onClick={toggleSidebar} aria-label={t("side.toggle")}>
          <SidebarIcon />
        </button>
        <button className="icon-btn" title={`${t("session.new")}（${isMac ? "⌘⇧N" : "Ctrl+Shift+N"}）`} onClick={newSession} aria-label={t("session.new")}>
          <Icon name="add" />
        </button>
      </div>
      <div className="side-slot" aria-hidden={!sidebar}>
        <Sidebar state={state} terms={terms} />
      </div>
      {sidebar && (
        <div className="side-resize" onPointerDown={side.onPointerDown} onDoubleClick={side.reset} title={t("side.resize")} role="separator" aria-orientation="vertical" />
      )}
      <main className="main">
        {session && window ? (
          <>
            <Tabs session={session} state={state!} terms={terms} />
            <LayoutView key={window.id} ctx={{ window, panels: state!.panels, terms, cwd: session.cwd, session: session.id, docs: session.docs ?? [] }} />
            <StatusBar session={session} status={status} />
          </>
        ) : session ? (
          <EmptyProject session={session} />
        ) : (
          <Empty status={status} error={error} hasState={!!state} />
        )}
      </main>
      {pick && <KindPicker req={pick} close={closePicker} />}
      <Toast />
      <TodoCardLayer />
      <UpdateNotice />
      <Onboarding />
      <SetupTerminal />
    </div>
  );
}

const COLLAPSED_KEY = "recursion.collapsedSessions";

function loadCollapsed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

function Sidebar({ state, terms }: { state: State | null; terms: Record<string, TermInfo> }) {
  // A project is a folder: one flat list, no groups.
  const groups: [string, Session[]][] = [["", state?.sessions ?? []]];
  const [editing, setEditing] = useState<string | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(loadCollapsed);
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      try {
        localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
      } catch {}
      return next;
    });
  const chats = useMemo(() => Object.fromEntries((state?.sessions ?? []).map((s) => [s.id, chatsOf(s, state!)])), [state]);
  const roster = useRoster(
    Object.values(chats)
      .flat()
      .map((c) => c.panel)
      .sort()
      .join(","),
  );

  const accounts = useAccounts();
  const loggedOut = accounts.filter((a) => !a.logged_in && !a.id.endsWith("-default")).length;
  return (
    <aside className="sidebar">
      <div className="side-top" data-tauri-drag-region />
      <nav className="sessions">
        {groups.map(([group, list]) => (
          <div key={group} className="group">
            {list.map((s) => {
              const on = s.id === state?.active_session;
              const edit = editing === s.id;
              return (
                <div key={s.id} className="sess-block">
                  <div className={`sess${on ? " on" : ""}`} onClick={() => (s.windows.length ? toggle(s.id) : send({ t: "session_select", session: s.id }))}>
                    {s.windows.length > 0 ? (
                      <button
                        className={`sess-fold${collapsed.has(s.id) ? " closed" : ""}`}
                        aria-label={collapsed.has(s.id) ? t("common.expand") : t("common.collapse")}
                        aria-expanded={!collapsed.has(s.id)}
                        onClick={(e) => (e.stopPropagation(), toggle(s.id))}
                      >
                        <Icon name="expand" size={12} />
                      </button>
                    ) : (
                      <span className="sess-fold-space" />
                    )}
                    <span className="sess-ic"><Icon name="folder" size={16} /></span>
                    {edit ? (
                      <input
                        autoFocus
                        className="sess-edit"
                        defaultValue={s.name}
                        onClick={(e) => e.stopPropagation()}
                        onBlur={(e) => {
                          const v = e.target.value.trim();
                          if (v) send({ t: "session_rename", session: s.id, name: v });
                          setEditing(null);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                          if (e.key === "Escape") setEditing(null);
                        }}
                      />
                    ) : (
                      <span className="sess-name" onDoubleClick={() => setEditing(s.id)} title={s.cwd}>
                        {s.name}
                      </span>
                    )}
                    <button
                      className="sess-more"
                      onClick={(e) => {
                        e.stopPropagation();
                        setMenu(menu === s.id ? null : s.id);
                      }}
                      aria-label={t("common.more")}
                    >
                      <Icon name="more" />
                    </button>
                    {menu === s.id && (
                      <div className="menu sess-menu" onClick={(e) => e.stopPropagation()} onMouseLeave={() => setMenu(null)}>
                        <button
                          onClick={(e) => {
                            const anchor = e.currentTarget.getBoundingClientRect();
                            setMenu(null);
                            requestKind(t("window.new"), anchor, (kind) => {
                              send({ t: "session_select", session: s.id });
                              send({ t: "window_create", session: s.id, kind });
                            });
                          }}
                        >
                          {t("window.new")}…
                        </button>
                        <button onClick={() => (setMenu(null), setEditing(s.id))}>{t("common.rename")}</button>
                        <button onClick={() => (setMenu(null), openSessionSettings(s.id))}>{t("scope.sessionMenu")}</button>
                        <button onClick={() => (setMenu(null), send({ t: "session_close", session: s.id }))} title={t("session.closeHint")}>
                          {t("session.close")}
                        </button>
                      </div>
                    )}
                  </div>
                  {state && !collapsed.has(s.id) && <SessionTree session={s} state={state} terms={terms} roster={roster} />}
                </div>
              );
            })}
          </div>
        ))}
      </nav>
      <div className="side-foot">
        <ClosedList closed={state?.closed ?? []} />
        <UsagePanel />
        <button className="side-accounts" onClick={() => openSettings()} title={`${t("settings.title")}（${isMac ? "⌘," : "Ctrl+,"}）`}>
          <Icon name="settings" size={14} /> {t("settings.title")}
          {loggedOut > 0 && <span className="side-warn" title={t("acct.loggedOutCount", { n: loggedOut })}>●</span>}
        </button>
      </div>
      <SettingsModal />
      <SessionSettingsModal />
    </aside>
  );
}

/** What was closed, to open again or delete for good. */
function ClosedList({ closed }: { closed: Closed[] }) {
  const [open, setOpen] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);
  if (!closed.length) return null;
  return (
    <div className="closed">
      <button className="side-accounts" onClick={() => (setOpen(!open), setConfirm(null))} aria-expanded={open}>
        <Icon name="recent" size={14} /> {t("closed.title")}
        <span className="dim">{closed.length}</span>
      </button>
      {open && (
        <div className="menu closed-menu" onMouseLeave={() => (setOpen(false), setConfirm(null))}>
          <div className="menu-label">{t("closed.hint")}</div>
          {[...closed].reverse().map((c) => {
            const name = c.kind === "session" ? c.session.name : c.panel.title || t("kind.workspace");
            const where = c.kind === "session" ? c.session.cwd.replace(/^\/(Users|home)\/[^/]+/, "~") : c.session_name;
            return (
              <div key={c.id} className="closed-row">
                <button className="closed-open" onClick={() => (setOpen(false), send({ t: "closed_reopen", id: c.id }))} title={t("closed.reopen")}>
                  <span className="closed-ic"><Icon name={c.kind === "session" ? "folder" : "chat"} size={14} /></span>
                  <span className="closed-main">
                    <b>{name}</b>
                    <span className="dim">
                      {where} · {ago(c.at)}
                    </span>
                  </span>
                </button>
                {confirm === c.id ? (
                  <button className="danger closed-del" onClick={() => (setConfirm(null), send({ t: "closed_delete", id: c.id }))}>
                    {t("closed.confirmDelete")}
                  </button>
                ) : (
                  <button className="closed-del" onClick={() => setConfirm(c.id)} title={t("closed.delete")} aria-label={t("closed.delete")}>
                    <Icon name="close" />
                  </button>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** The windows of the session on screen, each told by what it holds: its panels' icons and names
 * (as in the sidebar). Ctrl/⌘+1…9 still go by position. */
function Tabs({ session, state, terms }: { session: Session; state: State; terms: Record<string, TermInfo> }) {
  const drag = usePanelDrag();
  const over = drag?.target?.kind === "window" ? drag.target.window : null;
  const nth = panelNumbers(session, state.panels);
  // A right-click on a panel in a tab opens its menu; "rename" edits its name right there.
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  return (
    <div className="tabs" data-tauri-drag-region>
      {menu && state.panels[menu.id] && (
        <PanelContextMenu p={state.panels[menu.id]} at={menu} onRename={() => setRenaming(menu.id)} onClose={() => setMenu(null)} />
      )}
      {session.windows.map((w, i) => {
        const parts = leaves(w.root)
          .filter((id) => state.panels[id])
          .map((id) => ({ id, ...describe(state.panels[id], nth.get(id) ?? 1, session.docs ?? [], terms) }));
        return (
          <div
            key={w.id}
            data-window={w.id}
            className={`tab${w.id === session.active_window ? " on" : ""}${over === w.id ? " drop" : ""}`}
            onClick={() => send({ t: "window_select", session: session.id, window: w.id })}
            onPointerDown={(e) => w.id !== session.active_window && session.active_window && startTabDrag(e, parts.map((p) => p.id), session.active_window, parts.map((p) => p.label).join(" · "))}
            title={`${parts.map((p) => p.label).join(" · ")}（${isMac ? "⌘" : "Ctrl+"}${i + 1}）`}
          >
            {parts.map((p) => (
              <span key={p.id} className={`tab-part ic-${p.cls}`} onContextMenu={(e) => (e.preventDefault(), e.stopPropagation(), setMenu({ id: p.id, x: e.clientX, y: e.clientY }))}>
                <Icon name={p.icon} size={14} />
                {renaming === p.id ? (
                  <InlineEdit value={p.label} onSave={(name) => send({ t: "panel_rename", panel: p.id, name })} onDone={() => setRenaming(null)} />
                ) : (
                  <span className="tab-name">{p.label}</span>
                )}
              </span>
            ))}
            <button
              className="tab-x"
              title={t("window.close")}
              onClick={(e) => {
                e.stopPropagation();
                closeChecked(`window:${w.id}`, parts.map((p) => p.id), () => send({ t: "window_close", session: session.id, window: w.id }));
              }}
            >
              <Icon name="close" />
            </button>
          </div>
        );
      })}
      <button
        className="icon-btn"
        title={`${t("window.new")}（${isMac ? "⌘T" : "Ctrl+Shift+T"}）`}
        onClick={(e) => requestKind(t("window.new"), e.currentTarget.getBoundingClientRect(), (kind) => send({ t: "window_create", session: session.id, kind }))}
      >
        <Icon name="add" />
      </button>
      {drag && !drag.solo && <div className={`tab tab-ghost${drag.target?.kind === "new" ? " drop" : ""}`}><Icon name="add" size={14} /> {t("panel.ownWindow")}</div>}
      <div className="tabs-fill" data-tauri-drag-region />
    </div>
  );
}

function StatusBar({ session, status }: { session: Session; status: string }) {
  const wi = session.windows.findIndex((w) => w.id === session.active_window);
  const w = session.windows[wi];
  const ids = w ? leaves(w.root) : [];
  const pi = w ? ids.indexOf(w.focus) : -1;
  const m = isMac ? "⌘" : "Ctrl+Shift+";
  return (
    <footer className="status">
      <span>{session.cwd.replace(/^\/Users\/[^/]+/, "~")}</span>
      <span>
        window {wi + 1}/{session.windows.length} · panel {pi + 1}/{ids.length}
      </span>
      <span className="hint">
        {t("footer.keys", { m })}
      </span>
      {status !== "open" && <span className="warn">{t("conn.lost")}</span>}
    </footer>
  );
}

/** A project with nothing open in it (its last window was closed): open something here. */
function EmptyProject({ session }: { session: Session }) {
  const open = (kind: PanelKind) => send({ t: "window_create", session: session.id, kind });
  return (
    <div className="empty">
      <div className="tabs" data-tauri-drag-region>
        <div className="tabs-fill" data-tauri-drag-region />
      </div>
      <div className="empty-body">
        <div className="ws-glyph"><Icon name="terminal" size={40} /></div>
        <h1>{t("empty.projectTitle", { name: session.name })}</h1>
        <div className="empty-kinds">
          <button className="primary" onClick={() => open("workspace")}>{t("kind.workspace")}</button>
          <button onClick={() => open("terminal")}>{t("kind.terminal")}</button>
          <button onClick={() => open("browser")}>{t("kind.browser")}</button>
        </div>
      </div>
    </div>
  );
}

function Empty({ status, error, hasState }: { status: string; error: string | null; hasState: boolean }) {
  return (
    <div className="empty">
      <div className="tabs" data-tauri-drag-region>
        <div className="tabs-fill" data-tauri-drag-region />
      </div>
      <div className="empty-body">
        {!hasState ? (
          <>
            <div className="ws-glyph"><Icon name="terminal" size={40} /></div>
            <p>{status === "error" ? error : t("conn.connecting")}</p>
          </>
        ) : (
          <>
            <div className="ws-glyph"><Icon name="terminal" size={40} /></div>
            <h1>{t("empty.title")}</h1>
            <p className="dim">{t("empty.hint")}</p>
            <button className="primary" onClick={newSession}>
              {t("empty.open")}
            </button>
          </>
        )}
      </div>
    </div>
  );
}

function SidebarIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <rect x="1.5" y="2.5" width="13" height="11" rx="2.5" stroke="currentColor" strokeWidth="1.3" />
      <path d="M6 2.5v11" stroke="currentColor" strokeWidth="1.3" />
    </svg>
  );
}

function Toast() {
  const t = useToast();
  if (!t) return null;
  return (
    <div className={`toast${t.ok ? " ok" : ""}`} role="status" key={t.id}>
      {t.text}
    </div>
  );
}
