//! Stable error codes and the wire shape of a failure.
//!
//! The client switches on [`ErrorCode`] and never parses the message. That
//! separation is what lets the message be rewritten for clarity — or
//! translated — without breaking behaviour that depends on the outcome.
//!
//! Technical detail (the underlying error, the failing path, the SQL
//! state) is written to the agent log keyed by request id, and never returned.
//! `docs/security.md` §4 explains why: telling a caller precisely which
//! validation rule caught them is free reconnaissance.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use crate::enums::ParseEnumError;

macro_rules! error_codes {
    ( $( $variant:ident => $text:literal, $status:literal, $doc:literal ; )+ ) => {
        /// Machine-readable outcome. Stable across releases.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum ErrorCode {
            $(
                #[doc = $doc]
                #[serde(rename = $text)]
                $variant,
            )+
        }

        impl ErrorCode {
            pub const ALL: &'static [ErrorCode] = &[ $( ErrorCode::$variant ),+ ];

            pub const fn as_str(&self) -> &'static str {
                match self { $( ErrorCode::$variant => $text, )+ }
            }

            /// The HTTP status this code is served with. Kept beside the code so
            /// two handlers cannot disagree about what `NOT_FOUND` means.
            pub const fn http_status(&self) -> u16 {
                match self { $( ErrorCode::$variant => $status, )+ }
            }

            /// Whether retrying the identical request could plausibly succeed.
            /// Drives the client's automatic retry; a `false` here means a retry
            /// only produces load.
            pub const fn is_retryable(&self) -> bool {
                matches!(
                    self,
                    ErrorCode::RateLimited
                        | ErrorCode::AgentStarting
                        | ErrorCode::OperationInProgress
                        | ErrorCode::ProjectLocked
                )
            }
        }

        impl fmt::Display for ErrorCode {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for ErrorCode {
            type Err = ParseEnumError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $text => Ok(ErrorCode::$variant), )+
                    other => Err(ParseEnumError {
                        type_name: "ErrorCode",
                        value: other.to_string(),
                    }),
                }
            }
        }
    };
}

error_codes! {
    ValidationFailed      => "VALIDATION_FAILED",       422, "Request body failed schema validation.";
    Unauthenticated       => "UNAUTHENTICATED",         401, "Missing or invalid credentials.";
    SessionExpired        => "SESSION_EXPIRED",         401, "Valid token, past its expiry.";
    Forbidden             => "FORBIDDEN",               403, "Authenticated but not permitted.";
    NotFound              => "NOT_FOUND",               404, "No such resource.";
    Conflict              => "CONFLICT",                409, "Violates a uniqueness or state constraint.";
    ProjectLocked         => "PROJECT_LOCKED",          409, "Another operation holds this project's lock.";
    OperationInProgress   => "OPERATION_IN_PROGRESS",   409, "An identical idempotent request is still running.";
    PreconditionFailed    => "PRECONDITION_FAILED",     412, "State precondition not met, e.g. restore on a running project.";
    PayloadTooLarge       => "PAYLOAD_TOO_LARGE",       413, "Upload exceeds the configured limit.";
    RateLimited           => "RATE_LIMITED",            429, "Too many requests.";
    PortUnavailable       => "PORT_UNAVAILABLE",        409, "Requested host port is taken or out of range.";
    ResourceLimitExceeded => "RESOURCE_LIMIT_EXCEEDED", 409, "A configured ceiling would be exceeded.";
    ArchiveRejected       => "ARCHIVE_REJECTED",        422, "The archive failed a security check.";
    PathRejected          => "PATH_REJECTED",           422, "The path escaped the project root.";
    IntegrityCheckFailed  => "INTEGRITY_CHECK_FAILED",  422, "Checksum or archive integrity mismatch.";
    SetupRequired         => "SETUP_REQUIRED",          428, "No administrator exists yet.";
    AgentStarting         => "AGENT_STARTING",          503, "Migrations or reconciliation still running.";
    Internal              => "INTERNAL",                500, "Unexpected failure. Details are in the agent log only.";
}

/// One field that failed validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FieldError {
    /// Dotted path to the offending field, e.g. `resources.memory_limit_mb`.
    pub field: String,
    /// Why it was rejected, phrased for a person.
    pub message: String,
}

/// The error half of the response envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApiError {
    pub code: ErrorCode,
    /// Written for a person. Never a stack trace, never a raw driver message.
    pub message: String,
    /// Structured, non-sensitive context, e.g. `{"held_by":"RESTORE"}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
    /// Present only for `VALIDATION_FAILED`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldError>,
    /// Correlates this failure with the agent log.
    pub request_id: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: BTreeMap::new(),
            fields: Vec::new(),
            request_id: request_id.into(),
        }
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    pub fn with_fields(mut self, fields: Vec<FieldError>) -> Self {
        self.fields = fields;
        self
    }

    pub fn http_status(&self) -> u16 {
        self.code.http_status()
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::from_str(code.as_str()), Ok(*code));
        }
    }

    #[test]
    fn every_status_is_a_client_or_server_error() {
        for code in ErrorCode::ALL {
            let status = code.http_status();
            assert!(
                (400..=599).contains(&status),
                "{code} maps to {status}, which is not an error status"
            );
        }
    }

    #[test]
    fn only_transient_conditions_are_retryable() {
        assert!(ErrorCode::RateLimited.is_retryable());
        // Retrying these can never succeed without the caller changing something.
        assert!(!ErrorCode::ValidationFailed.is_retryable());
        assert!(!ErrorCode::Forbidden.is_retryable());
        assert!(!ErrorCode::PathRejected.is_retryable());
        assert!(!ErrorCode::ArchiveRejected.is_retryable());
    }

    #[test]
    fn empty_details_and_fields_are_omitted_from_the_wire_form() {
        let err = ApiError::new(ErrorCode::NotFound, "No such project.", "req_1");
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("details"), "got {json}");
        assert!(!json.contains("fields"), "got {json}");
    }

    #[test]
    fn details_survive_a_round_trip() {
        let err = ApiError::new(ErrorCode::ProjectLocked, "Busy.", "req_2")
            .with_detail("held_by", "RESTORE");
        let json = serde_json::to_string(&err).unwrap();
        let back: ApiError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, err);
        assert_eq!(
            back.details.get("held_by").map(String::as_str),
            Some("RESTORE")
        );
    }
}
