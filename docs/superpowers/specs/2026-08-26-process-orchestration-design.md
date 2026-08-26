# Multi-process execution without Docker

## Why

Panel Platform stopped requiring Docker in 0.1.15: `run_mode` defaults to
`HOST`, migration 0008 moved every row to it, and `crates/host-runner` starts
real processes. What did not change is the shape of a project. A project is one
`project_runtimes` row with one `start_command`, so a full-stack application — an
API and a web dev server, or anything with a worker — has no representation at
all.

Two problems follow from that, and this spec addresses both.

1. **A project cannot describe itself.** The single command is the whole model.
2. **Docker's vocabulary outlived Docker.** `lifecycle.rs` still falls back to
   `DockerRunner` for any non-`HOST` mode, `scaffold` writes a `Dockerfile` into
   every project including host ones, `working_dir` defaults to the container
   path `/app`, ports are called `container_port`, and the window still threads
   `dockerAvailable` through four components to gate a mode nothing produces.

Removing the second without the first would be a rename. Adding the first
without the second would leave a run path nobody takes and a daemon dependency
in a product that no longer has one.

## What this is not

Databases, import-in-place, the interactive terminal and public hosting are
designed in `2026-08-26-runtime-platform-roadmap.md` and are **out of scope
here**. Each depends on this spec landing first, and each gets its own spec.

## Decisions

Recorded because each closed off a real alternative.

**The process set lives in the database, edited in the application.** Not a
`panel.toml` committed to the user's repository, and not derived from a
`Procfile` or a compose file. Panel does not write into a user's project
directory, ports and environment variables already work exactly this way, and a
derived process set has no way to correct a wrong guess.

**Docker is excised, not deprecated.** The crate, the runner, the scaffolding,
the container columns, the `RunMode` enum and every frontend gate are deleted.
Leaving `docker-manager` compiled but uncalled would reproduce this repository's
most common defect — code that is written, tested, and never reached — in the
one subsystem this spec exists to simplify.

**A terminally failed process takes the project down.** Siblings stop in reverse
order, the project is `CRASHED` or `FAILED`, and the reason names the process.
The alternative, a `DEGRADED` project that is partly up, makes "running" mean
something the Run button cannot promise. No per-process `required` flag: one
rule, no configuration.

**Start is strictly sequential.** Process _n+1_ spawns after _n_ settles, or
after _n_ reports healthy when it has a health check. Concurrent start is a race
the user would have to win by luck.

## Data model

Migration `0009_processes.sql`. Three tables are rebuilt by copy, drop, rename
with an explicit column list, as 0003, 0004 and 0008 did, because SQLite cannot
alter a `CHECK` constraint or a column default.

### `project_processes` — new

    id              TEXT PRIMARY KEY
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE
    name            TEXT NOT NULL
    start_order     INTEGER NOT NULL
    command         TEXT NOT NULL
    working_dir     TEXT NOT NULL DEFAULT '.'
    install_command TEXT
    build_command   TEXT
    health_check_type   TEXT NOT NULL DEFAULT 'NONE'
        CHECK (health_check_type IN ('NONE','HTTP','TCP','COMMAND'))
    health_check_target TEXT
    health_interval_s     INTEGER NOT NULL DEFAULT 30
    health_timeout_s      INTEGER NOT NULL DEFAULT 5
    health_retries        INTEGER NOT NULL DEFAULT 3
    health_start_period_s INTEGER NOT NULL DEFAULT 20

    status          TEXT NOT NULL DEFAULT 'STOPPED'
        CHECK (status IN ('STOPPED','STARTING','RUNNING','RESTARTING',
                          'STOPPING','CRASHED','FAILED'))
    pid             INTEGER
    exit_code       INTEGER
    failure_reason  TEXT
    started_at      TEXT
    restart_count   INTEGER NOT NULL DEFAULT 0

    UNIQUE (project_id, name)
    UNIQUE (project_id, start_order)

`working_dir` is relative to `projects.directory`. The six columns below the
blank line are **observed**, never intended — the same rule `projects.status`
follows, and the reason `desired_state` is a separate column.

### `project_runtimes` — narrowed

Keeps what identifies the project's toolchain: `runtime`, `runtime_version`,
`package_manager`, `template_id`, `entry_file`, `publish_dir`.

Loses `start_command`, `install_command`, `build_command`, `working_dir` and the
six `health_*` columns. Those are per-process now.

The migration seeds one `project_processes` row per existing project, named
`main`, from the columns being removed, with `start_order = 0` and
`working_dir = '.'`. Every project that runs today runs unchanged afterwards.

### `project_ports` — renamed and linked

`container_port` becomes `port`. A nullable
`process_id TEXT REFERENCES project_processes(id) ON DELETE CASCADE` is added,
so the question "which process listens on 5173" has an answer. The
`UNIQUE (host_port, protocol, bind_address)` constraint is preserved exactly.

### `projects` — de-containerised

Drops `container_id`, `container_name`, `image_tag`, `network_name`,
`volume_name` and `run_mode`. `status` keeps every value it has, `CRASHED`
included.

## Orchestration

### `host-runner/src/orchestration.rs` — pure

A state machine. Input: the process rows, the current per-process state, and one
event (`Settled`, `Healthy`, `Exited { code }`, `StopRequested`,
`PrepareFailed`). Output: an ordered list of actions (`Spawn`,
`StopAll { reason }`, `WriteStatus`).

No tokio, no filesystem, no database, no clock. The ordering rule, the crash
rule and status aggregation all live here, which is what makes them testable
without spawning anything.

Aggregation:

| Every process                                            | Project      |
| -------------------------------------------------------- | ------------ |
| all running                                              | `RUNNING`    |
| any still coming up, none failed                         | `STARTING`   |
| one exited after having run, still within `MAX_RESTARTS` | `RESTARTING` |
| one exited after having run, past `MAX_RESTARTS`         | `CRASHED`    |
| one never started at all                                 | `FAILED`     |
| all stopped                                              | `STOPPED`    |

The first matching row wins, read top to bottom, so a project with one process
restarting and another running is `RESTARTING` rather than `RUNNING`. Siblings
are **not** stopped while a process is merely restarting — only when it exhausts
`MAX_RESTARTS` and becomes terminal.

Whether a process is restarted at all is not a new per-process field: it is read
from the project's existing `restart_policy` column and passed to the
supervisor's `restart_on_crash`. One project, one policy.

### `app-core/src/orchestrator.rs` — the driver

Owns one `SupervisorHandle` per process, turns real events into state-machine
input, executes the actions, writes the rows. Replaces the single-process
assumption in `runner/host.rs`.

Nothing inside `supervisor.rs` changes. Its settle window, output pump, health
polling, restart backoff and `STABLE_RUN` reset already do the right thing for
one process, and they now do it for each.

## Starting a project

1. **Resolve toolchains** through the existing `toolchain_flow`. A missing
   runtime is a blocker naming it, with an install offer — never a spawn that
   fails on "command not found".
2. **Allocate ports** through `project-manager/src/ports.rs`, whose bind test
   catches a port held by something outside Panel that the database cannot know
   about. Each allocated port is injected into its own process as `PORT`.
3. **Prepare.** The `(working_dir, install_command)` pairs, **deduplicated**,
   then `build_command` per process in `start_order`. Status is `BUILDING`,
   output is streamed to the project log. A non-zero exit stops the start here
   and reports the command's own output.
4. **Start** in `start_order`, each process gated on the previous settling or
   reporting healthy.
5. **Write what was observed** at every transition.

Deduplicating install is what makes per-process install commands correct in both
directions: a single-repository full-stack project declaring the same root
`npm install` on two processes runs it once, and a monorepo declaring two
workspace installs runs both.

Stopping reverses the order, so a process others were started against is the
last to go.

## Failures

| Case                   | Where it is caught                 | What the user sees                   |
| ---------------------- | ---------------------------------- | ------------------------------------ |
| Port already held      | Allocation, before any spawn       | The port, and an offer to reallocate |
| Missing runtime        | Toolchain resolution               | The runtime, and an install offer    |
| Install or build fails | Prepare                            | The failing command and its output   |
| Immediate exit         | Supervisor settle window           | The child's own stderr               |
| Crash loop             | `MAX_RESTARTS`, `STABLE_RUN` reset | Which process, and its last output   |
| Stuck on stop          | Grace period, then escalation      | Which process refused to exit        |

`kill_project` iterates processes in reverse order and escalates past the grace
period using `crates/platform`'s process-group termination.

With Docker gone, the exemption in `lifecycle.rs` that deliberately left
containers running at shutdown disappears. Nothing outlives the application any
more.

## Surface

`ProjectDetail` gains a process list: name, status, port, per-process log view,
per-process restart. Run, Stop and Restart continue to act on the project as a
whole.

Deleted: `crates/docker-manager`, `app-core/src/images.rs`,
`app-core/src/runner/docker.rs`, `app-core/src/runner.rs`'s `ProjectRunner`
trait (with one runner there is nothing to abstract over), `docker/templates/`,
`DockerStatus` from `SystemStatus`, `ErrorCode::DockerUnavailable` and
`ErrorCode::DockerOperationFailed`, `RunMode`, and `dockerAvailable` throughout
`api.ts`, `App.tsx`, `Projects.tsx`, `ProjectDetail.tsx`, `lib/projects.ts` and
`lib/projects.test.ts`. `runControls` loses its only blocking reason and is
reduced to the busy check. `bollard` leaves the lockfile.

`runtime_plan.rs`'s `container_port` field is renamed `port`, and its defaults
table stops being sourced from `docker/templates/`.

## Verification

Chosen deliberately, and the limitation is stated rather than implied:
**unit tests plus a manual checklist**, not committed fixture projects with real
spawn tests.

The orchestration state machine is pure precisely so this choice costs less than
it otherwise would. Ordering, the crash rule, aggregation, reverse-order stop
and prepare deduplication are all unit tested at that layer. The migration is
tested against a database seeded with rows in the pre-0009 shape.

What unit tests do **not** cover, and what the manual checklist must therefore
exercise on a real machine: an actual multi-process start, a real port conflict,
a real missing runtime, a real crash loop, a real stuck process, and three
projects running at once.

The checklist is `docs/superpowers/checklists/2026-08-26-process-orchestration.md`.

**As implemented, none of its twelve scenarios has been run.** Every claim in
this document about what happens when a process actually starts, crashes,
refuses to stop, or takes its siblings down with it is therefore unverified on
any machine. What _has_ run: the whole Rust suite and the window's, both clean,
including the state machine's rules, the prepare deduplication, and migration
0009 against a database seeded with rows in the pre-0009 shape.

`cargo clippy --workspace --all-targets` and `cargo test --workspace` are clean
and stay clean; the window has `npx tsc --noEmit`,
`npx eslint src --max-warnings 0` and `npx vitest run`.

Per this repository's convention, every module this spec adds states what has
actually run on the development machine and what has only compiled. The Unix
branch of process-group termination remains unverified: there is no Linux or
macOS here.
