// Settings window (⌘, / Ctrl+,): pages on the left, the page on the right.
import { useState, useSyncExternalStore } from "react";
import { AccountsPane } from "./Accounts";
import { isMac, send, useDaemonInfo, useSnapshot } from "./client";
import { languages, pick, t } from "./i18n";
import { Modal } from "./WorkspaceView";
import { UpdateSection, useAppVersion } from "./Update";
import { McpPane } from "./Mcp";
import { Icon, type IconName } from "./icons";

export type SettingsPage = "general" | "accounts" | "mcp" | "keys" | "about";
const PAGES: { id: SettingsPage; label: string; icon: IconName }[] = [
  { id: "general", get label() { return t("settings.general"); }, icon: "settings" },
  { id: "accounts", get label() { return t("settings.accounts"); }, icon: "user" },
  { id: "mcp", get label() { return t("settings.mcp"); }, icon: "plug" },
  { id: "keys", get label() { return t("settings.keys"); }, icon: "keyboard" },
  { id: "about", get label() { return t("settings.about"); }, icon: "info" },
];

// Open/closed lives outside React so the sidebar button and the keyboard shortcut share it.
let open: SettingsPage | null = null;
const subs = new Set<() => void>();
export function openSettings(page: SettingsPage = "general") {
  open = page;
  subs.forEach((f) => f());
}
function closeSettings() {
  open = null;
  subs.forEach((f) => f());
}
export function useSettingsOpen() {
  return useSyncExternalStore(
    (f) => (subs.add(f), () => subs.delete(f)),
    () => open,
  );
}

const mod = isMac ? "⌘" : "Ctrl+Shift+";
// Groups and descriptions are i18n keys.
const KEYS: { group: string; rows: [string, string][] }[] = [
  {
    group: "keys.layout",
    rows: [
      [isMac ? "⌘⇧N" : "Ctrl+Shift+N", "keys.newSession"],
      [`${mod}T`, "window.new"],
      [`${mod}D`, "panel.splitRight"],
      [isMac ? "⌘⇧D" : "Ctrl+Shift+Alt+D", "panel.splitDown"],
      [`${mod}W`, "keys.closePanel"],
      [`${mod}1–9`, "keys.gotoWindow"],
      [`${mod}[  ${mod}]`, "keys.prevNextWindow"],
      [`${mod}B`, "side.toggle"],
      ["Ctrl+H/J/K/L", "keys.movePanels"],
      [isMac ? "⌘C  ⌘V" : "Ctrl+Shift+C  Ctrl+Shift+V", "keys.termCopyPaste"],
      [isMac ? "⌘," : "Ctrl+,", "settings.title"],
      [isMac ? "⌘=  ⌘-  ⌘0" : "Ctrl+=  Ctrl+-  Ctrl+0", "keys.zoom"],
    ],
  },
  {
    group: "kind.workspace",
    rows: [
      ["Enter", "keys.send"],
      ["Shift+Enter", "keys.newline"],
      ["@", "keys.mention"],
      ["Esc", "keys.stopAgents"],
      ["keys.rightClick", "keys.rightClickWhat"],
      ["keys.select", "keys.selectWhat"],
    ],
  },
  {
    group: "keys.picker",
    rows: [
      ["T / G / B", "keys.pickerKinds"],
      ["↑ ↓  Enter", "keys.pickerMove"],
      ["Esc", "common.cancel"],
    ],
  },
];

function Keys() {
  return (
    <div className="set-keys">
      {KEYS.map((g) => (
        <section key={g.group}>
          <h3>{t(g.group)}</h3>
          <table>
            <tbody>
              {g.rows.map(([k, what]) => (
                <tr key={k + what}>
                  <td>
                    {k.split("  ").map((part) => (
                      <kbd key={part}>{part}</kbd>
                    ))}
                  </td>
                  <td>{t(what)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      ))}
    </div>
  );
}

export function ago(ms: number) {
  const m = Math.floor((Date.now() - ms) / 60000);
  if (m < 1) return t("time.justNow");
  if (m < 60) return t("time.minAgo", { n: m });
  const h = Math.floor(m / 60);
  return h < 24 ? t("time.hourAgo", { n: h }) : t("time.dayAgo", { n: Math.floor(h / 24) });
}

/** General settings: for now the language. */
function General() {
  const { state } = useSnapshot();
  const saved = state?.lang ?? null;
  return (
    <div className="set-general">
      <label className="field">
        <span className="field-label">{t("settings.language")}</span>
        <div className="select-wrap">
          <select value={saved ?? ""} onChange={(e) => send({ t: "set_lang", lang: e.target.value || null })}>
            <option value="">{t("settings.languageSystem", { name: languages.find((l) => l.code === pick(navigator.languages))?.name ?? "" })}</option>
            {languages.map((l) => (
              <option key={l.code} value={l.code}>
                {l.name}
              </option>
            ))}
          </select>
          <span className="chev" aria-hidden="true">
            <Icon name="dropdown" size={14} />
          </span>
        </div>
        <span className="field-hint">{t("settings.languageHint")}</span>
      </label>
    </div>
  );
}

function About() {
  const info = useDaemonInfo();
  const app = useAppVersion();
  const [confirm, setConfirm] = useState(false);
  const desktop = "__TAURI_INTERNALS__" in window;
  if (!info) return <p className="dim">{t("common.loading")}</p>;
  const rows: [string, string][] = [
    [t("about.version"), t("about.versionValue", { version: app ?? info.version, protocol: info.protocol })],
    [t("about.daemon"), t("about.daemonValue", { pid: info.pid, ago: ago(info.started) })],
    [t("about.home"), info.home],
    [t("about.exe"), info.exe ?? "—"],
  ];
  return (
    <div className="set-about">
      <table>
        <tbody>
          {rows.map(([k, v]) => (
            <tr key={k}>
              <th>{k}</th>
              <td>{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <UpdateSection />
      {desktop && (
        <div className="set-restart">
          <div>
            <b>{t("about.restart")}</b>
            <p className="dim">{t("about.restartHint")}</p>
          </div>
          {confirm ? (
            <button className="danger" onClick={() => (send({ t: "daemon_restart" }), closeSettings())} onMouseLeave={() => setConfirm(false)}>
              {t("about.confirmRestart")}
            </button>
          ) : (
            <button onClick={() => setConfirm(true)}>{t("about.restartButton")}</button>
          )}
        </div>
      )}
    </div>
  );
}

export function SettingsModal() {
  const page = useSettingsOpen();
  if (!page) return null;
  const title = PAGES.find((p) => p.id === page)!.label;
  return (
    <Modal title={t("settings.title")} onClose={closeSettings}>
      <div className="settings">
        <nav className="set-nav">
          <h2>{t("settings.title")}</h2>
          {PAGES.map((p) => (
            <button key={p.id} className={p.id === page ? "on" : ""} onClick={() => openSettings(p.id)}>
              <Icon name={p.icon} size={15} />
              {p.label}
            </button>
          ))}
        </nav>
        <div className="set-page">
          <h2>{title}</h2>
          {page === "general" && <General />}
          {page === "accounts" && <AccountsPane />}
          {page === "mcp" && <McpPane />}
          {page === "keys" && <Keys />}
          {page === "about" && <About />}
        </div>
      </div>
    </Modal>
  );
}
