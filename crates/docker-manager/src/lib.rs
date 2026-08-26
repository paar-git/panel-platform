//! Talking to the Docker daemon.
//!
//! The agent is the only component that does this. The desktop client does not
//! depend on this crate, so the boundary in `docs/architecture.md` Â§5 is a
//! compile-time fact rather than a convention.
//!
//! Phase 3 covers detection and status only. Container lifecycle, image builds,
//! log streaming and stats arrive in Phases 4 and 6.
//!
//! **No function here fabricates a daemon response.** When Docker is absent,
//! callers get `available: false` and an install hint. The only stub lives in
//! [`testing`], compiled solely under `cfg(test)` or the `testing` feature, and
//! never reachable from a release binary.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod container_spec;
pub mod lifecycle;
pub mod status;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::sync::Arc;
use std::time::Duration;

use bollard::Docker;
use project_host_platform::docker::{DockerEndpoint, DockerProvider};

pub use container_spec::{ContainerSpec, SecurityViolation, SpecInputs};
pub use status::DockerStatus;

/// How long to wait before deciding the daemon is not there. Short, because
/// this runs on the startup path and during health checks â€” a hung probe would
/// present as a hung agent.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

/// bollard's default negotiated API version. Using its constant rather than a
/// literal means a bollard upgrade cannot leave us pinned to a version the
/// daemon has dropped.
const API_VERSION: &bollard::ClientVersion = bollard::API_DEFAULT_VERSION;

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("no Docker endpoint could be reached")]
    Unreachable,
    #[error("Docker returned an error: {0}")]
    Daemon(String),
}

/// A live connection, with the endpoint it was made through.
#[derive(Clone)]
pub struct DockerConnection {
    client: Docker,
    endpoint: DockerEndpoint,
}

impl std::fmt::Debug for DockerConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerConnection")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl DockerConnection {
    pub fn endpoint(&self) -> &DockerEndpoint {
        &self.endpoint
    }

    pub fn client(&self) -> &Docker {
        &self.client
    }
}

/// Probes the daemon. One real implementation; stubbed only under test.
#[async_trait::async_trait]
pub trait DockerProbe: Send + Sync + std::fmt::Debug {
    async fn probe(&self) -> DockerStatus;
}

/// The production probe. Tries each platform candidate in order.
#[derive(Debug, Clone)]
pub struct BollardProbe {
    provider: Arc<dyn DockerProvider>,
}

impl BollardProbe {
    pub fn new(provider: Arc<dyn DockerProvider>) -> Self {
        Self { provider }
    }

    /// Connect to the first candidate that answers a ping.
    ///
    /// A successful `connect_*` call proves nothing â€” bollard's client is lazy
    /// and constructing one never touches the network. The ping is what makes
    /// this an honest answer rather than an assumption.
    pub async fn connect(&self) -> Result<DockerConnection, DockerError> {
        for endpoint in self.provider.candidates() {
            let Some(client) = build_client(&endpoint) else {
                continue;
            };
            // A named pipe that exists while the daemon is still starting will
            // accept a client and then never answer. Bounding ping is what
            // lets us move on to the next candidate, or give up, instead of
            // waiting forever.
            if matches!(
                tokio::time::timeout(CONNECT_TIMEOUT, client.ping()).await,
                Ok(Ok(_))
            ) {
                return Ok(DockerConnection { client, endpoint });
            }
        }
        Err(DockerError::Unreachable)
    }

    async fn probe_inner(&self) -> DockerStatus {
        let connection = match self.connect().await {
            Ok(connection) => connection,
            Err(_) => return DockerStatus::unavailable(self.provider.install_hint()),
        };

        let endpoint_kind = connection.endpoint().kind().to_string();

        let version =
            match tokio::time::timeout(CONNECT_TIMEOUT, connection.client().version()).await {
                Ok(Ok(version)) => version,
                // Reachable but unhealthy is a different state from absent, and the
                // user needs to tell them apart to know what to do.
                Ok(Err(error)) => return DockerStatus::degraded(endpoint_kind, error.to_string()),
                Err(_) => {
                    return DockerStatus::degraded(
                        endpoint_kind,
                        "The Docker daemon did not answer in time.".to_string(),
                    )
                }
            };

        let containers_running =
            match tokio::time::timeout(CONNECT_TIMEOUT, connection.client().info()).await {
                Ok(Ok(info)) => info
                    .containers_running
                    .and_then(|count| u32::try_from(count).ok()),
                Ok(Err(_)) | Err(_) => None,
            };

        DockerStatus {
            available: true,
            version: version.version,
            api_version: version.api_version,
            endpoint_kind: Some(endpoint_kind),
            install_hint: None,
            error: None,
            containers_running,
        }
    }
}

fn build_client(endpoint: &DockerEndpoint) -> Option<Docker> {
    match endpoint {
        DockerEndpoint::NamedPipe(path) => {
            #[cfg(windows)]
            {
                Docker::connect_with_named_pipe(path, CONNECT_TIMEOUT.as_secs(), API_VERSION).ok()
            }
            #[cfg(not(windows))]
            {
                let _ = path;
                None
            }
        }
        DockerEndpoint::UnixSocket(path) => {
            #[cfg(unix)]
            {
                Docker::connect_with_unix(
                    &path.to_string_lossy(),
                    CONNECT_TIMEOUT.as_secs(),
                    API_VERSION,
                )
                .ok()
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                None
            }
        }
        DockerEndpoint::Tcp(address) => Docker::connect_with_http(
            &format!("http://{address}"),
            CONNECT_TIMEOUT.as_secs(),
            API_VERSION,
        )
        .ok(),
    }
}

#[async_trait::async_trait]
impl DockerProbe for BollardProbe {
    async fn probe(&self) -> DockerStatus {
        // The connect timeout on the client is not enough: a Windows named
        // pipe that exists while Docker Desktop is still booting accepts the
        // client and then never answers ping. Wrapping the whole probe is what
        // stops that from looking like a hung application.
        match tokio::time::timeout(CONNECT_TIMEOUT, self.probe_inner()).await {
            Ok(status) => status,
            Err(_) => DockerStatus::degraded(
                "timeout".to_string(),
                "The Docker daemon did not answer in time.".to_string(),
            ),
        }
    }
}

/// The probe used in production.
pub fn system_probe() -> BollardProbe {
    BollardProbe::new(Arc::new(
        project_host_platform::docker::SystemDockerProvider,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_host_platform::docker::DockerInstallHint;

    #[derive(Debug)]
    struct NoCandidates;

    impl DockerProvider for NoCandidates {
        fn candidates(&self) -> Vec<DockerEndpoint> {
            Vec::new()
        }
        fn install_hint(&self) -> DockerInstallHint {
            DockerInstallHint {
                summary: "not installed".to_string(),
                detail: "install it".to_string(),
                url: "https://example.invalid".to_string(),
            }
        }
    }

    #[tokio::test]
    async fn no_candidates_means_unavailable_not_a_pretend_daemon() {
        let probe = BollardProbe::new(Arc::new(NoCandidates));
        let status = probe.probe().await;
        assert!(!status.available);
        assert!(status.install_hint.is_some());
        assert_eq!(status.version, None);
        assert_eq!(status.containers_running, None);
    }

    #[tokio::test]
    async fn connecting_with_no_candidates_is_an_error() {
        let probe = BollardProbe::new(Arc::new(NoCandidates));
        assert!(matches!(
            probe.connect().await,
            Err(DockerError::Unreachable)
        ));
    }

    #[derive(Debug)]
    struct BlackHole(String);

    impl DockerProvider for BlackHole {
        fn candidates(&self) -> Vec<project_host_platform::docker::DockerEndpoint> {
            vec![project_host_platform::docker::DockerEndpoint::Tcp(
                self.0.clone(),
            )]
        }
        fn install_hint(&self) -> project_host_platform::docker::DockerInstallHint {
            project_host_platform::docker::DockerInstallHint {
                summary: "not installed".to_string(),
                detail: "install it".to_string(),
                url: "https://example.invalid".to_string(),
            }
        }
    }

    #[tokio::test]
    async fn a_daemon_that_accepts_and_never_replies_does_not_hang_the_probe() {
        // A TCP port that accepts connections and then sits silent is what a
        // Docker named pipe looks like while Docker Desktop is still starting
        // after a reboot. The connect timeout used to apply only to client
        // construction, so ping/version waited forever and the window never
        // opened.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr").to_string();
        let _server = std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                std::thread::sleep(std::time::Duration::from_secs(30));
                drop(stream);
            }
        });

        let probe = BollardProbe::new(Arc::new(BlackHole(address)));
        let result = tokio::time::timeout(std::time::Duration::from_secs(6), probe.probe()).await;
        assert!(
            result.is_ok(),
            "the Docker probe hung past its timeout on a mute endpoint"
        );
        let status = result.expect("timed out");
        assert!(!status.available, "a mute daemon is not a working daemon");
    }

    /// Runs against whatever this host actually has. It asserts internal
    /// consistency rather than availability, so it is meaningful with or
    /// without a daemon and never claims Docker works when it does not.
    #[tokio::test]
    async fn the_system_probe_reports_a_self_consistent_status() {
        let status = system_probe().probe().await;
        if status.available {
            assert!(status.endpoint_kind.is_some());
            assert!(status.install_hint.is_none());
        } else {
            assert!(status.version.is_none(), "an absent daemon has no version");
            assert!(
                status.install_hint.is_some() || status.error.is_some(),
                "an absent daemon must explain itself"
            );
        }
    }

    #[test]
    fn a_named_pipe_client_is_only_built_on_windows() {
        let endpoint = DockerEndpoint::NamedPipe("//./pipe/docker_engine".to_string());
        assert_eq!(build_client(&endpoint).is_some(), cfg!(windows));
    }
}
