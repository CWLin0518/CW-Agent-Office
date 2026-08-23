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

/** Width / height for a regular (top-level) agent node card, in canvas
 * (math) units. */
export const AGENT_NODE_WIDTH = 220
export const AGENT_NODE_HEIGHT = 92
/** Subagents (`agent.parentAgentId` set) render smaller — see P4.5 §1. */
export const SUBAGENT_NODE_WIDTH = 156
export const SUBAGENT_NODE_HEIGHT = 68
const NODE_HSPACING = 64
const NODE_VSPACING = 56

function nodeSizeForAgent(agent: AgentProfile): { width: number; height: number } {
  return agent.parentAgentId
    ? { width: SUBAGENT_NODE_WIDTH, height: SUBAGENT_NODE_HEIGHT }
    : { width: AGENT_NODE_WIDTH, height: AGENT_NODE_HEIGHT }
}

/**
 * A visual node on the canvas — distinct from agent identity so the same
 * agent can have more than one node (docs/cw/04_客製化設計.md §8, P4.6):
 * "the agent is still one single running thing, this is purely a visual
 * relationship depiction." Client-only (never persisted to the backend) —
 * see `useAgentCanvasData`'s `instances` state.
 *
 * `instanceId === agentId` marks the "default" instance (seeded once from
 * the pre-P4.6 one-node-per-agent world, or the first instance an agent
 * ever gets) — its position keeps coming from `agent.layoutX`/`layoutY`
 * (backend-persisted, unchanged from before P4.6). Any other instance's
 * `position` is this client's own local override.
 */
export interface CanvasNodeInstance {
  instanceId: string
  agentId: string
  position?: { x: number; y: number }
}

export interface AgentCanvasNodeData {
  instanceId: string
  agent: AgentProfile
  runtimeState: AgentRuntimeState
  /** Ordered `AgentLink.id`s of every AUTHORED link touching this agent as
   * from/to, respectively — one visual port dot per id, plus a trailing "+"
   * slot to create a new one (docs/cw/04_客製化設計.md §1). Only populated on
   * an agent's primary instance (see `primaryInstanceIdByAgentId` below);
   * any other (duplicate) instance gets empty arrays — it never anchors a
   * real edge, so it never anchors a real port dot either, just the "+".
   * Derived/ownership edges deliberately don't consume a slot — they're not
   * something a user drags to create, so they keep anchoring at the node's
   * vertical center (the pre-existing behavior, via `computePortEdgeGeometry`
   * called with no slot args). */
  outputLinkIds: string[]
  inputLinkIds: string[]
}

/** Vertical spacing (canvas units) between stacked port slots on one side of
 * a node. */
export const PORT_SLOT_SPACING = 22

/** Offset from a node's vertical CENTER for the `index`-th of `total`
 * stacked port slots — symmetric around 0, so `total === 1` (no connections
 * yet, just the "+") lands exactly on center, identical to the single fixed
 * port position every node had before per-connection slots existed. */
export function computePortSlotCenterOffset(index: number, total: number): number {
  return (index - (total - 1) / 2) * PORT_SLOT_SPACING
}

/** `link` edges are `agent_links` rows (authored, hand-drawn, interactive;
 * derived, automatic, read-only). `ownership` edges are derived purely
 * client-side from `agent.parentAgentId` — deliberately not persisted as an
 * `agent_links` row, since that table's kind enum is about communication
 * relationships, not "who manages whom" (docs/cw/04_客製化設計.md §1, P4.5). */
export type AgentCanvasEdgeData = { kind: 'link'; link: AgentLink } | { kind: 'ownership' }

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
  instance: CanvasNodeInstance,
  agent: AgentProfile,
  index: number,
  columns: number,
): { x: number; y: number } {
  if (instance.position) {
    return { x: Math.max(0, instance.position.x), y: Math.max(0, instance.position.y) }
  }
  // Default instance (or a duplicate somehow created with no position yet)
  // falls back to the agent's own backend-persisted layout, unchanged from
  // pre-P4.6 behavior.
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
  instances: CanvasNodeInstance[],
): AgentCanvasGraphView {
  const agentById = new Map(agents.map((agent) => [agent.id, agent]))
  const statusByAgentId = new Map(statuses.map((status) => [status.agentId, status.state]))
  // Instances whose agent no longer exists in this workspace are silently
  // dropped (e.g. deleted through some other flow) — nothing to render and
  // nothing meaningful to keep a position for.
  const liveInstances = instances.filter((instance) => agentById.has(instance.agentId))
  const columns = pickColumnCount(liveInstances.length)

  // `AgentLink`/ownership relationships are keyed by agentId, not
  // instanceId — an agent with multiple canvas instances gets its edges
  // anchored to a single "primary" instance (the first one found, in
  // `instances` order) rather than fanned out per instance-pair. This keeps
  // exactly one rendered line per `AgentLink` row (matching the backend,
  // which has no per-instance concept of a link) instead of an M×N
  // cross-product that would make color/disconnect/bidirectional actions
  // ambiguous about which rendered copy the user meant — an acceptable
  // trade given feature 8 is explicitly a "purely visual" depiction.
  const primaryInstanceIdByAgentId = new Map<string, string>()
  for (const instance of liveInstances) {
    if (!primaryInstanceIdByAgentId.has(instance.agentId)) {
      primaryInstanceIdByAgentId.set(instance.agentId, instance.instanceId)
    }
  }

  // One ordered port-slot list per agent, authored links only (see
  // `AgentCanvasNodeData.outputLinkIds` doc comment) — only ever populated
  // for an agent whose primary instance is actually live.
  const outputLinkIdsByAgentId = new Map<string, string[]>()
  const inputLinkIdsByAgentId = new Map<string, string[]>()
  for (const link of links) {
    if (link.kind !== 'authored') continue
    if (!primaryInstanceIdByAgentId.has(link.fromAgentId) || !primaryInstanceIdByAgentId.has(link.toAgentId)) continue
    const outputList = outputLinkIdsByAgentId.get(link.fromAgentId) ?? []
    outputList.push(link.id)
    outputLinkIdsByAgentId.set(link.fromAgentId, outputList)
    const inputList = inputLinkIdsByAgentId.get(link.toAgentId) ?? []
    inputList.push(link.id)
    inputLinkIdsByAgentId.set(link.toAgentId, inputList)
  }

  let maxX = 0
  let maxY = 0
  const nodes: GraphCanvasNode<AgentCanvasNodeData>[] = liveInstances.map((instance, index) => {
    const agent = agentById.get(instance.agentId) as AgentProfile
    const position = resolveNodePosition(instance, agent, index, columns)
    const size = nodeSizeForAgent(agent)
    maxX = Math.max(maxX, position.x + size.width)
    maxY = Math.max(maxY, position.y + size.height)
    const isPrimaryInstance = primaryInstanceIdByAgentId.get(agent.id) === instance.instanceId
    return {
      id: instance.instanceId,
      x: position.x,
      y: position.y,
      width: size.width,
      height: size.height,
      data: {
        instanceId: instance.instanceId,
        agent,
        runtimeState: statusByAgentId.get(agent.id) ?? 'unknown',
        outputLinkIds: isPrimaryInstance ? outputLinkIdsByAgentId.get(agent.id) ?? [] : [],
        inputLinkIds: isPrimaryInstance ? inputLinkIdsByAgentId.get(agent.id) ?? [] : [],
      },
    }
  })

  const linkEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = links
    .filter((link) => primaryInstanceIdByAgentId.has(link.fromAgentId) && primaryInstanceIdByAgentId.has(link.toAgentId))
    .map((link) => ({
      fromId: primaryInstanceIdByAgentId.get(link.fromAgentId) as string,
      toId: primaryInstanceIdByAgentId.get(link.toAgentId) as string,
      data: { kind: 'link', link },
    }))

  const ownershipEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = agents
    .filter(
      (agent) =>
        agent.parentAgentId &&
        primaryInstanceIdByAgentId.has(agent.parentAgentId) &&
        primaryInstanceIdByAgentId.has(agent.id),
    )
    .map((agent) => ({
      fromId: primaryInstanceIdByAgentId.get(agent.parentAgentId as string) as string,
      toId: primaryInstanceIdByAgentId.get(agent.id) as string,
      data: { kind: 'ownership' },
    }))

  const edges = [...linkEdges, ...ownershipEdges]

  return {
    nodes,
    edges,
    bounds: {
      width: Math.max(maxX + NODE_HSPACING, 800),
      height: Math.max(maxY + NODE_VSPACING, 480),
    },
  }
}

export interface PortEdgeGeometry {
  path: string
  start: { x: number; y: number }
  end: { x: number; y: number }
}

/** A wire's anchor slot on one side of a node — `total === 1` (the default)
 * anchors at the node's vertical center, matching every node's single fixed
 * port before per-connection slots existed; `total > 1` fans slots out
 * symmetrically via `computePortSlotCenterOffset`. */
export interface PortSlot {
  index: number
  total: number
}

const CENTER_SLOT: PortSlot = { index: 0, total: 1 }

/** Grasshopper-style wire geometry: always output (right side of `from`) to
 * input (left side of `to`), regardless of the nodes' relative vertical
 * position — unlike `computeQuadraticEdgeGeometry` (the shared engine's
 * generic node-to-node heuristic, which picks whichever side is closer),
 * ports have a fixed side, so the wire is a horizontal S-curve that loops
 * around when `to` sits above/below/left of `from`. Used for `link` and
 * `ownership` edges so ports and wires visually line up consistently —
 * `ownership` and derived `link` edges call this with no slot args (center
 * anchor, unchanged pre-multi-slot behavior); authored `link` edges pass
 * each side's actual slot so multiple connections fan out to their own
 * distinct port dot instead of converging on one point. */
export function computePortEdgeGeometry(
  from: { x: number; y: number; width?: number; height?: number },
  to: { x: number; y: number; width?: number; height?: number },
  fromSlot: PortSlot = CENTER_SLOT,
  toSlot: PortSlot = CENTER_SLOT,
): PortEdgeGeometry {
  const fromWidth = from.width ?? AGENT_NODE_WIDTH
  const fromHeight = from.height ?? AGENT_NODE_HEIGHT
  const toHeight = to.height ?? AGENT_NODE_HEIGHT
  const start = {
    x: from.x + fromWidth,
    y: from.y + fromHeight / 2 + computePortSlotCenterOffset(fromSlot.index, fromSlot.total),
  }
  const end = {
    x: to.x,
    y: to.y + toHeight / 2 + computePortSlotCenterOffset(toSlot.index, toSlot.total),
  }
  const offset = Math.min(120, Math.max(32, Math.abs(end.x - start.x) / 2))
  const c1 = { x: start.x + offset, y: start.y }
  const c2 = { x: end.x - offset, y: end.y }
  return {
    start,
    end,
    path: `M ${start.x} ${start.y} C ${c1.x} ${c1.y}, ${c2.x} ${c2.y}, ${end.x} ${end.y}`,
  }
}
