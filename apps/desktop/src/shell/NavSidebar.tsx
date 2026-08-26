/**
 * The navigation rail.
 *
 * A fixed column of places rather than a rail of tools: the product has five
 * sections and a list of recent projects, and a person arriving at it should
 * be able to see all six at once rather than discover them by clicking icons.
 *
 * It states where you are and nothing else. Every action it offers belongs to
 * `App`, which owns the polling and the run actions — this file decides what
 * is on screen, not what is true.
 */
import type { ProjectSummary } from '../api';
import { Button } from '../ui/primitives';
import ProjectMark from '../ui/ProjectMark';
import StatusDot from './StatusDot';

/** The sections the rail can be on. */
export type SectionId = 'overview' | 'projects' | 'activity' | 'discord' | 'settings';

const SECTIONS: readonly { id: SectionId; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'projects', label: 'Projects' },
  { id: 'activity', label: 'Activity' },
  { id: 'discord', label: 'Discord' },
  { id: 'settings', label: 'Settings' },
];

export default function NavSidebar({
  section,
  projects,
  openProject,
  running,
  collapsed,
  onSection,
  onOpenProject,
  onNewProject,
  onToggleCollapsed,
}: {
  section: SectionId;
  projects: ProjectSummary[] | null;
  /** The project whose detail screen is open, if any. */
  openProject: ProjectSummary | null;
  /** How many projects have something running. Shown as a live count. */
  running: number;
  collapsed: boolean;
  onSection: (section: SectionId) => void;
  onOpenProject: (id: string) => void;
  onNewProject: () => void;
  onToggleCollapsed: () => void;
}) {
  // The four most recently touched, which is what "recent" can honestly mean
  // without a separate record of what was opened when.
  const recent = (projects ?? []).slice(0, 4);

  if (collapsed) {
    return (
      <nav
        aria-label="Sections"
        className="sidebar-surface flex w-[52px] shrink-0 flex-col items-center gap-1 py-3"
      >
        <button
          type="button"
          onClick={onNewProject}
          title="New project"
          aria-label="New project"
          className="grid h-[26px] w-[26px] place-items-center rounded-[8px] bg-accent text-[13px] font-bold text-canvas"
        >
          +
        </button>
        <div className="mt-2 flex flex-col gap-1">
          {SECTIONS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              onClick={() => onSection(entry.id)}
              title={entry.label}
              aria-label={entry.label}
              aria-current={section === entry.id ? 'page' : undefined}
              className={`h-[30px] w-[30px] rounded-[8px] text-[11px] ${
                section === entry.id ? 'bg-raised text-ink' : 'text-faint hover:bg-raised'
              }`}
            >
              {entry.label.charAt(0)}
            </button>
          ))}
        </div>
        <div className="flex-1" />
        <button
          type="button"
          onClick={onToggleCollapsed}
          title="Expand the sidebar"
          aria-label="Expand the sidebar"
          className="h-[26px] w-[26px] rounded-[8px] text-faint hover:bg-raised"
        >
          ›
        </button>
      </nav>
    );
  }

  return (
    <nav aria-label="Sections" className="sidebar-surface flex w-[216px] shrink-0 flex-col">
      <div className="flex h-[52px] shrink-0 items-center gap-2.5 px-3">
        <div className="grid h-[26px] w-[26px] place-items-center rounded-[8px] bg-accent text-[13px] font-bold text-canvas">
          P
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-semibold tracking-[-0.01em]">
            Panel Platform
          </div>
          <div className="-mt-px truncate text-[11px] text-faint">Project workspace</div>
        </div>
      </div>

      <div className="px-2 pb-2.5">
        <Button variant="primary" full onClick={onNewProject}>
          + New project
        </Button>
      </div>

      <div className="flex flex-col gap-px px-2">
        {SECTIONS.map((entry) => {
          const active = section === entry.id;
          return (
            <button
              key={entry.id}
              type="button"
              onClick={() => onSection(entry.id)}
              aria-current={active ? 'page' : undefined}
              className={`flex h-[30px] items-center rounded-[8px] px-2.5 text-left text-[12.5px] transition-colors ${
                active ? 'bg-raised font-medium text-ink' : 'text-muted hover:bg-raised'
              }`}
            >
              {entry.label}
              {entry.id === 'projects' && projects !== null && (
                <span className="tabular ml-auto rounded-full bg-overlay px-1.5 py-px text-[11px] text-faint">
                  {projects.length}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {recent.length > 0 && (
        <>
          <div className="mx-2 mt-3.5 mb-1.5 px-2 text-[10.5px] font-semibold tracking-[0.12em] text-faint">
            RECENT
          </div>
          <div className="flex flex-col gap-px px-2">
            {recent.map((entry) => {
              const active = openProject?.id === entry.id;
              return (
                <button
                  key={entry.id}
                  type="button"
                  onClick={() => onOpenProject(entry.id)}
                  aria-current={active ? 'page' : undefined}
                  className={`flex h-[30px] items-center gap-2 rounded-[8px] px-2 text-left text-[12.5px] transition-colors ${
                    active ? 'bg-raised text-ink' : 'text-muted hover:bg-raised'
                  }`}
                >
                  <ProjectMark projectId={entry.id} runtime={entry.projectType} size={18} />
                  <span className="min-w-0 flex-1 truncate">{entry.displayName}</span>
                  <StatusDot status={entry.status} />
                </button>
              );
            })}
          </div>
        </>
      )}

      <div className="flex-1" />

      <div className="flex flex-col gap-2 border-t border-edge p-2.5">
        <div className="flex items-center gap-2 px-2 text-[11.5px] text-faint">
          <span
            aria-hidden
            className={`h-1.5 w-1.5 rounded-full ${running > 0 ? 'bg-ok' : 'bg-faint'}`}
          />
          {running === 0
            ? 'Nothing running'
            : `${running} ${running === 1 ? 'project' : 'projects'} running`}
        </div>
        <button
          type="button"
          onClick={onToggleCollapsed}
          className="flex items-center gap-2 rounded-[8px] px-2 py-1 text-left text-[11.5px] text-faint hover:bg-raised hover:text-muted"
        >
          ‹ Collapse
        </button>
      </div>
    </nav>
  );
}
