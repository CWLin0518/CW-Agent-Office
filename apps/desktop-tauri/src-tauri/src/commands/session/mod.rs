use gt_agent_session::{
    derive_session_title_from_task, run_discovery, DiscoveryCache, Provider, ProviderScanner,
    ResumeService, SessionRelaunchMode,
};
use gt_changefeed::{GitStatusSnapshot, SessionActivityEvent};
use gt_task::AgentToolKind;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::app_state::AppState;
use crate::commands::agent::{
    capability::materialize_capability_for_launch, resolve_agent_repository,
};

static DISCOVERY_CACHE: Mutex<Option<HashMap<String, DiscoveryCache>>> = Mutex::new(None);

pub(crate) fn discovery_cache_key(workspace_id: &str, provider: Option<Provider>) -> String {
    match provider {
        Some(provider) => format!("{workspace_id}:{}", provider.as_str()),
        None => workspace_id.to_string(),
    }
}

fn parse_provider(provider: Option<String>) -> Result<Option<Provider>, String> {
    match provider {
        None => Ok(None),
        Some(value) => Provider::from_str_opt(&value)
            .map(Some)
            .ok_or_else(|| format!("unsupported provider: {value}")),
    }
}

pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn normalize_existing_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn resolve_workspace_session_cwd_path(
    workspace_root: &Path,
    cwd: &str,
) -> Result<PathBuf, String> {
    let normalized_workspace_root = normalize_existing_path(workspace_root);
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return Err("SESSION_CWD_INVALID: cwd is required".to_string());
    }
    let raw_cwd = Path::new(cwd);
    let candidate = if raw_cwd.is_absolute() {
        raw_cwd.to_path_buf()
    } else {
        normalized_workspace_root.join(raw_cwd)
    };
    let metadata = candidate.metadata().map_err(|error| {
        format!(
            "SESSION_CWD_INVALID: cwd '{}' is not accessible: {error}",
            candidate.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "SESSION_CWD_INVALID: cwd '{}' must be a directory",
            candidate.display()
        ));
    }
    let normalized_candidate = normalize_existing_path(&candidate);
    if !normalized_candidate.starts_with(&normalized_workspace_root) {
        return Err(format!(
            "SESSION_CWD_OUTSIDE_WORKSPACE: cwd '{}' is outside workspace '{}'",
            normalized_candidate.display(),
            normalized_workspace_root.display()
        ));
    }
    Ok(normalized_candidate)
}

fn resolve_workspace_session_cwd(
    state: &AppState,
    workspace_id: &str,
    cwd: &str,
) -> Result<PathBuf, String> {
    let workspace_root = state.workspace_root_path(workspace_id)?;
    resolve_workspace_session_cwd_path(&workspace_root, cwd)
}

#[tauri::command]
pub fn session_list(
    workspace_id: String,
    provider: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let limit = limit.unwrap_or(50);
    let offset = offset.unwrap_or(0);
    let provider = parse_provider(provider)?;
    state
        .session_registry
        .backfill_missing_titles(&workspace_id, provider, limit)
        .map_err(|e| e.to_string())?;
    let cards = state
        .session_registry
        .list_cards_by_workspace(&workspace_id, provider, limit, offset)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "cards": cards, "limit": limit, "offset": offset }))
}

#[tauri::command]
pub fn session_discover(
    workspace_id: String,
    cwd: String,
    provider: Option<String>,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let provider = parse_provider(provider)?;
    let force = force.unwrap_or(false);
    let resolved_cwd = resolve_workspace_session_cwd(state.inner(), &workspace_id, &cwd)?;
    let scanner = ProviderScanner::new(home_dir());
    let cache_key = discovery_cache_key(&workspace_id, provider);
    let mut cache_guard = DISCOVERY_CACHE
        .lock()
        .map_err(|_| "discovery cache lock poisoned")?;
    let caches = cache_guard.get_or_insert_with(HashMap::new);
    let cache = caches
        .entry(cache_key)
        .or_insert_with(|| DiscoveryCache::new(30_000));
    let result = run_discovery(
        &state.session_registry,
        &scanner,
        cache,
        &workspace_id,
        resolved_cwd.as_path(),
        provider,
        force,
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({
        "cards": result.cards,
        "newCount": result.new_count,
        "updatedCount": result.updated_count,
    }))
}

#[tauri::command]
pub fn session_get(
    workspace_id: String,
    gto_session_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let detail = state
        .session_registry
        .get_detail_for_workspace(&workspace_id, &gto_session_id)
        .map_err(|e| e.to_string())?;
    match detail {
        Some(d) => Ok(json!({ "session": d.session, "stats": d.stats })),
        None => Ok(json!({ "session": null, "stats": null })),
    }
}

#[tauri::command]
pub fn session_end(
    workspace_id: String,
    gto_session_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let updated = state
        .session_registry
        .update_lifecycle_for_workspace(
            &workspace_id,
            &gto_session_id,
            gt_agent_session::Lifecycle::Stopped,
            None,
        )
        .map_err(|e| e.to_string())?;
    if !updated {
        return Err("SESSION_END_NOT_FOUND".to_string());
    }
    state
        .session_registry
        .finalize_stopped_stats(&gto_session_id)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn session_launch(
    workspace_id: String,
    station_id: String,
    agent_id: String,
    provider: String,
    cwd: String,
    terminal_session_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    let provider = Provider::from_str_opt(&provider)
        .ok_or_else(|| format!("unsupported provider: {provider}"))?;
    let resolved_cwd = resolve_workspace_session_cwd(state.inner(), &workspace_id, &cwd)?;
    let gto_session_id = state
        .session_registry
        .launch_session(
            &workspace_id,
            &station_id,
            &agent_id,
            provider,
            resolved_cwd.to_string_lossy().as_ref(),
            terminal_session_id.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    emit_session_updated(
        &app,
        &workspace_id,
        &gto_session_id,
        terminal_session_id.as_deref(),
        None,
    );
    Ok(json!({ "gtoSessionId": gto_session_id }))
}

#[tauri::command]
pub fn session_resume_bind(
    workspace_id: String,
    gto_session_id: String,
    terminal_session_id: String,
    station_id: String,
    agent_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    let rebound = state
        .session_registry
        .resume_bind(
            &workspace_id,
            &gto_session_id,
            &terminal_session_id,
            &station_id,
            &agent_id,
        )
        .map_err(|e| e.to_string())?;
    if !rebound {
        return Err("SESSION_RESUME_BIND_NOT_FOUND".to_string());
    }
    let title = state
        .session_registry
        .get_for_workspace(&workspace_id, &gto_session_id)
        .ok()
        .flatten()
        .and_then(|session| session.title);
    emit_session_updated(
        &app,
        &workspace_id,
        &gto_session_id,
        Some(&terminal_session_id),
        title.as_deref(),
    );
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn session_resume_check(
    workspace_id: Option<String>,
    gto_session_id: Option<String>,
    relaunch_mode: Option<String>,
    expected_provider: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    let mode = relaunch_mode
        .as_deref()
        .and_then(SessionRelaunchMode::from_str_opt)
        .unwrap_or(SessionRelaunchMode::Resume);
    let expected = parse_provider(expected_provider)?;

    let needs_session = !matches!(
        mode,
        SessionRelaunchMode::ContinueLast | SessionRelaunchMode::ForkLast
    );

    if needs_session {
        let Some(workspace_id) = workspace_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            return Ok(json!({ "check": "not_found" }));
        };
        let Some(gto_session_id) = gto_session_id.filter(|id| !id.trim().is_empty()) else {
            return Ok(json!({ "check": "not_found" }));
        };
        let session = state
            .session_registry
            .get_for_workspace(workspace_id, &gto_session_id)
            .map_err(|e| e.to_string())?;
        let Some(session) = session else {
            return Ok(json!({ "check": "not_found" }));
        };
        if let Some(expected) = expected {
            if session.provider != expected {
                return Ok(json!({ "check": "providerMismatch" }));
            }
        }
        let check = ResumeService::validate_resumable(&session);
        // Materialize this agent's MCP capability snapshot (docs/cw/08_MCP_Hook_Skill掛載設計.md
        // §2.4) so a resumed session actually gets its `--mcp-config`/`-p`
        // overlay — best-effort: any failure along the way (repo, CLI probe,
        // materialize IO) just falls back to `None` (launch unmounted)
        // rather than failing the whole resume check.
        let materialized = resolve_agent_repository(&app).ok().and_then(|repo| {
            let workspace_root = state.workspace_root_path(&session.workspace_id).ok()?;
            let tool_kind = match session.provider {
                Provider::Claude => AgentToolKind::Claude,
                Provider::Codex => AgentToolKind::Codex,
            };
            materialize_capability_for_launch(
                &repo,
                &session.workspace_id,
                &workspace_root,
                &session.agent_id,
                Path::new(&session.cwd),
                tool_kind,
            )
        });
        let launch_command = ResumeService::build_relaunch_launch_command(
            Some(&session),
            session.provider,
            mode,
            materialized.as_ref(),
        );
        let steps = ResumeService::build_relaunch_commands(
            Some(&session),
            session.provider,
            mode,
            materialized.as_ref(),
        );
        return Ok(json!({ "check": check, "launchCommand": launch_command, "steps": steps }));
    }

    let Some(provider) = expected else {
        return Err("expected_provider is required for continueLast and forkLast".to_string());
    };
    let launch_command = ResumeService::build_relaunch_launch_command(None, provider, mode, None);
    let steps = ResumeService::build_relaunch_commands(None, provider, mode, None);
    Ok(json!({
        "check": "canResume",
        "launchCommand": launch_command,
        "steps": steps,
    }))
}

#[tauri::command]
pub fn session_update_title(
    workspace_id: String,
    gto_session_id: String,
    title: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("SESSION_UPDATE_TITLE_INVALID: title must not be empty".to_string());
    }
    let updated = state
        .session_registry
        .update_title_for_workspace(&workspace_id, &gto_session_id, &title)
        .map_err(|e| e.to_string())?;
    if !updated {
        return Err("SESSION_UPDATE_TITLE_NOT_FOUND".to_string());
    }
    emit_session_updated(&app, &workspace_id, &gto_session_id, None, Some(&title));
    Ok(json!({ "ok": true }))
}

/// The live session behind a station's terminal, for the pane's session card.
#[tauri::command]
pub fn session_current_for_terminal(
    workspace_id: String,
    terminal_session_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let current = state
        .session_registry
        .live_session_for_terminal(&workspace_id, &terminal_session_id)
        .map_err(|e| e.to_string())?;
    Ok(match current {
        Some((gto_session_id, title)) => json!({ "gtoSessionId": gto_session_id, "title": title }),
        None => Value::Null,
    })
}

/// Names the terminal's live session after the first prompt typed into it
/// (dispatched tasks are named from their task title server-side).
#[tauri::command]
pub fn session_name_from_task(
    workspace_id: String,
    terminal_session_id: String,
    task: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    let named = name_session_from_first_task(
        &app,
        state.inner(),
        &workspace_id,
        &terminal_session_id,
        &task,
    );
    Ok(json!({ "named": named }))
}

/// Gives the live session on `terminal_session_id` a title derived from its
/// first task, unless it already has one. Best-effort: naming must never fail
/// the dispatch or input that triggered it. Returns whether a title was set.
pub(crate) fn name_session_from_first_task(
    app: &AppHandle,
    state: &AppState,
    workspace_id: &str,
    terminal_session_id: &str,
    task: &str,
) -> bool {
    let Some(title) = derive_session_title_from_task(task) else {
        return false;
    };
    match state.session_registry.name_live_session_if_untitled(
        workspace_id,
        terminal_session_id,
        &title,
    ) {
        Ok(Some(gto_session_id)) => {
            tracing::info!(%gto_session_id, %title, "named session from first task");
            emit_session_updated(
                app,
                workspace_id,
                &gto_session_id,
                Some(terminal_session_id),
                Some(&title),
            );
            true
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(%error, "failed to name session from first task");
            false
        }
    }
}

fn emit_session_updated(
    app: &AppHandle,
    workspace_id: &str,
    gto_session_id: &str,
    terminal_session_id: Option<&str>,
    title: Option<&str>,
) {
    let _ = app.emit(
        "session/updated",
        json!({
            "workspaceId": workspace_id,
            "gtoSessionId": gto_session_id,
            "terminalSessionId": terminal_session_id,
            "title": title,
        }),
    );
}

#[tauri::command]
pub fn session_changefeed_query(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let feed = state
        .session_change_feed
        .lock()
        .map_err(|_| "lock poisoned")?;
    let snapshot = feed.last_snapshot(&workspace_id);
    Ok(json!({ "snapshot": snapshot }))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn session_changefeed_push(
    workspace_id: String,
    branch: String,
    dirty: bool,
    ahead: u32,
    behind: u32,
    staged_files: u32,
    unstaged_files: u32,
    untracked_files: u32,
    revision: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let snapshot = GitStatusSnapshot {
        workspace_id: workspace_id.clone(),
        available: true,
        branch: branch.clone(),
        dirty,
        ahead,
        behind,
        staged_files,
        unstaged_files,
        untracked_files,
        revision,
    };
    let mut feed = state
        .session_change_feed
        .lock()
        .map_err(|_| "lock poisoned")?;
    let items = feed.on_git_updated(&snapshot);
    if !items.is_empty() {
        drop(feed);
        let event = SessionActivityEvent { items };
        let _ = app.emit("gtoffice:session-activity", &event);
        Ok(json!({ "emitted": true }))
    } else {
        Ok(json!({ "emitted": false }))
    }
}
