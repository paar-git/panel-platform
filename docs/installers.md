# Installers and Releases

One installer per platform installs a single desktop application. There is no
background service, no service user, and no admin CLI: the single-process
rewrite removed all three, and this document describes what is actually built.

Two rules govern every path through these installers:

1. **Project data is never destroyed without an explicit answer.** Upgrades
   always preserve it; uninstall leaves it in place.
2. **Every step is idempotent.** Repair and upgrade re-run the same steps.
   Installing over an install, or creating an existing directory, succeeds
   quietly.

---

## 1. Outputs

Built by `.github/workflows/release.yml` on a `v*` tag, and attached to a draft
GitHub release.

| Platform | Artefacts                                                            |
| -------- | -------------------------------------------------------------------- |
| Windows  | `Panel.Platform_<version>_x64-setup.exe` (NSIS), `..._x64_en-US.msi` |
| Linux    | `Panel.Platform_<version>_amd64.deb`, `..._amd64.AppImage`           |
| Setup    | `PanelPlatformSetup.exe`, `panel-platform-setup-x86_64` — see §8     |
| Updates  | `latest.json`, plus a `.sig` beside each updater artefact            |
| Both     | `SHA256SUMS.txt`                                                     |

The NSIS `.exe` is the primary Windows artefact: it handles the WebView2
bootstrap, which is the one dependency a Tauri application cannot supply
itself. The MSI exists for deployment tooling that requires one.

On Linux the `.deb` is the primary artefact. The AppImage is the fallback for
distributions that are not Debian-derived, and is the **only** Linux artefact
that can update itself.

`rpm` is deliberately not built. Nothing has tested it, and shipping an
untested package format is worse than not shipping it.

---

## 2. Release process

1. Bump the version in all four places — `Cargo.toml`, `package.json`,
   `apps/desktop/package.json`, `apps/desktop/src-tauri/tauri.conf.json`.
   `scripts/check-version.sh` verifies they agree and runs in CI.
2. Tag `vX.Y.Z` and push it.
3. The workflow builds each platform, signs the updater artefacts, and opens a
   **draft** release.
4. Check the artefacts, then publish. Nothing reaches a user, and no client
   sees `latest.json`, until that press.

The draft step is the only gate between a green build and everyone's updater.

---

## 3. Windows

### Layout

```
C:\Users\<user>\AppData\Local\Panel Platform\    application
C:\ProgramData\ProjectHost\                      data — preserved across upgrade
    data\  config\  logs\  projects\  backups\  tmp\
```

The application installs per-user; the data directory is machine-wide because
projects outlive the account that created them.

### Install sequence

1. Install or repair the **WebView2 Evergreen runtime**.
2. Write program files, Start Menu entry, optional desktop shortcut.
3. Register the uninstall entry with publisher, version and icon.

The data directory is **not** created by the installer. The application creates
it and applies migrations on first launch, so that a user who has never opened
the application has nothing on disk to clean up.

Docker is not installed and not bundled. The application detects it at runtime
and explains its absence rather than refusing to start.

### Upgrade

Replace program files; leave `ProgramData` untouched; the application applies
pending migrations on next launch. **Running project containers are not
stopped** — Docker keeps them up, and the reconciler adopts them on start.

### Uninstall

Removes program files, shortcuts and the uninstall entry. **`ProgramData` is
left in place**, along with every project, backup and container. Removing a
user's running services during an uninstall of the manager would be
indefensible; the directory is named in the final uninstaller page so it can be
deleted by hand.

---

## 4. Linux

### Layout

```
/usr/bin/project-host-desktop                 binary
/usr/share/applications/panel-platform.desktop
~/.local/share/project-host/                  data, projects, backups, tmp
~/.config/project-host/                       config
~/.local/state/project-host/                  logs
```

Data is per-user, under the XDG base directories and honouring
`XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_STATE_HOME` when they are set to
absolute paths.

> These were `/var/lib`, `/etc` and `/var/log` until the first Linux smoke test
> ran. The installed `.deb` started as an ordinary user and died in under a
> second:
>
> ```
> Panel Platform could not start: could not prepare directories:
> could not create /var/lib/project-host
> ```
>
> Those locations came from the design where a background service owned the
> data and ran as its own service user. That service was deleted in the
> single-process rewrite; the paths were not. Nothing caught it because the
> `.deb` had never been installed and started anywhere.

### Dependencies

`libwebkit2gtk-4.1-0` and `libgtk-3-0`, declared in the `.deb`. Docker is not a
dependency at all — not even `Recommends`. Forcing a particular Docker package
on a user who already runs one is presumptuous, and the application is useful
without it.

### AppImage

Ships the same application with no package manager involved. `chmod +x` and
run. This is the artefact the updater can replace in place.

---

## 5. Updates

`crates/updater` decides **whether** to offer a release: it rejects a version
older than or equal to the installed one, requires `https` from a host on
`ALLOWED_HOSTS`, and requires a signature. `tauri-plugin-updater` performs the
install, verifying against a minisign public key compiled into the binary from
`tauri.conf.json` — never one supplied by the feed.

The private key lives only in the `TAURI_SIGNING_PRIVATE_KEY` repository
secret and in the maintainer's offline copy. **Losing it means no existing
install can ever be updated again**, because clients trust exactly one key.

| Artefact | Can self-update            |
| -------- | -------------------------- |
| NSIS/MSI | Yes — installer takes over |
| AppImage | Yes — replaced in place    |
| `.deb`   | **No** — dpkg owns it      |

Nothing installs without the user pressing the button.

### 5.1 What the update manager can and cannot offer

The window is driven by `screenFor` in `apps/desktop/src/update.ts`, a pure
function of the store's state, and shows twelve states — idle, checking, no
update, ahead of published, available, preparing, downloading, verifying,
installing, restart required, completed, failed. Nothing advances on a timer:
every stage comes from an `update://progress` event the core actually emitted,
and percentage, size, rate and time remaining each render nothing rather than a
guess when the reported data cannot support one.

**There is no pause and no cancel, by design.** `tauri-plugin-updater`
downloads, verifies and installs in a single call that cannot be interrupted or
resumed. Offering either control would mean replacing that call with a
downloader of our own — which would also mean re-implementing minisign
verification against the compiled-in key, in application code, rather than
inheriting it from the plugin. That trades a real security guarantee for a
button, so it is not done. `Minimise` is offered instead: the window gets out of
the way and the update carries on.

`Close` is disabled rather than hidden while an update is in flight. Closing
during a download costs the download; closing during an install can leave a
half-written application.

| Control  | Backed by                              |
| -------- | -------------------------------------- |
| Install  | `install_update` (plugin, verified)    |
| Retry    | the same command, re-entered           |
| Restart  | `restart_app` → `AppHandle::restart()` |
| Minimise | `minimize_window` → `Window::minimize` |
| Close    | window state only, gated on safety     |

The `.deb` case is refused **before** anything is downloaded, and says which
command to run instead. An AppImage sets `APPIMAGE` to its own path when it
runs, so its absence on Linux identifies a packaged install. Left to the
plugin, the same case fails partway through with a complaint about a missing
environment variable — true, and no use to the person reading it.

---

## 6. Signing

| Platform | Mechanism                           | State          |
| -------- | ----------------------------------- | -------------- |
| Windows  | Authenticode over every `.exe`/`.msi` | Signed       |
| Linux    | Detached signature over the `.deb`  | **Not signed** |
| Updates  | Minisign, verified before any write | Signed         |

Unsigned Windows binaries trip SmartScreen and train users to click through
warnings, so the Windows artefacts are signed with **Azure Trusted Signing** —
Microsoft's own service, which issues short-lived certificates against an
identity Microsoft has validated and needs no hardware token on a build
machine.

Three repository *variables* select the account, and three *secrets*
authenticate to it:

| Name                       | Kind     | Example                             |
| -------------------------- | -------- | ----------------------------------- |
| `AZURE_CODESIGN_ENDPOINT`  | variable | `https://weu.codesigning.azure.net` |
| `AZURE_CODESIGN_ACCOUNT`   | variable | the Trusted Signing account name    |
| `AZURE_CODESIGN_PROFILE`   | variable | the certificate profile name        |
| `AZURE_TENANT_ID`          | secret   | the service principal's tenant      |
| `AZURE_CLIENT_ID`          | secret   | the service principal               |
| `AZURE_CLIENT_SECRET`      | secret   | its secret                          |

The service principal needs the **Trusted Signing Certificate Profile Signer**
role on the account; owning the subscription is not sufficient and does not
imply it.

The Tauri bundle is signed through `bundle.windows.signCommand`, written into
`tauri.signing.conf.json` by the release workflow and merged over
`tauri.conf.json` at build time. That file is generated rather than committed
so that a clone with no Azure account still builds: when the variables are
absent the workflow writes nothing, passes no extra `--config`, and produces
the same unsigned artefacts it always did, with a warning in the log saying so.
A release cut that way is not broken — it is the old behaviour, and the
release notes' SmartScreen paragraph still applies to it.

**Signing does not silence SmartScreen on day one.** Azure Trusted Signing
issues an OV-class certificate, and SmartScreen's verdict follows the
reputation that certificate accumulates across downloads, not the mere
presence of a signature. Expect the warning to persist for a while after the
first signed release and then stop. Only an EV certificate suppresses it
immediately; that is the trade this chose against, because EV requires a
hardware token or HSM and costs roughly thirty times as much per year.

---

## 7. Testing

Split by what actually runs, because "tested" covering both a machine check and
a human one is how an untested thing gets called tested.

### 7.1 Automated, in CI

These run in the release workflow, against the artefacts the draft release
carries — downloaded from it, never rebuilt. They must pass before the draft is
fit to publish.

| Test                                                          | Job             |
| ------------------------------------------------------------- | --------------- |
| Setup program resolves, downloads and verifies a real release | `bootstrap`     |
| `.deb` installs with `dpkg -i`, repaired by `apt-get -f`      | `smoke-linux`   |
| Package reports `install ok installed`, binary is executable  | `smoke-linux`   |
| Installed binary starts under `xvfb` and survives 10s         | `smoke-linux`   |
| AppImage is executable, extracts, starts and survives 10s     | `smoke-linux`   |
| NSIS `.exe` installs silently with `/S`                       | `smoke-windows` |
| Installation directory and executable exist                   | `smoke-windows` |
| Installed executable starts and survives 10s                  | `smoke-windows` |
| Uninstaller runs                                              | `smoke-windows` |
| SHA-256 for every attached asset                              | `checksums`     |
| Tag matches the version in all four manifests                 | `check-version` |

**What these prove:** the package installs, the files land, the application
starts, and it does not crash in its first ten seconds.

**What they do not prove:** that any feature works. No window is looked at, no
project is created, no container is started, no data survives an upgrade. A
process that starts and sits there passes every one of these.

### 7.2 Manual, still requiring a human and a VM

None of these have been run. They need machines that do not exist yet, and the
update rows additionally need two published releases.

| Test                                                 | Host                          |
| ---------------------------------------------------- | ----------------------------- |
| Clean install → application opens → project starts   | Windows 11 VM, Docker Desktop |
| Upgrade preserves data and running containers        | Windows VM                    |
| Uninstall leaves `ProgramData` intact                | Windows VM                    |
| `.deb` install → window appears and is usable        | Ubuntu 22.04 + 24.04 VMs      |
| `apt upgrade` preserves data                         | Ubuntu VM                     |
| AppImage self-updates                                | Ubuntu VM                     |
| Update offered, downloaded, verified, installed      | both, two published releases  |
| Install with Docker absent opens with guidance       | both                          |
| Data survives uninstall and is found on reinstall    | both                          |
| Setup program installs end to end, not only verifies | both                          |

---

## 8. The setup program

`crates/setup`. One small binary per platform, attached to the same release as
the installers. A user downloads it, runs it, and it works out which of the
five artefacts their machine needs, downloads that one, proves it came from
Panel Platform, and starts it.

It exists because the download page cannot know which artefact a visitor needs,
and because four of the five choices are wrong for any given reader.

### Sequence

```
check latest release → select asset → confirm → download → verify → hand off
```

| Host                                   | Gets               | Installed by             |
| -------------------------------------- | ------------------ | ------------------------ |
| Windows x64                            | `*_x64-setup.exe`  | The NSIS installer's UI  |
| Linux, `dpkg` **and** `pkexec` present | `*_amd64.deb`      | `pkexec dpkg -i`         |
| Linux, either missing                  | `*_amd64.AppImage` | Placed in `~/.local/bin` |

On Windows and for the `.deb` it adds no install logic of its own — those
installers exist and §7.1 smoke-tests them. The AppImage is the exception,
because no packager owns it, and that path never asks for root.

### What it trusts

The releases it can see come from `releases/latest`, which **excludes drafts**.
That is the behaviour this design wants: a draft is by definition not fit to
install, so the setup program cannot reach one even for the person who built it.

| Check    | Against                             | Defeats                          |
| -------- | ----------------------------------- | -------------------------------- |
| minisign | Public key compiled into the binary | A forged or substituted artefact |
| SHA-256  | `SHA256SUMS.txt` from the release   | Corruption in transit            |

Both are required and the signature is the one that matters: `SHA256SUMS.txt`
travels the same channel as the artefact, so anyone able to substitute one can
substitute the other. It is an integrity check, not an authenticity check.

The public key is read from `tauri.conf.json` by `build.rs`, so the setup
program and the in-app updater trust the same key by construction. A file that
fails either check is deleted rather than run, and nothing is written to disk
until it has passed. There is no override flag.

The setup program is signed like every other Windows artefact here — see §6.
It is signed in the `bootstrap` job *before* it is uploaded, and therefore
before `checksums` hashes it; hashing an unsigned build of a file that ships
signed would break every verification the release notes ask people to run.

### `--silent`

The same pipeline as text, for machines with no display. `--dry-run` stops
after verification without changing anything, which is what the `bootstrap` job
runs.
