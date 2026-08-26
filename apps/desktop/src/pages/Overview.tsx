/**
 * How this machine and its projects are doing.
 *
 * The first screen, and the only one that answers a question rather than
 * listing a thing: is anything wrong, and if so where. Everything on it is
 * either a number the core measured or a row the core wrote — there is no
 * summary here that the sections below could contradict.
 *
 * "Needs attention" is the part worth being careful with. It reports the
 * projects the core has recorded as crashed, failed or unhealthy, and says
 * nothing at all when there are none. An empty state that says "everything is
 * behaving" is only honest if the check behind it is real.
 */
import type { ActivityEntry, MachineLoad, ProjectSummary, SystemStatus } from '../api';
import { formatBytes, formatDuration, formatRelative } from '../lib/format';
import { isRunning, statusLook } from '../lib/projects';
import { Button, Card, CardHeader, DataRow, Meter, Stat } from '../ui/primitives';
import ProjectMark from '../ui/ProjectMark';
import StatusDot from '../shell/StatusDot';

/** The statuses that mean somebody should look at this project. */
function needsAttention(project: ProjectSummary): boolean {
  return ['CRASHED', 'FAILED', 'UNHEALTHY'].includes(project.status.toUpperCase());
}

export default function Overview({
  status,
  load,
  projects,
  activity,
  onNewProject,
  onOpenProject,
  onGoProjects,
  onGoActivity,
}: {
  status: SystemStatus | null;
  load: MachineLoad | null;
  projects: ProjectSummary[] | null;
  activity: ActivityEntry[] | null;
  onNewProject: () => void;
  onOpenProject: (id: string) => void;
  onGoProjects: () => void;
  onGoActivity: () => void;
}) {
  const all = projects ?? [];
  const running = all.filter((project) => isRunning(project.status)).length;
  const stopped = all.filter((project) => project.status.toUpperCase() === 'STOPPED').length;
  const troubled = all.filter(needsAttention);

  const memoryUsed =
    load === null ? 0 : Math.max(0, load.totalMemoryBytes - load.availableMemoryBytes);
  const memoryPercent =
    load === null || load.totalMemoryBytes === 0 ? 0 : (memoryUsed / load.totalMemoryBytes) * 100;

  return (
    <div className="flex max-w-[940px] flex-col gap-[18px]">
      <div className="flex items-end justify-between gap-4">
        <div>
          <h1 className="text-[28px] leading-tight font-semibold tracking-[-0.025em]">Overview</h1>
          <p className="mt-0.5 text-[13px] text-muted">
            How this machine and its projects are doing.
          </p>
        </div>
        <Button variant="primary" onClick={onNewProject}>
          + New project
        </Button>
      </div>

      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        <Stat label="Projects" value={projects === null ? '—' : all.length} />
        <Stat
          label="Running"
          value={projects === null ? '—' : running}
          tone={running > 0 ? 'ok' : 'neutral'}
        />
        <Stat label="Stopped" value={projects === null ? '—' : stopped} />
        <Stat
          label="Failed"
          value={projects === null ? '—' : troubled.length}
          tone={troubled.length > 0 ? 'danger' : 'neutral'}
        />
      </div>

      <div className="grid items-start gap-3.5 lg:grid-cols-[1.35fr_1fr]">
        <Card>
          <CardHeader
            title="This machine"
            subtitle={
              load === null ? 'not sampled yet' : `${load.logicalCores} cores · sampled every 4s`
            }
          />
          <div className="grid grid-cols-1 gap-4 p-4 sm:grid-cols-2">
            <Meter
              label="CPU"
              value={load?.cpuPercent ?? 0}
              unknown={load === null || load.cpuPercent === null}
              caption={load?.cpuPercent === null ? undefined : 'across all cores'}
            />
            <Meter
              label="Memory"
              value={memoryPercent}
              unknown={load === null || !load.measured}
              caption={
                load === null
                  ? undefined
                  : `${formatBytes(memoryUsed)} / ${formatBytes(load.totalMemoryBytes)}`
              }
            />
          </div>
          <div className="border-t border-edge">
            <DataRow
              label="Application uptime"
              value={status === null ? '—' : formatDuration(status.uptimeSeconds)}
              mono
            />
            <DataRow label="Version" value={status?.appVersion ?? '—'} mono />
            <DataRow
              label="Headroom"
              value={load === null ? '—' : formatBytes(load.headroomBytes)}
              mono
              hint="What is left after the reserve this machine keeps for itself."
            />
          </div>
        </Card>

        <div className="flex flex-col gap-3.5">
          <Card>
            <CardHeader title="Needs attention" />
            {troubled.length === 0 ? (
              <p className="flex items-center gap-2 px-4 py-3.5 text-[13px] text-ok">
                <span
                  aria-hidden
                  className="grid h-4 w-4 place-items-center rounded-full bg-ok/15 text-[10px]"
                >
                  ✓
                </span>
                Everything is behaving.
              </p>
            ) : (
              <ul>
                {troubled.map((project) => (
                  <li key={project.id}>
                    <button
                      type="button"
                      onClick={() => onOpenProject(project.id)}
                      className="flex w-full items-center gap-2.5 px-4 py-2.5 text-left transition-colors hover:bg-raised"
                    >
                      <StatusDot status={project.status} />
                      <span className="min-w-0 flex-1 truncate text-[13px]">
                        {project.displayName}
                      </span>
                      <span className="text-[12px] text-danger">
                        {statusLook(project.status).label}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card>
            <CardHeader
              title="Recently opened"
              actions={
                <button
                  type="button"
                  onClick={onGoProjects}
                  className="text-[12px] text-accent hover:underline"
                >
                  All projects
                </button>
              }
            />
            {all.length === 0 ? (
              <p className="px-4 py-3.5 text-[12.5px] text-faint">
                No projects yet. The button above makes one.
              </p>
            ) : (
              <ul>
                {all.slice(0, 3).map((project) => (
                  <li key={project.id}>
                    <button
                      type="button"
                      onClick={() => onOpenProject(project.id)}
                      className="flex w-full items-center gap-2.5 px-4 py-2.5 text-left transition-colors hover:bg-raised"
                    >
                      <ProjectMark projectId={project.id} runtime={project.projectType} size={26} />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[13px]">{project.displayName}</span>
                        <span className="block truncate font-mono text-[11px] text-faint">
                          {project.slug}
                        </span>
                      </span>
                      <StatusDot status={project.status} />
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card>
            <CardHeader
              title="Recent activity"
              actions={
                <button
                  type="button"
                  onClick={onGoActivity}
                  className="text-[12px] text-accent hover:underline"
                >
                  View all
                </button>
              }
            />
            {activity === null || activity.length === 0 ? (
              <p className="px-4 py-3.5 text-[12.5px] text-faint">Nothing has happened yet.</p>
            ) : (
              <ul>
                {activity.slice(0, 4).map((entry) => (
                  <li
                    key={entry.id}
                    className="flex items-baseline gap-3 px-4 py-2.5 text-[12.5px]"
                  >
                    <span className="min-w-0 flex-1 truncate">
                      {entry.action}
                      {entry.targetLabel !== null && (
                        <span className="text-faint"> · {entry.targetLabel}</span>
                      )}
                    </span>
                    <span className="shrink-0 text-[11px] text-faint">
                      {formatRelative(entry.occurredAt)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
}
