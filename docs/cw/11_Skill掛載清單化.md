# 11 Skill 掛載清單化：Workspace / 全域技能勾選 UI

> 承接 `docs/cw/09_P3.5-capability開發進度.md` §1.3 明確跳過的範圍：「Skills 子分頁不做勾選 workspace 內已存在的技能」。這份文件記錄把該缺口補上的實作，以及一輪獨立 code review（`林家維/2_Agents/BIM Agents/code-reviewer.md` persona）抓到的問題與修正。
>
> 狀態（2026-08-25）：程式碼已完成，已過一輪獨立 subagent code review 並修正其中 1 個 Critical + 2 個 Important 問題，**尚未 commit、尚未真人操作驗收**。

## 1. 這次做了什麼

Capabilities → Skills 子分頁原本只能手動輸入 Skill ID + 本機 `SKILL.md` 路徑。改成兩組可勾選清單：

- **專案工作區技能**：掃描 `<workspace_root>/.claude/skills/*/SKILL.md`
- **全域安裝技能**：掃描 `$CLAUDE_CONFIG_DIR/skills/*/SKILL.md`，未設定時 fallback 到 `~/.claude/skills/*/SKILL.md`（跟 Claude Code CLI 本身認的路徑一致）

每一列旁邊有一個 info 圖示，點開會展開顯示該技能 `SKILL.md` frontmatter 裡的 `description`（沒有就顯示「沒有提供說明」），讓使用者掛載前能先看這個技能做什麼。勾選/取消勾選對應既有 `SkillCapability.enabled` 欄位——關閉只是不掛載，不刪除設定，跟同一分頁 MCP Servers 清單的 on/off 開關語意一致（`docs/cw/10_P3.6-capability開發進度.md` §2）。

已掛載但掃描不到來源檔案的舊資料（例如這次改動之前手動輸入的、或來源檔案已搬移/刪除）不會被靜默丟棄，會顯示在「其他已掛載（找不到來源文件）」區塊，只能停用/移除，不能重新指向。

### 涉及檔案

後端（Rust）：
- 新增 `crates/gt-agent/src/capability/discovery.rs`：`SkillScope`、`DiscoveredSkill`、`list_available_skills()`、最小 YAML frontmatter 解析、6 個單元測試
- `crates/gt-agent/src/capability/mod.rs`：re-export `discovery`；`AgentCapabilitySnapshot::validate_for_tool` 新增重複 id 檢查（見第 2 節）
- `apps/desktop-tauri/src-tauri/src/commands/agent.rs`：`get_workspace_root` 改 `pub(crate)`
- `apps/desktop-tauri/src-tauri/src/commands/agent/capability.rs`：新 command `agent_capability_list_available_skills`（唯讀掃描）
- `apps/desktop-tauri/src-tauri/src/lib.rs`：註冊新 command

前端：
- `apps/desktop-web/src/shell/integration/desktop-api.ts`：`SkillScope`/`DiscoveredSkill` 型別、`agentCapabilityListAvailableSkills` wrapper
- `apps/desktop-web/src/features/workspace-hub/station-capabilities-model.ts`：移除 `createEmptySkill`（手動新增列已被清單取代，不再有呼叫點）；新增 `findMountedSkillEntry`/`toggleDiscoveredSkill`/`unmatchedSkillEntries`
- `apps/desktop-web/src/features/workspace-hub/StationCapabilitiesTab.tsx`：`SkillsEditor` 從手動輸入列表改寫成兩組清單 + fallback 區塊 + 可展開的 info 詳情列（新元件 `SkillChecklistGroup`）
- `apps/desktop-web/src/features/workspace-hub/StationManageModal.scss`：對應樣式

## 2. Code Review 記錄

用 `林家維/2_Agents/BIM Agents/code-reviewer.md` 這份 persona 餵給一個獨立 subagent 審查（五維度：正確性/可讀性/架構/安全/效能）。

**Critical（已修）**：`SkillCapability.id` 在 materialize 階段兼做目的地目錄名（`materialize.rs` 的 `write_skills_flag_overlay`/`sync_skills_copy_fallback` 都用 `skills_dir.join(&skill.id)`）。新 UI 用 `source_path` 分辨兩個掃描範圍（workspace vs. global），使用者可以同時勾選兩個目錄名相同、`source_path` 不同的技能，存檔時不會被擋下——materialize 時會後蓋前（旗標疊加模式）或靜默跳過第二個（複製 fallback 模式），使用者完全看不到任何錯誤。已修：`AgentCapabilitySnapshot::validate_for_tool` 新增 `validate_no_duplicate_enabled_skill_ids()`，對所有 *enabled* 的 skill 做 id 唯一性檢查，重複就整包拒絕存檔並回錯誤訊息（只檢查 enabled 的,因為 disabled 的技能本來就不會 materialize，不構成衝突）。新增 2 個單元測試：`rejects_two_enabled_skills_sharing_the_same_id`、`allows_duplicate_id_when_only_one_copy_is_enabled`。

**Important（已修）**：
1. 掃描失敗（`agentCapabilityListAvailableSkills` 拋錯）時，原本 `available` 永遠停在 `null`，導致「其他已掛載」fallback 區塊的 render 條件（`available !== null`）也跟著不顯示——已掛載但技能已失去清單畫面就完全看不到、也無法操作，即使 `draft.skills` 本身沒被動到。已修：`catch` 分支把 `available` fallback 成 `[]`（而非留著 `null`），讓 fallback 區塊在掃描失敗時仍然顯示，同時保留錯誤訊息。
2. 掃描的 `useEffect` 沒有處理「請求還沒回來、`workspaceId` 已經換了」的競態——舊回應可能在新 `workspaceId` 生效後才 resolve，把舊工作區的技能清單（含絕對路徑）蓋到新工作區的畫面上。已修：改用標準的 `cancelled` flag（effect cleanup 時設為 `true`），resolve/reject 時先檢查再套用 state。

**Important（已知、判斷為合理的範圍決策，不修改程式碼，本文件即是書面確認）**：移除手動輸入 Skill ID + 路徑的入口後，若某個技能的 `SKILL.md`不在 workspace `.claude/skills/` 或全域 skills 根目錄底下（例如某個 Claude Code plugin 私有目錄），現在完全無法新增掛載——只有這次改動之前就存在的舊資料能透過「其他已掛載」區塊繼續被停用/移除，不能重新指向或新增同類型的項目。這符合 `docs/cw/09_P3.5-capability開發進度.md` §1.3 原本的定位（手動輸入是「掃描功能還沒做」的暫代方案，不是要長期並存的另一條路徑），故判斷為合理、可接受的範圍縮減，而非改寫過程中的意外副作用。若之後有「技能來源在兩個掃描根目錄之外」的實際需求出現，需要另外規劃（例如讓使用者在設定裡新增自訂技能根目錄，而不是重新開放逐筆手動路徑輸入）。

**Suggestion（未採納，記錄留待未來）**：
- `.station-capabilities-skill-path` 的 `!important` 可以之後檢查是否能改用選擇器優先權解決
- 展開的詳情列目前只顯示 description + path，沒有顯示 `id`——顯示出來可以讓使用者自己肉眼發現上面 Critical 提到的跨範圍撞名情況，作為後端檢查之外的防禦層
- 沒有手動重新掃描按鈕：使用者在 modal 開著的時候於磁碟上增刪 `SKILL.md`，清單不會即時反映，要等分頁/modal 重新掛載

## 3. 驗證現況

- `cargo test -p gt-agent`：45 個測試全過（含新增的 discovery 6 個 + validate_for_tool 2 個）
- `cargo test -p gt-agent -p gt-storage`：全過
- `cargo check --workspace` / `cargo clippy -p gt-agent --all-targets` / `cargo fmt -p gt-agent`：過，無新增警告
- `npx tsc --noEmit`、`npm run typecheck`（含 production build + shared-types typecheck）：過
- `npx eslint`（本次改動的每個前端檔案）：過，零錯誤（開發過程中抓到並修正一個 `react-hooks/set-state-in-effect` 違規）

尚未做：真人在畫面上的操作驗收（這個環境沒有 UI 自動化工具）、`npm run build:tauri` production build。

## 3.1 後續補做：欄位置左修正 + 關鍵字搜尋

使用者實際看畫面後回報「skill 欄位名字要靠左」。追查發現一個真正的既有 CSS 規格問題，不是憑感覺猜的：`.station-form-grid label { flex-direction: column; }`（specificity `(0,1,1)`）比單一 class 的 `.station-capabilities-skill-checkbox`（`(0,1,0)`）優先權更高，導致這個 checkbox+名稱的 `<label>` 實際上被強制成直向排列、`align-items:center` 在直向下變成水平置中——checkbox 跟名稱看起來是「疊在一起且置中」，不是「checkbox 在左、名稱接著往右」。修法：改用複合選擇器 `.station-form-checkbox.station-capabilities-skill-checkbox`（specificity `(0,2,0)`，穩定贏過 ancestor 規則），明確設定 `flex-direction: row`。同時把這個修飾 class 也補到「其他已掛載」區塊原本沒有的 checkbox 上，保持一致。

另外新增關鍵字搜尋：`SkillsEditor` 新增一個搜尋框（`station-capabilities-skill-search`），用 `station-capabilities-model.ts` 新增的 `filterDiscoveredSkills`/`filterSkillEntries` 兩個純函式，對名稱／id／說明做不分大小寫的子字串比對，同時套用在工作區、全域、以及「其他已掛載」三個區塊。有搜尋字串但清單為空時，顯示「沒有符合搜尋條件的技能」而不是「這個工作區沒有技能」，避免使用者誤以為根本沒掃描到任何技能。

驗證：`npx tsc --noEmit`、`npx eslint`（改動檔案）、`npm --workspace apps/desktop-web run build`（production build，含 SCSS 編譯）皆過。

## 4. 建議的驗收清單

- [ ] 開啟一個 Claude agent 的能力分頁，切到 Skills 子分頁，確認「專案工作區技能」「全域安裝技能」兩組清單分別顯示
- [ ] 若工作區 `.claude/skills/` 或 `~/.claude/skills/` 下沒有任何技能，確認顯示對應的「沒有找到」文字而不是空白或報錯
- [ ] 勾選一個技能，點資訊圖示展開，確認顯示 SKILL.md 的 description
- [ ] 儲存後重開 modal，確認勾選狀態還在
- [ ] 取消勾選一個技能再存檔，確認該技能仍留在快照裡（`enabled: false`），下次重開仍看得到、可以再勾回來
- [ ] 手動在 `.claude/skills/` 底下建立兩個同名資料夾（一個在 workspace，一個在全域），確認同時勾選兩者時存檔會被拒絕並顯示清楚的錯誤訊息（Critical 修復的回歸測試）
- [ ] 讓 Codex agent 開啟能力分頁，確認 Skills 子分頁仍顯示「尚未支援」
- [ ] 確認技能清單裡 checkbox 跟名稱是同一行、名稱靠左對齊（不是疊在 checkbox 下面或置中）
- [ ] 在搜尋框輸入關鍵字，確認三個區塊（工作區/全域/其他已掛載）都會即時過濾；清空搜尋框後清單恢復完整
