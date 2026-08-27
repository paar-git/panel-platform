# Multi-Process Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give a project more than one process, and delete Docker from the codebase entirely.

**Architecture:** A new pure state machine in `host-runner` decides ordering, the crash rule and status aggregation with no I/O; a new driver in `app-core` owns one supervisor handle per process and executes what the state machine decides. Migration 0009 moves per-process execution fields out of `project_runtimes` into a new `project_processes` table and drops every container column. The Docker runner, crate, templates and frontend gating are deleted rather than deprecated.

**Tech Stack:** Rust 2021 (workspace, `unsafe_code = "forbid"`), sqlx + SQLite, tokio, Tauri 2, React 19 + TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-08-26-process-orchestration-design.md`

## Global Constraints

- Workspace lints are `deny` for `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`. Test modules opt out with the existing `#![cfg_attr(test, allow(...))]` block — copy it from `crates/host-runner/src/lib.rs`.
- `unsafe_code = "forbid"` is workspace-wide and cannot be downgraded.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` must be clean after every task.
- Frontend gates: `npx tsc --noEmit`, `npx eslint src --max-warnings 0`, `npx vitest run` from `apps/desktop`.
- `pnpm contracts:check` must pass after any change to `crates/api-types` — contracts are generated, not hand-edited.
- Status is written from what was **observed**, never from what was intended. This is why `status` and `desired_state` are separate columns.
- Every new module states in its header what has actually run on this machine and what has only compiled. This machine is Windows 11 with no Docker, no WSL and no Linux.
- SQLite cannot alter a `CHECK` constraint or a column default. Table changes are done by copy → drop → rename with the column list **written out**, never `SELECT *`. Follow `migrations/0008_local_runtime.sql` as the model.
- Commit after every task. Never push.

---

### Task 1: Migration 0009 — schema

**Files:**

- Create: `crates/database/migrations/0009_processes.sql`
- Modify: `crates/database/src/lib.rs` (add the `include_str!` const beside `LOCAL_RUNTIME_MIGRATION` at line 70, and register it in the migration list)
- Test: `crates/database/src/schema_parity.rs` (test module at the bottom)

**Interfaces:**

- Consumes: nothing.
- Produces: tables `project_processes` (new), `project_runtimes` (narrowed), `project_ports` (`container_port` → `port`, plus `process_id`), `projects` (container columns and `run_mode` dropped). Const `PROCESSES_MIGRATION: &str`.

- [ ] **Step 1: Write the failing test**

Add to the test module in `crates/database/src/schema_parity.rs`:

```rust
#[tokio::test]
async fn migration_0009_seeds_one_process_per_existing_runtime() {
    let db = Database::open_in_memory().await.expect("open");
    db.migrate().await.expect("migrate");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_processes")
        .fetch_one(db.pool())
        .await
        .expect("count");
    assert_eq!(count, 0, "a fresh database has no projects and so no processes");

    let body = table_body(crate::PROCESSES_MIGRATION, "project_processes")
        .expect("project_processes is created by 0009");
    let statuses = check_values(body, "status").expect("status has a CHECK");
    assert_eq!(
        statuses,
        vec![
            "STOPPED", "STARTING", "RUNNING", "RESTARTING",
            "STOPPING", "CRASHED", "FAILED",
        ]
    );
}

#[tokio::test]
async fn migration_0009_drops_every_container_column() {
    let db = Database::open_in_memory().await.expect("open");
    db.migrate().await.expect("migrate");

    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('projects')")
        .fetch_all(db.pool())
        .await
        .expect("columns");

    for gone in [
        "container_id", "container_name", "image_tag",
        "network_name", "volume_name", "run_mode",
    ] {
        assert!(!columns.iter().any(|c| c == gone), "{gone} should be dropped");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p project-host-database migration_0009 -- --nocapture`
Expected: FAIL — `PROCESSES_MIGRATION` is not defined.

- [ ] **Step 3: Write the migration**

Create `crates/database/migrations/0009_processes.sql`. Open it with a comment block in the house style explaining _why_ — see 0008's header for the register to match. Then:

```sql
CREATE TABLE project_processes (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    start_order     INTEGER NOT NULL,
    command         TEXT NOT NULL,
    working_dir     TEXT NOT NULL DEFAULT '.',
    install_command TEXT,
    build_command   TEXT,

    health_check_type     TEXT NOT NULL DEFAULT 'NONE' CHECK (health_check_type IN (
        'NONE','HTTP','TCP','COMMAND')),
    health_check_target   TEXT,
    health_interval_s     INTEGER NOT NULL DEFAULT 30,
    health_timeout_s      INTEGER NOT NULL DEFAULT 5,
    health_retries        INTEGER NOT NULL DEFAULT 3,
    health_start_period_s INTEGER NOT NULL DEFAULT 20,

    status          TEXT NOT NULL DEFAULT 'STOPPED' CHECK (status IN (
        'STOPPED','STARTING','RUNNING','RESTARTING','STOPPING','CRASHED','FAILED')),
    pid             INTEGER,
    exit_code       INTEGER,
    failure_reason  TEXT,
    started_at      TEXT,
    restart_count   INTEGER NOT NULL DEFAULT 0,

    UNIQUE (project_id, name),
    UNIQUE (project_id, start_order),
    CHECK (name GLOB '[a-z0-9][a-z0-9-]*'),
    CHECK (start_order >= 0)
);

CREATE INDEX idx_processes_project ON project_processes (project_id);

-- One process per existing project, named `main`, carrying the columns that
-- are about to leave `project_runtimes`. Every project that runs today runs
-- unchanged after this migration.
--
-- `working_dir` is not copied. The old default was `/app`, a path inside a
-- container that does not exist on this machine; the new value is relative to
-- `projects.directory`, and `.` is what every existing project meant.
INSERT INTO project_processes (
    id, project_id, name, start_order, command, working_dir,
    install_command, build_command,
    health_check_type, health_check_target,
    health_interval_s, health_timeout_s, health_retries, health_start_period_s
)
SELECT
    lower(hex(randomblob(16))), project_id, 'main', 0, start_command, '.',
    install_command, build_command,
    health_check_type, health_check_target,
    health_interval_s, health_timeout_s, health_retries, health_start_period_s
FROM project_runtimes;
```

Then rebuild `project_runtimes` without the moved columns, rebuild `project_ports` renaming `container_port` to `port` and adding `process_id TEXT REFERENCES project_processes(id) ON DELETE CASCADE`, and rebuild `projects` without `container_id`, `container_name`, `image_tag`, `network_name`, `volume_name`, `run_mode`. Copy the `projects` column list and every `CHECK` from 0008 verbatim minus those six columns, and recreate `idx_projects_status` and `idx_projects_desired` afterwards. Preserve `project_ports`' `UNIQUE (host_port, protocol, bind_address)` exactly — it is what makes double allocation a database error rather than a race.

Add to `crates/database/src/lib.rs` beside the others:

```rust
/// Multi-process projects, and the removal of every container column.
pub const PROCESSES_MIGRATION: &str = include_str!("../migrations/0009_processes.sql");
```

and append it to the migration list `migrate()` runs.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p project-host-database`
Expected: PASS. Existing parity tests that assert on `run_mode` will now fail — delete those assertions; the column is gone by design.

- [ ] **Step 5: Commit**

```bash
git add crates/database/migrations/0009_processes.sql crates/database/src/lib.rs crates/database/src/schema_parity.rs
git commit -m "Add project_processes and drop the container columns"
```

---

### Task 2: Process records and queries

**Files:**

- Modify: `crates/database/src/projects.rs`
- Test: same file, existing `#[cfg(test)] mod tests`

**Interfaces:**

- Consumes: Task 1's tables.
- Produces:
  - `pub struct ProcessRecord { id, project_id, name: String, start_order: i64, command: String, working_dir: String, install_command: Option<String>, build_command: Option<String>, health_check_type: String, health_check_target: Option<String>, health_interval_s: i64, health_timeout_s: i64, health_retries: i64, health_start_period_s: i64, status: String, pid: Option<i64>, exit_code: Option<i64>, failure_reason: Option<String>, started_at: Option<String>, restart_count: i64 }`
  - `pub struct NewProcess { name, start_order, command, working_dir, install_command, build_command, health_check_type, health_check_target, health_interval_s, health_timeout_s, health_retries, health_start_period_s }`
  - `pub async fn list_processes(db: &Database, project_id: &str) -> Result<Vec<ProcessRecord>>` — ordered by `start_order` ascending.
  - `pub async fn replace_processes(db: &Database, project_id: &str, processes: &[NewProcess]) -> Result<Vec<ProcessRecord>>` — one transaction, deletes then inserts.
  - `pub async fn set_process_status(db: &Database, process_id: &str, status: &str, pid: Option<i64>, exit_code: Option<i64>, failure_reason: Option<&str>) -> Result<()>`
  - `pub async fn increment_process_restart_count(db: &Database, process_id: &str) -> Result<i64>`
- Also: `RuntimeRecord` and `RuntimeSpec` lose `start_command`, `install_command`, `build_command`, `working_dir` and the six `health_*` fields. `PortRecord::container_port` becomes `PortRecord::port`, and `PortRecord` gains `process_id: Option<String>`. `NewPort::container_port` becomes `NewPort::port`, plus `process_id: Option<String>`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn processes_come_back_in_start_order() {
    let db = fixture().await;
    let project = seed_project(&db, "ordered").await;

    replace_processes(&db, &project.id, &[
        NewProcess::simple("web", 1, "npm run dev"),
        NewProcess::simple("api", 0, "npm run api"),
    ])
    .await
    .expect("replace");

    let names: Vec<String> = list_processes(&db, &project.id)
        .await
        .expect("list")
        .into_iter()
        .map(|p| p.name)
        .collect();

    assert_eq!(names, vec!["api", "web"], "start_order decides, not insert order");
}

#[tokio::test]
async fn replacing_processes_removes_the_previous_set() {
    let db = fixture().await;
    let project = seed_project(&db, "replaced").await;

    replace_processes(&db, &project.id, &[NewProcess::simple("main", 0, "node a.js")])
        .await
        .expect("first");
    replace_processes(&db, &project.id, &[NewProcess::simple("only", 0, "node b.js")])
        .await
        .expect("second");

    let all = list_processes(&db, &project.id).await.expect("list");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "only");
}

#[tokio::test]
async fn a_process_status_is_written_and_read_back() {
    let db = fixture().await;
    let project = seed_project(&db, "status").await;
    let made = replace_processes(&db, &project.id, &[NewProcess::simple("main", 0, "node a.js")])
        .await
        .expect("replace");

    set_process_status(&db, &made[0].id, "CRASHED", None, Some(1), Some("exited immediately"))
        .await
        .expect("set");

    let back = list_processes(&db, &project.id).await.expect("list");
    assert_eq!(back[0].status, "CRASHED");
    assert_eq!(back[0].exit_code, Some(1));
    assert_eq!(back[0].failure_reason.as_deref(), Some("exited immediately"));
}
```

`NewProcess::simple(name, start_order, command)` is a test constructor filling the rest with defaults — add it under `#[cfg(test)]`. Reuse whatever `fixture()`/`seed_project()` helpers the existing test module already has; if it does not have them, write them the way the existing tests build a database.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p project-host-database processes`
Expected: FAIL — `list_processes`, `replace_processes`, `NewProcess` not defined.

- [ ] **Step 3: Implement**

Add `ProcessRecord`, `NewProcess`, a `process_from_row` helper matching the existing `project_from_row` style at line 190, and the four query functions. `replace_processes` takes a transaction, `DELETE FROM project_processes WHERE project_id = ?`, then inserts each with a fresh id, then commits and returns `list_processes`.

Then narrow `RuntimeRecord`/`RuntimeSpec` and rename the port fields as listed under Interfaces. This will break `find_runtime`'s `SELECT`, `list_ports`, `create_project`, `set_primary_host_port`, `record_container` and `set_run_mode`. Delete `record_container` and `set_run_mode` outright — the columns they write are gone. Fix the rest by following the compiler.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p project-host-database`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/database/src/projects.rs
git commit -m "Read and write a project's process rows"
```

---

### Task 3: The orchestration state machine — ordering

**Files:**

- Create: `crates/host-runner/src/orchestration.rs`
- Modify: `crates/host-runner/src/lib.rs` (declare `pub mod orchestration;` and re-export)
- Test: inline `#[cfg(test)] mod tests` in the new file

**Interfaces:**

- Consumes: nothing. This module depends on no other crate and does no I/O — no tokio, no filesystem, no database, no clock. That is what makes it testable.
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState { Pending, Starting, Running, Restarting, Stopping, Stopped, Crashed, Failed }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Settled { index: usize },
    Healthy { index: usize },
    Exited { index: usize, code: Option<i32>, terminal: bool },
    StopRequested,
    PrepareFailed { index: usize, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Spawn { index: usize },
    Stop { index: usize },
    WriteProcessStatus { index: usize, state: ProcessState, reason: Option<String> },
    WriteProjectStatus { status: ProjectStatus },
}

/// One process as the machine sees it: only what a decision depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process { pub has_health_check: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectStatus(pub &'static str);

#[derive(Debug, Clone)]
pub struct Machine { /* private */ }

impl Machine {
    pub fn new(processes: Vec<Process>) -> Self;
    /// Begin a start. Returns the actions that open it.
    pub fn start(&mut self) -> Vec<Action>;
    pub fn handle(&mut self, event: Event) -> Vec<Action>;
    pub fn state(&self, index: usize) -> Option<ProcessState>;
    pub fn project_status(&self) -> ProjectStatus;
}
```

`ProjectStatus` wraps the wire string (`"RUNNING"`, `"STARTING"`, `"RESTARTING"`, `"CRASHED"`, `"FAILED"`, `"STOPPED"`) rather than importing `api-types`: this crate deliberately depends on neither the wire format nor the database, for the same reason `detection` does not.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_start_spawns_only_the_first_process() {
    let mut machine = Machine::new(vec![
        Process { has_health_check: false },
        Process { has_health_check: false },
        Process { has_health_check: false },
    ]);

    let actions = machine.start();

    assert!(actions.contains(&Action::Spawn { index: 0 }));
    assert!(!actions.contains(&Action::Spawn { index: 1 }),
        "process 1 waits for process 0 to settle");
    assert_eq!(machine.state(1), Some(ProcessState::Pending));
}

#[test]
fn settling_advances_to_the_next_process() {
    let mut machine = Machine::new(vec![
        Process { has_health_check: false },
        Process { has_health_check: false },
    ]);
    machine.start();

    let actions = machine.handle(Event::Settled { index: 0 });

    assert_eq!(machine.state(0), Some(ProcessState::Running));
    assert!(actions.contains(&Action::Spawn { index: 1 }));
}

#[test]
fn a_process_with_a_health_check_gates_on_health_not_on_settling() {
    let mut machine = Machine::new(vec![
        Process { has_health_check: true },
        Process { has_health_check: false },
    ]);
    machine.start();

    let after_settle = machine.handle(Event::Settled { index: 0 });
    assert!(!after_settle.contains(&Action::Spawn { index: 1 }),
        "settling is not enough when a health check exists");

    let after_health = machine.handle(Event::Healthy { index: 0 });
    assert!(after_health.contains(&Action::Spawn { index: 1 }));
}

#[test]
fn the_last_process_running_makes_the_project_running() {
    let mut machine = Machine::new(vec![
        Process { has_health_check: false },
        Process { has_health_check: false },
    ]);
    machine.start();
    machine.handle(Event::Settled { index: 0 });
    let actions = machine.handle(Event::Settled { index: 1 });

    assert_eq!(machine.project_status(), ProjectStatus("RUNNING"));
    assert!(actions.contains(&Action::WriteProjectStatus {
        status: ProjectStatus("RUNNING")
    }));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p project-host-host-runner orchestration`
Expected: FAIL — module `orchestration` does not exist.

- [ ] **Step 3: Implement**

Write the module with a header in the house style saying what it is and why it is pure. `Machine` holds `processes: Vec<Process>` and `states: Vec<ProcessState>`. `start()` sets every state to `Pending`, sets index 0 to `Starting`, returns `[WriteProcessStatus{0, Starting}, Spawn{0}, WriteProjectStatus{STARTING}]`. `handle` matches on the event; `Settled` on a process with no health check, and `Healthy` on one with, both mark it `Running` and spawn the next `Pending` index if there is one. Leave `Exited`, `StopRequested` and `PrepareFailed` returning an empty `Vec` for now — Task 4 implements them.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p project-host-host-runner orchestration`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/host-runner/src/orchestration.rs crates/host-runner/src/lib.rs
git commit -m "Decide process start ordering without running anything"
```

---

### Task 4: The state machine — crashes, aggregation, reverse stop

**Files:**

- Modify: `crates/host-runner/src/orchestration.rs`
- Test: same file

**Interfaces:**

- Consumes: Task 3's `Machine`, `Event`, `Action`, `ProcessState`, `ProjectStatus`.
- Produces: no new types. `handle` now implements `Exited`, `StopRequested` and `PrepareFailed`.

The rules, from the spec:

- `Exited { terminal: false }` — the supervisor will retry. State becomes `Restarting`. **Siblings are not touched.** Project becomes `RESTARTING`.
- `Exited { terminal: true }` on a process that had reached `Running` — state `Crashed`, every other non-stopped process gets a `Stop` action **in reverse index order**, project becomes `CRASHED`.
- `Exited { terminal: true }` on a process that never reached `Running` — state `Failed`, same reverse stop, project becomes `FAILED`.
- `PrepareFailed` — state `Failed`, reverse stop, project `FAILED`.
- `StopRequested` — every non-stopped process gets a `Stop` in reverse index order, project `STOPPED` once all are stopped.

Aggregation is read top to bottom, first match wins: any `Failed` → `FAILED`; any `Crashed` → `CRASHED`; any `Restarting` → `RESTARTING`; any `Pending`/`Starting`/`Stopping` → `STARTING`; all `Running` → `RUNNING`; otherwise `STOPPED`.

- [ ] **Step 1: Write the failing test**

```rust
fn three_running() -> Machine {
    let mut machine = Machine::new(vec![
        Process { has_health_check: false },
        Process { has_health_check: false },
        Process { has_health_check: false },
    ]);
    machine.start();
    machine.handle(Event::Settled { index: 0 });
    machine.handle(Event::Settled { index: 1 });
    machine.handle(Event::Settled { index: 2 });
    machine
}

#[test]
fn a_restart_leaves_siblings_alone() {
    let mut machine = three_running();

    let actions = machine.handle(Event::Exited { index: 1, code: Some(1), terminal: false });

    assert_eq!(machine.state(1), Some(ProcessState::Restarting));
    assert_eq!(machine.state(0), Some(ProcessState::Running));
    assert_eq!(machine.state(2), Some(ProcessState::Running));
    assert!(!actions.iter().any(|a| matches!(a, Action::Stop { .. })),
        "one blip must not tear the project down");
    assert_eq!(machine.project_status(), ProjectStatus("RESTARTING"));
}

#[test]
fn a_terminal_crash_stops_the_siblings_in_reverse_order() {
    let mut machine = three_running();

    let actions = machine.handle(Event::Exited { index: 0, code: Some(1), terminal: true });

    assert_eq!(machine.state(0), Some(ProcessState::Crashed));
    let stops: Vec<usize> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Stop { index } => Some(*index),
            _ => None,
        })
        .collect();
    assert_eq!(stops, vec![2, 1], "the process others were started against dies last");
    assert_eq!(machine.project_status(), ProjectStatus("CRASHED"));
}

#[test]
fn a_process_that_never_ran_fails_rather_than_crashes() {
    let mut machine = Machine::new(vec![
        Process { has_health_check: false },
        Process { has_health_check: false },
    ]);
    machine.start();

    machine.handle(Event::Exited { index: 0, code: Some(127), terminal: true });

    assert_eq!(machine.state(0), Some(ProcessState::Failed));
    assert_eq!(machine.project_status(), ProjectStatus("FAILED"),
        "never started and died after running are different situations");
}

#[test]
fn a_stop_request_stops_everything_in_reverse_order() {
    let mut machine = three_running();

    let actions = machine.handle(Event::StopRequested);

    let stops: Vec<usize> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Stop { index } => Some(*index),
            _ => None,
        })
        .collect();
    assert_eq!(stops, vec![2, 1, 0]);
}

#[test]
fn prepare_failing_fails_the_project_without_spawning_anything() {
    let mut machine = Machine::new(vec![Process { has_health_check: false }]);

    let actions = machine.handle(Event::PrepareFailed {
        index: 0,
        reason: "npm install exited 1".to_string(),
    });

    assert_eq!(machine.project_status(), ProjectStatus("FAILED"));
    assert!(!actions.contains(&Action::Spawn { index: 0 }));
    assert!(actions.contains(&Action::WriteProcessStatus {
        index: 0,
        state: ProcessState::Failed,
        reason: Some("npm install exited 1".to_string()),
    }));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p project-host-host-runner orchestration`
Expected: FAIL — the new events return empty action lists, so every assertion about `Stop` and status fails.

- [ ] **Step 3: Implement**

Fill in the three event arms and write `project_status()` as the top-to-bottom aggregation above. Extract the reverse-order stop into a private `fn stop_others(&mut self, except: Option<usize>) -> Vec<Action>` so the crash path and the stop path cannot drift apart.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p project-host-host-runner && cargo clippy -p project-host-host-runner --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/host-runner/src/orchestration.rs
git commit -m "Decide what a crash means when a project has several processes"
```

---

### Task 5: Prepare — deduplicated install and build

**Files:**

- Create: `crates/app-core/src/orchestrator/prepare.rs`
- Create: `crates/app-core/src/orchestrator.rs` (module declaration only at this task)
- Modify: `crates/app-core/src/lib.rs` (declare `pub mod orchestrator;`)
- Test: inline in `prepare.rs`

**Interfaces:**

- Consumes: `projects::ProcessRecord` (Task 2).
- Produces: `pub fn prepare_steps(processes: &[ProcessRecord]) -> Vec<PrepareStep>` where

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareStep {
    pub phase: &'static str,      // "install" or "build"
    pub working_dir: String,
    pub command: String,
    /// The index of the process this step is attributed to, for error reporting.
    pub process_index: usize,
}
```

Install steps come first, deduplicated by `(working_dir, command)`, in first-appearance order. Build steps follow, **not** deduplicated — a build is per-process output and running it once for two processes would leave one unbuilt. Empty and whitespace-only commands are skipped.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn one_root_install_declared_twice_runs_once() {
    let steps = prepare_steps(&[
        process("api", 0, ".", Some("npm install"), None),
        process("web", 1, ".", Some("npm install"), None),
    ]);

    let installs: Vec<&PrepareStep> = steps.iter().filter(|s| s.phase == "install").collect();
    assert_eq!(installs.len(), 1, "the same install in the same directory is one step");
}

#[test]
fn two_workspace_installs_both_run() {
    let steps = prepare_steps(&[
        process("api", 0, "packages/api", Some("npm install"), None),
        process("web", 1, "packages/web", Some("npm install"), None),
    ]);

    let installs: Vec<&PrepareStep> = steps.iter().filter(|s| s.phase == "install").collect();
    assert_eq!(installs.len(), 2, "same command, different directory, different work");
}

#[test]
fn every_install_precedes_every_build() {
    let steps = prepare_steps(&[
        process("api", 0, ".", Some("npm install"), Some("npm run build:api")),
        process("web", 1, ".", Some("npm install"), Some("npm run build:web")),
    ]);

    let phases: Vec<&str> = steps.iter().map(|s| s.phase).collect();
    assert_eq!(phases, vec!["install", "build", "build"]);
}

#[test]
fn builds_are_not_deduplicated() {
    let steps = prepare_steps(&[
        process("api", 0, ".", None, Some("npm run build")),
        process("web", 1, ".", None, Some("npm run build")),
    ]);

    assert_eq!(steps.len(), 2, "a build produces per-process output");
}

#[test]
fn a_blank_command_is_not_a_step() {
    let steps = prepare_steps(&[process("main", 0, ".", Some("   "), None)]);
    assert!(steps.is_empty());
}
```

`process(name, order, dir, install, build)` is a test constructor building a `ProcessRecord` with defaults — write it under `#[cfg(test)]`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p project-host-core prepare`
Expected: FAIL — `prepare_steps` not defined.

- [ ] **Step 3: Implement**

Pure function, no I/O. Use a `BTreeSet<(String, String)>` of seen `(working_dir, command)` pairs for the install pass.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p project-host-core prepare`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app-core/src/orchestrator.rs crates/app-core/src/orchestrator/prepare.rs crates/app-core/src/lib.rs
git commit -m "Work out which install and build steps a project actually needs"
```

---

### Task 6: The driver — replace HostRunner

**Files:**

- Modify: `crates/app-core/src/orchestrator.rs` (the driver itself)
- Delete: `crates/app-core/src/runner/host.rs` (its logic moves here)
- Modify: `crates/app-core/src/lifecycle.rs:97-104` (`runner_for` disappears — see Task 7)
- Test: inline in `orchestrator.rs`

**Interfaces:**

- Consumes: `orchestration::Machine` (Tasks 3–4), `prepare_steps` (Task 5), `projects::list_processes` / `set_process_status` (Task 2), and everything `runner/host.rs` already used: `resolve_toolchain`, `static_command`, `build_command`, `health_policy`, `translate`, `HostRegistry`.
- Produces:

```rust
pub struct Orchestrator { registry: ProcessRegistry, logs_root: PathBuf }

impl Orchestrator {
    pub fn new(registry: ProcessRegistry, logs_root: PathBuf) -> Self;
    pub async fn start(&self, ctx: StartContext<'_>) -> Result<Observed, LifecycleError>;
    pub async fn stop(&self, project: &ProjectRecord) -> Result<(), LifecycleError>;
    pub async fn kill(&self, project: &ProjectRecord) -> Result<(), LifecycleError>;
    pub async fn observe(&self, project: &ProjectRecord) -> Result<Option<Observed>, LifecycleError>;
    pub async fn restart_process(&self, project: &ProjectRecord, process_name: &str) -> Result<(), LifecycleError>;
}
```

`ProcessRegistry` replaces `HostRegistry`: the map becomes `BTreeMap<String, Vec<(String, SupervisorHandle)>>` — project id to an ordered list of `(process_id, handle)`. Keep `all()`, `running()` and `handle()`; `handle(project_id)` returns the **first** process's handle so the existing console command keeps working unchanged until Task 10 gives it a process argument.

Move `resolve_toolchain`, `static_command`, `build_command`, `health_policy`, `translate`, `primary_port` and `today` from `runner/host.rs` into `orchestrator.rs` verbatim — they are correct and this task is not the place to change them. `build_command` takes the per-process `working_dir` joined onto the project directory instead of the runtime's.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn starting_a_project_with_no_process_rows_is_an_error_that_says_so() {
    let app = test_state().await;
    let project = seed_project(&app, "empty").await;

    let error = orchestrator(&app)
        .start(start_context(&app, &project))
        .await
        .expect_err("a project with no processes cannot start");

    assert!(
        error.to_string().contains("no processes"),
        "got {error}"
    );
}

#[tokio::test]
async fn a_port_conflict_is_reported_before_anything_spawns() {
    let app = test_state().await;
    let project = seed_project_holding_a_taken_port(&app, "conflicted").await;

    let error = orchestrator(&app)
        .start(start_context(&app, &project))
        .await
        .expect_err("the port is held");

    assert!(matches!(error, LifecycleError::PortConflict(_)));
    let processes = projects::list_processes(app.db(), &project.id).await.expect("list");
    assert!(
        processes.iter().all(|p| p.status == "STOPPED"),
        "nothing should have been marked started"
    );
}
```

Build the helpers from the existing ones in `lifecycle.rs`'s test module (see its `AppState`-with-in-memory-database fixture around line 532) — that module already knows how to make a state without a daemon.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p project-host-core orchestrator`
Expected: FAIL — `Orchestrator` not defined.

- [ ] **Step 3: Implement**

`start` in this order, matching the spec's section 3:

1. `projects::list_processes`; empty is an error naming the project.
2. Resolve the toolchain once from the runtime row (unchanged logic).
3. Stop and deregister any handles already held for this project, then `crate::ports::check`. Keep the existing comment explaining why the order matters — a restart would otherwise find its own process holding the port.
4. `crate::env::resolve` once, reused for every step and process.
5. `prepare_steps(&processes)`, each run through `run_step`, project status `BUILDING`. On failure feed `Event::PrepareFailed` into the machine, write the resulting statuses, and return the error.
6. Build the `Machine` from the process rows and drive it: execute `Spawn` by building the command and calling `project_host_host_runner::start`, then await either the settle or, when the process has a health check, the health transition, feeding `Settled`/`Healthy` back in. Write every `WriteProcessStatus` and `WriteProjectStatus` the machine emits.

`stop` feeds `Event::StopRequested` and executes the `Stop` actions in the order given. `kill` does the same but calls `handle.kill()`. `restart_process` stops and respawns one named process without touching the others.

Static projects: the `is_static` branch applies per process — a process whose project runtime is `STATIC` uses `static_command`. Keep the `publish_dir` traversal check exactly as written; it is the security boundary and this task must not touch it.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p project-host-core && cargo clippy -p project-host-core --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app-core/src/orchestrator.rs crates/app-core/src/runner/host.rs
git commit -m "Run every one of a project's processes, in order"
```

---

### Task 7: Delete the Docker runner

**Files:**

- Delete: `crates/app-core/src/runner/docker.rs`, `crates/app-core/src/runner.rs`, `crates/app-core/src/images.rs`
- Modify: `crates/app-core/src/lifecycle.rs`, `crates/app-core/src/lib.rs`, `crates/app-core/src/state.rs`, `crates/app-core/src/health.rs`, `crates/app-core/src/reconcile.rs`, `crates/app-core/src/shutdown.rs`
- Delete: `docker/` (the whole directory)

**Interfaces:**

- Consumes: Task 6's `Orchestrator`.
- Produces: `lifecycle` calling `Orchestrator` directly. `StartContext` and `Observed` move from `runner.rs` into `lifecycle.rs`; the `ProjectRunner` trait is deleted — with one runner there is nothing to abstract over.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn scaffolding_writes_starter_files_but_never_a_dockerfile() {
    let directory = tempfile::tempdir().expect("tempdir");

    scaffold(directory.path(), "NODEJS").expect("scaffold");

    assert!(
        !directory.path().join("Dockerfile").exists(),
        "a Dockerfile is not something this product writes any more"
    );
    assert!(
        directory.path().join("index.js").exists(),
        "starter files are still written"
    );
}
```

`scaffold` loses its `ImageSpec` parameter and takes the runtime wire value instead; `starter_files` stays, `dockerfile_for` goes with `images.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p project-host-core scaffold`
Expected: FAIL to compile — `scaffold` still takes `&ImageSpec`.

- [ ] **Step 3: Implement**

Delete the three files and the `docker/` directory. In `lifecycle.rs`: delete `runner_for`, `spec_for`, `image_tag`, the Docker health-word translation, `LifecycleError::Docker`, and the shutdown exemption that deliberately left containers running — nothing outlives the application now, so `stop_all` stops everything. Point every call site at `Orchestrator`. Remove `project-host-docker-manager` from `crates/app-core/Cargo.toml`.

Rewrite the module header of `lifecycle.rs`: its numbered steps still describe writing a `Dockerfile` and recording what Docker said, and both are now false.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p project-host-core && cargo clippy -p project-host-core --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A crates/app-core docker
git commit -m "Delete the Docker runner and the Dockerfile scaffolding"
```

---

### Task 8: Delete docker-manager and the Docker wire types

**Files:**

- Delete: `crates/docker-manager/` (whole crate)
- Modify: `Cargo.toml` (remove the workspace member), `crates/api-types/src/dto.rs`, `crates/api-types/src/contract.rs`, `crates/api-types/src/errors.rs`, `crates/api-types/src/enums.rs`, `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src-tauri/Cargo.toml`
- Test: `crates/api-types` existing tests

**Interfaces:**

- Consumes: Task 7.
- Produces: `SystemStatus` without `docker_status`; `ErrorCode` without `DockerUnavailable` and `DockerOperationFailed`; no `RunMode` enum; no `set_project_run_mode` Tauri command.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn no_error_code_names_docker() {
    for code in ErrorCode::ALL {
        assert!(
            !code.as_str().contains("DOCKER"),
            "{} outlived the daemon",
            code.as_str()
        );
    }
}
```

Also delete `enums.rs:385`'s `assert_eq!(RunMode::Docker.as_str(), "DOCKER");` and `errors.rs:189`'s `DockerUnavailable.is_retryable()` assertion.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p project-host-api-types`
Expected: FAIL — `DOCKER_UNAVAILABLE` is still in the list.

- [ ] **Step 3: Implement**

Remove `DockerStatus` from `dto.rs` and from `contract.rs:29`; remove the two error codes; remove `RunMode` from `enums.rs`; delete the crate directory and its workspace member line; delete `set_project_run_mode` and `apply_run_mode` from `apps/desktop/src-tauri/src/lib.rs` and from the `invoke_handler` list; drop `bollard` and `project-host-docker-manager` from every `Cargo.toml` that names them.

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm contracts && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS. `pnpm contracts` regenerates the TypeScript contract — commit the regenerated file.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Remove the Docker crate and every wire type that named it"
```

---

### Task 9: Unthread dockerAvailable from the window

**Files:**

- Modify: `apps/desktop/src/api.ts:18-21,54,107,244,751,903`, `apps/desktop/src/App.tsx:216,314-324,427,470,534,584`, `apps/desktop/src/pages/Projects.tsx:9,47,53`, `apps/desktop/src/pages/ProjectDetail.tsx:81,88,134,735`, `apps/desktop/src/lib/projects.ts:172-190`
- Test: `apps/desktop/src/lib/projects.test.ts:141-178`

**Interfaces:**

- Consumes: Task 8's contract.
- Produces: `runControls(project, { busy })` — the `dockerAvailable` option is gone and the function's only blocking reason is `busy`.

- [ ] **Step 1: Write the failing test**

Replace the five Docker tests in `projects.test.ts` with:

```typescript
describe('runControls', () => {
  it('blocks only while the project is busy', () => {
    expect(runControls(project(), { busy: false })).toEqual({ blocked: false });
    expect(runControls(project(), { busy: true }).blocked).toBe(true);
  });

  it('never blocks for a reason outside the project', () => {
    // Docker used to block here. Nothing outside the project does any more:
    // a missing runtime is caught at start and offers an install, which is a
    // better answer than a disabled button with no explanation.
    expect(runControls(project(), { busy: false }).reason).toBeUndefined();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/desktop && npx vitest run src/lib/projects.test.ts`
Expected: FAIL — `runControls` still requires `dockerAvailable`.

- [ ] **Step 3: Implement**

Delete `dockerAvailable`, `dockerSummary`, `dockerVersion`, `dockerHint` and `dockerEnabled` from `api.ts` and its mapping comment at line 54; drop the prop from `App.tsx`'s three component calls and from its `useMemo` dependency list at line 427; drop it from `Projects.tsx` and `ProjectDetail.tsx`'s prop types; simplify `runControls`. Rewrite the three explanatory comments that describe Docker behaviour (`api.ts:107`, `api.ts:244`, `ProjectDetail.tsx:735`, `Projects.tsx:9`) to say what is now true rather than deleting them — per-project CPU and memory are still not measured, and that paragraph should say so without blaming a daemon.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd apps/desktop && npx tsc --noEmit && npx eslint src --max-warnings 0 && npx vitest run`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src
git commit -m "Stop asking the window whether Docker is available"
```

---

### Task 10: Process commands over the wire

**Files:**

- Modify: `apps/desktop/src-tauri/src/lib.rs`, `crates/api-types/src/dto.rs`
- Test: `crates/api-types` contract test

Also in this task: `kill_project` (`apps/desktop/src-tauri/src/lib.rs:1021`) and `host_projects_running` (line 1015) are repointed at `Orchestrator` — `kill_project` now stops a project's processes in reverse order and escalates past the grace period, which is the spec's stuck-process remedy, and `host_projects_running` counts projects rather than handles now that one project owns several.

**Interfaces:**

- Consumes: Tasks 2 and 6.
- Produces:
  - `ProcessSummary { id, name, startOrder, status, port: number | null, exitCode: number | null, failureReason: string | null, restartCount }` in the generated contract.
  - `list_project_processes(project_id) -> Vec<ProcessSummary>`
  - `restart_project_process(project_id, process_name) -> ()`
  - `project_console(project_id, process_name: Option<String>)` — the existing command gains an optional process argument; `None` keeps today's behaviour of the first process.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_contract_carries_a_process_summary() {
    let contract = generate_contract();
    assert!(contract.contains("ProcessSummary"));
    assert!(contract.contains("startOrder"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p project-host-api-types contract`
Expected: FAIL.

- [ ] **Step 3: Implement**

Add the DTO, add the three commands, register them in `invoke_handler`. `list_project_processes` reads rows and joins the allocated port from `project_ports` by `process_id`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm contracts && cargo test --workspace`
Expected: PASS. Commit the regenerated contract.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Expose a project's processes to the window"
```

---

### Task 11: The process list in ProjectDetail

**Files:**

- Create: `apps/desktop/src/components/ProcessList.tsx`
- Create: `apps/desktop/src/components/ProcessList.test.tsx`
- Modify: `apps/desktop/src/pages/ProjectDetail.tsx`, `apps/desktop/src/api.ts`

**Interfaces:**

- Consumes: Task 10's `ProcessSummary`, `listProjectProcesses`, `restartProjectProcess`.
- Produces: `<ProcessList projectId={...} processes={...} onRestart={...} />`.

Every control must be wired to a real command. A restart button that does not call `restart_project_process` is the failure mode this whole plan exists to avoid.

- [ ] **Step 1: Write the failing test**

```typescript
it('names the process that failed and why', () => {
  render(
    <ProcessList
      projectId="p1"
      processes={[
        { id: '1', name: 'api', startOrder: 0, status: 'CRASHED', port: 3001,
          exitCode: 1, failureReason: 'ECONNREFUSED', restartCount: 5 },
        { id: '2', name: 'web', startOrder: 1, status: 'STOPPED', port: 5173,
          exitCode: null, failureReason: null, restartCount: 0 },
      ]}
      onRestart={() => {}}
    />,
  );

  expect(screen.getByText('api')).toBeInTheDocument();
  expect(screen.getByText(/ECONNREFUSED/)).toBeInTheDocument();
  expect(screen.getByText('3001')).toBeInTheDocument();
});

const processes: ProcessSummary[] = [
  { id: '1', name: 'api', startOrder: 0, status: 'CRASHED', port: 3001,
    exitCode: 1, failureReason: 'ECONNREFUSED', restartCount: 5 },
  { id: '2', name: 'web', startOrder: 1, status: 'STOPPED', port: 5173,
    exitCode: null, failureReason: null, restartCount: 0 },
];

it('restarts the process it names, not the project', async () => {
  const onRestart = vi.fn();
  render(<ProcessList projectId="p1" processes={processes} onRestart={onRestart} />);

  await userEvent.click(screen.getAllByRole('button', { name: /restart/i })[0]);

  expect(onRestart).toHaveBeenCalledWith('api');
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/desktop && npx vitest run src/components/ProcessList.test.tsx`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

Build the component following the existing component conventions in `apps/desktop/src/components`. Add `listProjectProcesses` and `restartProjectProcess` wrappers to `api.ts` beside the existing invoke wrappers. Mount it in `ProjectDetail.tsx`, polled on the same interval the page already uses for status.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd apps/desktop && npx tsc --noEmit && npx eslint src --max-warnings 0 && npx vitest run`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src
git commit -m "Show a project's processes, and restart one of them"
```

---

### Task 12: Detection proposes a process set

**Files:**

- Modify: `crates/app-core/src/runtime_plan.rs`, `apps/desktop/src-tauri/src/lib.rs` (`create_project`)
- Test: `crates/app-core/src/runtime_plan.rs` test module

**Interfaces:**

- Consumes: Tasks 2 and 5.
- Produces: `RuntimePlan` gains `pub processes: Vec<NewProcess>`, and its `container_port` field is renamed `port`. `create_project` writes the process rows via `replace_processes`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_plan_always_proposes_at_least_one_process() {
    for runtime in Runtime::ALL {
        let plan = plan_named(runtime.as_str()).expect("plan");
        assert!(
            !plan.processes.is_empty(),
            "{} proposed nothing to run",
            runtime.as_str()
        );
        assert_eq!(plan.processes[0].start_order, 0);
    }
}

#[test]
fn a_single_runtime_project_proposes_exactly_one_process_named_main() {
    let plan = plan_named("NODEJS").expect("plan");
    assert_eq!(plan.processes.len(), 1);
    assert_eq!(plan.processes[0].name, "main");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p project-host-core runtime_plan`
Expected: FAIL — `RuntimePlan` has no `processes`.

- [ ] **Step 3: Implement**

Have `defaults_for` return a single `NewProcess` named `main` carrying the start, install and build commands it already produces, with `working_dir` `.`. Rename `container_port` to `port` throughout. Update `create_project` to call `replace_processes` with the plan's processes after creating the project row.

Multi-process _detection_ — proposing `api` and `web` from a monorepo — is deliberately not in this task. The plumbing that makes a process set possible is what this plan delivers; teaching detection to find one is its own change against a data model that by then exists.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Propose a process set when a project is created"
```

---

### Task 13: The manual checklist

**Files:**

- Create: `docs/superpowers/checklists/2026-08-26-process-orchestration.md`

**Interfaces:**

- Consumes: everything above.
- Produces: the document that carries the verification burden the unit tests deliberately do not.

The spec chose unit tests plus a manual checklist over committed fixture projects. That choice is only honest if the checklist is specific enough to actually run, and if the spec's claim about what has and has not executed on this machine is kept true.

- [ ] **Step 1: Write the checklist**

One section per scenario, each with the project to use, the exact steps, and the observable result:

1. Single Node project — import, Run, reach it on its port, Stop.
2. Two-process full-stack — API and web, confirm the API reaches `RUNNING` before the web process spawns.
3. Port conflict — hold the allocated port with another program, press Run, confirm the message names the port and that nothing spawned.
4. Missing runtime — rename the Python executable off `PATH`, press Run, confirm the blocker names Python and offers an install.
5. Crash loop — a process that exits immediately; confirm five restarts with doubling backoff, then `CRASHED` naming the process and showing its last output.
6. Restart reset — a process that runs over 60 seconds then crashes; confirm the restart count starts again rather than accumulating toward the limit.
7. Stuck process — a child that ignores the graceful stop; confirm escalation after the grace period.
8. Three projects at once — confirm distinct ports, independent logs, and that stopping one leaves the others up.
9. Terminal crash tears down siblings — kill the API past its restart limit; confirm the web process is stopped in reverse order and the project reports which process failed.
10. Restart one process — confirm the siblings keep their pids.
11. Migration — open a database created before 0009, confirm each project has one `main` process and still starts.
12. Application quit — confirm no child survives.

- [ ] **Step 2: Update the spec's verification section**

After running the checklist, record in `docs/superpowers/specs/2026-08-26-process-orchestration-design.md` which of the twelve actually ran and which did not. Per this repository's convention, an unrun scenario is stated, not omitted.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers
git commit -m "Write down what still has to be checked by hand"
```
