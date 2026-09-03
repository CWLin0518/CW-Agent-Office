import type { AgentStation } from '@features/workspace-hub'

export interface TaskDraftState {
  markdown: string
  // A previously unsent draft restored from storage. Kept out of `markdown`
  // so it never gets silently combined with newly typed text — the editor
  // only pulls it in when the user explicitly accepts it (Tab), mirroring
  // the CLI's own ghost-text-suggestion pattern.
  previewMarkdown: string
  targetStationIds: string[]
}

export interface TaskCenterNotice {
  kind: 'info' | 'success' | 'error'
  message: string
}

// Structurally matches desktop-api.ts's TaskDispatchPreviewTarget without
// importing it, so this model stays free of Tauri-runtime-specific types.
export interface TaskSendPreviewTarget {
  targetAgentId: string
  appendedText: string
}

export interface TaskSendPreviewGroup {
  appendedText: string
  targetAgentIds: string[]
}

// Groups targets by the exact text dispatch would append after the sender's
// own markdown, so identically-configured targets render as one preview
// block instead of one per agent. Targets with nothing appended are dropped.
export function groupTaskSendPreviewTargets(
  targets: TaskSendPreviewTarget[],
): TaskSendPreviewGroup[] {
  const groups: TaskSendPreviewGroup[] = []
  const groupIndexByText = new Map<string, number>()
  targets.forEach(({ targetAgentId, appendedText }) => {
    const trimmed = appendedText.trim()
    if (!trimmed) {
      return
    }
    const existingIndex = groupIndexByText.get(trimmed)
    if (existingIndex !== undefined) {
      groups[existingIndex].targetAgentIds.push(targetAgentId)
      return
    }
    groupIndexByText.set(trimmed, groups.length)
    groups.push({ appendedText: trimmed, targetAgentIds: [targetAgentId] })
  })
  return groups
}

export type TaskDispatchStatus = 'sending' | 'sent' | 'failed'

export interface TaskDispatchRecord {
  batchId: string
  taskId: string
  title: string
  markdown: string
  targetStationId: string
  targetStationName: string
  createdAtMs: number
  status: TaskDispatchStatus
  taskFilePath?: string | null
  detail?: string
}

export interface StationTaskSignal {
  nonce: number
  taskId: string
  title: string
  receivedAtMs: number
}

export interface TaskCenterWorkspaceSnapshot {
  version: 2
  updatedAtMs: number
  draft: TaskDraftState
  dispatchHistory: TaskDispatchRecord[]
}

export type TaskMarkdownSnippet = 'heading' | 'code' | 'checklist'

export const DEFAULT_TASK_QUICK_DISPATCH_OPACITY = 0.9

const TASK_CENTER_LOCAL_STORAGE_PREFIX = 'gtoffice.task-center'
const TASK_CENTER_DRAFT_FILE_REL = '.gtoffice/tasks/.task-center-draft.json'

function dedupeStationIds(stationIds: string[]): string[] {
  const seen = new Set<string>()
  const result: string[] = []
  stationIds.forEach((stationId) => {
    const normalized = stationId.trim()
    if (!normalized || seen.has(normalized)) {
      return
    }
    seen.add(normalized)
    result.push(normalized)
  })
  return result
}

export function createInitialTaskDraft(
  stations: AgentStation[],
  activeStationId: string,
): TaskDraftState {
  const hasActive = stations.some((station) => station.id === activeStationId)
  const fallback = stations[0]?.id ?? ''
  return {
    markdown: '',
    previewMarkdown: '',
    targetStationIds: hasActive ? [activeStationId] : fallback ? [fallback] : [],
  }
}

const TASK_DRAFT_PREVIEW_MERGE_SEPARATOR = '\n\n---\n\n'

// Splits a restored draft into a fresh empty editor plus a recallable preview,
// so a stale/unsent draft from a previous session never re-merges into new
// typing unnoticed. Merges the restored `markdown` (most recent unsent
// content) with any previously-restored, never-consumed preview — neither is
// dropped silently, since a user may have left the last session with both an
// in-progress edit and an untouched preview still sitting around.
export function splitRestoredTaskDraftIntoPreview(draft: TaskDraftState): TaskDraftState {
  const parts = [draft.previewMarkdown, draft.markdown]
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
  return {
    ...draft,
    markdown: '',
    previewMarkdown: parts.join(TASK_DRAFT_PREVIEW_MERGE_SEPARATOR),
  }
}

// Inserts the preview text at the given cursor position in the current
// markdown, returning the new value and the cursor position just after the
// inserted text (so the caller can restore selection/focus).
export function insertTaskDraftPreviewAtCursor(
  markdown: string,
  previewMarkdown: string,
  cursor: number,
): { markdown: string; cursor: number } {
  const safeCursor = Math.max(0, Math.min(cursor, markdown.length))
  const nextMarkdown = markdown.slice(0, safeCursor) + previewMarkdown + markdown.slice(safeCursor)
  return {
    markdown: nextMarkdown,
    cursor: safeCursor + previewMarkdown.length,
  }
}

export function areTaskTargetsEqual(left: string[], right: string[]): boolean {
  if (left.length !== right.length) {
    return false
  }
  return left.every((item, index) => item === right[index])
}

export function resolveValidTaskTargets(
  stations: AgentStation[],
  draftTargetStationIds: string[],
): string[] {
  const stationIdSet = new Set(stations.map((station) => station.id))
  return dedupeStationIds(draftTargetStationIds).filter((stationId) =>
    stationIdSet.has(stationId),
  )
}

export function toggleTaskTarget(
  previous: string[],
  stationId: string,
  checked: boolean,
): string[] {
  const normalized = stationId.trim()
  if (!normalized) {
    return previous
  }
  if (checked) {
    return dedupeStationIds([...previous, normalized])
  }
  return previous.filter((item) => item !== normalized)
}

export function extractTaskTitleFromMarkdown(markdown: string): string {
  const trimmed = markdown.trim()
  if (!trimmed) {
    return '未命名任务'
  }

  const lines = trimmed
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)

  if (lines.length === 0) {
    return '未命名任务'
  }

  const firstHeading = lines.find((line) => /^#{1,6}\s+/.test(line))
  const candidate = (firstHeading ?? lines[0]).replace(/^#{1,6}\s+/, '').trim()
  if (!candidate) {
    return '未命名任务'
  }

  const compact = candidate.replace(/\s+/g, ' ')
  return compact.length > 72 ? `${compact.slice(0, 72)}...` : compact
}

export function buildDispatchRecord(input: {
  batchId: string
  taskId: string
  title: string
  markdown: string
  targetStationId: string
  targetStationName: string
  createdAtMs: number
  status: TaskDispatchStatus
  taskFilePath?: string | null
  detail?: string
}): TaskDispatchRecord {
  return {
    batchId: input.batchId,
    taskId: input.taskId,
    title: input.title,
    markdown: input.markdown,
    targetStationId: input.targetStationId,
    targetStationName: input.targetStationName,
    createdAtMs: input.createdAtMs,
    status: input.status,
    taskFilePath: input.taskFilePath,
    detail: input.detail,
  }
}

export function buildTaskDispatchCommand(taskId: string, taskFilePath: string): string {
  const escaped = taskFilePath.replace(/'/g, `'\\''`)
  return `echo '[gt-task] assigned ${taskId} from ${escaped}'`
}

export function pushTaskDispatchHistory(
  previous: TaskDispatchRecord[],
  nextRecord: TaskDispatchRecord,
  limit = 40,
): TaskDispatchRecord[] {
  return [nextRecord, ...previous].slice(0, limit)
}

export function replaceTaskDispatchRecord(
  previous: TaskDispatchRecord[],
  taskId: string,
  patch: Partial<TaskDispatchRecord>,
): TaskDispatchRecord[] {
  return previous.map((record) =>
    record.taskId === taskId ? { ...record, ...patch } : record,
  )
}

export function buildMarkdownSnippet(snippet: TaskMarkdownSnippet): string {
  if (snippet === 'heading') {
    return '\n## 子任务\n- [ ] '
  }
  if (snippet === 'code') {
    return '\n```bash\n# command\n```\n'
  }
  return '\n- [ ] 待办-1\n- [ ] 待办-2\n'
}

export function buildTaskCenterStorageKey(workspaceId: string): string {
  return `${TASK_CENTER_LOCAL_STORAGE_PREFIX}:${workspaceId}`
}

export function buildTaskCenterDraftFilePath(): string {
  return TASK_CENTER_DRAFT_FILE_REL
}

export function normalizeTaskQuickDispatchOpacity(
  value: unknown,
  fallback = DEFAULT_TASK_QUICK_DISPATCH_OPACITY,
): number {
  if (typeof value !== 'number' || Number.isNaN(value)) {
    return fallback
  }
  // Allow fully transparent (0) through fully opaque (1).
  return Math.max(0, Math.min(1, Math.round(value * 100) / 100))
}

function isTaskDispatchRecord(value: unknown): value is TaskDispatchRecord {
  if (!value || typeof value !== 'object') {
    return false
  }
  const record = value as Record<string, unknown>
  return (
    typeof record.taskId === 'string' &&
    typeof record.title === 'string' &&
    typeof record.targetStationId === 'string' &&
    typeof record.targetStationName === 'string' &&
    typeof record.createdAtMs === 'number' &&
    typeof record.status === 'string'
  )
}

export function buildTaskCenterWorkspaceSnapshot(input: {
  updatedAtMs: number
  draft: TaskDraftState
  dispatchHistory: TaskDispatchRecord[]
}): TaskCenterWorkspaceSnapshot {
  return {
    version: 2,
    updatedAtMs: input.updatedAtMs,
    draft: {
      markdown: input.draft.markdown,
      previewMarkdown: input.draft.previewMarkdown,
      targetStationIds: dedupeStationIds(input.draft.targetStationIds),
    },
    dispatchHistory: [...input.dispatchHistory],
  }
}

export function serializeTaskCenterWorkspaceSnapshot(
  snapshot: TaskCenterWorkspaceSnapshot,
): string {
  return JSON.stringify(snapshot, null, 2)
}

export function parseTaskCenterWorkspaceSnapshot(
  raw: string,
): TaskCenterWorkspaceSnapshot | null {
  try {
    const parsed = JSON.parse(raw) as unknown
    if (!parsed || typeof parsed !== 'object') {
      return null
    }
    const record = parsed as Record<string, unknown>
    const draft = record.draft as Record<string, unknown> | undefined
    if (!draft || typeof draft !== 'object') {
      return null
    }

    const dispatchHistoryRaw = Array.isArray(record.dispatchHistory)
      ? record.dispatchHistory.filter((item): item is TaskDispatchRecord =>
          isTaskDispatchRecord(item),
        )
      : []

    const targetStationIdsRaw =
      Array.isArray(draft.targetStationIds) &&
      draft.targetStationIds.every((id) => typeof id === 'string')
        ? (draft.targetStationIds as string[])
        : typeof draft.targetStationId === 'string'
          ? [draft.targetStationId]
          : []

    const markdown =
      typeof draft.markdown === 'string'
        ? draft.markdown
        : typeof draft.title === 'string'
          ? `# ${draft.title.trim()}\n\n`
          : ''

    const previewMarkdown = typeof draft.previewMarkdown === 'string' ? draft.previewMarkdown : ''

    const dispatchHistory = dispatchHistoryRaw.map((item) => ({
      ...item,
      batchId: typeof item.batchId === 'string' ? item.batchId : item.taskId,
      markdown: typeof item.markdown === 'string' ? item.markdown : '',
      taskFilePath: typeof item.taskFilePath === 'string' ? item.taskFilePath : null,
    }))

    return {
      version: 2,
      updatedAtMs:
        typeof record.updatedAtMs === 'number' ? record.updatedAtMs : Date.now(),
      draft: {
        markdown,
        previewMarkdown,
        targetStationIds: dedupeStationIds(targetStationIdsRaw),
      },
      dispatchHistory: dispatchHistory.slice(0, 40),
    }
  } catch {
    return null
  }
}
