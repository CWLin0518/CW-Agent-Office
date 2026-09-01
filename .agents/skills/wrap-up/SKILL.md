---
name: wrap-up
description: 整理本次工作 session 的進度為一份 Markdown，寫入 GT-Office agent 輸出目錄（Agent Canvas 可直接檢視）。當使用者說要收工、結束這次工作、要求整理進度或寫交接紀錄時使用。
---

把「這次工作 session 做了什麼」整理成一份繁體中文 Markdown 進度報告，存到 GT-Office 既有的 agent 輸出目錄慣例（`.gtoffice/agents/<agent_id>/outputs/`，見 `docs/cw/14_Agent輸出清單化.md`）。這個目錄會被 Agent Canvas 自動掃描，出現在對應 agent 節點的「輸出」清單裡，可直接點開閱覽。

## 步驟

1. **決定輸出目錄**（依序判斷，第一個成立就用它）：
   - 環境變數 `GTO_OUTPUT_DIR` 有值 → 直接用這個路徑。
   - 否則 → 用 `<repo 根目錄>/.gtoffice/agents/codex/outputs/`（repo 根目錄用 `git rev-parse --show-toplevel` 取得；`codex` 是「直接在編輯器裡開的 Codex CLI session、未經 GT-Office 派發」固定使用的 agent id）。
   - 目錄不存在就建立。

2. **蒐集本次工作內容**（只看這次 session 實際發生的事，不要編造）：
   - `git status` / `git diff` 看有哪些未 commit 的改動
   - `git log` 看這次 session 建立的 commit（如果有）
   - 回顧這次對話中做的決策、踩過的坑、還沒解決的問題
   - 目標是「下一個人接手需要知道的最少必要資訊」，不是逐字複述對話

3. **寫檔**：
   - 檔名：`progress-<YYYYMMDD-HHmm>.md`（用當下時間，避免覆蓋前一份）
   - 內容結構：
     - `# 進度摘要` + 時間戳記
     - `## 本次完成`：做了什麼、改了哪些檔案（用相對路徑列出）
     - `## 關鍵決策`：為什麼這樣做，特別是不明顯的取捨
     - `## 未解決 / 待辦`：還缺什麼、已知問題
     - `## 驗證狀態`：跑過什麼驗證（test / typecheck / build / lint / 手動驗證），或明確寫「未驗證 + 原因」
     - `## 下一步建議`：接手者應該先做什麼

4. 寫完後用一句話回報檔案的完整路徑，不用把全文貼回對話。

若使用者有額外補充說明想強調的重點，一併納入報告。
