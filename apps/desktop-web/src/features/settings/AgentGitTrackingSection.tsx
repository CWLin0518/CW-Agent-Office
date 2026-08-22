import { useCallback, useEffect, useState } from 'react'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { desktopApi, type AgentProfile } from '@shell/integration/desktop-api'
import { isWorkspaceRootWorkdir } from '@features/workspace/station-workdir-model'
import './AgentGitTrackingSection.scss'

interface AgentGitTrackingSectionProps {
  locale: Locale
  workspaceId: string | null
}

export function AgentGitTrackingSection({ locale, workspaceId }: AgentGitTrackingSectionProps) {
  const [agents, setAgents] = useState<AgentProfile[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [pendingAgentIds, setPendingAgentIds] = useState<Set<string>>(new Set())

  const reload = useCallback(async () => {
    if (!workspaceId) {
      setAgents([])
      return
    }
    setLoading(true)
    setError(null)
    try {
      const response = await desktopApi.agentList(workspaceId)
      setAgents(response.agents)
    } catch {
      setError(t(locale, 'settingsModal.agentGitTracking.loadError'))
    } finally {
      setLoading(false)
    }
  }, [locale, workspaceId])

  useEffect(() => {
    void reload()
  }, [reload])

  const handleToggle = async (agent: AgentProfile, nextTracked: boolean) => {
    if (!workspaceId || pendingAgentIds.has(agent.id)) {
      return
    }
    setError(null)
    setPendingAgentIds((previous) => new Set(previous).add(agent.id))
    setAgents((previous) =>
      previous.map((item) => (item.id === agent.id ? { ...item, gitTracked: nextTracked } : item)),
    )
    try {
      const response = await desktopApi.agentGitTrackingSet({
        workspaceId,
        agentId: agent.id,
        gitTracked: nextTracked,
      })
      setAgents((previous) =>
        previous.map((item) => (item.id === agent.id ? response.agent : item)),
      )
    } catch {
      setAgents((previous) =>
        previous.map((item) => (item.id === agent.id ? { ...item, gitTracked: !nextTracked } : item)),
      )
      setError(t(locale, 'settingsModal.agentGitTracking.updateError'))
    } finally {
      setPendingAgentIds((previous) => {
        const next = new Set(previous)
        next.delete(agent.id)
        return next
      })
    }
  }

  return (
    <section className="agent-git-tracking-card" aria-labelledby="agent-git-tracking-title">
      <div className="agent-git-tracking-card__copy">
        <h4 id="agent-git-tracking-title">{t(locale, 'settingsModal.agentGitTracking.title')}</h4>
        <p>{t(locale, 'settingsModal.agentGitTracking.description')}</p>
      </div>

      {!workspaceId ? (
        <p className="agent-git-tracking-card__hint">{t(locale, 'settingsModal.agentGitTracking.noWorkspace')}</p>
      ) : loading ? (
        <p className="agent-git-tracking-card__hint">{t(locale, 'settingsModal.agentGitTracking.loading')}</p>
      ) : agents.length === 0 ? (
        <p className="agent-git-tracking-card__hint">{t(locale, 'settingsModal.agentGitTracking.empty')}</p>
      ) : (
        <div className="agent-git-tracking-table" role="table">
          <div className="agent-git-tracking-table__row agent-git-tracking-table__row--head" role="row">
            <span role="columnheader">{t(locale, 'settingsModal.agentGitTracking.columnAgent')}</span>
            <span role="columnheader">{t(locale, 'settingsModal.agentGitTracking.columnWorkdir')}</span>
            <span role="columnheader">{t(locale, 'settingsModal.agentGitTracking.columnTracked')}</span>
          </div>
          {agents.map((agent) => {
            const isRootWorkdir = isWorkspaceRootWorkdir(agent.workdir)
            const disabled = pendingAgentIds.has(agent.id) || (agent.gitTracked && isRootWorkdir)
            return (
              <div className="agent-git-tracking-table__row" role="row" key={agent.id}>
                <span role="cell" className="agent-git-tracking-table__name">
                  {agent.name}
                </span>
                <span role="cell" className="agent-git-tracking-table__workdir">
                  {isRootWorkdir ? '.' : (agent.workdir ?? '.')}
                </span>
                <span role="cell" className="agent-git-tracking-table__checkbox-cell">
                  <label className="agent-git-tracking-checkbox">
                    <input
                      type="checkbox"
                      checked={agent.gitTracked}
                      disabled={disabled}
                      title={
                        agent.gitTracked && isRootWorkdir
                          ? t(locale, 'settingsModal.agentGitTracking.rootWorkdirHint')
                          : undefined
                      }
                      onChange={(event) => {
                        void handleToggle(agent, event.target.checked)
                      }}
                    />
                  </label>
                </span>
              </div>
            )
          })}
        </div>
      )}

      {error ? <p className="agent-git-tracking-card__error">{error}</p> : null}
    </section>
  )
}
