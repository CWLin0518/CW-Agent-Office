/**
 * Graph model for agent-canvas (docs/cw/04_客製化設計.md §1, P4 MVP): agents
 * as nodes, `agent_links` rows as edges. Mirrors the shape of
 * `business-designer/model/designer-graph.ts` (grid fallback layout, pure
 * `(agents, links, statuses) -> view` builder) but with its own node/edge
 * data — the two features intentionally do not share a data model, only the
 * canvas rendering engine (`@/components/graph-canvas`).
 */

import type {
  AgentLink,
  AgentProfile,
  AgentRuntimeState,
  AgentRuntimeStatus,
} from '@shell/integration/desktop-api'
import type { GraphCanvasEdge, GraphCanvasNode } from '@/components/graph-canvas'

/** Width / height for an agent node card, in canvas (math) units. */
export const AGENT_NODE_WIDTH = 220
export const AGENT_NODE_HEIGHT = 92
const NODE_HSPACING = 64
const NODE_VSPACING = 56

export interface AgentCanvasNodeData {
  agent: AgentProfile
  runtimeState: AgentRuntimeState
}

export interface AgentCanvasEdgeData {
  link: AgentLink
}

export interface AgentCanvasGraphView {
  nodes: GraphCanvasNode<AgentCanvasNodeData>[]
  edges: GraphCanvasEdge<AgentCanvasEdgeData>[]
  bounds: { width: number; height: number }
}

/** Pick a column count that keeps the auto-layout grid reasonably square. */
function pickColumnCount(count: number): number {
  if (count <= 1) return 1
  if (count <= 4) return 2
  if (count <= 9) return 3
  return Math.ceil(Math.sqrt(count))
}

function resolveNodePosition(
  agent: AgentProfile,
  index: number,
  columns: number,
): { x: number; y: number } {
  if (
    typeof agent.layoutX === 'number' &&
    Number.isFinite(agent.layoutX) &&
    typeof agent.layoutY === 'number' &&
    Number.isFinite(agent.layoutY)
  ) {
    return { x: Math.max(0, agent.layoutX), y: Math.max(0, agent.layoutY) }
  }
  const col = index % columns
  const row = Math.floor(index / columns)
  return {
    x: col * (AGENT_NODE_WIDTH + NODE_HSPACING) + NODE_HSPACING,
    y: row * (AGENT_NODE_HEIGHT + NODE_VSPACING) + NODE_VSPACING,
  }
}

export function buildAgentCanvasGraph(
  agents: AgentProfile[],
  links: AgentLink[],
  statuses: AgentRuntimeStatus[],
): AgentCanvasGraphView {
  const statusByAgentId = new Map(statuses.map((status) => [status.agentId, status.state]))
  const columns = pickColumnCount(agents.length)

  let maxX = 0
  let maxY = 0
  const nodes: GraphCanvasNode<AgentCanvasNodeData>[] = agents.map((agent, index) => {
    const position = resolveNodePosition(agent, index, columns)
    maxX = Math.max(maxX, position.x + AGENT_NODE_WIDTH)
    maxY = Math.max(maxY, position.y + AGENT_NODE_HEIGHT)
    return {
      id: agent.id,
      x: position.x,
      y: position.y,
      data: { agent, runtimeState: statusByAgentId.get(agent.id) ?? 'unknown' },
    }
  })

  const agentIds = new Set(agents.map((agent) => agent.id))
  const edges: GraphCanvasEdge<AgentCanvasEdgeData>[] = links
    .filter((link) => agentIds.has(link.fromAgentId) && agentIds.has(link.toAgentId))
    .map((link) => ({ fromId: link.fromAgentId, toId: link.toAgentId, data: { link } }))

  return {
    nodes,
    edges,
    bounds: {
      width: Math.max(maxX + NODE_HSPACING, 800),
      height: Math.max(maxY + NODE_VSPACING, 480),
    },
  }
}
