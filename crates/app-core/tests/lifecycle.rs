//! The real startup and shutdown path, against a real database on disk.
//!
//! Not mocked and not in memory: this creates the directory layout, runs the
//! migrations, performs crash recovery, probes Docker and closes the pool, in
//! the same order and by the same code the application will use. It is the
//! closest thing to "does it work" that exists while there is no window to
//! open, and it covers the paths that are hardest to exercise by hand — an
//! unclean shutdown in particular.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use project_host_core::{resolve_paths, AppConfig, Mode, Runtime};
use project_host_database::SUPPORTED_SCHEMA_VERSION;
use project_host_docker_manager::DockerProbe;
use project_host_docker_manager::DockerStatus;
use project_host_platform::PathProvider;

fn config_rooted_at(root: &std::path::Path) -> AppConfig {
    AppConfig {
        mode: Mode::Development,
        log_json: false,
        data_dir: Some(root.to_path_buf()),
        ..AppConfig::default()
    }
}

#[tokio::test]
async fn a_cold_start_creates_everything_it_needs() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");

    let runtime = Runtime::start(config, paths.clone())
        .await
        .expect("start from nothing");

    assert!(
        paths.database_path().exists(),
        "the database file should have been created"
    );
    assert_eq!(
        runtime.state().inner().schema_version,
        SUPPORTED_SCHEMA_VERSION,
        "migrations should have run to the version this build expects"
    );
    assert!(
        !runtime.state().inner().instance_id.is_empty(),
        "the run should be identifiable in the logs"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn docker_is_probed_at_startup_and_its_absence_is_not_fatal() {
    // The rule from the design: an application that refuses to start without
    // Docker cannot tell the user why Docker is missing. The probe is deferred
    // until after the window would open, so this asserts both that start did
    // not wait for it and that the later pass produced a description.
    let directory = tempfile::tempdir().expect("temp dir");
    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");

    let runtime = Runtime::start(config, paths).await.expect("start");
    assert!(
        runtime.state().docker_status().await.is_unchecked(),
        "Docker must not sit on the path that opens the window"
    );

    runtime.complete_optional_startup().await;
    let status = runtime.state().docker_status().await;

    assert!(
        !status.summary().is_empty(),
        "the probe should describe what it found either way"
    );
    assert!(
        !status.is_unchecked(),
        "optional startup should have asked Docker by now"
    );
    if !status.available {
        assert!(
            status.install_hint.is_some() || status.error.is_some(),
            "an absent daemon must come with a reason or a hint, not silence"
        );
    }

    runtime.shutdown().await;
}

#[tokio::test]
async fn a_clean_shutdown_is_recorded_and_the_next_start_is_uneventful() {
    let directory = tempfile::tempdir().expect("temp dir");

    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");
    Runtime::start(config, paths.clone())
        .await
        .expect("first start")
        .shutdown()
        .await;

    let config = config_rooted_at(directory.path());
    let runtime = Runtime::start(config, paths).await.expect("second start");

    assert!(
        runtime.recovery.is_uneventful(),
        "a start after a clean stop should have nothing to repair, got {:?}",
        runtime.recovery
    );
    assert!(runtime.recovery.integrity_ok);

    runtime.shutdown().await;
}

#[tokio::test]
async fn a_start_after_an_unclean_stop_runs_recovery() {
    // Dropping the runtime without calling `shutdown` is exactly what a power
    // cut looks like to the next start: the clean-shutdown flag is still 0.
    let directory = tempfile::tempdir().expect("temp dir");

    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");
    let runtime = Runtime::start(config, paths.clone())
        .await
        .expect("first start");
    drop(runtime);

    let config = config_rooted_at(directory.path());
    let runtime = Runtime::start(config, paths)
        .await
        .expect("recovering start");

    assert!(
        runtime.recovery.integrity_ok,
        "the database should still be intact after an unclean stop"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn the_database_survives_being_opened_repeatedly() {
    // Guards against a WAL or locking mistake that only appears on the third or
    // fourth run, which is the kind of bug a single-shot test never sees.
    let directory = tempfile::tempdir().expect("temp dir");

    for attempt in 0..4 {
        let config = config_rooted_at(directory.path());
        let paths = resolve_paths(&config).expect("paths");
        let runtime = Runtime::start(config, paths)
            .await
            .unwrap_or_else(|error| panic!("start {attempt} failed: {error}"));
        assert!(
            runtime.recovery.integrity_ok,
            "integrity lost on run {attempt}"
        );
        runtime.shutdown().await;
    }
}

#[tokio::test]
async fn two_instances_get_different_identities() {
    let directory = tempfile::tempdir().expect("temp dir");

    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");
    let first = Runtime::start(config, paths.clone()).await.expect("start");
    let first_id = first.state().inner().instance_id.clone();
    first.shutdown().await;

    let config = config_rooted_at(directory.path());
    let second = Runtime::start(config, paths).await.expect("start");
    let second_id = second.state().inner().instance_id.clone();
    second.shutdown().await;

    assert_ne!(
        first_id, second_id,
        "log lines from two runs must be distinguishable"
    );
}

#[tokio::test]
async fn a_refused_configuration_never_reaches_startup() {
    // Validation is meant to happen before anything is opened or created.
    let config = AppConfig {
        port_pool_start: 30_000,
        port_pool_end: 30_000,
        ..AppConfig::default()
    };
    assert!(config.validate().is_err(), "an empty port pool is unusable");
}

/// A Docker probe that never returns. Stands in for a named pipe that exists
/// while Docker Desktop is still booting after a reboot — the case that used
/// to make clicking the app do nothing.
#[derive(Debug)]
struct HungDocker;

#[async_trait::async_trait]
impl DockerProbe for HungDocker {
    async fn probe(&self) -> DockerStatus {
        std::future::pending().await
    }
}

#[tokio::test]
async fn start_does_not_wait_for_a_hung_docker_probe() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");

    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        Runtime::start_with_probe(config, paths, Arc::new(HungDocker)),
    )
    .await;

    let runtime = match result {
        Ok(Ok(runtime)) => runtime,
        Ok(Err(error)) => panic!("start failed: {error}"),
        Err(_) => panic!("startup waited for a Docker probe that never returned"),
    };

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "startup took {:?}",
        started.elapsed()
    );
    let status = runtime.state().docker_status().await;
    assert!(
        status.is_unchecked() || !status.available,
        "a hung probe must not be reported as a working daemon"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn startup_records_which_stages_ran_and_how_long_they_took() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");

    let runtime = Runtime::start(config, paths).await.expect("start");
    let diagnostics = runtime.startup_diagnostics();

    let names: Vec<&str> = diagnostics
        .stages
        .iter()
        .map(|stage| stage.name.as_str())
        .collect();
    assert!(
        names.contains(&"directories"),
        "missing directories stage, got {names:?}"
    );
    assert!(
        names.contains(&"database"),
        "missing database stage, got {names:?}"
    );
    assert!(
        names.contains(&"recovery"),
        "missing recovery stage, got {names:?}"
    );
    assert!(
        !names.contains(&"docker"),
        "docker was on the critical path, got {names:?}"
    );
    assert!(
        diagnostics.total_ms < 5_000,
        "critical startup took {}ms",
        diagnostics.total_ms
    );
    for stage in &diagnostics.stages {
        assert!(
            !stage.outcome.is_empty(),
            "stage {} has no outcome",
            stage.name
        );
    }

    runtime.shutdown().await;
}

#[tokio::test]
async fn optional_startup_does_not_run_a_machine_scan() {
    // A machine scan spawns PowerShell and `wsl`. Those open a visible console
    // on Windows, which is what "a PowerShell window appears when I click the
    // app" was. Docker is probed; the scan is not.
    let directory = tempfile::tempdir().expect("temp dir");
    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");
    let runtime = Runtime::start(config, paths).await.expect("start");
    runtime.complete_optional_startup().await;

    let diagnostics = runtime.startup_diagnostics();
    let names: Vec<&str> = diagnostics
        .stages
        .iter()
        .map(|stage| stage.name.as_str())
        .collect();
    assert!(
        !names.contains(&"machine_scan"),
        "the launch path still ran a PowerShell machine scan: {names:?}"
    );

    runtime.shutdown().await;
}

#[tokio::test]
async fn optional_startup_survives_a_hung_docker_probe() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = config_rooted_at(directory.path());
    let paths = resolve_paths(&config).expect("paths");
    let runtime = tokio::time::timeout(
        Duration::from_secs(2),
        Runtime::start_with_probe(config, paths, Arc::new(HungDocker)),
    )
    .await
    .expect("start waited for Docker")
    .expect("start");

    let result =
        tokio::time::timeout(Duration::from_secs(12), runtime.complete_optional_startup()).await;
    assert!(
        result.is_ok(),
        "optional startup did not give up on a hung Docker probe"
    );

    let status = runtime.state().docker_status().await;
    assert!(
        !status.available,
        "a probe that never answered must not look like a working daemon"
    );
    assert!(
        status.install_hint.is_some() || status.error.is_some(),
        "the user needs a reason, not a permanent 'checking' state"
    );

    runtime.shutdown().await;
}
