// ---------------------------------------------------------------------------
// GENERATED FILE — DO NOT EDIT.
//
// Produced from crates/api-types by `cargo run -p project-host-api-types
// --bin emit-contracts`. Edit the Rust types and regenerate; CI fails if this
// file differs from what the generator produces.
// ---------------------------------------------------------------------------

import { z } from 'zod';

export const errorCodeSchema = z.union([z.literal('VALIDATION_FAILED'), z.literal('UNAUTHENTICATED'), z.literal('SESSION_EXPIRED'), z.literal('FORBIDDEN'), z.literal('NOT_FOUND'), z.literal('CONFLICT'), z.literal('PROJECT_LOCKED'), z.literal('OPERATION_IN_PROGRESS'), z.literal('PRECONDITION_FAILED'), z.literal('PAYLOAD_TOO_LARGE'), z.literal('RATE_LIMITED'), z.literal('PORT_UNAVAILABLE'), z.literal('RESOURCE_LIMIT_EXCEEDED'), z.literal('ARCHIVE_REJECTED'), z.literal('PATH_REJECTED'), z.literal('INTEGRITY_CHECK_FAILED'), z.literal('SETUP_REQUIRED'), z.literal('AGENT_STARTING'), z.literal('INTERNAL')]);

export const fieldErrorSchema = z.object({
  field: z.string(),
  message: z.string(),
});

export const apiErrorSchema = z.object({
  code: errorCodeSchema,
  details: z.record(z.string(), z.string()).optional(),
  fields: z.array(fieldErrorSchema).optional(),
  message: z.string(),
  request_id: z.string(),
});

export const auditIdSchema = z.string();

export const auditResultSchema = z.enum(['SUCCESS', 'FAILURE', 'DENIED']);

export const userIdSchema = z.string();

export const auditEntrySchema = z.object({
  action: z.string(),
  client_label: z.string().nullable().optional(),
  error_code: z.string().nullable().optional(),
  id: auditIdSchema,
  occurred_at: z.string(),
  request_id: z.string().nullable().optional(),
  result: auditResultSchema,
  source_addr: z.string().nullable().optional(),
  target_label: z.string().nullable().optional(),
  target_type: z.string().nullable().optional(),
  user_id: userIdSchema.nullable().optional(),
});

export const availabilitySchema = z.enum(['UNKNOWN', 'AVAILABLE', 'UNAVAILABLE']);

export const backupIdSchema = z.string();

export const backupStatusSchema = z.enum(['PENDING', 'CREATING', 'COMPLETED', 'FAILED', 'CANCELLED', 'CORRUPT']);

export const projectIdSchema = z.string();

export const backupSummarySchema = z.object({
  checksum_sha256: z.string().nullable().optional(),
  completed_at: z.string().nullable().optional(),
  created_at: z.string(),
  id: backupIdSchema,
  includes_config: z.boolean(),
  includes_files: z.boolean(),
  includes_volumes: z.boolean(),
  note: z.string().nullable().optional(),
  project_id: projectIdSchema,
  size_bytes: z.number().int().nullable().optional(),
  status: backupStatusSchema,
  verified_at: z.string().nullable().optional(),
});

export const connectivitySchema = z.object({
  agent: availabilitySchema,
  checked_at: z.string(),
  internet: availabilitySchema,
  lan: availabilitySchema,
});

export const containerEventTypeSchema = z.enum(['CREATED', 'STARTED', 'STOPPED', 'RESTARTED', 'DIED', 'OOM_KILLED', 'HEALTH_PASS', 'HEALTH_FAIL', 'DESTROYED']);

export const containerEventSummarySchema = z.object({
  detail: z.string().nullable().optional(),
  event_type: containerEventTypeSchema,
  exit_code: z.number().int().nullable().optional(),
  occurred_at: z.string(),
  project_id: projectIdSchema,
});

export const envVarInputSchema = z.object({
  is_secret: z.boolean().optional(),
  key: z.string(),
  value: z.string(),
});

export const networkModeSchema = z.enum(['NONE', 'INTERNAL', 'LAN', 'INTERNET']);

export const portRequestSchema = z.object({
  expose_to_lan: z.boolean().optional(),
  host_port: z.number().int().nullable().optional(),
  is_primary: z.boolean().optional(),
  port: z.number().int(),
});

export const networkConfigRequestSchema = z.object({
  mode: networkModeSchema,
  ports: z.array(portRequestSchema).optional(),
});

export const sourceCredentialSchema = z.string();

export const sourceTypeSchema = z.enum(['EMPTY', 'ZIP_UPLOAD', 'LOCAL_FOLDER', 'DUPLICATE', 'IMPORT_ARCHIVE', 'GIT_CLONE', 'REMOTE_ARCHIVE']);

export const projectSourceSchema = z.object({
  credential: sourceCredentialSchema.nullable().optional(),
  git_ref: z.string().nullable().optional(),
  kind: sourceTypeSchema,
  local_path: z.string().nullable().optional(),
  repo_url: z.string().nullable().optional(),
  source_project_id: projectIdSchema.nullable().optional(),
  subdirectory: z.string().nullable().optional(),
  upload_id: z.string().nullable().optional(),
});

export const projectTypeSchema = z.enum(['DISCORD_BOT', 'NODE_APP', 'PYTHON_APP', 'WEBSITE', 'STATIC_SITE', 'REST_API', 'WORKER', 'SERVICE']);

export const resourceLimitsSchema = z.object({
  cpu_limit_cores: z.number(),
  memory_limit_mb: z.number().int(),
  process_limit: z.number().int(),
  storage_limit_mb: z.number().int(),
});

export const restartPolicySchema = z.enum(['NO', 'ON_FAILURE', 'UNLESS_STOPPED', 'ALWAYS']);

export const healthCheckTypeSchema = z.enum(['NONE', 'HTTP', 'TCP', 'COMMAND']);

export const healthCheckConfigSchema = z.object({
  interval_seconds: z.number().int(),
  kind: healthCheckTypeSchema,
  retries: z.number().int(),
  start_period_seconds: z.number().int(),
  target: z.string().nullable().optional(),
  timeout_seconds: z.number().int(),
});

export const packageManagerSchema = z.enum(['PNPM', 'NPM', 'YARN', 'BUN', 'DENO', 'PIP', 'POETRY', 'UV', 'PIPENV', 'GO_MODULES', 'CARGO', 'MAVEN', 'GRADLE', 'COMPOSER', 'BUNDLER', 'NUGET', 'NONE']);

export const runtimeSchema = z.enum(['NODEJS', 'TYPESCRIPT', 'BUN', 'DENO', 'PYTHON', 'GO', 'RUST', 'JAVA', 'PHP', 'RUBY', 'DOTNET', 'STATIC', 'POLYGLOT']);

export const runtimeConfigSchema = z.object({
  build_command: z.string().nullable().optional(),
  entry_file: z.string().nullable().optional(),
  health_check: healthCheckConfigSchema,
  install_command: z.string().nullable().optional(),
  package_manager: packageManagerSchema,
  publish_dir: z.string().nullable().optional(),
  runtime: runtimeSchema,
  runtime_version: z.string(),
  start_command: z.string(),
  template_id: z.string(),
  working_dir: z.string(),
});

export const createProjectRequestSchema = z.object({
  autostart: z.boolean(),
  color: z.string().nullable().optional(),
  description: z.string().optional(),
  display_name: z.string(),
  environment: z.array(envVarInputSchema).optional(),
  icon: z.string().nullable().optional(),
  network: networkConfigRequestSchema,
  project_type: projectTypeSchema,
  resources: resourceLimitsSchema.optional(),
  restart_policy: restartPolicySchema,
  runtime_config: runtimeConfigSchema,
  source: projectSourceSchema,
});

export const deleteProjectRequestSchema = z.object({
  confirm_name: z.string(),
  remove_volumes: z.boolean().optional(),
});

export const deploymentIdSchema = z.string();

export const deploymentStatusSchema = z.enum(['PENDING', 'BUILDING', 'STARTING', 'SUCCEEDED', 'FAILED', 'CANCELLED', 'INTERRUPTED']);

export const deploymentTypeSchema = z.enum(['INITIAL', 'REBUILD', 'RESTORE', 'CONFIG_CHANGE', 'IMPORT']);

export const deploymentSummarySchema = z.object({
  deployment_type: deploymentTypeSchema,
  duration_ms: z.number().int().nullable().optional(),
  error_code: z.string().nullable().optional(),
  error_message: z.string().nullable().optional(),
  finished_at: z.string().nullable().optional(),
  id: deploymentIdSchema,
  image_tag: z.string().nullable().optional(),
  project_id: projectIdSchema,
  started_at: z.string(),
  status: deploymentStatusSchema,
});

export const desiredStateSchema = z.enum(['RUNNING', 'STOPPED', 'ARCHIVED']);

export const envVarIdSchema = z.string();

export const envVarSummarySchema = z.object({
  id: envVarIdSchema,
  is_secret: z.boolean(),
  is_set: z.boolean(),
  key: z.string(),
  restart_required: z.boolean(),
  updated_at: z.string(),
  value: z.string().nullable().optional(),
});

export const healthStateSchema = z.enum(['UNKNOWN', 'STARTING', 'HEALTHY', 'UNHEALTHY', 'NONE']);

export const hostMetricsSchema = z.object({
  cpu_percent: z.number(),
  cpu_temperature_c: z.number().nullable().optional(),
  disk_read_bytes_per_sec: z.number().int(),
  disk_total_bytes: z.number().int(),
  disk_used_bytes: z.number().int(),
  disk_write_bytes_per_sec: z.number().int(),
  memory_total_bytes: z.number().int(),
  memory_used_bytes: z.number().int(),
  net_rx_bytes_per_sec: z.number().int(),
  net_tx_bytes_per_sec: z.number().int(),
  process_count: z.number().int(),
  sampled_at: z.string(),
  swap_total_bytes: z.number().int(),
  swap_used_bytes: z.number().int(),
  uptime_seconds: z.number().int(),
});

export const logLineSchema = z.object({
  message: z.string(),
  stream: z.string(),
  timestamp: z.string(),
});

export const loginRequestSchema = z.object({
  client_label: z.string().nullable().optional(),
  email: z.string(),
  password: z.string(),
});

export const sessionIdSchema = z.string();

export const userRoleSchema = z.enum(['ADMIN']);

export const userSummarySchema = z.object({
  display_name: z.string(),
  email: z.string(),
  id: userIdSchema,
  last_login_at: z.string().nullable().optional(),
  role: userRoleSchema,
});

export const loginResponseSchema = z.object({
  expires_at: z.string(),
  session_id: sessionIdSchema,
  token: z.string(),
  user: userSummarySchema,
});

export const portIdSchema = z.string();

export const portMappingSchema = z.object({
  bind_address: z.string(),
  host_port: z.number().int().nullable().optional(),
  id: portIdSchema,
  is_primary: z.boolean(),
  port: z.number().int(),
  protocol: z.string(),
});

export const networkConfigSchema = z.object({
  mode: networkModeSchema,
  ports: z.array(portMappingSchema),
});

export const notificationIdSchema = z.string();

export const notificationLevelSchema = z.enum(['INFO', 'SUCCESS', 'WARNING', 'ERROR']);

export const notificationSummarySchema = z.object({
  body: z.string(),
  created_at: z.string(),
  id: notificationIdSchema,
  level: notificationLevelSchema,
  project_id: projectIdSchema.nullable().optional(),
  read_at: z.string().nullable().optional(),
  title: z.string(),
});

export const operationIdSchema = z.string();

export const operationHandleSchema = z.object({
  accepted_at: z.string(),
  kind: z.string(),
  operation_id: operationIdSchema,
  project_id: projectIdSchema,
});

export const pageRequestSchema = z.object({
  cursor: z.string().nullable().optional(),
  limit: z.number().int().nullable().optional(),
});

export const platformCapabilitiesSchema = z.object({
  cpu_temperature: z.boolean(),
  firewall_management: z.boolean(),
  linux_capability_dropping: z.boolean(),
  per_container_disk_io: z.boolean(),
  read_only_root_filesystem: z.boolean(),
  secure_storage_backend: z.string(),
  storage_quota_enforcement: z.boolean(),
});

export const projectStatusSchema = z.enum(['CREATING', 'STOPPED', 'STARTING', 'RUNNING', 'STOPPING', 'RESTARTING', 'BUILDING', 'CRASHED', 'FAILED', 'UNHEALTHY', 'ARCHIVED', 'DELETING']);

export const projectDetailSchema = z.object({
  archived_at: z.string().nullable().optional(),
  autostart: z.boolean(),
  color: z.string().nullable().optional(),
  container_id: z.string().nullable().optional(),
  container_name: z.string(),
  created_at: z.string(),
  description: z.string(),
  desired_state: desiredStateSchema,
  display_name: z.string(),
  has_credential: z.boolean().optional(),
  health: healthStateSchema,
  icon: z.string().nullable().optional(),
  id: projectIdSchema,
  image_tag: z.string().nullable().optional(),
  last_exit_code: z.number().int().nullable().optional(),
  last_failure_at: z.string().nullable().optional(),
  last_failure_reason: z.string().nullable().optional(),
  network: networkConfigSchema,
  project_type: projectTypeSchema,
  resources: resourceLimitsSchema,
  restart_count: z.number().int(),
  restart_policy: restartPolicySchema,
  runtime: runtimeSchema,
  runtime_config: runtimeConfigSchema,
  slug: z.string(),
  source_commit: z.string().nullable().optional(),
  source_ref: z.string().nullable().optional(),
  source_type: sourceTypeSchema,
  source_url: z.string().nullable().optional(),
  started_at: z.string().nullable().optional(),
  status: projectStatusSchema,
  updated_at: z.string(),
});

export const projectMetricsSchema = z.object({
  cpu_percent: z.number(),
  disk_read_bytes: z.number().int().nullable().optional(),
  disk_write_bytes: z.number().int().nullable().optional(),
  memory_bytes: z.number().int(),
  memory_limit_bytes: z.number().int(),
  net_rx_bytes: z.number().int(),
  net_tx_bytes: z.number().int(),
  project_id: projectIdSchema,
  sampled_at: z.string(),
});

export const projectSummarySchema = z.object({
  color: z.string().nullable().optional(),
  created_at: z.string(),
  desired_state: desiredStateSchema,
  display_name: z.string(),
  health: healthStateSchema,
  icon: z.string().nullable().optional(),
  id: projectIdSchema,
  project_type: projectTypeSchema,
  restart_count: z.number().int(),
  runtime: runtimeSchema,
  slug: z.string(),
  started_at: z.string().nullable().optional(),
  status: projectStatusSchema,
});

export const responseMetaSchema = z.object({
  request_id: z.string(),
  server_time: z.string(),
});

export const serverInfoSchema = z.object({
  agent_uptime_seconds: z.number().int(),
  agent_version: z.string(),
  arch: z.string(),
  bind_address: z.string(),
  capabilities: platformCapabilitiesSchema,
  host_uptime_seconds: z.number().int(),
  hostname: z.string(),
  instance_id: z.string(),
  lan_enabled: z.boolean(),
  os: z.string(),
  os_version: z.string(),
  schema_version: z.number().int(),
});

export const setupAdministratorResponseSchema = z.object({
  recovery_codes: z.array(z.string()),
  user: userSummarySchema,
});

export const setupStatusSchema = z.object({
  administrator_exists: z.boolean(),
  agent_version: z.string(),
  schema_version: z.number().int(),
});

export const updateProjectRequestSchema = z.object({
  autostart: z.boolean().nullable().optional(),
  color: z.string().nullable().optional(),
  description: z.string().nullable().optional(),
  display_name: z.string().nullable().optional(),
  icon: z.string().nullable().optional(),
  resources: resourceLimitsSchema.nullable().optional(),
  restart_policy: restartPolicySchema.nullable().optional(),
});

/** Cursor-paginated result set for an arbitrary item schema. */
export const pageOf = <T extends z.ZodTypeAny>(item: T) =>
  z.object({
    items: z.array(item),
    next_cursor: z.string().nullish(),
    has_more: z.boolean(),
  });

export const responseMetaEnvelopeSchema = z.object({
  request_id: z.string(),
  server_time: z.string(),
});

/** Success or failure, discriminated by `ok`. */
export const apiResponseOf = <T extends z.ZodTypeAny>(data: T) =>
  z.union([
    z.object({ ok: z.literal(true), data, meta: responseMetaEnvelopeSchema }),
    z.object({ ok: z.literal(false), error: apiErrorSchema }),
  ]);
