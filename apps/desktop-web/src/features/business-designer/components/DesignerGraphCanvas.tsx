import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from 'react'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'
import { GraphCanvas } from '@/components/graph-canvas'
import type { GraphCanvasHandle, GraphEdgeGeometry } from '@/components/graph-canvas'
import type { DesignerBlock } from '../model/designer-blocks'
import { designerBlockKindLabel } from '../model/designer-block-labels'
import type { DesignerLayoutPosition } from '../model/designer-document'
import {
  buildGraphView,
  NODE_HEIGHT,
  NODE_WIDTH,
  normalizeDesignerNodePosition,
  type DesignerGraphEdge,
  type DesignerGraphNode,
} from '../model/designer-graph'
import type { DesignerDerivedEdge, DesignerGap } from '../model/designer-validation'

interface DesignerGraphCanvasProps {
  locale: Locale
  blocks: DesignerBlock[]
  gaps: DesignerGap[]
  edges: DesignerDerivedEdge[]
  layout: Record<string, DesignerLayoutPosition> | null | undefined
  selectedBlockId: string | null
  drillBlockId: string | null
  onSelectBlock: (blockId: string | null) => void
  onOpenDrill: (blockId: string) => void
  onCloseDrill: () => void
  onMoveBlock: (blockId: string, position: DesignerLayoutPosition) => void
  onDeleteBlock: (block: DesignerBlock) => void
  onCreateBlock: (kind: DesignerCanvasCreateKind, position?: DesignerLayoutPosition) => void
}

export type DesignerCanvasCreateKind = 'entityModel' | 'businessFlow' | 'apiContract'

const CREATE_KINDS: DesignerCanvasCreateKind[] = ['entityModel', 'businessFlow', 'apiContract']

const INTERACTIVE_CHROME_SELECTOR =
  '.designer-node-shell, .designer-canvas-controls, .designer-canvas-create-menu'

function isInteractiveChrome(target: HTMLElement): boolean {
  return Boolean(target.closest(INTERACTIVE_CHROME_SELECTOR))
}

export const DesignerGraphCanvas = memo(function DesignerGraphCanvas({
  locale,
  blocks,
  gaps,
  edges,
  layout,
  selectedBlockId,
  drillBlockId,
  onSelectBlock,
  onOpenDrill,
  onCloseDrill,
  onMoveBlock,
  onDeleteBlock,
  onCreateBlock,
}: DesignerGraphCanvasProps) {
  const view = useMemo(() => buildGraphView(blocks, gaps, edges, layout), [
    blocks,
    gaps,
    edges,
    layout,
  ])

  const graphRef = useRef<GraphCanvasHandle | null>(null)

  const [createMenu, setCreateMenu] = useState<{
    clientX: number
    clientY: number
    canvasX: number
    canvasY: number
  } | null>(null)
  const createMenuRef = useRef<HTMLDivElement | null>(null)

  const openCreateMenuAt = useCallback((clientX: number, clientY: number) => {
    const point = graphRef.current?.screenToCanvasPoint(clientX, clientY)
    if (!point) return
    setCreateMenu({ clientX, clientY, canvasX: point.x, canvasY: point.y })
  }, [])

  useEffect(() => {
    if (!createMenu) return
    createMenuRef.current
      ?.querySelector<HTMLButtonElement>('.designer-canvas-create-menu-item')
      ?.focus()
  }, [createMenu])

  const handleCreateMenuKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (
      event.key !== 'ArrowDown' &&
      event.key !== 'ArrowUp' &&
      event.key !== 'Home' &&
      event.key !== 'End' &&
      event.key !== 'Escape'
    ) {
      return
    }
    event.preventDefault()
    if (event.key === 'Escape') {
      setCreateMenu(null)
      graphRef.current?.focusViewport()
      return
    }
    const buttons = Array.from(
      createMenuRef.current?.querySelectorAll<HTMLButtonElement>(
        '.designer-canvas-create-menu-item',
      ) ?? [],
    )
    if (buttons.length === 0) return
    const currentIndex = buttons.findIndex((button) => button === document.activeElement)
    const nextIndex =
      event.key === 'Home'
        ? 0
        : event.key === 'End'
          ? buttons.length - 1
          : event.key === 'ArrowDown'
            ? (Math.max(0, currentIndex) + 1) % buttons.length
            : (currentIndex <= 0 ? buttons.length : currentIndex) - 1
    buttons[nextIndex]?.focus()
  }, [])

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const meta = event.metaKey || event.ctrlKey
      if (!meta || event.key.toLowerCase() !== 'n') {
        return
      }
      const target = event.target as HTMLElement | null
      const viewport = graphRef.current?.getViewportElement()
      if (!viewport) return
      if (
        target &&
        (!viewport.contains(target) ||
          target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.isContentEditable)
      ) {
        return
      }
      const rect = viewport.getBoundingClientRect()
      event.preventDefault()
      openCreateMenuAt(rect.left + rect.width / 2, rect.top + rect.height / 2)
    }

    function onDismiss(event: MouseEvent) {
      if (!createMenu) return
      const target = event.target as HTMLElement | null
      if (target?.closest('.designer-canvas-create-menu')) return
      setCreateMenu(null)
    }

    function onEscape(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setCreateMenu(null)
      }
    }

    window.addEventListener('keydown', onKey)
    window.addEventListener('keydown', onEscape)
    window.addEventListener('mousedown', onDismiss)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keydown', onEscape)
      window.removeEventListener('mousedown', onDismiss)
    }
  }, [createMenu, openCreateMenuAt])

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key !== 'Backspace' && event.key !== 'Delete') {
        return
      }
      const target = event.target as HTMLElement | null
      if (
        target &&
        (target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.isContentEditable)
      ) {
        return
      }
      if (!selectedBlockId || selectedBlockId === 'brief') {
        return
      }
      const selectedBlock = blocks.find((block) => block.id === selectedBlockId)
      if (!selectedBlock) {
        return
      }
      event.preventDefault()
      onDeleteBlock(selectedBlock)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [blocks, selectedBlockId, onDeleteBlock])

  const handleViewportClick = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      // Click on background deselects.
      if (event.target === event.currentTarget) {
        onSelectBlock(null)
        onCloseDrill()
      }
    },
    [onSelectBlock, onCloseDrill],
  )

  const handleViewportContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      const nodeShell = (event.target as HTMLElement).closest<HTMLElement>('.designer-node-shell')
      event.preventDefault()
      if (nodeShell) {
        const blockId = nodeShell.dataset.graphNodeId
        if (blockId) {
          onSelectBlock(blockId)
        }
        setCreateMenu(null)
        return
      }
      onSelectBlock(null)
      openCreateMenuAt(event.clientX, event.clientY)
    },
    [onSelectBlock, openCreateMenuAt],
  )

  const graphNodes = useMemo(
    () =>
      view.nodes.map((node) => ({
        id: node.block.id,
        x: node.position.x,
        y: node.position.y,
        data: node,
      })),
    [view.nodes],
  )
  const graphEdges = useMemo(
    () =>
      view.edges.map((edge) => ({
        fromId: edge.from.id,
        toId: edge.to.id,
        data: edge,
      })),
    [view.edges],
  )
  const pinnedNodeIds = useMemo(
    () => new Set([selectedBlockId, drillBlockId].filter((id): id is string => Boolean(id))),
    [selectedBlockId, drillBlockId],
  )

  const handleCommitNodePosition = useCallback(
    (blockId: string, position: { x: number; y: number }) => {
      onMoveBlock(blockId, normalizeDesignerNodePosition(position))
    },
    [onMoveBlock],
  )

  const getNodeClassName = useCallback(
    (node: { data: DesignerGraphNode }) => {
      const selected =
        selectedBlockId === node.data.block.id || drillBlockId === node.data.block.id
      return [
        selected ? 'is-selected' : '',
        node.data.hasError ? 'has-error' : node.data.gapCount > 0 ? 'has-warning' : '',
      ]
        .filter(Boolean)
        .join(' ')
    },
    [selectedBlockId, drillBlockId],
  )

  const getNodeAriaLabel = useCallback(
    (node: { data: DesignerGraphNode }) =>
      t(locale, 'designer.canvas.nodeLabel', {
        kind: designerBlockKindLabel(locale, node.data.block.kind),
        title: node.data.block.title || node.data.block.id,
      }),
    [locale],
  )

  const getNodeAriaPressed = useCallback(
    (node: { data: DesignerGraphNode }) =>
      selectedBlockId === node.data.block.id || drillBlockId === node.data.block.id,
    [selectedBlockId, drillBlockId],
  )

  if (blocks.length === 0) {
    return (
      <div className="designer-canvas">
        <div className="designer-canvas-empty">
          <AppIcon name="designer" aria-hidden="true" />
          <p>{t(locale, 'designer.canvas.empty')}</p>
        </div>
      </div>
    )
  }

  return (
    <GraphCanvas
      ref={graphRef}
      width={view.bounds.width}
      height={view.bounds.height}
      nodeWidth={NODE_WIDTH}
      nodeHeight={NODE_HEIGHT}
      nodes={graphNodes}
      edges={graphEdges}
      pinnedNodeIds={pinnedNodeIds}
      isInteractiveChrome={isInteractiveChrome}
      onCommitNodePosition={handleCommitNodePosition}
      onNodeClick={onSelectBlock}
      onNodeFocus={onSelectBlock}
      onNodeDoubleClick={onOpenDrill}
      onViewportClick={handleViewportClick}
      onViewportContextMenu={handleViewportContextMenu}
      wrapperClassName="designer-canvas"
      viewportClassName="designer-canvas-viewport"
      svgClassName="designer-canvas-svg"
      nodesLayerClassName="designer-canvas-nodes"
      nodeShellClassName="designer-node-shell"
      getNodeClassName={getNodeClassName}
      getNodeAriaLabel={getNodeAriaLabel}
      getNodeAriaPressed={getNodeAriaPressed}
      renderNode={(node) => (
        <NodeView
          node={node.data}
          locale={locale}
          selected={selectedBlockId === node.data.block.id || drillBlockId === node.data.block.id}
          onDeleteBlock={onDeleteBlock}
        />
      )}
      renderEdge={(edge, _from, _to, geometry, arrowMarkerUrl) => (
        <EdgeView edge={edge.data} geometry={geometry} arrowMarkerUrl={arrowMarkerUrl} />
      )}
    >
      {(zoomApi) => (
        <>
          <div className="designer-canvas-controls">
            <button
              type="button"
              className="designer-icon-button"
              onClick={zoomApi.zoomIn}
              title={t(locale, 'designer.canvas.zoomIn')}
              aria-label={t(locale, 'designer.canvas.zoomIn')}
            >
              <AppIcon name="plus" aria-hidden="true" />
            </button>
            <button
              type="button"
              className="designer-icon-button"
              onClick={zoomApi.resetZoom}
              title={t(locale, 'designer.canvas.zoomReset')}
              aria-label={t(locale, 'designer.canvas.zoomReset')}
            >
              <span className="designer-canvas-zoom-label">
                {Math.round(zoomApi.zoom * 100)}%
              </span>
            </button>
            <button
              type="button"
              className="designer-icon-button"
              onClick={zoomApi.zoomOut}
              title={t(locale, 'designer.canvas.zoomOut')}
              aria-label={t(locale, 'designer.canvas.zoomOut')}
            >
              <AppIcon name="minus" aria-hidden="true" />
            </button>
          </div>

          {createMenu ? (
            <div
              className="designer-canvas-create-menu"
              role="menu"
              ref={createMenuRef}
              onKeyDown={handleCreateMenuKeyDown}
              style={{ left: createMenu.clientX, top: createMenu.clientY }}
            >
              <div className="designer-canvas-create-menu-title">
                {t(locale, 'designer.canvas.createMenu')}
              </div>
              {CREATE_KINDS.map((kind) => (
                <button
                  key={kind}
                  type="button"
                  role="menuitem"
                  className="designer-canvas-create-menu-item"
                  onClick={() => {
                    onCreateBlock(kind, { x: createMenu.canvasX, y: createMenu.canvasY })
                    setCreateMenu(null)
                  }}
                >
                  {designerBlockKindLabel(locale, kind)}
                </button>
              ))}
            </div>
          ) : null}
        </>
      )}
    </GraphCanvas>
  )
})

interface NodeViewProps {
  node: DesignerGraphNode
  locale: Locale
  selected: boolean
  onDeleteBlock: (block: DesignerBlock) => void
}

/** Inner card content only — `<GraphCanvas>` owns the interactive shell
 * (ref/position/drag/focus/keyboard wiring) around this. */
function NodeView({ node, locale, selected, onDeleteBlock }: NodeViewProps) {
  const stateClassName = [
    selected ? 'is-selected' : '',
    node.hasError ? 'has-error' : node.gapCount > 0 ? 'has-warning' : '',
  ]
    .filter(Boolean)
    .join(' ')
  const nodeClassName = ['designer-node', stateClassName].filter(Boolean).join(' ')
  const kindLabel = designerBlockKindLabel(locale, node.block.kind)
  const nodeTitle = node.block.title || node.block.id

  return (
    <div className={nodeClassName}>
      <div className="designer-node-header">
        <span className="designer-node-kind">{kindLabel}</span>
        <span className="designer-node-title">{nodeTitle}</span>
        {node.block.id !== 'brief' ? (
          <button
            type="button"
            className="designer-node-delete-btn"
            data-no-drag
            onPointerDown={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              onDeleteBlock(node.block)
            }}
            title={t(locale, 'designer.canvas.deleteBlock')}
            aria-label={t(locale, 'designer.canvas.deleteBlock')}
          >
            <AppIcon name="trash" aria-hidden="true" />
          </button>
        ) : null}
        {node.gapCount > 0 ? (
          <span
            className={`designer-node-gap-badge${node.hasError ? ' is-error' : ''}`}
            aria-label={t(locale, 'designer.canvas.gapCount', {
              count: node.gapCount,
              plural: node.gapCount === 1 ? '' : 's',
            })}
          >
            <AppIcon name="alert-triangle" aria-hidden="true" />
            {node.gapCount}
          </span>
        ) : null}
      </div>
      <div className="designer-node-meta">{nodeFingerprint(node.block)}</div>
    </div>
  )
}

/** A short summary of the block's payload, e.g. "3 fields" / "2 endpoints". */
function nodeFingerprint(block: DesignerBlock): string {
  const payload = block.payload as Record<string, unknown> | undefined
  if (!payload || typeof payload !== 'object') return block.id
  switch (block.kind) {
    case 'entityModel': {
      const fields = Array.isArray(payload.fields) ? payload.fields.length : 0
      return `${fields} field${fields === 1 ? '' : 's'}`
    }
    case 'businessFlow': {
      const states = Array.isArray(payload.states) ? payload.states.length : 0
      const transitions = Array.isArray(payload.transitions) ? payload.transitions.length : 0
      return `${states} state · ${transitions} ↦`
    }
    case 'apiContract': {
      const endpoints = Array.isArray(payload.endpoints) ? payload.endpoints.length : 0
      return `${endpoints} endpoint${endpoints === 1 ? '' : 's'}`
    }
    case 'text': {
      const md = typeof payload.markdown === 'string' ? payload.markdown : ''
      return md ? truncate(md, 40) : block.id
    }
    default:
      return block.id
  }
}

function truncate(text: string, max: number): string {
  const trimmed = text.trim().replace(/\s+/g, ' ')
  return trimmed.length <= max ? trimmed : trimmed.slice(0, max - 1) + '…'
}

interface EdgeViewProps {
  edge: DesignerGraphEdge
  geometry: GraphEdgeGeometry
  arrowMarkerUrl: string
}

function EdgeView({ edge, geometry, arrowMarkerUrl }: EdgeViewProps) {
  const className = `designer-canvas-edge designer-canvas-edge--${edge.relation}`
  return (
    <g pointerEvents="none">
      <path d={geometry.path} className={className} markerEnd={arrowMarkerUrl} />
      <text
        className="designer-canvas-edge-label"
        x={geometry.control.x}
        y={geometry.control.y - 4}
        textAnchor="middle"
      >
        {edge.relation}
      </text>
    </g>
  )
}
