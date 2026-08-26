//! Adding, listing, starting and stopping Discord bots.
//!
//! The composition point for the connection, the way [`crate::integration`] is
//! for the stored rules: `database` holds the rows, `discord-gateway` owns the
//! socket, `security` holds the key, and none of them knows about the others.
//!
//! # Why a token is checked before it is stored
//!
//! [`add_bot`] calls Discord once before writing anything. A token that is
//! wrong is overwhelmingly the common failure — pasted with a space, copied
//! from the wrong application, reset since — and finding out at *store* time
//! means the user is told immediately, next to the field they just filled.
//! Storing first and failing later means a bot that sits in the list looking
//! configured and fails every time it is started.
//!
//! It also supplies the application id, which the user should not have to
//! find and paste separately when Discord will state it.

use project_host_database::discord as storage;
use project_host_database::{Database, DatabaseError};
use project_host_discord_gateway::rest::DiscordRest;
use project_host_discord_gateway::{
    BotToken, ConnectionConfig, ConnectionStatus, GatewayError, Supervisor,
};
use project_host_security::{EncryptionKey, Secret};

use crate::integration::{self, IntegrationError};

#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error("database error: {0}")]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    Integration(#[from] IntegrationError),
    #[error("{0}")]
    Gateway(GatewayError),
    #[error(
        "this installation has no encryption key, so a bot token cannot be stored safely. \
         Check the application log for why secure storage could not be opened."
    )]
    NoKey,
    #[error("no bot with id {0}")]
    NoSuchBot(String),
    #[error("no project with id {0}")]
    NoSuchProject(String),
}

impl From<GatewayError> for BotError {
    fn from(error: GatewayError) -> Self {
        BotError::Gateway(error)
    }
}

impl BotError {
    /// What to show the user. Never carries a token.
    pub fn user_facing(&self) -> String {
        match self {
            BotError::Gateway(error) => error.user_facing(),
            other => other.to_string(),
        }
    }
}

/// One bot, as the window needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotView {
    pub id: String,
    pub label: String,
    pub application_id: String,
    pub autostart: bool,
    /// `stopped`, `connecting`, `connected` or `failed`.
    pub status: String,
    pub failure_reason: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub reconnects: u32,
    /// How many servers are linked through this bot.
    pub linked_servers: u32,
    /// The projects this bot reports on.
    pub project_ids: Vec<String>,
}

/// Every configured bot, with what its connection is doing right now.
///
/// Reads the list from the database rather than from the supervisor: a bot that
/// has never been started must still appear, or the user cannot start it.
pub async fn list_bots(db: &Database, supervisor: &Supervisor) -> Result<Vec<BotView>, BotError> {
    let stored = storage::list_bots(db).await?;
    let guilds = storage::list_guilds(db).await?;

    // Gathered before the map so the closure stays synchronous. One query per
    // bot, and the count is small — a user with enough bots for this to matter
    // has a different problem.
    let mut selections = Vec::with_capacity(stored.len());
    for bot in &stored {
        selections.push(storage::list_bot_projects(db, &bot.id).await?);
    }

    Ok(stored
        .into_iter()
        .zip(selections)
        .map(|(bot, project_ids)| {
            let observed = supervisor.observe(&bot.id);
            let linked_servers = guilds
                .iter()
                .filter(|guild| guild.bot_row_id.as_deref() == Some(bot.id.as_str()))
                .count();

            BotView {
                status: observed.status.as_str().to_string(),
                failure_reason: observed.failure_reason,
                uptime_seconds: observed.uptime.map(|elapsed| elapsed.as_secs()),
                reconnects: observed.reconnects,
                linked_servers: u32::try_from(linked_servers).unwrap_or(u32::MAX),
                project_ids,
                id: bot.id,
                label: bot.label,
                application_id: bot.application_id,
                autostart: bot.autostart,
            }
        })
        .collect())
}

/// Check a token with Discord, then store it encrypted.
///
/// Returns the new bot's row id.
pub async fn add_bot<R: DiscordRest>(
    db: &Database,
    key: Option<&EncryptionKey>,
    rest: &R,
    label: &str,
    token: Secret<String>,
) -> Result<String, BotError> {
    let Some(key) = key else {
        return Err(BotError::NoKey);
    };

    let bot_token = BotToken::new(token.expose().trim());
    if bot_token.is_empty() {
        return Err(BotError::Gateway(GatewayError::InvalidToken));
    }

    // The one network call before anything is written.
    let who = rest.current_user(&bot_token).await?;

    // A blank label is the likely case when the user just pastes a token, and
    // "Unnamed bot" in a list is less useful than what Discord calls it.
    let label = if label.trim().is_empty() {
        who.username.clone()
    } else {
        label.trim().to_string()
    };

    let id = integration::add_bot(
        db,
        key,
        &label,
        &who.id,
        &Secret::new(bot_token.expose().to_string()),
    )
    .await?;

    Ok(id)
}

/// Start a bot's connection.
pub async fn start_bot(
    db: &Database,
    key: Option<&EncryptionKey>,
    supervisor: &Supervisor,
    bot_row_id: &str,
) -> Result<(), BotError> {
    let Some(key) = key else {
        return Err(BotError::NoKey);
    };

    let Some((_application_id, token)) = integration::load_bot_token(db, key, bot_row_id).await?
    else {
        return Err(BotError::NoSuchBot(bot_row_id.to_string()));
    };

    supervisor.start(ConnectionConfig {
        bot_row_id: bot_row_id.to_string(),
        token: BotToken::new(token.expose().to_string()),
    })?;

    Ok(())
}

/// Stop a bot's connection. Stopping one that is not running is success.
pub async fn stop_bot(supervisor: &Supervisor, bot_row_id: &str) {
    supervisor.stop(bot_row_id).await;
}

/// Forget a bot: stop it, then delete it and everything linked through it.
///
/// Stopped first. Deleting the row of a live connection would leave a session
/// running that nothing in the window can see or stop.
pub async fn forget_bot(
    db: &Database,
    supervisor: &Supervisor,
    bot_row_id: &str,
) -> Result<bool, BotError> {
    supervisor.stop(bot_row_id).await;
    Ok(storage::forget_bot(db, bot_row_id).await?)
}

/// Rename a bot, or change whether it starts with the application.
pub async fn update_bot(
    db: &Database,
    bot_row_id: &str,
    label: &str,
    autostart: bool,
) -> Result<bool, BotError> {
    let label = label.trim();
    if label.is_empty() {
        return Ok(false);
    }
    Ok(storage::update_bot(db, bot_row_id, label, autostart).await?)
}

/// Choose which projects a bot reports on.
///
/// The whole set is replaced rather than adding or removing one at a time: the
/// window shows a list of tick boxes, so the thing the user produced is a set,
/// and sending it as one avoids a half-applied selection if a call is lost.
///
/// Unknown project ids are refused rather than skipped. Silently dropping one
/// means the window shows a project as covered that the core never recorded.
pub async fn set_bot_projects(
    db: &Database,
    bot_row_id: &str,
    project_ids: &[String],
) -> Result<(), BotError> {
    if storage::find_bot(db, bot_row_id).await?.is_none() {
        return Err(BotError::NoSuchBot(bot_row_id.to_string()));
    }

    for id in project_ids {
        if project_host_database::projects::find_project(db, id)
            .await?
            .is_none()
        {
            return Err(BotError::NoSuchProject(id.clone()));
        }
    }

    storage::set_bot_projects(db, bot_row_id, project_ids).await?;
    Ok(())
}

/// Start every bot marked `autostart`, at application start.
///
/// Failures are logged rather than returned: one bot with a revoked token must
/// not stop the others from connecting, and must not stop the window opening.
pub async fn start_autostart_bots(
    db: &Database,
    key: Option<&EncryptionKey>,
    supervisor: &Supervisor,
) {
    let bots = match storage::list_bots(db).await {
        Ok(bots) => bots,
        Err(error) => {
            tracing::error!(%error, "could not read the bot list at startup");
            return;
        }
    };

    for bot in bots.into_iter().filter(|bot| bot.autostart) {
        if let Err(error) = start_bot(db, key, supervisor, &bot.id).await {
            tracing::error!(bot = %bot.id, error = %error.user_facing(), "autostart failed");
        }
    }
}

/// How many bots are connected right now.
pub fn connected_count(supervisor: &Supervisor) -> usize {
    supervisor.connected_count()
}

/// Whether a status string names a live connection.
pub fn is_live(status: &str) -> bool {
    status == ConnectionStatus::Connected.as_str()
        || status == ConnectionStatus::Connecting.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_host_discord_gateway::rest::{
        BotUser, CreatedChannel, PartialGuild, PostedMessage,
    };
    use std::sync::Mutex;

    /// A Discord that answers however the test needs it to.
    #[derive(Debug)]
    struct FakeRest {
        answer: Mutex<Result<BotUser, u16>>,
        calls: Mutex<u32>,
    }

    impl FakeRest {
        fn accepting() -> Self {
            Self {
                answer: Mutex::new(Ok(BotUser {
                    id: "999999999999999999".to_string(),
                    username: "Panel Bot".to_string(),
                })),
                calls: Mutex::new(0),
            }
        }

        fn rejecting() -> Self {
            Self {
                answer: Mutex::new(Err(401)),
                calls: Mutex::new(0),
            }
        }
    }

    impl DiscordRest for FakeRest {
        async fn current_user(&self, _token: &BotToken) -> Result<BotUser, GatewayError> {
            *self.calls.lock().unwrap() += 1;
            match &*self.answer.lock().unwrap() {
                Ok(user) => Ok(user.clone()),
                Err(_) => Err(GatewayError::InvalidToken),
            }
        }

        async fn current_user_guilds(
            &self,
            _token: &BotToken,
        ) -> Result<Vec<PartialGuild>, GatewayError> {
            Ok(Vec::new())
        }

        async fn create_text_channel(
            &self,
            _token: &BotToken,
            _guild_id: &str,
            name: &str,
        ) -> Result<CreatedChannel, GatewayError> {
            Ok(CreatedChannel {
                id: "1".to_string(),
                name: name.to_string(),
            })
        }

        async fn post_message(
            &self,
            _token: &BotToken,
            _channel_id: &str,
            _content: &str,
        ) -> Result<PostedMessage, GatewayError> {
            Ok(PostedMessage {
                id: "1".to_string(),
            })
        }

        async fn edit_message(
            &self,
            _token: &BotToken,
            _channel_id: &str,
            _message_id: &str,
            _content: &str,
        ) -> Result<PostedMessage, GatewayError> {
            Ok(PostedMessage {
                id: "1".to_string(),
            })
        }
    }

    async fn db() -> Database {
        Database::open_in_memory().await.expect("in-memory")
    }

    /// A project to attach a bot to.
    async fn a_project(database: &Database, slug: &str) -> String {
        use project_host_api_types::ProjectType;
        use project_host_database::projects::{self, NewProcess, NewProject, RuntimeSpec};

        let project = projects::create_project(
            database,
            &NewProject {
                slug: slug.to_string(),
                display_name: slug.to_string(),
                description: String::new(),
                project_type: ProjectType::DiscordBot,
                icon: None,
                color: None,
                source_type: "EMPTY".to_string(),
                directory: format!("/var/lib/project-host/projects/{slug}"),
                source_url: None,
                source_ref: None,
                source_commit: None,
                autostart: false,
                restart_policy: "UNLESS_STOPPED".to_string(),
                network_mode: "INTERNET".to_string(),
                memory_limit_mb: 512,
                cpu_limit_cores: 1.0,
                storage_limit_mb: 2048,
                process_limit: 128,
                runtime: RuntimeSpec {
                    runtime: "NODEJS".to_string(),
                    runtime_version: "22".to_string(),
                    package_manager: "NPM".to_string(),
                    entry_file: Some("index.js".to_string()),
                    publish_dir: None,
                    template_id: "nodejs".to_string(),
                },
                processes: vec![NewProcess::simple("main", 0, "node index.js")],
                ports: Vec::new(),
            },
        )
        .await
        .expect("create project");
        project.id
    }

    #[tokio::test]
    async fn a_valid_token_is_verified_then_stored() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let rest = FakeRest::accepting();

        let id = add_bot(
            &database,
            Some(&key),
            &rest,
            "",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("should be accepted");

        assert_eq!(
            *rest.calls.lock().unwrap(),
            1,
            "Discord should be asked once"
        );

        let stored = storage::find_bot(&database, &id)
            .await
            .expect("query")
            .expect("present");
        assert_eq!(
            stored.application_id, "999999999999999999",
            "the application id should come from Discord, not the user"
        );
        assert_eq!(
            stored.label, "Panel Bot",
            "a blank label should fall back to the bot's own name"
        );
    }

    #[tokio::test]
    async fn a_rejected_token_is_never_written() {
        // The failure this prevents: a bot that sits in the list looking
        // configured and fails every time it is started.
        let database = db().await;
        let key = EncryptionKey::generate();

        let error = add_bot(
            &database,
            Some(&key),
            &FakeRest::rejecting(),
            "My Bot",
            Secret::new("a-wrong-token".to_string()),
        )
        .await
        .expect_err("should be refused");

        assert!(matches!(
            error,
            BotError::Gateway(GatewayError::InvalidToken)
        ));
        assert!(
            storage::list_bots(&database)
                .await
                .expect("list")
                .is_empty(),
            "nothing should have been stored"
        );
    }

    #[tokio::test]
    async fn a_blank_token_is_refused_without_asking_discord() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let rest = FakeRest::accepting();

        let error = add_bot(
            &database,
            Some(&key),
            &rest,
            "My Bot",
            Secret::new("   ".to_string()),
        )
        .await
        .expect_err("should be refused");

        assert!(matches!(
            error,
            BotError::Gateway(GatewayError::InvalidToken)
        ));
        assert_eq!(
            *rest.calls.lock().unwrap(),
            0,
            "a blank token should not cost a request"
        );
    }

    #[tokio::test]
    async fn without_a_key_a_token_is_refused_rather_than_stored_in_the_clear() {
        let database = db().await;

        let error = add_bot(
            &database,
            None,
            &FakeRest::accepting(),
            "My Bot",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect_err("should be refused");

        assert!(matches!(error, BotError::NoKey));
        assert!(storage::list_bots(&database)
            .await
            .expect("list")
            .is_empty());
    }

    #[tokio::test]
    async fn a_bot_that_was_never_started_lists_as_stopped() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let supervisor = Supervisor::new();

        add_bot(
            &database,
            Some(&key),
            &FakeRest::accepting(),
            "My Bot",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("add");

        let bots = list_bots(&database, &supervisor).await.expect("list");
        assert_eq!(bots.len(), 1);
        assert_eq!(bots[0].status, "stopped");
        assert_eq!(bots[0].linked_servers, 0);
        assert!(bots[0].failure_reason.is_none());
    }

    #[tokio::test]
    async fn a_bot_covers_the_projects_it_is_given() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let supervisor = Supervisor::new();

        let bot = add_bot(
            &database,
            Some(&key),
            &FakeRest::accepting(),
            "My Bot",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("add");

        let first = a_project(&database, "alpha").await;
        let second = a_project(&database, "beta").await;

        set_bot_projects(&database, &bot, &[first.clone(), second.clone()])
            .await
            .expect("select");

        let bots = list_bots(&database, &supervisor).await.expect("list");
        assert_eq!(bots[0].project_ids, vec![first.clone(), second.clone()]);

        // Unticking one must leave the other alone.
        set_bot_projects(&database, &bot, std::slice::from_ref(&second))
            .await
            .expect("reduce");
        let bots = list_bots(&database, &supervisor).await.expect("list");
        assert_eq!(bots[0].project_ids, vec![second]);
    }

    #[tokio::test]
    async fn selecting_no_projects_clears_the_set() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let supervisor = Supervisor::new();

        let bot = add_bot(
            &database,
            Some(&key),
            &FakeRest::accepting(),
            "My Bot",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("add");
        let project = a_project(&database, "alpha").await;

        set_bot_projects(&database, &bot, &[project])
            .await
            .expect("select");
        set_bot_projects(&database, &bot, &[]).await.expect("clear");

        let bots = list_bots(&database, &supervisor).await.expect("list");
        assert!(
            bots[0].project_ids.is_empty(),
            "an empty selection should mean none, not unchanged"
        );
    }

    #[tokio::test]
    async fn a_project_that_does_not_exist_is_refused() {
        // Skipping it would show a project as covered that was never recorded.
        let database = db().await;
        let key = EncryptionKey::generate();

        let bot = add_bot(
            &database,
            Some(&key),
            &FakeRest::accepting(),
            "My Bot",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("add");

        let error = set_bot_projects(&database, &bot, &["prj_nonexistent".to_string()])
            .await
            .expect_err("should refuse");
        assert!(matches!(error, BotError::NoSuchProject(_)));
    }

    #[tokio::test]
    async fn forgetting_a_bot_drops_its_project_selection() {
        // The cascade in 0007. A selection pointing at a deleted bot would be
        // a row nothing can reach and nothing cleans up.
        let database = db().await;
        let key = EncryptionKey::generate();
        let supervisor = Supervisor::new();

        let bot = add_bot(
            &database,
            Some(&key),
            &FakeRest::accepting(),
            "My Bot",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("add");
        let project = a_project(&database, "alpha").await;
        set_bot_projects(&database, &bot, std::slice::from_ref(&project))
            .await
            .expect("select");

        forget_bot(&database, &supervisor, &bot)
            .await
            .expect("forget");

        assert!(storage::bots_for_project(&database, &project)
            .await
            .expect("query")
            .is_empty());
    }

    #[tokio::test]
    async fn starting_a_bot_that_does_not_exist_says_so() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let supervisor = Supervisor::new();

        let error = start_bot(&database, Some(&key), &supervisor, "bot_missing")
            .await
            .expect_err("should fail");
        assert!(matches!(error, BotError::NoSuchBot(_)));
    }

    #[tokio::test]
    async fn forgetting_a_bot_removes_it() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let supervisor = Supervisor::new();

        let id = add_bot(
            &database,
            Some(&key),
            &FakeRest::accepting(),
            "My Bot",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("add");

        assert!(forget_bot(&database, &supervisor, &id)
            .await
            .expect("forget"));
        assert!(list_bots(&database, &supervisor)
            .await
            .expect("list")
            .is_empty());
    }

    #[tokio::test]
    async fn renaming_a_bot_keeps_its_token() {
        let database = db().await;
        let key = EncryptionKey::generate();

        let id = add_bot(
            &database,
            Some(&key),
            &FakeRest::accepting(),
            "Old Name",
            Secret::new("a-real-looking-token".to_string()),
        )
        .await
        .expect("add");

        assert!(update_bot(&database, &id, "New Name", true)
            .await
            .expect("rename"));

        let (_, token) = integration::load_bot_token(&database, &key, &id)
            .await
            .expect("load")
            .expect("present");
        assert_eq!(
            token.expose(),
            "a-real-looking-token",
            "renaming must not disturb the stored token"
        );
    }
}
