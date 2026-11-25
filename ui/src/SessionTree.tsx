// Sidebar tree under each session: its windows, what each window holds, and the agents of each chat.
import { viewerPath } from "./browsers";
import { t } from "./i18n";
import { useRef, useState } from "react";
import { accountName, requestAddAgent, send, useAccounts } from "./client";
import { Avatar, STATUS_LABEL } from "./WorkspaceView";
import { leaves, PERM_MODES, type Doc, type Member, type Panel, type PanelKind, type Session, type State, type TermInfo } from "./types";

function k(n: number) {
  if (n >= 1e6) return `${+(n / 1e6).toFixed(n % 1e6 ? 1 : 0)}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}K`;
  return String(n);
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
  return (
    <div className={`tree-agent st-${m.status}${busy ? " busy" : ""}`} onClick={() => send({ t: "panel_focus", panel })} title={STATUS_LABEL[m.status]}>
      <Avatar m={m} size={16} />
      <span className="tree-agent-main">
        <span className="tree-agent-name">
          {editing ? (
            <InlineEdit value={m.alias} prefix="@" onSave={(alias) => send({ t: "ws_rename_agent", panel, member: m.id, alias })} onDone={() => setEditing(false)} />
          ) : (
            <b onDoubleClick={(e) => (e.stopPropagation(), setEditing(true))} title={t("common.doubleClickRename")}>
              @{m.alias}
            </b>
          )}
          {!editing &&
            (busy ? (
              <span className="tree-agent-model tree-agent-busy">
                {STATUS_LABEL[m.status]}
                <span className="dots" aria-hidden="true">
                  <i>.</i>
                  <i>.</i>
                  <i>.</i>
                </span>
              </span>
            ) : (
              <span className="tree-agent-model" title={acct ? t("agent.account", { acct }) : undefined}>
                {m.model ?? m.kind}
                {acct && ` · ${acct}`}
                {m.mode && m.mode !== "edits" && <span className={`tree-mode m-${m.mode}`}> · {PERM_MODES.find((p) => p.id === m.mode)?.label}</span>}
              </span>
            ))}
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
          ■
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
          ⋯
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
              {p.label}
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
function InlineEdit({ value, prefix, onSave, onDone }: { value: string; prefix?: string; onSave: (v: string) => void; onDone: () => void }) {
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

const ICON: Record<PanelKind, string> = { terminal: "›_", workspace: "✳", browser: "◎" };

function host(url: string) {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** Icon, label and detail for one panel, as the tree shows it. */
function describe(p: Panel, docs: Doc[], terms: Record<string, TermInfo>) {
  if (p.kind === "terminal") return { icon: ICON.terminal, cls: "terminal", label: t("kind.terminal"), detail: terms[p.id]?.fg ?? "shell" };
  if (p.kind === "workspace") return { icon: ICON.workspace, cls: "workspace", label: p.title || t("kind.workspace"), detail: "" };
  if (p.url?.startsWith("doc:")) {
    const d = docs.find((x) => x.id === p.url!.slice(4));
    return { icon: "▤", cls: "browser", label: t("kind.doc"), detail: d?.title ?? t("common.deleted") };
  }
  const file = viewerPath(p.url);
  if (file !== null) return { icon: "▤", cls: "browser", label: t("kind.file"), detail: file.slice(file.lastIndexOf("/") + 1) };
  return { icon: ICON.browser, cls: "browser", label: t("kind.browser"), detail: p.url ? host(p.url) : t("browser.newTab") };
}

function PanelRow({
  p,
  docs,
  terms,
  members,
  folded,
  onFold,
}: {
  p: Panel;
  docs: Doc[];
  terms: Record<string, TermInfo>;
  members: Member[];
  folded: boolean;
  onFold: () => void;
}) {
  const d = describe(p, docs, terms);
  return (
    <>
      <div className="tree-row" onClick={() => send({ t: "panel_focus", panel: p.id })} title={`${d.label}${d.detail ? ` · ${d.detail}` : ""}`}>
        {members.length > 0 ? (
          <button
            className={`sess-fold tree-fold${folded ? " closed" : ""}`}
            aria-label={folded ? t("tree.expandAgents") : t("tree.collapseAgents")}
            aria-expanded={!folded}
            onClick={(e) => (e.stopPropagation(), onFold())}
          >
            ›
          </button>
        ) : (
          <span className="sess-fold-space tree-fold" />
        )}
        <span className={`tree-ic ic-${d.cls}`}>{d.icon}</span>
        <span className="tree-label">
          {d.label}
          {d.detail && <span className="tree-detail"> · {d.detail}</span>}
        </span>
        {p.kind === "workspace" && (
          <button className="tree-add" title={t("agent.add")} aria-label={t("agent.addToChat")} onClick={(e) => (e.stopPropagation(), requestAddAgent(p.id))}>
            ＋
          </button>
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

/** One window (a tab): its panels side by side in the tree, its name above them if it has one,
 * and "⋯" to rename or close it. The one on screen is lit as a whole. */
function WindowGroup({ session, w, index, on, children }: { session: string; w: Session["windows"][number]; index: number; on: boolean; children: React.ReactNode }) {
  const [editing, setEditing] = useState(false);
  const name = w.name && w.name !== String(index + 1) ? w.name : "";
  const rename = (v: string) => send({ t: "window_rename", session, window: w.id, name: v });
  return (
    <div className={`tree-win${on ? " on" : ""}`}>
      {editing ? (
        <div className="tree-win-name">
          <InlineEdit value={name} onSave={rename} onDone={() => setEditing(false)} />
        </div>
      ) : (
        name && (
          <div className="tree-win-name" onDoubleClick={() => setEditing(true)} title={t("window.doubleClickRename")}>
            {name}
          </div>
        )
      )}
      {children}
      {!editing && <WindowMenu onRename={() => setEditing(true)} onClose={() => send({ t: "window_close", session, window: w.id })} />}
    </div>
  );
}

/** "⋯" on a window row: rename or close it. */
function WindowMenu({ onRename, onClose }: { onRename: () => void; onClose: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        className="sess-more tree-more"
        aria-label={t("window.menu")}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        ⋯
      </button>
      {open && (
        <div className="menu sess-menu" onClick={(e) => e.stopPropagation()} onMouseLeave={() => setOpen(false)}>
          <button onClick={() => (setOpen(false), onRename())}>{t("common.rename")}</button>
          <button className="danger" onClick={() => (setOpen(false), onClose())}>
            {t("window.close")}
          </button>
        </div>
      )}
    </>
  );
}

const FOLDED_KEY = "recursion.collapsedChats";

function loadFolded(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(FOLDED_KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}

/** Session → windows → panels (with their kind) → agents of each chat. */
export function SessionTree({ session, state, terms, roster }: { session: Session; state: State; terms: Record<string, TermInfo>; roster: Record<string, Member[]> }) {
  const docs = session.docs ?? [];
  // Chats whose agent list is folded away, kept across restarts like folded sessions.
  const [folded, setFolded] = useState<Set<string>>(loadFolded);
  const fold = (id: string) =>
    setFolded((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      try {
        localStorage.setItem(FOLDED_KEY, JSON.stringify([...next]));
      } catch {}
      return next;
    });
  return (
    <div className="tree">
      {session.windows.map((w, i) => (
        <WindowGroup key={w.id} session={session.id} w={w} index={i} on={w.id === session.active_window}>
          {leaves(w.root)
            .filter((id) => state.panels[id])
            .map((id) => (
              <PanelRow key={id} p={state.panels[id]} docs={docs} terms={terms} members={roster[id] ?? []} folded={folded.has(id)} onFold={() => fold(id)} />
            ))}
        </WindowGroup>
      ))}
    </div>
  );
}
