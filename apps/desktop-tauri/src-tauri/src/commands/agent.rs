use std::path::{Path, PathBuf};

use gt_abstractions::{WorkspaceId, WorkspaceService};
use gt_agent::{
    default_agent_workdir, prompt_file_name_for_tool, AgentPolicy, AgentPolicyRepository,
    AgentProfile, AgentRepository, AgentScope, AgentState, CreateAgentInput, UpdateAgentInput,
};
use gt_storage::{SqliteAgentRepository, SqliteStorage};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;

pub(crate) mod binding_cleanup;
pub(crate) mod capability;
pub(crate) mod communication;

use binding_cleanup::{
    apply_direct_agent_binding_cleanup, collect_direct_agent_binding_dependencies,
    DirectBindingCleanupMode,
};

pub(crate) fn to_command_error(error: impl ToString) -> String {
    error.to_string()
}

pub(crate) fn ensure_workspace_exists(state: &AppState, workspace_id: &str) -> Result<(), String> {
    let workspace_id = WorkspaceId::new(workspace_id);
    state
        .workspace_service
        .get_context(&workspace_id)
        .map(|_| ())
        .map_err(to_command_error)
}

pub(crate) fn get_workspace_root(state: &AppState, workspace_id: &str) -> Result<PathBuf, String> {
    let workspace_id = WorkspaceId::new(workspace_id);
    let context = state
        .workspace_service
        .get_context(&workspace_id)
        .map_err(to_command_error)?;
    Ok(PathBuf::from(context.root))
}

pub(crate) fn resolve_agent_repository<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<SqliteAgentRepository, String> {
    let base_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("AGENT_STORAGE_PATH_FAILED: {error}"))?;
    std::fs::create_dir_all(&base_dir)
        .map_err(|error| format!("AGENT_STORAGE_PATH_FAILED: {error}"))?;
    let db_path = base_dir.join("gtoffice.db");
    let storage = SqliteStorage::new(db_path).map_err(to_command_error)?;
    Ok(SqliteAgentRepository::new(storage))
}

pub(crate) fn parse_agent_state(value: Option<String>) -> Result<AgentState, String> {
    match value.as_deref().map(str::trim) {
        None => Ok(AgentState::Ready),
        Some("ready") => Ok(AgentState::Ready),
        Some("paused") => Ok(AgentState::Paused),
        Some("blocked") => Ok(AgentState::Blocked),
        Some("terminated") => Ok(AgentState::Terminated),
        Some(other) => Err(format!("AGENT_STATE_INVALID: {other}")),
    }
}

fn parse_direct_binding_cleanup_mode(
    value: Option<&str>,
    replacement_agent_id: Option<&str>,
) -> Result<Option<DirectBindingCleanupMode>, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("reject") => Ok(None),
        Some("disable") => Ok(Some(DirectBindingCleanupMode::Disable)),
        Some("delete") => Ok(Some(DirectBindingCleanupMode::Delete)),
        Some("rebind") => {
            let replacement_agent_id = replacement_agent_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "CHANNEL_BINDING_REPLACEMENT_AGENT_INVALID: replacementAgentId is required"
                        .to_string()
                })?;
            Ok(Some(DirectBindingCleanupMode::Rebind {
                replacement_agent_id: replacement_agent_id.to_string(),
            }))
        }
        Some(other) => Err(format!("AGENT_DELETE_CLEANUP_MODE_INVALID: {other}")),
    }
}
pub(crate) fn normalize_relative_workdir(value: &str) -> Option<String> {
    let normalized = value.trim().replace('\\', "/").replace("/./", "/");
    if normalized.starts_with('/') || normalized.starts_with('~') || normalized.contains(':') {
        return None;
    }
    let normalized = normalized.trim_matches('/').to_string();
    if normalized.is_empty() || normalized == "." {
        return Some(".".to_string());
    }
    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.is_empty() || segments.contains(&"..") {
        return None;
    }
    Some(segments.join("/"))
}

pub(crate) fn resolve_agent_tool(tool: Option<String>) -> String {
    let normalized = tool.unwrap_or_else(|| "codex".to_string());
    let lowered = normalized.trim().to_ascii_lowercase();
    if lowered.contains("claude") {
        "claude".to_string()
    } else {
        "codex".to_string()
    }
}

pub(crate) fn resolve_update_agent_tool(
    existing_tool: &str,
    requested_tool: Option<String>,
) -> String {
    match requested_tool
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(_) => resolve_agent_tool(requested_tool),
        None => resolve_agent_tool(Some(existing_tool.to_string())),
    }
}

pub(crate) fn resolve_update_agent_prompt_file_name(
    existing_tool: &str,
    requested_tool: Option<&str>,
    existing_prompt_file_name: Option<&str>,
    requested_prompt_file_name: Option<&str>,
) -> Option<String> {
    if let Some(requested_file_name) = requested_prompt_file_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(requested_file_name.to_string());
    }

    let resolved_existing_tool = resolve_agent_tool(Some(existing_tool.to_string()));
    let resolved_requested_tool = requested_tool
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolve_agent_tool(Some(value.to_string())))
        .unwrap_or_else(|| resolved_existing_tool.clone());
    let existing_default = prompt_file_name_for_tool(resolved_existing_tool.as_str());

    match existing_prompt_file_name {
        Some(existing_file_name) if Some(existing_file_name) != existing_default => {
            Some(existing_file_name.to_string())
        }
        _ => prompt_file_name_for_tool(resolved_requested_tool.as_str()).map(str::to_string),
    }
}

pub(crate) fn should_write_prompt_file_on_update(
    existing_tool: &str,
    requested_tool: Option<&str>,
    _existing_prompt_file_name: Option<&str>,
    requested_prompt_file_name: Option<&str>,
    prompt_content: Option<&str>,
    prompt_enabled: bool,
) -> bool {
    if !prompt_enabled {
        return false;
    }

    let requested_tool = requested_tool
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let requested_prompt_file_name = requested_prompt_file_name
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if prompt_content.is_some() || requested_prompt_file_name.is_some() {
        return true;
    }

    let Some(requested_tool) = requested_tool else {
        return false;
    };

    let resolved_existing_tool = resolve_agent_tool(Some(existing_tool.to_string()));
    let resolved_requested_tool = resolve_agent_tool(Some(requested_tool.to_string()));
    resolved_existing_tool != resolved_requested_tool
}

fn resolve_agent_workdir(
    name: &str,
    workdir: Option<String>,
    custom_workdir: bool,
) -> Result<(String, bool), String> {
    let default_workdir = default_agent_workdir(name);
    if custom_workdir {
        let requested = workdir
            .as_deref()
            .and_then(normalize_relative_workdir)
            .ok_or_else(|| "AGENT_WORKDIR_INVALID".to_string())?;
        if requested == default_workdir {
            return Ok((default_workdir, false));
        }
        return Ok((requested, true));
    }
    Ok((default_workdir, false))
}

fn ensure_path_within_workspace(
    workspace_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, String> {
    let normalized = normalize_relative_workdir(relative_path)
        .ok_or_else(|| "AGENT_WORKDIR_INVALID".to_string())?;
    let joined = workspace_root.join(&normalized);
    let canonical_base = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut existing_ancestor = joined.as_path();
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor.parent().unwrap_or(workspace_root);
        if existing_ancestor == workspace_root {
            break;
        }
    }
    let canonical_existing_ancestor = existing_ancestor
        .canonicalize()
        .unwrap_or_else(|_| existing_ancestor.to_path_buf());
    if !canonical_existing_ancestor.starts_with(&canonical_base) {
        return Err("AGENT_WORKDIR_OUTSIDE_WORKSPACE".to_string());
    }
    Ok(joined)
}

fn resolve_prompt_relative_path(workdir: &str, file_name: &str) -> String {
    let normalized_workdir = workdir.trim().trim_matches('/');
    if normalized_workdir.is_empty() || normalized_workdir == "." {
        return file_name.to_string();
    }
    format!("{normalized_workdir}/{file_name}")
}

fn ordered_prompt_file_candidates(tool: &str) -> Vec<&'static str> {
    let mut candidates = Vec::new();
    if let Some(default_file_name) = prompt_file_name_for_tool(tool) {
        candidates.push(default_file_name);
    }
    for candidate in ["CLAUDE.md", "AGENTS.md"] {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

pub(crate) fn resolve_prompt_file_name(
    tool: &str,
    prompt_file_name: Option<&str>,
) -> Result<Option<String>, String> {
    let default = prompt_file_name_for_tool(tool).map(str::to_string);
    match prompt_file_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(default),
        Some(file_name @ ("CLAUDE.md" | "AGENTS.md")) => Ok(Some(file_name.to_string())),
        Some(other) => Err(format!("AGENT_PROMPT_FILE_INVALID: {other}")),
    }
}

pub(crate) fn write_prompt_file(
    workspace_root: &Path,
    workdir: &str,
    tool: &str,
    prompt_file_name: Option<&str>,
    prompt_content: Option<String>,
) -> Result<Option<(String, String)>, String> {
    let Some(file_name) = resolve_prompt_file_name(tool, prompt_file_name)? else {
        return Ok(None);
    };
    let relative_path = resolve_prompt_relative_path(workdir, &file_name);
    let absolute_path = ensure_path_within_workspace(workspace_root, &relative_path)?;
    let workdir_path = absolute_path
        .parent()
        .unwrap_or(workspace_root)
        .to_path_buf();
    std::fs::create_dir_all(&workdir_path)
        .map_err(|error| format!("AGENT_WORKDIR_CREATE_FAILED: {error}"))?;
    let content = prompt_content.unwrap_or_default();
    match std::fs::read_to_string(&absolute_path) {
        Ok(existing_content) if existing_content == content => {
            return Ok(Some((file_name, relative_path)));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("AGENT_PROMPT_READ_FAILED: {error}")),
    }
    std::fs::write(&absolute_path, content)
        .map_err(|error| format!("AGENT_PROMPT_WRITE_FAILED: {error}"))?;
    Ok(Some((file_name, relative_path)))
}

/// Reads the content at an arbitrary local filesystem path, deliberately
/// *without* the workspace-bound checks `ensure_path_within_workspace` does —
/// this path is expected to live outside the workspace (docs/cw/04_客製化設計.md §2).
/// Only existence/file-ness is validated; system error text is never passed
/// through to the caller.
pub(crate) fn read_external_template_content(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("AGENT_EXTERNAL_TEMPLATE_PATH_INVALID".to_string());
    }
    let candidate = Path::new(trimmed);
    if !candidate.exists() {
        return Err("AGENT_EXTERNAL_TEMPLATE_NOT_FOUND".to_string());
    }
    if !candidate.is_file() {
        return Err("AGENT_EXTERNAL_TEMPLATE_NOT_A_FILE".to_string());
    }
    std::fs::read_to_string(candidate)
        .map_err(|_error| "AGENT_EXTERNAL_TEMPLATE_READ_FAILED".to_string())
}

fn strip_managed_output_guidance(content: &str) -> String {
    const START: &str = "<!-- gtoffice:output-guidance:start -->";
    const END: &str = "<!-- gtoffice:output-guidance:end -->";
    if let Some(start) = content.find(START) {
        if let Some(relative_end) = content[start..].find(END) {
            let end = start + relative_end + END.len();
            let before = content[..start].trim_end();
            let after = content[end..].trim_start();
            return match (before.is_empty(), after.is_empty()) {
                (true, true) => String::new(),
                (true, false) => format!("{after}\n"),
                (false, true) => format!("{before}\n"),
                (false, false) => format!("{before}\n\n{after}"),
            };
        }
    }
    let trimmed = content.trim();
    if trimmed.starts_with("# 输出文件")
        && trimmed.contains(".gtoffice/agents/")
        && trimmed.contains("/outputs/")
        && trimmed.contains("会自动出现在 Agent Canvas 该 Agent 节点的「输出」清单节点里")
    {
        return String::new();
    }
    content.to_string()
}

const SESSION_BOUNDARY_GUIDANCE_START: &str = "<!-- gtoffice:session-boundary-guidance:start -->";
const SESSION_BOUNDARY_GUIDANCE_END: &str = "<!-- gtoffice:session-boundary-guidance:end -->";

/// Body text for the managed session-boundary-auto-split guidance block
/// (docs/cw/04_客製化設計.md's "內建自動分 session" direction). Self-contained
/// (doesn't assume the user's `~/.claude/skills/session-boundary-planner`
/// skill is installed) and tool-agnostic (plain prose, not a Claude-only
/// Skill invocation) since this gets written into both Claude and Codex
/// prompt files. Uses a distinct `restart_in_place` signal action — never
/// `new_session` — so it can't collide with the user's global
/// `session-boundary-trigger` PostToolUse hook, which only reacts to
/// `new_session` and opens a separate card instead of restarting this one.
///
/// `SESSION_BOUNDARY_GUIDANCE_START` also doubles as the enablement signal
/// for the user's global `~/.claude/hooks/session-boundary-checklist.js`
/// Stop hook (docs/cw/20_SessionBoundary自動判斷Stop_Hook設計.md — outside
/// this repo, not checked in): that hook only forces a boundary re-check
/// when this marker is present in the station's CLAUDE.md/AGENTS.md. If you
/// change the wording of the boundary rules below, check whether that
/// hook's own copy of the checklist needs the same update.
fn session_boundary_guidance_block() -> String {
    format!(
        "{SESSION_BOUNDARY_GUIDANCE_START}\n\
# 任務邊界自動重啟\n\
\n\
Session 啟動時，先用 `ls -la .claude/session-handoff/` 檢查你目前的工作目錄\n\
下有沒有檔案，有的話看其中**最近修改**的那一個（不論檔名是 `-handoff.md` 還是\n\
已被改名的 `-handoff.consumed.md`——觸發這次重啟的訊號檔通常在你啟動前就已經\n\
被改名過）。如果它的修改時間就在你啟動前不久，代表你是被前一個 session 交接\n\
進來的，直接讀取內容、依裡面的下一階段目標接續執行，不需要等待使用者重新\n\
下達指令；目錄不存在、是空的、或最近的檔案時間對不上，就正常等待使用者輸入。\n\
\n\
在多步驟工作中主動判斷任務邊界，不要只看抽象敘述，逐條核對是否符合具體\n\
情境。\n\
\n\
第一步：這一輪你剛完成的，是不是下列任一種「已完成、且達到穩定狀態」的\n\
產出？只要符合其中一項，就代表已經完成了一個可以交接的獨立段落——不需要\n\
等到整個任務全部做完才算數，多階段任務裡每完成一個獨立階段/子項目都要\n\
重新走一次這個檢查：(a) 完成多步驟計畫、TODO 或使用者交辦清單裡的其中\n\
一項，且產出已經穩定（可執行、測試通過、文件寫完），不論清單上還有沒有\n\
其他項目未做、也不論下一項是不是這個大任務裡「理所當然的下一步」——這點\n\
本身不構成不檢查的理由；(b) 完成一段有明確目的的程式碼變更，處於可編譯/\n\
可執行/可通過既有測試的狀態，即使還沒進到下一個功能；即使改動很小、\n\
耗時很短，只要已達可驗證的穩定狀態也算完整段落，不要因為「事情很小」\n\
跳過檢查；(c) 這一輪回覆的\n\
內容是在處理由另一個 Agent 或終端機透過任務派發指派給你的工作（不是使用\n\
者本人直接輸入的指示），且已完成、手上沒有對方交代的下一步；(d) 從一個\n\
階段換到另一個階段：規劃/設計轉寫程式、寫程式轉測試或 review、除錯完\n\
回頭做原本被中斷的功能；(e) 接下來要做的事跟目前累積的推理過程/上下文\n\
幾乎沒有關聯，一個全新 session 只讀交接檔就能無縫接手。一項都不符合就\n\
還沒到邊界，回覆一句「尚未到達任務邊界，繼續執行」即可。\n\
\n\
第二步：符合第一步任一項時，預設視為到達邊界，只有以下明確定義的例外才\n\
不算——你現在正在改的是同一個函式/同一個 bug 還沒寫完的半成品，中斷會\n\
留下編譯不過或邏輯不完整的狀態，下一個 session 光看交接檔無法還原這個\n\
未完成的中間狀態，一定要接續你腦中還記得的細節才能收尾。「這是大任務裡\n\
邏輯上的下一步」不算例外，即使技術上\n\
相關——例如五步驟重構做完第 2 步，只要第 2 步本身已是可編譯、可獨立驗證\n\
的穩定狀態，第 3 步即使是順理成章的下一步，仍要當作邊界處理。使用者或\n\
派工者已明確要求「這批做完直接接著做下一批、不用交接」時，例外才成立。\n\
情境 (c) 幾乎不該落入這個例外——你完成的工作只存在於你自己這個 session\n\
裡，派工你的 Agent/終端機不會自動知道你做完了，除非你寫出交接訊號讓\n\
GT Office 重啟這個 session，對方會一直以為你還在做，除非派工訊息明講\n\
「完成後留著、不用交接、我會接著丟下一批給你」。\n\
\n\
判定已到達邊界、且目前階段的工作已經穩定完成時，用 Write 工具把交接內容\n\
寫到**你目前的工作目錄**（這個 session 啟動時所在的資料夾，不是整個專案／\n\
repo 的最上層目錄）下的固定相對路徑 `.claude/session-handoff/<phase-slug>-handoff.md`\n\
（`<phase-slug>` 用當前階段的簡短 kebab-case 名稱；目錄不存在就建立），\n\
內容第一行必須是這個固定訊號：\n\
\n\
`<SESSION_CONTROL action=\"restart_in_place\" />`\n\
\n\
訊號下面接著寫：已完成工作、決策、修改的檔案、未解決問題、測試結果、下一\n\
階段目標、限制條件——不要複製對話歷史，只寫下一階段真正需要的資訊。寫出\n\
這個訊號會讓 GT Office 自動終止並重啟這個 agent 的終端機 session，所以只在\n\
真的到達邊界、且交接內容已經確定可信賴時才寫出訊號；單純草擬 handoff 內容\n\
時不要一併寫出訊號。\n\
{SESSION_BOUNDARY_GUIDANCE_END}"
    )
}

fn strip_managed_session_boundary_guidance(content: &str) -> String {
    let Some(start) = content.find(SESSION_BOUNDARY_GUIDANCE_START) else {
        return content.to_string();
    };
    let Some(relative_end) = content[start..].find(SESSION_BOUNDARY_GUIDANCE_END) else {
        return content.to_string();
    };
    let end = start + relative_end + SESSION_BOUNDARY_GUIDANCE_END.len();
    let before = content[..start].trim_end();
    let after = content[end..].trim_start();
    match (before.is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("{after}\n"),
        (false, true) => format!("{before}\n"),
        (false, false) => format!("{before}\n\n{after}"),
    }
}

/// Strips any stale managed block, then re-appends a fresh one when `enabled`.
/// Always returns `Some` (an empty string is a valid, harmless
/// `write_prompt_file` input) so callers don't need to special-case `None`.
fn apply_session_boundary_guidance(content: Option<String>, enabled: bool) -> Option<String> {
    let stripped = strip_managed_session_boundary_guidance(&content.unwrap_or_default());
    if !enabled {
        return Some(stripped);
    }
    let block = session_boundary_guidance_block();
    let composed = if stripped.trim().is_empty() {
        block
    } else {
        format!("{}\n\n{block}", stripped.trim_end())
    };
    Some(composed)
}

fn delete_prompt_file(workspace_root: &Path, relative_path: &str) -> Result<(), String> {
    let absolute_path = ensure_path_within_workspace(workspace_root, relative_path)?;
    match std::fs::remove_file(absolute_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("AGENT_PROMPT_DELETE_FAILED: {error}")),
    }
}

pub(crate) fn read_prompt_file(
    workspace_root: &Path,
    agent: &AgentProfile,
) -> Result<(String, Option<String>, Option<String>), String> {
    if let (Some(relative_path), Some(file_name)) = (
        agent.prompt_file_relative_path.as_deref(),
        agent.prompt_file_name.clone(),
    ) {
        let absolute_path = ensure_path_within_workspace(workspace_root, relative_path)?;
        if absolute_path.exists() {
            let content = std::fs::read_to_string(&absolute_path).unwrap_or_default();
            return Ok((content, Some(file_name), Some(relative_path.to_string())));
        }
    }

    if let Some(workdir) = agent.workdir.as_deref() {
        for candidate in ordered_prompt_file_candidates(agent.tool.as_str()) {
            let relative_path = resolve_prompt_relative_path(workdir, candidate);
            let absolute_path = ensure_path_within_workspace(workspace_root, &relative_path)?;
            if !absolute_path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&absolute_path).unwrap_or_default();
            return Ok((content, Some(candidate.to_string()), Some(relative_path)));
        }
    }

    Ok((String::new(), None, None))
}

fn find_agent(
    repo: &SqliteAgentRepository,
    workspace_id: &str,
    agent_id: &str,
) -> Result<AgentProfile, String> {
    repo.list_agents(workspace_id)
        .map_err(to_command_error)?
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| "AGENT_NOT_FOUND".to_string())
}

#[tauri::command]
pub fn agent_list(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let agents = repo.list_agents(&workspace_id).map_err(to_command_error)?;
    Ok(json!({ "agents": agents }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreateRequest {
    pub workspace_id: String,
    pub agent_id: Option<String>,
    pub name: String,
    pub tool: Option<String>,
    pub workdir: Option<String>,
    pub custom_workdir: Option<bool>,
    pub scope: Option<AgentScope>,
    pub employee_no: Option<String>,
    pub state: Option<String>,
    pub prompt_enabled: Option<bool>,
    pub prompt_file_name: Option<String>,
    pub prompt_content: Option<String>,
    #[serde(default)]
    pub output_collection_enabled: Option<bool>,
    #[serde(default)]
    pub session_boundary_auto_split_enabled: Option<bool>,
    #[serde(default)]
    pub communicate_with_all: Option<bool>,
    pub launch_command: Option<String>,
    #[serde(default)]
    pub external_template_path: Option<String>,
    /// Set when this agent is created via agent-canvas's "new subagent"
    /// context menu action (docs/cw/04_客製化設計.md §1, P4.5). Must name an
    /// existing agent in the same workspace whose policy allows spawning.
    #[serde(default)]
    pub parent_agent_id: Option<String>,
}

pub(crate) fn agent_create_with_repo(
    request: AgentCreateRequest,
    repo: &SqliteAgentRepository,
    workspace_root: &Path,
) -> Result<Value, String> {
    let agent_state = parse_agent_state(request.state)?;
    let tool = resolve_agent_tool(request.tool);
    let name = request.name.trim().to_string();
    let output_collection_enabled = request.output_collection_enabled.unwrap_or(false);
    let session_boundary_auto_split_enabled =
        request.session_boundary_auto_split_enabled.unwrap_or(false);
    let prompt_enabled = request.prompt_enabled.unwrap_or(false);
    let (workdir, custom_workdir) = resolve_agent_workdir(
        name.as_str(),
        request.workdir,
        request.custom_workdir.unwrap_or(false),
    )?;
    ensure_path_within_workspace(workspace_root, &workdir)?;

    let parent_agent_id = request
        .parent_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(parent_id) = parent_agent_id.as_deref() {
        let parent = find_agent(repo, &request.workspace_id, parent_id)?;
        let parent_policy = repo
            .get_agent_policy(&request.workspace_id, &parent.id)
            .unwrap_or_default();
        if !parent_policy.agent.allow_subagent_spawn {
            return Err("AGENT_POLICY_SUBAGENT_SPAWN_DENIED".to_string());
        }
    }

    let requested_external_template_path = request
        .external_template_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    // Materialize the external template's content up front, before touching
    // storage, so a bad path fails the whole create instead of requiring a
    // rollback. Only read it when it will actually be used: prompt writing is
    // enabled and the caller didn't already supply explicit prompt content
    // (e.g. the UI's "load from external path" flow pre-fills promptContent
    // client-side so a user's edits after loading are respected here, not
    // silently overwritten by re-reading the file). `external_template_path`
    // is only persisted on the agent record when it was actually the source
    // of the written content — otherwise it would misrepresent provenance
    // (e.g. prompt writing disabled, or the user typed content by hand
    // without clicking "Load") for a field future stages may treat as "this
    // is where the content came from".
    let (prompt_content, external_template_path) = if prompt_enabled {
        match request.prompt_content.as_deref().map(str::trim) {
            Some(content) if !content.is_empty() => (request.prompt_content.clone(), None),
            _ => match requested_external_template_path.as_deref() {
                Some(path) => (
                    Some(read_external_template_content(path)?),
                    requested_external_template_path.clone(),
                ),
                None => (request.prompt_content.clone(), None),
            },
        }
    } else {
        (request.prompt_content.clone(), None)
    };

    let input = CreateAgentInput {
        workspace_id: request.workspace_id.clone(),
        agent_id: request.agent_id,
        name: name.clone(),
        tool: tool.clone(),
        workdir: Some(workdir.clone()),
        custom_workdir,
        scope: request.scope.unwrap_or_default(),
        employee_no: request.employee_no,
        state: agent_state,
        launch_command: request.launch_command,
        output_collection_enabled,
        session_boundary_auto_split_enabled,
        communicate_with_all: request.communicate_with_all.unwrap_or(false),
        order_index: None,
        parent_agent_id,
        external_template_path,
    };

    let agent = repo.create_agent(input).map_err(to_command_error)?;
    if prompt_enabled {
        let prompt_content =
            apply_session_boundary_guidance(prompt_content, session_boundary_auto_split_enabled);
        if let Err(error) = write_prompt_file(
            workspace_root,
            workdir.as_str(),
            tool.as_str(),
            request.prompt_file_name.as_deref(),
            prompt_content,
        ) {
            let _ = repo.delete_agent(&request.workspace_id, &agent.id);
            return Err(error);
        }
    }
    let refreshed = find_agent(repo, &request.workspace_id, &agent.id)?;
    Ok(json!({ "agent": refreshed }))
}

fn agent_create_with_context<R: tauri::Runtime>(
    request: AgentCreateRequest,
    state: &AppState,
    app: &AppHandle<R>,
) -> Result<Value, String> {
    ensure_workspace_exists(state, &request.workspace_id)?;
    let repo = resolve_agent_repository(app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let workspace_root = get_workspace_root(state, &request.workspace_id)?;
    let workspace_id = request.workspace_id.clone();
    let response = agent_create_with_repo(request, &repo, &workspace_root)?;
    let _ = crate::local_bridge::refresh_directory_snapshot(app, state, &workspace_id);
    Ok(response)
}

#[tauri::command]
pub fn agent_create(
    request: AgentCreateRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    agent_create_with_context(request, state.inner(), &app)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReadExternalTemplateRequest {
    pub external_template_path: String,
}

/// Lets the "load from external path" UI preview a template's content before
/// the user commits to creating the agent. Not workspace-scoped on purpose —
/// see read_external_template_content.
#[tauri::command]
pub fn agent_read_external_template(
    request: AgentReadExternalTemplateRequest,
) -> Result<Value, String> {
    let content = read_external_template_content(&request.external_template_path)?;
    Ok(json!({ "content": content }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUpdateRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub name: String,
    pub tool: Option<String>,
    pub workdir: Option<String>,
    pub custom_workdir: Option<bool>,
    pub employee_no: Option<String>,
    pub state: Option<String>,
    pub prompt_enabled: Option<bool>,
    pub prompt_file_name: Option<String>,
    pub prompt_content: Option<String>,
    #[serde(default)]
    pub output_collection_enabled: Option<bool>,
    #[serde(default)]
    pub session_boundary_auto_split_enabled: Option<bool>,
    #[serde(default)]
    pub communicate_with_all: Option<bool>,
    pub launch_command: Option<String>,
}

pub(crate) fn agent_update_with_repo(
    request: AgentUpdateRequest,
    repo: &SqliteAgentRepository,
    workspace_root: &Path,
) -> Result<Value, String> {
    let existing_agent = find_agent(repo, &request.workspace_id, &request.agent_id)?;
    let agent_state = parse_agent_state(request.state)?;
    let tool = resolve_update_agent_tool(existing_agent.tool.as_str(), request.tool.clone());
    let name = request.name.trim().to_string();
    let (workdir, custom_workdir) = resolve_agent_workdir(
        name.as_str(),
        request.workdir,
        request.custom_workdir.unwrap_or(false),
    )?;
    ensure_path_within_workspace(workspace_root, &workdir)?;
    let (existing_prompt_content, existing_prompt_file_name, existing_prompt_file_relative_path) =
        read_prompt_file(workspace_root, &existing_agent)?;
    let prompt_enabled = request
        .prompt_enabled
        .unwrap_or(existing_prompt_file_name.is_some());
    let session_boundary_auto_split_enabled = request
        .session_boundary_auto_split_enabled
        .unwrap_or(existing_agent.session_boundary_auto_split_enabled);
    let should_write_prompt = ((request.output_collection_enabled.is_some()
        || request.session_boundary_auto_split_enabled.is_some())
        && existing_prompt_file_name.is_some())
        || should_write_prompt_file_on_update(
            existing_agent.tool.as_str(),
            request.tool.as_deref(),
            existing_prompt_file_name.as_deref(),
            request.prompt_file_name.as_deref(),
            request.prompt_content.as_deref(),
            prompt_enabled,
        );
    let prompt_file_name = resolve_update_agent_prompt_file_name(
        existing_agent.tool.as_str(),
        request.tool.as_deref(),
        existing_prompt_file_name.as_deref(),
        request.prompt_file_name.as_deref(),
    );
    let input = UpdateAgentInput {
        workspace_id: request.workspace_id.clone(),
        agent_id: request.agent_id.clone(),
        name: name.clone(),
        tool: tool.clone(),
        workdir: Some(workdir.clone()),
        custom_workdir,
        employee_no: request.employee_no,
        state: agent_state,
        launch_command: request.launch_command,
        output_collection_enabled: request
            .output_collection_enabled
            .unwrap_or(existing_agent.output_collection_enabled),
        session_boundary_auto_split_enabled,
        communicate_with_all: request
            .communicate_with_all
            .unwrap_or(existing_agent.communicate_with_all),
    };

    let agent = repo.update_agent(input).map_err(to_command_error)?;
    if !prompt_enabled {
        if let Some(existing_relative_path) = existing_prompt_file_relative_path.as_deref() {
            delete_prompt_file(workspace_root, existing_relative_path)?;
        }
    } else if should_write_prompt {
        let prompt_content = strip_managed_output_guidance(
            &request
                .prompt_content
                .unwrap_or_else(|| existing_prompt_content.clone()),
        );
        let prompt_content = apply_session_boundary_guidance(
            Some(prompt_content),
            session_boundary_auto_split_enabled,
        );
        let written = write_prompt_file(
            workspace_root,
            workdir.as_str(),
            tool.as_str(),
            prompt_file_name.as_deref(),
            prompt_content,
        )?;
        if let (Some(existing_relative_path), Some((_, written_relative_path))) = (
            existing_prompt_file_relative_path.as_deref(),
            written.as_ref(),
        ) {
            if existing_relative_path != written_relative_path {
                delete_prompt_file(workspace_root, existing_relative_path)?;
            }
        }
    }
    let refreshed = find_agent(repo, &request.workspace_id, &agent.id)?;
    Ok(json!({ "agent": refreshed }))
}

fn agent_update_with_context<R: tauri::Runtime>(
    request: AgentUpdateRequest,
    state: &AppState,
    app: &AppHandle<R>,
) -> Result<Value, String> {
    ensure_workspace_exists(state, &request.workspace_id)?;
    let repo = resolve_agent_repository(app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let workspace_root = get_workspace_root(state, &request.workspace_id)?;
    let workspace_id = request.workspace_id.clone();
    let response = agent_update_with_repo(request, &repo, &workspace_root)?;
    let _ = crate::local_bridge::refresh_directory_snapshot(app, state, &workspace_id);
    Ok(response)
}

#[tauri::command]
pub fn agent_update(
    request: AgentUpdateRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    agent_update_with_context(request, state.inner(), &app)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDeleteRequest {
    pub workspace_id: String,
    pub agent_id: String,
    #[serde(default)]
    pub cleanup_mode: Option<String>,
    #[serde(default)]
    pub replacement_agent_id: Option<String>,
    /// When true, also removes the deleted agent's dedicated workdir
    /// directory from disk (see `delete_agent_workdir`). Never set for the
    /// workspace-root workdir, and skipped by `delete_agent_workdir` itself
    /// if another remaining agent still points at the same directory.
    #[serde(default)]
    pub delete_workdir: bool,
}

pub(crate) fn agent_delete_with_repo<F>(
    request: AgentDeleteRequest,
    state: &AppState,
    repo: &SqliteAgentRepository,
    mut persist_route_bindings: F,
) -> Result<Value, String>
where
    F: FnMut() -> Result<(), String>,
{
    let cleanup_mode = parse_direct_binding_cleanup_mode(
        request.cleanup_mode.as_deref(),
        request.replacement_agent_id.as_deref(),
    )?;
    let blocking_bindings = collect_direct_agent_binding_dependencies(
        &state.task_service,
        &request.workspace_id,
        &request.agent_id,
    );
    if !blocking_bindings.is_empty() && cleanup_mode.is_none() {
        return Ok(json!({
            "deleted": false,
            "errorCode": "AGENT_DELETE_BLOCKED_BY_CHANNEL_BINDINGS",
            "blockingBindings": blocking_bindings,
        }));
    }
    let binding_cleanup = if let Some(cleanup_mode) = cleanup_mode {
        if let DirectBindingCleanupMode::Rebind {
            replacement_agent_id,
        } = &cleanup_mode
        {
            if replacement_agent_id == &request.agent_id {
                return Err(
                    "CHANNEL_BINDING_REPLACEMENT_AGENT_INVALID: replacementAgentId must differ from the deleted agent"
                        .to_string(),
                );
            }
            crate::commands::tool_adapter::validate_binding_target_selector(
                repo,
                &request.workspace_id,
                replacement_agent_id,
            )?;
        }
        let cleanup = apply_direct_agent_binding_cleanup(
            &state.task_service,
            &request.workspace_id,
            &request.agent_id,
            cleanup_mode,
        )?;
        state.task_service.clear_external_idempotency_cache();
        persist_route_bindings()?;
        Some(cleanup)
    } else {
        None
    };
    let deleted = repo
        .delete_agent(&request.workspace_id, &request.agent_id)
        .map_err(to_command_error)?;
    Ok(json!({
        "deleted": deleted,
        "bindingCleanup": binding_cleanup.as_ref().map(|cleanup| json!({
            "matchedCount": cleanup.matched_count,
            "updatedCount": cleanup.updated_count,
            "deletedCount": cleanup.deleted_count,
            "disabledCount": cleanup.disabled_count,
            "reboundToAgentId": cleanup.rebound_to_agent_id,
        })),
        "blockingBindings": Value::Null,
    }))
}

/// Recursively removes a just-deleted agent's dedicated workdir from disk.
/// Never touches the workspace root ("." — the default workdir), and skips
/// deletion entirely if any remaining agent in the workspace still points at
/// the same normalized workdir (e.g. a subagent sharing its parent's
/// directory), so this can never destroy another agent's files.
fn delete_agent_workdir(
    workspace_root: &Path,
    repo: &SqliteAgentRepository,
    workspace_id: &str,
    workdir: &str,
) -> Result<(), String> {
    let Some(normalized) = normalize_relative_workdir(workdir) else {
        return Ok(());
    };
    if normalized == "." {
        return Ok(());
    }
    let still_in_use = repo
        .list_agents(workspace_id)
        .map_err(to_command_error)?
        .into_iter()
        .any(|agent| {
            let agent_workdir = agent
                .workdir
                .clone()
                .unwrap_or_else(|| default_agent_workdir(&agent.name));
            normalize_relative_workdir(&agent_workdir).as_deref() == Some(normalized.as_str())
        });
    if still_in_use {
        return Ok(());
    }
    let absolute_path = ensure_path_within_workspace(workspace_root, &normalized)?;
    match std::fs::remove_dir_all(&absolute_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("AGENT_WORKDIR_DELETE_FAILED: {error}")),
    }
}

fn agent_delete_with_context<R: tauri::Runtime>(
    request: AgentDeleteRequest,
    state: &AppState,
    app: &AppHandle<R>,
) -> Result<Value, String> {
    ensure_workspace_exists(state, &request.workspace_id)?;
    let repo = resolve_agent_repository(app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let workspace_id = request.workspace_id.clone();
    let delete_workdir = request.delete_workdir;
    let target_workdir = if delete_workdir {
        find_agent(&repo, &workspace_id, &request.agent_id)
            .ok()
            .and_then(|agent| agent.workdir)
    } else {
        None
    };
    let response = agent_delete_with_repo(request, state, &repo, || {
        crate::commands::tool_adapter::persist_route_bindings(app, state)
    })?;
    if response.get("deleted").and_then(Value::as_bool) == Some(true) {
        if let Ok(workspace_root) = get_workspace_root(state, &workspace_id) {
            let _ = resync_agent_gitignore(&workspace_root, &repo, &workspace_id);
            if let Some(workdir) = target_workdir {
                let _ = delete_agent_workdir(&workspace_root, &repo, &workspace_id, &workdir);
            }
        }
    }
    let _ = crate::local_bridge::refresh_directory_snapshot(app, state, &workspace_id);
    Ok(response)
}

#[tauri::command]
pub fn agent_delete(
    request: AgentDeleteRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    agent_delete_with_context(request, state.inner(), &app)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPromptReadRequest {
    pub workspace_id: String,
    pub agent_id: String,
}

pub(crate) fn agent_prompt_read_with_repo(
    request: AgentPromptReadRequest,
    repo: &SqliteAgentRepository,
    workspace_root: &Path,
) -> Result<Value, String> {
    let agent = find_agent(repo, &request.workspace_id, &request.agent_id)?;
    let (prompt_content, prompt_file_name, prompt_file_relative_path) =
        read_prompt_file(workspace_root, &agent)?;
    Ok(json!({
        "promptContent": prompt_content,
        "promptFileName": prompt_file_name,
        "promptFileRelativePath": prompt_file_relative_path,
    }))
}

fn agent_prompt_read_with_context<R: tauri::Runtime>(
    request: AgentPromptReadRequest,
    state: &AppState,
    app: &AppHandle<R>,
) -> Result<Value, String> {
    ensure_workspace_exists(state, &request.workspace_id)?;
    let repo = resolve_agent_repository(app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let workspace_root = get_workspace_root(state, &request.workspace_id)?;
    agent_prompt_read_with_repo(request, &repo, &workspace_root)
}

#[tauri::command]
pub fn agent_prompt_read(
    request: AgentPromptReadRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    agent_prompt_read_with_context(request, state.inner(), &app)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReorderRequest {
    pub workspace_id: String,
    pub ordered_agent_ids: Vec<String>,
}

#[tauri::command]
pub fn agent_reorder(
    request: AgentReorderRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    repo.reorder_agents(&request.workspace_id, request.ordered_agent_ids)
        .map_err(to_command_error)?;
    Ok(json!({ "reordered": true }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicyReadRequest {
    pub workspace_id: String,
    pub agent_id: String,
}

/// Phase A (docs/cw/04_客製化設計.md §3): returns `AgentPolicy::default()`
/// (fully permissive) when the agent has no snapshot yet, so a brand new
/// agent's permissions tab shows an accurate "nothing restricted" state
/// rather than an error.
#[tauri::command]
pub fn agent_policy_read(
    request: AgentPolicyReadRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let policy = repo
        .get_agent_policy(&request.workspace_id, &request.agent_id)
        .map_err(to_command_error)?;
    Ok(json!({ "policy": policy }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicySaveRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub policy: AgentPolicy,
}

/// Appends a new immutable snapshot and repoints `agents.policy_snapshot_id`
/// at it — never overwrites a prior snapshot in place (docs/cw/04_客製化設計.md §3).
#[tauri::command]
pub fn agent_policy_save(
    request: AgentPolicySaveRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;
    let snapshot_id = repo
        .save_agent_policy(&request.workspace_id, &request.agent_id, &request.policy)
        .map_err(to_command_error)?;
    Ok(json!({ "snapshotId": snapshot_id }))
}

const AGENT_GITIGNORE_BLOCK_START: &str =
    "# gtoffice:agent-git-tracking:start (managed by GT Office — do not edit)";
const AGENT_GITIGNORE_BLOCK_END: &str = "# gtoffice:agent-git-tracking:end";

/// Rewrites the managed block in the workspace `.gitignore`, replacing it with
/// one `/relative/path/` entry per currently-untracked agent workdir. Runs the
/// full set every call (instead of patching a single line) so the block always
/// reflects `agents.git_tracked` exactly, even if a previous write was
/// interrupted or an agent/workdir was renamed or deleted in between.
fn sync_agent_gitignore_block(
    workspace_root: &Path,
    untracked_relative_paths: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let gitignore_path = workspace_root.join(".gitignore");
    let existing = std::fs::read_to_string(&gitignore_path).unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();
    let mut in_managed_block = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == AGENT_GITIGNORE_BLOCK_START {
            in_managed_block = true;
            continue;
        }
        if trimmed == AGENT_GITIGNORE_BLOCK_END {
            in_managed_block = false;
            continue;
        }
        if in_managed_block {
            continue;
        }
        lines.push(line.to_string());
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    if !untracked_relative_paths.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(AGENT_GITIGNORE_BLOCK_START.to_string());
        for path in untracked_relative_paths {
            lines.push(format!("/{path}/"));
        }
        lines.push(AGENT_GITIGNORE_BLOCK_END.to_string());
    }

    if lines.is_empty() {
        if gitignore_path.exists() {
            std::fs::remove_file(&gitignore_path)
                .map_err(|error| format!("AGENT_GITIGNORE_WRITE_FAILED: {error}"))?;
        }
        return Ok(());
    }

    let mut content = lines.join("\n");
    content.push('\n');
    std::fs::write(&gitignore_path, content)
        .map_err(|error| format!("AGENT_GITIGNORE_WRITE_FAILED: {error}"))
}

/// Recomputes the full untracked-workdir set from `agents.git_tracked` and
/// rewrites the managed `.gitignore` block to match. Shared by the toggle
/// command and by agent deletion, so a deleted untracked agent's entry never
/// lingers as a stray `.gitignore` line.
fn resync_agent_gitignore(
    workspace_root: &Path,
    repo: &SqliteAgentRepository,
    workspace_id: &str,
) -> Result<(), String> {
    let mut untracked_paths = std::collections::BTreeSet::new();
    for agent in repo.list_agents(workspace_id).map_err(to_command_error)? {
        if agent.git_tracked {
            continue;
        }
        let workdir = agent
            .workdir
            .clone()
            .unwrap_or_else(|| default_agent_workdir(&agent.name));
        let Some(normalized) = normalize_relative_workdir(&workdir) else {
            continue;
        };
        if normalized == "." {
            continue;
        }
        if ensure_path_within_workspace(workspace_root, &normalized).is_ok() {
            untracked_paths.insert(normalized);
        }
    }
    sync_agent_gitignore_block(workspace_root, &untracked_paths)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGitTrackingSetRequest {
    pub workspace_id: String,
    pub agent_id: String,
    pub git_tracked: bool,
}

/// Toggles whether an agent's workdir is kept out of the workspace's git
/// history, then resyncs the managed `.gitignore` block so the file on disk
/// can never drift from the database.
#[tauri::command]
pub fn agent_git_tracking_set(
    request: AgentGitTrackingSetRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    ensure_workspace_exists(&state, &request.workspace_id)?;
    let repo = resolve_agent_repository(&app)?;
    repo.ensure_schema().map_err(to_command_error)?;

    let target = repo
        .list_agents(&request.workspace_id)
        .map_err(to_command_error)?
        .into_iter()
        .find(|agent| agent.id == request.agent_id)
        .ok_or_else(|| "AGENT_NOT_FOUND".to_string())?;
    let target_workdir = target
        .workdir
        .clone()
        .unwrap_or_else(|| default_agent_workdir(&target.name));
    if !request.git_tracked && normalize_relative_workdir(&target_workdir).as_deref() == Some(".") {
        return Err("AGENT_GIT_TRACKING_ROOT_WORKDIR_UNSUPPORTED".to_string());
    }

    let updated_agent = repo
        .set_git_tracked(
            &request.workspace_id,
            &request.agent_id,
            request.git_tracked,
        )
        .map_err(to_command_error)?;

    let workspace_root = get_workspace_root(&state, &request.workspace_id)?;
    resync_agent_gitignore(&workspace_root, &repo, &request.workspace_id)?;

    Ok(json!({ "agent": updated_agent }))
}

/// Covers `agent_create_with_repo`'s P4.5 subagent-creation gate
/// (docs/cw/04_客製化設計.md §1) directly against a scratch `SqliteAgentRepository`
/// — bypasses the Tauri `AppHandle`/`AppState` machinery `agent_create_with_context`
/// needs, which isn't available in a unit test.
#[cfg(test)]
mod subagent_creation_tests {
    use super::*;
    use gt_agent::AgentPolicy;
    use std::path::PathBuf;

    struct ScratchRepo {
        _db_path: PathBuf,
        workspace_root: PathBuf,
        repo: SqliteAgentRepository,
    }

    impl Drop for ScratchRepo {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self._db_path.display()));
            }
            let _ = std::fs::remove_dir_all(&self.workspace_root);
        }
    }

    fn scratch_repo_with_parent(name: &str, allow_subagent_spawn: bool) -> ScratchRepo {
        let unique = format!("{name}-{}", uuid::Uuid::new_v4());
        let db_path = std::env::temp_dir().join(format!("gt-agent-cmd-test-{unique}.db"));
        let workspace_root = std::env::temp_dir().join(format!("gt-agent-cmd-test-ws-{unique}"));
        std::fs::create_dir_all(&workspace_root).expect("create scratch workspace root");

        let storage = SqliteStorage::new(&db_path).expect("open scratch storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");

        let parent = repo
            .create_agent(CreateAgentInput {
                workspace_id: "ws-1".to_string(),
                agent_id: Some("parent-agent".to_string()),
                name: "Parent".to_string(),
                tool: "codex".to_string(),
                workdir: Some(".".to_string()),
                custom_workdir: false,
                scope: AgentScope::Station,
                employee_no: None,
                state: AgentState::Ready,
                launch_command: None,
                output_collection_enabled: false,
                session_boundary_auto_split_enabled: false,
                communicate_with_all: false,
                order_index: None,
                parent_agent_id: None,
                external_template_path: None,
            })
            .expect("create parent agent");
        let mut policy = AgentPolicy::default();
        policy.agent.allow_subagent_spawn = allow_subagent_spawn;
        repo.save_agent_policy("ws-1", &parent.id, &policy)
            .expect("save parent policy");

        ScratchRepo {
            _db_path: db_path,
            workspace_root,
            repo,
        }
    }

    fn subagent_request(parent_agent_id: &str) -> AgentCreateRequest {
        AgentCreateRequest {
            workspace_id: "ws-1".to_string(),
            agent_id: None,
            name: "Subagent".to_string(),
            tool: Some("codex".to_string()),
            workdir: None,
            custom_workdir: None,
            scope: None,
            employee_no: None,
            state: None,
            prompt_enabled: None,
            prompt_file_name: None,
            prompt_content: None,
            output_collection_enabled: None,
            session_boundary_auto_split_enabled: None,
            communicate_with_all: None,
            launch_command: None,
            external_template_path: None,
            parent_agent_id: Some(parent_agent_id.to_string()),
        }
    }

    #[test]
    fn subagent_creation_succeeds_when_parent_allows_spawn() {
        let scratch = scratch_repo_with_parent("allowed", true);
        let created = agent_create_with_repo(
            subagent_request("parent-agent"),
            &scratch.repo,
            &scratch.workspace_root,
        )
        .expect("subagent creation should succeed");
        assert_eq!(
            created["agent"]["parentAgentId"].as_str(),
            Some("parent-agent")
        );
    }

    #[test]
    fn subagent_creation_denied_when_parent_policy_disallows_spawn() {
        let scratch = scratch_repo_with_parent("denied", false);
        let error = agent_create_with_repo(
            subagent_request("parent-agent"),
            &scratch.repo,
            &scratch.workspace_root,
        )
        .expect_err("subagent creation should be denied");
        assert_eq!(error, "AGENT_POLICY_SUBAGENT_SPAWN_DENIED");

        let remaining = scratch.repo.list_agents("ws-1").expect("list agents");
        assert_eq!(
            remaining.len(),
            1,
            "a denied create must not leave a partially-created subagent behind"
        );
    }

    #[test]
    fn subagent_creation_fails_for_nonexistent_parent() {
        let scratch = scratch_repo_with_parent("missing-parent", true);
        let error = agent_create_with_repo(
            subagent_request("does-not-exist"),
            &scratch.repo,
            &scratch.workspace_root,
        )
        .expect_err("subagent creation should fail for an unknown parent");
        assert_eq!(error, "AGENT_NOT_FOUND");
    }
}

#[cfg(test)]
mod session_boundary_guidance_tests {
    use super::*;

    #[test]
    fn apply_appends_block_to_existing_content_when_enabled() {
        let result = apply_session_boundary_guidance(Some("# My Agent\n".to_string()), true);
        let content = result.expect("some content");
        assert!(content.starts_with("# My Agent"));
        assert!(content.contains(SESSION_BOUNDARY_GUIDANCE_START));
        assert!(content.contains(SESSION_BOUNDARY_GUIDANCE_END));
        assert!(content.contains("restart_in_place"));
    }

    #[test]
    fn apply_writes_only_the_block_when_content_is_empty() {
        let result = apply_session_boundary_guidance(None, true);
        let content = result.expect("some content");
        assert!(content.starts_with(SESSION_BOUNDARY_GUIDANCE_START));
        assert!(content.trim_end().ends_with(SESSION_BOUNDARY_GUIDANCE_END));
    }

    #[test]
    fn apply_strips_block_and_leaves_other_content_untouched_when_disabled() {
        let with_block = apply_session_boundary_guidance(Some("# My Agent\n".to_string()), true)
            .expect("some content");
        let stripped =
            apply_session_boundary_guidance(Some(with_block), false).expect("some content");
        // strip_managed_session_boundary_guidance's (before non-empty, after
        // empty) branch deliberately keeps a trailing newline after `before`
        // (same convention as strip_managed_output_guidance).
        assert_eq!(stripped, "# My Agent\n");
    }

    #[test]
    fn apply_is_idempotent_across_repeated_toggles() {
        let base = Some("# My Agent\n\nSome other instructions.".to_string());
        let enabled_once = apply_session_boundary_guidance(base, true).expect("some content");
        let enabled_twice = apply_session_boundary_guidance(Some(enabled_once.clone()), true)
            .expect("some content");
        assert_eq!(
            enabled_once, enabled_twice,
            "re-applying while enabled must not duplicate the block"
        );
        assert_eq!(
            enabled_twice
                .matches(SESSION_BOUNDARY_GUIDANCE_START)
                .count(),
            1
        );
    }

    #[test]
    fn strip_leaves_unrelated_content_unchanged() {
        let content = "# My Agent\n\nNo managed block here.";
        assert_eq!(strip_managed_session_boundary_guidance(content), content);
    }
}

/// Covers `delete_agent_workdir` — the fix for agents deleted in GT Office
/// leaving their dedicated `custom_workdir` directory behind on disk.
#[cfg(test)]
mod delete_agent_workdir_tests {
    use super::*;
    use std::path::PathBuf;

    struct ScratchRepo {
        _db_path: PathBuf,
        workspace_root: PathBuf,
        repo: SqliteAgentRepository,
    }

    impl Drop for ScratchRepo {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self._db_path.display()));
            }
            let _ = std::fs::remove_dir_all(&self.workspace_root);
        }
    }

    fn scratch_repo(name: &str) -> ScratchRepo {
        let unique = format!("{name}-{}", uuid::Uuid::new_v4());
        let db_path = std::env::temp_dir().join(format!("gt-agent-workdir-test-{unique}.db"));
        let workspace_root =
            std::env::temp_dir().join(format!("gt-agent-workdir-test-ws-{unique}"));
        std::fs::create_dir_all(&workspace_root).expect("create scratch workspace root");

        let storage = SqliteStorage::new(&db_path).expect("open scratch storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");

        ScratchRepo {
            _db_path: db_path,
            workspace_root,
            repo,
        }
    }

    fn create_agent_with_workdir(
        repo: &SqliteAgentRepository,
        workspace_id: &str,
        id: &str,
        workdir: &str,
    ) {
        repo.create_agent(CreateAgentInput {
            workspace_id: workspace_id.to_string(),
            agent_id: Some(id.to_string()),
            name: id.to_string(),
            tool: "codex".to_string(),
            workdir: Some(workdir.to_string()),
            custom_workdir: workdir != ".",
            scope: AgentScope::Station,
            employee_no: None,
            state: AgentState::Ready,
            launch_command: None,
            output_collection_enabled: false,
            session_boundary_auto_split_enabled: false,
            communicate_with_all: false,
            order_index: None,
            parent_agent_id: None,
            external_template_path: None,
        })
        .expect("create agent");
    }

    #[test]
    fn removes_dedicated_workdir_directory() {
        let scratch = scratch_repo("removes");
        create_agent_with_workdir(&scratch.repo, "ws-1", "alpha", ".gtoffice/alpha");
        let alpha_dir = scratch.workspace_root.join(".gtoffice/alpha");
        std::fs::create_dir_all(&alpha_dir).expect("create agent workdir");
        std::fs::write(alpha_dir.join("CLAUDE.md"), "hello").expect("seed prompt file");
        scratch
            .repo
            .delete_agent("ws-1", "alpha")
            .expect("delete agent record");

        delete_agent_workdir(
            &scratch.workspace_root,
            &scratch.repo,
            "ws-1",
            ".gtoffice/alpha",
        )
        .expect("delete workdir");

        assert!(!alpha_dir.exists());
    }

    #[test]
    fn never_deletes_workspace_root() {
        let scratch = scratch_repo("root-safety");
        let marker = scratch.workspace_root.join("keep-me.txt");
        std::fs::write(&marker, "still here").expect("seed marker file");

        delete_agent_workdir(&scratch.workspace_root, &scratch.repo, "ws-1", ".")
            .expect("no-op delete for root workdir");

        assert!(marker.exists());
    }

    #[test]
    fn skips_deletion_when_another_agent_still_uses_the_same_workdir() {
        let scratch = scratch_repo("shared");
        create_agent_with_workdir(&scratch.repo, "ws-1", "alpha", ".gtoffice/shared");
        create_agent_with_workdir(&scratch.repo, "ws-1", "beta", ".gtoffice/shared");
        let shared_dir = scratch.workspace_root.join(".gtoffice/shared");
        std::fs::create_dir_all(&shared_dir).expect("create shared workdir");
        scratch
            .repo
            .delete_agent("ws-1", "alpha")
            .expect("delete alpha's record");

        delete_agent_workdir(
            &scratch.workspace_root,
            &scratch.repo,
            "ws-1",
            ".gtoffice/shared",
        )
        .expect("skip delete while beta still uses the directory");

        assert!(
            shared_dir.exists(),
            "beta's still-live workdir must survive alpha's deletion"
        );
    }
}
