//! The desktop shell.
//!
//! Deliberately thin. Every command here does three things and no more: take
//! validated arguments, call into a domain crate, and turn the result into
//! something the window can render. No business logic lives in this crate,
//! because anything that lives here cannot be tested without a window.
//!
//! The [`Runtime`] is started once, before the window opens, and handed to
//! Tauri as managed state. Commands borrow it; none of them own it. Docker,
//! WSL and other optional probes run after the window exists, so a hung
//! daemon cannot keep the window closed.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};

use project_host_core::provisioning::SourceSpec;
use project_host_core::{resolve_paths, AppConfig, AppState, Runtime, StartupDiagnostics};
use project_host_database::projects;
use project_host_platform::PathProvider;
use project_host_project_manager::names::{sanitise_display_name, Slug};
use project_host_project_manager::ports::PortPool;
use project_host_security::Secret;
use project_host_updater::{evaluate, ReleaseManifest, UpdateCheck, UpdatePreferences};

/// Which file imports are running, and which the user has asked to stop.
///
/// `running` exists so the sweep for staging directories left by a crashed
/// import can tell wreckage from an import that is still copying into one.
#[derive(Debug, Clone, Default)]
struct FileImportCancels {
    import_ids: Arc<Mutex<HashSet<String>>>,
    running: Arc<Mutex<HashSet<String>>>,
}

/// Marks one import as running for as long as it is held.
///
/// A guard rather than two calls so that every exit path releases it, including
/// `?` returning early and the blocking task panicking — the same reason
/// [`UpdateActivity`] uses one.
struct RunningImport {
    imports: FileImportCancels,
    import_id: String,
}

impl Drop for RunningImport {
    fn drop(&mut self) {
        self.imports.running().remove(&self.import_id);
        // A cancellation is only ever about the import it named, and that
        // import is over.
        self.imports.clear(&self.import_id);
    }
}

impl FileImportCancels {
    /// A panic elsewhere poisons the mutex for the rest of the process. The set
    /// behind it is a plain `HashSet<String>` with no invariant a panic could
    /// have left half-applied, so the poison is recovered from rather than
    /// propagated: the alternative is that one unrelated panic silently
    /// cancels, or refuses to cancel, every import until the app is restarted.
    fn ids(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.import_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn clear(&self, import_id: &str) {
        self.ids().remove(import_id);
    }

    fn cancel(&self, import_id: String) {
        self.ids().insert(import_id);
    }

    fn is_cancelled(&self, import_id: &str) -> bool {
        self.ids().contains(import_id)
    }

    fn running(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn start(&self, import_id: String) -> RunningImport {
        self.running().insert(import_id.clone());
        RunningImport {
            imports: self.clone(),
            import_id,
        }
    }

    fn is_running(&self, import_id: &str) -> bool {
        self.running().contains(import_id)
    }
}

/// How long the release check may take before giving up.
///
/// Short, because it runs at startup and a slow feed must never delay the
/// window appearing.
const UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What a command hands back when it fails.
///
/// A string rather than a rich type: these are shown to a person, and a failed
/// command must never leak a path, a query or a token into the interface.
#[derive(Debug, Serialize)]
pub struct CommandError {
    message: String,
}

impl<E: std::fmt::Display> From<E> for CommandError {
    fn from(error: E) -> Self {
        // The full error goes to the log; the window gets the summary.
        tracing::error!(%error, "command failed");
        Self {
            message: error.to_string(),
        }
    }
}

type CommandResult<T> = Result<T, CommandError>;

/// Whether the core has finished starting. The window is shown before that
/// happens, so the loading page can ask this without talking to `AppState`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum LaunchStatus {
    Starting,
    Ready,
    Failed { message: String },
}

/// The state the window shows in its header.
#[derive(Debug, Serialize)]
pub struct SystemStatus {
    pub app_version: String,
    pub schema_version: u32,
    pub uptime_seconds: u64,
    pub started_at: String,
}

/// One row in the project list.
#[derive(Debug, Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub project_type: String,
    pub status: String,
    pub desired_state: String,
    pub color: Option<String>,
}

#[tauri::command]
async fn system_status(state: tauri::State<'_, AppState>) -> CommandResult<SystemStatus> {
    // Bound through the deref first: `tauri::State` has its own inherent
    // `inner()`, which returns the managed value and would otherwise shadow
    // `AppState::inner()` at every call site.
    let app: &AppState = &state;
    let facts = app.inner();

    Ok(SystemStatus {
        app_version: facts.app_version.clone(),
        schema_version: facts.schema_version,
        uptime_seconds: app.uptime_seconds(),
        started_at: facts.started_at_wall.clone(),
    })
}

/// How long each startup stage took. Surfaced so a slow launch names the
/// stage that caused it, rather than only that the window was late.
#[tauri::command]
fn startup_diagnostics(
    diagnostics: tauri::State<'_, Arc<Mutex<StartupDiagnostics>>>,
) -> CommandResult<StartupDiagnostics> {
    Ok(diagnostics
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone())
}

#[tauri::command]
fn launch_status(status: tauri::State<'_, Arc<Mutex<LaunchStatus>>>) -> LaunchStatus {
    status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[tauri::command]
async fn list_projects(state: tauri::State<'_, AppState>) -> CommandResult<Vec<ProjectSummary>> {
    let records = projects::list_projects(state.database(), false, None, 200).await?;
    Ok(records
        .into_iter()
        .map(|record| ProjectSummary {
            id: record.id,
            slug: record.slug,
            display_name: record.display_name,
            description: record.description,
            project_type: record.project_type,
            status: record.status,
            desired_state: record.desired_state,
            color: record.color,
        })
        .collect())
}

/// What the creation form sends.
#[derive(Debug, serde::Deserialize)]
pub struct NewProjectRequest {
    pub display_name: String,
    pub description: String,
    /// A runtime to use regardless of what the files say. Absent means "look at
    /// the files and decide", which is the normal case for anything fetched.
    ///
    /// An empty project has no files, so one is required there — nothing can be
    /// detected from nothing.
    #[serde(default)]
    pub runtime: Option<String>,
    /// Where the files come from. Absent means an empty project, which keeps
    /// older callers working.
    #[serde(default)]
    pub source: Option<SourceRequest>,
}

/// The source half of the creation form.
///
/// `Deserialize` only — this type is never sent back, which is what keeps the
/// token out of every response. `Debug` is hand-written for the same reason it is
/// on `SourceCredential`: the realistic leak is a handler logging the request it
/// failed to process.
#[derive(serde::Deserialize)]
pub struct SourceRequest {
    /// `EMPTY`, `GIT_CLONE`, `REMOTE_ARCHIVE` or `GITHUB_CLI`.
    pub kind: String,
    #[serde(default)]
    pub url: Option<String>,
    /// Branch or tag for `GIT_CLONE`. A commit id is refused with an explanation:
    /// fetching an arbitrary object by id needs the server's permission, and most
    /// servers do not give it.
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub subdirectory: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

impl std::fmt::Debug for SourceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceRequest")
            .field("kind", &self.kind)
            .field("url", &self.url)
            .field("git_ref", &self.git_ref)
            .field("subdirectory", &self.subdirectory)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Turn the form's strings into a source specification.
///
/// The only validation here is "did the form fill in the fields this kind
/// needs". Whether the URL is one the application may fetch is
/// `file-manager`'s answer, not this function's — it is asked later, in the one
/// place that opens connections.
fn source_spec_from(request: Option<SourceRequest>) -> CommandResult<SourceSpec> {
    let Some(request) = request else {
        return Ok(SourceSpec::Empty);
    };

    let trimmed = |value: Option<String>| -> Option<String> {
        value
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };

    let url = trimmed(request.url);
    let token = trimmed(request.token).map(Secret::new);

    match request.kind.as_str() {
        "EMPTY" => Ok(SourceSpec::Empty),
        "GIT_CLONE" => Ok(SourceSpec::Git {
            url: url.ok_or_else(|| CommandError {
                message: "A repository address is needed to clone from.".to_string(),
            })?,
            git_ref: trimmed(request.git_ref),
            subdirectory: trimmed(request.subdirectory),
            token,
        }),
        "REMOTE_ARCHIVE" => Ok(SourceSpec::Archive {
            url: url.ok_or_else(|| CommandError {
                message: "An archive address is needed to download from.".to_string(),
            })?,
            token,
        }),
        "GITHUB_CLI" => Ok(SourceSpec::GitHubCli {
            // The same field the other kinds use for a URL; here it holds
            // `owner/repo`, which `github_cli` parses and validates.
            repo: url.ok_or_else(|| CommandError {
                message: "A repository name like `owner/repo` is needed.".to_string(),
            })?,
            git_ref: trimmed(request.git_ref),
            subdirectory: trimmed(request.subdirectory),
        }),
        other => Err(CommandError {
            message: format!("`{other}` is not a source this build offers."),
        }),
    }
}

/// Whether the GitHub CLI can be used, and as whom.
///
/// Asked before the option is offered, so a user without `gh` is told to install
/// it or paste a token rather than meeting a failure at Create.
#[derive(Debug, Serialize)]
pub struct GitHubCliStatus {
    pub installed: bool,
    /// The logged-in account, when there is one. `None` with `installed: true`
    /// means `gh` is there but nobody is logged in.
    pub account: Option<String>,
    /// What to tell the user, when something needs doing.
    pub hint: Option<String>,
}

#[tauri::command]
async fn github_cli_status() -> CommandResult<GitHubCliStatus> {
    use project_host_file_manager::github_cli::{self, GhCommand};

    // Spawning a process is blocking work, and this is called while the dialog is
    // being drawn.
    let status = tokio::task::spawn_blocking(|| {
        if !github_cli::is_available(&GhCommand) {
            return GitHubCliStatus {
                installed: false,
                account: None,
                hint: Some(
                    "The GitHub CLI (`gh`) is not installed, or not on the PATH. Install it,                      or use `Git repository` with a token instead."
                        .to_string(),
                ),
            };
        }

        match github_cli::logged_in_user(&GhCommand) {
            Ok(account) => GitHubCliStatus {
                installed: true,
                account,
                hint: None,
            },
            Err(_) => GitHubCliStatus {
                installed: true,
                account: None,
                hint: Some("`gh` is installed but nobody is logged in. Run `gh auth login`.".to_string()),
            },
        }
    })
    .await
    .map_err(|_| CommandError {
        message: "Checking the GitHub CLI did not finish.".to_string(),
    })?;

    Ok(status)
}

/// The runtimes this build can plan for, for the override list.
///
/// Served from the same table the planner uses, so the interface cannot offer a
/// choice that fails on Create.
#[derive(Debug, Serialize)]
pub struct RuntimeOption {
    pub id: String,
    pub label: String,
}

#[tauri::command]
fn supported_runtimes() -> Vec<RuntimeOption> {
    project_host_core::runtime_plan::supported_runtimes()
        .into_iter()
        .map(|(id, label)| RuntimeOption {
            id: id.to_string(),
            label: label.to_string(),
        })
        .collect()
}

/// What a created project's runtime turned out to be.
///
/// Returned alongside the project because the interesting half of "create" is now
/// an answer rather than an echo: the user did not say `GO`, the files did.
#[derive(Debug, Serialize)]
pub struct CreatedProject {
    #[serde(flatten)]
    pub project: ProjectSummary,
    /// The runtime wire value that was stored.
    pub runtime: String,
    /// True when it came from the files rather than from the user.
    pub detected: bool,
    /// Every language found in the tree, for display.
    pub languages: Vec<String>,
    /// Detection warnings, in the words the user should read.
    pub notes: Vec<String>,
}

/// Choose the runtime for a project whose files are already in place.
///
/// An empty project cannot be detected — there is nothing to look at — so it
/// needs an explicit choice, and saying so beats reporting "no language found"
/// for a directory that was deliberately created empty.
fn plan_runtime(
    directory: &std::path::Path,
    source: &SourceSpec,
    named: Option<&str>,
) -> CommandResult<project_host_core::RuntimePlan> {
    let empty_source = matches!(source, SourceSpec::Empty);

    match (named, empty_source) {
        (Some(runtime), _) if empty_source => {
            project_host_core::plan_named(runtime).map_err(CommandError::from)
        }
        (None, true) => Err(CommandError {
            message: "An empty project has no files to inspect, so it needs a \
                      runtime. Choose one, or start from a repository instead."
                .to_string(),
        }),
        (named, false) => project_host_core::plan_detected(directory, named).map_err(|error| {
            // Detection's message names every marker file it looked for, which is
            // the most useful thing a user can be told here.
            CommandError {
                message: error.to_string(),
            }
        }),
        // Unreachable: covered by the first arm.
        (Some(runtime), _) => project_host_core::plan_named(runtime).map_err(CommandError::from),
    }
}

/// Create a project.
///
/// The user supplies a display name and nothing else that becomes an
/// identifier. The slug, directory, container, network and volume names are all
/// derived from a generated id — see `project-manager`'s `names` module for why
/// a user-provided name must never become a path or a container name.
#[tauri::command]
async fn create_project(
    state: tauri::State<'_, AppState>,
    request: NewProjectRequest,
) -> CommandResult<CreatedProject> {
    let app: &AppState = &state;

    let display_name = sanitise_display_name(request.display_name.trim());
    if display_name.is_empty() {
        return Err(CommandError {
            message: "A project needs a name.".to_string(),
        });
    }
    if display_name.chars().count() > 60 {
        return Err(CommandError {
            message: "That name is too long — 60 characters at most.".to_string(),
        });
    }

    let count = projects::list_projects(app.database(), true, None, 1000)
        .await?
        .len();
    let limit = app.config().max_projects as usize;
    if count >= limit {
        return Err(CommandError {
            message: format!("You have reached the limit of {limit} projects."),
        });
    }

    // The id comes first: everything that names anything is derived from it.
    let id = project_host_api_types::ids::ProjectId::generate();
    let slug = Slug::from_project_id(id.as_str());

    let taken: std::collections::BTreeSet<u16> = projects::allocated_host_ports(app.database())
        .await?
        .into_iter()
        .collect();
    let pool = PortPool::new(app.config().port_pool_start, app.config().port_pool_end);
    let host_port = pool.allocate(&taken).map_err(CommandError::from)?;

    let projects_root = app
        .config()
        .projects_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("projects"));
    let directory = projects_root.join(slug.as_str());

    // Staging lives *inside* the projects directory rather than in the
    // application's temp directory. Promotion is a rename, a rename is atomic
    // only within one filesystem, and a user who pointed `projects_dir` at
    // another volume would otherwise get a cross-device failure at the last step
    // of every fetch.
    let staging_root = projects_root.join(".staging");
    std::fs::create_dir_all(&staging_root).map_err(CommandError::from)?;

    let spec = source_spec_from(request.source)?;

    // The files come first. A fetch that fails leaves nothing on disk and
    // nothing in the database, because the row below has not been written yet;
    // the other order would leave a project the user can see and cannot use
    // every time a remote is unreachable.
    let outcome = project_host_core::provisioning::materialise_source(
        &spec,
        &directory,
        &staging_root,
        id.as_str(),
    )
    .await?;

    // The runtime is decided *after* the files are there, because deciding it
    // before means asking the user a question the files can answer. An empty
    // project has no files, so there a choice is required and detection would
    // only report that it found nothing.
    let plan = match plan_runtime(&directory, &spec, request.runtime.as_deref()) {
        Ok(plan) => plan,
        Err(error) => {
            // Nothing is in the database yet, but the files are on disk. They go,
            // so a retry with a corrected runtime starts from a clean directory.
            project_host_core::provisioning::discard_directory(&directory);
            return Err(error);
        }
    };

    // Sized to what this machine measured at startup rather than to three
    // constants. A user who later sets a limit deliberately keeps it: only the
    // starting values come from here.
    let defaults = app.resource_defaults();

    let record = match projects::create_project(
        app.database(),
        &projects::NewProject {
            slug: slug.to_string(),
            display_name: display_name.clone(),
            description: sanitise_display_name(request.description.trim()),
            project_type: plan.project_type,
            icon: None,
            color: None,
            source_type: spec.source_type().to_string(),
            directory: directory.display().to_string(),
            source_url: outcome.source_url.clone(),
            source_ref: outcome.source_ref.clone(),
            source_commit: outcome.source_commit.clone(),
            autostart: false,
            restart_policy: "UNLESS_STOPPED".to_string(),
            network_mode: "INTERNET".to_string(),
            memory_limit_mb: defaults.memory_limit_mb,
            cpu_limit_cores: defaults.cpu_limit_cores,
            // Not part of ResourceDefaults: no probe here measures what a
            // project will store, so tiering it would be a guess dressed as a
            // measurement.
            storage_limit_mb: 2048,
            process_limit: defaults.process_limit,
            runtime: plan.spec.clone(),
            processes: plan.processes.clone(),
            ports: vec![projects::NewPort {
                port: plan.port,
                host_port: Some(i64::from(host_port)),
                protocol: "tcp".to_string(),
                bind_address: "127.0.0.1".to_string(),
                is_primary: true,
                // Linked once the process rows exist: `create_project` writes
                // the processes and the ports in one transaction, and the
                // process ids are minted inside it.
                process_id: None,
            }],
        },
    )
    .await
    {
        Ok(record) => record,
        Err(error) => {
            // The files are already there. Leaving them would make the slug's
            // directory occupied for a project that does not exist, and the next
            // attempt with the same id would refuse to write into it.
            project_host_core::provisioning::discard_directory(&directory);
            return Err(CommandError::from(error));
        }
    };

    // No key is held at runtime yet, so this stores nothing and says so. See
    // `provisioning`'s module documentation.
    let stored = project_host_core::provisioning::store_source_token(
        app.database(),
        None,
        &record.id,
        &spec,
    )
    .await?;

    if stored.token_used_but_not_stored {
        tracing::info!(
            project = %record.id,
            "the access token was used for the fetch and not stored: no key store exists yet"
        );
    }

    Ok(CreatedProject {
        project: ProjectSummary {
            id: record.id,
            slug: record.slug,
            display_name: record.display_name,
            description: record.description,
            project_type: record.project_type,
            status: record.status,
            desired_state: record.desired_state,
            color: record.color,
        },
        runtime: plan.spec.runtime,
        detected: plan.detected,
        languages: plan.languages,
        notes: plan.notes,
    })
}

/// Start a project.
///
/// Long-running: building an image the first time takes minutes. The window
/// keeps the button in a pending state rather than this pretending to be fast.
/// `force` skips the memory check. Sent only by the user answering a refusal
/// that stated the numbers — never a stored preference, and absent means false.
#[tauri::command]
async fn start_project(
    state: tauri::State<'_, AppState>,
    project_id: String,
    force: Option<bool>,
) -> CommandResult<String> {
    let app: &AppState = &state;
    project_host_core::lifecycle::start_forcing(app, &project_id, force.unwrap_or(false))
        .await
        .map_err(CommandError::from)
}

/// One command the user is being asked to allow, before anything runs.
#[derive(Debug, Serialize)]
pub struct ToolchainStepDto {
    pub describes: String,
    pub elevated: bool,
}

/// What pressing Start should do about the project's language.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ToolchainReadinessDto {
    /// Nothing is missing. Start normally.
    Ready,
    /// Show these steps and start only if the user agrees.
    NeedsInstall {
        display_name: String,
        steps: Vec<ToolchainStepDto>,
        /// Whether any step raises an elevation prompt, so the window can say
        /// so before the prompt appears rather than after.
        needs_elevation: bool,
    },
    /// Nothing can be offered. `fixable` decides whether a retry is shown.
    Blocked { message: String, fixable: bool },
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolchainProgress {
    pub step: usize,
    pub of: usize,
    pub describes: String,
}

impl ToolchainProgress {
    const EVENT: &'static str = "toolchain://progress";
}

/// Read the project's runtime and work out what this machine is missing.
async fn readiness_for(
    app: &AppState,
    project_id: &str,
) -> CommandResult<(String, project_host_core::Readiness)> {
    let runtime = project_host_database::projects::find_runtime(app.database(), project_id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError {
            message: "This project has no runtime recorded.".to_string(),
        })?;

    // Full scan: toolchain install needs WSL and firmware virtualization, which
    // the launch snapshot deliberately skipped. Bounded subprocesses, so a
    // wedged WSL delays Start rather than the window.
    let snapshot =
        tokio::task::spawn_blocking(|| project_host_platform::probe::SystemScanner.snapshot_full())
            .await
            .map_err(|error| CommandError {
                message: format!("the machine scan did not finish: {error}"),
            })?;
    let host = project_host_core::host_from_snapshot(
        &snapshot,
        project_host_core::toolchain_flow::winget_present(),
    );

    // The project's install command, from whichever process declares one. A
    // toolchain is a property of the project, so any process needing an
    // install is enough to make the toolchain required.
    let processes = projects::list_processes(app.database(), project_id).await?;
    let install = project_host_core::toolchain_flow::project_install_for(
        processes
            .iter()
            .find_map(|process| process.install_command.as_deref()),
    );

    let readiness = project_host_core::assess(
        &runtime.runtime,
        &host,
        &project_host_core::MachineResolver,
        install.as_ref(),
    );

    Ok((runtime.runtime, readiness))
}

/// Relaunch the application, so an installed update can take effect.
///
/// The updater writes the new files while this process is still running; the
/// new version is only on screen after a restart. Until this existed the
/// finished state could only *tell* the user to close and reopen, which is the
/// one step of the update it could not perform for them.
///
/// Never returns: `restart` replaces the process. Typed as returning
/// `CommandResult<()>` anyway so the front end can await it like any other
/// command and report a failure to even reach it.
#[tauri::command]
async fn restart_app(app: tauri::AppHandle) -> CommandResult<()> {
    tracing::info!("restarting to finish an update");
    app.restart();
}

/// Minimise the update window.
///
/// A long download should be able to get out of the way without being
/// cancelled — and cancelling is not offered, so this is the only way to put it
/// aside.
#[tauri::command]
async fn minimize_window(window: tauri::Window) -> CommandResult<()> {
    window.minimize().map_err(|error| CommandError {
        message: format!("The window could not be minimised: {error}"),
    })
}

/// What the machine is missing for this project, without changing anything.
#[tauri::command]
async fn toolchain_readiness(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> CommandResult<ToolchainReadinessDto> {
    let app: &AppState = &state;
    let (runtime, readiness) = readiness_for(app, &project_id).await?;

    Ok(match readiness {
        project_host_core::Readiness::Ready => ToolchainReadinessDto::Ready,

        project_host_core::Readiness::NeedsInstall { steps } => {
            ToolchainReadinessDto::NeedsInstall {
                display_name: project_host_toolchain::spec_for(&runtime)
                    .map(|spec| spec.display_name.to_string())
                    .unwrap_or(runtime),
                needs_elevation: steps.iter().any(|step| step.elevated),
                steps: steps
                    .iter()
                    .map(|step| ToolchainStepDto {
                        describes: step.describes.clone(),
                        elevated: step.elevated,
                    })
                    .collect(),
            }
        }

        project_host_core::Readiness::Blocked(blocker) => ToolchainReadinessDto::Blocked {
            message: blocker.to_string(),
            fixable: blocker.is_fixable(),
        },
    })
}

/// Install what `toolchain_readiness` reported missing.
///
/// The plan is recomputed here rather than taken from the window: a step list
/// that arrived over the boundary is a command line chosen by the caller, and
/// this one is run elevated.
#[tauri::command]
async fn install_toolchain(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> CommandResult<()> {
    let app: &AppState = &state;
    let (runtime, readiness) = readiness_for(app, &project_id).await?;

    let steps = match readiness {
        project_host_core::Readiness::Ready => return Ok(()),
        project_host_core::Readiness::Blocked(blocker) => {
            return Err(CommandError {
                message: blocker.to_string(),
            })
        }
        project_host_core::Readiness::NeedsInstall { steps } => steps,
    };

    let snapshot =
        tokio::task::spawn_blocking(|| project_host_platform::probe::SystemScanner.snapshot_full())
            .await
            .map_err(|error| CommandError {
                message: format!("the machine scan did not finish: {error}"),
            })?;
    let host = project_host_core::host_from_snapshot(
        &snapshot,
        project_host_core::toolchain_flow::winget_present(),
    );

    // Installing blocks on subprocesses that raise an elevation prompt and
    // wait for a person, which is far too long to hold the async runtime.
    tokio::task::spawn_blocking(move || {
        let mut report = |progress: project_host_core::toolchain_flow::Progress| {
            let payload = ToolchainProgress {
                step: progress.step,
                of: progress.of,
                describes: progress.describes,
            };
            if let Err(error) = app_handle.emit(ToolchainProgress::EVENT, payload) {
                tracing::warn!(%error, "could not report install progress to the window");
            }
        };

        project_host_core::toolchain_flow::install(&runtime, &steps, &host, &mut report)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("the install could not be started: {error}"),
    })?
    .map_err(|blocker| CommandError {
        message: blocker.to_string(),
    })
}

#[tauri::command]
async fn stop_project(state: tauri::State<'_, AppState>, project_id: String) -> CommandResult<()> {
    let app: &AppState = &state;
    project_host_core::lifecycle::stop(app, &project_id)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
async fn restart_project(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> CommandResult<String> {
    let app: &AppState = &state;
    project_host_core::lifecycle::restart(app, &project_id)
        .await
        .map_err(CommandError::from)
}

/// Set a project's run mode, and answer with the row as it now reads.
///
/// Refuses anything but the two words rather than letting the database's CHECK
/// refuse it: the message a user should see names the choices, not the
/// constraint.
/// What the machine is carrying, and what each running project costs.
///
/// Every project is measured: they are all processes of this application, so
/// each one has a pid and the reading covers its whole process tree. A project
/// with several processes is the sum of them.
#[derive(Debug, Serialize)]
pub struct MachineLoad {
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub reserve_bytes: u64,
    pub headroom_bytes: u64,
    pub cpu_percent: Option<f32>,
    pub logical_cores: u32,
    /// Whether anything has been sampled yet. Before the first tick the numbers
    /// are placeholders and the interface should say so rather than draw zeroes.
    pub measured: bool,
    pub running: Vec<RunningProjectDto>,
}

#[derive(Debug, Serialize)]
pub struct RunningProjectDto {
    pub project_id: String,
    pub display_name: String,
    pub memory_bytes: u64,
    pub cpu_percent: Option<f32>,
    /// Whether the figure is a reading rather than the project's declared
    /// limit. True whenever the project is running here.
    pub measured: bool,
    /// When the current run started, RFC 3339. The window turns this into an
    /// uptime rather than the core sending a number that is stale the moment
    /// it is serialised.
    pub started_at: Option<String>,
    /// The project's primary published port, if it has one.
    pub port: Option<i64>,
}

#[tauri::command]
async fn machine_load(state: tauri::State<'_, AppState>) -> CommandResult<MachineLoad> {
    let app: &AppState = &state;
    let machine = app.machine_usage().await;
    let reserve_bytes = app.reserve().bytes_for(machine.total_memory_bytes);

    // Uptime and port are read here rather than added to the governor's own
    // `RunningProject`: that struct feeds the admission decision, and a field
    // only the window needs has no business being part of what decides whether
    // a project is allowed to start. The extra reads are one per *running*
    // project, which is a handful, not one per project on the machine.
    let mut running = Vec::new();
    for project in project_host_core::lifecycle::running_projects(app).await {
        let record = projects::find_project(app.database(), &project.project_id)
            .await
            .ok()
            .flatten();

        let port = projects::list_ports(app.database(), &project.project_id)
            .await
            .ok()
            .and_then(|ports| {
                ports
                    .iter()
                    .find(|port| port.is_primary)
                    .or_else(|| ports.first())
                    .and_then(|port| port.host_port.or(Some(port.port)))
            });

        running.push(RunningProjectDto {
            project_id: project.project_id,
            display_name: project.display_name,
            // Every project is measured now: they are all processes of this
            // application, and a pid is a thing that can be read.
            measured: true,
            memory_bytes: project.usage.memory_bytes,
            cpu_percent: project.usage.cpu_percent,
            started_at: record.and_then(|record| record.started_at),
            port,
        });
    }

    Ok(MachineLoad {
        total_memory_bytes: machine.total_memory_bytes,
        available_memory_bytes: machine.available_memory_bytes,
        reserve_bytes,
        headroom_bytes: machine.available_memory_bytes.saturating_sub(reserve_bytes),
        cpu_percent: machine.cpu_percent,
        logical_cores: machine.logical_cores,
        measured: machine.is_known(),
        running,
    })
}

/// How many host projects would stop if the window were closed now.
///
/// Docker projects are not counted: they outlive this application. The number
/// exists so the quit dialog can say what quitting costs, rather than making
/// the user discover it afterwards.
#[tauri::command]
async fn host_projects_running(state: tauri::State<'_, AppState>) -> CommandResult<usize> {
    let app: &AppState = &state;
    Ok(project_host_core::lifecycle::host_projects_running(app).await)
}

#[tauri::command]
async fn kill_project(state: tauri::State<'_, AppState>, project_id: String) -> CommandResult<()> {
    let app: &AppState = &state;
    project_host_core::lifecycle::kill(app, &project_id)
        .await
        .map_err(CommandError::from)
}

// ---------------------------------------------------------------------- power

/// What the power manager is doing and why.
///
/// The reason is carried all the way to the window rather than being
/// reconstructed there. A profile shown without its sentence is the machine
/// behaving differently for reasons the interface cannot explain, which is the
/// thing this whole area exists to avoid.
#[derive(Debug, Serialize)]
pub struct PowerStatus {
    pub mode: String,
    pub profile: String,
    pub reason: String,
    /// What the policy asked for.
    pub prevent_sleep: bool,
    /// What actually happened. Not the same thing: a platform can refuse, and
    /// showing the request as though it were the outcome would be a lie the
    /// user only discovers when their machine sleeps.
    pub sleep_held: bool,
    /// False before the first tick, so the window can say "measuring" rather
    /// than draw zeroes as though they were readings.
    pub measured: bool,
    pub cpu_percent: Option<f32>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub hottest_celsius: Option<f32>,
    pub hottest_sensor: Option<String>,
    pub battery_percent: Option<f32>,
    pub charging: Option<bool>,
    pub power_source: String,
    pub active_projects: usize,
    pub warnings: Vec<PowerWarningDto>,
}

#[derive(Debug, Serialize)]
pub struct PowerWarningDto {
    /// `thermal`, `low_battery` or `memory`.
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct PowerJournalPage {
    pub entries: Vec<PowerJournalEntryDto>,
    /// Pass back as `since` next time.
    pub cursor: u64,
}

#[derive(Debug, Serialize)]
pub struct PowerJournalEntryDto {
    pub seq: u64,
    /// RFC 3339, formatted here rather than in the power crate — which stores
    /// epoch seconds precisely so it need not depend on the product's
    /// timestamp format.
    pub at: String,
    pub summary: String,
    pub reason: String,
}

fn power_warning_dto(warning: &project_host_power::policy::Warning) -> PowerWarningDto {
    use project_host_power::policy::Warning;
    match warning {
        Warning::Thermal { message, .. } => PowerWarningDto {
            kind: "thermal".to_string(),
            message: message.clone(),
        },
        Warning::LowBattery { message, .. } => PowerWarningDto {
            kind: "low_battery".to_string(),
            message: message.clone(),
        },
        Warning::Memory { message, .. } => PowerWarningDto {
            kind: "memory".to_string(),
            message: message.clone(),
        },
    }
}

/// A one-line description of a journal event.
fn power_event_summary(event: &project_host_power::journal::Event) -> String {
    use project_host_power::journal::Event;
    match event {
        Event::ProfileChanged { from, to } => {
            format!(
                "Power profile changed from {} to {}",
                from.as_str(),
                to.as_str()
            )
        }
        Event::WarningRaised { .. } => "A warning started applying".to_string(),
        Event::WarningCleared { .. } => "A warning stopped applying".to_string(),
        Event::SleepHoldChanged { held: true } => "Sleep is being held off".to_string(),
        Event::SleepHoldChanged { held: false } => "Sleep is no longer being held off".to_string(),
    }
}

#[tauri::command]
async fn power_status(state: tauri::State<'_, AppState>) -> CommandResult<PowerStatus> {
    let app: &AppState = &state;
    let manager = app.power().read().await;

    // Before the first tick there is nothing measured. Reported as such rather
    // than by ticking here: a render must never be the thing that changes the
    // machine's power behaviour.
    let Some(snapshot) = manager.latest() else {
        return Ok(PowerStatus {
            mode: manager.mode().as_str().to_string(),
            profile: project_host_power::Profile::Balanced.as_str().to_string(),
            reason: "The machine has not been measured yet.".to_string(),
            prevent_sleep: false,
            sleep_held: false,
            measured: false,
            cpu_percent: None,
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            hottest_celsius: None,
            hottest_sensor: None,
            battery_percent: None,
            charging: None,
            power_source: "unknown".to_string(),
            active_projects: 0,
            warnings: Vec::new(),
        });
    };

    Ok(PowerStatus {
        mode: snapshot.mode.as_str().to_string(),
        profile: snapshot.profile.as_str().to_string(),
        reason: snapshot.reason.clone(),
        prevent_sleep: snapshot.prevent_sleep,
        sleep_held: snapshot.sleep_held,
        measured: snapshot.sample.is_measured(),
        cpu_percent: snapshot.sample.cpu_percent,
        memory_used_bytes: snapshot.sample.memory_used_bytes,
        memory_total_bytes: snapshot.sample.memory_total_bytes,
        hottest_celsius: snapshot.sample.hottest_celsius,
        hottest_sensor: snapshot.sample.hottest_sensor.clone(),
        battery_percent: snapshot.sample.battery_percent,
        charging: snapshot.sample.charging,
        power_source: match snapshot.sample.power_source {
            project_host_power::policy::PowerSource::Ac => "ac",
            project_host_power::policy::PowerSource::Battery => "battery",
            project_host_power::policy::PowerSource::Unknown => "unknown",
        }
        .to_string(),
        active_projects: snapshot.active_projects,
        warnings: snapshot.warnings.iter().map(power_warning_dto).collect(),
    })
}

/// Choose how power is managed.
///
/// An unknown word is rejected rather than quietly treated as automatic: a
/// window sending one has a bug, and silently running a different policy than
/// the one it thinks it set is the hardest kind to find.
#[tauri::command]
async fn set_power_mode(state: tauri::State<'_, AppState>, mode: String) -> CommandResult<()> {
    let app: &AppState = &state;
    let parsed = project_host_power::Mode::parse(&mode)
        .ok_or_else(|| CommandError::from(format!("`{mode}` is not a power mode")))?;

    app.power().write().await.set_mode(parsed);
    tracing::info!(mode = parsed.as_str(), "power mode changed");
    Ok(())
}

#[tauri::command]
async fn power_journal(
    state: tauri::State<'_, AppState>,
    since: u64,
) -> CommandResult<PowerJournalPage> {
    let app: &AppState = &state;
    let (entries, cursor) = app.power().read().await.journal_since(since);

    Ok(PowerJournalPage {
        entries: entries
            .into_iter()
            .map(|entry| PowerJournalEntryDto {
                seq: entry.seq,
                at: project_host_database::time::format_unix_seconds(entry.at as i64),
                summary: power_event_summary(&entry.event),
                reason: entry.reason,
            })
            .collect(),
        cursor,
    })
}

/// Set a project's scheduling priority and whether it holds sleep off.
///
/// Both optional, so the window can change one without having to send the
/// other and risk overwriting a setting it had not refreshed.
#[tauri::command]
async fn set_project_power(
    state: tauri::State<'_, AppState>,
    project_id: String,
    priority: Option<String>,
    keep_awake: Option<bool>,
) -> CommandResult<()> {
    let app: &AppState = &state;

    if let Some(word) = &priority {
        let upper = word.to_ascii_uppercase();
        if !matches!(upper.as_str(), "LOW" | "NORMAL" | "HIGH") {
            return Err(CommandError::from(format!("`{word}` is not a priority")));
        }
    }

    let update = projects::ProjectUpdate {
        priority: priority.map(|word| word.to_ascii_uppercase()),
        keep_awake,
        ..Default::default()
    };

    projects::update_project(app.database(), &project_id, &update).await?;
    Ok(())
}

// ------------------------------------------------------------------- console

/// One project's output.
#[derive(Debug, Serialize)]
pub struct ProjectConsole {
    pub lines: Vec<ConsoleLineDto>,
    /// Pass back as `since` on the next poll. Zero for a file-backed console,
    /// which has no cursor to advance.
    pub cursor: u64,
    /// Whether this came from a running supervisor. `false` means the project
    /// is not running and this is the record of when it was — so a window can
    /// offer "follow" only where following means anything.
    pub live: bool,
}

#[derive(Debug, Serialize)]
pub struct ConsoleLineDto {
    pub seq: u64,
    pub at: String,
    /// `stdout`, `stderr` or `system`.
    pub stream: String,
    pub text: String,
}

/// Read a project's console.
///
/// `since` is the cursor from the previous call, or zero to start. Ignored for
/// a project that is not running: a file cannot answer it.
#[tauri::command]
async fn project_console(
    state: tauri::State<'_, AppState>,
    project_id: String,
    since: u64,
) -> CommandResult<ProjectConsole> {
    let app: &AppState = &state;
    let console = project_host_core::logs::read(app, &project_id, since).await;

    Ok(ProjectConsole {
        lines: console
            .lines
            .into_iter()
            .map(|line| ConsoleLineDto {
                seq: line.seq,
                at: line.at,
                stream: line.stream.as_str().to_string(),
                text: line.text,
            })
            .collect(),
        cursor: console.cursor,
        live: console.live,
    })
}

// ----------------------------------------------------------- project processes

/// Every process of a project, in start order, with the port each listens on.
#[tauri::command]
async fn list_project_processes(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> CommandResult<Vec<ProcessSummary>> {
    let app: &AppState = &state;
    let processes = projects::list_processes(app.database(), &project_id).await?;
    let ports = projects::list_ports(app.database(), &project_id).await?;

    Ok(processes
        .into_iter()
        .map(|process| {
            let port = ports
                .iter()
                .find(|port| port.process_id.as_deref() == Some(process.id.as_str()))
                .and_then(|port| port.host_port);

            ProcessSummary {
                id: process.id,
                name: process.name,
                start_order: process.start_order,
                command: process.command,
                working_dir: process.working_dir,
                install_command: process.install_command,
                build_command: process.build_command,
                status: process.status,
                port,
                exit_code: process.exit_code,
                failure_reason: process.failure_reason,
                restart_count: process.restart_count,
            }
        })
        .collect())
}

/// Restart one process, leaving its siblings alone.
///
/// Deliberately not "restart the project": the user pointed at one process,
/// and the others keep their pids. A project-wide restart is `restart_project`.
#[tauri::command]
async fn restart_project_process(
    state: tauri::State<'_, AppState>,
    project_id: String,
    process_name: String,
) -> CommandResult<()> {
    let app: &AppState = &state;
    let record = projects::find_project(app.database(), &project_id)
        .await?
        .ok_or_else(|| CommandError {
            message: "No project with that id.".to_string(),
        })?;

    project_host_core::lifecycle::orchestrator_for(app)
        .restart_process(&record, &process_name)
        .await?;
    Ok(())
}

// -------------------------------------------------------------- project files

/// One row in a directory listing, as the window reads it.
#[derive(Debug, Serialize)]
pub struct FileEntryDto {
    pub name: String,
    pub path: String,
    /// `file`, `directory` or `other`.
    pub kind: String,
    pub size_bytes: u64,
    pub modified_unix_ms: Option<i64>,
    pub is_symlink: bool,
}

impl From<project_host_file_manager::FileEntry> for FileEntryDto {
    fn from(entry: project_host_file_manager::FileEntry) -> Self {
        Self {
            name: entry.name,
            path: entry.path,
            kind: entry.kind.as_str().to_string(),
            size_bytes: entry.size_bytes,
            modified_unix_ms: entry.modified_unix_ms,
            is_symlink: entry.is_symlink,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ListingDto {
    pub path: String,
    pub entries: Vec<FileEntryDto>,
    /// True when there were more entries than the limit. The window says so
    /// rather than showing a partial listing as if it were complete.
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct TextFileDto {
    pub path: String,
    pub text: String,
    pub size_bytes: u64,
    pub language: String,
    /// True when the project is mid-build or mid-deletion. The editor opens the
    /// file read-only rather than letting a save race the operation.
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileImportProgressDto {
    pub import_id: String,
    pub project_id: String,
    pub copied_bytes: u64,
    pub total_bytes: u64,
    pub copied_files: u64,
    pub total_files: u64,
    pub current_path: String,
}

impl FileImportProgressDto {
    const EVENT: &'static str = "project-files://import-progress";
}

/// Statuses during which a project's files must not be written.
///
/// `BUILDING` copies the tree into an image; `DELETING` is removing it. A save
/// landing in either window either vanishes or corrupts what is being read.
fn is_read_only_status(status: &str) -> bool {
    matches!(status, "BUILDING" | "DELETING")
}

/// A project's root directory, from its stored record.
///
/// Every file command starts here, and every one of them passes a *relative*
/// request string alongside it. The window cannot express an absolute path,
/// which is the invariant `file-manager` was built around: `SafePath` is
/// constructed on this side of the bridge, from a root the database supplied and
/// a string that is only ever a suffix.
async fn project_root(
    app: &AppState,
    project_id: &str,
) -> CommandResult<(std::path::PathBuf, bool)> {
    let record = projects::find_project(app.database(), project_id)
        .await?
        .ok_or_else(|| CommandError {
            message: "That project no longer exists.".to_string(),
        })?;
    let root = std::path::PathBuf::from(record.directory);

    // A project row written before creation materialised any files has a
    // directory that does not exist yet. The row owns that path — it is `UNIQUE`
    // in the schema and derived from the project's own id — so creating it is
    // what an empty project should have looked like all along, and is better
    // than greeting the user with a path error they can do nothing about.
    if !root.exists() {
        std::fs::create_dir_all(&root).map_err(CommandError::from)?;
    }

    Ok((root, is_read_only_status(&record.status)))
}

fn file_limits(app: &AppState) -> project_host_file_manager::FileLimits {
    project_host_file_manager::FileLimits {
        max_upload_bytes: app.config().max_upload_bytes,
        ..project_host_file_manager::FileLimits::default()
    }
}

#[tauri::command]
async fn list_project_files(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
) -> CommandResult<ListingDto> {
    let app: &AppState = &state;
    let (root, _) = project_root(app, &project_id).await?;
    let listing =
        project_host_file_manager::operations::list_directory(&root, &path, &file_limits(app))
            .map_err(CommandError::from)?;

    Ok(ListingDto {
        path: listing.path,
        entries: listing
            .entries
            .into_iter()
            .map(FileEntryDto::from)
            .collect(),
        truncated: listing.truncated,
    })
}

#[tauri::command]
async fn read_project_file(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
) -> CommandResult<TextFileDto> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    let file =
        project_host_file_manager::operations::read_text_file(&root, &path, &file_limits(app))
            .map_err(CommandError::from)?;

    Ok(TextFileDto {
        path: file.path,
        text: file.text,
        size_bytes: file.size_bytes,
        language: file.language.to_string(),
        read_only,
    })
}

#[tauri::command]
async fn write_project_file(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    text: String,
) -> CommandResult<FileEntryDto> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message: "This project is being built or removed; its files cannot be saved right now."
                .to_string(),
        });
    }

    project_host_file_manager::operations::write_text_file(&root, &path, &text, &file_limits(app))
        .map(FileEntryDto::from)
        .map_err(CommandError::from)
}

#[tauri::command]
async fn begin_project_file_upload(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    upload_id: String,
    total_size: u64,
) -> CommandResult<()> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    project_host_file_manager::operations::begin_upload(
        &root,
        &path,
        &upload_id,
        total_size,
        &file_limits(app),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
async fn append_project_file_upload(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    upload_id: String,
    offset: u64,
    bytes: Vec<u8>,
) -> CommandResult<u64> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    project_host_file_manager::operations::append_upload(
        &root,
        &path,
        &upload_id,
        offset,
        &bytes,
        &file_limits(app),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
async fn finish_project_file_upload(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    upload_id: String,
    total_size: u64,
) -> CommandResult<FileEntryDto> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    project_host_file_manager::operations::finish_upload(
        &root,
        &path,
        &upload_id,
        total_size,
        &file_limits(app),
    )
    .map(FileEntryDto::from)
    .map_err(CommandError::from)
}

#[tauri::command]
async fn cancel_project_file_upload(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    upload_id: String,
) -> CommandResult<()> {
    let app: &AppState = &state;
    let (root, _) = project_root(app, &project_id).await?;
    project_host_file_manager::operations::cancel_upload(&root, &path, &upload_id)
        .map_err(CommandError::from)
}

/// Copy paths from this machine into the project.
///
/// `unwrap_paths` is the subset of `source_paths` whose *contents* should be
/// imported rather than the folder itself — a project dropped into a project,
/// where keeping the folder would produce `MyProject/MyProject/package.json`.
/// Absent means none, which is what every caller other than a project drop
/// wants. `destination_names` maps an absolute source to the final name it
/// should land under — what a "keep both" resolution produces.
// A Tauri command's arguments *are* its request body: each one is a named
// field the window sends, so splitting them into a struct to satisfy a lint
// would only move the same eight names one level down.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn import_project_files(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    cancels: tauri::State<'_, FileImportCancels>,
    project_id: String,
    target_directory: String,
    source_paths: Vec<String>,
    unwrap_paths: Option<Vec<String>>,
    destination_names: Option<Vec<(String, String)>>,
    import_id: String,
) -> CommandResult<Vec<FileEntryDto>> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    let limits = file_limits(app);
    let unwrap: std::collections::HashSet<String> =
        unwrap_paths.unwrap_or_default().into_iter().collect();
    let names: std::collections::HashMap<String, String> =
        destination_names.unwrap_or_default().into_iter().collect();
    let sources: Vec<project_host_file_manager::ImportSource> = source_paths
        .into_iter()
        .map(|path| {
            if unwrap.contains(&path) {
                project_host_file_manager::ImportSource::unwrapped(path)
            } else if let Some(name) = names.get(&path) {
                project_host_file_manager::ImportSource::renamed(path, name.clone())
            } else {
                project_host_file_manager::ImportSource::nested(path)
            }
        })
        .collect();
    let cancels_for_work = cancels.inner().clone();
    // The guard releases both the running mark and any cancellation when it
    // drops. Nothing is cleared on the way *in*: an import id is generated per
    // attempt and never reused, so there is nothing stale to clear — and
    // clearing would throw away a cancellation that arrived between the window
    // queueing this import and the blocking copy asking whether it was
    // cancelled.
    let _running = cancels.inner().start(import_id.clone());
    let cancels_for_sweep = cancels_for_work.clone();
    let import_id_for_work = import_id.clone();
    let project_id_for_work = project_id.clone();
    let app_handle_for_work = app_handle.clone();
    let sweep_root = root.clone();

    let report = tokio::task::spawn_blocking(move || {
        // Clear up after any import this project never got to finish, now that
        // there is a blocking thread to do it on. Best effort: wreckage from a
        // previous run must not fail the import the user just asked for.
        match project_host_file_manager::operations::remove_abandoned_import_staging(
            &sweep_root,
            |id| cancels_for_sweep.is_running(id),
        ) {
            Ok(removed) if !removed.is_empty() => {
                tracing::info!(?removed, "removed staging left by an unfinished import");
            }
            Err(error) => tracing::warn!(%error, "could not sweep abandoned import staging"),
            Ok(_) => {}
        }

        project_host_file_manager::operations::import_local_sources(
            &root,
            &target_directory,
            &sources,
            &import_id_for_work,
            &limits,
            |event| {
                let progress = FileImportProgressDto {
                    import_id: import_id_for_work.clone(),
                    project_id: project_id_for_work.clone(),
                    copied_bytes: event.copied_bytes,
                    total_bytes: event.total_bytes,
                    copied_files: event.copied_files,
                    total_files: event.total_files,
                    current_path: event.current_path,
                };
                if let Err(error) = app_handle_for_work.emit(FileImportProgressDto::EVENT, progress)
                {
                    tracing::warn!(%error, "could not report file import progress to the window");
                }
            },
            || cancels_for_work.is_cancelled(&import_id_for_work),
        )
    })
    .await
    .map_err(CommandError::from)
    .and_then(|result| result.map_err(CommandError::from))?;

    Ok(report.entries.into_iter().map(FileEntryDto::from).collect())
}

#[tauri::command]
async fn cancel_project_file_import(
    cancels: tauri::State<'_, FileImportCancels>,
    import_id: String,
) -> CommandResult<()> {
    cancels.cancel(import_id);
    Ok(())
}

#[tauri::command]
async fn create_project_file(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    directory: bool,
) -> CommandResult<FileEntryDto> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    let result = if directory {
        project_host_file_manager::operations::create_directory(&root, &path)
    } else {
        project_host_file_manager::operations::create_file(&root, &path)
    };
    result.map(FileEntryDto::from).map_err(CommandError::from)
}

#[tauri::command]
async fn rename_project_file(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    new_name: String,
) -> CommandResult<FileEntryDto> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    project_host_file_manager::operations::rename(&root, &path, &new_name)
        .map(FileEntryDto::from)
        .map_err(CommandError::from)
}

// ----------------------------------------------------------- machine metrics

/// What this machine is doing right now.
///
/// Host-wide, not per project: measuring a container's own CPU and memory means
/// reading Docker's stats stream, which the manager does not do yet. These are
/// the real numbers for the machine the projects run on, which is the question
/// "is this box healthy?" actually asks.
#[derive(Debug, Serialize)]
pub struct SystemMetrics {
    /// Percent across all cores, 0–100.
    pub cpu_percent: f32,
    pub cpu_count: usize,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    /// The disk holding the projects directory, not every disk on the machine.
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_mount: String,
}

/// The sampler, kept between calls.
///
/// CPU percentage is a difference between two samples, so a fresh `System` per
/// call reports zero every time. Holding it as managed state is what makes the
/// number real.
#[derive(Default)]
struct Metrics(Arc<Mutex<Option<sysinfo::System>>>);

#[tauri::command]
async fn system_metrics(
    state: tauri::State<'_, AppState>,
    metrics: tauri::State<'_, Metrics>,
) -> CommandResult<SystemMetrics> {
    use project_host_platform::PathProvider;
    use sysinfo::{Disks, System};

    let app: &AppState = &state;
    let paths = project_host_core::resolve_paths(app.config()).map_err(CommandError::from)?;
    let projects_dir = paths.projects_dir().to_path_buf();

    let (cpu_percent, cpu_count, memory_used_bytes, memory_total_bytes) = {
        let mut guard = metrics.0.lock().map_err(|_| CommandError {
            message: "The metrics sampler is unavailable.".to_string(),
        })?;
        let system = guard.get_or_insert_with(System::new);
        // Twice, because a percentage needs a previous sample to compare
        // against. The first call after startup would otherwise read zero.
        system.refresh_cpu_usage();
        system.refresh_memory();
        (
            system.global_cpu_usage(),
            system.cpus().len(),
            system.used_memory(),
            system.total_memory(),
        )
    };

    // The disk the projects actually live on: the one whose mount point is the
    // longest prefix of the projects directory.
    let disks = Disks::new_with_refreshed_list();
    let best = disks
        .list()
        .iter()
        .filter(|disk| projects_dir.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .or_else(|| disks.list().first());

    let (disk_used_bytes, disk_total_bytes, disk_mount) = match best {
        Some(disk) => (
            disk.total_space().saturating_sub(disk.available_space()),
            disk.total_space(),
            disk.mount_point().to_string_lossy().into_owned(),
        ),
        None => (0, 0, String::new()),
    };

    Ok(SystemMetrics {
        cpu_percent,
        cpu_count,
        memory_used_bytes,
        memory_total_bytes,
        disk_used_bytes,
        disk_total_bytes,
        disk_mount,
    })
}

// -------------------------------------------------------------- activity log

/// One entry of the audit log, as the activity feed shows it.
#[derive(Debug, Serialize)]
pub struct ActivityEntry {
    pub id: String,
    pub occurred_at: String,
    pub action: String,
    pub result: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub target_label: Option<String>,
    pub error_code: Option<String>,
}

/// What has happened, newest first.
///
/// Read straight from the audit log the core already writes for every state
/// change. Nothing here is generated for the sake of having a feed — an empty
/// list means nothing has happened yet.
#[tauri::command]
async fn recent_activity(
    state: tauri::State<'_, AppState>,
    project_id: Option<String>,
    limit: u32,
) -> CommandResult<Vec<ActivityEntry>> {
    let app: &AppState = &state;
    let records = project_host_database::audit::list(
        app.database(),
        None,
        project_id.as_deref(),
        None,
        limit.clamp(1, 200),
    )
    .await?;

    Ok(records
        .into_iter()
        .map(|record| ActivityEntry {
            id: record.id,
            occurred_at: record.occurred_at,
            action: record.action,
            result: record.result,
            target_type: record.target_type,
            target_id: record.target_id,
            target_label: record.target_label,
            error_code: record.error_code,
        })
        .collect())
}

// ------------------------------------------------------------ project detail

#[derive(Debug, Serialize)]
pub struct RuntimeDetail {
    pub runtime: String,
    pub runtime_version: String,
    pub package_manager: String,
    pub entry_file: Option<String>,
}

/// One process of a project, as the window shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSummary {
    pub id: String,
    pub name: String,
    pub start_order: i64,
    pub command: String,
    pub working_dir: String,
    pub install_command: Option<String>,
    pub build_command: Option<String>,
    pub status: String,
    /// The port this process listens on, when it has one.
    pub port: Option<i64>,
    pub exit_code: Option<i64>,
    /// Why it stopped, when it did not stop cleanly.
    pub failure_reason: Option<String>,
    pub restart_count: i64,
}

#[derive(Debug, Serialize)]
pub struct PortMapping {
    pub port: i64,
    pub host_port: Option<i64>,
    pub protocol: String,
}

/// An environment variable, never its secret value.
///
/// Secrets are stored encrypted and there is no reason for the window to hold
/// the plaintext: the list shows which keys exist and whether each is a secret,
/// which is what the screen displays.
#[derive(Debug, Serialize)]
pub struct EnvVarSummary {
    pub key: String,
    pub is_secret: bool,
    pub restart_required: bool,
    /// Present only for a plain value.
    pub value: Option<String>,
}

/// Everything one project knows about itself.
#[derive(Debug, Serialize)]
pub struct ProjectDetail {
    pub id: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub project_type: String,
    pub status: String,
    pub desired_state: String,
    pub health: String,
    /// `LOW`, `NORMAL` or `HIGH`. Scheduling only — never a cap.
    pub priority: String,
    /// Whether this project running holds automatic sleep off.
    pub keep_awake: bool,
    pub restart_policy: String,
    pub network_mode: String,
    pub autostart: bool,
    pub directory: String,
    pub source_type: String,
    pub source_url: Option<String>,
    pub source_ref: Option<String>,
    pub source_commit: Option<String>,
    pub memory_limit_mb: i64,
    pub cpu_limit_cores: f64,
    pub storage_limit_mb: i64,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
    pub last_exit_code: Option<i64>,
    pub last_failure_at: Option<String>,
    pub last_failure_reason: Option<String>,
    pub restart_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub runtime: Option<RuntimeDetail>,
    pub ports: Vec<PortMapping>,
    pub env_vars: Vec<EnvVarSummary>,
}

#[tauri::command]
async fn project_details(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> CommandResult<ProjectDetail> {
    let app: &AppState = &state;
    let record = projects::find_project(app.database(), &project_id)
        .await?
        .ok_or_else(|| CommandError {
            message: "That project no longer exists.".to_string(),
        })?;

    let runtime = projects::find_runtime(app.database(), &project_id).await?;
    let ports = projects::list_ports(app.database(), &project_id).await?;
    let variables =
        project_host_database::environment::list_variables(app.database(), &project_id).await?;

    Ok(ProjectDetail {
        id: record.id,
        slug: record.slug,
        display_name: record.display_name,
        description: record.description,
        project_type: record.project_type,
        status: record.status,
        desired_state: record.desired_state,
        health: record.health,
        priority: record.priority,
        keep_awake: record.keep_awake,
        restart_policy: record.restart_policy,
        network_mode: record.network_mode,
        autostart: record.autostart,
        directory: record.directory,
        source_type: record.source_type,
        source_url: record.source_url,
        source_ref: record.source_ref,
        source_commit: record.source_commit,
        memory_limit_mb: record.memory_limit_mb,
        cpu_limit_cores: record.cpu_limit_cores,
        storage_limit_mb: record.storage_limit_mb,
        started_at: record.started_at,
        stopped_at: record.stopped_at,
        last_exit_code: record.last_exit_code,
        last_failure_at: record.last_failure_at,
        last_failure_reason: record.last_failure_reason,
        restart_count: record.restart_count,
        created_at: record.created_at,
        updated_at: record.updated_at,
        runtime: runtime.map(|runtime| RuntimeDetail {
            runtime: runtime.runtime,
            runtime_version: runtime.runtime_version,
            package_manager: runtime.package_manager,
            entry_file: runtime.entry_file,
        }),
        ports: ports
            .into_iter()
            .map(|port| PortMapping {
                port: port.port,
                host_port: port.host_port,
                protocol: port.protocol,
            })
            .collect(),
        env_vars: variables
            .into_iter()
            .map(|variable| {
                let is_secret = !matches!(
                    variable.value,
                    project_host_database::environment::StoredValue::Plain(_)
                );
                EnvVarSummary {
                    key: variable.key,
                    is_secret,
                    restart_required: variable.restart_required,
                    value: match variable.value {
                        project_host_database::environment::StoredValue::Plain(value) => {
                            Some(value)
                        }
                        // Deliberately not decrypted. The screen shows that a
                        // secret exists, never what it is.
                        _ => None,
                    },
                }
            })
            .collect(),
    })
}

#[derive(Debug, Serialize)]
pub struct DeploymentSummary {
    pub id: String,
    pub deployment_type: String,
    pub status: String,
    pub image_tag: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<i64>,
}

#[tauri::command]
async fn project_deployments(
    state: tauri::State<'_, AppState>,
    project_id: String,
    limit: u32,
) -> CommandResult<Vec<DeploymentSummary>> {
    let app: &AppState = &state;
    let records = projects::list_deployments(app.database(), &project_id, limit).await?;
    Ok(records
        .into_iter()
        .map(|record| DeploymentSummary {
            id: record.id,
            image_tag: record.image_tag,
            deployment_type: record.deployment_type,
            status: record.status,
            error_code: record.error_code,
            error_message: record.error_message,
            started_at: record.started_at,
            finished_at: record.finished_at,
            duration_ms: record.duration_ms,
        })
        .collect())
}

/// Starts, stops and crashes — the restart history the overview shows.
#[derive(Debug, Serialize)]
pub struct ContainerEvent {
    pub id: String,
    pub event_type: String,
    pub exit_code: Option<i64>,
    pub detail: Option<String>,
    pub occurred_at: String,
}

#[tauri::command]
async fn project_events(
    state: tauri::State<'_, AppState>,
    project_id: String,
    limit: u32,
) -> CommandResult<Vec<ContainerEvent>> {
    let app: &AppState = &state;
    let records = projects::list_container_events(app.database(), &project_id, limit).await?;
    Ok(records
        .into_iter()
        .map(|record| ContainerEvent {
            id: record.id,
            event_type: record.event_type,
            exit_code: record.exit_code,
            detail: record.detail,
            occurred_at: record.occurred_at,
        })
        .collect())
}

// ---------------------------------------------------------- import inspection

/// What one dropped path turned out to be.
///
/// The window cannot answer any of this itself: it is handed operating-system
/// paths by the drag-and-drop event and has no filesystem access at all. So the
/// core looks, and the window decides what to offer.
#[derive(Debug, Serialize)]
pub struct ImportCandidate {
    /// The absolute path, echoed back so the window can name it in the import.
    pub path: String,
    pub name: String,
    pub is_directory: bool,
    /// True when the evidence says this folder is a project in its own right.
    pub is_project: bool,
    /// The score behind that decision, so the interface can explain itself.
    pub score: u32,
    /// The markers found, e.g. `package.json`, `src/`.
    pub signals: Vec<String>,
    /// The top-level names inside, for the preview. Bounded: a folder with ten
    /// thousand entries would otherwise be sent through the bridge to draw a
    /// list nobody reads.
    pub children: Vec<String>,
    pub child_count: usize,
    /// `Node.js`, `Rust`, `Tauri`… absent when nothing identified it.
    pub ecosystem: Option<String>,
    /// True when this folder holds several packages rather than being one.
    pub is_monorepo: bool,
    /// Projects found *inside* this one.
    pub nested: Vec<NestedProject>,
}

/// A project found inside another.
///
/// Whether it deserves to be treated separately depends on why it is there. A
/// package under a monorepo's `packages/` belongs to its parent and must not be
/// split out; two unrelated projects someone dropped into one folder are
/// independent and should be.
#[derive(Debug, Serialize)]
pub struct NestedProject {
    pub path: String,
    /// Relative to the folder it was found in, so the preview can show a tree.
    pub relative: String,
    pub name: String,
    pub ecosystem: Option<String>,
    pub score: u32,
    /// True when the parent is a workspace and this sits in a workspace folder.
    pub belongs_to_workspace: bool,
}

/// How deep the search for nested projects goes.
///
/// Two levels finds `packages/api` and `apps/web` without walking a whole
/// `node_modules`, which is the only thing at that depth worth avoiding.
const NESTED_SEARCH_DEPTH: usize = 2;

/// Look for projects inside a folder.
///
/// Generated directories are skipped: every `node_modules` contains hundreds of
/// folders with a `package.json`, and reporting them as nested projects would
/// bury the real answer.
fn find_nested_projects(
    root: &std::path::Path,
    current: &std::path::Path,
    parent_is_monorepo: bool,
    depth: usize,
    found: &mut Vec<NestedProject>,
) {
    if depth > NESTED_SEARCH_DEPTH || found.len() >= 24 {
        return;
    }

    let Ok(listing) = std::fs::read_dir(current) else {
        return;
    };

    for item in listing.flatten() {
        if !item.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        let lower = name.to_lowercase();
        if project_host_file_manager::project_detect::GENERATED_DIRECTORIES
            .contains(&lower.as_str())
            || lower.starts_with('.')
        {
            continue;
        }

        let path = item.path();
        let detection = project_host_file_manager::detect_directory(&path);
        if detection.is_project {
            let relative = path
                .strip_prefix(root)
                .map(|value| value.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| name.clone());
            // A package sitting in a workspace folder of a monorepo belongs to
            // its parent. Splitting it out would break the workspace.
            let in_workspace_folder = current
                .file_name()
                .map(|folder| {
                    project_host_file_manager::project_detect::WORKSPACE_DIRECTORIES
                        .contains(&folder.to_string_lossy().to_lowercase().as_str())
                })
                .unwrap_or(false);

            found.push(NestedProject {
                path: path.to_string_lossy().into_owned(),
                relative,
                name,
                ecosystem: detection.ecosystem,
                score: detection.score,
                belongs_to_workspace: parent_is_monorepo && in_workspace_folder,
            });
            // Not descended into: a project inside a project inside a project is
            // detail the preview cannot use.
            continue;
        }

        find_nested_projects(root, &path, parent_is_monorepo, depth + 1, found);
    }
}

/// The cap on names returned per folder. Enough to recognise a project by.
const IMPORT_PREVIEW_CHILDREN: usize = 24;

/// Look at what is being dropped, without copying anything.
#[tauri::command]
async fn inspect_import_paths(source_paths: Vec<String>) -> CommandResult<Vec<ImportCandidate>> {
    let mut candidates = Vec::new();

    for path in source_paths {
        let source = std::path::PathBuf::from(&path);
        let Ok(metadata) = std::fs::symlink_metadata(&source) else {
            continue;
        };
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());

        if !metadata.is_dir() || metadata.is_symlink() {
            candidates.push(ImportCandidate {
                path,
                name,
                is_directory: false,
                is_project: false,
                score: 0,
                signals: Vec::new(),
                children: Vec::new(),
                child_count: 0,
                ecosystem: None,
                is_monorepo: false,
                nested: Vec::new(),
            });
            continue;
        }

        let detection = project_host_file_manager::detect_directory(&source);
        let mut children = Vec::new();
        let mut child_count = 0usize;
        if let Ok(listing) = std::fs::read_dir(&source) {
            for item in listing.flatten() {
                child_count += 1;
                if children.len() < IMPORT_PREVIEW_CHILDREN {
                    let mut label = item.file_name().to_string_lossy().into_owned();
                    if item.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                        label.push('/');
                    }
                    children.push(label);
                }
            }
        }
        children.sort();

        let mut nested = Vec::new();
        find_nested_projects(&source, &source, detection.is_monorepo, 1, &mut nested);

        candidates.push(ImportCandidate {
            path,
            name,
            is_directory: true,
            is_project: detection.is_project,
            score: detection.score,
            signals: detection.signals,
            children,
            child_count,
            ecosystem: detection.ecosystem,
            is_monorepo: detection.is_monorepo,
            nested,
        });
    }

    Ok(candidates)
}

/// One thing an organised import would create.
#[derive(Debug, Serialize)]
pub struct PlannedDestinationDto {
    /// The absolute path it comes from — a child, when a folder is unwrapped.
    pub source: String,
    /// Where it lands, relative to the project root.
    pub relative: String,
    pub is_directory: bool,
    pub total_files: u64,
    pub total_bytes: u64,
    /// What is already at that path, if anything: `file` or `directory`.
    pub existing: Option<String>,
}

/// Work out what an import would create, without copying anything.
///
/// The window cannot compute this: unwrapping a folder lands its children and
/// the window cannot read a directory. Returning the sizes at the same time
/// means one walk answers both "what will collide?" and "how much is there?",
/// which is what the conflict dialog and the progress bar each need before the
/// first byte moves.
#[tauri::command]
async fn plan_import_destinations(
    state: tauri::State<'_, AppState>,
    project_id: String,
    target_directory: String,
    source_paths: Vec<String>,
    unwrap_paths: Option<Vec<String>>,
) -> CommandResult<Vec<PlannedDestinationDto>> {
    let app: &AppState = &state;
    let (root, _) = project_root(app, &project_id).await?;
    let unwrap: std::collections::HashSet<String> =
        unwrap_paths.unwrap_or_default().into_iter().collect();

    let sources: Vec<project_host_file_manager::ImportSource> = source_paths
        .into_iter()
        .map(|path| {
            if unwrap.contains(&path) {
                project_host_file_manager::ImportSource::unwrapped(path)
            } else {
                project_host_file_manager::ImportSource::nested(path)
            }
        })
        .collect();

    let planned = project_host_file_manager::operations::plan_import_destinations(
        &root,
        &target_directory,
        &sources,
        &file_limits(app),
    )
    .map_err(CommandError::from)?;

    Ok(planned
        .into_iter()
        .map(|entry| {
            let existing = project_host_file_manager::operations::stat(&root, &entry.relative)
                .ok()
                .map(|found| match found.kind {
                    project_host_file_manager::EntryKind::Directory => "directory".to_string(),
                    _ => "file".to_string(),
                });
            PlannedDestinationDto {
                source: entry.source.to_string_lossy().into_owned(),
                relative: entry.relative,
                is_directory: entry.is_directory,
                total_files: entry.total_files,
                total_bytes: entry.total_bytes,
                existing,
            }
        })
        .collect())
}

/// Move a file or folder to another place in the same project.
///
/// What the explorer's drag-and-drop needs, and what `rename_project_file`
/// deliberately refuses to do: that one takes a single path component so a
/// rename cannot quietly relocate a file. This one takes a whole destination
/// path, and the core still resolves it inside the project root.
#[tauri::command]
async fn move_project_file(
    state: tauri::State<'_, AppState>,
    project_id: String,
    from: String,
    to: String,
) -> CommandResult<FileEntryDto> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    project_host_file_manager::operations::move_entry(&root, &from, &to)
        .map(FileEntryDto::from)
        .map_err(CommandError::from)
}

/// Copy a file or folder within the project — the explorer's "Duplicate".
#[tauri::command]
async fn copy_project_file(
    state: tauri::State<'_, AppState>,
    project_id: String,
    from: String,
    to: String,
) -> CommandResult<FileEntryDto> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    project_host_file_manager::operations::copy(&root, &from, &to, &file_limits(app))
        .map(FileEntryDto::from)
        .map_err(CommandError::from)
}

/// The project's directory on this machine.
///
/// The window has no other way to learn it: every file command takes a path
/// relative to the root and builds the absolute one itself. "Copy path" in the
/// explorer is the first thing that genuinely needs the absolute form, and it
/// only ever displays it.
#[tauri::command]
async fn project_root_path(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> CommandResult<String> {
    let app: &AppState = &state;
    let (root, _) = project_root(app, &project_id).await?;
    Ok(root.to_string_lossy().into_owned())
}

/// Show a project file in the system's file manager.
///
/// The path is resolved through the same guard every other file command uses,
/// so a relative path that climbs out of the project is refused here too rather
/// than handed to a shell.
#[tauri::command]
async fn reveal_project_path(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
) -> CommandResult<()> {
    let app: &AppState = &state;
    let (root, _) = project_root(app, &project_id).await?;
    let target = project_host_file_manager::operations::stat(&root, &path)
        .map(|entry| root.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR)))
        .map_err(CommandError::from)?;

    // Arguments are passed as a list, never through a shell, so a file name
    // containing shell metacharacters is a file name and nothing else.
    let mut command = if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("explorer.exe");
        // One argument, not two: Explorer only understands the selection flag
        // when the path is glued to it.
        let mut select = std::ffi::OsString::from("/select,");
        select.push(&target);
        command.arg(select);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg("-R");
        command.arg(&target);
        command
    } else {
        // No portable "select this file" on Linux; the containing directory is
        // what every desktop environment agrees on.
        let mut command = std::process::Command::new("xdg-open");
        command.arg(target.parent().unwrap_or(&root));
        command
    };

    command.spawn().map(|_| ()).map_err(|error| CommandError {
        message: format!("Could not open the system file manager: {error}"),
    })
}

#[tauri::command]
async fn delete_project_file(
    state: tauri::State<'_, AppState>,
    project_id: String,
    path: String,
    recursive: bool,
) -> CommandResult<()> {
    let app: &AppState = &state;
    let (root, read_only) = project_root(app, &project_id).await?;
    if read_only {
        return Err(CommandError {
            message:
                "This project is being built or removed; its files cannot be changed right now."
                    .to_string(),
        });
    }

    project_host_file_manager::operations::delete(&root, &path, recursive)
        .map_err(CommandError::from)
}

#[tauri::command]
async fn search_project_files(
    state: tauri::State<'_, AppState>,
    project_id: String,
    query: String,
) -> CommandResult<Vec<FileEntryDto>> {
    let app: &AppState = &state;
    let (root, _) = project_root(app, &project_id).await?;
    project_host_file_manager::operations::search(&root, "", &query, &file_limits(app))
        .map(|entries| entries.into_iter().map(FileEntryDto::from).collect())
        .map_err(CommandError::from)
}

// ------------------------------------------------------------------- discord

/// One configured bot, as the Discord screen shows it.
#[derive(Debug, Serialize)]
pub struct DiscordBotDto {
    pub id: String,
    pub label: String,
    pub application_id: String,
    pub autostart: bool,
    /// `stopped`, `connecting`, `connected` or `failed`.
    pub status: String,
    pub failure_reason: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub reconnects: u32,
    pub linked_servers: u32,
    pub project_ids: Vec<String>,
}

impl From<project_host_core::bots::BotView> for DiscordBotDto {
    fn from(view: project_host_core::bots::BotView) -> Self {
        Self {
            id: view.id,
            label: view.label,
            application_id: view.application_id,
            autostart: view.autostart,
            status: view.status,
            failure_reason: view.failure_reason,
            uptime_seconds: view.uptime_seconds,
            reconnects: view.reconnects,
            linked_servers: view.linked_servers,
            project_ids: view.project_ids,
        }
    }
}

/// Whether this installation can hold a bot token at all.
///
/// The screen needs this before it offers to take one: a machine whose secure
/// storage could not be opened must say so up front rather than accepting a
/// token and failing to save it.
#[derive(Debug, Serialize)]
pub struct DiscordReadiness {
    pub can_store_secrets: bool,
    /// `os-keychain` or `restricted-file`. `None` when there is no key.
    pub key_backend: Option<String>,
    pub connected: u32,
}

#[tauri::command]
async fn discord_readiness(state: tauri::State<'_, AppState>) -> CommandResult<DiscordReadiness> {
    let app: &AppState = &state;
    Ok(DiscordReadiness {
        can_store_secrets: app.master_key().is_some(),
        key_backend: app.key_backend().map(str::to_string),
        connected: u32::try_from(app.discord().connected_count()).unwrap_or(u32::MAX),
    })
}

#[tauri::command]
async fn discord_bots(state: tauri::State<'_, AppState>) -> CommandResult<Vec<DiscordBotDto>> {
    let app: &AppState = &state;
    let bots = project_host_core::bots::list_bots(app.database(), app.discord())
        .await
        .map_err(|error| CommandError::from(error.user_facing()))?;
    Ok(bots.into_iter().map(DiscordBotDto::from).collect())
}

/// Verify a token with Discord and store it encrypted.
///
/// The token arrives as a `String` because that is what crosses the IPC
/// boundary, and becomes a `Secret` on the first line so nothing downstream can
/// log it by accident.
#[tauri::command]
async fn add_discord_bot(
    state: tauri::State<'_, AppState>,
    label: String,
    token: String,
) -> CommandResult<String> {
    let app: &AppState = &state;
    let token = Secret::new(token);

    let rest = project_host_discord_gateway::RestClient::new()
        .map_err(|error| CommandError::from(error.user_facing()))?;

    project_host_core::bots::add_bot(app.database(), app.master_key(), &rest, &label, token)
        .await
        .map_err(|error| CommandError::from(error.user_facing()))
}

#[tauri::command]
async fn start_discord_bot(state: tauri::State<'_, AppState>, bot_id: String) -> CommandResult<()> {
    let app: &AppState = &state;
    project_host_core::bots::start_bot(app.database(), app.master_key(), app.discord(), &bot_id)
        .await
        .map_err(|error| CommandError::from(error.user_facing()))
}

#[tauri::command]
async fn stop_discord_bot(state: tauri::State<'_, AppState>, bot_id: String) -> CommandResult<()> {
    let app: &AppState = &state;
    project_host_core::bots::stop_bot(app.discord(), &bot_id).await;
    Ok(())
}

#[tauri::command]
async fn forget_discord_bot(
    state: tauri::State<'_, AppState>,
    bot_id: String,
) -> CommandResult<bool> {
    let app: &AppState = &state;
    project_host_core::bots::forget_bot(app.database(), app.discord(), &bot_id)
        .await
        .map_err(|error| CommandError::from(error.user_facing()))
}

#[tauri::command]
async fn update_discord_bot(
    state: tauri::State<'_, AppState>,
    bot_id: String,
    label: String,
    autostart: bool,
) -> CommandResult<bool> {
    let app: &AppState = &state;
    project_host_core::bots::update_bot(app.database(), &bot_id, &label, autostart)
        .await
        .map_err(|error| CommandError::from(error.user_facing()))
}

/// Choose which projects a bot reports on.
#[tauri::command]
async fn set_discord_bot_projects(
    state: tauri::State<'_, AppState>,
    bot_id: String,
    project_ids: Vec<String>,
) -> CommandResult<()> {
    let app: &AppState = &state;
    project_host_core::bots::set_bot_projects(app.database(), &bot_id, &project_ids)
        .await
        .map_err(|error| CommandError::from(error.user_facing()))
}

/// The configuration the window shows on the settings screen.
///
/// Read-only for now. Every value here is loaded at startup from `config.toml`
/// and the `PROJECT_HOST_*` environment; there is no write path yet, so the
/// screen shows what is in force rather than pretending to accept edits.
#[derive(Debug, Serialize)]
pub struct AppSettings {
    pub mode: String,
    pub log_level: String,
    pub log_json: bool,
    pub log_retention_days: u16,
    pub max_projects: u32,
    pub max_upload_bytes: u64,
    pub port_pool_start: u16,
    pub port_pool_end: u16,
    pub port_pool_size: u32,
    pub docker_enabled: bool,
    pub data_dir: String,
    pub projects_dir: String,
    pub logs_dir: String,
    pub backups_dir: String,
}

#[tauri::command]
async fn app_settings(state: tauri::State<'_, AppState>) -> CommandResult<AppSettings> {
    use project_host_platform::PathProvider;

    let app: &AppState = &state;
    let config = app.config();
    let paths = project_host_core::resolve_paths(config).map_err(CommandError::from)?;

    Ok(AppSettings {
        mode: format!("{:?}", config.mode).to_lowercase(),
        log_level: config.log_level.as_str().to_string(),
        log_json: config.log_json,
        log_retention_days: config.log_retention_days,
        max_projects: config.max_projects,
        max_upload_bytes: config.max_upload_bytes,
        port_pool_start: config.port_pool_start,
        port_pool_end: config.port_pool_end,
        port_pool_size: config.port_pool_size(),
        docker_enabled: config.docker_enabled,
        data_dir: paths.data_dir().display().to_string(),
        projects_dir: paths.projects_dir().display().to_string(),
        logs_dir: paths.log_dir().display().to_string(),
        backups_dir: paths.backups_dir().display().to_string(),
    })
}

/// Serialises the install so two presses cannot both replace the application.
///
/// The window already disables its button while an install runs, but the button
/// exists in two places, a periodic check can fire between them, and neither of
/// those facts should be what keeps two installers from running at once. The
/// flag lives here, on the one side that both callers go through.
#[derive(Debug, Default)]
pub struct UpdateActivity {
    installing: std::sync::atomic::AtomicBool,
}

impl UpdateActivity {
    /// Claim the right to install, or `None` if someone already holds it.
    fn claim(&self) -> Option<InstallGuard<'_>> {
        self.installing
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .ok()
            .map(|_| InstallGuard { activity: self })
    }

    pub fn is_installing(&self) -> bool {
        self.installing.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Releases the claim however the install ends — including on an early return
/// or a panic, which is the reason this is a guard rather than a pair of calls.
struct InstallGuard<'a> {
    activity: &'a UpdateActivity,
}

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.activity
            .installing
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

/// How far an install has got, for the window's progress bar.
///
/// Emitted as `update://progress`. `total_bytes` is `None` when the server sent
/// no `Content-Length`, which is why the window must be able to render an
/// indeterminate bar rather than assuming a percentage is always available.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    /// `downloading`, `verifying`, `installing` or `restarting`.
    pub phase: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

impl UpdateProgress {
    const EVENT: &'static str = "update://progress";
}

/// Ask the release feed whether there is a newer version.
///
/// Every rule about *whether to offer* an update lives in the `updater` crate;
/// this only performs the fetch, which is the one part that needs a network and
/// therefore cannot be unit tested.
///
/// What this cannot see is as important as what it can: the feed is
/// `releases/latest`, which GitHub resolves to the newest *published,
/// non-prerelease* release. A commit, a tag, a branch and a draft release are
/// all invisible to it — pushing code is not shipping it. A pre-release that
/// does reach the feed is refused a second time by [`project_host_updater`],
/// which only offers one on the beta channel.
#[tauri::command]
async fn check_for_update() -> CommandResult<UpdateCheck> {
    let client = reqwest::Client::builder()
        .timeout(UPDATE_TIMEOUT)
        .user_agent(concat!("project-host/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(CommandError::from)?;

    let response = client
        .get(project_host_updater::RELEASE_FEED_URL)
        .send()
        .await
        .map_err(CommandError::from)?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // The honest answer while no release has ever been published, and a
        // clearer one than "the feed returned 404".
        return Err(CommandError {
            message: "No releases have been published yet.".to_string(),
        });
    }
    let response = response.error_for_status().map_err(CommandError::from)?;

    let manifest: ReleaseManifest = response.json().await.map_err(CommandError::from)?;

    evaluate(
        &manifest,
        project_host_updater::CURRENT_VERSION,
        &UpdatePreferences::default(),
    )
    .map_err(CommandError::from)
}

/// Why this installation cannot replace itself, or `None` when it can.
///
/// Only the `.deb` cannot: `docs/installers.md` §5 lists NSIS, MSI and AppImage
/// as self-updating and dpkg's package as not. An AppImage sets `APPIMAGE` to
/// its own path when it runs, so its absence on Linux means the binary came
/// from the package.
fn self_update_unavailable() -> Option<String> {
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        return Some(
            "This copy was installed from the .deb package, which apt owns and \
             the application cannot replace by itself. Update it with \
             `sudo apt install ./Panel.Platform_<version>_amd64.deb`, or \
             download the new release from the website."
                .to_string(),
        );
    }
    None
}

/// Download the offered update, verify its signature, and install it.
///
/// Only ever called after [`check_for_update`] has returned an update and the
/// user has pressed the button; nothing here decides whether an update *should*
/// happen. The plugin performs its own fetch of the same manifest, so this
/// costs one extra request at the moment of the click — the alternative is
/// handing the plugin a manifest we parsed ourselves, which would move
/// signature verification off the path that Tauri actually audits.
///
/// The signature is checked against the public key compiled into the binary
/// from `tauri.conf.json`, never one supplied by the feed. On Windows the
/// installer takes over and the process exits; on Linux the AppImage is
/// replaced in place. A `.deb` install cannot update itself and returns an
/// error saying so.
#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    activity: tauri::State<'_, UpdateActivity>,
) -> CommandResult<()> {
    use tauri_plugin_updater::UpdaterExt;

    // Before anything is fetched: two installers replacing the same files is
    // the one failure here with no good ending.
    let Some(_guard) = activity.claim() else {
        return Err(CommandError {
            message: "An update is already being installed.".to_string(),
        });
    };

    // A `.deb` install is owned by dpkg: replacing those files from inside the
    // running process would leave the package manager describing a version that
    // is no longer on disk. The plugin refuses, but its message is about a
    // missing `APPIMAGE` variable, which tells the reader nothing about what to
    // do. This says it plainly, before any download happens.
    if let Some(message) = self_update_unavailable() {
        return Err(CommandError { message });
    }

    let update = app
        .updater()
        .map_err(|error| CommandError {
            message: format!("The updater could not start: {error}"),
        })?
        .check()
        .await
        .map_err(|error| CommandError {
            message: format!("The release feed could not be reached: {error}"),
        })?;

    let Some(update) = update else {
        return Err(CommandError {
            message: "There is no update to install.".to_string(),
        });
    };

    let emit = |progress: UpdateProgress| {
        // A window that has gone away is not a reason to abandon an install
        // that is already replacing files.
        if let Err(error) = app.emit(UpdateProgress::EVENT, progress) {
            tracing::warn!(%error, "could not report update progress to the window");
        }
    };

    emit(UpdateProgress {
        phase: "downloading",
        downloaded_bytes: 0,
        total_bytes: None,
    });

    // `download` verifies the signature against the public key compiled into
    // this binary before it returns the bytes, so "verifying" is the step
    // between the last chunk and the installer starting — not a stage we
    // perform ourselves and could get wrong.
    let mut downloaded: u64 = 0;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                emit(UpdateProgress {
                    phase: "downloading",
                    downloaded_bytes: downloaded,
                    total_bytes: total,
                });
            },
            || {
                emit(UpdateProgress {
                    phase: "verifying",
                    downloaded_bytes: 0,
                    total_bytes: None,
                });
            },
        )
        .await
        .map_err(|error| CommandError {
            message: format!("The update could not be downloaded or verified: {error}"),
        })?;

    emit(UpdateProgress {
        phase: "installing",
        downloaded_bytes: downloaded,
        total_bytes: Some(downloaded),
    });

    // The installer is about to end this process — on Windows by calling
    // `std::process::exit(0)` from inside `install`, which runs no destructor
    // and gives the database no chance to close. A checkpoint folds the
    // write-ahead log back into the file first, so the new version opens a
    // tidy database instead of replaying a log written by the old one.
    //
    // A checkpoint rather than a shutdown, deliberately: it is idempotent and
    // leaves the application completely usable, so an install that fails on the
    // next line has cost the user nothing. Nothing is lost either way — the log
    // is crash-safe and startup recovery handles an unclean stop — but "the
    // update worked and the first thing it did was recover" reads like damage.
    {
        let state = app.state::<AppState>();
        if let Err(error) = state.database().checkpoint().await {
            tracing::warn!(%error, "could not checkpoint the database before installing");
        }
    }

    update.install(bytes).map_err(|error| CommandError {
        message: format!("The update could not be installed: {error}"),
    })?;

    // Only Linux gets here. On Windows the plugin hands over to the installer
    // and calls `std::process::exit(0)` inside `install`, and the installer
    // starts the new version itself — NSIS with `/UPDATE`, the MSI with
    // `AUTOLAUNCHAPP=True`. On Linux the AppImage has been replaced on disk and
    // this process is still the old one, so it has to be the thing that goes.
    emit(UpdateProgress {
        phase: "restarting",
        downloaded_bytes: downloaded,
        total_bytes: Some(downloaded),
    });
    tracing::info!("update installed; restarting into the new version");
    app.restart();
}

// ------------------------------------------------------------ static serving

/// What a static-site process was asked to serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticServerRequest {
    pub root: std::path::PathBuf,
    pub port: u16,
}

/// Whether this process was started to serve a static site rather than to open
/// a window.
///
/// A static project's start command is this very executable with
/// `--serve-static <directory> --port <n>`, which is how a static site runs on
/// a machine with no toolchain and no container daemon. The flag's spelling
/// lives in `app-core` beside the code that produces it, so the two cannot
/// drift into a project spawning an argument nothing handles.
pub fn static_server_request() -> Option<StaticServerRequest> {
    parse_static_arguments(std::env::args().skip(1))
}

/// The argument parse, split out so it can be tested without a process.
fn parse_static_arguments<I>(arguments: I) -> Option<StaticServerRequest>
where
    I: IntoIterator<Item = String>,
{
    let arguments: Vec<String> = arguments.into_iter().collect();
    let flag = project_host_core::orchestrator::SERVE_STATIC_FLAG;

    let index = arguments.iter().position(|value| value == flag)?;
    let root = arguments.get(index + 1)?;
    // A flag with no directory after it is a malformed invocation, not a
    // request to serve the current working directory — which would publish
    // whatever this process happened to be started in.
    if root.starts_with("--") {
        return None;
    }

    let port = arguments
        .iter()
        .position(|value| value == "--port")
        .and_then(|at| arguments.get(at + 1))
        .and_then(|value| value.parse().ok())?;

    Some(StaticServerRequest {
        root: std::path::PathBuf::from(root),
        port,
    })
}

/// Serve a static site until the process is stopped.
///
/// Loopback only. A static preview must not become a file server for whatever
/// network the machine is on, and the project's port is reachable from the
/// browser on this machine either way.
pub fn serve_static(request: StaticServerRequest) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(project_host_static_server::serve(
        request.root,
        &format!("127.0.0.1:{}", request.port),
    ))?;
    Ok(())
}

/// Build and run the application.
///
/// The window opens first, on a loading page. Directories, the database and
/// crash recovery run after that, because waiting for them is what made a
/// click look like nothing happened. Docker and autostart run later still.
/// PowerShell is not spawned on this path at all.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load(&std::path::PathBuf::from("config.toml"))?;
    let paths = resolve_paths(&config)?;
    paths.ensure_all()?;
    let _logging = match project_host_core::logging::init(&config, paths.log_dir()) {
        Ok(guard) => Some(guard),
        Err(error) => {
            eprintln!("logging could not be initialised: {error}");
            None
        }
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let launch = Arc::new(Mutex::new(LaunchStatus::Starting));
    let launch_for_thread = launch.clone();
    let core_thread = Arc::new(Mutex::new(None::<std::thread::JoinHandle<()>>));
    let core_thread_for_setup = core_thread.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(launch)
        .manage(FileImportCancels::default())
        .manage(UpdateActivity::default())
        .manage(Metrics::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            let thread = std::thread::Builder::new()
                .name("panel-core".into())
                .spawn(move || {
                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        tracing::error!("the core runtime could not be created");
                        return;
                    };
                    runtime.block_on(async move {
                        match Runtime::start(config, paths).await {
                            Ok(mut started) => {
                                started.spawn_usage_sampler(shutdown_rx.clone());
                                started.spawn_reconciler(shutdown_rx.clone());
                                started.spawn_power_manager(shutdown_rx.clone());

                                handle.manage(started.state().clone());
                                handle.manage(started.diagnostics_handle());
                                *launch_for_thread
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                    LaunchStatus::Ready;
                                let _ = handle.emit("core://ready", ());

                                started.complete_optional_startup().await;
                                started.start_automatic_workloads().await;

                                let mut shutdown_rx = shutdown_rx;
                                loop {
                                    if *shutdown_rx.borrow() {
                                        break;
                                    }
                                    if shutdown_rx.changed().await.is_err() {
                                        break;
                                    }
                                }
                                started.shutdown().await;
                            }
                            Err(error) => {
                                tracing::error!(%error, "the core could not start");
                                let message = error.to_string();
                                *launch_for_thread
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                    LaunchStatus::Failed {
                                        message: message.clone(),
                                    };
                                let _ = handle.emit("core://failed", message);
                            }
                        }
                    });
                })
                .map_err(|error| error.to_string())?;
            *core_thread_for_setup
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(thread);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            system_status,
            startup_diagnostics,
            launch_status,
            list_projects,
            create_project,
            start_project,
            toolchain_readiness,
            install_toolchain,
            stop_project,
            restart_project,
            kill_project,
            host_projects_running,
            list_project_processes,
            restart_project_process,
            machine_load,
            power_status,
            set_power_mode,
            power_journal,
            set_project_power,
            project_console,
            app_settings,
            check_for_update,
            install_update,
            restart_app,
            minimize_window,
            supported_runtimes,
            github_cli_status,
            list_project_files,
            read_project_file,
            write_project_file,
            begin_project_file_upload,
            append_project_file_upload,
            finish_project_file_upload,
            cancel_project_file_upload,
            import_project_files,
            cancel_project_file_import,
            create_project_file,
            rename_project_file,
            move_project_file,
            copy_project_file,
            delete_project_file,
            search_project_files,
            discord_readiness,
            discord_bots,
            add_discord_bot,
            start_discord_bot,
            stop_discord_bot,
            forget_discord_bot,
            update_discord_bot,
            set_discord_bot_projects,
            inspect_import_paths,
            plan_import_destinations,
            project_root_path,
            reveal_project_path,
            system_metrics,
            recent_activity,
            project_details,
            project_deployments,
            project_events
        ])
        .build(tauri::generate_context!())?
        .run(move |_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Stop host projects and close the database cleanly, so the next
                // start does not have to run recovery. Project *containers* are
                // untouched: Docker keeps them running under their own restart
                // policy. Host projects are children of this process and would
                // die with it regardless — stopping them deliberately is what
                // gets STOPPED recorded instead of a row claiming they run.
                let _ = shutdown_tx.send_replace(true);
                if let Some(thread) = core_thread
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take()
                {
                    let _ = thread.join();
                }
            }
        });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the guard: the second caller is refused, and the
    /// refusal is not permanent.
    #[test]
    fn only_one_install_can_run_at_a_time() {
        let activity = UpdateActivity::default();
        assert!(!activity.is_installing());

        let first = activity.claim().expect("the first caller takes the claim");
        assert!(activity.is_installing());
        assert!(
            activity.claim().is_none(),
            "a second install started while the first was running"
        );

        drop(first);
        assert!(!activity.is_installing());
        assert!(
            activity.claim().is_some(),
            "the claim was never released, so no further update could ever install"
        );
    }

    /// An install that fails must not wedge the button forever. The guard is a
    /// guard rather than two calls precisely so that every exit path releases
    /// it, including `?` returning early.
    #[test]
    fn a_failed_install_releases_the_claim() {
        let activity = UpdateActivity::default();

        fn fails(activity: &UpdateActivity) -> Result<(), &'static str> {
            let _guard = activity.claim().ok_or("busy")?;
            Err("the download failed")
        }

        assert_eq!(fails(&activity), Err("the download failed"));
        assert!(!activity.is_installing());
        assert_eq!(fails(&activity), Err("the download failed"));
    }

    /// The user can press Cancel while the import row is still `queued`, which
    /// is before the blocking copy has asked whether it was cancelled. That
    /// request has to survive until the copy starts, or the import runs on
    /// after the user stopped it.
    #[test]
    fn a_cancel_that_arrives_before_the_import_starts_is_still_honoured() {
        let cancels = FileImportCancels::default();

        cancels.cancel("import-1".to_string());

        assert!(
            cancels.is_cancelled("import-1"),
            "the import started anyway after the user cancelled it"
        );
    }

    /// Import ids are generated per attempt and never reused, so one import's
    /// cancellation must not reach the next one.
    #[test]
    fn cancelling_one_import_does_not_cancel_another() {
        let cancels = FileImportCancels::default();

        cancels.cancel("import-1".to_string());

        assert!(!cancels.is_cancelled("import-2"));
        cancels.clear("import-1");
        assert!(!cancels.is_cancelled("import-1"));
    }

    /// A panic anywhere while the set is locked poisons the mutex for the rest
    /// of the process. Treating that as "cancelled" would mean every later
    /// import in the session fails with a cancellation the user never asked
    /// for, and only a restart would fix it.
    #[test]
    fn a_poisoned_lock_does_not_cancel_every_later_import() {
        let cancels = FileImportCancels::default();
        let poisoner = cancels.clone();

        let panicked = std::thread::spawn(move || {
            let _held = poisoner.import_ids.lock().expect("lock");
            panic!("a thread panicked while holding the lock");
        })
        .join();
        assert!(panicked.is_err(), "the test needs the lock to be poisoned");

        assert!(
            !cancels.is_cancelled("import-1"),
            "a poisoned lock cancelled an import nobody cancelled"
        );
        cancels.cancel("import-1".to_string());
        assert!(
            cancels.is_cancelled("import-1"),
            "a poisoned lock made cancellation stop working"
        );
    }

    /// The sweep for staging directories left by a crashed import asks this
    /// which imports are still copying. Getting it wrong in this direction
    /// deletes files out from under a running import.
    #[test]
    fn an_import_is_marked_as_running_for_exactly_as_long_as_it_runs() {
        let imports = FileImportCancels::default();
        assert!(!imports.is_running("import-1"));

        let running = imports.start("import-1".to_string());
        assert!(imports.is_running("import-1"));
        assert!(!imports.is_running("import-2"));

        drop(running);
        assert!(
            !imports.is_running("import-1"),
            "a finished import stayed marked as running, so its staging is never swept"
        );
    }

    /// Every exit path has to release the mark, including a panic on the
    /// blocking thread that copies the files.
    #[test]
    fn an_import_that_panics_is_no_longer_marked_as_running() {
        let imports = FileImportCancels::default();
        let for_thread = imports.clone();

        let panicked = std::thread::spawn(move || {
            let _running = for_thread.start("import-1".to_string());
            panic!("the copy thread panicked");
        })
        .join();

        assert!(panicked.is_err(), "the test needs the thread to panic");
        assert!(!imports.is_running("import-1"));
    }

    /// A cancellation names one import. Once that import is over the request
    /// has been served and must not linger.
    #[test]
    fn finishing_an_import_releases_its_cancellation() {
        let imports = FileImportCancels::default();
        let running = imports.start("import-1".to_string());

        imports.cancel("import-1".to_string());
        assert!(imports.is_cancelled("import-1"));

        drop(running);
        assert!(!imports.is_cancelled("import-1"));
    }

    #[test]
    fn the_progress_event_name_is_namespaced() {
        // Tauri events share one namespace with everything else the window
        // listens to.
        assert_eq!(UpdateProgress::EVENT, "update://progress");
    }
}
