//! State shared across every command handler.

use std::sync::Arc;

use project_host_compatibility::{Assessment, ResourceDefaults};
use project_host_database::Database;
use tokio::sync::RwLock;

use crate::config::AppConfig;
use crate::orchestrator::ProcessRegistry;
use project_host_resources::{MachineUsage, Reserve, SystemUsage, UsageSource};

use crate::keys::MasterKey;
use project_host_discord_gateway::Supervisor;

/// Everything a command handler needs. Cheap to clone: one `Arc`.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

/// `Debug` is written by hand rather than derived, because this struct ends up
/// inside error contexts and a derived one would print the whole configuration.
pub struct Inner {
    pub config: AppConfig,
    pub database: Database,
    /// Last observed daemon status, refreshed on a timer rather than probed per
    /// call — a status bar that pinged Docker on every render would turn a slow
    /// daemon into a slow interface.
    /// What this machine is, and the resource defaults that follow from it.
    ///
    /// Decided once at startup: the hardware does not change while the process
    /// runs, and re-scanning per project creation would spawn subprocesses on a
    /// path the user is waiting on. Existing projects are never touched — a
    /// user who set a limit deliberately does not have it overwritten.
    pub assessment: Assessment,
    pub instance_id: String,
    pub app_version: String,
    pub schema_version: u32,
    pub started_at: std::time::Instant,
    pub started_at_wall: String,
    /// Every host project this process is running.
    ///
    /// Lives here because it is process-scoped by nature: a supervisor handle
    /// owns a child of *this* process, so nothing outside the process can hold
    /// a meaningful one.
    pub host_projects: ProcessRegistry,
    /// Where usage figures come from. Behind a trait so a test can describe a
    /// machine that is out of memory, which is the case that matters and the
    /// one that cannot be arranged for real.
    pub usage: Arc<dyn UsageSource>,
    /// The last machine sample, refreshed on a timer rather than read per call.
    ///
    /// The same reason `docker_status` is: walking the process table on every
    /// render would turn a busy machine into an unusable interface.
    pub machine_usage: RwLock<MachineUsage>,
    /// How much memory is held back from the budget.
    pub reserve: Reserve,
    /// Every Discord bot this process is connected as.
    ///
    /// Process-scoped for the same reason `host_projects` is: a live gateway
    /// session belongs to *this* process, so nothing outside it could hold a
    /// meaningful handle.
    pub discord: Supervisor,
    /// Sleep behaviour and scheduling priority for this machine.
    ///
    /// Behind a lock because it is genuinely mutable state — a moving average,
    /// a cooldown, what each project was last set to — and one writer (the
    /// background tick) contends with occasional readers (the window asking
    /// what the current profile is). A `RwLock` rather than a `Mutex` so that
    /// rendering never waits on another render.
    pub power: RwLock<project_host_power::PowerManager>,
    /// The key every stored secret is encrypted under.
    ///
    /// `None` when secure storage could not be opened. That is a degraded but
    /// runnable state — projects still work — so it is reported rather than
    /// fatal, and the features that need a key say why they are unavailable.
    pub master_key: Option<MasterKey>,
}

/// Who this process is: the facts fixed at startup that never change.
///
/// Grouped rather than passed as four loose strings, because three of them are
/// `String` and a caller that swapped two would compile.
#[derive(Debug, Clone)]
pub struct Identity {
    /// Fresh on every start, so log lines from two runs can be told apart.
    pub instance_id: String,
    pub app_version: String,
    pub schema_version: u32,
    /// Wall-clock start time, for display. Uptime uses the monotonic clock.
    pub started_at_wall: String,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        database: Database,
        assessment: Assessment,
        identity: Identity,
        master_key: Option<MasterKey>,
    ) -> Self {
        Self(Arc::new(Inner {
            config,
            database,
            assessment,
            instance_id: identity.instance_id,
            app_version: identity.app_version,
            schema_version: identity.schema_version,
            started_at: std::time::Instant::now(),
            started_at_wall: identity.started_at_wall,
            host_projects: ProcessRegistry::new(),
            usage: Arc::new(SystemUsage::new()),
            // Unknown until the sampler has run once. Admission treats that as
            // "cannot say" and allows, which is the honest answer before
            // anything has been measured.
            machine_usage: RwLock::new(MachineUsage::unknown()),
            reserve: Reserve::default(),
            discord: Supervisor::new(),
            power: RwLock::new(project_host_power::PowerManager::new(Arc::new(
                project_host_power::monitor::MachineMonitor::new(),
            ))),
            master_key,
        }))
    }

    /// The last machine sample.
    pub async fn machine_usage(&self) -> MachineUsage {
        *self.0.machine_usage.read().await
    }

    /// Re-sample and store. Called by the background sampler.
    pub async fn refresh_machine_usage(&self) -> MachineUsage {
        // Sampled outside the lock: reading the process table is the slow part,
        // and holding the write lock across it would block every render.
        let sample = self.0.usage.machine();
        *self.0.machine_usage.write().await = sample;
        sample
    }

    pub fn usage_source(&self) -> &Arc<dyn UsageSource> {
        &self.0.usage
    }

    pub fn reserve(&self) -> Reserve {
        self.0.reserve
    }

    /// Replace the usage source and the last sample.
    ///
    /// For tests that need a machine which is out of memory — the case the
    /// governor exists for, and the one that cannot be arranged for real
    /// without making the machine running the tests unusable.
    #[cfg(test)]
    pub async fn set_usage_for_test(&self, sample: MachineUsage) {
        *self.0.machine_usage.write().await = sample;
    }

    /// Every host project this process is running.
    pub fn host_projects(&self) -> &ProcessRegistry {
        &self.0.host_projects
    }

    /// Where a project's run log is written.
    ///
    /// Falls back to a relative `logs` directory for the same reason
    /// `projects_dir` does: a configuration that named neither should still
    /// produce a working application rather than a panic at the first start.
    pub fn logs_root(&self) -> std::path::PathBuf {
        self.0
            .config
            .log_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("logs"))
    }

    pub fn inner(&self) -> &Inner {
        &self.0
    }

    pub fn config(&self) -> &AppConfig {
        &self.0.config
    }

    pub fn database(&self) -> &Database {
        &self.0.database
    }

    /// Every Discord bot this process is connected as.
    pub fn discord(&self) -> &Supervisor {
        &self.0.discord
    }

    /// Sleep behaviour and scheduling priority.
    ///
    /// Returned as the lock rather than a snapshot because both readers and the
    /// background tick need it, and a method per question would put the whole
    /// power interface on `AppState`.
    pub fn power(&self) -> &RwLock<project_host_power::PowerManager> {
        &self.0.power
    }

    /// The key stored secrets are encrypted under, if there is one.
    pub fn master_key(&self) -> Option<&project_host_security::EncryptionKey> {
        self.0.master_key.as_ref().map(|held| &held.key)
    }

    /// Which mechanism is holding the key, for the window to report honestly.
    pub fn key_backend(&self) -> Option<&'static str> {
        self.0.master_key.as_ref().map(|held| held.backend.as_str())
    }

    /// The resource limits a newly created project should start with.
    pub fn resource_defaults(&self) -> ResourceDefaults {
        self.0.assessment.defaults
    }

    pub fn assessment(&self) -> Assessment {
        self.0.assessment
    }

    /// Monotonic, so a system clock change cannot produce a negative uptime.
    pub fn uptime_seconds(&self) -> u64 {
        self.0.started_at.elapsed().as_secs()
    }
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("instance_id", &self.instance_id)
            .field("app_version", &self.app_version)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("instance_id", &self.0.instance_id)
            .field("app_version", &self.0.app_version)
            .finish_non_exhaustive()
    }
}
