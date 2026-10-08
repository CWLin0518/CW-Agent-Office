use crate::AgentResult;

/// Shared authorization for agent dispatch and channel publication. Human
/// messages are handled by callers before entering this agent-only gate.
pub fn authorize_agent_communication(
    sender_agent_id: &str,
    target_agent_ids: &[String],
    allow_gto_send: bool,
    broadcast_agent_ids: &[String],
    mut has_edge: impl FnMut(&str, &str) -> AgentResult<bool>,
) -> Result<(), AgentCommunicationError> {
    if !allow_gto_send {
        return Err(AgentCommunicationError {
            code: "AGENT_POLICY_GTO_SEND_DENIED",
            message: format!("agent '{sender_agent_id}' policy denies agent communication"),
        });
    }
    for target in target_agent_ids {
        if target == sender_agent_id
            || broadcast_agent_ids
                .iter()
                .any(|id| id == sender_agent_id || id == target)
        {
            continue;
        }
        let connected =
            has_edge(sender_agent_id, target).map_err(|error| AgentCommunicationError {
                code: "LOCAL_BRIDGE_INTERNAL",
                message: format!("authored edge lookup failed: {error}"),
            })?;
        if !connected {
            return Err(AgentCommunicationError {
                code: "AGENT_POLICY_EDGE_REQUIRED",
                message: format!(
                    "no authored agent-canvas edge between '{sender_agent_id}' and '{target}'"
                ),
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct AgentCommunicationError {
    pub code: &'static str,
    pub message: String,
}

#[cfg(test)]
#[path = "tests/communication_tests.rs"]
mod tests;
