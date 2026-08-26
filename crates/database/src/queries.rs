//! Typed queries.
//!
//! SQL lives here rather than in `agent-core` so the statements sit beside the
//! migration that defines the tables, and so the API layer cannot quietly grow
//! its own ad-hoc queries that bypass these invariants.

use project_host_api_types::{SessionId, UserId};
use sqlx::Row;

use crate::error::{DatabaseError, Result};
use crate::time;
use crate::Database;

/// A user as stored. The password hash never leaves this crate's callers in a
/// response — see `crates/api-types`, which has no field for it.
#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
    pub role: String,
    pub last_login_at: Option<String>,
}

/// A session joined with the user it belongs to.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub id: String,
    pub user_id: String,
    pub client_label: String,
    pub absolute_expires_at: String,
    pub idle_expires_at: String,
    pub revoked_at: Option<String>,
}

impl SessionRecord {
    /// Expired or revoked sessions are not valid, whichever came first.
    pub fn is_valid_at(&self, now: &str) -> bool {
        self.revoked_at.is_none()
            && self.absolute_expires_at.as_str() > now
            && self.idle_expires_at.as_str() > now
    }
}

// ---------------------------------------------------------------- users

pub async fn administrator_exists(database: &Database) -> Result<bool> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(database.pool())
        .await?;
    Ok(count > 0)
}

/// Create the first administrator.
///
/// `email_lower` carries the uniqueness constraint, so `Admin@Example.com` and
/// `admin@example.com` cannot both exist — two accounts differing only in case
/// would be an authentication ambiguity, not a feature.
pub async fn create_user(
    database: &Database,
    email: &str,
    display_name: &str,
    password_hash: &str,
) -> Result<UserRecord> {
    let id = UserId::generate().to_string();
    let now = time::now();

    sqlx::query(
        "INSERT INTO users (id, email, email_lower, display_name, password_hash,
                            role, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'ADMIN', ?, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(email.to_lowercase())
    .bind(display_name)
    .bind(password_hash)
    .bind(&now)
    .bind(&now)
    .execute(database.pool())
    .await?;

    Ok(UserRecord {
        id,
        email: email.to_string(),
        display_name: display_name.to_string(),
        password_hash: password_hash.to_string(),
        role: "ADMIN".to_string(),
        last_login_at: None,
    })
}

pub async fn find_user_by_email(database: &Database, email: &str) -> Result<Option<UserRecord>> {
    let row = sqlx::query(
        "SELECT id, email, display_name, password_hash, role, last_login_at
         FROM users WHERE email_lower = ?",
    )
    .bind(email.to_lowercase())
    .fetch_optional(database.pool())
    .await?;

    Ok(row.map(|row| UserRecord {
        id: row.get("id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        password_hash: row.get("password_hash"),
        role: row.get("role"),
        last_login_at: row.get("last_login_at"),
    }))
}

/// The administrator, for the local bootstrap path.
///
/// Version one has exactly one. Ordering by `created_at` makes the choice
/// deterministic anyway, so adding roles later cannot turn this into a
/// silently arbitrary pick.
pub async fn find_first_administrator(database: &Database) -> Result<Option<UserRecord>> {
    let row = sqlx::query(
        "SELECT id, email, display_name, password_hash, role, last_login_at
         FROM users WHERE role = 'ADMIN' ORDER BY created_at, id LIMIT 1",
    )
    .fetch_optional(database.pool())
    .await?;

    Ok(row.map(|row| UserRecord {
        id: row.get("id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        password_hash: row.get("password_hash"),
        role: row.get("role"),
        last_login_at: row.get("last_login_at"),
    }))
}

pub async fn find_user_by_id(database: &Database, user_id: &str) -> Result<Option<UserRecord>> {
    let row = sqlx::query(
        "SELECT id, email, display_name, password_hash, role, last_login_at
         FROM users WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(database.pool())
    .await?;

    Ok(row.map(|row| UserRecord {
        id: row.get("id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        password_hash: row.get("password_hash"),
        role: row.get("role"),
        last_login_at: row.get("last_login_at"),
    }))
}

pub async fn record_login(database: &Database, user_id: &str) -> Result<()> {
    let now = time::now();
    sqlx::query(
        "UPDATE users SET last_login_at = ?, failed_logins = 0, updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(&now)
    .bind(user_id)
    .execute(database.pool())
    .await?;
    Ok(())
}

/// Store hashed recovery codes. Plaintext exists only in the setup response.
pub async fn store_recovery_codes(
    database: &Database,
    user_id: &str,
    hashes: &[String],
) -> Result<()> {
    let now = time::now();
    let mut transaction = database.pool().begin().await?;
    for hash in hashes {
        sqlx::query(
            "INSERT INTO recovery_codes (id, user_id, code_hash, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(UserId::generate().to_string())
        .bind(user_id)
        .bind(hash)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------- sessions

/// Create a session. Only the token's hash is stored.
pub async fn create_session(
    database: &Database,
    user_id: &str,
    token_hash: &str,
    client_label: &str,
    source_addr: Option<&str>,
    idle_expires_at: &str,
    absolute_expires_at: &str,
) -> Result<String> {
    let id = SessionId::generate().to_string();
    let now = time::now();

    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, client_label, source_addr,
                               created_at, last_seen_at, idle_expires_at, absolute_expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(token_hash)
    .bind(client_label)
    .bind(source_addr)
    .bind(&now)
    .bind(&now)
    .bind(idle_expires_at)
    .bind(absolute_expires_at)
    .execute(database.pool())
    .await?;

    Ok(id)
}

/// Look a session up by the hash of the presented token.
pub async fn find_session_by_token_hash(
    database: &Database,
    token_hash: &str,
) -> Result<Option<SessionRecord>> {
    let row = sqlx::query(
        "SELECT id, user_id, client_label, absolute_expires_at, idle_expires_at, revoked_at
         FROM sessions WHERE token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(database.pool())
    .await?;

    Ok(row.map(|row| SessionRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        client_label: row.get("client_label"),
        absolute_expires_at: row.get("absolute_expires_at"),
        idle_expires_at: row.get("idle_expires_at"),
        revoked_at: row.get("revoked_at"),
    }))
}

/// Push the idle expiry forward on use.
pub async fn touch_session(
    database: &Database,
    session_id: &str,
    idle_expires_at: &str,
) -> Result<()> {
    sqlx::query("UPDATE sessions SET last_seen_at = ?, idle_expires_at = ? WHERE id = ?")
        .bind(time::now())
        .bind(idle_expires_at)
        .bind(session_id)
        .execute(database.pool())
        .await?;
    Ok(())
}

pub async fn revoke_session(database: &Database, session_id: &str) -> Result<()> {
    sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(time::now())
        .bind(session_id)
        .execute(database.pool())
        .await?;
    Ok(())
}

/// Revoke every session for a user. Used by "log out everywhere" and after a
/// password change.
pub async fn revoke_all_sessions(database: &Database, user_id: &str) -> Result<u64> {
    let result =
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
            .bind(time::now())
            .bind(user_id)
            .execute(database.pool())
            .await?;
    Ok(result.rows_affected())
}

/// Delete sessions that expired more than a week ago.
pub async fn prune_expired_sessions(database: &Database, before: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM sessions WHERE absolute_expires_at < ?")
        .bind(before)
        .execute(database.pool())
        .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------- agent state

/// Record that the agent has started. Also reports whether the previous stop
/// was clean — a `false` here is what triggers recovery.
pub async fn begin_agent_session(
    database: &Database,
    agent_version: &str,
    schema_version: u32,
    instance_id: &str,
    bind_address: &str,
) -> Result<bool> {
    let previous_was_clean: Option<i64> =
        sqlx::query_scalar("SELECT last_clean_shutdown FROM agent_state WHERE id = 1")
            .fetch_optional(database.pool())
            .await?;

    let now = time::now();
    sqlx::query(
        "INSERT INTO agent_state (id, agent_version, schema_version, instance_id, started_at,
                                  last_heartbeat_at, last_clean_shutdown, bind_address)
         VALUES (1, ?, ?, ?, ?, ?, 0, ?)
         ON CONFLICT(id) DO UPDATE SET
             agent_version = excluded.agent_version,
             schema_version = excluded.schema_version,
             instance_id = excluded.instance_id,
             started_at = excluded.started_at,
             last_heartbeat_at = excluded.last_heartbeat_at,
             last_clean_shutdown = 0,
             bind_address = excluded.bind_address",
    )
    .bind(agent_version)
    .bind(i64::from(schema_version))
    .bind(instance_id)
    .bind(&now)
    .bind(&now)
    .bind(bind_address)
    .execute(database.pool())
    .await?;

    // No previous row means a fresh install, which is a clean start.
    Ok(previous_was_clean.unwrap_or(1) == 1)
}

pub async fn record_clean_shutdown(database: &Database) -> Result<()> {
    sqlx::query(
        "UPDATE agent_state SET last_clean_shutdown = 1, last_heartbeat_at = ? WHERE id = 1",
    )
    .bind(time::now())
    .execute(database.pool())
    .await?;
    Ok(())
}

pub async fn record_heartbeat(database: &Database) -> Result<()> {
    sqlx::query("UPDATE agent_state SET last_heartbeat_at = ? WHERE id = 1")
        .bind(time::now())
        .execute(database.pool())
        .await?;
    Ok(())
}

/// Fail loudly rather than returning a default: callers that need agent state
/// cannot proceed sensibly without it.
pub async fn agent_instance_id(database: &Database) -> Result<String> {
    sqlx::query_scalar("SELECT instance_id FROM agent_state WHERE id = 1")
        .fetch_optional(database.pool())
        .await?
        .ok_or(DatabaseError::NotFound {
            entity: "agent_state",
        })
}
