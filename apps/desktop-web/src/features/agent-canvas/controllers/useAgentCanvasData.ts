import { useCallback, useEffect, useMemo, useState } from 'react'
import { desktopApi } from '@shell/integration/desktop-api'
import type { AgentLink, AgentProfile, AgentRuntimeStatus } from '@shell/integration/desktop-api'
import { buildAgentCanvasGraph, type AgentCanvasGraphView, type CanvasNodeInstance } from '../model/agent-canvas-graph'

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

  const removeInstancesFromCanvas = useCallback(
    (instanceIds: string[]) => {
      if (!workspaceId || instanceIds.length === 0) return
      const idSet = new Set(instanceIds)
      setInstances((previous) => {
        const next = previous.filter((instance) => !idSet.has(instance.instanceId))
        saveInstances(workspaceId, next)
        return next
      })
    },
    [workspaceId],
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
    () => buildAgentCanvasGraph(agents, links, statuses, instances),
    [agents, links, statuses, instances],
  )

  const commitAgentLayout = useCallback(
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

  const commitInstancePosition = useCallback(
    (instanceId: string, agentId: string, position: { x: number; y: number }) => {
      if (instanceId === agentId) {
        commitAgentLayout(agentId, position)
        return
      }
      if (!workspaceId) return
      setInstances((previous) => {
        const next = previous.map((instance) =>
          instance.instanceId === instanceId ? { ...instance, position } : instance,
        )
        saveInstances(workspaceId, next)
        return next
      })
    },
    [workspaceId, commitAgentLayout],
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
      const next: CanvasNodeInstance[] = [
        ...instances,
        isFirstInstanceForAgent ? { instanceId, agentId } : { instanceId, agentId, position },
      ]
      setInstances(next)
      saveInstances(workspaceId, next)
      if (isFirstInstanceForAgent) commitAgentLayout(agentId, position)
    },
    [workspaceId, instances, commitAgentLayout],
  )

  const createAuthoredLink = useCallback(
    async (fromAgentId: string, toAgentId: string) => {
      if (!workspaceId) return
      await desktopApi.agentCanvasCreateAuthoredLink(workspaceId, fromAgentId, toAgentId)
      await reload()
    },
    [workspaceId, reload],
  )

  const deleteAuthoredLink = useCallback(
    async (fromAgentId: string, toAgentId: string) => {
      if (!workspaceId) return
      await desktopApi.agentCanvasDeleteAuthoredLink(workspaceId, fromAgentId, toAgentId)
      await reload()
    },
    [workspaceId, reload],
  )

  const setAgentColor = useCallback(
    (agentIds: string[], color: string | null) => {
      if (!workspaceId || agentIds.length === 0) return
      const agentIdSet = new Set(agentIds)
      setAgents((previous) =>
        previous.map((agent) => (agentIdSet.has(agent.id) ? { ...agent, color } : agent)),
      )
      for (const agentId of agentIds) {
        void desktopApi.agentCanvasSetAgentColor(workspaceId, agentId, color)
      }
    },
    [workspaceId],
  )

  const setLinkColor = useCallback(
    (linkIds: string[], color: string | null) => {
      if (!workspaceId || linkIds.length === 0) return
      const linkIdSet = new Set(linkIds)
      setLinks((previous) => previous.map((link) => (linkIdSet.has(link.id) ? { ...link, color } : link)))
      for (const link of links) {
        if (!linkIdSet.has(link.id)) continue
        void desktopApi.agentCanvasSetLinkColor(workspaceId, link.fromAgentId, link.toAgentId, color)
      }
    },
    [workspaceId, links],
  )

  const setLinkBidirectional = useCallback(
    (linkIds: string[], bidirectional: boolean) => {
      if (!workspaceId || linkIds.length === 0) return
      const linkIdSet = new Set(linkIds)
      setLinks((previous) =>
        previous.map((link) => (linkIdSet.has(link.id) ? { ...link, bidirectional } : link)),
      )
      for (const link of links) {
        if (!linkIdSet.has(link.id)) continue
        void desktopApi.agentCanvasSetLinkBidirectional(workspaceId, link.fromAgentId, link.toAgentId, bidirectional)
      }
    },
    [workspaceId, links],
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
    createAuthoredLink,
    deleteAuthoredLink,
    deleteDerivedLink,
    removeInstancesFromCanvas,
    addInstance,
    setAgentColor,
    setLinkColor,
    setLinkBidirectional,
  }
}
