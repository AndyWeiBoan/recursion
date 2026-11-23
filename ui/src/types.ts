import { t } from "./i18n";

export type PanelKind = "workspace" | "terminal" | "browser";
export type Dir = "row" | "column";

export type Node =
  | { type: "split"; id: string; dir: Dir; children: Node[]; sizes: number[] }
  | { type: "leaf"; panel: string };

export interface Panel {
  id: string;
  kind: PanelKind;
  url?: string | null;
  /** A chat's own name, from its settings. */
  title?: string;
}

/** Starting values for agents added in a session. */
export interface AgentDefaults {
  kind?: AgentKind;
  claude_model?: string;
  codex_model?: string;
  mode?: PermMode;
}

/** A chat's name, standing rules for its agents, and spending limit. */
export interface ChatSettings {
  name: string;
  rules: string;
  budget_usd?: number;
}

export interface Window {
  id: string;
  name: string;
  root: Node;
  focus: string;
}

export interface Session {
  id: string;
  name: string;
  cwd: string;
  group: string;
  windows: Window[];
  active_window: string | null;
  docs?: Doc[];
  defaults?: AgentDefaults;
}

/** An HTML document an agent wrote for the session; browser panels show it at `doc:<id>`. */
export interface Doc {
  id: string;
  title: string;
  kind?: "html" | "diagram";
  /** The chat panel that wrote it. */
  chat?: string;
  author: string;
  rev: number;
  updated: number;
}

export interface State {
  sessions: Session[];
  panels: Record<string, Panel>;
  active_session: string | null;
  /** The human's language; absent follows the system. */
  lang?: string | null;
  /** Sessions and chats the human closed, newest last; they can be opened again. */
  closed?: Closed[];
}

export type Closed = { id: string; at: number } & (
  | { kind: "session"; session: Session; panels: Panel[] }
  | { kind: "chat"; panel: Panel; session: string; session_name: string; cwd: string }
);

export interface TermInfo {
  alive: boolean;
  fg: string | null;
}

export type Cmd =
  | { t: "session_create"; name: string; cwd: string; kind?: PanelKind }
  | { t: "session_close"; session: string }
  | { t: "closed_reopen"; id: string }
  | { t: "closed_delete"; id: string }
  | { t: "session_rename"; session: string; name: string }
  | { t: "session_group"; session: string; group: string }
  | { t: "session_select"; session: string }
  | { t: "window_create"; session: string; kind?: PanelKind }
  | { t: "window_select"; session: string; window: string }
  | { t: "window_rename"; session: string; window: string; name: string }
  | { t: "window_close"; session: string; window: string }
  | { t: "panel_split"; panel: string; dir: Dir; kind?: PanelKind }
  | { t: "panel_close"; panel: string }
  | { t: "panel_focus"; panel: string }
  | { t: "panel_kind"; panel: string; kind: PanelKind }
  | { t: "panel_url"; panel: string; url: string }
  | { t: "layout_resize"; window: string; split: string; sizes: number[] }
  | { t: "term_attach"; panel: string; cols: number; rows: number }
  | { t: "term_input"; panel: string; data: string }
  | { t: "term_resize"; panel: string; cols: number; rows: number }
  | { t: "term_restart"; panel: string; cols: number; rows: number }
  | { t: "ws_attach"; panel: string }
  | { t: "ws_add_agent"; panel: string; kind: AgentKind; alias: string; model?: string | null; account?: string | null; mode: PermMode }
  | { t: "ws_set_mode"; panel: string; member: string; mode: PermMode }
  | { t: "set_lang"; lang: string | null }
  | { t: "session_settings"; session: string; cwd: string; defaults: AgentDefaults }
  | { t: "ws_settings"; panel: string; settings: ChatSettings }
  | { t: "ws_compact"; panel: string; member: string }
  | { t: "accounts_list" }
  | { t: "accounts_refresh" }
  | { t: "daemon_info" }
  | { t: "daemon_restart" }
  | { t: "account_create"; kind: AgentKind; name?: string }
  | { t: "account_adopt"; kind: AgentKind }
  | { t: "logins_detect" }
  | { t: "cli_status"; fresh?: boolean }
  | { t: "cli_install"; kind: AgentKind }
  | { t: "setup_close"; panel: string }
  | { t: "account_login"; account: string }
  | { t: "account_rename"; account: string; name: string }
  | { t: "account_remove"; account: string }
  | { t: "ws_remove_agent"; panel: string; member: string }
  | { t: "ws_rename_agent"; panel: string; member: string; alias: string }
  | { t: "ws_send"; panel: string; text: string; reply_to?: string; files?: string[] }
  | { t: "files_list"; panel: string }
  | { t: "file_get"; panel: string; path: string }
  | { t: "file_open"; panel: string; path: string }
  | { t: "files_grant"; panel: string; paths: string[] }
  | { t: "image_paste"; panel: string; data: string; mime: string }
  | { t: "ws_stop_agent"; panel: string; member: string }
  | { t: "poll_create"; panel: string; question: string; description: string; options: string[]; vote?: number | null }
  | { t: "poll_vote"; panel: string; poll: string; option: number }
  | { t: "poll_close"; panel: string; poll: string }
  | { t: "ws_perm"; request_id: string; decision: "allow" | "always" | "deny" }
  | { t: "ws_stop"; panel: string }
  | { t: "ws_send_now"; panel: string }
  | { t: "models_list"; kind: AgentKind }
  | { t: "doc_open"; panel: string; doc: string }
  | { t: "artifact_open"; panel: string; artifact: string }
  | { t: "output_remove"; panel: string; id: string }
  | { t: "ws_roster" }
  | { t: "thread_open"; panel: string; msg: string; quote: string; text: string }
  | { t: "thread_send"; panel: string; thread: string; text: string }
  | { t: "thread_bring"; panel: string; thread: string }
  | { t: "doc_get"; session: string; doc: string }
  | { t: "doc_action"; session: string; doc: string; action: "reply" | "prompt"; data: string }
  | { t: "browser_host" }
  | { t: "browse_result"; id: number; ok: boolean; result?: unknown; error?: string };

export type AgentKind = "claude" | "codex";

export type MemberStatus = "idle" | "starting" | "thinking" | "waiting_permission" | "compacting";

/** How much an agent may do without asking. */
export type PermMode = "ask" | "edits" | "auto";
export const PERM_MODES: { id: PermMode; label: string; hint: string }[] = [
  { id: "ask", get label() { return t("perm.ask"); }, get hint() { return t("perm.ask.hint"); } },
  { id: "edits", get label() { return t("perm.edits"); }, get hint() { return t("perm.edits.hint"); } },
  { id: "auto", get label() { return t("perm.auto"); }, get hint() { return t("perm.auto.hint"); } },
];

export interface Member {
  id: string;
  kind: AgentKind;
  alias: string;
  mode?: PermMode;
  model?: string | null;
  /** Account id; absent = the default account of its kind. */
  account?: string | null;
  cli_session_id?: string | null;
  cost_usd: number;
  status: MemberStatus;
  usage: Record<string, number>;
  context_used?: number;
  context_window?: number;
}

/** One login of a CLI (see accounts.rs). The default one of each kind is the CLI's own. */
export interface Account {
  id: string;
  kind: AgentKind;
  name: string;
  email?: string | null;
  plan?: string | null;
  logged_in: boolean;
  usage: Record<string, number>;
  usage_at: number;
  /** When each window resets, seconds since epoch. */
  resets?: Record<string, number>;
}

export type PermState = "pending" | "allowed" | "always_allowed" | "denied";

/** Tokens one agent turn used; `input` includes `cached`. Codex has no price. */
export interface TurnUsage {
  input: number;
  cached: number;
  output: number;
  cost_usd?: number;
}

/** A message a doc page sent in the human's name (`recursion.reply` / `sendPrompt`). */
export interface DocVia {
  doc: string;
  title: string;
  data: string;
}

/** The message a human message answers, as it read when sent. `from` is "you" or a member id. */
export interface ReplyRef {
  msg: string;
  from: string;
  text: string;
}

export type Msg = { id: string; ts: number; from: string; seq: number; to?: string[]; streaming: boolean; turn?: boolean; via?: DocVia; from_thread?: { thread: string; num: number }; reply_to?: ReplyRef; files?: string[]; usage?: TurnUsage } & (
  | { kind: "text"; text: string }
  | { kind: "tool"; tool_use_id: string; name: string; summary: string; detail: string; result?: string | null; is_error: boolean }
  | { kind: "permission"; request_id: string; tool: string; summary: string; detail: string; state: PermState }
  | { kind: "system"; text: string }
  | { kind: "error"; text: string }
  | { kind: "guard"; text: string; resolved: boolean }
  | { kind: "relogin"; account: string; name: string; text: string; resolved: boolean }
  | { kind: "doc"; doc: string; title: string; updated: boolean }
  | { kind: "poll"; poll: string }
  | { kind: "poll_closed"; poll: string }
);

export interface Vote {
  /** "you" or a member id. */
  voter: string;
  option: number;
  reason: string;
  ts: number;
}

export interface Poll {
  id: string;
  num: number;
  question: string;
  /** Background for the question; may be empty. */
  description: string;
  options: string[];
  /** Members asked to vote. */
  voters: string[];
  votes: Vote[];
  status: "open" | "closed";
  created: number;
  closed?: number | null;
  /** The chat message showing it. */
  msg: string;
  /** Taken off the 產出 list (the chat still shows it). */
  removed?: boolean;
}

export type ThreadStatus = "replying" | "waiting" | "brought";

export interface ThreadMsg {
  /** "you", a member id, or "system". */
  from: string;
  text: string;
  ts: number;
}

/** A side discussion about one main-chat message, in a fork of its author's conversation. */
export interface Thread {
  id: string;
  num: number;
  msg: string;
  quote: string;
  /** About the whole message, not a selected part. */
  whole?: boolean;
  agent: string;
  fork?: string | null;
  messages: ThreadMsg[];
  status: ThreadStatus;
  updated: number;
  /** Taken off the 產出 list (the chat still shows it). */
  removed?: boolean;
}

export interface Workspace {
  members: Member[];
  messages: Msg[];
  threads?: Thread[];
  polls?: Poll[];
  /** Documents and pages the agents made for this chat. */
  artifacts?: Artifact[];
  round?: { active: boolean; queued: number };
  settings?: ChatSettings;
  /** Agents that left, by member id, so their messages and votes keep their names. */
  former?: Record<string, { alias: string; kind: AgentKind }>;
}

export interface Artifact {
  id: string;
  kind: "doc" | "page" | "diagram";
  /** Doc id, or the page's url. */
  target: string;
  title: string;
  /** Member id of the agent that made it. */
  by: string;
  created: number;
  updated: number;
  /** Taken off the 產出 list (the chat still shows it). */
  removed?: boolean;
}

export function leaves(n: Node): string[] {
  return n.type === "leaf" ? [n.panel] : n.children.flatMap(leaves);
}

export interface Chat {
  panel: string;
  /** "群聊 · window 2" style label, for menus and the tree when a session has several chats. */
  label: string;
}

/** The chat panels of a session, in window then layout order. */
export function chatsOf(s: Session, state: State): Chat[] {
  const out: Chat[] = [];
  s.windows.forEach((w, wi) => {
    const ids = leaves(w.root).filter((p) => state.panels[p]?.kind === "workspace");
    ids.forEach((p, i) => out.push({ panel: p, label: `${t("kind.workspace")} · window ${wi + 1}${ids.length > 1 ? ` (${i + 1})` : ""}${w.name && w.name !== String(wi + 1) ? ` ${w.name}` : ""}` }));
  });
  return out;
}

export const KIND_LABEL: Record<PanelKind, string> = {
  get workspace() { return t("kind.workspace"); },
  get terminal() { return t("kind.terminal"); },
  get browser() { return t("kind.browser"); },
};
