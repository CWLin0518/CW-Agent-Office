use crate::sqlite::SqliteStorage;
use gt_agent::{
    AgentCapabilityAuditRepository, AgentCapabilityRepository, AgentCapabilitySnapshot, AgentError,
    AgentLink, AgentLinkKind, AgentLinkRepository, AgentPolicy, AgentPolicyRepository,
    AgentProfile, AgentRepository, AgentResult, AgentScope, AgentState, CreateAgentInput,
    HookAuditEntry, UpdateAgentInput,
};
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone)]
pub struct SqliteAgentRepository {
    storage: SqliteStorage,
}

impl SqliteAgentRepository {
    pub fn new(storage: SqliteStorage) -> Self {
        Self { storage }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    fn connection(&self) -> AgentResult<rusqlite::Connection> {
        self.storage
            .open_connection()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })
    }

    // NOTE: `agent_links`/`agent_policy_snapshots` have no FK cascade, so this
    // clears both explicitly. `agent_policy_snapshots` cleanup is still
    // deferred (not yet exercised by any workspace-reset path in P3); revisit
    // together if that changes.
    pub fn reset_workspace_state_in_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        workspace_id: &str,
    ) -> AgentResult<()> {
        tx.execute(
            "DELETE FROM agents WHERE workspace_id = ?1",
            params![workspace_id],
        )
        .map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        tx.execute(
            "DELETE FROM agent_links WHERE workspace_id = ?1",
            params![workspace_id],
        )
        .map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        Ok(())
    }

    fn migrate_legacy_schema(conn: &rusqlite::Connection) -> AgentResult<()> {
        let columns = conn
            .prepare("PRAGMA table_info(agents)")
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        if columns
            .iter()
            .any(|column| column == "role_id" || column == "role_workspace_id")
        {
            conn.execute_batch(
                r#"
                PRAGMA foreign_keys = OFF;
                BEGIN IMMEDIATE;
                CREATE TABLE agents__without_roles (
                  id TEXT NOT NULL, workspace_id TEXT NOT NULL, name TEXT NOT NULL,
                  tool TEXT NOT NULL DEFAULT 'codex', workdir TEXT,
                  custom_workdir INTEGER NOT NULL DEFAULT 0, scope TEXT NOT NULL DEFAULT 'station',
                  state TEXT NOT NULL, employee_no TEXT, policy_snapshot_id TEXT,
                  launch_command TEXT, order_index INTEGER NOT NULL DEFAULT 0,
                  created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
                  PRIMARY KEY (id, workspace_id)
                );
                INSERT INTO agents__without_roles
                SELECT id, workspace_id, name, tool, workdir, custom_workdir,
                  COALESCE(scope, 'station'), state, employee_no, policy_snapshot_id,
                  launch_command, COALESCE(order_index, 0), created_at_ms, updated_at_ms
                FROM agents;
                DROP TABLE agents;
                ALTER TABLE agents__without_roles RENAME TO agents;
                COMMIT;
                PRAGMA foreign_keys = ON;
            "#,
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        }
        conn.execute_batch("DROP TABLE IF EXISTS deleted_system_role_seeds; DROP TABLE IF EXISTS agent_roles; DROP TABLE IF EXISTS org_departments;")
            .map_err(|error| AgentError::Storage { message: error.to_string() })
    }
}

const AGENT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agents (
  id TEXT NOT NULL, workspace_id TEXT NOT NULL, name TEXT NOT NULL,
  tool TEXT NOT NULL DEFAULT 'codex', workdir TEXT,
  custom_workdir INTEGER NOT NULL DEFAULT 0, scope TEXT NOT NULL DEFAULT 'station',
  state TEXT NOT NULL, employee_no TEXT, policy_snapshot_id TEXT,
  launch_command TEXT, output_collection_enabled INTEGER NOT NULL DEFAULT 0,
  session_boundary_auto_split_enabled INTEGER NOT NULL DEFAULT 0,
  communicate_with_all INTEGER NOT NULL DEFAULT 0,
  order_index INTEGER NOT NULL DEFAULT 0,
  parent_agent_id TEXT, external_template_path TEXT,
  git_tracked INTEGER NOT NULL DEFAULT 1,
  layout_x REAL, layout_y REAL, color TEXT,
  created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
  PRIMARY KEY (id, workspace_id)
);
"#;

const AGENT_LINKS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_links (
  id TEXT NOT NULL PRIMARY KEY, workspace_id TEXT NOT NULL,
  from_agent_id TEXT NOT NULL, to_agent_id TEXT NOT NULL,
  kind TEXT NOT NULL, color TEXT, bidirectional INTEGER NOT NULL DEFAULT 0,
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_links_workspace
  ON agent_links(workspace_id, created_at_ms DESC);

-- One row per (workspace, from, to, kind): derived links are upserted so a
-- chatty pair of agents doesn't grow this table per-dispatch, and authored
-- links (P4.5) can only exist once per direction/kind anyway.
CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_links_pair
  ON agent_links(workspace_id, from_agent_id, to_agent_id, kind);
"#;

const AGENT_POLICY_SNAPSHOTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_policy_snapshots (
  id TEXT NOT NULL PRIMARY KEY, workspace_id TEXT NOT NULL,
  agent_id TEXT NOT NULL, policy_json TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_policy_snapshots_agent
  ON agent_policy_snapshots(workspace_id, agent_id, created_at_ms DESC);
"#;

const AGENT_CAPABILITY_SNAPSHOTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_capability_snapshots (
  id TEXT NOT NULL PRIMARY KEY, workspace_id TEXT NOT NULL,
  agent_id TEXT NOT NULL, capability_json TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_capability_snapshots_agent
  ON agent_capability_snapshots(workspace_id, agent_id, created_at_ms DESC);
"#;

/// Version-lock ledger for hook rules (docs/cw/08_MCP_Hook_Skill掛載設計.md §2.5
/// 決策3, §3): a hook's content hash lands here only after the user has
/// walked through the full preview UI and explicitly confirmed it. Presence
/// of `(workspace_id, agent_id, hook_hash)` is the *entire* gate — saving a
/// capability snapshot containing a hook whose hash isn't in this table must
/// be rejected (see `agent_capability_save` in the Tauri command layer).
/// `hook_hash` is `HookCapability::content_hash()` (SHA-256, not the
/// DefaultHasher used elsewhere in this design for cache-busting — this one
/// is a persisted security gate, not a cache key, so it needs to stay
/// correct across a Rust/std upgrade, not just within one build).
const AGENT_HOOK_CONFIRMATIONS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_hook_confirmations (
  workspace_id TEXT NOT NULL, agent_id TEXT NOT NULL, hook_hash TEXT NOT NULL,
  confirmed_by TEXT NOT NULL, confirmed_at_ms INTEGER NOT NULL,
  PRIMARY KEY (workspace_id, agent_id, hook_hash)
);
"#;

/// Append-only audit trail for every hook actually applied via
/// `agent_capability_save` (docs/cw/08_MCP_Hook_Skill掛載設計.md §3: "每條
/// hook 的 apply 動作都要寫進 audit_repository"), mirroring the shape (not
/// the literal schema — that one's AI-config-specific fields don't fit
/// hooks) of `gt-ai-config`'s `ai_config_audit_logs` /
/// `AiConfigAuditLogInput` pattern: who confirmed it, what the content was
/// at apply time, when.
const AGENT_CAPABILITY_AUDIT_LOGS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_capability_audit_logs (
  id TEXT NOT NULL PRIMARY KEY, workspace_id TEXT NOT NULL, agent_id TEXT NOT NULL,
  hook_hash TEXT NOT NULL, event TEXT NOT NULL, matcher TEXT, command TEXT NOT NULL,
  confirmed_by TEXT NOT NULL, created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_capability_audit_logs_agent
  ON agent_capability_audit_logs(workspace_id, agent_id, created_at_ms DESC);
"#;

impl AgentRepository for SqliteAgentRepository {
    fn ensure_schema(&self) -> AgentResult<()> {
        let conn = self.connection()?;
        conn.execute_batch(AGENT_SCHEMA)
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        Self::migrate_legacy_schema(&conn)?;
        // Additive columns for DBs created before this field existed. Run after the
        // legacy rebuild above so a rebuilt table still picks these up.
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN parent_agent_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN external_template_path TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN git_tracked INTEGER NOT NULL DEFAULT 1",
            [],
        );
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN layout_x REAL", []);
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN layout_y REAL", []);
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN color TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN capability_snapshot_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN output_collection_enabled INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN session_boundary_auto_split_enabled INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN communicate_with_all INTEGER NOT NULL DEFAULT 0",
            [],
        );
        conn.execute_batch(AGENT_LINKS_SCHEMA)
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        // Additive columns for `agent_links` (docs/cw/04_客製化設計.md §8, P4.6)
        // — wire color and the unidirectional/bidirectional display toggle.
        // `bidirectional` defaults to 0 (false) deliberately: today's real
        // rendering default is a single arrowhead (`renderEdge` only ever
        // sets `markerEnd`), so defaulting to false keeps every pre-existing
        // authored link's appearance unchanged after this migration runs.
        let _ = conn.execute("ALTER TABLE agent_links ADD COLUMN color TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE agent_links ADD COLUMN bidirectional INTEGER NOT NULL DEFAULT 0",
            [],
        );
        conn.execute_batch(AGENT_POLICY_SNAPSHOTS_SCHEMA)
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        conn.execute_batch(AGENT_CAPABILITY_SNAPSHOTS_SCHEMA)
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        conn.execute_batch(AGENT_HOOK_CONFIRMATIONS_SCHEMA)
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        conn.execute_batch(AGENT_CAPABILITY_AUDIT_LOGS_SCHEMA)
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })
    }

    fn reset_workspace_state(&self, workspace_id: &str) -> AgentResult<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        self.reset_workspace_state_in_tx(&tx, workspace_id)?;
        tx.commit().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })
    }

    fn list_agents(&self, workspace_id: &str) -> AgentResult<Vec<AgentProfile>> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare("SELECT id, workspace_id, name, tool, workdir, custom_workdir, scope, state, employee_no, policy_snapshot_id, launch_command, output_collection_enabled, order_index, parent_agent_id, external_template_path, git_tracked, layout_x, layout_y, color, capability_snapshot_id, created_at_ms, updated_at_ms, session_boundary_auto_split_enabled, communicate_with_all FROM agents WHERE workspace_id = ?1 ORDER BY order_index, created_at_ms")
            .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        let rows = stmt
            .query_map(params![workspace_id], |row| {
                let state: String = row.get(7)?;
                let scope: String = row.get(6)?;
                Ok(AgentProfile {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    name: row.get(2)?,
                    tool: row.get(3)?,
                    workdir: row.get(4)?,
                    custom_workdir: row.get::<_, i32>(5)? != 0,
                    scope: AgentScope::from_storage_str(&scope),
                    state: AgentState::from_storage_str(&state),
                    employee_no: row.get(8)?,
                    policy_snapshot_id: row.get(9)?,
                    launch_command: row.get(10)?,
                    output_collection_enabled: row.get::<_, i32>(11)? != 0,
                    order_index: row.get(12)?,
                    parent_agent_id: row.get(13)?,
                    external_template_path: row.get(14)?,
                    git_tracked: row.get::<_, i32>(15)? != 0,
                    layout_x: row.get(16)?,
                    layout_y: row.get(17)?,
                    color: row.get(18)?,
                    capability_snapshot_id: row.get(19)?,
                    prompt_file_name: None,
                    prompt_file_relative_path: None,
                    created_at_ms: row.get(20)?,
                    updated_at_ms: row.get(21)?,
                    session_boundary_auto_split_enabled: row.get::<_, i32>(22)? != 0,
                    communicate_with_all: row.get::<_, i32>(23)? != 0,
                })
            })
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })
    }

    fn create_agent(&self, input: CreateAgentInput) -> AgentResult<AgentProfile> {
        if input.name.trim().is_empty() || input.tool.trim().is_empty() {
            return Err(AgentError::InvalidArgument {
                message: "agent name and tool are required".to_string(),
            });
        }
        let conn = self.connection()?;
        let id = input
            .agent_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let order_index = input.order_index.unwrap_or_else(|| {
            conn.query_row(
                "SELECT COALESCE(MAX(order_index), 0) + 1 FROM agents WHERE workspace_id = ?1",
                params![input.workspace_id],
                |row| row.get(0),
            )
            .unwrap_or(1)
        });
        let now = Self::now_ms();
        conn.execute("INSERT INTO agents (id, workspace_id, name, tool, workdir, custom_workdir, scope, state, employee_no, policy_snapshot_id, launch_command, output_collection_enabled, order_index, parent_agent_id, external_template_path, created_at_ms, updated_at_ms, session_boundary_auto_split_enabled, communicate_with_all) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)", params![id, input.workspace_id, input.name, input.tool, input.workdir, if input.custom_workdir { 1 } else { 0 }, input.scope.as_str(), input.state.as_str(), input.employee_no, input.launch_command, if input.output_collection_enabled { 1 } else { 0 }, order_index, input.parent_agent_id, input.external_template_path, now, now, if input.session_boundary_auto_split_enabled { 1 } else { 0 }, if input.communicate_with_all { 1 } else { 0 }])
            .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        self.list_agents(&input.workspace_id)?
            .into_iter()
            .find(|agent| agent.id == id)
            .ok_or(AgentError::Storage {
                message: "created agent was not found".to_string(),
            })
    }

    fn update_agent(&self, input: UpdateAgentInput) -> AgentResult<AgentProfile> {
        let conn = self.connection()?;
        let updated = conn.execute("UPDATE agents SET name = ?1, tool = ?2, workdir = ?3, custom_workdir = ?4, state = ?5, employee_no = ?6, launch_command = ?7, output_collection_enabled = ?8, session_boundary_auto_split_enabled = ?9, communicate_with_all = ?10, updated_at_ms = ?11 WHERE workspace_id = ?12 AND id = ?13", params![input.name, input.tool, input.workdir, if input.custom_workdir { 1 } else { 0 }, input.state.as_str(), input.employee_no, input.launch_command, if input.output_collection_enabled { 1 } else { 0 }, if input.session_boundary_auto_split_enabled { 1 } else { 0 }, if input.communicate_with_all { 1 } else { 0 }, Self::now_ms(), input.workspace_id, input.agent_id])
            .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        if updated == 0 {
            return Err(AgentError::InvalidArgument {
                message: "agent_id not found".to_string(),
            });
        }
        self.list_agents(&input.workspace_id)?
            .into_iter()
            .find(|agent| agent.id == input.agent_id)
            .ok_or(AgentError::Storage {
                message: "updated agent was not found".to_string(),
            })
    }

    fn set_git_tracked(
        &self,
        workspace_id: &str,
        agent_id: &str,
        tracked: bool,
    ) -> AgentResult<AgentProfile> {
        let conn = self.connection()?;
        let updated = conn
            .execute(
                "UPDATE agents SET git_tracked = ?1, updated_at_ms = ?2 WHERE workspace_id = ?3 AND id = ?4",
                params![if tracked { 1 } else { 0 }, Self::now_ms(), workspace_id, agent_id],
            )
            .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        if updated == 0 {
            return Err(AgentError::InvalidArgument {
                message: "agent_id not found".to_string(),
            });
        }
        self.list_agents(workspace_id)?
            .into_iter()
            .find(|agent| agent.id == agent_id)
            .ok_or(AgentError::Storage {
                message: "updated agent was not found".to_string(),
            })
    }

    fn delete_agent(&self, workspace_id: &str, agent_id: &str) -> AgentResult<bool> {
        let conn = self.connection()?;
        Ok(conn
            .execute(
                "DELETE FROM agents WHERE workspace_id = ?1 AND id = ?2",
                params![workspace_id, agent_id],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?
            > 0)
    }

    fn reorder_agents(&self, workspace_id: &str, ordered_ids: Vec<String>) -> AgentResult<()> {
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        for (index, id) in ordered_ids.iter().enumerate() {
            tx.execute("UPDATE agents SET order_index = ?1, updated_at_ms = ?2 WHERE workspace_id = ?3 AND id = ?4", params![index as i32 + 1, Self::now_ms(), workspace_id, id]).map_err(|error| AgentError::Storage { message: error.to_string() })?;
        }
        tx.commit().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })
    }
}

impl AgentPolicyRepository for SqliteAgentRepository {
    fn save_agent_policy(
        &self,
        workspace_id: &str,
        agent_id: &str,
        policy: &AgentPolicy,
    ) -> AgentResult<String> {
        let policy_json = policy
            .to_json()
            .map_err(|error| AgentError::InvalidArgument {
                message: format!("invalid policy: {error}"),
            })?;
        let mut conn = self.connection()?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Self::now_ms();
        let tx = conn.transaction().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        tx.execute(
            "INSERT INTO agent_policy_snapshots (id, workspace_id, agent_id, policy_json, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, workspace_id, agent_id, policy_json, now],
        )
        .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        let updated = tx
            .execute(
                "UPDATE agents SET policy_snapshot_id = ?1, updated_at_ms = ?2 WHERE workspace_id = ?3 AND id = ?4",
                params![id, now, workspace_id, agent_id],
            )
            .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        if updated == 0 {
            return Err(AgentError::InvalidArgument {
                message: "agent_id not found".to_string(),
            });
        }
        tx.commit().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        Ok(id)
    }

    fn get_agent_policy(&self, workspace_id: &str, agent_id: &str) -> AgentResult<AgentPolicy> {
        let conn = self.connection()?;
        let policy_json: Option<String> = conn
            .query_row(
                "SELECT policy_json FROM agent_policy_snapshots WHERE workspace_id = ?1 AND agent_id = ?2 ORDER BY created_at_ms DESC LIMIT 1",
                params![workspace_id, agent_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        match policy_json {
            Some(json) => AgentPolicy::from_json(&json).map_err(|error| AgentError::Storage {
                message: format!("corrupt policy snapshot: {error}"),
            }),
            None => Ok(AgentPolicy::default()),
        }
    }
}

impl AgentCapabilityRepository for SqliteAgentRepository {
    fn save_agent_capability(
        &self,
        workspace_id: &str,
        agent_id: &str,
        capability: &AgentCapabilitySnapshot,
    ) -> AgentResult<String> {
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        let tool: Option<String> = tx
            .query_row(
                "SELECT tool FROM agents WHERE workspace_id = ?1 AND id = ?2",
                params![workspace_id, agent_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        let tool = tool.ok_or_else(|| AgentError::InvalidArgument {
            message: "agent_id not found".to_string(),
        })?;
        capability
            .validate_for_tool(&tool)
            .map_err(|message| AgentError::InvalidArgument { message })?;

        let capability_json =
            capability
                .to_json()
                .map_err(|error| AgentError::InvalidArgument {
                    message: format!("invalid capability: {error}"),
                })?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Self::now_ms();
        tx.execute(
            "INSERT INTO agent_capability_snapshots (id, workspace_id, agent_id, capability_json, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, workspace_id, agent_id, capability_json, now],
        )
        .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        tx.execute(
            "UPDATE agents SET capability_snapshot_id = ?1, updated_at_ms = ?2 WHERE workspace_id = ?3 AND id = ?4",
            params![id, now, workspace_id, agent_id],
        )
        .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        tx.commit().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        Ok(id)
    }

    fn get_agent_capability(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> AgentResult<AgentCapabilitySnapshot> {
        let conn = self.connection()?;
        let capability_json: Option<String> = conn
            .query_row(
                "SELECT capability_json FROM agent_capability_snapshots WHERE workspace_id = ?1 AND agent_id = ?2 ORDER BY created_at_ms DESC LIMIT 1",
                params![workspace_id, agent_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| AgentError::Storage { message: error.to_string() })?;
        match capability_json {
            Some(json) => {
                AgentCapabilitySnapshot::from_json(&json).map_err(|error| AgentError::Storage {
                    message: format!("corrupt capability snapshot: {error}"),
                })
            }
            None => Ok(AgentCapabilitySnapshot::default()),
        }
    }
}

impl AgentCapabilityAuditRepository for SqliteAgentRepository {
    fn confirmed_hook_hashes(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> AgentResult<std::collections::HashSet<String>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT hook_hash FROM agent_hook_confirmations WHERE workspace_id = ?1 AND agent_id = ?2",
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        let rows = stmt
            .query_map(params![workspace_id, agent_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        rows.collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })
    }

    fn confirm_hook_hashes(
        &self,
        workspace_id: &str,
        agent_id: &str,
        hook_hashes: &[String],
        confirmed_by: &str,
    ) -> AgentResult<()> {
        if hook_hashes.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection()?;
        let now = Self::now_ms();
        let tx = conn.transaction().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        for hash in hook_hashes {
            tx.execute(
                "INSERT INTO agent_hook_confirmations (workspace_id, agent_id, hook_hash, confirmed_by, confirmed_at_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT (workspace_id, agent_id, hook_hash) DO NOTHING",
                params![workspace_id, agent_id, hash, confirmed_by, now],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        }
        tx.commit().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })
    }

    fn record_hook_audit(&self, entries: &[HookAuditEntry]) -> AgentResult<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection()?;
        let tx = conn.transaction().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        for entry in entries {
            tx.execute(
                "INSERT INTO agent_capability_audit_logs \
                 (id, workspace_id, agent_id, hook_hash, event, matcher, command, confirmed_by, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    entry.id,
                    entry.workspace_id,
                    entry.agent_id,
                    entry.hook_hash,
                    entry.event,
                    entry.matcher,
                    entry.command,
                    entry.confirmed_by,
                    entry.created_at_ms,
                ],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        }
        tx.commit().map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })
    }

    fn list_hook_audit_logs(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> AgentResult<Vec<HookAuditEntry>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, agent_id, hook_hash, event, matcher, command, confirmed_by, created_at_ms \
                 FROM agent_capability_audit_logs WHERE workspace_id = ?1 AND agent_id = ?2 ORDER BY created_at_ms DESC",
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        let rows = stmt
            .query_map(params![workspace_id, agent_id], |row| {
                Ok(HookAuditEntry {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    hook_hash: row.get(3)?,
                    event: row.get(4)?,
                    matcher: row.get(5)?,
                    command: row.get(6)?,
                    confirmed_by: row.get(7)?,
                    created_at_ms: row.get(8)?,
                })
            })
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })
    }
}

impl AgentLinkRepository for SqliteAgentRepository {
    fn record_derived_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<()> {
        let conn = self.connection()?;
        let now = Self::now_ms();
        conn.execute(
            "INSERT INTO agent_links (id, workspace_id, from_agent_id, to_agent_id, kind, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, 'derived', ?5) \
             ON CONFLICT(workspace_id, from_agent_id, to_agent_id, kind) \
             DO UPDATE SET created_at_ms = excluded.created_at_ms",
            params![
                uuid::Uuid::new_v4().to_string(),
                workspace_id,
                from_agent_id,
                to_agent_id,
                now
            ],
        )
        .map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        Ok(())
    }

    fn create_authored_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<()> {
        // The unique index on (workspace, from, to, kind) is direction-sensitive,
        // but authored edges are a direction-agnostic "these two may talk"
        // declaration (see `has_authored_edge`) — so a plain upsert would let
        // drawing a->b and separately b->a create two independent rows that
        // `delete_authored_link` (below) couldn't both revoke with a single
        // right-click. Guard against that by treating either existing
        // direction as "already drawn" up front.
        if self.has_authored_edge(workspace_id, from_agent_id, to_agent_id)? {
            return Ok(());
        }
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO agent_links (id, workspace_id, from_agent_id, to_agent_id, kind, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, 'authored', ?5) \
             ON CONFLICT(workspace_id, from_agent_id, to_agent_id, kind) DO NOTHING",
            params![
                uuid::Uuid::new_v4().to_string(),
                workspace_id,
                from_agent_id,
                to_agent_id,
                Self::now_ms()
            ],
        )
        .map_err(|error| AgentError::Storage {
            message: error.to_string(),
        })?;
        Ok(())
    }

    fn delete_authored_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<bool> {
        // Direction-agnostic to match `has_authored_edge`/`create_authored_link`
        // — deletes whichever direction the row actually exists in (there is
        // only ever at most one, per the guard in `create_authored_link`), so
        // revoking an edge always fully revokes it regardless of which
        // direction it happened to be drawn in.
        let conn = self.connection()?;
        let affected = conn
            .execute(
                "DELETE FROM agent_links WHERE workspace_id = ?1 AND kind = 'authored' \
                 AND ((from_agent_id = ?2 AND to_agent_id = ?3) OR (from_agent_id = ?3 AND to_agent_id = ?2))",
                params![workspace_id, from_agent_id, to_agent_id],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        Ok(affected > 0)
    }

    fn delete_derived_link(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
    ) -> AgentResult<bool> {
        // Direction-specific — unlike `delete_authored_link`, an a->b derived
        // row and a b->a derived row are independent facts (see the trait
        // doc comment), so only the exact direction requested is cleared.
        let conn = self.connection()?;
        let affected = conn
            .execute(
                "DELETE FROM agent_links WHERE workspace_id = ?1 AND from_agent_id = ?2 \
                 AND to_agent_id = ?3 AND kind = 'derived'",
                params![workspace_id, from_agent_id, to_agent_id],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        Ok(affected > 0)
    }

    fn has_authored_edge(
        &self,
        workspace_id: &str,
        agent_a: &str,
        agent_b: &str,
    ) -> AgentResult<bool> {
        let conn = self.connection()?;
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM agent_links WHERE workspace_id = ?1 AND kind = 'authored' \
                 AND ((from_agent_id = ?2 AND to_agent_id = ?3) OR (from_agent_id = ?3 AND to_agent_id = ?2)) \
                 LIMIT 1",
                params![workspace_id, agent_a, agent_b],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        Ok(exists.is_some())
    }

    fn list_links(&self, workspace_id: &str) -> AgentResult<Vec<AgentLink>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, from_agent_id, to_agent_id, kind, color, bidirectional, created_at_ms \
                 FROM agent_links WHERE workspace_id = ?1 ORDER BY created_at_ms DESC",
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        let rows = stmt
            .query_map(params![workspace_id], |row| {
                let kind: String = row.get(4)?;
                Ok(AgentLink {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    from_agent_id: row.get(2)?,
                    to_agent_id: row.get(3)?,
                    kind: AgentLinkKind::from_storage_str(&kind),
                    color: row.get(5)?,
                    bidirectional: row.get::<_, i64>(6)? != 0,
                    created_at_ms: row.get(7)?,
                })
            })
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })
    }

    fn set_agent_layout(
        &self,
        workspace_id: &str,
        agent_id: &str,
        x: f64,
        y: f64,
    ) -> AgentResult<()> {
        let conn = self.connection()?;
        let updated = conn
            .execute(
                "UPDATE agents SET layout_x = ?1, layout_y = ?2, updated_at_ms = ?3 WHERE workspace_id = ?4 AND id = ?5",
                params![x, y, Self::now_ms(), workspace_id, agent_id],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        if updated == 0 {
            return Err(AgentError::InvalidArgument {
                message: "agent_id not found".to_string(),
            });
        }
        Ok(())
    }

    fn set_agent_color(
        &self,
        workspace_id: &str,
        agent_id: &str,
        color: Option<String>,
    ) -> AgentResult<()> {
        let conn = self.connection()?;
        let updated = conn
            .execute(
                "UPDATE agents SET color = ?1, updated_at_ms = ?2 WHERE workspace_id = ?3 AND id = ?4",
                params![color, Self::now_ms(), workspace_id, agent_id],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        if updated == 0 {
            return Err(AgentError::InvalidArgument {
                message: "agent_id not found".to_string(),
            });
        }
        Ok(())
    }

    fn set_link_color(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
        color: Option<String>,
    ) -> AgentResult<()> {
        // Direction-agnostic, same WHERE shape as `delete_authored_link`.
        let conn = self.connection()?;
        let updated = conn
            .execute(
                "UPDATE agent_links SET color = ?1 WHERE workspace_id = ?2 AND kind = 'authored' \
                 AND ((from_agent_id = ?3 AND to_agent_id = ?4) OR (from_agent_id = ?4 AND to_agent_id = ?3))",
                params![color, workspace_id, from_agent_id, to_agent_id],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        if updated == 0 {
            return Err(AgentError::InvalidArgument {
                message: "authored link not found".to_string(),
            });
        }
        Ok(())
    }

    fn set_link_bidirectional(
        &self,
        workspace_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
        bidirectional: bool,
    ) -> AgentResult<()> {
        // Direction-agnostic, same WHERE shape as `delete_authored_link`.
        let conn = self.connection()?;
        let updated = conn
            .execute(
                "UPDATE agent_links SET bidirectional = ?1 WHERE workspace_id = ?2 AND kind = 'authored' \
                 AND ((from_agent_id = ?3 AND to_agent_id = ?4) OR (from_agent_id = ?4 AND to_agent_id = ?3))",
                params![bidirectional, workspace_id, from_agent_id, to_agent_id],
            )
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        if updated == 0 {
            return Err(AgentError::InvalidArgument {
                message: "authored link not found".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod p0_migration_tests {
    use super::*;
    use std::path::PathBuf;

    struct ScratchDb {
        path: PathBuf,
    }

    impl ScratchDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gt-storage-p0-test-{name}-{}.db",
                uuid::Uuid::new_v4()
            ));
            Self { path }
        }
    }

    impl Drop for ScratchDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    /// Pre-P0 `agents` schema, hand-copied from this file's history: no
    /// `parent_agent_id`/`external_template_path` columns.
    const LEGACY_AGENT_SCHEMA: &str = r#"
    CREATE TABLE agents (
      id TEXT NOT NULL, workspace_id TEXT NOT NULL, name TEXT NOT NULL,
      tool TEXT NOT NULL DEFAULT 'codex', workdir TEXT,
      custom_workdir INTEGER NOT NULL DEFAULT 0, scope TEXT NOT NULL DEFAULT 'station',
      state TEXT NOT NULL, employee_no TEXT, policy_snapshot_id TEXT,
      launch_command TEXT, order_index INTEGER NOT NULL DEFAULT 0,
      created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
      PRIMARY KEY (id, workspace_id)
    );
    "#;

    #[test]
    fn ensure_schema_migrates_pre_existing_db_without_data_loss_and_is_idempotent() {
        let scratch = ScratchDb::new("migrate");
        {
            let conn = rusqlite::Connection::open(&scratch.path).expect("open legacy db");
            conn.execute_batch(LEGACY_AGENT_SCHEMA)
                .expect("create legacy schema");
            conn.execute(
                "INSERT INTO agents (id, workspace_id, name, tool, workdir, custom_workdir, scope, state, employee_no, policy_snapshot_id, launch_command, order_index, created_at_ms, updated_at_ms) VALUES ('legacy-1', 'ws-1', 'Legacy Agent', 'codex', '.', 0, 'station', 'ready', NULL, NULL, NULL, 1, 1, 1)",
                [],
            )
            .expect("insert legacy row");
        }

        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("first ensure_schema");
        // Idempotency: a second call must not error on "duplicate column".
        repo.ensure_schema().expect("second ensure_schema");

        let agents = repo.list_agents("ws-1").expect("list agents");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "legacy-1");
        assert_eq!(agents[0].parent_agent_id, None);
        assert_eq!(agents[0].external_template_path, None);

        let conn = repo.connection().expect("connection");
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('agent_links', 'agent_policy_snapshots', 'agent_capability_snapshots')",
                [],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(table_count, 3);
    }

    #[test]
    fn create_agent_round_trips_parent_template_and_output_collection() {
        let scratch = ScratchDb::new("roundtrip");
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");

        repo.create_agent(CreateAgentInput {
            workspace_id: "ws-1".to_string(),
            agent_id: Some("agent-with-parent".to_string()),
            name: "Child".to_string(),
            tool: "codex".to_string(),
            workdir: Some(".".to_string()),
            custom_workdir: false,
            scope: AgentScope::Station,
            employee_no: None,
            state: AgentState::Ready,
            launch_command: None,
            output_collection_enabled: true,
            session_boundary_auto_split_enabled: false,
            communicate_with_all: false,
            order_index: None,
            parent_agent_id: Some("agent-parent".to_string()),
            external_template_path: Some("/tmp/template.md".to_string()),
        })
        .expect("create agent with parent/template");

        repo.create_agent(CreateAgentInput {
            workspace_id: "ws-1".to_string(),
            agent_id: Some("agent-without-parent".to_string()),
            name: "Root".to_string(),
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
        .expect("create agent without parent/template");

        let agents = repo.list_agents("ws-1").expect("list agents");
        let with_parent = agents
            .iter()
            .find(|agent| agent.id == "agent-with-parent")
            .expect("agent-with-parent present");
        assert_eq!(with_parent.parent_agent_id.as_deref(), Some("agent-parent"));
        assert!(with_parent.output_collection_enabled);
        assert_eq!(
            with_parent.external_template_path.as_deref(),
            Some("/tmp/template.md")
        );

        let without_parent = agents
            .iter()
            .find(|agent| agent.id == "agent-without-parent")
            .expect("agent-without-parent present");
        assert_eq!(without_parent.parent_agent_id, None);
        assert_eq!(without_parent.external_template_path, None);
        assert!(!without_parent.output_collection_enabled);
    }
}

#[cfg(test)]
mod p3_agent_policy_tests {
    use super::*;
    use std::path::PathBuf;

    struct ScratchDb {
        path: PathBuf,
    }

    impl ScratchDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gt-storage-p3-test-{name}-{}.db",
                uuid::Uuid::new_v4()
            ));
            Self { path }
        }
    }

    impl Drop for ScratchDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    fn repo_with_one_agent(scratch: &ScratchDb, agent_id: &str) -> SqliteAgentRepository {
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");
        repo.create_agent(CreateAgentInput {
            workspace_id: "ws-1".to_string(),
            agent_id: Some(agent_id.to_string()),
            name: "Agent".to_string(),
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
        .expect("create agent");
        repo
    }

    #[test]
    fn agent_with_no_snapshot_returns_default_permissive_policy() {
        let scratch = ScratchDb::new("no-snapshot");
        let repo = repo_with_one_agent(&scratch, "agent-1");

        let policy = repo
            .get_agent_policy("ws-1", "agent-1")
            .expect("get_agent_policy");
        assert_eq!(policy, AgentPolicy::default());
    }

    #[test]
    fn save_agent_policy_round_trips_and_repoints_snapshot_id() {
        let scratch = ScratchDb::new("round-trip");
        let repo = repo_with_one_agent(&scratch, "agent-1");

        let mut policy_v1 = AgentPolicy::default();
        policy_v1
            .shell
            .denied_commands
            .push("powershell".to_string());
        policy_v1.execution.max_concurrency = Some(2);
        policy_v1.agent.allow_gto_send = false;
        let snapshot_v1 = repo
            .save_agent_policy("ws-1", "agent-1", &policy_v1)
            .expect("save policy v1");

        let read_back_v1 = repo
            .get_agent_policy("ws-1", "agent-1")
            .expect("get policy v1");
        assert_eq!(read_back_v1, policy_v1);

        let agent = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-1")
            .expect("agent-1 present");
        assert_eq!(
            agent.policy_snapshot_id.as_deref(),
            Some(snapshot_v1.as_str())
        );

        // A second save must append a new snapshot, not overwrite the first —
        // agent_policy_snapshots is meant to stay an auditable history.
        let mut policy_v2 = AgentPolicy::default();
        policy_v2.execution.max_concurrency = Some(5);
        let snapshot_v2 = repo
            .save_agent_policy("ws-1", "agent-1", &policy_v2)
            .expect("save policy v2");
        assert_ne!(snapshot_v1, snapshot_v2);

        let read_back_v2 = repo
            .get_agent_policy("ws-1", "agent-1")
            .expect("get policy v2");
        assert_eq!(read_back_v2, policy_v2);

        let agent = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-1")
            .expect("agent-1 present");
        assert_eq!(
            agent.policy_snapshot_id.as_deref(),
            Some(snapshot_v2.as_str())
        );

        let conn = repo.connection().expect("connection");
        let snapshot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_policy_snapshots WHERE workspace_id = 'ws-1' AND agent_id = 'agent-1'",
                [],
                |row| row.get(0),
            )
            .expect("count snapshots");
        assert_eq!(snapshot_count, 2, "old snapshot must not be overwritten");
    }

    #[test]
    fn save_agent_policy_fails_for_unknown_agent() {
        let scratch = ScratchDb::new("unknown-agent");
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");

        let result = repo.save_agent_policy("ws-1", "does-not-exist", &AgentPolicy::default());
        assert!(
            result.is_err(),
            "saving a policy for an unknown agent must fail"
        );
    }
}

#[cfg(test)]
mod p3_5_agent_capability_tests {
    use super::*;
    use gt_agent::{HookCapability, McpServerCapability, McpTransport, SkillCapability};
    use std::path::PathBuf;

    struct ScratchDb {
        path: PathBuf,
    }

    impl ScratchDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gt-storage-p3-5-test-{name}-{}.db",
                uuid::Uuid::new_v4()
            ));
            Self { path }
        }
    }

    impl Drop for ScratchDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    fn repo_with_one_agent(
        scratch: &ScratchDb,
        agent_id: &str,
        tool: &str,
    ) -> SqliteAgentRepository {
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");
        repo.create_agent(CreateAgentInput {
            workspace_id: "ws-1".to_string(),
            agent_id: Some(agent_id.to_string()),
            name: "Agent".to_string(),
            tool: tool.to_string(),
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
        .expect("create agent");
        repo
    }

    fn sample_mcp_only_capability() -> AgentCapabilitySnapshot {
        let mut capability = AgentCapabilitySnapshot::default();
        capability.mcp_servers.push(McpServerCapability {
            id: "fs".to_string(),
            name: None,
            transport: McpTransport::Stdio,
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "mcp-server-fs".to_string()],
            env: Default::default(),
            url: None,
            enabled: true,
        });
        capability
    }

    #[test]
    fn agent_with_no_snapshot_returns_empty_capability() {
        let scratch = ScratchDb::new("no-snapshot");
        let repo = repo_with_one_agent(&scratch, "agent-1", "claude");

        let capability = repo
            .get_agent_capability("ws-1", "agent-1")
            .expect("get_agent_capability");
        assert_eq!(capability, AgentCapabilitySnapshot::default());
    }

    #[test]
    fn save_agent_capability_round_trips_and_repoints_snapshot_id() {
        let scratch = ScratchDb::new("round-trip");
        let repo = repo_with_one_agent(&scratch, "agent-1", "claude");

        let capability_v1 = sample_mcp_only_capability();
        let snapshot_v1 = repo
            .save_agent_capability("ws-1", "agent-1", &capability_v1)
            .expect("save capability v1");

        let read_back_v1 = repo
            .get_agent_capability("ws-1", "agent-1")
            .expect("get capability v1");
        assert_eq!(read_back_v1, capability_v1);

        let agent = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-1")
            .expect("agent-1 present");
        assert_eq!(
            agent.capability_snapshot_id.as_deref(),
            Some(snapshot_v1.as_str())
        );

        // A second save must append a new snapshot, not overwrite the first —
        // agent_capability_snapshots is meant to stay an auditable history.
        let mut capability_v2 = sample_mcp_only_capability();
        capability_v2.mcp_servers[0].id = "fs-v2".to_string();
        let snapshot_v2 = repo
            .save_agent_capability("ws-1", "agent-1", &capability_v2)
            .expect("save capability v2");
        assert_ne!(snapshot_v1, snapshot_v2);

        let read_back_v2 = repo
            .get_agent_capability("ws-1", "agent-1")
            .expect("get capability v2");
        assert_eq!(read_back_v2, capability_v2);

        let conn = repo.connection().expect("connection");
        let snapshot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_capability_snapshots WHERE workspace_id = 'ws-1' AND agent_id = 'agent-1'",
                [],
                |row| row.get(0),
            )
            .expect("count snapshots");
        assert_eq!(snapshot_count, 2, "old snapshot must not be overwritten");
    }

    #[test]
    fn save_agent_capability_fails_for_unknown_agent() {
        let scratch = ScratchDb::new("unknown-agent");
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");

        let result =
            repo.save_agent_capability("ws-1", "does-not-exist", &sample_mcp_only_capability());
        assert!(
            result.is_err(),
            "saving a capability for an unknown agent must fail"
        );
    }

    #[test]
    fn save_agent_capability_round_trips_skills_and_hooks_for_codex_agent() {
        let scratch = ScratchDb::new("codex-capability");
        let repo = repo_with_one_agent(&scratch, "agent-1", "codex");
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
        let snapshot_id = repo
            .save_agent_capability("ws-1", "agent-1", &snapshot)
            .expect("Codex supports skills and command hooks");
        drop(repo);
        let reopened =
            SqliteAgentRepository::new(SqliteStorage::new(&scratch.path).expect("reopen storage"));
        assert_eq!(
            reopened
                .get_agent_capability("ws-1", "agent-1")
                .expect("read persisted capability"),
            snapshot
        );
        let agent = reopened.list_agents("ws-1").expect("list agents").remove(0);
        assert_eq!(
            agent.capability_snapshot_id.as_deref(),
            Some(snapshot_id.as_str())
        );
    }

    #[test]
    fn global_capability_switch_survives_reopening_storage_without_changing_mounts() {
        let scratch = ScratchDb::new("global-switch");
        let mut repo = repo_with_one_agent(&scratch, "agent-1", "claude");
        assert!(
            repo.get_agent_capability("ws-1", "agent-1")
                .expect("read new agent")
                .global_capabilities_enabled
        );
        let mut snapshot = sample_mcp_only_capability();
        for enabled in [false, true] {
            snapshot.global_capabilities_enabled = enabled;
            repo.save_agent_capability("ws-1", "agent-1", &snapshot)
                .expect("save switch");
            drop(repo);
            repo = SqliteAgentRepository::new(
                SqliteStorage::new(&scratch.path).expect("reopen storage"),
            );
            assert_eq!(
                repo.get_agent_capability("ws-1", "agent-1")
                    .expect("read switch and mounts"),
                snapshot
            );
        }
        assert!(repo
            .get_agent_capability("other-workspace", "agent-1")
            .expect("read unrelated workspace")
            .mcp_servers
            .is_empty());
    }

    #[test]
    fn save_agent_capability_allows_mcp_only_for_codex_agent() {
        let scratch = ScratchDb::new("codex-mcp-ok");
        let repo = repo_with_one_agent(&scratch, "agent-1", "codex");

        let result = repo.save_agent_capability("ws-1", "agent-1", &sample_mcp_only_capability());
        assert!(
            result.is_ok(),
            "codex agents must still accept MCP-only capability snapshots"
        );
    }
}

#[cfg(test)]
mod p3_5_3_agent_capability_audit_tests {
    use super::*;
    use gt_agent::HookCapability;
    use std::path::PathBuf;

    struct ScratchDb {
        path: PathBuf,
    }

    impl ScratchDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gt-storage-p3-5-3-test-{name}-{}.db",
                uuid::Uuid::new_v4()
            ));
            Self { path }
        }
    }

    impl Drop for ScratchDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    fn repo(scratch: &ScratchDb) -> SqliteAgentRepository {
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");
        repo
    }

    fn sample_hook() -> HookCapability {
        HookCapability {
            event: "PreToolUse".to_string(),
            matcher: Some("Bash".to_string()),
            command: "echo about-to-run-bash".to_string(),
            note: None,
        }
    }

    #[test]
    fn no_hooks_confirmed_by_default() {
        let scratch = ScratchDb::new("empty");
        let repo = repo(&scratch);
        let confirmed = repo
            .confirmed_hook_hashes("ws-1", "agent-1")
            .expect("confirmed_hook_hashes");
        assert!(confirmed.is_empty());
    }

    #[test]
    fn confirm_hook_hashes_is_idempotent_and_scoped_per_agent() {
        let scratch = ScratchDb::new("confirm");
        let repo = repo(&scratch);
        let hash = sample_hook().content_hash();

        repo.confirm_hook_hashes(
            "ws-1",
            "agent-1",
            std::slice::from_ref(&hash),
            "System Admin",
        )
        .expect("confirm once");
        // Re-confirming must not error or duplicate.
        repo.confirm_hook_hashes(
            "ws-1",
            "agent-1",
            std::slice::from_ref(&hash),
            "System Admin",
        )
        .expect("confirm again");

        let confirmed_a1 = repo
            .confirmed_hook_hashes("ws-1", "agent-1")
            .expect("confirmed for agent-1");
        assert_eq!(confirmed_a1.len(), 1);
        assert!(confirmed_a1.contains(&hash));

        // A different agent must not see agent-1's confirmation.
        let confirmed_a2 = repo
            .confirmed_hook_hashes("ws-1", "agent-2")
            .expect("confirmed for agent-2");
        assert!(confirmed_a2.is_empty());
    }

    #[test]
    fn confirm_hook_hashes_with_empty_slice_is_a_no_op() {
        let scratch = ScratchDb::new("empty-slice");
        let repo = repo(&scratch);
        repo.confirm_hook_hashes("ws-1", "agent-1", &[], "System Admin")
            .expect("no-op confirm must succeed");
        assert!(repo
            .confirmed_hook_hashes("ws-1", "agent-1")
            .expect("confirmed_hook_hashes")
            .is_empty());
    }

    #[test]
    fn record_and_list_hook_audit_logs_newest_first() {
        let scratch = ScratchDb::new("audit");
        let repo = repo(&scratch);

        let entry_old = HookAuditEntry {
            id: "audit-1".to_string(),
            workspace_id: "ws-1".to_string(),
            agent_id: "agent-1".to_string(),
            hook_hash: "hash-old".to_string(),
            event: "PreToolUse".to_string(),
            matcher: Some("Bash".to_string()),
            command: "echo old".to_string(),
            confirmed_by: "System Admin".to_string(),
            created_at_ms: 1_000,
        };
        let entry_new = HookAuditEntry {
            id: "audit-2".to_string(),
            created_at_ms: 2_000,
            hook_hash: "hash-new".to_string(),
            command: "echo new".to_string(),
            ..entry_old.clone()
        };

        repo.record_hook_audit(&[entry_old.clone(), entry_new.clone()])
            .expect("record audit");

        let logs = repo
            .list_hook_audit_logs("ws-1", "agent-1")
            .expect("list audit logs");
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].id, "audit-2", "newest entry must come first");
        assert_eq!(logs[1].id, "audit-1");

        // A different agent must not see agent-1's audit trail.
        assert!(repo
            .list_hook_audit_logs("ws-1", "agent-2")
            .expect("list audit logs for agent-2")
            .is_empty());
    }

    #[test]
    fn record_hook_audit_with_empty_slice_is_a_no_op() {
        let scratch = ScratchDb::new("audit-empty");
        let repo = repo(&scratch);
        repo.record_hook_audit(&[])
            .expect("no-op record must succeed");
        assert!(repo
            .list_hook_audit_logs("ws-1", "agent-1")
            .expect("list audit logs")
            .is_empty());
    }
}

#[cfg(test)]
mod p4_agent_link_tests {
    use super::*;
    use std::path::PathBuf;

    struct ScratchDb {
        path: PathBuf,
    }

    impl ScratchDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gt-storage-p4-test-{name}-{}.db",
                uuid::Uuid::new_v4()
            ));
            Self { path }
        }
    }

    impl Drop for ScratchDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    fn repo_with_two_agents(scratch: &ScratchDb) -> SqliteAgentRepository {
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");
        for agent_id in ["agent-a", "agent-b"] {
            repo.create_agent(CreateAgentInput {
                workspace_id: "ws-1".to_string(),
                agent_id: Some(agent_id.to_string()),
                name: agent_id.to_string(),
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
            .expect("create agent");
        }
        repo
    }

    #[test]
    fn record_derived_link_upserts_instead_of_growing_unbounded() {
        let scratch = ScratchDb::new("upsert");
        let repo = repo_with_two_agents(&scratch);

        repo.record_derived_link("ws-1", "agent-a", "agent-b")
            .expect("record first dispatch");
        repo.record_derived_link("ws-1", "agent-a", "agent-b")
            .expect("record second dispatch between same pair");
        repo.record_derived_link("ws-1", "agent-b", "agent-a")
            .expect("record dispatch in the opposite direction");

        let links = repo.list_links("ws-1").expect("list links");
        assert_eq!(
            links.len(),
            2,
            "repeated dispatches between the same ordered pair must upsert, not insert new rows"
        );
        assert!(links.iter().all(|link| link.kind == AgentLinkKind::Derived));
    }

    #[test]
    fn delete_derived_link_only_clears_the_exact_direction_requested() {
        let scratch = ScratchDb::new("derived-delete");
        let repo = repo_with_two_agents(&scratch);

        repo.record_derived_link("ws-1", "agent-a", "agent-b")
            .expect("record a -> b dispatch");
        repo.record_derived_link("ws-1", "agent-b", "agent-a")
            .expect("record b -> a dispatch");

        assert!(
            !repo
                .delete_derived_link("ws-1", "agent-x", "agent-y")
                .expect("delete with nothing to delete"),
            "deleting a nonexistent derived row should report false, not error"
        );

        assert!(repo
            .delete_derived_link("ws-1", "agent-a", "agent-b")
            .expect("delete the a -> b direction"));

        let remaining = repo.list_links("ws-1").expect("list links");
        assert_eq!(
            remaining.len(),
            1,
            "only the a -> b row should be gone; b -> a is an independent fact"
        );
        assert_eq!(remaining[0].from_agent_id, "agent-b");
        assert_eq!(remaining[0].to_agent_id, "agent-a");
    }

    #[test]
    fn create_authored_link_is_idempotent_and_direction_agnostic_for_lookup() {
        let scratch = ScratchDb::new("authored-create");
        let repo = repo_with_two_agents(&scratch);

        repo.create_authored_link("ws-1", "agent-a", "agent-b")
            .expect("create authored link");
        repo.create_authored_link("ws-1", "agent-a", "agent-b")
            .expect("re-drawing the same edge is a no-op, not an error");

        let links = repo.list_links("ws-1").expect("list links");
        assert_eq!(
            links.len(),
            1,
            "duplicate draws must not create a second row"
        );
        assert_eq!(links[0].kind, AgentLinkKind::Authored);

        assert!(repo
            .has_authored_edge("ws-1", "agent-a", "agent-b")
            .expect("lookup forward direction"));
        assert!(repo
            .has_authored_edge("ws-1", "agent-b", "agent-a")
            .expect("lookup reverse direction"));
    }

    #[test]
    fn create_authored_link_in_reverse_direction_is_a_no_op_and_single_delete_revokes_it() {
        let scratch = ScratchDb::new("authored-reverse");
        let repo = repo_with_two_agents(&scratch);

        repo.create_authored_link("ws-1", "agent-a", "agent-b")
            .expect("draw a -> b");
        repo.create_authored_link("ws-1", "agent-b", "agent-a")
            .expect("drawing the reverse direction must not error");

        let links = repo.list_links("ws-1").expect("list links");
        assert_eq!(
            links.len(),
            1,
            "the reverse-direction draw must not create a second row for the same undirected pair"
        );

        assert!(repo
            .delete_authored_link("ws-1", "agent-b", "agent-a")
            .expect("delete via the direction that was never actually stored"));
        assert!(
            !repo
                .has_authored_edge("ws-1", "agent-a", "agent-b")
                .expect("lookup after delete"),
            "a single delete (regardless of direction) must fully revoke the edge"
        );
    }

    #[test]
    fn has_authored_edge_is_false_when_no_edge_drawn() {
        let scratch = ScratchDb::new("authored-missing");
        let repo = repo_with_two_agents(&scratch);

        assert!(!repo
            .has_authored_edge("ws-1", "agent-a", "agent-b")
            .expect("lookup with no edges recorded"));

        repo.record_derived_link("ws-1", "agent-a", "agent-b")
            .expect("record a derived link");
        assert!(
            !repo
                .has_authored_edge("ws-1", "agent-a", "agent-b")
                .expect("lookup after only a derived link exists"),
            "a derived link must not satisfy an authored-edge check"
        );
    }

    #[test]
    fn delete_authored_link_removes_row_and_reports_whether_one_existed() {
        let scratch = ScratchDb::new("authored-delete");
        let repo = repo_with_two_agents(&scratch);

        assert!(
            !repo
                .delete_authored_link("ws-1", "agent-a", "agent-b")
                .expect("delete with nothing to delete"),
            "deleting a nonexistent edge should report false, not error"
        );

        repo.create_authored_link("ws-1", "agent-a", "agent-b")
            .expect("create authored link");
        assert!(repo
            .delete_authored_link("ws-1", "agent-a", "agent-b")
            .expect("delete existing edge"));
        assert!(!repo
            .has_authored_edge("ws-1", "agent-a", "agent-b")
            .expect("lookup after delete"));
    }

    #[test]
    fn set_link_color_and_bidirectional_are_direction_agnostic() {
        let scratch = ScratchDb::new("link-color-bidi");
        let repo = repo_with_two_agents(&scratch);
        repo.create_authored_link("ws-1", "agent-a", "agent-b")
            .expect("create authored link");

        // Drawn a -> b; set via the reverse (b, a) order to confirm the
        // lookup is direction-agnostic like `delete_authored_link`.
        repo.set_link_color("ws-1", "agent-b", "agent-a", Some("blue".to_string()))
            .expect("set link color");
        repo.set_link_bidirectional("ws-1", "agent-b", "agent-a", true)
            .expect("set link bidirectional");

        let link = repo
            .list_links("ws-1")
            .expect("list links")
            .into_iter()
            .find(|link| link.kind == AgentLinkKind::Authored)
            .expect("authored link present");
        assert_eq!(link.color, Some("blue".to_string()));
        assert!(link.bidirectional);

        repo.set_link_color("ws-1", "agent-a", "agent-b", None)
            .expect("reset link color to default");
        repo.set_link_bidirectional("ws-1", "agent-a", "agent-b", false)
            .expect("reset link bidirectional to unidirectional");
        let reset_link = repo
            .list_links("ws-1")
            .expect("list links")
            .into_iter()
            .find(|link| link.kind == AgentLinkKind::Authored)
            .expect("authored link present");
        assert_eq!(reset_link.color, None, "None resets to default gray");
        assert!(!reset_link.bidirectional);

        assert!(
            repo.set_link_color(
                "ws-1",
                "agent-a",
                "does-not-exist",
                Some("blue".to_string())
            )
            .is_err(),
            "setting color for a nonexistent authored link must fail"
        );
        assert!(
            repo.set_link_bidirectional("ws-1", "agent-a", "does-not-exist", true)
                .is_err(),
            "setting bidirectional for a nonexistent authored link must fail"
        );
    }

    #[test]
    fn set_agent_layout_persists_position_and_fails_for_unknown_agent() {
        let scratch = ScratchDb::new("layout");
        let repo = repo_with_two_agents(&scratch);

        repo.set_agent_layout("ws-1", "agent-a", 120.5, -40.0)
            .expect("set layout");
        let agent = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-a")
            .expect("agent-a present");
        assert_eq!(agent.layout_x, Some(120.5));
        assert_eq!(agent.layout_y, Some(-40.0));

        let other = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-b")
            .expect("agent-b present");
        assert_eq!(other.layout_x, None, "unrelated agent must be untouched");

        let result = repo.set_agent_layout("ws-1", "does-not-exist", 0.0, 0.0);
        assert!(
            result.is_err(),
            "setting layout for an unknown agent must fail"
        );
    }

    #[test]
    fn set_agent_color_persists_and_resets_and_fails_for_unknown_agent() {
        let scratch = ScratchDb::new("agent-color");
        let repo = repo_with_two_agents(&scratch);

        repo.set_agent_color("ws-1", "agent-a", Some("blue".to_string()))
            .expect("set color");
        let agent = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-a")
            .expect("agent-a present");
        assert_eq!(agent.color, Some("blue".to_string()));

        let other = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-b")
            .expect("agent-b present");
        assert_eq!(other.color, None, "unrelated agent must be untouched");

        repo.set_agent_color("ws-1", "agent-a", None)
            .expect("reset color to default");
        let reset_agent = repo
            .list_agents("ws-1")
            .expect("list agents")
            .into_iter()
            .find(|agent| agent.id == "agent-a")
            .expect("agent-a present");
        assert_eq!(reset_agent.color, None, "None resets to default gray");

        let result = repo.set_agent_color("ws-1", "does-not-exist", Some("blue".to_string()));
        assert!(
            result.is_err(),
            "setting color for an unknown agent must fail"
        );
    }

    #[test]
    fn reset_workspace_state_clears_agent_links() {
        let scratch = ScratchDb::new("reset");
        let storage = SqliteStorage::new(&scratch.path).expect("open storage");
        let repo = SqliteAgentRepository::new(storage);
        repo.ensure_schema().expect("ensure_schema");
        repo.create_agent(CreateAgentInput {
            workspace_id: "ws-1".to_string(),
            agent_id: Some("agent-a".to_string()),
            name: "agent-a".to_string(),
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
        .expect("create agent-a");
        repo.record_derived_link("ws-1", "agent-a", "agent-b")
            .expect("record link");

        repo.reset_workspace_state("ws-1")
            .expect("reset workspace state");

        assert!(repo.list_agents("ws-1").expect("list agents").is_empty());
        assert!(repo.list_links("ws-1").expect("list links").is_empty());
    }
}
