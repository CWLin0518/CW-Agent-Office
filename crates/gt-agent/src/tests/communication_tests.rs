use super::*;

#[test]
fn denies_unconnected_target_before_any_message_is_sent() {
    let targets = vec!["connected".into(), "unconnected".into()];
    let error = authorize_agent_communication("sender", &targets, true, &[], |_, target| {
        Ok(target == "connected")
    })
    .expect_err("every target requires authorization");
    assert_eq!(error.code, "AGENT_POLICY_EDGE_REQUIRED");
}

#[test]
fn policy_denial_overrides_broadcast_and_self_exceptions() {
    let error = authorize_agent_communication(
        "sender",
        &["sender".into()],
        false,
        &["sender".into()],
        |_, _| panic!("policy denial must precede edge lookup"),
    )
    .expect_err("explicit deny wins");
    assert_eq!(error.code, "AGENT_POLICY_GTO_SEND_DENIED");
}

#[test]
fn permits_connected_status_and_handover_targets() {
    assert!(
        authorize_agent_communication("sender", &["parent".into()], true, &[], |_, _| Ok(true))
            .is_ok()
    );
}

#[test]
fn preserves_self_and_broadcast_exceptions() {
    for (sender, targets, broadcast) in [
        ("a", vec!["a".into()], vec![]),
        ("a", vec!["b".into()], vec!["a".into()]),
        ("a", vec!["b".into()], vec!["b".into()]),
    ] {
        assert!(
            authorize_agent_communication(sender, &targets, true, &broadcast, |_, _| panic!(
                "explicit exceptions do not need edges"
            ))
            .is_ok()
        );
    }
}

#[test]
fn fails_closed_when_edge_lookup_fails() {
    let error = authorize_agent_communication("sender", &["target".into()], true, &[], |_, _| {
        Err(crate::AgentError::Storage {
            message: "lookup unavailable".into(),
        })
    })
    .expect_err("lookup error must not allow communication");
    assert_eq!(error.code, "LOCAL_BRIDGE_INTERNAL");
}
