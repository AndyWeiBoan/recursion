// Sidebar footer: each account's 5-hour and weekly limits. Folds to one line; remembers which.
import { t } from "./i18n";
import { useEffect, useRef, useState } from "react";
import { send, useAccounts } from "./client";
import { openSettings } from "./Settings";
import type { Account } from "./types";
import { Collapse } from "./Collapse";
import { BrandMark, Icon } from "./icons";

const KEY = "recursion.usageOpen";
const WINDOWS: [string, string][] = [
  ["five_hour", "usage.5h"],
  ["seven_day", "usage.7d"],
];
const KIND = { claude: "Claude", codex: "Codex" };

const level = (v: number) => (v > 0.9 ? "hi" : v > 0.7 ? "mid" : "lo");

/** How long until a window resets ("2 小時"), from a reset time in seconds. */
function resetIn(at?: number) {
  if (!at) return "";
  const m = Math.round((at * 1000 - Date.now()) / 60000);
  if (m <= 0) return "";
  if (m < 60) return t("time.min", { n: m });
  const h = Math.round(m / 60);
  return h < 48 ? t("time.hour", { n: h }) : t("time.day", { n: Math.round(h / 24) });
}

/** "3 分鐘前": when an account's numbers were last read. */
function ago(ms: number) {
  const m = Math.floor((Date.now() - ms) / 60000);
  if (m < 1) return t("time.justNow");
  if (m < 60) return t("time.minAgo", { n: m });
  const h = Math.floor(m / 60);
  return h < 24 ? t("time.hourAgo", { n: h }) : t("time.dayAgo", { n: Math.floor(h / 24) });
}

const label = (a: Account) => `${KIND[a.kind]} · ${a.name}`;
const hasData = (a: Account) => WINDOWS.some(([k]) => a.usage[k] !== undefined);

export function UsagePanel() {
  const all = useAccounts();
  // Every account, one block each: limits are per login, not per agent.
  const shown = [...all].sort((a, b) => (a.kind === b.kind ? 0 : a.kind === "claude" ? -1 : 1));
  const [open, setOpen] = useState(() => {
    try {
      return localStorage.getItem(KEY) !== "0";
    } catch {
      return true;
    }
  });
  // Opening it asks the providers for fresh numbers (the daemon also polls every 5 minutes).
  useEffect(() => {
    if (open) send({ t: "accounts_refresh" });
  }, [open]);
  // ↻ asks every account again; it spins until new numbers arrive (or gives up after a while).
  const [refreshing, setRefreshing] = useState(false);
  const timer = useRef<number>();
  useEffect(() => {
    setRefreshing(false);
    window.clearTimeout(timer.current);
  }, [all]);
  const refresh = () => {
    send({ t: "accounts_refresh" });
    setRefreshing(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setRefreshing(false), 15000);
  };
  // Keeps "updated 3 minutes ago" current.
  const [, tick] = useState(0);
  useEffect(() => {
    if (!open) return;
    const id = window.setInterval(() => tick((n) => n + 1), 30000);
    return () => window.clearInterval(id);
  }, [open]);
  if (!shown.length) return null;
  const toggle = () =>
    setOpen((v) => {
      try {
        localStorage.setItem(KEY, v ? "0" : "1");
      } catch {}
      return !v;
    });
  return (
    <div className={`usage-panel${open ? " open" : ""}`}>
      <div className="usage-top">
        <button className="usage-head" onClick={toggle} aria-expanded={open} title={open ? t("usage.collapse") : t("usage.expand")}>
          <Icon name="usage" size={14} /> {t("usage.title")}
        </button>
        {open && (
          <button className={`usage-refresh${refreshing ? " spinning" : ""}`} onClick={refresh} title={t("usage.refresh")} aria-label={t("usage.refresh")}>
            <Icon name="reload" size={14} />
          </button>
        )}
      </div>
      <Collapse open={open}>
        <div className="usage-accts">
          {shown.map((a) => (
            <button key={a.id} className="usage-acct" onClick={() => openSettings("accounts")} title={t("usage.openAccounts", { who: a.email ?? t("acct.notLoggedIn") })}>
              <span className="usage-name">
                <BrandMark kind={a.kind} size={12} />
                {label(a)}
              </span>
              {a.email && a.email !== a.name && <span className="usage-email">{a.email}</span>}
              {!hasData(a) && <span className="usage-none">{a.logged_in ? t("usage.unavailable") : t("acct.notLoggedIn")}</span>}
              {hasData(a) && a.logged_in && a.expired && <span className="usage-none"><Icon name="key" size={12} /> {t("usage.stale")}</span>}
              {WINDOWS.filter(([k]) => a.usage[k] !== undefined).map(([k, name]) => {
                const v = Math.min(1, a.usage[k]);
                return (
                  <span key={k} className="usage-line" title={resetIn(a.resets?.[k]) ? t("usage.resetsIn", { when: resetIn(a.resets?.[k]) }) : undefined}>
                    <span className="usage-win">{t(name)}</span>
                    <span className="ctx-bar">
                      <i className={`ctx-fill ${level(v)}`} style={{ width: `${v * 100}%` }} />
                    </span>
                    <span className="usage-num">{Math.round(v * 100)}%</span>
                  </span>
                );
              })}
              {a.usage_at > 0 && (
                <span className="usage-at" title={new Date(a.usage_at).toLocaleString()}>
                  {t("usage.updated", { when: ago(a.usage_at) })}
                </span>
              )}
            </button>
          ))}
        </div>
      </Collapse>
    </div>
  );
}
