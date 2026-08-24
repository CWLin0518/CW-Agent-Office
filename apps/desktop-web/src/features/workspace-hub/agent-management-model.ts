export type ManagedAgentProvider = 'claude' | 'codex'

export interface AgentProviderOption {
  key: ManagedAgentProvider
  label: string
  promptFileName: string
}

export interface AgentProviderSnapshot {
  agent: string
  installStatus: {
    installed: boolean
  }
  configStatus: string
}

function normalizeAgentWorkdir(value: string | null | undefined): string {
  const normalized = (value ?? '')
    .trim()
    .replace(/\\/g, '/')
    .replace(/^\/+/, '')
    .replace(/\/+/g, '/')
    .replace(/\/$/, '')
  if (!normalized || normalized === '.') {
    return '.'
  }
  const segments = normalized.split('/').filter((segment) => segment && segment !== '.')
  if (segments.length === 0) {
    return '.'
  }
  return segments.join('/')
}

function normalizeSegment(value: string): string {
  const normalized = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '')
  return normalized || 'agent'
}

export function buildDefaultAgentWorkdir(_name: string): string {
  return '.'
}

export function buildSuggestedAgentWorkdir(name: string): string {
  return `.gtoffice/${normalizeSegment(name)}`
}

export function resolvePromptFileNameForProvider(provider: ManagedAgentProvider): string {
  switch (provider) {
    case 'claude':
      return 'CLAUDE.md'
    case 'codex':
    default:
      return 'AGENTS.md'
  }
}

export function resolvePromptFileRelativePathForProvider(
  provider: ManagedAgentProvider,
  workdir: string | null | undefined,
): string {
  const fileName = resolvePromptFileNameForProvider(provider)
  const normalizedWorkdir = normalizeAgentWorkdir(workdir)
  if (normalizedWorkdir === '.') {
    return fileName
  }
  return `${normalizedWorkdir}/${fileName}`
}

export function isWorkspaceRootAgentWorkdir(workdir: string | null | undefined): boolean {
  return normalizeAgentWorkdir(workdir) === '.'
}

export function resolveProviderLabel(provider: ManagedAgentProvider): string {
  switch (provider) {
    case 'claude':
      return 'Claude Code'
    case 'codex':
    default:
      return 'Codex CLI'
  }
}

export function resolveManagedProviderKey(tool: string | null | undefined): ManagedAgentProvider {
  const normalized = tool?.trim().toLowerCase() ?? ''
  if (normalized.includes('claude')) {
    return 'claude'
  }
  return 'codex'
}

export interface AgentModelOption {
  value: string
  label: string
  /** Compact form for space-constrained UI (canvas node badges, terminal
   * headers) — the full `label` includes the versioned model id in
   * parentheses, which is too long for those spots. */
  shortLabel: string
}

const CLAUDE_MODEL_OPTIONS: AgentModelOption[] = [
  { value: 'claude-opus-5', label: 'Opus 5 (claude-opus-5)', shortLabel: 'Opus 5' },
  { value: 'claude-sonnet-5', label: 'Sonnet 5 (claude-sonnet-5)', shortLabel: 'Sonnet 5' },
  {
    value: 'claude-haiku-4-5-20251001',
    label: 'Haiku 4.5 (claude-haiku-4-5-20251001)',
    shortLabel: 'Haiku 4.5',
  },
]

const CODEX_MODEL_OPTIONS: AgentModelOption[] = [
  {
    value: 'gpt-5.1-codex-max',
    label: 'GPT-5.1 Codex Max (gpt-5.1-codex-max)',
    shortLabel: 'GPT-5.1 Codex Max',
  },
  { value: 'gpt-5.1-codex', label: 'GPT-5.1 Codex (gpt-5.1-codex)', shortLabel: 'GPT-5.1 Codex' },
  { value: 'gpt-5.1', label: 'GPT-5.1 (gpt-5.1)', shortLabel: 'GPT-5.1' },
  { value: 'o3', label: 'o3', shortLabel: 'o3' },
]

export function resolveModelOptionsForProvider(provider: ManagedAgentProvider): AgentModelOption[] {
  return provider === 'claude' ? CLAUDE_MODEL_OPTIONS : CODEX_MODEL_OPTIONS
}

const LAUNCH_COMMAND_MODEL_FLAG_PATTERN = /\s*(?:--model|-m)(?:=|\s+)(?:"[^"]*"|'[^']*'|\S+)/g
const LAUNCH_COMMAND_MODEL_FLAG_CAPTURE_PATTERN = /(?:--model|-m)(?:=|\s+)("[^"]*"|'[^']*'|\S+)/

/** Reads the `--model`/`-m` value already present in a raw launch command, if any. */
export function extractModelFromLaunchCommand(launchCommand: string | null | undefined): string {
  const match = (launchCommand ?? '').match(LAUNCH_COMMAND_MODEL_FLAG_CAPTURE_PATTERN)
  if (!match) {
    return ''
  }
  return match[1].replace(/^["']|["']$/g, '')
}

/**
 * Rewrites a raw launch command so its `--model` flag matches `model`
 * (removing the flag entirely when `model` is empty). Falls back to the bare
 * `provider` binary name as the base command when `launchCommand` is blank,
 * and collapses back to `''` when the result would just be that bare binary
 * name, so the form's placeholder can show through again.
 */
export function applyModelToLaunchCommand(
  launchCommand: string | null | undefined,
  provider: ManagedAgentProvider,
  model: string,
): string {
  const stripped = (launchCommand ?? '').replace(LAUNCH_COMMAND_MODEL_FLAG_PATTERN, '').trim()
  if (!model) {
    return stripped === provider ? '' : stripped
  }
  const base = stripped || provider
  return `${base} --model ${model}`
}

/** Prefills the model selector from an existing agent's launch command, if it matches a known option. */
export function resolveInitialAgentModel(
  provider: ManagedAgentProvider,
  launchCommand: string | null | undefined,
): string {
  const extracted = extractModelFromLaunchCommand(launchCommand)
  if (!extracted) {
    return ''
  }
  const isKnown = resolveModelOptionsForProvider(provider).some((option) => option.value === extracted)
  return isKnown ? extracted : ''
}

/**
 * Compact "which model is this agent running" label for canvas nodes and
 * terminal headers — read back out of the launch command's `--model` flag
 * (see `applyModelToLaunchCommand`), since model isn't its own persisted
 * field. Returns `''` when no model flag is set (the CLI's own default
 * applies), so callers can skip rendering a badge entirely.
 */
export function resolveAgentModelDisplayLabel(
  tool: string | null | undefined,
  launchCommand: string | null | undefined,
): string {
  const value = extractModelFromLaunchCommand(launchCommand)
  if (!value) {
    return ''
  }
  const provider = resolveManagedProviderKey(tool)
  const known = resolveModelOptionsForProvider(provider).find((option) => option.value === value)
  return known ? known.shortLabel : value
}

function isSelectableProvider(
  agent: AgentProviderSnapshot,
): agent is AgentProviderSnapshot & { agent: ManagedAgentProvider } {
  if (agent.agent !== 'claude' && agent.agent !== 'codex') {
    return false
  }
  return agent.installStatus.installed || agent.configStatus === 'configured'
}

export function resolveAvailableAgentProviders(snapshotAgents: AgentProviderSnapshot[]): AgentProviderOption[] {
  return snapshotAgents
    .filter(isSelectableProvider)
    .map((agent) => ({
      key: agent.agent,
      label: resolveProviderLabel(agent.agent),
      promptFileName: resolvePromptFileNameForProvider(agent.agent),
    }))
}
