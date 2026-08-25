use std::path::Path;

use gt_agent::{
    AgentCapabilityAuditRepository, AgentCapabilityRepository, AgentCapabilitySnapshot,
    AgentRepository, HookAuditEntry, HookCapability, MaterializedCapability,
};
use gt_storage::SqliteAgentRepository;
use gt_task::AgentToolKind;
use gt_tools::agent_installer::{AgentInstaller, AgentType};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::app_state::AppState;

use super::{
    ensure_workspace_exists, get_workspace_root, resolve_agent_repository, to_command_error,
};

/// Runs whichever of `gt_agent::materialize_claude_capability` /
/// `materialize_codex_capability` matches `tool_kind` for this agent's
/// stored MCP capability snapshot (docs/cw/08_MCP_Hook_Skill掛載設計.md
/// §2.3/§2.4) — shared by an agent's very first launch
/// (`tool_adapter::tool_profiles::resolve_launch_command`) and every later
/// resume/relaunch (`session::session_resume_check`), so both mount the
/// identical thing instead of drifting apart. Previously neither call site
/// actually invoked materialize at all (only `agent_capability_save` wrote
/// the snapshot to the database) — a saved MCP server was never folded into
/// any launched CLI's `--mcp-config`/`-p` flag, so it never actually
/// mounted regardless of session state.
///
/// Best-effort throughout: any failure (repo read, CLI capability probe,
/// materialize IO) is logged and treated as "nothing to overlay" rather
/// than propagated — launching unmounted beats not launching at all.
pub(crate) fn materialize_capability_for_launch(
    repo: &SqliteAgentRepository,
    workspace_id: &str,
    workspace_root: &Path,
    agent_id: &str,
    agent_workdir: &Path,
    tool_kind: AgentToolKind,
) -> Option<MaterializedCapability> {
    let snapshot = match repo.get_agent_capability(workspace_id, agent_id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(
                agent_id,
                %error,
                "failed to read agent capability snapshot; launching unmounted"
            );
            return None;
        }
    };
    if snapshot.mcp_servers.is_empty() {
        return None;
    }

    match tool_kind {
        AgentToolKind::Claude => {
            let support = AgentInstaller::detect_capability_support(AgentType::ClaudeCode);
            match gt_agent::materialize_claude_capability(
                workspace_root,
                agent_id,
                agent_workdir,
                &snapshot,
                &support,
            ) {
                Ok(paths) => Some(MaterializedCapability::Claude(paths)),
                Err(error) => {
                    tracing::warn!(
                        agent_id,
                        %error,
                        "failed to materialize Claude MCP capability; launching unmounted"
                    );
                    None
                }
            }
        }
        AgentToolKind::Codex => {
            let Some(codex_home) = gt_agent::resolve_codex_home() else {
                tracing::warn!(
                    agent_id,
                    "could not resolve CODEX_HOME; launching unmounted"
                );
                return None;
            };
            match gt_agent::materialize_codex_capability(
                &codex_home,
                workspace_id,
                agent_id,
                &snapshot,
            ) {
                Ok(Some(profile)) => Some(MaterializedCapability::Codex(profile)),
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(
                        agent_id,
                        %error,
                        "failed to materialize Codex MCP capability; launching unmounted"
                    );
                    None
                }
            }
        }
        AgentToolKind::Shell | AgentToolKind::Unknown => None,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityListAvailableSkillsRequest {
    pub workspace_id: String,
}

/// Backs the Skills sub-tab's checklist (split into "workspace" vs. "global"
/// sections in the UI) — fills the gap docs/cw/09_P3.5-capability開發進度.md
/// §1.3 explicitly left open ("Skills 子分頁不做勾選 workspace 內已存在的技能").
/// Read-only: never touches `agent_capability_snapshots`.
#[tauri::command]
pub fn agent_capability_list_available_skills(
    request: AgentCapabilityListAvailableSkillsRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let workspace_root = get_workspace_root(&state, &request.workspace_id)?;
    let skills = gt_agent::list_available_skills(&workspace_root);
    Ok(json!({ "skills": skills }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityListAvailableHooksRequest {
    pub workspace_id: String,
}

/// Backs the Hooks sub-tab's checklist, same shape as
/// `agent_capability_list_available_skills` but scanning `.claude/settings.json`
/// hooks instead of `SKILL.md` files (docs/cw/11_Skill掛載清單化.md's pattern
/// extended to hooks). Read-only: never touches `agent_capability_snapshots`
/// and does not grant any exemption from the preview/confirm flow that still
/// gates `agent_capability_save` for every hook, discovered or hand-typed.
#[tauri::command]
pub fn agent_capability_list_available_hooks(
    request: AgentCapabilityListAvailableHooksRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let workspace_root = get_workspace_root(&state, &request.workspace_id)?;
    let hooks = gt_agent::list_available_hooks(&workspace_root);
    Ok(json!({ "hooks": hooks }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityListOutputFilesRequest {
    pub workspace_id: String,
    pub agent_id: String,
}

/// Backs the Agent Canvas output list node (docs/cw/14_Agent輸出清單化.md
/// §4.1) — read-only scan of this agent's fixed output directory
/// (`<workspace_root>/.gtoffice/agents/<agent_id>/outputs/`), same
/// "directory doesn't exist yet" == empty-list convention as
/// `agent_capability_list_available_skills`/`_hooks` above. Lives alongside
/// those two rather than in the `agent_canvas` command module: like skills/
/// hooks discovery, this is a read-only scan of files belonging to the
/// agent itself, not canvas-specific state (links/layout/color) the way the
/// rest of `agent_canvas::*` is.
#[tauri::command]
pub fn agent_capability_list_output_files(
    request: AgentCapabilityListOutputFilesRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let workspace_root = get_workspace_root(&state, &request.workspace_id)?;
    let files = gt_agent::list_agent_output_files(&workspace_root, &request.agent_id);
    Ok(json!({ "files": files }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityOpenOutputFileRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub file_name: String,
}

/// Backs the Agent Canvas output list node's non-markdown rows (docs/cw/14_Agent輸出清單化.md
/// §3.2/§4.4) — hands the file off to the OS default program (a `.html` file
/// opens in the user's default browser, anything else opens in whatever the
/// OS associates with it). Reuses the same `open` crate `fs_show_in_folder`
/// already depends on (`apps/desktop-tauri/src-tauri/Cargo.toml`'s
/// `open = "5.3"`) rather than adding a Tauri plugin for this. `file_name`
/// (not a client-supplied absolute path) is what's trusted here —
/// `resolve_agent_output_file_path` reconstructs the real path server-side
/// and rejects anything that isn't a single, existing, plain file name
/// directly inside this agent's outputs directory.
#[tauri::command]
pub fn agent_capability_open_output_file(
    request: AgentCapabilityOpenOutputFileRequest,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let workspace_root = get_workspace_root(&state, &request.workspace_id)?;
    let path = gt_agent::resolve_agent_output_file_path(
        &workspace_root,
        &request.agent_id,
        &request.file_name,
    )
    .map_err(|error| format!("AGENT_OUTPUT_FILE_INVALID: {error}"))?;
    open::that(&path).map_err(|error| format!("AGENT_OUTPUT_OPEN_FAILED: {error}"))?;
    Ok(json!({ "opened": true }))
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
                "note": hook.note,
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
