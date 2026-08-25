use std::path::{Path, PathBuf};

use serde::Serialize;

/// Where a `list_available_skills` result came from — surfaced to the UI so
/// the Capabilities → Skills tab can render two separate checklists (project
/// workspace vs. globally installed) instead of one flat list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    Workspace,
    Global,
}

/// One skill found on disk, ready to be checked off in the UI without the
/// user having to hand-type an id + path (docs/cw/09 §1.3 skipped this;
/// this is that gap being filled). `source_path` is the absolute path to the
/// skill's `SKILL.md` — exactly what `SkillCapability.source_path` expects.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_path: String,
    pub scope: SkillScope,
}

/// Scans the two filesystem locations Claude Code itself reads project vs.
/// user-level skills from (docs/cw/08_MCP_Hook_Skill掛載設計.md §1):
/// - workspace-scoped: `<workspace_root>/.claude/skills/<id>/SKILL.md`
/// - globally installed: `$CLAUDE_CONFIG_DIR/skills/<id>/SKILL.md`, falling
///   back to `~/.claude/skills/<id>/SKILL.md` when `CLAUDE_CONFIG_DIR` is
///   unset — the same override Claude Code CLI itself honors.
///
/// Best-effort throughout: a missing root directory yields an empty list for
/// that scope (most workspaces have no `.claude/skills/` at all — that's not
/// an error), and a single unreadable/malformed `SKILL.md` is skipped rather
/// than failing the whole scan.
pub fn list_available_skills(workspace_root: &Path) -> Vec<DiscoveredSkill> {
    let mut skills = scan_skills_dir(
        &workspace_root.join(".claude").join("skills"),
        SkillScope::Workspace,
    );
    if let Some(global_root) = resolve_global_skills_root() {
        skills.extend(scan_skills_dir(&global_root, SkillScope::Global));
    }
    skills
}

fn resolve_global_skills_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("skills"));
        }
    }
    gt_tools::agent_installer::AgentInstaller::user_home_dir()
        .map(|home| home.join(".claude").join("skills"))
}

fn scan_skills_dir(dir: &Path, scope: SkillScope) -> Vec<DiscoveredSkill> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!(?dir, %error, "skills directory not readable; skipping scope");
            return out;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let skill_md_path = path.join("SKILL.md");
        let content = match std::fs::read_to_string(&skill_md_path) {
            Ok(content) => content,
            Err(_) => continue, // no SKILL.md in this directory — not a skill
        };
        let (name, description) = parse_skill_frontmatter(&content);
        out.push(DiscoveredSkill {
            id: id.to_string(),
            name: name.unwrap_or_else(|| id.to_string()),
            description: description.unwrap_or_default(),
            source_path: skill_md_path.to_string_lossy().into_owned(),
            scope,
        });
    }
    out.sort_by_key(|skill| skill.name.to_lowercase());
    out
}

/// Minimal YAML-frontmatter reader — only pulls `name`/`description` as
/// single-line scalars (surrounding quotes stripped). Not a general YAML
/// parser: every real `SKILL.md` (this repo's own `.claude/skills/`, and
/// Claude Code's own convention) writes these two fields as plain
/// `key: value` lines between a pair of `---` markers, and skill content is
/// trusted local disk content, not untrusted input parsed for a security
/// decision.
fn parse_skill_frontmatter(content: &str) -> (Option<String>, Option<String>) {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let mut name = None;
    let mut description = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }
    (name, description)
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// Where a `list_available_hooks` result came from — same "workspace vs.
/// global" split the Hooks sub-tab's checklist uses (mirrors `SkillScope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookScope {
    Workspace,
    Global,
}

/// One hook rule already configured in a `.claude/settings.json` this app
/// didn't write itself — surfaced so the Hooks sub-tab can offer a checklist
/// (docs/cw/11_Skill掛載清單化.md's pattern) instead of only hand-typing a
/// rule. Unlike `DiscoveredSkill`, mounting one of these still goes through
/// the mandatory preview/confirm flow (docs/cw/08_MCP_Hook_Skill掛載設計.md
/// §2.5 決策3) — discovery only removes the retyping, not the confirmation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredHook {
    pub event: String,
    #[serde(default)]
    pub matcher: Option<String>,
    pub command: String,
    pub source_path: String,
    pub scope: HookScope,
    /// Best-effort explanation of what this hook does, read from the leading
    /// comment block of the script `command` invokes — there is nowhere else
    /// to get this from: the real `.claude/settings.json` hook schema has no
    /// description field (confirmed against Claude Code's hooks reference),
    /// so a scanned-but-not-yet-mounted hook would otherwise show nothing
    /// but the raw shell command. `None` whenever `command` doesn't look like
    /// it invokes a script file, the file can't be resolved/read, or the
    /// file has no leading comment — this is a convenience, not a guarantee.
    #[serde(default)]
    pub inferred_description: Option<String>,
}

/// Scans the two `.claude/settings.json` files Claude Code itself reads
/// hooks from — same two roots `list_available_skills` uses (workspace vs.
/// global), pointed at `settings.json` instead of `skills/`:
/// - workspace-scoped: `<workspace_root>/.claude/settings.json`
/// - globally installed: `$CLAUDE_CONFIG_DIR/settings.json`, falling back to
///   `~/.claude/settings.json` when unset
///
/// Only `settings.json` is read, not `settings.local.json` — same accepted
/// scope limitation docs/cw/09_P3.5-capability開發進度.md §1.3 already
/// carries for skills (a hook defined somewhere else needs a hand-typed
/// entry via `HooksEditor`, not a third scan root). Best-effort throughout:
/// a missing/malformed file yields an empty list for that scope rather than
/// an error (most workspaces have no `.claude/settings.json` at all).
pub fn list_available_hooks(workspace_root: &Path) -> Vec<DiscoveredHook> {
    let mut hooks = scan_settings_hooks(
        &workspace_root.join(".claude").join("settings.json"),
        HookScope::Workspace,
        workspace_root,
    );
    if let Some(global_settings) = resolve_global_settings_path() {
        hooks.extend(scan_settings_hooks(
            &global_settings,
            HookScope::Global,
            workspace_root,
        ));
    }
    hooks
}

fn resolve_global_settings_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("settings.json"));
        }
    }
    gt_tools::agent_installer::AgentInstaller::user_home_dir()
        .map(|home| home.join(".claude").join("settings.json"))
}

/// `.claude/settings.json`'s real shape nests hook events under a top-level
/// `"hooks"` key (`{"hooks": {"PreToolUse": [{"matcher": ..., "hooks":
/// [{"type": "command", "command": ...}]}]}}`) — confirmed against Claude
/// Code's published hooks reference. This is a different shape from
/// `materialize.rs`'s `build_settings_json`, which targets the synthetic
/// `--settings <file>` overlay this app generates for itself
/// (`.gtoffice/agents/<agent_id>/runtime/settings.json`) and puts event
/// names directly at the root with no `"hooks"` wrapper — this function
/// reads the user's *real* settings file, not that generated one, so it
/// must use the wrapped shape instead.
fn scan_settings_hooks(
    path: &Path,
    scope: HookScope,
    workspace_root: &Path,
) -> Vec<DiscoveredHook> {
    let mut out = Vec::new();
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            tracing::debug!(?path, %error, "settings file not readable; skipping scope");
            return out;
        }
    };
    let root: serde_json::Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            tracing::debug!(?path, %error, "settings file is not valid JSON; skipping scope");
            return out;
        }
    };
    let Some(events) = root.get("hooks").and_then(|value| value.as_object()) else {
        return out;
    };
    for (event, matcher_entries) in events {
        let Some(matcher_entries) = matcher_entries.as_array() else {
            continue;
        };
        for matcher_entry in matcher_entries {
            let matcher = matcher_entry
                .get("matcher")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let Some(handlers) = matcher_entry
                .get("hooks")
                .and_then(|value| value.as_array())
            else {
                continue;
            };
            for handler in handlers {
                if handler.get("type").and_then(|value| value.as_str()) != Some("command") {
                    continue; // v1 `HookCapability` only models shell-command handlers
                }
                let Some(command) = handler.get("command").and_then(|value| value.as_str()) else {
                    continue;
                };
                out.push(DiscoveredHook {
                    event: event.clone(),
                    matcher: matcher.clone(),
                    command: command.to_string(),
                    source_path: path.to_string_lossy().into_owned(),
                    scope,
                    inferred_description: infer_hook_description(command, workspace_root),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        (
            a.event.as_str(),
            a.matcher.as_deref().unwrap_or(""),
            a.command.as_str(),
        )
            .cmp(&(
                b.event.as_str(),
                b.matcher.as_deref().unwrap_or(""),
                b.command.as_str(),
            ))
    });
    out
}

/// Extensions recognized as "this token is probably a script file, not a
/// flag/argument" when scanning a hook's `command` for something readable.
const SCRIPT_EXTENSIONS: &[&str] = &[".sh", ".bash", ".js", ".mjs", ".cjs", ".py", ".ps1", ".rb"];

/// Best-effort: find a script path in `command`, resolve it against
/// `workspace_root`, and read its leading comment block as a description.
/// Every step is allowed to fail silently (`None`) — this is a convenience
/// for the checklist UI, not something any other logic depends on.
fn infer_hook_description(command: &str, workspace_root: &Path) -> Option<String> {
    let raw_path = extract_script_path(command)?;
    let resolved = resolve_script_path(&raw_path, workspace_root);
    read_leading_comment(&resolved)
}

/// Splits a shell command into tokens, respecting single/double-quoted
/// substrings so a quoted path containing spaces (e.g. `"C:\Users\A B\x.js"`)
/// stays one token. Not a full shell parser (no escape sequences, no
/// variable-aware splitting) — good enough for finding a script path, not
/// for actually running anything.
fn tokenize_command(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in command.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Picks the first token that looks like a script file path — the thing
/// worth reading for a description, as opposed to the interpreter
/// (`node`/`bash`/`python`) or flags in front of it.
fn extract_script_path(command: &str) -> Option<String> {
    tokenize_command(command).into_iter().find(|token| {
        let lower = token.to_lowercase();
        SCRIPT_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
    })
}

/// Expands the placeholders a hook `command` commonly uses to refer to the
/// project/home directory (`${CLAUDE_PROJECT_DIR}` / `$CLAUDE_PROJECT_DIR`,
/// a leading `~`), then resolves the result against `workspace_root` if it's
/// still relative. This is a best-effort guess, not the real substitution
/// Claude Code performs at hook-execution time — a global hook's
/// `${CLAUDE_PROJECT_DIR}` genuinely depends on whichever project is active
/// when it actually runs, and `workspace_root` (the project currently open
/// in GT Office) is the only candidate available while scanning.
///
/// Deliberately joins path segments via `PathBuf::push` instead of string
/// concatenation: `workspace_root` on Windows is typically the output of
/// `canonicalize()` and carries the `\\?\` extended-length prefix, under
/// which Windows refuses to treat `/` as a separator. A hook `command`
/// almost always uses `/` after `$CLAUDE_PROJECT_DIR` (Claude Code's own
/// examples do) — concatenating that onto a `\\?\`-prefixed string yields a
/// path Windows can't resolve, silently breaking every real workspace while
/// a tempdir-based test (no `\\?\` prefix) stays green.
fn resolve_script_path(raw: &str, workspace_root: &Path) -> PathBuf {
    if let Some(rest) = raw
        .strip_prefix("${CLAUDE_PROJECT_DIR}")
        .or_else(|| raw.strip_prefix("$CLAUDE_PROJECT_DIR"))
    {
        return join_segments(workspace_root, rest);
    }
    if let Some(rest) = raw.strip_prefix('~') {
        if let Some(home) = gt_tools::agent_installer::AgentInstaller::user_home_dir() {
            return join_segments(&home, rest);
        }
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        join_segments(workspace_root, raw)
    }
}

/// Appends `rest` onto `base` one path component at a time (splitting on
/// both `/` and `\`), rather than gluing strings together — see
/// `resolve_script_path` for why that distinction matters on Windows.
fn join_segments(base: &Path, rest: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for segment in rest.split(['/', '\\']).filter(|segment| !segment.is_empty()) {
        path.push(segment);
    }
    path
}

/// Cap on how much of a script's leading comment gets surfaced — this is a
/// UI hint, not a document viewer, and a giant comment block would blow out
/// the checklist row's expanded detail panel.
const INFERRED_DESCRIPTION_MAX_CHARS: usize = 500;

/// Reads the contiguous block of comment lines at the top of a script
/// (after an optional shebang), stopping at the first blank or code line.
/// Supports `#` (shell/Python/PowerShell/Ruby), `//`/`///` (JS/TS line
/// comments), and `/* ... */` (JS/TS block comments) — the styles every
/// extension in `SCRIPT_EXTENSIONS` actually uses. Returns `None` for a
/// missing/unreadable file or a file with no leading comment.
fn read_leading_comment(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines().peekable();
    if lines.peek()?.trim_start().starts_with("#!") {
        lines.next();
    }

    let mut collected: Vec<String> = Vec::new();
    let mut in_block_comment = false;
    for line in lines {
        let trimmed = line.trim();
        if in_block_comment {
            match trimmed.strip_suffix("*/") {
                Some(before_close) => {
                    push_non_empty(&mut collected, before_close.trim_start_matches('*').trim());
                    in_block_comment = false;
                }
                None => push_non_empty(&mut collected, trimmed.trim_start_matches('*').trim()),
            }
            continue;
        }
        if trimmed.is_empty() {
            if collected.is_empty() {
                continue; // blank lines before the comment starts don't count
            }
            break; // blank line after the comment block ends it
        }
        if let Some(rest) = trimmed.strip_prefix("/*") {
            match rest.trim().strip_suffix("*/") {
                Some(inline) => push_non_empty(&mut collected, inline.trim()),
                None => {
                    push_non_empty(&mut collected, rest.trim());
                    in_block_comment = true;
                }
            }
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("///")
            .or_else(|| trimmed.strip_prefix("//"))
        {
            collected.push(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            collected.push(rest.trim().to_string());
            continue;
        }
        break; // first non-comment, non-blank line ends the header
    }

    if collected.is_empty() {
        return None;
    }
    Some(truncate_chars(
        &collected.join("\n"),
        INFERRED_DESCRIPTION_MAX_CHARS,
    ))
}

fn push_non_empty(collected: &mut Vec<String>, text: &str) {
    if !text.is_empty() {
        collected.push(text.to_string());
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, id: &str, frontmatter: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), frontmatter).unwrap();
    }

    #[test]
    fn scans_workspace_skills_with_frontmatter() {
        let temp = tempfile::tempdir().unwrap();
        let skills_dir = temp.path().join(".claude").join("skills");
        write_skill(
            &skills_dir,
            "code-review",
            "---\nname: Code Review\ndescription: Reviews a diff for bugs.\n---\nbody\n",
        );

        let found = scan_skills_dir(&skills_dir, SkillScope::Workspace);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "code-review");
        assert_eq!(found[0].name, "Code Review");
        assert_eq!(found[0].description, "Reviews a diff for bugs.");
        assert_eq!(found[0].scope, SkillScope::Workspace);
        assert!(found[0].source_path.ends_with("SKILL.md"));
    }

    #[test]
    fn falls_back_to_directory_name_when_frontmatter_missing_fields() {
        let temp = tempfile::tempdir().unwrap();
        let skills_dir = temp.path().join(".claude").join("skills");
        write_skill(
            &skills_dir,
            "no-frontmatter",
            "just a plain body, no frontmatter\n",
        );

        let found = scan_skills_dir(&skills_dir, SkillScope::Workspace);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "no-frontmatter");
        assert_eq!(found[0].description, "");
    }

    #[test]
    fn missing_skills_directory_yields_empty_not_error() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("does-not-exist");
        assert!(scan_skills_dir(&missing, SkillScope::Workspace).is_empty());
    }

    #[test]
    fn directory_without_skill_md_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let skills_dir = temp.path().join(".claude").join("skills");
        std::fs::create_dir_all(skills_dir.join("empty-dir")).unwrap();

        assert!(scan_skills_dir(&skills_dir, SkillScope::Workspace).is_empty());
    }

    #[test]
    fn list_available_skills_only_scans_workspace_when_no_home_dir_env_set() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(
            &temp.path().join(".claude").join("skills"),
            "local-only",
            "---\nname: Local Only\ndescription: workspace-scoped\n---\n",
        );

        let found = list_available_skills(temp.path());
        assert!(found
            .iter()
            .any(|s| s.id == "local-only" && s.scope == SkillScope::Workspace));
    }

    #[test]
    fn quoted_frontmatter_values_are_unquoted() {
        let temp = tempfile::tempdir().unwrap();
        let skills_dir = temp.path().join(".claude").join("skills");
        write_skill(
            &skills_dir,
            "quoted",
            "---\nname: \"Quoted Name\"\ndescription: 'single quoted'\n---\n",
        );

        let found = scan_skills_dir(&skills_dir, SkillScope::Workspace);
        assert_eq!(found[0].name, "Quoted Name");
        assert_eq!(found[0].description, "single quoted");
    }

    fn write_settings(root: &Path, content: &str) -> PathBuf {
        let dir = root.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn scans_hooks_nested_under_the_hooks_key() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(
            temp.path(),
            r#"{
                "permissions": {"allow": []},
                "hooks": {
                    "PreToolUse": [
                        {
                            "matcher": "Bash",
                            "hooks": [{"type": "command", "command": "echo about-to-run"}]
                        }
                    ]
                }
            }"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].event, "PreToolUse");
        assert_eq!(found[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(found[0].command, "echo about-to-run");
        assert_eq!(found[0].scope, HookScope::Workspace);
        assert!(found[0].source_path.ends_with("settings.json"));
    }

    #[test]
    fn empty_matcher_string_becomes_none() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "echo done"}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].matcher, None);
    }

    #[test]
    fn multiple_handlers_in_one_matcher_entry_become_separate_rows() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"PostToolUse": [{"matcher": "Edit", "hooks": [
                {"type": "command", "command": "echo one"},
                {"type": "command", "command": "echo two"}
            ]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].command, "echo one");
        assert_eq!(found[1].command, "echo two");
    }

    #[test]
    fn non_command_handler_types_are_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "http", "url": "https://example.com"}]}]}}"#,
        );

        assert!(scan_settings_hooks(&path, HookScope::Workspace, temp.path()).is_empty());
    }

    #[test]
    fn missing_settings_file_yields_empty_not_error() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join(".claude").join("settings.json");
        assert!(scan_settings_hooks(&missing, HookScope::Workspace, temp.path()).is_empty());
    }

    #[test]
    fn malformed_json_yields_empty_not_panic() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(temp.path(), "{ not valid json");
        assert!(scan_settings_hooks(&path, HookScope::Workspace, temp.path()).is_empty());
    }

    #[test]
    fn settings_without_a_hooks_key_yields_empty() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(temp.path(), r#"{"permissions": {"allow": []}}"#);
        assert!(scan_settings_hooks(&path, HookScope::Workspace, temp.path()).is_empty());
    }

    #[test]
    fn list_available_hooks_only_scans_workspace_when_no_home_dir_env_set() {
        let temp = tempfile::tempdir().unwrap();
        write_settings(
            temp.path(),
            r#"{"hooks": {"Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "echo bye"}]}]}}"#,
        );

        let found = list_available_hooks(temp.path());
        assert!(found
            .iter()
            .any(|h| h.event == "Stop" && h.scope == HookScope::Workspace));
    }

    #[test]
    fn root_level_event_keys_without_the_hooks_wrapper_are_ignored() {
        // This is the shape `materialize.rs`'s `build_settings_json` writes for
        // its own synthetic `--settings <file>` overlay, not what a real
        // `.claude/settings.json` looks like — must not be misread as hooks.
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        assert!(scan_settings_hooks(&path, HookScope::Workspace, temp.path()).is_empty());
    }

    #[test]
    fn infers_description_from_shell_script_leading_comment() {
        let temp = tempfile::tempdir().unwrap();
        let script_dir = temp.path().join(".claude").join("hooks");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(
            script_dir.join("detect.sh"),
            "#!/usr/bin/env bash\n# Detects when CLAUDE.md changed and reminds the user to review it.\n# Fires on every PostToolUse for Bash/Write/Edit.\n\necho \"actual code starts here\"\n",
        )
        .unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"PostToolUse": [{"matcher": "Bash|Write|Edit", "hooks": [{"type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/detect.sh"}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].inferred_description.as_deref(),
            Some(
                "Detects when CLAUDE.md changed and reminds the user to review it.\nFires on every PostToolUse for Bash/Write/Edit."
            )
        );
    }

    #[test]
    #[cfg(windows)]
    fn infers_description_when_workspace_root_has_the_windows_verbatim_prefix() {
        // `canonicalize()` — what `gt-workspace` actually returns for a real
        // workspace root — prefixes Windows paths with `\\?\`, under which
        // Windows refuses `/` as a separator. A hook `command` almost always
        // uses `/` after `$CLAUDE_PROJECT_DIR`, so this reproduces the real
        // production shape a plain (non-canonicalized) tempdir path can't.
        let temp = tempfile::tempdir().unwrap();
        let canonical_root = temp.path().canonicalize().unwrap();
        assert!(
            canonical_root.to_string_lossy().starts_with(r"\\?\"),
            "expected canonicalize() to add the \\\\?\\ prefix on Windows"
        );
        let script_dir = canonical_root.join(".claude").join("hooks");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(
            script_dir.join("detect.sh"),
            "#!/usr/bin/env bash\n# Detects when CLAUDE.md changed.\n\necho done\n",
        )
        .unwrap();
        let path = write_settings(
            &canonical_root,
            r#"{"hooks": {"PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/detect.sh"}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, &canonical_root);
        assert_eq!(
            found[0].inferred_description.as_deref(),
            Some("Detects when CLAUDE.md changed.")
        );
    }

    #[test]
    fn infers_description_from_js_line_comments_after_skipping_shebang() {
        let temp = tempfile::tempdir().unwrap();
        let script_dir = temp.path().join(".claude").join("hooks");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(
            script_dir.join("check.js"),
            "#!/usr/bin/env node\n// Reminds Claude to ask before writing code.\n// Only fires on UserPromptSubmit.\nconst x = 1\n",
        )
        .unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"UserPromptSubmit": [{"matcher": "", "hooks": [{"type": "command", "command": "node \"${CLAUDE_PROJECT_DIR}/.claude/hooks/check.js\""}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(
            found[0].inferred_description.as_deref(),
            Some("Reminds Claude to ask before writing code.\nOnly fires on UserPromptSubmit.")
        );
    }

    #[test]
    fn infers_description_from_a_block_comment() {
        let temp = tempfile::tempdir().unwrap();
        let script_dir = temp.path().join(".claude").join("hooks");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(
            script_dir.join("check.js"),
            "/*\n * Blocks writes to .env files.\n * Fires on PreToolUse for Write|Edit.\n */\nconst x = 1\n",
        )
        .unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"PreToolUse": [{"matcher": "Write|Edit", "hooks": [{"type": "command", "command": "node .claude/hooks/check.js"}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(
            found[0].inferred_description.as_deref(),
            Some("Blocks writes to .env files.\nFires on PreToolUse for Write|Edit.")
        );
    }

    #[test]
    fn inline_commands_with_no_script_file_have_no_inferred_description() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "echo done"}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(found[0].inferred_description, None);
    }

    #[test]
    fn missing_script_file_yields_no_inferred_description_not_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "bash .claude/hooks/does-not-exist.sh"}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(found[0].inferred_description, None);
    }

    #[test]
    fn script_with_no_leading_comment_yields_no_inferred_description() {
        let temp = tempfile::tempdir().unwrap();
        let script_dir = temp.path().join(".claude").join("hooks");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(script_dir.join("bare.sh"), "echo \"no comment here\"\n").unwrap();
        let path = write_settings(
            temp.path(),
            r#"{"hooks": {"Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "bash .claude/hooks/bare.sh"}]}]}}"#,
        );

        let found = scan_settings_hooks(&path, HookScope::Workspace, temp.path());
        assert_eq!(found[0].inferred_description, None);
    }

    #[test]
    fn tokenize_command_keeps_a_quoted_path_with_spaces_as_one_token() {
        let tokens = tokenize_command(r#"node "C:\Users\A B\hooks\check.js" --flag"#);
        assert_eq!(
            tokens,
            vec![
                "node".to_string(),
                r"C:\Users\A B\hooks\check.js".to_string(),
                "--flag".to_string(),
            ]
        );
    }

    #[test]
    fn extract_script_path_skips_the_interpreter_and_picks_the_script() {
        assert_eq!(
            extract_script_path(r#"node "C:\hooks\check.js""#),
            Some(r"C:\hooks\check.js".to_string())
        );
        assert_eq!(extract_script_path("echo hi"), None);
    }
}
