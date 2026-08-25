# 14 Agent 輸出清單化：畫布輸出清單節點設計

> 需求原話：「若 agent 執行的結果會產出 md 檔或網頁等說明用的文件，請在 agent canvas 的 agent 節點 output 自動接上一個清單節點，選擇清單上的某一檔案可以直接閱覽內容，若是網頁則是直接打開瀏覽器。」
>
> 狀態（2026-08-25）：**P1-P4 皆已完成**（程式碼已寫完，過 `cargo test`/`cargo clippy`/`npm run typecheck`，尚未 commit、尚未真人操作驗收）。P4 原本因為牽涉「新建 agent 時 CLAUDE.md/AGENTS.md 產生邏輯」這個不同子系統而先跳過，後來使用者實測發現 agent 把產出檔案寫到 workspace 之外（桌面），確認 P4 這道引導確實必要，回頭補做——實作方式與範圍見第 6 節 P4 小節。

## 1. 現況地基（本次調查結果）

### 1.1 Agent Canvas 已有的「掛載清單節點」先例，可以直接延伸

`apps/desktop-web/src/features/agent-canvas/model/agent-canvas-graph.ts` 已經有三種「一個 agent 一個摺疊清單節點」的掛載節點模式（MCP 例外，是一個 server 一個節點；Skill/Hook 是整個 agent 一個節點）：

- `AgentCanvasMcpNodeData` / `AgentCanvasSkillNodeData` / `AgentCanvasHookNodeData`
- 共同規則：只有「該 agent 至少有一項掛載」才會出現節點（presence-based，見 `skillMountId`/`hookMountId` 的 `null` 語意）
- 節點固定尺寸 `SKILL_NODE_WIDTH/HEIGHT = 160×48`、`HOOK_NODE_WIDTH/HEIGHT` 同規格
- 節點位置是 client-only、可拖曳、可設定顏色（`skillNodePositions`/`hookNodePositions`/`mountNodeColors`，存在 `useAgentCanvasData` 的 localStorage bucket）
- 渲染上是「摺疊 header + 展開列出清單項目」的卡片（`AgentCanvasNodeCard.tsx` 的 `AgentCanvasSkillNodeCard`/`AgentCanvasHookNodeCard`），可直接照抄這個 UI 骨架
- 但這三種都是 agent 的**輸入端**（掛載的能力，接在節點左側 input port stack，見 `computeAgentInputPortLayout`）——新的「輸出清單節點」語意相反，必須接在**右側 output 端**，見第 4.2 節。

`docs/cw/11_Skill掛載清單化.md`、`docs/cw/12_Hook掛載清單化.md` 是這個模式從「手動輸入」演化成「掃描 + 勾選清單」的先例，本次的「掃描 workspace 上的輸出檔案 + 唯讀清單」精神上更接近唯讀掃描的部分（`list_available_hooks`），不需要勾選/掛載的寫入邏輯。

### 1.2 沒有「Agent 產出了什麼檔案」的資料層——需要新建

調查確認以下皆不存在：

- `crates/gt-task`：`TaskDispatchTargetResult`/`TaskDispatchResult` 只描述「派發任務」成功/失敗，不是「產出結果」
- `crates/gt-agent/src/models.rs`：完全沒有 result/output/artifact 相關欄位
- `apps/desktop-web/src/features/task-center/task-center-model.ts` 的 `TaskDispatchRecord`：只有派發面的欄位（`taskFilePath` 是任務指令檔路徑，不是產出）
- Change Feed（`crates/gt-changefeed`）：只比對 git 狀態快照，`FilesChanged` 只有 `staged/unstaged/untracked` 的**數量**，沒有實際檔案路徑，也無法歸屬到特定 agent（一個 workspace 可能有多個 agent 共用）

`crates/gt-agent/src/capability/materialize.rs` 已有的目錄慣例：`<workspace_root>/.gtoffice/agents/<agent_id>/runtime/`（agent 的 capability materialize 輸出）。這是本次輸出目錄設計的直接參照點，見第 3.1 節。

### 1.3 內容檢視元件現況

- Markdown：`apps/desktop-web/src/components/editor/MarkdownRenderer.tsx`（ReactMarkdown + remark-gfm + rehype-highlight，已處理相對路徑資源解析）已成熟可用，`file-explorer` 的 `FileEditorPane.tsx` 已在用它——**可直接複用**，不需要新寫 markdown 渲染邏輯。
- 網頁（HTML）：應用內完全沒有瀏覽器/iframe 預覽元件。

## 2. 需求拆解

1. **偵測**：agent 執行完後，怎麼知道它「產出」了哪些 md/網頁檔案？
2. **呈現**：agent 節點的 output 端自動接一個清單節點，列出偵測到的檔案。
3. **互動**：點清單裡的項目——md 直接在 App 內看內容；網頁直接開瀏覽器。

## 3. 已拍板決策（2026-08-25）

### 3.1 偵測方式：固定輸出目錄慣例

每個 agent 有一個固定路徑：

```
<workspace_root>/.gtoffice/agents/<agent_id>/outputs/
```

與現有 `.../runtime/`（materialize 用）是同一個 `.gtoffice/agents/<agent_id>/` 底下的兄弟目錄，慣例一致、不衝突。畫布唯讀掃描這個目錄（遞迴或僅第一層，實作階段再定），依副檔名分類：

- `.md` / `.markdown` → `kind: 'markdown'`
- `.html` / `.htm` → `kind: 'webpage'`
- 其餘副檔名 → `kind: 'other'`（先掃到、清單裡列出，點擊一律交給系統預設程式開啟，不特別處理）

不新建資料庫表——跟 `list_available_hooks`/`list_available_skills` 一樣是「唯讀掃描檔案系統、每次讀即時反映」，沒有持久化的必要，也不會有「清單跟磁碟不同步」的問題。

**這是一個慣例，不是自動的**：v1 不強迫、不攔截 agent 的實際輸出行為，只掃描這個目錄。若 agent 沒有把產出檔案放進這個目錄，清單就是空的，節點也就不會出現（presence-based，同 Skill/Hook 規則）。要讓這個慣例生效，需要在 agent 的 prompt/policy 材料裡加入「產出文件請放進 `.gtoffice/agents/<agent_id>/outputs/`」的引導——這件事本次不做，留給下一輪任務派發相關的工作（`docs/WORKFLOWS.md`/task-center 派發流程）處理，本文件只定資料層跟畫布呈現。

被否決的兩個方案，記錄理由方便之後回頭查：

- **擴充 Change Feed 帶實際檔案路徑**：可以更「自動」（不需要 agent 配合放檔案），但一個 workspace 可能有多個 agent 同時活動，git diff 無法準確歸屬是哪個 agent 產出的，只能用「最近有派發任務的 agent」做啟發式猜測，準確度不夠、且會把「輸出清單」的正確性建立在一個本質上是猜測的機制上。
- **任務結果顯式回報**：準確度最高，但需要修改 `gto`/task-dispatch 協議讓 agent 明確回報 `outputFiles`，工程量大、牽涉 agent 端行為，不是這次的最小切片。

### 3.2 開啟網頁：系統預設瀏覽器

點擊 `kind: 'webpage'` 的項目 → 呼叫 Tauri 的 `shell.open`（或對應的 `opener` plugin）叫出使用者的系統預設瀏覽器開啟該檔案的本機路徑，**不在 App 內建瀏覽器/iframe 預覽面板**。理由：地基完全沒有（現況調查確認），符合原話「直接打開瀏覽器」的字面意思，工程量最小。

`kind: 'other'` 一併用同一個 `shell.open` 呼叫（開啟方式泛化成「非 markdown 一律交給系統」），不用為每種副檔名各寫一套判斷。

### 3.3 本次範圍：只出設計文件

本文件定案後即結束這輪對話的產出。下一輪對話從第 6 節路線圖的 P1 開始動工，實作前需再讀一次本文件對照當時程式碼是否有漂移。

## 4. 資料模型設計

### 4.1 後端（Rust）

新 Tauri command（暫名，實作時再確認實際命名慣例，比照 `agent_capability_list_available_hooks` 的唯讀掃描 command 風格）：

```rust
// crates/gt-agent/src/capability/discovery.rs（或新檔案，視實作時份量決定）
pub struct AgentOutputFile {
    pub id: String,           // 穩定 id，可用相對路徑本身或其 hash
    pub file_name: String,
    pub relative_path: String,  // 相對於 outputs/ 目錄
    pub kind: AgentOutputKind,   // Markdown | Webpage | Other
    pub modified_at_ms: i64,
    pub size_bytes: u64,
}

pub fn list_agent_output_files(
    workspace_root: &Path,
    agent_id: &str,
) -> Result<Vec<AgentOutputFile>, ...>
```

- 目錄不存在 → 回傳空陣列，不是錯誤（同 `list_available_hooks` 對 `.claude/settings.json` 不存在的處理方式）
- 路徑驗證：讀取範圍必須鎖死在 `<workspace_root>/.gtoffice/agents/<agent_id>/outputs/` 內，不得跳脫（`CLAUDE.md`「自訂 cwd 必須位於 workspace 內」同精神），實作時對照現有 file-explorer 讀取 command 用的邊界檢查方式
- `apps/desktop-tauri/src-tauri/src/commands/agent/`（或 `agent_canvas/`，需對齊 01 章「feature 與 commands 一比一對齊」規則，兩者哪個更合適留給實作時判斷）新增對應的 command 入口
- 開啟檔案（markdown 讀內容 / webpage 交給系統開啟）分別對應既有的「讀檔」command 與新的「開啟系統預設程式」command（後者若 Tauri opener plugin 已在專案內就直接用，不重複造輪子——需要在實作前確認 `apps/desktop-tauri/src-tauri/Cargo.toml` 是否已有這個 plugin）

### 4.2 前端節點模型（`agent-canvas-graph.ts`）

新增第四種 mount node kind：

```ts
export interface AgentCanvasOutputNodeData {
  kind: 'output'
  id: string
  agentId: string
  files: AgentOutputFile[]
  color?: string | null
}
```

- `buildOutputNodeId(agentId)`，比照 `buildSkillNodeId`/`buildHookNodeId`
- `OUTPUT_NODE_WIDTH/HEIGHT`：沿用 `160×48`（跟 Skill/Hook 同規格，畫布左右兩側的掛載節點外觀一致）
- **關鍵差異**：Skill/Hook/MCP 三者是輸入端，接在 agent 節點左側、fan 進 `computeAgentInputPortLayout` 的組合堆疊；Output 節點語意是「這個 agent 產出了什麼」，必須接在 agent 節點**右側**，跟 `outputLinkIds`（agent 之間手畫連線的輸出埠）用同一側，但不共用同一個堆疊——需要一個新的 `computeAgentOutputPortLayout`（或擴充現有函式接受 side 參數），把 `outputLinkIds` 的 port slot 跟 `outputMountId` 的 port slot 分開計算，避免跟既有 agent-to-agent 連線的手畫連線槽位混在一起
- `AgentCanvasAgentNodeData` 新增欄位 `outputMountId: string | null`，presence 規則同 `skillMountId`（只有主要 instance、且該 agent 掃到至少一個輸出檔案時才非 null）
- 新 edge kind：`{ kind: 'output-mount'; agentId: string; mountId: string }`，方向是 agent → output 節點（跟 `mcp-mount`/`skill-mount`/`hook-mount` 的方向相反，那三者是「掛載節點 → agent」），渲染時可能需要視覺上跟輸入方向的掛載線區分（例如箭頭方向或顏色），具體樣式留給實作階段的 `AgentCanvasPane.tsx renderEdge`

### 4.3 `useAgentCanvasData.ts`

- 新增 `outputFilesByAgentId: Record<string, AgentOutputFile[]>`，`reload()` 併發呼叫新 command（比照現有 `capabilityByAgentId` 的 best-effort per-agent 讀取模式，一個 agent 讀失敗不影響其他 agent）
- 新增 `outputNodePositions`（client-only 拖曳位置）+ `commitOutputNodePosition`，比照 `mcpNodePositions`/`commitMcpNodePosition` 的 localStorage bucket 模式（新 prefix，例如 `agent-canvas.outputPositions`）
- 輪詢頻率沿用現有 `POLL_INTERVAL_MS = 8000`，不需要另開一條輪詢——跟其他 capability 資料一起在同一個 `reload()` 週期更新即可

### 4.4 UI（`AgentCanvasNodeCard.tsx`）

新增 `AgentCanvasOutputNodeCard`，摺疊 header 顯示「輸出（N）」，展開列出每個檔案：檔名 + 依 `kind` 給不同 icon。點擊項目行為：

- `kind === 'markdown'`：讀取檔案內容 → 用既有 `MarkdownRenderer` 顯示。呈現位置建議做成一個輕量彈出面板（例如 `AgentCanvasOutputPreviewModal.tsx`），不佔用畫布空間；是否要複用 Inspector 側欄留給實作階段依實際畫面配置決定
- `kind === 'webpage'` / `kind === 'other'`：呼叫系統開啟 command（見 4.1），不在 App 內處理

## 5. 涉及檔案清單（實作階段預覽，非最終定案）

後端：
- `crates/gt-agent/src/capability/discovery.rs`（或新檔案）：`AgentOutputFile`/`AgentOutputKind`/`list_agent_output_files`
- `apps/desktop-tauri/src-tauri/src/commands/agent/`（或 `agent_canvas/`）：新 command 入口（掃描 + 開啟系統程式）
- `apps/desktop-tauri/src-tauri/src/lib.rs`：註冊新 command

前端：
- `apps/desktop-web/src/shell/integration/desktop-api.ts`：新型別 + wrapper
- `apps/desktop-web/src/features/agent-canvas/model/agent-canvas-graph.ts`：`AgentCanvasOutputNodeData`、`buildOutputNodeId`、`outputMountId`、`output-mount` edge、輸出側 port layout
- `apps/desktop-web/src/features/agent-canvas/controllers/useAgentCanvasData.ts`：`outputFilesByAgentId`、`outputNodePositions`、`commitOutputNodePosition`
- `apps/desktop-web/src/features/agent-canvas/components/AgentCanvasNodeCard.tsx`：`AgentCanvasOutputNodeCard`
- 新增 `AgentCanvasOutputPreviewModal.tsx`（或等效元件）：markdown 內容檢視
- `apps/desktop-web/src/features/agent-canvas/AgentCanvasPane.tsx`：`renderEdge` 補 `output-mount` 分支
- `apps/desktop-web/src/shell/i18n/messages.ts`：新增文案（清單節點 header、空狀態等）

### 5.1 實際完成後的差異（P1-P4 都做完，補記於此，不回頭改上面的實作前預覽）

- Markdown 內容檢視沒有另外新增 `AgentCanvasOutputPreviewModal.tsx`，直接在 `AgentCanvasPane.tsx` 內用既有的 `createPortal` + `MarkdownRenderer`（`apps/desktop-web/src/components/editor/MarkdownRenderer.tsx`）組出彈出面板，未另立元件檔
- 「開啟系統預設程式」沒有新增 Tauri plugin——`fs_show_in_folder`（`apps/desktop-tauri/src-tauri/src/commands/file_explorer/mod.rs`）已經在用 `open` crate（`open = "5.3"`，`apps/desktop-tauri/src-tauri/Cargo.toml`），新 command `agent_capability_open_output_file` 直接複用同一顆 crate，沒有新增依賴
- 「開啟檔案」command 沒有放進 `agent_canvas` 模組，而是跟 P1 的 `agent_capability_list_output_files` 放在同一個檔案 `apps/desktop-tauri/src-tauri/src/commands/agent/capability.rs`（唯讀掃描 agent 自己的檔案，跟 skill/hook discovery 同一類，不是畫布視覺狀態）
- P4（本文件狀態列已更新）額外新增/修改：`crates/gt-agent/src/models.rs`（`default_output_guidance_prompt_content`）、`apps/desktop-tauri/src-tauri/src/commands/agent.rs`（`agent_create_with_repo` 掛勾點）

## 6. 路線圖

| 階段 | 內容 | 驗證方式 |
|---|---|---|
| P1 | 後端唯讀掃描：`list_agent_output_files` + command + 型別，不接 UI | `cargo test`（含目錄不存在/空目錄/混合副檔名的單元測試）、`cargo clippy`、手動在某 agent 的 `outputs/` 目錄放測試檔案驗證掃描結果 |
| P2 | 畫布節點模型 + 摺疊清單 UI（先不做點擊互動，只驗證節點出現/消失、展開/收合、port 位置正確） | `npm run typecheck`、`npm run build:tauri`、手動驗證畫布渲染 |
| P3 | 點擊互動：markdown 內容檢視、webpage/other 系統開啟 | 手動驗證：放一份 md、一份 html，分別點擊確認行為符合預期 |
| P4 | Agent prompt 引導「產出請放進 outputs/」——實測發現 P1-P3 完成後，agent 產出的檔案預設不會落在這個目錄（範例：一次真實測試裡 agent 把 HTML 寫到了桌面，完全在 workspace 之外），確認這道引導必要。實作範圍刻意收窄：只在**建立新 agent、且使用者完全沒填寫 prompt 內容（沒手動輸入、也沒載入外部範本）**時，`crates/gt-agent/src/models.rs` 的 `default_output_guidance_prompt_content(agent_id)` 才會被寫進 CLAUDE.md/AGENTS.md，內容是一段告知 `.gtoffice/agents/<agent_id>/outputs/` 用途的短說明；使用者只要填了任何 prompt 內容或載入外部範本，完全不受影響。掛勾點在 `apps/desktop-tauri/src-tauri/src/commands/agent.rs` 的 `agent_create_with_repo`，只加在 `repo.create_agent()` 之後、`write_prompt_file()` 之前一段條件判斷，不改動任何既有 agent 的 prompt、也不影響「編輯 agent」流程 | `cargo test -p gt-agent models`、`cargo check --workspace`、`cargo clippy -p gt-agent`/`-p gtoffice-desktop-tauri --lib` 皆過；`gtoffice-desktop-tauri --lib` 的完整測試套件目前因既有、無關的 roles 功能未完成（`local_bridge_tests.rs` 缺 `role_key`/`list_roles`/`seed_agent_defaults`，docs/cw/12 已記錄過同一問題）而整體編譯失敗，`agent_tests.rs` 內既有的 create-agent 測試無法用 `cargo test` 實際跑過——改以人工檢查：目前唯三個 `prompt_enabled = Some(true)` 的既有測試都同時給了非空 `prompt_content`，不會觸發新的引導分支，理論上不受影響，但這點仍待這個既有阻塞問題解掉後、或真人在畫面上建立一個「不填 prompt」的新 agent 實際驗證 |

## 7. 手動驗收清單（真人在畫面上操作）

程式碼尚未 commit，以下是給人在實際跑起來的 App 裡驗證整條流程用的步驟。**舊 agent 不會自動獲得 P4 的引導**（見狀態列說明）——要驗證完整流程必須用「新建」的 agent，不能拿既有 agent 測。

### 7.1 準備：重新編譯啟動 App

這些改動都是後端 Rust + 前端 TS，需要重跑一次 `npm run tauri dev`（或對應的開發啟動指令）讓新程式碼生效，單純重整網頁不會套用 Rust 端的變更。

### 7.2 驗證 P4：新 agent 是否拿到引導文字

1. 在某個 workspace 裡新建一個 agent，**建立當下不要輸入任何 prompt 內容**（留空，也不要用「從外部路徑載入範本」）
2. 建立完成後，打開這個 agent 的 prompt 編輯畫面（或直接去檔案系統看 `<workspace_root>/CLAUDE.md`／`<workspace_root>/AGENTS.md`，實際檔名依 agent 的 tool 而定，工作目錄若不是 `.` 則在對應子目錄下）
3. 確認內容包含 `.gtoffice/agents/<這個新 agent 的 id>/outputs/` 這一段路徑（agent id 可以在 agent 管理畫面或 URL/資料上看到）
4. 對照組：另外用同樣方式建一個 agent，但這次**手動輸入任意 prompt 內容**，確認它的 prompt 檔案「沒有」被加上這段引導文字（沒被動過）

### 7.3 驗證 P1-P3：畫布輸出清單節點是否正確出現與運作

有兩種驗證路徑，選一種即可：

**A. 端到端（最貼近真實情境）**：用 7.2 建立的、拿到引導的那個新 agent，實際派發一個任務，要求它產出一份說明文件（例如「寫一份這次分析的 markdown 報告」）。因為它的 prompt 已經知道 `.gtoffice/agents/<agent_id>/outputs/` 這個慣例，正常情況下它應該會把檔案寫進那裡。完成後回到 Agent Canvas，等一次輪詢（約 8 秒）或切換分頁再切回來，確認該 agent 節點右側出現「輸出」清單節點。

**B. 手動放測試檔（排除 agent 是否真的遵守慣例的變數，單獨驗證畫布機制）**：
1. 直接在 `<workspace_root>/.gtoffice/agents/<某 agent_id>/outputs/` 這個目錄手動建立測試檔案（目錄不存在就自己建），至少放一份 `.md` 和一份 `.html`
2. 回到 Agent Canvas，等輪詢或重新整理，確認該 agent 節點右側出現「輸出」清單節點（虛線框、顯示檔案數量）

不論走哪條路徑，接著驗證：

- 點清單節點的摺疊 header，確認展開後列出剛剛放的檔案，檔名前有對應圖示（md 一種、html 另一種）
- 點清單裡的 `.md` 檔案項目，確認彈出一個內嵌預覽視窗，正確渲染 markdown 內容；點右上角關閉按鈕可以關掉
- 點清單裡的 `.html` 檔案項目，確認**沒有**跳出 App 內視窗，而是叫出系統預設瀏覽器把該檔案打開
- 右鍵點輸出清單節點，確認會跳出跟 MCP/Skill/Hook 節點一樣的顏色選單，選一個顏色確認節點虛線框變色
- 拖曳輸出清單節點，確認可以自由移動位置；重新整理頁面後位置有記住（client-only 儲存）
- 把 `outputs/` 目錄裡的檔案全部刪掉，等下一次輪詢，確認輸出清單節點從畫布上消失（presence-based，沒有檔案就不畫）

每階段結束都要能過 `npm run typecheck` + `cargo check --workspace`，不做「看起來對」就宣布完成。
