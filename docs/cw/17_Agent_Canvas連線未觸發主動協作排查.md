# Agent Canvas 連線未觸發主動協作排查

> 日期：2026-08-26  
> 狀態：已完成程式碼層調研，尚未實作  
> 現象：使用者在 Agent Canvas 上連接兩個 Agent 後，若雙方的 system prompt／`CLAUDE.md`／`AGENTS.md` 沒有描述協作方式，Agent 通常不會主動呼叫已連線的另一個 Agent。

## 1. 結論

目前 Agent Canvas 的 authored edge 只落實了「允許誰傳訊給誰」的授權邊界，沒有被轉換成模型可理解的「協作上下文」。

因此，畫線後雖然 `gto send` 可以通過 edge policy，模型本身仍不知道：

- 哪些 Agent 與自己相連；
- 對方的名稱、角色或適合承接的工作；
- 什麼情況應主動委派；
- 應使用哪個 `agent_id` 與哪組 `gto` 指令；
- 連線在 Agent 啟動後發生變更時，最新拓撲是什麼。

修正方向不是直接改寫使用者維護的 prompt 檔，而是由 GT Office 在啟動或派發任務時，根據當下的 Canvas authored links 動態組裝一段 collaboration context。

## 2. 現況證據

### 2.1 Canvas 連線已有持久化與授權用途

`agent_links` 會保存使用者手動畫出的 authored edge。`apps/desktop-tauri/src-tauri/src/local_bridge.rs` 的 `ensure_agent_allowed_to_dispatch` 會在 Agent 執行 `task.dispatch_batch`／`gto send` 時檢查：

1. sender policy 的 `allow_gto_send` 是否開啟；
2. sender 與每個 target 之間是否存在 authored edge；
3. 沒有 edge 時回傳 `AGENT_POLICY_EDGE_REQUIRED`。

這證明 Canvas edge 現在是有效的安全授權資料，不只是視覺元素。但授權只回答「能不能傳」，不會讓模型知道「何時應該傳」。

### 2.2 Directory snapshot 沒有協作拓撲

`apps/desktop-tauri/src-tauri/src/local_bridge.rs` 的 `build_directory_snapshot` 目前輸出：

- `agentId`
- `name`
- `state`
- `online`
- `sessionId`
- `toolKind`
- `resolvedCwd`

其中沒有 authored links、`connectedAgentIds`、角色摘要或可委派能力。因此即使 Agent 主動執行 `gto directory snapshot`，拿到的也只是 Agent 清單與 runtime 狀態，不足以理解 Canvas 上的協作關係。

### 2.3 Agent 啟動流程沒有注入拓撲

`apps/desktop-tauri/src-tauri/src/commands/tool_adapter/tool_profiles.rs` 啟動 CLI 時會：

1. 從 launch context 取得 `initialPrompt`；
2. 建立 terminal session；
3. 啟動 Claude Code／Codex；
4. 若有 `initialPrompt`，延遲後提交到 CLI。

此流程目前沒有查詢 `agent_links`，也沒有把連線關係附加至 `initialPrompt`。`GTO_WORKSPACE_ID`、`GTO_AGENT_ID` 與 `GTO_STATION_ID` 雖然已放入環境變數，但它們只提供 runtime identity，不包含協作對象或委派規則。

### 2.4 靜態 prompt 不是可靠的拓撲載體

新建 Agent 的 prompt 檔可以包含使用者自訂內容，目前也有輸出目錄的預設引導。但 Canvas edge 是可隨時新增、刪除與調整的 workspace runtime state：

- 使用者可能在 Agent 啟動後才畫線；
- 同一份 prompt 內容不應混入會頻繁變動的 topology；
- 自動覆寫 `CLAUDE.md`／`AGENTS.md` 容易破壞使用者內容；
- Agent 使用外部模板時，更不應由畫布操作隱式修改模板 materialization 結果。

因此協作拓撲應採動態注入，而不是直接持久化到 prompt 檔。

## 3. 建議設計

### 3.1 建立統一的 collaboration context builder

在後端新增一個可測試的純組裝層，例如：

```text
build_collaboration_context(workspace_id, agent_id) -> Option<String>
```

它應讀取：

- 當前 Agent profile；
- 與當前 Agent 相連的 authored links；
- 相鄰 Agent 的 id、名稱、角色摘要與 runtime availability；
- edge 的方向／`bidirectional` 語意；
- 當前 topology revision。

輸出示意：

```text
## GT Office collaboration context

You are agent-a in workspace ws-123.

Connected collaborators:
- agent-b — Research Agent
- agent-c — Revit Agent

When a task contains a separable part matching a connected agent's role,
delegate it proactively with:
gto agent send-task --target-agent-id <agent-id> ...

Do not delegate trivial work. Keep the returned taskId for follow-ups.
Only connected agents are authorized collaboration targets.
```

這段內容應保持短、結構固定且由資料產生，避免把完整 Canvas 或不相關 Agent 全部塞進 context。

### 3.2 注入時機

建議分兩階段實作。

#### P1：Agent 啟動時注入

在 tool launch 組裝 `initialPrompt` 時附加 collaboration context。這可以解決「啟動前已經畫好線，但 Agent 不知道」的主要問題。

組裝順序建議為：

1. 使用者／任務提供的原始 initial prompt；
2. GT Office runtime identity；
3. collaboration context。

不得修改磁碟上的 `CLAUDE.md`／`AGENTS.md`。

#### P2：每次新任務派發時刷新

Agent 可能長時間運行，而 Canvas topology 可能在 session 期間改變。派發新任務時應重新產生 collaboration context，或至少在 topology revision 改變時附加更新。

這比單純在啟動時注入可靠，也避免為了更新拓撲而重啟 Agent。

### 3.3 Directory snapshot 增加拓撲資料

建議讓 `directory.get`／`gto directory snapshot` 回傳與授權層一致的協作資訊，例如每個 Agent 增加：

```json
{
  "agentId": "agent-a",
  "connectedAgents": [
    {
      "agentId": "agent-b",
      "direction": "outbound",
      "bidirectional": false
    }
  ]
}
```

也可採 snapshot 根層級的 `authoredLinks`，避免每個節點重複資料。最終格式應以消費端是否主要查「我的鄰居」或渲染完整拓撲決定，但不得另做一套與 `agent_links` 不一致的授權資料來源。

### 3.4 角色與能力資訊

只有 Agent 名稱仍不足以讓模型判斷該委派給誰。collaboration context 至少應提供短角色摘要；若角色模型已有 description／instructions，應重用現有欄位，不為此另建一份自由文字設定。

能力資訊應採摘要，而非注入完整 MCP、Skill、Hook 配置。例如：

```text
- agent-c — Revit Agent; capabilities: Revit MCP, material inspection
```

詳細能力仍由 Agent 在需要時透過既有 CLI／directory 查詢，避免 prompt 膨脹。

## 4. 執行中 Agent 的安全處理

Canvas edge 變更後，不應直接往正在工作的 terminal 寫入文字。這可能：

- 插入目前尚未送出的使用者輸入；
- 打斷 CLI 正在執行的工作；
- 讓 topology update 被模型誤認成新任務；
- 在 Claude Code／Codex 不同終端狀態下產生不一致結果。

建議策略：

1. edge 變更只更新後端 topology revision；
2. Agent 正在執行時不即時注入；
3. 下一次人類或 Agent 派發新任務時附加最新版 context；
4. 若未來要支援即時刷新，只能在 runtime 明確為 idle 且 terminal input buffer 為空時送出具型別的 context update；
5. UI 可顯示「協作關係將於下一個任務生效」。

這樣不需要停止或重啟正在工作的 Agent。

## 5. 建議實作範圍

### 5.1 後端／domain

- `AgentLinkRepository` 新增列出某 Agent authored 鄰居的查詢，或以既有 `list_links` 在 service 層安全過濾；
- 建立 collaboration context model 與 builder；
- 所有查詢必須顯式攜帶 `workspace_id`；
- 確認單向 edge 與 `bidirectional` 的實際派工語意；
- context builder 不依賴 Tauri command，便於 unit test 與其他 adapter 重用。

### 5.2 Tool launch

- 在 `commands/tool_adapter/tool_profiles.rs` 的 initial prompt 組裝點接入 builder；
- Claude Code 與 Codex 共用相同語意，只在必要時調整命令示例；
- custom launch path 也必須定義是否注入，避免標準 launch 與 custom launch 行為不同。

### 5.3 Task dispatch

- 在新任務被交付給 Agent 前取得最新 topology revision；
- revision 未變時可省略重複的完整內容；
- revision 改變時附加最新協作者清單；
- 不把 topology context 當成使用者 task markdown 持久內容，以免污染任務正文與歷史顯示。

### 5.4 Directory API／CLI

- 擴充 directory snapshot 契約；
- 更新 `tools/gto` 型別、README 與測試；
- 保持舊版 consumer 對新增欄位的相容性。

## 6. 不建議方案

### 6.1 每次畫線就改寫 system prompt 檔

不建議，因為會混合使用者設定與 runtime state，還會導致版本控制噪音、外部模板漂移及併發覆寫風險。

### 6.2 只在 UI 顯示「已連線」

這只能改善人類理解，模型仍看不到拓撲，無法解決主動協作問題。

### 6.3 Canvas 連線後由 GT Office 自動派工

edge 只代表允許或預期協作，不代表任何任務都應被拆分。若後端看到線就自動派工，容易產生重複工作、循環委派與不必要成本。是否委派應由收到任務的 Agent 根據明确的協作指令與角色摘要判断。

### 6.4 只提供全 workspace Agent 清單

列出所有 Agent 會模糊 authored edge 的授權語意。模型應優先看到与自己相連且可派工的鄰居，而不是自行嘗試未授權 target。

## 7. 驗收標準

### 7.1 基本行為

- A 與 B 有 authored edge，啟動 A 後，A 的動態上下文能看到 B 的 id、名稱與角色摘要；
- A 與 C 沒有 authored edge，A 的動態上下文不應把 C 列為可協作對象；
- 任務包含適合 B 的獨立子工作時，A 能主動使用正確的 `gto agent send-task` 指令；
- 簡單、不可分割的任務不應為了有連線就強制委派；
- 派工後保留 `taskId`，後續狀態與 handover 使用同一 task thread。

### 7.2 動態拓撲

- A 啟動後新增 A—B edge，不重啟 A；
- 不向執行中的 terminal 即時寫入文字；
- 下一次派發給 A 的任務包含最新版 collaboration context；
- 刪除 edge 後，下一個任務不再把 B 列為可派工對象；
- topology revision 沒變時，不重複注入不必要的長內容。

### 7.3 安全與隔離

- 不得列出其他 workspace 的 Agent 或 edge；
- `allow_gto_send=false` 仍優先拒絕派工；
- 模型看到的 connected targets 必須與授權檢查使用同一份 authored edge 資料；
- context 不包含 secrets、完整 capability 設定或不必要的本機路徑；
- 不修改使用者的 `CLAUDE.md`／`AGENTS.md`。

### 7.4 最小驗證

- repository 鄰居查詢 unit tests；
- collaboration context builder unit tests；
- workspace isolation tests；
- direction／bidirectional tests；
- tool launch initial prompt 組裝測試；
- directory snapshot 與 `tools/gto` contract tests；
- `npm run typecheck`；
- `cargo check --workspace`；
- 真人驗收一次「啟動前畫線」與一次「啟動後畫線、下一任務生效」。

## 8. 建議開發順序

1. 先定義 authored edge 的方向是否影響派工；
2. 新增 repository 查詢與 collaboration context builder；
3. 接入標準 tool launch；
4. 擴充 directory snapshot 與 CLI 契約；
5. 接入 task-time topology refresh；
6. 最後補 UI 的生效時機提示；
7. 完成自動測試後再進行真人多 Agent 驗收。

第一個可交付的最小閉環是「啟動前已有 authored edge時，Agent 啟動 prompt 會得到正確且 workspace-scoped 的協作者清單」。完成這一段即可直接驗證本次問題的核心假設，再繼續做執行中拓撲刷新。
