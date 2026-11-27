// Connection to recursiond plus a tiny external store for React.
import { syncLang, t } from "./i18n";
import { useEffect, useSyncExternalStore } from "react";
import { agentBrowse } from "./browsers";
import { setThreadView } from "./threadsUI";
import type { Account, AgentKind, Cmd, Member, Msg, State, TermInfo, Workspace } from "./types";

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
  if (!port || !token) throw new Error(t("conn.browserNeedsToken"));
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
      // The desktop app runs agents' browser steps in its native webviews.
      if (isTauri) send({ t: "browser_host" });
      send({ t: "accounts_list" });
      reconnectHooks.forEach((f) => f());
    };
    sock.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.t !== "ws_delta") flushDeltas();
      switch (m.t) {
        case "state":
          set({ state: m.state, terms: m.terms ?? {} });
          syncLang(m.state.lang);
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
        case "ws_settings":
          wsUpdate(m.panel, (w) => ({ ...w, settings: m.settings }));
          break;
        case "ws_poll":
          wsUpdate(m.panel, (w) => {
            const list = w.polls ?? [];
            const i = list.findIndex((x) => x.id === m.poll.id);
            return { ...w, polls: i >= 0 ? list.map((x, k) => (k === i ? m.poll : x)) : [...list, m.poll] };
          });
          break;
        case "ws_todos":
          wsUpdate(m.panel, (w) => ({ ...w, todo_lists: m.lists }));
          break;
        case "ws_artifact":
          wsUpdate(m.panel, (w) => {
            const list = w.artifacts ?? [];
            const i = list.findIndex((x) => x.id === m.artifact.id);
            return { ...w, artifacts: i >= 0 ? list.map((x, k) => (k === i ? m.artifact : x)) : [...list, m.artifact] };
          });
          break;
        case "daemon_info":
          daemonInfo = m;
          accountSubs.forEach((f) => f());
          break;
        case "accounts":
          accounts = m.accounts;
          accountsLoaded = true;
          accountSubs.forEach((f) => f());
          break;
        case "mcp_state":
          setMcp(m);
          break;
        case "cli_status":
          setSetup({ cli: { claude: m.claude ?? null, codex: m.codex ?? null } });
          break;
        case "logins_found":
          setSetup({ logins: m.found });
          break;
        case "setup_panel":
          setSetup({ panel: { id: m.panel, title: m.title } });
          break;
        case "browse_req":
          agentBrowse(m.panel, m.op, m.args).then(
            (result) => send({ t: "browse_result", id: m.id, ok: true, result }),
            (e) => send({ t: "browse_result", id: m.id, ok: false, error: String(e) }),
          );
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
          addDelta(m.panel, m.id, m.text);
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
          wsUpdate(m.panel, (w) => ({ ...w, round: { active: m.active, waiting: m.waiting } }));
          break;
        case "ws_msg_removed":
          wsUpdate(m.panel, (w) => ({ ...w, messages: w.messages.filter((x) => x.id !== m.id) }));
          break;
        case "doc":
          docHtml.set(`${m.session}/${m.doc}`, m.html ?? null);
          docSubs.forEach((f) => f());
          break;
        case "files":
          projectFiles.set(m.panel, m.files);
          fileSubs.forEach((f) => f());
          break;
        case "file":
          fileContent.set(`${m.panel}/${m.path}`, m.file);
          fileSubs.forEach((f) => f());
          break;
        case "files_granted":
          grantWaiters.get(m.panel)?.(m.paths);
          grantWaiters.delete(m.panel);
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
    const r = await open({ directory: true, multiple: false, title: t("folder.pick") });
    return typeof r === "string" ? r : null;
  }
  return window.prompt(t("folder.prompt"), "") || null;
}

// ---------- workspaces (group chats) ----------
const workspaces = new Map<string, Workspace>();
const wsSubs = new Map<string, Set<() => void>>();

function wsChanged(panel: string) {
  wsSubs.get(panel)?.forEach((f) => f());
}
// Streamed text comes a few characters at a time, dozens of times a second. Each change redraws
// the chat, so they are gathered and applied together, at most every DELTA_MS (anything else
// from the daemon applies them first, to keep the order).
const DELTA_MS = 50;
const deltas = new Map<string, Map<string, string>>();
let deltaTimer: ReturnType<typeof setTimeout> | null = null;

function addDelta(panel: string, id: string, text: string) {
  let p = deltas.get(panel);
  if (!p) deltas.set(panel, (p = new Map()));
  p.set(id, (p.get(id) ?? "") + text);
  deltaTimer ??= setTimeout(flushDeltas, DELTA_MS);
}

function flushDeltas() {
  if (deltaTimer) clearTimeout(deltaTimer);
  deltaTimer = null;
  for (const [panel, texts] of deltas)
    wsUpdate(panel, (w) => ({
      ...w,
      messages: w.messages.map((x) => (x.kind === "text" && texts.has(x.id) ? { ...x, text: x.text + texts.get(x.id) } : x)),
    }));
  deltas.clear();
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
let toastMsg: { id: number; text: string; ok: boolean } | null = null;
const toastSubs = new Set<() => void>();
/** A short note at the bottom of the window: an error by default, `ok` for a confirmation. */
export function toast(text: string, ok = false) {
  const id = Date.now();
  toastMsg = { id, text, ok };
  toastSubs.forEach((f) => f());
  setTimeout(() => {
    if (toastMsg?.id === id) {
      toastMsg = null;
      toastSubs.forEach((f) => f());
    }
  }, ok ? 1800 : 5000);
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
// Project files (📎 picker) per panel, and the content of files open in viewers.
const projectFiles = new Map<string, string[]>();
const fileContent = new Map<string, FileContent>();
const fileSubs = new Set<() => void>();
export interface FileContent {
  text?: string;
  image?: string;
  error?: string;
  size: number;
}

const grantWaiters = new Map<string, (paths: string[]) => void>();

/**
 * Pick files anywhere on the computer (the system's own dialog) and let this chat use them:
 * picking is the go-ahead for its agents to read them. Resolves to the accepted paths.
 */
export async function pickComputerFiles(panel: string): Promise<string[]> {
  let paths: string[] = [];
  if (isTauri) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: true, directory: false, title: t("file.fromComputer") });
    paths = Array.isArray(r) ? r : typeof r === "string" ? [r] : [];
  } else {
    const p = window.prompt(t("file.fromComputerPrompt"), "");
    paths = p ? [p] : [];
  }
  return grantComputerFiles(panel, paths);
}

/** Let this chat use files from anywhere on the computer (picked, or dragged in). Resolves to the accepted paths. */
export function grantComputerFiles(panel: string, paths: string[]): Promise<string[]> {
  if (!paths.length) return Promise.resolve([]);
  return new Promise((resolve) => {
    grantWaiters.set(panel, resolve);
    send({ t: "files_grant", panel, paths });
  });
}

/** A picture pasted into the composer: the daemon keeps it and lets this chat use it. Resolves to its path. */
export async function pasteImage(panel: string, blob: Blob): Promise<string | null> {
  const data = await new Promise<string>((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result).split(",")[1] ?? "");
    r.onerror = () => reject(r.error);
    r.readAsDataURL(blob);
  });
  return new Promise((resolve) => {
    grantWaiters.set(panel, (paths) => resolve(paths[0] ?? null));
    send({ t: "image_paste", panel, data, mime: blob.type });
  });
}

/** The project's files for a chat panel, asked for when `active` turns on; undefined while loading. */
export function useProjectFiles(panel: string, active: boolean): string[] | undefined {
  useEffect(() => {
    if (active) send({ t: "files_list", panel });
  }, [panel, active]);
  return useSyncExternalStore(
    (f) => (fileSubs.add(f), () => fileSubs.delete(f)),
    () => projectFiles.get(panel),
  );
}

/** A project file's content for the viewer; `rev` asks again (reload). */
export function useProjectFile(panel: string, path: string, rev: number): FileContent | undefined {
  useEffect(() => {
    send({ t: "file_get", panel, path });
  }, [panel, path, rev]);
  return useSyncExternalStore(
    (f) => (fileSubs.add(f), () => fileSubs.delete(f)),
    () => fileContent.get(`${panel}/${path}`),
  );
}

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

// A chat's add-agent form or its settings, asked for from outside the chat (a panel's menu).
type ChatAsk = "addAgent" | "settings";
const chatListeners = new Map<string, (what: ChatAsk) => void>();
export function onChatRequest(panel: string, f: (what: ChatAsk) => void) {
  chatListeners.set(panel, f);
  return () => {
    if (chatListeners.get(panel) === f) chatListeners.delete(panel);
  };
}
/** Focus the chat panel and open that there (once it is on screen). */
function requestInChat(panel: string, what: ChatAsk) {
  send({ t: "panel_focus", panel });
  let tries = 0;
  const tick = () => {
    const f = chatListeners.get(panel);
    if (f) f(what);
    else if (tries++ < 20) setTimeout(tick, 50);
  };
  tick();
}
export const requestAddAgent = (panel: string) => requestInChat(panel, "addAgent");
export const requestChatSettings = (panel: string) => requestInChat(panel, "settings");

// ---------- accounts ----------
/** The human's MCP servers (keys hidden), their last connection test, and what the CLIs have. */
export interface McpServer {
  command?: string | null;
  args?: string[];
  /** Values are never sent here: `null` means "set, unchanged". */
  env?: Record<string, string | null>;
  url?: string | null;
  headers?: Record<string, string | null>;
  sse?: boolean;
  from?: string | null;
}
export interface McpStatus {
  state: "testing" | "ok" | "failed";
  tools?: number;
  error?: string;
  at: number;
}
export interface McpFound {
  name: string;
  from: string;
  server: McpServer;
  state: "new" | "same" | "changed";
}
export interface McpState {
  servers: Record<string, McpServer>;
  status: Record<string, McpStatus>;
  found: McpFound[];
}
let mcp: McpState | null = null;
const mcpSubs = new Set<() => void>();
function setMcp(m: McpState) {
  mcp = { servers: m.servers, status: m.status, found: m.found };
  mcpSubs.forEach((f) => f());
}
/** The MCP list, asked for once when first used. */
export function useMcp(): McpState | null {
  useEffect(() => send({ t: "mcp_list" }), []);
  return useSyncExternalStore(
    (f) => (mcpSubs.add(f), () => mcpSubs.delete(f)),
    () => mcp,
  );
}

let accounts: Account[] = [];
let accountsLoaded = false;
const accountSubs = new Set<() => void>();

/** Whether the daemon has sent the account list yet (an empty list then means "none"). */
export function useAccountsLoaded(): boolean {
  return useSyncExternalStore(
    (f) => (accountSubs.add(f), () => accountSubs.delete(f)),
    () => accountsLoaded,
  );
}

export interface CliInfo {
  path: string;
  version: string;
}
/** What the setup screens know: installed CLIs, logins found on this computer, the open setup terminal. */
export interface SetupState {
  cli: { claude: CliInfo | null; codex: CliInfo | null } | null;
  logins: { kind: AgentKind; email?: string | null; plan?: string | null }[] | null;
  panel: { id: string; title: string } | null;
}
let setup: SetupState = { cli: null, logins: null, panel: null };
const setupSubs = new Set<() => void>();
export function setSetup(p: Partial<SetupState>) {
  setup = { ...setup, ...p };
  setupSubs.forEach((f) => f());
}
/** Look for logins the CLIs already have; `logins` is null until the answer comes (it takes seconds). */
export function detectLogins() {
  setSetup({ logins: null });
  send({ t: "logins_detect" });
}
export function useSetup(): SetupState {
  return useSyncExternalStore(
    (f) => (setupSubs.add(f), () => setupSubs.delete(f)),
    () => setup,
  );
}

export function useAccounts(): Account[] {
  return useSyncExternalStore(
    (f) => (accountSubs.add(f), () => accountSubs.delete(f)),
    () => accounts,
  );
}

/** The account name to show next to an agent, or "" for the default account. */
export function accountName(list: Account[], m: { account?: string | null }) {
  const a = m.account ? list.find((x) => x.id === m.account) : undefined;
  return a ? a.name || a.email || "" : "";
}

export interface DaemonInfo {
  version: string;
  protocol: number;
  pid: number;
  home: string;
  exe: string | null;
  started: number;
}
let daemonInfo: DaemonInfo | null = null;

/** The daemon's version and whereabouts (asks for it when first used). */
export function useDaemonInfo(): DaemonInfo | null {
  useEffect(() => send({ t: "daemon_info" }), []);
  return useSyncExternalStore(
    (f) => (accountSubs.add(f), () => accountSubs.delete(f)),
    () => daemonInfo,
  );
}
