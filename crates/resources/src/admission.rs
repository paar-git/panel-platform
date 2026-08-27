//! Deciding whether the machine can take one more project.
//!
//! Pure. Every branch here is reachable from a constructed [`MachineUsage`],
//! which is the point: the cases worth testing are "1 GB free, wants 2 GB", and
//! none of them can be arranged by making the development machine genuinely run
//! out of memory.

use serde::{Deserialize, Serialize};

use crate::usage::{MachineUsage, RunningProject};

/// Memory held back from the budget, whatever the machine says is free.
///
/// An operating system down to its last few hundred megabytes is already
/// thrashing, and a governor that spent the final byte would be the thing that
/// killed the machine. The reserve is the larger of an absolute floor and a
/// proportion, so it is sensible on a 4 GB machine and on a 128 GB one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Reserve {
    pub floor_bytes: u64,
    pub fraction_of_total: f64,
}

impl Default for Reserve {
    fn default() -> Self {
        Self {
            floor_bytes: 2 * 1024 * 1024 * 1024,
            fraction_of_total: 0.15,
        }
    }
}

impl Reserve {
    /// What this reserve holds back on a machine of the given size.
    pub fn bytes_for(&self, total_memory_bytes: u64) -> u64 {
        let proportional = (total_memory_bytes as f64 * self.fraction_of_total.clamp(0.0, 0.9))
            .round()
            .max(0.0) as u64;
        self.floor_bytes.max(proportional)
    }
}

/// Why a project was not admitted, with the numbers that say so.
///
/// Carries the running set because a refusal that says "2.1 GB free, this needs
/// 4 GB" is only actionable next to the list of what is holding the rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shortfall {
    pub wanted_bytes: u64,
    /// What the machine says is free, before the reserve.
    pub available_bytes: u64,
    /// What is actually spendable: available, less the reserve.
    pub headroom_bytes: u64,
    pub reserve_bytes: u64,
    /// What is already up, largest first.
    pub running: Vec<RunningProject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Admission {
    Allow,
    Refuse(Shortfall),
}

impl Admission {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Admission::Allow)
    }
}

/// Decide whether a project wanting `wanted_bytes` should start.
///
/// # What "wanted" means
///
/// The project's own `memory_limit_mb` for a container, which is exact because
/// the daemon will enforce it. For a host project it is an estimate — nothing
/// enforces it — refined by the observed peak of previous runs where there is
/// one. The caller decides which; this only does the arithmetic.
///
/// # An unmeasured machine allows
///
/// Before the first sample there is no basis for a refusal, and a governor that
/// refused everything until its sampler had warmed up would look exactly like a
/// broken application. Allowing is the honest answer to "I do not know yet".
pub fn admit(
    wanted_bytes: u64,
    machine: MachineUsage,
    reserve: Reserve,
    running: &[RunningProject],
) -> Admission {
    if !machine.is_known() {
        return Admission::Allow;
    }

    let reserve_bytes = reserve.bytes_for(machine.total_memory_bytes);
    let headroom_bytes = machine.available_memory_bytes.saturating_sub(reserve_bytes);

    if wanted_bytes <= headroom_bytes {
        return Admission::Allow;
    }

    let mut running: Vec<RunningProject> = running.to_vec();
    running.sort_by_key(|project| std::cmp::Reverse(project.usage.memory_bytes));

    Admission::Refuse(Shortfall {
        wanted_bytes,
        available_bytes: machine.available_memory_bytes,
        headroom_bytes,
        reserve_bytes,
        running,
    })
}

impl Shortfall {
    /// The refusal, as a sentence naming the numbers.
    ///
    /// Written here rather than in the interface so that the same words appear
    /// in a log line, a dialog and an error, and so that it is covered by these
    /// tests rather than by none.
    pub fn message(&self) -> String {
        let gigabytes = |bytes: u64| format!("{:.1} GB", bytes as f64 / 1_073_741_824.0);

        let mut text = format!(
            "This project needs about {}, and only {} can be spared right now ({} free, {} held back so the machine stays responsive).",
            gigabytes(self.wanted_bytes),
            gigabytes(self.headroom_bytes),
            gigabytes(self.available_bytes),
            gigabytes(self.reserve_bytes),
        );

        if let Some(largest) = self.running.first() {
            text.push_str(&format!(
                " The largest project running is {} at {}.",
                largest.display_name,
                gigabytes(largest.usage.memory_bytes)
            ));
        }

        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::Usage;

    const GB: u64 = 1024 * 1024 * 1024;

    fn machine(total_gb: u64, available_gb: u64) -> MachineUsage {
        MachineUsage {
            total_memory_bytes: total_gb * GB,
            available_memory_bytes: available_gb * GB,
            cpu_percent: Some(20.0),
            logical_cores: 8,
        }
    }

    fn project(name: &str, memory_gb: f64) -> RunningProject {
        RunningProject {
            project_id: format!("prj_{name}"),
            display_name: name.to_string(),
            usage: Usage {
                memory_bytes: (memory_gb * GB as f64) as u64,
                cpu_percent: Some(5.0),
            },
        }
    }

    #[test]
    fn a_project_that_fits_is_admitted() {
        let decision = admit(GB, machine(16, 10), Reserve::default(), &[]);
        assert_eq!(decision, Admission::Allow);
    }

    /// The case the whole crate exists for.
    #[test]
    fn a_project_that_does_not_fit_is_refused() {
        let decision = admit(8 * GB, machine(16, 4), Reserve::default(), &[]);
        assert!(!decision.is_allowed());
    }

    /// The reserve is what stops the last byte being spent. With 3 GB free on a
    /// 16 GB machine the reserve is 2.4 GB, so only 0.6 GB is actually
    /// spendable — and a 1 GB project must be refused even though 3 > 1.
    #[test]
    fn the_reserve_is_held_back_rather_than_spent() {
        let decision = admit(GB, machine(16, 3), Reserve::default(), &[]);
        match decision {
            Admission::Refuse(shortfall) => {
                assert_eq!(shortfall.available_bytes, 3 * GB);
                assert_eq!(
                    shortfall.reserve_bytes,
                    Reserve::default().bytes_for(16 * GB)
                );
                assert!(shortfall.headroom_bytes < GB);
            }
            Admission::Allow => panic!("3 GB free is not 3 GB spendable"),
        }
    }

    /// On a small machine the proportional reserve is less than the floor, so
    /// the floor is what applies.
    #[test]
    fn a_small_machine_keeps_the_absolute_floor() {
        let reserve = Reserve::default();
        assert_eq!(
            reserve.bytes_for(4 * GB),
            2 * GB,
            "15% of 4 GB is under 2 GB"
        );
    }

    /// On a large one the proportion is bigger, and it wins.
    #[test]
    fn a_large_machine_keeps_a_proportion() {
        let reserve = Reserve::default();
        // Rounded, not truncated: the implementation rounds, and a test that
        // truncated would be asserting a different function.
        assert_eq!(
            reserve.bytes_for(128 * GB),
            (128.0 * GB as f64 * 0.15).round() as u64
        );
        assert!(reserve.bytes_for(128 * GB) > reserve.floor_bytes);
    }

    /// Exactly at the boundary is allowed. Refusing something that fits to the
    /// byte would be an off-by-one the user experiences as superstition.
    #[test]
    fn a_project_that_fits_exactly_is_admitted() {
        let total = 16 * GB;
        let reserve = Reserve::default();
        let headroom = 4 * GB;
        let available = headroom + reserve.bytes_for(total);

        let decision = admit(
            headroom,
            MachineUsage {
                total_memory_bytes: total,
                available_memory_bytes: available,
                cpu_percent: None,
                logical_cores: 8,
            },
            reserve,
            &[],
        );
        assert_eq!(decision, Admission::Allow);
    }

    /// Before the sampler has run there is no basis for a refusal, and refusing
    /// everything until it has would look like a broken application.
    #[test]
    fn an_unmeasured_machine_admits_rather_than_guessing() {
        let decision = admit(64 * GB, MachineUsage::unknown(), Reserve::default(), &[]);
        assert_eq!(decision, Admission::Allow);
    }

    /// A refusal is only actionable next to what is holding the memory, so the
    /// biggest consumer comes first.
    #[test]
    fn a_refusal_lists_what_is_running_largest_first() {
        let running = [
            project("small", 0.5),
            project("enormous", 6.0),
            project("medium", 2.0),
        ];
        match admit(8 * GB, machine(16, 4), Reserve::default(), &running) {
            Admission::Refuse(shortfall) => {
                let names: Vec<&str> = shortfall
                    .running
                    .iter()
                    .map(|project| project.display_name.as_str())
                    .collect();
                assert_eq!(names, vec!["enormous", "medium", "small"]);
            }
            Admission::Allow => panic!("expected a refusal"),
        }
    }

    #[test]
    fn the_refusal_reads_as_a_sentence_with_the_numbers_in_it() {
        let running = [project("discord-bot", 6.0)];
        match admit(8 * GB, machine(16, 4), Reserve::default(), &running) {
            Admission::Refuse(shortfall) => {
                let message = shortfall.message();
                assert!(message.contains("8.0 GB"), "wanted: {message}");
                assert!(message.contains("4.0 GB"), "available: {message}");
                assert!(message.contains("discord-bot"), "largest: {message}");
                assert!(message.ends_with('.'), "not a sentence: {message}");
            }
            Admission::Allow => panic!("expected a refusal"),
        }
    }

    /// A machine with less free memory than the reserve has no headroom at all,
    /// and the subtraction must not wrap around into an enormous budget.
    #[test]
    fn a_machine_below_its_reserve_has_no_headroom_rather_than_all_of_it() {
        match admit(GB, machine(16, 1), Reserve::default(), &[]) {
            Admission::Refuse(shortfall) => assert_eq!(shortfall.headroom_bytes, 0),
            Admission::Allow => panic!("a machine under its reserve can spare nothing"),
        }
    }

    /// A configured reserve of zero is honoured — it is the user's machine —
    /// but the arithmetic still has to hold.
    #[test]
    fn a_zero_reserve_spends_everything_available() {
        let reserve = Reserve {
            floor_bytes: 0,
            fraction_of_total: 0.0,
        };
        assert_eq!(reserve.bytes_for(16 * GB), 0);
        assert_eq!(
            admit(4 * GB, machine(16, 4), reserve, &[]),
            Admission::Allow
        );
    }
}
