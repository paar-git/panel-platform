//! Turning a project's runtime spec into a command this machine can run.
//!
//! Pure on purpose. Nothing here spawns anything, which is what makes every
//! runtime's translation testable on a machine that has none of them installed
//! — and this is the layer where the mistakes are, not in the spawning.
//!
//! The trap it exists to prevent: `RuntimeSpec::working_dir` defaults to
//! `/app`. That is a path inside a container image, and on the host it does not
//! exist. A host command must run in the project's own directory, and
//! [`start_command`] is the only place that decides so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::probe::{ExecutableResolver, Toolchain};

/// A command ready to be spawned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("{runtime} is not installed on this machine; looked for {}", looked_for.join(", "))]
    ToolchainMissing {
        runtime: String,
        looked_for: Vec<String>,
    },
    #[error("the project has no start command")]
    NoStartCommand,
    #[error("the start command `{0}` could not be read as a command")]
    Unparsable(String),
    /// The first word of the start command is not a program on this machine.
    ///
    /// Distinct from [`CommandError::ToolchainMissing`], which is about the
    /// language. A machine can have Node and still not have `pnpm`, and telling
    /// the user to install Node would send them to fix something that is not
    /// broken.
    #[error(
        "`{program}` is not a program on this machine. \
         It is the first word of the start command `{command}`; \
         install it, or change the start command to something that is installed."
    )]
    ProgramNotFound { program: String, command: String },
}

/// Everything needed to build one command.
#[derive(Debug, Clone)]
pub struct CommandInputs<'a> {
    pub runtime: &'a str,
    /// The spec's command, as the user or the planner wrote it.
    pub command: &'a str,
    /// Where the project's files actually are on this machine.
    pub project_directory: &'a Path,
    pub toolchain: &'a Toolchain,
    /// How a bare program name becomes a path this machine can execute.
    ///
    /// Needed because the start command's first word is usually *not* the
    /// language toolchain. `npm start` runs `npm`, not `node`, and on Windows
    /// `npm` is `npm.cmd` — a name `CreateProcess` will not find, because its
    /// own `PATH` search only ever appends `.exe`. Resolving here is what makes
    /// `npm start`, `pnpm dev` and `yarn serve` work on Windows at all.
    pub resolver: &'a dyn ExecutableResolver,
    pub env: BTreeMap<String, String>,
    /// The host port allocated to this project, passed through the
    /// environment. There is no port mapping without a container: the process
    /// binds the real port itself, so it has to be told which one.
    pub port: Option<u16>,
}

/// Build the command that starts a project.
pub fn start_command(inputs: CommandInputs<'_>) -> Result<ProcessCommand, CommandError> {
    if let Toolchain::Missing { looked_for } = inputs.toolchain {
        return Err(CommandError::ToolchainMissing {
            runtime: inputs.runtime.to_string(),
            looked_for: looked_for.clone(),
        });
    }

    let mut words = split_command(inputs.command)?;
    if words.is_empty() {
        return Err(CommandError::NoStartCommand);
    }
    let program = resolve_program(
        &words.remove(0),
        inputs.command,
        inputs.project_directory,
        inputs.resolver,
    )?;

    let mut env = inputs.env;
    if let Some(port) = inputs.port {
        env.insert("PORT".to_string(), port.to_string());
    }

    Ok(ProcessCommand {
        program,
        args: words,
        // Never `working_dir`: that is `/app`, a path inside an image.
        cwd: inputs.project_directory.to_path_buf(),
        env,
    })
}

/// Turn the start command's first word into something spawnable.
///
/// Three cases, in order:
///
/// * A word that already contains a separator is a path the user wrote. It is
///   not resolved against `PATH` — that would be ignoring what they said — but
///   a *relative* one is anchored to the project's directory. It has to be:
///   `Command` hands the program name to the operating system, and Windows
///   resolves a relative program against *this* process's working directory
///   rather than the child's `current_dir`. `./server` would be looked for
///   beside the application rather than beside the project, and a build that
///   had just succeeded would start nothing.
/// * A bare name is resolved against `PATH`, with the platform's executable
///   suffixes. This is the case that matters: `npm` on Windows is `npm.cmd`.
/// * A bare name that resolves to nothing is refused *by name*, before anything
///   is spawned, rather than becoming an operating-system error about a file
///   that does not exist.
fn resolve_program(
    word: &str,
    command: &str,
    project_directory: &Path,
    resolver: &dyn ExecutableResolver,
) -> Result<String, CommandError> {
    if word.contains('/') || word.contains('\\') {
        let written = Path::new(word);
        if written.is_absolute() {
            return Ok(word.to_string());
        }
        // Component by component rather than a plain `join`, so the `./` that
        // a start command almost always carries does not survive into the
        // middle of the path as `demo\.\server`. Spawnable either way, but
        // the path is shown to the user when a start fails.
        let mut anchored = project_directory.to_path_buf();
        for part in written.components() {
            match part {
                std::path::Component::CurDir => {}
                other => anchored.push(other),
            }
        }
        return Ok(anchored.to_string_lossy().into_owned());
    }

    match resolver.resolve(word) {
        Some(path) => Ok(path.to_string_lossy().into_owned()),
        None => Err(CommandError::ProgramNotFound {
            program: word.to_string(),
            command: command.to_string(),
        }),
    }
}

/// Split a command line into words, respecting single and double quotes.
///
/// Not a shell. There is deliberately no expansion, no globbing, no pipes and
/// no redirection: a start command that needs those is asking for a shell, and
/// handing one an unquoted project path is how a directory with a space in it
/// becomes two arguments — or worse.
///
/// Public because a project's `install_command` needs the same treatment
/// before it can be planned as a step, and a second implementation of this
/// would be a second thing to get wrong.
pub fn split_command(command: &str) -> Result<Vec<String>, CommandError> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;

    for character in command.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                // A quoted empty string is still an argument.
                started = true;
            }
            None if character.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(character);
                started = true;
            }
        }
    }

    if quote.is_some() {
        return Err(CommandError::Unparsable(command.to_string()));
    }
    if started {
        words.push(current);
    }

    Ok(words)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;

    fn found() -> Toolchain {
        Toolchain::Found {
            executable: PathBuf::from("/usr/bin/node"),
            version: Some("v22.3.0".to_string()),
        }
    }

    /// A machine that has whatever the test says it has, at a predictable path.
    ///
    /// `installed` holds the name *as the machine spells it*, so a test can
    /// describe a Windows box where `npm` is only ever `npm.cmd`.
    #[derive(Debug)]
    struct FakeMachine {
        installed: Vec<&'static str>,
    }

    impl ExecutableResolver for FakeMachine {
        fn resolve(&self, name: &str) -> Option<PathBuf> {
            self.installed
                .iter()
                .find(|entry| entry.split('.').next() == Some(name))
                .map(|entry| PathBuf::from(format!("/usr/bin/{entry}")))
        }

        fn version(&self, _executable: &Path) -> Option<String> {
            None
        }
    }

    /// The ordinary machine most tests want: everything they mention is there.
    fn anything() -> FakeMachine {
        FakeMachine {
            installed: vec![
                "node", "npm", "pnpm", "yarn", "python", "python3", "go", "cargo", "bun",
            ],
        }
    }

    fn inputs<'a>(
        runtime: &'a str,
        command: &'a str,
        toolchain: &'a Toolchain,
        resolver: &'a dyn ExecutableResolver,
    ) -> CommandInputs<'a> {
        CommandInputs {
            runtime,
            command,
            project_directory: Path::new("/projects/demo"),
            toolchain,
            resolver,
            env: BTreeMap::new(),
            port: None,
        }
    }

    /// The whole reason this module is pure. `working_dir` is `/app` in every
    /// runtime spec the planner writes, and on the host that directory does not
    /// exist — a command that ran there would fail with a path error that says
    /// nothing about the real problem.
    #[test]
    fn the_container_working_directory_never_reaches_a_host_command() {
        let toolchain = found();
        let machine = anything();
        let command =
            start_command(inputs("NODEJS", "npm start", &toolchain, &machine)).expect("command");

        assert_eq!(command.cwd, PathBuf::from("/projects/demo"));
        assert!(
            !command.cwd.to_string_lossy().contains("app"),
            "the container path leaked into the host command: {:?}",
            command.cwd
        );
    }

    #[test]
    fn a_command_is_split_into_a_program_and_its_arguments() {
        let toolchain = found();
        let machine = anything();
        let command = start_command(inputs(
            "NODEJS",
            "node server.js --port 3000",
            &toolchain,
            &machine,
        ))
        .expect("c");

        assert!(command.program.ends_with("node"), "{}", command.program);
        assert_eq!(command.args, vec!["server.js", "--port", "3000"]);
    }

    /// The defect this resolution exists for. On Windows `npm` is `npm.cmd`,
    /// and `CreateProcess` only ever appends `.exe` to a bare name — so
    /// `Command::new("npm")` fails to find an npm that is plainly installed,
    /// and every Node project started with `npm start` reported a program that
    /// was right there as missing.
    #[test]
    fn a_program_that_is_only_a_script_on_this_machine_is_still_found() {
        let toolchain = found();
        let windows = FakeMachine {
            installed: vec!["node.exe", "npm.cmd", "pnpm.cmd"],
        };

        let command = start_command(inputs("NODEJS", "npm run start", &toolchain, &windows))
            .expect("command");
        assert!(
            command.program.ends_with("npm.cmd"),
            "the bare name was not resolved: {}",
            command.program
        );
        assert_eq!(command.args, vec!["run", "start"]);
    }

    /// A machine can have Node and not have pnpm. Saying "NODEJS is not
    /// installed" would send the user to fix something that is not broken, so
    /// the missing program is named instead.
    #[test]
    fn a_missing_program_is_named_rather_than_blamed_on_the_language() {
        let toolchain = found();
        let without_pnpm = FakeMachine {
            installed: vec!["node", "npm"],
        };

        match start_command(inputs("NODEJS", "pnpm dev", &toolchain, &without_pnpm)) {
            Err(CommandError::ProgramNotFound { program, command }) => {
                assert_eq!(program, "pnpm");
                assert_eq!(command, "pnpm dev");
            }
            other => panic!("expected ProgramNotFound, got {other:?}"),
        }
    }

    /// A path the user wrote is what the user meant. Resolving it against
    /// `PATH` would silently run a different program with the same file name —
    /// so a machine with nothing installed still gets the path it was given,
    /// rather than `ProgramNotFound`.
    #[test]
    fn a_program_written_as_a_path_is_not_looked_up_on_path() {
        let toolchain = found();
        let nothing = FakeMachine { installed: vec![] };

        let command = start_command(inputs(
            "GENERIC",
            "./bin/server --config prod.toml",
            &toolchain,
            &nothing,
        ))
        .expect("command");
        assert!(
            command.program.ends_with(r"bin\server") || command.program.ends_with("bin/server"),
            "not the path that was written: {}",
            command.program
        );
        assert_eq!(command.args, vec!["--config", "prod.toml"]);
    }

    /// A relative program is anchored to the project, not to wherever this
    /// application happens to have been started from.
    ///
    /// Windows makes this load-bearing rather than tidy: `CreateProcess`
    /// resolves a relative program against the *calling* process's working
    /// directory and ignores the child's. `./server`, which is what a Go
    /// project's build produces and its start command names, would be looked
    /// for beside the application — where it is not.
    #[test]
    fn a_relative_program_is_anchored_to_the_project_directory() {
        let toolchain = found();
        let nothing = FakeMachine { installed: vec![] };

        let command =
            start_command(inputs("GO", "./server", &toolchain, &nothing)).expect("command");

        // Compared as a path rather than as text: the separator the join uses
        // is the platform's, and asserting on the string would only pass on
        // whichever platform wrote the assertion.
        assert_eq!(
            std::path::Path::new(&command.program),
            std::path::Path::new("/projects/demo").join("server"),
            "not anchored to the project: {}",
            command.program
        );
    }

    /// An absolute path is already an answer, and joining it to the project
    /// would corrupt it.
    #[test]
    fn an_absolute_program_is_passed_through_untouched() {
        let toolchain = found();
        let nothing = FakeMachine { installed: vec![] };

        let written = if cfg!(windows) {
            "C:/tools/server"
        } else {
            "/usr/local/bin/server"
        };
        let command =
            start_command(inputs("GENERIC", written, &toolchain, &nothing)).expect("command");
        assert_eq!(command.program, written);
    }

    /// Without a container there is no port mapping, so the process has to be
    /// told which port it was given.
    #[test]
    fn the_allocated_port_is_passed_through_the_environment() {
        let toolchain = found();
        let machine = anything();
        let command = start_command(CommandInputs {
            port: Some(8081),
            ..inputs("NODEJS", "npm start", &toolchain, &machine)
        })
        .expect("command");

        assert_eq!(command.env.get("PORT").map(String::as_str), Some("8081"));
    }

    #[test]
    fn the_projects_own_environment_is_kept_and_port_does_not_erase_it() {
        let toolchain = found();
        let machine = anything();
        let mut env = BTreeMap::new();
        env.insert("TOKEN".to_string(), "secret".to_string());

        let command = start_command(CommandInputs {
            env,
            port: Some(9000),
            ..inputs("NODEJS", "npm start", &toolchain, &machine)
        })
        .expect("command");

        assert_eq!(command.env.get("TOKEN").map(String::as_str), Some("secret"));
        assert_eq!(command.env.get("PORT").map(String::as_str), Some("9000"));
    }

    /// A directory with a space in it is the ordinary case on Windows —
    /// `C:\Users\Someone\My Projects`. Splitting on whitespace alone turns one
    /// argument into two and the project starts against the wrong path.
    #[test]
    fn quoted_arguments_survive_splitting() {
        let toolchain = found();
        let machine = anything();
        let command = start_command(inputs(
            "PYTHON",
            "python -m http.server --directory \"My Projects/site\"",
            &toolchain,
            &machine,
        ))
        .expect("command");

        assert_eq!(
            command.args.last().map(String::as_str),
            Some("My Projects/site")
        );
        assert_eq!(command.args.len(), 4);
    }

    #[test]
    fn single_quotes_work_the_same_way() {
        let toolchain = found();
        let machine = anything();
        let command = start_command(inputs(
            "NODEJS",
            "node -e 'console.log(1)'",
            &toolchain,
            &machine,
        ))
        .expect("command");

        assert_eq!(command.args, vec!["-e", "console.log(1)"]);
    }

    #[test]
    fn an_unclosed_quote_is_refused_rather_than_guessed_at() {
        let toolchain = found();
        let machine = anything();
        assert!(matches!(
            start_command(inputs("NODEJS", "node \"server.js", &toolchain, &machine)),
            Err(CommandError::Unparsable(_))
        ));
    }

    /// The refusal names the runtime and what was looked for, because the user
    /// has to know what to install.
    #[test]
    fn a_missing_toolchain_refuses_to_build_a_command() {
        let missing = Toolchain::Missing {
            looked_for: vec!["go".to_string()],
        };
        let machine = anything();

        match start_command(inputs("GO", "go run .", &missing, &machine)) {
            Err(CommandError::ToolchainMissing {
                runtime,
                looked_for,
            }) => {
                assert_eq!(runtime, "GO");
                assert_eq!(looked_for, vec!["go"]);
            }
            other => panic!("expected ToolchainMissing, got {other:?}"),
        }
    }

    #[test]
    fn the_refusal_reads_as_a_sentence() {
        let missing = Toolchain::Missing {
            looked_for: vec!["python3".to_string(), "python".to_string()],
        };
        let machine = anything();
        let error = start_command(inputs("PYTHON", "python app.py", &missing, &machine))
            .expect_err("must be refused");

        assert_eq!(
            error.to_string(),
            "PYTHON is not installed on this machine; looked for python3, python"
        );
    }

    #[test]
    fn an_empty_start_command_is_refused() {
        let toolchain = found();
        let machine = anything();
        assert_eq!(
            start_command(inputs("NODEJS", "   ", &toolchain, &machine)),
            Err(CommandError::NoStartCommand)
        );
    }

    #[test]
    fn extra_whitespace_between_arguments_is_not_an_argument() {
        let toolchain = found();
        let machine = anything();
        let command = start_command(inputs(
            "NODEJS",
            "  node    server.js  ",
            &toolchain,
            &machine,
        ))
        .expect("c");

        assert!(command.program.ends_with("node"));
        assert_eq!(command.args, vec!["server.js"]);
    }
}
