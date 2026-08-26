//! Connection pool and migrations.

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::{DatabaseError, Result};

/// Schema version this build understands. Bumped alongside a migration that is
/// not backward compatible.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 9;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// The agent's handle on its SQLite file.
#[derive(Clone, Debug)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open (creating if absent) and migrate.
    pub async fn open(path: &Path) -> Result<Self> {
        let options = Self::connect_options(path)?;
        // SQLite permits a single writer. A small pool turns lock contention
        // into a short queue instead of a storm of SQLITE_BUSY retries.
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .min_connections(1)
            .connect_with(options)
            .await
            .map_err(DatabaseError::Connection)?;

        let database = Self { pool };
        database.migrate().await?;
        Ok(database)
    }

    /// An isolated in-memory database, migrated. For tests.
    ///
    /// `max_connections(1)` matters: each new connection to `:memory:` would
    /// otherwise get its own empty database, and a test would see its tables
    /// vanish between queries.
    pub async fn open_in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(DatabaseError::Connection)?
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(DatabaseError::Connection)?;

        let database = Self { pool };
        database.migrate().await?;
        Ok(database)
    }

    fn connect_options(path: &Path) -> Result<SqliteConnectOptions> {
        Ok(SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            // Readers never block the writer.
            .journal_mode(SqliteJournalMode::Wal)
            // WAL-safe. FULL costs more than it buys for this workload.
            .synchronous(SqliteSynchronous::Normal)
            // Off by default in SQLite, and it is a *per-connection* setting.
            // Setting it here means every pooled connection gets it — a pool
            // that sets it once silently loses referential integrity on every
            // other connection.
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5))
            .pragma("temp_store", "MEMORY")
            .pragma("cache_size", "-16000"))
    }

    /// Apply pending migrations with foreign keys disabled, then verify none
    /// were left dangling.
    ///
    /// Foreign keys are off for the duration because a migration that rebuilds
    /// a table has to drop the old one, and with enforcement on, SQLite's
    /// implicit `DELETE FROM` inside `DROP TABLE` fires every
    /// `ON DELETE CASCADE` pointing at it — which for `projects` would take the
    /// user's environment variables, ports and backups with it.
    ///
    /// The pragma cannot live in the migration file: sqlx wraps each migration
    /// in a transaction, `PRAGMA foreign_keys` is a no-op inside one, and the
    /// SQLite driver ignores the `-- no-transaction` marker entirely. It also
    /// cannot go on `connect_options`, because that would leave enforcement off
    /// for the life of every pooled connection.
    ///
    /// `foreign_key_check` afterwards is what makes this safe rather than
    /// merely convenient: a migration that did orphan a row fails startup here
    /// instead of being discovered by a query months later.
    async fn migrate(&self) -> Result<()> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(DatabaseError::Connection)?;

        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *connection)
            .await?;

        let outcome = MIGRATOR
            .run(&mut *connection)
            .await
            .map_err(DatabaseError::Migration);

        // Restored even when a migration failed: this connection goes back to
        // the pool either way, and one with enforcement off is a connection
        // that quietly accepts a broken write.
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await?;

        outcome?;

        let orphans = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *connection)
            .await?
            .len();
        if orphans > 0 {
            return Err(DatabaseError::MigrationBrokeReferences {
                orphans: orphans.try_into().unwrap_or(u32::MAX),
            });
        }

        Ok(())
    }

    /// Highest applied migration version.
    pub async fn schema_version(&self) -> Result<u32> {
        let applied: Option<i64> =
            sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
                .fetch_one(&self.pool)
                .await?;
        Ok(applied.unwrap_or(0).max(0) as u32)
    }

    /// Refuse to run against a database written by a newer build.
    ///
    /// A downgrade after a failed update must fail loudly. Continuing would let
    /// old code write rows that violate constraints it does not know about.
    pub async fn assert_schema_supported(&self) -> Result<()> {
        let found = self.schema_version().await?;
        if found > SUPPORTED_SCHEMA_VERSION {
            return Err(DatabaseError::SchemaTooNew {
                found,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    /// Checkpoint the WAL. Called during graceful shutdown so a restart does
    /// not begin by replaying a large log.
    pub async fn checkpoint(&self) -> Result<()> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Verify the file is not corrupt. Run on startup after an unclean stop.
    pub async fn integrity_check(&self) -> Result<bool> {
        let result: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&self.pool)
            .await?;
        Ok(result == "ok")
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
