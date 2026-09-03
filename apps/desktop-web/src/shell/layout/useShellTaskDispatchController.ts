import { useCallback, useEffect, useMemo, useState } from 'react'
import type { MutableRefObject } from 'react'
import {
  areTaskTargetsEqual,
  buildTaskCenterDraftFilePath,
  createInitialTaskDraft,
  parseQuickDispatchRailPrefs,
  QUICK_DISPATCH_RAIL_STORAGE_KEY,
  resolveTaskTargetIdsForDispatch,
  resolveValidTaskTargets,
  useTaskDispatchActions,
  useTaskCenterDraftPersistence,
  type TaskCenterNotice,
  type TaskDispatchRecord,
  type TaskDraftState,
  type TaskSendPreviewTarget,
} from '@features/task-center'
import type { AgentStation } from '@features/workspace-hub'
import type { Locale } from '../i18n/ui-locale'
import type { StationTerminalRuntime } from './ShellRoot.shared'
import {
  TASK_DISPATCH_HISTORY_LIMIT,
  TASK_DRAFT_PERSIST_DEBOUNCE_MS,
  describeError,
  normalizeStationToolKind,
} from './ShellRoot.shared'
import { desktopApi } from '../integration/desktop-api'
import { resolveStationRuntimeRegistrationCleanup } from '@features/terminal/runtime'

interface UseShellTaskDispatchControllerInput {
  initialStations: AgentStation[]
  activeWorkspaceId: string | null
  activeStationId: string
  locale: Locale
  stationsRef: MutableRefObject<AgentStation[]>
  stationTerminalsRef: MutableRefObject<Record<string, StationTerminalRuntime>>
  activeWorkspaceIdRef: MutableRefObject<string | null>
  stationSubmitSequenceRef: MutableRefObject<Record<string, string>>
  tauriRuntime: boolean
  // Terminal callbacks passed from the terminal controller
  ensureStationTerminalSession: (stationId: string) => Promise<string | null>
  submitStationTerminal: (stationId: string) => Promise<boolean>
  reconcileStationRuntimeRegistration: (input: {
    workspaceId: string
    stationId: string
    expectedSessionId: string | null
  }) => Promise<void>
}

export interface ShellTaskDispatchController {
  taskDraft: TaskDraftState
  taskDispatchHistory: TaskDispatchRecord[]
  taskSending: boolean
  taskRetryingTaskId: string | null
  taskDraftSavedAtMs: number | null
  taskNotice: TaskCenterNotice | null
  taskSendPreviewTargets: TaskSendPreviewTarget[]
  taskSuppressOutputCollection: boolean
  setTaskSuppressOutputCollection: React.Dispatch<React.SetStateAction<boolean>>
  setTaskDraft: React.Dispatch<React.SetStateAction<TaskDraftState>>
  setTaskDispatchHistory: React.Dispatch<React.SetStateAction<TaskDispatchRecord[]>>
  setTaskSending: React.Dispatch<React.SetStateAction<boolean>>
  setTaskRetryingTaskId: React.Dispatch<React.SetStateAction<string | null>>
  setTaskDraftSavedAtMs: React.Dispatch<React.SetStateAction<number | null>>
  setTaskNotice: React.Dispatch<React.SetStateAction<TaskCenterNotice | null>>
  updateTaskDraft: (patch: Partial<TaskDraftState>) => void
  insertTaskSnippet: (snippet: import('@features/task-center').TaskMarkdownSnippet) => void
  dispatchTaskToAgent: () => Promise<void>
  retryTaskDispatch: (taskId: string) => Promise<void>
  handleTaskSend: () => void
  handleRetryDispatchTask: (taskId: string) => Promise<void>
}

export function useShellTaskDispatchController({
  initialStations,
  activeWorkspaceId,
  activeStationId,
  locale,
  stationsRef,
  stationTerminalsRef,
  activeWorkspaceIdRef,
  stationSubmitSequenceRef,
  tauriRuntime,
  ensureStationTerminalSession,
  submitStationTerminal,
  reconcileStationRuntimeRegistration,
}: UseShellTaskDispatchControllerInput): ShellTaskDispatchController {
  // --- State ---
  const [taskDraft, setTaskDraft] = useState<TaskDraftState>(() =>
    createInitialTaskDraft(initialStations, initialStations[0]?.id ?? ''),
  )
  const [taskDispatchHistory, setTaskDispatchHistory] = useState<TaskDispatchRecord[]>([])
  const [taskSending, setTaskSending] = useState(false)
  const [taskRetryingTaskId, setTaskRetryingTaskId] = useState<string | null>(null)
  const [taskDraftSavedAtMs, setTaskDraftSavedAtMs] = useState<number | null>(null)
  const [taskNotice, setTaskNotice] = useState<TaskCenterNotice | null>(null)
  const [taskSendPreviewTargets, setTaskSendPreviewTargets] = useState<TaskSendPreviewTarget[]>([])
  // Per-send override for the "will also be sent" preview card — resets each
  // session (not persisted with the draft) since it's a one-off choice, not a
  // lasting preference. The target's own outputCollectionEnabled setting is
  // untouched; this only affects the next dispatch.
  const [taskSuppressOutputCollection, setTaskSuppressOutputCollection] = useState(false)

  // --- Derived ---
  const taskCenterDraftFilePath = useMemo(() => buildTaskCenterDraftFilePath(), [])

  // --- Effects ---

  // Keep task draft target station IDs in sync when stations change
  useEffect(() => {
    const stations = stationsRef.current
    const nextTargetIds = resolveValidTaskTargets(stations, taskDraft.targetStationIds)
    if (areTaskTargetsEqual(nextTargetIds, taskDraft.targetStationIds)) {
      return
    }
    setTaskDraft((prev) => ({
      ...prev,
      targetStationIds: nextTargetIds,
    }))
  }, [stationsRef, taskDraft.targetStationIds])

  // Follow-active: when the global active station changes, lock receivers to it.
  // Lives here (not only in the overlay) so activation always wins even if the
  // floating composer effect is skipped or a draft snapshot races it.
  useEffect(() => {
    let followActiveAgent = true
    try {
      if (typeof window !== 'undefined') {
        followActiveAgent = parseQuickDispatchRailPrefs(
          window.localStorage.getItem(QUICK_DISPATCH_RAIL_STORAGE_KEY),
        ).followActiveAgent
      }
    } catch {
      followActiveAgent = true
    }
    if (!followActiveAgent) {
      return
    }

    setTaskDraft((prev) => {
      const nextTargetIds = resolveTaskTargetIdsForDispatch({
        stations: stationsRef.current,
        activeStationId,
        currentTargetIds: prev.targetStationIds,
        followActiveAgent: true,
      })
      if (areTaskTargetsEqual(nextTargetIds, prev.targetStationIds)) {
        return prev
      }
      return {
        ...prev,
        targetStationIds: nextTargetIds,
      }
    })
  }, [activeStationId, setTaskDraft, stationsRef])

  // Live preview of what dispatch would silently append after the sender's
  // own markdown (output-collection instructions, etc. — see
  // crates/gt-task's `dispatch_appended_sections`), so Task Brief can show it
  // before send instead of it only showing up in the terminal afterward.
  // Depends only on the target selection, never on `taskDraft.markdown` —
  // the appended text is derived purely from each target's own settings.
  useEffect(() => {
    let cancelled = false
    const targets = taskDraft.targetStationIds
    // Route the "nothing to fetch" case through the same promise chain as the
    // real fetch, so every `setTaskSendPreviewTargets` call happens inside a
    // `.then`/`.catch` callback rather than synchronously in the effect body.
    const fetchPreview =
      tauriRuntime && activeWorkspaceId && targets.length > 0
        ? desktopApi.taskDispatchPreview({ workspaceId: activeWorkspaceId, targets })
        : Promise.resolve<{ targets: TaskSendPreviewTarget[] }>({ targets: [] })
    void fetchPreview
      .then((response) => {
        if (!cancelled) {
          setTaskSendPreviewTargets(response.targets)
        }
      })
      .catch(() => {
        if (!cancelled) {
          setTaskSendPreviewTargets([])
        }
      })
    return () => {
      cancelled = true
    }
  }, [tauriRuntime, activeWorkspaceId, taskDraft.targetStationIds])

  // --- Callbacks ---

  const readTaskCenterSnapshotFile = useCallback(
    async (input: { workspaceId: string; taskCenterDraftFilePath: string }) => {
      if (!tauriRuntime) {
        return null
      }
      try {
        const file = await desktopApi.fsReadFile(input.workspaceId, input.taskCenterDraftFilePath)
        if (!file.previewable) {
          return null
        }
        return file.content
      } catch {
        return null
      }
    },
    [tauriRuntime],
  )

  const writeTaskCenterSnapshotFile = useCallback(
    async (input: {
      workspaceId: string
      taskCenterDraftFilePath: string
      serializedSnapshot: string
    }) => {
      if (!tauriRuntime) {
        return
      }
      await desktopApi.fsWriteFile(
        input.workspaceId,
        input.taskCenterDraftFilePath,
        input.serializedSnapshot,
      )
    },
    [tauriRuntime],
  )

  // --- Hooks ---

  useTaskCenterDraftPersistence({
    activeWorkspaceId,
    taskCenterDraftFilePath,
    stationsRef,
    activeStationId,
    taskDraft,
    taskDispatchHistory,
    taskDispatchHistoryLimit: TASK_DISPATCH_HISTORY_LIMIT,
    persistDebounceMs: TASK_DRAFT_PERSIST_DEBOUNCE_MS,
    setTaskDraft,
    setTaskDispatchHistory,
    setTaskSending,
    setTaskRetryingTaskId,
    setTaskDraftSavedAtMs,
    setTaskNotice,
    onReadTaskSnapshotFile: readTaskCenterSnapshotFile,
    onWriteTaskSnapshotFile: writeTaskCenterSnapshotFile,
  })

  const ensureTaskTargetRuntime = useCallback(
    async (input: { workspaceId: string; targetStationId: string }) => {
      if (!tauriRuntime) {
        return
      }
      const station = stationsRef.current.find((item) => item.id === input.targetStationId)
      if (!station) {
        return
      }
      const sessionId = await ensureStationTerminalSession(station.id)
      if (!sessionId) {
        return
      }
      const currentStation = stationsRef.current.find((item) => item.id === input.targetStationId)
      const runtimeRegistrationCleanup = resolveStationRuntimeRegistrationCleanup(
        input.workspaceId,
        activeWorkspaceIdRef.current,
        Boolean(currentStation),
        sessionId,
        stationTerminalsRef.current[input.targetStationId],
      )
      if (runtimeRegistrationCleanup?.action === 'unregister') {
        void desktopApi.agentRuntimeUnregister(input.workspaceId, input.targetStationId).catch(() => {
          // Runtime sync effect will retry from current station ownership.
        })
        return
      }
      const registrationSessionId = runtimeRegistrationCleanup?.sessionId ?? sessionId
      const registrationResolvedCwd =
        runtimeRegistrationCleanup?.resolvedCwd ??
        stationTerminalsRef.current[input.targetStationId]?.resolvedCwd ??
        null
      const registrationStation = currentStation ?? station
      await desktopApi.agentRuntimeRegister({
        workspaceId: input.workspaceId,
        agentId: input.targetStationId,
        stationId: input.targetStationId,
        sessionId: registrationSessionId,
        toolKind: normalizeStationToolKind(registrationStation.tool),
        resolvedCwd: registrationResolvedCwd,
        submitSequence: stationSubmitSequenceRef.current[input.targetStationId] ?? null,
        online: true,
      })
      await reconcileStationRuntimeRegistration({
        workspaceId: input.workspaceId,
        stationId: input.targetStationId,
        expectedSessionId: registrationSessionId,
      })
    },
    [
      tauriRuntime,
      stationsRef,
      ensureStationTerminalSession,
      stationTerminalsRef,
      activeWorkspaceIdRef,
      stationSubmitSequenceRef,
      reconcileStationRuntimeRegistration,
    ],
  )

  const dispatchTaskBatch = useCallback(
    async (input: {
      workspaceId: string
      title: string
      markdown: string
      targetStationIds: string[]
    }) => {
      const response = await desktopApi.taskDispatchBatch({
        workspaceId: input.workspaceId,
        sender: { type: 'human', agentId: null },
        targets: input.targetStationIds,
        title: input.title,
        markdown: input.markdown,
        attachments: [],
        suppressOutputCollectionInstructions: taskSuppressOutputCollection,
      })
      const postSubmitResults = await Promise.all(
        response.results.map(async (result) => {
          if (result.status !== 'sent') {
            return result
          }
          const submitted = await submitStationTerminal(result.targetAgentId)
          if (submitted) {
            return result
          }
          return {
            ...result,
            status: 'failed' as const,
            detail: 'XTERM_SUBMIT_FAILED',
          }
        }),
      )
      return {
        ...response,
        results: postSubmitResults,
      }
    },
    [submitStationTerminal, taskSuppressOutputCollection],
  )

  const {
    updateTaskDraft,
    insertTaskSnippet,
    dispatchTaskToAgent,
    retryTaskDispatch,
  } = useTaskDispatchActions({
    locale,
    activeWorkspaceId,
    stationsRef,
    taskDraft,
    taskDispatchHistory,
    taskSending,
    taskRetryingTaskId,
    setTaskDraft,
    setTaskDispatchHistory,
    setTaskSending,
    setTaskRetryingTaskId,
    setTaskNotice,
    onEnsureTaskTargetRuntime: ensureTaskTargetRuntime,
    onDispatchTaskBatch: dispatchTaskBatch,
    describeError,
    taskDispatchHistoryLimit: TASK_DISPATCH_HISTORY_LIMIT,
  })

  const handleTaskSend = useCallback(() => {
    void dispatchTaskToAgent()
  }, [dispatchTaskToAgent])

  const handleRetryDispatchTask = useCallback(
    async (taskId: string) => {
      await retryTaskDispatch(taskId)
    },
    [retryTaskDispatch],
  )

  return {
    taskDraft,
    taskDispatchHistory,
    taskSending,
    taskRetryingTaskId,
    taskDraftSavedAtMs,
    taskNotice,
    taskSendPreviewTargets,
    taskSuppressOutputCollection,
    setTaskSuppressOutputCollection,
    setTaskDraft,
    setTaskDispatchHistory,
    setTaskSending,
    setTaskRetryingTaskId,
    setTaskDraftSavedAtMs,
    setTaskNotice,
    updateTaskDraft,
    insertTaskSnippet,
    dispatchTaskToAgent,
    retryTaskDispatch,
    handleTaskSend,
    handleRetryDispatchTask,
  }
}
