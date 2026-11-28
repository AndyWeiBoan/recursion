// Sidebar tree under each session: its windows, what each window holds, and the agents of each chat.
import { overlay, viewerPath } from "./browsers";
import { t } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { accountName, requestAddAgent, requestChatSettings, send, useAccounts, useModels, useRoster } from "./client";
import { Avatar, isRunning, modelLabel, STATUS_LABEL } from "./WorkspaceView";
import { leaves, PERM_MODES, type Doc, type Member, type Panel, type PanelKind, type Session, type State, type TermInfo } from "./types";
import { Icon, type IconName } from "./icons";
import { openAgentInfo } from "./AgentInfo";
import { effortsFor, openAgentSettings, saveAgent } from "./AgentSettings";
import { Collapse } from "./Collapse";
import { startSort } from "./sortDrag";
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
  const more = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState(false);
  const busy = m.status !== "idle";
  const running = isRunning(m);
  return (
    <div
      className={`tree-agent st-${m.status}${busy ? " busy" : ""}`}
      onClick={() => send({ t: "panel_focus", panel })}
      // Up or down among its chat's agents.
      onPointerDown={(e) => {
        const item = e.currentTarget;
        startSort(e, { item, axis: "y", items: () => [...item.parentElement!.querySelectorAll<HTMLElement>(":scope > .tree-agent")], onDrop: (to) => send({ t: "ws_move_agent", panel, member: m.id, to }) });
      }}
      title={STATUS_LABEL[m.status]}
    >
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
          ref={more}
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
        <FloatMenu anchor={more.current} onClose={() => setMenu(false)}>
          <button onClick={() => (setMenu(false), openAgentInfo({ panel, member: m.id }))}>{t("info.open")}</button>
          <button onClick={() => (setMenu(false), openAgentSettings({ panel, member: m.id }))}>{t("agent.settings")}</button>
          <button onClick={() => (setMenu(false), setEditing(true))}>{t("common.rename")}</button>
          <button disabled={!m.cli_session_id} title={t("agent.compactHint")} onClick={() => (setMenu(false), send({ t: "ws_compact", panel, member: m.id }))}>
            {t("agent.compact")}
          </button>
          <div className="menu-sep" />
          <EffortMenu panel={panel} m={m} done={() => setMenu(false)} />
          <SubMenu label={t("perm.title")}>
            {PERM_MODES.map((p) => (
              <button key={p.id} className={`menu-check${(m.mode ?? "edits") === p.id ? " on" : ""}`} title={p.hint} onClick={() => (setMenu(false), saveAgent(panel, m, { mode: p.id }))}>
                <Icon name={p.id} size={13} /> {p.label}
              </button>
            ))}
          </SubMenu>
          <div className="menu-sep" />
          <button className="danger" onClick={() => (setMenu(false), send({ t: "ws_remove_agent", panel, member: m.id }))}>
            {t("agent.remove", { alias: m.alias })}
          </button>
        </FloatMenu>
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

/** A ⋯ button's menu, drawn above everything at the button (not inside the scrolling sidebar,
 * which would cut it off near its bottom) and opening upward when there is no room below. */
export function FloatMenu({ anchor, onClose, children }: { anchor: HTMLElement | null; onClose: () => void; children: React.ReactNode }) {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const restore = overlay();
    const away = (e: Event) => {
      const el = e.target as HTMLElement;
      if (!el.closest?.(".float-menu") && !anchor?.contains(el)) close.current();
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
  }, [anchor]);
  if (!anchor) return null;
  const r = anchor.getBoundingClientRect();
  return createPortal(
    <div
      className="menu float-menu"
      style={{ top: r.bottom + 4, right: window.innerWidth - r.right }}
      ref={(el) => {
        if (el && r.bottom + 4 + el.offsetHeight > window.innerHeight - 8) el.style.top = `${Math.max(8, r.top - 4 - el.offsetHeight)}px`;
      }}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
      onMouseLeave={() => close.current()}
    >
      {children}
    </div>,
    document.body,
  );
}

/** A panel's menu: the same from its ⋯, from a right-click on its sidebar row, and on its tab. */
function PanelMenuItems({ p, close, onRename }: { p: Panel; close: () => void; onRename: () => void }) {
  const agents = useRoster(p.id)[p.id]?.length ?? 0;
  return (
    <>
      {p.kind === "workspace" && <button onClick={() => (close(), requestAddAgent(p.id))}>{t("agent.addToChat")}</button>}
      {p.kind === "workspace" && <button onClick={() => (close(), requestChatSettings(p.id))}>{t("scope.chat")}…</button>}
      <button onClick={() => (close(), onRename())}>{t("common.rename")}</button>
      {/* They can come back from the recently closed list. */}
      {p.kind === "workspace" && (
        <button className="danger" disabled={!agents} onClick={() => (close(), send({ t: "ws_remove_all_agents", panel: p.id }))}>
          {t("chat.removeAll")}
        </button>
      )}
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
  shown,
  docs,
  terms,
  members,
  folded,
  onFold,
  onDrag,
}: {
  p: Panel;
  n: number;
  /** It is the panel on screen. */
  shown: boolean;
  docs: Doc[];
  terms: Record<string, TermInfo>;
  members: Member[];
  folded: boolean;
  onFold: () => void;
  /** Pressed on its row: maybe the start of a drag (see SessionTree). */
  onDrag: (e: React.PointerEvent, item: HTMLElement) => void;
}) {
  const d = describe(p, n, docs, terms);
  const [editing, setEditing] = useState(false);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  return (
    <div className="tree-item">
      {at && <PanelContextMenu p={p} at={at} onRename={() => setEditing(true)} onClose={() => setAt(null)} />}
      <div
        className="tree-row"
        onPointerDown={(e) => !editing && onDrag(e, e.currentTarget.parentElement!)}
        // A chat with agents: the first click shows it (unfolding it), the next ones fold it.
        onClick={() => {
          if (shown && members.length) return onFold();
          send({ t: "panel_focus", panel: p.id });
          if (folded) onFold();
        }}
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
      {members.length > 0 && (
        <Collapse open={!folded}>
          <div className="tree-agents">
            {members.map((m) => (
              <AgentRow key={m.id} m={m} panel={p.id} />
            ))}
          </div>
        </Collapse>
      )}
    </div>
  );
}

/** A menu item that opens its own menu beside it, on hover or click (inside the menu, so moving
 *  into it doesn't close the menu). It opens upward when there is no room below. */
function SubMenu({ label, children }: { label: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="submenu-wrap" onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
      <button className={`submenu-btn${open ? " on" : ""}`} onClick={() => setOpen(true)} aria-expanded={open}>
        {label}
        <Icon name="expand" size={13} />
      </button>
      {open && (
        <div
          className="menu submenu"
          ref={(el) => {
            if (el && el.getBoundingClientRect().bottom > window.innerHeight - 8) el.classList.add("up");
          }}
        >
          {children}
        </div>
      )}
    </div>
  );
}

/** An agent's effort, from its menu: the levels its model takes, its default one marked. */
function EffortMenu({ panel, m, done }: { panel: string; m: Member; done: () => void }) {
  const list = useModels(m.kind);
  const { efforts, def } = effortsFor(m, list.models, m.model ?? "");
  if (!list.loading && !efforts.length) return null;
  const shown = m.effort || def;
  return (
    <SubMenu label={t("agent.effort")}>
      {list.loading && <span className="menu-label">{t("agent.modelsLoading", { kind: m.kind === "claude" ? "Claude" : "Codex" })}</span>}
      {efforts.map((e) => (
        // The default level picks "default": it follows the CLI if that changes.
        <button key={e} className={`menu-check effort-item${shown === e ? " on" : ""}`} onClick={() => (done(), saveAgent(panel, m, { effort: e === def ? null : e }))}>
          {e}
          {e === def && <span className="effort-def">{t("agent.defaultEffort")}</span>}
        </button>
      ))}
    </SubMenu>
  );
}

/** One window (a tab): its panels, set apart from the next window by a line. The one on screen
 * (of the session on screen: every session has an active window) is lit as a whole. Windows have no names or menu here: their panels do. */
function WindowGroup({ on, grip, children }: { on: boolean; grip?: (e: React.PointerEvent, item: HTMLElement) => void; children: React.ReactNode }) {
  return (
    <div className={`tree-win${on ? " on" : ""}`}>
      {/* A window of several panels is dragged by this (its panels' rows reorder them). */}
      {grip && (
        <span className="tree-win-grip" onPointerDown={(e) => grip(e, e.currentTarget.parentElement!)} title={t("tree.dragWindow")} aria-hidden>
          <Icon name="grip" size={12} />
        </span>
      )}
      {children}
    </div>
  );
}

/** "⋯" at the end of a row, opening the menu `children` draws (given a way to close it). */
function RowMenu({ label, children }: { label: string; children: (close: () => void) => React.ReactNode }) {
  const [open, setOpen] = useState(false);
  const more = useRef<HTMLButtonElement>(null);
  return (
    <span className="tree-menu-wrap">
      <button
        ref={more}
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
        <FloatMenu anchor={more.current} onClose={() => setOpen(false)}>
          {children(() => setOpen(false))}
        </FloatMenu>
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
  // Windows go up and down among the project's; a window's panels only among themselves.
  const dragWindow = (w: string) => (e: React.PointerEvent, item: HTMLElement) =>
    startSort(e, { item, axis: "y", items: () => [...item.parentElement!.querySelectorAll<HTMLElement>(":scope > .tree-win")], onDrop: (to) => send({ t: "window_move", session: session.id, window: w, to }) });
  return (
    <div className="tree">
      {session.windows.map((w) => {
        const ids = leaves(w.root);
        const solo = ids.length === 1;
        const dragPanel = (id: string) => (e: React.PointerEvent, item: HTMLElement) => {
          if (solo) return dragWindow(w.id)(e, item.closest<HTMLElement>(".tree-win")!);
          if (ids.some((x) => !state.panels[x])) return;
          startSort(e, {
            item,
            axis: "y",
            items: () => [...item.parentElement!.querySelectorAll<HTMLElement>(":scope > .tree-item")],
            onDrop: (to) => {
              const order = ids.filter((x) => x !== id);
              order.splice(to, 0, id);
              send({ t: "panel_order", window: w.id, panels: order });
            },
          });
        };
        return (
          <WindowGroup key={w.id} on={session.id === state.active_session && w.id === session.active_window} grip={solo ? undefined : dragWindow(w.id)}>
            {ids
              .filter((id) => state.panels[id])
              .map((id) => (
                <PanelRow key={id} p={state.panels[id]} n={nth.get(id) ?? 1} shown={session.id === state.active_session && w.id === session.active_window && w.focus === id} docs={docs} terms={terms} members={roster[id] ?? []} folded={folded.has(id)} onFold={() => fold(id)} onDrag={dragPanel(id)} />
              ))}
          </WindowGroup>
        );
      })}
    </div>
  );
}
