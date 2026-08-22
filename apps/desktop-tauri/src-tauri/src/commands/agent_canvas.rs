//! Backend for agent-canvas (docs/cw/04_客製化設計.md §1, P4 MVP): listing the
//! current workspace's `agent_links` and a minimal runtime-status projection,
//! plus persisting the one thing the read-only MVP still lets the user change
//! — a node's dragged canvas position.

use gt_agent::{AgentLinkRepository, AgentRepository, AgentRuntimeState, AgentRuntimeStatus};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use super::agent::{ensure_workspace_exists, resolve_agent_repository, to_command_error};
use crate::app_state::AppState;

#[tauri::command]
pub fn agent_canvas_list_links(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let links = repo.list_links(&workspace_id).map_err(to_command_error)?;
    Ok(json!({ "links": links }))
}

#[tauri::command]
pub fn agent_canvas_runtime_status(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let agents = repo.list_agents(&workspace_id).map_err(to_command_error)?;

    // gt-task only knows about currently-registered (online) runtimes — it has
    // no view of the full agent roster, so it can't tell "known agent that's
    // offline" apart from "id that doesn't exist" (see the doc comment on
    // `TaskService::agent_runtime_status`). This command has both the roster
    // (via AgentRepository, above) and the online set, so it does the merge:
    // anyone in the roster but missing from the online set is Offline.
    let online_statuses = state.task_service.agent_runtime_status(&workspace_id);
    let statuses: Vec<AgentRuntimeStatus> = agents
        .into_iter()
        .map(|agent| {
            online_statuses
                .iter()
                .find(|status| status.agent_id == agent.id)
                .cloned()
                .unwrap_or(AgentRuntimeStatus {
                    agent_id: agent.id,
                    workspace_id: workspace_id.clone(),
                    state: AgentRuntimeState::Offline,
                    updated_at_ms: 0,
                })
        })
        .collect();
    Ok(json!({ "statuses": statuses }))
}

#[tauri::command]
pub fn agent_canvas_set_layout(
    workspace_id: String,
    agent_id: String,
    x: f64,
    y: f64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    repo.set_agent_layout(&workspace_id, &agent_id, x, y)
        .map_err(to_command_error)?;
    Ok(json!({}))
}
