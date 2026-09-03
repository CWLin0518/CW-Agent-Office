use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const OUTPUT_DIR_ENV: &str = "GTO_OUTPUT_DIR";
pub const LOG_FILE_ENV: &str = "GTO_LOG_FILE";
pub const HANDOFF_FILE_ENV: &str = "GTO_HANDOFF_FILE";
pub const ARTIFACT_DIR_ENV: &str = "GTO_ARTIFACT_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOutputContract {
    pub output_dir: PathBuf,
    pub log_file: PathBuf,
    pub handoff_file: PathBuf,
    pub artifact_dir: PathBuf,
}

impl AgentOutputContract {
    pub fn env(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                OUTPUT_DIR_ENV.to_string(),
                self.output_dir.to_string_lossy().into_owned(),
            ),
            (
                LOG_FILE_ENV.to_string(),
                self.log_file.to_string_lossy().into_owned(),
            ),
            (
                HANDOFF_FILE_ENV.to_string(),
                self.handoff_file.to_string_lossy().into_owned(),
            ),
            (
                ARTIFACT_DIR_ENV.to_string(),
                self.artifact_dir.to_string_lossy().into_owned(),
            ),
        ])
    }
}

/// Creates the provider-neutral output contract for one new Agent session.
/// Files stay flat because the Agent Canvas output scanner is intentionally
/// non-recursive. The handoff path is stable, so a new handoff replaces only
/// this Agent's previous GT Office handoff and never touches provider-owned
/// locations such as `.claude/session-handoff/`.
pub fn create_agent_output_contract(
    workspace_root: &Path,
    agent_id: &str,
    agent_name: &str,
) -> Result<AgentOutputContract, String> {
    let output_dir = workspace_root
        .join(".gtoffice")
        .join("agents")
        .join(agent_id)
        .join("outputs");
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("AGENT_OUTPUT_DIR_CREATE_FAILED: {error}"))?;

    let slug = crate::normalize_agent_slug(agent_name);
    let now_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("AGENT_OUTPUT_CLOCK_INVALID: {error}"))?
        .as_secs() as i64;
    let mut offset = 0_i64;
    let log_file = loop {
        let stamp = utc_compact_timestamp(now_seconds + offset);
        let candidate = output_dir.join(format!("{slug}-{stamp}-log.md"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                writeln!(
                    file,
                    "# {agent_name} Log\n\n- Started: {stamp} UTC\n- Agent ID: `{agent_id}`\n"
                )
                .map_err(|error| format!("AGENT_OUTPUT_LOG_CREATE_FAILED: {error}"))?;
                break candidate;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => offset += 1,
            Err(error) => return Err(format!("AGENT_OUTPUT_LOG_CREATE_FAILED: {error}")),
        }
    };

    Ok(AgentOutputContract {
        output_dir: output_dir.clone(),
        log_file,
        handoff_file: output_dir.join(format!("{slug}-handoff.md")),
        artifact_dir: output_dir,
    })
}

// Gregorian UTC conversion without adding a date/time dependency solely for
// a portable file name. Based on the public-domain civil-from-days algorithm.
fn utc_compact_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_compact_utc_timestamp() {
        assert_eq!(utc_compact_timestamp(0), "19700101-000000");
        assert_eq!(utc_compact_timestamp(1_782_822_896), "20260630-123456");
    }

    #[test]
    fn creates_unique_log_and_stable_handoff_paths() {
        let temp = tempfile::tempdir().unwrap();
        let first =
            create_agent_output_contract(temp.path(), "agent-1", "Tool Create Engineer").unwrap();
        let second =
            create_agent_output_contract(temp.path(), "agent-1", "Tool Create Engineer").unwrap();
        assert_ne!(first.log_file, second.log_file);
        assert_eq!(first.handoff_file, second.handoff_file);
        assert!(first.log_file.is_file());
        assert!(first
            .log_file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-log.md"));
    }
}
