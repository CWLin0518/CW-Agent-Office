# 08 Agent 能力掛載設計：按 Agent 各自配置 MCP / Tool / Hook / Skill

> 現況核對（2026-10-08）：本文件保留歷史紀錄，當前待辦以 [TODO](../TODO.md) 為準。能力掛載與 Canvas 已提交，不再是「待開始／尚未 commit」。Codex 已支援 Skills/Hooks；舊操作清單需對照現行實作，驗收完成須有紀錄。

> 狀態（2026-10-08）：能力掛載已實作；本輪同步現行契約。下列流程描述 GT Office 的生成／啟動行為，真實 CLI 的 MCP 呼叫與 Skills/Hooks 觸發仍需端到端驗收。
>
> 定位：延續 [04_客製化設計.md](04_客製化設計.md) 第 3 節「新增 Agent 的權限分頁」——那份文件已經把 17 類權限分成 Phase A（已有掛勾點）跟 Phase B（需要先建子系統），其中「工具/MCP」被列進 Phase B，但**沒有展開細節**；Hook、Skill 這兩個概念完全沒被 17 類清單涵蓋。這份文件把「工具/MCP」的 Phase B 展開，並補上 Hook / Skill。

---

## 現行實作契約（2026-10-08）

- 共用快照是 `AgentCapabilitySnapshot`，儲存在 `agent_capability_snapshots.capability_json`。Claude、Codex 都接受 MCP／Skills／command Hooks；Hook 保存走 preview → content-hash confirm → save → audit。
- Claude：materialize 至 workspace 的 `.gtoffice/agents/<id>/runtime/`；MCP/Hook 以 `--mcp-config`／`--settings` 疊加。Skills 走支援旗標或 agent workdir 的 `.claude/skills` 複製 fallback，manifest 管理自有檔案。
- Codex：在原 Codex home 寫入 `gtoffice-<slug>-<hash>.config.toml`，以 `codex -p <profile>` 啟動；hash 納入 workspace 與 agent 身分。MCP 生成 TOML server 設定，Skills 生成 `skills.config` 的 path/enabled，Hooks 生成 events/matcher/command 表。不複製 base config、不另設隔離 CODEX_HOME，也不宣稱這份 overlay 隔離了使用者全部全域設定。
- 首次啟動與 resume/relaunch 共用 `materialize_capability_for_launch`，產物為 `MaterializedCapability::{Claude,Codex}`。`resume.rs` 仍回傳要輸入 PTY 的命令字串，未改為 argv；Codex `-p` 必須位於 resume/fork 子命令之前。
- 「使用全域 Hook/Skill/權限設定」僅適用 Claude。關閉後加 `--setting-sources project,local`；全域清單永遠唯讀。Codex 沒有此來源排除語意，UI 不顯示該主開關或 Claude 的全域清單；既有快照布林欄位仍保留但不影響 Codex 啟動。
- 快取：Claude hash 包含 snapshot、support 與 agent_workdir；Codex 以 namespaced profile 名作併發鎖域。首次啟動與恢復都從快照重算。materialize 錯誤目前採 warning + 未掛載啟動，不保證保存成功就一定掛載成功。
- 目前 Hook 僅支援 command，不支援 prompt／timeout 欄位；provider 版本能力與真實觸發請以端到端驗收為準。

以下保留地基與分階段說明；與本節不一致的舊決策編號只代表歷史背景。

## 0. 先講一個容易混淆的區分：這不是「權限」，是「掛載」

04 文件的 `AgentPolicy` / `policy_snapshot_id` 設計，回答的是「Agent 能不能做 X」（allow / deny / require-approval），適合「這個 agent 可不可以呼叫某個 MCP 工具」這種是非題。

但「這個 agent 要掛哪個 MCP server、哪些 skill 檔案、哪些 hook 腳本」是**「配置什麼東西存在」**的問題，不是是非題。所以這份設計提出一個獨立於 `AgentPolicy` 的新概念：`AgentCapabilityProfile`（掛載了什麼），而政策系統可以疊加在它之上（例如：掛了 MCP-X，但政策規定呼叫 MCP-X 的某個工具需要人工核准）。

**順序關係**：沒有先「掛載」，04 文件 Phase B 的「工具/MCP」政策面也沒東西可以勾選/管制。所以這份設計是 04 文件 Phase A → Phase B 之間的銜接步驟，建議命名為 **P3.5-capability**（避免跟 04 文件已用掉的 P4.5 撞名）。

---

## 1. 現況地基

- `AgentProfile`（`crates/gt-agent/src/models.rs`）已有 `tool`（provider）、`workdir`、`launch_command: Option<String>`（本來就可覆寫）、`policy_snapshot_id`（已用於不可變政策快照）。
- 實際「組出啟動指令、寫進 PTY」的唯一進場口是 `crates/gt-agent-session/src/resume.rs` 的 `ResumeService::build_relaunch_launch_command` / `build_resume_launch_command`——目前回傳單純字串，例如 `"claude --resume xxx"`、`"codex resume --last"`。這是本設計唯一需要插旗標的地方。
- Claude Code CLI 原生支援專案層級 `.mcp.json`、`.claude/settings.json`（含 hooks）、`.claude/skills/`，但這些是**依 `cwd` 讀取**——同一個 workspace 內如果多個 agent 共用同一個 cwd，會互相污染彼此的掛載內容，這正是「按 agent 各自掛載」目前做不到的根本原因。
- Claude Code CLI 存在 `--mcp-config <file>`（疊加額外 MCP server）、`--settings <file>`（疊加額外 settings，含 hooks）、`CLAUDE_CONFIG_DIR`（改變使用者層級設定根目錄）這類機制，可以在啟動時做「疊加」而不用動使用者專案本身的設定檔——但這些旗標的確切行為要用**目前釘選的 Claude Code CLI 版本**重新核對一次，不要憑舊版記憶假設。
- Codex CLI 這邊（`~/.codex/config.toml` 的 `mcp_servers`、`CODEX_HOME` 環境變數）目前調研不足，不確定是否有對稱的疊加機制。
- `crates/gt-terminal/src/lib.rs`（約 1532-1593 行）建立 PTY session 時已經支援呼叫端透過 `TerminalCreateRequest::env` 注入自訂環境變數（先 `env_clear()` 再套白名單，最後套呼叫端傳入的 `request.env`）——這代表「每個 agent session 給不同的 `CLAUDE_CONFIG_DIR` / `CODEX_HOME`」這件事，PTY 層的能力已經現成可用，不需要新增，只需要在 materialize 完成後把路徑塞進這個既有的 `env` map。

---

## 2. 設計

### 2.1 資料模型：`AgentCapabilityProfile`

新表 `agent_capability_snapshots`（比照 `agent_policy_snapshots` 的不可變快照精神，每次修改產生新快照，方便稽核）：

```sql
CREATE TABLE agent_capability_snapshots (
  id             TEXT PRIMARY KEY,
  workspace_id   TEXT NOT NULL,
  agent_id       TEXT NOT NULL,
  capability_json TEXT NOT NULL,
  created_at_ms  INTEGER NOT NULL
);
```

`AgentProfile` 新增：

```rust
pub capability_snapshot_id: Option<String>,
```

用 `ADD COLUMN` 手法新增（比照 `crates/gt-storage/src/ai_config_repository.rs:132` 的寫法，不要照抄 `agent_repository.rs` 69-86 行那段整表重建——那是「移除欄位」的情境，新增可空欄位不需要那麼複雜）。

`capability_json` 的形狀（provider-agnostic 中介層，materialize 時才轉成 provider 專屬格式）：

```json
{
  "mcpServers": [
    { "id": "...", "transport": "stdio|sse|http", "command": "...", "args": [], "env": {} }
  ],
  "skills": [
    { "id": "...", "sourcePath": "...", "enabled": true }
  ],
  "hooks": [
    { "event": "PreToolUse|PostToolUse|...", "matcher": "...", "command": "..." }
  ]
}
```

用 JSON blob 而不是正規化多張表：這三種東西的欄位形狀會跟著 Claude Code / Codex 版本演進（尤其 hooks 的 event 類型清單），比照 04 文件對「AI 設定變更」的態度——不追著上游 schema 頻繁 migration，存 JSON、用 serde 結構做「讀時驗證」即可。

**Provider 支援範圍**：Claude 與 Codex 均接受 MCP、Skills、Hooks。Codex materialize 生成 profile TOML 的對應欄位；支援快照與生成檔案不等於已完成實際 CLI 呼叫／觸發驗收。全域設定來源排除只支援 Claude。

### 2.2 Provider 能力探測：`ProviderCapabilitySupport`

在 materialize 之前，需要知道「目前釘選的這支 CLI 二進位，到底支援哪些疊加旗標」——不能寫死假設。擴充 `crates/gt-tools/src/agent_installer.rs`（04 文件第 5 節已經點名這支檔案之後要拆成 Provider Descriptor，這裡先用小改動不等大重構）：

```rust
pub struct ProviderCapabilitySupport {
    pub supports_mcp_config_flag: bool,     // --mcp-config
    pub supports_settings_flag: bool,       // --settings（承載 hooks）
    pub supports_skills_dir_flag: bool,     // 是否有「疊加一個額外 skills 目錄」的旗標
}

impl AgentInstaller {
    pub fn detect_capability_support(agent: AgentType) -> ProviderCapabilitySupport { ... }
}
```

探測方式：對已安裝的二進位跑 `<cli> --help`（或已知子指令的 `--help`），比對輸出裡有沒有目標旗標字串；探測結果快取在 `.gtoffice/cache/provider-capability.json`，跟安裝/升級動作一起失效重探（避免每次啟動 agent 都重新跑一次 `--help`）。這一步是 2.3 節「Claude 旗標優先、複製 fallback」（決策 1）能夠真的落地判斷的依據，不是可有可無的裝飾。

### 2.3 Materialize：啟動前把 profile 轉成 provider 看得懂的檔案

新模組建議放 `crates/gt-agent` 底下，跟未來的 policy 模組同級（例如 `crates/gt-agent/src/capability.rs`，規模大了再考慮獨立成 crate）：

```rust
fn materialize(
    workspace_id: &str,
    agent_id: &str,
    snapshot: &AgentCapabilitySnapshot,
    support: &ProviderCapabilitySupport,
) -> MaterializedPaths
```

輸出到 `.gtoffice/agents/<agent_id>/runtime/`（workspace 內、但獨立於 agent 自己的 workdir，不跟使用者檔案混在一起，也不進 git）：

- `mcp.json` — Claude Code `--mcp-config` 格式；Codex 場景輸出對應的 TOML `mcp_servers` 片段
- `settings.json` — 只放 `hooks` 欄位（`--settings` 是疊加合併、不是整份取代，所以只需要這一份 agent 專屬的疊加層）
- `skills/<skill-id>/SKILL.md` — 從 `sourcePath` 複製或依內容寫入，`support.supports_skills_dir_flag` 為 false 時才會用到（見 2.4 節）

### 2.4 注入點：擴充 `resume.rs` 的命令組裝

`build_relaunch_launch_command` / `build_resume_launch_command` 目前直接回傳拼好的字串。設計上：

- 兩個函式接受 `Option<&MaterializedCapability>`，回傳型別維持單一命令字串，對齊既有 PTY 輸入契約；首次啟動也透過 `apply_capability_overlay_to_command` 套用相同產物。

**Claude（決策 1：旗標優先、複製 fallback）**：
1. 讀 2.2 節探測到的 `ProviderCapabilitySupport`
2. MCP / hooks：`supports_mcp_config_flag` / `supports_settings_flag` 為 true 時，附加 `--mcp-config <path>/mcp.json`、`--settings <path>/settings.json`；為 false 時記一筆警告並跳過（這兩個功能沒有安全的檔案複製 fallback，跳過比硬套一個可能不相容的旗標安全）
3. Skills：`supports_skills_dir_flag` 為 true 時比照 MCP 用旗標疊加；為 false 時 fallback 成「啟動前把 profile 的 skills **複製**進 agent 自己 workdir 的 `.claude/skills/`」——複製前先做差異比對，只寫入/覆蓋 GT Office 自己管理的 skill id（用檔名前綴或一份 manifest 記錄哪些檔案是本機制寫的），不覆蓋使用者原本手動放在該目錄下的其他 skill

**Codex（現行）**：materialize 寫入原 Codex home 的 namespaced profile 檔案，命令插入 `-p <profile_name>`，在 resume/fork 子命令之前。MCP、Skills、Hooks 都生成至同一份 TOML，不使用原隔離 CODEX_HOME 方案；全域來源排除主開關只作用 Claude。

### 2.5 UI

比照 04 文件第 3 節「權限」分頁的做法，在 `StationManageModal.tsx` 新增一個相鄰分頁（例如「能力 / Capabilities」）：

- **MCP servers**：下拉選一個已在 Settings 全域註冊過的 server，或手動填 stdio 指令；Claude / Codex 都可用
- **Skills**：從本機路徑匯入一個 `SKILL.md`，或勾選 workspace 內已存在的技能；Claude 與 Codex 都可使用
- **Hooks（決策 3：開放自訂指令 + 完整 preview UI）**：使用者可以自己填 hook 指令（不限預設範本），但送出前必須經過完整 preview 畫面——顯示「這個 hook 會在什麼事件（PreToolUse/PostToolUse/…）、比對什麼 matcher 時，實際執行哪一行指令」，逐條列出而不是丟一整包 JSON 給使用者自己看。Claude 與 Codex 都可使用此分頁

### 2.6 快取策略：多畫布節點共用同一 agent（決策 4）

04 文件 P4.6 提到「同一 agent 可有多個畫布節點」，代表同一個 `agent_id` 可能同時有多個 PTY session 各自要 materialize。設計：

- **content-hash 判斷**：`materialize()` 執行前，先對 `capability_json + support（探測結果）+ provider` 算一個 hash，跟 `.gtoffice/agents/<agent_id>/runtime/.capability-hash` 裡記錄的上次 hash 比對——相同就跳過整個 materialize（不重寫檔案、不重新複製 skills），只回傳既有的 `MaterializedPaths`
- **併發安全**：多個 session 幾乎同時啟動同一個 agent 時，materialize 用「寫暫存檔 + rename」的方式落地（避免另一個併發呼叫讀到寫一半的檔案），並在 `.capability-hash` 檔案上用檔案鎖（或 `crates/gt-storage` 現有的 `Mutex<Connection>` 模式，改成一個輕量的 per-agent 記憶體鎖）序列化「檢查 hash → 寫檔 → 更新 hash」這三步，避免兩個 session 同時判斷「需要重寫」而互相覆蓋對方寫到一半的檔案
- **快取失效時機**：capability snapshot 變更（使用者在 UI 改了掛載內容）、或 `detect_capability_support` 重新探測出不同結果（CLI 升級）時，兩者都會讓 hash 改變，自動觸發下一次啟動重新 materialize，不需要額外的手動失效機制

### 2.7 與 04 文件既有 `AgentPolicy` 的銜接

04 文件 Phase B「工具/MCP」類別，在有了這份 capability 設計之後可以具體化成：允許/拒絕清單只針對「已經掛載的」MCP server / tool 名稱做勾選。也就是說政策面天然依賴這份設計先落地。

---

## 3. 供應鏈 / 安全考量

Hook 是「使用者定義、啟動時會被實際執行的指令」，風險等級不低於 04 文件第 5 節對「外部 Provider Descriptor」的警戒（原文：「這是一份外部、可編輯的檔案，內容會被拿去執行」）。**因為決策 3 選了開放自訂指令，這一節的份量比原本「只給範本」的版本重，不能省略**：

- Hook 的建立/修改比照 `CLAUDE.md` 明文要求的 `preview -> validate -> confirm -> apply -> audit`，直接套用 `gt-ai-config`（`crates/gt-ai-config/src/service.rs`）已經有的 `audit_repository` 模式，不要重新發明一套
- **version-lock 比照 04 文件第 5 節對外部 Provider Descriptor 的做法**：每條 hook 規則存一個 content hash；hook 內容第一次建立、或跟上次使用者確認過的 hash 不一樣時，一律強制先過 2.5 節那個逐條列出「事件/matcher/實際指令」的 preview 畫面，使用者手動確認過才允許 apply；hash 沒變就不用每次重新 confirm，但 preview 畫面本身不能省略、不能靜默套用
- materialize 寫入 `.gtoffice/agents/<agent_id>/runtime/` 前，先做基本格式驗證（serde 反序列化失敗要擋下、不能靜默略過壞掉的 hook 定義後讓 CLI 自己炸開）
- 每條 hook 的 apply 動作都要寫進 `audit_repository`（誰、什麼時候、確認了哪個 hash、指令內容是什麼），這是「開放自訂指令」相對「只給範本」多出來的稽核義務，不是可選項

---

## 4. 涉及檔案（新增/擴充）

| 動作 | 檔案 |
|---|---|
| 新表 | `crates/gt-storage`：`agent_capability_snapshots` |
| 新欄位 | `crates/gt-agent/src/models.rs`：`AgentProfile.capability_snapshot_id`（ADD COLUMN） |
| 新模組 | `crates/gt-agent/src/capability/`：`AgentCapabilitySnapshot` 結構、`materialize()`、content-hash 快取與 per-agent 鎖 |
| 擴充 | `crates/gt-tools/src/agent_installer.rs`：`ProviderCapabilitySupport` 探測 + `.gtoffice/cache/provider-capability.json` 快取 |
| 改動簽名 | `crates/gt-agent-session/src/resume.rs`：`build_relaunch_launch_command` / `build_resume_launch_command` 接受 `MaterializedCapability`，回傳型別保持命令字串 |
| 改動呼叫點 | 建立/恢復 PTY session 共用 materialize 產物；Claude 追加旗標，Codex 插入 `-p <profile_name>`，不為能力掛載覆寫 CODEX_HOME |
| 新 command | `apps/desktop-tauri/src-tauri/src/commands/agent/`：capability 的 CRUD + preview/confirm/apply/audit（hook 部分套用 `gt-ai-config` 既有的 audit_repository 模式） |
| 新 UI 分頁 | `apps/desktop-web/src/features/workspace-hub/StationManageModal.tsx`：「能力 / Capabilities」分頁，含 hook 的逐條 preview 畫面 |

---

## 5. 與 04 文件路線圖的銜接

沿用 04 文件「小步、可驗證」的節奏，建議拆成：

| 階段 | 內容 |
|---|---|
| P3.5-capability-0 | 資料模型地基：`agent_capability_snapshots` 表、`AgentProfile.capability_snapshot_id`、`ProviderCapabilitySupport` 探測邏輯（不接 UI，先能跑測試） |
| P3.5-capability-1 | Claude 路徑：materialize + `resume.rs` 注入 + 旗標/複製雙軌、content-hash 快取（先驗證 Claude 這條路徑能真的把 MCP server 掛上去、agent 之間互不污染） |
| P3.5-capability-2 | Codex 路徑：namespaced profile + MCP/Skills/Hooks TOML（已實作，待真實 CLI 驗收） |
| P3.5-capability-3 | UI：能力分頁三個子分頁（MCP / Skills / Hooks），Hook 的完整 preview + version-lock confirm 流程 |

每個階段結束都要能過 `npm run typecheck` + `cargo check --workspace`，且要有明確驗證方式（單元測試涵蓋 materialize 的 hash 快取行為、resume.rs 組出的實際指令字串），不是「看起來對就好」。

---

## 6. 三種能力的設定流程

三者共用同一份 `AgentCapabilitySnapshot` 資料模型與同一個 materialize 進場口，但「使用者要不要手動確認」「materialize 後怎麼疊加進 CLI」不一樣，分開說明。

### 6.1 MCP 設定流程

```text
使用者在 StationManageModal →「能力」分頁 → MCP servers 子分頁
  │
  ├─ 1. 選來源：Settings 全域已註冊的 MCP server（下拉選）
  │        或手動填 stdio/sse/http 連線資訊（command/args/env 或 url）
  │
  ├─ 2. 儲存 → capability command 寫入新的一筆 agent_capability_snapshots
  │        （不可變快照，追加不覆寫）
  │        AgentProfile.capability_snapshot_id 改指向這筆新快照
  │
  └─ 3. 下次這個 agent 啟動／resume PTY 時：
        │
        ├─ a. 讀 capability_json，算 content hash，跟
        │      .gtoffice/agents/<agent_id>/runtime/.capability-hash 比對
        │      → 沒變就跳過 materialize，直接沿用既有的 MaterializedPaths
        │
        ├─ b. 有變：呼叫 materialize()
        │      ├─ Claude：ProviderCapabilitySupport.supports_mcp_config_flag=true
        │      │     → 寫 mcp.json，組 --mcp-config <path> 疊加進啟動指令
        │      └─ Codex：寫進使用者真實、未隔離的 $CODEX_HOME 底下一個
        │            GT-Office-namespaced 的 sibling 檔案
        │            `<profile_name>.config.toml`（只含這個 agent 的
        │            [mcp_servers] 表），never 動 CODEX_HOME/config.toml
        │            本身，也 never 隔離或改寫 CODEX_HOME 環境變數——
        │            $CODEX_HOME 底下還放著 auth.json 等登入憑證，隔離
        │            會直接把 Codex 登入弄壞（`codex doctor` 會報
        │            `✗ auth`），改用 `-p <profile_name>` 疊加旗標即可
        │            merge 進 base config，不需要隔離（見
        │            materialize_codex_capability 的完整說明）
        │
        └─ c. resume.rs 的 build_relaunch_launch_command（resume/relaunch）
              或 apply_capability_overlay_to_command（agent 第一次啟動）
              組出最終 command，寫進 PTY → CLI 啟動時載入這個 MCP server；
              Codex 的 `-p <profile_name>` 一定接在 subcommand 之前
```

**確認強度**：跟第 3 節 Hook 不同，MCP 設定 v1 不強制逐條 preview——存檔即成立新快照，下次啟動就生效。理由：MCP server 本身「能不能被呼叫」的風險管制屬於 04 文件 Phase B「工具/MCP」政策類別的範圍（見 2.7 節），這份文件負責的是「掛不掛得上」，不是「掛上後准不准用」。如果之後發現使用者常常誤填 MCP server 造成啟動失敗或安全疑慮，可以再補一層輕量 preview，但不在這次 v1 範圍內。

### 6.2 Skill 設定流程

```text
使用者在「能力」分頁 → Skills 子分頁（Claude／Codex 均可使用）
  │
  ├─ 1. 匯入方式：從本機路徑選一個 SKILL.md
  │        或勾選 workspace 內已存在、其他 agent 也在用的技能
  │
  ├─ 2. 儲存 → 新的 capability snapshot（skills 陣列：id / sourcePath / enabled）
  │
  └─ 3. 下次啟動時 materialize：Codex 生成 skills.config 的 path/enabled；以下為 Claude 路徑：
        │
        ├─ a. 讀 ProviderCapabilitySupport.supports_skills_dir_flag
        │
        ├─ b. true：用旗標疊加一個額外 skills 目錄
        │      （實際旗標名稱要對照目前釘選的 Claude Code CLI 版本，見 2.2 節）
        │
        ├─ c. false（fallback）：啟動前把 profile 裡的 skill
        │      複製進 agent 自己 workdir 的 .claude/skills/<skill-id>/
        │      → 用一份 manifest 記錄「這些檔案是 GT Office 寫的」
        │      → 不覆蓋使用者原本手動放在該目錄下的其他 skill
        │
        └─ d. content-hash 沒變就跳過重新複製（見 2.6 節）
              → Claude Code CLI 啟動時原生掃描 .claude/skills/，自動載入
```

**確認強度**：跟 MCP 一樣，v1 不強制 preview——Skill 本身是「文字內容的能力說明」，執行風險遠低於 Hook（Hook 是會被直接執行的指令）。

### 6.3 Hook 設定流程（三者中管制最嚴）

```text
使用者在「能力」分頁 → Hooks 子分頁（Claude／Codex 均可使用）
  │
  ├─ 1. 填寫一條規則：事件類型（PreToolUse/PostToolUse/…）
  │        + matcher（比對什麼工具/路徑）+ 實際要執行的指令
  │
  ├─ 2. 送出前：強制彈出完整 preview 畫面
  │        逐條列出「這個 hook 會在『{event}』事件、比對『{matcher}』時，
  │        實際執行『{command}』這一行指令」——不是丟一整包 JSON 給使用者看
  │
  ├─ 3. 判斷是否需要使用者手動點「確認」：
  │        │
  │        ├─ 這條規則第一次建立，或內容 hash 跟上次使用者確認過的不一樣
  │        │     → 一定要等使用者手動確認，才把這個 hash 記為「已確認」
  │        │
  │        └─ hash 沒變（沒改過內容）
  │              → 不用每次重新跳確認對話框，但 preview 畫面本身仍會顯示
  │                （只是不擋流程，比照 04 文件 Provider Descriptor 的 version-lock）
  │
  ├─ 4. 確認後 apply：
  │        ├─ 寫入新的 agent_capability_snapshots 快照
  │        └─ 寫一筆 gt-ai-config 既有的 audit_repository 紀錄
  │              （誰、何時、確認了哪個 hash、指令內容是什麼）
  │
  └─ 5. 下次啟動時 materialize：
        ├─ 先做基本格式驗證（serde 反序列化失敗要擋下，壞掉的 hook
        │    定義不能靜默略過讓 CLI 自己炸開）
        ├─ 寫進 agent runtime 目錄下的 settings.json 的 hooks 欄位
        └─ Claude：附加 --settings <path> 疊加進啟動指令（Codex v1 不支援）
```

### 6.4 三者一覽

| | MCP | Skill | Hook |
|---|---|---|---|
| 風險等級 | 中（會連外部服務/執行 stdio 指令） | 低（純文字內容） | 高（啟動時會被直接執行的指令） |
| 是否強制 preview | 否 | 否 | **是**，逐條列出事件/matcher/指令 |
| 是否需要手動 confirm | 否（存檔即生效） | 否 | **是**，內容 hash 變更時強制 |
| 是否寫入 audit | 選配 | 選配 | **是**，每次 apply 都寫 |
| Claude 疊加方式 | `--mcp-config` | 旗標優先、複製 fallback | `--settings` |
| Codex v1 支援 | ✓ | ✗（UI 停用） | ✗（UI 停用） |

---

## 7. 已拍板決策（2026-08-24）

以下 4 題已由使用者逐一拍板，內容已同步反映進第 1-6 節：

1. **Skills 掛載方式**：兩者都做，旗標優先、複製 fallback（見 2.4 節）。判斷依據來自 2.2 節新增的 `ProviderCapabilitySupport` 探測。
2. **Codex 支援範圍**：v1 兩個 provider 一起做，Codex 先求最小可用，只支援 MCP，Skills/Hooks 明確擋下並在 UI 標示未支援（見 2.1、2.4 節）。
3. **Hook 開放程度**：開放自訂指令，但強制配一套完整 preview UI，且比照 04 文件 Provider Descriptor 的 version-lock（content hash + 強制 confirm）機制（見 2.5、第 3 節）。
4. **多畫布節點快取**：這次一併設計 content-hash 快取 + per-agent 併發鎖（見 2.6 節），不留到之後。
