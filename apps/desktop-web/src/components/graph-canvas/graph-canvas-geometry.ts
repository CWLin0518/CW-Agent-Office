import type { GraphEdgeGeometry } from './graph-canvas-types'

/**
 * Quadratic-curve edge geometry between two fixed-size boxes: connects the
 * right/left edges when the boxes are roughly horizontal to each other,
 * otherwise top/bottom. Pure function extracted from
 * business-designer's `EdgeSvg` (no routing engine, just a heuristic).
 */
export function computeQuadraticEdgeGeometry(
  from: { x: number; y: number },
  to: { x: number; y: number },
  nodeWidth: number,
  nodeHeight: number,
): GraphEdgeGeometry {
  const fromX = from.x + nodeWidth / 2
  const fromY = from.y + nodeHeight / 2
  const toX = to.x + nodeWidth / 2
  const toY = to.y + nodeHeight / 2
  const dx = toX - fromX
  const dy = toY - fromY
  const horizontal = Math.abs(dx) >= Math.abs(dy)
  const start = horizontal
    ? { x: from.x + (dx > 0 ? nodeWidth : 0), y: fromY }
    : { x: fromX, y: from.y + (dy > 0 ? nodeHeight : 0) }
  const end = horizontal
    ? { x: to.x + (dx > 0 ? 0 : nodeWidth), y: toY }
    : { x: toX, y: to.y + (dy > 0 ? 0 : nodeHeight) }

  const midX = (start.x + end.x) / 2
  const midY = (start.y + end.y) / 2
  const offset = Math.min(40, Math.max(16, Math.abs(horizontal ? dy : dx) / 4))
  const control = horizontal ? { x: midX, y: midY + offset } : { x: midX + offset, y: midY }
  const path = `M ${start.x} ${start.y} Q ${control.x} ${control.y} ${end.x} ${end.y}`

  return { start, end, control, path }
}
