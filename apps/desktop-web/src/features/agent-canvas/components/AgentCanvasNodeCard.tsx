import type { Locale } from '@shell/i18n/ui-locale'
import type { AgentCanvasNodeData } from '../model/agent-canvas-graph'
import { statusLabel } from '../model/agent-canvas-status-label'

interface AgentCanvasNodeCardProps {
  node: AgentCanvasNodeData
  locale: Locale
}

/** Inner card content only — `<GraphCanvas>` owns the interactive shell
 * (ref/position/drag wiring) around this. */
export function AgentCanvasNodeCard({ node, locale }: AgentCanvasNodeCardProps) {
  const { agent, runtimeState } = node
  const title = agent.name || agent.id

  return (
    <div className="agent-canvas-node">
      <div className="agent-canvas-node-header">
        <span
          className={`agent-canvas-node-status-dot agent-canvas-node-status-dot--${runtimeState}`}
          aria-hidden="true"
        />
        <span className="agent-canvas-node-title">{title}</span>
      </div>
      <div className="agent-canvas-node-meta">
        <span className="agent-canvas-node-tool">{agent.tool}</span>
        <span className="agent-canvas-node-status-label">{statusLabel(locale, runtimeState)}</span>
      </div>
    </div>
  )
}
