use gt_abstractions::AbstractionError;
use gt_agent::AgentRepository;
use gt_task::{
    AgentRuntimeRegistration, ChannelAckEvent, ChannelMessageEvent, ChannelPublishRequest,
    TaskDispatchBatchRequest, TaskDispatchProgressEvent, TaskGetThreadRequest,
    TaskListThreadsRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::{thread, time::Duration};
use tauri::{AppHandle, Emitter, State};

use crate::app_state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeUnregisterRequest {
    pub workspace_id: String,
    pub agent_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelListMessagesRequest {
    pub workspace_id: String,
    #[serde(default)]
    pub target_agent_id: Option<String>,
    #[serde(default)]
    pub sender_agent_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

fn to_terminal_error(error: AbstractionError) -> String {
    match error {
        AbstractionError::WorkspaceNotFound { workspace_id } => {
            format!("WORKSPACE_NOT_FOUND: workspace '{workspace_id}' does not exist")
        }
        AbstractionError::InvalidArgument { message } => message,
        AbstractionError::AccessDenied { message } => message,
        AbstractionError::Conflict { message } => format!("TERMINAL_CONFLICT: {message}"),
        AbstractionError::Internal { message } => message,
    }
}

fn emit_channel_events(
    app: &AppHandle,
    message_events: &[ChannelMessageEvent],
    ack_events: &[ChannelAckEvent],
) {
    for event in message_events {
        let _ = app.emit("channel/message", event);
    }
    for event in ack_events {
        let _ = app.emit("channel/ack", event);
    }
}

fn emit_dispatch_progress_events(app: &AppHandle, events: &[TaskDispatchProgressEvent]) {
    for event in events {
        let _ = app.emit("task/dispatch_progress", event);
    }
}

pub(crate) fn write_terminal_with_submit(
    state: &AppState,
    session_id: &str,
    command: &str,
    submit_sequence: &str,
) -> Result<(), String> {
    let chunks = build_terminal_submit_chunks(command, submit_sequence);
    write_terminal_chunks(state, session_id, &chunks, !command.is_empty())
}

pub(crate) fn write_terminal_command_with_submit(
    state: &AppState,
    session_id: &str,
    command: &str,
    submit_sequence: &str,
) -> Result<(), String> {
    let chunks = build_terminal_command_submit_chunks(command, submit_sequence);
    write_terminal_chunks(state, session_id, &chunks, !command.is_empty())
}

fn write_terminal_chunks(
    state: &AppState,
    session_id: &str,
    chunks: &[String],
    has_command: bool,
) -> Result<(), String> {
    for (index, chunk) in chunks.iter().enumerate() {
        let accepted = state
            .terminal_provider
            .write_session(session_id, chunk)
            .map_err(to_terminal_error)?;
        if !accepted {
            let reason = match index {
                0 if has_command => "CHANNEL_DELIVERY_FAILED: terminal write rejected",
                0 => "CHANNEL_DELIVERY_FAILED: terminal submit rejected",
                1 => "CHANNEL_DELIVERY_FAILED: terminal submit rejected",
                _ => "CHANNEL_DELIVERY_FAILED: terminal hard submit rejected",
            };
            return Err(reason.to_string());
        }
        if index + 1 < chunks.len() {
            thread::sleep(Duration::from_millis(terminal_chunk_delay_ms(
                index,
                has_command,
            )));
        }
    }
    Ok(())
}

pub(crate) fn terminal_chunk_delay_ms(index: usize, has_command: bool) -> u64 {
    // Codex renders long PTY writes as bracketed pasted content. Sending Enter
    // immediately can arrive before that paste has been committed to its input
    // editor, leaving `[Pasted Content ...]` staged instead of submitted.
    if has_command && index == 0 {
        500
    } else {
        150
    }
}

pub(crate) fn build_terminal_submit_chunks(command: &str, submit_sequence: &str) -> Vec<String> {
    let mut chunks = Vec::with_capacity(3);
    if !command.is_empty() {
        chunks.push(command.to_string());
    }
    chunks.push(submit_sequence.to_string());
    chunks.push("\r".to_string());
    chunks
}

pub(crate) fn build_terminal_command_submit_chunks(
    command: &str,
    submit_sequence: &str,
) -> Vec<String> {
    let mut chunks = Vec::with_capacity(2);
    if !command.is_empty() {
        chunks.push(command.to_string());
    }
    chunks.push(submit_sequence.to_string());
    chunks
}

#[tauri::command]
pub fn task_list(scope: Option<String>) -> Result<Value, String> {
    Ok(json!({ "scope": scope.unwrap_or_else(|| "global".to_string()), "tasks": [] }))
}

#[tauri::command]
pub fn task_dispatch_batch(
    request: TaskDispatchBatchRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if request.workspace_id.trim().is_empty() {
        return Err("TASK_DISPATCH_INVALID: workspaceId is required".to_string());
    }
    if request.targets.is_empty() {
        return Err("TASK_DISPATCH_INVALID: targets must not be empty".to_string());
    }
    if request.markdown.trim().is_empty() {
        return Err("TASK_DISPATCH_INVALID: markdown must not be empty".to_string());
    }

    let workspace_root = state.workspace_root_path(&request.workspace_id)?;
    let repo = crate::commands::agent::resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(|error| error.to_string())?;
    let mut output_directories = HashMap::new();
    let mut session_boundary_agents = std::collections::HashSet::new();
    for agent in repo
        .list_agents(&request.workspace_id)
        .map_err(|error| error.to_string())?
    {
        if !request.targets.contains(&agent.id) {
            continue;
        }
        if agent.session_boundary_auto_split_enabled {
            session_boundary_agents.insert(agent.id.clone());
        }
        if agent.output_collection_enabled {
            let output_dir = workspace_root
                .join(".gtoffice")
                .join("agents")
                .join(&agent.id)
                .join("outputs");
            std::fs::create_dir_all(&output_dir)
                .map_err(|error| format!("AGENT_OUTPUT_DIR_CREATE_FAILED: {error}"))?;
            output_directories.insert(agent.id, output_dir.to_string_lossy().into_owned());
        }
    }
    let outcome = state.task_service.dispatch_batch_with_output_directories(
        &request,
        &workspace_root,
        &output_directories,
        &session_boundary_agents,
        |session_id, command, submit_sequence| {
            write_terminal_with_submit(state.inner(), session_id, command, submit_sequence)
        },
    );

    emit_dispatch_progress_events(&app, &outcome.progress_events);
    emit_channel_events(&app, &outcome.message_events, &outcome.ack_events);
    crate::commands::tool_adapter::bind_task_wait_reply_sessions(
        state.inner(),
        &request,
        &outcome.response.results,
    );

    serde_json::to_value(outcome.response).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn task_list_threads(
    request: TaskListThreadsRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if request.workspace_id.trim().is_empty() {
        return Err("TASK_THREAD_LIST_INVALID: workspaceId is required".to_string());
    }
    let threads = state.task_service.list_task_threads(
        &request.workspace_id,
        request.agent_id.as_deref(),
        request.limit.unwrap_or(20) as usize,
    );
    Ok(json!({ "threads": threads }))
}

#[tauri::command]
pub fn task_get_thread(
    request: TaskGetThreadRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if request.workspace_id.trim().is_empty() {
        return Err("TASK_THREAD_GET_INVALID: workspaceId is required".to_string());
    }
    if request.task_id.trim().is_empty() {
        return Err("TASK_THREAD_GET_INVALID: taskId is required".to_string());
    }
    let thread = state
        .task_service
        .get_task_thread(&request.workspace_id, &request.task_id);
    let wait_state = state.task_wait_state(&request.workspace_id, &request.task_id)?;
    Ok(json!({ "thread": thread, "waitState": wait_state }))
}

#[tauri::command]
pub fn channel_publish(
    request: ChannelPublishRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if request.workspace_id.trim().is_empty() {
        return Err("CHANNEL_PUBLISH_INVALID: workspaceId is required".to_string());
    }

    let outcome = state.task_service.publish(&request);
    emit_channel_events(&app, &outcome.message_events, &outcome.ack_events);
    serde_json::to_value(outcome.response).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn channel_list_messages(
    request: ChannelListMessagesRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if request.workspace_id.trim().is_empty() {
        return Err("CHANNEL_LIST_INVALID: workspaceId is required".to_string());
    }
    let messages = state.task_service.list_messages(
        &request.workspace_id,
        request.target_agent_id.as_deref(),
        request.sender_agent_id.as_deref(),
        request.task_id.as_deref(),
        request.limit.unwrap_or(20) as usize,
    );
    Ok(json!({ "messages": messages }))
}

#[tauri::command]
pub fn agent_runtime_register(
    request: AgentRuntimeRegistration,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if request.workspace_id.trim().is_empty() {
        return Err("AGENT_RUNTIME_INVALID: workspaceId is required".to_string());
    }
    if request.agent_id.trim().is_empty() {
        return Err("AGENT_RUNTIME_INVALID: agentId is required".to_string());
    }
    if request.session_id.trim().is_empty() {
        return Err("AGENT_RUNTIME_INVALID: sessionId is required".to_string());
    }

    let registered = state.task_service.register_runtime(request.clone());
    crate::local_bridge::spawn_refresh_directory_snapshot(
        app.clone(),
        state.inner().clone(),
        request.workspace_id.clone(),
    );
    Ok(json!({
        "workspaceId": request.workspace_id,
        "agentId": request.agent_id,
        "stationId": request.station_id,
        "sessionId": request.session_id,
        "toolKind": request.tool_kind,
        "resolvedCwd": request.resolved_cwd,
        "submitSequence": request.submit_sequence,
        "providerSession": request.provider_session,
        "registered": registered,
    }))
}

#[tauri::command]
pub fn agent_runtime_unregister(
    request: AgentRuntimeUnregisterRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if request.workspace_id.trim().is_empty() {
        return Err("AGENT_RUNTIME_INVALID: workspaceId is required".to_string());
    }
    if request.agent_id.trim().is_empty() {
        return Err("AGENT_RUNTIME_INVALID: agentId is required".to_string());
    }
    let unregistered = state
        .task_service
        .unregister_runtime(&request.workspace_id, &request.agent_id);
    crate::local_bridge::spawn_refresh_directory_snapshot(
        app.clone(),
        state.inner().clone(),
        request.workspace_id.clone(),
    );
    Ok(json!({
        "workspaceId": request.workspace_id,
        "agentId": request.agent_id,
        "unregistered": unregistered,
    }))
}

#[tauri::command]
pub fn changefeed_query(
    workspace_id: String,
    session_id: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    Ok(json!({
        "workspaceId": workspace_id,
        "sessionId": session_id,
        "limit": limit.unwrap_or(100),
        "events": []
    }))
}
