// Zooming the whole app (⌘= ⌘- ⌘0, Ctrl on Linux), like a browser page; the level is remembered.
import { toast } from "./client";
import { t } from "./i18n";

const KEY = "recursion.zoom";
const LEVELS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
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

/** Put back the remembered level at start. */
export function restoreZoom() {
  if (level !== 1) void apply();
}

/** One step in (+1) or out (-1), or back to 100% (0). */
export function zoom(step: -1 | 0 | 1) {
  if (!desktop) return;
  const i = LEVELS.indexOf(level);
  level = step === 0 ? 1 : LEVELS[Math.max(0, Math.min(LEVELS.length - 1, i + step))];
  try {
    localStorage.setItem(KEY, String(level));
  } catch {}
  void apply();
  toast(t("zoom.level", { pct: Math.round(level * 100) }), true);
}
