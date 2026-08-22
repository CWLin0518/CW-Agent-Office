use serde::{Deserialize, Serialize};

/// Phase A of docs/cw/04_客製化設計.md §3: the five permission categories that
/// have a real, already-existing enforcement hook point (File System / Shell /
/// Git / Agent invoke / Execution). Every field defaults to the fully
/// permissive value so an agent with no stored policy snapshot behaves exactly
/// as it did before this feature existed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicy {
    #[serde(default)]
    pub file_system: FileSystemPolicy,
    #[serde(default)]
    pub shell: ShellPolicy,
    #[serde(default)]
    pub git: GitPolicy,
    #[serde(default)]
    pub agent: AgentInvokePolicy,
    #[serde(default)]
    pub execution: ExecutionPolicy,
}

impl AgentPolicy {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// `denied_path_prefixes` entries are matched against the workspace-relative
/// path being accessed; an empty list means no restriction beyond the
/// existing workspace boundary check.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSystemPolicy {
    #[serde(default)]
    pub denied_path_prefixes: Vec<String>,
}

/// `denied_commands` entries are matched against the resolved shell/launch
/// command name (case-insensitive, e.g. "powershell", "bash") a terminal
/// session would start with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellPolicy {
    #[serde(default)]
    pub denied_commands: Vec<String>,
}

/// `denied_subcommands` entries are matched against the git subcommand name
/// (e.g. "push", "push --force"). Only enforced where an agent identity is
/// actually available for a git action — see docs/cw/04_客製化設計.md §3 and
/// the P3 implementation notes on why git actions are otherwise workspace/UI
/// driven, not agent-invoked, in this codebase today.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPolicy {
    #[serde(default)]
    pub denied_subcommands: Vec<String>,
}

/// v1 per docs/cw/04_客製化設計.md §3: a plain switch, not yet tied to
/// authored/derived edges (that upgrade is P4.5, once agent-canvas edges
/// exist).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInvokePolicy {
    #[serde(default = "default_true")]
    pub allow_gto_send: bool,
    #[serde(default = "default_true")]
    pub allow_subagent_spawn: bool,
}

impl Default for AgentInvokePolicy {
    fn default() -> Self {
        Self {
            allow_gto_send: true,
            allow_subagent_spawn: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPolicy {
    pub timeout_seconds: Option<u64>,
    pub max_steps: Option<u32>,
    pub max_concurrency: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_fully_permissive() {
        let policy = AgentPolicy::default();
        assert!(policy.file_system.denied_path_prefixes.is_empty());
        assert!(policy.shell.denied_commands.is_empty());
        assert!(policy.git.denied_subcommands.is_empty());
        assert!(policy.agent.allow_gto_send);
        assert!(policy.agent.allow_subagent_spawn);
        assert_eq!(policy.execution.timeout_seconds, None);
        assert_eq!(policy.execution.max_steps, None);
        assert_eq!(policy.execution.max_concurrency, None);
    }

    #[test]
    fn json_round_trips() {
        let mut policy = AgentPolicy::default();
        policy.shell.denied_commands.push("powershell".to_string());
        policy.agent.allow_subagent_spawn = false;
        policy.execution.timeout_seconds = Some(120);

        let json = policy.to_json().expect("serialize");
        let restored = AgentPolicy::from_json(&json).expect("deserialize");
        assert_eq!(policy, restored);
    }

    #[test]
    fn missing_fields_deserialize_to_defaults() {
        let policy = AgentPolicy::from_json("{}").expect("deserialize empty object");
        assert_eq!(policy, AgentPolicy::default());
    }
}
