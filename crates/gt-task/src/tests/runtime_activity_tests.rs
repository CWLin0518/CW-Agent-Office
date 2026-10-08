use super::*;
use crate::{AgentRuntimeRegistration, AgentToolKind};

#[test]
fn publish_authorizes_the_same_targets_as_delivery_for_all_message_types() {
    use crate::{ChannelDescriptor, ChannelKind, ChannelMessageType, ChannelPublishRequest};
    for message_type in [
        ChannelMessageType::TaskInstruction,
        ChannelMessageType::Status,
        ChannelMessageType::Handover,
    ] {
        let mut request = ChannelPublishRequest {
            workspace_id: "ws".into(),
            sender_agent_id: Some("agent".into()),
            channel: ChannelDescriptor {
                kind: ChannelKind::Direct,
                id: " target ".into(),
            },
            target_agent_ids: vec![],
            message_type,
            payload: serde_json::json!({}),
            idempotency_key: None,
        };
        assert_eq!(request.resolved_target_agent_ids(), vec!["target"]);
        request.target_agent_ids = vec![" other ".into(), "other".into(), "".into()];
        assert_eq!(request.resolved_target_agent_ids(), vec!["other"]);
        request.target_agent_ids.clear();
        request.channel.kind = ChannelKind::Broadcast;
        assert!(request.resolved_target_agent_ids().is_empty());
    }
}

fn register(service: &TaskService, workspace: &str, session: &str) {
    service.register_runtime(AgentRuntimeRegistration {
        workspace_id: workspace.into(),
        agent_id: "agent".into(),
        station_id: "station".into(),
        session_id: session.into(),
        tool_kind: AgentToolKind::Claude,
        resolved_cwd: None,
        submit_sequence: None,
        provider_session: None,
        online: true,
    });
}

#[test]
fn output_without_messages_becomes_active_then_quiet_idle() {
    let service = TaskService::default();
    register(&service, "ws", "s");
    assert_eq!(
        service
            .agent_runtime_status_at("ws", 100)
            .first()
            .unwrap()
            .state,
        AgentRuntimeState::Idle
    );
    service.observe_terminal_output("ws", "s", 100);
    assert_eq!(
        service
            .agent_runtime_status_at("ws", 100)
            .first()
            .unwrap()
            .state,
        AgentRuntimeState::Active
    );
    assert_eq!(
        service
            .agent_runtime_status_at("ws", 10_101)
            .first()
            .unwrap()
            .state,
        AgentRuntimeState::Idle
    );
}

#[test]
fn scopes_output_and_exit_to_workspace_and_current_session() {
    let service = TaskService::default();
    register(&service, "ws", "s");
    register(&service, "other", "s");
    service.observe_terminal_output("ws", "s", 100);
    assert_eq!(
        service
            .agent_runtime_status_at("other", 100)
            .first()
            .unwrap()
            .state,
        AgentRuntimeState::Idle
    );
    register(&service, "ws", "replacement");
    service.observe_terminal_output("ws", "s", 200);
    service.observe_terminal_state("ws", "s", "exited");
    assert_eq!(
        service
            .agent_runtime_status_at("ws", 200)
            .first()
            .unwrap()
            .state,
        AgentRuntimeState::Idle
    );
    service.observe_terminal_state("ws", "replacement", "failed");
    assert!(service.agent_runtime_status("ws").is_empty());
    assert_eq!(service.agent_runtime_status("other").len(), 1);
}

#[test]
fn out_of_order_output_does_not_move_activity_backwards() {
    let service = TaskService::default();
    register(&service, "ws", "s");
    service.observe_terminal_output("ws", "s", 500);
    service.observe_terminal_output("ws", "s", 100);
    assert_eq!(
        service
            .agent_runtime_status_at("ws", 10_400)
            .first()
            .unwrap()
            .state,
        AgentRuntimeState::Active
    );
    service.unregister_runtime("ws", "agent");
    register(&service, "ws", "s");
    assert_eq!(
        service
            .agent_runtime_status_at("ws", 10_400)
            .first()
            .unwrap()
            .state,
        AgentRuntimeState::Idle
    );
}
