// Moving a panel by dragging its header, the way a tiling window manager does: the panel itself
// shrinks and follows the pointer, the panel it is over lights up, and letting go there swaps the
// two. Only transforms move while dragging (no layout), so terminals don't redraw at every step;
// on the drop both panels glide into their new places.
import { useLayoutEffect, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { send } from "./client";
import { t } from "./i18n";
import { shield } from "./Layout";
import type { Node } from "./types";

type Box = { left: number; top: number; width: number; height: number };
type Drag = { panel: string; home: Box; target: { panel: string; box: Box } | null };

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

function targetAt(x: number, y: number, from: string): Drag["target"] {
  for (const el of document.querySelectorAll<HTMLElement>(".layout [data-panel]")) {
    if (el.dataset.panel === from) continue;
    const r = el.getBoundingClientRect();
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return { panel: el.dataset.panel!, box: boxOf(r) };
  }
  return null;
}

/** Pointer down on a panel's header: past a few pixels of movement it becomes a drag. */
export function startPanelDrag(e: React.PointerEvent, panel: string, alone: boolean) {
  if (alone || e.button !== 0 || isControl(e.target)) return;
  const x0 = e.clientX;
  const y0 = e.clientY;
  let el: HTMLElement | null = null;
  let undo: (() => void)[] = [];
  const move = (ev: PointerEvent) => {
    if (!el) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      el = panelEl(panel);
      if (!el) return;
      const r = el.getBoundingClientRect();
      // Shrink around the point that was grabbed, so it stays under the pointer.
      el.style.transformOrigin = `${x0 - r.left}px ${y0 - r.top}px`;
      el.style.scale = String(Math.min(0.6, 480 / r.width, 360 / r.height));
      undo = [shield("grabbing"), overlay()];
      document.body.classList.add("panel-dragging");
      set({ panel, home: boxOf(r), target: null }, panel);
    }
    el.style.translate = `${ev.clientX - x0}px ${ev.clientY - y0}px`;
    const target = targetAt(ev.clientX, ev.clientY, panel);
    if (target?.panel !== drag?.target?.panel) set({ ...drag!, target });
  };
  const end = (drop: boolean) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("keydown", key, true);
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
      send({ t: "panel_move", panel, target: d.target.panel, side: "center" });
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
  const key = (ev: KeyboardEvent) => {
    if (ev.key !== "Escape" || !drag) return;
    ev.preventDefault();
    ev.stopPropagation();
    end(false);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("keydown", key, true);
  window.addEventListener("blur", cancel);
}

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
      {d.target && (
        <div className="panel-drop-zone" style={d.target.box}>
          <span>{t("panel.swap")}</span>
        </div>
      )}
    </>,
    document.body,
  );
}
