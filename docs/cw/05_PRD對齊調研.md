# 05 PRD 對齊調研（P0 產出）

> 這是 [04_客製化設計.md](04_客製化設計.md) 第 9 節決策 4 要求的「先花一輪對照 `AGENT_RUNTIME_UPGRADE_PRD.md`」的具體落地。範圍：只回答「P0-P3 這次要做的東西，有沒有跟 PRD 的方向衝突」，不落地 PRD 的完整 Agent Runtime Contract——那是 P4（agent-canvas，本次不做）的範圍。

## 結論

**本次 P0-P3 的 schema 變動不與 PRD 衝突，但有一處要留意，記錄在下面，供 P4 動工前參考。**

## 對照細節

`docs/AGENT_RUNTIME_UPGRADE_PRD.md` 定義了一套 Lifecycle State（`idle / launching / working / waiting / blocked / completed / failed / recovering / stopped / unknown`，PRD 第 69-83 行）和 Status Authority 優先序（Provider Hook > 結構化事件/人工確認 > 原生會話恢復/進程觀察 > 屏幕快照 > Unknown，PRD 第 86-94 行），核心訴求是把目前分散在 Station 卡片狀態、終端進程探測、Provider session 的運行時資訊統一成一份 `Runtime Snapshot`。

現有 `crates/gt-agent/src/models.rs` 的 `AgentState`（`Ready / Paused / Blocked / Terminated`）是一個粗顆粒度的「Agent 記錄本身的狀態」欄位，語意上跟 PRD 的 Lifecycle State 不是同一層概念——`AgentState` 描述的是這筆 Agent 設定記錄的生命週期（是否啟用/暫停/刪除），PRD 的 Lifecycle State 描述的是「這個 Agent 現在實際在做什麼」，是即時、會頻繁變動的運行時投影，不該存進 `agents` 表的一個欄位。兩者不衝突，但容易混淆，未來 P4 若要接 PRD 的 Runtime Snapshot，**不應該**把 Lifecycle State 塞進 `AgentState` 或試圖讓兩者合併成一個欄位。

## P0-P3 範圍內的檢查結果

- **`agent_policy_snapshots` 表**（P3 用）：純粹是「這個 Agent 在某個時間點的權限設定快照」，跟 PRD 的 Runtime Observation/Snapshot 概念不重疊，不衝突。
- **`agent_links` 表**（P0 建表，P4 才接 UI）：`kind` 欄位目前只有 `authored`/`derived` 兩種語意（04 文件第 1 節），跟 PRD 無關，不衝突。
- **`AgentState` 沒有新增任何值**：P0-P3 沒有在這個 enum 裡加新狀態（例如沒有加 `blocked`/`working` 這種跟 PRD Lifecycle State 撞名的值），維持現況的 4 個值不變。這是刻意的——如果現在就加一個語意含糊的新狀態，之後 P4 對齊 PRD 時会需要拆解「這個狀態到底是 AgentState 的意思還是 Lifecycle State 的意思」，現在不加就不會製造這個歧義。
- **Human Approval（04 文件第 3 節 Phase B）**：文件已經指出這應該建在 PRD 的 `blocked` Lifecycle State 之上，而不是另開一套獨立中斷機制。本次 P3 Phase A 不含 Human Approval（它在 Phase B），所以這次不需要現在就決定怎麼接，只需要確認 Phase A 五類（File System/Shell/Git/Agent/Execution）都不會跟這個未來決定衝突——確認結果：Phase A 是同步的「允許/拒絕」布林開關，不涉及非同步的「等待人工核准」流程，跟 PRD 的 `blocked` 狀態機制在架構上是分離的兩層，不衝突。

## 給 P4（本次不做）的路標

P4 動工前，agent-canvas 節點狀態資料層要嘛直接借用 PRD 的 `Runtime Snapshot`/`Lifecycle State` 雛形，要嘛做成不綁定畫面的通用資料層（04 文件第 1 節、第 6.2 節已有此要求）。本次調研額外補充一點：**這份 Runtime Snapshot 資料層應該是獨立於 `agents` 表之外的一張新表或記憶體結構**（例如 `agent_runtime_snapshots`，非持久化或短期持久化皆可），不要嘗試往 `agents` 表加更多欄位去表示即時狀態——`agents` 表現在已經承載了「設定 + 記錄生命週期」的職責（`state`、`policy_snapshot_id`、`parent_agent_id`），再疊加高頻變動的運行時狀態會讓這張表的寫入頻率和職責邊界都模糊掉。
