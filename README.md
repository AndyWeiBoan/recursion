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
- **討論串**：在 agent 的回覆上按右鍵討論整則，或反白一段文字討論那段；在分岔的對話裡跟原作者討論，主對話看不到；可搜尋（整則／片段），結論可帶回主對話。
- **投票**：輸入框左邊的 📊 發起（標題、描述、選項，自己也可以投）；每個 agent 用 `poll_vote` 選一個並附理由，全部投完才公開誰選了什麼，結果會告訴所有 agent；過去的投票在「📎 產出 → 投票」，agent 也能用 `poll_list`／`poll_read` 查。
- **回覆**：在任何訊息上按右鍵 →「回覆」，送出的訊息會帶著被回覆的那段，點一下跳回原訊息；agent 也會知道你在回哪一則。
- **agent 的瀏覽器**：每個 agent 在群聊旁有自己的瀏覽器 panel，用 `browse_open`／`browse_read`／`browse_click`／`browse_fill`／`browse_screenshot` 開網頁、填表單、截圖，你看得到它在做什麼（macOS、Linux 都可以）；網頁打不開時會回報錯誤。
- **多帳號**：設定 →「帳號」管理 Claude／Codex 的多個登入（每個帳號一個設定資料夾，設定、skills 等連回原本的 `~/.claude`／`~/.codex`），也可以直接加入這台電腦 CLI 原本的登入；顯示 email、方案、5 小時／7 天用量；加 agent 時選帳號；登入過期時聊天裡出現「重新登入」卡片。
- **MCP**：設定 →「MCP」是 Recursion 自己的一份 MCP 清單（`~/.recursion/mcp.json`，權限 0600），可以從 Claude／Codex 的設定勾選導入，或自己新增；每個都會實際連線測試，失敗看得到原因、可以重試。每個 session 在它的設定裡勾要用哪些（新 session 沿用上一次的勾選），agent 只拿到勾的那些、不用每次允許；CLI 自己設定的 MCP 不會給 agent。
- **最近關閉**：關掉 session 或群聊不會刪掉，側邊欄「最近關閉」可以打開回來，或在那裡永久刪除。同一個資料夾只會有一個 session。
- **第一次使用的引導**：檢查 Claude Code／Codex 有沒有裝，沒裝的話用官方安裝程式幫你裝（在 app 裡的終端看得到進度），然後加入帳號。
- **自動更新**（macOS）：新版發佈在 [recursion-releases](https://github.com/AndyWeiBoan/recursion-releases)，右下角會通知，在「設定 → 軟體資訊」更新，完成後自動重開。發版用 `scripts/release.sh`（在 Mac 上打包、簽章、上傳）。
- **Session／群聊設定**：session 的資料夾和新 agent 的預設（種類、模型、權限）；群聊的名稱、給所有 agent 的規則、花費上限。
- **模型選單**直接問 CLI（Claude 的 initialize、Codex 的 `model/list`），跟它們自己的 `/model` 一致。
- **畫圖**：agent 用 `diagram_write` 交出 JSON（架構圖、流程圖、時序圖），daemon 自己排版、拉線、畫成 SVG，有排不好的地方會告訴 agent 讓它改；圖在旁邊的 panel 打開，可以縮放、切深淺色、匯出 PNG／SVG，點方塊會標出跟它有關的線，也列在「📎 產出 → 圖」。想法和輸入格式參考 [Archify](https://github.com/tt-a1i/archify)（MIT），程式是我們自己的。
- **多語系**：設定 →「一般」→「語言」切換（繁體中文、English，或跟隨系統），馬上生效。要加一種語言：複製 `locales/en.json` 成 `locales/<代碼>.json`（例如 `ja.json`），改 `_name` 和裡面的字，重新編譯就會出現在選單裡；少翻的句子會顯示英文。`cargo test` 會檢查內建語言的 key 和 `{參數}` 都對得上。

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
pnpm test:thread # 討論串：分岔、記憶、不外洩、帶回主對話（KIND=codex 改測 Codex）
pnpm test:browse # agent 操作瀏覽器（會開一個真的 app 視窗；需要 pnpm dev 的 vite）
```

## 結構
| 路徑 | 內容 |
|---|---|
| `crates/recursiond` | 背景服務：版面模型、存檔、PTY、WebSocket（`~/.recursion/daemon.json` 記錄 port 與 token） |
| `src-tauri` | 桌面客戶端：確保 daemon 在跑，把位址交給前端 |
| `ui` | 前端（React + xterm.js），直接用 WebSocket 連 daemon |
| `locales` | 介面文字，一個語言一個 JSON 檔，前端和 daemon 共用 |
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
