// Settings for one project (its prompt, guidance files and MCP servers) and for one chat (its name,
// standing rules for the agents, a usage limit, and its changes to the project's files and
// servers). An agent's own changes are in AgentSettings.tsx; project → chat → agent, nearest wins.
import { useEffect, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { copyText } from "./clipboard";
import { send, toast, useAccounts, useChatLayers, useGuide, useGuides, useLibrary, useMcp, useSnapshot, useWorkspace, type GuideFile, type LibraryKind } from "./client";
import { BrandMark } from "./icons";
import { LibraryCard } from "./Library";
import { t } from "./i18n";
import { useCardPlace } from "./TodoCard";
import { chatsOf, type ChatSettings } from "./types";
import { Modal, renderMarkdown } from "./WorkspaceView";
import { StatusDot, statusLine } from "./Mcp";
import { PickList, PickTabs, layered, type Below, type Layer, type Overrides } from "./PickList";
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
      <SessionForm key={s.id} id={s.id} name={s.name} cwd={s.cwd} prompt={s.prompt ?? ""} mcp={s.mcp ?? []} skills={s.skills ?? []} rules={s.rules ?? []} onDone={() => openSessionSettings(null)} />
    </Modal>
  );
}

/** A project's settings: the prompt every agent of it gets, the guidance files in its folder
 * ticked for them (see guides.rs), its MCP servers. Its folder is what the project is, so it
 * isn't edited here; nor are defaults for new agents (the add-agent form remembers the last). */
function SessionForm({ id, name, cwd, prompt: initialPrompt, mcp: initialMcp, skills: initialSkills, rules: initialRules, onDone }: { id: string; name: string; cwd: string; prompt: string; mcp: string[]; skills: string[]; rules: string[]; onDone: () => void }) {
  const [mcp, setMcp] = useState(initialMcp);
  const [prompt, setPrompt] = useState(initialPrompt);
  const guides = useGuides(id);
  const [picks, setPicks] = useState<string[] | null>(null);
  const picked = picks ?? guides?.picks ?? [];
  const [tab, setTab] = useState<"guides" | "mcp" | "skill" | "rule">("guides");
  const [skills, setSkills] = useState(initialSkills);
  const [rules, setRules] = useState(initialRules);
  // The project's chats, each following it but for what it set itself.
  const { state } = useSnapshot();
  const s = state?.sessions.find((x) => x.id === id);
  const chats = s && state ? chatsOf(s, state) : [];
  const layers = useChatLayers(id);
  const [clears, setClears] = useState<Clears>({});
  const members = chats.map((c) => {
    const l = layers?.find((x) => x.panel === c.panel);
    return { key: c.panel, name: c.label, own: { guides: l?.guides, mcp: l?.mcp, skills: l?.skills, rule_picks: l?.rule_picks } };
  });
  const below = (f: Field) => belowOf(members, f, clears, setClears, "chats");
  const save = () => {
    send({ t: "session_settings", session: id, cwd, defaults: {}, mcp, skills, rules, prompt: prompt.trim(), ...(picks ? { guides: picks } : {}) });
    sendClears((panel) => ({ panel }), clears);
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
          { id: "skill", label: "Skills", n: skills.length },
          { id: "rule", label: t("lib.rules"), n: rules.length },
        ]}
        on={tab}
        pick={setTab}
      />
      {tab === "guides" ? (
        <GuidePicker session={id} found={guides?.found} picked={picked} onChange={setPicks} below={below("guides")} />
      ) : tab === "mcp" ? (
        <McpPicker value={mcp} onChange={setMcp} below={below("mcp")} onManage={() => (onDone(), openSettings("mcp", "mcp"))} />
      ) : (
        <LibraryPicker key={tab} kind={tab} value={tab === "skill" ? skills : rules} onChange={tab === "skill" ? setSkills : setRules} below={below(tab === "skill" ? "skills" : "rule_picks")} onManage={() => (onDone(), openSettings("mcp", tab))} />
      )}
      <button className="add-submit" type="submit">
        {t("scope.save")}
      </button>
    </form>
  );
}

/** What a chat's agents start from: its project's ticked files and servers, and the files found. */
export function useProjectOf(panel: string) {
  const { state } = useSnapshot();
  const s = state?.sessions.find((x) => state && chatsOf(x, state).some((c) => c.panel === panel));
  const guides = useGuides(s?.id ?? "");
  return { session: s, found: guides?.found, guides: guides?.picks ?? [], mcp: s?.mcp ?? [], skills: s?.skills ?? [], rules: s?.rules ?? [] };
}

type Field = "guides" | "mcp" | "skills" | "rule_picks";
/** The reminder's "change them too", kept until the form is saved: member → field → names. */
type Clears = Record<string, Partial<Record<Field, string[]>>>;
const without = (o: Overrides | undefined, names: string[] = []) => Object.fromEntries(Object.entries(o ?? {}).filter(([k]) => !names.includes(k)));
/** The layer below for one field: its members with their own settings (less what is to be
 *  cleared), and how its rows count them. */
function belowOf(members: { key: string; name: string; mark?: React.ReactNode; own: Partial<Record<Field, Overrides>> }[], field: Field, clears: Clears, setClears: (c: Clears) => void, unit: "agents" | "chats"): Below {
  return {
    members: members.map((m) => ({ key: m.key, name: m.name, mark: m.mark, own: without(m.own[field], clears[m.key]?.[field]) })),
    count: (n, total) => t(n === total ? `layer.all.${unit}` : n === 0 ? `layer.none.${unit}` : `layer.some.${unit}`, { n, total }),
    noun: t(`layer.noun.${unit}`),
    clear: (id, keys) => {
      const next = { ...clears };
      for (const k of keys) next[k] = { ...next[k], [field]: [...(next[k]?.[field] ?? []), id] };
      setClears(next);
    },
  };
}
/** The cleared settings, sent when the form is saved. */
function sendClears(panelOf: (key: string) => { panel: string; member?: string }, clears: Clears) {
  for (const [key, fields] of Object.entries(clears))
    for (const [field, names] of Object.entries(fields)) if (names?.length) send({ t: "layer_clear", ...panelOf(key), field: field as Field, names });
}

/** Where a row of a chat's or an agent's list comes from, when that layer didn't change it. */
export const fromLayer = (chat?: Overrides) => (id: string) => (chat && id in chat ? t("layer.chat") : t("layer.project"));

/** The guidance files found in the folder, to tick for the agents and preview. */
export function GuidePicker({ session, found, picked, onChange, layer, below }: { session: string; found?: GuideFile[]; picked?: string[]; onChange?: (p: string[]) => void; layer?: Layer; below?: Below }) {
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
        layer={layer}
        below={below}
        onOpen={setOpen}
        search={t("scope.guidesSearch")}
        empty={t("scope.guidesNone")}
      />
      <span className="field-hint pick-hint">{found.length > 0 ? t(layer ? "layer.guidesHint" : "scope.guidesHint") : ""}</span>
      {open && <GuideCard session={session} path={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

/** Recursion's Skills or rules, to tick for the agents (a project's, or over the layer above). */
export function LibraryPicker({ kind, value, onChange, layer, below, onManage }: { kind: LibraryKind; value?: string[]; onChange?: (v: string[]) => void; layer?: Layer; below?: Below; onManage: () => void }) {
  const lib = useLibrary(kind);
  const [open, setOpen] = useState<string | null>(null);
  const items = lib?.items ?? [];
  return (
    <div className="field">
      <PickList
        items={items.map((i) => ({ id: i.name, label: i.name, meta: i.paths?.length ? t("lib.onlyFor", { paths: i.paths.join(", ") }) : i.description }))}
        picked={value}
        onChange={onChange}
        layer={layer}
        below={below}
        onOpen={setOpen}
        search={t("lib.search", { noun: kind === "skill" ? "Skills" : t("lib.rules") })}
        empty={t("lib.pickNone", { noun: kind === "skill" ? "Skills" : t("lib.rules") })}
        action={
          <button type="button" className="picks-action" onClick={onManage}>
            {t("lib.manage")}
          </button>
        }
      />
      <span className="field-hint pick-hint">{t(kind === "skill" ? (layer ? "lib.skillPickLayer" : "lib.skillPick") : layer ? "lib.rulePickLayer" : "lib.rulePick")}</span>
      {open && <LibraryCard kind={kind} name={open} path={null} onClose={() => setOpen(null)} />}
    </div>
  );
}

/** The human's MCP servers, to tick for this project's agents. */
export function McpPicker({ value, onChange, onManage, layer, below }: { value?: string[]; onChange?: (v: string[]) => void; onManage: () => void; layer?: Layer; below?: Below }) {
  const m = useMcp();
  const names = Object.keys(m?.servers ?? {});
  return (
    <div className="field">
      <PickList
        items={names.map((n) => ({ id: n, label: n, lead: <StatusDot status={m!.status[n]} />, meta: statusLine(m!.status[n]) }))}
        picked={value}
        onChange={onChange}
        layer={layer}
        below={below}
        search={t("mcp.search")}
        empty={t("mcp.noneYet")}
        action={
          <button type="button" className="picks-action" onClick={onManage}>
            {t("mcp.manage")}
          </button>
        }
      />
      <span className="field-hint pick-hint">{t(layer ? "layer.mcpHint" : "mcp.sessionHint")}</span>
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

/** A chat's settings: its name, the rules every agent gets, the usage limit, and its changes
 *  to the project's instructions files and MCP servers. */
export function ChatSettingsForm({ panel, onDone }: { panel: string; onDone: () => void }) {
  const ws = useWorkspace(panel);
  const cur: ChatSettings = ws?.settings ?? { name: "", rules: "" };
  const [name, setName] = useState(cur.name);
  const [rules, setRules] = useState(cur.rules);
  const [limit, setLimit] = useState<number | null>(cur.usage_limit ?? null);
  const [guides, setGuides] = useState<Overrides>(cur.guides ?? {});
  const [mcp, setMcp] = useState<Overrides>(cur.mcp ?? {});
  const [tab, setTab] = useState<"guides" | "mcp" | "skill" | "rule" | "usage">("guides");
  const [skills, setSkills] = useState<Overrides>(cur.skills ?? {});
  const [rulePicks, setRulePicks] = useState<Overrides>(cur.rule_picks ?? {});
  const project = useProjectOf(panel);
  const from = fromLayer();
  const accounts = useAccounts();
  // Its agents, each following the chat but for what it set itself.
  const [clears, setClears] = useState<Clears>({});
  const agents = (ws?.members ?? []).map((m) => ({ key: m.id, name: `@${m.alias}`, mark: <BrandMark kind={m.kind} size={14} />, own: { guides: m.guides, mcp: m.mcp, skills: m.skills, rule_picks: m.rule_picks } }));
  const below = (f: Field) => belowOf(agents, f, clears, setClears, "agents");
  // How much of their windows the accounts this chat's agents run under have used, for the limit's hint.
  const used = [...new Set((ws?.members ?? []).map((m) => m.account ?? `${m.kind}-default`))]
    .map((id) => accounts.find((a) => a.id === id))
    .filter((a): a is NonNullable<typeof a> => !!a)
    .map((a) => `${a.kind === "claude" ? "Claude" : "Codex"}${a.id.endsWith("-default") ? "" : `（${a.name || a.email}）`} ${Object.entries(a.usage).map(([k, u]) => `${k === "five_hour" ? t("usage.5h") : k === "seven_day" ? t("usage.7d") : k} ${Math.round(u * 100)}%`).join(" · ")}`)
    .filter((x) => /%/.test(x));
  const save = () => {
    send({ t: "ws_settings", panel, settings: { name: name.trim(), rules: rules.trim(), usage_limit: limit ?? undefined, guides, mcp, skills, rule_picks: rulePicks } });
    sendClears((member) => ({ panel, member }), clears);
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
        <textarea value={rules} onChange={(e) => setRules(e.target.value)} rows={4} placeholder={t("scope.rulesPlaceholder")} />
        <span className="field-hint">{t("scope.rulesHint")}</span>
      </label>
      {/* What the agents may use and how much of their accounts, one tab each, in a box of one height. */}
      <div className="scope-tabbox">
        <PickTabs
          tabs={[
            { id: "guides", label: t("scope.guidesTab"), n: project.found ? layered(project.guides, [guides]).filter((p) => project.found!.some((f) => f.path === p)).length : undefined },
            { id: "mcp", label: "MCP", n: layered(project.mcp, [mcp]).length },
            { id: "skill", label: "Skills", n: layered(project.skills, [skills]).length },
            { id: "rule", label: t("lib.rules"), n: layered(project.rules, [rulePicks]).length },
            { id: "usage", label: t("scope.usageLimit") },
          ]}
          on={tab}
          pick={setTab}
        />
        {tab === "guides" ? (
          <GuidePicker session={project.session?.id ?? ""} found={project.found} layer={{ base: project.guides, from, own: guides, onOwn: setGuides }} below={below("guides")} />
        ) : tab === "mcp" ? (
          <McpPicker layer={{ base: project.mcp, from, own: mcp, onOwn: setMcp }} below={below("mcp")} onManage={() => (onDone(), openSettings("mcp", "mcp"))} />
        ) : tab === "skill" ? (
          <LibraryPicker key="skill" kind="skill" layer={{ base: project.skills, from, own: skills, onOwn: setSkills }} below={below("skills")} onManage={() => (onDone(), openSettings("mcp", "skill"))} />
        ) : tab === "rule" ? (
          <LibraryPicker key="rule" kind="rule" layer={{ base: project.rules, from, own: rulePicks, onOwn: setRulePicks }} below={below("rule_picks")} onManage={() => (onDone(), openSettings("mcp", "rule"))} />
        ) : (
          <div className="field">
            <div className="mode-seg" role="radiogroup" aria-label={t("scope.usageLimit")} style={{ gridTemplateColumns: `repeat(${LIMITS.length}, 1fr)` }}>
              {LIMITS.map((p) => (
                <button key={p ?? "none"} type="button" role="radio" aria-checked={limit === p} className={limit === p ? "on" : ""} onClick={() => setLimit(p)}>
                  {p === null ? t("scope.noLimit") : `${p}%`}
                </button>
              ))}
            </div>
            <span className="field-hint">
              {t("scope.usageHint")}
              {used.length > 0 && (
                <>
                  <br />
                  {t("scope.usageNow", { list: used.join("；") })}
                </>
              )}
            </span>
          </div>
        )}
      </div>
      <button className="add-submit" type="submit">
        {t("scope.save")}
      </button>
    </form>
  );
}

/** The usage limits a chat can pick: none, or a share of an account's windows. */
const LIMITS: (number | null)[] = [null, 50, 70, 80, 90];
