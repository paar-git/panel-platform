/**
 * What a project actually runs, one row per process.
 *
 * A project used to be one command, so there was nothing to list. Now it can
 * be an API, a web dev server and a worker, and when one of them dies the
 * first thing the user needs is *which* — a project that says only "crashed"
 * makes them read logs to find out something the row already knows.
 *
 * Every control here is wired to a real command. The restart button restarts
 * the process it names and leaves the siblings alone, which is the whole
 * reason it exists beside the project-wide one.
 */
import { useState } from 'react';

import type { ProcessSummary } from '../api';

/** The dot beside a process name, and what it means. */
function statusTone(status: string): { tone: string; label: string } {
  switch (status) {
    case 'RUNNING':
      return { tone: 'bg-emerald-500', label: 'Running' };
    case 'STARTING':
      return { tone: 'bg-sky-500', label: 'Starting' };
    case 'RESTARTING':
      return { tone: 'bg-amber-500', label: 'Restarting' };
    case 'STOPPING':
      return { tone: 'bg-amber-500', label: 'Stopping' };
    // Died after having run, and died having never run. Deliberately distinct:
    // one means the code threw, the other that it could not start at all.
    case 'CRASHED':
      return { tone: 'bg-red-500', label: 'Crashed' };
    case 'FAILED':
      return { tone: 'bg-red-500', label: 'Failed' };
    default:
      return { tone: 'bg-neutral-400', label: 'Stopped' };
  }
}

export function ProcessList({
  projectId,
  processes,
  onRestart,
}: {
  projectId: string;
  processes: ProcessSummary[];
  onRestart: (processName: string) => void | Promise<void>;
}) {
  const [busy, setBusy] = useState<string | null>(null);

  if (processes.length === 0) {
    return (
      <p className="px-4 py-3 text-[13px] text-neutral-500">
        This project has no processes, so there is nothing to run. Add one in its settings.
      </p>
    );
  }

  async function restart(name: string) {
    setBusy(name);
    try {
      await onRestart(name);
    } finally {
      setBusy(null);
    }
  }

  return (
    <ul className="divide-y divide-neutral-200 dark:divide-neutral-800" data-project={projectId}>
      {processes.map((process) => {
        const { tone, label } = statusTone(process.status);
        return (
          <li key={process.id} className="flex items-start gap-3 px-4 py-3">
            <span className={`mt-[6px] h-2 w-2 shrink-0 rounded-full ${tone}`} aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <div className="flex items-baseline gap-2">
                <span className="text-[13px] font-medium">{process.name}</span>
                <span className="text-[12px] text-neutral-500">{label}</span>
                {process.port !== null && (
                  <span className="text-[12px] text-neutral-500" title="The port it listens on">
                    {process.port}
                  </span>
                )}
              </div>

              <p
                className="truncate font-mono text-[12px] text-neutral-500"
                title={process.command}
              >
                {process.command}
              </p>

              {process.failureReason !== null && (
                <p className="mt-1 text-[12px] text-red-600 dark:text-red-400">
                  {process.failureReason}
                  {process.exitCode !== null && ` (exit ${process.exitCode})`}
                </p>
              )}

              {process.restartCount > 0 && (
                <p className="mt-1 text-[12px] text-neutral-500">
                  Restarted {process.restartCount} {process.restartCount === 1 ? 'time' : 'times'}
                </p>
              )}
            </div>

            <button
              type="button"
              className="shrink-0 rounded border border-neutral-300 px-2 py-1 text-[12px] disabled:opacity-50 dark:border-neutral-700"
              disabled={busy !== null}
              onClick={() => void restart(process.name)}
            >
              {busy === process.name ? 'Restarting…' : 'Restart'}
            </button>
          </li>
        );
      })}
    </ul>
  );
}
