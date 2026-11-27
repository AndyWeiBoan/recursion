// Accounts window: every login of each CLI, who it is, how much of its limits is used, and a way
// to add one or log in again (the CLI's own login runs in a setup terminal, see Setup.tsx).
import { t } from "./i18n";
import { useEffect, useState } from "react";
import { detectLogins, send, useAccounts } from "./client";
import { accountDetail, accountLabel, AddAccountChoices, KINDS } from "./Setup";
import type { Account, AgentKind } from "./types";
import { Icon } from "./icons";

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
  const [editing, setEditing] = useState(false);
  const [confirm, setConfirm] = useState(false);
  return (
    <div className="acct-row">
      {a.logged_in && a.expired ? (
        <span className="acct-dot-key" title={t(a.expired.why === "usage" ? "acct.expiredWhy.usage" : "acct.expiredWhy.agent", { time: new Date(a.expired.at).toLocaleString() })}>
          <Icon name="key" />
        </span>
      ) : (
        <span className={`acct-dot${a.logged_in ? " on" : ""}`} title={a.logged_in ? t("acct.loggedIn") : t("acct.notLoggedIn")} />
      )}
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
              {accountLabel(a)}
            </b>
          )}
        </div>
        <span className="acct-who">
          {!a.logged_in ? t("acct.notLoggedIn") : a.expired ? <b className="acct-expired">{t("acct.expired")}</b> : accountDetail(a) || t("acct.loggedIn")}
        </span>
        <Usage a={a} />
        {a.logged_in && a.expired && Object.keys(a.usage).length > 0 && <span className="usage-none">{t("usage.stale")}</span>}
      </div>
      <div className="acct-acts">
        <button className={a.logged_in && a.expired ? "primary" : undefined} onClick={onLogin}>
          {a.logged_in ? t("acct.relogin") : t("acct.login")}
        </button>
        {confirm ? (
          <button className="danger" onClick={() => (send({ t: "account_remove", account: a.id }), detectLogins())} onMouseLeave={() => setConfirm(false)}>
            {t("common.confirmDelete")}
          </button>
        ) : (
          <button className="danger" onClick={() => setConfirm(true)} title={a.id.endsWith("-default") ? t("acct.removeKeepsLogin") : undefined}>
            {t("acct.remove")}
          </button>
        )}
      </div>
    </div>
  );
}

/** The accounts page of the settings. */
export function AccountsPane() {
  const accounts = useAccounts();
  const [adding, setAdding] = useState<AgentKind | null>(null);
  const login = (id: string) => send({ t: "account_login", account: id });
  useEffect(() => (send({ t: "accounts_refresh" }), detectLogins()), []);
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
            <AddAccountChoices kind={kind} />
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
