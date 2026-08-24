use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Ready,
    Paused,
    Blocked,
    Terminated,
}

impl AgentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Ready => "ready",
            AgentState::Paused => "paused",
            AgentState::Blocked => "blocked",
            AgentState::Terminated => "terminated",
        }
    }

    pub fn from_storage_str(value: &str) -> Self {
        match value {
            "paused" => AgentState::Paused,
            "blocked" => AgentState::Blocked,
            "terminated" => AgentState::Terminated,
            _ => AgentState::Ready,
        }
    }
}

/// Where an agent profile is surfaced in the UI. `Station` agents appear in the
/// global workspace-hub station list; `Designer` agents are owned by the
/// business designer and only surface inside the designer pane. Defaults to
/// `Station` for backward compatibility (existing agents predate the field).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentScope {
    #[default]
    Station,
    Designer,
}

impl AgentScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentScope::Station => "station",
            AgentScope::Designer => "designer",
        }
    }

    pub fn from_storage_str(value: &str) -> Self {
        match value {
            "designer" => AgentScope::Designer,
            _ => AgentScope::Station,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub tool: String,
    pub workdir: Option<String>,
    pub custom_workdir: bool,
    #[serde(default)]
    pub scope: AgentScope,
    pub state: AgentState,
    pub employee_no: Option<String>,
    pub policy_snapshot_id: Option<String>,
    /// Points at the most recently saved `agent_capability_snapshots` row
    /// (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.1). `None` = no MCP/skill/hook
    /// mount configured yet.
    #[serde(default)]
    pub capability_snapshot_id: Option<String>,
    pub prompt_file_name: Option<String>,
    pub prompt_file_relative_path: Option<String>,
    pub launch_command: Option<String>,
    pub order_index: i32,
    /// Points at the `AgentProfile.id` of the agent that created this one via
    /// agent-canvas's "new subagent" action. `None` for top-level agents.
    #[serde(default)]
    pub parent_agent_id: Option<String>,
    /// Local filesystem path an agent's prompt content was seeded from at creation
    /// time (see docs/cw/04_客製化設計.md §2). Not workspace-bound, not re-synced.
    #[serde(default)]
    pub external_template_path: Option<String>,
    /// Whether this agent's workdir should stay out of the workspace's
    /// `.gitignore`-managed untracked block. `true` (tracked, the default) means
    /// git sees the agent's files normally; `false` means the workdir is kept
    /// out of version control.
    #[serde(default = "default_git_tracked")]
    pub git_tracked: bool,
    /// Node position on the agent-canvas (docs/cw/04_客製化設計.md §1), in
    /// canvas coordinate space. `None` until the user has dragged the node at
    /// least once; the canvas falls back to an auto-layout in that case.
    #[serde(default)]
    pub layout_x: Option<f64>,
    #[serde(default)]
    pub layout_y: Option<f64>,
    /// Node border color on the agent-canvas (docs/cw/04_客製化設計.md §1,
    /// P4.6), one of a small preset palette. `None` = default gray.
    #[serde(default)]
    pub color: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

fn default_git_tracked() -> bool {
    true
}

/// A connection between two agents on the agent-canvas (docs/cw/04_客製化設計.md
/// §1). `Derived` links are written automatically whenever a `gto send`
/// dispatch succeeds (see local_bridge's dispatch/publish handlers); `Authored`
/// links are user hand-drawn on the canvas and are also what local_bridge's
/// edge-scoped `gto send` authorization checks against (P4.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentLinkKind {
    Authored,
    Derived,
}

impl AgentLinkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentLinkKind::Authored => "authored",
            AgentLinkKind::Derived => "derived",
        }
    }

    pub fn from_storage_str(value: &str) -> Self {
        match value {
            "authored" => AgentLinkKind::Authored,
            _ => AgentLinkKind::Derived,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLink {
    pub id: String,
    pub workspace_id: String,
    pub from_agent_id: String,
    pub to_agent_id: String,
    pub kind: AgentLinkKind,
    /// Wire color preset on agent-canvas (docs/cw/04_客製化設計.md §8, P4.6),
    /// same palette as `AgentProfile::color`. `None` = default gray.
    #[serde(default)]
    pub color: Option<String>,
    /// Whether this authored link renders/behaves as two-way (double
    /// arrowhead) vs one-way (single arrowhead) — a display/state toggle,
    /// not something a reverse drag ever creates (see `create_authored_link`,
    /// which already dedupes a pair to one row regardless of direction).
    /// Meaningless for `Derived`/`kind`-agnostic contexts; defaults to
    /// `false` so every pre-existing authored link keeps today's
    /// single-arrowhead appearance after this column is added.
    #[serde(default)]
    pub bidirectional: bool,
    pub created_at_ms: i64,
}

/// Minimal, canvas-agnostic runtime status projection (docs/cw/05_PRD對齊調研.md
/// "給 P4 的路標"). Deliberately NOT the full Runtime Snapshot / Lifecycle State
/// contract from docs/AGENT_RUNTIME_UPGRADE_PRD.md — this is a small, in-memory
/// summary derived from gt-task's existing runtime registrations, kept
/// independent of both the `agents` table and the canvas UI so it can be
/// upgraded to the full PRD contract later without callers changing shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeState {
    Unknown,
    Offline,
    Idle,
    Active,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeStatus {
    pub agent_id: String,
    pub workspace_id: String,
    pub state: AgentRuntimeState,
    pub updated_at_ms: i64,
}

pub(crate) fn normalize_tool_provider_key(tool: &str) -> &'static str {
    let normalized = tool.trim().to_ascii_lowercase();
    if normalized.contains("claude") {
        "claude"
    } else {
        "codex"
    }
}

pub fn prompt_file_name_for_tool(tool: &str) -> Option<&'static str> {
    match normalize_tool_provider_key(tool) {
        "claude" => Some("CLAUDE.md"),
        "codex" => Some("AGENTS.md"),
        _ => None,
    }
}

pub fn default_prompt_content(agent_name: &str, tool: &str) -> String {
    let file_name = prompt_file_name_for_tool(tool).unwrap_or("AGENTS.md");
    format!(
        "# {agent_name}\n\n这是 {file_name}。\n\n它用于定义这个 Agent 的系统提示词、协作边界和输出偏好。\n你可以直接在这里输入要求；留空时系统会写入这段默认说明。\n"
    )
}

pub fn normalize_agent_slug(value: &str) -> String {
    let lowered = value.trim().to_ascii_lowercase();
    let mut output = String::with_capacity(lowered.len());
    let mut last_was_dash = false;
    for ch in lowered.chars() {
        let allowed =
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-');
        if allowed {
            output.push(ch);
            last_was_dash = false;
            continue;
        }
        if !last_was_dash {
            output.push('-');
            last_was_dash = true;
        }
    }
    let normalized = output.trim_matches('-').to_string();
    if normalized.is_empty() {
        "agent".to_string()
    } else {
        normalized
    }
}

pub fn default_agent_workdir(_name: &str) -> String {
    ".".to_string()
}

pub fn prompt_file_relative_path(workdir: &str, tool: &str) -> Option<String> {
    let file_name = prompt_file_name_for_tool(tool)?;
    let normalized_workdir = workdir.trim().trim_matches('/');
    if normalized_workdir.is_empty() || normalized_workdir == "." {
        return Some(file_name.to_string());
    }
    Some(format!("{normalized_workdir}/{file_name}"))
}

#[cfg(test)]
mod tests {
    use super::{default_agent_workdir, prompt_file_name_for_tool, prompt_file_relative_path};

    #[test]
    fn default_agent_workdir_uses_workspace_root() {
        assert_eq!(default_agent_workdir("My Product Agent"), ".");
        assert_eq!(default_agent_workdir("  "), ".");
    }

    #[test]
    fn prompt_file_metadata_matches_supported_providers() {
        assert_eq!(prompt_file_name_for_tool("claude"), Some("CLAUDE.md"));
        assert_eq!(prompt_file_name_for_tool("codex"), Some("AGENTS.md"));
        assert_eq!(
            prompt_file_relative_path(".gtoffice/alpha", "codex"),
            Some(".gtoffice/alpha/AGENTS.md".to_string())
        );
        assert_eq!(
            prompt_file_relative_path(".", "codex"),
            Some("AGENTS.md".to_string())
        );
    }
}
