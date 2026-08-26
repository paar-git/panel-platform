//! Application core: configuration, logging, lifecycle, shared state.
//!
//! This crate owns everything the application does that is not the user
//! interface. It deliberately has no dependency on Tauri, so all of it can be
//! tested with `cargo test` and none of it needs a window.
//!
//! The desktop app in `apps/desktop` is a thin shell: it starts a [`Runtime`],
//! hands the resulting [`AppState`] to Tauri, and exposes commands that call
//! into the domain crates. There is no server, no port and no login, because
//! there is nothing to authenticate to — the process runs as the user, on the
//! user's own machine, and can already do anything the user can.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod bots;
pub mod config;
pub mod env;
pub mod health;
pub mod images;
pub mod integration;
pub mod keys;
pub mod lifecycle;
pub mod logging;
pub mod logs;
pub mod ports;
pub mod power_flow;
pub mod provisioning;
pub mod reconcile;
pub mod runner;
pub mod runtime;
pub mod runtime_plan;
pub mod shutdown;
pub mod startup;
pub mod state;
pub mod toolchain_flow;

pub use config::{AppConfig, ConfigError, LogLevel, Mode};
pub use health::{Health, HealthReport};
pub use images::{dockerfile_for, starter_files, ImageSpec};
pub use integration::IntegrationError;
pub use logging::{LoggingGuard, RequestContext};
pub use provisioning::{materialise_source, ProvisionError, SourceOutcome, SourceSpec};
pub use runner::{Observed, ProjectRunner, StartContext};
pub use runtime::{
    autostart_workloads, resolve_paths, run_optional_startup, Runtime, RuntimeError, APP_VERSION,
};
pub use runtime_plan::{plan_detected, plan_named, PlanError, RuntimePlan};
pub use shutdown::{wait_for_signal, Shutdown};
pub use startup::{StartupDiagnostics, StartupStage};
pub use state::{AppState, Identity};
pub use toolchain_flow::{assess, host_from_snapshot, MachineResolver, Readiness};
