//! What the agent knows about the daemon right now.

use project_host_platform::docker::DockerInstallHint;
use serde::{Deserialize, Serialize};

/// The daemon's state as last observed.
///
/// Three distinct situations, never collapsed into a boolean:
/// available, absent (with a hint), and reachable-but-failing (with an error).
/// A user whose Docker is installed-but-broken needs different advice from one
/// who has not installed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerStatus {
    pub available: bool,
    pub version: Option<String>,
    pub api_version: Option<String>,
    /// `npipe`, `unix-socket` or `tcp`.
    pub endpoint_kind: Option<String>,
    /// Present only when absent: what the user should do.
    pub install_hint: Option<String>,
    /// Present only when reachable but failing.
    pub error: Option<String>,
    pub containers_running: Option<u32>,
}

impl DockerStatus {
    /// No endpoint answered.
    pub fn unavailable(hint: DockerInstallHint) -> Self {
        Self {
            available: false,
            version: None,
            api_version: None,
            endpoint_kind: None,
            install_hint: Some(format!("{} {} See {}", hint.summary, hint.detail, hint.url)),
            error: None,
            containers_running: None,
        }
    }

    /// Reached the daemon, but it would not answer properly.
    pub fn degraded(endpoint_kind: String, error: String) -> Self {
        Self {
            available: false,
            version: None,
            api_version: None,
            endpoint_kind: Some(endpoint_kind),
            install_hint: None,
            error: Some(error),
            containers_running: None,
        }
    }

    /// The probe has not finished yet. Distinct from "not installed": the
    /// window can open with this, and the status bar must not tell the user
    /// to install Docker before anyone has looked.
    pub fn unchecked() -> Self {
        Self {
            available: false,
            version: None,
            api_version: None,
            endpoint_kind: None,
            install_hint: None,
            error: None,
            containers_running: None,
        }
    }

    /// True when nobody has asked the daemon yet.
    pub fn is_unchecked(&self) -> bool {
        !self.available && self.install_hint.is_none() && self.error.is_none()
    }

    /// A short line for logs and the status bar.
    pub fn summary(&self) -> String {
        if self.is_unchecked() {
            return "Docker status not yet checked".to_string();
        }
        if self.available {
            match &self.version {
                Some(version) => format!("Docker {version} available"),
                None => "Docker available".to_string(),
            }
        } else if self.error.is_some() {
            "Docker reachable but not responding".to_string()
        } else {
            "Docker not available".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint() -> DockerInstallHint {
        DockerInstallHint {
            summary: "Docker Desktop was not found.".to_string(),
            detail: "Install it.".to_string(),
            url: "https://docs.docker.com/".to_string(),
        }
    }

    #[test]
    fn an_unavailable_status_carries_a_hint_and_no_version() {
        let status = DockerStatus::unavailable(hint());
        assert!(!status.available);
        assert!(status.install_hint.is_some());
        assert!(status.version.is_none());
        assert!(status.error.is_none());
        assert_eq!(status.summary(), "Docker not available");
    }

    #[test]
    fn the_hint_includes_the_url_a_user_needs() {
        let status = DockerStatus::unavailable(hint());
        let text = status.install_hint.unwrap_or_default();
        assert!(text.contains("https://docs.docker.com/"), "{text}");
    }

    #[test]
    fn a_degraded_status_is_distinct_from_an_absent_one() {
        // Installed-but-broken and not-installed need different advice.
        let status = DockerStatus::degraded("npipe".to_string(), "timeout".to_string());
        assert!(!status.available);
        assert!(
            status.install_hint.is_none(),
            "do not tell them to install what is installed"
        );
        assert_eq!(status.error.as_deref(), Some("timeout"));
        assert_eq!(status.summary(), "Docker reachable but not responding");
    }

    #[test]
    fn an_available_status_summarises_its_version() {
        let status = DockerStatus {
            available: true,
            version: Some("27.0.3".to_string()),
            api_version: Some("1.46".to_string()),
            endpoint_kind: Some("npipe".to_string()),
            install_hint: None,
            error: None,
            containers_running: Some(3),
        };
        assert_eq!(status.summary(), "Docker 27.0.3 available");
    }

    #[test]
    fn status_round_trips_through_json() {
        let status = DockerStatus::unavailable(hint());
        let json = serde_json::to_string(&status).expect("serialise");
        let back: DockerStatus = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, status);
    }

    #[test]
    fn an_unchecked_status_is_neither_available_nor_a_missing_install() {
        // Shown for the few seconds between the window opening and the
        // background probe finishing. Collapsing this into "not installed"
        // would flash a Docker install hint on every launch, including on
        // machines where Docker is about to answer.
        let status = DockerStatus::unchecked();
        assert!(!status.available);
        assert!(status.install_hint.is_none());
        assert!(status.error.is_none());
        assert!(status.is_unchecked());
        assert_eq!(status.summary(), "Docker status not yet checked");
    }
}
