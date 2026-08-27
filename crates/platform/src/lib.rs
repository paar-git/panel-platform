//! Operating-system differences, isolated.
//!
//! The rule this crate exists to enforce: no `#[cfg(windows)]` or `#[cfg(unix)]`
//! appears anywhere else in the workspace. Business logic asks for a capability
//! or a path; it never asks which operating system it is running on.
//!
//! Phase 2 implements the path adapter and platform detection, because the
//! configuration system depends on them. Service management, Docker discovery,
//! secure storage, notifications, firewall rules and metrics are Phase 3.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod docker;
pub mod error;
pub mod info;
pub mod paths;
pub mod ports;
pub mod probe;
pub mod process;
pub mod secure_storage;
pub mod snapshot;

pub use docker::{DockerEndpoint, DockerInstallHint, DockerProvider, SystemDockerProvider};
pub use error::PlatformError;
pub use info::{Capabilities, PlatformInfo};
pub use paths::{platform_paths, PathProvider, StandardPaths};
pub use ports::{is_free, owner_of, PortOwner};
pub use probe::{FixedProbe, SystemProbe, SystemScanner};
pub use process::{
    as_group_leader, descendants, hide_console, hide_console_async, is_alive, kill_tree,
    terminate_tree,
};
pub use secure_storage::{
    open_secure_storage, FileStorage, SecureStorageProvider, StorageBackend, StorageError,
};
pub use snapshot::{
    Architecture, CpuInfo, GpuInfo, LinuxInfo, MemoryInfo, OsInfo, PackageManager, StorageKind,
    SystemSnapshot, VirtualizationInfo, VolumeInfo, WindowsInfo,
};
