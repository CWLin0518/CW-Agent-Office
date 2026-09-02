use crate::{AgentLink, AgentLinkKind, AgentProfile};

/// Builds the short, runtime-only prompt fragment that teaches an agent about
/// the authored canvas edges it is allowed to use for delegation.
pub fn build_collaboration_context(
    workspace_id: &str,
    agent_id: &str,
    agents: &[AgentProfile],
    links: &[AgentLink],
) -> Option<String> {
    let mut collaborators = links
        .iter()
        .filter(|link| {
            link.workspace_id == workspace_id
                && link.kind == AgentLinkKind::Authored
                && (link.from_agent_id == agent_id || link.to_agent_id == agent_id)
        })
        .filter_map(|link| {
            let collaborator_id = if link.from_agent_id == agent_id {
                &link.to_agent_id
            } else {
                &link.from_agent_id
            };
            agents
                .iter()
                .find(|agent| agent.workspace_id == workspace_id && agent.id == *collaborator_id)
        })
        .collect::<Vec<_>>();

    collaborators.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    collaborators.dedup_by(|left, right| left.id == right.id);
    if collaborators.is_empty() {
        return None;
    }

    let connected = collaborators
        .iter()
        .map(|agent| format!("{} — {}; tool: {}", agent.id, agent.name, agent.tool))
        .collect::<Vec<_>>()
        .join("; ");

    Some(format!(
        "[GT Office collaboration context] You are {agent_id} in workspace {workspace_id}. Connected collaborators: {connected}. When a task has a meaningful, separable part suited to a connected collaborator, delegate it proactively with: gto agent send-task --target-agent-id <agent-id> --title <title> --markdown <markdown> --json. Do not delegate trivial work. Keep the returned taskId for status and handover follow-ups. Only the connected agents listed above are authorized collaboration targets."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentScope, AgentState};

    fn agent(workspace_id: &str, id: &str, name: &str) -> AgentProfile {
        AgentProfile {
            id: id.into(),
            workspace_id: workspace_id.into(),
            name: name.into(),
            tool: "codex".into(),
            workdir: None,
            custom_workdir: false,
            scope: AgentScope::Station,
            state: AgentState::Ready,
            employee_no: None,
            policy_snapshot_id: None,
            capability_snapshot_id: None,
            prompt_file_name: None,
            prompt_file_relative_path: None,
            launch_command: None,
            output_collection_enabled: false,
            session_boundary_auto_split_enabled: false,
            order_index: 0,
            parent_agent_id: None,
            external_template_path: None,
            git_tracked: true,
            layout_x: None,
            layout_y: None,
            color: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn link(workspace_id: &str, from: &str, to: &str, kind: AgentLinkKind) -> AgentLink {
        AgentLink {
            id: format!("{from}-{to}"),
            workspace_id: workspace_id.into(),
            from_agent_id: from.into(),
            to_agent_id: to.into(),
            kind,
            color: None,
            bidirectional: false,
            created_at_ms: 0,
        }
    }

    #[test]
    fn includes_only_authored_neighbors_in_the_same_workspace() {
        let agents = vec![
            agent("ws", "a", "Alpha"),
            agent("ws", "b", "Builder"),
            agent("other", "c", "Other"),
        ];
        let links = vec![
            link("ws", "b", "a", AgentLinkKind::Authored),
            link("ws", "a", "missing", AgentLinkKind::Authored),
            link("ws", "a", "b", AgentLinkKind::Derived),
            link("other", "a", "c", AgentLinkKind::Authored),
        ];
        let context = build_collaboration_context("ws", "a", &agents, &links).expect("context");
        assert!(context.contains("b — Builder; tool: codex"));
        assert!(!context.contains("missing"));
        assert!(!context.contains("Other"));
    }
}
