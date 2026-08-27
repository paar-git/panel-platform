//! The aggregate used to emit the contract.
//!
//! Deriving `JsonSchema` on one struct that mentions every payload type makes
//! `schemars` collect them all into a single `$defs` block, which the generator
//! then walks. A type that is not reachable from here is not exported — so
//! adding a payload means adding a field, and forgetting to is caught by the
//! contract test rather than by a client that cannot parse a response.

use schemars::JsonSchema;

use crate::dto::*;
use crate::envelope::{PageRequest, ResponseMeta};
use crate::errors::{ApiError, FieldError};

/// Not a request or a response — an index of every type crossing the wire.
#[derive(Debug, JsonSchema)]
#[allow(dead_code)]
pub struct ApiContract {
    // envelope
    response_meta: ResponseMeta,
    page_request: PageRequest,
    api_error: ApiError,
    field_error: FieldError,

    // server
    server_info: ServerInfo,
    platform_capabilities: PlatformCapabilities,
    connectivity: Connectivity,

    // metrics
    host_metrics: HostMetrics,
    project_metrics: ProjectMetrics,

    // projects
    project_summary: ProjectSummary,
    project_detail: ProjectDetail,
    runtime_config: RuntimeConfig,
    health_check_config: HealthCheckConfig,
    resource_limits: ResourceLimits,
    network_config: NetworkConfig,
    port_mapping: PortMapping,
    create_project_request: CreateProjectRequest,
    update_project_request: UpdateProjectRequest,
    delete_project_request: DeleteProjectRequest,
    project_source: ProjectSource,
    network_config_request: NetworkConfigRequest,
    port_request: PortRequest,
    operation_handle: OperationHandle,

    // environment
    env_var_summary: EnvVarSummary,
    env_var_input: EnvVarInput,

    // history
    deployment_summary: DeploymentSummary,
    container_event_summary: ContainerEventSummary,
    audit_entry: AuditEntry,

    // backups
    backup_summary: BackupSummary,

    // logs
    log_line: LogLine,

    // auth
    login_request: LoginRequest,
    login_response: LoginResponse,
    user_summary: UserSummary,
    setup_status: SetupStatus,
    setup_administrator_response: SetupAdministratorResponse,
    notification_summary: NotificationSummary,
}

/// The JSON Schema for every wire type, as a `serde_json::Value`.
pub fn contract_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(ApiContract);
    schema.to_value()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn the_schema_defines_every_payload_type() {
        let schema = contract_schema();
        let defs = schema
            .get("$defs")
            .and_then(Value::as_object)
            .expect("contract schema must have $defs");

        for expected in [
            "ServerInfo",
            "ProjectSummary",
            "ProjectDetail",
            "CreateProjectRequest",
            "EnvVarSummary",
            "BackupSummary",
            "AuditEntry",
            "ApiError",
            "LoginResponse",
        ] {
            assert!(defs.contains_key(expected), "missing {expected} in $defs");
        }
    }

    #[test]
    fn the_contract_generates_without_hitting_an_unsupported_construct() {
        let schema = contract_schema();
        let (typescript, zod) = crate::codegen::generate(&schema)
            .unwrap_or_else(|error| panic!("contract codegen failed: {error}"));

        assert!(typescript.contains("export interface ProjectSummary {"));
        assert!(zod.contains("export const projectSummarySchema = z.object({"));
    }

    #[test]
    fn project_detail_is_emitted_flat() {
        // ProjectDetail flattens ProjectSummary. The wire format is one object,
        // so the generated interface must contain the summary's fields directly.
        let schema = contract_schema();
        let (typescript, _) = crate::codegen::generate(&schema)
            .unwrap_or_else(|error| panic!("contract codegen failed: {error}"));

        let start = typescript
            .find("export interface ProjectDetail {")
            .expect("ProjectDetail interface must exist");
        let body = &typescript[start..];
        let end = body.find("\n}").unwrap_or(body.len());
        let body = &body[..end];

        assert!(
            body.contains("display_name"),
            "flattened summary field missing:\n{body}"
        );
        assert!(
            body.contains("status"),
            "flattened summary field missing:\n{body}"
        );
        assert!(
            body.contains("container_name"),
            "own field missing:\n{body}"
        );
    }

    #[test]
    fn no_secret_bearing_field_reaches_the_contract() {
        // A blunt guard: if a type ever gains a field that looks like it carries
        // a decrypted secret, this fails and forces a deliberate decision.
        let schema = serde_json::to_string(&contract_schema()).unwrap_or_default();
        for forbidden in [
            "\"password_hash\"",
            "\"value_cipher\"",
            "\"encryption_key\"",
            // Storage columns for a source token. A project's remote credential
            // is write-only, so neither the ciphertext nor its nonce has any
            // business in a generated client.
            "\"ciphertext\"",
            "\"nonce\"",
        ] {
            assert!(
                !schema.contains(forbidden),
                "{forbidden} must never appear in the public contract"
            );
        }
    }
}
