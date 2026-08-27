//! Connecting the power manager to the projects it is managing.
//!
//! The `power` crate deliberately knows nothing about databases, supervisors or
//! Discord — it is handed a list of running processes and returns a decision.
//! This module is the translation, and it is the only place that knows both
//! halves.
//!
//! # Where each field comes from
//!
//! A project's pid is only knowable from its supervisor handle, and its
//! priority and keep-awake setting are only knowable from its row. Neither
//! source can answer alone, so a tick reads the registry first — that being the
//! authority on what is *actually running* — and then looks up the rows for
//! exactly those projects. Doing it the other way round would mean reading
//! every row on the machine every two seconds to find the two that are up.

use project_host_power::manager::RunningProject;
use project_host_power::power::Priority;

use crate::state::AppState;

/// Every host project that is up, as the power manager needs to see it.
///
/// A project whose row cannot be read is skipped rather than defaulted. The
/// alternative — assuming `Normal` and no keep-awake — would silently release a
/// sleep hold the user asked for, and a laptop that suspends with somebody's
/// bot on it is a worse failure than a project left at whatever priority the
/// operating system chose.
pub async fn running_projects(app: &AppState) -> Vec<RunningProject> {
    let handles = app.host_projects().all().await;
    if handles.is_empty() {
        return Vec::new();
    }

    let mut projects = Vec::with_capacity(handles.len());
    for (id, processes) in handles {
        // The project's first running process. Priority and the sleep hold are
        // per project, not per process, and the operating system applies both
        // to a process tree — so the leader is the one to name.
        let Some(observed) = processes
            .iter()
            .map(|process| process.handle.observe())
            .find(|observed| observed.status == project_host_host_runner::HostStatus::Running)
        else {
            continue;
        };

        let Ok(Some(row)) =
            project_host_database::projects::find_project(app.database(), &id).await
        else {
            tracing::debug!(project = %id, "running project has no readable row; left alone");
            continue;
        };

        projects.push(RunningProject {
            id,
            pid: observed.pid,
            priority: Priority::parse(&row.priority),
            keep_awake: row.keep_awake,
        });
    }

    projects
}

/// Whether somebody is connected to this machine remotely.
///
/// Read from the session environment rather than from an API, because every
/// API that answers this properly is FFI the workspace forbids. On Windows a
/// Remote Desktop session names itself in `SESSIONNAME`; on Linux and macOS an
/// SSH session sets `SSH_CONNECTION`. Both are conventions rather than
/// guarantees, which is why getting this wrong has to be survivable — and it
/// is: the only thing it changes is a preference for responsiveness, and being
/// wrong in either direction costs a little power or a little latency.
pub fn remote_session() -> bool {
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return true;
    }

    std::env::var("SESSIONNAME")
        .map(|name| name.to_ascii_uppercase().starts_with("RDP-"))
        .unwrap_or(false)
}

/// One pass: read what is running, decide, act, record.
pub async fn tick(app: &AppState) {
    let projects = running_projects(app).await;
    let remote = remote_session();
    let now = std::time::Instant::now();
    let wall = wall_seconds();

    app.power()
        .write()
        .await
        .tick(&projects, remote, now, wall)
        .await;
}

/// Seconds since the epoch, for the journal only.
///
/// Wall-clock time is written down; every window, cooldown and average in the
/// power crate runs on the monotonic clock, so a machine whose clock jumps
/// cannot be made to think a cooldown elapsed.
fn wall_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;

    /// Whatever this machine is, the answer has to be a definite one rather
    /// than a panic on a machine with an unusual environment.
    #[test]
    fn a_remote_session_is_answered_rather_than_guessed_at() {
        let _ = remote_session();
    }

    #[test]
    fn the_wall_clock_is_after_the_epoch() {
        assert!(wall_seconds() > 1_700_000_000);
    }
}
