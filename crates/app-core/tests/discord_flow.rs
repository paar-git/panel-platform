//! The Discord path end to end, minus the network.
//!
//! The unit tests in the `discord` crate prove each rule in isolation. This
//! proves the rules and the storage actually join up: rows written by the
//! settings screen are read back, assembled into a policy, used to authorise a
//! real button press, and turned into a message that is safe to send.
//!
//! Everything here is real — a real SQLite database, real encryption, real
//! `custom_id` strings. The only thing missing is the websocket to Discord.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use project_host_api_types::ProjectType;
use project_host_core::integration;
use project_host_database::discord::{self as storage, NewChannels, NewGuildLink};
use project_host_database::projects::{self, NewPort, NewProcess, NewProject, RuntimeSpec};
use project_host_database::Database;
use project_host_discord::events::{format_event, format_log_batch, Redactor};
use project_host_discord::permissions::{Action, Actor, Denied};
use project_host_discord::{Control, EventKind, Permission, Snowflake};
use project_host_security::{EncryptionKey, Secret};

const OWNER: &str = "111111111111111111";
const OPERATOR_ROLE: &str = "222222222222222222";
const VIEWER_ROLE: &str = "333333333333333333";
const GUILD: &str = "444444444444444444";
const NOSY_USER: &str = "555555555555555555";

async fn db() -> Database {
    Database::open_in_memory().await.expect("open")
}

fn id(text: &str) -> Snowflake {
    text.parse().expect("valid snowflake")
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
            runtime: RuntimeSpec {
                runtime: "NODEJS".to_string(),
                runtime_version: "22".to_string(),
                package_manager: "PNPM".to_string(),
                entry_file: Some("index.js".to_string()),
                publish_dir: None,
                template_id: "nodejs".to_string(),
            },
            processes: vec![NewProcess::simple("main", 0, "node index.js")],
            ports: vec![NewPort {
                port: 3000,
                host_port: Some(21_000),
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

/// A server linked with a viewer role, an operator role and one blocked user.
async fn a_configured_guild(database: &Database) -> String {
    let guild = storage::link_guild(
        database,
        &NewGuildLink {
            guild_id: GUILD.to_string(),
            guild_name: "My Server".to_string(),
            linked_by_user_id: OWNER.to_string(),
            allow_guild_owner: true,
            bot_row_id: a_bot(database).await,
        },
    )
    .await
    .expect("link");

    storage::upsert_grant(database, &guild, "role", VIEWER_ROLE, "view")
        .await
        .expect("viewer grant");
    storage::upsert_grant(database, &guild, "role", OPERATOR_ROLE, "operate")
        .await
        .expect("operator grant");
    storage::block_user(database, &guild, NOSY_USER, Some("kept stopping things"))
        .await
        .expect("block");

    guild
}

/// A bot to hang linked servers off, since 0006 gives every server an owner.
async fn a_bot(database: &Database) -> String {
    storage::add_bot(
        database,
        &storage::BotCredentials {
            label: "My Bot".to_string(),
            application_id: "999999999999999999".to_string(),
            token_cipher: vec![7u8; 64],
            token_nonce: vec![3u8; 24],
        },
    )
    .await
    .expect("add bot")
}

// ------------------------------------------------------------- the bot token

#[tokio::test]
async fn the_bot_token_survives_a_round_trip_through_encryption_and_storage() {
    let database = db().await;
    let key = EncryptionKey::generate();
    let token = "MTIzNDU2Nzg5MDEyMzQ1Njc4.GaBcDe.FgHiJkLmNoPqRsTuVwXyZ";

    let id = integration::add_bot(
        &database,
        &key,
        "My Bot",
        "999999999999999999",
        &Secret::new(token.to_string()),
    )
    .await
    .expect("save");

    let (application_id, loaded) = integration::load_bot_token(&database, &key, &id)
        .await
        .expect("load")
        .expect("present");

    assert_eq!(application_id, "999999999999999999");
    assert_eq!(loaded.expose(), token);
}

#[tokio::test]
async fn the_stored_bot_token_is_not_readable_without_the_key() {
    // The claim being tested is that the bytes on disk are ciphertext, not that
    // some code path chooses not to show them.
    let database = db().await;
    let key = EncryptionKey::generate();
    let token = "MTIzNDU2Nzg5MDEyMzQ1Njc4.GaBcDe.FgHiJkLmNoPqRsTuVwXyZ";

    let id = integration::add_bot(
        &database,
        &key,
        "My Bot",
        "999999999999999999",
        &Secret::new(token.to_string()),
    )
    .await
    .expect("save");

    let stored = storage::find_bot(&database, &id)
        .await
        .expect("load")
        .expect("present");
    let raw = String::from_utf8_lossy(&stored.token_cipher);
    assert!(
        !raw.contains(token),
        "the token is sitting there in the clear"
    );
    assert!(
        !raw.contains("MTIzNDU2"),
        "even a prefix of it should not survive"
    );
}

#[tokio::test]
async fn a_different_key_cannot_decrypt_the_token() {
    let database = db().await;
    let id = integration::add_bot(
        &database,
        &EncryptionKey::generate(),
        "My Bot",
        "999999999999999999",
        &Secret::new("a-real-looking-token-value".to_string()),
    )
    .await
    .expect("save");

    let error = integration::load_bot_token(&database, &EncryptionKey::generate(), &id)
        .await
        .expect_err("a different key must not work");
    assert!(matches!(error, integration::IntegrationError::Decrypt));
}

#[tokio::test]
async fn an_unconfigured_integration_is_not_an_error() {
    // Discord is optional. "No bot" must be a normal answer.
    let database = db().await;
    let loaded = integration::load_bot_token(&database, &EncryptionKey::generate(), "bot_missing")
        .await
        .expect("no bot is fine");
    assert!(loaded.is_none());
}

// --------------------------------------------------------- stored → policy

#[tokio::test]
async fn stored_grants_become_a_working_access_policy() {
    let database = db().await;
    a_configured_guild(&database).await;

    let policy = integration::access_policy_for(&database, GUILD)
        .await
        .expect("assemble")
        .expect("linked");

    let viewer = Actor::new(id("600000000000000000"), [id(VIEWER_ROLE)]);
    let operator = Actor::new(id("700000000000000000"), [id(OPERATOR_ROLE)]);

    assert_eq!(policy.permission_for(&viewer), Some(Permission::View));
    assert_eq!(policy.permission_for(&operator), Some(Permission::Operate));

    // The whole point of the model: reading is not controlling.
    assert!(policy.authorise(&viewer, Action::Status).is_ok());
    assert!(matches!(
        policy.authorise(&viewer, Action::Restart),
        Err(Denied::Insufficient { .. })
    ));
    assert!(policy.authorise(&operator, Action::Restart).is_ok());
}

#[tokio::test]
async fn a_stranger_in_the_server_can_do_nothing() {
    let database = db().await;
    a_configured_guild(&database).await;

    let policy = integration::access_policy_for(&database, GUILD)
        .await
        .expect("assemble")
        .expect("linked");

    let stranger = Actor::new(id("800000000000000000"), []);
    assert_eq!(policy.permission_for(&stranger), None);
    assert_eq!(
        policy.authorise(&stranger, Action::Status),
        Err(Denied::NotGranted)
    );
}

#[tokio::test]
async fn a_block_stored_in_the_database_is_enforced() {
    let database = db().await;
    a_configured_guild(&database).await;

    let policy = integration::access_policy_for(&database, GUILD)
        .await
        .expect("assemble")
        .expect("linked");

    // Blocked even though they hold the operator role.
    let blocked = Actor::new(id(NOSY_USER), [id(OPERATOR_ROLE)]);
    assert_eq!(
        policy.authorise(&blocked, Action::Stop),
        Err(Denied::Blocked)
    );
}

#[tokio::test]
async fn the_person_who_linked_the_server_keeps_administrative_access() {
    let database = db().await;
    a_configured_guild(&database).await;

    let policy = integration::access_policy_for(&database, GUILD)
        .await
        .expect("assemble")
        .expect("linked");

    let owner = Actor::new(id(OWNER), []);
    assert!(policy.authorise(&owner, Action::Unlink).is_ok());
}

#[tokio::test]
async fn an_unlinked_server_has_no_policy_at_all() {
    let database = db().await;
    assert!(integration::access_policy_for(&database, GUILD)
        .await
        .expect("query")
        .is_none());
}

#[tokio::test]
async fn a_stored_permission_level_this_build_does_not_know_is_refused_loudly() {
    // Skipping the row would silently change someone's permissions. Refusing
    // stops the interaction instead, which is the safe direction.
    let database = db().await;
    let guild = a_configured_guild(&database).await;

    sqlx::query(
        "INSERT INTO discord_grants (id, guild_row_id, subject_kind, subject_id, level, created_at)
         VALUES ('grt_x', ?, 'role', '900000000000000000', 'operate', '2026-01-01T00:00:00Z')",
    )
    .bind(&guild)
    .execute(database.pool())
    .await
    .expect("insert");

    // Corrupt it behind the CHECK, the way a future schema change might.
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(database.pool())
        .await
        .expect("pragma");
    sqlx::query("UPDATE discord_grants SET level = 'superuser' WHERE id = 'grt_x'")
        .execute(database.pool())
        .await
        .expect("corrupt");

    let error = integration::access_policy_for(&database, GUILD)
        .await
        .expect_err("must not guess");
    assert!(error.to_string().contains("superuser"), "got {error}");
}

// ------------------------------------------------------- a full button press

#[tokio::test]
async fn a_button_press_travels_from_panel_to_authorised_action() {
    // The complete path a click takes, in order: the panel is built, Discord
    // hands the id back, it is decoded, and only then is permission decided.
    let database = db().await;
    a_configured_guild(&database).await;
    let project = a_project(&database, "my-bot").await;
    let project_id = project.parse().expect("a valid project id");

    // 1. The panel is built and posted.
    let encoded = Control::new(Action::Restart, project_id)
        .expect("buildable")
        .encode()
        .expect("fits Discord's limit");

    // 2. Months later, Discord hands that string back on an interaction.
    let decoded = Control::decode(&encoded).expect("our own id must decode");
    assert_eq!(decoded.action, Action::Restart);
    assert_eq!(decoded.project.as_str(), project);

    // 3. Permission is decided now, with the roles on this interaction.
    let policy = integration::access_policy_for(&database, GUILD)
        .await
        .expect("assemble")
        .expect("linked");

    let operator = Actor::new(id("700000000000000000"), [id(OPERATOR_ROLE)]);
    assert!(policy.authorise(&operator, decoded.action).is_ok());

    // The same button, pressed by someone who has since lost the role.
    let demoted = Actor::new(id("700000000000000000"), [id(VIEWER_ROLE)]);
    assert!(
        policy.authorise(&demoted, decoded.action).is_err(),
        "the old panel must not carry old permissions"
    );
}

#[tokio::test]
async fn a_crafted_custom_id_cannot_reach_a_project() {
    // The attack: an interaction naming a project the panel never offered.
    for hostile in [
        "ph1:stop:prj_../../etc/passwd",
        "ph1:stop:",
        "ph1:unlink:prj_zzzz",
        "ph1:stop:prj_0193000000007000800000000000abcd:extra",
    ] {
        assert!(
            Control::decode(hostile).is_err(),
            "{hostile:?} should not decode into an action"
        );
    }
}

// --------------------------------------------------- stored → notifications

#[tokio::test]
async fn stored_settings_become_working_notification_settings() {
    let database = db().await;
    let guild = a_configured_guild(&database).await;
    let project = a_project(&database, "my-bot").await;

    let events: Vec<String> = EventKind::sensible_defaults()
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect();

    storage::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "1000".to_string(),
            control_channel_id: "1001".to_string(),
            logs_channel_name: "my-bot-logs".to_string(),
            control_channel_name: "my-bot-control".to_string(),
        },
        &events,
    )
    .await
    .expect("record");

    storage::update_notification_settings(
        &database,
        &project,
        None,
        Some(Some("1002")),
        Some(5_000),
    )
    .await
    .expect("settings");

    let settings = integration::notification_settings_for(&database, &project)
        .await
        .expect("assemble")
        .expect("linked");

    assert!(settings.enabled);
    assert_eq!(settings.batch_window_ms, 5_000);
    assert_eq!(settings.mention_role_on_failure, Some(id("1002")));
    assert!(settings.should_send(EventKind::Crashed));
    assert!(
        !settings.should_send(EventKind::LogOutput),
        "log forwarding stays off unless asked for"
    );
}

#[tokio::test]
async fn an_unlinked_project_has_no_notification_settings() {
    let database = db().await;
    let project = a_project(&database, "my-bot").await;
    assert!(integration::notification_settings_for(&database, &project)
        .await
        .expect("query")
        .is_none());
}

#[tokio::test]
async fn a_crash_notification_is_built_from_stored_settings_and_is_safe_to_send() {
    // The end of the whole chain: settings out of the database, a hostile log
    // line and a real secret in, a message that cannot ping or leak out.
    let database = db().await;
    let guild = a_configured_guild(&database).await;
    let project = a_project(&database, "my-bot").await;

    storage::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "1000".to_string(),
            control_channel_id: "1001".to_string(),
            logs_channel_name: "my-bot-logs".to_string(),
            control_channel_name: "my-bot-control".to_string(),
        },
        &[EventKind::Crashed.as_str().to_string()],
    )
    .await
    .expect("record");

    storage::update_notification_settings(&database, &project, None, Some(Some("1002")), None)
        .await
        .expect("mention role");

    let settings = integration::notification_settings_for(&database, &project)
        .await
        .expect("assemble")
        .expect("linked");

    let token = "MTIzNDU2Nzg5MDEyMzQ1Njc4.GaBcDe.FgHiJkLmNoPqRsTuVwXyZ";
    let redactor = Redactor::new([token.to_string()]);

    assert!(settings.should_send(EventKind::Crashed));
    let message = format_event(
        EventKind::Crashed,
        "my-bot",
        &format!("Error: login failed for token {token}\n@everyone look at this"),
        &settings,
        &redactor,
    );

    assert!(!message.content.contains(token), "the token leaked");
    assert!(!message.content.contains("@everyone"), "the message pings");
    assert_eq!(
        message.mention_role,
        Some(id("1002")),
        "a crash should ping the configured role"
    );
    assert!(message.content.chars().count() <= 2000);
}

#[tokio::test]
async fn a_muted_project_produces_nothing_even_for_a_crash() {
    let database = db().await;
    let guild = a_configured_guild(&database).await;
    let project = a_project(&database, "my-bot").await;

    storage::record_channels(
        &database,
        &NewChannels {
            project_id: project.clone(),
            guild_row_id: guild,
            logs_channel_id: "1000".to_string(),
            control_channel_id: "1001".to_string(),
            logs_channel_name: "my-bot-logs".to_string(),
            control_channel_name: "my-bot-control".to_string(),
        },
        &[EventKind::Crashed.as_str().to_string()],
    )
    .await
    .expect("record");

    storage::update_notification_settings(&database, &project, Some(false), None, None)
        .await
        .expect("mute");

    let settings = integration::notification_settings_for(&database, &project)
        .await
        .expect("assemble")
        .expect("linked");

    for kind in EventKind::ALL {
        assert!(!settings.should_send(*kind), "{kind:?} escaped the mute");
    }
}

#[tokio::test]
async fn a_chatty_project_is_batched_into_messages_discord_will_accept() {
    let lines: Vec<String> = (0..500)
        .map(|index| format!("[{index}] request handled in {index}ms"))
        .collect();

    let messages = format_log_batch("my-bot", &lines, &Redactor::default());

    assert!(messages.len() > 1, "500 lines should not be one message");
    for message in &messages {
        assert!(
            message.content.chars().count() <= 2000,
            "Discord would reject a message of {} characters",
            message.content.chars().count()
        );
    }
}

// ------------------------------------------------------------- name defaults

#[tokio::test]
async fn channel_templates_fall_back_to_defaults_for_a_server_that_never_set_them() {
    let database = db().await;
    let guild = a_configured_guild(&database).await;

    let templates = integration::channel_templates_for(&database, &guild)
        .await
        .expect("templates");

    assert_eq!(templates.len(), 2);
    let rendered: Vec<String> = templates
        .iter()
        .map(|(kind, template)| template.render("my-bot", "My Bot", *kind))
        .collect();
    assert_eq!(rendered, vec!["my-bot-logs", "my-bot-control"]);
}

#[tokio::test]
async fn a_customised_template_is_used_and_a_broken_one_falls_back() {
    let database = db().await;
    let guild = a_configured_guild(&database).await;

    storage::set_channel_template(&database, &guild, "logs", "{name}-output")
        .await
        .expect("custom");
    // Stored before an update removed the placeholder it used.
    storage::set_channel_template(&database, &guild, "control", "{nonsense}")
        .await
        .expect("broken");

    let templates = integration::channel_templates_for(&database, &guild)
        .await
        .expect("templates");

    let rendered: Vec<String> = templates
        .iter()
        .map(|(kind, template)| template.render("my-bot", "My Bot", *kind))
        .collect();

    assert_eq!(rendered[0], "my-bot-output", "the custom template was used");
    assert_eq!(
        rendered[1], "my-bot-control",
        "a broken template should fall back, not block the project"
    );
}
