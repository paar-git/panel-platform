//! Deciding what a project's processes should do, without doing any of it.
//!
//! A project used to be one command. Now it is several — an API, a web dev
//! server, a worker — and something has to decide which starts next, what a
//! crash means for the others, and what the project's status is when its
//! processes disagree. That is this module, and it is deliberately the only
//! place those three questions are answered.
//!
//! # Why it does nothing
//!
//! No tokio, no filesystem, no database, no clock, no process. It takes the
//! shape of the process list and one event, and returns a list of actions for
//! somebody else to carry out. Every rule that matters is therefore testable
//! by calling a function, which is the whole reason the rules live here rather
//! than inside the async driver that executes them.
//!
//! This also keeps the crate's existing boundary: `host-runner` depends on
//! neither the wire format nor the database, so [`ProjectStatus`] carries the
//! status word as a string rather than importing `api-types`' enum. `app-core`
//! has the test that the two agree.
//!
//! **Verified by unit tests only.** No process has been started by this
//! module, because it cannot start one.

/// Where one process is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Not started yet, because an earlier process has not finished coming up.
    Pending,
    Starting,
    Running,
    /// Exited, and the supervisor is going to try again.
    Restarting,
    Stopping,
    Stopped,
    /// Exited for good, after having run.
    Crashed,
    /// Never ran at all.
    Failed,
}

impl ProcessState {
    /// The wire value, matching the `CHECK` on `project_processes.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            // A process waiting its turn has not been started, and `STOPPED`
            // is what the column's default already says about it.
            Self::Pending | Self::Stopped => "STOPPED",
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Restarting => "RESTARTING",
            Self::Stopping => "STOPPING",
            Self::Crashed => "CRASHED",
            Self::Failed => "FAILED",
        }
    }

    fn is_finished(self) -> bool {
        matches!(self, Self::Stopped | Self::Crashed | Self::Failed)
    }
}

/// What actually happened, reported by whoever was watching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The process survived its settle window.
    Settled { index: usize },
    /// The process's health check passed for the first time.
    Healthy { index: usize },
    /// The process exited. `terminal` is the supervisor's answer to "am I
    /// going to try again", which depends on the restart policy and on how
    /// many attempts have already been made — a question this module
    /// deliberately does not own.
    Exited {
        index: usize,
        code: Option<i32>,
        terminal: bool,
    },
    /// Somebody asked the project to stop.
    StopRequested,
    /// An install or build step failed, so this process never ran.
    PrepareFailed { index: usize, reason: String },
}

/// What the driver should do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Spawn {
        index: usize,
    },
    Stop {
        index: usize,
    },
    WriteProcessStatus {
        index: usize,
        state: ProcessState,
        reason: Option<String>,
    },
    WriteProjectStatus {
        status: ProjectStatus,
    },
}

/// One process, as a decision depends on it. Deliberately not the database
/// row: nothing here needs the command or the working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    /// Whether coming up means "survived the settle window" or "reported
    /// healthy". A process with a health check has not started until it
    /// passes, which is what lets an API gate the web server that calls it.
    pub has_health_check: bool,
}

/// The project's status, in the vocabulary `projects.status` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectStatus(pub &'static str);

impl ProjectStatus {
    pub const RUNNING: Self = Self("RUNNING");
    pub const STARTING: Self = Self("STARTING");
    pub const RESTARTING: Self = Self("RESTARTING");
    pub const CRASHED: Self = Self("CRASHED");
    pub const FAILED: Self = Self("FAILED");
    pub const STOPPED: Self = Self("STOPPED");

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// The processes of one project, and where each of them is.
#[derive(Debug, Clone)]
pub struct Machine {
    processes: Vec<Process>,
    states: Vec<ProcessState>,
    /// Whether a stop was asked for, so that a process exiting during one is
    /// not mistaken for a crash.
    stopping: bool,
}

impl Machine {
    pub fn new(processes: Vec<Process>) -> Self {
        let states = vec![ProcessState::Stopped; processes.len()];
        Self {
            processes,
            states,
            stopping: false,
        }
    }

    /// Begin a start: everything waits, and the first process goes.
    pub fn start(&mut self) -> Vec<Action> {
        self.stopping = false;
        self.states.fill(ProcessState::Pending);

        let mut actions = Vec::new();
        if self.processes.is_empty() {
            return actions;
        }

        self.set(0, ProcessState::Starting, None, &mut actions);
        actions.push(Action::Spawn { index: 0 });
        actions.push(Action::WriteProjectStatus {
            status: self.project_status(),
        });
        actions
    }

    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        let mut actions = Vec::new();

        match event {
            Event::Settled { index } => {
                // A process with a health check has not started merely by
                // surviving; `Healthy` is what advances it.
                if self.has_health_check(index) {
                    return actions;
                }
                self.came_up(index, &mut actions);
            }
            Event::Healthy { index } => {
                if !self.has_health_check(index) {
                    return actions;
                }
                self.came_up(index, &mut actions);
            }
            Event::Exited {
                index,
                code: _,
                terminal,
            } => {
                if self.stopping {
                    self.set(index, ProcessState::Stopped, None, &mut actions);
                } else if !terminal {
                    // The supervisor will try again. One process restarting is
                    // not a reason to tear down the ones that are working.
                    self.set(index, ProcessState::Restarting, None, &mut actions);
                } else {
                    // Having run and then died is a different situation from
                    // never having run, and the two want different answers
                    // from the user. The column keeps them apart.
                    let ran = matches!(
                        self.state(index),
                        Some(ProcessState::Running) | Some(ProcessState::Restarting)
                    );
                    let state = if ran {
                        ProcessState::Crashed
                    } else {
                        ProcessState::Failed
                    };
                    self.set(index, state, None, &mut actions);
                    actions.extend(self.stop_others(Some(index)));
                }
            }
            Event::StopRequested => {
                self.stopping = true;
                actions.extend(self.stop_others(None));
            }
            Event::PrepareFailed { index, reason } => {
                // Nothing was spawned, so there is nothing to have crashed.
                self.set(index, ProcessState::Failed, Some(reason), &mut actions);
                actions.extend(self.stop_others(Some(index)));
            }
        }

        actions.push(Action::WriteProjectStatus {
            status: self.project_status(),
        });
        actions
    }

    pub fn state(&self, index: usize) -> Option<ProcessState> {
        self.states.get(index).copied()
    }

    /// The project's status, from its processes.
    ///
    /// Read top to bottom, first match wins. The order is the point: a project
    /// with one process restarting and another running is `RESTARTING`, not
    /// `RUNNING`, because reporting it as running would hide the thing the
    /// user needs to see.
    pub fn project_status(&self) -> ProjectStatus {
        if self.states.is_empty() {
            return ProjectStatus::STOPPED;
        }
        if self.any(ProcessState::Failed) {
            return ProjectStatus::FAILED;
        }
        if self.any(ProcessState::Crashed) {
            return ProjectStatus::CRASHED;
        }
        if self.any(ProcessState::Restarting) {
            return ProjectStatus::RESTARTING;
        }
        if self
            .states
            .iter()
            .any(|state| matches!(state, ProcessState::Pending | ProcessState::Starting))
        {
            return ProjectStatus::STARTING;
        }
        if self.states.iter().all(|s| *s == ProcessState::Running) {
            return ProjectStatus::RUNNING;
        }
        if self.any(ProcessState::Stopping) {
            return ProjectStatus::STARTING;
        }
        ProjectStatus::STOPPED
    }

    /// A process finished coming up: mark it, and let the next one go.
    fn came_up(&mut self, index: usize, actions: &mut Vec<Action>) {
        if self.stopping {
            return;
        }
        self.set(index, ProcessState::Running, None, actions);

        if let Some(next) = self
            .states
            .iter()
            .position(|state| *state == ProcessState::Pending)
        {
            self.set(next, ProcessState::Starting, None, actions);
            actions.push(Action::Spawn { index: next });
        }
    }

    /// Stop everything still up, in reverse order.
    ///
    /// Reverse because a process was started against the ones before it: the
    /// API that the web server calls should be the last thing to go, not the
    /// first. `except` is the process that has already died and so has nothing
    /// left to stop.
    fn stop_others(&mut self, except: Option<usize>) -> Vec<Action> {
        let mut actions = Vec::new();

        for index in (0..self.states.len()).rev() {
            if Some(index) == except {
                continue;
            }
            let Some(state) = self.state(index) else {
                continue;
            };
            if state.is_finished() {
                continue;
            }
            self.set(index, ProcessState::Stopping, None, &mut actions);
            actions.push(Action::Stop { index });
        }

        actions
    }

    fn set(
        &mut self,
        index: usize,
        state: ProcessState,
        reason: Option<String>,
        actions: &mut Vec<Action>,
    ) {
        let Some(slot) = self.states.get_mut(index) else {
            return;
        };
        *slot = state;
        actions.push(Action::WriteProcessStatus {
            index,
            state,
            reason,
        });
    }

    fn any(&self, state: ProcessState) -> bool {
        self.states.contains(&state)
    }

    fn has_health_check(&self, index: usize) -> bool {
        self.processes
            .get(index)
            .is_some_and(|process| process.has_health_check)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(count: usize) -> Machine {
        Machine::new(vec![
            Process {
                has_health_check: false
            };
            count
        ])
    }

    fn spawns(actions: &[Action]) -> Vec<usize> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Spawn { index } => Some(*index),
                _ => None,
            })
            .collect()
    }

    fn stops(actions: &[Action]) -> Vec<usize> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Stop { index } => Some(*index),
                _ => None,
            })
            .collect()
    }

    /// Every process up, so the interesting tests start from a working
    /// project rather than rebuilding one each time.
    fn three_running() -> Machine {
        let mut machine = plain(3);
        machine.start();
        machine.handle(Event::Settled { index: 0 });
        machine.handle(Event::Settled { index: 1 });
        machine.handle(Event::Settled { index: 2 });
        assert_eq!(machine.project_status(), ProjectStatus::RUNNING);
        machine
    }

    #[test]
    fn a_start_spawns_only_the_first_process() {
        let mut machine = plain(3);

        let actions = machine.start();

        assert_eq!(spawns(&actions), [0]);
        assert_eq!(machine.state(1), Some(ProcessState::Pending));
        assert_eq!(machine.state(2), Some(ProcessState::Pending));
        assert_eq!(machine.project_status(), ProjectStatus::STARTING);
    }

    #[test]
    fn settling_advances_to_the_next_process() {
        let mut machine = plain(2);
        machine.start();

        let actions = machine.handle(Event::Settled { index: 0 });

        assert_eq!(machine.state(0), Some(ProcessState::Running));
        assert_eq!(spawns(&actions), [1]);
    }

    #[test]
    fn a_process_with_a_health_check_gates_on_health_not_on_settling() {
        let mut machine = Machine::new(vec![
            Process {
                has_health_check: true,
            },
            Process {
                has_health_check: false,
            },
        ]);
        machine.start();

        let after_settle = machine.handle(Event::Settled { index: 0 });
        assert!(
            spawns(&after_settle).is_empty(),
            "surviving is not the same as being ready to be called"
        );

        let after_health = machine.handle(Event::Healthy { index: 0 });
        assert_eq!(spawns(&after_health), [1]);
    }

    #[test]
    fn the_last_process_coming_up_makes_the_project_running() {
        let mut machine = plain(2);
        machine.start();
        machine.handle(Event::Settled { index: 0 });

        let actions = machine.handle(Event::Settled { index: 1 });

        assert_eq!(machine.project_status(), ProjectStatus::RUNNING);
        assert!(actions.contains(&Action::WriteProjectStatus {
            status: ProjectStatus::RUNNING
        }));
    }

    #[test]
    fn a_project_with_no_processes_starts_nothing() {
        let mut machine = plain(0);
        let actions = machine.start();
        assert!(actions.is_empty());
        assert_eq!(machine.project_status(), ProjectStatus::STOPPED);
    }

    #[test]
    fn a_restart_leaves_siblings_alone() {
        let mut machine = three_running();

        let actions = machine.handle(Event::Exited {
            index: 1,
            code: Some(1),
            terminal: false,
        });

        assert_eq!(machine.state(1), Some(ProcessState::Restarting));
        assert_eq!(machine.state(0), Some(ProcessState::Running));
        assert_eq!(machine.state(2), Some(ProcessState::Running));
        assert!(
            stops(&actions).is_empty(),
            "one blip must not tear the project down"
        );
        assert_eq!(machine.project_status(), ProjectStatus::RESTARTING);
    }

    #[test]
    fn a_terminal_crash_stops_the_siblings_in_reverse_order() {
        let mut machine = three_running();

        let actions = machine.handle(Event::Exited {
            index: 0,
            code: Some(1),
            terminal: true,
        });

        assert_eq!(machine.state(0), Some(ProcessState::Crashed));
        assert_eq!(
            stops(&actions),
            [2, 1],
            "the process the others were started against goes last"
        );
        assert_eq!(machine.project_status(), ProjectStatus::CRASHED);
    }

    #[test]
    fn a_process_that_never_ran_fails_rather_than_crashes() {
        let mut machine = plain(2);
        machine.start();

        machine.handle(Event::Exited {
            index: 0,
            code: Some(127),
            terminal: true,
        });

        assert_eq!(machine.state(0), Some(ProcessState::Failed));
        assert_eq!(
            machine.project_status(),
            ProjectStatus::FAILED,
            "`the runtime is missing` and `your code threw` want different answers"
        );
    }

    #[test]
    fn a_process_that_ran_and_then_died_crashes() {
        let mut machine = three_running();

        machine.handle(Event::Exited {
            index: 2,
            code: Some(1),
            terminal: true,
        });

        assert_eq!(machine.state(2), Some(ProcessState::Crashed));
        assert_eq!(machine.project_status(), ProjectStatus::CRASHED);
    }

    #[test]
    fn a_restart_that_then_gives_up_still_counts_as_having_run() {
        let mut machine = three_running();

        machine.handle(Event::Exited {
            index: 1,
            code: Some(1),
            terminal: false,
        });
        machine.handle(Event::Exited {
            index: 1,
            code: Some(1),
            terminal: true,
        });

        assert_eq!(
            machine.state(1),
            Some(ProcessState::Crashed),
            "a process that ran, then crash-looped, has still run"
        );
    }

    #[test]
    fn a_stop_request_stops_everything_in_reverse_order() {
        let mut machine = three_running();

        let actions = machine.handle(Event::StopRequested);

        assert_eq!(stops(&actions), [2, 1, 0]);
    }

    #[test]
    fn a_process_exiting_during_a_stop_is_not_a_crash() {
        let mut machine = three_running();
        machine.handle(Event::StopRequested);

        machine.handle(Event::Exited {
            index: 2,
            code: Some(0),
            terminal: true,
        });

        assert_eq!(
            machine.state(2),
            Some(ProcessState::Stopped),
            "it exited because it was asked to"
        );
    }

    #[test]
    fn everything_stopped_leaves_the_project_stopped() {
        let mut machine = three_running();
        machine.handle(Event::StopRequested);

        for index in (0..3).rev() {
            machine.handle(Event::Exited {
                index,
                code: Some(0),
                terminal: true,
            });
        }

        assert_eq!(machine.project_status(), ProjectStatus::STOPPED);
    }

    #[test]
    fn prepare_failing_fails_the_project_without_spawning_anything() {
        let mut machine = plain(1);

        let actions = machine.handle(Event::PrepareFailed {
            index: 0,
            reason: "npm install exited 1".to_string(),
        });

        assert_eq!(machine.project_status(), ProjectStatus::FAILED);
        assert!(spawns(&actions).is_empty());
        assert!(actions.contains(&Action::WriteProcessStatus {
            index: 0,
            state: ProcessState::Failed,
            reason: Some("npm install exited 1".to_string()),
        }));
    }

    #[test]
    fn an_event_for_a_process_that_does_not_exist_is_ignored() {
        // The driver and the machine index the same list, but a stale event
        // arriving after a process set changed must not panic — the workspace
        // denies indexing that can.
        let mut machine = plain(1);
        machine.start();

        let actions = machine.handle(Event::Settled { index: 7 });

        assert_eq!(machine.state(0), Some(ProcessState::Starting));
        assert!(spawns(&actions).is_empty());
    }

    /// The state machine's words and the column's `CHECK` are two lists of the
    /// same values, and two lists drift.
    #[test]
    fn every_state_has_a_wire_value_the_schema_allows() {
        let allowed = [
            "STOPPED",
            "STARTING",
            "RUNNING",
            "RESTARTING",
            "STOPPING",
            "CRASHED",
            "FAILED",
        ];

        for state in [
            ProcessState::Pending,
            ProcessState::Starting,
            ProcessState::Running,
            ProcessState::Restarting,
            ProcessState::Stopping,
            ProcessState::Stopped,
            ProcessState::Crashed,
            ProcessState::Failed,
        ] {
            assert!(
                allowed.contains(&state.as_str()),
                "{state:?} writes {}, which the CHECK refuses",
                state.as_str()
            );
        }
    }
}
