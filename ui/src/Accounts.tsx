// Accounts window: every login of each CLI, who it is, how much of its limits is used, and a way
// to add one or log in again (the CLI's own login runs in a terminal window).
import { t } from "./i18n";
import { useEffect, useState } from "react";
import { send, useAccounts } from "./client";
import type { Account, AgentKind } from "./types";

const KINDS: { kind: AgentKind; name: string }[] = [
  { kind: "claude", name: "Claude" },
  { kind: "codex", name: "Codex" },
];

const WINDOWS: [string, string][] = [
  ["five_hour", "usage.5h.long"],
  ["seven_day", "usage.7d.long"],
];

function Usage({ a }: { a: Account }) {
  const shown = WINDOWS.filter(([k]) => a.usage[k] !== undefined);
  if (!shown.length) return null;
  return (
    <span className="acct-usage">
      {shown.map(([k, key]) => {
        const label = t(key);
        const v = Math.min(1, a.usage[k]);
        return (
          <span key={k} className="acct-meter" title={t("usage.meter", { label, pct: Math.round(v * 100) })}>
            <span className="dim">{label}</span>
            <span className="ctx-bar">
              <i className={`ctx-fill ${v > 0.9 ? "hi" : v > 0.7 ? "mid" : "lo"}`} style={{ width: `${v * 100}%` }} />
            </span>
            <span className="ctx-num">{Math.round(v * 100)}%</span>
          </span>
        );
      })}
    </span>
  );
}

function Row({ a, onLogin }: { a: Account; onLogin: () => void }) {
  const isDefault = a.id.endsWith("-default");
  const [editing, setEditing] = useState(false);
  const [confirm, setConfirm] = useState(false);
  return (
    <div className="acct-row">
      <span className={`acct-dot${a.logged_in ? " on" : ""}`} title={a.logged_in ? t("acct.loggedIn") : t("acct.notLoggedIn")} />
      <div className="acct-main">
        <div className="acct-name">
          {editing ? (
            <input
              autoFocus
              className="tree-edit"
              defaultValue={a.name}
              onBlur={(e) => (send({ t: "account_rename", account: a.id, name: e.currentTarget.value }), setEditing(false))}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
                if (e.key === "Escape") setEditing(false);
              }}
            />
          ) : (
            <b onDoubleClick={() => setEditing(true)} title={t("common.doubleClickRename")}>
              {a.name}
            </b>
          )}
          {isDefault && <span className="acct-tag">{t("acct.cliDefault")}</span>}
        </div>
        <span className="acct-who">{a.logged_in ? [a.email, a.plan].filter(Boolean).join(" · ") || t("acct.loggedIn") : t("acct.notLoggedIn")}</span>
        <Usage a={a} />
      </div>
      <div className="acct-acts">
        <button onClick={onLogin}>{a.logged_in ? t("acct.relogin") : t("acct.login")}</button>
        {!isDefault &&
          (confirm ? (
            <button className="danger" onClick={() => send({ t: "account_remove", account: a.id })} onMouseLeave={() => setConfirm(false)}>
              {t("common.confirmDelete")}
            </button>
          ) : (
            <button className="danger" onClick={() => setConfirm(true)}>
              {t("common.delete")}
            </button>
          ))}
      </div>
    </div>
  );
}

function AddAccount({ kind, onDone }: { kind: AgentKind; onDone: () => void }) {
  const [name, setName] = useState("");
  return (
    <form
      className="acct-add"
      onSubmit={(e) => {
        e.preventDefault();
        if (!name.trim()) return;
        send({ t: "account_create", kind, name: name.trim() });
        onDone();
      }}
    >
      <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={t("acct.namePlaceholder")} />
      <button type="submit" disabled={!name.trim()}>
        {t("acct.createAndLogin")}
      </button>
    </form>
  );
}

/** The accounts page of the settings. `onLeave` closes the settings when a login window opens. */
export function AccountsPane({ onLeave }: { onLeave: () => void }) {
  const accounts = useAccounts();
  const [adding, setAdding] = useState<AgentKind | null>(null);
  const login = (id: string) => (send({ t: "account_login", account: id }), onLeave());
  useEffect(() => send({ t: "accounts_refresh" }), []);
  return (
    <div className="acct">
      <p className="acct-hint">{t("acct.hint")}</p>
      {KINDS.map(({ kind, name }) => (
        <section key={kind} className="acct-group">
          <h3>{name}</h3>
          {accounts
            .filter((a) => a.kind === kind)
            .map((a) => (
              <Row key={a.id} a={a} onLogin={() => login(a.id)} />
            ))}
          {adding === kind ? (
            <AddAccount kind={kind} onDone={onLeave} />
          ) : (
            <button className="acct-new" onClick={() => setAdding(kind)}>
              {t("acct.add", { name })}
            </button>
          )}
        </section>
      ))}
    </div>
  );
}
