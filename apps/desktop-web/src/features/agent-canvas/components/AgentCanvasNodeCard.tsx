import type { PointerEvent as ReactPointerEvent, MouseEvent as ReactMouseEvent, CSSProperties } from 'react'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'
import { resolveAgentModelDisplayLabel } from '@features/workspace-hub/agent-management-model'
import { computePortSlotCenterOffset, type AgentCanvasNodeData } from '../model/agent-canvas-graph'
import { statusLabel } from '../model/agent-canvas-status-label'

export type AgentPortKind = 'input' | 'output'

export interface AgentPortHandlers {
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerMove: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerUp: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerCancel: (event: ReactPointerEvent<HTMLDivElement>) => void
}

interface AgentCanvasNodeCardProps {
  node: AgentCanvasNodeData
  locale: Locale
  /** Grasshopper-style connection ports (docs/cw/04_客製化設計.md §1, P4.5) —
   * the drag-to-connect gesture, its pointer-capture lifecycle, and the
   * disconnect context menu all live in `AgentCanvasPane`; this component
   * only wires the returned handlers onto the port elements. The "+" (add
   * connection) always gets drag handlers; an already-connected dot only
   * does when passed its own `linkId` — `AgentCanvasPane` gates that drag
   * behind Ctrl+Shift (a "rewire" gesture) so a plain pointerdown on a
   * connected dot still falls through to its right-click-only behavior
   * (see `onPortSlotContextMenu`). */
  getPortHandlers: (portKind: AgentPortKind, rewireLinkId?: string) => AgentPortHandlers
  /** Right-clicking an existing connected dot (one per already-drawn
   * authored link, docs/cw/04_客製化設計.md §1, P4.6) — the caller resolves
   * `linkId` back to the full `AgentLink` and opens the same wire context
   * menu a click on the wire itself would. */
  onPortSlotContextMenu: (portKind: AgentPortKind, linkId: string, event: ReactMouseEvent<HTMLDivElement>) => void
  /** Fired by the gear button that appears on hover, top-right of the node —
   * the caller owns opening the actual edit-agent UI (agent-canvas only
   * requests it), mirroring `onRequestCreateSubagent`'s split. */
  onRequestEdit?: (agentId: string) => void
}

/** One side's stack of port dots: one per already-connected authored link,
 * plus a trailing "+" to start a new one. `total` is shared across every
 * dot AND the "+" so they're evenly spaced and centered as one group. */
function PortSlots({
  portKind,
  linkIds,
  agentId,
  locale,
  getPortHandlers,
  onPortSlotContextMenu,
}: {
  portKind: AgentPortKind
  linkIds: string[]
  agentId: string
  locale: Locale
  getPortHandlers: (portKind: AgentPortKind, rewireLinkId?: string) => AgentPortHandlers
  onPortSlotContextMenu: (portKind: AgentPortKind, linkId: string, event: ReactMouseEvent<HTMLDivElement>) => void
}) {
  const total = linkIds.length + 1
  const addHandlers = getPortHandlers(portKind)
  return (
    <>
      {linkIds.map((linkId, index) => (
        <div
          key={linkId}
          className={`agent-canvas-port agent-canvas-port--${portKind}`}
          style={{ transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(index, total)}px))` }}
          data-no-drag
          data-port={portKind}
          data-agent-id={agentId}
          title={t(locale, portKind === 'input' ? 'agentCanvas.port.input' : 'agentCanvas.port.output')}
          onContextMenu={(event) => onPortSlotContextMenu(portKind, linkId, event)}
          {...getPortHandlers(portKind, linkId)}
        />
      ))}
      <div
        className={`agent-canvas-port agent-canvas-port--${portKind} agent-canvas-port--add`}
        style={{
          transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(linkIds.length, total)}px))`,
        }}
        data-no-drag
        data-port={portKind}
        data-agent-id={agentId}
        title={t(locale, portKind === 'input' ? 'agentCanvas.port.addInput' : 'agentCanvas.port.addOutput')}
        {...addHandlers}
      >
        <AppIcon name="plus" aria-hidden="true" />
      </div>
    </>
  )
}

/** Inner card content only — `<GraphCanvas>` owns the interactive shell
 * (ref/position/drag wiring) around this. Ports are siblings of the card,
 * not children, so their `position: absolute` anchors to the shell (the box
 * edge geometry actually connects wires to), not to the card's own padding. */
export function AgentCanvasNodeCard({
  node,
  locale,
  getPortHandlers,
  onPortSlotContextMenu,
  onRequestEdit,
}: AgentCanvasNodeCardProps) {
  const { agent, runtimeState, outputLinkIds, inputLinkIds } = node
  const title = agent.name || agent.id
  const isSubagent = Boolean(agent.parentAgentId)
  const modelLabel = resolveAgentModelDisplayLabel(agent.tool, agent.launchCommand)

  return (
    <>
      <div
        className={`agent-canvas-node${isSubagent ? ' agent-canvas-node--subagent' : ''}`}
        style={agent.color ? ({ '--agent-canvas-node-color': agent.color } as CSSProperties) : undefined}
      >
        {onRequestEdit && (
          <button
            type="button"
            className="agent-canvas-node-edit-btn"
            data-no-drag
            title={t(locale, '编辑 Agent', 'Edit agent')}
            aria-label={t(locale, '编辑 Agent', 'Edit agent')}
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              onRequestEdit(agent.id)
            }}
            onContextMenu={(event) => event.stopPropagation()}
          >
            <AppIcon name="settings" aria-hidden="true" />
          </button>
        )}
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
            {modelLabel && <span className="agent-canvas-node-model">{modelLabel}</span>}
            <span className="agent-canvas-node-status-label">{statusLabel(locale, runtimeState)}</span>
          </div>
        )}
      </div>
      <PortSlots
        portKind="input"
        linkIds={inputLinkIds}
        agentId={agent.id}
        locale={locale}
        getPortHandlers={getPortHandlers}
        onPortSlotContextMenu={onPortSlotContextMenu}
      />
      <PortSlots
        portKind="output"
        linkIds={outputLinkIds}
        agentId={agent.id}
        locale={locale}
        getPortHandlers={getPortHandlers}
        onPortSlotContextMenu={onPortSlotContextMenu}
      />
    </>
  )
}
