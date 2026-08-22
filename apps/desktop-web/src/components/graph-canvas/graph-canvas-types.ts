/**
 * Domain-agnostic node/edge shapes for `<GraphCanvas>`. `TData` carries
 * whatever domain payload the caller needs to render (a `DesignerBlock`, an
 * `AgentProfile`, ...) — the engine never looks inside it.
 */
export interface GraphCanvasNode<TData> {
  id: string
  x: number
  y: number
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
