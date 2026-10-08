import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from 'react'

import { desktopApi, createDefaultAgentCapability } from '@shell/integration/desktop-api'
import type {
  AgentCapabilityHookPreviewItem,
  AgentCapabilitySnapshot,
  DiscoveredHook,
  DiscoveredSkill,
  HookCapability,
  McpServerCapability,
  McpTransport,
  SkillCapability,
} from '@shell/integration/desktop-api'
import type { Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'

import {
  buildSavableCapabilitySnapshot,
  createEmptyHook,
  createEmptyMcpServer,
  filterDiscoveredHooks,
  filterDiscoveredSkills,
  filterHookEntries,
  filterSkillEntries,
  findMountedHookEntry,
  findMountedSkillEntry,
  formatArgsTextarea,
  formatEnvTextarea,
  isMcpServerDraftValid,
  isHookDraftValid,
  isSkillsOrHooksSupportedForToolKind,
  parseArgsTextarea,
  parseEnvTextarea,
  requiresHookPreviewBeforeSave,
  setAllHooksEnabled,
  setAllMcpServersEnabled,
  setAllSkillsEnabled,
  toggleDiscoveredHook,
  toggleDiscoveredSkill,
  unconfirmedHookHashes,
  unmatchedHookEntries,
  unmatchedSkillEntries,
  updateMountedHookNote,
  type CapabilitySubTab,
} from './station-capabilities-model'

/** Single-user desktop app — matches the literal value already used for
 * `gt-ai-config`'s equivalent apply flow
 * (`useProviderWorkspaceController.ts`'s `aiConfigApplyPatch(..., 'System Admin')`). */
const CAPABILITY_CONFIRMED_BY = 'System Admin'

const HOOK_EVENT_OPTIONS = [
  'PreToolUse',
  'PostToolUse',
  'UserPromptSubmit',
  'Stop',
  'SubagentStop',
  'SessionStart',
  'SessionEnd',
  'PreCompact',
  'Notification',
]

export interface StationCapabilitiesTabHandle {
  requestSave: () => Promise<boolean>
}

interface StationCapabilitiesTabProps {
  locale: Locale
  workspaceId: string
  agentId: string
  toolKind: string
  active: boolean
  onSavingChange?: (saving: boolean) => void
  /** Fired after a successful save (either path: direct, or via the hook
   * preview/confirm flow) — lets a caller invalidate data derived from this
   * agent's capability snapshot elsewhere (e.g. agent-canvas's MCP/Skill/Hook
   * mount nodes, which otherwise only pick up the change on their own next
   * poll tick). */
  onSaved?: () => void
}

export const StationCapabilitiesTab = forwardRef<StationCapabilitiesTabHandle, StationCapabilitiesTabProps>(
  function StationCapabilitiesTab({ locale, workspaceId, agentId, toolKind, active, onSavingChange, onSaved }, ref) {
    const [subTab, setSubTab] = useState<CapabilitySubTab>('mcp')
    const [loading, setLoading] = useState(false)
    const [loadError, setLoadError] = useState<string | null>(null)
    const [draft, setDraft] = useState<AgentCapabilitySnapshot>(createDefaultAgentCapability())
    const [saving, setSaving] = useState(false)
    const [saveError, setSaveError] = useState<string | null>(null)
    const [preview, setPreview] = useState<{
      items: AgentCapabilityHookPreviewItem[]
      confirming: boolean
      error: string | null
    } | null>(null)
    const confirmButtonRef = useRef<HTMLButtonElement | null>(null)
    const loadedForKey = useRef<string | null>(null)

    const skillsHooksSupported = isSkillsOrHooksSupportedForToolKind(toolKind)

    useEffect(() => {
      if (!active) {
        return
      }
      const key = `${workspaceId}::${agentId}`
      if (loadedForKey.current === key) {
        return
      }
      loadedForKey.current = key
      setLoading(true)
      setLoadError(null)
      desktopApi
        .agentCapabilityRead({ workspaceId, agentId })
        .then((response) => {
          setDraft(response.capability)
        })
        .catch((error) => {
          setLoadError(error instanceof Error ? error.message : String(error))
        })
        .finally(() => {
          setLoading(false)
        })
    }, [active, workspaceId, agentId])

    useEffect(() => {
      onSavingChange?.(saving)
    }, [saving, onSavingChange])

    useEffect(() => {
      if (preview) {
        confirmButtonRef.current?.focus()
      }
    }, [preview])

    const runSave = useCallback(
      async (capability: AgentCapabilitySnapshot) => {
        setSaving(true)
        setSaveError(null)
        try {
          await desktopApi.agentCapabilitySave({
            workspaceId,
            agentId,
            capability,
            confirmedBy: CAPABILITY_CONFIRMED_BY,
          })
          onSaved?.()
          return true
        } catch (error) {
          setSaveError(error instanceof Error ? error.message : String(error))
          return false
        } finally {
          setSaving(false)
        }
      },
      [workspaceId, agentId, onSaved],
    )

    const requestSave = useCallback(async (): Promise<boolean> => {
      // Set synchronously (before the first `await`) so the footer's Save
      // button is disabled for the *entire* requestSave lifecycle, not just
      // once `runSave` starts — otherwise a second click during the
      // `agentCapabilityPreviewHooks` round trip races a duplicate preview
      // request in (both harmless, since preview is read-only, but still a
      // real double-submission window worth closing).
      setSaving(true)
      const savable = buildSavableCapabilitySnapshot(draft)
      if (!requiresHookPreviewBeforeSave(savable.hooks)) {
        return runSave(savable)
      }
      setSaveError(null)
      try {
        const response = await desktopApi.agentCapabilityPreviewHooks({
          workspaceId,
          agentId,
          hooks: savable.hooks,
        })
        setPreview({ items: response.items, confirming: false, error: null })
        return false
      } catch (error) {
        setSaveError(error instanceof Error ? error.message : String(error))
        return false
      } finally {
        setSaving(false)
      }
    }, [draft, workspaceId, agentId, runSave])

    useImperativeHandle(ref, () => ({ requestSave }), [requestSave])

    const handleConfirmAndSave = useCallback(async () => {
      if (!preview) {
        return
      }
      setPreview((previous) => (previous ? { ...previous, confirming: true, error: null } : previous))
      const toConfirm = unconfirmedHookHashes(preview.items)
      try {
        if (toConfirm.length > 0) {
          await desktopApi.agentCapabilityConfirmHooks({
            workspaceId,
            agentId,
            hookHashes: toConfirm,
            confirmedBy: CAPABILITY_CONFIRMED_BY,
          })
        }
        const savable = buildSavableCapabilitySnapshot(draft)
        const saved = await runSave(savable)
        if (saved) {
          setPreview(null)
        } else {
          setPreview((previous) => (previous ? { ...previous, confirming: false } : previous))
        }
      } catch (error) {
        setPreview((previous) =>
          previous
            ? {
                ...previous,
                confirming: false,
                error: error instanceof Error ? error.message : String(error),
              }
            : previous,
        )
      }
    }, [preview, draft, workspaceId, agentId, runSave])

    if (!active) {
      return null
    }

    return (
      <section className="station-form-grid station-capabilities-tab">
        {toolKind === 'claude' && (
          <div className="station-form-field station-form-span-2">
            <div className="station-form-surface">
              <label className="station-form-checkbox">
                <input
                  type="checkbox"
                  checked={draft.globalCapabilitiesEnabled}
                  disabled={saving}
                  onChange={(event) => {
                    const checked = event.target.checked
                    setDraft((previous) => ({ ...previous, globalCapabilitiesEnabled: checked }))
                  }}
                />
                <span>
                  {locale === 'zh-CN'
                    ? '使用全域 Hook / Skill / 权限设定'
                    : 'Use global Hook/Skill/permission settings'}
                </span>
              </label>
              <p>
                {locale === 'zh-CN'
                  ? '关闭后，此 agent 不再自动套用你在 ~/.claude/settings.json 与 ~/.claude/skills/ 累积的全域设定（含权限允许清单）。下方全域清单仅供检视，无法逐项勾选或取消——要单独调整某一条全域 Hook/Skill，请改用下方「手动新增 / 其他已挂载」区块。'
                  : 'When off, this agent stops automatically inheriting your accumulated global ~/.claude/settings.json / ~/.claude/skills/ config (including the permission allowlist). The global list below is view-only and cannot be checked/unchecked item by item — to control one specific global Hook/Skill, use the "Manually Added / Other Mounted" section below instead.'}
              </p>
            </div>
          </div>
        )}

        <div className="station-form-segmented station-form-span-2">
          {(['mcp', 'skills', 'hooks'] as CapabilitySubTab[]).map((tab) => {
            const disabled = tab !== 'mcp' && !skillsHooksSupported
            const tabLabel =
              tab === 'mcp'
                ? 'MCP Servers'
                : tab === 'skills'
                  ? 'Skills'
                  : 'Hooks'
            return (
              <button
                key={tab}
                type="button"
                className={`station-form-inline-action${subTab === tab ? ' active' : ''}`}
                disabled={disabled}
                onClick={() => setSubTab(tab)}
              >
                {tabLabel}
              </button>
            )
          })}
        </div>

        {loading ? (
          <p className="station-form-span-2">
            {locale === 'zh-CN' ? '正在载入能力设定…' : 'Loading capabilities…'}
          </p>
        ) : loadError ? (
          <p className="station-form-span-2 station-form-error-text">
            {locale === 'zh-CN' ? `载入失败：${loadError}` : `Failed to load: ${loadError}`}
          </p>
        ) : (
          <>
            {subTab === 'mcp' && (
              <McpServersEditor
                locale={locale}
                servers={draft.mcpServers}
                disabled={saving}
                onChange={(mcpServers) => setDraft((previous) => ({ ...previous, mcpServers }))}
              />
            )}
            {subTab === 'skills' && !skillsHooksSupported && (
              <p className="station-form-span-2 station-form-surface">
                {locale === 'zh-CN'
                  ? 'Codex agent 尚未支援 Skills。'
                  : 'Skills are not yet supported for Codex agents.'}
              </p>
            )}
            {subTab === 'skills' && skillsHooksSupported && (
              <SkillsEditor
                locale={locale}
                workspaceId={workspaceId}
                skills={draft.skills}
                disabled={saving}
                globalCapabilitiesEnabled={toolKind === 'claude' ? draft.globalCapabilitiesEnabled : undefined}
                onChange={(skills) => setDraft((previous) => ({ ...previous, skills }))}
              />
            )}
            {subTab === 'hooks' && !skillsHooksSupported && (
              <p className="station-form-span-2 station-form-surface">
                {locale === 'zh-CN'
                  ? 'Codex agent 尚未支援 Hooks。'
                  : 'Hooks are not yet supported for Codex agents.'}
              </p>
            )}
            {subTab === 'hooks' && skillsHooksSupported && (
              <HooksEditor
                locale={locale}
                workspaceId={workspaceId}
                hooks={draft.hooks}
                disabled={saving}
                globalCapabilitiesEnabled={toolKind === 'claude' ? draft.globalCapabilitiesEnabled : undefined}
                onChange={(hooks) => setDraft((previous) => ({ ...previous, hooks }))}
              />
            )}
          </>
        )}

        {saveError && (
          <p className="station-form-span-2 station-form-error-text">
            {locale === 'zh-CN' ? `保存失败：${saveError}` : `Failed to save: ${saveError}`}
          </p>
        )}

        {preview && (
          <div className="station-hook-preview-backdrop" role="presentation">
            <div
              className="station-hook-preview-dialog"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="station-hook-preview-title"
            >
              <h3 id="station-hook-preview-title">
                {locale === 'zh-CN' ? '确认 Hook 内容' : 'Confirm Hook Content'}
              </h3>
              <p>
                {locale === 'zh-CN'
                  ? '以下指令会在对应事件触发时实际执行，请逐条确认后再保存。'
                  : 'These commands will actually run when their event fires — review each one before saving.'}
              </p>
              <ul className="station-hook-preview-list">
                {preview.items.map((item, index) => (
                  <li key={`${item.hash}-${index}`} className="station-form-surface">
                    <div className="station-hook-preview-item-header">
                      <strong>{item.event}</strong>
                      {item.alreadyConfirmed ? (
                        <span className="station-hook-preview-badge">
                          {locale === 'zh-CN' ? '已确认' : 'Already confirmed'}
                        </span>
                      ) : (
                        <span className="station-hook-preview-badge station-hook-preview-badge--new">
                          {locale === 'zh-CN' ? '新增 / 已变更' : 'New / changed'}
                        </span>
                      )}
                    </div>
                    <p>
                      {locale === 'zh-CN' ? 'Matcher：' : 'Matcher: '}
                      <code>{item.matcher?.trim() ? item.matcher : locale === 'zh-CN' ? '（全部）' : '(all tools)'}</code>
                    </p>
                    <p>
                      {locale === 'zh-CN' ? '实际指令：' : 'Command: '}
                      <code>{item.command}</code>
                    </p>
                    {item.note?.trim() && (
                      <p>
                        {locale === 'zh-CN' ? '备注：' : 'Note: '}
                        {item.note}
                      </p>
                    )}
                  </li>
                ))}
              </ul>
              {preview.error && (
                <p className="station-form-error-text">
                  {locale === 'zh-CN' ? `确认失败：${preview.error}` : `Failed to confirm: ${preview.error}`}
                </p>
              )}
              <div className="station-hook-preview-actions">
                <button
                  type="button"
                  className="station-form-btn subtle"
                  disabled={preview.confirming}
                  onClick={() => setPreview(null)}
                >
                  {locale === 'zh-CN' ? '取消' : 'Cancel'}
                </button>
                <button
                  type="button"
                  ref={confirmButtonRef}
                  className="station-form-btn"
                  disabled={preview.confirming}
                  onClick={() => void handleConfirmAndSave()}
                >
                  {preview.confirming
                    ? locale === 'zh-CN'
                      ? '保存中…'
                      : 'Saving…'
                    : locale === 'zh-CN'
                      ? '确认并保存'
                      : 'Confirm & Save'}
                </button>
              </div>
            </div>
          </div>
        )}
      </section>
    )
  },
)

function McpServersEditor({
  locale,
  servers,
  disabled,
  onChange,
}: {
  locale: Locale
  servers: McpServerCapability[]
  disabled: boolean
  onChange: (servers: McpServerCapability[]) => void
}) {
  const updateAt = (index: number, patch: Partial<McpServerCapability>) => {
    onChange(servers.map((server, i) => (i === index ? { ...server, ...patch } : server)))
  }
  const removeAt = (index: number) => {
    onChange(servers.filter((_, i) => i !== index))
  }
  return (
    <div className="station-form-field station-form-span-2 station-capabilities-list">
      {servers.length > 0 && (
        <div className="station-form-inline-row station-form-inline-row--end">
          <button
            type="button"
            className="station-form-inline-action"
            disabled={disabled}
            onClick={() => onChange(setAllMcpServersEnabled(servers, true))}
          >
            {locale === 'zh-CN' ? '全選開啟' : 'Enable All'}
          </button>
          <button
            type="button"
            className="station-form-inline-action"
            disabled={disabled}
            onClick={() => onChange(setAllMcpServersEnabled(servers, false))}
          >
            {locale === 'zh-CN' ? '全選關閉' : 'Disable All'}
          </button>
        </div>
      )}
      {servers.map((server, index) => (
        <div key={index} className="station-form-surface station-capabilities-row">
          <div className="station-capabilities-row-header">
            <label className="station-form-field station-capabilities-row-name-field">
              <span>{locale === 'zh-CN' ? '显示名称（Agent Canvas 节点用）' : 'Display name (used on Agent Canvas node)'}</span>
              <input
                type="text"
                placeholder={locale === 'zh-CN' ? '例如：文件搜索' : 'e.g. File Search'}
                disabled={disabled}
                value={server.name ?? ''}
                onChange={(event) => updateAt(index, { name: event.target.value })}
              />
            </label>
            <label className="station-form-field station-capabilities-row-id-field">
              <span>Server ID</span>
              <input
                type="text"
                placeholder="server-id"
                disabled={disabled}
                value={server.id}
                onChange={(event) => updateAt(index, { id: event.target.value })}
              />
            </label>
            <label className="station-form-field station-capabilities-row-transport-field">
              <span>{locale === 'zh-CN' ? '传输方式' : 'Transport'}</span>
              <select
                disabled={disabled}
                value={server.transport}
                onChange={(event) => updateAt(index, { transport: event.target.value as McpTransport })}
              >
                <option value="stdio">stdio</option>
                <option value="sse">sse</option>
                <option value="http">http</option>
              </select>
            </label>
            <div className="station-capabilities-row-actions">
              <button
                type="button"
                role="switch"
                aria-checked={server.enabled}
                aria-label={locale === 'zh-CN' ? '启用此 MCP 服务' : 'Enable this MCP server'}
                className={`station-capabilities-row-enabled-toggle${server.enabled ? ' active' : ''}`}
                disabled={disabled}
                onClick={() => updateAt(index, { enabled: !server.enabled })}
              >
                <span className="station-capabilities-row-enabled-toggle-thumb" />
              </button>
              <button
                type="button"
                className="station-form-tag-chip-delete"
                disabled={disabled}
                onClick={() => removeAt(index)}
                aria-label={locale === 'zh-CN' ? '移除' : 'Remove'}
              >
                <AppIcon name="close" className="vb-icon" aria-hidden="true" />
              </button>
            </div>
          </div>
          {server.transport === 'stdio' ? (
            <>
              <label className="station-form-field">
                <span>{locale === 'zh-CN' ? '指令' : 'Command'}</span>
                <input
                  type="text"
                  placeholder="npx"
                  disabled={disabled}
                  value={server.command ?? ''}
                  onChange={(event) => updateAt(index, { command: event.target.value })}
                />
              </label>
              <label className="station-form-field">
                <span>{locale === 'zh-CN' ? '参数（每行一个）' : 'Args (one per line)'}</span>
                <textarea
                  rows={2}
                  disabled={disabled}
                  value={formatArgsTextarea(server.args)}
                  onChange={(event) => updateAt(index, { args: parseArgsTextarea(event.target.value) })}
                />
              </label>
              <label className="station-form-field">
                <span>{locale === 'zh-CN' ? '环境变量（KEY=value，每行一个）' : 'Env vars (KEY=value, one per line)'}</span>
                <textarea
                  rows={2}
                  disabled={disabled}
                  value={formatEnvTextarea(server.env)}
                  onChange={(event) => updateAt(index, { env: parseEnvTextarea(event.target.value) })}
                />
              </label>
            </>
          ) : (
            <label className="station-form-field">
              <span>URL</span>
              <input
                type="text"
                placeholder="https://example.com/mcp"
                disabled={disabled}
                value={server.url ?? ''}
                onChange={(event) => updateAt(index, { url: event.target.value })}
              />
            </label>
          )}
          {!isMcpServerDraftValid(server) && (
            <p className="station-form-error-text">
              {locale === 'zh-CN'
                ? '未完成的项目不会被保存。'
                : 'Incomplete rows are not saved.'}
            </p>
          )}
        </div>
      ))}
      <button
        type="button"
        className="station-form-inline-action"
        disabled={disabled}
        onClick={() => onChange([...servers, createEmptyMcpServer()])}
      >
        <AppIcon name="plus" className="vb-icon" aria-hidden="true" />
        {locale === 'zh-CN' ? '新增 MCP Server' : 'Add MCP Server'}
      </button>
    </div>
  )
}

function SkillsEditor({
  locale,
  workspaceId,
  skills,
  disabled,
  globalCapabilitiesEnabled,
  onChange,
}: {
  locale: Locale
  workspaceId: string
  skills: SkillCapability[]
  disabled: boolean
  /** Whether the agent-level "use global capabilities" master switch is on.
   * The global-scope checklist below is always view-only regardless of this
   * value — it only changes the badge each row shows (applied vs. not). */
  globalCapabilitiesEnabled?: boolean
  onChange: (skills: SkillCapability[]) => void
}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [searchQuery, setSearchQuery] = useState('')
  const [available, setAvailable] = useState<DiscoveredSkill[] | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    desktopApi
      .agentCapabilityListAvailableSkills({ workspaceId })
      .then((response) => {
        if (cancelled) return
        setAvailable(response.skills)
        setLoadError(null)
      })
      .catch((error) => {
        if (cancelled) return
        setLoadError(error instanceof Error ? error.message : String(error))
        setAvailable((previous) => previous ?? [])
      })
    return () => {
      cancelled = true
    }
  }, [workspaceId])

  const toggleExpanded = (key: string) => {
    setExpanded((previous) => {
      const next = new Set(previous)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
      }
      return next
    })
  }

  const workspaceSkillsAll = available?.filter((skill) => skill.scope === 'workspace') ?? []
  const globalSkillsAll = available?.filter((skill) => skill.scope === 'global') ?? []
  const unmatchedAll = available ? unmatchedSkillEntries(skills, available) : []

  const hasQuery = searchQuery.trim().length > 0
  const workspaceSkills = filterDiscoveredSkills(workspaceSkillsAll, searchQuery)
  const globalSkills = filterDiscoveredSkills(globalSkillsAll, searchQuery)
  const unmatched = filterSkillEntries(unmatchedAll, searchQuery)

  const noSearchMatchLabel =
    locale === 'zh-CN' ? '没有符合搜尋條件的技能。' : 'No skills match your search.'

  return (
    <div className="station-form-field station-form-span-2 station-capabilities-list">
      {available === null && !loadError && (
        <p>{locale === 'zh-CN' ? '正在扫描可用技能…' : 'Scanning available skills…'}</p>
      )}
      {loadError && (
        <p className="station-form-error-text">
          {locale === 'zh-CN' ? `扫描失败：${loadError}` : `Failed to scan: ${loadError}`}
        </p>
      )}
      {available !== null && (
        <>
          <div className="station-form-inline-row station-form-inline-row--end">
            <button
              type="button"
              className="station-form-inline-action"
              disabled={disabled}
              onClick={() => onChange(setAllSkillsEnabled(skills, workspaceSkillsAll, true))}
            >
              {locale === 'zh-CN' ? '全選開啟' : 'Enable All'}
            </button>
            <button
              type="button"
              className="station-form-inline-action"
              disabled={disabled}
              onClick={() => onChange(setAllSkillsEnabled(skills, workspaceSkillsAll, false))}
            >
              {locale === 'zh-CN' ? '全選關閉' : 'Disable All'}
            </button>
          </div>
          <label className="station-capabilities-skill-search">
            <AppIcon name="search" className="vb-icon" aria-hidden="true" />
            <input
              type="text"
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder={
                locale === 'zh-CN'
                  ? '搜尋技能名称、ID 或说明…'
                  : 'Search skills by name, id, or description…'
              }
              aria-label={locale === 'zh-CN' ? '搜尋技能' : 'Search skills'}
            />
          </label>
          <SkillChecklistGroup
            locale={locale}
            title={locale === 'zh-CN' ? '专案工作区技能' : 'Project Workspace Skills'}
            emptyLabel={
              hasQuery && workspaceSkillsAll.length > 0
                ? noSearchMatchLabel
                : locale === 'zh-CN'
                  ? '这个工作区的 .claude/skills/ 底下没有找到技能。'
                  : 'No skills found under this workspace’s .claude/skills/.'
            }
            items={workspaceSkills}
            skills={skills}
            disabled={disabled}
            expanded={expanded}
            onToggleExpanded={toggleExpanded}
            onChange={onChange}
          />
          {globalCapabilitiesEnabled !== undefined && (
            <SkillChecklistGroup
              locale={locale}
              title={
                locale === 'zh-CN' ? '全域安装技能（仅供检视）' : 'Globally Installed Skills (view-only)'
              }
              emptyLabel={
                hasQuery && globalSkillsAll.length > 0
                  ? noSearchMatchLabel
                  : locale === 'zh-CN'
                    ? '没有找到全域安装的技能。'
                    : 'No globally installed skills found.'
              }
              items={globalSkills}
              skills={skills}
              disabled={disabled}
              viewOnly
              viewOnlyActive={globalCapabilitiesEnabled}
              expanded={expanded}
              onToggleExpanded={toggleExpanded}
              onChange={onChange}
            />
          )}
          {unmatched.length > 0 && (
            <div className="station-capabilities-skill-group">
              <h4>
                {locale === 'zh-CN' ? '手动新增 / 其他已挂载' : 'Manually Added / Other Mounted'}
              </h4>
              {unmatched.map((skill) => (
                <div key={skill.sourcePath} className="station-form-surface station-capabilities-skill-row">
                  <div className="station-capabilities-row-header">
                    <label className="station-form-checkbox station-capabilities-skill-checkbox">
                      <input
                        type="checkbox"
                        disabled={disabled}
                        checked={skill.enabled}
                        onChange={(event) =>
                          onChange(
                            skills.map((entry) =>
                              entry.sourcePath === skill.sourcePath
                                ? { ...entry, enabled: event.target.checked }
                                : entry,
                            ),
                          )
                        }
                      />
                      <span>{skill.id}</span>
                    </label>
                    <button
                      type="button"
                      className="station-form-tag-chip-delete"
                      disabled={disabled}
                      onClick={() => onChange(skills.filter((entry) => entry.sourcePath !== skill.sourcePath))}
                      aria-label={locale === 'zh-CN' ? '移除' : 'Remove'}
                    >
                      <AppIcon name="close" className="vb-icon" aria-hidden="true" />
                    </button>
                  </div>
                  <p className="station-capabilities-skill-path">{skill.sourcePath}</p>
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  )
}

function SkillChecklistGroup({
  locale,
  title,
  emptyLabel,
  items,
  skills,
  disabled,
  viewOnly,
  viewOnlyActive,
  expanded,
  onToggleExpanded,
  onChange,
}: {
  locale: Locale
  title: string
  emptyLabel: string
  items: DiscoveredSkill[]
  skills: SkillCapability[]
  disabled: boolean
  /** When true, this group renders as a plain read-only list — no checkbox,
   * no per-row on/off control — since every row here shares one fate
   * decided entirely by the agent-level "use global capabilities" master
   * switch (see docs/cw/21_全域Hook_Skill開關設計.md): there is no
   * meaningful "check just this one" action to offer. */
  viewOnly?: boolean
  /** Only meaningful when `viewOnly` is true — whether the master switch is
   * currently on, shown as a status badge per row instead of a checkbox. */
  viewOnlyActive?: boolean
  expanded: Set<string>
  onToggleExpanded: (key: string) => void
  onChange: (skills: SkillCapability[]) => void
}) {
  return (
    <div className="station-capabilities-skill-group">
      <h4>{title}</h4>
      {items.length === 0 ? (
        <p className="station-capabilities-skill-empty">{emptyLabel}</p>
      ) : (
        items.map((item) => {
          const key = `${item.scope}:${item.sourcePath}`
          const mounted = findMountedSkillEntry(skills, item)
          const checked = Boolean(mounted?.enabled)
          const isExpanded = expanded.has(key)
          return (
            <div key={key} className="station-form-surface station-capabilities-skill-row">
              <div className="station-capabilities-row-header">
                {viewOnly ? (
                  <div className="station-form-checkbox station-capabilities-skill-checkbox">
                    <span>{item.name}</span>
                  </div>
                ) : (
                  <label className="station-form-checkbox station-capabilities-skill-checkbox">
                    <input
                      type="checkbox"
                      disabled={disabled}
                      checked={checked}
                      onChange={(event) => onChange(toggleDiscoveredSkill(skills, item, event.target.checked))}
                    />
                    <span>{item.name}</span>
                  </label>
                )}
                {viewOnly && (
                  <span
                    className={`station-hook-preview-badge${viewOnlyActive ? ' station-hook-preview-badge--new' : ''}`}
                  >
                    {viewOnlyActive
                      ? locale === 'zh-CN'
                        ? '套用中'
                        : 'Applied'
                      : locale === 'zh-CN'
                        ? '未套用'
                        : 'Not applied'}
                  </span>
                )}
                <button
                  type="button"
                  className="station-capabilities-skill-info-toggle"
                  onClick={() => onToggleExpanded(key)}
                  aria-expanded={isExpanded}
                  aria-label={locale === 'zh-CN' ? '查看技能说明' : 'View skill details'}
                >
                  <AppIcon name="info" className="vb-icon" aria-hidden="true" />
                </button>
              </div>
              {isExpanded && (
                <div className="station-capabilities-skill-detail">
                  <p>
                    {item.description ||
                      (locale === 'zh-CN' ? '（没有提供说明）' : '(No description provided.)')}
                  </p>
                  <p className="station-capabilities-skill-path">{item.sourcePath}</p>
                </div>
              )}
            </div>
          )
        })
      )}
    </div>
  )
}

function HooksEditor({
  locale,
  workspaceId,
  hooks,
  disabled,
  globalCapabilitiesEnabled,
  onChange,
}: {
  locale: Locale
  workspaceId: string
  hooks: HookCapability[]
  disabled: boolean
  /** See the analogous prop on `SkillsEditor`. */
  globalCapabilitiesEnabled?: boolean
  onChange: (hooks: HookCapability[]) => void
}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [searchQuery, setSearchQuery] = useState('')
  const [available, setAvailable] = useState<DiscoveredHook[] | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    desktopApi
      .agentCapabilityListAvailableHooks({ workspaceId })
      .then((response) => {
        if (cancelled) return
        setAvailable(response.hooks)
        setLoadError(null)
      })
      .catch((error) => {
        if (cancelled) return
        setLoadError(error instanceof Error ? error.message : String(error))
        setAvailable((previous) => previous ?? [])
      })
    return () => {
      cancelled = true
    }
  }, [workspaceId])

  const toggleExpanded = (key: string) => {
    setExpanded((previous) => {
      const next = new Set(previous)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
      }
      return next
    })
  }

  const updateManual = (target: HookCapability, patch: Partial<HookCapability>) => {
    onChange(hooks.map((hook) => (hook === target ? { ...hook, ...patch } : hook)))
  }
  const removeManual = (target: HookCapability) => {
    onChange(hooks.filter((hook) => hook !== target))
  }

  const workspaceHooksAll = available?.filter((hook) => hook.scope === 'workspace') ?? []
  const globalHooksAll = available?.filter((hook) => hook.scope === 'global') ?? []
  // Reference-preserving filter (`unmatchedHookEntries` only drops entries,
  // never clones survivors), so `updateManual`/`removeManual` can match rows
  // by identity below without needing an id field on `HookCapability`.
  const manualAll = available ? unmatchedHookEntries(hooks, available) : hooks

  const hasQuery = searchQuery.trim().length > 0
  const workspaceHooks = filterDiscoveredHooks(workspaceHooksAll, searchQuery)
  const globalHooks = filterDiscoveredHooks(globalHooksAll, searchQuery)
  const manualHooks = filterHookEntries(manualAll, searchQuery)

  const noSearchMatchLabel =
    locale === 'zh-CN' ? '没有符合搜尋條件的 Hook。' : 'No hooks match your search.'

  return (
    <div className="station-form-field station-form-span-2 station-capabilities-list">
      <p>
        {locale === 'zh-CN'
          ? 'Hook 指令会在保存前进入完整预览画面，逐条确认后才会真正生效——不论是从下方清单勾选，还是手动新增。'
          : 'Hooks go through a full preview screen before saving — nothing takes effect until you confirm each one, whether checked from the list below or added manually.'}
      </p>

      {loadError && (
        <p className="station-form-error-text">
          {locale === 'zh-CN' ? `扫描失败：${loadError}` : `Failed to scan: ${loadError}`}
        </p>
      )}
      {available === null && !loadError && (
        <p>{locale === 'zh-CN' ? '正在扫描已设定的 Hook…' : 'Scanning configured hooks…'}</p>
      )}
      {available !== null && (
        <>
          <div className="station-form-inline-row station-form-inline-row--end">
            <button
              type="button"
              className="station-form-inline-action"
              disabled={disabled}
              onClick={() => onChange(setAllHooksEnabled(hooks, workspaceHooksAll, true))}
            >
              {locale === 'zh-CN' ? '全選開啟' : 'Enable All'}
            </button>
            <button
              type="button"
              className="station-form-inline-action"
              disabled={disabled}
              onClick={() => onChange(setAllHooksEnabled(hooks, workspaceHooksAll, false))}
            >
              {locale === 'zh-CN' ? '全選關閉' : 'Disable All'}
            </button>
          </div>
          <label className="station-capabilities-skill-search">
            <AppIcon name="search" className="vb-icon" aria-hidden="true" />
            <input
              type="text"
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder={
                locale === 'zh-CN'
                  ? '搜尋事件、matcher 或指令…'
                  : 'Search by event, matcher, or command…'
              }
              aria-label={locale === 'zh-CN' ? '搜尋 Hook' : 'Search hooks'}
            />
          </label>
          <HookChecklistGroup
            locale={locale}
            title={locale === 'zh-CN' ? '专案工作区已设定的 Hook' : 'Hooks Configured in This Workspace'}
            emptyLabel={
              hasQuery && workspaceHooksAll.length > 0
                ? noSearchMatchLabel
                : locale === 'zh-CN'
                  ? '这个工作区的 .claude/settings.json 底下没有找到 Hook。'
                  : 'No hooks found in this workspace’s .claude/settings.json.'
            }
            items={workspaceHooks}
            hooks={hooks}
            disabled={disabled}
            expanded={expanded}
            onToggleExpanded={toggleExpanded}
            onChange={onChange}
          />
          {globalCapabilitiesEnabled !== undefined && (
            <HookChecklistGroup
              locale={locale}
              title={
                locale === 'zh-CN' ? '全域已设定的 Hook（仅供检视）' : 'Globally Configured Hooks (view-only)'
              }
              emptyLabel={
                hasQuery && globalHooksAll.length > 0
                  ? noSearchMatchLabel
                  : locale === 'zh-CN'
                    ? '没有找到全域设定的 Hook。'
                    : 'No globally configured hooks found.'
              }
              items={globalHooks}
              hooks={hooks}
              disabled={disabled}
              viewOnly
              viewOnlyActive={globalCapabilitiesEnabled}
              expanded={expanded}
              onToggleExpanded={toggleExpanded}
              onChange={onChange}
            />
          )}
        </>
      )}

      <div className="station-capabilities-skill-group">
        <h4>{locale === 'zh-CN' ? '手动新增 / 其他已挂载的 Hook' : 'Manually Added / Other Mounted Hooks'}</h4>
        {manualHooks.length === 0 && !hasQuery && (
          <p className="station-capabilities-skill-empty">
            {locale === 'zh-CN'
              ? '还没有手动新增的 Hook。'
              : 'No manually added hooks yet.'}
          </p>
        )}
        {manualHooks.length === 0 && hasQuery && (
          <p className="station-capabilities-skill-empty">{noSearchMatchLabel}</p>
        )}
        {manualHooks.map((hook, index) => (
          <div key={index} className="station-form-surface station-capabilities-row">
            <div className="station-capabilities-row-header">
              <select
                disabled={disabled}
                value={hook.event}
                onChange={(event) => updateManual(hook, { event: event.target.value })}
              >
                {HOOK_EVENT_OPTIONS.map((option) => (
                  <option key={option} value={option}>
                    {option}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="station-form-tag-chip-delete"
                disabled={disabled}
                onClick={() => removeManual(hook)}
                aria-label={locale === 'zh-CN' ? '移除' : 'Remove'}
              >
                <AppIcon name="close" className="vb-icon" aria-hidden="true" />
              </button>
            </div>
            <label className="station-form-field">
              <span>{locale === 'zh-CN' ? 'Matcher（留空 = 全部工具）' : 'Matcher (empty = all tools)'}</span>
              <input
                type="text"
                placeholder="Bash"
                disabled={disabled}
                value={hook.matcher ?? ''}
                onChange={(event) => updateManual(hook, { matcher: event.target.value })}
              />
            </label>
            <label className="station-form-field">
              <span>{locale === 'zh-CN' ? '实际执行的指令' : 'Command to run'}</span>
              <textarea
                rows={2}
                disabled={disabled}
                value={hook.command}
                onChange={(event) => updateManual(hook, { command: event.target.value })}
              />
            </label>
            <label className="station-form-field">
              <span>{locale === 'zh-CN' ? '备注（触发时机与用途说明）' : 'Note (when it fires and what it does)'}</span>
              <textarea
                rows={2}
                disabled={disabled}
                value={hook.note ?? ''}
                onChange={(event) => updateManual(hook, { note: event.target.value })}
              />
            </label>
            {!isHookDraftValid(hook) && (
              <p className="station-form-error-text">
                {locale === 'zh-CN'
                  ? '未完成的项目不会被保存。'
                  : 'Incomplete rows are not saved.'}
              </p>
            )}
          </div>
        ))}
        <button
          type="button"
          className="station-form-inline-action"
          disabled={disabled}
          onClick={() => onChange([...hooks, createEmptyHook()])}
        >
          <AppIcon name="plus" className="vb-icon" aria-hidden="true" />
          {locale === 'zh-CN' ? '新增 Hook' : 'Add Hook'}
        </button>
      </div>
    </div>
  )
}

function HookChecklistGroup({
  locale,
  title,
  emptyLabel,
  items,
  hooks,
  disabled,
  viewOnly,
  viewOnlyActive,
  expanded,
  onToggleExpanded,
  onChange,
}: {
  locale: Locale
  title: string
  emptyLabel: string
  items: DiscoveredHook[]
  hooks: HookCapability[]
  disabled: boolean
  /** See the identical prop on `SkillChecklistGroup`. */
  viewOnly?: boolean
  /** See the identical prop on `SkillChecklistGroup`. */
  viewOnlyActive?: boolean
  expanded: Set<string>
  onToggleExpanded: (key: string) => void
  onChange: (hooks: HookCapability[]) => void
}) {
  return (
    <div className="station-capabilities-skill-group">
      <h4>{title}</h4>
      {items.length === 0 ? (
        <p className="station-capabilities-skill-empty">{emptyLabel}</p>
      ) : (
        items.map((item) => {
          const key = `${item.scope}:${item.sourcePath}:${item.event}:${item.matcher ?? ''}:${item.command}`
          const mounted = findMountedHookEntry(hooks, item)
          const checked = Boolean(mounted)
          const isExpanded = expanded.has(key)
          const label = item.matcher ? `${item.event} · ${item.matcher}` : item.event
          return (
            <div key={key} className="station-form-surface station-capabilities-skill-row">
              <div className="station-capabilities-row-header">
                {viewOnly ? (
                  <div className="station-form-checkbox station-capabilities-skill-checkbox">
                    <span>{label}</span>
                  </div>
                ) : (
                  <label className="station-form-checkbox station-capabilities-skill-checkbox">
                    <input
                      type="checkbox"
                      disabled={disabled}
                      checked={checked}
                      onChange={(event) => onChange(toggleDiscoveredHook(hooks, item, event.target.checked))}
                    />
                    <span>{label}</span>
                  </label>
                )}
                {viewOnly && (
                  <span
                    className={`station-hook-preview-badge${viewOnlyActive ? ' station-hook-preview-badge--new' : ''}`}
                  >
                    {viewOnlyActive
                      ? locale === 'zh-CN'
                        ? '套用中'
                        : 'Applied'
                      : locale === 'zh-CN'
                        ? '未套用'
                        : 'Not applied'}
                  </span>
                )}
                <button
                  type="button"
                  className="station-capabilities-skill-info-toggle"
                  onClick={() => onToggleExpanded(key)}
                  aria-expanded={isExpanded}
                  aria-label={locale === 'zh-CN' ? '查看 Hook 指令' : 'View hook command'}
                >
                  <AppIcon name="info" className="vb-icon" aria-hidden="true" />
                </button>
              </div>
              {isExpanded && (
                <div className="station-capabilities-skill-detail">
                  {item.inferredDescription && (
                    <p>
                      {locale === 'zh-CN'
                        ? '推测的功能说明（读取脚本开头注释，仅供参考）：'
                        : 'Inferred description (from the script’s leading comment, for reference only): '}
                      {item.inferredDescription}
                    </p>
                  )}
                  <p>
                    {locale === 'zh-CN' ? '实际执行的指令：' : 'Command: '}
                    <code>{item.command}</code>
                  </p>
                  <p className="station-capabilities-skill-path">{item.sourcePath}</p>
                  {!viewOnly && mounted && (
                    <label className="station-form-field">
                      <span>{locale === 'zh-CN' ? '备注（触发时机与用途说明）' : 'Note (when it fires and what it does)'}</span>
                      <textarea
                        rows={2}
                        disabled={disabled}
                        value={mounted.note ?? ''}
                        onChange={(event) => onChange(updateMountedHookNote(hooks, item, event.target.value))}
                      />
                    </label>
                  )}
                </div>
              )}
            </div>
          )
        })
      )}
    </div>
  )
}
