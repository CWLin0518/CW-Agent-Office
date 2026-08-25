import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { desktopApi } from '@shell/integration/desktop-api'
import type {
  AgentCapabilitySnapshot,
  AgentLink,
  AgentProfile,
  AgentRuntimeStatus,
  HookCapability,
  McpServerCapability,
  SkillCapability,
} from '@shell/integration/desktop-api'
import { buildAgentCanvasGraph, type AgentCanvasGraphView, type CanvasNodeInstance } from '../model/agent-canvas-graph'

/** Mirrors `StationCapabilitiesTab.tsx`'s own constant of the same name and
 * purpose — this file can't import that one (it's feature-local, not
 * exported), and it's a one-line primitive, not worth plumbing through a
 * shared module for. See that file's doc comment for why "System Admin" is
 * the right value here (no per-user identity in this app yet). */
const CAPABILITY_CONFIRMED_BY = 'System Admin'

/** How often to re-poll links + runtime status while the pane is active.
 * There is no push/event mechanism for either yet (see docs/cw/04_客製化設計.md
 * §1) — this is a pane-lifetime interval, not new background infrastructure. */
const POLL_INTERVAL_MS = 8000

/** Canvas node placement is deliberately client-only (docs/cw/04_客製化設計.md
 * §8, P4.6) — it never deletes/creates the underlying `AgentProfile`, so
 * there's nothing for the backend to persist. Workspace-scoped key, same
 * `${prefix}:${workspaceId}` shape as `buildTaskCenterStorageKey`. */
const INSTANCES_STORAGE_PREFIX = 'agent-canvas.instances'
/** Superseded by `INSTANCES_STORAGE_PREFIX` — kept only as the one-time
 * migration source (see `reload`'s migration block below) for workspaces
 * that used P4.6's earlier "remove from canvas" model (a plain hidden-id
 * set, before multi-instance existed). Never written to again. */
const LEGACY_REMOVED_AGENT_IDS_STORAGE_PREFIX = 'agent-canvas.removedAgentIds'

function buildInstancesStorageKey(workspaceId: string): string {
  return `${INSTANCES_STORAGE_PREFIX}:${workspaceId}`
}

function isValidInstance(value: unknown): value is CanvasNodeInstance {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Partial<CanvasNodeInstance>
  if (typeof candidate.instanceId !== 'string' || typeof candidate.agentId !== 'string') return false
  if (candidate.position === undefined) return true
  return (
    typeof candidate.position === 'object' &&
    candidate.position !== null &&
    Number.isFinite(candidate.position.x) &&
    Number.isFinite(candidate.position.y)
  )
}

/** `null` specifically means "never initialized" (storage key absent) —
 * distinct from `[]`, which means "initialized, currently empty" (e.g. the
 * user removed every node). Only `null` triggers the one-time migration. */
function loadInstancesOrNull(workspaceId: string): CanvasNodeInstance[] | null {
  try {
    const raw = window.localStorage.getItem(buildInstancesStorageKey(workspaceId))
    if (raw === null) return null
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter(isValidInstance) : []
  } catch {
    return null
  }
}

function saveInstances(workspaceId: string, instances: CanvasNodeInstance[]): void {
  try {
    window.localStorage.setItem(buildInstancesStorageKey(workspaceId), JSON.stringify(instances))
  } catch {
    // Ignore local storage quota/runtime errors — worst case, canvas layout
    // changes made this session don't survive a reload.
  }
}

/** Whether an agent has a real, backend-persisted canvas layout position —
 * shared by every undo entry below that needs to know what to revert an
 * agent's position back to (or that there's nothing to revert to). */
function persistedAgentLayout(agent: AgentProfile | undefined): { x: number; y: number } | null {
  return agent &&
    typeof agent.layoutX === 'number' &&
    Number.isFinite(agent.layoutX) &&
    typeof agent.layoutY === 'number' &&
    Number.isFinite(agent.layoutY)
    ? { x: agent.layoutX, y: agent.layoutY }
    : null
}

/** Client-only, same reasoning as `INSTANCES_STORAGE_PREFIX` above — a
 * mount node (MCP/Skill/Hook) has no backend column to persist a position
 * in. Keyed by the mount's own node id (`buildMcpNodeId`/`buildSkillNodeId`/
 * `buildHookNodeId`), so an id going stale (e.g. an MCP server's own `id`
 * changing identity server-side) is a non-issue — it just seeds a fresh
 * default position, same as any other never-seen mount. One storage
 * bucket (and prefix) per mount KIND, not a shared one, so an MCP server,
 * a Skill, and a Hook could theoretically collide on the same synthetic id
 * shape without their positions overwriting each other. */
const MCP_POSITIONS_STORAGE_PREFIX = 'agent-canvas.mcpPositions'
const SKILL_POSITIONS_STORAGE_PREFIX = 'agent-canvas.skillPositions'
const HOOK_POSITIONS_STORAGE_PREFIX = 'agent-canvas.hookPositions'

function isValidPositionsRecord(value: unknown): value is Record<string, { x: number; y: number }> {
  if (typeof value !== 'object' || value === null) return false
  return Object.values(value as Record<string, unknown>).every((position) => {
    if (typeof position !== 'object' || position === null) return false
    const candidate = position as Partial<{ x: unknown; y: unknown }>
    return Number.isFinite(candidate.x) && Number.isFinite(candidate.y)
  })
}

function loadPositions(storagePrefix: string, workspaceId: string): Record<string, { x: number; y: number }> {
  try {
    const raw = window.localStorage.getItem(`${storagePrefix}:${workspaceId}`)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    return isValidPositionsRecord(parsed) ? parsed : {}
  } catch {
    return {}
  }
}

function savePositions(
  storagePrefix: string,
  workspaceId: string,
  positions: Record<string, { x: number; y: number }>,
): void {
  try {
    window.localStorage.setItem(`${storagePrefix}:${workspaceId}`, JSON.stringify(positions))
  } catch {
    // Ignore local storage quota/runtime errors — same tradeoff as `saveInstances`.
  }
}

function loadLegacyRemovedAgentIds(workspaceId: string): Set<string> {
  try {
    const raw = window.localStorage.getItem(`${LEGACY_REMOVED_AGENT_IDS_STORAGE_PREFIX}:${workspaceId}`)
    if (!raw) return new Set()
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed) ? new Set(parsed.filter((id): id is string => typeof id === 'string')) : new Set()
  } catch {
    return new Set()
  }
}

interface UseAgentCanvasDataResult {
  graph: AgentCanvasGraphView
  loaded: boolean
  isEmpty: boolean
  /** Commits a node's position. Routes to the backend-persisted
   * `agents.layout_x/y` path for an instance's default (`instanceId ===
   * agentId`) node — unchanged from pre-P4.6 — or to purely client-side
   * `instances` storage for any other (duplicate) instance. */
  commitInstancePosition: (instanceId: string, agentId: string, position: { x: number; y: number }) => void
  /** Commits a dragged position for one MCP-mount node — always client-only
   * (there's no backend column for this, unlike an agent instance's
   * default-position path), keyed by the node's own id
   * (`buildMcpNodeId(agentId, server.id)`). */
  commitMcpNodePosition: (mcpNodeId: string, position: { x: number; y: number }) => void
  /** Sibling of `commitMcpNodePosition` for the Skill-mount node — see
   * `AgentCanvasSkillNodeData`. */
  commitSkillNodePosition: (skillNodeId: string, position: { x: number; y: number }) => void
  /** Sibling of `commitMcpNodePosition` for the Hook-mount node — see
   * `AgentCanvasHookNodeData`. */
  commitHookNodePosition: (hookNodeId: string, position: { x: number; y: number }) => void
  /** Flips one MCP server's `enabled` flag within its agent's capability
   * snapshot and re-saves the whole snapshot (skills/hooks carried through
   * unchanged) — the canvas node's on/off switch, mirrored from the same
   * field editable in the Capabilities tab. Never removes the mount, only
   * whether the next materialize actually includes it. */
  setMcpServerEnabled: (agentId: string, serverId: string, enabled: boolean) => void
  createAuthoredLink: (fromAgentId: string, toAgentId: string) => Promise<void>
  deleteAuthoredLink: (fromAgentId: string, toAgentId: string) => Promise<void>
  deleteDerivedLink: (fromAgentId: string, toAgentId: string) => Promise<void>
  /** Removes one canvas node instance — never calls anything that deletes
   * the underlying `AgentProfile` (docs/cw/04_客製化設計.md §1, P4.6). If it
   * was the last instance for that agent, the agent simply has no node on
   * canvas until dragged back from the standby rail. */
  removeInstancesFromCanvas: (instanceIds: string[]) => void
  /** Adds a new canvas node instance for an agent that already has zero or
   * more instances — dragging the same agent from the standby rail twice
   * creates two independent, purely-visual nodes (docs/cw/04_客製化設計.md
   * §8, P4.6). */
  addInstance: (agentId: string, position: { x: number; y: number }) => void
  /** Sets the node border color for one or more agents (batch, for a
   * multi-node selection) — backend-persisted (docs/cw/04_客製化設計.md §1,
   * P4.6). `color: null` resets to the default gray. Per-agent, not
   * per-instance — every instance of the same agent shares one color. */
  setAgentColor: (agentIds: string[], color: string | null) => void
  /** Sets the display color for one or more authored links, addressed by
   * `AgentLink.id` (batch, for a multi-wire selection) — backend-persisted
   * (docs/cw/04_客製化設計.md §8, P4.6). `color: null` resets to default gray. */
  setLinkColor: (linkIds: string[], color: string | null) => void
  /** Toggles the unidirectional/bidirectional display flag for one or more
   * authored links, addressed by `AgentLink.id` (docs/cw/04_客製化設計.md §8,
   * P4.6). */
  setLinkBidirectional: (linkIds: string[], bidirectional: boolean) => void
  /** Reverts the most recent undoable canvas action (node move/add/remove,
   * authored link create/delete, agent/link color, link bidirectional) —
   * no-op if there's nothing to undo. Derived-link deletion is intentionally
   * not undoable (there's no "create a derived link" operation to reverse
   * it with). */
  undo: () => void
}

/** Data loading + polling for agent-canvas, kept out of the presentational
 * `AgentCanvasPane` component — mirrors `useDesignerDocumentState`'s split
 * between a controller hook and a rendering component. */
export function useAgentCanvasData(
  workspaceId: string | null,
  active: boolean,
): UseAgentCanvasDataResult {
  const [agents, setAgents] = useState<AgentProfile[]>([])
  // Mirrors `agents` for callbacks (`commitAgentLayout`/`addInstance`) that
  // only need the latest value to compute an undo entry's "previous state"
  // — reading through this ref instead of depending on `agents` directly
  // keeps those callbacks' identity stable across the 8s poll tick (which
  // replaces `agents` with a new array reference every time, changed or
  // not), instead of recreating them — and every callback built on top of
  // them in `AgentCanvasPane.tsx` — on every single poll.
  const agentsRef = useRef(agents)
  useEffect(() => {
    agentsRef.current = agents
  })
  const [links, setLinks] = useState<AgentLink[]>([])
  const [statuses, setStatuses] = useState<AgentRuntimeStatus[]>([])
  const [loaded, setLoaded] = useState(false)
  const [instances, setInstances] = useState<CanvasNodeInstance[]>(() =>
    workspaceId ? loadInstancesOrNull(workspaceId) ?? [] : [],
  )
  // Re-read `instances` whenever the workspace id changes (it's keyed by
  // workspace id in storage, so switching workspaces must not carry over
  // the previous workspace's set). Deferred via setTimeout(0), same as this
  // file's own polling effect below — the read runs inside the timer
  // callback rather than synchronously in the effect body, which is the
  // "subscribe to an external system" shape effects are meant to have.
  useEffect(() => {
    const id = window.setTimeout(() => {
      setInstances(workspaceId ? loadInstancesOrNull(workspaceId) ?? [] : [])
    }, 0)
    return () => window.clearTimeout(id)
  }, [workspaceId])

  // `AgentCapabilitySnapshot` per agent — no bulk "capabilities for every
  // agent in a workspace" endpoint exists yet, so `reload()` below fires one
  // `agentCapabilityRead` per agent (this workspace's agent counts are
  // small; a bulk endpoint can be added later if that stops being cheap).
  // Kept as the FULL snapshot, not just `mcpServers`, so `setMcpServerEnabled`
  // can re-save without clobbering an agent's skills/hooks it never touched.
  const [capabilityByAgentId, setCapabilityByAgentId] = useState<Record<string, AgentCapabilitySnapshot>>({})
  // Read by `reload()`'s per-agent failure fallback below — `reload` is a
  // stable-identity `useCallback` (deps: `[workspaceId]`, same reasoning as
  // `agentsRef` above), so it can't close over `capabilityByAgentId`
  // directly without going stale after the first successful read.
  const capabilityByAgentIdRef = useRef(capabilityByAgentId)
  useEffect(() => {
    capabilityByAgentIdRef.current = capabilityByAgentId
  })
  const mcpServersByAgentId = useMemo<Record<string, McpServerCapability[]>>(
    () => Object.fromEntries(Object.entries(capabilityByAgentId).map(([agentId, capability]) => [agentId, capability.mcpServers])),
    [capabilityByAgentId],
  )
  const skillsByAgentId = useMemo<Record<string, SkillCapability[]>>(
    () => Object.fromEntries(Object.entries(capabilityByAgentId).map(([agentId, capability]) => [agentId, capability.skills])),
    [capabilityByAgentId],
  )
  const hooksByAgentId = useMemo<Record<string, HookCapability[]>>(
    () => Object.fromEntries(Object.entries(capabilityByAgentId).map(([agentId, capability]) => [agentId, capability.hooks])),
    [capabilityByAgentId],
  )

  const [mcpNodePositions, setMcpNodePositions] = useState<Record<string, { x: number; y: number }>>(() =>
    workspaceId ? loadPositions(MCP_POSITIONS_STORAGE_PREFIX, workspaceId) : {},
  )
  const [skillNodePositions, setSkillNodePositions] = useState<Record<string, { x: number; y: number }>>(() =>
    workspaceId ? loadPositions(SKILL_POSITIONS_STORAGE_PREFIX, workspaceId) : {},
  )
  const [hookNodePositions, setHookNodePositions] = useState<Record<string, { x: number; y: number }>>(() =>
    workspaceId ? loadPositions(HOOK_POSITIONS_STORAGE_PREFIX, workspaceId) : {},
  )
  // Same re-read-on-workspace-change shape as `instances` above.
  useEffect(() => {
    const id = window.setTimeout(() => {
      setMcpNodePositions(workspaceId ? loadPositions(MCP_POSITIONS_STORAGE_PREFIX, workspaceId) : {})
      setSkillNodePositions(workspaceId ? loadPositions(SKILL_POSITIONS_STORAGE_PREFIX, workspaceId) : {})
      setHookNodePositions(workspaceId ? loadPositions(HOOK_POSITIONS_STORAGE_PREFIX, workspaceId) : {})
    }, 0)
    return () => window.clearTimeout(id)
  }, [workspaceId])

  // Undo history: a stack of inverse-action closures, pushed by every
  // mutator below right after it applies its change. `isUndoingRef` guards
  // against an undo closure's own calls (e.g. `deleteAuthoredLink` re-run to
  // undo a `createAuthoredLink`) re-recording themselves — undo only ever
  // walks backward, there's no redo.
  const historyRef = useRef<Array<() => void | Promise<void>>>([])
  const isUndoingRef = useRef(false)
  const HISTORY_LIMIT = 50

  const pushHistory = useCallback((entry: () => void | Promise<void>) => {
    if (isUndoingRef.current) return
    historyRef.current.push(entry)
    if (historyRef.current.length > HISTORY_LIMIT) historyRef.current.shift()
  }, [])

  // Chains every backend write onto the previous one's completion — both
  // undo entries AND the forward mutators' own writes route through this,
  // so a fire-and-forget forward write (e.g. `setAgentColor`'s
  // `desktopApi.agentCanvasSetAgentColor` call) can never be overtaken by
  // an undo issued right after it: the undo's write simply waits its turn
  // in the same queue instead of racing the still-in-flight original and
  // possibly landing first, which would otherwise leave the backend on the
  // ORIGINAL (post-change) value even though the UI shows the reverted one.
  const writeQueueRef = useRef<Promise<void>>(Promise.resolve())

  const enqueueWrite = useCallback((write: () => unknown) => {
    writeQueueRef.current = writeQueueRef.current.then(async () => {
      try {
        await write()
      } catch (error) {
        console.error('[agent-canvas] write failed', error)
      }
    })
  }, [])

  const undo = useCallback(() => {
    const entry = historyRef.current.pop()
    if (!entry) return
    enqueueWrite(async () => {
      isUndoingRef.current = true
      try {
        await entry()
      } finally {
        isUndoingRef.current = false
      }
    })
  }, [enqueueWrite])

  const removeInstancesFromCanvas = useCallback(
    (instanceIds: string[]) => {
      if (!workspaceId || instanceIds.length === 0) return
      const idSet = new Set(instanceIds)
      const removed = instances.filter((instance) => idSet.has(instance.instanceId))
      setInstances((previous) => {
        const next = previous.filter((instance) => !idSet.has(instance.instanceId))
        saveInstances(workspaceId, next)
        return next
      })
      if (removed.length === 0) return
      pushHistory(() => {
        setInstances((previous) => {
          const existingIds = new Set(previous.map((instance) => instance.instanceId))
          const restored = [...previous, ...removed.filter((instance) => !existingIds.has(instance.instanceId))]
          saveInstances(workspaceId, restored)
          return restored
        })
      })
    },
    [workspaceId, instances, pushHistory],
  )

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

      // Best-effort per agent — one failed read shouldn't blank out every
      // other agent's already-known MCP nodes on canvas.
      const capabilityEntries = await Promise.all(
        agentsResponse.agents.map(async (agent) => {
          try {
            const response = await desktopApi.agentCapabilityRead({ workspaceId, agentId: agent.id })
            return [agent.id, response.capability] as const
          } catch {
            return [agent.id, capabilityByAgentIdRef.current[agent.id]] as const
          }
        }),
      )
      setCapabilityByAgentId(
        Object.fromEntries(capabilityEntries.filter((entry): entry is [string, AgentCapabilitySnapshot] => Boolean(entry[1]))),
      )

      // One-time migration: a workspace that never had an `instances` key
      // predates multi-instance support — seed one default instance
      // (`instanceId === agentId`, so it keeps using `agent.layoutX/layoutY`)
      // per agent not previously hidden via the old removed-ids model, so
      // existing canvases don't suddenly go blank. Runs at most once ever
      // per workspace: after `saveInstances` below, `loadInstancesOrNull`
      // returns non-null on every later reload/poll, skipping this block.
      if (loadInstancesOrNull(workspaceId) === null) {
        const legacyRemoved = loadLegacyRemovedAgentIds(workspaceId)
        const seeded: CanvasNodeInstance[] = agentsResponse.agents
          .filter((agent) => !legacyRemoved.has(agent.id))
          .map((agent) => ({ instanceId: agent.id, agentId: agent.id }))
        saveInstances(workspaceId, seeded)
        setInstances(seeded)
      }
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

  const graph = useMemo(
    () =>
      buildAgentCanvasGraph(
        agents,
        links,
        statuses,
        instances,
        mcpServersByAgentId,
        mcpNodePositions,
        skillsByAgentId,
        hooksByAgentId,
        skillNodePositions,
        hookNodePositions,
      ),
    [
      agents,
      links,
      statuses,
      instances,
      mcpServersByAgentId,
      mcpNodePositions,
      skillsByAgentId,
      hooksByAgentId,
      skillNodePositions,
      hookNodePositions,
    ],
  )

  const commitAgentLayout = useCallback(
    (agentId: string, position: { x: number; y: number }) => {
      const previousPosition = persistedAgentLayout(
        agentsRef.current.find((agent) => agent.id === agentId),
      )
      setAgents((previous) =>
        previous.map((agent) =>
          agent.id === agentId ? { ...agent, layoutX: position.x, layoutY: position.y } : agent,
        ),
      )
      if (workspaceId) {
        enqueueWrite(() => desktopApi.agentCanvasSetLayout(workspaceId, agentId, position.x, position.y))
      }
      // Nothing meaningful to revert to if the agent never had a persisted
      // layout before (e.g. its very first canvas placement) — leaving it as
      // the newly-committed position is harmless since that case is only
      // ever hit via `addInstance`, which supplies its own combined undo
      // entry covering both the instance and this layout together.
      if (previousPosition) {
        pushHistory(() => commitAgentLayout(agentId, previousPosition))
      }
    },
    [workspaceId, pushHistory, enqueueWrite],
  )

  const commitInstancePosition = useCallback(
    (instanceId: string, agentId: string, position: { x: number; y: number }) => {
      if (instanceId === agentId) {
        commitAgentLayout(agentId, position)
        return
      }
      if (!workspaceId) return
      const previousInstance = instances.find((instance) => instance.instanceId === instanceId)
      const previousPosition = previousInstance?.position
      setInstances((previous) => {
        const next = previous.map((instance) =>
          instance.instanceId === instanceId ? { ...instance, position } : instance,
        )
        saveInstances(workspaceId, next)
        return next
      })
      pushHistory(() => {
        setInstances((previous) => {
          const next = previous.map((instance) =>
            instance.instanceId === instanceId
              ? previousPosition
                ? { ...instance, position: previousPosition }
                : { instanceId: instance.instanceId, agentId: instance.agentId }
              : instance,
          )
          saveInstances(workspaceId, next)
          return next
        })
      })
    },
    [workspaceId, instances, commitAgentLayout, pushHistory],
  )

  const commitMcpNodePosition = useCallback(
    (mcpNodeId: string, position: { x: number; y: number }) => {
      if (!workspaceId) return
      const previousPosition = mcpNodePositions[mcpNodeId]
      setMcpNodePositions((previous) => {
        const next = { ...previous, [mcpNodeId]: position }
        savePositions(MCP_POSITIONS_STORAGE_PREFIX, workspaceId, next)
        return next
      })
      pushHistory(() => {
        setMcpNodePositions((previous) => {
          const next = { ...previous }
          if (previousPosition) {
            next[mcpNodeId] = previousPosition
          } else {
            delete next[mcpNodeId]
          }
          savePositions(MCP_POSITIONS_STORAGE_PREFIX, workspaceId, next)
          return next
        })
      })
    },
    [workspaceId, mcpNodePositions, pushHistory],
  )

  // Skill/Hook mount nodes are draggable the same way an MCP mount node is
  // (client-only position, undoable) — see `commitMcpNodePosition` above,
  // which this mirrors exactly, just against the Skill/Hook position stores.
  const commitSkillNodePosition = useCallback(
    (skillNodeId: string, position: { x: number; y: number }) => {
      if (!workspaceId) return
      const previousPosition = skillNodePositions[skillNodeId]
      setSkillNodePositions((previous) => {
        const next = { ...previous, [skillNodeId]: position }
        savePositions(SKILL_POSITIONS_STORAGE_PREFIX, workspaceId, next)
        return next
      })
      pushHistory(() => {
        setSkillNodePositions((previous) => {
          const next = { ...previous }
          if (previousPosition) {
            next[skillNodeId] = previousPosition
          } else {
            delete next[skillNodeId]
          }
          savePositions(SKILL_POSITIONS_STORAGE_PREFIX, workspaceId, next)
          return next
        })
      })
    },
    [workspaceId, skillNodePositions, pushHistory],
  )

  const commitHookNodePosition = useCallback(
    (hookNodeId: string, position: { x: number; y: number }) => {
      if (!workspaceId) return
      const previousPosition = hookNodePositions[hookNodeId]
      setHookNodePositions((previous) => {
        const next = { ...previous, [hookNodeId]: position }
        savePositions(HOOK_POSITIONS_STORAGE_PREFIX, workspaceId, next)
        return next
      })
      pushHistory(() => {
        setHookNodePositions((previous) => {
          const next = { ...previous }
          if (previousPosition) {
            next[hookNodeId] = previousPosition
          } else {
            delete next[hookNodeId]
          }
          savePositions(HOOK_POSITIONS_STORAGE_PREFIX, workspaceId, next)
          return next
        })
      })
    },
    [workspaceId, hookNodePositions, pushHistory],
  )

  const setMcpServerEnabled = useCallback(
    (agentId: string, serverId: string, enabled: boolean) => {
      if (!workspaceId) return
      const previousSnapshot = capabilityByAgentId[agentId]
      if (!previousSnapshot) return
      const previousServer = previousSnapshot.mcpServers.find((server) => server.id === serverId)
      if (!previousServer || previousServer.enabled === enabled) return

      const applyEnabled = (snapshot: AgentCapabilitySnapshot, nextEnabled: boolean): AgentCapabilitySnapshot => ({
        ...snapshot,
        mcpServers: snapshot.mcpServers.map((server) =>
          server.id === serverId ? { ...server, enabled: nextEnabled } : server,
        ),
      })

      const nextSnapshot = applyEnabled(previousSnapshot, enabled)
      setCapabilityByAgentId((previous) => ({ ...previous, [agentId]: nextSnapshot }))
      enqueueWrite(() =>
        desktopApi.agentCapabilitySave({
          workspaceId,
          agentId,
          capability: nextSnapshot,
          confirmedBy: CAPABILITY_CONFIRMED_BY,
        }),
      )
      pushHistory(async () => {
        setCapabilityByAgentId((previous) => ({ ...previous, [agentId]: previousSnapshot }))
        // Awaited directly, same reasoning as every other undo entry in this
        // file — `undo()` already runs this inside its own queued turn.
        await desktopApi.agentCapabilitySave({
          workspaceId,
          agentId,
          capability: previousSnapshot,
          confirmedBy: CAPABILITY_CONFIRMED_BY,
        })
      })
    },
    [workspaceId, capabilityByAgentId, pushHistory, enqueueWrite],
  )

  const addInstance = useCallback(
    (agentId: string, position: { x: number; y: number }) => {
      if (!workspaceId) return
      // Matches `CanvasNodeInstance`'s documented convention: the FIRST
      // instance an agent ever gets on canvas is its "default" one
      // (`instanceId === agentId`), so it persists via the same backend
      // `agents.layout_x/y` path every pre-P4.6, single-instance canvas
      // already used — without this check, an agent created after the
      // one-time migration would have its position permanently stuck in
      // client-only storage the first time it's dragged onto the canvas,
      // unlike every migration-seeded agent.
      const isFirstInstanceForAgent = !instances.some((instance) => instance.agentId === agentId)
      const instanceId = isFirstInstanceForAgent
        ? agentId
        : `${agentId}:${Math.random().toString(36).slice(2)}-${Date.now().toString(36)}`
      const previousAgentLayout = isFirstInstanceForAgent
        ? persistedAgentLayout(agentsRef.current.find((agent) => agent.id === agentId))
        : null
      const next: CanvasNodeInstance[] = [
        ...instances,
        isFirstInstanceForAgent ? { instanceId, agentId } : { instanceId, agentId, position },
      ]
      setInstances(next)
      saveInstances(workspaceId, next)
      if (isFirstInstanceForAgent) {
        // Suppress `commitAgentLayout`'s own undo entry here — this
        // action's undo entry below already restores the prior agent
        // layout (if any) as part of undoing the whole "add" as one step,
        // e.g. re-dragging a previously-removed default instance back onto
        // the canvas shouldn't cost two Ctrl+Z presses to fully reverse.
        const wasUndoing = isUndoingRef.current
        isUndoingRef.current = true
        commitAgentLayout(agentId, position)
        isUndoingRef.current = wasUndoing
      }
      pushHistory(() => {
        setInstances((previous) => {
          const reverted = previous.filter((instance) => instance.instanceId !== instanceId)
          saveInstances(workspaceId, reverted)
          return reverted
        })
        if (previousAgentLayout) {
          commitAgentLayout(agentId, previousAgentLayout)
        }
      })
    },
    [workspaceId, instances, commitAgentLayout, pushHistory],
  )

  const createAuthoredLink = useCallback(
    async (fromAgentId: string, toAgentId: string) => {
      if (!workspaceId) return
      await desktopApi.agentCanvasCreateAuthoredLink(workspaceId, fromAgentId, toAgentId)
      await reload()
      pushHistory(async () => {
        await desktopApi.agentCanvasDeleteAuthoredLink(workspaceId, fromAgentId, toAgentId)
        await reload()
      })
    },
    [workspaceId, reload, pushHistory],
  )

  const deleteAuthoredLink = useCallback(
    async (fromAgentId: string, toAgentId: string) => {
      if (!workspaceId) return
      // Captured before the delete so undo can restore the link's display
      // attributes too — the recreated link gets a brand new backend id, so
      // this can't be re-addressed by the old `AgentLink.id` afterward, only
      // by the (still-stable) agent pair, directly via `desktopApi`.
      const previousLink = links.find(
        (link) => link.kind === 'authored' && link.fromAgentId === fromAgentId && link.toAgentId === toAgentId,
      )
      await desktopApi.agentCanvasDeleteAuthoredLink(workspaceId, fromAgentId, toAgentId)
      await reload()
      if (previousLink) {
        pushHistory(async () => {
          await desktopApi.agentCanvasCreateAuthoredLink(workspaceId, fromAgentId, toAgentId)
          if (previousLink.color) {
            await desktopApi.agentCanvasSetLinkColor(workspaceId, fromAgentId, toAgentId, previousLink.color)
          }
          if (previousLink.bidirectional) {
            await desktopApi.agentCanvasSetLinkBidirectional(workspaceId, fromAgentId, toAgentId, true)
          }
          await reload()
        })
      }
    },
    [workspaceId, links, reload, pushHistory],
  )

  const setAgentColor = useCallback(
    (agentIds: string[], color: string | null) => {
      if (!workspaceId || agentIds.length === 0) return
      const agentIdSet = new Set(agentIds)
      const previousColors = new Map(
        agents.filter((agent) => agentIdSet.has(agent.id)).map((agent) => [agent.id, agent.color ?? null] as const),
      )
      setAgents((previous) =>
        previous.map((agent) => (agentIdSet.has(agent.id) ? { ...agent, color } : agent)),
      )
      enqueueWrite(() =>
        Promise.all(
          agentIds.map((agentId) => desktopApi.agentCanvasSetAgentColor(workspaceId, agentId, color)),
        ),
      )
      pushHistory(async () => {
        setAgents((previous) =>
          previous.map((agent) =>
            agentIdSet.has(agent.id) ? { ...agent, color: previousColors.get(agent.id) ?? null } : agent,
          ),
        )
        // Awaited directly (not `enqueueWrite` again) — `undo()` already
        // runs this whole entry inside its own queued turn, so awaiting
        // here is what makes that turn (and so the shared queue) actually
        // wait for this write before letting anything queued after it run.
        await Promise.all(
          [...previousColors].map(([agentId, previousColor]) =>
            desktopApi.agentCanvasSetAgentColor(workspaceId, agentId, previousColor),
          ),
        )
      })
    },
    [workspaceId, agents, pushHistory, enqueueWrite],
  )

  const setLinkColor = useCallback(
    (linkIds: string[], color: string | null) => {
      if (!workspaceId || linkIds.length === 0) return
      const linkIdSet = new Set(linkIds)
      // Keyed by `AgentLink.id` (not agent pair) — an authored and a
      // derived link can share the same (fromAgentId, toAgentId) pair (see
      // `AgentCanvasPane.tsx`'s `resolveSelectedAuthoredLinks`, which only
      // ever selects authored ones), so matching by pair alone would also
      // revert an unrelated derived link's color on undo. `id` stays valid
      // here since this function never recreates the link (unlike
      // `deleteAuthoredLink`'s undo, which has to re-address by pair because
      // the recreated link gets a new id).
      const previous = links
        .filter((link) => linkIdSet.has(link.id))
        .map((link) => ({
          id: link.id,
          fromAgentId: link.fromAgentId,
          toAgentId: link.toAgentId,
          previousColor: link.color ?? null,
        }))
      setLinks((previousLinks) => previousLinks.map((link) => (linkIdSet.has(link.id) ? { ...link, color } : link)))
      enqueueWrite(() =>
        Promise.all(
          links
            .filter((link) => linkIdSet.has(link.id))
            .map((link) => desktopApi.agentCanvasSetLinkColor(workspaceId, link.fromAgentId, link.toAgentId, color)),
        ),
      )
      pushHistory(async () => {
        setLinks((previousLinks) =>
          previousLinks.map((link) => {
            const match = previous.find((entry) => entry.id === link.id)
            return match ? { ...link, color: match.previousColor } : link
          }),
        )
        // Awaited directly (not `enqueueWrite`) — `undo()` already runs this
        // whole entry inside its own queued turn on the shared write queue,
        // so awaiting here is what makes that turn (and so the queue) wait
        // for this write before anything queued after it runs — see the
        // forward write above, which enqueues onto that same queue.
        await Promise.all(
          previous.map((entry) =>
            desktopApi.agentCanvasSetLinkColor(workspaceId, entry.fromAgentId, entry.toAgentId, entry.previousColor),
          ),
        )
      })
    },
    [workspaceId, links, pushHistory, enqueueWrite],
  )

  const setLinkBidirectional = useCallback(
    (linkIds: string[], bidirectional: boolean) => {
      if (!workspaceId || linkIds.length === 0) return
      const linkIdSet = new Set(linkIds)
      // Keyed by `AgentLink.id`, same reasoning as `setLinkColor` above.
      const previous = links
        .filter((link) => linkIdSet.has(link.id))
        .map((link) => ({
          id: link.id,
          fromAgentId: link.fromAgentId,
          toAgentId: link.toAgentId,
          previousBidirectional: Boolean(link.bidirectional),
        }))
      setLinks((previousLinks) =>
        previousLinks.map((link) => (linkIdSet.has(link.id) ? { ...link, bidirectional } : link)),
      )
      enqueueWrite(() =>
        Promise.all(
          links
            .filter((link) => linkIdSet.has(link.id))
            .map((link) =>
              desktopApi.agentCanvasSetLinkBidirectional(
                workspaceId,
                link.fromAgentId,
                link.toAgentId,
                bidirectional,
              ),
            ),
        ),
      )
      pushHistory(async () => {
        setLinks((previousLinks) =>
          previousLinks.map((link) => {
            const match = previous.find((entry) => entry.id === link.id)
            return match ? { ...link, bidirectional: match.previousBidirectional } : link
          }),
        )
        await Promise.all(
          previous.map((entry) =>
            desktopApi.agentCanvasSetLinkBidirectional(
              workspaceId,
              entry.fromAgentId,
              entry.toAgentId,
              entry.previousBidirectional,
            ),
          ),
        )
      })
    },
    [workspaceId, links, pushHistory, enqueueWrite],
  )

  const deleteDerivedLink = useCallback(
    async (fromAgentId: string, toAgentId: string) => {
      if (!workspaceId) return
      await desktopApi.agentCanvasDeleteDerivedLink(workspaceId, fromAgentId, toAgentId)
      await reload()
    },
    [workspaceId, reload],
  )

  return {
    graph,
    loaded,
    isEmpty: loaded && agents.length === 0,
    commitInstancePosition,
    commitMcpNodePosition,
    commitSkillNodePosition,
    commitHookNodePosition,
    setMcpServerEnabled,
    createAuthoredLink,
    deleteAuthoredLink,
    deleteDerivedLink,
    removeInstancesFromCanvas,
    addInstance,
    setAgentColor,
    setLinkColor,
    setLinkBidirectional,
    undo,
  }
}
