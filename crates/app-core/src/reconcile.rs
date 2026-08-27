//! Keeping the stored status equal to what is actually happening.
//!
//! Without this module the database is only ever written at the moment a
//! lifecycle call returns. A project started at nine o'clock and killed by its
//! own bug at ten past reads `RUNNING` until somebody presses a button — which
//! is exactly the failure the request names: *do not show Running just because
//! a start button was clicked.*
//!
//! Two jobs, and they are different enough to be separate functions:
//!
//! * [`at_startup`] runs once, before the window opens. The supervisor registry
//!   is empty by definition at that point — a handle owns a child of *this*
//!   process — so any row claiming to be up is describing a process from a
//!   previous run that is gone.
//! * [`sweep`] runs on a timer for as long as the application does, and carries
//!   what each live supervisor observed into the row.
//!
//! # Why no pid is ever adopted
//!
//! The request asks for reconciliation that never trusts an old pid, because
//! operating systems reuse them. This design cannot make that mistake: nothing
//! anywhere writes a pid to the database. A supervisor handle is the only thing
//! that knows a pid, it lives in memory, and it dies with the process that
//! spawned the child. A previous run's projects are therefore *known* to be
//! gone rather than checked for — which is a stronger guarantee than any
//! amount of careful pid validation, and it is the reason host projects are
//! stopped on the way out instead of being detached.

use project_host_api_types::{DesiredState, ProjectStatus};
use project_host_database::projects;

use crate::orchestrator::ProcessRegistry;
use crate::state::AppState;

/// What one startup reconciliation found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupReport {
    /// Projects whose row claimed they were running and whose process is gone.
    pub stale_running: Vec<String>,
    /// Of those, the ones the user still wants running — the list an "start
    /// these again?" prompt is built from.
    pub wanted_running: Vec<String>,
}

/// Correct every row that describes a process from a previous run.
///
/// Called before the window opens. `RUNNING` and `UNHEALTHY` are the words that
/// can survive an unclean stop; the transient ones (`STARTING`, `STOPPING`,
/// `RESTARTING`, `BUILDING`) are already reset by the database's own recovery,
/// which runs earlier and knows nothing about processes.
///
/// The row is written as `STOPPED` rather than `CRASHED`. It is not known that
/// the project crashed — the far likelier story is that the application was
/// closed, or the machine was restarted — and recording a crash that may not
/// have happened would put a red status and a failure reason against a project
/// that did nothing wrong. `desired_state` is deliberately left alone, so the
/// pair "wants RUNNING, is STOPPED" survives to drive the restart prompt.
pub async fn at_startup(app: &AppState) -> StartupReport {
    let mut report = StartupReport::default();

    let Ok(records) = projects::list_projects(app.database(), false, None, 1000).await else {
        tracing::warn!("could not read the project list to reconcile it");
        return report;
    };

    for record in records {
        if !claims_to_be_up(&record.status) {
            continue;
        }

        if let Err(error) =
            projects::record_stopped(app.database(), &record.id, Some(0), None).await
        {
            tracing::warn!(project = %record.id, %error, "could not correct a stale status");
            continue;
        }

        if record.desired_state == DesiredState::Running.as_str() {
            report.wanted_running.push(record.id.clone());
        }
        report.stale_running.push(record.id);
    }

    if !report.stale_running.is_empty() {
        tracing::info!(
            corrected = report.stale_running.len(),
            wanted = report.wanted_running.len(),
            "corrected projects whose stored status outlived their process"
        );
    }

    report
}

/// Statuses that assert a live process.
///
/// Only these two. The transient words are the database recovery's to clear —
/// it runs before this and does not need a registry to know that nothing is
/// mid-start after a restart.
fn claims_to_be_up(status: &str) -> bool {
    status == ProjectStatus::Running.as_str() || status == ProjectStatus::Unhealthy.as_str()
}

/// Carry what every live supervisor observed into its row.
///
/// Returns how many rows it changed, which is only used by the tests and by a
/// log line: a sweep that changes nothing must be silent, or the log becomes
/// one line every two seconds forever.
pub async fn sweep(app: &AppState) -> usize {
    let handles: std::collections::BTreeMap<String, _> =
        app.host_projects().all().await.into_iter().collect();

    let Ok(records) = projects::list_projects(app.database(), false, None, 1000).await else {
        return 0;
    };

    let mut changed = 0;
    for record in records {
        let Some(processes) = handles.get(&record.id).filter(|set| !set.is_empty()) else {
            // No supervisor. Either the project is stopped, which is what the
            // row already says, or the row is stale — and a row that went stale
            // *while this process was running* means the supervisor was
            // removed, which only stop and kill do, and both write the row
            // themselves. Nothing to do either way.
            continue;
        };

        // The first process that is not running decides, because that is the
        // one with something to report. All running means the project is.
        let observed = processes
            .iter()
            .map(|process| crate::orchestrator::observed_from(&process.handle))
            .find(|observed| observed.status != project_host_api_types::ProjectStatus::Running)
            .or_else(|| {
                processes
                    .first()
                    .map(|process| crate::orchestrator::observed_from(&process.handle))
            });
        let Some(observed) = observed else {
            continue;
        };
        if apply(app, &record, &observed).await {
            changed += 1;
        }
    }

    changed
}

/// Write one project's observed state, if it differs from what is stored.
///
/// `true` when something was written. The comparison matters: an unconditional
/// write every two seconds per project would rewrite `updated_at` on every row
/// forever, which turns "when did this last change" into "now", always.
async fn apply(
    app: &AppState,
    record: &projects::ProjectRecord,
    observed: &crate::orchestrator::Observed,
) -> bool {
    let db = app.database();
    let stored_status = record.status.as_str();
    let observed_status = observed.status.as_str();

    // A project the interface believes is mid-transition is not this task's to
    // correct: a start in flight legitimately reads STARTING while its
    // supervisor already reads Running, and overwriting that would race the
    // call that is about to write the real answer.
    if matches!(
        stored_status,
        "STARTING" | "STOPPING" | "RESTARTING" | "BUILDING" | "CREATING" | "DELETING"
    ) {
        return false;
    }

    match observed.status {
        ProjectStatus::Crashed => {
            if stored_status == observed_status {
                return false;
            }
            if let Err(error) = projects::record_crashed(
                db,
                &record.id,
                observed.exit_code,
                observed.failure_reason.as_deref(),
            )
            .await
            {
                tracing::warn!(project = %record.id, %error, "could not record a crash");
                return false;
            }
            tracing::info!(
                project = %record.id,
                exit_code = ?observed.exit_code,
                "a running project exited on its own"
            );
            true
        }

        ProjectStatus::Failed => {
            if stored_status == observed_status {
                return false;
            }
            if let Err(error) = projects::set_status(db, &record.id, ProjectStatus::Failed, None)
                .await
                .and(
                    projects::record_stopped(
                        db,
                        &record.id,
                        observed.exit_code,
                        observed.failure_reason.as_deref(),
                    )
                    .await,
                )
            {
                tracing::warn!(project = %record.id, %error, "could not record a failure");
                return false;
            }
            true
        }

        ProjectStatus::Stopped => {
            if stored_status == observed_status {
                return false;
            }
            if let Err(error) = projects::record_stopped(db, &record.id, Some(0), None).await {
                tracing::warn!(project = %record.id, %error, "could not record a stop");
                return false;
            }
            true
        }

        // Running, with a health verdict that may have changed even when the
        // status has not — which is the case an equality check on the status
        // alone would miss.
        _ => {
            let stored_health = record.health.as_str();
            let observed_health = observed.health.map(|health| health.as_str());
            if stored_status == observed_status
                && observed_health.is_none_or(|health| health == stored_health)
            {
                return false;
            }
            if let Err(error) =
                projects::set_status(db, &record.id, observed.status, observed.health).await
            {
                tracing::warn!(project = %record.id, %error, "could not record a status change");
                return false;
            }
            true
        }
    }
}

/// Start every project marked `autostart` whose user wants it running.
///
/// Both conditions, not either. `autostart` is the standing instruction and
/// `desired_state` is the last thing the user actually did: a project the user
/// stopped deliberately before quitting must stay stopped, or the stop button
/// only works until the next launch.
///
/// Failures are logged and never propagated. One project with a missing runtime
/// must not stop the other nine from starting, and must not stop the window
/// from opening.
pub async fn start_autostart_projects(app: &AppState) -> Vec<String> {
    let Ok(records) = projects::list_projects(app.database(), false, None, 1000).await else {
        return Vec::new();
    };

    let mut started = Vec::new();
    for record in records {
        if !record.autostart || record.desired_state != DesiredState::Running.as_str() {
            continue;
        }

        match crate::lifecycle::start(app, &record.id).await {
            Ok(_) => started.push(record.id),
            Err(error) => {
                tracing::warn!(
                    project = %record.id,
                    error = %error,
                    "a project set to start automatically could not be started"
                );
            }
        }
    }

    if !started.is_empty() {
        tracing::info!(count = started.len(), "started projects automatically");
    }
    started
}

/// How many host projects this process is supervising, whatever their state.
///
/// Used by the sweep's log line and by tests; a registry with entries and a
/// database with none is the state that would mean the two had drifted.
pub async fn supervised(registry: &ProcessRegistry) -> usize {
    registry.all().await.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_host_api_types::{HealthState, ProjectType};
    use project_host_database::projects::{NewProcess, NewProject, RuntimeSpec};

    async fn app() -> AppState {
        let database = project_host_database::Database::open_in_memory()
            .await
            .expect("in-memory database");

        AppState::new(
            crate::config::AppConfig::default(),
            database,
            project_host_compatibility::Assessment {
                tier: project_host_compatibility::PerformanceTier::Standard,
                defaults: project_host_compatibility::ResourceDefaults {
                    memory_limit_mb: 512,
                    cpu_limit_cores: 1.0,
                    process_limit: 128,
                },
            },
            crate::state::Identity {
                instance_id: "test".to_string(),
                app_version: "0.0.0-test".to_string(),
                schema_version: 8,
                started_at_wall: project_host_database::time::now(),
            },
            None,
        )
    }

    async fn a_project(app: &AppState, slug: &str) -> String {
        projects::create_project(
            app.database(),
            &NewProject {
                slug: slug.to_string(),
                display_name: slug.to_string(),
                description: String::new(),
                project_type: ProjectType::Service,
                icon: None,
                color: None,
                source_type: "EMPTY".to_string(),
                directory: format!("projects/{slug}"),
                source_url: None,
                source_ref: None,
                source_commit: None,
                autostart: false,
                restart_policy: "NO".to_string(),
                network_mode: "INTERNET".to_string(),
                memory_limit_mb: 512,
                cpu_limit_cores: 1.0,
                storage_limit_mb: 1024,
                process_limit: 128,
                runtime: RuntimeSpec {
                    runtime: "NODEJS".to_string(),
                    runtime_version: "latest".to_string(),
                    package_manager: "NPM".to_string(),
                    entry_file: None,
                    publish_dir: None,
                    template_id: "node".to_string(),
                },
                processes: vec![NewProcess::simple("main", 0, "node index.js")],
                ports: Vec::new(),
            },
        )
        .await
        .expect("create")
        .id
    }

    async fn status_of(app: &AppState, project_id: &str) -> String {
        projects::find_project(app.database(), project_id)
            .await
            .expect("query")
            .expect("row")
            .status
    }

    /// The requirement, in one test: the application crashed while a project
    /// was running, and the next launch must not open onto a row claiming it
    /// still is.
    #[tokio::test]
    async fn a_row_left_claiming_to_run_is_corrected_at_startup() {
        let app = app().await;
        let project = a_project(&app, "left-running").await;

        projects::set_desired_state(app.database(), &project, DesiredState::Running)
            .await
            .expect("desired");
        projects::set_status(app.database(), &project, ProjectStatus::Running, None)
            .await
            .expect("status");

        let report = at_startup(&app).await;

        assert_eq!(report.stale_running, vec![project.clone()]);
        assert_eq!(
            report.wanted_running,
            vec![project.clone()],
            "a project the user wanted running is what a restart prompt is built from"
        );
        assert_eq!(status_of(&app, &project).await, "STOPPED");
    }

    /// An unhealthy project is still a project claiming a live process.
    #[tokio::test]
    async fn an_unhealthy_row_is_corrected_too() {
        let app = app().await;
        let project = a_project(&app, "left-unhealthy").await;
        projects::set_status(
            app.database(),
            &project,
            ProjectStatus::Unhealthy,
            Some(HealthState::Unhealthy),
        )
        .await
        .expect("status");

        at_startup(&app).await;
        assert_eq!(status_of(&app, &project).await, "STOPPED");
    }

    /// Correcting a stale row must not be recorded as a crash. The likely
    /// story is that the user closed the application, and putting a failure
    /// reason against a project that did nothing wrong is worse than silence.
    #[tokio::test]
    async fn a_corrected_row_is_not_blamed_for_crashing() {
        let app = app().await;
        let project = a_project(&app, "closed-cleanly").await;
        projects::set_status(app.database(), &project, ProjectStatus::Running, None)
            .await
            .expect("status");

        at_startup(&app).await;

        let record = projects::find_project(app.database(), &project)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(record.status, "STOPPED");
        assert!(record.last_failure_reason.is_none());
    }

    /// A project that was already stopped is not touched, so `updated_at` does
    /// not move on every launch for every project that is not running.
    #[tokio::test]
    async fn a_stopped_project_is_left_alone() {
        let app = app().await;
        let project = a_project(&app, "already-stopped").await;
        projects::set_status(app.database(), &project, ProjectStatus::Stopped, None)
            .await
            .expect("status");

        let before = projects::find_project(app.database(), &project)
            .await
            .expect("query")
            .expect("row")
            .updated_at;

        let report = at_startup(&app).await;
        assert!(report.stale_running.is_empty());

        let after = projects::find_project(app.database(), &project)
            .await
            .expect("query")
            .expect("row")
            .updated_at;
        assert_eq!(before, after);
    }

    /// A sweep with no supervisors changes nothing. The registry being empty is
    /// the ordinary state of a machine with nothing running, not a discrepancy.
    #[tokio::test]
    async fn a_sweep_with_nothing_supervised_writes_nothing() {
        let app = app().await;
        let project = a_project(&app, "quiet").await;
        projects::set_status(app.database(), &project, ProjectStatus::Stopped, None)
            .await
            .expect("status");

        assert_eq!(sweep(&app).await, 0);
        assert_eq!(supervised(app.host_projects()).await, 0);
    }

    /// The case the sweep exists for: a supervised project dies on its own and
    /// the row follows within one tick, without anybody pressing anything.
    #[tokio::test]
    async fn a_project_that_dies_on_its_own_is_recorded_as_crashed() {
        let app = app().await;
        let project = a_project(&app, "dies-alone").await;

        let directory = tempfile::tempdir().expect("temp dir");
        #[cfg(windows)]
        let dying = project_host_host_runner::ProcessCommand {
            program: "cmd".to_string(),
            args: vec![
                "/C".to_string(),
                "ping -n 2 127.0.0.1 >NUL && exit 7".to_string(),
            ],
            cwd: directory.path().to_path_buf(),
            env: Default::default(),
        };
        #[cfg(unix)]
        let dying = project_host_host_runner::ProcessCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "sleep 1; exit 7".to_string()],
            cwd: directory.path().to_path_buf(),
            env: Default::default(),
        };

        let handle =
            project_host_host_runner::start(project_host_host_runner::SupervisorConfig::new(
                dying,
                directory.path().join("run.log"),
            ))
            .await
            .expect("start");

        app.host_projects()
            .insert_for_test(&project, handle.clone())
            .await;
        projects::set_status(app.database(), &project, ProjectStatus::Running, None)
            .await
            .expect("status");

        // Sweep until it notices, the way the timer would. Waiting for the
        // *status* rather than for a sweep that changed something: the first
        // sweep legitimately writes the health verdict, which is a change and
        // is not the one being waited for.
        let mut recorded = false;
        for _ in 0..80 {
            sweep(&app).await;
            if status_of(&app, &project).await == "CRASHED" {
                recorded = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(recorded, "the sweep never noticed the project had died");

        let record = projects::find_project(app.database(), &project)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(
            record.status, "CRASHED",
            "a project that ran and then died is crashed, not merely stopped"
        );
        assert_eq!(record.last_exit_code, Some(7));

        // …and a second sweep does not rewrite the same fact.
        assert_eq!(sweep(&app).await, 0, "the sweep rewrote an unchanged row");
    }

    /// A project mid-start is the lifecycle call's to describe. Overwriting
    /// STARTING from a sweep would race the call about to write the real one.
    #[tokio::test]
    async fn a_project_that_is_mid_transition_is_not_overwritten() {
        let app = app().await;
        let project = a_project(&app, "starting").await;
        projects::set_status(app.database(), &project, ProjectStatus::Starting, None)
            .await
            .expect("status");

        assert_eq!(sweep(&app).await, 0);
        assert_eq!(status_of(&app, &project).await, "STARTING");
    }

    /// Autostart needs both the standing instruction and the user's last
    /// action. A project the user stopped on purpose must stay stopped.
    #[tokio::test]
    async fn a_project_the_user_stopped_is_not_started_automatically() {
        let app = app().await;
        let project = a_project(&app, "stopped-on-purpose").await;

        projects::update_project(
            app.database(),
            &project,
            &projects::ProjectUpdate {
                autostart: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("autostart");
        projects::set_desired_state(app.database(), &project, DesiredState::Stopped)
            .await
            .expect("desired");

        assert!(
            start_autostart_projects(&app).await.is_empty(),
            "the stop button only works until the next launch"
        );
        assert_ne!(
            status_of(&app, &project).await,
            "RUNNING",
            "a project the user stopped was started behind their back"
        );
    }

    /// A project with autostart off is never started, whatever it wants.
    #[tokio::test]
    async fn a_project_without_autostart_is_left_alone() {
        let app = app().await;
        let project = a_project(&app, "manual-only").await;
        projects::set_desired_state(app.database(), &project, DesiredState::Running)
            .await
            .expect("desired");

        assert!(start_autostart_projects(&app).await.is_empty());
    }
}
