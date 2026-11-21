// Settings window (⌘, / Ctrl+,): pages on the left, the page on the right.
import { useState, useSyncExternalStore } from "react";
import { AccountsPane } from "./Accounts";
import { isMac, send, useDaemonInfo } from "./client";
import { Modal } from "./WorkspaceView";

export type SettingsPage = "accounts" | "keys" | "about";
const PAGES: { id: SettingsPage; label: string; icon: string }[] = [
  { id: "accounts", label: "帳號", icon: "👤" },
  { id: "keys", label: "快捷鍵", icon: "⌨" },
  { id: "about", label: "關於", icon: "ⓘ" },
];

// Open/closed lives outside React so the sidebar button and the keyboard shortcut share it.
let open: SettingsPage | null = null;
const subs = new Set<() => void>();
export function openSettings(page: SettingsPage = "accounts") {
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
const KEYS: { group: string; rows: [string, string][] }[] = [
  {
    group: "版面",
    rows: [
      [isMac ? "⌘⇧N" : "Ctrl+Shift+N", "新 session（選專案資料夾）"],
      [`${mod}T`, "新 window"],
      [`${mod}D`, "左右分割"],
      [isMac ? "⌘⇧D" : "Ctrl+Shift+Alt+D", "上下分割"],
      [`${mod}W`, "關閉 panel"],
      [`${mod}1–9`, "切到第幾個 window"],
      [`${mod}[  ${mod}]`, "上一個／下一個 window"],
      [`${mod}B`, "顯示／隱藏側欄"],
      ["Ctrl+H/J/K/L", "在 panel 之間移動（終端前景是 vim／nvim 時交給它）"],
      [isMac ? "⌘C  ⌘V" : "Ctrl+Shift+C  Ctrl+Shift+V", "在終端機裡複製、貼上"],
      [isMac ? "⌘," : "Ctrl+,", "設定"],
    ],
  },
  {
    group: "群聊",
    rows: [
      ["Enter", "送出"],
      ["Shift+Enter", "換行"],
      ["@", "指定要問哪個 agent（沒有 @ 就問全部）"],
      ["Esc", "停止正在回覆的 agent"],
      ["在訊息上按右鍵", "回覆它、針對整則或選取的那段開討論串、複製"],
      ["反白一段字", "針對那段開討論串"],
    ],
  },
  {
    group: "選 panel 種類時",
    rows: [
      ["T / G / B", "終端／群聊／瀏覽器"],
      ["↑ ↓  Enter", "移動、選取"],
      ["Esc", "取消"],
    ],
  },
];

function Keys() {
  return (
    <div className="set-keys">
      {KEYS.map((g) => (
        <section key={g.group}>
          <h3>{g.group}</h3>
          <table>
            <tbody>
              {g.rows.map(([k, what]) => (
                <tr key={k + what}>
                  <td>
                    {k.split("  ").map((part) => (
                      <kbd key={part}>{part}</kbd>
                    ))}
                  </td>
                  <td>{what}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      ))}
    </div>
  );
}

function ago(ms: number) {
  const m = Math.floor((Date.now() - ms) / 60000);
  if (m < 1) return "剛剛";
  if (m < 60) return `${m} 分鐘前`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h} 小時前` : `${Math.floor(h / 24)} 天前`;
}

function About() {
  const info = useDaemonInfo();
  const [confirm, setConfirm] = useState(false);
  const desktop = "__TAURI_INTERNALS__" in window;
  if (!info) return <p className="dim">讀取中…</p>;
  const rows: [string, string][] = [
    ["版本", `${info.version}（協定 ${info.protocol}）`],
    ["背景服務", `執行中，pid ${info.pid}，${ago(info.started)}啟動`],
    ["資料資料夾", info.home],
    ["執行檔", info.exe ?? "—"],
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
      {desktop && (
        <div className="set-restart">
          <div>
            <b>重啟背景服務</b>
            <p className="dim">換上新編譯的 daemon 用。版面和對話都會保留，但開著的終端會被關掉、正在回覆的 agent 會中斷。</p>
          </div>
          {confirm ? (
            <button className="danger" onClick={() => (send({ t: "daemon_restart" }), closeSettings())} onMouseLeave={() => setConfirm(false)}>
              確定重啟？
            </button>
          ) : (
            <button onClick={() => setConfirm(true)}>重啟</button>
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
    <Modal title="設定" onClose={closeSettings}>
      <div className="settings">
        <nav className="set-nav">
          <h2>設定</h2>
          {PAGES.map((p) => (
            <button key={p.id} className={p.id === page ? "on" : ""} onClick={() => openSettings(p.id)}>
              <span aria-hidden="true">{p.icon}</span>
              {p.label}
            </button>
          ))}
        </nav>
        <div className="set-page">
          <h2>{title}</h2>
          {page === "accounts" && <AccountsPane onLeave={closeSettings} />}
          {page === "keys" && <Keys />}
          {page === "about" && <About />}
        </div>
      </div>
    </Modal>
  );
}
