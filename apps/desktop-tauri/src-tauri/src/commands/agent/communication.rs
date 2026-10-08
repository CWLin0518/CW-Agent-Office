use gt_agent::{
    authorize_agent_communication, AgentCommunicationError, AgentLinkRepository,
    AgentPolicyRepository, AgentRepository,
};
use gt_task::DispatchSenderType;
use tauri::AppHandle;

use super::resolve_agent_repository;

pub(crate) fn ensure_agent_allowed_to_send(
    app: &AppHandle,
    workspace_id: &str,
    sender_type: &DispatchSenderType,
    sender_agent_id: Option<&str>,
    target_agent_ids: &[String],
) -> Result<(), AgentCommunicationError> {
    if *sender_type == DispatchSenderType::Human {
        return Ok(());
    }
    let sender = sender_agent_id
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| AgentCommunicationError {
            code: "AGENT_COMMUNICATION_INVALID_SENDER",
            message: "agent sender requires an agentId".into(),
        })?;
    let internal_error = |message: String| AgentCommunicationError {
        code: "LOCAL_BRIDGE_INTERNAL",
        message,
    };
    let repo = resolve_agent_repository(app).map_err(internal_error)?;
    let agents = repo
        .list_agents(workspace_id)
        .map_err(|e| internal_error(e.to_string()))?;
    if !agents.iter().any(|agent| agent.id == sender) {
        return Err(AgentCommunicationError {
            code: "AGENT_COMMUNICATION_INVALID_SENDER",
            message: format!("agent '{sender}' does not belong to workspace '{workspace_id}'"),
        });
    }
    let policy = repo
        .get_agent_policy(workspace_id, sender)
        .map_err(|e| internal_error(e.to_string()))?;
    let broadcast_ids = agents
        .into_iter()
        .filter(|agent| agent.communicate_with_all)
        .map(|agent| agent.id)
        .collect::<Vec<_>>();
    let result = authorize_agent_communication(
        sender,
        target_agent_ids,
        policy.agent.allow_gto_send,
        &broadcast_ids,
        |from, to| repo.has_authored_edge(workspace_id, from, to),
    );
    if let Err(error) = &result {
        tracing::warn!(
            workspace_id,
            sender_agent_id = sender,
            code = error.code,
            "agent communication denied"
        );
    }
    result
}
