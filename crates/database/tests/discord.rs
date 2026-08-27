//! Discord integration storage, against a real SQLite database.
//!
//! In-memory rather than mocked, for the same reason as the other storage
//! tests: several of the rules are `CHECK` constraints in the migration, and a
//! mock cannot enforce them. The ones worth stating plainly are that a bot
//! token has nowhere to live unencrypted, and that a project's control panel
//! cannot be pointed at its own log channel.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use project_host_api_types::ProjectType;
use project_host_database::discord::{self, BotCredentials, NewChannels, NewGuildLink};
use project_host_database::projects::{self, NewPort, NewProcess, NewProject, RuntimeSpec};
use project_host_database::{schema_parity, Database, DISCORD_MIGRATION};
use project_host_discord::{ChannelKind, EventKind, Permission};

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

/// A distinct host port per slug; `project_ports` is unique on it.
fn host_port_for(slug: &str) -> i64 {
    let sum: i64 = slug.bytes().map(i64::from).sum::<i64>() * 7 + slug.len() as i64 * 131;
    20_000 + (sum % 20_000)
}

async fn a_project(database: &Database, slug: &str) -> String {
    let project = projects::create_project(
        database,
        &NewProject {
            slug: slug.to_string(),
            display_name: "My Bot".to_string(),
            description: "a bot".to_string(),
            project_type: ProjectType::DiscordBot,
            icon: None,
            color: None,
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
                host_port: Some(host_port_for(slug)),
                protocol: "tcp".to_string(),
                bind_address: "127.0.0.1".to_string(),
                is_primary: true,
                process_id: None,
            }],
        },
    )
    .await
    .expect("create project");
    project.id
}

async fn a_guild(database: &Database, guild_id: &str) -> String {
    let bot = discord::add_bot(database, &credentials())
        .await
        .expect("bot");
    discord::link_guild(
        database,
        &NewGuildLink {
            guild_id: guild_id.to_string(),
            guild_name: "My Server".to_string(),
            linked_by_user_id: "111111111111111111".to_string(),
            allow_guild_owner: true,
            bot_row_id: bot,
        },
    )
    .await
    .expect("link guild")
}

fn credentials() -> BotCredentials {
    BotCredentials {
        label: "My Bot".to_string(),
        application_id: "999999999999999999".to_string(),
        token_cipher: vec![7u8; 64],
        // XChaCha20-Poly1305 nonces are 24 bytes, and the schema says so.
        token_nonce: vec![3u8; 24],
    }
}

// -------------------------------------------------------------- bot credentials

#[tokio::test]
async fn bot_credentials_round_trip_as_ciphertext() {
    let database = db().await;
    let id = discord::add_bot(&database, &credentials())
        .await
        .expect("save");

    let loaded = discord::find_bot(&database, &id)
        .await
        .expect("load")
        .expect("present");
    assert_eq!(loaded.application_id, credentials().application_id);
    assert_eq!(loaded.token_cipher, credentials().token_cipher);
    assert_eq!(loaded.token_nonce, credentials().token_nonce);
    assert_eq!(loaded.label, "My Bot");
    assert!(!loaded.autostart, "a new bot does not start on its own");
}

#[tokio::test]
async fn there_is_nowhere_to_store_a_bot_token_in_the_clear() {
    // The structural claim: no column on `discord_bots` could hold a readable
    // token, so a future writer would have to alter the table to leak one.
    // Widening the table to many rows in 0006 must not have weakened it.
    let database = db().await;
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('discord_bots') ORDER BY name")
            .fetch_all(database.pool())
            .await
            .expect("pragma");

    assert_eq!(
        columns,
        vec![
            "application_id".to_string(),
            "autostart".to_string(),
            "created_at".to_string(),
            "id".to_string(),
            "label".to_string(),
            "token_cipher".to_string(),
            "token_nonce".to_string(),
            "updated_at".to_string(),
        ]
    );
}

#[tokio::test]
async fn a_nonce_of_the_wrong_length_is_refused_by_the_database() {
    // A 12-byte nonce would mean somebody had swapped the cipher for one this
    // application does not use, and the ciphertext would be undecryptable.
    let database = db().await;
    let wrong = BotCredentials {
        token_nonce: vec![3u8; 12],
        ..credentials()
    };
    assert!(discord::add_bot(&database, &wrong).await.is_err());
}

#[tokio::test]
async fn re_adding_the_same_application_rotates_rather_than_duplicating() {
    // Two rows for one Discord application would mean two connections
    // identifying as the same bot, which Discord answers by closing one.
    let database = db().await;
    let first = discord::add_bot(&database, &credentials())
        .await
        .expect("save");

    let rotated = BotCredentials {
        token_cipher: vec![9u8; 64],
        ..credentials()
    };
    let second = discord::add_bot(&database, &rotated).await.expect("rotate");
    assert_eq!(first, second, "the same row should be reused");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM discord_bots")
        .fetch_one(database.pool())
        .await
        .expect("count");
    assert_eq!(count, 1);

    let loaded = discord::find_bot(&database, &first)
        .await
        .expect("load")
        .expect("present");
    assert_eq!(loaded.token_cipher, vec![9u8; 64]);
}

#[tokio::test]
async fn a_second_distinct_bot_is_kept_alongside_the_first() {
    // The point of 0006: an installation may hold more than one.
    let database = db().await;
    discord::add_bot(&database, &credentials())
        .await
        .expect("first");

    let other = BotCredentials {
        label: "Private Bot".to_string(),
        application_id: "888888888888888888".to_string(),
        ..credentials()
    };
    discord::add_bot(&database, &other).await.expect("second");

    let bots = discord::list_bots(&database).await.expect("list");
    assert_eq!(bots.len(), 2);
    assert_eq!(bots[0].label, "My Bot", "oldest first");
    assert_eq!(bots[1].label, "Private Bot");
}

#[tokio::test]
async fn forgetting_a_bot_takes_its_linked_servers_with_it() {
    // The cascade the migration declares: a server whose bot is gone has
    // nothing left that could reach it.
    let database = db().await;
    let bot = discord::add_bot(&database, &credentials())
        .await
        .expect("bot");
    discord::link_guild(
        &database,
        &NewGuildLink {
            guild_id: "555555555555555555".to_string(),
            guild_name: "My Server".to_string(),
            linked_by_user_id: "111111111111111111".to_string(),
            allow_guild_owner: true,
            bot_row_id: bot.clone(),
        },
    )
    .await
    .expect("link");

    assert!(discord::forget_bot(&database, &bot).await.expect("forget"));
    assert!(discord::list_guilds(&database)
        .await
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn forgetting_credentials_removes_the_row_entirely() {
    let database = db().await;
    let id = discord::add_bot(&database, &credentials())
        .await
        .expect("save");
    assert!(discord::forget_bot(&database, &id).await.expect("forget"));
    assert_eq!(discord::find_bot(&database, &id).await.expect("load"), None);
}

// ---------------------------------------------------------------------- guilds

#[tokio::test]
async fn linking_the_same_server_twice_updates_it_rather_than_failing() {
    // The common case is a user re-running the link flow after renaming their
    // server. Failing with a uniqueness error would be unhelpful.
    let database = db().await;
    let first = a_guild(&database, "222222222222222222").await;
    let bot = discord::list_bots(&database).await.expect("bots")[0]
        .id
        .clone();

    let second = discord::link_guild(
        &database,
        &NewGuildLink {
            guild_id: "222222222222222222".to_string(),
            guild_name: "Renamed Server".to_string(),
            linked_by_user_id: "111111111111111111".to_string(),
            allow_guild_owner: false,
            bot_row_id: bot,
        },
    )
    .await
    .expect("relink");

    assert_eq!(first, second, "the same row");
    let guild = discord::find_guild(&database, "222222222222222222")
        .await
        .expect("find")
        .expect("present");
    assert_eq!(guild.guild_name, "Renamed Server");
    assert!(!guild.allow_guild_owner);
}

#[tokio::test]
async fn a_discord_id_is_stored_as_text_so_it_cannot_be_rounded() {
    // Snowflakes exceed 2^53. Stored as an INTEGER and read through any layer
    // that uses a float, the last digits would change.
    let database = db().await;
    let precise = "1234567890123456789";
    a_guild(&database, precise).await;

    let guild = discord::find_guild(&database, precise)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(guild.guild_id, precise);
}

#[tokio::test]
async fn unlinking_a_server_removes_everything_that_belonged_to_it() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    discord::upsert_grant(&database, &guild, "role", "333", "operate")
        .await
        .expect("grant");
    discord::block_user(&database, &guild, "444", None)
        .await
        .expect("block");
    discord::set_channel_template(&database, &guild, "logs", "{slug}-out")
        .await
        .expect("template");
    discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild.clone(),
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "my-bot-logs".to_string(),
            control_channel_name: "my-bot-control".to_string(),
        },
        &["started".to_string()],
    )
    .await
    .expect("channels");

    assert!(discord::unlink_guild(&database, &guild)
        .await
        .expect("unlink"));

    assert!(discord::list_grants(&database, &guild)
        .await
        .unwrap()
        .is_empty());
    assert!(discord::list_blocked_users(&database, &guild)
        .await
        .unwrap()
        .is_empty());
    assert!(discord::list_channel_templates(&database, &guild)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        discord::find_channels(&database, &project).await.unwrap(),
        None,
        "channel rows should cascade with their server"
    );
    assert!(
        discord::list_enabled_events(&database, &project)
            .await
            .unwrap()
            .is_empty(),
        "event rows should cascade with their channel row"
    );
}

// ---------------------------------------------------------------------- grants

#[tokio::test]
async fn a_second_grant_for_the_same_subject_replaces_the_first() {
    // Two rows for one role would make "the highest grant wins" depend on the
    // order rows happened to come back in.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;

    discord::upsert_grant(&database, &guild, "role", "333", "view")
        .await
        .expect("grant");
    discord::upsert_grant(&database, &guild, "role", "333", "administer")
        .await
        .expect("regrant");

    let grants = discord::list_grants(&database, &guild).await.expect("list");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].level, "administer");
}

#[tokio::test]
async fn a_role_and_a_user_with_the_same_id_are_separate_grants() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;

    discord::upsert_grant(&database, &guild, "role", "333", "view")
        .await
        .expect("role grant");
    discord::upsert_grant(&database, &guild, "user", "333", "operate")
        .await
        .expect("user grant");

    assert_eq!(
        discord::list_grants(&database, &guild).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn an_unknown_permission_level_is_refused_before_it_reaches_sqlite() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;

    let error = discord::upsert_grant(&database, &guild, "role", "333", "superuser")
        .await
        .expect_err("unknown level");
    assert!(error.to_string().contains("superuser"), "got {error}");

    let error = discord::upsert_grant(&database, &guild, "channel", "333", "view")
        .await
        .expect_err("unknown subject");
    assert!(error.to_string().contains("channel"), "got {error}");
}

#[tokio::test]
async fn a_removed_grant_is_gone() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let id = discord::upsert_grant(&database, &guild, "role", "333", "view")
        .await
        .expect("grant");

    assert!(discord::remove_grant(&database, &id).await.expect("remove"));
    assert!(!discord::remove_grant(&database, &id).await.expect("again"));
    assert!(discord::list_grants(&database, &guild)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn blocking_the_same_user_twice_updates_the_reason() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;

    discord::block_user(&database, &guild, "444", Some("spamming"))
        .await
        .expect("block");
    discord::block_user(&database, &guild, "444", Some("still spamming"))
        .await
        .expect("reblock");

    let blocked = discord::list_blocked_users(&database, &guild)
        .await
        .expect("list");
    assert_eq!(blocked, vec!["444".to_string()]);

    assert!(discord::unblock_user(&database, &guild, "444")
        .await
        .expect("unblock"));
    assert!(discord::list_blocked_users(&database, &guild)
        .await
        .unwrap()
        .is_empty());
}

// -------------------------------------------------------------------- channels

#[tokio::test]
async fn recording_channels_writes_the_events_in_the_same_transaction() {
    // A channel row without its event rows is a project whose channels exist
    // and which silently reports nothing.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    let events: Vec<String> = EventKind::sensible_defaults()
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect();

    discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "my-bot-logs".to_string(),
            control_channel_name: "my-bot-control".to_string(),
        },
        &events,
    )
    .await
    .expect("record");

    let stored = discord::find_channels(&database, &project)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(stored.logs_channel_id, "555");
    assert!(stored.enabled, "a newly linked project reports by default");
    assert_eq!(stored.batch_window_ms, 2000);

    let mut enabled = discord::list_enabled_events(&database, &project)
        .await
        .expect("events");
    enabled.sort();
    let mut expected = events;
    expected.sort();
    assert_eq!(enabled, expected);
}

#[tokio::test]
async fn a_project_cannot_send_its_control_panel_to_its_own_log_channel() {
    // It would be buried under the logs within minutes.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    let result = discord::record_channels(
        &database,
        &NewChannels {
            project_id: project,
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "555".to_string(),
            logs_channel_name: "my-bot-logs".to_string(),
            control_channel_name: "my-bot-logs".to_string(),
        },
        &[],
    )
    .await;

    assert!(result.is_err(), "the CHECK should refuse this");
}

#[tokio::test]
async fn two_projects_cannot_share_a_log_channel() {
    // Otherwise one channel would carry two projects' logs interleaved, and
    // neither project's panel would be trustworthy.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let first = a_project(&database, "first-bot").await;
    let second = a_project(&database, "second-bot").await;

    discord::record_channels(
        &database,
        &NewChannels {
            project_id: first,
            guild_row_id: guild.clone(),
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &[],
    )
    .await
    .expect("first");

    let result = discord::record_channels(
        &database,
        &NewChannels {
            project_id: second,
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "557".to_string(),
            logs_channel_name: "b-logs".to_string(),
            control_channel_name: "b-control".to_string(),
        },
        &[],
    )
    .await;

    assert!(result.is_err(), "the UNIQUE should refuse this");
}

#[tokio::test]
async fn an_unknown_event_kind_is_refused_by_the_database() {
    // The CHECK is the last line: this is what stops a typo in a settings
    // payload becoming a silently disabled notification.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    let result = discord::record_channels(
        &database,
        &NewChannels {
            project_id: project,
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &["exploded".to_string()],
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn a_failed_event_write_leaves_no_channel_row_behind() {
    // The transaction under test: the bad event is the second statement.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    let _ = discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &["started".to_string(), "exploded".to_string()],
    )
    .await;

    assert_eq!(
        discord::find_channels(&database, &project).await.unwrap(),
        None,
        "the channel row should have rolled back with the events"
    );
}

#[tokio::test]
async fn muting_a_project_does_not_discard_its_event_list() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &["started".to_string(), "crashed".to_string()],
    )
    .await
    .expect("record");

    discord::update_notification_settings(&database, &project, Some(false), None, None)
        .await
        .expect("mute");

    let stored = discord::find_channels(&database, &project)
        .await
        .unwrap()
        .expect("present");
    assert!(!stored.enabled);
    assert_eq!(
        discord::list_enabled_events(&database, &project)
            .await
            .unwrap()
            .len(),
        2,
        "the configuration should survive being muted"
    );
}

#[tokio::test]
async fn a_mention_role_can_be_set_and_then_cleared() {
    // "Leave alone" and "clear it" are different requests, and NULL cannot mean
    // both. This is the test that keeps them apart.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &[],
    )
    .await
    .expect("record");

    discord::update_notification_settings(&database, &project, None, Some(Some("777")), None)
        .await
        .expect("set mention");
    assert_eq!(
        discord::find_channels(&database, &project)
            .await
            .unwrap()
            .unwrap()
            .mention_role_on_failure,
        Some("777".to_string())
    );

    // A settings update that does not mention the role must leave it alone.
    discord::update_notification_settings(&database, &project, Some(true), None, Some(5000))
        .await
        .expect("unrelated update");
    assert_eq!(
        discord::find_channels(&database, &project)
            .await
            .unwrap()
            .unwrap()
            .mention_role_on_failure,
        Some("777".to_string()),
        "an unrelated update should not have cleared it"
    );

    discord::update_notification_settings(&database, &project, None, Some(None), None)
        .await
        .expect("clear mention");
    assert_eq!(
        discord::find_channels(&database, &project)
            .await
            .unwrap()
            .unwrap()
            .mention_role_on_failure,
        None
    );
}

#[tokio::test]
async fn an_absurd_batch_window_is_refused_by_the_database() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &[],
    )
    .await
    .expect("record");

    // Ten minutes of batching would make the log channel useless.
    assert!(
        discord::update_notification_settings(&database, &project, None, None, Some(600_000))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn deleting_a_project_takes_its_discord_channels_with_it() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &["started".to_string()],
    )
    .await
    .expect("record");

    projects::begin_delete(&database, &project)
        .await
        .expect("begin delete");
    projects::finish_delete(&database, &project)
        .await
        .expect("finish delete");

    assert_eq!(
        discord::find_channels(&database, &project).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn the_control_message_id_can_be_recorded_and_cleared() {
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;
    let project = a_project(&database, "my-bot").await;

    discord::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "555".to_string(),
            control_channel_id: "556".to_string(),
            logs_channel_name: "a-logs".to_string(),
            control_channel_name: "a-control".to_string(),
        },
        &[],
    )
    .await
    .expect("record");

    discord::set_control_message(&database, &project, Some("888"))
        .await
        .expect("set");
    assert_eq!(
        discord::find_channels(&database, &project)
            .await
            .unwrap()
            .unwrap()
            .control_message_id,
        Some("888".to_string())
    );

    // Cleared when the message is deleted in Discord, so the next update posts
    // a fresh panel instead of failing to edit a message that is gone.
    discord::set_control_message(&database, &project, None)
        .await
        .expect("clear");
    assert_eq!(
        discord::find_channels(&database, &project)
            .await
            .unwrap()
            .unwrap()
            .control_message_id,
        None
    );
}

// --------------------------------------------------------------- schema parity

#[tokio::test]
async fn every_event_kind_is_accepted_by_the_database() {
    // The list in the migration and the Rust enum are two copies of the same
    // thing, and two copies drift. This turns drift into a failing test.
    let body = schema_parity::table_body(DISCORD_MIGRATION, "discord_enabled_events")
        .expect("table exists");
    let mut allowed = schema_parity::check_values(body, "event_kind").expect("a CHECK list");
    allowed.sort();

    let mut expected: Vec<String> = EventKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect();
    expected.sort();

    assert_eq!(allowed, expected);
}

#[tokio::test]
async fn every_permission_level_is_accepted_by_the_database() {
    let body = schema_parity::table_body(DISCORD_MIGRATION, "discord_grants").expect("table");
    let mut allowed = schema_parity::check_values(body, "level").expect("a CHECK list");
    allowed.sort();

    let mut expected: Vec<String> = [
        Permission::View,
        Permission::Operate,
        Permission::Administer,
    ]
    .iter()
    .map(|level| level.as_str().to_string())
    .collect();
    expected.sort();

    assert_eq!(allowed, expected);
}

#[tokio::test]
async fn every_channel_kind_is_accepted_by_the_database() {
    let body =
        schema_parity::table_body(DISCORD_MIGRATION, "discord_channel_templates").expect("table");
    let mut allowed = schema_parity::check_values(body, "kind").expect("a CHECK list");
    allowed.sort();

    let mut expected: Vec<String> = ChannelKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect();
    expected.sort();

    assert_eq!(allowed, expected);
}

#[tokio::test]
async fn a_default_channel_template_is_short_enough_for_the_schema() {
    // The migration caps a template at 100 characters. A default that could not
    // be stored would fail on first use rather than in a test.
    let database = db().await;
    let guild = a_guild(&database, "222222222222222222").await;

    for kind in ChannelKind::ALL {
        discord::set_channel_template(&database, &guild, kind.as_str(), kind.default_template())
            .await
            .unwrap_or_else(|error| panic!("{kind:?} default rejected: {error}"));
    }
}
