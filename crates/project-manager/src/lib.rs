//! Project lifecycle: detection, templates, naming and port allocation.
//!
//! Phase 4 delivers everything that can be verified without a Docker daemon —
//! which is most of it, because the security-critical parts are about what is
//! *constructed*, not what Docker then does with it.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod detection;
pub mod env_vars;
pub mod names;
pub mod ports;

pub use detection::{detect, Detection, PackageManager, Runtime};
pub use env_vars::{EnvVar, EnvVarError, EnvVarView, ParsedDotenv};
pub use names::{sanitise_display_name, Slug};
pub use ports::{PortError, PortPool};
