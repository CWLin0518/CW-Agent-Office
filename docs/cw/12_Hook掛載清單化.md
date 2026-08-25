# 12 Hook 掛載清單化：延伸 Skill 的掃描 + 勾選模式

> 承接 [11_Skill掛載清單化.md](11_Skill掛載清單化.md) 的模式，把同樣的「掃描既有設定 + 勾選清單」做法延伸到 Hooks 子分頁。跟 Skill 不同的是：`docs/cw/08_MCP_Hook_Skill掛載設計.md` 決策 3 明確要求 Hook 保留「開放自訂指令」的手動輸入路徑（風險最高、需要逐條 preview + confirm），所以這次是**清單與手動輸入並存**，不是像 Skill 那樣整個取代掉手動輸入。
>
> 狀態（2026-08-25）：程式碼已完成，已過 `cargo test` / `cargo clippy` / `npm run typecheck`（含 production build）/ `npx eslint`，**尚未 commit、尚未真人操作驗收**。

## 1. 這次做了什麼

Capabilities → Hooks 子分頁原本只有手動輸入（事件下拉 + matcher 輸入框 + 指令 textarea）。新增一組可勾選清單，掃描來源：

- **專案工作區已設定的 Hook**：讀 `<workspace_root>/.claude/settings.json` 的 `hooks` 區塊
- **全域已設定的 Hook**：讀 `$CLAUDE_CONFIG_DIR/settings.json`，未設定時 fallback 到 `~/.claude/settings.json`

勾選/取消勾選對應 `draft.hooks` 陣列的成員身分——`HookCapability` 沒有 `enabled` 欄位（跟 `SkillCapability`/`McpServerCapability` 不同），所以「掛載」單純是「在陣列裡」，取消勾選是直接移除該筆，不是像 Skill 那樣切一個旗標。

手動輸入表單保留，改名「手動新增的 Hook」，只顯示清單掃描不到、視為使用者自訂的項目（用事件+matcher+指令三者比對，不新增額外 id 欄位）。不論來源是勾選還是手動輸入，存檔前都要走原本就有的完整 preview + confirm 流程（`requiresHookPreviewBeforeSave` 只看 `hooks.length > 0`，沒有為清單來源開特例）。

### 跟 Skill 掃描的關鍵差異

`.claude/settings.json` 的真實 hooks 格式是巢狀在頂層 `"hooks"` 鍵底下（`{"hooks": {"PreToolUse": [...]}}`），這點已經對照 Claude Code 官方 hooks 文件核實過。這**不同於**這個 repo 自己 `materialize.rs` 的 `build_settings_json` 產生的格式——那是給自己合成的 `--settings <file>` overlay 用的（`.gtoffice/agents/<agent_id>/runtime/settings.json`），事件名稱直接放在根層級、沒有 `"hooks"` 包裝。兩者是不同檔案、不同用途，掃描讀「使用者真正的」`.claude/settings.json` 時必須用有包裝的那個格式，寫回自己生成的 runtime overlay 時維持原本無包裝的格式不變（`materialize.rs` 沒有改動）。

### 涉及檔案

後端（Rust）：
- `crates/gt-agent/src/capability/discovery.rs`：新增 `HookScope`、`DiscoveredHook`、`list_available_hooks()`、`scan_settings_hooks()`（解析 `hooks` 巢狀鍵，只認 `type: "command"` 的 handler），10 個單元測試（含「根層級事件鍵格式必須被忽略，那是 materialize 自己的 overlay 格式」的防呆測試）
- `apps/desktop-tauri/src-tauri/src/commands/agent/capability.rs`：新 command `agent_capability_list_available_hooks`（唯讀掃描，不影響 preview/confirm 流程）
- `apps/desktop-tauri/src-tauri/src/lib.rs`：註冊新 command

前端：
- `apps/desktop-web/src/shell/integration/desktop-api.ts`：`HookScope`/`DiscoveredHook` 型別、`agentCapabilityListAvailableHooks` wrapper
- `apps/desktop-web/src/features/workspace-hub/station-capabilities-model.ts`：新增 `findMountedHookEntry`/`toggleDiscoveredHook`/`unmatchedHookEntries`/`filterDiscoveredHooks`/`filterHookEntries`，以及內部用的 `hookContentKey`（event+matcher+command 用 `\u0000` 分隔比對身分，純記憶體內比對，不是 persisted 的 `HookCapability::content_hash`）
- `apps/desktop-web/src/features/workspace-hub/StationCapabilitiesTab.tsx`：`HooksEditor` 改寫成「掃描清單（新元件 `HookChecklistGroup`，比照 `SkillChecklistGroup`）+ 手動新增區塊」；手動區塊改用物件參照比對（`hook === target`）取代原本的陣列 index，避免清單套用搜尋過濾後 index 對不上

樣式：沿用既有 `.station-capabilities-skill-*` class（本來就是通用的勾選清單樣式，沒有寫死「skill」字樣的內容），沒有新增 SCSS。

## 1.1 後續補做：每筆 Hook 增加「備註」欄位

每筆 Hook 新增一個 `note`（`Option<String>`）欄位，讓使用者說明這筆 Hook 的觸發時機與功能——純 UI 用途的自由文字，**不影響 `content_hash`**（`HookCapability::content_hash()` 只吃 event/matcher/command）也**不會寫進** `materialize.rs` 生成的 `.gtoffice/agents/<agent_id>/runtime/settings.json`（真實 Claude Code hooks schema沒有這個欄位）。這個設計決策的理由：備註純粹是描述性metadata，不改變 Hook 實際會執行什麼，所以編輯備註不應該逼使用者重新走一次 preview/confirm。已加一個專門測試 `hook_content_hash_ignores_note` 鎖住這個行為。

- **手動新增的 Hook**：表單多一個「備註」textarea，跟 event/matcher/command 平行
- **清單勾選來源的 Hook**：勾選後在展開的詳情面板裡出現同一個備註 textarea（未勾選的掃描結果沒有對應的掛載項目，自然沒有備註可編輯）
- Preview + confirm 畫面：如果有填備註，逐條列出時會多顯示一行「備注：...」，方便存檔前核對

涉及檔案（追加）：`crates/gt-agent/src/capability/mod.rs`（`HookCapability.note` 欄位 + 測試）、`apps/desktop-tauri/src-tauri/src/commands/agent/capability.rs`（`agent_capability_preview_hooks` 回傳值多帶 `note`）、`apps/desktop-web/src/shell/integration/desktop-api.ts`、`station-capabilities-model.ts`（`updateMountedHookNote`、`buildSavableCapabilitySnapshot` 補 note 正規化）、`StationCapabilitiesTab.tsx`。

## 1.2 後續補做：Agent Canvas 節點卡片的 Hook 顯示改用備註

Agent Canvas 上每個 agent 節點展開 Hooks 清單時，原本可見文字跟 tooltip 都是 `hook.command`（原始指令字串），使用者光看畫布看不出這個 Hook 是做什麼用的。改成：有填備註就顯示備註（人看得懂的說明），沒填才退回顯示原始指令；tooltip 則備註跟指令都顯示（備註在上、指令在下），需要核對實際指令時仍看得到。涉及檔案：`apps/desktop-web/src/features/agent-canvas/components/AgentCanvasNodeCard.tsx` 的 `AgentCanvasHookNodeCard`。

## 1.3 後續補做：掃描到但還沒掛載的 Hook，自動從腳本推測功能說明

1.1 的備註欄位只解決「已掛載」的 Hook——一筆**還沒勾選、只是掃描到**的 Hook（來源是使用者真實的 `.claude/settings.json`），沒有地方能填備註，因為它在 `draft.hooks` 裡根本還沒有對應的項目。而真實 `.claude/settings.json` 本身沒有描述欄位（已對照 Claude Code 官方 hooks 文件確認），所以光看掃描結果，使用者除了讀原始指令字串以外無從得知這個 Hook 的功能。

解法：`list_available_hooks` 掃描到一筆 hook 之後，嘗試從 `command` 裡解析出它呼叫的腳本檔路徑（辨識 `.sh`/`.bash`/`.js`/`.mjs`/`.cjs`/`.py`/`.ps1`/`.rb`，處理 `${CLAUDE_PROJECT_DIR}`/`$CLAUDE_PROJECT_DIR` 展開跟 `~`），讀取該腳本開頭的註解區塊（支援 `#`、`//`/`///`、`/* */` 三種註解風格，自動跳過 shebang），當作 `DiscoveredHook.inferredDescription` 顯示在清單展開的詳情面板裡，並清楚標註「推測的功能說明（讀取腳本開頭注釋，僅供參考）」。

這是盡力而為（best-effort），不是保證：
- 指令是 inline 指令（例如 `echo hi`）、沒有指向任何腳本檔 → `None`
- 解析出的路徑實際上不存在、讀不到 → `None`
- 腳本開頭沒有註解 → `None`
- 從 `command` 字串猜測腳本路徑本身是啟發式判斷（用簡單的、有處理引號的 tokenizer 取第一個副檔名符合的 token），不是完整的 shell 語法解析，不保證 100% 準確

勾選一筆有 `inferredDescription` 的掃描結果時，會直接把這段說明帶進新掛載的 `HookCapability.note`（使用者仍可再編輯或清空）——不用手動重複打一次腳本已經寫好的說明。搜尋框（`filterDiscoveredHooks`）也把 `inferredDescription`納入比對範圍。

涉及檔案：`crates/gt-agent/src/capability/discovery.rs`（`DiscoveredHook.inferred_description`、`infer_hook_description`/`extract_script_path`/`resolve_script_path`/`read_leading_comment`/`tokenize_command` + 8 個新測試）、`apps/desktop-web/src/shell/integration/desktop-api.ts`（`DiscoveredHook.inferredDescription`）、`station-capabilities-model.ts`（`filterDiscoveredHooks` 納入比對、`toggleDiscoveredHook` 掛載時帶入 `note`）、`StationCapabilitiesTab.tsx`（`HookChecklistGroup` 詳情面板顯示）。

## 2. 驗證現況

- `cargo test -p gt-agent`：63 個測試全過（discovery 模組 18 個 hook 相關測試，含 8 個腳本推測說明測試 + `hook_content_hash_ignores_note`）
- `cargo test -p gt-storage`：25 個測試全過
- `cargo clippy -p gt-agent -p gt-storage --all-targets`、`cargo fmt`（改動檔案）：過，無新增警告
- `cargo check --workspace`：過
- `npm run typecheck`（`apps/desktop-web` production build + `shared-types` typecheck）：過
- `npx eslint`（本次改動的四個前端檔案）：過，零錯誤

（`cargo clippy -p gtoffice-desktop-tauri --all-targets` 目前有 41 個既有、跟這次改動無關的編譯錯誤——`local_bridge_tests.rs` 缺 `role_key`/`list_roles`/`seed_agent_defaults`，屬於另一個進行中的 roles 功能，已用 `git stash` 確認在這次改動之前就存在，不在這次範圍內處理。）

尚未做：真人在畫面上的操作驗收（這個環境沒有 UI 自動化工具）。

## 3. 建議的驗收清單

- [ ] 在某個 workspace 的 `.claude/settings.json` 手動加一條 `hooks.PreToolUse`，開啟該 workspace 內 Claude agent 的能力分頁 → Hooks 子分頁，確認「专案工作区已设定的 Hook」清單顯示這一條
- [ ] 勾選一條掃描到的 Hook，確認觸發存檔時的完整 preview + confirm 畫面（不能因為是勾選來源就跳過）
- [ ] 存檔後重開 modal，確認勾選狀態還在（陣列裡有這筆）
- [ ] 取消勾選一條已掛載的掃描 Hook，確認它從「手动新增的 Hook」區塊消失（不是變成停用狀態留著，因為 `HookCapability` 沒有 `enabled`）
- [ ] 在「手动新增的 Hook」區塊新增一條自訂指令並存檔，確認不受清單掃描影響、走原本手動輸入的流程
- [ ] 讓 Codex agent 開啟能力分頁，確認 Hooks 子分頁仍顯示「尚未支援」
- [ ] 在搜尋框輸入關鍵字，確認工作區/全域/手動新增三個區塊都會即時過濾
- [ ] `~/.claude/settings.json` 或 workspace `.claude/settings.json` 不存在時，確認清單顯示「沒有找到」而不是報錯或空白
