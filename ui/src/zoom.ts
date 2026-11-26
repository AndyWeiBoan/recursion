// Zooming the whole app (⌘= ⌘- ⌘0, Ctrl on Linux), like a browser page; the level is remembered.
import { toast } from "./client";
import { t } from "./i18n";

const KEY = "recursion.zoom";
export const LEVELS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];

/** `level` moved one step in (+1) or out (-1), or back to 100% (0). */
export const stepLevel = (level: number, step: -1 | 0 | 1) => {
  if (step === 0) return 1;
  const i = LEVELS.indexOf(level);
  return LEVELS[Math.max(0, Math.min(LEVELS.length - 1, (i < 0 ? LEVELS.indexOf(1) : i) + step))];
};
const desktop = "__TAURI_INTERNALS__" in window;

let level = 1;
try {
  const saved = Number(localStorage.getItem(KEY));
  if (LEVELS.includes(saved)) level = saved;
} catch {}

async function apply() {
  // Outside the app (the browser dev page), the browser's own zoom does this.
  if (!desktop) return;
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  await getCurrentWebview().setZoom(level);
}

/** The app's zoom: page pixels times this are window pixels (what native views are placed in). */
export const zoomLevel = () => (desktop ? level : 1);

/** ⌘= ⌘- ⌘0 the app caught natively (the View menu on macOS, a browser panel's page on Linux):
 *  `panel` is the browser panel that had the keyboard, whose page zooms instead of the app. */
export async function listenZoomKeys(zoomPage: (panel: string, step: -1 | 0 | 1) => void) {
  if (!desktop) return;
  const { listen } = await import("@tauri-apps/api/event");
  await listen<{ panel: string | null; step: -1 | 0 | 1 }>("zoom_key", ({ payload }) => (payload.panel ? zoomPage(payload.panel, payload.step) : zoom(payload.step)));
}

/** Put back the remembered level at start. */
export function restoreZoom() {
  if (level !== 1) void apply();
}

/** One step in (+1) or out (-1), or back to 100% (0). */
export function zoom(step: -1 | 0 | 1) {
  if (!desktop) return;
  level = stepLevel(level, step);
  try {
    localStorage.setItem(KEY, String(level));
  } catch {}
  void apply();
  toast(t("zoom.level", { pct: Math.round(level * 100) }), true);
}
