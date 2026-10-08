# Acceptance evidence - 2026-10-09

Baseline: HEAD 1a152cf, version 0.7.5, including uncommitted fixes. Windows 10 Home 10.0.19045; Node v24.19.0; Claude CLI 2.1.294; Codex CLI 0.161.0.

## Final acceptance status

On 2026-10-09, the user explicitly confirmed that all seven items formerly listed under "已實作，仍需補驗收證據" were successfully verified. These acceptance items are now closed based on the user's report. No additional per-item logs, screenshots, or platform/version details were supplied. This update records user acceptance; it does not claim that the agent independently reran desktop or model E2E verification.

## First-round agent evidence (historical)

513 tests passed in the first round. At that time, all seven groups were only partially verified by the agent. The Pending column and blockers below describe that earlier round; the subsequent user confirmation above supersedes their acceptance status. Code and test-infrastructure observations remain separate engineering follow-ups until any later fixes are checked.

| Group | Evidence | Pending |
| --- | --- | --- |
| Global Skills/Hooks switch | SQLite defaults and switch persistence after reopening; mounts unchanged; Codex Skills/Hooks saved and reloaded; materialization tests | Desktop badges, save/reopen, actual Claude global-source exclusion and manual mounts |
| Canvas and capabilities | Graph models; link CRUD, position/color/bidirectional persistence; authorization matches delivery targets; workspace/current-session runtime isolation | Desktop gestures, Subagent, instances, undo, capability toggles, actual dispatch |
| Collaboration context | Initial builder tests and launch injection present | Real delegation, resumed-session context, topology refresh |
| Session Boundary and output collection | Joint instructions do not conflict; suppression precedence; relaunch models | Actual boundary recognition, restart and new session in desktop |
| Claude/Codex capabilities | Materialization tests; actual isolated Codex profile merges base and overlay MCP; resume/fork parse profile flags | Actual MCP calls, Skills loading, Hooks triggering, custom launches and multi-workspace E2E |
| Terminal and packaging | Actual Windows 10 PowerShell PTY output; renderer/replay/cache/recovery/input/resize models; ConPTY configuration tests | WebView long output, scrollback, resize/switch/recovery, packaged sideload, Windows 11 and macOS |
| CLI updates and session names | Update planning/process blocking tests; workspace-scoped naming and first-task naming; protocol-reply filtering; installed CLI versions | Actual installation/update and desktop naming/blocking workflows |

## Commands and results

- cargo test -p gt-agent -p gt-task -p gt-agent-session --lib: 84 + 13 + 84 = 181 passed.
- cargo test -p gt-storage --lib: 26 passed.
- cargo test -p gt-terminal: 5 library + 7 integration = 12 passed. Unix-only tests are not compiled on Windows.
- cargo test -p gt-tools --lib: 27 passed.
- Focused frontend compilation and node --test --test-reporter=spec: 267 passed.
- cargo clippy -p gt-storage -p gt-terminal --all-targets -- -D warnings: passed.
- Modified Rust files rustfmt and git diff --check: passed.
- codex -p qa_overlay mcp list --json: actual CLI listed both qa_base from config.toml and qa_overlay from qa_overlay.config.toml in isolated CODEX_HOME. resume --help and fork --help with that profile exited 0. No MCP server or model was invoked. Original CODEX_HOME restored.

Frontend reproduction: temporary config extends tsconfig.tests.json and includes tests/terminal-*.test.ts, tests/station-terminal-*.test.ts, tests/agent-canvas-*.test.ts, tests/session-*.test.ts, tests/workspace-session-model.test.ts, tests/detached-terminal-bridge.test.ts. Run npx tsc -p <config> in apps/desktop-web, then execute corresponding .test-dist/tests files with node --test, excluding station-terminal-style.test.js and detached-terminal-bridge.test.js. Temporary config removed.

## Test fixes, gaps and blockers

- Windows PTY test previously hardcoded /bin/bash. It now uses the platform shell and a split output marker to avoid passing on echoed input; receive timeouts retry within the overall deadline. Actual PTY test passed.
- Stale storage test expected Codex Skills/Hooks rejection. Replaced with save/reopen verification under the current contract; added global-switch persistence coverage. No product behavior changed in this verification round.
- Full frontend npx tsc -p tsconfig.tests.json has 9 diagnostics from removed role modules/fields and obsolete mapAgentProfileToStation arguments in role-management-model, station-delete-binding-cleanup-model, station-scope-mapping and workspace-terminal-session-store. Full suite has not passed.
- detached-terminal-bridge fails before assertions because Node cannot resolve @features alias. station-terminal-style fails because SCSS is absent from .test-dist. Neither was verified; these are harness failures, not demonstrated product failures.
- Native computer-use retries and reset still reported Computer Use native pipe is unavailable, os error 2. No desktop snapshot/screenshot or UI acceptance evidence obtained.
- session_resume_check in commands/session/mod.rs materializes capabilities but does not rebuild initial-launch collaboration context. ContinueLast/ForkLast pass None capabilities. Agent/workspace identity and current topology need an explicit resume contract. Existing-session topology refresh remains unverified.
