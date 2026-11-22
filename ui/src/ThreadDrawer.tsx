// 產出 drawer on the right of a chat panel: one searchable list of its threads, documents and pages
// → one thread (its quote, talk, reply box), or the document / page opened in its panel.
import { useDragWidth } from "./Layout";
import { t as tr } from "./i18n";
import { useEffect, useMemo, useRef, useState } from "react";
import { send } from "./client";
import { closeThreads, openThreads, setThreadSearch, setThreadView, unzoom, type ArtifactFilter, type ThreadsUI } from "./threadsUI";
import { Avatar, renderMarkdown } from "./WorkspaceView";
import { PollRow, PollView } from "./Polls";
import type { Artifact, Member, Poll, Thread, Workspace } from "./types";

const STATUS: Record<Thread["status"], string> = {
  get replying() { return tr("thread.replying"); },
  get waiting() { return tr("thread.waiting"); },
  get brought() { return tr("thread.brought"); },
};

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

/** `#3` → thread number, `@claude` → agent, anything else → fuzzy over kind (整則/片段), quote, talk and agent. */
function matches(t: Thread, query: string, alias: string) {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((tok) => {
      if (/^#\d+$/.test(tok)) return t.num === Number(tok.slice(1));
      if (tok.startsWith("@")) return alias.toLowerCase().includes(tok.slice(1));
      return fuzzy(tok, `${t.whole ? tr("thread.whole") : tr("thread.part")} ${t.quote} ${t.messages.map((m) => m.text).join(" ")} ${alias}`.toLowerCase());
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
  const size = useDragWidth({ key: "recursion.drawerWidth", initial: 380, min: 280, max: 760, edge: "left" });
  return (
    <aside
      className={`thread-drawer${size.dragging ? " resizing" : ""}`}
      style={{ "--td-w": `${size.width}px` } as React.CSSProperties}
      ref={root}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          back();
        }
      }}
    >
      <div className="td-resize" onPointerDown={size.onPointerDown} onDoubleClick={size.reset} title={tr("outputs.resize")} role="separator" aria-orientation="vertical" />
      {ui.view.kind === "list" && <OutputList panel={panel} ws={ws} byId={byId} ui={ui} />}
      {ui.view.kind === "new" && <NewThread panel={panel} msg={ui.view.msg} quote={ui.view.quote} who={byId.get(ws.messages.find((m) => m.id === (ui.view as { msg: string }).msg)?.from ?? "")} />}
      {ui.view.kind === "poll" &&
        (() => {
          const id = ui.view.id;
          const p = (ws.polls ?? []).find((x) => x.id === id);
          return p ? <PollView panel={panel} p={p} byId={byId} /> : <div className="td-empty">{tr("poll.notFound")}</div>;
        })()}
      {ui.view.kind === "thread" &&
        (() => {
          const id = ui.view.id;
          const t = threads.find((x) => x.id === id);
          return t ? <ThreadView panel={panel} t={t} byId={byId} /> : <div className="td-empty">{tr("thread.notFound")}</div>;
        })()}
    </aside>
  );
}

const KINDS: { id: ArtifactFilter; label: string }[] = [
  { id: "all", get label() { return tr("outputs.all"); } },
  { id: "thread", get label() { return tr("outputs.threads"); } },
  { id: "doc", get label() { return tr("outputs.docs"); } },
  { id: "diagram", get label() { return tr("outputs.diagrams"); } },
  { id: "page", get label() { return tr("outputs.pages"); } },
  { id: "poll", get label() { return tr("outputs.polls"); } },
];
const ICON = { thread: "💬", doc: "📄", diagram: "📐", page: "🌐" };
const KIND_WORD = {
  get doc() { return tr("kind.doc"); },
  get page() { return tr("kind.page"); },
  get diagram() { return tr("kind.diagram"); },
};

/** `@claude` → agent, `#n` → threads only, anything else → fuzzy over the title, address and kind. */
function artifactMatches(a: Artifact, query: string, alias: string) {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((tok) => {
      if (/^#\d+$/.test(tok)) return false;
      if (tok.startsWith("@")) return alias.toLowerCase().includes(tok.slice(1));
      return fuzzy(tok, `${KIND_WORD[a.kind]} ${a.title} ${a.target} ${alias}`.toLowerCase());
    });
}

function host(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

type Item = { kind: "thread"; at: number; t: Thread } | { kind: Artifact["kind"]; at: number; a: Artifact } | { kind: "poll"; at: number; p: Poll };

/** `#n` → poll number, `@x` → nobody (polls have no author), else fuzzy over 投票, question, options. */
function pollMatches(p: Poll, query: string) {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((tok) => {
      if (/^#\d+$/.test(tok)) return p.num === Number(tok.slice(1));
      if (tok.startsWith("@")) return false;
      return fuzzy(tok, `${tr("outputs.polls")} ${p.question} ${p.options.join(" ")}`.toLowerCase());
    });
}

/** The drawer's list: threads, documents and pages of this chat, newest first. */
function OutputList({ panel, ws, byId, ui }: { panel: string; ws: Workspace; byId: Map<string, Member>; ui: ThreadsUI }) {
  // Things taken off the list stay in the chat, just not here.
  const threads = useMemo(() => (ws.threads ?? []).filter((t) => !t.removed), [ws.threads]);
  const artifacts = useMemo(() => (ws.artifacts ?? []).filter((a) => !a.removed), [ws.artifacts]);
  const polls = useMemo(() => (ws.polls ?? []).filter((p) => !p.removed), [ws.polls]);
  const anchor = ui.msg ? ws.messages.find((m) => m.id === ui.msg) : undefined;
  // A message's 💬 badge asks for its threads only.
  const filter = anchor ? "thread" : ui.filter;
  const count = (k: ArtifactFilter) =>
    k === "all" ? threads.length + artifacts.length + polls.length : k === "thread" ? threads.length : k === "poll" ? polls.length : artifacts.filter((a) => a.kind === k).length;
  const shown = useMemo(() => {
    const items: Item[] = [];
    if (filter === "all" || filter === "thread")
      for (const t of threads) if ((!ui.msg || t.msg === ui.msg) && matches(t, ui.query, byId.get(t.agent)?.alias ?? "")) items.push({ kind: "thread", at: t.updated, t });
    if (!anchor)
      for (const a of artifacts)
        if ((filter === "all" || filter === a.kind) && artifactMatches(a, ui.query, byId.get(a.by)?.alias ?? "")) items.push({ kind: a.kind, at: a.updated, a });
    if (!anchor && (filter === "all" || filter === "poll"))
      for (const p of polls) if (pollMatches(p, ui.query)) items.push({ kind: "poll", at: p.closed ?? p.created, p });
    return items.sort((x, y) => y.at - x.at);
  }, [threads, artifacts, polls, filter, anchor, ui.msg, ui.query, byId]);
  const empty = {
    all: tr("outputs.emptyAll"),
    thread: tr("outputs.emptyThreads"),
    doc: tr("outputs.emptyDocs"),
    diagram: tr("outputs.emptyDiagrams"),
    page: tr("outputs.emptyPages"),
    poll: tr("outputs.emptyPolls"),
  }[filter];
  return (
    <>
      <header className="td-head">
        <b>{tr("outputs.title")}</b>
        <span className="dim">{count("all")}</span>
        <button className="td-x" onClick={() => closeThreads(panel)} aria-label={tr("outputs.close")} title={tr("common.close") + "（Esc）"}>
          ✕
        </button>
      </header>
      {!anchor && (
        <div className="td-tabs" role="tablist">
          {KINDS.map((k) => (
            <button key={k.id} role="tab" aria-selected={filter === k.id} className={filter === k.id ? "on" : ""} onClick={() => setThreadSearch(panel, { filter: k.id })}>
              {k.label}
              {k.id !== "all" && count(k.id) > 0 && <span className="dim"> {count(k.id)}</span>}
            </button>
          ))}
        </div>
      )}
      <div className="td-search">
        {anchor && (
          <span className="td-chip" title={tr("outputs.anchorHint")}>
            {tr("outputs.anchor", { time: time(anchor.ts), alias: byId.get(anchor.from)?.alias ?? "?" })}
            <button onClick={() => setThreadSearch(panel, { msg: null })} aria-label={tr("outputs.clearFilter")}>
              ✕
            </button>
          </span>
        )}
        <input
          autoFocus
          value={ui.query}
          onChange={(e) => setThreadSearch(panel, { query: e.target.value })}
          placeholder={anchor ? tr("outputs.searchThese") : tr("outputs.search")}
          spellCheck={false}
        />
      </div>
      <div className="td-list">
        {shown.map((it) =>
          it.kind === "thread" ? (
            <ThreadRow key={it.t.id} panel={panel} t={it.t} who={byId.get(it.t.agent)} />
          ) : it.kind === "poll" ? (
            <PollRow key={it.p.id} panel={panel} p={it.p} />
          ) : (
            <ArtifactRow key={it.a.id} panel={panel} a={it.a} who={byId.get(it.a.by)} />
          ),
        )}
        {shown.length === 0 && <div className="td-empty">{threads.length + artifacts.length + polls.length > 0 && ui.query ? tr("outputs.noMatch") : empty}</div>}
      </div>
      {anchor && (
        <button className="td-new" onClick={() => setThreadView(panel, { kind: "new", msg: anchor.id, quote: "" })}>
          {tr("thread.newForReply")}
        </button>
      )}
    </>
  );
}

function ThreadRow({ panel, t, who }: { panel: string; t: Thread; who?: Member }) {
  // What the human asked about the quote: says what the thread is about better than the quote alone.
  const ask = t.messages.find((m) => m.from === "you")?.text.replace(/\s+/g, " ").trim();
  return (
    <div className={`td-row st-${t.status}`}>
      <button className="td-open" onClick={() => setThreadView(panel, { kind: "thread", id: t.id })}>
        <span className="td-num">
          <span className="td-icon">{ICON.thread}</span>#{t.num}
        </span>
        <span className="td-main">
          <span className="td-quote">
            <Kind whole={!!t.whole} />「{t.quote.replace(/\s+/g, " ")}」
          </span>
          {ask && (
            <span className="td-ask" title={ask}>
              <b>{tr("thread.youAsked")}</b>
              {ask}
            </span>
          )}
          <span className="td-meta">
            @{who?.alias ?? tr("agent.gone")} · {tr("thread.messages", { n: t.messages.filter((m) => m.from !== "system").length })} · <i className="td-dot" />
            {STATUS[t.status]}
          </span>
        </span>
      </button>
      <button className="td-goto" onClick={() => showInChat(t.msg)} title={tr("thread.findOriginal")} aria-label={tr("thread.originalOf", { num: t.num })}>
        ↖
      </button>
      <RemoveButton panel={panel} id={t.id} name={tr("thread.name", { num: t.num })} />
    </div>
  );
}

/** Takes an item off the 產出 list; the chat keeps showing it. */
export function RemoveButton({ panel, id, name }: { panel: string; id: string; name: string }) {
  return (
    <button className="td-goto td-remove" onClick={() => send({ t: "output_remove", panel, id })} title={tr("outputs.remove")} aria-label={tr("outputs.removeNamed", { name })}>
      ✕
    </button>
  );
}

function ArtifactRow({ panel, a, who }: { panel: string; a: Artifact; who?: Member }) {
  const title = a.title || (a.kind === "page" ? host(a.target) : tr("doc.untitled"));
  return (
    <div className="td-row">
      <button
        className="td-open"
        // The drawer hides the chat's splits, where it opens: show them again, and keep the list.
        onClick={() => (send({ t: "artifact_open", panel, artifact: a.id }), unzoom())}
        title={a.kind === "page" ? tr("outputs.openPage", { alias: who?.alias ?? "agent", url: a.target }) : tr("outputs.openDoc")}
      >
        <span className="td-num">
          <span className="td-icon">{ICON[a.kind]}</span>
        </span>
        <span className="td-main">
          <span className="td-quote">{title}</span>
          <span className="td-meta">
            @{who?.alias ?? tr("agent.gone")} · {a.kind === "page" ? `${host(a.target)} · ` : ""}
            {time(a.updated)}
          </span>
        </span>
      </button>
      <RemoveButton panel={panel} id={a.id} name={title} />
    </div>
  );
}

/** Whether a thread is about the whole reply or a selected part of it. */
function Kind({ whole }: { whole: boolean }) {
  return (
    <span className={`td-kind ${whole ? "whole" : "part"}`} title={whole ? tr("thread.wholeHint") : tr("thread.partHint")}>
      {whole ? tr("thread.whole") : tr("thread.part")}
    </span>
  );
}

function Quote({ text, msg, whole }: { text: string; msg: string; whole: boolean }) {
  return (
    <>
      <span className="td-quote-label">
        <Kind whole={whole} />
      </span>
      <button className="td-quote-block" onClick={() => showInChat(msg)} title={tr("thread.showInChat")}>
        {text}
      </button>
    </>
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
      <button className="td-send" onClick={submit} disabled={disabled || !text.trim()} aria-label={tr("keys.send")}>
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
          ← {tr("common.back")}
        </button>
        <b>{tr("thread.new")}</b>
      </header>
      <div className="td-body">
        <p className="td-hint">{tr("thread.newHint", { alias: who?.alias ?? "agent" })}</p>
        {quote ? <Quote text={quote} msg={msg} whole={false} /> : <span className="td-quote-label"><Kind whole /></span>}
      </div>
      <Composer
        placeholder={sent ? tr("thread.opening") : tr("thread.askPlaceholder", { alias: who?.alias ?? "agent" })}
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
          ← {tr("common.back")}
        </button>
        <b>#{t.num}</b>
        <span className="dim">
          @{who?.alias ?? tr("agent.gone")} · {STATUS[t.status]}
        </span>
        <button className="td-x" onClick={() => closeThreads(panel)} aria-label={tr("thread.close")} title={tr("drawer.closeHint")}>
          ✕
        </button>
      </header>
      <div className="td-body" ref={box}>
        <Quote text={t.quote} msg={t.msg} whole={!!t.whole} />
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
            <span className="spin">✳</span> {tr("thread.agentReplying", { alias: who?.alias ?? "agent" })}
          </div>
        )}
      </div>
      <Composer placeholder={busy ? tr("thread.waitAgent") : tr("thread.replyPlaceholder", { alias: who?.alias ?? "agent" })} disabled={busy} onSend={(text) => send({ t: "thread_send", panel, thread: t.id, text })} />
      <div className="td-foot">
        <button className="td-bring" disabled={busy || !t.fork} onClick={() => send({ t: "thread_bring", panel, thread: t.id })} title={tr("thread.bringHint")}>
          ↩ {tr("thread.bring")}
        </button>
      </div>
    </>
  );
}

/** Double-click on a reply: its whole-message thread if there is one, else start one. */
export function openWholeThread(panel: string, msg: string, threads: Thread[]) {
  const t = threads.filter((x) => x.msg === msg && x.whole).sort((a, b) => b.updated - a.updated)[0];
  if (t) openThreads(panel, { view: { kind: "thread", id: t.id }, msg: null, query: "" });
  else openThreads(panel, { view: { kind: "new", msg, quote: "" }, msg });
}

/** Open the drawer from a message's 💬: its threads if it has some, else a new one. */
export function openThreadsFor(panel: string, msg: string, count: number, selection: string) {
  if (count > 0 && !selection) openThreads(panel, { view: { kind: "list" }, msg, query: "" });
  else openThreads(panel, { view: { kind: "new", msg, quote: selection }, msg });
}
