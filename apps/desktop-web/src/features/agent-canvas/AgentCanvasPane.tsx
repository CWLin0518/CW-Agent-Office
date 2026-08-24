import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent as ReactDragEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import { createPortal } from 'react-dom'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'
import type { AgentLink } from '@shell/integration/desktop-api'
import {
  GraphCanvas,
  type GraphCanvasHandle,
  type GraphCanvasMarqueeMode,
  type GraphCanvasMarqueeRect,
} from '@/components/graph-canvas'
import { AgentCanvasColorSwatches } from './components/AgentCanvasColorSwatches'
import { AgentCanvasNodeCard, type AgentPortHandlers, type AgentPortKind } from './components/AgentCanvasNodeCard'
import { useAgentCanvasData } from './controllers/useAgentCanvasData'
import { parseAgentCanvasDragPayload } from './model/agent-canvas-drag'
import { AGENT_NODE_HEIGHT, AGENT_NODE_WIDTH, computePortEdgeGeometry } from './model/agent-canvas-graph'
import { statusLabel } from './model/agent-canvas-status-label'
import './AgentCanvasPane.scss'

interface AgentCanvasPaneProps {
  locale: Locale
  workspaceId: string | null
  /** Whether this pane is the one currently shown — polling pauses while
   * inactive, mirroring `BusinessDesignerPane`'s `active` prop. */
  active: boolean
  /** Fired from a node's context menu "New Subagent" action
   * (docs/cw/04_客製化設計.md §1, P4.5) — the caller owns opening the actual
   * create-agent UI (agent-canvas only requests it). */
  onRequestCreateSubagent?: (parentAgentId: string) => void
  /** Fired by a node's hover gear button — the caller owns opening the
   * actual edit-agent UI (agent-canvas only requests it), mirroring
   * `onRequestCreateSubagent`'s split. */
  onRequestEditAgent?: (agentId: string) => void
}

interface WireDragState {
  pointerId: number
  sourceAgentId: string
  sourcePortKind: AgentPortKind
  sourceClientPoint: { x: number; y: number }
  currentClientPoint: { x: number; y: number }
  /** Set when this drag started from an ALREADY-CONNECTED dot with
   * Ctrl+Shift held (docs/cw/04_客製化設計.md §1) — rewires that specific
   * link's grabbed end to wherever the drag completes (keeping the other,
   * un-grabbed end fixed) instead of creating a brand new link. */
  rewireLinkId?: string
}

interface NodeContextMenuState {
  /** An instance id (not an agent id) — see `AgentCanvasNodeData.instanceId` /
   * `CanvasNodeInstance` in `model/agent-canvas-graph.ts`. Named explicitly
   * (not `nodeId`) so a call site needing the agent id is never tempted to
   * pass this straight through unresolved. */
  instanceId: string
  clientX: number
  clientY: number
}

/** Which nodes/edges are currently selected — owned here, not by the shared
 * `GraphCanvas` engine (also used by business-designer, which has no
 * multi-select concept at all). `edgeIds` holds `AgentLink.id`s of selected
 * authored links (the only kind selectable). */
interface AgentCanvasSelection {
  nodeIds: Set<string>
  edgeIds: Set<string>
}

interface WireContextMenuState {
  clientX: number
  clientY: number
}

const EMPTY_SELECTION: AgentCanvasSelection = { nodeIds: new Set(), edgeIds: new Set() }

function isEditableEventTarget(target: EventTarget | null): boolean {
  const element = target as HTMLElement | null
  return Boolean(
    element &&
      (element.tagName === 'INPUT' ||
        element.tagName === 'TEXTAREA' ||
        element.tagName === 'SELECT' ||
        element.isContentEditable),
  )
}

// `.agent-canvas-edge-hit` (wires) is included alongside nodes/controls/menus
// so that starting a pointer gesture on a wire — a click-to-select target
// since Phase 4 — is never mistaken by the viewport for "empty background":
// `setPointerCapture` on the viewport during a marquee-start would otherwise
// redirect the wire's own compatibility `click` event away from it.
function isChromeElement(target: HTMLElement): boolean {
  return Boolean(
    target.closest(
      '.agent-canvas-node-shell, .agent-canvas-controls, .agent-canvas-context-menu, .agent-canvas-edge-hit',
    ),
  )
}

/** Finds the port element (if any) under a screen point — used on pointerup
 * to resolve a drag's drop target. Pointer capture doesn't affect hit-testing
 * (only event *routing*), but a port dot is a small (10px) target sitting
 * exactly where a wire's curve terminates — and every wire has its own wide
 * (16px) invisible hit-path for easier wire selection, which can end up
 * painted on top of that same point. Checking only the single topmost
 * element (`elementFromPoint`) would then resolve to the wire instead of the
 * port underneath it, silently failing to complete an otherwise-correct
 * drop. `elementsFromPoint` (plural) returns every element stacked at that
 * point, front-to-back, so this searches through all of them for the first
 * actual port — the wire on top no longer hides it. */
function resolvePortAt(clientX: number, clientY: number): { agentId: string; portKind: AgentPortKind } | null {
  const stack = document.elementsFromPoint(clientX, clientY)
  for (const element of stack) {
    if (!(element instanceof HTMLElement)) continue
    const portElement = element.closest<HTMLElement>('[data-port]')
    if (!portElement) continue
    const agentId = portElement.dataset.agentId
    const portKind = portElement.dataset.port
    if (agentId && (portKind === 'input' || portKind === 'output')) {
      return { agentId, portKind }
    }
  }
  return null
}

/** Screen-space (not canvas-space) wire preview path — deliberately not
 * routed through GraphCanvas's own SVG/pan-zoom transform: raw
 * `clientX`/`clientY` from pointer events are already viewport-relative, so
 * a `position: fixed` overlay tracks the cursor with no coordinate
 * conversion needed, even mid-pan/zoom. */
function buildWirePreviewPath(start: { x: number; y: number }, end: { x: number; y: number }): string {
  const offset = Math.min(120, Math.max(24, Math.abs(end.x - start.x) / 2))
  const c1 = { x: start.x + offset, y: start.y }
  const c2 = { x: end.x - offset, y: end.y }
  return `M ${start.x} ${start.y} C ${c1.x} ${c1.y}, ${c2.x} ${c2.y}, ${end.x} ${end.y}`
}

/** Node marquee hit-tests — `'contain'` requires the whole node box inside
 * the rect; `'intersect'` accepts any overlap (standard AABB test). The
 * `width`/`height` fallbacks mirror `computePortEdgeGeometry`'s defensive
 * fallback below — `buildAgentCanvasGraph` always sets both explicitly
 * today, so this is future-proofing, not a case actually hit. */
function isNodeFullyContained(
  node: { x: number; y: number; width?: number; height?: number },
  rect: GraphCanvasMarqueeRect,
): boolean {
  const width = node.width ?? AGENT_NODE_WIDTH
  const height = node.height ?? AGENT_NODE_HEIGHT
  return (
    node.x >= rect.x &&
    node.y >= rect.y &&
    node.x + width <= rect.x + rect.width &&
    node.y + height <= rect.y + rect.height
  )
}

function isNodeIntersecting(
  node: { x: number; y: number; width?: number; height?: number },
  rect: GraphCanvasMarqueeRect,
): boolean {
  const width = node.width ?? AGENT_NODE_WIDTH
  const height = node.height ?? AGENT_NODE_HEIGHT
  return (
    node.x < rect.x + rect.width &&
    node.x + width > rect.x &&
    node.y < rect.y + rect.height &&
    node.y + height > rect.y
  )
}

/** Wires have no box — their two port endpoints stand in as zero-area
 * points, so containment and intersection collapse to the same point-in-rect
 * test (no separate geometry function needed). */
function isPointInRect(point: { x: number; y: number }, rect: GraphCanvasMarqueeRect): boolean {
  return (
    point.x >= rect.x && point.x <= rect.x + rect.width && point.y >= rect.y && point.y <= rect.y + rect.height
  )
}

export function AgentCanvasPane({
  locale,
  workspaceId,
  active,
  onRequestCreateSubagent,
  onRequestEditAgent,
}: AgentCanvasPaneProps) {
  const {
    graph,
    isEmpty,
    commitInstancePosition,
    createAuthoredLink,
    deleteAuthoredLink,
    deleteDerivedLink,
    removeInstancesFromCanvas,
    addInstance,
    setAgentColor,
    setLinkColor,
    setLinkBidirectional,
  } = useAgentCanvasData(workspaceId, active)
  const [wireDrag, setWireDrag] = useState<WireDragState | null>(null)
  const [nodeContextMenu, setNodeContextMenu] = useState<NodeContextMenuState | null>(null)
  const [wireContextMenu, setWireContextMenu] = useState<WireContextMenuState | null>(null)
  const [selection, setSelection] = useState<AgentCanvasSelection>(EMPTY_SELECTION)
  // Purely a display filter — hiding derived edges never touches the
  // underlying `agent_links` rows, only what this pane renders.
  const [derivedEdgesVisible, setDerivedEdgesVisible] = useState(true)
  // Mirrors whichever port element currently holds pointer capture for the
  // in-progress drag (set in handlePortPointerDown, cleared in
  // handlePortPointerUp/Cancel) so the Escape path — which has no pointer
  // event of its own to read `currentTarget` from — can still release
  // capture explicitly. Implicit release-on-pointerup is spec behavior, but
  // this is a cross-platform desktop app (WebView2 / WKWebView / WebKitGTK
  // via Tauri) and the cost of being explicit here is one ref.
  const capturedPortRef = useRef<{ element: HTMLDivElement; pointerId: number } | null>(null)
  // Only used to convert a standby-rail drop's screen coordinates into
  // canvas space via `screenToCanvasPoint` (docs/cw/04_客製化設計.md §8,
  // P4.6) — same pattern `business-designer/DesignerGraphCanvas.tsx` already
  // uses for its own drop-position handling.
  const graphCanvasRef = useRef<GraphCanvasHandle | null>(null)

  const closeMenus = useCallback(() => {
    setNodeContextMenu(null)
    setWireContextMenu(null)
  }, [])

  // Selection/menus are pane-local UI state, not scoped to a workspace — the
  // pane stays mounted across workspace switches (`ShellRootView` keys it
  // once, not per-workspace), so without this a stale node id selected in
  // one workspace could leak into another (e.g. a Delete keypress right
  // after switching would hide the wrong workspace's agent). Deferred via
  // setTimeout(0), same as `useAgentCanvasData`'s workspace-change reload —
  // required so this reads as "react to an external change" rather than a
  // synchronous setState-in-effect (this repo's React Compiler ESLint rule).
  useEffect(() => {
    const id = window.setTimeout(() => {
      setSelection(EMPTY_SELECTION)
      closeMenus()
    }, 0)
    return () => window.clearTimeout(id)
  }, [workspaceId, closeMenus])

  // Esc cancels an in-progress wire drag / whichever context menu is open —
  // a window listener (not scoped to the canvas) mirrors how GraphCanvas
  // itself wires its own space-pan/zoom shortcuts. Keyed off booleans, not
  // the `wireDrag` object itself, so this doesn't tear down/resubscribe on
  // every pointermove while a wire is being dragged.
  const isDragging = wireDrag !== null
  const hasOpenMenu = nodeContextMenu !== null || wireContextMenu !== null
  useEffect(() => {
    if (!isDragging && !hasOpenMenu) return
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== 'Escape') return
      const captured = capturedPortRef.current
      if (captured && captured.element.hasPointerCapture?.(captured.pointerId)) {
        captured.element.releasePointerCapture(captured.pointerId)
      }
      capturedPortRef.current = null
      setWireDrag(null)
      closeMenus()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [isDragging, hasOpenMenu, closeMenus])

  const reportLinkActionError = useCallback(
    (error: unknown) => {
      window.alert(`${t(locale, 'agentCanvas.error.linkActionFailed')}: ${String(error)}`)
    },
    [locale],
  )

  const handlePortPointerDown = useCallback(
    (
      agentId: string,
      portKind: AgentPortKind,
      event: ReactPointerEvent<HTMLDivElement>,
      rewireLinkId?: string,
    ) => {
      if (event.button !== 0) return
      // An already-connected dot only starts a drag when Ctrl+Shift is held
      // (the rewire gesture) — a plain pointerdown on it does nothing here,
      // falling through to its existing right-click-only behavior. The "+"
      // (no `rewireLinkId`) is unaffected — it always starts a drag,
      // regardless of modifiers, exactly as before this feature existed.
      if (rewireLinkId && !(event.ctrlKey && event.shiftKey)) return
      event.stopPropagation()
      event.currentTarget.setPointerCapture?.(event.pointerId)
      capturedPortRef.current = { element: event.currentTarget, pointerId: event.pointerId }
      const rect = event.currentTarget.getBoundingClientRect()
      setWireDrag({
        pointerId: event.pointerId,
        sourceAgentId: agentId,
        sourcePortKind: portKind,
        sourceClientPoint: { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 },
        currentClientPoint: { x: event.clientX, y: event.clientY },
        rewireLinkId,
      })
    },
    [],
  )

  const handlePortPointerMove = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    // Read everything off `event` synchronously, here in the handler — React
    // nulls out a SyntheticEvent's `currentTarget` once the handler that
    // received it returns, and the `setWireDrag` updater below runs later
    // (during React's deferred state-update processing), so reading
    // `event.currentTarget` *inside* the updater throws on a null dereference.
    //
    // Recomputed every move (not just once at drag-start) so the preview's
    // anchor tracks the source port's live screen position — zoom (wheel or
    // Ctrl/Cmd+±) and pan aren't blocked by pointer capture and can happen
    // mid-drag, which would otherwise desync a frozen start point from the
    // port it's supposed to originate from.
    const rect = event.currentTarget.getBoundingClientRect()
    const sourceClientPoint = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
    const currentClientPoint = { x: event.clientX, y: event.clientY }
    setWireDrag((previous) =>
      previous && previous.pointerId === event.pointerId
        ? { ...previous, sourceClientPoint, currentClientPoint }
        : previous,
    )
  }, [])

  // Deliberately reads `wireDrag` from render scope (not a functional update)
  // since resolving the drop target and firing the create/delete call are
  // side effects that don't belong inside a `setState` updater.
  const handlePortPointerUp = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (!wireDrag || wireDrag.pointerId !== event.pointerId) return
      if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId)
      }
      capturedPortRef.current = null
      const target = resolvePortAt(event.clientX, event.clientY)
      const drag = wireDrag
      setWireDrag(null)
      // Common to both modes: can't drop back onto the agent you grabbed
      // from — for a fresh connection that's a self-link; for a rewire,
      // that's "the end didn't actually move anywhere."
      if (!target || target.agentId === drag.sourceAgentId) return

      if (drag.rewireLinkId) {
        // Rewire (Ctrl+Shift-drag from an already-connected dot): keep the
        // OTHER (un-grabbed) end of the link fixed, replace the grabbed
        // end's agent with wherever this was dropped. The target's specific
        // port kind doesn't matter here — unlike a fresh connection, the
        // role of the moved end is already fixed by which dot was grabbed,
        // so any port on the target agent identifies "reconnect to this
        // agent" equally well.
        const linkEdge = graph.edges.find(
          (edge) =>
            edge.data.kind === 'link' && edge.data.link.kind === 'authored' && edge.data.link.id === drag.rewireLinkId,
        )
        if (!linkEdge || linkEdge.data.kind !== 'link') return
        const { link } = linkEdge.data
        const sourceIsOutput = drag.sourcePortKind === 'output'
        const newFromAgentId = sourceIsOutput ? target.agentId : link.fromAgentId
        const newToAgentId = sourceIsOutput ? link.toAgentId : target.agentId
        // Dropping onto the link's own other (un-grabbed) end would make it
        // a self-link — invalid. (Dropping back onto the grabbed end's own
        // original agent is already excluded above, by the unconditional
        // `target.agentId === drag.sourceAgentId` check.)
        if (newFromAgentId === newToAgentId) return
        // Create before delete (not the reverse) so a failure is
        // recoverable rather than destructive: if `createAuthoredLink`
        // fails, the original link is untouched; if the follow-up
        // `deleteAuthoredLink` then fails, the user ends up with both the
        // old and new link (visible, fixable) instead of the link silently
        // vanishing.
        createAuthoredLink(newFromAgentId, newToAgentId)
          .then(() => deleteAuthoredLink(link.fromAgentId, link.toAgentId))
          .catch(reportLinkActionError)
        return
      }

      if (target.portKind === drag.sourcePortKind) return
      const sourceIsOutput = drag.sourcePortKind === 'output'
      const fromAgentId = sourceIsOutput ? drag.sourceAgentId : target.agentId
      const toAgentId = sourceIsOutput ? target.agentId : drag.sourceAgentId
      // Requirement: holding Shift while completing a port-to-port connect
      // gesture disconnects instead of connecting — without Shift, dragging
      // between two already-connected ports is a silent no-op (see
      // `create_authored_link`'s dedup), so Shift is what lets the same
      // gesture actually remove that edge.
      const action = event.shiftKey ? deleteAuthoredLink : createAuthoredLink
      action(fromAgentId, toAgentId).catch(reportLinkActionError)
    },
    [wireDrag, graph.edges, createAuthoredLink, deleteAuthoredLink, reportLinkActionError],
  )

  const handlePortPointerCancel = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    if (capturedPortRef.current?.pointerId === event.pointerId) {
      capturedPortRef.current = null
    }
    setWireDrag((previous) => (previous && previous.pointerId === event.pointerId ? null : previous))
  }, [])

  // No `onContextMenu` here — right-clicking the "+" (add connection)
  // element has nothing to disconnect (it's never an existing link);
  // disconnecting is now precisely per-dot, via `handlePortSlotContextMenu`
  // on each already-connected slot instead. `rewireLinkId`, when passed
  // (only by an already-connected dot, never the "+"), gates the drag
  // behind Ctrl+Shift inside `handlePortPointerDown` and rewires that link
  // on drop instead of creating a new one — see `handlePortPointerUp`.
  const getPortHandlers = useCallback(
    (agentId: string, portKind: AgentPortKind, rewireLinkId?: string): AgentPortHandlers => ({
      onPointerDown: (event) => handlePortPointerDown(agentId, portKind, event, rewireLinkId),
      onPointerMove: handlePortPointerMove,
      onPointerUp: handlePortPointerUp,
      onPointerCancel: handlePortPointerCancel,
    }),
    [handlePortPointerDown, handlePortPointerMove, handlePortPointerUp, handlePortPointerCancel],
  )

  const handleNodeClick = useCallback((nodeId: string, event: ReactPointerEvent<HTMLDivElement>) => {
    setSelection((previous) => {
      if (!event.shiftKey) return { nodeIds: new Set([nodeId]), edgeIds: new Set() }
      const nodeIds = new Set(previous.nodeIds)
      if (nodeIds.has(nodeId)) nodeIds.delete(nodeId)
      else nodeIds.add(nodeId)
      return { nodeIds, edgeIds: previous.edgeIds }
    })
  }, [])

  // Modifier -> what a marquee drag targets (feature 4 in
  // docs/cw/06_P4.5開發進度.md's P4.6 follow-up): plain drag = nodes only;
  // Shift = wires only; Shift+Ctrl = both. Ctrl alone (no Shift) has no
  // defined meaning here, so it falls back to the plain/nodes-only case.
  // Always REPLACES the current selection, consistent with plain-click.
  const handleMarqueeSelect = useCallback(
    (rect: GraphCanvasMarqueeRect, mode: GraphCanvasMarqueeMode, modifiers: { shiftKey: boolean; ctrlKey: boolean }) => {
      const wantsNodes = !modifiers.shiftKey || modifiers.ctrlKey
      const wantsEdges = modifiers.shiftKey
      const nodeIds = new Set<string>()
      const edgeIds = new Set<string>()
      if (wantsNodes) {
        for (const node of graph.nodes) {
          const hit = mode === 'contain' ? isNodeFullyContained(node, rect) : isNodeIntersecting(node, rect)
          if (hit) nodeIds.add(node.id)
        }
      }
      if (wantsEdges) {
        const nodeById = new Map(graph.nodes.map((node) => [node.id, node]))
        for (const edge of graph.edges) {
          if (edge.data.kind !== 'link' || edge.data.link.kind !== 'authored') continue
          const from = nodeById.get(edge.fromId)
          const to = nodeById.get(edge.toId)
          if (!from || !to) continue
          const geometry = computePortEdgeGeometry(from, to)
          const hit =
            mode === 'contain'
              ? isPointInRect(geometry.start, rect) && isPointInRect(geometry.end, rect)
              : isPointInRect(geometry.start, rect) || isPointInRect(geometry.end, rect)
          if (hit) edgeIds.add(edge.data.link.id)
        }
      }
      setSelection({ nodeIds, edgeIds })
    },
    [graph.nodes, graph.edges],
  )

  // `GraphCanvas` only knows the dragged node's id (an instance id since
  // P4.6) — resolve it back to the owning agent id via `graph.nodes` before
  // handing off to `commitInstancePosition`, which needs both to decide
  // whether this is a default instance (backend-persisted) or a duplicate
  // (client-only).
  const handleCommitNodePosition = useCallback(
    (instanceId: string, position: { x: number; y: number }) => {
      const node = graph.nodes.find((candidate) => candidate.id === instanceId)
      if (!node) return
      commitInstancePosition(instanceId, node.data.agent.id, position)
    },
    [graph.nodes, commitInstancePosition],
  )

  const handleNodeContextMenu = useCallback(
    (nodeId: string, event: ReactMouseEvent<HTMLDivElement>) => {
      event.preventDefault()
      // Right-clicking a node outside the current selection replaces it (a
      // stray right-click shouldn't bulk-affect an unrelated multi-selection);
      // right-clicking a node already inside the selection keeps it, so the
      // menu's actions (e.g. delete) apply to the whole selected set.
      setSelection((previous) =>
        previous.nodeIds.has(nodeId) ? previous : { nodeIds: new Set([nodeId]), edgeIds: new Set() },
      )
      setNodeContextMenu({ instanceId: nodeId, clientX: event.clientX, clientY: event.clientY })
    },
    [],
  )

  const handleViewportClick = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      if (event.target !== event.currentTarget) return
      closeMenus()
      setSelection(EMPTY_SELECTION)
    },
    [closeMenus],
  )

  // Standby-rail drop target (docs/cw/04_客製化設計.md §8, P4.6) — native
  // HTML5 drag-and-drop, not this pane's own pointer-based wire/marquee
  // dragging, since the drag source (left rail) and this drop target are
  // separate shell panes with no shared parent state. `getData()` isn't
  // readable until the actual `drop` event (browsers return `''` for it
  // during `dragover`/`dragenter`, by spec, for security) — so this always
  // allows the drop rather than trying to sniff the payload early; a drop
  // that turns out not to be ours is a harmless no-op in `handleCanvasDrop`.
  const handleCanvasDragOver = useCallback((event: ReactDragEvent<HTMLDivElement>) => {
    event.preventDefault()
    event.dataTransfer.dropEffect = 'copy'
  }, [])

  const handleCanvasDrop = useCallback(
    (event: ReactDragEvent<HTMLDivElement>) => {
      event.preventDefault()
      const agentId = parseAgentCanvasDragPayload(event.dataTransfer)
      if (!agentId) return
      const dropPoint = graphCanvasRef.current?.screenToCanvasPoint(event.clientX, event.clientY) ?? { x: 0, y: 0 }
      // Center the new node under the cursor rather than anchoring its
      // top-left corner there — approximates with the regular (non-subagent)
      // node size since the dropped agent's own size isn't known here
      // without an extra lookup; a subagent landing slightly off-center is a
      // harmless cosmetic approximation.
      addInstance(agentId, { x: dropPoint.x - AGENT_NODE_WIDTH / 2, y: dropPoint.y - AGENT_NODE_HEIGHT / 2 })
    },
    [addInstance],
  )

  const handleDeleteSelectedNodes = useCallback(() => {
    setSelection((previous) => {
      // `selection.nodeIds` holds instance ids (== `node.id`, per
      // `buildAgentCanvasGraph`) — pass straight through, no agentId lookup
      // needed since removal is instance-scoped, not agent-scoped.
      if (previous.nodeIds.size > 0) removeInstancesFromCanvas([...previous.nodeIds])
      return EMPTY_SELECTION
    })
    closeMenus()
  }, [removeInstancesFromCanvas, closeMenus])

  // Only authored links are selectable — derived (auto-recorded, read-only)
  // and ownership edges have no user-facing selected-state actions (feature
  // 3 in docs/cw/06_P4.5開發進度.md's P4.6 follow-up is authored-only), so
  // there's nothing for selecting them to enable.
  const handleAuthoredEdgeClick = useCallback((link: AgentLink, event: ReactMouseEvent<SVGGElement>) => {
    setSelection((previous) => {
      if (!event.shiftKey) return { nodeIds: new Set(), edgeIds: new Set([link.id]) }
      const edgeIds = new Set(previous.edgeIds)
      if (edgeIds.has(link.id)) edgeIds.delete(link.id)
      else edgeIds.add(link.id)
      return { nodeIds: previous.nodeIds, edgeIds }
    })
  }, [])

  const handlePickNodeColor = useCallback(
    (color: string | null) => {
      if (selection.nodeIds.size === 0) return
      // Color is per-agent (backend column), not per-instance — resolve
      // each selected instance id back to its agent id via `graph.nodes`
      // before calling the agent-scoped setter (multiple selected
      // instances of the same agent naturally collapse to one call each
      // via the Set below, which is harmless — same value, same agent).
      const agentIds = new Set(
        graph.nodes.filter((node) => selection.nodeIds.has(node.id)).map((node) => node.data.agent.id),
      )
      setAgentColor([...agentIds], color)
    },
    [selection.nodeIds, graph.nodes, setAgentColor],
  )

  // Delete/Backspace removes the selected node(s) from the canvas (not the
  // underlying agents) — scoped to when there's a selection and focus isn't
  // in an editable control, mirroring the existing Escape listener's shape.
  useEffect(() => {
    if (selection.nodeIds.size === 0) return
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== 'Delete' && event.key !== 'Backspace') return
      if (isEditableEventTarget(event.target)) return
      event.preventDefault()
      handleDeleteSelectedNodes()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [selection.nodeIds, handleDeleteSelectedNodes])

  // Right-clicking a wire not already selected replaces the selection with
  // just that wire (clearing any selected nodes) — mirrors
  // `handleNodeContextMenu`'s convention; a wire already inside a
  // multi-selection keeps the whole selection so the menu's actions apply
  // to every selected wire at once.
  // Untyped-element event (not `<SVGGElement>` specifically) since this is
  // shared by right-clicking the wire's own `<g>` AND right-clicking one of
  // its port dots (an `HTMLDivElement`, see `handlePortSlotContextMenu`) —
  // only `preventDefault`/`clientX`/`clientY` are read, neither element-specific.
  const handleWireContextMenu = useCallback((link: AgentLink, event: ReactMouseEvent) => {
    event.preventDefault()
    setNodeContextMenu(null)
    setSelection((previous) =>
      previous.edgeIds.has(link.id) ? previous : { nodeIds: new Set(), edgeIds: new Set([link.id]) },
    )
    setWireContextMenu({ clientX: event.clientX, clientY: event.clientY })
  }, [])

  /** Resolves `selection.edgeIds` (or any other id set) back to full
   * `AgentLink` objects — only `AgentLink.id` is stored in selection/passed
   * around elsewhere. */
  const resolveSelectedAuthoredLinks = useCallback(
    (edgeIds: ReadonlySet<string>): AgentLink[] =>
      graph.edges
        .filter((edge) => edge.data.kind === 'link' && edge.data.link.kind === 'authored' && edgeIds.has(edge.data.link.id))
        .map((edge) => (edge.data as { kind: 'link'; link: AgentLink }).link),
    [graph.edges],
  )

  // Right-clicking a node's connected port dot (one per authored link, see
  // `AgentCanvasNodeCard`'s `PortSlots`) opens the exact same wire context
  // menu right-clicking the wire's own line does — a dot is just another
  // rendering of that same link, not a separate concept.
  const handlePortSlotContextMenu = useCallback(
    (_portKind: AgentPortKind, linkId: string, event: ReactMouseEvent<HTMLDivElement>) => {
      const [link] = resolveSelectedAuthoredLinks(new Set([linkId]))
      if (link) handleWireContextMenu(link, event)
    },
    [resolveSelectedAuthoredLinks, handleWireContextMenu],
  )

  const selectedAuthoredLinks = resolveSelectedAuthoredLinks(selection.edgeIds)

  // Side effects (the confirm dialog, the delete calls) run directly in the
  // handler body, reading `selection.edgeIds` from closure — NOT inside a
  // `setSelection` updater. An updater function can run twice under
  // StrictMode (React double-invokes updaters to surface impurity bugs);
  // a plain event-handler body calling `setSelection` with a fixed value
  // does not get that treatment, so this avoids ever double-firing the
  // delete calls.
  const handleDisconnectSelectedWires = useCallback(() => {
    if (selection.edgeIds.size === 0) return
    if (!window.confirm(t(locale, 'agentCanvas.edge.deleteConfirm'))) return
    const links = resolveSelectedAuthoredLinks(selection.edgeIds)
    Promise.all(links.map((link) => deleteAuthoredLink(link.fromAgentId, link.toAgentId))).catch(
      reportLinkActionError,
    )
    setSelection(EMPTY_SELECTION)
    closeMenus()
  }, [selection.edgeIds, resolveSelectedAuthoredLinks, deleteAuthoredLink, reportLinkActionError, locale, closeMenus])

  const handlePickWireColor = useCallback(
    (color: string | null) => {
      if (selection.edgeIds.size > 0) setLinkColor([...selection.edgeIds], color)
    },
    [selection.edgeIds, setLinkColor],
  )

  const allSelectedWiresBidirectional =
    selectedAuthoredLinks.length > 0 && selectedAuthoredLinks.every((link) => link.bidirectional)

  // Same "read from closure, no setState updater" shape as
  // `handleDisconnectSelectedWires` above — depends on `selection.edgeIds`
  // (real, trackable state) and the memoized `resolveSelectedAuthoredLinks`,
  // not on the unmemoized `allSelectedWiresBidirectional` const, so the
  // React Compiler can track this dependency safely.
  const handleToggleSelectedWiresBidirectional = useCallback(() => {
    if (selection.edgeIds.size === 0) return
    const links = resolveSelectedAuthoredLinks(selection.edgeIds)
    const allBidirectional = links.length > 0 && links.every((link) => link.bidirectional)
    setLinkBidirectional([...selection.edgeIds], !allBidirectional)
  }, [selection.edgeIds, resolveSelectedAuthoredLinks, setLinkBidirectional])

  const handleDeleteDerivedEdge = useCallback(
    (fromAgentId: string, toAgentId: string, alsoReverse: boolean) => {
      if (!window.confirm(t(locale, 'agentCanvas.edge.derivedDeleteConfirm'))) return
      // A merged bidirectional edge (see `reverseLinkId` in
      // `buildAgentCanvasGraph`) is drawn as one line — clearing it should
      // clear both underlying directional rows, not leave the reverse one to
      // silently reappear as a single-arrow line. `Promise.all` (not two
      // independent `.catch`s) so a failure on either side surfaces one
      // error, not two stacked `window.alert`s — same pattern as
      // `handleDisconnectSelectedWires` above.
      const deletions = alsoReverse
        ? [deleteDerivedLink(fromAgentId, toAgentId), deleteDerivedLink(toAgentId, fromAgentId)]
        : [deleteDerivedLink(fromAgentId, toAgentId)]
      Promise.all(deletions).catch(reportLinkActionError)
    },
    [deleteDerivedLink, reportLinkActionError, locale],
  )

  const getSelectedNodeBoxes = useCallback(
    () => graph.nodes.filter((node) => selection.nodeIds.has(node.id)),
    [graph.nodes, selection.nodeIds],
  )

  // Align/distribute (feature 7 in docs/cw/06_P4.5開發進度.md's P4.6
  // follow-up) — pure position math over `selection.nodeIds`, then commits
  // each moved node exactly like a manual drag would (`commitInstancePosition`
  // already persists default instances via `agentCanvasSetLayout`, and keeps
  // duplicate instances purely client-side); no new backend/schema.
  const handleAlignSelectedNodes = useCallback(
    (direction: 'left' | 'right' | 'top' | 'bottom') => {
      const boxes = getSelectedNodeBoxes()
      if (boxes.length < 2) return
      if (direction === 'left') {
        const targetX = boxes.reduce((min, box) => Math.min(min, box.x), Infinity)
        for (const box of boxes) commitInstancePosition(box.id, box.data.agent.id, { x: targetX, y: box.y })
      } else if (direction === 'right') {
        const targetRight = boxes.reduce((max, box) => Math.max(max, box.x + (box.width ?? AGENT_NODE_WIDTH)), -Infinity)
        for (const box of boxes) {
          commitInstancePosition(box.id, box.data.agent.id, {
            x: targetRight - (box.width ?? AGENT_NODE_WIDTH),
            y: box.y,
          })
        }
      } else if (direction === 'top') {
        const targetY = boxes.reduce((min, box) => Math.min(min, box.y), Infinity)
        for (const box of boxes) commitInstancePosition(box.id, box.data.agent.id, { x: box.x, y: targetY })
      } else {
        const targetBottom = boxes.reduce((max, box) => Math.max(max, box.y + (box.height ?? AGENT_NODE_HEIGHT)), -Infinity)
        for (const box of boxes) {
          commitInstancePosition(box.id, box.data.agent.id, {
            x: box.x,
            y: targetBottom - (box.height ?? AGENT_NODE_HEIGHT),
          })
        }
      }
    },
    [getSelectedNodeBoxes, commitInstancePosition],
  )

  // Distributes by each box's CENTER point (not its raw x/y origin) — matches
  // the `AlignHorizontalDistributeCenter`/`AlignVerticalDistributeCenter`
  // icons wired to these buttons, and gives sane results for a mixed
  // regular-agent/subagent selection (different box sizes per
  // `model/agent-canvas-graph.ts`'s `AGENT_NODE_*`/`SUBAGENT_NODE_*`
  // constants) — distributing by origin would visibly diverge from what
  // those icons promise once box sizes differ.
  const handleDistributeSelectedNodes = useCallback(
    (axis: 'horizontal' | 'vertical') => {
      const boxes = getSelectedNodeBoxes()
      if (boxes.length < 3) return
      const sizeOf = (box: (typeof boxes)[number]) =>
        axis === 'horizontal' ? box.width ?? AGENT_NODE_WIDTH : box.height ?? AGENT_NODE_HEIGHT
      const originOf = (box: (typeof boxes)[number]) => (axis === 'horizontal' ? box.x : box.y)
      const centerOf = (box: (typeof boxes)[number]) => originOf(box) + sizeOf(box) / 2
      const sorted = [...boxes].sort((a, b) => centerOf(a) - centerOf(b))
      const first = sorted[0]
      const last = sorted[sorted.length - 1]
      const step = (centerOf(last) - centerOf(first)) / (sorted.length - 1)
      sorted.forEach((box, index) => {
        if (index === 0 || index === sorted.length - 1) return
        const targetOrigin = centerOf(first) + step * index - sizeOf(box) / 2
        commitInstancePosition(
          box.id,
          box.data.agent.id,
          axis === 'horizontal' ? { x: targetOrigin, y: box.y } : { x: box.x, y: targetOrigin },
        )
      })
    },
    [getSelectedNodeBoxes, commitInstancePosition],
  )

  if (isEmpty) {
    return (
      <div className="agent-canvas">
        <div className="agent-canvas-empty">
          <AppIcon name="route" aria-hidden="true" />
          <p>{t(locale, 'agentCanvas.empty')}</p>
        </div>
      </div>
    )
  }

  const visibleEdges = derivedEdgesVisible
    ? graph.edges
    : graph.edges.filter((edge) => !(edge.data.kind === 'link' && edge.data.link.kind === 'derived'))

  // The swatch row shows a color as "active" only when every selected node
  // agrees on it — a mixed multi-selection shows no active swatch, same as
  // how a mixed text selection shows no active style in a rich text editor.
  const selectedNodeColors = new Set(
    graph.nodes.filter((node) => selection.nodeIds.has(node.id)).map((node) => node.data.agent.color ?? null),
  )
  const activeNodeColor = selectedNodeColors.size === 1 ? [...selectedNodeColors][0] : null

  // Same "only active if every selected wire agrees" rule as node colors.
  const selectedWireColors = new Set(selectedAuthoredLinks.map((link) => link.color ?? null))
  const activeWireColor = selectedWireColors.size === 1 ? [...selectedWireColors][0] : null

  return (
    <GraphCanvas
      ref={graphCanvasRef}
      width={graph.bounds.width}
      height={graph.bounds.height}
      nodeWidth={AGENT_NODE_WIDTH}
      nodeHeight={AGENT_NODE_HEIGHT}
      nodes={graph.nodes}
      edges={visibleEdges}
      isInteractiveChrome={isChromeElement}
      onCommitNodePosition={handleCommitNodePosition}
      onNodeClick={handleNodeClick}
      onNodeContextMenu={handleNodeContextMenu}
      onViewportClick={handleViewportClick}
      onMarqueeSelect={handleMarqueeSelect}
      onViewportDragOver={handleCanvasDragOver}
      onViewportDrop={handleCanvasDrop}
      wrapperClassName="agent-canvas"
      viewportClassName="agent-canvas-viewport"
      svgClassName="agent-canvas-svg"
      nodesLayerClassName="agent-canvas-nodes"
      nodeShellClassName="agent-canvas-node-shell"
      marqueeClassName="agent-canvas-marquee"
      getNodeClassName={(node) =>
        `agent-canvas-node-shell--${node.data.runtimeState}${
          selection.nodeIds.has(node.id) ? ' agent-canvas-node-shell--selected' : ''
        }`
      }
      getNodeAriaPressed={(node) => selection.nodeIds.has(node.id)}
      pinnedNodeIds={selection.nodeIds}
      getNodeAriaLabel={(node) =>
        t(locale, 'agentCanvas.nodeLabel', {
          name: node.data.agent.name || node.data.agent.id,
          status: statusLabel(locale, node.data.runtimeState),
        })
      }
      renderNode={(node) => (
        <AgentCanvasNodeCard
          node={node.data}
          locale={locale}
          getPortHandlers={(portKind, rewireLinkId) => getPortHandlers(node.data.agent.id, portKind, rewireLinkId)}
          onPortSlotContextMenu={handlePortSlotContextMenu}
          onRequestEdit={onRequestEditAgent}
        />
      )}
      renderEdge={(edge, from, to, _engineGeometry, arrowMarkerUrl) => {
        if (edge.data.kind === 'ownership') {
          // Center-anchored (no slot args) — ownership lines don't consume a
          // port dot, unchanged from before per-connection slots existed.
          const geometry = computePortEdgeGeometry(from, to)
          return (
            <g pointerEvents="none">
              <title>{t(locale, 'agentCanvas.edge.ownershipTitle')}</title>
              <path d={geometry.path} className="agent-canvas-edge agent-canvas-edge--ownership" />
            </g>
          )
        }
        const { link, reverseLinkId } = edge.data
        if (link.kind === 'derived') {
          // Also center-anchored — derived lines are read-only observations,
          // never created via drag, so they don't need/get their own slot.
          const geometry = computePortEdgeGeometry(from, to)
          // `reverseLinkId` is only set when the opposite direction was ALSO
          // recorded (docs on `AgentCanvasEdgeData.reverseLinkId`) — draw a
          // double-headed arrow instead of a single one so the merged edge
          // still reads as "these two talk both ways," without a second,
          // directionally-mirrored line looping back to the previous agent.
          const isBidirectional = Boolean(reverseLinkId)
          return (
            <g
              pointerEvents="auto"
              className="agent-canvas-edge-hit"
              onContextMenu={(event) => {
                event.preventDefault()
                handleDeleteDerivedEdge(link.fromAgentId, link.toAgentId, isBidirectional)
              }}
            >
              <title>{t(locale, 'agentCanvas.edge.derivedTitle')}</title>
              {/* Invisible wide stroke widens the clickable/right-clickable
                  area well beyond the thin visible line below — a bare
                  1.5px stroke is a tiny, easy-to-miss target. */}
              <path d={geometry.path} className="agent-canvas-edge-hit-path" />
              <path
                d={geometry.path}
                className="agent-canvas-edge agent-canvas-edge--derived"
                markerStart={isBidirectional ? arrowMarkerUrl : undefined}
                markerEnd={arrowMarkerUrl}
              />
            </g>
          )
        }
        // Authored links each get their own port dot on both ends (see
        // `AgentCanvasNodeCard`'s `PortSlots`) — anchor the curve to the
        // exact same slot the dot renders at, via `outputLinkIds`/
        // `inputLinkIds`' index of this specific link, so the line visually
        // starts/ends right at its own dot instead of the node's center.
        const fromTotal = from.data.outputLinkIds.length + 1
        const fromIndex = from.data.outputLinkIds.indexOf(link.id)
        const toTotal = to.data.inputLinkIds.length + 1
        const toIndex = to.data.inputLinkIds.indexOf(link.id)
        // Should be unreachable — `outputLinkIds`/`inputLinkIds` are built
        // from the same pass over `links` that produces this very edge (see
        // `buildAgentCanvasGraph`). Falling back to index 0 keeps rendering
        // instead of crashing, but a future refactor breaking that
        // invariant deserves a visible signal, not a silently overlapping wire.
        if (import.meta.env.DEV && (fromIndex === -1 || toIndex === -1)) {
          console.warn('[agent-canvas] authored link missing from its own node port-slot list', link.id)
        }
        const geometry = computePortEdgeGeometry(
          from,
          to,
          { index: fromIndex === -1 ? 0 : fromIndex, total: fromTotal },
          { index: toIndex === -1 ? 0 : toIndex, total: toTotal },
        )
        const isSelected = selection.edgeIds.has(link.id)
        return (
          <g
            pointerEvents="auto"
            className="agent-canvas-edge-hit"
            onClick={(event) => handleAuthoredEdgeClick(link, event)}
            onContextMenu={(event) => handleWireContextMenu(link, event)}
          >
            <title>{t(locale, 'agentCanvas.edge.authoredTitle')}</title>
            {/* Invisible wide stroke widens the clickable/right-clickable
                area well beyond the thin visible line below — a bare 1.5px
                stroke is a tiny, easy-to-miss target for left-click select. */}
            <path d={geometry.path} className="agent-canvas-edge-hit-path" />
            <path
              d={geometry.path}
              className={`agent-canvas-edge agent-canvas-edge--authored${
                isSelected ? ' agent-canvas-edge--selected' : ''
              }`}
              style={link.color ? ({ '--agent-canvas-edge-color': link.color } as CSSProperties) : undefined}
              markerStart={link.bidirectional ? arrowMarkerUrl : undefined}
              markerEnd={arrowMarkerUrl}
            />
          </g>
        )
      }}
    >
      {(zoomApi) => (
        <>
          <div className="agent-canvas-notice">{t(locale, 'agentCanvas.notice.edgeRequired')}</div>
          <div className="agent-canvas-controls">
            <button
              type="button"
              className={`agent-canvas-icon-button${derivedEdgesVisible ? '' : ' is-active'}`}
              onClick={() => setDerivedEdgesVisible((previous) => !previous)}
              title={t(locale, derivedEdgesVisible ? 'agentCanvas.toggleDerived.hide' : 'agentCanvas.toggleDerived.show')}
              aria-label={t(locale, derivedEdgesVisible ? 'agentCanvas.toggleDerived.hide' : 'agentCanvas.toggleDerived.show')}
              aria-pressed={!derivedEdgesVisible}
            >
              <AppIcon name={derivedEdgesVisible ? 'eye' : 'eye-off'} aria-hidden="true" />
            </button>
            <div className="agent-canvas-controls-divider" aria-hidden="true" />
            <button
              type="button"
              className="agent-canvas-icon-button"
              disabled={selection.nodeIds.size < 2}
              onClick={() => handleAlignSelectedNodes('left')}
              title={t(locale, 'agentCanvas.align.left')}
              aria-label={t(locale, 'agentCanvas.align.left')}
            >
              <AppIcon name="align-left" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="agent-canvas-icon-button"
              disabled={selection.nodeIds.size < 2}
              onClick={() => handleAlignSelectedNodes('right')}
              title={t(locale, 'agentCanvas.align.right')}
              aria-label={t(locale, 'agentCanvas.align.right')}
            >
              <AppIcon name="align-right" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="agent-canvas-icon-button"
              disabled={selection.nodeIds.size < 2}
              onClick={() => handleAlignSelectedNodes('top')}
              title={t(locale, 'agentCanvas.align.top')}
              aria-label={t(locale, 'agentCanvas.align.top')}
            >
              <AppIcon name="align-top" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="agent-canvas-icon-button"
              disabled={selection.nodeIds.size < 2}
              onClick={() => handleAlignSelectedNodes('bottom')}
              title={t(locale, 'agentCanvas.align.bottom')}
              aria-label={t(locale, 'agentCanvas.align.bottom')}
            >
              <AppIcon name="align-bottom" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="agent-canvas-icon-button"
              disabled={selection.nodeIds.size < 3}
              onClick={() => handleDistributeSelectedNodes('horizontal')}
              title={t(locale, 'agentCanvas.align.distributeHorizontal')}
              aria-label={t(locale, 'agentCanvas.align.distributeHorizontal')}
            >
              <AppIcon name="distribute-horizontal" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="agent-canvas-icon-button"
              disabled={selection.nodeIds.size < 3}
              onClick={() => handleDistributeSelectedNodes('vertical')}
              title={t(locale, 'agentCanvas.align.distributeVertical')}
              aria-label={t(locale, 'agentCanvas.align.distributeVertical')}
            >
              <AppIcon name="distribute-vertical" aria-hidden="true" />
            </button>
            <div className="agent-canvas-controls-divider" aria-hidden="true" />
            <button
              type="button"
              className="agent-canvas-icon-button"
              onClick={zoomApi.zoomIn}
              title={t(locale, 'agentCanvas.zoomIn')}
              aria-label={t(locale, 'agentCanvas.zoomIn')}
            >
              <AppIcon name="plus" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="agent-canvas-icon-button"
              onClick={zoomApi.resetZoom}
              title={t(locale, 'agentCanvas.zoomReset')}
              aria-label={t(locale, 'agentCanvas.zoomReset')}
            >
              <span className="agent-canvas-zoom-label">{Math.round(zoomApi.zoom * 100)}%</span>
            </button>
            <button
              type="button"
              className="agent-canvas-icon-button"
              onClick={zoomApi.zoomOut}
              title={t(locale, 'agentCanvas.zoomOut')}
              aria-label={t(locale, 'agentCanvas.zoomOut')}
            >
              <AppIcon name="minus" aria-hidden="true" />
            </button>
          </div>
          {nodeContextMenu &&
            createPortal(
              <div
                className="agent-canvas-context-menu"
                style={{ left: nodeContextMenu.clientX, top: nodeContextMenu.clientY }}
              >
                <button
                  type="button"
                  onClick={() => {
                    // `nodeContextMenu.instanceId` is an instance id, not an
                    // agent id (since P4.6's multi-instance model) — a
                    // subagent's `parentAgentId` needs the real agent, which
                    // only matters for non-default instances (a default
                    // instance's id happens to already equal its agent id,
                    // but a dragged-in duplicate's does not).
                    const node = graph.nodes.find((candidate) => candidate.id === nodeContextMenu.instanceId)
                    if (node) onRequestCreateSubagent?.(node.data.agent.id)
                    closeMenus()
                  }}
                >
                  {t(locale, 'agentCanvas.contextMenu.newSubagent')}
                </button>
                <div className="agent-canvas-context-menu-section-label">
                  {t(locale, 'agentCanvas.contextMenu.changeColor')}
                </div>
                <AgentCanvasColorSwatches locale={locale} activeColor={activeNodeColor} onPick={handlePickNodeColor} />
                <button type="button" onClick={handleDeleteSelectedNodes}>
                  {t(locale, 'agentCanvas.contextMenu.delete')}
                </button>
              </div>,
              document.body,
            )}
          {wireContextMenu &&
            createPortal(
              <div
                className="agent-canvas-context-menu"
                style={{ left: wireContextMenu.clientX, top: wireContextMenu.clientY }}
              >
                <div className="agent-canvas-context-menu-section-label">
                  {t(locale, 'agentCanvas.contextMenu.changeColor')}
                </div>
                <AgentCanvasColorSwatches locale={locale} activeColor={activeWireColor} onPick={handlePickWireColor} />
                <button type="button" onClick={handleToggleSelectedWiresBidirectional}>
                  {t(
                    locale,
                    allSelectedWiresBidirectional
                      ? 'agentCanvas.wire.makeUnidirectional'
                      : 'agentCanvas.wire.makeBidirectional',
                  )}
                </button>
                <button type="button" onClick={handleDisconnectSelectedWires}>
                  {t(locale, 'agentCanvas.edge.disconnect')}
                </button>
              </div>,
              document.body,
            )}
          {wireDrag &&
            createPortal(
              // `<svg>` is a replaced element — under `position: fixed` +
              // `inset: 0` alone (see .agent-canvas-wire-preview) a
              // replaced element with `width`/`height: auto` does NOT
              // stretch to fill both offsets the way a normal box would; it
              // falls back to its intrinsic default size (~300×150 in most
              // engines), so without an explicit size here the preview line
              // only ever draws inside a small top-left box instead of
              // across the whole screen. `100vw`/`100vh` sizes it
              // unambiguously to the viewport regardless of that quirk.
              <svg
                className="agent-canvas-wire-preview"
                width="100vw"
                height="100vh"
                aria-hidden="true"
              >
                <path
                  d={buildWirePreviewPath(wireDrag.sourceClientPoint, wireDrag.currentClientPoint)}
                  className="agent-canvas-wire-preview-path"
                />
              </svg>,
              document.body,
            )}
        </>
      )}
    </GraphCanvas>
  )
}
