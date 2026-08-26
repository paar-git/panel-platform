-- A project gets more than one process, and the last of Docker leaves the
-- schema.
--
-- Until now a project was one `project_runtimes` row with one `start_command`.
-- That shape cannot describe a full-stack application — an API and a web dev
-- server, or anything with a worker — so the execution fields move out of
-- `project_runtimes` and into a row per process. What stays behind in
-- `project_runtimes` is what identifies the project's *toolchain*, which is a
-- property of the project and not of any one process.
--
-- Four changes, four rebuilds.
--
-- 1. `project_processes` is new. Its lower half is deliberately *observed*
--    state — status, pid, exit code, the reason it failed — written from what
--    a process actually did and never from what was intended, the same rule
--    `projects.status` follows and the reason `desired_state` is separate.
--
-- 2. `projects` loses `container_id`, `container_name`, `image_tag`,
--    `network_name`, `volume_name` and `run_mode`. 0008 moved every row to
--    HOST and kept DOCKER in the CHECK because `docker-manager` still
--    compiled. It does not any more: the runner, the crate, the generated
--    Dockerfiles and the templates are all gone, so a column recording which
--    of two substrates to use has one value and no purpose.
--
-- 3. `project_runtimes` loses `start_command`, `install_command`,
--    `build_command`, `working_dir` and the six `health_*` columns.
--
-- 4. `project_ports` renames `container_port` to `port` — there is no
--    container for it to be inside — and gains `process_id`, so that "which
--    process listens on 5173" has an answer.
--
-- SQLite cannot alter a CHECK constraint or a column default, so each rebuild
-- is copy, drop, rename, with the column list written out rather than
-- `SELECT *`, because a positional copy is the mistake that survives review.
-- `Database::migrate` disables foreign keys for the duration and runs
-- `foreign_key_check` afterwards.

-- ------------------------------------------------------------ projects

CREATE TABLE projects_rebuilt (
    id             TEXT PRIMARY KEY,
    slug           TEXT NOT NULL UNIQUE,
    display_name   TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    project_type   TEXT NOT NULL CHECK (project_type IN (
        'DISCORD_BOT','NODE_APP','PYTHON_APP','WEBSITE','STATIC_SITE',
        'REST_API','WORKER','SERVICE')),
    icon           TEXT,
    color          TEXT,

    status         TEXT NOT NULL DEFAULT 'CREATING' CHECK (status IN (
        'CREATING','STOPPED','STARTING','RUNNING','STOPPING','RESTARTING',
        'BUILDING','CRASHED','FAILED','UNHEALTHY','ARCHIVED','DELETING')),
    desired_state  TEXT NOT NULL DEFAULT 'STOPPED' CHECK (desired_state IN (
        'RUNNING','STOPPED','ARCHIVED')),
    health         TEXT NOT NULL DEFAULT 'UNKNOWN' CHECK (health IN (
        'UNKNOWN','STARTING','HEALTHY','UNHEALTHY','NONE')),

    source_type    TEXT NOT NULL CHECK (source_type IN (
        'EMPTY','ZIP_UPLOAD','LOCAL_FOLDER','DUPLICATE','IMPORT_ARCHIVE',
        'GIT_CLONE','REMOTE_ARCHIVE')),
    directory      TEXT NOT NULL UNIQUE,

    source_url     TEXT,
    source_ref     TEXT,
    source_commit  TEXT,

    autostart      INTEGER NOT NULL DEFAULT 0 CHECK (autostart IN (0, 1)),
    restart_policy TEXT NOT NULL DEFAULT 'UNLESS_STOPPED' CHECK (restart_policy IN (
        'NO','ON_FAILURE','UNLESS_STOPPED','ALWAYS')),
    network_mode   TEXT NOT NULL DEFAULT 'INTERNAL' CHECK (network_mode IN (
        'NONE','INTERNAL','LAN','INTERNET')),

    priority       TEXT NOT NULL DEFAULT 'NORMAL' CHECK (priority IN ('LOW','NORMAL','HIGH')),
    keep_awake     INTEGER NOT NULL DEFAULT 0 CHECK (keep_awake IN (0, 1)),

    memory_limit_mb  INTEGER NOT NULL DEFAULT 512,
    cpu_limit_cores  REAL    NOT NULL DEFAULT 1.0,
    storage_limit_mb INTEGER NOT NULL DEFAULT 2048,
    process_limit    INTEGER NOT NULL DEFAULT 128,

    started_at          TEXT,
    stopped_at          TEXT,
    last_exit_code      INTEGER,
    last_failure_at     TEXT,
    last_failure_reason TEXT,
    restart_count       INTEGER NOT NULL DEFAULT 0,

    archived_at TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,

    CHECK (memory_limit_mb BETWEEN 64 AND 65536),
    CHECK (cpu_limit_cores > 0 AND cpu_limit_cores <= 64),
    CHECK (storage_limit_mb BETWEEN 128 AND 1048576),
    CHECK (process_limit BETWEEN 8 AND 4096),
    CHECK (slug GLOB '[a-z0-9][a-z0-9-]*'),
    CHECK (source_type NOT IN ('GIT_CLONE','REMOTE_ARCHIVE') OR source_url IS NOT NULL),
    CHECK (source_type IN ('GIT_CLONE','REMOTE_ARCHIVE') OR source_url IS NULL),
    CHECK (source_type = 'GIT_CLONE' OR (source_ref IS NULL AND source_commit IS NULL)),
    CHECK (source_url IS NULL OR source_url NOT LIKE '%@%')
);

INSERT INTO projects_rebuilt (
    id, slug, display_name, description, project_type, icon, color,
    status, desired_state, health,
    source_type, directory, source_url, source_ref, source_commit,
    autostart, restart_policy, network_mode,
    priority, keep_awake,
    memory_limit_mb, cpu_limit_cores, storage_limit_mb, process_limit,
    started_at, stopped_at, last_exit_code, last_failure_at,
    last_failure_reason, restart_count,
    archived_at, created_at, updated_at
)
SELECT
    id, slug, display_name, description, project_type, icon, color,
    status, desired_state, health,
    source_type, directory, source_url, source_ref, source_commit,
    autostart, restart_policy, network_mode,
    priority, keep_awake,
    memory_limit_mb, cpu_limit_cores, storage_limit_mb, process_limit,
    started_at, stopped_at, last_exit_code, last_failure_at,
    last_failure_reason, restart_count,
    archived_at, created_at, updated_at
FROM projects;

DROP TABLE projects;

ALTER TABLE projects_rebuilt RENAME TO projects;

CREATE INDEX idx_projects_status ON projects (status);
CREATE INDEX idx_projects_desired ON projects (desired_state) WHERE archived_at IS NULL;

-- --------------------------------------------------- project_processes

CREATE TABLE project_processes (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    start_order     INTEGER NOT NULL,
    command         TEXT NOT NULL,
    -- Relative to `projects.directory`. The old `project_runtimes.working_dir`
    -- defaulted to `/app`, a path inside a container that does not exist on
    -- this machine.
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

    -- Observed, never intended.
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

-- ---------------------------------------------------- project_runtimes

CREATE TABLE project_runtimes_rebuilt (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    runtime         TEXT NOT NULL CHECK (runtime IN (
        'NODEJS','TYPESCRIPT','BUN','DENO','PYTHON','GO','RUST','JAVA','PHP',
        'RUBY','DOTNET','STATIC','POLYGLOT')),
    runtime_version TEXT NOT NULL,
    package_manager TEXT NOT NULL DEFAULT 'NONE' CHECK (package_manager IN (
        'PNPM','NPM','YARN','BUN','DENO','PIP','POETRY','UV','PIPENV',
        'GO_MODULES','CARGO','MAVEN','GRADLE','COMPOSER','BUNDLER','NUGET','NONE')),
    entry_file      TEXT,
    publish_dir     TEXT,
    template_id     TEXT NOT NULL
);

INSERT INTO project_runtimes_rebuilt (
    project_id, runtime, runtime_version, package_manager,
    entry_file, publish_dir, template_id
)
SELECT
    project_id, runtime, runtime_version, package_manager,
    entry_file, publish_dir, template_id
FROM project_runtimes;

DROP TABLE project_runtimes;

ALTER TABLE project_runtimes_rebuilt RENAME TO project_runtimes;

-- ------------------------------------------------------- project_ports

CREATE TABLE project_ports_rebuilt (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    -- The port the project's own process binds. Called `container_port` until
    -- there stopped being a container.
    port         INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    host_port    INTEGER CHECK (host_port BETWEEN 1024 AND 65535),
    protocol     TEXT NOT NULL DEFAULT 'tcp' CHECK (protocol IN ('tcp','udp')),
    bind_address TEXT NOT NULL DEFAULT '127.0.0.1',
    is_primary   INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    -- Which process listens on it. Nullable: a port can be allocated to a
    -- project before its process set is decided.
    process_id   TEXT REFERENCES project_processes(id) ON DELETE CASCADE,
    -- Makes double allocation a database error rather than a race.
    UNIQUE (host_port, protocol, bind_address)
);

-- Existing ports belong to the one process this migration just created for
-- their project, which is the only process there is.
INSERT INTO project_ports_rebuilt (
    id, project_id, port, host_port, protocol, bind_address, is_primary, process_id
)
SELECT
    old.id, old.project_id, old.container_port, old.host_port, old.protocol,
    old.bind_address, old.is_primary,
    (SELECT p.id FROM project_processes p
      WHERE p.project_id = old.project_id AND p.name = 'main')
FROM project_ports old;

DROP TABLE project_ports;

ALTER TABLE project_ports_rebuilt RENAME TO project_ports;

CREATE INDEX idx_ports_project ON project_ports (project_id);
CREATE INDEX idx_ports_process ON project_ports (process_id);
