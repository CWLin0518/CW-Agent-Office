use crate::{AgentLink, AgentPolicy, AgentProfile, AgentScope, AgentState};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("storage error: {message}")]
    Storage { message: String },
}

pub type AgentResult<T> = Result<T, AgentError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentInput {
    pub workspace_id: String,
    pub agent_id: Option<String>,
    pub name: String,
    pub tool: String,
    pub workdir: Option<String>,
    pub custom_workdir: bool,
    #[serde(default)]
    pub scope: AgentScope,
    pub employee_no: Option<String>,
    pub state: AgentState,
    pub launch_command: Option<String>,
    pub order_index: Option<i32>,
    #[serde(default)]
    pub parent_agent_id: Option<String>,
    #[serde(default)]
    pub external_template_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentInput {
    pub workspace_id: String,
    pub agent_id: String,
    pub name: String,
    pub tool: String,
    pub workdir: Option<String>,
    pub custom_workdir: bool,
    pub employee_no: Option<String>,
    pub state: AgentState,
    pub launch_command: Option<String>,
}

pub trait AgentRepository: Send + Sync {
    fn ensure_schema(&self) -> AgentResult<()>;
    fn reset_workspace_state(&self, workspace_id: &str) -> AgentResult<()>;
    fn list_agents(&self, workspace_id: &str) -> AgentResult<Vec<AgentProfile>>;
    fn create_agent(&self, input: CreateAgentInput) -> AgentResult<AgentProfile>;
    fn update_agent(&self, input: UpdateAgentInput) -> AgentResult<AgentProfile>;
    fn delete_agent(&self, workspace_id: &str, agent_id: &str) -> AgentResult<bool>;
    fn reorder_agents(&self, workspace_id: &str, ordered_ids: Vec<String>) -> AgentResult<()>;
    /// Sets `agents.git_tracked` directly, independent of `update_agent`'s
    /// full-row overwrite, so toggling this flag never clobbers unrelated
    /// fields concurrent edits may have changed.
    fn set_git_tracked(
        &self,
        workspace_id: &str,
        agent_id: &str,
        tracked: bool,
    ) -> AgentResult<AgentProfile>;
}

/// Backed by `agent_policy_snapshots` (docs/cw/04_客製化設計.md §3): every save
/// appends a new immutable snapshot row and repoints `agents.policy_snapshot_id`
/// at it, rather than overwriting a row in place, so past policy states stay
/// auditable.
pub trait AgentPolicyRepository: Send + Sync {
    /// Returns the new snapshot id.
    fn save_agent_policy(
        &self,
        workspace_id: &str,
        agent_id: &str,
        policy: &AgentPolicy,
    ) -> AgentResult<String>;

    /// Returns `AgentPolicy::default()` (fully permissive) when the agent has
    /// no snapshot yet, so agents created before this feature existed behave
    /// unchanged.
    fn get_agent_policy(&self, workspace_id: &str, agent_id: &str) -> AgentResult<AgentPolicy>;
}

/// Backed by `agent_links` (docs/cw/04_客製化設計.md §1) and `agents.layout_x`/
/// `agents.layout_y`. P4 only produces `AgentLinkKind::Derived` rows (written
/// from local_bridge's dispatch/publish handlers); hand-drawn `Authored` links
/// are P4.5.
pub trait AgentLinkRepository: Send + Sync {
    /// Upserts the "last interacted at" row for this ordered (from, to) pair —
    /// one row per pair, not one row per dispatch, so a chatty pair of agents
    /// doesn't grow the table unbounded.
    fn record_derived_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<()>;

    fn list_links(&self, workspace_id: &str) -> AgentResult<Vec<AgentLink>>;

    /// Sets `agents.layout_x`/`agents.layout_y` directly, independent of
    /// `update_agent`'s full-row overwrite — same rationale as
    /// `AgentRepository::set_git_tracked`.
    fn set_agent_layout(
        &self,
        workspace_id: &str,
        agent_id: &str,
        x: f64,
        y: f64,
    ) -> AgentResult<()>;
}
