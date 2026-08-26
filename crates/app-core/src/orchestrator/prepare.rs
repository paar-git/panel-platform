//! Which install and build steps a project actually needs.
//!
//! Install and build used to be two fields on the project's single runtime
//! row, so there was one of each and no question about it. Now they belong to
//! processes, and a project with three processes has three of each — most of
//! which are the same step written down three times.
//!
//! # The rule, and why it differs by phase
//!
//! **Installs are deduplicated by directory and command; builds are not.**
//!
//! A single-repository full-stack project declares the same root
//! `npm install` on its API and on its web process. Running it twice is
//! wasted minutes on every start. A monorepo declares `npm install` in
//! `packages/api` and in `packages/web`: the same words, different work, and
//! running it once would leave half the project without its dependencies.
//! Keying on `(working_dir, command)` gets both right without asking anyone.
//!
//! A build is the opposite. Two processes that both run `npm run build` are
//! producing their own output, and skipping the second leaves one of them
//! running against a stale or absent bundle. So builds run once per process,
//! in start order.
//!
//! This module is pure: it decides, and something else runs what it decided.

use std::collections::BTreeSet;

use project_host_database::projects::ProcessRecord;

/// One command to run before any process starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareStep {
    /// `"install"` or `"build"`, which is what the log line is labelled with.
    pub phase: &'static str,
    /// Relative to the project directory, as stored.
    pub working_dir: String,
    pub command: String,
    /// Which process this step was declared by, so a failure can name it.
    pub process_index: usize,
}

/// Every step, installs first, in the order they should run.
pub fn prepare_steps(processes: &[ProcessRecord]) -> Vec<PrepareStep> {
    let mut steps = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

    for (index, process) in processes.iter().enumerate() {
        let Some(command) = usable(process.install_command.as_deref()) else {
            continue;
        };
        let key = (process.working_dir.clone(), command.to_string());
        if !seen.insert(key) {
            continue;
        }
        steps.push(PrepareStep {
            phase: "install",
            working_dir: process.working_dir.clone(),
            command: command.to_string(),
            process_index: index,
        });
    }

    // Every install precedes every build. A build in one directory can depend
    // on a dependency installed in another — that is what a workspace is — so
    // interleaving them would work by luck.
    for (index, process) in processes.iter().enumerate() {
        let Some(command) = usable(process.build_command.as_deref()) else {
            continue;
        };
        steps.push(PrepareStep {
            phase: "build",
            working_dir: process.working_dir.clone(),
            command: command.to_string(),
            process_index: index,
        });
    }

    steps
}

/// A command worth running. Blank is not a step; the column is nullable and
/// the interface writes an empty string when a field is cleared.
fn usable(command: Option<&str>) -> Option<&str> {
    command.map(str::trim).filter(|command| !command.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(
        name: &str,
        order: i64,
        working_dir: &str,
        install: Option<&str>,
        build: Option<&str>,
    ) -> ProcessRecord {
        ProcessRecord {
            id: format!("prc_{name}"),
            project_id: "prj_1".to_string(),
            name: name.to_string(),
            start_order: order,
            command: "node index.js".to_string(),
            working_dir: working_dir.to_string(),
            install_command: install.map(str::to_string),
            build_command: build.map(str::to_string),
            health_check_type: "NONE".to_string(),
            health_check_target: None,
            health_interval_s: 30,
            health_timeout_s: 5,
            health_retries: 3,
            health_start_period_s: 20,
            status: "STOPPED".to_string(),
            pid: None,
            exit_code: None,
            failure_reason: None,
            started_at: None,
            restart_count: 0,
        }
    }

    fn phases(steps: &[PrepareStep]) -> Vec<&str> {
        steps.iter().map(|step| step.phase).collect()
    }

    #[test]
    fn one_root_install_declared_twice_runs_once() {
        let steps = prepare_steps(&[
            process("api", 0, ".", Some("npm install"), None),
            process("web", 1, ".", Some("npm install"), None),
        ]);

        assert_eq!(
            steps.len(),
            1,
            "the same install in the same place is one step"
        );
        assert_eq!(steps[0].command, "npm install");
    }

    #[test]
    fn two_workspace_installs_both_run() {
        let steps = prepare_steps(&[
            process("api", 0, "packages/api", Some("npm install"), None),
            process("web", 1, "packages/web", Some("npm install"), None),
        ]);

        assert_eq!(
            steps.len(),
            2,
            "same words, different directory, different work"
        );
    }

    #[test]
    fn every_install_precedes_every_build() {
        let steps = prepare_steps(&[
            process(
                "api",
                0,
                ".",
                Some("npm install"),
                Some("npm run build:api"),
            ),
            process(
                "web",
                1,
                ".",
                Some("npm install"),
                Some("npm run build:web"),
            ),
        ]);

        assert_eq!(phases(&steps), ["install", "build", "build"]);
    }

    #[test]
    fn builds_are_not_deduplicated() {
        let steps = prepare_steps(&[
            process("api", 0, ".", None, Some("npm run build")),
            process("web", 1, ".", None, Some("npm run build")),
        ]);

        assert_eq!(
            steps.len(),
            2,
            "each process builds its own output; skipping one leaves it stale"
        );
    }

    #[test]
    fn a_blank_command_is_not_a_step() {
        let steps = prepare_steps(&[
            process("main", 0, ".", Some("   "), Some("")),
            process("other", 1, ".", None, None),
        ]);

        assert!(steps.is_empty());
    }

    #[test]
    fn a_step_names_the_process_that_declared_it() {
        let steps = prepare_steps(&[
            process("api", 0, ".", None, None),
            process("web", 1, ".", Some("pnpm install"), None),
        ]);

        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].process_index, 1,
            "a failure has to be able to say which process asked for this"
        );
    }

    #[test]
    fn a_project_with_nothing_to_prepare_prepares_nothing() {
        assert!(prepare_steps(&[]).is_empty());
    }

    #[test]
    fn the_same_command_in_the_root_and_a_subdirectory_are_both_kept() {
        let steps = prepare_steps(&[
            process("api", 0, ".", Some("npm install"), None),
            process("web", 1, "web", Some("npm install"), None),
        ]);

        let dirs: Vec<&str> = steps.iter().map(|s| s.working_dir.as_str()).collect();
        assert_eq!(dirs, [".", "web"]);
    }
}
