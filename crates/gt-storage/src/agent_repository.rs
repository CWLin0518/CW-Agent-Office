use crate::sqlite::SqliteStorage;
use gt_agent::{
    AgentError, AgentLink, AgentLinkKind, AgentLinkRepository, AgentPolicy, AgentPolicyRepository,
    AgentProfile, AgentRepository, AgentResult, AgentScope, AgentState, CreateAgentInput,
    UpdateAgentInput,
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
  launch_command TEXT, order_index INTEGER NOT NULL DEFAULT 0,
  parent_agent_id TEXT, external_template_path TEXT,
  git_tracked INTEGER NOT NULL DEFAULT 1,
  layout_x REAL, layout_y REAL,
  created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
  PRIMARY KEY (id, workspace_id)
);
"#;

const AGENT_LINKS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_links (
  id TEXT NOT NULL PRIMARY KEY, workspace_id TEXT NOT NULL,
  from_agent_id TEXT NOT NULL, to_agent_id TEXT NOT NULL,
  kind TEXT NOT NULL, created_at_ms INTEGER NOT NULL
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
        conn.execute_batch(AGENT_LINKS_SCHEMA)
            .map_err(|error| AgentError::Storage {
                message: error.to_string(),
            })?;
        conn.execute_batch(AGENT_POLICY_SNAPSHOTS_SCHEMA)
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
        let mut stmt = conn.prepare("SELECT id, workspace_id, name, tool, workdir, custom_workdir, scope, state, employee_no, policy_snapshot_id, launch_command, order_index, parent_agent_id, external_template_path, git_tracked, layout_x, layout_y, created_at_ms, updated_at_ms FROM agents WHERE workspace_id = ?1 ORDER BY order_index, created_at_ms")
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
                    order_index: row.get(11)?,
                    parent_agent_id: row.get(12)?,
                    external_template_path: row.get(13)?,
                    git_tracked: row.get::<_, i32>(14)? != 0,
                    layout_x: row.get(15)?,
                    layout_y: row.get(16)?,
                    prompt_file_name: None,
                    prompt_file_relative_path: None,
                    created_at_ms: row.get(17)?,
                    updated_at_ms: row.get(18)?,
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
        conn.execute("INSERT INTO agents (id, workspace_id, name, tool, workdir, custom_workdir, scope, state, employee_no, policy_snapshot_id, launch_command, order_index, parent_agent_id, external_template_path, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?13, ?14, ?15)", params![id, input.workspace_id, input.name, input.tool, input.workdir, if input.custom_workdir { 1 } else { 0 }, input.scope.as_str(), input.state.as_str(), input.employee_no, input.launch_command, order_index, input.parent_agent_id, input.external_template_path, now, now])
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
        let updated = conn.execute("UPDATE agents SET name = ?1, tool = ?2, workdir = ?3, custom_workdir = ?4, state = ?5, employee_no = ?6, launch_command = ?7, updated_at_ms = ?8 WHERE workspace_id = ?9 AND id = ?10", params![input.name, input.tool, input.workdir, if input.custom_workdir { 1 } else { 0 }, input.state.as_str(), input.employee_no, input.launch_command, Self::now_ms(), input.workspace_id, input.agent_id])
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

    fn list_links(&self, workspace_id: &str) -> AgentResult<Vec<AgentLink>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, from_agent_id, to_agent_id, kind, created_at_ms \
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
                    created_at_ms: row.get(5)?,
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
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('agent_links', 'agent_policy_snapshots')",
                [],
                |row| row.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(table_count, 2);
    }

    #[test]
    fn create_agent_round_trips_parent_agent_id_and_external_template_path() {
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
