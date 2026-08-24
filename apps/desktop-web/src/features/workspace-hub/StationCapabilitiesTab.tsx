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
  createEmptySkill,
  formatArgsTextarea,
  formatEnvTextarea,
  isMcpServerDraftValid,
  isHookDraftValid,
  isSkillDraftValid,
  isSkillsOrHooksSupportedForToolKind,
  parseArgsTextarea,
  parseEnvTextarea,
  requiresHookPreviewBeforeSave,
  unconfirmedHookHashes,
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
}

export const StationCapabilitiesTab = forwardRef<StationCapabilitiesTabHandle, StationCapabilitiesTabProps>(
  function StationCapabilitiesTab({ locale, workspaceId, agentId, toolKind, active, onSavingChange }, ref) {
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
          return true
        } catch (error) {
          setSaveError(error instanceof Error ? error.message : String(error))
          return false
        } finally {
          setSaving(false)
        }
      },
      [workspaceId, agentId],
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
                skills={draft.skills}
                disabled={saving}
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
                hooks={draft.hooks}
                disabled={saving}
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
      {servers.map((server, index) => (
        <div key={index} className="station-form-surface station-capabilities-row">
          <div className="station-capabilities-row-header">
            <input
              type="text"
              placeholder={locale === 'zh-CN' ? '名称（显示用）' : 'Name (display only)'}
              disabled={disabled}
              value={server.name ?? ''}
              onChange={(event) => updateAt(index, { name: event.target.value })}
            />
            <input
              type="text"
              placeholder="Server ID"
              disabled={disabled}
              value={server.id}
              onChange={(event) => updateAt(index, { id: event.target.value })}
            />
            <select
              disabled={disabled}
              value={server.transport}
              onChange={(event) => updateAt(index, { transport: event.target.value as McpTransport })}
            >
              <option value="stdio">stdio</option>
              <option value="sse">sse</option>
              <option value="http">http</option>
            </select>
            <label className="station-capabilities-row-enabled-toggle">
              <input
                type="checkbox"
                disabled={disabled}
                checked={server.enabled}
                onChange={(event) => updateAt(index, { enabled: event.target.checked })}
              />
              <span>{locale === 'zh-CN' ? '启用' : 'Enabled'}</span>
            </label>
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
  skills,
  disabled,
  onChange,
}: {
  locale: Locale
  skills: SkillCapability[]
  disabled: boolean
  onChange: (skills: SkillCapability[]) => void
}) {
  const updateAt = (index: number, patch: Partial<SkillCapability>) => {
    onChange(skills.map((skill, i) => (i === index ? { ...skill, ...patch } : skill)))
  }
  const removeAt = (index: number) => {
    onChange(skills.filter((_, i) => i !== index))
  }
  return (
    <div className="station-form-field station-form-span-2 station-capabilities-list">
      {skills.map((skill, index) => (
        <div key={index} className="station-form-surface station-capabilities-row">
          <div className="station-capabilities-row-header">
            <input
              type="text"
              placeholder={locale === 'zh-CN' ? 'Skill ID' : 'Skill ID'}
              disabled={disabled}
              value={skill.id}
              onChange={(event) => updateAt(index, { id: event.target.value })}
            />
            <label className="station-form-checkbox">
              <input
                type="checkbox"
                disabled={disabled}
                checked={skill.enabled}
                onChange={(event) => updateAt(index, { enabled: event.target.checked })}
              />
              <span>{locale === 'zh-CN' ? '启用' : 'Enabled'}</span>
            </label>
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
          <label className="station-form-field">
            <span>{locale === 'zh-CN' ? '本机 SKILL.md 路径' : 'Local SKILL.md path'}</span>
            <input
              type="text"
              disabled={disabled}
              value={skill.sourcePath}
              onChange={(event) => updateAt(index, { sourcePath: event.target.value })}
            />
          </label>
          {!isSkillDraftValid(skill) && (
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
        onClick={() => onChange([...skills, createEmptySkill()])}
      >
        <AppIcon name="plus" className="vb-icon" aria-hidden="true" />
        {locale === 'zh-CN' ? '新增 Skill' : 'Add Skill'}
      </button>
    </div>
  )
}

function HooksEditor({
  locale,
  hooks,
  disabled,
  onChange,
}: {
  locale: Locale
  hooks: HookCapability[]
  disabled: boolean
  onChange: (hooks: HookCapability[]) => void
}) {
  const updateAt = (index: number, patch: Partial<HookCapability>) => {
    onChange(hooks.map((hook, i) => (i === index ? { ...hook, ...patch } : hook)))
  }
  const removeAt = (index: number) => {
    onChange(hooks.filter((_, i) => i !== index))
  }
  return (
    <div className="station-form-field station-form-span-2 station-capabilities-list">
      <p>
        {locale === 'zh-CN'
          ? 'Hook 指令会在保存前进入完整预览画面，逐条确认后才会真正生效。'
          : 'Hooks go through a full preview screen before saving — nothing takes effect until you confirm each one.'}
      </p>
      {hooks.map((hook, index) => (
        <div key={index} className="station-form-surface station-capabilities-row">
          <div className="station-capabilities-row-header">
            <select
              disabled={disabled}
              value={hook.event}
              onChange={(event) => updateAt(index, { event: event.target.value })}
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
              onClick={() => removeAt(index)}
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
              onChange={(event) => updateAt(index, { matcher: event.target.value })}
            />
          </label>
          <label className="station-form-field">
            <span>{locale === 'zh-CN' ? '实际执行的指令' : 'Command to run'}</span>
            <textarea
              rows={2}
              disabled={disabled}
              value={hook.command}
              onChange={(event) => updateAt(index, { command: event.target.value })}
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
  )
}
