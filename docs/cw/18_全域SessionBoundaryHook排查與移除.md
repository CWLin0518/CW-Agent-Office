# 全域 Session Boundary Hook 排查與移除

> 日期：2026-09-03
> 狀態：已排查完畢，使用者已移除該全域 hook；專案內建版本尚未設計/實作
> 現象：使用者原本想要「多階段任務執行到一半，Agent 自己判斷該開新 session 接手時，能自動把新 session 開出來」，用的是**個人機器上的全域 Claude Code hook**（不屬於這個 repo）。實測多次出現「Agent 判定邊界、寫了交接檔，但沒有真的開出新 session」的狀況。

## 1. 結論

1. 這個機制**從頭到尾都不是 GT Office 專案的一部分**：它是使用者個人的全域 Claude Code 設定（`~/.claude/hooks/`、`~/.claude/skills/`、`~/.claude/settings.json`），只是在偵測到 GT Office 有在跑時，會借用 GT Office 內建的本機 bridge (`tool.launch`) 來開新 session；GT Office 沒在跑就整個退化成「開一個跟 GT Office 無關的獨立 PowerShell 視窗」。
2. 排查過程中確實抓到一個真的 bug（訊號比對規則寫死），已修好；但修好之後又發現兩個**跟 hook 本身無關、屬於「這條路徑天生的設計限制」**的問題：
   - 透過這條 bridge 路徑開的新 session，後端會真的建立、註冊，但**畫面上不會自動出現卡片**（這條路徑不綁定 webview 視窗、不 emit 前端在聽的更新事件，是程式碼註解裡明講的已知落差，不是意外）。
   - 這條路徑要不要「視為同一個 Agent 重啟」，完全取決於觸發當下的環境變數（`GTO_WORKSPACE_ID`/`GTO_AGENT_ID`/`GTO_STATION_ID`）有沒有被正確帶進來；只要觸發來源不是那個 Agent 自己的站台終端機，就會生出一個身分無關的全新匿名 Agent。
3. 基於以上，使用者判斷這條路徑不值得繼續修補，決定**移除這個全域 hook**，改為之後在 GT Office 專案內建「自動判斷任務邊界並開新 session」的正式功能。本文件記錄排查過程、證據與移除後的殘留狀態，作為未來要在專案內設計這個功能時的背景資料。

## 2. 原本的機制長什麼樣

- `~/.claude/skills/session-boundary-planner/SKILL.md`：定義一套「多階段任務規劃 + session boundary 判斷規則」。Agent 在**執行過程中**（不只是規劃階段）只要自己判定符合 Session Boundary Rules 任一條，就可以：
  1. 用 `Write` 工具把交接內容寫到固定路徑 `.claude/session-handoff/<phase-slug>-handoff.md`；
  2. 內容第一行放固定訊號 `<SESSION_CONTROL action="new_session" />`。
  - SKILL.md 原文明講：「這個自動化機制本身沒有審批關卡，是這個環境的使用者刻意接受的權衡（個人機器、圖方便）」——也就是說，任何看得到這個全域 skill 的 Claude session，理論上都可能在使用者沒有明確下指令的情況下，自己判斷「到邊界了」就觸發一次真實的新 session 啟動。
- `~/.claude/hooks/session-boundary-trigger.js`：全域 `PostToolUse` hook（matcher: `Write`），偵測到上述「路徑 + 訊號」同時成立時：
  1. 優先讀 `~/.gtoffice/mcp/runtime.json` 拿到 GT Office 內建 bridge 的連線資訊，呼叫 `tool.launch` 方法（跟 GT Office UI 上「開新 Claude session」走同一條後端路徑）；
  2. bridge 連不上（GT Office 沒在跑）就退回用 Windows 工作排程器（schtasks）開一個獨立的 PowerShell 視窗執行 `claude` CLI。

## 3. 排查過程與證據

### 3.1 Bug #1：訊號比對規則寫死，agent 用詞一有出入就整個不觸發（已修正，現已隨檔案一起移除）

- `SKILL.md` 規定訊號字串必須精確等於 `<SESSION_CONTROL action="new_session" />`，hook 原本的 `SESSION_CONTROL_TAG_PATTERN` 也是寫死比對 `action="new_session"`。
- 實測時某次 agent 自己代換成語意相近但沒定義過的值 `action="restart_in_place"`，比對不到，hook 判定「沒有訊號」直接靜默結束（`process.exit(0)`）——不呼叫 bridge、不退回 fallback、也不回傳任何訊息給 agent 或使用者，使用者只會看到「Agent 判定了邊界、寫了 handoff，但什麼都沒發生」。
- 修法：既然 hook 對每個 action 值的處理邏輯完全相同（`main()` 從未讀取捕獲到的 action 值），把 regex 放寬成「只要有這個標籤、action 屬性非空字串」就觸發，不再要求精確等於 `new_session`。

### 3.2 加了診斷 log 之後，才分辨出後兩次「沒觸發」根本不是同一個問題

Bug #1 修好後，使用者又測到兩次「沒開新 session」，為了不再靠檔案時間戳/殘留物事後推敲，在 hook 裡加了逐行 JSON 診斷 log（`~/.claude/session-boundary-trigger.log`，含 `pid`／`stage`／關鍵欄位），之後才分辨出：

- **第一次「沒觸發」其實是走錯機制**：agent 這次用的是 `wrap-up` 這個 skill（整理進度、寫到 GT Office 的 agent 輸出目錄 `.gtoffice/agents/<agent-id>/outputs/`），根本不是 `session-boundary-planner` 的交接流程，寫入的路徑、內容格式都不符合 hook 期待的模式，log 顯示 `skip_path_not_handoff`——hook **正確**判斷這不是要交接的訊號，不是 bug。
- **第二次明確重現（指定路徑 + 指定訊號，排除掉「agent 自己選錯機制」這個變因）才真的觸發成功**：log 依序出現 `signal_matched` → `bridge_ok` → `done`（`mode: "bridge"`, `launchOk: true`），整段流程 2 秒內完成。

### 3.3 bridge_ok 之後，畫面上還是沒有卡片——定位到兩個「設計限制」而非 bug

用 `ListAgents` 確認：`bridge_ok` 那次呼叫確實在後端建立了一個真實存在、可互動的新 Claude session（`agent-94`，啟動時間與 log 時間吻合）。但 GT Office 畫面上完全沒有出現對應的新卡片。追進 Tauri 後端程式碼後找到兩個原因：

**(a) 這條 bridge 路徑本身不會更新前端畫面**

`apps/desktop-tauri/src-tauri/src/local_bridge.rs:475-492`（`bridge_tool_launch` 函式上方註解，屬於既有程式碼、非本次新增）：

> Known gap vs. `workspace_open`: this does not bind any webview `Window` to the workspace or emit `workspace/updated` / `workspace/active_changed`, because a bridge caller has no window of its own to bind. If the target workspace isn't already open in some window, the new session/runtime is still created and registered correctly, but it may not visibly appear until the user opens that workspace through the normal UI themselves.

也就是說：這是外部呼叫者（一支跑在 GT Office 進程之外的 hook script）借用 GT Office 後端能力時的已知限制，不是這次排查才發現的新 bug。

**(b) 「重啟同一個 Agent」還是「開全新匿名 Agent」，取決於觸發當下有沒有帶對環境變數**

`apps/desktop-tauri/src-tauri/src/commands/tool_adapter/tool_profiles.rs:612-613`（`build_runtime_identity`）：呼叫時的 `context` 有帶 `agentId`/`stationId` 才會沿用既有身分；沒帶就用 `Uuid::new_v4()` 生成全新的匿名 id（`dock-claude-xxxxxxxx` 這種格式），並且**每次都會建立一個全新的底層 terminal session**（`state.terminal_provider.create_session(...)`，無條件執行，不檢查是否已有同 id 的 session 在跑）。

hook 這邊只有在「觸發它的那個 `claude` 進程本身是 GT Office 幫某個 Agent 站台開的終端機」時，才會繼承到 `GTO_WORKSPACE_ID`/`GTO_AGENT_ID`/`GTO_STATION_ID` 這三個環境變數並轉傳給 `tool.launch`。這次的最小重現是在**工作區根目錄**（`agent連線協做測試\.claude\session-handoff\`）下執行 `Write`，不是在 test-agent 自己的站台子目錄（`agent連線協做測試\.gtoffice\test-agent\...`）裡，所以那三個變數不存在，hook 呼叫 `tool.launch` 時沒帶身分資訊，後端因此生出一個跟 test-agent 完全無關的全新匿名 session（`agent-94`）——這不是「重啟同一個 Agent」失敗，而是這次觸發環境本來就不符合「重啟」的前提條件。

## 4. 決策：移除全域 hook

使用者已手動刪除 `C:\Users\User\.claude\hooks\session-boundary-trigger.js` 這個檔案，理由：這一整套依賴「個人全域 hook + 沒有確認關卡的自動觸發 + 依賴環境變數猜身分 + bridge 路徑不更新畫面」的機制，體驗和可靠性都不夠支撐正式使用，真正要的是 **GT Office 專案內建**的「自動判斷任務邊界並開新 session」功能，而不是外部腳本東拼西湊借用後端 API。

### 4.1 移除後的殘留狀態（已清乾淨）

- ✅ `C:\Users\User\.claude\hooks\session-boundary-trigger.js` — 已刪除。
- ✅ `C:\Users\User\.claude\settings.json` 的 `hooks.PostToolUse`（`matcher: "Write"` → 這支已刪除的腳本）— 已一併移除，只留下 `UserPromptSubmit` 那個 code-review 提醒 hook；已重新驗證 `settings.json` 仍是合法 JSON。
- ✅ `C:\Users\User\.claude\skills\session-boundary-planner\` — 整個 skill 資料夾已刪除，避免 agent 之後還照著舊指示寫 `.claude/session-handoff/*-handoff.md` 卻沒人處理。

## 5. 後續方向（尚未設計，僅記錄待決問題）

若要在 GT Office 專案內建這個功能，至少要回答：

1. **要不要保留「Agent 自行判斷、無人工確認」這個高自動化模式？** 原本全域 skill 的定位是「使用者刻意接受的個人機器權衡」，套用到專案正式功能上是否合適，還是應該加一道確認關卡（例如比照 `docs/17_...` 已有的「AI 配置變更需 preview → validate → confirm → apply → audit」精神）。
2. **新 session 的身分延續規則要怎麼定義**，才不會像這次一樣，因觸發來源環境不同就意外開出無關聯的匿名 Agent；理想上應該由後端根據「當前是哪個 Agent/Station 在觸發」明確判斷，而不是依賴呼叫端有沒有轉傳環境變數。
3. **開新 session 必須連動 UI 即時顯示**，不能重蹈 `bridge_tool_launch` 目前「不綁定視窗、不 emit 更新事件」的已知落差——這代表專案內建版本很可能不能直接沿用現有的 `bridge_tool_launch`/`tool.launch` 路徑，而要走一條會正確觸發 `workspace/updated` 事件、讓 Agent Canvas 即時畫出新卡片的路徑。
4. **交接內容的資料結構**要不要沿用 SKILL.md 現有的 Handoff Requirement 欄位（completed work / decisions / modified files / unresolved issues / test results / next session objective / constraints），或設計成結構化資料而非自由格式 Markdown，方便下一個 session 可靠解析。

這幾點建議之後另開一份設計文件處理，不在本次排查範圍內。
