// One look for every "what may the agents use" list: a filter, how many are ticked, five rows in
// sight (the rest scroll), a tick per row and, where there is something to read, a preview.
// Used for a project's guidance files and MCP servers; Rules, Skills and roles will use it too.
// Below the project (a chat, an agent) a row follows the layer above unless that layer changed it:
// then it says so, and can go back (`layer`).
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

/** A layer's own changes to the one above: on or off by id; what isn't named follows above. */
export type Overrides = Record<string, boolean>;

/** `base` with each layer's changes applied, nearest last (as the daemon does). */
export function layered(base: string[], layers: (Overrides | undefined)[]): string[] {
  let out = [...base];
  for (const l of layers)
    for (const [id, on] of Object.entries(l ?? {})) {
      if (on && !out.includes(id)) out.push(id);
      if (!on) out = out.filter((x) => x !== id);
    }
  return out;
}

/** Below the top layer: what the layer above gives (`base`), where each of those comes from,
 *  and this layer's own changes. */
export type Layer = { base: string[]; from: (id: string) => string; own: Overrides; onOwn: (o: Overrides) => void };

/** `action`: a button for the list's own page (e.g. managing MCP servers), beside the filter.
 *  Always the same height, empty or not, so switching tabs doesn't move what is below.
 *  With `layer`, what is on comes from it, and `picked` / `onChange` are not used. */
export function PickList({ items, picked = [], onChange, onOpen, search, empty, action, layer }: { items: PickItem[]; picked?: string[]; onChange?: (p: string[]) => void; onOpen?: (id: string) => void; search: string; empty: React.ReactNode; action?: React.ReactNode; layer?: Layer }) {
  const [q, setQ] = useState("");
  if (!items.length)
    return (
      <div className="picks picks-none">
        <span>{empty}</span>
        {action}
      </div>
    );
  const shown = items.filter((f) => f.label.toLowerCase().includes(q.trim().toLowerCase()));
  const isOn = (id: string) => (layer ? (layer.own[id] ?? layer.base.includes(id)) : picked.includes(id));
  const toggle = (id: string, on: boolean) => {
    if (!layer) return onChange?.(on ? [...picked.filter((p) => p !== id), id] : picked.filter((p) => p !== id));
    // Back to what the layer above says is following it again.
    const { [id]: _, ...rest } = layer.own;
    layer.onOwn(on === layer.base.includes(id) ? rest : { ...rest, [id]: on });
  };
  const reset = (id: string) => {
    const { [id]: _, ...rest } = layer!.own;
    layer!.onOwn(rest);
  };
  return (
    <div className="picks">
      <div className="picks-tools">
        <label className="picks-search">
          <Icon name="search" size={14} />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder={search} spellCheck={false} />
        </label>
        <span className="dim">{t("pick.picked", { n: items.filter((f) => isOn(f.id)).length })}</span>
        {action}
      </div>
      <div className="picks-list" role="list">
        {shown.map((f) => {
          const on = isOn(f.id);
          const own = !!layer && f.id in layer.own;
          const cut = f.label.lastIndexOf("/") + 1;
          return (
            <div key={f.id} className={`pick-row${on ? " on" : ""}${layer && !own ? " inherited" : ""}`} role="listitem">
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
              <span className="dim pick-meta">
                {layer &&
                  (own ? (
                    <>
                      <span className="pick-src own">{t("layer.own")}</span>
                      <button type="button" className="pick-reset" onClick={() => reset(f.id)}>
                        {t("layer.reset")}
                      </button>
                    </>
                  ) : (
                    <span className="pick-src">{layer.from(f.id)}</span>
                  ))}
                {f.meta}
              </span>
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

/** Tabs over such lists, each with its count.
 *  `sub`: the second level, under a first one. */
export function PickTabs<K extends string>({ tabs, on, pick, sub }: { tabs: { id: K; label: string; n?: React.ReactNode }[]; on: K; pick: (k: K) => void; sub?: boolean }) {
  return (
    <div className={`pick-tabs${sub ? " sub" : ""}`} role="tablist">
      {tabs.map((x) => (
        <button key={x.id} type="button" role="tab" aria-selected={on === x.id} className={on === x.id ? "on" : ""} onClick={() => pick(x.id)}>
          {x.label}
          {x.n !== undefined && <span className="dim">{x.n}</span>}
        </button>
      ))}
    </div>
  );
}
