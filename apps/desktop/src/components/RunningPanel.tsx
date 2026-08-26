/**
 * Everything running on this machine, in one place.
 *
 * The per-project pages answer "how is this project"; nothing answered "what is
 * this machine carrying right now", which is the question somebody asks when
 * the fans spin up. One row per running project, with what it costs and how to
 * reach it.
 *
 * Measured and declared figures are never mixed silently. A host project's
 * memory is read from its process tree; a container's is its declared limit,
 * because the daemon's stats endpoint is not wired up — so those rows say so
 * rather than presenting a bound as a reading.
 */
import { useCallback, useEffect, useState } from 'react';

import { errorMessage, machineLoad, type MachineLoad, type RunningProject } from '../api';
import { formatBytes, formatDuration, percentOf } from '../lib/format';
import { Badge, Card, CardHeader, DataRow } from '../ui/primitives';

/** Matches the core's own sampler, so the numbers change when it resamples. */
const POLL_MS = 2000;

/** Seconds a project has been up, or null when the core did not say. */
export function uptimeSeconds(startedAt: string | null, now: Date = new Date()): number | null {
  if (startedAt === null) return null;
  const started = Date.parse(startedAt);
  if (Number.isNaN(started)) return null;
  const seconds = Math.floor((now.getTime() - started) / 1000);
  // A clock that disagrees with the core's must not render as a negative
  // uptime; zero is the honest floor.
  return seconds < 0 ? 0 : seconds;
}

export default function RunningPanel({ onOpenProject }: { onOpenProject?: (id: string) => void }) {
  const [load, setLoad] = useState<MachineLoad | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  /** Re-rendered on a timer so uptimes count up between polls. */
  const [, setTick] = useState(0);

  const read = useCallback(() => {
    machineLoad()
      .then((next) => {
        setLoad(next);
        setFailure(null);
      })
      .catch((error: unknown) => setFailure(errorMessage(error)));
  }, []);

  useEffect(() => {
    read();
    const timer = setInterval(read, POLL_MS);
    return () => clearInterval(timer);
  }, [read]);

  useEffect(() => {
    const timer = setInterval(() => setTick((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  const running: RunningProject[] = load?.running ?? [];

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card className="lg:col-span-2">
        <CardHeader
          title="Running now"
          subtitle="Every project this machine is carrying"
          actions={
            <Badge tone={running.length > 0 ? 'ok' : 'neutral'} dot>
              {running.length}
            </Badge>
          }
        />

        {failure !== null && <p className="px-4 pb-2 text-[12px] text-danger">{failure}</p>}

        {running.length === 0 ? (
          <p className="px-4 py-6 text-center text-[13px] text-muted">Nothing is running.</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-[13px]">
              <thead>
                <tr className="border-y border-edge text-[11px] uppercase tracking-wide text-muted">
                  <th className="px-4 py-2 text-left font-medium">Project</th>
                  <th className="px-4 py-2 text-right font-medium">Memory</th>
                  <th className="px-4 py-2 text-right font-medium">CPU</th>
                  <th className="px-4 py-2 text-right font-medium">Uptime</th>
                  <th className="px-4 py-2 text-right font-medium">Port</th>
                </tr>
              </thead>
              <tbody>
                {running.map((project) => {
                  const up = uptimeSeconds(project.startedAt);
                  return (
                    <tr
                      key={project.projectId}
                      onClick={() => onOpenProject?.(project.projectId)}
                      className={`border-b border-edge/60 last:border-b-0 ${
                        onOpenProject ? 'cursor-pointer hover:bg-raised' : ''
                      }`}
                    >
                      <td className="px-4 py-2">
                        <span className="text-ink">{project.displayName}</span>
                        {!project.measured && (
                          <span
                            className="ml-2 text-[11px] text-muted"
                            title="A container's figure is its declared limit, not a reading."
                          >
                            declared
                          </span>
                        )}
                      </td>
                      <td className="px-4 py-2 text-right tabular">
                        {formatBytes(project.memoryBytes)}
                      </td>
                      <td className="px-4 py-2 text-right tabular">
                        {/* Absent rather than zero: a container's CPU is not
                            measured, and 0% would be an invented reading. */}
                        {project.cpuPercent === null ? '—' : `${Math.round(project.cpuPercent)}%`}
                      </td>
                      <td className="px-4 py-2 text-right tabular">
                        {up === null ? '—' : formatDuration(up)}
                      </td>
                      <td className="px-4 py-2 text-right tabular">
                        {project.port === null ? '—' : project.port}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      <Card>
        <CardHeader title="This machine" subtitle="What is left after what is running" />
        <div className="py-1">
          <DataRow
            label="Memory in use"
            value={
              load === null || !load.measured
                ? 'measuring…'
                : `${percentOf(
                    load.totalMemoryBytes - load.availableMemoryBytes,
                    load.totalMemoryBytes,
                  )} of ${formatBytes(load.totalMemoryBytes)}`
            }
          />
          <DataRow
            label="Headroom"
            value={load === null || !load.measured ? '—' : formatBytes(load.headroomBytes)}
          />
          {/* Named rather than folded into headroom: a user who sees 4 GB free
              and is refused a start deserves to know why. */}
          <DataRow
            label="Held back"
            value={load === null || !load.measured ? '—' : formatBytes(load.reserveBytes)}
          />
          <DataRow
            label="Processor"
            value={
              load === null || load.cpuPercent === null
                ? '—'
                : `${Math.round(load.cpuPercent)}% of ${load.logicalCores} cores`
            }
          />
        </div>
      </Card>
    </div>
  );
}
