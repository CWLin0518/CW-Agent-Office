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
  type DragEvent as ReactDragEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import type {
  GraphCanvasEdge,
  GraphCanvasHandle,
  GraphCanvasMarqueeMode,
  GraphCanvasMarqueeRect,
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
  /** Fired on a pointerdown→pointerup with no meaningful movement (a click).
   * The raw event is passed through (unexamined by the engine itself, same
   * as `onNodeContextMenu`) so callers can read modifier keys, e.g. for
   * Shift-click multi-select — a caller-owned concern, not something this
   * shared engine (also used by business-designer, which has no multi-select)
   * needs to know about. */
  onNodeClick?: (nodeId: string, event: ReactPointerEvent<HTMLDivElement>) => void
  onNodeFocus?: (nodeId: string) => void
  /** Also fired for an Enter keypress directly on the node shell (not a child). */
  onNodeDoubleClick?: (nodeId: string) => void
  /** Fired on a right-click / context-menu activation on a node shell. Callers
   * that use this should call `event.preventDefault()` themselves to suppress
   * the native browser menu. */
  onNodeContextMenu?: (nodeId: string, event: ReactMouseEvent<HTMLDivElement>) => void
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
  /**
   * Drag-a-rectangle-on-empty-background select. Mutually exclusive with
   * space-armed pan (pan wins if both are "available" — space held always
   * means the user wants to pan). The engine only computes and reports the
   * rectangle (in canvas space) plus which way it was dragged and which
   * modifier keys were held at drag-start; it has no opinion on what the
   * rectangle *selects* — that's caller-owned, same reasoning as
   * `onNodeClick` forwarding raw events instead of the engine knowing about
   * "selection." Only fires for a real drag (same movement threshold node
   * dragging uses) — a plain click on background does not call this, so the
   * caller's `onViewportClick` still handles that case unchanged.
   */
  onMarqueeSelect?: (
    rect: GraphCanvasMarqueeRect,
    mode: GraphCanvasMarqueeMode,
    modifiers: { shiftKey: boolean; ctrlKey: boolean },
  ) => void
  /** Raw pass-through native drag-and-drop handlers on the viewport, same
   * shape as `onViewportClick` — a caller (agent-canvas's standby rail drop
   * target) owns interpreting `event.dataTransfer`; the engine only forwards
   * the event. `onViewportDragOver` must call `event.preventDefault()` for
   * the browser to allow a drop here at all. */
  onViewportDragOver?: (event: ReactDragEvent<HTMLDivElement>) => void
  onViewportDrop?: (event: ReactDragEvent<HTMLDivElement>) => void
  wrapperClassName?: string
  viewportClassName?: string
  svgClassName?: string
  nodesLayerClassName?: string
  nodeShellClassName?: string
  marqueeClassName?: string
}

interface DragState {
  nodeId: string
  startPointerX: number
  startPointerY: number
  startPosition: { x: number; y: number }
}

interface PanState {
  pointerId: number
  /** Which button started this pan — right-drag-pan (2) needs its trailing
   * `contextmenu` event suppressed if the drag actually moved (see
   * `suppressNextContextMenuRef`); space-armed left-drag-pan (0) never
   * produces a context menu to begin with, so this only matters for 2. */
  button: 0 | 2
  startPointerX: number
  startPointerY: number
  startScrollLeft: number
  startScrollTop: number
}

interface MarqueeState {
  pointerId: number
  /** Canvas-space start point — fixed for the drag's duration. */
  startCanvasX: number
  startCanvasY: number
  /** Raw client start point, kept only for the same movement-threshold
   * "was this actually a drag" check node-dragging uses. */
  startClientX: number
  startClientY: number
  shiftKey: boolean
  ctrlKey: boolean
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
    onNodeContextMenu,
    renderNode,
    getNodeClassName,
    getNodeAriaLabel,
    getNodeAriaPressed,
    renderEdge,
    children,
    onViewportClick,
    onViewportContextMenu,
    onMarqueeSelect,
    onViewportDragOver,
    onViewportDrop,
    wrapperClassName = 'graph-canvas',
    viewportClassName = 'graph-canvas-viewport',
    svgClassName = 'graph-canvas-svg',
    nodesLayerClassName = 'graph-canvas-nodes',
    nodeShellClassName = 'graph-canvas-node-shell',
    marqueeClassName = 'graph-canvas-marquee',
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

  const clientPointToCanvas = useCallback(
    (clientX: number, clientY: number): { x: number; y: number } => {
      const viewport = viewportRef.current
      if (!viewport) return { x: 0, y: 0 }
      const rect = viewport.getBoundingClientRect()
      return {
        x: (clientX - rect.left + viewport.scrollLeft) / zoom,
        y: (clientY - rect.top + viewport.scrollTop) / zoom,
      }
    },
    [zoom],
  )

  useImperativeHandle(
    forwardedRef,
    (): GraphCanvasHandle => ({
      getViewportElement: () => viewportRef.current,
      screenToCanvasPoint: clientPointToCanvas,
      focusViewport: () => viewportRef.current?.focus({ preventScroll: true }),
    }),
    [clientPointToCanvas],
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
        onNodeClick?.(state.nodeId, event)
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

  const marqueeStateRef = useRef<MarqueeState | null>(null)
  const [marqueeRect, setMarqueeRect] = useState<GraphCanvasMarqueeRect | null>(null)
  // Coalesces `setMarqueeRect` to at most once per animation frame — raw
  // pointermove can fire faster than the display refresh rate, and (unlike
  // node-dragging, which mutates a DOM ref directly) marquee re-renders every
  // visible node/edge on each update, so this follows the same
  // `requestAnimationFrame`-coalescing precedent as this file's own
  // `viewportWindow` scroll/resize updater below.
  const marqueeMoveFrameRef = useRef(0)
  const pendingMarqueeMoveRef = useRef<{ clientX: number; clientY: number } | null>(null)
  // Set right before a real marquee drag (movement past the threshold)
  // calls `onMarqueeSelect`, so the `click` event the browser still
  // synthesizes on pointerup doesn't immediately reach `onViewportClick` and
  // clear the selection the drag just made — background pointerdown/up with
  // no movement (a plain click) never sets this, so that case is unaffected.
  const suppressNextClickRef = useRef(false)
  // Set right before a real right-drag-pan (movement past the threshold)
  // finishes, so the `contextmenu` event the browser still fires on right
  // mouseup doesn't pop up a menu right after panning — a plain right-click
  // with no movement never sets this, so `onViewportContextMenu` still
  // fires normally for that case (see `handleViewportContextMenuInternal`).
  const suppressNextContextMenuRef = useRef(false)

  const handleViewportPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const target = event.target as HTMLElement
      const isChrome = Boolean(isInteractiveChrome?.(target))
      const isEditableTarget = target.closest('input, textarea, select, [contenteditable="true"]')
      if (!isChrome && !isEditableTarget) {
        viewportRef.current?.focus({ preventScroll: true })
      }
      if (isChrome) return
      const viewport = viewportRef.current
      if (!viewport) return

      // Right-drag pan — always available (not gated behind space, unlike
      // the left-button pan below), same underlying pan mechanism either
      // way. Node/wire/port right-clicks never reach here: they're all
      // covered by `isInteractiveChrome` (nodes/ports via
      // `.agent-canvas-node-shell`, wires via `.agent-canvas-edge-hit`),
      // which already bailed out above — this only ever starts for a
      // right-mousedown on empty canvas background. `isEditableTarget` is
      // excluded too, mirroring the marquee-select branch below it — no
      // current caller renders an editable field directly on canvas
      // background (outside chrome), but if one ever does, a right-drag
      // there should behave like normal text interaction, not hijack it
      // into a pan.
      if (event.button === 2 && !isEditableTarget) {
        event.preventDefault()
        panStateRef.current = {
          pointerId: event.pointerId,
          button: 2,
          startPointerX: event.clientX,
          startPointerY: event.clientY,
          startScrollLeft: viewport.scrollLeft,
          startScrollTop: viewport.scrollTop,
        }
        viewport.classList.add('is-panning')
        viewport.setPointerCapture?.(event.pointerId)
        return
      }

      if (event.button !== 0) return
      if (spacePanArmed) {
        event.preventDefault()
        panStateRef.current = {
          pointerId: event.pointerId,
          button: 0,
          startPointerX: event.clientX,
          startPointerY: event.clientY,
          startScrollLeft: viewport.scrollLeft,
          startScrollTop: viewport.scrollTop,
        }
        viewport.classList.add('is-panning')
        viewport.setPointerCapture?.(event.pointerId)
        return
      }
      if (!onMarqueeSelect || isEditableTarget) return
      event.preventDefault()
      const canvasPoint = clientPointToCanvas(event.clientX, event.clientY)
      marqueeStateRef.current = {
        pointerId: event.pointerId,
        startCanvasX: canvasPoint.x,
        startCanvasY: canvasPoint.y,
        startClientX: event.clientX,
        startClientY: event.clientY,
        shiftKey: event.shiftKey,
        ctrlKey: event.ctrlKey || event.metaKey,
      }
      setMarqueeRect({ x: canvasPoint.x, y: canvasPoint.y, width: 0, height: 0 })
      viewport.classList.add('is-marqueeing')
      viewport.setPointerCapture?.(event.pointerId)
    },
    [spacePanArmed, isInteractiveChrome, onMarqueeSelect, clientPointToCanvas],
  )

  const handleViewportPointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const panState = panStateRef.current
      const viewport = viewportRef.current
      if (panState && viewport && panState.pointerId === event.pointerId) {
        event.preventDefault()
        viewport.scrollLeft = panState.startScrollLeft - (event.clientX - panState.startPointerX)
        viewport.scrollTop = panState.startScrollTop - (event.clientY - panState.startPointerY)
        return
      }
      const marqueeState = marqueeStateRef.current
      if (!marqueeState || marqueeState.pointerId !== event.pointerId) return
      pendingMarqueeMoveRef.current = { clientX: event.clientX, clientY: event.clientY }
      if (marqueeMoveFrameRef.current) return
      marqueeMoveFrameRef.current = window.requestAnimationFrame(() => {
        marqueeMoveFrameRef.current = 0
        const pending = pendingMarqueeMoveRef.current
        const state = marqueeStateRef.current
        if (!pending || !state) return
        const current = clientPointToCanvas(pending.clientX, pending.clientY)
        setMarqueeRect({
          x: Math.min(state.startCanvasX, current.x),
          y: Math.min(state.startCanvasY, current.y),
          width: Math.abs(current.x - state.startCanvasX),
          height: Math.abs(current.y - state.startCanvasY),
        })
      })
    },
    [clientPointToCanvas],
  )

  /** Shared by pointerup (`commit=true`) and pointercancel (`commit=false`)
   * — cancel always discards without ever calling `onMarqueeSelect` or
   * setting `suppressNextClickRef`, matching how `handleNodePointerCancel`
   * treats a cancelled node-drag as "discard," not "commit," elsewhere in
   * this file. Returns whether a marquee drag was actually in progress. */
  const finishMarqueeDrag = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>, commit: boolean): boolean => {
      const marqueeState = marqueeStateRef.current
      if (!marqueeState || marqueeState.pointerId !== event.pointerId) return false
      marqueeStateRef.current = null
      if (marqueeMoveFrameRef.current) {
        window.cancelAnimationFrame(marqueeMoveFrameRef.current)
        marqueeMoveFrameRef.current = 0
      }
      const viewport = viewportRef.current
      viewport?.classList.remove('is-marqueeing')
      viewport?.releasePointerCapture?.(event.pointerId)
      if (commit && onMarqueeSelect) {
        const dx = (event.clientX - marqueeState.startClientX) / zoom
        const dy = (event.clientY - marqueeState.startClientY) / zoom
        const moved = Math.abs(dx) > 1 || Math.abs(dy) > 1
        if (moved) {
          const current = clientPointToCanvas(event.clientX, event.clientY)
          const rect: GraphCanvasMarqueeRect = {
            x: Math.min(marqueeState.startCanvasX, current.x),
            y: Math.min(marqueeState.startCanvasY, current.y),
            width: Math.abs(current.x - marqueeState.startCanvasX),
            height: Math.abs(current.y - marqueeState.startCanvasY),
          }
          // "From left to right" vs "from right to left" per the product
          // requirement — a plain client-X comparison (canvas-space is a
          // monotonic transform of client-space, so the sign is identical).
          const mode: GraphCanvasMarqueeMode = current.x >= marqueeState.startCanvasX ? 'contain' : 'intersect'
          suppressNextClickRef.current = true
          onMarqueeSelect(rect, mode, { shiftKey: marqueeState.shiftKey, ctrlKey: marqueeState.ctrlKey })
        }
      }
      setMarqueeRect(null)
      return true
    },
    [zoom, clientPointToCanvas, onMarqueeSelect],
  )

  const handleViewportPointerUp = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const panState = panStateRef.current
      const viewport = viewportRef.current
      if (panState && panState.pointerId === event.pointerId) {
        panStateRef.current = null
        viewport?.classList.remove('is-panning')
        viewport?.releasePointerCapture?.(event.pointerId)
        // Right-drag-pan's trailing `contextmenu` event only needs
        // suppressing if the drag actually moved — a right mousedown+up
        // with no movement in between is a plain right-click, which should
        // still open whatever `onViewportContextMenu` normally shows.
        if (panState.button === 2) {
          const dx = event.clientX - panState.startPointerX
          const dy = event.clientY - panState.startPointerY
          if (Math.abs(dx) > 1 || Math.abs(dy) > 1) suppressNextContextMenuRef.current = true
        }
        return
      }
      finishMarqueeDrag(event, true)
    },
    [finishMarqueeDrag],
  )

  const handleViewportPointerCancel = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const panState = panStateRef.current
      const viewport = viewportRef.current
      if (panState && panState.pointerId === event.pointerId) {
        panStateRef.current = null
        viewport?.classList.remove('is-panning')
        viewport?.releasePointerCapture?.(event.pointerId)
        return
      }
      finishMarqueeDrag(event, false)
    },
    [finishMarqueeDrag],
  )

  // Focus loss (Alt-Tab, etc.) mid-drag never delivers a pointerup/cancel to
  // this window — without this, the marquee rectangle and its ref would be
  // stuck indefinitely, mirroring the existing space-pan blur handler above
  // (which has the same problem for panning) but scoped to marquee state.
  useEffect(() => {
    function onBlur() {
      if (marqueeMoveFrameRef.current) {
        window.cancelAnimationFrame(marqueeMoveFrameRef.current)
        marqueeMoveFrameRef.current = 0
      }
      if (marqueeStateRef.current) {
        viewportRef.current?.classList.remove('is-marqueeing')
        viewportRef.current?.releasePointerCapture?.(marqueeStateRef.current.pointerId)
      }
      marqueeStateRef.current = null
      setMarqueeRect(null)
    }
    window.addEventListener('blur', onBlur)
    return () => window.removeEventListener('blur', onBlur)
  }, [])

  const handleViewportClickInternal = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      if (suppressNextClickRef.current) {
        suppressNextClickRef.current = false
        return
      }
      onViewportClick?.(event)
    },
    [onViewportClick],
  )

  const handleViewportContextMenuInternal = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      if (suppressNextContextMenuRef.current) {
        suppressNextContextMenuRef.current = false
        event.preventDefault()
        return
      }
      onViewportContextMenu?.(event)
    },
    [onViewportContextMenu],
  )

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
      const nodeRight = node.x + (node.width ?? nodeWidth)
      const nodeBottom = node.y + (node.height ?? nodeHeight)
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
        onClick={handleViewportClickInternal}
        onContextMenu={handleViewportContextMenuInternal}
        onDragOver={onViewportDragOver}
        onDrop={onViewportDrop}
        onPointerDown={handleViewportPointerDown}
        onPointerMove={handleViewportPointerMove}
        onPointerUp={handleViewportPointerUp}
        onPointerCancel={handleViewportPointerCancel}
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
                // The anchor point (refX, refY) is placed exactly at the
                // path's endpoint; the visible tip sits at local x=10 in this
                // viewBox. Any refX < 10 leaves a gap between the anchor and
                // the tip, which "auto"/"auto-start-reverse" then rotates
                // into the path's tangent direction — for `markerEnd` that
                // overshoots the tip a bit further FORWARD past the true
                // endpoint (usually hidden under the port dot painted on top
                // of the wire), but for `markerStart` (`orient="auto-start-
                // reverse"` flips the start marker 180°) it overshoots
                // BACKWARD, i.e. visibly past the dot on a bidirectional
                // wire's tail end. refX="10" (== the tip's own x) puts the
                // tip exactly on the path's mathematical endpoint in both
                // cases, matching the port-dot-center math in
                // `agent-canvas-graph.ts`'s `PORT_EDGE_HANG`.
                refX="10"
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
                  style={{
                    left: node.x,
                    top: node.y,
                    width: node.width ?? nodeWidth,
                    minHeight: node.height ?? nodeHeight,
                  }}
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
                  onContextMenu={(event) => onNodeContextMenu?.(node.id, event)}
                  onKeyDown={(event) => handleNodeKeyDown(node.id, event)}
                >
                  {renderNode(node)}
                </div>
              )
            })}
          </div>
          {marqueeRect && (
            <div
              className={marqueeClassName}
              style={{
                position: 'absolute',
                left: marqueeRect.x,
                top: marqueeRect.y,
                width: marqueeRect.width,
                height: marqueeRect.height,
                pointerEvents: 'none',
              }}
            />
          )}
        </div>
      </div>
      {children?.(zoomApi)}
    </div>
  )
}

export const GraphCanvas = forwardRef(GraphCanvasInner) as <TNodeData, TEdgeData>(
  props: GraphCanvasProps<TNodeData, TEdgeData> & { ref?: React.ForwardedRef<GraphCanvasHandle> },
) => ReturnType<typeof GraphCanvasInner>
