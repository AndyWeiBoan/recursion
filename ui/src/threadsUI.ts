// 產出 drawer (threads, documents, pages) state per chat panel, plus the panel the layout is zoomed to while a drawer is open.
// Kept outside React: zooming re-mounts the chat panel, and the drawer must survive that.
import { useSyncExternalStore } from "react";

export type ThreadView = { kind: "list" } | { kind: "thread"; id: string } | { kind: "new"; msg: string; quote: string } | { kind: "poll"; id: string };

export interface ThreadsUI {
  open: boolean;
  view: ThreadView;
  /** Search text typed in the list. */
  query: string;
  /** Only threads about this main-chat message (set by its 💬 badge). */
  msg: string | null;
  /** Which kind of 產出 the list shows. */
  filter: ArtifactFilter;
}

export type ArtifactFilter = "all" | "thread" | "doc" | "page" | "poll";

const CLOSED: ThreadsUI = { open: false, view: { kind: "list" }, query: "", msg: null, filter: "all" };
let state: Record<string, ThreadsUI> = {};
let zoomed: string | null = null;
const subs = new Set<() => void>();
const notify = () => subs.forEach((f) => f());
const subscribe = (f: () => void) => (subs.add(f), () => subs.delete(f));

export function useThreadsUI(panel: string): ThreadsUI {
  return useSyncExternalStore(subscribe, () => state[panel] ?? CLOSED);
}

/** The chat panel shown alone (its splits hidden, not closed) while its drawer is open. */
export function useZoomed(): string | null {
  return useSyncExternalStore(subscribe, () => zoomed);
}

export function openThreads(panel: string, patch: Partial<ThreadsUI>) {
  // One drawer at a time: it owns the zoom.
  if (zoomed && zoomed !== panel) state = { ...state, [zoomed]: { ...(state[zoomed] ?? CLOSED), open: false } };
  state = { ...state, [panel]: { ...(state[panel] ?? CLOSED), ...patch, open: true } };
  zoomed = panel;
  notify();
}

/** Keep the drawer open but show the chat's splits again (something it opened is in one of them). */
export function unzoom() {
  if (!zoomed) return;
  zoomed = null;
  notify();
}

export function closeThreads(panel: string) {
  state = { ...state, [panel]: { ...(state[panel] ?? CLOSED), open: false } };
  if (zoomed === panel) zoomed = null;
  notify();
}

export function setThreadView(panel: string, view: ThreadView) {
  if (!state[panel]?.open) return;
  state = { ...state, [panel]: { ...state[panel], view } };
  notify();
}

export function setThreadSearch(panel: string, patch: Partial<Pick<ThreadsUI, "query" | "msg" | "filter">>) {
  state = { ...state, [panel]: { ...(state[panel] ?? CLOSED), ...patch } };
  notify();
}
