// One look for every "what may the agents use" list: a filter, how many are ticked, five rows in
// sight (the rest scroll), a tick per row and, where there is something to read, a preview.
// Used for a project's guidance files and MCP servers; Rules, Skills and roles will use it too.
import { useState } from "react";
import { t } from "./i18n";
import { Icon } from "./icons";

export type PickItem = {
  id: string;
  /** Shown as `dir/` dimmed and the name bright when it is a path. */
  label: string;
  /** On the right: lines, a status… */
  meta?: React.ReactNode;
  /** Before the name: a status dot… */
  lead?: React.ReactNode;
};

/** `action`: a button for the list's own page (e.g. managing MCP servers), beside the filter.
 *  Always the same height, empty or not, so switching tabs doesn't move what is below. */
export function PickList({ items, picked, onChange, onOpen, search, empty, action }: { items: PickItem[]; picked: string[]; onChange: (p: string[]) => void; onOpen?: (id: string) => void; search: string; empty: React.ReactNode; action?: React.ReactNode }) {
  const [q, setQ] = useState("");
  if (!items.length)
    return (
      <div className="picks picks-none">
        <span>{empty}</span>
        {action}
      </div>
    );
  const shown = items.filter((f) => f.label.toLowerCase().includes(q.trim().toLowerCase()));
  const toggle = (id: string, on: boolean) => onChange(on ? [...picked.filter((p) => p !== id), id] : picked.filter((p) => p !== id));
  return (
    <div className="picks">
      <div className="picks-tools">
        <label className="picks-search">
          <Icon name="search" size={14} />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder={search} spellCheck={false} />
        </label>
        <span className="dim">{t("pick.picked", { n: picked.filter((p) => items.some((f) => f.id === p)).length })}</span>
        {action}
      </div>
      <div className="picks-list" role="list">
        {shown.map((f) => {
          const on = picked.includes(f.id);
          const cut = f.label.lastIndexOf("/") + 1;
          return (
            <div key={f.id} className={`pick-row${on ? " on" : ""}`} role="listitem">
              <input type="checkbox" checked={on} onChange={(e) => toggle(f.id, e.target.checked)} aria-label={f.label} />
              <span className="pick-name">
                {f.lead}
                {onOpen ? (
                  <button type="button" onClick={() => onOpen(f.id)} title={t("pick.preview", { name: f.label })}>
                    <span className="dim">{f.label.slice(0, cut)}</span>
                    <b>{f.label.slice(cut)}</b>
                  </button>
                ) : (
                  <b>{f.label}</b>
                )}
              </span>
              <span className="dim pick-meta">{f.meta}</span>
              {onOpen ? (
                <button type="button" className="pick-eye" onClick={() => onOpen(f.id)} aria-label={t("pick.preview", { name: f.label })}>
                  <Icon name="preview" size={14} />
                </button>
              ) : (
                <span />
              )}
            </div>
          );
        })}
        {shown.length === 0 && <div className="picks-empty">{t("pick.noMatch")}</div>}
      </div>
    </div>
  );
}

/** Tabs over such lists, each with its count. */
export function PickTabs<K extends string>({ tabs, on, pick }: { tabs: { id: K; label: string; n?: number }[]; on: K; pick: (k: K) => void }) {
  return (
    <div className="pick-tabs" role="tablist">
      {tabs.map((x) => (
        <button key={x.id} type="button" role="tab" aria-selected={on === x.id} className={on === x.id ? "on" : ""} onClick={() => pick(x.id)}>
          {x.label}
          {x.n !== undefined && <span className="dim">{x.n}</span>}
        </button>
      ))}
    </div>
  );
}
