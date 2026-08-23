import { useEffect, useState, type DragEvent as ReactDragEvent } from 'react'
import { desktopApi } from '@shell/integration/desktop-api'
import type { AgentProfile } from '@shell/integration/desktop-api'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { setAgentCanvasDragPayload } from '../model/agent-canvas-drag'
import './AgentCanvasStandbyRail.scss'

interface AgentCanvasStandbyRailProps {
  locale: Locale
  workspaceId: string | null
  /** Whether the agent-canvas pane is the one currently shown — mirrors
   * `AgentCanvasPane`'s own `active` prop; this rail only needs to refetch
   * when it (re)becomes visible, not continuously poll like the canvas
   * itself does for links/runtime status. */
  active: boolean
}

/** Left-pane "standby" list for agent-canvas (docs/cw/04_客製化設計.md §8,
 * P4.6): every agent in the workspace, listed once regardless of whether it
 * already has a node on the canvas — dragging the same agent onto the
 * canvas more than once is the whole point of this rail (multiple visual
 * instances of one running agent). Deliberately a self-contained
 * `desktopApi.agentList` fetch (same pattern already used directly in
 * several other files — `useShellStationController.ts`,
 * `AgentGitTrackingSection.tsx`, etc.) rather than sharing state with
 * `AgentCanvasPane`'s own `useAgentCanvasData`, since the two are separate
 * shell panes (left rail vs. main canvas) with no existing shared
 * controller between them. */
export function AgentCanvasStandbyRail({ locale, workspaceId, active }: AgentCanvasStandbyRailProps) {
  const [agents, setAgents] = useState<AgentProfile[]>([])

  useEffect(() => {
    if (!active || !workspaceId) return
    let cancelled = false
    void desktopApi.agentList(workspaceId).then((response) => {
      if (!cancelled) setAgents(response.agents)
    })
    return () => {
      cancelled = true
    }
  }, [active, workspaceId])

  function handleDragStart(agentId: string, event: ReactDragEvent<HTMLDivElement>) {
    setAgentCanvasDragPayload(event.dataTransfer, agentId)
    event.dataTransfer.effectAllowed = 'copy'
  }

  return (
    <aside className="agent-canvas-standby-rail">
      <h2 className="agent-canvas-standby-rail-title">{t(locale, 'agentCanvas.standbyRail.title')}</h2>
      <p className="agent-canvas-standby-rail-subtitle">{t(locale, 'agentCanvas.standbyRail.subtitle')}</p>
      {agents.length === 0 ? (
        <p className="agent-canvas-standby-rail-empty">{t(locale, 'agentCanvas.standbyRail.empty')}</p>
      ) : (
        <ul className="agent-canvas-standby-rail-list">
          {agents.map((agent) => (
            <li key={agent.id}>
              <div
                className="agent-canvas-standby-rail-card"
                draggable
                onDragStart={(event) => handleDragStart(agent.id, event)}
                title={t(locale, 'agentCanvas.standbyRail.dragHint')}
              >
                <span className="agent-canvas-standby-rail-card-name">{agent.name || agent.id}</span>
                <span className="agent-canvas-standby-rail-card-tool">{agent.tool}</span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </aside>
  )
}
