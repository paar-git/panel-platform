# Checking multi-process execution by hand

## Why this document exists

The design chose **unit tests plus a manual checklist** over committed fixture
projects exercised by real spawn tests. That choice is only honest if the
checklist is specific enough to actually run, and if what has and has not been
executed is written down rather than implied.

The orchestration state machine is pure, so ordering, the crash rule, status
aggregation, reverse-order stop and prepare deduplication are all covered by
unit tests. **Nothing below is.** Every scenario here exercises a real process
on a real machine, which no test in this repository does.

Record the result beside each item. An unrun scenario stays marked unrun — this
repository's convention is to say so out loud rather than leave it to be
assumed.

| Legend | Meaning                                                        |
| ------ | -------------------------------------------------------------- |
| ☐      | not yet run                                                    |
| ✅     | run, behaved as described                                      |
| ❌     | run, did not behave as described — file the defect and link it |

---

## 1. A single Node project ☐

**Setup.** Any directory with a `package.json` whose `start` script runs a
server that binds `process.env.PORT`.

1. Import it, accept the detected process set.
2. Press Run.
3. Open the allocated port in a browser.
4. Press Stop.

**Expect.** One process named `main` reaching `RUNNING`. The page loads. After
Stop the process list shows `STOPPED` and the port is free.

## 2. Two processes, in order ☐

**Setup.** A project with two process rows: `api` (order 0, a server on its
`PORT`) and `web` (order 1, a Vite dev server). Give `api` an HTTP health check
against `/health`.

1. Press Run.
2. Watch the process list.

**Expect.** `api` reaches `RUNNING` **before** `web` leaves `STOPPED`. This is
the whole point of ordering: `web` must not spawn until `api` answers its
health check. If both go at once, the gating is broken.

## 3. Port conflict ☐

**Setup.** Note the port a stopped project was allocated. Hold it with another
program (`python -m http.server <port>` or similar).

1. Press Run.

**Expect.** The start refuses **before anything spawns**, with a message naming
the port. Every process row stays `STOPPED`. Nothing appears in the log. If a
process spawned and then died on `EADDRINUSE`, the pre-flight bind test is not
running.

## 4. Missing runtime ☐

**Setup.** A Python project, with `python` removed from `PATH` for the session.

1. Press Run.

**Expect.** A blocker naming Python and offering to install it. Not a spawn
failure, and not "command not found".

## 5. Crash loop ☐

**Setup.** A one-process project whose command exits non-zero immediately
(`node -e "process.exit(1)"`).

1. Press Run.
2. Watch for about a minute.

**Expect.** Five restart attempts with doubling backoff — roughly 1s, 2s, 4s,
8s, 16s — then the process settles on `CRASHED`, the restart count reads 5, and
the process list shows the child's own last output. No sixth attempt.

## 6. The restart counter resets ☐

**Setup.** A process that runs for more than 60 seconds and then exits
non-zero (`node -e "setTimeout(() => process.exit(1), 70000)"`).

1. Press Run, wait for the crash, then let it restart and crash again.

**Expect.** The restart count does **not** accumulate toward the limit across
the two crashes. An attempt that stayed up past `STABLE_RUN` resets it. A crash
loop is a rate, not a total; if the count climbs, a project that crashes once a
week will eventually be abandoned as a crash loop.

## 7. A stuck process ☐

**Setup.** A process that ignores a graceful stop — on Windows, a Node process
with a `SIGTERM` handler that does nothing and an open interval.

1. Press Run, then Stop.

**Expect.** The stop waits out the grace period, then escalates and the process
dies. The row ends `STOPPED`, not `STOPPING` forever.

> On Windows this uses `crates/platform`'s process-group termination, which has
> run on this machine. **The Unix branch has never run** — there is no Linux or
> macOS here.

## 8. Three projects at once ☐

1. Run three different projects.

**Expect.** Three distinct allocated ports, three independent log streams, and
stopping one leaves the other two `RUNNING`.

## 9. A terminal crash tears down the siblings ☐

**Setup.** The two-process project from scenario 2, with `api` made to
crash-loop.

1. Press Run, wait for `api` to exhaust its restarts.

**Expect.** `web` is stopped **after** `api` becomes terminal, in reverse order.
The project reports `CRASHED`, and the reason names `api` — not a generic
failure. `web` must not be torn down while `api` is merely `RESTARTING`.

## 10. Restarting one process ☐

**Setup.** The two-process project, running, with both pids noted.

1. Press Restart on `web` only.

**Expect.** `web` gets a new pid; `api` keeps its old one. If both pids change,
the per-process restart is restarting the project.

## 11. An older database upgrades ☐

**Setup.** A copy of a `project-host.db` written by 0.1.15 (schema 8) with at
least one project in it.

1. Open it with this build.

**Expect.** Schema 9. Every project has exactly one process named `main`
carrying its old start command, with `working_dir` `.` rather than `/app`. Its
ports survive with the same numbers, now linked to that process. The project
still starts.

> Covered by a unit test at the SQL level
> (`an_existing_project_gains_one_process_and_keeps_its_port`). What that test
> cannot show is that the upgraded project still _runs_, which is what this
> scenario adds.

## 12. Quitting stops everything ☐

1. Run two projects, then quit the application.
2. Check the process table for orphans.

**Expect.** No child survives. This is a behaviour change: containers used to
outlive the application deliberately, and nothing does now. The rows should
read `status = STOPPED` beside `desired_state = RUNNING`, which is what a
"start these again?" prompt on the next launch reads.

---

## After running these

Update the **Verification** section of
`docs/superpowers/specs/2026-08-26-process-orchestration-design.md` with which
scenarios actually ran. Leaving it unstated would let the spec imply coverage
it does not have, which is the failure this whole document exists to prevent.
