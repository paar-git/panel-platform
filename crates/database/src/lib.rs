//! SQLite storage for the agent.
//!
//! The agent is the only process that opens this file. The desktop client
//! reaches data through the API and never touches the database directly — that
//! is what allows the constraints here to be trusted as a last line of defence
//! rather than one of several writers' conventions.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod audit;
pub mod discord;
pub mod environment;
pub mod error;
pub mod locks;
pub mod pool;
pub mod projects;
pub mod queries;
pub mod recovery;
pub mod schema_parity;
pub mod source_credentials;
pub mod time;

pub use audit::{AuditEvent, AuditResult};
pub use discord::{BotCredentials, BotRecord, ChannelRecord, GrantRecord, GuildRecord};
pub use environment::{EnvVarRecord, StoredValue};
pub use error::{DatabaseError, Result};
pub use locks::{LockHeld, Operation};
pub use pool::{Database, SUPPORTED_SCHEMA_VERSION};
pub use projects::{NewProject, ProjectRecord, ProjectUpdate};
pub use recovery::{recover, RecoveryReport};
pub use source_credentials::SourceCredentialRecord;

/// The migration text, for the parity test and for diagnostics.
pub const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_initial.sql");

/// The Discord integration migration, for the same reasons.
pub const DISCORD_MIGRATION: &str = include_str!("../migrations/0002_discord.sql");

/// The remote-sources migration, which rebuilds `projects`. Note that after
/// this migration the authoritative definition of that table is here and not in
/// [`INITIAL_MIGRATION`] — which is why the enum-parity test reads the live
/// schema out of `sqlite_master` rather than trusting either file.
pub const REMOTE_SOURCES_MIGRATION: &str = include_str!("../migrations/0003_remote_sources.sql");

/// The runtimes migration, which rebuilds `project_runtimes`.
pub const RUNTIMES_MIGRATION: &str = include_str!("../migrations/0004_runtimes.sql");

/// The run-mode migration, which adds a column rather than rebuilding a table.
pub const RUN_MODE_MIGRATION: &str = include_str!("../migrations/0005_run_mode.sql");

/// The migration that replaces the one-row `discord_bot` table with many-row
/// `discord_bots`, carrying any existing credentials forward.
pub const DISCORD_BOTS_MIGRATION: &str = include_str!("../migrations/0006_discord_bots.sql");

/// The migration recording which projects a bot covers.
pub const DISCORD_BOT_PROJECTS_MIGRATION: &str =
    include_str!("../migrations/0007_discord_bot_projects.sql");

/// The migration that rebuilds `projects` for local execution: a CRASHED
/// status, HOST as the run mode every row now carries, and the two columns the
/// resource manager reads.
pub const LOCAL_RUNTIME_MIGRATION: &str = include_str!("../migrations/0008_local_runtime.sql");

/// The migration that gives a project more than one process, and takes the
/// last container columns out of the schema. After it, the authoritative
/// definition of `projects`, `project_runtimes` and `project_ports` is here.
pub const PROCESSES_MIGRATION: &str = include_str!("../migrations/0009_processes.sql");
