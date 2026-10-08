//! Discovers which models the locally installed Claude Code / Codex CLI can
//! run, so the agent model picker follows CLI upgrades instead of a list that
//! is hardcoded in the UI.
//!
//! - Codex caches the server-side model list it fetched last in
//!   `$CODEX_HOME/models_cache.json` (default `~/.codex`). That cache is
//!   refreshed by the CLI itself, so it tracks new releases automatically.
//! - Claude Code has no local model list, but its `--model` aliases
//!   (`opus`, `sonnet`, `haiku`, `opusplan`) always resolve to the newest model
//!   the installed CLI version knows about.
//!
//! The user's configured default model (Codex `config.toml` / Claude
//! `settings.json`) is always included so a pinned id stays selectable.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_installer::{AgentInstaller, AgentType};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelOption {
    pub value: String,
    pub label: String,
    pub short_label: String,
    /// `alias` | `cache` | `config` | `builtin` — where the option came from.
    pub source: String,
}

impl AgentModelOption {
    fn new(value: &str, label: &str, short_label: &str, source: &str) -> Self {
        Self {
            value: value.to_string(),
            label: label.to_string(),
            short_label: short_label.to_string(),
            source: source.to_string(),
        }
    }
}

/// Paths the discovery reads from; injectable so tests never touch the real
/// home directory.
#[derive(Debug, Clone)]
pub struct AgentModelSources {
    pub codex_home: Option<PathBuf>,
    pub claude_home: Option<PathBuf>,
}

impl AgentModelSources {
    pub fn from_env() -> Self {
        let home = AgentInstaller::user_home_dir();
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".codex")));
        let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".claude")));
        Self {
            codex_home,
            claude_home,
        }
    }
}

pub fn discover_agent_models(agent: AgentType) -> Vec<AgentModelOption> {
    discover_agent_models_from(agent, &AgentModelSources::from_env())
}

pub fn discover_agent_models_from(
    agent: AgentType,
    sources: &AgentModelSources,
) -> Vec<AgentModelOption> {
    let mut options = match agent {
        AgentType::ClaudeCode => claude_models(sources.claude_home.as_deref()),
        AgentType::Codex => codex_models(sources.codex_home.as_deref()),
    };
    let mut seen = std::collections::HashSet::new();
    options.retain(|option| !option.value.is_empty() && seen.insert(option.value.clone()));
    tracing::debug!(agent = ?agent, count = options.len(), "discovered agent models");
    options
}

fn claude_models(claude_home: Option<&Path>) -> Vec<AgentModelOption> {
    let mut options = vec![
        AgentModelOption::new("opus", "Opus (latest)", "Opus", "alias"),
        AgentModelOption::new("sonnet", "Sonnet (latest)", "Sonnet", "alias"),
        AgentModelOption::new("haiku", "Haiku (latest)", "Haiku", "alias"),
        AgentModelOption::new(
            "opusplan",
            "Opus Plan (Opus plans, Sonnet executes)",
            "Opus Plan",
            "alias",
        ),
    ];
    let configured = claude_home
        .and_then(|home| std::fs::read_to_string(home.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|json| {
            json.get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| std::env::var("ANTHROPIC_MODEL").ok());
    if let Some(model) = configured.map(|value| value.trim().to_string()) {
        if is_safe_model_id(&model) {
            options.push(AgentModelOption::new(&model, &model, &model, "config"));
        }
    }
    options
}

fn codex_models(codex_home: Option<&Path>) -> Vec<AgentModelOption> {
    let mut options = codex_home
        .and_then(|home| std::fs::read_to_string(home.join("models_cache.json")).ok())
        .map(|text| parse_codex_models_cache(&text))
        .unwrap_or_default();
    if options.is_empty() {
        options = vec![
            AgentModelOption::new(
                "gpt-5.1-codex-max",
                "GPT-5.1 Codex Max",
                "GPT-5.1 Codex Max",
                "builtin",
            ),
            AgentModelOption::new("gpt-5.1-codex", "GPT-5.1 Codex", "GPT-5.1 Codex", "builtin"),
            AgentModelOption::new("gpt-5.1", "GPT-5.1", "GPT-5.1", "builtin"),
        ];
    }
    if let Some(model) = codex_home
        .and_then(|home| std::fs::read_to_string(home.join("config.toml")).ok())
        .and_then(|text| parse_codex_config_model(&text))
    {
        options.push(AgentModelOption::new(&model, &model, &model, "config"));
    }
    options
}

/// Parses Codex's `models_cache.json` (`{ "models": [ { "slug", "display_name",
/// "visibility", "priority", ... } ] }`). Tolerates a bare array and
/// `id`/`model`/`displayName` spellings so a cache format tweak degrades to
/// fewer labels rather than an empty list.
pub fn parse_codex_models_cache(text: &str) -> Vec<AgentModelOption> {
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(items) = json
        .get("models")
        .and_then(Value::as_array)
        .or_else(|| json.as_array())
    else {
        return Vec::new();
    };
    let mut ranked = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let slug = ["slug", "id", "model"]
                .iter()
                .find_map(|key| item.get(*key).and_then(Value::as_str))?
                .trim();
            if !is_safe_model_id(slug) {
                return None;
            }
            let visibility = item
                .get("visibility")
                .and_then(Value::as_str)
                .unwrap_or("list");
            if matches!(visibility, "hide" | "hidden" | "none") {
                return None;
            }
            let display = ["display_name", "displayName", "name"]
                .iter()
                .find_map(|key| item.get(*key).and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(slug);
            let priority = item
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            let label = if display == slug {
                slug.to_string()
            } else {
                format!("{display} ({slug})")
            };
            Some((
                priority,
                index,
                AgentModelOption {
                    value: slug.to_string(),
                    label,
                    short_label: display.to_string(),
                    source: "cache".to_string(),
                },
            ))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    ranked.into_iter().map(|(_, _, option)| option).collect()
}

/// Reads the top-level `model = "..."` key from Codex's `config.toml`
/// (ignoring keys inside `[tables]`).
pub fn parse_codex_config_model(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return None;
        }
        let Some(rest) = line.strip_prefix("model") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'').trim();
        return is_safe_model_id(value).then(|| value.to_string());
    }
    None
}

/// Model ids end up interpolated into a launch command (`--model <id>`), so
/// only plain identifier characters are accepted.
fn is_safe_model_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_cache_sorted_by_priority_and_skips_hidden() {
        let text = r#"{"fetched_at":"x","models":[
            {"slug":"gpt-9-codex","display_name":"GPT-9 Codex","visibility":"list","priority":2},
            {"slug":"internal","display_name":"Internal","visibility":"hide","priority":0},
            {"slug":"gpt-9","display_name":"GPT-9","priority":1}
        ]}"#;
        let models = parse_codex_models_cache(text);
        let values = models.iter().map(|m| m.value.as_str()).collect::<Vec<_>>();
        assert_eq!(values, vec!["gpt-9", "gpt-9-codex"]);
        assert_eq!(models[1].label, "GPT-9 Codex (gpt-9-codex)");
        assert_eq!(models[1].short_label, "GPT-9 Codex");
    }

    #[test]
    fn rejects_unsafe_model_ids() {
        let models = parse_codex_models_cache(r#"[{"slug":"x; rm -rf /"},{"id":"ok-1"}]"#);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].value, "ok-1");
    }

    #[test]
    fn reads_top_level_codex_config_model_only() {
        assert_eq!(
            parse_codex_config_model("# c\nmodel = \"gpt-9\"\n[profiles.a]\nmodel = \"x\""),
            Some("gpt-9".to_string())
        );
        assert_eq!(
            parse_codex_config_model("model_provider = \"a\"\n[p]\nmodel = \"x\""),
            None
        );
    }

    #[test]
    fn codex_falls_back_to_builtin_and_includes_configured_model() {
        let dir = std::env::temp_dir().join(format!(
            "gt-agent-models-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("config.toml"), "model = \"gpt-custom\"\n").expect("write");
        let sources = AgentModelSources {
            codex_home: Some(dir.clone()),
            claude_home: None,
        };
        let models = discover_agent_models_from(AgentType::Codex, &sources);
        assert!(models.iter().any(|m| m.source == "builtin"));
        assert_eq!(models.last().map(|m| m.value.as_str()), Some("gpt-custom"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_lists_latest_aliases() {
        let sources = AgentModelSources {
            codex_home: None,
            claude_home: None,
        };
        let models = discover_agent_models_from(AgentType::ClaudeCode, &sources);
        assert_eq!(models[0].value, "opus");
        assert!(models.iter().any(|m| m.value == "sonnet"));
    }
}
