// Putting things in another order by dragging them: projects, windows, panels and agents in the
// sidebar, windows in the tab bar. The one picked up follows the pointer along its list (and no
// further), the others slide aside to show where it goes, and letting go sends the new place.
// Until the new order comes back from the daemon they stay where they were shown, so nothing
// jumps. Only a press that moves is a drag: a click, double-click or right-click is left alone.
import { shield } from "./Layout";

type Sort = {
  /** What moves (a row, or the block it heads). */
  item: HTMLElement;
  /** Its list, in order, itself included. */
  items: () => HTMLElement[];
  axis: "x" | "y";
  /** Let go at another place: its index in the list without it. */
  onDrop: (to: number) => void;
  /** Called while dragging; true takes the drag elsewhere (the pointer left the list for a place
   *  that means something else), and this one ends. */
  away?: (ev: PointerEvent) => boolean;
};

const SLIDE = "transform .16s cubic-bezier(.2, .8, .2, 1)";

/** Presses on these are theirs, not the start of a drag. */
const own = (t: EventTarget | null) => !!(t as HTMLElement)?.closest?.("button, input, textarea, select, a, [contenteditable]");

export function startSort(e: React.PointerEvent, o: Sort) {
  if (e.button !== 0 || own(e.target)) return;
  const x0 = e.clientX;
  const y0 = e.clientY;
  const y = o.axis === "y";
  let started = false;
  let list: HTMLElement[] = [];
  let from = 0;
  let to = 0;
  let rects: DOMRect[] = [];
  let step = 0;
  let undo: (() => void) | null = null;
  const start = () => {
    started = true;
    list = o.items();
    from = list.indexOf(o.item);
    to = from;
    rects = list.map((el) => el.getBoundingClientRect());
    // What the others move by: its size and the space after it.
    const r = rects[from];
    const next = rects[from + 1] ?? rects[from - 1];
    const gap = next ? Math.max(0, y ? (next.top > r.top ? next.top - r.bottom : r.top - next.bottom) : next.left > r.left ? next.left - r.right : r.left - next.right) : 0;
    step = (y ? r.height : r.width) + gap;
    list.forEach((el) => (el.style.transition = el === o.item ? "none" : SLIDE));
    o.item.classList.add("sorting");
    undo = shield("grabbing");
  };
  const move = (ev: PointerEvent) => {
    if (!started) {
      if (Math.abs(y ? ev.clientY - y0 : ev.clientX - x0) < 5) return;
      start();
      if (from < 0) return end(false);
    }
    if (o.away?.(ev)) return end(false, true);
    // Along its list only, from the first place to the last.
    const first = rects[0];
    const last = rects[rects.length - 1];
    const r = rects[from];
    const min = y ? first.top - r.top : first.left - r.left;
    const max = y ? last.bottom - r.bottom : last.right - r.right;
    const d = Math.max(min, Math.min(max, y ? ev.clientY - y0 : ev.clientX - x0));
    o.item.style.transform = y ? `translateY(${d}px)` : `translateX(${d}px)`;
    // Going up (left), its leading edge past another's middle takes that one's place; going
    // down (right), its trailing edge. Edges, not middles: a tall block (a project with its
    // windows open) still gets past a short one.
    const lead = (y ? r.top : r.left) + d;
    const trail = (y ? r.bottom : r.right) + d;
    to = from;
    rects.forEach((q, i) => {
      const c = y ? q.top + q.height / 2 : q.left + q.width / 2;
      if (i < from && lead < c) to = Math.min(to, i);
      if (i > from && trail > c) to = Math.max(to, i);
    });
    list.forEach((el, i) => {
      if (el === o.item) return;
      const shift = i > from && i <= to ? -step : i < from && i >= to ? step : 0;
      el.style.transform = shift ? (y ? `translateY(${shift}px)` : `translateX(${shift}px)`) : "";
    });
  };
  const clear = (animate: boolean) => {
    list.forEach((el) => {
      el.style.transition = animate ? SLIDE : "none";
      el.style.transform = "";
    });
    o.item.classList.remove("sorting");
    requestAnimationFrame(() => list.forEach((el) => (el.style.transition = "")));
  };
  const end = (drop: boolean, handedOff = false) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("keydown", key, true);
    undo?.();
    if (!started) return;
    // The click that ends a drag isn't a click on the row. It comes right after the release, if
    // at all (not when let go over another element), so only that moment's is eaten, not the next.
    if (!handedOff) {
      const eat = (c: MouseEvent) => (c.stopPropagation(), c.preventDefault());
      window.addEventListener("click", eat, { capture: true, once: true });
      setTimeout(() => window.removeEventListener("click", eat, { capture: true }), 0);
    }
    if (!drop || to === from) return clear(true);
    o.onDrop(to);
    // Shown where it was dropped until the list comes back in its new order (or a moment passes).
    const item = o.item;
    const t0 = performance.now();
    const wait = () => {
      const now = o.items();
      if (now.indexOf(item) === to || !item.isConnected || performance.now() - t0 > 800) return clear(false);
      requestAnimationFrame(wait);
    };
    requestAnimationFrame(wait);
  };
  const up = () => end(true);
  const key = (ev: KeyboardEvent) => {
    if (ev.key === "Escape" && started) (ev.stopPropagation(), end(false));
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("keydown", key, true);
}
