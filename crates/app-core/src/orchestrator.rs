//! Running every one of a project's processes.
//!
//! The composition layer, in the same sense as `toolchain_flow`: `host-runner`
//! knows how to probe a toolchain, build a command, supervise one child, and
//! decide what a set of them should do; it knows nothing about projects, rows
//! or statuses. This joins the two and is the only place that holds both.
//!
//! # What a project gives up by running here
//!
//! There is no container, so there is no filesystem isolation, no network
//! isolation, no resource limit and no non-root user. A project runs as the
//! user, with the user's files and the user's network, and it is a child of
//! this process.
//!
//! The consequence that shapes this module: **projects stop when the
//! application quits.** They are not detached and not adopted on the next
//! launch. Recording a pid and reattaching fails on exactly the case that
//! matters — after a reboot the pid has been reused, and the application
//! adopts a stranger's process.
//!
//! # Where the decisions are
//!
//! Not here. Ordering, the crash rule and status aggregation live in
//! [`orchestration`](project_host_host_runner::orchestration), which is pure
//! and has no way to spawn anything. This module turns what happened into
//! events, feeds them in, and carries out the actions that come back.
//!
//! **Verified on Windows only, and only by unit tests.** No multi-process
//! project has been started by this module on any machine. See
//! `docs/superpowers/checklists/2026-08-26-process-orchestration.md` for what
//! that leaves unproven.

pub mod prepare;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use project_host_api_types::{HealthState, ProjectStatus};
use project_host_database::projects::{self, ProcessRecord, ProjectRecord, RuntimeRecord};
use project_host_database::Database;
use project_host_host_runner::health::{Check, Health};
use project_host_host_runner::orchestration::{
    Action, Event, Machine, Process, ProcessState, ProjectStatus as MachineStatus,
};
use project_host_host_runner::probe::Toolchain;
use project_host_host_runner::supervisor::{
    HealthPolicy, HostObserved, HostStatus, SupervisorConfig, SupervisorHandle, DEFAULT_GRACE,
};
use project_host_host_runner::{CommandInputs, ProcessCommand};
use tokio::sync::RwLock;

use crate::lifecycle::LifecycleError;
use crate::toolchain_flow::MachineResolver;

pub use prepare::{prepare_steps, PrepareStep};

/// What is true about a project right now, in this application's vocabulary.
///
/// A process has no health string and no image; it has a status, an exit code
/// and, when it failed, a reason worth showing someone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub status: ProjectStatus,
    /// `None` means "nothing to say", which leaves the column as it was. It is
    /// not the same as `Some(HealthState::None)`, which means the project has
    /// no health check.
    pub health: Option<HealthState>,
    pub exit_code: Option<i64>,
    pub failure_reason: Option<String>,
}

/// Everything needed to start one project.
pub struct StartContext<'a> {
    pub db: &'a Database,
    pub project: &'a ProjectRecord,
    pub directory: &'a Path,
    /// The key a project's secret environment variables are stored under.
    ///
    /// Here rather than fetched inside, because the key belongs to the
    /// installation and not to the project. `None` is a runnable state: the
    /// plaintext variables are still applied and the secrets are reported as
    /// unavailable by name.
    pub master_key: Option<&'a project_host_security::EncryptionKey>,
}

impl std::fmt::Debug for StartContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartContext")
            .field("project", &self.project.id)
            .field("directory", &self.directory)
            .field("master_key", &self.master_key.map(|_| "<held>"))
            .finish()
    }
}

/// Every process this application is running, by project.
///
/// Process-scoped by nature rather than by choice: a supervisor handle owns a
/// child of *this* process, so a registry that outlived the process would be
/// describing children that no longer exist.
#[derive(Debug, Clone, Default)]
pub struct ProcessRegistry(Arc<RwLock<BTreeMap<String, Vec<RunningProcess>>>>);

/// One supervised process, and which row it belongs to.
#[derive(Debug, Clone)]
pub struct RunningProcess {
    pub process_id: String,
    pub name: String,
    pub handle: SupervisorHandle,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    async fn set(&self, project_id: &str, processes: Vec<RunningProcess>) {
        self.0
            .write()
            .await
            .insert(project_id.to_string(), processes);
    }

    /// Register a handle without going through a start.
    ///
    /// Only for tests that need a registry with something in it: a real
    /// `Orchestrator::start` would need a project whose toolchain is actually
    /// installed on whatever machine is running the suite.
    #[cfg(test)]
    pub async fn insert_for_test(&self, project_id: &str, handle: SupervisorHandle) {
        self.set(
            project_id,
            vec![RunningProcess {
                process_id: format!("prc_{project_id}"),
                name: "main".to_string(),
                handle,
            }],
        )
        .await;
    }

    async fn remove(&self, project_id: &str) -> Vec<RunningProcess> {
        self.0.write().await.remove(project_id).unwrap_or_default()
    }

    /// Swap one process's handle for a fresh one, leaving its siblings alone.
    ///
    /// What a single-process restart needs: the project's entry keeps its
    /// order and its other handles, and only the named one is replaced.
    /// Answers whether the name was there to replace.
    async fn replace_handle(
        &self,
        project_id: &str,
        process_name: &str,
        handle: SupervisorHandle,
    ) -> bool {
        let mut registry = self.0.write().await;
        let Some(processes) = registry.get_mut(project_id) else {
            return false;
        };
        let Some(found) = processes
            .iter_mut()
            .find(|process| process.name == process_name)
        else {
            return false;
        };
        found.handle = handle;
        true
    }

    /// Every process of one project, in start order.
    pub async fn processes(&self, project_id: &str) -> Vec<RunningProcess> {
        self.0
            .read()
            .await
            .get(project_id)
            .cloned()
            .unwrap_or_default()
    }

    /// One project's first supervisor.
    ///
    /// What the console reads when no process was named. Reading a project's
    /// output is not a lifecycle operation, so it does not go through the
    /// orchestrator.
    pub async fn handle(&self, project_id: &str) -> Option<SupervisorHandle> {
        self.processes(project_id)
            .await
            .first()
            .map(|process| process.handle.clone())
    }

    /// One named process of one project.
    pub async fn handle_named(
        &self,
        project_id: &str,
        process_name: &str,
    ) -> Option<SupervisorHandle> {
        self.processes(project_id)
            .await
            .into_iter()
            .find(|process| process.name == process_name)
            .map(|process| process.handle)
    }

    /// Every project id and its processes.
    pub async fn all(&self) -> Vec<(String, Vec<RunningProcess>)> {
        self.0
            .read()
            .await
            .iter()
            .map(|(id, processes)| (id.clone(), processes.clone()))
            .collect()
    }

    /// How many *projects* have at least one process up.
    ///
    /// Projects rather than processes: this is what the quit dialog counts,
    /// and "3 projects will stop" is the sentence a user can act on where
    /// "7 processes will stop" is not.
    pub async fn running(&self) -> usize {
        self.0
            .read()
            .await
            .values()
            .filter(|processes| {
                processes
                    .iter()
                    .any(|process| process.handle.observe().status == HostStatus::Running)
            })
            .count()
    }
}

/// Starts projects as processes on this machine.
#[derive(Debug)]
pub struct Orchestrator {
    registry: ProcessRegistry,
    logs_root: PathBuf,
}

impl Orchestrator {
    pub fn new(registry: ProcessRegistry, logs_root: PathBuf) -> Self {
        Self {
            registry,
            logs_root,
        }
    }

    /// Start every process, in order.
    pub async fn start(&self, ctx: StartContext<'_>) -> Result<Observed, LifecycleError> {
        let StartContext {
            db,
            project,
            directory,
            master_key,
        } = ctx;

        let processes = projects::list_processes(db, &project.id).await?;
        if processes.is_empty() {
            // Refused by name rather than started as nothing. A project with
            // no processes is a configuration mistake, and a start that
            // silently succeeds having run nothing is the worst answer to it.
            return Err(LifecycleError::NoProcesses(project.id.clone()));
        }

        let runtime = projects::find_runtime(db, &project.id)
            .await?
            .ok_or_else(|| {
                LifecycleError::Scaffold("the project has no runtime row".to_string())
            })?;

        let is_static = runtime.runtime == STATIC;
        // A static site is served by this application and has no toolchain to
        // find, so the probe is skipped rather than answered hopefully.
        let toolchain = if is_static {
            Toolchain::NotRequired
        } else {
            resolve_toolchain(&runtime.runtime)?
        };

        let log_path = project_host_host_runner::log_path(&self.logs_root, &project.slug, &today());

        // Anything already registered is a supervisor from a previous run.
        // Ending it first is what stops a restart from leaving two processes
        // fighting over one port.
        for previous in self.registry.remove(&project.id).await.iter().rev() {
            let _ = previous.handle.stop(DEFAULT_GRACE).await;
        }

        // Only now can the port be asked about. Before the loop above, a
        // restart would find its own still-running process holding the port
        // and refuse itself; after it, whatever still has the port is
        // genuinely something else.
        crate::ports::check(db, project)
            .await
            .map_err(LifecycleError::PortConflict)?;

        // What the user configured on the settings screen. Read once and used
        // for install and build as well as for the start: a dependency install
        // that needs a private registry token needs it at install time, not
        // only at run time.
        let environment = crate::env::resolve(db, &project.id, master_key).await?;
        if !environment.undecryptable_keys.is_empty() {
            tracing::warn!(
                project = %project.id,
                keys = ?environment.undecryptable_keys,
                "these secret variables could not be decrypted and were not passed to the project"
            );
        }

        let ports = port_by_process(db, &project.id, &processes).await?;

        let mut machine = Machine::new(
            processes
                .iter()
                .map(|process| Process {
                    has_health_check: process.health_check_type != "NONE",
                })
                .collect(),
        );

        // Install and build run to completion before any process starts, and
        // their failure fails the start with their own output attached.
        for step in prepare_steps(&processes) {
            let command = build_command(
                &runtime,
                &step.command,
                &joined(directory, &step.working_dir),
                &toolchain,
                ports.get(step.process_index).copied().flatten(),
                &environment,
            )?;

            if let Err(error) =
                project_host_host_runner::run_step(step.phase, command, log_path.clone()).await
            {
                let actions = machine.handle(Event::PrepareFailed {
                    index: step.process_index,
                    reason: error.to_string(),
                });
                self.apply(db, &processes, &actions, &[]).await;
                return Err(LifecycleError::from(error));
            }
        }

        let mut running: Vec<RunningProcess> = Vec::new();
        let mut actions = machine.start();

        loop {
            let spawn = actions.iter().find_map(|action| match action {
                Action::Spawn { index } => Some(*index),
                _ => None,
            });

            self.apply(db, &processes, &actions, &running).await;

            let Some(index) = spawn else {
                break;
            };
            let Some(process) = processes.get(index) else {
                break;
            };

            let port = ports.get(index).copied().flatten();
            let command = if is_static {
                static_command(
                    &joined(directory, &process.working_dir),
                    runtime.publish_dir.as_deref(),
                    port,
                    &environment,
                )?
            } else {
                build_command(
                    &runtime,
                    &process.command,
                    &joined(directory, &process.working_dir),
                    &toolchain,
                    port,
                    &environment,
                )?
            };

            let mut config = SupervisorConfig::new(command, log_path.clone());
            config.health = health_policy(process, port);
            // A process follows the project's restart-policy column. `NO` is
            // the only value that means "leave it alone".
            config.restart_on_crash = project.restart_policy != "NO";

            match project_host_host_runner::start(config).await {
                Ok(handle) => {
                    running.push(RunningProcess {
                        process_id: process.id.clone(),
                        name: process.name.clone(),
                        handle: handle.clone(),
                    });
                    self.registry.set(&project.id, running.clone()).await;

                    let observed = handle.observe();
                    if observed.status != HostStatus::Running {
                        // It did not survive its settle window. The supervisor
                        // already has the child's own words; the machine
                        // decides what that means for the siblings.
                        actions = machine.handle(Event::Exited {
                            index,
                            code: observed.exit_code.and_then(|code| i32::try_from(code).ok()),
                            terminal: true,
                        });
                        continue;
                    }

                    if process.health_check_type == "NONE" {
                        actions = machine.handle(Event::Settled { index });
                    } else if await_health(&handle, process).await {
                        actions = machine.handle(Event::Healthy { index });
                    } else {
                        // Stop it first. The machine reads `Exited` as "this
                        // one is already dead", and so excludes it from the
                        // teardown of its siblings — which is right for a real
                        // exit and wrong here, because a process that failed
                        // its health check is still running and still holding
                        // its port. Without this the project is recorded
                        // FAILED while its main process keeps going.
                        if let Err(error) = handle.stop(DEFAULT_GRACE).await {
                            tracing::warn!(
                                project = %project.id,
                                process = %process.name,
                                %error,
                                "an unhealthy process would not stop"
                            );
                        }
                        actions = machine.handle(Event::Exited {
                            index,
                            code: None,
                            terminal: true,
                        });
                    }
                }
                Err(error) => {
                    actions = machine.handle(Event::PrepareFailed {
                        index,
                        reason: error.to_string(),
                    });
                    self.apply(db, &processes, &actions, &running).await;
                    self.stop_all(&running).await;
                    return Err(LifecycleError::from(error));
                }
            }
        }

        self.registry.set(&project.id, running.clone()).await;

        let status = machine.project_status();
        if status == MachineStatus::RUNNING {
            projects::record_started(db, &project.id).await?;
        }

        Ok(observed_for(status, &running))
    }

    /// Stop every process, in reverse order.
    pub async fn stop(&self, project: &ProjectRecord) -> Result<(), LifecycleError> {
        let running = self.registry.remove(&project.id).await;
        self.stop_all(&running).await;
        Ok(())
    }

    /// Kill every process immediately, in reverse order.
    ///
    /// What a stuck process gets. `stop` asks and waits out the grace period;
    /// this does not ask.
    pub async fn kill(&self, project: &ProjectRecord) -> Result<(), LifecycleError> {
        let running = self.registry.remove(&project.id).await;
        for process in running.iter().rev() {
            if let Err(error) = process.handle.kill().await {
                tracing::warn!(
                    project = %project.id,
                    process = %process.name,
                    %error,
                    "a process could not be killed"
                );
            }
        }
        Ok(())
    }

    /// What the project's processes are reporting, or `None` if it is not
    /// running here.
    pub async fn observe(
        &self,
        project: &ProjectRecord,
    ) -> Result<Option<Observed>, LifecycleError> {
        let running = self.registry.processes(&project.id).await;
        if running.is_empty() {
            return Ok(None);
        }
        Ok(Some(observed_for(aggregate(&running), &running)))
    }

    /// Stop and restart one named process, leaving its siblings alone.
    ///
    /// The one operation that deliberately does not go through the state
    /// machine: the user asked about this process, not about the project, and
    /// the siblings keep their pids.
    ///
    /// Both halves are done here, and the second is the one that used to be
    /// missing. A supervisor is bound to the child it was created with: once
    /// `stop` has run, its task records the exit and returns, and no restart
    /// policy applies because the exit was requested. Stopping alone therefore
    /// ended the process for good while the window reported it restarted. So a
    /// fresh supervisor is spawned and swapped into the registry in its place.
    pub async fn restart_process(
        &self,
        ctx: StartContext<'_>,
        process_name: &str,
    ) -> Result<(), LifecycleError> {
        let StartContext {
            db,
            project,
            directory,
            master_key,
        } = ctx;

        let Some(previous) = self.registry.handle_named(&project.id, process_name).await else {
            return Err(LifecycleError::NoSuchProcess {
                project: project.id.clone(),
                process: process_name.to_string(),
            });
        };

        let processes = projects::list_processes(db, &project.id).await?;
        let Some((index, process)) = processes
            .iter()
            .enumerate()
            .find(|(_, candidate)| candidate.name == process_name)
        else {
            return Err(LifecycleError::NoSuchProcess {
                project: project.id.clone(),
                process: process_name.to_string(),
            });
        };

        let runtime = projects::find_runtime(db, &project.id)
            .await?
            .ok_or_else(|| {
                LifecycleError::Scaffold("the project has no runtime row".to_string())
            })?;
        let is_static = runtime.runtime == STATIC;
        let toolchain = if is_static {
            Toolchain::NotRequired
        } else {
            resolve_toolchain(&runtime.runtime)?
        };

        // Everything is resolved before the running process is touched. A
        // restart that stopped first and then discovered the toolchain had
        // gone would leave the user with neither the old process nor a new
        // one, having reported a restart.
        let environment = crate::env::resolve(db, &project.id, master_key).await?;
        let port = port_by_process(db, &project.id, &processes)
            .await?
            .get(index)
            .copied()
            .flatten();
        let command = if is_static {
            static_command(
                &joined(directory, &process.working_dir),
                runtime.publish_dir.as_deref(),
                port,
                &environment,
            )?
        } else {
            build_command(
                &runtime,
                &process.command,
                &joined(directory, &process.working_dir),
                &toolchain,
                port,
                &environment,
            )?
        };

        // Only now. The old process has to be gone before the new one is
        // spawned, or the two fight over the port.
        previous.stop(DEFAULT_GRACE).await?;

        let log_path = project_host_host_runner::log_path(&self.logs_root, &project.slug, &today());
        let mut config = SupervisorConfig::new(command, log_path);
        config.health = health_policy(process, port);
        config.restart_on_crash = project.restart_policy != "NO";

        let handle = project_host_host_runner::start(config).await?;
        if !self
            .registry
            .replace_handle(&project.id, process_name, handle.clone())
            .await
        {
            // The project was stopped out from under the restart. The new
            // process is not in the registry, so nothing could ever stop it.
            let _ = handle.stop(DEFAULT_GRACE).await;
            return Err(LifecycleError::NoSuchProcess {
                project: project.id.clone(),
                process: process_name.to_string(),
            });
        }

        let observed = handle.observe();
        let state = match observed.status {
            HostStatus::Running => ProcessState::Running,
            _ => ProcessState::Crashed,
        };
        if let Err(error) = projects::set_process_status(
            db,
            &process.id,
            state.as_str(),
            handle.pid().map(i64::from),
            observed.exit_code,
            observed.failure_reason.as_deref(),
        )
        .await
        {
            tracing::warn!(process = %process.name, %error, "could not record a restarted process");
        }
        if let Err(error) = projects::increment_process_restart_count(db, &process.id).await {
            tracing::warn!(process = %process.name, %error, "could not count a restart");
        }

        Ok(())
    }

    async fn stop_all(&self, running: &[RunningProcess]) {
        for process in running.iter().rev() {
            if let Err(error) = process.handle.stop(DEFAULT_GRACE).await {
                tracing::warn!(process = %process.name, %error, "a process would not stop");
            }
        }
    }

    /// Carry out what the machine decided.
    async fn apply(
        &self,
        db: &Database,
        processes: &[ProcessRecord],
        actions: &[Action],
        running: &[RunningProcess],
    ) {
        for action in actions {
            match action {
                Action::WriteProcessStatus {
                    index,
                    state,
                    reason,
                } => {
                    let Some(process) = processes.get(*index) else {
                        continue;
                    };
                    let pid = running
                        .iter()
                        .find(|candidate| candidate.process_id == process.id)
                        .and_then(|candidate| candidate.handle.pid())
                        .map(i64::from);
                    let exit_code = running
                        .iter()
                        .find(|candidate| candidate.process_id == process.id)
                        .and_then(|candidate| candidate.handle.observe().exit_code);

                    if let Err(error) = projects::set_process_status(
                        db,
                        &process.id,
                        state.as_str(),
                        pid,
                        exit_code,
                        reason.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!(process = %process.name, %error, "could not record a process status");
                    }
                }
                Action::Stop { index } => {
                    let Some(process) = processes.get(*index) else {
                        continue;
                    };
                    if let Some(found) = running
                        .iter()
                        .find(|candidate| candidate.process_id == process.id)
                    {
                        if let Err(error) = found.handle.stop(DEFAULT_GRACE).await {
                            tracing::warn!(process = %process.name, %error, "a sibling would not stop");
                        }
                    }
                }
                // Spawning is driven by the loop, which needs the handle the
                // spawn produces. The project's status is written by the
                // lifecycle layer from what `start` returns.
                Action::Spawn { .. } | Action::WriteProjectStatus { .. } => {}
            }
        }
    }
}

/// Wait for a process to report healthy, or give up.
///
/// Bounded by the check's own configuration — its start period plus its
/// retries — because a health check that never passes must fail the start
/// rather than hang it. Returns whether it passed.
async fn await_health(handle: &SupervisorHandle, process: &ProcessRecord) -> bool {
    let start_period = Duration::from_secs(process.health_start_period_s.clamp(0, 3600) as u64);
    let interval = Duration::from_secs(process.health_interval_s.clamp(1, 3600) as u64);
    let retries = process.health_retries.clamp(1, 100) as u32;
    let deadline = start_period + interval * retries;

    let began = std::time::Instant::now();
    loop {
        match handle.observe().health {
            Health::Passing => return true,
            // Not yet asked. The supervisor starts every process at
            // `Health::None` and only writes a verdict after the first tick,
            // which is scheduled a whole start period away — twenty seconds by
            // default. Reading this as "nothing to wait for" is what made the
            // gate pass instantly for every process, so a process was reported
            // healthy before its check had run once, its successor spawned
            // immediately, and a check that could never pass could never fail
            // the start. There is always a policy here: `await_health` is
            // called only when `health_check_type` is not `NONE`, which is the
            // same condition under which `health_policy` returns one.
            Health::None => {}
            Health::Failing(_) => {}
        }

        if handle.observe().status != HostStatus::Running {
            return false;
        }
        if began.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The port each process was allocated, indexed the same way the process list
/// is.
async fn port_by_process(
    db: &Database,
    project_id: &str,
    processes: &[ProcessRecord],
) -> Result<Vec<Option<u16>>, LifecycleError> {
    let ports = projects::list_ports(db, project_id).await?;

    // A port with no process is the project's, from before the process set
    // existed. It goes to the first process, which is what it meant.
    let unassigned = ports
        .iter()
        .find(|port| port.process_id.is_none() && port.is_primary)
        .or_else(|| ports.iter().find(|port| port.process_id.is_none()))
        .and_then(|port| port.host_port)
        .and_then(|port| u16::try_from(port).ok());

    Ok(processes
        .iter()
        .enumerate()
        .map(|(index, process)| {
            ports
                .iter()
                .find(|port| port.process_id.as_deref() == Some(process.id.as_str()))
                .and_then(|port| port.host_port)
                .and_then(|port| u16::try_from(port).ok())
                .or(if index == 0 { unassigned } else { None })
        })
        .collect())
}

/// A process's working directory as a real path.
///
/// The stored value is relative to the project, and `.` is the common case.
fn joined(directory: &Path, working_dir: &str) -> PathBuf {
    let trimmed = working_dir.trim();
    if trimmed.is_empty() || trimmed == "." {
        return directory.to_path_buf();
    }
    directory.join(trimmed)
}

/// Find the runtime's toolchain, or refuse with what was looked for.
///
/// Refused here rather than at spawn time so the message names the runtime and
/// the executables tried, instead of being an operating-system error about a
/// file that does not exist.
fn resolve_toolchain(runtime: &str) -> Result<Toolchain, LifecycleError> {
    // A runtime with no candidate executable cannot be answered by finding
    // one. `STATIC` is served by this application and never reaches here;
    // `POLYGLOT` needs several toolchains at once, and probing for one and
    // failing would refuse a project that runs perfectly well. What checks
    // such a project is the start command's own program, by name.
    if project_host_host_runner::candidates_for(runtime).is_empty() {
        return Ok(Toolchain::NotRequired);
    }

    match project_host_host_runner::probe(runtime, &MachineResolver) {
        found @ Toolchain::Found { .. } => Ok(found),
        Toolchain::NotRequired => Ok(Toolchain::NotRequired),
        Toolchain::Missing { looked_for } => Err(LifecycleError::ToolchainMissing {
            runtime: runtime.to_string(),
            looked_for,
        }),
    }
}

/// The runtime wire value for a static site.
const STATIC: &str = "STATIC";

/// The flag that puts this application into static-server mode.
///
/// Defined here and read by the desktop shell's `main`, so the two cannot
/// drift apart into a project that spawns a flag nothing handles.
pub const SERVE_STATIC_FLAG: &str = "--serve-static";

/// The command that serves a static project's files.
///
/// This application's own executable, in its static-server mode. Reaching for
/// whatever the machine happened to have — `python -m http.server`,
/// `npx serve` — was rejected: it would make a static site the only runtime
/// whose ability to start depends on a language the project does not use.
fn static_command(
    directory: &Path,
    publish_dir: Option<&str>,
    port: Option<u16>,
    environment: &crate::env::Resolved,
) -> Result<ProcessCommand, LifecycleError> {
    let executable = std::env::current_exe().map_err(|error| {
        LifecycleError::Scaffold(format!(
            "this application could not find its own executable, so it cannot \
             serve a static site: {error}"
        ))
    })?;

    let root = match publish_dir.map(str::trim).filter(|value| !value.is_empty()) {
        // Relative to the project, and refused if it could climb out of it: a
        // `publish_dir` of `../..` would publish the user's whole disk on a
        // port.
        //
        // Checked by *component*, not by comparing the joined path with the
        // root. `directory.join("../..")` still begins with `directory` as a
        // string, so `starts_with` says yes to exactly the path this is meant
        // to refuse — the components have to be looked at before any joining
        // happens.
        Some(relative) => {
            // Read the same way on every platform, because the value is not a
            // path on *this* machine — it comes from the project's config and
            // travels with the project.
            let mut candidate = directory.to_path_buf();
            for part in relative.split(['/', '\\']) {
                match part {
                    "" | "." => {}
                    ".." => {
                        return Err(LifecycleError::Scaffold(
                            "the publish directory climbs out of the project".to_string(),
                        ))
                    }
                    part if part.contains(':') => {
                        return Err(LifecycleError::Scaffold(
                            "the publish directory is an absolute path".to_string(),
                        ))
                    }
                    part => candidate.push(part),
                }
            }
            candidate
        }
        None => directory.to_path_buf(),
    };

    let port = port.unwrap_or(0);
    let mut env = environment.values.clone();
    env.insert("PORT".to_string(), port.to_string());

    Ok(ProcessCommand {
        program: executable.to_string_lossy().into_owned(),
        args: vec![
            SERVE_STATIC_FLAG.to_string(),
            root.to_string_lossy().into_owned(),
            "--port".to_string(),
            port.to_string(),
        ],
        cwd: directory.to_path_buf(),
        env,
    })
}

fn build_command(
    runtime: &RuntimeRecord,
    text: &str,
    directory: &Path,
    toolchain: &Toolchain,
    port: Option<u16>,
    environment: &crate::env::Resolved,
) -> Result<ProcessCommand, LifecycleError> {
    project_host_host_runner::start_command(CommandInputs {
        runtime: &runtime.runtime,
        command: text,
        project_directory: directory,
        // The same resolver the toolchain probe uses, so `npm` resolves to
        // `npm.cmd` on Windows rather than to nothing.
        resolver: &MachineResolver,
        toolchain,
        env: environment.values.clone(),
        port,
    })
    .map_err(LifecycleError::from)
}

/// The health policy from the process row, or `None` when no check is
/// configured.
fn health_policy(process: &ProcessRecord, port: Option<u16>) -> Option<HealthPolicy> {
    if process.health_check_type == "NONE" {
        return None;
    }
    Some(HealthPolicy {
        check: Check::resolved(
            &process.health_check_type,
            process.health_check_target.as_deref(),
            process.health_timeout_s,
            port,
        ),
        interval: Duration::from_secs(process.health_interval_s.clamp(1, 3600) as u64),
        start_period: Duration::from_secs(process.health_start_period_s.clamp(0, 3600) as u64),
    })
}

/// Today, as the log file names it.
fn today() -> String {
    project_host_database::time::now()
        .get(..10)
        .unwrap_or("unknown")
        .to_string()
}

/// What a supervisor handle is reporting, in this application's vocabulary.
///
/// The only way anything outside this module reads a handle. `reconcile` needs
/// it to carry a crash into the row.
pub fn observed_from(handle: &SupervisorHandle) -> Observed {
    translate(&handle.observe())
}

/// The status of a set of running processes, by the same rule the state
/// machine uses.
fn aggregate(running: &[RunningProcess]) -> MachineStatus {
    if running.is_empty() {
        return MachineStatus::STOPPED;
    }
    if running
        .iter()
        .any(|p| p.handle.observe().status == HostStatus::Failed)
    {
        return MachineStatus::FAILED;
    }
    if running
        .iter()
        .any(|p| p.handle.observe().status == HostStatus::Crashed)
    {
        return MachineStatus::CRASHED;
    }
    if running
        .iter()
        .all(|p| p.handle.observe().status == HostStatus::Running)
    {
        return MachineStatus::RUNNING;
    }
    MachineStatus::STOPPED
}

/// The project's observed state, with the first unhappy process's words.
fn observed_for(status: MachineStatus, running: &[RunningProcess]) -> Observed {
    let unhappy = running
        .iter()
        .map(|process| (process, process.handle.observe()))
        .find(|(_, observed)| observed.status != HostStatus::Running);

    let (exit_code, failure_reason) = match &unhappy {
        Some((process, observed)) => (
            observed.exit_code,
            observed
                .failure_reason
                .clone()
                // Which process failed is the first thing the user needs, and
                // the supervisor does not know its name.
                .map(|reason| format!("{}: {reason}", process.name)),
        ),
        None => (None, None),
    };

    Observed {
        status: match status.as_str() {
            "RUNNING" => ProjectStatus::Running,
            "CRASHED" => ProjectStatus::Crashed,
            "FAILED" => ProjectStatus::Failed,
            "STARTING" | "RESTARTING" => ProjectStatus::Starting,
            _ => ProjectStatus::Stopped,
        },
        health: Some(health_of(running)),
        exit_code,
        failure_reason,
    }
}

/// The project's health: the worst of its processes'.
fn health_of(running: &[RunningProcess]) -> HealthState {
    let mut any_check = false;
    for process in running {
        match process.handle.observe().health {
            Health::None => {}
            Health::Passing => any_check = true,
            Health::Failing(_) => return HealthState::Unhealthy,
        }
    }
    if any_check {
        HealthState::Healthy
    } else {
        HealthState::None
    }
}

/// `host-runner`'s vocabulary in this application's.
fn translate(observed: &HostObserved) -> Observed {
    Observed {
        status: match observed.status {
            HostStatus::Running => ProjectStatus::Running,
            HostStatus::Stopped => ProjectStatus::Stopped,
            HostStatus::Crashed => ProjectStatus::Crashed,
            HostStatus::Failed => ProjectStatus::Failed,
        },
        health: Some(match observed.health {
            // No check configured. Not "unknown": nothing was asked, and the
            // column has a word for that.
            Health::None => HealthState::None,
            Health::Passing => HealthState::Healthy,
            Health::Failing(_) => HealthState::Unhealthy,
        }),
        exit_code: observed.exit_code,
        failure_reason: observed.failure_reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(name: &str, working_dir: &str) -> ProcessRecord {
        ProcessRecord {
            id: format!("prc_{name}"),
            project_id: "prj_1".to_string(),
            name: name.to_string(),
            start_order: 0,
            command: "node index.js".to_string(),
            working_dir: working_dir.to_string(),
            install_command: None,
            build_command: None,
            health_check_type: "NONE".to_string(),
            health_check_target: None,
            health_interval_s: 30,
            health_timeout_s: 5,
            health_retries: 3,
            health_start_period_s: 20,
            status: "STOPPED".to_string(),
            pid: None,
            exit_code: None,
            failure_reason: None,
            started_at: None,
            restart_count: 0,
        }
    }

    #[test]
    fn a_dot_working_directory_is_the_project_itself() {
        let root = Path::new("/projects/shop");
        assert_eq!(joined(root, "."), root);
        assert_eq!(joined(root, ""), root);
        assert_eq!(joined(root, "   "), root);
    }

    #[test]
    fn a_relative_working_directory_lands_inside_the_project() {
        let root = Path::new("/projects/shop");
        assert_eq!(joined(root, "packages/api"), root.join("packages/api"));
    }

    /// The one place a stored value becomes a served directory, and the whole
    /// security boundary of a static project.
    #[test]
    fn a_publish_directory_cannot_climb_out_of_the_project() {
        let environment = crate::env::Resolved::default();

        for hostile in [
            "..",
            "../..",
            "../../etc",
            "a/../..",
            "..\\..",
            "C:\\Windows",
        ] {
            let refused = static_command(
                Path::new("/projects/shop"),
                Some(hostile),
                Some(3000),
                &environment,
            );
            assert!(refused.is_err(), "`{hostile}` was accepted");
        }
    }

    #[test]
    fn a_publish_directory_inside_the_project_is_served() {
        let environment = crate::env::Resolved::default();
        let command = static_command(
            Path::new("/projects/shop"),
            Some("dist"),
            Some(3000),
            &environment,
        )
        .expect("dist is inside the project");

        assert!(command.args.iter().any(|arg| arg.ends_with("dist")));
    }

    #[test]
    fn a_process_without_a_health_check_has_no_policy() {
        assert!(health_policy(&process("main", "."), Some(3000)).is_none());
    }

    #[test]
    fn a_process_with_a_health_check_has_one() {
        let mut record = process("api", ".");
        record.health_check_type = "HTTP".to_string();
        record.health_check_target = Some("/health".to_string());

        assert!(health_policy(&record, Some(3000)).is_some());
    }

    /// A project whose processes are all up is running; anything else is not.
    #[test]
    fn an_empty_process_set_is_stopped() {
        assert_eq!(aggregate(&[]), MachineStatus::STOPPED);
        assert_eq!(
            observed_for(MachineStatus::STOPPED, &[]).status,
            ProjectStatus::Stopped
        );
    }
}
