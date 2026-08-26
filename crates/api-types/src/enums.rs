//! Enumerations shared by the agent, the database and the desktop client.
//!
//! These are stored in SQLite as `TEXT` with `CHECK` constraints. The
//! `ALL` slice on each type is what the migration parity test in
//! `crates/database` compares against the constraint, so a variant added here
//! without a matching migration fails the build rather than failing at runtime
//! when someone finally saves that value.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A value that did not match any variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not a valid {type_name}")]
pub struct ParseEnumError {
    pub type_name: &'static str,
    pub value: String,
}

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $( $variant:ident => $text:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum $name {
            $(
                #[serde(rename = $text)]
                $variant,
            )+
        }

        impl $name {
            /// Every variant, in declaration order. Used by the schema parity test.
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];

            /// The wire and storage representation.
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $( $name::$variant => $text, )+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ParseEnumError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $text => Ok($name::$variant), )+
                    other => Err(ParseEnumError {
                        type_name: stringify!($name),
                        value: other.to_string(),
                    }),
                }
            }
        }
    };
}

string_enum! {
    /// Version one has a single role. The type exists so that adding roles later
    /// is a migration of one column rather than a redesign of every check.
    UserRole { Admin => "ADMIN" }
}

string_enum! {
    /// What the user says the project is. Affects presentation and defaults,
    /// never isolation — every type gets identical container hardening.
    ProjectType {
        DiscordBot => "DISCORD_BOT",
        NodeApp => "NODE_APP",
        PythonApp => "PYTHON_APP",
        Website => "WEBSITE",
        StaticSite => "STATIC_SITE",
        RestApi => "REST_API",
        Worker => "WORKER",
        Service => "SERVICE",
    }
}

string_enum! {
    /// Which approved template family builds the image.
    ///
    /// `TYPESCRIPT` is its own runtime rather than a flavour of `NODEJS`
    /// because it is the presence of a compile step, not a different
    /// interpreter, that changes how the image is built.
    ///
    /// `POLYGLOT` is for a project that genuinely needs more than one
    /// toolchain — a Python service with a Node build, say. Its image carries
    /// several toolchains and is correspondingly large, which is why it is a
    /// deliberate choice and never a detection default.
    Runtime {
        NodeJs => "NODEJS",
        TypeScript => "TYPESCRIPT",
        Bun => "BUN",
        Deno => "DENO",
        Python => "PYTHON",
        Go => "GO",
        Rust => "RUST",
        Java => "JAVA",
        Php => "PHP",
        Ruby => "RUBY",
        DotNet => "DOTNET",
        Static => "STATIC",
        Polyglot => "POLYGLOT",
    }
}

string_enum! {
    PackageManager {
        Pnpm => "PNPM",
        Npm => "NPM",
        Yarn => "YARN",
        Bun => "BUN",
        Deno => "DENO",
        Pip => "PIP",
        Poetry => "POETRY",
        Uv => "UV",
        Pipenv => "PIPENV",
        GoModules => "GO_MODULES",
        Cargo => "CARGO",
        Maven => "MAVEN",
        Gradle => "GRADLE",
        Composer => "COMPOSER",
        Bundler => "BUNDLER",
        NuGet => "NUGET",
        None => "NONE",
    }
}

string_enum! {
    /// Observed state. Distinct from [`DesiredState`]: the reconciler exists
    /// precisely because these two disagree after a crash or a reboot.
    ///
    /// `Crashed` and `Failed` are both bad endings and deliberately not the
    /// same word. `Failed` is a project that never got off the ground — the
    /// runtime is not installed, the port was taken, the entry file is not
    /// there — and the fix is configuration. `Crashed` is a project that
    /// started cleanly, ran, and then exited on its own, where the fix is in
    /// the user's code. Collapsing the two forces the interface to guess which
    /// happened, and it can only guess wrong half the time.
    ProjectStatus {
        Creating => "CREATING",
        Stopped => "STOPPED",
        Starting => "STARTING",
        Running => "RUNNING",
        Stopping => "STOPPING",
        Restarting => "RESTARTING",
        Building => "BUILDING",
        Crashed => "CRASHED",
        Failed => "FAILED",
        Unhealthy => "UNHEALTHY",
        Archived => "ARCHIVED",
        Deleting => "DELETING",
    }
}

string_enum! {
    /// What the user asked for. Survives restarts; the reconciler converges
    /// observed state towards it.
    DesiredState {
        Running => "RUNNING",
        Stopped => "STOPPED",
        Archived => "ARCHIVED",
    }
}

string_enum! {
    RestartPolicy {
        No => "NO",
        OnFailure => "ON_FAILURE",
        UnlessStopped => "UNLESS_STOPPED",
        Always => "ALWAYS",
    }
}

string_enum! {
    /// Per-project network reach. `Internal` is the default: a project gets a
    /// dedicated network with no outbound route until someone asks for more.
    NetworkMode {
        None => "NONE",
        Internal => "INTERNAL",
        Lan => "LAN",
        Internet => "INTERNET",
    }
}

string_enum! {
    DeploymentType {
        Initial => "INITIAL",
        Rebuild => "REBUILD",
        Restore => "RESTORE",
        ConfigChange => "CONFIG_CHANGE",
        Import => "IMPORT",
    }
}

string_enum! {
    DeploymentStatus {
        Pending => "PENDING",
        Building => "BUILDING",
        Starting => "STARTING",
        Succeeded => "SUCCEEDED",
        Failed => "FAILED",
        Cancelled => "CANCELLED",
        Interrupted => "INTERRUPTED",
    }
}

string_enum! {
    ContainerEventType {
        Created => "CREATED",
        Started => "STARTED",
        Stopped => "STOPPED",
        Restarted => "RESTARTED",
        Died => "DIED",
        OomKilled => "OOM_KILLED",
        HealthPass => "HEALTH_PASS",
        HealthFail => "HEALTH_FAIL",
        Destroyed => "DESTROYED",
    }
}

string_enum! {
    BackupStatus {
        Pending => "PENDING",
        Creating => "CREATING",
        Completed => "COMPLETED",
        Failed => "FAILED",
        Cancelled => "CANCELLED",
        Corrupt => "CORRUPT",
    }
}

string_enum! {
    BackupOperationKind {
        Create => "CREATE",
        Restore => "RESTORE",
        Verify => "VERIFY",
        Export => "EXPORT",
        Import => "IMPORT",
        Delete => "DELETE",
    }
}

string_enum! {
    BackupOperationState {
        Pending => "PENDING",
        Running => "RUNNING",
        Completed => "COMPLETED",
        Failed => "FAILED",
        Cancelled => "CANCELLED",
        Interrupted => "INTERRUPTED",
    }
}

string_enum! {
    SourceType {
        Empty => "EMPTY",
        ZipUpload => "ZIP_UPLOAD",
        LocalFolder => "LOCAL_FOLDER",
        Duplicate => "DUPLICATE",
        ImportArchive => "IMPORT_ARCHIVE",
        GitClone => "GIT_CLONE",
        RemoteArchive => "REMOTE_ARCHIVE",
    }
}

string_enum! {
    AuditResult {
        Success => "SUCCESS",
        Failure => "FAILURE",
        Denied => "DENIED",
    }
}

string_enum! {
    /// `None` means the workload has no meaningful check — a Discord bot serves
    /// nothing. Inventing a check that always passes would be worse than saying so.
    HealthState {
        Unknown => "UNKNOWN",
        Starting => "STARTING",
        Healthy => "HEALTHY",
        Unhealthy => "UNHEALTHY",
        None => "NONE",
    }
}

string_enum! {
    HealthCheckType {
        None => "NONE",
        Http => "HTTP",
        Tcp => "TCP",
        Command => "COMMAND",
    }
}

string_enum! {
    ConnectionKind {
        Local => "LOCAL",
        Lan => "LAN",
        Tailscale => "TAILSCALE",
        Manual => "MANUAL",
    }
}

string_enum! {
    NotificationLevel {
        Info => "INFO",
        Success => "SUCCESS",
        Warning => "WARNING",
        Error => "ERROR",
    }
}

string_enum! {
    /// The five connectivity states from `docs/architecture.md` §8, as one
    /// reusable tri-state. `Unknown` is a real answer — probing may be disabled.
    Availability {
        Unknown => "UNKNOWN",
        Available => "AVAILABLE",
        Unavailable => "UNAVAILABLE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_its_string_form() {
        for status in ProjectStatus::ALL {
            assert_eq!(ProjectStatus::from_str(status.as_str()), Ok(*status));
        }
        for mode in NetworkMode::ALL {
            assert_eq!(NetworkMode::from_str(mode.as_str()), Ok(*mode));
        }
    }

    #[test]
    fn serde_uses_the_same_representation_as_as_str() {
        for status in ProjectStatus::ALL {
            let json = serde_json::to_string(status).unwrap();
            assert_eq!(json, format!("\"{}\"", status.as_str()));
            let back: ProjectStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, *status);
        }
    }

    /// How a project runs is stored per project, and `DOCKER` is the value
    /// every project that already exists has. Getting a wire value wrong here
    /// would silently change what an existing project does.
    ///
    /// There is deliberately no `Default` impl: the default lives on the
    /// database column, and a second one in Rust could disagree with it.

    #[test]
    fn a_variant_of_another_enum_does_not_deserialise() {
        // OOM_KILLED belongs to ContainerEventType. Enums that share a storage
        // representation must still refuse each other's values.
        assert!(serde_json::from_str::<ProjectStatus>("\"OOM_KILLED\"").is_err());
        assert!(ContainerEventType::from_str("RUNNING").is_err());
    }

    #[test]
    fn unknown_values_are_rejected_with_the_type_named() {
        let err = ProjectStatus::from_str("SLEEPING").unwrap_err();
        assert_eq!(err.type_name, "ProjectStatus");
        assert_eq!(err.value, "SLEEPING");
        assert_eq!(err.to_string(), "`SLEEPING` is not a valid ProjectStatus");
    }

    #[test]
    fn all_slices_are_complete_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for value in ProjectStatus::ALL {
            assert!(seen.insert(value.as_str()), "duplicate {value}");
        }
        assert_eq!(seen.len(), 12);
    }
}
