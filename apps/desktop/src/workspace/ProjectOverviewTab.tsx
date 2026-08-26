/**
 * A project, at a glance.
 *
 * Deliberately not a dashboard. Everything here is one of two things: a fact
 * about this project that is true right now, or an action on it. There are no
 * charts, because a project that has been up for four minutes has nothing to
 * plot, and no cards inside cards — the sections are separated by rules.
 *
 * Figures that are not measured are absent rather than zero. A container's CPU
 * is not read by this application, and drawing `0%` would be inventing it.
 */
import { useCallback, useEffect, useState } from 'react';

import {
  errorMessage,
  machineLoad,
  projectDetails,
  type ProjectDetail,
  type ProjectSummary,
  type RunningProject,
} from '../api';
import { uptimeSeconds } from '../components/RunningPanel';
import { formatBytes, formatDuration } from '../lib/format';
import { statusLook } from '../lib/projects';
import StatusDot from '../shell/StatusDot';
import Icon from '../ui/Icon';
import { toast } from '../ui/toast';

export default function ProjectOverviewTab({
  project,
  onOpenConsole,
  onOpenFiles,
  onRestart,
  busy,
}: {
  project: ProjectSummary;
  onOpenConsole: () => void;
  onOpenFiles: () => void;
  onRestart: () => void;
  busy: boolean;
}) {
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [live, setLive] = useState<RunningProject | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [, setTick] = useState(0);

  const load = useCallback(() => {
    projectDetails(project.id)
      .then((next) => {
        setDetail(next);
        setFailure(null);
      })
      .catch((error: unknown) => setFailure(errorMessage(error)));
  }, [project.id]);

  useEffect(load, [load, project.status]);

  // The live figures come from the machine sampler rather than the record, so
  // they change while the page is open.
  useEffect(() => {
    function read() {
      machineLoad()
        .then((next) =>
          setLive(next.running.find((entry) => entry.projectId === project.id) ?? null),
        )
        .catch(() => setLive(null));
    }
    read();
    const timer = setInterval(read, 2000);
    return () => clearInterval(timer);
  }, [project.id]);

  useEffect(() => {
    const timer = setInterval(() => setTick((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  const look = statusLook(project.status);
  const running = project.status === 'RUNNING';
  const up = uptimeSeconds(live?.startedAt ?? detail?.startedAt ?? null);
  const port =
    live?.port ?? detail?.ports.find((entry) => entry.hostPort !== null)?.hostPort ?? null;

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      {/* ---------------------------------------------------------- header */}
      <div className="border-b border-edge px-4 py-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <h1 className="text-[16px] font-semibold text-ink">{project.displayName}</h1>
          <span className="flex items-center gap-1.5 text-[12px] text-muted">
            <StatusDot status={project.status} />
            {look.label}
          </span>
        </div>

        <div className="mt-1 flex flex-wrap items-center gap-x-4 gap-y-0.5 text-[12px] text-faint tabular">
          <span className="not-tabular">{humanType(project.projectType)}</span>
          {running && up !== null && <span>Uptime {formatDuration(up)}</span>}
          {running && port !== null && <span>Port {port}</span>}
          {running && live?.memoryBytes !== undefined && (
            <span title={live.measured ? 'Measured' : "The project's declared limit"}>
              {formatBytes(live.memoryBytes)}
              {!live.measured && <span className="not-tabular"> limit</span>}
            </span>
          )}
        </div>

        <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
          <SmallButton
            icon="restart"
            label="Restart"
            disabled={!running || busy}
            onClick={onRestart}
          />
          <SmallButton icon="terminal" label="Console" onClick={onOpenConsole} />
          <SmallButton icon="file" label="Files" onClick={onOpenFiles} />
          {running && port !== null && (
            <a
              href={`http://localhost:${port}`}
              target="_blank"
              rel="noreferrer"
              className="flex h-[26px] items-center gap-1.5 rounded-[5px] border border-edge bg-raised px-2 text-[12.5px] text-ink hover:bg-overlay"
            >
              <Icon name="external" size={13} />
              Open
            </a>
          )}
        </div>
      </div>

      {failure !== null && <p className="px-4 py-2 text-[12px] text-danger">{failure}</p>}

      {/* --------------------------------------------------------- details */}
      <Section label="Local address">
        {running && port !== null ? (
          <Copyable text={`http://localhost:${port}`} />
        ) : (
          <Plain text={running ? 'This project publishes no port.' : 'Not running.'} />
        )}
      </Section>

      <Section label="Resources">
        <Fact
          label="Memory"
          value={live === null ? '—' : formatBytes(live.memoryBytes)}
          note={live !== null && !live.measured ? 'declared limit' : undefined}
        />
        <Fact
          label="Processor"
          value={
            live === null || live.cpuPercent === null ? '—' : `${Math.round(live.cpuPercent)}%`
          }
        />
      </Section>

      <Section label="Project directory">
        {detail === null ? <Plain text="—" /> : <Copyable text={detail.directory} mono />}
      </Section>
    </div>
  );
}

function humanType(type: string): string {
  return type
    .toLowerCase()
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

function Section({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="border-b border-edge/60 px-4 py-2.5">
      <h2 className="mb-1 text-[10.5px] font-semibold tracking-wide text-faint uppercase">
        {label}
      </h2>
      {children}
    </section>
  );
}

function Fact({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="flex items-baseline gap-3 py-0.5 text-[12.5px]">
      <span className="w-20 shrink-0 text-muted">{label}</span>
      <span className="text-ink tabular">{value}</span>
      {note !== undefined && <span className="text-[11px] text-faint">{note}</span>}
    </div>
  );
}

function Plain({ text }: { text: string }) {
  return <p className="text-[12.5px] text-muted">{text}</p>;
}

function Copyable({ text, mono = false }: { text: string; mono?: boolean }) {
  return (
    <div className="flex items-center gap-1.5">
      <span
        className={`min-w-0 flex-1 truncate text-[12.5px] text-ink select-text ${
          mono ? 'font-mono text-[12px]' : ''
        }`}
        title={text}
      >
        {text}
      </span>
      <button
        type="button"
        title="Copy"
        aria-label="Copy"
        onClick={() => {
          void navigator.clipboard
            .writeText(text)
            .then(() => toast.success('Copied'))
            .catch(() => toast.error('Could not copy'));
        }}
        className="grid h-5 w-5 shrink-0 place-items-center rounded-[3px] text-faint hover:bg-raised hover:text-ink"
      >
        <Icon name="copy" size={12} />
      </button>
    </div>
  );
}

function SmallButton({
  icon,
  label,
  onClick,
  disabled = false,
}: {
  icon: 'restart' | 'terminal' | 'file';
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex h-[26px] items-center gap-1.5 rounded-[5px] border border-edge bg-raised px-2 text-[12.5px] text-ink hover:bg-overlay disabled:cursor-not-allowed disabled:text-faint disabled:hover:bg-raised"
    >
      <Icon name={icon} size={13} />
      {label}
    </button>
  );
}
