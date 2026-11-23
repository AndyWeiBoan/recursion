// Files dragged in from the file manager. The window gets them (with their real paths) as a
// whole, so the panel under the pointer is the one they go to: a chat attaches them, a terminal
// types their paths.
import { useSyncExternalStore } from "react";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const takers = new Map<string, (paths: string[]) => void>();
let over: string | null = null;
const listeners = new Set<() => void>();

function setOver(panel: string | null) {
  if (panel === over) return;
  over = panel;
  listeners.forEach((l) => l());
}

/** Let a panel take dropped files while it's on screen; returns the undo. */
export function takeDrops(panel: string, take: (paths: string[]) => void) {
  takers.set(panel, take);
  return () => {
    if (takers.get(panel) === take) takers.delete(panel);
  };
}

/** The panel files are being dragged over right now, if it takes them. */
export function useDropOver(): string | null {
  return useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => over,
  );
}

let listening = false;
/** Start listening (once, at startup). Outside the app (the browser dev page) there is nothing to do. */
export async function listenForDrops() {
  if (!isTauri || listening) return;
  listening = true;
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  const view = getCurrentWebview();
  // The event's position is in device pixels; the page's are CSS pixels (and the app may be zoomed).
  let scale = window.devicePixelRatio;
  const panelAt = (p: { x: number; y: number }) => {
    const el = document.elementFromPoint(p.x / scale, p.y / scale);
    const id = (el?.closest("[data-panel]") as HTMLElement | null)?.dataset.panel ?? null;
    return id && takers.has(id) ? id : null;
  };
  await view.onDragDropEvent(async (e) => {
    const d = e.payload;
    if (d.type === "enter") scale = (await view.size()).width / window.innerWidth || window.devicePixelRatio;
    if (d.type === "enter" || d.type === "over") setOver(panelAt(d.position));
    else if (d.type === "drop") {
      const panel = panelAt(d.position);
      setOver(null);
      if (panel && d.paths.length) takers.get(panel)?.(d.paths);
    } else setOver(null);
  });
}
