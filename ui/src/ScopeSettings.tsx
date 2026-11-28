// Settings for one project (its prompt, guidance files and MCP servers) and for one chat (its name,
// standing rules for the agents, and a spending limit).
import { useEffect, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { copyText } from "./clipboard";
import { send, toast, useGuide, useGuides, useMcp, useSnapshot, useWorkspace, type GuideFile } from "./client";
import { t } from "./i18n";
import { useCardPlace } from "./TodoCard";
import type { ChatSettings } from "./types";
import { Modal, renderMarkdown } from "./WorkspaceView";
import { StatusDot, statusLine } from "./Mcp";
import { PickList, PickTabs } from "./PickList";
import { openSettings } from "./Settings";
import { Icon } from "./icons";

// Which session's settings are open; outside React so the sidebar menu can open them.
let openId: string | null = null;
const subs = new Set<() => void>();
export function openSessionSettings(id: string | null) {
  openId = id;
  subs.forEach((f) => f());
}

export function SessionSettingsModal() {
  const id = useSyncExternalStore(
    (f) => (subs.add(f), () => subs.delete(f)),
    () => openId,
  );
  const { state } = useSnapshot();
  const s = id ? state?.sessions.find((x) => x.id === id) : undefined;
  if (!s) return null;
  return (
    <Modal title={t("scope.session")} onClose={() => openSessionSettings(null)}>
      <SessionForm key={s.id} id={s.id} name={s.name} cwd={s.cwd} prompt={s.prompt ?? ""} mcp={s.mcp ?? []} onDone={() => openSessionSettings(null)} />
    </Modal>
  );
}

/** A project's settings: the prompt every agent of it gets, the guidance files in its folder
 * ticked for them (see guides.rs), its MCP servers. Its folder is what the project is, so it
 * isn't edited here; nor are defaults for new agents (the add-agent form remembers the last). */
function SessionForm({ id, name, cwd, prompt: initialPrompt, mcp: initialMcp, onDone }: { id: string; name: string; cwd: string; prompt: string; mcp: string[]; onDone: () => void }) {
  const [mcp, setMcp] = useState(initialMcp);
  const [prompt, setPrompt] = useState(initialPrompt);
  const guides = useGuides(id);
  const [picks, setPicks] = useState<string[] | null>(null);
  const picked = picks ?? guides?.picks ?? [];
  const [tab, setTab] = useState<"guides" | "mcp">("guides");
  const save = () => {
    send({ t: "session_settings", session: id, cwd, defaults: {}, mcp, prompt: prompt.trim(), ...(picks ? { guides: picks } : {}) });
    onDone();
  };
  return (
    <form className="poll-form scope-form" onSubmit={(e) => (e.preventDefault(), save())}>
      <h3>{t("scope.sessionTitle", { name })}</h3>
      <label className="field">
        <span className="scope-label">
          {t("scope.prompt")}
          <span className="dim">{[...prompt].length}</span>
        </span>
        <textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={5} placeholder={t("scope.promptPlaceholder")} />
        <span className="field-hint">
          {t("scope.promptHint")} <b>{t("scope.promptOver")}</b>
        </span>
      </label>
      {/* What the agents may use, one tab each, in one look (PickList); Rules, Skills and roles
          join as tabs of the same kind. */}
      <PickTabs
        tabs={[
          { id: "guides", label: t("scope.guidesTab"), n: guides ? picked.filter((p) => guides.found.some((f) => f.path === p)).length : undefined },
          { id: "mcp", label: "MCP", n: mcp.length },
        ]}
        on={tab}
        pick={setTab}
      />
      {tab === "guides" ? (
        <GuidePicker session={id} found={guides?.found} picked={picked} onChange={setPicks} />
      ) : (
        <McpPicker value={mcp} onChange={setMcp} onManage={() => (onDone(), openSettings("mcp"))} />
      )}
      <button className="add-submit" type="submit">
        {t("scope.save")}
      </button>
    </form>
  );
}

/** The guidance files found in the folder, to tick for the agents and preview. */
function GuidePicker({ session, found, picked, onChange }: { session: string; found?: GuideFile[]; picked: string[]; onChange: (p: string[]) => void }) {
  const [open, setOpen] = useState<string | null>(null);
  // Still looking: the same room, so nothing jumps when the list arrives.
  if (!found)
    return (
      <div className="field">
        <div className="picks picks-none" />
        <span className="field-hint pick-hint" />
      </div>
    );
  return (
    <div className="field">
      <PickList
        items={found.map((f) => ({ id: f.path, label: f.path, meta: t("scope.guideLines", { n: f.lines }) }))}
        picked={picked}
        onChange={onChange}
        onOpen={setOpen}
        search={t("scope.guidesSearch")}
        empty={t("scope.guidesNone")}
      />
      <span className="field-hint pick-hint">{found.length > 0 ? t("scope.guidesHint") : ""}</span>
      {open && <GuideCard session={session} path={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

/** The human's MCP servers, to tick for this project's agents. */
function McpPicker({ value, onChange, onManage }: { value: string[]; onChange: (v: string[]) => void; onManage: () => void }) {
  const m = useMcp();
  const names = Object.keys(m?.servers ?? {});
  return (
    <div className="field">
      <PickList
        items={names.map((n) => ({ id: n, label: n, lead: <StatusDot status={m!.status[n]} />, meta: statusLine(m!.status[n]) }))}
        picked={value}
        onChange={onChange}
        search={t("mcp.search")}
        empty={t("mcp.noneYet")}
        action={
          <button type="button" className="picks-action" onClick={onManage}>
            {t("mcp.manage")}
          </button>
        }
      />
      <span className="field-hint pick-hint">{t("mcp.sessionHint")}</span>
    </div>
  );
}

/** A guidance file's preview: its Markdown rendered, in a card moved and resized like a todo's. */
function GuideCard({ session, path, onClose }: { session: string; path: string; onClose: () => void }) {
  const text = useGuide(session, path);
  const { place, drag, resize } = useCardPlace("recursion.guideCard");
  useEffect(() => overlay(), []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.isComposing) (e.stopPropagation(), onClose());
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  const copy = () => text !== undefined && void copyText(text).then((ok) => toast(ok ? t("todoCard.copied") : t("code.copyFailed"), ok));
  return createPortal(
    <div className="todo-card" style={{ left: place.x, top: place.y, width: place.w, height: place.h }} role="dialog" aria-label={path}>
      <header className="tc-head" onPointerDown={drag}>
        <span className="tc-title guide-title">{path}</span>
        <button type="button" onClick={copy} disabled={text === undefined}>
          {t("todoCard.copy")}
        </button>
        <button type="button" className="tc-x" onClick={onClose} aria-label={t("common.close")} title={t("common.close") + "（Esc）"}>
          <Icon name="close" />
        </button>
      </header>
      <div className="tc-body md" dangerouslySetInnerHTML={{ __html: text === undefined ? "" : renderMarkdown(text, {}) }} />
      <div className="tc-grip" onPointerDown={resize} aria-hidden />
    </div>,
    document.body,
  );
}

/** A chat's settings: its name, the rules every agent gets, and the spending limit. */
export function ChatSettingsForm({ panel, onDone }: { panel: string; onDone: () => void }) {
  const ws = useWorkspace(panel);
  const cur: ChatSettings = ws?.settings ?? { name: "", rules: "" };
  const [name, setName] = useState(cur.name);
  const [rules, setRules] = useState(cur.rules);
  const [budget, setBudget] = useState(cur.budget_usd ? String(cur.budget_usd) : "");
  const spent = (ws?.members ?? []).reduce((s, m) => s + (m.cost_usd || 0), 0);
  const value = Number(budget);
  const bad = budget.trim() !== "" && !(value > 0);
  const save = () => {
    send({ t: "ws_settings", panel, settings: { name: name.trim(), rules: rules.trim(), budget_usd: budget.trim() ? value : undefined } });
    onDone();
  };
  return (
    <form className="poll-form scope-form" onSubmit={(e) => (e.preventDefault(), save())}>
      <h3>{t("scope.chat")}</h3>
      <label className="field">
        <span>{t("scope.chatName")}</span>
        <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={t("kind.workspace")} />
      </label>
      <label className="field">
        <span>{t("scope.rules")}</span>
        <textarea value={rules} onChange={(e) => setRules(e.target.value)} rows={6} placeholder={t("scope.rulesPlaceholder")} />
        <span className="field-hint">{t("scope.rulesHint")}</span>
      </label>
      <label className="field">
        <span>{t("scope.budget")}</span>
        <div className="scope-row">
          <span className="scope-dollar">$</span>
          <input value={budget} onChange={(e) => setBudget(e.target.value)} inputMode="decimal" placeholder={t("scope.noLimit")} />
        </div>
        <span className={`field-hint${bad ? " warn" : ""}`}>{bad ? t("scope.budgetBad") : t("scope.budgetHint", { spent: spent.toFixed(2) })}</span>
      </label>
      <button className="add-submit" type="submit" disabled={bad}>
        {t("scope.save")}
      </button>
    </form>
  );
}
