# Running projects on this machine: the four specs after orchestration

## Why this document exists

The request behind it was one sentence — a user imports a project, presses Run,
and it works — but it names five subsystems: multi-process execution, databases,
import-in-place, an interactive terminal, and public hosting with custom
domains. One spec covering all five would be a spec nobody could execute, and
this repository already has a recurring defect of code that is written, tested
and never called. That defect is what scope outrunning a plan looks like here.

So the work is decomposed. Multi-process execution and the Docker excision are
specified in full in `2026-08-26-process-orchestration-design.md`. This document
records the design decisions for the other four, so that each can be written up
and built without re-deciding anything, and so the order is deliberate rather
than whatever came to hand.

## Order, and what forces it

    orchestration (A)
      ├── databases (B)        a database is a supervised process
      ├── import + Run (C)     detection must propose a process *set*
      └── hosting (E)          routes to a process's allocated port
    terminal (D)               independent of all of them

A comes first because B, C and E all express themselves in terms of processes,
and until a project can have more than one there is nothing for them to attach
to. D touches nothing the others touch and can land whenever.

E comes last on purpose. It is the largest, it carries the most security
surface, and it is worth nothing until A through C make projects start
reliably. Exposing an unreliable project to the public internet is not a
feature.

## B · Databases as managed services

A project that needs Postgres had Docker to get it from. Nothing replaced that,
so this is the largest functional hole the Docker removal leaves.

**A database is a process the project depends on.** That is the whole design,
and it is why B waits for A: a `project_services` row is a `project_processes`
row with a `start_order` below every user process, a data directory, and a
connection string that gets injected.

**Obtained through the existing toolchain catalog.** `winget_id` on Windows,
`linux_packages` for apt, dnf, pacman and zypper elsewhere — the same
`ToolchainSpec` shape, the same blocker when a platform packages nothing, the
same install offer the user already sees for a missing runtime. Rejected:
downloading portable server binaries into the app data directory, which would
make this application responsible for shipping and updating database engines,
and which several engines do not offer per platform anyway.

Engines: PostgreSQL, MySQL, MongoDB, Redis. SQLite needs nothing — it is a file
the project already owns, and offering to install it would be theatre.

Each project gets its own data directory under the application data root and its
own allocated port, so two projects wanting Postgres do not fight. The
connection string is injected as `DATABASE_URL` (or the engine's conventional
name) through the existing environment path. Stopping the project stops its
services; the data directory survives, because a database that forgets on
restart is not a database.

**The limitation must be stated in the interface before anything is installed.**
This puts a real database server on the user's machine, system-wide, at their
consent. That is the price of not requiring Docker, and hiding it would be
dishonest.

## C · Import in place, and the Run button end to end

`SourceSpec` in `app-core/src/provisioning.rs` already documents the way in:
`LOCAL_FOLDER`, `ZIP_UPLOAD` and `DUPLICATE` are absent only because nothing in
the interface asks for them, and adding one is "a variant and a match arm, not a
redesign". `file-manager` can already do the copying.

`LocalFolder { path, copy: bool }`. With `copy: false` the directory is
registered where it sits and Panel never moves the user's files — the behaviour
someone expects from an editor. `projects.directory` is already `UNIQUE`, which
is what stops the same folder being registered twice.

Import then runs detection, which after A proposes a **set** of processes rather
than one command, and drops the user on a confirmation screen showing exactly
what will be installed, built and run, with every value editable. Nothing is
executed before that screen is accepted: detection reads files and never runs
anything from the project, and that rule does not bend for import.

Then Run, exactly as the orchestration spec describes it.

## D · Interactive terminal

A real PTY through `portable-pty`, one session per project, rooted at the
project's directory, carrying the project's environment variables and the
resolved toolchain `PATH` so that `npm` means the same `npm` the project runs
with. Output streams to an xterm.js pane; sessions end when the project view is
closed.

There is no PTY anywhere in the tree today — `project_console` tails logs and is
read-only — so this is new code rather than a change to existing code, which is
why it is independent.

**Stated plainly, once, in writing:** a terminal is arbitrary code execution on
the user's machine, by design. That is what a development environment is. But it
ends any remaining pretence of a sandbox, and the security posture should say so
rather than let the absence of a statement imply otherwise.

## E · Public hosting and custom domains

Two halves, and only the second was ever in doubt.

**The local half — `crates/gateway`.** A reverse proxy that terminates on
`*.localhost` and routes by hostname to each project's allocated port, so a
project has a stable name instead of a port number that changes. It reuses the
path-safety work in `crates/static-server`, whose `resolve` is already a pure
function tested against the traversal attempts that matter.

**The public half — a managed tunnel.** A bundled tunnel client connects
outward to a provider edge, which terminates TLS and forwards to the local
gateway. No router configuration, no port forwarding, no privileged ports, and
it works behind CGNAT — which the direct alternative does not.

Rejected: binding 80 and 443 directly with ACME HTTP-01 certificates and asking
the user to forward ports. It is self-owned, and it fails for every user behind
carrier-grade NAT, requires elevation for privileged ports, and publishes a home
IP address. The cost accepted instead is a third-party account and a provider in
the data path, which the interface must name before a domain is connected.

`static-server` binding loopback only is a deliberate decision, not an
oversight. Nothing in E changes it; the gateway is the only thing that ever
faces outward.

## Verification, across all four

The choice made for A applies here too: unit tests plus a manual checklist,
rather than committed fixture projects exercised by real spawn tests.

The consequence is worth writing down. This repository's own convention is that
a module states what has actually run on the development machine and what has
only compiled, because this machine has no Docker, no WSL and no Linux, and
whole categories of code here have never met the thing they talk to. Every spec
in this roadmap adds more of that category — a database engine, a PTY, a tunnel
provider — and each must say so in its module headers rather than leave it
implied.
