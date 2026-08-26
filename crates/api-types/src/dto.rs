//! Request and response payloads.
//!
//! These are the authoritative shapes. `packages/shared-types` and
//! `packages/api-contracts` are generated from them, so a change here that is
//! not regenerated fails CI rather than drifting silently.
//!
//! Note what is absent: no *response* in this module can carry a decrypted
//! secret. [`EnvVarSummary`] models a secret as `value: None` plus
//! `is_set: true`, which is the only shape the API ever produces for one.
//!
//! Two request types necessarily carry a secret inbound — [`EnvVarInput`] and
//! [`SourceCredential`] — because a value has to arrive somehow. Both are
//! write-only: nothing that is serialised back to a caller contains them, and
//! [`SourceCredential`] redacts itself in `Debug`.

use core::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::enums::*;
use crate::ids::*;

// ---------------------------------------------------------------- server

/// Answer to "what am I connected to". Fetched immediately after authenticating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServerInfo {
    pub agent_version: String,
    pub schema_version: u32,
    /// Changes on every agent restart. A client seeing a new value knows its
    /// cached stream state is stale.
    pub instance_id: String,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub hostname: String,
    pub agent_uptime_seconds: u64,
    pub host_uptime_seconds: u64,
    pub bind_address: String,
    pub lan_enabled: bool,
    pub capabilities: PlatformCapabilities,
}

/// What this platform can actually do. The UI hides what is unavailable rather
/// than showing a control that silently does nothing — see
/// `docs/platform-support.md` §5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlatformCapabilities {
    pub cpu_temperature: bool,
    pub per_container_disk_io: bool,
    pub storage_quota_enforcement: bool,
    pub linux_capability_dropping: bool,
    pub read_only_root_filesystem: bool,
    /// Which secure-storage backend is really in use. Reported so the UI can
    /// tell the truth when it has fallen back to an encrypted key file.
    pub secure_storage_backend: String,
    pub firewall_management: bool,
}

/// The independent connectivity states from `docs/architecture.md` §8.
/// Deliberately not collapsed into one flag: an unplugged cable and an
/// unreachable agent need different remedies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Connectivity {
    pub agent: Availability,
    pub lan: Availability,
    pub internet: Availability,
    pub checked_at: String,
}

// ---------------------------------------------------------------- metrics

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HostMetrics {
    pub sampled_at: String,
    pub cpu_percent: f32,
    /// `None` where the platform does not expose it, which is common on Windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_temperature_c: Option<f32>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_read_bytes_per_sec: u64,
    pub disk_write_bytes_per_sec: u64,
    pub net_rx_bytes_per_sec: u64,
    pub net_tx_bytes_per_sec: u64,
    pub process_count: u32,
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectMetrics {
    pub project_id: ProjectId,
    pub sampled_at: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub memory_limit_bytes: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_read_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_write_bytes: Option<u64>,
}

// ---------------------------------------------------------------- projects

/// Row shape for the project list. Deliberately smaller than [`ProjectDetail`]
/// so listing many projects stays cheap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub slug: String,
    pub display_name: String,
    pub project_type: ProjectType,
    pub runtime: Runtime,
    pub status: ProjectStatus,
    pub desired_state: DesiredState,
    pub health: HealthState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    pub restart_count: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectDetail {
    #[serde(flatten)]
    pub summary: ProjectSummary,
    pub description: String,
    pub source_type: SourceType,
    /// Where a `GIT_CLONE` or `REMOTE_ARCHIVE` project came from. The URL is
    /// safe to return because a token never travels inside it — see
    /// [`ProjectSource::credential`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// The commit that was actually checked out, which is the only honest
    /// answer to "what is running" when the ref was a moving branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    /// Whether a token is stored for this project's remote. Deliberately a
    /// boolean: there is no route that returns the token itself.
    #[serde(default)]
    pub has_credential: bool,
    pub runtime_config: RuntimeConfig,
    pub resources: ResourceLimits,
    pub network: NetworkConfig,
    pub autostart: bool,
    pub restart_policy: RestartPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    pub container_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeConfig {
    pub runtime: Runtime,
    pub runtime_version: String,
    pub package_manager: PackageManager,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_command: Option<String>,
    pub start_command: String,
    pub working_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_dir: Option<String>,
    pub template_id: String,
    pub health_check: HealthCheckConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HealthCheckConfig {
    pub kind: HealthCheckType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub interval_seconds: u32,
    pub timeout_seconds: u32,
    pub retries: u32,
    pub start_period_seconds: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            kind: HealthCheckType::None,
            target: None,
            interval_seconds: 30,
            timeout_seconds: 5,
            retries: 3,
            start_period_seconds: 20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResourceLimits {
    pub memory_limit_mb: u32,
    pub cpu_limit_cores: f32,
    pub storage_limit_mb: u32,
    pub process_limit: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_limit_mb: 512,
            cpu_limit_cores: 1.0,
            storage_limit_mb: 2048,
            process_limit: 128,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkConfig {
    pub mode: NetworkMode,
    pub ports: Vec<PortMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PortMapping {
    pub id: PortId,
    pub port: u16,
    /// `None` until the agent allocates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    pub protocol: String,
    /// `127.0.0.1` unless the user explicitly asked for LAN exposure.
    pub bind_address: String,
    pub is_primary: bool,
}

/// What the creation wizard submits. The server generates the identifier, slug,
/// directory and container name; none of them derive from `display_name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CreateProjectRequest {
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub project_type: ProjectType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub source: ProjectSource,
    pub runtime_config: RuntimeConfig,
    #[serde(default)]
    pub resources: ResourceLimits,
    pub network: NetworkConfigRequest,
    pub autostart: bool,
    pub restart_policy: RestartPolicy,
    #[serde(default)]
    pub environment: Vec<EnvVarInput>,
}

/// Where the project's files come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectSource {
    pub kind: SourceType,
    /// Upload session id for `ZIP_UPLOAD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_id: Option<String>,
    /// Absolute host path for `LOCAL_FOLDER`. Validated server-side before use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// Source project for `DUPLICATE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_project_id: Option<ProjectId>,
    /// `https://` remote for `GIT_CLONE` and `REMOTE_ARCHIVE`. Validated
    /// against `file-manager`'s `remote_url` rules before any connection is
    /// opened, and again for every redirect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    /// Branch, tag or full commit id for `GIT_CLONE`. Omitted means the
    /// remote's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Path within the fetched tree to promote, for repositories that hold more
    /// than one project. Relative; traversal is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdirectory: Option<String>,
    /// Access token for a private remote. Write-only: this field is populated
    /// on the way in and is never returned, which is why responses carry
    /// `has_credential` instead. `Debug` is hand-written so a token cannot
    /// reach a log line through a derived formatter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<SourceCredential>,
}

/// A token for a private remote, kept in its own type so that the only way to
/// read it is to ask for it by name.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceCredential(String);

impl SourceCredential {
    pub fn new(token: String) -> Self {
        Self(token)
    }

    /// The plaintext. Every caller of this is a place a token could leak, so
    /// there is exactly one: the encryption step on the way to storage.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Redacted. A derived `Debug` here would put tokens in every error log that
/// formats a `CreateProjectRequest`.
impl fmt::Debug for SourceCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SourceCredential(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkConfigRequest {
    pub mode: NetworkMode,
    #[serde(default)]
    pub ports: Vec<PortRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PortRequest {
    pub port: u16,
    /// Omit to let the agent allocate from its pool. Values below 1024 are
    /// rejected, so privileged-port abuse is not expressible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    #[serde(default)]
    pub expose_to_lan: bool,
    #[serde(default)]
    pub is_primary: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateProjectRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autostart: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<RestartPolicy>,
}

/// Deleting requires echoing the display name. The confirmation is part of the
/// contract, not a dialog the API would happily let a caller skip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeleteProjectRequest {
    pub confirm_name: String,
    #[serde(default)]
    pub remove_volumes: bool,
}

/// Returned by any endpoint that starts background work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OperationHandle {
    pub operation_id: OperationId,
    pub project_id: ProjectId,
    pub kind: String,
    pub accepted_at: String,
}

// ---------------------------------------------------------------- env vars

/// A variable as the API returns it.
///
/// For a secret, `value` is always `None` and `is_set` reports whether one has
/// been stored. There is no representation in which a secret's value travels to
/// a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvVarSummary {
    pub id: EnvVarId,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub is_secret: bool,
    pub is_set: bool,
    pub restart_required: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvVarInput {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub is_secret: bool,
}

// ---------------------------------------------------------------- history

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeploymentSummary {
    pub id: DeploymentId,
    pub project_id: ProjectId,
    pub deployment_type: DeploymentType,
    pub status: DeploymentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContainerEventSummary {
    pub project_id: ProjectId,
    pub event_type: ContainerEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub occurred_at: String,
}

/// An audit entry. `target_label` is a copy rather than a join, so the record
/// still reads sensibly after the thing it describes has been deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditEntry {
    pub id: AuditId,
    pub occurred_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_addr: Option<String>,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    pub result: AuditResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

// ---------------------------------------------------------------- backups

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupSummary {
    pub id: BackupId,
    pub project_id: ProjectId,
    pub status: BackupStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_sha256: Option<String>,
    pub includes_files: bool,
    pub includes_volumes: bool,
    pub includes_config: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

// ---------------------------------------------------------------- logs

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogLine {
    pub timestamp: String,
    /// `stdout` or `stderr`. Kept distinct so the UI can colour and filter them.
    pub stream: String,
    pub message: String,
}

// ---------------------------------------------------------------- auth

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_label: Option<String>,
}

/// The token is returned exactly once, to the desktop client's Rust core, which
/// puts it in the OS keychain. It never reaches the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LoginResponse {
    pub token: String,
    pub session_id: SessionId,
    pub user: UserSummary,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserSummary {
    pub id: UserId,
    pub email: String,
    pub display_name: String,
    pub role: UserRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_login_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupStatus {
    pub administrator_exists: bool,
    pub agent_version: String,
    pub schema_version: u32,
}

/// Recovery codes are in the response because this is the only moment they
/// exist in plaintext. No route returns them again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupAdministratorResponse {
    pub user: UserSummary,
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationSummary {
    pub id: NotificationId,
    pub level: NotificationLevel,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_at: Option<String>,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_variable_has_no_value_on_the_wire() {
        let secret = EnvVarSummary {
            id: EnvVarId::generate(),
            key: "DISCORD_TOKEN".to_string(),
            value: None,
            is_secret: true,
            is_set: true,
            restart_required: true,
            updated_at: "2026-07-29T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&secret).unwrap();
        assert!(
            !json.contains("\"value\""),
            "secret leaked a value key: {json}"
        );
        assert!(json.contains("\"is_set\":true"));
    }

    #[test]
    fn defaults_match_the_documented_resource_limits() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.memory_limit_mb, 512);
        assert_eq!(limits.storage_limit_mb, 2048);
        assert_eq!(limits.process_limit, 128);
        assert!((limits.cpu_limit_cores - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn health_check_defaults_to_none_rather_than_a_check_that_always_passes() {
        assert_eq!(HealthCheckConfig::default().kind, HealthCheckType::None);
        assert_eq!(HealthCheckConfig::default().target, None);
    }

    #[test]
    fn project_detail_flattens_its_summary() {
        let detail = sample_detail();
        let json = serde_json::to_value(&detail).unwrap();
        // `summary` must not appear as a nested key: the client sees one object.
        assert!(json.get("summary").is_none());
        assert_eq!(json["display_name"], serde_json::json!("My Bot"));
        assert_eq!(json["status"], serde_json::json!("RUNNING"));
    }

    #[test]
    fn project_detail_round_trips() {
        let detail = sample_detail();
        let json = serde_json::to_string(&detail).unwrap();
        let back: ProjectDetail = serde_json::from_str(&json).unwrap();
        assert_eq!(back, detail);
    }

    fn sample_detail() -> ProjectDetail {
        ProjectDetail {
            summary: ProjectSummary {
                id: ProjectId::generate(),
                slug: "quiet-harbor-4f2a".to_string(),
                display_name: "My Bot".to_string(),
                project_type: ProjectType::DiscordBot,
                runtime: Runtime::NodeJs,
                status: ProjectStatus::Running,
                desired_state: DesiredState::Running,
                health: HealthState::None,
                icon: None,
                color: None,
                started_at: Some("2026-07-29T00:00:00Z".to_string()),
                restart_count: 0,
                created_at: "2026-07-29T00:00:00Z".to_string(),
            },
            description: String::new(),
            source_type: SourceType::ZipUpload,
            source_url: None,
            source_ref: None,
            source_commit: None,
            has_credential: false,
            runtime_config: RuntimeConfig {
                runtime: Runtime::NodeJs,
                runtime_version: "22".to_string(),
                package_manager: PackageManager::Pnpm,
                install_command: Some("pnpm install --frozen-lockfile".to_string()),
                build_command: None,
                start_command: "node index.js".to_string(),
                working_dir: "/app".to_string(),
                entry_file: Some("index.js".to_string()),
                publish_dir: None,
                template_id: "nodejs".to_string(),
                health_check: HealthCheckConfig::default(),
            },
            resources: ResourceLimits::default(),
            network: NetworkConfig {
                mode: NetworkMode::Internet,
                ports: Vec::new(),
            },
            autostart: true,
            restart_policy: RestartPolicy::UnlessStopped,
            container_id: None,
            container_name: "ph_quiet-harbor-4f2a".to_string(),
            image_tag: None,
            last_exit_code: None,
            last_failure_at: None,
            last_failure_reason: None,
            archived_at: None,
            updated_at: "2026-07-29T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn a_source_credential_redacts_itself_in_debug() {
        let credential = SourceCredential::new("ghp_realtokenvalue".to_string());
        let rendered = format!("{credential:?}");
        assert!(
            !rendered.contains("ghp_realtokenvalue"),
            "token reached a Debug rendering: {rendered}"
        );
        assert_eq!(rendered, "SourceCredential(<redacted>)");
    }

    #[test]
    fn a_credential_inside_a_request_does_not_leak_through_the_requests_debug() {
        // The realistic leak is not `{credential:?}` but a handler logging the
        // whole request it failed to process.
        let source = ProjectSource {
            kind: SourceType::GitClone,
            upload_id: None,
            local_path: None,
            source_project_id: None,
            repo_url: Some("https://github.com/owner/repo.git".to_string()),
            git_ref: Some("main".to_string()),
            subdirectory: None,
            credential: Some(SourceCredential::new("ghp_realtokenvalue".to_string())),
        };
        let rendered = format!("{source:?}");
        assert!(
            !rendered.contains("ghp_realtokenvalue"),
            "token reached a Debug rendering: {rendered}"
        );
        // The URL is not a secret and stays visible, which is what makes the
        // rendering worth logging at all.
        assert!(rendered.contains("github.com/owner/repo.git"));
    }

    #[test]
    fn project_detail_reports_a_credential_as_a_boolean_and_never_a_value() {
        let mut detail = sample_detail();
        detail.source_type = SourceType::GitClone;
        detail.source_url = Some("https://github.com/owner/repo.git".to_string());
        detail.source_ref = Some("main".to_string());
        detail.source_commit = Some("0f5c1d0a".to_string());
        detail.has_credential = true;

        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains("\"has_credential\":true"));
        assert!(
            !json.contains("credential\":\"") && !json.contains("token"),
            "a detail response grew a credential-shaped field: {json}"
        );
    }

    #[test]
    fn the_new_sources_survive_the_wire() {
        for kind in [SourceType::GitClone, SourceType::RemoteArchive] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: SourceType = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back, "{json} did not round trip");
        }
        assert_eq!(
            serde_json::to_string(&SourceType::GitClone).unwrap(),
            "\"GIT_CLONE\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::RemoteArchive).unwrap(),
            "\"REMOTE_ARCHIVE\""
        );
    }
}
