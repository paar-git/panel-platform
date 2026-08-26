/**
 * The project list.
 *
 * Two shapes of the same data: a grid of cards for browsing, and a dense table
 * for scanning many. Which one is remembered, along with the filters, because
 * someone who works in the list view is not choosing it again every morning.
 *
 * The card shows what the core actually knows about a project. Live CPU and
 * memory per project are not among them — that needs Docker's stats stream,
 * which the manager does not read — so the card does not pretend to have them.
 */
import { useEffect, useMemo, useState } from 'react';

import {
  killProject,
  restartProject,
  startProject,
  stopProject,
  type ProjectSummary,
} from '../api';
import {
  applyListOptions,
  defaultListOptions,
  isFiltered,
  runtimeLabel,
  runtimesIn,
  type ListOptions,
  type SortKey,
  type StatusFilter,
  type ViewMode,
} from '../lib/projectList';
import { isRunning, runControls, statusLook } from '../lib/projects';
import { isDeclined, useToolchainGate } from '../components/useToolchainGate';
import ProjectMark from '../ui/ProjectMark';
import Icon from '../ui/Icon';
import { Menu, useMenu } from '../ui/overlays';
import Select from '../ui/Select';
import { Badge, Button, Card, EmptyState, IconButton, Skeleton, TextInput } from '../ui/primitives';
import StatusDot from '../shell/StatusDot';
import { toast } from '../ui/toast';

const VIEW_KEY = 'panel.projectsView.v1';

export default function Projects({
  projects,
  onRefresh,
  onOpen,
  onNewProject,
}: {
  projects: ProjectSummary[] | null;
  onRefresh: () => Promise<void>;
  onOpen: (id: string) => void;
  onNewProject: () => void;
}) {
  const [options, setOptions] = useState<ListOptions>(defaultListOptions);
  const [mode, setMode] = useState<ViewMode>(() =>
    typeof window === 'undefined' || window.localStorage.getItem(VIEW_KEY) === 'list'
      ? 'list'
      : 'grid',
  );
  const [busy, setBusy] = useState<string | null>(null);
  const { gate, guard } = useToolchainGate();

  useEffect(() => {
    try {
      window.localStorage.setItem(VIEW_KEY, mode);
    } catch {
      // A storage that refuses is not worth interrupting anyone over.
    }
  }, [mode]);

  const visible = useMemo(() => applyListOptions(projects ?? [], options), [projects, options]);
  const runtimes = useMemo(() => runtimesIn(projects ?? []), [projects]);

  async function act(
    project: ProjectSummary,
    verb: string,
    action: (id: string) => Promise<unknown>,
  ) {
    setBusy(project.id);
    try {
      // Gated here rather than at each button: the cards, the rows and the
      // context menu all come through this one function, and a check wired to
      // some of them would make the same project behave differently depending
      // on which control was used.
      const gated = action === startProject || action === restartProject ? guard(action) : action;

      await gated(project.id);
      await onRefresh();
      toast.success(`${project.displayName} ${verb}`);
    } catch (error) {
      // Declining an install is an answer, not a failure to report.
      if (!isDeclined(error)) {
        toast.error(
          `Could not ${verb.replace(/ed$/, '')} ${project.displayName}`,
          error instanceof Error ? error.message : String(error),
        );
      }
    } finally {
      setBusy(null);
    }
  }

  const patch = (next: Partial<ListOptions>) => setOptions((current) => ({ ...current, ...next }));

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* A workspace header, not a page banner: 16px title, one line of
          context, and the actions on the same row. */}
      <div
        className="flex shrink-0 items-center gap-2 border-b border-edge px-3"
        style={{ height: 'var(--h-panel-header)' }}
      >
        <h1 className="text-[13px] font-semibold text-ink">Projects</h1>
        <span className="truncate text-[11.5px] text-faint">
          {projects === null ? 'Loading…' : `${projects.length} on this machine`}
        </span>
        <span className="flex-1" />
        <IconButton icon="refresh" label="Refresh" size="sm" onClick={() => void onRefresh()} />
        <button
          type="button"
          onClick={onNewProject}
          className="flex h-[26px] items-center gap-1.5 rounded-[5px] border border-accent/40 bg-accent-soft px-2 text-[12.5px] text-accent hover:brightness-125"
        >
          <Icon name="plus" size={13} />
          New project
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        {projects !== null && projects.length > 0 && (
          <div className="mb-3 flex flex-wrap items-center gap-1.5">
            <div className="min-w-[180px] max-w-[320px] flex-1">
              <TextInput
                value={options.query}
                onChange={(query) => patch({ query })}
                placeholder="Search projects"
              />
            </div>

            <div className="w-[128px]">
              <Select<StatusFilter>
                value={options.status}
                onChange={(status) => patch({ status })}
                options={[
                  { value: 'all', label: 'Any status' },
                  { value: 'running', label: 'Running' },
                  { value: 'stopped', label: 'Stopped' },
                  { value: 'failed', label: 'Failed' },
                ]}
              />
            </div>

            <div className="w-[138px]">
              <Select
                value={options.runtime}
                onChange={(runtime) => patch({ runtime })}
                options={[
                  { value: 'all', label: 'Any runtime' },
                  ...runtimes.map((id) => ({ value: id, label: runtimeLabel(id) })),
                ]}
              />
            </div>

            <div className="w-[138px]">
              <Select<SortKey>
                value={options.sort}
                onChange={(sort) => patch({ sort })}
                options={[
                  { value: 'name', label: 'Sort by name' },
                  { value: 'status', label: 'Sort by status' },
                  { value: 'runtime', label: 'Sort by runtime' },
                ]}
              />
            </div>

            <div className="flex gap-0.5 rounded-[5px] border border-edge bg-raised p-0.5">
              <IconButton
                icon="grid"
                label="Grid view"
                size="sm"
                active={mode === 'grid'}
                onClick={() => setMode('grid')}
              />
              <IconButton
                icon="list"
                label="List view"
                size="sm"
                active={mode === 'list'}
                onClick={() => setMode('list')}
              />
            </div>
          </div>
        )}

        {projects === null && (
          <div className="grid gap-2.5 [grid-template-columns:repeat(auto-fill,minmax(260px,300px))]">
            <Skeleton className="h-[132px]" />
            <Skeleton className="h-[132px]" />
            <Skeleton className="h-[132px]" />
          </div>
        )}

        {projects !== null && projects.length === 0 && (
          <Card>
            <EmptyState
              icon="projects"
              title="No projects yet"
              description="A project is a folder on this machine that Panel Platform builds, runs and keeps running — a bot, an API, a website, or anything else with a start command."
              actions={
                <>
                  <Button variant="primary" icon="plus" onClick={onNewProject}>
                    Create project
                  </Button>
                  <Button icon="git" onClick={onNewProject}>
                    Clone repository
                  </Button>
                  <Button icon="download" onClick={onNewProject}>
                    From archive URL
                  </Button>
                </>
              }
            >
              <div className="mt-6 flex flex-wrap justify-center gap-1.5">
                {['Node.js', 'Python', 'Go', 'Rust', 'Static site', 'Dockerfile'].map((example) => (
                  <span
                    key={example}
                    className="rounded-full border border-edge px-2.5 py-1 text-[12px] text-muted"
                  >
                    {example}
                  </span>
                ))}
              </div>
            </EmptyState>
          </Card>
        )}

        {projects !== null && projects.length > 0 && visible.length === 0 && (
          <Card>
            <EmptyState
              icon="search"
              title="Nothing matches"
              description={
                isFiltered(options)
                  ? 'No project matches the current search and filters.'
                  : 'There is nothing to show.'
              }
              actions={
                <Button onClick={() => setOptions(defaultListOptions)}>Clear filters</Button>
              }
            />
          </Card>
        )}

        {visible.length > 0 &&
          (mode === 'grid' ? (
            <div className="stagger grid gap-2.5 [grid-template-columns:repeat(auto-fill,minmax(260px,300px))]">
              {visible.map((project) => (
                <ProjectCard
                  key={project.id}
                  project={project}
                  busy={busy === project.id}
                  onOpen={() => onOpen(project.id)}
                  onAct={act}
                />
              ))}
            </div>
          ) : (
            <Card className="overflow-hidden">
              <ul className="stagger">
                {visible.map((project) => (
                  <ProjectRow
                    key={project.id}
                    project={project}
                    busy={busy === project.id}
                    onOpen={() => onOpen(project.id)}
                    onAct={act}
                  />
                ))}
              </ul>
            </Card>
          ))}

        {gate}
      </div>
    </div>
  );
}

type ActFn = (
  project: ProjectSummary,
  verb: string,
  action: (id: string) => Promise<unknown>,
) => Promise<void>;

/**
 * Whether a project's controls should be usable.
 *
 * A thin wrapper over `runControls` so the list and the detail page cannot
 * disagree about when a button is live.
 */
function useControls(project: ProjectSummary, busy: boolean) {
  const { blocked, reason } = runControls(project, { busy });
  return { look: statusLook(project.status), running: isRunning(project.status), blocked, reason };
}

function ProjectCard({
  project,
  busy,
  onOpen,
  onAct,
}: {
  project: ProjectSummary;
  busy: boolean;
  onOpen: () => void;
  onAct: ActFn;
}) {
  const { look, running, blocked, reason } = useControls(project, busy);
  const menu = useMenu();

  return (
    <div className="flex h-[150px] flex-col rounded-[7px] border border-edge bg-surface transition-colors duration-100 hover:border-edge-strong">
      {/* Identity. The mark and the name are the row a person scans; the
          runtime and the state are the line underneath it. */}
      <div className="flex items-start gap-2.5 p-3">
        <ProjectMark projectId={project.id} runtime={project.projectType} size={26} />

        <button type="button" onClick={onOpen} className="min-w-0 flex-1 text-left">
          <span className="block truncate text-[13px] font-medium text-ink">
            {project.displayName}
          </span>
          <span className="mt-0.5 block truncate text-[11.5px] text-faint">
            {runtimeLabel(project.projectType)}
          </span>
        </button>

        <IconButton icon="more" label="More actions" size="sm" onClick={menu.open} />
      </div>

      {/* State. Never colour alone — the word is always there too. */}
      <div className="flex min-w-0 items-center gap-2 px-3 text-[12px]">
        <StatusDot status={project.status} />
        <span className="truncate text-muted">{look.label}</span>
        {project.desiredState.toUpperCase() === 'RUNNING' && !running && (
          <span className="shrink-0 text-[11px] text-warn">wants to run</span>
        )}
      </div>

      <div className="mt-auto flex items-center gap-1.5 border-t border-edge px-2 py-2">
        <button
          type="button"
          disabled={blocked}
          title={reason ?? (running ? 'Stop this project' : 'Run this project')}
          onClick={() =>
            running
              ? void onAct(project, 'stopped', stopProject)
              : void onAct(project, 'started', startProject)
          }
          className={`flex h-[26px] items-center gap-1.5 rounded-[5px] border px-2 text-[12.5px] disabled:cursor-not-allowed disabled:opacity-55 ${
            running
              ? 'border-edge-strong bg-raised text-ink hover:bg-overlay'
              : 'border-ok/40 bg-ok-soft text-ok hover:brightness-125'
          }`}
        >
          <Icon name={running ? 'stop' : 'play'} size={13} />
          {running ? 'Stop' : 'Run'}
        </button>

        {running && (
          <button
            type="button"
            disabled={blocked}
            title={reason ?? 'Restart this project'}
            onClick={() => void onAct(project, 'restarted', restartProject)}
            className="grid h-[26px] w-[26px] place-items-center rounded-[5px] border border-edge bg-raised text-muted hover:text-ink disabled:cursor-not-allowed disabled:opacity-55"
          >
            <Icon name="restart" size={13} />
          </button>
        )}

        <span className="flex-1" />

        <button
          type="button"
          onClick={onOpen}
          className="h-[26px] rounded-[5px] px-2 text-[12.5px] text-muted hover:bg-raised hover:text-ink"
        >
          Open
        </button>
      </div>

      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          items={projectMenuItems(project, { running, blocked, reason }, onOpen, onAct)}
        />
      )}
    </div>
  );
}

function ProjectRow({
  project,
  busy,
  onOpen,
  onAct,
}: {
  project: ProjectSummary;
  busy: boolean;
  onOpen: () => void;
  onAct: ActFn;
}) {
  const { look, running, blocked, reason } = useControls(project, busy);
  const menu = useMenu();

  return (
    <li className="flex items-center gap-3 border-b border-edge px-3 py-2 last:border-b-0 hover:bg-raised/50">
      <ProjectMark projectId={project.id} runtime={project.projectType} size={28} />

      <button type="button" onClick={onOpen} className="min-w-0 flex-[2] text-left">
        <span className="block truncate text-[13px] font-medium text-ink hover:text-accent">
          {project.displayName}
        </span>
        <span className="block truncate text-[12px] text-muted">
          {project.description || project.slug}
        </span>
      </button>

      <span className="hidden min-w-0 flex-1 truncate text-[12px] text-muted md:block">
        {runtimeLabel(project.projectType)}
      </span>

      <Badge tone={look.tone} dot>
        {look.label}
      </Badge>

      <div className="flex shrink-0 items-center gap-1">
        {running ? (
          <IconButton
            icon="stop"
            label={reason ?? 'Stop'}
            size="sm"
            disabled={blocked}
            onClick={() => void onAct(project, 'stopped', stopProject)}
          />
        ) : (
          <IconButton
            icon="play"
            label={reason ?? 'Start'}
            size="sm"
            disabled={blocked}
            onClick={() => void onAct(project, 'started', startProject)}
          />
        )}
        <IconButton
          icon="restart"
          label={reason ?? 'Restart'}
          size="sm"
          disabled={blocked || !running}
          onClick={() => void onAct(project, 'restarted', restartProject)}
        />
        <IconButton icon="more" label="More actions" size="sm" onClick={menu.open} />
      </div>

      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          items={projectMenuItems(project, { running, blocked, reason }, onOpen, onAct)}
        />
      )}
    </li>
  );
}

function projectMenuItems(
  project: ProjectSummary,
  state: { running: boolean; blocked: boolean; reason?: string },
  onOpen: () => void,
  onAct: ActFn,
) {
  return [
    { id: 'open', label: 'Open project', icon: 'external' as const, run: onOpen },
    {
      id: 'restart',
      label: 'Restart',
      icon: 'restart' as const,
      disabled: state.blocked || !state.running,
      reason: state.reason ?? 'The project is not running',
      run: () => void onAct(project, 'restarted', restartProject),
    },
    {
      id: 'kill',
      label: 'Force kill',
      icon: 'power' as const,
      danger: true,
      disabled: state.blocked || !state.running,
      reason: state.reason ?? 'The project is not running',
      run: () => void onAct(project, 'killed', killProject),
    },
  ];
}
