# 專案目前待辦

> 更新：2026-10-09；核對基準：本地 HEAD `1a152cf`，版本 `0.7.5`；以下已完成項目包含本輪尚未提交的修正。
> 本文件依目前程式碼、提交紀錄及本次測試整理。舊設計／進度文件保留歷史背景；其「未提交」「尚未開發」與舊驗收行為不應直接當作現況。

## 優先處理：已確認的缺口

- [x] **修正 Windows session scanner 路徑測試**（2026-10-08）。根因是測試 fixture 使用 POSIX 路徑，在 Windows 屬於缺少 drive prefix 的 root-relative 路徑，走到不存在檔案的 canonicalize 分支。已改用各平台有效的絕對路徑並明確斷言 `is_absolute()`，不改產品編碼規則。`cargo test -p gt-agent-session --lib`：84 通過、0 失敗；scanner rustfmt 檢查與該 crate all-targets clippy（deny warnings）通過。POSIX 分支保留原測試，但本次未在 macOS/Linux 執行。
- [x] **統一 Canvas 與終端活動狀態來源**（2026-10-08）。`gt-task/runtime_activity.rs` 接收後端可見／隱藏終端輸出與生命週期事件，依 workspace + current session 投影 Active/Idle，取代五分鐘 channel-message 判斷。退出移除 presence；換 session 清除活動，舊 session 事件不影響新 session。Canvas 訂閱終端事件，合併刷新狀態並保留八秒輪詢。活動是十秒內有輸出的證據，沒有輸出為 Idle；靜默思考／等待確認不做猜測，詳見 [API 契約](API_CONTRACTS.md)。
- [x] **釐清 Codex 的全域設定主開關語意**（2026-10-08）。全域來源排除只支援 Claude；Codex UI 已移除該主開關與 Claude 全域唯讀清單，Skills/Hooks 手動掛載能力仍保留。既有快照欄位不刪除；Codex 啟動仍忽略此布林值。
- [x] **決定並補齊 Agent 通訊授權範圍**（2026-10-08）。bridge 與 Tauri 的 dispatch/publish 共用 agent feature 的 fail-closed 檢查；Agent 必須屬於 workspace，任一目標未授權則在送出前拒絕，Direct channel id fallback 與實際 delivery 使用相同目標。Task/status/handover 都受 allow_gto_send 與 authored edge 約束；保留人類發訊、自己派發、communicate-with-all 例外，明確 deny 仍優先。已補規則與目標正規化測試，更新 Canvas 提示。
- [x] **同步能力掛載設計與實作差異**（2026-10-08）。已更新 [08 契約](cw/08_MCP_Hook_Skill掛載設計.md)：Codex namespaced profile、MCP/Skills/Hooks TOML、共用首次啟動／resume 的 materialize 入口、命令字串契約、Claude-only 全域開關、快取與 warning + 未掛載啟動限制。

## 已實作，驗收完成

2026-10-09：使用者明確回報原「已實作，仍需補驗收證據」七項全部驗證成功，依此結案。以下說明保留原驗收範圍，其中「待驗證／仍需」為原清單措辭，已由本次回報更新為通過。第一輪自動化驗證與後續使用者驗收分別記錄於 [驗收紀錄](qa/2026-10-09-acceptance.md)；未另外提供逐項操作紀錄、截圖或平台版本。

- [x] **全域 Hook/Skill 開關**：依 [21 文件](cw/21_全域Hook_Skill開關設計.md) 第 7 節更新清單驗證唯讀徽章、保存／重開、Claude 全域來源排除與手動掛載。全域清單永遠唯讀，不再測試「關閉後自動全選／可個別勾選」。
- [x] **Canvas 與能力節點完整操作回歸**：連線建立／移除、未連線派發限制（關閉 communicate-with-all，排除自己派發例外）、Subagent、框選、對齊、多 instance、顏色與重開後持久化、MCP 節點開關及 Ctrl+Z。已有 `8ab8a94`、`b68365e`、`9a84644` 及後續修復提交；依現行畫面調整 [P4.5](cw/06_P4.5開發進度.md)、[P4.6](cw/07_P4.6開發進度.md)、[P3.6](cw/10_P3.6-capability開發進度.md) 的歷史清單後執行。
- [x] **協作上下文注入回歸**：`gt-agent::build_collaboration_context` 已存在，tool_profiles 啟動時已附加 context，並支援 communicate-with-all；[17 排查](cw/17_Agent_Canvas連線未觸發主動協作排查.md) 的「尚未實作」已過期。待驗證真實委派、連線變更後既有 session 的上下文刷新，以及 resume 路徑；不要重新安排已完成的初始注入。
- [x] **Session Boundary 與輸出收集聯合驗收**：依 [19](cw/19_輸出收集與SessionBoundary訊息衝突排查.md)／[20](cw/20_SessionBoundary自動判斷Stop_Hook設計.md)，在乾淨 session 開啟兩個功能，確認任務邊界會正確重啟並產生新 session，輸出收集不阻斷重啟。個人全域 Hook 部署不等於可重現的專案交付或實機驗收。
- [x] **Claude／Codex 能力端到端驗證**：首次啟動、resume/fork、自訂 launch command、多工作區隔離；MCP 要在真實 session 中被呼叫，Skills/Hooks 要確認實際載入／觸發，不只驗證保存或 profile 生成。Codex Skills/Hooks 已有實作與單元測試，不再列為未支援。
- [x] **Windows 終端修復後回歸及跨平台打包 QA**：已有 `8411428`、`1b977e7`、`9f47d99` 的 ConPTY 設定、bundling 與 dev 載入修復，以及 `19a6974` 的結束原因紀錄；仍需在 Windows 10／11 驗證長輸出、scrollback、縮放／切換／恢復，並補 macOS 打包操作紀錄。舊「渲染破圖待開始」改為回歸驗證，若仍出錯才建立具體修復項目。
- [x] **同步能力掛載設計與實作差異**（2026-10-08）。已更新 [08 契約](cw/08_MCP_Hook_Skill掛載設計.md)：Codex namespaced profile、MCP/Skills/Hooks TOML、共用首次啟動／resume 的 materialize 入口、命令字串契約、Claude-only 全域開關、快取與 warning + 未掛載啟動限制。
- [x] **近期 CLI 更新與 session 名稱功能回歸**：對照 `38ab76a`／`f7bfef6` 驗證 Claude/Codex 更新與程序阻擋；對照 `eb59b02`／`3de16f5`／`d98984a` 驗證 session 卡片、重新命名、首個任務自動命名及終端回覆不誤命名。這些是已提交功能，需補當前版本驗收紀錄。

## 第一輪驗收發現的工程追蹤（2026-10-09）

以下保留第一輪程式碼／測試環境觀察，與上述使用者回報的功能驗收分開追蹤；本次僅同步驗收文件，未重新核對後續修正。

- [ ] **補齊 resume 協作上下文**：session_resume_check 未重新注入目前拓撲；ContinueLast／ForkLast 傳入 None capability。先釐清 agent／workspace 身分契約，再修正及驗證。
- [ ] **修復前端測試基礎設施**：完整測試編譯有 9 個過期 role 相關診斷；另有 Node alias 與 SCSS 未複製問題。
- [x] **補足桌面驗收回報**：使用者已確認上述七項全部驗證成功；第一輪 agent 的 native pipe 連線失敗（os error 2）仍保留為歷史環境限制。

## 後續規劃，尚未排期

- [ ] **P5 Provider Descriptor／版本鎖重構**：`crates/gt-tools/src/agent_installer.rs` 仍是 `AgentType::{ClaudeCode, Codex}`，未找到原設計的通用 Provider Descriptor。CLI 更新修復不等於這項重構已完成。
- [ ] **Git/VCS 政策、執行逾時與最大步數 enforcement**：`StationManageModal.tsx` 仍明確標示只保存、尚未執行；需要 Agent 身分與任務生命週期支援後再落地。
- [ ] **其餘 Phase B 權限與 Office Lobby**：保留 [客製化設計](cw/04_客製化設計.md) 的後續方向；不把已完成的 MCP／能力掛載重新列入未開發清單。Agent Runtime PRD 的完整差距尚未逐項稽核，需另做範圍明確的對照才排期。

## 初次核對與驗證（歷史）

- 核對開始時 `git status --short` 無輸出；上述核心功能在 HEAD 程式碼及提交紀錄中存在，「尚未 commit」已過期。
- `cargo test -p gt-agent --lib capability`：69 通過、0 失敗，包含 Codex Skills/Hooks materialize 測試。
- `cargo test -p gt-agent --lib collaboration`：3 通過、0 失敗；初始協作上下文組裝已實作。
- 初次核對 `cargo test -p gt-agent-session --lib`：83 通過、1 失敗；本輪修正測試 fixture 後重新執行：84 通過、0 失敗。
- GitHub：政策指定的 `Laplace-bit/GT-Office` open issues 查詢為空；實際 origin 是 `CWLin0518/CW-Agent-Office`，其 Issues 已停用。兩者不可混用為這份 fork 的完整待辦來源。
- 初次核對僅更新文件／看板；本輪另修正 scanner 測試，`rustfmt --edition 2021 --check crates/gt-agent-session/src/scanner.rs` 與 `cargo clippy -p gt-agent-session --all-targets -- -D warnings` 通過。未重新執行全倉 typecheck/build/clippy，歷史通過紀錄不代表本次 HEAD 全部驗證通過。

## 優先缺口修正驗證（2026-10-08）

- `cargo test -p gt-agent -p gt-task -p gt-agent-session --lib`：84 + 13 + 84 個測試通過。
- `cargo test -p gt-task`：13 個 library + 27 個 integration 測試通過；1 個既有手動效能測試按預設 ignored。修正 integration fixture 的已移除 role_key／缺少 suppress_output_collection_instructions 欄位，恢復受影響 crate 的完整驗證路徑。
- `cargo clippy -p gt-agent -p gt-task -p gt-agent-session --all-targets -- -D warnings`：通過。
- `cargo check --workspace`、`npm run typecheck`（含 production web build）、改動前端檔案 eslint：通過。
- `cargo clippy -p gtoffice-desktop-tauri --lib -- -D warnings`：通過。最小配套修正：Unix process parser 使用非 Windows cfg；Business Designer 的 text 診斷採等價 match guard，排除既有 collapsible_match 警告。未改上述功能行為。
- 最後一輪前端 `npx tsc -b`／改動檔案 eslint、改動 Rust 檔案 rustfmt check、`git diff --check`：通過。
- 未做桌面操作、macOS/Linux 執行與真實付費 CLI 測試；仍保留「已實作，待補驗收證據」清單。模型 waiting/failed/attention 的完整 Runtime Snapshot 尚需 provider 訊號，不等同此次活動投影修正。
