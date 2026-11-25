// Native browser webviews (one per browser panel), positioned by the panel that owns them.
// They are OS views stacked above our HTML, so anything that pops over a panel (menus, the
// kind picker) must hide them while it is open: see `overlay()`.
import { t } from "./i18n";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const nativeBrowser = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface BrowserEvent {
  panel: string;
  url: string | null;
  title: string | null;
  loading: boolean | null;
}

interface Entry {
  /** Last URL the webview itself reported (or the one it was created with). */
  url: string;
  /** Mounted panel's "put yourself back" callback; null while the panel is off screen. */
  reshow: (() => void) | null;
}

const entries = new Map<string, Entry>();
const listeners = new Map<string, (e: BrowserEvent) => void>();
let overlays = 0;

if (nativeBrowser) {
  listen<BrowserEvent>("browser", ({ payload }) => {
    const en = entries.get(payload.panel);
    if (en && payload.url) en.url = payload.url;
    listeners.get(payload.panel)?.(payload);
  });
}

export type Bounds = { x: number; y: number; w: number; h: number };

// One queue for all calls: a show that creates the webview must finish before anything else touches it.
let queue: Promise<unknown> = Promise.resolve();
const call = (cmd: string, args: Record<string, unknown>) =>
  (queue = queue.then(() => invoke(cmd, args)).catch((e) => console.warn(`[browser] ${cmd}:`, e)));

export function onBrowserEvent(panel: string, fn: (e: BrowserEvent) => void) {
  listeners.set(panel, fn);
  return () => {
    if (listeners.get(panel) === fn) listeners.delete(panel);
  };
}

/** Show the panel's webview at `b`; creates it on first use, navigates if `url` changed. */
export function showBrowser(panel: string, url: string, b: Bounds, reshow: () => void) {
  const en = entries.get(panel);
  if (!en) {
    entries.set(panel, { url, reshow });
    if (overlays === 0) call("browser_show", { panel, url, bounds: b });
    return;
  }
  en.reshow = reshow;
  if (en.url !== url) {
    en.url = url;
    call("browser_navigate", { panel, url });
  }
  if (overlays === 0) call("browser_show", { panel, url, bounds: b });
}

export function moveBrowser(panel: string, b: Bounds) {
  if (entries.has(panel)) call("browser_bounds", { panel, bounds: b });
}

export function hideBrowser(panel: string) {
  const en = entries.get(panel);
  if (!en) return;
  en.reshow = null;
  call("browser_hide", { panel });
}

export function browserGo(panel: string, action: "back" | "forward" | "reload" | "focus") {
  if (entries.has(panel)) call("browser_go", { panel, action });
}

/** One step of an agent driving its browser panel (see `browser_agent` in the app). The webview
 *  is created hidden if the panel has never been on screen; `args.url` is the panel's page. */
export async function agentBrowse(panel: string, op: string, args: { url?: string } & Record<string, unknown>): Promise<unknown> {
  if (!nativeBrowser) throw new Error(t("browser.noNative"));
  const url = String(args.url ?? "");
  const en = entries.get(panel);
  // Whoever shows the panel next must not navigate it again: the agent's step already did.
  if (en) en.url = url;
  else entries.set(panel, { url, reshow: null });
  const ensured = queue.then(() => invoke("browser_ensure", { panel, url }));
  queue = ensured.catch(() => {});
  await ensured;
  return invoke("browser_agent", { panel, op, args });
}

/** Close webviews whose panel is gone or is no longer a browser. */
export function pruneBrowsers(alive: Set<string>) {
  for (const id of [...entries.keys()]) {
    if (alive.has(id)) continue;
    entries.delete(id);
    call("browser_close", { panel: id });
  }
}

/** Something is drawn over the panels: hide every webview until the returned function runs. */
export function overlay(): () => void {
  if (!nativeBrowser) return () => {};
  if (overlays++ === 0) for (const [id, en] of entries) if (en.reshow) call("browser_hide", { panel: id });
  let done = false;
  return () => {
    if (done) return;
    done = true;
    if (--overlays === 0) for (const en of entries.values()) en.reshow?.();
  };
}

/** The project file a panel shows in the viewer (`file:<path>`), or null. A real `file://` URL is a web page. */
export function viewerPath(url: string | null | undefined): string | null {
  return url?.startsWith("file:") && !url.startsWith("file://") ? url.slice(5) : null;
}

/** What the user typed → a URL: keeps schemes, http for localhost, https for hosts, else a search. */
export function normalizeUrl(input: string): string {
  const s = input.trim();
  if (/^[a-z][a-z0-9+.-]*:/i.test(s) && !/^localhost:\d/i.test(s)) return s;
  if (s.startsWith("/")) return `file://${s}`;
  if (/^(localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0)(:\d+)?(\/|$)/i.test(s)) return `http://${s}`;
  if (!/\s/.test(s) && /^[^/]+\.[a-z]{2,}(:\d+)?(\/|$)/i.test(s)) return `https://${s}`;
  return `https://duckduckgo.com/?q=${encodeURIComponent(s)}`;
}
