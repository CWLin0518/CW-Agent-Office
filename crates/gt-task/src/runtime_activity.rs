use crate::{now_ms, purge_agent_messages_locked, TaskService};
use gt_agent::{AgentRuntimeState, AgentRuntimeStatus};

/// Activity is evidence of recent terminal output, not a claim that the model
/// is thinking. Quiet sessions are Idle. This spans the canvas's 8s fallback
/// polling period; channel messages never extend the activity window.
const TERMINAL_ACTIVITY_WINDOW_MS: u64 = 10_000;

impl TaskService {
    pub fn observe_terminal_output(&self, workspace_id: &str, session_id: &str, ts_ms: u64) {
        let Ok(mut guard) = self.state.write() else {
            return;
        };
        let keys = guard
            .runtimes
            .iter()
            .filter(|(_, runtime)| {
                runtime.workspace_id == workspace_id && runtime.session_id == session_id
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            let last = guard.terminal_activity.entry(key).or_default();
            *last = (*last).max(ts_ms);
        }
    }

    /// Old-session exit events cannot unregister a replacement session.
    pub fn observe_terminal_state(&self, workspace_id: &str, session_id: &str, state: &str) {
        if !matches!(state, "exited" | "killed" | "failed") {
            return;
        }
        let Ok(mut guard) = self.state.write() else {
            return;
        };
        let ended = guard
            .runtimes
            .iter()
            .filter(|(_, runtime)| {
                runtime.workspace_id == workspace_id && runtime.session_id == session_id
            })
            .map(|(key, runtime)| (key.clone(), runtime.agent_id.clone()))
            .collect::<Vec<_>>();
        for (key, agent_id) in ended {
            guard.runtimes.remove(&key);
            guard.terminal_activity.remove(&key);
            purge_agent_messages_locked(&mut guard, workspace_id, &agent_id);
            tracing::debug!(
                workspace_id,
                session_id,
                agent_id,
                state,
                "agent terminal ended"
            );
        }
    }

    /// Canvas and directory snapshots share this workspace/session-scoped
    /// terminal projection. Missing registrations are Offline in roster views.
    pub fn agent_runtime_status(&self, workspace_id: &str) -> Vec<AgentRuntimeStatus> {
        self.agent_runtime_status_at(workspace_id, now_ms())
    }

    fn agent_runtime_status_at(&self, workspace_id: &str, now: u64) -> Vec<AgentRuntimeStatus> {
        let Ok(guard) = self.state.read() else {
            return Vec::new();
        };
        guard
            .runtimes
            .iter()
            .filter(|(_, runtime)| runtime.workspace_id == workspace_id)
            .map(|(key, runtime)| AgentRuntimeStatus {
                agent_id: runtime.agent_id.clone(),
                workspace_id: workspace_id.to_string(),
                state: if guard
                    .terminal_activity
                    .get(key)
                    .is_some_and(|last| now.saturating_sub(*last) <= TERMINAL_ACTIVITY_WINDOW_MS)
                {
                    AgentRuntimeState::Active
                } else {
                    AgentRuntimeState::Idle
                },
                updated_at_ms: now as i64,
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "tests/runtime_activity_tests.rs"]
mod tests;
