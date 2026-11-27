// Moving a panel by dragging its header, the way a tiling window manager does: the panel itself
// shrinks and follows the pointer, the panel it is over lights up, and letting go there swaps the
// two. Only transforms move while dragging (no layout), so terminals don't redraw at every step;
// on the drop both panels glide into their new places. Dropped on the window tabs instead, it
// leaves its window: onto another tab it joins that window, onto the empty part of the tab bar it
// becomes a window of its own.
import { useLayoutEffect, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { send } from "./client";
import { t } from "./i18n";
import { shield } from "./Layout";
import type { Node } from "./types";

type Box = { left: number; top: number; width: number; height: number };
type Target = { kind: "swap"; panel: string; box: Box } | { kind: "window"; window: string } | { kind: "new" };
// `solo`: the only panel of its window (it can join another window, but not get a window of its own).
type Drag = { panel: string; solo: boolean; home: Box; target: Target | null };

let drag: Drag | null = null;
// The panel lifted out of the layout: from the pickup until it has landed (after the drop, the
// new layout takes a moment to come back from the daemon).
let lifted: string | null = null;
const listeners = new Set<() => void>();
const subscribe = (l: () => void) => (listeners.add(l), () => listeners.delete(l));
const set = (d: Drag | null, l: string | null = lifted) => {
  drag = d;
  lifted = l;
  listeners.forEach((f) => f());
};

// Where every panel was at the drop (the dragged one where it was let go), for the glide; and the
// element still held at that spot until the new layout arrives.
let before: { at: number; rects: Map<string, DOMRect>; held: HTMLElement } | null = null;

const panelEl = (id: string) => document.querySelector<HTMLElement>(`.layout [data-panel="${CSS.escape(id)}"]`);
const isControl = (el: EventTarget | null) => el instanceof Element && !!el.closest("button, input, textarea, select, a, [contenteditable], .menu");
const boxOf = (r: DOMRect): Box => ({ left: r.left, top: r.top, width: r.width, height: r.height });

function release(el: HTMLElement) {
  el.style.translate = el.style.scale = el.style.transformOrigin = "";
  set(drag, null);
}

const inside = (r: DOMRect, x: number, y: number) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;

function targetAt(x: number, y: number, from: string, solo: boolean): Target | null {
  const bar = document.querySelector<HTMLElement>(".main > .tabs");
  if (bar && inside(bar.getBoundingClientRect(), x, y)) {
    for (const tab of bar.querySelectorAll<HTMLElement>(".tab[data-window]:not(.on)")) {
      if (inside(tab.getBoundingClientRect(), x, y)) return { kind: "window", window: tab.dataset.window! };
    }
    return solo ? null : { kind: "new" };
  }
  if (solo) return null;
  for (const el of document.querySelectorAll<HTMLElement>(".layout [data-panel]")) {
    if (el.dataset.panel === from) continue;
    const r = el.getBoundingClientRect();
    if (inside(r, x, y)) return { kind: "swap", panel: el.dataset.panel!, box: boxOf(r) };
  }
  return null;
}

/** Whether a panel can be picked up: not while it fills the window for its drawer, and not the
 *  only panel of the only window (it would have nowhere to go). */
export const movable = (zoomed: boolean, solo: boolean, windows: number) => !zoomed && (!solo || windows > 1);

/** Pointer down on a panel's header: past a few pixels of movement it becomes a drag. */
export function startPanelDrag(e: React.PointerEvent, panel: string, solo: boolean, can: boolean) {
  if (!can || e.button !== 0 || isControl(e.target)) return;
  const x0 = e.clientX;
  const y0 = e.clientY;
  let el: HTMLElement | null = null;
  let undo: (() => void)[] = [];
  let scale = 1;
  let oy = 0;
  const move = (ev: PointerEvent) => {
    if (!el) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      el = panelEl(panel);
      if (!el) return;
      const r = el.getBoundingClientRect();
      // Shrink around the point that was grabbed, so it stays under the pointer.
      el.style.transformOrigin = `${x0 - r.left}px ${y0 - r.top}px`;
      scale = Math.min(0.6, 480 / r.width, 360 / r.height);
      oy = y0 - r.top;
      el.style.scale = String(scale);
      undo = [shield("grabbing"), overlay()];
      document.body.classList.add("panel-dragging");
      set({ panel, solo, home: boxOf(r), target: null }, panel);
    }
    // Over the tabs it gets small and hangs below them, so the tab it would go to stays in sight.
    const bar = document.querySelector<HTMLElement>(".main > .tabs")?.getBoundingClientRect();
    const onBar = !!bar && ev.clientY <= bar.bottom;
    const s = onBar ? Math.min(scale, 0.3) : scale;
    el.style.scale = String(s);
    const dy = onBar ? Math.max(ev.clientY - y0, bar!.bottom + 6 - y0 + oy * s) : ev.clientY - y0;
    el.style.translate = `${ev.clientX - x0}px ${dy}px`;
    const target = targetAt(ev.clientX, ev.clientY, panel, solo);
    if (key(target) !== key(drag?.target ?? null)) set({ ...drag!, target });
  };
  const end = (drop: boolean) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("keydown", esc, true);
    window.removeEventListener("blur", cancel);
    undo.forEach((u) => u());
    document.body.classList.remove("panel-dragging");
    const d = drag;
    set(null);
    if (!el || !d) return;
    if (drop && d.target) {
      const rects = new Map<string, DOMRect>();
      document.querySelectorAll<HTMLElement>(".layout [data-panel]").forEach((p) => rects.set(p.dataset.panel!, p.getBoundingClientRect()));
      before = { at: performance.now(), rects, held: el };
      const to = d.target;
      if (to.kind === "swap") send({ t: "panel_move", panel, target: to.panel, side: "center" });
      else send({ t: "panel_to_window", panel, window: to.kind === "window" ? to.window : null });
      // If the new layout never comes (the daemon refused), it goes back.
      const held = el;
      setTimeout(() => {
        if (before?.held !== held) return;
        before = null;
        goHome(held);
      }, 1500);
    } else goHome(el);
  };
  const up = () => end(true);
  const cancel = () => end(false);
  // Escape puts it back, and doesn't reach the terminal underneath.
  const esc = (ev: KeyboardEvent) => {
    if (ev.key !== "Escape" || !drag) return;
    ev.preventDefault();
    ev.stopPropagation();
    end(false);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("keydown", esc, true);
  window.addEventListener("blur", cancel);
}

const key = (t: Target | null) => (!t ? "" : t.kind === "swap" ? `p:${t.panel}` : t.kind === "window" ? `w:${t.window}` : "new");

function goHome(el: HTMLElement) {
  const from = el.getBoundingClientRect();
  release(el);
  glide(el, from, el.getBoundingClientRect());
}

function glide(el: HTMLElement, from: DOMRect, to: DOMRect) {
  if (Math.abs(from.left - to.left) < 1 && Math.abs(from.top - to.top) < 1 && Math.abs(from.width - to.width) < 1 && Math.abs(from.height - to.height) < 1) return;
  el.animate(
    [
      { transformOrigin: "0 0", transform: `translate(${from.left - to.left}px, ${from.top - to.top}px) scale(${from.width / to.width}, ${from.height / to.height})` },
      { transformOrigin: "0 0", transform: "none" },
    ],
    { duration: 220, easing: "cubic-bezier(.2,.7,.3,1)" },
  );
}

/** The panel being dragged, if any. */
export const useDraggedPanel = () => useSyncExternalStore(subscribe, () => lifted);

/** For the tab bar while a panel is dragged: whether it may become a window of its own, and
 *  where it would go if let go now. */
export const usePanelDrag = () => useSyncExternalStore(subscribe, () => drag);

/** Where it came from and the panel it would swap with; and the glide once the layout changed. */
export function PanelDragLayer({ root }: { root: Node }) {
  const d = useSyncExternalStore(subscribe, () => drag);
  useLayoutEffect(() => {
    if (!before) return;
    const { rects, held, at } = before;
    before = null;
    release(held);
    if (performance.now() - at > 1500) return;
    document.querySelectorAll<HTMLElement>(".layout [data-panel]").forEach((el) => {
      const from = rects.get(el.dataset.panel!);
      if (from) glide(el, from, el.getBoundingClientRect());
    });
  }, [root]);
  if (!d) return null;
  return createPortal(
    <>
      <div className="panel-home" style={d.home} />
      {d.target?.kind === "swap" && (
        <div className="panel-drop-zone" style={d.target.box}>
          <span>{t("panel.swap")}</span>
        </div>
      )}
    </>,
    document.body,
  );
}

type Side = "left" | "right" | "top" | "bottom";

/** Pointer down on a tab of another window: dragged into the layout on screen, that window's
 *  panels join this one, beside the panel let go over (on the side of it nearest the pointer).
 *  The window they left goes away. A plain click still just switches to it. */
export function startTabDrag(e: React.PointerEvent, panels: string[], into: string, label: string) {
  if (e.button !== 0 || isControl(e.target) || !panels.length) return;
  const x0 = e.clientX;
  const y0 = e.clientY;
  let ghost: HTMLDivElement | null = null;
  let zone: HTMLDivElement | null = null;
  let undo: (() => void)[] = [];
  let at: { panel: string; side: Side } | null = null;
  const place = (x: number, y: number) => {
    at = null;
    for (const el of document.querySelectorAll<HTMLElement>(".layout [data-panel]")) {
      const r = el.getBoundingClientRect();
      if (!inside(r, x, y)) continue;
      // The edge nearest the pointer, as a share of the panel's size.
      const d = { left: (x - r.left) / r.width, right: (r.right - x) / r.width, top: (y - r.top) / r.height, bottom: (r.bottom - y) / r.height };
      const side = (Object.keys(d) as Side[]).reduce((a, b) => (d[b] < d[a] ? b : a));
      at = { panel: el.dataset.panel!, side };
      const half = side === "left" || side === "right" ? { width: r.width / 2, height: r.height } : { width: r.width, height: r.height / 2 };
      Object.assign(zone!.style, { left: `${side === "right" ? r.left + r.width / 2 : r.left}px`, top: `${side === "bottom" ? r.top + r.height / 2 : r.top}px`, width: `${half.width}px`, height: `${half.height}px`, display: "" });
      return;
    }
    zone!.style.display = "none";
  };
  const move = (ev: PointerEvent) => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      ghost = document.createElement("div");
      ghost.className = "ref-ghost";
      ghost.textContent = label;
      zone = document.createElement("div");
      zone.className = "panel-drop-zone";
      zone.style.display = "none";
      document.body.append(ghost, zone);
      undo = [shield("grabbing"), overlay()];
    }
    ghost.style.translate = `${ev.clientX + 12}px ${ev.clientY + 10}px`;
    place(ev.clientX, ev.clientY);
  };
  const end = (drop: boolean) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("blur", cancel);
    if (!ghost) return;
    ghost.remove();
    zone?.remove();
    undo.forEach((u) => u());
    // The drag ends in a click on the tab: not a "switch to it".
    const eat = (c: MouseEvent) => (c.stopPropagation(), c.preventDefault());
    window.addEventListener("click", eat, true);
    setTimeout(() => window.removeEventListener("click", eat, true));
    const to = at as { panel: string; side: Side } | null;
    if (!drop || !to) return;
    for (const p of panels) send({ t: "panel_to_window", panel: p, window: into });
    if (panels.length === 1) send({ t: "panel_move", panel: panels[0], target: to.panel, side: to.side });
  };
  const up = () => end(true);
  const cancel = () => end(false);
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("blur", cancel);
}
