//! Keeps the Rust enums and the SQL `CHECK` constraints in agreement.
//!
//! Both exist for good reasons — the constraint protects the file from any
//! writer, the Rust enum protects the code from typos — but two lists of the
//! same values drift. These helpers extract the constraint lists from the
//! migration text so a test can compare them, turning a drift into a failed
//! build instead of a runtime insert that a `CHECK` refuses in production.

/// The `CREATE TABLE` body for one table, or `None` if there is no such table.
///
/// Accepts a quoted table name as well as a bare one. Migration text is written
/// bare, but the definition SQLite keeps in `sqlite_master` for a table that has
/// been through an `ALTER TABLE ... RENAME TO` is quoted — and reading the live
/// schema is the only way to be sure which migration currently owns a table.
pub fn table_body<'a>(sql: &'a str, table: &str) -> Option<&'a str> {
    let start = [
        format!("CREATE TABLE {table} ("),
        format!("CREATE TABLE \"{table}\" ("),
    ]
    .iter()
    .find_map(|needle| sql.find(needle.as_str()).map(|at| at + needle.len()))?;

    let rest = sql.get(start..)?;

    // Walk to the matching close paren, tracking nesting so the parentheses
    // inside CHECK expressions do not end the body early.
    let mut depth = 1usize;
    for (index, character) in rest.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return rest.get(..index);
                }
            }
            _ => {}
        }
    }
    None
}

/// The quoted values of `<column> IN (...)` within a table body.
pub fn check_values(table_body: &str, column: &str) -> Option<Vec<String>> {
    let needle = format!("{column} IN (");
    let start = table_body.find(&needle)? + needle.len();
    let rest = table_body.get(start..)?;
    let end = rest.find(')')?;
    let list = rest.get(..end)?;

    let values: Vec<String> = list
        .split(',')
        .map(str::trim)
        .filter_map(|token| token.strip_prefix('\'')?.strip_suffix('\''))
        .map(str::to_string)
        .collect();

    (!values.is_empty()).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
CREATE TABLE things (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK (status IN ('A','B','C')),
    CHECK (n BETWEEN 1 AND 2)
);
CREATE TABLE others (
    status TEXT NOT NULL CHECK (status IN ('X'))
);";

    #[test]
    fn extracts_the_right_table_body() {
        let body = table_body(SAMPLE, "things").expect("things");
        assert!(body.contains("'A','B','C'"));
        assert!(!body.contains("'X'"), "body leaked into the next table");
    }

    #[test]
    fn scopes_a_column_to_its_own_table() {
        // `status` exists in both tables; each must yield its own list.
        let things = table_body(SAMPLE, "things").expect("things");
        let others = table_body(SAMPLE, "others").expect("others");
        assert_eq!(
            check_values(things, "status").expect("things.status"),
            ["A", "B", "C"]
        );
        assert_eq!(
            check_values(others, "status").expect("others.status"),
            ["X"]
        );
    }

    #[test]
    fn nested_parentheses_do_not_end_the_body_early() {
        let body = table_body(SAMPLE, "things").expect("things");
        assert!(
            body.contains("BETWEEN 1 AND 2"),
            "stopped at a nested paren"
        );
    }

    #[test]
    fn a_renamed_tables_quoted_definition_is_still_found() {
        // What `sqlite_master` holds after a table rebuild.
        let renamed = "CREATE TABLE \"things\" (\n    status TEXT CHECK (status IN ('A','B'))\n)";
        let body = table_body(renamed, "things").expect("quoted things");
        assert_eq!(check_values(body, "status").expect("status"), ["A", "B"]);
    }

    #[test]
    fn missing_things_return_none() {
        assert!(table_body(SAMPLE, "nope").is_none());
        let body = table_body(SAMPLE, "things").expect("things");
        assert!(check_values(body, "absent_column").is_none());
    }

    /// The statuses a process can be in, in the order the state machine moves
    /// through them. Compared against the migration rather than assumed,
    /// because `host-runner`'s `ProcessState` is a second list of the same
    /// values and two lists drift.
    #[tokio::test]
    async fn a_process_status_check_lists_every_state() {
        let body = table_body(crate::PROCESSES_MIGRATION, "project_processes")
            .expect("0009 creates project_processes");
        let statuses = check_values(body, "status").expect("status has a CHECK");

        assert_eq!(
            statuses,
            [
                "STOPPED",
                "STARTING",
                "RUNNING",
                "RESTARTING",
                "STOPPING",
                "CRASHED",
                "FAILED",
            ]
        );
    }

    /// Not merely absent from the new `CREATE TABLE` — absent from the live
    /// schema after every migration has run, which is the only thing that
    /// proves the rebuild replaced the table rather than adding a second one.
    #[tokio::test]
    async fn the_container_columns_are_gone_from_projects() {
        let database = crate::Database::open_in_memory().await.expect("open");

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('projects')")
                .fetch_all(database.pool())
                .await
                .expect("columns");

        assert!(!columns.is_empty(), "projects should exist");
        for gone in [
            "container_id",
            "container_name",
            "image_tag",
            "network_name",
            "volume_name",
            "run_mode",
        ] {
            assert!(
                !columns.iter().any(|column| column == gone),
                "`{gone}` outlived the daemon; columns are {columns:?}"
            );
        }
    }

    /// The rename, checked on the live schema for the same reason.
    #[tokio::test]
    async fn a_port_is_a_port_and_knows_its_process() {
        let database = crate::Database::open_in_memory().await.expect("open");

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('project_ports')")
                .fetch_all(database.pool())
                .await
                .expect("columns");

        assert!(columns.iter().any(|column| column == "port"));
        assert!(columns.iter().any(|column| column == "process_id"));
        assert!(
            !columns.iter().any(|column| column == "container_port"),
            "there is no container for it to be inside"
        );
    }

    /// A database with real rows in the pre-0009 shape, migrated.
    ///
    /// The tests above open a fresh database, where every `INSERT ... SELECT`
    /// in 0009 copies nothing. This is the one that exercises them: the whole
    /// promise of the migration is that a project which runs today runs
    /// unchanged afterwards, and that claim is about existing rows.
    ///
    /// Migrations 0001 to 0008 are applied by hand rather than through
    /// `Database::open_in_memory`, which would apply 0009 as well and leave
    /// nothing to seed.
    #[tokio::test]
    async fn an_existing_project_gains_one_process_and_keeps_its_port() {
        use sqlx::Executor as _;

        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect");
        let mut connection = pool.acquire().await.expect("acquire");

        connection
            .execute("PRAGMA foreign_keys = OFF")
            .await
            .expect("pragma");

        for migration in [
            crate::INITIAL_MIGRATION,
            crate::DISCORD_MIGRATION,
            crate::REMOTE_SOURCES_MIGRATION,
            crate::RUNTIMES_MIGRATION,
            crate::RUN_MODE_MIGRATION,
            crate::DISCORD_BOTS_MIGRATION,
            crate::DISCORD_BOT_PROJECTS_MIGRATION,
            crate::LOCAL_RUNTIME_MIGRATION,
        ] {
            connection
                .execute(migration)
                .await
                .expect("a migration that shipped should apply");
        }

        connection
            .execute(
                "INSERT INTO projects (
                     id, slug, display_name, project_type, source_type, directory,
                     created_at, updated_at
                 ) VALUES (
                     'p1', 'shop', 'Shop', 'NODE_APP', 'EMPTY', '/tmp/shop',
                     '2026-08-26T00:00:00Z', '2026-08-26T00:00:00Z'
                 )",
            )
            .await
            .expect("seed project");

        connection
            .execute(
                "INSERT INTO project_runtimes (
                     project_id, runtime, runtime_version, package_manager,
                     install_command, build_command, start_command,
                     working_dir, template_id
                 ) VALUES (
                     'p1', 'NODEJS', '22', 'NPM',
                     'npm ci', 'npm run build', 'npm start',
                     '/app', 'nodejs'
                 )",
            )
            .await
            .expect("seed runtime");

        connection
            .execute(
                "INSERT INTO project_ports (
                     id, project_id, container_port, host_port, is_primary
                 ) VALUES ('port1', 'p1', 3000, 20001, 1)",
            )
            .await
            .expect("seed port");

        connection
            .execute(crate::PROCESSES_MIGRATION)
            .await
            .expect("0009 should apply to a populated database");

        let (name, order, command, working_dir, install): (String, i64, String, String, String) =
            sqlx::query_as(
                "SELECT name, start_order, command, working_dir, install_command
                   FROM project_processes WHERE project_id = 'p1'",
            )
            .fetch_one(&mut *connection)
            .await
            .expect("the project should have exactly one process");

        assert_eq!(name, "main");
        assert_eq!(order, 0);
        assert_eq!(command, "npm start", "the start command became the process");
        assert_eq!(
            working_dir, ".",
            "`/app` was a path inside a container, not on this machine"
        );
        assert_eq!(install, "npm ci");

        let (port, host_port, process_id): (i64, i64, Option<String>) = sqlx::query_as(
            "SELECT port, host_port, process_id FROM project_ports WHERE id = 'port1'",
        )
        .fetch_one(&mut *connection)
        .await
        .expect("the port survived the rebuild");

        assert_eq!(port, 3000, "renamed, not lost");
        assert_eq!(host_port, 20001);
        assert!(
            process_id.is_some(),
            "an existing port belongs to the one process there is"
        );

        let orphans = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *connection)
            .await
            .expect("check")
            .len();
        assert_eq!(orphans, 0, "the rebuild left dangling references");
    }

    /// The execution fields left `project_runtimes`; what identifies the
    /// toolchain stayed.
    #[tokio::test]
    async fn project_runtimes_keeps_the_toolchain_and_loses_the_commands() {
        let database = crate::Database::open_in_memory().await.expect("open");

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('project_runtimes')")
                .fetch_all(database.pool())
                .await
                .expect("columns");

        for kept in ["runtime", "runtime_version", "package_manager", "template_id"] {
            assert!(columns.iter().any(|column| column == kept), "lost {kept}");
        }
        for moved in [
            "start_command",
            "install_command",
            "build_command",
            "working_dir",
            "health_check_type",
        ] {
            assert!(
                !columns.iter().any(|column| column == moved),
                "`{moved}` belongs to a process now"
            );
        }
    }
}
