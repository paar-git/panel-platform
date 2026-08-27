//! Starting and stopping a project.
//!
//! The sequence, and why it is this order:
//!
//! 1. **Admit it** — refuse before anything is written or spawned, so a
//!    refusal leaves the project exactly as it was.
//! 2. **Write the intent** — `desired_state`, then `STARTING`.
//! 3. **Hand it to the orchestrator**, which resolves the toolchain, allocates
//!    ports, runs install and build, and starts each process in order.
//! 4. **Record what was observed**, never what was intended.
//!
//! That last rule is the reason the database has both `status` and
//! `desired_state`, and it is enforced here rather than trusted to each
//! caller: every outcome goes through [`record`].
//!
//! **Nothing here outlives the application.** A project is a set of children
//! of this process, so quitting stops it. That was not true when a daemon ran
//! the containers, and it is why shutdown now stops everything rather than
//! exempting anything.

use std::path::Path;

use crate::orchestrator::{Observed, Orchestrator, StartContext};
use crate::state::AppState;
use project_host_api_types::{DesiredState, ProjectStatus};
use project_host_database::{projects, Database};
use project_host_host_runner::supervisor::{HostStatus, DEFAULT_GRACE};
use project_host_resources::{admit, Admission, RunningProject, Shortfall, Usage};

#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error("database error: {0}")]
    Database(#[from] project_host_database::DatabaseError),
    #[error("no project with id {0}")]
    NoSuchProject(String),
    #[error("could not prepare the project directory: {0}")]
    Scaffold(String),
    #[error("the build failed: {0}")]
    Build(String),

    /// Host mode only. The failure is reported before anything is spawned, so
    /// the message can name the runtime and the executables tried rather than
    /// being an operating-system error about a file that does not exist.
    #[error("{runtime} is not installed on this machine; looked for {}", looked_for.join(", "))]
    ToolchainMissing {
        runtime: String,
        looked_for: Vec<String>,
    },

    #[error("{0}")]
    Host(#[from] project_host_host_runner::HostError),

    #[error("{0}")]
    Command(#[from] project_host_host_runner::CommandError),

    /// The machine cannot take another project. Boxed because `Shortfall`
    /// carries the running set, and an enum whose largest variant is a vector
    /// of projects makes every `Result` in this module that size.
    #[error("{}", .0.message())]
    NotEnoughMemory(Box<Shortfall>),

    /// The port this project binds is already held. Reported before anything is
    /// spawned, so the message names the port and its holder rather than being
    /// an `EADDRINUSE` buried in the project's own output.
    ///
    /// Boxed for the same reason as above.
    #[error("{}", .0.message())]
    PortConflict(Box<crate::ports::Conflict>),

    /// A project with nothing to run. A configuration mistake rather than a
    /// failure, and refused by name: a start that succeeds having run nothing
    /// is the worst possible answer to it.
    #[error("project `{0}` has no processes to run")]
    NoProcesses(String),

    #[error("project `{project}` has no process named `{process}`")]
    NoSuchProcess { project: String, process: String },
}

/// The orchestrator for one project.
///
/// There is one substrate now, so this is a constructor rather than a choice.
/// It stays a function because the registry and the log root come from the
/// application state, and every caller would otherwise reach for both.
pub fn orchestrator_for(app: &AppState) -> Orchestrator {
    Orchestrator::new(app.host_projects().clone(), app.logs_root())
}

/// Write a project's starter files if they are absent.
///
/// Never overwrites: once a project exists, its files belong to the user. A
/// scaffold that clobbered an edited file on every start would be a data-loss
/// bug wearing a convenience hat. That is also what makes this safe to call
/// for a fetched repository — its own files are already there, so nothing is
/// written at all.
///
/// No `Dockerfile` is written any more. One used to be, for every project
/// including those that never went near a daemon.
pub fn scaffold(directory: &Path, runtime: &str) -> Result<(), LifecycleError> {
    std::fs::create_dir_all(directory)
        .map_err(|error| LifecycleError::Scaffold(error.to_string()))?;

    for (relative, contents) in crate::starter::starter_files(runtime) {
        let path = directory.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| LifecycleError::Scaffold(error.to_string()))?;
        }
        write_if_absent(&path, contents)?;
    }

    Ok(())
}

fn write_if_absent(path: &Path, contents: &str) -> Result<(), LifecycleError> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, contents).map_err(|error| LifecycleError::Scaffold(error.to_string()))
}

/// Start a project: install, build, then every process in order.
///
/// `force` skips the memory check. The estimate can be wrong and the user knows
/// things the governor does not, so overriding is offered on refusal — as a
/// button in a dialog that states the numbers, never as a setting that turns
/// the governor off.
pub async fn start_forcing(
    app: &AppState,
    project_id: &str,
    force: bool,
) -> Result<String, LifecycleError> {
    let db = app.database();
    let project = projects::find_project(db, project_id)
        .await?
        .ok_or_else(|| LifecycleError::NoSuchProject(project_id.to_string()))?;

    // Before anything is written or spawned. A refusal must leave the project
    // exactly as it was: writing STARTING and then refusing would leave a row
    // claiming a start that never began.
    //
    // The *port* check is not here but inside the host runner, because it can
    // only be asked once the project's own previous process has been stopped —
    // on a restart the port it wants is the port it is still holding.
    if !force {
        admit_project(app, &project).await?;
    }

    projects::set_desired_state(db, project_id, DesiredState::Running).await?;
    projects::set_status(db, project_id, ProjectStatus::Starting, None).await?;

    let directory = std::path::PathBuf::from(&project.directory);
    let outcome = orchestrator_for(app)
        .start(StartContext {
            db,
            project: &project,
            directory: &directory,
            master_key: app.master_key(),
        })
        .await;

    record(db, project_id, outcome).await
}

/// Start a project, refusing if this machine cannot take another.
pub async fn start(app: &AppState, project_id: &str) -> Result<String, LifecycleError> {
    start_forcing(app, project_id, false).await
}

/// Stop every host project this process is running, and record that it stopped.
///
/// Called on the way out. Docker projects are deliberately untouched: the daemon
/// outlives this application and keeps them running under their own restart
/// policy. A host project has no such daemon — it is a child of this process —
/// so quitting stops it.
///
/// **`desired_state` is left alone.** A project stopped because the application
/// is quitting still *wants* to be running, and the honest row for that is
/// `desired_state = RUNNING` beside `status = STOPPED`. That pair is also
/// exactly what a "start these again" prompt on the next launch needs to find,
/// which is why no separate list is written.
///
/// Answers with the ids that were running, for the quit dialog to report.
pub async fn stop_host_projects(app: &AppState) -> Vec<String> {
    let registry = app.host_projects();
    let mut stopped = Vec::new();

    for (project_id, processes) in registry.all().await {
        if !processes
            .iter()
            .any(|process| process.handle.observe().status == HostStatus::Running)
        {
            continue;
        }

        // Reverse order, for the reason the orchestrator stops in reverse: a
        // process was started against the ones before it.
        for process in processes.iter().rev() {
            if let Err(error) = process.handle.stop(DEFAULT_GRACE).await {
                // Report and carry on. One process that will not die must not
                // prevent the others from being stopped, and must not prevent
                // the database from being closed cleanly.
                tracing::warn!(%project_id, process = %process.name, %error,
                    "could not stop a process on shutdown");
            }
        }

        // Written from what happened, like every other status write. The
        // process is gone whether or not the stop reported cleanly.
        if let Err(error) =
            projects::record_stopped(app.database(), &project_id, Some(0), None).await
        {
            tracing::warn!(%project_id, %error, "could not record a host project as stopped");
        }
        stopped.push(project_id);
    }

    stopped
}

/// What every project that is currently up is costing this machine.
///
/// Host projects are measured: their supervisor holds a pid, and the reading
/// covers the whole process tree because `npm start`'s memory lives in the
/// `node` it spawned.
///
/// Docker projects are **not** measured. The daemon's stats endpoint is not
/// wired up, and this machine has no daemon to wire it against. Their declared
/// `memory_limit_mb` is counted instead, which is an upper bound rather than a
/// reading: a container cannot exceed the limit the daemon enforces. Counting
/// the bound errs towards refusing a start, which is the safe direction — the
/// opposite error would let the machine overcommit on a guess.
pub async fn running_projects(app: &AppState) -> Vec<RunningProject> {
    let Ok(records) = projects::list_projects(app.database(), false, None, 500).await else {
        return Vec::new();
    };

    let handles: std::collections::BTreeMap<String, _> =
        app.host_projects().all().await.into_iter().collect();

    let mut running = Vec::new();
    for record in records {
        if !is_running(&record.status) {
            continue;
        }

        // Every process of the project, summed. A project's cost is the cost
        // of everything it runs, and counting only the first would understate
        // a full-stack project by however much its web server uses.
        let usage = match handles.get(&record.id) {
            Some(processes) if !processes.is_empty() => processes
                .iter()
                .filter_map(|process| process.handle.pid())
                .filter_map(|pid| app.usage_source().process_tree(pid))
                .fold(Usage::default(), |total, usage| Usage {
                    memory_bytes: total.memory_bytes + usage.memory_bytes,
                    cpu_percent: match (total.cpu_percent, usage.cpu_percent) {
                        (None, other) => other,
                        (some, None) => some,
                        (Some(a), Some(b)) => Some(a + b),
                    },
                }),
            // Not running here: its declared limit is the only figure
            // available, and it is an upper bound rather than a reading.
            _ => Usage {
                memory_bytes: record.memory_limit_mb.max(0).unsigned_abs() * 1024 * 1024,
                cpu_percent: None,
            },
        };

        running.push(RunningProject {
            project_id: record.id,
            display_name: record.display_name,
            usage,
        });
    }

    running
}

/// What a project is expected to need, in bytes.
///
/// Its own `memory_limit_mb`. For a container that is exact, because the daemon
/// enforces it. For a host project it is an estimate and nothing enforces it —
/// which is precisely why the start is gated rather than the process capped.
fn wanted_bytes(project: &projects::ProjectRecord) -> u64 {
    project.memory_limit_mb.max(0).unsigned_abs() * 1024 * 1024
}

/// Decide whether this machine can take one more project.
///
/// Called before the runner is dispatched, so both substrates are gated without
/// either runner knowing the governor exists.
pub async fn admit_project(
    app: &AppState,
    project: &projects::ProjectRecord,
) -> Result<(), LifecycleError> {
    let running = running_projects(app).await;
    let decision = admit(
        wanted_bytes(project),
        app.machine_usage().await,
        app.reserve(),
        &running,
    );

    match decision {
        Admission::Allow => Ok(()),
        Admission::Refuse(shortfall) => Err(LifecycleError::NotEnoughMemory(Box::new(shortfall))),
    }
}

/// Whether a stored status word means the project is up or on its way there.
///
/// Used to refuse changes that only make sense on a stopped project. The
/// transitional words count as running: a project that is STARTING is not a
/// project it is safe to reconfigure.
pub fn is_running(status: &str) -> bool {
    matches!(
        status,
        "RUNNING" | "STARTING" | "RESTARTING" | "STOPPING" | "BUILDING" | "UNHEALTHY"
    )
}

/// How many host projects would stop if the application quit now.
///
/// What the quit dialog counts. Separate from [`stop_host_projects`] because it
/// is asked before the user has decided, and must change nothing.
pub async fn host_projects_running(app: &AppState) -> usize {
    app.host_projects().running().await
}

/// Write what a runner observed, and answer with the status word.
///
/// The single place a lifecycle outcome becomes a row. Both substrates go
/// through it, which is what makes "status is what was observed, never what was
/// intended" a property of the module rather than a habit of each caller.
async fn record(
    db: &Database,
    project_id: &str,
    outcome: Result<Observed, LifecycleError>,
) -> Result<String, LifecycleError> {
    match outcome {
        Ok(observed) => {
            projects::set_status(db, project_id, observed.status, observed.health).await?;
            Ok(observed.status.as_str().to_string())
        }
        Err(error) => {
            // The observed status is FAILED whatever the intent was. Recording
            // the intent as the status is how a panel ends up claiming a
            // project runs when it does not.
            projects::set_status(db, project_id, ProjectStatus::Failed, None).await?;
            Err(error)
        }
    }
}

/// Stop a project. Its data volume and files are untouched.
pub async fn stop(app: &AppState, project_id: &str) -> Result<(), LifecycleError> {
    let db = app.database();
    let project = projects::find_project(db, project_id)
        .await?
        .ok_or_else(|| LifecycleError::NoSuchProject(project_id.to_string()))?;

    projects::set_desired_state(db, project_id, DesiredState::Stopped).await?;
    projects::set_status(db, project_id, ProjectStatus::Stopping, None).await?;

    orchestrator_for(app).stop(&project).await?;

    // A user-requested stop is a clean one: exit 0, no failure reason.
    projects::record_stopped(db, project_id, Some(0), None).await?;
    Ok(())
}

/// Kill a project immediately. Its data volume and files are untouched.
pub async fn kill(app: &AppState, project_id: &str) -> Result<(), LifecycleError> {
    let db = app.database();
    let project = projects::find_project(db, project_id)
        .await?
        .ok_or_else(|| LifecycleError::NoSuchProject(project_id.to_string()))?;

    projects::set_desired_state(db, project_id, DesiredState::Stopped).await?;
    projects::set_status(db, project_id, ProjectStatus::Stopping, None).await?;

    orchestrator_for(app).kill(&project).await?;

    projects::record_stopped(db, project_id, None, None).await?;
    Ok(())
}

/// Restart one of a project's processes, leaving its siblings alone.
///
/// Separate from [`restart`], which restarts the whole project: the user
/// pointed at one process, and the others keep their pids.
pub async fn restart_process(
    app: &AppState,
    project_id: &str,
    process_name: &str,
) -> Result<(), LifecycleError> {
    let db = app.database();
    let project = projects::find_project(db, project_id)
        .await?
        .ok_or_else(|| LifecycleError::NoSuchProject(project_id.to_string()))?;

    let directory = std::path::PathBuf::from(&project.directory);
    orchestrator_for(app)
        .restart_process(
            StartContext {
                db,
                project: &project,
                directory: &directory,
                master_key: app.master_key(),
            },
            process_name,
        )
        .await
}

/// Restart a project in place, without rebuilding its image.
pub async fn restart(app: &AppState, project_id: &str) -> Result<String, LifecycleError> {
    let db = app.database();
    let project = projects::find_project(db, project_id)
        .await?
        .ok_or_else(|| LifecycleError::NoSuchProject(project_id.to_string()))?;

    // `RESTARTING` is now written before the runner is asked, rather than only
    // once a container was known to exist. A restart of something not running is
    // still a start, and now passes through `RESTARTING` on its way to
    // `STARTING` instead of going straight there. Both end in the same place;
    // the intermediate word is the honest one, because a restart is what was
    // asked for.
    projects::set_desired_state(db, project_id, DesiredState::Running).await?;
    projects::set_status(db, project_id, ProjectStatus::Restarting, None).await?;

    let directory = std::path::PathBuf::from(&project.directory);
    // A restart is a stop followed by a start. There is no image to keep, so
    // there is nothing a dedicated restart path could save.
    let orchestrator = orchestrator_for(app);
    let _ = orchestrator.stop(&project).await;
    let outcome = orchestrator
        .start(StartContext {
            db,
            project: &project,
            directory: &directory,
            master_key: app.master_key(),
        })
        .await;

    record(db, project_id, outcome).await
}

/// Tests that need a real `AppState`: the governor, and shutdown.
///
/// Separate from the module below because those are pure functions needing
/// nothing, and these need a database, a project row and a described machine.
#[cfg(test)]
mod tests_with_state {
    use super::*;
    use project_host_api_types::ProjectType;
    use project_host_database::projects::{NewProcess, NewProject};

    /// An `AppState` backed by an in-memory database.
    async fn test_state() -> AppState {
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
                schema_version: 5,
                started_at_wall: project_host_database::time::now(),
            },
            // No key: these tests never store a secret, and a test that
            // silently acquired one from the real keychain would be reaching
            // outside its sandbox.
            None,
        )
    }

    async fn a_host_project(app: &AppState, slug: &str) -> String {
        let project = projects::create_project(
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
                runtime: project_host_database::projects::RuntimeSpec {
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
        .expect("create");

        project.id
    }

    const GB: u64 = 1024 * 1024 * 1024;

    /// A machine with plenty to spare.
    fn roomy() -> project_host_resources::MachineUsage {
        project_host_resources::MachineUsage {
            total_memory_bytes: 32 * GB,
            available_memory_bytes: 24 * GB,
            cpu_percent: Some(10.0),
            logical_cores: 8,
        }
    }

    /// A machine with nothing to spare. Constructed rather than arranged: the
    /// alternative is exhausting the memory of whatever runs the tests.
    fn full() -> project_host_resources::MachineUsage {
        project_host_resources::MachineUsage {
            total_memory_bytes: 32 * GB,
            available_memory_bytes: GB / 2,
            cpu_percent: Some(99.0),
            logical_cores: 8,
        }
    }

    /// The requirement, in one test: on a full machine, starting is refused —
    /// and the refusal says what the numbers are rather than just failing.
    #[tokio::test]
    async fn a_full_machine_refuses_another_project() {
        let app = test_state().await;
        let project_id = a_host_project(&app, "crowded-port-9a11").await;
        app.set_usage_for_test(full()).await;

        let project = projects::find_project(app.database(), &project_id)
            .await
            .expect("query")
            .expect("row");

        match admit_project(&app, &project).await {
            Err(LifecycleError::NotEnoughMemory(shortfall)) => {
                let message = shortfall.message();
                assert!(message.contains("GB"), "no numbers in: {message}");
                assert_eq!(shortfall.headroom_bytes, 0, "a full machine spares nothing");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_roomy_machine_admits() {
        let app = test_state().await;
        let project_id = a_host_project(&app, "quiet-port-9a12").await;
        app.set_usage_for_test(roomy()).await;

        let project = projects::find_project(app.database(), &project_id)
            .await
            .expect("query")
            .expect("row");
        admit_project(&app, &project).await.expect("512 MB fits");
    }

    /// Before the sampler has run there is no basis for a refusal. Refusing
    /// everything until it warms up would look exactly like a broken
    /// application.
    #[tokio::test]
    async fn an_unmeasured_machine_admits_rather_than_blocking_the_first_start() {
        let app = test_state().await;
        let project_id = a_host_project(&app, "unmeasured-9a13").await;

        let project = projects::find_project(app.database(), &project_id)
            .await
            .expect("query")
            .expect("row");
        admit_project(&app, &project)
            .await
            .expect("nothing measured yet means nothing to refuse on");
    }

    /// The override exists because the estimate can be wrong and the user knows
    /// things the governor does not. `start` refuses; `start_forcing(.., true)`
    /// gets past the gate — and then fails for its own reasons, which is a
    /// different failure and the point of the assertion.
    #[tokio::test]
    async fn forcing_gets_past_the_governor() {
        let app = test_state().await;
        let project_id = a_host_project(&app, "forced-port-9a14").await;
        app.set_usage_for_test(full()).await;

        let refused = start(&app, &project_id).await;
        assert!(
            matches!(refused, Err(LifecycleError::NotEnoughMemory(_))),
            "got {refused:?}"
        );

        // NODEJS on a machine without Node, or a start that fails for any other
        // reason — either way it is no longer the governor refusing.
        let forced = start_forcing(&app, &project_id, true).await;
        assert!(
            !matches!(forced, Err(LifecycleError::NotEnoughMemory(_))),
            "forcing must not still be refused by the governor: {forced:?}"
        );
    }

    /// Nothing running is not a failure, and must not stop the database from
    /// being closed cleanly.
    #[tokio::test]
    async fn shutting_down_with_nothing_running_does_nothing() {
        let app = test_state().await;
        assert!(stop_host_projects(&app).await.is_empty());
        assert_eq!(host_projects_running(&app).await, 0);
    }

    /// The property the whole path exists for: a host project that was running
    /// is recorded as stopped, so the next launch does not open onto an
    /// interface claiming it runs.
    #[tokio::test]
    async fn quitting_stops_host_projects_and_records_it() {
        let app = test_state().await;
        let project_id = a_host_project(&app, "quiet-harbor-4f2a").await;

        // Start something long-running directly through the supervisor, which
        // is what HostRunner would have registered.
        let directory = tempfile::tempdir().expect("temp dir");
        #[cfg(windows)]
        let command = project_host_host_runner::ProcessCommand {
            program: "cmd".to_string(),
            args: vec!["/C".to_string(), "ping -n 60 127.0.0.1 >NUL".to_string()],
            cwd: directory.path().to_path_buf(),
            env: Default::default(),
        };
        #[cfg(unix)]
        let command = project_host_host_runner::ProcessCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "sleep 60".to_string()],
            cwd: directory.path().to_path_buf(),
            env: Default::default(),
        };

        let handle =
            project_host_host_runner::start(project_host_host_runner::SupervisorConfig::new(
                command,
                directory.path().join("run.log"),
            ))
            .await
            .expect("start");

        app.host_projects()
            .insert_for_test(&project_id, handle.clone())
            .await;

        assert_eq!(host_projects_running(&app).await, 1);

        // Mark it as running and wanted, the way a real start would have.
        projects::set_desired_state(app.database(), &project_id, DesiredState::Running)
            .await
            .expect("desired");
        projects::set_status(app.database(), &project_id, ProjectStatus::Running, None)
            .await
            .expect("status");

        let stopped = stop_host_projects(&app).await;
        assert_eq!(stopped, vec![project_id.clone()]);

        let project = projects::find_project(app.database(), &project_id)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(project.status, ProjectStatus::Stopped.as_str());

        // Still *wanted* running. That pair — wanted running, observed stopped
        // — is what a "start these again" prompt on the next launch reads.
        assert_eq!(project.desired_state, "RUNNING");

        assert!(!project_host_platform::is_alive(
            handle.pid().unwrap_or(u32::MAX)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every runtime gets something to run, and none of them gets a
    /// `Dockerfile`.
    #[test]
    fn the_scaffold_writes_starter_files_and_never_a_dockerfile() {
        for runtime in project_host_project_manager::detection::Runtime::ALL {
            let directory = tempfile::tempdir().expect("temp dir");
            scaffold(directory.path(), runtime.as_str()).expect("scaffold");

            assert!(
                !directory.path().join("Dockerfile").exists(),
                "{} was given a Dockerfile",
                runtime.as_str()
            );

            // Polyglot is the one runtime with nothing sensible to scaffold:
            // it is by definition a project that already has files.
            if runtime != project_host_project_manager::detection::Runtime::Polyglot {
                let written = std::fs::read_dir(directory.path()).expect("read").count();
                assert!(
                    written > 0,
                    "{} was scaffolded with nothing at all",
                    runtime.as_str()
                );
            }
        }
    }

    /// Once a project exists, its files belong to the user.
    #[test]
    fn a_scaffold_never_overwrites_an_edited_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        scaffold(directory.path(), "NODEJS").expect("first");

        let index = directory.path().join("index.js");
        std::fs::write(
            &index, "// mine
",
        )
        .expect("edit");

        scaffold(directory.path(), "NODEJS").expect("second");

        assert_eq!(
            std::fs::read_to_string(&index).expect("read"),
            "// mine
",
            "the scaffold clobbered a file the user had edited"
        );
    }
}
