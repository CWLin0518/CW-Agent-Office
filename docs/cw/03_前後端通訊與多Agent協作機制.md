# 03 前後端通訊與多 Agent 協作機制

> 這一章把「前端怎麼跟後端說話」和「Agent 之間怎麼互相協作」放在一起講，因為多 Agent 協作機制本質上就是這套通訊機制的延伸應用。這也是你之後要重新設計「操作/協作機制」時最核心要看懂的部分。

## 1. 前端 ↔ 後端：兩種溝通方式

### 方式一：invoke（前端主動問，後端回答一次）

前端呼叫後端的函式，等後端做完，回傳結果。類似「打電話問一件事，掛電話前就有答案」。

```typescript
// 前端
const result = await invoke("git_status", { workspaceId });
```

```rust
// 後端，對應 commands/git/ 底下某個檔案
#[tauri::command]
pub async fn git_status(workspace_id: String) -> ResultEnvelope<GitStatusPayload> {
    // 驗證輸入 → 呼叫 gt-git crate → 包成 ResultEnvelope 回傳
}
```

所有 command 都回傳統一格式 `ResultEnvelope<T>`（定義在 `packages/shared-types`）：

```typescript
interface ResultEnvelope<T = unknown> {
  ok: boolean;                 // 這次呼叫成不成功
  data: T | null;               // 成功時的資料
  error: { code: string; message: string; details?: unknown } | null;  // 失敗時的錯誤細節
  traceId: string;               // 追蹤 ID，前後端 log 可以用同一個 ID 對上
}
```

好處：前端不用寫一堆 `try/catch`，看 `ok` 欄位就能判斷成功或失敗，`traceId` 讓你出問題時能在前後端 log 裡追同一條請求。

### 方式二：event（後端主動推，前端被動收）

有些事後端不知道前端什麼時候想聽，例如終端機新輸出、Git 狀態變化——這種用 event，後端做完就主動推，前端訂閱著收。

```rust
// 後端
app_handle.emit("git/updated", payload);
```

```typescript
// 前端
listen("git/updated", (event) => { /* 更新畫面 */ });
```

常見的 event 用途：終端機輸出串流（PTY → xterm.js）、change feed 通知（檔案/Git 變化）、任務派發進度、外部 Channel 訊息。

### workspace_id 是硬邊界

所有跟 workspace 有關的 command，都**必須**帶 `workspace_id`。後端在真正做事之前，會先解析這個 workspace 的 context（根路徑、權限、預設終端機 cwd），確認操作沒有跑出這個 workspace 的邊界。終端機的自訂 cwd 也一定要驗證在 workspace 內，不能任意指定路徑。這是安全邊界，不是可有可無的參數。

## 2. Agent 之間怎麼協作：`gto` 協議

這是目前專案裡「多 Agent 同時工作」實際運作的機制，也是你未來要改「操作機制」時最直接相關的部分。

### 核心概念

每個 Station 卡片跑一個 Agent（一個真的 Claude Code 或 Codex CLI 進程，跑在一個 PTY 終端機裡）。多個 Station 共用同一個 workspace context。Agent 之間不是直接互相呼叫，而是透過一個本地的「郵局」——`gto` CLI + local bridge——來互相傳訊息、派任務、回報進度。

```
Agent A（Station 1，例如 Manager 角色）
    │  執行 `gto send A B "幫我做 X"`
    ▼
gto CLI ──讀 token── ~/.gtoffice/mcp/runtime.json
    │  帶 token 呼叫
    ▼
local bridge（跑在 Tauri 主進程裡，127.0.0.1 + 隨機 port）
    │  建立任務紀錄，寫進 Agent B 的 inbox
    ▼
Agent B（Station 2）
    │  用 `gto inbox B` 讀到任務，或被 Tauri event 通知
    │  做完後執行 `gto agent reply-status` / `gto agent handover`
    ▼
訊息透過 local bridge 傳回，Agent A 或 UI 能看到回覆
```

### `gto` 常用指令

| 指令 | 用途 |
|---|---|
| `gto agents` | 查看目前 workspace 有哪些 Agent |
| `gto directory snapshot` | 取得整個 workspace 的 Agent 目錄快照 |
| `gto send <from> <to> <text>` | 把任務從一個 Agent 派給另一個 Agent |
| `gto send ... --wait` / `gto wait <taskId> --from <agent>` | 同步派任務，等對方回覆 |
| `gto agent reply-status` | 回報簡短進度 |
| `gto agent handover` | 回報完成、交接下一步（含 blocker） |
| `gto inbox <agent>` | 看某個 Agent 有哪些待處理任務 |
| `gto thread <taskId>` | 看某個任務的完整訊息歷史 |

### local bridge 是什麼

`local_bridge.rs`（在 Tauri 主進程裡）起一個只聽 `127.0.0.1` 的 server，port 隨機分配，用寫在本地檔案的 token 做認證。它對外提供：

- **Agent endpoints** — Agent 身分與角色的 CRUD
- **Task endpoints** — 派發、回報進度、交接、查看討論串
- **Channel endpoints** — 傳訊息、列訊息（給外部 Telegram/Feishu/WeChat 用）
- **Directory endpoints** — workspace 內 Agent 的快照查詢

`tools/gto-agent-mcp-sidecar` 另外提供 MCP 協議支援（給支援 MCP 的 Agent 用），但**目前主要協作路徑還是 `gto` CLI**，不是 MCP。

### 對應到後端 crate

`gto` 協議背後對應的領域邏輯主要在 `crates/gt-task`（任務派發、進度事件、Agent runtime 註冊）和 `crates/gt-agent`（Agent 身分與角色）。前端 `features/task-center` 提供任務追蹤/派工的 UI。

## 3. Agent 輸出怎麼被觀察：Display Channel vs Data Channel

一個 Agent 進程的輸出，同時走兩條路：

| Channel | 內容 | 用途 |
|---|---|---|
| Display Channel | PTY 原始輸出 → VT100 parsing → xterm.js | 給人看的終端機畫面 |
| Data Channel | 結構化 JSON/key-value 輸出 → `channel_sinks` → 外部服務 | 給機器/外部系統看的資料 |

外部 Channel（Telegram / Feishu / WeChat）的訊息走 inbound（`channel_adapter_runtime.rs` 監聽 webhook）和 outbound（`channel_sinks.rs` 發送）兩條路，對應到前端的 `tool-adapter` feature。

長期規劃是往 MCP（Model Context Protocol）方向走，讓 Agent 直接說結構化協議、不用再解析終端機文字——但這是長期方向，目前主力還是 PTY + VT parsing。

## 4. 現狀 vs 尚未實作的規劃

**已經在跑的**：Station 多開、`gto` 任務派發/回報、local bridge、Channel 轉發。

**還在提案階段、尚未落地**（記錄在 `docs/AGENT_RUNTIME_UPGRADE_PRD.md`）：

- 統一的 Agent Runtime Contract（把 Station 狀態、進程偵測、Provider session、螢幕快照、Channel 事件等分散的狀態來源，歸併成一個可信的 snapshot）
- Attention Projection（工作台上該優先看哪個 Station 的提示機制）
- gto 自動化控制面（讓本地自動化流程，而不只是人或 Agent 手動，觸發任務派發）
- 任務隔離用的 Git worktree（不同 Agent 平行做任務時，不會互相覆蓋同一個工作目錄）
- 受控擴展機制

這份 PRD 目前狀態是「提案」，還沒有對應的程式碼落地。如果你想做的「操作/協作機制」跟這幾點方向接近，值得先讀一遍這份 PRD，避免重新發明已經想過的設計。

## 5. 給你的具體建議

如果要改「操作/協作機制」，建議的閱讀順序：

1. `docs/WORKFLOWS.md` 的 `Agent Collaboration via gto` 一節 — 先搞懂現在使用者/Agent 實際怎麼操作
2. `crates/gt-task/` — 任務模型的程式碼
3. `apps/desktop-tauri/src-tauri/src/local_bridge.rs` — bridge server 怎麼收發請求
4. `tools/gto/` — CLI 本身怎麼跟 bridge 溝通
5. `docs/AGENT_RUNTIME_UPGRADE_PRD.md` — 如果想做更大幅度的協作機制升級，先看這份避免重工
