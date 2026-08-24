use std::collections::HashSet;

use crate::{
    AgentCapabilitySnapshot, AgentLink, AgentPolicy, AgentProfile, AgentScope, AgentState,
};
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

/// Backed by `agent_capability_snapshots` (docs/cw/08_MCP_Hook_Skill掛載設計.md
/// §2.1): same immutable-snapshot-append pattern as `AgentPolicyRepository` —
/// every save appends a new row and repoints `agents.capability_snapshot_id`.
pub trait AgentCapabilityRepository: Send + Sync {
    /// Returns the new snapshot id. Implementations must reject (not
    /// silently accept) a snapshot that fails
    /// `AgentCapabilitySnapshot::validate_for_tool` for this agent's stored
    /// `tool` — see that method's doc comment for why.
    fn save_agent_capability(
        &self,
        workspace_id: &str,
        agent_id: &str,
        capability: &AgentCapabilitySnapshot,
    ) -> AgentResult<String>;

    /// Returns `AgentCapabilitySnapshot::default()` (nothing mounted) when
    /// the agent has no snapshot yet.
    fn get_agent_capability(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> AgentResult<AgentCapabilitySnapshot>;
}

/// One row of `agent_capability_audit_logs` — what `agent_capability_save`
/// (Tauri command layer) writes for every hook rule actually included in a
/// saved snapshot, per docs/cw/08_MCP_Hook_Skill掛載設計.md §3's requirement
/// that "每條 hook 的 apply 動作都要寫進 audit_repository".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookAuditEntry {
    pub id: String,
    pub workspace_id: String,
    pub agent_id: String,
    pub hook_hash: String,
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
    pub confirmed_by: String,
    pub created_at_ms: i64,
}

/// Backed by `agent_hook_confirmations` / `agent_capability_audit_logs`
/// (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.5 決策3, §3) — the version-lock
/// gate and audit trail for hook rules specifically. MCP servers and skills
/// have no equivalent: they're not user-authored shell commands, so they
/// don't carry the same "silently executes something the user never
/// actually reviewed" risk §3 calls out.
pub trait AgentCapabilityAuditRepository: Send + Sync {
    /// Every hook content hash (`HookCapability::content_hash`) already
    /// confirmed for this `(workspace_id, agent_id)`. A hook not in this
    /// set has never been shown to the user in the preview UI and confirmed
    /// — `agent_capability_save`'s caller must reject saving it.
    fn confirmed_hook_hashes(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> AgentResult<HashSet<String>>;

    /// Idempotent: confirming an already-confirmed hash again is a no-op,
    /// not a duplicate row or an error — re-confirming the same content
    /// shouldn't be possible to get wrong.
    fn confirm_hook_hashes(
        &self,
        workspace_id: &str,
        agent_id: &str,
        hook_hashes: &[String],
        confirmed_by: &str,
    ) -> AgentResult<()>;

    /// Appends one row per entry. Append-only — never updates or deletes an
    /// existing audit row.
    fn record_hook_audit(&self, entries: &[HookAuditEntry]) -> AgentResult<()>;

    /// Newest first. Not currently surfaced in the UI (no history view in
    /// this phase), but kept queryable — an audit trail nobody can read
    /// isn't meaningfully an audit trail.
    fn list_hook_audit_logs(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> AgentResult<Vec<HookAuditEntry>>;
}

/// Backed by `agent_links` (docs/cw/04_客製化設計.md §1) and `agents.layout_x`/
/// `agents.layout_y`. `AgentLinkKind::Derived` rows are written from
/// local_bridge's dispatch/publish handlers; `AgentLinkKind::Authored` rows
/// are hand-drawn by the user on agent-canvas (P4.5) and are also what
/// local_bridge's edge-scoped `gto send` authorization checks against (see
/// `has_authored_edge`).
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

    /// Idempotent: drawing the same edge twice is a no-op, not a second row
    /// (the `agent_links` unique index on (workspace, from, to, kind) already
    /// enforces this at the storage layer).
    fn create_authored_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<()>;

    /// Returns whether a row existed and was removed.
    fn delete_authored_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<bool>;

    /// Clears one direction's "last interacted at" record — unlike
    /// `delete_authored_link`, this is direction-specific (an `a -> b`
    /// derived row and a `b -> a` derived row are two independent facts, each
    /// meaning "this agent actually dispatched to that one," not one shared
    /// undirected relationship). Returns whether a row existed and was
    /// removed. A future `gto send` between the same pair simply re-records
    /// it — this only clears the existing visualization, not the ability to
    /// re-derive it.
    fn delete_derived_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<bool>;

    /// Direction-agnostic: an authored edge drawn `a -> b` also authorizes
    /// `b -> a`, matching the canvas UI's "these two may talk" semantics
    /// (docs/cw/04_客製化設計.md §1) rather than a one-way permission.
    fn has_authored_edge(
        &self,
        workspace_id: &str,
        agent_a: &str,
        agent_b: &str,
    ) -> AgentResult<bool>;

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

    /// Sets `agents.color` directly — a node border color preset on
    /// agent-canvas (docs/cw/04_客製化設計.md §1, P4.6), colocated with
    /// `set_agent_layout` since both are canvas-cosmetic `agents` columns,
    /// not general agent CRUD. `None` resets to the default (gray).
    fn set_agent_color(
        &self,
        workspace_id: &str,
        agent_id: &str,
        color: Option<String>,
    ) -> AgentResult<()>;

    /// Sets an authored link's display color — direction-agnostic (matches
    /// `from`/`to` OR either direction), same as `delete_authored_link`,
    /// since an authored edge is one undirected row regardless of which
    /// direction it was originally drawn.
    fn set_link_color(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
        color: Option<String>,
    ) -> AgentResult<()>;

    /// Sets an authored link's bidirectional (double-arrowhead) vs
    /// unidirectional (single-arrowhead) display flag — direction-agnostic,
    /// same rationale as `set_link_color`.
    fn set_link_bidirectional(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
        bidirectional: bool,
    ) -> AgentResult<()>;
}
