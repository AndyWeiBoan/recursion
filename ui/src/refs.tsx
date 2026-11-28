// References to the chat's 產出 in a message. An item of the drawer is dragged into the box (or
// added with its ↳ button); it goes with the message as an id, the agents are told what it is and
// which tool reads it, and the chip in the bubble opens it. Pictures are attached as files.
import { overlay } from "./browsers";
import { send, toast, useWorkspace } from "./client";
import { t as tr } from "./i18n";
import { openThreads, unzoom } from "./threadsUI";
import { showTodoList } from "./Todos";
import type { OutputRef, Workspace } from "./types";
import { Icon, type IconName } from "./icons";

/** What can go in the box from the drawer. */
export type Payload = { ref: OutputRef } | { image: string };

const takers = new Map<string, (p: Payload) => void>();

/** Let a chat's box take references while it's on screen; returns the undo. */
export function takeRefs(panel: string, take: (p: Payload) => void) {
  takers.set(panel, take);
  return () => {
    if (takers.get(panel) === take) takers.delete(panel);
  };
}

export function addToChat(panel: string, p: Payload) {
  takers.get(panel)?.(p);
}

export const REF_ICON: Record<OutputRef["kind"], IconName> = { doc: "doc", diagram: "diagram", page: "page", file: "file", poll: "poll", todo: "todo", thread: "thread" };

/** Its current title, or that it is gone (taken off the 產出 list, or deleted). */
function look(ws: Workspace | null, r: OutputRef): { title: string; gone: boolean } {
  if (!ws) return { title: r.title, gone: false };
  const found = (title: string | undefined) => (title === undefined ? { title: r.title, gone: true } : { title: title || r.title, gone: false });
  switch (r.kind) {
    case "doc":
    case "diagram":
    case "page":
    case "file":
      return found((ws.artifacts ?? []).find((a) => a.id === r.id && !a.removed)?.title);
    case "poll":
      return found((ws.polls ?? []).find((p) => p.id === r.id && !p.removed)?.question);
    case "thread":
      return found((ws.threads ?? []).find((t) => t.id === r.id && !t.removed)?.quote);
    case "todo": {
      const list = (ws.todo_lists ?? []).find((l) => l.id === r.id);
      return found(r.item ? list?.items.find((t) => t.id === r.item)?.text : list?.name);
    }
  }
}

function openRef(panel: string, r: OutputRef) {
  switch (r.kind) {
    case "doc":
    case "diagram":
    case "page":
      send({ t: "artifact_open", panel, artifact: r.id });
      return unzoom();
    case "poll":
      return openThreads(panel, { view: { kind: "poll", id: r.id }, msg: null });
    case "thread":
      return openThreads(panel, { view: { kind: "thread", id: r.id }, msg: null });
    case "todo":
      showTodoList(panel, r.id);
      return openThreads(panel, { view: { kind: "list" }, filter: "todo", msg: null });
  }
}

export function RefChip({ panel, r, onRemove }: { panel: string; r: OutputRef; onRemove?: () => void }) {
  const ws = useWorkspace(panel);
  const { title, gone } = look(ws, r);
  return (
    <span className={`file-chip ref-chip${gone ? " gone" : ""}`}>
      <button className="file-chip-open" disabled={gone} onClick={() => openRef(panel, r)} title={gone ? tr("ref.goneHint") : tr("ref.open", { title })}>
        <span className="ref-icon"><Icon name={REF_ICON[r.kind]} size={13} /></span>
        <span className="ref-title">{title.replace(/\s+/g, " ")}</span>
        {gone && <span className="ref-gone">{tr("ref.gone")}</span>}
      </button>
      {onRemove && (
        <button className="file-chip-x" onClick={onRemove} aria-label={tr("ref.remove", { title })}>
          <Icon name="close" />
        </button>
      )}
    </span>
  );
}

/** The drawer row's ↳ button: the same as dragging it into the box. */
export function QuoteButton({ panel, p }: { panel: string; p: Payload }) {
  return (
    <button className="td-goto td-quote-btn" onClick={(e) => (e.stopPropagation(), addToChat(panel, p))} title={tr("ref.add")} aria-label={tr("ref.add")}>
      <Icon name="quote" />
    </button>
  );
}

const box = (x: number, y: number) => document.elementFromPoint(x, y)?.closest<HTMLElement>(".ws-input") ?? null;
const panelOf = (el: HTMLElement) => el.closest<HTMLElement>("[data-panel]")?.dataset.panel;

/** Drag a drawer row by pointer (HTML drag and drop differs between the platforms' webviews). */
export function startRefDrag(e: React.PointerEvent, panel: string, p: Payload, label: string) {
  if (e.button !== 0 || (e.target as HTMLElement).closest("input, textarea, button:not(.td-open), .todo-grip")) return;
  const x0 = e.clientX,
    y0 = e.clientY;
  let ghost: HTMLDivElement | null = null;
  let restore: (() => void) | null = null;
  let over: HTMLElement | null = null;
  const mark = (el: HTMLElement | null) => {
    if (el === over) return;
    over?.classList.remove("ref-over", "ref-wrong");
    over = el;
    el?.classList.add(panelOf(el) === panel ? "ref-over" : "ref-wrong");
  };
  const move = (ev: PointerEvent) => {
    if (!ghost) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      ghost = document.createElement("div");
      ghost.className = "ref-ghost";
      ghost.textContent = label;
      document.body.append(ghost);
      document.body.classList.add("ref-dragging");
      restore = overlay(); // native browser panels would draw over it
      window.getSelection()?.removeAllRanges();
    }
    ghost.style.translate = `${ev.clientX + 12}px ${ev.clientY + 10}px`;
    mark(box(ev.clientX, ev.clientY));
  };
  const end = (ev: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", end);
    window.removeEventListener("pointercancel", end);
    if (!ghost) return;
    ghost.remove();
    restore?.();
    document.body.classList.remove("ref-dragging");
    mark(null);
    // The drag ends in a click on whatever it started on: not an "open".
    const eat = (c: MouseEvent) => (c.stopPropagation(), c.preventDefault());
    window.addEventListener("click", eat, true);
    setTimeout(() => window.removeEventListener("click", eat, true));
    const target = ev.type === "pointerup" ? box(ev.clientX, ev.clientY) : null;
    if (!target) return;
    if (panelOf(target) === panel) addToChat(panel, p);
    else toast(tr("ref.otherChat"));
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", end);
  window.addEventListener("pointercancel", end);
}
