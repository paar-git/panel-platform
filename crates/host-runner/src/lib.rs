//! Running a project as an ordinary process on this machine.
//!
//! The alternative to a container. A project in host mode is started from the
//! same [`RuntimeSpec`] a container project is — `install_command`,
//! `build_command`, `start_command` — because none of those fields ever
//! mentioned Docker. What differs is the substrate underneath them.
//!
//! What a container gives and this does not: filesystem isolation, network
//! isolation, a non-root user, and a daemon that outlives the application. Host
//! mode is therefore never selected without the user saying so, and the code
//! that offers it is responsible for saying what is given up.
//!
//! This crate depends on neither `docker-manager` nor `api-types`, for the same
//! reason `detection` depends on neither: it should be possible to reason about
//! how a process is started without also holding the container model, or the
//! wire format, in mind.
//!
//! **Verified on Windows only.** The machine this was written on has no Linux
//! and no macOS. Anything below that differs per platform is marked where it
//! appears.
//!
//! [`RuntimeSpec`]: https://docs.rs/project-host-database

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod command;
pub mod health;
pub mod orchestration;
pub mod output;
pub mod probe;
pub mod supervisor;

pub use command::{split_command, start_command, CommandError, CommandInputs, ProcessCommand};
pub use health::{check, Check, Health};
pub use orchestration::{Action, Event, Machine, Process, ProcessState, ProjectStatus};
pub use output::{log_path, LogLine, Stream, Tail};
pub use probe::{candidates_for, probe, ExecutableResolver, Toolchain};
pub use supervisor::{
    run_step, start, HealthPolicy, HostError, HostObserved, HostStatus, SupervisorConfig,
    SupervisorHandle, DEFAULT_GRACE, MAX_RESTARTS,
};

/// Now, as a log line records it.
///
/// Hand-rolled rather than pulled from `chrono` or the `database` crate's
/// `time`: this crate deliberately depends on neither the wire format nor the
/// database, and a log timestamp needs nothing beyond seconds. The format is
/// RFC 3339 in UTC, which is what every other timestamp in the product uses.
pub(crate) fn now() -> String {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format_epoch_seconds(elapsed.as_secs())
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Split out so the civil-date arithmetic — the only part with anything to get
/// wrong — can be tested against known instants.
pub(crate) fn format_epoch_seconds(seconds: u64) -> String {
    let days = seconds / 86_400;
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// Days since 1970-01-01 as a civil date.
///
/// Howard Hinnant's `civil_from_days`, which is the standard solution and is
/// exact for every date this will ever see. Written out rather than reasoned
/// about at each call site.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod time_tests {
    use super::*;

    /// Known instants, including the two that catch an off-by-one in the
    /// civil-date arithmetic: a leap day and the day after a century that is
    /// not a leap year.
    #[test]
    fn epoch_seconds_become_the_date_they_are() {
        for (seconds, expected) in [
            (0u64, "1970-01-01T00:00:00Z"),
            (1, "1970-01-01T00:00:01Z"),
            (86_399, "1970-01-01T23:59:59Z"),
            (86_400, "1970-01-02T00:00:00Z"),
            // 2000-02-29, a leap day in a century year that *is* a leap year.
            (951_782_400, "2000-02-29T00:00:00Z"),
            (951_868_800, "2000-03-01T00:00:00Z"),
            (1_767_225_600, "2026-01-01T00:00:00Z"),
        ] {
            assert_eq!(format_epoch_seconds(seconds), expected, "at {seconds}");
        }
    }

    /// The shape matters as much as the value: this string is parsed by the
    /// window and sorted against timestamps the database wrote.
    #[test]
    fn a_timestamp_is_rfc_3339_in_utc() {
        let stamp = now();
        assert_eq!(stamp.len(), 20, "got {stamp}");
        assert!(stamp.ends_with('Z'), "got {stamp}");
        assert!(stamp.as_str() > "2026-01-01T00:00:00Z", "got {stamp}");
    }
}
