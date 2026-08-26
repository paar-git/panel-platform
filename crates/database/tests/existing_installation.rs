//! Opening the database file an older build left behind.
//!
//! The schema tests apply one migration at a time to an in-memory database. This
//! one is the upgrade a user actually performs: a file on disk written by an
//! earlier release, opened by this build through `Database::open`, with rows in
//! it that must still be there afterwards.
//!
//! It is written against the real migration texts rather than a fixture, so a
//! future migration that drops a user's projects fails here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use project_host_api_types::{EnvVarId, ProjectId};
use project_host_database::{
    time, Database, DISCORD_MIGRATION, INITIAL_MIGRATION, SUPPORTED_SCHEMA_VERSION,
};

/// A database file in the shape the first release shipped: migrations 1 and 2
/// applied, recorded in `_sqlx_migrations` exactly as sqlx records them, so this
/// build resumes from 3 the way it would on a user's machine.
async fn installation_at_version_two(directory: &std::path::Path) -> (std::path::PathBuf, String) {
    let migrations = directory.join("migrations");
    std::fs::create_dir_all(&migrations).expect("migrations dir");
    std::fs::write(migrations.join("0001_initial.sql"), INITIAL_MIGRATION).expect("0001");
    std::fs::write(migrations.join("0002_discord.sql"), DISCORD_MIGRATION).expect("0002");

    let path = directory.join("project-host.db");
    let url = format!(
        "sqlite://{}?mode=rwc",
        path.display().to_string().replace('\\', "/")
    );
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("open the old file");

    sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .expect("read the old migrations")
        .run(&pool)
        .await
        .expect("apply the old migrations");

    let id = ProjectId::generate().to_string();
    let now = time::now();
    sqlx::query(
        "INSERT INTO projects (id, slug, display_name, project_type, source_type,
                               directory, created_at, updated_at)
         VALUES (?, ?, 'Made before the upgrade', 'NODE_APP', 'EMPTY', ?, ?, ?)",
    )
    .bind(&id)
    .bind(format!("proj-{}", &id[4..]))
    .bind(format!("/var/lib/project-host/projects/{id}"))
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("a project from the old build");

    sqlx::query(
        "INSERT INTO project_runtimes (project_id, runtime, runtime_version, package_manager,
                                       start_command, working_dir, template_id)
         VALUES (?, 'NODEJS', '22', 'NPM', 'node index.js', '/app', 'nodejs')",
    )
    .bind(&id)
    .execute(&pool)
    .await
    .expect("its runtime row");

    sqlx::query(
        "INSERT INTO environment_variables (id, project_id, key, value_plain, is_secret,
                                            created_at, updated_at)
         VALUES (?, ?, 'KEEP_ME', 'yes', 0, ?, ?)",
    )
    .bind(EnvVarId::generate().to_string())
    .bind(&id)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("its environment variable");

    // A configured Discord bot, and a server linked to it, in the only shape
    // migration 0002 allowed: one row, pinned by `CHECK (id = 1)`.
    sqlx::query(
        "INSERT INTO discord_bot (id, application_id, token_cipher, token_nonce, updated_at)
         VALUES (1, '999999999999999999', ?, ?, ?)",
    )
    .bind(vec![7u8; 64])
    .bind(vec![3u8; 24])
    .bind(&now)
    .execute(&pool)
    .await
    .expect("a bot from the old build");

    sqlx::query(
        "INSERT INTO discord_guilds (id, guild_id, guild_name, linked_by_user_id,
                                     allow_guild_owner, created_at, updated_at)
         VALUES ('gld_from_the_old_build', '222222222222222222', 'My Server',
                 '111111111111111111', 1, ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("its linked server");

    pool.close().await;
    (path, id)
}

#[tokio::test]
async fn an_older_installation_upgrades_without_losing_anything() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (path, project) = installation_at_version_two(directory.path()).await;

    let database = Database::open(&path).await.expect("open and migrate");
    assert_eq!(
        database.schema_version().await.expect("version"),
        SUPPORTED_SCHEMA_VERSION,
        "the upgrade should have run to this build's schema"
    );

    let record = project_host_database::projects::find_project(&database, &project)
        .await
        .expect("query")
        .expect("the project the user made before the upgrade is gone");
    assert_eq!(record.display_name, "Made before the upgrade");

    // `project_runtimes` is rebuilt by 0004 and again by 0009, and `projects`
    // by 0003 and 0009. Every one is copy-drop-rename, which is where rows get
    // lost.
    let runtime = project_host_database::projects::find_runtime(&database, &project)
        .await
        .expect("query")
        .expect("the runtime row did not survive the rebuild");
    assert_eq!(runtime.runtime, "NODEJS");

    // 0009 moved the start command out of the runtime and into a process. An
    // installation made before that migration must still know what to run, or
    // the upgrade silently turns a working project into one that cannot start.
    let processes = project_host_database::projects::list_processes(&database, &project)
        .await
        .expect("query");
    assert_eq!(
        processes.len(),
        1,
        "the upgrade should make exactly one process"
    );
    assert_eq!(processes[0].name, "main");
    assert_eq!(processes[0].command, "node index.js");
    assert_eq!(
        processes[0].working_dir, ".",
        "`/app` was a container path and does not exist on this machine"
    );

    let variables = project_host_database::environment::list_variables(&database, &project)
        .await
        .expect("query");
    assert_eq!(variables.len(), 1, "the environment variable was lost");
    assert_eq!(variables[0].key, "KEEP_ME");

    assert!(
        database.integrity_check().await.expect("integrity check"),
        "the upgraded file should not be corrupt"
    );
}

/// 0006 replaces the one-row `discord_bot` with many-row `discord_bots`. A user
/// who configured a bot before that migration keeps it — they may no longer
/// have a copy of the token, so losing the row means losing the integration.
#[tokio::test]
async fn a_bot_configured_before_the_widening_is_carried_forward() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (path, _project) = installation_at_version_two(directory.path()).await;

    let database = Database::open(&path).await.expect("open and migrate");

    let bots = project_host_database::discord::list_bots(&database)
        .await
        .expect("query");
    assert_eq!(bots.len(), 1, "the configured bot did not survive 0006");
    assert_eq!(bots[0].application_id, "999999999999999999");
    assert_eq!(
        bots[0].token_cipher,
        vec![7u8; 64],
        "the ciphertext must be carried across byte for byte, or the token is lost"
    );
    assert_eq!(bots[0].token_nonce, vec![3u8; 24]);
    assert!(
        !bots[0].autostart,
        "an upgrade must not start connecting on its own"
    );

    // The server it was linked to must still be reachable, and must now know
    // which bot reaches it — there was only one, so the answer is unambiguous.
    let guild = project_host_database::discord::find_guild(&database, "222222222222222222")
        .await
        .expect("query")
        .expect("the linked server did not survive 0006");
    assert_eq!(
        guild.bot_row_id.as_deref(),
        Some(bots[0].id.as_str()),
        "the carried-forward server should be adopted by the carried-forward bot"
    );

    assert!(
        database.integrity_check().await.expect("integrity check"),
        "the upgraded file should not be corrupt"
    );
}

/// The upgrade must also leave a file this build can *write* to — the failure a
/// user sees is not a missing row, it is the next thing they try to save.
#[tokio::test]
async fn an_upgraded_installation_can_still_be_written_to() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (path, project) = installation_at_version_two(directory.path()).await;

    let database = Database::open(&path).await.expect("open and migrate");

    project_host_database::projects::set_status(
        &database,
        &project,
        project_host_api_types::ProjectStatus::Running,
        Some(project_host_api_types::HealthState::Healthy),
    )
    .await
    .expect("status should be writable after an upgrade");

    project_host_database::environment::upsert_variable(
        &database,
        &project,
        "ADDED_AFTER_UPGRADE",
        &project_host_database::environment::StoredValue::Plain("yes".to_string()),
    )
    .await
    .expect("a variable should be writable after an upgrade");

    let record = project_host_database::projects::find_project(&database, &project)
        .await
        .expect("query")
        .expect("a row");
    assert_eq!(record.status, "RUNNING");
    assert_eq!(record.health, "HEALTHY");
}
