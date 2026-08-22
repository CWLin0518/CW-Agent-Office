export interface SessionTabDescriptor {
  sessionId: string
  label: string
  isActive: boolean
}

/**
 * Builds display tabs for a station's live sessions, in creation order.
 * Labels are ordinal ("Session 1", "Session 2", ...) since session ids are
 * opaque backend identifiers with no user-meaningful content.
 */
export function buildSessionTabDescriptors(
  sessionTabs: string[],
  activeSessionId: string | null | undefined,
): SessionTabDescriptor[] {
  return sessionTabs.map((sessionId, index) => ({
    sessionId,
    label: `${index + 1}`,
    isActive: sessionId === activeSessionId,
  }))
}
