# Agent Canvas 與終端狀態不同步排查

> 修正（2026-10-08）：Canvas 已改用 workspace/session-scoped 終端輸出與生命週期投影，含隱藏終端 metadata，已移除五分鐘 channel 判斷。十秒內有輸出為 Active，安靜 online session 為 Idle；退出後 Offline。前端訂閱事件並保留八秒輪詢。靜默思考／等待輸入尚無可靠 provider 訊號，未宣稱完整 Runtime Snapshot 已完成。以下為修正前的歷史排查，現行契約見 [API](../API_CONTRACTS.md)。

> 記錄日期：2026-08-26  
> 本輪範圍：只做靜態、唯讀診斷並記錄問題；未重啟、停止、寫入或派發任何正在運行的 Agent／terminal，也未修改產品程式碼。

## 1. 現象

GT Office 的終端中 Agent 正在執行工作時，Agent Canvas 節點不一定顯示為 `active`，可能仍顯示 `idle`（或在 runtime 註冊尚未建立時顯示 `offline`）。Canvas 的顯示與使用者在終端看到的實際執行狀態因此不一致。

## 2. 結論

根因不是單純的 8 秒輪詢延遲，而是 **Canvas 與終端採用不同的狀態來源與狀態語意**：

- 終端狀態由 terminal lifecycle event 維護，能得到 `running`、`exited`、`killed`、`failed` 等 PTY session 狀態。
- Canvas 沒有讀取上述 terminal lifecycle state。它呼叫 `agent_canvas_runtime_status`，後端再從 `TaskService` 的 runtime registration 與 channel message 推導狀態。
- 對已註冊的 runtime，後端只在該 Agent 最近 5 分鐘有 channel message 時回傳 `active`；否則固定回傳 `idle`。也就是說，「終端內模型正在思考／執行命令／輸出內容」本身不會使 Canvas 變成 `active`。
- Canvas 端雖每 8 秒重抓一次資料，但輪詢只會持續取得同一套 channel-message-based projection，無法補足 terminal activity 訊號。

因此目前 Canvas 的 `active` 實際含義較接近「最近有 GT Office channel 活動」，而不是「Agent 的終端目前正在工作」。名稱與 UI 呈現容易讓使用者誤認為它代表 terminal execution state。

## 3. 程式碼證據

### 3.1 Canvas 的資料更新方式

`apps/desktop-web/src/features/agent-canvas/controllers/useAgentCanvasData.ts`

- `POLL_INTERVAL_MS = 8000`。
- `reload()` 同時呼叫 `agentList`、`agentCanvasListLinks`、`agentCanvasRuntimeStatus`。
- 只有 Canvas pane 為 `active` 時才啟動輪詢。
- 捕捉錯誤後保留舊 graph，沒有將錯誤暴露為可觀測狀態；短暫失敗也可能讓舊狀態繼續顯示，但這不是本次語意不一致的主要根因。

### 3.2 Canvas 後端的狀態來源

`apps/desktop-tauri/src-tauri/src/commands/agent_canvas.rs`

- `agent_canvas_runtime_status` 讀取 `state.task_service.agent_runtime_status(workspace_id)`。
- roster 中沒有 runtime registration 的 Agent 會被補成 `Offline`。
- 此 command 沒有查詢 `TerminalService` 的 session state，也沒有合併 terminal lifecycle event。

`crates/gt-task/src/lib.rs`

- `agent_runtime_status()` 設定 `ACTIVE_WINDOW_MS = 5 * 60 * 1000`。
- runtime 只要仍在 `runtimes` map 就視為 online。
- 最近 5 分鐘的 `channel_messages` 若 sender 或 target 是該 Agent，狀態為 `Active`；否則為 `Idle`。
- 沒有檢查 PTY 是否持續輸出、CLI Agent 是否正在推理、是否等待輸入，也沒有讀取 terminal 的 `stateRaw`。

### 3.3 終端其實已有另一套生命週期狀態

`apps/desktop-web/src/shell/layout/useShellTerminalController.ts`

- `onStateChanged` 接收 terminal state event 並更新 station terminal 的 `stateRaw`。
- session 進入 `exited`、`killed`、`failed` 時會解除 Agent runtime registration。
- runtime registration sync 主要依據是 station 是否綁定 `sessionId`；存在 session 時註冊為 `online: true`。

這只能同步「有無 live session／online」，不能區分 live session 裡的 Agent 是正在工作、等待使用者輸入，或只是停在 prompt。因此 Canvas 可大致跟上 online/offline，但無法跟上 active/idle。

## 4. 影響範圍

- 正在終端執行、但未透過 channel message 互動的 Agent：Canvas 會顯示 `idle`。
- 曾有 channel message 的 Agent：即使終端已回到等待狀態，Canvas 仍可能在 5 分鐘窗口內顯示 `active`。
- Canvas 不在目前導覽頁時不輪詢；切回時會立即 reload，因此背景期間的畫面資料不更新，但切回後最多主要受一次 command 時間影響。這是次要的即時性限制。
- runtime register/unregister 呼叫採 best-effort 且部分錯誤被忽略；若解除註冊失敗，可能短暫留下 stale online 狀態。這是另一個可能放大不同步的次要因素。

## 5. 建議後續修正方向（尚未實作）

應先定義統一、與畫面無關的 Agent Runtime Snapshot，再讓 Station 與 Canvas 共用，而不是讓 Canvas 直接依賴 channel message：

1. 後端以 `workspace_id + agent_id + session_id` 關聯 terminal session 與 Agent runtime registration。
2. lifecycle 至少分開表達：
   - presence：`offline` / `online`
   - execution：`idle` / `active` / `waiting` / `failed`（實際枚舉需對齊 `docs/AGENT_RUNTIME_UPGRADE_PRD.md`）
   - attention：是否需要使用者介入
3. terminal state/output/activity 或 provider session signal 更新統一 snapshot；channel message 只能作為 activity evidence 之一，不能是 `active` 的唯一判定依據。
4. Canvas 訂閱 runtime snapshot 事件，並保留低頻輪詢作為重連／漏事件補償。
5. UI 顯示 snapshot 的明確欄位，避免把 online、terminal running、Agent working 三種概念壓成一個狀態。
6. 補測試覆蓋：terminal 正在輸出但沒有 channel message、Agent 回到 prompt、等待確認、session 結束、事件漏失後輪詢修復、workspace 切換。

## 6. 修改前的最小驗證計畫

後續動工前，先以不影響既有 Agent 的測試／mock 重現下列狀態矩陣：

| Terminal session | Channel activity | 目前 Canvas | 期望 Canvas |
| --- | --- | --- | --- |
| running 且 Agent 執行中 | 無 | idle | active |
| running 但停在 prompt | 5 分鐘內有 | active | idle 或 waiting |
| exited/killed/failed | 任意 | 依 unregister 是否成功 | offline/failed（依契約） |

正式修改時應以 `docs/AGENT_RUNTIME_UPGRADE_PRD.md` 與 `docs/cw/05_PRD對齊調研.md` 為契約基礎，避免再增加第三套狀態模型。
