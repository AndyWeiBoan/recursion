import { useEffect, useMemo, useState } from "react";
import { getSnapshot, isMac, pickFolder, send, useRoster, useSnapshot, useToast } from "./client";
import { KindPicker, type PickRequest } from "./KindPicker";
import { LayoutView } from "./Layout";
import { closePicker, pickerOpen, registerPicker, requestKind } from "./picker";
import { pruneTerminals } from "./terminals";
import { overlay, pruneBrowsers } from "./browsers";
import { SessionTree } from "./SessionTree";
import { closeThreads, useZoomed } from "./threadsUI";
import { chatsOf, leaves, type Session, type State, type TermInfo } from "./types";

const VIM_LIKE = new Set(["nvim", "vim", "vi", "view", "nvim-qt"]);

function activeOf(state: State | null) {
  const session = state?.sessions.find((s) => s.id === state.active_session) ?? state?.sessions[0];
  const window = session?.windows.find((w) => w.id === session.active_window) ?? session?.windows[0];
  return { session, window };
}

async function newSession() {
  const cwd = await pickFolder();
  if (!cwd) return;
  const name = cwd.replace(/\/+$/, "").split("/").pop() || cwd;
  requestKind(`在「${name}」開啟`, null, (kind) => send({ t: "session_create", name, cwd, kind }));
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
        requestKind("新 window", null, (kind) => send({ t: "window_create", session: session.id, kind }));
      } else if (k === "d" && window) {
        stop();
        const down = isMac ? e.shiftKey : e.altKey;
        const panel = window.focus;
        requestKind(down ? "上下分割" : "左右分割", null, (kind) => send({ t: "panel_split", panel, dir: down ? "column" : "row", kind }));
      } else if (k === "w" && window) {
        stop();
        send({ t: "panel_close", panel: window.focus });
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

export default function App() {
  const { state, terms, status, error } = useSnapshot();
  const [sidebar, setSidebar] = useState(true);
  const [pick, setPick] = useState<PickRequest | null>(null);
  useEffect(() => registerPicker(setPick), []);
  const toggleSidebar = useMemo(() => () => setSidebar((v) => !v), []);
  useShortcuts(toggleSidebar);
  const { session, window } = activeOf(state);

  useEffect(() => {
    if (!state) return;
    const alive = new Set(Object.values(state.panels).filter((p) => p.kind === "terminal").map((p) => p.id));
    pruneTerminals(alive);
    pruneBrowsers(new Set(Object.values(state.panels).filter((p) => p.kind === "browser" && !p.url?.startsWith("doc:")).map((p) => p.id)));
  }, [state]);
  useEffect(() => (pick ? overlay() : undefined), [pick]);
  // A drawer's chat was closed or turned into something else: drop the zoom with it.
  const zoomed = useZoomed();
  useEffect(() => {
    if (zoomed && state && state.panels[zoomed]?.kind !== "workspace") closeThreads(zoomed);
  }, [state, zoomed]);

  return (
    <div className={`app${sidebar ? "" : " no-sidebar"}`}>
      <div className="corner" data-tauri-drag-region>
        <button className="icon-btn" title={`側欄（${isMac ? "⌘B" : "Ctrl+Shift+B"}）`} onClick={toggleSidebar} aria-label="顯示或隱藏側欄">
          <SidebarIcon />
        </button>
        <button className="icon-btn" title={`新 session（${isMac ? "⌘⇧N" : "Ctrl+Shift+N"}）`} onClick={newSession} aria-label="新 session">
          ＋
        </button>
      </div>
      <div className="side-slot" aria-hidden={!sidebar}>
        <Sidebar state={state} terms={terms} />
      </div>
      <main className="main">
        {session && window ? (
          <>
            <Tabs session={session} />
            <LayoutView key={window.id} ctx={{ window, panels: state!.panels, terms, cwd: session.cwd, session: session.id, docs: session.docs ?? [] }} />
            <StatusBar session={session} status={status} />
          </>
        ) : (
          <Empty status={status} error={error} hasState={!!state} />
        )}
      </main>
      {pick && <KindPicker req={pick} close={closePicker} />}
      <Toast />
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
  const groups = useMemo(() => {
    const m = new Map<string, Session[]>();
    state?.sessions.forEach((s) => m.set(s.group, [...(m.get(s.group) ?? []), s]));
    return [...m.entries()];
  }, [state]);
  const [editing, setEditing] = useState<{ id: string; field: "name" | "group" } | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
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

  return (
    <aside className="sidebar">
      <div className="side-top" data-tauri-drag-region />
      <nav className="sessions">
        {groups.map(([group, list]) => (
          <div key={group} className="group">
            <div className="group-name">{group}</div>
            {list.map((s) => {
              const on = s.id === state?.active_session;
              const edit = editing?.id === s.id ? editing.field : null;
              return (
                <div key={s.id} className="sess-block">
                  <div className={`sess${on ? " on" : ""}`} onClick={() => send({ t: "session_select", session: s.id })} title={s.cwd}>
                    {s.windows.length > 0 ? (
                      <button
                        className={`sess-fold${collapsed.has(s.id) ? " closed" : ""}`}
                        aria-label={collapsed.has(s.id) ? "展開" : "收合"}
                        aria-expanded={!collapsed.has(s.id)}
                        onClick={(e) => (e.stopPropagation(), toggle(s.id))}
                      >
                        ›
                      </button>
                    ) : (
                      <span className="sess-fold-space" />
                    )}
                    <span className="sess-ic">✳</span>
                    {edit ? (
                      <input
                        autoFocus
                        className="sess-edit"
                        defaultValue={edit === "name" ? s.name : s.group}
                        onClick={(e) => e.stopPropagation()}
                        onBlur={(e) => {
                          const v = e.target.value.trim();
                          if (v) send(edit === "name" ? { t: "session_rename", session: s.id, name: v } : { t: "session_group", session: s.id, group: v });
                          setEditing(null);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                          if (e.key === "Escape") setEditing(null);
                        }}
                      />
                    ) : (
                      <span className="sess-name" onDoubleClick={() => setEditing({ id: s.id, field: "name" })}>
                        {s.name}
                      </span>
                    )}
                    <button
                      className="sess-more"
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirmDelete(null);
                          setMenu(menu === s.id ? null : s.id);
                      }}
                      aria-label="更多"
                    >
                      ⋯
                    </button>
                    {menu === s.id && (
                      <div className="menu sess-menu" onClick={(e) => e.stopPropagation()} onMouseLeave={() => setMenu(null)}>
                        <button
                          onClick={(e) => {
                            const anchor = e.currentTarget.getBoundingClientRect();
                            setMenu(null);
                            requestKind("新 window", anchor, (kind) => {
                              send({ t: "session_select", session: s.id });
                              send({ t: "window_create", session: s.id, kind });
                            });
                          }}
                        >
                          新 window…
                        </button>
                        <button onClick={() => (setMenu(null), setEditing({ id: s.id, field: "name" }))}>重新命名</button>
                        <button onClick={() => (setMenu(null), setEditing({ id: s.id, field: "group" }))}>移到其他分組</button>
                        {confirmDelete === s.id ? (
                          <button className="danger" onClick={() => (setMenu(null), send({ t: "session_delete", session: s.id }))}>
                            確定刪除？終端會被關閉
                          </button>
                        ) : (
                          <button className="danger" onClick={() => setConfirmDelete(s.id)}>
                            刪除
                          </button>
                        )}
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
    </aside>
  );
}

function Tabs({ session }: { session: Session }) {
  const [editing, setEditing] = useState<string | null>(null);
  return (
    <div className="tabs" data-tauri-drag-region>
      {session.windows.map((w, i) => (
        <div
          key={w.id}
          className={`tab${w.id === session.active_window ? " on" : ""}`}
          onClick={() => send({ t: "window_select", session: session.id, window: w.id })}
          onDoubleClick={() => setEditing(w.id)}
          title="雙擊重新命名"
        >
          <span className="tab-n">{i + 1}</span>
          {editing === w.id ? (
            <input
              autoFocus
              className="tab-edit"
              defaultValue={w.name}
              onBlur={(e) => {
                const v = e.target.value.trim();
                if (v) send({ t: "window_rename", session: session.id, window: w.id, name: v });
                setEditing(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                if (e.key === "Escape") setEditing(null);
              }}
            />
          ) : (
            w.name && w.name !== String(i + 1) && <span className="tab-name">{w.name}</span>
          )}
          <button
            className="tab-x"
            title="關閉 window"
            onClick={(e) => {
              e.stopPropagation();
              send({ t: "window_close", session: session.id, window: w.id });
            }}
          >
            ✕
          </button>
        </div>
      ))}
      <button
        className="icon-btn"
        title={`新 window（${isMac ? "⌘T" : "Ctrl+Shift+T"}）`}
        onClick={(e) => requestKind("新 window", e.currentTarget.getBoundingClientRect(), (kind) => send({ t: "window_create", session: session.id, kind }))}
      >
        ＋
      </button>
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
        {m}D 分割 · {m}T 新 window · {m}W 關閉 · Ctrl+HJKL 移動
      </span>
      {status !== "open" && <span className="warn">與 recursiond 的連線中斷，重新連線中…</span>}
    </footer>
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
            <div className="ws-glyph">❯_</div>
            <p>{status === "error" ? error : "正在連線到 recursiond…"}</p>
          </>
        ) : (
          <>
            <div className="ws-glyph">❯_</div>
            <h1>選一個專案資料夾開始</h1>
            <p className="dim">每個 session 對應一個專案；關掉視窗後，終端會在背景繼續跑。</p>
            <button className="primary" onClick={newSession}>
              開啟專案資料夾
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
    <div className="toast" role="status" key={t.id}>
      {t.text}
    </div>
  );
}
