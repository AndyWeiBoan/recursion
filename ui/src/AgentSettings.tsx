// One agent's settings, from its ⋯ menu: its name, model, effort, its own rules, what it may do
// without asking, and its changes to the chat's instructions files and MCP servers — for this
// agent in this chat only. A new model, effort or mode restarts its CLI
// before its next turn (the conversation is resumed), so they can change while it answers.
import { useState, useSyncExternalStore } from "react";
import { send, useModels, useWorkspace, type ModelOption } from "./client";
import { GuidePicker, LibraryPicker, McpPicker, fromLayer, useProjectOf } from "./ScopeSettings";
import { openSettings } from "./Settings";
import { t } from "./i18n";
import { Icon } from "./icons";
import { PickTabs, layered, type Overrides } from "./PickList";
import { PERM_MODES, type Member, type PermMode } from "./types";
import { Modal, isRunning } from "./WorkspaceView";

type Open = { panel: string; member: string };

// Claude doesn't say which effort is a model's default; an agent running it with none picked does
// (its `running_effort`), so what one agent showed is remembered for the others.
const DEFAULTS = "recursion.defaultEffort";
function learned(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(DEFAULTS) ?? "{}") ?? {};
  } catch {
    return {};
  }
}
function learn(key: string, effort: string) {
  const all = learned();
  if (all[key] === effort) return;
  try {
    localStorage.setItem(DEFAULTS, JSON.stringify({ ...all, [key]: effort }));
  } catch {}
}
let open: Open | null = null;
const subs = new Set<() => void>();
export function openAgentSettings(o: Open | null) {
  open = o;
  subs.forEach((f) => f());
}

/** The efforts `model` takes (for agent `m`), and which of them it runs when none is picked. */
export function effortsFor(m: Member, models: ModelOption[], model: string) {
  const picked = models.find((x) => x.value === model);
  const key = (v: string) => `${m.kind}:${v}`;
  if (!m.effort && m.running_effort) learn(key(m.model ?? ""), m.running_effort);
  const def = (model === (m.model ?? "") && !m.effort && m.running_effort) || picked?.default_effort || learned()[key(model)] || null;
  return { efforts: picked?.efforts ?? [], def };
}

/** An agent's settings sent with `patch` changed and the rest as they are (from its menu). */
export function saveAgent(panel: string, m: Member, patch: { effort?: string | null; mode?: PermMode }) {
  send({ t: "ws_agent_settings", panel, member: m.id, alias: m.alias, model: m.model ?? null, effort: m.effort ?? null, prompt: m.prompt ?? "", mode: m.mode ?? "edits", guides: m.guides ?? {}, mcp: m.mcp ?? {}, skills: m.skills ?? {}, rule_picks: m.rule_picks ?? {}, ...patch });
}

/** The open dialog, if any. Mounted once, at the top of the app. */
export function AgentSettingsLayer() {
  const o = useSyncExternalStore(
    (f) => (subs.add(f), () => subs.delete(f)),
    () => open,
  );
  const ws = useWorkspace(o?.panel ?? "");
  const m = o && ws?.members.find((x) => x.id === o.member);
  if (!o || !m) return null;
  return (
    <Modal title={t("agent.settingsTitle", { alias: m.alias })} onClose={() => openAgentSettings(null)}>
      <AgentForm key={`${o.panel}/${m.id}`} panel={o.panel} m={m} onDone={() => openAgentSettings(null)} />
    </Modal>
  );
}

function AgentForm({ panel, m, onDone }: { panel: string; m: Member; onDone: () => void }) {
  const [alias, setAlias] = useState(m.alias);
  const [model, setModel] = useState(m.model ?? "");
  const [effort, setEffort] = useState(m.effort ?? "");
  const [prompt, setPrompt] = useState(m.prompt ?? "");
  const [mode, setMode] = useState<PermMode>(m.mode ?? "edits");
  const [group, setGroup] = useState<"how" | "uses">("how");
  const [uses, setUses] = useState<"guides" | "mcp" | "skill" | "rule">("guides");
  const [skills, setSkills] = useState<Overrides>(m.skills ?? {});
  const [rulePicks, setRulePicks] = useState<Overrides>(m.rule_picks ?? {});
  const [guides, setGuides] = useState<Overrides>(m.guides ?? {});
  const [mcp, setMcp] = useState<Overrides>(m.mcp ?? {});
  // What its chat gives it: the project's, as the chat changed them.
  const chat = useWorkspace(panel)?.settings;
  const project = useProjectOf(panel);
  const guideBase = layered(project.guides, [chat?.guides]);
  const mcpBase = layered(project.mcp, [chat?.mcp]);
  const skillBase = layered(project.skills, [chat?.skills]);
  const ruleBase = layered(project.rules, [chat?.rule_picks]);
  const list = useModels(m.kind);
  // The model it runs may not be in the CLI's menu (a full id typed elsewhere): still offered.
  const options = list.models.some((x) => x.value === model) ? list.models : [...list.models, { value: model, label: model || t("agent.defaultModel"), description: "", efforts: [] }];
  const { efforts, def } = effortsFor(m, options, model);
  // The level "default" is for the picked model: no separate choice, that level says so.
  const shownEffort = effort || def;
  const pickModel = (v: string) => {
    setModel(v);
    const next = options.find((x) => x.value === v)?.efforts ?? [];
    if (effort && !next.includes(effort)) setEffort("");
  };
  const restarts = (model || null) !== (m.model ?? null) || (effort || null) !== (m.effort ?? null) || mode !== (m.mode ?? "edits");
  const same = (a: Overrides, b?: Overrides) => JSON.stringify(Object.entries(a).sort()) === JSON.stringify(Object.entries(b ?? {}).sort());
  const changed = restarts || alias.trim() !== m.alias || prompt.trim() !== (m.prompt ?? "") || !same(guides, m.guides) || !same(mcp, m.mcp) || !same(skills, m.skills) || !same(rulePicks, m.rule_picks);
  const busy = isRunning(m) || m.status === "waiting_permission";
  const id = `agent-${m.id}`;
  return (
    <form
      className="add-card agent-form"
      onSubmit={(e) => {
        e.preventDefault();
        send({ t: "ws_agent_settings", panel, member: m.id, alias: alias.trim(), model: model || null, effort: effort || null, prompt: prompt.trim(), mode, guides, mcp, skills, rule_picks: rulePicks });
        onDone();
      }}
    >
      <div className="add-head">
        <h3>{t("agent.settingsTitle", { alias: m.alias })}</h3>
        <p>{t("agent.settingsScope")}</p>
      </div>
      <div className="agent-form-two">
        <div className="field">
          <label htmlFor={`alias-${id}`}>{t("agent.alias")}</label>
          <div className="input-wrap">
            <span className="at">@</span>
            <input id={`alias-${id}`} value={alias} onChange={(e) => setAlias(e.target.value)} spellCheck={false} autoComplete="off" />
          </div>
        </div>
        <div className="field">
          <label htmlFor={`model-${id}`}>{t("agent.model")}</label>
          <div className="select-wrap">
            <select id={`model-${id}`} value={model} onChange={(e) => pickModel(e.target.value)}>
              {options.map((x) => (
                <option key={x.value} value={x.value} title={x.description}>
                  {x.label}
                </option>
              ))}
            </select>
            <span className="chev" aria-hidden="true">
              <Icon name="dropdown" size={14} />
            </span>
          </div>
        </div>
      </div>
      <div className="field">
        <label htmlFor={`prompt-${id}`} className="scope-label">
          {t("agent.prompt")}
          <span className="dim">{[...prompt].length}</span>
        </label>
        <textarea id={`prompt-${id}`} value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={3} placeholder={t("agent.promptPlaceholder")} />
        <span className="field-hint">{t("agent.promptHint")}</span>
      </div>
      {/* Two tabs: its behavior (effort and permissions, one above the other) and its resources
          (instructions files, MCP servers: a tab each). One height whichever is on. */}
      <div className="scope-tabbox">
        <PickTabs
          tabs={[
            { id: "how", label: t("agent.howTab") },
            { id: "uses", label: t("agent.toolsTitle") },
          ]}
          on={group}
          pick={setGroup}
        />
        {group === "how" ? (
          <>
          <div className="field">
            <span className="field-label">{t("agent.effort")}</span>
            {efforts.length ? (
              <div className="mode-seg effort-seg" role="radiogroup" aria-label={t("agent.effort")} style={{ gridTemplateColumns: `repeat(${efforts.length}, 1fr)` }}>
                {efforts.map((e) => (
                  // Picking the default level picks "default": it follows the CLI if that changes.
                  <button key={e} type="button" role="radio" aria-checked={shownEffort === e} className={shownEffort === e ? "on" : ""} onClick={() => setEffort(e === def ? "" : e)}>
                    {e}
                    {e === def && <span className="effort-def">{t("agent.defaultEffort")}</span>}
                  </button>
                ))}
              </div>
            ) : null}
            <span className="field-hint">{list.loading ? t("agent.modelsLoading", { kind: m.kind === "claude" ? "Claude" : "Codex" }) : !efforts.length ? t("agent.effortNone") : def ? t(`agent.effortHint.${m.kind}`) : t("agent.effortDefaultUnknown")}</span>
          </div>
          <div className="field">
            <span className="field-label">{t("perm.title")}</span>
            <div className="mode-seg" role="radiogroup" aria-label={t("perm.title")}>
              {PERM_MODES.map((p) => (
                <button key={p.id} type="button" role="radio" aria-checked={mode === p.id} className={`${mode === p.id ? "on" : ""} m-${p.id}`} onClick={() => setMode(p.id)}>
                  <Icon name={p.id} size={13} /> {p.label}
                </button>
              ))}
            </div>
            <span className={`field-hint${mode === "auto" ? " warn" : ""}`}>{PERM_MODES.find((p) => p.id === mode)?.hint}</span>
          </div>
          </>
        ) : (
          <>
            <PickTabs
              sub
              tabs={[
                { id: "guides", label: t("scope.guidesTab"), n: project.found ? layered(guideBase, [guides]).filter((p) => project.found!.some((f) => f.path === p)).length : undefined },
                { id: "mcp", label: "MCP", n: layered(mcpBase, [mcp]).length },
                { id: "skill", label: "Skills", n: layered(skillBase, [skills]).length },
                { id: "rule", label: t("lib.rules"), n: layered(ruleBase, [rulePicks]).length },
              ]}
              on={uses}
              pick={setUses}
            />
            {uses === "guides" ? (
              <GuidePicker session={project.session?.id ?? ""} found={project.found} layer={{ base: guideBase, from: fromLayer(chat?.guides), own: guides, onOwn: setGuides }} />
            ) : uses === "mcp" ? (
              <McpPicker layer={{ base: mcpBase, from: fromLayer(chat?.mcp), own: mcp, onOwn: setMcp }} onManage={() => (onDone(), openSettings("mcp", "mcp"))} />
            ) : uses === "skill" ? (
              <LibraryPicker key="skill" kind="skill" layer={{ base: skillBase, from: fromLayer(chat?.skills), own: skills, onOwn: setSkills }} onManage={() => (onDone(), openSettings("mcp", "skill"))} />
            ) : (
              <LibraryPicker key="rule" kind="rule" layer={{ base: ruleBase, from: fromLayer(chat?.rule_picks), own: rulePicks, onOwn: setRulePicks }} onManage={() => (onDone(), openSettings("mcp", "rule"))} />
            )}
          </>
        )}
      </div>
      {busy && restarts && <p className="agent-form-note">{t("agent.settingsBusy")}</p>}
      <div className="mcp-form-foot">
        <button type="button" onClick={onDone}>
          {t("common.cancel")}
        </button>
        <button className="primary" type="submit" disabled={!changed || !alias.trim()}>
          {t("scope.save")}
        </button>
      </div>
    </form>
  );
}
