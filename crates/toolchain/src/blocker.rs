//! Why this machine cannot be given the toolchain a project needs.
//!
//! Every variant names concrete values. "Installation failed", without the
//! package and the exit code, is the failure mode this design exists to
//! replace.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Blocker {
    #[error(
        "{platform} installs software through a package manager, and none was \
         found. {remedy}"
    )]
    NoPackageManager { platform: String, remedy: String },

    #[error(
        "{display_name} is not published for {platform} through a package \
         manager. Install it from {vendor} and start the project again."
    )]
    NotPackagedForPlatform {
        display_name: String,
        platform: String,
        vendor: String,
    },

    #[error("There is no toolchain to install for a project of type {runtime}.")]
    RuntimeUnsupported { runtime: String },

    #[error(
        "This project declares several languages, which cannot be resolved to \
         one toolchain. Install what it needs by hand, or run it in a \
         container, where the image carries them."
    )]
    PolyglotUnresolvable,

    #[error(
        "Installing {display_name} needs your permission, and the request was \
         dismissed. Nothing was changed."
    )]
    NotAuthorised { display_name: String },

    #[error("Installing {display_name} failed: {program} exited with code {code}. {output}")]
    StepFailed {
        display_name: String,
        program: String,
        code: i32,
        output: String,
    },

    /// The program never started, so it never exited and has no exit code.
    ///
    /// Distinct from [`Blocker::StepFailed`] because the two send the user to
    /// different places: a failed install means reading an installer's output,
    /// a missing program means installing that program. Reporting this as a
    /// step that "exited with code -1" produced a sentence that contradicted
    /// itself, and named an installer that had in fact never run.
    #[error(
        "Installing {display_name} needs `{program}`, which is not a program on \
         this machine. Nothing was run. Install it, or change the command that \
         asks for it."
    )]
    ProgramNotFound {
        display_name: String,
        program: String,
    },

    #[error(
        "{display_name} was installed, but {executable} is still not on this \
         program's PATH. Restart Panel Platform and start the project again."
    )]
    StillMissingAfterInstall {
        display_name: String,
        executable: String,
    },

    #[error(
        "Could not determine which operating system this is, so no installer \
         can be matched to it."
    )]
    HostUnrecognised,
}

impl Blocker {
    /// Whether the user can do something about this on this machine.
    ///
    /// Drives whether the interface offers a retry. Offering one for a runtime
    /// that has no installer sends the user round a loop that cannot
    /// terminate.
    pub fn is_fixable(&self) -> bool {
        match self {
            Blocker::NoPackageManager { .. }
            | Blocker::NotAuthorised { .. }
            | Blocker::StepFailed { .. }
            | Blocker::ProgramNotFound { .. }
            | Blocker::StillMissingAfterInstall { .. }
            | Blocker::NotPackagedForPlatform { .. } => true,
            Blocker::RuntimeUnsupported { .. }
            | Blocker::PolyglotUnresolvable
            | Blocker::HostUnrecognised => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dismissed prompt is not a failed install, and telling them apart is
    /// the difference between "try again" and "something is wrong". The same
    /// distinction `setup::handoff` draws for pkexec.
    #[test]
    fn a_dismissed_prompt_reads_differently_from_a_failure() {
        let declined = Blocker::NotAuthorised {
            display_name: "Node.js".to_string(),
        };
        let failed = Blocker::StepFailed {
            display_name: "Node.js".to_string(),
            program: "winget".to_string(),
            code: 1,
            output: String::new(),
        };

        assert_ne!(declined.to_string(), failed.to_string());
        assert!(declined.to_string().contains("permission"));
        assert!(
            declined.to_string().contains("Nothing was changed"),
            "a dismissed prompt must say the machine is untouched"
        );
    }

    /// The exit code is what makes a failure report actionable rather than a
    /// complaint.
    #[test]
    fn a_failed_step_names_the_program_and_the_code() {
        let message = Blocker::StepFailed {
            display_name: "Python 3".to_string(),
            program: "winget".to_string(),
            code: -1978335212,
            output: "No package found matching input criteria".to_string(),
        }
        .to_string();

        assert!(message.contains("winget"));
        assert!(message.contains("-1978335212"));
        assert!(message.contains("No package found"));
    }

    /// The install worked; only this process's inherited PATH is stale. Telling
    /// the user it failed would send them to reinstall software they have.
    #[test]
    fn a_stale_path_tells_the_user_to_restart_rather_than_reinstall() {
        let message = Blocker::StillMissingAfterInstall {
            display_name: "Node.js".to_string(),
            executable: "node".to_string(),
        }
        .to_string();

        assert!(message.contains("Restart"));
        assert!(
            !message.contains("failed"),
            "a successful install must not be reported as a failure"
        );
    }

    #[test]
    fn a_runtime_with_no_installer_is_not_offered_as_something_to_retry() {
        assert!(!Blocker::PolyglotUnresolvable.is_fixable());
        assert!(!Blocker::RuntimeUnsupported {
            runtime: "STATIC".to_string()
        }
        .is_fixable());
        assert!(Blocker::NotAuthorised {
            display_name: "Go".to_string()
        }
        .is_fixable());
    }

    #[test]
    fn a_blocker_round_trips_through_json() {
        let blocker = Blocker::StillMissingAfterInstall {
            display_name: "Go".to_string(),
            executable: "go".to_string(),
        };
        let json = serde_json::to_string(&blocker).expect("serialise");
        let back: Blocker = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, blocker);
    }
}
