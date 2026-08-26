//! Making a decision take effect.
//!
//! [`policy`](crate::policy) decides and explains; this module is the only
//! place that changes anything. Two levers, and deliberately only two:
//!
//! * **holding off sleep**, through each platform's intended mechanism; and
//! * **process scheduling priority**, which changes what the operating system
//!   does when two things want the processor at once.
//!
//! # What is not here, and why
//!
//! There is no power-plan or power-overlay lever. `powercfg` on Windows can set
//! an active scheme, and it would have been the shortest way to make a
//! "Performance" button look like it did something — but a machine on a custom
//! or vendor-managed plan either ignores it or has its user's own configuration
//! silently replaced. Both outcomes are worse than not having the control:
//! one is a lie, the other is damage. The same reasoning rules out touching
//! clocks, voltages or power limits, which belong to a hardware utility rather
//! than to something that hosts projects.
//!
//! Neither lever here can terminate a workload. That is the property that makes
//! them the right two: the worst a wrong decision can do is leave a project
//! scheduled lower than ideal, which is slow, not dead.
//!
//! # Why priority is set by running a command
//!
//! Changing another process's priority is a platform API call on every
//! operating system, and this workspace forbids `unsafe`. The alternative to
//! shelling out would be a dependency that wraps those calls; each one carries
//! more surface than the two commands below, for a change that happens at most
//! once every few minutes per project. `renice` and PowerShell's
//! `PriorityClass` are the platforms' own supported interfaces to exactly this.

use std::process::Stdio;

use crate::policy::Profile;

/// What a project asked to be worth when the machine is busy.
///
/// Stored per project as the `priority` column, which is why the words match
/// the database's `CHECK` constraint rather than any operating system's names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Yield to everything else. For a project whose slowness nobody notices.
    Low,
    #[default]
    Normal,
    /// Keep this one responsive when something has to give.
    High,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "LOW",
            Priority::Normal => "NORMAL",
            Priority::High => "HIGH",
        }
    }

    /// Read the stored word.
    ///
    /// Unrecognised text is `Normal` rather than an error. The column is
    /// constrained to three values, so anything else means the row was written
    /// by a future version — and refusing to schedule a project because its
    /// priority is a word this build has not heard of would turn a cosmetic
    /// mismatch into an outage.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_uppercase().as_str() {
            "LOW" => Priority::Low,
            "HIGH" => Priority::High,
            _ => Priority::Normal,
        }
    }
}

/// What a project should actually be scheduled at, given the profile in force.
///
/// The profile shades the project's own setting rather than replacing it: a
/// project the user marked `High` is still the one that should suffer last on
/// an efficiency profile, even though everything moves down together.
///
/// `High` is never *granted* by a profile — only kept. Performance raising
/// every project to `High` would mean the application competing with the
/// window server and the user's editor for the processor, which is how a
/// background tool becomes the reason a machine feels broken.
pub fn effective(priority: Priority, profile: Profile) -> Priority {
    match profile {
        Profile::Performance => priority,
        Profile::Balanced => match priority {
            Priority::High => Priority::High,
            _ => Priority::Normal,
        },
        Profile::Efficiency => match priority {
            Priority::High => Priority::Normal,
            _ => Priority::Low,
        },
    }
}

/// The command that sets `pid` to `priority` on this platform.
///
/// Separated from running it so the shape of the command is a test rather than
/// something only observable by changing a real process's priority and asking
/// the operating system whether it worked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Windows: `PriorityClass` on a `Process` object.
///
/// `AboveNormal` rather than `High` for the top setting. `High` on Windows
/// outranks most of the desktop, and a hosted project that outranks the shell
/// makes the whole machine stutter to keep one bot responsive.
#[cfg(windows)]
pub fn priority_command(pid: u32, priority: Priority) -> PriorityCommand {
    let class = match priority {
        Priority::Low => "BelowNormal",
        Priority::Normal => "Normal",
        Priority::High => "AboveNormal",
    };
    PriorityCommand {
        program: "powershell.exe".to_string(),
        args: vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-WindowStyle".to_string(),
            "Hidden".to_string(),
            "-Command".to_string(),
            format!("(Get-Process -Id {pid}).PriorityClass = '{class}'"),
        ],
    }
}

/// Unix: `renice`, in the direction the niceness scale actually runs — a
/// *higher* number is a *lower* priority.
///
/// `High` asks for niceness 0 and not a negative number. Lowering niceness
/// needs privilege this application does not have and should not ask for, and a
/// command that fails every time on every machine is not a feature.
#[cfg(not(windows))]
pub fn priority_command(pid: u32, priority: Priority) -> PriorityCommand {
    let niceness = match priority {
        Priority::Low => "10",
        Priority::Normal => "0",
        Priority::High => "0",
    };
    PriorityCommand {
        program: "renice".to_string(),
        args: vec![
            "-n".to_string(),
            niceness.to_string(),
            "-p".to_string(),
            pid.to_string(),
        ],
    }
}

/// Why a priority change did not happen.
///
/// Every variant is survivable and none of them stop a project. A project
/// running at the wrong priority is running.
#[derive(Debug, thiserror::Error)]
pub enum PriorityError {
    #[error("could not run `{program}`: {source}")]
    NotRun {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{program}` exited with code {code}")]
    Refused { program: String, code: i64 },
}

/// Ask the operating system to schedule `pid` at `priority`.
///
/// Output is discarded rather than logged: the useful half is the exit status,
/// and PowerShell's error text for a process that exited a moment ago is four
/// lines of stack that say "no such process" at the end of it.
pub async fn apply_priority(pid: u32, priority: Priority) -> Result<(), PriorityError> {
    let command = priority_command(pid, priority);

    let mut spawned = tokio::process::Command::new(&command.program);
    spawned
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        // Same reason as the probe: a console-subsystem PowerShell otherwise
        // flashes a terminal every time a project's priority is applied.
        spawned.creation_flags(0x0800_0000);
    }
    let status = spawned
        .status()
        .await
        .map_err(|source| PriorityError::NotRun {
            program: command.program.clone(),
            source,
        })?;

    if status.success() {
        return Ok(());
    }

    Err(PriorityError::Refused {
        program: command.program,
        code: status.code().map_or(-1, i64::from),
    })
}

/// Holds automatic sleep off for as long as it exists.
///
/// One of these for the whole application rather than one per project. The
/// operating system is being told "somebody is relying on this machine", which
/// is not a claim that gets truer when five projects make it.
pub struct SleepHold {
    /// `None` when sleep is not being held, or when the platform refused —
    /// which is not an error worth propagating, because a machine that sleeps
    /// when it was asked not to is a worse experience, not a broken one.
    guard: Option<keepawake::KeepAwake>,
    held: bool,
}

// `keepawake::KeepAwake` is opaque and implements no `Debug`, so the derive
// cannot see through it. The workspace warns on missing `Debug`, and the honest
// thing to print is whether the hold is on.
impl std::fmt::Debug for SleepHold {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SleepHold")
            .field("held", &self.held)
            .finish()
    }
}

impl Default for SleepHold {
    fn default() -> Self {
        Self::new()
    }
}

impl SleepHold {
    pub fn new() -> Self {
        Self {
            guard: None,
            held: false,
        }
    }

    /// Whether sleep is currently being held off.
    pub fn held(&self) -> bool {
        self.held
    }

    /// Hold sleep off, or stop holding it.
    ///
    /// Idempotent, and that matters more than it looks: this is called from a
    /// timer every few seconds, and re-creating the platform's power assertion
    /// on every tick would churn a system-wide resource to express a state that
    /// has not changed. Returns whether anything actually changed, which is
    /// what the caller logs.
    pub fn set(&mut self, wanted: bool) -> bool {
        if wanted == self.held {
            return false;
        }

        if wanted {
            match keepawake::Builder::default()
                .idle(true)
                .sleep(true)
                .reason("A hosted project is running")
                .app_name("Panel")
                .app_reverse_domain("dev.realpargit.panel")
                .create()
            {
                Ok(guard) => {
                    self.guard = Some(guard);
                    self.held = true;
                    true
                }
                Err(error) => {
                    // Reported once, at the moment it fails, and then left
                    // alone. `held` stays false, so the next tick will try
                    // again rather than believing a hold it never got.
                    tracing::warn!(%error, "could not ask the system to stay awake");
                    false
                }
            }
        } else {
            // Dropping the guard is what releases the assertion.
            self.guard = None;
            self.held = false;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_stored_words_survive_a_round_trip() {
        for priority in [Priority::Low, Priority::Normal, Priority::High] {
            assert_eq!(Priority::parse(priority.as_str()), priority);
        }
    }

    /// A row written by a newer build must not stop the project running.
    #[test]
    fn an_unknown_priority_reads_as_normal_rather_than_failing() {
        assert_eq!(Priority::parse("URGENT"), Priority::Normal);
        assert_eq!(Priority::parse(""), Priority::Normal);
        assert_eq!(Priority::parse("low"), Priority::Low);
    }

    #[test]
    fn performance_leaves_every_project_where_the_user_put_it() {
        for priority in [Priority::Low, Priority::Normal, Priority::High] {
            assert_eq!(effective(priority, Profile::Performance), priority);
        }
    }

    /// The point of the setting: whatever the profile, a project the user
    /// marked High is never scheduled below one they did not.
    #[test]
    fn a_high_priority_project_is_never_overtaken_by_a_lower_one() {
        for profile in [Profile::Performance, Profile::Balanced, Profile::Efficiency] {
            let high = effective(Priority::High, profile);
            assert!(high >= effective(Priority::Normal, profile));
            assert!(high >= effective(Priority::Low, profile));
        }
    }

    #[test]
    fn efficiency_moves_everything_down_without_dropping_high_to_the_bottom() {
        assert_eq!(
            effective(Priority::High, Profile::Efficiency),
            Priority::Normal
        );
        assert_eq!(
            effective(Priority::Normal, Profile::Efficiency),
            Priority::Low
        );
        assert_eq!(effective(Priority::Low, Profile::Efficiency), Priority::Low);
    }

    /// No profile hands out `High`. Only a user does.
    #[test]
    fn no_profile_promotes_a_project_the_user_did_not_promote() {
        for profile in [Profile::Performance, Profile::Balanced, Profile::Efficiency] {
            assert_ne!(effective(Priority::Normal, profile), Priority::High);
            assert_ne!(effective(Priority::Low, profile), Priority::High);
        }
    }

    #[test]
    fn the_command_names_the_process_it_is_about() {
        let command = priority_command(4321, Priority::Low);
        assert!(
            command.args.iter().any(|arg| arg.contains("4321")),
            "the pid is not in {:?}",
            command.args
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_priority_changes_do_not_open_a_console() {
        let command = priority_command(1, Priority::Normal);
        assert_eq!(command.program, "powershell.exe");
        assert!(
            command
                .args
                .windows(2)
                .any(
                    |pair| pair.first().map(String::as_str) == Some("-WindowStyle")
                        && pair.get(1).map(String::as_str) == Some("Hidden")
                ),
            "priority changes would flash a PowerShell window: {:?}",
            command.args
        );
    }

    /// Nothing here may raise a process above the desktop it is running on.
    #[cfg(windows)]
    #[test]
    fn windows_never_asks_for_a_class_that_outranks_the_shell() {
        for priority in [Priority::Low, Priority::Normal, Priority::High] {
            let command = priority_command(1, priority);
            let script = command.args.join(" ");
            assert!(
                !script.contains("'High'") && !script.contains("RealTime"),
                "a class that outranks the desktop was asked for: {script}"
            );
        }
    }

    /// Negative niceness needs privilege this application does not have.
    #[cfg(not(windows))]
    #[test]
    fn unix_never_asks_for_a_niceness_it_cannot_be_granted() {
        for priority in [Priority::Low, Priority::Normal, Priority::High] {
            let command = priority_command(1, priority);
            assert!(
                !command.args.iter().any(|arg| arg.starts_with('-')
                    && arg.len() > 1
                    && arg[1..].chars().all(|character| character.is_ascii_digit())),
                "a negative niceness was asked for: {:?}",
                command.args
            );
        }
    }

    #[test]
    fn a_hold_that_is_already_held_is_not_taken_twice() {
        let mut hold = SleepHold::new();
        assert!(!hold.held());

        // The platform may refuse in a test environment with no session, which
        // is not a failure of this logic: what is being asserted is that a
        // repeated request never reports a second change.
        let first = hold.set(true);
        assert!(!hold.set(true), "asking twice reported a second change");

        if first {
            assert!(hold.held());
            assert!(hold.set(false), "releasing a held lock reported no change");
        }
        assert!(!hold.set(false), "releasing twice reported a second change");
        assert!(!hold.held());
    }

    /// A pid that cannot exist must come back as an error rather than a panic
    /// or a hang.
    #[tokio::test]
    async fn asking_about_a_process_that_is_not_there_fails_quietly() {
        let result = apply_priority(u32::MAX, Priority::Low).await;
        assert!(result.is_err(), "a nonexistent process reported success");
    }
}
