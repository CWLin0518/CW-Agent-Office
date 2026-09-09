# SessionBoundary 自動判斷 Stop Hook 設計

> 日期：2026-09-08，第二輪修正 2026-09-09（見第 8 節）
> 狀態：已設計並在使用者個人全域環境（`~/.claude/`）落地、經 code-reviewer 審查一輪並修正（見第 6 節）、通過腳本層級手動測試；實機使用後回報清單判斷過於保守，已收斂清單措辭並同步 repo 內 `session_boundary_guidance_block()`（見第 8 節）；尚未實機驗證「GT Office 站台終端機裡真的到達邊界時，這條路徑會不會正確觸發並開出新 session」
> 背景：[18_全域SessionBoundaryHook排查與移除.md](18_全域SessionBoundaryHook排查與移除.md)、[19_輸出收集與SessionBoundary訊息衝突排查.md](19_輸出收集與SessionBoundary訊息衝突排查.md)

## 1. 要解決的問題

GT Office 現有的「任務邊界自動重啟」機制（`session_boundary_auto_split_enabled` 開啟時，`apps/desktop-tauri/src-tauri/src/commands/agent.rs` 的 `session_boundary_guidance_block()` 會把邊界判斷規則寫進 CLAUDE.md/AGENTS.md）完全依賴 Agent 在生成回覆的過程中「自己想起來」要不要檢查邊界。這套規則本身是純語意 heuristic（「下一階段目標不同」「上一階段已達穩定檢查點」……），沒有任何客觀、可程式化驗證的觸發條件，所以：

- 判斷頻率不確定：Agent 可能整段長任務都沒意識到要檢查，也可能在無關緊要的子任務切換上就誤判。
- 這不是 prompt 措辭能單靠調整字句解決的問題——缺的是「有沒有機會被逼著檢查一次」，不是「規則寫得夠不夠精確」。

## 2. 設計方向：用 `Stop` Hook 強制檢查點，語意判斷仍交給 Agent

不新增另一套判斷規則，也不取代 `session_boundary_guidance_block` 既有的邊界條件清單（兩邊用同一套判準文字，保持一致）。改用 Claude Code 的 `Stop` hook，在**每一輪 Agent 準備把控制權交還之前**強制插入一次「對照清單自我檢查」，把「要不要檢查」從 Agent 自由心證，變成一個保證會發生、但只發生一次的機制性事件。

Hook 的職責到「逼 Agent 檢查一次、看它有沒有寫出交接訊號」為止；訊號寫出之後,真正開新 session 的動作完全沿用既有機制（GT Office 前端的 reconciliation 掃描/檔案監聽偵測到 `.claude/session-handoff/*-handoff.md` 訊號檔），這一段本次沒有改動。

### 2.1 機制細節

Claude Code 的 `Stop` hook 在 Agent 每輪回覆結束、準備停止時觸發，透過 stdin 收到一份 JSON（含 `stop_hook_active` 欄位），並可以在 stdout 輸出 `{"decision":"block","reason":"..."}` 來攔截這次停止——一旦攔截，`reason` 的內容會被當成新的一輪輸入餵回 Agent，逼它針對這段文字再生成一次回覆。這正好提供「強制檢查點」所需要的兩個能力：**攔截**與**注入清單內容**。

- **防無限迴圈**：`stop_hook_active` 為 `true` 代表這次 Stop 本身就是「上一次被本 hook 攔截」之後的結果，此時一律放行，確保每輪最多只被逼問一次。
- **清單內容**：`reason` 裡放的判準文字，措辭沿用 `session_boundary_guidance_block()` 既有的邊界條件（視為邊界 / 不視為邊界的具體列舉），以及達標時要用 Write 工具寫出 `<SESSION_CONTROL action="restart_in_place" />` 訊號的既有格式，確保跟現有機制對得上。

### 2.2 範圍限制：兩層 guard，全域安裝也不會誤觸發

這個 hook 註冊在使用者個人的 `~/.claude/settings.json`（全域層級），會套用到這台機器上所有 Claude Code session。要避免的不只是「跟 GT Office 完全無關的獨立終端機工作」，還有「GT Office 站台終端機，但這個 agent 明確關閉了任務邊界自動重啟」——這兩種情況都必須放行，缺一不可，兩層 guard 都要通過才會真正攔截：

1. **是不是 GT Office 幫某個 Agent 站台開的終端機**：檢查 `GTO_WORKSPACE_ID`、`GTO_AGENT_ID`、`GTO_STATION_ID` 三個環境變數（見 [18_全域SessionBoundaryHook排查與移除.md](18_全域SessionBoundaryHook排查與移除.md) 3.3(b)）。**注意**：這三個變數是每一個站台終端機啟動時無條件寫入的（`apps/desktop-web/src/shell/layout/useShellTerminalController.ts` 建構 `terminalEnv` 時沒有依 `sessionBoundaryAutoSplitEnabled` 篩選），不能代表這個 agent 真的開啟了任務邊界自動重啟——這是第一版設計遺漏的地方，code review 抓出後補上第 2 層。
2. **這個站台是否真的開啟了任務邊界自動重啟**：檢查站台工作目錄下的 `CLAUDE.md`/`AGENTS.md` 裡有沒有 `session_boundary_guidance_block()` 寫入的既有標記（`<!-- gtoffice:session-boundary-guidance:start -->`）。這個標記本來就只在 `session_boundary_auto_split_enabled` 開啟時才會被寫入（`apply_session_boundary_guidance()`），是現成、單一來源的權威訊號，不需要在 repo 裡另外新增一個平行的環境變數去維護一致性。

兩層都沒過，腳本立刻 `exit(0)` 放行，不做任何事、不輸出任何內容——對獨立終端機工作、以及明確關閉此功能的站台而言，都是零成本、零干擾。

## 3. 實作位置

**不屬於這個 repo**，比照 `docs/cw/18` 對舊版全域 hook 的定位——這是使用者個人機器上的全域設定，不隨 CW-Agent-Office 專案版本控制：

- 腳本：`~/.claude/hooks/session-boundary-checklist.js`（純 Node.js，無外部依賴，風格比照同目錄下既有的 `prompt-code-review-check.js`）。
- 註冊：`~/.claude/settings.json` 的 `hooks.Stop`，與既有的 `hooks.UserPromptSubmit`（code-review 提醒 hook）並列，兩者互不影響。

## 4. 已測試項目

第一版經 code-reviewer 審查後（見第 6 節）修正為兩層 guard，腳本層級用模擬 stdin JSON 手動測試六種分支，行為皆符合預期：

| 情境 | 預期行為 | 結果 |
|---|---|---|
| `stop_hook_active: true` | 直接放行（防迴圈） | exit 0，無輸出 |
| 無 `GTO_*` 環境變數、`stop_hook_active: false` | 直接放行（第一層範圍限制） | exit 0，無輸出 |
| 有 `GTO_*` 環境變數，但 cwd 的 CLAUDE.md/AGENTS.md 沒有 session-boundary 標記 | 直接放行（第二層範圍限制） | exit 0，無輸出 |
| 有 `GTO_*` 環境變數，且 cwd 的 CLAUDE.md 有 session-boundary 標記、`stop_hook_active: false` | 攔截並輸出完整清單（含警語與工作目錄澄清） | exit 0，輸出 `{"decision":"block","reason":"..."}` |
| stdin 不是合法 JSON | 直接放行，不阻擋 Agent | exit 0，無輸出 |

`~/.claude/settings.json` 在編輯後重新驗證仍是合法 JSON。

第二層測試（cwd 檔案讀取）必須用 Windows 原生路徑（例如 `C:\Users\...`）而非 POSIX 風格的 `/tmp/...` 路徑——後者在這個環境下是 Git Bash 的 MSYS 路徑模擬，Windows 版 `node.exe` 的 `fs` 呼叫看不到同一份路徑對應，會誤判成「檔案不存在」。這只是測試腳本本身要注意的環境細節，跟 hook 邏輯無關：Claude Code 實際呼叫 hook 時，`cwd` 一定是原生 Windows 路徑。

## 5. 已知、確定會發生的行為變化（不是待觀察的機率問題）

只要兩層 guard 都通過（GT Office 站台終端機 + 該站台真的開啟此功能），**每一次**該 agent 停止回覆都保證會被攔截一次、強制多生成一輪——`stop_hook_active` 的迴圈防護只保證「這一輪最多只被逼問一次」，不代表「不常發生」。對於跟任務邊界完全無關的短問答（例如使用者只是問一句話），agent 也會多回一句「尚未到達任務邊界，繼續執行」，這是確定的、每輪都有的 2x round-trip 成本，不是機率性的，使用者要知道這是接受這個功能時一併接受的常態代價。若之後覺得成本太高，可考慮限制觸發頻率（例如只在偵測到明顯「產出/決策」訊號的回覆才攔截），但這次沒有實作這個優化。

## 6. Code Review 記錄（2026-09-08）

第一版經 code-reviewer 審查，結論 REQUEST CHANGES，抓到以下問題，已在本次一併修正：

1. **Critical**：第一版只檢查 `GTO_*` 環境變數就攔截，沒有檢查該站台是否真的開啟 `session_boundary_auto_split_enabled`，會導致明確關閉此功能的 agent 也被強制攔截、甚至可能寫出孤兒 handoff 檔案。修法見第 2.2 節第 2 層 guard（改讀 CLAUDE.md/AGENTS.md 裡的既有標記，不新增 repo 端的環境變數）。
2. **Important**：`CHECKLIST_REASON` 漏抄了 `session_boundary_guidance_block()` 裡「只在真的到達邊界、且交接內容已經確定可信賴時才寫出訊號」的警語，以及「工作目錄是 session 啟動時所在的資料夾、不是 repo 最上層目錄」的澄清（後者正是 `docs/cw/18` 記錄過的實際踩坑案例）。兩處都已補回，見腳本內 `CHECKLIST_REASON` 常量。
3. **Important**：本文件第 5 節原本把「每輪延遲/token 成本」寫成待觀察的機率性問題，實際上是確定發生的常態行為，已改寫為第 5 節現在的版本。

## 7. 尚未驗證、需要下次實機補測的項目

- **真正在 GT Office 站台終端機裡，任務跑到邊界時，這條路徑會不會被正確觸發並最終開出新 session**——這需要一個開啟了 `session_boundary_auto_split_enabled` 的 station，讓它跑一個真的會跨越邊界的多階段任務，全程觀察：Stop hook 有沒有攔截、Agent 有沒有依清單判斷並寫出訊號檔、既有的 reconciliation 掃描有沒有偵測到並重啟終端機。
- **`Stop` 事件的 stdin 交付時機（尤其是 `cwd` 欄位）是否跟 `UserPromptSubmit` 一致**——這次的 I/O 假設經過交叉查證但沒有實機呼叫驗證過。
- **與 `enrich_dispatch_markdown`（`crates/gt-task/src/lib.rs`）既有的 `output_collection_enabled` 派工提示是否會有新的措辭衝突**——`docs/cw/19` 修過的是「`.claude/session-handoff` 歸誰管」的衝突，這次新增的 `reason` 文字沒有改到那段派工提示，理論上不衝突，但沒有實測過兩者同時作用的情境。

## 8. 第二輪修正：清單措辭過於保守、多終端機情境未觸發（2026-09-09）

> 現象（使用者在另一個 GT Office 管理的專案裡實機使用後回報，非本 repo）：
> 1. Agent 完成一個大任務裡的一個小任務時，不覺得該開新 session。
> 2. 一段 coding 工作結束時，Agent 沒有判斷要開新 session。
> 3. 多終端機協作時，Agent A 把任務派發給另一個終端機的 Agent B，B 做完被指派的任務後，沒有觸發 hook。

### 8.1 根因分析

**問題 1、2（清單措辭問題，非機制問題）**：hook 本身每輪都有正確攔截、逼 Agent 檢查一次（機制層面沒有問題），但第 1 版清單的邊界條件只列了「下一階段目標不同」「上一階段已達穩定檢查點」這類抽象敘述，且第二輪覆核時（見 8.2）發現第一版清單原本沒有明確收斂的「延續」判準，容易讓 Agent 用「反正下一步跟現在有關聯」這種直覺理由，把任何連續工作都合理化成「不算邊界、繼續」，而不會主動把「完成一個小任務／一段 coding」本身視為值得交接的獨立段落。這不是 Stop hook 沒有觸發，是觸發之後 Agent 自由心證仍然偏向保守——跟第 1 版原始設計文件第 5 節分析的「判斷頻率不確定」是同一類問題，只是這次證實光靠「強制檢查一次」還不夠，清單內容本身的判準力道也要夠。

**問題 3（很可能是配置問題，不是清單措辭問題）**：Stop hook 只要兩層 guard 都通過就會攔截，跟被攔截的這一輪回覆內容是不是在處理派發任務無關——也就是說，只要 B 這個 agent 本身的兩層 guard 都成立，B 做完被派發的任務、準備結束這輪回覆時，hook 必然會攔截一次（跟 A、B 之間有沒有派發關係無關）。派發本身（`task_dispatch_batch` → `write_terminal_with_submit`，見 `apps/desktop-tauri/src-tauri/src/commands/task_center/mod.rs`）是把文字連同送出序列直接寫進 B 的終端機，跟使用者親手輸入、送出走的是同一條路徑，不會讓 B 的 Claude Code CLI 少一次正常的使用者回合／`Stop` 事件。因此「B 做完沒觸發」幾乎可以排除是 hook 機制本身的問題，需要依序檢查 B（**目標** agent，不是發派任務的 A）自己是否滿足以下三個各自獨立、且都是**逐 agent**設定的前提：

0. **B 這個站台底層 provider 是不是 Claude Code，而不是 Codex**——`~/.claude/hooks/session-boundary-checklist.js` 是 Claude Code 專屬的 `Stop` hook 機制，`docs/cw/21` 第 31 行與 `StationCapabilitiesTab.tsx` 都記錄過「Codex 不支援 Hooks/Skills 分頁」「`--setting-sources` 是 Claude CLI 專屬旗標」。如果 B 這個站台掛的是 Codex，不管下面兩項設定是否打開，這整套機制對 B 而言從一開始就不存在，不是「沒觸發」而是「本來就不適用」。多終端機協作時混用 Claude 派工、Codex 執行是實際可能發生的情境，這是**第一個**該排除的可能性，比下面兩項配置檢查更根本。
1. **B 的能力分頁「使用全域 Hook / Skill / 權限設定」主開關是否為開**（`docs/cw/21_全域Hook_Skill開關設計.md`）。這個開關關閉時，B 啟動 Claude CLI 會帶上 `--setting-sources project,local`，此時使用者個人全域的 `~/.claude/settings.json`（包含這個 Stop hook 的註冊）**完全不會被載入**——不是清單判斷錯誤，是 hook 根本沒被掛上去，且第 21 篇文件記錄過這個開關關閉後全域清單會變成「一律唯讀、無法個別保留單一 hook」，沒有部分繞過的方法。
2. **B 自己的「任務邊界自動重啟」（`session_boundary_auto_split_enabled`）checkbox 是否為開**（對應第 2.2 節第 2 層 guard）。這是每個 agent 各自獨立的欄位，只在 A 身上打開、B 身上沒開，B 的 CLAUDE.md/AGENTS.md 就不會有 `SESSION_BOUNDARY_GUIDANCE_START` 標記，hook 對 B 的每一輪 Stop 都會在第二層 guard 直接放行、不攔截——使用者很容易誤以為「我已經在協作的 agent 上開了這個功能」，但實際上只開在發派任務的那個 agent（A）身上，沒有意識到接收任務的 B 也要各自單獨開啟。

這三項都是**逐 agent 的既有設定**（第 0 項甚至是選 provider 這個更上層的決定），不是這次清單修正能處理的範圍——如果使用者的「其他專案」裡 B 這三項有任一項不成立，即使清單措辭改到再清楚，B 也不會被強制檢查。建議下次遇到「派發後沒觸發」時，依序排查這三項，而不是先懷疑清單文字。

### 8.2 清單修正內容（v2）

在完成上面的根因分析後，找了一支獨立、對這次設計完全沒有背景知識的 subagent，把 v1 草案的下一版（v2 draft）連同四個具體情境（多步驟重構做完一步／獨立修好一個小 bug／被別的 Agent 派發任務做完／還在跟使用者討論設計方向）一起交給它，請它代入「收到這份清單的那個 agent」實際套用判斷，並挑清單本身的模糊處。結果：

- 情境 2（獨立小 bug）、情境 3（被派發任務做完）、情境 4（設計討論中）三個都判斷正確、無爭議。
- **情境 1（多步驟重構做完其中一步）判斷錯誤**：subagent 指出 v2 draft 第二步的「延續」例外寫成「下一步跟剛完成的緊密耦合到無法切開的程度」，這句話沒有可操作的判準，會被「反正下一步是同一個重構的延續」這種說法濫用，讓 Agent 把「大任務裡理所當然的下一步」也算進例外，直接架空了「預設視為邊界，除非例外」這個舉證責任反轉的設計意圖——這正好對應使用者回報的問題 1。

依這個回饋收斂例外條件的判準：把「延續」例外收窄成**唯一**一條可操作的判準——現在正在改的是同一個函式／同一個 bug 還沒寫完的半成品，中斷會留下編譯不過或邏輯不完整的狀態；並明講一句反例：「這是大任務裡邏輯上的下一步」本身不算例外，即使技術上相關。同時在 a、b 兩項各補一句提醒，直接對應問題 1、2：任務清單裡的下一項不是不檢查的理由；改動再小、耗時再短，只要已達可驗證的穩定狀態也要檢查。

最終版已同步套用到兩處（措辭逐字對應，僅因為程式碼字串跟 hook 腳本陣列的斷行方式不同才有微小換行差異）：

- `~/.claude/hooks/session-boundary-checklist.js` 的 `CHECKLIST_REASON`（全域環境，不屬於此 repo）。
- `apps/desktop-tauri/src-tauri/src/commands/agent.rs` 的 `session_boundary_guidance_block()`（本 repo，寫入 CLAUDE.md/AGENTS.md 的持久規則）——兩處維持同一套判準文字是既有設計要求（見 §371-377 行程式碼註解），這次一併更新，不只改全域 hook 那邊。

### 8.3 驗證

- `cargo check -p gtoffice-desktop-tauri` — 通過。
- `cargo fmt -p gtoffice-desktop-tauri -- --check` — `agent.rs`（本次修改的檔案）本身乾淨；剩下的格式落差在 `local_bridge.rs`，跟本次改動無關、`docs/cw/19` 已記錄過是既有問題。
- `cargo test -p gtoffice-desktop-tauri --lib session_boundary_guidance_tests` — 因 `src/tests/local_bridge_tests.rs`（`AgentRuntimeRegistration::role_key`、`list_roles`、`seed_agent_defaults` 等符號缺失）既有的、與本次改動無關的編譯失敗，整個 lib test 目標無法編譯；用 `git stash` 只還原 `agent.rs` 重新編譯，同樣的錯誤依然出現，證實是修改前就存在的問題，不在本次範圍內處理。
- `node --check ~/.claude/hooks/session-boundary-checklist.js` — 通過。
- **腳本層級 dry-run**（code-reviewer 建議補上的低成本驗證）：合成 `{"stop_hook_active": false, "cwd": "<臨時目錄>"}` 當 stdin，帶上 `GTO_WORKSPACE_ID`/`GTO_AGENT_ID`/`GTO_STATION_ID` 三個環境變數，並在該臨時目錄放一份含 `SESSION_BOUNDARY_GUIDANCE_MARKER` 的 `CLAUDE.md`，實際跑一次 `node session-boundary-checklist.js`，確認輸出是合法 JSON、`decision` 為 `"block"`、`reason` 裡完整含有 `restart_in_place` 訊號、「第一步」「第二步」「唯一例外」等新版關鍵措辭——確認兩層 guard 通過時的攔截路徑本身沒有壞掉。過程中踩到一個純屬測試工具的坑：用 PowerShell `[System.IO.File]::WriteAllText` 搭配 `Encoding]::UTF8` 寫 stdin 檔案預設會帶 BOM，Node 的 `JSON.parse` 遇到開頭 BOM 直接拋例外、被腳本的 catch 吞掉靜默放行（誤以為 guard 沒過），改用不帶 BOM 的 `UTF8Encoding($false)` 才解決——這純粹是這次 dry-run 腳本本身的編碼細節，跟 hook 邏輯無關，記錄下來避免以後重工。
- ⚠️ 未做：實機驗證新版清單真的能讓 Agent 在問題 1/2 的情境下正確判斷（dry-run 只能確認攔截路徑的 JSON 輸出正確，不能驗證 Agent 讀到這段文字後實際的判斷行為）；問題 3 的三項前提（provider、全域開關、`session_boundary_auto_split_enabled`）也未在使用者「其他專案」裡實際確認，需要使用者自行檢查。
