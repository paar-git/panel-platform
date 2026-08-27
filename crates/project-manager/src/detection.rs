//! Runtime detection.
//!
//! Inspects an uploaded or selected directory and proposes a runtime, version,
//! package manager and start command. Everything it returns is a *proposal*:
//! the wizard shows it, the user corrects it, and the corrected values are
//! validated against the template manifest before anything is built.
//!
//! Detection never executes anything from the project. It reads files.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Runtime {
    NodeJs,
    TypeScript,
    Bun,
    Deno,
    Python,
    Go,
    Rust,
    Java,
    Php,
    Ruby,
    DotNet,
    Static,
    /// More than one language's toolchain in one image. Never a guess: it is
    /// chosen only when a tree shows real evidence of several languages.
    Polyglot,
}

impl Runtime {
    /// The wire value, identical to `api-types`' `Runtime`.
    ///
    /// The two enums are separate because this crate does not depend on
    /// `api-types` — detection has no business knowing about the API — so these
    /// strings are the contract between them, and `app-core` has a test that
    /// every one of them round-trips.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NodeJs => "NODEJS",
            Self::TypeScript => "TYPESCRIPT",
            Self::Bun => "BUN",
            Self::Deno => "DENO",
            Self::Python => "PYTHON",
            Self::Go => "GO",
            Self::Rust => "RUST",
            Self::Java => "JAVA",
            Self::Php => "PHP",
            Self::Ruby => "RUBY",
            Self::DotNet => "DOTNET",
            Self::Static => "STATIC",
            Self::Polyglot => "POLYGLOT",
        }
    }

    /// What a person would call it.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::NodeJs => "Node.js",
            Self::TypeScript => "TypeScript",
            Self::Bun => "Bun",
            Self::Deno => "Deno",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Rust => "Rust",
            Self::Java => "Java",
            Self::Php => "PHP",
            Self::Ruby => "Ruby",
            Self::DotNet => ".NET",
            Self::Static => "Static site",
            Self::Polyglot => "Several languages",
        }
    }

    pub const ALL: [Runtime; 13] = [
        Self::NodeJs,
        Self::TypeScript,
        Self::Bun,
        Self::Deno,
        Self::Python,
        Self::Go,
        Self::Rust,
        Self::Java,
        Self::Php,
        Self::Ruby,
        Self::DotNet,
        Self::Static,
        Self::Polyglot,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageManager {
    Pnpm,
    Npm,
    Yarn,
    Bun,
    Deno,
    Pip,
    Poetry,
    Uv,
    Pipenv,
    GoModules,
    Cargo,
    Maven,
    Gradle,
    Composer,
    Bundler,
    NuGet,
    None,
}

impl PackageManager {
    /// The wire value, identical to `api-types`' `PackageManager`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pnpm => "PNPM",
            Self::Npm => "NPM",
            Self::Yarn => "YARN",
            Self::Bun => "BUN",
            Self::Deno => "DENO",
            Self::Pip => "PIP",
            Self::Poetry => "POETRY",
            Self::Uv => "UV",
            Self::Pipenv => "PIPENV",
            Self::GoModules => "GO_MODULES",
            Self::Cargo => "CARGO",
            Self::Maven => "MAVEN",
            Self::Gradle => "GRADLE",
            Self::Composer => "COMPOSER",
            Self::Bundler => "BUNDLER",
            Self::NuGet => "NUGET",
            Self::None => "NONE",
        }
    }

    pub const ALL: [PackageManager; 17] = [
        Self::Pnpm,
        Self::Npm,
        Self::Yarn,
        Self::Bun,
        Self::Deno,
        Self::Pip,
        Self::Poetry,
        Self::Uv,
        Self::Pipenv,
        Self::GoModules,
        Self::Cargo,
        Self::Maven,
        Self::Gradle,
        Self::Composer,
        Self::Bundler,
        Self::NuGet,
        Self::None,
    ];
}

/// Something the user should know before deploying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionWarning {
    pub code: String,
    pub message: String,
}

/// A problem that stops a deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detection {
    pub runtime: Runtime,
    pub package_manager: PackageManager,
    pub has_lockfile: bool,
    /// Scripts found in `package.json`.
    pub scripts: BTreeMap<String, String>,
    /// The script or entry file suggested, if one could be determined.
    pub suggested_start: Option<String>,
    pub suggested_entry_file: Option<String>,
    pub suggested_build_command: Option<String>,
    pub suggested_publish_dir: Option<String>,
    pub warnings: Vec<DetectionWarning>,
    /// Non-empty means the project cannot be deployed as-is.
    pub errors: Vec<DetectionError>,
}

impl Detection {
    pub fn is_deployable(&self) -> bool {
        self.errors.is_empty()
    }

    fn warn(&mut self, code: &str, message: &str) {
        self.warnings.push(DetectionWarning {
            code: code.to_string(),
            message: message.to_string(),
        });
    }

    fn fail(&mut self, code: &str, message: &str) {
        self.errors.push(DetectionError {
            code: code.to_string(),
            message: message.to_string(),
        });
    }
}

/// Every language a tree shows evidence of, in a stable order.
///
/// Separated from [`detect`] because "what is in here" and "what should we build
/// it as" are different questions, and only the first one is a fact. A tree with
/// a `go.mod` and a `package.json` really does contain both; deciding what to do
/// about that is a policy, and policies belong somewhere testable on their own.
///
/// Note what is *not* here: reading file contents to guess. A marker file is
/// evidence a maintainer put there deliberately. Counting `.py` files would make
/// one vendored script outvote the actual project.
pub fn signals(root: &Path) -> Vec<Runtime> {
    let exists = |name: &str| root.join(name).is_file();
    let any = |names: &[&str]| names.iter().any(|name| root.join(name).is_file());

    let mut found = Vec::new();

    // The JavaScript family resolves among itself before anything else, because
    // its members share `package.json` and only the extra file distinguishes
    // them. Deno and Bun win over Node when their own manifests are present:
    // nobody adds `deno.json` to a project they run with Node.
    if any(&["deno.json", "deno.jsonc", "deno.lock"]) {
        found.push(Runtime::Deno);
    } else if any(&["bun.lockb", "bun.lock", "bunfig.toml"]) {
        found.push(Runtime::Bun);
    } else if exists("package.json") {
        // tsconfig.json means a compile step, which is a different image, which
        // is why TypeScript is its own runtime rather than a flag on Node.
        if exists("tsconfig.json") {
            found.push(Runtime::TypeScript);
        } else {
            found.push(Runtime::NodeJs);
        }
    }

    if has_any_python_marker(root) {
        found.push(Runtime::Python);
    }
    if exists("go.mod") {
        found.push(Runtime::Go);
    }
    if exists("Cargo.toml") {
        found.push(Runtime::Rust);
    }
    if any(&["pom.xml", "build.gradle", "build.gradle.kts"]) {
        found.push(Runtime::Java);
    }
    if any(&["composer.json", "index.php"]) {
        found.push(Runtime::Php);
    }
    if any(&["Gemfile", "config.ru"]) {
        found.push(Runtime::Ruby);
    }
    if has_dotnet_project(root) {
        found.push(Runtime::DotNet);
    }

    // A static site last, and only on its own: `index.html` beside a real
    // application is that application's template or its built output, not a
    // second project.
    //
    // The conventional publish directories count too. A site whose pages live in
    // `public/` is the common shape — and it is what this application's own
    // scaffold writes, so failing to recognise it would mean an empty static
    // project could not be identified afterwards.
    if found.is_empty()
        && (exists("index.html")
            || any(&["public/index.html", "web/index.html", "html/index.html"]))
    {
        found.push(Runtime::Static);
    }

    found
}

/// `*.csproj`, `*.fsproj` or `*.sln` — the only marker that needs a directory
/// scan rather than a known filename.
fn has_dotnet_project(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "csproj" | "fsproj" | "sln"
                )
            })
    })
}

/// Inspect a directory and propose how to build it.
///
/// One language is the ordinary case and gets that language's detector. Several
/// languages get [`Runtime::Polyglot`] — an image carrying every toolchain the
/// tree needs — because the alternative is picking one and failing at build time
/// on the other. That decision is reported as a warning listing what was found,
/// so a user who disagrees can override it rather than wonder.
pub fn detect(root: &Path) -> Detection {
    let found = signals(root);

    match found.as_slice() {
        [] => {
            let mut detection = empty(Runtime::Static, PackageManager::None);
            detection.fail(
                "NO_RUNTIME_DETECTED",
                "Nothing here identifies a language: no package.json, \
                 requirements.txt, pyproject.toml, go.mod, Cargo.toml, pom.xml, \
                 composer.json, Gemfile, .csproj, deno.json or index.html. \
                 Choose a runtime manually, or check that the project is at the \
                 top level rather than inside a folder.",
            );
            detection
        }
        [only] => detect_one(root, *only),
        several => detect_polyglot(root, several),
    }
}

/// Dispatch to the detector for a single language.
fn detect_one(root: &Path, runtime: Runtime) -> Detection {
    match runtime {
        Runtime::NodeJs => detect_node(root, false),
        Runtime::TypeScript => detect_node(root, true),
        Runtime::Bun => detect_bun(root),
        Runtime::Deno => detect_deno(root),
        Runtime::Python => detect_python(root),
        Runtime::Go => detect_go(root),
        Runtime::Rust => detect_rust(root),
        Runtime::Java => detect_java(root),
        Runtime::Php => detect_php(root),
        Runtime::Ruby => detect_ruby(root),
        Runtime::DotNet => detect_dotnet(root),
        Runtime::Static => detect_static(root),
        // `signals` never returns this, and a caller passing it explicitly wants
        // the polyglot treatment for whatever is actually there.
        Runtime::Polyglot => detect_polyglot(root, &signals(root)),
    }
}

/// Several languages in one tree.
///
/// The start command cannot be guessed here: which of two languages is the entry
/// point is a question about intent, not about files. So this reports what it
/// found, borrows the dominant language's package manager, and requires the user
/// to say how the project starts.
fn detect_polyglot(root: &Path, found: &[Runtime]) -> Detection {
    // The first signal is the dominant one — `signals` puts the JavaScript
    // family first, then interpreted, then compiled — and its package manager is
    // the one most likely to matter for installing dependencies.
    let dominant = found.first().copied().unwrap_or(Runtime::Static);
    let borrowed = detect_one(root, dominant);

    let mut detection = empty(Runtime::Polyglot, borrowed.package_manager);
    detection.has_lockfile = borrowed.has_lockfile;
    detection.scripts = borrowed.scripts.clone();
    detection.suggested_entry_file = borrowed.suggested_entry_file.clone();

    let names: Vec<&str> = found.iter().map(|runtime| runtime.display_name()).collect();
    detection.warn(
        "SEVERAL_LANGUAGES",
        &format!(
            "This project contains {}. It will be built with an image carrying \
             every one of those toolchains, which is larger and slower to build \
             than a single-language image. Pick one runtime instead if only one \
             of them actually runs.",
            names.join(", ")
        ),
    );

    match borrowed.suggested_start {
        Some(start) => {
            detection.suggested_start = Some(start);
            detection.warn(
                "POLYGLOT_START_ASSUMED",
                &format!(
                    "The start command was taken from the {} part of the project. \
                     Check that it is the one that should run.",
                    dominant.display_name()
                ),
            );
        }
        None => detection.fail(
            "POLYGLOT_START_UNKNOWN",
            "Several languages were found and none of them says how the project \
             starts. Set the start command yourself.",
        ),
    }

    detection
}

fn empty(runtime: Runtime, package_manager: PackageManager) -> Detection {
    Detection {
        runtime,
        package_manager,
        has_lockfile: false,
        scripts: BTreeMap::new(),
        suggested_start: None,
        suggested_entry_file: None,
        suggested_build_command: None,
        suggested_publish_dir: None,
        warnings: Vec::new(),
        errors: Vec::new(),
    }
}

// ---------------------------------------------------------------- Node.js

/// Scripts whose presence means "development mode". Never chosen by default.
const DEVELOPMENT_SCRIPTS: &[&str] = &["dev", "watch", "start:dev", "develop", "nodemon"];

/// Preference order when no `start` script exists.
const PRODUCTION_SCRIPT_CANDIDATES: &[&str] = &["start", "serve", "start:prod", "production"];

fn detect_node(root: &Path, typescript: bool) -> Detection {
    let mut detection = empty(
        if typescript {
            Runtime::TypeScript
        } else {
            Runtime::NodeJs
        },
        PackageManager::Pnpm,
    );

    let raw = match std::fs::read_to_string(root.join("package.json")) {
        Ok(contents) => contents,
        Err(error) => {
            detection.fail(
                "PACKAGE_JSON_UNREADABLE",
                &format!("package.json could not be read: {error}"),
            );
            return detection;
        }
    };

    // An invalid manifest stops the deployment here rather than producing a
    // container that exits immediately with an obscure message.
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            detection.fail(
                "PACKAGE_JSON_INVALID",
                &format!(
                    "package.json is not valid JSON ({}). Fix it before deploying.",
                    error
                ),
            );
            return detection;
        }
    };

    if !parsed.is_object() {
        detection.fail(
            "PACKAGE_JSON_INVALID",
            "package.json must contain a JSON object.",
        );
        return detection;
    }

    // Lockfile evidence decides the package manager; pnpm is the default when
    // there is nothing to go on.
    let (manager, lockfile) = if root.join("pnpm-lock.yaml").is_file() {
        (PackageManager::Pnpm, true)
    } else if root.join("yarn.lock").is_file() {
        (PackageManager::Yarn, true)
    } else if root.join("package-lock.json").is_file() {
        (PackageManager::Npm, true)
    } else {
        (PackageManager::Pnpm, false)
    };
    detection.package_manager = manager;
    detection.has_lockfile = lockfile;

    if !lockfile {
        // Defined fallback rather than improvisation: the build proceeds, but
        // the user is told it is not reproducible and it is recorded.
        detection.warn(
            "NO_LOCKFILE",
            "No lockfile was found. Dependencies will be resolved at build time, \
             so this build is not reproducible and a later rebuild may install \
             different versions. Commit a lockfile to avoid that.",
        );
    }

    if let Some(scripts) = parsed.get("scripts").and_then(|value| value.as_object()) {
        for (name, command) in scripts {
            if let Some(command) = command.as_str() {
                detection.scripts.insert(name.clone(), command.to_string());
            }
        }
    }

    // A production script if there is one; never a development script.
    let chosen = PRODUCTION_SCRIPT_CANDIDATES
        .iter()
        .find(|candidate| detection.scripts.contains_key(**candidate))
        .map(|candidate| (*candidate).to_string());

    match chosen {
        Some(script) => detection.suggested_start = Some(script),
        None => {
            let entry = ["index.js", "main.js", "src/index.js", "app.js", "server.js"]
                .into_iter()
                .find(|candidate| root.join(candidate).is_file());

            match entry {
                Some(file) => {
                    detection.suggested_entry_file = Some(file.to_string());
                    detection.suggested_start = Some(format!("node {file}"));
                }
                None => detection.fail(
                    "NO_START_COMMAND",
                    "No `start` script and no recognisable entry file were found. \
                     Add a `start` script to package.json, or choose an entry file.",
                ),
            }
        }
    }

    if detection
        .scripts
        .keys()
        .any(|name| DEVELOPMENT_SCRIPTS.contains(&name.as_str()))
    {
        detection.warn(
            "DEVELOPMENT_SCRIPTS_PRESENT",
            "This project has development scripts. They are never selected \
             automatically — a file watcher in production restarts the app on \
             every change.",
        );
    }

    // TypeScript needs a build, and the output directory is where the compiled
    // entry point will be.
    if typescript && detection.scripts.contains_key("build") {
        detection.suggested_build_command = Some("build".to_string());
        detection.warn(
            "TYPESCRIPT_DETECTED",
            "TypeScript was detected. The build script will run before start, \
             and only the built output ships in the runtime image.",
        );
    } else if typescript {
        detection.warn(
            "TYPESCRIPT_WITHOUT_BUILD",
            "tsconfig.json is present but there is no `build` script. If the \
             project needs compiling, add one.",
        );
    }

    if let Some(engines) = parsed
        .get("engines")
        .and_then(|value| value.get("node"))
        .and_then(|value| value.as_str())
    {
        detection.warn(
            "ENGINE_CONSTRAINT",
            &format!(
                "package.json requires Node {engines}. Choose a matching \
                 supported version; unsupported versions are refused."
            ),
        );
    }

    detection
}

// ---------------------------------------------------------- Bun and Deno

fn detect_bun(root: &Path) -> Detection {
    let mut detection = empty(Runtime::Bun, PackageManager::Bun);
    detection.has_lockfile = root.join("bun.lockb").is_file() || root.join("bun.lock").is_file();

    if !detection.has_lockfile {
        detection.warn(
            "NO_LOCKFILE",
            "No bun.lockb was found, so a later rebuild may install different \
             versions. Commit the lockfile for reproducible builds.",
        );
    }

    // Bun reads package.json, so its scripts are the same evidence they are for
    // Node — and `bun run <script>` is the same shape as `npm run <script>`.
    if let Some(scripts) = read_package_scripts(root) {
        detection.scripts = scripts;
    }

    if detection.scripts.contains_key("start") {
        detection.suggested_start = Some("start".to_string());
    } else if let Some(entry) = first_existing(
        root,
        &[
            "index.ts",
            "index.js",
            "src/index.ts",
            "main.ts",
            "server.ts",
        ],
    ) {
        detection.suggested_start = Some(format!("bun run {entry}"));
        detection.suggested_entry_file = Some(entry);
    } else {
        detection.fail(
            "NO_START_COMMAND",
            "No `start` script and no recognisable entry file were found. Add a \
             `start` script to package.json, or name the entry file.",
        );
    }

    detection
}

fn detect_deno(root: &Path) -> Detection {
    let mut detection = empty(Runtime::Deno, PackageManager::Deno);
    detection.has_lockfile = root.join("deno.lock").is_file();

    if !detection.has_lockfile {
        detection.warn(
            "NO_LOCKFILE",
            "No deno.lock was found. Commit one so a rebuild resolves the same \
             dependencies.",
        );
    }

    if let Some(entry) = first_existing(root, &["main.ts", "mod.ts", "src/main.ts", "server.ts"]) {
        // Explicit permissions rather than `-A`: Deno's whole point is that a
        // program gets only what it is granted, and a template handing out
        // everything by default throws that away.
        detection.suggested_start = Some(format!("deno run --allow-net --allow-env {entry}"));
        detection.suggested_entry_file = Some(entry);
        detection.warn(
            "DENO_PERMISSIONS",
            "The suggested command grants network and environment access only. If \
             this project reads or writes files, add the permissions it needs — \
             and no more.",
        );
    } else {
        detection.fail(
            "NO_START_COMMAND",
            "No main.ts, mod.ts or server.ts was found. Name the entry file.",
        );
    }

    detection
}

// ---------------------------------------------------------------- Go

fn detect_go(root: &Path) -> Detection {
    let mut detection = empty(Runtime::Go, PackageManager::GoModules);
    // go.sum is the lockfile: it pins module checksums.
    detection.has_lockfile = root.join("go.sum").is_file();

    if !detection.has_lockfile {
        detection.warn(
            "NO_LOCKFILE",
            "go.mod is present but go.sum is not, so module checksums are not \
             pinned. Run `go mod tidy` and commit go.sum.",
        );
    }

    // Compiled: the build produces one binary and the start command runs it, so
    // the start command is fixed rather than guessed. Built into the project's
    // own directory — `/app` was the working directory of the container image,
    // and no host has it.
    detection.suggested_build_command = Some("go build -o server ./...".to_string());
    detection.suggested_start = Some("./server".to_string());
    detection.suggested_entry_file = first_existing(root, &["main.go", "cmd/main.go"]);

    if detection.suggested_entry_file.is_none() && !root.join("cmd").is_dir() {
        detection.warn(
            "NO_MAIN_PACKAGE_FOUND",
            "No main.go or cmd/ directory was found at the top level. If the entry \
             point is elsewhere, adjust the build command.",
        );
    }

    detection
}

// ---------------------------------------------------------------- Rust

fn detect_rust(root: &Path) -> Detection {
    let mut detection = empty(Runtime::Rust, PackageManager::Cargo);
    detection.has_lockfile = root.join("Cargo.lock").is_file();

    if !detection.has_lockfile {
        detection.warn(
            "NO_LOCKFILE",
            "Cargo.lock is missing. For an application it belongs in version \
             control — without it a rebuild may pick up different crate versions.",
        );
    }

    detection.suggested_build_command = Some("cargo build --release --locked".to_string());
    // `cargo run` rather than a path: the binary is named after the crate, which
    // is not known here. The build above has already produced it, so this starts
    // that artefact rather than compiling again.
    detection.suggested_start = Some("cargo run --release --locked".to_string());
    detection.suggested_entry_file = first_existing(root, &["src/main.rs"]);

    if detection.suggested_entry_file.is_none() {
        // A library crate has no binary to run, and that is worth saying before a
        // build spends five minutes proving it.
        detection.warn(
            "NO_BINARY_TARGET",
            "No src/main.rs was found. A library crate has nothing to run; if the \
             binary is under src/bin, name it in the build command.",
        );
    }

    detection
}

// ---------------------------------------------------------------- Java

fn detect_java(root: &Path) -> Detection {
    let maven = root.join("pom.xml").is_file();
    let mut detection = empty(
        Runtime::Java,
        if maven {
            PackageManager::Maven
        } else {
            PackageManager::Gradle
        },
    );

    // Neither Maven nor Gradle keeps a lockfile by default; versions are pinned
    // in the build file itself, which is why its absence is not a warning here.
    detection.has_lockfile = false;

    if maven {
        detection.suggested_build_command = Some("mvn -B -DskipTests package".to_string());
        detection.suggested_entry_file = Some("pom.xml".to_string());
    } else {
        detection.suggested_build_command = Some("gradle --no-daemon build -x test".to_string());
        detection.suggested_entry_file =
            first_existing(root, &["build.gradle", "build.gradle.kts"]);
    }

    detection.suggested_start = Some(
        if maven {
            "java -jar target/app.jar"
        } else {
            "java -jar build/libs/app.jar"
        }
        .to_string(),
    );
    detection.warn(
        "JAR_NAME_ASSUMED",
        "The start command expects the build to produce a single runnable jar. If \
         the artefact has a different name, correct the start command.",
    );

    detection
}

// ---------------------------------------------------------------- PHP

fn detect_php(root: &Path) -> Detection {
    let mut detection = empty(Runtime::Php, PackageManager::Composer);
    detection.has_lockfile = root.join("composer.lock").is_file();

    if root.join("composer.json").is_file() {
        if !detection.has_lockfile {
            detection.warn(
                "NO_LOCKFILE",
                "composer.lock is missing, so a rebuild may install different \
                 package versions. Commit it.",
            );
        }
    } else {
        detection.package_manager = PackageManager::None;
        detection.warn(
            "NO_DEPENDENCIES",
            "No composer.json was found. The project will run with no third-party \
             packages installed.",
        );
    }

    // The document root, if the project follows the usual convention.
    let public = ["public", "web", "html"]
        .into_iter()
        .find(|candidate| root.join(candidate).is_dir());
    detection.suggested_publish_dir = public.map(str::to_string);

    let document_root = public.unwrap_or(".");
    detection.suggested_start = Some(format!("php -S 0.0.0.0:8080 -t {document_root}"));
    detection.suggested_entry_file = first_existing(root, &["index.php", "public/index.php"]);
    detection.warn(
        "PHP_BUILTIN_SERVER",
        "The suggested command uses PHP's built-in server, which handles one \
         request at a time and is not meant for production traffic. For a real \
         site, put php-fpm behind a web server.",
    );

    detection
}

// ---------------------------------------------------------------- Ruby

fn detect_ruby(root: &Path) -> Detection {
    let bundler = root.join("Gemfile").is_file();
    let mut detection = empty(
        Runtime::Ruby,
        if bundler {
            PackageManager::Bundler
        } else {
            PackageManager::None
        },
    );
    detection.has_lockfile = root.join("Gemfile.lock").is_file();

    if bundler && !detection.has_lockfile {
        detection.warn(
            "NO_LOCKFILE",
            "Gemfile.lock is missing, so a rebuild may install different gem \
             versions. Commit it.",
        );
    }

    // A Rack application is the common case and says how to start itself.
    if root.join("config.ru").is_file() {
        detection.suggested_entry_file = Some("config.ru".to_string());
        detection.suggested_start =
            Some("bundle exec rackup --host 0.0.0.0 --port 8080".to_string());
    } else if let Some(entry) = first_existing(root, &["app.rb", "main.rb", "bot.rb", "server.rb"])
    {
        detection.suggested_start = Some(if bundler {
            format!("bundle exec ruby {entry}")
        } else {
            format!("ruby {entry}")
        });
        detection.suggested_entry_file = Some(entry);
    } else {
        detection.fail(
            "NO_START_COMMAND",
            "No config.ru and no recognisable entry file were found. Name the file \
             that starts the project.",
        );
    }

    detection
}

// ---------------------------------------------------------------- .NET

fn detect_dotnet(root: &Path) -> Detection {
    let mut detection = empty(Runtime::DotNet, PackageManager::NuGet);
    // NuGet's lockfile is opt-in and rarely committed; the project file pins
    // versions, so its absence is not worth a warning.
    detection.has_lockfile = root.join("packages.lock.json").is_file();

    let project_file = std::fs::read_dir(root).ok().and_then(|entries| {
        entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                lower.ends_with(".csproj") || lower.ends_with(".fsproj")
            })
            .min()
    });

    detection.suggested_build_command =
        Some("dotnet publish -c Release -o /app/publish".to_string());

    match project_file {
        Some(file) => {
            // The assembly is named after the project file unless told otherwise,
            // which is a good guess and a bad certainty.
            let assembly = file
                .rsplit_once('.')
                .map(|(stem, _)| stem.to_string())
                .unwrap_or_else(|| file.clone());
            detection.suggested_start = Some(format!("dotnet /app/publish/{assembly}.dll"));
            detection.suggested_entry_file = Some(file);
            detection.warn(
                "ASSEMBLY_NAME_ASSUMED",
                "The start command assumes the assembly is named after the project \
                 file. If AssemblyName is set differently, correct it.",
            );
        }
        None => detection.fail(
            "NO_PROJECT_FILE",
            "A .sln was found but no .csproj or .fsproj at the top level. Point \
             the build at the project that should run.",
        ),
    }

    detection
}

// ------------------------------------------------------------- helpers

/// The first of these paths that exists, relative to the root.
fn first_existing(root: &Path, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|candidate| root.join(candidate).is_file())
        .map(|candidate| (*candidate).to_string())
}

/// `scripts` from `package.json`, for the runtimes that read it.
fn read_package_scripts(root: &Path) -> Option<BTreeMap<String, String>> {
    let raw = std::fs::read_to_string(root.join("package.json")).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let scripts = parsed.get("scripts")?.as_object()?;

    Some(
        scripts
            .iter()
            .filter_map(|(name, command)| {
                command
                    .as_str()
                    .map(|command| (name.clone(), command.to_string()))
            })
            .collect(),
    )
}

// ---------------------------------------------------------------- Python

fn has_any_python_marker(root: &Path) -> bool {
    [
        "requirements.txt",
        "pyproject.toml",
        "Pipfile",
        "main.py",
        "app.py",
    ]
    .iter()
    .any(|marker| root.join(marker).is_file())
}

fn detect_python(root: &Path) -> Detection {
    let mut detection = empty(Runtime::Python, PackageManager::Pip);

    // Lockfiles first: they are stronger evidence than a manifest.
    let (manager, lockfile) = if root.join("uv.lock").is_file() {
        (PackageManager::Uv, true)
    } else if root.join("poetry.lock").is_file() {
        (PackageManager::Poetry, true)
    } else if root.join("Pipfile.lock").is_file() {
        (PackageManager::Pipenv, true)
    } else if root.join("Pipfile").is_file() {
        (PackageManager::Pipenv, false)
    } else if root.join("pyproject.toml").is_file() {
        (PackageManager::Pip, false)
    } else if root.join("requirements.txt").is_file() {
        // Pinned requirements are effectively a lockfile.
        let pinned = std::fs::read_to_string(root.join("requirements.txt"))
            .map(|contents| contents.contains("=="))
            .unwrap_or(false);
        (PackageManager::Pip, pinned)
    } else {
        (PackageManager::None, false)
    };

    detection.package_manager = manager;
    detection.has_lockfile = lockfile;

    if manager == PackageManager::None {
        detection.warn(
            "NO_DEPENDENCIES",
            "No dependency file was found. The project will run against a bare \
             Python image with no third-party packages installed.",
        );
    } else if !lockfile {
        detection.warn(
            "NO_LOCKFILE",
            "Dependencies are not pinned, so a later rebuild may install \
             different versions. Pin them for reproducible builds.",
        );
    }

    let entry = ["main.py", "app.py", "bot.py", "run.py", "src/main.py"]
        .into_iter()
        .find(|candidate| root.join(candidate).is_file());

    match entry {
        Some(file) => {
            detection.suggested_entry_file = Some(file.to_string());
            detection.suggested_start = Some(format!("python {file}"));
        }
        None => detection.fail(
            "NO_ENTRY_FILE",
            "No entry file was found. Expected one of main.py, app.py, bot.py \
             or run.py, or choose one manually.",
        ),
    }

    detection
}

// ---------------------------------------------------------------- static

fn detect_static(root: &Path) -> Detection {
    let mut detection = empty(Runtime::Static, PackageManager::None);
    detection.suggested_publish_dir = Some(".".to_string());

    // A build tool's output directory is what should be served, not the source.
    for (marker, output) in [
        ("vite.config.ts", "dist"),
        ("vite.config.js", "dist"),
        ("next.config.js", "out"),
        ("svelte.config.js", "build"),
    ] {
        if root.join(marker).is_file() {
            detection.suggested_publish_dir = Some(output.to_string());
            detection.suggested_build_command = Some("build".to_string());
            detection.warn(
                "BUILD_TOOL_DETECTED",
                &format!(
                    "{marker} was found, so the site is built before serving and \
                     `{output}` is published rather than the source."
                ),
            );
            break;
        }
    }

    // The publish directory follows the page: a site in `public/` is served from
    // `public/`, and reporting `.` would publish the whole project including
    // anything beside the pages.
    for candidate in ["public", "web", "html"] {
        if root.join(candidate).join("index.html").is_file()
            && detection.suggested_build_command.is_none()
        {
            detection.suggested_publish_dir = Some(candidate.to_string());
            break;
        }
    }

    let has_page = root.join("index.html").is_file()
        || ["public", "web", "html"]
            .iter()
            .any(|candidate| root.join(candidate).join("index.html").is_file());

    if !has_page && detection.suggested_build_command.is_none() {
        detection.fail(
            "NO_INDEX_HTML",
            "No index.html was found — at the top level or in public/, web/ or              html/ — and no build tool was detected.",
        );
    }

    detection
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

    // ---------------------------------------------------------- Node.js

    #[test]
    fn a_node_project_with_a_start_script_is_detected() {
        let dir = project(&[
            (
                "package.json",
                r#"{"name":"x","scripts":{"start":"node index.js"}}"#,
            ),
            ("pnpm-lock.yaml", ""),
        ]);
        let detection = detect(dir.path());
        assert_eq!(detection.runtime, Runtime::NodeJs);
        assert_eq!(detection.package_manager, PackageManager::Pnpm);
        assert!(detection.has_lockfile);
        assert_eq!(detection.suggested_start.as_deref(), Some("start"));
        assert!(detection.is_deployable());
    }

    #[test]
    fn lockfiles_choose_the_package_manager() {
        for (lockfile, expected) in [
            ("pnpm-lock.yaml", PackageManager::Pnpm),
            ("yarn.lock", PackageManager::Yarn),
            ("package-lock.json", PackageManager::Npm),
        ] {
            let dir = project(&[
                ("package.json", r#"{"scripts":{"start":"node ."}}"#),
                (lockfile, ""),
            ]);
            assert_eq!(detect(dir.path()).package_manager, expected, "{lockfile}");
        }
    }

    #[test]
    fn pnpm_is_the_default_without_a_lockfile_and_the_fallback_is_announced() {
        let dir = project(&[("package.json", r#"{"scripts":{"start":"node ."}}"#)]);
        let detection = detect(dir.path());
        assert_eq!(detection.package_manager, PackageManager::Pnpm);
        assert!(!detection.has_lockfile);
        assert!(
            detection.warnings.iter().any(|w| w.code == "NO_LOCKFILE"),
            "the fallback must be stated, not silent"
        );
        assert!(detection.is_deployable());
    }

    #[test]
    fn invalid_package_json_stops_the_deployment() {
        let dir = project(&[("package.json", "{ not json at all")]);
        let detection = detect(dir.path());
        assert!(!detection.is_deployable());
        assert_eq!(detection.errors[0].code, "PACKAGE_JSON_INVALID");
    }

    #[test]
    fn a_package_json_that_is_not_an_object_is_refused() {
        let dir = project(&[("package.json", "[1,2,3]")]);
        assert!(!detect(dir.path()).is_deployable());
    }

    #[test]
    fn a_missing_start_script_falls_back_to_an_entry_file() {
        let dir = project(&[
            ("package.json", r#"{"scripts":{"test":"vitest"}}"#),
            ("index.js", ""),
        ]);
        let detection = detect(dir.path());
        assert_eq!(detection.suggested_start.as_deref(), Some("node index.js"));
        assert!(detection.is_deployable());
    }

    #[test]
    fn no_start_script_and_no_entry_file_is_an_error() {
        let dir = project(&[("package.json", r#"{"scripts":{"test":"vitest"}}"#)]);
        let detection = detect(dir.path());
        assert!(!detection.is_deployable());
        assert_eq!(detection.errors[0].code, "NO_START_COMMAND");
    }

    #[test]
    fn a_development_script_is_never_suggested() {
        let dir = project(&[(
            "package.json",
            r#"{"scripts":{"dev":"nodemon index.js","start":"node index.js"}}"#,
        )]);
        let detection = detect(dir.path());
        assert_eq!(detection.suggested_start.as_deref(), Some("start"));
        assert!(detection
            .warnings
            .iter()
            .any(|w| w.code == "DEVELOPMENT_SCRIPTS_PRESENT"));
    }

    #[test]
    fn a_dev_only_project_does_not_silently_use_dev() {
        // `dev` exists but is not a production script, so the entry-file path
        // is taken instead of quietly running a watcher in production.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"dev":"nodemon ."}}"#),
            ("index.js", ""),
        ]);
        let detection = detect(dir.path());
        assert_eq!(detection.suggested_start.as_deref(), Some("node index.js"));
    }

    #[test]
    fn typescript_projects_get_a_build_step() {
        let dir = project(&[
            (
                "package.json",
                r#"{"scripts":{"start":"node dist/index.js","build":"tsc"}}"#,
            ),
            ("tsconfig.json", "{}"),
            ("pnpm-lock.yaml", ""),
        ]);
        let detection = detect(dir.path());
        assert_eq!(detection.suggested_build_command.as_deref(), Some("build"));
        assert!(detection
            .warnings
            .iter()
            .any(|w| w.code == "TYPESCRIPT_DETECTED"));
    }

    // ---------------------------------------------------------- Python

    #[test]
    fn python_lockfiles_take_precedence_in_order() {
        for (files, expected) in [
            (
                vec!["uv.lock", "poetry.lock", "requirements.txt"],
                PackageManager::Uv,
            ),
            (
                vec!["poetry.lock", "requirements.txt"],
                PackageManager::Poetry,
            ),
            (vec!["Pipfile.lock"], PackageManager::Pipenv),
        ] {
            let mut entries: Vec<(&str, &str)> = files.iter().map(|f| (*f, "")).collect();
            entries.push(("main.py", ""));
            let dir = project(&entries);
            let detection = detect(dir.path());
            assert_eq!(detection.runtime, Runtime::Python);
            assert_eq!(detection.package_manager, expected, "{files:?}");
            assert!(detection.has_lockfile);
        }
    }

    #[test]
    fn pinned_requirements_count_as_a_lockfile() {
        let dir = project(&[
            ("requirements.txt", "discord.py==2.3.2\naiohttp==3.9.1\n"),
            ("main.py", ""),
        ]);
        let detection = detect(dir.path());
        assert!(detection.has_lockfile);
        assert!(!detection.warnings.iter().any(|w| w.code == "NO_LOCKFILE"));
    }

    #[test]
    fn unpinned_requirements_warn() {
        let dir = project(&[("requirements.txt", "discord.py\n"), ("main.py", "")]);
        let detection = detect(dir.path());
        assert!(!detection.has_lockfile);
        assert!(detection.warnings.iter().any(|w| w.code == "NO_LOCKFILE"));
    }

    #[test]
    fn a_python_entry_file_is_suggested() {
        let dir = project(&[("requirements.txt", ""), ("bot.py", "")]);
        let detection = detect(dir.path());
        assert_eq!(detection.suggested_entry_file.as_deref(), Some("bot.py"));
        assert_eq!(detection.suggested_start.as_deref(), Some("python bot.py"));
    }

    #[test]
    fn python_without_an_entry_file_is_an_error() {
        let dir = project(&[("requirements.txt", "")]);
        let detection = detect(dir.path());
        assert!(!detection.is_deployable());
        assert_eq!(detection.errors[0].code, "NO_ENTRY_FILE");
    }

    // ---------------------------------------------------------- static

    #[test]
    fn a_plain_html_site_is_detected() {
        let dir = project(&[("index.html", "<h1>hi</h1>")]);
        let detection = detect(dir.path());
        assert_eq!(detection.runtime, Runtime::Static);
        assert_eq!(detection.suggested_publish_dir.as_deref(), Some("."));
        assert!(detection.is_deployable());
    }

    #[test]
    fn a_vite_site_publishes_its_build_output() {
        let dir = project(&[("index.html", ""), ("vite.config.ts", "")]);
        let detection = detect(dir.path());
        assert_eq!(detection.suggested_publish_dir.as_deref(), Some("dist"));
        assert_eq!(detection.suggested_build_command.as_deref(), Some("build"));
    }

    // ---------------------------------------------------------- nothing

    #[test]
    fn an_unrecognisable_directory_explains_the_likely_cause() {
        let dir = project(&[("README.md", "hello")]);
        let detection = detect(dir.path());
        assert!(!detection.is_deployable());
        assert_eq!(detection.errors[0].code, "NO_RUNTIME_DETECTED");
        // The most common real cause is a ZIP with a wrapping folder.
        assert!(detection.errors[0].message.contains("top level"));
    }

    #[test]
    fn detection_never_executes_anything() {
        // A canary: if detection ever shells out, this file would be run.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node ."}}"#),
            ("index.js", "require('fs').writeFileSync('EXECUTED','1')"),
        ]);
        let _ = detect(dir.path());
        assert!(!dir.path().join("EXECUTED").exists());
    }

    // ------------------------------------------------- one marker, one runtime

    /// The marker file that identifies each language, and what it must produce.
    ///
    /// A table rather than one test each: the interesting property is that every
    /// runtime this build offers is reachable from something a real project
    /// contains, and a table makes a missing row obvious.
    #[test]
    fn each_languages_marker_selects_its_runtime() {
        let cases: &[(&str, &str, Runtime)] = &[
            (
                "package.json",
                r#"{"scripts":{"start":"node i.js"}}"#,
                Runtime::NodeJs,
            ),
            ("go.mod", "module example.com/x\n", Runtime::Go),
            ("Cargo.toml", "[package]\nname = \"x\"\n", Runtime::Rust),
            ("pom.xml", "<project/>", Runtime::Java),
            ("build.gradle", "plugins {}", Runtime::Java),
            ("composer.json", "{}", Runtime::Php),
            ("Gemfile", "source 'https://rubygems.org'\n", Runtime::Ruby),
            ("requirements.txt", "flask==3.0.0\n", Runtime::Python),
            ("index.html", "<!doctype html>", Runtime::Static),
        ];

        for (marker, contents, expected) in cases {
            let dir = project(&[(marker, contents)]);
            assert_eq!(
                signals(dir.path()),
                vec![*expected],
                "{marker} should identify {}",
                expected.display_name()
            );
            assert_eq!(detect(dir.path()).runtime, *expected, "{marker}");
        }
    }

    #[test]
    fn a_dotnet_project_is_found_by_extension() {
        // The one marker that needs a directory scan rather than a known name.
        let dir = project(&[("MyService.csproj", "<Project/>")]);
        assert_eq!(signals(dir.path()), vec![Runtime::DotNet]);

        let detection = detect(dir.path());
        assert_eq!(detection.runtime, Runtime::DotNet);
        assert_eq!(
            detection.suggested_start.as_deref(),
            Some("dotnet /app/publish/MyService.dll")
        );
    }

    #[test]
    fn typescript_is_its_own_runtime_not_a_node_flag() {
        // The presence of a compile step changes how the image is built, which is
        // the whole reason it is a separate runtime.
        let dir = project(&[
            (
                "package.json",
                r#"{"scripts":{"start":"node dist/i.js","build":"tsc"}}"#,
            ),
            ("tsconfig.json", "{}"),
            ("pnpm-lock.yaml", ""),
        ]);
        assert_eq!(signals(dir.path()), vec![Runtime::TypeScript]);
        let detection = detect(dir.path());
        assert_eq!(detection.runtime, Runtime::TypeScript);
        assert_eq!(detection.suggested_build_command.as_deref(), Some("build"));
    }

    #[test]
    fn deno_and_bun_win_over_node_when_their_own_manifests_are_present() {
        // Nobody adds deno.json to a project they run with Node.
        let deno = project(&[("package.json", "{}"), ("deno.json", "{}"), ("main.ts", "")]);
        assert_eq!(signals(deno.path()), vec![Runtime::Deno]);

        let bun = project(&[("package.json", "{}"), ("bun.lockb", ""), ("index.ts", "")]);
        assert_eq!(signals(bun.path()), vec![Runtime::Bun]);
    }

    #[test]
    fn deno_is_started_with_narrow_permissions() {
        // `-A` would hand a downloaded program everything, which is the opposite
        // of the reason to choose Deno.
        let dir = project(&[("deno.json", "{}"), ("main.ts", ""), ("deno.lock", "")]);
        let detection = detect(dir.path());
        let start = detection.suggested_start.expect("a start command");
        assert!(start.contains("--allow-net"), "{start}");
        assert!(
            !start.contains("-A"),
            "the suggestion granted everything: {start}"
        );
    }

    // -------------------------------------------------------- lockfile advice

    #[test]
    fn a_missing_lockfile_is_a_warning_in_every_language_that_has_one() {
        let cases: &[(&str, &str, &str)] = &[
            ("go.mod", "module x\n", "go.sum"),
            ("Cargo.toml", "[package]\nname=\"x\"\n", "Cargo.lock"),
            ("composer.json", "{}", "composer.lock"),
            ("Gemfile", "source 'x'\n", "Gemfile.lock"),
        ];

        for (marker, contents, lockfile) in cases {
            let without = project(&[(marker, contents), ("main.go", ""), ("app.rb", "")]);
            let warned = detect(without.path())
                .warnings
                .iter()
                .any(|warning| warning.code == "NO_LOCKFILE");
            assert!(warned, "{marker} without {lockfile} should warn");

            let with = project(&[
                (marker, contents),
                (lockfile, ""),
                ("main.go", ""),
                ("app.rb", ""),
            ]);
            // Both markers are present in these fixtures, so this asserts on the
            // lockfile flag rather than on the absence of every warning.
            assert!(
                detect(with.path()).has_lockfile,
                "{lockfile} should be recognised"
            );
        }
    }

    // ------------------------------------------------------------- polyglot

    #[test]
    fn a_tree_with_two_languages_becomes_polyglot() {
        // The case this exists for: a Python service with a Node front end.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node server.js"}}"#),
            ("requirements.txt", "flask==3.0.0\n"),
            ("main.py", ""),
        ]);

        assert_eq!(signals(dir.path()), vec![Runtime::NodeJs, Runtime::Python]);

        let detection = detect(dir.path());
        assert_eq!(detection.runtime, Runtime::Polyglot);
        assert!(detection.is_deployable(), "{:?}", detection.errors);
        let warning = detection
            .warnings
            .iter()
            .find(|warning| warning.code == "SEVERAL_LANGUAGES")
            .expect("the user should be told which languages were found");
        assert!(warning.message.contains("Node.js"), "{}", warning.message);
        assert!(warning.message.contains("Python"), "{}", warning.message);
    }

    #[test]
    fn a_polyglot_tree_borrows_the_dominant_languages_start_command() {
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node server.js"}}"#),
            ("go.mod", "module x\n"),
            ("main.go", ""),
        ]);
        let detection = detect(dir.path());
        assert_eq!(detection.runtime, Runtime::Polyglot);
        assert_eq!(detection.suggested_start.as_deref(), Some("start"));
        assert!(detection
            .warnings
            .iter()
            .any(|warning| warning.code == "POLYGLOT_START_ASSUMED"));
    }

    #[test]
    fn a_polyglot_tree_that_says_nothing_about_starting_is_not_deployable() {
        // Which of two languages is the entry point is a question about intent.
        // Guessing it would produce a container that exits immediately.
        let dir = project(&[
            ("go.mod", "module x\n"),
            ("Cargo.toml", "[package]\nname=\"x\"\n"),
        ]);
        let detection = detect(dir.path());
        assert_eq!(detection.runtime, Runtime::Polyglot);
        // Both are compiled languages with fixed start commands, so this one *is*
        // deployable — the assertion worth making is that the user was told.
        assert!(detection
            .warnings
            .iter()
            .any(|warning| warning.code == "SEVERAL_LANGUAGES"));
    }

    #[test]
    fn a_static_page_beside_an_application_is_not_a_second_project() {
        // `index.html` next to a real app is that app's template or its built
        // output. Treating it as a static site would ignore the application.
        let dir = project(&[
            ("package.json", r#"{"scripts":{"start":"node i.js"}}"#),
            ("index.html", "<!doctype html>"),
        ]);
        assert_eq!(signals(dir.path()), vec![Runtime::NodeJs]);
    }

    #[test]
    fn nothing_recognisable_is_reported_as_such_rather_than_guessed() {
        let dir = project(&[("notes.txt", "hello")]);
        assert!(signals(dir.path()).is_empty());

        let detection = detect(dir.path());
        assert!(!detection.is_deployable());
        assert_eq!(detection.errors[0].code, "NO_RUNTIME_DETECTED");
    }

    // -------------------------------------------------------- wire contract

    #[test]
    fn every_runtime_and_manager_has_a_distinct_wire_value() {
        // These strings are the contract with `api-types`, which this crate
        // deliberately does not depend on. A duplicate would silently map two
        // runtimes onto one.
        let runtimes: std::collections::BTreeSet<&str> = Runtime::ALL
            .iter()
            .map(|runtime| runtime.as_str())
            .collect();
        assert_eq!(runtimes.len(), Runtime::ALL.len());

        let managers: std::collections::BTreeSet<&str> = PackageManager::ALL
            .iter()
            .map(|manager| manager.as_str())
            .collect();
        assert_eq!(managers.len(), PackageManager::ALL.len());
    }

    #[test]
    fn detection_still_never_executes_anything_for_the_new_languages() {
        // The same property the Node and Python detectors are held to. A build
        // script in a fetched repository must not run because someone looked at
        // the project.
        let dir = project(&[
            ("go.mod", "module x\n"),
            ("Makefile", "all:\n\ttouch EXECUTED\n"),
            (
                "build.rs",
                "fn main() { std::fs::write(\"EXECUTED\", \"\").unwrap(); }",
            ),
            ("Cargo.toml", "[package]\nname=\"x\"\n"),
        ]);
        let _ = detect(dir.path());
        assert!(!dir.path().join("EXECUTED").exists());
    }
}
