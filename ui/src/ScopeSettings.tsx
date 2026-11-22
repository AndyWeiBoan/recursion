// Settings for one session (its folder, defaults for new agents) and for one chat (its name,
// standing rules for the agents, and a spending limit).
import { useState, useSyncExternalStore } from "react";
import { pickFolder, send, useModels, useSnapshot, useWorkspace } from "./client";
import { t } from "./i18n";
import { PERM_MODES, type AgentDefaults, type AgentKind, type ChatSettings, type PermMode } from "./types";
import { Modal } from "./WorkspaceView";

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
      <SessionForm key={s.id} id={s.id} name={s.name} cwd={s.cwd} defaults={s.defaults ?? {}} onDone={() => openSessionSettings(null)} />
    </Modal>
  );
}

function ModelSelect({ kind, value, onChange }: { kind: AgentKind; value: string; onChange: (v: string) => void }) {
  const list = useModels(kind);
  return (
    <div className="select-wrap">
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">{t("agent.defaultModel")}</option>
        {list.models
          .filter((m) => m.value)
          .map((m) => (
            <option key={m.value} value={m.value}>
              {m.label}
            </option>
          ))}
      </select>
      <span className="chev" aria-hidden="true">
        ⌄
      </span>
    </div>
  );
}

function SessionForm({ id, name, cwd: initialCwd, defaults, onDone }: { id: string; name: string; cwd: string; defaults: AgentDefaults; onDone: () => void }) {
  const [cwd, setCwd] = useState(initialCwd);
  const [kind, setKind] = useState<AgentKind | "">(defaults.kind ?? "");
  const [claudeModel, setClaudeModel] = useState(defaults.claude_model ?? "");
  const [codexModel, setCodexModel] = useState(defaults.codex_model ?? "");
  const [mode, setMode] = useState<PermMode | "">(defaults.mode ?? "");
  const save = () => {
    const d: AgentDefaults = {};
    if (kind) d.kind = kind;
    if (claudeModel) d.claude_model = claudeModel;
    if (codexModel) d.codex_model = codexModel;
    if (mode) d.mode = mode;
    send({ t: "session_settings", session: id, cwd: cwd.trim(), defaults: d });
    onDone();
  };
  return (
    <form className="poll-form scope-form" onSubmit={(e) => (e.preventDefault(), save())}>
      <h3>{t("scope.sessionTitle", { name })}</h3>
      <div className="field">
        <span>{t("scope.folder")}</span>
        <div className="scope-row">
          <input value={cwd} onChange={(e) => setCwd(e.target.value)} spellCheck={false} />
          <button type="button" className="scope-btn" onClick={() => void pickFolder().then((p) => p && setCwd(p))}>
            {t("scope.choose")}
          </button>
        </div>
        <span className="field-hint">{t("scope.folderHint")}</span>
      </div>
      <div className="field">
        <span>{t("scope.defaults")}</span>
        <span className="field-hint">{t("scope.defaultsHint")}</span>
        <div className="scope-grid">
          <span className="scope-k">{t("agent.kind")}</span>
          <div className="mode-seg three">
            {(["", "claude", "codex"] as const).map((k) => (
              <button key={k || "none"} type="button" className={kind === k ? "on" : ""} onClick={() => setKind(k)}>
                {k === "" ? t("scope.unset") : k === "claude" ? "Claude" : "Codex"}
              </button>
            ))}
          </div>
          <span className="scope-k">Claude {t("agent.model")}</span>
          <ModelSelect kind="claude" value={claudeModel} onChange={setClaudeModel} />
          <span className="scope-k">Codex {t("agent.model")}</span>
          <ModelSelect kind="codex" value={codexModel} onChange={setCodexModel} />
          <span className="scope-k">{t("perm.title")}</span>
          <div className="mode-seg four">
            <button type="button" className={mode === "" ? "on" : ""} onClick={() => setMode("")}>
              {t("scope.unset")}
            </button>
            {PERM_MODES.map((p) => (
              <button key={p.id} type="button" className={`${mode === p.id ? "on" : ""} m-${p.id}`} onClick={() => setMode(p.id)} title={p.hint}>
                {p.label}
              </button>
            ))}
          </div>
        </div>
      </div>
      <button className="add-submit" type="submit" disabled={!cwd.trim()}>
        {t("scope.save")}
      </button>
    </form>
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
