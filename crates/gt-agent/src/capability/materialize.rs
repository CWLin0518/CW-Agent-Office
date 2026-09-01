use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use gt_tools::agent_installer::ProviderCapabilitySupport;
use serde_json::{json, Value};

use super::{AgentCapabilitySnapshot, HookCapability, McpServerCapability, McpTransport};

/// Where materialize() wrote things, for `resume.rs` (docs/cw/08_MCP_Hook_Skill掛載設計.md
/// §2.4) to fold into the launch command. `None` on a field means "this
/// category had nothing to mount, or the CLI can't take an overlay for it" —
/// either way, the caller must not fabricate a flag for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterializedPaths {
    /// `.gtoffice/agents/<agent_id>/runtime/` — always computed, even when
    /// nothing was written into it, so callers have a stable place to look.
    pub runtime_dir: PathBuf,
    /// Set only when `mcpServers` was non-empty AND the CLI supports
    /// `--mcp-config` (no safe fallback exists for MCP — see §2.4 決策 1).
    pub mcp_config_path: Option<PathBuf>,
    /// Set only when `hooks` was non-empty AND the CLI supports `--settings`.
    pub settings_path: Option<PathBuf>,
    /// Set only when skills were copy-fallback'd into the agent's own
    /// workdir (`<workdir>/.claude/skills`) — i.e. when
    /// `supports_skills_dir_flag` was false and at least one skill is
    /// enabled. `None` when nothing needed copying, including when a
    /// skills-dir flag existed and was used instead (no copy happened).
    pub skills_copied_to: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum MaterializeError {
    #[error("invalid capability snapshot: {0}")]
    InvalidSnapshot(String),
    #[error("io error writing {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn io_err(path: &Path, source: io::Error) -> MaterializeError {
    MaterializeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

const MANAGED_SKILLS_MANIFEST_FILE: &str = ".gtoffice-managed-skills.json";
const CAPABILITY_HASH_FILE: &str = ".capability-hash";

/// Claude-only materialize path (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.3/§2.4
/// 決策1). Codex's path is `materialize_codex_capability` below — much
/// simpler than §2.4 決策2's design-time sketch (CODEX_HOME isolation +
/// config.toml merge) once verified against the real CLI; see that
/// function's doc comment for why. This function must not be called for a
/// Codex agent (callers should have already rejected skills/hooks for Codex
/// via `AgentCapabilitySnapshot::validate_for_tool`, but this function only
/// ever emits `--mcp-config`/`--settings`/skills output, so calling it for a
/// Codex agent wouldn't be wrong, just pointless — Codex doesn't read these
/// flags).
///
/// `workspace_root` is the filesystem root of the workspace (`runtime_dir` is
/// computed under `<workspace_root>/.gtoffice/agents/<agent_id>/runtime/`).
/// `agent_workdir` is that agent's own project directory, used only for the
/// skills copy-fallback target (`<agent_workdir>/.claude/skills/`).
///
/// Content-hash short-circuit + per-agent lock (§2.6, 決策4): concurrent
/// calls for the *same* `agent_id` serialize on an in-memory per-agent lock;
/// calls for *different* agents never block each other. Within the lock, if
/// the hash of `(snapshot, support)` matches what's recorded in
/// `<runtime_dir>/.capability-hash`, this returns the previously-computed
/// paths without touching the filesystem again.
pub fn materialize_claude_capability(
    workspace_root: &Path,
    agent_id: &str,
    agent_workdir: &Path,
    snapshot: &AgentCapabilitySnapshot,
    support: &ProviderCapabilitySupport,
) -> Result<MaterializedPaths, MaterializeError> {
    snapshot
        .validate_for_tool("claude")
        .map_err(MaterializeError::InvalidSnapshot)?;
    for skill in &snapshot.skills {
        validate_skill_id(&skill.id).map_err(MaterializeError::InvalidSnapshot)?;
    }

    let lock = agent_lock(agent_id);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let runtime_dir = workspace_root
        .join(".gtoffice")
        .join("agents")
        .join(agent_id)
        .join("runtime");
    let content_hash = compute_content_hash(agent_workdir, snapshot, support);

    if let Some(cached) =
        read_cached_paths_if_hash_matches(&runtime_dir, agent_workdir, &content_hash)
    {
        return Ok(cached);
    }

    std::fs::create_dir_all(&runtime_dir).map_err(|error| io_err(&runtime_dir, error))?;

    let enabled_mcp_server_count = snapshot.mcp_servers.iter().filter(|s| s.enabled).count();
    let mcp_config_path = if enabled_mcp_server_count > 0 {
        if support.supports_mcp_config_flag {
            let path = runtime_dir.join("mcp.json");
            write_atomic(
                &path,
                &build_mcp_config_json(&snapshot.mcp_servers).to_string(),
            )?;
            Some(path)
        } else {
            tracing::warn!(
                agent_id,
                "agent has {} enabled MCP server(s) configured but the pinned Claude Code CLI \
                 does not support --mcp-config; skipping (no safe fallback exists for MCP \
                 overlays)",
                enabled_mcp_server_count
            );
            None
        }
    } else {
        None
    };

    let settings_path = if !snapshot.hooks.is_empty() {
        if support.supports_settings_flag {
            let path = runtime_dir.join("settings.json");
            write_atomic(&path, &build_settings_json(&snapshot.hooks).to_string())?;
            Some(path)
        } else {
            tracing::warn!(
                agent_id,
                "agent has {} hook(s) configured but the pinned Claude Code CLI does not support \
                 --settings; skipping (no safe fallback exists for hooks)",
                snapshot.hooks.len()
            );
            None
        }
    } else {
        None
    };

    let skills_copied_to = if support.supports_skills_dir_flag {
        // Flag-overlay path: write directly into the runtime dir; nothing
        // copied into the agent's own workdir.
        let skills_dir = runtime_dir.join("skills");
        write_skills_flag_overlay(&skills_dir, snapshot)?;
        None
    } else {
        sync_skills_copy_fallback(agent_workdir, snapshot)?
    };

    write_atomic(&runtime_dir.join(CAPABILITY_HASH_FILE), &content_hash)?;

    Ok(MaterializedPaths {
        runtime_dir,
        mcp_config_path,
        settings_path,
        skills_copied_to,
    })
}

/// Keyed by whatever uniquely identifies the *output* a caller is about to
/// write, not necessarily the raw `agent_id` — the Claude path passes
/// `agent_id` directly (its `runtime_dir` is already workspace-scoped via
/// `workspace_root`, so `agent_id` alone is a safe lock domain there); the
/// Codex path passes the fully-disambiguated `profile_name` instead (see
/// `materialize_codex_capability`), because two different `(workspace_id,
/// agent_id)` pairs could otherwise share a lossy `normalize_agent_slug`
/// output and race on the same global `$CODEX_HOME` file with two different
/// locks protecting nothing.
fn agent_lock(lock_key: &str) -> Arc<Mutex<()>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.entry(lock_key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Covers every input that affects materialize's output, not just the
/// snapshot/support pair — `agent_workdir` in particular, because the
/// copy-fallback skills target lives under it (§2.4 決策1). Missing it here
/// previously let a workdir change alone (snapshot/support unchanged) hit
/// the cache and silently skip re-copying skills into the new workdir — a
/// stale-cache "fake mount" bug, not a hypothetical one.
fn compute_content_hash(
    agent_workdir: &Path,
    snapshot: &AgentCapabilitySnapshot,
    support: &ProviderCapabilitySupport,
) -> String {
    let snapshot_json = snapshot.to_json().unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    agent_workdir.to_string_lossy().hash(&mut hasher);
    snapshot_json.hash(&mut hasher);
    support.supports_mcp_config_flag.hash(&mut hasher);
    support.supports_settings_flag.hash(&mut hasher);
    support.supports_skills_dir_flag.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Reconstructs what a previous materialize call would have returned,
/// without re-deriving it from disk state beyond existence checks — the hash
/// match already tells us the inputs haven't changed, so what *should* exist
/// is fully determined by `snapshot`/`support`, not by re-reading the
/// written files' contents.
fn read_cached_paths_if_hash_matches(
    runtime_dir: &Path,
    agent_workdir: &Path,
    expected_hash: &str,
) -> Option<MaterializedPaths> {
    let recorded = std::fs::read_to_string(runtime_dir.join(CAPABILITY_HASH_FILE)).ok()?;
    if recorded.trim() != expected_hash {
        return None;
    }

    let mcp_config_path = runtime_dir.join("mcp.json");
    let settings_path = runtime_dir.join("settings.json");
    let skills_copy_dir = agent_workdir.join(".claude").join("skills");

    Some(MaterializedPaths {
        runtime_dir: runtime_dir.to_path_buf(),
        mcp_config_path: mcp_config_path.is_file().then_some(mcp_config_path),
        settings_path: settings_path.is_file().then_some(settings_path),
        skills_copied_to: skills_copy_dir.is_dir().then_some(skills_copy_dir),
    })
}

fn build_mcp_config_json(servers: &[McpServerCapability]) -> Value {
    let mut mcp_servers = serde_json::Map::new();
    for server in servers.iter().filter(|server| server.enabled) {
        let entry = match server.transport {
            McpTransport::Stdio => json!({
                "command": server.command.clone().unwrap_or_default(),
                "args": server.args,
                "env": server.env,
            }),
            McpTransport::Sse => json!({
                "type": "sse",
                "url": server.url.clone().unwrap_or_default(),
            }),
            McpTransport::Http => json!({
                "type": "http",
                "url": server.url.clone().unwrap_or_default(),
            }),
        };
        mcp_servers.insert(server.id.clone(), entry);
    }
    json!({ "mcpServers": mcp_servers })
}

/// Claude Code hooks schema: each event name maps to a list of `{matcher,
/// hooks: [{type: "command", command}]}` entries. One `HookCapability`
/// becomes one entry (matchers are not deduped/merged across rules —
/// simplest correct mapping, not an optimization target).
///
/// **No `"hooks"` wrapper key at the root** — confirmed against the
/// CLI-bundled `plugin-dev` skill shipped with the pinned Claude Code
/// install (`~/.claude/plugins/marketplaces/claude-plugins-official/plugins/
/// plugin-dev/skills/hook-development/SKILL.md`, "Settings Format (Direct)"
/// section): a *plugin's* `hooks.json` wraps events in `{"hooks": {...}}`,
/// but `.claude/settings.json` — which is what `--settings <file>` loads —
/// puts event names directly at the top level, e.g. `{"PreToolUse": [...]}`.
/// This function targets the settings-file shape, not the plugin shape.
/// Event name validity (`PreToolUse`/`PostToolUse`/`UserPromptSubmit`/`Stop`/
/// `SubagentStop`/`SessionStart`/`SessionEnd`/`PreCompact`/`Notification`)
/// and the `matcher`+`hooks[].type`+`hooks[].command` shape are cross-checked
/// against that same bundle's `validate-hook-schema.sh`.
fn build_settings_json(hooks: &[HookCapability]) -> Value {
    let mut by_event: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for hook in hooks {
        by_event
            .entry(hook.event.as_str())
            .or_default()
            .push(json!({
                "matcher": hook.matcher.clone().unwrap_or_default(),
                "hooks": [
                    { "type": "command", "command": hook.command }
                ],
            }));
    }
    Value::Object(
        by_event
            .into_iter()
            .map(|(k, v)| (k.to_string(), Value::Array(v)))
            .collect(),
    )
}

/// `skill.id` gets joined straight into a filesystem path in both
/// `write_skills_flag_overlay` and `sync_skills_copy_fallback` (the latter
/// also `remove_dir_all`s a path built the same way when clearing a stale
/// managed skill) — this is the last line of defense against a `..` or
/// absolute-path id escaping `skills_dir`/`.claude/skills`, regardless of
/// whether an upstream validator (repository save, future UI) already
/// checked it. Also blocks a leading `.` so a skill id can never collide
/// with `MANAGED_SKILLS_MANIFEST_FILE`/`CAPABILITY_HASH_FILE`.
fn validate_skill_id(id: &str) -> Result<(), String> {
    let is_safe_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.';
    if !id.is_empty() && !id.starts_with('.') && id.chars().all(is_safe_char) {
        Ok(())
    } else {
        Err(format!(
            "skill id '{id}' is not a safe path segment (must be non-empty, ASCII \
             alphanumeric/-/_/. only, and not start with '.')"
        ))
    }
}

fn write_skills_flag_overlay(
    skills_dir: &Path,
    snapshot: &AgentCapabilitySnapshot,
) -> Result<(), MaterializeError> {
    if skills_dir.exists() {
        std::fs::remove_dir_all(skills_dir).map_err(|error| io_err(skills_dir, error))?;
    }
    for skill in snapshot.skills.iter().filter(|skill| skill.enabled) {
        let dest_dir = skills_dir.join(&skill.id);
        std::fs::create_dir_all(&dest_dir).map_err(|error| io_err(&dest_dir, error))?;
        let dest_file = dest_dir.join("SKILL.md");
        let content = std::fs::read_to_string(&skill.source_path)
            .map_err(|error| io_err(Path::new(&skill.source_path), error))?;
        std::fs::write(&dest_file, content).map_err(|error| io_err(&dest_file, error))?;
    }
    Ok(())
}

/// Copy-fallback path (§2.4 決策1): writes into `<agent_workdir>/.claude/skills/`,
/// tracking which skill ids GT Office owns in a sibling manifest so a skill
/// the user placed there by hand — or a skill removed from this snapshot —
/// is never silently touched/deleted.
fn sync_skills_copy_fallback(
    agent_workdir: &Path,
    snapshot: &AgentCapabilitySnapshot,
) -> Result<Option<PathBuf>, MaterializeError> {
    let skills_dir = agent_workdir.join(".claude").join("skills");
    let manifest_path = skills_dir.join(MANAGED_SKILLS_MANIFEST_FILE);
    let mut managed_ids = read_managed_skills_manifest(&manifest_path);

    let enabled_ids: Vec<&str> = snapshot
        .skills
        .iter()
        .filter(|skill| skill.enabled)
        .map(|skill| skill.id.as_str())
        .collect();

    if enabled_ids.is_empty() && managed_ids.is_empty() {
        return Ok(None);
    }

    std::fs::create_dir_all(&skills_dir).map_err(|error| io_err(&skills_dir, error))?;

    // Remove GT-Office-managed skill dirs that are no longer in the
    // snapshot. Never touches an id that isn't in our own manifest.
    for stale_id in managed_ids
        .iter()
        .filter(|id| !enabled_ids.contains(&id.as_str()))
        .cloned()
        .collect::<Vec<_>>()
    {
        let stale_dir = skills_dir.join(&stale_id);
        if stale_dir.is_dir() {
            std::fs::remove_dir_all(&stale_dir).map_err(|error| io_err(&stale_dir, error))?;
        }
    }

    // A dest dir that exists but wasn't in the manifest we just read is
    // something GT Office never wrote — most likely a skill the user placed
    // by hand under the same id. Skip it rather than overwrite it; the
    // design explicitly requires never touching those (§2.4 決策1).
    let previously_managed = managed_ids.clone();
    let mut newly_managed_ids: Vec<String> = Vec::new();
    for skill in snapshot.skills.iter().filter(|skill| skill.enabled) {
        let dest_dir = skills_dir.join(&skill.id);
        if dest_dir.exists() && !previously_managed.iter().any(|id| id == &skill.id) {
            tracing::warn!(
                skill_id = %skill.id,
                "skipping skill copy: a directory with this id already exists and is not \
                 GT-Office-managed (likely hand-placed by the user); rename the skill id to \
                 avoid the collision"
            );
            continue;
        }
        std::fs::create_dir_all(&dest_dir).map_err(|error| io_err(&dest_dir, error))?;
        let dest_file = dest_dir.join("SKILL.md");
        let content = std::fs::read_to_string(&skill.source_path)
            .map_err(|error| io_err(Path::new(&skill.source_path), error))?;
        std::fs::write(&dest_file, content).map_err(|error| io_err(&dest_file, error))?;
        newly_managed_ids.push(skill.id.clone());
    }

    managed_ids = newly_managed_ids;
    write_managed_skills_manifest(&manifest_path, &managed_ids)?;

    if managed_ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some(skills_dir))
    }
}

fn read_managed_skills_manifest(manifest_path: &Path) -> Vec<String> {
    std::fs::read_to_string(manifest_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

fn write_managed_skills_manifest(
    manifest_path: &Path,
    managed_ids: &[String],
) -> Result<(), MaterializeError> {
    let json = serde_json::to_string_pretty(managed_ids).unwrap_or_else(|_| "[]".to_string());
    write_atomic(manifest_path, &json)
}

/// Temp-file-then-rename so a concurrent reader (a different process/thread
/// than the one holding `agent_lock`, e.g. a stray manual read) never
/// observes a half-written file (§2.6). The temp name mixes in a
/// process-local monotonic counter alongside the PID: two *different*
/// target paths always get different temp names anyway (derived from each
/// target's own filename), but this adds a cheap extra margin in case a
/// future caller ever writes the same target without holding `agent_lock`.
fn write_atomic(path: &Path, contents: &str) -> Result<(), MaterializeError> {
    static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let counter = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| io_err(parent, error))?;
    let tmp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "materialize".to_string()),
        std::process::id(),
        counter
    ));
    std::fs::write(&tmp_path, contents).map_err(|error| io_err(&tmp_path, error))?;
    std::fs::rename(&tmp_path, path).map_err(|error| io_err(path, error))?;
    Ok(())
}

/// What `materialize_codex_capability` wrote, for `resume.rs` to fold into
/// the launch command as `codex -p <profile_name> ...` (the `-p`/`--profile`
/// flag must come *before* the subcommand — confirmed empirically, see
/// below).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedCodexProfile {
    /// Pass as `-p <profile_name>` (or `--profile`). Built by
    /// `codex_profile_name` as `gtoffice-<slug>-<hash8>` — namespaced so it
    /// can never collide with a profile name the user picked themselves,
    /// and hash-suffixed so it can't collide with another agent's either
    /// (see that function's doc comment for why the slug alone isn't
    /// enough).
    pub profile_name: String,
    /// `<codex_home>/<profile_name>.config.toml` — informational; callers
    /// launch via `profile_name`, not this path directly.
    pub profile_path: PathBuf,
}

/// Provider-tagged materialize result, so a single caller (`resume.rs`)
/// can carry "what did materialize produce for this agent" without needing
/// to know the agent's provider ahead of time — the two providers' outputs
/// get folded into a launch command in structurally different ways (Claude:
/// append `--mcp-config`/`--settings` after the subcommand; Codex: insert
/// `-p <profile>` *before* the subcommand), so this stays two variants
/// rather than one shared struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializedCapability {
    Claude(MaterializedPaths),
    Codex(MaterializedCodexProfile),
}

/// Codex materialize path (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.4 決策2).
///
/// **This deliberately does NOT match §2.4 決策2's original design-time
/// text** (isolate `CODEX_HOME` per agent, copy the user's non-`mcp_servers`
/// config into it, merge in the agent's `mcp_servers`). That design was
/// explicitly flagged in the doc as "本設計工程風險最高的部分" pending
/// verification against the real CLI — verification found a materially
/// better mechanism, so per the doc's own instruction ("若驗證後發現行為
/// 不同，這裡的合併邏輯要重新設計，不要照本節文字直接動工") this function
/// implements the revised design instead:
///
/// - `codex --help` documents a global `-p, --profile <name>` flag: "Layer
///   `$CODEX_HOME/<name>.config.toml` on top of the base user config."
///   Empirically confirmed (not just read from `--help`) with the pinned
///   Codex CLI: a profile file containing only `[mcp_servers.foo]` and
///   `codex -p <profile> mcp list` showed BOTH `foo` and the server already
///   defined in the *base* `~/.codex/config.toml` — i.e. this is a genuine
///   overlay/merge, not a replace. `-p` also parses correctly ahead of
///   `resume`/`fork` (`codex -p <profile> resume --help` / `fork --help`
///   both exit 0), which is what `resume.rs` actually needs to launch.
/// - This means full `CODEX_HOME` isolation is not just riskier but
///   *unnecessary* — and isolating `CODEX_HOME` would have been actively
///   harmful: `~/.codex/` also holds `auth.json` (login credentials) and
///   session/sandbox state, none of which is part of `config.toml`.
///   Empirically, pointing `CODEX_HOME` at an empty directory makes
///   `codex doctor` report `✗ auth — no Codex credentials were found`, i.e.
///   isolating `CODEX_HOME` the way §2.4 決策2 sketched would have broken
///   login for every Codex agent unless `auth.json` were separately copied
///   too (which the original design didn't account for).
/// - So: never touch `CODEX_HOME`. Just write a small, GT-Office-namespaced
///   profile file directly into the user's real, un-isolated `$CODEX_HOME`
///   (resolve it with `resolve_codex_home`, which mirrors Codex's own
///   `CODEX_HOME` env var / `$HOME/.codex` fallback) containing only this
///   agent's `[mcp_servers]` table, and launch with `-p <profile_name>`
///   (see `codex_profile_name`). The doc's actual concern in 決策2 point 1
///   ("不直接寫使用者全域的 ~/.codex/config.toml（那是共用檔案，多 agent
///   會互相覆蓋）") is fully satisfied: this never touches `config.toml`
///   itself, only a uniquely-named sibling file — "uniquely" now meaning
///   unique per `(workspace_id, agent_id)`, not just per `agent_id`, since
///   `$CODEX_HOME` is global but `agent_id` is only unique within one
///   workspace (see `codex_profile_name`'s doc comment).
///
/// Returns `Ok(None)` when the snapshot has no enabled capability (nothing to
/// overlay — callers should launch with no `-p` flag at all, same as
/// today). Same content-hash short-circuit + per-agent lock discipline as
/// the Claude path (§2.6, 決策4), except the lock is keyed by the computed
/// `profile_name`, not the raw `agent_id` — see `agent_lock`'s doc comment
/// for why that distinction matters here specifically.
///
/// `workspace_id` matters even though Codex has no per-workspace directory
/// the way Claude's `runtime_dir` does: `agents.id` is only unique *within*
/// a workspace (`PRIMARY KEY (id, workspace_id)`, and callers may supply
/// their own `agent_id`), but `$CODEX_HOME` is one global, un-scoped
/// directory. Without `workspace_id` folded into the profile name, two
/// different workspaces' agents sharing an `agent_id` string would silently
/// overwrite each other's MCP config — exactly the "多 agent 會互相覆蓋" a
/// hazard §2.4 決策2 point 1 already warned about, just recreated at the
/// (workspace, agent) level instead of the (agent) level.
pub fn materialize_codex_capability(
    codex_home: &Path,
    workspace_id: &str,
    agent_id: &str,
    snapshot: &AgentCapabilitySnapshot,
) -> Result<Option<MaterializedCodexProfile>, MaterializeError> {
    snapshot
        .validate_for_tool("codex")
        .map_err(MaterializeError::InvalidSnapshot)?;

    let profile_name = codex_profile_name(workspace_id, agent_id);
    let lock = agent_lock(&profile_name);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let profile_path = codex_home.join(format!("{profile_name}.config.toml"));
    let hash_path = codex_home.join(format!("{profile_name}.capability-hash"));

    let has_enabled_capability = snapshot.mcp_servers.iter().any(|item| item.enabled)
        || snapshot.skills.iter().any(|item| item.enabled)
        || !snapshot.hooks.is_empty();
    if !has_enabled_capability {
        // Nothing enabled to overlay (either no servers at all, or every
        // one of them is toggled off). Clean up a stale profile from a
        // previous snapshot that did have enabled servers, so a leftover
        // file never gets picked up by a future `-p` launch that assumes
        // it's current.
        if let Err(error) = std::fs::remove_file(&profile_path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    path = %profile_path.display(),
                    %error,
                    "failed to remove stale Codex profile file"
                );
            }
        }
        if let Err(error) = std::fs::remove_file(&hash_path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    path = %hash_path.display(),
                    %error,
                    "failed to remove stale Codex profile hash file"
                );
            }
        }
        return Ok(None);
    }

    let content_hash = compute_codex_content_hash(snapshot);
    if let Ok(recorded) = std::fs::read_to_string(&hash_path) {
        if recorded.trim() == content_hash && profile_path.is_file() {
            return Ok(Some(MaterializedCodexProfile {
                profile_name,
                profile_path,
            }));
        }
    }

    let toml_text = build_codex_profile_toml(snapshot)
        .map_err(|error| MaterializeError::InvalidSnapshot(error.to_string()))?;
    write_atomic(&profile_path, &toml_text)?;
    write_atomic(&hash_path, &content_hash)?;

    Ok(Some(MaterializedCodexProfile {
        profile_name,
        profile_path,
    }))
}

fn compute_codex_content_hash(snapshot: &AgentCapabilitySnapshot) -> String {
    let snapshot_json = snapshot.to_json().unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    snapshot_json.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// `normalize_agent_slug(agent_id)` alone is lossy (case-folded, punctuation
/// collapsed, empty/all-punctuation inputs all fall back to the literal
/// string `"agent"`) and `agent_id` is only unique within one `workspace_id`
/// — so the human-readable slug is kept only for debuggability, and a hash
/// of the *raw* `(workspace_id, agent_id)` pair is appended to actually
/// guarantee two different agents never collide on one profile file. A
/// `DefaultHasher` is fine here for the same reason it's fine in
/// `compute_content_hash`/`compute_codex_content_hash`: this only needs
/// same-process (really, same-build) uniqueness, not cross-run persistence
/// or cryptographic strength — the absolute worst case if the algorithm
/// ever changed across a Rust/std upgrade is one harmless orphaned profile
/// file left behind under a now-unreferenced old name, not a collision.
fn codex_profile_name(workspace_id: &str, agent_id: &str) -> String {
    let slug = crate::normalize_agent_slug(agent_id);
    let mut hasher = DefaultHasher::new();
    workspace_id.hash(&mut hasher);
    agent_id.hash(&mut hasher);
    format!("gtoffice-{slug}-{:08x}", hasher.finish() as u32)
}

/// Resolves `$CODEX_HOME` the same way the Codex CLI itself does: the
/// `CODEX_HOME` env var when set to a non-empty value, else `<home>/.codex`
/// — reusing `AgentInstaller::user_home_dir`'s existing
/// HOME/USERPROFILE/HOMEDRIVE+HOMEPATH fallback chain rather than
/// reimplementing it. Returns `None` only if neither yields anything (no
/// `CODEX_HOME` set and no resolvable home directory at all).
pub fn resolve_codex_home() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("CODEX_HOME") {
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    gt_tools::agent_installer::AgentInstaller::user_home_dir().map(|home| home.join(".codex"))
}

/// Builds `[mcp_servers.<id>]` tables matching the real `config.toml` shape
/// (empirically confirmed via `codex mcp add`): stdio servers get
/// `command`/`args`/optional `env` sub-table; sse/http servers get `url`
/// (Codex's `codex mcp add --url` only distinguishes "stdio vs URL", not a
/// separate sse/http type the way Claude does, so both `McpTransport::Sse`
/// and `McpTransport::Http` map to the same `url`-only shape here).
fn build_codex_profile_toml(
    snapshot: &AgentCapabilitySnapshot,
) -> Result<String, toml::ser::Error> {
    let mut mcp_servers = toml::value::Table::new();
    for server in snapshot.mcp_servers.iter().filter(|server| server.enabled) {
        let mut entry = toml::value::Table::new();
        match server.transport {
            McpTransport::Stdio => {
                entry.insert(
                    "command".to_string(),
                    toml::Value::String(server.command.clone().unwrap_or_default()),
                );
                entry.insert(
                    "args".to_string(),
                    toml::Value::Array(
                        server
                            .args
                            .iter()
                            .map(|arg| toml::Value::String(arg.clone()))
                            .collect(),
                    ),
                );
                if !server.env.is_empty() {
                    let mut env_table = toml::value::Table::new();
                    for (key, value) in &server.env {
                        env_table.insert(key.clone(), toml::Value::String(value.clone()));
                    }
                    entry.insert("env".to_string(), toml::Value::Table(env_table));
                }
            }
            McpTransport::Sse | McpTransport::Http => {
                entry.insert(
                    "url".to_string(),
                    toml::Value::String(server.url.clone().unwrap_or_default()),
                );
            }
        }
        mcp_servers.insert(server.id.clone(), toml::Value::Table(entry));
    }
    let mut root = toml::value::Table::new();
    if !mcp_servers.is_empty() {
        root.insert("mcp_servers".to_string(), toml::Value::Table(mcp_servers));
    }

    let skill_configs = snapshot
        .skills
        .iter()
        .filter(|skill| skill.enabled)
        .map(|skill| {
            let skill_dir = Path::new(&skill.source_path)
                .parent()
                .unwrap_or_else(|| Path::new(&skill.source_path));
            let mut entry = toml::value::Table::new();
            entry.insert(
                "path".to_string(),
                toml::Value::String(skill_dir.to_string_lossy().into_owned()),
            );
            entry.insert("enabled".to_string(), toml::Value::Boolean(true));
            toml::Value::Table(entry)
        })
        .collect::<Vec<_>>();
    if !skill_configs.is_empty() {
        let mut skills = toml::value::Table::new();
        skills.insert("config".to_string(), toml::Value::Array(skill_configs));
        root.insert("skills".to_string(), toml::Value::Table(skills));
    }

    let mut hooks = toml::value::Table::new();
    for hook in &snapshot.hooks {
        let mut handler = toml::value::Table::new();
        handler.insert(
            "type".to_string(),
            toml::Value::String("command".to_string()),
        );
        handler.insert(
            "command".to_string(),
            toml::Value::String(hook.command.clone()),
        );

        let mut group = toml::value::Table::new();
        if let Some(matcher) = hook.matcher.as_deref().filter(|value| !value.is_empty()) {
            group.insert(
                "matcher".to_string(),
                toml::Value::String(matcher.to_string()),
            );
        }
        group.insert(
            "hooks".to_string(),
            toml::Value::Array(vec![toml::Value::Table(handler)]),
        );
        hooks
            .entry(hook.event.clone())
            .or_insert_with(|| toml::Value::Array(Vec::new()))
            .as_array_mut()
            .expect("hook event is always initialized as an array")
            .push(toml::Value::Table(group));
    }
    if !hooks.is_empty() {
        root.insert("hooks".to_string(), toml::Value::Table(hooks));
    }
    toml::to_string_pretty(&toml::Value::Table(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{HookCapability, McpServerCapability, McpTransport, SkillCapability};

    /// Module-level (not per-test) so `resolve_codex_home_prefers_env_var_over_default`
    /// and `resolve_codex_home_falls_back_to_home_dot_codex_when_env_var_unset` — both
    /// of which mutate the process-global `CODEX_HOME` env var — actually
    /// serialize against each other under `cargo test`'s default multi-threaded
    /// runner. A `static` declared inside each test function would be a
    /// *distinct* item per function, not a shared lock, and would silently
    /// stop protecting anything.
    static CODEX_HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn stdio_mcp(id: &str, command: &str) -> McpServerCapability {
        McpServerCapability {
            id: id.to_string(),
            name: None,
            transport: McpTransport::Stdio,
            command: Some(command.to_string()),
            args: vec!["-y".to_string()],
            env: BTreeMap::new(),
            url: None,
            enabled: true,
        }
    }

    fn full_support() -> ProviderCapabilitySupport {
        ProviderCapabilitySupport {
            supports_mcp_config_flag: true,
            supports_settings_flag: true,
            supports_skills_dir_flag: false,
        }
    }

    fn write_skill_source(dir: &Path, id: &str, body: &str) -> String {
        let path = dir.join(format!("{id}.md"));
        std::fs::write(&path, body).expect("write skill source");
        path.to_string_lossy().to_string()
    }

    #[test]
    fn writes_mcp_json_and_settings_json_when_flags_supported() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));
        snapshot.hooks.push(HookCapability {
            event: "PreToolUse".to_string(),
            matcher: Some("Bash".to_string()),
            command: "echo hi".to_string(),
            note: None,
        });

        let result = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &full_support(),
        )
        .expect("materialize");

        let mcp_path = result.mcp_config_path.expect("mcp config written");
        let mcp_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_path).expect("read mcp.json"))
                .expect("parse mcp.json");
        assert_eq!(mcp_json["mcpServers"]["fs"]["command"], "npx");

        let settings_path = result.settings_path.expect("settings written");
        let settings_json: Value = serde_json::from_str(
            &std::fs::read_to_string(&settings_path).expect("read settings.json"),
        )
        .expect("parse settings.json");
        assert_eq!(settings_json["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            settings_json["PreToolUse"][0]["hooks"][0]["command"],
            "echo hi"
        );

        assert!(result
            .runtime_dir
            .ends_with(Path::new(".gtoffice/agents/agent-a/runtime")));
    }

    #[test]
    fn disabled_mcp_server_is_omitted_from_mcp_json_and_from_the_is_anything_to_write_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");

        let mut enabled_server = stdio_mcp("fs", "npx");
        enabled_server.name = Some("Filesystem".to_string());
        let mut disabled_server = stdio_mcp("git", "npx");
        disabled_server.enabled = false;

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(enabled_server);
        snapshot.mcp_servers.push(disabled_server);

        let result = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &full_support(),
        )
        .expect("materialize");

        let mcp_path = result.mcp_config_path.expect("mcp config written");
        let mcp_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_path).expect("read mcp.json"))
                .expect("parse mcp.json");
        assert!(mcp_json["mcpServers"]["fs"].is_object());
        assert!(mcp_json["mcpServers"]["git"].is_null());

        // Every server disabled must behave like "nothing configured" —
        // no mcp.json written, regardless of CLI flag support.
        let mut all_disabled_snapshot = AgentCapabilitySnapshot::default();
        let mut only_server = stdio_mcp("fs", "npx");
        only_server.enabled = false;
        all_disabled_snapshot.mcp_servers.push(only_server);
        let unsupported_result = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &all_disabled_snapshot,
            &ProviderCapabilitySupport::default(),
        )
        .expect("materialize with unsupported flags");
        assert_eq!(unsupported_result.mcp_config_path, None);
    }

    #[test]
    fn skips_mcp_and_settings_without_writing_when_flags_unsupported() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));
        snapshot.hooks.push(HookCapability {
            event: "PreToolUse".to_string(),
            matcher: None,
            command: "echo hi".to_string(),
            note: None,
        });

        let unsupported = ProviderCapabilitySupport::default();
        let result = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &unsupported,
        )
        .expect("materialize");

        assert_eq!(result.mcp_config_path, None);
        assert_eq!(result.settings_path, None);
        assert!(!result.runtime_dir.join("mcp.json").exists());
        assert!(!result.runtime_dir.join("settings.json").exists());
    }

    #[test]
    fn copy_fallback_writes_skill_and_tracks_manifest_without_touching_user_skills() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");
        let source_dir = temp.path().join("sources");
        std::fs::create_dir_all(&source_dir).expect("create source dir");

        // A skill the user placed by hand, unrelated to GT Office.
        let user_skill_dir = agent_workdir
            .join(".claude")
            .join("skills")
            .join("hand-placed");
        std::fs::create_dir_all(&user_skill_dir).expect("create user skill dir");
        std::fs::write(user_skill_dir.join("SKILL.md"), "# hand placed").expect("write user skill");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: write_skill_source(&source_dir, "reviewer", "# reviewer skill"),
            enabled: true,
        });

        let result = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &ProviderCapabilitySupport::default(),
        )
        .expect("materialize");

        let copied_dir = result.skills_copied_to.expect("skills copied");
        let copied_content = std::fs::read_to_string(copied_dir.join("reviewer").join("SKILL.md"))
            .expect("read copy");
        assert_eq!(copied_content, "# reviewer skill");

        // Hand-placed skill must survive untouched.
        assert!(user_skill_dir.join("SKILL.md").exists());

        // Removing the skill from the snapshot must delete the managed copy
        // but still leave the hand-placed one alone.
        let empty_snapshot = AgentCapabilitySnapshot::default();
        let result2 = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &empty_snapshot,
            &ProviderCapabilitySupport::default(),
        )
        .expect("re-materialize with skill removed");
        assert_eq!(result2.skills_copied_to, None);
        assert!(!copied_dir.join("reviewer").exists());
        assert!(user_skill_dir.join("SKILL.md").exists());
    }

    #[test]
    fn copy_fallback_skips_an_id_that_collides_with_a_hand_placed_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");
        let source_dir = temp.path().join("sources");
        std::fs::create_dir_all(&source_dir).expect("create source dir");

        // The user already has a hand-placed directory using the SAME id a
        // GT Office skill is about to claim.
        let colliding_dir = agent_workdir
            .join(".claude")
            .join("skills")
            .join("reviewer");
        std::fs::create_dir_all(&colliding_dir).expect("create colliding dir");
        std::fs::write(
            colliding_dir.join("SKILL.md"),
            "# hand placed, not GT Office",
        )
        .expect("write hand-placed content");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: write_skill_source(&source_dir, "reviewer", "# GT Office reviewer"),
            enabled: true,
        });

        materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &ProviderCapabilitySupport::default(),
        )
        .expect("materialize");

        let content =
            std::fs::read_to_string(colliding_dir.join("SKILL.md")).expect("read collided file");
        assert_eq!(
            content, "# hand placed, not GT Office",
            "a hand-placed directory sharing a skill id must not be overwritten"
        );
    }

    #[test]
    fn rejects_unsafe_skill_ids_without_writing_anything() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");
        let source_dir = temp.path().join("sources");
        std::fs::create_dir_all(&source_dir).expect("create source dir");

        for unsafe_id in ["../../escape", "..", ".hidden", "a/b", ""] {
            let mut snapshot = AgentCapabilitySnapshot::default();
            snapshot.skills.push(SkillCapability {
                id: unsafe_id.to_string(),
                source_path: write_skill_source(&source_dir, "safe", "# body"),
                enabled: true,
            });

            let result = materialize_claude_capability(
                &workspace_root,
                "agent-a",
                &agent_workdir,
                &snapshot,
                &ProviderCapabilitySupport::default(),
            );
            assert!(result.is_err(), "skill id {unsafe_id:?} must be rejected");
        }

        assert!(
            !agent_workdir.join(".claude").exists(),
            "no skill directory should have been created for any rejected id"
        );
    }

    #[test]
    fn changing_agent_workdir_alone_invalidates_the_cache_and_recopies_skills() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let old_workdir = workspace_root.join("agent-a-old");
        let new_workdir = workspace_root.join("agent-a-new");
        std::fs::create_dir_all(&old_workdir).expect("create old workdir");
        std::fs::create_dir_all(&new_workdir).expect("create new workdir");
        let source_dir = temp.path().join("sources");
        std::fs::create_dir_all(&source_dir).expect("create source dir");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: write_skill_source(&source_dir, "reviewer", "# reviewer skill"),
            enabled: true,
        });
        let support = ProviderCapabilitySupport::default();

        materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &old_workdir,
            &snapshot,
            &support,
        )
        .expect("materialize with old workdir");

        // Same agent_id, same snapshot, same support — only agent_workdir
        // changes (e.g. the user moved the agent's project directory).
        let result = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &new_workdir,
            &snapshot,
            &support,
        )
        .expect("materialize with new workdir");

        let copied_dir = result.skills_copied_to.expect(
            "skills must be (re-)copied into the new workdir, not skipped via a stale cache hit",
        );
        assert!(copied_dir.starts_with(&new_workdir));
        assert!(copied_dir.join("reviewer").join("SKILL.md").exists());
    }

    #[test]
    fn content_hash_short_circuits_repeated_materialize_calls() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));

        let first = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &full_support(),
        )
        .expect("first materialize");
        let mcp_path = first.mcp_config_path.clone().expect("mcp path");

        // Prove the short-circuit by hand-corrupting the written file: a
        // real rewrite would restore canonical content, so if a second call
        // with the same snapshot/support leaves the corruption in place, no
        // write happened.
        std::fs::write(&mcp_path, "not valid json, deliberately corrupted")
            .expect("corrupt mcp.json to detect a rewrite");

        let second = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &full_support(),
        )
        .expect("second materialize");

        let contents_after = std::fs::read_to_string(&mcp_path).expect("read mcp.json");
        assert_eq!(
            contents_after, "not valid json, deliberately corrupted",
            "hash-matched call must not rewrite mcp.json"
        );
        assert_eq!(second.mcp_config_path, Some(mcp_path));
    }

    #[test]
    fn changing_snapshot_invalidates_the_hash_and_rewrites() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));
        materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &full_support(),
        )
        .expect("first materialize");

        snapshot.mcp_servers.push(stdio_mcp("git", "npx"));
        let second = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &full_support(),
        )
        .expect("second materialize");

        let mcp_json: Value = serde_json::from_str(
            &std::fs::read_to_string(second.mcp_config_path.expect("mcp path"))
                .expect("read mcp.json"),
        )
        .expect("parse mcp.json");
        assert!(mcp_json["mcpServers"]["git"].is_object());
    }

    #[test]
    fn rejects_invalid_snapshot_without_writing_anything() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let agent_workdir = workspace_root.join("agent-a");
        std::fs::create_dir_all(&agent_workdir).expect("create workdir");

        let mut snapshot = AgentCapabilitySnapshot::default();
        let mut broken = stdio_mcp("fs", "npx");
        broken.command = None;
        snapshot.mcp_servers.push(broken);

        let result = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &agent_workdir,
            &snapshot,
            &full_support(),
        );
        assert!(result.is_err());
        assert!(!workspace_root
            .join(".gtoffice")
            .join("agents")
            .join("agent-a")
            .exists());
    }

    #[test]
    fn different_agents_do_not_share_a_lock_or_output_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp.path().join("workspace");
        let workdir_a = workspace_root.join("agent-a");
        let workdir_b = workspace_root.join("agent-b");
        std::fs::create_dir_all(&workdir_a).expect("create workdir a");
        std::fs::create_dir_all(&workdir_b).expect("create workdir b");

        let mut snapshot_a = AgentCapabilitySnapshot::default();
        snapshot_a.mcp_servers.push(stdio_mcp("fs-a", "npx"));
        let mut snapshot_b = AgentCapabilitySnapshot::default();
        snapshot_b.mcp_servers.push(stdio_mcp("fs-b", "npx"));

        let result_a = materialize_claude_capability(
            &workspace_root,
            "agent-a",
            &workdir_a,
            &snapshot_a,
            &full_support(),
        )
        .expect("materialize a");
        let result_b = materialize_claude_capability(
            &workspace_root,
            "agent-b",
            &workdir_b,
            &snapshot_b,
            &full_support(),
        )
        .expect("materialize b");

        assert_ne!(result_a.runtime_dir, result_b.runtime_dir);
        let mcp_a: Value = serde_json::from_str(
            &std::fs::read_to_string(result_a.mcp_config_path.unwrap()).unwrap(),
        )
        .unwrap();
        let mcp_b: Value = serde_json::from_str(
            &std::fs::read_to_string(result_b.mcp_config_path.unwrap()).unwrap(),
        )
        .unwrap();
        assert!(mcp_a["mcpServers"]["fs-a"].is_object());
        assert!(mcp_b["mcpServers"]["fs-b"].is_object());
    }

    fn read_toml(path: &Path) -> toml::Value {
        std::fs::read_to_string(path)
            .expect("read profile toml")
            .parse::<toml::Value>()
            .expect("parse profile toml")
    }

    #[test]
    fn codex_writes_a_namespaced_profile_file_with_stdio_and_http_servers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));
        snapshot.mcp_servers.push(McpServerCapability {
            id: "remote".to_string(),
            name: None,
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some("https://example.com/mcp".to_string()),
            enabled: true,
        });

        let result = materialize_codex_capability(&codex_home, "ws-1", "agent-a", &snapshot)
            .expect("materialize")
            .expect("some profile, snapshot has servers");

        let expected_name = codex_profile_name("ws-1", "agent-a");
        assert!(expected_name.starts_with("gtoffice-agent-a-"));
        assert_eq!(result.profile_name, expected_name);
        assert_eq!(
            result.profile_path,
            codex_home.join(format!("{expected_name}.config.toml"))
        );

        let parsed = read_toml(&result.profile_path);
        assert_eq!(parsed["mcp_servers"]["fs"]["command"].as_str(), Some("npx"));
        assert_eq!(
            parsed["mcp_servers"]["remote"]["url"].as_str(),
            Some("https://example.com/mcp")
        );
        assert!(parsed["mcp_servers"]["remote"].get("command").is_none());
    }

    #[test]
    fn codex_omits_disabled_servers_and_returns_none_when_every_server_is_disabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");

        let mut enabled_server = stdio_mcp("fs", "npx");
        enabled_server.name = Some("Filesystem".to_string());
        let mut disabled_server = stdio_mcp("git", "npx");
        disabled_server.enabled = false;

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(enabled_server);
        snapshot.mcp_servers.push(disabled_server);

        let result = materialize_codex_capability(&codex_home, "ws-1", "agent-a", &snapshot)
            .expect("materialize")
            .expect("one enabled server, some profile written");
        let parsed = read_toml(&result.profile_path);
        assert!(parsed["mcp_servers"]["fs"].as_table().is_some());
        assert!(parsed["mcp_servers"].get("git").is_none());

        let mut all_disabled = AgentCapabilitySnapshot::default();
        let mut only_server = stdio_mcp("fs", "npx");
        only_server.enabled = false;
        all_disabled.mcp_servers.push(only_server);
        let all_disabled_result =
            materialize_codex_capability(&codex_home, "ws-1", "agent-a", &all_disabled)
                .expect("materialize with everything disabled");
        assert_eq!(all_disabled_result, None);
        assert!(
            !result.profile_path.exists(),
            "a stale profile from the earlier (partially enabled) snapshot must be removed \
             once every server becomes disabled"
        );
    }

    #[test]
    fn codex_returns_none_and_writes_nothing_for_an_empty_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");

        let result = materialize_codex_capability(
            &codex_home,
            "ws-1",
            "agent-a",
            &AgentCapabilitySnapshot::default(),
        )
        .expect("materialize");

        assert_eq!(result, None);
        let expected_name = codex_profile_name("ws-1", "agent-a");
        assert!(!codex_home
            .join(format!("{expected_name}.config.toml"))
            .exists());
    }

    #[test]
    fn codex_removes_a_stale_profile_when_the_snapshot_becomes_empty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));
        let first = materialize_codex_capability(&codex_home, "ws-1", "agent-a", &snapshot)
            .expect("materialize")
            .expect("profile written");
        assert!(first.profile_path.is_file());

        let result = materialize_codex_capability(
            &codex_home,
            "ws-1",
            "agent-a",
            &AgentCapabilitySnapshot::default(),
        )
        .expect("re-materialize with empty snapshot");
        assert_eq!(result, None);
        assert!(
            !first.profile_path.exists(),
            "a stale profile file must be removed once its snapshot goes empty"
        );
    }

    #[test]
    fn codex_content_hash_short_circuits_repeated_calls() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.mcp_servers.push(stdio_mcp("fs", "npx"));

        let first = materialize_codex_capability(&codex_home, "ws-1", "agent-a", &snapshot)
            .expect("materialize")
            .expect("profile written");
        std::fs::write(
            &first.profile_path,
            "not valid toml, deliberately corrupted",
        )
        .expect("corrupt profile to detect a rewrite");

        let second = materialize_codex_capability(&codex_home, "ws-1", "agent-a", &snapshot)
            .expect("re-materialize")
            .expect("still some profile");

        let contents = std::fs::read_to_string(&second.profile_path).expect("read profile");
        assert_eq!(
            contents, "not valid toml, deliberately corrupted",
            "hash-matched call must not rewrite the profile file"
        );
    }

    #[test]
    fn codex_materializes_skills_and_hooks_into_the_profile() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");

        let mut snapshot = AgentCapabilitySnapshot::default();
        snapshot.skills.push(SkillCapability {
            id: "reviewer".to_string(),
            source_path: "/tmp/reviewer/SKILL.md".to_string(),
            enabled: true,
        });
        snapshot.hooks.push(HookCapability {
            event: "PreToolUse".to_string(),
            matcher: Some("Bash".to_string()),
            command: "echo inspect".to_string(),
            note: None,
        });

        let result = materialize_codex_capability(&codex_home, "ws-1", "agent-a", &snapshot)
            .expect("materialize")
            .expect("profile");
        let contents = std::fs::read_to_string(result.profile_path).expect("read profile");
        assert!(contents.contains("[[skills.config]]"));
        assert!(contents.contains("enabled = true"));
        assert!(contents.contains("[[hooks.PreToolUse]]"));
        assert!(contents.contains("matcher = \"Bash\""));
        assert!(contents.contains("command = \"echo inspect\""));
    }

    #[test]
    fn codex_different_workspaces_with_the_same_agent_id_never_collide() {
        let temp = tempfile::tempdir().expect("tempdir");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");

        let mut snapshot_a = AgentCapabilitySnapshot::default();
        snapshot_a.mcp_servers.push(stdio_mcp("fs-ws-a", "npx"));
        let mut snapshot_b = AgentCapabilitySnapshot::default();
        snapshot_b.mcp_servers.push(stdio_mcp("fs-ws-b", "npx"));

        // Same agent_id, different workspace_id — this is exactly the
        // collision the design must avoid, since agents.id is only unique
        // *within* a workspace.
        let result_a = materialize_codex_capability(&codex_home, "ws-a", "agent-1", &snapshot_a)
            .expect("materialize ws-a")
            .expect("profile written for ws-a");
        let result_b = materialize_codex_capability(&codex_home, "ws-b", "agent-1", &snapshot_b)
            .expect("materialize ws-b")
            .expect("profile written for ws-b");

        assert_ne!(result_a.profile_name, result_b.profile_name);
        assert_ne!(result_a.profile_path, result_b.profile_path);

        let toml_a = read_toml(&result_a.profile_path);
        let toml_b = read_toml(&result_b.profile_path);
        assert!(toml_a["mcp_servers"]["fs-ws-a"].as_table().is_some());
        assert!(toml_b["mcp_servers"]["fs-ws-b"].as_table().is_some());
        assert!(
            toml_a.get("mcp_servers").unwrap().get("fs-ws-b").is_none(),
            "ws-a's profile must not contain ws-b's server"
        );
    }

    #[test]
    fn codex_profile_name_disambiguates_agent_ids_that_normalize_to_the_same_slug() {
        // "Agent-1" and "agent!!!1" both normalize (case-fold + punctuation
        // collapse) toward the same lossy slug shape — the hash suffix must
        // still keep them apart.
        let name_a = codex_profile_name("ws-1", "Agent-1");
        let name_b = codex_profile_name("ws-1", "agent!!!1");
        assert_ne!(name_a, name_b);
    }

    #[test]
    fn resolve_codex_home_prefers_env_var_over_default() {
        let _guard = CODEX_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let previous = std::env::var_os("CODEX_HOME");
        std::env::set_var("CODEX_HOME", "/custom/codex/home");
        let resolved = resolve_codex_home();
        match previous {
            Some(value) => std::env::set_var("CODEX_HOME", value),
            None => std::env::remove_var("CODEX_HOME"),
        }

        assert_eq!(resolved, Some(PathBuf::from("/custom/codex/home")));
    }

    #[test]
    fn resolve_codex_home_falls_back_to_home_dot_codex_when_env_var_unset() {
        let _guard = CODEX_HOME_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let previous = std::env::var_os("CODEX_HOME");
        std::env::remove_var("CODEX_HOME");
        let resolved = resolve_codex_home();
        if let Some(value) = previous {
            std::env::set_var("CODEX_HOME", value);
        }

        let resolved = resolved.expect("some home dir resolvable in this test environment");
        assert!(resolved.ends_with(".codex"));
    }
}
