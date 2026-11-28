// An agent's info card, from its ⋯ menu: what it runs on, what Recursion tells it, and what its
// CLI says it has. Claude reports its tools, MCP servers, skills, subagents and plugins; Codex
// only its model, approval and sandbox, so its card says so where it has nothing. Floats and
// moves like a todo card.
import { useEffect, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { overlay } from "./browsers";
import { useAgentInfo, useWorkspace } from "./client";
import { t } from "./i18n";
import { Icon } from "./icons";
import { useCardPlace } from "./TodoCard";
import { PERM_MODES, type Member } from "./types";
import { Avatar, modelLabel } from "./WorkspaceView";

type Open = { panel: string; member: string };
let open: Open | null = null;
const subs = new Set<() => void>();
export function openAgentInfo(o: Open | null) {
  open = o;
  subs.forEach((f) => f());
}

/** The open card, if any. Mounted once, at the top of the app. */
export function AgentInfoLayer() {
  const o = useSyncExternalStore(
    (f) => (subs.add(f), () => subs.delete(f)),
    () => open,
  );
  return o ? <AgentInfoCard key={`${o.panel}/${o.member}`} {...o} /> : null;
}

const k = (n: number) => (n >= 1e6 ? `${+(n / 1e6).toFixed(1)}M` : n >= 1000 ? `${Math.round(n / 1000)}K` : String(n));
const list = (v: unknown): string[] => (Array.isArray(v) ? v.map((x) => (typeof x === "string" ? x : String((x as { name?: string })?.name ?? JSON.stringify(x)))) : []);
const show = (v: unknown) => (v == null || v === "" ? "—" : typeof v === "object" ? JSON.stringify(v) : String(v));

function AgentInfoCard({ panel, member }: Open) {
  const ws = useWorkspace(panel);
  const m = ws?.members.find((x) => x.id === member);
  const [refresh, setRefresh] = useState(0);
  const info = useAgentInfo(panel, member, refresh);
  const { place, drag, resize } = useCardPlace("recursion.agentCard");
  const close = () => openAgentInfo(null);
  useEffect(() => overlay(), []);
  useEffect(() => {
    if (ws && !m) close();
  }, [ws, m]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.isComposing) (e.stopPropagation(), close());
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  if (!m) return null;
  return createPortal(
    <div className="todo-card agent-card" style={{ left: place.x, top: place.y, width: place.w, height: place.h }} role="dialog" aria-label={`@${m.alias}`}>
      <header className="tc-head" onPointerDown={drag}>
        <Avatar m={m} size={18} />
        <span className="tc-title">
          @{m.alias} <span className="dim">{modelLabel(m)}</span>
        </span>
        <button onClick={() => setRefresh((n) => n + 1)} title={t("info.refresh")} aria-label={t("info.refresh")}>
          <Icon name="reload" size={14} />
        </button>
        <button className="tc-x" onClick={close} aria-label={t("common.close")} title={t("common.close") + "（Esc）"}>
          <Icon name="close" />
        </button>
      </header>
      <div className="tc-body info-body">{info ? <Body m={m} info={info} /> : <p className="dim">{t("info.loading")}</p>}</div>
      <div className="tc-grip" onPointerDown={resize} aria-hidden />
    </div>,
    document.body,
  );
}

function Body({ m, info }: { m: Member; info: NonNullable<ReturnType<typeof useAgentInfo>> }) {
  const cli = info.cli ?? {};
  const claude = info.kind === "claude";
  const PART: Record<string, string> = {
    system: t("info.part.system"),
    project: t("info.part.project"),
    rules: t("info.part.rules"),
    agent: t("info.part.agent"),
    members: t("info.part.members"),
    todos: t("info.part.todos"),
  };
  return (
    <>
      <section>
        <h4>{t("info.overview")}</h4>
        <dl className="info-grid">
          <dt>{t("agent.model")}</dt>
          <dd>{modelLabel(m)}</dd>
          <dt>{t("settings.accounts")}</dt>
          <dd>{info.account}</dd>
          <dt>{t("perm.title")}</dt>
          <dd>{PERM_MODES.find((p) => p.id === (m.mode ?? "edits"))?.label}</dd>
          {claude ? (
            <>
              <dt>Claude Code</dt>
              <dd>{show(cli.version)}</dd>
            </>
          ) : (
            <>
              <dt>Sandbox</dt>
              <dd>{show(cli.sandbox)}</dd>
              <dt>{t("info.effort")}</dt>
              <dd>{show(cli.reasoningEffort)}</dd>
            </>
          )}
          <dt>{t("info.context")}</dt>
          <dd>{m.context_window ? `${k(m.context_used ?? 0)} / ${k(m.context_window)}` : "—"}</dd>
          {m.cost_usd > 0 && (
            <>
              <dt>{t("info.cost")}</dt>
              <dd>${m.cost_usd.toFixed(2)}</dd>
            </>
          )}
        </dl>
      </section>

      <section>
        <h4>{t("info.reads")}</h4>
        <p className="dim info-note">{t("info.readsHint")}</p>
        {info.parts.map((p) => (
          <details key={p.id} className="info-part">
            <summary>
              {PART[p.id]}
              <span className="dim">{p.text.trim() ? t("info.chars", { n: [...p.text].length }) : t("info.none")}</span>
            </summary>
            {p.text.trim() ? <pre>{p.text.trim()}</pre> : <p className="dim">{t("info.none")}</p>}
          </details>
        ))}
      </section>

      <section>
        <h4>{t("info.has")}</h4>
        {!info.running ? (
          <p className="dim info-note">{t("info.notStarted")}</p>
        ) : claude ? (
          <>
            <Chips title={t("info.tools")} items={list(cli.tools)} />
            <Chips title="MCP" items={list(cli.mcp_servers).length ? (cli.mcp_servers as { name: string; status?: string }[]).map((s) => (s.status ? `${s.name} · ${s.status}` : s.name)) : []} />
            <Chips title="Skills" items={list(cli.skills)} />
            <Chips title={t("info.subagents")} items={list(cli.agents)} />
            <Chips title="Plugins" items={list(cli.plugins)} />
          </>
        ) : (
          <>
            <Chips title="MCP" items={["recursion", ...info.mcp]} />
            <p className="dim info-note">{t("info.codexLess")}</p>
          </>
        )}
      </section>
    </>
  );
}

function Chips({ title, items }: { title: string; items: string[] }) {
  return (
    <details className="info-part" open={items.length > 0 && items.length <= 12}>
      <summary>
        {title}
        <span className="dim">{items.length}</span>
      </summary>
      {items.length ? (
        <div className="info-chips">
          {items.map((x) => (
            <code key={x}>{x}</code>
          ))}
        </div>
      ) : (
        <p className="dim">{t("info.none")}</p>
      )}
    </details>
  );
}
