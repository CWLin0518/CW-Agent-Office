import type {
  AgentCapabilityHookPreviewItem,
  AgentCapabilitySnapshot,
  HookCapability,
  McpServerCapability,
  SkillCapability,
} from '../../shell/integration/desktop-api.js'

export type CapabilitySubTab = 'mcp' | 'skills' | 'hooks'

/** Codex agents only get `mcpServers` materialized (v1 scope,
 * docs/cw/08_MCP_Hook_Skill掛載設計.md §2.1 決策2) — the Skills/Hooks
 * sub-tabs must be disabled, not just "empty," for a Codex agent. */
export function isSkillsOrHooksSupportedForToolKind(toolKind: string): boolean {
  return toolKind === 'claude'
}

export function createEmptyMcpServer(): McpServerCapability {
  return { id: '', transport: 'stdio', command: '', args: [], env: {}, url: '' }
}

export function createEmptySkill(): SkillCapability {
  return { id: '', sourcePath: '', enabled: true }
}

export function createEmptyHook(): HookCapability {
  return { event: 'PreToolUse', matcher: '', command: '' }
}

export function isMcpServerDraftValid(server: McpServerCapability): boolean {
  if (!server.id.trim()) {
    return false
  }
  if (server.transport === 'stdio') {
    return Boolean(server.command?.trim())
  }
  return Boolean(server.url?.trim())
}

export function isSkillDraftValid(skill: SkillCapability): boolean {
  return Boolean(skill.id.trim()) && Boolean(skill.sourcePath.trim())
}

export function isHookDraftValid(hook: HookCapability): boolean {
  return Boolean(hook.event.trim()) && Boolean(hook.command.trim())
}

/** Args are edited as one newline-separated textarea, matching how the
 * existing Permissions tab edits its `string[]` fields
 * (`updateShellDeniedCommands` et al. in StationManageModal.tsx). */
export function parseArgsTextarea(rawValue: string): string[] {
  return rawValue
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
}

export function formatArgsTextarea(args: string[]): string {
  return args.join('\n')
}

/** Env vars are edited as `KEY=value` lines, one per line — a plain-text
 * shape rather than a key/value mini-table, kept intentionally simple for a
 * v1 manual-entry-only form (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.5).
 * Lines without `=` are ignored (not surfaced as a hard form error) so a
 * user mid-typing a line never gets blocked from saving other rows. */
export function parseEnvTextarea(rawValue: string): Record<string, string> {
  const env: Record<string, string> = {}
  for (const line of rawValue.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) {
      continue
    }
    const separatorIndex = trimmed.indexOf('=')
    if (separatorIndex <= 0) {
      continue
    }
    const key = trimmed.slice(0, separatorIndex).trim()
    const value = trimmed.slice(separatorIndex + 1).trim()
    if (key) {
      env[key] = value
    }
  }
  return env
}

export function formatEnvTextarea(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join('\n')
}

/** Normalizes a draft snapshot right before sending it to
 * `agentCapabilitySave` — drops any in-progress-but-incomplete rows (an
 * empty "add new server" row left in the list) rather than rejecting the
 * whole save, and trims `matcher`/`url`/`command` down to `null` when
 * blank so the backend doesn't persist empty strings where `Option<String>`
 * means "unset." */
export function buildSavableCapabilitySnapshot(
  draft: AgentCapabilitySnapshot,
): AgentCapabilitySnapshot {
  return {
    mcpServers: draft.mcpServers.filter(isMcpServerDraftValid).map((server) => ({
      ...server,
      id: server.id.trim(),
      command: server.transport === 'stdio' ? (server.command?.trim() || null) : null,
      url: server.transport === 'stdio' ? null : (server.url?.trim() || null),
    })),
    skills: draft.skills.filter(isSkillDraftValid).map((skill) => ({
      ...skill,
      id: skill.id.trim(),
      sourcePath: skill.sourcePath.trim(),
    })),
    hooks: draft.hooks.filter(isHookDraftValid).map((hook) => ({
      ...hook,
      event: hook.event.trim(),
      matcher: hook.matcher?.trim() || null,
      command: hook.command.trim(),
    })),
  }
}

/** Whether the preview-and-confirm modal must be shown before saving —
 * required whenever there's at least one hook to preview
 * (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.5 決策3: preview is never skipped
 * once a hook exists, even if every hash is already confirmed). */
export function requiresHookPreviewBeforeSave(hooks: HookCapability[]): boolean {
  return hooks.length > 0
}

export function unconfirmedHookHashes(items: AgentCapabilityHookPreviewItem[]): string[] {
  return items.filter((item) => !item.alreadyConfirmed).map((item) => item.hash)
}
