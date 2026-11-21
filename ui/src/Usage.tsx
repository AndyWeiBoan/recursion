// Sidebar footer: each account's 5-hour and weekly limits. Folds to one line; remembers which.
import { useEffect, useState } from "react";
import { send, useAccounts } from "./client";
import { openSettings } from "./Settings";
import type { Account } from "./types";

const KEY = "recursion.usageOpen";
const WINDOWS: [string, string][] = [
  ["five_hour", "5h"],
  ["seven_day", "週"],
];
const KIND = { claude: "Claude", codex: "Codex" };
// Same marks and colours as the agent picker.
const MARK = { claude: { glyph: "✳", color: "#e8714a" }, codex: { glyph: "◎", color: "#3fb8a8" } };

const level = (v: number) => (v > 0.9 ? "hi" : v > 0.7 ? "mid" : "lo");

/** How long until a window resets ("2 小時"), from a reset time in seconds. */
function resetIn(at?: number) {
  if (!at) return "";
  const m = Math.round((at * 1000 - Date.now()) / 60000);
  if (m <= 0) return "";
  if (m < 60) return `${m} 分`;
  const h = Math.round(m / 60);
  return h < 48 ? `${h} 小時` : `${Math.round(h / 24)} 天`;
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
      <button className="usage-head" onClick={toggle} aria-expanded={open} title={open ? "收起用量" : "展開用量"}>
        <span className={`sess-fold usage-fold${open ? "" : " closed"}`} aria-hidden="true">
          ›
        </span>
        <span>用量</span>
        {!open && (
          <span className="usage-sum">
            {shown.filter(hasData).map((a) => {
              const v = Math.max(...WINDOWS.map(([k]) => a.usage[k] ?? 0));
              return (
                <span key={a.id} className={`usage-pct ${level(v)}`} title={`${label(a)}：最高 ${Math.round(v * 100)}%`}>
                  <i style={{ color: MARK[a.kind].color }}>{MARK[a.kind].glyph}</i>
                  {a.name} {Math.round(v * 100)}%
                </span>
              );
            })}
          </span>
        )}
      </button>
      {open &&
        shown.map((a) => (
          <button key={a.id} className="usage-acct" onClick={() => openSettings("accounts")} title={`${a.email ?? "還沒登入"}　點一下打開帳號設定`}>
            <span className="usage-name">
              <i style={{ color: MARK[a.kind].color }}>{MARK[a.kind].glyph}</i>
              {label(a)}
            </span>
            {a.email && <span className="usage-email">{a.email}</span>}
            {!hasData(a) && <span className="usage-none">{a.logged_in ? "讀不到用量" : "還沒登入"}</span>}
            {WINDOWS.filter(([k]) => a.usage[k] !== undefined).map(([k, name]) => {
              const v = Math.min(1, a.usage[k]);
              return (
                <span key={k} className="usage-line">
                  <span className="usage-win">{name}</span>
                  <span className="ctx-bar">
                    <i className={`ctx-fill ${level(v)}`} style={{ width: `${v * 100}%` }} />
                  </span>
                  <span className="usage-num">{Math.round(v * 100)}%</span>
                  {resetIn(a.resets?.[k]) && (
                    <span className="usage-reset" title={`${resetIn(a.resets?.[k])}後重置`}>
                      ↻ {resetIn(a.resets?.[k])}
                    </span>
                  )}
                </span>
              );
            })}
          </button>
        ))}
    </div>
  );
}
