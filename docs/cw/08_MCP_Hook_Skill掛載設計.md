# 08 Agent 能力掛載設計：按 Agent 各自配置 MCP / Tool / Hook / Skill

> 狀態：設計已定案（2026-08-24，見第 6 節）。這份文件延續 04 文件的體例——每節先講地基，再講設計，關鍵決策集中列在文末，動工前不需要再回頭確認這些點。
>
> 定位：延續 [04_客製化設計.md](04_客製化設計.md) 第 3 節「新增 Agent 的權限分頁」——那份文件已經把 17 類權限分成 Phase A（已有掛勾點）跟 Phase B（需要先建子系統），其中「工具/MCP」被列進 Phase B，但**沒有展開細節**；Hook、Skill 這兩個概念完全沒被 17 類清單涵蓋。這份文件把「工具/MCP」的 Phase B 展開，並補上 Hook / Skill。

---

## 0. 先講一個容易混淆的區分：這不是「權限」，是「掛載」

04 文件的 `AgentPolicy` / `policy_snapshot_id` 設計，回答的是「Agent 能不能做 X」（allow / deny / require-approval），適合「這個 agent 可不可以呼叫某個 MCP 工具」這種是非題。

但「這個 agent 要掛哪個 MCP server、哪些 skill 檔案、哪些 hook 腳本」是**「配置什麼東西存在」**的問題，不是是非題。所以這份設計提出一個獨立於 `AgentPolicy` 的新概念：`AgentCapabilityProfile`（掛載了什麼），而政策系統可以疊加在它之上（例如：掛了 MCP-X，但政策規定呼叫 MCP-X 的某個工具需要人工核准）。

**順序關係**：沒有先「掛載」，04 文件 Phase B 的「工具/MCP」政策面也沒東西可以勾選/管制。所以這份設計是 04 文件 Phase A → Phase B 之間的銜接步驟，建議命名為 **P3.5-capability**（避免跟 04 文件已用掉的 P4.5 撞名）。

---

## 1. 現況地基

- `AgentProfile`（`crates/gt-agent/src/models.rs`）已有 `tool`（provider）、`workdir`、`launch_command: Option<String>`（本來就可覆寫）、`policy_snapshot_id`（預留欄位，目前永遠寫 NULL）。
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

**Provider 支援範圍不對稱（決策 2，見第 6 節）**：`capability_json` 的形狀是共用的，但 v1 階段 Codex agent 只有 `mcpServers` 欄位會被實際 materialize；`skills` / `hooks` 欄位對 Codex agent 允許填但**寫入時直接擋掉並回錯誤訊息**（不要靜默忽略——靜默忽略等於使用者以為掛上了、其實沒掛上，正是 04 文件反覆警告的「假掛載」）。UI 層（2.4 節）在選到 Codex agent 時，Skills / Hooks 分頁直接顯示「Codex 尚未支援」並停用輸入，不要等後端報錯才發現。

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

- 兩個函式簽名多接一個 `Option<&MaterializedPaths>` 參數；建議順手把回傳型別從「單一字串」改成「command + args 陣列」，之後疊加旗標不用處理字串轉義問題（這是唯一牽動既有簽名的改動，記得同步改 `resume.rs` 現有的單元測試）

**Claude（決策 1：旗標優先、複製 fallback）**：
1. 讀 2.2 節探測到的 `ProviderCapabilitySupport`
2. MCP / hooks：`supports_mcp_config_flag` / `supports_settings_flag` 為 true 時，附加 `--mcp-config <path>/mcp.json`、`--settings <path>/settings.json`；為 false 時記一筆警告並跳過（這兩個功能沒有安全的檔案複製 fallback，跳過比硬套一個可能不相容的旗標安全）
3. Skills：`supports_skills_dir_flag` 為 true 時比照 MCP 用旗標疊加；為 false 時 fallback 成「啟動前把 profile 的 skills **複製**進 agent 自己 workdir 的 `.claude/skills/`」——複製前先做差異比對，只寫入/覆蓋 GT Office 自己管理的 skill id（用檔名前綴或一份 manifest 記錄哪些檔案是本機制寫的），不覆蓋使用者原本手動放在該目錄下的其他 skill

**Codex（決策 2：v1 只做 MCP，最小可用）**：
1. 不直接寫使用者全域的 `~/.codex/config.toml`（那是共用檔案，多 agent 會互相覆蓋）
2. 啟動 PTY 時透過 `TerminalCreateRequest::env` 注入 `CODEX_HOME=.gtoffice/agents/<agent_id>/runtime/codex-home`（依 1 節，這個注入點已現成可用）
3. materialize 時，先讀使用者原本 `~/.codex/config.toml` 的非 `mcp_servers` 內容複製一份到這個隔離的 `codex-home/config.toml`，再把 agent 專屬的 `mcp_servers` 表併進去——**這一步是本設計工程風險最高的部分**，因為前提是「Codex 讀取一個完整檔案而非疊加式旗標」的假設要先用實際版本驗證過，若驗證後發現行為不同，這裡的合併邏輯要重新設計，不要照本節文字直接動工
4. Skills / Hooks 依 2.1 節，v1 對 Codex agent 一律不 materialize，UI 停用對應輸入

### 2.5 UI

比照 04 文件第 3 節「權限」分頁的做法，在 `StationManageModal.tsx` 新增一個相鄰分頁（例如「能力 / Capabilities」）：

- **MCP servers**：下拉選一個已在 Settings 全域註冊過的 server，或手動填 stdio 指令；Claude / Codex 都可用
- **Skills**：從本機路徑匯入一個 `SKILL.md`，或勾選 workspace 內已存在的技能；Codex agent 停用此分頁並顯示「尚未支援」
- **Hooks（決策 3：開放自訂指令 + 完整 preview UI）**：使用者可以自己填 hook 指令（不限預設範本），但送出前必須經過完整 preview 畫面——顯示「這個 hook 會在什麼事件（PreToolUse/PostToolUse/…）、比對什麼 matcher 時，實際執行哪一行指令」，逐條列出而不是丟一整包 JSON 給使用者自己看。Codex agent 停用此分頁

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
| 新模組 | `crates/gt-agent/src/capability.rs`：`AgentCapabilitySnapshot` 結構、`materialize()`、content-hash 快取與 per-agent 鎖 |
| 擴充 | `crates/gt-tools/src/agent_installer.rs`：`ProviderCapabilitySupport` 探測 + `.gtoffice/cache/provider-capability.json` 快取 |
| 改動簽名 | `crates/gt-agent-session/src/resume.rs`：`build_relaunch_launch_command` / `build_resume_launch_command` 接受 `MaterializedPaths`，回傳型別改成 command+args |
| 改動呼叫點 | 建立/恢復 PTY session 的既有邏輯：把 `CLAUDE_CONFIG_DIR`（skills fallback 時不需要，但 Codex 的 `CODEX_HOME` 一定需要）塞進 `TerminalCreateRequest::env` |
| 新 command | `apps/desktop-tauri/src-tauri/src/commands/agent/`：capability 的 CRUD + preview/confirm/apply/audit（hook 部分套用 `gt-ai-config` 既有的 audit_repository 模式） |
| 新 UI 分頁 | `apps/desktop-web/src/features/workspace-hub/StationManageModal.tsx`：「能力 / Capabilities」分頁，含 hook 的逐條 preview 畫面 |

---

## 5. 與 04 文件路線圖的銜接

沿用 04 文件「小步、可驗證」的節奏，建議拆成：

| 階段 | 內容 |
|---|---|
| P3.5-capability-0 | 資料模型地基：`agent_capability_snapshots` 表、`AgentProfile.capability_snapshot_id`、`ProviderCapabilitySupport` 探測邏輯（不接 UI，先能跑測試） |
| P3.5-capability-1 | Claude 路徑：materialize + `resume.rs` 注入 + 旗標/複製雙軌、content-hash 快取（先驗證 Claude 這條路徑能真的把 MCP server 掛上去、agent 之間互不污染） |
| P3.5-capability-2 | Codex 路徑：`CODEX_HOME` 隔離 + config.toml 合併（工程風險最高，安排在 Claude 路徑驗證過後） |
| P3.5-capability-3 | UI：能力分頁三個子分頁（MCP / Skills / Hooks），Hook 的完整 preview + version-lock confirm 流程 |

每個階段結束都要能過 `npm run typecheck` + `cargo check --workspace`，且要有明確驗證方式（單元測試涵蓋 materialize 的 hash 快取行為、resume.rs 組出的實際指令字串），不是「看起來對就好」。

---

## 6. 已拍板決策（2026-08-24）

以下 4 題已由使用者逐一拍板，內容已同步反映進第 1-5 節：

1. **Skills 掛載方式**：兩者都做，旗標優先、複製 fallback（見 2.4 節）。判斷依據來自 2.2 節新增的 `ProviderCapabilitySupport` 探測。
2. **Codex 支援範圍**：v1 兩個 provider 一起做，Codex 先求最小可用，只支援 MCP，Skills/Hooks 明確擋下並在 UI 標示未支援（見 2.1、2.4 節）。
3. **Hook 開放程度**：開放自訂指令，但強制配一套完整 preview UI，且比照 04 文件 Provider Descriptor 的 version-lock（content hash + 強制 confirm）機制（見 2.5、第 3 節）。
4. **多畫布節點快取**：這次一併設計 content-hash 快取 + per-agent 併發鎖（見 2.6 節），不留到之後。
