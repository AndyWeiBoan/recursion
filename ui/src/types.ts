export type PanelKind = "workspace" | "terminal" | "browser";
export type Dir = "row" | "column";

export type Node =
  | { type: "split"; id: string; dir: Dir; children: Node[]; sizes: number[] }
  | { type: "leaf"; panel: string };

export interface Panel {
  id: string;
  kind: PanelKind;
  url?: string | null;
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
}

export interface State {
  sessions: Session[];
  panels: Record<string, Panel>;
  active_session: string | null;
}

export interface TermInfo {
  alive: boolean;
  fg: string | null;
}

export type Cmd =
  | { t: "session_create"; name: string; cwd: string; kind?: PanelKind }
  | { t: "session_delete"; session: string }
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
  | { t: "ws_add_agent"; panel: string; kind: "claude"; alias: string; model?: string | null }
  | { t: "ws_remove_agent"; panel: string; member: string }
  | { t: "ws_send"; panel: string; text: string }
  | { t: "ws_perm"; request_id: string; decision: "allow" | "always" | "deny" }
  | { t: "ws_stop"; panel: string }
  | { t: "ws_continue"; panel: string };

export type MemberStatus = "idle" | "starting" | "thinking" | "waiting_permission";

export interface Member {
  id: string;
  kind: "claude";
  alias: string;
  model?: string | null;
  cli_session_id?: string | null;
  cost_usd: number;
  status: MemberStatus;
  usage: Record<string, number>;
}

export type PermState = "pending" | "allowed" | "always_allowed" | "denied";

export type Msg = { id: string; ts: number; from: string; seq: number; to?: string[]; streaming: boolean } & (
  | { kind: "text"; text: string }
  | { kind: "tool"; tool_use_id: string; name: string; summary: string; detail: string; result?: string | null; is_error: boolean }
  | { kind: "permission"; request_id: string; tool: string; summary: string; detail: string; state: PermState }
  | { kind: "system"; text: string }
  | { kind: "error"; text: string }
  | { kind: "guard"; text: string; resolved: boolean }
);

export interface Workspace {
  members: Member[];
  messages: Msg[];
}

export function leaves(n: Node): string[] {
  return n.type === "leaf" ? [n.panel] : n.children.flatMap(leaves);
}

export const KIND_LABEL: Record<PanelKind, string> = {
  workspace: "群聊",
  terminal: "終端",
  browser: "瀏覽器",
};
