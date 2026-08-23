/**
 * Domain-agnostic node/edge shapes for `<GraphCanvas>`. `TData` carries
 * whatever domain payload the caller needs to render (a `DesignerBlock`, an
 * `AgentProfile`, ...) — the engine never looks inside it.
 */
export interface GraphCanvasNode<TData> {
  id: string
  x: number
  y: number
  /** Per-node box size override — falls back to the canvas-wide `nodeWidth`/
   * `nodeHeight` props when omitted. Lets callers render some nodes smaller
   * (e.g. agent-canvas's subagent cards) without every node needing one. */
  width?: number
  height?: number
  data: TData
}

export interface GraphCanvasEdge<TData> {
  fromId: string
  toId: string
  data: TData
}

export interface GraphEdgeGeometry {
  start: { x: number; y: number }
  end: { x: number; y: number }
  control: { x: number; y: number }
  path: string
}

/** Imperative escape hatch for callers that need viewport coordinates for
 * their own UI (e.g. a context-menu positioned in canvas space). */
export interface GraphCanvasHandle {
  getViewportElement: () => HTMLDivElement | null
  screenToCanvasPoint: (clientX: number, clientY: number) => { x: number; y: number }
  focusViewport: () => void
}

export interface GraphCanvasZoomApi {
  zoom: number
  setZoom: (zoom: number) => void
  zoomIn: () => void
  zoomOut: () => void
  resetZoom: () => void
}

/** How a marquee/box-select rectangle decides what it covers: `'contain'`
 * requires a node/edge to be fully enclosed; `'intersect'` accepts anything
 * merely overlapping. `<GraphCanvas>` derives this from drag direction
 * (left-to-right vs right-to-left) and leaves the caller to apply it. */
export type GraphCanvasMarqueeMode = 'contain' | 'intersect'

export interface GraphCanvasMarqueeRect {
  x: number
  y: number
  width: number
  height: number
}
