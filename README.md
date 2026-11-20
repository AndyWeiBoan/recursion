# Recursion

人類 + 多個 coding agent（Claude、Codex）的群聊工作台。
agent 的回覆可以是可互動的 HTML；版面像 tmux：Session → Window → Panel（群聊、終端、瀏覽器）。

> 名字梗：AI 精神病圈的「recursion」黑話，也是 agent 互相 @ 的遞迴。

- 規格：[SPEC.md](SPEC.md)
- v1 開發文件：[docs/v1-design.html](docs/v1-design.html)

## 狀態
- **M0 骨架 + 終端**：session、window、可拖曳的分割 panel、多個終端；關掉 app 終端在背景繼續跑，重開版面還在。
- **M1 群聊接上 Claude**：在群聊 panel 加入 Claude（可選模型），串流回覆、Markdown、工具呼叫摘要、核准卡片（允許／以後都允許／拒絕）、停止、用量與花費；對話存檔，重開後接續。
- **多 agent 群聊（回合制）**：你發言開啟一個回合，被點到的 agent（有 @ 就是被 @ 的，沒有就是全部）同時作答、每人一次，然後等你；agent 之間的 @ 不會觸發對方，下一回合才會看到。1.5 秒內連續送出的訊息合併成同一回合；回合中送出的訊息排進下一回合，可「中斷本回合，立即送出」。聊天版面（你在右、agent 在左）、輸入 @ 跳出成員選單、群聊中途可加人。
- **Codex**：群聊裡可以加入 Codex（`codex app-server`，沙箱 workspace-write、需要越權時會跳核准），跟 Claude 互相 @。
- **瀏覽器 panel**：系統內建的瀏覽器引擎（macOS WKWebView、Linux WebKitGTK），上一頁／下一頁／重新載入，記住最後的網址。
- **Session 文件**：agent 用 `doc_write` 把長文、大表格、圖表存成 HTML 文件，對話裡是一張卡片，自動在旁邊的 panel 打開；其他 agent 也讀得到（`doc_read`）。
- **模型選單**直接問 CLI（Claude 的 initialize、Codex 的 `model/list`），跟它們自己的 `/model` 一致。

## 開發
需要 Rust（`brew install rustup && rustup default stable`）、Node、pnpm。
Linux 另外要 WebKitGTK：Fedora `sudo dnf install webkit2gtk4.1-devel libsoup3-devel librsvg2-devel openssl-devel`。

```bash
pnpm install
pnpm dev        # 開發模式（會先編譯 recursiond）
pnpm app        # 打包 target/release/bundle/macos/Recursion.app
pnpm test       # Rust 單元測試 + 型別檢查 + daemon 端到端測試
pnpm test:agent # 用真的 Claude（haiku）跑單一 agent 端到端測試，會用到少量額度
pnpm test:group # 兩個真的 Claude 的回合制端到端測試
pnpm test:codex # Codex + Claude 同一個群聊的端到端測試
pnpm test:doc   # Claude 寫 session 文件、Codex 讀回來（真的 agent，會用到少量額度）
```

## 結構
| 路徑 | 內容 |
|---|---|
| `crates/recursiond` | 背景服務：版面模型、存檔、PTY、WebSocket（`~/.recursion/daemon.json` 記錄 port 與 token） |
| `src-tauri` | 桌面客戶端：確保 daemon 在跑，把位址交給前端 |
| `ui` | 前端（React + xterm.js），直接用 WebSocket 連 daemon |
| `scripts/smoke.mjs` | daemon 端到端測試 |

## 快捷鍵（M0）
| macOS | Linux | 動作 |
|---|---|---|
| `⌘⇧N` | `Ctrl+Shift+N` | 新 session（選專案資料夾） |
| `⌘T` | `Ctrl+Shift+T` | 新 window |
| `⌘D` / `⌘⇧D` | `Ctrl+Shift+D` / `Ctrl+Shift+Alt+D` | 左右 / 上下分割 |
| `⌘W` | `Ctrl+Shift+W` | 關閉 panel |
| `⌘1–9`、`⌘[` `⌘]` | `Ctrl+Shift+1–9`、`[` `]` | 切換 window |
| `⌘B` | `Ctrl+Shift+B` | 顯示 / 隱藏側欄 |
| `Ctrl+H/J/K/L` | 同 | 在 panel 間移動（終端前景是 nvim/vim 時交給它） |

停止背景服務：`kill $(python3 -c "import json;print(json.load(open('$HOME/.recursion/daemon.json'))['pid'])")`
