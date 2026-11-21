// Connection to recursiond plus a tiny external store for React.
import { useEffect, useSyncExternalStore } from "react";
import { setThreadView } from "./threadsUI";
import type { AgentKind, Cmd, Member, Msg, State, TermInfo, Workspace } from "./types";

export type ConnStatus = "connecting" | "open" | "closed" | "error";

interface Snapshot {
  state: State | null;
  terms: Record<string, TermInfo>;
  status: ConnStatus;
  error: string | null;
}

type TermListener = {
  onReplay(data: Uint8Array): void;
  onOutput(data: Uint8Array): void;
  onExit(): void;
};

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let snap: Snapshot = { state: null, terms: {}, status: "connecting", error: null };
const subs = new Set<() => void>();
const termListeners = new Map<string, TermListener>();
const reconnectHooks = new Set<() => void>();
let ws: WebSocket | null = null;
let lastError: string | null = null;

function set(p: Partial<Snapshot>) {
  snap = { ...snap, ...p };
  subs.forEach((f) => f());
}

function b64(data: string): Uint8Array {
  const bin = atob(data);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

async function daemonAddress(): Promise<{ port: number; token: string }> {
  if (isTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke("daemon_info");
  }
  // Browser dev mode: http://localhost:1420/?port=…&token=…
  const q = new URLSearchParams(location.search);
  const port = Number(q.get("port"));
  const token = q.get("token") ?? "";
  if (!port || !token) throw new Error("瀏覽器模式需要在網址加上 ?port=…&token=…（見 ~/.recursion/daemon.json）");
  return { port, token };
}

let retry = 0;
export async function connect() {
  set({ status: "connecting" });
  try {
    const { port, token } = await daemonAddress();
    const sock = new WebSocket(`ws://127.0.0.1:${port}/ws?token=${encodeURIComponent(token)}`);
    ws = sock;
    sock.onopen = () => {
      retry = 0;
      set({ status: "open", error: null });
      reconnectHooks.forEach((f) => f());
    };
    sock.onmessage = (e) => {
      const m = JSON.parse(e.data);
      switch (m.t) {
        case "state":
          set({ state: m.state, terms: m.terms ?? {} });
          break;
        case "replay":
          termListeners.get(m.panel)?.onReplay(b64(m.data));
          break;
        case "out":
          termListeners.get(m.panel)?.onOutput(b64(m.data));
          break;
        case "exit":
          termListeners.get(m.panel)?.onExit();
          break;
        case "error":
          lastError = m.msg;
          console.warn("recursiond:", m.msg);
          toast(m.msg);
          break;
        case "ws_thread":
          wsUpdate(m.panel, (w) => {
            const list = w.threads ?? [];
            const i = list.findIndex((x) => x.id === m.thread.id);
            return { ...w, threads: i >= 0 ? list.map((x, k) => (k === i ? m.thread : x)) : [...list, m.thread] };
          });
          break;
        case "thread_opened":
          setThreadView(m.panel, { kind: "thread", id: m.thread });
          break;
        case "ws_roster":
          for (const [panel, members] of Object.entries(m.panels)) rosterSet(panel, members as Member[]);
          break;
        case "ws_state":
          rosterSet(m.panel, m.ws.members);
          workspaces.set(m.panel, { ...m.ws, round: m.round });
          wsChanged(m.panel);
          break;
        case "ws_msg":
          wsUpdate(m.panel, (w) => {
            const i = w.messages.findIndex((x) => x.id === m.msg.id);
            const messages = [...w.messages];
            if (i >= 0) messages[i] = m.msg as Msg;
            else messages.push(m.msg as Msg);
            return { ...w, messages };
          });
          break;
        case "ws_delta":
          wsUpdate(m.panel, (w) => ({
            ...w,
            messages: w.messages.map((x) => (x.id === m.id && x.kind === "text" ? { ...x, text: x.text + m.text } : x)),
          }));
          break;
        case "ws_member":
          rosterUpsert(m.panel, m.member as Member);
          wsUpdate(m.panel, (w) => {
            const i = w.members.findIndex((x) => x.id === m.member.id);
            const members = [...w.members];
            if (i >= 0) members[i] = m.member as Member;
            else members.push(m.member as Member);
            return { ...w, members };
          });
          break;
        case "ws_round":
          wsUpdate(m.panel, (w) => ({ ...w, round: { active: m.active, queued: m.queued } }));
          break;
        case "ws_msg_removed":
          wsUpdate(m.panel, (w) => ({ ...w, messages: w.messages.filter((x) => x.id !== m.id) }));
          break;
        case "doc":
          docHtml.set(`${m.session}/${m.doc}`, m.html ?? null);
          docSubs.forEach((f) => f());
          break;
        case "models":
          setModels(m.kind, { models: m.models, error: m.error ?? null, loading: false });
          break;
        case "ws_member_removed":
          rosterSet(m.panel, (roster[m.panel] ?? []).filter((x) => x.id !== m.member));
          wsUpdate(m.panel, (w) => ({ ...w, members: w.members.filter((x) => x.id !== m.member) }));
          break;
      }
    };
    sock.onclose = () => {
      if (ws !== sock) return;
      set({ status: "closed" });
      setTimeout(connect, Math.min(4000, 300 * 2 ** retry++));
    };
  } catch (err) {
    set({ status: "error", error: String(err instanceof Error ? err.message : err) });
    setTimeout(connect, Math.min(6000, 500 * 2 ** retry++));
  }
}

export function send(cmd: Cmd) {
  if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(cmd));
}

export function onTerm(panel: string, l: TermListener) {
  termListeners.set(panel, l);
  return () => {
    if (termListeners.get(panel) === l) termListeners.delete(panel);
  };
}

/** Called after every (re)connect — terminals re-attach to get their scrollback. */
export function onReconnect(f: () => void) {
  reconnectHooks.add(f);
  return () => reconnectHooks.delete(f);
}

export function useSnapshot(): Snapshot {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => snap,
  );
}

export function getSnapshot() {
  return snap;
}

export function lastDaemonError() {
  return lastError;
}

export async function pickFolder(): Promise<string | null> {
  if (isTauri) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ directory: true, multiple: false, title: "選擇專案資料夾" });
    return typeof r === "string" ? r : null;
  }
  return window.prompt("專案資料夾的完整路徑", "") || null;
}

// ---------- workspaces (group chats) ----------
const workspaces = new Map<string, Workspace>();
const wsSubs = new Map<string, Set<() => void>>();

function wsChanged(panel: string) {
  wsSubs.get(panel)?.forEach((f) => f());
}
function wsUpdate(panel: string, f: (w: Workspace) => Workspace) {
  const cur = workspaces.get(panel);
  if (!cur) return;
  workspaces.set(panel, f(cur));
  wsChanged(panel);
}

export function useWorkspace(panel: string): Workspace | null {
  return useSyncExternalStore(
    (f) => {
      let set = wsSubs.get(panel);
      if (!set) wsSubs.set(panel, (set = new Set()));
      set.add(f);
      return () => set!.delete(f);
    },
    () => workspaces.get(panel) ?? null,
  );
}

// ---------- toasts ----------
let toastMsg: { id: number; text: string } | null = null;
const toastSubs = new Set<() => void>();
export function toast(text: string) {
  const id = Date.now();
  toastMsg = { id, text };
  toastSubs.forEach((f) => f());
  setTimeout(() => {
    if (toastMsg?.id === id) {
      toastMsg = null;
      toastSubs.forEach((f) => f());
    }
  }, 5000);
}
export function useToast() {
  return useSyncExternalStore(
    (f) => {
      toastSubs.add(f);
      return () => toastSubs.delete(f);
    },
    () => toastMsg,
  );
}

export async function openExternal(url: string) {
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener");
  }
}

export const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

// Model lists come from the CLIs themselves (recursiond asks `claude` / `codex`), so they match
// each CLI's own /model menu without us maintaining a list.
export interface ModelOption {
  value: string;
  label: string;
  description: string;
}
export interface ModelList {
  models: ModelOption[];
  error: string | null;
  loading: boolean;
}

const LOADING: ModelList = { models: [], error: null, loading: true };
const modelLists = new Map<AgentKind, ModelList>();
const modelSubs = new Set<() => void>();

function setModels(kind: AgentKind, l: ModelList) {
  modelLists.set(kind, l);
  modelSubs.forEach((f) => f());
}

export function useModels(kind: AgentKind): ModelList {
  useEffect(() => {
    const cur = modelLists.get(kind);
    if (cur && !cur.error) return;
    setModels(kind, LOADING);
    send({ t: "models_list", kind });
  }, [kind]);
  return useSyncExternalStore(
    (f) => (modelSubs.add(f), () => modelSubs.delete(f)),
    () => modelLists.get(kind) ?? LOADING,
  );
}

// Document contents, fetched on demand and refetched when the doc's rev changes.
const docHtml = new Map<string, string | null>();
const docSubs = new Set<() => void>();

/** HTML of a session doc (undefined while loading, null if it is gone). */
export function useDocHtml(session: string, doc: string, rev: number): string | null | undefined {
  const key = `${session}/${doc}`;
  useEffect(() => {
    send({ t: "doc_get", session, doc });
  }, [session, doc, rev]);
  return useSyncExternalStore(
    (f) => (docSubs.add(f), () => docSubs.delete(f)),
    () => docHtml.get(key),
  );
}

// Members of every chat panel, for the sidebar (chats that aren't on screen included).
// Seeded by `ws_roster`, then kept current by the member events every client receives.
let roster: Record<string, Member[]> = {};
const rosterSubs = new Set<() => void>();

function rosterSet(panel: string, members: Member[]) {
  roster = { ...roster, [panel]: members };
  rosterSubs.forEach((f) => f());
}

function rosterUpsert(panel: string, member: Member) {
  const cur = roster[panel] ?? [];
  const i = cur.findIndex((x) => x.id === member.id);
  rosterSet(panel, i >= 0 ? cur.map((x, k) => (k === i ? member : x)) : [...cur, member]);
}

/** Members per chat panel. Re-requests the list whenever the set of chats (`chatsKey`) changes. */
export function useRoster(chatsKey: string): Record<string, Member[]> {
  useEffect(() => {
    if (snap.status === "open") send({ t: "ws_roster" });
  }, [chatsKey, snap.status]);
  return useSyncExternalStore(
    (f) => (rosterSubs.add(f), () => rosterSubs.delete(f)),
    () => roster,
  );
}

// "Add an agent to this chat" from outside the chat (the sidebar menu).
const addAgentListeners = new Map<string, () => void>();
export function onAddAgentRequest(panel: string, f: () => void) {
  addAgentListeners.set(panel, f);
  return () => {
    if (addAgentListeners.get(panel) === f) addAgentListeners.delete(panel);
  };
}
/** Focus the chat panel and open its add-agent form (once it is on screen). */
export function requestAddAgent(panel: string) {
  send({ t: "panel_focus", panel });
  let tries = 0;
  const tick = () => {
    const f = addAgentListeners.get(panel);
    if (f) f();
    else if (tries++ < 20) setTimeout(tick, 50);
  };
  tick();
}
