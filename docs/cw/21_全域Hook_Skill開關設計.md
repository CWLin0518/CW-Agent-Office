# 全域 Hook/Skill 開關：讓「取消勾選」對全域項目真的生效

> 日期：2026-09-08
> 狀態：已實作、已通過 `cargo check --workspace`、`cargo test -p gt-agent -p gt-agent-session`、`cargo clippy -p gt-agent -p gt-agent-session --all-targets -- -D warnings`、`npm run typecheck`、`npx eslint`；尚未真人操作驗收
> 背景：[08_MCP_Hook_Skill掛載設計.md](../08_MCP_Hook_Skill掛載設計.md)、[11_Skill掛載清單化.md](11_Skill掛載清單化.md)、[12_Hook掛載清單化.md](12_Hook掛載清單化.md)

## 1. 問題

GT Office 的能力掛載清單讓使用者為每個 agent 勾選要掛載的 Hook/Skill，來源包含「全域已設定」（掃描使用者真正的 `~/.claude/settings.json`／`~/.claude/skills/`）。實測發現：取消勾選一筆全域項目，該 agent 還是會觸發它。

根因：GT Office 對 Claude Code 的整合方式是**疊加式 overlay**——`--mcp-config`/`--settings <runtime overlay>` 只會「額外」載入使用者勾選的項目，Claude Code CLI 本身仍會照常自動載入使用者真正的全域 `~/.claude/settings.json`（所有 hooks/permissions/env）與全域 `~/.claude/skills/*`，兩者並存疊加，不是「勾選清單取代全域設定」。GT Office 的勾選 UI 從未真正抑制過全域來源。

## 2. 已驗證的解法：`--setting-sources`

Claude Code CLI 有 `--setting-sources <user,project,local 的子集合>` 旗標，指定不含 `user` 時，會讓使用者真正的全域 `~/.claude/settings.json`（含其中的 hooks）與全域 `~/.claude/skills/*` 都不再自動載入。本次會話中用兩種方式各自空跑驗證過：

1. **Skill**：在 `~/.claude/skills/` 建立一個唯一命名的臨時 skill，`claude -p "列出你能用的所有 skill"`（預設）會列出它；加上 `--setting-sources project,local` 後就不會列出。
2. **Hook**：在 `~/.claude/settings.json` 的 `hooks.UserPromptSubmit` 暫時加一條會寫入 marker 檔的測試 hook，`claude -p "say hi"`（預設）會產生 marker；加上 `--setting-sources project,local` 後不會產生 marker。

兩次測試都已清除臨時檔案/還原設定，重新驗證過 `settings.json` 合法性。

**副作用**：這個旗標是整個來源一起排除，不能只排除單一 hook/skill——使用者累積在 `~/.claude/settings.json` 裡的 `permissions.allow` 清單也會一起失效，該 agent 之後會需要更多手動確認。CLI 層面無法精細規避這一點。

## 3. 設計：agent 層級主開關，而非逐項勾選直接生效

新增一個 agent 層級主開關（「使用全域 Hook / Skill / 權限設定」）：

- **開（預設值，等同現狀）**：完全不變，全域設定照常全部套用；個別全域項目的勾選框僅供檢視、不可互動（避免「看起來能控制、實際上控制不了」的誤導 UI，這正是原始 bug 的體感）。
- **關**：launch command 加上 `--setting-sources project,local`，使用者真正的全域設定完全不再自動套用；此時個別全域 Hook/Skill 的勾選框才變成有意義的操作——勾選的項目透過 GT Office 既有的 overlay 機制個別重新掛載回來。切換為「關」的當下，畫面會把目前掃描到的全域項目全部預設勾選（避免關閉主開關的瞬間，原本在用的東西無聲消失），使用者之後再自行取消勾選不要的。

範圍：只動 Claude 供應商。Codex 目前不支援 Hooks/Skills 分頁（`docs/cw/12` 已記錄），`--setting-sources` 也是 Claude CLI 專屬旗標。

## 4. 關鍵設計決策：開關放進 `AgentCapabilitySnapshot`，不是 `AgentProfile`

一開始考慮比照 `session_boundary_auto_split_enabled`/`output_collection_enabled`（`agents` 資料表欄位），但那條路需要新增 DB 欄位＋migration，且這個布林值要跟「已掛載的 Hook/Skill 清單」互動（切換時要改 `draft.hooks`/`draft.skills`），但清單狀態活在 `StationCapabilitiesTab.tsx` 元件內、跟 `AgentProfile` 的存檔路徑（`StationManageModal.tsx` 的 Save 按鈕）是兩條分開的存檔流程，需要額外的跨元件橋接。

改放進 `AgentCapabilitySnapshot`（`crates/gt-agent/src/capability/mod.rs`）之後：

- 這個 struct 整包序列化成 `agent_capability_snapshots.capability_json` 一個 TEXT 欄位，**新增欄位不需要任何 DB schema/migration 改動**。
- 開關狀態、Hook/Skill 掛載清單本來就活在同一份 `draft: AgentCapabilitySnapshot` 狀態裡，同一個 `setDraft` 呼叫就能同時改開關跟清單。
- 存檔天生就走 `agent_capability_save`，新欄位自動隨之持久化。
- `agent_capability_preview_hooks`/`agent_capability_confirm_hooks` 只針對 `hooks` 陣列做「執行任意指令，需要逐條 preview+confirm」的風險控管（docs/cw/08 §2.5 決策3）；這個新開關性質上更接近 Skill/MCP 的「是否啟用」層級，不需要也不應該接進 hook 專屬的 confirm-hash 機制。

### 4.1 `Default` derive 陷阱（差點讓每個新 agent 預設關閉）

`AgentCapabilitySnapshot`原本是 `#[derive(..., Default, ...)]`。`serde(default = "...")` 只影響「JSON 反序列化時缺這個 key 怎麼補」，**不影響** `AgentCapabilitySnapshot::default()` 這個 Rust 呼叫本身——衍生的 `Default` 對 `bool` 一律給 `false`。而 `SqliteAgentRepository::get_agent_capability`（`crates/gt-storage/src/agent_repository.rs`）在**全新 agent、從來沒存過 capability** 時就是回傳 `Ok(AgentCapabilitySnapshot::default())`。

如果沒處理，每個新建立的 agent 一開始就會是「關閉」狀態，跟預期完全相反。修法：把 `Default` derive 拿掉，改成手寫 `impl Default for AgentCapabilitySnapshot`，讓 `global_capabilities_enabled` 明確等於 `true`。這個坑值得記錄下來——以後任何在這個 struct 上新增「預設不是 `false`/空集合」的欄位，都要重新檢查這一點。

## 5. 涉及檔案

後端（Rust）：

- `crates/gt-agent/src/capability/mod.rs`：`AgentCapabilitySnapshot` 新增 `global_capabilities_enabled` 欄位；`Default` derive 改手寫 impl（見 4.1）。
- `crates/gt-agent/src/capability/materialize.rs`：`MaterializedPaths` 新增同名欄位；`materialize_claude_capability` 兩個回傳分支（正常路徑、快取命中路徑）都把值從 `snapshot.global_capabilities_enabled` 帶過去。**修正**（code review 抓到）：快取命中分支那段用 struct-update 語法覆寫欄位的寫法其實是防禦性的，不是有效的效能設計——`compute_content_hash` 是對整包 `snapshot.to_json()` 取雜湊，`AgentCapabilitySnapshot` 沒有幫這個欄位加 `#[serde(skip)]`，所以它本來就會被含進雜湊輸入：只要這個開關改變，`content_hash` 就會跟著變、直接落入正常路徑（cache miss），快取命中分支在這個情境下根本不會被打到。程式碼註解已更新說明這一點。
- `apps/desktop-tauri/src-tauri/src/commands/agent/capability.rs`：`materialize_capability_for_launch` 的「三個陣列都空就提早回傳 None」判斷，加上 `&& snapshot.global_capabilities_enabled` 條件——否則「開關關閉但沒勾選任何個別全域項目」的 agent 會被提早判定沒東西要 overlay，`--setting-sources` 永遠沒機會被加上去。
- `crates/gt-agent-session/src/resume.rs`：`apply_capability_overlay` 在既有的 `--mcp-config`/`--settings` 之後加 `--setting-sources project,local`（當 `!materialized.global_capabilities_enabled`）。不需要改任何函式簽名——`materialized: &MaterializedPaths` 本來就有這個新欄位。新增 3 個測試涵蓋開/關兩種情況與「這個 Claude 專屬旗標絕不會滲到 Codex 命令」。

前端（TypeScript）：

- `apps/desktop-web/src/shell/integration/desktop-api.ts`：`AgentCapabilitySnapshot` 型別新增 `globalCapabilitiesEnabled: boolean`；`createDefaultAgentCapability()` 補上 `true`。不需要新的 invoke wrapper——`agentCapabilityRead`/`agentCapabilitySave` 本來就整包傳遞這個型別。
- `apps/desktop-web/src/features/workspace-hub/station-capabilities-model.ts`：
  - `buildSavableCapabilitySnapshot` 補上把 `globalCapabilitiesEnabled` 原樣帶過去（原本漏掉，TypeScript 因為型別變成必填欄位而直接編譯失敗，及早抓到）。
  - 新增 `enableAllGlobalHooks`（直接複用既有 `setAllHooksEnabled`，過濾 `scope === 'global'`）與 `enableAllGlobalSkills`（**沒有**複用 `setAllSkillsEnabled`——那個函式有「把所有不在 discovered 清單裡的已掛載項目一併強制設成 enabled」的 fallback，是給它自己的「全選開啟/關閉」UI 用的；如果拿一個只過濾出 global 的子集合傳進去，會把工作區項目也一併強制覆寫，所以改用跟 `setAllHooksEnabled` 一樣的單純迴圈）。
- `apps/desktop-web/src/features/workspace-hub/StationCapabilitiesTab.tsx`：
  - **把 discovered hooks/skills 的抓取從 `HooksEditor`/`SkillsEditor` 內部搬到父層** `StationCapabilitiesTab` 本身。原因：主開關切到「關」的當下，需要同時拿得到 discovered 清單才能自動勾選，但 `HooksEditor`/`SkillsEditor` 只有在對應子分頁被選中時才會掛載——如果邏輯留在子元件內，使用者在「Hooks」子分頁切換主開關時，`SkillsEditor` 根本沒掛載、拿不到 skill 清單，等使用者之後才切到「Skills」分頁，時機已經錯過。
  - 新增主開關 checkbox（`station-form-checkbox`，跟 `StationManageModal.tsx` 既有的 `session_boundary_auto_split_enabled` checkbox 同一個 class），放在子分頁切換列上方，只在 `skillsHooksSupported` 為真時顯示（Codex 不顯示）。
  - `onChange`：偵測 `true → false` 這個瞬間轉換時，用 `enableAllGlobalHooks`/`enableAllGlobalSkills` 把當下已知的全域項目全部併入 `draft`；其餘情況單純寫入開關值。
  - `HooksEditor`/`SkillsEditor` 的「全選開啟/全選關閉」按鈕、以及全域那組 `HookChecklistGroup`/`SkillChecklistGroup`，在主開關為「開」時，改吃 `disabled={disabled || globalCapabilitiesEnabled}`（只鎖住全域那組，工作區那組不受影響），並在標題文字加註「唯讀」提示。

## 6. 驗證現況

- `cargo check --workspace` — 通過。
- `cargo test -p gt-agent --lib capability` — 69 個測試全過（含既有的 `missing_fields_deserialize_to_defaults`，確認舊資料反序列化不受影響）。
- `cargo test -p gt-agent-session --lib` — 除了一個跟本次改動完全無關、在改動前就存在的既有失敗（`scanner::tests::test_project_key_encoding`，Windows 環境下的路徑編碼假設問題，已用 `git stash` 確認 main 分支上同樣失敗）之外全過，含本次新增的 3 個 `apply_capability_overlay`/`--setting-sources` 測試。
- `cargo clippy -p gt-agent -p gt-agent-session --all-targets -- -D warnings` — 無警告。
- `cargo fmt`（改動檔案）— 已跑。
- `npm run typecheck`（`apps/desktop-web` production build + `shared-types`）— 通過。
- `npx eslint`（本次改動的三個前端檔案）— 通過，零錯誤。

尚未做：真人在畫面上的操作驗收（這個環境沒有 UI 自動化工具）。

## 7. 建議的驗收清單

- [ ] 開一個新 agent，確認能力分頁的主開關預設是「開」
- [ ] 切換主開關為「關」，確認全域 Hook/Skill 清單自動全部勾上，且變成可互動狀態（工作區清單不受影響）
- [ ] 存檔、重開 modal，確認開關狀態與勾選狀態都有正確持久化
- [ ] 取消勾選其中一條全域 Hook，存檔後實際啟動這個 agent，確認終端機真正跑出來的指令帶有 `--setting-sources project,local`，且該 Hook 真的不再觸發（可比照本次驗證用的 marker-hook 手法）
- [ ] 主開關開著的時候，確認全域項目的勾選框是 disabled 狀態，且「全選開啟/關閉」按鈕不會影響到它們
- [ ] 主開關開著的時候，確認全域項目的勾選框顯示為**已勾選**（因為它們此時確實生效中），而不是照著實際掛載狀態顯示未勾選
- [ ] 全域 Hook/Skill 掃描尚未完成時，確認主開關本身是 disabled 狀態、無法搶在掃描完成前切換
- [ ] Codex agent 的能力分頁，確認完全看不到這個新開關

## 8. Code Review 記錄（2026-09-08）

第一版經 code-reviewer 審查，結論 REQUEST CHANGES，抓到以下問題，已修正：

1. **Important（已修）**：主開關的 `disabled` 只綁定 `saving`，沒有等待 `discoveredHooks`/`discoveredSkills` 這兩個獨立、非同步的掃描完成。若使用者在掃描完成前就把開關切成「關」，`onChange` 裡的自動勾選邏輯會拿到 `[]`（尚未載入完成時是 `null`，fallback 成空陣列），導致「切換為關的當下自動勾上目前已知全域項目」這個安全機制完全沒有生效——等於重現了這個功能原本要修的 bug。修法：`disabled` 加上 `|| discoveredHooks === null || discoveredSkills === null`，並顯示「正在掃描…」提示。
2. **Important（已修）**：全域清單勾選框的 `checked` 狀態算的是「是否已被個別掛載」，不是「目前是否實際生效」。當主開關為「開」（預設值，多數 agent 的實際狀態）時，一個從未個別掛載過任何全域項目的 agent，清單裡每一項都會顯示成**未勾選**——但主開關開著代表這些項目全部生效中，這是跟原始 bug 對稱的另一種誤導 UI（「看起來沒生效、實際上生效中」）。修法：`SkillChecklistGroup`/`HookChecklistGroup` 新增 `forceChecked` prop，全域那組在主開關為開時一律強制顯示已勾選。
3. **Suggestion（已修）**：`materialize.rs` 的註解與本文件都誤稱 `global_capabilities_enabled`「不進雜湊快取比對」，實際上 `compute_content_hash` 是對整包 `snapshot.to_json()` 取雜湊，這個欄位本來就會被含進去——切換開關本身就會讓 `content_hash` 改變、直接落入正常寫入路徑，快取命中分支那段 struct-update 覆寫其實是防禦性寫法，不是有效的效能設計。已修正程式碼註解與本文件第 5 節的說明（見上方「修正」標註）。

修正後重新驗證：`cargo check --workspace`、`npm run typecheck`、`npx eslint`（見下方更新後的驗證記錄）。

## 9. 後續簡化（2026-09-08）：全域清單改為「純檢視」，不再有「關閉後可個別勾選」模式

第 3 節原設計中「主開關關閉後，全域清單變成可互動、可個別勾選掛載」這個模式，使用回饋後決定移除——全域清單一律唯讀，**唯一**的控制點就是最上方的主開關；要單獨保留/移除某一條全域項目，改用「手動新增 / 其他已挂載」區塊管理，不再透過全域清單本身勾選。

改動：

- 前端（`station-capabilities-model.ts`）：移除 `enableAllGlobalHooks`/`enableAllGlobalSkills`（不再需要「關閉主開關瞬間自動勾上全域項目」這個安全機制，因為全域項目再也不會透過清單勾選掛載）。`unmatchedHookEntries`/`unmatchedSkillEntries` 判斷「已掛載但未比對到 discovered 項目」時，改成只拿 `scope === 'workspace'` 的 discovered 項目來比對——原本個別勾選掛載、內容剛好對應到某條全域項目的舊資料，因此會落入「手動新增 / 其他已挂载」區塊（該區塊本來就有刪除/取消勾選按鈕），而不是消失在唯讀清單裡無法移除。
- 前端（`StationCapabilitiesTab.tsx`）：
  - `SkillChecklistGroup`/`HookChecklistGroup` 的 `forceChecked` prop 改成 `viewOnly` + `viewOnlyActive`——`viewOnly` 為真時整組不渲染 checkbox，只顯示名稱＋一個「套用中／未套用」徽章（複用既有的 `.station-hook-preview-badge` class）；徽章狀態單純反映主開關 `globalCapabilitiesEnabled`，不再讀取個別掛載狀態。
  - 全域清單分組永遠傳 `viewOnly`（不論主開關開或關），`disabled`/`forceChecked` 的開關條件分支移除。
  - 「全選開啟／全選關閉」按鈕永遠只作用在工作區清單，不再依主開關分支到全域清單。
  - Discovered hooks/skills 的抓取邏輯搬回 `HooksEditor`/`SkillsEditor` 內部（原本第 65 點提到的「搬到父層」理由——主開關切換瞬間需要同時取得兩份清單來自動勾選——已經不存在，因為主開關切換不再對 `draft.hooks`/`draft.skills` 產生任何副作用）。主開關本身的 `disabled` 條件也簡化為只看 `saving`，不再需要等待兩個掃描都完成。

驗證：`npm run typecheck`（`tsc -b` + `vite build` + shared-types）、`npx eslint`（兩個改動檔案）皆通過。尚未做：真人操作驗收。

### 9.1 Code Review 抓到的兩個問題（已修）

1. **Important**：`HookChecklistGroup` 展開的詳情面板裡，備註 `<textarea>` 只判斷 `mounted` 是否存在，沒有一併判斷 `viewOnly`——只要某條全域 Hook 的內容剛好對應到一筆已掛載資料（例如這次改動前遺留的舊資料），唯讀分組底下還是會冒出一個可編輯、會呼叫 `onChange` 的備註欄位，跟「全域清單一律唯讀」的設計目標矛盾。修法：條件改成 `!viewOnly && mounted`。
2. **Important**：`setAllSkillsEnabled` 原本除了迴圈套用 `discovered` 清單外，還有一段 fallback：把所有 `sourcePath` 不在 `discovered` 裡的已掛載項目一併強制設成 `enabled`。這段邏輯是在「呼叫端永遠傳全部 discovered（工作區＋全域）」的前提下才安全——原本用來一併處理「來源檔案真的找不到」的孤兒項目。這次改動後 `StationCapabilitiesTab.tsx` 的「全選開啟/關閉」按鈕改傳只有工作區範圍的 `workspaceSkillsAll`，這段 fallback 就會連帶把手動新增與內容剛好對應全域項目的已掛載技能一併強制翻轉，蓋掉使用者在「手動新增 / 其他已挂载」區塊裡的個別勾選。修法：拿掉這段 fallback，改成跟 `setAllHooksEnabled` 一樣「只作用在 checklist 來源的項目」的單純迴圈。

修正後重新驗證：`npm run typecheck`、`npx eslint`（兩個改動檔案）皆通過。
