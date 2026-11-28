// Sidebar tree under each session: its windows, what each window holds, and the agents of each chat.
import { overlay, viewerPath } from "./browsers";
import { t } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { accountName, requestAddAgent, requestChatSettings, send, useAccounts } from "./client";
import { Avatar, isRunning, modelLabel, STATUS_LABEL } from "./WorkspaceView";
import { leaves, PERM_MODES, type Doc, type Member, type Panel, type PanelKind, type Session, type State, type TermInfo } from "./types";
import { Icon, type IconName } from "./icons";
import { closeChecked } from "./closing";

function k(n: number) {
  if (n >= 1e6) return `${+(n / 1e6).toFixed(n % 1e6 ? 1 : 0)}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}K`;
  return String(n);
}

/** What it may do without asking, as an icon (its name and meaning on hover). */
function Mode({ m }: { m: Member }) {
  const p = PERM_MODES.find((x) => x.id === (m.mode ?? "edits"));
  return p ? (
    <span className="tree-mode" title={`${p.label}：${p.hint}`}>
      <Icon name={p.id} size={12} />
    </span>
  ) : null;
}

function ContextBar({ m }: { m: Member }) {
  const used = m.context_used ?? 0;
  const max = m.context_window ?? 0;
  const pct = max ? Math.min(1, used / max) : 0;
  const level = pct > 0.9 ? "hi" : pct > 0.7 ? "mid" : "lo";
  return (
    <span className="ctx" title={max ? t("ctx.title", { used: used.toLocaleString(), max: max.toLocaleString(), pct: Math.round(pct * 100) }) : t("ctx.none")}>
      <span className="ctx-bar">
        <i className={`ctx-fill ${level}`} style={{ width: `${pct * 100}%` }} />
      </span>
      <span className="ctx-num">{max ? `${k(used)}/${k(max)}` : "—"}</span>
    </span>
  );
}

function AgentRow({ m, panel }: { m: Member; panel: string }) {
  const acct = accountName(useAccounts(), m);
  const [menu, setMenu] = useState(false);
  const [editing, setEditing] = useState(false);
  const busy = m.status !== "idle";
  const running = isRunning(m);
  return (
    <div className={`tree-agent st-${m.status}${busy ? " busy" : ""}`} onClick={() => send({ t: "panel_focus", panel })} title={STATUS_LABEL[m.status]}>
      <Avatar m={m} size={16} running={running} />
      <span className="tree-agent-main">
        <span className="tree-agent-name">
          {editing ? (
            <InlineEdit value={m.alias} prefix="@" onSave={(alias) => send({ t: "ws_rename_agent", panel, member: m.id, alias })} onDone={() => setEditing(false)} />
          ) : (
            <b onDoubleClick={(e) => (e.stopPropagation(), setEditing(true))} title={t("common.doubleClickRename")}>
              @{m.alias}
            </b>
          )}
          {/* Working: its mark moves and three dots light in turn; the line stays as when idle.
              Waiting for the human says so: nothing moves then. */}
          {!editing && running && (
            <span className="dots tree-dots" aria-hidden="true">
              <i>.</i>
              <i>.</i>
              <i>.</i>
            </span>
          )}
          {!editing &&
            (m.status === "waiting_permission" ? (
              <span className="tree-agent-waiting" title={m.waiting_for ?? undefined}>
                {STATUS_LABEL.waiting_permission}
              </span>
            ) : (
              <span className="tree-agent-model" title={acct ? t("agent.account", { acct }) : undefined}>
                {modelLabel(m)}
                {acct && ` · ${acct}`}
              </span>
            ))}
          {!editing && <Mode m={m} />}
        </span>
        <ContextBar m={m} />
      </span>
      {!editing && busy && (
        <button
          className="tree-stop"
          aria-label={t("agent.stop", { alias: m.alias })}
          title={t("agent.stopHint", { alias: m.alias })}
          onClick={(e) => {
            e.stopPropagation();
            send({ t: "ws_stop_agent", panel, member: m.id });
          }}
        >
          <Icon name="stop" />
        </button>
      )}
      {!editing && !busy && (
        <button
          className="sess-more"
          aria-label={t("agent.menu", { alias: m.alias })}
          onClick={(e) => {
            e.stopPropagation();
            setMenu((v) => !v);
          }}
        >
          <Icon name="more" />
        </button>
      )}
      {menu && (
        <div className="menu sess-menu" onClick={(e) => e.stopPropagation()} onMouseLeave={() => setMenu(false)}>
          <button onClick={() => (setMenu(false), setEditing(true))}>{t("common.rename")}</button>
          <button disabled={!m.cli_session_id} title={t("agent.compactHint")} onClick={() => (setMenu(false), send({ t: "ws_compact", panel, member: m.id }))}>
            {t("agent.compact")}
          </button>
          <div className="menu-sep" />
          <span className="menu-label">{t("perm.title")}</span>
          {PERM_MODES.map((p) => (
            <button key={p.id} className={`menu-check${(m.mode ?? "edits") === p.id ? " on" : ""}`} title={p.hint} onClick={() => (setMenu(false), send({ t: "ws_set_mode", panel, member: m.id, mode: p.id }))}>
              <Icon name={p.id} size={13} /> {p.label}
            </button>
          ))}
          <div className="menu-sep" />
          <button className="danger" onClick={() => (setMenu(false), send({ t: "ws_remove_agent", panel, member: m.id }))}>
            {t("agent.remove", { alias: m.alias })}
          </button>
        </div>
      )}
    </div>
  );
}

/** Text field that replaces a name in place: Enter or leaving it saves, Esc cancels. */
export function InlineEdit({ value, prefix, onSave, onDone }: { value: string; prefix?: string; onSave: (v: string) => void; onDone: () => void }) {
  // Opening it can switch windows, and the terminal or chat box there grabs the focus a moment
  // later; that blur must not count as "done", so take the focus back while the edit is fresh.
  const opened = useRef(performance.now());
  const closed = useRef(false);
  const finish = (v: string | null) => {
    if (closed.current) return;
    closed.current = true;
    if (v !== null && v.trim() && v.trim() !== value) onSave(v.trim());
    onDone();
  };
  return (
    <span className="tree-edit-wrap" onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      {prefix}
      <input
        autoFocus
        className="tree-edit"
        defaultValue={value}
        onFocus={(e) => e.currentTarget.select()}
        onBlur={(e) => {
          if (closed.current) return;
          if (performance.now() - opened.current < 600) {
            const el = e.currentTarget;
            setTimeout(() => el.isConnected && el.focus());
            return;
          }
          finish(e.currentTarget.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") finish(e.currentTarget.value);
          if (e.key === "Escape") finish(null);
        }}
      />
    </span>
  );
}

const ICON: Record<PanelKind, IconName> = { terminal: "terminal", workspace: "chat", browser: "browser" };

function host(url: string) {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** Unnamed panels are numbered by kind across their session: Chat 1, Chat 2, Terminal 1… */
export function panelNumbers(session: Session, panels: State["panels"]) {
  const nth = new Map<string, number>();
  const seen: Record<string, number> = {};
  for (const w of session.windows)
    for (const id of leaves(w.root)) {
      const p = panels[id];
      if (p) nth.set(id, (seen[p.kind] = (seen[p.kind] ?? 0) + 1));
    }
  return nth;
}

/** Icon, name and detail of one panel, the same in the tree, the tabs and the panel's header.
 * `n`: its number among the session's panels of its kind, for the default name ("Chat 2"). */
export function describe(p: Panel, n: number, docs: Doc[], terms: Record<string, TermInfo>) {
  const named = (kind: string) => p.title || `${t(kind)} ${n}`;
  if (p.kind === "terminal") return { icon: ICON.terminal, cls: "terminal", label: named("kind.terminal"), detail: terms[p.id]?.fg ?? "shell" };
  if (p.kind === "workspace") return { icon: ICON.workspace, cls: "workspace", label: named("kind.workspace"), detail: "" };
  if (p.url?.startsWith("doc:")) {
    const d = docs.find((x) => x.id === p.url!.slice(4));
    return { icon: "doc" as IconName, cls: "browser", label: named("kind.doc"), detail: d?.title ?? t("common.deleted") };
  }
  const file = viewerPath(p.url);
  if (file !== null) return { icon: "file" as IconName, cls: "browser", label: named("kind.file"), detail: file.slice(file.lastIndexOf("/") + 1) };
  return { icon: ICON.browser, cls: "browser", label: named("kind.browser"), detail: p.url ? host(p.url) : t("browser.newTab") };
}

/** A panel's menu: the same from its ⋯, from a right-click on its sidebar row, and on its tab. */
function PanelMenuItems({ p, close, onRename }: { p: Panel; close: () => void; onRename: () => void }) {
  return (
    <>
      {p.kind === "workspace" && <button onClick={() => (close(), requestAddAgent(p.id))}>{t("agent.addToChat")}</button>}
      {p.kind === "workspace" && <button onClick={() => (close(), requestChatSettings(p.id))}>{t("scope.chat")}…</button>}
      <button onClick={() => (close(), onRename())}>{t("common.rename")}</button>
      <button className="danger" onClick={() => closeChecked(`panel:${p.id}`, [p.id], () => (close(), send({ t: "panel_close", panel: p.id })))}>
        {t("common.close")}
      </button>
    </>
  );
}

/** That menu where the pointer was right-clicked (above native browser panels too). */
export function PanelContextMenu({ p, at, onRename, onClose }: { p: Panel; at: { x: number; y: number }; onRename: () => void; onClose: () => void }) {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const restore = overlay();
    const away = (e: Event) => {
      if (!(e.target as HTMLElement).closest?.(".ctx-menu")) close.current();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") (e.stopPropagation(), close.current());
    };
    window.addEventListener("mousedown", away, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", close.current);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", close.current);
      restore();
    };
  }, []);
  return createPortal(
    <div
      className="menu ctx-menu"
      role="menu"
      style={{ left: at.x, top: at.y }}
      ref={(el) => {
        if (!el) return;
        el.style.left = `${Math.max(8, Math.min(at.x, window.innerWidth - el.offsetWidth - 8))}px`;
        el.style.top = `${Math.max(8, Math.min(at.y, window.innerHeight - el.offsetHeight - 8))}px`;
      }}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      <PanelMenuItems p={p} close={onClose} onRename={onRename} />
    </div>,
    document.body,
  );
}

function PanelRow({
  p,
  n,
  docs,
  terms,
  members,
  folded,
  onFold,
}: {
  p: Panel;
  n: number;
  docs: Doc[];
  terms: Record<string, TermInfo>;
  members: Member[];
  folded: boolean;
  onFold: () => void;
}) {
  const d = describe(p, n, docs, terms);
  const [editing, setEditing] = useState(false);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  return (
    <>
      {at && <PanelContextMenu p={p} at={at} onRename={() => setEditing(true)} onClose={() => setAt(null)} />}
      <div
        className="tree-row"
        onClick={() => send({ t: "panel_focus", panel: p.id })}
        onContextMenu={(e) => (e.preventDefault(), setAt({ x: e.clientX, y: e.clientY }))}
      >
        {members.length > 0 ? (
          <button
            className={`sess-fold tree-fold${folded ? " closed" : ""}`}
            aria-label={folded ? t("tree.expandAgents") : t("tree.collapseAgents")}
            aria-expanded={!folded}
            onClick={(e) => (e.stopPropagation(), onFold())}
          >
            <Icon name="expand" size={12} />
          </button>
        ) : (
          <span className="sess-fold-space tree-fold" />
        )}
        <span className={`tree-ic ic-${d.cls}`}><Icon name={d.icon} size={16} /></span>
        {editing ? (
          <InlineEdit value={d.label} onSave={(name) => send({ t: "panel_rename", panel: p.id, name })} onDone={() => setEditing(false)} />
        ) : (
          <span className="tree-label" onDoubleClick={(e) => (e.stopPropagation(), setEditing(true))} title={`${d.label}${d.detail ? ` · ${d.detail}` : ""}（${t("common.doubleClickRename")}）`}>
            {d.label}
            {d.detail && <span className="tree-detail"> · {d.detail}</span>}
          </span>
        )}
        {!editing && (
          <RowMenu label={t("common.more")}>
            {(close) => <PanelMenuItems p={p} close={close} onRename={() => setEditing(true)} />}
          </RowMenu>
        )}
      </div>
      {members.length > 0 && !folded && (
        <div className="tree-agents">
          {members.map((m) => (
            <AgentRow key={m.id} m={m} panel={p.id} />
          ))}
        </div>
      )}
    </>
  );
}

/** One window (a tab): its panels, set apart from the next window by a line. The one on screen
 * (of the session on screen: every session has an active window) is lit as a whole. Windows have no names or menu here: their panels do. */
function WindowGroup({ on, children }: { on: boolean; children: React.ReactNode }) {
  return <div className={`tree-win${on ? " on" : ""}`}>{children}</div>;
}

/** "⋯" at the end of a row, opening the menu `children` draws (given a way to close it). */
function RowMenu({ label, children }: { label: string; children: (close: () => void) => React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="tree-menu-wrap">
      <button
        className="sess-more tree-more"
        aria-label={label}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        <Icon name="more" />
      </button>
      {open && (
        <div className="menu sess-menu" onClick={(e) => e.stopPropagation()} onMouseLeave={() => setOpen(false)}>
          {children(() => setOpen(false))}
        </div>
      )}
    </span>
  );
}

const FOLDED_KEY = "recursion.collapsedChats";

function loadFolded(key = FOLDED_KEY): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(key) ?? "[]"));
  } catch {
    return new Set();
  }
}

/** Folded ids, kept across restarts. */
function useFolded(key: string) {
  const [folded, setFolded] = useState<Set<string>>(() => loadFolded(key));
  const fold = (id: string) =>
    setFolded((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      try {
        localStorage.setItem(key, JSON.stringify([...next]));
      } catch {}
      return next;
    });
  return [folded, fold] as const;
}

/** Session → windows → panels (with their kind) → agents of each chat. */
export function SessionTree({ session, state, terms, roster }: { session: Session; state: State; terms: Record<string, TermInfo>; roster: Record<string, Member[]> }) {
  const docs = session.docs ?? [];
  // Chats whose agent list is folded away, kept across restarts like folded sessions.
  const [folded, fold] = useFolded(FOLDED_KEY);
  const nth = panelNumbers(session, state.panels);
  return (
    <div className="tree">
      {session.windows.map((w) => (
        <WindowGroup key={w.id} on={session.id === state.active_session && w.id === session.active_window}>
          {leaves(w.root)
            .filter((id) => state.panels[id])
            .map((id) => (
              <PanelRow key={id} p={state.panels[id]} n={nth.get(id) ?? 1} docs={docs} terms={terms} members={roster[id] ?? []} folded={folded.has(id)} onFold={() => fold(id)} />
            ))}
        </WindowGroup>
      ))}
    </div>
  );
}
