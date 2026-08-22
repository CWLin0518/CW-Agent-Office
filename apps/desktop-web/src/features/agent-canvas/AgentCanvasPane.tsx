import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'
import { GraphCanvas } from '@/components/graph-canvas'
import { AgentCanvasNodeCard } from './components/AgentCanvasNodeCard'
import { useAgentCanvasData } from './controllers/useAgentCanvasData'
import { AGENT_NODE_HEIGHT, AGENT_NODE_WIDTH } from './model/agent-canvas-graph'
import { statusLabel } from './model/agent-canvas-status-label'
import './AgentCanvasPane.scss'

interface AgentCanvasPaneProps {
  locale: Locale
  workspaceId: string | null
  /** Whether this pane is the one currently shown — polling pauses while
   * inactive, mirroring `BusinessDesignerPane`'s `active` prop. */
  active: boolean
}

function isChromeElement(target: HTMLElement): boolean {
  return Boolean(target.closest('.agent-canvas-node-shell, .agent-canvas-controls'))
}

export function AgentCanvasPane({ locale, workspaceId, active }: AgentCanvasPaneProps) {
  const { graph, isEmpty, commitNodePosition } = useAgentCanvasData(workspaceId, active)

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

  return (
    <GraphCanvas
      width={graph.bounds.width}
      height={graph.bounds.height}
      nodeWidth={AGENT_NODE_WIDTH}
      nodeHeight={AGENT_NODE_HEIGHT}
      nodes={graph.nodes}
      edges={graph.edges}
      isInteractiveChrome={isChromeElement}
      onCommitNodePosition={commitNodePosition}
      wrapperClassName="agent-canvas"
      viewportClassName="agent-canvas-viewport"
      svgClassName="agent-canvas-svg"
      nodesLayerClassName="agent-canvas-nodes"
      nodeShellClassName="agent-canvas-node-shell"
      getNodeClassName={(node) => `agent-canvas-node-shell--${node.data.runtimeState}`}
      getNodeAriaLabel={(node) =>
        t(locale, 'agentCanvas.nodeLabel', {
          name: node.data.agent.name || node.data.agent.id,
          status: statusLabel(locale, node.data.runtimeState),
        })
      }
      renderNode={(node) => <AgentCanvasNodeCard node={node.data} locale={locale} />}
      renderEdge={(edge, _from, _to, geometry, arrowMarkerUrl) => (
        <g pointerEvents="none">
          <title>
            {edge.data.link.kind === 'authored'
              ? t(locale, 'agentCanvas.edge.authoredTitle')
              : t(locale, 'agentCanvas.edge.derivedTitle')}
          </title>
          <path
            d={geometry.path}
            className={`agent-canvas-edge agent-canvas-edge--${edge.data.link.kind}`}
            markerEnd={arrowMarkerUrl}
          />
        </g>
      )}
    >
      {(zoomApi) => (
        <div className="agent-canvas-controls">
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
      )}
    </GraphCanvas>
  )
}
