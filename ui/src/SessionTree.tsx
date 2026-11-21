// Sidebar tree under each session: its windows, what each window holds, and the agents of each chat.
import { useRef, useState } from "react";
import { accountName, requestAddAgent, send, useAccounts } from "./client";
import { Avatar, STATUS_LABEL } from "./WorkspaceView";
import { leaves, type Doc, type Member, type Panel, type PanelKind, type Session, type State, type TermInfo } from "./types";

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
    <span className="ctx" title={max ? `context：${used.toLocaleString()} / ${max.toLocaleString()} tokens（${Math.round(pct * 100)}%）` : "還沒有 context 資料（agent 回覆一次後就有）"}>
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
            <b onDoubleClick={(e) => (e.stopPropagation(), setEditing(true))} title="雙擊重新命名">
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
              <span className="tree-agent-model" title={acct ? `帳號：${acct}` : undefined}>
                {m.model ?? m.kind}
                {acct && ` · ${acct}`}
              </span>
            ))}
        </span>
        <ContextBar m={m} />
      </span>
      {!editing && busy && (
        <button
          className="tree-stop"
          aria-label={`停止 @${m.alias}`}
          title={`停止 @${m.alias}（其他 agent 繼續）`}
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
          aria-label={`@${m.alias} 的選單`}
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
          <button onClick={() => (setMenu(false), setEditing(true))}>重新命名</button>
          <button className="danger" onClick={() => (setMenu(false), send({ t: "ws_remove_agent", panel, member: m.id }))}>
            移除 @{m.alias}
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
  if (p.kind === "terminal") return { icon: ICON.terminal, cls: "terminal", label: "終端", detail: terms[p.id]?.fg ?? "shell" };
  if (p.kind === "workspace") return { icon: ICON.workspace, cls: "workspace", label: "群聊", detail: "" };
  if (p.url?.startsWith("doc:")) {
    const d = docs.find((x) => x.id === p.url!.slice(4));
    return { icon: "▤", cls: "browser", label: "文件", detail: d?.title ?? "已刪除" };
  }
  return { icon: ICON.browser, cls: "browser", label: "瀏覽器", detail: p.url ? host(p.url) : "新分頁" };
}

function PanelRow({
  p,
  docs,
  terms,
  members,
  onClick,
  prefix,
  name,
  rename,
  closeWindow,
  folded,
  onFold,
}: {
  p: Panel;
  docs: Doc[];
  terms: Record<string, TermInfo>;
  members: Member[];
  onClick: () => void;
  prefix?: React.ReactNode;
  /** The window's own name, shown instead of the kind when set. */
  name?: string;
  /** Present when this row stands for a whole window, which can then be renamed or closed. */
  rename?: (v: string) => void;
  closeWindow?: () => void;
  folded: boolean;
  onFold: () => void;
}) {
  const d = describe(p, docs, terms);
  const [editing, setEditing] = useState(false);
  return (
    <>
      <div
        className="tree-row tree-row-menu"
        onClick={onClick}
        onDoubleClick={rename && (() => setEditing(true))}
        title={`${d.label}${d.detail ? ` · ${d.detail}` : ""}${rename ? "（雙擊重新命名 window）" : ""}`}
      >
        {prefix}
        {members.length > 0 ? (
          <button
            className={`sess-fold tree-fold${folded ? " closed" : ""}`}
            aria-label={folded ? "展開 agent 列表" : "收合 agent 列表"}
            aria-expanded={!folded}
            onClick={(e) => (e.stopPropagation(), onFold())}
            onDoubleClick={(e) => e.stopPropagation()}
          >
            ›
          </button>
        ) : (
          <span className="sess-fold-space tree-fold" />
        )}
        <span className={`tree-ic ic-${d.cls}`}>{d.icon}</span>
        {editing && rename ? (
          <InlineEdit value={name ?? ""} onSave={rename} onDone={() => setEditing(false)} />
        ) : (
          <span className="tree-label">
            {name || d.label}
            {(name ? d.label : d.detail) && <span className="tree-detail"> · {name ? d.label : d.detail}</span>}
          </span>
        )}
        {p.kind === "workspace" && (
          <button className="tree-add" title="加入 agent" aria-label="加入 agent 到這個群聊" onClick={(e) => (e.stopPropagation(), requestAddAgent(p.id))}>
            ＋
          </button>
        )}
        {rename && closeWindow && !editing && <WindowMenu onRename={() => setEditing(true)} onClose={closeWindow} />}
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

function SplitRow({
  num,
  name,
  count,
  onClick,
  rename,
  closeWindow,
}: {
  num: React.ReactNode;
  name: string;
  count: number;
  onClick: () => void;
  rename: (v: string) => void;
  closeWindow: () => void;
}) {
  const [editing, setEditing] = useState(false);
  return (
    <div className="tree-row tree-row-menu" onClick={onClick} onDoubleClick={() => setEditing(true)} title="雙擊重新命名 window">
      {num}
      <span className="sess-fold-space tree-fold" />
      <span className="tree-ic ic-split">◫</span>
      {editing ? (
        <InlineEdit value={name} onSave={rename} onDone={() => setEditing(false)} />
      ) : (
        <span className="tree-label">
          {name || "分割"}
          <span className="tree-detail"> · {count} 格</span>
        </span>
      )}
      {!editing && <WindowMenu onRename={() => setEditing(true)} onClose={closeWindow} />}
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
        aria-label="window 選單"
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
          <button onClick={() => (setOpen(false), onRename())}>重新命名</button>
          <button className="danger" onClick={() => (setOpen(false), onClose())}>
            關閉 window
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
      {session.windows.map((w, i) => {
        const ids = leaves(w.root).filter((id) => state.panels[id]);
        const on = w.id === session.active_window;
        const select = () => send({ t: "window_select", session: session.id, window: w.id });
        const num = <span className="tree-num">{i + 1}</span>;
        const name = w.name && w.name !== String(i + 1) ? w.name : "";
        const rename = (v: string) => send({ t: "window_rename", session: session.id, window: w.id, name: v });
        const closeWindow = () => send({ t: "window_close", session: session.id, window: w.id });
        if (ids.length === 1) {
          const p = state.panels[ids[0]];
          return (
            <div key={w.id} className={`tree-win${on ? " on" : ""}`}>
              <PanelRow p={p} docs={docs} terms={terms} members={roster[p.id] ?? []} onClick={select} prefix={num} name={name} rename={rename} closeWindow={closeWindow} folded={folded.has(p.id)} onFold={() => fold(p.id)} />
            </div>
          );
        }
        return (
          <div key={w.id} className={`tree-win${on ? " on" : ""}`}>
            <SplitRow num={num} name={name} count={ids.length} onClick={select} rename={rename} closeWindow={closeWindow} />
            <div className="tree-panels">
              {ids.map((id) => (
                <PanelRow key={id} p={state.panels[id]} docs={docs} terms={terms} members={roster[id] ?? []} onClick={() => send({ t: "panel_focus", panel: id })} folded={folded.has(id)} onFold={() => fold(id)} />
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}
