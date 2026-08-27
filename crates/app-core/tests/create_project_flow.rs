//! Reproduction: the whole create-a-project path, against a real database file.
//!
//! Written because "value rejected by a database constraint" reached a user
//! twice, and both times every existing test passed: the schema tests insert
//! values they spell out themselves, and the parity test compares the Rust enum
//! with the CHECK list. Neither watches what the *creation path* actually
//! writes. This does.

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
use project_host_core::runtime_plan::{plan_named, supported_runtimes};
use project_host_database::projects::{self, NewPort, NewProcess, NewProject};
use project_host_database::{environment, Database};
use project_host_project_manager::names::Slug;

async fn database() -> (tempfile::TempDir, Database) {
    let directory = tempfile::tempdir().expect("temp dir");
    let database = Database::open(&directory.path().join("project-host.db"))
        .await
        .expect("open");
    (directory, database)
}

/// Exactly what `create_project` in the desktop shell builds, minus Tauri.
fn new_project(
    runtime: &str,
    source_type: &str,
    source_url: Option<&str>,
    source_ref: Option<&str>,
    source_commit: Option<&str>,
    host_port: i64,
) -> NewProject {
    let id = project_host_api_types::ids::ProjectId::generate();
    let slug = Slug::from_project_id(id.as_str());
    let plan = plan_named(runtime).expect("plan");

    NewProject {
        slug: slug.to_string(),
        display_name: "My Project".to_string(),
        description: String::new(),
        project_type: plan.project_type,
        icon: None,
        color: None,
        source_type: source_type.to_string(),
        directory: format!("C:\\ProgramData\\ProjectHost\\projects\\{slug}"),
        source_url: source_url.map(str::to_string),
        source_ref: source_ref.map(str::to_string),
        source_commit: source_commit.map(str::to_string),
        autostart: false,
        restart_policy: "UNLESS_STOPPED".to_string(),
        network_mode: "INTERNET".to_string(),
        memory_limit_mb: 512,
        cpu_limit_cores: 1.0,
        storage_limit_mb: 2048,
        process_limit: 128,
        runtime: plan.spec.clone(),
        processes: vec![NewProcess::simple("main", 0, "node index.js")],
        ports: vec![NewPort {
            port: plan.port,
            host_port: Some(host_port),
            protocol: "tcp".to_string(),
            bind_address: "127.0.0.1".to_string(),
            is_primary: true,
            process_id: None,
        }],
    }
}

/// Every runtime the interface offers, created the way the interface creates it.
#[tokio::test]
async fn every_runtime_the_interface_offers_can_actually_be_created() {
    let (_directory, database) = database().await;

    // A distinct host port each time: they are `UNIQUE` together with the
    // protocol and bind address.
    for (port, (runtime, label)) in (20_000..).zip(supported_runtimes()) {
        let new = new_project(runtime, "EMPTY", None, None, None, port);
        projects::create_project(&database, &new)
            .await
            .unwrap_or_else(|error| {
                panic!("creating a {label} ({runtime}) project failed: {error}")
            });
    }
}

/// The three source shapes the desktop can produce, with the provenance each
/// one actually carries.
#[tokio::test]
async fn every_source_shape_the_interface_produces_can_be_stored() {
    let (_directory, database) = database().await;

    /// A source type with the provenance that source actually carries: URL,
    /// ref, commit. The schema constrains which combinations are legal.
    struct Provenance {
        source_type: &'static str,
        url: Option<&'static str>,
        git_ref: Option<&'static str>,
        commit: Option<&'static str>,
    }

    let cases = [
        Provenance {
            source_type: "EMPTY",
            url: None,
            git_ref: None,
            commit: None,
        },
        Provenance {
            source_type: "GIT_CLONE",
            url: Some("https://github.com/owner/repo.git"),
            git_ref: Some("main"),
            commit: Some("0123456789abcdef0123456789abcdef01234567"),
        },
        Provenance {
            source_type: "REMOTE_ARCHIVE",
            url: Some("https://example.com/project.zip"),
            git_ref: None,
            commit: None,
        },
    ];

    for (port, case) in (21_000..).zip(cases) {
        let new = new_project(
            "NODEJS",
            case.source_type,
            case.url,
            case.git_ref,
            case.commit,
            port,
        );
        projects::create_project(&database, &new)
            .await
            .unwrap_or_else(|error| panic!("a {} project failed: {error}", case.source_type));
    }
}

/// Every value the running application can write into an enum column.
///
/// The schema tests insert lists they spell out themselves. This one goes
/// through the enums the application actually holds, so a variant that exists in
/// Rust and not in a `CHECK` is caught here rather than by a user.
#[tokio::test]
async fn every_enum_value_this_build_can_write_is_accepted() {
    let (_directory, database) = database().await;
    let project = projects::create_project(
        &database,
        &new_project("NODEJS", "EMPTY", None, None, None, 22_000),
    )
    .await
    .expect("create");

    for status in ProjectStatus::ALL {
        projects::set_status(&database, &project.id, *status, None)
            .await
            .unwrap_or_else(|error| panic!("status {status} was refused: {error}"));
    }

    for health in HealthState::ALL {
        projects::set_status(
            &database,
            &project.id,
            ProjectStatus::Running,
            Some(*health),
        )
        .await
        .unwrap_or_else(|error| panic!("health {health} was refused: {error}"));
    }

    for desired in DesiredState::ALL {
        projects::set_desired_state(&database, &project.id, *desired)
            .await
            .unwrap_or_else(|error| panic!("desired state {desired} was refused: {error}"));
    }

    for event in ContainerEventType::ALL {
        projects::record_container_event(&database, &project.id, *event, None, None)
            .await
            .unwrap_or_else(|error| panic!("event {event} was refused: {error}"));
    }

    for kind in DeploymentType::ALL {
        let deployment = projects::begin_deployment(&database, &project.id, *kind, None)
            .await
            .unwrap_or_else(|error| panic!("deployment type {kind} was refused: {error}"));

        for status in DeploymentStatus::ALL {
            projects::advance_deployment(&database, &deployment, *status, None)
                .await
                .unwrap_or_else(|error| panic!("deployment status {status} was refused: {error}"));
        }
    }

    // Every project type must be creatable, not merely spellable: the value the
    // planner picks goes into a column with its own CHECK list.
    for project_type in ProjectType::ALL {
        let mut new = new_project("NODEJS", "EMPTY", None, None, None, 22_100);
        new.project_type = *project_type;
        new.ports[0].host_port = Some(22_100 + i64::from(*project_type as u8));
        projects::create_project(&database, &new)
            .await
            .unwrap_or_else(|error| panic!("project type {project_type} was refused: {error}"));
    }
}

/// Create, read, update, delete — and the children that hang off a project.
#[tokio::test]
async fn a_project_can_be_created_read_updated_and_deleted() {
    let (_directory, database) = database().await;

    let created = projects::create_project(
        &database,
        &new_project("PYTHON", "EMPTY", None, None, None, 23_000),
    )
    .await
    .expect("create");

    // Read, by id and by slug, and its children.
    let read = projects::find_project(&database, &created.id)
        .await
        .expect("find")
        .expect("a row");
    assert_eq!(read, created);
    assert!(projects::find_project_by_slug(&database, &created.slug)
        .await
        .expect("find by slug")
        .is_some());
    let runtime = projects::find_runtime(&database, &created.id)
        .await
        .expect("runtime")
        .expect("a runtime row");
    assert_eq!(runtime.runtime, "PYTHON");
    assert_eq!(
        projects::list_ports(&database, &created.id)
            .await
            .expect("ports")
            .len(),
        1
    );
    assert_eq!(
        projects::list_projects(&database, false, None, 100)
            .await
            .expect("list")
            .len(),
        1
    );

    // Update.
    let updated = projects::update_project(
        &database,
        &created.id,
        &projects::ProjectUpdate {
            display_name: Some("Renamed".to_string()),
            memory_limit_mb: Some(1024),
            ..Default::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(updated.display_name, "Renamed");
    assert_eq!(updated.memory_limit_mb, 1024);

    // A variable, to prove the child tables take what the application writes.
    environment::upsert_variable(
        &database,
        &created.id,
        "API_URL",
        &environment::StoredValue::Plain("https://example.com".to_string()),
    )
    .await
    .expect("store a variable");

    // Archive and bring back, since both write status columns.
    projects::archive_project(&database, &created.id)
        .await
        .expect("archive");
    projects::unarchive_project(&database, &created.id)
        .await
        .expect("unarchive");

    // Delete, and the children go with it.
    projects::begin_delete(&database, &created.id)
        .await
        .expect("begin delete");
    projects::finish_delete(&database, &created.id)
        .await
        .expect("finish delete");
    assert!(projects::find_project(&database, &created.id)
        .await
        .expect("find")
        .is_none());
    assert!(projects::find_runtime(&database, &created.id)
        .await
        .expect("runtime")
        .is_none());
    assert!(environment::list_variables(&database, &created.id)
        .await
        .expect("variables")
        .is_empty());
}

/// What survives closing the application and opening it again.
#[tokio::test]
async fn what_was_written_is_still_there_after_a_restart() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("project-host.db");

    let created = {
        let database = Database::open(&path).await.expect("open");
        let created = projects::create_project(
            &database,
            &new_project("GO", "EMPTY", None, None, None, 24_000),
        )
        .await
        .expect("create");
        environment::upsert_variable(
            &database,
            &created.id,
            "TOKEN_NAME",
            &environment::StoredValue::Plain("value".to_string()),
        )
        .await
        .expect("variable");
        database.checkpoint().await.expect("checkpoint");
        database.close().await;
        created
    };

    // A second open runs the migrations again against a populated file, which is
    // what every launch after the first one does.
    let database = Database::open(&path).await.expect("reopen");
    let found = projects::find_project(&database, &created.id)
        .await
        .expect("find")
        .expect("the project should have survived");
    assert_eq!(found, created);
    assert_eq!(
        projects::find_runtime(&database, &created.id)
            .await
            .expect("runtime")
            .expect("a runtime row")
            .runtime,
        "GO"
    );
    assert_eq!(
        environment::list_variables(&database, &created.id)
            .await
            .expect("variables")
            .len(),
        1
    );
    assert!(database.integrity_check().await.expect("integrity"));
}
