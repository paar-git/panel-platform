/**
 * The 48px strip across the top.
 *
 * Three zones and no wasted space: identity and the project on the left, search
 * in the middle, the run control and window actions on the right. The run
 * control sits here rather than on the project page because it is the thing a
 * user reaches for most, and it must be in the same place whatever the window
 * is showing.
 */
import { type ProjectSummary, type SystemStatus } from '../api';
import Icon from '../ui/Icon';
import Logo from '../ui/Logo';
import ProjectSwitcher from './ProjectSwitcher';
import RunControl from './RunControl';

export default function WorkspaceTopBar({
  status,
  projects,
  project,
  busy,
  updateAvailable,
  onOpenPalette,
  onOpenProject,
  onBrowseProjects,
  onStart,
  onStop,
  onRestart,
  onOpenConsole,
  onConfigureRun,
  onOpenActivity,
  onInstallUpdate,
}: {
  status: SystemStatus | null;
  projects: ProjectSummary[] | null;
  project: ProjectSummary | null;
  busy: boolean;
  updateAvailable: string | null;
  onOpenPalette: () => void;
  onOpenProject: (id: string) => void;
  onBrowseProjects: () => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onOpenConsole: () => void;
  onConfigureRun: () => void;
  onOpenActivity: () => void;
  onInstallUpdate: () => void;
}) {
  return (
    <header
      className="flex shrink-0 items-center gap-2 border-b border-edge bg-surface px-2"
      style={{ height: 'var(--h-topbar)' }}
    >
      {/* ------------------------------------------------------------ left */}
      <div className="flex min-w-0 items-center gap-1.5">
        <span className="grid h-7 w-7 shrink-0 place-items-center">
          <Logo size={20} />
        </span>
        <span className="hidden shrink-0 text-[12.5px] font-medium text-muted lg:inline">
          Panel
        </span>
        {project !== null && (
          <span aria-hidden className="shrink-0 text-faint">
            /
          </span>
        )}
        <ProjectSwitcher
          projects={projects}
          current={project}
          onOpen={onOpenProject}
          onBrowse={onBrowseProjects}
        />
      </div>

      {/* ---------------------------------------------------------- centre */}
      <div className="flex min-w-0 flex-1 justify-center px-2">
        <button
          type="button"
          onClick={onOpenPalette}
          title="Search projects and commands (Ctrl+K)"
          className="flex h-[28px] w-full max-w-[420px] min-w-0 items-center gap-2 rounded-[5px] border border-edge bg-canvas px-2 text-[12.5px] text-faint hover:border-edge-strong hover:text-muted"
        >
          <Icon name="search" size={13} />
          <span className="min-w-0 flex-1 truncate text-left">
            Search files, projects, commands…
          </span>
          <kbd className="hidden shrink-0 rounded-[3px] border border-edge px-1 text-[10px] text-faint md:inline">
            Ctrl K
          </kbd>
        </button>
      </div>

      {/* ----------------------------------------------------------- right */}
      <div className="flex shrink-0 items-center gap-1.5">
        <RunControl
          project={project}
          busy={busy}
          onStart={onStart}
          onStop={onStop}
          onRestart={onRestart}
          onOpenConsole={onOpenConsole}
          onConfigure={onConfigureRun}
        />

        {updateAvailable !== null && (
          <button
            type="button"
            onClick={onInstallUpdate}
            title={`Version ${updateAvailable} is available`}
            className="h-[26px] rounded-[5px] border border-accent/40 bg-accent-soft px-2 text-[12px] text-accent hover:brightness-125"
          >
            Update
          </button>
        )}

        <button
          type="button"
          onClick={onOpenActivity}
          aria-label="Activity"
          title="Activity"
          className="grid h-[28px] w-[28px] place-items-center rounded-[5px] text-muted hover:bg-raised hover:text-ink"
        >
          <Icon name="bell" size={15} />
        </button>

        <span
          className="hidden shrink-0 px-1 text-[11px] text-faint lg:inline"
          title={status === null ? 'Connecting…' : `Panel ${status.appVersion}`}
        >
          {status?.appVersion ?? '—'}
        </span>
      </div>
    </header>
  );
}
