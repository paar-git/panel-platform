//! What to run, in what order, to give this machine a toolchain.
//!
//! Pure. The platform, the package manager and the probe result are arguments,
//! never things this module looks up, so every platform's plan is checked on
//! every host.
//!
//! Nothing here spawns anything. The mistakes live in this layer, not in the
//! spawning, and this is the layer a test can reach.

use project_host_platform::PackageManager;

use crate::blocker::Blocker;
use crate::catalog::{prerequisite, spec_for};

/// The machine, reduced to what deciding an install actually depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    Windows {
        /// Fresh Windows Server images frequently lack it, which is the machine
        /// this feature exists for.
        winget_present: bool,
    },
    Linux {
        manager: Option<PackageManager>,
    },
    /// The probe could not identify the operating system.
    Unknown,
}

/// One command in a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Whether this command needs administrator or root.
    pub elevated: bool,
    pub program: String,
    pub args: Vec<String>,
    /// What this step installs, named the way a failure should name it.
    ///
    /// Per step rather than per plan: the last step of a plan installs the
    /// *project's* dependencies, and reporting that failure as "Installing
    /// Node.js failed" names software that installed perfectly and sends the
    /// user to reinstall it.
    pub subject: String,
    /// Shown to the user verbatim before anything runs.
    pub describes: String,
}

/// A project's own dependency install, already split into words by the caller.
///
/// Taken pre-split rather than as a string so this crate does not carry a
/// second command-line splitter; `host_runner::command` owns that, and two
/// implementations of it would be two things to get wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInstall {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Nothing to do: the toolchain is present, or the runtime needs none.
    Nothing,
    Install {
        steps: Vec<Step>,
    },
    Blocked(Blocker),
}

/// Decide what would give `runtime` a working toolchain on `host`.
pub fn plan(
    runtime: &str,
    toolchain_present: bool,
    host: &Host,
    project_install: Option<&ProjectInstall>,
) -> Plan {
    let mut steps = Vec::new();

    if !toolchain_present {
        match toolchain_steps(runtime, host) {
            Ok(planned) => steps = planned,
            Err(blocker) => return Plan::Blocked(blocker),
        }
    }

    // Last, and unelevated: this is the first command that belongs to the
    // project rather than to the machine, and elevation ends before it.
    if let Some(install) = project_install {
        steps.push(Step {
            elevated: false,
            program: install.program.clone(),
            args: install.args.clone(),
            subject: "the project's own dependencies".to_string(),
            describes: format!(
                "Install the project's own dependencies ({} {})",
                install.program,
                install.args.join(" ")
            ),
        });
    }

    if steps.is_empty() {
        Plan::Nothing
    } else {
        Plan::Install { steps }
    }
}

fn toolchain_steps(runtime: &str, host: &Host) -> Result<Vec<Step>, Blocker> {
    match runtime {
        // A static site is served, not executed. Nothing to install.
        "STATIC" => return Ok(Vec::new()),
        "POLYGLOT" => return Err(Blocker::PolyglotUnresolvable),
        _ => {}
    }

    let spec = spec_for(runtime).ok_or_else(|| Blocker::RuntimeUnsupported {
        runtime: runtime.to_string(),
    })?;

    match host {
        Host::Unknown => Err(Blocker::HostUnrecognised),

        Host::Windows { winget_present } => {
            let mut steps = Vec::new();

            if !winget_present {
                steps.push(bootstrap_winget());
            }

            for id in spec.prerequisites {
                let entry = prerequisite(id).ok_or_else(|| Blocker::RuntimeUnsupported {
                    runtime: runtime.to_string(),
                })?;
                let package = entry
                    .winget_id
                    .ok_or_else(|| Blocker::NotPackagedForPlatform {
                        display_name: entry.display_name.to_string(),
                        platform: "Windows".to_string(),
                        vendor: spec.vendor.to_string(),
                    })?;
                steps.push(winget_step(package, entry.display_name));
            }

            let package = spec
                .winget_id
                .ok_or_else(|| Blocker::NotPackagedForPlatform {
                    display_name: spec.display_name.to_string(),
                    platform: "Windows".to_string(),
                    vendor: spec.vendor.to_string(),
                })?;
            steps.push(winget_step(package, spec.display_name));

            Ok(steps)
        }

        Host::Linux { manager: None } => Err(Blocker::NoPackageManager {
            platform: "Linux".to_string(),
            remedy: "Panel Platform installs toolchains through apt, dnf, pacman \
                     or zypper. Install the toolchain with this distribution's \
                     own package manager and start the project again."
                .to_string(),
        }),

        Host::Linux {
            manager: Some(manager),
        } => {
            let package =
                spec.linux_package(*manager)
                    .ok_or_else(|| Blocker::NotPackagedForPlatform {
                        display_name: spec.display_name.to_string(),
                        platform: "Linux".to_string(),
                        vendor: spec.vendor.to_string(),
                    })?;

            let mut steps = Vec::new();

            // Installing from a stale index is how a package that exists is
            // reported as not found.
            if *manager == PackageManager::Apt {
                steps.push(Step {
                    elevated: true,
                    program: "apt-get".to_string(),
                    args: vec!["update".to_string()],
                    subject: "the package list".to_string(),
                    describes: "Refresh the package list".to_string(),
                });
            }

            for id in spec.prerequisites {
                let entry = prerequisite(id).ok_or_else(|| Blocker::RuntimeUnsupported {
                    runtime: runtime.to_string(),
                })?;
                let package = entry.linux_package(*manager).ok_or_else(|| {
                    Blocker::NotPackagedForPlatform {
                        display_name: entry.display_name.to_string(),
                        platform: "Linux".to_string(),
                        vendor: spec.vendor.to_string(),
                    }
                })?;
                steps.push(linux_step(*manager, package, entry.display_name));
            }

            steps.push(linux_step(*manager, package, spec.display_name));

            Ok(steps)
        }
    }
}

/// Register App Installer, which is what supplies `winget`.
///
/// Windows Server images ship the package without registering it for the user,
/// which is why this is a registration rather than a download.
fn bootstrap_winget() -> Step {
    Step {
        elevated: true,
        program: "powershell.exe".to_string(),
        args: vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            "Add-AppxPackage -RegisterByFamilyName -MainPackage \
             Microsoft.DesktopAppInstaller_8wekyb3d8bbwe"
                .to_string(),
        ],
        subject: "App Installer".to_string(),
        describes: "Enable App Installer, which provides winget".to_string(),
    }
}

fn winget_step(package: &str, display_name: &str) -> Step {
    Step {
        elevated: true,
        program: "winget".to_string(),
        args: vec![
            "install".to_string(),
            "--id".to_string(),
            package.to_string(),
            // Without --exact a mistyped id installs whatever ranks first.
            "--exact".to_string(),
            "--silent".to_string(),
            // The user approved this package by name in the offer; the prompts
            // these suppress are winget's, not a licence decision made for them.
            "--accept-package-agreements".to_string(),
            "--accept-source-agreements".to_string(),
        ],
        subject: display_name.to_string(),
        describes: format!("Install {display_name} ({package}) using winget"),
    }
}

fn linux_step(manager: PackageManager, package: &str, display_name: &str) -> Step {
    let (program, mut args) = match manager {
        PackageManager::Apt => ("apt-get", vec!["install".to_string(), "-y".to_string()]),
        PackageManager::Dnf => ("dnf", vec!["install".to_string(), "-y".to_string()]),
        PackageManager::Pacman => ("pacman", vec!["-S".to_string(), "--noconfirm".to_string()]),
        PackageManager::Zypper => (
            "zypper",
            vec!["--non-interactive".to_string(), "install".to_string()],
        ),
    };
    args.push(package.to_string());

    Step {
        elevated: true,
        program: program.to_string(),
        args,
        subject: display_name.to_string(),
        describes: format!(
            "Install {display_name} ({package}) using {}",
            manager.as_str()
        ),
    }
}

/// Wrap a command so it runs with administrator or root rights.
///
/// `unsafe_code` is forbidden across this workspace, so `ShellExecuteEx` — the
/// direct way to raise a UAC prompt — is unavailable. `Start-Process -Verb
/// RunAs` raises the same prompt without FFI. `-Wait` and the propagated
/// `ExitCode` are what keep a failed install from looking like a success.
pub fn elevate(host: &Host, program: &str, args: &[String]) -> (String, Vec<String>) {
    match host {
        Host::Windows { .. } => {
            let mut script = format!(
                "try {{ $p = Start-Process -FilePath '{}'",
                single_quote(program)
            );

            if !args.is_empty() {
                let list: Vec<String> = args
                    .iter()
                    .map(|arg| format!("'{}'", single_quote(arg)))
                    .collect();
                script.push_str(&format!(" -ArgumentList {}", list.join(",")));
            }

            // A dismissed prompt throws rather than returning, so without the
            // catch this exits 1 and reads as a failed install. 1223 is
            // ERROR_CANCELLED, which `execute` maps to NotAuthorised.
            script.push_str(" -Verb RunAs -Wait -PassThru } catch { exit 1223 }; exit $p.ExitCode");

            (
                "powershell.exe".to_string(),
                vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                    script,
                ],
            )
        }

        // pkexec rather than sudo: it prompts through the desktop, where this
        // program's user already is, and needs no terminal that a
        // double-clicked binary does not have. The same choice, for the same
        // reason, as `setup::handoff`.
        Host::Linux { .. } => {
            let mut all = vec![program.to_string()];
            all.extend_from_slice(args);
            ("pkexec".to_string(), all)
        }

        // Unreachable through `plan`, which blocks an unknown host before any
        // step exists. Running the command unwrapped is the conservative
        // answer: it fails for want of rights rather than escalating.
        Host::Unknown => (program.to_string(), args.to_vec()),
    }
}

/// Escape a value for a PowerShell single-quoted string, where the only
/// metacharacter is the quote itself and it is escaped by doubling.
fn single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Host {
        Host::Windows {
            winget_present: true,
        }
    }

    fn linux(manager: PackageManager) -> Host {
        Host::Linux {
            manager: Some(manager),
        }
    }

    fn steps(plan: Plan) -> Vec<Step> {
        match plan {
            Plan::Install { steps } => steps,
            other => panic!("expected an install plan, got {other:?}"),
        }
    }

    fn blocker(plan: Plan) -> Blocker {
        match plan {
            Plan::Blocked(blocker) => blocker,
            other => panic!("expected a blocker, got {other:?}"),
        }
    }

    /// The overwhelmingly common path: the machine already has what the project
    /// needs, and pressing Start must not produce an offer.
    #[test]
    fn a_present_toolchain_needs_no_plan() {
        assert_eq!(plan("NODEJS", true, &windows(), None), Plan::Nothing);
    }

    #[test]
    fn a_static_site_needs_no_toolchain() {
        assert_eq!(plan("STATIC", false, &windows(), None), Plan::Nothing);
    }

    /// Picking one toolchain for a project that declared several would leave it
    /// half provisioned and still broken.
    #[test]
    fn polyglot_is_refused_rather_than_guessed_at() {
        assert_eq!(
            blocker(plan("POLYGLOT", false, &windows(), None)),
            Blocker::PolyglotUnresolvable
        );
    }

    #[test]
    fn an_unknown_runtime_is_refused_by_name() {
        assert_eq!(
            blocker(plan("PASCAL", false, &windows(), None)),
            Blocker::RuntimeUnsupported {
                runtime: "PASCAL".to_string()
            }
        );
    }

    #[test]
    fn an_unidentified_operating_system_cannot_be_planned_for() {
        assert_eq!(
            blocker(plan("NODEJS", false, &Host::Unknown, None)),
            Blocker::HostUnrecognised
        );
    }

    /// The plan must name the package, not merely "install Node".
    #[test]
    fn windows_installs_through_winget_naming_the_package() {
        let steps = steps(plan("NODEJS", false, &windows(), None));
        let last = steps.last().expect("a step");

        assert!(last.args.contains(&"OpenJS.NodeJS.LTS".to_string()));
        assert!(last.describes.contains("Node.js"));
    }

    #[test]
    fn linux_installs_through_the_manager_the_machine_actually_has() {
        let apt = steps(plan("PYTHON", false, &linux(PackageManager::Apt), None));
        assert!(apt
            .iter()
            .any(|step| step.args.contains(&"python3".to_string())));

        let pacman = steps(plan("PYTHON", false, &linux(PackageManager::Pacman), None));
        assert!(pacman
            .iter()
            .any(|step| step.args.contains(&"python".to_string())));
    }

    /// Without build tools the toolchain install reports success and the first
    /// dependency with a native binding fails afterwards.
    #[test]
    fn prerequisites_are_ordered_before_the_toolchain_that_needs_them() {
        let steps = steps(plan("NODEJS", false, &windows(), None));

        let build_tools = steps
            .iter()
            .position(|step| step.describes.contains("build tools"))
            .expect("build tools are planned for Node");
        let node = steps
            .iter()
            .position(|step| step.args.contains(&"OpenJS.NodeJS.LTS".to_string()))
            .expect("Node is planned");

        assert!(
            build_tools < node,
            "prerequisites must be installed first, got {steps:#?}"
        );
    }

    /// The machine this feature exists for. Without this step the flow dead
    /// ends on exactly the servers it was built to serve.
    #[test]
    fn a_windows_machine_without_winget_bootstraps_it_first() {
        let steps = steps(plan(
            "NODEJS",
            false,
            &Host::Windows {
                winget_present: false,
            },
            None,
        ));

        assert!(
            steps[0].describes.contains("App Installer"),
            "the first step must supply the package manager, got {:?}",
            steps[0].describes
        );
    }

    #[test]
    fn a_linux_machine_with_no_package_manager_is_blocked_with_a_remedy() {
        let blocker = blocker(plan("NODEJS", false, &Host::Linux { manager: None }, None));

        match blocker {
            Blocker::NoPackageManager { remedy, .. } => {
                assert!(!remedy.is_empty(), "a blocker must say what to do next")
            }
            other => panic!("expected NoPackageManager, got {other:?}"),
        }
    }

    /// Bun ships its own installer and is in no distribution's repository.
    /// Inventing a package name would fail at the moment the user pressed Start.
    #[test]
    fn a_runtime_linux_does_not_package_is_refused_with_the_vendors_name() {
        let blocker = blocker(plan("BUN", false, &linux(PackageManager::Apt), None));

        match blocker {
            Blocker::NotPackagedForPlatform {
                display_name,
                vendor,
                ..
            } => {
                assert_eq!(display_name, "Bun");
                assert!(vendor.contains("bun.sh"), "got {vendor}");
            }
            other => panic!("expected NotPackagedForPlatform, got {other:?}"),
        }
    }

    /// Elevation must end before any of the project's own code runs.
    #[test]
    fn the_projects_dependency_install_runs_last_and_unelevated() {
        let install = ProjectInstall {
            program: "npm".to_string(),
            args: vec!["install".to_string()],
        };
        let steps = steps(plan("NODEJS", false, &windows(), Some(&install)));
        let last = steps.last().expect("a step");

        assert_eq!(last.program, "npm");
        assert!(
            !last.elevated,
            "project code must never run with administrator rights"
        );
        assert!(
            steps[..steps.len() - 1].iter().all(|step| step.elevated),
            "every install step before it needs elevation"
        );
    }

    /// The mislabelling a user met: `npm install` failed and was reported as
    /// "Installing Node.js (for TypeScript) failed", naming a toolchain that
    /// had installed without complaint and sending them to reinstall it.
    ///
    /// Every step carries what it is installing, so no caller can label them
    /// all with the name of the language.
    #[test]
    fn a_step_names_what_it_installs_rather_than_the_language() {
        let install = ProjectInstall {
            program: "npm".to_string(),
            args: vec!["install".to_string()],
        };
        let steps = steps(plan("NODEJS", false, &windows(), Some(&install)));
        let last = steps.last().expect("a step");

        assert!(
            !last.subject.contains("Node.js"),
            "the project's own dependencies are not the toolchain, got {:?}",
            last.subject
        );
        assert!(
            steps
                .iter()
                .any(|step| step.subject == "Node.js" || step.subject.contains("build tools")),
            "the toolchain steps still name the toolchain, got {steps:#?}"
        );
        assert!(
            steps.iter().all(|step| !step.subject.is_empty()),
            "a step with no subject would produce a failure that names nothing"
        );
    }

    /// A toolchain that is already present still leaves the project's own
    /// dependencies to install.
    #[test]
    fn a_present_toolchain_still_installs_the_projects_dependencies() {
        let install = ProjectInstall {
            program: "npm".to_string(),
            args: vec!["install".to_string()],
        };
        let steps = steps(plan("NODEJS", true, &windows(), Some(&install)));

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].program, "npm");
    }

    /// Every installable runtime produces a plan on Windows and on all four
    /// Linux managers, or a blocker that names why. What must never happen is
    /// a panic or an empty plan at the moment a user presses Start.
    #[test]
    fn every_runtime_produces_a_plan_or_a_named_blocker_on_every_platform() {
        let hosts = [
            windows(),
            linux(PackageManager::Apt),
            linux(PackageManager::Dnf),
            linux(PackageManager::Pacman),
            linux(PackageManager::Zypper),
        ];

        for spec in crate::catalog() {
            for host in &hosts {
                match plan(spec.runtime, false, host, None) {
                    Plan::Install { steps } => {
                        assert!(!steps.is_empty(), "{} planned nothing", spec.id)
                    }
                    Plan::Blocked(blocker) => assert!(
                        !blocker.to_string().is_empty(),
                        "{} blocked without a reason",
                        spec.id
                    ),
                    Plan::Nothing => panic!("{} is missing and yet needs nothing", spec.id),
                }
            }
        }
    }

    /// `unsafe_code` is forbidden workspace-wide, so `ShellExecuteEx` is not
    /// available and elevation goes through `Start-Process -Verb RunAs`. The
    /// exit code must be propagated, or a failed install looks like a success.
    #[test]
    fn windows_elevation_requests_runas_and_propagates_the_exit_code() {
        let (program, args) = elevate(&windows(), "winget", &["install".to_string()]);
        let script = args.join(" ");

        assert!(program.to_lowercase().contains("powershell"));
        assert!(script.contains("-Verb RunAs"));
        assert!(
            script.contains("-Wait"),
            "the plan must not race the installer"
        );
        assert!(script.contains("ExitCode"));
    }

    /// pkexec prompts through the desktop and needs no terminal, which a
    /// double-clicked binary does not have. The same choice `setup::handoff`
    /// made, for the same reason.
    #[test]
    fn linux_elevation_uses_pkexec_rather_than_sudo() {
        let (program, args) = elevate(
            &linux(PackageManager::Apt),
            "apt-get",
            &["install".to_string()],
        );

        assert_eq!(program, "pkexec");
        assert_eq!(args, vec!["apt-get".to_string(), "install".to_string()]);
    }

    /// Dismissing the UAC prompt makes `Start-Process` throw, so `$p` is never
    /// assigned and the script would exit 1 — indistinguishable from a failed
    /// install. The cancellation code has to be produced deliberately.
    #[test]
    fn a_dismissed_prompt_exits_with_the_cancellation_code() {
        let (_, args) = elevate(&windows(), "winget", &["install".to_string()]);
        let script = args.join(" ");

        assert!(script.contains("catch"), "got {script}");
        assert!(
            script.contains("1223"),
            "a dismissed prompt must be distinguishable from a failure, got {script}"
        );
    }

    /// A package id reaching PowerShell unescaped would end the quoted string.
    /// Nothing in the catalogue contains a quote today; this is what keeps that
    /// from becoming an injection the day something does.
    #[test]
    fn a_quote_in_an_argument_cannot_end_the_powershell_string() {
        let (_, args) = elevate(&windows(), "winget", &["a'; rm -rf /; '".to_string()]);
        let script = args.join(" ");

        assert!(
            script.contains("a''; rm -rf /; ''"),
            "single quotes must be doubled, got {script}"
        );
    }
}
