//! Measuring what the machine and its projects are actually using.

use serde::{Deserialize, Serialize};

/// What one project is using right now.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    pub memory_bytes: u64,
    /// Percentage of one core. `None` on the first sample: `sysinfo` derives
    /// CPU from the delta between two refreshes, so the first has nothing to
    /// compare against.
    ///
    /// Reported as unknown rather than zero deliberately. A governor that read
    /// the first tick as "0% busy, plenty of room" would admit a project onto a
    /// machine that is pinned.
    pub cpu_percent: Option<f32>,
}

/// What the machine as a whole is doing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MachineUsage {
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub cpu_percent: Option<f32>,
    pub logical_cores: u32,
}

impl MachineUsage {
    /// A machine about which nothing is yet known.
    ///
    /// Not `Default`: zero total memory is not a plausible machine, and a
    /// caller that reached for `Default` would get a value that refuses
    /// everything. This exists for the moment before the first sample, and
    /// [`admission::admit`](crate::admission::admit) treats it as "cannot say".
    pub fn unknown() -> Self {
        Self {
            total_memory_bytes: 0,
            available_memory_bytes: 0,
            cpu_percent: None,
            logical_cores: 0,
        }
    }

    /// Whether anything has actually been measured.
    pub fn is_known(&self) -> bool {
        self.total_memory_bytes > 0
    }
}

/// One project that is up, and what it costs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunningProject {
    pub project_id: String,
    pub display_name: String,
    pub usage: Usage,
}

/// Where usage figures come from.
///
/// A trait so the governor can be tested against a machine that is not this
/// one — including machines that are out of memory, which is the case that
/// matters and the one that cannot be arranged for real.
pub trait UsageSource: Send + Sync + std::fmt::Debug {
    fn machine(&self) -> MachineUsage;

    /// Usage of a process and every descendant, or `None` if the tree is gone.
    ///
    /// The tree, not the process: `npm start`'s memory lives in the `node` it
    /// spawned, and a reading that looked only at `npm` would report a
    /// half-gigabyte project as using four megabytes.
    fn process_tree(&self, root_pid: u32) -> Option<Usage>;
}

/// Reads this machine, through `sysinfo`.
#[derive(Debug)]
pub struct SystemUsage {
    inner: std::sync::Mutex<Inner>,
}

#[derive(Debug)]
struct Inner {
    system: sysinfo::System,
    /// How many CPU refreshes have happened.
    ///
    /// Counted rather than inferred from the reading. `sysinfo` derives CPU
    /// from the delta between two refreshes, and on the first one it has no
    /// delta — but it does not report that as zero. On the machine this was
    /// written on the first reading is **100.0**, which is not merely wrong but
    /// wrong in the direction that matters: a governor believing it would think
    /// the machine was pinned and refuse everything.
    ///
    /// So the count is the authority, not the value.
    cpu_samples: u32,
}

impl Default for SystemUsage {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemUsage {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(Inner {
                system: sysinfo::System::new(),
                cpu_samples: 0,
            }),
        }
    }

    /// Lock, recovering from poisoning: a panicked sampler must not make every
    /// later reading fail.
    fn locked(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl UsageSource for SystemUsage {
    fn machine(&self) -> MachineUsage {
        let inner = &mut *self.locked();
        inner.system.refresh_memory();
        inner.system.refresh_cpu_usage();
        inner.cpu_samples = inner.cpu_samples.saturating_add(1);

        MachineUsage {
            total_memory_bytes: inner.system.total_memory(),
            available_memory_bytes: inner.system.available_memory(),
            cpu_percent: (inner.cpu_samples >= 2).then(|| inner.system.global_cpu_usage()),
            logical_cores: u32::try_from(inner.system.cpus().len()).unwrap_or(0),
        }
    }

    fn process_tree(&self, root_pid: u32) -> Option<Usage> {
        let system = &mut self.locked().system;
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu(),
        );

        let root = sysinfo::Pid::from_u32(root_pid);
        system.process(root)?;

        let members = descendants_of(system, root);
        let mut memory = 0u64;
        let mut cpu = 0f32;
        for pid in &members {
            if let Some(process) = system.process(*pid) {
                memory = memory.saturating_add(process.memory());
                cpu += process.cpu_usage();
            }
        }

        Some(Usage {
            memory_bytes: memory,
            cpu_percent: (cpu > 0.0).then_some(cpu),
        })
    }
}

/// A pid and everything descended from it, the root included.
fn descendants_of(system: &sysinfo::System, root: sysinfo::Pid) -> Vec<sysinfo::Pid> {
    let parents: Vec<(sysinfo::Pid, Option<sysinfo::Pid>)> = system
        .processes()
        .iter()
        .map(|(pid, process)| (*pid, process.parent()))
        .collect();

    let mut found = vec![root];
    let mut seen = std::collections::BTreeSet::from([root]);
    let mut frontier = vec![root];

    while let Some(current) = frontier.pop() {
        for (pid, parent) in &parents {
            if *parent == Some(current) && seen.insert(*pid) {
                found.push(*pid);
                frontier.push(*pid);
            }
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unmeasured_machine_says_so_rather_than_reporting_zeroes_as_facts() {
        let unknown = MachineUsage::unknown();
        assert!(!unknown.is_known());
        assert_eq!(unknown.cpu_percent, None);
    }

    #[test]
    fn this_machine_reports_a_plausible_amount_of_memory() {
        let source = SystemUsage::new();
        let machine = source.machine();

        assert!(machine.is_known(), "the scan found no memory at all");
        assert!(
            machine.total_memory_bytes > 1024 * 1024 * 1024,
            "a machine running this test has more than a gigabyte"
        );
        assert!(machine.available_memory_bytes <= machine.total_memory_bytes);
        assert!(machine.logical_cores >= 1);
    }

    /// The first reading has no delta to work from, so CPU is unknown.
    ///
    /// This is not a theoretical worry. On the machine this was written on the
    /// first reading is `100.0` — so a governor that trusted it would believe
    /// an idle machine was pinned. The second reading is a real measurement.
    #[test]
    fn cpu_is_unknown_until_there_are_two_samples_to_compare() {
        let source = SystemUsage::new();
        assert_eq!(source.machine().cpu_percent, None, "the first sample");

        std::thread::sleep(std::time::Duration::from_millis(250));
        let second = source.machine().cpu_percent;
        assert!(second.is_some(), "the second sample is a real measurement");
        assert!(
            second.unwrap_or(-1.0) >= 0.0,
            "got {second:?}, which is not a percentage"
        );
    }

    #[test]
    fn a_process_that_does_not_exist_has_no_usage() {
        let source = SystemUsage::new();
        // A pid this high is not in use on any machine running these tests.
        assert!(source.process_tree(u32::MAX - 1).is_none());
    }

    /// The reading has to cover the tree, not the root. This process is the
    /// root, and the test asserts it reports something rather than nothing.
    #[test]
    fn this_process_reports_its_own_memory() {
        let source = SystemUsage::new();
        let usage = source
            .process_tree(std::process::id())
            .expect("this process exists");
        assert!(
            usage.memory_bytes > 1024 * 1024,
            "the test runner uses more than a megabyte; got {}",
            usage.memory_bytes
        );
    }
}
