// A todo item's card: its details in Markdown, floating above everything. Opened by clicking the
// item; moved by its title bar and resized by its corner grip (both remembered); the human edits it
// here, agents with `todo_note`. Links open; right-click one to copy it.
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { copyText } from "./clipboard";
import { openExternal, send, toast, useWorkspace } from "./client";
import { t as tr } from "./i18n";
import { renderMarkdown } from "./WorkspaceView";
import { Icon } from "./icons";

type Open = { panel: string; list: string; item: string };
let open: Open | null = null;
const subs = new Set<() => void>();

export function openTodoCard(o: Open | null) {
  open = o;
  subs.forEach((f) => f());
}

const useOpen = () =>
  useSyncExternalStore(
    (f) => (subs.add(f), () => subs.delete(f)),
    () => open,
  );

const KEY = "recursion.todoCard";
type Place = { x: number; y: number; w: number; h: number };
function savedPlace(): Place {
  try {
    const p = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (p && [p.x, p.y, p.w, p.h].every(Number.isFinite)) return p;
  } catch {}
  return { x: Math.max(16, window.innerWidth / 2 - 190), y: 80, w: 380, h: 300 };
}
// Not too big (a long card scrolls inside), and on screen however the window changed since.
const MAX_W = 720,
  MAX_H = 600;
const fit = (p: Place): Place => {
  const w = Math.min(Math.max(p.w, 280), MAX_W, window.innerWidth - 16);
  const h = Math.min(Math.max(p.h, 180), MAX_H, window.innerHeight - 16);
  return { w, h, x: Math.min(Math.max(p.x, 8), window.innerWidth - w - 8), y: Math.min(Math.max(p.y, 8), window.innerHeight - h - 8) };
};
function savePlace(p: Place) {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {}
}

/** The open card, if any. Mounted once, at the top of the app. */
export function TodoCardLayer() {
  const o = useOpen();
  return o ? <TodoCard key={`${o.list}/${o.item}`} {...o} /> : null;
}

function TodoCard({ panel, list, item }: Open) {
  const ws = useWorkspace(panel);
  const l = ws?.todo_lists?.find((x) => x.id === list);
  const todo = l?.items.find((x) => x.id === item);
  const note = todo?.note ?? "";
  const [place, setPlace] = useState(() => fit(savedPlace()));
  const [editing, setEditing] = useState<string | null>(null);
  const [link, setLink] = useState<{ x: number; y: number; href: string } | null>(null);
  const card = useRef<HTMLDivElement>(null);
  const close = () => openTodoCard(null);

  // Browser panels are drawn above the page: hide them while the card is up.
  useEffect(() => overlay(), []);
  // The item went away (deleted here or by someone else).
  useEffect(() => {
    if (ws && !todo) close();
  }, [ws, todo]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.isComposing) return;
      e.stopPropagation();
      if (link) setLink(null);
      else if (editing !== null) setEditing(null);
      else close();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [editing, link]);
  if (!todo || !l) return null;

  const drag = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
    e.preventDefault();
    const x0 = e.clientX - place.x,
      y0 = e.clientY - place.y;
    let last = place;
    const move = (ev: PointerEvent) => {
      last = fit({ ...place, x: ev.clientX - x0, y: ev.clientY - y0 });
      setPlace(last);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("card-dragging");
      savePlace(last);
    };
    document.body.classList.add("card-dragging");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  // Our own corner grip: CSS `resize` does nothing in macOS's WebView, whose scrolling body
  // covers the corner. Growing stops at the window's edge rather than moving the card.
  const resize = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX - place.w,
      y0 = e.clientY - place.h;
    let last = place;
    const move = (ev: PointerEvent) => {
      const w = Math.min(Math.max(ev.clientX - x0, 280), MAX_W, window.innerWidth - place.x - 8);
      const h = Math.min(Math.max(ev.clientY - y0, 180), MAX_H, window.innerHeight - place.y - 8);
      last = { ...place, w, h };
      setPlace(last);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("card-resizing");
      savePlace(last);
    };
    document.body.classList.add("card-resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  const save = () => {
    if (editing === null) return;
    send({ t: "todo_update", panel, list, id: item, note: editing });
    setEditing(null);
  };
  const copy = () => void copyText(note).then((ok) => toast(ok ? tr("todoCard.copied") : tr("code.copyFailed"), ok));

  return createPortal(
    <div ref={card} className="todo-card" style={{ left: place.x, top: place.y, width: place.w, height: place.h }} role="dialog" aria-label={todo.text}>
      <header className="tc-head" onPointerDown={drag}>
        <span className="tc-num">#{todo.num}</span>
        <span className="tc-title" title={`${l.name} · #${todo.num} ${todo.text}`}>
          {todo.text}
        </span>
        {editing === null ? (
          <>
            <button onClick={() => setEditing(note)} title={tr("todoCard.edit")}>
              {tr("todoCard.edit")}
            </button>
            <button onClick={copy} disabled={!note} title={tr("todoCard.copyHint")}>
              {tr("todoCard.copy")}
            </button>
          </>
        ) : (
          <>
            <button className="primary" onClick={save} title={tr("todoCard.saveHint")}>
              {tr("todoCard.save")}
            </button>
            <button onClick={() => setEditing(null)}>{tr("common.cancel")}</button>
          </>
        )}
        <button className="tc-x" onClick={close} aria-label={tr("common.close")} title={tr("common.close") + "（Esc）"}>
          <Icon name="close" />
        </button>
      </header>
      {editing !== null ? (
        <textarea
          className="tc-edit"
          autoFocus
          value={editing}
          placeholder={tr("todoCard.placeholder")}
          onChange={(e) => setEditing(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") (e.preventDefault(), save());
          }}
        />
      ) : note ? (
        <div
          className="tc-body md"
          dangerouslySetInnerHTML={{ __html: renderMarkdown(note, {}) }}
          onClick={(e) => {
            const a = (e.target as HTMLElement).closest("a");
            if (!a?.href) return;
            e.preventDefault();
            void openExternal(a.href);
          }}
          onContextMenu={(e) => {
            const a = (e.target as HTMLElement).closest("a");
            if (!a?.href) return;
            e.preventDefault();
            setLink({ x: e.clientX, y: e.clientY, href: a.href });
          }}
        />
      ) : (
        <button className="tc-empty" onClick={() => setEditing("")}>
          {tr("todoCard.empty")}
        </button>
      )}
      {link && (
        <div className="menu ctx-menu tc-link-menu" style={{ left: link.x - place.x, top: link.y - place.y }} onMouseLeave={() => setLink(null)}>
          <button
            onClick={() => {
              void copyText(link.href).then((ok) => toast(ok ? tr("todoCard.linkCopied") : tr("code.copyFailed"), ok));
              setLink(null);
            }}
          >
            <Icon name="copy" size={14} /> {tr("todoCard.copyLink")}
          </button>
          <button onClick={() => (void openExternal(link.href), setLink(null))}><Icon name="open" size={14} /> {tr("todoCard.openLink")}</button>
        </div>
      )}
      <div className="tc-grip" onPointerDown={resize} title={tr("todoCard.resize")} aria-hidden />
    </div>,
    document.body,
  );
}
