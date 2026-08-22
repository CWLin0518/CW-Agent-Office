use std::path::PathBuf;

use gt_abstractions::{AgentPolicyProvider, WorkspaceId};
use gt_agent::{AgentPolicy, AgentPolicyRepository, AgentRepository};
use gt_storage::{SqliteAgentRepository, SqliteStorage};

/// Backs `AgentPolicyProvider` with the real `agent_policy_snapshots` table
/// (docs/cw/04_客製化設計.md §3, P3 Phase A). Lives at the app layer (not in
/// gt-terminal/gt-task/gt-git) because it needs `gt-storage`, which those
/// lower crates deliberately don't depend on — see
/// `gt_abstractions::AgentPolicyProvider`'s doc comment for why this is a
/// trait object rather than a generic parameter.
///
/// `base_dir` is the Tauri app data directory, only resolvable once the
/// `AppHandle` exists (during `.setup()`), which is after `AppState::default()`
/// constructs the long-lived `PtyTerminalProvider`/`TaskService`/`GitService`
/// instances this gets injected into. Each lookup opens its own connection
/// (matching the existing `resolve_agent_repository` convention in
/// commands/agent.rs — SqliteAgentRepository is a cheap path wrapper, not a
/// held-open connection) rather than caching a repository handle.
#[derive(Debug, Clone)]
pub struct SqliteAgentPolicyProvider {
    base_dir: PathBuf,
}

impl SqliteAgentPolicyProvider {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn repository(&self) -> Option<SqliteAgentRepository> {
        let db_path = self.base_dir.join("gtoffice.db");
        let storage = SqliteStorage::new(db_path).ok()?;
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().ok()?;
        Some(repo)
    }
}

impl AgentPolicyProvider for SqliteAgentPolicyProvider {
    fn policy_for(&self, workspace_id: &WorkspaceId, agent_id: &str) -> AgentPolicy {
        let Some(repo) = self.repository() else {
            return AgentPolicy::default();
        };
        repo.get_agent_policy(workspace_id.as_str(), agent_id)
            .unwrap_or_default()
    }
}
