# Recursion

人類 + 多個 coding agent（Claude、Codex）的群聊工作台。
agent 的回覆可以是可互動的 HTML；版面像 tmux：Session → Window → Panel（群聊、終端、瀏覽器）。

> 名字梗：AI 精神病圈的「recursion」黑話，也是 agent 互相 @ 的遞迴。

- 規格：[SPEC.md](SPEC.md)
- v1 開發文件：[docs/v1-design.html](docs/v1-design.html)

## 狀態
- **M0 骨架 + 終端**：session、window、可拖曳的分割 panel、多個終端；關掉 app 終端在背景繼續跑，重開版面還在。
- **M1 群聊接上 Claude**：在群聊 panel 加入 Claude（可選模型），串流回覆、Markdown、工具呼叫摘要、核准卡片（允許／以後都允許／拒絕）、停止、用量與花費；對話存檔，重開後接續。
- 瀏覽器 panel（M6）目前是佔位畫面。

## 開發
需要 Rust（`brew install rustup && rustup default stable`）、Node、pnpm。

```bash
pnpm install
pnpm dev        # 開發模式（會先編譯 recursiond）
pnpm app        # 打包 target/release/bundle/macos/Recursion.app
pnpm test       # Rust 單元測試 + 型別檢查 + daemon 端到端測試
pnpm test:agent # 用真的 Claude（haiku）跑群聊端到端測試，會用到少量額度
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
