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

/// Node border color preset on agent-canvas (docs/cw/04_客製化設計.md §1,
/// P4.6). `color: None` resets to the default (gray).
#[tauri::command]
pub fn agent_canvas_set_agent_color(
    workspace_id: String,
    agent_id: String,
    color: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    repo.set_agent_color(&workspace_id, &agent_id, color)
        .map_err(to_command_error)?;
    Ok(json!({}))
}

/// Both endpoints require `from_agent_id` and `to_agent_id` to already exist
/// in this workspace, so authored edges can never point at a dangling or
/// cross-workspace agent id (docs/cw/04_客製化設計.md §1, P4.5).
fn ensure_distinct_agents_exist(
    repo: &impl AgentRepository,
    workspace_id: &str,
    from_agent_id: &str,
    to_agent_id: &str,
) -> Result<(), String> {
    if from_agent_id == to_agent_id {
        return Err("an agent cannot be linked to itself".to_string());
    }
    let agents = repo.list_agents(workspace_id).map_err(to_command_error)?;
    for agent_id in [from_agent_id, to_agent_id] {
        if !agents.iter().any(|agent| agent.id == agent_id) {
            return Err(format!("agent '{agent_id}' not found in workspace"));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn agent_canvas_create_authored_link(
    workspace_id: String,
    from_agent_id: String,
    to_agent_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    ensure_distinct_agents_exist(&repo, &workspace_id, &from_agent_id, &to_agent_id)?;
    repo.create_authored_link(&workspace_id, &from_agent_id, &to_agent_id)
        .map_err(to_command_error)?;
    Ok(json!({}))
}

/// Wire display color (docs/cw/04_客製化設計.md §8, P4.6). `color: None`
/// resets to the default (gray).
#[tauri::command]
pub fn agent_canvas_set_link_color(
    workspace_id: String,
    from_agent_id: String,
    to_agent_id: String,
    color: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    repo.set_link_color(&workspace_id, &from_agent_id, &to_agent_id, color)
        .map_err(to_command_error)?;
    Ok(json!({}))
}

/// Toggles a wire between unidirectional (single arrowhead) and
/// bidirectional (double arrowhead) — a display/state property, not
/// something a reverse drag creates (docs/cw/04_客製化設計.md §8, P4.6).
#[tauri::command]
pub fn agent_canvas_set_link_bidirectional(
    workspace_id: String,
    from_agent_id: String,
    to_agent_id: String,
    bidirectional: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    repo.set_link_bidirectional(&workspace_id, &from_agent_id, &to_agent_id, bidirectional)
        .map_err(to_command_error)?;
    Ok(json!({}))
}

#[tauri::command]
pub fn agent_canvas_delete_authored_link(
    workspace_id: String,
    from_agent_id: String,
    to_agent_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let deleted = repo
        .delete_authored_link(&workspace_id, &from_agent_id, &to_agent_id)
        .map_err(to_command_error)?;
    Ok(json!({ "deleted": deleted }))
}

/// Clears one derived (auto-recorded `gto send` activity) edge — a user
/// asked for a way to dismiss stale-looking activity lines from the canvas
/// without that meaning "these agents may no longer talk" (that's what
/// authored/`delete_authored_link` means). A future real dispatch between
/// the same pair simply re-records it.
#[tauri::command]
pub fn agent_canvas_delete_derived_link(
    workspace_id: String,
    from_agent_id: String,
    to_agent_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let deleted = repo
        .delete_derived_link(&workspace_id, &from_agent_id, &to_agent_id)
        .map_err(to_command_error)?;
    Ok(json!({ "deleted": deleted }))
}
