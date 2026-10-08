import { useCallback, useEffect, useRef, useState } from 'react'

import { desktopApi, type SessionCurrentForTerminal } from '@shell/integration/desktop-api'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'

import './SessionTitleCard.scss'

export interface SessionTitleCardProps {
  locale: Locale
  workspaceId: string
  /** Station terminal session; the card shows the live GT Office session on it. */
  terminalSessionId: string
}

const SESSION_TITLE_MAX_LENGTH = 80

/**
 * Name of the station's current session, shown next to the agent name.
 * Untitled sessions are named from their first task by the backend; the
 * pencil lets the user rename it by hand at any time.
 */
export function SessionTitleCard({ locale, workspaceId, terminalSessionId }: SessionTitleCardProps) {
  const [session, setSession] = useState<SessionCurrentForTerminal | null>(null)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState<string | null>(null)
  const gtoSessionIdRef = useRef<string | null>(null)

  const refresh = useCallback(() => {
    void desktopApi
      .sessionCurrentForTerminal(workspaceId, terminalSessionId)
      .then((current) => {
        gtoSessionIdRef.current = current?.gtoSessionId ?? null
        setSession(current)
      })
      .catch((reason) => {
        console.warn('failed to load current session', reason)
      })
  }, [terminalSessionId, workspaceId])

  useEffect(() => {
    refresh()
    let disposed = false
    let cleanup: (() => void) | null = null
    void desktopApi
      .subscribeSessionUpdated((payload) => {
        if (
          payload.workspaceId === workspaceId &&
          (payload.terminalSessionId === terminalSessionId || payload.gtoSessionId === gtoSessionIdRef.current)
        ) {
          refresh()
        }
      })
      .then((unlisten) => {
        if (disposed) {
          unlisten()
        } else {
          cleanup = unlisten
        }
      })
    return () => {
      disposed = true
      cleanup?.()
    }
  }, [refresh, terminalSessionId, workspaceId])

  if (!session) {
    return null
  }

  const title = session.title?.trim() ?? ''
  const placeholder = t(locale, '等待首次任务命名', 'Named after first task')

  const startEditing = () => {
    setDraft(title)
    setError(null)
    setEditing(true)
  }

  const commit = () => {
    const next = draft.trim()
    setEditing(false)
    if (!next || next === title) {
      return
    }
    const previous = session
    setSession({ ...session, title: next })
    void desktopApi.sessionUpdateTitle(workspaceId, session.gtoSessionId, next).catch((reason) => {
      setSession(previous)
      setError(String(reason))
    })
  }

  if (editing) {
    return (
      <div className="session-title-card is-editing" data-no-drag>
        <input
          className="session-title-card__input"
          value={draft}
          maxLength={SESSION_TITLE_MAX_LENGTH}
          aria-label={t(locale, 'Session 名称', 'Session name')}
          placeholder={placeholder}
          autoFocus
          onChange={(event) => setDraft(event.target.value)}
          onClick={(event) => event.stopPropagation()}
          onBlur={commit}
          onKeyDown={(event) => {
            event.stopPropagation()
            if (event.key === 'Enter') {
              event.preventDefault()
              commit()
            } else if (event.key === 'Escape') {
              event.preventDefault()
              setEditing(false)
            }
          }}
        />
      </div>
    )
  }

  return (
    <div
      className={`session-title-card${title ? '' : ' is-untitled'}`}
      title={error ?? (title || placeholder)}
      data-no-drag
    >
      <span className="session-title-card__label">{title || placeholder}</span>
      <button
        type="button"
        className="session-title-card__edit"
        aria-label={t(locale, '重命名 Session', 'Rename session')}
        title={t(locale, '重命名 Session', 'Rename session')}
        onClick={(event) => {
          event.stopPropagation()
          startEditing()
        }}
      >
        <AppIcon name="pencil" className="vb-icon" aria-hidden="true" />
      </button>
    </div>
  )
}
