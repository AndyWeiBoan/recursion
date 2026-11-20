// Connection to recursiond plus a tiny external store for React.
import { useSyncExternalStore } from "react";
import type { Cmd, State, TermInfo } from "./types";

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

export const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
