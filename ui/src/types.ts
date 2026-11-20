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
  | { t: "session_create"; name: string; cwd: string }
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
  | { t: "term_restart"; panel: string; cols: number; rows: number };

export function leaves(n: Node): string[] {
  return n.type === "leaf" ? [n.panel] : n.children.flatMap(leaves);
}

export const KIND_LABEL: Record<PanelKind, string> = {
  workspace: "群聊",
  terminal: "終端",
  browser: "瀏覽器",
};
