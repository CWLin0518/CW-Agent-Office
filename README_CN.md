<div align="center">

# CW Agent Office

### 為 Claude Code 與 Codex CLI 打造的視覺化多 Agent 桌面工作空間

**在同一個桌面應用程式中管理 Agent、掛載能力並協調工作。**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Latest Release](https://img.shields.io/github/v/release/CWLin0518/CW-Agent-Office?color=green&label=Download)](https://github.com/CWLin0518/CW-Agent-Office/releases)

[版本下載](https://github.com/CWLin0518/CW-Agent-Office/releases) · [文件](docs/README.md) · [English](README.md)

</div>

---

## CW Agent Office 是什麼？

CW Agent Office 是以 GT Office 為基礎改版的跨平台桌面應用程式，採用 **Tauri + React + Rust** 建構，整合 **Claude Code、Codex CLI**、終端機、檔案、Git、任務與通訊頻道。你可以在同一個工作區管理多個 Agent，透過協作畫布設定通訊關係、掛載能力並檢視成果。

## 為什麼使用 CW Agent Office？

將分散的終端機、Agent 設定、任務交接與專案成果集中管理，並用視覺連線明確表達 Agent 之間的協作關係。

---

## 核心功能

### 視覺化 Agent 協作畫布

![Agent 協作畫布：Agent 連線、MCP、Skills、Hooks 與輸出節點](docs/assets/agent-canvas.jpg)

Agent 節點顯示提供者、模型與活動狀態；能力和輸出節點呈現各 Agent 的設定與成果。

### Agent 工作站與通訊頻道

| Agent 工作站 | 通訊頻道 |
|:---:|:---:|
| ![Agent 工作站](docs/assets/agents-view.png) | ![通訊頻道](docs/assets/channel-view.png) |
| 在同一工作區啟動與管理多個 AI Agent | 連接 Telegram、微信與飛書 |

### 任務、檔案與 Git

| 任務 | 檔案總管 | Git |
|:---:|:---:|:---:|
| ![任務中心](docs/assets/task-view.png) | ![檔案總管](docs/assets/explorer-view.png) | ![Git 工作空間](docs/assets/git-view.png) |
| 追蹤任務與進度 | 瀏覽與編輯專案檔案 | 檢視變更並管理 Git 操作 |

### 功能總覽

| 功能 | 說明 |
|---|---|
| **協作畫布** | 拖曳 Agent 節點、建立通訊連線，支援框選、對齊、顏色、縮放與同一 Agent 的多個視覺節點。 |
| **能力掛載** | 為 Claude 與 Codex Agent 設定 MCP、Skills、Hooks；Claude 另提供全域 Skills／Hooks 納入控制。 |
| **通訊權限** | 任務派發、狀態回報與交接遵循手動連線及通訊權限，另提供與所有 Agent 溝通的選項。 |
| **輸出收集** | 從畫布檢視 Agent 輸出清單，開啟產生的檔案與成果。 |
| **Agent 工作站** | 啟動與管理 Claude Code、Codex CLI，設定模型、啟動指令與獨立終端機。 |
| **工作階段** | 檢視歷史、恢復或分支工作階段，使用獨立工作台視窗。 |
| **任務與協作** | 透過任務中心及 `gto` 派發工作、檢視收件匣與討論串、回報進度和交接。 |
| **檔案與預覽** | 瀏覽、搜尋、編輯檔案，預覽 Markdown、圖片、PDF、音訊與影片。 |
| **Git** | 檢視狀態、差異與提交歷史，管理提交、分支、標籤、暫存與合併衝突。 |
| **通訊頻道** | 設定 Telegram、微信與飛書連線及 Agent 綁定。 |
| **業務設計器** | 編輯節點式設計文件，使用預覽與歷史紀錄。 |
| **工作區設定** | 管理多個工作區、版面配置、主題、顯示偏好與快捷鍵。 |

## 適合誰使用？

- 同時使用多個 Claude Code 或 Codex CLI Agent 的開發者。
- 希望以視覺介面設定協作關係與掛載能力的使用者。
- 需要整合終端機、任務、檔案、成果與 Git 的專案負責人。

---

## 快速開始

### 下載版本

請至 [Releases](https://github.com/CWLin0518/CW-Agent-Office/releases) 查看可用建置產物；本機開發可依下方步驟從原始碼啟動。

### 從原始碼啟動

前置需求：**Node.js 20+**、**Rust stable** 與 Tauri 2 所需的平台依賴。請先安裝並完成欲使用的 CLI Agent 登入設定。

```bash
git clone https://github.com/CWLin0518/CW-Agent-Office.git
cd CW-Agent-Office
npm install
npm run dev:tauri
```

### 開始協作

1. 開啟專案資料夾作為工作區。
2. 建立 Agent，設定提供者、模型與掛載能力。
3. 將 Agent 拖曳至畫布，連接需要互相溝通的 Agent。
4. 從工作台啟動 Agent，透過任務中心或 `gto` 派發工作。
5. 檢視回報、交接與輸出檔案，透過檔案介面及 Git 檢查成果。

---

## 架構

```text
CW-Agent-Office/
├── apps/desktop-web/       # React + TypeScript + Vite 前端
├── apps/desktop-tauri/     # Tauri 桌面外殼
├── crates/                # Rust 領域模組
├── packages/shared-types/ # 共用契約
├── tools/gto/             # Agent 通訊 CLI
├── scripts/               # 開發、建置與版本發布工具
└── docs/                  # 架構、設計與驗收文件
```

前端功能位於 `apps/desktop-web/src/features/`；Rust 模組提供工作區、終端機、Git、任務與 Agent 能力。專案操作與 Agent 通訊以工作區 ID 區分範圍。

---

## 文件

| 文件 | 內容 |
|---|---|
| [文件索引](docs/README.md) | 技術文件入口 |
| [系統架構](docs/ARCHITECTURE.md) | 模組邊界與資料流 |
| [核心工作流程](docs/WORKFLOWS.md) | 操作流程與多工作站協作 |
| [API 契約](docs/API_CONTRACTS.md) | 指令、事件與共用型別 |
| [客製化文件](docs/cw/README.md) | 中文架構說明與改版設計 |
| [目前進度](docs/TODO.md) | 實作狀態、驗收紀錄與待辦 |
| [依賴策略](docs/DEPENDENCIES.md) | 依賴管理規範 |
| [版本發布流程](docs/release-process.md) | 版本管理、CI 與產物發布 |

---

## 專案來源與授權

本專案由 **[Laplace-bit 的 GT Office](https://github.com/Laplace-bit/GT-Office)** 改版而來，沿用原始專案的桌面應用程式、工作區管理、終端機、Git 與 Agent 通訊基礎，並進一步客製化視覺協作畫布、能力掛載與輸出工作流程。

原始專案的開發成果歸功於 **Laplace-bit 與 GT Office 貢獻者**。本改版倉庫為 [CWLin0518/CW-Agent-Office](https://github.com/CWLin0518/CW-Agent-Office)，保留 [Apache License 2.0](LICENSE) 授權。
