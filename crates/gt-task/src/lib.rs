use gt_abstractions::{AgentPolicyProvider, AllowAllAgentPolicyProvider, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, warn};

mod runtime_activity;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DispatchSenderType {
    #[default]
    Human,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DispatchSender {
    #[serde(rename = "type", default)]
    pub sender_type: DispatchSenderType,
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAttachment {
    pub path: String,
    pub name: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDispatchBatchRequest {
    pub workspace_id: String,
    #[serde(default)]
    pub sender: DispatchSender,
    pub targets: Vec<String>,
    pub title: String,
    pub markdown: String,
    #[serde(default)]
    pub attachments: Vec<TaskAttachment>,
    #[serde(default)]
    pub submit_sequences: HashMap<String, String>,
    // Per-dispatch override, enforced by `dispatch_appended_sections`: when
    // true, the output-collection instructions section is never appended for
    // this call, regardless of any target's `output_collection_enabled`.
    // Does not touch that per-agent setting itself — it only applies to this
    // one send.
    #[serde(default)]
    pub suppress_output_collection_instructions: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskDispatchStatus {
    Sent,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskDispatchProgressStatus {
    Sending,
    Sent,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDispatchTargetResult {
    pub target_agent_id: String,
    pub task_id: String,
    pub status: TaskDispatchStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDispatchPreviewTarget {
    pub target_agent_id: String,
    // Everything dispatch would append after the sender's own markdown for
    // this target (output-collection instructions, etc.) — empty when this
    // target's settings add nothing. Never includes the sender's own text,
    // so a caller can show it as a separate "will also be sent" preview.
    pub appended_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDispatchBatchResponse {
    pub batch_id: String,
    pub results: Vec<TaskDispatchTargetResult>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskThreadState {
    Open,
    Replied,
    HandedOver,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Direct,
    Group,
    Broadcast,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMessageType {
    TaskInstruction,
    Status,
    Handover,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDescriptor {
    pub kind: ChannelKind,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPublishRequest {
    pub workspace_id: String,
    pub channel: ChannelDescriptor,
    #[serde(default)]
    pub sender_agent_id: Option<String>,
    #[serde(default)]
    pub target_agent_ids: Vec<String>,
    #[serde(rename = "type")]
    pub message_type: ChannelMessageType,
    pub payload: Value,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

impl ChannelPublishRequest {
    /// Authorization and delivery must use the same normalized recipients,
    /// including a direct channel's id when explicit targets are omitted.
    pub fn resolved_target_agent_ids(&self) -> Vec<String> {
        resolve_publish_targets(self)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExternalPeerKind {
    Direct,
    Group,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalInboundMessage {
    pub channel: String,
    #[serde(default)]
    pub account_id: String,
    pub peer_kind: ExternalPeerKind,
    pub peer_id: String,
    pub sender_id: String,
    #[serde(default)]
    pub sender_name: Option<String>,
    pub message_id: String,
    pub text: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub workspace_id_hint: Option<String>,
    #[serde(default)]
    pub target_agent_id_hint: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExternalAccessPolicyMode {
    Pairing,
    Allowlist,
    Open,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRouteBinding {
    pub workspace_id: String,
    pub channel: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub peer_kind: Option<ExternalPeerKind>,
    #[serde(default)]
    pub peer_pattern: Option<String>,
    pub target_agent_id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub created_at_ms: Option<u64>,
    #[serde(default)]
    pub bot_name: Option<String>,
    #[serde(default = "default_binding_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalRouteResolution {
    pub workspace_id: String,
    pub target_agent_id: String,
    pub matched_by: String,
}

#[derive(Debug, Clone)]
enum ExternalRouteSelection {
    Matched(ExternalRouteResolution),
    Miss,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAccessEntry {
    pub channel: String,
    pub account_id: String,
    pub identity: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalInboundStatus {
    Dispatched,
    Duplicate,
    PairingRequired,
    Denied,
    RouteNotFound,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalInboundResponse {
    pub trace_id: String,
    pub status: ExternalInboundStatus,
    #[serde(default)]
    pub idempotent_hit: bool,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub target_agent_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub pairing_code: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelFailedTarget {
    pub agent_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPublishResponse {
    pub message_id: String,
    pub accepted_targets: Vec<String>,
    pub failed_targets: Vec<ChannelFailedTarget>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelAckStatus {
    Delivered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessageEvent {
    pub workspace_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_agent_id: Option<String>,
    pub target_agent_id: String,
    #[serde(rename = "type")]
    pub message_type: ChannelMessageType,
    pub payload: Value,
    pub ts_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAckEvent {
    pub workspace_id: String,
    pub message_id: String,
    pub target_agent_id: String,
    pub status: ChannelAckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub ts_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDispatchProgressEvent {
    pub batch_id: String,
    pub workspace_id: String,
    pub target_agent_id: String,
    pub task_id: String,
    pub status: TaskDispatchProgressStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderSessionMetadata {
    pub provider: AgentToolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_confidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeRegistration {
    pub workspace_id: String,
    pub agent_id: String,
    pub station_id: String,
    pub session_id: String,
    #[serde(default)]
    pub tool_kind: AgentToolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_sequence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session: Option<AgentProviderSessionMetadata>,
    #[serde(default = "default_true")]
    pub online: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolKind {
    Claude,
    Codex,
    Shell,
    #[default]
    Unknown,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPublishOutcome {
    pub response: ChannelPublishResponse,
    pub message_events: Vec<ChannelMessageEvent>,
    pub ack_events: Vec<ChannelAckEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelListMessagesResponse {
    pub messages: Vec<ChannelMessageEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListThreadsRequest {
    pub workspace_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskThreadSummary {
    pub task_id: String,
    pub title: String,
    pub state: TaskThreadState,
    pub root_message_id: String,
    pub latest_message_id: String,
    #[serde(rename = "latestMessageType")]
    pub latest_message_type: ChannelMessageType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_sender_agent_id: Option<String>,
    pub latest_target_agent_id: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskThreadDetail {
    pub summary: TaskThreadSummary,
    pub messages: Vec<ChannelMessageEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListThreadsResponse {
    pub threads: Vec<TaskThreadSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGetThreadRequest {
    pub workspace_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGetThreadResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<TaskThreadDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDispatchBatchOutcome {
    pub response: TaskDispatchBatchResponse,
    pub progress_events: Vec<TaskDispatchProgressEvent>,
    pub message_events: Vec<ChannelMessageEvent>,
    pub ack_events: Vec<ChannelAckEvent>,
}

#[derive(Default)]
struct TaskServiceState {
    runtimes: HashMap<String, AgentRuntimeRegistration>,
    terminal_activity: HashMap<String, u64>,
    channel_seq: HashMap<String, u64>,
    channel_messages: Vec<ChannelMessageEvent>,
    route_bindings: Vec<ChannelRouteBinding>,
    access_policies: HashMap<String, ExternalAccessPolicyMode>,
    allowlist_entries: HashSet<String>,
    pairing_requests: HashMap<String, PairingRequestRecord>,
    idempotency_cache: HashMap<String, ExternalInboundResponse>,
}

#[derive(Clone)]
pub struct TaskService {
    state: Arc<RwLock<TaskServiceState>>,
    id_counter: Arc<AtomicU64>,
    agent_policy_provider: Arc<RwLock<Arc<dyn AgentPolicyProvider>>>,
}

impl Default for TaskService {
    fn default() -> Self {
        Self {
            state: Arc::new(RwLock::new(TaskServiceState::default())),
            id_counter: Arc::new(AtomicU64::new(0)),
            agent_policy_provider: Arc::new(RwLock::new(Arc::new(AllowAllAgentPolicyProvider))),
        }
    }
}

impl TaskService {
    /// Late-bound because the concrete storage-backed provider (app layer)
    /// isn't constructible until the Tauri `AppHandle` exists, which is after
    /// `AppState::default()` already constructed this `TaskService` — see
    /// `gt_terminal`'s identical pattern for the same reason.
    pub fn set_agent_policy_provider(&self, provider: Arc<dyn AgentPolicyProvider>) {
        if let Ok(mut guard) = self.agent_policy_provider.write() {
            *guard = provider;
        }
    }

    pub fn list_messages(
        &self,
        workspace_id: &str,
        target_agent_id: Option<&str>,
        sender_agent_id: Option<&str>,
        task_id: Option<&str>,
        limit: usize,
    ) -> Vec<ChannelMessageEvent> {
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        let target_agent_id = target_agent_id
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let sender_agent_id = sender_agent_id
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let task_id = task_id.map(str::trim).filter(|value| !value.is_empty());
        let limit = limit.max(1);

        let mut messages: Vec<ChannelMessageEvent> = guard
            .channel_messages
            .iter()
            .filter(|message| message.workspace_id == workspace_id)
            .filter(|message| {
                target_agent_id
                    .map(|agent_id| message.target_agent_id == agent_id)
                    .unwrap_or(true)
            })
            .filter(|message| {
                sender_agent_id
                    .map(|agent_id| message.sender_agent_id.as_deref() == Some(agent_id))
                    .unwrap_or(true)
            })
            .filter(|message| {
                task_id
                    .map(|task_id| {
                        message.payload.get("taskId").and_then(Value::as_str) == Some(task_id)
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        messages.sort_by_key(|m| std::cmp::Reverse(m.ts_ms));
        messages.truncate(limit);
        messages
    }

    pub fn list_task_threads(
        &self,
        workspace_id: &str,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Vec<TaskThreadSummary> {
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        let limit = limit.max(1);
        let agent_id = agent_id.map(str::trim).filter(|value| !value.is_empty());

        let mut grouped: HashMap<String, Vec<ChannelMessageEvent>> = HashMap::new();
        for message in guard
            .channel_messages
            .iter()
            .filter(|message| message.workspace_id == workspace_id)
        {
            let Some(task_id) = message_task_id(message) else {
                continue;
            };
            grouped
                .entry(task_id.to_string())
                .or_default()
                .push(message.clone());
        }

        let mut threads = grouped
            .into_iter()
            .filter_map(|(task_id, messages)| {
                if agent_id
                    .map(|agent_id| {
                        messages
                            .iter()
                            .any(|message| message_involves_agent(message, agent_id))
                    })
                    .unwrap_or(true)
                {
                    build_task_thread_summary(&task_id, &messages)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        threads.sort_by_key(|t| std::cmp::Reverse(t.updated_at_ms));
        threads.truncate(limit);
        threads
    }

    pub fn get_task_thread(&self, workspace_id: &str, task_id: &str) -> Option<TaskThreadDetail> {
        let guard = self.state.read().ok()?;
        let normalized_task_id = task_id.trim();
        if normalized_task_id.is_empty() {
            return None;
        }

        let messages = guard
            .channel_messages
            .iter()
            .filter(|message| message.workspace_id == workspace_id)
            .filter(|message| message_task_id(message) == Some(normalized_task_id))
            .cloned()
            .collect::<Vec<_>>();
        let summary = build_task_thread_summary(normalized_task_id, &messages)?;
        Some(TaskThreadDetail { summary, messages })
    }

    pub fn register_runtime(&self, registration: AgentRuntimeRegistration) -> bool {
        let key = runtime_key(&registration.workspace_id, &registration.agent_id);
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        if !registration.online {
            guard.runtimes.remove(&key);
            guard.terminal_activity.remove(&key);
            purge_agent_messages_locked(
                &mut guard,
                registration.workspace_id.as_str(),
                registration.agent_id.as_str(),
            );
            return false;
        }
        debug!(
            workspace_id = %registration.workspace_id,
            agent_id = %registration.agent_id,
            station_id = %registration.station_id,
            session_id = %registration.session_id,
            tool_kind = ?registration.tool_kind,
            resolved_cwd = ?registration.resolved_cwd,
            provider_session = ?registration.provider_session,
            "registered agent runtime"
        );
        if guard
            .runtimes
            .get(&key)
            .is_some_and(|previous| previous.session_id != registration.session_id)
        {
            guard.terminal_activity.remove(&key);
        }
        guard.runtimes.insert(key, registration);
        true
    }

    pub fn update_runtime_provider_session(
        &self,
        workspace_id: &str,
        agent_id: &str,
        provider_session: Option<AgentProviderSessionMetadata>,
    ) -> bool {
        let key = runtime_key(workspace_id, agent_id);
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(runtime) = guard.runtimes.get_mut(&key) else {
            return false;
        };
        runtime.provider_session = provider_session;
        true
    }

    pub fn unregister_runtime(&self, workspace_id: &str, agent_id: &str) -> bool {
        let key = runtime_key(workspace_id, agent_id);
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let removed = guard.runtimes.remove(&key).is_some();
        guard.terminal_activity.remove(&key);
        if removed {
            purge_agent_messages_locked(&mut guard, workspace_id, agent_id);
        }
        removed
    }

    pub fn list_runtimes(&self, workspace_id: Option<&str>) -> Vec<AgentRuntimeRegistration> {
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        let mut runtimes: Vec<AgentRuntimeRegistration> =
            guard.runtimes.values().cloned().collect();
        if let Some(workspace_id) = workspace_id {
            runtimes.retain(|runtime| runtime.workspace_id == workspace_id);
        }
        runtimes
    }

    pub fn upsert_route_binding(&self, binding: ChannelRouteBinding) -> bool {
        let mut normalized = normalize_binding(binding);
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };

        if let Some(existing) = guard
            .route_bindings
            .iter_mut()
            .find(|entry| route_binding_identity_matches(entry, &normalized))
        {
            normalized.created_at_ms = existing
                .created_at_ms
                .or(normalized.created_at_ms)
                .or(Some(now_ms()));
            normalized.bot_name = normalized.bot_name.or_else(|| existing.bot_name.clone());
            *existing = normalized;
            return false;
        }

        if normalized.created_at_ms.is_none() {
            normalized.created_at_ms = Some(now_ms());
        }
        guard.route_bindings.push(normalized);
        true
    }

    pub fn delete_route_binding(&self, binding: ChannelRouteBinding) -> bool {
        let normalized = normalize_binding(binding);
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };

        let initial_len = guard.route_bindings.len();
        guard
            .route_bindings
            .retain(|entry| !route_binding_identity_matches(entry, &normalized));

        guard.route_bindings.len() < initial_len
    }

    pub fn list_route_bindings(&self, workspace_id: Option<&str>) -> Vec<ChannelRouteBinding> {
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        guard
            .route_bindings
            .iter()
            .filter(|binding| {
                workspace_id
                    .map(|workspace| binding.workspace_id == workspace)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    pub fn set_external_access_policy(
        &self,
        channel: &str,
        account_id: &str,
        mode: ExternalAccessPolicyMode,
    ) {
        let key = access_policy_key(channel, account_id);
        if let Ok(mut guard) = self.state.write() {
            guard.access_policies.insert(key, mode);
        }
    }

    pub fn get_external_access_policy(
        &self,
        channel: &str,
        account_id: &str,
    ) -> ExternalAccessPolicyMode {
        let key = access_policy_key(channel, account_id);
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return ExternalAccessPolicyMode::Pairing,
        };
        guard
            .access_policies
            .get(&key)
            .copied()
            .unwrap_or(ExternalAccessPolicyMode::Pairing)
    }

    pub fn approve_external_access(&self, channel: &str, account_id: &str, identity: &str) -> bool {
        let key = allowlist_key(channel, account_id, identity);
        let pairing_key = pairing_key(channel, account_id, identity);
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        guard.pairing_requests.remove(&pairing_key);
        guard.allowlist_entries.insert(key)
    }

    pub fn list_external_access(
        &self,
        channel: &str,
        account_id: Option<&str>,
    ) -> Vec<ExternalAccessEntry> {
        let channel_key = normalize_token(channel);
        let scoped_account = account_id
            .map(normalize_account_id)
            .unwrap_or_else(|| "default".to_string());
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        guard
            .allowlist_entries
            .iter()
            .filter_map(|entry| parse_allowlist_key(entry))
            .filter(|(entry_channel, entry_account, _)| {
                if entry_channel.as_str() != channel_key.as_str() {
                    return false;
                }
                account_id.is_none() || entry_account.as_str() == scoped_account.as_str()
            })
            .map(
                |(entry_channel, entry_account, identity)| ExternalAccessEntry {
                    channel: entry_channel,
                    account_id: entry_account,
                    identity,
                    approved: true,
                },
            )
            .collect()
    }

    pub fn clear_external_account_scope(&self, channel: &str, account_id: &str) {
        let channel_key = normalize_token(channel);
        let account_key = normalize_account_id(account_id);
        let access_policy_key = access_policy_key(&channel_key, &account_key);
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        guard.access_policies.remove(&access_policy_key);
        guard.allowlist_entries.retain(|entry| {
            parse_allowlist_key(entry)
                .map(|(entry_channel, entry_account, _)| {
                    !(entry_channel == channel_key && entry_account == account_key)
                })
                .unwrap_or(true)
        });
        guard.pairing_requests.retain(|entry, _| {
            parse_allowlist_key(entry)
                .map(|(entry_channel, entry_account, _)| {
                    !(entry_channel == channel_key && entry_account == account_key)
                })
                .unwrap_or(true)
        });
    }

    pub fn resolve_external_route(
        &self,
        inbound: &ExternalInboundMessage,
    ) -> Option<ExternalRouteResolution> {
        match self.resolve_external_route_selection(inbound, |_| true, None) {
            ExternalRouteSelection::Matched(route) => Some(route),
            ExternalRouteSelection::Miss | ExternalRouteSelection::Ambiguous => None,
        }
    }

    pub fn is_external_route_ambiguous(&self, inbound: &ExternalInboundMessage) -> bool {
        matches!(
            self.resolve_external_route_selection(inbound, |_| true, None),
            ExternalRouteSelection::Ambiguous
        )
    }

    pub fn resolve_external_route_preferring_workspace(
        &self,
        preferred_workspace_id: &str,
        inbound: &ExternalInboundMessage,
    ) -> Option<ExternalRouteResolution> {
        let preferred_workspace_id = preferred_workspace_id.trim();
        if preferred_workspace_id.is_empty() {
            return self.resolve_external_route(inbound);
        }
        match self.resolve_external_route_selection(
            inbound,
            |binding| binding.workspace_id == preferred_workspace_id,
            None,
        ) {
            ExternalRouteSelection::Matched(route) => Some(route),
            ExternalRouteSelection::Miss | ExternalRouteSelection::Ambiguous => None,
        }
    }

    pub fn resolve_external_route_in_workspace(
        &self,
        workspace_id: &str,
        inbound: &ExternalInboundMessage,
    ) -> Option<ExternalRouteResolution> {
        let workspace_id = workspace_id.trim();
        if workspace_id.is_empty() {
            return None;
        }
        match self.resolve_external_route_selection(
            inbound,
            |binding| binding.workspace_id == workspace_id,
            None,
        ) {
            ExternalRouteSelection::Matched(route) => Some(route),
            ExternalRouteSelection::Miss | ExternalRouteSelection::Ambiguous => None,
        }
    }

    fn resolve_external_route_selection<F>(
        &self,
        inbound: &ExternalInboundMessage,
        workspace_filter: F,
        preferred_workspace_id: Option<&str>,
    ) -> ExternalRouteSelection
    where
        F: Fn(&ChannelRouteBinding) -> bool,
    {
        let channel = normalize_token(&inbound.channel);
        let account_id = normalize_account_id(&inbound.account_id);
        let peer_id = inbound.peer_id.trim();
        if peer_id.is_empty() {
            return ExternalRouteSelection::Miss;
        }

        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return ExternalRouteSelection::Miss,
        };
        let mut candidates: Vec<(i32, i32, &ChannelRouteBinding, String)> = Vec::new();
        for binding in &guard.route_bindings {
            if !workspace_filter(binding) {
                continue;
            }
            if !binding.enabled {
                continue;
            }
            if normalize_token(&binding.channel) != channel {
                continue;
            }
            if !binding_account_matches(binding, &account_id) {
                continue;
            }
            if let Some(kind) = binding.peer_kind {
                if kind != inbound.peer_kind {
                    continue;
                }
            }
            if let Some(pattern) = binding.peer_pattern.as_deref() {
                if !wildcard_matches(pattern, peer_id) {
                    continue;
                }
            }
            let score = route_score(binding);
            let preferred_workspace_score = preferred_workspace_id
                .map(|workspace_id| i32::from(binding.workspace_id == workspace_id))
                .unwrap_or(0);
            let matched_by = resolve_matched_by(binding);
            candidates.push((score, preferred_workspace_score, binding, matched_by));
        }
        if candidates.is_empty() {
            return ExternalRouteSelection::Miss;
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let (top_score, top_preferred_score, _, _) = candidates[0];
        let top_tier: Vec<_> = candidates
            .iter()
            .filter(|(score, preferred_score, _, _)| {
                *score == top_score && *preferred_score == top_preferred_score
            })
            .collect();
        let mut distinct_targets = HashSet::new();
        for (_, _, binding, _) in &top_tier {
            distinct_targets.insert((
                binding.workspace_id.as_str(),
                binding.target_agent_id.as_str(),
            ));
        }
        if distinct_targets.len() > 1 {
            return ExternalRouteSelection::Ambiguous;
        }
        let (_, _, selected, matched_by) = top_tier[0];
        ExternalRouteSelection::Matched(ExternalRouteResolution {
            workspace_id: selected.workspace_id.clone(),
            target_agent_id: selected.target_agent_id.clone(),
            matched_by: matched_by.clone(),
        })
    }

    pub fn ensure_external_pairing(
        &self,
        channel: &str,
        account_id: &str,
        identity: &str,
    ) -> (String, bool, u64) {
        const PAIRING_TTL_MS: u64 = 60 * 60 * 1000;
        let key = pairing_key(channel, account_id, identity);
        let now = now_ms();
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => {
                return ("PAIRING_ERR".to_string(), false, now + PAIRING_TTL_MS);
            }
        };
        if let Some(existing) = guard.pairing_requests.get_mut(&key) {
            if existing.expires_at_ms > now {
                existing.last_seen_at_ms = now;
                return (existing.code.clone(), false, existing.expires_at_ms);
            }
        }
        let code = generate_pairing_code(self.id_counter.fetch_add(1, Ordering::Relaxed) + 1);
        let expires_at_ms = now + PAIRING_TTL_MS;
        guard.pairing_requests.insert(
            key,
            PairingRequestRecord {
                code: code.clone(),
                expires_at_ms,
                last_seen_at_ms: now,
            },
        );
        (code, true, expires_at_ms)
    }

    pub fn is_external_allowed(&self, channel: &str, account_id: &str, identity: &str) -> bool {
        let key = allowlist_key(channel, account_id, identity);
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        guard.allowlist_entries.contains(&key)
    }

    pub fn check_external_idempotency(
        &self,
        idempotency_key: &str,
    ) -> Option<ExternalInboundResponse> {
        let guard = self.state.read().ok()?;
        guard.idempotency_cache.get(idempotency_key).cloned()
    }

    pub fn store_external_idempotency(
        &self,
        idempotency_key: String,
        response: ExternalInboundResponse,
    ) {
        if let Ok(mut guard) = self.state.write() {
            guard.idempotency_cache.insert(idempotency_key, response);
        }
    }

    pub fn clear_external_idempotency_cache(&self) {
        if let Ok(mut guard) = self.state.write() {
            guard.idempotency_cache.clear();
        }
    }

    pub fn doctor_external_snapshot(&self) -> Value {
        let guard = match self.state.read() {
            Ok(guard) => guard,
            Err(_) => {
                return json!({
                    "ok": false,
                    "error": "TASK_STATE_LOCK_POISONED",
                });
            }
        };
        let now = now_ms();
        let pending_pairing = guard
            .pairing_requests
            .values()
            .filter(|entry| entry.expires_at_ms > now)
            .count();
        json!({
            "ok": true,
            "routeBindings": guard.route_bindings.len(),
            "allowlistEntries": guard.allowlist_entries.len(),
            "pairingPending": pending_pairing,
            "idempotencyEntries": guard.idempotency_cache.len(),
        })
    }

    pub fn publish(&self, request: &ChannelPublishRequest) -> ChannelPublishOutcome {
        let now = now_ms();
        let base_message_id = self.next_id("msg");
        let mut message_events = Vec::new();
        let mut ack_events = Vec::new();
        let mut accepted_targets = Vec::new();
        let mut failed_targets = Vec::new();

        let targets = resolve_publish_targets(request);
        for (index, target_agent_id) in targets.iter().enumerate() {
            let runtime = self.runtime_for(&request.workspace_id, target_agent_id);
            if runtime.is_none() {
                failed_targets.push(ChannelFailedTarget {
                    agent_id: target_agent_id.clone(),
                    reason: "AGENT_OFFLINE".to_string(),
                });
                ack_events.push(ChannelAckEvent {
                    workspace_id: request.workspace_id.clone(),
                    message_id: format!("{base_message_id}-{index}"),
                    target_agent_id: target_agent_id.clone(),
                    status: ChannelAckStatus::Failed,
                    reason: Some("AGENT_OFFLINE".to_string()),
                    ts_ms: now,
                });
                continue;
            }

            let message_id = format!("{base_message_id}-{index}");
            let channel_id = display_channel_id(
                &request.channel.kind,
                &request.channel.id,
                Some(target_agent_id),
            );
            let seq = self.next_channel_seq(&channel_id);
            message_events.push(ChannelMessageEvent {
                workspace_id: request.workspace_id.clone(),
                channel_id,
                message_id: message_id.clone(),
                seq,
                sender_agent_id: request.sender_agent_id.clone(),
                target_agent_id: target_agent_id.clone(),
                message_type: request.message_type.clone(),
                payload: request.payload.clone(),
                ts_ms: now,
            });
            ack_events.push(ChannelAckEvent {
                workspace_id: request.workspace_id.clone(),
                message_id,
                target_agent_id: target_agent_id.clone(),
                status: ChannelAckStatus::Delivered,
                reason: None,
                ts_ms: now,
            });
            accepted_targets.push(target_agent_id.clone());
        }

        self.store_channel_messages(&message_events);

        ChannelPublishOutcome {
            response: ChannelPublishResponse {
                message_id: base_message_id,
                accepted_targets,
                failed_targets,
            },
            message_events,
            ack_events,
        }
    }

    pub fn dispatch_batch<F>(
        &self,
        request: &TaskDispatchBatchRequest,
        workspace_root: &Path,
        write_terminal: F,
    ) -> TaskDispatchBatchOutcome
    where
        F: FnMut(&str, &str, &str) -> Result<(), String>,
    {
        self.dispatch_batch_with_output_directories(
            request,
            workspace_root,
            &HashMap::new(),
            &HashSet::new(),
            write_terminal,
        )
    }

    pub fn dispatch_batch_with_output_directories<F>(
        &self,
        request: &TaskDispatchBatchRequest,
        _workspace_root: &Path,
        output_directories: &HashMap<String, String>,
        session_boundary_agents: &HashSet<String>,
        mut write_terminal: F,
    ) -> TaskDispatchBatchOutcome
    where
        F: FnMut(&str, &str, &str) -> Result<(), String>,
    {
        let batch_id = self.next_id("batch");
        let title = sanitize_title(&request.title);
        let sender = request.sender.clone();
        let mut results = Vec::new();
        let mut progress_events = Vec::new();
        let mut message_events = Vec::new();
        let mut ack_events = Vec::new();

        // Execution policy (docs/cw/04_客製化設計.md §3, P3 Phase A): the only
        // limit enforceable at this hook point with existing infrastructure is
        // max_concurrency, capping how many distinct targets one dispatch_batch
        // call from a given sending agent may address. `timeout_seconds` and
        // `max_steps` are intentionally NOT enforced here — this dispatch is a
        // fire-and-forget terminal write with no task-lifecycle tracking
        // (no notion of "elapsed time" or "a step" exists anywhere in this
        // service to measure them against), so faking that check would be
        // exactly the "假權限" the design doc warns against. They stay in the
        // policy schema for whenever a real task-lifecycle layer exists to
        // back them (see docs/cw/05_PRD對齊調研.md on Runtime Snapshot).
        let max_concurrency = match (&sender.sender_type, sender.agent_id.as_deref()) {
            (DispatchSenderType::Agent, Some(sender_agent_id)) if !sender_agent_id.is_empty() => {
                let evaluator = self
                    .agent_policy_provider
                    .read()
                    .map(|guard| guard.clone())
                    .unwrap_or_else(|_| Arc::new(AllowAllAgentPolicyProvider));
                evaluator
                    .policy_for(
                        &WorkspaceId::new(request.workspace_id.clone()),
                        sender_agent_id,
                    )
                    .execution
                    .max_concurrency
            }
            _ => None,
        };

        for (target_index, target_agent_id) in normalize_agent_ids(&request.targets)
            .into_iter()
            .enumerate()
        {
            let task_id = self.next_id("task");

            if let Some(max_concurrency) = max_concurrency {
                if target_index >= max_concurrency as usize {
                    let detail = "AGENT_POLICY_CONCURRENCY_EXCEEDED".to_string();
                    results.push(TaskDispatchTargetResult {
                        target_agent_id: target_agent_id.clone(),
                        task_id: task_id.clone(),
                        status: TaskDispatchStatus::Failed,
                        detail: Some(detail.clone()),
                        task_file_path: None,
                    });
                    progress_events.push(TaskDispatchProgressEvent {
                        batch_id: batch_id.clone(),
                        workspace_id: request.workspace_id.clone(),
                        target_agent_id,
                        task_id,
                        status: TaskDispatchProgressStatus::Failed,
                        detail: Some(detail),
                    });
                    continue;
                }
            }
            progress_events.push(TaskDispatchProgressEvent {
                batch_id: batch_id.clone(),
                workspace_id: request.workspace_id.clone(),
                target_agent_id: target_agent_id.clone(),
                task_id: task_id.clone(),
                status: TaskDispatchProgressStatus::Sending,
                detail: None,
            });

            let runtime = self.runtime_for(&request.workspace_id, &target_agent_id);
            let Some(runtime) = runtime else {
                let detail = "AGENT_OFFLINE".to_string();
                results.push(TaskDispatchTargetResult {
                    target_agent_id: target_agent_id.clone(),
                    task_id: task_id.clone(),
                    status: TaskDispatchStatus::Failed,
                    detail: Some(detail.clone()),
                    task_file_path: None,
                });
                progress_events.push(TaskDispatchProgressEvent {
                    batch_id: batch_id.clone(),
                    workspace_id: request.workspace_id.clone(),
                    target_agent_id,
                    task_id,
                    status: TaskDispatchProgressStatus::Failed,
                    detail: Some(detail),
                });
                continue;
            };

            let message_id = self.next_id("msg");
            let channel_id = display_channel_id(&ChannelKind::Direct, &target_agent_id, None);
            let seq = self.next_channel_seq(&channel_id);
            let payload = json!({
                "batchId": batch_id,
                "taskId": task_id,
                "title": title,
                "taskFilePath": Value::Null,
                "attachments": request.attachments,
                "sender": {
                    "type": match sender.sender_type {
                        DispatchSenderType::Human => "human",
                        DispatchSenderType::Agent => "agent",
                    },
                    "agentId": sender.agent_id,
                },
            });

            let submit_sequence = resolve_submit_sequence(request, &target_agent_id, &runtime);
            let command = build_task_dispatch_command(&enrich_dispatch_markdown(
                &request.markdown,
                request,
                &task_id,
                output_directories.get(&target_agent_id),
                session_boundary_agents.contains(&target_agent_id),
            ));
            if let Err(error) = write_terminal(&runtime.session_id, &command, &submit_sequence) {
                warn!(
                    workspace_id = %request.workspace_id,
                    agent_id = %target_agent_id,
                    session_id = %runtime.session_id,
                    error = %error,
                    "task dispatch terminal write failed"
                );
                ack_events.push(ChannelAckEvent {
                    workspace_id: request.workspace_id.clone(),
                    message_id,
                    target_agent_id: target_agent_id.clone(),
                    status: ChannelAckStatus::Failed,
                    reason: Some(error.clone()),
                    ts_ms: now_ms(),
                });
                results.push(TaskDispatchTargetResult {
                    target_agent_id: target_agent_id.clone(),
                    task_id: task_id.clone(),
                    status: TaskDispatchStatus::Failed,
                    detail: Some(error.clone()),
                    task_file_path: None,
                });
                progress_events.push(TaskDispatchProgressEvent {
                    batch_id: batch_id.clone(),
                    workspace_id: request.workspace_id.clone(),
                    target_agent_id,
                    task_id,
                    status: TaskDispatchProgressStatus::Failed,
                    detail: Some(error),
                });
                continue;
            }

            message_events.push(ChannelMessageEvent {
                workspace_id: request.workspace_id.clone(),
                channel_id,
                message_id: message_id.clone(),
                seq,
                sender_agent_id: sender.agent_id.clone(),
                target_agent_id: target_agent_id.clone(),
                message_type: ChannelMessageType::TaskInstruction,
                payload,
                ts_ms: now_ms(),
            });
            ack_events.push(ChannelAckEvent {
                workspace_id: request.workspace_id.clone(),
                message_id,
                target_agent_id: target_agent_id.clone(),
                status: ChannelAckStatus::Delivered,
                reason: None,
                ts_ms: now_ms(),
            });
            results.push(TaskDispatchTargetResult {
                target_agent_id: target_agent_id.clone(),
                task_id: task_id.clone(),
                status: TaskDispatchStatus::Sent,
                detail: None,
                task_file_path: None,
            });
            progress_events.push(TaskDispatchProgressEvent {
                batch_id: batch_id.clone(),
                workspace_id: request.workspace_id.clone(),
                target_agent_id,
                task_id,
                status: TaskDispatchProgressStatus::Sent,
                detail: None,
            });
        }

        self.store_channel_messages(&message_events);

        TaskDispatchBatchOutcome {
            response: TaskDispatchBatchResponse { batch_id, results },
            progress_events,
            message_events,
            ack_events,
        }
    }

    // Read-only counterpart to `dispatch_batch_with_output_directories`: computes
    // exactly what that call would silently append after the sender's markdown
    // for each target, without writing to any terminal or touching the
    // filesystem. Callers always send as a human here (Task Brief is the only
    // caller), so the agent-to-agent reply-instruction section never applies —
    // `dispatch_appended_sections` already encodes that via `sender_type`.
    pub fn preview_dispatch_appended_sections(
        &self,
        workspace_id: &str,
        targets: &[String],
        output_directories: &HashMap<String, String>,
        session_boundary_agents: &HashSet<String>,
    ) -> Vec<TaskDispatchPreviewTarget> {
        let synthetic_request = TaskDispatchBatchRequest {
            workspace_id: workspace_id.to_string(),
            sender: DispatchSender::default(),
            targets: targets.to_vec(),
            title: String::new(),
            markdown: String::new(),
            attachments: Vec::new(),
            submit_sequences: HashMap::new(),
            suppress_output_collection_instructions: false,
        };
        normalize_agent_ids(targets)
            .into_iter()
            .map(|target_agent_id| {
                let sections = dispatch_appended_sections(
                    &synthetic_request,
                    "",
                    output_directories.get(&target_agent_id),
                    session_boundary_agents.contains(&target_agent_id),
                );
                TaskDispatchPreviewTarget {
                    appended_text: sections.join("\n\n"),
                    target_agent_id,
                }
            })
            .collect()
    }

    fn runtime_for(&self, workspace_id: &str, agent_id: &str) -> Option<AgentRuntimeRegistration> {
        let guard = self.state.read().ok()?;
        guard
            .runtimes
            .get(&runtime_key(workspace_id, agent_id))
            .cloned()
    }

    fn next_channel_seq(&self, channel_id: &str) -> u64 {
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return 1,
        };
        let next = guard.channel_seq.get(channel_id).copied().unwrap_or(0) + 1;
        guard.channel_seq.insert(channel_id.to_string(), next);
        next
    }

    fn next_id(&self, prefix: &str) -> String {
        let seq = self.id_counter.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{prefix}_{:x}_{:x}", now_ms(), seq)
    }

    fn store_channel_messages(&self, messages: &[ChannelMessageEvent]) {
        const MAX_CHANNEL_MESSAGES: usize = 512;

        if messages.is_empty() {
            return;
        }
        let mut guard = match self.state.write() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        guard.channel_messages.extend(messages.iter().cloned());
        if guard.channel_messages.len() > MAX_CHANNEL_MESSAGES {
            let remove_count = guard.channel_messages.len() - MAX_CHANNEL_MESSAGES;
            guard.channel_messages.drain(0..remove_count);
        }
    }
}

fn runtime_key(workspace_id: &str, agent_id: &str) -> String {
    format!("{workspace_id}:{agent_id}")
}

fn purge_agent_messages_locked(guard: &mut TaskServiceState, workspace_id: &str, agent_id: &str) {
    guard.channel_messages.retain(|message| {
        if message.workspace_id != workspace_id {
            return true;
        }
        message.target_agent_id != agent_id && message.sender_agent_id.as_deref() != Some(agent_id)
    });
}

fn message_task_id(message: &ChannelMessageEvent) -> Option<&str> {
    message
        .payload
        .get("taskId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn message_involves_agent(message: &ChannelMessageEvent, agent_id: &str) -> bool {
    message.target_agent_id == agent_id || message.sender_agent_id.as_deref() == Some(agent_id)
}

fn build_task_thread_summary(
    task_id: &str,
    messages: &[ChannelMessageEvent],
) -> Option<TaskThreadSummary> {
    let first = messages.first()?;
    let latest = messages.last()?;
    Some(TaskThreadSummary {
        task_id: task_id.to_string(),
        title: task_thread_title(messages).unwrap_or_else(|| sanitize_title(task_id)),
        state: task_thread_state(messages),
        root_message_id: first.message_id.clone(),
        latest_message_id: latest.message_id.clone(),
        latest_message_type: latest.message_type.clone(),
        latest_sender_agent_id: latest.sender_agent_id.clone(),
        latest_target_agent_id: latest.target_agent_id.clone(),
        updated_at_ms: latest.ts_ms,
    })
}

fn task_thread_title(messages: &[ChannelMessageEvent]) -> Option<String> {
    messages
        .iter()
        .filter_map(|message| message.payload.get("title").and_then(Value::as_str))
        .map(sanitize_title)
        .find(|title| !title.trim().is_empty())
}

fn task_thread_state(messages: &[ChannelMessageEvent]) -> TaskThreadState {
    let Some(instruction) = messages
        .iter()
        .find(|message| message.message_type == ChannelMessageType::TaskInstruction)
    else {
        return TaskThreadState::Open;
    };
    let assignee_id = instruction.target_agent_id.as_str();

    // A reminder from the requester is still outbound traffic. Only a message
    // authored by the original assignee proves that the assignee replied.
    if messages.iter().any(|message| {
        message.sender_agent_id.as_deref() == Some(assignee_id)
            && message.message_type == ChannelMessageType::Handover
    }) {
        TaskThreadState::HandedOver
    } else if messages.iter().any(|message| {
        message.sender_agent_id.as_deref() == Some(assignee_id)
            && message.message_type == ChannelMessageType::Status
    }) {
        TaskThreadState::Replied
    } else {
        TaskThreadState::Open
    }
}

fn normalize_token(value: &str) -> String {
    value.trim().to_lowercase()
}

fn normalize_optional_token(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(normalize_token)
        .filter(|value| !value.is_empty())
}

fn normalize_optional_text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_account_id(value: &str) -> String {
    let normalized = normalize_token(value);
    if normalized.is_empty() {
        "default".to_string()
    } else {
        normalized
    }
}

fn normalize_route_binding_account_id(value: &Option<String>) -> String {
    let normalized = value.as_deref().map(normalize_token).unwrap_or_default();
    if normalized.is_empty() {
        "default".to_string()
    } else {
        normalized
    }
}

fn route_binding_identity_matches(left: &ChannelRouteBinding, right: &ChannelRouteBinding) -> bool {
    left.workspace_id == right.workspace_id
        && normalize_token(&left.channel) == normalize_token(&right.channel)
        && normalize_route_binding_account_id(&left.account_id)
            == normalize_route_binding_account_id(&right.account_id)
        && left.peer_kind == right.peer_kind
        && normalize_optional_token(&left.peer_pattern)
            == normalize_optional_token(&right.peer_pattern)
}

fn default_binding_enabled() -> bool {
    true
}

fn normalize_binding(mut binding: ChannelRouteBinding) -> ChannelRouteBinding {
    binding.channel = normalize_token(&binding.channel);
    binding.workspace_id = binding.workspace_id.trim().to_string();
    binding.target_agent_id = binding.target_agent_id.trim().to_string();
    binding.account_id = normalize_optional_token(&binding.account_id);
    binding.peer_pattern = binding
        .peer_pattern
        .as_deref()
        .map(str::trim)
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    binding.bot_name = normalize_optional_text(&binding.bot_name);
    binding
}

fn route_score(binding: &ChannelRouteBinding) -> i32 {
    let mut score = binding.priority.saturating_mul(1000);
    if binding.account_id.is_some() {
        score += 100;
    }
    if binding.peer_kind.is_some() {
        score += 50;
    }
    if binding.peer_pattern.is_some() {
        score += 30;
    }
    score
}

fn resolve_matched_by(binding: &ChannelRouteBinding) -> String {
    if binding.peer_pattern.is_some() {
        return "binding.peer".to_string();
    }
    if binding.account_id.is_some() {
        return "binding.account".to_string();
    }
    "binding.channel".to_string()
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern.eq_ignore_ascii_case(value.trim());
    }
    let mut remaining = value.to_lowercase();
    for segment in pattern
        .to_lowercase()
        .split('*')
        .filter(|segment| !segment.is_empty())
    {
        if let Some(pos) = remaining.find(segment) {
            remaining = remaining[(pos + segment.len())..].to_string();
        } else {
            return false;
        }
    }
    true
}

fn binding_account_matches(binding: &ChannelRouteBinding, account_id: &str) -> bool {
    match binding.account_id.as_deref() {
        None => true,
        Some("*") => true,
        Some(value) => normalize_token(value) == account_id,
    }
}

fn access_policy_key(channel: &str, account_id: &str) -> String {
    format!(
        "{}:{}",
        normalize_token(channel),
        normalize_account_id(account_id)
    )
}

fn allowlist_key(channel: &str, account_id: &str, identity: &str) -> String {
    format!(
        "{}:{}:{}",
        normalize_token(channel),
        normalize_account_id(account_id),
        normalize_token(identity)
    )
}

fn pairing_key(channel: &str, account_id: &str, identity: &str) -> String {
    allowlist_key(channel, account_id, identity)
}

fn parse_allowlist_key(value: &str) -> Option<(String, String, String)> {
    let mut segments = value.splitn(3, ':');
    let channel = segments.next()?.to_string();
    let account_id = segments.next()?.to_string();
    let identity = segments.next()?.to_string();
    Some((channel, account_id, identity))
}

fn generate_pairing_code(seed: u64) -> String {
    let value = (seed ^ now_ms()) & 0xFFFF_FFFF;
    format!("{value:08X}")
}

#[derive(Debug, Clone)]
struct PairingRequestRecord {
    code: String,
    expires_at_ms: u64,
    last_seen_at_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_agent_ids(raw: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in raw {
        let normalized = value.trim();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_string()) {
            result.push(normalized.to_string());
        }
    }
    result
}

fn resolve_publish_targets(request: &ChannelPublishRequest) -> Vec<String> {
    let targets = normalize_agent_ids(&request.target_agent_ids);
    if !targets.is_empty() {
        return targets;
    }
    match request.channel.kind {
        ChannelKind::Direct => normalize_agent_ids(std::slice::from_ref(&request.channel.id)),
        ChannelKind::Group | ChannelKind::Broadcast => Vec::new(),
    }
}

fn display_channel_id(kind: &ChannelKind, id: &str, direct_target: Option<&str>) -> String {
    match kind {
        ChannelKind::Direct => {
            let target = direct_target.unwrap_or(id);
            format!("direct://{target}")
        }
        ChannelKind::Group => format!("group://{id}"),
        ChannelKind::Broadcast => format!("broadcast://{id}"),
    }
}

fn sanitize_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return "未命名任务".to_string();
    }
    trimmed.to_string()
}

fn is_digits_only(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_csi_u_enter_sequence(raw: &str) -> bool {
    if raw == "\x1b[13u" {
        return true;
    }
    raw.strip_prefix("\x1b[13;")
        .and_then(|tail| tail.strip_suffix('u'))
        .is_some_and(is_digits_only)
}

fn is_csi_tilde_enter_sequence(raw: &str) -> bool {
    if raw == "\x1b[13~" {
        return true;
    }
    raw.strip_prefix("\x1b[13;")
        .and_then(|tail| tail.strip_suffix('~'))
        .is_some_and(is_digits_only)
}

fn is_modify_other_keys_enter_sequence(raw: &str) -> bool {
    raw.strip_prefix("\x1b[27;13;")
        .and_then(|tail| tail.strip_suffix('~'))
        .is_some_and(is_digits_only)
}

fn normalize_submit_sequence(raw: &str) -> Option<String> {
    match raw {
        "\r" | "\n" | "\r\n" => Some("\r".to_string()),
        "\x1bOM" => Some(raw.to_string()),
        _ if is_csi_u_enter_sequence(raw) => Some(raw.to_string()),
        _ if is_csi_tilde_enter_sequence(raw) => Some(raw.to_string()),
        _ if is_modify_other_keys_enter_sequence(raw) => Some(raw.to_string()),
        _ => None,
    }
}

fn resolve_submit_sequence(
    request: &TaskDispatchBatchRequest,
    target_agent_id: &str,
    runtime: &AgentRuntimeRegistration,
) -> String {
    if let Some(sequence) = runtime
        .submit_sequence
        .as_deref()
        .and_then(normalize_submit_sequence)
    {
        return sequence;
    }
    request
        .submit_sequences
        .get(target_agent_id)
        .and_then(|value| normalize_submit_sequence(value))
        .unwrap_or_else(|| "\r".to_string())
}

fn build_task_dispatch_command(markdown: &str) -> String {
    // Interactive agent TUIs interpret raw CR/LF bytes as submit actions unless
    // the text arrives through a bracketed-paste path. Task dispatch writes
    // directly to the PTY, so normalize the complete instruction into one line
    // and let the caller append exactly one submit sequence afterward.
    markdown.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_managed_agent_reply_instruction(
    request: &TaskDispatchBatchRequest,
    task_id: &str,
) -> Option<String> {
    if request.sender.sender_type != DispatchSenderType::Agent {
        return None;
    }
    let sender_agent_id = request
        .sender
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    Some(format!(
        "## GT Office Reply\n\nDo not reply only inside your own terminal.\n\nIf you need to reply to the original sender, use `gto` directly:\n- `gto agent reply-status --task-id {task_id} --target-agent-id {sender_agent_id} --detail \\\"<your reply text>\\\" --workspace-id {workspace_id} --json`\n\nIf you need to return a completion summary, blockers, or next steps, use:\n- `gto agent handover --task-id {task_id} --target-agent-id {sender_agent_id} --summary \\\"<summary>\\\" --workspace-id {workspace_id} --json`\n",
        workspace_id = request.workspace_id,
        sender_agent_id = sender_agent_id,
        task_id = task_id,
    ))
}

// Everything dispatch appends after the sender's own markdown — the reply
// instruction (agent senders only) and the output-collection instructions
// (targets with `output_collection_enabled`). Split out from
// `enrich_dispatch_markdown` so a preview can compute exactly this, without
// duplicating the template text (and risking it drifting from what actually
// gets sent).
fn dispatch_appended_sections(
    request: &TaskDispatchBatchRequest,
    task_id: &str,
    output_directory: Option<&String>,
    session_boundary_enabled: bool,
) -> Vec<String> {
    let mut sections = Vec::new();
    if let Some(reply_instruction) = build_managed_agent_reply_instruction(request, task_id) {
        sections.push(reply_instruction.trim().to_string());
    }
    // Self-enforcing regardless of caller: a caller that resolves real
    // output-collection settings but still sets this flag (or forgets to
    // special-case it) must not have that data leak into the appended text.
    // `task_dispatch_batch` also skips resolving those settings in the first
    // place when suppressed, purely as an optimization (no pointless
    // `.gtoffice/agents/<id>/outputs` directory creation) — never rely on
    // that as the actual enforcement point.
    if let Some(output_directory) =
        output_directory.filter(|_| !request.suppress_output_collection_instructions)
    {
        // The `.claude/session-handoff` sentence below must never contradict
        // the CLAUDE.md/AGENTS.md guidance `session_boundary_guidance_block`
        // (apps/desktop-tauri/src-tauri/src/commands/agent.rs) writes for an
        // agent with `session_boundary_auto_split_enabled` on — that guidance
        // instructs the agent to actively write into `.claude/session-handoff`
        // when it judges a task boundary reached. A blanket "do not touch"
        // here previously collided with it (docs/cw/19_輸出收集與SessionBoundary訊息衝突排查.md), leaving
        // the agent with two simultaneous, opposite instructions about the
        // same path.
        let handoff_sentence = if session_boundary_enabled {
            "`.claude/session-handoff` is reserved for this agent's separate session-boundary auto-restart signal (see your system prompt's own boundary guidance) — do not use it for GT Office output-collection handoffs; use `GTO_HANDOFF_FILE` for those instead."
        } else {
            "Do not copy, move, or delete files in `.claude/session-handoff`. These are task-scoped execution paths, not system-prompt rules."
        };
        sections.push(format!(
            "## GT Office Output\n\nThe managed output directory is available as `GTO_OUTPUT_DIR` (`{output_directory}`). Maintain this session's work record by appending important progress, decisions, changed files, and verification results to `GTO_LOG_FILE`. When a handoff is needed, replace the file at `GTO_HANDOFF_FILE`; it is the only GT Office handoff retained for this agent. If the user requests an additional deliverable such as Markdown, HTML, or JSON, write it as a separate file in `GTO_ARTIFACT_DIR`. {handoff_sentence}"
        ));
    }
    sections
}

fn enrich_dispatch_markdown(
    markdown: &str,
    request: &TaskDispatchBatchRequest,
    task_id: &str,
    output_directory: Option<&String>,
    session_boundary_enabled: bool,
) -> String {
    let mut sections = Vec::new();
    let body = markdown.trim();
    if !body.is_empty() {
        sections.push(body.to_string());
    }
    sections.extend(dispatch_appended_sections(
        request,
        task_id,
        output_directory,
        session_boundary_enabled,
    ));
    sections.join("\n\n")
}

pub fn module_name() -> &'static str {
    "gt-task"
}

#[cfg(test)]
mod output_collection_tests {
    use super::*;

    #[test]
    fn task_scoped_output_instruction_names_env_and_directory() {
        let request = TaskDispatchBatchRequest {
            workspace_id: "ws-1".to_string(),
            sender: DispatchSender::default(),
            targets: vec!["agent-1".to_string()],
            title: "Report".to_string(),
            markdown: "Create a report".to_string(),
            attachments: vec![],
            submit_sequences: HashMap::new(),
            suppress_output_collection_instructions: false,
        };
        let output_dir = "C:/project/.gtoffice/agents/agent-1/outputs".to_string();
        let enriched = enrich_dispatch_markdown(
            &request.markdown,
            &request,
            "task-1",
            Some(&output_dir),
            false,
        );
        assert!(enriched.contains("GTO_OUTPUT_DIR"));
        assert!(enriched.contains("GTO_LOG_FILE"));
        assert!(enriched.contains("GTO_HANDOFF_FILE"));
        assert!(enriched.contains("GTO_ARTIFACT_DIR"));
        assert!(enriched.contains(".claude/session-handoff"));
        assert!(enriched.contains(&output_dir));
        assert!(enriched.contains("task-scoped execution paths"));
    }

    #[test]
    fn suppress_output_collection_instructions_wins_even_when_the_caller_still_resolves_real_settings(
    ) {
        // The Tauri command layer skips resolving output-collection settings
        // when this flag is set, purely as an optimization — but the flag
        // must be self-enforcing at this layer too, for any caller (present
        // or future) that resolves real settings anyway and passes them in
        // regardless.
        let request = TaskDispatchBatchRequest {
            workspace_id: "ws-1".to_string(),
            sender: DispatchSender::default(),
            targets: vec!["agent-1".to_string()],
            title: "Report".to_string(),
            markdown: "Create a report".to_string(),
            attachments: vec![],
            submit_sequences: HashMap::new(),
            suppress_output_collection_instructions: true,
        };
        let output_dir = "C:/project/.gtoffice/agents/agent-1/outputs".to_string();
        let enriched = enrich_dispatch_markdown(
            &request.markdown,
            &request,
            "task-1",
            Some(&output_dir),
            false,
        );
        assert_eq!(enriched, "Create a report");
        assert!(!enriched.contains("GTO_OUTPUT_DIR"));
        assert!(!enriched.contains(&output_dir));
    }

    // Regression test for docs/cw/19_輸出收集與SessionBoundary訊息衝突排查.md: when an agent has BOTH
    // output_collection_enabled and session_boundary_auto_split_enabled on,
    // the output-collection guidance must not tell it to leave
    // `.claude/session-handoff` alone — that directly contradicts the
    // separate CLAUDE.md/AGENTS.md guidance instructing it to actively write
    // there when it judges a task boundary reached.
    #[test]
    fn output_instruction_does_not_contradict_session_boundary_guidance_when_both_enabled() {
        let request = TaskDispatchBatchRequest {
            workspace_id: "ws-1".to_string(),
            sender: DispatchSender::default(),
            targets: vec!["agent-1".to_string()],
            title: "Report".to_string(),
            markdown: "Create a report".to_string(),
            attachments: vec![],
            submit_sequences: HashMap::new(),
            suppress_output_collection_instructions: false,
        };
        let output_dir = "C:/project/.gtoffice/agents/agent-1/outputs".to_string();
        let enriched = enrich_dispatch_markdown(
            &request.markdown,
            &request,
            "task-1",
            Some(&output_dir),
            true,
        );
        assert!(enriched.contains("GTO_HANDOFF_FILE"));
        assert!(enriched.contains(".claude/session-handoff"));
        assert!(
            !enriched.contains("Do not copy, move, or delete files in `.claude/session-handoff`")
        );
        assert!(!enriched.contains("task-scoped execution paths"));
    }

    #[test]
    fn preview_appended_sections_matches_what_dispatch_would_actually_append() {
        let service = TaskService::default();
        let output_directories: HashMap<String, String> = [(
            "agent-1".to_string(),
            "C:/project/.gtoffice/agents/agent-1/outputs".to_string(),
        )]
        .into_iter()
        .collect();
        let session_boundary_agents: HashSet<String> = HashSet::new();

        let previews = service.preview_dispatch_appended_sections(
            "ws-1",
            &["agent-1".to_string(), "agent-2".to_string()],
            &output_directories,
            &session_boundary_agents,
        );

        let agent_1 = previews
            .iter()
            .find(|preview| preview.target_agent_id == "agent-1")
            .expect("agent-1 preview present");
        assert!(agent_1.appended_text.contains("GTO_OUTPUT_DIR"));
        // Must never leak the sender's own markdown into the appended text —
        // a preview is only what dispatch adds on top of it.
        assert!(!agent_1
            .appended_text
            .contains("this markdown must never appear"));

        let agent_2 = previews
            .iter()
            .find(|preview| preview.target_agent_id == "agent-2")
            .expect("agent-2 preview present");
        assert_eq!(agent_2.appended_text, "");
    }

    #[test]
    fn preview_never_includes_the_agent_reply_instruction_since_task_brief_always_sends_as_human() {
        // preview_dispatch_appended_sections always builds its synthetic
        // request with the default (Human) sender — this locks that in, since
        // the reply-instruction block only makes sense for agent-to-agent
        // dispatch and Task Brief (its only caller) is always human-authored.
        let service = TaskService::default();
        let previews = service.preview_dispatch_appended_sections(
            "ws-1",
            &["agent-1".to_string()],
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].appended_text, "");
    }
}

#[cfg(test)]
mod p3_execution_policy_tests {
    use super::*;
    use gt_agent::AgentPolicy;

    struct FixedAgentPolicyProvider {
        policy: AgentPolicy,
    }

    impl AgentPolicyProvider for FixedAgentPolicyProvider {
        fn policy_for(&self, _workspace_id: &WorkspaceId, _agent_id: &str) -> AgentPolicy {
            self.policy.clone()
        }
    }

    fn register_target(service: &TaskService, workspace_id: &str, agent_id: &str) {
        service.register_runtime(AgentRuntimeRegistration {
            workspace_id: workspace_id.to_string(),
            agent_id: agent_id.to_string(),
            station_id: agent_id.to_string(),
            session_id: format!("session-{agent_id}"),
            tool_kind: AgentToolKind::Unknown,
            resolved_cwd: None,
            submit_sequence: None,
            provider_session: None,
            online: true,
        });
    }

    #[test]
    fn max_concurrency_caps_targets_dispatched_from_a_policy_bound_sender() {
        let service = TaskService::default();
        let mut policy = AgentPolicy::default();
        policy.execution.max_concurrency = Some(1);
        service.set_agent_policy_provider(Arc::new(FixedAgentPolicyProvider { policy }));

        register_target(&service, "ws-1", "agent-a");
        register_target(&service, "ws-1", "agent-b");

        let request = TaskDispatchBatchRequest {
            workspace_id: "ws-1".to_string(),
            sender: DispatchSender {
                sender_type: DispatchSenderType::Agent,
                agent_id: Some("agent-sender".to_string()),
            },
            targets: vec!["agent-a".to_string(), "agent-b".to_string()],
            title: "test".to_string(),
            markdown: "do the thing".to_string(),
            attachments: Vec::new(),
            submit_sequences: HashMap::new(),
            suppress_output_collection_instructions: false,
        };

        let outcome = service.dispatch_batch(&request, Path::new("."), |_, _, _| Ok(()));

        assert_eq!(outcome.response.results.len(), 2);
        let sent_count = outcome
            .response
            .results
            .iter()
            .filter(|result| result.status == TaskDispatchStatus::Sent)
            .count();
        assert_eq!(sent_count, 1, "only max_concurrency targets should be sent");
        let denied = outcome
            .response
            .results
            .iter()
            .find(|result| result.status == TaskDispatchStatus::Failed)
            .expect("one target should be policy-denied");
        assert_eq!(
            denied.detail.as_deref(),
            Some("AGENT_POLICY_CONCURRENCY_EXCEEDED")
        );
    }

    #[test]
    fn human_sender_is_not_subject_to_agent_execution_policy() {
        let service = TaskService::default();
        let mut policy = AgentPolicy::default();
        policy.execution.max_concurrency = Some(1);
        service.set_agent_policy_provider(Arc::new(FixedAgentPolicyProvider { policy }));

        register_target(&service, "ws-1", "agent-a");
        register_target(&service, "ws-1", "agent-b");

        let request = TaskDispatchBatchRequest {
            workspace_id: "ws-1".to_string(),
            sender: DispatchSender {
                sender_type: DispatchSenderType::Human,
                agent_id: None,
            },
            targets: vec!["agent-a".to_string(), "agent-b".to_string()],
            title: "test".to_string(),
            markdown: "do the thing".to_string(),
            attachments: Vec::new(),
            submit_sequences: HashMap::new(),
            suppress_output_collection_instructions: false,
        };

        let outcome = service.dispatch_batch(&request, Path::new("."), |_, _, _| Ok(()));

        let sent_count = outcome
            .response
            .results
            .iter()
            .filter(|result| result.status == TaskDispatchStatus::Sent)
            .count();
        assert_eq!(
            sent_count, 2,
            "a human-initiated dispatch has no agent policy to enforce"
        );
    }
}

#[cfg(test)]
mod p4_agent_canvas_tests {
    use super::*;

    fn register(service: &TaskService, workspace_id: &str, agent_id: &str, online: bool) {
        service.register_runtime(AgentRuntimeRegistration {
            workspace_id: workspace_id.to_string(),
            agent_id: agent_id.to_string(),
            station_id: agent_id.to_string(),
            session_id: format!("session-{agent_id}"),
            tool_kind: AgentToolKind::Unknown,
            resolved_cwd: None,
            submit_sequence: None,
            provider_session: None,
            online,
        });
    }

    #[test]
    fn agent_runtime_status_reports_idle_and_active_for_registered_agents_only() {
        let service = TaskService::default();
        // register_runtime with online:false removes any entry rather than
        // storing one, so "offline" agents never show up here at all — the
        // agent-canvas command layer is expected to treat any agent id absent
        // from this result as Offline.
        register(&service, "ws-1", "agent-offline", false);
        register(&service, "ws-1", "agent-idle", true);
        register(&service, "ws-1", "agent-active", true);

        // Sender is deliberately a third, unregistered id so it can't
        // accidentally satisfy the "agent-idle" or "agent-active" assertions
        // below via its own sender_agent_id match.
        let request = TaskDispatchBatchRequest {
            workspace_id: "ws-1".to_string(),
            sender: DispatchSender {
                sender_type: DispatchSenderType::Agent,
                agent_id: Some("agent-external-sender".to_string()),
            },
            targets: vec!["agent-active".to_string()],
            title: "test".to_string(),
            markdown: "do the thing".to_string(),
            attachments: Vec::new(),
            submit_sequences: HashMap::new(),
            suppress_output_collection_instructions: false,
        };
        let outcome = service.dispatch_batch(&request, Path::new("."), |_, _, _| Ok(()));
        assert_eq!(outcome.response.results[0].status, TaskDispatchStatus::Sent);

        let statuses = service.agent_runtime_status("ws-1");
        let find = |agent_id: &str| {
            statuses
                .iter()
                .find(|status| status.agent_id == agent_id)
                .unwrap_or_else(|| panic!("expected status for {agent_id}"))
        };

        assert!(
            !statuses
                .iter()
                .any(|status| status.agent_id == "agent-offline"),
            "an agent registered with online:false must not appear in the result at all"
        );
        assert_eq!(
            find("agent-idle").state,
            gt_agent::AgentRuntimeState::Idle,
            "registered and online, but not party to any recent dispatch"
        );
        assert_eq!(
            find("agent-active").state,
            gt_agent::AgentRuntimeState::Idle,
            "channel messages alone must not mark a terminal as Active"
        );
        assert!(
            !statuses
                .iter()
                .any(|status| status.agent_id == "agent-not-registered"),
            "agent-canvas should treat a missing registration as Unknown by simply not finding a row, not by fabricating one"
        );
    }

    #[test]
    fn agent_runtime_status_is_scoped_to_workspace() {
        let service = TaskService::default();
        register(&service, "ws-1", "agent-a", true);
        register(&service, "ws-2", "agent-b", true);

        let statuses = service.agent_runtime_status("ws-1");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].agent_id, "agent-a");
    }
}
