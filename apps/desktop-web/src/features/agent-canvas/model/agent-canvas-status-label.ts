import { t, type Locale } from '@shell/i18n/ui-locale'
import type { AgentRuntimeState } from '@shell/integration/desktop-api'

export function statusLabel(locale: Locale, state: AgentRuntimeState): string {
  switch (state) {
    case 'active':
      return t(locale, 'agentCanvas.status.active')
    case 'idle':
      return t(locale, 'agentCanvas.status.idle')
    case 'offline':
      return t(locale, 'agentCanvas.status.offline')
    default:
      return t(locale, 'agentCanvas.status.unknown')
  }
}
