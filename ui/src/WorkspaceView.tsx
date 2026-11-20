// Group-chat panel: Rex/Claude-Code-style transcript (❯ you, ● agent, ⎿ tool result) with Markdown.
import DOMPurify from "dompurify";
import { marked } from "marked";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { onReconnect, openExternal, send, useWorkspace } from "./client";
import type { Member, Msg } from "./types";

marked.setOptions({ gfm: true, breaks: false });

function renderMarkdown(text: string): string {
  const html = marked.parse(text, { async: false }) as string;
  return DOMPurify.sanitize(html, { ADD_ATTR: ["target"] });
}

const MODELS: { value: string; label: string }[] = [
  { value: "", label: "預設模型" },
  { value: "opus", label: "Opus" },
  { value: "sonnet", label: "Sonnet" },
  { value: "haiku", label: "Haiku" },
];

const STATUS_LABEL: Record<Member["status"], string> = {
  idle: "閒置",
  starting: "啟動中",
  thinking: "回覆中",
  waiting_permission: "等你核准",
};

export function WorkspaceView({ panel, focused }: { panel: string; focused: boolean }) {
  const ws = useWorkspace(panel);
  useEffect(() => {
    send({ t: "ws_attach", panel });
    const off = onReconnect(() => send({ t: "ws_attach", panel }));
    return () => {
      off();
    };
  }, [panel]);

  const members = ws?.members ?? [];
  const byId = useMemo(() => new Map(members.map((m) => [m.id, m])), [members]);
  const busy = members.filter((m) => m.status !== "idle");

  if (!ws) return <div className="ws"><div className="ws-log" /></div>;

  return (
    <div className="ws">
      <MembersBar panel={panel} members={members} />
      {members.length === 0 && ws.messages.length === 0 ? (
        <div className="ws-log ws-center">
          <AddAgent panel={panel} big />
        </div>
      ) : (
        <Transcript messages={ws.messages} byId={byId} busy={busy} />
      )}
      <Composer panel={panel} disabled={members.length === 0} busy={busy.length > 0} focused={focused} />
    </div>
  );
}

function MembersBar({ panel, members }: { panel: string; members: Member[] }) {
  const [menu, setMenu] = useState<string | null>(null);
  if (members.length === 0) return null;
  const cost = members.reduce((s, m) => s + (m.cost_usd || 0), 0);
  const usage = members.find((m) => m.usage?.five_hour !== undefined)?.usage;
  return (
    <div className="ws-members">
      {members.map((m) => (
        <div key={m.id} className="chip-wrap">
          <button className={`chip st-${m.status}`} onClick={() => setMenu(menu === m.id ? null : m.id)} title={STATUS_LABEL[m.status]}>
            <i />@{m.alias}
            {m.model && <span className="chip-model">{m.model}</span>}
          </button>
          {menu === m.id && (
            <div className="menu" onMouseLeave={() => setMenu(null)}>
              <button className="danger" onClick={() => (setMenu(null), send({ t: "ws_remove_agent", panel, member: m.id }))}>
                移除 @{m.alias}
              </button>
            </div>
          )}
        </div>
      ))}
      <span className="ws-meta">
        {usage?.five_hour !== undefined && <>5h {Math.round(usage.five_hour * 100)}%</>}
        {usage?.seven_day !== undefined && <> · 7d {Math.round(usage.seven_day * 100)}%</>}
        {cost > 0 && <> · ${cost.toFixed(2)}</>}
      </span>
    </div>
  );
}

function AddAgent({ panel, big }: { panel: string; big?: boolean }) {
  const [alias, setAlias] = useState("claude");
  const [model, setModel] = useState("");
  return (
    <form
      className={`add-agent${big ? " big" : ""}`}
      onSubmit={(e) => {
        e.preventDefault();
        send({ t: "ws_add_agent", panel, kind: "claude", alias, model: model || null });
      }}
    >
      {big && (
        <>
          <div className="ws-glyph">✳</div>
          <h3>加入一個 agent 開始對話</h3>
          <p className="dim">目前支援 Claude，用你已登入的 claude 帳號。</p>
        </>
      )}
      <div className="add-row">
        <label>
          <span>別名</span>
          <span className="alias-in">
            @<input id={`alias-${panel}`} value={alias} onChange={(e) => setAlias(e.target.value)} spellCheck={false} />
          </span>
        </label>
        <label>
          <span>模型</span>
          <select id={`model-${panel}`} value={model} onChange={(e) => setModel(e.target.value)}>
            {MODELS.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      <button className="primary" type="submit">
        加入 Claude
      </button>
    </form>
  );
}

function Transcript({ messages, byId, busy }: { messages: Msg[]; byId: Map<string, Member>; busy: Member[] }) {
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const last = messages[messages.length - 1];
  const lastLen = last && last.kind === "text" ? last.text.length : 0;

  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages.length, lastLen, busy.length]);

  return (
    <div
      className="ws-log"
      ref={box}
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      }}
      onClick={(e) => {
        const a = (e.target as HTMLElement).closest("a");
        if (a?.href) {
          e.preventDefault();
          openExternal(a.href);
        }
      }}
    >
      {messages.map((m) => (
        <MessageView key={m.id} m={m} who={byId.get(m.from)} />
      ))}
      {busy.map((b) => (
        <div key={b.id} className="ws-busy">
          <span className="spin">✳</span> @{b.alias} {STATUS_LABEL[b.status]}… <span className="dim">esc 停止</span>
        </div>
      ))}
    </div>
  );
}

const MessageView = memo(function MessageView({ m, who }: { m: Msg; who?: Member }) {
  const [open, setOpen] = useState(false);
  if (m.from === "you" && m.kind === "text") {
    return <div className="m-you">{m.text}</div>;
  }
  switch (m.kind) {
    case "text":
      return (
        <div className="m-agent">
          <span className="dot">●</span>
          <div className="m-body">
            {who && <div className="m-who">@{who.alias}</div>}
            <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text) + (m.streaming ? '<span class="caret"></span>' : "") }} />
          </div>
        </div>
      );
    case "tool": {
      const firstLine = (m.result ?? "").split("\n").find((l) => l.trim()) ?? "";
      return (
        <div className={`m-tool${m.is_error ? " err" : ""}`}>
          <button className="m-tool-head" onClick={() => setOpen((v) => !v)}>
            <span className="dot">●</span>
            <b>{m.name}</b>
            {m.summary && <span className="m-tool-sum">({m.summary})</span>}
          </button>
          <div className="m-tool-res">
            ⎿ {m.result == null ? (m.streaming ? "…" : "執行中…") : firstLine || "（沒有輸出）"}
            {m.result && m.result.split("\n").length > 1 && !open && <span className="dim"> · 點上方展開</span>}
          </div>
          {open && (
            <div className="m-tool-detail">
              {m.detail && <pre>{m.detail}</pre>}
              {m.result && <pre className="res">{m.result}</pre>}
            </div>
          )}
        </div>
      );
    }
    case "permission":
      return (
        <div className={`m-perm st-${m.state}`}>
          <div className="m-perm-head">
            <span className="dot">◆</span>
            {who && <span className="m-who">@{who.alias}</span>} 想要使用 <b>{m.tool}</b>
          </div>
          <code className="m-perm-sum">{m.summary || m.tool}</code>
          {m.detail && m.detail !== m.summary && (
            <details>
              <summary>詳細內容</summary>
              <pre>{m.detail}</pre>
            </details>
          )}
          {m.state === "pending" ? (
            <div className="m-perm-acts">
              <button className="yes" onClick={() => send({ t: "ws_perm", request_id: m.request_id, decision: "allow" })}>
                允許
              </button>
              <button onClick={() => send({ t: "ws_perm", request_id: m.request_id, decision: "always" })}>以後都允許</button>
              <button className="no" onClick={() => send({ t: "ws_perm", request_id: m.request_id, decision: "deny" })}>
                拒絕
              </button>
            </div>
          ) : (
            <div className="m-perm-state">
              {m.state === "allowed" ? "✓ 已允許" : m.state === "always_allowed" ? "✓ 已允許（以後不再詢問）" : "✕ 已拒絕"}
            </div>
          )}
        </div>
      );
    case "system":
      return <div className="m-sys">{m.text}</div>;
    case "error":
      return (
        <div className="m-err">
          <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text) }} />
        </div>
      );
  }
});

function Composer({ panel, disabled, busy, focused }: { panel: string; disabled: boolean; busy: boolean; focused: boolean }) {
  const [text, setText] = useState("");
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (focused && !disabled) ref.current?.focus();
  }, [focused, disabled]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 200) + "px";
  }, [text]);
  const submit = () => {
    if (!text.trim() || disabled) return;
    send({ t: "ws_send", panel, text });
    setText("");
  };
  return (
    <div className="ws-input">
      <span className="prompt">❯</span>
      <textarea
        ref={ref}
        rows={1}
        value={text}
        disabled={disabled}
        placeholder={disabled ? "先加入一個 agent" : "傳訊息給 agent（Enter 送出，Shift+Enter 換行）"}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing || e.keyCode === 229) return; // IME (注音、拼音) still composing
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            submit();
          } else if (e.key === "Escape" && busy) {
            e.preventDefault();
            send({ t: "ws_stop", panel });
          }
        }}
      />
      {busy && (
        <button className="stop-btn" onClick={() => send({ t: "ws_stop", panel })} title="停止（esc）">
          ■ 停止
        </button>
      )}
    </div>
  );
}
