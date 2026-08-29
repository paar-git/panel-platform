//! Deciding whether this machine can run a project's language, and fixing it.
//!
//! The composition layer. `host-runner` knows how to look for a toolchain,
//! `toolchain` knows what to install and how, and neither knows about the
//! other. This joins them and is the only place that holds both.
//!
//! The split matters for the same reason it does in `provisioning`: the two
//! crates underneath are pure and exhaustively tested, so what is left here is
//! the joining, and the joining is small enough to read.

use std::path::{Path, PathBuf};

use project_host_host_runner::probe::{ExecutableResolver, Toolchain};
use project_host_platform::SystemSnapshot;
use project_host_toolchain::plan::{Host, ProjectInstall, Step};
use project_host_toolchain::refresh::{find_executable, merged_path, suffixes_for};
use project_host_toolchain::{plan, Blocker, Plan};

/// What pressing Start should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Start the project. Nothing is missing.
    Ready,
    /// Show the user what would be installed, and start only if they agree.
    NeedsInstall { steps: Vec<Step> },
    /// Nothing can be offered, and this says why.
    Blocked(Blocker),
}

/// Reduce a machine scan to what an install decision depends on.
///
/// Pure, so a Linux machine's host is derived correctly while running the tests
/// on Windows.
pub fn host_from_snapshot(snapshot: &SystemSnapshot, winget_present: bool) -> Host {
    // `windows` and `linux` are populated only on the platform they name, so
    // they identify the machine without parsing a display string. A scan that
    // filled neither is unknown rather than assumed to be either: guessing
    // would run a Windows plan against a Linux machine.
    match (&snapshot.windows, &snapshot.linux) {
        (Some(_), _) => Host::Windows { winget_present },
        (None, Some(linux)) => Host::Linux {
            manager: linux.package_manager,
        },
        (None, None) => Host::Unknown,
    }
}

/// Decide what Start should do for a project of `runtime`.
pub fn assess(
    runtime: &str,
    host: &Host,
    resolver: &dyn ExecutableResolver,
    project_install: Option<&ProjectInstall>,
) -> Readiness {
    // `STATIC` has no candidates, so probing reports Missing for it. That is
    // correct for `host-runner`, whose question is "can I start this", and
    // wrong here, whose question is "is anything to install" — the planner
    // answers Nothing for it, which is why the probe result is not the
    // decision.
    let present = matches!(
        project_host_host_runner::probe::probe(runtime, resolver),
        Toolchain::Found { .. }
    );

    match plan(runtime, present, host, project_install) {
        Plan::Nothing => Readiness::Ready,
        Plan::Install { steps } => Readiness::NeedsInstall { steps },
        Plan::Blocked(blocker) => Readiness::Blocked(blocker),
    }
}

/// A project's dependency install, if it has one that can be run.
///
/// An unparsable command yields `None` rather than an error: the toolchain is
/// still worth installing, and a project whose `install_command` cannot be read
/// fails later with a message about that command rather than about Node.
pub fn project_install_for(install_command: Option<&str>) -> Option<ProjectInstall> {
    let command = install_command?.trim();
    if command.is_empty() {
        return None;
    }

    let mut words = project_host_host_runner::split_command(command).ok()?;
    if words.is_empty() {
        return None;
    }

    let program = words.remove(0);
    Some(ProjectInstall {
        program,
        args: words,
    })
}

/// How far an approved install has got, for anything showing progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub step: usize,
    pub of: usize,
    pub describes: String,
}

/// Carry out an offer the user approved.
///
/// Runs every step in order, stopping at the first failure — a toolchain that
/// did not install must not be followed by a dependency install that will fail
/// for a reason the user cannot connect to it.
///
/// **Unverified.** No elevated session exists on the machine this was written
/// on, so the UAC and pkexec paths below have never been executed.
pub fn install(
    runtime: &str,
    steps: &[Step],
    host: &Host,
    report: &mut dyn FnMut(Progress),
) -> Result<(), Blocker> {
    let display_name = project_host_toolchain::spec_for(runtime)
        .map(|spec| spec.display_name.to_string())
        .unwrap_or_else(|| runtime.to_string());

    for (index, step) in steps.iter().enumerate() {
        report(Progress {
            step: index + 1,
            of: steps.len(),
            describes: step.describes.clone(),
        });

        // No display name is passed: each step names its own subject, so a
        // failed `npm install` is reported as the project's dependencies
        // rather than as the toolchain that installed without complaint.
        project_host_toolchain::execute::run(step, host)?;
    }

    // Success is finding the executable, never a zero exit code: the install
    // writes PATH into the registry and this process cannot see that change.
    let candidates: Vec<String> = project_host_host_runner::probe::candidates_for(runtime)
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    if candidates.is_empty() {
        // STATIC and POLYGLOT resolve to no executable, so there is nothing to
        // confirm and nothing was installed for them either.
        return Ok(());
    }

    project_host_toolchain::execute::confirm(&candidates, &display_name).map(|_| ())
}

/// Whether `winget` can be found, which decides if a Windows plan needs a
/// bootstrap step first.
pub fn winget_present() -> bool {
    MachineResolver.resolve("winget").is_some()
}

/// Resolves executables against this machine, using a `PATH` rebuilt from
/// where installers actually write rather than the one inherited at launch.
///
/// The reason this exists rather than a plain `PATH` walk: after an install in
/// this same session, the inherited copy is stale, and a probe that trusts it
/// reports working software as missing.
#[derive(Debug, Default)]
pub struct MachineResolver;

impl ExecutableResolver for MachineResolver {
    fn resolve(&self, name: &str) -> Option<PathBuf> {
        let windows = cfg!(windows);
        let directories = merged_path(
            None,
            None,
            &std::env::var("PATH").unwrap_or_default(),
            windows,
        );

        find_executable(&directories, name, suffixes_for(windows), &|path: &Path| {
            path.is_file()
        })
    }

    fn version(&self, executable: &Path) -> Option<String> {
        let output = std::process::Command::new(executable)
            .arg("--version")
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().next().map(|line| line.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use project_host_host_runner::probe::candidates_for;
    use project_host_platform::{LinuxInfo, PackageManager, WindowsInfo};

    #[derive(Debug)]
    struct FakeMachine {
        installed: Vec<&'static str>,
    }

    impl ExecutableResolver for FakeMachine {
        fn resolve(&self, name: &str) -> Option<PathBuf> {
            self.installed
                .contains(&name)
                .then(|| PathBuf::from(format!("/usr/bin/{name}")))
        }

        fn version(&self, _executable: &Path) -> Option<String> {
            Some("1.2.3".to_string())
        }
    }

    fn machine(installed: &[&'static str]) -> FakeMachine {
        FakeMachine {
            installed: installed.to_vec(),
        }
    }

    fn windows_snapshot() -> SystemSnapshot {
        let mut snapshot = SystemSnapshot::unknown();
        snapshot.windows = Some(WindowsInfo::default());
        snapshot
    }

    fn linux_snapshot(manager: Option<PackageManager>) -> SystemSnapshot {
        let mut snapshot = SystemSnapshot::unknown();
        snapshot.linux = Some(LinuxInfo {
            distro_id: Some("ubuntu".to_string()),
            version_id: Some("24.04".to_string()),
            package_manager: manager,
        });
        snapshot
    }

    #[test]
    fn a_windows_machine_carries_whether_winget_is_there() {
        assert_eq!(
            host_from_snapshot(&windows_snapshot(), false),
            Host::Windows {
                winget_present: false
            }
        );
    }

    #[test]
    fn a_linux_machine_carries_the_package_manager_the_scan_found() {
        assert_eq!(
            host_from_snapshot(&linux_snapshot(Some(PackageManager::Dnf)), false),
            Host::Linux {
                manager: Some(PackageManager::Dnf)
            }
        );
    }

    /// A scan that identified neither must not be treated as either. Guessing
    /// here would run a Windows plan against a Linux machine.
    #[test]
    fn a_machine_the_scan_could_not_identify_is_unknown() {
        assert_eq!(
            host_from_snapshot(&SystemSnapshot::unknown(), false),
            Host::Unknown
        );
    }

    /// The common path. A machine that already has Node must not be offered an
    /// install of it.
    #[test]
    fn a_machine_with_the_toolchain_is_ready() {
        let readiness = assess(
            "NODEJS",
            &Host::Windows {
                winget_present: true,
            },
            &machine(&["node"]),
            None,
        );

        assert_eq!(readiness, Readiness::Ready);
    }

    #[test]
    fn a_machine_without_the_toolchain_is_offered_an_install_naming_it() {
        let readiness = assess(
            "NODEJS",
            &Host::Windows {
                winget_present: true,
            },
            &machine(&[]),
            None,
        );

        match readiness {
            Readiness::NeedsInstall { steps } => assert!(steps
                .iter()
                .any(|step| step.args.contains(&"OpenJS.NodeJS.LTS".to_string()))),
            other => panic!("expected an install offer, got {other:?}"),
        }
    }

    /// `python3` is preferred, but a machine with only `python` already has
    /// what the project needs and must not be given a second one.
    #[test]
    fn a_machine_with_the_fallback_candidate_is_still_ready() {
        let readiness = assess(
            "PYTHON",
            &Host::Windows {
                winget_present: true,
            },
            &machine(&["python"]),
            None,
        );

        assert_eq!(readiness, Readiness::Ready);
    }

    #[test]
    fn a_static_site_is_ready_on_a_machine_with_nothing_installed() {
        let readiness = assess(
            "STATIC",
            &Host::Windows {
                winget_present: true,
            },
            &machine(&[]),
            None,
        );

        assert_eq!(readiness, Readiness::Ready);
    }

    #[test]
    fn a_blocked_machine_reports_the_blocker_rather_than_an_offer() {
        let readiness = assess("POLYGLOT", &Host::Unknown, &machine(&[]), None);

        assert_eq!(readiness, Readiness::Blocked(Blocker::PolyglotUnresolvable));
    }

    #[test]
    fn a_projects_install_command_is_split_into_a_program_and_arguments() {
        assert_eq!(
            project_install_for(Some("npm ci --omit=dev")),
            Some(ProjectInstall {
                program: "npm".to_string(),
                args: vec!["ci".to_string(), "--omit=dev".to_string()],
            })
        );
    }

    #[test]
    fn a_project_with_no_install_command_contributes_no_step() {
        assert_eq!(project_install_for(None), None);
        assert_eq!(project_install_for(Some("   ")), None);
    }

    /// The toolchain is still worth installing, so an unreadable command must
    /// not take the whole offer down with it.
    #[test]
    fn an_unparsable_install_command_is_skipped_rather_than_fatal() {
        assert_eq!(project_install_for(Some("npm install \"unclosed")), None);
    }

    /// Candidates come from `host-runner`, so a runtime it can probe and a
    /// runtime this can install must stay the same set. A mismatch would show
    /// up as an offer for something that is already there, or none for
    /// something that is not.
    #[test]
    fn every_runtime_with_probe_candidates_can_be_assessed() {
        let host = Host::Windows {
            winget_present: true,
        };

        for spec in project_host_toolchain::catalog() {
            assert!(
                !candidates_for(spec.runtime).is_empty(),
                "{} can be installed but not probed for",
                spec.runtime
            );

            assert!(
                !matches!(
                    assess(spec.runtime, &host, &machine(&[]), None),
                    Readiness::Blocked(_)
                ),
                "{} is installable and yet blocked on Windows",
                spec.runtime
            );
        }
    }
}
