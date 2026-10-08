<div align="center">

# GT Office · CW-Agent-Office

### A visual desktop workspace for Claude Code and Codex CLI

Manage agents, connect collaborators, mount capabilities, and inspect deliverables alongside terminals, files, and Git.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

[Downloads](https://github.com/CWLin0518/CW-Agent-Office/releases) · [Documentation](docs/README.md) · [繁體中文](README_CN.md)

</div>

This repository is the **CW-Agent-Office fork of [GT Office](https://github.com/Laplace-bit/GT-Office)**. Built with Tauri, React, and Rust, it brings terminal-based AI agents into a persistent, workspace-scoped desktop environment. Current source version: **0.7.6**.

## Agent Canvas

![Agent Canvas with collaboration links, capability mounts, and output nodes](docs/assets/agent-canvas.jpg)

- Drag standby agents onto the canvas; one agent can have multiple visual instances.
- Draw collaboration connections to define which agents can dispatch tasks, report progress, and hand over work.
- Inspect mounted **MCP servers, Skills, and Hooks**, toggle MCP servers, and view enabled skill counts.
- Expand **Output** nodes to browse collected deliverables and open previews.
- Organize nodes with selection, alignment, colors, zoom, and undo; restore saved layouts when reopening the workspace.

Connections define communication permissions; they do not automatically execute a workflow. Self-messages and **communicate with all** are exceptions to the connection requirement, while explicit communication denial still takes precedence.

## Current features

| Area | Available capabilities |
|---|---|
| Workspaces | Open project directories, switch workspace tabs, and persist agent configuration and layout. |
| Agent workstations | Launch Claude Code or Codex CLI in embedded terminals; configure prompts, models, and launch commands; install or update supported CLIs. |
| Sessions | Browse session history, rename sessions, and continue or fork supported CLI sessions. |
| Capability mounts | Configure agent-specific MCP, Skills, and Hooks for Claude and Codex. Claude additionally supports excluding global Skill/Hook sources; Codex has no equivalent global-source switch. |
| Collaboration | Use the bundled `gto` CLI for discovery, task dispatch, progress replies, handovers, inboxes, and task threads. |
| Tasks and change feed | Inspect task progress, collaboration messages, and workspace changes. |
| Files | Browse and search files, edit text with Monaco, and preview Markdown, images, PDFs, audio, and video. |
| Git | Inspect status and diffs, browse the commit graph, manage branches and stashes, and use commit-history actions. |
| Channels | Configure Telegram, WeChat, and Feishu adapters and bind agents to external channels; service credentials and setup are required. |
| Business Designer | Structure requirements as typed blocks, inspect derived relationships and validation gaps, and review proposed agent patches. |
| Desktop settings | Chinese/English UI, light/dark themes, keyboard shortcuts, and workspace/window layout controls. |

Activity indicators reflect recent terminal output and session lifecycle events. They do not reliably distinguish silent reasoning from waiting for input. Saved configuration and session history do not mean a terminated process keeps running.

## Quick start

### Install a desktop build

Choose an available asset from [this fork’s releases](https://github.com/CWLin0518/CW-Agent-Office/releases). The build workflow targets **Windows, macOS, and Linux**; available assets depend on each release. See [release notes](docs/releases/) for packaging details.

Claude Code and Codex CLI require their own installation and authentication. Use the app’s provider setup or an existing CLI installation, then authenticate with your chosen provider.

### Run from source

Prerequisites: **Node.js 20+**, **Rust stable**, Git, and the platform build environment for Tauri 2. Windows needs MSVC C++ build tools and WebView2; macOS needs Xcode command-line tools; Linux needs WebKitGTK and the desktop libraries listed in the [release workflow](.github/workflows/release.yml).

```bash
git clone https://github.com/CWLin0518/CW-Agent-Office.git
cd CW-Agent-Office
npm ci
npm run dev:tauri
```

`npm run dev:web` starts the frontend alone; terminals, filesystem access, and other native capabilities require the desktop backend.

### Set up a collaboration workspace

1. Open a project directory as a workspace.
2. Add Claude or Codex agents and configure their prompts and models.
3. Mount the MCP servers, Skills, and Hooks each agent needs.
4. Place agents on the canvas and connect collaborators.
5. Launch their terminals, dispatch tasks, and inspect replies and Output nodes.

### Communicate with `gto`

With the desktop app and local bridge running, execute in an agent terminal:

```bash
gto directory snapshot --json
gto agent send-task --target-agent-id <agent-id> --title "Review changes" --markdown "Review the diff and report findings." --json
gto agent task-thread --task-id <returned-task-id> --json
```

Agent terminals receive `GTO_WORKSPACE_ID` and `GTO_AGENT_ID`. Elsewhere, provide the appropriate workspace and agent flags. Keep the returned `taskId` for progress replies and handovers. If `gto` is not on PATH, use `node tools/gto/bin/gto.mjs` from the repository root. See the [CLI guide](tools/gto/README.md) for commands and limitations.

## Scope and known limitations

- Supported coding-agent providers are **Claude Code and Codex CLI**; Gemini CLI support was removed.
- Some saved policy fields, including Git/VCS policy, execution timeout, and maximum steps, are not yet enforced at runtime.
- Collaboration context refresh on resumed sessions and frontend test infrastructure still have engineering follow-ups.
- Provider Descriptor/version-lock refactoring and further permission controls remain planned work.

See [current status and evidence](docs/TODO.md) for tracked gaps and acceptance records. Design proposals may describe capabilities beyond the current implementation.

## Development

```bash
npm run typecheck
cargo check --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
npm run build:tauri
```

```text
apps/desktop-web/       React UI and feature controllers
apps/desktop-tauri/     Tauri shell and feature-aligned commands
crates/                Rust domain and infrastructure modules
packages/shared-types/ Shared contracts
tools/gto/             Agent communication CLI
docs/                  Architecture, workflows, QA, and release notes
```

## Documentation and contributions

- [Documentation index](docs/README.md)
- [Architecture](docs/ARCHITECTURE.md) · [Workflows](docs/WORKFLOWS.md)
- [API contracts](docs/API_CONTRACTS.md) · [Dependency policy](docs/DEPENDENCIES.md)
- [Current status](docs/TODO.md) · [Release process](docs/release-process.md)
- [Contributing](CONTRIBUTING.md) · [Repository instructions](AGENTS.md)

Upstream issues are tracked at [Laplace-bit/GT-Office](https://github.com/Laplace-bit/GT-Office/issues), as specified in the [issue-tracker policy](docs/agents/issue-tracker.md). Distinguish upstream behavior from fork-specific changes when reporting problems.

Licensed under [Apache 2.0](LICENSE).
