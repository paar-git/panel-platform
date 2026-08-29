//! Running a plan. The only file in this crate that touches the machine.
//!
//! Everything worth testing was decided before control reached here: which
//! packages, in what order, wrapped in which elevation. What remains is
//! spawning a process and reading an exit code, and the exit codes are the part
//! that matters — a dismissed prompt and a failed install must not arrive as
//! the same outcome.
//!
//! **Unverified.** This host has no Linux and no elevated session, so neither
//! the `pkexec` path nor a real UAC prompt has been executed. The mapping below
//! is written from the documented codes, not from having watched them.

use std::path::PathBuf;
use std::process::Command;

use crate::blocker::Blocker;
use crate::plan::{elevate, Host, Step};
use crate::refresh::{find_executable, merged_path, suffixes_for};

/// `ERROR_CANCELLED`: the user dismissed the UAC prompt.
const WINDOWS_CANCELLED: i32 = 1223;
/// pkexec's own codes for "dismissed" and "not available".
const PKEXEC_DISMISSED: i32 = 126;
const PKEXEC_UNAVAILABLE: i32 = 127;

/// Run one step, elevating it if the plan said to.
///
/// What the user is told about is [`Step::subject`] — the thing this step
/// installs, not the shell that wrapped it: "Installing Node.js failed" is
/// useful where "powershell.exe exited 1" is not. It is carried per step
/// rather than passed in for the whole plan, because a plan's last step
/// installs the *project's* dependencies and reporting that failure as
/// "Installing Node.js failed" names software that installed perfectly.
pub fn run(step: &Step, host: &Host) -> Result<(), Blocker> {
    let (program, args) = if step.elevated {
        elevate(host, &step.program, &step.args)
    } else {
        (step.program.clone(), step.args.clone())
    };

    // `CreateProcess` searches `PATH` appending only `.exe`, so a bare `npm` —
    // which on Windows exists solely as `npm.cmd` — is never found, on a
    // machine that has npm. `host_runner::command` resolves the program before
    // spawning for exactly this reason. Resolution is also what makes an
    // install in *this* session visible: the search path is rebuilt from the
    // registry, which the copy this process inherited is stale relative to.
    //
    // A name that resolves to nothing is still handed to the operating system
    // rather than refused here: resolution walks directories, and a program
    // reachable some way this does not model should not be blocked by our
    // failure to find it. The error below is what reports it if the OS agrees.
    let spawnable = resolve(&program).unwrap_or_else(|| PathBuf::from(&program));

    let mut command = Command::new(&spawnable);
    command.args(&args);
    project_host_platform::hide_console(&mut command);
    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            // Nothing ran, so there is no exit code and no output to quote.
            Blocker::ProgramNotFound {
                display_name: step.subject.clone(),
                program: step.program.clone(),
            }
        } else {
            Blocker::StepFailed {
                display_name: step.subject.clone(),
                program: step.program.clone(),
                code: -1,
                output: error.to_string(),
            }
        }
    })?;

    match output.status.code() {
        Some(0) | None => Ok(()),

        Some(code)
            if code == WINDOWS_CANCELLED
                || code == PKEXEC_DISMISSED
                || code == PKEXEC_UNAVAILABLE =>
        {
            Err(Blocker::NotAuthorised {
                display_name: step.subject.clone(),
            })
        }

        Some(code) => Err(Blocker::StepFailed {
            display_name: step.subject.clone(),
            program: step.program.clone(),
            code,
            output: last_lines(&output.stderr, &output.stdout),
        }),
    }
}

/// Turn a program name into a path this machine can actually spawn.
///
/// `None` for a name that is already a path — the caller wrote it, and
/// resolving it against `PATH` would be ignoring what they said — and for a
/// bare name nothing was found for.
fn resolve(program: &str) -> Option<PathBuf> {
    if program.contains('/') || program.contains('\\') {
        return None;
    }

    let windows = cfg!(windows);
    find_executable(
        &search_path(windows),
        program,
        suffixes_for(windows),
        &|path| path.is_file(),
    )
}

/// Where to look for an executable: the registry's `PATH` ahead of the stale
/// copy this process inherited at launch.
fn search_path(windows: bool) -> Vec<PathBuf> {
    merged_path(
        machine_path().as_deref(),
        user_path().as_deref(),
        &std::env::var("PATH").unwrap_or_default(),
        windows,
    )
}

/// Confirm an install by finding the executable, against a `PATH` rebuilt from
/// where the installer wrote it rather than the one this process inherited.
///
/// Returns [`Blocker::StillMissingAfterInstall`] rather than a failure: the
/// install did work, and telling the user otherwise sends them to reinstall
/// software they have.
pub fn confirm(candidates: &[String], display_name: &str) -> Result<PathBuf, Blocker> {
    let windows = cfg!(windows);
    let directories = search_path(windows);

    for name in candidates {
        if let Some(found) = find_executable(&directories, name, suffixes_for(windows), &|path| {
            path.is_file()
        }) {
            return Ok(found);
        }
    }

    Err(Blocker::StillMissingAfterInstall {
        display_name: display_name.to_string(),
        executable: candidates.join(", "),
    })
}

/// The machine `PATH` as the registry holds it, which is where an installer
/// writes and what this process's copy is stale relative to.
///
/// Read through PowerShell rather than a registry crate because
/// `unsafe_code` is forbidden workspace-wide and this needs no new dependency
/// to be correct. `None` on any failure — the caller still has the inherited
/// path, which is better than nothing.
#[cfg(windows)]
fn machine_path() -> Option<String> {
    read_environment("Machine")
}

#[cfg(windows)]
fn user_path() -> Option<String> {
    read_environment("User")
}

#[cfg(windows)]
fn read_environment(scope: &str) -> Option<String> {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &format!("[Environment]::GetEnvironmentVariable('Path','{scope}')"),
    ]);
    project_host_platform::hide_console(&mut command);
    let output = command.output().ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// On Linux a package manager writes into directories that are already on
/// `PATH`; there is no second copy to consult.
#[cfg(not(windows))]
fn machine_path() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn user_path() -> Option<String> {
    None
}

/// The tail of a failed command's output, which is where package managers put
/// the reason. The whole log would be unreadable in a dialog.
fn last_lines(stderr: &[u8], stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(if stderr.is_empty() { stdout } else { stderr });

    text.lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason a package manager gives is on the last lines, and the whole
    /// log would not fit in a dialog.
    #[test]
    fn a_failure_report_keeps_the_end_of_the_output_where_the_reason_is() {
        let stderr = b"resolving\n\nfetching\nE: Unable to locate package nodejs\n";

        assert_eq!(
            last_lines(stderr, b""),
            "resolving fetching E: Unable to locate package nodejs"
        );
    }

    #[test]
    fn stdout_is_used_when_a_command_failed_without_writing_to_stderr() {
        assert_eq!(last_lines(b"", b"No package found"), "No package found");
    }

    #[test]
    fn only_the_last_three_lines_are_kept() {
        assert_eq!(last_lines(b"a\nb\nc\nd\ne", b""), "c d e");
    }

    /// The defect a user actually met, reported as "Installing Node.js (for
    /// TypeScript) failed: npm exited with code -1. program not found".
    ///
    /// On Windows `npm` is `npm.cmd`, and `CreateProcess`'s own `PATH` search
    /// only ever appends `.exe` — so a step naming a bare `npm` never starts,
    /// on a machine that has npm, on the `PATH` this process inherited.
    /// `host_runner::command` resolves the program before spawning for exactly
    /// this reason; this crate did not.
    #[cfg(windows)]
    #[test]
    fn a_step_whose_program_is_a_cmd_actually_runs() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let script = directory.path().join("panel-cmd-probe.cmd");
        std::fs::write(&script, "@echo off\r\nexit /b 0\r\n").expect("write the script");

        // Prepending rather than replacing: every other directory stays
        // reachable, so this cannot break a test running beside it.
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{};{path}", directory.path().to_string_lossy()),
        );

        let step = Step {
            elevated: false,
            program: "panel-cmd-probe".to_string(),
            args: Vec::new(),
            subject: "the project's own dependencies".to_string(),
            describes: "Run the probe".to_string(),
        };

        let result = run(
            &step,
            &Host::Windows {
                winget_present: true,
            },
        );

        std::env::set_var("PATH", path);

        assert!(
            result.is_ok(),
            "a program that exists only as a .cmd must still run, got {result:?}"
        );
    }

    /// A program that never started did not exit, so it has no exit code. The
    /// old report invented -1 and read "npm exited with code -1. program not
    /// found" — a sentence that contradicts itself, and sends the user looking
    /// for a failing installer rather than a missing program.
    #[test]
    fn a_program_that_cannot_be_started_is_not_reported_as_having_exited() {
        let step = Step {
            elevated: false,
            program: "panel-no-such-program-anywhere".to_string(),
            args: Vec::new(),
            subject: "the project's own dependencies".to_string(),
            describes: "Run something that is not there".to_string(),
        };

        let result = run(&step, &Host::Linux { manager: None });

        match result {
            Err(Blocker::ProgramNotFound {
                program,
                display_name,
            }) => {
                assert_eq!(program, "panel-no-such-program-anywhere");
                assert_eq!(display_name, "the project's own dependencies");
            }
            other => panic!("expected ProgramNotFound, got {other:?}"),
        }
    }

    /// Confirmation must never claim to have found something on a machine that
    /// has nothing, which is what a candidate list that is empty would do.
    #[test]
    fn confirming_nothing_is_a_failure_rather_than_a_success() {
        let result = confirm(&[], "Node.js");

        assert!(matches!(
            result,
            Err(Blocker::StillMissingAfterInstall { .. })
        ));
    }
}
