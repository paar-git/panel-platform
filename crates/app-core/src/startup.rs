//! Timing of startup stages.
//!
//! A launch that is slow again should say *which* stage was slow, not just
//! that the window took a while. Each stage records its own duration and
//! whether it finished, timed out, or failed, so a log line or the diagnostics
//! command is enough to point at the cause.

use serde::Serialize;

/// One named step of startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupStage {
    pub name: String,
    pub duration_ms: u64,
    /// `ok`, `timeout`, or a short error summary.
    pub outcome: String,
}

/// Every stage that ran, in order, plus the total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct StartupDiagnostics {
    pub stages: Vec<StartupStage>,
    pub total_ms: u64,
}

impl StartupDiagnostics {
    pub fn record(
        &mut self,
        name: &str,
        duration: std::time::Duration,
        outcome: impl Into<String>,
    ) {
        let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let outcome = outcome.into();
        if duration_ms >= 500 {
            tracing::warn!(
                stage = name,
                duration_ms,
                outcome = outcome.as_str(),
                "startup stage was slow"
            );
        } else {
            tracing::info!(
                stage = name,
                duration_ms,
                outcome = outcome.as_str(),
                "startup stage"
            );
        }
        self.stages.push(StartupStage {
            name: name.to_string(),
            duration_ms,
            outcome,
        });
        self.total_ms = self.total_ms.saturating_add(duration_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_slow_stage_is_still_recorded_with_its_name() {
        let mut diagnostics = StartupDiagnostics::default();
        diagnostics.record("database", Duration::from_millis(12), "ok");
        diagnostics.record("docker", Duration::from_millis(4000), "timeout");
        assert_eq!(diagnostics.stages.len(), 2);
        assert_eq!(diagnostics.stages[0].name, "database");
        assert_eq!(diagnostics.stages[1].outcome, "timeout");
        assert_eq!(diagnostics.total_ms, 4012);
    }
}
