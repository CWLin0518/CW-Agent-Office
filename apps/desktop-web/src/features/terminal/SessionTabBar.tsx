import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'
import { buildSessionTabDescriptors } from './session-tab-model'
import './SessionTabBar.scss'

interface SessionTabBarProps {
  locale: Locale
  sessionTabs: string[]
  activeSessionId: string | null | undefined
  onSwitchTab: (sessionId: string) => void
  onCloseTab: (sessionId: string) => void
  onAddTab: () => void
}

export function SessionTabBar({
  locale,
  sessionTabs,
  activeSessionId,
  onSwitchTab,
  onCloseTab,
  onAddTab,
}: SessionTabBarProps) {
  const tabs = buildSessionTabDescriptors(sessionTabs, activeSessionId)
  if (tabs.length === 0) {
    return null
  }

  return (
    <div className="station-terminal-tab-bar" role="tablist">
      {tabs.map((tab) => (
        <div
          key={tab.sessionId}
          className={`station-terminal-tab${tab.isActive ? ' active' : ''}`}
          onClick={() => onSwitchTab(tab.sessionId)}
          onAuxClick={(event) => {
            if (event.button === 1 && tabs.length > 1) {
              event.preventDefault()
              onCloseTab(tab.sessionId)
            }
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault()
              onSwitchTab(tab.sessionId)
            } else if ((event.key === 'Delete' || event.key === 'Backspace') && tabs.length > 1) {
              // The sole remaining tab has no visible close affordance (see the
              // close-button gate below) — closing the last terminal session goes
              // through the dedicated force-close button's confirm dialog instead.
              event.preventDefault()
              onCloseTab(tab.sessionId)
            }
          }}
          role="tab"
          tabIndex={tab.isActive ? 0 : -1}
          aria-selected={tab.isActive}
          title={t(locale, '会话 {label}', 'Session {label}', { label: tab.label })}
        >
          <span className="station-terminal-tab-label">{tab.label}</span>
          {tabs.length > 1 ? (
            <button
              type="button"
              className="station-terminal-tab-close"
              onClick={(event) => {
                event.stopPropagation()
                onCloseTab(tab.sessionId)
              }}
              title={t(locale, '关闭会话', 'Close session')}
              aria-label={t(locale, '关闭会话', 'Close session')}
            >
              <AppIcon name="close" />
            </button>
          ) : null}
        </div>
      ))}
      <button
        type="button"
        className="station-terminal-tab-add"
        onClick={onAddTab}
        title={t(locale, '开启新会话', 'New session')}
        aria-label={t(locale, '开启新会话', 'New session')}
      >
        <AppIcon name="plus" />
      </button>
    </div>
  )
}
