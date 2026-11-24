// The 待辦 tab of the 產出 drawer: the chat's named todo lists, then one list's items. Items are
// ordered by dragging; ticking one doesn't move it; the 完成區 below holds what's put away
// (ticked or not). An item being done shows who is on it. The human and the agents share them.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { send, useWorkspace } from "./client";
import { t as tr } from "./i18n";
import type { Member, Todo, TodoList, TodoState } from "./types";

// Which list is open, per chat; outside React so it survives the drawer re-mounting.
const openList = new Map<string, string>();

const progress = (l: TodoList) => ({ done: l.items.filter((x) => x.state === "done").length, total: l.items.length });

function Bar({ done, total }: { done: number; total: number }) {
  return (
    <span className={`todo-bar${total > 0 && done === total ? " full" : ""}`} aria-hidden="true">
      <span style={{ width: total ? `${(done / total) * 100}%` : 0 }} />
    </span>
  );
}

function who(id: string, members: Member[]) {
  return id === "you" ? tr("todo.you") : "@" + (members.find((m) => m.id === id)?.alias ?? "?");
}

/** Text that turns into an input on double-click. */
function Editable({ text, onSave, className }: { text: string; onSave: (t: string) => void; className?: string }) {
  const [editing, setEditing] = useState(false);
  const [v, setV] = useState(text);
  if (!editing)
    return (
      <span className={className} onDoubleClick={(e) => (e.stopPropagation(), setV(text), setEditing(true))} title={tr("todo.editHint")}>
        {text}
      </span>
    );
  const save = () => {
    setEditing(false);
    if (v.trim() && v.trim() !== text) onSave(v.trim());
  };
  return (
    <input
      className="todo-edit"
      autoFocus
      value={v}
      onChange={(e) => setV(e.target.value)}
      onBlur={save}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.nativeEvent.isComposing || e.keyCode === 229) return;
        if (e.key === "Enter") save();
        if (e.key === "Escape") setEditing(false);
      }}
    />
  );
}

function AddBox({ placeholder, onAdd }: { placeholder: string; onAdd: (t: string) => void }) {
  const [v, setV] = useState("");
  return (
    <input
      className="todo-new"
      value={v}
      placeholder={placeholder}
      onChange={(e) => setV(e.target.value)}
      onKeyDown={(e) => {
        if (e.nativeEvent.isComposing || e.keyCode === 229) return;
        if (e.key === "Enter" && v.trim()) (onAdd(v.trim()), setV(""));
      }}
    />
  );
}

/** Click once to arm, again to delete. */
function DeleteButton({ onDelete }: { onDelete: () => void }) {
  const [armed, setArmed] = useState(false);
  return (
    <button
      className={`todo-del${armed ? " armed" : ""}`}
      onClick={(e) => (e.stopPropagation(), armed ? onDelete() : setArmed(true))}
      onMouseLeave={() => setArmed(false)}
      title={tr(armed ? "common.confirmDelete" : "common.delete")}
      aria-label={tr("common.delete")}
    >
      {armed ? tr("common.confirmDelete") : "✕"}
    </button>
  );
}

function Lists({ panel, lists, open }: { panel: string; lists: TodoList[]; open: (id: string) => void }) {
  return (
    <div className="todos">
      {lists.length === 0 && <p className="todo-empty">{tr("todo.noLists")}</p>}
      <ul className="todo-lists">
        {lists.map((l) => {
          const p = progress(l);
          return (
            <li key={l.id} className="todo-listrow" onClick={() => open(l.id)}>
              <span className="todo-listmain">
                <Editable className="todo-listname" text={l.name} onSave={(name) => send({ t: "todo_list_rename", panel, list: l.id, name })} />
                <span className="todo-count">
                  ({p.done}/{p.total})
                </span>
                <DeleteButton onDelete={() => send({ t: "todo_list_remove", panel, list: l.id })} />
              </span>
              <Bar {...p} />
            </li>
          );
        })}
      </ul>
      <AddBox placeholder={tr("todo.newList")} onAdd={(name) => send({ t: "todo_list_create", panel, name })} />
    </div>
  );
}

// While dragging, the row is lifted out and follows the pointer; a slot of its height opens
// where it would land and the other rows slide to make room. `index` is within that zone.
type Lift = { id: string; x0: number; y0: number; left: number; top: number; width: number; height: number };
type Slot = { archived: boolean; index: number };
const DUR = 180;

function Items({ panel, list, members, back }: { panel: string; list: TodoList; members: Member[]; back: () => void }) {
  const [lift, setLift] = useState<Lift | null>(null);
  const [slot, setSlot] = useState<Slot | null>(null);
  // The order shown right after a drop, until the daemon's comes back.
  const [order, setOrder] = useState<Todo[] | null>(null);
  useEffect(() => setOrder(null), [list.items]);
  const box = useRef<HTMLDivElement>(null);
  const awayRef = useRef<HTMLDivElement>(null);
  const floatRef = useRef<HTMLDivElement>(null);
  const slotRef = useRef<Slot | null>(null);
  slotRef.current = slot;
  const all = order ?? list.items;
  const rest = lift ? all.filter((t) => t.id !== lift.id) : all;
  const active = rest.filter((x) => !x.archived);
  const away = rest.filter((x) => x.archived);
  const restRef = useRef({ active, away, all });
  restRef.current = { active, away, all };
  const p = progress(list);
  const update = (item: Todo, patch: { text?: string; state?: TodoState }) => send({ t: "todo_update", panel, list: list.id, id: item.id, ...patch });

  // Rows slide from where they were to where they are now (FLIP). Positions are layout ones, in
  // the scrolled content, so scrolling or a running slide doesn't count as a move.
  const prev = useRef(new Map<string, number>());
  const land = useRef<{ id: string; x: number; y: number } | null>(null);
  useLayoutEffect(() => {
    const next = new Map<string, number>();
    box.current?.querySelectorAll<HTMLElement>("li[data-todo]").forEach((li) => {
      const id = li.dataset.todo!;
      const y = (li.parentElement as HTMLElement).offsetTop + li.offsetTop;
      next.set(id, y);
      const from = land.current?.id === id ? land.current : prev.current.has(id) ? { x: 0, y: prev.current.get(id)! } : null;
      if (from && (from.y !== y || from.x)) {
        li.getAnimations().forEach((a) => a.cancel());
        li.animate([{ transform: `translate(${from.x}px, ${from.y - y}px)` }, { transform: "none" }], { duration: DUR, easing: "cubic-bezier(.2,.7,.3,1)" });
      }
    });
    land.current = null;
    prev.current = next;
  });

  useEffect(() => {
    if (!lift) return;
    const el = box.current!;
    let pt = { x: lift.x0, y: lift.y0 };
    // Where the lifted row would land for the pointer at y: the zone it's over, then past how many rows' middles.
    const retarget = () => {
      const r = el.getBoundingClientRect();
      const cy = pt.y - r.top - el.clientTop + el.scrollTop;
      const archived = !!awayRef.current && cy >= awayRef.current.offsetTop;
      const cur = slotRef.current;
      const ul = el.querySelector<HTMLElement>(`[data-zone="${archived ? "away" : "active"}"] ul`);
      if (!ul) return;
      const hole = ul.querySelector<HTMLElement>("[data-slot]");
      if (hole && cur?.archived === archived && cy >= ul.offsetTop + hole.offsetTop && cy < ul.offsetTop + hole.offsetTop + hole.offsetHeight) return;
      let index = 0;
      ul.querySelectorAll<HTMLElement>("li[data-todo]").forEach((li) => {
        if (ul.offsetTop + li.offsetTop + li.offsetHeight / 2 < cy) index++;
      });
      if (cur?.archived !== archived || cur.index !== index) setSlot({ archived, index });
    };
    // Near the top or bottom edge, the list scrolls along.
    let raf = 0;
    const tick = () => {
      const r = el.getBoundingClientRect();
      const edge = 36;
      const v = pt.y < r.top + edge ? -Math.ceil((r.top + edge - pt.y) / 4) : pt.y > r.bottom - edge ? Math.ceil((pt.y - r.bottom + edge) / 4) : 0;
      if (v) {
        el.scrollTop += v;
        retarget();
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    const move = (e: PointerEvent) => {
      pt = { x: e.clientX, y: e.clientY };
      if (floatRef.current) floatRef.current.style.transform = `translate(${pt.x - lift.x0}px, ${pt.y - lift.y0}px)`;
      retarget();
    };
    // Put it down: into the slot, or (Escape) back where it was. Either way it glides from the pointer.
    const finish = (drop: boolean) => {
      const s = slotRef.current;
      const f = floatRef.current?.getBoundingClientRect();
      const hole = el.querySelector<HTMLElement>("[data-slot]")?.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const { active, away, all } = restRef.current;
      const item = all.find((t) => t.id === lift.id)!;
      if (f) land.current = { id: lift.id, x: hole ? f.left - hole.left : 0, y: f.top - r.top - el.clientTop + el.scrollTop };
      if (drop && s) {
        const index = s.archived ? active.length + s.index : s.index;
        const rest = [...active, ...away];
        rest.splice(index, 0, { ...item, archived: s.archived });
        setOrder(rest);
        send({ t: "todo_move", panel, list: list.id, id: lift.id, index, archived: s.archived });
      }
      setLift(null);
      setSlot(null);
    };
    const up = () => finish(true);
    const key = (e: KeyboardEvent) => e.key === "Escape" && finish(false);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", key);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", key);
    };
  }, [lift, panel, list.id]);

  const pickUp = (e: React.PointerEvent, item: Todo) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const li = (e.currentTarget as HTMLElement).closest("li")!;
    const r = li.getBoundingClientRect();
    const zone = all.filter((t) => t.archived === item.archived);
    setSlot({ archived: item.archived, index: zone.indexOf(item) });
    setLift({ id: item.id, x0: e.clientX, y0: e.clientY, left: r.left, top: r.top, width: r.width, height: r.height });
  };

  const row = (item: Todo, floating = false) => {
    const done = item.state === "done";
    return (
      <li key={item.id} data-todo={floating ? undefined : item.id} className={`todo st-${item.state}`}>
        <span className="todo-grip" onPointerDown={(e) => pickUp(e, item)} title={tr("todo.dragHint")} aria-label={tr("todo.dragHint")}>
          ⋮⋮
        </span>
        <input type="checkbox" checked={done} onChange={(e) => update(item, { state: e.target.checked ? "done" : "todo" })} aria-label={tr(done ? "todo.undo" : "todo.done")} />
        <span className="todo-num">#{item.num}</span>
        {done ? (
          <span className="todo-text" title={item.done_by ? tr("todo.doneBy", { who: who(item.done_by, members) }) : undefined}>
            {item.text}
          </span>
        ) : (
          <Editable className="todo-text" text={item.text} onSave={(text) => update(item, { text })} />
        )}
        {item.state === "doing" && item.worker ? (
          <button className="todo-doing on" onClick={() => update(item, { state: "todo" })} title={tr("todo.releaseHint")}>
            {tr("todo.doingBy", { who: who(item.worker, members) })}
          </button>
        ) : (
          !done && (
            <button className="todo-doing" onClick={() => update(item, { state: "doing" })} title={tr("todo.doingHint")}>
              {tr("todo.doing")}
            </button>
          )
        )}
        <DeleteButton onDelete={() => send({ t: "todo_remove", panel, list: list.id, id: item.id })} />
      </li>
    );
  };

  const rows = (zone: Todo[], archived: boolean) => {
    const out = zone.map((t) => row(t));
    if (lift && slot?.archived === archived) out.splice(slot.index, 0, <li key="slot" data-slot className="todo-slot" style={{ height: lift.height }} />);
    return out;
  };
  const lifted = lift && all.find((t) => t.id === lift.id);

  return (
    <div ref={box} className={`todos${lift ? " is-dragging" : ""}`}>
      <div className="todo-head">
        <button className="todo-back" onClick={back}>
          ‹ {tr("todo.allLists")}
        </button>
        <Editable className="todo-listname" text={list.name} onSave={(name) => send({ t: "todo_list_rename", panel, list: list.id, name })} />
        <span className="todo-count">
          ({p.done}/{p.total})
        </span>
      </div>
      <Bar {...p} />
      <div data-zone="active" className="todo-zone">
        {active.length === 0 && !(lift && !slot?.archived) && <p className="todo-empty">{tr("todo.empty")}</p>}
        <ul>{rows(active, false)}</ul>
        <AddBox placeholder={tr("todo.addPlaceholder")} onAdd={(text) => send({ t: "todo_add", panel, list: list.id, text })} />
      </div>
      <div ref={awayRef} data-zone="away" className="todo-zone todo-away">
        <h4>
          {tr("todo.away")} <span className="dim">{away.length || ""}</span>
        </h4>
        {away.length === 0 && !(lift && slot?.archived) && <p className="todo-empty">{tr("todo.awayHint")}</p>}
        <ul>{rows(away, true)}</ul>
      </div>
      {lift &&
        lifted &&
        createPortal(
          <div ref={floatRef} className="todo-float" style={{ left: lift.left, top: lift.top, width: lift.width }}>
            <ul>{row(lifted, true)}</ul>
          </div>,
          document.body,
        )}
    </div>
  );
}

/** The 待辦 tab: every list, or the one that's open. */
export function TodoPane({ panel }: { panel: string }) {
  const ws = useWorkspace(panel);
  const [open, setOpen] = useState<string | null>(openList.get(panel) ?? null);
  const go = (id: string | null) => {
    if (id) openList.set(panel, id);
    else openList.delete(panel);
    setOpen(id);
  };
  if (!ws) return null;
  const lists = ws.todo_lists ?? [];
  const list = open ? lists.find((l) => l.id === open) : undefined;
  return list ? <Items key={list.id} panel={panel} list={list} members={ws.members} back={() => go(null)} /> : <Lists panel={panel} lists={lists} open={go} />;
}
