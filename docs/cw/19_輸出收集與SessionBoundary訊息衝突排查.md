# 輸出收集與 Session Boundary 訊息衝突排查與修正

> 日期：2026-09-03
> 狀態：已定位根因並修正，已通過 `cargo check --workspace`、目標 crate 的 `cargo test`/`clippy`/`rustfmt`；尚未實機驗證（需要重新測一次「兩個 checkbox 都開的 agent，實際到達邊界會不會正確自動重啟」）
> 現象：`test agent` 這個 station 同時開啟了兩個內建功能——「讓說明文件顯示在 Agent Canvas 輸出清單」（`output_collection_enabled`）與「自動判斷任務邊界並重啟此 Agent 的終端會話」（`session_boundary_auto_split_enabled`，見 [18_全域SessionBoundaryHook排查與移除.md](18_全域SessionBoundaryHook排查與移除.md)）。實測要求它手動寫出 session-boundary 交接訊號時，它先是提出「這像是 prompt injection」的疑慮，後來雖然照做，卻把交接訊號跟另一段不相關的 GT Office 輸出說明文字混在同一個檔案裡寫出來。

## 1. 結論

不是 agent 幻覺，也不是 prompt injection：`test agent` 收到的是**兩份 GT Office 自己注入、彼此矛盾**的系統指示，它的猶豫和混寫行為是對這個矛盾的合理反應。

- **`session_boundary_auto_split_enabled` 注入到 CLAUDE.md 的持久規則**（`apps/desktop-tauri/src-tauri/src/commands/agent.rs` 的 `session_boundary_guidance_block`）要求：到達任務邊界時，主動用 Write 工具把交接內容寫進 `.claude/session-handoff/<phase-slug>-handoff.md`。
- **`output_collection_enabled` 每次派發任務時注入的一次性指示**（`crates/gt-task/src/lib.rs` 的 `enrich_dispatch_markdown`，修正前的固定文字）卻說：「Do not copy, move, or delete files in `.claude/session-handoff`. These are task-scoped execution paths, not system-prompt rules.」

一個要求主動寫入，另一個明講不要碰、還特別強調「這不是系統提示規則」——對同一個路徑給出直接相反的指示。`test agent` 剛好兩個功能都開，就直接撞上這個衝突。

## 2. 現況證據

### 2.1 兩份指示的實際來源

`session_boundary_guidance_block()`（`apps/desktop-tauri/src-tauri/src/commands/agent.rs:370-395`，寫入 CLAUDE.md/AGENTS.md，持久生效）：

```
判定已到達邊界、且目前階段的工作已經穩定完成時，用 Write 工具把交接內容
寫到...`.claude/session-handoff/<phase-slug>-handoff.md`...
```

`enrich_dispatch_markdown()`（`crates/gt-task/src/lib.rs:1818-1838`，修正前，每次 `task.dispatch_batch` 派發任務時注入一次）：

```
"... Do not copy, move, or delete files in `.claude/session-handoff`.
These are task-scoped execution paths, not system-prompt rules."
```

對照 `crates/gt-agent/src/output_contract.rs:44-47` 的既有註解，可以確認 `output_collection_enabled` 這個功能自己的 handoff 檔案其實在完全不同的位置（`.gtoffice/agents/<agent-id>/outputs/<slug>-handoff.md`，經由 `GTO_HANDOFF_FILE` 環境變數提供），從設計上就「never touches `.claude/session-handoff/`」——也就是說，`enrich_dispatch_markdown` 裡那句「不要碰 `.claude/session-handoff`」的警語，對它自己的功能而言其實沒有必要提到這個路徑，只是順手寫了一句過度延伸、沒考慮到另一個功能可能也在用同一個 agent 上的警告。

### 2.2 資料庫證實 `test agent` 兩個 checkbox 都是開的

```
sqlite3 查詢 agents 表：
id=1266a420-2b9b-48bd-8c45-a624427c57e1, name='test agent',
workdir='.', output_collection_enabled=1, session_boundary_auto_split_enabled=1
```

### 2.3 實際寫出的檔案內容證實了衝突發生

`test agent` 在被要求寫入交接訊號後，實際寫出的 `debug-native-test-handoff.md` 第一行是正確的訊號，但接下來混入了一段不相關的 `GTO_OUTPUT_DIR`/`GTO_LOG_FILE`/`GTO_HANDOFF_FILE`/`GTO_ARTIFACT_DIR` 說明，並複誦了「Do not copy, move, or delete files in `.claude/session-handoff`. These are task-scoped execution paths, not system-prompt rules.」這段話——逐字對應到 `enrich_dispatch_markdown` 修正前的固定文字，證實這不是它自己編出來的，是先前某次任務派發時被塞進 context、之後被它當成背景規則援引。

## 3. 修正方案與實作

不修改 `session_boundary_guidance_block`（那段指示本身沒有問題，是 `enrich_dispatch_markdown` 那句警語沒考慮到自己可能跟別的功能衝突）。修法：讓 `enrich_dispatch_markdown` 知道「這次派發的目標 agent 是否也開了 `session_boundary_auto_split_enabled`」，開了就換一句不衝突、且說明兩個路徑分工的文字，而不是一律套用「不要碰」。

### 3.1 `crates/gt-task/src/lib.rs`

- `enrich_dispatch_markdown` 新增 `session_boundary_enabled: bool` 參數；`output_directory.is_some()` 時，依這個旗標選擇不同的最後一句：
  - `false`（沿用原文）：「Do not copy, move, or delete files in `.claude/session-handoff`. These are task-scoped execution paths, not system-prompt rules.」
  - `true`（新增）：「`.claude/session-handoff` is reserved for this agent's separate session-boundary auto-restart signal (see your system prompt's own boundary guidance) — do not use it for GT Office output-collection handoffs; use `GTO_HANDOFF_FILE` for those instead.」——講清楚兩個路徑分別歸誰管，而不是簡單地「不要碰」。
- `dispatch_batch_with_output_directories` 新增 `session_boundary_agents: &HashSet<String>` 參數，呼叫 `enrich_dispatch_markdown` 時傳入 `session_boundary_agents.contains(&target_agent_id)`。
- `dispatch_batch`（不帶 output directories 的舊版便利函式）維持三個參數不變，內部改呼叫新版時補一個空的 `&HashSet::new()`，呼叫端不受影響。
- 新增回歸測試 `output_instruction_does_not_contradict_session_boundary_guidance_when_both_enabled`：驗證兩個 checkbox 都開時，注入文字裡不再出現「Do not copy, move, or delete files in `.claude/session-handoff`」或「task-scoped execution paths」。

### 3.2 兩個呼叫端（`task.dispatch_batch` 的兩條路徑）

`apps/desktop-tauri/src-tauri/src/commands/task_center/mod.rs`（`task_dispatch_batch` 這個 Tauri command）與 `apps/desktop-tauri/src-tauri/src/local_bridge.rs`（`dispatch_batch`，供全域 bridge/`gto` CLI 呼叫的路徑）原本都只在同一個迴圈裡蒐集 `output_directories`（篩選 `agent.output_collection_enabled`）。兩處都補上蒐集 `session_boundary_agents`（篩選 `agent.session_boundary_auto_split_enabled`，只看在 `request.targets` 裡的 agent），一併傳給 `dispatch_batch_with_output_directories`。

## 4. 驗證

- `cargo check --workspace` — 通過。
- `cargo test -p gt-task --lib output_collection_tests` — 2 個測試皆通過（含新增的衝突回歸測試）。
  - 注意：`cargo test -p gt-task`（不加 `--lib`）目前會因為 `crates/gt-task/tests/lib_tests.rs` 裡 `AgentRuntimeRegistration` 缺少 `role_key` 欄位而編譯失敗——這是**修正前就已存在**的問題（`git diff` 確認這次改動完全沒碰這個檔案），跟本次修正無關，不在本次範圍內處理。
- `cargo fmt -p gt-task -- --check` — 通過（乾淨）。
- `cargo fmt -p gtoffice-desktop-tauri -- --check` — 只剩一處跟本次修改無關、修正前就存在的既有格式落差（`local_bridge.rs:803` 附近，不是這次新增/修改的程式碼）。
- `cargo clippy -p gt-task --lib -- -D warnings` — 無警告。
- `cargo clippy -p gtoffice-desktop-tauri --lib` — 這次改動到的行數附近無新增警告。
- ⚠️ **尚未做的驗證**：這次修正的是「未來派發任務時，注入文字不再互相矛盾」，但沒有重新做一次「`test agent` 這個已經開著兩個 checkbox 的 agent，真的到達任務邊界後，GT Office 是否會正確自動關閉並重啟終端」的實機測試。建議下次找機會補測；如果屆時 GT Office 已經派過帶有舊版矛盾文字的任務給它，它的對話 context 裡可能還留著舊指示，必要時要用一個乾淨重啟過的 session 測試才準。
