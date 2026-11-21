// Threads drawer on the right of a chat panel: searchable list → one thread (its quote, talk, reply box).
import { useEffect, useMemo, useRef, useState } from "react";
import { send } from "./client";
import { closeThreads, openThreads, setThreadSearch, setThreadView, type ThreadsUI } from "./threadsUI";
import { Avatar, renderMarkdown } from "./WorkspaceView";
import type { Member, Msg, Thread, Workspace } from "./types";

const STATUS: Record<Thread["status"], string> = { replying: "回覆中…", waiting: "等你回", brought: "已帶回" };

function time(ts: number) {
  const d = new Date(ts);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** Every character of `needle` appears in `hay`, in order. */
function fuzzy(needle: string, hay: string) {
  let i = 0;
  for (const c of hay) if (c === needle[i] && ++i === needle.length) return true;
  return needle.length === 0;
}

/** `#3` → thread number, `@claude` → agent, anything else → fuzzy over quote, talk and agent. */
function matches(t: Thread, query: string, alias: string) {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((tok) => {
      if (/^#\d+$/.test(tok)) return t.num === Number(tok.slice(1));
      if (tok.startsWith("@")) return alias.toLowerCase().includes(tok.slice(1));
      return fuzzy(tok, `${t.quote} ${t.messages.map((m) => m.text).join(" ")} ${alias}`.toLowerCase());
    });
}

/** Scroll the main chat to a message and flash it. */
export function showInChat(msg: string) {
  const el = document.querySelector<HTMLElement>(`[data-msg="${msg}"]`);
  if (!el) return;
  el.scrollIntoView({ block: "center", behavior: "smooth" });
  el.classList.remove("flash");
  void el.offsetWidth;
  el.classList.add("flash");
}

export function ThreadDrawer({ panel, ws, byId, ui }: { panel: string; ws: Workspace; byId: Map<string, Member>; ui: ThreadsUI }) {
  const threads = ws.threads ?? [];
  const root = useRef<HTMLElement>(null);
  const back = () => (ui.view.kind === "list" ? closeThreads(panel) : setThreadView(panel, { kind: "list" }));
  return (
    <aside
      className="thread-drawer"
      ref={root}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          back();
        }
      }}
    >
      {ui.view.kind === "list" && <ThreadList panel={panel} threads={threads} messages={ws.messages} byId={byId} ui={ui} />}
      {ui.view.kind === "new" && <NewThread panel={panel} msg={ui.view.msg} quote={ui.view.quote} who={byId.get(ws.messages.find((m) => m.id === (ui.view as { msg: string }).msg)?.from ?? "")} />}
      {ui.view.kind === "thread" &&
        (() => {
          const id = ui.view.id;
          const t = threads.find((x) => x.id === id);
          return t ? <ThreadView panel={panel} t={t} byId={byId} /> : <div className="td-empty">找不到這個討論串</div>;
        })()}
    </aside>
  );
}

function ThreadList({ panel, threads, messages, byId, ui }: { panel: string; threads: Thread[]; messages: Msg[]; byId: Map<string, Member>; ui: ThreadsUI }) {
  const anchor = ui.msg ? messages.find((m) => m.id === ui.msg) : undefined;
  const shown = useMemo(
    () =>
      threads
        .filter((t) => !ui.msg || t.msg === ui.msg)
        .filter((t) => matches(t, ui.query, byId.get(t.agent)?.alias ?? ""))
        .sort((a, b) => b.updated - a.updated),
    [threads, ui.msg, ui.query, byId],
  );
  return (
    <>
      <header className="td-head">
        <b>討論串</b>
        <span className="dim">{threads.length}</span>
        <button className="td-x" onClick={() => closeThreads(panel)} aria-label="關閉討論串" title="關閉（Esc）">
          ✕
        </button>
      </header>
      <div className="td-search">
        {anchor && (
          <span className="td-chip" title="只看這則回覆的討論串">
            回覆 {time(anchor.ts)} @{byId.get(anchor.from)?.alias ?? "?"}
            <button onClick={() => setThreadSearch(panel, { msg: null })} aria-label="清除條件">
              ✕
            </button>
          </span>
        )}
        <input
          autoFocus
          value={ui.query}
          onChange={(e) => setThreadSearch(panel, { query: e.target.value })}
          placeholder={anchor ? "在這幾個裡面搜尋…" : "搜尋：#3、@claude、或任何字"}
          spellCheck={false}
        />
      </div>
      <div className="td-list">
        {shown.map((t) => {
          const who = byId.get(t.agent);
          return (
            <div key={t.id} className={`td-row st-${t.status}`}>
              <button className="td-open" onClick={() => setThreadView(panel, { kind: "thread", id: t.id })}>
                <span className="td-num">#{t.num}</span>
                <span className="td-main">
                  <span className="td-quote">「{t.quote.replace(/\s+/g, " ")}」</span>
                  <span className="td-meta">
                    @{who?.alias ?? "已離開"} · {t.messages.filter((m) => m.from !== "system").length} 則 · <i className="td-dot" />
                    {STATUS[t.status]}
                  </span>
                </span>
              </button>
              <button className="td-goto" onClick={() => showInChat(t.msg)} title="在主對話中找到原訊息" aria-label={`#${t.num} 的原訊息`}>
                ↖
              </button>
            </div>
          );
        })}
        {shown.length === 0 && <div className="td-empty">{threads.length ? "沒有符合的討論串" : "還沒有討論串。在 agent 的回覆旁按 💬 開一個。"}</div>}
      </div>
      {anchor && (
        <button className="td-new" onClick={() => setThreadView(panel, { kind: "new", msg: anchor.id, quote: "" })}>
          ＋ 對這則回覆開新的討論串
        </button>
      )}
    </>
  );
}

function Quote({ text, msg }: { text: string; msg: string }) {
  return (
    <button className="td-quote-block" onClick={() => showInChat(msg)} title="在主對話中顯示">
      {text}
    </button>
  );
}

function Composer({ placeholder, onSend, disabled }: { placeholder: string; onSend: (text: string) => void; disabled?: boolean }) {
  const [text, setText] = useState("");
  const submit = () => {
    if (!text.trim() || disabled) return;
    onSend(text.trim());
    setText("");
  };
  return (
    <div className="td-compose">
      <textarea
        autoFocus
        value={text}
        rows={3}
        placeholder={placeholder}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            submit();
          }
        }}
      />
      <button className="td-send" onClick={submit} disabled={disabled || !text.trim()} aria-label="送出">
        ↑
      </button>
    </div>
  );
}

function NewThread({ panel, msg, quote, who }: { panel: string; msg: string; quote: string; who?: Member }) {
  const [sent, setSent] = useState(false);
  return (
    <>
      <header className="td-head">
        <button className="td-back" onClick={() => setThreadView(panel, { kind: "list" })}>
          ← 返回
        </button>
        <b>新的討論串</b>
      </header>
      <div className="td-body">
        <p className="td-hint">只有你和 @{who?.alias ?? "agent"} 看得到這段討論，它從主對話分岔出來，記得前面所有內容。</p>
        {quote ? <Quote text={quote} msg={msg} /> : <div className="td-hint">（整則回覆）</div>}
      </div>
      <Composer
        placeholder={sent ? "開啟中…" : `問 @${who?.alias ?? "agent"}…（Enter 送出）`}
        disabled={sent}
        onSend={(text) => {
          setSent(true);
          send({ t: "thread_open", panel, msg, quote, text });
        }}
      />
    </>
  );
}

function ThreadView({ panel, t, byId }: { panel: string; t: Thread; byId: Map<string, Member> }) {
  const who = byId.get(t.agent);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [t.messages.length, t.status]);
  const busy = t.status === "replying";
  return (
    <>
      <header className="td-head">
        <button className="td-back" onClick={() => setThreadView(panel, { kind: "list" })}>
          ← 返回
        </button>
        <b>#{t.num}</b>
        <span className="dim">
          @{who?.alias ?? "已離開"} · {STATUS[t.status]}
        </span>
        <button className="td-x" onClick={() => closeThreads(panel)} aria-label="關閉討論串" title="關閉（Esc 返回）">
          ✕
        </button>
      </header>
      <div className="td-body" ref={box}>
        <Quote text={t.quote} msg={t.msg} />
        {t.messages.map((m, i) =>
          m.from === "system" ? (
            <div key={i} className="td-sys">
              {m.text}
            </div>
          ) : m.from === "you" ? (
            <div key={i} className="td-msg you">
              {m.text}
            </div>
          ) : (
            <div key={i} className="td-msg agent">
              <span className="td-who">
                <Avatar m={byId.get(m.from)} size={16} /> @{byId.get(m.from)?.alias ?? "agent"} · {time(m.ts)}
              </span>
              <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text, []) }} />
            </div>
          ),
        )}
        {busy && (
          <div className="td-sys">
            <span className="spin">✳</span> @{who?.alias ?? "agent"} 回覆中…
          </div>
        )}
      </div>
      <Composer placeholder={busy ? "等 agent 回完…" : `回覆 @${who?.alias ?? "agent"}…（Enter 送出）`} disabled={busy} onSend={(text) => send({ t: "thread_send", panel, thread: t.id, text })} />
      <div className="td-foot">
        <button className="td-bring" disabled={busy || !t.fork} onClick={() => send({ t: "thread_bring", panel, thread: t.id })} title="請 agent 寫一段結論，貼回主對話">
          ↩ 帶回主對話
        </button>
      </div>
    </>
  );
}

/** Open the drawer from a message's 💬: its threads if it has some, else a new one. */
export function openThreadsFor(panel: string, msg: string, count: number, selection: string) {
  if (count > 0 && !selection) openThreads(panel, { view: { kind: "list" }, msg, query: "" });
  else openThreads(panel, { view: { kind: "new", msg, quote: selection }, msg });
}
