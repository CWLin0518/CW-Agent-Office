use gt_agent::{
    AgentCapabilityAuditRepository, AgentCapabilityRepository, AgentCapabilitySnapshot,
    AgentRepository, HookAuditEntry, HookCapability,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::app_state::AppState;

use super::{ensure_workspace_exists, resolve_agent_repository, to_command_error};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityReadRequest {
    pub workspace_id: String,
    pub agent_id: String,
}

/// Returns `AgentCapabilitySnapshot::default()` (nothing mounted) when the
/// agent has no snapshot yet — mirrors `agent_policy_read`'s "no snapshot
/// yet" handling. Also returns the confirmed hook hashes for this agent so
/// the Capabilities tab can render "already confirmed" state for existing
/// hooks without a second round trip.
#[tauri::command]
pub fn agent_capability_read(
    request: AgentCapabilityReadRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let capability = repo
        .get_agent_capability(&request.workspace_id, &request.agent_id)
        .map_err(to_command_error)?;
    let confirmed_hook_hashes = repo
        .confirmed_hook_hashes(&request.workspace_id, &request.agent_id)
        .map_err(to_command_error)?;
    Ok(json!({
        "capability": capability,
        "confirmedHookHashes": confirmed_hook_hashes,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityPreviewHooksRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub hooks: Vec<HookCapability>,
}

/// Pure preview — never writes anything. Per docs/cw/08_MCP_Hook_Skill掛載設計.md
/// §2.5 決策3/§3, a hook must be shown here (event/matcher/command spelled
/// out per rule, not a JSON blob) before the UI is allowed to call
/// `agent_capability_confirm_hooks` for it.
#[tauri::command]
pub fn agent_capability_preview_hooks(
    request: AgentCapabilityPreviewHooksRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let confirmed = repo
        .confirmed_hook_hashes(&request.workspace_id, &request.agent_id)
        .map_err(to_command_error)?;

    let items: Vec<Value> = request
        .hooks
        .iter()
        .map(|hook| {
            let hash = hook.content_hash();
            json!({
                "event": hook.event,
                "matcher": hook.matcher,
                "command": hook.command,
                "hash": hash.clone(),
                "alreadyConfirmed": confirmed.contains(&hash),
            })
        })
        .collect();
    Ok(json!({ "items": items }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityConfirmHooksRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub hook_hashes: Vec<String>,
    pub confirmed_by: String,
}

/// Records that the user has walked through the preview for exactly these
/// hashes and clicked confirm. Idempotent (see
/// `AgentCapabilityAuditRepository::confirm_hook_hashes`) — calling this
/// again for an already-confirmed hash changes nothing.
#[tauri::command]
pub fn agent_capability_confirm_hooks(
    request: AgentCapabilityConfirmHooksRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    if request.confirmed_by.trim().is_empty() {
        return Err("CAPABILITY_CONFIRMED_BY_REQUIRED: confirmedBy must not be empty".to_string());
    }
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    repo.confirm_hook_hashes(
        &request.workspace_id,
        &request.agent_id,
        &request.hook_hashes,
        &request.confirmed_by,
    )
    .map_err(to_command_error)?;
    Ok(json!({ "confirmed": true }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilitySaveRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub capability: AgentCapabilitySnapshot,
    pub confirmed_by: String,
}

/// The `apply` step of docs/cw/08_MCP_Hook_Skill掛載設計.md §3's
/// preview -> validate -> confirm -> apply -> audit flow for hooks (MCP
/// servers / skills have no version-lock — only hooks run arbitrary
/// user-authored commands). Rejects the save outright if any hook in
/// `capability.hooks` hasn't been confirmed for this `(workspace_id,
/// agent_id)` — a client that skips the preview/confirm round trip (or
/// tries to sneak in a hook it never actually showed the user) gets an
/// error here, not a silent write. `AgentCapabilitySnapshot::validate_for_tool`
/// (Codex skills/hooks rejection, MCP transport-field checks) still runs
/// inside `save_agent_capability` itself, unchanged from earlier phases.
#[tauri::command]
pub fn agent_capability_save(
    request: AgentCapabilitySaveRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    if request.confirmed_by.trim().is_empty() {
        return Err("CAPABILITY_CONFIRMED_BY_REQUIRED: confirmedBy must not be empty".to_string());
    }
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;

    let confirmed = repo
        .confirmed_hook_hashes(&request.workspace_id, &request.agent_id)
        .map_err(to_command_error)?;
    let unconfirmed: Vec<&HookCapability> = request
        .capability
        .hooks
        .iter()
        .filter(|hook| !confirmed.contains(&hook.content_hash()))
        .collect();
    if !unconfirmed.is_empty() {
        return Err(format!(
            "CAPABILITY_HOOKS_NOT_CONFIRMED: {} hook(s) have not been previewed and confirmed \
             yet — call agent_capability_preview_hooks then agent_capability_confirm_hooks first",
            unconfirmed.len()
        ));
    }

    let snapshot_id = repo
        .save_agent_capability(
            &request.workspace_id,
            &request.agent_id,
            &request.capability,
        )
        .map_err(to_command_error)?;

    if !request.capability.hooks.is_empty() {
        let now = now_ms();
        let entries: Vec<HookAuditEntry> = request
            .capability
            .hooks
            .iter()
            .map(|hook| HookAuditEntry {
                id: uuid::Uuid::new_v4().to_string(),
                workspace_id: request.workspace_id.clone(),
                agent_id: request.agent_id.clone(),
                hook_hash: hook.content_hash(),
                event: hook.event.clone(),
                matcher: hook.matcher.clone(),
                command: hook.command.clone(),
                confirmed_by: request.confirmed_by.clone(),
                created_at_ms: now,
            })
            .collect();
        repo.record_hook_audit(&entries).map_err(to_command_error)?;
    }

    Ok(json!({ "snapshotId": snapshot_id }))
}
