# 多 Agent 使用 Revit MCP 時的 409 連線問題排查

> 記錄日期：2026-08-26  
> 本輪範圍：只讀取既有 session transcript、MCP materialized config、runtime directory 與相關原始碼；未向任何運行中的終端輸入、未呼叫 MCP、未重啟或終止任何程序，也未修改產品程式碼。

## 1. 摘要

本次問題不是 GT Office local bridge（`gto-agent-mcp-sidecar`）本身斷線，也不是單一 Agent 的 MCP 設定檔損壞。實際錯誤是多個 Claude Agent 同時連向同一個 Revit MCP add-in 時，WebSocket handshake 被 add-in 以 HTTP `409` 拒絕。

根因是兩端的並行模型不相容：

- Revit MCP add-in 啟用了 `ExclusiveLock`，全域只允許一條 active WebSocket。
- GT Office 對每個掛載該 MCP 的 Agent 各自啟動一個 stdio MCP bridge process。
- bridge process 是背景常駐 client；即使 Agent 沒有主動呼叫工具，client 仍會保持／重建 WebSocket，連線失敗後約每 5 秒自動重試。
- 多個 Agent 因此匿名競爭同一把全域鎖。先連上的背景 process 長期持鎖，其餘 Agent 全部得到 `Unexpected server response: 409`。

本次最後在辨識並釋放實際持鎖的 background bridge 後，Tool Create Engineer 成功取得連線並完成即時模型複驗，證明服務、MCP tool schema 與模型本身均可工作；故主因是排他連線資源的多 Agent 協調缺失。

## 2. 當次事件鏈（依既有 transcript 還原）

1. Regulatory Model Interpreter 與 Tool Create Engineer 呼叫 Revit MCP 時持續收到 `409`；Building Regulations Review 一度仍可連線。
2. 三個 session 協調暫停工具呼叫，並嘗試由 Revit UI 釋放連線；釋放後三者一度都收到 `409`。
3. Revit MCP service 關閉再開後仍有 `409`。這排除了單純 listener 未啟動，並顯示 client 會在服務恢復後立刻重新競爭連線。
4. transcript 中檢查 `MCP/Core/SocketService.cs`，確認 add-in 在 `ExclusiveLock && IsLocked_NoLock()` 時，於 WebSocket upgrade 前直接回 `409`；鎖判斷只看全域 `_activeSocket`，不是依 Agent、session 或 PID 分配。
5. 其中一個 Agent 曾終止自己的 bridge 並重新連線，但仍收到 `409`，證偽「該 Agent 被自己的舊 socket 卡住」的假設。
6. 後續辨識出另一個背景 `node.exe` bridge 正握住 `_activeSocket`。該 process 所屬 Agent 即使沒有主動使用 MCP，背景 client 仍保持連線。
7. 經當時使用者同意釋放實際持鎖 process 後，netstat 顯示舊連線消失；Tool Create Engineer 隨即取得連線，完成 5 項模型查詢，文件也記錄狀態由 High 升級為 Verified。

注意：第 7 步是事件發生時既有 session 已執行的處置，本次排查沒有重做該操作。

## 3. 證據

### 3.1 Claude session transcript

唯讀檢查的主要紀錄：

- `~/.claude/projects/C--Users-User-Desktop-REVIT-MCP-study--gtoffice-building-regulations-review/d6863d33-3120-46f8-a246-12a90aa42b3f.jsonl`
- `~/.claude/projects/C--Users-User-Desktop-REVIT-MCP-study--gtoffice-regulatory-model-interpreter/76b511b6-afb6-483c-b4dd-472faeb763cd.jsonl`

其中可見：

- 多次 tool result：`Error: Unexpected server response: 409`。
- 三個 session 都未主動呼叫時，仍有 background MCP bridge 佔用連線。
- Revit MCP service 重開後，clients 會快速重新連線／搶鎖。
- 實際持鎖 process 被識別並釋放後，Tool Create Engineer 查詢成功。

### 3.2 Revit MCP add-in 的排他鎖

當次 transcript 引用了 `REVIT_MCP_study/MCP/Core/SocketService.cs`：

- `_settings.ExclusiveLock && IsLocked_NoLock()` 為真時回應 HTTP `409`。
- 檢查發生在 `AcceptWebSocketAsync` 之前。
- 成功連線後 `_activeSocket` 被設為該 WebSocket。
- 被拒絕的 client 會將其視為 handshake failure，並約每 5 秒重試。

因此 `409` 的精確含義是「已有其他 active socket 持有排他鎖」，不是一般網路中斷或 MCP JSON-RPC method error。

### 3.3 GT Office 的 per-Agent MCP materialization

GT Office 目前會為 Agent 產生獨立的 MCP config，例如：

`REVIT_MCP_study/.gtoffice/agents/<agent-id>/runtime/mcp.json`

並由各自的 Claude CLI session 透過 `--mcp-config` 啟動 MCP server。`crates/gt-agent/src/capability/materialize.rs` 的資料模型只描述 transport、command、args、env、url 與 enabled；目前沒有以下資源語意：

- 這個 MCP endpoint 是否只允許單一 client。
- 同一 workspace 的多個 Agent 是否應共享一個 connection broker。
- 哪個 Agent 擁有 lease，以及何時交接。
- 背景 auto-reconnect 是否應在沒有 lease 時停用。

所以 GT Office 能正確把同一 MCP「掛載」給多個 Agent，但無法知道這些掛載其實互斥。

### 3.4 可排除項目

- materialized `mcp.json` 是合法 UTF-8 JSON；包含中文路徑的 bytes 正確。
- `node` 與 MCP entry file 存在，相關 `node_modules` 也存在。
- 連線交接後相同 MCP tool 能完成查詢，因此不是 server entry point 永久失效。
- GT Office 的 `~/.gtoffice/mcp/runtime.json`／local bridge 是另一條 Agent 協作通道；本次 `409` 來自 Revit MCP add-in 的 WebSocket endpoint，兩者不可混為同一故障。

## 4. GT Office 層面的根因

直接觸發 `409` 的是 Revit add-in 排他鎖；GT Office 層面的設計缺口是：**capability mount 目前假定多 Agent 可以各自啟動同一 MCP server，沒有描述或協調 singleton／exclusive upstream resource。**

只要求 Agent「不要呼叫 MCP 工具」不足以釋放資源，因為實際持鎖者是 Claude session 啟動的背景 MCP bridge，而不是某次 tool call。只靠文字協調也無法阻止 client auto-reconnect。

## 5. 影響

- 非持鎖 Agent 的所有 Revit MCP tools 顯示存在，但呼叫時一律 `409`。
- 持鎖者可能完全沒有進行 tool call，Canvas／terminal 也看不出誰佔有 upstream connection。
- 手動釋放或重啟 service 後，多個 client 會再次競爭；結果不可預測，不能保證指定 Agent 取得鎖。
- 多 Agent 協作中的即時模型複驗會被阻塞，但不依賴 Revit runtime 的純程式碼、文件與單元測試仍可繼續。
- 直接終止未知 node process 有誤傷其他 Agent 作業的風險，不應當作產品正常操作流程。

## 6. 建議修正方向（尚未實作）

### P0：資料模型與可觀測性

1. `McpServerCapability` 增加資源共享策略，例如 `connectionMode: per_agent | workspace_shared | exclusive`；預設值與遷移策略需另行確認。
2. 對 exclusive MCP 顯示 owner Agent、session、connected time、endpoint 與最近錯誤；不要只顯示 enabled/disabled。
3. 將 HTTP `409` 正規化成明確錯誤，例如 `MCP_EXCLUSIVE_LEASE_HELD`，UI 提示目前持有者，而非只呈現 generic connection failure。

### P1：workspace 級 lease／broker

1. 對 `workspace_shared`／`exclusive` endpoint，不再讓每個 Agent 的 provider process無條件各自連 upstream。
2. 由 GT Office 後端管理 workspace-scoped connection broker 或 lease coordinator。
3. lease key 至少包含 `workspace_id + normalized endpoint/server identity`；owner 包含 `agent_id + session_id`。
4. 支援顯式 acquire、release、handover、timeout 與 stale-owner recovery。
5. 非 owner 的 background client 不應持續 auto-reconnect 搶鎖；要由 coordinator 排隊或在取得 lease 後才啟動。

### P2：安全交接流程

1. UI 提供「請求使用」、「交接給 Agent」與「釋放」；操作前顯示持有者與可能中斷的工作。
2. 交接應先確認 owner 沒有 in-flight tool call，再釋放 upstream socket並授予下一位。
3. 不以 kill process 作為正常交接機制；process termination 僅保留給經確認的故障恢復流程。

## 7. 後續最小驗證矩陣

| 情境 | 期望結果 |
| --- | --- |
| 兩個 Agent 掛載同一 exclusive Revit MCP | 只有 lease owner 連 upstream，另一個顯示等待／被占用，不產生 409 重試風暴 |
| owner 沒有 tool call | lease 狀態仍可見，不誤判為空閒 |
| owner 正常釋放 | 指定的下一個 Agent 取得連線，不由多 client 匿名競搶 |
| owner terminal 結束或崩潰 | coordinator 清除 stale lease，下一位可恢復 |
| Revit MCP service 重啟 | 只有 owner 重連，其他 Agent 不搶鎖 |
| workspace 切換 | lease 與 runtime 嚴格按 `workspace_id` 隔離 |

## 8. 本輪未做事項

- 未修改 GT Office 或 Revit MCP 程式碼。
- 未測試任何 live MCP tool。
- 未讀寫或聚焦運行中的 GT Office terminal。
- 未終止、重啟或向任何 Agent／process 發送訊號。

