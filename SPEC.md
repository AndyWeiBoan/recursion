# Recursion — 設計規格（草稿 v0.5）

> 名字梗：AI 精神病圈的黑話「recursion / spiral」，同時是 agent 互相 @ 的遞迴。

## 定位
人類 + 多個 coding agent 的群聊工作台。agent 回覆可以是可互動的 HTML；
同一個 app 裡用 panel 開群聊、終端、網頁。

## 平台
- **正式支援：macOS、Linux（僅限有桌面環境）**
- 不支援：無桌面的 Linux
- Windows：暫不列入（之後再評估）

## 架構
- **全 Rust，不帶 Node**。兩個執行檔：
  - `recursiond`：常駐背景服務。session、對話、成員、agent 進程、PTY、權限、MCP 工具都在這裡。
  - Tauri app：客戶端，透過 WebSocket 連 `recursiond`。關掉 app 不影響 `recursiond`。
- 只做本機單人使用。**遠端使用者暫不考慮**（不在 v1、也不預留設計）。

### Agent 驅動（直接用使用者已安裝並登入的 CLI）
| Agent | 方式 |
|---|---|
| Claude | `claude -p --input-format stream-json --output-format stream-json --include-partial-messages`，`--resume` 續聊、`--fork-session` 開討論串、`--append-system-prompt` 注入群聊規則、`--mcp-config` 掛我們的工具、`--permission-prompt-tool` 把權限詢問導到我們的 UI |
| Codex | `codex app-server`（JSON-RPC，可中途請求核准，`thread/fork` 開討論串）。⚠️ 官方標 experimental |

## 版面模型（參考 tmux）
```
recursiond
└─ Session   一個專案：cwd、沙箱設定、允許路徑
   └─ Window  分頁，一種分割版面
      └─ Panel  三種：
           workspace  一個獨立的群聊（自己的 agent 成員、對話、討論串）
           terminal   終端（只給人用）
           browser    網站或本機 HTML
```
- 一個 session 可以有多個 workspace，各自獨立；agent 屬於 workspace。
- 所有 workspace 共用 session 的 cwd 與沙箱。
- 討論串是 workspace panel 的附屬內容，顯示在該 panel 右側的抽屜裡（不是另開 panel）。
- 版面持久化；關掉視窗 = detach，重開自動接回。

## 已決定
| 主題 | 決定 |
|---|---|
| 群聊 | 共用對話紀錄；可 @ 某 agent；agent 可設 alias；agent 也能 @ 別的 agent |
| 沒有 @ 時 | 人類訊息發給這個 workspace 的所有 agent |
| 回合制（取代 agent 互相觸發） | 人類發言開啟一個回合；被點到的 agent（有 @ 就是被 @ 的，沒有就是全部）**同時**作答、每人一次；回合結束後全部安靜等人類。agent 回覆裡的 @ 不會觸發對方，對方下一回合會看到。連續送出的人類訊息 1.5 秒內合併為同一回合；回合進行中送出的訊息排進下一回合，可按「中斷本回合，立即送出」。因不會循環，移除防遞迴上限，只保留 [PASS] |
| agent 可見內容 | 所有人的文字發言；看不到別人的工具呼叫細節 |
| agent 執行指令 | 用 CLI 自己的工具（Claude 的 Bash、Codex 的 shell）在**背景**執行，結果以 `⎿` 摘要顯示在對話裡。**agent 不開、不碰 terminal panel**；terminal panel 只給人用 |
| 權限 | 以 session cwd 為界，由 **OS 沙箱強制**（Claude Code sandbox：macOS Seatbelt / Linux bubblewrap；Codex sandbox）。cwd 內讀寫自動允許。cwd 外被 OS 拒絕 → agent 呼叫 `request_access(path)` → 原生對話框 / 資料夾選擇器 → 同意後加入允許清單並 resume。agent 的 shell 指令一律跳核准卡片（可勾「以後都允許這個指令」） |
| HTML 產出 | 工具呼叫：`show_widget`（對話內）、`open_pane`（開一個 browser panel）、`read_me`（按需載入設計規範），由內建 MCP server 提供給所有 agent。sandboxed iframe、邊串流邊畫。頁面 API：`recursion.reply(data)`、`recursion.sendPrompt(text)`、`recursion.ask(text)`。fallback：` ```html ` 區塊 |
| Session 文件（2026-11-20） | 長文、大表格、圖表不塞在對話裡：agent 呼叫 `doc_write(title, html, id?)` 存成 session 的 HTML 文件，對話裡只放一張卡片，第一次建立時自動在群聊旁邊分割出 panel 打開（不搶鍵盤焦點）。卡片點了就打開／聚焦那個 panel。**同一個 id 重寫 = 直接取代**，只留最新版（卡片標「更新了文件」），不做版本紀錄。群聊裡所有 agent 都會收到「@x 分享了文件 `id`」，可用 `doc_read`／`doc_list` 讀。內容存在 `~/.recursion/docs/<session>/<id>.html`，中繼資料在 session 裡；刪 session 一起刪。顯示用 sandbox iframe（`allow-scripts`，不給 `allow-same-origin`），不用原生 WebView，所以存再多份都不花資源，花資源的只有「正在顯示的 panel」。工具由 daemon 內建的 MCP server 提供（`/mcp/<每個 agent 一把 key>`，streamable HTTP 的最簡形式），Claude 走 `--mcp-config`、Codex 走 thread/start 的 `config.mcp_servers`；Codex 呼叫前的確認（elicitation）對自家 server 自動同意 |
| 討論串 | 可對任一段落 / 條列項 / 表格列 / 反白文字 / 整個 widget 開討論串。預設由該訊息作者回答，可 @ 他人。不觸發群發、其他 agent 看不到。實作：Claude `--resume --fork-session`、Codex `thread/fork` |
| 討論串介面 | workspace panel 右側抽屜，兩種畫面：**列表**（這個 panel 開過的所有討論串）→ 點進去是**討論串內容**（引用原文、對話、自己的輸入框、「↩ 帶回主對話」），左上「← 返回」回列表。列表項目顯示：引用原文開頭、負責的 agent、訊息數、狀態（回覆中／等你回／已帶回主對話），依最近動靜排序。**完成**：收到列表底部變灰、徽章變淡；**刪除**：底部提示「已刪除［復原］」，刪除後移除徽章並丟棄 agent 分支，已帶回的摘要保留。抽屜可收到右邊，只留細邊欄（數量 + 未讀點）。panel 太窄時抽屜改成覆蓋。💬 徽章掛在被引用的那一句旁邊；點徽章直接開該串；點引用原文→主對話捲到該處並高亮。快捷鍵：prefix + `t` 開關抽屜；抽屜內 `j/k` 移動、`Enter` 打開、`h`／`Backspace` 返回、`dd` 刪除 |
| 人用瀏覽器 | browser panel 用 OS 內建 WebView（macOS WKWebView、Linux WebKitGTK），Tauri 子 WebView，可開任意網站 |
| Agent 用瀏覽器 | **v1：隱藏的 OS WebView**，自寫控制層並以 MCP 工具提供：`browse_open`、`browse_read`（轉 Markdown）、`browse_click`、`browse_fill`、`browse_eval`、`browse_screenshot`。與 browser panel 共用 cookie／登入狀態；對話中顯示「🌐 @agent 正在瀏覽 …」+「在 panel 開啟」。**之後再評估**自研輕量 agentic browser（參考 Lightpanda 設計，不使用其 AGPL 程式碼） |
| 快捷鍵 | ① app 快捷鍵（macOS `⌘T/⌘D/⌘W/⌘1–9/⌘K`，Linux `Ctrl+Shift` 同等）；所有操作也能用滑鼠與指令面板完成 ② prefix 模式，預設 **`Ctrl+;`**（可改成單按右 ⌘），按下後顯示 which-key 提示：`hjkl` 移動、`s/v` 分割、`x` 關閉、`z` 放大、`c` 新 window、`n/p`、`1–9`、`w` workspace、`a` agent、`d` detach、`[` 捲動複製模式、`?` 說明 ③ nvim 式導航：**`Ctrl+H/J/K/L` 預設開啟**直接切 panel；終端前景程式是 nvim/vim 時交給它，此時用 prefix+`hjkl` 離開。workspace panel 有 vim 式操作（`Esc`、`j/k` 移動區塊、`Enter` 開討論串、`/` 搜尋、`i` 回輸入框）。終端有 vim 按鍵的捲動複製模式。**不做 nvim 外掛** |
| 介面風格 | 參考 Rex：左側依專案分組的 session 列表；workspace panel 是終端風格對話（`❯` 使用者輸入、`●` agent 發言、`⎿` 工具結果、底部狀態列）；對話內容是真的 HTML（表格、表單、草稿） |
| 多帳號 | 每個帳號一個設定資料夾 `~/.recursion/accounts/<id>/`，啟動 agent 時設 `CLAUDE_CONFIG_DIR` / `CODEX_HOME`（同 nodeterm 做法）。**共用全域設定**：`CLAUDE.md`、`settings.json`、skills、agents、MCP 設定以連結指回 `~/.claude/`（Codex 同理指回 `~/.codex/`），只有登入資料各自獨立。每個 provider 有一個預設帳號；新增 agent 時可選帳號；agent 標籤顯示帳號名。**agent 建立後不能換帳號**（對話紀錄存在帳號資料夾內）。帳號列表顯示 5h／7d 用量；快用完時**只提醒** |
| 帳號認證引導 | **首次開啟**：用 `claude auth status`／`codex login status` 偵測 CLI 是否安裝與登入，已登入的直接「匯入為帳號『預設』」；未安裝則顯示安裝指令 + 複製鈕 +「重新檢查」。**新增帳號**：① 選 Claude／Codex ② 取名 ③ 內嵌小終端跑 `claude auth login`／`codex login`（帶該帳號的設定資料夾），不自己模仿 CLI 流程；背景輪詢登入狀態 ④ 成功顯示偵測到的 email 與方案；email 與既有帳號重複時提醒。**登入過期**：agent 回報認證錯誤 → 對話中出現「帳號『X』需要重新登入［重新登入］」卡片 → 完成後 agent 自動接續 |
| 紀錄 | `~/.recursion/` |

## 技術風險
- Tauri `Webview::eval` 不回傳值 → agent 瀏覽器需用平台原生 API：macOS `WKWebView evaluateJavaScript:completionHandler:` / `takeSnapshot`；Linux `webkit_web_view_evaluate_javascript` / `get_snapshot`。**不可**對外部網站開放 Tauri IPC。
- Tauri 多 WebView（子 WebView）為 unstable feature；Linux 上最常出問題 → 早期先驗證。
- Codex app-server 與 claude stream-json 協定文件少，CLI 升版可能要跟著調整。
- 隱藏 WebView 記憶體約 50–100MB/頁 → 限制同時開啟頁數、閒置自動關閉。
- `CLAUDE_CONFIG_DIR` 不在 Claude Code 官方環境變數文件裡（社群與 nodeterm 都在用）；macOS 上登入資料存在 Keychain，要先驗證多個設定資料夾不會互相覆蓋。寫死 `~/.claude` 的 hook 會跨帳號。
- 偵測終端前景程序（決定 `Ctrl+H/J/K/L` 給誰）：macOS／Linux 用 `tcgetpgrp` + 程序名稱。

## 待決定
- **Lua 設定檔**（使用者需求，2026-11-20）：設定改用 Lua，參考 WezTerm / Neovim，`~/.config/recursion/init.lua`，Rust 端以 `mlua` 嵌入。待討論：開放哪些設定（快捷鍵、預設 agent 與模型、外觀、群聊規則、事件 hook）、熱重載、錯誤提示方式。

## v1 開發順序
| # | 里程碑 | 內容 | 完成標準 |
|---|---|---|---|
| M0 | 骨架 | `recursiond` + Tauri 客戶端、WebSocket、Session／Window／Panel 模型與持久化、detach／attach、Rex 風格外框 | 建立 session 與分割版面；關掉 app 再開，版面還在 |
| M1 | 單一 Claude | workspace panel、stream-json 驅動、串流 Markdown、權限卡片、resume；首次開啟偵測並匯入預設帳號；先驗證 `CLAUDE_CONFIG_DIR` 多帳號與 Keychain | 能跟一個 Claude 正常對話、改檔 |
| M2 | 終端與快捷鍵 | terminal panel（portable-pty + xterm.js）、三層快捷鍵、which-key、`Ctrl+H/J/K/L` 導航、捲動複製模式 | 不碰滑鼠就能開、分割、切換 panel |
| M3 | HTML widget | 內建 MCP server（rmcp）：`show_widget` / `open_pane` / `read_me`；iframe sandbox + 串流 morphdom；`reply` / `sendPrompt` | agent 丟出表單，填完資料回到 agent |
| M4 | 多 agent 群聊與多帳號 | Codex app-server、alias、@ 路由、群發、防遞迴五層、多個 workspace；帳號管理、新增帳號引導、新增 agent 選帳號、用量顯示與提醒、重新登入卡片 | Claude + Codex 能互相 @、會自動停；兩個 Claude 帳號的 agent 能在同一群聊 |
| M5 | OS 沙箱 | 沙箱設定、`request_access` 原生對話框、指令核准與記憶 | cwd 外存取需要同意 |
| M6 | 瀏覽器 | browser panel；agent 用隱藏 WebView + 原生 eval / snapshot 工具 | agent 能讀網頁、截圖；人能開任意網站 |
| M7 | 討論串 | 區塊錨點、討論串 panel、fork、帶回主對話 | 能對表格某列開串並帶回結論 |
| M8 | Linux + 打包 | Linux 桌面驗證、GitHub Actions 產出 dmg / AppImage / deb | 兩平台都能安裝執行 |
