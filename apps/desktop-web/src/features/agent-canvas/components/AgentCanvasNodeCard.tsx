import type { PointerEvent as ReactPointerEvent, MouseEvent as ReactMouseEvent, CSSProperties } from 'react'
import { t, type Locale } from '@shell/i18n/ui-locale'
import type { AgentCanvasNodeData } from '../model/agent-canvas-graph'
import { statusLabel } from '../model/agent-canvas-status-label'

export type AgentPortKind = 'input' | 'output'

export interface AgentPortHandlers {
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerMove: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerUp: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerCancel: (event: ReactPointerEvent<HTMLDivElement>) => void
  onContextMenu: (event: ReactMouseEvent<HTMLDivElement>) => void
}

interface AgentCanvasNodeCardProps {
  node: AgentCanvasNodeData
  locale: Locale
  /** Grasshopper-style connection ports (docs/cw/04_客製化設計.md §1, P4.5) —
   * the drag-to-connect gesture, its pointer-capture lifecycle, and the
   * disconnect context menu all live in `AgentCanvasPane`; this component
   * only wires the returned handlers onto the two port elements it owns. */
  getPortHandlers: (portKind: AgentPortKind) => AgentPortHandlers
}

/** Inner card content only — `<GraphCanvas>` owns the interactive shell
 * (ref/position/drag wiring) around this. Ports are siblings of the card,
 * not children, so their `position: absolute` anchors to the shell (the box
 * edge geometry actually connects wires to), not to the card's own padding. */
export function AgentCanvasNodeCard({ node, locale, getPortHandlers }: AgentCanvasNodeCardProps) {
  const { agent, runtimeState } = node
  const title = agent.name || agent.id
  const isSubagent = Boolean(agent.parentAgentId)

  return (
    <>
      <div
        className={`agent-canvas-node${isSubagent ? ' agent-canvas-node--subagent' : ''}`}
        style={agent.color ? ({ '--agent-canvas-node-color': agent.color } as CSSProperties) : undefined}
      >
        <div className="agent-canvas-node-header">
          <span
            className={`agent-canvas-node-status-dot agent-canvas-node-status-dot--${runtimeState}`}
            aria-hidden="true"
          />
          <span className="agent-canvas-node-title">{title}</span>
        </div>
        {!isSubagent && (
          <div className="agent-canvas-node-meta">
            <span className="agent-canvas-node-tool">{agent.tool}</span>
            <span className="agent-canvas-node-status-label">{statusLabel(locale, runtimeState)}</span>
          </div>
        )}
      </div>
      <div
        className="agent-canvas-port agent-canvas-port--input"
        data-no-drag
        data-port="input"
        data-agent-id={agent.id}
        title={t(locale, 'agentCanvas.port.input')}
        {...getPortHandlers('input')}
      />
      <div
        className="agent-canvas-port agent-canvas-port--output"
        data-no-drag
        data-port="output"
        data-agent-id={agent.id}
        title={t(locale, 'agentCanvas.port.output')}
        {...getPortHandlers('output')}
      />
    </>
  )
}
