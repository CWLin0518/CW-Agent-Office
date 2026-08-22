import {
  Fragment,
  forwardRef,
  useCallback,
  useEffect,
  useId,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import type {
  GraphCanvasEdge,
  GraphCanvasHandle,
  GraphCanvasNode,
  GraphCanvasZoomApi,
} from './graph-canvas-types'
import { computeQuadraticEdgeGeometry } from './graph-canvas-geometry'

export interface GraphCanvasProps<TNodeData, TEdgeData> {
  /** Content bounds in canvas space (pre-zoom), e.g. from a layout pass. */
  width: number
  height: number
  /** Fixed node box size, used for viewport culling and edge geometry. */
  nodeWidth: number
  nodeHeight: number
  nodes: GraphCanvasNode<TNodeData>[]
  edges: GraphCanvasEdge<TEdgeData>[]
  /** Node ids that must never be culled even when off-screen (e.g. selection). */
  pinnedNodeIds?: Set<string>
  /** Node count above which off-screen nodes/edges are culled. */
  cullingThreshold?: number
  /**
   * Identifies DOM subtrees the caller renders as canvas chrome (node
   * shells, toolbars, menus) — space-armed pan and viewport auto-focus are
   * suppressed while the pointer/keyboard target is inside one of these, so
   * panning doesn't fight with interacting with a node or a control.
   */
  isInteractiveChrome?: (target: HTMLElement) => boolean
  /** Fired once per drag on pointerup, with the final canvas-space position. */
  onCommitNodePosition: (nodeId: string, position: { x: number; y: number }) => void
  /** Fired on a pointerdown→pointerup with no meaningful movement (a click). */
  onNodeClick?: (nodeId: string) => void
  onNodeFocus?: (nodeId: string) => void
  /** Also fired for an Enter keypress directly on the node shell (not a child). */
  onNodeDoubleClick?: (nodeId: string) => void
  /**
   * Inner visual content only — the engine owns the shell div (ref, position
   * style, drag/focus/keyboard wiring) so ref access never crosses a props
   * boundary (keeps this React-Compiler-clean: refs are only ever touched
   * where they're declared, inside this file).
   */
  renderNode: (node: GraphCanvasNode<TNodeData>) => ReactNode
  /** Extra class name(s) appended to the node shell, e.g. for selection/error
   * state styling. The shell always also carries `nodeShellClassName`. */
  getNodeClassName?: (node: GraphCanvasNode<TNodeData>) => string | undefined
  getNodeAriaLabel?: (node: GraphCanvasNode<TNodeData>) => string | undefined
  getNodeAriaPressed?: (node: GraphCanvasNode<TNodeData>) => boolean | undefined
  renderEdge: (
    edge: GraphCanvasEdge<TEdgeData>,
    from: GraphCanvasNode<TNodeData>,
    to: GraphCanvasNode<TNodeData>,
    geometry: ReturnType<typeof computeQuadraticEdgeGeometry>,
    arrowMarkerUrl: string,
  ) => ReactNode
  /** Rendered as a sibling of the pannable viewport, inside the outer
   * wrapper — the natural slot for zoom controls / overlays. */
  children?: (zoomApi: GraphCanvasZoomApi) => ReactNode
  /**
   * Raw pass-through handlers wired directly onto the internal viewport div
   * (not a wrapping element — the outer wrapper must stay the direct child
   * callers' CSS expects), so `event.currentTarget` is reliably the
   * viewport itself. Use `event.target === event.currentTarget` to detect a
   * true background click, or `.closest(...)` to detect a node/chrome hit.
   */
  onViewportClick?: (event: ReactMouseEvent<HTMLDivElement>) => void
  onViewportContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void
  wrapperClassName?: string
  viewportClassName?: string
  svgClassName?: string
  nodesLayerClassName?: string
  nodeShellClassName?: string
}

interface DragState {
  nodeId: string
  startPointerX: number
  startPointerY: number
  startPosition: { x: number; y: number }
}

interface PanState {
  pointerId: number
  startPointerX: number
  startPointerY: number
  startScrollLeft: number
  startScrollTop: number
}

interface CanvasViewportWindow {
  x: number
  y: number
  width: number
  height: number
}

const ZOOM_LEVELS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2]
const DEFAULT_CULLING_THRESHOLD = 50

function GraphCanvasInner<TNodeData, TEdgeData>(
  {
    width,
    height,
    nodeWidth,
    nodeHeight,
    nodes,
    edges,
    pinnedNodeIds,
    cullingThreshold = DEFAULT_CULLING_THRESHOLD,
    isInteractiveChrome,
    onCommitNodePosition,
    onNodeClick,
    onNodeFocus,
    onNodeDoubleClick,
    renderNode,
    getNodeClassName,
    getNodeAriaLabel,
    getNodeAriaPressed,
    renderEdge,
    children,
    onViewportClick,
    onViewportContextMenu,
    wrapperClassName = 'graph-canvas',
    viewportClassName = 'graph-canvas-viewport',
    svgClassName = 'graph-canvas-svg',
    nodesLayerClassName = 'graph-canvas-nodes',
    nodeShellClassName = 'graph-canvas-node-shell',
  }: GraphCanvasProps<TNodeData, TEdgeData>,
  forwardedRef: React.ForwardedRef<GraphCanvasHandle>,
) {
  const [zoom, setZoomRaw] = useState(1)
  const setZoom = useCallback((next: number) => {
    setZoomRaw(Math.min(2, Math.max(0.4, Number.isFinite(next) ? next : 1)))
  }, [])
  const zoomApi = useMemo<GraphCanvasZoomApi>(
    () => ({
      zoom,
      setZoom,
      zoomIn: () => {
        const idx = ZOOM_LEVELS.findIndex((z) => z > zoom)
        if (idx !== -1) setZoom(ZOOM_LEVELS[idx])
      },
      zoomOut: () => {
        const reversed = [...ZOOM_LEVELS].reverse()
        const idx = reversed.findIndex((z) => z < zoom)
        if (idx !== -1) setZoom(reversed[idx])
      },
      resetZoom: () => setZoom(1),
    }),
    [zoom, setZoom],
  )

  const viewportRef = useRef<HTMLDivElement | null>(null)
  const [spacePanArmed, setSpacePanArmed] = useState(false)
  const panStateRef = useRef<PanState | null>(null)

  useImperativeHandle(
    forwardedRef,
    (): GraphCanvasHandle => ({
      getViewportElement: () => viewportRef.current,
      screenToCanvasPoint: (clientX, clientY) => {
        const viewport = viewportRef.current
        if (!viewport) return { x: 0, y: 0 }
        const rect = viewport.getBoundingClientRect()
        return {
          x: (clientX - rect.left + viewport.scrollLeft) / zoom,
          y: (clientY - rect.top + viewport.scrollTop) / zoom,
        }
      },
      focusViewport: () => viewportRef.current?.focus({ preventScroll: true }),
    }),
    [zoom],
  )

  // Zoom keyboard shortcuts (Cmd/Ctrl+0/+/-).
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const meta = event.metaKey || event.ctrlKey
      if (!meta) return
      const target = event.target as HTMLElement | null
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) {
        return
      }
      if (event.key === '0') {
        event.preventDefault()
        setZoom(1)
      } else if (event.key === '=' || event.key === '+') {
        event.preventDefault()
        setZoom(zoom + 0.1)
      } else if (event.key === '-') {
        event.preventDefault()
        setZoom(zoom - 0.1)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [zoom, setZoom])

  // Space-armed pan mode.
  useEffect(() => {
    function isEditableTarget(target: EventTarget | null): boolean {
      const element = target as HTMLElement | null
      return Boolean(
        element &&
          (element.tagName === 'INPUT' ||
            element.tagName === 'TEXTAREA' ||
            element.tagName === 'SELECT' ||
            element.isContentEditable),
      )
    }
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null
      if (
        event.code !== 'Space' ||
        event.repeat ||
        isEditableTarget(event.target) ||
        (target && isInteractiveChrome?.(target))
      ) {
        return
      }
      event.preventDefault()
      setSpacePanArmed(true)
    }
    function onKeyUp(event: KeyboardEvent) {
      if (event.code !== 'Space') return
      setSpacePanArmed(false)
      panStateRef.current = null
      viewportRef.current?.classList.remove('is-panning')
    }
    function onBlur() {
      setSpacePanArmed(false)
      panStateRef.current = null
      viewportRef.current?.classList.remove('is-panning')
    }
    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('keyup', onKeyUp)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('keyup', onKeyUp)
      window.removeEventListener('blur', onBlur)
    }
  }, [isInteractiveChrome])

  // Wheel-zoom (Ctrl/Cmd+wheel, incl. trackpad pinch). Native listener since
  // React's onWheel is passive and can't preventDefault.
  useEffect(() => {
    const node = viewportRef.current
    if (!node) return
    function onWheel(event: WheelEvent) {
      if (!(event.ctrlKey || event.metaKey)) return
      event.preventDefault()
      const factor = Math.exp(-event.deltaY * 0.0015)
      setZoomRaw((prev) => {
        const next = Math.min(2, Math.max(0.4, prev * factor))
        return Number.isFinite(next) ? next : prev
      })
    }
    node.addEventListener('wheel', onWheel, { passive: false })
    return () => node.removeEventListener('wheel', onWheel)
  }, [])

  const dragStateRef = useRef<DragState | null>(null)
  const nodeRefs = useRef(new Map<string, HTMLDivElement | null>())

  const handleNodePointerDown = useCallback(
    (node: GraphCanvasNode<TNodeData>, event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return
      const target = event.target as HTMLElement
      if (target.closest('[data-no-drag]')) return
      dragStateRef.current = {
        nodeId: node.id,
        startPointerX: event.clientX,
        startPointerY: event.clientY,
        startPosition: { x: node.x, y: node.y },
      }
      nodeRefs.current.get(node.id)?.classList.add('is-dragging')
      event.currentTarget.setPointerCapture?.(event.pointerId)
    },
    [],
  )

  const handleNodePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const state = dragStateRef.current
      if (!state) return
      const el = nodeRefs.current.get(state.nodeId)
      if (!el) return
      const dx = (event.clientX - state.startPointerX) / zoom
      const dy = (event.clientY - state.startPointerY) / zoom
      el.style.transform = `translate3d(${dx}px, ${dy}px, 0)`
    },
    [zoom],
  )

  const handleNodePointerUp = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const state = dragStateRef.current
      if (!state) return
      const el = nodeRefs.current.get(state.nodeId)
      const dx = (event.clientX - state.startPointerX) / zoom
      const dy = (event.clientY - state.startPointerY) / zoom
      const moved = Math.abs(dx) > 1 || Math.abs(dy) > 1
      const finalPosition = { x: state.startPosition.x + dx, y: state.startPosition.y + dy }
      el?.classList.remove('is-dragging')
      if (el) el.style.transform = ''
      dragStateRef.current = null
      if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
        event.currentTarget.releasePointerCapture?.(event.pointerId)
      }
      if (moved) {
        onCommitNodePosition(state.nodeId, finalPosition)
      } else {
        onNodeClick?.(state.nodeId)
      }
    },
    [zoom, onCommitNodePosition, onNodeClick],
  )

  const handleNodePointerCancel = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const state = dragStateRef.current
    if (!state) return
    const el = nodeRefs.current.get(state.nodeId)
    el?.classList.remove('is-dragging')
    if (el) el.style.transform = ''
    dragStateRef.current = null
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture?.(event.pointerId)
    }
  }, [])

  const handleNodeKeyDown = useCallback(
    (nodeId: string, event: ReactKeyboardEvent<HTMLDivElement>) => {
      if (event.target !== event.currentTarget || event.key !== 'Enter') return
      event.preventDefault()
      onNodeDoubleClick?.(nodeId)
    },
    [onNodeDoubleClick],
  )

  const handleViewportPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const target = event.target as HTMLElement
      const isChrome = Boolean(isInteractiveChrome?.(target))
      const isEditableTarget = target.closest('input, textarea, select, [contenteditable="true"]')
      if (!isChrome && !isEditableTarget) {
        viewportRef.current?.focus({ preventScroll: true })
      }
      if (!spacePanArmed || event.button !== 0 || isChrome) {
        return
      }
      const viewport = viewportRef.current
      if (!viewport) return
      event.preventDefault()
      panStateRef.current = {
        pointerId: event.pointerId,
        startPointerX: event.clientX,
        startPointerY: event.clientY,
        startScrollLeft: viewport.scrollLeft,
        startScrollTop: viewport.scrollTop,
      }
      viewport.classList.add('is-panning')
      viewport.setPointerCapture?.(event.pointerId)
    },
    [spacePanArmed, isInteractiveChrome],
  )

  const handleViewportPointerMove = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const state = panStateRef.current
    const viewport = viewportRef.current
    if (!state || !viewport || state.pointerId !== event.pointerId) return
    event.preventDefault()
    viewport.scrollLeft = state.startScrollLeft - (event.clientX - state.startPointerX)
    viewport.scrollTop = state.startScrollTop - (event.clientY - state.startPointerY)
  }, [])

  const handleViewportPointerUp = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const state = panStateRef.current
    const viewport = viewportRef.current
    if (!state || state.pointerId !== event.pointerId) return
    panStateRef.current = null
    viewport?.classList.remove('is-panning')
    viewport?.releasePointerCapture?.(event.pointerId)
  }, [])

  const [viewportWindow, setViewportWindow] = useState<CanvasViewportWindow | null>(null)
  useEffect(() => {
    const viewport = viewportRef.current
    if (!viewport) return
    let frame = 0
    function updateViewportWindow() {
      frame = 0
      setViewportWindow({
        x: viewport!.scrollLeft / zoom,
        y: viewport!.scrollTop / zoom,
        width: viewport!.clientWidth / zoom,
        height: viewport!.clientHeight / zoom,
      })
    }
    function scheduleUpdate() {
      if (frame) return
      frame = window.requestAnimationFrame(updateViewportWindow)
    }
    updateViewportWindow()
    viewport.addEventListener('scroll', scheduleUpdate, { passive: true })
    const resizeObserver = new ResizeObserver(scheduleUpdate)
    resizeObserver.observe(viewport)
    window.addEventListener('resize', scheduleUpdate)
    return () => {
      if (frame) window.cancelAnimationFrame(frame)
      viewport.removeEventListener('scroll', scheduleUpdate)
      resizeObserver.disconnect()
      window.removeEventListener('resize', scheduleUpdate)
    }
  }, [zoom, width, height])

  const nodeById = useMemo(() => new Map(nodes.map((node) => [node.id, node])), [nodes])

  const { visibleNodes, visibleEdges } = useMemo(() => {
    if (nodes.length <= cullingThreshold || !viewportWindow) {
      return { visibleNodes: nodes, visibleEdges: edges }
    }
    const bufferX = viewportWindow.width
    const bufferY = viewportWindow.height
    const left = viewportWindow.x - bufferX
    const top = viewportWindow.y - bufferY
    const right = viewportWindow.x + viewportWindow.width + bufferX
    const bottom = viewportWindow.y + viewportWindow.height + bufferY
    const filteredNodes = nodes.filter((node) => {
      if (pinnedNodeIds?.has(node.id)) return true
      const nodeRight = node.x + nodeWidth
      const nodeBottom = node.y + nodeHeight
      return nodeRight >= left && node.x <= right && nodeBottom >= top && node.y <= bottom
    })
    const visibleIds = new Set(filteredNodes.map((node) => node.id))
    const filteredEdges = edges.filter(
      (edge) => visibleIds.has(edge.fromId) && visibleIds.has(edge.toId),
    )
    return { visibleNodes: filteredNodes, visibleEdges: filteredEdges }
  }, [nodes, edges, viewportWindow, pinnedNodeIds, cullingThreshold, nodeWidth, nodeHeight])

  const arrowMarkerId = useId()
  const arrowMarkerUrl = `url(#${arrowMarkerId})`

  const wrapperStyle: CSSProperties = { width, height, transformOrigin: '0 0' }
  if (zoom !== 1) {
    wrapperStyle.transform = `scale(${zoom})`
  }

  return (
    <div className={wrapperClassName}>
      <div
        className={`${viewportClassName}${spacePanArmed ? ' is-space-pan-armed' : ''}`}
        ref={viewportRef}
        tabIndex={-1}
        onClick={onViewportClick}
        onContextMenu={onViewportContextMenu}
        onPointerDown={handleViewportPointerDown}
        onPointerMove={handleViewportPointerMove}
        onPointerUp={handleViewportPointerUp}
        onPointerCancel={handleViewportPointerUp}
      >
        <div style={wrapperStyle}>
          <svg
            className={svgClassName}
            width={width}
            height={height}
            viewBox={`0 0 ${width} ${height}`}
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true"
          >
            <defs>
              <marker
                id={arrowMarkerId}
                viewBox="0 0 10 10"
                refX="9"
                refY="5"
                markerWidth="7"
                markerHeight="7"
                orient="auto-start-reverse"
              >
                <path d="M 0 0 L 10 5 L 0 10 z" fill="currentColor" />
              </marker>
            </defs>
            {visibleEdges.map((edge, index) => {
              const from = nodeById.get(edge.fromId)
              const to = nodeById.get(edge.toId)
              if (!from || !to) return null
              const geometry = computeQuadraticEdgeGeometry(from, to, nodeWidth, nodeHeight)
              return (
                <Fragment key={`${edge.fromId}-${edge.toId}-${index}`}>
                  {renderEdge(edge, from, to, geometry, arrowMarkerUrl)}
                </Fragment>
              )
            })}
          </svg>
          <div className={nodesLayerClassName}>
            {visibleNodes.map((node) => {
              const extraClassName = getNodeClassName?.(node)
              return (
                <div
                  key={node.id}
                  ref={(el) => {
                    nodeRefs.current.set(node.id, el)
                  }}
                  className={extraClassName ? `${nodeShellClassName} ${extraClassName}` : nodeShellClassName}
                  data-graph-node-id={node.id}
                  style={{ left: node.x, top: node.y, width: nodeWidth, minHeight: nodeHeight }}
                  role="button"
                  tabIndex={0}
                  aria-label={getNodeAriaLabel?.(node)}
                  aria-pressed={getNodeAriaPressed?.(node)}
                  onPointerDown={(event) => handleNodePointerDown(node, event)}
                  onPointerMove={handleNodePointerMove}
                  onPointerUp={handleNodePointerUp}
                  onPointerCancel={handleNodePointerCancel}
                  onFocus={() => onNodeFocus?.(node.id)}
                  onDoubleClick={() => onNodeDoubleClick?.(node.id)}
                  onKeyDown={(event) => handleNodeKeyDown(node.id, event)}
                >
                  {renderNode(node)}
                </div>
              )
            })}
          </div>
        </div>
      </div>
      {children?.(zoomApi)}
    </div>
  )
}

export const GraphCanvas = forwardRef(GraphCanvasInner) as <TNodeData, TEdgeData>(
  props: GraphCanvasProps<TNodeData, TEdgeData> & { ref?: React.ForwardedRef<GraphCanvasHandle> },
) => ReturnType<typeof GraphCanvasInner>
