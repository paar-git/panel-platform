// ---------------------------------------------------------------------------
// GENERATED FILE — DO NOT EDIT.
//
// Produced from crates/api-types by `cargo run -p project-host-api-types
// --bin emit-contracts`. Edit the Rust types and regenerate; CI fails if this
// file differs from what the generator produces.
// ---------------------------------------------------------------------------

/** Cursor-paginated result set. */
export interface Page<T> {
  items: T[];
  next_cursor?: string | null;
  has_more: boolean;
}

/** Metadata present on every successful response. */
export interface ResponseMetaEnvelope {
  request_id: string;
  server_time: string;
}

/** Success or failure, discriminated by `ok`. */
export type ApiResponse<T> =
  | { ok: true; data: T; meta: ResponseMetaEnvelope }
  | { ok: false; error: ApiError };

/** Machine-readable outcome. Stable across releases. */
export type ErrorCode = 'VALIDATION_FAILED' | 'UNAUTHENTICATED' | 'SESSION_EXPIRED' | 'FORBIDDEN' | 'NOT_FOUND' | 'CONFLICT' | 'PROJECT_LOCKED' | 'OPERATION_IN_PROGRESS' | 'PRECONDITION_FAILED' | 'PAYLOAD_TOO_LARGE' | 'RATE_LIMITED' | 'PORT_UNAVAILABLE' | 'RESOURCE_LIMIT_EXCEEDED' | 'ARCHIVE_REJECTED' | 'PATH_REJECTED' | 'INTEGRITY_CHECK_FAILED' | 'SETUP_REQUIRED' | 'AGENT_STARTING' | 'INTERNAL';

/** One field that failed validation. */
export interface FieldError {
  /** Dotted path to the offending field, e.g. `resources.memory_limit_mb`. */
  field: string;
  /** Why it was rejected, phrased for a person. */
  message: string;
}

/** The error half of the response envelope. */
export interface ApiError {
  code: ErrorCode;
  /** Structured, non-sensitive context, e.g. `{"held_by":"RESTORE"}`. */
  details?: Record<string, string>;
  /** Present only for `VALIDATION_FAILED`. */
  fields?: FieldError[];
  /** Written for a person. Never a stack trace, never a raw driver message. */
  message: string;
  /** Correlates this failure with the agent log. */
  request_id: string;
}

/** Identifier prefixed with `aud_`. */
export type AuditId = string;

export type AuditResult = 'SUCCESS' | 'FAILURE' | 'DENIED';

/** Identifier prefixed with `usr_`. */
export type UserId = string;

/** An audit entry. `target_label` is a copy rather than a join, so the record still reads sensibly after the thing it describes has been deleted. */
export interface AuditEntry {
  action: string;
  client_label?: string | null;
  error_code?: string | null;
  id: AuditId;
  occurred_at: string;
  request_id?: string | null;
  result: AuditResult;
  source_addr?: string | null;
  target_label?: string | null;
  target_type?: string | null;
  user_id?: UserId | null;
}

/** The five connectivity states from `docs/architecture.md` §8, as one reusable tri-state. `Unknown` is a real answer — probing may be disabled. */
export type Availability = 'UNKNOWN' | 'AVAILABLE' | 'UNAVAILABLE';

/** Identifier prefixed with `bkp_`. */
export type BackupId = string;

export type BackupStatus = 'PENDING' | 'CREATING' | 'COMPLETED' | 'FAILED' | 'CANCELLED' | 'CORRUPT';

/** Identifier prefixed with `prj_`. */
export type ProjectId = string;

export interface BackupSummary {
  checksum_sha256?: string | null;
  completed_at?: string | null;
  created_at: string;
  id: BackupId;
  includes_config: boolean;
  includes_files: boolean;
  includes_volumes: boolean;
  note?: string | null;
  project_id: ProjectId;
  size_bytes?: number | null;
  status: BackupStatus;
  verified_at?: string | null;
}

/** The independent connectivity states from `docs/architecture.md` §8. Deliberately not collapsed into one flag: an unplugged cable and an unreachable agent need different remedies. */
export interface Connectivity {
  agent: Availability;
  checked_at: string;
  internet: Availability;
  lan: Availability;
}

export type ContainerEventType = 'CREATED' | 'STARTED' | 'STOPPED' | 'RESTARTED' | 'DIED' | 'OOM_KILLED' | 'HEALTH_PASS' | 'HEALTH_FAIL' | 'DESTROYED';

export interface ContainerEventSummary {
  detail?: string | null;
  event_type: ContainerEventType;
  exit_code?: number | null;
  occurred_at: string;
  project_id: ProjectId;
}

export interface EnvVarInput {
  is_secret?: boolean;
  key: string;
  value: string;
}

/** Per-project network reach. `Internal` is the default: a project gets a dedicated network with no outbound route until someone asks for more. */
export type NetworkMode = 'NONE' | 'INTERNAL' | 'LAN' | 'INTERNET';

export interface PortRequest {
  expose_to_lan?: boolean;
  /** Omit to let the agent allocate from its pool. Values below 1024 are rejected, so privileged-port abuse is not expressible. */
  host_port?: number | null;
  is_primary?: boolean;
  port: number;
}

export interface NetworkConfigRequest {
  mode: NetworkMode;
  ports?: PortRequest[];
}

/** A token for a private remote, kept in its own type so that the only way to read it is to ask for it by name. */
export type SourceCredential = string;

export type SourceType = 'EMPTY' | 'ZIP_UPLOAD' | 'LOCAL_FOLDER' | 'DUPLICATE' | 'IMPORT_ARCHIVE' | 'GIT_CLONE' | 'REMOTE_ARCHIVE';

/** Where the project's files come from. */
export interface ProjectSource {
  /** Access token for a private remote. Write-only: this field is populated on the way in and is never returned, which is why responses carry `has_credential` instead. `Debug` is hand-written so a token cannot reach a log line through a derived formatter. */
  credential?: SourceCredential | null;
  /** Branch, tag or full commit id for `GIT_CLONE`. Omitted means the remote's default branch. */
  git_ref?: string | null;
  kind: SourceType;
  /** Absolute host path for `LOCAL_FOLDER`. Validated server-side before use. */
  local_path?: string | null;
  /** `https://` remote for `GIT_CLONE` and `REMOTE_ARCHIVE`. Validated against `file-manager`'s `remote_url` rules before any connection is opened, and again for every redirect. */
  repo_url?: string | null;
  /** Source project for `DUPLICATE`. */
  source_project_id?: ProjectId | null;
  /** Path within the fetched tree to promote, for repositories that hold more than one project. Relative; traversal is refused. */
  subdirectory?: string | null;
  /** Upload session id for `ZIP_UPLOAD`. */
  upload_id?: string | null;
}

/** What the user says the project is. Affects presentation and defaults, never isolation — every type gets identical container hardening. */
export type ProjectType = 'DISCORD_BOT' | 'NODE_APP' | 'PYTHON_APP' | 'WEBSITE' | 'STATIC_SITE' | 'REST_API' | 'WORKER' | 'SERVICE';

export interface ResourceLimits {
  cpu_limit_cores: number;
  memory_limit_mb: number;
  process_limit: number;
  storage_limit_mb: number;
}

export type RestartPolicy = 'NO' | 'ON_FAILURE' | 'UNLESS_STOPPED' | 'ALWAYS';

export type HealthCheckType = 'NONE' | 'HTTP' | 'TCP' | 'COMMAND';

export interface HealthCheckConfig {
  interval_seconds: number;
  kind: HealthCheckType;
  retries: number;
  start_period_seconds: number;
  target?: string | null;
  timeout_seconds: number;
}

export type PackageManager = 'PNPM' | 'NPM' | 'YARN' | 'BUN' | 'DENO' | 'PIP' | 'POETRY' | 'UV' | 'PIPENV' | 'GO_MODULES' | 'CARGO' | 'MAVEN' | 'GRADLE' | 'COMPOSER' | 'BUNDLER' | 'NUGET' | 'NONE';

/** Which approved template family builds the image.  `TYPESCRIPT` is its own runtime rather than a flavour of `NODEJS` because it is the presence of a compile step, not a different interpreter, that changes how the image is built.  `POLYGLOT` is for a project that genuinely needs more than one toolchain — a Python service with a Node build, say. Its image carries several toolchains and is correspondingly large, which is why it is a deliberate choice and never a detection default. */
export type Runtime = 'NODEJS' | 'TYPESCRIPT' | 'BUN' | 'DENO' | 'PYTHON' | 'GO' | 'RUST' | 'JAVA' | 'PHP' | 'RUBY' | 'DOTNET' | 'STATIC' | 'POLYGLOT';

export interface RuntimeConfig {
  build_command?: string | null;
  entry_file?: string | null;
  health_check: HealthCheckConfig;
  install_command?: string | null;
  package_manager: PackageManager;
  publish_dir?: string | null;
  runtime: Runtime;
  runtime_version: string;
  start_command: string;
  template_id: string;
  working_dir: string;
}

/** What the creation wizard submits. The server generates the identifier, slug, directory and container name; none of them derive from `display_name`. */
export interface CreateProjectRequest {
  autostart: boolean;
  color?: string | null;
  description?: string;
  display_name: string;
  environment?: EnvVarInput[];
  icon?: string | null;
  network: NetworkConfigRequest;
  project_type: ProjectType;
  resources?: ResourceLimits;
  restart_policy: RestartPolicy;
  runtime_config: RuntimeConfig;
  source: ProjectSource;
}

/** Deleting requires echoing the display name. The confirmation is part of the contract, not a dialog the API would happily let a caller skip. */
export interface DeleteProjectRequest {
  confirm_name: string;
  remove_volumes?: boolean;
}

/** Identifier prefixed with `dep_`. */
export type DeploymentId = string;

export type DeploymentStatus = 'PENDING' | 'BUILDING' | 'STARTING' | 'SUCCEEDED' | 'FAILED' | 'CANCELLED' | 'INTERRUPTED';

export type DeploymentType = 'INITIAL' | 'REBUILD' | 'RESTORE' | 'CONFIG_CHANGE' | 'IMPORT';

export interface DeploymentSummary {
  deployment_type: DeploymentType;
  duration_ms?: number | null;
  error_code?: string | null;
  error_message?: string | null;
  finished_at?: string | null;
  id: DeploymentId;
  image_tag?: string | null;
  project_id: ProjectId;
  started_at: string;
  status: DeploymentStatus;
}

/** What the user asked for. Survives restarts; the reconciler converges observed state towards it. */
export type DesiredState = 'RUNNING' | 'STOPPED' | 'ARCHIVED';

/** Identifier prefixed with `env_`. */
export type EnvVarId = string;

/** A variable as the API returns it.  For a secret, `value` is always `None` and `is_set` reports whether one has been stored. There is no representation in which a secret's value travels to a client. */
export interface EnvVarSummary {
  id: EnvVarId;
  is_secret: boolean;
  is_set: boolean;
  key: string;
  restart_required: boolean;
  updated_at: string;
  value?: string | null;
}

/** `None` means the workload has no meaningful check — a Discord bot serves nothing. Inventing a check that always passes would be worse than saying so. */
export type HealthState = 'UNKNOWN' | 'STARTING' | 'HEALTHY' | 'UNHEALTHY' | 'NONE';

export interface HostMetrics {
  cpu_percent: number;
  /** `None` where the platform does not expose it, which is common on Windows. */
  cpu_temperature_c?: number | null;
  disk_read_bytes_per_sec: number;
  disk_total_bytes: number;
  disk_used_bytes: number;
  disk_write_bytes_per_sec: number;
  memory_total_bytes: number;
  memory_used_bytes: number;
  net_rx_bytes_per_sec: number;
  net_tx_bytes_per_sec: number;
  process_count: number;
  sampled_at: string;
  swap_total_bytes: number;
  swap_used_bytes: number;
  uptime_seconds: number;
}

export interface LogLine {
  message: string;
  /** `stdout` or `stderr`. Kept distinct so the UI can colour and filter them. */
  stream: string;
  timestamp: string;
}

export interface LoginRequest {
  client_label?: string | null;
  email: string;
  password: string;
}

/** Identifier prefixed with `ses_`. */
export type SessionId = string;

/** Version one has a single role. The type exists so that adding roles later is a migration of one column rather than a redesign of every check. */
export type UserRole = 'ADMIN';

export interface UserSummary {
  display_name: string;
  email: string;
  id: UserId;
  last_login_at?: string | null;
  role: UserRole;
}

/** The token is returned exactly once, to the desktop client's Rust core, which puts it in the OS keychain. It never reaches the webview. */
export interface LoginResponse {
  expires_at: string;
  session_id: SessionId;
  token: string;
  user: UserSummary;
}

/** Identifier prefixed with `prt_`. */
export type PortId = string;

export interface PortMapping {
  /** `127.0.0.1` unless the user explicitly asked for LAN exposure. */
  bind_address: string;
  /** `None` until the agent allocates one. */
  host_port?: number | null;
  id: PortId;
  is_primary: boolean;
  port: number;
  protocol: string;
}

export interface NetworkConfig {
  mode: NetworkMode;
  ports: PortMapping[];
}

/** Identifier prefixed with `ntf_`. */
export type NotificationId = string;

export type NotificationLevel = 'INFO' | 'SUCCESS' | 'WARNING' | 'ERROR';

export interface NotificationSummary {
  body: string;
  created_at: string;
  id: NotificationId;
  level: NotificationLevel;
  project_id?: ProjectId | null;
  read_at?: string | null;
  title: string;
}

/** Identifier prefixed with `op_`. */
export type OperationId = string;

/** Returned by any endpoint that starts background work. */
export interface OperationHandle {
  accepted_at: string;
  kind: string;
  operation_id: OperationId;
  project_id: ProjectId;
}

/** Query parameters for a paginated endpoint. */
export interface PageRequest {
  /** The `next_cursor` from the previous page. */
  cursor?: string | null;
  /** Defaults to 50, clamped to `MAX_LIMIT`. */
  limit?: number | null;
}

/** What this platform can actually do. The UI hides what is unavailable rather than showing a control that silently does nothing — see `docs/platform-support.md` §5. */
export interface PlatformCapabilities {
  cpu_temperature: boolean;
  firewall_management: boolean;
  linux_capability_dropping: boolean;
  per_container_disk_io: boolean;
  read_only_root_filesystem: boolean;
  /** Which secure-storage backend is really in use. Reported so the UI can tell the truth when it has fallen back to an encrypted key file. */
  secure_storage_backend: string;
  storage_quota_enforcement: boolean;
}

/** Observed state. Distinct from [`DesiredState`]: the reconciler exists precisely because these two disagree after a crash or a reboot.  `Crashed` and `Failed` are both bad endings and deliberately not the same word. `Failed` is a project that never got off the ground — the runtime is not installed, the port was taken, the entry file is not there — and the fix is configuration. `Crashed` is a project that started cleanly, ran, and then exited on its own, where the fix is in the user's code. Collapsing the two forces the interface to guess which happened, and it can only guess wrong half the time. */
export type ProjectStatus = 'CREATING' | 'STOPPED' | 'STARTING' | 'RUNNING' | 'STOPPING' | 'RESTARTING' | 'BUILDING' | 'CRASHED' | 'FAILED' | 'UNHEALTHY' | 'ARCHIVED' | 'DELETING';

/** Row shape for the project list. Deliberately smaller than [`ProjectDetail`] so listing many projects stays cheap. */
export interface ProjectDetail {
  archived_at?: string | null;
  autostart: boolean;
  color?: string | null;
  container_id?: string | null;
  container_name: string;
  created_at: string;
  description: string;
  desired_state: DesiredState;
  display_name: string;
  /** Whether a token is stored for this project's remote. Deliberately a boolean: there is no route that returns the token itself. */
  has_credential?: boolean;
  health: HealthState;
  icon?: string | null;
  id: ProjectId;
  image_tag?: string | null;
  last_exit_code?: number | null;
  last_failure_at?: string | null;
  last_failure_reason?: string | null;
  network: NetworkConfig;
  project_type: ProjectType;
  resources: ResourceLimits;
  restart_count: number;
  restart_policy: RestartPolicy;
  runtime: Runtime;
  runtime_config: RuntimeConfig;
  slug: string;
  /** The commit that was actually checked out, which is the only honest answer to "what is running" when the ref was a moving branch. */
  source_commit?: string | null;
  source_ref?: string | null;
  source_type: SourceType;
  /** Where a `GIT_CLONE` or `REMOTE_ARCHIVE` project came from. The URL is safe to return because a token never travels inside it — see [`ProjectSource::credential`]. */
  source_url?: string | null;
  started_at?: string | null;
  status: ProjectStatus;
  updated_at: string;
}

export interface ProjectMetrics {
  cpu_percent: number;
  disk_read_bytes?: number | null;
  disk_write_bytes?: number | null;
  memory_bytes: number;
  memory_limit_bytes: number;
  net_rx_bytes: number;
  net_tx_bytes: number;
  project_id: ProjectId;
  sampled_at: string;
}

/** Row shape for the project list. Deliberately smaller than [`ProjectDetail`] so listing many projects stays cheap. */
export interface ProjectSummary {
  color?: string | null;
  created_at: string;
  desired_state: DesiredState;
  display_name: string;
  health: HealthState;
  icon?: string | null;
  id: ProjectId;
  project_type: ProjectType;
  restart_count: number;
  runtime: Runtime;
  slug: string;
  started_at?: string | null;
  status: ProjectStatus;
}

/** Metadata attached to every successful response. */
export interface ResponseMeta {
  /** Echoed from the request, or generated when the client omitted it. */
  request_id: string;
  /** Agent's clock, RFC 3339 UTC. Lets the client detect a skewed local clock rather than silently rendering nonsense timestamps. */
  server_time: string;
}

/** Answer to "what am I connected to". Fetched immediately after authenticating. */
export interface ServerInfo {
  agent_uptime_seconds: number;
  agent_version: string;
  arch: string;
  bind_address: string;
  capabilities: PlatformCapabilities;
  host_uptime_seconds: number;
  hostname: string;
  /** Changes on every agent restart. A client seeing a new value knows its cached stream state is stale. */
  instance_id: string;
  lan_enabled: boolean;
  os: string;
  os_version: string;
  schema_version: number;
}

/** Recovery codes are in the response because this is the only moment they exist in plaintext. No route returns them again. */
export interface SetupAdministratorResponse {
  recovery_codes: string[];
  user: UserSummary;
}

export interface SetupStatus {
  administrator_exists: boolean;
  agent_version: string;
  schema_version: number;
}

export interface UpdateProjectRequest {
  autostart?: boolean | null;
  color?: string | null;
  description?: string | null;
  display_name?: string | null;
  icon?: string | null;
  resources?: ResourceLimits | null;
  restart_policy?: RestartPolicy | null;
}
