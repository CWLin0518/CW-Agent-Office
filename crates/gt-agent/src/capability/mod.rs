use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod discovery;
mod materialize;

pub use discovery::*;
pub use materialize::*;

/// Provider-agnostic "what's mounted" profile for one agent (docs/cw/08_MCP_Hook_Skill掛載設計.md
/// §2.1). Distinct from `AgentPolicy` (is/isn't allowed): this describes what
/// MCP servers / skills / hooks exist to be allowed or denied in the first
/// place. Stored as a JSON blob (`agent_capability_snapshots.capability_json`)
/// rather than normalized tables because the shape of these three things
/// tracks upstream Claude Code / Codex CLI churn (especially hook event
/// types) faster than a migration-per-field would be worth.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilitySnapshot {
    #[serde(default)]
    pub mcp_servers: Vec<McpServerCapability>,
    #[serde(default)]
    pub skills: Vec<SkillCapability>,
    #[serde(default)]
    pub hooks: Vec<HookCapability>,
}

impl AgentCapabilitySnapshot {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Provider-independent validation. Codex now supports both skill
    /// overrides and lifecycle hooks in profile config, so the former
    /// MCP-only restriction no longer applies.
    pub fn validate_for_tool(&self, _tool: &str) -> Result<(), String> {
        for server in &self.mcp_servers {
            server.validate_transport_fields()?;
        }
        self.validate_no_duplicate_enabled_skill_ids()?;

        Ok(())
    }

    /// `SkillCapability.id` doubles as the destination directory name both
    /// for the skills-dir-flag overlay and the copy-fallback path
    /// (`materialize.rs`'s `write_skills_flag_overlay` /
    /// `sync_skills_copy_fallback` both do `skills_dir.join(&skill.id)`).
    /// The UI's checklist (workspace scope + global scope) lets a user
    /// enable two discovered skills that happen to share a directory name —
    /// distinguishable to the UI by `source_path`, but not by `id` — which
    /// would otherwise silently overwrite one skill's content with the
    /// other's at materialize time (last one processed wins) or silently
    /// skip the second one, with no error surfaced anywhere. Disabled
    /// skills never materialize, so only *enabled* ids need to be unique.
    fn validate_no_duplicate_enabled_skill_ids(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for skill in self.skills.iter().filter(|skill| skill.enabled) {
            if !seen.insert(skill.id.as_str()) {
                return Err(format!(
                    "duplicate enabled skill id '{}': two enabled skills cannot share the same \
                     id (e.g. one from the workspace scope, one from the global scope) — they \
                     would overwrite or silently shadow each other when mounted",
                    skill.id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Sse,
    Http,
}

/// Field requirements differ by `transport` and are enforced by
/// `validate_transport_fields`, not by the type system, because the real
/// `.mcp.json` / `--mcp-config` schema itself is transport-tagged this way —
/// confirmed empirically against the pinned Claude Code CLI (`claude mcp add
/// --transport sse ...` writes `{"type":"sse","url":"..."}`, no `command`;
/// `claude mcp add-json` for stdio writes `{"command":...,"args":...,"env":...}`,
/// no `url`/`type`). See docs/cw/08_MCP_Hook_Skill掛載設計.md §2.2's warning
/// not to assume CLI-facing shapes from memory — this one was checked, not
/// guessed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerCapability {
    pub id: String,
    /// User-facing display label only — never written into `mcp.json` /
    /// Codex `config.toml` (those stay keyed by `id`, which the CLIs treat
    /// as the server's identity). `None`/blank means "show `id` instead."
    #[serde(default)]
    pub name: Option<String>,
    pub transport: McpTransport,
    /// Required (non-empty) for `Stdio`; unused for `Sse`/`Http`.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Required (non-empty) for `Sse`/`Http`; unused for `Stdio`.
    #[serde(default)]
    pub url: Option<String>,
    /// When `false`, materialize skips this server entirely (same "absent
    /// = not mounted" semantics as everywhere else in this module) without
    /// dropping it from the snapshot — lets a user temporarily disable a
    /// mount without losing its configuration.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl McpServerCapability {
    pub fn validate_transport_fields(&self) -> Result<(), String> {
        match self.transport {
            McpTransport::Stdio => {
                if Self::is_blank(&self.command) {
                    return Err(format!(
                        "MCP server '{}': stdio transport requires a non-empty `command`",
                        self.id
                    ));
                }
            }
            McpTransport::Sse | McpTransport::Http => {
                if Self::is_blank(&self.url) {
                    return Err(format!(
                        "MCP server '{}': {:?} transport requires a non-empty `url`",
                        self.id, self.transport
                    ));
                }
            }
        }
        Ok(())
    }

    fn is_blank(value: &Option<String>) -> bool {
        value.as_deref().map(str::trim).unwrap_or("").is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCapability {
    pub id: String,
    pub source_path: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// One hook rule. `matcher` follows Claude Code's per-event matcher syntax
/// (tool-name glob, empty = match all); `command` is a user-authored shell
/// command run verbatim, so every write path into this struct MUST go
/// through the preview/confirm flow described in docs/cw/08_MCP_Hook_Skill掛載設計.md
/// §2.5/§3 before being persisted — this struct itself does not enforce that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookCapability {
    pub event: String,
    #[serde(default)]
    pub matcher: Option<String>,
    pub command: String,
    /// Free-text annotation of when this rule fires and what it's for —
    /// purely a UI convenience for the person maintaining the hook list.
    /// Deliberately excluded from `content_hash` (editing a note must not
    /// force re-confirming a hook whose actual behavior didn't change) and
    /// never written by `materialize.rs`'s `build_settings_json` (the real
    /// `.claude/settings.json` hook schema has no such field).
    #[serde(default)]
    pub note: Option<String>,
}

impl HookCapability {
    /// SHA-256 over `event`, `matcher` (empty string when `None` — matches
    /// how `matcher` is already serialized in `materialize`'s
    /// `build_settings_json`), and `command`, each separated by a NUL byte
    /// so e.g. `event="a", command="bc"` can never collide with
    /// `event="ab", command="c"`.
    ///
    /// This is the version-lock identity from docs/cw/08_MCP_Hook_Skill掛載設計.md
    /// §2.5 決策3/§3: a hook's hash must land in `agent_hook_confirmations`
    /// (a persisted table) before it can be saved, so — unlike every other
    /// hash in this design (materialize's cache-busting `DefaultHasher`
    /// uses) — this one has to keep producing the *same* value for the
    /// *same* content across a Rust/std/compiler upgrade. `DefaultHasher`
    /// makes no such guarantee; SHA-256 does.
    pub fn content_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.event.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.matcher.as_deref().unwrap_or("").as_bytes());
        hasher.update([0u8]);
        hasher.update(self.command.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_mcp(id: &str, command: &str) -> McpServerCapability {
        McpServerCapability {
            id: id.to_string(),
            name: None,
            transport: McpTransport::Stdio,
            command: Some(command.to_string()),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            enabled: true,
        }
    }

    #[test]
    fn json_round_trips() {
        let mut snapshot = AgentCapabilitySnapshot::default();
        let mut fs_server = stdio_mcp("fs", "npx");
        fs_server.args = vec!["-y".to_string(), "mcp-server-fs".to_string()];
        snapshot.mcp_servers.push(fs_server);
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/tmp/reviewer/SKILL.md".to_string(),
            enabled: true,
        });
        snapshot.hooks.push(HookCapability {
            event: "PreToolUse".to_string(),
            matcher: Some("Bash".to_string()),
            command: "echo about-to-run-bash".to_string(),
            note: Some("Warns before any Bash command runs".to_string()),
        });

        let json = snapshot.to_json().expect("serialize");
        let restored = AgentCapabilitySnapshot::from_json(&json).expect("deserialize");
        assert_eq!(snapshot, restored);
    }

    #[test]
    fn missing_fields_deserialize_to_defaults() {
        let snapshot = AgentCapabilitySnapshot::from_json("{}").expect("deserialize empty object");
        assert_eq!(snapshot, AgentCapabilitySnapshot::default());
    }

    #[test]
    fn mcp_only_snapshot_is_valid_for_codex() {
        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));
        assert!(snapshot.validate_for_tool("codex").is_ok());
        assert!(snapshot.validate_for_tool("Codex CLI").is_ok());
    }

    #[test]
    fn stdio_transport_requires_command() {
        let mut server = stdio_mcp("fs", "npx");
        server.command = None;
        assert!(server.validate_transport_fields().is_err());

        let mut blank = stdio_mcp("fs", "npx");
        blank.command = Some("   ".to_string());
        assert!(blank.validate_transport_fields().is_err());
    }

    #[test]
    fn sse_and_http_transports_require_url_not_command() {
        let sse_missing_url = McpServerCapability {
            id: "remote".to_string(),
            name: None,
            transport: McpTransport::Sse,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            enabled: true,
        };
        assert!(sse_missing_url.validate_transport_fields().is_err());

        let sse_ok = McpServerCapability {
            url: Some("https://example.com/sse".to_string()),
            ..sse_missing_url
        };
        assert!(sse_ok.validate_transport_fields().is_ok());

        let http_ok = McpServerCapability {
            transport: McpTransport::Http,
            ..sse_ok
        };
        assert!(http_ok.validate_transport_fields().is_ok());
    }

    #[test]
    fn save_rejects_snapshot_with_malformed_mcp_server() {
        let mut snapshot = AgentCapabilitySnapshot::default();
        let mut broken = stdio_mcp("fs", "npx");
        broken.command = None;
        snapshot.mcp_servers.push(broken);
        assert!(snapshot.validate_for_tool("claude").is_err());
    }

    #[test]
    fn rejects_two_enabled_skills_sharing_the_same_id() {
        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/workspace/.claude/skills/reviewer/SKILL.md".to_string(),
            enabled: true,
        });
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/home/user/.claude/skills/reviewer/SKILL.md".to_string(),
            enabled: true,
        });
        let error = snapshot
            .validate_for_tool("claude")
            .expect_err("duplicate enabled skill ids must be rejected");
        assert!(error.contains("reviewer"));
    }

    #[test]
    fn allows_duplicate_id_when_only_one_copy_is_enabled() {
        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/workspace/.claude/skills/reviewer/SKILL.md".to_string(),
            enabled: true,
        });
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/home/user/.claude/skills/reviewer/SKILL.md".to_string(),
            enabled: false,
        });
        assert!(snapshot.validate_for_tool("claude").is_ok());
    }

    #[test]
    fn skills_and_hooks_are_allowed_for_codex() {
        let mut with_skill = AgentCapabilitySnapshot::default();
        with_skill.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/tmp/reviewer/SKILL.md".to_string(),
            enabled: true,
        });
        assert!(with_skill.validate_for_tool("codex").is_ok());

        let mut with_hook = AgentCapabilitySnapshot::default();
        with_hook.hooks.push(HookCapability {
            event: "PreToolUse".to_string(),
            matcher: None,
            command: "echo hi".to_string(),
            note: None,
        });
        assert!(with_hook.validate_for_tool("codex").is_ok());
    }

    #[test]
    fn skills_and_hooks_are_allowed_for_claude() {
        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/tmp/reviewer/SKILL.md".to_string(),
            enabled: true,
        });
        snapshot.hooks.push(HookCapability {
            event: "PreToolUse".to_string(),
            matcher: None,
            command: "echo hi".to_string(),
            note: None,
        });
        assert!(snapshot.validate_for_tool("claude").is_ok());
    }

    #[test]
    fn hook_content_hash_is_deterministic_and_sensitive_to_every_field() {
        let base = HookCapability {
            event: "PreToolUse".to_string(),
            matcher: Some("Bash".to_string()),
            command: "echo hi".to_string(),
            note: None,
        };
        assert_eq!(base.content_hash(), base.content_hash());

        let different_event = HookCapability {
            event: "PostToolUse".to_string(),
            ..base.clone()
        };
        assert_ne!(base.content_hash(), different_event.content_hash());

        let different_matcher = HookCapability {
            matcher: Some("Edit".to_string()),
            ..base.clone()
        };
        assert_ne!(base.content_hash(), different_matcher.content_hash());

        let different_command = HookCapability {
            command: "echo bye".to_string(),
            ..base.clone()
        };
        assert_ne!(base.content_hash(), different_command.content_hash());
    }

    #[test]
    fn hook_content_hash_does_not_collide_across_the_event_command_boundary() {
        let a = HookCapability {
            event: "a".to_string(),
            matcher: None,
            command: "bc".to_string(),
            note: None,
        };
        let b = HookCapability {
            event: "ab".to_string(),
            matcher: None,
            command: "c".to_string(),
            note: None,
        };
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn hook_content_hash_treats_none_matcher_same_as_empty_string_matcher() {
        let none_matcher = HookCapability {
            event: "PreToolUse".to_string(),
            matcher: None,
            command: "echo hi".to_string(),
            note: None,
        };
        let empty_matcher = HookCapability {
            matcher: Some(String::new()),
            ..none_matcher.clone()
        };
        assert_eq!(none_matcher.content_hash(), empty_matcher.content_hash());
    }

    #[test]
    fn hook_content_hash_ignores_note() {
        let undocumented = HookCapability {
            event: "PreToolUse".to_string(),
            matcher: Some("Bash".to_string()),
            command: "echo hi".to_string(),
            note: None,
        };
        let documented = HookCapability {
            note: Some("Warns before any Bash command runs".to_string()),
            ..undocumented.clone()
        };
        assert_eq!(
            undocumented.content_hash(),
            documented.content_hash(),
            "editing a hook's note must not invalidate its version-lock confirmation"
        );
    }
}
