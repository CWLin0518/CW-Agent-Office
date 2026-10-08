<div align="center">

# CW Agent Office

### A Visual Multi-Agent Desktop Workspace for Claude Code and Codex CLI

**Manage agents, connect their capabilities, and coordinate work in one desktop app.**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Latest Release](https://img.shields.io/github/v/release/CWLin0518/CW-Agent-Office?color=green&label=Download)](https://github.com/CWLin0518/CW-Agent-Office/releases)

[Releases](https://github.com/CWLin0518/CW-Agent-Office/releases) · [Documentation](docs/README.md) · [繁體中文](README_CN.md)

</div>

---

## What Is CW Agent Office?

CW Agent Office is a customized version of GT Office, a cross-platform desktop application built with **Tauri + React + Rust**. It brings **Claude Code, Codex CLI**, terminals, files, Git, tasks, and channels into one workspace, with a visual canvas for organizing agent communication, capabilities, and outputs.

## Why CW Agent Office?

Keep terminals, agent configuration, task handovers, and project outputs together, and make agent collaboration relationships visible through canvas connections.

---

##  Core Features

### Visual Agent Collaboration Canvas

![Agent collaboration canvas with connections, MCP, Skills, Hooks, and output nodes](docs/assets/agent-canvas.jpg)

Agent nodes show the provider, model, and activity status; capability and output nodes expose configuration and results.

### Agent Workstations and Channels

| Agent Workstations | Channels |
|:---:|:---:|
| ![Agent workstations](docs/assets/agents-view.png) | ![Communication channels](docs/assets/channel-view.png) |
| Launch and manage multiple AI agents in one workspace | Connect agents to Telegram, WeChat, and Feishu |

### Tasks, Files, and Git

| Tasks | Explorer | Git |
|:---:|:---:|:---:|
| ![Task center](docs/assets/task-view.png) | ![File explorer](docs/assets/explorer-view.png) | ![Git workspace](docs/assets/git-view.png) |
| Track tasks and progress | Browse and edit project files | Review changes and manage Git operations |

### Feature Overview

| Feature | Description |
|---|---|
| **Collaboration canvas** | Drag agent nodes, draw communication connections, and use multi-selection, alignment, colors, zoom, and multiple visual instances of an agent. |
| **Capability mounts** | Configure MCP, Skills, and Hooks for Claude and Codex agents; Claude also has global Skills/Hooks inclusion controls. |
| **Communication permissions** | Task dispatch, status reports, and handovers follow authored connections and explicit permissions, with a communicate-with-all option. |
| **Output collection** | Inspect agent output lists and open generated files from the canvas. |
| **Agent workstations** | Launch and manage Claude Code and Codex CLI with model settings, launch commands, and individual terminals. |
| **Sessions** | Browse history, resume or fork sessions, and use detached workbench windows. |
| **Tasks and collaboration** | Dispatch work, inspect inboxes and threads, and send status reports and handovers through the task center and gto. |
| **Files and previews** | Browse, search, and edit files; preview Markdown, images, PDFs, audio, and video. |
| **Git** | Review status, diffs, and history; manage commits, branches, tags, stashes, and merge conflicts. |
| **Channels** | Configure Telegram, WeChat, and Feishu connections and agent bindings. |
| **Business designer** | Edit node-based design documents with previews and history. |
| **Workspace preferences** | Manage multiple workspaces, layouts, themes, display preferences, and shortcuts. |

## Who CW Agent Office Is For

- Developers running multiple Claude Code or Codex CLI agents.
- Users who want to configure collaboration and capabilities visually.
- Project owners who need terminals, tasks, files, outputs, and Git in one workspace.

---

##  Quick Start

### Release Builds

Check [Releases](https://github.com/CWLin0518/CW-Agent-Office/releases) for available build artifacts, or run from source below.

### Run from Source

Prerequisites: **Node.js 20+**, **Rust stable**, and the platform dependencies required by Tauri 2. Install and authenticate the CLI agents you intend to use.

```bash
git clone https://github.com/CWLin0518/CW-Agent-Office.git
cd CW-Agent-Office
npm install
npm run dev:tauri
```

### Start a Collaboration

1. Open a project folder as a workspace.
2. Create agents and configure their providers, models, and capabilities.
3. Drag agents onto the canvas and connect those that should communicate.
4. Launch agents from the workbench and dispatch tasks through the task center or `gto`.
5. Inspect reports, handovers, and outputs; review results through Files and Git.

---

##  Architecture

```text
CW-Agent-Office/
 apps/desktop-web/       # React + TypeScript + Vite
 apps/desktop-tauri/     # Tauri shell
 crates/                # Rust domain modules
 packages/shared-types/ # Shared contracts
 tools/gto/             # Agent communication CLI
 scripts/               # Development, build, release
 docs/                  # Architecture, design, QA
```

Frontend features live under `apps/desktop-web/src/features/`; Rust modules provide workspace, terminal, Git, task, and agent capabilities. Workspace IDs scope project operations and agent communication.

---

##  Documentation

| Document | Location |
|---|---|
| Documentation index | [docs/README.md](docs/README.md) |
| Architecture | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Workflows | [docs/WORKFLOWS.md](docs/WORKFLOWS.md) |
| API contracts | [docs/API_CONTRACTS.md](docs/API_CONTRACTS.md) |
| Customization architecture and design | [docs/cw/README.md](docs/cw/README.md) |
| Current status and acceptance records | [docs/TODO.md](docs/TODO.md) |
| Dependency policy | [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md) |
| Release process | [docs/release-process.md](docs/release-process.md) |

---

## Project Origin and License

This project is a modified version of **[GT Office by Laplace-bit](https://github.com/Laplace-bit/GT-Office)**. It builds on the upstream desktop application, workspace, terminal, Git, and agent communication foundations, with further customization of the collaboration canvas, capability mounting, and output workflows.

Credit for the original project belongs to **Laplace-bit and the GT Office contributors**. This modified repository is maintained at [CWLin0518/CW-Agent-Office](https://github.com/CWLin0518/CW-Agent-Office) and retains the [Apache License 2.0](LICENSE).
