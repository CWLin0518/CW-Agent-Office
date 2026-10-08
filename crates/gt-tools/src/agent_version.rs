//! Installed-vs-latest version check for the managed agent CLIs, backing the
//! "update Claude Code / Codex" action.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent_installer::{AgentInstaller, AgentType};

const VERSION_PROBE_TIMEOUT_MS: u64 = 5_000;
const LATEST_VERSION_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentVersionInfo {
    pub installed: bool,
    pub installed_version: Option<String>,
    /// `None` when the registry could not be reached (offline, npm missing).
    pub latest_version: Option<String>,
    pub update_available: bool,
}

pub fn npm_package_name(agent: AgentType) -> &'static str {
    match agent {
        AgentType::ClaudeCode => "@anthropic-ai/claude-code",
        AgentType::Codex => "@openai/codex",
    }
}

pub fn agent_version_info(agent: AgentType) -> AgentVersionInfo {
    let executable = AgentInstaller::launch_executable_hint(agent);
    let installed_version = executable
        .as_deref()
        .and_then(|exe| run_with_timeout(exe, &["--version"], VERSION_PROBE_TIMEOUT_MS))
        .and_then(|output| parse_version(&output));
    let latest_version = fetch_latest_version(agent);
    let update_available = match (installed_version.as_deref(), latest_version.as_deref()) {
        (Some(installed), Some(latest)) => is_newer(latest, installed),
        _ => false,
    };
    tracing::info!(
        agent = ?agent,
        installed = ?installed_version,
        latest = ?latest_version,
        update_available,
        "agent cli version check"
    );
    AgentVersionInfo {
        installed: executable.is_some(),
        installed_version,
        latest_version,
        update_available,
    }
}

fn fetch_latest_version(agent: AgentType) -> Option<String> {
    let package = npm_package_name(agent);
    let output = if cfg!(target_os = "windows") {
        run_with_timeout(
            "cmd",
            &["/C", "npm", "view", package, "version"],
            LATEST_VERSION_TIMEOUT_MS,
        )
    } else {
        run_with_timeout(
            "npm",
            &["view", package, "version"],
            LATEST_VERSION_TIMEOUT_MS,
        )
    }?;
    parse_version(&output)
}

/// Runs `program args` and returns stdout+stderr when it exits successfully
/// before `timeout_ms`; kills it and returns `None` otherwise.
fn run_with_timeout(program: &str, args: &[&str], timeout_ms: u64) -> Option<String> {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_millis(timeout_ms) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(program, "agent version probe timed out");
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some(text)
}

/// Extracts the first `major.minor.patch` token, e.g. `2.0.14 (Claude Code)`
/// or `codex-cli 0.46.0`.
pub fn parse_version(text: &str) -> Option<String> {
    text.split(|ch: char| ch.is_whitespace() || ch == '(' || ch == ')' || ch == ',')
        .map(|token| token.trim_start_matches('v'))
        .find(|token| {
            let core = token.split(['-', '+']).next().unwrap_or("");
            let parts = core.split('.').collect::<Vec<_>>();
            parts.len() >= 3
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        })
        .map(str::to_string)
}

fn numeric_parts(version: &str) -> Vec<u64> {
    version
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// `true` when `candidate` is a strictly higher release than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    numeric_parts(candidate) > numeric_parts(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_version_output() {
        assert_eq!(
            parse_version("2.0.14 (Claude Code)\n").as_deref(),
            Some("2.0.14")
        );
        assert_eq!(parse_version("codex-cli 0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(
            parse_version("v1.2.3-beta.1").as_deref(),
            Some("1.2.3-beta.1")
        );
        assert_eq!(parse_version("no version"), None);
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("2.1.0", "2.0.14"));
        assert!(!is_newer("2.0.14", "2.0.14"));
        assert!(!is_newer("1.9.0", "2.0.0"));
    }
}
