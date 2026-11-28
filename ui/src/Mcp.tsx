// Settings › MCP: the MCP servers agents can be given. Copied from Claude's and Codex's settings or
// added by hand, each tested by connecting to it; a session then turns on the ones it wants.
import { useState } from "react";
import { send, useMcp, type McpServer, type McpStatus } from "./client";
import { t } from "./i18n";
import { Icon } from "./icons";

/** Where a server was copied from, for people. */
function fromLabel(from?: string | null) {
  if (!from) return t("mcp.fromHand");
  if (from === "codex") return "Codex";
  if (from === "claude") return t("mcp.fromClaudeAll");
  return t("mcp.fromClaudeProject", { path: from.slice("claude:".length).replace(/^\/(Users|home)\/[^/]+/, "~") });
}

function what(s: McpServer) {
  return s.command ? [s.command, ...(s.args ?? [])].join(" ") : (s.url ?? "");
}

export function StatusDot({ status }: { status?: McpStatus }) {
  const cls = status?.state === "ok" ? "ok" : status?.state === "failed" ? "bad" : status?.state === "testing" ? "busy" : "";
  return <span className={`mcp-dot ${cls}`} aria-hidden="true" />;
}

export function statusLine(s?: McpStatus) {
  if (!s) return t("mcp.untested");
  if (s.state === "testing") return t("mcp.testing");
  if (s.state === "ok") return t("mcp.ok", { n: s.tools ?? 0 });
  return t("mcp.failed");
}

type Pair = { k: string; v: string; kept: boolean };

function Pairs({ label, hint, rows, onChange }: { label: string; hint: string; rows: Pair[]; onChange: (rows: Pair[]) => void }) {
  const set = (i: number, p: Partial<Pair>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...p } : r)));
  return (
    <div className="field">
      <span className="field-label">{label}</span>
      {rows.map((r, i) => (
        <div key={i} className="mcp-pair">
          <input value={r.k} onChange={(e) => set(i, { k: e.target.value })} placeholder={t("mcp.key")} spellCheck={false} />
          <input
            type="password"
            value={r.v}
            onChange={(e) => set(i, { v: e.target.value, kept: false })}
            placeholder={r.kept ? t("mcp.valueKept") : t("mcp.value")}
            autoComplete="off"
            spellCheck={false}
          />
          <button type="button" onClick={() => onChange(rows.filter((_, j) => j !== i))} aria-label={t("common.delete")} title={t("common.delete")}>
            <Icon name="close" />
          </button>
        </div>
      ))}
      <button type="button" className="acct-new" onClick={() => onChange([...rows, { k: "", v: "", kept: false }])}>
        <Icon name="add" size={14} /> {t("mcp.addPair")}
      </button>
      <span className="field-hint">{hint}</span>
    </div>
  );
}

const toPairs = (m?: Record<string, string | null>): Pair[] => Object.keys(m ?? {}).map((k) => ({ k, v: "", kept: true }));
// A kept value goes back as null: "unchanged" (the page never has it).
const fromPairs = (rows: Pair[]) => Object.fromEntries(rows.filter((r) => r.k.trim()).map((r) => [r.k.trim(), r.kept && !r.v ? null : r.v]));

function Form({ name: oldName, server, onDone }: { name?: string; server?: McpServer; onDone: () => void }) {
  const [name, setName] = useState(oldName ?? "");
  const [http, setHttp] = useState(!!server?.url);
  const [command, setCommand] = useState(server?.command ?? "");
  const [args, setArgs] = useState((server?.args ?? []).join("\n"));
  const [url, setUrl] = useState(server?.url ?? "");
  const [sse, setSse] = useState(!!server?.sse);
  const [env, setEnv] = useState<Pair[]>(toPairs(server?.env));
  const [headers, setHeaders] = useState<Pair[]>(toPairs(server?.headers));
  const okName = /^[A-Za-z0-9_-]+$/.test(name.trim()) && name.trim() !== "recursion";
  const ok = okName && (http ? /^https?:\/\//.test(url.trim()) : !!command.trim());
  const save = () => {
    const s = http
      ? { url: url.trim(), headers: fromPairs(headers), sse }
      : { command: command.trim(), args: args.split("\n").map((a) => a.trim()).filter(Boolean), env: fromPairs(env) };
    send({ t: "mcp_save", name: name.trim(), old: oldName, server: s });
    onDone();
  };
  return (
    <form className="mcp-form" onSubmit={(e) => (e.preventDefault(), ok && save())}>
      <label className="field">
        <span className="field-label">{t("mcp.name")}</span>
        <input autoFocus={!oldName} value={name} onChange={(e) => setName(e.target.value)} placeholder="github" spellCheck={false} />
        <span className={`field-hint${name && !okName ? " warn" : ""}`}>{t("mcp.nameHint")}</span>
      </label>
      <div className="mode-seg two">
        <button type="button" className={!http ? "on" : ""} onClick={() => setHttp(false)}>
          {t("mcp.stdio")}
        </button>
        <button type="button" className={http ? "on" : ""} onClick={() => setHttp(true)}>
          {t("mcp.http")}
        </button>
      </div>
      {http ? (
        <>
          <label className="field">
            <span className="field-label">{t("mcp.url")}</span>
            <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://…/mcp" spellCheck={false} />
          </label>
          <label className="mcp-check">
            <input type="checkbox" checked={sse} onChange={(e) => setSse(e.target.checked)} />
            {t("mcp.sse")}
          </label>
          <Pairs label={t("mcp.headers")} hint={t("mcp.headersHint")} rows={headers} onChange={setHeaders} />
        </>
      ) : (
        <>
          <label className="field">
            <span className="field-label">{t("mcp.command")}</span>
            <input value={command} onChange={(e) => setCommand(e.target.value)} placeholder="npx" spellCheck={false} />
          </label>
          <label className="field">
            <span className="field-label">{t("mcp.args")}</span>
            <textarea rows={3} value={args} onChange={(e) => setArgs(e.target.value)} placeholder={"-y\n@modelcontextprotocol/server-github"} spellCheck={false} />
            <span className="field-hint">{t("mcp.argsHint")}</span>
          </label>
          <Pairs label={t("mcp.env")} hint={t("mcp.envHint")} rows={env} onChange={setEnv} />
        </>
      )}
      <div className="mcp-form-foot">
        <button type="button" onClick={onDone}>
          {t("common.cancel")}
        </button>
        <button className="primary" type="submit" disabled={!ok}>
          {t("mcp.saveTest")}
        </button>
      </div>
    </form>
  );
}

function Row({ name, server, status }: { name: string; server: McpServer; status?: McpStatus }) {
  const [editing, setEditing] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [open, setOpen] = useState(false);
  if (editing) return <Form name={name} server={server} onDone={() => setEditing(false)} />;
  return (
    <div className="acct-row mcp-row">
      <StatusDot status={status} />
      <div className="acct-main">
        <div className="acct-name">
          <b>{name}</b>
          <span className="acct-tag">{fromLabel(server.from)}</span>
        </div>
        <span className="acct-who mono">{what(server)}</span>
        <span className={`mcp-status ${status?.state ?? ""}`}>
          {statusLine(status)}
          {status?.state === "failed" && status.error && (
            <button className="mcp-more" onClick={() => setOpen(!open)}>
              {open ? t("common.collapse") : t("mcp.why")}
            </button>
          )}
        </span>
        {open && status?.state === "failed" && <pre className="mcp-error">{status.error}</pre>}
      </div>
      <div className="acct-acts">
        <button disabled={status?.state === "testing"} onClick={() => send({ t: "mcp_test", name })}>
          {status?.state === "failed" ? t("mcp.retry") : t("mcp.test")}
        </button>
        <button onClick={() => setEditing(true)}>{t("mcp.edit")}</button>
        {confirm ? (
          <button className="danger" onClick={() => send({ t: "mcp_remove", name })} onMouseLeave={() => setConfirm(false)}>
            {t("common.confirmDelete")}
          </button>
        ) : (
          <button className="danger" onClick={() => setConfirm(true)}>
            {t("common.delete")}
          </button>
        )}
      </div>
    </div>
  );
}

/** What Claude and Codex have set up: pick which to copy in. */
function Import() {
  const m = useMcp();
  const found = m?.found ?? [];
  const key = (f: { name: string; from: string }) => `${f.from}\n${f.name}`;
  const [picked, setPicked] = useState<Set<string> | null>(null);
  // Not yet copied in starts ticked.
  const ticks = picked ?? new Set(found.filter((f) => f.state === "new").map(key));
  if (!found.length) return <p className="dim mcp-none">{t("mcp.foundNone")}</p>;
  const toggle = (k: string) => {
    const next = new Set(ticks);
    if (!next.delete(k)) next.add(k);
    setPicked(next);
  };
  return (
    <div className="mcp-import">
      {found.map((f) => (
        <label key={key(f)} className="mcp-found">
          <input type="checkbox" checked={ticks.has(key(f))} onChange={() => toggle(key(f))} />
          <span className="acct-main">
            <span className="acct-name">
              <b>{f.name}</b>
              <span className="acct-tag">{fromLabel(f.from)}</span>
              {f.state !== "new" && <span className={`acct-tag ${f.state}`}>{t(f.state === "same" ? "mcp.imported" : "mcp.changed")}</span>}
            </span>
            <span className="acct-who mono">{what(f.server)}</span>
          </span>
        </label>
      ))}
      <button
        className="primary mcp-import-btn"
        disabled={!ticks.size}
        onClick={() => {
          send({ t: "mcp_import", picks: found.filter((f) => ticks.has(key(f))).map((f) => [f.name, f.from]) });
          setPicked(new Set());
        }}
      >
        {t("mcp.importPicked", { n: ticks.size })}
      </button>
    </div>
  );
}

export function McpPane() {
  const m = useMcp();
  const [adding, setAdding] = useState(false);
  if (!m) return <p className="dim">{t("common.loading")}</p>;
  const names = Object.keys(m.servers);
  return (
    <div className="acct mcp">
      <p className="acct-hint">{t("mcp.hint")}</p>
      <section className="acct-group">
        <h3>{t("mcp.mine")}</h3>
        {!names.length && !adding && <p className="dim mcp-none">{t("mcp.none")}</p>}
        {names.map((n) => (
          <Row key={n} name={n} server={m.servers[n]} status={m.status[n]} />
        ))}
        {adding ? (
          <Form onDone={() => setAdding(false)} />
        ) : (
          <button className="acct-new" onClick={() => setAdding(true)}>
            <Icon name="add" size={14} /> {t("mcp.add")}
          </button>
        )}
      </section>
      <section className="acct-group">
        <h3>{t("mcp.found")}</h3>
        <p className="acct-hint">{t("mcp.foundHint")}</p>
        <Import />
      </section>
    </div>
  );
}

/** A session's MCP servers: tick the ones its agents get. */
