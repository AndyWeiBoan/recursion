// Group-chat panel. Chat layout: your messages on the right, agents on the left with avatar + name.
import DOMPurify from "dompurify";
import { marked } from "marked";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { onAddAgentRequest, onReconnect, openExternal, send, useModels, useRoster, useSnapshot, useWorkspace } from "./client";
import { chatsOf, leaves, type AgentKind, type DocVia, type Member, type Msg, type TurnUsage } from "./types";

marked.setOptions({ gfm: true, breaks: false });

const AVATAR_COLORS = ["#e8714a", "#3fb8a8", "#6a8cff", "#c792ea", "#e0a83a", "#4ab36b", "#e05d8c", "#4aa3d9"];
export function colorFor(id: string) {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return AVATAR_COLORS[h % AVATAR_COLORS.length];
}

/** Markdown → sanitized HTML, with @alias mentions highlighted (outside tags and code). */
function renderMarkdown(text: string, aliases: string[]): string {
  let html = DOMPurify.sanitize(marked.parse(text, { async: false }) as string);
  if (aliases.length) {
    const known = new Set(aliases.map((a) => a.toLowerCase()).concat(["all", "全部", "所有人"]));
    let inCode = 0;
    html = html
      .split(/(<[^>]+>)/)
      .map((part) => {
        if (part.startsWith("<")) {
          if (/^<(code|pre)\b/i.test(part)) inCode++;
          else if (/^<\/(code|pre)>/i.test(part)) inCode = Math.max(0, inCode - 1);
          return part;
        }
        if (inCode) return part;
        return part.replace(/(^|[^\w.])@([\w\-一-鿿]+)/g, (m, pre, name) => (known.has(name.toLowerCase()) ? `${pre}<span class="mention">@${name}</span>` : m));
      })
      .join("");
  }
  return html;
}

const KIND_NAME: Record<AgentKind, string> = { claude: "Claude", codex: "Codex" };

export const STATUS_LABEL: Record<Member["status"], string> = {
  idle: "閒置",
  starting: "啟動中",
  thinking: "思考中",
  waiting_permission: "等你核准",
};

function time(ts: number) {
  const d = new Date(ts);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function tokens(n: number) {
  return n < 1000 ? String(n) : n < 1e6 ? `${(n / 1000).toFixed(n < 1e4 ? 1 : 0)}k` : `${(n / 1e6).toFixed(1)}M`;
}

function UsageTag({ u }: { u: TurnUsage }) {
  const cost = u.cost_usd === undefined ? "" : u.cost_usd < 0.01 ? " · <$0.01" : ` · $${u.cost_usd.toFixed(2)}`;
  const detail = `這一輪：輸入 ${u.input.toLocaleString()} tokens（其中快取 ${u.cached.toLocaleString()}）、輸出 ${u.output.toLocaleString()} tokens${
    u.cost_usd === undefined ? "" : `、約 $${u.cost_usd.toFixed(4)}`
  }`;
  return (
    <span className="usage" title={detail}>
      {tokens(u.input + u.output)} tokens
      {cost}
    </span>
  );
}

/** What a doc page sent back, as a card: field → value when it is a JSON object, text otherwise. */
function DocSubmission({ via, panel }: { via: DocVia; panel: string }) {
  let fields: [string, string][] | null = null;
  try {
    const v = JSON.parse(via.data);
    if (v && typeof v === "object" && !Array.isArray(v))
      fields = Object.entries(v).map(([k, x]) => [k, Array.isArray(x) ? x.join("、") : typeof x === "object" && x !== null ? JSON.stringify(x) : String(x)]);
  } catch {}
  return (
    <div className="doc-sub">
      <button className="doc-sub-head" onClick={() => send({ t: "doc_open", panel, doc: via.doc })} title="打開這份文件">
        <span className="m-doc-ic" aria-hidden="true">
          ▤
        </span>
        <span>
          你在<b>「{via.title}」</b>送出
        </span>
      </button>
      {fields ? (
        <dl className="doc-sub-fields">
          {fields.map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
      ) : (
        <div className="doc-sub-text">{via.data}</div>
      )}
    </div>
  );
}

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
      <MembersBar panel={panel} members={members} bigForm={members.length === 0 && ws.messages.length === 0} />
      {members.length === 0 && ws.messages.length === 0 ? (
        <div className="ws-log ws-center">
          <AddAgent panel={panel} big />
        </div>
      ) : (
        <Transcript panel={panel} messages={ws.messages} byId={byId} busy={busy} />
      )}
      <Composer panel={panel} members={members} disabled={members.length === 0} busy={busy.length > 0} focused={focused} round={ws.round} />
    </div>
  );
}

export function Avatar({ m, size = 26 }: { m?: Member; size?: number }) {
  const label = (m?.alias ?? "?").slice(0, 1).toUpperCase();
  return (
    <span className="avatar" style={{ width: size, height: size, background: m ? colorFor(m.id) : "#4a4b52", fontSize: size * 0.46 }}>
      {label}
    </span>
  );
}

/** Centered dialog over the whole app. Closes on ✕, Esc, or a click outside. */
function Modal({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const restore = overlay(); // native browser panels would draw on top of it
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close.current();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      restore();
    };
  }, []);
  return createPortal(
    <div className="modal-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-label={title}>
        <button className="modal-x" onClick={onClose} aria-label="關閉" title="關閉（Esc）">
          ✕
        </button>
        {children}
      </div>
    </div>,
    document.body,
  );
}

// Who is in the chat is listed in the sidebar tree; this bar keeps "add" and the account meters.
function MembersBar({ panel, members, bigForm }: { panel: string; members: Member[]; bigForm: boolean }) {
  const [adding, setAdding] = useState(false);
  useEffect(() => onAddAgentRequest(panel, () => setAdding(true)), [panel]);
  // The empty chat shows the big form instead; otherwise "add" must always be reachable,
  // including when every agent was removed from a chat that has history.
  if (bigForm) return <div className="ws-members is-empty" />;
  const cost = members.reduce((s, m) => s + (m.cost_usd || 0), 0);
  const usage = members.find((m) => m.usage?.five_hour !== undefined)?.usage;
  return (
    <div className="ws-members">
      <div className="chip-wrap">
        <button className="chip add" onClick={() => setAdding((v) => !v)}>
          ＋ agent
        </button>
        {adding && (
          <Modal title="新增 agent" onClose={() => setAdding(false)}>
            <AddAgent panel={panel} onDone={() => setAdding(false)} />
          </Modal>
        )}
      </div>
      <span className="ws-meta">
        {usage?.five_hour !== undefined && <>5h {Math.round(usage.five_hour * 100)}%</>}
        {usage?.seven_day !== undefined && <> · 7d {Math.round(usage.seven_day * 100)}%</>}
        {cost > 0 && <> · ${cost.toFixed(2)}</>}
      </span>
    </div>
  );
}

const KIND_INFO: Record<AgentKind, { vendor: string; glyph: string; color: string; hint: string }> = {
  claude: { vendor: "Anthropic", glyph: "✳", color: "#e8714a", hint: "用你登入的 claude 帳號" },
  codex: { vendor: "OpenAI", glyph: "◎", color: "#3fb8a8", hint: "用你登入的 codex 帳號" },
};

function AddAgent({ panel, big, onDone }: { panel: string; big?: boolean; onDone?: () => void }) {
  const [kind, setKind] = useState<AgentKind>("claude");
  // Any chat of this session can be the target; the one it was opened from is the default.
  const { state } = useSnapshot();
  const chats = useMemo(() => {
    const s = state?.sessions.find((x) => x.windows.some((w) => leaves(w.root).includes(panel)));
    return s && state ? chatsOf(s, state) : [];
  }, [state, panel]);
  const [target, setTarget] = useState(panel);
  const roster = useRoster(chats.map((c) => c.panel).join(","));
  const free = (base: string, chat = target) => {
    const taken = (roster[chat] ?? []).map((m) => m.alias.toLowerCase());
    if (!taken.includes(base)) return base;
    for (let i = 2; ; i++) if (!taken.includes(base + i)) return base + i;
  };
  const [alias, setAlias] = useState(free("claude"));
  const [touched, setTouched] = useState(false);
  // The member list may arrive after the form opens: keep the suggested alias free of clashes.
  const takenKey = (roster[target] ?? []).map((m) => m.alias).join(",");
  useEffect(() => {
    if (!touched) setAlias(free(kind));
  }, [takenKey]);
  const [model, setModel] = useState("");
  const list = useModels(kind);
  const options = list.models.length ? list.models : [{ value: "", label: "預設模型", description: "" }];
  const picked = options.find((m) => m.value === model);
  const id = `${panel}${big ? "-big" : "-pop"}`;
  return (
    <form
      className={`add-card${big ? " big" : ""}`}
      onSubmit={(e) => {
        e.preventDefault();
        send({ t: "ws_add_agent", panel: target, kind, alias, model: model || null });
        if (target !== panel) send({ t: "panel_focus", panel: target });
        onDone?.();
      }}
    >
      <div className="add-head">
        <h3>{big ? "這個群聊還沒有成員" : "新增 agent"}</h3>
        {big && <p>加入一個 agent 開始對話，之後隨時可以再加人。</p>}
      </div>
      {chats.length > 1 && (
        <div className="field">
          <label htmlFor={`chat-${id}`}>加到哪個群聊</label>
          <div className="select-wrap">
            <select
              id={`chat-${id}`}
              value={target}
              onChange={(e) => {
                setTarget(e.target.value);
                if (!touched) setAlias(free(kind, e.target.value));
              }}
            >
              {chats.map((c) => (
                <option key={c.panel} value={c.panel}>
                  {c.label}
                  {(roster[c.panel] ?? []).length ? `：${roster[c.panel].map((m) => `@${m.alias}`).join(" ")}` : "（還沒有人）"}
                </option>
              ))}
            </select>
            <span className="chev" aria-hidden="true">
              ⌄
            </span>
          </div>
        </div>
      )}
      <div className="kind-cards" role="radiogroup" aria-label="agent 種類">
        {(["claude", "codex"] as AgentKind[]).map((k) => (
          <button
            key={k}
            type="button"
            role="radio"
            aria-checked={kind === k}
            className={`kind-card${kind === k ? " on" : ""}`}
            onClick={() => {
              setKind(k);
              setModel("");
              if (!touched) setAlias(free(k));
            }}
          >
            <span className="kind-glyph" style={{ background: KIND_INFO[k].color }}>
              {KIND_INFO[k].glyph}
            </span>
            <span className="kind-text">
              <b>{KIND_NAME[k]}</b>
              <span>{KIND_INFO[k].vendor}</span>
            </span>
            <span className="kind-check" aria-hidden="true">
              ✓
            </span>
          </button>
        ))}
      </div>
      <div className="field">
        <label htmlFor={`alias-${id}`}>別名</label>
        <div className="input-wrap">
          <span className="at">@</span>
          <input
            id={`alias-${id}`}
            value={alias}
            onChange={(e) => (setAlias(e.target.value), setTouched(true))}
            spellCheck={false}
            autoComplete="off"
            autoFocus={!big}
          />
        </div>
      </div>
      <div className="field">
        <label htmlFor={`model-${id}`}>模型</label>
        <div className="select-wrap">
          <select id={`model-${id}`} value={model} onChange={(e) => setModel(e.target.value)}>
            {options.map((m) => (
              <option key={m.value} value={m.value} title={m.description}>
                {m.label}
              </option>
            ))}
          </select>
          <span className="chev" aria-hidden="true">
            ⌄
          </span>
        </div>
        <span className="field-hint">
          {list.loading ? `正在問 ${KIND_NAME[kind]} 有哪些模型…` : list.error ? `讀不到模型清單：${list.error}` : picked?.description}
        </span>
      </div>
      <button className="add-submit" type="submit" disabled={!alias.trim()}>
        加入 @{alias.trim() || "…"}
      </button>
      <p className="add-hint">{KIND_INFO[kind].hint}</p>
    </form>
  );
}

function Transcript({ panel, messages, byId, busy }: { panel: string; messages: Msg[]; byId: Map<string, Member>; busy: Member[] }) {
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const last = messages[messages.length - 1];
  const lastLen = last && last.kind === "text" ? last.text.length : 0;
  const aliases = useMemo(() => [...byId.values()].map((m) => m.alias), [byId]);
  // Agents already visibly producing output (streaming text, running tool) don't need the spinner.
  const producing = new Set(
    messages.filter((m) => (m.kind === "text" || m.kind === "tool") && (m.streaming || (m.kind === "tool" && m.result == null))).map((m) => m.from),
  );
  const waiting = busy.filter((b) => b.status === "waiting_permission" || !producing.has(b.id));

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
      {messages.map((m, i) => {
        const prev = messages[i - 1];
        const first = !prev || prev.from !== m.from || prev.kind === "system" || prev.kind === "guard" || !!m.turn;
        return <MessageView key={m.id} panel={panel} m={m} who={byId.get(m.from)} byId={byId} first={first} aliases={aliases} />;
      })}
      {waiting.map((b) => (
        <div key={b.id} className="row agent">
          <Avatar m={b} />
          <div className="ws-busy">
            <span className="spin">✳</span> @{b.alias} {STATUS_LABEL[b.status]}… <span className="dim">esc 停止</span>
          </div>
        </div>
      ))}
    </div>
  );
}

const MessageView = memo(function MessageView({
  panel,
  m,
  who,
  byId,
  first,
  aliases,
}: {
  panel: string;
  m: Msg;
  who?: Member;
  byId: Map<string, Member>;
  first: boolean;
  aliases: string[];
}) {
  const [open, setOpen] = useState(false);

  if (m.from === "you" && m.kind === "text") {
    const to = m.to?.length ? m.to.map((id) => "@" + (byId.get(id)?.alias ?? "?")).join(" ") : "全部";
    return (
      <div className="row you">
        <div className="bubble-col">
          {m.via ? <DocSubmission via={m.via} panel={panel} /> : <div className="bubble you" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text, aliases) }} />}
          <div className="meta">
            → {to} · {time(m.ts)}
          </div>
        </div>
      </div>
    );
  }
  if (m.kind === "system") return <div className="m-sys">{m.text}</div>;
  if (m.kind === "guard") {
    return (
      <div className={`m-guard${m.resolved ? " done" : ""}`}>
        <span>⚠ {m.text}</span>
        {!m.resolved && (
          <span className="acts">
            <button onClick={() => send({ t: "ws_stop", panel })}>停止</button>
          </span>
        )}
      </div>
    );
  }

  let body: React.ReactNode;
  switch (m.kind) {
    case "text":
      body = (
        <div className="bubble agent">
          <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text, aliases) + (m.streaming ? '<span class="caret"></span>' : "") }} />
        </div>
      );
      break;
    case "tool": {
      const firstLine = (m.result ?? "").split("\n").find((l) => l.trim()) ?? "";
      body = (
        <div className={`m-tool${m.is_error ? " err" : ""}`}>
          <button className="m-tool-head" onClick={() => setOpen((v) => !v)} title="點擊展開">
            <span className="tool-ic">{open ? "▾" : "▸"}</span>
            <b>{m.name}</b>
            {m.summary && <span className="m-tool-sum">{m.summary}</span>}
          </button>
          <div className="m-tool-res">⎿ {m.result == null ? "執行中…" : firstLine || "（沒有輸出）"}</div>
          {open && (
            <div className="m-tool-detail">
              {m.detail && <pre>{m.detail}</pre>}
              {m.result && <pre className="res">{m.result}</pre>}
            </div>
          )}
        </div>
      );
      break;
    }
    case "permission":
      body = (
        <div className={`m-perm st-${m.state}`}>
          <div className="m-perm-head">
            想要使用 <b>{m.tool}</b>
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
      break;
    case "doc":
      body = (
        <button className="m-doc" onClick={() => send({ t: "doc_open", panel, doc: m.doc })} title="在旁邊的 panel 打開">
          <span className="m-doc-ic" aria-hidden="true">
            ▤
          </span>
          <span className="m-doc-text">
            <b>{m.title}</b>
            <span>{m.updated ? "更新了文件" : "文件"} · 點一下打開</span>
          </span>
        </button>
      );
      break;
    case "error":
      body = (
        <div className="m-err">
          <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text, []) }} />
        </div>
      );
      break;
  }
  return (
    <div className={`row agent${first ? " first" : ""}`}>
      {first ? <Avatar m={who} /> : <span className="avatar-space" />}
      <div className="agent-col">
        {first && (
          <div className="name">
            <b style={{ color: who ? colorFor(who.id) : undefined }}>@{who?.alias ?? "agent"}</b>
            {who && <span>{who.model ?? who.kind}</span>}
            <span>{time(m.ts)}</span>
            {m.usage && <UsageTag u={m.usage} />}
          </div>
        )}
        {body}
      </div>
    </div>
  );
});

function Composer({
  panel,
  members,
  disabled,
  busy,
  focused,
  round,
}: {
  panel: string;
  members: Member[];
  disabled: boolean;
  busy: boolean;
  focused: boolean;
  round?: { active: boolean; queued: number };
}) {
  const [text, setText] = useState("");
  const [sel, setSel] = useState(0);
  const [caret, setCaret] = useState(0);
  const [menuOff, setMenuOff] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (focused && !disabled) ref.current?.focus();
  }, [focused, disabled]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight + 2, 200) + "px";
    el.style.overflowY = el.scrollHeight > 200 ? "auto" : "hidden";
  }, [text]);

  // @mention autocomplete for the word under the caret.
  const token = /(^|\s)@([\w\-一-鿿]*)$/.exec(text.slice(0, caret));
  const options = token
    ? [...members.map((m) => ({ alias: m.alias, label: m.model ?? m.kind, m })), { alias: "all", label: "所有 agent", m: undefined }].filter((o) =>
        o.alias.toLowerCase().startsWith(token[2].toLowerCase()),
      )
    : [];
  const showMenu = options.length > 0 && !disabled && !menuOff;

  const pick = (alias: string) => {
    if (!token) return;
    const start = caret - token[2].length - 1;
    const next = text.slice(0, start) + "@" + alias + " " + text.slice(caret);
    setText(next);
    const pos = start + alias.length + 2;
    requestAnimationFrame(() => {
      ref.current?.setSelectionRange(pos, pos);
      setCaret(pos);
    });
    setSel(0);
  };
  const submit = () => {
    if (!text.trim() || disabled) return;
    send({ t: "ws_send", panel, text });
    setText("");
    setCaret(0);
  };
  const queued = round?.active ? round.queued : 0;
  return (
    <div className="ws-input">
      {queued > 0 && (
        <div className="queue-bar">
          <span>已排隊 {queued} 則，本回合結束後送出</span>
          <button onClick={() => send({ t: "ws_send_now", panel })}>中斷本回合，立即送出</button>
        </div>
      )}
      {showMenu && (
        <div className="mention-menu" role="listbox">
          {options.map((o, i) => (
            <button key={o.alias} className={i === Math.min(sel, options.length - 1) ? "on" : ""} onMouseDown={(e) => (e.preventDefault(), pick(o.alias))}>
              {o.m ? <Avatar m={o.m} size={18} /> : <span className="avatar all">＊</span>}@{o.alias}
              <span className="dim">{o.label}</span>
            </button>
          ))}
        </div>
      )}
      <textarea
        ref={ref}
        rows={1}
        value={text}
        disabled={disabled}
        placeholder={disabled ? "先加入一個 agent" : "傳訊息（沒有 @ 就問全部 agent；Enter 送出，Shift+Enter 換行）"}
        onChange={(e) => {
          setText(e.target.value);
          setCaret(e.target.selectionStart ?? e.target.value.length);
          setSel(0);
          setMenuOff(false);
        }}
        onSelect={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing || e.keyCode === 229) return; // IME (注音、拼音) still composing
          if (showMenu) {
            if (e.key === "ArrowDown") return e.preventDefault(), setSel((s) => (s + 1) % options.length);
            if (e.key === "ArrowUp") return e.preventDefault(), setSel((s) => (s + options.length - 1) % options.length);
            if (e.key === "Tab" || e.key === "Enter") return e.preventDefault(), pick(options[Math.min(sel, options.length - 1)].alias);
            if (e.key === "Escape") return e.preventDefault(), setMenuOff(true);
          }
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            submit();
          } else if (e.key === "Escape" && busy) {
            e.preventDefault();
            send({ t: "ws_stop", panel });
          }
        }}
      />
      {busy ? (
        <button className="stop-btn" onClick={() => send({ t: "ws_stop", panel })} title="停止（esc）">
          ■ 停止
        </button>
      ) : (
        <button className="send-btn" onClick={submit} disabled={disabled || !text.trim()} title="送出（Enter）">
          ↑
        </button>
      )}
    </div>
  );
}
