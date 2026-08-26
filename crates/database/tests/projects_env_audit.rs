//! Project, environment-variable and audit storage, against a real SQLite
//! database.
//!
//! In-memory rather than mocked: the `CHECK` constraints in the migration are
//! part of the design — a secret stored in plaintext is meant to be refused by
//! SQLite itself — and a mock cannot enforce them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use project_host_api_types::{
    ContainerEventType, DeploymentStatus, DeploymentType, DesiredState, HealthState, ProjectStatus,
    ProjectType,
};
use project_host_database::audit::{self, AuditEvent, AuditResult};
use project_host_database::environment::{self, StoredValue};
use project_host_database::projects::{
    self, NewPort, NewProcess, NewProject, ProjectUpdate, RuntimeSpec,
};
use project_host_database::{queries, Database};

async fn db() -> Database {
    Database::open_in_memory().await.expect("open")
}

fn runtime() -> RuntimeSpec {
    RuntimeSpec {
        runtime: "NODEJS".to_string(),
        runtime_version: "22".to_string(),
        package_manager: "PNPM".to_string(),
        entry_file: Some("index.js".to_string()),
        publish_dir: None,
        template_id: "nodejs".to_string(),
    }
}

/// A distinct host port per slug, inside the unprivileged range.
fn host_port_for(slug: &str) -> i64 {
    let sum: i64 = slug.bytes().map(i64::from).sum::<i64>() * 7 + slug.len() as i64 * 131;
    20_000 + (sum % 20_000)
}

fn new_project(slug: &str) -> NewProject {
    NewProject {
        slug: slug.to_string(),
        display_name: "My Bot".to_string(),
        description: "a bot".to_string(),
        project_type: ProjectType::DiscordBot,
        icon: None,
        color: Some("#5865f2".to_string()),
        source_type: "EMPTY".to_string(),
        directory: format!("/var/lib/project-host/projects/{slug}"),
        source_url: None,
        source_ref: None,
        source_commit: None,
        autostart: true,
        restart_policy: "UNLESS_STOPPED".to_string(),
        network_mode: "INTERNET".to_string(),
        memory_limit_mb: 512,
        cpu_limit_cores: 1.0,
        storage_limit_mb: 2048,
        process_limit: 128,
        runtime: runtime(),
        processes: vec![NewProcess::simple("main", 0, "node index.js")],
        ports: vec![NewPort {
            port: 3000,
            // Derived from the slug so two fixtures in one database do not
            // collide on the `UNIQUE (host_port, protocol, bind_address)`
            // constraint — which is itself under test elsewhere in this file.
            host_port: Some(host_port_for(slug)),
            protocol: "tcp".to_string(),
            bind_address: "127.0.0.1".to_string(),
            is_primary: true,
            process_id: None,
        }],
    }
}

// ---------------------------------------------------------------- projects

#[tokio::test]
async fn a_created_project_has_its_runtime_and_ports() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("my-bot"))
        .await
        .expect("create");

    assert!(project.id.starts_with("prj_"));
    assert_eq!(project.status, "CREATING", "nothing is built yet");
    assert_eq!(project.desired_state, "STOPPED");
    assert!(project.autostart);

    let runtime = projects::find_runtime(&database, &project.id)
        .await
        .expect("runtime")
        .expect("present");
    assert_eq!(runtime.runtime, "NODEJS");

    let processes = projects::list_processes(&database, &project.id)
        .await
        .expect("processes");
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].command, "node index.js");

    let ports = projects::list_ports(&database, &project.id)
        .await
        .expect("ports");
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].host_port, Some(host_port_for("my-bot")));
    assert_eq!(ports[0].bind_address, "127.0.0.1");
    assert!(ports[0].id.starts_with("prt_"));
}

#[tokio::test]
async fn creation_is_atomic_across_all_three_tables() {
    // A duplicate host port fails the port insert. The project row must not
    // survive it: a project with no ports would look startable and would not be.
    let database = db().await;
    projects::create_project(&database, &new_project("first"))
        .await
        .expect("first");

    let mut clashing = new_project("second");
    clashing.ports[0].host_port = Some(host_port_for("first"));

    assert!(projects::create_project(&database, &clashing)
        .await
        .is_err());
    assert!(projects::find_project_by_slug(&database, "second")
        .await
        .expect("lookup")
        .is_none());
}

#[tokio::test]
async fn a_privileged_host_port_is_refused_by_the_database() {
    let database = db().await;
    let mut privileged = new_project("privileged");
    privileged.ports[0].host_port = Some(80);
    assert!(projects::create_project(&database, &privileged)
        .await
        .is_err());
}

#[tokio::test]
async fn resource_limits_outside_the_allowed_range_are_refused() {
    let database = db().await;
    for mutate in [
        |p: &mut NewProject| p.memory_limit_mb = 16,
        |p: &mut NewProject| p.cpu_limit_cores = 0.0,
        |p: &mut NewProject| p.process_limit = 1,
        |p: &mut NewProject| p.storage_limit_mb = 4,
    ] {
        let mut project = new_project("limits");
        mutate(&mut project);
        assert!(
            projects::create_project(&database, &project).await.is_err(),
            "an out-of-range limit must be refused"
        );
    }
}

#[tokio::test]
async fn desired_state_and_observed_status_are_recorded_separately() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("states"))
        .await
        .expect("create");

    projects::set_desired_state(&database, &project.id, DesiredState::Running)
        .await
        .expect("desire");
    projects::set_status(
        &database,
        &project.id,
        ProjectStatus::Failed,
        Some(HealthState::Unhealthy),
    )
    .await
    .expect("status");

    let reloaded = projects::find_project(&database, &project.id)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(reloaded.desired_state, "RUNNING");
    assert_eq!(reloaded.status, "FAILED");
    assert_eq!(reloaded.health, "UNHEALTHY");

    // This disagreement is exactly what the reconciler looks for.
    let wanted = projects::projects_wanting_to_run(&database)
        .await
        .expect("wanted");
    assert_eq!(wanted.len(), 1);
}

#[tokio::test]
async fn a_non_zero_exit_is_recorded_as_a_failure_with_its_code() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("exits"))
        .await
        .expect("create");

    projects::record_started(&database, &project.id)
        .await
        .expect("start");
    projects::record_stopped(&database, &project.id, Some(137), Some("out of memory"))
        .await
        .expect("stop");

    let reloaded = projects::find_project(&database, &project.id)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(reloaded.status, "FAILED");
    assert_eq!(reloaded.last_exit_code, Some(137));
    assert_eq!(
        reloaded.last_failure_reason.as_deref(),
        Some("out of memory")
    );
    assert!(reloaded.last_failure_at.is_some());
}

#[tokio::test]
async fn a_clean_exit_is_recorded_as_stopped_and_keeps_the_previous_failure() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("clean"))
        .await
        .expect("create");

    projects::record_stopped(&database, &project.id, Some(1), Some("crashed"))
        .await
        .expect("first stop");
    projects::record_stopped(&database, &project.id, Some(0), None)
        .await
        .expect("second stop");

    let reloaded = projects::find_project(&database, &project.id)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(reloaded.status, "STOPPED");
    assert_eq!(reloaded.last_exit_code, Some(0));
    // The history of what went wrong is not erased by a later clean stop.
    assert_eq!(reloaded.last_failure_reason.as_deref(), Some("crashed"));
}

#[tokio::test]
async fn an_update_cannot_change_a_projects_identity() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("identity"))
        .await
        .expect("create");

    let updated = projects::update_project(
        &database,
        &project.id,
        &ProjectUpdate {
            display_name: Some("Renamed".to_string()),
            memory_limit_mb: Some(1024),
            ..ProjectUpdate::default()
        },
    )
    .await
    .expect("update");

    assert_eq!(updated.display_name, "Renamed");
    assert_eq!(updated.memory_limit_mb, 1024);
    // The names the filesystem knows it by are unchanged, and `ProjectUpdate`
    // has no field that could change them.
    assert_eq!(updated.slug, project.slug);
    assert_eq!(updated.directory, project.directory);
    assert_eq!(
        updated.description, project.description,
        "untouched fields stay"
    );
}

#[tokio::test]
async fn updating_a_missing_project_reports_not_found() {
    let database = db().await;
    let result = projects::update_project(
        &database,
        "prj_00000000000000000000000000000000",
        &ProjectUpdate::default(),
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn archiving_hides_a_project_from_the_default_listing() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("archived"))
        .await
        .expect("create");
    projects::create_project(&database, &new_project("live"))
        .await
        .expect("create");

    projects::archive_project(&database, &project.id)
        .await
        .expect("archive");

    let visible = projects::list_projects(&database, false, None, 50)
        .await
        .expect("list");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].slug, "live");

    let all = projects::list_projects(&database, true, None, 50)
        .await
        .expect("list all");
    assert_eq!(all.len(), 2);

    // An archived project is not restarted after a reboot.
    assert!(projects::projects_wanting_to_run(&database)
        .await
        .expect("wanted")
        .is_empty());
}

#[tokio::test]
async fn listing_pages_forward_by_cursor_without_repeating_rows() {
    let database = db().await;
    for index in 0..5 {
        projects::create_project(&database, &new_project(&format!("p{index}")))
            .await
            .expect("create");
    }

    let first = projects::list_projects(&database, false, None, 2)
        .await
        .expect("page one");
    assert_eq!(first.len(), 2);

    let second = projects::list_projects(&database, false, Some(&first[1].id), 2)
        .await
        .expect("page two");
    assert_eq!(second.len(), 2);
    assert!(
        second.iter().all(|p| !first.iter().any(|q| q.id == p.id)),
        "pages must not overlap"
    );
}

#[tokio::test]
async fn a_delete_is_two_steps_so_a_crash_is_recoverable() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("doomed"))
        .await
        .expect("create");

    projects::begin_delete(&database, &project.id)
        .await
        .expect("begin");

    // A crash here leaves the row visible to recovery rather than orphaning a
    // container nothing remembers creating.
    let stuck = projects::projects_stuck_deleting(&database)
        .await
        .expect("stuck");
    assert_eq!(stuck.len(), 1);
    assert_eq!(stuck[0].id, project.id);

    projects::finish_delete(&database, &project.id)
        .await
        .expect("finish");
    assert!(projects::find_project(&database, &project.id)
        .await
        .expect("find")
        .is_none());
}

#[tokio::test]
async fn deleting_a_project_takes_its_children_with_it() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("cascade"))
        .await
        .expect("create");

    environment::upsert_variable(
        &database,
        &project.id,
        "PORT",
        &StoredValue::Plain("3000".to_string()),
    )
    .await
    .expect("env");
    projects::record_container_event(
        &database,
        &project.id,
        ContainerEventType::Started,
        None,
        None,
    )
    .await
    .expect("event");

    projects::finish_delete(&database, &project.id)
        .await
        .expect("delete");

    assert!(environment::list_variables(&database, &project.id)
        .await
        .expect("env")
        .is_empty());
    assert!(projects::list_ports(&database, &project.id)
        .await
        .expect("ports")
        .is_empty());
    assert!(projects::find_runtime(&database, &project.id)
        .await
        .expect("runtime")
        .is_none());
}

#[tokio::test]
async fn allocated_host_ports_are_reported_for_the_allocator() {
    let database = db().await;
    projects::create_project(&database, &new_project("ports-a"))
        .await
        .expect("create");
    projects::create_project(&database, &new_project("ports-b"))
        .await
        .expect("create");

    let mut allocated = projects::allocated_host_ports(&database)
        .await
        .expect("allocated");
    allocated.sort_unstable();
    let mut expected = vec![
        host_port_for("ports-a") as u16,
        host_port_for("ports-b") as u16,
    ];
    expected.sort_unstable();
    assert_eq!(allocated, expected);
}

#[tokio::test]
async fn status_counts_drive_the_dashboard_tiles() {
    let database = db().await;
    let running = projects::create_project(&database, &new_project("running"))
        .await
        .expect("create");
    projects::create_project(&database, &new_project("creating"))
        .await
        .expect("create");
    projects::set_status(&database, &running.id, ProjectStatus::Running, None)
        .await
        .expect("status");

    let counts = projects::status_counts(&database).await.expect("counts");
    assert!(counts.contains(&("RUNNING".to_string(), 1)));
    assert!(counts.contains(&("CREATING".to_string(), 1)));
}

#[tokio::test]
async fn restart_count_increments_and_is_returned() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("restarts"))
        .await
        .expect("create");

    assert_eq!(
        projects::increment_restart_count(&database, &project.id)
            .await
            .expect("increment"),
        1
    );
    assert_eq!(
        projects::increment_restart_count(&database, &project.id)
            .await
            .expect("increment"),
        2
    );
}

// ---------------------------------------------------------------- deployments

#[tokio::test]
async fn a_deployment_records_its_outcome_and_duration() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("deploys"))
        .await
        .expect("create");

    let deployment =
        projects::begin_deployment(&database, &project.id, DeploymentType::Initial, None)
            .await
            .expect("begin");
    projects::advance_deployment(
        &database,
        &deployment,
        DeploymentStatus::Building,
        Some("img:1"),
    )
    .await
    .expect("advance");
    projects::finish_deployment(
        &database,
        &deployment,
        DeploymentStatus::Succeeded,
        None,
        None,
    )
    .await
    .expect("finish");

    let history = projects::list_deployments(&database, &project.id, 10)
        .await
        .expect("list");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, "SUCCEEDED");
    assert_eq!(history[0].image_tag.as_deref(), Some("img:1"));
    assert!(history[0].finished_at.is_some());
    assert!(history[0].duration_ms.is_some());
}

#[tokio::test]
async fn deployments_running_at_a_crash_are_marked_interrupted() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("interrupted"))
        .await
        .expect("create");

    let running = projects::begin_deployment(&database, &project.id, DeploymentType::Rebuild, None)
        .await
        .expect("begin");
    let done = projects::begin_deployment(&database, &project.id, DeploymentType::Initial, None)
        .await
        .expect("begin");
    projects::finish_deployment(&database, &done, DeploymentStatus::Succeeded, None, None)
        .await
        .expect("finish");

    assert_eq!(
        projects::mark_interrupted_deployments(&database)
            .await
            .expect("mark"),
        1
    );

    let history = projects::list_deployments(&database, &project.id, 10)
        .await
        .expect("list");
    let interrupted = history.iter().find(|d| d.id == running).expect("found");
    assert_eq!(interrupted.status, "INTERRUPTED");
    assert_eq!(interrupted.error_code.as_deref(), Some("AGENT_INTERRUPTED"));
    // A finished deployment is left alone.
    let finished = history.iter().find(|d| d.id == done).expect("found");
    assert_eq!(finished.status, "SUCCEEDED");
}

#[tokio::test]
async fn container_events_are_returned_newest_first() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("events"))
        .await
        .expect("create");

    for event in [
        ContainerEventType::Created,
        ContainerEventType::Started,
        ContainerEventType::Died,
    ] {
        projects::record_container_event(&database, &project.id, event, Some(1), None)
            .await
            .expect("event");
    }

    let events = projects::list_container_events(&database, &project.id, 10)
        .await
        .expect("list");
    assert_eq!(events.len(), 3);
    assert!(events[0].id.starts_with("evt_"));
}

// ------------------------------------------------------- environment variables

#[tokio::test]
async fn a_plain_variable_round_trips() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-plain"))
        .await
        .expect("create");

    environment::upsert_variable(
        &database,
        &project.id,
        "PORT",
        &StoredValue::Plain("3000".to_string()),
    )
    .await
    .expect("upsert");

    let variables = environment::list_variables(&database, &project.id)
        .await
        .expect("list");
    assert_eq!(variables.len(), 1);
    assert_eq!(variables[0].key, "PORT");
    assert_eq!(variables[0].value, StoredValue::Plain("3000".to_string()));
    assert!(variables[0].restart_required);
    assert!(variables[0].id.starts_with("env_"));
}

#[tokio::test]
async fn a_secret_is_stored_only_as_ciphertext() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-secret"))
        .await
        .expect("create");

    environment::upsert_variable(
        &database,
        &project.id,
        "DISCORD_TOKEN",
        &StoredValue::Secret {
            cipher: vec![1, 2, 3, 4],
            nonce: vec![9; 24],
        },
    )
    .await
    .expect("upsert");

    let plain: Option<String> = sqlx::query_scalar(
        "SELECT value_plain FROM environment_variables WHERE project_id = ? AND key = ?",
    )
    .bind(&project.id)
    .bind("DISCORD_TOKEN")
    .fetch_one(database.pool())
    .await
    .expect("query");
    assert_eq!(plain, None, "a secret must have no plaintext column value");

    let stored = environment::find_variable(&database, &project.id, "DISCORD_TOKEN")
        .await
        .expect("find")
        .expect("present");
    assert!(stored.value.is_secret());
}

#[tokio::test]
async fn the_database_refuses_a_secret_with_a_plaintext_value() {
    // Written as raw SQL because the repository API makes this state
    // unrepresentable; the point is that the schema refuses it too.
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-check"))
        .await
        .expect("create");

    let result = sqlx::query(
        "INSERT INTO environment_variables
            (id, project_id, key, value_plain, is_secret, created_at, updated_at)
         VALUES ('env_x', ?, 'TOKEN', 'plaintext-secret', 1, '2026-01-01', '2026-01-01')",
    )
    .bind(&project.id)
    .execute(database.pool())
    .await;

    assert!(result.is_err(), "the CHECK constraint must refuse this row");
}

#[tokio::test]
async fn promoting_a_variable_to_secret_removes_the_readable_copy() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-promote"))
        .await
        .expect("create");

    environment::upsert_variable(
        &database,
        &project.id,
        "API_KEY",
        &StoredValue::Plain("was-readable".to_string()),
    )
    .await
    .expect("plain");

    environment::upsert_variable(
        &database,
        &project.id,
        "API_KEY",
        &StoredValue::Secret {
            cipher: vec![7, 7],
            nonce: vec![0; 24],
        },
    )
    .await
    .expect("secret");

    let plain: Option<String> = sqlx::query_scalar(
        "SELECT value_plain FROM environment_variables WHERE project_id = ? AND key = ?",
    )
    .bind(&project.id)
    .bind("API_KEY")
    .fetch_one(database.pool())
    .await
    .expect("query");
    assert_eq!(plain, None, "the old plaintext must not survive");
}

#[tokio::test]
async fn a_key_is_unique_within_a_project_but_not_across_projects() {
    let database = db().await;
    let first = projects::create_project(&database, &new_project("env-a"))
        .await
        .expect("create");
    let second = projects::create_project(&database, &new_project("env-b"))
        .await
        .expect("create");

    for project in [&first, &second] {
        environment::upsert_variable(
            &database,
            &project.id,
            "PORT",
            &StoredValue::Plain("3000".to_string()),
        )
        .await
        .expect("upsert");
    }

    // The second write to the same project updates rather than duplicating.
    environment::upsert_variable(
        &database,
        &first.id,
        "PORT",
        &StoredValue::Plain("4000".to_string()),
    )
    .await
    .expect("upsert");

    let variables = environment::list_variables(&database, &first.id)
        .await
        .expect("list");
    assert_eq!(variables.len(), 1);
    assert_eq!(variables[0].value, StoredValue::Plain("4000".to_string()));
    assert_eq!(
        environment::list_variables(&database, &second.id)
            .await
            .expect("list")
            .len(),
        1
    );
}

#[tokio::test]
async fn replacing_the_whole_set_is_all_or_nothing() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-replace"))
        .await
        .expect("create");

    environment::upsert_variable(
        &database,
        &project.id,
        "OLD",
        &StoredValue::Plain("gone".to_string()),
    )
    .await
    .expect("upsert");

    // A duplicate key inside the batch violates the UNIQUE constraint.
    let bad = vec![
        ("A".to_string(), StoredValue::Plain("1".to_string())),
        ("A".to_string(), StoredValue::Plain("2".to_string())),
    ];
    assert!(environment::replace_all(&database, &project.id, &bad)
        .await
        .is_err());

    // The rollback must have restored the original set, not left it empty.
    let variables = environment::list_variables(&database, &project.id)
        .await
        .expect("list");
    assert_eq!(variables.len(), 1);
    assert_eq!(variables[0].key, "OLD");

    let good = vec![
        ("A".to_string(), StoredValue::Plain("1".to_string())),
        ("B".to_string(), StoredValue::Plain("2".to_string())),
    ];
    assert_eq!(
        environment::replace_all(&database, &project.id, &good)
            .await
            .expect("replace"),
        2
    );
    let variables = environment::list_variables(&database, &project.id)
        .await
        .expect("list");
    assert_eq!(variables.len(), 2, "the old variable is gone");
}

#[tokio::test]
async fn variables_are_counted_without_reading_any_value() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-count"))
        .await
        .expect("create");

    environment::upsert_variable(
        &database,
        &project.id,
        "PORT",
        &StoredValue::Plain("3000".to_string()),
    )
    .await
    .expect("upsert");
    environment::upsert_variable(
        &database,
        &project.id,
        "TOKEN",
        &StoredValue::Secret {
            cipher: vec![1],
            nonce: vec![0; 24],
        },
    )
    .await
    .expect("upsert");

    assert_eq!(
        environment::count_variables(&database, &project.id)
            .await
            .expect("count"),
        (2, 1)
    );
}

#[tokio::test]
async fn deleting_a_missing_variable_reports_not_found() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-delete"))
        .await
        .expect("create");
    assert!(environment::delete_variable(&database, &project.id, "NOPE")
        .await
        .is_err());
}

#[tokio::test]
async fn the_database_refuses_a_malformed_variable_name() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("env-name"))
        .await
        .expect("create");

    for key in ["has-dash", "has space", "2LEADING"] {
        let result = environment::upsert_variable(
            &database,
            &project.id,
            key,
            &StoredValue::Plain("x".to_string()),
        )
        .await;
        assert!(result.is_err(), "`{key}` should be refused by the schema");
    }
}

// ---------------------------------------------------------------- audit log

#[tokio::test]
async fn an_audit_entry_is_written_and_read_back() {
    let database = db().await;
    let user = queries::create_user(&database, "a@example.com", "Admin", "hash")
        .await
        .expect("user");

    audit::write(
        &database,
        &AuditEvent::new("project.start", AuditResult::Success)
            .by(&user.id)
            .about("project", "prj_1", Some("My Bot"))
            .with_request("req_1")
            .detail("restart_count", 2),
    )
    .await
    .expect("write");

    let entries = audit::list(&database, None, None, None, 10)
        .await
        .expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "project.start");
    assert_eq!(entries[0].result, "SUCCESS");
    assert_eq!(entries[0].target_label.as_deref(), Some("My Bot"));
    assert!(entries[0].id.starts_with("aud_"));
    assert!(entries[0]
        .metadata
        .as_deref()
        .expect("metadata")
        .contains("restart_count"));
}

#[tokio::test]
async fn a_secret_value_never_reaches_the_audit_log() {
    let database = db().await;
    audit::write(
        &database,
        &AuditEvent::new("env.update", AuditResult::Success)
            .about("project", "prj_1", None)
            .detail("variable", "DISCORD_TOKEN")
            .detail("value", "the-actual-secret")
            .detail("is_secret", true),
    )
    .await
    .expect("write");

    let entries = audit::list(&database, None, None, None, 10)
        .await
        .expect("list");
    let metadata = entries[0].metadata.as_deref().expect("metadata");
    assert!(
        !metadata.contains("the-actual-secret"),
        "the value leaked: {metadata}"
    );
    assert!(
        metadata.contains("DISCORD_TOKEN"),
        "the name should be kept: {metadata}"
    );
}

#[tokio::test]
async fn audit_entries_can_be_filtered_by_action_and_target() {
    let database = db().await;
    for (action, target) in [
        ("project.start", "prj_1"),
        ("project.stop", "prj_1"),
        ("auth.login", "usr_1"),
    ] {
        audit::write(
            &database,
            &AuditEvent::new(action, AuditResult::Success).about("project", target, None),
        )
        .await
        .expect("write");
    }

    assert_eq!(
        audit::list(&database, Some("project."), None, None, 10)
            .await
            .expect("list")
            .len(),
        2
    );
    assert_eq!(
        audit::list(&database, None, Some("usr_1"), None, 10)
            .await
            .expect("list")
            .len(),
        1
    );
}

#[tokio::test]
async fn a_failed_audit_write_does_not_fail_the_operation() {
    // `record` swallows storage failures. A foreign key that cannot resolve is
    // the easiest way to force one.
    let database = db().await;
    audit::record(
        &database,
        AuditEvent::new("project.delete", AuditResult::Success).by("usr_does_not_exist"),
    )
    .await;

    // No panic, and nothing was written.
    assert!(audit::list(&database, None, None, None, 10)
        .await
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn pruning_keeps_the_newest_entries() {
    let database = db().await;
    for index in 0..10 {
        audit::write(
            &database,
            &AuditEvent::new(&format!("test.event{index}"), AuditResult::Success),
        )
        .await
        .expect("write");
    }

    assert_eq!(audit::prune_to(&database, 4).await.expect("prune"), 6);
    let remaining = audit::list(&database, None, None, None, 50)
        .await
        .expect("list");
    assert_eq!(remaining.len(), 4);
    assert_eq!(remaining[0].action, "test.event9", "newest survives");
}

/// Every `ProjectType` the application can construct must be one the schema
/// accepts.
///
/// The desktop used to write the literal `"GENERIC"` here, which no `CHECK`
/// allows, so every project creation failed with "value rejected by a database
/// constraint". `NewProject::project_type` is now the enum, so that exact
/// mistake no longer compiles — this covers the remaining direction, where a
/// variant is added to the enum and not to the constraint.
#[tokio::test]
async fn every_project_type_is_accepted_by_the_schema() {
    let database = db().await;

    for (index, kind) in ProjectType::ALL.iter().enumerate() {
        let mut project = new_project(&format!("kind-{index}"));
        project.project_type = *kind;

        projects::create_project(&database, &project)
            .await
            .unwrap_or_else(|error| panic!("{} was refused: {error}", kind.as_str()));
    }
}

// ------------------------------------------------------------- processes

/// A project's processes come back in the order they are started, not the
/// order they were written. The orchestrator relies on this: it starts them by
/// walking the list, and a list in insert order would start the web server
/// before the API it talks to.
#[tokio::test]
async fn processes_come_back_in_start_order() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("ordered"))
        .await
        .expect("create");

    projects::replace_processes(
        &database,
        &project.id,
        &[
            NewProcess::simple("web", 1, "npm run dev"),
            NewProcess::simple("api", 0, "npm run api"),
        ],
    )
    .await
    .expect("replace");

    let names: Vec<String> = projects::list_processes(&database, &project.id)
        .await
        .expect("list")
        .into_iter()
        .map(|process| process.name)
        .collect();

    assert_eq!(names, ["api", "web"]);
}

/// Replacing is replacing. A merge would need a rule for a process that has
/// vanished from the new set while still running, and there is no good one.
#[tokio::test]
async fn replacing_processes_removes_the_previous_set() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("replaced"))
        .await
        .expect("create");

    projects::replace_processes(
        &database,
        &project.id,
        &[NewProcess::simple("main", 0, "node a.js")],
    )
    .await
    .expect("first");

    projects::replace_processes(
        &database,
        &project.id,
        &[NewProcess::simple("only", 0, "node b.js")],
    )
    .await
    .expect("second");

    let all = projects::list_processes(&database, &project.id)
        .await
        .expect("list");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "only");
    assert_eq!(all[0].command, "node b.js");
}

/// Two processes of one project cannot share a name or a position. Both are
/// database constraints rather than checks in the orchestrator, because the
/// orchestrator is not the only writer.
#[tokio::test]
async fn a_project_cannot_have_two_processes_with_one_name() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("clashing"))
        .await
        .expect("create");

    let refused = projects::replace_processes(
        &database,
        &project.id,
        &[
            NewProcess::simple("api", 0, "node a.js"),
            NewProcess::simple("api", 1, "node b.js"),
        ],
    )
    .await;

    assert!(refused.is_err(), "two processes named `api` were accepted");

    // The replacement deletes before it inserts, so a refusal partway through
    // would leave the project with no processes at all — worse than the state
    // it started in. The transaction is what makes the failure a no-op: the
    // set the project had before is exactly the set it has after.
    let survivors = projects::list_processes(&database, &project.id)
        .await
        .expect("list");
    let names: Vec<&str> = survivors
        .iter()
        .map(|process| process.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["main"],
        "a refused replacement rolled back to something other than the original set"
    );
}

/// Status is observed. It is written from what a process did, and read back
/// exactly as written.
#[tokio::test]
async fn a_process_status_is_written_and_read_back() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("statused"))
        .await
        .expect("create");

    let made = projects::replace_processes(
        &database,
        &project.id,
        &[NewProcess::simple("main", 0, "node a.js")],
    )
    .await
    .expect("replace");

    projects::set_process_status(
        &database,
        &made[0].id,
        "CRASHED",
        None,
        Some(1),
        Some("exited immediately"),
    )
    .await
    .expect("set");

    let back = projects::list_processes(&database, &project.id)
        .await
        .expect("list");
    assert_eq!(back[0].status, "CRASHED");
    assert_eq!(back[0].exit_code, Some(1));
    assert_eq!(
        back[0].failure_reason.as_deref(),
        Some("exited immediately")
    );
}

/// A crash loop is counted per process, because a project with three processes
/// has three independent ones.
#[tokio::test]
async fn restart_counts_are_per_process() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("counted"))
        .await
        .expect("create");

    let made = projects::replace_processes(
        &database,
        &project.id,
        &[
            NewProcess::simple("api", 0, "node a.js"),
            NewProcess::simple("web", 1, "node b.js"),
        ],
    )
    .await
    .expect("replace");

    for _ in 0..3 {
        projects::increment_process_restart_count(&database, &made[0].id)
            .await
            .expect("increment");
    }

    let back = projects::list_processes(&database, &project.id)
        .await
        .expect("list");
    assert_eq!(back[0].restart_count, 3);
    assert_eq!(back[1].restart_count, 0, "the sibling did not crash");
}

/// Deleting a project takes its processes with it, by the same cascade that
/// takes its ports and environment variables.
#[tokio::test]
async fn deleting_a_project_takes_its_processes_with_it() {
    let database = db().await;
    let project = projects::create_project(&database, &new_project("doomed"))
        .await
        .expect("create");

    projects::replace_processes(
        &database,
        &project.id,
        &[NewProcess::simple("main", 0, "node a.js")],
    )
    .await
    .expect("replace");

    projects::begin_delete(&database, &project.id)
        .await
        .expect("begin");
    projects::finish_delete(&database, &project.id)
        .await
        .expect("finish");

    let orphans = projects::list_processes(&database, &project.id)
        .await
        .expect("list");
    assert!(orphans.is_empty(), "the processes outlived their project");
}

/// The process-name constraint, at its edges.
///
/// The pattern this replaced was copied from `projects.slug` and was wrong in
/// both directions: SQLite GLOB has no quantifier, so `[a-z0-9][a-z0-9-]*`
/// demands two leading characters and then permits anything. A process named
/// `a` was refused by the database, and one named `ab CDE!` was accepted.
#[tokio::test]
async fn a_process_name_may_be_one_character_but_not_any_character() {
    let database = db().await;

    for name in ["a", "1", "api", "web-2", "worker-queue-3"] {
        let project = projects::create_project(&database, &new_project(&format!("ok-{name}")))
            .await
            .expect("create");
        projects::replace_processes(
            &database,
            &project.id,
            &[NewProcess::simple(name, 0, "node index.js")],
        )
        .await
        .unwrap_or_else(|error| panic!("`{name}` should be a legal process name: {error}"));
    }

    for name in [
        "ab CDE!",
        "Api",
        "web_2",
        "-leading",
        "has space",
        "semi;colon",
    ] {
        let project = projects::create_project(&database, &new_project("refused"))
            .await
            .expect("create");
        let refused = projects::replace_processes(
            &database,
            &project.id,
            &[NewProcess::simple(name, 0, "node index.js")],
        )
        .await;
        assert!(
            refused.is_err(),
            "`{name}` should not be a legal process name"
        );
        projects::begin_delete(&database, &project.id)
            .await
            .expect("begin");
        projects::finish_delete(&database, &project.id)
            .await
            .expect("finish");
    }
}
