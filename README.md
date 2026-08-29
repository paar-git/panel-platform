# Panel Platform

A cross-platform **desktop application** for running and managing projects on a
machine you own — Discord bots, Node.js and Python applications, websites,
static sites, REST APIs and background workers, each in its own Docker
container.

A project can start empty, or be installed from a **GitHub repository or any
HTTPS git remote**, downloaded from an **archive URL**, or named as `owner/repo`
and cloned with **your own `gh` login**. Paste
`https://github.com/owner/some-cli.git` and it becomes a project.

**The language is detected, not asked for.** Thirteen runtimes — Node,
TypeScript, Bun, Deno, Python, Go, Rust, Java, PHP, Ruby, .NET, static sites, and
a polyglot image for a project that needs several at once. Files can then be
**edited in the application**, in a Monaco editor with a file tree and tabs.

It is a desktop program, not a web panel. Everything runs in one process on your
own machine: a Tauri 2 window on top of a Rust core that owns the database,
the project files and the Docker connection.

Projects keep running when the window is closed, because Docker's own daemon
keeps them running under their restart policy. What pauses with the application
is the part that watches them — scheduled backups, log capture and metrics.

Optionally, projects can also be watched and controlled from **Discord**: each
project gets a log channel and a control panel, with role-based permissions.

---

## Download the app

### ➡ [**Download the latest release**](https://github.com/paar-git/panel-platform/releases/latest)

Or download the setup program for your system from that page — it works out
which file you need, checks it against the project's signature, and starts the
installer.

**You do not need to clone this repository, download the source code, or
install any development tools.** No Node.js, no Rust, no pnpm, no toolchain —
everything the application needs is inside the installer. The "Source code"
archives GitHub attaches to every release are only for people who intend to
build it themselves.

| Platform           | File                                      | Notes                                       |
| ------------------ | ----------------------------------------- | ------------------------------------------- |
| Windows 10/11 x64  | `Panel.Platform_<version>_x64-setup.exe`  | Recommended. Installs the WebView2 runtime. |
| Windows 10/11 x64  | `Panel.Platform_<version>_x64_en-US.msi`  | For deployment tooling and Group Policy.    |
| Debian, Ubuntu     | `Panel.Platform_<version>_amd64.deb`      | `sudo apt install ./<file>.deb`             |
| Other Linux x86-64 | `Panel.Platform_<version>_amd64.AppImage` | `chmod +x` and run. Self-updates.           |

### Installing

**Windows** — run the `-setup.exe`. It installs the WebView2 runtime if the
machine does not already have it, and the application appears in the Start Menu
as _Panel Platform_.

**Debian or Ubuntu** — `sudo apt install ./Panel.Platform_<version>_amd64.deb`

**Any other Linux** — `chmod +x` the AppImage and run it.

### What you need

Nothing to install or open the application. **Docker is required to actually
run projects**, because a project runs in a container — the application
installs and opens without it and tells you it is missing, rather than failing
at launch. Install [Docker Desktop](https://www.docker.com/products/docker-desktop/)
on Windows, or Docker Engine on Linux.

### These builds are unsigned

There is no code-signing certificate yet, and the release notes say so rather
than leaving you to guess:

- **Windows** shows a "Windows protected your PC" warning on first run,
  offering only "Don't run" until you choose "More info" → "Run anyway". The
  release can sign its Windows builds through Azure Trusted Signing as soon as
  a certificate exists; the machinery is in place and waiting on the account.
- **Linux** packages carry no repository signature.

Every release has a `SHA256SUMS.txt`. To check what you downloaded:

```powershell
# Windows
Get-FileHash .\Panel.Platform_0.1.0_x64-setup.exe -Algorithm SHA256
```

```bash
# Linux
sha256sum -c SHA256SUMS.txt --ignore-missing
```

### Updates

The application checks for new releases and offers them; nothing installs
without you pressing the button. Downloads are verified against a signing key
compiled into the binary, not one supplied by the feed.

**The `.deb` cannot update itself** — a package manager owns those files, so
Debian and Ubuntu users upgrade by installing the next `.deb`. The AppImage and
both Windows installers update in place.

> **Not yet verified.** No release has been published, so the update path has
> never run end to end, and neither installer has been installed on a clean
> machine. See [Verification status](#verification-status).

---

## Status: it launches

`pnpm --filter project-host-desktop exec tauri build` produces a signed-in-name
`Panel Platform` bundle plus an MSI and an NSIS installer, and running it opens
a window. On first launch it created
`C:\ProgramData\ProjectHost\` with its `data`, `config`, `logs`, `projects`,
`backups` and `tmp` directories, migrated the database to schema version 2 (28
tables) and recorded its instance in `agent_state`. That is observed, not
inferred.

The window has a dashboard, a project list with a working creation dialog, a
per-project console screen, a file editor, and a settings screen showing the
configuration in force. It is not yet the thirty-item interface in the
specification.

Verified on the development machine: **635 Rust tests, 52 TypeScript tests**,
clippy clean under `-D warnings`, `cargo fmt`, ESLint, Prettier, `tsc`, and the
generated TypeScript matching its Rust source.

**Not verified:** the clean-shutdown path — the launch above was force-killed,
so `last_clean_shutdown` stayed `0` and the window's own exit handler has never
run. Also unverified: anything needing a Docker daemon, a Linux host, or a
Discord bot token. See [Verification status](#verification-status).

**Projects can be created.** The dialog generates the id, derives the slug from
it, allocates a host port by testing real availability, and writes the project
across three tables in one transaction. **No container has ever been started**,
because the machine this was built on has no Docker daemon.

**Projects can be fetched from a remote.** A real shallow clone of
`github.com/octocat/Hello-World` over HTTPS, and a real archive download through
GitHub's redirect to codeload, both run and both promoted into a project
directory — verified by hand through network-gated tests, not inferred. The
files can then be listed, opened, edited and saved through the window's own
commands.

**Project files can be edited.** The tree, tabs, dirty state and save path are
built on the file operations Phase 5 tested, and Monaco is bundled rather than
fetched from a CDN. The interface itself has not been looked at on screen — the
bundle builds and the logic is tested, and no claim is made beyond that.

### A note on the architecture

Earlier revisions split this into a background OS service plus a thin desktop
client that talked to it over authenticated HTTPS on loopback. That has been
removed in favour of a single process. Gone with it: the HTTP API, the TLS
identity, password login, session tokens, login rate limiting, and the
Windows Service and systemd adapters — roughly 2,400 lines whose only purpose
was protecting a network listener that no longer exists. Nothing authenticates
to the application now, because the process runs as you, on your machine, and
can already do anything you can.

The domain crates were untouched by that change, which is the point of them
having had no idea how they were being called.

| Phase                                                    | State                 |
| -------------------------------------------------------- | --------------------- |
| 1 — Architecture                                         | ✅ Complete           |
| 2 — Foundation: workspaces, database, contracts, logging | ✅ Complete           |
| 3 — Application core: lifecycle, Docker, platform        | ✅ Complete           |
| 4 — Project management: templates, lifecycle, limits     | ◑ Partial (see below) |
| 5 — Files and environment variables                      | ◑ Partial (see below) |
| 5b — Remote sources, editing, languages                  | ◑ Partial (see below) |
| 6 — Logs and metrics                                     | Not started           |
| 7 — Backups                                              | Not started           |
| 8 — Desktop UI                                           | ◑ Partial (see below) |
| 9 — Discord integration                                  | ◑ Partial (see below) |
| 10 — Installers and updates                              | ◑ Partial (see below) |
| 11 — Security review                                     | Not started           |
| 12 — Final verification                                  | Not started           |

Phase 9 was remote management. With the service gone there is no remote to
manage, and the slot is now the Discord integration — which is what a user
actually wanted remote management for.

---

## The design

Read in this order:

| Document                                                              | Covers                                                              |
| --------------------------------------------------------------------- | ------------------------------------------------------------------- |
| [development.md](docs/development.md)                                 | Bootstrap, commands, the contract pipeline, adding a migration      |
| [architecture.md](docs/architecture.md)                               | Components, trust boundaries, crate layout, data flow, recovery     |
| [platform-support.md](docs/platform-support.md)                       | Adapter traits, Windows and Linux specifics, capability degradation |
| [agent-desktop-communication.md](docs/agent-desktop-communication.md) | Tauri IPC, TLS transport, auth, streaming, contract generation      |
| [database-schema.md](docs/database-schema.md)                         | Full SQLite schema, constraints, transactions, retention            |
| [api-design.md](docs/api-design.md)                                   | Every route, error codes, pagination, idempotency                   |
| [security.md](docs/security.md)                                       | Threat model, attackers, controls, verification plan                |
| [docker.md](docs/docker.md)                                           | Container hardening, templates, networking, runtimes                |
| [offline-mode.md](docs/offline-mode.md)                               | Five connectivity states, Discord-bot reconnection                  |
| [remote-management.md](docs/remote-management.md)                     | Pairing, device keys, certificate pinning, addressing               |
| [installers.md](docs/installers.md)                                   | MSI, `.deb`, AppImage, upgrade and uninstall behaviour              |
| [testing-strategy.md](docs/testing-strategy.md)                       | Layers, host gating, what is skipped and why                        |
| [remote-sources-and-editing.md](docs/remote-sources-and-editing.md)   | Git and archive sources, URL rules, the editor, what has been run   |
| [file-tree.md](docs/file-tree.md)                                     | Complete target layout                                              |

---

## Stack

| Layer     | Choice                                                            |
| --------- | ----------------------------------------------------------------- |
| Desktop   | Tauri 2, Rust, React 19, TypeScript, Vite, Tailwind               |
| Core      | Rust — tokio, bollard, sqlx                                       |
| Fetching  | gix (in-process git, no host binary), reqwest + rustls, zip, tar  |
| Editing   | Monaco, bundled with its workers — nothing is fetched from a CDN  |
| Database  | SQLite (WAL) via SQLx; PostgreSQL is not required and not used    |
| Transport | Tauri IPC, in-process. No sockets, no ports, no certificates      |
| Secrets   | XChaCha20-Poly1305 at rest, for environment variables and the bot |
| Discord   | serenity (gateway not yet wired)                                  |
| Contracts | Rust types → JSON Schema → generated TypeScript and Zod           |
| Testing   | `cargo test`, Vitest, Playwright, Docker integration tests        |

Node.js is a build-time dependency of the frontend only. **The application does
not require Node.js on the host**, and nothing the product installs runs on the
host outside a container.

> With one exception, the documents listed above still describe the client/server
> split and its HTTPS API. They are **out of date** as of the single-process
> change and have not yet been rewritten. The exception is
> [remote-sources-and-editing.md](docs/remote-sources-and-editing.md), which was
> written against the code as it is; `database-schema.md` has also been brought up
> to schema version 3.

---

## Requirements

|         | Minimum                                                       |
| ------- | ------------------------------------------------------------- |
| Windows | 10 1809+ or 11, x64, WebView2, Docker Desktop                 |
| Linux   | Ubuntu 22.04+ / Debian 12+, x64, Docker Engine, WebKitGTK 4.1 |

Docker is required to run projects. The application installs and starts without
it, and explains what is missing rather than failing obscurely.

---

## Development environment

Verified on the machine this was designed on:

| Tool       | Version       | State       |
| ---------- | ------------- | ----------- |
| Node.js    | 24.16.0       | ✅          |
| pnpm       | 11.1.1        | ✅          |
| Rust       | 1.96.0 (msvc) | ✅ compiles |
| git        | 2.53.0        | ✅          |
| WebView2   | installed     | ✅          |
| Docker     | —             | ❌ absent   |
| WSL        | —             | ❌ absent   |
| Linux host | —             | ❌ none     |

### What "partial" means for Phases 4 and 5

Both phases split cleanly into work that can be verified on a machine with no
Docker daemon and work that cannot. The verifiable half is **done and tested**;
the rest is not started, and is not pretended to be.

| Phase 4                                                 | State                      |
| ------------------------------------------------------- | -------------------------- |
| Three approved templates with manifests and Dockerfiles | ✅ shipped                 |
| Manifest validation (versions, managers, health checks) | ✅ 51 tests                |
| Runtime detection: Node, Python, static                 | ✅ tested incl. edge cases |
| Slug / container / network / volume naming              | ✅ tested                  |
| Host port allocation and conflict detection             | ✅ tested                  |
| Container spec with full hardening + violation checker  | ✅ 31 tests                |
| Actually building an image or starting a container      | ❌ needs Docker            |
| Health checks observed, restart behaviour, reconciler   | ❌ needs Docker            |

| Phase 5                                                          | State          |
| ---------------------------------------------------------------- | -------------- |
| `SafePath`: traversal, absolute, UNC, reserved names, ADS        | ✅ 20 tests    |
| Symlink escape and the TOCTOU recheck                            | ✅ tested      |
| ZIP entry validation: Zip Slip, bombs, ratios, devices           | ✅ 14 tests    |
| Self-cleaning staging with atomic promote                        | ✅ tested      |
| Extraction driven through a real ZIP reader, staged and promoted | ✅ 12 tests    |
| File explorer operations: list, read, write, move, copy, search  | ✅ 27 tests    |
| Environment variable manager: validation, `.env` import/export   | ✅ 28 tests    |
| Project, environment and audit storage in SQLite                 | ✅ 33 tests    |
| Exposing all of the above through the desktop app                | ❌ not started |

| Phase 5b — Remote sources and editing                           | State                    |
| --------------------------------------------------------------- | ------------------------ |
| URL and address rules: schemes, userinfo, SSRF, redirect chains | ✅ 20 tests              |
| Git clone: isolated config, no submodules, no hooks, budgets    | ✅ tested                |
| Archive download: caps, magic-number sniffing, token scope      | ✅ tested                |
| tar.gz through the existing ZIP entry rules                     | ✅ tested                |
| Migration to schema version 3, with the rebuild of `projects`   | ✅ tested against SQLite |
| Credential encryption, binding and the schema's refusals        | ✅ 8 tests               |
| A real clone and a real archive download from github.com        | ✅ run by hand           |
| Seven file commands, and the editor's tab and dirty-state rules | ✅ 21 Vitest tests       |
| Monaco bundled with no CDN reference in the built bundle        | ✅ checked               |
| Storing an access token                                         | ❌ needs a key store     |
| A symbolic link escaping a cloned tree                          | ❌ needs the privilege   |
| The editor seen on screen                                       | ❌ not looked at         |

The two features are usable and the gaps are specific. `docs/remote-sources-and-editing.md`
§8 has the full table, including how to run the network-gated tests.

**The key store is the one real hole.** A token entered for a private remote
authenticates the fetch and is then dropped, because nothing in this application
holds an encryption key at runtime — the same missing join layer Phase 5 records
below. The interface says so where the token is typed. The encrypt-and-store path
is written and tested against real encryption; wiring it up is one argument at one
call site once a key store exists.

| Phase 9 — Discord                                             | State          |
| ------------------------------------------------------------- | -------------- |
| Permission model: roles, users, blocks, lockout safety        | ✅ 16 tests    |
| Control panel and its `custom_id` encoding                    | ✅ 13 tests    |
| Channel naming: templates, sanitising, hostile names          | ✅ 15 tests    |
| Event routing, mention and code-block safety, secret masking  | ✅ 21 tests    |
| Discord ids that survive JavaScript                           | ✅ 7 tests     |
| Storage: bot token, servers, grants, channels, event settings | ✅ 27 tests    |
| Connecting to Discord and actually sending anything           | ❌ needs a bot |

### What Phase 9 does and does not reach

The whole rule set is built and tested; the gateway connection is not written.
Concretely, the crate answers "may this person press this button", "what does
this message say", and "is it safe to send" — and nothing yet holds a websocket
open to Discord, so no message has ever been sent.

What is worth stating plainly, because it is what tests were written for:

- **A project's log output cannot ping your server.** A Discord bot that logs
  the messages it receives would otherwise let any stranger trigger
  `@everyone`. Mentions are neutralised before sending.
- **A project's log output cannot escape its code block.** The fence is chosen
  longer than the longest backtick run in the content, so no output can close
  it and inject Markdown or links.
- **Secret environment variable values are masked** before a log line leaves
  for Discord — the case being a bot crashing on startup and printing its own
  token.
- **Authorisation happens when the button is pressed, not when it is drawn.**
  A panel sits in a channel for months; the person who eventually clicks it may
  have gained or lost roles since.
- **The bot token has nowhere to be stored unencrypted.** The table has a
  ciphertext column and a nonce column and nothing else.

Phases 6, 7, 8, 10, 11 and 12 are **not started**.

### What Phase 5 added, and what it does not yet reach

The security-critical half of the file and environment work is now real code
with real tests rather than validation helpers:

- **Archive import** opens an actual ZIP, applies the entry rules one at a time
  while extracting into a UUID-named staging directory, and renames it into
  place only on complete success. A Zip Slip entry or a bomb aborts the import
  and the staging directory removes itself — verified by asserting that neither
  the project nor the staging directory exists afterwards.
- **File operations** never accept a path; they accept a project root and a
  request string, and construct a `SafePath` themselves. Symbolic links (and
  Windows junctions, which `symlink_metadata` reports the same way) are listed
  so they can be deleted, and refused as the target of every other operation.
- **Environment variables** are validated, parsed from `.env` with the syntax
  people really have, and exported to `.env.example` with secret values omitted
  entirely rather than masked. A round-trip test parses the exported file back
  and compares values, because the quoting rules are where this goes wrong.
- **Storage** for projects spans three tables written in one transaction, and
  the environment-variable repository never sees an encryption key — a secret
  arrives as ciphertext and the schema `CHECK` refuses any row that pairs
  `is_secret = 1` with a plaintext value. That refusal is asserted directly
  against SQLite with raw SQL.

What is **not** built: none of this is reachable over the API yet. There are no
`/projects`, `/files` or `/env` routes, and the agent does not yet encrypt a
secret on its way to storage — the key management exists in `security`, the
storage exists in `database`, and the layer that joins them does not.

### Verification status

What Phase 3 shipped, split by whether it has actually been run:

| Behaviour                                                         | Status                               |
| ----------------------------------------------------------------- | ------------------------------------ |
| Agent starts as a standalone process and serves HTTPS             | ✅ run by hand and in tests          |
| Loopback-only bind (`netstat` shows `127.0.0.1:8787`)             | ✅ verified, LAN address refused     |
| TLS with a pinned self-signed certificate, stable across restarts | ✅ verified                          |
| Database creation, migrations, schema-version guard               | ✅ verified                          |
| Crash detection and startup recovery                              | ✅ verified (hard-killed, restarted) |
| Graceful shutdown: clean flag, WAL checkpoint, token removal      | ✅ verified in an integration test   |
| Administrator setup, bootstrap token, recovery codes              | ✅ verified over HTTPS               |
| Login, sessions, logout, revocation                               | ✅ verified over HTTPS               |
| Login rate limiting with exponential lockout                      | ✅ verified over HTTPS               |
| Docker detection reporting a real absence with a hint             | ✅ verified (no daemon on this host) |
| Operation locking, expiry, per-project isolation                  | ✅ verified in integration tests     |
| A shallow git clone over HTTPS, and an archive URL download       | ✅ run by hand against github.com    |
| A byte budget interrupting a clone that was already running       | ✅ verified                          |
| Docker **connected** behaviour — version, containers, events      | ❌ needs a daemon                    |
| Windows Service registration and SCM lifecycle                    | ❌ needs an elevated Windows session |
| systemd unit installation, `Type=notify` readiness                | ❌ needs an Ubuntu/Debian host       |
| Linux file modes and the dedicated service user                   | ❌ needs a Linux host                |

The unverified rows are implemented, not stubbed. Each is marked in its own
source file with why it could not be run here. No claim is made that they work.

**What that means for claims of completeness.** Roughly two thirds of the
planned test suite runs here: all Rust unit and integration tests, the container
_specification_ security assertions, SQLite migrations, contract generation, the
whole frontend, and the Windows platform suite. Anything requiring a Docker
daemon, systemd, a `.deb`, or a second machine cannot run, and will be reported
as **skipped and unverified** — never as passing. `docs/testing-strategy.md` §2
has the exact table.

Reaching all 30 acceptance criteria needs a Windows machine with Docker Desktop,
an Ubuntu machine with Docker Engine, and both on one network.

---

## Licence

[MIT](LICENSE).
