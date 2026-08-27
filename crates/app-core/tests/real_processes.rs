//! Starting actual processes, on this actual machine.
//!
//! Everything else that tests the orchestrator tests a decision. This file
//! tests the thing itself: it writes a real script to a real directory, asks
//! the orchestrator to run it, and waits for the operating system to agree.
//!
//! # Why this exists despite the design saying it would not
//!
//! The design chose unit tests plus a manual checklist. That choice left the
//! entire execution path — spawn, settle, health, ordering, stop, kill —
//! unexecuted on any machine, which is exactly the condition this repository's
//! recurring defect grows in. These tests close the gap for the scenarios a
//! test *can* reach; the checklist keeps the ones needing a window.
//!
//! # Skipping rather than failing
//!
//! Every test here needs Node on `PATH`. A machine without it is not a broken
//! build, so each returns early with a printed note instead of failing. That
//! is a deliberate trade: a silent skip can hide a regression, so the note is
//! printed loudly and `nodejs_is_present` records what was decided.
//!
//! **Verified on Windows only.** No Linux or macOS machine was available.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::time::{Duration, Instant};

use project_host_api_types::ProjectType;
use project_host_core::orchestrator::{Orchestrator, StartContext};
use project_host_core::state::{AppState, Identity};
use project_host_database::projects::{self, NewProcess, NewProject, RuntimeSpec};
use project_host_database::Database;

/// Whether Node can be found. Printed rather than assumed.
fn nodejs_is_present() -> bool {
    let found = std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok();
    if !found {
        eprintln!("SKIPPED: node is not on PATH, so nothing here can be executed");
    }
    found
}

async fn state(directory: &Path) -> AppState {
    let database = Database::open_in_memory()
        .await
        .expect("in-memory database");

    let config = project_host_core::AppConfig {
        data_dir: Some(directory.to_path_buf()),
        ..Default::default()
    };

    AppState::new(
        config,
        database,
        project_host_compatibility::Assessment {
            tier: project_host_compatibility::PerformanceTier::Standard,
            defaults: project_host_compatibility::ResourceDefaults {
                memory_limit_mb: 512,
                cpu_limit_cores: 1.0,
                process_limit: 128,
            },
        },
        Identity {
            instance_id: "test".to_string(),
            app_version: "test".to_string(),
            schema_version: 9,
            started_at_wall: project_host_database::time::now(),
        },
        None,
    )
}

/// A project on disk with the given processes, and its row.
async fn project_with(
    app: &AppState,
    directory: &Path,
    slug: &str,
    processes: Vec<NewProcess>,
) -> projects::ProjectRecord {
    projects::create_project(
        app.database(),
        &NewProject {
            slug: slug.to_string(),
            display_name: slug.to_string(),
            description: String::new(),
            project_type: ProjectType::NodeApp,
            icon: None,
            color: None,
            source_type: "EMPTY".to_string(),
            directory: directory.display().to_string(),
            source_url: None,
            source_ref: None,
            source_commit: None,
            autostart: false,
            // Off unless a test asks for it: a supervisor retrying in the
            // background outlives the assertion that provoked it.
            restart_policy: "NO".to_string(),
            network_mode: "INTERNET".to_string(),
            memory_limit_mb: 512,
            cpu_limit_cores: 1.0,
            storage_limit_mb: 2048,
            process_limit: 128,
            runtime: RuntimeSpec {
                runtime: "NODEJS".to_string(),
                runtime_version: "22".to_string(),
                package_manager: "NPM".to_string(),
                entry_file: None,
                publish_dir: None,
                template_id: "nodejs".to_string(),
            },
            processes,
            ports: Vec::new(),
        },
    )
    .await
    .expect("create the project")
}

fn orchestrator(app: &AppState, directory: &Path) -> Orchestrator {
    Orchestrator::new(app.host_projects().clone(), directory.join("logs"))
}

/// A script that stays up until it is stopped.
const STAYS_UP: &str = "setInterval(() => {}, 1000); console.log('up');\n";

/// A script that exits non-zero straight away.
const DIES_AT_ONCE: &str = "console.error('nope'); process.exit(3);\n";

fn write(directory: &Path, name: &str, body: &str) {
    std::fs::write(directory.join(name), body).expect("write the script");
}

/// Wait for a condition, or give up. Polling rather than sleeping a fixed
/// time: a fixed sleep is either flaky or slow, and usually both.
async fn until<F: Fn() -> bool>(limit: Duration, condition: F) -> bool {
    let began = Instant::now();
    while began.elapsed() < limit {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    condition()
}

#[tokio::test(flavor = "multi_thread")]
async fn one_process_starts_and_then_stops() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "index.js", STAYS_UP);

    let app = state(directory.path()).await;
    let project = project_with(
        &app,
        directory.path(),
        "single",
        vec![NewProcess::simple("main", 0, "node index.js")],
    )
    .await;

    let orchestrator = orchestrator(&app, directory.path());
    let observed = orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect("the project should start");

    assert_eq!(
        observed.status.as_str(),
        "RUNNING",
        "a script that stays up should be running; got {observed:?}"
    );

    let running = app.host_projects().processes(&project.id).await;
    assert_eq!(running.len(), 1);
    let pid = running[0]
        .handle
        .pid()
        .expect("a running process has a pid");
    assert!(
        project_host_platform::is_alive(pid),
        "the operating system does not agree that {pid} is alive"
    );

    // The row is written from what happened, not from what was asked.
    let rows = projects::list_processes(app.database(), &project.id)
        .await
        .expect("rows");
    assert_eq!(rows[0].status, "RUNNING");
    assert_eq!(rows[0].pid, Some(i64::from(pid)));

    orchestrator.stop(&project).await.expect("stop");

    assert!(
        until(
            Duration::from_secs(10),
            || !project_host_platform::is_alive(pid)
        )
        .await,
        "the process was still alive after a stop"
    );
}

/// The claim the whole ordering rule rests on.
#[tokio::test(flavor = "multi_thread")]
async fn processes_start_in_order() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    // Each records the moment it started. If the second is spawned before the
    // first has settled, the two stamps land within the same instant.
    write(
        directory.path(),
        "first.js",
        "require('fs').writeFileSync('first.txt', String(Date.now()));\
         setInterval(() => {}, 1000);\n",
    );
    write(
        directory.path(),
        "second.js",
        "require('fs').writeFileSync('second.txt', String(Date.now()));\
         setInterval(() => {}, 1000);\n",
    );

    let app = state(directory.path()).await;
    let project = project_with(
        &app,
        directory.path(),
        "ordered",
        vec![
            NewProcess::simple("first", 0, "node first.js"),
            NewProcess::simple("second", 1, "node second.js"),
        ],
    )
    .await;

    let orchestrator = orchestrator(&app, directory.path());
    let observed = orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect("both processes should start");

    assert_eq!(observed.status.as_str(), "RUNNING");
    assert_eq!(app.host_projects().processes(&project.id).await.len(), 2);

    let first: u64 = std::fs::read_to_string(directory.path().join("first.txt"))
        .expect("the first process never ran")
        .trim()
        .parse()
        .expect("a timestamp");
    let second: u64 = std::fs::read_to_string(directory.path().join("second.txt"))
        .expect("the second process never ran")
        .trim()
        .parse()
        .expect("a timestamp");

    assert!(
        second >= first,
        "the second process started before the first: {first} then {second}"
    );

    orchestrator.stop(&project).await.expect("stop");
}

/// Stopping reverses the order, so a process others were started against is
/// the last to go.
#[tokio::test(flavor = "multi_thread")]
async fn stopping_leaves_nothing_alive() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "index.js", STAYS_UP);

    let app = state(directory.path()).await;
    let project = project_with(
        &app,
        directory.path(),
        "many",
        vec![
            NewProcess::simple("a", 0, "node index.js"),
            NewProcess::simple("b", 1, "node index.js"),
            NewProcess::simple("c", 2, "node index.js"),
        ],
    )
    .await;

    let orchestrator = orchestrator(&app, directory.path());
    orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect("start");

    let pids: Vec<u32> = app
        .host_projects()
        .processes(&project.id)
        .await
        .iter()
        .filter_map(|process| process.handle.pid())
        .collect();
    assert_eq!(pids.len(), 3, "three processes should be up");

    orchestrator.stop(&project).await.expect("stop");

    for pid in pids {
        assert!(
            until(
                Duration::from_secs(10),
                || !project_host_platform::is_alive(pid)
            )
            .await,
            "{pid} survived the stop"
        );
    }
}

/// A process that dies immediately must fail the start with its own words,
/// rather than reporting a start that did not happen.
#[tokio::test(flavor = "multi_thread")]
async fn a_process_that_dies_at_once_does_not_report_running() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "index.js", DIES_AT_ONCE);

    let app = state(directory.path()).await;
    let project = project_with(
        &app,
        directory.path(),
        "doomed",
        vec![NewProcess::simple("main", 0, "node index.js")],
    )
    .await;

    let observed = orchestrator(&app, directory.path())
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await;

    // Either an error or an observed non-running status is correct. What is
    // not correct is RUNNING.
    if let Ok(observed) = observed {
        assert_ne!(
            observed.status.as_str(),
            "RUNNING",
            "a process that exited 3 was reported as running"
        );
    }

    let rows = projects::list_processes(app.database(), &project.id)
        .await
        .expect("rows");
    assert_ne!(
        rows[0].status, "RUNNING",
        "the row claims a process runs when it exited"
    );
}

/// The sibling rule, against real processes: one that cannot start must not
/// leave the others running behind it.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_leaves_no_sibling_running() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "good.js", STAYS_UP);
    write(directory.path(), "bad.js", DIES_AT_ONCE);

    let app = state(directory.path()).await;
    let project = project_with(
        &app,
        directory.path(),
        "mixed",
        vec![
            NewProcess::simple("good", 0, "node good.js"),
            NewProcess::simple("bad", 1, "node bad.js"),
        ],
    )
    .await;

    let orchestrator = orchestrator(&app, directory.path());

    let _ = orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await;

    let pids: Vec<u32> = app
        .host_projects()
        .processes(&project.id)
        .await
        .iter()
        .filter_map(|process| process.handle.pid())
        .collect();

    for pid in pids {
        assert!(
            until(
                Duration::from_secs(10),
                || !project_host_platform::is_alive(pid)
            )
            .await,
            "{pid} was left running after a sibling failed to start"
        );
    }
}

/// Several projects at once, which is the case the registry exists for.
#[tokio::test(flavor = "multi_thread")]
async fn three_projects_run_side_by_side() {
    if !nodejs_is_present() {
        return;
    }

    let root = tempfile::tempdir().expect("temp dir");
    let app = state(root.path()).await;
    let orchestrator = orchestrator(&app, root.path());

    let mut started = Vec::new();
    for slug in ["one", "two", "three"] {
        let directory = root.path().join(slug);
        std::fs::create_dir_all(&directory).expect("project dir");
        write(&directory, "index.js", STAYS_UP);

        let project = project_with(
            &app,
            &directory,
            slug,
            vec![NewProcess::simple("main", 0, "node index.js")],
        )
        .await;

        orchestrator
            .start(StartContext {
                db: app.database(),
                project: &project,
                directory: &directory,
                master_key: None,
            })
            .await
            .unwrap_or_else(|error| panic!("{slug} should start: {error}"));

        started.push(project);
    }

    assert_eq!(
        app.host_projects().running().await,
        3,
        "three projects should be up at once"
    );

    // Stopping one must leave the others alone — the registry is keyed by
    // project, and a bug that shared state between them shows up here.
    orchestrator
        .stop(&started[0])
        .await
        .expect("stop the first");

    assert_eq!(
        app.host_projects().running().await,
        2,
        "stopping one project took another down with it"
    );

    for project in &started[1..] {
        orchestrator.stop(project).await.expect("stop");
    }
}

/// A project with no processes is refused by name rather than starting
/// nothing and reporting success.
#[tokio::test(flavor = "multi_thread")]
async fn a_project_with_no_processes_is_refused() {
    let directory = tempfile::tempdir().expect("temp dir");
    let app = state(directory.path()).await;
    let project = project_with(&app, directory.path(), "empty", Vec::new()).await;

    let error = orchestrator(&app, directory.path())
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect_err("a project with nothing to run cannot start");

    assert!(
        error.to_string().contains("no processes"),
        "the refusal should say what is wrong; got {error}"
    );
}

/// Install and build run before the start command, in that order, and their
/// output lands where the user can read it.
#[tokio::test(flavor = "multi_thread")]
async fn install_and_build_run_before_the_process() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "index.js", STAYS_UP);
    write(
        directory.path(),
        "step.js",
        "require('fs').appendFileSync('order.txt', process.argv[2] + '\\n');\n",
    );

    let app = state(directory.path()).await;
    let mut process = NewProcess::simple("main", 0, "node index.js");
    process.install_command = Some("node step.js install".to_string());
    process.build_command = Some("node step.js build".to_string());

    let project = project_with(&app, directory.path(), "prepared", vec![process]).await;

    let orchestrator = orchestrator(&app, directory.path());
    orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect("start");

    let order = std::fs::read_to_string(directory.path().join("order.txt"))
        .expect("neither prepare step ran");
    let steps: Vec<&str> = order.lines().collect();
    assert_eq!(
        steps,
        ["install", "build"],
        "install must precede build, and both must precede the start"
    );

    orchestrator.stop(&project).await.expect("stop");
}

/// A failing install must stop the start there, with nothing spawned.
#[tokio::test(flavor = "multi_thread")]
async fn a_failing_install_stops_the_start() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "index.js", STAYS_UP);

    let app = state(directory.path()).await;
    let mut process = NewProcess::simple("main", 0, "node index.js");
    process.install_command = Some("node -e \"process.exit(9)\"".to_string());

    let project = project_with(&app, directory.path(), "badinstall", vec![process]).await;

    let started = orchestrator(&app, directory.path())
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await;

    assert!(started.is_err(), "a failing install must fail the start");
    assert!(
        app.host_projects().processes(&project.id).await.is_empty(),
        "nothing should have been spawned after the install failed"
    );
}

/// A port allocated to a process reaches it as `PORT`. Without this a project
/// binds whatever it defaults to and the allocation means nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_allocated_port_reaches_the_process() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(
        directory.path(),
        "index.js",
        "require('fs').writeFileSync('port.txt', String(process.env.PORT ?? 'unset'));\
         setInterval(() => {}, 1000);\n",
    );

    let app = state(directory.path()).await;
    let project = project_with(
        &app,
        directory.path(),
        "ported",
        vec![NewProcess::simple("main", 0, "node index.js")],
    )
    .await;

    // Allocated the way the real flow does it, after the project exists.
    let processes = projects::list_processes(app.database(), &project.id)
        .await
        .expect("processes");
    sqlx::query(
        "INSERT INTO project_ports (id, project_id, port, host_port, is_primary, process_id)
         VALUES ('prt_test', ?, 3000, 28123, 1, ?)",
    )
    .bind(&project.id)
    .bind(&processes[0].id)
    .execute(app.database().pool())
    .await
    .expect("allocate a port");

    let orchestrator = orchestrator(&app, directory.path());
    orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect("start");

    let seen = std::fs::read_to_string(directory.path().join("port.txt")).expect("the process ran");
    assert_eq!(
        seen.trim(),
        "28123",
        "the allocated host port did not reach the process"
    );

    orchestrator.stop(&project).await.expect("stop");
}

/// Restarting one process must actually restart it.
///
/// The regression this guards: `restart_process` used to stop the process and
/// stop there. A supervisor is bound to the child it was created with, so once
/// its stop has run the supervision task records the exit and returns — and
/// the exit was requested, so no restart policy applies. The process was gone
/// for good while the window reported it restarted and the row still read
/// RUNNING. Checked by pid, because "the row says RUNNING" was exactly the
/// thing that was wrong.
#[tokio::test(flavor = "multi_thread")]
async fn restarting_one_process_replaces_it_and_leaves_its_sibling_alone() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    write(directory.path(), "api.js", STAYS_UP);
    write(directory.path(), "web.js", STAYS_UP);

    let app = state(directory.path()).await;
    let project = project_with(
        &app,
        directory.path(),
        "restart-one",
        vec![
            NewProcess::simple("api", 0, "node api.js"),
            NewProcess::simple("web", 1, "node web.js"),
        ],
    )
    .await;

    let orchestrator = orchestrator(&app, directory.path());
    orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect("both processes should start");

    let before = app.host_projects().processes(&project.id).await;
    assert_eq!(before.len(), 2);
    let api_pid = before[0].handle.pid().expect("api has a pid");
    let web_pid = before[1].handle.pid().expect("web has a pid");

    orchestrator
        .restart_process(
            StartContext {
                db: app.database(),
                project: &project,
                directory: directory.path(),
                master_key: None,
            },
            "web",
        )
        .await
        .expect("web should restart");

    // Gathered before the project is stopped, and asserted after it. A panic
    // with two `setInterval` processes still up holds the test binary open and
    // turns a one-second failure into a ten-minute hang.
    let after = app.host_projects().processes(&project.id).await;
    let count = after.len();
    let new_web_pid = after.get(1).and_then(|process| process.handle.pid());
    let api_pid_now = after.first().and_then(|process| process.handle.pid());
    let new_web_alive = new_web_pid.is_some_and(project_host_platform::is_alive);
    let api_still_alive = project_host_platform::is_alive(api_pid);
    let old_web_died = until(Duration::from_secs(10), || {
        !project_host_platform::is_alive(web_pid)
    })
    .await;
    let web_row = projects::list_processes(app.database(), &project.id)
        .await
        .expect("rows")
        .into_iter()
        .find(|row| row.name == "web")
        .expect("web row");

    orchestrator.stop(&project).await.expect("stop");

    assert_eq!(count, 2, "a restart is not a removal");
    let new_web_pid = new_web_pid.expect("web should be running again after a restart, not gone");
    assert_ne!(
        new_web_pid, web_pid,
        "web kept its pid, so nothing was restarted"
    );
    assert!(
        new_web_alive,
        "the operating system does not agree that the restarted web is alive"
    );
    assert!(
        old_web_died,
        "the old web process is still alive after being replaced"
    );

    // The sibling was not touched. If both pids changed, this restarted the
    // project rather than the process the user pointed at.
    assert_eq!(api_pid_now, Some(api_pid), "api was restarted too");
    assert!(api_still_alive);

    assert_eq!(web_row.status, "RUNNING");
    assert_eq!(web_row.pid, Some(i64::from(new_web_pid)));
    assert_eq!(web_row.restart_count, 1, "the restart was not counted");
}

/// A health check that can never pass must fail the start, and take the
/// process it was checking down with it.
///
/// Two regressions in one test, because the first hid the second.
///
/// The gate read the supervisor's initial `Health::None` as "no check
/// configured, nothing to wait for" and returned success on its first
/// iteration — before the check had run even once, since the first poll is
/// scheduled a whole start period away. Every health check passed instantly,
/// so no health check could ever fail a start.
///
/// Underneath it: a failed check fed `Exited { terminal: true }` to the state
/// machine, which reads that as "this one is already dead" and so excludes it
/// from the teardown. The process was still running. The project was recorded
/// FAILED while its process kept going and kept its port.
#[tokio::test(flavor = "multi_thread")]
async fn a_health_check_that_never_passes_fails_the_start_and_stops_the_process() {
    if !nodejs_is_present() {
        return;
    }

    let directory = tempfile::tempdir().expect("temp dir");
    // Up, and staying up — but never listening on anything. The TCP check
    // below therefore cannot pass, however long it is given.
    write(directory.path(), "index.js", STAYS_UP);

    let app = state(directory.path()).await;
    let mut process = NewProcess::simple("main", 0, "node index.js");
    process.health_check_type = "TCP".to_string();
    // A port nothing in this test binds. Not the allocated one: the point is a
    // check that fails, not one that races.
    process.health_check_target = Some("28471".to_string());
    process.health_start_period_s = 0;
    process.health_interval_s = 1;
    process.health_retries = 2;
    process.health_timeout_s = 1;

    let project = project_with(&app, directory.path(), "unhealthy", vec![process]).await;

    let orchestrator = orchestrator(&app, directory.path());
    let observed = orchestrator
        .start(StartContext {
            db: app.database(),
            project: &project,
            directory: directory.path(),
            master_key: None,
        })
        .await
        .expect("the start itself reports its outcome rather than erroring");

    // Every fact is gathered, and the project stopped, *before* anything is
    // asserted. A panic between the two would leave a `setInterval` node
    // running with nothing left to stop it — and a leaked child holds the test
    // binary open, so the suite hangs for ten minutes instead of failing in
    // one second. Cleaning up first makes a failure fail fast.
    let status = observed.status.as_str().to_string();
    let pid = app
        .host_projects()
        .processes(&project.id)
        .await
        .first()
        .and_then(|process| process.handle.pid());
    let died = match pid {
        Some(pid) => {
            until(Duration::from_secs(10), || {
                !project_host_platform::is_alive(pid)
            })
            .await
        }
        None => true,
    };

    orchestrator.stop(&project).await.expect("stop");

    assert_ne!(
        status, "RUNNING",
        "a process that never answered its health check was reported running"
    );
    // And it is not still out there holding its port.
    assert!(died, "the unhealthy process ({pid:?}) was left running");
}
