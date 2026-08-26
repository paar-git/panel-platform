//! A project's environment, as the process it starts will see it.
//!
//! This is the composition point for two things that deliberately know nothing
//! about each other: `database`'s `environment` module, which stores a value as
//! either plaintext or ciphertext and never decides which, and `security`,
//! which can decrypt but has no idea what a project is.
//!
//! # Why this exists at all
//!
//! Host projects were started with an empty environment. Everything a user had
//! configured on the project's settings screen — a database URL, an API key, a
//! bot token — was written to the database, displayed back in the interface,
//! and then not passed to the process. A Discord bot reading
//! `process.env.DISCORD_TOKEN` got `undefined` and failed to log in, and
//! nothing in the interface suggested why, because the variable was plainly
//! there on the screen.
//!
//! # What a process does and does not inherit
//!
//! The child inherits this process's environment and has the project's
//! variables laid over the top. That is the behaviour a person expects from
//! running the command themselves in a terminal: `PATH` and `HOME` are there,
//! and what they configured wins over what they did not.
//!
//! Secrets are decrypted here, held only as long as it takes to build the
//! command, and never logged. [`Resolved::redacted_keys`] exists so a caller
//! can say *which* variables were applied without being able to say what is in
//! them.

use std::collections::BTreeMap;

use project_host_database::environment::{self, StoredValue};
use project_host_database::Database;
use project_host_security::encryption::{associated_data, decrypt, Ciphertext, EncryptionKey};

/// Variables that cannot come from a project's own configuration.
///
/// `PORT` is set from the allocated port after this map is applied, and a
/// project that overrode it would bind a port the application does not think it
/// owns — so the conflict check, the health probe and the address shown in the
/// interface would all be about a different port from the one in use.
///
/// The rest are the operating system's, and a project that overwrote its own
/// `PATH` would usually be doing it by accident.
const RESERVED: &[&str] = &["PORT", "PATH", "SYSTEMROOT", "WINDIR"];

/// A project's variables, ready to hand to a process.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// Every variable, secrets included and in the clear. Consumed by the
    /// spawn and dropped immediately after.
    pub values: BTreeMap<String, String>,
    /// The names of the variables that were secret. For the log line, so it can
    /// say what was applied without saying what any of it was.
    pub secret_keys: Vec<String>,
    /// Variables that were configured and deliberately not applied, because
    /// the runtime owns them. Reported so the interface can explain the
    /// omission rather than the user finding it by debugging.
    pub ignored_keys: Vec<String>,
    /// Secrets that could not be decrypted, by name.
    ///
    /// This happens when the master key has changed — a restored backup, a
    /// keychain entry that was deleted — and it must not be silent. A bot
    /// started without its token fails at Discord with an authentication error
    /// that says nothing about the real cause.
    pub undecryptable_keys: Vec<String>,
}

impl Resolved {
    /// Every key that was applied, with secret values replaced by a marker.
    ///
    /// The only form of this map that is safe to log or to show.
    pub fn redacted_keys(&self) -> Vec<(String, String)> {
        self.values
            .keys()
            .map(|key| {
                let value = if self.secret_keys.contains(key) {
                    "<secret>".to_string()
                } else {
                    self.values.get(key).cloned().unwrap_or_default()
                };
                (key.clone(), value)
            })
            .collect()
    }
}

/// Read a project's environment and decrypt what needs decrypting.
///
/// A missing key is not an error: an installation whose secure storage could
/// not be opened still runs projects, and the plaintext variables are still
/// applied. The secrets are reported in `undecryptable_keys` instead, which is
/// the honest answer and the one a caller can pass on.
pub async fn resolve(
    db: &Database,
    project_id: &str,
    key: Option<&EncryptionKey>,
) -> Result<Resolved, project_host_database::DatabaseError> {
    let stored = environment::list_variables(db, project_id).await?;
    let mut resolved = Resolved::default();

    for variable in stored {
        if RESERVED.contains(&variable.key.as_str()) {
            resolved.ignored_keys.push(variable.key);
            continue;
        }

        match variable.value {
            StoredValue::Plain(value) => {
                resolved.values.insert(variable.key, value);
            }
            StoredValue::Secret { cipher, nonce } => {
                let Some(key) = key else {
                    resolved.undecryptable_keys.push(variable.key);
                    continue;
                };
                let ciphertext = Ciphertext {
                    bytes: cipher,
                    nonce,
                };
                // The associated data binds the ciphertext to this project and
                // this key, so a row moved between projects fails to decrypt
                // rather than silently handing another project's secret over.
                match decrypt(
                    key,
                    &ciphertext,
                    &associated_data(project_id, &variable.key),
                ) {
                    Ok(plaintext) => {
                        resolved
                            .values
                            .insert(variable.key.clone(), plaintext.expose().to_string());
                        resolved.secret_keys.push(variable.key);
                    }
                    Err(_) => resolved.undecryptable_keys.push(variable.key),
                }
            }
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_host_security::Secret;

    async fn db() -> Database {
        Database::open_in_memory().await.expect("in-memory")
    }

    async fn a_project(database: &Database, slug: &str) -> String {
        use project_host_api_types::ProjectType;
        use project_host_database::projects::{self, NewProcess, NewProject, RuntimeSpec};

        projects::create_project(
            database,
            &NewProject {
                slug: slug.to_string(),
                display_name: slug.to_string(),
                description: String::new(),
                project_type: ProjectType::Service,
                icon: None,
                color: None,
                source_type: "EMPTY".to_string(),
                directory: format!("projects/{slug}"),
                source_url: None,
                source_ref: None,
                source_commit: None,
                autostart: false,
                restart_policy: "NO".to_string(),
                network_mode: "INTERNET".to_string(),
                memory_limit_mb: 512,
                cpu_limit_cores: 1.0,
                storage_limit_mb: 1024,
                process_limit: 128,
                runtime: RuntimeSpec {
                    runtime: "NODEJS".to_string(),
                    runtime_version: "latest".to_string(),
                    package_manager: "NPM".to_string(),
                    entry_file: None,
                    publish_dir: None,
                    template_id: "node".to_string(),
                },
                processes: vec![NewProcess::simple("main", 0, "node index.js")],
                ports: Vec::new(),
            },
        )
        .await
        .expect("create")
        .id
    }

    async fn store_secret(
        database: &Database,
        key: &EncryptionKey,
        project_id: &str,
        name: &str,
        value: &str,
    ) {
        let ciphertext = project_host_security::encryption::encrypt(
            key,
            &Secret::new(value.to_string()),
            &associated_data(project_id, name),
        )
        .expect("encrypt");

        environment::upsert_variable(
            database,
            project_id,
            name,
            &StoredValue::Secret {
                cipher: ciphertext.bytes,
                nonce: ciphertext.nonce,
            },
        )
        .await
        .expect("upsert");
    }

    /// The bug this module exists for: a configured variable has to reach the
    /// process, or a bot reading `process.env.DISCORD_TOKEN` gets nothing.
    #[tokio::test]
    async fn a_plain_variable_reaches_the_process() {
        let database = db().await;
        let project = a_project(&database, "env-plain").await;

        environment::upsert_variable(
            &database,
            &project,
            "DATABASE_URL",
            &StoredValue::Plain("postgres://localhost/app".to_string()),
        )
        .await
        .expect("upsert");

        let resolved = resolve(&database, &project, None).await.expect("resolve");
        assert_eq!(
            resolved.values.get("DATABASE_URL").map(String::as_str),
            Some("postgres://localhost/app")
        );
        assert!(resolved.secret_keys.is_empty());
    }

    #[tokio::test]
    async fn a_secret_is_decrypted_for_the_process_and_named_but_not_shown() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let project = a_project(&database, "env-secret").await;

        store_secret(&database, &key, &project, "DISCORD_TOKEN", "a-real-token").await;

        let resolved = resolve(&database, &project, Some(&key))
            .await
            .expect("resolve");
        assert_eq!(
            resolved.values.get("DISCORD_TOKEN").map(String::as_str),
            Some("a-real-token"),
            "the process has to receive the real value"
        );
        assert_eq!(resolved.secret_keys, vec!["DISCORD_TOKEN".to_string()]);

        // …and the only form anything may log carries the name and not the value.
        let redacted = resolved.redacted_keys();
        assert_eq!(
            redacted,
            vec![("DISCORD_TOKEN".to_string(), "<secret>".to_string())]
        );
        assert!(
            !format!("{redacted:?}").contains("a-real-token"),
            "the token leaked into the loggable form"
        );
    }

    /// A key that cannot decrypt a stored secret must be reported, not skipped.
    /// A bot started without its token fails at Discord with an authentication
    /// error that says nothing about the real cause.
    #[tokio::test]
    async fn a_secret_that_cannot_be_decrypted_is_reported_by_name() {
        let database = db().await;
        let original = EncryptionKey::generate();
        let project = a_project(&database, "env-rekeyed").await;

        store_secret(&database, &original, &project, "API_KEY", "value").await;

        let resolved = resolve(&database, &project, Some(&EncryptionKey::generate()))
            .await
            .expect("resolve");

        assert_eq!(resolved.undecryptable_keys, vec!["API_KEY".to_string()]);
        assert!(
            !resolved.values.contains_key("API_KEY"),
            "an undecryptable secret must not reach the process as anything"
        );
    }

    /// Without a key the plaintext half still works. An installation whose
    /// keychain could not be opened runs projects; it just cannot hand over
    /// secrets.
    #[tokio::test]
    async fn without_a_key_the_plain_variables_are_still_applied() {
        let database = db().await;
        let key = EncryptionKey::generate();
        let project = a_project(&database, "env-nokey").await;

        environment::upsert_variable(
            &database,
            &project,
            "LOG_LEVEL",
            &StoredValue::Plain("debug".to_string()),
        )
        .await
        .expect("upsert");
        store_secret(&database, &key, &project, "API_KEY", "value").await;

        let resolved = resolve(&database, &project, None).await.expect("resolve");
        assert_eq!(
            resolved.values.get("LOG_LEVEL").map(String::as_str),
            Some("debug")
        );
        assert_eq!(resolved.undecryptable_keys, vec!["API_KEY".to_string()]);
    }

    /// `PORT` is the application's to set. A project that overrode it would
    /// bind a port nothing else in the application knows about, so the conflict
    /// check, the health probe and the address on screen would all be wrong.
    #[tokio::test]
    async fn a_variable_the_runtime_owns_is_ignored_and_said_so() {
        let database = db().await;
        let project = a_project(&database, "env-reserved").await;

        environment::upsert_variable(
            &database,
            &project,
            "PORT",
            &StoredValue::Plain("3000".to_string()),
        )
        .await
        .expect("upsert");

        let resolved = resolve(&database, &project, None).await.expect("resolve");
        assert!(!resolved.values.contains_key("PORT"));
        assert_eq!(resolved.ignored_keys, vec!["PORT".to_string()]);
    }

    #[tokio::test]
    async fn a_project_with_no_variables_resolves_to_nothing() {
        let database = db().await;
        let project = a_project(&database, "env-empty").await;

        let resolved = resolve(&database, &project, None).await.expect("resolve");
        assert!(resolved.values.is_empty());
        assert!(resolved.ignored_keys.is_empty());
        assert!(resolved.undecryptable_keys.is_empty());
    }
}
