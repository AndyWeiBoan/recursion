// Group-chat panel. Chat layout: your messages on the right, agents on the left with avatar + name.
import { ChatSettingsForm } from "./ScopeSettings";
import { openSettings } from "./Settings";
import { accountLabel } from "./Setup";
import { FileChip, FilePicker, isImagePath } from "./Files";
import { useDragWidth } from "./Layout";
import { BrandMark, Icon, type IconName } from "./icons";
import { apply, codeBlock, continueList, inCodeBlock, prefix, wrap, type Edit } from "./format";
import { t as tr } from "./i18n";
import { copyText, readClipboardImage } from "./clipboard";
import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/common";
import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { openThreadsFor, openWholeThread, showInChat, ThreadDrawer } from "./ThreadDrawer";
import { outcome, PollCard, PollForm } from "./Polls";
import { closeThreads, openThreads, useThreadsUI } from "./threadsUI";
import { accountName, isMac, onChatRequest, toast, onReconnect, openExternal, grantComputerFiles, pasteImage, send, useAccounts, useModels, useRoster, useSnapshot, useWorkspace } from "./client";
import { takeDrops } from "./drop";
import { RefChip, takeRefs } from "./refs";
import { chatsOf, leaves, PERM_MODES, type Account, type AgentDefaults, type AgentKind, type PermMode, type DocVia, type Member, type Msg, type OutputRef, type Poll, type Round, type Thread, type TurnUsage } from "./types";
import { markSeen } from "./seen";

// Code blocks get colours: the fence's language when it names one, otherwise a confident guess.
// Its own instance: the shared `marked` would stack the extension on every hot reload.
const marked = new Marked(
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code, lang) {
      const name = lang.split(/[\s{]/)[0];
      if (name && hljs.getLanguage(name)) return hljs.highlight(code, { language: name, ignoreIllegals: true }).value;
      // A weak guess colours plain text (diagrams, logs) at random: leave those plain.
      const guess = hljs.highlightAuto(code);
      return guess.relevance >= 5 ? guess.value : hljs.highlight(code, { language: "plaintext" }).value;
    },
  }),
  { gfm: true, breaks: false },
);

// One listener for every rendered code block's copy button (chat, threads, …); once, even when
// a dev reload runs this module again.
const w = window as { __codeCopy?: boolean };
if (!w.__codeCopy) {
  w.__codeCopy = true;
  document.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest?.(".code-copy");
    const pre = btn?.parentElement?.querySelector("pre");
    if (!btn || !pre) return;
    e.preventDefault();
    e.stopPropagation();
    void copyText(pre.textContent ?? "").then((ok) => {
      if (!ok) return toast(tr("code.copyFailed"));
      toast(tr("code.copied"), true);
      btn.classList.add("done");
      btn.textContent = tr("code.copiedShort");
      setTimeout(() => {
        btn.classList.remove("done");
        btn.textContent = tr("code.copy");
      }, 1500);
    });
  });
}

const AVATAR_COLORS = ["#e8714a", "#3fb8a8", "#6a8cff", "#c792ea", "#e0a83a", "#4ab36b", "#e05d8c", "#4aa3d9"];
export function colorFor(id: string) {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return AVATAR_COLORS[h % AVATAR_COLORS.length];
}

/** Markdown → sanitized HTML, with @alias mentions highlighted (outside tags and code) in the
 * colour of that agent's name. `mentions`: alias (lower case) → colour. */
// Rendering is the costly part of drawing a message (parse, sanitize, highlight); the same text
// always gives the same HTML.
const rendered = new Map<string, string>();
const RENDERED_MAX = 3000;

export function renderMarkdown(text: string, mentions: Record<string, string>): string {
  const key = Object.entries(mentions).flat().join(" ") + "\0" + text;
  let html = rendered.get(key);
  if (html === undefined) {
    html = renderUncached(text, mentions);
    if (rendered.size >= RENDERED_MAX) rendered.delete(rendered.keys().next().value!);
    rendered.set(key, html);
  }
  return html;
}

function renderUncached(text: string, mentions: Record<string, string>): string {
  let html = DOMPurify.sanitize(marked.parse(text, { async: false }) as string);
  // Every code block gets a copy button (handled by the click listener below).
  html = html.replace(/<pre>/g, `<div class="code-wrap"><button class="code-copy" type="button" title="${tr("code.copy")}">${tr("code.copy")}</button><pre>`).replace(/<\/pre>/g, "</pre></div>");
  if (Object.keys(mentions).length) {
    const known = new Set(Object.keys(mentions).concat(["all", "全部", "所有人"]));
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
        return part.replace(/(^|[^\w.])@([\w\-一-鿿]+)/g, (m, pre, name) => (known.has(name.toLowerCase()) ? `${pre}<span class="mention"${mentions[name.toLowerCase()] ? ` style="--mc:${mentions[name.toLowerCase()]}"` : ""}>@${name}</span>` : m));
      })
      .join("");
  }
  return html;
}

const KIND_NAME: Record<AgentKind, string> = { claude: "Claude", codex: "Codex" };

export const STATUS_LABEL: Record<Member["status"], string> = {
  get idle() { return tr("status.idle"); },
  get starting() { return tr("status.starting"); },
  get thinking() { return tr("status.thinking"); },
  get waiting_permission() { return tr("status.waitingPermission"); },
  get compacting() { return tr("status.compacting"); },
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
  const detail =
    tr("usage.turn", { input: u.input.toLocaleString(), cached: u.cached.toLocaleString(), output: u.output.toLocaleString() }) +
    (u.cost_usd === undefined ? "" : tr("usage.turnCost", { cost: u.cost_usd.toFixed(4) }));
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
      <button className="doc-sub-head" onClick={() => send({ t: "doc_open", panel, doc: via.doc })} title={tr("doc.openThis")}>
        <span className="m-doc-ic" aria-hidden="true">
          <Icon name="doc" />
        </span>
        <span>
          {tr("doc.sentFromBefore")}<b>「{via.title}」</b>{tr("doc.sentFromAfter")}
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
  // Agents that left stay findable by id, so their old messages, votes and threads keep their names.
  const byId = useMemo(() => {
    const gone = Object.entries(ws?.former ?? {}).map(([id, f]): [string, Member] => [id, { id, kind: f.kind, alias: f.alias, cost_usd: 0, status: "idle", usage: {} }]);
    return new Map([...gone, ...members.map((m): [string, Member] => [m.id, m])]);
  }, [members, ws?.former]);
  const busy = members.filter((m) => m.status !== "idle");
  // On screen (only the window shown is drawn) and the app in front: what is here has been seen.
  const newest = ws?.messages.length ? ws.messages[ws.messages.length - 1].seq : 0;
  useEffect(() => {
    if (!newest) return;
    const mark = () => !document.hidden && markSeen(panel, newest);
    mark();
    document.addEventListener("visibilitychange", mark);
    return () => document.removeEventListener("visibilitychange", mark);
  }, [panel, newest]);
  const ui = useThreadsUI(panel);
  const threadCounts = useMemo(() => {
    const c = new Map<string, number>();
    for (const t of ws?.threads ?? []) if (!t.removed) c.set(t.msg, (c.get(t.msg) ?? 0) + 1);
    return c;
  }, [ws?.threads]);

  if (!ws) return <div className="ws"><div className="ws-log" /></div>;

  return (
    <div className="ws-shell">
      <div className="ws">
        {members.length === 0 && ws.messages.length === 0 ? (
          <div className="ws-log ws-center">
            <AddAgent panel={panel} big />
          </div>
        ) : (
          <Transcript panel={panel} messages={ws.messages} byId={byId} threadCounts={threadCounts} threads={ws.threads ?? []} polls={ws.polls ?? []} waiting={ws.round?.waiting} />
        )}
        <Composer panel={panel} members={members} messages={ws.messages} disabled={members.length === 0} busy={busy.length > 0} working={busy} focused={focused && !ui.open} round={ws.round} />
      </div>
      {ui.open && <ThreadDrawer panel={panel} ws={ws} byId={byId} ui={ui} />}
    </div>
  );
}

/** The model an agent runs, short: the one picked, else what its CLI reported running (its
 * default), without the maker's prefix and date ("claude-opus-4-1-20250805" → "opus-4-1"). The
 * mark beside it already says whose. */
export function modelLabel(m: Member) {
  const raw = m.model ?? m.running_model;
  return raw ? raw.replace(/^claude-/, "").replace(/-\d{8}$/, "").replace(/\[1m\]$/, "") : tr("agent.defaultModelShort");
}

/** An agent: its maker's mark (Claude's, or OpenAI's for Codex). Its @name says which one. */
/** `running`: it is working on this (its mark moves; see BrandMark). */
export function Avatar({ m, size = 26, running = false }: { m?: Member; size?: number; running?: boolean }) {
  return <span className="avatar" style={{ width: size, height: size }}>{m ? <BrandMark kind={m.kind} size={size} running={running} /> : "?"}</span>;
}

/** Working: answering, starting up or compacting (not waiting for the human, not idle). */
export const isRunning = (m?: Member) => m?.status === "thinking" || m?.status === "starting" || m?.status === "compacting";

/** Centered dialog over the whole app. Closes on ✕, Esc, or a click outside. */
/** `escape: false` keeps Esc for what is inside (a terminal), and a click outside does not close it. */
export function Modal({ title, onClose, children, escape = true }: { title: string; onClose: () => void; children: React.ReactNode; escape?: boolean }) {
  const close = useRef(onClose);
  close.current = onClose;
  const esc = useRef(escape);
  esc.current = escape;
  useEffect(() => {
    const restore = overlay(); // native browser panels would draw on top of it
    const onKey = (e: KeyboardEvent) => {
      // A card floating above it (a todo's, a guidance file's) or an import's confirmation takes Esc first.
      if (e.key === "Escape" && esc.current && !document.querySelector(".todo-card, .lib-confirm-scrim")) {
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
    <div className="modal-scrim" onMouseDown={(e) => escape && e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-label={title}>
        <button className="modal-x" onClick={onClose} aria-label={tr("common.close")} title={tr("common.close") + "（Esc）"}>
          <Icon name="close" />
        </button>
        {children}
      </div>
    </div>,
    document.body,
  );
}

/** The account an agent runs under, if that account needs logging in again. */
function expiredAccount(m: Member, accounts: Account[]) {
  const a = accounts.find((x) => x.id === (m.account ?? `${m.kind}-default`));
  return a?.expired ? a : undefined;
}

/** The chat's part of its panel header: who is in it, add an agent, the 產出 drawer, what it cost. */
export function ChatHead({ panel }: { panel: string }) {
  const ws = useWorkspace(panel);
  const ui = useThreadsUI(panel);
  const accounts = useAccounts();
  const [adding, setAdding] = useState(false);
  const [setting, setSetting] = useState(false);
  const members = ws?.members ?? [];
  // The empty chat shows the big form instead; otherwise "add" must always be reachable,
  // including when every agent was removed from a chat that has history.
  const empty = !ws || (members.length === 0 && ws.messages.length === 0);
  // Asked from the sidebar: an empty chat already shows the add-agent form, so a second one must
  // not be left waiting to pop up once the first agent is in.
  const emptyNow = useRef(empty);
  emptyNow.current = empty;
  useEffect(() => onChatRequest(panel, (what) => (what === "settings" ? setSetting(true) : !emptyNow.current && setAdding(true))), [panel]);
  const settings = setting && (
    <Modal title={tr("scope.chat")} onClose={() => setSetting(false)}>
      <ChatSettingsForm panel={panel} onDone={() => setSetting(false)} />
    </Modal>
  );
  if (empty) return <span className="title">{settings}</span>;
  // As the 產出 list counts them: no pages the agents opened, and the pictures the human attached.
  const pictures = ws.messages.reduce((n, m) => n + (m.from === "you" ? (m.files ?? []).filter(isImagePath).length : 0), 0);
  const outputs = pictures + [ws.threads, (ws.artifacts ?? []).filter((a) => a.kind !== "page"), ws.polls].reduce((n, l) => n + (l ?? []).filter((x) => !x.removed).length, 0);
  return (
    <span className="title chat-head">
      <span className="chat-faces">
        {members.map((m) => {
          const gone = expiredAccount(m, accounts);
          return gone ? (
            <button
              key={m.id}
              className="chat-face expired"
              title={tr("acct.expiredMember", { alias: m.alias, name: gone.name })}
              onClick={() => send({ t: "account_login", account: gone.id })}
            >
              <Avatar m={m} size={18} />
              <span className="chat-face-key" aria-hidden="true">
                <Icon name="key" />
              </span>
            </button>
          ) : (
            <span key={m.id} className={`chat-face st-${m.status}`} title={`@${m.alias} · ${modelLabel(m)}`}>
              <Avatar m={m} size={18} running={isRunning(m)} />
            </span>
          );
        })}
        <button className="chat-add" onClick={() => setAdding((v) => !v)} title={tr("agent.new")} aria-label={tr("agent.new")}>
          <Icon name="add" />
        </button>
      </span>
      {adding && (
        <Modal title={tr("agent.new")} onClose={() => setAdding(false)}>
          <AddAgent panel={panel} onDone={() => setAdding(false)} />
        </Modal>
      )}
      <button
        className={`chat-outputs icon-only${ui.open ? " on" : ""}`}
        onClick={() => (ui.open ? closeThreads(panel) : openThreads(panel, { view: { kind: "list" }, msg: null, query: "" }))}
        title={tr("outputs.buttonHint")}
        aria-label={tr("outputs.title")}
      >
        <Icon name="outputs" />
        {outputs > 0 && <span className="chat-outputs-n">{outputs}</span>}
      </button>
      <button className={`chat-gear${ws.settings?.rules ? " has-rules" : ""}`} onClick={() => setSetting(true)} title={tr("scope.chat")} aria-label={tr("scope.chat")}>
        <Icon name="settings" />
      </button>
      {settings}
    </span>
  );
}

const KIND_INFO: Record<AgentKind, { vendor: string; hint: string }> = {
  claude: { vendor: "Anthropic", get hint() { return tr("agent.kindHint", { cli: "claude" }); } },
  codex: { vendor: "OpenAI", get hint() { return tr("agent.kindHint", { cli: "codex" }); } },
};

// The add-agent form starts from what was picked the last time (kind, model per kind, mode).
const LAST_AGENT = "recursion.lastAgent";
function lastAgent(): AgentDefaults {
  try {
    return JSON.parse(localStorage.getItem(LAST_AGENT) ?? "{}") ?? {};
  } catch {
    return {};
  }
}
function rememberAgent(kind: AgentKind, model: string, mode: PermMode) {
  const d = { ...lastAgent(), kind, mode, [kind === "claude" ? "claude_model" : "codex_model"]: model || undefined };
  try {
    localStorage.setItem(LAST_AGENT, JSON.stringify(d));
  } catch {}
}

function AddAgent({ panel, big, onDone }: { panel: string; big?: boolean; onDone?: () => void }) {
  // Any chat of this session can be the target; the one it was opened from is the default.
  const { state } = useSnapshot();
  const [defaults] = useState(lastAgent);
  const defaultModel = (k: AgentKind) => (k === "claude" ? defaults.claude_model : defaults.codex_model) ?? "";
  const [kind, setKind] = useState<AgentKind>(defaults.kind ?? "claude");
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
  const [alias, setAlias] = useState(free(kind));
  const [touched, setTouched] = useState(false);
  // The member list may arrive after the form opens: keep the suggested alias free of clashes.
  const takenKey = (roster[target] ?? []).map((m) => m.alias).join(",");
  useEffect(() => {
    if (!touched) setAlias(free(kind));
  }, [takenKey]);
  const [model, setModel] = useState(defaultModel(kind));
  const [mode, setMode] = useState<PermMode>(defaults.mode ?? "edits");
  const allAccounts = useAccounts();
  const accounts = allAccounts.filter((a) => a.kind === kind);
  // A kind with no account in Recursion can't be added: its card is greyed out, and the form
  // starts on the other kind if that one has an account.
  const usable = (k: AgentKind) => allAccounts.some((a) => a.kind === k);
  useEffect(() => {
    const other: AgentKind = kind === "claude" ? "codex" : "claude";
    if (allAccounts.length && !usable(kind) && usable(other)) {
      setKind(other);
      setModel(defaultModel(other));
      if (!touched) setAlias(free(other));
    }
  }, [allAccounts.length]);
  const [account, setAccount] = useState("");
  const pickedAccount = accounts.find((a) => a.id === account) ?? accounts.find((a) => a.logged_in) ?? accounts[0];
  const list = useModels(kind);
  const options = list.models.length ? list.models : [{ value: "", label: tr("agent.defaultModel"), description: "" }];
  const picked = options.find((m) => m.value === model);
  const id = `${panel}${big ? "-big" : "-pop"}`;
  return (
    <form
      className={`add-card${big ? " big" : ""}`}
      onSubmit={(e) => {
        e.preventDefault();
        if (!pickedAccount) return;
        send({ t: "ws_add_agent", panel: target, kind, alias, model: model || null, account: pickedAccount.id, mode });
        rememberAgent(kind, model, mode);
        if (target !== panel) send({ t: "panel_focus", panel: target });
        onDone?.();
      }}
    >
      <div className="add-head">
        <h3>{big ? tr("chat.noMembers") : tr("agent.new")}</h3>
        {big && <p>{tr("chat.noMembersHint")}</p>}
      </div>
      {chats.length > 1 && (
        <div className="field">
          <label htmlFor={`chat-${id}`}>{tr("agent.whichChat")}</label>
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
                  {(roster[c.panel] ?? []).length ? `：${roster[c.panel].map((m) => `@${m.alias}`).join(" ")}` : tr("chat.nobodyYet")}
                </option>
              ))}
            </select>
            <span className="chev" aria-hidden="true">
              <Icon name="dropdown" size={14} />
            </span>
          </div>
        </div>
      )}
      <div className="kind-cards" role="radiogroup" aria-label={tr("agent.kind")}>
        {(["claude", "codex"] as AgentKind[]).map((k) => (
          <button
            key={k}
            type="button"
            role="radio"
            aria-checked={kind === k}
            className={`kind-card${kind === k ? " on" : ""}`}
            disabled={!usable(k)}
            title={usable(k) ? undefined : tr("agent.noAccountOfKind", { name: KIND_NAME[k] })}
            onClick={() => {
              setKind(k);
              setModel(defaultModel(k));
              setAccount("");
              if (!touched) setAlias(free(k));
            }}
          >
            <BrandMark kind={k} size={30} />
            <span className="kind-text">
              <b>{KIND_NAME[k]}</b>
              <span>{usable(k) ? KIND_INFO[k].vendor : tr("agent.noAccount")}</span>
            </span>
            <span className="kind-check" aria-hidden="true">
              <Icon name="check" />
            </span>
          </button>
        ))}
      </div>
      <div className="field">
        <label htmlFor={`alias-${id}`}>{tr("agent.alias")}</label>
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
      {accounts.length === 0 && (
        <div className="field">
          <span className="field-hint warn">
            {tr("acct.noneOfKind", { name: kind === "claude" ? "Claude" : "Codex" })}{" "}
            <button type="button" className="link" onClick={() => openSettings("accounts")}>
              {tr("acct.addOne")}
            </button>
          </span>
        </div>
      )}
      {accounts.length > 1 && (
        <div className="field">
          <label htmlFor={`account-${id}`}>{tr("settings.accounts")}</label>
          <div className="select-wrap">
            <select id={`account-${id}`} value={pickedAccount?.id ?? ""} onChange={(e) => setAccount(e.target.value)}>
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {accountLabel(a)}
                  {a.email && a.email !== accountLabel(a) ? ` · ${a.email}` : ""}
                  {!a.logged_in ? tr("acct.notLoggedInParen") : a.expired ? tr("acct.expiredParen") : ""}
                </option>
              ))}
            </select>
            <span className="chev" aria-hidden="true">
              <Icon name="dropdown" size={14} />
            </span>
          </div>
          {pickedAccount && !pickedAccount.logged_in && <span className="field-hint warn">{tr("acct.loginFirst")}</span>}
          {pickedAccount?.logged_in && pickedAccount.expired && <span className="field-hint warn">{tr("acct.expired")}</span>}
        </div>
      )}
      <div className="field">
        <label htmlFor={`model-${id}`}>{tr("agent.model")}</label>
        <div className="select-wrap">
          <select id={`model-${id}`} value={model} onChange={(e) => setModel(e.target.value)}>
            {options.map((m) => (
              <option key={m.value} value={m.value} title={m.description}>
                {m.label}
              </option>
            ))}
          </select>
          <span className="chev" aria-hidden="true">
            <Icon name="dropdown" size={14} />
          </span>
        </div>
        <span className="field-hint">
          {list.loading ? tr("agent.modelsLoading", { kind: KIND_NAME[kind] }) : list.error ? tr("agent.modelsError", { error: list.error }) : picked?.description}
        </span>
      </div>
      <div className="field">
        <span className="field-label" id={`mode-${id}`}>
          {tr("perm.title")}
        </span>
        <div className="mode-seg" role="radiogroup" aria-labelledby={`mode-${id}`}>
          {PERM_MODES.map((p) => (
            <button key={p.id} type="button" role="radio" aria-checked={mode === p.id} className={`${mode === p.id ? "on" : ""} m-${p.id}`} onClick={() => setMode(p.id)}>
              <Icon name={p.id} size={13} /> {p.label}
            </button>
          ))}
        </div>
        <span className={`field-hint${mode === "auto" ? " warn" : ""}`}>{PERM_MODES.find((p) => p.id === mode)?.hint}</span>
      </div>
      <button className="add-submit" type="submit" disabled={!alias.trim() || !pickedAccount}>
        {tr("agent.join", { alias: alias.trim() || "…" })}
      </button>
      <p className="add-hint">{KIND_INFO[kind].hint}</p>
    </form>
  );
}

/** The message a reply answers, quoted at the top of the reply's bubble; click to go to it. */
function ReplyQuote({ r, byId }: { r: NonNullable<Msg["reply_to"]>; byId: Map<string, Member> }) {
  return (
    <button className="reply-quote" onClick={() => showInChat(r.msg)} title={tr("reply.jumpToOriginal")}>
      <span className="reply-who">{authorName(r.from, byId)}</span>
      <span className="reply-text">{excerpt(r.text)}</span>
    </button>
  );
}

/** Everyone still working, on one line above the chat box (so scrolling up never hides it),
 *  grouped by what they are doing. */
function BusyLine({ busy }: { busy: Member[] }) {
  if (!busy.length) return null;
  return (
    <div className="ws-busy-row">
      <Icon name="busy" size={13} />
      <span className="ws-busy-faces">
        {busy.map((b) => (
          <Avatar key={b.id} m={b} size={18} />
        ))}
      </span>
      <span className="ws-busy">
        {[...new Set(busy.map((b) => b.status))].map((st, i) => (
          <span key={st}>
            {i > 0 && " · "}
            {busy
              .filter((b) => b.status === st)
              .map((b) => `@${b.alias}`)
              .join(tr("common.listSep"))}{" "}
            {STATUS_LABEL[st]}…
          </span>
        ))}
      </span>
      <span className="dim">{tr("chat.escStops")}</span>
    </div>
  );
}

/** Whether a document is a diagram (the chat card says so). */
function useIsDiagram(doc: string | null) {
  const { state } = useSnapshot();
  return !!doc && !!state?.sessions.some((s) => s.docs?.some((d) => d.id === doc && d.kind === "diagram"));
}

// The message each chat's composer is replying to. Outside React: the menu sets it, the composer shows it.
let replying: Record<string, string | null> = {};
const replySubs = new Set<() => void>();
function setReplying(panel: string, msg: string | null) {
  replying = { ...replying, [panel]: msg };
  replySubs.forEach((f) => f());
}
function useReplying(panel: string) {
  return useSyncExternalStore(
    (f) => (replySubs.add(f), () => replySubs.delete(f)),
    () => replying[panel] ?? null,
  );
}

// Rewind mode: the human's message the chat will start again from, per chat. Nothing is deleted
// until the new text is sent.
let rewinding: Record<string, string | null> = {};
const rewindSubs = new Set<() => void>();
function setRewinding(panel: string, msg: string | null) {
  rewinding = { ...rewinding, [panel]: msg };
  rewindSubs.forEach((f) => f());
}
function useRewinding(panel: string) {
  return useSyncExternalStore(
    (f) => (rewindSubs.add(f), () => rewindSubs.delete(f)),
    () => rewinding[panel] ?? null,
  );
}

/** Who wrote a message, as shown above a reply. */
const authorName = (from: string, byId: Map<string, Member>) => (from === "you" ? tr("common.you") : "@" + (byId.get(from)?.alias ?? tr("agent.gone")));
/** One line of a message's text for a reply preview. */
const excerpt = (text: string) => text.replace(/[#>*_`|\-]+/g, " ").replace(/\s+/g, " ").trim();

type CtxMenu = { x: number; y: number; msg: Msg; threadable: boolean; quote: string };

function Transcript({
  panel,
  messages,
  byId,
  threadCounts,
  threads,
  polls,
  waiting,
}: {
  panel: string;
  messages: Msg[];
  byId: Map<string, Member>;
  threadCounts: Map<string, number>;
  threads: Thread[];
  polls: Poll[];
  waiting?: Round["waiting"];
}) {
  const queued = useMemo(() => new Set((waiting ?? []).flatMap((w) => w.msgs)), [waiting]);
  const cut = useRewinding(panel);
  const pollById = useMemo(() => new Map(polls.map((p) => [p.id, p])), [polls]);
  const box = useRef<HTMLDivElement>(null);
  // Following the newest message while the view is at the bottom. Scrolled up, it stays put
  // (several agents writing at once would otherwise keep dragging it down) and a button jumps
  // back, counting what came in meanwhile.
  const stick = useRef(true);
  const [away, setAway] = useState<number | null>(null); // messages.length when it left the bottom
  const jumping = useRef(false);
  // @names in messages, in the colour of each agent's name.
  const aliases = useMemo(() => Object.fromEntries([...byId.values()].map((m) => [m.alias.toLowerCase(), colorFor(m.id)])), [byId]);
  const last = messages[messages.length - 1];

  // Sending something brings you to it.
  const sent = useRef(messages.length);
  if (messages.length !== sent.current) {
    if (messages.length > sent.current && last?.from === "you") stick.current = true;
    sent.current = messages.length;
  }
  // Any change (also a reply still being written further up) keeps the bottom in view.
  useLayoutEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
    if (stick.current && away !== null) setAway(null);
  });
  const toLatest = () => {
    const el = box.current;
    if (!el) return;
    stick.current = jumping.current = true;
    setAway(null);
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  };

  const [menu, setMenu] = useState<CtxMenu | null>(null);
  // One block per agent turn, even when other agents' messages land in between: its words in
  // one bubble, its tool calls as one line under it, then cards (documents, approvals, errors).
  const { turns, inTurn } = useMemo(() => {
    const turns = new Map<string, Msg[]>();
    const inTurn = new Set<string>();
    const open = new Map<string, string>();
    const own = new Set(["text", "tool", "doc", "permission", "error"]);
    for (const m of messages) {
      if (m.from === "you" || m.from === "system" || !own.has(m.kind)) continue;
      const cur = open.get(m.from);
      if (m.turn || !cur || (m.kind === "text" && m.from_thread)) {
        open.set(m.from, m.id);
        turns.set(m.id, [m]);
      } else {
        turns.get(cur)!.push(m);
        inTurn.add(m.id);
      }
    }
    return { turns, inTurn };
  }, [messages]);
  // New messages since leaving the bottom, as the log shows them (an agent's turn is one).
  const unseen = away === null ? 0 : messages.slice(away).filter((m) => m.from !== "you" && !inTurn.has(m.id)).length;
  // Text selected inside one agent reply → a "discuss this" button next to it.
  const [pick, setPick] = useState<{ x: number; y: number; msg: string; quote: string } | null>(null);
  const readSelection = (at?: { x: number; y: number }) => {
    const sel = window.getSelection();
    const bubbleOf = (n: Node | null) => (n instanceof Element ? n : n?.parentElement)?.closest<HTMLElement>("[data-threadable]") ?? null;
    const b = sel && !sel.isCollapsed ? bubbleOf(sel.anchorNode) : null;
    const quote = sel?.toString().trim() ?? "";
    if (!b || b !== bubbleOf(sel!.focusNode) || !box.current?.contains(b) || !quote) return setPick(null);
    // Mouse: just under where the button was let go. Keyboard: under the end the caret moved to.
    let p = at && { x: at.x + 4, y: at.y + 16 };
    if (!p) {
      const end = document.createRange();
      end.setStart(sel!.focusNode!, sel!.focusOffset);
      const r = end.getBoundingClientRect();
      const all = sel!.getRangeAt(0).getBoundingClientRect();
      p = r.height ? { x: r.left, y: r.bottom + 6 } : { x: all.left, y: all.bottom + 6 };
    }
    setPick({ ...p, msg: b.dataset.msg!, quote });
  };
  useEffect(() => {
    const onChange = () => {
      if (window.getSelection()?.isCollapsed) setPick(null);
    };
    document.addEventListener("selectionchange", onChange);
    return () => document.removeEventListener("selectionchange", onChange);
  }, []);

  // Each agent's latest turn: its avatar moves while the agent is still at it, older ones don't.
  const lastTurn = new Map<string, string>();
  for (const m of messages) if (m.turn) lastTurn.set(m.from, m.id);
  const view = (m: Msg, i: number) => {
    const turn = turns.get(m.id);
    if (turn) {
      const texts = turn.filter((x) => x.kind === "text");
      const tools = turn.filter((x) => x.kind === "tool");
      const rest = turn.filter((x) => x.kind !== "text" && x.kind !== "tool");
      const blocks: { m: Msg; parts?: Msg[]; group?: Msg[] }[] = [
        ...(texts.length ? [{ m: texts[0], parts: texts }] : []),
        ...(tools.length ? [{ m: tools[0], group: tools.length > 1 ? tools : undefined }] : []),
        ...rest.map((x) => ({ m: x })),
      ];
      const who = byId.get(m.from);
      return blocks.map((b, k) => (
        <MessageView key={b.m.id} panel={panel} m={b.m} who={who} byId={byId} first={k === 0} live={k === 0 && isRunning(who) && lastTurn.get(m.from) === m.id} head={turn[0]} aliases={aliases} threads={threadCounts.get(b.m.id) ?? 0} parts={b.parts} partThreads={threadCounts} group={b.group} />
      ));
    }
    const prev = messages[i - 1];
    const first = !prev || prev.from !== m.from || prev.kind === "system" || prev.kind === "guard" || !!m.turn;
    return (
      <MessageView key={m.id} panel={panel} m={m} who={byId.get(m.from)} byId={byId} first={first} aliases={aliases} threads={threadCounts.get(m.id) ?? 0} poll={m.kind === "poll" || m.kind === "poll_closed" ? pollById.get(m.poll) : undefined} queued={queued.has(m.id)} />
    );
  };
  return (
    <div className="ws-log-wrap">
    <div
      className="ws-log"
      ref={box}
      onMouseUp={(e) => {
        const at = { x: e.clientX, y: e.clientY };
        setTimeout(() => readSelection(at));
      }}
      onKeyUp={(e) => e.shiftKey && readSelection()}
      // Right-click a message: reply to it, discuss it (whole or the selected part), copy.
      onContextMenu={(e) => {
        const el = (e.target as HTMLElement).closest<HTMLElement>("[data-msg]");
        if (!el || (e.target as HTMLElement).closest("a, input, textarea")) return;
        const msg = messages.find((x) => x.id === el.dataset.msg);
        if (!msg || msg.kind !== "text") return;
        e.preventDefault();
        const sel = window.getSelection();
        const quote = sel && !sel.isCollapsed && el.contains(sel.anchorNode) && el.contains(sel.focusNode) ? sel.toString().trim() : "";
        setPick(null);
        setMenu({ x: e.clientX, y: e.clientY, msg, threadable: el.dataset.threadable !== undefined, quote });
      }}
      onScroll={(e) => {
        const el = e.currentTarget;
        const bottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        if (jumping.current) {
          if (bottom) jumping.current = false;
        } else {
          stick.current = bottom;
          if (bottom) setAway(null);
          else if (away === null) setAway(messages.length);
        }
        setPick(null);
        setMenu(null);
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
        if (inTurn.has(m.id)) return null;
        // Rewinding: this message and everything after it fade (see .rewind-cut).
        if (m.id === cut) return [<div key="rewind-cut" className="rewind-cut">{tr("rewind.cut")}</div>, view(m, i)];
        return view(m, i);
      })}
      {pick &&
        createPortal(
          <button
            className="sel-thread"
            style={{ left: pick.x, top: pick.y }}
            ref={(el) => {
              // Keep it fully on screen: its width is only known once it is drawn.
              if (!el) return;
              const w = el.offsetWidth, h = el.offsetHeight;
              el.style.left = `${Math.max(8, Math.min(pick.x, window.innerWidth - w - 8))}px`;
              el.style.top = `${Math.min(pick.y, window.innerHeight - h - 8)}px`;
            }}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              openThreadsFor(panel, pick.msg, 0, pick.quote);
              window.getSelection()?.removeAllRanges();
              setPick(null);
            }}
          >
            <Icon name="thread" size={14} /> {tr("menu.discussThis")}
          </button>,
          document.body,
        )}
      {menu && <MessageMenu panel={panel} menu={menu} threads={threads} onClose={() => setMenu(null)} />}
    </div>
      {away !== null && (
        <button className="to-latest" onClick={toLatest}>
          <Icon name="down" size={14} /> {unseen ? tr("chat.newMessages", { n: unseen }) : tr("chat.toLatest")}
        </button>
      )}
    </div>
  );
}

function MessageMenu({ panel, menu, threads, onClose }: { panel: string; menu: CtxMenu; threads: Thread[]; onClose: () => void }) {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const restore = overlay(); // native browser panels would draw on top of it
    const away = (e: Event) => {
      if (!(e.target as HTMLElement).closest?.(".ctx-menu")) close.current();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close.current();
      }
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
  const m = menu.msg;
  const text = m.kind === "text" ? m.text : "";
  const hasWhole = threads.some((t) => t.msg === m.id && t.whole);
  const act = (f: () => void) => () => (f(), onClose());
  return createPortal(
    <div
      className="menu ctx-menu"
      role="menu"
      style={{ left: menu.x, top: menu.y }}
      ref={(el) => {
        // Keep it fully on screen: its size is only known once it is drawn.
        if (!el) return;
        el.style.left = `${Math.max(8, Math.min(menu.x, window.innerWidth - el.offsetWidth - 8))}px`;
        el.style.top = `${Math.max(8, Math.min(menu.y, window.innerHeight - el.offsetHeight - 8))}px`;
      }}
      onContextMenu={(e) => e.preventDefault()}
    >
      <button role="menuitem" onClick={act(() => setReplying(panel, m.id))}>
        <Icon name="reply" size={14} /> {tr("menu.reply")}
      </button>
      {m.from === "you" && (
        <button role="menuitem" onClick={act(() => setRewinding(panel, m.id))}>
          <Icon name="rewind" size={14} /> {tr("menu.rewind")}
        </button>
      )}
      {menu.threadable && (
        <button role="menuitem" onClick={act(() => openWholeThread(panel, m.id, threads))}>
          <Icon name="thread" size={14} /> {hasWhole ? tr("menu.openWholeThread") : tr("menu.discussWhole")}
        </button>
      )}
      {menu.threadable && menu.quote && (
        <button role="menuitem" onClick={act(() => (openThreadsFor(panel, m.id, 0, menu.quote), window.getSelection()?.removeAllRanges()))}>
          <Icon name="thread" size={14} /> {tr("menu.discussSelection")}
        </button>
      )}
      <button role="menuitem" onClick={act(() => void copyText(menu.quote || text))}>
        <Icon name="copy" size={14} /> {menu.quote ? tr("menu.copySelection") : tr("menu.copyWhole")}
      </button>
    </div>,
    document.body,
  );
}

// A turn's parts come in a new array every time the chat changes; it is the same turn if its
// messages are the same objects (an unchanged message keeps its object).
function sameProps<P extends object>(a: P, b: P) {
  for (const k in a) {
    const x = a[k] as unknown, y = b[k] as unknown;
    if (x === y) continue;
    if (!Array.isArray(x) || !Array.isArray(y) || x.length !== y.length || x.some((v, i) => v !== y[i])) return false;
  }
  return true;
}

const MessageView = memo(function MessageView({
  panel,
  m,
  who,
  byId,
  first,
  aliases,
  threads,
  group,
  poll,
  parts,
  partThreads,
  head,
  queued,
  live = false,
}: {
  panel: string;
  m: Msg;
  who?: Member;
  byId: Map<string, Member>;
  first: boolean;
  aliases: Record<string, string>;
  /** Threads opened on this message. */
  threads: number;
  /** For the first of several tool calls in a row: all of them, shown as one line. */
  group?: Msg[];
  /** For a poll message: the poll. */
  poll?: Poll;
  /** An agent's whole turn in one bubble: its text messages, each a part of it. */
  parts?: Msg[];
  /** Threads per message, for the parts. */
  partThreads?: Map<string, number>;
  /** The turn's first message: the header shows its time and usage. */
  head?: Msg;
  /** A human message still waiting for busy agents. */
  queued?: boolean;
  /** The latest turn of an agent still at it: its avatar moves. */
  live?: boolean;
}) {
  const accounts = useAccounts();

  if (m.kind === "poll") return poll ? <div className="row poll-row" data-msg={m.id}><PollCard panel={panel} p={poll} byId={byId} /></div> : null;
  if (m.kind === "poll_closed")
    return poll ? (
      <button className="m-sys m-poll-closed" onClick={() => showInChat(poll.msg)} title={tr("poll.jumpToCard")}>
        <Icon name="poll" size={14} /> {tr("poll.endedLine", { num: poll.num, outcome: outcome(poll) })}
      </button>
    ) : null;

  if (m.from === "you" && m.kind === "text") {
    const to = m.to?.length ? m.to.map((id) => "@" + (byId.get(id)?.alias ?? "?")).join(" ") : tr("outputs.all");
    return (
      <div className="row you">
        <div className="bubble-col">
          {m.via ? (
            <DocSubmission via={m.via} panel={panel} />
          ) : (
            <div className="bubble you" data-msg={m.id}>
              {m.reply_to && <ReplyQuote r={m.reply_to} byId={byId} />}
              {m.text && <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text, aliases) }} />}
              {!!(m.files?.length || m.refs?.length) && (
                <div className="bubble-files">
                  {m.files?.map((f) => (
                    <FileChip key={f} panel={panel} path={f} />
                  ))}
                  {m.refs?.map((r) => (
                    <RefChip key={`${r.kind}/${r.id}/${r.item ?? ""}`} panel={panel} r={r} />
                  ))}
                </div>
              )}
            </div>
          )}
          <div className="meta">{queued ? <span className="queued">{tr("queue.queued")}</span> : <>→ {to} · {time(m.ts)}</>}</div>
        </div>
      </div>
    );
  }
  if (m.kind === "system") return <div className="m-sys">{m.text}</div>;
  if (m.kind === "relogin") {
    return (
      <div className={`m-guard m-relogin${m.resolved ? " done" : ""}`}>
        <span>
          <Icon name="key" size={14} /> {tr("acct.needsRelogin", { name: m.name })}
          <span className="dim">{m.resolved ? "　" + tr("acct.reloggedIn") : `　${m.text.slice(0, 120)}`}</span>
        </span>
        {!m.resolved && (
          <span className="acts">
            <button onClick={() => send({ t: "account_login", account: m.account })}>{tr("acct.relogin")}</button>
          </span>
        )}
      </div>
    );
  }

  if (m.kind === "guard") {
    return (
      <div className={`m-guard${m.resolved ? " done" : ""}`}>
        <span><Icon name="warn" size={14} /> {m.text}</span>
        {!m.resolved && (
          <span className="acts">
            <button onClick={() => send({ t: "ws_stop", panel })}>{tr("common.stop")}</button>
          </span>
        )}
      </div>
    );
  }

  const diagram = useIsDiagram(m.kind === "doc" ? m.doc : null);
  let body: React.ReactNode;
  switch (m.kind) {
    case "text":
      body = (
        <div className="bubble agent">
          {(parts ?? [m]).map((p) =>
            p.kind !== "text" ? null : <TextPart key={p.id} panel={panel} m={p} who={who} byId={byId} aliases={aliases} threads={partThreads?.get(p.id) ?? (p.id === m.id ? threads : 0)} />,
          )}
        </div>
      );
      break;
    case "tool":
      body = group && group.length > 1 ? <ToolGroup tools={group} /> : <ToolView m={m} />;
      break;
    case "permission":
      body = (
        <div className={`m-perm st-${m.state}`}>
          <div className="m-perm-head">
            {tr("perm.wantsBefore")}<b>{m.tool}</b>{tr("perm.wantsAfter")}
          </div>
          <code className="m-perm-sum">{m.summary || m.tool}</code>
          {m.detail && m.detail !== m.summary && (
            <details>
              <summary>{tr("perm.details")}</summary>
              <pre>{m.detail}</pre>
            </details>
          )}
          {m.state === "pending" ? (
            <div className="m-perm-acts">
              <button className="yes" onClick={() => send({ t: "ws_perm", request_id: m.request_id, decision: "allow" })}>
                {tr("perm.allow")}
              </button>
              <button onClick={() => send({ t: "ws_perm", request_id: m.request_id, decision: "always" })}>{tr("perm.always")}</button>
              <button className="no" onClick={() => send({ t: "ws_perm", request_id: m.request_id, decision: "deny" })}>
                {tr("perm.deny")}
              </button>
            </div>
          ) : (
            <div className="m-perm-state">
              <Icon name={m.state === "denied" ? "close" : "check"} size={13} /> {m.state === "allowed" ? tr("perm.allowed") : m.state === "always_allowed" ? tr("perm.alwaysAllowed") : tr("perm.denied")}
            </div>
          )}
        </div>
      );
      break;
    case "doc":
      body = (
        <button className="m-doc" onClick={() => send({ t: "doc_open", panel, doc: m.doc })} title={tr("doc.openBeside")}>
          <span className="m-doc-ic" aria-hidden="true">
            <Icon name={diagram ? "diagram" : "doc"} size={15} />
          </span>
          <span className="m-doc-text">
            <b>{m.title}</b>
            <span>
              {m.updated ? tr(diagram ? "doc.updatedDiagram" : "doc.updated") : tr(diagram ? "kind.diagram" : "kind.doc")} · {tr("doc.clickToOpen")}
            </span>
          </span>
        </button>
      );
      break;
    case "error":
      body = (
        <div className="m-err">
          <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.text, {}) }} />
        </div>
      );
      break;
  }
  return (
    <div className={`row agent${first ? " first" : ""}`}>
      {first ? <Avatar m={who} running={live} /> : <span className="avatar-space" />}
      <div className="agent-col">
        {first && (
          <div className="name">
            <b style={{ color: who ? colorFor(who.id) : undefined }}>@{who?.alias ?? "agent"}</b>
            {who && (
              <span>
                {modelLabel(who)}
                {accountName(accounts, who) && ` · ${accountName(accounts, who)}`}
              </span>
            )}
            <span>{time((head ?? m).ts)}</span>
            {(head ?? m).usage && <UsageTag u={(head ?? m).usage!} />}
          </div>
        )}
        {body}
      </div>
    </div>
  );
}, sameProps);

/** One text message inside an agent's turn bubble: right-click, threads and quotes work per part. */
function TextPart({ panel, m, who, byId, aliases, threads }: { panel: string; m: Extract<Msg, { kind: "text" }>; who?: Member; byId: Map<string, Member>; aliases: Record<string, string>; threads: number }) {
  return (
    <div className="bubble-part" data-msg={m.id} data-threadable={!m.streaming && who ? "" : undefined}>
      {m.from_thread && (
        <button className="from-thread" onClick={() => openThreads(panel, { view: { kind: "thread", id: m.from_thread!.thread }, msg: null })}>
          <Icon name="bring" size={13} /> {tr("thread.conclusionOf", { num: m.from_thread.num })}
        </button>
      )}
      {m.reply_to && <ReplyQuote r={m.reply_to} byId={byId} />}
      {/* While it streams, a reply's opening `↩#12` isn't turned into the quote yet: hide it. */}
      <div className="md" dangerouslySetInnerHTML={{ __html: renderMarkdown(m.streaming ? m.text.replace(/^\s*↩\s*#\d+\s*/, "") : m.text, aliases) + (m.streaming ? '<span class="caret"></span>' : "") }} />
      {!m.streaming && who && threads > 0 && (
        <button
          className="thread-btn has"
          title={tr("thread.countHint", { n: threads })}
          // Keep the text selection: it becomes the quote.
          onMouseDown={(e) => e.preventDefault()}
          onClick={(e) => {
            const sel = window.getSelection();
            const bubble = e.currentTarget.parentElement!;
            const quote = sel && !sel.isCollapsed && bubble.contains(sel.anchorNode) ? sel.toString().trim() : "";
            openThreadsFor(panel, m.id, threads, quote);
          }}
        >
          <Icon name="thread" size={13} /> {threads}
        </button>
      )}
    </div>
  );
}

type ToolMsg = Extract<Msg, { kind: "tool" }>;

function ToolView({ m }: { m: Msg }) {
  const [open, setOpen] = useState(false);
  if (m.kind !== "tool") return null;
  const firstLine = (m.result ?? "").split("\n").find((l) => l.trim()) ?? "";
  return (
    <div className={`m-tool${m.is_error ? " err" : ""}`}>
      <button className="m-tool-head" onClick={() => setOpen((v) => !v)} title={tr("tool.clickToExpand")}>
        <span className="tool-ic"><Icon name={open ? "dropdown" : "expand"} size={12} /></span>
        <b>{m.name}</b>
        {m.summary && <span className="m-tool-sum">{m.summary}</span>}
      </button>
      <div className="m-tool-res"><Icon name="result" size={12} /> {m.result == null ? tr("tool.running") : firstLine || tr("tool.noOutput")}</div>
      {open && (
        <div className="m-tool-detail">
          {m.detail && <pre>{m.detail}</pre>}
          {m.result && <pre className="res">{m.result}</pre>}
        </div>
      )}
    </div>
  );
}

/** Several tool calls in a row, as one line: what's running now, or how many and which. Click to list them. */
function ToolGroup({ tools }: { tools: Msg[] }) {
  const [open, setOpen] = useState(false);
  const ts = tools as ToolMsg[];
  const running = ts.find((t) => t.result == null);
  const failed = ts.filter((t) => t.is_error).length;
  const counts = new Map<string, number>();
  for (const t of ts) counts.set(t.name, (counts.get(t.name) ?? 0) + 1);
  const names = [...counts].map(([n, c]) => (c > 1 ? `${n} ×${c}` : n)).join("、");
  return (
    <div className={`m-tools${failed ? " err" : ""}${running ? " live" : ""}`}>
      <button className="m-tool-head" onClick={() => setOpen((v) => !v)} aria-expanded={open} title={open ? tr("common.collapse") : tr("tool.expandEach")}>
        <span className="tool-ic"><Icon name={open ? "dropdown" : "expand"} size={12} /></span>
        {running && !open ? (
          <>
            <Icon name="busy" size={12} />
            <b>{running.name}</b>
            {running.summary && <span className="m-tool-sum">{running.summary}</span>}
            <span className="m-tools-n">{tr("tool.nth", { n: ts.indexOf(running) + 1 })}</span>
          </>
        ) : (
          <span className="m-tools-sum">
            {tr("tool.used", { n: ts.length, names })}
            {failed > 0 && <span className="m-tools-fail">　{tr("tool.failed", { n: failed })}</span>}
          </span>
        )}
      </button>
      {open && (
        <div className="m-tools-list">
          {ts.map((t) => (
            <ToolView key={t.id} m={t} />
          ))}
        </div>
      )}
    </div>
  );
}

function Composer({
  panel,
  members,
  messages,
  disabled,
  busy,
  working,
  focused,
  round,
}: {
  panel: string;
  members: Member[];
  messages: Msg[];
  disabled: boolean;
  busy: boolean;
  /** Agents still answering, for the line above the box. */
  working: Member[];
  focused: boolean;
  round?: Round;
}) {
  const [text, setText] = useState("");
  const [sel, setSel] = useState(0);
  const [caret, setCaret] = useState(0);
  const [menuOff, setMenuOff] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  const replyId = useReplying(panel);
  const [polling, setPolling] = useState(false);
  // Project files this message references (📎), and whether the picker is open.
  const [files, setFiles] = useState<string[]>([]);
  const [picking, setPicking] = useState(false);
  // Files dragged in from the file manager are attached like ones picked from the computer.
  useEffect(() => {
    if (disabled) return;
    return takeDrops(panel, (paths) => {
      void grantComputerFiles(panel, paths).then((ok) => setFiles((f) => [...f, ...ok.filter((p) => !f.includes(p))]));
      ref.current?.focus();
    });
  }, [panel, disabled]);
  // Things from the 產出 drawer this message points at (dragged in, or added with ↳).
  const [refs, setRefs] = useState<OutputRef[]>([]);
  const sameRef = (a: OutputRef, b: OutputRef) => a.kind === b.kind && a.id === b.id && a.item === b.item;
  useEffect(() => {
    if (disabled) return;
    return takeRefs(panel, (p) => {
      if ("image" in p) setFiles((f) => (f.includes(p.image) ? f : [...f, p.image]));
      else setRefs((r) => (r.some((x) => sameRef(x, p.ref)) ? r : [...r, p.ref]));
      ref.current?.focus();
    });
  }, [panel, disabled]);
  const reply = replyId ? messages.find((m) => m.id === replyId) : undefined;
  // Rewind mode: the box holds the target's text; cancelling brings back the draft from before
  // the first Rewind, however many times the target changed since.
  const rewindId = useRewinding(panel);
  const rewindAt = rewindId ? messages.findIndex((m) => m.id === rewindId) : -1;
  const draft = useRef<{ text: string; files: string[]; refs: OutputRef[] } | null>(null);
  const shownRewind = useRef<string | null>(null);
  const fill = (t: string) => {
    setText(t);
    requestAnimationFrame(() => {
      const el = ref.current;
      if (!el) return;
      el.focus();
      el.selectionStart = el.selectionEnd = t.length;
      setCaret(t.length);
    });
  };
  const leaveRewind = (restore: boolean) => {
    const d = draft.current;
    draft.current = shownRewind.current = null;
    setRewinding(panel, null);
    if (restore && d) {
      setFiles(d.files);
      setRefs(d.refs);
      fill(d.text);
    }
  };
  useEffect(() => {
    if (!rewindId || rewindId === shownRewind.current) return;
    const m = messages[rewindAt];
    if (!m || m.kind !== "text") return;
    if (!shownRewind.current) draft.current = { text, files, refs };
    shownRewind.current = rewindId;
    setReplying(panel, null);
    setHist(null);
    setFiles(m.files ?? []);
    setRefs(m.refs ?? []);
    fill(m.text);
  }, [rewindId]);
  // The target went away meanwhile (rewound elsewhere, say).
  useEffect(() => {
    if (rewindId && rewindAt < 0 && shownRewind.current) {
      toast(tr("chat.err.rewindGone"));
      leaveRewind(true);
    }
  }, [rewindId, rewindAt]);
  const doomed = rewindAt < 0 ? 0 : messages.slice(rewindAt + 1).filter((m) => m.kind === "text").length;
  // ↑ / ↓ in an empty box walk through what the human sent in this chat, newest first.
  const sent = useMemo(() => {
    const out: string[] = [];
    for (const m of messages) if (m.from === "you" && m.kind === "text" && m.text.trim() && out[out.length - 1] !== m.text) out.push(m.text);
    return out;
  }, [messages]);
  const [hist, setHist] = useState<number | null>(null);
  const recall = (i: number | null) => {
    setHist(i);
    const t = i === null ? "" : sent[i];
    setText(t);
    requestAnimationFrame(() => {
      const el = ref.current;
      if (el) el.selectionStart = el.selectionEnd = t.length;
      setCaret(t.length);
    });
  };
  useEffect(() => {
    if ((focused || replyId) && !disabled) ref.current?.focus();
  }, [focused, disabled, replyId]);
  // The box grows with the text (up to 200px). Dragging its top edge sets a height of its own
  // instead; double-clicking the edge goes back to growing.
  const AUTO = 200;
  const tall = useDragWidth({ key: "recursion.composerHeight", initial: AUTO, min: 60, max: 900, edge: "top", from: () => ref.current?.offsetHeight ?? AUTO });
  const fixed = tall.width !== AUTO || tall.dragging;
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    const h = fixed ? tall.width : Math.min(el.scrollHeight + 2, AUTO);
    el.style.height = h + "px";
    el.style.overflowY = el.scrollHeight > h ? "auto" : "hidden";
  }, [text, tall.width, fixed]);

  // @mention autocomplete for the word under the caret.
  const token = /(^|\s)@([\w\-一-鿿]*)$/.exec(text.slice(0, caret));
  const options = token
    ? [...members.map((m) => ({ alias: m.alias, label: modelLabel(m), m })), { alias: "all", label: tr("mention.all"), m: undefined }].filter((o) =>
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
  // Formatting: the toolbar and Ctrl/⌘ shortcuts write Markdown around the selection.
  const fmt = (make: (t: string, a: number, b: number) => Edit) => {
    const el = ref.current;
    if (!el || disabled) return;
    apply(el, make(el.value, el.selectionStart, el.selectionEnd));
  };
  const TOOLS: { key: string; make: (t: string, a: number, b: number) => Edit; kbd?: string }[] = [
    { key: "fmt.bold", make: (t, a, b) => wrap(t, a, b, "**"), kbd: "B" },
    { key: "fmt.italic", make: (t, a, b) => wrap(t, a, b, "*"), kbd: "I" },
    { key: "fmt.strike", make: (t, a, b) => wrap(t, a, b, "~~") },
    { key: "fmt.code", make: (t, a, b) => wrap(t, a, b, "`"), kbd: "E" },
    { key: "fmt.codeBlock", make: codeBlock },
    { key: "fmt.bullets", make: (t, a, b) => prefix(t, a, b, "bullet") },
    { key: "fmt.numbers", make: (t, a, b) => prefix(t, a, b, "number") },
  ];
  const submit = () => {
    if ((!text.trim() && !files.length && !refs.length) || disabled) return;
    if (rewindId) {
      send({ t: "ws_rewind", panel, msg: rewindId, text, files, refs });
      leaveRewind(false);
    } else send({ t: "ws_send", panel, text, reply_to: reply?.id, files, refs });
    setReplying(panel, null);
    setText("");
    setHist(null);
    setFiles([]);
    setRefs([]);
    setCaret(0);
  };
  const alias = (id: string) => "@" + (members.find((x) => x.id === id)?.alias ?? "?");
  // Who this would go to (as the daemon reads @mentions: none, or @all, means everyone) and
  // whose account needs logging in again: they would be skipped.
  const accounts = useAccounts();
  const named = [...text.replace(/```[\s\S]*?```|`[^`]*`/g, "").matchAll(/(?:^|[^\p{L}\p{N}])@([\p{L}\p{N}_.-]+)/gu)].map((x) => x[1].replace(/\.+$/, "").toLowerCase());
  const everyone = !named.length || named.some((n) => n === "all" || n === "全部" || n === "所有人");
  const skipped = members.filter((m) => (everyone || named.includes(m.alias.toLowerCase())) && expiredAccount(m, accounts));
  return (
    <div className="ws-input">
      <BusyLine busy={working} />
      {skipped.length > 0 && (text.trim() || files.length || refs.length) ? (
        <div className={`queue-bar expired-bar${round?.waiting?.length ? " stacked" : ""}`}>
          <span className="queue-list"><Icon name="key" size={13} /> {tr("acct.expiredSend", { who: skipped.map((m) => "@" + m.alias).join("、") })}</span>
          <button onClick={() => send({ t: "account_login", account: expiredAccount(skipped[0], accounts)!.id })}>{tr("acct.relogin")}</button>
        </div>
      ) : null}
      {!!round?.waiting?.length && (
        <div className="queue-bar">
          <span className="queue-list">
            {round.waiting.map((w, i) => (
              <span key={i}>{tr("queue.waiting", { n: w.msgs.length, who: w.on.map(alias).join("、") })}</span>
            ))}
          </span>
          <button onClick={() => send({ t: "ws_send_now", panel })}>{tr("queue.sendNow")}</button>
        </div>
      )}
      {rewindAt >= 0 && (
        <div className="reply-bar rewind-bar">
          <span className="reply-ref">
            <span className="reply-who"><Icon name="rewind" size={13} /> {tr("rewind.title")}</span>
            <span className="reply-text">{tr("rewind.detail", { n: doomed })}</span>
          </span>
          <button className="reply-x" onClick={() => leaveRewind(true)} aria-label={tr("rewind.cancel")} title={tr("rewind.cancel") + "（Esc）"}>
            <Icon name="close" />
          </button>
        </div>
      )}
      {reply && reply.kind === "text" && (
        <div className="reply-bar">
          <button className="reply-ref" onClick={() => showInChat(reply.id)} title={tr("reply.jump")}>
            <span className="reply-who"><Icon name="reply" size={13} /> {tr("reply.to")} {authorName(reply.from, new Map(members.map((x) => [x.id, x])))}</span>
            <span className="reply-text">{excerpt(reply.text)}</span>
          </button>
          <button className="reply-x" onClick={() => setReplying(panel, null)} aria-label={tr("reply.cancel")} title={tr("reply.cancel") + "（Esc）"}>
            <Icon name="close" />
          </button>
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
      {polling && (
        <Modal title={tr("poll.start")} onClose={() => setPolling(false)}>
          <PollForm panel={panel} onDone={() => setPolling(false)} />
        </Modal>
      )}
      <div className={`cmp-box${disabled ? " off" : ""}${tall.dragging ? " resizing" : ""}`}>
        <div className="cmp-resize" onPointerDown={tall.onPointerDown} onDoubleClick={tall.reset} title={tr("composer.resize")} role="separator" aria-orientation="horizontal" />
      <div className="cmp-bar" role="toolbar" aria-label={tr("fmt.toolbar")}>
        {TOOLS.map((tool) => (
          <button
            key={tool.key}
            className="cmp-tool"
            disabled={disabled}
            // Keep the text selected: the click must not take the focus.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => fmt(tool.make)}
            title={tr(tool.key) + (tool.kbd ? `（${isMac ? "⌘" : "Ctrl+"}${tool.kbd}）` : "")}
            aria-label={tr(tool.key)}
          >
            <Icon name={tool.key.slice(4) as IconName} />
          </button>
        ))}
        <span className="cmp-sep" />
        <button className={`cmp-tool${picking ? " on" : ""}`} onMouseDown={(e) => e.preventDefault()} onClick={() => setPicking((v) => !v)} disabled={disabled} title={tr("file.attach")} aria-label={tr("file.attach")}>
          <Icon name="attach" />
        </button>
        <button className="cmp-tool" onClick={() => setPolling(true)} disabled={disabled} title={tr("poll.buttonHint")} aria-label={tr("poll.start")}>
          <Icon name="poll" />
        </button>
      </div>
      {picking && (
        <FilePicker
          panel={panel}
          onPick={(path) => {
            setFiles((f) => (f.includes(path) ? f : [...f, path]));
            setPicking(false);
            ref.current?.focus();
          }}
          onClose={() => setPicking(false)}
        />
      )}
      {files.length + refs.length > 0 && (
        <div className="cmp-files">
          {files.map((f) => (
            <FileChip key={f} panel={panel} path={f} onRemove={() => setFiles((x) => x.filter((y) => y !== f))} />
          ))}
          {refs.map((r) => (
            <RefChip key={`${r.kind}/${r.id}/${r.item ?? ""}`} panel={panel} r={r} onRemove={() => setRefs((x) => x.filter((y) => !sameRef(y, r)))} />
          ))}
        </div>
      )}
      <textarea
        ref={ref}
        rows={1}
        value={text}
        disabled={disabled}
        placeholder={disabled ? tr("composer.addAgentFirst") : tr("composer.placeholder")}
        onChange={(e) => {
          setHist(null);
          setText(e.target.value);
          setCaret(e.target.selectionStart ?? e.target.value.length);
          setSel(0);
          setMenuOff(false);
        }}
        onSelect={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
        onPaste={(e) => {
          // Pictures on the clipboard (a screenshot, an image copied from a page) are attached.
          if (disabled) return;
          const attach = (f: File) => void pasteImage(panel, f).then((p) => p && setFiles((x) => (x.includes(p) ? x : [...x, p])));
          const ok = (f: File | null): f is File => !!f && /^image\/(png|jpeg|gif|webp)$/.test(f.type);
          const pics = [...e.clipboardData.files, ...[...e.clipboardData.items].filter((i) => i.kind === "file").map((i) => i.getAsFile())].filter(ok);
          const text = e.clipboardData.getData("text/plain");
          if (pics.length) {
            if (!text) e.preventDefault();
            new Set(pics).forEach(attach);
          } else if (!text) {
            // The webview left the picture out (⌘V of a screenshot on macOS): read it ourselves.
            e.preventDefault();
            void readClipboardImage().then((f) => f && attach(f));
          }
        }}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing || e.keyCode === 229) return; // IME (注音、拼音) still composing
          if (showMenu) {
            if (e.key === "ArrowDown") return e.preventDefault(), setSel((s) => (s + 1) % options.length);
            if (e.key === "ArrowUp") return e.preventDefault(), setSel((s) => (s + options.length - 1) % options.length);
            if (e.key === "Tab" || e.key === "Enter") return e.preventDefault(), pick(options[Math.min(sel, options.length - 1)].alias);
            if (e.key === "Escape") return e.preventDefault(), setMenuOff(true);
          }
          const el = e.currentTarget;
          const mod = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
          if (mod && !e.shiftKey && !e.altKey) {
            const tool = TOOLS.find((x) => x.kbd && x.kbd.toLowerCase() === e.key.toLowerCase());
            if (tool) return e.preventDefault(), fmt(tool.make);
          }
          const collapsed = el.selectionStart === el.selectionEnd;
          const plain = !e.shiftKey && !e.altKey && !e.metaKey && !e.ctrlKey;
          // Backspace at the very start takes off the last attachment (a pasted picture, say).
          if (plain && collapsed && e.key === "Backspace" && el.selectionStart === 0 && refs.length) {
            return e.preventDefault(), setRefs((x) => x.slice(0, -1));
          }
          if (plain && collapsed && e.key === "Backspace" && el.selectionStart === 0 && files.length) {
            return e.preventDefault(), setFiles((x) => x.slice(0, -1));
          }
          if (plain && collapsed && e.key === "ArrowUp" && sent.length && (hist !== null || !el.value)) {
            // From an empty box, or while already walking back; a draft never gets replaced.
            const next = hist === null ? sent.length - 1 : Math.max(0, hist - 1);
            if (hist === null || !el.value.slice(0, el.selectionStart).includes("\n")) return e.preventDefault(), recall(next);
          }
          if (plain && collapsed && e.key === "ArrowDown" && hist !== null && !el.value.slice(el.selectionStart).includes("\n")) {
            return e.preventDefault(), recall(hist + 1 < sent.length ? hist + 1 : null);
          }
          // Inside a ``` block, Enter and Tab write code instead of sending / leaving the box.
          if (collapsed && inCodeBlock(el.value, el.selectionStart)) {
            if (e.key === "Tab" && !e.shiftKey) return e.preventDefault(), fmt((_, a, b) => ({ from: a, to: b, insert: "  ", selStart: a + 2, selEnd: a + 2 }));
            if (e.key === "Enter") return;
          }
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            // On a list line Enter starts the next item (or ends the list on an empty one).
            const next = collapsed ? continueList(el.value, el.selectionStart) : null;
            if (next) apply(el, next);
            else submit();
          } else if (e.key === "Escape" && rewindId) {
            e.preventDefault();
            leaveRewind(true);
          } else if (e.key === "Escape" && reply) {
            e.preventDefault();
            setReplying(panel, null);
          } else if (e.key === "Escape" && busy) {
            e.preventDefault();
            send({ t: "ws_stop", panel });
          }
        }}
      />
      </div>
      {/* Stopping lives on each agent in the sidebar (and Esc stops all); sending while they work queues. */}
      <button className="send-btn" onClick={submit} disabled={disabled || (!text.trim() && !files.length && !refs.length)} title={busy ? tr("composer.sendQueued") : tr("composer.send")}>
        <Icon name="up" />
      </button>
    </div>
  );
}
