import { useEffect, useState } from 'react'

import { desktopApi } from '@shell/integration/desktop-api'

import type { AgentModelOption, ManagedAgentProvider } from './agent-management-model'

/**
 * Asks the backend which models the installed `provider` CLI can run (Codex's
 * own model cache, Claude Code's version-tracking aliases, plus the user's
 * configured default). Re-fetched each time `enabled` turns on so the list
 * follows CLI upgrades; `null` until loaded or outside the desktop runtime,
 * in which case callers fall back to the static list.
 */
export function useAgentModelOptions(
  provider: ManagedAgentProvider,
  enabled: boolean,
): AgentModelOption[] | null {
  // Keyed by provider so a provider switch never shows the previous
  // provider's list while the new one loads.
  const [loaded, setLoaded] = useState<{ provider: ManagedAgentProvider; options: AgentModelOption[] } | null>(
    null,
  )

  useEffect(() => {
    if (!enabled || !desktopApi.isTauriRuntime()) {
      return
    }
    let cancelled = false
    desktopApi
      .agentModelOptions(provider === 'claude' ? 'ClaudeCode' : 'Codex')
      .then((items) => {
        if (!cancelled) {
          setLoaded({
            provider,
            options: items.map(({ value, label, shortLabel }) => ({ value, label, shortLabel })),
          })
        }
      })
      .catch((error) => {
        console.warn('failed to load agent model options', error)
      })
    return () => {
      cancelled = true
    }
  }, [enabled, provider])

  return enabled && loaded?.provider === provider ? loaded.options : null
}
