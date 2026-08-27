import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  createProject,
  errorMessage,
  listProjects,
  restartProject,
  startProject,
  stopProject,
  systemStatus,
  powerStatus,
  machineLoad,
  recentActivity,
  type ActivityEntry,
  type MachineLoad,
  type PowerStatus,
  type ProjectSummary,
  type SystemStatus,
} from './api';
import { recordRecent } from './lib/recent';
import { isRunning, runControls } from './lib/projects';
import { toRequest, type Draft } from './lib/wizard';
import Activity from './pages/Activity';
import NewProjectWizard, { CreatedSummary } from './pages/NewProjectWizard';
import Overview from './pages/Overview';
import Projects from './pages/Projects';
import {
  applyAppearance,
  defaultAppearance,
  effectFor,
  normaliseAppearance,
} from './lib/appearance';
import ThemeEffects from './components/ThemeEffects';
import Settings, { type Preferences } from './pages/Settings';
import Discord from './pages/Discord';
import ProjectDetail from './pages/ProjectDetail';
import AppShell from './shell/AppShell';
import {
  loadShellLayout,
  saveShellLayout,
  type SectionId,
  type ShellLayout,
} from './shell/shellLayout';
import CommandPalette from './shell/CommandPalette';
import Icon from './ui/Icon';
import { isDeclined, useToolchainGate } from './components/useToolchainGate';
import UpdateManager from './components/UpdateManager';
import { installBusy } from './update';
import { toast, ToastHost } from './ui/toast';
import { updateStore, useUpdate } from './useUpdate';
import type { Command } from './workspace/commands';
import ProjectWorkspace from './workspace/ProjectWorkspace';

/** How often the shell refreshes its view of the core. */
const REFRESH_MS = 5000;

const PREFERENCES_KEY = 'panel.preferences.v1';

const defaultPreferences: Preferences = {
  collapsedSidebar: false,
  confirmDestructive: true,
  appearance: defaultAppearance,
  startupView: 'last',
  notifyStateChanges: true,
  developerMode: false,
};

function loadPreferences(): Preferences {
  try {
    const raw = window.localStorage.getItem(PREFERENCES_KEY);
    if (!raw) return defaultPreferences;
    const parsed: unknown = JSON.parse(raw);
    if (parsed === null || typeof parsed !== 'object') return defaultPreferences;
    const stored = parsed as Partial<Preferences>;
    return {
      collapsedSidebar:
        typeof stored.collapsedSidebar === 'boolean'
          ? stored.collapsedSidebar
          : defaultPreferences.collapsedSidebar,
      confirmDestructive:
        typeof stored.confirmDestructive === 'boolean'
          ? stored.confirmDestructive
          : defaultPreferences.confirmDestructive,
      // Validated rather than trusted: an unknown theme id written to the DOM
      // would match no token block and leave the window unstyled.
      appearance: normaliseAppearance(stored.appearance),
      startupView:
        stored.startupView === 'overview' ||
        stored.startupView === 'projects' ||
        stored.startupView === 'activity' ||
        stored.startupView === 'last'
          ? stored.startupView
          : defaultPreferences.startupView,
      notifyStateChanges:
        typeof stored.notifyStateChanges === 'boolean'
          ? stored.notifyStateChanges
          : defaultPreferences.notifyStateChanges,
      developerMode:
        typeof stored.developerMode === 'boolean'
          ? stored.developerMode
          : defaultPreferences.developerMode,
    };
  } catch {
    return defaultPreferences;
  }
}

/**
 * The application shell.
 *
 * Holds the four things every screen needs — where we are, what the core says,
 * which project is open, and this window's preferences — and nothing else. Each
 * page owns its own data beyond that.
 */
export default function App() {
  /**
   * Where the shell is pointed. Owned here rather than inside `AppShell` so the
   * command palette can move it — a palette that cannot reach the navigation is
   * half a palette.
   */
  const [layout, setLayout] = useState<ShellLayout>(() =>
    loadShellLayout(typeof window === 'undefined' ? undefined : window.localStorage),
  );
  const patchLayout = useCallback((changes: Partial<ShellLayout>) => {
    setLayout((previous) => {
      const next = { ...previous, ...changes };
      saveShellLayout(typeof window === 'undefined' ? undefined : window.localStorage, next);
      return next;
    });
  }, []);
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [power, setPower] = useState<PowerStatus | null>(null);
  /** Drives the status bar's CPU and memory figures. */
  const [load, setLoad] = useState<MachineLoad | null>(null);
  const [projects, setProjects] = useState<ProjectSummary[] | null>(null);
  // Only for Overview's summary. The Activity section reads its own, with
  // filters and paging that a five-row summary has no use for.
  const [activity, setActivity] = useState<ActivityEntry[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  const [openProject, setOpenProject] = useState<string | null>(null);
  /** True once the user asks for the editor rather than the project's pages. */
  const [editing, setEditing] = useState(false);

  const [creating, setCreating] = useState(false);
  const [created, setCreated] = useState<Awaited<ReturnType<typeof createProject>> | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const [preferences, setPreferences] = useState<Preferences>(loadPreferences);

  const update = useUpdate();
  const [updatesOpen, setUpdatesOpen] = useState(false);
  /** Reported once per session, so a poll failure does not toast every 5s. */
  const reportedFailure = useRef(false);

  /**
   * Show the update manager whenever an install owns the application.
   *
   * An install can be started from four places, and one of them — the editor's
   * Help menu — is on a screen that has no room to report progress. Rather than
   * each entry point remembering to open the window, the window opens itself
   * for the only condition that requires it. A *check* deliberately does not
   * trigger this: the periodic one runs every six hours and must stay silent.
   */
  const updateOwnsApp = installBusy(update) || update.phase.state === 'installed';
  useEffect(() => {
    if (updateOwnsApp) setUpdatesOpen(true);
  }, [updateOwnsApp]);

  const openUpdates = useCallback(() => setUpdatesOpen(true), []);

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextProjects, nextPower] = await Promise.all([
        systemStatus(),
        listProjects(),
        // Tolerated separately: the power manager is a background task, and a
        // machine whose sensors will not answer must not cost the user their
        // project list.
        powerStatus().catch(() => null),
      ]);
      machineLoad()
        .then(setLoad)
        .catch(() => setLoad(null));
      // Not awaited with the rest: an audit log that will not answer is worth
      // an empty summary card, never a failed refresh of the project list.
      recentActivity(6)
        .then(setActivity)
        .catch(() => setActivity(null));
      setStatus(nextStatus);
      setProjects(nextProjects);
      setPower(nextPower);
      setFailure(null);
      reportedFailure.current = false;
    } catch (error) {
      const message = errorMessage(error);
      setFailure(message);
      if (!reportedFailure.current) {
        reportedFailure.current = true;
        toast.error('Lost contact with the core', message);
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  // Update checking starts here because this component is mounted for the life
  // of the window. It used to be started by the update banner, which the top
  // bar replaced — deleting that without moving this would have ended periodic
  // checking entirely.
  //
  // A failure to *check* is deliberately silent: being unable to reach GitHub
  // is not worth interrupting anyone over, and the application is designed to
  // run with no internet at all. Settings has an explicit check that reports.
  useEffect(() => {
    updateStore.start();
    // Deliberately not stopped on unmount: the timer lives as long as the
    // window, and tearing it down here would end checking the first time React
    // remounted this in development.
  }, []);

  // Announce a project that changed state on its own — one that fell over, or
  // one the supervisor restarted. Only transitions are reported, never the
  // first load, or opening the window would toast once per running project.
  const lastStatuses = useRef<Map<string, string> | null>(null);
  useEffect(() => {
    if (!projects) return;

    const current = new Map(projects.map((item) => [item.id, item.status]));
    const previous = lastStatuses.current;
    lastStatuses.current = current;

    if (!previous || !preferences.notifyStateChanges) return;

    for (const [id, status] of current) {
      const before = previous.get(id);
      if (before === undefined || before === status) continue;

      const project = projects.find((item) => item.id === id);
      if (!project) continue;

      const label = `${project.displayName} is now ${status.toLowerCase()}`;
      if (status === 'FAILED') toast.error(label, 'It stopped without being asked to.');
      else if (isRunning(status)) toast.success(label);
      else toast.info(label);
    }
  }, [projects, preferences.notifyStateChanges]);

  // Written to the document rather than threaded through the tree: a theme is
  // an attribute the token blocks in `styles.css` respond to, so switching one
  // re-paints without re-rendering anything.
  useEffect(() => {
    applyAppearance(document.documentElement, preferences.appearance);
  }, [preferences.appearance]);

  // The background the current theme asks for, if any. Read here rather than
  // inside the effects layer so the layer stays a renderer with no opinion
  // about which theme is on.
  const effect = useMemo(() => effectFor(preferences.appearance), [preferences.appearance]);

  const patchPreferences = useCallback((next: Partial<Preferences>) => {
    setPreferences((current) => {
      const merged = { ...current, ...next };
      try {
        window.localStorage.setItem(PREFERENCES_KEY, JSON.stringify(merged));
      } catch {
        // A storage that refuses is not worth interrupting anyone over.
      }
      return merged;
    });
  }, []);

  const openProjectById = useCallback((id: string) => {
    setOpenProject(id);
    setEditing(false);
    // Recorded even though nothing reads the list yet in the new shell: the
    // record is what "recently opened" will be built from, and losing it while
    // the surface is rebuilt would mean starting that history from empty.
    recordRecent(window.localStorage, id);
  }, []);

  const project = openProject === null ? null : projects?.find((item) => item.id === openProject);

  // What the rail and the bar both report. Counted from the rows rather than
  // from the load sample, which can be a tick behind a start.
  const runningCount = (projects ?? []).filter((entry) => isRunning(entry.status)).length;

  const { gate, guard } = useToolchainGate();

  /** Everything the palette can run. Each one is a real action of the shell. */

  /**
   * Run one lifecycle action and report it.
   *
   * At component level rather than inside the command list, because the top
   * bar's Run control needs exactly the same behaviour — and two copies of
   * "start it, refresh, toast" is how the palette and the button come to
   * disagree about whether a start succeeded.
   */
  const runAction = useCallback(
    async (item: ProjectSummary, verb: string, action: (id: string) => Promise<unknown>) => {
      try {
        await action(item.id);
        await refresh();
        toast.success(`${item.displayName} ${verb}`);
      } catch (error) {
        // Declining an install is an answer, not a failure to report.
        if (isDeclined(error)) return;
        toast.error(
          `Could not ${verb.replace(/ed$/, '')} ${item.displayName}`,
          errorMessage(error),
        );
      }
    },
    [refresh],
  );

  const commands = useMemo<Command[]>(() => {
    const target = project ?? null;
    // The same decision the Start button makes, from the same function. The
    // palette used to test Docker's availability on its own, which disabled
    // Start for every project on a machine with no Docker while the button
    // beside it worked. Nothing asks about Docker now. `busy` is false because
    // the palette tracks no in-flight action of its own; a project
    // mid-transition is already caught by its status.
    const startBlock =
      target === null
        ? { blocked: true, reason: undefined }
        : runControls(target, {
            busy: false,
          });
    return [
      {
        id: 'project.new',
        title: 'New Project',
        category: 'Projects',
        run: () => setCreating(true),
      },
      ...(
        [
          ['overview', 'Overview'],
          ['projects', 'Projects'],
          ['activity', 'Activity'],
          ['discord', 'Discord'],
          ['settings', 'Settings'],
        ] as [SectionId, string][]
      ).map(([id, title]) => ({
        id: `go.${id}`,
        title: `Go to ${title}`,
        category: 'Go',
        run: () => {
          setOpenProject(null);
          patchLayout({ section: id });
        },
      })),
      {
        id: 'view.sidebar',
        title: 'Collapse Sidebar',
        category: 'View',
        keybinding: 'Ctrl+B',
        run: () => patchLayout({ sidebarCollapsed: !layout.sidebarCollapsed }),
      },
      {
        id: 'app.refresh',
        title: 'Refresh',
        category: 'Application',
        run: () => void refresh(),
      },
      {
        id: 'app.update',
        title: 'Check for Updates',
        category: 'Application',
        run: () => {
          openUpdates();
          void updateStore.check();
        },
      },
      {
        id: 'project.start',
        title: 'Start This Project',
        category: 'Projects',
        enabled: target !== null && !isRunning(target.status) && !startBlock.blocked,
        reason:
          target === null
            ? 'No project is open'
            : (startBlock.reason ?? 'The project is already running'),
        run: () => {
          if (target) void runAction(target, 'started', guard(startProject));
        },
      },
      {
        id: 'project.stop',
        title: 'Stop This Project',
        category: 'Projects',
        enabled: target !== null && isRunning(target.status),
        reason: target === null ? 'No project is open' : 'The project is not running',
        run: () => {
          if (target) void runAction(target, 'stopped', stopProject);
        },
      },
      {
        id: 'project.restart',
        title: 'Restart This Project',
        category: 'Projects',
        enabled: target !== null && isRunning(target.status),
        reason: target === null ? 'No project is open' : 'The project is not running',
        run: () => {
          if (target) void runAction(target, 'restarted', guard(restartProject));
        },
      },
      {
        id: 'project.files',
        title: 'Open Files of This Project',
        category: 'Projects',
        enabled: target !== null,
        reason: 'No project is open',
        run: () => setEditing(true),
      },
    ];
  }, [
    guard,
    runAction,
    patchLayout,
    layout.sidebarCollapsed,
    patchPreferences,
    preferences.collapsedSidebar,
    project,
    refresh,
  ]);

  // The shortcuts that belong to the shell. The editor registers its own while
  // it is mounted, and takes Ctrl+B for its side bar — so these stand down
  // whenever the workspace is what is on screen.
  useEffect(() => {
    if (editing && project) return;

    function onKeyDown(event: KeyboardEvent) {
      const modifier = event.ctrlKey || event.metaKey;
      if (!modifier) return;
      const key = event.key.toLowerCase();

      if (key === 'k' || (key === 'p' && event.shiftKey)) {
        event.preventDefault();
        setPaletteOpen(true);
      } else if (key === 'b') {
        event.preventDefault();
        patchPreferences({ collapsedSidebar: !preferences.collapsedSidebar });
      } else if (key === 'n' && !event.shiftKey) {
        event.preventDefault();
        setCreating(true);
      }
    }

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [editing, patchPreferences, preferences.collapsedSidebar, project]);

  // The editor is a complete shell of its own — its own menu bar, activity bar
  // and status bar. Nesting it inside this one would put two navigation
  // systems on screen at once.
  if (editing && project) {
    return (
      <>
        {/* No effects layer here. The editor fills the window with opaque
            panels of its own, so a canvas behind it would be a frame loop
            nobody could see. */}
        <ProjectWorkspace
          key={project.id}
          project={project}
          status={status}
          onRefreshProjects={refresh}
          onLeave={() => setEditing(false)}
          onOpenSettings={() => {
            setEditing(false);
            patchLayout({ section: 'settings' });
          }}
          onOpenUpdates={openUpdates}
        />
        {/* Rendered in both shells rather than above them: the editor returns
            early, and an update started from its Help menu would otherwise have
            nowhere to report itself. */}
        <UpdateManager
          open={updatesOpen}
          currentVersion={status?.appVersion ?? '—'}
          onClose={() => setUpdatesOpen(false)}
        />
        <ToastHost />
      </>
    );
  }

  return (
    <>
      {/* Before the shell, not inside it: the layer is fixed and the shell is
          lifted above it, so the canvas colour comes from `body` and the effect
          shows through the gaps between panels rather than being painted over
          by the shell's own background. */}
      <ThemeEffects effect={effect} motion={preferences.appearance.motion} />
      <div className="theme-effects-above h-full">
        <AppShell
          section={layout.section}
          projects={projects}
          project={project ?? null}
          running={runningCount}
          failure={failure}
          updateAvailable={update.check?.state === 'available' ? update.check.newVersion : null}
          sidebarCollapsed={layout.sidebarCollapsed}
          onSection={(section) => {
            // Choosing a section leaves whatever project was open: the rail
            // and the detail screen are the same slot, and staying on the
            // project would make the rail lie about where you are.
            setOpenProject(null);
            patchLayout({ section });
          }}
          onOpenProject={openProjectById}
          onNewProject={() => setCreating(true)}
          onRefresh={() => void refresh()}
          onOpenPalette={() => setPaletteOpen(true)}
          onOpenActivity={() => patchLayout({ section: 'activity' })}
          onInstallUpdate={openUpdates}
          onToggleSidebar={() => patchLayout({ sidebarCollapsed: !layout.sidebarCollapsed })}
          overviewPane={
            <Overview
              status={status}
              load={load}
              projects={projects}
              activity={activity}
              onNewProject={() => setCreating(true)}
              onOpenProject={openProjectById}
              onGoProjects={() => patchLayout({ section: 'projects' })}
              onGoActivity={() => patchLayout({ section: 'activity' })}
            />
          }
          detailsPane={
            project ? (
              <ProjectDetail
                key={project.id}
                project={project}
                developerMode={preferences.developerMode}
                onRefreshProjects={refresh}
                onBack={() => setOpenProject(null)}
                onOpenFiles={() => setEditing(true)}
              />
            ) : null
          }
          activityPane={<Activity projects={projects} onOpenProject={openProjectById} />}
          discordPane={<Discord />}
          settingsPane={
            <Settings
              status={status}
              power={power}
              projects={projects}
              preferences={preferences}
              onPreferences={patchPreferences}
              onOpenUpdates={() => {
                openUpdates();
                void updateStore.check();
              }}
              onResetLayout={() => {
                try {
                  window.localStorage.removeItem(PREFERENCES_KEY);
                  window.localStorage.removeItem('workspace.layout.v1');
                  window.localStorage.removeItem('shell.layout.v2');
                  window.localStorage.removeItem('panel.projectsView.v1');
                } catch {
                  // Nothing to do: the defaults apply on the next start anyway.
                }
                setPreferences(defaultPreferences);
              }}
            />
          }
          projectsPane={
            <>
              {created && (
                <div className="mb-4">
                  <CreatedSummary
                    created={created}
                    onDismiss={() => setCreated(null)}
                    onOpen={() => {
                      const id = created.id;
                      setCreated(null);
                      openProjectById(id);
                    }}
                  />
                </div>
              )}
              <Projects
                projects={projects}
                onRefresh={refresh}
                onOpen={openProjectById}
                onNewProject={() => setCreating(true)}
              />
            </>
          }
        />

        {/* A slide-over rather than a tool: activity is something you glance
            at and dismiss, and giving it a rail slot would put a permanent
            entry on screen for an occasional question. */}
        {activityOpen && (
          <div
            role="dialog"
            aria-label="Activity"
            className="fixed inset-0 z-40 flex justify-end bg-black/40"
            onClick={() => setActivityOpen(false)}
          >
            <div
              className="flex h-full w-[420px] max-w-[92vw] flex-col border-l border-edge bg-surface"
              onClick={(event) => event.stopPropagation()}
            >
              <div
                className="flex shrink-0 items-center justify-between border-b border-edge px-3"
                style={{ height: 'var(--h-panel-header)' }}
              >
                <span className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Activity
                </span>
                <button
                  type="button"
                  aria-label="Close activity"
                  onClick={() => setActivityOpen(false)}
                  className="grid h-6 w-6 place-items-center rounded-[3px] text-muted hover:bg-raised hover:text-ink"
                >
                  <Icon name="close" size={13} />
                </button>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto">
                <Activity
                  projects={projects}
                  onOpenProject={(id) => {
                    setActivityOpen(false);
                    openProjectById(id);
                  }}
                />
              </div>
            </div>
          </div>
        )}

        {creating && (
          <NewProjectWizard
            onClose={() => setCreating(false)}
            onCreate={async (draft: Draft) => {
              const result = await createProject(toRequest(draft));
              await refresh();
              return result;
            }}
            onCreated={(result, startNow) => {
              setCreated(result);
              recordRecent(window.localStorage, result.id);
              toast.success(`${result.displayName} created`);
              if (startNow) {
                // A project created on a machine that lacks its language is the
                // most likely place to meet this offer, not the least.
                guard(startProject)(result.id)
                  .then(() => refresh())
                  .then(() => toast.success(`${result.displayName} started`))
                  .catch((error: unknown) => {
                    if (isDeclined(error)) return;
                    toast.error('Could not start the project', errorMessage(error));
                  });
              }
            }}
          />
        )}

        {paletteOpen && (
          <CommandPalette
            commands={commands}
            projects={projects ?? []}
            onOpenProject={openProjectById}
            onClose={() => setPaletteOpen(false)}
          />
        )}

        {gate}

        <UpdateManager
          open={updatesOpen}
          currentVersion={status?.appVersion ?? '—'}
          onClose={() => setUpdatesOpen(false)}
        />

        <ToastHost />
      </div>
    </>
  );
}
