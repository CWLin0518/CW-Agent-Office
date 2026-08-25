import type {
  AgentCapabilityHookPreviewItem,
  AgentCapabilitySnapshot,
  DiscoveredHook,
  DiscoveredSkill,
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
  return { id: '', name: '', transport: 'stdio', command: '', args: [], env: {}, url: '', enabled: true }
}

/** Bulk on/off for every configured MCP server row — the "select all"
 * counterpart to the per-row enabled toggle in `McpServersEditor`. */
export function setAllMcpServersEnabled(
  servers: McpServerCapability[],
  enabled: boolean,
): McpServerCapability[] {
  return servers.map((server) => ({ ...server, enabled }))
}

export function createEmptyHook(): HookCapability {
  return { event: 'PreToolUse', matcher: '', command: '', note: '' }
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

/** Finds the mounted entry matching a discovered skill by `sourcePath` — the
 * one field that uniquely identifies a scanned `SKILL.md` even if two skills
 * (one workspace, one global) happen to share the same `id`. */
export function findMountedSkillEntry(
  skills: SkillCapability[],
  discovered: DiscoveredSkill,
): SkillCapability | undefined {
  return skills.find((skill) => skill.sourcePath === discovered.sourcePath)
}

/** Applies a checklist toggle for one discovered skill: checking it adds a
 * new mounted entry (or re-enables an existing disabled one); unchecking
 * flips `enabled` to `false` rather than deleting the entry — same "off
 * means unmounted, not forgotten" semantics `McpServersEditor`'s toggle
 * already uses (`docs/cw/10_P3.6-capability開發進度.md` §2 決策). */
export function toggleDiscoveredSkill(
  skills: SkillCapability[],
  discovered: DiscoveredSkill,
  checked: boolean,
): SkillCapability[] {
  const existingIndex = skills.findIndex((skill) => skill.sourcePath === discovered.sourcePath)
  if (existingIndex === -1) {
    if (!checked) {
      return skills
    }
    return [...skills, { id: discovered.id, sourcePath: discovered.sourcePath, enabled: true }]
  }
  return skills.map((skill, index) =>
    index === existingIndex ? { ...skill, enabled: checked } : skill,
  )
}

/** Bulk on/off across every scanned skill (workspace + global) plus any
 * already-mounted "unmatched" entries (source file not found) — the "select
 * all" counterpart to `toggleDiscoveredSkill`'s per-row checkbox. Mounts a
 * fresh entry for each discovered skill when turning everything on; leaves
 * an unmounted discovered skill alone when turning everything off (nothing
 * to disable). Unmatched entries only ever get their `enabled` flag flipped,
 * never removed — same "off means unmounted, not forgotten" semantics as the
 * single-row toggle. */
export function setAllSkillsEnabled(
  skills: SkillCapability[],
  discovered: DiscoveredSkill[],
  enabled: boolean,
): SkillCapability[] {
  let next = skills
  for (const item of discovered) {
    next = toggleDiscoveredSkill(next, item, enabled)
  }
  const discoveredPaths = new Set(discovered.map((item) => item.sourcePath))
  return next.map((skill) => (discoveredPaths.has(skill.sourcePath) ? skill : { ...skill, enabled }))
}

/** Mounted skills that don't match any scanned `SKILL.md` — e.g. one added
 * before this checklist existed, or whose source file has since moved/been
 * deleted. Surfaced in a separate "unmatched" section rather than silently
 * dropped so a save doesn't quietly un-mount something the user still wants. */
export function unmatchedSkillEntries(
  skills: SkillCapability[],
  discovered: DiscoveredSkill[],
): SkillCapability[] {
  const discoveredPaths = new Set(discovered.map((skill) => skill.sourcePath))
  return skills.filter((skill) => !discoveredPaths.has(skill.sourcePath))
}

/** Keyword search over a scanned skill's id/name/description — case
 * insensitive substring match, blank query matches everything. */
export function filterDiscoveredSkills(items: DiscoveredSkill[], query: string): DiscoveredSkill[] {
  const needle = query.trim().toLowerCase()
  if (!needle) {
    return items
  }
  return items.filter(
    (item) =>
      item.name.toLowerCase().includes(needle) ||
      item.id.toLowerCase().includes(needle) ||
      item.description.toLowerCase().includes(needle),
  )
}

/** Same keyword search, applied to the "unmatched" fallback rows — those
 * only carry an `id`/`sourcePath` (no scanned name/description to search). */
export function filterSkillEntries(items: SkillCapability[], query: string): SkillCapability[] {
  const needle = query.trim().toLowerCase()
  if (!needle) {
    return items
  }
  return items.filter(
    (item) => item.id.toLowerCase().includes(needle) || item.sourcePath.toLowerCase().includes(needle),
  )
}

export function isHookDraftValid(hook: HookCapability): boolean {
  return Boolean(hook.event.trim()) && Boolean(hook.command.trim())
}

/** Identity key for matching a mounted `HookCapability` against a scanned
 * `DiscoveredHook` — `HookCapability` has no id field (unlike `SkillCapability`),
 * so content itself (event + matcher + command) is the only thing that can
 * identify "the same rule". Not cryptographic (unlike the backend's
 * `HookCapability::content_hash`, which is a persisted version-lock key) —
 * this is only ever compared in-memory within one render. */
function hookContentKey(event: string, matcher: string | null | undefined, command: string): string {
  return [event, matcher ?? '', command].join('\u0000')
}

/** Finds the mounted entry matching a discovered hook by content — the hook
 * equivalent of `findMountedSkillEntry`. */
export function findMountedHookEntry(
  hooks: HookCapability[],
  discovered: DiscoveredHook,
): HookCapability | undefined {
  const key = hookContentKey(discovered.event, discovered.matcher, discovered.command)
  return hooks.find((hook) => hookContentKey(hook.event, hook.matcher, hook.command) === key)
}

/** Applies a checklist toggle for one discovered hook. Unlike
 * `toggleDiscoveredSkill`, `HookCapability` has no `enabled` flag to flip —
 * presence in the array is the only on/off signal — so unchecking removes
 * the entry outright rather than disabling it. Mounting still requires the
 * usual preview/confirm before save; this only skips the retyping.
 *
 * Seeds the mounted entry's `note` from `discovered.inferredDescription`
 * (the scanned script's leading comment) when there is one — the user can
 * still edit or clear it afterward, but starting from the script's own
 * description beats an empty note field for a hook they didn't author. */
export function toggleDiscoveredHook(
  hooks: HookCapability[],
  discovered: DiscoveredHook,
  checked: boolean,
): HookCapability[] {
  const key = hookContentKey(discovered.event, discovered.matcher, discovered.command)
  const existingIndex = hooks.findIndex((hook) => hookContentKey(hook.event, hook.matcher, hook.command) === key)
  if (checked) {
    if (existingIndex !== -1) {
      return hooks
    }
    return [
      ...hooks,
      {
        event: discovered.event,
        matcher: discovered.matcher,
        command: discovered.command,
        note: discovered.inferredDescription ?? undefined,
      },
    ]
  }
  if (existingIndex === -1) {
    return hooks
  }
  return hooks.filter((_, index) => index !== existingIndex)
}

/** Bulk on/off across every scanned hook (workspace + global) — the "select
 * all" counterpart to `toggleDiscoveredHook`'s per-row checkbox. Only ever
 * touches checklist-sourced rows; manually-typed hooks (no discovered match)
 * are left as-is since they have no on/off state of their own — they're
 * removed via their own delete button, not this bulk action. */
export function setAllHooksEnabled(
  hooks: HookCapability[],
  discovered: DiscoveredHook[],
  enabled: boolean,
): HookCapability[] {
  let next = hooks
  for (const item of discovered) {
    next = toggleDiscoveredHook(next, item, enabled)
  }
  return next
}

/** Updates the `note` of whichever mounted hook matches a discovered rule's
 * content — used by the checklist's expanded detail panel so a hook mounted
 * via the checklist (not the manual form) can still carry an annotation. */
export function updateMountedHookNote(
  hooks: HookCapability[],
  discovered: DiscoveredHook,
  note: string,
): HookCapability[] {
  const key = hookContentKey(discovered.event, discovered.matcher, discovered.command)
  return hooks.map((hook) =>
    hookContentKey(hook.event, hook.matcher, hook.command) === key ? { ...hook, note } : hook,
  )
}

/** Mounted hooks that don't match any scanned `.claude/settings.json` rule —
 * hand-typed custom hooks (still an explicitly supported path, decision 3 of
 * docs/cw/08_MCP_Hook_Skill掛載設計.md) plus anything whose source rule has
 * since changed on disk. Rendered in the manual editor below the checklist. */
export function unmatchedHookEntries(hooks: HookCapability[], discovered: DiscoveredHook[]): HookCapability[] {
  const discoveredKeys = new Set(
    discovered.map((hook) => hookContentKey(hook.event, hook.matcher, hook.command)),
  )
  return hooks.filter((hook) => !discoveredKeys.has(hookContentKey(hook.event, hook.matcher, hook.command)))
}

/** Keyword search over a scanned hook's event/matcher/command/inferred
 * description — case insensitive substring match, blank query matches
 * everything. Including the inferred description lets a search like "env"
 * find a hook whose script comment mentions ".env files" even when neither
 * the event name nor the command string itself contains that word. */
export function filterDiscoveredHooks(items: DiscoveredHook[], query: string): DiscoveredHook[] {
  const needle = query.trim().toLowerCase()
  if (!needle) {
    return items
  }
  return items.filter(
    (item) =>
      item.event.toLowerCase().includes(needle) ||
      (item.matcher ?? '').toLowerCase().includes(needle) ||
      item.command.toLowerCase().includes(needle) ||
      (item.inferredDescription ?? '').toLowerCase().includes(needle),
  )
}

/** Same keyword search, applied to the manually-entered/unmatched hooks. */
export function filterHookEntries(items: HookCapability[], query: string): HookCapability[] {
  const needle = query.trim().toLowerCase()
  if (!needle) {
    return items
  }
  return items.filter(
    (item) =>
      item.event.toLowerCase().includes(needle) ||
      (item.matcher ?? '').toLowerCase().includes(needle) ||
      item.command.toLowerCase().includes(needle),
  )
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
      name: server.name?.trim() || null,
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
      note: hook.note?.trim() || null,
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
