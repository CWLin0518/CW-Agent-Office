import { useState, type PointerEvent as ReactPointerEvent, type MouseEvent as ReactMouseEvent, type CSSProperties } from 'react'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon, type AppIconName } from '@shell/ui/icons'
import { resolveAgentModelDisplayLabel } from '@features/workspace-hub/agent-management-model'
import {
  computeAgentInputPortLayout,
  computeAgentOutputPortLayout,
  computePortSlotCenterOffset,
  type AgentCanvasAgentNodeData,
  type AgentCanvasHookNodeData,
  type AgentCanvasMcpNodeData,
  type AgentCanvasOutputNodeData,
  type AgentCanvasSkillNodeData,
} from '../model/agent-canvas-graph'
import { statusLabel } from '../model/agent-canvas-status-label'

export type AgentPortKind = 'input' | 'output'

/** Shared by every mount node card (MCP/Skill/Hook) below — a mount's
 * dashed-outline color only ever needs one CSS custom property set, and
 * only when it's non-default (see `AgentCanvasPane.scss`'s
 * `--agent-canvas-mount-color` fallback-to-gray convention). */
function mountColorStyle(color: string | null | undefined): CSSProperties | undefined {
  return color ? ({ '--agent-canvas-mount-color': color } as CSSProperties) : undefined
}

export interface AgentPortHandlers {
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerMove: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerUp: (event: ReactPointerEvent<HTMLDivElement>) => void
  onPointerCancel: (event: ReactPointerEvent<HTMLDivElement>) => void
}

interface AgentCanvasNodeCardProps {
  node: AgentCanvasAgentNodeData
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
 * dot AND the "+" so they're evenly spaced and centered as one group.
 * `mountId` (output side only, docs/cw/14_Agent輸出清單化.md §4.2) fans a
 * SECOND, non-interactive dot into the same stack after the link ids — same
 * "mount dot gets its own modifier class" shape `InputPortSlots` below uses
 * for MCP mounts, just with at most one item instead of an array. */
function PortSlots({
  portKind,
  linkIds,
  mountId,
  agentId,
  locale,
  getPortHandlers,
  onPortSlotContextMenu,
}: {
  portKind: AgentPortKind
  linkIds: string[]
  mountId?: string | null
  agentId: string
  locale: Locale
  getPortHandlers: (portKind: AgentPortKind, rewireLinkId?: string) => AgentPortHandlers
  onPortSlotContextMenu: (portKind: AgentPortKind, linkId: string, event: ReactMouseEvent<HTMLDivElement>) => void
}) {
  const hasMount = mountId != null
  // Shares its arithmetic with `AgentCanvasPane`'s `renderEdge` (both read
  // off `computeAgentOutputPortLayout`) so the mount dot's slot here can
  // never drift from where the `output-mount` wire anchors to it — same
  // "single source of truth" reasoning `computeAgentInputPortLayout` and
  // `InputPortSlots` below already follow for the input side.
  const { total } = computeAgentOutputPortLayout({
    outputLinkIds: linkIds,
    outputMountId: mountId ?? null,
  })
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
      {hasMount && (
        <div
          className={`agent-canvas-port agent-canvas-port--${portKind} agent-canvas-port--output-mount`}
          style={{ transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(linkIds.length, total)}px))` }}
          title={t(locale, '輸出檔案', 'Output files')}
          aria-hidden="true"
        />
      )}
      <div
        className={`agent-canvas-port agent-canvas-port--${portKind} agent-canvas-port--add`}
        style={{
          transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(linkIds.length + (hasMount ? 1 : 0), total)}px))`,
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

/** Input side only — same shape as `PortSlots` above, but fans a SECOND,
 * non-interactive list of MCP-mount ids into the same stack after the
 * authored-link ids (link items keep their original index within
 * `linkIds`, so `computePortEdgeGeometry`'s `toIndex = inputLinkIds.indexOf(link.id)`
 * elsewhere still lines up — only the mount dots and the trailing "+" shift
 * to make room). A mount dot gets its own modifier class so it reads as "a
 * mounted tool," not another agent's wire — see `agentCanvas-graph.ts`'s
 * `AgentCanvasAgentNodeData.mcpMountIds` doc comment for why they share
 * this side at all. */
function InputPortSlots({
  linkIds,
  mcpMountIds,
  skillMountId,
  hookMountId,
  agentId,
  locale,
  getPortHandlers,
  onPortSlotContextMenu,
}: {
  linkIds: string[]
  mcpMountIds: string[]
  skillMountId: string | null
  hookMountId: string | null
  agentId: string
  locale: Locale
  getPortHandlers: (portKind: AgentPortKind, rewireLinkId?: string) => AgentPortHandlers
  onPortSlotContextMenu: (portKind: AgentPortKind, linkId: string, event: ReactMouseEvent<HTMLDivElement>) => void
}) {
  const { total, skillIndex, hookIndex } = computeAgentInputPortLayout({
    inputLinkIds: linkIds,
    mcpMountIds,
    skillMountId,
    hookMountId,
  })
  const addHandlers = getPortHandlers('input')
  return (
    <>
      {linkIds.map((linkId, index) => (
        <div
          key={linkId}
          className="agent-canvas-port agent-canvas-port--input"
          style={{ transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(index, total)}px))` }}
          data-no-drag
          data-port="input"
          data-agent-id={agentId}
          title={t(locale, 'agentCanvas.port.input')}
          onContextMenu={(event) => onPortSlotContextMenu('input', linkId, event)}
          {...getPortHandlers('input', linkId)}
        />
      ))}
      {mcpMountIds.map((mountId, index) => (
        <div
          key={mountId}
          className="agent-canvas-port agent-canvas-port--input agent-canvas-port--mcp"
          style={{
            transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(linkIds.length + index, total)}px))`,
          }}
          title={t(locale, '掛載的 MCP', 'Mounted MCP')}
          aria-hidden="true"
        />
      ))}
      {skillMountId !== null && skillIndex !== null && (
        <div
          className="agent-canvas-port agent-canvas-port--input agent-canvas-port--skill"
          style={{ transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(skillIndex, total)}px))` }}
          title={t(locale, '掛載的 Skill', 'Mounted Skills')}
          aria-hidden="true"
        />
      )}
      {hookMountId !== null && hookIndex !== null && (
        <div
          className="agent-canvas-port agent-canvas-port--input agent-canvas-port--hook"
          style={{ transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(hookIndex, total)}px))` }}
          title={t(locale, '掛載的 Hook', 'Mounted Hooks')}
          aria-hidden="true"
        />
      )}
      <div
        className="agent-canvas-port agent-canvas-port--input agent-canvas-port--add"
        style={{
          transform: `translateY(calc(-50% + ${computePortSlotCenterOffset(total - 1, total)}px))`,
        }}
        data-no-drag
        data-port="input"
        data-agent-id={agentId}
        title={t(locale, 'agentCanvas.port.addInput')}
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
  const {
    agent,
    runtimeState,
    outputLinkIds,
    inputLinkIds,
    childAgentIds,
    mcpMountIds,
    skillMountId,
    hookMountId,
    outputMountId,
  } = node
  const title = agent.name || agent.id
  const isSubagent = Boolean(agent.parentAgentId)
  const modelLabel = resolveAgentModelDisplayLabel(agent.tool, agent.launchCommand)
  const communicatesWithAll = Boolean(agent.communicateWithAll)

  return (
    <>
      <div
        className={`agent-canvas-node${isSubagent ? ' agent-canvas-node--subagent' : ''}${
          communicatesWithAll ? ' agent-canvas-node--broadcast' : ''
        }`}
        title={communicatesWithAll ? t(locale, 'agentCanvas.node.communicateWithAll') : undefined}
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
          {communicatesWithAll && (
            <span className="agent-canvas-node-broadcast-badge">{t(locale, 'agentCanvas.node.allBadge')}</span>
          )}
        </div>
        {!isSubagent && (
          <div className="agent-canvas-node-meta">
            <span className="agent-canvas-node-tool">{agent.tool}</span>
            {modelLabel && <span className="agent-canvas-node-model">{modelLabel}</span>}
            <span className="agent-canvas-node-status-label">{statusLabel(locale, runtimeState)}</span>
          </div>
        )}
      </div>
      <InputPortSlots
        linkIds={inputLinkIds}
        mcpMountIds={mcpMountIds}
        skillMountId={skillMountId}
        hookMountId={hookMountId}
        agentId={agent.id}
        locale={locale}
        getPortHandlers={getPortHandlers}
        onPortSlotContextMenu={onPortSlotContextMenu}
      />
      <PortSlots
        portKind="output"
        linkIds={outputLinkIds}
        mountId={outputMountId}
        agentId={agent.id}
        locale={locale}
        getPortHandlers={getPortHandlers}
        onPortSlotContextMenu={onPortSlotContextMenu}
      />
      {childAgentIds.map((childAgentId, index) => (
        <div
          key={childAgentId}
          className="agent-canvas-port agent-canvas-port--bottom"
          style={{
            transform: `translateX(calc(-50% + ${computePortSlotCenterOffset(index, childAgentIds.length)}px))`,
          }}
          aria-hidden="true"
        />
      ))}
      {isSubagent && <div className="agent-canvas-port agent-canvas-port--top" aria-hidden="true" />}
    </>
  )
}

interface AgentCanvasMcpNodeCardProps {
  node: AgentCanvasMcpNodeData
  locale: Locale
  /** Flips `McpServerCapability.enabled` for this one server within its
   * agent's capability snapshot (`useAgentCanvasData`'s
   * `setMcpServerEnabled`) — never removes the mount, just toggles whether
   * the next materialize actually includes it. */
  onToggleEnabled: (agentId: string, serverId: string, enabled: boolean) => void
  /** Right-click — the caller (`AgentCanvasPane`) owns opening the color
   * context menu, mirroring `AgentCanvasNodeCard`'s own
   * `onPortSlotContextMenu` split. */
  onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void
}

/** MCP-mount node — connects from its own right side into the owning
 * agent's left-side port stack (see `InputPortSlots` above); the anchor
 * dot here is a single, non-interactive point since a mount node only ever
 * has the one outgoing edge. */
export function AgentCanvasMcpNodeCard({ node, locale, onToggleEnabled, onContextMenu }: AgentCanvasMcpNodeCardProps) {
  const { agentId, server } = node
  const title = server.name?.trim() || server.id
  return (
    <>
      <div
        className={`agent-canvas-mcp-node${server.enabled ? '' : ' agent-canvas-mcp-node--disabled'}`}
        style={mountColorStyle(node.color)}
        onContextMenu={onContextMenu}
      >
        <span className="agent-canvas-mcp-node-title">{title}</span>
        <button
          type="button"
          role="switch"
          aria-checked={server.enabled}
          className={`agent-canvas-mcp-node-toggle${server.enabled ? ' active' : ''}`}
          data-no-drag
          title={t(
            locale,
            server.enabled ? '停用此 MCP' : '啟用此 MCP',
            server.enabled ? 'Disable this MCP' : 'Enable this MCP',
          )}
          aria-label={t(
            locale,
            server.enabled ? '停用此 MCP' : '啟用此 MCP',
            server.enabled ? 'Disable this MCP' : 'Enable this MCP',
          )}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation()
            onToggleEnabled(agentId, server.id, !server.enabled)
          }}
        >
          <span className="agent-canvas-mcp-node-toggle-thumb" />
        </button>
      </div>
      <div className="agent-canvas-port agent-canvas-port--mcp-anchor" aria-hidden="true" />
    </>
  )
}

/** Shared collapsed header + expand toggle for `AgentCanvasSkillNodeCard`/
 * `AgentCanvasHookNodeCard` below — both are read-only dropdown summaries
 * (one node per agent, not per item, unlike the MCP node above), so the only
 * interactive control either has is "expand/collapse," not a mutation.
 * `enabledCount`/`totalCount` (rather than one plain `count`) so the badge
 * itself communicates how many of the mounted items are actually enabled —
 * the collapsed header is otherwise the only place that fact is visible
 * without expanding the dropdown. Renders as just the number when every
 * mounted item is enabled (the common case, and always true for Hook, which
 * has no per-item enabled flag — see `AgentCanvasHookNodeCard`), or
 * `enabled/total` the moment at least one is disabled. */
function CapabilityDropdownHeader({
  iconName,
  title,
  enabledCount,
  totalCount,
  expanded,
  onToggle,
  locale,
  expandLabelZh,
  expandLabelEn,
  collapseLabelZh,
  collapseLabelEn,
}: {
  iconName: AppIconName
  title: string
  enabledCount: number
  totalCount: number
  expanded: boolean
  onToggle: () => void
  locale: Locale
  expandLabelZh: string
  expandLabelEn: string
  collapseLabelZh: string
  collapseLabelEn: string
}) {
  const toggleLabel = expanded ? t(locale, collapseLabelZh, collapseLabelEn) : t(locale, expandLabelZh, expandLabelEn)
  const countLabel = enabledCount === totalCount ? `${totalCount}` : `${enabledCount}/${totalCount}`
  const countTitle = t(
    locale,
    `已啟用 ${enabledCount} 個（共掛載 ${totalCount} 個）`,
    `${enabledCount} enabled (of ${totalCount} mounted)`,
  )
  return (
    <>
      <AppIcon name={iconName} className="agent-canvas-capability-node-icon" aria-hidden="true" />
      <span className="agent-canvas-capability-node-title">{title}</span>
      <span className="agent-canvas-capability-node-count" title={countTitle}>
        {countLabel}
      </span>
      <button
        type="button"
        className="agent-canvas-capability-node-toggle"
        data-no-drag
        aria-expanded={expanded}
        title={toggleLabel}
        aria-label={toggleLabel}
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation()
          onToggle()
        }}
      >
        <AppIcon name={expanded ? 'chevron-up' : 'chevron-down'} aria-hidden="true" />
      </button>
    </>
  )
}

interface AgentCanvasSkillNodeCardProps {
  node: AgentCanvasSkillNodeData
  locale: Locale
  /** Same split as `AgentCanvasMcpNodeCardProps.onContextMenu`. */
  onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void
}

/** Skill-mount node — one per agent, summarizing every mounted Skill as a
 * collapsed header that expands in place into a read-only list (id +
 * disabled tag, `sourcePath` as the tooltip). Enabling/disabling an
 * individual skill stays a Capabilities-tab action; this node only shows
 * what's mounted. Connects into the owning agent's left-side port stack,
 * same as an MCP node — see `InputPortSlots` above. */
export function AgentCanvasSkillNodeCard({ node, locale, onContextMenu }: AgentCanvasSkillNodeCardProps) {
  const [expanded, setExpanded] = useState(false)
  const { skills } = node
  const enabledCount = skills.filter((skill) => skill.enabled).length
  return (
    <>
      <div
        className="agent-canvas-capability-node agent-canvas-capability-node--skill"
        style={mountColorStyle(node.color)}
        onContextMenu={onContextMenu}
      >
        <CapabilityDropdownHeader
          iconName="sparkles"
          title="Skills"
          enabledCount={enabledCount}
          totalCount={skills.length}
          expanded={expanded}
          onToggle={() => setExpanded((previous) => !previous)}
          locale={locale}
          expandLabelZh="展開已掛載的 Skill 清單"
          expandLabelEn="Expand mounted skills"
          collapseLabelZh="收合 Skill 清單"
          collapseLabelEn="Collapse skills"
        />
        {expanded && (
          <div
            className="agent-canvas-capability-node-dropdown"
            data-no-drag
            onPointerDown={(event) => event.stopPropagation()}
          >
            {skills.map((skill) => (
              <div
                key={skill.id}
                className={`agent-canvas-capability-node-dropdown-item${
                  skill.enabled ? '' : ' agent-canvas-capability-node-dropdown-item--disabled'
                }`}
                title={skill.sourcePath}
              >
                <span className="agent-canvas-capability-node-dropdown-item-name">{skill.id}</span>
                {!skill.enabled && (
                  <span className="agent-canvas-capability-node-dropdown-item-tag">
                    {t(locale, '已停用', 'Disabled')}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="agent-canvas-port agent-canvas-port--mcp-anchor" aria-hidden="true" />
    </>
  )
}

interface AgentCanvasHookNodeCardProps {
  node: AgentCanvasHookNodeData
  locale: Locale
  /** Same split as `AgentCanvasMcpNodeCardProps.onContextMenu`. */
  onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void
}

/** Hook sibling of `AgentCanvasSkillNodeCard` above — each row shows the
 * event (+ matcher, when set) as its label. The visible secondary line
 * prefers the hook's `note` (what it's for, in plain language) when one was
 * filled in, falling back to the raw `command` when it wasn't — a shell
 * command alone doesn't tell a reader what the hook does at a glance, which
 * is exactly the note field's purpose. The tooltip always includes the
 * command (plus the note above it when present) since the command is still
 * the part worth double-checking before trusting a mounted hook. */
export function AgentCanvasHookNodeCard({ node, locale, onContextMenu }: AgentCanvasHookNodeCardProps) {
  const [expanded, setExpanded] = useState(false)
  const { hooks } = node
  return (
    <>
      <div
        className="agent-canvas-capability-node agent-canvas-capability-node--hook"
        style={mountColorStyle(node.color)}
        onContextMenu={onContextMenu}
      >
        <CapabilityDropdownHeader
          iconName="hooks"
          title="Hooks"
          // `HookCapability` has no per-item `enabled` flag (unlike
          // `SkillCapability`) — every mounted hook runs, so enabled ==
          // total here, same "just the number" collapsed form the header
          // renders whenever nothing is disabled.
          enabledCount={hooks.length}
          totalCount={hooks.length}
          expanded={expanded}
          onToggle={() => setExpanded((previous) => !previous)}
          locale={locale}
          expandLabelZh="展開已掛載的 Hook 清單"
          expandLabelEn="Expand mounted hooks"
          collapseLabelZh="收合 Hook 清單"
          collapseLabelEn="Collapse hooks"
        />
        {expanded && (
          <div
            className="agent-canvas-capability-node-dropdown"
            data-no-drag
            onPointerDown={(event) => event.stopPropagation()}
          >
            {hooks.map((hook, index) => {
              const note = hook.note?.trim()
              const tooltip = note ? `${note}\n\n${locale === 'zh-CN' ? '指令：' : 'Command: '}${hook.command}` : hook.command
              return (
                <div
                  key={`${hook.event}:${hook.matcher ?? ''}:${index}`}
                  className="agent-canvas-capability-node-dropdown-item"
                  title={tooltip}
                >
                  <span className="agent-canvas-capability-node-dropdown-item-name">
                    {hook.event}
                    {hook.matcher ? ` · ${hook.matcher}` : ''}
                  </span>
                  <span className="agent-canvas-capability-node-dropdown-item-command">
                    {note || hook.command}
                  </span>
                </div>
              )
            })}
          </div>
        )}
      </div>
      <div className="agent-canvas-port agent-canvas-port--mcp-anchor" aria-hidden="true" />
    </>
  )
}

interface AgentCanvasOutputNodeCardProps {
  node: AgentCanvasOutputNodeData
  locale: Locale
  /** Same split as `AgentCanvasMcpNodeCardProps.onContextMenu`. */
  onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void
  /** Fired when a row is clicked (docs/cw/14_Agent輸出清單化.md §4.4) — the
   * caller decides what that means (markdown: open an in-app preview;
   * webpage/other: hand off to the system default program), this component
   * only reports which file was picked. */
  onFileClick: (file: AgentCanvasOutputNodeData['files'][number]) => void
}

/** Output-list sibling of `AgentCanvasSkillNodeCard`/`AgentCanvasHookNodeCard`
 * above — same collapsed-header-expands-to-list shape, but anchored to the
 * agent's OUTPUT (right) side (see `InputPortSlots` vs. the output `PortSlots`
 * call in `AgentCanvasNodeCard`) and, unlike those two read-only summaries,
 * each row is clickable (`onFileClick`). */
export function AgentCanvasOutputNodeCard({ node, locale, onContextMenu, onFileClick }: AgentCanvasOutputNodeCardProps) {
  const [expanded, setExpanded] = useState(false)
  const { files } = node
  return (
    <>
      <div
        className="agent-canvas-capability-node agent-canvas-capability-node--output"
        style={mountColorStyle(node.color)}
        onContextMenu={onContextMenu}
      >
        <CapabilityDropdownHeader
          iconName="file-text"
          title={t(locale, '輸出', 'Output')}
          // Every scanned output file is "enabled" (there's no per-item
          // on/off concept for a produced file, unlike a mounted Skill) —
          // same "just the number" collapsed form Hooks uses.
          enabledCount={files.length}
          totalCount={files.length}
          expanded={expanded}
          onToggle={() => setExpanded((previous) => !previous)}
          locale={locale}
          expandLabelZh="展開輸出檔案清單"
          expandLabelEn="Expand output files"
          collapseLabelZh="收合輸出檔案清單"
          collapseLabelEn="Collapse output files"
        />
        {expanded && (
          <div
            className="agent-canvas-capability-node-dropdown"
            data-no-drag
            onPointerDown={(event) => event.stopPropagation()}
          >
            {files.map((file) => (
              <button
                type="button"
                key={file.id}
                className="agent-canvas-capability-node-dropdown-item agent-canvas-capability-node-dropdown-item--clickable"
                title={file.absolutePath}
                onClick={() => onFileClick(file)}
              >
                <AppIcon
                  name={file.kind === 'webpage' ? 'external' : 'file-text'}
                  className="agent-canvas-capability-node-dropdown-item-icon"
                  aria-hidden="true"
                />
                <span className="agent-canvas-capability-node-dropdown-item-name">{file.fileName}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      {/* Anchors at the node's LEFT edge, not `.agent-canvas-port--mcp-anchor`'s
          right (docs/cw/14_Agent輸出清單化.md §4.2) — the `output-mount` edge
          is reversed (`from` = agent, `to` = this node), so the wire enters
          HERE at `to.x` (this node's left edge), not its right. */}
      <div className="agent-canvas-port agent-canvas-port--output-anchor" aria-hidden="true" />
    </>
  )
}
