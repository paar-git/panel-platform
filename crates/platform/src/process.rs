//! Spawning a child that can be ended along with everything it spawned.
//!
//! The problem this exists to solve: `npm start` spawns `node`. Killing `npm`
//! leaves `node` running and holding the port, and the next start then fails
//! with a port conflict that has no visible cause. A project is a process
//! *tree*, and the tree is what has to be ended.
//!
//! # Why this is not done the usual way
//!
//! The usual answers are a Windows Job Object and a `setsid`/`killpg` pair, and
//! both need FFI. The workspace sets `unsafe_code = "forbid"`, which an `allow`
//! at crate or module level cannot downgrade, and there is no `unsafe` anywhere
//! in the tree. So this uses only what is safe:
//!
//! - **Windows** — [`CommandExt::creation_flags`] with `CREATE_NEW_PROCESS_GROUP`
//!   at spawn, and `taskkill /T` to walk the tree.
//! - **Unix** — [`CommandExt::process_group`], stable since Rust 1.64 and safe,
//!   and the `kill` program, given every pid in the tree by name.
//!
//! Shelling out to `taskkill` and `kill` is not elegant. It is the only way to
//! reach this capability without FFI, and the alternative — an `unsafe` block,
//! or a dependency that hides one — was rejected when the rule was written.
//!
//! # Why Unix names the pids instead of signalling the group
//!
//! `kill -TERM -<pid>` — a *negated* pid — signals a process group, and it is
//! the obvious way to end a tree. It is not used here, because a negated pid is
//! a different kind of thing from a pid and the failure is not a no-op:
//! `kill(0, sig)` and `kill(-1, sig)` mean "everything in my own process group"
//! and "everything I am allowed to signal". A wrong number does not miss, it
//! hits something else.
//!
//! It cost a day to learn that. On a Linux CI runner this module signalled a
//! negated pid and the *runner itself* died — "the runner has received a
//! shutdown signal" — while the `sh` it was aiming at, a group leader in its
//! own right, survived untouched. The signal had not gone where the number
//! said.
//!
//! So [`descendants`] walks the tree and every process is named. Nothing
//! computed here can address a group, so the worst a stale pid can do is signal
//! one process that has already exited.
//!
//! The cost is a race the group form does not have: a process that forks
//! between the walk and the signal is not on the list. That is the trade
//! accepted — a tree that is missed is a bug to fix, a runner that is killed is
//! a machine nobody can use.
//!
//! # What "graceful" means here, honestly
//!
//! On Unix it means `SIGTERM`, which is the real thing.
//!
//! On Windows there is no equivalent for a console process without
//! `GenerateConsoleCtrlEvent`, which is FFI. `taskkill` without `/F` posts
//! `WM_CLOSE`, which a console application ignores. So on Windows the grace
//! period is a genuine wait — the child gets it, and most children that were
//! going to exit on their own will — but the polite signal at the start of it
//! does not reach a console process. What actually ends it is the forced kill at
//! the end. This is a real limitation and it is written down rather than papered
//! over.
//!
//! [`CommandExt::creation_flags`]: std::os::windows::process::CommandExt::creation_flags
//! [`CommandExt::process_group`]: std::os::unix::process::CommandExt::process_group

use std::time::Duration;

use tokio::process::Command;

use crate::error::PlatformError;

/// `CREATE_NEW_PROCESS_GROUP`, from `winbase.h`.
///
/// Spelled out rather than taken from a bindings crate: it is one constant, it
/// has never changed, and the alternative is a dependency whose only purpose is
/// to hold it.
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// `CREATE_NO_WINDOW`, from `winbase.h`.
///
/// A console-subsystem program — PowerShell, `wsl`, `cmd` — otherwise opens a
/// visible terminal for as long as it runs. That flash on launch is what this
/// flag exists to prevent.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Prevent a console window from appearing when this command is spawned.
///
/// No-op off Windows. Callers still redirect stdio; this is the half that stops
/// Windows allocating a console in the first place.
pub fn hide_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command;
}

/// The same as [`hide_console`], for a Tokio command.
pub fn hide_console_async(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command;
}

/// Configure a command so its children can be ended as a group.
///
/// Call this before spawning. It is the half of the capability that must happen
/// at spawn time; [`terminate_tree`] and [`kill_tree`] are the other half.
pub fn as_group_leader(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        // Tokio's `Command` carries `creation_flags` itself; the `std`
        // extension trait is not needed and importing it warns as unused.
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 0 means "a new group whose id is this child's pid", which is what
        // makes the pid usable as a group id when signalling later. Reached
        // through `as_std_mut` because this one is a `std` extension trait.
        command.as_std_mut().process_group(0);
    }
    command
}

/// Ask a process tree to stop, and end it if it will not.
///
/// Returns once the tree is gone. A tree that has already exited is success:
/// that is the state the caller asked for.
pub async fn terminate_tree(pid: u32, grace: Duration) -> Result<(), PlatformError> {
    if !is_alive(pid) {
        return Ok(());
    }

    request_stop(pid).await;

    // Poll rather than sleep the whole grace period: a child that exits
    // immediately should not hold a shutdown open for ten seconds.
    let deadline = std::time::Instant::now() + grace;
    while std::time::Instant::now() < deadline {
        if !is_alive(pid) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    kill_tree(pid).await
}

/// End a process tree now.
pub async fn kill_tree(pid: u32) -> Result<(), PlatformError> {
    if !is_alive(pid) {
        return Ok(());
    }

    #[cfg(windows)]
    let outcome = run(Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"])).await;

    #[cfg(unix)]
    let outcome = signal_tree(pid, "KILL").await;

    // `taskkill` and `kill` both fail when the process is already gone, which
    // is the race this function is most likely to lose and least needs to
    // report. The liveness check is the authority, not the exit code.
    if outcome.is_err() && is_alive(pid) {
        return Err(PlatformError::ProcessSurvived { pid });
    }
    Ok(())
}

/// The polite request, such as each platform has one.
async fn request_stop(pid: u32) {
    #[cfg(windows)]
    let _ = run(Command::new("taskkill").args(["/PID", &pid.to_string(), "/T"])).await;

    #[cfg(unix)]
    let _ = signal_tree(pid, "TERM").await;
}

/// Send one signal to every process in a tree, each named by its own pid.
///
/// The root is signalled first so a supervisor gets the news before the child
/// it would otherwise restart. Both are in the same `kill` call, so the gap is
/// an argument's width rather than a process spawn.
///
/// See the module docs for why this does not signal the process group.
#[cfg(unix)]
async fn signal_tree(root: u32, signal: &str) -> Result<(), ()> {
    let mut targets = vec![root];
    targets.extend(descendants(root));

    let mut command = Command::new("kill");
    command.arg(format!("-{signal}"));
    for target in &targets {
        // Formatted from a `u32`, so it cannot carry a sign and cannot be read
        // as a group. That is the property this whole approach rests on.
        command.arg(target.to_string());
    }

    run(&mut command).await
}

/// Run a helper program, silently, and answer whether it succeeded.
async fn run(command: &mut Command) -> Result<(), ()> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    match command.status().await {
        Ok(status) if status.success() => Ok(()),
        _ => Err(()),
    }
}

/// Whether a process with this id exists right now.
///
/// Reads the process table rather than signalling, because the signalling form
/// of this question needs FFI on both platforms.
///
/// A zombie does not count. On Unix a child that has exited stays in the table
/// until its parent reaps it, and every caller here is asking "is this thing
/// still running", not "is there still a row for it". Counting one costs the
/// full grace period: [`terminate_tree`] polls this until the process is gone,
/// so a zombie root is a process that never appears to stop.
pub fn is_alive(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    system
        .process(pid)
        .is_some_and(|process| process.status() != sysinfo::ProcessStatus::Zombie)
}

/// Every process descended from `root`, `root` itself excluded.
///
/// Used by the tests to prove a tree really was ended, and by callers that want
/// to report what a project is actually running.
pub fn descendants(root: u32) -> Vec<u32> {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );

    let parents: Vec<(u32, Option<u32>)> = system
        .processes()
        .iter()
        .map(|(pid, process)| (pid.as_u32(), process.parent().map(|parent| parent.as_u32())))
        .collect();

    // Breadth-first from the root. The process table is a forest and a cycle is
    // impossible, but `seen` guards the walk anyway: this runs against a table
    // that changed while it was being read.
    let mut seen = std::collections::BTreeSet::from([root]);
    let mut frontier = vec![root];
    let mut found = Vec::new();

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
    fn hide_console_is_safe_to_call_on_this_platform() {
        let mut command = std::process::Command::new("echo");
        hide_console(&mut command);
        let mut async_command = Command::new("echo");
        hide_console_async(&mut async_command);
    }

    /// A command that runs long enough to be observed, and that itself spawns a
    /// child that does the same. The point of the test is the grandchild.
    fn nested_sleeper() -> Command {
        let mut command;
        #[cfg(windows)]
        {
            command = Command::new("cmd");
            command.args(["/C", "ping -n 30 127.0.0.1 >NUL"]);
        }
        #[cfg(unix)]
        {
            command = Command::new("sh");
            command.args(["-c", "sleep 30"]);
        }
        command
    }

    /// Waits until `root` has at least one descendant, or gives up.
    async fn wait_for_descendant(root: u32) -> Vec<u32> {
        for _ in 0..100 {
            let children = descendants(root);
            if !children.is_empty() {
                return children;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Vec::new()
    }

    /// The failure this prevents: killing `npm` and leaving `node` holding the
    /// port, so the next start fails for a reason nothing reports.
    #[tokio::test]
    async fn killing_a_tree_leaves_no_grandchild_behind() {
        let mut command = nested_sleeper();
        as_group_leader(&mut command);
        let mut child = command.spawn().expect("spawn");
        let root = child.id().expect("a pid");

        let grandchildren = wait_for_descendant(root).await;
        assert!(
            !grandchildren.is_empty(),
            "the test needs a real grandchild to be worth running"
        );

        kill_tree(root).await.expect("kill");
        let _ = child.wait().await;

        assert!(!is_alive(root), "the root survived");
        for pid in grandchildren {
            assert!(!is_alive(pid), "a grandchild ({pid}) survived the kill");
        }
    }

    #[tokio::test]
    async fn terminating_something_already_gone_is_success() {
        let mut command;
        #[cfg(windows)]
        {
            command = Command::new("cmd");
            command.args(["/C", "exit 0"]);
        }
        #[cfg(unix)]
        {
            command = Command::new("true");
        }
        as_group_leader(&mut command);
        let mut child = command.spawn().expect("spawn");
        let pid = child.id().expect("a pid");
        let _ = child.wait().await;

        terminate_tree(pid, Duration::from_secs(1))
            .await
            .expect("terminating an exited process is the state the caller wanted");
    }

    /// The grace period is a bound, not a sleep: a child that goes away on its
    /// own is noticed immediately. A shutdown that always waited the full period
    /// would take minutes to stop a dozen projects that had all already exited.
    ///
    /// The child here exits by itself rather than in response to the stop
    /// request, and that is deliberate. On Windows nothing this crate can send
    /// makes a console process exit politely — see the module documentation —
    /// so a test that asserted the request itself worked would be asserting
    /// something untrue on the only platform it runs on.
    #[tokio::test]
    async fn terminate_notices_a_tree_that_exits_during_the_grace_period() {
        let mut command;
        #[cfg(windows)]
        {
            command = Command::new("cmd");
            command.args(["/C", "ping -n 2 127.0.0.1 >NUL"]);
        }
        #[cfg(unix)]
        {
            command = Command::new("sh");
            command.args(["-c", "sleep 1"]);
        }
        as_group_leader(&mut command);
        let mut child = command.spawn().expect("spawn");
        let root = child.id().expect("a pid");

        let started = std::time::Instant::now();
        terminate_tree(root, Duration::from_secs(30))
            .await
            .expect("terminate");
        let _ = child.wait().await;

        assert!(
            started.elapsed() < Duration::from_secs(15),
            "waited {:?}, so it slept the grace period rather than polling it",
            started.elapsed()
        );
        assert!(!is_alive(root));
    }

    /// The limitation stated in the module documentation, pinned as a test so
    /// that it is noticed if it ever stops being true: on Windows a console
    /// process does not exit on the polite request, so the grace period is
    /// spent in full and the forced kill is what ends it.
    #[cfg(windows)]
    #[tokio::test]
    async fn on_windows_a_console_process_ignores_the_polite_request() {
        let mut command = nested_sleeper();
        as_group_leader(&mut command);
        let mut child = command.spawn().expect("spawn");
        let root = child.id().expect("a pid");
        wait_for_descendant(root).await;

        request_stop(root).await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        assert!(
            is_alive(root),
            "a console process exited on the polite request — \
             the module documentation says this cannot happen and should be revisited"
        );

        kill_tree(root).await.expect("kill");
        let _ = child.wait().await;
    }
}
