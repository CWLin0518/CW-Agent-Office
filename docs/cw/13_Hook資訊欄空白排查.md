# 13 Hook 資訊欄展開空白排查

> 承接 [12_Hook掛載清單化.md](12_Hook掛載清單化.md) §1.3「掃描到但還沒掛載的 Hook，自動從腳本推測功能說明」。這份文件記錄一次實際排查：使用者在 Capabilities → Hooks 分頁勾選一筆掃描到的 Hook、展開詳情面板後看不到任何推測說明，只看到原始指令跟來源路徑。
>
> 狀態（2026-08-25）：已排查完成，結論是**編譯版本落後於原始碼**，不是解析邏輯或腳本本身的問題；本文件同時記錄「正常情況下應該看到什麼」，供之後重新編譯驗收時對照。

## 1. 問題現象

專案：`C:\Users\User\Desktop\REVIT_MCP_study`（workspace 內既有 `.claude/settings.json`，定義了 4 支 `PostToolUse`/matcher 型 Hook）。

操作：Capabilities → Hooks 分頁 → 專案工作區已設定的 Hook 清單 → 勾選 `PostToolUse · Bash|Write|Edit`（對應 `detect-claudemd-trigger.sh`）→ 點資訊圖示展開詳情。

預期：應該看到「推測的功能說明」區塊。實際：只看到「實際執行的指令」跟來源路徑，說明區塊完全沒有出現。

## 2. 排查過程

先假設是腳本本身沒有寫開頭註解（`read_leading_comment` 的正常回傳情況之一），實際打開腳本檔核對：

```bash
$ head -9 "C:\Users\User\Desktop\REVIT_MCP_study\.claude\hooks\detect-claudemd-trigger.sh"
#!/bin/bash
# detect-claudemd-trigger.sh
# PostToolUse hook: 偵測三種觸發事件，注入雙向驗證指令
#
# 觸發條件：
#   1. git merge / pull / rebase（合併外部 PR）
#   2. 寫入 .claude/skills/ 路徑（Domain 升級 Skill）
#   3. 寫入 MCP-Server/src/tools/ 或 MCP/Core/CommandExecutor（Tools 檢討）
```

腳本明明有完整的開頭註解，排除「腳本沒寫註解」這個可能。接著直接在 `gt-agent` crate 裡加一個暫時測試，對這個專案真實的 `.claude/settings.json` 呼叫 `scan_settings_hooks(...)` 驗證解析邏輯本身：

```
event=PostToolUse matcher=Some("Bash|Write|Edit")
command="$CLAUDE_PROJECT_DIR"/.claude/hooks/detect-claudemd-trigger.sh
inferred_description=Some("detect-claudemd-trigger.sh\nPostToolUse hook: 偵測三種觸發事件，注入雙向驗證指令\n\n觸發條件：\n1. git merge / pull / rebase（合併外部 PR）\n2. 寫入 .claude/skills/ 路徑（Domain 升級 Skill）\n3. 寫入 MCP-Server/src/tools/ 或 MCP/Core/CommandExecutor（Tools 檢討）")
```

4 支 Hook 全部都正確解析出推測說明（測試完即刪除，不是留存的正式測試）。這證明：
- `${CLAUDE_PROJECT_DIR}`/`$CLAUDE_PROJECT_DIR` 展開正確
- 腳本路徑解析、讀檔、註解擷取全部正確
- 問題不在 `discovery.rs` 的解析邏輯，也不在腳本本身

## 3. 根本原因

`DiscoveredHook.inferred_description` 是**這次會話裡才加進 `gt-agent` crate 的新邏輯**（12 號文件 §1.3）。GT Office 的後端是編譯後的 Rust 原生二進位檔（Tauri 應用程式），**修改 Rust 原始碼不會讓正在執行中的 GT Office 自動生效**——前端（React/TS）有 dev server 熱重載，但後端 command 的行為要看當初編譯進二進位檔裡的版本，改完原始碼之後必須：

1. 重新編譯（`npm run build:tauri`，或開發模式下讓 `cargo build`/`tauri dev` 重新編過一次）
2. 完全關閉並重新開啟 GT Office（不是重新整理視窗）

在完成這兩步之前，畫面上看到的行為都反映的是**改動前**的舊二進位檔，不代表新邏輯有問題。

## 4. 打開 Hook 資訊時「應該」看到的內容

以 `detect-claudemd-trigger.sh` 為例，重新編譯 + 重啟後，勾選該筆 Hook 並展開詳情面板，應該依序看到：

1. **推測的功能說明**（標註「讀取腳本開頭注釋，僅供參考」）——腳本開頭連續註解行、去掉 `#`/`//`/`/* */` 標記後逐行接起來：
   ```
   detect-claudemd-trigger.sh
   PostToolUse hook: 偵測三種觸發事件，注入雙向驗證指令

   觸發條件：
   1. git merge / pull / rebase（合併外部 PR）
   2. 寫入 .claude/skills/ 路徑（Domain 升級 Skill）
   3. 寫入 MCP-Server/src/tools/ 或 MCP/Core/CommandExecutor（Tools 檢討）
   ```
2. **實際執行的指令**——原始 `command` 字串：`"$CLAUDE_PROJECT_DIR"/.claude/hooks/detect-claudemd-trigger.sh`
3. **來源路徑**（`source_path`）——這筆 Hook 是從哪個 `.claude/settings.json` 掃描到的
4. 若已勾選（掛載）：額外出現一個可編輯的「備註」textarea，預設值會自動帶入第 1 項的推測說明，使用者可再自行修改或清空

## 5. 之後怎麼判斷「真的沒有推測說明」vs「還沒重新編譯」

- 先確認已經重新編譯並完全重啟過 GT Office
- 若重啟後展開詳情**完全沒有第 1 項推測說明區塊**（只剩指令跟來源路徑），才代表：
  - 該腳本開頭真的沒有可辨識的註解，或
  - `command` 字串裡辨識不出腳本路徑（例如純 inline 指令、副檔名不在 `.sh`/`.bash`/`.js`/`.mjs`/`.cjs`/`.py`/`.ps1`/`.rb` 支援清單內）
- 遇到這種情況，把該筆 Hook 的 `command` 內容提供出來，可以判斷是腳本本身沒寫註解，還是解析邏輯目前沒涵蓋到的情況（例如目前 `.ps1` 只支援 `#` 行註解，還沒支援 PowerShell 的 `<# ... #>` 區塊註解，屬於已知限制）
