import { useCallback, useEffect, useMemo, useState } from 'react'
import { desktopApi } from '@shell/integration/desktop-api'
import type { AgentLink, AgentProfile, AgentRuntimeStatus } from '@shell/integration/desktop-api'
import { buildAgentCanvasGraph, type AgentCanvasGraphView } from '../model/agent-canvas-graph'

/** How often to re-poll links + runtime status while the pane is active.
 * There is no push/event mechanism for either yet (see docs/cw/04_客製化設計.md
 * §1) — this is a pane-lifetime interval, not new background infrastructure. */
const POLL_INTERVAL_MS = 8000

interface UseAgentCanvasDataResult {
  graph: AgentCanvasGraphView
  loaded: boolean
  isEmpty: boolean
  commitNodePosition: (agentId: string, position: { x: number; y: number }) => void
}

/** Data loading + polling for agent-canvas, kept out of the presentational
 * `AgentCanvasPane` component — mirrors `useDesignerDocumentState`'s split
 * between a controller hook and a rendering component. */
export function useAgentCanvasData(
  workspaceId: string | null,
  active: boolean,
): UseAgentCanvasDataResult {
  const [agents, setAgents] = useState<AgentProfile[]>([])
  const [links, setLinks] = useState<AgentLink[]>([])
  const [statuses, setStatuses] = useState<AgentRuntimeStatus[]>([])
  const [loaded, setLoaded] = useState(false)

  const reload = useCallback(async () => {
    if (!workspaceId) return
    try {
      const [agentsResponse, linksResponse, statusResponse] = await Promise.all([
        desktopApi.agentList(workspaceId),
        desktopApi.agentCanvasListLinks(workspaceId),
        desktopApi.agentCanvasRuntimeStatus(workspaceId),
      ])
      setAgents(agentsResponse.agents)
      setLinks(linksResponse.links)
      setStatuses(statusResponse.statuses)
      setLoaded(true)
    } catch {
      // Best-effort refresh — keep the previous graph on a transient failure
      // rather than clearing the canvas.
    }
  }, [workspaceId])

  useEffect(() => {
    if (!active || !workspaceId) return
    // Deferred (not called synchronously here) — reload() runs inside these
    // timer callbacks, which is the "subscribe to an external system" shape
    // effects are meant to have, rather than a direct setState-on-mount call.
    const tick = () => void reload()
    const initial = window.setTimeout(tick, 0)
    const interval = window.setInterval(tick, POLL_INTERVAL_MS)
    return () => {
      window.clearTimeout(initial)
      window.clearInterval(interval)
    }
  }, [active, workspaceId, reload])

  const graph = useMemo(() => buildAgentCanvasGraph(agents, links, statuses), [agents, links, statuses])

  const commitNodePosition = useCallback(
    (agentId: string, position: { x: number; y: number }) => {
      setAgents((previous) =>
        previous.map((agent) =>
          agent.id === agentId ? { ...agent, layoutX: position.x, layoutY: position.y } : agent,
        ),
      )
      if (!workspaceId) return
      void desktopApi.agentCanvasSetLayout(workspaceId, agentId, position.x, position.y)
    },
    [workspaceId],
  )

  return { graph, loaded, isEmpty: loaded && agents.length === 0, commitNodePosition }
}
