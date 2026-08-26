const DERIVED_EDGES_VISIBLE_STORAGE_PREFIX = 'agent-canvas.derivedEdgesVisible'

function storageKey(workspaceId: string): string {
  return `${DERIVED_EDGES_VISIBLE_STORAGE_PREFIX}:${workspaceId}`
}

/**
 * Derived-wire visibility is workspace-scoped presentation state. Keeping it
 * in local storage prevents the pane's conditional mount/unmount during
 * navigation from silently resetting an explicit user choice.
 */
export function loadDerivedEdgesVisible(workspaceId: string | null): boolean {
  if (!workspaceId || typeof window === 'undefined') return true
  try {
    const stored = window.localStorage.getItem(storageKey(workspaceId))
    return stored === null ? true : stored === 'true'
  } catch {
    return true
  }
}

export function saveDerivedEdgesVisible(workspaceId: string | null, visible: boolean): void {
  if (!workspaceId || typeof window === 'undefined') return
  try {
    window.localStorage.setItem(storageKey(workspaceId), String(visible))
  } catch {
    // A storage failure only limits persistence; the current pane state still
    // reflects the user's choice.
  }
}
