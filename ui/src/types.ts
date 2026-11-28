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
  /** An agent whose account used this much (percent) of a usage window sits out; absent = no limit. */
  usage_limit?: number;
  /** This chat's changes to its project's instructions files, MCP servers, Skills and rules (on/off by name). */
  guides?: Record<string, boolean>;
  mcp?: Record<string, boolean>;
  skills?: Record<string, boolean>;
  /** The library's rules (`rules` is the chat's own text). */
  rule_picks?: Record<string, boolean>;
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
  /** The human's MCP servers this session's agents get. */
  mcp?: string[];
  /** What every agent of the project is told (a chat's rules go above it). */
  prompt?: string;
  /** Guidance files ticked for the agents; absent = not chosen yet (the daemon's defaults). */
  guides?: string[] | null;
  /** Skills and rules of Recursion's library its agents get. */
  skills?: string[];
  rules?: string[];
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
  /** Agents removed from a chat, to put back into it (while it is open). */
  | { kind: "agents"; panel: string; chat_name: string; members: Member[] }
);

export interface TermInfo {
  alive: boolean;
  fg: string | null;
}

export type Cmd =
  | { t: "session_create"; name: string; cwd: string; kind?: PanelKind }
  | { t: "session_close"; session: string }
  | { t: "closed_reopen"; id: string }
  | { t: "mcp_list" }
  | { t: "mcp_save"; name: string; old?: string; server: { command?: string; args?: string[]; env?: Record<string, string | null>; url?: string; headers?: Record<string, string | null>; sse?: boolean } }
  | { t: "mcp_remove"; name: string }
  | { t: "mcp_import"; picks: [string, string][] }
  | { t: "mcp_test"; name: string }
  | { t: "closed_delete"; id: string }
  | { t: "session_rename"; session: string; name: string }
  | { t: "session_group"; session: string; group: string }
  | { t: "session_select"; session: string }
  | { t: "window_create"; session: string; kind?: PanelKind }
  | { t: "window_select"; session: string; window: string }
  | { t: "window_rename"; session: string; window: string; name: string }
  | { t: "panel_rename"; panel: string; name: string }
  | { t: "window_close"; session: string; window: string }
  | { t: "panel_split"; panel: string; dir: Dir; kind?: PanelKind }
  | { t: "panel_move"; panel: string; target: string; side: "left" | "right" | "top" | "bottom" | "center" }
  | { t: "panel_to_window"; panel: string; window: string | null }
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
  | { t: "session_settings"; session: string; cwd: string; defaults: AgentDefaults; mcp?: string[]; prompt?: string; guides?: string[]; skills?: string[]; rules?: string[] }
  | { t: "library_list"; kind: "skill" | "rule" }
  | { t: "chat_layers"; session: string }
  | { t: "layer_clear"; panel: string; member?: string; field: "guides" | "mcp" | "skills" | "rule_picks"; names: string[] }
  | { t: "library_import"; kind: "skill" | "rule"; paths: string[] }
  | { t: "library_remove"; kind: "skill" | "rule"; name: string }
  | { t: "library_read"; kind: "skill" | "rule"; name: string | null; path: string | null }
  | { t: "guides_list"; session: string }
  | { t: "agent_info"; panel: string; member: string }
  | { t: "guide_read"; session: string; path: string }
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
  | { t: "ws_remove_all_agents"; panel: string }
  | { t: "session_move"; session: string; to: number }
  | { t: "window_move"; session: string; window: string; to: number }
  | { t: "panel_order"; window: string; panels: string[] }
  | { t: "ws_move_agent"; panel: string; member: string; to: number }
  | { t: "ws_agent_settings"; panel: string; member: string; alias: string; model: string | null; effort: string | null; prompt: string; mode: PermMode; guides: Record<string, boolean>; mcp: Record<string, boolean>; skills: Record<string, boolean>; rule_picks: Record<string, boolean> }
  | { t: "ws_send"; panel: string; text: string; reply_to?: string; files?: string[]; refs?: OutputRef[] }
  | { t: "files_list"; panel: string }
  | { t: "file_get"; panel: string; path: string }
  | { t: "file_open"; panel: string; path: string }
  | { t: "files_grant"; panel: string; paths: string[] }
  | { t: "image_paste"; panel: string; data: string; mime: string }
  | { t: "ws_stop_agent"; panel: string; member: string }
  | { t: "poll_create"; panel: string; question: string; description: string; options: string[]; vote?: number | null }
  | { t: "poll_vote"; panel: string; poll: string; option: number }
  | { t: "poll_close"; panel: string; poll: string }
  | { t: "todo_list_create"; panel: string; name: string }
  | { t: "todo_list_rename"; panel: string; list: string; name: string }
  | { t: "todo_list_remove"; panel: string; list: string }
  | { t: "todo_add"; panel: string; list: string; text: string }
  | { t: "todo_update"; panel: string; list: string; id: string; text?: string; note?: string; state?: TodoState }
  | { t: "todo_move"; panel: string; list: string; id: string; index: number; archived: boolean }
  | { t: "todo_remove"; panel: string; list: string; id: string }
  | { t: "ws_perm"; request_id: string; decision: "allow" | "always" | "deny" }
  | { t: "ws_stop"; panel: string }
  | { t: "ws_send_now"; panel: string }
  | { t: "ws_rewind"; panel: string; msg: string; text: string; files: string[]; refs: OutputRef[] }
  | { t: "models_list"; kind: AgentKind }
  | { t: "doc_open"; panel: string; doc: string }
  | { t: "artifact_open"; panel: string; artifact: string }
  | { t: "output_remove"; panel: string; id: string }
  | { t: "ws_roster" }
  | { t: "thread_open"; panel: string; msg: string; quote: string; text: string }
  | { t: "thread_send"; panel: string; thread: string; text: string }
  | { t: "thread_rewind"; panel: string; thread: string; at: number; text: string }
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
  /** How hard it thinks, in its CLI's words; absent = the CLI's default. */
  effort?: string | null;
  /** The effort its CLI says it last ran with. */
  running_effort?: string | null;
  /** The human's rules for this agent alone. */
  prompt?: string;
  /** Its own changes to its chat's instructions files, MCP servers, Skills and rules. */
  guides?: Record<string, boolean>;
  mcp?: Record<string, boolean>;
  skills?: Record<string, boolean>;
  rule_picks?: Record<string, boolean>;
  /** While waiting for the human's permission: what for. */
  waiting_for?: string | null;
  /** The `seq` of its latest reply (see seen.ts). */
  replied?: number;
  /** What the CLI says it runs: `model`, or its own default when none was picked. */
  running_model?: string | null;
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
  /** Logged in, but the provider turned the login down since (see accounts.rs). */
  expired?: { at: number; why: string } | null;
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
/** One of the chat's outputs a human message points at. */
export interface OutputRef {
  kind: "doc" | "diagram" | "page" | "poll" | "todo" | "thread";
  /** The artifact's, poll's, thread's or todo list's id. */
  id: string;
  /** A todo item's id, when it is one item of the list. */
  item?: string;
  /** Its title when sent. */
  title: string;
}

export interface ReplyRef {
  msg: string;
  from: string;
  text: string;
}

export type Msg = { id: string; ts: number; from: string; seq: number; to?: string[]; streaming: boolean; turn?: boolean; via?: DocVia; from_thread?: { thread: string; num: number }; reply_to?: ReplyRef; files?: string[]; refs?: OutputRef[]; usage?: TurnUsage } & (
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

export type TodoState = "todo" | "doing" | "done";

/** One item of a todo list. */
export interface Todo {
  id: string;
  num: number;
  text: string;
  /** Its card: details in Markdown. */
  note?: string;
  state: TodoState;
  /** Who is on it while it's being done: "you" or a member id. */
  worker?: string;
  /** "you" or a member id. */
  by: string;
  done_by?: string;
  /** Put away in the list's 完成區. */
  archived: boolean;
  created: number;
}

/** A named todo list of a chat; items in the human's order. */
export interface TodoList {
  id: string;
  num: number;
  name: string;
  items: Todo[];
  created: number;
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

/** Who is answering, and the human's messages waiting for busy agents (`on`: the agents each waits for). */
export interface Round {
  active: boolean;
  waiting: { msgs: string[]; on: string[] }[];
}

export interface Workspace {
  members: Member[];
  messages: Msg[];
  threads?: Thread[];
  polls?: Poll[];
  todo_lists?: TodoList[];
  /** Documents and pages the agents made for this chat. */
  artifacts?: Artifact[];
  round?: Round;
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
/** A session's chats, named as the sidebar names them: their own name, else "Chat 2" (numbered
 * across the session, like SessionTree's panelNumbers). */
export function chatsOf(s: Session, state: State): Chat[] {
  const out: Chat[] = [];
  for (const w of s.windows)
    for (const p of leaves(w.root).filter((p) => state.panels[p]?.kind === "workspace"))
      out.push({ panel: p, label: state.panels[p].title || `${t("kind.workspace")} ${out.length + 1}` });
  return out;
}

