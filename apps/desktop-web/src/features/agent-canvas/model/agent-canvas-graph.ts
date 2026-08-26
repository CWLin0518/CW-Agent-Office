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
  AgentOutputFile,
  AgentProfile,
  AgentRuntimeState,
  AgentRuntimeStatus,
  HookCapability,
  McpServerCapability,
  SkillCapability,
} from '@shell/integration/desktop-api'
import type { GraphCanvasEdge, GraphCanvasNode } from '@/components/graph-canvas'

/** Width / height for a regular (top-level) agent node card, in canvas
 * (math) units. */
export const AGENT_NODE_WIDTH = 220
export const AGENT_NODE_HEIGHT = 92
/** Subagents (`agent.parentAgentId` set) render smaller — see P4.5 §1. */
export const SUBAGENT_NODE_WIDTH = 156
export const SUBAGENT_NODE_HEIGHT = 68
/** MCP-mount nodes — one per (agent, mounted MCP server) pair, smaller still
 * since they only ever show a name + on/off toggle. */
export const MCP_NODE_WIDTH = 160
export const MCP_NODE_HEIGHT = 48
const MCP_NODE_VSPACING = 12
/** Skill/Hook-mount nodes — one PER AGENT (not per item, unlike MCP), a
 * collapsed dropdown header that expands in place to list every mounted
 * SKILL/HOOK (docs/cw/10_P3.6-capability開發進度.md §2.3's MCP node-ification,
 * extended per a later request: SKILL/HOOK summarize as one node each rather
 * than fanning out one node per item). Same collapsed footprint as an MCP
 * node so the three kinds stack tidily in the same left-side column. */
export const SKILL_NODE_WIDTH = 160
export const SKILL_NODE_HEIGHT = 48
export const HOOK_NODE_WIDTH = 160
export const HOOK_NODE_HEIGHT = 48
/** Output-list node (docs/cw/14_Agent輸出清單化.md §4.2) — same collapsed
 * footprint as the MCP/Skill/Hook mount nodes, but anchored to the agent's
 * OUTPUT (right) side instead of its input (left) side, since it summarizes
 * what the agent produced rather than what's mounted on it. */
export const OUTPUT_NODE_WIDTH = 160
export const OUTPUT_NODE_HEIGHT = 48
const NODE_HSPACING = 64
const NODE_VSPACING = 56

/** The synthetic node/position-storage id for one agent's one mounted MCP
 * server — shared between `buildAgentCanvasGraph` (node/edge construction)
 * and the controller (`useAgentCanvasData`'s client-only position storage +
 * `commitMcpNodePosition`/`setMcpServerEnabled`), so both sides always
 * agree on the same id for the same (agent, server) pair. */
export function buildMcpNodeId(agentId: string, serverId: string): string {
  return `mcp:${agentId}:${serverId}`
}

/** One agent has at most one Skill-mount node and one Hook-mount node (unlike
 * MCP's per-server node), so these need no third id component. */
export function buildSkillNodeId(agentId: string): string {
  return `skill:${agentId}`
}

export function buildHookNodeId(agentId: string): string {
  return `hook:${agentId}`
}

/** Sibling of `buildSkillNodeId`/`buildHookNodeId` for the output-list node
 * (docs/cw/14_Agent輸出清單化.md §4.2) — one per agent, same "at most one"
 * rule. */
export function buildOutputNodeId(agentId: string): string {
  return `output:${agentId}`
}

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

export interface AgentCanvasAgentNodeData {
  kind: 'agent'
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
  /** Live `agent.id`s whose `parentAgentId` is this agent, in stable order —
   * one bottom-center port dot per id (docs request: subagents connect from
   * the parent's bottom point, not the left/right Grasshopper-style anchor
   * every other edge kind uses). Only populated on an agent's primary
   * instance, same reasoning as `outputLinkIds`/`inputLinkIds` above. */
  childAgentIds: string[]
  /** `buildMcpNodeId(agentId, server.id)` for every MCP server currently
   * mounted on this agent, in the same order as `mcpServersByAgentId`'s
   * list — fanned into the SAME left-side port stack as `inputLinkIds`
   * (link items first, then these), so a mounted tool reads as another
   * kind of "input" alongside authored-link connections, just visually
   * distinguished. Only populated on the primary instance, same as above. */
  mcpMountIds: string[]
  /** `buildSkillNodeId(agentId)` when this agent has at least one mounted
   * Skill, else `null` (no node — an agent with zero skills gets no
   * dropdown node at all, same "presence == has something to show" rule as
   * MCP mounts). Fanned into the same combined left-side stack, after the
   * MCP mounts — see `computeAgentInputPortLayout`. Only populated on the
   * primary instance, same as `mcpMountIds`. */
  skillMountId: string | null
  /** `buildHookNodeId(agentId)`, same "null when nothing mounted" rule as
   * `skillMountId`, fanned in after it. */
  hookMountId: string | null
  /** `buildOutputNodeId(agentId)` when this agent has at least one file in
   * its fixed output directory (docs/cw/14_Agent輸出清單化.md §3.1/§4.2),
   * else `null` — same "presence == has something to show" rule as
   * `skillMountId`/`hookMountId`. Unlike those three, this fans into the
   * agent's OUTPUT-side port stack (with `outputLinkIds`), not the input-side
   * one — see `computeAgentOutputPortLayout`. Only populated on the primary
   * instance, same as the other mount ids. */
  outputMountId: string | null
}

/** One MCP-server-mount node — a draggable canvas citizen (position is
 * client-only, see `useAgentCanvasData`'s `mcpNodePositions`), auto-created
 * whenever `server` appears in its owning agent's capability snapshot and
 * removed the moment it doesn't (never orphaned on canvas after an
 * unmount). `server.enabled` (not this node's presence) is what the on/off
 * switch actually controls — disabling never deletes the mount, so the node
 * stays visible either way, just dimmed. */
export interface AgentCanvasMcpNodeData {
  kind: 'mcp'
  id: string
  agentId: string
  server: McpServerCapability
  /** Client-only display color for this node's dashed outline (right-click
   * to change, docs P4.x follow-up), same "`null`/absent == default gray"
   * convention as `AgentProfile.color`/`AgentLink.color` — see
   * `useAgentCanvasData`'s `setMountNodeColor`. */
  color?: string | null
}

/** One Skill-mount node per agent (not per item) — a collapsed dropdown
 * header (`AgentCanvasSkillNodeCard`) that expands in place to list every
 * `SkillCapability` currently mounted on this agent. Read-only: unlike the
 * MCP node's on/off switch, this canvas node has no interactive mutation —
 * enabling/disabling an individual skill stays a Capabilities-tab action
 * (decided when this node kind was designed, to keep the canvas surface from
 * duplicating that tab's per-item controls). Draggable / position
 * client-only, same as an MCP node — see `useAgentCanvasData`'s
 * `skillNodePositions`. */
export interface AgentCanvasSkillNodeData {
  kind: 'skill'
  id: string
  agentId: string
  skills: SkillCapability[]
  /** Same "right-click to change, default gray" convention as
   * `AgentCanvasMcpNodeData.color`. */
  color?: string | null
}

/** Hook sibling of `AgentCanvasSkillNodeData` — same one-node-per-agent,
 * read-only-dropdown, draggable-position shape. */
export interface AgentCanvasHookNodeData {
  kind: 'hook'
  id: string
  agentId: string
  hooks: HookCapability[]
  /** Same "right-click to change, default gray" convention as
   * `AgentCanvasMcpNodeData.color`. */
  color?: string | null
}

/** Output-list node — one per agent, summarizing every file found in that
 * agent's fixed output directory (docs/cw/14_Agent輸出清單化.md §3.1/§4.2).
 * Read-only, same shape as `AgentCanvasSkillNodeData`/`AgentCanvasHookNodeData`
 * (collapsed dropdown header expanding to a list), but anchored to the
 * agent's OUTPUT side — see `AgentCanvasAgentNodeData.outputMountId`. */
export interface AgentCanvasOutputNodeData {
  kind: 'output'
  id: string
  agentId: string
  files: AgentOutputFile[]
  /** Same "right-click to change, default gray" convention as
   * `AgentCanvasMcpNodeData.color`. */
  color?: string | null
}

export type AgentCanvasNodeData =
  | AgentCanvasAgentNodeData
  | AgentCanvasMcpNodeData
  | AgentCanvasSkillNodeData
  | AgentCanvasHookNodeData
  | AgentCanvasOutputNodeData

/** Vertical spacing (canvas units) between stacked port slots on one side of
 * a node. */
export const PORT_SLOT_SPACING = 22

// A port dot's CSS offset (`AgentCanvasPane.scss`'s `.agent-canvas-port--*`
// rules — `left`/`right`/`top`/`bottom: -5px`) is exactly `-radius` (the dot
// is 10px wide), so its CENTER lands exactly ON the node's bare edge — half
// the dot overlaps the border, half hangs outside it. Because of that,
// `computePortEdgeGeometry`/`computeVerticalPortEdgeGeometry` below need no
// extra compensation constant: `from.x + fromWidth` / `to.x` (and their
// vertical equivalents) already ARE the dot's center. A previous version of
// this file tried to keep a separate "how far the dot hangs past the edge"
// constant in sync with the CSS by hand — it drifted (twice) as the CSS
// value or unit changed underneath it. Keeping the CSS offset pinned to
// `-radius` removes the second number entirely, so there's nothing left to
// go out of sync.

/** Offset from a node's vertical CENTER for the `index`-th of `total`
 * stacked port slots — symmetric around 0, so `total === 1` (no connections
 * yet, just the "+") lands exactly on center, identical to the single fixed
 * port position every node had before per-connection slots existed. */
export function computePortSlotCenterOffset(index: number, total: number): number {
  return (index - (total - 1) / 2) * PORT_SLOT_SPACING
}

export interface AgentInputPortLayout {
  /** Combined slot count: authored-link inputs, MCP mounts, the Skill mount
   * (if any), the Hook mount (if any), plus the trailing "+". */
  total: number
  /** Slot index of the Skill mount dot, or `null` when this agent has none. */
  skillIndex: number | null
  /** Slot index of the Hook mount dot, or `null` when this agent has none. */
  hookIndex: number | null
}

/** Narrowed to just the fields the layout math needs — `InputPortSlots`
 * (`AgentCanvasNodeCard.tsx`) only ever has `linkIds`/`mcpMountIds`/
 * `skillMountId`/`hookMountId` as separate props, not a full
 * `AgentCanvasAgentNodeData`, so this stays structurally satisfied by both
 * that call site and `renderEdge`'s (which does have the full node). */
export interface AgentInputPortLayoutInput {
  inputLinkIds: string[]
  mcpMountIds: string[]
  skillMountId: string | null
  hookMountId: string | null
}

/** Single source of truth for how an agent's combined left-side port stack
 * is laid out (authored-link inputs, then MCP mounts, then the Skill mount,
 * then the Hook mount, then the trailing "+") — used by both
 * `AgentCanvasNodeCard`'s `InputPortSlots` (rendering the dots) and
 * `AgentCanvasPane`'s `renderEdge` (anchoring `mcp-mount`/`skill-mount`/
 * `hook-mount`/authored-link wires to that exact same slot), so the two can
 * never drift out of sync about which slot is which — the same
 * two-call-sites-must-agree hazard the port-dot-offset comment above
 * describes for `computePortEdgeGeometry`. */
export function computeAgentInputPortLayout(node: AgentInputPortLayoutInput): AgentInputPortLayout {
  const mcpStart = node.inputLinkIds.length
  const skillIndex = node.skillMountId !== null ? mcpStart + node.mcpMountIds.length : null
  const hookIndex =
    node.hookMountId !== null ? mcpStart + node.mcpMountIds.length + (node.skillMountId !== null ? 1 : 0) : null
  const total =
    node.inputLinkIds.length +
    node.mcpMountIds.length +
    (node.skillMountId !== null ? 1 : 0) +
    (node.hookMountId !== null ? 1 : 0) +
    1
  return { total, skillIndex, hookIndex }
}

export interface AgentOutputPortLayout {
  /** Combined slot count: authored-link outputs, plus the output-mount node
   * (if any), plus the trailing "+". */
  total: number
  /** Slot index of the output-mount dot, or `null` when this agent has no
   * output files. */
  mountIndex: number | null
}

/** Narrowed input for `computeAgentOutputPortLayout`, same "just the fields
 * the layout math needs" reasoning as `AgentInputPortLayoutInput`. */
export interface AgentOutputPortLayoutInput {
  outputLinkIds: string[]
  outputMountId: string | null
}

/** Output-side sibling of `computeAgentInputPortLayout` — fans the
 * output-mount node into the SAME combined right-side stack as authored
 * output links (docs/cw/14_Agent輸出清單化.md §4.2), one slot simpler than
 * the input side since there's only one possible mount kind here (not
 * three). Used by both `AgentCanvasNodeCard`'s output `PortSlots` (rendering
 * the dots) and `AgentCanvasPane`'s `renderEdge` (anchoring the
 * `output-mount` wire to that same slot). */
export function computeAgentOutputPortLayout(node: AgentOutputPortLayoutInput): AgentOutputPortLayout {
  const mountIndex = node.outputMountId !== null ? node.outputLinkIds.length : null
  const total = node.outputLinkIds.length + (node.outputMountId !== null ? 1 : 0) + 1
  return { total, mountIndex }
}

/** `link` edges are `agent_links` rows (authored, hand-drawn, interactive;
 * derived, automatic, read-only). `ownership` edges are derived purely
 * client-side from `agent.parentAgentId` — deliberately not persisted as an
 * `agent_links` row, since that table's kind enum is about communication
 * relationships, not "who manages whom" (docs/cw/04_客製化設計.md §1, P4.5). */
export type AgentCanvasEdgeData =
  | {
      kind: 'link'
      link: AgentLink
      /** Only set for a `derived` link when the opposite direction ALSO has
       * its own row (e.g. both `a -> b` and `b -> a` were recorded from real
       * `gto send` traffic) — the backend keeps these as two independent
       * facts (see `AgentLinkRepository.delete_derived_link`), but rendering
       * them as two separate edges produced two visually different curves
       * (`computePortEdgeGeometry` is directional: it always exits `from`'s
       * right side and enters `to`'s left side), which reads as a second
       * wire looping backward into the previous agent. Collapsing them into
       * one edge with a double-headed arrow instead keeps the "two agents
       * talk to each other" fact visible without the backward-looking
       * duplicate line. */
      bidirectional?: boolean
    }
  | { kind: 'ownership' }
  /** From an `AgentCanvasMcpNodeData` node to the agent it's mounted on —
   * `mountId` matches the MCP node's own `id` (== `buildMcpNodeId(agentId,
   * server.id)`), used to resolve this edge's slot within the agent's
   * combined left-side port stack (see `AgentCanvasAgentNodeData.mcpMountIds`). */
  | { kind: 'mcp-mount'; agentId: string; mountId: string }
  /** From an `AgentCanvasSkillNodeData`/`AgentCanvasHookNodeData` node to the
   * agent it summarizes — same shape/purpose as `mcp-mount`, resolved via
   * `computeAgentInputPortLayout`'s `skillIndex`/`hookIndex` instead of an
   * array `.indexOf` (there's at most one of each per agent, no list to
   * index into). */
  | { kind: 'skill-mount'; agentId: string; mountId: string }
  | { kind: 'hook-mount'; agentId: string; mountId: string }
  /** From the agent to its `AgentCanvasOutputNodeData` node — direction is
   * REVERSED from `mcp-mount`/`skill-mount`/`hook-mount` above (which point
   * mount -> agent): those three sit to the agent's LEFT (inputs), this one
   * sits to the agent's RIGHT (an output), so the natural left-to-right wire
   * is agent -> mount instead. Resolved via `computeAgentOutputPortLayout`'s
   * `mountIndex`, same "at most one, no array to index into" reasoning as
   * `skill-mount`/`hook-mount`. */
  | { kind: 'output-mount'; agentId: string; mountId: string }

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
  mcpServersByAgentId: Record<string, McpServerCapability[]>,
  mcpNodePositions: Record<string, { x: number; y: number }>,
  skillsByAgentId: Record<string, SkillCapability[]>,
  hooksByAgentId: Record<string, HookCapability[]>,
  skillNodePositions: Record<string, { x: number; y: number }>,
  hookNodePositions: Record<string, { x: number; y: number }>,
  /** Keyed by the mount node's own id (`buildMcpNodeId`/`buildSkillNodeId`/
   * `buildHookNodeId`) — shared across all three kinds since each id shape
   * is already unique to its kind (see `useAgentCanvasData`'s
   * `MOUNT_COLORS_STORAGE_PREFIX`). */
  mountNodeColors: Record<string, string> = {},
  outputFilesByAgentId: Record<string, AgentOutputFile[]> = {},
  outputNodePositions: Record<string, { x: number; y: number }> = {},
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

  // Same "only populated on the primary instance" rule as the two maps
  // above, keyed by the PARENT's agentId — `agent.parentAgentId` itself
  // stays live-checked via `primaryInstanceIdByAgentId.has(...)` so a
  // subagent whose parent was deleted never contributes a dangling entry.
  const childAgentIdsByParentAgentId = new Map<string, string[]>()
  for (const agent of agents) {
    if (!agent.parentAgentId) continue
    if (!primaryInstanceIdByAgentId.has(agent.parentAgentId) || !primaryInstanceIdByAgentId.has(agent.id)) continue
    const children = childAgentIdsByParentAgentId.get(agent.parentAgentId) ?? []
    children.push(agent.id)
    childAgentIdsByParentAgentId.set(agent.parentAgentId, children)
  }

  let maxX = 0
  let maxY = 0
  // Resolved canvas position of each agent's PRIMARY instance only — the
  // one an MCP-mount node auto-positions relative to (see the MCP node
  // loop below); a duplicate instance never anchors a mount node, same
  // "primary only" rule the link/ownership maps above already follow.
  const primaryAgentPositions = new Map<string, { x: number; y: number }>()
  const nodes: GraphCanvasNode<AgentCanvasNodeData>[] = liveInstances.map((instance, index) => {
    const agent = agentById.get(instance.agentId) as AgentProfile
    const position = resolveNodePosition(instance, agent, index, columns)
    const size = nodeSizeForAgent(agent)
    maxX = Math.max(maxX, position.x + size.width)
    maxY = Math.max(maxY, position.y + size.height)
    const isPrimaryInstance = primaryInstanceIdByAgentId.get(agent.id) === instance.instanceId
    if (isPrimaryInstance) primaryAgentPositions.set(agent.id, position)
    return {
      id: instance.instanceId,
      x: position.x,
      y: position.y,
      width: size.width,
      height: size.height,
      data: {
        kind: 'agent' as const,
        instanceId: instance.instanceId,
        agent,
        runtimeState: statusByAgentId.get(agent.id) ?? 'unknown',
        outputLinkIds: isPrimaryInstance ? outputLinkIdsByAgentId.get(agent.id) ?? [] : [],
        inputLinkIds: isPrimaryInstance ? inputLinkIdsByAgentId.get(agent.id) ?? [] : [],
        childAgentIds: isPrimaryInstance ? childAgentIdsByParentAgentId.get(agent.id) ?? [] : [],
        mcpMountIds: isPrimaryInstance
          ? (mcpServersByAgentId[agent.id] ?? []).map((server) => buildMcpNodeId(agent.id, server.id))
          : [],
        skillMountId:
          isPrimaryInstance && (skillsByAgentId[agent.id]?.length ?? 0) > 0 ? buildSkillNodeId(agent.id) : null,
        hookMountId:
          isPrimaryInstance && (hooksByAgentId[agent.id]?.length ?? 0) > 0 ? buildHookNodeId(agent.id) : null,
        outputMountId:
          isPrimaryInstance && (outputFilesByAgentId[agent.id]?.length ?? 0) > 0
            ? buildOutputNodeId(agent.id)
            : null,
      },
    }
  })

  // One node + one `mcp-mount` edge per (agent, mounted MCP server) pair,
  // anchored to the agent's primary instance only. Position is client-only
  // and draggable (see `useAgentCanvasData`'s `mcpNodePositions`) — falls
  // back to a computed default (to the agent's left, fanned vertically for
  // more than one server) exactly like `resolveNodePosition` already does
  // for an agent instance with no stored position, only ever "seeded" for
  // real once the user actually drags it.
  const mcpNodes: GraphCanvasNode<AgentCanvasNodeData>[] = []
  const mcpMountEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = []
  // Skill/Hook mount nodes below reuse this same loop (one node summarizing
  // ALL mounted items, not one per item like MCP) — their default vertical
  // offset stacks after however many MCP nodes this agent already has, so
  // the three kinds don't overlap in the left column.
  const skillNodes: GraphCanvasNode<AgentCanvasNodeData>[] = []
  const hookNodes: GraphCanvasNode<AgentCanvasNodeData>[] = []
  const skillMountEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = []
  const hookMountEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = []
  // Output-list node — unlike the three above, this anchors to the agent's
  // RIGHT side (an output, not an input), so its default position is offset
  // past the agent's own width instead of subtracted from its x (see the
  // loop body below); it also doesn't stack against the MCP/Skill/Hook
  // column since it lives in a column of its own.
  const outputNodes: GraphCanvasNode<AgentCanvasNodeData>[] = []
  const outputMountEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = []
  for (const [agentId, agentPosition] of primaryAgentPositions) {
    const servers = mcpServersByAgentId[agentId] ?? []
    const primaryInstanceId = primaryInstanceIdByAgentId.get(agentId) as string
    servers.forEach((server, serverIndex) => {
      const mountId = buildMcpNodeId(agentId, server.id)
      const stored = mcpNodePositions[mountId]
      const position = stored
        ? { x: Math.max(0, stored.x), y: Math.max(0, stored.y) }
        : {
            x: Math.max(0, agentPosition.x - MCP_NODE_WIDTH - NODE_HSPACING),
            y: Math.max(0, agentPosition.y + serverIndex * (MCP_NODE_HEIGHT + MCP_NODE_VSPACING)),
          }
      maxX = Math.max(maxX, position.x + MCP_NODE_WIDTH)
      maxY = Math.max(maxY, position.y + MCP_NODE_HEIGHT)
      mcpNodes.push({
        id: mountId,
        x: position.x,
        y: position.y,
        width: MCP_NODE_WIDTH,
        height: MCP_NODE_HEIGHT,
        data: { kind: 'mcp' as const, id: mountId, agentId, server, color: mountNodeColors[mountId] ?? null },
      })
      mcpMountEdges.push({
        fromId: mountId,
        toId: primaryInstanceId,
        data: { kind: 'mcp-mount' as const, agentId, mountId },
      })
    })

    const skills = skillsByAgentId[agentId] ?? []
    if (skills.length > 0) {
      const mountId = buildSkillNodeId(agentId)
      const stored = skillNodePositions[mountId]
      const position = stored
        ? { x: Math.max(0, stored.x), y: Math.max(0, stored.y) }
        : {
            x: Math.max(0, agentPosition.x - SKILL_NODE_WIDTH - NODE_HSPACING),
            y: Math.max(0, agentPosition.y + servers.length * (MCP_NODE_HEIGHT + MCP_NODE_VSPACING)),
          }
      maxX = Math.max(maxX, position.x + SKILL_NODE_WIDTH)
      maxY = Math.max(maxY, position.y + SKILL_NODE_HEIGHT)
      skillNodes.push({
        id: mountId,
        x: position.x,
        y: position.y,
        width: SKILL_NODE_WIDTH,
        height: SKILL_NODE_HEIGHT,
        data: { kind: 'skill' as const, id: mountId, agentId, skills, color: mountNodeColors[mountId] ?? null },
      })
      skillMountEdges.push({
        fromId: mountId,
        toId: primaryInstanceId,
        data: { kind: 'skill-mount' as const, agentId, mountId },
      })
    }

    const hooks = hooksByAgentId[agentId] ?? []
    if (hooks.length > 0) {
      const mountId = buildHookNodeId(agentId)
      const stored = hookNodePositions[mountId]
      const position = stored
        ? { x: Math.max(0, stored.x), y: Math.max(0, stored.y) }
        : {
            x: Math.max(0, agentPosition.x - HOOK_NODE_WIDTH - NODE_HSPACING),
            y: Math.max(
              0,
              agentPosition.y + (servers.length + (skills.length > 0 ? 1 : 0)) * (MCP_NODE_HEIGHT + MCP_NODE_VSPACING),
            ),
          }
      maxX = Math.max(maxX, position.x + HOOK_NODE_WIDTH)
      maxY = Math.max(maxY, position.y + HOOK_NODE_HEIGHT)
      hookNodes.push({
        id: mountId,
        x: position.x,
        y: position.y,
        width: HOOK_NODE_WIDTH,
        height: HOOK_NODE_HEIGHT,
        data: { kind: 'hook' as const, id: mountId, agentId, hooks, color: mountNodeColors[mountId] ?? null },
      })
      hookMountEdges.push({
        fromId: mountId,
        toId: primaryInstanceId,
        data: { kind: 'hook-mount' as const, agentId, mountId },
      })
    }

    const outputFiles = outputFilesByAgentId[agentId] ?? []
    if (outputFiles.length > 0) {
      const mountId = buildOutputNodeId(agentId)
      const stored = outputNodePositions[mountId]
      const agentWidth = nodeSizeForAgent(agentById.get(agentId) as AgentProfile).width
      const position = stored
        ? { x: Math.max(0, stored.x), y: Math.max(0, stored.y) }
        : {
            x: Math.max(0, agentPosition.x + agentWidth + NODE_HSPACING),
            y: Math.max(0, agentPosition.y),
          }
      maxX = Math.max(maxX, position.x + OUTPUT_NODE_WIDTH)
      maxY = Math.max(maxY, position.y + OUTPUT_NODE_HEIGHT)
      outputNodes.push({
        id: mountId,
        x: position.x,
        y: position.y,
        width: OUTPUT_NODE_WIDTH,
        height: OUTPUT_NODE_HEIGHT,
        data: { kind: 'output' as const, id: mountId, agentId, files: outputFiles, color: mountNodeColors[mountId] ?? null },
      })
      // Reversed direction vs. the mcp/skill/hook-mount edges above — see
      // `AgentCanvasEdgeData`'s `output-mount` doc comment.
      outputMountEdges.push({
        fromId: primaryInstanceId,
        toId: mountId,
        data: { kind: 'output-mount' as const, agentId, mountId },
      })
    }
  }

  const liveLinks = links.filter(
    (link) => primaryInstanceIdByAgentId.has(link.fromAgentId) && primaryInstanceIdByAgentId.has(link.toAgentId),
  )
  // Authored links are already deduped direction-agnostically at the backend
  // (`create_authored_link`/`has_authored_edge`) — at most one row per
  // unordered pair, so each maps straight to its own edge. Derived links have
  // no such guarantee (`a -> b` and `b -> a` are two independent "last
  // interacted at" facts) — group by unordered pair first so a pair with
  // both directions recorded renders as ONE edge (double arrowhead) instead
  // of two directionally-mirrored curves that look like a wire looping back
  // to the previous agent. Keyed by a nested Map (not a joined string) so an
  // agent id that happens to contain the join separator can't collide two
  // unrelated pairs into one. `agent_links` has a
  // `UNIQUE(workspace_id, from_agent_id, to_agent_id, kind)` index, so each
  // inner array holds at most 2 rows (one per direction).
  const derivedByPair = new Map<string, Map<string, AgentLink[]>>()
  const authoredLinks: AgentLink[] = []
  for (const link of liveLinks) {
    if (link.kind !== 'derived') {
      authoredLinks.push(link)
      continue
    }
    const [a, b] = [link.fromAgentId, link.toAgentId].sort()
    const inner = derivedByPair.get(a) ?? new Map<string, AgentLink[]>()
    const pairLinks = inner.get(b) ?? []
    pairLinks.push(link)
    inner.set(b, pairLinks)
    derivedByPair.set(a, inner)
  }
  const derivedEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = []
  for (const inner of derivedByPair.values()) {
    for (const pairLinks of inner.values()) {
      // `list_links` orders rows by most-recently-active first, so which
      // direction happens to be `pairLinks[0]` flips between polls for a
      // pair that's genuinely talking both ways. Start from a stable row for
      // reverse-direction detection and for the one-way/equal-position
      // fallback; mutual rendering is oriented by canvas position below.
      const stableLinks = [...pairLinks].sort(
        (left, right) => left.fromAgentId.localeCompare(right.fromAgentId) || left.id.localeCompare(right.id),
      )
      const first = stableLinks[0]
      const bidirectional = pairLinks.some(
        (candidate) =>
          candidate.fromAgentId === first.toAgentId && candidate.toAgentId === first.fromAgentId,
      )
      // A mutual relationship has no semantic source side. Always orient its
      // merged wire from the node currently on the left to the node on the
      // right, avoiding a backward output-to-input loop. Dragging a node
      // rebuilds the graph and therefore updates this orientation.
      const firstPosition = primaryAgentPositions.get(first.fromAgentId) as { x: number; y: number }
      const secondPosition = primaryAgentPositions.get(first.toAgentId) as { x: number; y: number }
      const firstComesBefore =
        firstPosition.x < secondPosition.x ||
        (firstPosition.x === secondPosition.x &&
          (firstPosition.y < secondPosition.y ||
            (firstPosition.y === secondPosition.y && first.fromAgentId < first.toAgentId)))
      const leftAgentId = firstComesBefore ? first.fromAgentId : first.toAgentId
      const rightAgentId = firstComesBefore ? first.toAgentId : first.fromAgentId
      const primary = bidirectional
        ? pairLinks.find(
            (candidate) => candidate.fromAgentId === leftAgentId && candidate.toAgentId === rightAgentId,
          ) ?? first
        : first
      derivedEdges.push({
        fromId: primaryInstanceIdByAgentId.get(primary.fromAgentId) as string,
        toId: primaryInstanceIdByAgentId.get(primary.toAgentId) as string,
        data: { kind: 'link' as const, link: primary, bidirectional },
      })
    }
  }
  const linkEdges: GraphCanvasEdge<AgentCanvasEdgeData>[] = [
    ...authoredLinks.map((link) => ({
      fromId: primaryInstanceIdByAgentId.get(link.fromAgentId) as string,
      toId: primaryInstanceIdByAgentId.get(link.toAgentId) as string,
      data: { kind: 'link' as const, link },
    })),
    ...derivedEdges,
  ]

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

  const edges = [
    ...linkEdges,
    ...ownershipEdges,
    ...mcpMountEdges,
    ...skillMountEdges,
    ...hookMountEdges,
    ...outputMountEdges,
  ]

  return {
    nodes: [...nodes, ...mcpNodes, ...skillNodes, ...hookNodes, ...outputNodes],
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
 * around when `to` sits above/below/left of `from`. Used for `link` edges
 * (derived edges call this with no slot args — center anchor, unchanged
 * pre-multi-slot behavior; authored edges pass each side's actual slot so
 * multiple connections fan out to their own distinct port dot) and for
 * `mcp-mount`/`skill-mount`/`hook-mount` edges (`from` = the mount node,
 * `to` = the owning agent — the mount node sits to the agent's left, so
 * "exit `from`'s right, enter `to`'s left" is exactly the anchor wanted).
 * `ownership` edges use the vertical
 * sibling below instead — a parent-to-subagent relationship reads top-down,
 * not left-to-right. */
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

/** Vertical sibling of `computePortEdgeGeometry`, rotated 90°: always exits
 * the bottom-center of `from` and enters the top-center of `to`, regardless
 * of actual relative position — same fixed-side philosophy (a vertical
 * S-curve loops around if `to` isn't actually below `from`). Used
 * exclusively for `ownership` (parent → subagent) edges, matching the
 * reference layout: a parent box above, a straight-reading connector down
 * to each subagent box below it. `fromSlot`'s offset fans out along the
 * parent's bottom edge (X axis) when it owns more than one subagent, so
 * multiple children don't all converge on the exact same pixel; `toSlot`
 * stays centered (`CENTER_SLOT`) since a subagent has exactly one parent. */
export function computeVerticalPortEdgeGeometry(
  from: { x: number; y: number; width?: number; height?: number },
  to: { x: number; y: number; width?: number; height?: number },
  fromSlot: PortSlot = CENTER_SLOT,
  toSlot: PortSlot = CENTER_SLOT,
): PortEdgeGeometry {
  const fromWidth = from.width ?? AGENT_NODE_WIDTH
  const fromHeight = from.height ?? AGENT_NODE_HEIGHT
  const toWidth = to.width ?? AGENT_NODE_WIDTH
  const start = {
    x: from.x + fromWidth / 2 + computePortSlotCenterOffset(fromSlot.index, fromSlot.total),
    y: from.y + fromHeight,
  }
  const end = {
    x: to.x + toWidth / 2 + computePortSlotCenterOffset(toSlot.index, toSlot.total),
    y: to.y,
  }
  const offset = Math.min(120, Math.max(32, Math.abs(end.y - start.y) / 2))
  const c1 = { x: start.x, y: start.y + offset }
  const c2 = { x: end.x, y: end.y - offset }
  return {
    start,
    end,
    path: `M ${start.x} ${start.y} C ${c1.x} ${c1.y}, ${c2.x} ${c2.y}, ${end.x} ${end.y}`,
  }
}
