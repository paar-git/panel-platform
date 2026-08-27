/**
 * The application's frame.
 *
 * A navigation rail, a bar, and one section at a time. The rail lists places
 * rather than tools, which is the whole difference between this shell and the
 * editor's: here you are somewhere, and there you are working on something.
 *
 * State lives in `App`, which owns the polling and the run actions. This
 * component decides what is on screen, not what is true — which is why the
 * sections can be rearranged without touching anything that talks to the core.
 */
import type { ProjectSummary } from '../api';
import NavSidebar, { type SectionId } from './NavSidebar';
import TopBar from './TopBar';

export type { SectionId };

export default function AppShell({
  section,
  projects,
  project,
  running,
  failure,
  updateAvailable,
  sidebarCollapsed,
  overviewPane,
  projectsPane,
  detailsPane,
  activityPane,
  discordPane,
  settingsPane,
  onSection,
  onOpenProject,
  onNewProject,
  onRefresh,
  onOpenPalette,
  onOpenActivity,
  onInstallUpdate,
  onToggleSidebar,
}: {
  section: SectionId;
  projects: ProjectSummary[] | null;
  /** The project whose detail screen is open. Non-null means the detail
   *  screen replaces whatever section the rail is on. */
  project: ProjectSummary | null;
  /** How many projects have something up. */
  running: number;
  /** The last thing that went wrong, if the core is unreachable. */
  failure: string | null;
  updateAvailable: string | null;
  sidebarCollapsed: boolean;

  overviewPane: React.ReactNode;
  projectsPane: React.ReactNode;
  /** The open project's full record. */
  detailsPane: React.ReactNode;
  activityPane: React.ReactNode;
  discordPane: React.ReactNode;
  settingsPane: React.ReactNode;

  onSection: (section: SectionId) => void;
  onOpenProject: (id: string) => void;
  onNewProject: () => void;
  onRefresh: () => void;
  onOpenPalette: () => void;
  onOpenActivity: () => void;
  onInstallUpdate: () => void;
  onToggleSidebar: () => void;
}) {
  // A project being open wins over the rail's section: opening one is a
  // navigation, and the rail highlights the project rather than a section.
  const body = project
    ? detailsPane
    : section === 'overview'
      ? overviewPane
      : section === 'projects'
        ? projectsPane
        : section === 'activity'
          ? activityPane
          : section === 'discord'
            ? discordPane
            : settingsPane;

  return (
    <div className="flex h-full min-h-0 bg-canvas text-ink">
      <NavSidebar
        section={section}
        projects={projects}
        openProject={project}
        running={running}
        collapsed={sidebarCollapsed}
        onSection={onSection}
        onOpenProject={onOpenProject}
        onNewProject={onNewProject}
        onToggleCollapsed={onToggleSidebar}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <TopBar
          running={running}
          updateAvailable={updateAvailable}
          onOpenPalette={onOpenPalette}
          onInstallUpdate={onInstallUpdate}
          onOpenActivity={onOpenActivity}
          onRefresh={onRefresh}
        />

        {failure !== null && (
          <div
            role="alert"
            className="flex items-center gap-3 border-b border-danger/30 bg-danger/10 px-4 py-2.5 text-[12.5px] text-danger"
          >
            <span aria-hidden>!</span>
            <span className="min-w-0 flex-1">{failure}</span>
            <button type="button" onClick={onRefresh} className="shrink-0 hover:underline">
              Try again
            </button>
          </div>
        )}

        <main className="min-h-0 flex-1 overflow-auto px-[30px] pt-[26px] pb-[30px]">{body}</main>
      </div>
    </div>
  );
}
