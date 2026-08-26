//! Structured health.
//!
//! Two audiences with different needs. A service watchdog asks "is this process
//! alive" and must get a cheap answer with no configuration in it. An operator
//! asks "what is wrong" and needs detail. They are separate endpoints, and this
//! module produces both.

use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    /// Everything the application itself needs is working.
    Ok,
    /// The application is running, but something it depends on is not. Docker being
    /// absent is the common case, and it is deliberately not `Fail`: projects
    /// cannot start, yet files, backups and settings all work.
    Degraded,
    /// The application cannot do its job.
    Fail,
}

/// The cheap answer, for a watchdog. Reveals nothing about configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Liveness {
    pub status: Health,
    pub uptime_seconds: u64,
}

/// One thing that was checked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub status: Health,
    pub detail: Option<String>,
}

/// The detailed answer, for an authenticated operator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub status: Health,
    pub app_version: String,
    pub instance_id: String,
    pub uptime_seconds: u64,
    pub checks: Vec<Check>,
}

pub fn liveness(state: &AppState) -> Liveness {
    Liveness {
        status: Health::Ok,
        uptime_seconds: state.uptime_seconds(),
    }
}

/// Run every check and combine them.
pub async fn report(state: &AppState) -> HealthReport {
    let mut checks = Vec::new();

    // Database: an actual query, not a cached flag. A pool that has lost its
    // file would otherwise report healthy right up until the first write.
    let database_check = match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(state.database().pool())
        .await
    {
        Ok(_) => Check {
            name: "database".to_string(),
            status: Health::Ok,
            detail: None,
        },
        Err(error) => Check {
            name: "database".to_string(),
            status: Health::Fail,
            detail: Some(error.to_string()),
        },
    };
    let database_failed = database_check.status == Health::Fail;
    checks.push(database_check);

    let docker = state.docker_status().await;
    checks.push(Check {
        name: "docker".to_string(),
        status: if docker.available || docker.is_unchecked() {
            Health::Ok
        } else {
            Health::Degraded
        },
        detail: if docker.available {
            docker.version.clone()
        } else {
            docker.install_hint.clone().or_else(|| docker.error.clone())
        },
    });

    let status = if database_failed {
        Health::Fail
    } else if checks.iter().any(|check| check.status == Health::Degraded) {
        Health::Degraded
    } else {
        Health::Ok
    };

    HealthReport {
        status,
        app_version: state.inner().app_version.clone(),
        instance_id: state.inner().instance_id.clone(),
        uptime_seconds: state.uptime_seconds(),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_serialises_in_lower_case() {
        let json = serde_json::to_string(&Health::Degraded).expect("serialise");
        assert_eq!(json, "\"degraded\"");
    }

    #[test]
    fn liveness_carries_no_configuration() {
        // A watchdog endpoint that leaked the bind address or version would be
        // reconnaissance for anything that can reach it.
        let json = serde_json::to_string(&Liveness {
            status: Health::Ok,
            uptime_seconds: 42,
        })
        .expect("serialise");
        assert!(json.contains("uptime_seconds"));
        assert!(!json.contains("bind"), "{json}");
        assert!(!json.contains("version"), "{json}");
    }
}
