/**
 * One project, in depth.
 *
 * The tabs follow what the core can actually answer. Overview, Deployments,
 * History, Networking, Environment, Resources and Settings are all reading real
 * rows the database already keeps; Files hands over to the editor workspace.
 *
 * Two tabs a hosting panel would normally have are absent rather than empty:
 * there is no Console because nothing in the core executes a command inside a
 * running project, and no Logs because nothing collects a project's output. A
 * tab that opens onto an apology is worse than a tab that is not there — but
 * the Overview says both plainly, so the absence is not a mystery either.
 */
import { useCallback, useEffect, useState } from 'react';

import {
  errorMessage,
  setProjectPower,
  type ProjectPriority,
  killProject,
  projectDeployments,
  projectDetails,
  projectEvents,
  recentActivity,
  restartProject,
  revealProjectPath,
  startProject,
  stopProject,
  type ActivityEntry,
  type ContainerEvent,
  type DeploymentSummary,
  type ProjectDetail as Detail,
  type ProjectSummary,
} from '../api';
import {
  baseName,
  formatBytes,
  formatDuration,
  formatElapsed,
  formatRelative,
  formatTimestamp,
  uptimeSeconds,
} from '../lib/format';
import { runtimeLabel } from '../lib/projectList';
import { describeAction, healthLook, isRunning, runControls, statusLook } from '../lib/projects';
import { isDeclined, useToolchainGate } from '../components/useToolchainGate';
import ProjectMark from '../ui/ProjectMark';
import Icon from '../ui/Icon';
import { ConfirmDialog } from '../ui/overlays';
import {
  Badge,
  Button,
  Card,
  CardHeader,
  DataRow,
  EmptyState,
  IconButton,
  Skeleton,
  Tabs,
  Toggle,
} from '../ui/primitives';
import { toast } from '../ui/toast';
import ProjectConsole from '../components/ProjectConsole';
import Select from '../ui/Select';

type TabId =
  | 'overview'
  | 'console'
  | 'deployments'
  | 'history'
  | 'networking'
  | 'environment'
  | 'resources'
  | 'settings';

export default function ProjectDetail({
  project,
  developerMode = false,
  onRefreshProjects,
  onBack,
  onOpenFiles,
}: {
  project: ProjectSummary;
  /** Shows the internal identifiers a bug report needs. */
  developerMode?: boolean;
  onRefreshProjects: () => Promise<void>;
  onBack: () => void;
  onOpenFiles: () => void;
}) {
  const [tab, setTab] = useState<TabId>('overview');
  const [detail, setDetail] = useState<Detail | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [deployments, setDeployments] = useState<DeploymentSummary[] | null>(null);
  const [events, setEvents] = useState<ContainerEvent[] | null>(null);
  const [activity, setActivity] = useState<ActivityEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmKill, setConfirmKill] = useState(false);
  const { gate, guard } = useToolchainGate();
  /** Re-rendered on a timer so the uptime counts up rather than freezing. */
  const [, setTick] = useState(0);

  const load = useCallback(() => {
    projectDetails(project.id)
      .then((next) => {
        setDetail(next);
        setFailure(null);
      })
      .catch((error: unknown) => setFailure(errorMessage(error)));
    projectDeployments(project.id, 20)
      .then(setDeployments)
      .catch(() => setDeployments([]));
    projectEvents(project.id, 30)
      .then(setEvents)
      .catch(() => setEvents([]));
    recentActivity(20, project.id)
      .then(setActivity)
      .catch(() => setActivity([]));
  }, [project.id]);

  useEffect(load, [load, project.status]);

  useEffect(() => {
    const timer = setInterval(() => setTick((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  const look = statusLook(project.status);
  const running = isRunning(project.status);
  const { blocked, reason: blockedReason } = runControls(project, { busy });

  async function act(verb: string, action: (id: string) => Promise<unknown>) {
    setBusy(true);
    try {
      await action(project.id);
      await onRefreshProjects();
      load();
      toast.success(`${project.displayName} ${verb}`);
    } catch (error) {
      // Declining an install is an answer, not a failure to report.
      if (!isDeclined(error)) {
        toast.error(`Could not ${verb.replace(/ed$/, '')} the project`, errorMessage(error));
      }
    } finally {
      setBusy(false);
    }
  }

  const uptime = uptimeSeconds(detail?.startedAt ?? null);

  return (
    <div className="mx-auto w-full max-w-[1200px] px-8 py-6">
      <button
        type="button"
        onClick={onBack}
        className="mb-4 inline-flex items-center gap-1.5 text-[13px] text-muted hover:text-ink"
      >
        <Icon name="chevron-left" size={14} />
        Projects
      </button>

      <header className="mb-5 flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <ProjectMark projectId={project.id} runtime={project.projectType} size={40} />
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="truncate text-[20px] leading-tight font-semibold tracking-tight">
                {project.displayName}
              </h1>
              <Badge tone={look.tone} dot>
                {look.label}
              </Badge>
              {detail && detail.health.toUpperCase() !== 'NONE' && (
                <Badge tone={healthLook(detail.health).tone}>
                  {healthLook(detail.health).label}
                </Badge>
              )}
            </div>
            <p className="mt-1 truncate text-[13px] text-muted">
              {project.description || project.slug}
            </p>
            {developerMode && (
              <p className="mt-1 font-mono text-[11px] text-faint select-text">
                {project.id} · {project.projectType}
              </p>
            )}
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {running ? (
            <>
              <Button
                icon="stop"
                disabled={blocked}
                title={blockedReason ?? 'Stop this project'}
                onClick={() => void act('stopped', stopProject)}
              >
                Stop
              </Button>
              <Button
                icon="restart"
                disabled={blocked}
                title={blockedReason ?? 'Restart this project'}
                onClick={() => void act('restarted', guard(restartProject))}
              >
                Restart
              </Button>
              <IconButton
                icon="power"
                label={blockedReason ?? 'Force kill'}
                disabled={blocked}
                onClick={() => setConfirmKill(true)}
              />
            </>
          ) : (
            <Button
              variant="primary"
              icon="play"
              disabled={blocked}
              title={blockedReason ?? 'Start this project'}
              onClick={() => void act('started', guard(startProject))}
            >
              Start
            </Button>
          )}
          <Button icon="file" onClick={onOpenFiles}>
            Files
          </Button>
        </div>
      </header>

      {failure && (
        <Card className="mb-4 border-danger/30 bg-danger-soft px-4 py-3">
          <p className="text-[13px] text-danger">{failure}</p>
        </Card>
      )}

      <Tabs<TabId>
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: 'overview', label: 'Overview' },
          { id: 'console', label: 'Console' },
          { id: 'deployments', label: 'Deployments', badge: deployments?.length },
          { id: 'history', label: 'History', badge: events?.length },
          { id: 'networking', label: 'Networking', badge: detail?.ports.length },
          { id: 'environment', label: 'Environment', badge: detail?.envVars.length },
          { id: 'resources', label: 'Resources' },
          { id: 'settings', label: 'Settings' },
        ]}
      />

      <div className="pt-4">
        {detail === null ? (
          <div className="grid gap-4 lg:grid-cols-2">
            <Skeleton className="h-56" />
            <Skeleton className="h-56" />
          </div>
        ) : tab === 'overview' ? (
          <Overview
            detail={detail}
            uptime={uptime}
            activity={activity}
            onOpenFiles={onOpenFiles}
            onReveal={() => {
              revealProjectPath(project.id, '').catch((error: unknown) =>
                toast.error('Could not open the folder', errorMessage(error)),
              );
            }}
          />
        ) : tab === 'deployments' ? (
          <Deployments deployments={deployments} />
        ) : tab === 'history' ? (
          <History events={events} detail={detail} />
        ) : tab === 'networking' ? (
          <Networking detail={detail} />
        ) : tab === 'environment' ? (
          <Environment detail={detail} />
        ) : tab === 'console' ? (
          <ProjectConsole projectId={project.id} />
        ) : tab === 'resources' ? (
          <Resources detail={detail} project={project} onChanged={load} />
        ) : (
          <Settings detail={detail} onChanged={load} />
        )}
      </div>

      {gate}

      {confirmKill && (
        <ConfirmDialog
          title={`Force kill ${project.displayName}?`}
          description="The process is stopped immediately, without letting it shut down cleanly."
          confirmLabel="Force kill"
          danger
          onCancel={() => setConfirmKill(false)}
          onConfirm={() => {
            setConfirmKill(false);
            void act('killed', killProject);
          }}
        />
      )}
    </div>
  );
}

// ------------------------------------------------------------------ overview

function Overview({
  detail,
  uptime,
  activity,
  onOpenFiles,
  onReveal,
}: {
  detail: Detail;
  uptime: number | null;
  activity: ActivityEntry[] | null;
  onOpenFiles: () => void;
  onReveal: () => void;
}) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader title="State" />
        <div className="px-4 py-1">
          <DataRow label="Uptime" value={uptime === null ? '—' : formatDuration(uptime)} />
          <DataRow label="Started" value={formatRelative(detail.startedAt)} />
          <DataRow label="Last stopped" value={formatRelative(detail.stoppedAt)} />
          <DataRow label="Restarts" value={detail.restartCount} />
          <DataRow
            label="Last exit code"
            value={detail.lastExitCode === null ? '—' : detail.lastExitCode}
          />
          <DataRow label="Wanted state" value={detail.desiredState.toLowerCase()} />
          <DataRow label="Autostart" value={detail.autostart ? 'on' : 'off'} />
        </div>
        {detail.lastFailureReason && (
          <div className="mx-4 mb-4 rounded-[8px] border border-danger/30 bg-danger-soft px-3 py-2">
            <p className="text-[12px] font-medium text-danger">Last failure</p>
            <p className="mt-0.5 text-[12px] break-words text-muted">{detail.lastFailureReason}</p>
            <p className="mt-1 text-[11px] text-faint">{formatTimestamp(detail.lastFailureAt)}</p>
          </div>
        )}
      </Card>

      <Card>
        <CardHeader title="Runtime" />
        <div className="px-4 py-1">
          <DataRow label="Type" value={runtimeLabel(detail.projectType)} />
          {detail.runtime ? (
            <>
              <DataRow label="Runtime" value={runtimeLabel(detail.runtime.runtime)} />
              <DataRow label="Version" value={detail.runtime.runtimeVersion || '—'} />
              <DataRow label="Package manager" value={detail.runtime.packageManager || '—'} />
              <DataRow label="Install" value={detail.runtime.installCommand ?? '—'} mono />
              <DataRow label="Build" value={detail.runtime.buildCommand ?? '—'} mono />
              <DataRow label="Start" value={detail.runtime.startCommand || '—'} mono />
              <DataRow label="Working directory" value={detail.runtime.workingDir || '—'} mono />
            </>
          ) : (
            <DataRow label="Runtime" value="not recorded" />
          )}
        </div>
      </Card>

      <Card>
        <CardHeader
          title="Source and storage"
          actions={
            <>
              <Button size="sm" onClick={onOpenFiles}>
                Open files
              </Button>
              <IconButton
                icon="external"
                label="Show in file manager"
                size="sm"
                onClick={onReveal}
              />
            </>
          }
        />
        <div className="px-4 py-1">
          <DataRow label="Source" value={detail.sourceType.toLowerCase()} />
          {detail.sourceUrl && <DataRow label="Remote" value={detail.sourceUrl} mono />}
          {detail.sourceRef && <DataRow label="Reference" value={detail.sourceRef} mono />}
          {detail.sourceCommit && (
            <DataRow label="Commit" value={detail.sourceCommit.slice(0, 12)} mono />
          )}
          <DataRow label="Folder" value={baseName(detail.directory)} mono hint={detail.directory} />
          <DataRow label="Created" value={formatRelative(detail.createdAt)} />
        </div>
      </Card>

      <Card>
        <CardHeader title="Recent activity" />
        {activity === null ? (
          <div className="space-y-2 p-4">
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-4 w-1/2" />
          </div>
        ) : activity.length === 0 ? (
          <p className="px-4 py-5 text-[13px] text-muted">Nothing recorded for this project yet.</p>
        ) : (
          <ul className="px-4 py-1">
            {activity.slice(0, 8).map((entry) => (
              <li
                key={entry.id}
                className="flex items-center gap-3 border-b border-edge/60 py-2 last:border-b-0"
              >
                <span className="min-w-0 flex-1 truncate text-[13px] text-ink">
                  {describeAction(entry.action, null)}
                </span>
                <span className="shrink-0 text-[12px] text-faint">
                  {formatRelative(entry.occurredAt)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {/* Said once, plainly, rather than as two tabs that open onto an
          apology. */}
      <Card className="lg:col-span-2">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
          <span className="text-muted">
            <Icon name="terminal" size={16} />
          </span>
          <p className="min-w-[240px] flex-1 text-[13px] text-muted">
            Live output and an interactive console are not built into the core yet, so this project
            has no log stream to show. Its files, state and history above are real.
          </p>
        </div>
      </Card>
    </div>
  );
}

// --------------------------------------------------------------- deployments

function Deployments({ deployments }: { deployments: DeploymentSummary[] | null }) {
  if (deployments === null) return <Skeleton className="h-40" />;
  if (deployments.length === 0) {
    return (
      <Card>
        <EmptyState
          icon="container"
          title="No deployments yet"
          description="A deployment is recorded each time the project is built or started. Start the project to create the first one."
        />
      </Card>
    );
  }

  return (
    <Card className="overflow-hidden">
      <ul>
        {deployments.map((deployment) => {
          const failed = deployment.status.toUpperCase().includes('FAIL');
          return (
            <li
              key={deployment.id}
              className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-edge px-4 py-2.5 last:border-b-0"
            >
              <Badge tone={failed ? 'danger' : 'ok'} dot>
                {deployment.status.toLowerCase()}
              </Badge>
              <span className="text-[13px] text-ink">
                {deployment.deploymentType.toLowerCase()}
              </span>
              <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-muted">
                {deployment.imageTag ?? ''}
              </span>
              <span className="tabular shrink-0 text-[12px] text-muted">
                {formatElapsed(deployment.durationMs)}
              </span>
              <span
                title={formatTimestamp(deployment.startedAt)}
                className="w-[76px] shrink-0 text-right text-[12px] text-faint"
              >
                {formatRelative(deployment.startedAt)}
              </span>
              {deployment.errorMessage && (
                <p className="w-full text-[12px] break-words text-danger">
                  {deployment.errorMessage}
                </p>
              )}
            </li>
          );
        })}
      </ul>
    </Card>
  );
}

// ------------------------------------------------------------------- history

function History({ events, detail }: { events: ContainerEvent[] | null; detail: Detail }) {
  return (
    <div className="grid gap-4 lg:grid-cols-[1fr_280px]">
      <Card className="overflow-hidden">
        <CardHeader title="Container events" subtitle="Starts, stops and crashes" />
        {events === null ? (
          <div className="space-y-2 p-4">
            <Skeleton className="h-5" />
            <Skeleton className="h-5" />
          </div>
        ) : events.length === 0 ? (
          <p className="px-4 py-6 text-center text-[13px] text-muted">
            Nothing has happened to this project&apos;s container yet.
          </p>
        ) : (
          <ul>
            {events.map((event) => (
              <li
                key={event.id}
                className="flex items-center gap-3 border-b border-edge/60 px-4 py-2 last:border-b-0"
              >
                <span className="text-[13px] text-ink">{event.eventType.toLowerCase()}</span>
                {event.exitCode !== null && (
                  <Badge tone={event.exitCode === 0 ? 'neutral' : 'danger'}>
                    exit {event.exitCode}
                  </Badge>
                )}
                <span className="min-w-0 flex-1 truncate text-[12px] text-muted">
                  {event.detail ?? ''}
                </span>
                <span
                  title={formatTimestamp(event.occurredAt)}
                  className="shrink-0 text-[12px] text-faint"
                >
                  {formatRelative(event.occurredAt)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card>
        <CardHeader title="Summary" />
        <div className="px-4 py-1">
          <DataRow label="Restarts" value={detail.restartCount} />
          <DataRow
            label="Last exit code"
            value={detail.lastExitCode === null ? '—' : detail.lastExitCode}
          />
          <DataRow label="Last failure" value={formatRelative(detail.lastFailureAt)} />
          <DataRow label="Restart policy" value={detail.restartPolicy.toLowerCase()} />
        </div>
      </Card>
    </div>
  );
}

// ---------------------------------------------------------------- networking

function Networking({ detail }: { detail: Detail }) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card className="overflow-hidden">
        <CardHeader title="Ports" />
        {detail.ports.length === 0 ? (
          <p className="px-4 py-5 text-[13px] text-muted">
            No port is mapped. A project that does not listen on one does not need it.
          </p>
        ) : (
          <ul>
            {detail.ports.map((port) => (
              <li
                key={`${port.containerPort}-${port.protocol}`}
                className="flex items-center gap-3 border-b border-edge/60 px-4 py-2.5 last:border-b-0"
              >
                <span className="tabular font-mono text-[13px] text-ink">
                  {port.hostPort ?? '—'}
                </span>
                <Icon name="arrow-right" size={14} className="text-faint" />
                <span className="tabular font-mono text-[13px] text-ink">{port.containerPort}</span>
                <span className="flex-1 text-[12px] text-muted uppercase">{port.protocol}</span>
                {port.hostPort !== null && (
                  <span className="font-mono text-[12px] text-accent select-text">
                    localhost:{port.hostPort}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card>
        <CardHeader title="Network" />
        <div className="px-4 py-1">
          <DataRow label="Mode" value={detail.networkMode.toLowerCase()} />
          <DataRow label="Container" value={detail.containerName ?? '—'} mono />
          <DataRow label="Image" value={detail.imageTag ?? '—'} mono />
          <DataRow
            label="Health check"
            value={detail.runtime?.healthCheckType.toLowerCase() ?? '—'}
          />
          {detail.runtime?.healthCheckTarget && (
            <DataRow label="Health target" value={detail.runtime.healthCheckTarget} mono />
          )}
        </div>
      </Card>
    </div>
  );
}

// --------------------------------------------------------------- environment

function Environment({ detail }: { detail: Detail }) {
  const [revealed, setRevealed] = useState<string[]>([]);

  if (detail.envVars.length === 0) {
    return (
      <Card>
        <EmptyState
          icon="settings"
          title="No environment variables"
          description="Nothing is set for this project. Editing them from this screen is not built yet — the core stores them, but exposes no write command."
        />
      </Card>
    );
  }

  return (
    <Card className="overflow-hidden">
      <CardHeader
        title="Environment variables"
        subtitle="Secret values are never sent to this window"
      />
      <ul>
        {detail.envVars.map((variable) => {
          const shown = revealed.includes(variable.key);
          return (
            <li
              key={variable.key}
              className="flex items-center gap-3 border-b border-edge/60 px-4 py-2 last:border-b-0"
            >
              <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink select-text">
                {variable.key}
              </span>
              {variable.isSecret ? (
                <Badge tone="warn">secret</Badge>
              ) : (
                <span className="min-w-0 flex-1 truncate text-right font-mono text-[12px] text-muted select-text">
                  {shown ? (variable.value ?? '') : '••••••••'}
                </span>
              )}
              {!variable.isSecret && (
                <button
                  type="button"
                  onClick={() =>
                    setRevealed((current) =>
                      shown
                        ? current.filter((key) => key !== variable.key)
                        : [...current, variable.key],
                    )
                  }
                  className="shrink-0 text-[12px] text-accent hover:underline"
                >
                  {shown ? 'Hide' : 'Show'}
                </button>
              )}
              {variable.restartRequired && <Badge tone="neutral">restart needed</Badge>}
            </li>
          );
        })}
      </ul>
    </Card>
  );
}

// ----------------------------------------------------------------- resources

function Resources({
  detail,
  project,
  onChanged,
}: {
  detail: Detail;
  project: ProjectSummary;
  onChanged: () => void;
}) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <PowerCard detail={detail} project={project} onChanged={onChanged} />
      <Card>
        <CardHeader title="Limits" subtitle="What this project is allowed to use" />
        <div className="px-4 py-1">
          <DataRow
            label="Memory"
            value={
              detail.memoryLimitMb > 0
                ? formatBytes(detail.memoryLimitMb * 1024 * 1024)
                : 'unlimited'
            }
          />
          <DataRow
            label="CPU"
            value={detail.cpuLimitCores > 0 ? `${detail.cpuLimitCores} cores` : 'unlimited'}
          />
          <DataRow
            label="Disk"
            value={
              detail.storageLimitMb > 0
                ? formatBytes(detail.storageLimitMb * 1024 * 1024)
                : 'unlimited'
            }
          />
        </div>
      </Card>

      <Card>
        <CardHeader title="Live usage" />
        <div className="px-4 py-5">
          <p className="text-[13px] text-muted">
            Per-project CPU and memory are not measured. Reading them means streaming Docker&apos;s
            stats API, which the container manager does not do yet — so nothing is shown here rather
            than a number nobody collected.
          </p>
          <p className="mt-3 text-[13px] text-muted">
            The machine&apos;s own CPU, memory and disk are measured, and are on the Overview page.
          </p>
        </div>
      </Card>
    </div>
  );
}

/**
 * What this project is worth when the machine is busy, and whether it is a
 * reason to keep the machine awake.
 *
 * Neither control can stop or slow a project into failure. Priority moves the
 * operating system's scheduling only — it is never a cap — and that is said on
 * the card rather than left for the user to assume, because "Low" is a word
 * that sounds like a limit.
 */
function PowerCard({
  detail,
  project,
  onChanged,
}: {
  detail: Detail;
  project: ProjectSummary;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);

  // The row is the authority: it is what the power manager reads. `project`
  // carries the summary the list was drawn from, which can be a poll behind.
  const priority = (detail.priority ?? 'NORMAL') as ProjectPriority;
  const keepAwake = detail.keepAwake ?? false;
  // Every project runs on this machine now.
  const host = true;

  async function apply(changes: { priority?: ProjectPriority; keepAwake?: boolean }) {
    setBusy(true);
    try {
      await setProjectPower(project.id, changes);
      onChanged();
    } catch (error) {
      toast.error('Could not change the power settings', errorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader title="Power" subtitle="How this project is treated when the machine is busy" />

      {!host ? (
        <p className="px-4 py-5 text-[13px] text-muted">
          These apply to projects that run as a process on this machine. A container is scheduled by
          the daemon, which this application does not manage.
        </p>
      ) : (
        <div className="px-4 pb-4 pt-2">
          <label className="block text-[12px] text-muted" htmlFor="project-priority">
            Scheduling priority
          </label>
          <Select
            id="project-priority"
            value={priority}
            disabled={busy}
            onChange={(value) => void apply({ priority: value as ProjectPriority })}
            options={[
              { value: 'LOW', label: 'Low — yield to everything else' },
              { value: 'NORMAL', label: 'Normal' },
              { value: 'HIGH', label: 'High — keep this one responsive' },
            ]}
          />
          <p className="mt-1.5 text-[12px] text-muted">
            This changes the order the operating system runs things in. It is never a limit, and
            nothing here can stop this project or make it fail.
          </p>

          <div className="mt-4 flex items-start justify-between gap-3 border-t border-edge pt-3">
            <div className="min-w-0">
              <p className="text-[13px] text-ink">Keep this machine awake</p>
              <p className="mt-0.5 text-[12px] text-muted">
                While this project is running, hold off automatic sleep. Off by default: stopping a
                machine sleeping is something to opt into, per project.
              </p>
            </div>
            <Toggle
              checked={keepAwake}
              disabled={busy}
              onChange={(next) => void apply({ keepAwake: next })}
              label="Keep this machine awake"
            />
          </div>
        </div>
      )}
    </Card>
  );
}

// ------------------------------------------------------------------ settings

function Settings({ detail }: { detail: Detail; onChanged: () => void }) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader title="Identity" />
        <div className="px-4 py-1">
          <DataRow label="Name" value={detail.displayName} />
          <DataRow label="Slug" value={detail.slug} mono />
          <DataRow label="Description" value={detail.description || '—'} />
          <DataRow label="Identifier" value={detail.id} mono />
        </div>
      </Card>

      <Card>
        <CardHeader title="Behaviour" />
        <div className="px-4 py-1">
          <DataRow label="Autostart" value={detail.autostart ? 'on' : 'off'} />
          <DataRow label="Restart policy" value={detail.restartPolicy.toLowerCase()} />
          <DataRow label="Updated" value={formatRelative(detail.updatedAt)} />
        </div>
      </Card>

      <Card className="lg:col-span-2">
        <div className="px-4 py-3">
          <p className="text-[13px] text-muted">
            Apart from the run mode above, changing a project&apos;s configuration after it is
            created is not built yet: the core has no command that writes these fields. They are
            shown here because they are what the project is actually running with.
          </p>
        </div>
      </Card>
    </div>
  );
}
