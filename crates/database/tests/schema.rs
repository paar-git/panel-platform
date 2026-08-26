//! Integration tests against a real SQLite database.
//!
//! These need no Docker and no Linux, so they run everywhere — which is the
//! point: the constraints that protect secrets, ports and concurrency are
//! verified on any developer machine, not only on the release hardware.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use project_host_api_types::*;
use project_host_database::schema_parity::{check_values, table_body};
use project_host_database::{
    time, Database, DatabaseError, DISCORD_MIGRATION, INITIAL_MIGRATION, REMOTE_SOURCES_MIGRATION,
    RUNTIMES_MIGRATION,
};
use sqlx::Row;

async fn db() -> Database {
    Database::open_in_memory()
        .await
        .expect("in-memory database should open and migrate")
}

/// Insert a minimal valid project and return its id.
async fn insert_project(database: &Database) -> String {
    let id = ProjectId::generate().to_string();
    let now = time::now();
    sqlx::query(
        "INSERT INTO projects (id, slug, display_name, project_type, source_type,
                               directory, created_at, updated_at)
         VALUES (?, ?, ?, 'NODE_APP', 'EMPTY', ?, ?, ?)",
    )
    .bind(&id)
    // The whole UUID body, not a prefix: two UUIDv7 values minted in the same
    // millisecond share their leading bytes and would collide on the slug.
    .bind(format!("proj-{}", &id[4..]))
    .bind("Test Project")
    .bind(format!("/var/lib/project-host/projects/{id}"))
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await
    .expect("project insert should succeed");
    id
}

// ---------------------------------------------------------------- migrations

#[tokio::test]
async fn migrations_apply_and_report_their_version() {
    let database = db().await;
    // Tied to the constant rather than a literal: a new migration that forgets
    // to bump `SUPPORTED_SCHEMA_VERSION` is the bug worth catching here, and a
    // literal would instead fail every time a migration is added correctly.
    assert_eq!(
        database.schema_version().await.unwrap(),
        project_host_database::SUPPORTED_SCHEMA_VERSION
    );
    database.assert_schema_supported().await.unwrap();
}

#[tokio::test]
async fn every_expected_table_exists() {
    let database = db().await;
    let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(database.pool())
        .await
        .unwrap();
    let tables: Vec<String> = rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();

    for expected in [
        "users",
        "sessions",
        "trusted_clients",
        "recovery_codes",
        "pairing_codes",
        "projects",
        "project_runtimes",
        "project_ports",
        "deployments",
        "container_events",
        "environment_variables",
        "project_backups",
        "backup_operations",
        "project_locks",
        "audit_logs",
        "project_metrics",
        "system_metrics",
        "notifications",
        "system_settings",
        "agent_state",
    ] {
        assert!(
            tables.iter().any(|name| name == expected),
            "missing table {expected}"
        );
    }
}

#[tokio::test]
async fn a_fresh_database_passes_its_integrity_check() {
    assert!(db().await.integrity_check().await.unwrap());
}

// ------------------------------------------------------------ enum parity

/// Each Rust enum must list exactly the values its `CHECK` constraint allows.
/// A variant added on one side only would otherwise fail at insert time, in
/// production, on the one code path that finally uses it.
///
/// Read from the *live* schema rather than from the migration text. Migration
/// 0003 rebuilds `projects`, so the definition in 0001 is no longer the one in
/// force, and a test trusting the initial file would keep passing while the
/// database it describes had moved on.
#[tokio::test]
async fn rust_enums_match_the_check_constraints() {
    let database = db().await;
    let rows = sqlx::query("SELECT name, sql FROM sqlite_master WHERE type = 'table'")
        .fetch_all(database.pool())
        .await
        .unwrap();
    let schema: std::collections::HashMap<String, String> = rows
        .iter()
        .map(|row| (row.get::<String, _>("name"), row.get::<String, _>("sql")))
        .collect();

    let assert_parity = |table: &str, column: &str, variants: &[&str]| {
        let sql = schema
            .get(table)
            .unwrap_or_else(|| panic!("no live table {table}"));
        let body =
            table_body(sql, table).unwrap_or_else(|| panic!("no CREATE TABLE body for {table}"));
        let allowed = check_values(body, column)
            .unwrap_or_else(|| panic!("no CHECK list for {table}.{column}"));

        let mut expected: Vec<&str> = variants.to_vec();
        let mut actual: Vec<&str> = allowed.iter().map(String::as_str).collect();
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(
            actual, expected,
            "{table}.{column} disagrees with its Rust enum"
        );
    };

    let as_strs = |values: &[&'static str]| values.to_vec();

    assert_parity("users", "role", &as_strs(&["ADMIN"]));

    assert_parity(
        "projects",
        "project_type",
        &ProjectType::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "projects",
        "status",
        &ProjectStatus::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "projects",
        "desired_state",
        &DesiredState::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "projects",
        "health",
        &HealthState::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "projects",
        "restart_policy",
        &RestartPolicy::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "projects",
        "network_mode",
        &NetworkMode::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "projects",
        "source_type",
        &SourceType::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "project_runtimes",
        "runtime",
        &Runtime::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
    );
    assert_parity(
        "project_runtimes",
        "package_manager",
        &PackageManager::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    // A health check belongs to a process, not to the project's toolchain.
    assert_parity(
        "project_processes",
        "health_check_type",
        &HealthCheckType::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "deployments",
        "type",
        &DeploymentType::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "deployments",
        "status",
        &DeploymentStatus::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "container_events",
        "type",
        &ContainerEventType::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "project_backups",
        "status",
        &BackupStatus::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "backup_operations",
        "kind",
        &BackupOperationKind::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "backup_operations",
        "state",
        &BackupOperationState::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "audit_logs",
        "result",
        &AuditResult::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
    assert_parity(
        "notifications",
        "level",
        &NotificationLevel::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>(),
    );
}

// ------------------------------------------------------- secret constraint

#[tokio::test]
async fn a_secret_cannot_be_stored_in_plaintext() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    let result = sqlx::query(
        "INSERT INTO environment_variables
           (id, project_id, key, value_plain, is_secret, created_at, updated_at)
         VALUES (?, ?, 'DISCORD_TOKEN', 'a-real-token', 1, ?, ?)",
    )
    .bind(EnvVarId::generate().to_string())
    .bind(&project)
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await;

    let error = DatabaseError::from(result.unwrap_err());
    assert!(
        matches!(error, DatabaseError::CheckViolation),
        "a plaintext secret must be refused by the database, got {error:?}"
    );
}

#[tokio::test]
async fn a_non_secret_cannot_be_stored_encrypted() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    let result = sqlx::query(
        "INSERT INTO environment_variables
           (id, project_id, key, value_cipher, value_nonce, is_secret, created_at, updated_at)
         VALUES (?, ?, 'PORT', X'00', X'01', 0, ?, ?)",
    )
    .bind(EnvVarId::generate().to_string())
    .bind(&project)
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await;

    assert!(matches!(
        DatabaseError::from(result.unwrap_err()),
        DatabaseError::CheckViolation
    ));
}

#[tokio::test]
async fn a_well_formed_secret_is_accepted() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    sqlx::query(
        "INSERT INTO environment_variables
           (id, project_id, key, value_cipher, value_nonce, is_secret, created_at, updated_at)
         VALUES (?, ?, 'DISCORD_TOKEN', X'DEADBEEF', X'0102', 1, ?, ?)",
    )
    .bind(EnvVarId::generate().to_string())
    .bind(&project)
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await
    .expect("an encrypted secret should be accepted");
}

#[tokio::test]
async fn environment_variable_keys_are_constrained() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    for bad_key in ["2FA", "MY-KEY", "MY KEY", "KEY;rm -rf /"] {
        let result = sqlx::query(
            "INSERT INTO environment_variables
               (id, project_id, key, value_plain, is_secret, created_at, updated_at)
             VALUES (?, ?, ?, 'x', 0, ?, ?)",
        )
        .bind(EnvVarId::generate().to_string())
        .bind(&project)
        .bind(bad_key)
        .bind(&now)
        .bind(&now)
        .execute(database.pool())
        .await;
        assert!(result.is_err(), "key {bad_key:?} should have been refused");
    }
}

// ------------------------------------------------------------ concurrency

#[tokio::test]
async fn a_project_can_hold_only_one_lock() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    sqlx::query(
        "INSERT INTO project_locks (project_id, operation, operation_id, acquired_at, expires_at)
         VALUES (?, 'RESTORE', ?, ?, ?)",
    )
    .bind(&project)
    .bind(OperationId::generate().to_string())
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await
    .expect("first lock should be granted");

    // The primary key is what refuses the second operation. This is the
    // mechanism behind "no two simultaneous restores".
    let result = sqlx::query(
        "INSERT INTO project_locks (project_id, operation, operation_id, acquired_at, expires_at)
         VALUES (?, 'REBUILD', ?, ?, ?)",
    )
    .bind(&project)
    .bind(OperationId::generate().to_string())
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await;

    assert!(matches!(
        DatabaseError::from(result.unwrap_err()),
        DatabaseError::UniqueViolation { .. }
    ));
}

#[tokio::test]
async fn two_projects_cannot_claim_the_same_host_port() {
    let database = db().await;
    let first = insert_project(&database).await;
    let second = insert_project(&database).await;

    let claim = |project: String| {
        let pool = database.pool().clone();
        async move {
            sqlx::query(
                "INSERT INTO project_ports (id, project_id, port, host_port)
                 VALUES (?, ?, 3000, 20001)",
            )
            .bind(PortId::generate().to_string())
            .bind(project)
            .execute(&pool)
            .await
        }
    };

    claim(first).await.expect("first claim should succeed");
    let result = claim(second).await;
    assert!(matches!(
        DatabaseError::from(result.unwrap_err()),
        DatabaseError::UniqueViolation { .. }
    ));
}

#[tokio::test]
async fn privileged_ports_cannot_be_stored() {
    let database = db().await;
    let project = insert_project(&database).await;

    for port in [80, 443, 1023] {
        let result = sqlx::query(
            "INSERT INTO project_ports (id, project_id, port, host_port)
             VALUES (?, ?, 3000, ?)",
        )
        .bind(PortId::generate().to_string())
        .bind(&project)
        .bind(port)
        .execute(database.pool())
        .await;
        assert!(result.is_err(), "port {port} should have been refused");
    }
}

// ------------------------------------------------- referential integrity

#[tokio::test]
async fn foreign_keys_are_enforced_on_every_connection() {
    // SQLite disables foreign keys by default and the setting is per
    // connection. This is the test that catches a pool configured to set it
    // only once.
    let database = db().await;
    let result = sqlx::query(
        "INSERT INTO environment_variables
           (id, project_id, key, value_plain, is_secret, created_at, updated_at)
         VALUES (?, 'prj_does_not_exist', 'KEY', 'v', 0, ?, ?)",
    )
    .bind(EnvVarId::generate().to_string())
    .bind(time::now())
    .bind(time::now())
    .execute(database.pool())
    .await;

    assert!(matches!(
        DatabaseError::from(result.unwrap_err()),
        DatabaseError::ForeignKeyViolation
    ));
}

#[tokio::test]
async fn deleting_a_project_cascades_to_its_children() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    sqlx::query(
        "INSERT INTO environment_variables
           (id, project_id, key, value_plain, is_secret, created_at, updated_at)
         VALUES (?, ?, 'PORT', '3000', 0, ?, ?)",
    )
    .bind(EnvVarId::generate().to_string())
    .bind(&project)
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await
    .unwrap();

    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&project)
        .execute(database.pool())
        .await
        .unwrap();

    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM environment_variables WHERE project_id = ?")
            .bind(&project)
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn audit_rows_survive_the_deletion_of_what_they_describe() {
    let database = db().await;
    let project = insert_project(&database).await;

    sqlx::query(
        "INSERT INTO audit_logs (id, occurred_at, action, target_type, target_id,
                                 target_label, result)
         VALUES (?, ?, 'project.delete', 'project', ?, 'Test Project', 'SUCCESS')",
    )
    .bind(AuditId::generate().to_string())
    .bind(time::now())
    .bind(&project)
    .execute(database.pool())
    .await
    .unwrap();

    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&project)
        .execute(database.pool())
        .await
        .unwrap();

    // An audit log that vanishes with its subject is not an audit log.
    let label: String =
        sqlx::query_scalar("SELECT target_label FROM audit_logs WHERE target_id = ?")
            .bind(&project)
            .fetch_one(database.pool())
            .await
            .expect("the audit row must outlive the project");
    assert_eq!(label, "Test Project");
}

// --------------------------------------------------------------- invariants

#[tokio::test]
async fn resource_limits_outside_their_bounds_are_refused() {
    let database = db().await;
    let now = time::now();

    for (column, value) in [
        ("memory_limit_mb", "32"),
        ("memory_limit_mb", "999999"),
        ("cpu_limit_cores", "0"),
        ("process_limit", "2"),
    ] {
        let id = ProjectId::generate().to_string();
        let sql = format!(
            "INSERT INTO projects (id, slug, display_name, project_type, source_type,
                                   directory, created_at, updated_at, {column})
             VALUES (?, ?, 'x', 'NODE_APP', 'EMPTY', ?, ?, ?, {value})"
        );
        let result = sqlx::query(&sql)
            .bind(&id)
            // The whole UUID body, not a prefix: two UUIDv7 values minted in the same
            // millisecond share their leading bytes and would collide on the slug.
            .bind(format!("proj-{}", &id[4..]))
            .bind(format!("/projects/{id}"))
            .bind(&now)
            .bind(&now)
            .execute(database.pool())
            .await;
        assert!(
            result.is_err(),
            "{column} = {value} should have been refused"
        );
    }
}

#[tokio::test]
async fn agent_state_holds_at_most_one_row() {
    let database = db().await;
    let now = time::now();

    let insert = |id: i64| {
        let pool = database.pool().clone();
        let now = now.clone();
        async move {
            sqlx::query(
                "INSERT INTO agent_state (id, agent_version, schema_version, instance_id,
                                          started_at, last_heartbeat_at, bind_address)
                 VALUES (?, '0.1.0', 1, 'inst', ?, ?, '127.0.0.1:8787')",
            )
            .bind(id)
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
        }
    };

    insert(1).await.expect("the single row should insert");
    assert!(
        insert(2).await.is_err(),
        "a second agent_state row must be refused"
    );
}

#[tokio::test]
async fn timestamps_sort_in_chronological_order() {
    let database = db().await;
    let project = insert_project(&database).await;

    for seconds in [1_600_000_000i64, 1_700_000_000, 1_650_000_000] {
        sqlx::query(
            "INSERT INTO container_events (id, project_id, type, occurred_at)
             VALUES (?, ?, 'STARTED', ?)",
        )
        .bind(AuditId::generate().to_string())
        .bind(&project)
        .bind(time::format_unix_seconds(seconds))
        .execute(database.pool())
        .await
        .unwrap();
    }

    let rows = sqlx::query(
        "SELECT occurred_at FROM container_events WHERE project_id = ? ORDER BY occurred_at DESC",
    )
    .bind(&project)
    .fetch_all(database.pool())
    .await
    .unwrap();

    let ordered: Vec<String> = rows.iter().map(|row| row.get::<String, _>(0)).collect();
    assert_eq!(ordered[0], time::format_unix_seconds(1_700_000_000));
    assert_eq!(ordered[2], time::format_unix_seconds(1_600_000_000));
}

// ------------------------------------------------------- remote sources (v3)

/// A pool holding a schema-version-2 database with one project and children,
/// built by applying the first two migrations by hand.
///
/// Migration 0003 rebuilds `projects`, and the failure mode worth testing is
/// not "does the new table exist" but "did the rebuild take the user's data with
/// it". That can only be seen by populating the old shape first, so these tests
/// drive the migration text directly rather than through `Database::open`.
async fn v2_database_with_data() -> (sqlx::SqlitePool, String) {
    // One connection, for two reasons: every connection to `:memory:` would
    // otherwise get its own empty database, and `PRAGMA foreign_keys` is
    // per-connection — a pragma set on one connection and a migration run on
    // another is the exact mistake these tests exist to catch.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite");

    sqlx::raw_sql(INITIAL_MIGRATION)
        .execute(&pool)
        .await
        .expect("0001 should apply");
    sqlx::raw_sql(DISCORD_MIGRATION)
        .execute(&pool)
        .await
        .expect("0002 should apply");

    let id = ProjectId::generate().to_string();
    let now = time::now();
    sqlx::query(
        "INSERT INTO projects (id, slug, display_name, project_type, source_type,
                               directory, created_at, updated_at)
         VALUES (?, ?, 'Kept', 'NODE_APP', 'ZIP_UPLOAD', ?, ?, ?)",
    )
    .bind(&id)
    .bind(format!("proj-{}", &id[4..]))
    .bind(format!("/var/lib/project-host/projects/{id}"))
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("project insert");

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
    .expect("env var insert");

    (pool, id)
}

/// Apply migration 0003 the way `Database::migrate` does: foreign keys off for
/// the duration, then checked.
async fn apply_0003(pool: &sqlx::SqlitePool) {
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(REMOTE_SOURCES_MIGRATION)
        .execute(pool)
        .await
        .expect("0003 should apply");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await
        .unwrap();

    let orphans = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(
        orphans.is_empty(),
        "the rebuild orphaned {} row(s)",
        orphans.len()
    );
}

#[tokio::test]
async fn rebuilding_projects_keeps_the_rows_and_their_children() {
    let (pool, project) = v2_database_with_data().await;
    apply_0003(&pool).await;

    let name: String = sqlx::query_scalar("SELECT display_name FROM projects WHERE id = ?")
        .bind(&project)
        .fetch_one(&pool)
        .await
        .expect("the project should have survived the rebuild");
    assert_eq!(name, "Kept");

    // The reason foreign keys are disabled during migration: with them on,
    // DROP TABLE projects cascades and this row is gone.
    let kept: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM environment_variables WHERE project_id = ? AND key = 'KEEP_ME'",
    )
    .bind(&project)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(kept, 1, "the rebuild cascaded into the child tables");
}

#[tokio::test]
async fn the_rebuilt_table_keeps_enforcing_its_cascade() {
    // Disabling foreign keys for the migration must not leave them disabled, and
    // the rebuilt table must still be a cascade parent.
    let (pool, project) = v2_database_with_data().await;
    apply_0003(&pool).await;

    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&project)
        .execute(&pool)
        .await
        .unwrap();

    let orphans: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM environment_variables WHERE project_id = ?")
            .bind(&project)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(orphans, 0, "deleting a project left its variables behind");
}

#[tokio::test]
async fn the_rebuilt_table_keeps_its_indexes() {
    let (pool, _) = v2_database_with_data().await;
    apply_0003(&pool).await;

    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'projects'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for expected in ["idx_projects_status", "idx_projects_desired"] {
        assert!(
            names.iter().any(|name| name == expected),
            "the rebuild dropped {expected}; it exists to keep the reconciler's \
             scan off a full table"
        );
    }
}

/// Insert a project with an arbitrary source, returning what the database said.
async fn try_source(
    database: &Database,
    source_type: &str,
    url: Option<&str>,
    git_ref: Option<&str>,
    commit: Option<&str>,
) -> std::result::Result<(), sqlx::Error> {
    let id = ProjectId::generate().to_string();
    let now = time::now();
    sqlx::query(
        "INSERT INTO projects (id, slug, display_name, project_type, source_type,
                               directory, source_url, source_ref, source_commit,
                               created_at, updated_at)
         VALUES (?, ?, 'Sourced', 'NODE_APP', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(format!("proj-{}", &id[4..]))
    .bind(source_type)
    .bind(format!("/var/lib/project-host/projects/{id}"))
    .bind(url)
    .bind(git_ref)
    .bind(commit)
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await
    .map(|_| ())
}

#[tokio::test]
async fn a_project_can_come_from_a_git_remote_or_an_archive_url() {
    let database = db().await;

    try_source(
        &database,
        "GIT_CLONE",
        Some("https://github.com/owner/repo.git"),
        Some("main"),
        Some("0f5c1d0a"),
    )
    .await
    .expect("a git clone should be a storable source");

    try_source(
        &database,
        "REMOTE_ARCHIVE",
        Some("https://example.com/release.tar.gz"),
        None,
        None,
    )
    .await
    .expect("an archive URL should be a storable source");
}

#[tokio::test]
async fn a_remote_source_without_a_url_is_refused() {
    let database = db().await;
    for kind in ["GIT_CLONE", "REMOTE_ARCHIVE"] {
        assert!(
            try_source(&database, kind, None, None, None).await.is_err(),
            "{kind} was stored with no record of where it came from"
        );
    }
}

#[tokio::test]
async fn a_local_source_cannot_claim_a_remote_url() {
    // Makes `source_url IS NOT NULL` a reliable question.
    let database = db().await;
    assert!(
        try_source(
            &database,
            "ZIP_UPLOAD",
            Some("https://example.com/repo.git"),
            None,
            None
        )
        .await
        .is_err(),
        "an uploaded project was allowed to claim a remote"
    );
}

#[tokio::test]
async fn only_a_git_clone_may_carry_a_ref_or_a_commit() {
    let database = db().await;
    assert!(
        try_source(
            &database,
            "REMOTE_ARCHIVE",
            Some("https://example.com/release.zip"),
            Some("main"),
            None
        )
        .await
        .is_err(),
        "an archive was allowed a git ref"
    );
    assert!(
        try_source(&database, "EMPTY", None, None, Some("0f5c1d0a"))
            .await
            .is_err(),
        "an empty project was allowed a commit id"
    );
}

#[tokio::test]
async fn a_url_carrying_a_token_in_its_userinfo_cannot_be_stored() {
    // The leak this prevents: a token in this column is a token in every backup
    // of the file and in any diagnostic that prints a project's origin.
    let database = db().await;
    assert!(
        try_source(
            &database,
            "GIT_CLONE",
            Some("https://user:ghp_token@github.com/owner/repo.git"),
            None,
            None
        )
        .await
        .is_err(),
        "a URL with embedded credentials reached the database"
    );
}

#[tokio::test]
async fn there_is_nowhere_to_store_a_source_token_in_the_clear() {
    let database = db().await;
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('project_source_credentials')")
            .fetch_all(database.pool())
            .await
            .unwrap();

    assert_eq!(
        columns,
        [
            "project_id",
            "ciphertext",
            "nonce",
            "created_at",
            "updated_at"
        ],
        "project_source_credentials grew a column a plaintext token could live in"
    );
}

#[tokio::test]
async fn a_source_token_nonce_of_the_wrong_length_is_refused() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    let wrong = sqlx::query(
        "INSERT INTO project_source_credentials (project_id, ciphertext, nonce,
                                                 created_at, updated_at)
         VALUES (?, X'0102', X'0102', ?, ?)",
    )
    .bind(&project)
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await;
    assert!(wrong.is_err(), "a 2-byte nonce was accepted");

    sqlx::query(
        "INSERT INTO project_source_credentials (project_id, ciphertext, nonce,
                                                 created_at, updated_at)
         VALUES (?, X'0102', ?, ?, ?)",
    )
    .bind(&project)
    .bind(vec![0u8; 24])
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await
    .expect("a 24-byte nonce is what XChaCha20-Poly1305 uses");
}

#[tokio::test]
async fn deleting_a_project_takes_its_source_token_with_it() {
    let database = db().await;
    let project = insert_project(&database).await;
    let now = time::now();

    sqlx::query(
        "INSERT INTO project_source_credentials (project_id, ciphertext, nonce,
                                                 created_at, updated_at)
         VALUES (?, X'0102', ?, ?, ?)",
    )
    .bind(&project)
    .bind(vec![0u8; 24])
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await
    .unwrap();

    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&project)
        .execute(database.pool())
        .await
        .unwrap();

    let left: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM project_source_credentials WHERE project_id = ?")
            .bind(&project)
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(left, 0, "a deleted project left its token in the database");
}

// ------------------------------------------------------------ runtimes (v4)

/// Apply one migration the way `Database::migrate` does.
async fn apply_with_foreign_keys_off(pool: &sqlx::SqlitePool, sql: &str) {
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("the migration should apply");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await
        .unwrap();

    let orphans = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(
        orphans.is_empty(),
        "the rebuild orphaned {} row(s)",
        orphans.len()
    );
}

#[tokio::test]
async fn rebuilding_project_runtimes_keeps_its_rows() {
    let (pool, project) = v2_database_with_data().await;

    sqlx::query(
        "INSERT INTO project_runtimes (project_id, runtime, runtime_version, package_manager,
                                       start_command, template_id)
         VALUES (?, 'NODEJS', '22', 'NPM', 'node index.js', 'nodejs')",
    )
    .bind(&project)
    .execute(&pool)
    .await
    .expect("runtime insert");

    apply_with_foreign_keys_off(&pool, REMOTE_SOURCES_MIGRATION).await;
    apply_with_foreign_keys_off(&pool, RUNTIMES_MIGRATION).await;

    let start: String =
        sqlx::query_scalar("SELECT start_command FROM project_runtimes WHERE project_id = ?")
            .bind(&project)
            .fetch_one(&pool)
            .await
            .expect("the runtime row should have survived the rebuild");
    assert_eq!(start, "node index.js");
}

#[tokio::test]
async fn the_rebuilt_runtimes_table_still_cascades() {
    let (pool, project) = v2_database_with_data().await;
    sqlx::query(
        "INSERT INTO project_runtimes (project_id, runtime, runtime_version, package_manager,
                                       start_command, template_id)
         VALUES (?, 'PYTHON', '3.12', 'PIP', 'python main.py', 'python')",
    )
    .bind(&project)
    .execute(&pool)
    .await
    .unwrap();

    apply_with_foreign_keys_off(&pool, REMOTE_SOURCES_MIGRATION).await;
    apply_with_foreign_keys_off(&pool, RUNTIMES_MIGRATION).await;

    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(&project)
        .execute(&pool)
        .await
        .unwrap();

    let left: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM project_runtimes WHERE project_id = ?")
            .bind(&project)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(left, 0, "deleting a project left its runtime row behind");
}

/// Try to store a runtime, returning what the database said.
async fn try_runtime(
    database: &Database,
    project: &str,
    runtime: &str,
    manager: &str,
) -> std::result::Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO project_runtimes (project_id, runtime, runtime_version, package_manager,
                                       template_id)
         VALUES (?, ?, '1', ?, 'tpl')
         ON CONFLICT(project_id) DO UPDATE SET
             runtime = excluded.runtime,
             package_manager = excluded.package_manager",
    )
    .bind(project)
    .bind(runtime)
    .bind(manager)
    .execute(database.pool())
    .await
    .map(|_| ())
}

#[tokio::test]
async fn every_runtime_this_build_offers_can_be_stored() {
    // The parity test proves the CHECK list matches the Rust enum. This proves
    // the values in that list are actually insertable, which is the thing a user
    // hits.
    let database = db().await;
    let project = insert_project(&database).await;

    for runtime in SourceRuntimes::ALL {
        try_runtime(&database, &project, runtime, "NONE")
            .await
            .unwrap_or_else(|error| panic!("{runtime} was refused: {error}"));
    }
}

/// The runtimes this build offers, spelled out rather than imported, so that
/// adding one to the enum without adding it to the migration fails here.
struct SourceRuntimes;

impl SourceRuntimes {
    const ALL: [&'static str; 13] = [
        "NODEJS",
        "TYPESCRIPT",
        "BUN",
        "DENO",
        "PYTHON",
        "GO",
        "RUST",
        "JAVA",
        "PHP",
        "RUBY",
        "DOTNET",
        "STATIC",
        "POLYGLOT",
    ];
}

#[tokio::test]
async fn every_package_manager_this_build_offers_can_be_stored() {
    let database = db().await;
    let project = insert_project(&database).await;

    for manager in [
        "PNPM",
        "NPM",
        "YARN",
        "BUN",
        "DENO",
        "PIP",
        "POETRY",
        "UV",
        "PIPENV",
        "GO_MODULES",
        "CARGO",
        "MAVEN",
        "GRADLE",
        "COMPOSER",
        "BUNDLER",
        "NUGET",
        "NONE",
    ] {
        try_runtime(&database, &project, "POLYGLOT", manager)
            .await
            .unwrap_or_else(|error| panic!("{manager} was refused: {error}"));
    }
}

#[tokio::test]
async fn a_runtime_this_build_does_not_offer_is_refused() {
    let database = db().await;
    let project = insert_project(&database).await;
    assert!(
        try_runtime(&database, &project, "COBOL", "NONE")
            .await
            .is_err(),
        "an unknown runtime reached the database"
    );
}
