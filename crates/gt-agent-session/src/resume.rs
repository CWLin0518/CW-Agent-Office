use std::path::Path;

use gt_agent::MaterializedCapability;

use crate::types::{
    GtoSession, Provider, ResumeCheck, ResumeStep, SessionRelaunchMode, SessionStats,
};

pub struct ResumeService;

impl ResumeService {
    /// Build the shell command that starts the provider CLI in resume mode (non-interactive TUI flags).
    ///
    /// `materialized` is what `gt_agent::materialize_claude_capability` /
    /// `materialize_codex_capability` (docs/cw/08_MCP_Hook_Skill掛載設計.md
    /// §2.3/§2.4) wrote for this agent, if the caller already ran the one
    /// matching `session.provider` — `None` means "launch with no
    /// capability overlay," same as before this parameter existed. A
    /// mismatched variant (e.g. `Codex(..)` for a `Provider::Claude`
    /// session) is treated the same as `None` — see `apply_capability_overlay`.
    pub fn build_resume_launch_command(
        session: &GtoSession,
        materialized: Option<&MaterializedCapability>,
    ) -> Option<String> {
        Self::build_relaunch_launch_command(
            Some(session),
            session.provider,
            SessionRelaunchMode::Resume,
            materialized,
        )
    }

    pub fn build_relaunch_launch_command(
        session: Option<&GtoSession>,
        provider: Provider,
        mode: SessionRelaunchMode,
        materialized: Option<&MaterializedCapability>,
    ) -> Option<String> {
        // Codex's overlay (`-p <profile>`) must sit *before* the
        // subcommand, unlike Claude's (appended after — see
        // `apply_capability_overlay`), so it's spliced in here rather than
        // in a uniform post-processing step.
        let codex_prefix = codex_profile_flag(provider, materialized);

        let base = match mode {
            SessionRelaunchMode::ContinueLast => Some(match provider {
                Provider::Claude => "claude --continue".to_string(),
                Provider::Codex => format!("codex{codex_prefix} resume --last"),
            }),
            SessionRelaunchMode::ForkLast => Some(match provider {
                Provider::Claude => "claude --fork-session --continue".to_string(),
                Provider::Codex => format!("codex{codex_prefix} fork --last"),
            }),
            SessionRelaunchMode::Fork => {
                let session = session?;
                if session.provider != provider {
                    return None;
                }
                match provider {
                    Provider::Claude => resolve_provider_session_id(session)
                        .map(|id| format!("claude --fork-session --resume {id}"))
                        .or_else(|| Some("claude --fork-session --continue".to_string())),
                    Provider::Codex => resolve_provider_session_id(session)
                        .map(|id| format!("codex{codex_prefix} fork {id}"))
                        .or_else(|| Some(format!("codex{codex_prefix} fork --last"))),
                }
            }
            SessionRelaunchMode::Resume => {
                let session = session?;
                if session.provider != provider {
                    return None;
                }
                match provider {
                    Provider::Claude => {
                        if let Some(id) = resolve_provider_session_id(session) {
                            Some(format!("claude --resume {id}"))
                        } else {
                            Some("claude --continue".to_string())
                        }
                    }
                    Provider::Codex => {
                        if let Some(id) = resolve_provider_session_id(session) {
                            Some(format!("codex{codex_prefix} resume {id}"))
                        } else {
                            Some(format!("codex{codex_prefix} resume --last"))
                        }
                    }
                }
            }
        };
        base.map(|command| apply_capability_overlay(command, provider, materialized))
    }

    pub fn build_resume_commands(
        session: &GtoSession,
        materialized: Option<&MaterializedCapability>,
    ) -> Vec<ResumeStep> {
        let Some(command) = Self::build_resume_launch_command(session, materialized) else {
            return Vec::new();
        };
        vec![ResumeStep::StartCli { command }]
    }

    pub fn build_relaunch_commands(
        session: Option<&GtoSession>,
        provider: Provider,
        mode: SessionRelaunchMode,
        materialized: Option<&MaterializedCapability>,
    ) -> Vec<ResumeStep> {
        let Some(command) =
            Self::build_relaunch_launch_command(session, provider, mode, materialized)
        else {
            return Vec::new();
        };
        vec![ResumeStep::StartCli { command }]
    }

    /// Applies the same capability overlay used for resume/relaunch
    /// (`--mcp-config`/`--settings` appended for Claude, `-p <profile>`
    /// spliced right after the program name for Codex — see
    /// `apply_capability_overlay`/`codex_profile_flag`) to an
    /// already-built base command. For an agent's very first launch
    /// (`tool_launch` in desktop-tauri), which has no `GtoSession`/resume
    /// mode to build a command through `build_relaunch_launch_command`, but
    /// still needs the identical flag-placement rules materialize
    /// (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.3/§2.4) produces.
    ///
    /// `command` is the program invocation to overlay — either the bare
    /// default (`"claude"`/`"codex"`, no subcommand) or a caller-supplied
    /// custom override that already starts with that same program name
    /// followed by its own extra flags (e.g. `"codex
    /// --dangerously-bypass-approvals-and-sandbox"`). Codex's `-p
    /// <profile>` is spliced in right after the first (program-name) token,
    /// not blindly appended at the end — for the bare-command case those
    /// are identical (nothing follows), but a custom override's trailing
    /// flags must come AFTER `-p`, never before it.
    pub fn apply_capability_overlay_to_command(
        command: String,
        provider: Provider,
        materialized: Option<&MaterializedCapability>,
    ) -> String {
        let flag = codex_profile_flag(provider, materialized);
        let command = if flag.is_empty() {
            command
        } else {
            match command.find(char::is_whitespace) {
                Some(index) => format!("{}{flag}{}", &command[..index], &command[index..]),
                None => format!("{command}{flag}"),
            }
        };
        apply_capability_overlay(command, provider, materialized)
    }

    pub fn validate_resumable(session: &GtoSession) -> ResumeCheck {
        let Some(log_path) = &session.provider_log_path else {
            return ResumeCheck::CanResume;
        };
        let path = Path::new(log_path);
        if !path.exists() {
            return ResumeCheck::LogFileMissing;
        }
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() == 0 => ResumeCheck::LogFileCorrupted,
            Ok(_) => ResumeCheck::CanResume,
            Err(_) => ResumeCheck::LogFileMissing,
        }
    }

    pub fn format_handover_prefix(title: &str, stats: &SessionStats) -> String {
        let last_commit_placeholder = stats.git_end_commit.as_deref().unwrap_or("—");
        crate::git_diff::build_handover_text(title, stats, Some(last_commit_placeholder))
    }
}

/// Appends `--mcp-config`/`--settings` flags for a Claude launch command when
/// materialize (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.3/§2.4 決策1) wrote
/// something to overlay. Also appends `--setting-sources project,local` when
/// `MaterializedPaths::global_capabilities_enabled` is `false`, so the
/// user's real global `~/.claude/settings.json`/`~/.claude/skills/*` stop
/// auto-loading for this agent — see `apply_capability_overlay` below and
/// docs/cw/21_全域Hook_Skill開關設計.md. A no-op for `Provider::Codex` and for `None`.
///
/// This keeps returning a single typed-into-the-terminal shell command line
/// (not a `program + args` tuple, unlike §2.4's literal suggestion) because
/// that's what every existing caller of this function actually consumes —
/// `apps/desktop-web`'s `executeSessionResumeSteps`/auto-launch-on-new-tab
/// flow types this string into a PTY, it never spawns argv directly.
/// Switching to `program + args` here would ripple into that frontend wire
/// contract, which is out of scope for a non-UI phase.
/// `-p <profile>` has to land *before* the subcommand (`codex -p x resume
/// ...`, not `codex resume ... -p x`) — confirmed empirically against the
/// pinned Codex CLI (see `gt_agent::materialize_codex_capability`'s doc
/// comment). Returns a leading-space-prefixed fragment (`" -p name"`) ready
/// to splice right after `"codex"`, or `""` when there's nothing to
/// overlay — a mismatched variant (materialize ran for the other provider,
/// or wasn't run at all) is treated the same as "nothing to overlay" rather
/// than an error, since a stale/missing overlay just means "launch
/// unmounted," not a broken launch.
fn codex_profile_flag(provider: Provider, materialized: Option<&MaterializedCapability>) -> String {
    if provider != Provider::Codex {
        return String::new();
    }
    match materialized {
        Some(MaterializedCapability::Codex(profile)) => {
            format!(" -p {}", profile.profile_name)
        }
        _ => String::new(),
    }
}

fn apply_capability_overlay(
    command: String,
    provider: Provider,
    materialized: Option<&MaterializedCapability>,
) -> String {
    if provider != Provider::Claude {
        return command;
    }
    let Some(MaterializedCapability::Claude(materialized)) = materialized else {
        return command;
    };

    let mut command = command;
    if let Some(path) = materialized.mcp_config_path.as_ref() {
        match quoted_path_arg(path) {
            Some(arg) => command.push_str(&format!(" --mcp-config {arg}")),
            None => tracing::warn!(
                path = %path.display(),
                "materialized mcp-config path contains a double quote; skipping overlay flag"
            ),
        }
    }
    if let Some(path) = materialized.settings_path.as_ref() {
        match quoted_path_arg(path) {
            Some(arg) => command.push_str(&format!(" --settings {arg}")),
            None => tracing::warn!(
                path = %path.display(),
                "materialized settings path contains a double quote; skipping overlay flag"
            ),
        }
    }
    if !materialized.global_capabilities_enabled {
        command.push_str(" --setting-sources project,local");
    }
    command
}

/// Double-quoting is valid for a space-containing path in both PowerShell
/// and POSIX shells (the two this codebase's PTYs actually run — see
/// `crates/gt-terminal`'s shell resolution), which is all a
/// GT-Office-generated `.gtoffice/agents/<id>/runtime/*.json` path ever
/// needs. Returns `None` (caller skips the flag rather than guessing a
/// cross-shell escape) if the path contains a `"`, since that's the one
/// character this simple quoting can't handle safely.
fn quoted_path_arg(path: &Path) -> Option<String> {
    let text = path.to_string_lossy();
    if text.contains('"') {
        return None;
    }
    Some(format!("\"{text}\""))
}

/// Provider session id: explicit field, else Claude jsonl stem / Codex rollout id from path.
pub fn resolve_provider_session_id(session: &GtoSession) -> Option<String> {
    if let Some(id) = session
        .provider_session_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Some(id.to_string());
    }
    session
        .provider_log_path
        .as_ref()
        .and_then(|p| provider_session_id_from_log_path(session.provider, Path::new(p)))
}

fn provider_session_id_from_log_path(provider: Provider, path: &Path) -> Option<String> {
    match provider {
        Provider::Claude => path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty() && s.contains('-')),
        Provider::Codex => {
            let stem = path.file_stem()?.to_string_lossy();
            // rollout-YYYY-MM-DDThh-mm-ss-<uuid> → take segment after last hyphen group
            stem.strip_prefix("rollout-")
                .and_then(|rest| rest.rsplit_once('-').map(|(_, uuid)| uuid.to_string()))
                .or_else(|| Some(stem.to_string()))
                .filter(|s| !s.is_empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GtoSession, Lifecycle, Provider, SessionStats};
    use gt_agent::{MaterializedCapability, MaterializedCodexProfile, MaterializedPaths};
    use std::fs;
    use std::path::PathBuf;

    fn make_session(
        provider: Provider,
        log_path: Option<&str>,
        provider_session_id: Option<&str>,
    ) -> GtoSession {
        GtoSession {
            gto_session_id: "s1".to_string(),
            workspace_id: "ws1".to_string(),
            agent_id: "a1".to_string(),
            station_id: "st1".to_string(),
            provider,
            provider_session_id: provider_session_id.map(|s| s.to_string()),
            provider_log_path: log_path.map(|s| s.to_string()),
            terminal_session_id: None,
            lifecycle: Lifecycle::Stopped,
            title: Some("test".to_string()),
            cwd: "/tmp".to_string(),
            started_at_ms: 0,
            ended_at_ms: None,
            last_activity_at_ms: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn test_claude_resume_command_with_id() {
        let session = make_session(
            Provider::Claude,
            Some("/tmp/550e8400-e29b-41d4-a716-446655440000.jsonl"),
            None,
        );
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, None).as_deref(),
            Some("claude --resume 550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn test_claude_resume_command_continue_fallback() {
        let session = make_session(Provider::Claude, None, None);
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, None).as_deref(),
            Some("claude --continue")
        );
    }

    #[test]
    fn test_codex_resume_command_with_id() {
        let session = make_session(Provider::Codex, None, Some("abc-uuid"));
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, None).as_deref(),
            Some("codex resume abc-uuid")
        );
    }

    #[test]
    fn test_codex_resume_command_last_fallback() {
        let session = make_session(Provider::Codex, None, None);
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, None).as_deref(),
            Some("codex resume --last")
        );
    }

    #[test]
    fn test_build_resume_commands_single_start() {
        let session = make_session(Provider::Codex, None, Some("id1"));
        let steps = ResumeService::build_resume_commands(&session, None);
        assert_eq!(steps.len(), 1);
        assert!(
            matches!(&steps[0], ResumeStep::StartCli { command } if command == "codex resume id1")
        );
    }

    #[test]
    fn test_claude_resume_command_appends_overlay_flags_when_materialized() {
        let session = make_session(
            Provider::Claude,
            Some("/tmp/550e8400-e29b-41d4-a716-446655440000.jsonl"),
            None,
        );
        let materialized = MaterializedCapability::Claude(MaterializedPaths {
            runtime_dir: PathBuf::from("/ws/.gtoffice/agents/a1/runtime"),
            mcp_config_path: Some(PathBuf::from("/ws/.gtoffice/agents/a1/runtime/mcp.json")),
            settings_path: Some(PathBuf::from(
                "/ws/.gtoffice/agents/a1/runtime/settings.json",
            )),
            skills_copied_to: None,
            global_capabilities_enabled: true,
        });
        let command = ResumeService::build_resume_launch_command(&session, Some(&materialized))
            .expect("command");
        assert_eq!(
            command,
            "claude --resume 550e8400-e29b-41d4-a716-446655440000 \
--mcp-config \"/ws/.gtoffice/agents/a1/runtime/mcp.json\" \
--settings \"/ws/.gtoffice/agents/a1/runtime/settings.json\""
        );
    }

    #[test]
    fn test_claude_resume_command_omits_overlay_flags_that_were_not_materialized() {
        let session = make_session(Provider::Claude, None, None);
        let materialized = MaterializedCapability::Claude(MaterializedPaths {
            runtime_dir: PathBuf::from("/ws/.gtoffice/agents/a1/runtime"),
            mcp_config_path: None,
            settings_path: None,
            skills_copied_to: None,
            global_capabilities_enabled: true,
        });
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, Some(&materialized)).as_deref(),
            Some("claude --continue")
        );
    }

    #[test]
    fn test_codex_resume_command_ignores_a_claude_shaped_materialized_value() {
        let session = make_session(Provider::Codex, None, Some("abc-uuid"));
        let materialized = MaterializedCapability::Claude(MaterializedPaths {
            runtime_dir: PathBuf::from("/ws/.gtoffice/agents/a1/runtime"),
            mcp_config_path: Some(PathBuf::from("/ws/.gtoffice/agents/a1/runtime/mcp.json")),
            settings_path: None,
            skills_copied_to: None,
            global_capabilities_enabled: true,
        });
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, Some(&materialized)).as_deref(),
            Some("codex resume abc-uuid")
        );
    }

    #[test]
    fn test_codex_resume_command_inserts_profile_flag_before_subcommand_when_materialized() {
        let session = make_session(Provider::Codex, None, Some("abc-uuid"));
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, Some(&materialized)).as_deref(),
            Some("codex -p gtoffice-agent-a resume abc-uuid")
        );
    }

    #[test]
    fn test_codex_last_fallback_still_inserts_profile_flag_before_subcommand() {
        let session = make_session(Provider::Codex, None, None);
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, Some(&materialized)).as_deref(),
            Some("codex -p gtoffice-agent-a resume --last")
        );
    }

    #[test]
    fn test_claude_resume_command_ignores_a_codex_shaped_materialized_value() {
        let session = make_session(Provider::Claude, None, None);
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        assert_eq!(
            ResumeService::build_resume_launch_command(&session, Some(&materialized)).as_deref(),
            Some("claude --continue")
        );
    }

    #[test]
    fn test_first_launch_overlay_appends_claude_flags_to_bare_command() {
        let materialized = MaterializedCapability::Claude(MaterializedPaths {
            runtime_dir: PathBuf::from("/ws/.gtoffice/agents/a1/runtime"),
            mcp_config_path: Some(PathBuf::from("/ws/.gtoffice/agents/a1/runtime/mcp.json")),
            settings_path: None,
            skills_copied_to: None,
            global_capabilities_enabled: true,
        });
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "claude".to_string(),
                Provider::Claude,
                Some(&materialized),
            ),
            "claude --mcp-config \"/ws/.gtoffice/agents/a1/runtime/mcp.json\""
        );
    }

    #[test]
    fn test_first_launch_overlay_appends_setting_sources_flag_when_global_capabilities_disabled() {
        let materialized = MaterializedCapability::Claude(MaterializedPaths {
            runtime_dir: PathBuf::from("/ws/.gtoffice/agents/a1/runtime"),
            mcp_config_path: None,
            settings_path: None,
            skills_copied_to: None,
            global_capabilities_enabled: false,
        });
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "claude".to_string(),
                Provider::Claude,
                Some(&materialized),
            ),
            "claude --setting-sources project,local"
        );
    }

    #[test]
    fn test_first_launch_overlay_omits_setting_sources_flag_when_global_capabilities_enabled() {
        let materialized = MaterializedCapability::Claude(MaterializedPaths {
            runtime_dir: PathBuf::from("/ws/.gtoffice/agents/a1/runtime"),
            mcp_config_path: None,
            settings_path: None,
            skills_copied_to: None,
            global_capabilities_enabled: true,
        });
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "claude".to_string(),
                Provider::Claude,
                Some(&materialized),
            ),
            "claude"
        );
    }

    #[test]
    fn test_setting_sources_flag_is_never_appended_for_codex() {
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        // A Claude-only flag must never leak onto a Codex launch command,
        // even in the degenerate case where materialize somehow produced a
        // Codex-shaped value — codex_profile_flag/apply_capability_overlay
        // both gate on `provider`, not on which `MaterializedCapability`
        // variant is present.
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "codex".to_string(),
                Provider::Codex,
                Some(&materialized),
            ),
            "codex -p gtoffice-agent-a"
        );
    }

    #[test]
    fn test_first_launch_overlay_inserts_codex_profile_flag_after_bare_command() {
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "codex".to_string(),
                Provider::Codex,
                Some(&materialized),
            ),
            "codex -p gtoffice-agent-a"
        );
    }

    #[test]
    fn test_first_launch_overlay_splices_codex_profile_flag_before_a_custom_command_s_own_trailing_flags(
    ) {
        // A custom `agent.launch_command` override (e.g.
        // `"codex --dangerously-bypass-approvals-and-sandbox"`) already
        // carries its own flags after the program name — `-p <profile>`
        // must land BEFORE those, not appended at the very end (which would
        // put it after the subcommand-position it needs to precede).
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-leader-a1b2c3d4".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-leader-a1b2c3d4.config.toml"),
        });
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "codex --dangerously-bypass-approvals-and-sandbox".to_string(),
                Provider::Codex,
                Some(&materialized),
            ),
            "codex -p gtoffice-leader-a1b2c3d4 --dangerously-bypass-approvals-and-sandbox"
        );
    }

    #[test]
    fn test_first_launch_overlay_is_a_no_op_when_nothing_materialized() {
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "claude".to_string(),
                Provider::Claude,
                None,
            ),
            "claude"
        );
        assert_eq!(
            ResumeService::apply_capability_overlay_to_command(
                "codex".to_string(),
                Provider::Codex,
                None,
            ),
            "codex"
        );
    }

    #[test]
    fn test_validate_resumable_exists() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.jsonl");
        fs::write(&file, "content").unwrap();
        let session = make_session(
            Provider::Claude,
            Some(file.to_string_lossy().as_ref()),
            None,
        );
        assert_eq!(
            ResumeService::validate_resumable(&session),
            ResumeCheck::CanResume
        );
    }

    #[test]
    fn test_validate_resumable_missing() {
        let session = make_session(Provider::Claude, Some("/nonexistent/file.jsonl"), None);
        assert_eq!(
            ResumeService::validate_resumable(&session),
            ResumeCheck::LogFileMissing
        );
    }

    #[test]
    fn test_claude_fork_with_id() {
        let session = make_session(
            Provider::Claude,
            Some("/tmp/550e8400-e29b-41d4-a716-446655440000.jsonl"),
            None,
        );
        assert_eq!(
            ResumeService::build_relaunch_launch_command(
                Some(&session),
                Provider::Claude,
                SessionRelaunchMode::Fork,
                None,
            )
            .as_deref(),
            Some("claude --fork-session --resume 550e8400-e29b-41d4-a716-446655440000")
        );
    }

    #[test]
    fn test_claude_fork_last_global() {
        assert_eq!(
            ResumeService::build_relaunch_launch_command(
                None,
                Provider::Claude,
                SessionRelaunchMode::ForkLast,
                None,
            )
            .as_deref(),
            Some("claude --fork-session --continue")
        );
    }

    #[test]
    fn test_codex_fork_with_id() {
        let session = make_session(Provider::Codex, None, Some("abc-uuid"));
        assert_eq!(
            ResumeService::build_relaunch_launch_command(
                Some(&session),
                Provider::Codex,
                SessionRelaunchMode::Fork,
                None,
            )
            .as_deref(),
            Some("codex fork abc-uuid")
        );
    }

    #[test]
    fn test_codex_continue_last_global() {
        assert_eq!(
            ResumeService::build_relaunch_launch_command(
                None,
                Provider::Codex,
                SessionRelaunchMode::ContinueLast,
                None,
            )
            .as_deref(),
            Some("codex resume --last")
        );
    }

    #[test]
    fn test_codex_continue_last_inserts_profile_flag_when_materialized() {
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        assert_eq!(
            ResumeService::build_relaunch_launch_command(
                None,
                Provider::Codex,
                SessionRelaunchMode::ContinueLast,
                Some(&materialized),
            )
            .as_deref(),
            Some("codex -p gtoffice-agent-a resume --last")
        );
    }

    #[test]
    fn test_codex_fork_last_inserts_profile_flag_when_materialized() {
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        assert_eq!(
            ResumeService::build_relaunch_launch_command(
                None,
                Provider::Codex,
                SessionRelaunchMode::ForkLast,
                Some(&materialized),
            )
            .as_deref(),
            Some("codex -p gtoffice-agent-a fork --last")
        );
    }

    #[test]
    fn test_codex_fork_with_id_inserts_profile_flag_when_materialized() {
        let session = make_session(Provider::Codex, None, Some("abc-uuid"));
        let materialized = MaterializedCapability::Codex(MaterializedCodexProfile {
            profile_name: "gtoffice-agent-a".to_string(),
            profile_path: PathBuf::from("/home/user/.codex/gtoffice-agent-a.config.toml"),
        });
        assert_eq!(
            ResumeService::build_relaunch_launch_command(
                Some(&session),
                Provider::Codex,
                SessionRelaunchMode::Fork,
                Some(&materialized),
            )
            .as_deref(),
            Some("codex -p gtoffice-agent-a fork abc-uuid")
        );
    }

    #[test]
    fn test_handover_prefix() {
        let stats = SessionStats {
            gto_session_id: "s1".to_string(),
            git_start_commit: Some("abc".to_string()),
            git_end_commit: Some("def456 fix: bug".to_string()),
            files_changed: 2,
            insertions: 10,
            deletions: 5,
            commits_ahead: 1,
            updated_at_ms: 0,
        };
        let text = ResumeService::format_handover_prefix("Fix bug", &stats);
        assert!(text.contains("Fix bug"));
        assert!(text.contains("2 files"));
    }
}
