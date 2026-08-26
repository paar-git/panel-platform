//! Storage for a project's remote access token.
//!
//! The property worth testing is not "can a blob be written and read back" but
//! that no layer here ever holds a usable token: the repository takes ciphertext,
//! the schema refuses anything else, and the plaintext exists only either side of
//! the `security` crate.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use project_host_api_types::ids::ProjectId;
use project_host_api_types::ProjectType;
use project_host_database::projects::{self, NewPort, NewProcess, NewProject, RuntimeSpec};
use project_host_database::source_credentials::{
    forget_source_credential, has_source_credential, load_source_credential,
    save_source_credential, SourceCredentialRecord,
};
use project_host_database::Database;
use project_host_security::encryption::{associated_data, decrypt, encrypt, EncryptionKey};
use project_host_security::secret::Secret;

const TOKEN: &str = "ghp_a_real_looking_personal_access_token";

async fn db() -> Database {
    Database::open_in_memory().await.expect("open")
}

fn runtime() -> RuntimeSpec {
    RuntimeSpec {
        runtime: "NODEJS".to_string(),
        runtime_version: "22".to_string(),
        package_manager: "NPM".to_string(),
        entry_file: Some("index.js".to_string()),
        publish_dir: None,
        template_id: "nodejs".to_string(),
    }
}

/// A project cloned from a git remote, which is the case a credential belongs to.
async fn a_cloned_project(database: &Database, slug: &str) -> String {
    let record = projects::create_project(
        database,
        &NewProject {
            slug: slug.to_string(),
            display_name: "Some CLI".to_string(),
            description: "installed from GitHub".to_string(),
            project_type: ProjectType::Worker,
            icon: None,
            color: None,
            source_type: "GIT_CLONE".to_string(),
            directory: format!("/var/lib/project-host/projects/{slug}"),
            source_url: Some("https://github.com/owner/private-repo.git".to_string()),
            source_ref: Some("main".to_string()),
            source_commit: Some("0f5c1d0ab1c2d3e4f5061728394a5b6c7d8e9f01".to_string()),
            autostart: false,
            restart_policy: "UNLESS_STOPPED".to_string(),
            network_mode: "INTERNET".to_string(),
            memory_limit_mb: 512,
            cpu_limit_cores: 1.0,
            storage_limit_mb: 2048,
            process_limit: 128,
            runtime: runtime(),
            processes: vec![NewProcess::simple("main", 0, "node index.js")],
            ports: Vec::<NewPort>::new(),
        },
    )
    .await
    .expect("the project should be created");
    record.id
}

/// Encrypt a token the way the application does.
fn encrypted(project_id: &str) -> SourceCredentialRecord {
    let key = EncryptionKey::generate();
    let ciphertext = encrypt(
        &key,
        &Secret::new(TOKEN.to_string()),
        &associated_data(project_id, "SOURCE_CREDENTIAL"),
    )
    .expect("encrypt");

    SourceCredentialRecord {
        project_id: project_id.to_string(),
        ciphertext: ciphertext.bytes,
        nonce: ciphertext.nonce,
    }
}

#[tokio::test]
async fn a_credential_round_trips_as_ciphertext() {
    let database = db().await;
    let project = a_cloned_project(&database, "proj-credential-a").await;

    let record = encrypted(&project);
    save_source_credential(&database, &record).await.unwrap();

    let loaded = load_source_credential(&database, &project)
        .await
        .unwrap()
        .expect("a credential should be stored");
    assert_eq!(loaded, record);
}

#[tokio::test]
async fn the_stored_bytes_do_not_contain_the_token() {
    // The blunt check. If this ever fails, the encryption step was skipped
    // somewhere between the caller and the column.
    let database = db().await;
    let project = a_cloned_project(&database, "proj-credential-b").await;
    save_source_credential(&database, &encrypted(&project))
        .await
        .unwrap();

    let stored: Vec<u8> = sqlx::query_scalar(
        "SELECT ciphertext FROM project_source_credentials WHERE project_id = ?",
    )
    .bind(&project)
    .fetch_one(database.pool())
    .await
    .unwrap();

    let needle = TOKEN.as_bytes();
    assert!(
        !stored.windows(needle.len()).any(|slice| slice == needle),
        "the token is in the database in the clear"
    );
}

#[tokio::test]
async fn a_stored_credential_decrypts_back_to_the_token() {
    // Storage takes ciphertext and returns ciphertext; the key never crosses the
    // boundary, so the decryption happens here, in the caller's role.
    let database = db().await;
    let project = a_cloned_project(&database, "proj-credential-c").await;

    let key = EncryptionKey::generate();
    let aad = associated_data(&project, "SOURCE_CREDENTIAL");
    let ciphertext = encrypt(&key, &Secret::new(TOKEN.to_string()), &aad).expect("encrypt");

    save_source_credential(
        &database,
        &SourceCredentialRecord {
            project_id: project.clone(),
            ciphertext: ciphertext.bytes.clone(),
            nonce: ciphertext.nonce.clone(),
        },
    )
    .await
    .unwrap();

    let loaded = load_source_credential(&database, &project)
        .await
        .unwrap()
        .expect("stored");

    let recovered = decrypt(
        &key,
        &project_host_security::encryption::Ciphertext {
            bytes: loaded.ciphertext,
            nonce: loaded.nonce,
        },
        &aad,
    )
    .expect("decrypt");

    assert_eq!(recovered.expose(), TOKEN);
}

#[tokio::test]
async fn a_credential_bound_to_one_project_does_not_decrypt_for_another() {
    // The associated data is the project id, so a row copied to another project
    // is useless rather than a working credential in the wrong place.
    let database = db().await;
    let project = a_cloned_project(&database, "proj-credential-d").await;

    let key = EncryptionKey::generate();
    let ciphertext = encrypt(
        &key,
        &Secret::new(TOKEN.to_string()),
        &associated_data(&project, "SOURCE_CREDENTIAL"),
    )
    .expect("encrypt");

    let wrong = decrypt(
        &key,
        &project_host_security::encryption::Ciphertext {
            bytes: ciphertext.bytes,
            nonce: ciphertext.nonce,
        },
        &associated_data("prj_someone_else", "SOURCE_CREDENTIAL"),
    );
    assert!(
        wrong.is_err(),
        "the ciphertext was not bound to its project"
    );
}

#[tokio::test]
async fn saving_twice_replaces_rather_than_failing() {
    // What happens when a user's token expires and they enter a new one.
    let database = db().await;
    let project = a_cloned_project(&database, "proj-credential-e").await;

    save_source_credential(&database, &encrypted(&project))
        .await
        .unwrap();
    let second = encrypted(&project);
    save_source_credential(&database, &second).await.unwrap();

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_source_credentials")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(rows, 1);

    let loaded = load_source_credential(&database, &project)
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(loaded, second, "the replacement did not take effect");
}

#[tokio::test]
async fn presence_can_be_asked_without_loading_the_credential() {
    let database = db().await;
    let project = a_cloned_project(&database, "proj-credential-f").await;

    assert!(!has_source_credential(&database, &project).await.unwrap());
    save_source_credential(&database, &encrypted(&project))
        .await
        .unwrap();
    assert!(has_source_credential(&database, &project).await.unwrap());
}

#[tokio::test]
async fn forgetting_a_credential_removes_the_row_entirely() {
    let database = db().await;
    let project = a_cloned_project(&database, "proj-credential-g").await;
    save_source_credential(&database, &encrypted(&project))
        .await
        .unwrap();

    forget_source_credential(&database, &project).await.unwrap();

    assert!(load_source_credential(&database, &project)
        .await
        .unwrap()
        .is_none());
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_source_credentials")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0, "a blanked row was left behind");
}

#[tokio::test]
async fn a_credential_for_a_project_that_does_not_exist_is_refused() {
    // The foreign key, doing its job: a token with no project is a token nothing
    // will ever delete.
    let database = db().await;
    let orphan = ProjectId::generate().to_string();
    let result = save_source_credential(&database, &encrypted(&orphan)).await;
    assert!(result.is_err(), "an orphaned credential was accepted");
}
